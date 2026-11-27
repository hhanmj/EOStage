import type { DownloadLayout } from "@/lib/generated/v3-contracts";
import type { DesktopNisarAppendSubsetBatchTaskV2Request, DesktopNisarSuccessorTaskV2Request } from "@/lib/desktopShellBridge";
import { getStorage } from "@/lib/webStorage";
import { acquireSourceOperation, hasPendingNisarRecovery, isSourceOperationInFlight, noteSourceOperationSuccess,
  releaseSourceOperation, sourceOperationEpoch, subscribeSourceOperation } from "@/lib/sourceOperationCoordination";

export const VALID_DOWNLOAD_LAYOUTS: ReadonlySet<DownloadLayout> = new Set<DownloadLayout>([
  "direct",
  "sentinel1_slc",
  "sentinel_orbit_aux_poeorb",
  "sar_dataset_processing",
]);

export type FrozenAppendSourceBatchMutation = {
  kind: "append_source_batch";
  request: {
    task_id: string;
    expected_task_revision: number;
    item_ids: [string, ...string[]];
    event_id: string;
    occurred_at: string;
  };
};

export type FrozenBurstAppendMutation = {
  kind: "burst_append";
  request: {
    request_id: string;
    task_id: string;
    selected_product_ids: string[];
    expected_task_revision: number;
    event_id: string;
    occurred_at: string;
  };
};

export type FrozenSuccessorSourceBatchMutation = {
  kind: "successor_source_batch";
  request: {
    source_task_id: string;
    expected_source_task_revision: number;
    new_task_id: string;
    item_ids: [string, ...string[]];
    output_root: string;
    layout: DownloadLayout;
    concurrency: number;
    created_at: string;
  };
};

export type FrozenBurstSuccessorMutation = {
  kind: "burst_successor";
  request: {
    request_id: string;
    source_task_id: string;
    expected_source_task_revision: number;
    selected_product_ids: string[];
    new_task_id: string;
    output_root: string;
    layout: "direct";
    concurrency: number;
    created_at: string;
  };
};

export type FrozenSourceMutation =
  | FrozenAppendSourceBatchMutation
  | FrozenBurstAppendMutation
  | FrozenSuccessorSourceBatchMutation
  | FrozenBurstSuccessorMutation
  | { kind: "nisar_append"; request: DesktopNisarAppendSubsetBatchTaskV2Request }
  | { kind: "nisar_successor"; request: DesktopNisarSuccessorTaskV2Request };

/** Local guard shared by the download page and drawer, restored on restart. */
export type SourceMutationGate = {
  sourceTaskId: string;
  targetTaskId: string;
  sourceBatchId: string;
  itemIds: string[];
  minRevision: number;
  pending: boolean;
  directSuccess?: boolean;
  error?: string;
  frozenMutation?: FrozenSourceMutation;
};

export function isNisarSourceMutation(gate: SourceMutationGate): boolean {
  return gate.frozenMutation?.kind === "nisar_append" || gate.frozenMutation?.kind === "nisar_successor";
}

/** A direct response must contain the intended target before local release. */
export function nisarDirectResponseContainsTarget(gate: SourceMutationGate, result: unknown): boolean {
  if (!isNisarSourceMutation(gate) || !result || typeof result !== "object") return false;
  const response = result as Record<string, unknown>;
  if (response.ok !== true || !response.core || typeof response.core !== "object") return false;
  const tasks = (response.core as Record<string, unknown>).tasks;
  if (!Array.isArray(tasks)) return false;
  const target = tasks.find((task: unknown): task is Record<string, unknown> =>
    !!task && typeof task === "object" && (task as Record<string, unknown>).task_id === gate.targetTaskId,
  );
  if (!target) return false;
  if (gate.frozenMutation?.kind === "nisar_append") {
    return typeof target.revision === "number"
      && Number.isSafeInteger(target.revision)
      && target.revision >= gate.minRevision;
  }
  return true;
}

const storageKey = "eostage.sourceMutationGate.v1";
let storageReady = true;

const GATE_ALLOWED_KEYS: ReadonlySet<string> = new Set([
  "sourceTaskId",
  "targetTaskId",
  "sourceBatchId",
  "itemIds",
  "minRevision",
  "pending",
  "directSuccess",
  "error",
  "frozenMutation",
]);

