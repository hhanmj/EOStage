import { useEffect, useMemo, useState } from "react";
import {
  AlertTriangle,
  CheckCircle2,
  FolderOpen,
  Layers,
  Loader2,
  Radar,
  X,
} from "lucide-react";
import { cn } from "@/lib/utils";
import {
  pickDirectory,
  queryOpenGeodata,
  type Bbox,
  type Json,
} from "@/lib/bridge";
import { AoiDrawMap } from "@/components/AoiDrawMap";
import { SpSelect } from "@/components/ui/SpSelect";
import { emitTopToast } from "@/components/ui/TopCenterToast";
import { recordSubsetSizeEstimate } from "@/lib/subsetSizeEstimateRegistry";
import { formatBytes } from "@/shell/api/agentStreamLogic";
import { SubsetScissorsIcon } from "./SubsetScissorsIcon";
import { formatBridgeErrorValue } from "@/lib/bridgeErrorFormat";
import { formatSignedLat, formatSignedLon } from "@/lib/geoCoords";
import {
  formatHumanSceneLabel,
  isTemporarySystemDownloadPath,
  TEMPORARY_DOWNLOAD_PATH_ERROR_ZH,
} from "@/lib/storagePathValidation";
import type { SceneRow } from "@/shell/api/types";

export interface NisarMultiLevelSubsetModalProps {
  productLevel: string;
  band: "L" | "S";
  scenes: readonly SceneRow[];
  aoiBbox: Bbox | null;
  aoiLabel: string;
  initialOutputRoot: string;
  batchId?: string | null;
  batchRevision?: number;
  rslcSelections?: ReadonlyArray<{ frequency: "A" | "B"; polarizations: string[] }>;
  onPreflightRslcBatch?: (opts: {
    aoiGeometry: Json;
    targetSceneIds: string[];
  }) => Promise<{
    eligibleCount: number;
    excludedCount: number;
    commonFrequencies: string[];
    commonPolarizations: string[];
  }>;
  onSubmitRslcBatch?: (opts: {
    frequency: "A" | "B";
    polarizations: string[];
    aoiBbox: Bbox;
    aoiGeometry: Json;
    outputRoot: string;
    estimatedSubsetBytes: number;
    targetSceneIds: string[];
  }) => Promise<void>;
  onClose: () => void;
  onSubmitted: () => Promise<void> | void;
}

const DEFAULT_LEVEL_FULL_BYTES: Record<string, number> = {
  GSLC: 4_500_000_000,
  GCOV: 1_800_000_000,
  GUNW: 1_200_000_000,
  GOFF: 650_000_000,
  RIFG: 1_600_000_000,
  RUNW: 1_600_000_000,
  ROFF: 900_000_000,
  RSLC: 2_800_000_000,
};

const LEVEL_DATASET_CHOICES: Record<
  string,
  Array<{ id: string; label: string; defaultChecked: boolean }>
> = {
  RSLC: [
    { id: "HH", label: "HH 同极化单视复影像 (SLC)", defaultChecked: true },
    { id: "HV", label: "HV 交叉极化复影像 (SLC)", defaultChecked: true },
    { id: "VH", label: "VH 交叉极化复影像 (SLC)", defaultChecked: false },
    { id: "VV", label: "VV 同极化单视复影像 (SLC)", defaultChecked: false },
  ],
  GSLC: [
    { id: "HH", label: "HH 复影像 (Complex SLC)", defaultChecked: true },
    { id: "HV", label: "HV 交叉极化", defaultChecked: true },
    { id: "VH", label: "VH 交叉极化", defaultChecked: true },
    { id: "VV", label: "VV 同极化", defaultChecked: true },
    { id: "rtcGammaToSigmaFactor", label: "RTC Gamma/Sigma 系数", defaultChecked: false },
    { id: "mask", label: "有效掩膜 (Mask)", defaultChecked: false },
  ],
  GCOV: [
    { id: "HHHH", label: "HHHH 后向散射强度", defaultChecked: true },
    { id: "VVVV", label: "VVVV 后向散射强度", defaultChecked: true },
    { id: "HVHV", label: "HVHV 交叉极化强度", defaultChecked: true },
    { id: "VHVH", label: "VHVH 交叉极化强度", defaultChecked: true },
    { id: "HHHV", label: "HHHV 协方差交叉项", defaultChecked: false },
    { id: "HHVV", label: "HHVV 协方差交叉项", defaultChecked: false },
    { id: "numberOfLooks", label: "视数图 (Number of Looks)", defaultChecked: false },
  ],
  GUNW: [
    { id: "unwrappedPhase", label: "解缠相位 (unwrappedPhase)", defaultChecked: true },
    { id: "coherenceMagnitude", label: "相干性幅度 (coherenceMagnitude)", defaultChecked: true },
    { id: "connectedComponents", label: "连通域编号 (connectedComponents)", defaultChecked: true },
    { id: "ionospherePhaseScreen", label: "电离层相位屏", defaultChecked: false },
  ],
  RUNW: [
    { id: "unwrappedPhase", label: "雷达网格解缠相位", defaultChecked: true },
    { id: "coherenceMagnitude", label: "相干性幅度", defaultChecked: true },
    { id: "connectedComponents", label: "连通域编号", defaultChecked: true },
  ],
  RIFG: [
    { id: "wrappedInterferogram", label: "缠绕复干涉图", defaultChecked: true },
    { id: "coherenceMagnitude", label: "相干性幅度", defaultChecked: true },
  ],
  GOFF: [
    { id: "alongTrackOffset", label: "方位向偏移量 (Along-Track)", defaultChecked: true },
    { id: "slantRangeOffset", label: "斜距向偏移量 (Slant-Range)", defaultChecked: true },
    { id: "correlationSurfacePeak", label: "相关峰置信度", defaultChecked: true },
  ],
  ROFF: [
    { id: "alongTrackOffset", label: "雷达网格方位向偏移", defaultChecked: true },
    { id: "slantRangeOffset", label: "雷达网格斜距向偏移", defaultChecked: true },
    { id: "correlationSurfacePeak", label: "相关峰置信度", defaultChecked: true },
  ],
};

