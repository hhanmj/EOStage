/**
 * C1 任务中心纯逻辑函数与文案定义
 * 依据 C1 契约与切片 3 事实约束
 */

import { t } from "../../i18n/index.ts";

import type {
  TaskLifecycleStatus,
  ApplicationTask,
  TaskItemAction,
} from "./types.ts";

/**
 * 判断任务是否在执行队列区（queued / running / paused）
 */
export function isTaskInQueue(status: TaskLifecycleStatus): boolean {
  return status === "queued" || status === "running" || status === "paused";
}

/**
 * 判断任务是否在历史记录区（completed / failed / cancelled / interrupted）
 */
export function isTaskInHistory(status: TaskLifecycleStatus): boolean {
  return (
    status === "completed" ||
    status === "failed" ||
    status === "cancelled" ||
    status === "interrupted"
  );
}

/**
 * 判断任务是否为 GACOS 网页表单人工申请记录（提交后即视为已提交申请并归入历史）
 */
export function isGacosWebformTask(
  task: Pick<ApplicationTask, "task_id"> & {
    source_provider_id?: string | null;
    provider_id?: string | null;
  }
): boolean {
  return (
    task.source_provider_id === "gacos.webform" ||
    task.provider_id === "gacos.webform" ||
    task.task_id.startsWith("gacos-request-")
  );
}

/**
 * 判断任务在 UI 侧是否归属「历史任务」（终态任务 + 已提交的非运行态 GACOS 申请）
 */
export function isTaskHistoryRow(
  task: Pick<ApplicationTask, "task_id" | "status"> & {
    source_provider_id?: string | null;
    provider_id?: string | null;
  }
): boolean {
  if (isTaskInHistory(task.status)) return true;
  if (isGacosWebformTask(task) && task.status !== "running") return true;
  return false;
}

/**
 * 将任务列表纯函数分区为队列和历史两个集合
 */
export function partitionTasks(tasks: ApplicationTask[]): {
  queue: ApplicationTask[];
  history: ApplicationTask[];
} {
  const queue: ApplicationTask[] = [];
  const history: ApplicationTask[] = [];

  for (const task of tasks) {
    if (isTaskHistoryRow(task)) {
      history.push(task);
    } else if (isTaskInQueue(task.status)) {
      queue.push(task);
    } else {
      history.push(task);
    }
  }

  return { queue, history };
}

/**
 * 严格按照 TaskLifecycleStatus 和输出目录存在性派生可用操作
 * 规则：
 * - 队列中 (queued / running) -> 暂停
 * - 暂停 (paused) -> 继续
 * - 失败 (failed) -> 重试、重新检索后重试、详情、打开目录(若有输出目录)
 * - 完成 (completed) -> 详情、打开目录(若有输出目录)
 * - 已取消/中断 (cancelled / interrupted) -> 重试、详情、打开目录(若有输出目录)
 * - queued / running / paused 也可在有输出目录时打开目录
 */
export function getAvailableTaskActions(
  status: TaskLifecycleStatus,
  hasOutputDir: boolean,
  hasReplayableSearch = true
): TaskItemAction[] {
  switch (status) {
    case "queued":
    case "running":
      return hasOutputDir ? ["pause", "cancel", "open_folder"] : ["pause", "cancel"];
    case "paused":
      return hasOutputDir ? ["resume", "cancel", "open_folder"] : ["resume", "cancel"];
    case "failed": {
      const actions: TaskItemAction[] = hasReplayableSearch
        ? ["retry", "retry_fallback", "details"]
        : ["retry", "details"];
      if (hasOutputDir) {
        actions.push("open_folder");
      }
      return actions;
    }
    case "completed": {
      const actions: TaskItemAction[] = ["details"];
      if (hasOutputDir) {
        actions.push("open_folder");
      }
      return actions;
    }
    case "cancelled":
    case "interrupted":
      return hasOutputDir ? ["retry", "details", "open_folder"] : ["retry", "details"];
    default:
      return [];
  }
}

export function isTaskFallbackEligible(
  task: Pick<ApplicationTask, "task_id" | "status" | "category" | "source_provider_id" | "task_type" | "provider">,
  eligibilityMap?: Record<string, boolean> | null
): boolean {
  if (task.status !== "failed") return false;
  if (
    task.category === "dem_convert" ||
    task.source_provider_id === "gacos.webform" ||
    task.task_type === "nisar_subset"
  ) {
    return false;
  }
  if (eligibilityMap && Object.prototype.hasOwnProperty.call(eligibilityMap, task.task_id)) {
    return Boolean(eligibilityMap[task.task_id]);
  }
  return true;
}

function parseTimestampEpochMs(value?: string | null): number | null {
  if (typeof value !== "string" || value.trim().length === 0) return null;
  const ms = Date.parse(value);
  return Number.isFinite(ms) ? ms : null;
}

/**
 * 置顶任务优先（按置顶列表顺序），其后按创建时间倒序排列（最新创建的任务在最上方）。
 */
export function sortTasksByPinAndCreatedAt<
  T extends Pick<ApplicationTask, "task_id" | "created_at" | "updated_at">,
>(tasks: readonly T[], pinnedTaskIds: readonly string[]): T[] {
  const pinRank = new Map<string, number>();
  pinnedTaskIds.forEach((id, idx) => {
    if (!pinRank.has(id)) pinRank.set(id, idx);
  });
  return [...tasks].sort((a, b) => {
    const rankA = pinRank.get(a.task_id);
    const rankB = pinRank.get(b.task_id);
    if (rankA !== undefined && rankB !== undefined) return rankA - rankB;
    if (rankA !== undefined) return -1;
    if (rankB !== undefined) return 1;

    const createdA = parseTimestampEpochMs(a.created_at) ?? parseTimestampEpochMs(a.updated_at);
    const createdB = parseTimestampEpochMs(b.created_at) ?? parseTimestampEpochMs(b.updated_at);
    if (createdA !== null && createdB !== null && createdA !== createdB) {
      return createdB - createdA;
    }
    if (createdA !== null && createdB === null) return -1;
    if (createdA === null && createdB !== null) return 1;
    return 0;
  });
}

export function formatTaskTimestamp(value?: string | null): string | null {
  if (typeof value !== "string" || value.trim().length === 0) return null;
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return null;
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`;
}

/** 任务的保存目录（任务总览 output_root）；空串或空白视为未知。只用于显示与本机打开。 */
export function taskOutputDir(task: Pick<ApplicationTask, "output_dir"> | null | undefined): string | null {
  const dir = task?.output_dir;
  return typeof dir === "string" && dir.trim().length > 0 ? dir : null;
}

/**
 * 「重新检索后重试」对话框安全文案
 * 严禁承诺续传或复用已下载内容！
 */
export const RETRY_FALLBACK_DIALOG_TEXT = {
  get title() { return t("downloads.fallback.title"); },
  get content() { return t("downloads.fallback.content"); },
  get confirmText() { return t("downloads.fallback.confirmText"); },
  get cancelText() { return t("downloads.task.cancel"); },
} as const;

/**
 * 「清除历史」对话框安全文案
 * 必须说明只清除历史记录，不删除已下载的文件
 */
export const CLEAR_HISTORY_DIALOG_TEXT = {
  get title() { return t("downloads.clearHistory.title"); },
  get content() { return t("downloads.clearHistory.content"); },
  get confirmText() { return t("downloads.clearHistory.confirmText"); },
  get cancelText() { return t("downloads.task.cancel"); },
} as const;

/**
 * Harness 演示台访问守卫谓词
 */
export function isHarnessAccessible(isDev: boolean): boolean {
  return Boolean(isDev);
}
