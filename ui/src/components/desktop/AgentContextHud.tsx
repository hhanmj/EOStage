/**
 * AgentContextHud.tsx
 * 对话框底部工具栏 (`agent-composer-toolbar`) 内的会话性能与上下文容量监控组件：
 * - 极简模式 (`hudMode === "minimal"`): 工具栏内仅渲染纯粹的圆圈 SVG 进度环触发器（无多余行内文字）；
 *   悬停或点击展开双层同心圆上下文容量分解托盘（含真实模型徽章、内联「极简 | 详细」切换、已用百分比与 ~XXK / 1.05M、
 *   对话消息/系统工具/技能/系统提示词四色分段条与图例、平均缓存命中率）；
 * - 详细模式 (`hudMode === "pro"`): 在工具栏内收敛渲染紧凑遥测胶囊（输出速率 TPS、Token 用量与缓存命中率、上下文进度环+比例），
 *   绝不占用输入框外部布局空间；悬停或点击弹出完整详细分解浮层。
 */

import React, { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Gauge, Database } from "lucide-react";
import { cn } from "@/lib/utils";
import { useI18n, type MessageKey, type MessageValues } from "@/i18n";
import {
  computeContextTelemetry,
  formatCompactTokens,
  getPerformanceAndUsagePrefs,
  setPerformanceOnlyPrefs,
  subscribePerformanceAndUsagePrefs,
  type ContextHudMode,
  type ContextTelemetryEventLike,
  type ContextTelemetryRunSummary,
} from "@/lib/performanceAndUsagePrefs";

export type PulseTabId = "activity" | "usage" | "context";

export interface AgentContextHudProps {
  runs?: readonly ContextTelemetryRunSummary[];
  events?: readonly ContextTelemetryEventLike[];
  draftText?: string;
  maxContextTokens?: number;
  modelLabel?: string;
  compact?: boolean;
  compactionPulse?: boolean;
  onOpenSettings?: () => void;
}

const RING_RADIUS = 6;
const RING_CIRCUMFERENCE = 2 * Math.PI * RING_RADIUS;

function cubicBezier032_072_0_1(t: number): number {
  const x = Math.max(0, Math.min(1, t));
  return 1 - Math.pow(1 - x, 3.2);
}

function canAnimateInLiveBrowser(el: HTMLElement | null): boolean {
  if (!el || typeof window === "undefined" || typeof window.requestAnimationFrame !== "function") {
    return false;
  }
  try {
    if (
      typeof window.matchMedia === "function" &&
      window.matchMedia("(prefers-reduced-motion: reduce)").matches
    ) {
      return false;
    }
  } catch {
    // ignore
  }
  return el.offsetWidth > 0 || el.offsetHeight > 0;
}

/**
 * 上下文仪表里程表数值插值：
 * - 首帧挂载或 jsdom / reduced-motion 下立即同步返回目标值，保障同步测试零延迟；
 * - 真实浏览器中当数值发生增量变化时，以 500ms cubic-bezier(0.32, 0.72, 0, 1) 平滑滚动。
 */
function useOdometerNumber(
  target: number,
  hostRef: React.RefObject<HTMLElement | null>,
  durationMs = 500
): number {
  const [displayVal, setDisplayVal] = useState<number>(target);
  const prevTargetRef = useRef<number>(target);
  const rafRef = useRef<number | null>(null);

  useEffect(() => {
    const fromVal = prevTargetRef.current;
    prevTargetRef.current = target;
    if (fromVal === target || !canAnimateInLiveBrowser(hostRef.current)) {
      setDisplayVal(target);
      return;
    }
    if (rafRef.current !== null) {
      window.cancelAnimationFrame(rafRef.current);
    }
    const start = performance.now();
    const tick = (now: number) => {
      const elapsed = now - start;
      const p = Math.min(1, elapsed / durationMs);
      const eased = cubicBezier032_072_0_1(p);
      setDisplayVal(fromVal + (target - fromVal) * eased);
      if (p < 1) {
        rafRef.current = window.requestAnimationFrame(tick);
      } else {
        rafRef.current = null;
        setDisplayVal(target);
      }
    };
    rafRef.current = window.requestAnimationFrame(tick);
    return () => {
      if (rafRef.current !== null) {
        window.cancelAnimationFrame(rafRef.current);
        rafRef.current = null;
      }
    };
  }, [durationMs, hostRef, target]);

  return canAnimateInLiveBrowser(hostRef.current) ? displayVal : target;
}