function bboxFromGeometryOrMeta(scene: SceneRow): Bbox | null {
  const meta = (scene.native_metadata || {}) as Record<string, unknown>;
  const rawBbox = meta.bbox;
  if (
    Array.isArray(rawBbox) &&
    rawBbox.length === 4 &&
    rawBbox.every((n) => typeof n === "number" && Number.isFinite(n))
  ) {
    return {
      west: Math.min(rawBbox[0], rawBbox[2]),
      south: Math.min(rawBbox[1], rawBbox[3]),
      east: Math.max(rawBbox[0], rawBbox[2]),
      north: Math.max(rawBbox[1], rawBbox[3]),
      crs: "EPSG:4326",
    };
  }
  const geom = scene.footprint_geojson as { coordinates?: unknown } | null;
  const ring = Array.isArray(geom?.coordinates) ? (geom.coordinates[0] as unknown) : null;
  if (Array.isArray(ring) && ring.length >= 3) {
    let w = Infinity;
    let s = Infinity;
    let e = -Infinity;
    let n = -Infinity;
    for (const pt of ring) {
      if (Array.isArray(pt) && pt.length >= 2) {
        const lon = Number(pt[0]);
        const lat = Number(pt[1]);
        if (Number.isFinite(lon) && Number.isFinite(lat)) {
          w = Math.min(w, lon);
          s = Math.min(s, lat);
          e = Math.max(e, lon);
          n = Math.max(n, lat);
        }
      }
    }
    if (Number.isFinite(w) && Number.isFinite(s)) {
      return { west: w, south: s, east: e, north: n, crs: "EPSG:4326" };
    }
  }
  return null;
}

function intersectionAreaRatio(aoi: Bbox, footprint: Bbox | null): number {
  if (!footprint) {
    return 0.25;
  }
  const fpArea = Math.max(
    1e-9,
    (footprint.east - footprint.west) * (footprint.north - footprint.south),
  );
  const iw = Math.max(0, Math.min(aoi.east, footprint.east) - Math.max(aoi.west, footprint.west));
  const ih = Math.max(0, Math.min(aoi.north, footprint.north) - Math.max(aoi.south, footprint.south));
  const inter = iw * ih;
  if (inter <= 0) {
    return 0;
  }
  return Math.max(0.01, Math.min(1.0, inter / fpArea));
}

function isImageOrPreviewLocator(url: string, role?: string | null, mediaType?: string | null): boolean {
  const normRole = (role || "").trim().toLowerCase();
  if (
    normRole === "thumbnail" ||
    normRole === "overview" ||
    normRole === "preview" ||
    normRole === "rendered_preview"
  ) {
    return true;
  }
  const normMedia = (mediaType || "").trim().toLowerCase();
  if (normMedia.startsWith("image/")) {
    return true;
  }
  const pathOnly = url.trim().toLowerCase().split("?")[0].split("#")[0];
  return /\.(jpe?g|png|webp|gif|bmp|svg)$/.test(pathOnly);
}

export function extractNisarSubsetDownloadUrl(scene: SceneRow): string {
  const meta = (scene.native_metadata || {}) as Record<string, unknown>;
  const candidates = [scene.download_url, meta.data_url, meta.href, meta.download_url];
  for (const c of candidates) {
    if (
      typeof c === "string" &&
      c.trim().startsWith("http") &&
      !isImageOrPreviewLocator(c)
    ) {
      return c.trim();
    }
  }
  if (Array.isArray(scene.assets)) {
    const h5Asset = scene.assets.find(
      (a) =>
        typeof a?.url === "string" &&
        a.url.trim().startsWith("http") &&
        !isImageOrPreviewLocator(a.url, a.role, a.media_type),
    );
    if (h5Asset?.url) {
      return h5Asset.url.trim();
    }
  }
  return "";
}

