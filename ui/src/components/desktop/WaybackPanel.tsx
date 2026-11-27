import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Eye, EyeOff, Layers, Loader2, RefreshCw, ScanSearch, X } from "lucide-react";
import { useI18n } from "@/i18n";
import type { Bbox } from "@/lib/bridge";
import type { GeodataRasterOverlaySpec } from "@/components/WorkbenchMap";
import { SpSelect } from "@/components/ui/SpSelect";
import { cn } from "@/lib/utils";
import {
  type WaybackPeriodSelection,
  type WaybackScanResult,
  type WaybackVersion,
  cancelWaybackScan,
  fetchWaybackVersions,
  getWaybackScanStatus,
  isValidWaybackVersionId,
  probeWaybackMaxZoom,
  startWaybackScan,
  waybackTileUrlTemplate,
} from "@/lib/waybackClient";

export interface WaybackPanelProps {
  workingAoi: Bbox | null;
  selectedZooms: number[];
  activeVersionId: string;
  selectedPeriods: WaybackPeriodSelection[];
  probedMaxZoom: number | null;
  onChangeActiveVersionId: (versionId: string) => void;
  onChangeSelectedPeriods: (periods: WaybackPeriodSelection[]) => void;
  onChangeProbedMaxZoom: (maxZoom: number | null) => void;
  onPreviewWaybackOverlay?: (overlay: GeodataRasterOverlaySpec | null) => void;
}

