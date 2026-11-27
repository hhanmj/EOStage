type StatusCoordinator = {
  activate(): void;
  dispose(): void;
  poll(): Promise<unknown> | null;
  refresh(): Promise<unknown>;
};

type TimerHost = {
  setInterval(callback: () => void, intervalMs: number): number;
  clearInterval(id: number): void;
};

const MUTABLE_TASK_STATES = new Set(["queued", "running", "paused", "cancelling"]);

export const DOWNLOAD_STATUS_POLL_INTERVAL_MS = 900;

function browserTimers(): TimerHost {
  return {
    setInterval: (callback, intervalMs) => window.setInterval(callback, intervalMs),
    clearInterval: (id) => window.clearInterval(id),
  };
}

export function hasMutableDownloadTask(taskStates: readonly string[]): boolean {
  return taskStates.some((state) => MUTABLE_TASK_STATES.has(state));
}

/**
 * Keeps the existing latest-response coordinator as the only request authority.
 * The scheduler only decides whether a bounded refresh cadence is necessary.
 */
export class ActiveOnlyStatusPolling {
  private timerId: number | null = null;
  private disposed = false;
  private readonly coordinator: StatusCoordinator;
  private readonly timers: TimerHost;
  private readonly intervalMs: number;

  constructor(
    coordinator: StatusCoordinator,
    timers: TimerHost = browserTimers(),
    intervalMs = DOWNLOAD_STATUS_POLL_INTERVAL_MS,
  ) {
    this.coordinator = coordinator;
    this.timers = timers;
    this.intervalMs = intervalMs;
  }

  hydrate(): Promise<unknown> {
    if (this.disposed) return Promise.resolve(null);
    this.coordinator.activate();
    return this.coordinator.refresh();
  }

  setTaskStates(taskStates: readonly string[]) {
    if (this.disposed) return;
    if (hasMutableDownloadTask(taskStates)) {
      this.coordinator.activate();
      if (this.timerId === null) {
        this.timerId = this.timers.setInterval(() => {
          void this.coordinator.poll();
        }, this.intervalMs);
      }
      return;
    }
    this.stopTimer();
  }

  refreshAfterAction(): Promise<unknown> {
    if (this.disposed) return Promise.resolve(null);
    this.coordinator.activate();
    return this.coordinator.refresh();
  }

  dispose() {
    if (this.disposed) return;
    this.disposed = true;
    this.stopTimer();
    this.coordinator.dispose();
  }

  private stopTimer() {
    if (this.timerId === null) return;
    this.timers.clearInterval(this.timerId);
    this.timerId = null;
  }
}
