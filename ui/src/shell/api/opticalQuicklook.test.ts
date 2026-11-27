import assert from "node:assert/strict";
import test from "node:test";
import type { SarProduct } from "../../lib/generated/v3-contracts.ts";
import { projectSarProductToScene } from "../../lib/sarProduct.ts";
import { projectSnapshotItemToSceneRow } from "./rowProjection.ts";
import type { SnapshotItem } from "./types.ts";
import {
  createOpticalQuicklookQueue,
  opticalQuicklookCacheKey,
  opticalQuicklookPlan,
  type OpticalQuicklookRequest,
} from "./opticalQuicklookLogic.ts";

const footprint = {
  type: "Polygon",
  coordinates: [[[110, 30], [111, 30], [111, 31], [110, 31], [110, 30]]],
};

function sceneFor(product: Partial<SarProduct>) {
  return projectSarProductToScene({
    product_key: "product-key",
    product_id: "product-id",
    scene_id: "scene-name",
    provider: "provider",
    dataset: "dataset",
    footprint_geojson: footprint,
    native_metadata: {},
    ...product,
  } as SarProduct);
}

function mpcSentinel2() {
  const itemId = "S2B_MSIL2A_20250129T022849_R046_T51RUH_20250129T040545";
  const key = `PROVIDER::microsoft.planetary-computer::COLLECTION::sentinel-2-l2a::ITEM::${itemId}`;
  return sceneFor({
    product_key: key,
    provider_product_key: key,
    provider_product_id: itemId,
    product_id: itemId,
    scene_id: "S2B_MSIL2A_20250129T022849_N0511_R046_T51RUH_20250129T040545.SAFE",
    provider: "microsoft.planetary-computer",
    native_metadata: {
      logical_dataset: "optical.sentinel-2.l2a",
      native_collection_id: "sentinel-2-l2a",
      collection_profile_id: "planetary-computer.sentinel-2-l2a.scene.v1",
      identity_rule_id: "planetary-computer.sentinel-2-l2a.identity.v1",
      filename_rule_id: "planetary-computer.sentinel-2-l2a.product-uri.v1",
      resolver_policy_id: "planetary-computer.sentinel-2-l2a.search-only.v1",
      source_kind: "sentinel2",
      product_kind: "optical_scene",
    },
  });
}

function cdseSentinel2() {
  const key = "PROVIDER::copernicus.dataspace.odata::COLLECTION::SENTINEL-2::ITEM::165ba4c9-997c-4cea-970f-33ab9bdb8084";
  return sceneFor({
    product_key: key,
    provider_product_key: key,
    provider_product_id: "165ba4c9-997c-4cea-970f-33ab9bdb8084",
    product_id: "165ba4c9-997c-4cea-970f-33ab9bdb8084",
    scene_id: "S2B_MSIL2A_20250129T022849_N0511_R046_T51RUH_20250129T040545.SAFE",
    provider: "copernicus.dataspace.odata",
    native_metadata: {
      logical_dataset: "optical.sentinel-2.l2a",
      native_collection_id: "SENTINEL-2",
      source_kind: "sentinel2",
      product_kind: "optical_scene",
    },
  });
}

function hlsScene(type: "S30" | "L30") {
  const itemId = `HLS.${type}.T56LPN.2026302T235739.v2.0`;
  const collection = `hls2-${type.toLowerCase()}`;
  const key = `PROVIDER::microsoft.planetary-computer::COLLECTION::${collection}::ITEM::${itemId}`;
  return projectSnapshotItemToSceneRow({
    item_id: key,
    provider_id: "microsoft.planetary-computer",
    display_name: itemId,
    footprint_geojson: footprint,
    metadata: {
      logical_dataset: `optical.hls.${type.toLowerCase()}.v2`,
      native_collection_id: collection,
      provider_product_id: itemId,
      provider_product_key: key,
      product_kind: "optical_scene",
    },
    payload: {},
  } as SnapshotItem);
}

