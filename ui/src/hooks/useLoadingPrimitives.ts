import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  LOADING_DELAY_MS,
  LOADING_MIN_VISIBLE_MS,
  LONG_WORK_REASSURE_MS,
  REFRESH_TURN_MS,
  computeFullTurnStopMs,
  computeRemainingVisibleHoldMs,
  estimateDownloadTaskEta,
  type EtaEstimateResult,
  type TaskEtaLike,
} from "../lib/loadingStateMachine.ts";

/**
 * Delays showing a spinner/skeleton/bar until `active` stays true for `delayMs` (300ms),
 * eliminating sub-300ms flashes. Once shown, keeps it visible at least `minVisibleMs` (500ms).
 */
export function useDelayedFlag(
  active: boolean,
  delayMs: number = LOADING_DELAY_MS,
  minVisibleMs: number = LOADING_MIN_VISIBLE_MS
): boolean {
  const [visible, setVisible] = useState<boolean>(() => active && delayMs <= 0);
  const shownAtRef = useRef<number | null>(active && delayMs <= 0 ? Date.now() : null);

  useEffect(() => {
    if (active) {
      if (visible) return;
      if (delayMs <= 0) {
        shownAtRef.current = Date.now();
        setVisible(true);
        return;
      }
      const timer = window.setTimeout(() => {
        shownAtRef.current = Date.now();
        setVisible(true);
      }, delayMs);
      return () => {
        window.clearTimeout(timer);
      };
    }

    if (!visible) {
      shownAtRef.current = null;
      return;
    }

    const rem = computeRemainingVisibleHoldMs(
      shownAtRef.current ?? Date.now(),
      Date.now(),
      minVisibleMs
    );
    if (rem <= 0) {
      shownAtRef.current = null;
      setVisible(false);
      return;
    }
    const timer = window.setTimeout(() => {
      shownAtRef.current = null;
      setVisible(false);
    }, rem);
    return () => {
      window.clearTimeout(timer);
    };
  }, [active, delayMs, minVisibleMs, visible]);

  return visible;
}

/**
 * Returns true once `active` has run uninterrupted past `thresholdMs` (default 10s),
 * letting uncountable long operations surface calm reassurance copy.
 */
export function useElapsedStage(
  active: boolean,
  thresholdMs: number = LONG_WORK_REASSURE_MS
): boolean {
  const [overdue, setOverdue] = useState(false);

  useEffect(() => {
    if (!active) {
      setOverdue(false);
      return;
    }
    const timer = window.setTimeout(() => {
      setOverdue(true);
    }, thresholdMs);
    return () => {
      window.clearTimeout(timer);
    };
  }, [active, thresholdMs]);

  return overdue;
}

/**
 * Turns a refresh/sync icon immediately from press, completes at least one full turn
 * (default 640ms) even on instant responses, and stops cleanly on an integer turn.
 */
export function useFullTurnSpin(
  active: boolean,
  turnMs: number = REFRESH_TURN_MS
): { spinning: boolean; spinClass: string } {
  const [spinning, setSpinning] = useState<boolean>(active);
  const startedAtRef = useRef<number | null>(active ? Date.now() : null);

  useEffect(() => {
    if (active) {
      if (startedAtRef.current === null) {
        startedAtRef.current = Date.now();
      }
      setSpinning(true);
      return;
    }

    if (!spinning) {
      startedAtRef.current = null;
      return;
    }

    const startedAt = startedAtRef.current ?? Date.now();
    const stopInMs = computeFullTurnStopMs(startedAt, Date.now(), turnMs);
    if (stopInMs <= 16) {
      startedAtRef.current = null;
      setSpinning(false);
      return;
    }
    const timer = window.setTimeout(() => {
      startedAtRef.current = null;
      setSpinning(false);
    }, stopInMs);
    return () => {
      window.clearTimeout(timer);
    };
  }, [active, spinning, turnMs]);

  return {
    spinning,
    spinClass: spinning ? "sp-refresh-full-turn" : "",
  };
}

export function useLiveNow(active: boolean, intervalMs = 1000): number {
  const [nowMs, setNowMs] = useState<number>(() => Date.now());

  useEffect(() => {
    setNowMs(Date.now());
    if (!active) return;
    const id = window.setInterval(() => {
      setNowMs(Date.now());
    }, intervalMs);
    return () => window.clearInterval(id);
  }, [active, intervalMs]);

  return nowMs;
}

/**
 * Computes live countdown ETA ("估算中…" <1s, "约 … 剩余" >=1s) for running
 * download/transfer tasks using a stable per-run start timestamp.
 */