const FROZEN_MUTATION_KEYS = ["kind", "request"] as const;
const APPEND_REQUEST_KEYS = ["task_id", "expected_task_revision", "item_ids", "event_id", "occurred_at"] as const;
const BURST_APPEND_REQUEST_KEYS = ["request_id", "task_id", "selected_product_ids", "expected_task_revision", "event_id", "occurred_at"] as const;
const SUCCESSOR_REQUEST_KEYS = ["source_task_id", "expected_source_task_revision", "new_task_id", "item_ids", "output_root", "layout", "concurrency", "created_at"] as const;
const BURST_SUCCESSOR_REQUEST_KEYS = ["request_id", "source_task_id", "expected_source_task_revision", "selected_product_ids", "new_task_id", "output_root", "layout", "concurrency", "created_at"] as const;

function hasExactKeys(obj: Record<string, unknown>, requiredKeys: readonly string[]): boolean {
  const keys = Object.keys(obj);
  if (keys.length !== requiredKeys.length) return false;
  for (let index = 0; index < requiredKeys.length; index += 1) {
    if (!Object.prototype.hasOwnProperty.call(obj, requiredKeys[index])) return false;
  }
  return true;
}

function hasOnlyAllowedKeys(obj: Record<string, unknown>, allowedKeys: ReadonlySet<string>): boolean {
  const keys = Object.keys(obj);
  for (let index = 0; index < keys.length; index += 1) {
    if (!allowedKeys.has(keys[index])) return false;
  }
  return true;
}

function isValidString(val: unknown, maxLen = 512): val is string {
  return typeof val === "string" && val.length > 0 && val.length <= maxLen;
}

function isStrictUniqueStringArray(
  val: unknown,
  min = 1,
  max = 250,
  maxLen = 512,
): val is string[] {
  if (!Array.isArray(val) || val.length < min || val.length > max) return false;
  const seen = new Set<string>();
  for (let index = 0; index < val.length; index += 1) {
    const item = val[index];
    if (typeof item !== "string" || !item || item.length > maxLen) return false;
    if (seen.has(item)) return false;
    seen.add(item);
  }
  return true;
}

function areStrictEqualStringArrays(expected: string[], actual: unknown): boolean {
  if (!Array.isArray(actual) || expected.length !== actual.length) return false;
  for (let index = 0; index < expected.length; index += 1) {
    if (expected[index] !== actual[index]) return false;
  }
  return true;
}

type GateContext = {
  sourceTaskId: string;
  targetTaskId: string;
  sourceBatchId: string;
  itemIds: string[];
  minRevision: number;
};