test("HLS quicklooks use validated frozen S30 and L30 snapshot identities", () => {
  for (const type of ["S30", "L30"] as const) {
    const scene = hlsScene(type);
    const plan = opticalQuicklookPlan(scene);
    assert.equal(plan?.kind, "mpc_tile");
    if (plan?.kind !== "mpc_tile") continue;
    const url = new URL(plan.url);
    assert.equal(url.origin, "https://planetarycomputer.microsoft.com");
    // Whole-scene centered preview replaces single Z=8 tile corner wedges per User Feedback #6.
    assert.equal(url.pathname, "/api/data/v1/item/preview.png");
    assert.equal(url.searchParams.get("collection"), `hls2-${type.toLowerCase()}`);
    assert.equal(url.searchParams.get("item"), `HLS.${type}.T56LPN.2026302T235739.v2.0`);
    assert.deepEqual(url.searchParams.getAll("assets"), ["B04", "B03", "B02"]);
    assert.equal(url.searchParams.get("rescale"), "0,3000");
    assert.equal(url.searchParams.get("color_formula"), "gamma RGB 2.7, saturation 1.5, sigmoidal RGB 15 0.55");
    assert.equal(url.searchParams.get("nodata"), "-9999");
    assert.equal(url.searchParams.get("max_size"), "192");
    assert.equal(url.searchParams.get("format"), "png");
  }
});

test("HLS quicklooks reject invalid source, collection, item, key and footprint", () => {
  const scene = hlsScene("S30");
  const metadata = scene.native_metadata!;
  const invalid = [
    { ...scene, native_metadata: { ...metadata, native_collection_id: "hls2-l30" } },
    { ...scene, native_metadata: { ...metadata, provider_product_id: "bad&id=other" } },
    { ...scene, native_metadata: { ...metadata, provider_product_id: "HLS.L30.T56LPN.2026302T235739.v2.0" } },
    { ...scene, native_metadata: { ...metadata, provider_product_key: "forged" } },
    { ...scene, product_key: "forged" },
    { ...scene, provider: "other" },
    { ...scene, footprint_geojson: null },
  ];
  for (const row of invalid) {
    assert.deepEqual(opticalQuicklookPlan(row), { kind: "unavailable", title: "HLS 快视图身份或范围无效" });
  }
});

test("optical quicklook routes exact MPC and CDSE Sentinel-2 identities through anonymous MPC tiles", () => {
  const mpcPlan = opticalQuicklookPlan(mpcSentinel2());
  assert.equal(mpcPlan?.kind, "mpc_tile");
  if (mpcPlan?.kind === "mpc_tile") {
    assert.match(mpcPlan.url, /^https:\/\/planetarycomputer\.microsoft\.com\/api\/data\/v1\/item\/preview\.png\?/);
    assert.match(mpcPlan.url, /collection=sentinel-2-l2a/);
    assert.match(mpcPlan.url, /assets=visual/);
    assert.match(mpcPlan.url, /max_size=192/);
  }

  assert.equal(opticalQuicklookPlan(cdseSentinel2())?.kind, "sentinel2_map_preview");
  const cdseL1c = sceneFor({
    product_key: "PROVIDER::copernicus.dataspace.odata::COLLECTION::SENTINEL-2::ITEM::l1c-uuid-1",
    provider_product_key: "PROVIDER::copernicus.dataspace.odata::COLLECTION::SENTINEL-2::ITEM::l1c-uuid-1",
    provider_product_id: "l1c-uuid-1",
    product_id: "l1c-uuid-1",
    scene_id: "S2A_MSIL1C_20260726T125321_N0512_R138_T27VXL_20260726T221014.SAFE",
    provider: "copernicus.dataspace.odata",
    native_metadata: {
      logical_dataset: "optical.sentinel-2.l1c",
      native_collection_id: "SENTINEL-2",
      source_kind: "sentinel2",
      product_kind: "optical_scene",
    },
  });
  assert.equal(opticalQuicklookPlan(cdseL1c)?.kind, "cdse_quicklook");
  assert.equal(opticalQuicklookPlan({
    ...mpcSentinel2(),
    sar_product: { ...mpcSentinel2().sar_product!, product_key: "forged" },
  })?.kind, "unavailable");
  assert.equal(opticalQuicklookPlan({ ...mpcSentinel2(), footprint_geojson: null })?.kind, "unavailable");
});

