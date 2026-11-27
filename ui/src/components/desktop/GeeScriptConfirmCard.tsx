import React, { useEffect, useState } from "react";
import {
  AlertTriangle,
  CheckCircle2,
  Code2,
  Columns2,
  Download,
  Layers,
  Loader2,
  Play,
  ShieldAlert,
  ShieldCheck,
} from "lucide-react";
import { useI18n, type MessageKey, type MessageValues } from "@/i18n";
import {
  geeArtifactGet,
  geeChunkedDownload,
  geeScriptExecute,
  geeScriptPreview,
  pushGeeArtifactToMap,
  type GeeArtifactRecord,
  type GeeDataLayerRecord,
  type GeeScriptReviewFinding,
  type GeeTimeSeriesWindow,
} from "@/lib/geeCloudSuiteClient";

function extractDataLayerRecord(raw: unknown): GeeDataLayerRecord | null {
  if (!raw || typeof raw !== "object") {
    return null;
  }
  const rec = raw as Partial<GeeDataLayerRecord>;
  if (typeof rec.dataset_id !== "string" || rec.dataset_id.trim().length === 0) {
    return null;
  }
  return rec as GeeDataLayerRecord;
}

function formatProvenanceTimeRange(
  rawTimeRange: GeeDataLayerRecord["time_range"] | string | undefined,
  t: TranslateFn
): string {
  if (typeof rawTimeRange === "string") {
    return rawTimeRange.trim() || t("agent.gee.provenanceTimeless");
  }
  if (!rawTimeRange || typeof rawTimeRange !== "object") {
    return t("agent.gee.provenanceTimeless");
  }
  if (rawTimeRange.is_timeless) {
    return t("agent.gee.provenanceTimeless");
  }
  const start = typeof rawTimeRange.start_date === "string" ? rawTimeRange.start_date.trim() : "";
  const end = typeof rawTimeRange.end_date === "string" ? rawTimeRange.end_date.trim() : "";
  if (start && end) {
    return `${start}..${end}`;
  }
  return start || end || t("agent.gee.provenanceTimeless");
}

function renderProvenanceStrip(
  rawRecord: unknown,
  t: TranslateFn
): React.ReactNode {
  const record = extractDataLayerRecord(rawRecord);
  if (!record) {
    return null;
  }
  const sourceLabel =
    record.dataset_source === "official"
      ? t("agent.gee.provenanceSourceOfficial")
      : record.dataset_source === "community"
        ? t("agent.gee.provenanceSourceCommunity")
        : t("agent.gee.provenanceSourceUnknown");
  const scaleValue = record.analysis_scale_m ?? record.native_scale_m;
  const timeLabel = formatProvenanceTimeRange(record.time_range, t);

  return (
    <div
      data-testid="gee-provenance-strip"
      className="rounded-sp-md border border-sp-line bg-sp-surface-muted px-3 py-2 flex flex-col gap-1 text-xs text-sp-text-secondary"
    >
      <div className="flex flex-wrap items-center gap-2">
        <span className="font-semibold text-sp-text">{t("agent.gee.provenanceHeading")}</span>
        <span
          className={
            record.dataset_source === "community"
              ? "rounded-sp-sm bg-sp-warning-bg text-sp-warning-text px-2 py-0"
              : "rounded-sp-sm bg-sp-surface border border-sp-line text-sp-text-secondary px-2 py-0"
          }
        >
          {sourceLabel}
        </span>
        {record.deprecated && (
          <span className="rounded-sp-sm bg-sp-danger-bg text-sp-danger-text px-2 py-0">
            {t("mapWorkbench.gee.deprecatedBadge")}
          </span>
        )}
        <span className="font-mono text-sp-text truncate">{record.dataset_id}</span>
      </div>
      <div className="flex flex-wrap items-center gap-2 text-sp-text-muted">
        {typeof scaleValue === "number" && scaleValue > 0 && (
          <span>{t("agent.gee.provenanceScale", { scale: String(scaleValue) })}</span>
        )}
        <span>· {timeLabel}</span>
        {record.temporal_reducer && <span>· {record.temporal_reducer}</span>}
        {record.qa_mask_method && <span>· {record.qa_mask_method}</span>}
        {record.verification_status && (
          <span className="font-mono">· {record.verification_status}</span>
        )}
      </div>
    </div>
  );
}
import { formatBytes } from "@/shell/api/agentStreamLogic";

export interface GeeStreamCardProps {
  toolId: string;
  toolArgs?: Record<string, any>;
  summary: Record<string, any>;
  scriptPreview?: string | null;
  onViewOnMap?: () => void;
}

