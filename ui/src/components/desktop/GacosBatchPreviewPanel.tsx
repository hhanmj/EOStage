import React, { useEffect, useMemo, useState } from "react";
import {
  AlertTriangle,
  CalendarDays,
  CheckCircle2,
  Clock,
  Layers,
  Loader2,
  Mail,
  RotateCcw,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { getCredentialStatus, type Bbox } from "@/lib/bridge";
import { formatSignedLat, formatSignedLon } from "@/lib/geoCoords";
import {
  parseGacosManualDatesList,
  splitGacosDatesIntoBatches,
  type GacosBatchSubmitState,
} from "@/lib/companionGacosFlow";
import {
  getWorkbenchToolOptions,
  subscribeWorkbenchToolOptions,
  type WorkbenchToolOptionsSnapshot,
} from "./ManualConditionsPanel";

export interface GacosBatchPreviewPanelProps {
  workingAoi: Bbox | null;
  aoiLabel?: string;
  startDate: string | null;
  endDate: string | null;
  gacosEmail?: string | null;
  batchStatuses?: ReadonlyMap<number, GacosBatchSubmitState>;
  selectedBatchIndices?: readonly number[] | null;
  onToggleBatchIndex?: (batchIndex: number) => void;
  onSelectAllBatches?: (totalBatches: number) => void;
  onRetryBatch?: (batchIndex: number, dates: readonly string[]) => void;
  submitting?: boolean;
}

function compactDate(raw?: string | null): string | null {
  const digits = (raw ?? "").replace(/[^0-9]/g, "");
  return /^\d{8}$/.test(digits) ? digits : null;
}

export function GacosBatchPreviewPanel({
  workingAoi,
  aoiLabel,
  startDate,
  endDate,
  gacosEmail,
  batchStatuses,
  selectedBatchIndices,
  onToggleBatchIndex,
  onSelectAllBatches,
  onRetryBatch,
  submitting = false,
}: GacosBatchPreviewPanelProps) {
  const [toolOptions, setToolOptions] = useState<WorkbenchToolOptionsSnapshot>(() =>
    getWorkbenchToolOptions()
  );
  const [resolvedEmail, setResolvedEmail] = useState<string | null>(gacosEmail ?? null);
  const [gacosAvailable, setGacosAvailable] = useState<boolean>(Boolean(gacosEmail));

  useEffect(() => {
    return subscribeWorkbenchToolOptions(() => {
      setToolOptions(getWorkbenchToolOptions());
    });
  }, []);

  useEffect(() => {
    if (gacosEmail !== undefined) {
      setResolvedEmail(gacosEmail);
      setGacosAvailable(Boolean(gacosEmail));
      return;
    }
    let cancelled = false;
    void getCredentialStatus()
      .then((res) => {
        if (cancelled || !res) return;
        const rec = res as unknown as {
          gacos?: string;
          gacos_email?: string | null;
        };
        const rawStatus = typeof rec.gacos === "string" ? rec.gacos.trim() : "";
        const statusMail =
          rawStatus && rawStatus !== "none" && rawStatus !== "unavailable" && rawStatus !== "available"
            ? rawStatus.startsWith("email:")
              ? rawStatus.slice(6).trim()
              : rawStatus.includes("@")
                ? rawStatus
                : null
            : null;
        const explicitMail =
          typeof rec.gacos_email === "string" && rec.gacos_email.trim()
            ? rec.gacos_email.trim()
            : null;
        const mail = explicitMail || statusMail;
        setResolvedEmail(mail);
        setGacosAvailable(
          Boolean(mail) ||
            (Boolean(rawStatus) && rawStatus !== "none" && rawStatus !== "unavailable")
        );
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [gacosEmail]);

  const parsedDates = useMemo(() => {
    const raw = (toolOptions.gacosDatesText || "").trim();
    if (raw) {
      return parseGacosManualDatesList(raw);
    }
    const fallback = Array.from(
      new Set([compactDate(startDate), compactDate(endDate)].filter((d): d is string => Boolean(d)))
    );
    return { validDates: fallback, invalidTokens: [] as string[] };
  }, [toolOptions.gacosDatesText, startDate, endDate]);

  const batches = useMemo(
    () => splitGacosDatesIntoBatches(parsedDates.validDates, 20),
    [parsedDates.validDates]
  );

  const utcHour = String(Number.parseInt(toolOptions.gacosUtcHour || "10", 10)).padStart(2, "0");
  const utcMinute = String(Number.parseInt(toolOptions.gacosUtcMinute || "0", 10)).padStart(2, "0");
  const formatLabel = toolOptions.gacosOutputFormat === "binary" ? "Binary Grid (.ztd)" : "GeoTIFF (.tif)";

  const aoiSpan = useMemo(() => {
    if (!workingAoi) return null;
    const lonSpan = Math.abs(workingAoi.east - workingAoi.west);
    const latSpan = Math.abs(workingAoi.north - workingAoi.south);
    const status =
      lonSpan > 10 || latSpan > 10
        ? "too_large"
        : lonSpan < 0.05 || latSpan < 0.05
          ? "too_small"
          : lonSpan <= 2 && latSpan <= 2
            ? "optimal"
            : "ok";
    return { lonSpan, latSpan, status };
  }, [workingAoi]);

  const activeSelectionSet = useMemo(() => {
    if (!selectedBatchIndices || selectedBatchIndices.length === 0) {
      return new Set(batches.map((_, idx) => idx));
    }
    return new Set(selectedBatchIndices);
  }, [selectedBatchIndices, batches]);

  return (
    <div
      data-testid="gacos-batch-preview-panel"
      className="flex-1 overflow-y-auto p-3 space-y-3 pb-20"
    >
      {/* Top summary card */}
      <div className="rounded-xl border border-sp-line bg-sp-surface p-3 space-y-2 text-xs">
        <div className="flex items-center justify-between gap-2">
          <div className="flex items-center gap-1.5 font-semibold text-sp-text">
            <Layers className="h-3.5 w-3.5 text-sp-accent shrink-0" />
            <span>GACOS 申请拆批与参数预检</span>
          </div>
          <span
            data-testid="gacos-batch-count-badge"
            className="rounded-full border border-sp-line bg-sp-surface-muted px-2 py-0.5 font-mono text-xs font-semibold text-sp-text whitespace-nowrap"
          >
            {batches.length > 0
              ? `共 ${batches.length} 批 · ${parsedDates.validDates.length} 个日期`
              : "暂未添加有效日期"}
          </span>
        </div>

        <div className="grid grid-cols-1 gap-1.5 text-xs text-sp-text-secondary">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <span className="inline-flex items-center gap-1">
              <Clock className="h-3.5 w-3.5 text-sp-text-muted" />
              <span>过境时刻：{utcHour}:{utcMinute} UTC</span>
            </span>
            <span className="font-mono text-sp-text">格式：{formatLabel}</span>
          </div>

          <div className="flex items-center justify-between gap-2">
            <span className="inline-flex items-center gap-1 min-w-0 truncate">
              <Mail className="h-3.5 w-3.5 text-sp-text-muted shrink-0" />
              <span className="truncate">
                通知邮箱：
                {resolvedEmail
                  ? resolvedEmail
                  : gacosAvailable
                    ? "已配置 GACOS 服务邮箱"
                    : "待配置（设置 → 数据服务账号）"}
              </span>
            </span>
          </div>

          {workingAoi && aoiSpan ? (
            <div className="rounded-lg bg-sp-surface-muted px-2.5 py-1.5 space-y-1">
              <div className="flex flex-wrap items-center justify-between gap-1 font-mono text-xs text-sp-text">
                <span>
                  {aoiLabel ? `${aoiLabel} · ` : ""}{formatSignedLon(workingAoi.west, 2)} ~ {formatSignedLon(workingAoi.east, 2)} / {formatSignedLat(workingAoi.south, 2)} ~ {formatSignedLat(workingAoi.north, 2)}
                </span>
                <span>{aoiSpan.lonSpan.toFixed(2)}° × {aoiSpan.latSpan.toFixed(2)}°</span>
              </div>
              <div
                className={cn(
                  "text-xs font-medium",
                  aoiSpan.status === "too_large" || aoiSpan.status === "too_small"
                    ? "text-sp-danger-text"
                    : "text-sp-success-text"
                )}
              >
                {aoiSpan.status === "too_large"
                  ? "超出官方 10° 最大限制，请缩小研究区"
                  : aoiSpan.status === "too_small"
                    ? "跨度小于 0.05°，请适当扩大研究区"
                    : aoiSpan.status === "optimal"
                      ? "推荐 2° × 2° 内高稳定性研究区范围"
                      : "符合 GACOS 0.05°–10° 范围要求"}
              </div>
            </div>
          ) : (
            <div className="rounded-lg bg-sp-warning-bg/50 px-2.5 py-1.5 text-xs text-sp-warning-text">
              请先在地图上绘制或通过行政区搜索设定研究区 AOI。
            </div>
          )}
        </div>

        {batches.length > 1 && onSelectAllBatches && (
          <div className="flex items-center justify-between pt-1 border-t border-sp-line/70">
            <span className="text-sp-text-muted">
              已勾选 {activeSelectionSet.size} / {batches.length} 批（每批最多 20 个日期）
            </span>
            <button
              type="button"
              data-testid="gacos-select-all-batches-btn"
              onClick={() => onSelectAllBatches(batches.length)}
              className="rounded-sp-sm border border-sp-line bg-sp-surface-muted px-2 py-0.5 text-xs font-medium text-sp-text hover:bg-sp-hover cursor-pointer"
            >
              全选批次
            </button>
          </div>
        )}
      </div>

      {parsedDates.invalidTokens.length > 0 && (
        <div className="rounded-xl border border-sp-danger-text/30 bg-sp-danger-bg px-3 py-2 text-xs text-sp-danger-text flex items-start gap-1.5">
          <AlertTriangle className="h-3.5 w-3.5 shrink-0 mt-0.5" />
          <span>
            已跳过无效日期格式：{parsedDates.invalidTokens.slice(0, 5).join("、")}
            （须为 8 位公历 YYYYMMDD）
          </span>
        </div>
      )}

      {/* Batch Cards */}
      {batches.length === 0 ? (
        <div className="rounded-xl border border-dashed border-sp-line bg-sp-surface-muted/50 px-4 py-8 text-center text-xs text-sp-text-muted space-y-1.5">
          <CalendarDays className="h-7 w-7 mx-auto text-sp-text-muted/80" />
          <p className="m-0 font-medium text-sp-text-secondary">
            在左侧输入 8 位日期或扫描本地 SLC 目录后，此处将自动拆批预览
          </p>
          <p className="m-0">单批最多包含 20 个日期，支持逐批勾选与单批重试</p>
        </div>
      ) : (
        <div className="space-y-2.5">
          {batches.map((dateChunk, idx) => {
            const batchState = batchStatuses?.get(idx);
            const status = batchState?.status ?? "pending";
            const checked = activeSelectionSet.has(idx);
            return (
              <div
                key={`gacos-batch-${idx}-${dateChunk[0]}`}
                data-testid={`gacos-batch-card-${idx}`}
                data-batch-status={status}
                className={cn(
                  "rounded-xl border bg-sp-surface p-3 space-y-2 text-xs transition-colors",
                  status === "failed"
                    ? "border-sp-danger-text/40"
                    : status === "submitted" || status === "duplicate"
                      ? "border-sp-success-text/40"
                      : checked
                        ? "border-sp-line"
                        : "border-sp-line/60 opacity-75"
                )}
              >
                <div className="flex items-center justify-between gap-2">
                  <label className="inline-flex items-center gap-2 font-semibold text-sp-text cursor-pointer min-w-0">
                    <input
                      type="checkbox"
                      aria-label={`选择第 ${idx + 1} 批`}
                      checked={checked}
                      disabled={submitting || status === "submitting"}
                      onChange={() => onToggleBatchIndex?.(idx)}
                      className="rounded border-sp-line accent-sp-accent"
                    />
                    <span className="truncate">
                      第 {idx + 1} / {batches.length} 批（{dateChunk.length} 个日期）
                    </span>
                  </label>

                  <span
                    data-testid={`gacos-batch-status-${idx}`}
                    className={cn(
                      "inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-xs font-semibold shrink-0",
                      status === "submitted" || status === "duplicate"
                        ? "bg-sp-success-bg text-sp-success-text"
                        : status === "failed"
                          ? "bg-sp-danger-bg text-sp-danger-text"
                          : status === "submitting"
                            ? "bg-sp-accent/15 text-sp-accent"
                            : "bg-sp-surface-muted text-sp-text-secondary border border-sp-line"
                    )}
                  >
                    {status === "submitting" && <Loader2 className="h-3 w-3 animate-spin" />}
                    {(status === "submitted" || status === "duplicate") && (
                      <CheckCircle2 className="h-3 w-3" />
                    )}
                    {status === "failed" && <AlertTriangle className="h-3 w-3" />}
                    <span>
                      {status === "submitting"
                        ? "提交中…"
                        : status === "submitted"
                          ? "已提交申请"
                          : status === "duplicate"
                            ? "已存在记录"
                            : status === "failed"
                              ? "提交失败"
                              : "待提交"}
                    </span>
                  </span>
                </div>

                <div className="flex flex-wrap gap-1">
                  {dateChunk.map((dt) => (
                    <span
                      key={dt}
                      className="rounded bg-sp-surface-muted border border-sp-line px-1.5 py-0.5 font-mono text-xs text-sp-text tabular-nums"
                    >
                      {dt}
                    </span>
                  ))}
                </div>

                <div className="flex flex-wrap items-center justify-between gap-2 text-xs text-sp-text-muted pt-0.5">
                  <span className="font-mono">
                    {utcHour}:{utcMinute} UTC · {formatLabel}
                  </span>
                  {batchState?.taskId && (
                    <span className="font-mono truncate max-w-[180px]" title={batchState.taskId}>
                      #{batchState.taskId}
                    </span>
                  )}
                </div>

                {status === "failed" && (
                  <div className="rounded-lg bg-sp-danger-bg px-2.5 py-1.5 flex items-center justify-between gap-2 text-xs text-sp-danger-text">
                    <span className="min-w-0 break-words">
                      {batchState?.error || "当前批次提交失败"}
                    </span>
                    {onRetryBatch && (
                      <button
                        type="button"
                        data-testid={`gacos-batch-retry-${idx}`}
                        disabled={submitting}
                        onClick={() => onRetryBatch(idx, dateChunk)}
                        className="shrink-0 inline-flex items-center gap-1 rounded-sp-sm border border-sp-danger-text/40 bg-sp-surface px-2 py-0.5 text-xs font-semibold text-sp-text hover:bg-sp-hover cursor-pointer"
                      >
                        <RotateCcw className="h-3 w-3" />
                        <span>重试此批</span>
                      </button>
                    )}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