test("optical quicklook routes Landsat and reports unsupported sources without inventing URLs", () => {
  const landsatItem = "LC09_L2SP_123045_20260101_02_T1";
  const landsatKey = `PROVIDER::microsoft.planetary-computer::COLLECTION::landsat-c2-l2::ITEM::${landsatItem}`;
  const landsat = sceneFor({
    product_key: landsatKey,
    provider_product_key: landsatKey,
    provider_product_id: landsatItem,
    product_id: landsatItem,
    scene_id: "LC09_L2SP_123045_20260101_20260103_02_T1",
    provider: "microsoft.planetary-computer",
    native_metadata: {
      logical_dataset: "optical.landsat.c2.l2",
      native_collection_id: "landsat-c2-l2",
      source_kind: "landsat",
      product_kind: "optical_scene",
      properties: { entity_id: landsatItem, browse_available: true },
    },
  });
  const mpcPlan = opticalQuicklookPlan(landsat);
  assert.equal(mpcPlan?.kind, "mpc_tile");
  if (mpcPlan?.kind === "mpc_tile") {
    assert.match(mpcPlan.url, /collection=landsat-c2-l2/);
    assert.match(mpcPlan.url, /assets=red&assets=green&assets=blue/);
  }

  assert.equal(opticalQuicklookPlan({
    ...landsat,
    provider: "usgs.eros.m2m",
    sar_product: {
      ...landsat.sar_product!,
      provider: "usgs.eros.m2m",
      native_metadata: {
        ...landsat.sar_product!.native_metadata,
        native_collection_id: "landsat_ot_c2_l2",
      },
    },
  })?.kind, "landsat_browse");

  assert.equal(opticalQuicklookPlan({
    ...landsat,
    provider: "element84.earth-search",
    sar_product: { ...landsat.sar_product!, provider: "element84.earth-search" },
  })?.kind, "unavailable");

  const hls = opticalQuicklookPlan(sceneFor({
    provider: "microsoft.planetary-computer",
    native_metadata: { logical_dataset: "optical.hls.l30.v2", product_kind: "optical_scene" },
  }));
  assert.equal(hls?.kind, "unavailable");
  assert.match(hls?.kind === "unavailable" ? hls.title : "", /HLS/);
  assert.equal(opticalQuicklookPlan(sceneFor({
    provider: "asf.catalog",
    native_metadata: { dataset: "SENTINEL-1", product_kind: "sar_grd" },
  })), null);
});

test("optical quicklook queue caches per batch/item, coalesces requests, bounds concurrency and never retries failures", async () => {
  const pending = new Map<string, { resolve: (url: string) => void; reject: (error: Error) => void }>();
  const calls: string[] = [];
  let active = 0;
  let peak = 0;
  const queue = createOpticalQuicklookQueue({
    load: (request) => {
      const key = opticalQuicklookCacheKey(request.scene, request.batchId);
      calls.push(key);
      active += 1;
      peak = Math.max(peak, active);
      return new Promise<string>((resolve, reject) => {
        pending.set(key, {
          resolve: (url) => { active -= 1; resolve(url); },
          reject: (error) => { active -= 1; reject(error); },
        });
      });
    },
    loadImage: async () => {},
  });
  const requests = Array.from({ length: 6 }, (_, index): OpticalQuicklookRequest => ({
    scene: { ...mpcSentinel2(), scene_id: `scene-${index}`, product_key: `scene-${index}` },
    batchId: index === 5 ? "batch-b" : "batch-a",
  }));
  const results = requests.map((request) => queue.enqueue(request));
  const duplicate = queue.enqueue(requests[0]!);
  assert.equal(calls.length, 4);
  assert.equal(peak, 4);

  for (let index = 0; index < 4; index += 1) {
    const request = requests[index]!;
    const key = opticalQuicklookCacheKey(request.scene, request.batchId);
    if (index === 1) pending.get(key)!.reject(new Error("failed"));
    else pending.get(key)!.resolve(`https://planetarycomputer.microsoft.com/${index}.png`);
    await Promise.resolve();
    await Promise.resolve();
  }
  assert.equal(calls.length, 6);
  for (let index = 4; index < 6; index += 1) {
    const request = requests[index]!;
    pending.get(opticalQuicklookCacheKey(request.scene, request.batchId))!.resolve(`https://planetarycomputer.microsoft.com/${index}.png`);
  }
  assert.deepEqual(await Promise.all(results), ["loaded", "failed", "loaded", "loaded", "loaded", "loaded"]);
  assert.equal(await duplicate, "loaded");
  const firstKey = opticalQuicklookCacheKey(requests[0]!.scene, requests[0]!.batchId);
  const failedKey = opticalQuicklookCacheKey(requests[1]!.scene, requests[1]!.batchId);
  assert.equal(queue.getCachedUrl(firstKey), "https://planetarycomputer.microsoft.com/0.png");
  assert.equal(queue.getCachedState(failedKey), "failed");
  await queue.enqueue(requests[0]!);
  await queue.enqueue(requests[1]!);
  assert.equal(calls.length, 6);
});

