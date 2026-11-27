import React, { useMemo, useState } from "react";
import { Activity, Cpu, Database, Download, Globe, Wrench } from "lucide-react";
import { useI18n } from "@/i18n";
import { EmptyState } from "@/components/ui/EmptyState";
import { EmptyStateIcon } from "@/components/ui/EmptyStateVisuals";

export interface DailyUsageSeriesPoint {
  date: string;
  prompt_tokens: number;
  completion_tokens: number;
  reasoning_tokens: number;
  runs: number;
  download_bytes: number;
}

export interface ToolUsageBreakdownRow {
  tool_id: string;
  call_count: number;
  success_count: number;
}

export interface ProviderDownloadBreakdownRow {
  source_id: string;
  display_name: string;
  task_count: number;
  bytes_transferred: number;
}

export interface GeeQuotaStatusSummary {
  configured: boolean;
  verified: boolean;
  project_id?: string | null;
  auth_mode: string;
}

export interface ModelUsageBreakdownItem {
  route: string;
  channel_id: string;
  model_id: string;
  run_count: number;
  model_calls: number;
  tool_calls: number;
  prompt_tokens: number;
  completion_tokens: number;
  reasoning_tokens: number;
  cached_tokens: number;
  total_tokens: number;
}

export interface UsageAnalyticsDashboardStats {
  collected_at?: string;
  prompt_tokens: number;
  completion_tokens: number;
  reasoning_tokens: number;
  cached_tokens: number;
  total_tokens: number;
  run_count: number;
  download_total_tasks: number;
  download_completed_tasks: number;
  download_active_tasks: number;
  download_failed_tasks: number;
  download_transferred_bytes: number;
  model_breakdown?: ModelUsageBreakdownItem[];
  daily_series?: DailyUsageSeriesPoint[];
  tool_breakdown?: ToolUsageBreakdownRow[];
  provider_download_breakdown?: ProviderDownloadBreakdownRow[];
  gee_quota_status?: GeeQuotaStatusSummary | null;
}

interface UsageAnalyticsDashboardProps {
  stats: UsageAnalyticsDashboardStats | null;
  loading?: boolean;
  formatBytes?: (bytes?: number | null) => string;
}

const DONUT_COLORS = [
  "#0ea5e9",
  "#10b981",
  "#8b5cf6",
  "#f59e0b",
  "#ec4899",
  "#64748b",
];

