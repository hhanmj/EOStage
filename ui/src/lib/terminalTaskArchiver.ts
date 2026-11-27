import { useCallback, useEffect, useRef, useState } from "react";

export type TerminalArchiveTask = {
  task_id?: string;
  revision?: number;
  state: string;
  cancelled?: boolean;
  task_type?: string | null;
};

export function isNisarSubsetAwaitingRetry(task: TerminalArchiveTask): boolean {
  return task.task_type === "nisar_subset" && !task.cancelled
    && ["failed", "interrupted", "timeout"].includes(task.state);
}

export function hasServerArchiveEvidence(taskId: string, observedKeys: readonly string[]): boolean {
  return observedKeys.some((key) => key.slice(0, key.lastIndexOf(":")) === taskId);
}

export type TerminalArchiveResult = { ok: boolean; error?: string; code?: string | null };

export type TimeoutHost = {
  now(): number;
  setTimeout(callback: () => void, delayMs: number): number;
  clearTimeout(id: number): void;
};

type ArchiveCallbacks = {
  archive(taskId: string, revision: number): Promise<TerminalArchiveResult>;
  onSuccess(taskId: string, revision: number): void | Promise<void>;
  onError(taskId: string, revision: number, error: unknown): void;
  onReconcile?(taskId: string, revision: number): void | Promise<void>;
};

const TERMINAL_STATES = new Set([
  "completed", "finished", "failed", "cancelled", "interrupted", "timeout",
]);

const browserTimeouts: TimeoutHost = {
  now: () => Date.now(),
  setTimeout: (callback, delayMs) => window.setTimeout(callback, delayMs),
  clearTimeout: (id) => window.clearTimeout(id),
};

export const TERMINAL_ARCHIVE_RETRY_MS = 1_000;
export const TERMINAL_ARCHIVE_MAX_ATTEMPTS = 3;
export const SERVER_ARCHIVE_EVIDENCE_CAP = 512;

export type ServerArchiveEvidence = {
  task_id?: string;
  revision?: number;
};

class ServerObservedArchiveKeyCache {
  private readonly keys = new Map<string, true>();

  constructor(private readonly capacity: number) {}

  observe(rows: readonly ServerArchiveEvidence[]): boolean {
    let changed = false;
    for (const row of rows) {
      const rawTaskId = row.task_id;
      if (typeof rawTaskId !== "string") continue;
      const taskId = rawTaskId.trim();
      const revision = row.revision;
      if (
        !taskId || taskId !== rawTaskId || Array.from(taskId).length > 128 ||
        /[\u0000-\u001f\u007f]/.test(taskId) || revision === undefined ||
        !Number.isSafeInteger(revision) || revision < 0
      ) continue;
      const key = `${taskId}:${revision}`;
      if (this.keys.has(key)) {
        this.keys.delete(key);
      } else {
        changed = true;
      }
      this.keys.set(key, true);
    }
    while (this.keys.size > this.capacity) {
      const oldest = this.keys.keys().next().value as string | undefined;
      if (oldest === undefined) break;
      this.keys.delete(oldest);
      changed = true;
    }
    return changed;
  }

  snapshot(): string[] {
    return [...this.keys.keys()];
  }
}

/**
 * Session-scoped evidence owned only by successful server history reads and
 * fixed archive acknowledgements. Mutable/local display rows never enter it.
 */
export function useServerObservedArchiveKeys() {
  const cacheRef = useRef<ServerObservedArchiveKeyCache | null>(null);
  if (cacheRef.current === null) {
    cacheRef.current = new ServerObservedArchiveKeyCache(SERVER_ARCHIVE_EVIDENCE_CAP);
  }
  const cache = cacheRef.current;
  const [observedKeys, setObservedKeys] = useState<readonly string[]>([]);
  const observeHistoryRows = useCallback((rows: readonly ServerArchiveEvidence[]) => {
    if (cache.observe(rows)) setObservedKeys(cache.snapshot());
  }, [cache]);
  const observeAck = useCallback((taskId: string, revision: number) => {
    if (cache.observe([{ task_id: taskId, revision }])) setObservedKeys(cache.snapshot());
  }, [cache]);
  return { observedKeys, observeHistoryRows, observeAck };
}

const PERMANENT_RECONCILE_CODES = new Set([
  "V3_STORE_REVISION_CONFLICT",
  "V3_STORE_ALREADY_EXISTS",
]);

const TRANSIENT_ARCHIVE_CODES = new Set([
  "GUI000",
  "V3_APPLICATION_BUSY",
  "V3_APPLICATION_TIMEOUT",
  "V3_APPLICATION_UNAVAILABLE",
  "V3_STORE_BUSY",
]);

/**
 * Stable production authority for terminal archive side effects. Attempt keys
 * survive StrictMode effect replay, are registered before dispatch, and are
 * pruned only after the corresponding task:revision is observed in history.
 */
export class TerminalTaskArchiveRuntime {
  private active = false;
  private enabled = false;
  private callbacks: ArchiveCallbacks;
  private readonly timers: TimeoutHost;
  private readonly retryMs: number;
  private readonly candidates = new Map<string, { taskId: string; revision: number }>();
  private readonly observed = new Set<string>();
  private readonly attempted = new Set<string>();
  private readonly inFlight = new Set<string>();
  private readonly retryTimers = new Map<string, number>();
  private readonly attempts = new Map<string, number>();

  constructor(callbacks: ArchiveCallbacks, timers: TimeoutHost = browserTimeouts, retryMs = TERMINAL_ARCHIVE_RETRY_MS) {
    this.callbacks = callbacks;
    this.timers = timers;
    this.retryMs = retryMs;
  }

