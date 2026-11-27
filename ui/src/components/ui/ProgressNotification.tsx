import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AlertCircle, ArrowDownCircle, CheckCircle2, FolderArchive, Package, X } from "lucide-react";
import { cn } from "@/lib/utils";
import type { ApplicationTask } from "@/shell/api/types";
import {
  LONG_WORK_ENTRY_DELAY_MS,
  LONG_WORK_REASSURE_MS,
  estimateRemainingEta,
} from "@/lib/loadingStateMachine";
import {
  dismissLongWork,
  subscribeLongWork,
  type LongWorkItem,
} from "@/lib/longWorkRegistry";
import type { DesktopPendingDownloadPreparation } from "@/components/desktop/DesktopDownloadDrawer";
import { lookupSubsetSizeEstimate } from "@/lib/subsetSizeEstimateRegistry";
import { formatBytes } from "@/shell/api/rowProjection";
import { stripInternalDiagnosticSuffix } from "@/shell/api/resultListLogic";

export const DOWNLOAD_TOAST_AUTO_DISMISS_MS = 2000;
export const PROGRESS_TOAST_EXIT_MS = 260;

const COMPLETED_AUTO_DISMISS_MS = DOWNLOAD_TOAST_AUTO_DISMISS_MS;
const FAILED_AUTO_DISMISS_MS = DOWNLOAD_TOAST_AUTO_DISMISS_MS;
const TERMINAL_FALLBACK_GRACE_MS = 1000;

export interface ProgressNotificationHostProps {
  tasks: readonly ApplicationTask[];
  pendingPreparations: readonly DesktopPendingDownloadPreparation[];
  suppressDownloads?: boolean;
  /** Suppresses all floating top-right toast cards (e.g. while left notification drawer is open). */
  suppressAll?: boolean;
  /** Task IDs whose terminal notifications were already marked read or cleared in the notification center. */
  readTaskIds?: ReadonlySet<string>;
  /** Override auto-dismiss timeout (ms) for floating progress toasts; defaults to 2000ms. */
  autoDismissMs?: number;
  /** Override smooth dissolve exit duration (ms); defaults to 260ms. */
  exitAnimationMs?: number;
  onDismissTaskNotification?: (taskId: string) => void;
  onDismissPreparation?: (id: string) => void;
  onOpenTask?: (taskId: string) => void;
  onCancelTask?: (task: ApplicationTask) => void;
  onOpenDownloads?: () => void;
}

interface ProjectedCard {
  id: string;
  phaseKey: string;
  kind: "task" | "prep" | "longwork";
  category: "transfer" | "export" | "plugin";
  title: string;
  status: "running" | "completed" | "failed";
  percent: number | null;
  detailLine: string;
  etaLine: string;
  reassureLine: string;
  onCancel?: () => void;
  onDismiss?: () => void;
  onAutoDismissCleanup?: () => void;
  onOpen?: () => void;
}

interface CardTimerEntry {
  phaseKey: string;
  remainingMs: number;
  startedAtMs: number | null;
  timerId: number | null;
}

