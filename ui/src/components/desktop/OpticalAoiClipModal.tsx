import { useEffect, useMemo, useRef, useState } from "react";
import {
  AlertTriangle,
  CheckCircle2,
  FolderOpen,
  Layers,
  Loader2,
  RotateCcw,
  ShieldCheck,
  X,
} from "lucide-react";
import { useI18n } from "@/i18n";
import { cn } from "@/lib/utils";
import { AoiDrawMap } from "@/components/AoiDrawMap";
import { SpSelect } from "@/components/ui/SpSelect";
import { SubsetScissorsIcon } from "@/components/desktop/SubsetScissorsIcon";
import { recordSubsetSizeEstimate } from "@/lib/subsetSizeEstimateRegistry";
import { isTauriDesktopHost } from "@/lib/bridge";
import { formatBridgeErrorValue } from "@/lib/bridgeErrorFormat";
import { formatSignedLat, formatSignedLon } from "@/lib/geoCoords";
import { formatBytes } from "@/shell/api/agentStreamLogic";
import {
  formatHumanSceneLabel,
  isTemporarySystemDownloadPath,
  TEMPORARY_DOWNLOAD_PATH_ERROR_ZH,
} from "@/lib/storagePathValidation";
import type { Bbox, Json } from "@/lib/bridge";
import type {
  DesktopOpticalCloudMaskPolicy,
  DesktopOpticalDerivedCreateTaskRequest,
  DesktopOpticalDerivedPrecheckBatchRequest,
  DesktopOpticalMosaicStrategy,
  DesktopOpticalPrecheckReport,
  DesktopOpticalPrecheckResult,
  DesktopOpticalResampling,
} from "@/lib/desktopShellBridge";
import {
  availableBandsForOpticalFamily,
  bandPresetOrder,
  extractWgs84RingsAndBbox,
  resolveOpticalDerivedSelection,
  type OpticalAoiSceneLike,
  type OpticalBandPresetId,
} from "@/lib/opticalAoiClipLogic";

export type OpticalAoiClipModalProps = {
  sourceBatchId: string;
  expectedWorkbenchRevision: number;
  scenes: readonly OpticalAoiSceneLike[];
  initialOutputRoot: string;
  initialConcurrency: number;
  aoiGeometry?: Json | Record<string, unknown> | null;
  aoiBbox?: Bbox | readonly [number, number, number, number] | null;
  aoiLabel?: string | null;
  onClose: () => void;
  onPrecheck: (request: DesktopOpticalDerivedPrecheckBatchRequest) => Promise<DesktopOpticalPrecheckResult>;
  onPickDirectory?: () => Promise<string | null>;
  onSubmit: (request: DesktopOpticalDerivedCreateTaskRequest) => Promise<void>;
};

function createOpticalClientTaskId(): string {
  const randomId = globalThis.crypto?.randomUUID?.();
  return `task-opt-${randomId || `${Date.now()}-${Math.random().toString(16).slice(2, 8)}`}`;
}

function createOpticalRequestId(): string {
  const randomId = globalThis.crypto?.randomUUID?.();
  return `req-opt-${randomId || `${Date.now()}-${Math.random().toString(16).slice(2, 8)}`}`;
}

function bboxToGeoJsonPolygon(b: Bbox): Json {
  return {
    type: "Polygon",
    coordinates: [
      [
        [b.west, b.south],
        [b.east, b.south],
        [b.east, b.north],
        [b.west, b.north],
        [b.west, b.south],
      ],
    ],
  };
}

function resolveSceneFootprintGeometry(scene: OpticalAoiSceneLike): Json | null {
  if (scene.footprint_geojson && typeof scene.footprint_geojson === "object") {
    return scene.footprint_geojson;
  }
  if (scene.footprint_bbox) {
    return bboxToGeoJsonPolygon(scene.footprint_bbox);
  }
  const meta =
    scene.native_metadata && typeof scene.native_metadata === "object" && !Array.isArray(scene.native_metadata)
      ? (scene.native_metadata as Record<string, unknown>)
      : null;
  const rawBbox = meta?.bbox;
  if (
    Array.isArray(rawBbox) &&
    rawBbox.length === 4 &&
    rawBbox.every((v) => typeof v === "number" && Number.isFinite(v))
  ) {
    return bboxToGeoJsonPolygon({
      west: Math.min(rawBbox[0], rawBbox[2]),
      south: Math.min(rawBbox[1], rawBbox[3]),
      east: Math.max(rawBbox[0], rawBbox[2]),
      north: Math.max(rawBbox[1], rawBbox[3]),
      crs: "EPSG:4326",
    });
  }
  return null;
}

