/**
 * 缩略图与快视图纯逻辑模块（W5-3c）
 * 包含资产角色匹配、CSP 白名单校验与限并发不重试队列。
 */

import type { SceneRow } from "../../lib/bridge.ts";

/**
 * 镜像 src-tauri/tauri.conf.json:22-58 中 img-src 的 https 主机白名单。
 * 严禁私自修改 tauri.conf.json。当 URL 不在允许列表内时，绝不向网络发起请求（会被 CSP 拦截），直接走占位符流程。
 */
export const CSP_ALLOWED_IMAGE_HOSTS = new Set<string>([
  "nisar.asf.earthdatacloud.nasa.gov",
  "planetarycomputer.microsoft.com",
  "datapool.asf.alaska.edu",
  "cumulus.asf.earthdatacloud.nasa.gov",
  "data.lpdaac.earthdatacloud.nasa.gov",
  "sentinel-cogs.s3.us-west-2.amazonaws.com",
  "earthengine.googleapis.com",
  "bhoonidhi.nrsc.gov.in",
  "tile.openstreetmap.org",
  "a.basemaps.cartocdn.com",
  "b.basemaps.cartocdn.com",
  "c.basemaps.cartocdn.com",
  "d.basemaps.cartocdn.com",
  "server.arcgisonline.com",
  "wayback-a.maptiles.arcgis.com",
  "a.tile.opentopomap.org",
  "b.tile.opentopomap.org",
  "c.tile.opentopomap.org",
  "mt0.google.com",
  "mt1.google.com",
  "mt2.google.com",
  "mt3.google.com",
  "webrd01.is.autonavi.com",
  "webrd02.is.autonavi.com",
  "webrd03.is.autonavi.com",
  "webrd04.is.autonavi.com",
  "webst01.is.autonavi.com",
  "webst02.is.autonavi.com",
  "webst03.is.autonavi.com",
  "webst04.is.autonavi.com",
  "t0.tianditu.gov.cn",
  "t1.tianditu.gov.cn",
  "t2.tianditu.gov.cn",
  "t3.tianditu.gov.cn",
  "t4.tianditu.gov.cn",
  "t5.tianditu.gov.cn",
  "t6.tianditu.gov.cn",
  "t7.tianditu.gov.cn",
]);

/**
 * Approved external public preview hosts that cannot be loaded directly in the
 * WebView (either omitted from CSP or blocked from direct S3 fetches) and are
 * instead proxied via the allowlisted Rust command `geodata_fetch_preview_data_url`.
 */
export const PROXYABLE_PREVIEW_HOSTS = new Set<string>([
  "sentinel-s1-l1c.s3.eu-central-1.amazonaws.com",
  "sentinel-s1-l1c.s3.amazonaws.com",
]);

export function isProxyablePreviewUrl(url?: string | null): url is string {
  if (!url || typeof url !== "string") return false;
  const trimmed = url.trim();
  if (!trimmed.startsWith("https://")) return false;
  try {
    const parsed = new URL(trimmed);
    if (parsed.protocol !== "https:") return false;
    if (parsed.username || parsed.password || parsed.port) return false;
    return PROXYABLE_PREVIEW_HOSTS.has(parsed.hostname.toLowerCase());
  } catch {
    return false;
  }
}

export type ThumbnailState = "placeholder" | "pending" | "loaded" | "failed";
export type PlaceholderReason = "none" | "blocked";

/**
 * 纯函数：判定占位符的原因（"none" 表示来源未提供；"blocked" 表示有 URL 但不符合 CSP 白名单）
 */
export function placeholderReason(row?: Partial<SceneRow> | null): PlaceholderReason {
  const url = thumbnailUrlFor(row);
  if (!url) {
    return "none";
  }
  if (!isAllowedByCsp(url)) {
    return "blocked";
  }
  return "none";
}

export interface ThumbnailQueueOptions {
  maxConcurrent?: number;
  load: (url: string) => Promise<void>;
}

export interface ThumbnailQueue {
  enqueue: (url: string) => Promise<"loaded" | "failed">;
  cancel: (url: string) => void;
  getInFlightCount: () => number;
  getCachedState: (url: string) => "loaded" | "failed" | undefined;
}

function isValidHttpOrDataUrl(url: unknown): url is string {
  if (typeof url !== "string") return false;
  const trimmed = url.trim();
  return (
    trimmed.startsWith("https://") ||
    trimmed.startsWith("data:") ||
    trimmed.startsWith("blob:")
  );
}