function formatDuration(sec: number, t: (key: MessageKey, values?: MessageValues) => string): string {
  if (!Number.isFinite(sec) || sec <= 0) return "—";
  if (sec < 60) {
    return t("agent.pulse.durationSeconds", {
      value: sec < 10 ? sec.toFixed(1) : String(Math.round(sec)),
    });
  }
  const m = Math.floor(sec / 60);
  const s = Math.round(sec % 60);
  return t("agent.pulse.durationMinutesSeconds", { minutes: m, seconds: s });
}

function pctOf(part: number, whole: number): number {
  if (!(whole > 0) || !(part > 0)) return 0;
  return Math.min(100, (part / whole) * 100);
}

function LegendRow({
  dotClass,
  label,
  value,
  note,
}: {
  dotClass?: string;
  label: string;
  value: React.ReactNode;
  note?: React.ReactNode;
}) {
  return (
    <div className="flex items-center justify-between gap-3">
      <span className="flex min-w-0 items-center gap-2 text-sp-text-secondary">
        {dotClass ? (
          <span className={cn("h-2 w-2 shrink-0 rounded-full", dotClass)} />
        ) : null}
        <span className="truncate">{label}</span>
      </span>
      <span className="shrink-0 tabular-nums">
        <strong className="font-semibold text-sp-text">{value}</strong>
        {note ? <span className="ml-1 text-sp-text-muted">{note}</span> : null}
      </span>
    </div>
  );
}