export function useTaskEta(task: TaskEtaLike): EtaEstimateResult {
  const isRunning = task.status === "running";
  const [runningSinceMs, setRunningSinceMs] = useState<number | null>(() =>
    isRunning ? Date.now() : null
  );
  const prevRef = useRef<{ taskId: string; status: string }>({
    taskId: task.task_id,
    status: task.status,
  });

  useEffect(() => {
    const prev = prevRef.current;
    if (isRunning) {
      if (prev.taskId !== task.task_id || prev.status !== "running") {
        setRunningSinceMs(Date.now());
      } else {
        setRunningSinceMs((cur) => cur ?? Date.now());
      }
    } else {
      setRunningSinceMs(null);
    }
    prevRef.current = { taskId: task.task_id, status: task.status };
  }, [isRunning, task.status, task.task_id]);

  const nowMs = useLiveNow(isRunning);
  return estimateDownloadTaskEta(task, runningSinceMs, nowMs);
}

export interface UseCancellableActionOptions {
  /** Sampling cadence while active (ms). Defaults to 100ms for smooth 0.1s elapsed feedback. */
  tickMs?: number;
  /** Optional delay before surfacing a non-empty elapsedLabel (ms). Defaults to 0ms. */
  showLabelAfterMs?: number;
  /** Prefix placed before the formatted elapsed time. Defaults to "已用时". */
  labelPrefix?: string;
  /** Optional hook called synchronously when `cancel()` interrupts an active action. */
  onCancel?: () => void;
}

export interface CancellableActionHandle {
  pending: boolean;
  elapsedSeconds: number;
  elapsedMs: number;
  elapsedLabel: string;
  run: <TResult>(
    fn: (signal: AbortSignal) => Promise<TResult>
  ) => Promise<TResult | undefined>;
  cancel: () => void;
}

/**
 * Coordinates a cancellable async action with a monotonically increasing generation token
 * and an AbortController, plus live elapsed time reporting while pending.
 */
export function useCancellableAction(
  options: UseCancellableActionOptions = {}
): CancellableActionHandle {
  const {
    tickMs = 100,
    showLabelAfterMs = 0,
    labelPrefix = "已用时",
    onCancel,
  } = options;

  const [pending, setPending] = useState(false);
  const [elapsedMs, setElapsedMs] = useState(0);

  const generationRef = useRef(0);
  const controllerRef = useRef<AbortController | null>(null);
  const startedAtRef = useRef<number | null>(null);
  const mountedRef = useRef(true);
  const onCancelRef = useRef(onCancel);
  onCancelRef.current = onCancel;

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      generationRef.current += 1;
      controllerRef.current?.abort();
      controllerRef.current = null;
    };
  }, []);

  useEffect(() => {
    if (!pending) {
      startedAtRef.current = null;
      return;
    }
    const startedAt = startedAtRef.current ?? Date.now();
    startedAtRef.current = startedAt;
    const step = Math.max(50, tickMs);
    const id = window.setInterval(() => {
      if (!mountedRef.current || startedAtRef.current === null) return;
      setElapsedMs(Math.max(0, Date.now() - startedAtRef.current));
    }, step);
    return () => window.clearInterval(id);
  }, [pending, tickMs]);

  const cancel = useCallback(() => {
    const wasPending = controllerRef.current !== null;
    generationRef.current += 1;
    if (controllerRef.current) {
      controllerRef.current.abort();
      controllerRef.current = null;
    }
    startedAtRef.current = null;
    if (mountedRef.current) {
      setPending(false);
      setElapsedMs(0);
    }
    if (wasPending) {
      onCancelRef.current?.();
    }
  }, []);

  const run = useCallback(
    async <TResult,>(
      fn: (signal: AbortSignal) => Promise<TResult>
    ): Promise<TResult | undefined> => {
      generationRef.current += 1;
      const myGeneration = generationRef.current;
      if (controllerRef.current) {
        controllerRef.current.abort();
      }
      const controller = new AbortController();
      controllerRef.current = controller;
      startedAtRef.current = Date.now();
      if (mountedRef.current) {
        setElapsedMs(0);
        setPending(true);
      }
      try {
        const result = await fn(controller.signal);
        if (generationRef.current !== myGeneration || controller.signal.aborted) {
          return undefined;
        }
        return result;
      } catch (error) {
        if (generationRef.current !== myGeneration || controller.signal.aborted) {
          return undefined;
        }
        throw error;
      } finally {
        if (generationRef.current === myGeneration) {
          controllerRef.current = null;
          startedAtRef.current = null;
          if (mountedRef.current) {
            setPending(false);
          }
        }
      }
    },
    []
  );

  const elapsedSeconds = useMemo(
    () => Number((elapsedMs / 1000).toFixed(1)),
    [elapsedMs]
  );

  const elapsedLabel = useMemo(() => {
    if (!pending || elapsedMs < showLabelAfterMs) return "";
    if (elapsedMs >= 60_000) {
      const totalSecs = Math.floor(elapsedMs / 1000);
      const mins = Math.floor(totalSecs / 60);
      const secs = totalSecs % 60;
      return `${labelPrefix} ${mins}m ${String(secs).padStart(2, "0")}s`;
    }
    return `${labelPrefix} ${(elapsedMs / 1000).toFixed(1)}s`;
  }, [pending, elapsedMs, showLabelAfterMs, labelPrefix]);

  return {
    pending,
    elapsedSeconds,
    elapsedMs,
    elapsedLabel,
    run,
    cancel,
  };
}
