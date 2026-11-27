type LatestStatusCoordinatorOptions<T> = {
  read: () => Promise<T>;
  publish: (value: T) => void;
  accept?: (value: T) => boolean;
  onError?: (error: unknown) => void;
};

export class LatestStatusCoordinator<T> {
  private readonly read: () => Promise<T>;
  private readonly publish: (value: T) => void;
  private readonly accept: (value: T) => boolean;
  private readonly onError: (error: unknown) => void;
  private active = true;
  private generation = 0;
  private currentRequest: Promise<T | null> | null = null;
  private currentRequestGeneration: number | null = null;
  private staleGenerationRefresh: Promise<T | null> | null = null;

  constructor(options: LatestStatusCoordinatorOptions<T>) {
    this.read = options.read;
    this.publish = options.publish;
    this.accept = options.accept ?? (() => true);
    this.onError = options.onError ?? (() => undefined);
  }

  activate() {
    if (!this.active) this.generation += 1;
    this.active = true;
  }

  dispose() {
    this.active = false;
    this.generation += 1;
  }

  poll(): Promise<T | null> | null {
    if (!this.active) return null;
    if (this.currentRequest) {
      if (this.currentRequestGeneration === this.generation) return null;
      if (this.staleGenerationRefresh) return this.staleGenerationRefresh;
      const refresh = this.refresh().finally(() => {
        if (this.staleGenerationRefresh === refresh) this.staleGenerationRefresh = null;
      });
      this.staleGenerationRefresh = refresh;
      return refresh;
    }
    return this.startRead();
  }

  async refresh(): Promise<T | null> {
    if (!this.active) return null;
    while (this.currentRequest) await this.currentRequest;
    if (!this.active) return null;
    return this.startRead();
  }

  private startRead(): Promise<T | null> {
    if (!this.active) return Promise.resolve(null);
    const requestGeneration = this.generation;
    const request = Promise.resolve()
      .then(this.read)
      .then(
        (value) => {
          if (!this.active || requestGeneration !== this.generation) return null;
          if (!this.accept(value)) return null;
          this.publish(value);
          return value;
        },
        (error) => {
          if (this.active && requestGeneration === this.generation) this.onError(error);
          return null;
        },
      )
      .finally(() => {
        if (this.currentRequest === request) {
          this.currentRequest = null;
          this.currentRequestGeneration = null;
        }
      });
    this.currentRequest = request;
    this.currentRequestGeneration = requestGeneration;
    return request;
  }
}