export function AgentContextHud({
  runs = [],
  events = [],
  draftText = "",
  maxContextTokens = 1_000_000,
  modelLabel,
  compact = false,
  compactionPulse = false,
}: AgentContextHudProps) {
  const { t } = useI18n();
  const [hudMode, setHudMode] = useState<ContextHudMode>(
    () => getPerformanceAndUsagePrefs().hudMode
  );
  const [hoveredTab, setHoveredTab] = useState<PulseTabId | null>(null);
  const [pinnedTab, setPinnedTab] = useState<PulseTabId | null>(null);
  const [autoCompactFlash, setAutoCompactFlash] = useState(false);
  const rootRef = useRef<HTMLDivElement | null>(null);
  const popoverRef = useRef<HTMLDivElement | null>(null);
  const prevContextTokensRef = useRef<number>(0);
  const [popoverOffset, setPopoverOffset] = useState<{ shiftX: number; maxWidth: number | null }>({
    shiftX: 0,
    maxWidth: null,
  });

  useEffect(() => {
    return subscribePerformanceAndUsagePrefs((prefs) => {
      setHudMode(prefs.hudMode);
      if (prefs.hudMode === "minimal") {
        setPinnedTab((prev) => (prev && prev !== "context" ? "context" : prev));
        setHoveredTab((prev) => (prev && prev !== "context" ? "context" : prev));
      }
    });
  }, []);

  useEffect(() => {
    if (!pinnedTab && !hoveredTab) return;
    const handleOutside = (e: MouseEvent) => {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) {
        setPinnedTab(null);
        setHoveredTab(null);
      }
    };
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setPinnedTab(null);
        setHoveredTab(null);
      }
    };
    document.addEventListener("mousedown", handleOutside);
    document.addEventListener("keydown", handleKey);
    return () => {
      document.removeEventListener("mousedown", handleOutside);
      document.removeEventListener("keydown", handleKey);
    };
  }, [pinnedTab, hoveredTab]);

  const snap = useMemo(
    () => computeContextTelemetry({ runs, events, draftText, maxContextTokens }),
    [runs, events, draftText, maxContextTokens]
  );

  // 自动压缩瞬态翡翠绿呼吸（当上下文 Token 发生 >=20% 压缩回落时触发 1.2s 脉冲）
  useEffect(() => {
    const prev = prevContextTokensRef.current;
    const curr = snap.currentContextTokens;
    prevContextTokensRef.current = curr;
    if (prev >= 2000 && curr > 0 && curr <= prev * 0.8) {
      setAutoCompactFlash(true);
      const timer = window.setTimeout(() => setAutoCompactFlash(false), 1200);
      return () => window.clearTimeout(timer);
    }
  }, [snap.currentContextTokens]);

  const isCompacting = Boolean(compactionPulse || autoCompactFlash);
  const animatedContextTokens = useOdometerNumber(snap.currentContextTokens, rootRef);
  const animatedContextPct = useOdometerNumber(snap.contextPct, rootRef);

  const isPro = hudMode === "pro";
  void compact;

  const rawActiveTab: PulseTabId | null = pinnedTab ?? hoveredTab;
  const activeTab: PulseTabId | null =
    !isPro && rawActiveTab && rawActiveTab !== "context" ? "context" : rawActiveTab;

  useLayoutEffect(() => {
    if (!activeTab) {
      setPopoverOffset((prev) =>
        prev.shiftX === 0 && prev.maxWidth === null ? prev : { shiftX: 0, maxWidth: null }
      );
      return;
    }
    const clampPopover = () => {
      const rootEl = rootRef.current;
      const popEl = popoverRef.current;
      if (!rootEl || !popEl || typeof window === "undefined") return;
      const boundsEl =
        (rootEl.closest(".cds-composer-shell") as HTMLElement | null) ??
        (rootEl.closest(".agent-split") as HTMLElement | null);
      const boundsRect = boundsEl?.getBoundingClientRect();
      const popRect = popEl.getBoundingClientRect();
      if (popRect.width <= 0) return;
      const minLeft = boundsRect ? Math.max(8, boundsRect.left + 8) : 8;
      const maxRight = boundsRect
        ? Math.min(window.innerWidth - 8, boundsRect.right - 8)
        : window.innerWidth - 8;
      const availWidth = Math.max(220, Math.floor(maxRight - minLeft));
      const nextMaxWidth = availWidth < 340 ? availWidth : null;
      const naturalLeft = popRect.left - popoverOffset.shiftX;
      const naturalRight = popRect.right - popoverOffset.shiftX;
      let shiftX = 0;
      if (naturalLeft < minLeft) {
        shiftX = minLeft - naturalLeft;
      }
      if (naturalRight + shiftX > maxRight) {
        shiftX -= naturalRight + shiftX - maxRight;
      }
      if (naturalLeft + shiftX < minLeft) {
        shiftX = minLeft - naturalLeft;
      }
      const roundedShift = Math.round(shiftX);
      setPopoverOffset((prev) =>
        prev.shiftX === roundedShift && prev.maxWidth === nextMaxWidth
          ? prev
          : { shiftX: roundedShift, maxWidth: nextMaxWidth }
      );
    };
    clampPopover();
    window.addEventListener("resize", clampPopover);
    return () => window.removeEventListener("resize", clampPopover);
  }, [activeTab, hudMode, popoverOffset.shiftX]);

  const handleContextMenu = (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setPerformanceOnlyPrefs({ hudMode: isPro ? "minimal" : "pro" });
  };

  const handlePillClick = (tab: PulseTabId) => {
    setPinnedTab((prev) => (prev === tab ? null : tab));
  };

  const handleModeSwitch = (mode: ContextHudMode, e: React.MouseEvent) => {
    e.stopPropagation();
    setPerformanceOnlyPrefs({ hudMode: mode });
  };

  const usageLabel = snap.hasReportedUsage
    ? `${formatCompactTokens(snap.totalTokens)} tok`
    : "— tok";
  const cacheRateDisplay = snap.hasCacheData
    ? `${snap.cacheHitPct.toFixed(1)}%`
    : t("agent.pulse.unreported");
  const modelSec = Math.max(0, snap.totalTimeSec - snap.toolSec);
  const ringPct =
    snap.currentContextTokens <= 0 ? 0 : Math.min(100, Math.max(4, snap.contextPct));
  const ringOffset = RING_CIRCUMFERENCE - (ringPct / 100) * RING_CIRCUMFERENCE;
  const contextTone = snap.isNearAutoCompact
    ? "warn"
    : snap.contextPct >= 50
      ? "info"
      : "safe";
  const ringColor =
    contextTone === "warn"
      ? "text-sp-warning-text"
      : contextTone === "info"
        ? "text-sp-accent"
        : "text-sp-success-text";

  const pillClass = (tab: PulseTabId, circleOnly = false) =>
    cn(
      "sp-morph-island-shell inline-flex h-7 items-center rounded-sp-sm border text-xs tabular-nums whitespace-nowrap cursor-pointer shrink-0",
      circleOnly ? "w-7 justify-center px-0 rounded-full" : "gap-1 px-1.5",
      activeTab === tab
        ? "border-sp-line bg-sp-card text-sp-text sp-solid-glass-card"
        : "border-transparent text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text"
    );

  const pillTitle = t("agent.pulse.pillTitle");
  const resolvedModelBadge =
    modelLabel && modelLabel.trim().length > 0 ? modelLabel.trim() : t("agent.pulse.currentModelFallback");

  const usedPctDisplay =
    snap.currentContextTokens > 0 && snap.contextPct === 0
      ? "<0.1%"
      : `${animatedContextPct.toFixed(1)}%`;

  const usedTokensDisplay =
    snap.currentContextTokens > 0
      ? `~${formatCompactTokens(Math.round(animatedContextTokens))}`
      : "~0";

  // 四色分类进度条各段占最大上下文窗口的宽度百分比（五色科研语义令牌）
  const catList = [
    {
      key: "messages" as const,
      label: t("agent.pulse.catMessages"),
      dotClass: "bg-sp-sem-cobalt-dot",
      barClass: "bg-sp-sem-cobalt-dot",
      tokens: snap.categories.messages.tokens,
      pct: snap.categories.messages.pct,
    },
    {
      key: "tools" as const,
      label: t("agent.pulse.catTools"),
      dotClass: "bg-sp-sem-amber-dot",
      barClass: "bg-sp-sem-amber-dot",
      tokens: snap.categories.tools.tokens,
      pct: snap.categories.tools.pct,
    },
    {
      key: "skills" as const,
      label: t("agent.pulse.catSkills"),
      dotClass: "bg-sp-sem-violet-dot",
      barClass: "bg-sp-sem-violet-dot",
      tokens: snap.categories.skills.tokens,
      pct: snap.categories.skills.pct,
    },
    {
      key: "system" as const,
      label: t("agent.pulse.catSystem"),
      dotClass: "bg-sp-sem-teal-dot",
      barClass: "bg-sp-sem-teal-dot",
      tokens: snap.categories.system.tokens,
      pct: snap.categories.system.pct,
    },
  ];

  const renderModeToggle = () => (
    <div
      data-testid="pulse-mode-toggle"
      className="sp-field-well inline-flex shrink-0 items-center rounded-full border border-sp-line bg-sp-hover/70 p-0.5 text-xs"
    >
      <button
        type="button"
        data-testid="pulse-mode-btn-minimal"
        onClick={(e) => handleModeSwitch("minimal", e)}
        className={cn(
          "rounded-full px-2 py-0.5 font-medium transition-all duration-150 cursor-pointer",
          !isPro
            ? "bg-sp-card text-sp-text sp-solid-glass-card"
            : "text-sp-text-muted hover:text-sp-text"
        )}
      >
        {t("agent.pulse.modeMinimal")}
      </button>
      <button
        type="button"
        data-testid="pulse-mode-btn-pro"
        onClick={(e) => handleModeSwitch("pro", e)}
        className={cn(
          "rounded-full px-2 py-0.5 font-medium transition-all duration-150 cursor-pointer",
          isPro
            ? "bg-sp-card text-sp-text sp-solid-glass-card"
            : "text-sp-text-muted hover:text-sp-text"
        )}
      >
        {t("agent.pulse.modePro")}
      </button>
    </div>
  );

  return (
    <div
      ref={rootRef}
      data-testid="agent-context-hud"
      data-hud-mode={hudMode}
      onMouseLeave={() => setHoveredTab(null)}
      className="relative inline-flex flex-nowrap items-center gap-1 select-none shrink-0"
    >
      {/* 详细模式 (`pro`)：底栏仅保留核心无重复指标（轮步 / 总用量 / 纯圆环），细节下沉至对应浮层 */}
      {isPro && (
        <>
          <button
            type="button"
            data-testid="pulse-pill-activity"
            aria-pressed={activeTab === "activity"}
            onMouseEnter={() => setHoveredTab("activity")}
            onClick={() => handlePillClick("activity")}
            onContextMenu={handleContextMenu}
            title={pillTitle}
            className={pillClass("activity")}
          >
            <Gauge className="h-3.5 w-3.5 shrink-0 text-sp-text-muted" />
            <span>
              <strong className="font-semibold text-sp-text">{t("agent.pulse.turnsUnit", { turns: snap.turns })}</strong> {t("agent.pulse.turnsStepsCompact", { steps: snap.steps })}
            </span>
          </button>

          <button
            type="button"
            data-testid="pulse-pill-usage"
            aria-pressed={activeTab === "usage"}
            onMouseEnter={() => setHoveredTab("usage")}
            onClick={() => handlePillClick("usage")}
            onContextMenu={handleContextMenu}
            title={pillTitle}
            className={pillClass("usage")}
          >
            <Database className="h-3.5 w-3.5 shrink-0 text-sp-text-muted" />
            <strong className="font-semibold text-sp-text">{usageLabel}</strong>
          </button>
        </>
      )}

      {/* 上下文状态触发器：极简与详细模式下均仅渲染纯粹圆圈 SVG 进度环（不在圆环旁堆叠重复数字） */}
      <button
        type="button"
        data-testid="pulse-pill-context"
        aria-label={t("agent.pulse.contextPillAria", { pct: usedPctDisplay })}
        aria-pressed={activeTab === "context"}
        onMouseEnter={() => setHoveredTab("context")}
        onClick={() => handlePillClick("context")}
        onContextMenu={handleContextMenu}
        title={pillTitle}
        className={pillClass("context", true)}
      >
        <svg
          data-testid="pulse-context-ring-svg"
          className="h-4 w-4 shrink-0 -rotate-90"
          viewBox="0 0 16 16"
          aria-hidden="true"
        >
          <circle
            cx="8"
            cy="8"
            r={RING_RADIUS}
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            className="text-sp-line"
          />
          <circle
            cx="8"
            cy="8"
            r={RING_RADIUS}
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeDasharray={RING_CIRCUMFERENCE}
            strokeDashoffset={ringOffset}
            strokeLinecap="round"
            style={{
              transition:
                "stroke-dashoffset 420ms cubic-bezier(0.22, 1, 0.36, 1), color 240ms ease",
            }}
            className={isCompacting ? "text-sp-sem-emerald-dot" : ringColor}
          />
        </svg>
      </button>

      {/* 悬停 / 点击浮层面板：双层同心圆托盘 + Solid Glass 镜面边缘 + Morph Island 平滑形变 */}
      {activeTab && (
        <div
          ref={popoverRef}
          role="dialog"
          aria-label={t("agent.pulse.popoverAria")}
          data-testid="pulse-hud-popover"
          data-morph-island="true"
          data-compaction-pulse={isCompacting ? "true" : "false"}
          style={
            popoverOffset.shiftX !== 0 || popoverOffset.maxWidth !== null
              ? {
                  transform:
                    popoverOffset.shiftX !== 0
                      ? `translateX(${popoverOffset.shiftX}px)`
                      : undefined,
                  maxWidth:
                    popoverOffset.maxWidth !== null
                      ? `${popoverOffset.maxWidth}px`
                      : undefined,
                }
              : undefined
          }
          className="sp-solid-glass-popover sp-morph-island-shell absolute bottom-[calc(100%+8px)] right-0 z-50 w-[340px] rounded-[18px] border border-sp-line bg-sp-canvas p-1.5 text-xs text-sp-text dark:bg-sp-surface"
        >
          {activeTab === "activity" && (
            <div data-testid="pulse-panel-activity" className="sp-morph-content-enter space-y-1.5">
              <div className="flex items-center justify-between gap-2 px-2.5 pt-1.5 pb-1">
                <div className="flex items-center gap-2 min-w-0">
                  <span className="text-[13px] font-semibold tracking-tight text-sp-text">{t("agent.pulse.sessionStatsHeading")}</span>
                  <span className="sp-field-well rounded-full bg-sp-hover px-2 py-0.5 font-mono tabular-nums text-sp-text-secondary">
                    {t("agent.pulse.turnsStepsFull", { turns: snap.turns, steps: snap.steps })}
                  </span>
                </div>
                {renderModeToggle()}
              </div>
              <div className="sp-solid-glass-card rounded-[13px] bg-sp-surface p-3 space-y-2.5 dark:bg-sp-card">
                <div className="sp-meter-track flex h-1.5 w-full gap-0.5 overflow-hidden rounded-full bg-sp-hover">
                  <div
                    style={{
                      width: `${pctOf(modelSec, snap.totalTimeSec)}%`,
                      transition: "width 420ms cubic-bezier(0.22, 1, 0.36, 1)",
                    }}
                    className="bg-sp-sem-violet-dot"
                  />
                  <div
                    style={{
                      width: `${pctOf(snap.toolSec, snap.totalTimeSec)}%`,
                      transition: "width 420ms cubic-bezier(0.22, 1, 0.36, 1)",
                    }}
                    className="bg-sp-sem-emerald-dot"
                  />
                </div>
                <div className="space-y-2">
                  <LegendRow dotClass="bg-sp-sem-violet-dot" label={t("agent.pulse.modelDurationLabel")} value={formatDuration(modelSec, t)} />
                  <LegendRow dotClass="bg-sp-sem-emerald-dot" label={t("agent.pulse.toolDurationLabel")} value={formatDuration(snap.toolSec, t)} />
                  <LegendRow label={t("agent.pulse.tpsLabel")} value={snap.tps > 0 ? `${snap.tps} tok/s` : "—"} />
                </div>
              </div>
            </div>
          )}

          {activeTab === "usage" && (
            <div data-testid="pulse-panel-usage" className="sp-morph-content-enter space-y-1.5">
              <div className="flex items-center justify-between gap-2 px-2.5 pt-1.5 pb-1">
                <div className="flex items-center gap-2 min-w-0">
                  <span className="text-[13px] font-semibold tracking-tight text-sp-text">{t("agent.pulse.tokenUsageHeading")}</span>
                  {snap.hasReportedUsage ? (
                    <span className="sp-field-well rounded-full bg-sp-hover px-2 py-0.5 font-mono tabular-nums text-sp-text-secondary">
                      {snap.totalTokens.toLocaleString("en-US")}
                    </span>
                  ) : null}
                </div>
                {renderModeToggle()}
              </div>
              <div className="sp-solid-glass-card rounded-[13px] bg-sp-surface p-3 space-y-2.5 dark:bg-sp-card">
                {snap.hasReportedUsage && (
                  <div className="sp-meter-track flex h-1.5 w-full gap-0.5 overflow-hidden rounded-full bg-sp-hover">
                    <div
                      style={{
                        width: `${pctOf(snap.inputTokens, snap.totalTokens)}%`,
                        transition: "width 420ms cubic-bezier(0.22, 1, 0.36, 1)",
                      }}
                      className="bg-sp-accent"
                    />
                    <div
                      style={{
                        width: `${pctOf(snap.outputTokens, snap.totalTokens)}%`,
                        transition: "width 420ms cubic-bezier(0.22, 1, 0.36, 1)",
                      }}
                      className="bg-sp-warning-text"
                    />
                  </div>
                )}
                <div className="space-y-2">
                  <LegendRow
                    dotClass="bg-sp-accent"
                    label={t("agent.pulse.inputTokensLabel")}
                    value={snap.hasReportedUsage ? formatCompactTokens(snap.inputTokens) : "—"}
                  />
                  <LegendRow
                    dotClass="bg-sp-warning-text"
                    label={t("agent.pulse.outputTokensLabel")}
                    value={snap.hasReportedUsage ? formatCompactTokens(snap.outputTokens) : "—"}
                  />
                  <LegendRow
                    dotClass="bg-sp-sem-emerald-dot"
                    label={t("agent.pulse.cacheReadLabel")}
                    value={
                      snap.cachedTokens > 0
                        ? `${formatCompactTokens(snap.cachedTokens)} (${cacheRateDisplay})`
                        : cacheRateDisplay
                    }
                  />
                </div>
              </div>
              <div className="px-2.5 pt-0.5 pb-1 text-sp-text-muted">
                {t("agent.pulse.reportedRunsNote", { count: snap.reportedRunsCount })}
              </div>
            </div>
          )}

          {activeTab === "context" && (
            <div data-testid="pulse-panel-context" className="sp-morph-content-enter space-y-1.5">
              {/* 顶行托盘眉栏：上下文容量 + 真实模型徽章 + 内联 [极简 | 详细] 凹槽切换开关 */}
              <div className="flex items-center justify-between gap-2 px-2.5 pt-1.5 pb-1">
                <div className="flex min-w-0 items-center gap-1.5">
                  <span className="shrink-0 text-[13px] font-semibold tracking-tight text-sp-text">
                    {t("agent.pulse.contextCapacityHeading")}
                  </span>
                  <span
                    data-testid="pulse-context-model-badge"
                    title={resolvedModelBadge}
                    className="sp-field-well max-w-[132px] truncate rounded-full border border-sp-line bg-sp-hover px-2 py-0.5 text-xs font-medium text-sp-text-secondary"
                  >
                    {resolvedModelBadge}
                  </span>
                </div>
                {renderModeToggle()}
              </div>

              {/* 内层纯白/碳锌实体玻璃卡片：汇总数值 + 内凹四色分段槽 + 四行图例明细 */}
              <div className="sp-solid-glass-card rounded-[13px] bg-sp-surface p-3 space-y-2.5 dark:bg-sp-card">
                {/* 汇总行：左侧 已用 XX.X%，右侧 ~XXK / 1.05M（带里程表平滑插值） */}
                <div className="flex items-baseline justify-between gap-2 tabular-nums">
                  <span className="text-sm">
                    <strong className="font-semibold text-sp-text">
                      {t("agent.pulse.usedPctLabel", { pct: usedPctDisplay })}
                    </strong>
                  </span>
                  <span
                    data-testid="pulse-context-summary-ratio"
                    className="font-mono text-sp-text-muted font-medium tabular-nums"
                  >
                    {usedTokensDisplay} / {formatCompactTokens(snap.maxContextTokens)}
                  </span>
                </div>

                {/* 四色分段进度条（带 Solid Glass 内凹槽 .sp-meter-track 与弹簧宽度过渡） */}
                <div
                  data-testid="pulse-context-segmented-bar"
                  className="sp-meter-track relative flex h-2 w-full gap-0.5 overflow-hidden rounded-full bg-sp-hover"
                >
                  {catList.map((cat) => {
                    const widthPct =
                      snap.currentContextTokens > 0 && cat.tokens > 0
                        ? Math.max(1.5, (cat.tokens / snap.maxContextTokens) * 100)
                        : 0;
                    return widthPct > 0 ? (
                      <div
                        key={cat.key}
                        data-testid={`pulse-bar-seg-${cat.key}`}
                        style={{
                          width: `${Math.min(100, widthPct)}%`,
                          transition: "width 420ms cubic-bezier(0.22, 1, 0.36, 1)",
                        }}
                        className={cn(
                          "h-full",
                          isCompacting ? "bg-sp-sem-emerald-dot" : cat.barClass
                        )}
                      />
                    ) : null;
                  })}
                  {isPro && (
                    <div
                      style={{ left: `${snap.autoCompactPct}%` }}
                      className="absolute bottom-0 top-0 w-0.5 bg-sp-danger-text"
                    />
                  )}
                </div>

                {/* 四行图例明细 */}
                <div data-testid="pulse-context-legend" className="space-y-1.5 pt-0.5">
                  {catList.map((cat) => (
                    <div
                      key={cat.key}
                      data-testid={`pulse-legend-${cat.key}`}
                      className="flex items-center justify-between gap-3 tabular-nums"
                    >
                      <span className="flex min-w-0 items-center gap-2 text-sp-text-secondary">
                        <span className={cn("h-2 w-2 shrink-0 rounded-full", cat.dotClass)} />
                        <span className="truncate">{cat.label}</span>
                      </span>
                      <span className="shrink-0 font-mono text-sp-text-secondary">
                        <strong className="font-semibold text-sp-text">
                          ~{formatCompactTokens(cat.tokens)}
                        </strong>
                        <span className="mx-1 text-sp-text-muted">·</span>
                        <span>
                          {cat.tokens > 0 && cat.pct === 0
                            ? "<0.1%"
                            : `${cat.pct.toFixed(1)}%`}
                        </span>
                      </span>
                    </div>
                  ))}
                </div>
              </div>

              {/* 底部托盘栏：极简模式展示平均缓存命中率；详细模式仅保留自动压缩水位与剩余空间（严禁与会话统计/Token用量面板重复） */}
              <div className="px-2.5 pt-1 pb-1 space-y-1 tabular-nums">
                {!isPro ? (
                  <div className="flex items-center justify-between">
                    <span className="text-sp-text-secondary">{t("agent.pulse.avgCacheHitLabel")}</span>
                    <strong
                      data-testid="pulse-context-cache-hit-rate"
                      className="font-mono font-semibold text-sp-text"
                    >
                      {cacheRateDisplay}
                    </strong>
                  </div>
                ) : (
                  <div className="space-y-1">
                    <LegendRow
                      label={t("agent.pulse.autoCompactThresholdLabel", {
                        tokens: formatCompactTokens(snap.autoCompactTokens),
                        pct: snap.autoCompactPct,
                      })}
                      value={`${pctOf(snap.currentContextTokens, snap.autoCompactTokens).toFixed(0)}%`}
                    />
                    <div className="text-sp-text-muted">
                      {t("agent.pulse.remainingToCompact", {
                        tokens: formatCompactTokens(snap.remainingToCompactTokens),
                      })}
                    </div>
                  </div>
                )}
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