export function WaybackPanel({
  workingAoi,
  selectedZooms,
  activeVersionId,
  selectedPeriods,
  probedMaxZoom,
  onChangeActiveVersionId,
  onChangeSelectedPeriods,
  onChangeProbedMaxZoom,
  onPreviewWaybackOverlay,
}: WaybackPanelProps) {
  const { t } = useI18n();
  const [versions, setVersions] = useState<WaybackVersion[]>([]);
  const [versionsLoading, setVersionsLoading] = useState<boolean>(false);
  const [versionsError, setVersionsError] = useState<string | null>(null);
  const [versionsCacheSource, setVersionsCacheSource] = useState<string>("");

  const [probingZoom, setProbingZoom] = useState<boolean>(false);
  const [probeError, setProbeError] = useState<string | null>(null);
  const [previewEnabled, setPreviewEnabled] = useState<boolean>(false);

  const [scanMode, setScanMode] = useState<"fast" | "fine" | "official">("fast");
  const [scanId, setScanId] = useState<string | null>(null);
  const [scanStatus, setScanStatus] = useState<"idle" | "running" | "completed" | "cancelled" | "failed">("idle");
  const [scanCompleted, setScanCompleted] = useState<number>(0);
  const [scanTotal, setScanTotal] = useState<number>(0);
  const [scanEffectiveLayer, setScanEffectiveLayer] = useState<number | null>(null);
  const [scanFromCache, setScanFromCache] = useState<boolean>(false);
  const [scanError, setScanError] = useState<string | null>(null);
  const [scanResult, setScanResult] = useState<WaybackScanResult | null>(null);
  const [scanViewTab, setScanViewTab] = useState<"captures" | "releases">("captures");
  const [onlyChangedReleases, setOnlyChangedReleases] = useState<boolean>(true);

  const activeScanIdRef = useRef<string | null>(null);
  const pollTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const clearPollTimer = useCallback(() => {
    if (pollTimerRef.current !== null) {
      clearTimeout(pollTimerRef.current);
      pollTimerRef.current = null;
    }
  }, []);

  const aoiKey = workingAoi
    ? `${workingAoi.west.toFixed(5)},${workingAoi.south.toFixed(5)},${workingAoi.east.toFixed(5)},${workingAoi.north.toFixed(5)}`
    : "";

  const loadVersions = useCallback(
    async (forceRefresh = false) => {
      setVersionsLoading(true);
      setVersionsError(null);
      try {
        const res = await fetchWaybackVersions(forceRefresh);
        const list = Array.isArray(res.versions) ? res.versions : [];
        setVersions(list);
        setVersionsCacheSource(res.cache_source || "");
        if (list.length > 0 && !isValidWaybackVersionId(activeVersionId)) {
          const newest = list[0];
          onChangeActiveVersionId(newest.release_num);
          if (selectedPeriods.length === 0) {
            onChangeSelectedPeriods([
              {
                releaseNum: newest.release_num,
                dateLabel: newest.release_date || newest.release_num,
                label: newest.label,
              },
            ]);
          }
        }
      } catch (err) {
        setVersionsError(err instanceof Error ? err.message : String(err));
      } finally {
        setVersionsLoading(false);
      }
    },
    [activeVersionId, onChangeActiveVersionId, onChangeSelectedPeriods, selectedPeriods.length],
  );

  useEffect(() => {
    void loadVersions(false);
    // Only fetch on mount (refresh handled via button).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const runZoomProbe = useCallback(async () => {
    if (!workingAoi) {
      onChangeProbedMaxZoom(null);
      setProbeError(null);
      return;
    }
    setProbingZoom(true);
    setProbeError(null);
    try {
      const res = await probeWaybackMaxZoom(
        [workingAoi.west, workingAoi.south, workingAoi.east, workingAoi.north],
        isValidWaybackVersionId(activeVersionId) ? activeVersionId : undefined,
      );
      if (typeof res.max_zoom === "number" && Number.isFinite(res.max_zoom)) {
        onChangeProbedMaxZoom(res.max_zoom);
      }
    } catch (err) {
      setProbeError(err instanceof Error ? err.message : String(err));
    } finally {
      setProbingZoom(false);
    }
  }, [workingAoi, activeVersionId, onChangeProbedMaxZoom]);

  // Reset scan state and re-probe real maxZoom whenever workingAoi changes (Condition F2).
  useEffect(() => {
    let cancelled = false;
    clearPollTimer();
    const prevScanId = activeScanIdRef.current;
    if (prevScanId) {
      activeScanIdRef.current = null;
      void cancelWaybackScan(prevScanId);
    }
    setScanId(null);
    setScanStatus("idle");
    setScanCompleted(0);
    setScanTotal(0);
    setScanResult(null);
    setScanError(null);
    setScanFromCache(false);
    setScanEffectiveLayer(null);
    onChangeProbedMaxZoom(null);

    if (!workingAoi) {
      setProbeError(null);
      return;
    }

    setProbingZoom(true);
    setProbeError(null);
    void probeWaybackMaxZoom(
      [workingAoi.west, workingAoi.south, workingAoi.east, workingAoi.north],
      isValidWaybackVersionId(activeVersionId) ? activeVersionId : undefined,
    )
      .then((res) => {
        if (cancelled) return;
        if (typeof res.max_zoom === "number" && Number.isFinite(res.max_zoom)) {
          onChangeProbedMaxZoom(res.max_zoom);
        }
      })
      .catch((err) => {
        if (cancelled) return;
        setProbeError(err instanceof Error ? err.message : String(err));
      })
      .finally(() => {
        if (!cancelled) {
          setProbingZoom(false);
        }
      });

    return () => {
      cancelled = true;
    };
    // Triggered deliberately by AOI key changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [aoiKey]);

  // Keep map preview overlay synchronized with toggle & activeVersionId, and clean up on unmount.
  useEffect(() => {
    if (!onPreviewWaybackOverlay) return;
    if (!previewEnabled || !isValidWaybackVersionId(activeVersionId)) {
      onPreviewWaybackOverlay(null);
      return;
    }
    const ver = versions.find((v) => v.release_num === activeVersionId);
    onPreviewWaybackOverlay({
      id: `wayback-preview-${activeVersionId}`,
      label: ver
        ? `Esri Wayback ${ver.release_date || activeVersionId} (#${activeVersionId})`
        : `Esri Wayback #${activeVersionId}`,
      tileUrlTemplate: waybackTileUrlTemplate(activeVersionId),
      bbox: workingAoi,
      attribution: "Esri World Imagery Wayback",
      maxZoom: probedMaxZoom ?? 20,
    });
  }, [
    previewEnabled,
    activeVersionId,
    versions,
    workingAoi,
    probedMaxZoom,
    onPreviewWaybackOverlay,
  ]);

  useEffect(() => {
    return () => {
      clearPollTimer();
      const pendingScanId = activeScanIdRef.current;
      if (pendingScanId) {
        activeScanIdRef.current = null;
        void cancelWaybackScan(pendingScanId);
      }
      onPreviewWaybackOverlay?.(null);
      onChangeProbedMaxZoom(null);
    };
    // Unmount cleanup only.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const versionOptions = useMemo(() => {
    if (versions.length === 0) {
      return isValidWaybackVersionId(activeVersionId)
        ? [{ value: activeVersionId, label: `Wayback #${activeVersionId}` }]
        : [{ value: "", label: t("mapWorkbench.wayback.loadingVersions") }];
    }
    return versions.map((v) => ({
      value: v.release_num,
      label: `${v.release_date || t("mapWorkbench.wayback.unknownDate")} · #${v.release_num}`,
    }));
  }, [versions, activeVersionId, t]);

  const selectedPeriodIds = useMemo(
    () => new Set(selectedPeriods.map((p) => p.releaseNum)),
    [selectedPeriods],
  );

  const togglePeriodSelection = useCallback(
    (entry: WaybackPeriodSelection) => {
      const exists = selectedPeriodIds.has(entry.releaseNum);
      if (exists) {
        onChangeSelectedPeriods(selectedPeriods.filter((p) => p.releaseNum !== entry.releaseNum));
      } else {
        onChangeSelectedPeriods([...selectedPeriods, entry]);
      }
    },
    [selectedPeriodIds, selectedPeriods, onChangeSelectedPeriods],
  );

  const handleSelectDropdownVersion = useCallback(
    (nextId: string) => {
      onChangeActiveVersionId(nextId);
      if (!isValidWaybackVersionId(nextId)) return;
      const ver = versions.find((v) => v.release_num === nextId);
      const nextPeriod: WaybackPeriodSelection = {
        releaseNum: nextId,
        dateLabel: ver?.release_date || nextId,
        label: ver?.label,
      };
      if (selectedPeriods.length <= 1) {
        onChangeSelectedPeriods([nextPeriod]);
      } else if (!selectedPeriodIds.has(nextId)) {
        onChangeSelectedPeriods([nextPeriod, ...selectedPeriods]);
      }
    },
    [versions, selectedPeriods, selectedPeriodIds, onChangeActiveVersionId, onChangeSelectedPeriods],
  );

  const pollScanProgress = useCallback(
    async (id: string) => {
      if (activeScanIdRef.current !== id) return;
      try {
        const st = await getWaybackScanStatus(id);
        if (activeScanIdRef.current !== id) return;
        setScanCompleted(st.completed ?? 0);
        setScanTotal(st.total ?? 0);
        if (typeof st.effective_layer === "number") {
          setScanEffectiveLayer(st.effective_layer);
        }
        setScanFromCache(Boolean(st.from_cache));

        if (st.status === "completed") {
          activeScanIdRef.current = null;
          setScanStatus("completed");
          setScanResult(st.result ?? null);
          return;
        }
        if (st.status === "cancelled" || st.status === "failed" || st.status === "not_found") {
          activeScanIdRef.current = null;
          setScanStatus(st.status === "failed" ? "failed" : "cancelled");
          if (st.error) {
            setScanError(st.error);
          }
          return;
        }
        pollTimerRef.current = setTimeout(() => {
          void pollScanProgress(id);
        }, 400);
      } catch (err) {
        if (activeScanIdRef.current !== id) return;
        activeScanIdRef.current = null;
        setScanStatus("failed");
        setScanError(err instanceof Error ? err.message : String(err));
      }
    },
    [],
  );

  const handleStartScan = useCallback(
    async (forceRefresh = false) => {
      if (!workingAoi) return;
      clearPollTimer();
      if (activeScanIdRef.current) {
        void cancelWaybackScan(activeScanIdRef.current);
      }
      setScanStatus("running");
      setScanError(null);
      setScanCompleted(0);
      setScanTotal(versions.length);

      const scanZoom = Math.min(20, Math.max(10, selectedZooms[selectedZooms.length - 1] ?? probedMaxZoom ?? 18));
      try {
        const res = await startWaybackScan(
          [workingAoi.west, workingAoi.south, workingAoi.east, workingAoi.north],
          scanZoom,
          scanMode,
          forceRefresh,
        );
        setScanId(res.scan_id);
        setScanCompleted(res.completed ?? 0);
        setScanTotal(res.total ?? versions.length);
        setScanFromCache(Boolean(res.from_cache));
        if (typeof res.effective_layer === "number") {
          setScanEffectiveLayer(res.effective_layer);
        }
        if (res.status === "completed" && res.result) {
          activeScanIdRef.current = null;
          setScanStatus("completed");
          setScanResult(res.result);
        } else {
          activeScanIdRef.current = res.scan_id;
          pollTimerRef.current = setTimeout(() => {
            void pollScanProgress(res.scan_id);
          }, 350);
        }
      } catch (err) {
        activeScanIdRef.current = null;
        setScanStatus("failed");
        setScanError(err instanceof Error ? err.message : String(err));
      }
    },
    [workingAoi, clearPollTimer, versions.length, selectedZooms, probedMaxZoom, scanMode, pollScanProgress],
  );

  const handleCancelScan = useCallback(() => {
    clearPollTimer();
    const currentId = activeScanIdRef.current ?? scanId;
    activeScanIdRef.current = null;
    setScanStatus("cancelled");
    if (currentId) {
      void cancelWaybackScan(currentId);
    }
  }, [clearPollTimer, scanId]);

  const visibleReleases = useMemo(() => {
    if (!scanResult) return [];
    return onlyChangedReleases
      ? scanResult.release_summaries.filter((r) => r.has_local_change)
      : scanResult.release_summaries;
  }, [scanResult, onlyChangedReleases]);

  const activeVersionMeta = useMemo(
    () => versions.find((v) => v.release_num === activeVersionId) ?? null,
    [versions, activeVersionId],
  );

  return (
    <div
      data-testid="wayback-panel"
      className="flex flex-col gap-2.5 rounded-sp-md border border-sp-line bg-sp-surface p-2.5"
    >
      <div className="flex items-center justify-between gap-1.5">
        <div className="flex items-center gap-1.5 min-w-0">
          <Layers className="h-3.5 w-3.5 text-sp-text-secondary shrink-0" />
          <span
            className="text-xs font-medium text-sp-text truncate"
            title={t("mapWorkbench.wayback.titleTooltip")}
          >
            {t("mapWorkbench.wayback.title")}
          </span>
          {versions.length > 0 && (
            <span
              data-testid="wayback-version-count"
              className="rounded-sp-sm bg-sp-surface-muted px-1.5 py-0.5 font-mono text-xs tabular-nums text-sp-text-muted whitespace-nowrap shrink-0"
            >
              {t("mapWorkbench.wayback.versionCount", { count: versions.length })}
            </span>
          )}
        </div>
        <button
          type="button"
          data-testid="wayback-refresh-versions"
          onClick={() => void loadVersions(true)}
          disabled={versionsLoading}
          title={
            versionsCacheSource
              ? t("mapWorkbench.wayback.refreshSourceTitle", { source: versionsCacheSource })
              : t("mapWorkbench.wayback.refreshTitle")
          }
          className="h-6 px-2 rounded-sp-sm border border-sp-line bg-sp-surface text-xs text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover disabled:opacity-50 inline-flex items-center gap-1 cursor-pointer whitespace-nowrap shrink-0"
        >
          <RefreshCw className={cn("h-3 w-3", versionsLoading && "animate-spin")} />
          <span>{t("mapWorkbench.wayback.refresh")}</span>
        </button>
      </div>

      {versionsError && (
        <p data-testid="wayback-versions-error" className="text-xs text-sp-danger-text leading-snug">
          {versionsError}
        </p>
      )}

      <div className="flex items-center gap-1.5">
        <div className="flex-1 min-w-0">
          <SpSelect
            value={activeVersionId}
            onChange={handleSelectDropdownVersion}
            options={versionOptions}
            aria-label={t("mapWorkbench.wayback.selectReleaseAria")}
          />
        </div>
        <button
          type="button"
          data-testid="wayback-toggle-preview"
          disabled={!isValidWaybackVersionId(activeVersionId)}
          onClick={() => setPreviewEnabled((prev) => !prev)}
          title={
            previewEnabled
              ? t("mapWorkbench.wayback.previewOffTitle")
              : t("mapWorkbench.wayback.previewOnTitle")
          }
          className={cn(
            "h-8 px-2.5 rounded-sp-sm border text-xs inline-flex items-center gap-1 shrink-0 transition-colors cursor-pointer disabled:opacity-50 disabled:cursor-not-allowed",
            previewEnabled
              ? "border-sp-text bg-sp-selected text-sp-text font-medium"
              : "border-sp-line bg-sp-surface text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover",
          )}
        >
          {previewEnabled ? <Eye className="h-3.5 w-3.5" /> : <EyeOff className="h-3.5 w-3.5" />}
          <span>{previewEnabled ? t("mapWorkbench.wayback.previewing") : t("mapWorkbench.wayback.preview")}</span>
        </button>
      </div>

      {/* Real per-AOI maxZoom probe indicator */}
      <div className="flex items-center justify-between gap-2 text-xs">
        <div className="flex items-center gap-1.5 min-w-0">
          {probingZoom ? (
            <>
              <Loader2 className="h-3 w-3 animate-spin text-sp-text-muted shrink-0" />
              <span className="text-sp-text-muted">{t("mapWorkbench.wayback.probingZoom")}</span>
            </>
          ) : probeError ? (
            <span data-testid="wayback-probe-error" className="text-sp-danger-text truncate" title={probeError}>
              {t("mapWorkbench.wayback.probeFailed", { error: probeError })}
            </span>
          ) : probedMaxZoom !== null ? (
            <span data-testid="wayback-probed-max-zoom" className="text-sp-text-secondary tabular-nums">
              {t("mapWorkbench.wayback.probedMaxZoomPrefix")}{" "}
              <strong className="font-mono text-sp-text">Z{probedMaxZoom}</strong>
              {activeVersionMeta?.release_date && (
                <span className="inline-block whitespace-nowrap">
                  {" · "}
                  {t("mapWorkbench.wayback.currentReleaseLabel", {
                    date: activeVersionMeta.release_date,
                  })}
                </span>
              )}
            </span>
          ) : (
            <span className="text-sp-text-muted">{t("mapWorkbench.wayback.bindAoiToProbeHint")}</span>
          )}
        </div>
        {workingAoi && (
          <button
            type="button"
            data-testid="wayback-reprobe-zoom"
            onClick={() => void runZoomProbe()}
            disabled={probingZoom}
            className="text-xs text-sp-text-muted hover:text-sp-text underline underline-offset-2 shrink-0 cursor-pointer disabled:opacity-50"
          >
            {t("mapWorkbench.wayback.reprobeZoom")}
          </button>
        )}
      </div>

      {/* Metadata change scan controls */}
      <div className="flex flex-col gap-1.5 border-t border-sp-line pt-2">
        <div className="flex flex-wrap items-center justify-between gap-1.5">
          <div className="flex flex-wrap items-center gap-1">
            {(
              [
                { id: "fast", label: t("mapWorkbench.wayback.modeFast") },
                { id: "fine", label: t("mapWorkbench.wayback.modeFine") },
                { id: "official", label: t("mapWorkbench.wayback.modeOfficial") },
              ] as const
            ).map((m) => (
              <button
                key={m.id}
                type="button"
                data-testid={`wayback-scan-mode-${m.id}`}
                onClick={() => setScanMode(m.id)}
                className={cn(
                  "h-6 px-2 rounded-sp-sm border text-xs transition-colors cursor-pointer whitespace-nowrap shrink-0",
                  scanMode === m.id
                    ? "border-sp-text bg-sp-selected text-sp-text font-medium"
                    : "border-sp-line bg-sp-surface text-sp-text-secondary hover:bg-sp-hover",
                )}
              >
                {m.label}
              </button>
            ))}
          </div>

          {scanStatus === "running" ? (
            <button
              type="button"
              data-testid="wayback-cancel-scan"
              onClick={handleCancelScan}
              className="h-6 px-2 rounded-sp-sm border border-sp-line bg-sp-surface text-xs text-sp-text hover:bg-sp-hover inline-flex items-center gap-1 cursor-pointer whitespace-nowrap shrink-0"
            >
              <X className="h-3 w-3 shrink-0" />
              <span>{t("mapWorkbench.wayback.cancel")}</span>
            </button>
          ) : (
            <button
              type="button"
              data-testid="wayback-start-scan"
              disabled={!workingAoi}
              onClick={() => void handleStartScan(false)}
              className="h-6 px-2.5 rounded-sp-sm border border-sp-line bg-sp-surface text-xs font-medium text-sp-text hover:bg-sp-hover disabled:opacity-50 disabled:cursor-not-allowed inline-flex items-center gap-1 cursor-pointer whitespace-nowrap shrink-0"
            >
              <ScanSearch className="h-3 w-3 shrink-0" />
              <span>{t("mapWorkbench.wayback.scanLocalChanges")}</span>
            </button>
          )}
        </div>

        {scanStatus === "running" && (
          <div data-testid="wayback-scan-progress" className="flex flex-col gap-1">
            <div className="flex items-center justify-between text-xs text-sp-text-muted tabular-nums">
              <span>{t("mapWorkbench.wayback.scanningProgress")}</span>
              <span className="font-mono">
                {scanCompleted}/{scanTotal || "?"}
                {scanEffectiveLayer !== null ? ` · L${scanEffectiveLayer}` : ""}
              </span>
            </div>
            <div className="h-1 w-full rounded-full bg-sp-surface-muted overflow-hidden">
              <div
                className="h-full bg-sp-text transition-all duration-150"
                style={{
                  width: `${scanTotal > 0 ? Math.min(100, Math.round((scanCompleted / scanTotal) * 100)) : 8}%`,
                }}
              />
            </div>
          </div>
        )}

        {scanError && (
          <p data-testid="wayback-scan-error" className="text-xs text-sp-danger-text leading-snug">
            {scanError}
          </p>
        )}

        {scanResult && (
          <div data-testid="wayback-scan-results" className="flex flex-col gap-1.5">
            <div className="flex flex-wrap items-center justify-between gap-1 text-xs text-sp-text-muted tabular-nums">
              <span>
                {t("mapWorkbench.wayback.scanSummary", {
                  scanned: scanResult.total_releases_scanned,
                  changed: scanResult.changed_releases_count,
                  captures: scanResult.unique_captures_count,
                })}
              </span>
              <span className="font-mono">
                L{scanResult.effective_layer}
                {scanResult.used_fallback_layer ? t("mapWorkbench.wayback.fallbackLayerTag") : ""}
                {scanFromCache ? t("mapWorkbench.wayback.cacheTag") : ""}
              </span>
            </div>

            <div className="flex items-center justify-between gap-1">
              <div className="flex items-center gap-1">
                <button
                  type="button"
                  data-testid="wayback-tab-captures"
                  onClick={() => setScanViewTab("captures")}
                  className={cn(
                    "h-6 px-2 rounded-sp-sm text-xs cursor-pointer",
                    scanViewTab === "captures"
                      ? "bg-sp-selected text-sp-text font-medium"
                      : "text-sp-text-muted hover:text-sp-text",
                  )}
                >
                  {t("mapWorkbench.wayback.tabCaptures", { count: scanResult.captures.length })}
                </button>
                <button
                  type="button"
                  data-testid="wayback-tab-releases"
                  onClick={() => setScanViewTab("releases")}
                  className={cn(
                    "h-6 px-2 rounded-sp-sm text-xs cursor-pointer",
                    scanViewTab === "releases"
                      ? "bg-sp-selected text-sp-text font-medium"
                      : "text-sp-text-muted hover:text-sp-text",
                  )}
                >
                  {t("mapWorkbench.wayback.tabReleases", {
                    count: scanResult.release_summaries.length,
                  })}
                </button>
              </div>
              {scanViewTab === "releases" && (
                <label className="inline-flex items-center gap-1 text-xs text-sp-text-muted cursor-pointer select-none">
                  <input
                    type="checkbox"
                    checked={onlyChangedReleases}
                    onChange={(e) => setOnlyChangedReleases(e.target.checked)}
                    className="h-3 w-3 accent-[var(--sp-text)]"
                  />
                  <span>{t("mapWorkbench.wayback.onlyChanged")}</span>
                </label>
              )}
            </div>

            <div className="max-h-44 overflow-y-auto flex flex-col gap-1 pr-0.5">
              {scanViewTab === "captures" ? (
                scanResult.captures.length === 0 ? (
                  <p className="text-xs text-sp-text-muted py-1">
                    {t("mapWorkbench.wayback.noCapturesFound")}
                  </p>
                ) : (
                  scanResult.captures.map((cap) => {
                    const checked = selectedPeriodIds.has(cap.release_num);
                    const covPct = Math.round((cap.coverage_ratio ?? 1) * 100);
                    return (
                      <div
                        key={cap.capture_id}
                        data-testid={`wayback-capture-row-${cap.release_num}`}
                        className={cn(
                          "flex items-center justify-between gap-2 rounded-sp-sm border px-2 py-1 text-xs",
                          checked
                            ? "border-sp-text bg-sp-selected/60 text-sp-text"
                            : "border-sp-line bg-sp-surface text-sp-text-secondary",
                        )}
                      >
                        <label className="flex items-center gap-1.5 min-w-0 flex-1 cursor-pointer">
                          <input
                            type="checkbox"
                            checked={checked}
                            onChange={() =>
                              togglePeriodSelection({
                                releaseNum: cap.release_num,
                                dateLabel: cap.capture_date || cap.release_date || cap.release_num,
                                label: `${cap.capture_date} (#${cap.release_num})`,
                              })
                            }
                            className="h-3.5 w-3.5 shrink-0 accent-[var(--sp-text)]"
                          />
                          <div className="min-w-0 flex-1">
                            <div className="flex items-center gap-1.5 font-mono text-xs tabular-nums">
                              <span className="font-medium text-sp-text">{cap.capture_date || cap.release_date}</span>
                              <span className="text-sp-text-muted">#{cap.release_num}</span>
                              {covPct < 100 && (
                                <span className="text-xs text-sp-text-muted">
                                  {t("mapWorkbench.wayback.coveragePct", { pct: covPct })}
                                </span>
                              )}
                            </div>
                            {(cap.source_info || cap.resolution_m !== null) && (
                              <div className="truncate text-xs text-sp-text-muted">
                                {cap.source_info || "World Imagery"}
                                {cap.resolution_m !== null ? ` · ${cap.resolution_m.toFixed(2)}m` : ""}
                              </div>
                            )}
                          </div>
                        </label>
                        <button
                          type="button"
                          onClick={() => {
                            onChangeActiveVersionId(cap.release_num);
                            setPreviewEnabled(true);
                          }}
                          title={t("mapWorkbench.wayback.previewCaptureTitle")}
                          className="h-6 px-1.5 rounded-sp-sm border border-sp-line bg-sp-surface text-xs text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover shrink-0 cursor-pointer"
                        >
                          {t("mapWorkbench.wayback.preview")}
                        </button>
                      </div>
                    );
                  })
                )
              ) : visibleReleases.length === 0 ? (
                <p className="text-xs text-sp-text-muted py-1">
                  {t("mapWorkbench.wayback.noMatchingReleases")}
                </p>
              ) : (
                visibleReleases.map((rel) => {
                  const checked = selectedPeriodIds.has(rel.release_num);
                  return (
                    <div
                      key={rel.release_num}
                      data-testid={`wayback-release-row-${rel.release_num}`}
                      className={cn(
                        "flex items-center justify-between gap-2 rounded-sp-sm border px-2 py-1 text-xs",
                        checked
                          ? "border-sp-text bg-sp-selected/60 text-sp-text"
                          : "border-sp-line bg-sp-surface text-sp-text-secondary",
                      )}
                    >
                      <label className="flex items-center gap-1.5 min-w-0 flex-1 cursor-pointer">
                        <input
                          type="checkbox"
                          checked={checked}
                          onChange={() =>
                            togglePeriodSelection({
                              releaseNum: rel.release_num,
                              dateLabel: rel.capture_dates[0] || rel.release_date || rel.release_num,
                              label: rel.label,
                            })
                          }
                          className="h-3.5 w-3.5 shrink-0 accent-[var(--sp-text)]"
                        />
                        <div className="min-w-0 flex-1">
                          <div className="flex items-center gap-1.5 font-mono text-xs tabular-nums">
                            <span className="font-medium text-sp-text">{rel.release_date || rel.release_num}</span>
                            <span className="text-sp-text-muted">#{rel.release_num}</span>
                            {rel.has_local_change && (
                              <span className="rounded-sp-sm bg-sp-surface-muted px-1 text-xs text-sp-text">
                                {t("mapWorkbench.wayback.hasChangeBadge")}
                              </span>
                            )}
                          </div>
                          <div className="truncate text-xs text-sp-text-muted">
                            {rel.capture_dates.length > 0
                              ? t("mapWorkbench.wayback.captureDatesLine", {
                                  dates: rel.capture_dates.slice(0, 3).join(", "),
                                })
                              : rel.from_dead_service
                                ? t("mapWorkbench.wayback.upstreamUnreachable")
                                : t("mapWorkbench.wayback.noLocalChange")}
                          </div>
                        </div>
                      </label>
                      <button
                        type="button"
                        onClick={() => {
                          onChangeActiveVersionId(rel.release_num);
                          setPreviewEnabled(true);
                        }}
                        title={t("mapWorkbench.wayback.previewReleaseTitle")}
                        className="h-6 px-1.5 rounded-sp-sm border border-sp-line bg-sp-surface text-xs text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover shrink-0 cursor-pointer"
                      >
                        {t("mapWorkbench.wayback.preview")}
                      </button>
                    </div>
                  );
                })
              )}
            </div>
          </div>
        )}
      </div>

      {/* Selected batch periods summary */}
      <div className="flex items-center justify-between gap-2 border-t border-sp-line pt-1.5 text-xs">
        <span data-testid="wayback-selected-periods-count" className="text-sp-text-secondary tabular-nums">
          {t("mapWorkbench.wayback.selectedPeriodsLabel")}{" "}
          <strong className="font-mono text-sp-text">{selectedPeriods.length}</strong>{" "}
          {t("mapWorkbench.wayback.selectedPeriodsUnit")}
        </span>
        {selectedPeriods.length > 0 && (
          <button
            type="button"
            data-testid="wayback-clear-periods"
            onClick={() => onChangeSelectedPeriods([])}
            className="text-xs text-sp-text-muted hover:text-sp-text underline underline-offset-2 cursor-pointer"
          >
            {t("mapWorkbench.wayback.clearPeriods")}
          </button>
        )}
      </div>
    </div>
  );
}
