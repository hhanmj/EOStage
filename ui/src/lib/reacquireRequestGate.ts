/**
 * 重新下载请求防护门 (Reacquire Request Gate)
 *
 * 核心目标与安全性保护：
 * 1. 冻结与持久化：在向应用服务发出重新下载命令前，将包含 event_id 或 new_task_id 的
 *    完整请求对象持久化至 localStorage，按 sourceTaskId 索引。
 * 2. 避免盲目重试：请求异常、断网或返回 ok:false 时，提示结果待确认并严格保留记录；
 *    用户再次操作时仅允许按原冻结请求（相同 ID/参数）重试，绝不允许生成新 ID 盲重试。
 * 3. 严格成功清理：仅当收到直接可信的 ok:true 响应，且 core.tasks 包含目标 task_id 时才清理。
 *    清理时必须比对当前执行请求与持久化请求的完整一致性；失败、切源或组件卸载均予以保留。
 * 4. 同步 in-flight 锁与代际隔离：同 WebView 内各实例共享模块级同步锁防止双击，
 *    维护 source 成功代际，确保迟到响应和旧确认失效且不污染新任务。
 * 5. Fail-Closed 原则：存储读写错误或数据损坏时严格 fail-closed，锁定操作，不提供无依据的强制解除。
 *
 * 跨 WebView 局限性说明：
 * localStorage 受浏览器/WebView 同源策略保护，在当前 WebView 内受单线程 JS 事件循环与
 * 模块级同步锁保护，可严格保证进程内互斥。但 localStorage API 本身不提供跨独立进程/跨多 WebView
 * 窗口的原子性比较并交换（Compare-And-Swap）或分布式排他锁。若用户在多个独立打开的 WebView
 * 窗口中针对同一 sourceTaskId 并发触发重新下载，存在跨窗口并发竞争窗口，
 * 后端版本检查不能阻止所有不同目标 ID 的历史后继。
 */

import type { DownloadLayout } from "@/lib/generated/v3-contracts";
import { getStorage } from "@/lib/webStorage";
import { acquireSourceOperation, hasPendingNisarRecovery, isSourceOperationInFlight, noteSourceOperationSuccess,
  releaseSourceOperation, sourceOperationEpoch, subscribeSourceOperation } from "@/lib/sourceOperationCoordination";

export type ActiveReacquireRequest = {
  kind: "active";
  task_id: string;
  expected_revision: number;
  command: { kind: "reacquire_items"; item_ids: string[] };
  event_id: string;
  occurred_at: string;
};

export type TerminalReacquireRequest = {
  kind: "terminal";
  source_task_id: string;
  expected_source_task_revision: number;
  new_task_id: string;
  item_ids: [string, ...string[]];
  output_root: string;
  layout: DownloadLayout;
  concurrency: number;
  created_at: string;
  reacquire_item_ids: string[];
};

export type FrozenReacquireRequest = ActiveReacquireRequest | TerminalReacquireRequest;

export type ReacquireRecord = {
  sourceTaskId: string;
  itemId: string;
  displayName: string;
  targetTaskId: string;
  request: FrozenReacquireRequest;
  status: "pending_confirmation";
  lastError?: string | null;
  createdAt: string;
};

const STORAGE_KEY = "eostage.reacquireRequestGate.v1";

let storageCorrupted = false;
let storageCorruptReason: string | null = null;

function areStringArraysEqual(a: readonly string[], b: readonly string[]): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i += 1) {
    if (a[i] !== b[i]) return false;
  }
  return true;
}

