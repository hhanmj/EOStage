/**
 * EOStage W5-1c Notification Bell State & Event Derivation
 *
 * Pure module without React dependencies.
 * Handles task lifecycle event derivation, unread state persistence,
 * and UI display metadata mapping.
 */
import { t } from "../../i18n/index.ts";
import type { ApplicationTask } from "./types.ts";
import type { ShellRoute, SimpleStorage } from "./shellRoutes.ts";
import { humanTaskLabel } from "./taskCardNaming.ts";
import { pauseReasonLine } from "./taskCardLogic.ts";

export type BellEventKind =
  | "completed"
  | "failed"
  | "needs_confirmation"
  | "stop_all"
  | "disk_low"
  | "release_changelog";

export interface NotificationItem {
  id: string;
  task_id?: string;
  kind: BellEventKind;
  title: string;
  occurred_at: number;
  route: ShellRoute;
  detail?: string[];
}

export interface NotificationState {
  items: NotificationItem[];
  unreadIds: string[];
}

export const NOTIFICATION_STORAGE_KEY = "eostage.shell.notifications";
export const MAX_NOTIFICATIONS_CAP = 50;

/**
 * Derives notification events from task lifecycle transitions.
 * Emits an item when a task enters "completed" or "failed".
 *
 * Invariance rules:
 * - Tasks already in a terminal state in prev must NOT re-emit.
 * - Repeated identical snapshots emit zero events.
 * - Non-status mutations (e.g. done_bytes / speed updates) emit zero events.
 * - Newly observed tasks that appear already in a terminal state emit once.
 * - "needs_confirmation" is defined by the type contract, but no backend source
 *   produces it in this slice; it is never emitted by this function.
 */
export function deriveTaskEvents(
  prev: ApplicationTask[],
  next: ApplicationTask[]
): NotificationItem[] {
  if (!next || next.length === 0) return [];

  const prevMap = new Map<string, ApplicationTask>(
    prev.map((task) => [task.task_id, task])
  );

  const events: NotificationItem[] = [];

  for (const task of next) {
    if (!task || !task.task_id) continue;

    const currentStatus = task.status;
    const isTerminal = currentStatus === "completed" || currentStatus === "failed";
    const isDiskLowPaused = currentStatus === "paused" && task.pause_reason?.kind === "disk_low";

    if (!isTerminal && !isDiskLowPaused) {
      continue;
    }

    const prevTask = prevMap.get(task.task_id);

    if (isTerminal) {
      // If task was already observed in the same terminal status, do not re-emit
      if (prevTask && prevTask.status === currentStatus) {
        continue;
      }

      // Determine event kind
      const kind: BellEventKind = currentStatus === "completed" ? "completed" : "failed";

      // Format human-friendly title using task name or naming helper
      let title = (task.name || "").trim();
      if (!title || title.startsWith("task-") || title.startsWith("task_")) {
        const helper = humanTaskLabel({
          provider_id: task.provider,
          dataset_id: task.dataset,
          task_id: task.task_id,
          selected_count: task.items_count,
        });
        title = helper.title || t("shared.notificationBell.defaultTaskTitle");
      }

      const occurred_at = task.updated_at
        ? new Date(task.updated_at).getTime()
        : Date.now();

      events.push({
        id: `notif-${task.task_id}-${currentStatus}`,
        task_id: task.task_id,
        kind,
        title,
        occurred_at: isNaN(occurred_at) ? Date.now() : occurred_at,
        route: "downloads",
      });
      continue;
    }

    if (isDiskLowPaused) {
      // 如果上一快照已经处于同一磁盘不足暂停状态，不重复发送
      if (prevTask && prevTask.status === "paused" && prevTask.pause_reason?.kind === "disk_low") {
        continue;
      }

      let title = (task.name || "").trim();
      if (!title || title.startsWith("task-") || title.startsWith("task_")) {
        const helper = humanTaskLabel({
          provider_id: task.provider,
          dataset_id: task.dataset,
          task_id: task.task_id,
          selected_count: task.items_count,
        });
        title = helper.title || t("shared.notificationBell.defaultTaskTitle");
      }

      const occurred_at = task.updated_at
        ? new Date(task.updated_at).getTime()
        : Date.now();
      const line = pauseReasonLine(task.pause_reason);

      events.push({
        id: `notif-${task.task_id}-disk_low-${task.updated_at || "unknown"}`,
        task_id: task.task_id,
        kind: "disk_low",
        title,
        occurred_at: isNaN(occurred_at) ? Date.now() : occurred_at,
        route: "downloads",
        detail: line ? [line] : undefined,
      });
    }
  }

  return events;
}

/**
 * Merges newly derived notification events into current notification state.
 * Places newest items at the head and caps the collection at MAX_NOTIFICATIONS_CAP.
 */
