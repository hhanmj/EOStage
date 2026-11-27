import React, { useState, useEffect, useCallback, useMemo, useRef } from "react";
import {
  X,
  Download,
  Play,
  Pause,
  RotateCcw,
  RefreshCw,
  FolderOpen,
  CheckCircle2,
  AlertCircle,
  Clock,
  ArrowDownCircle,
  Loader2,
  Trash2,
  Info,
  ChevronRight,
  ArrowLeft,
  XCircle,
  HardDrive,
  Copy,
  Maximize2,
} from "lucide-react";
import { useI18n } from "@/i18n";
import { cn } from "@/lib/utils";
import type {
  AccountId,
  ApplicationTask,
  TaskLifecycleStatus,
  TaskSceneItem,
  UnifiedTaskItemAction,
} from "@/shell/api/types";
import {
  partitionTasks,
  getAvailableTaskActions,
  isGacosWebformTask,
  isTaskFallbackEligible,
  isTaskHistoryRow,
  sortTasksByPinAndCreatedAt,
  formatTaskTimestamp,
  taskOutputDir,
  RETRY_FALLBACK_DIALOG_TEXT,
  CLEAR_HISTORY_DIALOG_TEXT,
} from "@/shell/api/taskLogic";
import { getDesktopShellBridge } from "@/lib/desktopShellBridge";
import { updateApplicationDownloadTask } from "@/lib/bridge";
import { formatBytes, truncateMiddle } from "@/shell/api/rowProjection";
import { failureReasonLine, taskCardSizeLine, pauseReasonLine } from "@/shell/api/taskCardLogic";
import { explainActionError } from "@/shell/api/errorExplanations";
import { resolveAccountActionFromError } from "@/shell/api/opticalCreateError";
import { openTaskOutputErrorText, resolveTaskActionErrorText } from "@/shell/api/taskActionFlow";
import { resolveDownloadDrawerEscapeAction } from "@/shell/api/drawerEscapeLogic";
import { isSettingsModalOpen } from "@/shell/api/settingsModalLayer";
import {
  getTaskFallbackRequestGate,
  markTaskFallbackRequestUncertain,
  reconcileTaskFallbackRequestSnapshot,
  settleTaskFallbackRequest,
  startTaskFallbackRequest,
  subscribeTaskFallbackRequestGate,
  clearTaskFallbackRequest,
  type TaskFallbackRequestGate,
} from "@/shell/api/taskFallbackRequestGate";
import {
  getTasks,
  taskAction,
  retryTaskFallback,
  clearTaskHistory,
  openOutputDirectory,
  isFixtureBackend,
  createTaskActionRunner,
} from "@/shell/api";
import { EmptyState } from "@/components/ui/EmptyState";
import { EmptyStateIcon } from "@/components/ui/EmptyStateVisuals";
import { useTaskEta } from "@/hooks/useLoadingPrimitives";
import {
  effectiveGroupStatus,
  groupOrbitCompanionTasks,
  isOrbitCompanion,
} from "@/shell/api/orbitChildRows";
import { TaskArtifactsVerificationPanel } from "./TaskArtifactsVerificationPanel.tsx";
import { orbitItemAction } from "@/shell/api/orbitItemAction";
import type { OrbitRowAction } from "@/shell/api/orbitChildRows";
import { DownloadFilterBar, taskDataTypeKey } from "./DownloadFilterBar.tsx";
import {
  normalizeTaskDisplayName,
  normalizeTaskProviderLabel,
  taskDataTypeLabel,
  taskMatchesStatusFilter,
  type DownloadStatusFilter,
} from "./downloadFilterLogic.ts";
import { DownloadTaskRow } from "./DownloadTaskRow.tsx";
import { TaskSourceBatchPanel } from "@/components/TaskSourceBatchPanel";
import type { DownloadLayout } from "@/lib/generated/v3-contracts";

export interface DownloadTaskItem {
  id: string;
  name: string;
  provider: string;
  dataset: string;
  status: "running" | "paused" | "completed" | "failed" | "queued";
  doneBytes: number;
  totalBytes: number | null;
  speed: string;
  outputDir?: string;
  error?: string | null;
}

/** 「打开目录」交给上层的任务引用：上层按任务保存目录命令打开（不传本机路径）。 */
export type TaskOutputRef = { task_id: string; revision: number };

export interface DesktopPendingDownloadPreparation {
  id: string;
  title: string;
  count: number;
  preparedCount?: number;
  phase?: "preparing" | "committing";
  outputRoot: string;
  status: "preparing" | "failed";
  error?: string | null;
  actionAccountId?: AccountId | null;
  actionLabel?: string | null;
}

export interface DesktopDownloadDrawerProps {
  open: boolean;
  onClose?: () => void;
  // 外部可透传或由内部从 C1 API 自动加载
  tasks?: ApplicationTask[];
  onOpenFolder?: (task: TaskOutputRef) => void | Promise<void>;
  onTaskAction?: (
    taskId: string,
    revision: number,
    action: "pause" | "resume" | "retry" | "cancel"
  ) => Promise<void>;
  // 用于 harness 演示模式注入场景参数
  harnessData?: ApplicationTask[];
  variant?: "drawer" | "page";
  /** 打开时直接进入该任务详情；nonce 变化即为一次新请求 */
  focusTaskRequest?: { taskId: string; nonce: number } | null;
  onHistoricalMutationApplied?: () => void | Promise<void>;
  newTaskPolicy?: { concurrency: number; layout: DownloadLayout } | null;
  pendingPreparations?: readonly DesktopPendingDownloadPreparation[];
  onDismissPreparation?: (id: string) => void;
  onConfigureAccount?: (accountId?: AccountId) => void;
  /** 抽屉右上角「进入完整下载中心」入口；未提供时不渲染。 */
  onOpenFullPage?: () => void;
}

function getOptionalApplicationBridge() {
  try {
    return getDesktopShellBridge()?.application ?? null;
  } catch {
    return null;
  }
}

function getOptionalUnifiedBridge() {
  return getOptionalApplicationBridge()?.unified ?? null;
}

/**
 * 判断错误对象是否为确定性的 TypedError（携带显式非传输超时错误码），
 * 这类错误保证未在服务端发起或污染异步执行，可直接呈现明确原因而无需包裹「结果未能确认」兜底提示。
 */
function isDeterministicTypedError(err: unknown): boolean {
  if (!err || typeof err !== "object") return false;
  const code = (err as { code?: unknown }).code;
  if (typeof code !== "string" || code.trim().length === 0) return false;
  if (code.includes("TIMEOUT") || code.includes("UNKNOWN")) return false;
  return true;
}