export function areRequestsEqual(
  a: FrozenReacquireRequest,
  b: FrozenReacquireRequest,
): boolean {
  if (a.kind !== b.kind) return false;
  if (a.kind === "active" && b.kind === "active") {
    return (
      a.task_id === b.task_id &&
      a.expected_revision === b.expected_revision &&
      a.event_id === b.event_id &&
      a.occurred_at === b.occurred_at &&
      a.command.kind === b.command.kind &&
      areStringArraysEqual(a.command.item_ids, b.command.item_ids)
    );
  }
  if (a.kind === "terminal" && b.kind === "terminal") {
    return (
      a.source_task_id === b.source_task_id &&
      a.expected_source_task_revision === b.expected_source_task_revision &&
      a.new_task_id === b.new_task_id &&
      a.output_root === b.output_root &&
      a.layout === b.layout &&
      a.concurrency === b.concurrency &&
      a.created_at === b.created_at &&
      areStringArraysEqual(a.item_ids, b.item_ids) &&
      areStringArraysEqual(a.reacquire_item_ids, b.reacquire_item_ids)
    );
  }
  return false;
}

function validateRecord(entry: unknown): ReacquireRecord | null {
  if (!entry || typeof entry !== "object") return null;
  const raw = entry as Record<string, unknown>;

  if (typeof raw.sourceTaskId !== "string" || !raw.sourceTaskId || raw.sourceTaskId.length > 128) return null;
  if (typeof raw.itemId !== "string" || !raw.itemId || raw.itemId.length > 512) return null;
  if (typeof raw.displayName !== "string" || raw.displayName.length > 1024) return null;
  if (typeof raw.targetTaskId !== "string" || !raw.targetTaskId || raw.targetTaskId.length > 128) return null;
  if (raw.status !== "pending_confirmation") return null;
  if (typeof raw.createdAt !== "string" || !raw.createdAt) return null;
  if (raw.lastError !== undefined && raw.lastError !== null && typeof raw.lastError !== "string") return null;

  const req = raw.request;
  if (!req || typeof req !== "object") return null;
  const rawReq = req as Record<string, unknown>;

  if (rawReq.kind === "active") {
    if (rawReq.task_id !== raw.sourceTaskId) return null;
    if (!Number.isSafeInteger(rawReq.expected_revision) || (rawReq.expected_revision as number) < 0) return null;
    if (typeof rawReq.event_id !== "string" || !rawReq.event_id || rawReq.event_id.length > 128) return null;
    if (typeof rawReq.occurred_at !== "string" || !rawReq.occurred_at) return null;
    const cmd = rawReq.command as Record<string, unknown> | undefined;
    if (!cmd || cmd.kind !== "reacquire_items" || !Array.isArray(cmd.item_ids)) return null;
    if (cmd.item_ids.length !== 1 || cmd.item_ids[0] !== raw.itemId) return null;
    if (raw.targetTaskId !== raw.sourceTaskId) return null;

    return {
      sourceTaskId: raw.sourceTaskId,
      itemId: raw.itemId,
      displayName: raw.displayName,
      targetTaskId: raw.targetTaskId,
      status: "pending_confirmation",
      createdAt: raw.createdAt,
      lastError: (raw.lastError as string | undefined) ?? null,
      request: {
        kind: "active",
        task_id: rawReq.task_id as string,
        expected_revision: rawReq.expected_revision as number,
        event_id: rawReq.event_id as string,
        occurred_at: rawReq.occurred_at as string,
        command: {
          kind: "reacquire_items",
          item_ids: [...(cmd.item_ids as string[])],
        },
      },
    };
  }

  if (rawReq.kind === "terminal") {
    if (rawReq.source_task_id !== raw.sourceTaskId) return null;
    if (!Number.isSafeInteger(rawReq.expected_source_task_revision) || (rawReq.expected_source_task_revision as number) < 0) return null;
    if (typeof rawReq.new_task_id !== "string" || !rawReq.new_task_id || rawReq.new_task_id.length > 128) return null;
    if (rawReq.new_task_id !== raw.targetTaskId) return null;
    if (!Array.isArray(rawReq.item_ids) || rawReq.item_ids.length !== 1 || rawReq.item_ids[0] !== raw.itemId) return null;
    if (typeof rawReq.output_root !== "string" || !rawReq.output_root) return null;
    if (!["direct", "sentinel1_slc", "sentinel_orbit_aux_poeorb", "sar_dataset_processing"].includes(rawReq.layout as string)) return null;
    if (rawReq.concurrency !== 1) return null;
    if (typeof rawReq.created_at !== "string" || !rawReq.created_at) return null;
    if (!Array.isArray(rawReq.reacquire_item_ids) || rawReq.reacquire_item_ids.length !== 1 || rawReq.reacquire_item_ids[0] !== raw.itemId) return null;

    return {
      sourceTaskId: raw.sourceTaskId,
      itemId: raw.itemId,
      displayName: raw.displayName,
      targetTaskId: raw.targetTaskId,
      status: "pending_confirmation",
      createdAt: raw.createdAt,
      lastError: (raw.lastError as string | undefined) ?? null,
      request: {
        kind: "terminal",
        source_task_id: rawReq.source_task_id as string,
        expected_source_task_revision: rawReq.expected_source_task_revision as number,
        new_task_id: rawReq.new_task_id as string,
        item_ids: [raw.itemId],
        output_root: rawReq.output_root as string,
        layout: rawReq.layout as DownloadLayout,
        concurrency: rawReq.concurrency as number,
        created_at: rawReq.created_at as string,
        reacquire_item_ids: [...(rawReq.reacquire_item_ids as string[])],
      },
    };
  }

  return null;
}

