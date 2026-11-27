export const DEFAULT_DEM_MANAGER_STATUS_TIMEOUT_MS = 10_000;

export class DemManagerStatusTimeoutError extends Error {
  readonly code = "DEM_MANAGER_STATUS_TIMEOUT";
  readonly timeoutMs: number;

  constructor(timeoutMs: number) {
    super(`DEM 任务状态读取超过 ${timeoutMs} 毫秒，已停止等待；可稍后重试。`);
    this.name = "DemManagerStatusTimeoutError";
    this.timeoutMs = timeoutMs;
  }
}

type DemManagerStatusCoordinatorOptions<T> = {
  read: () => Promise<T>;
  onValue: (value: T) => void;
  onError: (error: unknown) => void;
  timeoutMs?: number;
};

export class DemManagerStatusCoordinator<T> {
  private readonly read: () => Promise<T>;
  private readonly onValue: (value: T) => void;
  private readonly onError: (error: unknown) => void;
  private readonly timeoutMs: number;
  private active = true;
  private lifecycleEpoch = 0;
  private currentRequest: Promise<T | null> | null = null;
  private cancelCurrentRequest: (() => void) | null = null;

  constructor(options: DemManagerStatusCoordinatorOptions<T>) {
    this.read = options.read;
    this.onValue = options.onValue;
    this.onError = options.onError;
    this.timeoutMs = Math.max(
      1,
      Math.floor(options.timeoutMs ?? DEFAULT_DEM_MANAGER_STATUS_TIMEOUT_MS),
    );
  }

  get hasInFlight() {
    return this.currentRequest !== null;
  }

  activate() {
    if (this.active) return;
    this.active = true;
    this.lifecycleEpoch += 1;
  }

  dispose() {
    if (!this.active && !this.currentRequest) return;
    this.active = false;
    this.lifecycleEpoch += 1;
    const cancel = this.cancelCurrentRequest;
    this.currentRequest = null;
    this.cancelCurrentRequest = null;
    cancel?.();
  }

  poll(): Promise<T | null> | null {
    if (!this.active || this.currentRequest) return null;
    return this.startRead();
  }

  async refresh(): Promise<T | null> {
    if (!this.active) return null;
    const lifecycleEpoch = this.lifecycleEpoch;
    while (this.currentRequest) {
      const pendingRequest = this.currentRequest;
      await pendingRequest;
      if (!this.active || lifecycleEpoch !== this.lifecycleEpoch) return null;
    }
    if (!this.active || lifecycleEpoch !== this.lifecycleEpoch) return null;
    return this.startRead();
  }

  private startRead(): Promise<T | null> {
    if (!this.active) return Promise.resolve(null);
    const lifecycleEpoch = this.lifecycleEpoch;
    let cancel: () => void = () => undefined;

    const operation = new Promise<T | null>((resolve) => {
      let settled = false;
      let timeoutId: ReturnType<typeof globalThis.setTimeout> | null = null;
      const finish = (value: T | null) => {
        if (settled) return;
        settled = true;
        if (timeoutId !== null) globalThis.clearTimeout(timeoutId);
        resolve(value);
      };
      const canPublish = () => this.active && lifecycleEpoch === this.lifecycleEpoch;
      const publishError = (error: unknown) => {
        if (!canPublish()) return;
        try {
          this.onError(error);
        } catch {
          // A rendering callback must not strand the shared flight.
        }
      };

      cancel = () => finish(null);
      timeoutId = globalThis.setTimeout(() => {
        publishError(new DemManagerStatusTimeoutError(this.timeoutMs));
        finish(null);
      }, this.timeoutMs);

      let source: Promise<T>;
      try {
        source = this.read();
      } catch (error) {
        publishError(error);
        finish(null);
        return;
      }
      void Promise.resolve(source).then(
        (value) => {
          if (settled || !canPublish()) {
            finish(null);
            return;
          }
          try {
            this.onValue(value);
          } catch (error) {
            publishError(error);
            finish(null);
            return;
          }
          finish(value);
        },
        (error) => {
          if (settled) return;
          publishError(error);
          finish(null);
        },
      );
    });

    const request = operation.finally(() => {
      if (this.currentRequest === request) this.currentRequest = null;
      if (this.cancelCurrentRequest === cancel) this.cancelCurrentRequest = null;
    });
    this.currentRequest = request;
    this.cancelCurrentRequest = cancel;
    return request;
  }
}
