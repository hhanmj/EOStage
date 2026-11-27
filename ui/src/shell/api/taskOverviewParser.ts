/**
 * StoreTaskSummary 与 StoreTaskHistoryPage 严格解析器
 * 依据 crates/insar-store/src/core_restore.rs 83-128 行与 436-448 行
 * 与 ui/src/lib/accountSessions.ts 同等严格度
 */

import type {
  StoreTaskSummary,
  StoreTaskHistoryPage,
  StoreTaskHistoryCursor,
  StoreTaskHistoryItem,
  TaskOverview,
  TaskLifecycleStatus,
  TaskFailureSummary,
  TaskPauseReason,
} from "./types.ts";

const VALID_TASK_STATUSES: readonly TaskLifecycleStatus[] = [
  "queued",
  "running",
  "paused",
  "completed",
  "failed",
  "cancelled",
  "interrupted",
];

function isObject(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

function expectString(v: unknown, field: string): string {
  if (typeof v !== "string") {
    throw new Error(`TASK_OVERVIEW_INVALID: ${field} must be string`);
  }
  return v;
}

function expectNullableString(v: unknown, field: string): string | null {
  if (v === undefined || v === null) return null;
  if (typeof v !== "string") {
    throw new Error(`TASK_OVERVIEW_INVALID: ${field} must be string or null`);
  }
  return v;
}

function expectNumber(v: unknown, field: string): number {
  if (typeof v !== "number" || Number.isNaN(v)) {
    throw new Error(`TASK_OVERVIEW_INVALID: ${field} must be number`);
  }
  return v;
}

function expectNullableNumber(v: unknown, field: string): number | null {
  if (v === undefined || v === null) return null;
  if (typeof v !== "number" || Number.isNaN(v)) {
    throw new Error(`TASK_OVERVIEW_INVALID: ${field} must be number or null`);
  }
  return v;
}

export function parseTaskFailureSummary(raw: unknown): TaskFailureSummary | null {
  if (raw === undefined || raw === null || !isObject(raw)) {
    return null;
  }
  const codes = raw.codes;
  const message_zh = raw.message_zh;
  const failed_items = raw.failed_items;
  const total_items = raw.total_items;

  if (!Array.isArray(codes) || !codes.every((c) => typeof c === "string")) {
    return null;
  }
  if (message_zh !== null && typeof message_zh !== "string") {
    return null;
  }
  if (typeof failed_items !== "number" || !Number.isFinite(failed_items)) {
    return null;
  }
  if (typeof total_items !== "number" || !Number.isFinite(total_items)) {
    return null;
  }
  return {
    codes,
    message_zh,
    failed_items,
    total_items,
  };
}

export function parseTaskPauseReason(raw: unknown): TaskPauseReason | null {
  if (!isObject(raw)) return null;
  if (raw.kind !== "disk_low") return null;
  if (typeof raw.free_bytes !== "number" || !Number.isFinite(raw.free_bytes) || raw.free_bytes < 0) {
    return null;
  }
  if (typeof raw.threshold_bytes !== "number" || !Number.isFinite(raw.threshold_bytes) || raw.threshold_bytes < 0) {
    return null;
  }
  let drive: string | null = null;
  if (raw.drive !== undefined && raw.drive !== null) {
    if (typeof raw.drive !== "string") {
      return null;
    }
    const trimmed = raw.drive.trim();
    drive = trimmed.length > 0 ? raw.drive : null;
  }
  return {
    kind: "disk_low",
    free_bytes: raw.free_bytes,
    threshold_bytes: raw.threshold_bytes,
    drive,
  };
}

export function parseStoreTaskSummary(raw: unknown): StoreTaskSummary {
  if (!isObject(raw)) {
    throw new Error("TASK_OVERVIEW_INVALID: StoreTaskSummary must be an object");
  }

  const status = expectString(raw.status, "status") as TaskLifecycleStatus;
  if (!VALID_TASK_STATUSES.includes(status)) {
    throw new Error(`TASK_OVERVIEW_INVALID: invalid status '${raw.status}'`);
  }

  return {
    task_id: expectString(raw.task_id, "task_id"),
    batch_id: expectString(raw.batch_id, "batch_id"),
    provider_id: expectString(raw.provider_id, "provider_id"),
    dataset_id: expectNullableString(raw.dataset_id, "dataset_id"),
    concurrency: expectNullableNumber(raw.concurrency, "concurrency"),
    task_type: expectNullableString(raw.task_type, "task_type"),
    created_at: expectString(raw.created_at, "created_at"),
    status,
    revision: expectNumber(raw.revision, "revision"),
    updated_at: expectString(raw.updated_at, "updated_at"),
    selected_count: expectNumber(raw.selected_count, "selected_count"),
    delivery_requested_count: expectNullableNumber(raw.delivery_requested_count, "delivery_requested_count"),
    delivery_rejected_count: expectNullableNumber(raw.delivery_rejected_count, "delivery_rejected_count"),
    checked_count: expectNumber(raw.checked_count, "checked_count"),
    active_count: expectNumber(raw.active_count, "active_count"),
    paused_count: expectNumber(raw.paused_count, "paused_count"),
    queued_count: expectNumber(raw.queued_count, "queued_count"),
    completed_count: expectNumber(raw.completed_count, "completed_count"),
    failed_count: expectNumber(raw.failed_count, "failed_count"),
    skipped_count: expectNumber(raw.skipped_count, "skipped_count"),
    cancelled_count: expectNumber(raw.cancelled_count, "cancelled_count"),
    terminal_bytes: expectNumber(raw.terminal_bytes, "terminal_bytes"),
    active_bytes: expectNumber(raw.active_bytes, "active_bytes"),
    other_bytes: expectNumber(raw.other_bytes, "other_bytes"),
    transferred_bytes: expectNumber(raw.transferred_bytes, "transferred_bytes"),
    total_bytes: expectNullableNumber(raw.total_bytes, "total_bytes"),
    source_total_bytes: expectNullableNumber(raw.source_total_bytes, "source_total_bytes"),
    bytes_per_second: expectNullableNumber(raw.bytes_per_second, "bytes_per_second") ?? undefined,
    transfer_active_millis: expectNullableNumber(raw.transfer_active_millis, "transfer_active_millis") ?? undefined,
    failure: parseTaskFailureSummary(raw.failure),
    pause_reason: parseTaskPauseReason(raw.pause_reason),
    ...(raw.output_root === undefined || raw.output_root === null
      ? {}
      : { output_root: expectString(raw.output_root, "output_root") }),
    source_task_id: expectNullableString(raw.source_task_id, "source_task_id"),
    auxiliary_kind: expectNullableString(raw.auxiliary_kind, "auxiliary_kind"),
  };
}

export function parseStoreTaskHistoryCursor(raw: unknown): StoreTaskHistoryCursor | null {
  if (raw === undefined || raw === null) return null;
  if (!isObject(raw)) {
    throw new Error("TASK_OVERVIEW_INVALID: cursor must be object or null");
  }
  return {
    archived_at_epoch_ms: expectNumber(raw.archived_at_epoch_ms, "archived_at_epoch_ms"),
    task_id: expectString(raw.task_id, "task_id"),
  };
}

export function parseStoreTaskHistoryItem(raw: unknown): StoreTaskHistoryItem {
  if (!isObject(raw)) {
    throw new Error("TASK_OVERVIEW_INVALID: history item must be object");
  }
  const archived_at = expectString(raw.archived_at, "archived_at");
  const summary = parseStoreTaskSummary(raw);
  return {
    ...summary,
    archived_at,
  };
}

export function parseStoreTaskHistoryPage(raw: unknown): StoreTaskHistoryPage {
  if (!isObject(raw)) {
    throw new Error("TASK_OVERVIEW_INVALID: history page must be object");
  }
  if (!Array.isArray(raw.items)) {
    throw new Error("TASK_OVERVIEW_INVALID: history page items must be array");
  }

  return {
    page_size: expectNumber(raw.page_size, "page_size"),
    has_more: typeof raw.has_more === "boolean" ? raw.has_more : (() => { throw new Error("TASK_OVERVIEW_INVALID: has_more must be boolean"); })(),
    next_cursor: parseStoreTaskHistoryCursor(raw.next_cursor),
    items: raw.items.map(parseStoreTaskHistoryItem),
  };
}

export function parseTaskOverview(raw: unknown): TaskOverview {
  if (!isObject(raw)) {
    throw new Error("TASK_OVERVIEW_INVALID: overview must be object");
  }
  if (!Array.isArray(raw.queue)) {
    throw new Error("TASK_OVERVIEW_INVALID: queue must be array");
  }

  return {
    queue: raw.queue.map(parseStoreTaskSummary),
    history_page: parseStoreTaskHistoryPage(raw.history_page),
  };
}
