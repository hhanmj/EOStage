import { t } from "../../i18n/index.ts";
import type {
  AgentActivityStatus,
  AgentStopAllResult,
  AgentStreamEvent,
} from "../../lib/generated/v3-contracts.ts";
import type { ApplicationTask } from "./types.ts";
import type { NotificationItem } from "./notificationBell.ts";

export interface StopAllAvailabilityOptions {
  liveRunIds: string[];
  agentTaskIds: string[];
  tasks?: ApplicationTask[] | null;
  status?: AgentActivityStatus | null;
}

export interface StopAllAvailabilityResult {
  enabled: boolean;
  reason_zh: string;
  live_runs?: number;
  agent_active_tasks?: number;
}

export interface StopAllSummaryResult {
  title_zh: string;
  lines: string[];
  partial: boolean;
}

const ACTIVE_TASK_STATUSES = new Set([
  "queued",
  "running",
  "paused",
  "interrupted",
]);

/**
 * 从流事件中提取所有 progress_card 建立的 created_task_ids
 * 只收有效非空字符串、去重且保序
 */
export function agentCreatedTaskIds(events: AgentStreamEvent[]): string[] {
  if (!Array.isArray(events)) return [];

  const seen = new Set<string>();
  const ids: string[] = [];

  for (const event of events) {
    if (!event || event.kind !== "progress_card" || !event.payload) continue;
    const taskIds = event.payload.created_task_ids;
    if (Array.isArray(taskIds)) {
      for (const rawId of taskIds) {
        if (typeof rawId === "string") {
          const trimmed = rawId.trim();
          if (trimmed && !seen.has(trimmed)) {
            seen.add(trimmed);
            ids.push(trimmed);
          }
        }
      }
    }
  }

  return ids;
}

/**
 * 全部停止按钮可用性判定
 * 有进行中运行，或 agentTaskIds 中任一在 tasks 里状态为 queued/running/paused/interrupted -> enabled
 * 否则 disabled，reason_zh 为「没有进行中的 Agent 运行或 Agent 建的活动任务」
 * 不在 tasks 里的 id 不算活动（不猜）
 */
export function stopAllAvailability(
  options: StopAllAvailabilityOptions
): StopAllAvailabilityResult {
  const { liveRunIds, agentTaskIds, tasks, status } = options;

  if (status !== undefined && status !== null) {
    const liveRuns = typeof status.live_runs === "number" ? Math.max(0, status.live_runs) : 0;
    const activeTasks = typeof status.agent_active_tasks === "number" ? Math.max(0, status.agent_active_tasks) : 0;
    const enabled = liveRuns + activeTasks > 0;
    return {
      enabled,
      reason_zh: enabled ? "" : t("shared.stopAll.noActiveReason"),
      live_runs: liveRuns,
      agent_active_tasks: activeTasks,
    };
  }

  if (Array.isArray(liveRunIds) && liveRunIds.length > 0) {
    return { enabled: true, reason_zh: "" };
  }

  if (Array.isArray(agentTaskIds) && agentTaskIds.length > 0 && Array.isArray(tasks)) {
    const taskMap = new Map<string, ApplicationTask>();
    for (const t of tasks) {
      if (t && t.task_id) {
        taskMap.set(t.task_id, t);
      }
    }

    for (const id of agentTaskIds) {
      const task = taskMap.get(id);
      if (task && ACTIVE_TASK_STATUSES.has(task.status)) {
        return { enabled: true, reason_zh: "" };
      }
    }
  }

  return {
    enabled: false,
    reason_zh: t("shared.stopAll.noActiveReason"),
  };
}

/**
 * 全部停止结果文本汇总
 * title「已全部停止」（无失败）或「部分未能停止」（有失败）
 * 第一行「取消了 N 个运行、M 个任务」；N=M=0 且无失败时「没有需要停止的运行或任务」
 * 有失败时列出未能停止的运行与任务（任务名在 tasks 中查找 name，找不到回退 task_id，不编造）
 */