export function DesktopDownloadDrawer({
  open,
  onClose = () => {},
  tasks: propTasks,
  onOpenFolder = openOutputDirectory,
  onTaskAction,
  harnessData,
  variant = "drawer",
  focusTaskRequest = null,
  onHistoricalMutationApplied,
  newTaskPolicy = null,
  pendingPreparations = [],
  onDismissPreparation,
  onConfigureAccount,
  onOpenFullPage,
}: DesktopDownloadDrawerProps) {
  const { t } = useI18n();
  const isPage = variant === "page";
  const [internalTasks, setInternalTasks] = useState<ApplicationTask[]>([]);
  const [selectedTaskDetail, setSelectedTaskDetail] = useState<ApplicationTask | null>(null);
  const [drawerSubview, setDrawerSubview] = useState<"tasks" | "artifacts">("tasks");

  // 对话框状态
  const [fallbackConfirmTask, setFallbackConfirmTask] = useState<ApplicationTask | null>(null);
  const [fallbackGate, setFallbackGate] = useState<TaskFallbackRequestGate | null>(null);
  const [fallbackDeterministicError, setFallbackDeterministicError] = useState<string | null>(null);
  const [fallbackEligibility, setFallbackEligibility] = useState<Record<string, boolean>>({});
  const [showClearHistoryConfirm, setShowClearHistoryConfirm] = useState(false);
  const fallbackPresentationEpochRef = useRef(0);
  const fallbackDialogTaskRef = useRef<ApplicationTask | null>(null);
  const suppressGateClearCloseRef = useRef(false);

  const fallbackLoading = fallbackGate?.pending ?? false;
  const fallbackError = fallbackGate?.error ?? null;
  const fallbackUncertain = fallbackGate?.uncertain ?? false;

  const resetFallbackState = useCallback(() => {
    fallbackPresentationEpochRef.current += 1;
    // 只关闭展示。已发出的服务端请求不能由前端撤销，gate 必须跨关闭/重开保留。
    fallbackDialogTaskRef.current = null;
    setFallbackConfirmTask(null);
    setFallbackGate(null);
    setFallbackDeterministicError(null);
  }, []);

  const openFallback = useCallback((task: ApplicationTask) => {
    fallbackPresentationEpochRef.current += 1;
    fallbackDialogTaskRef.current = task;
    setFallbackConfirmTask(task);
    setFallbackGate(getTaskFallbackRequestGate(task.task_id, task.revision));
    setFallbackDeterministicError(null);
  }, []);

  // 加载数据
  const reloadTasks = useCallback(async () => {
    if (harnessData) {
      setInternalTasks(harnessData);
      return;
    }
    if (propTasks) {
      setInternalTasks(propTasks);
      return;
    }
    try {
      const data = await getTasks();
      setInternalTasks(data);
    } catch {
      // 保持空态
    }
  }, [harnessData, propTasks]);

  useEffect(() => {
    if (open) {
      void reloadTasks();
    } else {
      setSelectedTaskDetail(null);
      resetFallbackState();
      setShowClearHistoryConfirm(false);
    }
  }, [open, reloadTasks, resetFallbackState]);

  const previousActiveElementRef = useRef<HTMLElement | null>(null);
  const prevOpenRef = useRef(open);

  const returnFocus = useCallback(() => {
    if (previousActiveElementRef.current && typeof previousActiveElementRef.current.focus === "function") {
      previousActiveElementRef.current.focus();
      previousActiveElementRef.current = null;
    }
  }, []);

  // 焦点管理：仅在 open 从 false 跳变为 true 时捕获当前 activeElement；在 open 变为 false 时归还焦点
  useEffect(() => {
    if (isPage) return;
    if (open && !prevOpenRef.current) {
      if (typeof document !== "undefined" && document.activeElement instanceof HTMLElement) {
        previousActiveElementRef.current = document.activeElement;
      }
    } else if (!open && prevOpenRef.current) {
      returnFocus();
    }
    prevOpenRef.current = open;
  }, [open, isPage, returnFocus]);

  // 组件卸载时若持有未归还焦点，执行归还
  useEffect(() => {
    return () => {
      if (!isPage) {
        returnFocus();
      }
    };
  }, [isPage, returnFocus]);

  // 键盘 Escape 支持关闭模态子对话框与抽屉本身（严格优先级：兜底确认 -> 清除历史确认 -> 任务详情 -> 抽屉本身）
  useEffect(() => {
    if (!open) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        // 偏好设置压在抽屉之上、或上层已消费该 Esc 时，抽屉不响应
        if (e.defaultPrevented || e.isComposing || isSettingsModalOpen()) return;
        const action = resolveDownloadDrawerEscapeAction({
          hasFallbackConfirm: Boolean(fallbackConfirmTask),
          hasClearHistoryConfirm: Boolean(showClearHistoryConfirm),
          hasSelectedTaskDetail: Boolean(selectedTaskDetail),
        });
        switch (action) {
          case "close_fallback":
            resetFallbackState();
            break;
          case "close_clear_history":
            setShowClearHistoryConfirm(false);
            break;
          case "close_task_detail":
            setSelectedTaskDetail(null);
            break;
          case "close_drawer":
            if (!isPage) {
              onClose();
            }
            break;
        }
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [
    open,
    fallbackConfirmTask,
    showClearHistoryConfirm,
    selectedTaskDetail,
    resetFallbackState,
    onClose,
  ]);

  const allTasks = harnessData || propTasks || internalTasks;

  const appliedFocusNonceRef = useRef<number | null>(null);
  useEffect(() => {
    if (!open || !focusTaskRequest || appliedFocusNonceRef.current === focusTaskRequest.nonce) return;
    const target = allTasks.find((task) => task.task_id === focusTaskRequest.taskId);
    if (!target) return;
    appliedFocusNonceRef.current = focusTaskRequest.nonce;
    setSelectedTaskDetail(target);
  }, [open, focusTaskRequest, allTasks]);
  const latestTasks = useRef(allTasks);
  latestTasks.current = allTasks;
  fallbackDialogTaskRef.current = fallbackConfirmTask;

  const fallbackTaskIsCurrent = (task = fallbackConfirmTask) => {
    if (!task) return false;
    const current = (latestTasks.current || allTasks).find((entry) => entry.task_id === task.task_id);
    return task.status === "failed" && current?.task_id === task.task_id && current.status === task.status && Number.isInteger(current.revision) && current.revision >= 0 && current.revision === task.revision;
  };

  const fallbackDiffOutcome = useMemo(() => {
    if (!fallbackConfirmTask || !fallbackGate || !fallbackGate.outcome?.needs_user_confirmation) {
      return null;
    }
    if (fallbackGate.taskId !== fallbackConfirmTask.task_id) {
      return null;
    }
    if (fallbackGate.taskRevision !== fallbackConfirmTask.revision) {
      return null;
    }

    // 第二步（pending 或 uncertain）：仅在当前快照仍是同任务同修订的失败任务时保留已确认差异。
    // 快照已变化时 gate 仍保留以防重复发送，但旧差异不得继续展示或确认。
    if (fallbackGate.step === "second") {
      const currentTask = (latestTasks.current || allTasks).find((t) => t.task_id === fallbackConfirmTask.task_id);
      if (!currentTask || currentTask.status !== "failed" || currentTask.revision !== fallbackConfirmTask.revision) {
        return null;
      }
      const outcomeTask = fallbackGate.outcome.task;
      if (
        !outcomeTask ||
        outcomeTask.task_id !== fallbackConfirmTask.task_id ||
        outcomeTask.revision !== fallbackConfirmTask.revision
      ) {
        return null;
      }
      return fallbackGate.outcome;
    }

    // 第一步：必须为已结清状态且快照未发生跨修订/状态变更（防 stale diff 呈现与确认）
    if (fallbackGate.step === "first") {
      if (fallbackGate.pending || fallbackGate.uncertain) {
        return null;
      }
      const currentTask = (latestTasks.current || allTasks).find((t) => t.task_id === fallbackConfirmTask.task_id);
      if (!currentTask || currentTask.status !== "failed" || currentTask.revision !== fallbackConfirmTask.revision) {
        return null;
      }
      const outcomeTask = fallbackGate.outcome.task;
      if (
        !outcomeTask ||
        outcomeTask.task_id !== fallbackConfirmTask.task_id ||
        outcomeTask.revision !== fallbackConfirmTask.revision
      ) {
        return null;
      }
      return fallbackGate.outcome;
    }

    return null;
  }, [fallbackConfirmTask, fallbackGate, allTasks]);

  const isStaleConfirmTask = Boolean(
    fallbackConfirmTask && !fallbackTaskIsCurrent(fallbackConfirmTask)
  );

  const displayFallbackError = fallbackError || fallbackDeterministicError || (
    isStaleConfirmTask && !fallbackLoading && !fallbackUncertain
      ? t("downloads.drawer.staleTask")
      : null
  );

  useEffect(() => {
    const taskId = fallbackConfirmTask?.task_id;
    if (!taskId) return;
    return subscribeTaskFallbackRequestGate((changedTaskId, gate) => {
      if (changedTaskId !== taskId || fallbackDialogTaskRef.current?.task_id !== changedTaskId) return;
      setFallbackGate(gate);
      if (!gate && !suppressGateClearCloseRef.current) {
        fallbackDialogTaskRef.current = null;
        setFallbackConfirmTask(null);
      }
    });
  }, [fallbackConfirmTask?.task_id]);

  const handleDeterministicFallbackError = useCallback(
    (
      requestGate: NonNullable<ReturnType<typeof startTaskFallbackRequest>>,
      detail: string,
    ) => {
      suppressGateClearCloseRef.current = true;
      try {
        clearTaskFallbackRequest(requestGate);
      } finally {
        suppressGateClearCloseRef.current = false;
      }
      setFallbackGate(null);
      setFallbackDeterministicError(detail);
    },
    [],
  );

  useEffect(() => {
    const task = fallbackDialogTaskRef.current;
    if (!task) return;
    const current = latestTasks.current.find((entry) => entry.task_id === task.task_id) ?? null;
    const gate = reconcileTaskFallbackRequestSnapshot(
      task.task_id,
      current ? { revision: current.revision, status: current.status } : null,
    );
    setFallbackGate(gate);
  }, [allTasks]);

  const fallbackHashLaunchRef = useRef<string | null>(null);
  useEffect(() => {
    if (typeof window === "undefined") return;
    if (!open) {
      fallbackHashLaunchRef.current = null;
      return;
    }
    const hash = window.location.hash;
    if (hash.includes("dialog=retry-fallback") || hash.includes("retry-fallback")) {
      if (fallbackHashLaunchRef.current === hash) return;
      const failed = allTasks.find((t) => t.status === "failed") || allTasks[0];
      if (failed) {
        fallbackHashLaunchRef.current = hash;
        openFallback(failed);
      }
    } else if (hash.includes("dialog=clear-history") || hash.includes("clear-history")) {
      setShowClearHistoryConfirm(true);
    } else if (hash.includes("scroll=history") || hash.includes("scroll=cancelled")) {
      const el = document.getElementById("download-drawer-history-section");
      if (el) el.scrollIntoView({ behavior: "instant" as any, block: "start" });
    }
  }, [open, allTasks, openFallback]);

  // 纯函数分区：队列 vs 历史
  const { queue, history } = useMemo(() => {
    return partitionTasks(allTasks);
  }, [allTasks]);

  const [typeFilter, setTypeFilter] = useState<string>("all");
  const [statusFilter, setStatusFilter] = useState<DownloadStatusFilter>("all");
  const [prioritizedTaskIds, setPrioritizedTaskIds] = useState<string[]>(() => {
    if (typeof window === "undefined") return [];
    try {
      const raw = window.localStorage.getItem("eostage.downloadPriorityTaskIds.v1");
      const parsed = raw ? JSON.parse(raw) : [];
      return Array.isArray(parsed) ? parsed.filter((id): id is string => typeof id === "string") : [];
    } catch {
      return [];
    }
  });

  const toggleTaskPriority = useCallback(
    async (taskId: string) => {
      const wasHigh = prioritizedTaskIds.includes(taskId);
      const next = wasHigh
        ? prioritizedTaskIds.filter((id) => id !== taskId)
        : [taskId, ...prioritizedTaskIds.filter((id) => id !== taskId)];
      setPrioritizedTaskIds(next);
      if (typeof window !== "undefined") {
        try {
          window.localStorage.setItem("eostage.downloadPriorityTaskIds.v1", JSON.stringify(next));
        } catch {
          // ignore storage quota errors
        }
      }
      const unified = getOptionalUnifiedBridge();
      if (unified?.taskSetPinned) {
        try {
          const persisted = await unified.taskSetPinned(taskId, !wasHigh);
          if (Array.isArray(persisted)) {
            setPrioritizedTaskIds(persisted);
            if (typeof window !== "undefined") {
              try {
                window.localStorage.setItem("eostage.downloadPriorityTaskIds.v1", JSON.stringify(persisted));
              } catch {
                // ignore storage quota errors
              }
            }
          }
        } catch {
          // 保持乐观本地置顶状态
        }
      }
      if (!wasHigh) {
        const target = allTasks.find((t) => t.task_id === taskId);
        if (target && target.status === "paused") {
          await handleResume(taskId);
        }
      }
    },
    [prioritizedTaskIds, allTasks]
  );

  // 打开抽屉/页面时从 Store 同步置顶列表，并对失败任务查询重检兜底资格
  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    const unified = getOptionalUnifiedBridge();
    if (unified?.listPinnedTasks) {
      void unified
        .listPinnedTasks()
        .then((pins) => {
          if (cancelled || !Array.isArray(pins)) return;
          setPrioritizedTaskIds(pins);
          if (typeof window !== "undefined") {
            try {
              window.localStorage.setItem("eostage.downloadPriorityTaskIds.v1", JSON.stringify(pins));
            } catch {
              // ignore storage quota errors
            }
          }
        })
        .catch(() => {});
    }
    return () => {
      cancelled = true;
    };
  }, [open]);

  const failedTaskIdsKey = useMemo(
    () =>
      allTasks
        .filter((t) => t.status === "failed")
        .map((t) => t.task_id)
        .join("|"),
    [allTasks]
  );

  useEffect(() => {
    if (!open || !failedTaskIdsKey) {
      return;
    }
    let cancelled = false;
    const unified = getOptionalUnifiedBridge();
    if (!unified?.taskRetryFallbackEligibility) return;
    const ids = failedTaskIdsKey.split("|").filter(Boolean);
    void unified
      .taskRetryFallbackEligibility(ids)
      .then((map) => {
        if (cancelled || !map || typeof map !== "object") return;
        setFallbackEligibility((prev) => ({ ...prev, ...map }));
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [open, failedTaskIdsKey]);

  const groupedAllTasks = useMemo(() => groupOrbitCompanionTasks(allTasks), [allTasks]);

  const filterAndSortTaskGroups = useCallback(
    (source: ApplicationTask[]) => {
      const grouped = groupOrbitCompanionTasks(source).filter(
        (group) =>
          (typeFilter === "all" || taskDataTypeKey(group.task) === typeFilter) &&
          taskMatchesStatusFilter(effectiveGroupStatus(group), statusFilter)
      );
      const byTaskId = new Map(grouped.map((group) => [group.task.task_id, group]));
      const orderedTasks = sortTasksByPinAndCreatedAt(
        grouped.map((group) => group.task),
        prioritizedTaskIds
      );
      return orderedTasks
        .map((task) => byTaskId.get(task.task_id))
        .filter((group): group is NonNullable<typeof group> => Boolean(group));
    },
    [typeFilter, statusFilter, prioritizedTaskIds]
  );

  const queueRows = useMemo(
    () => filterAndSortTaskGroups(queue),
    [filterAndSortTaskGroups, queue]
  );

  const historyRows = useMemo(
    () => filterAndSortTaskGroups(history),
    [filterAndSortTaskGroups, history]
  );
  const [orbitAnnouncement, setOrbitAnnouncement] = useState("");

  const actionRunner = useMemo(
    () =>
      createTaskActionRunner({
        taskAction: async (taskId, revision, action) => {
          if (
            onTaskAction &&
            (action === "pause" || action === "resume" || action === "retry" || action === "cancel")
          ) {
            await onTaskAction(taskId, revision, action);
            return;
          }
          await taskAction(taskId, revision, action);
        },
        fetchTasks: getTasks,
      }),
    [onTaskAction]
  );

  // 任务动作触发：统一经 taskActionRunner 执行（使用最新修订号，遇冲突自动刷新重试一次）
  const handlePause = async (taskId: string) => {
    const task = allTasks.find((t) => t.task_id === taskId);
    await actionRunner.run(taskId, "pause", task?.revision);
    await reloadTasks();
    await onHistoricalMutationApplied?.();
  };

  const handleResume = async (taskId: string) => {
    const task = allTasks.find((t) => t.task_id === taskId);
    await actionRunner.run(taskId, "resume", task?.revision);
    await reloadTasks();
    await onHistoricalMutationApplied?.();
  };

  const handleRetry = async (taskId: string) => {
    const task = allTasks.find((t) => t.task_id === taskId);
    await actionRunner.run(taskId, "retry", task?.revision);
    await reloadTasks();
    await onHistoricalMutationApplied?.();
  };

  const handleCancel = async (taskId: string) => {
    const task = allTasks.find((t) => t.task_id === taskId);
    await actionRunner.run(taskId, "cancel", task?.revision);
    await reloadTasks();
    await onHistoricalMutationApplied?.();
  };

  const handleOrbitAction = async (task: ApplicationTask, item: Parameters<typeof orbitItemAction>[1], action: OrbitRowAction) => {
    await orbitItemAction(task, item, action);
    await reloadTasks();
    await onHistoricalMutationApplied?.();
  };

  const handleItemAction = async (
    task: ApplicationTask,
    item: TaskSceneItem,
    action: UnifiedTaskItemAction
  ) => {
    if (!item.item_id) throw new Error(t("downloads.drawer.missingItemIdError"));
    const unified = getOptionalUnifiedBridge();
    if (unified?.taskItemAction) {
      await unified.taskItemAction(task.task_id, task.revision, item.item_id, action);
    } else {
      const kind =
        action === "pause"
          ? "pause_items"
          : action === "resume"
            ? "resume_items"
            : action === "cancel"
              ? "cancel_items"
              : "retry_items";
      const result = await updateApplicationDownloadTask({
        task_id: task.task_id,
        expected_revision: task.revision,
        command: { kind, item_ids: [item.item_id] },
        event_id: `scene-item-${action}-${globalThis.crypto.randomUUID()}`,
        occurred_at: new Date().toISOString(),
      });
      if (!result.ok) {
        throw Object.assign(new Error(result.error), {
          code: result.code,
          message_zh: result.error,
        });
      }
    }
    await reloadTasks();
    await onHistoricalMutationApplied?.();
  };

  const handleDeleteHistoryTask = async (taskId: string) => {
    const appBridge = getOptionalApplicationBridge();
    if (appBridge?.unified?.taskDeleteHistory) {
      await appBridge.unified.taskDeleteHistory(taskId);
    } else if (appBridge?.deleteDownloadHistory) {
      await appBridge.deleteDownloadHistory({
        task_id: taskId,
        deleted_at: new Date().toISOString(),
      });
    } else {
      throw new Error(t("downloads.drawer.deleteHistoryItemUnsupportedError"));
    }
    if (selectedTaskDetail?.task_id === taskId) {
      setSelectedTaskDetail(null);
    }
    setInternalTasks((prev) => prev.filter((t) => t.task_id !== taskId));
    await reloadTasks();
    await onHistoricalMutationApplied?.();
  };

  const handleFirstStepRetryFallback = async () => {
    if (!fallbackConfirmTask || fallbackLoading || fallbackUncertain) return;
    const requestTask = fallbackConfirmTask;
    if (!fallbackTaskIsCurrent(requestTask)) return;
    setFallbackDeterministicError(null);
    const startedEpoch = fallbackPresentationEpochRef.current;
    const requestGate = startTaskFallbackRequest({
      taskId: requestTask.task_id,
      taskRevision: requestTask.revision,
      step: "first",
    });
    if (!requestGate) return;
    setFallbackGate(requestGate);
    try {
      const outcome = await retryTaskFallback(requestTask.task_id, null);
      const snapshotEntry = latestTasks.current.find((t) => t.task_id === requestTask.task_id) ?? null;
      const snapshotMatchesAnchor = snapshotEntry !== null
        && snapshotEntry.revision === requestTask.revision
        && snapshotEntry.status === "failed";
      const dialogShowsThisTask = fallbackDialogTaskRef.current?.task_id === requestTask.task_id;
      const presentationReopened = dialogShowsThisTask && fallbackPresentationEpochRef.current !== startedEpoch;

      const validBatch = typeof outcome?.refreshed_batch_id === "string" && Boolean(outcome.refreshed_batch_id.trim());
      const validTask = outcome?.task?.task_id === requestTask.task_id && outcome.task.created === false &&
        Number.isInteger(outcome.task.revision) && outcome.task.revision >= requestTask.revision &&
        (outcome.needs_user_confirmation
          ? outcome.task.status === "failed" && outcome.task.revision === requestTask.revision
          : outcome.task.status !== "failed" && outcome.task.revision > requestTask.revision);
      const contractValid = Boolean(outcome && validBatch && validTask && typeof outcome.needs_user_confirmation === "boolean");

      const message = t("downloads.fallback.taskMismatch");

      if (!contractValid || !snapshotMatchesAnchor) {
        markTaskFallbackRequestUncertain(requestGate, message, { dropOutcome: false });
        return;
      }

      if (outcome.needs_user_confirmation === true) {
        settleTaskFallbackRequest(requestGate, outcome);
        return;
      }

      if (presentationReopened) {
        markTaskFallbackRequestUncertain(requestGate, message, { dropOutcome: true });
        return;
      }

      settleTaskFallbackRequest(requestGate, outcome);
      clearTaskFallbackRequest(requestGate);
      await reloadTasks();
    } catch (err: any) {
      const detail = err?.message_zh || err?.message || t("downloads.fallback.retryFailed");
      if (isDeterministicTypedError(err)) {
        handleDeterministicFallbackError(requestGate, detail);
        return;
      }
      const message = t("downloads.fallback.uncertainDetail", { detail });
      markTaskFallbackRequestUncertain(requestGate, message);
    }
  };

  const handleSecondStepConfirm = async () => {
    if (!fallbackConfirmTask || !fallbackDiffOutcome || fallbackLoading || fallbackUncertain) return;
    const requestTask = fallbackConfirmTask;
    const requestedBatchId = fallbackDiffOutcome.refreshed_batch_id;
    if (!fallbackTaskIsCurrent(requestTask)) return;
    setFallbackDeterministicError(null);
    const startedEpoch = fallbackPresentationEpochRef.current;
    const requestGate = startTaskFallbackRequest({
      taskId: requestTask.task_id,
      taskRevision: requestTask.revision,
      step: "second",
      outcome: fallbackDiffOutcome,
    });
    if (!requestGate) return;
    setFallbackGate(requestGate);
    try {
      const outcome = await retryTaskFallback(requestTask.task_id, requestedBatchId);
      const snapshotEntry = latestTasks.current.find((t) => t.task_id === requestTask.task_id) ?? null;
      const snapshotMatchesAnchor = snapshotEntry !== null
        && snapshotEntry.revision === requestTask.revision
        && snapshotEntry.status === "failed";
      const dialogShowsThisTask = fallbackDialogTaskRef.current?.task_id === requestTask.task_id;
      const presentationReopened = dialogShowsThisTask && fallbackPresentationEpochRef.current !== startedEpoch;

      const validTask = outcome?.task?.task_id === requestTask.task_id && outcome.task.created === false &&
        outcome.needs_user_confirmation === false && outcome.refreshed_batch_id === requestedBatchId &&
        Number.isInteger(outcome.task.revision) && outcome.task.revision > requestTask.revision &&
        outcome.task.status !== "failed";
      const contractValid = Boolean(validTask);

      const message = t("downloads.fallback.confirmationMismatch");

      if (!contractValid || !snapshotMatchesAnchor) {
        markTaskFallbackRequestUncertain(requestGate, message, { dropOutcome: false });
        return;
      }

      if (presentationReopened) {
        markTaskFallbackRequestUncertain(requestGate, message, { dropOutcome: true });
        return;
      }

      clearTaskFallbackRequest(requestGate);
      await reloadTasks();
    } catch (err: any) {
      const detail = err?.message_zh || err?.message || t("downloads.fallback.rebuildFailed");
      if (isDeterministicTypedError(err)) {
        handleDeterministicFallbackError(requestGate, detail);
        return;
      }
      const message = t("downloads.fallback.uncertainDetail", { detail });
      markTaskFallbackRequestUncertain(requestGate, message);
    }
  };

  const handleConfirmClearHistory = async () => {
    try {
      await clearTaskHistory();
      if (selectedTaskDetail && history.some((t) => t.task_id === selectedTaskDetail.task_id)) {
        setSelectedTaskDetail(null);
      }
      await reloadTasks();
      await onHistoricalMutationApplied?.();
    } finally {
      setShowClearHistoryConfirm(false);
    }
  };

  if (!open) return null;

  const panelBody = (
    <div
      tabIndex={-1}
      className={cn(
        "flex h-full flex-col relative outline-none",
        isPage
          ? "w-full bg-sp-surface text-sp-text"
          : "w-[460px] max-w-[95vw] bg-sp-canvas shadow-modal border-l border-sp-line"
      )}
      onClick={isPage ? undefined : (e) => e.stopPropagation()}
    >
        <div aria-live="polite" className="sr-only">{orbitAnnouncement}</div>
        {/* 抽屉 / 页面头部（EOStage Compact Header + Segmented Filter） */}
        <div
          data-testid={isPage ? "download-center-tour-anchor" : undefined}
          className="flex shrink-0 flex-col gap-3 border-b border-sp-line bg-sp-canvas px-6 py-3"
        >
          <div className="flex items-center justify-between max-w-5xl w-full mx-auto">
            <div className="flex items-center gap-3 min-w-0">
              {selectedTaskDetail ? (
                <button
                  type="button"
                  onClick={() => setSelectedTaskDetail(null)}
                  className="flex h-7 w-7 items-center justify-center rounded-sp-sm text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text transition-colors duration-fast ease-sp"
                  aria-label={t("downloads.drawer.backToTasks")}
                >
                  <ArrowLeft className="h-4 w-4" />
                </button>
              ) : (
                <Download className="h-4 w-4 text-sp-accent shrink-0" />
              )}
              <h2 className="text-[14px] font-semibold text-sp-text tracking-tight truncate">
                {t(selectedTaskDetail ? "downloads.drawer.detailTitle" : "downloads.drawer.title")}
              </h2>
              {!selectedTaskDetail && (
                <span className="rounded-full border border-sp-line bg-sp-card px-2 py-1 font-mono text-xs font-semibold text-sp-text-secondary shrink-0">
                  {t("downloads.drawer.taskCount", { total: groupedAllTasks.length })}
                </span>
              )}
            </div>

            {!isPage && (
              <div className="flex items-center gap-1 shrink-0">
              {onOpenFullPage && (
                <button
                  type="button"
                  data-testid="download-drawer-open-full-page-btn"
                  onClick={onOpenFullPage}
                  className="flex h-7 items-center gap-1 rounded-sp-sm px-2 text-[13px] text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text active:scale-95 transition-[colors,transform] duration-fast ease-sp cursor-pointer"
                  aria-label={t("downloads.drawer.openFullPage")}
                  title={t("downloads.drawer.openFullPage")}
                >
                  <Maximize2 className="h-3.5 w-3.5" />
                  <span>{t("downloads.drawer.openFullPageShort")}</span>
                </button>
              )}
              <button
                autoFocus
                type="button"
                onClick={onClose}
                className="flex h-7 w-7 items-center justify-center rounded-sp-sm text-sp-text-muted hover:bg-sp-hover hover:text-sp-text transition-colors duration-fast ease-sp cursor-pointer"
                aria-label={t("downloads.drawer.closeDrawer")}
                title={t("downloads.drawer.closeDrawerHint")}
              >
                <X className="h-4 w-4" />
              </button>
              </div>
            )}
          </div>

          {!selectedTaskDetail && (
            <div className="max-w-5xl w-full mx-auto space-y-2">
              <div
                role="tablist"
                aria-label={t("downloads.drawer.subviewsAria")}
                className="inline-flex items-center gap-1 rounded-sp-md bg-sp-hover p-1"
              >
                <button
                  type="button"
                  role="tab"
                  aria-selected={drawerSubview === "tasks"}
                  data-testid="download-subview-tasks"
                  onClick={() => setDrawerSubview("tasks")}
                  className={cn(
                    "flex h-7 items-center rounded-sp-sm px-3 text-[13px] outline-none transition-colors duration-fast ease-sp cursor-pointer",
                    drawerSubview === "tasks"
                      ? "bg-sp-surface font-semibold text-sp-text shadow-card dark:bg-sp-card"
                      : "text-sp-text-secondary hover:text-sp-text"
                  )}
                >
                  {t("downloads.drawer.tabTasks")}
                </button>
                <button
                  type="button"
                  role="tab"
                  aria-selected={drawerSubview === "artifacts"}
                  data-testid="download-subview-artifacts"
                  onClick={() => setDrawerSubview("artifacts")}
                  className={cn(
                    "flex h-7 items-center rounded-sp-sm px-3 text-[13px] outline-none transition-colors duration-fast ease-sp cursor-pointer",
                    drawerSubview === "artifacts"
                      ? "bg-sp-surface font-semibold text-sp-text shadow-card dark:bg-sp-card"
                      : "text-sp-text-secondary hover:text-sp-text"
                  )}
                >
                  {t("downloads.drawer.tabArtifacts")}
                </button>
              </div>
              {drawerSubview === "tasks" && (
                <DownloadFilterBar
                  tasks={allTasks}
                  selectedType={typeFilter}
                  onSelectType={setTypeFilter}
                  selectedStatus={statusFilter}
                  onSelectStatus={setStatusFilter}
                />
              )}
            </div>
          )}
        </div>

        {/* 内容区：列表视图 vs 详情二级视图 */}
        <div className="flex-1 overflow-y-auto px-6 py-5">
          <div className="max-w-5xl w-full mx-auto space-y-6">
          {selectedTaskDetail && !allTasks.some((task) => task.task_id === selectedTaskDetail.task_id) ? (
            <div role="status" className="space-y-3 text-sm text-sp-text-secondary">
              <p>{t("downloads.drawer.missingTask")}</p>
              <button type="button" onClick={() => setSelectedTaskDetail(null)} className="rounded-sp-sm px-2 py-1 text-sp-accent-strong hover:bg-sp-hover">{t("downloads.drawer.backToList")}</button>
            </div>
          ) : selectedTaskDetail ? (
            /* 任务详情二级视图（同一信息只在这一处详细展示） */
            <TaskDetailView
              task={allTasks.find((task) => task.task_id === selectedTaskDetail.task_id)!}
              showTimestamps={isPage}
              onBack={() => setSelectedTaskDetail(null)}
              onOpenFolder={onOpenFolder}
              onResume={() => handleResume(selectedTaskDetail.task_id)}
              onConfigureAccount={onConfigureAccount}
              newTaskPolicy={newTaskPolicy}
              onHistoricalMutationApplied={async () => {
                await reloadTasks();
                if (onHistoricalMutationApplied) {
                  await onHistoricalMutationApplied();
                }
              }}
            />
          ) : drawerSubview === "artifacts" ? (
            <TaskArtifactsVerificationPanel
              tasks={allTasks}
              onOpenFolder={onOpenFolder}
              onOpenTask={(taskId) => {
                const found = allTasks.find((entry) => entry.task_id === taskId);
                if (found) {
                  setDrawerSubview("tasks");
                  setSelectedTaskDetail(found);
                }
              }}
            />
          ) : (
            <>
              {/* 分区 1：执行队列 */}
              {(statusFilter === "all" ||
                statusFilter === "active" ||
                statusFilter === "paused" ||
                queueRows.length > 0 ||
                pendingPreparations.length > 0) && (
              <section className="space-y-3">
                <div className="flex items-center justify-between px-1">
                  <div className="flex items-center gap-2 text-xs font-bold text-sp-text">
                    <span>{t("downloads.drawer.queue")}</span>
                    <span className="text-xs font-mono text-sp-text-muted">
                      ({queueRows.length + pendingPreparations.length})
                    </span>
                  </div>
                </div>

                {queueRows.length === 0 && pendingPreparations.length === 0 ? (
                  queue.length > 0 && (typeFilter !== "all" || statusFilter !== "all") ? (
                    <EmptyState
                      kind="no-results"
                      size="card"
                      visual={<EmptyStateIcon name="filter" size={22} />}
                      title={t("downloads.drawer.noFilteredTasks")}
                      primaryAction={{
                        label: t("downloads.drawer.clearFilters"),
                        onClick: () => {
                          setTypeFilter("all");
                          setStatusFilter("all");
                        },
                      }}
                    />
                  ) : (
                    <EmptyState
                      kind="all-done"
                      size="card"
                      visual={<EmptyStateIcon name="inbox" size={22} neutral />}
                      title={t("downloads.drawer.emptyQueueTitle")}
                    />
                  )
                ) : (
                  <div className="space-y-2">
                    {pendingPreparations.map((prep) => (
                      <div
                        key={prep.id}
                        data-download-preparation-card={prep.id}
                        className="rounded-card border border-sp-line bg-sp-card px-4 py-3 space-y-2"
                      >
                        <div className="flex items-center justify-between gap-2">
                          <div className="flex items-center gap-2 min-w-0 flex-1">
                            {prep.status === "preparing" ? (
                              <Loader2 className="h-4 w-4 text-sp-accent animate-spin shrink-0" />
                            ) : (
                              <AlertCircle className="h-4 w-4 text-sp-danger-text shrink-0" />
                            )}
                            <span
                              className="text-xs font-semibold text-sp-text truncate"
                              title={prep.title}
                            >
                              {prep.title}
                            </span>
                          </div>
                          {prep.status === "preparing" ? (
                            <span className="rounded-full border border-sp-line bg-sp-surface px-2 py-1 text-xs font-semibold text-sp-text shrink-0 whitespace-nowrap">
                              {t("mapWorkbench.desktop.creatingTaskGeneric")}
                            </span>
                          ) : (
                            <button
                              type="button"
                              onClick={() => onDismissPreparation?.(prep.id)}
                              className="rounded-sp-sm px-2 py-1 text-[13px] text-sp-text-muted hover:bg-sp-hover hover:text-sp-text"
                            >
                              {t("mapWorkbench.results.close")}
                            </button>
                          )}
                        </div>
                        <div className="flex items-center justify-between gap-2 text-xs text-sp-text-muted">
                          <span>
                            {prep.status === "preparing" && prep.phase === "committing"
                              ? t("downloads.drawer.prepCommitting", { prepared: prep.count, total: prep.count })
                              : prep.status === "preparing" && typeof prep.preparedCount === "number"
                              ? t("downloads.drawer.prepPreparing", { prepared: prep.preparedCount, total: prep.count })
                              : t("downloads.drawer.prepPendingCount", { count: prep.count })}
                          </span>
                          <span className="font-mono truncate max-w-[260px]" title={prep.outputRoot}>
                            {prep.outputRoot}
                          </span>
                        </div>
                        {prep.status === "preparing" && prep.count > 0 && typeof prep.preparedCount === "number" && (
                          <div className="h-1.5 w-full overflow-hidden rounded-full bg-sp-surface-muted">
                            <div
                              className="h-full rounded-full bg-sp-accent transition-[width] duration-200 ease-out"
                              style={{
                                width: `${Math.min(100, Math.max(4, Math.round((prep.preparedCount / prep.count) * 100)))}%`,
                              }}
                            />
                          </div>
                        )}
                        {prep.status === "failed" && prep.error && (
                          <div role="alert" className="rounded-sp-sm bg-sp-danger-bg px-3 py-2 text-xs text-sp-danger-text">
                            <p className="break-words">{prep.error}</p>
                            {prep.actionAccountId && onConfigureAccount && (
                              <button
                                type="button"
                                data-testid="prep-configure-account-btn"
                                onClick={() => onConfigureAccount(prep.actionAccountId ?? undefined)}
                                className="mt-2 inline-flex h-sp-control items-center px-3 rounded-sp-sm bg-sp-accent-fill text-white text-[13px] font-semibold hover:opacity-90 cursor-pointer transition-colors duration-fast ease-sp"
                              >
                                {prep.actionLabel || t("accounts.loginCard.openSettings")}
                              </button>
                            )}
                          </div>
                        )}
                      </div>
                    ))}
                    {queueRows.map(({ task, companions }) => {
                      const isHighPriority = prioritizedTaskIds.includes(task.task_id);
                      const row = (
                        <DownloadTaskRow
                          task={task}
                          compact={!isPage}
                          canRetryFallback={isTaskFallbackEligible(task, fallbackEligibility)}
                          companions={companions}
                          independentOrbit={isOrbitCompanion(task)}
                          priority={isHighPriority ? "high" : "normal"}
                          onTogglePriority={isPage ? () => toggleTaskPriority(task.task_id) : undefined}
                          onPause={() => handlePause(task.task_id)}
                          onResume={() => handleResume(task.task_id)}
                          onRetry={() => handleRetry(task.task_id)}
                          onRetryFallback={() => openFallback(task)}
                          onViewDetails={() => setSelectedTaskDetail(task)}
                          onOpenFolder={onOpenFolder}
                          onCancel={() => handleCancel(task.task_id)}
                          onItemAction={isPage ? handleItemAction : undefined}
                          onOrbitAction={handleOrbitAction}
                          onOrbitAnnouncement={setOrbitAnnouncement}
                          onConfigureAccount={onConfigureAccount}
                        />
                      );
                      return companions.length > 0 ? (
                        <div key={task.task_id} role="group" aria-label={t("downloads.drawer.taskGroup", { name: task.name })}>{row}</div>
                      ) : <React.Fragment key={task.task_id}>{row}</React.Fragment>;
                    })}
                  </div>
                )}
              </section>
              )}

              {/* 分区 2：历史记录 */}
              {(statusFilter === "all" ||
                statusFilter === "completed" ||
                statusFilter === "failed" ||
                historyRows.length > 0) && (
              <section id="download-drawer-history-section" className="space-y-3 pt-4">
                <div className="flex items-center justify-between px-1">
                  <div className="flex items-center gap-2 text-xs font-bold text-sp-text">
                    <span>{t("downloads.drawer.history")}</span>
                    <span className="text-xs font-mono text-sp-text-muted">
                      ({historyRows.length})
                    </span>
                  </div>

                  {history.length > 0 && (
                    <button
                      type="button"
                      onClick={() => setShowClearHistoryConfirm(true)}
                      className="flex h-7 shrink-0 items-center gap-1 whitespace-nowrap px-3 rounded-sp-sm text-[13px] font-semibold text-sp-text-muted hover:text-sp-danger-text hover:bg-sp-danger-bg transition-colors duration-fast ease-sp"
                      aria-label={t("downloads.drawer.clearHistoryAria")}
                    >
                      <Trash2 className="h-3 w-3" />
                      {t("downloads.drawer.clearHistory")}
                    </button>
                  )}
                </div>

                {historyRows.length === 0 ? (
                  history.length > 0 && (typeFilter !== "all" || statusFilter !== "all") ? (
                    <EmptyState
                      kind="no-results"
                      size="card"
                      visual={<EmptyStateIcon name="filter" size={22} />}
                      title={t("downloads.drawer.noFilteredHistory")}
                      primaryAction={{
                        label: t("downloads.drawer.clearFilters"),
                        onClick: () => {
                          setTypeFilter("all");
                          setStatusFilter("all");
                        },
                      }}
                    />
                  ) : (
                    <EmptyState
                      kind="all-done"
                      size="card"
                      visual={<EmptyStateIcon name="archive" size={22} neutral />}
                      title={t("downloads.drawer.emptyHistoryTitle")}
                    />
                  )
                ) : (
                  <div className="space-y-1">
                    {historyRows.map(({ task, companions }) => {
                      const isHighPriority = prioritizedTaskIds.includes(task.task_id);
                      const row = (
                        <DownloadTaskRow
                          task={task}
                          compact={!isPage}
                          canRetryFallback={isTaskFallbackEligible(task, fallbackEligibility)}
                          companions={companions}
                          independentOrbit={isOrbitCompanion(task)}
                          priority={isHighPriority ? "high" : "normal"}
                          onTogglePriority={isPage ? () => toggleTaskPriority(task.task_id) : undefined}
                          onPause={() => handlePause(task.task_id)}
                          onResume={() => handleResume(task.task_id)}
                          onRetry={() => handleRetry(task.task_id)}
                          onRetryFallback={() => openFallback(task)}
                          onViewDetails={() => setSelectedTaskDetail(task)}
                          onOpenFolder={onOpenFolder}
                          onCancel={() => handleCancel(task.task_id)}
                          onDeleteHistory={isPage ? () => handleDeleteHistoryTask(task.task_id) : undefined}
                          onItemAction={isPage ? handleItemAction : undefined}
                          onOrbitAction={handleOrbitAction}
                          onOrbitAnnouncement={setOrbitAnnouncement}
                          onConfigureAccount={onConfigureAccount}
                        />
                      );
                      return (
                        <div key={task.task_id} id={`task-card-${task.task_id}`} data-task-status={task.status} role={companions.length > 0 ? "group" : undefined} aria-label={companions.length > 0 ? t("downloads.drawer.taskGroup", { name: task.name }) : undefined}>
                          {row}
                        </div>
                      );
                    })}
                  </div>
                )}
              </section>
              )}
            </>
          )}
          </div>
        </div>

        {/* 对话框 1-A：「重新检索后重试」第一步确认对话框 */}
        {fallbackConfirmTask && !fallbackDiffOutcome && (
          <div
            role="alertdialog"
            aria-modal="true"
            aria-label={RETRY_FALLBACK_DIALOG_TEXT.title}
            className="absolute inset-0 z-50 flex items-center justify-center p-4 bg-black/20"
            onClick={resetFallbackState}
          >
            <div
              className="w-full max-w-sm rounded-modal bg-sp-surface p-5 shadow-modal border border-sp-line space-y-4"
              onClick={(e) => e.stopPropagation()}
            >
              <div className="flex items-center gap-2 text-sp-warning-text font-bold text-sm">
                <AlertCircle className="h-4 w-4 shrink-0" />
                <h3>{RETRY_FALLBACK_DIALOG_TEXT.title}</h3>
              </div>

              <div className="space-y-2 text-xs text-sp-text leading-relaxed">
                <p>
                  {t("downloads.fallback.description")}
                </p>
                <p className="text-sp-text-muted">
                  {t("downloads.fallback.differenceNote")}
                </p>
                <div className="rounded-sp-sm bg-sp-warning-bg p-2 text-xs text-sp-warning-text">
                  {t("downloads.fallback.warning")}
                </div>
              </div>

              {fallbackLoading && <p role="status" className="text-xs text-sp-text-muted">{t("downloads.fallback.pending")}</p>}
              {displayFallbackError && (
                <div className="rounded-sp-sm bg-sp-danger-bg p-2 text-xs text-sp-danger-text">
                  {displayFallbackError}
                </div>
              )}

              <div className="flex items-center justify-end gap-2 pt-2 border-t border-sp-divider">
                <button
                  type="button"
                  disabled={fallbackLoading}
                  onClick={resetFallbackState}
                  className="flex h-sp-control items-center px-3 rounded-sp-sm text-[13px] font-semibold text-sp-text border border-sp-line hover:bg-sp-hover transition-colors duration-fast ease-sp disabled:opacity-50"
                >
                  {RETRY_FALLBACK_DIALOG_TEXT.cancelText}
                </button>
                <button
                  type="button"
                  disabled={fallbackLoading || fallbackUncertain || isStaleConfirmTask}
                  onClick={() => void handleFirstStepRetryFallback()}
                  className="flex h-sp-control items-center gap-2 px-3 rounded-sp-sm text-[13px] font-semibold bg-sp-accent-fill text-white enabled:hover:opacity-90 transition-opacity duration-fast ease-sp disabled:opacity-50"
                >
                  {fallbackLoading && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
                  <span>{RETRY_FALLBACK_DIALOG_TEXT.confirmText}</span>
                </button>
              </div>
            </div>
          </div>
        )}

        {/* 对话框 1-B：「重新检索后重试」第二步差异确认对话框 */}
        {fallbackConfirmTask && fallbackDiffOutcome && (
          <div
            role="alertdialog"
            aria-modal="true"
            aria-label={t("downloads.fallback.differenceTitle")}
            className="absolute inset-0 z-50 flex items-center justify-center p-4 bg-black/20"
            onClick={resetFallbackState}
          >
            <div
              className="w-full max-w-md rounded-modal bg-sp-surface p-5 shadow-modal border border-sp-line space-y-4"
              onClick={(e) => e.stopPropagation()}
            >
              <div className="flex items-center gap-2 text-sp-warning-text font-bold text-sm">
                <AlertCircle className="h-4 w-4 shrink-0" />
                <h3>{t("downloads.fallback.differenceTitle")}</h3>
              </div>

              <div className="space-y-3 text-xs text-sp-text leading-relaxed">
                <p className="text-sp-text-muted">
                  {t("downloads.fallback.differenceDescription")}
                </p>

                {fallbackDiffOutcome.missing_product_ids.length === 0 &&
                fallbackDiffOutcome.added_product_ids.length === 0 ? (
                  <div className="rounded-sp-sm bg-sp-hover p-3 text-xs text-sp-text-muted text-center">
                    {t("downloads.fallback.noChanges")}
                  </div>
                ) : (
                  <div className="space-y-3 max-h-56 overflow-y-auto pr-1">
                    {fallbackDiffOutcome.missing_product_ids.length > 0 && (
                      <div className="space-y-2 rounded-sp-sm bg-sp-danger-bg/40 p-3 border border-sp-danger-text/20">
                        <div className="font-semibold text-sp-danger-text text-xs flex items-center justify-between">
                          <span>{t("downloads.fallback.missingProducts", { amount: fallbackDiffOutcome.missing_product_ids.length })}</span>
                        </div>
                        <ul className="space-y-1 font-mono text-xs text-sp-danger-text/90">
                          {fallbackDiffOutcome.missing_product_ids.map((id) => (
                            <li key={id} title={id} className="truncate">
                              · {truncateMiddle(id, 20, 14)}
                            </li>
                          ))}
                        </ul>
                      </div>
                    )}

                    {fallbackDiffOutcome.added_product_ids.length > 0 && (
                      <div className="space-y-2 rounded-sp-sm bg-sp-warning-bg/40 p-3 border border-sp-warning-text/20">
                        <div className="font-semibold text-sp-warning-text text-xs flex items-center justify-between">
                          <span>{t("downloads.fallback.addedProducts", { amount: fallbackDiffOutcome.added_product_ids.length })}</span>
                        </div>
                        <ul className="space-y-1 font-mono text-xs text-sp-warning-text/90">
                          {fallbackDiffOutcome.added_product_ids.map((id) => (
                            <li key={id} title={id} className="truncate">
                              · {truncateMiddle(id, 20, 14)}
                            </li>
                          ))}
                        </ul>
                      </div>
                    )}
                  </div>
                )}

                <div className="rounded-sp-sm bg-sp-warning-bg p-2 text-xs text-sp-warning-text">
                  {t("downloads.fallback.confirmWarning")}
                </div>
              </div>

              {fallbackLoading && <p role="status" className="text-xs text-sp-text-muted">{t("downloads.fallback.pending")}</p>}
              {displayFallbackError && (
                <div className="rounded-sp-sm bg-sp-danger-bg p-2 text-xs text-sp-danger-text">
                  {displayFallbackError}
                </div>
              )}

              <div className="flex items-center justify-end gap-2 pt-2 border-t border-sp-divider">
                <button
                  type="button"
                  disabled={fallbackLoading}
                  onClick={resetFallbackState}
                  className="flex h-sp-control items-center px-3 rounded-sp-sm text-[13px] font-semibold text-sp-text border border-sp-line hover:bg-sp-hover transition-colors duration-fast ease-sp disabled:opacity-50"
                >
{t("downloads.task.cancel")}
                </button>
                <button
                  type="button"
                  disabled={fallbackLoading || fallbackUncertain || isStaleConfirmTask}
                  onClick={() => void handleSecondStepConfirm()}
                  className="flex h-sp-control items-center gap-2 px-3 rounded-sp-sm text-[13px] font-semibold bg-sp-accent-fill text-white enabled:hover:opacity-90 transition-opacity duration-fast ease-sp disabled:opacity-50"
                >
                  {fallbackLoading && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
                  <span>{t("downloads.fallback.confirmRebuild")}</span>
                </button>
              </div>
            </div>
          </div>
        )}

        {/* 对话框 2：「清除历史」确认对话框 */}
        {showClearHistoryConfirm && (
          <div
            role="alertdialog"
            aria-modal="true"
            aria-label={CLEAR_HISTORY_DIALOG_TEXT.title}
            className="absolute inset-0 z-50 flex items-center justify-center p-4 bg-black/20"
            onClick={() => setShowClearHistoryConfirm(false)}
          >
            <div
              className="w-full max-w-sm rounded-modal bg-sp-surface p-5 shadow-modal border border-sp-line space-y-4"
              onClick={(e) => e.stopPropagation()}
            >
              <div className="flex items-center gap-2 text-sp-danger-text font-bold text-sm">
                <Trash2 className="h-4 w-4 shrink-0" />
                <h3>{CLEAR_HISTORY_DIALOG_TEXT.title}</h3>
              </div>

              <div className="space-y-1 text-xs text-sp-text leading-relaxed">
                <p>{CLEAR_HISTORY_DIALOG_TEXT.content}</p>
              </div>

              <div className="flex items-center justify-end gap-2 pt-2 border-t border-sp-divider">
                <button
                  type="button"
                  onClick={() => setShowClearHistoryConfirm(false)}
                  className="flex h-sp-control items-center px-3 rounded-sp-sm text-[13px] font-semibold text-sp-text border border-sp-line hover:bg-sp-hover transition-colors duration-fast ease-sp"
                >
                  {CLEAR_HISTORY_DIALOG_TEXT.cancelText}
                </button>
                <button
                  type="button"
                  onClick={() => void handleConfirmClearHistory()}
                  className="flex h-sp-control items-center px-3 rounded-sp-sm text-[13px] font-semibold bg-sp-danger-bg text-sp-danger-text border border-sp-danger-text/30 hover:bg-sp-danger-bg/80 transition-colors duration-fast ease-sp"
                >
                  {CLEAR_HISTORY_DIALOG_TEXT.confirmText}
                </button>
              </div>
            </div>
          </div>
        )}
    </div>
  );

  if (isPage) {
    return panelBody;
  }

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label={t("downloads.drawer.title")}
      className="fixed inset-0 z-[2500] flex justify-end bg-black/20 select-none"
      onClick={onClose}
    >
      {panelBody}
    </div>
  );
}

/**
 * 统一任务卡组件（统一用于影像下载与 DEM 转换）
 */
export interface TaskCardProps {
  task: ApplicationTask;
  compact?: boolean;
  canRetryFallback?: boolean;
  companions?: ApplicationTask[];
  independentOrbit?: boolean;
  priority?: "high" | "normal";
  onTogglePriority?: () => void | Promise<void>;
  onPause?: () => void | Promise<void>;
  onResume?: () => void | Promise<void>;
  onRetry?: () => void | Promise<void>;
  onRetryFallback?: () => void | Promise<void>;
  onViewDetails?: () => void;
  onOpenFolder?: (task: TaskOutputRef) => void | Promise<void>;
  onCancel?: () => void | Promise<void>;
  onDeleteHistory?: () => void | Promise<void>;
  onItemAction?: (
    task: ApplicationTask,
    item: TaskSceneItem,
    action: UnifiedTaskItemAction
  ) => Promise<void>;
  onOrbitAction?: typeof orbitItemAction;
  onOrbitAnnouncement?: (message: string) => void;
  onConfigureAccount?: (accountId?: AccountId) => void;
}

export function TaskCard({
  task,
  onPause,
  onResume,
  onRetry,
  onRetryFallback,
  onViewDetails,
  onOpenFolder,
  onCancel,
  onConfigureAccount,
}: TaskCardProps) {
  const { t } = useI18n();
  const [actionError, setActionError] = useState<string | null>(null);
  const eta = useTaskEta(task);

  useEffect(() => {
    setActionError(null);
  }, [task.task_id]);

  const runAction = async (fn?: () => void | Promise<void>) => {
    if (!fn) return;
    setActionError(null);
    try {
      await fn();
    } catch (err: unknown) {
      setActionError(resolveTaskActionErrorText(err, t("downloads.task.actionFailed")));
    }
  };

  const openFolder = async (open: (task: TaskOutputRef) => void | Promise<void>) => {
    setActionError(null);
    try {
      await open({ task_id: task.task_id, revision: task.revision });
    } catch (err: unknown) {
      setActionError(openTaskOutputErrorText(err));
    }
  };

  // 字节未上报（归档历史）时不画由 0 字节算出的确定进度
  const hasTotalBytes =
    typeof task.total_bytes === "number" && task.total_bytes > 0 && task.bytes_reported !== false;
  const percent = hasTotalBytes
    ? Math.min(100, Math.round((task.done_bytes / task.total_bytes!) * 100))
    : null;

  const outputDir = taskOutputDir(task);
  const actions = getAvailableTaskActions(task.status, outputDir !== null);

  return (
    <div className="eo-settings-card transition-colors duration-fast ease-sp">
      <div className="eo-settings-card-content px-4 py-3 space-y-3">
        {/* 头部：类别徽章 + 标题 + 状态徽章 */}
        <div className="flex items-start justify-between gap-2">
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2 mb-1 flex-wrap">
              {/* 类别徽章：遵守基线，中性灰阶，颜色只表示状态 */}
              <span className="rounded-sp-sm border border-sp-line bg-sp-canvas px-2 py-1 text-xs font-semibold text-sp-text-secondary shrink-0">
                {t(task.category === "dem_convert" ? "downloads.task.demConvert" : "downloads.task.sceneDownload")}
              </span>
              <span className="font-mono text-xs text-sp-text-muted truncate">
                {(normalizeTaskProviderLabel(task.provider) || t("downloads.task.unknownSource"))} · {taskDataTypeLabel(taskDataTypeKey(task))}
              </span>
            </div>

            <h3
              className="text-sm font-semibold text-sp-text truncate cursor-default"
              title={normalizeTaskDisplayName(task)}
            >
              {normalizeTaskDisplayName(task)}
            </h3>
          </div>

          {/* 状态徽章：彩色只表示状态 */}
          <StatusBadge status={task.status} category={task.category} />
        </div>

        {/* 进度条：只有真实总量已知时才显示确定进度条；否则用不定进度 */}
        <div className="w-full h-1.5 rounded-full bg-sp-surface-muted overflow-hidden">
          {task.status === "completed" ? (
            <div className="h-full w-full bg-sp-success-text rounded-full" />
          ) : hasTotalBytes && percent !== null ? (
            <div
              className="h-full bg-sp-accent-fill rounded-full transition-[width] duration-base ease-sp"
              style={{ width: `${percent}%` }}
            />
          ) : task.status === "running" ? (
            /* 不定进度 (indeterminate) */
            <div className="h-full w-1/3 bg-sp-accent-fill rounded-full progress-indeterminate-bar" />
          ) : null}
        </div>

        {/* 磁盘空间不足已暂停提示行 */}
        {(() => {
          const pauseLine = pauseReasonLine(task.pause_reason);
          if (!pauseLine) return null;
          return (
            <div className="flex items-center gap-2 text-xs text-sp-warning-text">
              <HardDrive className="h-3.5 w-3.5 shrink-0" />
              <span>{pauseLine}</span>
            </div>
          );
        })()}

        {/* 操作失败提示行 */}
        {actionError && (
          <div role="alert" className="rounded-sp-sm bg-sp-danger-bg p-2 text-xs text-sp-danger-text">
            {actionError}
            <button type="button" aria-label={t("downloads.task.dismissError")} onClick={() => setActionError(null)} className="ml-2 text-[13px]">{t("downloads.task.close")}</button>
          </div>
        )}

        {/* 失败时的简短提示 */}
        {task.status === "failed" && task.error && (() => {
          const accountAction = resolveAccountActionFromError(
            {
              code: typeof task.error === "object" ? task.error?.code : undefined,
              needs_account: typeof task.error === "object" ? task.error?.needs_account : undefined,
              message_zh: typeof task.error === "string" ? task.error : task.error?.message_zh,
              error: task.provider || task.source_provider_id || task.dataset,
            },
            task.dataset || task.provider || task.source_provider_id,
          );
          return (
            <div className="rounded-sp-sm bg-sp-danger-bg p-2 text-xs text-sp-danger-text space-y-2">
              <div className="break-words">
                {typeof task.error === "string" ? task.error : task.error.message_zh}
              </div>
              {accountAction.actionAccountId && onConfigureAccount && (
                <button
                  type="button"
                  data-testid="task-configure-account-btn"
                  onClick={() => onConfigureAccount(accountAction.actionAccountId ?? undefined)}
                  className="inline-flex h-sp-control items-center px-3 rounded-sp-sm bg-sp-accent-fill text-white text-[13px] font-semibold hover:opacity-90 cursor-pointer transition-colors duration-fast ease-sp"
                >
                  {accountAction.actionLabel || t("accounts.loginCard.openSettings")}
                </button>
              )}
            </div>
          );
        })()}
      </div>

      {/* Concentric Beard：左侧吞吐速度与大小，右侧操作按钮组 */}
      <div className="eo-settings-card-beard flex-wrap gap-2">
        <div className="flex items-center gap-3 text-xs text-sp-text-muted font-mono">
          <span>{taskCardSizeLine(task, formatBytes)}</span>
          <span>·</span>
          <span>
            {task.status === "running"
              ? (task.speed_formatted ? task.speed_formatted : t("downloads.task.transferring"))
              : task.status === "paused"
                ? t("downloads.status.paused")
                : task.status === "failed"
                  ? t("downloads.task.interruptedFailure")
                  : task.status === "completed"
                    ? t("downloads.status.completed")
                    : task.status === "cancelled"
                      ? t("downloads.status.cancelled")
                      : task.status === "interrupted"
                        ? t("downloads.status.interrupted")
                        : t("downloads.task.waiting")}
          </span>
          {task.status === "running" && eta.kind !== "hidden" && (
            <>
              <span>·</span>
              <span data-testid="task-card-eta">{eta.textZh}</span>
            </>
          )}
        </div>

        {/* 操作按钮组：严格由 pure 映射派生 */}
        <div className="flex items-center justify-end gap-2 flex-wrap">
          {actions.includes("cancel") && onCancel && (
            <button
              type="button"
              onClick={() => void runAction(onCancel)}
              className="eo-settings-btn-secondary h-7 px-3"
              aria-label={t("downloads.task.cancelTask")}
            >
              <X className="h-3.5 w-3.5" />
              {t("downloads.task.cancel")}
            </button>
          )}
          {actions.includes("pause") && (
            <button
              type="button"
              onClick={() => void runAction(onPause)}
              className="eo-settings-btn-secondary h-7 px-3"
              aria-label={t("downloads.task.pauseTask")}
            >
              <Pause className="h-3.5 w-3.5" />
              {t("downloads.task.pause")}
            </button>
          )}

          {actions.includes("resume") && (
            <button
              type="button"
              onClick={() => void runAction(onResume)}
              className="eo-settings-btn-primary h-7 px-3"
              aria-label={t("downloads.task.resumeTask")}
            >
              <Play className="h-3.5 w-3.5" />
              {t("downloads.task.resume")}
            </button>
          )}

          {actions.includes("retry") && (
            <button
              type="button"
              onClick={() => void runAction(onRetry)}
              className="flex h-7 items-center gap-1 rounded-sp-sm px-3 text-[13px] font-semibold text-sp-warning-text border border-sp-warning-text/30 hover:bg-sp-warning-bg transition-colors duration-fast ease-sp"
              aria-label={t("downloads.task.normalRetry")}
            >
              <RotateCcw className="h-3.5 w-3.5" />
              {t("downloads.task.retry")}
            </button>
          )}

          {actions.includes("retry_fallback") && (
            <button
              type="button"
              onClick={() => void runAction(onRetryFallback)}
              className="flex h-7 items-center gap-1 rounded-sp-sm px-3 text-[13px] font-semibold text-sp-warning-text border border-sp-warning-text/30 hover:bg-sp-warning-bg transition-colors duration-fast ease-sp"
              aria-label={t("downloads.task.retryFallback")}
            >
              <RefreshCw className="h-3.5 w-3.5" />
              <span>{t("downloads.task.retryFallback")}</span>
            </button>
          )}

          {actions.includes("details") && (
            <button
              type="button"
              onClick={onViewDetails}
              className="flex h-7 items-center gap-1 rounded-sp-sm px-2 text-[13px] font-semibold text-sp-text-muted hover:text-sp-text hover:bg-sp-hover transition-colors duration-fast ease-sp"
              aria-label={t("downloads.task.viewDetails")}
            >
              <Info className="h-3.5 w-3.5" />
              {t("downloads.task.details")}
            </button>
          )}

          {/* 有保存目录就能打开（队列中的进行中/暂停任务与历史任务都一样，L4 r10 P2-2） */}
          {actions.includes("open_folder") && outputDir !== null && onOpenFolder && (
            <button
              type="button"
              onClick={() => {
                void openFolder(onOpenFolder);
              }}
              className="flex h-7 items-center gap-1 rounded-sp-sm px-2 text-[13px] font-semibold text-sp-text-muted hover:text-sp-text hover:bg-sp-hover transition-colors duration-fast ease-sp"
              aria-label={t("downloads.task.openFolderAria")}
              title={isFixtureBackend() ? t("downloads.task.fixtureFolderHint") : outputDir}
            >
              <FolderOpen className="h-3.5 w-3.5" />
              {t("downloads.task.openFolder")}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}

/**
 * 状态徽章：颜色只表示状态
 */
export function StatusBadge({
  status,
  category,
}: {
  status: TaskLifecycleStatus;
  category?: string;
}) {
  const { t } = useI18n();
  switch (status) {
    case "running":
      return (
        <span className="rounded-full bg-sp-selected px-2 py-1 text-xs font-semibold text-sp-accent-strong shrink-0 flex items-center gap-1">
          {t(category === "dem_convert" ? "downloads.status.converting" : "downloads.status.downloading")}
        </span>
      );
    case "completed":
      return (
        <span className="rounded-full bg-sp-success-bg px-2 py-1 text-xs font-semibold text-sp-success-text shrink-0 flex items-center gap-1">
          <CheckCircle2 className="h-3 w-3" />
{t("downloads.status.completed")}
        </span>
      );
    case "paused":
      return (
        <span className="rounded-full bg-sp-warning-bg px-2 py-1 text-xs font-semibold text-sp-warning-text shrink-0 flex items-center gap-1">
          <Pause className="h-3 w-3" />
{t("downloads.status.paused")}
        </span>
      );
    case "failed":
      return (
        <span className="rounded-full bg-sp-danger-bg px-2 py-1 text-xs font-semibold text-sp-danger-text shrink-0 flex items-center gap-1">
          <AlertCircle className="h-3 w-3" />
{t("downloads.status.failed")}
        </span>
      );
    case "queued":
      return (
        <span className="rounded-full bg-sp-surface-muted px-2 py-1 text-xs font-semibold text-sp-text-secondary shrink-0 flex items-center gap-1">
          <Clock className="h-3 w-3" />
{t("downloads.status.queued")}
        </span>
      );
    case "cancelled":
      return (
        <span className="rounded-full bg-sp-surface-muted px-2 py-1 text-xs font-semibold text-sp-text-muted shrink-0 flex items-center gap-1">
          <XCircle className="h-3 w-3" />
{t("downloads.status.cancelled")}
        </span>
      );
    case "interrupted":
      return (
        <span className="rounded-full bg-sp-surface-muted px-2 py-1 text-xs font-semibold text-sp-text-muted shrink-0 flex items-center gap-1">
          <XCircle className="h-3 w-3" />
{t("downloads.status.interrupted")}
        </span>
      );
    default:
      return null;
  }
}

/**
 * 任务详情二级视图（同一信息只在这一处详细展示）
 */
export function TaskDetailView({
  task,
  showTimestamps = false,
  onBack,
  onOpenFolder,
  onResume,
  onConfigureAccount,
  onHistoricalMutationApplied,
  newTaskPolicy,
}: {
  task: ApplicationTask;
  showTimestamps?: boolean;
  onBack?: () => void;
  onOpenFolder?: (task: TaskOutputRef) => void | Promise<void>;
  onResume?: () => void | Promise<void>;
  onConfigureAccount?: (accountId?: AccountId) => void;
  onHistoricalMutationApplied?: () => void | Promise<void>;
  newTaskPolicy?: { concurrency: number; layout: DownloadLayout } | null;
}) {
  const { t } = useI18n();
  const [actionError, setActionError] = useState<string | null>(null);
  const eta = useTaskEta(task);
  const outputDir = taskOutputDir(task);
  const [techOpen, setTechOpen] = useState(false);
  const [copyState, setCopyState] = useState<"idle" | "copied" | "failed">("idle");

  const copyTaskId = async () => {
    try {
      const clipboard = typeof navigator !== "undefined" ? navigator.clipboard : undefined;
      if (!clipboard || typeof clipboard.writeText !== "function") throw new Error("clipboard unavailable");
      await clipboard.writeText(task.task_id);
      setCopyState("copied");
    } catch {
      setCopyState("failed");
    }
  };

  useEffect(() => {
    if (copyState === "idle") return;
    const timer = window.setTimeout(() => setCopyState("idle"), 3000);
    return () => window.clearTimeout(timer);
  }, [copyState, task.task_id]);

  useEffect(() => {
    setActionError(null);
    setCopyState("idle");
  }, [task.task_id]);

  const handleResumeClick = async () => {
    if (!onResume) return;
    setActionError(null);
    try {
      await onResume();
    } catch (err: unknown) {
      setActionError(resolveTaskActionErrorText(err, t("downloads.task.actionFailed")));
    }
  };

  const handleOpenFolderClick = async () => {
    if (!onOpenFolder) return;
    setActionError(null);
    try {
      await onOpenFolder({ task_id: task.task_id, revision: task.revision });
    } catch (err: unknown) {
      setActionError(openTaskOutputErrorText(err, t("downloads.task.open")));
    }
  };

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        {onBack ? (
          <button
            type="button"
            onClick={onBack}
            className="flex h-7 items-center gap-1 rounded-sp-sm px-2 text-[13px] font-semibold text-sp-text-secondary hover:bg-sp-hover"
          >
            <ArrowLeft className="h-3.5 w-3.5" />
{t("downloads.drawer.backToList")}
          </button>
        ) : (
          <div />
        )}
        <StatusBadge status={task.status} category={task.category} />
      </div>

      {/* 磁盘空间不足已暂停提示行 */}
      {(() => {
        const pauseLine = pauseReasonLine(task.pause_reason);
        if (!pauseLine) return null;
        return (
          <div className="flex items-center gap-2 text-xs text-sp-warning-text px-1">
            <HardDrive className="h-3.5 w-3.5 shrink-0" />
            <span>{pauseLine}</span>
          </div>
        );
      })()}

      {/* 操作失败提示行 */}
      {actionError && (
        <div role="alert" className="rounded-sp-sm bg-sp-danger-bg p-2 text-xs text-sp-danger-text">
          {actionError}
          <button type="button" aria-label={t("downloads.task.dismissError")} onClick={() => setActionError(null)} className="ml-2">{t("downloads.task.close")}</button>
        </div>
      )}

      <div className="rounded-card bg-sp-surface p-4 shadow-card border border-sp-line space-y-3">
        <div>
          <div className="text-xs text-sp-text-muted mb-1">{t("downloads.detail.taskName")}</div>
          <div className="text-sm font-bold text-sp-text">{task.name}</div>
        </div>

        <div className="grid grid-cols-2 gap-3 pt-2 border-t border-sp-divider">
          <div>
            <div className="text-xs text-sp-text-muted">{t("downloads.detail.taskType")}</div>
            <div className="text-xs font-semibold text-sp-text mt-1">
              {t(task.category === "dem_convert" ? "downloads.detail.demTask" : "downloads.detail.sceneTask")}
            </div>
          </div>
          <div>
            <div className="text-xs text-sp-text-muted">{t("downloads.detail.provider")}</div>
            <div className="text-xs font-semibold text-sp-text mt-1">
              {task.provider || t("downloads.detail.unknown")}
            </div>
          </div>
        </div>

        {/* 技术信息：任务标识只在排查问题时需要，默认收起 */}
        <div className="pt-2 border-t border-sp-divider">
          <button
            type="button"
            aria-expanded={techOpen}
            onClick={() => setTechOpen((v) => !v)}
            className="flex items-center gap-1 text-[13px] text-sp-text-muted hover:text-sp-text cursor-pointer transition-colors duration-fast ease-sp"
          >
            <ChevronRight
              className={cn("h-3 w-3 transition-transform duration-fast ease-sp", techOpen && "rotate-90")}
            />
{t("downloads.detail.technicalInfo")}
          </button>
          {techOpen && (
            <div className="mt-2 pl-4">
              <div className="text-xs text-sp-text-muted">{t("downloads.detail.taskId")}</div>
              <div className="flex items-start justify-between gap-2 mt-1">
                <span className="text-xs font-mono text-sp-text-secondary break-all select-text">{task.task_id}</span>
                <button
                  type="button"
                  aria-label={t("downloads.detail.copyTaskId")}
                  title={t("downloads.detail.copyTaskId")}
                  onClick={() => void copyTaskId()}
                  className="flex h-7 shrink-0 items-center gap-1 px-2 rounded-sp-sm text-[13px] font-semibold text-sp-text-muted hover:text-sp-text hover:bg-sp-hover transition-colors duration-fast ease-sp"
                >
                  <Copy className="h-3 w-3" />
                  {t(copyState === "copied" ? "downloads.detail.copied" : copyState === "failed" ? "downloads.detail.copyFailed" : "downloads.detail.copy")}
                </button>
              </div>
            </div>
          )}
        </div>

        {outputDir !== null && (
          <div className="pt-2 border-t border-sp-divider">
            <div className="text-xs text-sp-text-muted">{t("downloads.detail.outputDir")}</div>
            <div className="flex items-center justify-between gap-2 mt-1">
              <span
                className="text-xs font-mono text-sp-text-secondary truncate"
                title={outputDir}
              >
                {outputDir}
              </span>
              {onOpenFolder && (
                <button
                  type="button"
                  onClick={() => void handleOpenFolderClick()}
                  className="flex h-7 items-center gap-1 px-3 rounded-sp-sm text-[13px] font-semibold text-sp-accent-strong hover:bg-sp-hover shrink-0"
                  title={isFixtureBackend() ? t("downloads.task.fixtureFolderHint") : t("downloads.task.openFolder")}
                >
                  <FolderOpen className="h-3 w-3" />
                  {t("downloads.task.open")}
                </button>
              )}
            </div>
          </div>
        )}

        {task.status === "failed" && (() => {
          const reason = failureReasonLine(task);
          const accountAction = resolveAccountActionFromError(
            {
              code: typeof task.error === "object" && task.error ? task.error.code : undefined,
              needs_account: typeof task.error === "object" && task.error ? task.error.needs_account : undefined,
              primary: reason.primary,
              secondary: reason.secondary,
              message_zh: typeof task.error === "string" ? task.error : task.error?.message_zh,
            },
            task.dataset || task.provider || task.source_provider_id,
          );
          return (
            <div className="pt-2 border-t border-sp-divider">
              <div className="text-xs font-semibold text-sp-danger-text mb-1">
{t("downloads.detail.failureReason")}
              </div>
              <div className="rounded-sp-sm bg-sp-danger-bg p-3 text-xs text-sp-danger-text leading-relaxed">
                <div>{reason.primary}</div>
                {reason.secondary && (
                  <div className="mt-1 text-xs opacity-90">{reason.secondary}</div>
                )}
                {accountAction.actionAccountId && onConfigureAccount && (
                  <button
                    type="button"
                    data-testid="task-configure-account-btn"
                    onClick={() => onConfigureAccount(accountAction.actionAccountId ?? undefined)}
                    className="mt-2 inline-flex h-sp-control items-center px-3 rounded-sp-sm bg-sp-accent-fill text-white text-[13px] font-semibold hover:opacity-90 cursor-pointer transition-colors duration-fast ease-sp"
                  >
                    {accountAction.actionLabel || t("accounts.loginCard.openSettings")}
                  </button>
                )}
              </div>
            </div>
          );
        })()}

        {task.scene_ids && task.scene_ids.length > 0 && (
          <div className="pt-2 border-t border-sp-divider">
            <div className="text-xs text-sp-text-muted mb-2">
{t("downloads.detail.productList", { amount: task.scene_ids.length })}
            </div>
            <div className="max-h-36 overflow-y-auto space-y-1 rounded-sp-sm bg-sp-canvas p-2 font-mono text-xs text-sp-text-secondary">
              {task.scene_ids.map((id, idx) => (
                <div key={idx} className="truncate" title={id}>
                  {id}
                </div>
              ))}
            </div>
          </div>
        )}

        {showTimestamps && (task.created_at || task.updated_at) && (
          <div className="pt-2 border-t border-sp-divider text-xs text-sp-text-muted flex justify-between font-mono">
            {task.created_at && (
              <span>
                {t("downloads.drawer.createdAtLabel", { time: formatTaskTimestamp(task.created_at) ?? task.created_at })}
              </span>
            )}
            {task.updated_at && (
              <span>
                {isTaskHistoryRow(task) ? t("downloads.drawer.endedAtLabel") : t("downloads.drawer.updatedAtLabel")}{" "}
                {formatTaskTimestamp(task.updated_at) ?? task.updated_at}
              </span>
            )}
          </div>
        )}

        {task.status === "paused" && !isGacosWebformTask(task) && onResume && (
          <div className="pt-2 border-t border-sp-divider flex justify-end">
            <button
              type="button"
              onClick={handleResumeClick}
              className="flex h-sp-control items-center gap-2 rounded-sp-sm px-3 text-[13px] font-semibold text-sp-accent-strong border border-sp-line hover:bg-sp-hover transition-colors duration-fast ease-sp cursor-pointer"
              aria-label={t("downloads.task.resumeTask")}
            >
              <Play className="h-3.5 w-3.5" />
              <span>{t("downloads.task.resume")}</span>
            </button>
          </div>
        )}

        {/* 来源检索结果与显式追加/历史后继面板 */}
        <div className="pt-3 border-t border-sp-divider">
          <div className="text-xs font-semibold text-sp-text mb-2">
            {t("downloads.sourceBatch.panelTitle")}
          </div>
          <TaskSourceBatchPanel
            taskId={task.task_id}
            taskTitle={task.name}
            taskStatus={task.status}
            taskCategory={task.category}
            datasetLabel={task.dataset}
            sourceProviderId={task.source_provider_id}
            taskType={task.task_type}
            outputDir={outputDir}
            newTaskPolicyReady={Boolean(newTaskPolicy)}
            newTaskConcurrency={newTaskPolicy?.concurrency}
            newTaskDownloadLayout={newTaskPolicy?.layout}
            onHistoricalMutationApplied={onHistoricalMutationApplied}
            compact={true}
          />
        </div>
      </div>
    </div>
  );
}