export function ProgressNotificationHost({
  tasks,
  pendingPreparations,
  suppressDownloads = false,
  suppressAll = false,
  readTaskIds,
  autoDismissMs = DOWNLOAD_TOAST_AUTO_DISMISS_MS,
  exitAnimationMs = PROGRESS_TOAST_EXIT_MS,
  onDismissTaskNotification,
  onDismissPreparation,
  onOpenTask,
  onCancelTask,
  onOpenDownloads,
}: ProgressNotificationHostProps) {
  const [longWorks, setLongWorks] = useState<LongWorkItem[]>([]);
  const [nowMs, setNowMs] = useState<number>(() => Date.now());
  const [dismissedTaskIds, setDismissedTaskIds] = useState<Set<string>>(() => new Set());
  const [dismissedPrepIds, setDismissedPrepIds] = useState<Set<string>>(() => new Set());
  const [autoDismissedPhases, setAutoDismissedPhases] = useState<Set<string>>(() => new Set());
  const [exitingCardIds, setExitingCardIds] = useState<Set<string>>(() => new Set());
  const [pausedCardIds, setPausedCardIds] = useState<Set<string>>(() => new Set());
  const [dragState, setDragState] = useState<{ id: string; startX: number; offsetX: number } | null>(null);

  // Track start times & active->terminal transitions seen in this session so historical
  // completed tasks at boot do not flood the top-right corner, while newly finished tasks
  // smoothly re-appear for 2s as a clickable completed/failed result notification.
  const seenTaskStartedRef = useRef<Map<string, number>>(new Map());
  const seenActiveTaskIdsRef = useRef<Set<string>>(new Set());
  const completedTaskAtRef = useRef<Map<string, number>>(new Map());
  const prepStartedAtRef = useRef<Map<string, number>>(new Map());
  const cardTimersRef = useRef<Map<string, CardTimerEntry>>(new Map());
  const exitTimersRef = useRef<Map<string, number>>(new Map());
  const pausedCardIdsRef = useRef<Set<string>>(pausedCardIds);
  pausedCardIdsRef.current = pausedCardIds;

  useEffect(() => subscribeLongWork(setLongWorks), []);

  const clearCardExitTimer = useCallback((cardId: string) => {
    const existing = exitTimersRef.current.get(cardId);
    if (existing !== undefined) {
      window.clearTimeout(existing);
      exitTimersRef.current.delete(cardId);
    }
  }, []);

  const clearCardAutoTimer = useCallback((cardId: string) => {
    const entry = cardTimersRef.current.get(cardId);
    if (entry && entry.timerId !== null) {
      window.clearTimeout(entry.timerId);
      entry.timerId = null;
      entry.startedAtMs = null;
    }
  }, []);

  useEffect(() => {
    return () => {
      for (const entry of cardTimersRef.current.values()) {
        if (entry.timerId !== null) {
          window.clearTimeout(entry.timerId);
        }
      }
      cardTimersRef.current.clear();
      for (const tid of exitTimersRef.current.values()) {
        window.clearTimeout(tid);
      }
      exitTimersRef.current.clear();
    };
  }, []);

  const triggerSmoothExit = useCallback(
    (cardId: string, onAfterExit: () => void) => {
      clearCardAutoTimer(cardId);
      clearCardExitTimer(cardId);
      if (exitAnimationMs <= 0) {
        onAfterExit();
        return;
      }
      setExitingCardIds((prev) => {
        if (prev.has(cardId)) return prev;
        const copy = new Set(prev);
        copy.add(cardId);
        return copy;
      });
      const tid = window.setTimeout(() => {
        exitTimersRef.current.delete(cardId);
        setExitingCardIds((prev) => {
          if (!prev.has(cardId)) return prev;
          const copy = new Set(prev);
          copy.delete(cardId);
          return copy;
        });
        onAfterExit();
      }, exitAnimationMs);
      exitTimersRef.current.set(cardId, tid);
    },
    [clearCardAutoTimer, clearCardExitTimer, exitAnimationMs]
  );

  // Keep transition timestamps synchronized with incoming `tasks` and `pendingPreparations`
  // so newly terminal (completed/failed) cards render immediately and start their 2s TTL.
  {
    const syncNow = Date.now();
    const activePrepIds = new Set(pendingPreparations.map((p) => p.id));
    for (const id of prepStartedAtRef.current.keys()) {
      if (!activePrepIds.has(id)) {
        prepStartedAtRef.current.delete(id);
      }
    }
    for (const prep of pendingPreparations) {
      if (!prepStartedAtRef.current.has(prep.id)) {
        prepStartedAtRef.current.set(prep.id, syncNow);
      }
    }
    for (const task of tasks) {
      if (task.status === "running" || task.status === "queued") {
        seenActiveTaskIdsRef.current.add(task.task_id);
        if (!seenTaskStartedRef.current.has(task.task_id)) {
          const createdMs = task.created_at ? Date.parse(task.created_at) : NaN;
          seenTaskStartedRef.current.set(
            task.task_id,
            Number.isFinite(createdMs) ? Math.min(createdMs, syncNow) : syncNow
          );
        }
        completedTaskAtRef.current.delete(task.task_id);
      } else if (
        (task.status === "completed" || task.status === "failed") &&
        seenActiveTaskIdsRef.current.has(task.task_id) &&
        !completedTaskAtRef.current.has(task.task_id)
      ) {
        completedTaskAtRef.current.set(task.task_id, syncNow);
      }
    }
  }

  const terminalFallbackTtl = useCallback(
    (status: "completed" | "failed") => {
      const base =
        status === "failed"
          ? Math.max(autoDismissMs, FAILED_AUTO_DISMISS_MS)
          : Math.max(autoDismissMs, COMPLETED_AUTO_DISMISS_MS);
      return base + exitAnimationMs + TERMINAL_FALLBACK_GRACE_MS;
    },
    [autoDismissMs, exitAnimationMs]
  );

  const hasActiveTimers = useMemo(() => {
    const hasRunningTasks = tasks.some(
      (t) =>
        (t.status === "running" || t.status === "queued") &&
        !dismissedTaskIds.has(t.task_id) &&
        !autoDismissedPhases.has(`task:${t.task_id}:running`)
    );
    const hasVisibleTerminalTasks = tasks.some((t) => {
      if (t.status !== "completed" && t.status !== "failed") return false;
      if (dismissedTaskIds.has(t.task_id)) return false;
      if (autoDismissedPhases.has(`task:${t.task_id}:${t.status}`)) return false;
      const finishedAt = completedTaskAtRef.current.get(t.task_id);
      if (finishedAt == null) return false;
      if (pausedCardIds.has(`task:${t.task_id}`)) return true;
      return nowMs - finishedAt < terminalFallbackTtl(t.status);
    });
    const hasRunningPrep = pendingPreparations.some(
      (p) =>
        p.status === "preparing" &&
        !dismissedPrepIds.has(p.id) &&
        !autoDismissedPhases.has(`prep:${p.id}:running`)
    );
    const hasRunningLong = longWorks.some(
      (w) => w.status === "running" && !autoDismissedPhases.has(`lw:${w.id}:running`)
    );
    const hasVisibleTerminalLong = longWorks.some((w) => {
      if (w.status !== "completed" && w.status !== "failed") return false;
      if (autoDismissedPhases.has(`lw:${w.id}:${w.status}`)) return false;
      const finishedAt = w.finishedAtMs ?? w.updatedAtMs;
      if (pausedCardIds.has(`lw:${w.id}`)) return true;
      return nowMs - finishedAt < terminalFallbackTtl(w.status);
    });
    return (
      hasRunningTasks ||
      hasVisibleTerminalTasks ||
      hasRunningPrep ||
      hasRunningLong ||
      hasVisibleTerminalLong
    );
  }, [
    tasks,
    pendingPreparations,
    longWorks,
    dismissedTaskIds,
    dismissedPrepIds,
    autoDismissedPhases,
    pausedCardIds,
    nowMs,
    terminalFallbackTtl,
  ]);

  useEffect(() => {
    if (!hasActiveTimers) return;
    const id = window.setInterval(() => {
      setNowMs(Date.now());
    }, 500);
    return () => {
      window.clearInterval(id);
    };
  }, [hasActiveTimers]);

  useEffect(() => {
    for (const item of longWorks) {
      if (item.status === "cancelled") {
        dismissLongWork(item.id);
        continue;
      }
      if (item.status === "completed" || item.status === "failed") {
        if (pausedCardIds.has(`lw:${item.id}`)) continue;
        const finishedAt = item.finishedAtMs ?? item.updatedAtMs;
        if (nowMs - finishedAt >= terminalFallbackTtl(item.status)) {
          dismissLongWork(item.id);
        }
      }
    }
  }, [longWorks, nowMs, pausedCardIds, terminalFallbackTtl]);

  const cards = useMemo<ProjectedCard[]>(() => {
    if (suppressAll) return [];
    const next: ProjectedCard[] = [];

    for (const task of suppressDownloads ? [] : tasks) {
      if (dismissedTaskIds.has(task.task_id)) continue;
      if ((task.status === "completed" || task.status === "failed") && readTaskIds?.has(task.task_id)) {
        continue;
      }
      const isRunning = task.status === "running" || task.status === "queued";
      const cardId = `task:${task.task_id}`;
      const cardStatus: ProjectedCard["status"] =
        task.status === "completed"
          ? "completed"
          : task.status === "failed"
            ? "failed"
            : "running";
      const phaseKey = `${cardId}:${cardStatus}`;
      if (autoDismissedPhases.has(phaseKey)) continue;

      const completedAt = completedTaskAtRef.current.get(task.task_id);
      const justFinished =
        (task.status === "completed" || task.status === "failed") &&
        completedAt != null &&
        (pausedCardIds.has(cardId) || nowMs - completedAt < terminalFallbackTtl(task.status));
      if (!isRunning && !justFinished) continue;

      const parsedCreated = task.created_at ? Date.parse(task.created_at) : NaN;
      const fallbackStart = Number.isFinite(parsedCreated) ? Math.min(parsedCreated, nowMs) : nowMs;
      const startedAt = seenTaskStartedRef.current.get(task.task_id) ?? fallbackStart;
      if (isRunning && nowMs - startedAt < LONG_WORK_ENTRY_DELAY_MS) continue;

      const hasBytesTotal =
        typeof task.total_bytes === "number" &&
        task.total_bytes > 0 &&
        task.bytes_reported !== false;
      const estimatedBytes =
        !hasBytesTotal && task.bytes_reported !== false
          ? lookupSubsetSizeEstimate(task.task_id, task.scene_ids)
          : null;
      const hasEstimatedBytes = typeof estimatedBytes === "number" && estimatedBytes > 0;
      const hasItemTotal = typeof task.items_count === "number" && task.items_count > 0;
      const itemsDone = task.completed_items_count ?? 0;

      const percent = hasBytesTotal
        ? Math.min(100, Math.round((task.done_bytes / task.total_bytes!) * 100))
        : hasEstimatedBytes
          ? Math.min(99, Math.round((task.done_bytes / estimatedBytes!) * 100))
          : hasItemTotal
            ? Math.min(100, Math.round((itemsDone / task.items_count!) * 100))
            : null;

      const eta =
        task.status === "running"
          ? hasBytesTotal
            ? estimateRemainingEta({
                startedAtMs: startedAt,
                nowMs,
                completed: task.done_bytes,
                total: task.total_bytes!,
                ratePerSecond: task.bytes_per_second ?? null,
              })
            : hasEstimatedBytes
              ? estimateRemainingEta({
                  startedAtMs: startedAt,
                  nowMs,
                  completed: task.done_bytes,
                  total: estimatedBytes!,
                  ratePerSecond: task.bytes_per_second ?? null,
                })
              : hasItemTotal
                ? estimateRemainingEta({
                    startedAtMs: startedAt,
                    nowMs,
                    completed: itemsDone,
                    total: task.items_count!,
                  })
                : { kind: "hidden" as const, secondsLeft: null, textZh: "" }
          : { kind: "hidden" as const, secondsLeft: null, textZh: "" };

      const rawTaskError =
        typeof task.error === "string" ? task.error : task.error?.message_zh;
      const cleanTaskError = stripInternalDiagnosticSuffix(rawTaskError) || rawTaskError;
      const detailLine =
        task.status === "completed"
          ? "已完成 · 点击查看下载详情"
          : task.status === "failed"
            ? cleanTaskError || "任务失败 · 点击查看详情"
            : task.status === "queued"
              ? hasEstimatedBytes
                ? `等待队列调度 · 预计 ~${formatBytes(estimatedBytes!)}`
                : "等待队列调度…"
              : percent !== null
                ? `${percent}%${hasEstimatedBytes && !hasBytesTotal ? ` · 预计 ~${formatBytes(estimatedBytes!)}` : ""}${task.speed_formatted ? ` · ${task.speed_formatted}` : ""}`
                : "正在准备传输通道…";

      const reassureLine =
        isRunning && percent === null && nowMs - startedAt >= LONG_WORK_REASSURE_MS
          ? "耗时较平时略长，仍在处理中…"
          : "";

      next.push({
        id: cardId,
        phaseKey,
        kind: "task",
        category: "transfer",
        title: task.name,
        status: cardStatus,
        percent,
        detailLine,
        etaLine: eta.textZh,
        reassureLine,
        onCancel:
          isRunning && onCancelTask
            ? () => onCancelTask(task)
            : undefined,
        onDismiss: () => {
          setDismissedTaskIds((prev) => {
            const copy = new Set(prev);
            copy.add(task.task_id);
            return copy;
          });
          onDismissTaskNotification?.(task.task_id);
        },
        onOpen: onOpenTask
          ? () => {
              onDismissTaskNotification?.(task.task_id);
              onOpenTask(task.task_id);
            }
          : onOpenDownloads,
      });
    }

    for (const prep of suppressDownloads ? [] : pendingPreparations) {
      if (dismissedPrepIds.has(prep.id)) continue;
      const cardId = `prep:${prep.id}`;
      const cardStatus: ProjectedCard["status"] = prep.status === "failed" ? "failed" : "running";
      const phaseKey = `${cardId}:${cardStatus}`;
      if (autoDismissedPhases.has(phaseKey)) continue;

      const startedAt = prepStartedAtRef.current.get(prep.id) ?? nowMs;
      if (prep.status === "preparing" && nowMs - startedAt < LONG_WORK_ENTRY_DELAY_MS) {
        continue;
      }
      next.push({
        id: cardId,
        phaseKey,
        kind: "prep",
        category: "transfer",
        title: prep.title,
        status: cardStatus,
        percent: null,
        detailLine:
          prep.status === "failed"
            ? prep.error || "任务创建失败"
            : `正在提交 ${prep.count} 个文件至下载队列…`,
        etaLine: "",
        reassureLine:
          prep.status === "preparing" && nowMs - startedAt >= LONG_WORK_REASSURE_MS
            ? "耗时较平时略长，仍在提交任务…"
            : "",
        onDismiss: () => {
          setDismissedPrepIds((prev) => {
            const copy = new Set(prev);
            copy.add(prep.id);
            return copy;
          });
          if (prep.status === "failed") {
            onDismissPreparation?.(prep.id);
          }
        },
        onOpen: onOpenDownloads,
      });
    }

    for (const item of longWorks) {
      if (item.status === "cancelled") {
        continue;
      }
      const cardId = `lw:${item.id}`;
      const cardStatus: ProjectedCard["status"] =
        item.status === "completed"
          ? "completed"
          : item.status === "failed"
            ? "failed"
            : "running";
      const phaseKey = `${cardId}:${cardStatus}`;
      if (autoDismissedPhases.has(phaseKey)) continue;

      if (item.status === "running" && nowMs - item.startedAtMs < LONG_WORK_ENTRY_DELAY_MS) {
        continue;
      }
      if (item.status === "completed" || item.status === "failed") {
        const finishedAt = item.finishedAtMs ?? item.updatedAtMs;
        if (!pausedCardIds.has(cardId) && nowMs - finishedAt >= terminalFallbackTtl(item.status)) {
          continue;
        }
      }
      const hasCount = item.total != null && item.total > 0;
      const percent = hasCount
        ? Math.min(100, Math.round((item.current / item.total!) * 100))
        : null;
      const eta =
        item.status === "running" && hasCount
          ? estimateRemainingEta({
              startedAtMs: item.startedAtMs,
              nowMs,
              completed: item.current,
              total: item.total!,
            })
          : { kind: "hidden" as const, secondsLeft: null, textZh: "" };

      const detailLine =
        item.status === "completed"
          ? item.doneSubtitle || "已完成 · 点击查看"
          : item.status === "failed"
            ? item.errorMessage || "处理失败"
            : hasCount
              ? `${item.current}/${item.total}${item.unitLabel ? ` ${item.unitLabel}` : ""} · ${percent}%`
              : item.stageLabel || "处理中…";

      const reassureLine =
        item.status === "running" && !hasCount && nowMs - item.startedAtMs >= LONG_WORK_REASSURE_MS
          ? "耗时较平时略长，仍在处理中…"
          : "";

      next.push({
        id: cardId,
        phaseKey,
        kind: "longwork",
        category: item.category === "export" ? "export" : item.category === "plugin" ? "plugin" : "transfer",
        title: item.status === "completed" && item.doneTitle ? item.doneTitle : item.title,
        status: cardStatus,
        percent,
        detailLine,
        etaLine: eta.textZh,
        reassureLine,
        onCancel: item.status === "running" ? item.onCancel : undefined,
        onDismiss: () => dismissLongWork(item.id),
        onAutoDismissCleanup:
          cardStatus === "completed" || cardStatus === "failed"
            ? () => dismissLongWork(item.id)
            : undefined,
        onOpen: item.onOpen,
      });
    }

    // Keep at most 3 concurrent floating progress notifications.
    return next.slice(0, 3);
  }, [
    tasks,
    pendingPreparations,
    suppressDownloads,
    suppressAll,
    readTaskIds,
    longWorks,
    nowMs,
    dismissedTaskIds,
    dismissedPrepIds,
    autoDismissedPhases,
    pausedCardIds,
    terminalFallbackTtl,
    onDismissTaskNotification,
    onCancelTask,
    onDismissPreparation,
    onOpenTask,
    onOpenDownloads,
  ]);

  const pauseCardTimer = useCallback(
    (cardId: string) => {
      setPausedCardIds((prev) => {
        if (prev.has(cardId)) return prev;
        const copy = new Set(prev);
        copy.add(cardId);
        return copy;
      });
      clearCardExitTimer(cardId);
      setExitingCardIds((prev) => {
        if (!prev.has(cardId)) return prev;
        const copy = new Set(prev);
        copy.delete(cardId);
        return copy;
      });
      const entry = cardTimersRef.current.get(cardId);
      if (entry && entry.timerId !== null) {
        window.clearTimeout(entry.timerId);
        const elapsed = entry.startedAtMs !== null ? Date.now() - entry.startedAtMs : 0;
        entry.remainingMs = Math.max(250, entry.remainingMs - elapsed);
        entry.timerId = null;
        entry.startedAtMs = null;
      }
    },
    [clearCardExitTimer]
  );

  const resumeCardTimer = useCallback((cardId: string) => {
    setPausedCardIds((prev) => {
      if (!prev.has(cardId)) return prev;
      const copy = new Set(prev);
      copy.delete(cardId);
      return copy;
    });
  }, []);

  useEffect(() => {
    const activeMap = new Map(cards.map((c) => [c.id, c]));

    for (const [cardId, entry] of cardTimersRef.current.entries()) {
      if (!activeMap.has(cardId)) {
        if (entry.timerId !== null) {
          window.clearTimeout(entry.timerId);
        }
        cardTimersRef.current.delete(cardId);
        clearCardExitTimer(cardId);
      }
    }

    setPausedCardIds((prev) => {
      let changed = false;
      for (const id of prev) {
        if (!activeMap.has(id)) {
          changed = true;
          break;
        }
      }
      if (!changed) return prev;
      const copy = new Set<string>();
      for (const id of prev) {
        if (activeMap.has(id)) copy.add(id);
      }
      return copy;
    });

    if (autoDismissMs <= 0) return;

    for (const card of cards) {
      const existing = cardTimersRef.current.get(card.id);
      const phaseChanged = !existing || existing.phaseKey !== card.phaseKey;
      if (phaseChanged) {
        if (existing && existing.timerId !== null) {
          window.clearTimeout(existing.timerId);
        }
        clearCardExitTimer(card.id);
        setExitingCardIds((prev) => {
          if (!prev.has(card.id)) return prev;
          const copy = new Set(prev);
          copy.delete(card.id);
          return copy;
        });
        cardTimersRef.current.set(card.id, {
          phaseKey: card.phaseKey,
          remainingMs: autoDismissMs,
          startedAtMs: null,
          timerId: null,
        });
      }

      const entry = cardTimersRef.current.get(card.id)!;
      if (pausedCardIds.has(card.id) || exitingCardIds.has(card.id)) {
        continue;
      }
      if (entry.timerId === null) {
        entry.startedAtMs = Date.now();
        const targetPhase = card.phaseKey;
        const onCleanup = card.onAutoDismissCleanup;
        entry.timerId = window.setTimeout(() => {
          const current = cardTimersRef.current.get(card.id);
          if (!current || current.phaseKey !== targetPhase) return;
          current.timerId = null;
          current.startedAtMs = null;
          if (pausedCardIdsRef.current.has(card.id)) return;
          triggerSmoothExit(card.id, () => {
            setAutoDismissedPhases((prev) => {
              if (prev.has(targetPhase)) return prev;
              const copy = new Set(prev);
              copy.add(targetPhase);
              return copy;
            });
            onCleanup?.();
          });
        }, entry.remainingMs);
      }
    }
  }, [
    cards,
    autoDismissMs,
    pausedCardIds,
    exitingCardIds,
    clearCardExitTimer,
    triggerSmoothExit,
  ]);

  if (cards.length === 0) return null;

  return (
    <div
      data-testid="progress-notification-host"
      aria-live="polite"
      className="fixed bottom-4 right-4 z-[2550] w-[344px] max-w-[calc(100vw-2rem)] flex flex-col-reverse gap-2 pointer-events-none"
    >
      {cards.map((card) => {
        const CategoryIcon =
          card.category === "export"
            ? FolderArchive
            : card.category === "plugin"
              ? Package
              : ArrowDownCircle;

        const isExiting = exitingCardIds.has(card.id);
        const cardDragOffset = !isExiting && dragState?.id === card.id ? dragState.offsetX : 0;

        return (
          <div
            key={card.id}
            data-testid="progress-notification-card"
            data-notification-status={card.status}
            data-exiting={isExiting ? "true" : "false"}
            onMouseEnter={() => pauseCardTimer(card.id)}
            onMouseLeave={(e) => {
              if (!e.currentTarget.contains(document.activeElement)) {
                resumeCardTimer(card.id);
              }
            }}
            onFocus={() => pauseCardTimer(card.id)}
            onBlur={(e) => {
              if (!e.currentTarget.contains(e.relatedTarget as Node | null)) {
                resumeCardTimer(card.id);
              }
            }}
            onPointerDown={(e) => {
              const target = e.target as HTMLElement | null;
              if (target?.closest("button")) return;
              setDragState({ id: card.id, startX: e.clientX, offsetX: 0 });
            }}
            onPointerMove={(e) => {
              if (!dragState || dragState.id !== card.id) return;
              const dx = Math.max(0, e.clientX - dragState.startX);
              if (dx > 0) {
                setDragState((prev) => (prev && prev.id === card.id ? { ...prev, offsetX: dx } : prev));
              }
            }}
            onPointerUp={(e) => {
              if (!dragState || dragState.id !== card.id) return;
              const dx = Math.max(dragState.offsetX, e.clientX - dragState.startX);
              setDragState(null);
              if (dx >= 60) {
                clearCardAutoTimer(card.id);
                clearCardExitTimer(card.id);
                card.onDismiss?.();
              }
            }}
            onPointerCancel={() => {
              if (dragState?.id === card.id) setDragState(null);
            }}
            onWheel={(e) => {
              // Touchpad two-finger swipe right in macOS/WebKit produces negative deltaX
              if (e.deltaX <= -50 && Math.abs(e.deltaX) > Math.abs(e.deltaY)) {
                triggerSmoothExit(card.id, () => card.onDismiss?.());
              }
            }}
            style={
              cardDragOffset > 0
                ? {
                    transform: `translateX(${cardDragOffset}px)`,
                    opacity: Math.max(0.2, 1 - cardDragOffset / 180),
                    transition: "none",
                  }
                : undefined
            }
            className={cn(
              isExiting ? "sp-progress-notif-exit" : "sp-progress-notif-enter",
              "sp-progress-notif-glass group pointer-events-auto relative flex items-start gap-2.5 rounded-[14px] border border-sp-line/70 bg-sp-surface/80 dark:bg-sp-surface/75 backdrop-blur-md p-3 shadow-lg select-none touch-pan-y"
            )}
          >
            <div
              className={cn(
                "w-8 h-8 rounded-[8px] flex items-center justify-center shrink-0 border",
                card.status === "completed"
                  ? "bg-sp-success-bg/85 border-sp-success-text/20 text-sp-success-text"
                  : card.status === "failed"
                    ? "bg-sp-danger-bg/85 border-sp-danger-text/25 text-sp-danger-text"
                    : "bg-sp-surface-muted/80 border-sp-line/70 text-sp-text-secondary"
              )}
            >
              {card.status === "completed" ? (
                <CheckCircle2 className="h-4 w-4 sp-check-morph-in" />
              ) : card.status === "failed" ? (
                <AlertCircle className="h-4 w-4" />
              ) : (
                <CategoryIcon className="h-4 w-4" />
              )}
            </div>

            <div className="min-w-0 flex-1 space-y-1">
              <div className="flex items-start justify-between gap-2">
                {card.onOpen ? (
                  <button
                    type="button"
                    onClick={card.onOpen}
                    className="text-left text-xs font-semibold text-sp-text hover:underline truncate cursor-pointer min-w-0 flex-1"
                    title={card.title}
                  >
                    {card.title}
                  </button>
                ) : (
                  <div className="text-xs font-semibold text-sp-text truncate flex-1" title={card.title}>
                    {card.title}
                  </div>
                )}

                <div className="flex items-center gap-1 shrink-0">
                  {card.status === "running" && card.onCancel && (
                    <button
                      type="button"
                      onClick={card.onCancel}
                      className="text-[12px] font-medium text-sp-text-muted hover:text-sp-text px-1.5 py-0.5 rounded hover:bg-sp-hover cursor-pointer transition-colors"
                    >
                      取消
                    </button>
                  )}
                  {card.onDismiss && (
                    <button
                      type="button"
                      onClick={() => triggerSmoothExit(card.id, () => card.onDismiss?.())}
                      aria-label="关闭进度通知"
                      className={cn(
                        "h-5 w-5 rounded flex items-center justify-center text-sp-text-muted hover:text-sp-text hover:bg-sp-hover cursor-pointer transition-opacity",
                        card.status === "running"
                          ? "opacity-70 group-hover:opacity-100 focus:opacity-100"
                          : "opacity-100"
                      )}
                    >
                      <X className="h-3.5 w-3.5" />
                    </button>
                  )}
                </div>
              </div>

              <div className="flex items-center justify-between gap-2 text-[12px] text-sp-text-muted">
                <span className={cn("truncate", card.status === "failed" && "text-sp-danger-text")}>
                  {card.detailLine}
                </span>
                {card.etaLine && (
                  <span className="shrink-0 font-mono text-[12px] text-sp-text-secondary">
                    {card.etaLine}
                  </span>
                )}
              </div>

              {card.reassureLine && (
                <div className="text-[12px] text-sp-text-muted">{card.reassureLine}</div>
              )}

              {card.status === "running" && (
                <div className="h-1.5 w-full rounded-full bg-sp-surface-muted/80 overflow-hidden mt-1">
                  {card.percent !== null ? (
                    <div
                      className="h-full bg-sp-accent-fill rounded-full transition-[width] duration-200 ease-out"
                      style={{ width: `${card.percent}%` }}
                    />
                  ) : (
                    <div className="h-full w-1/3 bg-sp-accent-fill rounded-full progress-indeterminate-bar" />
                  )}
                </div>
              )}
            </div>
          </div>
        );
      })}
    </div>
  );
}