/**
 * 纯函数：提取场景行的首选快视图 / 缩略图 URL。
 * 资产来源优先级：行内 assets（任务冻结资产优先），其次 native_metadata.assets
 * （冻结批次元数据中携带的 Provider 快视图，如 ASF Sentinel-1 的 Browse、
 * NISAR 的 Preview）。NISAR 官方预览资产的角色是 "preview"，排在
 * thumbnail/browse 之后作为第三优先级。
 * 1. 优先提取 role 为 "thumbnail" 的资产；
 * 2. 其次提取 role 为 "browse" 的资产；
 * 3. 再次提取 role 为 "preview" 的资产（NISAR）；
 * 4. URL 必须为合法的绝对 https URL 或 data:/blob: URI；
 * 5. 严禁字符串拼接、严禁依据 ID 盲猜；无真实有效 URL 时严格返回 null。
 */
export function thumbnailUrlFor(row?: Partial<SceneRow> | null): string | null {
  if (!row) return null;

  const metadataAssets = Array.isArray((row.native_metadata as any)?.assets)
    ? ((row.native_metadata as any).assets as NonNullable<Partial<SceneRow>["assets"]>)
    : [];
  const rawAssets: NonNullable<Partial<SceneRow>["assets"]> = [
    ...(row.assets ?? []),
    ...metadataAssets,
  ];

  // 按 thumbnail → browse → preview 的顺序取第一个有效地址
  for (const role of ["thumbnail", "browse", "preview"]) {
    for (const asset of rawAssets) {
      if (
        asset &&
        typeof asset === "object" &&
        typeof asset.role === "string" &&
        asset.role.toLowerCase() === role
      ) {
        const url = assetImageUrl(asset);
        if (url) return url;
      }
    }
  }

  return null;
}

/**
 * 资产自身的 url；冻结批次里的原始资产（如 NISAR）没有 url，地址在 routes 的
 * https locator 里。
 */
function assetImageUrl(asset: object): string | null {
  const { url, routes } = asset as { url?: unknown; routes?: unknown };
  if (isValidHttpOrDataUrl(url)) return url.trim();
  if (!Array.isArray(routes)) return null;
  for (const route of routes) {
    const { scheme, locator } = (route ?? {}) as { scheme?: unknown; locator?: unknown };
    if (scheme === "https" && isValidHttpOrDataUrl(locator) && locator.trim().startsWith("https://")) {
      return locator.trim();
    }
  }
  return null;
}

/**
 * 纯函数：校验 URL 是否被 CSP img-src 白名单所允许。
 * 针对不在白名单的域名（如 datapool.asf.alaska.edu、bhoonidhi 等），桌面端必然被拦截，
 * 故直接返回 false，提前避免无效的网络请求。
 */
export function isAllowedByCsp(
  url: string,
  allowlist: Set<string> = CSP_ALLOWED_IMAGE_HOSTS
): boolean {
  if (!url || typeof url !== "string") return false;
  const trimmed = url.trim();

  // data: 和 blob: 协议在 CSP 中被明确允许
  if (trimmed.startsWith("data:") || trimmed.startsWith("blob:")) {
    return true;
  }

  if (!trimmed.startsWith("https://")) {
    return false;
  }

  try {
    const parsed = new URL(trimmed);
    if (parsed.protocol !== "https:") {
      return false;
    }
    return allowlist.has(parsed.hostname.toLowerCase());
  } catch {
    return false;
  }
}

/**
 * 创建限并发、FIFO、失败不重试的缩略图加载队列。
 */
export function createThumbnailQueue(
  options: ThumbnailQueueOptions
): ThumbnailQueue {
  const maxConcurrent = Math.max(1, options.maxConcurrent ?? 4);
  const cache = new Map<string, "loaded" | "failed">();
  const inFlight = new Map<string, Promise<"loaded" | "failed">>();

  interface PendingItem {
    url: string;
    resolve: (status: "loaded" | "failed") => void;
    reject: (err: unknown) => void;
  }
  let pendingQueue: PendingItem[] = [];

  function pump() {
    while (inFlight.size < maxConcurrent && pendingQueue.length > 0) {
      const item = pendingQueue.shift()!;
      if (cache.has(item.url)) {
        item.resolve(cache.get(item.url)!);
        continue;
      }
      if (inFlight.has(item.url)) {
        inFlight.get(item.url)!.then(item.resolve, item.reject);
        continue;
      }
      dispatch(item.url).then(item.resolve, item.reject);
    }
  }

  function dispatch(url: string): Promise<"loaded" | "failed"> {
    const task = (async () => {
      try {
        await options.load(url);
        cache.set(url, "loaded");
        return "loaded" as const;
      } catch {
        // 失败后不重试，缓存失败结果
        cache.set(url, "failed");
        return "failed" as const;
      } finally {
        inFlight.delete(url);
        pump();
      }
    })();

    inFlight.set(url, task);
    return task;
  }

  return {
    enqueue(url: string): Promise<"loaded" | "failed"> {
      if (cache.has(url)) {
        return Promise.resolve(cache.get(url)!);
      }
      if (inFlight.has(url)) {
        return inFlight.get(url)!;
      }
      if (inFlight.size < maxConcurrent) {
        return dispatch(url);
      }
      return new Promise<"loaded" | "failed">((resolve, reject) => {
        pendingQueue.push({ url, resolve, reject });
      });
    },

    cancel(url: string) {
      const idx = pendingQueue.findIndex((item) => item.url === url);
      if (idx === -1) return;
      const [cancelled] = pendingQueue.splice(idx, 1);
      // 移除单条排队项并以 failed 结算，保留同 URL 其余订阅者
      cancelled.resolve("failed");
    },

    getInFlightCount(): number {
      return inFlight.size;
    },

    getCachedState(url: string): "loaded" | "failed" | undefined {
      return cache.get(url);
    },
  };
}