  configure(callbacks: ArchiveCallbacks) {
    this.callbacks = callbacks;
  }

  activate() {
    this.active = true;
    this.dispatchReady();
  }

  deactivate() {
    this.active = false;
    for (const timer of this.retryTimers.values()) this.timers.clearTimeout(timer);
    this.retryTimers.clear();
  }

  publish(enabled: boolean, tasks: readonly TerminalArchiveTask[], observedKeys: readonly string[]) {
    this.enabled = enabled;
    this.observed.clear();
    for (const key of observedKeys) {
      this.observed.add(key);
      this.attempted.delete(key);
      this.attempts.delete(key);
      this.candidates.delete(key);
      const timer = this.retryTimers.get(key);
      if (timer !== undefined) this.timers.clearTimeout(timer);
      this.retryTimers.delete(key);
    }
    this.candidates.clear();
    if (enabled) {
      for (const task of tasks) {
        const taskId = task.task_id?.trim() ?? "";
        const revision = task.revision;
        const state = task.cancelled ? "cancelled" : task.state;
        if (!taskId || revision === undefined || !Number.isSafeInteger(revision) || revision < 0 || !TERMINAL_STATES.has(state)) continue;
        // A failed subset still owns a resumable, task-bound checkpoint. Keep
        // its real failure visible until explicit retry, cancellation or completion;
        // archival would make the existing Store retry command immutable.
        if (isNisarSubsetAwaitingRetry(task)) continue;
        const key = `${taskId}:${revision}`;
        if (!this.observed.has(key)) this.candidates.set(key, { taskId, revision });
      }
    }
    this.dispatchReady();
  }

  private dispatchReady() {
    if (!this.active || !this.enabled) return;
    for (const [key, candidate] of this.candidates) {
      if (this.observed.has(key) || this.attempted.has(key) || this.inFlight.has(key) || this.retryTimers.has(key)) continue;
      this.dispatch(key, candidate);
    }
  }

  private dispatch(key: string, candidate: { taskId: string; revision: number }) {
    this.attempted.add(key);
    this.inFlight.add(key);
    this.attempts.set(key, (this.attempts.get(key) ?? 0) + 1);
    let request: Promise<TerminalArchiveResult>;
    try {
      request = this.callbacks.archive(candidate.taskId, candidate.revision);
    } catch (error) {
      request = Promise.reject(error);
    }
    void Promise.resolve(request).then(
      (result) => {
        this.inFlight.delete(key);
        if (!this.active || this.observed.has(key)) return;
        if (result.ok) {
          void this.callbacks.onSuccess(candidate.taskId, candidate.revision);
          return;
        }
        const code = result.code ?? "";
        if (PERMANENT_RECONCILE_CODES.has(code)) {
          if (code === "V3_STORE_REVISION_CONFLICT") {
            this.callbacks.onError(candidate.taskId, candidate.revision, result);
          }
          void this.callbacks.onReconcile?.(candidate.taskId, candidate.revision);
          return;
        }
        this.callbacks.onError(candidate.taskId, candidate.revision, result);
        if (TRANSIENT_ARCHIVE_CODES.has(code)) this.scheduleRetry(key, candidate);
      },
      (error) => {
        this.inFlight.delete(key);
        if (!this.active || this.observed.has(key)) return;
        this.callbacks.onError(candidate.taskId, candidate.revision, error);
        this.scheduleRetry(key, candidate);
      },
    );
  }

  private scheduleRetry(key: string, candidate: { taskId: string; revision: number }) {
    const attempts = this.attempts.get(key) ?? 0;
    if (attempts >= TERMINAL_ARCHIVE_MAX_ATTEMPTS) return;
    if (!this.candidates.has(key) || this.retryTimers.has(key)) return;
    const delayMs = this.retryMs * 2 ** Math.max(0, attempts - 1);
    const timer = this.timers.setTimeout(() => {
      this.retryTimers.delete(key);
      if (!this.active || !this.enabled || this.observed.has(key) || !this.candidates.has(key)) return;
      this.attempted.delete(key);
      this.dispatch(key, candidate);
    }, delayMs);
    this.retryTimers.set(key, timer);
  }
}

export function useTerminalTaskArchiver({
  enabled,
  tasks,
  observedKeys,
  archive,
  onSuccess,
  onError,
  onReconcile,
  timers = browserTimeouts,
  retryMs = TERMINAL_ARCHIVE_RETRY_MS,
}: {
  enabled: boolean;
  tasks: readonly TerminalArchiveTask[];
  observedKeys: readonly string[];
  archive(taskId: string, revision: number): Promise<TerminalArchiveResult>;
  onSuccess(taskId: string, revision: number): void | Promise<void>;
  onError(taskId: string, revision: number, error: unknown): void;
  onReconcile?(taskId: string, revision: number): void | Promise<void>;
  timers?: TimeoutHost;
  retryMs?: number;
}) {
  const runtimeRef = useRef<TerminalTaskArchiveRuntime | null>(null);
  if (runtimeRef.current === null) {
    runtimeRef.current = new TerminalTaskArchiveRuntime({ archive, onSuccess, onError, onReconcile }, timers, retryMs);
  }
  const runtime = runtimeRef.current;
  runtime.configure({ archive, onSuccess, onError, onReconcile });
  useEffect(() => {
    runtime.activate();
    return () => runtime.deactivate();
  }, [runtime]);
  useEffect(() => {
    runtime.publish(enabled, tasks, observedKeys);
  }, [enabled, observedKeys, runtime, tasks]);
}