function parseAndValidateFrozenMutation(
  raw: unknown,
  gate: GateContext,
): FrozenSourceMutation | null | "invalid" {
  if (raw === undefined || raw === null) return null;
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return "invalid";
  const entry = raw as Record<string, unknown>;
  if (!hasExactKeys(entry, FROZEN_MUTATION_KEYS)) return "invalid";
  if (typeof entry.kind !== "string" || !entry.request || typeof entry.request !== "object" || Array.isArray(entry.request)) return "invalid";
  const req = entry.request as Record<string, unknown>;

  // These checks preserve stored identity; the bridge and service own admission.
  if (entry.kind === "nisar_append" || entry.kind === "nisar_successor") {
    const append = entry.kind === "nisar_append";
    const keys = append
      ? ["task_id", "expected_task_revision", "source_item_ids", "event_id", "occurred_at"]
      : ["source_task_id", "expected_task_revision", "source_item_ids", "new_task_id", "output_root", "layout", "concurrency", "created_at", "recreate_original_items"];
    if (!hasExactKeys(req, keys)
      || typeof req.expected_task_revision !== "number"
      || !Number.isSafeInteger(req.expected_task_revision) || req.expected_task_revision < 0
      || !areStrictEqualStringArrays(gate.itemIds, req.source_item_ids)) return "invalid";
    if (append) {
      if (req.task_id !== gate.sourceTaskId || gate.targetTaskId !== gate.sourceTaskId
        || gate.minRevision !== req.expected_task_revision + 1
        || !isValidString(req.event_id, 512) || !isValidString(req.occurred_at, 128)) return "invalid";
      return { kind: "nisar_append", request: { task_id: gate.sourceTaskId,
        expected_task_revision: req.expected_task_revision, source_item_ids: [...gate.itemIds],
        event_id: req.event_id, occurred_at: req.occurred_at } };
    }
    if (req.source_task_id !== gate.sourceTaskId || req.new_task_id !== gate.targetTaskId
      || gate.sourceTaskId === gate.targetTaskId || gate.minRevision !== 0
      || typeof req.output_root !== "string" || req.output_root.length > 4096
      || typeof req.layout !== "string" || !VALID_DOWNLOAD_LAYOUTS.has(req.layout as DownloadLayout)
      || typeof req.concurrency !== "number" || !Number.isSafeInteger(req.concurrency)
      || req.concurrency < 1 || req.concurrency > 8 || !isValidString(req.created_at, 128)
      || typeof req.recreate_original_items !== "boolean") return "invalid";
    return { kind: "nisar_successor", request: { source_task_id: gate.sourceTaskId,
      expected_task_revision: req.expected_task_revision, source_item_ids: [...gate.itemIds],
      new_task_id: gate.targetTaskId, output_root: req.output_root, layout: req.layout as DownloadLayout,
      concurrency: req.concurrency, created_at: req.created_at, recreate_original_items: req.recreate_original_items } };
  }

  if (entry.kind === "append_source_batch") {
    if (!hasExactKeys(req, APPEND_REQUEST_KEYS)) return "invalid";
    if (req.task_id !== gate.sourceTaskId
      || gate.targetTaskId !== gate.sourceTaskId
      || typeof req.expected_task_revision !== "number"
      || !Number.isSafeInteger(req.expected_task_revision)
      || req.expected_task_revision < 0
      || gate.minRevision !== req.expected_task_revision + 1
      || !isValidString(req.event_id, 512)
      || !isValidString(req.occurred_at, 128)
      || !areStrictEqualStringArrays(gate.itemIds, req.item_ids)) {
      return "invalid";
    }
    return {
      kind: "append_source_batch",
      request: {
        task_id: req.task_id,
        expected_task_revision: req.expected_task_revision,
        item_ids: [...gate.itemIds] as [string, ...string[]],
        event_id: req.event_id,
        occurred_at: req.occurred_at,
      },
    };
  }

  if (entry.kind === "burst_append") {
    if (!hasExactKeys(req, BURST_APPEND_REQUEST_KEYS)) return "invalid";
    if (!isValidString(req.request_id, 512)
      || req.task_id !== gate.sourceTaskId
      || gate.targetTaskId !== gate.sourceTaskId
      || typeof req.expected_task_revision !== "number"
      || !Number.isSafeInteger(req.expected_task_revision)
      || req.expected_task_revision < 0
      || gate.minRevision !== req.expected_task_revision + 1
      || !isValidString(req.event_id, 512)
      || !isValidString(req.occurred_at, 128)
      || !areStrictEqualStringArrays(gate.itemIds, req.selected_product_ids)) {
      return "invalid";
    }
    return {
      kind: "burst_append",
      request: {
        request_id: req.request_id,
        task_id: req.task_id,
        expected_task_revision: req.expected_task_revision,
        selected_product_ids: [...gate.itemIds],
        event_id: req.event_id,
        occurred_at: req.occurred_at,
      },
    };
  }

  if (entry.kind === "successor_source_batch") {
    if (!hasExactKeys(req, SUCCESSOR_REQUEST_KEYS)) return "invalid";
    if (req.source_task_id !== gate.sourceTaskId
      || req.new_task_id !== gate.targetTaskId
      || gate.sourceTaskId === gate.targetTaskId
      || gate.minRevision !== 0
      || typeof req.expected_source_task_revision !== "number"
      || !Number.isSafeInteger(req.expected_source_task_revision)
      || req.expected_source_task_revision < 0
      || typeof req.output_root !== "string"
      || req.output_root.length > 2048
      || typeof req.layout !== "string"
      || !VALID_DOWNLOAD_LAYOUTS.has(req.layout as DownloadLayout)
      || typeof req.concurrency !== "number"
      || !Number.isSafeInteger(req.concurrency)
      || req.concurrency < 1
      || req.concurrency > 8
      || !isValidString(req.created_at, 128)
      || !areStrictEqualStringArrays(gate.itemIds, req.item_ids)) {
      return "invalid";
    }
    return {
      kind: "successor_source_batch",
      request: {
        source_task_id: req.source_task_id,
        expected_source_task_revision: req.expected_source_task_revision,
        new_task_id: req.new_task_id,
        item_ids: [...gate.itemIds] as [string, ...string[]],
        output_root: req.output_root,
        layout: req.layout as DownloadLayout,
        concurrency: req.concurrency,
        created_at: req.created_at,
      },
    };
  }

  if (entry.kind === "burst_successor") {
    if (!hasExactKeys(req, BURST_SUCCESSOR_REQUEST_KEYS)) return "invalid";
    if (!isValidString(req.request_id, 512)
      || req.source_task_id !== gate.sourceTaskId
      || req.new_task_id !== gate.targetTaskId
      || gate.sourceTaskId === gate.targetTaskId
      || gate.minRevision !== 0
      || typeof req.expected_source_task_revision !== "number"
      || !Number.isSafeInteger(req.expected_source_task_revision)
      || req.expected_source_task_revision < 0
      || typeof req.output_root !== "string"
      || req.output_root.length > 2048
      || req.layout !== "direct"
      || typeof req.concurrency !== "number"
      || !Number.isSafeInteger(req.concurrency)
      || req.concurrency < 1
      || req.concurrency > 8
      || !isValidString(req.created_at, 128)
      || !areStrictEqualStringArrays(gate.itemIds, req.selected_product_ids)) {
      return "invalid";
    }
    return {
      kind: "burst_successor",
      request: {
        request_id: req.request_id,
        source_task_id: req.source_task_id,
        expected_source_task_revision: req.expected_source_task_revision,
        selected_product_ids: [...gate.itemIds],
        new_task_id: req.new_task_id,
        output_root: req.output_root,
        layout: "direct",
        concurrency: req.concurrency,
        created_at: req.created_at,
      },
    };
  }

  return "invalid";
}