export function mergeNotificationEvents(
  current: NotificationState,
  newItems: NotificationItem[]
): NotificationState {
  if (!newItems || newItems.length === 0) {
    return current;
  }

  const existingIds = new Set(current.items.map((i) => i.id));
  const deduplicatedNewItems = newItems.filter((i) => !existingIds.has(i.id));

  if (deduplicatedNewItems.length === 0) {
    return current;
  }

  const mergedItems = [...deduplicatedNewItems, ...current.items].slice(
    0,
    MAX_NOTIFICATIONS_CAP
  );

  const itemIdsSet = new Set(mergedItems.map((i) => i.id));
  const newUnreadIds = new Set([
    ...current.unreadIds,
    ...deduplicatedNewItems.map((i) => i.id),
  ]);

  const filteredUnreadIds = Array.from(newUnreadIds).filter((id) =>
    itemIdsSet.has(id)
  );

  return {
    items: mergedItems,
    unreadIds: filteredUnreadIds,
  };
}

/**
 * Reads persisted notification state from storage.
 * Gracefully returns an empty state on missing or invalid data.
 */
export function readNotificationState(
  storage?: SimpleStorage | null
): NotificationState {
  const emptyState: NotificationState = { items: [], unreadIds: [] };
  if (!storage) return emptyState;

  try {
    const raw = storage.getItem(NOTIFICATION_STORAGE_KEY);
    if (!raw) return emptyState;

    const parsed = JSON.parse(raw);
    if (!parsed || !Array.isArray(parsed.items) || !Array.isArray(parsed.unreadIds)) {
      return emptyState;
    }

    const items: NotificationItem[] = parsed.items.slice(0, MAX_NOTIFICATIONS_CAP);
    const itemIdsSet = new Set(items.map((i) => i.id));
    const unreadIds: string[] = parsed.unreadIds.filter((id: any) =>
      typeof id === "string" && itemIdsSet.has(id)
    );

    return { items, unreadIds };
  } catch {
    return emptyState;
  }
}

/**
 * Writes notification state to storage with a 50-item cap.
 */
export function writeNotificationState(
  storage: SimpleStorage | null | undefined,
  state: NotificationState
): void {
  if (!storage) return;

  try {
    const cappedItems = state.items.slice(0, MAX_NOTIFICATIONS_CAP);
    const itemIdsSet = new Set(cappedItems.map((i) => i.id));
    const cappedUnreadIds = state.unreadIds.filter((id) => itemIdsSet.has(id));

    storage.setItem(
      NOTIFICATION_STORAGE_KEY,
      JSON.stringify({
        items: cappedItems,
        unreadIds: cappedUnreadIds,
      })
    );
  } catch {
    // Failsafe in case of QuotaExceeded or disabled localStorage
  }
}

/**
 * Marks a single notification item as read.
 */
export function markRead(
  current: NotificationState,
  id: string
): NotificationState {
  if (!current.unreadIds.includes(id)) {
    return current;
  }

  return {
    ...current,
    unreadIds: current.unreadIds.filter((unreadId) => unreadId !== id),
  };
}

/**
 * Marks all notification items as read.
 */
export function markAllRead(current: NotificationState): NotificationState {
  if (current.unreadIds.length === 0) {
    return current;
  }

  return {
    ...current,
    unreadIds: [],
  };
}

/**
 * Removes all notifications that are already marked read, retaining unread items.
 */
export function clearReadNotifications(current: NotificationState): NotificationState {
  const unreadSet = new Set(current.unreadIds);
  const remaining = current.items.filter((item) => unreadSet.has(item.id));
  if (remaining.length === current.items.length) {
    return current;
  }
  return {
    items: remaining,
    unreadIds: current.unreadIds.filter((id) => remaining.some((it) => it.id === id)),
  };
}

/**
 * Returns Chinese text label and design token classes for each notification kind.
 */
export function getNotificationKindMeta(kind: BellEventKind): {
  label: string;
  textClass: string;
  bgClass: string;
} {
  switch (kind) {
    case "completed":
      return {
        label: t("shared.notificationBell.labels.completed"),
        textClass: "text-sp-success-text",
        bgClass: "bg-sp-success-bg",
      };
    case "failed":
      return {
        label: t("shared.notificationBell.labels.failed"),
        textClass: "text-sp-danger-text",
        bgClass: "bg-sp-danger-bg",
      };
    case "needs_confirmation":
      return {
        label: t("shared.notificationBell.labels.approval"),
        textClass: "text-sp-warning-text",
        bgClass: "bg-sp-warning-bg",
      };
    case "stop_all":
      return {
        label: t("shared.notificationBell.labels.stoppedAll"),
        textClass: "text-sp-text-secondary",
        bgClass: "bg-sp-card",
      };
    case "disk_low":
      return {
        label: t("shared.notificationBell.labels.diskLow"),
        textClass: "text-sp-warning-text",
        bgClass: "bg-sp-warning-bg",
      };
    case "release_changelog":
      return {
        label: "更新日志",
        textClass: "text-sp-accent",
        bgClass: "bg-sp-accent/10",
      };
    default:
      return {
        label: t("shared.notificationBell.labels.default"),
        textClass: "text-sp-text-secondary",
        bgClass: "bg-sp-card",
      };
  }
}

/**
 * Formats notification timestamp as HH:mm.
 */
export function formatNotificationTime(timestamp: number): string {
  if (!timestamp || isNaN(timestamp)) return "--:--";
  const date = new Date(timestamp);
  const hours = date.getHours().toString().padStart(2, "0");
  const minutes = date.getMinutes().toString().padStart(2, "0");
  return `${hours}:${minutes}`;
}
