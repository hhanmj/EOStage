import type {
  DesktopBhoonidhiAssetSize,
  DesktopBhoonidhiAssetSizeRequest,
} from "./desktopShellBridge.ts";

type Consumer = { resolve: (value: DesktopBhoonidhiAssetSize) => void; reject: (error: Error) => void };
type Entry = {
  key: string;
  request: DesktopBhoonidhiAssetSizeRequest;
  consumers: Set<Consumer>;
  started: boolean;
  cancelled: boolean;
};
type Cached = { value: DesktopBhoonidhiAssetSize | null; error: Error | null; expires: number };
type Dependencies = {
  load: (request: DesktopBhoonidhiAssetSizeRequest) => Promise<DesktopBhoonidhiAssetSize>;
  cancel: (requestId: string) => void;
  onChange: () => void;
};

const aborted = () => new DOMException("产品大小读取已取消。", "AbortError");
const cacheKey = (batchId: string, productKey: string) => JSON.stringify([batchId, productKey]);
let cacheSequence = 0;

/**
 * Viewport-scoped, read-only Bhoonidhi size evidence.
 * At most two authenticated Range probes may be active at once.
 */
export class BhoonidhiAssetSizeCache {
  private readonly deps: Dependencies;
  private readonly capacity: number;
  private readonly values = new Map<string, Cached>();
  private readonly pending = new Map<string, Entry>();
  private queue: Entry[] = [];
  private running = 0;
  private sequence = 0;
  private readonly prefix = `bhoonidhi-size-${Date.now()}-${++cacheSequence}`;

  constructor(deps: Dependencies, options: { capacity?: number } = {}) {
    this.deps = deps;
    this.capacity = options.capacity ?? 128;
    if (!Number.isSafeInteger(this.capacity) || this.capacity < 1 || this.capacity > 256) {
      throw new Error("产品大小缓存容量无效。");
    }
  }

  private cached(key: string): Cached | null {
    const hit = this.values.get(key);
    if (!hit) return null;
    if (hit.expires <= Date.now()) {
      this.values.delete(key);
      return null;
    }
    return hit;
  }

  peek(batchId: string, productKey: string): DesktopBhoonidhiAssetSize | null {
    return this.cached(cacheKey(batchId, productKey))?.value ?? null;
  }

  status(batchId: string, productKey: string): "loading" | "available" | "failed" | null {
    const key = cacheKey(batchId, productKey);
    const hit = this.cached(key);
    if (hit) return hit.value ? "available" : "failed";
    return this.pending.has(key) ? "loading" : null;
  }

  acquire(batchId: string, productKey: string): {
    promise: Promise<DesktopBhoonidhiAssetSize>;
    release: () => void;
  } {
    const key = cacheKey(batchId, productKey);
    const hit = this.cached(key);
    if (hit?.value || hit?.error) {
      this.values.delete(key);
      this.values.set(key, hit);
      return {
        promise: hit.value ? Promise.resolve(hit.value) : Promise.reject(hit.error!),
        release: () => undefined,
      };
    }

    let entry = this.pending.get(key);
    if (!entry || entry.cancelled) {
      entry = {
        key,
        request: {
          request_id: `${this.prefix}-${++this.sequence}`,
          batch_id: batchId,
          item_id: productKey,
        },
        consumers: new Set(),
        started: false,
        cancelled: false,
      };
      this.pending.set(key, entry);
      this.queue.push(entry);
    }

    const owned = entry;
    let consumer!: Consumer;
    const promise = new Promise<DesktopBhoonidhiAssetSize>((resolve, reject) => {
      consumer = { resolve, reject };
    });
    owned.consumers.add(consumer);
    this.pump();

    return {
      promise,
      release: () => {
        if (!owned.consumers.delete(consumer)) return;
        consumer.reject(aborted());
        if (owned.consumers.size === 0) this.cancelEntry(owned);
      },
    };
  }

  private cancelEntry(entry: Entry): void {
    if (entry.cancelled) return;
    entry.cancelled = true;
    if (this.pending.get(entry.key) === entry) this.pending.delete(entry.key);
    this.queue = this.queue.filter(candidate => candidate !== entry);
    if (entry.started) {
      try { this.deps.cancel(entry.request.request_id); } catch { /* local release still removes the viewport consumer */ }
    }
  }

  private remember(key: string, record: Cached): void {
    this.values.delete(key);
    this.values.set(key, record);
    while (this.values.size > this.capacity) this.values.delete(this.values.keys().next().value!);
  }

  private pump(): void {
    while (this.running < 2 && this.queue.length > 0) {
      const entry = this.queue.shift();
      if (!entry || entry.cancelled || entry.consumers.size === 0) continue;
      entry.started = true;
      this.running += 1;
      void this.execute(entry);
    }
  }

  private async execute(entry: Entry): Promise<void> {
    let changed = false;
    try {
      const size = await this.deps.load(entry.request);
      if (entry.cancelled) return;
      if (
        size.product_key !== entry.request.item_id
        || !Number.isSafeInteger(size.size_bytes)
        || size.size_bytes <= 0
      ) {
        throw new Error("Bhoonidhi 产品大小证据无效。");
      }
      const value = Object.freeze({ ...size });
      this.remember(entry.key, { value, error: null, expires: Date.now() + 300_000 });
      for (const consumer of entry.consumers) consumer.resolve(value);
      changed = true;
    } catch (cause) {
      if (entry.cancelled) return;
      const error = cause instanceof Error ? cause : new Error("Bhoonidhi 产品大小读取失败。");
      this.remember(entry.key, { value: null, error, expires: Date.now() + 30_000 });
      for (const consumer of entry.consumers) consumer.reject(error);
      changed = true;
    } finally {
      entry.consumers.clear();
      if (this.pending.get(entry.key) === entry) this.pending.delete(entry.key);
      this.running -= 1;
      this.pump();
      if (changed) this.deps.onChange();
    }
  }

  dispose(): void {
    for (const entry of [...this.pending.values()]) {
      for (const consumer of entry.consumers) consumer.reject(aborted());
      entry.consumers.clear();
      this.cancelEntry(entry);
    }
    this.queue = [];
    this.values.clear();
  }
}