test("optical quicklook queue cancel never poisons in-flight, shared queued, or solo cancelled entries", async () => {
  const pending = new Map<string, { resolve: (url: string) => void; reject: (error: Error) => void }>();
  const calls: string[] = [];
  const queue = createOpticalQuicklookQueue({
    maxConcurrent: 2,
    load: (request) => {
      const key = opticalQuicklookCacheKey(request.scene, request.batchId);
      calls.push(key);
      return new Promise<string>((resolve, reject) => {
        pending.set(key, { resolve, reject });
      });
    },
    loadImage: async () => {},
  });

  const makeReq = (id: string): OpticalQuicklookRequest => ({
    scene: { ...mpcSentinel2(), scene_id: id, product_key: id },
    batchId: "batch-cancel-guard",
  });

  // 1. Cancelling while in-flight must NOT delete request or poison state.
  const r0 = makeReq("inflight-0");
  const k0 = opticalQuicklookCacheKey(r0.scene, r0.batchId);
  const p0 = queue.enqueue(r0);
  queue.cancel(k0);
  const p0Retry = queue.enqueue(r0);
  const r1 = makeReq("inflight-1");
  const k1 = opticalQuicklookCacheKey(r1.scene, r1.batchId);
  const p1 = queue.enqueue(r1);

  // 2. Fill pending queue: r2 enqueued twice, cancelled once; r3 enqueued once and cancelled.
  const r2 = makeReq("queued-shared");
  const k2 = opticalQuicklookCacheKey(r2.scene, r2.batchId);
  const p2a = queue.enqueue(r2);
  const p2b = queue.enqueue(r2);
  queue.cancel(k2);

  const r3 = makeReq("queued-solo");
  const k3 = opticalQuicklookCacheKey(r3.scene, r3.batchId);
  const p3Cancelled = queue.enqueue(r3);
  queue.cancel(k3);
  assert.equal(await p3Cancelled, "failed");
  assert.equal(queue.getCachedState(k3), undefined);

  // Complete in-flight r0 & r1 -> r2 runs for remaining subscriber p2b!
  pending.get(k0)!.resolve("https://planetarycomputer.microsoft.com/r0.png");
  pending.get(k1)!.resolve("https://planetarycomputer.microsoft.com/r1.png");
  await Promise.resolve();
  await Promise.resolve();
  assert.equal(await p0, "loaded");
  assert.equal(await p0Retry, "loaded");
  assert.equal(await p1, "loaded");
  assert.equal(queue.getCachedState(k0), "loaded");
  assert.equal(queue.getCachedUrl(k0), "https://planetarycomputer.microsoft.com/r0.png");

  assert.equal(calls.includes(k2), true);
  assert.equal(calls.includes(k3), false);
  pending.get(k2)!.resolve("https://planetarycomputer.microsoft.com/r2.png");
  assert.equal(await p2b, "loaded");
  assert.equal(queue.getCachedUrl(k2), "https://planetarycomputer.microsoft.com/r2.png");
  await p2a;

  // 3. Re-enqueuing solo-cancelled r3 succeeds normally.
  const p3Reload = queue.enqueue(r3);
  pending.get(k3)!.resolve("https://planetarycomputer.microsoft.com/r3.png");
  assert.equal(await p3Reload, "loaded");
  assert.equal(queue.getCachedUrl(k3), "https://planetarycomputer.microsoft.com/r3.png");
});
