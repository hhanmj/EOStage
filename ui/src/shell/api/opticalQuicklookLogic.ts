import { t } from "../../i18n/index.ts";
import type { SceneRow } from "../../lib/bridge.ts";
import {
  earthSearchSentinel2ThumbnailAsset,
  isEarthSearchSentinel2Product,
  isSentinel2L1cCatalogProduct,
  isSentinel2L2aCatalogProduct,
  isMicrosoftSentinel2SearchOnlyProduct,
  sentinel2MapTilePreview,
} from "../../lib/sentinel2Catalog.ts";
import {
  isLandsatC2L2CatalogProduct,
  landsatBrowseAvailable,
  landsatMapTilePreview,
  LANDSAT_C2_L2_PROVIDER_ID,
  MPC_LANDSAT_PROVIDER_ID,
} from "../../lib/landsatCatalog.ts";
import { hlsMapTilePreview } from "../../lib/hlsCatalog.ts";
import { sceneFootprintRenderModel } from "../../lib/sceneFootprint.ts";
import {
  createThumbnailQueue,
  isAllowedByCsp,
  loadBrowserImage,
} from "./thumbnailLogic.ts";

export type OpticalQuicklookPlan =
  | { kind: "mpc_tile"; url: string }
  | { kind: "sentinel2_map_preview" }
  | { kind: "cdse_quicklook" }
  | { kind: "landsat_browse" }
  | { kind: "unavailable"; title: string };

export type OpticalQuicklookRequest = {
  scene: SceneRow;
  batchId?: string | null;
};

export type OpticalQuicklookQueue = {
  enqueue: (request: OpticalQuicklookRequest) => Promise<"loaded" | "failed">;
  cancel: (cacheKey: string) => void;
  getCachedState: (cacheKey: string) => "loaded" | "failed" | undefined;
  getCachedUrl: (cacheKey: string) => string | undefined;
  getInFlightCount: () => number;
};

const QUICKLOOK_ZOOM = 8;
const MAX_MERCATOR_LATITUDE = 85.05112878;

function text(value: unknown): string {
  return typeof value === "string" ? value.trim() : "";
}

export function opticalQuicklookItemId(scene: SceneRow): string {
  return text(scene.product_key)
    || text(scene.sar_product?.provider_product_key)
    || text(scene.sar_product?.product_key)
    || text(scene.scene_id);
}

export function opticalQuicklookCacheKey(scene: SceneRow, batchId?: string | null): string {
  return JSON.stringify([
    text(batchId),
    text(scene.provider) || text(scene.sar_product?.provider),
    opticalQuicklookItemId(scene),
  ]);
}

function quicklookBounds(scene: SceneRow) {
  if (scene.footprint_geojson) {
    return sceneFootprintRenderModel(scene.footprint_geojson)?.bbox ?? null;
  }
  const bbox = scene.footprint_bbox;
  if (!bbox) return null;
  const values = [bbox.west, bbox.south, bbox.east, bbox.north];
  if (!values.every(Number.isFinite) || bbox.south >= bbox.north || bbox.west === bbox.east) return null;
  return bbox;
}

/**
 * Whole-scene centered preview image for Planetary Computer STAC/COG items,
 * falling back to center WebMercatorQuad tile for generic tile templates.
 */
export function opticalQuicklookTileUrl(urlTemplate: string, scene: SceneRow): string | null {
  const bounds = quicklookBounds(scene);
  if (!bounds) return null;

  if (urlTemplate.includes("/api/data/v1/item/tiles/WebMercatorQuad/")) {
    try {
      const dummy = urlTemplate.replace("{z}/{x}/{y}@1x", "0/0/0").replace("{z}/{x}/{y}", "0/0/0");
      const parsed = new URL(dummy);
      if (parsed.origin === "https://planetarycomputer.microsoft.com") {
        const previewUrl = new URL(`${parsed.origin}/api/data/v1/item/preview.png`);
        parsed.searchParams.forEach((value, key) => {
          previewUrl.searchParams.append(key, value);
        });
        previewUrl.searchParams.set("max_size", "192");
        if (!previewUrl.searchParams.has("format")) {
          previewUrl.searchParams.set("format", "png");
        }
        const resolved = previewUrl.toString();
        return resolved.includes("{") || !isAllowedByCsp(resolved) ? null : resolved;
      }
    } catch {
      // Fall through to generic center tile template below.
    }
  }

  const longitude = (bounds.west + bounds.east) / 2;
  const latitude = Math.max(-MAX_MERCATOR_LATITUDE, Math.min(
    MAX_MERCATOR_LATITUDE,
    (bounds.south + bounds.north) / 2,
  ));
  if (!Number.isFinite(longitude) || !Number.isFinite(latitude)) return null;
  const tileCount = 2 ** QUICKLOOK_ZOOM;
  const rawX = Math.floor((longitude + 180) / 360 * tileCount);
  const x = ((rawX % tileCount) + tileCount) % tileCount;
  const radians = latitude * Math.PI / 180;
  const y = Math.max(0, Math.min(tileCount - 1, Math.floor(
    (1 - Math.log(Math.tan(radians) + 1 / Math.cos(radians)) / Math.PI) / 2 * tileCount,
  )));
  const url = urlTemplate
    .replace("{z}", String(QUICKLOOK_ZOOM))
    .replace("{x}", String(x))
    .replace("{y}", String(y));
  return url.includes("{") || !isAllowedByCsp(url) ? null : url;
}

