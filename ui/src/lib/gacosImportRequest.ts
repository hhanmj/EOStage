import type { DownloadArchiveItem, DownloadStatus } from "./bridge.ts";

export type GacosImportRequestChoice = {
  key: string;
  taskId: string;
  itemId: string;
  state: string;
  label: string;
};

type JsonRecord = Record<string, unknown>;

function record(value: unknown): JsonRecord | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? value as JsonRecord
    : null;
}

function text(value: unknown): string {
  return typeof value === "string" ? value.trim() : "";
}

function groupLabel(
  group: JsonRecord | undefined,
  state: string,
  groupCount: number,
): string | undefined {
  if (!group) return undefined;
  const dates = Array.isArray(group.dates)
    ? group.dates.filter((date): date is string => typeof date === "string" && date.trim().length > 0)
    : [];
  const hour = typeof group.hour === "number" && Number.isInteger(group.hour) && group.hour >= 0 && group.hour <= 23
    ? group.hour
    : null;
  const minute = typeof group.minute === "number" && Number.isInteger(group.minute) && group.minute >= 0 && group.minute <= 59
    ? group.minute
    : null;
  const utc = hour === null || minute === null
    ? ""
    : "UTC " + String(hour).padStart(2, "0") + ":" + String(minute).padStart(2, "0");
  const bbox = groupCount > 1 && Array.isArray(group.bbox) && group.bbox.length === 4
    && group.bbox.every((value) => typeof value === "number" && Number.isFinite(value))
    ? "范围 " + group.bbox.map((value) => Number(value).toFixed(4)).join(", ")
    : "";
  const label = [dates.join(", "), utc, bbox, state].filter(Boolean).join(" · ");
  return label || undefined;
}

/**
 * Projects durable request groups into import choices. Frozen groups remain
 * visible when submission attempts are partial; an attempt only overlays the
 * group state. Unknown attempt item IDs are ignored so they cannot create an
 * import choice that the native parent snapshot would reject.
 */
export function collectGacosImportRequestChoices(
  statuses: DownloadStatus[],
  archive: DownloadArchiveItem[],
  submission: unknown,
  currentTaskId: string,
): GacosImportRequestChoice[] {
  const choices: GacosImportRequestChoice[] = [];
  const seen = new Set<string>();
  const add = (
    taskIdValue: unknown,
    itemIdValue: unknown,
    stateValue: unknown,
    labelValue?: unknown,
  ) => {
    const taskId = text(taskIdValue);
    const itemId = text(itemIdValue);
    if (!taskId || !itemId || (currentTaskId && taskId !== currentTaskId)) return;
    const key = taskId + "|" + itemId;
    if (seen.has(key)) return;
    seen.add(key);
    const state = text(stateValue) || "unknown";
    const label = text(labelValue) || "GACOS 申请项目 " + String(choices.length + 1) + " · " + state;
    choices.push({ key, taskId, itemId, state, label });
  };

  const submissionRecord = record(submission);
  const records = submissionRecord && Array.isArray(submissionRecord.records)
    ? submissionRecord.records
    : [];
  const requestGroups = submissionRecord && Array.isArray(submissionRecord.request_groups)
    ? submissionRecord.request_groups
    : [];
  const submissionTaskId = text(submissionRecord?.task_id);
  const taskState = text(submissionRecord?.task_status) || "unknown";
  const recordsByItemId = new Map<string, JsonRecord>();
  for (const value of records) {
    const item = record(value);
    const itemId = text(item?.item_id);
    if (itemId) recordsByItemId.set(itemId, item!);
  }

  if (submissionTaskId && requestGroups.length > 0) {
    // The request_groups list is the frozen membership authority. Records can
    // be partial when a later UTC batch was never sent after a stop/failure.
    for (const value of requestGroups) {
      const group = record(value);
      const itemId = text(group?.item_id);
      if (!group || !itemId) continue;
      const attempt = recordsByItemId.get(itemId);
      const state = text(attempt?.state) || taskState;
      add(submissionTaskId, itemId, state, groupLabel(group, state, requestGroups.length));
    }
  }

  if (choices.length > 0) return choices;
  for (const status of statuses) {
    if (status.approved_worker_operation !== "gacos_manual_request") continue;
    for (const itemId of status.selected_scene_ids ?? []) add(status.task_id, itemId, status.state);
  }
  for (const item of archive) {
    if (item.approved_worker_operation !== "gacos_manual_request") continue;
    for (const itemId of item.scene_ids ?? []) add(item.task_id, itemId, item.status);
  }
  return choices;
}

export function reconcileGacosImportRequestKey(
  currentKey: string,
  choices: readonly GacosImportRequestChoice[],
): string {
  if (choices.length === 1) {
    return choices.some((choice) => choice.key === currentKey)
      ? currentKey
      : choices[0]!.key;
  }
  return choices.some((choice) => choice.key === currentKey) ? currentKey : "";
}