function defaultFormatBytes(bytes?: number | null): string {
  if (typeof bytes !== "number" || !Number.isFinite(bytes) || bytes < 0) return "—";
  if (bytes === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let val = bytes;
  let idx = 0;
  while (val >= 1024 && idx < units.length - 1) {
    val /= 1024;
    idx += 1;
  }
  return `${val >= 100 || idx === 0 ? Math.round(val) : val.toFixed(1)} ${units[idx]}`;
}

function formatCompactNumber(value: number): string {
  if (!Number.isFinite(value)) return "0";
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(1)}K`;
  return String(Math.round(value));
}

function buildRecentEmptySeries(): DailyUsageSeriesPoint[] {
  const days: DailyUsageSeriesPoint[] = [];
  const today = new Date();
  for (let i = 29; i >= 0; i -= 1) {
    const dt = new Date(Date.UTC(today.getUTCFullYear(), today.getUTCMonth(), today.getUTCDate() - i));
    days.push({
      date: dt.toISOString().slice(0, 10),
      prompt_tokens: 0,
      completion_tokens: 0,
      reasoning_tokens: 0,
      runs: 0,
      download_bytes: 0,
    });
  }
  return days;
}

export const UsageAnalyticsDashboard: React.FC<UsageAnalyticsDashboardProps> = ({
  stats,
  loading = false,
  formatBytes = defaultFormatBytes,
}) => {
  const { t } = useI18n();
  const [chartMode, setChartMode] = useState<"tokens" | "downloads">("tokens");
  const [hoveredDayIndex, setHoveredDayIndex] = useState<number | null>(null);
  const [hoveredDonutIndex, setHoveredDonutIndex] = useState<number | null>(null);

  const series = useMemo<DailyUsageSeriesPoint[]>(() => {
    if (stats?.daily_series && stats.daily_series.length > 0) {
      return stats.daily_series;
    }
    return buildRecentEmptySeries();
  }, [stats?.daily_series]);

  const hasChartData = useMemo(() => {
    if (chartMode === "tokens") {
      return series.some(
        (d) =>
          d.prompt_tokens > 0 ||
          d.completion_tokens > 0 ||
          d.reasoning_tokens > 0 ||
          d.runs > 0,
      );
    }
    return series.some((d) => d.download_bytes > 0);
  }, [series, chartMode]);

  const maxChartValue = useMemo(() => {
    if (chartMode === "tokens") {
      return Math.max(
        1,
        ...series.map((d) => d.prompt_tokens + d.completion_tokens + d.reasoning_tokens),
      );
    }
    return Math.max(1, ...series.map((d) => d.download_bytes));
  }, [series, chartMode]);

  const donutSegments = useMemo(() => {
    const rows = [...(stats?.model_breakdown ?? [])]
      .filter((r) => r.total_tokens > 0)
      .sort((a, b) => b.total_tokens - a.total_tokens);
    if (rows.length === 0) return [];
    const top = rows.slice(0, 5);
    const rest = rows.slice(5);
    const merged =
      rest.length > 0
        ? [
            ...top.map((r) => ({ label: r.route, tokens: r.total_tokens, runs: r.run_count })),
            {
              label: t("settings.usageStats.otherModels", { count: rest.length }),
              tokens: rest.reduce((acc, r) => acc + r.total_tokens, 0),
              runs: rest.reduce((acc, r) => acc + r.run_count, 0),
            },
          ]
        : top.map((r) => ({ label: r.route, tokens: r.total_tokens, runs: r.run_count }));
    const total = merged.reduce((acc, s) => acc + s.tokens, 0);
    if (total <= 0) return [];
    const radius = 42;
    const circumference = 2 * Math.PI * radius;
    let offset = 0;
    return merged.map((seg, idx) => {
      const ratio = seg.tokens / total;
      const dashLen = ratio * circumference;
      const item = {
        ...seg,
        ratio,
        color: DONUT_COLORS[idx % DONUT_COLORS.length],
        dashArray: `${dashLen} ${circumference - dashLen}`,
        dashOffset: -offset,
      };
      offset += dashLen;
      return item;
    });
  }, [stats?.model_breakdown, t]);

  const toolRows = useMemo(() => stats?.tool_breakdown ?? [], [stats?.tool_breakdown]);
  const maxToolCalls = useMemo(
    () => Math.max(1, ...toolRows.map((t) => t.call_count)),
    [toolRows],
  );

  const providerRows = useMemo(
    () => stats?.provider_download_breakdown ?? [],
    [stats?.provider_download_breakdown],
  );
  const maxProviderBytes = useMemo(
    () => Math.max(1, ...providerRows.map((p) => p.bytes_transferred)),
    [providerRows],
  );

  const activePoint =
    hoveredDayIndex !== null && hoveredDayIndex >= 0 && hoveredDayIndex < series.length
      ? series[hoveredDayIndex]
      : null;

  const chartWidth = 640;
  const chartHeight = 168;
  const padLeft = 42;
  const padRight = 10;
  const padTop = 14;
  const padBottom = 26;
  const innerWidth = chartWidth - padLeft - padRight;
  const innerHeight = chartHeight - padTop - padBottom;
  const slotWidth = innerWidth / Math.max(1, series.length);
  const barWidth = Math.max(4, Math.min(14, slotWidth - 4));

  return (
    <div
      data-testid="usage-analytics-dashboard"
      className="mt-4 space-y-4"
    >
      {/* 1. 30-day interactive stacked SVG chart */}
      <div className="rounded-lg border border-black/10 dark:border-white/10 bg-black/[0.015] dark:bg-white/[0.02] p-3.5">
        <div className="flex flex-wrap items-center justify-between gap-2 mb-3">
          <div className="flex items-center gap-2">
            <Activity className="w-4 h-4 text-sky-500 shrink-0" />
            <div>
              <div className="text-[12.5px] font-semibold text-neutral-800 dark:text-neutral-100">
                {t("settings.usageStats.dailyChartTitle")}
              </div>
              <div className="text-[11px] text-neutral-500 dark:text-neutral-400">
                {chartMode === "tokens"
                  ? t("settings.usageStats.dailyChartTokensSubtitle")
                  : t("settings.usageStats.dailyChartDownloadsSubtitle")}
              </div>
            </div>
          </div>
          <div className="inline-flex items-center rounded-md border border-black/10 dark:border-white/10 bg-white dark:bg-neutral-900 p-0.5 text-[11px]">
            <button
              type="button"
              data-testid="usage-chart-mode-tokens"
              onClick={() => setChartMode("tokens")}
              className={`px-2.5 py-1 rounded transition-colors ${
                chartMode === "tokens"
                  ? "bg-sky-500/15 text-sky-700 dark:text-sky-300 font-semibold"
                  : "text-neutral-600 dark:text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-200"
              }`}
            >
              {t("settings.usageStats.modeTokens")}
            </button>
            <button
              type="button"
              data-testid="usage-chart-mode-downloads"
              onClick={() => setChartMode("downloads")}
              className={`px-2.5 py-1 rounded transition-colors ${
                chartMode === "downloads"
                  ? "bg-blue-500/15 text-blue-700 dark:text-blue-300 font-semibold"
                  : "text-neutral-600 dark:text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-200"
              }`}
            >
              {t("settings.usageStats.modeDownloads")}
            </button>
          </div>
        </div>

        <div className="relative" data-testid="usage-analytics-daily-chart">
          <svg
            viewBox={`0 0 ${chartWidth} ${chartHeight}`}
            className="w-full h-44 overflow-visible select-none"
            role="img"
            aria-label={t("settings.usageStats.dailyChartAria")}
          >
            {/* Horizontal grid lines */}
            {[0, 0.5, 1].map((ratio) => {
              const y = padTop + innerHeight * (1 - ratio);
              const val = maxChartValue * ratio;
              const label =
                chartMode === "tokens"
                  ? formatCompactNumber(val)
                  : val === 0
                    ? "0"
                    : formatBytes(val);
              return (
                <g key={ratio}>
                  <line
                    x1={padLeft}
                    y1={y}
                    x2={chartWidth - padRight}
                    y2={y}
                    stroke="currentColor"
                    strokeOpacity={0.1}
                    strokeDasharray={ratio === 0 ? undefined : "3 3"}
                  />
                  <text
                    x={padLeft - 6}
                    y={y + 3}
                    textAnchor="end"
                    fontSize="9"
                    fill="currentColor"
                    opacity={0.55}
                  >
                    {hasChartData ? label : ratio === 0 ? "0" : ""}
                  </text>
                </g>
              );
            })}

            {/* Bars */}
            {series.map((pt, idx) => {
              const xCenter = padLeft + idx * slotWidth + slotWidth / 2;
              const x = xCenter - barWidth / 2;
              const baseline = padTop + innerHeight;
              const isHovered = hoveredDayIndex === idx;

              if (chartMode === "tokens") {
                const totalDay = pt.prompt_tokens + pt.completion_tokens + pt.reasoning_tokens;
                const promptH = hasChartData ? (pt.prompt_tokens / maxChartValue) * innerHeight : 0;
                const compH = hasChartData ? (pt.completion_tokens / maxChartValue) * innerHeight : 0;
                const reasonH = hasChartData ? (pt.reasoning_tokens / maxChartValue) * innerHeight : 0;
                const totalH = promptH + compH + reasonH;
                const promptY = baseline - promptH;
                const compY = promptY - compH;
                const reasonY = compY - reasonH;
                const showDateLabel = idx === 0 || idx === 14 || idx === series.length - 1;

                return (
                  <g
                    key={pt.date}
                    onMouseEnter={() => setHoveredDayIndex(idx)}
                    onMouseLeave={() => setHoveredDayIndex((cur) => (cur === idx ? null : cur))}
                    onFocus={() => setHoveredDayIndex(idx)}
                    tabIndex={0}
                    className="cursor-pointer outline-none"
                  >
                    <rect
                      x={padLeft + idx * slotWidth}
                      y={padTop}
                      width={slotWidth}
                      height={innerHeight}
                      fill={isHovered ? "currentColor" : "transparent"}
                      fillOpacity={isHovered ? 0.06 : 0}
                      rx={2}
                    />
                    {totalDay === 0 && (
                      <rect
                        x={x}
                        y={baseline - 1.5}
                        width={barWidth}
                        height={1.5}
                        fill="currentColor"
                        fillOpacity={0.14}
                        rx={0.75}
                      />
                    )}
                    {promptH > 0 && (
                      <rect
                        x={x}
                        y={promptY}
                        width={barWidth}
                        height={Math.max(1, promptH)}
                        fill="#0ea5e9"
                        fillOpacity={isHovered ? 1 : 0.85}
                        rx={compH === 0 && reasonH === 0 ? 2 : 0}
                      />
                    )}
                    {compH > 0 && (
                      <rect
                        x={x}
                        y={compY}
                        width={barWidth}
                        height={Math.max(1, compH)}
                        fill="#10b981"
                        fillOpacity={isHovered ? 1 : 0.85}
                        rx={reasonH === 0 ? 2 : 0}
                      />
                    )}
                    {reasonH > 0 && (
                      <rect
                        x={x}
                        y={reasonY}
                        width={barWidth}
                        height={Math.max(1, reasonH)}
                        fill="#8b5cf6"
                        fillOpacity={isHovered ? 1 : 0.85}
                        rx={2}
                      />
                    )}
                    {totalH > 0 && isHovered && (
                      <rect
                        x={x - 1}
                        y={baseline - totalH - 1}
                        width={barWidth + 2}
                        height={totalH + 2}
                        fill="none"
                        stroke="currentColor"
                        strokeOpacity={0.35}
                        strokeWidth={1}
                        rx={2}
                      />
                    )}
                    {showDateLabel && (
                      <text
                        x={xCenter}
                        y={chartHeight - 6}
                        textAnchor="middle"
                        fontSize="9"
                        fill="currentColor"
                        opacity={0.55}
                      >
                        {pt.date.slice(5)}
                      </text>
                    )}
                  </g>
                );
              }

              const dlH = hasChartData ? (pt.download_bytes / maxChartValue) * innerHeight : 0;
              const showDateLabel = idx === 0 || idx === 14 || idx === series.length - 1;
              return (
                <g
                  key={pt.date}
                  onMouseEnter={() => setHoveredDayIndex(idx)}
                  onMouseLeave={() => setHoveredDayIndex((cur) => (cur === idx ? null : cur))}
                  onFocus={() => setHoveredDayIndex(idx)}
                  tabIndex={0}
                  className="cursor-pointer outline-none"
                >
                  <rect
                    x={padLeft + idx * slotWidth}
                    y={padTop}
                    width={slotWidth}
                    height={innerHeight}
                    fill={isHovered ? "currentColor" : "transparent"}
                    fillOpacity={isHovered ? 0.06 : 0}
                    rx={2}
                  />
                  {pt.download_bytes === 0 ? (
                    <rect
                      x={x}
                      y={baseline - 1.5}
                      width={barWidth}
                      height={1.5}
                      fill="currentColor"
                      fillOpacity={0.14}
                      rx={0.75}
                    />
                  ) : (
                    <rect
                      x={x}
                      y={baseline - Math.max(2, dlH)}
                      width={barWidth}
                      height={Math.max(2, dlH)}
                      fill="#3b82f6"
                      fillOpacity={isHovered ? 1 : 0.85}
                      rx={2}
                    />
                  )}
                  {showDateLabel && (
                    <text
                      x={xCenter}
                      y={chartHeight - 6}
                      textAnchor="middle"
                      fontSize="9"
                      fill="currentColor"
                      opacity={0.55}
                    >
                      {pt.date.slice(5)}
                    </text>
                  )}
                </g>
              );
            })}
          </svg>

          {activePoint ? (
            <div
              data-testid="usage-daily-tooltip"
              className="mt-2 flex flex-wrap items-center justify-between gap-2 rounded-md border border-sky-500/25 bg-sky-500/5 px-3 py-1.5 text-[11px] text-neutral-700 dark:text-neutral-200"
            >
              <span className="font-mono font-semibold">{activePoint.date}</span>
              {chartMode === "tokens" ? (
                <span>
                  {t("settings.usageStats.tooltipTokens", {
                    runs: activePoint.runs,
                    prompt: activePoint.prompt_tokens.toLocaleString(),
                    completion: activePoint.completion_tokens.toLocaleString(),
                    reasoning: activePoint.reasoning_tokens.toLocaleString(),
                  })}
                </span>
              ) : (
                <span>
                  {t("settings.usageStats.tooltipDownloads", {
                    bytes: formatBytes(activePoint.download_bytes),
                    runs: activePoint.runs,
                  })}
                </span>
              )}
            </div>
          ) : !hasChartData && !loading ? (
            <div className="mt-1 text-[11px] text-neutral-500 dark:text-neutral-400">
              {t("settings.usageStats.emptyDailyChart")}
            </div>
          ) : (
            <div className="mt-2 flex flex-wrap items-center gap-4 text-[11px] text-neutral-500 dark:text-neutral-400">
              {chartMode === "tokens" ? (
                <>
                  <span className="inline-flex items-center gap-1.5">
                    <span className="w-2.5 h-2.5 rounded-sm bg-sky-500 inline-block" /> Prompt
                  </span>
                  <span className="inline-flex items-center gap-1.5">
                    <span className="w-2.5 h-2.5 rounded-sm bg-emerald-500 inline-block" /> Completion
                  </span>
                  <span className="inline-flex items-center gap-1.5">
                    <span className="w-2.5 h-2.5 rounded-sm bg-violet-500 inline-block" /> Reasoning
                  </span>
                </>
              ) : (
                <span className="inline-flex items-center gap-1.5">
                  <span className="w-2.5 h-2.5 rounded-sm bg-blue-500 inline-block" /> {t("settings.usageStats.legendDailyBytes")}
                </span>
              )}
              <span className="ml-auto opacity-75">{t("settings.usageStats.hoverBarHint")}</span>
            </div>
          )}
        </div>
      </div>

      {/* 2. Channel / Model Distribution Donut + GEE Quota + Tool/Provider Breakdown */}
      <div className="grid grid-cols-1 md:grid-cols-2 gap-3.5">
        {/* Model Distribution Donut */}
        <div
          data-testid="usage-analytics-model-donut"
          className="rounded-lg border border-black/10 dark:border-white/10 bg-black/[0.015] dark:bg-white/[0.02] p-3.5"
        >
          <div className="flex items-center gap-2 mb-3">
            <Cpu className="w-4 h-4 text-emerald-500 shrink-0" />
            <div className="text-[12.5px] font-semibold text-neutral-800 dark:text-neutral-100">
              {t("settings.usageStats.donutTitle")}
            </div>
          </div>
          {donutSegments.length === 0 ? (
            <EmptyState
              kind="first-use"
              size="card"
              visual={<EmptyStateIcon name="chart" size={22} />}
              title={t("settings.usageStats.donutEmpty")}
            />
          ) : (
            <div className="flex items-center gap-4">
              <svg viewBox="0 0 112 112" className="w-28 h-28 shrink-0 -rotate-90">
                <circle
                  cx="56"
                  cy="56"
                  r="42"
                  fill="none"
                  stroke="currentColor"
                  strokeOpacity={0.08}
                  strokeWidth="14"
                />
                {donutSegments.map((seg, idx) => (
                  <circle
                    key={seg.label}
                    cx="56"
                    cy="56"
                    r="42"
                    fill="none"
                    stroke={seg.color}
                    strokeWidth={hoveredDonutIndex === idx ? 17 : 14}
                    strokeDasharray={seg.dashArray}
                    strokeDashoffset={seg.dashOffset}
                    className="transition-all cursor-pointer"
                    onMouseEnter={() => setHoveredDonutIndex(idx)}
                    onMouseLeave={() => setHoveredDonutIndex(null)}
                  />
                ))}
                <g className="rotate-90 origin-center">
                  <text
                    x="56"
                    y="53"
                    textAnchor="middle"
                    fontSize="11"
                    fontWeight="600"
                    fill="currentColor"
                  >
                    {formatCompactNumber(stats?.total_tokens ?? 0)}
                  </text>
                  <text
                    x="56"
                    y="66"
                    textAnchor="middle"
                    fontSize="8.5"
                    fill="currentColor"
                    opacity={0.6}
                  >
                    Tokens
                  </text>
                </g>
              </svg>
              <div className="min-w-0 flex-1 space-y-1.5">
                {donutSegments.map((seg, idx) => (
                  <div
                    key={seg.label}
                    onMouseEnter={() => setHoveredDonutIndex(idx)}
                    onMouseLeave={() => setHoveredDonutIndex(null)}
                    className={`flex items-center justify-between gap-2 text-[11px] rounded px-1.5 py-0.5 transition-colors ${
                      hoveredDonutIndex === idx ? "bg-black/5 dark:bg-white/10" : ""
                    }`}
                  >
                    <span className="flex items-center gap-1.5 min-w-0">
                      <span
                        className="w-2 h-2 rounded-full shrink-0"
                        style={{ backgroundColor: seg.color }}
                      />
                      <span className="truncate font-mono text-neutral-700 dark:text-neutral-200">
                        {seg.label}
                      </span>
                    </span>
                    <span className="shrink-0 text-neutral-500 dark:text-neutral-400">
                      {(seg.ratio * 100).toFixed(1)}%
                    </span>
                  </div>
                ))}
              </div>
            </div>
          )}
        </div>

        {/* Provider Download Breakdown + GEE Status */}
        <div
          data-testid="usage-analytics-providers"
          className="rounded-lg border border-black/10 dark:border-white/10 bg-black/[0.015] dark:bg-white/[0.02] p-3.5 flex flex-col justify-between gap-3"
        >
          <div>
            <div className="flex items-center gap-2 mb-3">
              <Download className="w-4 h-4 text-blue-500 shrink-0" />
              <div className="text-[12.5px] font-semibold text-neutral-800 dark:text-neutral-100">
                {t("settings.usageStats.providersTitle")}
              </div>
            </div>
            {providerRows.length === 0 ? (
              <EmptyState
                kind="first-use"
                size="card"
                visual={<EmptyStateIcon name="chart" size={22} />}
                title={t("settings.usageStats.providersEmpty")}
              />
            ) : (
              <div className="space-y-2">
                {providerRows.slice(0, 5).map((prov) => {
                  const pct = Math.max(
                    4,
                    Math.round((prov.bytes_transferred / maxProviderBytes) * 100),
                  );
                  return (
                    <div key={prov.source_id} className="space-y-1">
                      <div className="flex items-center justify-between text-[11px]">
                        <span className="font-medium text-neutral-700 dark:text-neutral-200 truncate">
                          {prov.display_name}
                        </span>
                        <span className="text-neutral-500 dark:text-neutral-400 shrink-0">
                          {t("settings.usageStats.providerTaskSummary", {
                            count: prov.task_count,
                            bytes: formatBytes(prov.bytes_transferred),
                          })}
                        </span>
                      </div>
                      <div className="h-1.5 rounded-full bg-black/10 dark:bg-white/10 overflow-hidden">
                        <div
                          className="h-full rounded-full bg-blue-500 transition-all"
                          style={{ width: `${prov.bytes_transferred > 0 ? pct : 0}%` }}
                        />
                      </div>
                    </div>
                  );
                })}
              </div>
            )}
          </div>

          {stats?.gee_quota_status && (
            <div
              data-testid="usage-analytics-gee"
              className="pt-2.5 border-t border-black/10 dark:border-white/10 flex flex-wrap items-center justify-between gap-2 text-[11px]"
            >
              <span className="inline-flex items-center gap-1.5 font-medium text-neutral-700 dark:text-neutral-200">
                <Globe className="w-3.5 h-3.5 text-emerald-500" />
                {t("settings.usageStats.geeStatusLabel")}
              </span>
              <span
                className={`px-2 py-0.5 rounded-full font-medium ${
                  stats.gee_quota_status.verified
                    ? "bg-emerald-500/15 text-emerald-700 dark:text-emerald-300"
                    : stats.gee_quota_status.configured
                      ? "bg-amber-500/15 text-amber-700 dark:text-amber-300"
                      : "bg-neutral-500/15 text-neutral-600 dark:text-neutral-400"
                }`}
              >
                {stats.gee_quota_status.verified
                  ? t("settings.usageStats.geeVerified", {
                      project: stats.gee_quota_status.project_id ? ` (${stats.gee_quota_status.project_id})` : "",
                    })
                  : stats.gee_quota_status.configured
                    ? t("settings.usageStats.geeConfigured", { mode: stats.gee_quota_status.auth_mode })
                    : t("settings.usageStats.geeUnconfigured")}
              </span>
            </div>
          )}
        </div>
      </div>

      {/* 3. Tool Calls Breakdown */}
      <div
        data-testid="usage-analytics-tools"
        className="rounded-lg border border-black/10 dark:border-white/10 bg-black/[0.015] dark:bg-white/[0.02] p-3.5"
      >
        <div className="flex items-center justify-between gap-2 mb-3">
          <div className="flex items-center gap-2">
            <Wrench className="w-4 h-4 text-violet-500 shrink-0" />
            <div className="text-[12.5px] font-semibold text-neutral-800 dark:text-neutral-100">
              {t("settings.usageStats.toolsTitle")}
            </div>
          </div>
          <span className="inline-flex items-center gap-1 text-[11px] text-neutral-500 dark:text-neutral-400">
            <Database className="w-3 h-3" />
            {t("settings.usageStats.toolsSourceNote")}
          </span>
        </div>
        {toolRows.length === 0 ? (
          <EmptyState
            kind="first-use"
            size="card"
            visual={<EmptyStateIcon name="chart" size={22} />}
            title={t("settings.usageStats.toolsEmpty")}
          />
        ) : (
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-2.5">
            {toolRows.slice(0, 8).map((tool) => {
              const barPct = Math.max(4, Math.round((tool.call_count / maxToolCalls) * 100));
              const successPct =
                tool.call_count > 0
                  ? Math.round((tool.success_count / tool.call_count) * 100)
                  : 0;
              return (
                <div key={tool.tool_id} className="space-y-1">
                  <div className="flex items-center justify-between gap-2 text-[11px]">
                    <span className="font-mono truncate text-neutral-700 dark:text-neutral-200">
                      {tool.tool_id}
                    </span>
                    <span className="shrink-0 text-neutral-500 dark:text-neutral-400">
                      {t("settings.usageStats.toolCallsSummary", {
                        calls: tool.call_count,
                        success: tool.success_count,
                        pct: successPct,
                      })}
                    </span>
                  </div>
                  <div className="h-1.5 rounded-full bg-black/10 dark:bg-white/10 overflow-hidden">
                    <div
                      className="h-full rounded-full bg-violet-500 transition-all"
                      style={{ width: `${barPct}%` }}
                    />
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
};