function loadRecords(): Map<string, ReacquireRecord> {
  try {
    const raw = getStorage().getItem(STORAGE_KEY);
    if (raw === null) {
      storageCorrupted = false;
      storageCorruptReason = null;
      return new Map();
    }
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) {
      storageCorrupted = true;
      storageCorruptReason = "重新下载门状态格式错误：根节点非数组结构";
      return new Map();
    }
    const loaded = new Map<string, ReacquireRecord>();
    for (const item of parsed) {
      const validated = validateRecord(item);
      if (!validated) {
        storageCorrupted = true;
        storageCorruptReason = "重新下载门状态校验失败：检测到无效或损坏的请求记录";
        return new Map();
      }
      if (loaded.has(validated.sourceTaskId)) {
        storageCorrupted = true;
        storageCorruptReason = `重新下载门状态重复：sourceTaskId(${validated.sourceTaskId})存在重复项`;
        return new Map();
      }
      loaded.set(validated.sourceTaskId, validated);
    }
    storageCorrupted = false;
    storageCorruptReason = null;
    return loaded;
  } catch (error) {
    storageCorrupted = true;
    storageCorruptReason = `读取重新下载门状态失败：${error instanceof Error ? error.message : String(error)}`;
    return new Map();
  }
}

const records = loadRecords();
const listeners = new Set<() => void>();
let gateVersion = 0;

function notify(): void {
  gateVersion += 1;
  listeners.forEach((listener) => {
    try {
      listener();
    } catch {
      // 保护订阅者异常不中断
    }
  });
}

export function reacquireGateVersion(): number { return gateVersion; }

function persistRecords(): boolean {
  if (storageCorrupted) return false;
  try {
    const serialized = JSON.stringify([...records.values()]);
    getStorage().setItem(STORAGE_KEY, serialized);
    return true;
  } catch (error) {
    storageCorrupted = true;
    storageCorruptReason = `持久化重新下载门状态失败：${error instanceof Error ? error.message : String(error)}`;
    notify();
    return false;
  }
}

export function isStorageCorrupted(): { corrupted: boolean; reason: string | null } {
  return { corrupted: storageCorrupted, reason: storageCorruptReason };
}

export function subscribeReacquireGate(listener: () => void): () => void {
  listeners.add(listener);
  const unsubscribeOperation = subscribeSourceOperation(listener);
  return () => {
    listeners.delete(listener);
    unsubscribeOperation();
  };
}

export function getPendingReacquireRecord(sourceTaskId: string): ReacquireRecord | null {
  if (storageCorrupted) return null;
  return records.get(sourceTaskId) ?? null;
}

export function tryAcquireInFlightLock(sourceTaskId: string): boolean {
  return acquireSourceOperation(sourceTaskId);
}

export function releaseInFlightLock(sourceTaskId: string): void {
  releaseSourceOperation(sourceTaskId);
}

