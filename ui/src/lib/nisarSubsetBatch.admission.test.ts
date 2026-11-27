import assert from "node:assert/strict";
import { registerHooks } from "node:module";
import test from "node:test";

import type { BoundedWorkbenchPageResult, DownloadBatchContext, SceneRow } from "./bridge.ts";
import type { DesktopWorkbenchPageRequest } from "./desktopShellBridge.ts";
let batchImportSequence = 0;

async function importBatchLoader() {
  const hooks = registerHooks({
    resolve(specifier, context, nextResolve) {
      if (specifier === "@/data/adminBoundaries") {
        return { url: new URL("../data/adminBoundaries.ts", import.meta.url).href, shortCircuit: true };
      }
      if (specifier === "./sarSearchForm") {
        return { url: new URL("./sarSearchForm.ts", context.parentURL).href, shortCircuit: true };
      }
      return nextResolve(specifier, context);
    },
  });
  try {
    batchImportSequence += 1;
    return await import(`./nisarSubsetBatch.ts?admission-${batchImportSequence}`);
  } finally {
    hooks.deregister();
  }
}

const context: DownloadBatchContext = {
  batch_id: "batch-nisar",
  created_at: "2026-08-01T00:00:00Z",
  aoi_name: "AOI",
  aoi_geojson: null,
  query: { logical_dataset: "sar.nisar.l.rslc.provisional.v1" },
  page_index: 0,
  page_size: 2,
  revision: 7,
  total_count: 2,
};

function scene(itemId: string, provider = "asf.nasa.earthdata"): SceneRow {
  return {
    product_key: itemId,
    product_id: itemId,
    provider,
    dataset: "sar.nisar.l.rslc.provisional.v1",
    scene_id: itemId,
    platform: "NISAR",
    product_type: "RSLC",
    beam_mode: "",
    polarization: "",
    acquisition_datetime: "2026-08-01T00:00:00Z",
    absolute_orbit: null,
    relative_orbit: null,
    path: null,
    frame: null,
    orbit_direction: "",
    has_url: false,
    sar_product: {
      product_key: itemId,
      product_id: itemId,
      scene_id: itemId,
      provider,
      dataset: "sar.nisar.l.rslc.provisional.v1",
      platform: "NISAR",
      instrument: "L-SAR",
      processing_level: "RSLC",
      product_family: "NISAR_L1_RSLC_PROVISIONAL_V1",
      beam_mode: null,
      beam_swath: null,
      polarization: null,
      flight_direction: null,
      look_direction: null,
      relative_orbit: null,
      absolute_orbit: null,
      frame: null,
      asf_frame: null,
      path: null,
      acquisition_start: "2026-08-01T00:00:00Z",
      acquisition_stop: null,
      footprint_geojson: null,
      footprint_bbox: null,
      download_url: null,
      file_name: `${itemId}.h5`,
      file_size_remote: 1,
      checksum: null,
      local_path: null,
      native_metadata: {},
      burst: null,
    },
  };
}

function page(
  checkedIds: string[] = ["item-2"],
  providerIds: string[] = ["asf.nasa.earthdata", "asf.nasa.earthdata"],
  scenes: SceneRow[] = [scene("item-1"), scene("item-2")],
  checkedCount = 1,
): BoundedWorkbenchPageResult {
  const ids = ["item-1", "item-2"];
  return {
    ok: true,
    page: {
      batch_id: "batch-nisar",
      revision: 7,
      page_index: 0,
      page_size: 2,
      total_count: 2,
      checked_count: checkedCount,
      render: null,
      items: ids.map((itemId, ordinal) => ({
        ordinal,
        checked: checkedIds.includes(itemId),
        item: {
          item_id: itemId,
          provider_id: providerIds[ordinal]!,
          display_name: itemId,
          acquisition_datetime: null,
          footprint_geojson: null,
          metadata: {},
          payload: {},
          asset_access: null,
          execution_policy: null,
        },
      })),
    },
    scenes,
    checked_item_ids: checkedIds,
  };
}

async function load(
  value: BoundedWorkbenchPageResult,
  baseContext: DownloadBatchContext = context,
) {
  const { loadCompleteNisarSubsetBatch } = await importBatchLoader();
  return loadCompleteNisarSubsetBatch({
    baseContext,
    aoiAuthority: null,
    pageSize: 2,
    loadPage: async () => value,
  });
}

