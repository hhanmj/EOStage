/**
 * AgentThinkingCard.tsx
 * Live ticking & auto-collapsing reasoning trace card for Agent 2.0.
 * Incorporates Kokonut UI per-character wave animation (blur-free for CJK)
 * with explicit `prefers-reduced-motion` support.
 *
 * Strict visibility invariant:
 * Returns `null` when `status !== "thinking"` AND `(thinkingText?.trim().length ?? 0) === 0`
 * so non-thinking turns NEVER render a bogus `已思考 0.0 秒` badge.
 */

import React, { useEffect, useMemo, useRef, useState } from "react";
import { Brain, ChevronRight } from "lucide-react";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n";
import { EO_MONOCHROME_ICON_CLASS } from "./agentToolSemantics";

export interface AgentThinkingCardProps {
  status: "thinking" | "done" | "idle";
  thinkingText?: string | null;
  startedAtMs?: number | null;
  durationMs?: number | null;
  /** 当模型未返回推理正文但有真实耗时时，是否渲染紧凑的顶部用时标题（无空展开箭头） */
  showElapsedWithoutText?: boolean;
  /** 是否嵌入在轮次统一「思考中 / 已思考」时间线内部（无嵌套二级折叠头，平铺展示思考段落） */
  embeddedInTurn?: boolean;
  defaultExpanded?: boolean;
  onExpandedChange?: (expanded: boolean) => void;
  onStop?: () => void;
  className?: string;
}

const WAVE_STEP_SEC = 0.07;
const WAVE_PERIOD_SEC = 1.55;

type TranslateFn = ReturnType<typeof useI18n>["t"];

/**
 * 统一思考与过程耗时格式化（秒 / 分+秒 / 小时+分+秒）：
 * - `< 60s`：`3.6 秒`（保留 0.1s 细腻度）
 * - `1m .. 59m59s`：`32 分 47 秒`（秒数固定两位补零 `00..59`，杜绝 `9 秒 -> 10 秒` 频繁跳宽）
 * - `>= 1h`：`1 小时 02 分 05 秒`（分钟与秒数均两位补零）
 */
export function formatThinkingElapsed(
  ms: number,
  translate: TranslateFn,
  isLive: boolean = false
): string {
  const safeMs = Number.isFinite(ms) ? Math.max(0, ms) : 0;
  if (safeMs < 60_000) {
    const secs = isLive
      ? Math.max(0, safeMs / 1000).toFixed(1)
      : Math.max(0.1, safeMs / 1000).toFixed(1);
    return translate("agent.thinking.durationSec", { seconds: secs });
  }
  const totalSec = Math.floor(safeMs / 1000);
  if (totalSec < 3600) {
    const minutes = Math.floor(totalSec / 60);
    const seconds = String(totalSec % 60).padStart(2, "0");
    return translate("agent.thinking.durationMinSec", { minutes, seconds });
  }
  const hours = Math.floor(totalSec / 3600);
  const minutes = String(Math.floor((totalSec % 3600) / 60)).padStart(2, "0");
  const seconds = String(totalSec % 60).padStart(2, "0");
  return translate("agent.thinking.durationHourMinSec", { hours, minutes, seconds });
}

/**
 * 将计时字符串中的纯数字片段锁定在等宽定宽盒（`calc(Nch + 1px)`）内渲染：
 * - 完整保持 `textContent` 不变（无额外空格，屏幕阅读器与文本断言完全兼容）；
 * - 隔离 CJK 与 ASCII 数字混排时的字体回退重排，确保 `0..9` 跳字时后续整行文字 `0.00px` 零抖动。
 */
export function renderWidthLockedTimerText(text: string): React.ReactNode {
  const parts = text.split(/(\d+(?:\.\d+)?)/g);
  return parts.map((part, idx) => {
    if (!part) return null;
    if (/^\d+(?:\.\d+)?$/.test(part)) {
      return (
        <span
          key={idx}
          data-timer-digits=""
          className="inline-block font-mono tabular-nums text-center whitespace-nowrap align-baseline"
          style={{ width: `calc(${part.length}ch + 1px)` }}
        >
          {part}
        </span>
      );
    }
    return <React.Fragment key={idx}>{part}</React.Fragment>;
  });
}

