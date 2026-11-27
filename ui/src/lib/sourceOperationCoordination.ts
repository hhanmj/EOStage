// Synchronous coordination within one JavaScript runtime. Persistent request
// records remain owned by their respective gates; this is not a cross-WebView lock.
const inFlight = new Set<string>();
const epochs = new Map<string, number>();
const attempts = new Map<string, number>();
const listeners = new Set<() => void>();

import { getStorage } from "@/lib/webStorage";

export const NISAR_RECOVERY_STORAGE_PREFIX = "insar:nisar-recovery:pending:v1:";

/** Existing or unreadable recovery records block new operations, never retries. */
export function hasPendingNisarRecovery(taskId: string): boolean {
  try {
    return getStorage().getItem(`${NISAR_RECOVERY_STORAGE_PREFIX}${taskId}`) !== null;
  } catch {
    return true;
  }
}

function notify(): void {
  for (const listener of listeners) {
    try { listener(); } catch { /* An observer must not interrupt coordination. */ }
  }
}

export function acquireSourceOperation(taskId: string): boolean {
  if (inFlight.has(taskId)) return false;
  inFlight.add(taskId);
  attempts.set(taskId, (attempts.get(taskId) ?? 0) + 1);
  notify();
  return true;
}

export function releaseSourceOperation(taskId: string): void {
  if (inFlight.delete(taskId)) notify();
}

export function isSourceOperationInFlight(taskId: string): boolean {
  return inFlight.has(taskId);
}

export function sourceOperationEpoch(taskId: string): number {
  return epochs.get(taskId) ?? 0;
}

export function sourceOperationAttempt(taskId: string): number {
  return attempts.get(taskId) ?? 0;
}

/** Stable primitive snapshot for React subscriptions, including rapid retries. */
export function sourceOperationSnapshot(taskId: string): string {
  return `${sourceOperationAttempt(taskId)}:${isSourceOperationInFlight(taskId) ? 1 : 0}`;
}

export function noteSourceOperationSuccess(taskId: string): void {
  epochs.set(taskId, sourceOperationEpoch(taskId) + 1);
  notify();
}

export function subscribeSourceOperation(listener: () => void): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}
