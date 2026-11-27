/**
 * AgentTaskProgressCard.tsx
 * 聊天流右上角按需浮现的多步任务进度卡（采用统一的 4-substep 新版弹簧展开动效 + 防闪烁状态守卫）：
 * - 简单单步问答完全隐藏，仅在多步计划（>=2 步）或多源复杂工具链（>=3 步）时自动浮现；
 * - 胶囊 <-> 展开卡片采用 `springDropdownMotion.ts` 连续弹簧形变（宽、高、圆角与 45ms 行级级联淡入，右上角锚定，零文字模糊）；
 * - 防闪烁纪律：完成对勾自绘动画（`sp-check-draw`）严格仅在步骤由进行中/待执行实时跃迁至完成时触发一次，展开卡片时已完成项保持稳定静止，绝不重播闪烁。
 */

import React, {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { AlertTriangle, ChevronDown, ListTodo, X } from "lucide-react";
import { cn } from "@/lib/utils";
import {
  SPRING_PRESETS,
  clamp,
  createSpring,
  prefersReducedMotion,
  settleIfClose,
  snapSpring,
  stepSpringSubdivided,
  tuneSpring,
  type Spring1D,
} from "@/lib/springDropdownMotion";
import type {
  StreamItem,
  StreamPlanCardItem,
  StreamProgressCardItem,
  StreamToolGroupItem,
} from "@/shell/api/agentStreamLogic";
import { projectPlanStepStatus } from "@/shell/api/agentConversationLogic";
import { resolveAgentToolDisplayName } from "@/shell/api/agentLogic";
import {
  renderToolSemanticGlyph,
  EO_MONOCHROME_ICON_CLASS,
} from "./agentToolSemantics";

export type TaskProgressStepStatus = "done" | "running" | "pending" | "failed";

export interface DerivedTaskProgressStep {
  id: string;
  toolId?: string;
  label: string;
  status: TaskProgressStepStatus;
}

export interface DerivedTurnTaskProgress {
  turnKey: string;
  steps: DerivedTaskProgressStep[];
  doneCount: number;
  totalCount: number;
  hasRunning: boolean;
  hasFailed: boolean;
  isTurnSettled: boolean;
}

export function deriveTurnTaskProgress(
  classifiedItems: readonly StreamItem[],
  activeRunId?: string | null
): DerivedTurnTaskProgress | null {
  if (!classifiedItems || classifiedItems.length === 0) return null;

  let lastUserIdx = -1;
  for (let i = classifiedItems.length - 1; i >= 0; i--) {
    const it = classifiedItems[i];
    if (it.kind === "message" && it.role === "user") {
      lastUserIdx = i;
      break;
    }
  }

  const slice = classifiedItems.slice(lastUserIdx + 1);
  if (slice.length === 0) return null;

  const turnRunId =
    slice.find(
      (it): it is Extract<StreamItem, { run_id: string }> =>
        "run_id" in it && Boolean(it.run_id)
    )?.run_id ?? `turn-${lastUserIdx}`;

  const hasTerminalEvent = slice.some(
    (it) => it.kind === "conclusion" || it.kind === "error"
  );
  const isTurnSettled =
    hasTerminalEvent && (!activeRunId || activeRunId !== turnRunId);

  let latestPlan: StreamPlanCardItem | null = null;
  let latestProgress: StreamProgressCardItem | null = null;
  for (const it of slice) {
    if (it.kind === "plan_card") latestPlan = it;
    if (it.kind === "progress_card") latestProgress = it;
  }

  if (latestPlan && latestPlan.steps.length >= 2) {
    const progressMatches =
      latestProgress && latestProgress.plan_id === latestPlan.plan_id
        ? latestProgress
        : null;
    const steps: DerivedTaskProgressStep[] = latestPlan.steps.map((st, idx) => {
      const projected = projectPlanStepStatus(
        st.status,
        progressMatches?.status ?? (isTurnSettled ? "completed" : null),
        progressMatches?.code ?? null
      );
      let status: TaskProgressStepStatus = "pending";
      if (projected === "done") status = "done";
      else if (projected === "running") status = "running";
      else if (projected === "failed" || projected === "expired") status = "failed";
      else if (isTurnSettled) status = "done";

      return {
        id: `${latestPlan!.plan_id}-${idx}`,
        toolId: st.tool_id,
        label: st.label_zh || resolveAgentToolDisplayName(st.tool_id),
        status,
      };
    });

    const doneCount = steps.filter((s) => s.status === "done").length;
    const hasRunning = steps.some((s) => s.status === "running");
    const hasFailed = steps.some((s) => s.status === "failed");
    return {
      turnKey: `plan:${latestPlan.plan_id}`,
      steps,
      doneCount,
      totalCount: steps.length,
      hasRunning,
      hasFailed,
      isTurnSettled,
    };
  }

  const rounds = slice
    .filter((it): it is StreamToolGroupItem => it.kind === "tool_group")
    .flatMap((g) => g.rounds || []);

  if (rounds.length < 3) return null;

  const steps: DerivedTaskProgressStep[] = rounds.slice(0, 8).map((r, idx) => {
    const baseName = resolveAgentToolDisplayName(r.tool_id);
    const shortSummary =
      typeof r.summary_zh === "string" &&
      r.summary_zh.trim().length > 0 &&
      r.summary_zh.trim().length <= 24
        ? ` · ${r.summary_zh.trim()}`
        : "";
    let status: TaskProgressStepStatus = "done";
    if (r.status === "running") status = isTurnSettled ? "done" : "running";
    else if (r.status === "failed") status = "failed";
    return {
      id: r.call_id || `tool-${idx}`,
      toolId: r.tool_id,
      label: `${baseName}${shortSummary}`,
      status,
    };
  });

  const doneCount = steps.filter((s) => s.status === "done").length;
  const hasRunning = steps.some((s) => s.status === "running");
  const hasFailed = steps.some((s) => s.status === "failed");
  return {
    turnKey: `tools:${turnRunId}`,
    steps,
    doneCount,
    totalCount: steps.length,
    hasRunning,
    hasFailed,
    isTurnSettled,
  };
}

function canAnimateSpringInBrowser(el: HTMLElement | null): boolean {
  if (!el || prefersReducedMotion()) return false;
  if (typeof window === "undefined" || typeof window.requestAnimationFrame !== "function") {
    return false;
  }
  return el.offsetHeight > 0 || el.offsetWidth > 0;
}

const EXPANDED_CARD_W = 268;
const PILL_FALLBACK_W = 148;
const PILL_FALLBACK_H = 30;

export interface AgentTaskProgressCardProps {
  classifiedItems: readonly StreamItem[];
  activeRunId?: string | null;
  compactWhenSplit?: boolean;
  className?: string;
}

export function AgentTaskProgressCard({
  classifiedItems,
  activeRunId,
  compactWhenSplit = false,
  className,
}: AgentTaskProgressCardProps) {
  const progress = useMemo(
    () => deriveTurnTaskProgress(classifiedItems, activeRunId),
    [classifiedItems, activeRunId]
  );

  const [dismissedTurnKey, setDismissedTurnKey] = useState<string | null>(null);
  const [userExpandedOverride, setUserExpandedOverride] = useState<{
    turnKey: string;
    expanded: boolean;
  } | null>(null);
  const [autoCollapsedTurnKey, setAutoCollapsedTurnKey] = useState<string | null>(() =>
    progress && progress.isTurnSettled ? progress.turnKey : null
  );
  const prevSettledRef = useRef<{ turnKey: string; settled: boolean } | null>(
    progress ? { turnKey: progress.turnKey, settled: progress.isTurnSettled } : null
  );

  // 防闪烁对勾守卫：仅记录从 running/pending 实时跃迁到 done 的步骤 ID；已完成的步骤在展开卡片时绝不重新播放 stroke-dashoffset 动画
  const prevStepStatusesRef = useRef<Map<string, TaskProgressStepStatus>>(new Map());
  const freshlyCompletedIds = useMemo(() => {
    const fresh = new Set<string>();
    if (!progress) return fresh;
    for (const st of progress.steps) {
      const prev = prevStepStatusesRef.current.get(st.id);
      if (prev !== undefined && prev !== "done" && st.status === "done") {
        fresh.add(st.id);
      }
    }
    return fresh;
  }, [progress]);

  useEffect(() => {
    if (!progress) {
      prevStepStatusesRef.current.clear();
      return;
    }
    const nextMap = new Map<string, TaskProgressStepStatus>();
    for (const st of progress.steps) {
      nextMap.set(st.id, st.status);
    }
    prevStepStatusesRef.current = nextMap;
  }, [progress]);

  useEffect(() => {
    if (!progress) {
      prevSettledRef.current = null;
      return;
    }
    const prev = prevSettledRef.current;
    prevSettledRef.current = { turnKey: progress.turnKey, settled: progress.isTurnSettled };

    if (!prev || prev.turnKey !== progress.turnKey) {
      if (progress.isTurnSettled) {
        setAutoCollapsedTurnKey(progress.turnKey);
      } else {
        setAutoCollapsedTurnKey(null);
      }
      return;
    }

    if (!prev.settled && progress.isTurnSettled) {
      const timer = window.setTimeout(() => {
        setAutoCollapsedTurnKey(progress.turnKey);
      }, 1200);
      return () => window.clearTimeout(timer);
    }
  }, [progress]);

  const isVisible = Boolean(progress && dismissedTurnKey !== progress.turnKey);
  const isExpanded = Boolean(
    progress &&
      (userExpandedOverride?.turnKey === progress.turnKey
        ? userExpandedOverride.expanded
        : !compactWhenSplit && autoCollapsedTurnKey !== progress.turnKey)
  );

  // `isClosingAnim` 仅在真实浏览器动画收缩期间短暂保持卡片内容挂载以平滑弹回胶囊；jsdom 或 reduced-motion 下立即同步切换
  const [isClosingAnim, setIsClosingAnim] = useState(false);
  const outerRef = useRef<HTMLElement | null>(null);
  const cardBodyRef = useRef<HTMLDivElement | null>(null);
  const pillBodyRef = useRef<HTMLDivElement | null>(null);
  const pillOverlayRef = useRef<HTMLDivElement | null>(null);
  const rafRef = useRef<number | null>(null);
  const settleTimerRef = useRef<number | null>(null);
  const prevExpandedRef = useRef<boolean>(isExpanded);
  const lastPillSizeRef = useRef<{ w: number; h: number }>({
    w: PILL_FALLBACK_W,
    h: PILL_FALLBACK_H,
  });

  const widthSpringRef = useRef<Spring1D>(
    createSpring(
      isExpanded ? EXPANDED_CARD_W : PILL_FALLBACK_W,
      SPRING_PRESETS.dockOpenWidth.k,
      SPRING_PRESETS.dockOpenWidth.c
    )
  );
  const heightSpringRef = useRef<Spring1D>(
    createSpring(
      isExpanded ? 140 : PILL_FALLBACK_H,
      SPRING_PRESETS.dockOpenHeight.k,
      SPRING_PRESETS.dockOpenHeight.c
    )
  );
  const radiusSpringRef = useRef<Spring1D>(
    createSpring(isExpanded ? 14 : 20, SPRING_PRESETS.selectOpen.k, SPRING_PRESETS.selectOpen.c)
  );
  const contentFadeRef = useRef<Spring1D>(
    createSpring(isExpanded ? 1 : 0, SPRING_PRESETS.fade.k, SPRING_PRESETS.fade.c)
  );
  const itemSpringsRef = useRef<Spring1D[]>([]);

  const stopRaf = useCallback(() => {
    if (rafRef.current !== null && typeof window !== "undefined") {
      window.cancelAnimationFrame(rafRef.current);
      rafRef.current = null;
    }
    if (settleTimerRef.current !== null && typeof window !== "undefined") {
      window.clearTimeout(settleTimerRef.current);
      settleTimerRef.current = null;
    }
  }, []);

  const clearSpringStyles = useCallback(() => {
    const outer = outerRef.current;
    const cardBody = cardBodyRef.current;
    const pillOverlay = pillOverlayRef.current;
    if (outer) {
      outer.style.width = "";
      outer.style.height = "";
      outer.style.borderRadius = "";
      outer.style.overflow = "";
      outer.style.transform = "";
    }
    if (pillOverlay) {
      pillOverlay.style.opacity = "0";
    }
    if (cardBody) {
      cardBody.style.opacity = "";
      const rows = cardBody.querySelectorAll<HTMLElement>("[data-spring-item]");
      rows.forEach((el) => {
        el.style.opacity = "";
        el.style.transform = "";
      });
    }
  }, []);

  useEffect(() => () => stopRaf(), [stopRaf]);

  useLayoutEffect(() => {
    if (!isVisible) {
      stopRaf();
      setIsClosingAnim(false);
      prevExpandedRef.current = isExpanded;
      return;
    }

    const outer = outerRef.current;
    const measureEl =
      pillBodyRef.current && pillBodyRef.current.offsetWidth > 0
        ? pillBodyRef.current
        : pillOverlayRef.current && pillOverlayRef.current.offsetWidth > 0
          ? pillOverlayRef.current
          : null;
    if (measureEl) {
      lastPillSizeRef.current = {
        w: measureEl.offsetWidth + 2,
        h: measureEl.offsetHeight + 2,
      };
    }

    const wasExpanded = prevExpandedRef.current;
    prevExpandedRef.current = isExpanded;
    if (wasExpanded === isExpanded) return;

    if (!canAnimateSpringInBrowser(outer)) {
      stopRaf();
      clearSpringStyles();
      setIsClosingAnim(false);
      return;
    }

    if (isExpanded) {
      // Pill -> Expanded Card 新版弹簧展开（双层叠化 + 外层 stagger 不覆盖已完成行的初始灰态）
      setIsClosingAnim(false);
      const cardBody = cardBodyRef.current;
      const targetH = cardBody && cardBody.offsetHeight > 0 ? cardBody.offsetHeight + 2 : 148;
      const startW = lastPillSizeRef.current.w;
      const startH = lastPillSizeRef.current.h;

      snapSpring(widthSpringRef.current, startW);
      snapSpring(heightSpringRef.current, startH);
      snapSpring(radiusSpringRef.current, 20);
      snapSpring(contentFadeRef.current, 0);

      tuneSpring(widthSpringRef.current, SPRING_PRESETS.dockOpenWidth.k, SPRING_PRESETS.dockOpenWidth.c, false);
      tuneSpring(heightSpringRef.current, SPRING_PRESETS.dockOpenHeight.k, SPRING_PRESETS.dockOpenHeight.c, false);
      tuneSpring(radiusSpringRef.current, SPRING_PRESETS.selectOpen.k, SPRING_PRESETS.selectOpen.c, false);
      tuneSpring(contentFadeRef.current, SPRING_PRESETS.fade.k, SPRING_PRESETS.fade.c, false);

      widthSpringRef.current.t = EXPANDED_CARD_W;
      heightSpringRef.current.t = targetH;
      radiusSpringRef.current.t = 14;
      contentFadeRef.current.t = 1;

      const rowEls = cardBody
        ? Array.from(cardBody.querySelectorAll<HTMLElement>("[data-spring-item]"))
        : [];
      itemSpringsRef.current = rowEls.map(() =>
        createSpring(0, SPRING_PRESETS.fade.k, SPRING_PRESETS.fade.c)
      );
      rowEls.forEach((el) => {
        el.style.opacity = "0";
        el.style.transform = "translate3d(0, 6px, 0)";
      });

      if (pillOverlayRef.current) {
        pillOverlayRef.current.style.opacity = "1";
      }
      if (cardBody) {
        cardBody.style.opacity = "0";
      }
      if (outer) {
        outer.style.width = `${startW}px`;
        outer.style.height = `${startH}px`;
        outer.style.borderRadius = "20px";
        outer.style.overflow = "hidden";
        outer.style.transformOrigin = "top right";
      }

      stopRaf();
      let last = performance.now();
      let elapsedSec = 0;
      const tickOpen = (now: number) => {
        const dt = Math.min((now - last) / 1000, 1 / 30);
        last = now;
        elapsedSec += dt;

        const liveCardH =
          cardBodyRef.current && cardBodyRef.current.offsetHeight > 0
            ? cardBodyRef.current.offsetHeight + 2
            : targetH;
        heightSpringRef.current.t = liveCardH;

        stepSpringSubdivided(widthSpringRef.current, dt);
        stepSpringSubdivided(heightSpringRef.current, dt);
        stepSpringSubdivided(radiusSpringRef.current, dt);
        stepSpringSubdivided(contentFadeRef.current, dt);

        const w = clamp(widthSpringRef.current.x, startW * 0.92, EXPANDED_CARD_W + 10);
        const h = clamp(heightSpringRef.current.x, startH * 0.92, liveCardH + 8);
        const r = clamp(radiusSpringRef.current.x, 12, 24);
        const op = clamp(contentFadeRef.current.x, 0, 1);
        const pillOp = clamp(1 - op * 2.6, 0, 1);
        const cardOp = clamp((op - 0.08) / 0.92, 0, 1);

        if (outerRef.current) {
          outerRef.current.style.width = `${w.toFixed(2)}px`;
          outerRef.current.style.height = `${h.toFixed(2)}px`;
          outerRef.current.style.borderRadius = `${r.toFixed(2)}px`;
        }
        if (pillOverlayRef.current) {
          pillOverlayRef.current.style.opacity = `${pillOp.toFixed(3)}`;
        }
        if (cardBodyRef.current) {
          cardBodyRef.current.style.opacity = `${cardOp.toFixed(3)}`;
        }

        let allRowsSettled = true;
        rowEls.forEach((el, idx) => {
          const sp = itemSpringsRef.current[idx];
          if (!sp) return;
          if (elapsedSec * 1000 >= idx * SPRING_PRESETS.staggerMs) {
            sp.t = 1;
            stepSpringSubdivided(sp, dt);
          }
          const rp = clamp(sp.x, 0, 1);
          el.style.opacity = `${rp.toFixed(3)}`;
          el.style.transform = `translate3d(0, ${((1 - rp) * 6).toFixed(2)}px, 0)`;
          if (!settleIfClose(sp, 0.01, 0.06) || sp.t < 1) {
            allRowsSettled = false;
          }
        });

        const settled =
          settleIfClose(widthSpringRef.current, 0.35, 1.2) &&
          settleIfClose(heightSpringRef.current, 0.35, 1.2) &&
          settleIfClose(radiusSpringRef.current, 0.2, 0.8) &&
          settleIfClose(contentFadeRef.current, 0.015, 0.08) &&
          allRowsSettled;

        if (settled) {
          stopRaf();
          clearSpringStyles();
          return;
        }
        rafRef.current = window.requestAnimationFrame(tickOpen);
      };
      rafRef.current = window.requestAnimationFrame(tickOpen);
      if (settleTimerRef.current === null && typeof window !== "undefined") {
        settleTimerRef.current = window.setTimeout(() => {
          settleTimerRef.current = null;
          stopRaf();
          clearSpringStyles();
        }, 380);
      }
      return;
    }

    // Expanded Card -> Pill 临界过阻尼无回弹收拢（k=440, c=44, ζ≈1.05，双层精准叠化零跳变）
    const startH = outer && outer.offsetHeight > 0 ? outer.offsetHeight : 148;
    const exactPillW =
      pillOverlayRef.current && pillOverlayRef.current.offsetWidth > 0
        ? pillOverlayRef.current.offsetWidth + 2
        : lastPillSizeRef.current.w;
    const exactPillH =
      pillOverlayRef.current && pillOverlayRef.current.offsetHeight > 0
        ? pillOverlayRef.current.offsetHeight + 2
        : lastPillSizeRef.current.h;
    lastPillSizeRef.current = { w: exactPillW, h: exactPillH };

    setIsClosingAnim(true);
    snapSpring(widthSpringRef.current, EXPANDED_CARD_W);
    snapSpring(heightSpringRef.current, startH);
    snapSpring(radiusSpringRef.current, 14);
    snapSpring(contentFadeRef.current, 1);

    tuneSpring(widthSpringRef.current, SPRING_PRESETS.pillCollapse.k, SPRING_PRESETS.pillCollapse.c, false);
    tuneSpring(heightSpringRef.current, SPRING_PRESETS.pillCollapse.k, SPRING_PRESETS.pillCollapse.c, false);
    tuneSpring(radiusSpringRef.current, SPRING_PRESETS.pillCollapse.k, SPRING_PRESETS.pillCollapse.c, false);
    tuneSpring(contentFadeRef.current, SPRING_PRESETS.pillCollapse.k, SPRING_PRESETS.pillCollapse.c, false);

    widthSpringRef.current.t = exactPillW;
    heightSpringRef.current.t = exactPillH;
    radiusSpringRef.current.t = 20;
    contentFadeRef.current.t = 0;

    if (pillOverlayRef.current) {
      pillOverlayRef.current.style.opacity = "0";
    }
    if (outer) {
      outer.style.width = `${EXPANDED_CARD_W}px`;
      outer.style.height = `${startH}px`;
      outer.style.borderRadius = "14px";
      outer.style.overflow = "hidden";
      outer.style.transformOrigin = "top right";
    }

    stopRaf();
    let last = performance.now();
    const tickClose = (now: number) => {
      const dt = Math.min((now - last) / 1000, 1 / 30);
      last = now;

      const livePillW =
        pillOverlayRef.current && pillOverlayRef.current.offsetWidth > 0
          ? pillOverlayRef.current.offsetWidth + 2
          : exactPillW;
      const livePillH =
        pillOverlayRef.current && pillOverlayRef.current.offsetHeight > 0
          ? pillOverlayRef.current.offsetHeight + 2
          : exactPillH;
      widthSpringRef.current.t = livePillW;
      heightSpringRef.current.t = livePillH;

      stepSpringSubdivided(widthSpringRef.current, dt);
      stepSpringSubdivided(heightSpringRef.current, dt);
      stepSpringSubdivided(radiusSpringRef.current, dt);
      stepSpringSubdivided(contentFadeRef.current, dt);

      const w = widthSpringRef.current.x;
      const h = heightSpringRef.current.x;
      const r = clamp(radiusSpringRef.current.x, 12, 24);
      const op = clamp(contentFadeRef.current.x, 0, 1);
      const pClose = 1 - op;
      const cardOp = clamp(1 - pClose * 2.2, 0, 1);
      const pillOp = clamp((pClose - 0.22) / 0.78, 0, 1);

      if (outerRef.current) {
        outerRef.current.style.width = `${w.toFixed(2)}px`;
        outerRef.current.style.height = `${h.toFixed(2)}px`;
        outerRef.current.style.borderRadius = `${r.toFixed(2)}px`;
      }
      if (cardBodyRef.current) {
        cardBodyRef.current.style.opacity = `${cardOp.toFixed(3)}`;
      }
      if (pillOverlayRef.current) {
        pillOverlayRef.current.style.opacity = `${pillOp.toFixed(3)}`;
      }

      const done =
        settleIfClose(widthSpringRef.current, 0.35, 1.2) &&
        settleIfClose(heightSpringRef.current, 0.35, 1.2) &&
        settleIfClose(contentFadeRef.current, 0.015, 0.08);

      if (done) {
        stopRaf();
        clearSpringStyles();
        setIsClosingAnim(false);
        return;
      }
      rafRef.current = window.requestAnimationFrame(tickClose);
    };
    rafRef.current = window.requestAnimationFrame(tickClose);
    if (settleTimerRef.current === null && typeof window !== "undefined") {
      settleTimerRef.current = window.setTimeout(() => {
        settleTimerRef.current = null;
        stopRaf();
        clearSpringStyles();
        setIsClosingAnim(false);
      }, 380);
    }
  }, [clearSpringStyles, isExpanded, isVisible, stopRaf]);

  if (!progress || !isVisible) {
    return null;
  }

  const pct =
    progress.totalCount > 0
      ? Math.min(100, Math.round((progress.doneCount / progress.totalCount) * 100))
      : 0;

  const toggleExpand = () => {
    setUserExpandedOverride({
      turnKey: progress.turnKey,
      expanded: !isExpanded,
    });
  };

  const renderCardView = isExpanded || isClosingAnim;
  const pillSummaryText =
    progress.isTurnSettled && progress.doneCount === progress.totalCount
      ? `已完成 ${progress.doneCount}/${progress.totalCount} 步`
      : `执行清单 ${progress.doneCount}/${progress.totalCount}`;

  return (
    <aside
      ref={outerRef}
      data-testid={isExpanded ? "agent-task-progress-card" : "agent-task-progress-pill"}
      data-spring-card="true"
      aria-label="本轮任务进度清单"
      className={cn(
        "relative border border-sp-line bg-sp-surface/95 text-xs text-sp-text backdrop-blur-xs select-none dark:bg-sp-card/95",
        renderCardView
          ? "w-[268px] rounded-[14px] sp-solid-glass-popover"
          : "inline-flex items-center rounded-full sp-solid-glass-card",
        className
      )}
    >
      {!renderCardView ? (
        <div
          ref={pillBodyRef}
          className="inline-flex items-center gap-1 pl-2.5 pr-1 py-1 whitespace-nowrap"
        >
          <button
            type="button"
            onClick={toggleExpand}
            aria-expanded="false"
            className="inline-flex items-center gap-1.5 cursor-pointer hover:text-sp-accent transition-colors"
          >
            {progress.hasFailed ? (
              <AlertTriangle className="h-3.5 w-3.5 shrink-0 text-sp-status-warning" />
            ) : (
              <svg
                viewBox="0 0 16 16"
                fill="none"
                className="h-3.5 w-3.5 shrink-0 text-sp-status-success"
              >
                <path
                  d="M3.5 8.5L6.5 11.5L12.5 4.5"
                  stroke="currentColor"
                  strokeWidth="1.8"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                />
              </svg>
            )}
            <span className="font-medium tabular-nums">{pillSummaryText}</span>
            <ChevronDown className="h-3 w-3 text-sp-text-muted" />
          </button>
          <button
            type="button"
            aria-label="关闭任务进度卡"
            onClick={() => setDismissedTurnKey(progress.turnKey)}
            className="flex h-5 w-5 items-center justify-center rounded-full text-sp-text-muted hover:bg-sp-hover hover:text-sp-text cursor-pointer"
          >
            <X className="h-3 w-3" />
          </button>
        </div>
      ) : (
        <>
          {/* 幽灵胶囊叠化层：既提供精准无误差的胶囊目标宽高，也在展开/收拢时平滑交叉叠化，杜绝收起回弹与跳变 */}
          <div
            ref={pillOverlayRef}
            aria-hidden="true"
            className="pointer-events-none absolute top-0 right-0 inline-flex items-center gap-1 pl-2.5 pr-1 py-1 whitespace-nowrap opacity-0"
          >
            <span className="inline-flex items-center gap-1.5">
              {progress.hasFailed ? (
                <AlertTriangle className="h-3.5 w-3.5 shrink-0 text-sp-status-warning" />
              ) : (
                <svg
                  viewBox="0 0 16 16"
                  fill="none"
                  className="h-3.5 w-3.5 shrink-0 text-sp-status-success"
                >
                  <path
                    d="M3.5 8.5L6.5 11.5L12.5 4.5"
                    stroke="currentColor"
                    strokeWidth="1.8"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                  />
                </svg>
              )}
              <span className="font-medium tabular-nums">{pillSummaryText}</span>
              <ChevronDown className="h-3 w-3 text-sp-text-muted" />
            </span>
            <span className="flex h-5 w-5 items-center justify-center">
              <X className="h-3 w-3 text-sp-text-muted" />
            </span>
          </div>

          <div ref={cardBodyRef} className="ml-auto w-[266px] p-2.5">
            {/* Header */}
            <div className="flex items-center justify-between gap-2 pb-1.5">
              <button
                type="button"
                onClick={toggleExpand}
                aria-expanded="true"
                className="flex min-w-0 items-center gap-1.5 cursor-pointer text-left"
            >
              {progress.hasRunning ? (
                <span
                  data-testid="task-progress-quiet-dots"
                  data-glyph-mode="orbit"
                  className="timeline-quiet-dots"
                  aria-hidden="true"
                >
                  <span />
                  <span />
                  <span />
                </span>
              ) : (
                <ListTodo
                  className={cn(
                    "h-3.5 w-3.5 shrink-0 stroke-[1.6]",
                    EO_MONOCHROME_ICON_CLASS
                  )}
                />
              )}
              <span className="font-semibold text-sp-text truncate">执行清单</span>
              <span className="rounded-full bg-sp-surface-muted border border-sp-line px-1.5 py-0.2 font-mono text-xs tabular-nums text-sp-text-secondary">
                {progress.doneCount}/{progress.totalCount}
              </span>
            </button>

            <div className="flex items-center gap-0.5 shrink-0">
              <button
                type="button"
                aria-label="收起任务进度卡"
                title="收起"
                onClick={toggleExpand}
                className="flex h-5 w-5 items-center justify-center rounded text-sp-text-muted hover:bg-sp-hover hover:text-sp-text cursor-pointer"
              >
                <ChevronDown className="h-3.5 w-3.5 rotate-180" />
              </button>
              <button
                type="button"
                aria-label="关闭任务进度卡"
                title="关闭"
                onClick={() => setDismissedTurnKey(progress.turnKey)}
                className="flex h-5 w-5 items-center justify-center rounded text-sp-text-muted hover:bg-sp-hover hover:text-sp-text cursor-pointer"
              >
                <X className="h-3.5 w-3.5" />
              </button>
            </div>
          </div>

          {/* Smooth Progress Track */}
          <div className="sp-meter-track mb-2 h-1 w-full overflow-hidden rounded-full bg-sp-hover">
            <div
              style={{ width: `${pct}%` }}
              className={cn(
                "h-full rounded-full transition-[width] duration-300",
                progress.hasFailed ? "bg-sp-status-warning" : "bg-sp-status-success"
              )}
            />
          </div>

          {/* Step Rows */}
          <div className="max-h-[188px] space-y-1 overflow-y-auto pr-0.5">
            {progress.steps.map((step) => {
              const isDone = step.status === "done";
              const isRunning = step.status === "running";
              const isFailed = step.status === "failed";
              const shouldAnimateCheck = isDone && freshlyCompletedIds.has(step.id);
              return (
                <div key={step.id} data-spring-item="">
                  <div
                    data-testid="agent-task-progress-row"
                    data-step-status={step.status}
                    className={cn(
                      "flex items-center gap-2 rounded-[7px] px-2 py-1",
                      isDone && "opacity-65 text-sp-text-muted",
                      isRunning && "sp-task-shimmer-line bg-sp-hover/60 font-medium"
                    )}
                  >
                    <span className="flex h-4 w-4 shrink-0 items-center justify-center">
                      {isDone ? (
                        <svg
                          viewBox="0 0 16 16"
                          fill="none"
                          className="h-3.5 w-3.5 text-sp-status-success"
                        >
                          <path
                            d="M3.5 8.5L6.5 11.5L12.5 4.5"
                            stroke="currentColor"
                            strokeWidth="1.8"
                            strokeLinecap="round"
                            strokeLinejoin="round"
                            className={shouldAnimateCheck ? "sp-check-draw" : undefined}
                          />
                        </svg>
                      ) : isRunning ? (
                        <span className="h-2 w-2 rounded-full bg-sp-accent" />
                      ) : isFailed ? (
                        <AlertTriangle className="h-3.5 w-3.5 text-sp-status-danger" />
                      ) : (
                        <span className="h-3 w-3 rounded-full border border-sp-line" />
                      )}
                    </span>

                    {step.toolId &&
                      renderToolSemanticGlyph(step.toolId, {
                        status: step.status,
                        className: "h-3 w-3 shrink-0",
                      })}

                    <span
                      className={cn(
                        "min-w-0 flex-1 truncate",
                        isFailed
                          ? "text-sp-danger-text"
                          : isDone
                            ? "text-sp-text-muted"
                            : "text-sp-text"
                      )}
                      title={step.label}
                    >
                      {step.label}
                    </span>
                  </div>
                </div>
              );
            })}
          </div>
          </div>
        </>
      )}
    </aside>
  );
}