function computeWavePhaseBaseSec(): number {
  if (typeof performance !== "undefined" && typeof performance.now === "function") {
    return -((performance.now() / 1000) % WAVE_PERIOD_SEC);
  }
  return 0;
}

function prefersReducedMotion(): boolean {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") {
    return false;
  }
  try {
    return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  } catch {
    return false;
  }
}

function extractLatestReasoningLine(text: string): string | null {
  if (!text) return null;
  const lines = text
    .split(/\r?\n/)
    .map((l) => l.replace(/^[#>*\-\d.\s]+/, "").trim())
    .filter((l) => l.length > 0);
  if (lines.length === 0) return null;
  const last = lines[lines.length - 1];
  return last.length > 56 ? `${last.slice(0, 56)}…` : last;
}

export function AgentThinkingCard({
  status,
  thinkingText,
  startedAtMs,
  durationMs,
  showElapsedWithoutText = false,
  embeddedInTurn = false,
  defaultExpanded,
  onExpandedChange,
  onStop: _onStop,
  className,
}: AgentThinkingCardProps) {
  const { t } = useI18n();
  const trimmedText = typeof thinkingText === "string" ? thinkingText.trim() : "";
  const isThinking = status === "thinking";

  const mountStartRef = useRef<number>(Date.now());
  const wavePhaseBaseRef = useRef<number>(computeWavePhaseBaseSec());
  const prevStatusRef = useRef<AgentThinkingCardProps["status"]>(status);
  const userPinnedRef = useRef<boolean>(false);

  const [expanded, setExpanded] = useState<boolean>(() => {
    if (defaultExpanded !== undefined) return defaultExpanded;
    return status === "thinking";
  });
  const onExpandedChangeRef = useRef(onExpandedChange);
  onExpandedChangeRef.current = onExpandedChange;

  useEffect(() => {
    onExpandedChangeRef.current?.(expanded);
  }, [expanded]);

  const effectiveStartMs =
    typeof startedAtMs === "number" && Number.isFinite(startedAtMs) && startedAtMs > 0
      ? startedAtMs
      : mountStartRef.current;

  const [liveElapsedMs, setLiveElapsedMs] = useState<number>(() => {
    if (typeof durationMs === "number" && Number.isFinite(durationMs) && durationMs >= 0) {
      return durationMs;
    }
    return Math.max(0, Date.now() - effectiveStartMs);
  });

  const [frozenElapsedMs, setFrozenElapsedMs] = useState<number | null>(() => {
    if (typeof durationMs === "number" && Number.isFinite(durationMs) && durationMs >= 0) {
      return durationMs;
    }
    return null;
  });

  const reducedMotion = prefersReducedMotion();

  useEffect(() => {
    const prev = prevStatusRef.current;
    prevStatusRef.current = status;

    if (status === "thinking" && prev !== "thinking") {
      mountStartRef.current =
        typeof startedAtMs === "number" && Number.isFinite(startedAtMs) && startedAtMs > 0
          ? startedAtMs
          : Date.now();
      if (!userPinnedRef.current && defaultExpanded === undefined) {
        setExpanded(true);
      }
    } else if (prev === "thinking" && status !== "thinking") {
      const elapsed =
        typeof durationMs === "number" && Number.isFinite(durationMs) && durationMs >= 0
          ? durationMs
          : Math.max(100, Date.now() - effectiveStartMs);
      setFrozenElapsedMs(elapsed);
      if (!userPinnedRef.current) {
        setExpanded(false);
      }
    }
  }, [defaultExpanded, durationMs, effectiveStartMs, startedAtMs, status]);

  useEffect(() => {
    if (!isThinking) return;
    const currentElapsed =
      typeof durationMs === "number" && Number.isFinite(durationMs) && durationMs >= 0
        ? durationMs
        : Math.max(0, Date.now() - effectiveStartMs);
    const tickIntervalMs = reducedMotion || currentElapsed >= 60_000 ? 500 : 100;
    const updateTick = () => {
      if (typeof durationMs === "number" && Number.isFinite(durationMs) && durationMs >= 0) {
        setLiveElapsedMs(durationMs);
      } else {
        setLiveElapsedMs(Math.max(0, Date.now() - effectiveStartMs));
      }
    };
    updateTick();
    const id = setInterval(updateTick, tickIntervalMs);
    return () => clearInterval(id);
  }, [durationMs, effectiveStartMs, isThinking, reducedMotion, liveElapsedMs >= 60_000]);

  const latestLinePreview = useMemo(
    () => (isThinking && !expanded ? extractLatestReasoningLine(trimmedText) : null),
    [expanded, isThinking, trimmedText]
  );

  const hasPositiveDuration =
    typeof durationMs === "number" && Number.isFinite(durationMs) && durationMs > 0;

  if (!isThinking && trimmedText.length === 0 && !(showElapsedWithoutText && hasPositiveDuration)) {
    return null;
  }

  const resolvedMs =
    typeof durationMs === "number" && Number.isFinite(durationMs) && durationMs >= 0
      ? durationMs
      : isThinking
        ? liveElapsedMs
        : frozenElapsedMs ?? liveElapsedMs;

  const formattedDuration = formatThinkingElapsed(resolvedMs, t, isThinking);

  const timerLabel = isThinking
    ? t("agent.thinking.liveDuration", { duration: formattedDuration })
    : t("agent.thinking.doneDuration", { duration: formattedDuration });

  const handleToggle = () => {
    userPinnedRef.current = true;
    setExpanded((prev) => !prev);
  };

  if (!isThinking && trimmedText.length === 0) {
    return (
      <div
        data-testid="agent-thinking-card"
        data-thinking-status={status}
        data-expanded="false"
        data-reduced-motion={reducedMotion ? "true" : "false"}
        className={cn("inline-flex items-center gap-1.5 text-xs text-sp-text-muted w-fit select-none", className)}
      >
        <Brain className={cn("h-3.5 w-3.5 shrink-0 stroke-[1.6]", EO_MONOCHROME_ICON_CLASS)} />
        <span className="tabular-nums">{renderWidthLockedTimerText(timerLabel)}</span>
      </div>
    );
  }

  // 嵌入在轮次统一「思考中 / 已思考」时间线内部时：平铺单色 Brain 节点与思考内容，不重复渲染二级折叠按钮
  if (embeddedInTurn) {
    const inlineLabel =
      resolvedMs > 0
        ? t("agent.thinking.inlineDurationLabel", { duration: formattedDuration })
        : t("agent.thinking.inlineStepNoTime");
    return (
      <div
        data-testid="agent-thinking-card"
        data-thinking-status={status}
        data-expanded="true"
        data-reduced-motion={reducedMotion ? "true" : "false"}
        className={cn("flex flex-col gap-1 px-1.5 py-1 text-xs w-full select-text", className)}
      >
        <div className="flex items-center gap-2 text-xs text-sp-text-muted select-none">
          <span className="flex w-4 shrink-0 items-center justify-center">
            <Brain className={cn("h-3.5 w-3.5 shrink-0 stroke-[1.6]", EO_MONOCHROME_ICON_CLASS)} />
          </span>
          <span className="tabular-nums">{renderWidthLockedTimerText(inlineLabel)}</span>
        </div>
        {trimmedText.length > 0 && (
          <div
            data-testid="agent-thinking-body"
            className="pl-6 py-0.5 max-h-56 overflow-y-auto text-xs leading-[1.65] text-sp-text-secondary whitespace-pre-wrap break-words select-text"
          >
            {trimmedText}
          </div>
        )}
      </div>
    );
  }

  // Split `"思考中 · 1.6 秒"` into wave head (`"思考中"`) + steady tabular suffix (`" · 1.6 秒"`)
  const dotSepIdx = isThinking ? timerLabel.indexOf(" · ") : -1;
  const waveHead = dotSepIdx > 0 ? timerLabel.slice(0, dotSepIdx) : null;
  const timerTail = dotSepIdx > 0 ? timerLabel.slice(dotSepIdx) : timerLabel;

  return (
    <div
      data-testid="agent-thinking-card"
      data-thinking-status={status}
      data-expanded={expanded ? "true" : "false"}
      data-reduced-motion={reducedMotion ? "true" : "false"}
      className={cn("flex flex-col gap-1 w-full", className)}
    >
      <div className="flex items-center gap-2 min-w-0">
        <button
          type="button"
          data-testid="agent-thinking-toggle"
          onClick={handleToggle}
          aria-expanded={expanded}
          aria-label={expanded ? t("agent.thinking.collapseAria") : t("agent.thinking.expandAria")}
          className="cds-btn-squish inline-flex items-center gap-1.5 h-7 px-2 -ml-2 rounded-md text-xs text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover transition-colors duration-fast ease-sp cursor-pointer shrink-0 select-none"
        >
          <ChevronRight
            className={cn(
              "h-3.5 w-3.5 text-sp-text-muted shrink-0",
              !reducedMotion && "transition-transform duration-fast ease-sp",
              expanded && "rotate-90"
            )}
          />
          <Brain
            className={cn(
              "h-3.5 w-3.5 shrink-0 stroke-[1.6]",
              EO_MONOCHROME_ICON_CLASS
            )}
          />
          {isThinking && (
            <span
              data-testid="thinking-quiet-dots"
              data-glyph-mode="wave"
              className="timeline-quiet-dots"
              aria-hidden="true"
            >
              <span />
              <span />
              <span />
            </span>
          )}
          <span className="font-medium tabular-nums">
            {waveHead ? (
              <>
                {Array.from(waveHead).map((ch, idx) => (
                  <span
                    key={idx}
                    className={cn(!reducedMotion && "sp-ai-char-wave")}
                    style={
                      !reducedMotion
                        ? {
                            animationDelay: `${(
                              wavePhaseBaseRef.current +
                              idx * WAVE_STEP_SEC
                            ).toFixed(2)}s`,
                          }
                        : undefined
                    }
                  >
                    {ch === " " ? "\u00A0" : ch}
                  </span>
                ))}
                <span>{renderWidthLockedTimerText(timerTail)}</span>
              </>
            ) : (
              renderWidthLockedTimerText(timerLabel)
            )}
          </span>
        </button>

        {latestLinePreview && (
          <span className="truncate text-xs text-sp-text-muted select-none flex-1 min-w-0">
            {latestLinePreview}
          </span>
        )}
      </div>

      {expanded && trimmedText.length > 0 && (
        <div
          data-testid="agent-thinking-body"
          className="ml-2.5 border-l-[1.5px] border-sp-line pl-3.5 py-1 max-h-56 overflow-y-auto text-xs leading-[1.65] text-sp-text-secondary whitespace-pre-wrap break-words select-text"
        >
          {trimmedText}
        </div>
      )}
    </div>
  );
}

export interface UnifiedTurnProcessHeaderProps {
  turnKey: string;
  isActive: boolean;
  isExpanded: boolean;
  onToggle: () => void;
  onStop?: () => void;
  startedAtMs?: number | null;
  durationMs?: number | null;
  toolStepCount: number;
  runningToolHint?: string | null;
  liveSummaryPreview?: string | null;
  className?: string;
}

/**
 * 轮次级统一的「思考中 / 已思考」总收纳开关：
 * - 执行中：单色 Brain 图标 + 柔和逐字波浪「思考中 · X.X 秒」+ 正在执行的工具简述，右侧提供紧凑「停止」按钮；
 * - 完成后：将思考与全部工具执行步骤一并收纳为单行「已思考 X.X 秒 · N 个步骤」，点击一键展开平铺时间线。
 */
export function UnifiedTurnProcessHeader({
  turnKey,
  isActive,
  isExpanded,
  onToggle,
  onStop: _onStop,
  startedAtMs,
  durationMs,
  toolStepCount,
  runningToolHint,
  liveSummaryPreview,
  className,
}: UnifiedTurnProcessHeaderProps) {
  const { t } = useI18n();
  const mountStartRef = useRef<number>(Date.now());
  const wavePhaseBaseRef = useRef<number>(computeWavePhaseBaseSec());
  const reducedMotion = prefersReducedMotion();

  const effectiveStartMs =
    typeof startedAtMs === "number" && Number.isFinite(startedAtMs) && startedAtMs > 0
      ? startedAtMs
      : mountStartRef.current;

  const [liveElapsedMs, setLiveElapsedMs] = useState<number>(() => {
    if (typeof durationMs === "number" && Number.isFinite(durationMs) && durationMs > 0) {
      return durationMs;
    }
    return Math.max(0, Date.now() - effectiveStartMs);
  });

  const isMultiMinute = liveElapsedMs >= 60_000;

  useEffect(() => {
    if (!isActive) return;
    const tickIntervalMs = reducedMotion || isMultiMinute ? 500 : 100;
    const updateTick = () => {
      setLiveElapsedMs(Math.max(0, Date.now() - effectiveStartMs));
    };
    updateTick();
    const id = setInterval(updateTick, tickIntervalMs);
    return () => clearInterval(id);
  }, [effectiveStartMs, isActive, isMultiMinute, reducedMotion]);

  const resolvedMs = isActive
    ? liveElapsedMs
    : typeof durationMs === "number" && Number.isFinite(durationMs) && durationMs > 0
      ? durationMs
      : 0;

  const formattedDuration = isActive
    ? formatThinkingElapsed(resolvedMs, t, true)
    : resolvedMs > 0
      ? formatThinkingElapsed(resolvedMs, t, false)
      : null;

  const liveFullLabel = isActive
    ? t("agent.thinking.liveDuration", {
        duration: formattedDuration ?? formatThinkingElapsed(0, t, true),
      })
    : "";
  const dotSepIdx = isActive ? liveFullLabel.indexOf(" · ") : -1;
  const waveHead = dotSepIdx > 0 ? liveFullLabel.slice(0, dotSepIdx) : null;
  const timerTail = dotSepIdx > 0 ? liveFullLabel.slice(dotSepIdx) : liveFullLabel;

  const settledLabel = !isActive
    ? toolStepCount > 0
      ? formattedDuration
        ? t("agent.thinking.doneDurationWithSteps", {
            duration: formattedDuration,
            count: toolStepCount,
          })
        : t("agent.thinking.doneStepsOnly", { count: toolStepCount })
      : t("agent.thinking.doneDuration", {
          duration: formattedDuration ?? formatThinkingElapsed(100, t, false),
        })
    : "";

  return (
    <div className={cn("flex items-center justify-between gap-2 w-full min-w-0", className)}>
      <button
        type="button"
        data-testid="agent-turn-process-toggle"
        data-turn-key={turnKey}
        data-turn-active={isActive ? "true" : "false"}
        aria-expanded={isExpanded}
        onClick={onToggle}
        className="cds-btn-squish inline-flex items-center gap-1.5 h-7 px-2 -ml-2 rounded-md text-xs text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover transition-colors duration-fast ease-sp cursor-pointer max-w-full w-fit min-w-0 select-none"
      >
        <ChevronRight
          className={cn(
            "h-3.5 w-3.5 text-sp-text-muted shrink-0",
            !reducedMotion && "transition-transform duration-fast ease-sp",
            isExpanded && "rotate-90"
          )}
        />
        <Brain
          className={cn("h-3.5 w-3.5 shrink-0 stroke-[1.6]", EO_MONOCHROME_ICON_CLASS)}
        />
        {isActive && (
          <span
            data-testid="turn-process-quiet-dots"
            data-glyph-mode="wave"
            className="timeline-quiet-dots"
            aria-hidden="true"
          >
            <span />
            <span />
            <span />
          </span>
        )}
        <span className="font-medium tabular-nums shrink-0">
          {isActive ? (
            waveHead ? (
              <>
                {Array.from(waveHead).map((ch, idx) => (
                  <span
                    key={idx}
                    className={cn(!reducedMotion && "sp-ai-char-wave")}
                    style={
                      !reducedMotion
                        ? {
                            animationDelay: `${(
                              wavePhaseBaseRef.current +
                              idx * WAVE_STEP_SEC
                            ).toFixed(2)}s`,
                          }
                        : undefined
                    }
                  >
                    {ch === " " ? "\u00A0" : ch}
                  </span>
                ))}
                <span>{renderWidthLockedTimerText(timerTail)}</span>
              </>
            ) : (
              renderWidthLockedTimerText(liveFullLabel)
            )
          ) : (
            renderWidthLockedTimerText(settledLabel)
          )}
        </span>
        {isActive && runningToolHint ? (
          <span className="truncate text-sp-text-muted">
            {t("agent.thinking.liveRunningTool", { tool: runningToolHint })}
          </span>
        ) : isActive && !isExpanded && liveSummaryPreview ? (
          <span className="truncate text-sp-text-muted">{liveSummaryPreview}</span>
        ) : null}
      </button>
    </div>
  );
}
