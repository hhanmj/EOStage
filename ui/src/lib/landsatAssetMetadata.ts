import type { DesktopLandsatAssetDetails, DesktopLandsatAssetDetailsRequest } from "./desktopShellBridge.ts";

type Consumer = { resolve: (value: DesktopLandsatAssetDetails) => void; reject: (error: Error) => void };
type Entry = {
  key: string; request: DesktopLandsatAssetDetailsRequest; consumers: Set<Consumer>;
  started: boolean; cancelled: boolean; foreground: boolean;
};
type Cached = { value: DesktopLandsatAssetDetails | null; error: Error | null; expires: number };
type Dependencies = {
  load: (request: DesktopLandsatAssetDetailsRequest) => Promise<DesktopLandsatAssetDetails>;
  cancel: (requestId: string) => void;
  onChange: () => void;
};
const aborted = () => new DOMException("资产读取已取消。", "AbortError");
const cacheKey = (batchId: string, productKey: string) => JSON.stringify([batchId, productKey]);
let cacheSequence = 0;

/** Disposable read-only metadata; neither this cache nor its viewport consumers mutate a SearchBatch. */
export class LandsatAssetMetadataCache {
  private readonly deps: Dependencies;
  private readonly capacity: number;
  private readonly values = new Map<string, Cached>();
  private readonly pending = new Map<string, Entry>();
  private queue: Entry[] = [];
  private running = 0;
  private sequence = 0;
  private readonly prefix = `landsat-metadata-${Date.now()}-${++cacheSequence}`;

  constructor(deps: Dependencies, options: { capacity?: number } = {}) {
    this.deps = deps;
    this.capacity = options.capacity ?? 128;
    if (!Number.isSafeInteger(this.capacity) || this.capacity < 1 || this.capacity > 256) {
      throw new Error("资产缓存容量无效。");
    }
  }

  private cached(key: string): Cached | null {
    const hit = this.values.get(key);
    if (!hit) return null;
    if (hit.expires <= Date.now()) { this.values.delete(key); return null; }
    return hit;
  }

  peek(batchId: string, productKey: string): DesktopLandsatAssetDetails | null {
    // Last known byte evidence remains displayable; reacquiring a stale entry revalidates it.
    return this.values.get(cacheKey(batchId, productKey))?.value ?? null;
  }

  status(batchId: string, productKey: string): "loading" | "available" | "failed" | null {
    const key = cacheKey(batchId, productKey);
    const value = this.cached(key);
    if (value) return value.value ? "available" : "failed";
    return this.pending.has(key) ? "loading" : null;
  }

  acquire(batchId: string, productKey: string, foreground = false): {
    promise: Promise<DesktopLandsatAssetDetails>; release: () => void;
  } {
    const key = cacheKey(batchId, productKey);
    const hit = this.cached(key);
    if (hit?.value || (hit?.error && !foreground)) {
      this.values.delete(key); this.values.set(key, hit);
      return { promise: hit.value ? Promise.resolve(hit.value) : Promise.reject(hit.error!), release: () => undefined };
    }
    if (hit) this.values.delete(key);
    let entry = this.pending.get(key);
    if (!entry || entry.cancelled) {
      entry = { key, request: { request_id: `${this.prefix}-${++this.sequence}`, batch_id: batchId, item_id: productKey },
        consumers: new Set(), started: false, cancelled: false, foreground };
      this.pending.set(key, entry);
      this.queue.push(entry);
    }
    if (foreground) entry.foreground = true;
    const owned = entry;
    let consumer!: Consumer;
    const promise = new Promise<DesktopLandsatAssetDetails>((resolve, reject) => { consumer = { resolve, reject }; });
    owned.consumers.add(consumer);
    this.pump();
    return { promise, release: () => {
      if (!owned.consumers.delete(consumer)) return;
      consumer.reject(aborted());
      if (owned.consumers.size === 0) this.cancelEntry(owned);
    } };
  }

  private cancelEntry(entry: Entry): void {
    if (entry.cancelled) return;
    entry.cancelled = true;
    if (this.pending.get(entry.key) === entry) this.pending.delete(entry.key);
    this.queue = this.queue.filter(candidate => candidate !== entry);
    if (entry.started) {
      // Keep the concurrency slot until the native request actually settles.
      try { this.deps.cancel(entry.request.request_id); } catch { /* local cancellation remains authoritative */ }
    }
  }

  private remember(key: string, record: Cached): void {
    this.values.delete(key); this.values.set(key, record);
    while (this.values.size > this.capacity) this.values.delete(this.values.keys().next().value!);
  }

  private pump(): void {
    while (this.running < 2 && this.queue.length > 0) {
      const priority = this.queue.findIndex(entry => entry.foreground);
      const [entry] = this.queue.splice(priority < 0 ? 0 : priority, 1);
      if (!entry || entry.cancelled || entry.consumers.size === 0) continue;
      entry.started = true;
      this.running += 1;
      void this.execute(entry);
    }
  }

  private async execute(entry: Entry): Promise<void> {
    let changed = false;
    try {
      const details = await this.deps.load(entry.request);
      if (entry.cancelled) return;
      if (details.product_key !== entry.request.item_id) throw new Error("资产详情身份与当前产品不一致。");
      const total = details.assets.reduce((sum, asset) => sum + asset.size_bytes, 0);
      if (!Number.isSafeInteger(details.total_size_bytes) || details.total_size_bytes <= 0
        || details.assets.length === 0 || total !== details.total_size_bytes
        || details.assets.some(asset => !Number.isSafeInteger(asset.size_bytes) || asset.size_bytes <= 0)
        || new Set(details.assets.map(asset => asset.asset_key)).size !== details.assets.length) {
        throw new Error("资产文件集大小证据不完整。");
      }
      const value = Object.freeze({ ...details, assets: details.assets.map(asset => Object.freeze({ ...asset })) });
      Object.freeze(value.assets);
      this.remember(entry.key, { value, error: null, expires: Date.now() + 300_000 });
      for (const consumer of entry.consumers) consumer.resolve(value);
      changed = true;
    } catch (cause) {
      if (entry.cancelled) return;
      const error = cause instanceof Error ? cause : new Error("资产详情加载失败。");
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
    for (const entry of this.pending.values()) {
      for (const consumer of entry.consumers) consumer.reject(aborted());
      entry.consumers.clear();
      this.cancelEntry(entry);
    }
    this.queue = [];
    this.values.clear();
  }
}
