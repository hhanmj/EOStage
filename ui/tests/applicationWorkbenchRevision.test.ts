import assert from "node:assert/strict";
import test from "node:test";

import {
  applicationWorkbenchFromCorePage,
  projectApplicationCoreSnapshot,
  searchSarProducts,
  validateApplicationWorkbenchPageBinding,
  type BoundedWorkbenchPageResult,
} from "../src/lib/bridge.ts";
import {
  clearTestDesktopShellBridge,
  registerTestDesktopShellBridge,
  type DesktopShellBridge,
} from "../src/lib/desktopShellBridge.ts";
import {
  parseCoreRestore,
  projectWorkbenchPage,
} from "../src/lib/coreRestoreProjection.ts";

function corePayload() {
  return {
    readiness: { schema_version: 11, ready: true },
    workspace_context: null,
    region_aoi: {
      region_id: "region-1",
      revision: 3,
      geometry: { type: "Polygon", coordinates: [[[179, -45], [-179, -45], [-179, -35], [179, -35], [179, -45]]] },
    },
    region_aois: [],
    dem_source_identity: null,
    tasks: [],
    legacy_task_warnings: [],
    workbench: {
      batch_id: "batch-1",
      provider_id: "ASF",
      created_at: "2026-08-13T00:00:00Z",
      aoi_name: "AOI",
      query: { provider: "ASF", dataset: "SENTINEL-1", start: null, end: null, target_count: 10_000 },
      search_stats: null,
      candidate_count: 1,
      checked_count: 1,
      current_page_item_count: 1,
      page_index: 0,
      page_size: 100,
      revision: 4,
      deferred: true,
      reason_code: "candidate_payload_deferred",
    },
  };
}

function pagePayload() {
  return {
    batch_id: "batch-1",
    revision: 4,
    page_index: 0,
    page_size: 100,
    total_count: 1,
    checked_count: 1,
    items: [{
      ordinal: 0,
      checked: true,
      item: {
        item_id: "item-0",
        provider_id: "ASF",
        display_name: "scene-0",
        acquisition_datetime: null,
        footprint_geojson: { type: "Polygon", coordinates: [[[179, -45], [-179, -45], [-179, -35], [179, -35], [179, -45]]] },
        metadata: { dataset: "SENTINEL-1", product_family: "SAR" },
        payload: {},
        asset_access: null,
        execution_policy: null,
      },
    }],
  };
}

function pageRender() {
  return {
    region_id: "region-1",
    revision: 3,
    items: [{
      item_id: "item-0",
      render_geometry: { type: "Polygon", coordinates: [[[179, -45], [181, -45], [181, -35], [179, -35], [179, -45]]] },
      render_bbox: { west: 179, south: -45, east: 181, north: -35 },
    }],
  };
}

test("application page projection rejects batch/revision/AOI authority mismatches without masking revision", () => {
  const core = parseCoreRestore(corePayload());
  assert.ok(core);
  const projected = projectWorkbenchPage({
    ok: true,
    readiness: { contract: "application.v1", status: "ready", store_schema_version: 11 },
    page: pagePayload(),
    render: pageRender(),
  });
  assert.equal(projected.ok, true);
  if (!projected.ok || !core) return;
  const base: BoundedWorkbenchPageResult = { ok: true, page: projected.page, scenes: [], checked_item_ids: [] };
  const binding = {
    request: { batch_id: "batch-1", expected_revision: 4, page_index: 0, page_size: 100 },
    base: { batch_id: "batch-1", revision: 4 },
    aoiAuthority: { region_id: "region-1", revision: 3 },
  } as const;
  assert.ok(applicationWorkbenchFromCorePage(core, base, binding));
  assert.equal(applicationWorkbenchFromCorePage(core, {
    ...base,
    page: { ...base.page, revision: 5 },
  }, binding), null);
  assert.equal(applicationWorkbenchFromCorePage(core, {
    ...base,
    page: { ...base.page, batch_id: "batch-other" },
  }, binding), null);
  assert.equal(applicationWorkbenchFromCorePage(core, {
    ...base,
    page: { ...base.page, page_index: 1 },
  }, binding), null, "search hydration must reject a wrong response page");
  assert.equal(applicationWorkbenchFromCorePage(core, {
    ...base,
    page: { ...base.page, page_size: 24 },
  }, binding), null, "search hydration must reject a wrong response page size");
  assert.equal(applicationWorkbenchFromCorePage(core, {
    ...base,
    page: { ...base.page, render: { ...base.page.render!, region_id: "region-other" } },
  }, binding), null);
  assert.equal(applicationWorkbenchFromCorePage(core, {
    ...base,
    page: { ...base.page, render: { ...base.page.render!, revision: 4 } },
  }, binding), null);
  assert.ok(applicationWorkbenchFromCorePage(core, {
    ...base,
    page: { ...base.page, render: null },
  }, { ...binding, aoiAuthority: null }), "legacy no-AOI binding may omit render");
  assert.equal(applicationWorkbenchFromCorePage(core, {
    ...base,
    page: { ...base.page, render: null },
  }, binding), null, "active native AOI must not install a page without whole render authority");
});