/**
 * 浏览器端通过离屏 Image 加载图片并异步解码
 */
export function loadBrowserImage(url: string): Promise<void> {
  return new Promise((resolve, reject) => {
    if (typeof Image === "undefined") {
      resolve();
      return;
    }
    const img = new Image();
    img.referrerPolicy = "no-referrer";
    img.decoding = "async";
    img.onload = () => resolve();
    img.onerror = () => reject(new Error("Image load failed"));
    img.src = url;
  });
}

/**
 * 全局单例缩略图队列（由所有 ThumbnailSlot 共享）
 */
export const sharedThumbnailQueue: ThumbnailQueue = createThumbnailQueue({
  maxConcurrent: 4,
  load: loadBrowserImage,
});

export interface GeodataPreviewResult {
  status: "loaded" | "failed";
  dataUrl: string | null;
}

export interface GeodataPreviewQueue {
  enqueue: (url: string) => Promise<GeodataPreviewResult>;
  cancel: (url: string) => void;
  getInFlightCount: () => number;
  getCachedState: (url: string) => "loaded" | undefined;
  getCachedUrl: (url: string) => string | undefined;
}

export function createGeodataPreviewQueue(options: {
  maxConcurrent?: number;
  maxCachedEntries?: number;
  fetchDataUrl: (url: string) => Promise<string | null>;
}): GeodataPreviewQueue {
  const maxConcurrent = Math.max(1, options.maxConcurrent ?? 4);
  const maxCachedEntries = Math.max(1, options.maxCachedEntries ?? 128);
  const cache = new Map<string, string>();
  const inFlight = new Map<string, Promise<GeodataPreviewResult>>();
  const pendingQueue: Array<{
    url: string;
    resolve: (res: GeodataPreviewResult) => void;
    reject: (err: unknown) => void;
  }> = [];

  function rememberSuccess(url: string, dataUrl: string) {
    if (cache.has(url)) cache.delete(url);
    cache.set(url, dataUrl);
    while (cache.size > maxCachedEntries) {
      const oldestKey = cache.keys().next().value;
      if (oldestKey === undefined) break;
      cache.delete(oldestKey);
    }
  }

  function pump() {
    while (inFlight.size < maxConcurrent && pendingQueue.length > 0) {
      const item = pendingQueue.shift()!;
      const cached = cache.get(item.url);
      if (cached) {
        item.resolve({ status: "loaded", dataUrl: cached });
        continue;
      }
      if (inFlight.has(item.url)) {
        inFlight.get(item.url)!.then(item.resolve, item.reject);
        continue;
      }
      dispatch(item.url).then(item.resolve, item.reject);
    }
  }

  function dispatch(url: string): Promise<GeodataPreviewResult> {
    const task = (async (): Promise<GeodataPreviewResult> => {
      try {
        const dataUrl = await options.fetchDataUrl(url);
        if (typeof dataUrl === "string" && dataUrl.startsWith("data:image/")) {
          rememberSuccess(url, dataUrl);
          return { status: "loaded", dataUrl };
        }
        return { status: "failed", dataUrl: null };
      } catch {
        return { status: "failed", dataUrl: null };
      } finally {
        inFlight.delete(url);
        pump();
      }
    })();

    inFlight.set(url, task);
    return task;
  }

  return {
    enqueue(url: string): Promise<GeodataPreviewResult> {
      const cached = cache.get(url);
      if (cached) {
        return Promise.resolve({ status: "loaded", dataUrl: cached });
      }
      if (inFlight.has(url)) {
        return inFlight.get(url)!;
      }
      if (inFlight.size < maxConcurrent) {
        return dispatch(url);
      }
      return new Promise<GeodataPreviewResult>((resolve, reject) => {
        pendingQueue.push({ url, resolve, reject });
      });
    },

    cancel(url: string) {
      const idx = pendingQueue.findIndex((item) => item.url === url);
      if (idx === -1) return;
      const [cancelled] = pendingQueue.splice(idx, 1);
      cancelled.resolve({ status: "failed", dataUrl: null });
    },

    getInFlightCount(): number {
      return inFlight.size;
    },

    getCachedState(url: string): "loaded" | undefined {
      return cache.has(url) ? "loaded" : undefined;
    },

    getCachedUrl(url: string): string | undefined {
      return cache.get(url);
    },
  };
}