export function isInFlight(sourceTaskId: string): boolean {
  return isSourceOperationInFlight(sourceTaskId);
}

export function getSourceSuccessEpoch(sourceTaskId: string): number {
  return sourceOperationEpoch(sourceTaskId);
}

export function stageReacquireRequest(
  record: ReacquireRecord,
): { ok: true } | { ok: false; error: string } {
  if (storageCorrupted) {
    return {
      ok: false,
      error: storageCorruptReason ?? "本地重新下载保护存储已损坏，已锁定操作，禁止发起请求。",
    };
  }

  if (!validateRecord(record)) {
    return { ok: false, error: "重新下载请求记录无效，已拒绝执行。" };
  }

  const existing = records.get(record.sourceTaskId);
  if (existing) {
    if (areRequestsEqual(existing.request, record.request)) {
      // 同一冻结请求重试，保留原记录
      return { ok: true };
    }
    return {
      ok: false,
      error:
        `当前任务存在尚未确认的重新下载请求 (目标 ID: ${existing.targetTaskId})。` +
        "为防止重复下载或覆盖，不允许生成新 ID 盲重试，必须重试原目标或等待系统确认。",
    };
  }

  if (hasPendingNisarRecovery(record.sourceTaskId)) {
    return { ok: false, error: "该来源有待确认的 NISAR 恢复请求，已阻止新的重新下载。" };
  }

  records.set(record.sourceTaskId, record);
  if (!persistRecords()) {
    records.delete(record.sourceTaskId);
    return {
      ok: false,
      error: storageCorruptReason ?? "持久化重新下载请求失败（本地存储不可用或已满），已拒绝执行。",
    };
  }

  notify();
  return { ok: true };
}

export function updateReacquireLastError(sourceTaskId: string, error: string): void {
  if (storageCorrupted) return;
  const existing = records.get(sourceTaskId);
  if (!existing) return;
  // This is display text, not request identity. Bound new errors without
  // rejecting or rewriting historical records when they are loaded.
  existing.lastError = error.slice(0, 2048);
  persistRecords();
  notify();
}

export function tryCommitReacquireSuccess(
  sourceTaskId: string,
  expectedRequest: FrozenReacquireRequest,
  responseSnapshot: unknown,
): { committed: boolean; reason?: string } {
  if (storageCorrupted) {
    return { committed: false, reason: "存储已处于损坏锁定状态" };
  }

  const existing = records.get(sourceTaskId);
  if (!existing) {
    return { committed: false, reason: "未找到待确认的重新下载请求记录" };
  }

  if (!areRequestsEqual(existing.request, expectedRequest)) {
    return { committed: false, reason: "当前提交的请求与待确认记录不一致，拒绝清理" };
  }

  if (!responseSnapshot || typeof responseSnapshot !== "object") {
    return { committed: false, reason: "服务响应结构无效" };
  }

  const snapshot = responseSnapshot as Record<string, unknown>;
  if (snapshot.ok !== true) {
    return { committed: false, reason: "服务响应未返回直接成功状态 (ok !== true)" };
  }

  const core = snapshot.core;
  if (!core || typeof core !== "object") {
    return { committed: false, reason: "服务响应缺少 core 状态投影" };
  }

  const tasks = (core as Record<string, unknown>).tasks;
  if (!Array.isArray(tasks)) {
    return { committed: false, reason: "服务响应 core.tasks 不是有效数组" };
  }

  const foundTarget = tasks.some(
    (item) =>
      item &&
      typeof item === "object" &&
      (item as Record<string, unknown>).task_id === existing.targetTaskId,
  );

  if (!foundTarget) {
    return {
      committed: false,
      reason: `服务响应中 core.tasks 未包含目标任务 ID (${existing.targetTaskId})`,
    };
  }

  records.delete(sourceTaskId);
  if (!persistRecords()) {
    // 写入失败时恢复，保持 fail-closed
    records.set(sourceTaskId, existing);
    return { committed: false, reason: "清理记录时写入 localStorage 失败" };
  }

  noteSourceOperationSuccess(sourceTaskId);
  notify();
  return { committed: true };
}