export function validateGate(raw: unknown): SourceMutationGate | null {
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return null;
  const gate = raw as Record<string, unknown>;

  if (!hasOnlyAllowedKeys(gate, GATE_ALLOWED_KEYS)) return null;

  if (typeof gate.sourceTaskId !== "string" || !gate.sourceTaskId || gate.sourceTaskId.length > 128
    || typeof gate.targetTaskId !== "string" || !gate.targetTaskId || gate.targetTaskId.length > 128
    || typeof gate.sourceBatchId !== "string" || !gate.sourceBatchId || gate.sourceBatchId.length > 512
    || typeof gate.minRevision !== "number" || !Number.isSafeInteger(gate.minRevision) || gate.minRevision < 0
    || typeof gate.pending !== "boolean") {
    return null;
  }

  if (gate.directSuccess !== undefined && typeof gate.directSuccess !== "boolean") return null;
  if (gate.error !== undefined && (typeof gate.error !== "string" || gate.error.length > 2048)) return null;

  const rawFrozen = gate.frozenMutation;
  const isNisar = !!rawFrozen && typeof rawFrozen === "object" && !Array.isArray(rawFrozen)
    && ["nisar_append", "nisar_successor"].includes(String((rawFrozen as Record<string, unknown>).kind));
  if (!isStrictUniqueStringArray(gate.itemIds, 1, isNisar ? 10_000 : 250, isNisar ? 1024 : 512)) return null;
  const itemIds = [...gate.itemIds];

  const gateContext: GateContext = {
    sourceTaskId: gate.sourceTaskId,
    targetTaskId: gate.targetTaskId,
    sourceBatchId: gate.sourceBatchId,
    itemIds,
    minRevision: gate.minRevision,
  };

  const parsedFrozen = parseAndValidateFrozenMutation(gate.frozenMutation, gateContext);
  if (parsedFrozen === "invalid") return null;

  return {
    sourceTaskId: gate.sourceTaskId,
    targetTaskId: gate.targetTaskId,
    sourceBatchId: gate.sourceBatchId,
    itemIds,
    minRevision: gate.minRevision,
    pending: gate.pending,
    ...(gate.directSuccess === true ? { directSuccess: true } : {}),
    ...(typeof gate.error === "string" ? { error: gate.error } : {}),
    ...(parsedFrozen ? { frozenMutation: parsedFrozen } : {}),
  };
}

function loadGates(): Map<string, SourceMutationGate> {
  try {
    const rawValue = getStorage().getItem(storageKey);
    if (rawValue === null) return new Map();
    const entries: unknown = JSON.parse(rawValue);
    if (!Array.isArray(entries)) {
      storageReady = false;
      return new Map();
    }
    const loaded = new Map<string, SourceMutationGate>();
    for (let index = 0; index < entries.length; index += 1) {
      const validated = validateGate(entries[index]);
      if (!validated || loaded.has(validated.sourceTaskId)) {
        storageReady = false;
        return new Map();
      }
      loaded.set(validated.sourceTaskId, {
        ...validated,
        pending: false,
      });
    }
    return loaded;
  } catch {
    storageReady = false;
    return new Map();
  }
}