export function NisarMultiLevelSubsetModal({
  productLevel,
  band,
  scenes,
  aoiBbox,
  aoiLabel,
  initialOutputRoot,
  rslcSelections,
  onPreflightRslcBatch,
  onSubmitRslcBatch,
  onClose,
  onSubmitted,
}: NisarMultiLevelSubsetModalProps) {
  const level = productLevel.toUpperCase();
  const baseChoices = LEVEL_DATASET_CHOICES[level] || LEVEL_DATASET_CHOICES.GSLC;

  const fallbackBbox = useMemo<Bbox>(() => {
    if (aoiBbox) return aoiBbox;
    const firstFp = scenes.length > 0 ? bboxFromGeometryOrMeta(scenes[0]) : null;
    if (firstFp) return firstFp;
    return { west: 117.15, south: 31.78, east: 117.35, north: 31.95, crs: "EPSG:4326" };
  }, [aoiBbox, scenes]);

  const initialRslcFreq =
    rslcSelections && rslcSelections.length > 0 && rslcSelections[0].frequency === "B"
      ? "frequencyB"
      : "frequencyA";
  const [editBbox, setEditBbox] = useState<Bbox>(fallbackBbox);
  const [frequency, setFrequency] = useState<"frequencyA" | "frequencyB">(initialRslcFreq);

  const layerChoices = useMemo(() => {
    if (level !== "RSLC" || !rslcSelections || rslcSelections.length === 0) {
      return baseChoices;
    }
    const freqCode = frequency === "frequencyB" ? "B" : "A";
    const match = rslcSelections.find((sel) => sel.frequency === freqCode) ?? rslcSelections[0];
    if (!match || match.polarizations.length === 0) {
      return baseChoices;
    }
    const baseById = new Map(baseChoices.map((c) => [c.id, c]));
    return match.polarizations.map((pol, index) => {
      const known = baseById.get(pol);
      return known
        ? known
        : { id: pol, label: `${pol} 极化复影像 (SLC)`, defaultChecked: index < 2 };
    });
  }, [baseChoices, frequency, level, rslcSelections]);

  const [selectedLayers, setSelectedLayers] = useState<string[]>(() =>
    layerChoices.filter((c) => c.defaultChecked).map((c) => c.id),
  );
  useEffect(() => {
    setSelectedLayers((prev) => {
      const validSet = new Set(layerChoices.map((c) => c.id));
      const kept = prev.filter((id) => validSet.has(id));
      if (kept.length > 0) return kept;
      const defaults = layerChoices.filter((c) => c.defaultChecked).map((c) => c.id);
      return defaults.length > 0 ? defaults : layerChoices.slice(0, 1).map((c) => c.id);
    });
  }, [layerChoices]);
  const [outputDir, setOutputDir] = useState<string>(initialOutputRoot);
  const [preflightBusy, setPreflightBusy] = useState(false);
  const [preflightInfo, setPreflightInfo] = useState<{
    gridKind: string;
    epsg: number | null;
    sourceDims: [number, number];
    subsetDims: [number, number];
    estimatedBytes: number;
    availableDatasets: string[];
  } | null>(null);
  const [rslcPreflightSummary, setRslcPreflightSummary] = useState<{
    eligibleCount: number;
    excludedCount: number;
    commonFrequencies: string[];
    commonPolarizations: string[];
  } | null>(null);
  const [submitBusy, setSubmitBusy] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [resultNote, setResultNote] = useState<string | null>(null);

  const firstH5Url = useMemo(
    () => (scenes.length > 0 ? extractNisarSubsetDownloadUrl(scenes[0]) : ""),
    [scenes],
  );
  const canPreflightRslc = level === "RSLC" && Boolean(onPreflightRslcBatch);
  const canPreflightRemoteH5 = !canPreflightRslc && Boolean(firstH5Url);

  useEffect(() => {
    const onKeyDown = (ev: KeyboardEvent) => {
      if (ev.key === "Escape" && !submitBusy) {
        onClose();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose, submitBusy]);

  // Instant (<50 ms) client-side AOI proportional volume & intersection estimate
  const instantEstimate = useMemo(() => {
    const defaultFull = DEFAULT_LEVEL_FULL_BYTES[level] || 2_000_000_000;
    let fullBytesSum = 0;
    let estSubsetSum = 0;
    let overlappingCount = 0;
    const ratios: number[] = [];
    const perSceneEstimates: Record<string, number> = {};
    const perSceneRatios: Record<string, number> = {};

    for (const sc of scenes) {
      const meta = (sc.native_metadata || {}) as Record<string, unknown>;
      const rawSize = Number(meta.size_bytes ?? meta.file_size ?? sc.file_size_remote ?? 0);
      const sceneFull = Number.isFinite(rawSize) && rawSize > 1_000_000 ? rawSize : defaultFull;
      const fp = bboxFromGeometryOrMeta(sc);
      const ratio = intersectionAreaRatio(editBbox, fp);
      if (ratio > 0) overlappingCount += 1;
      const layerFactor =
        layerChoices.length > 0
          ? Math.max(0.25, Math.min(1.0, selectedLayers.length / Math.max(1, layerChoices.length)))
          : 1.0;
      const effRatio = ratio > 0 ? ratio : 0;
      ratios.push(effRatio);
      fullBytesSum += sceneFull;
      const sceneEst = Math.max(64 * 1024, Math.round(sceneFull * effRatio * layerFactor * 1.1));
      estSubsetSum += effRatio > 0 ? sceneEst : 0;
      if (sc.scene_id) {
        perSceneRatios[sc.scene_id] = effRatio;
        if (effRatio > 0) {
          perSceneEstimates[sc.scene_id] = sceneEst;
        }
      }
    }

    const avgRatio =
      ratios.length > 0 ? ratios.reduce((a, b) => a + b, 0) / ratios.length : 0;
    return {
      fullBytesSum,
      estSubsetSum,
      overlappingCount,
      avgRatioPct: Math.round(avgRatio * 1000) / 10,
      hasNoOverlap: overlappingCount === 0 && scenes.length > 0,
      perSceneEstimates,
      perSceneRatios,
    };
  }, [editBbox, layerChoices.length, level, scenes, selectedLayers.length]);

  const footprintGeometries = useMemo<Json[]>(() => {
    return scenes
      .map((s) => s.footprint_geojson)
      .filter((g): g is Json => Boolean(g && typeof g === "object"));
  }, [scenes]);

  const requestedGeometry = useMemo<Json>(
    () => ({
      type: "Polygon",
      coordinates: [
        [
          [editBbox.west, editBbox.south],
          [editBbox.east, editBbox.south],
          [editBbox.east, editBbox.north],
          [editBbox.west, editBbox.north],
          [editBbox.west, editBbox.south],
        ],
      ],
    }),
    [editBbox],
  );

  const handlePreflightRemote = async () => {
    const first = scenes[0];
    if (!first) return;
    setPreflightBusy(true);
    setErrorMsg(null);
    try {
      if (canPreflightRslc && onPreflightRslcBatch) {
        const summary = await onPreflightRslcBatch({
          aoiGeometry: requestedGeometry,
          targetSceneIds: scenes.map((sc) => sc.scene_id),
        });
        setRslcPreflightSummary(summary);
        setPreflightInfo(null);
        return;
      }
      if (!firstH5Url) {
        setErrorMsg("当前所选产品缺少直接 HDF5 下载链接，将按覆盖面积比例估算并在队列执行时自动解析分片。");
        return;
      }
      const res = (await queryOpenGeodata({
        dataset: "nisar_hdf5_subset",
        tool_name: "preflight",
        bbox: [editBbox.west, editBbox.south, editBbox.east, editBbox.north],
        code: first.scene_id,
        osm_tag: firstH5Url,
        gee_params: {
          url: firstH5Url,
          granule_id: first.scene_id,
          product_level: level,
          band,
          frequency,
          datasets: selectedLayers,
        },
      })) as Record<string, any> | null;
      if (!res || !res.ok) {
        throw new Error(formatBridgeErrorValue(res) || "远端 HDF5 探测未返回有效结构");
      }
      setPreflightInfo({
        gridKind: String(res.grid_kind || "geocoded_grid"),
        epsg: typeof res.projection_epsg === "number" ? res.projection_epsg : null,
        sourceDims: (res.source_dims as [number, number]) || [0, 0],
        subsetDims: (res.subset_dims as [number, number]) || [0, 0],
        estimatedBytes: Number(res.estimated_subset_bytes || 0),
        availableDatasets: Array.isArray(res.available_datasets) ? res.available_datasets : [],
      });
    } catch (err) {
      setErrorMsg(formatBridgeErrorValue(err));
    } finally {
      setPreflightBusy(false);
    }
  };

  const handleSubmitSubset = async () => {
    if (scenes.length === 0) {
      setErrorMsg("请先在结果列表中勾选至少 1 景 NISAR 产品。");
      return;
    }
    if (selectedLayers.length === 0) {
      setErrorMsg("请至少选择 1 个极化通道或干涉图层。");
      return;
    }
    const cleanOut = outputDir.trim();
    if (!cleanOut) {
      setErrorMsg("请先选择或填写裁剪成果保存目录。");
      return;
    }
    if (isTemporarySystemDownloadPath(cleanOut)) {
      setErrorMsg(TEMPORARY_DOWNLOAD_PATH_ERROR_ZH);
      return;
    }

    setSubmitBusy(true);
    setErrorMsg(null);
    setResultNote(null);
    let succeeded = 0;
    let totalSavedBytes = 0;
    const failures: string[] = [];

    try {
      if (level === "RSLC" && onSubmitRslcBatch) {
        try {
          const totalEst = preflightInfo?.estimatedBytes || instantEstimate.estSubsetSum;
          recordSubsetSizeEstimate({
            totalEstimatedBytes: totalEst,
            sceneEstimates: instantEstimate.perSceneEstimates,
          });
          await onSubmitRslcBatch({
            frequency: frequency === "frequencyB" ? "B" : "A",
            polarizations: selectedLayers,
            aoiBbox: editBbox,
            aoiGeometry: requestedGeometry,
            outputRoot: cleanOut,
            estimatedSubsetBytes: totalEst,
            targetSceneIds: scenes.map((sc) => sc.scene_id),
          });
          await Promise.resolve(onSubmitted());
          onClose();
        } catch (err) {
          setErrorMsg(formatBridgeErrorValue(err));
        }
        return;
      }

      for (const sc of scenes) {
        const url = extractNisarSubsetDownloadUrl(sc);
        if (!url) {
          failures.push(`${formatHumanSceneLabel(sc)}: 缺少远程 HDF5 链接`);
          continue;
        }
        const estSceneBytes =
          instantEstimate.perSceneEstimates[sc.scene_id] ||
          (scenes.length > 0 && (preflightInfo?.estimatedBytes || instantEstimate.estSubsetSum) > 0
            ? Math.round((preflightInfo?.estimatedBytes || instantEstimate.estSubsetSum) / scenes.length)
            : 0);
        if (estSceneBytes > 0) {
          recordSubsetSizeEstimate({
            totalEstimatedBytes: estSceneBytes,
            sceneEstimates: { [sc.scene_id]: estSceneBytes },
          });
        }
        try {
          const res = (await queryOpenGeodata({
            dataset: "nisar_hdf5_subset",
            tool_name: "download",
            bbox: [editBbox.west, editBbox.south, editBbox.east, editBbox.north],
            code: sc.scene_id,
            osm_tag: url,
            output_path: cleanOut,
            gee_params: {
              url,
              granule_id: sc.scene_id,
              product_level: level,
              band,
              frequency,
              datasets: selectedLayers,
            },
          })) as Record<string, any> | null;
          if (!res || !res.ok) {
            throw new Error(formatBridgeErrorValue(res) || "HDF5 裁剪任务未返回成功结果");
          }
          if (typeof res.task_id === "string" && res.task_id.trim() && estSceneBytes > 0) {
            recordSubsetSizeEstimate({
              taskId: res.task_id.trim(),
              totalEstimatedBytes: estSceneBytes,
              sceneEstimates: { [sc.scene_id]: estSceneBytes },
            });
          }
          succeeded += 1;
          totalSavedBytes += Number(res.downloaded_bytes || 0);
        } catch (err) {
          failures.push(`${formatHumanSceneLabel(sc)}: ${formatBridgeErrorValue(err)}`);
        }
      }

      if (succeeded > 0) {
        await Promise.resolve(onSubmitted());
        const summaryMsg = `已完成 ${succeeded}/${scenes.length} 景 NISAR ${band}-Band ${level} AOI 在线裁剪（生成 ${formatBytes(totalSavedBytes)}）`;
        setResultNote(summaryMsg);
        emitTopToast({ tone: "success", text: summaryMsg });
        if (failures.length === 0) {
          onClose();
        }
      }
      if (failures.length > 0) {
        setErrorMsg(failures.slice(0, 2).join("；"));
      }
    } finally {
      setSubmitBusy(false);
    }
  };

  return (
    <div
      data-testid="nisar-multilevel-subset-modal"
      className="fixed inset-0 z-[1200] flex items-center justify-center bg-black/45 backdrop-blur-xs p-4"
      role="dialog"
      aria-modal="true"
      aria-label={`NISAR ${band}-Band ${level} AOI 在线裁剪下载`}
    >
      <div className="w-full max-w-4xl max-h-[90vh] overflow-hidden rounded-2xl border border-sp-line bg-sp-surface shadow-2xl flex flex-col">
        {/* Header */}
        <div className="flex items-center justify-between px-5 py-3.5 border-b border-sp-line bg-sp-canvas shrink-0">
          <div className="flex items-center gap-2.5 min-w-0">
            <div className="w-8 h-8 rounded-lg bg-sp-hover flex items-center justify-center shrink-0">
              <SubsetScissorsIcon className="w-4 h-4 text-sp-text" />
            </div>
            <div className="min-w-0">
              <div className="flex items-center gap-2">
                <h2 className="text-sm font-semibold text-sp-text truncate">
                  NISAR {band} 波段 {level} 在线 AOI 裁剪下载
                </h2>
                <span className="px-1.5 py-0.5 rounded bg-sp-hover border border-sp-line font-mono text-xs font-semibold text-sp-text">
                  HDF5 HTTP Range
                </span>
              </div>
              <p className="text-xs text-sp-text-muted truncate">
                研究区：{aoiLabel || "自定义矩形"} · 已选 {scenes.length} 景（按 AOI 范围仅拉取交集科学分片）
              </p>
            </div>
          </div>
          <button
            type="button"
            data-testid="nisar-multilevel-close-btn"
            onClick={onClose}
            disabled={submitBusy}
            aria-label="关闭窗口"
            className="p-1.5 rounded-lg text-sp-text-muted hover:text-sp-text hover:bg-sp-hover cursor-pointer disabled:opacity-50"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        {/* Body */}
        <div className="flex-1 min-h-0 overflow-y-auto p-5 grid grid-cols-1 lg:grid-cols-[1.15fr_1fr] gap-4">
          {/* Left: Map Preview & Coordinate Summary */}
          <div className="flex flex-col gap-3 min-w-0">
            <div className="rounded-xl overflow-hidden border border-sp-line h-[320px] relative">
              <AoiDrawMap
                bbox={editBbox}
                mode="rect"
                drawActive
                minHeight={320}
                footprintGeometries={footprintGeometries}
                requestedGeometry={requestedGeometry}
                onRectDraw={(next) => setEditBbox(next)}
                onGeometryImport={({ bbox }) => setEditBbox(bbox)}
              />
            </div>

            <div className="rounded-xl border border-sp-line bg-sp-card px-3.5 py-2.5 flex flex-col gap-1.5">
              <div className="flex items-center justify-between gap-2 text-xs">
                <span className="font-semibold text-sp-text">裁剪范围（可在左侧地图重绘框选）</span>
                <span className="font-mono text-sp-text-muted whitespace-nowrap tabular-nums">
                  覆盖景比例 {instantEstimate.avgRatioPct.toFixed(1)}%
                </span>
              </div>
              <div className="grid grid-cols-2 sm:grid-cols-4 gap-2 font-mono text-xs tabular-nums text-sp-text-secondary">
                <span className="whitespace-nowrap">{formatSignedLon(editBbox.west)}</span>
                <span className="whitespace-nowrap">{formatSignedLon(editBbox.east)}</span>
                <span className="whitespace-nowrap">{formatSignedLat(editBbox.south)}</span>
                <span className="whitespace-nowrap">{formatSignedLat(editBbox.north)}</span>
              </div>
            </div>

            {/* Volume & Pre-flight Box */}
            <div
              data-testid="nisar-multilevel-estimate-card"
              className="rounded-xl border border-sp-line bg-sp-card px-3.5 py-2.5 flex flex-col gap-2"
            >
              <div className="flex items-center justify-between gap-2">
                <span className="text-xs font-semibold text-sp-text">按 AOI 占比实时体积预估</span>
                {canPreflightRslc || canPreflightRemoteH5 ? (
                  <button
                    type="button"
                    data-testid="nisar-multilevel-preflight-btn"
                    onClick={() => void handlePreflightRemote()}
                    disabled={preflightBusy || submitBusy || scenes.length === 0}
                    title={
                      canPreflightRslc
                        ? "向后台核验当前研究区下已选景的有效相交数量，以及共有的科学频段和极化通道"
                        : "仅读取远端 HDF5 文件头元数据（不下载完整影像），获取精确的像元行列尺寸与投影参数"
                    }
                    className="h-7 px-2.5 rounded-sp-sm border border-sp-line bg-sp-surface text-xs text-sp-text hover:bg-sp-hover inline-flex items-center gap-1 cursor-pointer disabled:opacity-50"
                  >
                    {preflightBusy ? (
                      <Loader2 className="w-3 h-3 animate-spin" />
                    ) : (
                      <Radar className="w-3 h-3" />
                    )}
                    <span>
                      {canPreflightRslc ? "校验研究区与波段可用性" : "读取远端精确行列数（可选）"}
                    </span>
                  </button>
                ) : (
                  <span className="text-xs text-sp-text-muted whitespace-nowrap">
                    已按面积比例估算 · 入队自动解析
                  </span>
                )}
              </div>
              <div className="grid grid-cols-2 gap-2 text-xs">
                <div className="rounded-lg bg-sp-surface border border-sp-line px-2.5 py-1.5">
                  <div className="text-sp-text-muted">原始完整产品总计</div>
                  <div className="font-mono font-semibold text-sp-text tabular-nums">
                    {formatBytes(instantEstimate.fullBytesSum)}
                  </div>
                </div>
                <div className="rounded-lg bg-sp-surface border border-sp-line px-2.5 py-1.5">
                  <div className="text-sp-text-muted">预估裁剪后下载体积</div>
                  <div className="font-mono font-semibold text-sp-text tabular-nums">
                    {preflightInfo?.estimatedBytes
                      ? formatBytes(preflightInfo.estimatedBytes)
                      : `~${formatBytes(instantEstimate.estSubsetSum)}`}
                  </div>
                </div>
              </div>
              {instantEstimate.hasNoOverlap && (
                <div className="flex items-start gap-1.5 text-xs text-sp-warning-text">
                  <AlertTriangle className="w-3.5 h-3.5 shrink-0 mt-0.5" />
                  <span>当前 AOI 与景边界足迹未检测到重叠，请在地图上重绘覆盖该景影像的区域。</span>
                </div>
              )}
              {rslcPreflightSummary && (
                <div
                  data-testid="nisar-multilevel-rslc-preflight-info"
                  className="text-xs font-mono text-sp-text-secondary bg-sp-surface border border-sp-line rounded-lg px-2.5 py-1.5 space-y-0.5"
                >
                  <div>
                    有效相交：{rslcPreflightSummary.eligibleCount}/{scenes.length} 景
                    {rslcPreflightSummary.excludedCount > 0
                      ? `（已过滤不相交 ${rslcPreflightSummary.excludedCount} 景）`
                      : ""}
                  </div>
                  {rslcPreflightSummary.commonFrequencies.length > 0 && (
                    <div>
                      共同可用频段：{rslcPreflightSummary.commonFrequencies.join(" / ")} · 极化：
                      {rslcPreflightSummary.commonPolarizations.join(" / ") || "按默认"}
                    </div>
                  )}
                </div>
              )}
              {preflightInfo && (
                <div className="text-xs font-mono text-sp-text-muted bg-sp-surface border border-sp-line rounded-lg px-2.5 py-1.5 space-y-0.5">
                  <div>
                    网格：{preflightInfo.gridKind}
                    {preflightInfo.epsg ? ` · EPSG:${preflightInfo.epsg}` : ""}
                  </div>
                  <div>
                    全幅维度：{preflightInfo.sourceDims[0]} × {preflightInfo.sourceDims[1]} → 裁剪维度：
                    {preflightInfo.subsetDims[0]} × {preflightInfo.subsetDims[1]}
                  </div>
                </div>
              )}
            </div>
          </div>

          {/* Right: Frequency, Polarizations/Layers & Output Dir */}
          <div className="flex flex-col gap-3 min-w-0">
            <div className="rounded-xl border border-sp-line bg-sp-card p-3.5 flex flex-col gap-2">
              <div className="flex items-center gap-1.5 text-xs font-semibold text-sp-text">
                <Layers className="w-3.5 h-3.5" />
                <span>频段与科学图层选择</span>
              </div>
              <div className="flex flex-col gap-1">
                <span className="text-xs text-sp-text-secondary">Frequency 频段</span>
                <SpSelect
                  data-testid="nisar-multilevel-frequency-select"
                  value={frequency}
                  onChange={(v) => setFrequency(v === "frequencyB" ? "frequencyB" : "frequencyA")}
                  options={[
                    { value: "frequencyA", label: "frequencyA（主科学频段 · 20/40 MHz）" },
                    { value: "frequencyB", label: "frequencyB（次频段 · 5 MHz）" },
                  ]}
                  size="sm"
                />
              </div>

              <div className="flex flex-col gap-1.5 mt-1">
                <span className="text-xs text-sp-text-secondary">
                  待裁剪通道 / 图层（{selectedLayers.length}/{layerChoices.length}）
                </span>
                <div className="flex flex-col gap-1 max-h-[210px] overflow-y-auto pr-1">
                  {layerChoices.map((choice) => {
                    const checked = selectedLayers.includes(choice.id);
                    return (
                      <label
                        key={choice.id}
                        className="flex items-center gap-2 rounded-lg border border-sp-line bg-sp-surface px-2.5 py-1.5 text-xs cursor-pointer hover:bg-sp-hover"
                      >
                        <input
                          type="checkbox"
                          checked={checked}
                          onChange={(e) => {
                            if (e.target.checked) {
                              setSelectedLayers((prev) =>
                                prev.includes(choice.id) ? prev : [...prev, choice.id],
                              );
                            } else {
                              setSelectedLayers((prev) => prev.filter((k) => k !== choice.id));
                            }
                          }}
                        />
                        <span className="font-mono font-semibold text-sp-text">{choice.id}</span>
                        <span className="text-sp-text-muted truncate">{choice.label}</span>
                      </label>
                    );
                  })}
                </div>
              </div>
            </div>

            {/* Granules preview list */}
            <div className="rounded-xl border border-sp-line bg-sp-card p-3.5 flex flex-col gap-1.5">
              <div className="flex items-center justify-between gap-2 text-xs">
                <span className="font-semibold text-sp-text">已选中景清单（{scenes.length}）</span>
                <span className="text-sp-text-muted tabular-nums">
                  有效覆盖 {instantEstimate.overlappingCount}/{scenes.length} 景
                </span>
              </div>
              <div
                data-testid="nisar-multilevel-scene-list"
                className="max-h-[144px] overflow-y-auto space-y-1 pr-1"
              >
                {scenes.map((sc) => {
                  const humanLabel = formatHumanSceneLabel(sc) || sc.product_id || sc.scene_id;
                  const ratio = instantEstimate.perSceneRatios[sc.scene_id] ?? 0;
                  return (
                    <div
                      key={sc.scene_id}
                      className="flex items-center justify-between gap-2 min-h-[26px] text-xs font-mono text-sp-text-secondary bg-sp-surface border border-sp-line rounded px-2 py-1"
                      title={humanLabel}
                    >
                      <span className="min-w-0 flex-1 truncate text-sp-text">{humanLabel}</span>
                      <span
                        className={cn(
                          "shrink-0 tabular-nums text-xs",
                          ratio > 0 ? "text-sp-text-muted" : "text-sp-danger-text",
                        )}
                      >
                        {ratio > 0 ? `${(ratio * 100).toFixed(1)}%` : "不相交"}
                      </span>
                    </div>
                  );
                })}
              </div>
            </div>

            {/* Output folder */}
            <div className="rounded-xl border border-sp-line bg-sp-card p-3.5 flex flex-col gap-1.5">
              <span className="text-xs font-semibold text-sp-text">成果保存目录 (_SUBSET.h5)</span>
              <div className="flex items-center gap-1.5">
                <input
                  type="text"
                  data-testid="nisar-multilevel-output-input"
                  value={outputDir}
                  onChange={(e) => {
                    setOutputDir(e.target.value);
                    setErrorMsg(null);
                  }}
                  placeholder="请选择或输入输出文件夹路径"
                  className="h-8 flex-1 min-w-0 rounded-sp-sm border border-sp-line bg-sp-surface px-2.5 text-xs font-mono text-sp-text outline-none"
                />
                <button
                  type="button"
                  onClick={async () => {
                    const picked = await pickDirectory("选择 NISAR 裁剪成果保存目录").catch(() => null);
                    if (picked && picked.ok && picked.path) {
                      if (isTemporarySystemDownloadPath(picked.path)) {
                        setErrorMsg(TEMPORARY_DOWNLOAD_PATH_ERROR_ZH);
                        return;
                      }
                      setErrorMsg(null);
                      setOutputDir(picked.path);
                    }
                  }}
                  className="h-8 px-2.5 rounded-sp-sm border border-sp-line bg-sp-surface text-xs text-sp-text hover:bg-sp-hover inline-flex items-center gap-1 cursor-pointer shrink-0"
                >
                  <FolderOpen className="w-3.5 h-3.5" />
                  <span>浏览</span>
                </button>
              </div>
            </div>

            {errorMsg && (
              <div className="rounded-xl border border-sp-danger-text/30 bg-sp-danger-bg px-3 py-2 text-xs text-sp-danger-text flex items-start gap-1.5">
                <AlertTriangle className="w-3.5 h-3.5 shrink-0 mt-0.5" />
                <span>{errorMsg}</span>
              </div>
            )}
            {resultNote && (
              <div className="rounded-xl border border-sp-success-text/30 bg-sp-success-bg px-3 py-2 text-xs text-sp-success-text flex items-start gap-1.5">
                <CheckCircle2 className="w-3.5 h-3.5 shrink-0 mt-0.5" />
                <span>{resultNote}</span>
              </div>
            )}
          </div>
        </div>

        {/* Footer */}
        <div className="flex items-center justify-between gap-3 px-5 py-3.5 border-t border-sp-line bg-sp-canvas shrink-0">
          <div className="text-xs text-sp-text-muted">
            输出独立自包含 <span className="font-mono text-sp-text">*_SUBSET.h5</span>，完整保留轨道、Identification 与裁剪坐标维度。
          </div>
          <div className="flex items-center gap-2 shrink-0">
            <button
              type="button"
              onClick={onClose}
              disabled={submitBusy}
              className="h-8 px-3 rounded-sp-sm border border-sp-line bg-sp-surface text-xs font-medium text-sp-text hover:bg-sp-hover cursor-pointer disabled:opacity-50"
            >
              取消
            </button>
            <button
              type="button"
              data-testid="nisar-multilevel-submit-btn"
              onClick={() => void handleSubmitSubset()}
              disabled={submitBusy || scenes.length === 0}
              className="h-8 px-3.5 rounded-sp-sm bg-sp-accent-fill text-white text-xs font-semibold hover:opacity-90 disabled:opacity-50 cursor-pointer inline-flex items-center gap-1.5"
            >
              {submitBusy ? (
                <Loader2 className="w-3.5 h-3.5 animate-spin" />
              ) : (
                <SubsetScissorsIcon className="w-3.5 h-3.5" />
              )}
              <span>创建 AOI 裁剪下载任务</span>
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