type TranslateFn = (key: MessageKey, values?: MessageValues) => string;

function extractWindowMean(pt: GeeTimeSeriesWindow): number | null {
  if (typeof pt?.mean === "number" && Number.isFinite(pt.mean)) {
    return pt.mean;
  }
  if (typeof pt?.stats?.mean === "number" && Number.isFinite(pt.stats.mean)) {
    return pt.stats.mean;
  }
  return null;
}

function renderTimeSeriesTable(series: GeeTimeSeriesWindow[], unit: string, t: TranslateFn) {
  if (!Array.isArray(series) || series.length === 0) {
    return <div className="text-xs text-sp-text-muted">{t("agent.gee.tsEmpty")}</div>;
  }

  const validMeans = series
    .map(extractWindowMean)
    .filter((v): v is number => v !== null);
  const minVal = validMeans.length > 0 ? Math.min(...validMeans) : 0;
  const maxVal = validMeans.length > 0 ? Math.max(...validMeans) : 1;
  const span = maxVal - minVal;

  return (
    <div
      data-testid="gee-time-series-table"
      className="flex flex-col gap-2 rounded-sp-md border border-sp-line bg-sp-surface-muted p-3"
    >
      <div className="flex items-center justify-between text-xs text-sp-text-secondary font-semibold pb-1 border-b border-sp-divider">
        <span>{t("agent.gee.tsHeading", { count: series.length })}</span>
        {unit && <span className="font-mono text-sp-text-muted">{unit}</span>}
      </div>
      <div className="flex flex-col gap-2">
        {series.map((pt, idx) => {
          const mean = extractWindowMean(pt);
          const barPct =
            mean === null
              ? 0
              : span > 1e-6
              ? Math.max(8, Math.min(100, Math.round(((mean - minVal) / span) * 92 + 8)))
              : 65;
          const sceneCount = typeof pt.scene_count === "number" ? pt.scene_count : 0;

          return (
            <div
              key={`${pt.window_start}-${pt.window_end}-${idx}`}
              data-testid={`gee-time-series-row-${idx}`}
              className="grid grid-cols-[minmax(148px,auto)_56px_1fr_auto] items-center gap-2 text-xs"
            >
              <span className="font-mono text-sp-text truncate">
                {pt.window_start} ~ {pt.window_end}
              </span>
              <span className="font-mono text-sp-text-muted text-right">
                {t("agent.gee.tsSceneCount", { count: sceneCount })}
              </span>
              <div className="h-2 w-full rounded-full bg-sp-surface border border-sp-line overflow-hidden">
                {mean !== null && (
                  <div
                    data-testid={`gee-time-series-bar-${idx}`}
                    className="h-full rounded-full bg-sp-accent"
                    style={{ width: `${barPct}%` }}
                  />
                )}
              </div>
              <span className="font-mono text-sp-text font-semibold text-right min-w-[64px]">
                {mean !== null ? mean.toFixed(4) : t("agent.gee.tsNoImage")}
              </span>
            </div>
          );
        })}
      </div>
    </div>
  );
}

function renderStatsBadges(
  stats: Record<string, any> | undefined,
  unit: string,
  t: TranslateFn
) {
  if (!stats || typeof stats !== "object") return null;
  const entries: Array<{ label: string; value: string; highlight?: boolean }> = [];
  if (typeof stats.threshold_area_km2 === "number") {
    entries.push({
      label: t("agent.gee.statArea"),
      value: `${stats.threshold_area_km2.toFixed(3)} km²`,
      highlight: true,
    });
  }
  if (typeof stats.mean === "number") {
    entries.push({
      label: t("agent.gee.statMean"),
      value: `${stats.mean.toFixed(4)} ${unit}`.trim(),
    });
  }
  if (typeof stats.min === "number") {
    entries.push({ label: t("agent.gee.statMin"), value: stats.min.toFixed(4) });
  }
  if (typeof stats.max === "number") {
    entries.push({ label: t("agent.gee.statMax"), value: stats.max.toFixed(4) });
  }
  if (entries.length === 0) return null;
  const kvSep = t("agent.gee.statKvSep");

  return (
    <div className="flex flex-wrap gap-2">
      {entries.map((entry) => (
        <span
          key={entry.label}
          className={
            entry.highlight
              ? "rounded-sp-sm border border-sp-accent/40 bg-sp-accent/10 px-2 py-1 text-xs font-semibold text-sp-accent"
              : "rounded-sp-sm border border-sp-line bg-sp-surface-muted px-2 py-1 text-xs text-sp-text-secondary"
          }
        >
          {entry.label}
          {kvSep}
          {entry.value}
        </span>
      ))}
    </div>
  );
}