const gates = loadGates();

/**
 * Synchronous in-flight lock scoped to the current JavaScript runtime / WebView.
 * Defends against rapid double-clicks and concurrent component submission inside
 * the same view. Note: localStorage is not atomic across multiple WebView instances,
 * so this lock does not claim atomic cross-process mutual exclusion.
 */
const listeners = new Set<() => void>();
let gateVersion = 0;
const notify = () => {
  gateVersion += 1;
  listeners.forEach((listener) => {
    try { listener(); } catch { /* An observer must not interrupt gate updates. */ }
  });
};

export function sourceMutationGateVersion(): number { return gateVersion; }

export function isSourceMutationStorageCorrupted(): boolean {
  return !storageReady;
}

export function sourceMutationEpoch(taskId: string): number {
  return sourceOperationEpoch(taskId);
}

export function isSourceMutationInFlight(taskId: string): boolean {
  return isSourceOperationInFlight(taskId);
}

export function acquireSourceMutationInFlight(taskId: string): boolean {
  return storageReady && acquireSourceOperation(taskId);
}

export function releaseSourceMutationInFlight(taskId: string): void {
  releaseSourceOperation(taskId);
}

export function noteSourceMutationSuccess(gate: SourceMutationGate): void {
  if (gates.get(gate.sourceTaskId) !== gate) return;
  gate.directSuccess = true;
  noteSourceOperationSuccess(gate.sourceTaskId);
  persist();
  notify();
}

function persist(): boolean {
  if (!storageReady) return false;
  try {
    getStorage().setItem(
      storageKey,
      JSON.stringify([...gates.values()].map(({ error: _error, ...gate }) => gate)),
    );
    return true;
  } catch {
    return false;
  }
}

export function getSourceMutationGate(taskId: string): SourceMutationGate | null {
  return gates.get(taskId) ?? null;
}

export function subscribeSourceMutationGate(listener: () => void): () => void {
  listeners.add(listener);
  const unsubscribeOperation = subscribeSourceOperation(listener);
  return () => { listeners.delete(listener); unsubscribeOperation(); };
}

export function beginSourceMutation(gate: SourceMutationGate): SourceMutationGate | null {
  if (!storageReady || gates.has(gate.sourceTaskId) || hasPendingNisarRecovery(gate.sourceTaskId)) return null;
  const validated = validateGate(gate);
  if (!validated?.frozenMutation) return null;

  gates.set(validated.sourceTaskId, validated);
  if (!persist()) {
    gates.delete(validated.sourceTaskId);
    return null;
  }
  notify();
  return validated;
}

/**
 * Synchronously stages an active gate for retry under its original identity.
 * Validates that the gate matches the live record, is not cleared, passes full
 * structural verification, and retains its frozen mutation. Fails closed if
 * storage is corrupted or the live record has mutated.
 */
export function stageRetrySourceMutation(gate: SourceMutationGate): SourceMutationGate | null {
  if (!storageReady) return null;
  const current = gates.get(gate.sourceTaskId);
  if (!current || current !== gate || !current.frozenMutation) return null;
  const validated = validateGate(current);
  if (!validated || !validated.frozenMutation) return null;

  const previousPending = current.pending;
  const previousError = current.error;
  current.pending = true;
  current.error = undefined;
  if (!persist()) {
    current.pending = previousPending;
    current.error = previousError;
    return null;
  }
  notify();
  return current;
}

export function finishSourceMutation(gate: SourceMutationGate, confirmed: boolean, error?: string): boolean {
  if (!storageReady) return false;
  if (gates.get(gate.sourceTaskId) !== gate) return false;
  // Error text is not part of the frozen identity; keep it within the same
  // bound used by validateGate so a long bridge error cannot block retry.
  const boundedError = error?.slice(0, 2048);
  // A failed read must not invalidate another concurrent read of this gate.
  if (!confirmed && !gate.pending) return true;
  if (confirmed) {
    gates.delete(gate.sourceTaskId);
    if (!persist()) {
      gates.set(gate.sourceTaskId, { ...gate, pending: false, error: boundedError });
      notify();
      return false;
    }
  } else {
    gates.set(gate.sourceTaskId, { ...gate, pending: false, error: boundedError });
    persist();
  }
  notify();
  return true;
}