export function stopAllSummary(
  result: AgentStopAllResult,
  tasks?: ApplicationTask[] | null
): StopAllSummaryResult {
  const cancelledRuns = Array.isArray(result?.cancelled_runs) ? result.cancelled_runs : [];
  const failedRuns = Array.isArray(result?.failed_runs) ? result.failed_runs : [];
  const cancelledTasks = Array.isArray(result?.cancelled_tasks) ? result.cancelled_tasks : [];
  const failedTasks = Array.isArray(result?.failed_tasks) ? result.failed_tasks : [];

  const hasFailed = failedRuns.length > 0 || failedTasks.length > 0;
  const partial = hasFailed;
  const title_zh = hasFailed ? t("shared.stopAll.titlePartialFailed") : t("shared.stopAll.titleAllStopped");

  const lines: string[] = [];

  if (cancelledRuns.length === 0 && cancelledTasks.length === 0 && !hasFailed) {
    lines.push(t("shared.stopAll.nothingToStopLine"));
  } else {
    lines.push(t("shared.stopAll.cancelledSummaryLine", {
      runs: cancelledRuns.length,
      tasks: cancelledTasks.length,
    }));
  }

  if (failedRuns.length > 0) {
    lines.push(t("shared.stopAll.failedRunsLine", {
      runs: failedRuns.join(t("shared.stopAll.listSeparator")),
    }));
  }

  if (failedTasks.length > 0) {
    const taskMap = new Map<string, string>();
    if (Array.isArray(tasks)) {
      for (const taskItem of tasks) {
        if (taskItem && taskItem.task_id && taskItem.name) {
          const trimmed = taskItem.name.trim();
          if (trimmed) {
            taskMap.set(taskItem.task_id, trimmed);
          }
        }
      }
    }
    const failedLabels = failedTasks.map((id) => taskMap.get(id) || id);
    lines.push(t("shared.stopAll.failedTasksLine", {
      tasks: failedLabels.join(t("shared.stopAll.listSeparator")),
    }));
  }

  return {
    title_zh,
    lines,
    partial,
  };
}

/**
 * 全部停止结果通知条目构造
 */
export function stopAllNotification(
  result: AgentStopAllResult,
  tasks: ApplicationTask[] | null | undefined,
  occurredAt: number,
  idSuffix?: string
): NotificationItem {
  const summary = stopAllSummary(result, tasks);
  return {
    id: `notif-stop-all-${occurredAt}${idSuffix ? `-${idSuffix}` : ""}`,
    kind: "stop_all",
    title: summary.title_zh,
    occurred_at: occurredAt,
    route: "agent",
    detail: summary.lines,
  };
}

function areStringArraysEqual(a?: string[] | null, b?: string[] | null): boolean {
  const arrA = Array.isArray(a) ? a : [];
  const arrB = Array.isArray(b) ? b : [];
  if (arrA.length !== arrB.length) return false;
  for (let i = 0; i < arrA.length; i++) {
    if (arrA[i] !== arrB[i]) return false;
  }
  return true;
}

/**
 * 流事件去重逻辑：
 * 若流事件 payload 与 60 秒内本机刚收到的命令结果逐字段相同，则不重复入铃，否则入铃
 */
export function shouldAddStopAllStreamEvent(
  payload: AgentStopAllResult,
  lastLocal: { result: AgentStopAllResult; at: number } | null | undefined,
  now: number
): boolean {
  if (!lastLocal || !lastLocal.result) {
    return true;
  }

  // 检查是否在 60 秒内 (60_000 ms)
  if (now - lastLocal.at >= 60_000) {
    return true;
  }

  // 逐字段比对
  const sameCancelledRuns = areStringArraysEqual(
    payload.cancelled_runs,
    lastLocal.result.cancelled_runs
  );
  const sameFailedRuns = areStringArraysEqual(
    payload.failed_runs,
    lastLocal.result.failed_runs
  );
  const sameCancelledTasks = areStringArraysEqual(
    payload.cancelled_tasks,
    lastLocal.result.cancelled_tasks
  );
  const sameFailedTasks = areStringArraysEqual(
    payload.failed_tasks,
    lastLocal.result.failed_tasks
  );

  const exactlySame =
    sameCancelledRuns &&
    sameFailedRuns &&
    sameCancelledTasks &&
    sameFailedTasks;

  return !exactlySame;
}