export function GeeScriptConfirmCard({
  toolId,
  toolArgs = {},
  summary,
  scriptPreview,
  onViewOnMap,
}: GeeStreamCardProps) {
  const { locale, t } = useI18n();
  const propArtifactId =
    typeof summary?.artifact_id === "string" ? summary.artifact_id : null;
  const approvalId = typeof summary?.approval_id === "string" ? summary.approval_id : "";
  const quotaConfirmKey = summary?.quota_confirm_required === true ? "quota" : "";
  const preflightKey =
    toolId === "gee_export_preflight"
      ? `${typeof summary?.asset_id === "string" ? summary.asset_id : ""}::${typeof summary?.recommended_route === "string" ? summary.recommended_route : ""}`
      : "";
  const cardIdentityKey = `${toolId}::${approvalId}::${propArtifactId ?? ""}::${quotaConfirmKey}::${preflightKey}`;

  const [trackedIdentity, setTrackedIdentity] = useState<string>(cardIdentityKey);
  const [artifactId, setArtifactId] = useState<string | null>(propArtifactId);
  const [artifactRecord, setArtifactRecord] = useState<GeeArtifactRecord | null>(null);
  const [busy, setBusy] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [scriptDoneResult, setScriptDoneResult] = useState<Record<string, any> | null>(null);
  const [downloadDoneSummary, setDownloadDoneSummary] = useState<Record<string, any> | null>(null);
  const [showCode, setShowCode] = useState(true);
  const [resolvedPreview, setResolvedPreview] = useState<{
    approvalId: string;
    script: string | null;
    settled: boolean;
  }>({ approvalId: "", script: null, settled: false });

  if (trackedIdentity !== cardIdentityKey) {
    setTrackedIdentity(cardIdentityKey);
    setArtifactId(propArtifactId);
    setArtifactRecord(null);
    setBusy(false);
    setErrorMsg(null);
    setScriptDoneResult(null);
    setDownloadDoneSummary(null);
    setResolvedPreview({ approvalId: "", script: null, settled: false });
  }

  const artifact =
    artifactId && artifactRecord?.artifact_id === artifactId ? artifactRecord : null;

  useEffect(() => {
    if (toolId !== "gee_script_inspect" || !approvalId || scriptDoneResult) {
      return;
    }
    let cancelled = false;
    void geeScriptPreview(approvalId)
      .then((res) => {
        if (!cancelled) {
          const hydrated =
            typeof res?.script === "string" && res.script.trim().length > 0
              ? res.script
              : null;
          setResolvedPreview({
            approvalId,
            script: hydrated,
            settled: true,
          });
        }
      })
      .catch(() => {
        // Non-fatal in lightweight test fixtures without approval registry entry
        if (!cancelled) {
          setResolvedPreview({
            approvalId,
            script: null,
            settled: true,
          });
        }
      });
    return () => {
      cancelled = true;
    };
  }, [toolId, approvalId, scriptDoneResult]);

  useEffect(() => {
    if (!artifactId) {
      setArtifactRecord(null);
      return;
    }
    let cancelled = false;
    void geeArtifactGet(artifactId)
      .then((rec) => {
        if (!cancelled && rec?.artifact_id === artifactId) {
          setArtifactRecord(rec);
        }
      })
      .catch(() => {
        // Non-fatal in lightweight test fixtures without artifact persistence
      });
    return () => {
      cancelled = true;
    };
  }, [artifactId]);

  // 1) Custom Python `ee` Script Security & Sanity Confirmation Card
  if (toolId === "gee_script_inspect" && !scriptDoneResult) {
    const blockedReasons: string[] = Array.isArray(summary?.blocked_reasons)
      ? summary.blocked_reasons
      : [];
    const warnings: string[] = Array.isArray(summary?.scientific_warnings)
      ? summary.scientific_warnings
      : [];
    const findings: GeeScriptReviewFinding[] = Array.isArray(summary?.findings)
      ? summary.findings.filter(
          (item: unknown): item is GeeScriptReviewFinding =>
            Boolean(item && typeof item === "object" && typeof (item as { message?: unknown }).message === "string")
        )
      : [];
    const isBlocked = blockedReasons.length > 0 || !approvalId;
    const rawFallbackCode = scriptPreview || String(toolArgs?.script || "");
    const previewSettled =
      resolvedPreview.approvalId === approvalId && resolvedPreview.settled;
    const currentResolvedScript =
      resolvedPreview.approvalId === approvalId ? resolvedPreview.script : null;
    const awaitingRedactedHydration =
      Boolean(approvalId) &&
      !previewSettled &&
      !currentResolvedScript &&
      rawFallbackCode.includes("[local-reference]");
    const codeText =
      currentResolvedScript || (awaitingRedactedHydration ? "" : rawFallbackCode);
    const envReady = summary?.env_ready === true;

    const handleApproveRun = async () => {
      if (isBlocked || awaitingRedactedHydration || !codeText) return;
      setBusy(true);
      setErrorMsg(null);
      try {
        const bbox = Array.isArray(toolArgs?.bbox)
          ? (toolArgs.bbox as [number, number, number, number])
          : undefined;
        const res = await geeScriptExecute(approvalId, codeText, bbox);
        setScriptDoneResult(res);
        if (typeof res?.artifact_id === "string") {
          setArtifactId(res.artifact_id);
        }
      } catch (err: any) {
        setErrorMsg(err?.message || String(err));
      } finally {
        setBusy(false);
      }
    };

    return (
      <div
        data-testid="gee-script-confirm-card"
        className="w-full rounded-sp-card border border-sp-line bg-sp-surface p-3 shadow-sp-card flex flex-col gap-3"
      >
        <div className="flex items-center justify-between gap-2">
          <div className="flex items-center gap-2 text-xs font-semibold text-sp-text">
            {isBlocked ? (
              <ShieldAlert className="h-4 w-4 text-sp-danger-text shrink-0" />
            ) : (
              <ShieldCheck className="h-4 w-4 text-sp-accent shrink-0" />
            )}
            <span>{t("agent.gee.scriptConfirmTitle")}</span>
          </div>
          <button
            type="button"
            onClick={() => setShowCode((v) => !v)}
            className="text-[13px] text-sp-text-muted hover:text-sp-text inline-flex items-center gap-1 cursor-pointer"
          >
            <Code2 className="h-3.5 w-3.5" />
            <span>{showCode ? t("agent.gee.hideCode") : t("agent.gee.showCode")}</span>
          </button>
        </div>

        {isBlocked ? (
          <div className="rounded-sp-md border border-sp-danger-text/30 bg-sp-danger-bg p-3 text-xs text-sp-danger-text flex flex-col gap-1">
            <span className="font-semibold">{t("agent.gee.scriptBlockedHeading")}</span>
            {blockedReasons.map((reason, idx) => (
              <span key={idx}>· {reason}</span>
            ))}
          </div>
        ) : (
          <div className="text-xs text-sp-text-secondary leading-relaxed">
            {t("agent.gee.scriptPassedHint")}
            {!envReady && (
              <span className="block mt-1 text-sp-text-muted">
                {t("agent.gee.scriptEnvFirstRunHint")}
              </span>
            )}
          </div>
        )}

        {findings.length > 0 ? (
          <div
            data-testid="gee-script-findings"
            className="rounded-sp-md border border-sp-warning-text/30 bg-sp-surface-muted p-3 text-xs flex flex-col gap-2"
          >
            <div className="flex items-center gap-2 font-semibold text-sp-text">
              <AlertTriangle className="h-4 w-4 text-sp-warning-text shrink-0" />
              <span>{t("agent.gee.findingsHeading")}</span>
            </div>
            <div className="flex flex-col gap-1">
              {findings.map((finding, idx) => {
                const sev = String(finding.severity || "").toUpperCase();
                const sevLabel =
                  sev === "ERROR"
                    ? t("agent.gee.findingSeverityError")
                    : sev === "INFO"
                      ? t("agent.gee.findingSeverityInfo")
                      : t("agent.gee.findingSeverityWarn");
                const sevClass =
                  sev === "ERROR"
                    ? "bg-sp-danger-bg text-sp-danger-text"
                    : sev === "INFO"
                      ? "bg-sp-surface border border-sp-line text-sp-text-secondary"
                      : "bg-sp-warning-bg text-sp-warning-text";
                return (
                  <div
                    key={idx}
                    className="flex flex-wrap items-baseline gap-2 text-xs text-sp-text-secondary"
                  >
                    <span className={`rounded-sp-sm px-2 py-0 font-semibold ${sevClass}`}>
                      {sevLabel}
                    </span>
                    {typeof finding.line === "number" && finding.line > 0 && (
                      <span className="font-mono text-sp-text-muted">L{finding.line}</span>
                    )}
                    <span className="text-sp-text">{finding.message}</span>
                  </div>
                );
              })}
            </div>
          </div>
        ) : (
          warnings.length > 0 && (
            <div className="rounded-sp-md border border-sp-warning-text/30 bg-sp-warning-bg p-3 text-xs text-sp-warning-text flex flex-col gap-1">
              <div className="flex items-center gap-2 font-semibold">
                <AlertTriangle className="h-3.5 w-3.5 shrink-0" />
                <span>{t("agent.gee.calibrationWarningHeading")}</span>
              </div>
              {warnings.map((w, idx) => (
                <span key={idx}>· {w}</span>
              ))}
            </div>
          )
        )}

        {showCode && (codeText || awaitingRedactedHydration) && (
          <pre
            data-testid="gee-script-code-preview"
            className="max-h-48 overflow-auto rounded-sp-md border border-sp-line bg-sp-surface-muted p-3 text-xs font-mono text-sp-text leading-relaxed"
          >
            {awaitingRedactedHydration ? t("agent.gee.hydratingScriptPreview") : codeText}
          </pre>
        )}

        {errorMsg && (
          <div role="alert" className="text-xs text-sp-danger-text break-words">
            {errorMsg}
          </div>
        )}

        {!isBlocked && (
          <div className="flex items-center justify-end gap-2 pt-1">
            <button
              type="button"
              disabled={busy || awaitingRedactedHydration}
              data-testid="gee-approve-run-script-btn"
              onClick={() => void handleApproveRun()}
              className="h-8 px-4 rounded-sp-card bg-sp-accent-fill text-white text-[13px] font-semibold hover:opacity-90 disabled:opacity-50 inline-flex items-center gap-2 cursor-pointer"
            >
              {busy ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <Play className="h-3.5 w-3.5" />
              )}
              <span>
                {busy ? t("agent.gee.runningScriptBtn") : t("agent.gee.approveRunScriptBtn")}
              </span>
            </button>
          </div>
        )}
      </div>
    );
  }

  // 1.5) Offline Export Preflight Gate Card (must precede quota guardrail check)
  if (toolId === "gee_export_preflight") {
    const isReady = summary?.ready === true;
    const rawGrid = (summary?.grid_plan || {}) as Record<string, unknown>;
    const rawWidth =
      typeof summary?.raw_width_px === "number"
        ? summary.raw_width_px
        : Number(rawGrid.full_width ?? 0);
    const rawHeight =
      typeof summary?.raw_height_px === "number"
        ? summary.raw_height_px
        : Number(rawGrid.full_height ?? 0);
    const rawChunks =
      typeof summary?.raw_total_chunks === "number"
        ? summary.raw_total_chunks
        : Number(rawGrid.total_chunks ?? 0);
    const estimatedMb =
      typeof summary?.estimated_mb === "number"
        ? summary.estimated_mb
        : Number(rawGrid.estimated_mb ?? 0);
    const tileSafety = typeof summary?.tile_safety === "string" ? summary.tile_safety : "";
    const tileSafetyReason =
      typeof summary?.tile_safety_reason === "string" ? summary.tile_safety_reason : "";
    const recommendedRoute =
      typeof summary?.recommended_route === "string" ? summary.recommended_route : "";
    const routeLabel =
      recommendedRoute === "needs_parameters"
        ? t("agent.gee.preflightRouteNeedsParams")
        : recommendedRoute === "external_batch_export"
          ? t("agent.gee.preflightRouteExternal")
          : recommendedRoute === "local_chunked_geotiff_confirm"
            ? t("agent.gee.preflightRouteConfirm")
            : recommendedRoute === "local_chunked_geotiff_auto"
              ? t("agent.gee.preflightRouteAuto")
              : recommendedRoute;
    const missingParams = Array.isArray(summary?.missing_parameters)
      ? summary.missing_parameters.filter((x: unknown): x is string => typeof x === "string")
      : [];
    const preflightWarnings = Array.isArray(summary?.warnings)
      ? summary.warnings.filter(
          (x: unknown): x is string =>
            typeof x === "string" &&
            x.trim().length > 0 &&
            x.trim() !== tileSafetyReason.trim()
        )
      : [];
    const safeMb = Number.isFinite(estimatedMb) ? estimatedMb.toFixed(1) : "0.0";

    return (
      <div
        data-testid="gee-export-preflight-card"
        className="w-full rounded-sp-card border border-sp-line bg-sp-surface p-3 shadow-sp-card flex flex-col gap-3"
      >
        <div className="flex items-center justify-between gap-2">
          <div className="flex items-center gap-2 text-xs font-semibold text-sp-text">
            {isReady ? (
              <ShieldCheck className="h-4 w-4 text-sp-accent shrink-0" />
            ) : (
              <AlertTriangle className="h-4 w-4 text-sp-status-warning shrink-0" />
            )}
            <span>{t("agent.gee.preflightCardTitle")}</span>
          </div>
          <span
            className={
              isReady
                ? "shrink-0 rounded-full bg-sp-accent/10 px-2 py-0 text-xs font-semibold text-sp-accent"
                : "shrink-0 rounded-full bg-sp-warning-bg px-2 py-0 text-xs font-semibold text-sp-warning-text"
            }
          >
            {isReady
              ? t("agent.gee.preflightReadyBadge")
              : t("agent.gee.preflightBlockedBadge")}
          </span>
        </div>

        <div className="flex flex-wrap items-center gap-2 text-xs text-sp-text-secondary">
          {(rawWidth > 0 || rawHeight > 0 || rawChunks > 0) && (
            <span className="font-mono text-sp-text">
              {t("agent.gee.preflightGridSummary", {
                width: String(Math.round(rawWidth)),
                height: String(Math.round(rawHeight)),
                chunks: String(Math.round(rawChunks)),
                mb: safeMb,
              })}
            </span>
          )}
          {tileSafety && (
            <span
              className={
                tileSafety === "tile_unsafe"
                  ? "rounded-sp-sm bg-sp-danger-bg px-2 py-0 font-semibold text-sp-danger-text"
                  : "rounded-sp-sm border border-sp-line bg-sp-surface-muted px-2 py-0 text-sp-text-secondary"
              }
            >
              {tileSafety === "tile_unsafe"
                ? t("agent.gee.preflightTileUnsafe")
                : t("agent.gee.preflightTileSafe")}
            </span>
          )}
          {routeLabel && (
            <span className="text-sp-text font-semibold">
              {t("agent.gee.preflightRoutePrefix", { route: routeLabel })}
            </span>
          )}
        </div>

        {tileSafetyReason && (
          <div className="text-xs text-sp-text-secondary leading-relaxed">
            {tileSafetyReason}
          </div>
        )}

        {missingParams.length > 0 && (
          <div className="rounded-sp-md border border-sp-warning-text/30 bg-sp-warning-bg p-3 text-xs text-sp-warning-text">
            {t("agent.gee.preflightMissingParams", { params: missingParams.join(", ") })}
          </div>
        )}

        {preflightWarnings.length > 0 && (
          <div className="rounded-sp-md border border-sp-warning-text/30 bg-sp-warning-bg p-3 text-xs text-sp-warning-text flex flex-col gap-1">
            {preflightWarnings.map((w, idx) => (
              <span key={idx}>· {w}</span>
            ))}
          </div>
        )}

        {renderProvenanceStrip(summary?.data_layer_record, t)}
      </div>
    );
  }

  // 2) Quota & Scientific Resolution Guardrail Choice Card (> 9 chunks)
  if (summary?.quota_confirm_required === true && !downloadDoneSummary) {
    const plan = summary?.grid_plan || {};
    const handleResolutionChoice = async (fullRes: boolean) => {
      setBusy(true);
      setErrorMsg(null);
      try {
        const res = await geeChunkedDownload({
          asset_id: String(
            summary?.asset_id || toolArgs?.asset_id || "COPERNICUS/S2_SR_HARMONIZED"
          ),
          index_name: toolArgs?.index_name,
          bands: toolArgs?.bands,
          reducer: toolArgs?.reducer,
          bbox: plan?.bbox || toolArgs?.bbox,
          start_date: toolArgs?.start_date,
          end_date: toolArgs?.end_date,
          max_cloud: toolArgs?.max_cloud,
          scale_m: toolArgs?.scale_m,
          confirm_full_resolution: fullRes,
          use_preview_scale: !fullRes,
        });
        setDownloadDoneSummary(res);
        if (typeof res?.artifact_id === "string") {
          setArtifactId(res.artifact_id);
        }
      } catch (err: any) {
        setErrorMsg(err?.message || String(err));
      } finally {
        setBusy(false);
      }
    };

    return (
      <div
        data-testid="gee-quota-confirm-card"
        className="w-full rounded-sp-card border border-sp-line bg-sp-surface p-3 shadow-sp-card flex flex-col gap-3"
      >
        <div className="flex items-center gap-2 text-xs font-semibold text-sp-text">
          <AlertTriangle className="h-4 w-4 text-sp-status-warning shrink-0" />
          <span>{t("agent.gee.quotaGuardrailTitle")}</span>
        </div>
        <p className="text-xs text-sp-text-secondary leading-relaxed">
          {locale === "zh-CN" && summary?.message_zh
            ? summary.message_zh
            : t("agent.gee.quotaGuardrailFallback", {
                scale: plan.requested_scale_m ?? 10,
                chunks: plan.total_chunks ?? 12,
                width: plan.full_width ?? 0,
                height: plan.full_height ?? 0,
                mb: Number(plan.estimated_mb ?? 0).toFixed(1),
              })}
        </p>
        {errorMsg && (
          <div role="alert" className="text-xs text-sp-danger-text break-words">
            {errorMsg}
          </div>
        )}
        <div className="flex flex-wrap items-center gap-2 pt-1">
          <button
            type="button"
            disabled={busy}
            data-testid="gee-confirm-full-res-btn"
            onClick={() => void handleResolutionChoice(true)}
            className="h-8 px-3 rounded-sp-card bg-sp-accent-fill text-white text-[13px] font-semibold hover:opacity-90 disabled:opacity-50 inline-flex items-center gap-2 cursor-pointer"
          >
            {busy ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <Download className="h-3.5 w-3.5" />
            )}
            <span>
              {t("agent.gee.fullResMosaicBtn", {
                scale: plan.requested_scale_m ?? 10,
                chunks: plan.total_chunks ?? 12,
              })}
            </span>
          </button>
          <button
            type="button"
            disabled={busy}
            data-testid="gee-confirm-preview-res-btn"
            onClick={() => void handleResolutionChoice(false)}
            className="h-8 px-3 rounded-sp-card border border-sp-line bg-sp-surface text-[13px] font-semibold text-sp-text hover:bg-sp-hover disabled:opacity-50 inline-flex items-center gap-2 cursor-pointer"
          >
            <span>
              {t("agent.gee.previewResBtn", {
                scale: plan.suggested_preview_scale_m ?? 30,
              })}
            </span>
          </button>
        </div>
      </div>
    );
  }

  // 3) Artifact Card (Cloud Analysis / Change Detection / Time Series / Stitched GeoTIFF / Executed Script)
  const effectiveSummary = downloadDoneSummary || scriptDoneResult || summary || {};
  const effectiveOperation = String(
    artifact?.operation || effectiveSummary?.operation || toolArgs?.operation || ""
  );
  const rawSeries: GeeTimeSeriesWindow[] = Array.isArray(effectiveSummary?.series)
    ? effectiveSummary.series
    : Array.isArray(artifact?.series)
    ? artifact.series
    : Array.isArray(artifact?.stats?.series)
    ? artifact.stats.series
    : Array.isArray(effectiveSummary?.stats?.series)
    ? effectiveSummary.stats.series
    : [];
  const isTimeSeries = effectiveOperation === "time_series" || rawSeries.length > 0;
  const unitStr = String(
    artifact?.unit ||
      effectiveSummary?.unit ||
      effectiveSummary?.index_name ||
      toolArgs?.index_name ||
      ""
  );
  const title =
    artifact?.title ||
    effectiveSummary?.title ||
    effectiveSummary?.file_name ||
    (isTimeSeries
      ? t("agent.gee.defaultTimeSeriesTitle")
      : t("agent.gee.defaultArtifactTitle"));
  const hasSwipePair = Boolean(
    artifact &&
      (artifact.before_tile_url_template || artifact.before_preview_data_url) &&
      (artifact.after_tile_url_template || artifact.after_preview_data_url)
  );
  const statsObj = artifact?.stats || effectiveSummary?.stats || effectiveSummary?.result;
  const bytesWritten =
    artifact?.size_bytes ??
    (typeof effectiveSummary?.bytes_written === "number"
      ? effectiveSummary.bytes_written
      : null);

  return (
    <div
      data-testid="gee-artifact-card"
      className="w-full rounded-sp-card border border-sp-line bg-sp-surface p-3 shadow-sp-card flex flex-col gap-3"
    >
      <div className="flex items-center justify-between gap-2">
        <div className="flex items-center gap-2 min-w-0">
          <CheckCircle2 className="h-4 w-4 text-sp-success-text shrink-0" />
          <span className="text-xs font-semibold text-sp-text truncate">{title}</span>
        </div>
        {typeof bytesWritten === "number" && bytesWritten > 0 && (
          <span className="shrink-0 rounded-full bg-sp-surface-muted px-2 py-1 text-xs font-mono text-sp-text-secondary">
            GeoTIFF · {formatBytes(bytesWritten)}
          </span>
        )}
      </div>

      {/* Inline Thumbnail(s) or Time-Series Visualization */}
      {hasSwipePair && artifact ? (
        <div className="grid grid-cols-3 gap-2">
          {artifact.before_preview_data_url && (
            <div className="flex flex-col gap-1">
              <img
                src={artifact.before_preview_data_url}
                alt={t("agent.gee.beforePreviewAlt")}
                className="h-28 w-full rounded-sp-md border border-sp-line object-cover bg-sp-surface-muted"
              />
              <span className="text-xs text-center text-sp-text-muted">
                {t("agent.gee.beforeLabel")}
              </span>
            </div>
          )}
          {artifact.after_preview_data_url && (
            <div className="flex flex-col gap-1">
              <img
                src={artifact.after_preview_data_url}
                alt={t("agent.gee.afterPreviewAlt")}
                className="h-28 w-full rounded-sp-md border border-sp-line object-cover bg-sp-surface-muted"
              />
              <span className="text-xs text-center text-sp-text-muted">
                {t("agent.gee.afterLabel")}
              </span>
            </div>
          )}
          {artifact.preview_data_url && (
            <div className="flex flex-col gap-1">
              <img
                src={artifact.preview_data_url}
                alt={t("agent.gee.diffPreviewAlt")}
                className="h-28 w-full rounded-sp-md border border-sp-line object-cover bg-sp-surface-muted"
              />
              <span className="text-xs text-center text-sp-text-muted">
                {t("agent.gee.diffLabel")}
              </span>
            </div>
          )}
        </div>
      ) : artifact?.preview_data_url ? (
        <div className="flex items-center gap-3">
          <img
            src={artifact.preview_data_url}
            alt={title}
            className="h-32 w-32 rounded-sp-md border border-sp-line object-cover bg-sp-surface-muted shrink-0"
          />
          <div className="flex flex-col gap-2 min-w-0 flex-1">
            {renderStatsBadges(statsObj, unitStr, t)}
            {typeof effectiveSummary?.file_name === "string" && (
              <span className="text-xs font-mono text-sp-text-muted truncate">
                {t("agent.gee.filePrefix", { name: effectiveSummary.file_name })}
              </span>
            )}
            {typeof effectiveSummary?.chunks_stitched === "number" && (
              <span className="text-xs text-sp-text-muted">
                {t("agent.gee.chunksStitchedSummary", {
                  chunks: effectiveSummary.chunks_stitched,
                })}
              </span>
            )}
          </div>
        </div>
      ) : isTimeSeries ? (
        <div className="flex flex-col gap-2">
          {renderTimeSeriesTable(rawSeries, unitStr, t)}
          {renderStatsBadges(statsObj, unitStr, t)}
        </div>
      ) : (
        renderStatsBadges(statsObj, unitStr, t)
      )}

      {renderProvenanceStrip(
        artifact?.data_layer_record ??
          effectiveSummary?.data_layer_record ??
          summary?.data_layer_record,
        t
      )}

      {artifact && (artifact.tile_url_template || artifact.preview_data_url) && (
        <div className="flex flex-wrap items-center gap-2 pt-1">
          <button
            type="button"
            data-testid="gee-push-to-map-btn"
            onClick={() => {
              pushGeeArtifactToMap(artifact, "single");
              onViewOnMap?.();
            }}
            className="h-7 px-3 rounded-sp-md border border-sp-line bg-sp-surface text-[13px] font-semibold text-sp-text hover:bg-sp-hover inline-flex items-center gap-2 cursor-pointer"
          >
            <Layers className="h-3.5 w-3.5 text-sp-accent" />
            <span>{t("agent.gee.pushToMapBtn")}</span>
          </button>
          {hasSwipePair && (
            <button
              type="button"
              data-testid="gee-push-swipe-btn"
              onClick={() => {
                pushGeeArtifactToMap(artifact, "swipe");
                onViewOnMap?.();
              }}
              className="h-7 px-3 rounded-sp-md bg-sp-accent/12 border border-sp-accent/40 text-[13px] font-semibold text-sp-accent hover:bg-sp-accent/18 inline-flex items-center gap-2 cursor-pointer"
            >
              <Columns2 className="h-3.5 w-3.5" />
              <span>{t("agent.gee.enableSwipeBtn")}</span>
            </button>
          )}
        </div>
      )}
    </div>
  );
}
