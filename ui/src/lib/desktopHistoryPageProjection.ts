import type { CoreTaskSummaryProjection } from "./coreRestoreProjection.ts";
import { mapStoreTaskSummaryToApplicationTask } from "../shell/api/production.ts";
import type { ApplicationTask, StoreTaskSummary, TaskLifecycleStatus } from "../shell/api/types.ts";

export type DesktopValidatedHistoryTaskEntry = {
  archived_at: string;
  task: CoreTaskSummaryProjection;
};

export type DesktopHistoryTaskSource =
  | CoreTaskSummaryProjection
  | DesktopValidatedHistoryTaskEntry;

function isHistoryTaskEntry(
  source: DesktopHistoryTaskSource,
): source is DesktopValidatedHistoryTaskEntry {
  return (
    typeof source === "object" &&
    source !== null &&
    "task" in source &&
    typeof source.task === "object" &&
    source.task !== null
  );
}

function isQueueLifecycleStatus(status: TaskLifecycleStatus): boolean {
  return status === "queued" || status === "running" || status === "paused";
}

/**
 * 将已校验的 CoreTaskSummaryProjection 转换为 StoreTaskSummary。
 * CoreTaskSummaryProjection 未携带的字段（failure / pause_reason / source_task_id / auxiliary_kind）
 * 明确赋值为 null，绝不虚构失败详情或暂停原因。
 */
export function mapCoreTaskSummaryToStoreTaskSummary(
  task: CoreTaskSummaryProjection,
): StoreTaskSummary {
  return {
    task_id: task.task_id,
    batch_id: task.batch_id,
    provider_id: task.provider_id,
    dataset_id: task.dataset_id ?? null,
    concurrency: task.concurrency ?? null,
    task_type: task.task_type ?? null,
    created_at: task.created_at,
    status: task.status,
    revision: task.revision,
    updated_at: task.updated_at,
    selected_count: task.selected_count,
    delivery_requested_count: task.delivery_requested_count ?? null,
    delivery_rejected_count: task.delivery_rejected_count ?? null,
    checked_count: task.checked_count,
    active_count: task.active_count,
    paused_count: task.paused_count,
    queued_count: task.queued_count,
    completed_count: task.completed_count,
    failed_count: task.failed_count,
    skipped_count: task.skipped_count,
    cancelled_count: task.cancelled_count,
    terminal_bytes: task.terminal_bytes,
    active_bytes: task.active_bytes,
    other_bytes: task.other_bytes,
    transferred_bytes: task.transferred_bytes,
    total_bytes: task.total_bytes ?? null,
    source_total_bytes: task.source_total_bytes ?? null,
    bytes_per_second: task.bytes_per_second,
    transfer_active_millis: task.transfer_active_millis,
    failure: null,
    pause_reason: null,
    output_root: task.output_root,
    source_task_id: task.source_task_id ?? null,
    auxiliary_kind: task.auxiliary_kind ?? null,
  };
}

/**
 * 将 loadApplicationTaskHistoryPage 返回的已校验历史任务（或 CoreTaskSummaryProjection）
 * 投影为 ApplicationTask，复用 mapStoreTaskSummaryToApplicationTask 的显示语义，
 * 并显式标记 bytes_reported: false。
 */
export function projectHistoryTaskSummaryToApplicationTask(
  source: DesktopHistoryTaskSource,
): ApplicationTask {
  const summary = isHistoryTaskEntry(source) ? source.task : source;
  const storeSummary = mapCoreTaskSummaryToStoreTaskSummary(summary);
  return {
    ...mapStoreTaskSummaryToApplicationTask(storeSummary),
    bytes_reported: false,
  };
}

export const mapCoreTaskSummaryToApplicationTask =
  projectHistoryTaskSummaryToApplicationTask;

/**
 * 批量将历史分页条目投影为 ApplicationTask 列表，保持原分页顺序。
 */
export function projectHistoryTaskPageToApplicationTasks(
  items: readonly DesktopHistoryTaskSource[],
): ApplicationTask[] {
  return items.map(projectHistoryTaskSummaryToApplicationTask);
}

export const mapHistoryPageItemsToApplicationTasks =
  projectHistoryTaskPageToApplicationTasks;

/**
 * 纯去重合并函数：
 * - 输入最新 overview 任务列表与历史分页任务列表
 * - 按 task_id 去重，若同一 task_id 同时存在于 overview 与历史分页，保留 overview 的较新值
 * - 保留历史分页的原始顺序
 */
export function mergeOverviewAndHistoryPageTasks(
  overviewTasks: readonly ApplicationTask[],
  historyPageTasks: readonly ApplicationTask[],
): ApplicationTask[] {
  const overviewById = new Map<string, ApplicationTask>();
  for (const task of overviewTasks) {
    if (!overviewById.has(task.task_id)) {
      overviewById.set(task.task_id, task);
    }
  }

  const historyPageTaskIds = new Set<string>();
  for (const task of historyPageTasks) {
    historyPageTaskIds.add(task.task_id);
  }

  const merged: ApplicationTask[] = [];
  const emittedTaskIds = new Set<string>();

  for (const task of overviewTasks) {
    if (emittedTaskIds.has(task.task_id)) continue;
    if (!historyPageTaskIds.has(task.task_id) || isQueueLifecycleStatus(task.status)) {
      emittedTaskIds.add(task.task_id);
      merged.push(overviewById.get(task.task_id) ?? task);
    }
  }

  for (const historyTask of historyPageTasks) {
    if (emittedTaskIds.has(historyTask.task_id)) continue;
    emittedTaskIds.add(historyTask.task_id);
    merged.push(overviewById.get(historyTask.task_id) ?? historyTask);
  }

  return merged;
}

export const mergeOverviewWithHistoryPageTasks = mergeOverviewAndHistoryPageTasks;
export const dedupeAndMergeHistoryPageTasks = mergeOverviewAndHistoryPageTasks;
