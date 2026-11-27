import {
  getCdseQuicklook,
  getLandsatBrowse,
  getSentinel2MapPreview,
  getUsgsSessionStatus,
} from "../../lib/bridge.ts";
import { isDesktopControlledPreviewDataUrl } from "../../lib/catalogPreview.ts";
import { sentinel2ResolvedMapTilePreview } from "../../lib/sentinel2Catalog.ts";
import {
  createOpticalQuicklookQueue,
  opticalQuicklookItemId,
  opticalQuicklookPlan,
  opticalQuicklookTileUrl,
  type OpticalQuicklookRequest,
} from "./opticalQuicklookLogic.ts";
import { isAllowedByCsp, loadBrowserImage } from "./thumbnailLogic.ts";

let requestSequence = 0;

function nextRequestId(): string {
  requestSequence += 1;
  return `optical-quicklook-${Date.now()}-${requestSequence}`;
}

async function loadDesktopImage(dataUrl: string): Promise<string> {
  if (!isDesktopControlledPreviewDataUrl(dataUrl)) throw new Error("Invalid desktop quicklook");
  await loadBrowserImage(dataUrl);
  return dataUrl;
}

async function loadMpcImage(url: string): Promise<string> {
  if (!isAllowedByCsp(url)) throw new Error("MPC quicklook URL is not allowed");
  await loadBrowserImage(url);
  return url;
}

async function loadCdseQuicklook(request: OpticalQuicklookRequest): Promise<string> {
  const batchId = request.batchId?.trim();
  if (!batchId) throw new Error("CDSE quicklook requires a frozen batch");
  const result = await getCdseQuicklook({
    request_id: nextRequestId(),
    batch_id: batchId,
    item_id: opticalQuicklookItemId(request.scene),
  });
  if (!result.ok) throw new Error("CDSE quicklook failed");
  return loadDesktopImage(result.quicklook.data_url);
}

const MPC_QUICKLOOK_TIMEOUT_MS = 3500;

function raceWithTimeout<T>(promise: Promise<T>, ms: number, message: string): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  return Promise.race([
    promise,
    new Promise<never>((_, reject) => {
      timer = setTimeout(() => reject(new Error(message)), ms);
    }),
  ]).finally(() => {
    if (timer !== undefined) clearTimeout(timer);
  });
}

async function loadCdseWithMpcPreference(request: OpticalQuicklookRequest): Promise<string> {
  const batchId = request.batchId?.trim();
  const product = request.scene.sar_product;
  if (batchId && product) {
    try {
      const itemId = opticalQuicklookItemId(request.scene);
      const url = await raceWithTimeout(
        (async () => {
          const result = await getSentinel2MapPreview({
            request_id: nextRequestId(),
            batch_id: batchId,
            item_id: itemId,
          });
          if (!result.ok || result.preview.product_key !== itemId) {
            throw new Error("MPC quicklook identity mismatch");
          }
          const preview = sentinel2ResolvedMapTilePreview(product, result.preview.map_item_id);
          const resolved = preview ? opticalQuicklookTileUrl(preview.urlTemplate, request.scene) : null;
          if (!resolved) throw new Error("MPC quicklook tile is unavailable");
          return loadMpcImage(resolved);
        })(),
        MPC_QUICKLOOK_TIMEOUT_MS,
        "MPC quicklook timeout",
      );
      return url;
    } catch {
      // Fall through immediately to anonymous CDSE reviewed quicklook.
    }
  }
  return loadCdseQuicklook(request);
}

async function loadLandsatBrowse(request: OpticalQuicklookRequest): Promise<string> {
  const batchId = request.batchId?.trim();
  if (!batchId) throw new Error("Landsat browse requires a frozen batch");
  const status = await getUsgsSessionStatus();
  if (!status.ok || !status.status.authenticated) {
    throw new Error("Landsat browse requires an authenticated session");
  }
  const result = await getLandsatBrowse({
    request_id: nextRequestId(),
    batch_id: batchId,
    item_id: opticalQuicklookItemId(request.scene),
  });
  if (!result.ok) throw new Error("Landsat browse failed");
  return loadDesktopImage(result.browse.data_url);
}

export async function loadOpticalQuicklook(request: OpticalQuicklookRequest): Promise<string> {
  const plan = opticalQuicklookPlan(request.scene);
  if (!plan || plan.kind === "unavailable") throw new Error("Quicklook is unavailable");
  if (plan.kind === "mpc_tile") return loadMpcImage(plan.url);
  if (plan.kind === "sentinel2_map_preview") {
    return loadCdseWithMpcPreference(request);
  }
  if (plan.kind === "cdse_quicklook") {
    return loadCdseQuicklook(request);
  }
  return loadLandsatBrowse(request);
}

export const sharedOpticalQuicklookQueue = createOpticalQuicklookQueue({
  maxConcurrent: 4,
  load: loadOpticalQuicklook,
  // The adapter loads once so an MPC image failure can enter the CDSE fallback.
  loadImage: async () => {},
});