export function OpticalAoiClipModal({
  sourceBatchId,
  expectedWorkbenchRevision,
  scenes,
  initialOutputRoot,
  initialConcurrency,
  aoiGeometry,
  aoiBbox,
  aoiLabel,
  onClose,
  onPrecheck,
  onPickDirectory,
  onSubmit,
}: OpticalAoiClipModalProps) {
  const { t } = useI18n();
  const selection = useMemo(() => resolveOpticalDerivedSelection(scenes), [scenes]);
  const baseAoiPayload = useMemo(
    () => extractWgs84RingsAndBbox(aoiGeometry, aoiBbox),
    [aoiGeometry, aoiBbox],
  );
  const [customBbox, setCustomBbox] = useState<Bbox | null>(null);
  const [droppedGeometry, setDroppedGeometry] = useState<Json | null>(null);
  const [multiSceneMode, setMultiSceneMode] = useState<
    "batch_single_scene_clip" | "near_coincident_spatial_mosaic"
  >("batch_single_scene_clip");

  const mapBbox = useMemo<Bbox>(() => {
    if (customBbox) return customBbox;
    if (baseAoiPayload) {
      const [west, south, east, north] = baseAoiPayload.aoiBboxWgs84;
      return { west, south, east, north, crs: "EPSG:4326" };
    }
    const firstFp = scenes.find((s) => s.footprint_bbox)?.footprint_bbox;
    if (firstFp) return firstFp;
    return { west: 117.15, south: 31.78, east: 117.35, north: 31.95, crs: "EPSG:4326" };
  }, [baseAoiPayload, customBbox, scenes]);

  const aoiPayload = useMemo(() => {
    if (droppedGeometry) {
      return extractWgs84RingsAndBbox(droppedGeometry, customBbox) ?? extractWgs84RingsAndBbox(null, mapBbox);
    }
    if (customBbox) {
      return extractWgs84RingsAndBbox(null, customBbox);
    }
    if (baseAoiPayload) return baseAoiPayload;
    return extractWgs84RingsAndBbox(null, mapBbox);
  }, [baseAoiPayload, customBbox, droppedGeometry, mapBbox]);

  const requestedGeometry = useMemo<Json | null>(() => {
    if (droppedGeometry) return droppedGeometry;
    if (customBbox) return bboxToGeoJsonPolygon(customBbox);
    if (aoiGeometry && typeof aoiGeometry === "object") return aoiGeometry as Json;
    if (aoiPayload) return bboxToGeoJsonPolygon(mapBbox);
    return null;
  }, [aoiGeometry, aoiPayload, customBbox, droppedGeometry, mapBbox]);

  const footprintGeometries = useMemo<Json[]>(() => {
    return scenes
      .map((sc) => resolveSceneFootprintGeometry(sc))
      .filter((g): g is Json => Boolean(g));
  }, [scenes]);

  const family = selection.ok ? selection.family : "sentinel2_l2a";
  const mode = !selection.ok
    ? "single_scene_clip"
    : scenes.length <= 1
    ? "single_scene_clip"
    : multiSceneMode;
  const isBatchSingleClip = mode === "batch_single_scene_clip";
  const isMosaic = mode === "near_coincident_spatial_mosaic";
  const isSentinel2L1c = family === "sentinel2_l1c";
  const bandOptions = useMemo(() => availableBandsForOpticalFamily(family), [family]);

  const [bandPreset, setBandPreset] = useState<OpticalBandPresetId>("true_color");
  const [selectedBands, setSelectedBands] = useState<string[]>(() =>
    bandPresetOrder(family, "true_color"),
  );
  const [crsMode, setCrsMode] = useState<"auto" | "wgs84" | "custom">("auto");
  const [customCrs, setCustomCrs] = useState("");
  const [customResolution, setCustomResolution] = useState("");
  const [resampling, setResampling] = useState<DesktopOpticalResampling>("bilinear");
  const [cloudMask, setCloudMask] = useState<DesktopOpticalCloudMaskPolicy>("none");
  const [mosaicStrategy, setMosaicStrategy] = useState<DesktopOpticalMosaicStrategy>("quality_first");
  const [writeLineageSidecar, setWriteLineageSidecar] = useState(true);
  const [writeProvenanceSidecars, setWriteProvenanceSidecars] = useState(false);
  const [outputRoot, setOutputRoot] = useState(initialOutputRoot);
  useEffect(() => {
    if (!outputRoot && initialOutputRoot) {
      setOutputRoot(initialOutputRoot);
    }
  }, [initialOutputRoot, outputRoot]);
  const [overwrite, setOverwrite] = useState(true);

  const [precheckBusy, setPrecheckBusy] = useState(false);
  const [precheckError, setPrecheckError] = useState<string | null>(null);
  const [precheckReport, setPrecheckReport] = useState<DesktopOpticalPrecheckReport | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [submitError, setSubmitError] = useState<string | null>(null);

  const precheckSeqRef = useRef(0);
  const onPrecheckRef = useRef(onPrecheck);
  onPrecheckRef.current = onPrecheck;

  const customResolutionNum =
    customResolution.trim() === "" ? null : Number(customResolution.trim());
  const validCustomResolution =
    customResolutionNum === null
    || (Number.isFinite(customResolutionNum) && customResolutionNum > 0);

  const selectedBandsMinResM = useMemo(() => {
    const resMap = new Map(bandOptions.map((b) => [b.id, b.resolutionM]));
    let minRes = Number.POSITIVE_INFINITY;
    for (const bId of selectedBands) {
      const r = resMap.get(bId);
      if (typeof r === "number" && r > 0 && r < minRes) {
        minRes = r;
      }
    }
    if (Number.isFinite(minRes)) return minRes;
    return bandOptions[0]?.resolutionM ?? (family === "sentinel2_l2a" || family === "sentinel2_l1c" ? 10 : 30);
  }, [bandOptions, family, selectedBands]);

  const selectedBandsUniformNativeRes = useMemo(() => {
    const resMap = new Map(bandOptions.map((b) => [b.id, b.resolutionM]));
    const uniqueRes = new Set<number>();
    for (const bId of selectedBands) {
      const r = resMap.get(bId);
      if (typeof r === "number" && r > 0) uniqueRes.add(r);
    }
    return uniqueRes.size <= 1;
  }, [bandOptions, selectedBands]);

  const effectiveResolutionM =
    customResolutionNum !== null && validCustomResolution
      ? customResolutionNum
      : selectedBandsMinResM;

  useEffect(() => {
    const onKeyDown = (ev: KeyboardEvent) => {
      if (ev.key === "Escape" && !submitting) {
        onClose();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose, submitting]);

  const lastCustomResRef = useRef(customResolution);
  useEffect(() => {
    if (!selection.ok) {
      setPrecheckReport(null);
      setPrecheckBusy(false);
      return;
    }
    if (!validCustomResolution) {
      return;
    }

    const runPrecheck = () => {
      const seq = ++precheckSeqRef.current;
      setPrecheckBusy(true);
      setPrecheckError(null);

      const request: DesktopOpticalDerivedPrecheckBatchRequest = {
        source_batch_id: sourceBatchId,
        expected_workbench_revision: expectedWorkbenchRevision,
        item_ids: selection.sceneIds,
        mode,
        ...(aoiPayload
          ? {
              aoi_bbox_wgs84: aoiPayload.aoiBboxWgs84,
              aoi_rings_wgs84: aoiPayload.aoiRingsWgs84,
            }
          : {}),
        target_resolution_m: effectiveResolutionM,
      };

      void onPrecheckRef
        .current(request)
        .then((res) => {
          if (seq !== precheckSeqRef.current) return;
          if (!res.ok) {
            setPrecheckReport(null);
            setPrecheckError(formatBridgeErrorValue(res.error));
          } else {
            setPrecheckReport(res.report);
            setPrecheckError(null);
          }
        })
        .catch((err) => {
          if (seq !== precheckSeqRef.current) return;
          setPrecheckReport(null);
          setPrecheckError(formatBridgeErrorValue(err));
        })
        .finally(() => {
          if (seq === precheckSeqRef.current) setPrecheckBusy(false);
        });
    };

    const customResChanged = lastCustomResRef.current !== customResolution;
    lastCustomResRef.current = customResolution;
    if (customResChanged) {
      const timer = window.setTimeout(runPrecheck, 140);
      return () => window.clearTimeout(timer);
    }
    runPrecheck();
  }, [
    aoiPayload,
    customResolution,
    effectiveResolutionM,
    expectedWorkbenchRevision,
    mode,
    selection,
    sourceBatchId,
    validCustomResolution,
  ]);

  const handleSelectPreset = (preset: Exclude<OpticalBandPresetId, "custom">) => {
    setBandPreset(preset);
    setSelectedBands(bandPresetOrder(family, preset));
  };

  const handleToggleBand = (bandId: string) => {
    setBandPreset("custom");
    setSelectedBands((prev) =>
      prev.includes(bandId)
        ? prev.filter((id) => id !== bandId)
        : [...prev, bandId],
    );
  };

  const resolvedTargetCrs =
    crsMode === "wgs84"
      ? "EPSG:4326"
      : crsMode === "custom"
      ? customCrs.trim().toUpperCase()
      : null;

  const validCustomCrs =
    crsMode !== "custom" || /^EPSG:\d+$/i.test(customCrs.trim());

  const canSubmit =
    selection.ok
    && Boolean(precheckReport?.ok)
    && !precheckBusy
    && !submitting
    && selectedBands.length > 0
    && Boolean(outputRoot.trim())
    && validCustomResolution
    && validCustomCrs;

  const estPixelsPerBand = precheckReport?.estimated_output_pixels_per_band ?? 0;
  const estGeoTiffBytes =
    estPixelsPerBand > 0
      ? Math.max(64 * 1024, Math.round(estPixelsPerBand * Math.max(1, selectedBands.length) * 2))
      : 0;

  const handleConfirm = async () => {
    if (!canSubmit || !selection.ok) return;
    if (isTauriDesktopHost() && isTemporarySystemDownloadPath(outputRoot)) {
      setSubmitError(TEMPORARY_DOWNLOAD_PATH_ERROR_ZH);
      return;
    }
    setSubmitting(true);
    setSubmitError(null);
    const taskId = createOpticalClientTaskId();
    if (estGeoTiffBytes > 0) {
      const perItemBytes: Record<string, number> = {};
      const perSceneEst =
        selection.sceneIds.length > 0
          ? Math.max(64 * 1024, Math.round(estGeoTiffBytes / selection.sceneIds.length))
          : estGeoTiffBytes;
      for (const sid of selection.sceneIds) {
        perItemBytes[sid] = perSceneEst;
      }
      recordSubsetSizeEstimate({
        taskId,
        totalEstimatedBytes: estGeoTiffBytes,
        sceneEstimates: perItemBytes,
      });
    }
    try {
      await onSubmit({
        request_id: createOpticalRequestId(),
        task_id: taskId,
        source_batch_id: sourceBatchId,
        expected_workbench_revision: expectedWorkbenchRevision,
        item_ids: selection.sceneIds,
        mode,
        ...(aoiPayload
          ? {
              aoi_bbox_wgs84: aoiPayload.aoiBboxWgs84,
              aoi_rings_wgs84: aoiPayload.aoiRingsWgs84,
            }
          : {}),
        band_order: selectedBands,
        target_crs: resolvedTargetCrs,
        target_resolution_m: customResolutionNum,
        resampling,
        cloud_mask: isSentinel2L1c ? "none" : cloudMask,
        mosaic_strategy: mosaicStrategy,
        max_temporal_span_days: null,
        overwrite,
        write_lineage_sidecar: isMosaic ? writeLineageSidecar : false,
        write_provenance_sidecars: writeProvenanceSidecars,
        output_root: outputRoot.trim(),
        concurrency: Math.min(8, Math.max(1, Math.trunc(initialConcurrency) || 2)),
        created_at: new Date().toISOString(),
      });
    } catch (err) {
      setSubmitError(formatBridgeErrorValue(err));
    } finally {
      setSubmitting(false);
    }
  };

  const handleBrowseDir = async () => {
    if (!onPickDirectory) return;
    const picked = await onPickDirectory();
    if (picked && picked.trim()) {
      const clean = picked.trim();
      if (isTemporarySystemDownloadPath(clean)) {
        setSubmitError(TEMPORARY_DOWNLOAD_PATH_ERROR_ZH);
        return;
      }
      setSubmitError(null);
      setOutputRoot(clean);
    }
  };

  const utmLabel = precheckReport ? String(precheckReport.recommended_utm_epsg) : "—";
  const coveragePct = precheckReport
    ? Math.round(precheckReport.combined_aoi_coverage_fraction * 1000) / 10
    : 0;

  const sceneRowsForList = useMemo(() => {
    if (precheckReport && precheckReport.scene_coverages.length > 0) {
      return precheckReport.scene_coverages.map((cov) => {
        const matched = scenes.find((s) => (s.scene_id || s.id) === cov.scene_id);
        return {
          key: cov.scene_id,
          label: formatHumanSceneLabel({
            scene_id: cov.scene_id,
            file_name: matched?.file_name,
            title: matched?.title,
          }),
          intersects: cov.intersects_aoi,
          fractionPct: Math.round(cov.aoi_coverage_fraction * 1000) / 10,
          epsg: cov.scene_epsg,
        };
      });
    }
    return scenes.map((sc) => {
      const sid = (sc.scene_id || sc.id || "").trim();
      return {
        key: sid || "scene",
        label: formatHumanSceneLabel({
          scene_id: sid,
          file_name: sc.file_name,
          title: sc.title,
        }),
        intersects: null as boolean | null,
        fractionPct: null as number | null,
        epsg: null as number | null,
      };
    });
  }, [precheckReport, scenes]);

  const isExactNativeGrid =
    !isMosaic
    && customResolution.trim() === ""
    && crsMode === "auto"
    && !precheckReport?.crosses_utm_zones
    && selectedBandsUniformNativeRes;

  const resolutionQuickOptions = [10, 20, 30, 60] as const;

  return (
    <div
      className="fixed inset-0 z-[1700] flex items-center justify-center bg-black/45 backdrop-blur-xs p-4"
      role="dialog"
      aria-modal="true"
      aria-label={isMosaic ? t("panels.opticalAoiClip.modalTitleMosaic") : t("panels.opticalAoiClip.modalTitleSingle")}
      data-testid="optical-aoi-clip-modal"
    >
      <div className="w-full max-w-4xl max-h-[90vh] overflow-hidden rounded-2xl border border-sp-line bg-sp-surface shadow-2xl flex flex-col">
        {/* Header (Unified with NisarMultiLevelSubsetModal) */}
        <div className="flex items-center justify-between px-5 py-3.5 border-b border-sp-line bg-sp-canvas shrink-0">
          <div className="flex items-center gap-2.5 min-w-0">
            <div className="w-8 h-8 rounded-lg bg-sp-hover flex items-center justify-center shrink-0">
              {isMosaic ? (
                <Layers className="w-4 h-4 text-sp-text" />
              ) : (
                <SubsetScissorsIcon className="w-4 h-4 text-sp-text" />
              )}
            </div>
            <div className="min-w-0">
              <div className="flex items-center gap-2 flex-wrap">
                <h2 className="text-sm font-semibold text-sp-text truncate">
                  {isMosaic
                    ? t("panels.opticalAoiClip.modalTitleMosaic")
                    : t("panels.opticalAoiClip.modalTitleSingle")}
                </h2>
                <span
                  data-testid="optical-clip-mode-badge"
                  className="px-1.5 py-0.5 rounded bg-sp-hover border border-sp-line text-xs font-medium text-sp-text-secondary"
                >
                  {isMosaic
                    ? t("panels.opticalAoiClip.modeBadgeMosaic", { count: scenes.length })
                    : isBatchSingleClip
                    ? `批量逐景裁剪 (${scenes.length} 景)`
                    : t("panels.opticalAoiClip.modeBadgeSingle")}
                </span>
              </div>
              <p className="text-xs text-sp-text-muted truncate">
                {t("panels.opticalAoiClip.aoiLabel")}：{aoiLabel?.trim() || (baseAoiPayload ? t("panels.opticalAoiClip.fallbackAoiName") : "可在左侧地图绘制或拖入矢量")}{" "}
                · 已选 {scenes.length} 景（按需流式读取 {isSentinel2L1c ? "JP2" : "GeoTIFF / COG"}）
              </p>
            </div>
          </div>
          <button
            type="button"
            onClick={onClose}
            disabled={submitting}
            aria-label={t("panels.opticalAoiClip.closeLabel")}
            className="p-1.5 rounded-lg text-sp-text-muted hover:text-sp-text hover:bg-sp-hover cursor-pointer disabled:opacity-50"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        {/* Two-Column Body */}
        <div className="flex-1 min-h-0 overflow-y-auto p-5 grid grid-cols-1 lg:grid-cols-[1.12fr_1fr] gap-4 text-xs">
          {/* Left Column: Interactive AOI Map + Range Readout + Volume & Precheck Card */}
          <div className="flex flex-col gap-3 min-w-0">
            <div className="rounded-xl overflow-hidden border border-sp-line h-[300px] relative">
              <AoiDrawMap
                bbox={mapBbox}
                mode="rect"
                drawActive
                minHeight={300}
                footprintGeometries={footprintGeometries}
                requestedGeometry={requestedGeometry}
                onRectDraw={(next) => {
                  setDroppedGeometry(null);
                  setCustomBbox(next);
                }}
                onGeometryImport={({ bbox, geometry }) => {
                  setCustomBbox(bbox);
                  setDroppedGeometry(geometry);
                }}
              />
            </div>

            <div className="rounded-xl border border-sp-line bg-sp-card px-3.5 py-2.5 flex flex-col gap-1.5">
              <div className="flex items-center justify-between gap-2">
                <span className="font-semibold text-sp-text">裁剪范围（可在左侧地图框选或拖入矢量）</span>
                <div className="flex items-center gap-2 shrink-0">
                  {(customBbox || droppedGeometry) && baseAoiPayload && (
                    <button
                      type="button"
                      onClick={() => {
                        setCustomBbox(null);
                        setDroppedGeometry(null);
                      }}
                      className="inline-flex items-center gap-1 rounded border border-sp-line bg-sp-surface px-1.5 py-0.5 text-xs text-sp-text-secondary hover:bg-sp-hover cursor-pointer"
                    >
                      <RotateCcw className="w-3 h-3" />
                      <span>恢复原研究区</span>
                    </button>
                  )}
                  <span className="font-mono text-sp-text-muted whitespace-nowrap tabular-nums">
                    {precheckReport ? `覆盖率 ${coveragePct}%` : "等待预检"}
                  </span>
                </div>
              </div>
              <div className="grid grid-cols-2 sm:grid-cols-4 gap-2 font-mono text-xs tabular-nums text-sp-text-secondary">
                <span className="whitespace-nowrap">{formatSignedLon(mapBbox.west)}</span>
                <span className="whitespace-nowrap">{formatSignedLon(mapBbox.east)}</span>
                <span className="whitespace-nowrap">{formatSignedLat(mapBbox.south)}</span>
                <span className="whitespace-nowrap">{formatSignedLat(mapBbox.north)}</span>
              </div>
            </div>

            {/* Volume & Precheck Card (Matching NISAR card structure) */}
            <div
              data-testid="optical-precheck-panel"
              className="rounded-xl border border-sp-line bg-sp-card px-3.5 py-2.5 space-y-2.5"
            >
              <div className="flex items-center justify-between gap-2">
                <span className="font-semibold text-sp-text">按 AOI 占比实时输出预估与预检</span>
                <span className="font-mono text-xs text-sp-text-muted tabular-nums">
                  {effectiveResolutionM}m · EPSG:{utmLabel}
                </span>
              </div>

              <div className="grid grid-cols-2 gap-2">
                <div className="rounded-lg bg-sp-surface border border-sp-line px-2.5 py-1.5">
                  <div className="text-sp-text-muted">预计单波段输出像元</div>
                  <div
                    data-testid="optical-est-pixels"
                    className="font-mono font-semibold text-sp-text tabular-nums"
                  >
                    {estPixelsPerBand > 0 ? estPixelsPerBand.toLocaleString("en-US") : "—"}
                  </div>
                </div>
                <div className="rounded-lg bg-sp-surface border border-sp-line px-2.5 py-1.5">
                  <div className="text-sp-text-muted">预估 GeoTIFF 输出体积</div>
                  <div
                    data-testid="optical-est-bytes"
                    className="font-mono font-semibold text-sp-text tabular-nums"
                  >
                    {estGeoTiffBytes > 0 ? `~${formatBytes(estGeoTiffBytes)}` : "—"}
                  </div>
                </div>
              </div>

              {!selection.ok && (
                <div className="rounded-lg border border-sp-danger-text/30 bg-sp-danger-bg px-2.5 py-2 text-sp-danger-text">
                  {t("panels.opticalAoiClip.blockedUnsupportedSource")} ({selection.blockCode})
                </div>
              )}

              {precheckBusy && (
                <div className="flex items-center gap-2 text-sp-text-secondary">
                  <Loader2 className="w-3.5 h-3.5 animate-spin" />
                  <span>{t("panels.opticalAoiClip.precheckLoading")}</span>
                </div>
              )}

              {precheckError && !precheckBusy && (
                <div className="text-sp-danger-text flex items-start gap-1.5">
                  <AlertTriangle className="w-3.5 h-3.5 shrink-0 mt-0.5" />
                  <span>{t("panels.opticalAoiClip.precheckFailed", { error: precheckError })}</span>
                </div>
              )}

              {precheckReport && !precheckBusy && (
                <>
                  <div className="flex flex-wrap items-center gap-x-3 gap-y-1 font-medium text-sp-text">
                    <span className="inline-flex items-center gap-1">
                      {precheckReport.ok ? (
                        <CheckCircle2 className="w-3.5 h-3.5 text-emerald-600 dark:text-emerald-400" />
                      ) : (
                        <AlertTriangle className="w-3.5 h-3.5 text-sp-danger-text" />
                      )}
                      <span>{t("panels.opticalAoiClip.precheckCoverage", { fraction: String(coveragePct) })}</span>
                    </span>
                    <span className="text-sp-text-secondary">
                      {t("panels.opticalAoiClip.precheckRecommendedCrs", {
                        epsg: String(precheckReport.recommended_utm_epsg),
                      })}
                    </span>
                    {isMosaic && precheckReport.temporal_span_days !== null &&
                      precheckReport.temporal_span_days !== undefined && (
                        <span
                          className={cn(
                            precheckReport.within_near_coincident_limit
                              ? "text-sp-text-secondary"
                              : "text-sp-warning-text font-medium",
                          )}
                        >
                          {t("panels.opticalAoiClip.precheckTemporalSpan", {
                            span: precheckReport.temporal_span_days.toFixed(1),
                          })}{" "}
                          {precheckReport.within_near_coincident_limit
                            ? t("panels.opticalAoiClip.precheckTemporalOk")
                            : "（时相跨度 >16 天，请注意地物季节变化）"}
                        </span>
                      )}
                  </div>

                  {precheckReport.crosses_utm_zones && (
                    <div
                      data-testid="optical-cross-utm-warning"
                      className="rounded-lg border border-amber-500/40 bg-amber-500/10 px-2.5 py-2 text-sp-warning-text"
                    >
                      <div className="font-semibold">
                        {t("panels.opticalAoiClip.precheckCrossUtmTitle", {
                          zones: precheckReport.distinct_utm_zones.join(", "),
                        })}
                      </div>
                      <div className="mt-0.5 opacity-90">
                        {t("panels.opticalAoiClip.precheckCrossUtmHint", {
                          epsg: String(precheckReport.recommended_utm_epsg),
                        })}
                      </div>
                    </div>
                  )}

                  {precheckReport.blocking_reasons.length > 0 && (
                    <div
                      data-testid="optical-precheck-blocking"
                      className="rounded-lg border border-sp-danger-text/40 bg-red-500/10 px-2.5 py-2 text-sp-danger-text space-y-1"
                    >
                      <div className="font-semibold">{t("panels.opticalAoiClip.precheckBlockingTitle")}</div>
                      {precheckReport.blocking_reasons.map((reason) => {
                        const cleanedReason = reason.includes("OPTICAL_OUTPUT_PIXEL_LIMIT")
                          ? reason.replace(/^\[?OPTICAL_OUTPUT_PIXEL_LIMIT\]?\s*:?\s*/, "")
                          : reason;
                        const formattedReason = reason.includes("OPTICAL_OUTPUT_PIXEL_LIMIT")
                          ? `${cleanedReason.replace(/\b(\d{5,})\b/g, (digits) => Number(digits).toLocaleString("en-US"))}（可在右侧调大输出像元分辨率至 20m / 60m，或在左侧地图框选更小研究区范围）`
                          : cleanedReason;
                        return (
                          <div key={reason} className="leading-relaxed break-words">
                            {formattedReason}
                          </div>
                        );
                      })}
                    </div>
                  )}

                  {precheckReport.warnings.length > 0 && (
                    <div className="rounded-lg border border-sp-line bg-sp-surface px-2.5 py-2 text-sp-text-secondary space-y-0.5">
                      <div className="font-semibold text-sp-text">
                        {t("panels.opticalAoiClip.precheckWarningsTitle")}
                      </div>
                      {precheckReport.warnings.map((w) => (
                        <div key={w}>{w}</div>
                      ))}
                    </div>
                  )}
                </>
              )}
            </div>
          </div>

          {/* Right Column: Multi-scene Delivery Mode + Band Selection + Projection/Resolution + Mosaic + Visible Scene List + Output Dir */}
          <div className="flex flex-col gap-3 min-w-0">
            {scenes.length >= 2 && (
              <div
                data-testid="optical-multi-scene-mode-card"
                className="rounded-xl border border-sp-line bg-sp-card p-3.5 space-y-2"
              >
                <div className="font-semibold text-sp-text">多景输出模式</div>
                <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
                  <label className="flex items-start gap-2 rounded-sp-sm border border-sp-line bg-sp-surface p-2 cursor-pointer">
                    <input
                      type="radio"
                      name="optical-multi-scene-mode"
                      data-testid="optical-mode-batch-single"
                      checked={multiSceneMode === "batch_single_scene_clip"}
                      onChange={() => setMultiSceneMode("batch_single_scene_clip")}
                    />
                    <span className="min-w-0">
                      <span className="block font-medium text-sp-text">逐景分别裁剪（默认）</span>
                      <span className="block text-sp-text-muted">每景输出独立裁剪 GeoTIFF（共 {scenes.length} 个文件）</span>
                    </span>
                  </label>
                  <label className="flex items-start gap-2 rounded-sp-sm border border-sp-line bg-sp-surface p-2 cursor-pointer">
                    <input
                      type="radio"
                      name="optical-multi-scene-mode"
                      data-testid="optical-mode-mosaic"
                      checked={multiSceneMode === "near_coincident_spatial_mosaic"}
                      onChange={() => setMultiSceneMode("near_coincident_spatial_mosaic")}
                    />
                    <span className="min-w-0">
                      <span className="block font-medium text-sp-text">近同期空间镶嵌</span>
                      <span className="block text-sp-text-muted">仅真实观测像元合成 1 张 GeoTIFF</span>
                    </span>
                  </label>
                </div>
              </div>
            )}

            {/* Band Selection Card */}
            <div className="rounded-xl border border-sp-line bg-sp-card p-3.5 space-y-2">
              <div className="flex items-center justify-between gap-2">
                <span className="font-semibold text-sp-text">{t("panels.opticalAoiClip.bandPresetLabel")}</span>
                <span className="font-mono text-xs text-sp-text-muted">
                  已选 {selectedBands.length}/{bandOptions.length} 波段
                </span>
              </div>
              <div className="flex flex-wrap gap-1.5">
                {(
                  [
                    ["true_color", t("panels.opticalAoiClip.presetTrueColor")],
                    ["false_color_nir", t("panels.opticalAoiClip.presetFalseColorNir")],
                    ["swir", t("panels.opticalAoiClip.presetSwir")],
                    ["analysis_full", t("panels.opticalAoiClip.presetAnalysisFull")],
                  ] as const
                ).map(([presetId, label]) => (
                  <button
                    key={presetId}
                    type="button"
                    data-testid={`band-preset-${presetId}`}
                    onClick={() => handleSelectPreset(presetId)}
                    className={cn(
                      "px-2.5 py-1 rounded-sp-sm border font-medium transition-colors cursor-pointer",
                      bandPreset === presetId
                        ? "border-sp-text bg-sp-text text-sp-canvas"
                        : "border-sp-line bg-sp-surface text-sp-text-secondary hover:bg-sp-hover",
                    )}
                  >
                    {label}
                  </button>
                ))}
              </div>
              <div className="text-sp-text-muted">
                {t("panels.opticalAoiClip.selectedBandsLabel")}：{" "}
                <span className="font-mono text-sp-text">{selectedBands.join(", ") || "—"}</span>
              </div>
              <div className="flex flex-wrap gap-1.5">
                {bandOptions.map((band) => {
                  const checked = selectedBands.includes(band.id);
                  return (
                    <label
                      key={band.id}
                      className={cn(
                        "inline-flex items-center gap-1.5 px-2 py-1 rounded-sp-sm border cursor-pointer select-none",
                        checked
                          ? "border-sp-text bg-sp-surface text-sp-text font-medium"
                          : "border-sp-line bg-sp-surface/60 text-sp-text-muted hover:bg-sp-hover",
                      )}
                    >
                      <input
                        type="checkbox"
                        checked={checked}
                        onChange={() => handleToggleBand(band.id)}
                        className="rounded border-sp-line"
                      />
                      <span className="font-mono">{band.id}</span>
                      <span className="opacity-70">{band.resolutionM}m</span>
                    </label>
                  );
                })}
              </div>
            </div>

            {/* Projection, Resolution & Cloud Mask Card */}
            <div className="rounded-xl border border-sp-line bg-sp-card p-3.5 space-y-3">
              <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
                <div className="space-y-1.5">
                  <div className="font-semibold text-sp-text">{t("panels.opticalAoiClip.targetCrsLabel")}</div>
                  <div className="space-y-1">
                    <label className="flex items-center gap-1.5 cursor-pointer">
                      <input
                        type="radio"
                        name="optical-target-crs"
                        checked={crsMode === "auto"}
                        onChange={() => setCrsMode("auto")}
                      />
                      <span>{t("panels.opticalAoiClip.targetCrsAuto", { epsg: utmLabel })}</span>
                    </label>
                    <label className="flex items-center gap-1.5 cursor-pointer">
                      <input
                        type="radio"
                        name="optical-target-crs"
                        checked={crsMode === "wgs84"}
                        onChange={() => setCrsMode("wgs84")}
                      />
                      <span>{t("panels.opticalAoiClip.targetCrsWgs84")}</span>
                    </label>
                    <label className="flex items-center gap-1.5 cursor-pointer">
                      <input
                        type="radio"
                        name="optical-target-crs"
                        checked={crsMode === "custom"}
                        onChange={() => setCrsMode("custom")}
                      />
                      <span>{t("panels.opticalAoiClip.targetCrsCustom")}</span>
                    </label>
                    {crsMode === "custom" && (
                      <input
                        type="text"
                        value={customCrs}
                        onChange={(e) => setCustomCrs(e.target.value)}
                        placeholder={t("panels.opticalAoiClip.targetCrsPlaceholder")}
                        className="w-full h-8 px-2.5 rounded-sp-sm border border-sp-line bg-sp-surface font-mono text-sp-text"
                      />
                    )}
                  </div>
                </div>

                <div className="space-y-2">
                  <div className="space-y-1">
                    <div className="flex items-center justify-between gap-2">
                      <label className="block font-semibold text-sp-text">
                        {t("panels.opticalAoiClip.resamplingLabel")}
                      </label>
                      {isExactNativeGrid && (
                        <span className="text-sp-text-muted whitespace-nowrap">原生网格直读</span>
                      )}
                    </div>
                    <SpSelect
                      aria-label={t("panels.opticalAoiClip.resamplingLabel")}
                      data-testid="optical-resampling-select"
                      value={resampling}
                      disabled={isExactNativeGrid}
                      onChange={(v) => setResampling(v as DesktopOpticalResampling)}
                      size="sm"
                      options={[
                        { value: "bilinear", label: t("panels.opticalAoiClip.resamplingBilinear") },
                        { value: "nearest", label: t("panels.opticalAoiClip.resamplingNearest") },
                        { value: "cubic", label: t("panels.opticalAoiClip.resamplingCubic") },
                      ]}
                    />
                  </div>
                  <div className="space-y-1">
                    <div className="flex items-center justify-between gap-2 flex-wrap">
                      <label className="block font-semibold text-sp-text whitespace-nowrap">
                        {t("panels.opticalAoiClip.targetResolutionLabel")}
                      </label>
                      <div className="flex items-center gap-1 flex-nowrap">
                        <button
                          type="button"
                          onClick={() => setCustomResolution("")}
                          className={cn(
                            "px-1.5 py-0.5 rounded border text-xs cursor-pointer whitespace-nowrap",
                            customResolution === ""
                              ? "border-sp-text bg-sp-text text-sp-canvas"
                              : "border-sp-line bg-sp-surface text-sp-text-muted hover:bg-sp-hover",
                          )}
                        >
                          原生
                        </button>
                        {resolutionQuickOptions.map((resVal) => (
                          <button
                            key={resVal}
                            type="button"
                            data-testid={`optical-res-quick-${resVal}`}
                            onClick={() => setCustomResolution(String(resVal))}
                            className={cn(
                              "px-1.5 py-0.5 rounded border text-xs font-mono cursor-pointer whitespace-nowrap",
                              customResolution.trim() === String(resVal)
                                ? "border-sp-text bg-sp-text text-sp-canvas"
                                : "border-sp-line bg-sp-surface text-sp-text-muted hover:bg-sp-hover",
                            )}
                          >
                            {resVal}
                          </button>
                        ))}
                      </div>
                    </div>
                    <input
                      type="number"
                      data-testid="optical-target-resolution-input"
                      min={1}
                      step="any"
                      value={customResolution}
                      onChange={(e) => setCustomResolution(e.target.value)}
                      placeholder={t("panels.opticalAoiClip.targetResolutionAuto")}
                      className="w-full h-8 px-2.5 rounded-sp-sm border border-sp-line bg-sp-surface text-sp-text font-mono"
                    />
                  </div>
                </div>
              </div>

              <div className="pt-2 border-t border-sp-line space-y-1.5">
                <div className="font-semibold text-sp-text">{t("panels.opticalAoiClip.cloudMaskLabel")}</div>
                <div className="flex flex-col gap-1.5">
                  <label className="flex items-center gap-1.5 cursor-pointer">
                    <input
                      type="radio"
                      name="optical-cloud-mask"
                      data-testid="cloud-mask-none"
                      checked={(isSentinel2L1c ? "none" : cloudMask) === "none"}
                      onChange={() => setCloudMask("none")}
                    />
                    <span>{t("panels.opticalAoiClip.cloudMaskNone")}</span>
                  </label>
                  <label
                    className={cn(
                      "flex items-center gap-1.5",
                      isSentinel2L1c ? "opacity-50 cursor-not-allowed" : "cursor-pointer",
                    )}
                  >
                    <input
                      type="radio"
                      name="optical-cloud-mask"
                      data-testid="cloud-mask-nodata"
                      checked={!isSentinel2L1c && cloudMask === "cloud_to_nodata"}
                      disabled={isSentinel2L1c}
                      onChange={() => setCloudMask("cloud_to_nodata")}
                    />
                    <span>{t("panels.opticalAoiClip.cloudMaskToNodata")}</span>
                  </label>
                </div>
                {isSentinel2L1c && (
                  <p className="text-xs text-sp-text-muted">
                    Sentinel-2 L1C 为大气表观反射率产品，不含 L2A SCL 云掩膜波段，因此保留原始观测像元。
                  </p>
                )}
              </div>
            </div>

            {/* Mosaic options */}
            {isMosaic && (
              <div
                data-testid="optical-mosaic-options"
                className="space-y-2 rounded-xl border border-sp-line bg-sp-card p-3.5"
              >
                <div className="font-semibold text-sp-text">
                  {t("panels.opticalAoiClip.mosaicSectionTitle")}
                </div>
                <div className="flex flex-col gap-1.5">
                  <label className="flex items-center gap-1.5 cursor-pointer">
                    <input
                      type="radio"
                      name="optical-mosaic-strategy"
                      data-testid="mosaic-strategy-quality"
                      checked={mosaicStrategy === "quality_first"}
                      onChange={() => setMosaicStrategy("quality_first")}
                    />
                    <span>{t("panels.opticalAoiClip.mosaicStrategyQualityFirst")}</span>
                  </label>
                  <label className="flex items-center gap-1.5 cursor-pointer">
                    <input
                      type="radio"
                      name="optical-mosaic-strategy"
                      data-testid="mosaic-strategy-latest"
                      checked={mosaicStrategy === "latest_first"}
                      onChange={() => setMosaicStrategy("latest_first")}
                    />
                    <span>{t("panels.opticalAoiClip.mosaicStrategyLatestFirst")}</span>
                  </label>
                </div>
                <label className="flex items-center gap-1.5 cursor-pointer pt-1">
                  <input
                    type="checkbox"
                    checked={writeLineageSidecar}
                    onChange={(e) => setWriteLineageSidecar(e.target.checked)}
                  />
                  <span>{t("panels.opticalAoiClip.writeLineageLabel")}</span>
                </label>
              </div>
            )}

            {/* Selected Scenes List (Visible block-scroll container, never squished in flexbox) */}
            <div className="rounded-xl border border-sp-line bg-sp-card p-3.5 flex flex-col gap-1.5">
              <div className="flex items-center justify-between gap-2">
                <span className="font-semibold text-sp-text">
                  {t("panels.opticalAoiClip.sceneCoverageTitle", { count: sceneRowsForList.length })}
                </span>
              </div>
              <div
                data-testid="optical-clip-scene-list"
                className="max-h-[120px] overflow-y-auto space-y-1 pr-1"
              >
                {sceneRowsForList.map((row) => (
                  <div
                    key={row.key}
                    className="flex items-center justify-between gap-2 min-h-[26px] rounded border border-sp-line bg-sp-surface px-2 py-1"
                    title={row.label}
                  >
                    <span className="min-w-0 flex-1 font-mono text-sp-text truncate">{row.label}</span>
                    {row.intersects !== null && row.fractionPct !== null && (
                      <span
                        className={cn(
                          "shrink-0 tabular-nums text-xs",
                          row.intersects ? "text-sp-text-secondary" : "text-sp-danger-text",
                        )}
                      >
                        {row.intersects
                          ? t("panels.opticalAoiClip.sceneIntersects", {
                              fraction: String(row.fractionPct),
                            })
                          : t("panels.opticalAoiClip.sceneDisjoint")}
                        {row.epsg ? ` · EPSG:${row.epsg}` : ""}
                      </span>
                    )}
                  </div>
                ))}
              </div>
            </div>

            {/* Dual-Mode Output Directory Card */}
            <div className="rounded-xl border border-sp-line bg-sp-card p-3.5 space-y-2">
              <label className="block font-semibold text-sp-text">
                {t("panels.opticalAoiClip.outputDirLabel")}
              </label>
              <div className="flex items-center gap-1.5">
                <input
                  type="text"
                  data-testid="optical-clip-output-dir"
                  value={outputRoot}
                  onChange={(e) => {
                    setOutputRoot(e.target.value);
                    setSubmitError(null);
                  }}
                  placeholder={t("panels.opticalAoiClip.outputDirPlaceholder")}
                  className="h-8 flex-1 min-w-0 px-2.5 rounded-sp-sm border border-sp-line bg-sp-surface font-mono text-sp-text outline-none"
                />
                {onPickDirectory && (
                  <button
                    type="button"
                    onClick={() => void handleBrowseDir()}
                    className="h-8 px-2.5 rounded-sp-sm border border-sp-line bg-sp-surface text-sp-text hover:bg-sp-hover inline-flex items-center gap-1 cursor-pointer shrink-0"
                  >
                    <FolderOpen className="w-3.5 h-3.5" />
                    <span>{t("panels.opticalAoiClip.browseDirBtn")}</span>
                  </button>
                )}
              </div>
              {outputRoot.trim() === "" && (
                <p className="text-xs text-sp-warning-text">
                  请先填写或浏览选择非临时输出目录，完成后即可创建裁剪 / 镶嵌任务。
                </p>
              )}
              <label className="flex items-center gap-1.5 cursor-pointer text-sp-text-secondary">
                <input
                  type="checkbox"
                  checked={overwrite}
                  onChange={(e) => setOverwrite(e.target.checked)}
                />
                <span>{t("panels.opticalAoiClip.overwriteLabel")}</span>
              </label>
              <label className="flex items-center gap-1.5 cursor-pointer text-sp-text-secondary">
                <input
                  type="checkbox"
                  data-testid="optical-write-provenance-checkbox"
                  checked={writeProvenanceSidecars}
                  onChange={(e) => setWriteProvenanceSidecars(e.target.checked)}
                />
                <span>{t("panels.opticalAoiClip.writeProvenanceSidecarsLabel")}</span>
              </label>
            </div>

            {submitError && (
              <div className="rounded-xl border border-sp-danger-text/30 bg-sp-danger-bg px-3 py-2 text-sp-danger-text flex items-start gap-1.5">
                <AlertTriangle className="w-3.5 h-3.5 shrink-0 mt-0.5" />
                <span>{submitError}</span>
              </div>
            )}
          </div>
        </div>

        {/* Footer (Unified with NisarMultiLevelSubsetModal) */}
        <div className="flex items-center justify-between gap-3 px-5 py-3.5 border-t border-sp-line bg-sp-canvas shrink-0">
          <div className="flex items-center gap-1.5 min-w-0 text-xs text-sp-text-muted">
            <ShieldCheck className="w-3.5 h-3.5 shrink-0 text-sp-accent" />
            <span className="truncate">{t("panels.opticalAoiClip.provenanceNotice")}</span>
          </div>
          <div className="flex items-center gap-2 shrink-0">
            <button
              type="button"
              onClick={onClose}
              disabled={submitting}
              className="h-8 px-3 rounded-sp-sm border border-sp-line bg-sp-surface text-xs font-medium text-sp-text hover:bg-sp-hover cursor-pointer disabled:opacity-50"
            >
              {t("panels.opticalAoiClip.cancelBtn")}
            </button>
            <button
              type="button"
              data-testid="btn-confirm-optical-aoi-clip"
              disabled={!canSubmit}
              onClick={() => void handleConfirm()}
              className="h-8 px-3.5 rounded-sp-sm bg-sp-accent-fill text-white text-xs font-semibold hover:opacity-90 disabled:opacity-50 disabled:cursor-not-allowed inline-flex items-center gap-1.5 cursor-pointer"
            >
              {submitting ? (
                <Loader2 className="w-3.5 h-3.5 animate-spin" />
              ) : isMosaic ? (
                <Layers className="w-3.5 h-3.5" />
              ) : (
                <SubsetScissorsIcon className="w-3.5 h-3.5" />
              )}
              <span>
                {submitting
                  ? t("panels.opticalAoiClip.submittingBtn")
                  : t("panels.opticalAoiClip.submitBtn")}
              </span>
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