test("application page projection rejects a render authority when CoreRestore has no region", () => {
  const core = parseCoreRestore({ ...corePayload(), region_aoi: null });
  assert.ok(core);
  const projected = projectWorkbenchPage({
    ok: true,
    readiness: { contract: "application.v1", status: "ready", store_schema_version: 11 },
    page: pagePayload(),
    render: pageRender(),
  });
  assert.equal(projected.ok, true);
  if (!projected.ok || !core) return;
  assert.equal(applicationWorkbenchFromCorePage(core, {
    ok: true,
    page: projected.page,
    scenes: [],
    checked_item_ids: [],
  }), null);
  const legacyPage = projectWorkbenchPage({
    ok: true,
    readiness: { contract: "application.v1", status: "ready", store_schema_version: 11 },
    page: pagePayload(),
    render: null,
  });
  assert.equal(legacyPage.ok, true);
  if (legacyPage.ok) {
    assert.ok(applicationWorkbenchFromCorePage(core, {
      ok: true,
      page: legacyPage.page,
      scenes: [],
      checked_item_ids: [],
    }));
  }
});

test("production Tauri page hydration rejects deferred, stale, and corrupt responses before context masking", () => {
  const projected = projectWorkbenchPage({
    ok: true,
    readiness: { contract: "application.v1", status: "ready", store_schema_version: 11 },
    page: pagePayload(),
    render: pageRender(),
  });
  assert.equal(projected.ok, true);
  if (!projected.ok) return;
  const base = { batch_id: "batch-1", revision: 4 };
  const request = { batch_id: "batch-1", expected_revision: 4, page_index: 0, page_size: 100 };
  const authority = { region_id: "region-1", revision: 3 };
  const page: BoundedWorkbenchPageResult = { ok: true, page: projected.page, scenes: [], checked_item_ids: [] };
  assert.equal(validateApplicationWorkbenchPageBinding(page, request, base, authority), true);
  assert.equal(validateApplicationWorkbenchPageBinding(page, { ...request, expected_revision: 3 }, base, authority), false);
  assert.equal(validateApplicationWorkbenchPageBinding(page, { ...request, batch_id: "batch-old" }, base, authority), false);
  assert.equal(validateApplicationWorkbenchPageBinding(page, { ...request, page_index: 1 }, base, authority), false);
  assert.equal(validateApplicationWorkbenchPageBinding(page, { ...request, page_size: 24 }, base, authority), false);
  assert.equal(validateApplicationWorkbenchPageBinding({
    ...page,
    page: { ...page.page, revision: 3 },
  }, request, base, authority), false);
  assert.equal(validateApplicationWorkbenchPageBinding({
    ...page,
    page: { ...page.page, render: { ...page.page.render!, revision: 4 } },
  }, request, base, authority), false);
  assert.equal(validateApplicationWorkbenchPageBinding({
    ...page,
    page: { ...page.page, render: null },
  }, request, base, authority), false);
});

test("actual Tauri search hydration rejects wrong pages and missing whole render authority", async () => {
  const authority = {
    region_id: "region-1",
    revision: 3,
    envelope: {
      south: -45,
      north: -35,
      crosses_antimeridian: true,
      display_west: 179,
      display_east: 181,
      request_windows: [
        { west: 179, south: -45, east: 180, north: -35, crs: "EPSG:4326" },
        { west: -180, south: -45, east: -179, north: -35, crs: "EPSG:4326" },
      ],
    },
    render_geometry: { type: "Polygon", coordinates: [[[179, -45], [181, -45], [181, -35], [179, -35], [179, -45]]] },
    render_bbox: { west: 179, south: -45, east: 181, north: -35 },
  };
  const snapshot = {
    ok: true,
    readiness: { contract: "application.v1", status: "ready", store_schema_version: 11 },
    core: corePayload(),
    aoi_display: authority,
  };
  let responsePage: Record<string, unknown> = pagePayload();
  let responseRender: Record<string, unknown> | null = pageRender();
  registerTestDesktopShellBridge({
    kind: "tauri",
    application: {
      sarSearchAndFreeze: async () => snapshot,
      workbenchPage: async () => ({
        ok: true,
        readiness: { contract: "application.v1", status: "ready", store_schema_version: 11 },
        page: responsePage,
        render: responseRender,
      }),
    },
  } as unknown as DesktopShellBridge);
  const request = {
    provider: "ASF",
    dataset: "SENTINEL-1",
    max_results: 1,
  };
  const options = {
    request_id: "m1-test",
    batch_id: "batch-1",
    created_at: "2026-08-13T00:00:00Z",
    aoi_name: "AOI",
    aoi_geojson: null,
    page_size: 100,
  };
  try {
    responsePage = { ...pagePayload(), page_index: 1 };
    assert.equal((await searchSarProducts(request, options)).ok, false);
    responsePage = pagePayload();
    const validResult = await searchSarProducts(request, options);
    assert.equal(validResult.ok, true);
    responseRender = null;
    assert.equal((await searchSarProducts(request, options)).ok, false);
  } finally {
    clearTestDesktopShellBridge();
  }
});