export function opticalQuicklookPlan(scene?: SceneRow | null): OpticalQuicklookPlan | null {
  if (!scene) return null;
  const product = scene.sar_product;
  const metadata = scene.native_metadata && typeof scene.native_metadata === "object"
    ? scene.native_metadata as Record<string, unknown>
    : null;
  const logicalDataset = text(metadata?.logical_dataset);
  if (logicalDataset === "optical.hls.l30.v2" || logicalDataset === "optical.hls.s30.v2") {
    const preview = hlsMapTilePreview(scene);
    const url = preview ? opticalQuicklookTileUrl(preview.urlTemplate, scene) : null;
    return url
      ? { kind: "mpc_tile", url }
      : { kind: "unavailable", title: t("shared.opticalQuicklook.invalidHls") };
  }
  if (!product) return null;

  if (isSentinel2L2aCatalogProduct(product)) {
    if (isMicrosoftSentinel2SearchOnlyProduct(product)) {
      const preview = sentinel2MapTilePreview(product);
      const url = preview ? opticalQuicklookTileUrl(preview.urlTemplate, scene) : null;
      return url
        ? { kind: "mpc_tile", url }
        : { kind: "unavailable", title: t("shared.opticalQuicklook.invalidSentinel2") };
    }
    if (isEarthSearchSentinel2Product(product)) {
      const thumbUrl = earthSearchSentinel2ThumbnailAsset(product)?.url?.trim() ?? "";
      return thumbUrl && isAllowedByCsp(thumbUrl)
        ? { kind: "mpc_tile", url: thumbUrl }
        : { kind: "unavailable", title: t("shared.opticalQuicklook.invalidSentinel2") };
    }
    return quicklookBounds(scene)
      ? { kind: "sentinel2_map_preview" }
      : { kind: "cdse_quicklook" };
  }

  if (isSentinel2L1cCatalogProduct(product)) {
    return opticalQuicklookItemId(scene)
      ? { kind: "cdse_quicklook" }
      : { kind: "unavailable", title: t("shared.opticalQuicklook.invalidSentinel2") };
  }

  if (isLandsatC2L2CatalogProduct(product)) {
    if (product.provider === MPC_LANDSAT_PROVIDER_ID) {
      const preview = landsatMapTilePreview(product);
      const url = preview ? opticalQuicklookTileUrl(preview.urlTemplate, scene) : null;
      return url
        ? { kind: "mpc_tile", url }
        : { kind: "unavailable", title: t("shared.opticalQuicklook.invalidLandsat") };
    }
    if (product.provider === LANDSAT_C2_L2_PROVIDER_ID && landsatBrowseAvailable(product)) {
      return { kind: "landsat_browse" };
    }
    return { kind: "unavailable", title: t("shared.opticalQuicklook.landsatUnavailable") };
  }

  if (logicalDataset.startsWith("optical.") || metadata?.product_kind === "optical_scene") {
    return { kind: "unavailable", title: t("shared.opticalQuicklook.sourceUnavailable") };
  }
  return null;
}

export function createOpticalQuicklookQueue(options: {
  maxConcurrent?: number;
  load: (request: OpticalQuicklookRequest) => Promise<string>;
  loadImage?: (url: string) => Promise<void>;
}): OpticalQuicklookQueue {
  const requests = new Map<string, OpticalQuicklookRequest>();
  const urls = new Map<string, string>();
  const enqueueTokens = new Map<string, Array<{ cancelled: boolean }>>();
  const activeLoads = new Set<string>();
  const loadImage = options.loadImage ?? loadBrowserImage;
  const queue = createThumbnailQueue({
    maxConcurrent: options.maxConcurrent ?? 4,
    load: async (cacheKey) => {
      const request = requests.get(cacheKey);
      if (!request) throw new Error("Quicklook request is no longer active");
      activeLoads.add(cacheKey);
      try {
        const url = await options.load(request);
        if (!isAllowedByCsp(url)) throw new Error("Quicklook URL is not allowed");
        await loadImage(url);
        urls.set(cacheKey, url);
      } finally {
        activeLoads.delete(cacheKey);
        const tokens = enqueueTokens.get(cacheKey);
        if (!tokens || tokens.length === 0) {
          requests.delete(cacheKey);
        }
      }
    },
  });

  return {
    enqueue(request) {
      const cacheKey = opticalQuicklookCacheKey(request.scene, request.batchId);
      if (queue.getCachedState(cacheKey)) return queue.enqueue(cacheKey);
      requests.set(cacheKey, request);
      const token = { cancelled: false };
      const list = enqueueTokens.get(cacheKey) ?? [];
      list.push(token);
      enqueueTokens.set(cacheKey, list);
      return queue.enqueue(cacheKey).finally(() => {
        const current = enqueueTokens.get(cacheKey);
        if (!current) return;
        const idx = current.indexOf(token);
        if (idx >= 0) current.splice(idx, 1);
        if (current.length === 0) {
          enqueueTokens.delete(cacheKey);
          if (!activeLoads.has(cacheKey)) {
            requests.delete(cacheKey);
          }
        }
      });
    },
    cancel(cacheKey) {
      if (queue.getCachedState(cacheKey)) return;
      const list = enqueueTokens.get(cacheKey);
      if (!list || list.length === 0) return;
      const target = list.find((token) => !token.cancelled);
      if (!target) return;
      target.cancelled = true;
      const remainingActive = list.some((token) => !token.cancelled);
      if (!remainingActive && !activeLoads.has(cacheKey)) {
        requests.delete(cacheKey);
        queue.cancel(cacheKey);
      }
    },
    getCachedState: queue.getCachedState,
    getCachedUrl: (cacheKey) => urls.get(cacheKey),
    getInFlightCount: queue.getInFlightCount,
  };
}