test("complete subset batch rejects a checked-id count below frozen checked_count", async () => {
  const result = await load(page([]));
  assert.equal(result.ok, false);
  if (!result.ok) assert.match(result.error, /勾选|checked/i);
});

test("complete subset batch rejects mixed provider identities even when each row projection agrees", async () => {
  const result = await load(page(
    ["item-2"],
    ["asf.nasa.earthdata", "other.provider"],
    [scene("item-1"), scene("item-2", "other.provider")],
  ));
  assert.equal(result.ok, false);
  if (!result.ok) assert.match(result.error, /provider/i);
});

test("complete subset batch rejects any non-NISAR-RSLC row", async () => {
  const invalid = scene("item-2");
  invalid.sar_product = { ...invalid.sar_product!, processing_level: "GRD" };
  const result = await load(page(["item-2"], undefined, [scene("item-1"), invalid]));
  assert.equal(result.ok, false);
  if (!result.ok) assert.match(result.error, /NISAR RSLC/);
});

test("complete subset batch rejects page checked_count below the frozen entrance count", async () => {
  const result = await load(
    page([], undefined, undefined, 0),
    { ...context, checked_count: 1 },
  );
  assert.equal(result.ok, false);
  if (!result.ok) assert.match(result.error, /勾选|checked/i);
});

test("complete subset batch rejects checked-id set drift from the frozen complete set", async () => {
  const result = await load(
    page(["item-2"]),
    { ...context, checked_count: 1, checked_item_ids: ["item-1"] },
  );
  assert.equal(result.ok, false);
  if (!result.ok) assert.match(result.error, /勾选|checked/i);
});

test("complete subset batch rejects a uniform provider that differs from the frozen query provider", async () => {
  const provider = "other.provider";
  const result = await load(
    page(
      ["item-2"],
      [provider, provider],
      [scene("item-1", provider), scene("item-2", provider)],
    ),
    {
      ...context,
      query: { ...context.query, provider: "asf.nasa.earthdata" },
    },
  );
  assert.equal(result.ok, false);
  if (!result.ok) assert.match(result.error, /provider/i);
});

test("a 975-scene frozen NISAR SearchBatch opens through bounded internal pages", async () => {
  const { loadCompleteNisarSubsetBatch } = await importBatchLoader();
  const totalCount = 975;
  const pageSize = 50;
  const requests: DesktopWorkbenchPageRequest[] = [];
  const result = await loadCompleteNisarSubsetBatch({
    baseContext: {
      ...context,
      page_size: 250,
      total_count: totalCount,
      candidate_count: totalCount,
      checked_count: 1,
      checked_item_ids: ["item-974"],
    },
    aoiAuthority: null,
    pageSize,
    loadPage: async (request) => {
      requests.push(request);
      const start = request.page_index * request.page_size;
      const ids = Array.from(
        { length: Math.min(request.page_size, totalCount - start) },
        (_, offset) => `item-${start + offset}`,
      );
      return {
        ok: true as const,
        page: {
          batch_id: context.batch_id,
          revision: context.revision!,
          page_index: request.page_index,
          page_size: request.page_size,
          total_count: totalCount,
          checked_count: 1,
          render: null,
          items: ids.map((itemId, offset) => ({
            ordinal: start + offset,
            checked: itemId === "item-974",
            item: frozenNisarItemForAdmission(itemId),
          })),
        },
        scenes: ids.map((itemId) => scene(itemId)),
        checked_item_ids: ids.includes("item-974") ? ["item-974"] : [],
      };
    },
  });

  assert.equal(result.ok, true);
  if (result.ok) assert.equal(result.items.length, totalCount);
  assert.equal(requests.length, Math.ceil(totalCount / pageSize));
});

function frozenNisarItemForAdmission(itemId: string) {
  return {
    item_id: itemId,
    provider_id: "asf.nasa.earthdata",
    display_name: itemId,
    acquisition_datetime: null,
    footprint_geojson: null,
    metadata: {},
    payload: {},
    asset_access: null,
    execution_policy: null,
  };
}
