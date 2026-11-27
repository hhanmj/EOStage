/**
 * Pure, framework-free state math for loading/waiting rhythms:
 * - 300ms delay / 500ms minimum hold gate (prevents sub-300ms flicker)
 * - Integer full-turn rotation timing (>=640ms/turn so refresh icons finish at rest)
 * - Counted long-work ETA ("估算中…" <1s, formatted remaining time >=1s)
 */

export const LOADING_DELAY_MS = 300;
export const LOADING_MIN_VISIBLE_MS = 500;
export const REFRESH_TURN_MS = 640;
export const LONG_WORK_ENTRY_DELAY_MS = 400;
export const LONG_WORK_REASSURE_MS = 10_000;
export const ETA_WARMUP_MS = 1_000;

/**
 * Computes remaining hold time in ms when `active` stops after the loader has
 * already become visible at `shownAtMs`. Ensures total visible time >= minVisibleMs.
 */
export function computeRemainingVisibleHoldMs(
  shownAtMs: number,
  nowMs: number,
  minVisibleMs: number = LOADING_MIN_VISIBLE_MS
): number {
  const visibleElapsed = Math.max(0, nowMs - shownAtMs);
  return Math.max(0, minVisibleMs - visibleElapsed);
}

/**
 * Computes how many milliseconds remain until an action icon completes an exact
 * integer number of `turnMs` rotations (at least one full turn even for instant answers).
 */
export function computeFullTurnStopMs(
  startedAtMs: number,
  nowMs: number,
  turnMs: number = REFRESH_TURN_MS
): number {
  if (turnMs <= 0) return 0;
  const elapsed = Math.max(0, nowMs - startedAtMs);
  if (elapsed < turnMs) {
    return turnMs - elapsed;
  }
  const remainder = elapsed % turnMs;
  return remainder === 0 ? 0 : turnMs - remainder;
}

export interface EtaEstimateInput {
  startedAtMs: number;
  nowMs: number;
  completed?: number | null;
  total?: number | null;
  ratePerSecond?: number | null;
}

export interface EtaEstimateResult {
  kind: "hidden" | "estimating" | "eta";
  secondsLeft: number | null;
  textZh: string;
}

/**
 * Formats estimated remaining seconds into concise Chinese duration copy.
 */
export function formatEtaSecondsZh(seconds: number): string {
  const safe = Math.max(1, Math.ceil(seconds));
  if (safe < 60) {
    return `约 ${safe} 秒剩余`;
  }
  const totalMinutes = Math.floor(safe / 60);
  const remSeconds = safe % 60;
  if (totalMinutes < 60) {
    return remSeconds > 0 && totalMinutes < 10
      ? `约 ${totalMinutes} 分 ${remSeconds} 秒剩余`
      : `约 ${totalMinutes} 分钟剩余`;
  }
  const hours = Math.floor(totalMinutes / 60);
  const remMinutes = totalMinutes % 60;
  return remMinutes > 0 ? `约 ${hours} 小时 ${remMinutes} 分剩余` : `约 ${hours} 小时剩余`;
}

/**
 * Estimates time left for counted progress (bytes or items).
 * Shows "估算中…" during the first 1 second (or before measurable rate), then
 * returns remaining seconds & formatted text.
 */
export function estimateRemainingEta(input: EtaEstimateInput): EtaEstimateResult {
  const { startedAtMs, nowMs, completed, total, ratePerSecond } = input;
  if (total == null || !Number.isFinite(total) || total <= 0) {
    return { kind: "hidden", secondsLeft: null, textZh: "" };
  }
  const done = completed != null && Number.isFinite(completed) ? Math.max(0, completed) : 0;
  if (done >= total) {
    return { kind: "hidden", secondsLeft: 0, textZh: "" };
  }

  const elapsedMs = Math.max(0, nowMs - startedAtMs);
  if (elapsedMs < ETA_WARMUP_MS) {
    return { kind: "estimating", secondsLeft: null, textZh: "估算中…" };
  }

  const measuredRate =
    ratePerSecond != null && Number.isFinite(ratePerSecond) && ratePerSecond > 0
      ? ratePerSecond
      : done > 0
      ? done / (elapsedMs / 1000)
      : 0;

  if (!Number.isFinite(measuredRate) || measuredRate <= 0) {
    return { kind: "estimating", secondsLeft: null, textZh: "估算中…" };
  }

  const remainingUnits = total - done;
  const secondsLeft = Math.max(1, Math.ceil(remainingUnits / measuredRate));
  return {
    kind: "eta",
    secondsLeft,
    textZh: formatEtaSecondsZh(secondsLeft),
  };
}

export interface TaskEtaLike {
  task_id: string;
  status: string;
  done_bytes: number;
  total_bytes?: number | null;
  bytes_reported?: false;
  bytes_per_second?: number;
  items_count?: number;
  completed_items_count?: number;
}

/**
 * Estimates remaining ETA for a running download/transfer task from bytes first
 * (when reliable total_bytes exists) and falls back to item count (>1 items).
 */
export function estimateDownloadTaskEta(
  task: TaskEtaLike,
  runningStartedAtMs: number | null,
  nowMs: number
): EtaEstimateResult {
  if (task.status !== "running" || runningStartedAtMs == null) {
    return { kind: "hidden", secondsLeft: null, textZh: "" };
  }
  const hasBytes =
    typeof task.total_bytes === "number" &&
    task.total_bytes > 0 &&
    task.bytes_reported !== false;
  if (hasBytes) {
    return estimateRemainingEta({
      startedAtMs: runningStartedAtMs,
      nowMs,
      completed: task.done_bytes,
      total: task.total_bytes,
      ratePerSecond:
        typeof task.bytes_per_second === "number" && task.bytes_per_second > 0
          ? task.bytes_per_second
          : null,
    });
  }
  if (typeof task.items_count === "number" && task.items_count > 1) {
    return estimateRemainingEta({
      startedAtMs: runningStartedAtMs,
      nowMs,
      completed: task.completed_items_count ?? 0,
      total: task.items_count,
    });
  }
  return { kind: "hidden", secondsLeft: null, textZh: "" };
}
