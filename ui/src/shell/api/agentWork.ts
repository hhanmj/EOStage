/**
 * 「任务与成果」：本对话各次运行（AgentWorkItem）与下载任务列表的纯逻辑。
 * 任务状态只取自工作台已轮询的任务列表；列表里找不到的任务视为已清理，不猜测状态。
 */
import { t } from "../../i18n/index.ts";
import type { AgentRunStatus, AgentWorkItem } from "../../lib/generated/v3-contracts.ts";
import type { ApplicationTask } from "./types.ts";

export type AgentWorkFilter = "all" | "active" | "ended";

export interface AgentWorkTaskRow {
  task_id: string;
  /** null：任务不在当前任务列表中（已清理） */
  task: ApplicationTask | null;
}

export function isRunActive(status: AgentRunStatus | string): boolean {
  return status === "running";
}

function startedMs(item: AgentWorkItem): number | null {
  const ms = Date.parse(item.run.started_at);
  return Number.isNaN(ms) ? null : ms;
}

/** 新的运行在前；时间无法解析时保持后端给出的顺序。 */
export function sortWorkNewestFirst(items: readonly AgentWorkItem[]): AgentWorkItem[] {
  return items
    .map((item, index) => ({ item, index, ms: startedMs(item) }))
    .sort((a, b) => {
      if (a.ms !== null && b.ms !== null && a.ms !== b.ms) return b.ms - a.ms;
      return a.index - b.index;
    })
    .map((entry) => entry.item);
}

export function filterWork(items: readonly AgentWorkItem[], filter: AgentWorkFilter): AgentWorkItem[] {
  if (filter === "all") return [...items];
  const wantActive = filter === "active";
  return items.filter((item) => isRunActive(item.run.status) === wantActive);
}

export function workCounts(items: readonly AgentWorkItem[]): Record<AgentWorkFilter, number> {
  const active = items.filter((item) => isRunActive(item.run.status)).length;
  return { all: items.length, active, ended: items.length - active };
}

/** 本对话创建的下载任务数（去重）。 */
export function createdTaskCount(items: readonly AgentWorkItem[] | null | undefined): number {
  if (!items) return 0;
  const ids = new Set<string>();
  for (const item of items) for (const id of item.created_task_ids) ids.add(id);
  return ids.size;
}

export function joinRunTasks(item: AgentWorkItem, tasks: readonly ApplicationTask[]): AgentWorkTaskRow[] {
  const byId = new Map(tasks.map((task) => [task.task_id, task]));
  const seen = new Set<string>();
  const rows: AgentWorkTaskRow[] = [];
  for (const id of item.created_task_ids) {
    if (seen.has(id)) continue;
    seen.add(id);
    rows.push({ task_id: id, task: byId.get(id) ?? null });
  }
  return rows;
}

/** 运行时长（毫秒）：进行中的运行按 nowMs 计；时间缺失或无法解析返回 null。 */
export function runDurationMs(item: AgentWorkItem, nowMs: number): number | null {
  const start = startedMs(item);
  if (start === null) return null;
  let end: number;
  if (item.run.ended_at) {
    end = Date.parse(item.run.ended_at);
    if (Number.isNaN(end)) return null;
  } else if (isRunActive(item.run.status)) {
    end = nowMs;
  } else {
    return null;
  }
  return Math.max(0, end - start);
}

export function formatDuration(ms: number | null): string | null {
  if (ms === null || !Number.isFinite(ms) || ms < 0) return null;
  const total = Math.floor(ms / 1000);
  if (total < 60) return t("shared.agentWork.durationSeconds", { total });
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  if (minutes < 60) {
    return t("shared.agentWork.durationMinutesSeconds", {
      minutes,
      seconds: String(seconds).padStart(2, "0"),
    });
  }
  const hours = Math.floor(minutes / 60);
  return t("shared.agentWork.durationHoursMinutes", {
    hours,
    minutes: String(minutes % 60).padStart(2, "0"),
  });
}

export function workTitle(item: AgentWorkItem): string {
  const text = item.request?.trim();
  return text ? text : t("shared.agentWork.unrecordedRequest");
}

export function taskStatusLabel(task: ApplicationTask | null): string {
  if (!task) return t("shared.agentWork.taskCleaned");
  switch (task.status) {
    case "queued": return t("shared.agentWork.taskStatuses.queued");
    case "running": return t("shared.agentWork.taskStatuses.running");
    case "paused": return t("shared.agentWork.taskStatuses.paused");
    case "completed": return t("shared.agentWork.taskStatuses.completed");
    case "failed": return t("shared.agentWork.taskStatuses.failed");
    case "cancelled": return t("shared.agentWork.taskStatuses.cancelled");
    case "interrupted": return t("shared.agentWork.taskStatuses.interrupted");
    default: return String(task.status);
  }
}

/** 只有后端给出有意义的总量时才是确定进度；已完成为 100。 */
export function taskProgressPercent(task: ApplicationTask | null): number | null {
  if (!task) return null;
  if (task.status === "completed") return 100;
  const total = task.total_bytes;
  if (typeof total !== "number" || total <= 0) return null;
  return Math.min(100, Math.max(0, Math.round((task.done_bytes / total) * 100)));
}
