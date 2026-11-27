import assert from "node:assert/strict";
import test from "node:test";

import React, { act } from "react";

import type { AdminBoundary, AoiPreviewOk, Bbox, BoundedWorkbenchPageResult, DownloadBatchContext, SceneRow } from "@/lib/bridge";
import { loadCompleteNisarSubsetBatch } from "@/lib/nisarSubsetBatch";
import type { SarProduct } from "@/lib/generated/v3-contracts";
import {
  BHOONIDHI_COLLECTION_ID, BHOONIDHI_LOGICAL_DATASET,
  BHOONIDHI_PROFILE_ID, BHOONIDHI_PROVIDER_ID,
} from "@/lib/bhoonidhiSsarCatalog";
import {
  createNisarSubsetWorkspaceDraft,
  type NisarSubsetWorkspaceDraft,
  type NisarSubsetWorkspaceRow,
} from "@/lib/nisarSubsetWorkspace";
import { cleanup, fireEvent, render } from "../test-utils/render-harness";
import {
  NisarSubsetWorkspace,
  type NisarSubsetWorkspacePreflight,
} from "./NisarSubsetWorkspace";

const geometry = {
  type: "Polygon",
  coordinates: [[[110, 30], [111, 30], [111, 31], [110, 30]]],
};

const bbox: Bbox = {
  west: 110,
  east: 111,
  south: 30,
  north: 31,
  crs: "EPSG:4326",
};

function product(itemId: string): SarProduct {
  return {
    product_key: itemId,
    product_id: itemId,
    scene_id: itemId,
    provider: "asf.nasa.earthdata",
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
    footprint_geojson: geometry,
    footprint_bbox: bbox,
    download_url: null,
    file_name: `${itemId}.h5`,
    file_size_remote: 1_000,
    checksum: null,
    local_path: null,
    native_metadata: {},
    burst: null,
  };
}

function row(itemId: string, polarizations: string[] = ["HH", "HV"]): NisarSubsetWorkspaceRow {
  return {
    itemId,
    product: product(itemId),
    subsetEligible: true,
    blockedReason: null,
    overlapState: "unchecked",
    availableSelections: [{ frequency: "A", polarizations }],
  };
}

const rows = [row("scene-1"), row("scene-2", ["HH"])];

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((next) => { resolve = next; });
  return { promise, resolve };
}

function renderWorkspace(
  initialDraft: NisarSubsetWorkspaceDraft,
  overrides: Partial<React.ComponentProps<typeof NisarSubsetWorkspace>> = {},
) {
  return render(
    <NisarSubsetWorkspace
      initialDraft={initialDraft}
      rows={rows}
      sourceBatchLabel="batch-1"
      currentAoiName="当前工作 AOI"
      mapBbox={bbox}
      onClose={() => undefined}
      onPreviewItem={() => undefined}
      onReview={() => undefined}
      {...overrides}
    />,
  );
}

test("subset workspace keeps scene selection and AOI isolated when cancelled", () => {
  const draft = createNisarSubsetWorkspaceDraft("batch-1", rows, new Set(["scene-1"]), null, geometry);
  let closed = 0;
  const view = renderWorkspace(draft, { onClose: () => { closed += 1; } });

  assert.ok(view.getByRole("dialog", { name: "NISAR RSLC AOI 裁剪下载" }));
  fireEvent.click(view.getByRole("checkbox", { name: /scene-2/ }));
  assert.equal(view.getByText("已选 2 景").textContent, "已选 2 景");
  assert.equal((view.getByRole("checkbox", { name: /scene-1/ }) as HTMLInputElement).checked, true);
  assert.equal((view.getByRole("checkbox", { name: /scene-2/ }) as HTMLInputElement).checked, true);
  fireEvent.click(view.getByRole("button", { name: "清空范围" }));
  assert.ok(view.getByText("尚未选择 AOI"));
  fireEvent.click(view.getByRole("button", { name: "取消" }));

  assert.equal(closed, 1);
  assert.deepEqual(draft.selectedItemIds, ["scene-1"]);
  assert.deepEqual(draft.requestedGeometry, geometry);
  cleanup();
});

test("workspace freezes row capabilities after opening", () => {
  const mutableRows = [row("scene-1")];
  const draft = createNisarSubsetWorkspaceDraft(
    "batch-1", mutableRows, new Set(["scene-1"]), null, geometry,
  );
  const view = renderWorkspace(draft, { rows: mutableRows });

  mutableRows[0].product.product_id = "mutated-scene";
  fireEvent.click(view.getByRole("button", { name: "选择行政区" }));

  assert.ok(view.getByRole("checkbox", { name: /scene-1/ }));
  assert.equal(view.queryByRole("checkbox", { name: /mutated-scene/ }), null);
  cleanup();
});

test("workspace requires authoritative preflight before reviewing a common selection", async () => {
  const initialDraft = createNisarSubsetWorkspaceDraft(
    "batch-1", rows, new Set(["scene-1", "scene-2"]), null, geometry,
  );
  let reviewed: NisarSubsetWorkspaceDraft | null = null;
  let preflightCalls = 0;
  const view = renderWorkspace(initialDraft, {
    onPreflight: async () => {
      preflightCalls += 1;
      return {
        sourceBatchId: "batch-1",
        workbenchRevision: 7,
        eligibleItems: rows.map((candidate) => ({
          sourceItemId: candidate.itemId,
          effectiveGeometry: geometry,
          availableSelections: [{ frequency: "A" as const, polarizations: candidate.itemId === "scene-1" ? ["HH", "HV"] : ["HH"] }],
        })),
        excludedItems: [],
        commonSelections: [{ frequency: "A" as const, polarizations: ["HH"] }],
      };
    },
    onReview: (draft) => { reviewed = draft; },
  });

  assert.equal(view.queryByRole("radio", { name: "Frequency A" }), null);
  assert.equal((view.getByRole("button", { name: "检查相交与能力" }) as HTMLButtonElement).disabled, false);
  fireEvent.click(view.getByRole("button", { name: "检查相交与能力" }));
  await view.findByRole("radio", { name: "Frequency A" });
  assert.ok(view.getByRole("group", { name: "频率通道（单选）" }));
  assert.ok(view.getByRole("group", { name: "极化（可多选）" }));
  assert.equal(preflightCalls, 1);
  fireEvent.click(view.getByRole("radio", { name: "Frequency A" }));
  fireEvent.click(view.getByRole("checkbox", { name: "Polarization HH" }));
  fireEvent.click(view.getByRole("button", { name: "检查并下载" }));

  assert.ok(reviewed);
  assert.equal(reviewed.selectedFrequency, "A");
  assert.deepEqual(reviewed.selectedPolarizations, ["HH"]);
  assert.deepEqual(reviewed.requestedGeometry, geometry);
  cleanup();
});

test("workspace applies only the latest preflight after scene selection changes", async () => {
  const initialDraft = createNisarSubsetWorkspaceDraft(
    "batch-1", rows, new Set(["scene-1"]), null, geometry,
  );
  const stale = deferred<NisarSubsetWorkspacePreflight>();
  const current = deferred<NisarSubsetWorkspacePreflight>();
  const requestedDrafts: NisarSubsetWorkspaceDraft[] = [];
  const staleResult: NisarSubsetWorkspacePreflight = {
    sourceBatchId: "batch-1",
    workbenchRevision: 7,
    eligibleItems: rows.map((candidate) => ({
      sourceItemId: candidate.itemId,
      effectiveGeometry: geometry,
      availableSelections: [{ frequency: "A", polarizations: ["HH"] }],
    })),
    excludedItems: [],
    commonSelections: [{ frequency: "A", polarizations: ["HH"] }],
  };
  const currentGeometry = {
    type: "Polygon" as const,
    coordinates: [[[120, 20], [121, 20], [121, 21], [120, 20]]],
  };
  const currentResult: NisarSubsetWorkspacePreflight = {
    sourceBatchId: "batch-1",
    workbenchRevision: 7,
    eligibleItems: ["scene-1", "scene-2"].map((sourceItemId) => ({
      sourceItemId,
      effectiveGeometry: currentGeometry,
      availableSelections: [{ frequency: "B" as const, polarizations: ["HH"] }],
    })),
    excludedItems: [],
    commonSelections: [{ frequency: "B", polarizations: ["HH"] }],
  };
  const view = renderWorkspace(initialDraft, {
    onPreflight: (draft) => {
      requestedDrafts.push(structuredClone(draft));
      return requestedDrafts.length === 1 ? stale.promise : current.promise;
    },
  });

  try {
    fireEvent.click(view.getByRole("button", { name: "检查相交与能力" }));
    assert.equal(requestedDrafts.length, 1);
    assert.deepEqual(requestedDrafts[0]?.selectedItemIds, ["scene-1"]);

    fireEvent.click(view.getByRole("checkbox", { name: /scene-2 选择影像/ }));
    assert.equal(view.getByText("已选 2 景").textContent, "已选 2 景");

    await act(async () => {
      stale.resolve(staleResult);
      await stale.promise;
    });

    assert.equal(view.queryByText("已确认相交"), null);
    assert.equal(view.queryByText("实际相交范围"), null);
    assert.equal(view.queryByRole("radio", { name: "Frequency A" }), null);

    fireEvent.click(view.getByRole("button", { name: "检查相交与能力" }));
    assert.equal(requestedDrafts.length, 2);
    assert.deepEqual(requestedDrafts[1]?.selectedItemIds, ["scene-1", "scene-2"]);

    await act(async () => {
      current.resolve(currentResult);
      await current.promise;
    });

    assert.equal(view.getAllByText("已确认相交").length, 2);
    assert.ok(view.getByText("实际相交范围"));
    assert.equal(view.queryByRole("radio", { name: "Frequency A" }), null);
    assert.ok(view.getByRole("radio", { name: "Frequency B" }));
  } finally {
    await act(async () => {
      stale.resolve(staleResult);
      current.resolve(currentResult);
      await Promise.all([stale.promise, current.promise]);
    });
    cleanup();
  }
});

test("workspace resolves administrative and imported geometry without binding the main AOI", async () => {
  const initialDraft = createNisarSubsetWorkspaceDraft("batch-1", rows, new Set(["scene-1"]), null, null);
  const boundary: AdminBoundary = {
    label: "武汉市",
    display_name: "武汉市",
    bbox,
    provider: "nominatim",
    provider_object_id: "relation:1271231",
  };
  let adminSearches = 0;
  let imports = 0;
  const view = renderWorkspace(initialDraft, {
    onSearchAdministrativeAoi: async () => {
      adminSearches += 1;
      return { ok: true, results: [boundary] };
    },
    onResolveAdministrativeAoi: async () => ({ ok: true, geometry }),
    onImportBoundary: async () => {
      imports += 1;
      return {
        ok: true,
        preview: {
          ok: true,
          path: "single.geojson",
          file_name: "single.geojson",
          total_features: 1,
          fields: [],
          features: [{ id: "0", index: 1, source_index: 0, name: "single", properties: {} }],
          geojson: { type: "FeatureCollection", features: [{ type: "Feature", properties: {}, geometry }] },
        },
      };
    },
  });

  fireEvent.click(view.getByRole("button", { name: "选择行政区" }));
  fireEvent.change(view.getByRole("textbox", { name: "行政区关键词" }), { target: { value: "武汉" } });
  fireEvent.click(view.getByRole("button", { name: "搜索行政区" }));
  await view.findByRole("button", { name: "使用行政区：武汉市" });
  assert.equal(adminSearches, 1);
  fireEvent.click(view.getByRole("button", { name: "使用行政区：武汉市" }));
  assert.ok(await view.findByText("已选择 AOI"));

  fireEvent.click(view.getByRole("button", { name: "导入边界" }));
  await view.findByRole("dialog", { name: "选择裁剪边界要素" });
  fireEvent.click(view.getByRole("checkbox", { name: /single/ }));
  fireEvent.click(view.getByRole("button", { name: "应用所选边界" }));
  await view.findByText("已选择 AOI");
  assert.equal(imports, 1);
  cleanup();
});

test("workspace keeps eligible and no-overlap scenes separate within one batch", async () => {
  const batchRows = [row("scene-1", ["HH"]), row("scene-2", ["HH"])];
  const draft = createNisarSubsetWorkspaceDraft(
    "batch-1", batchRows, new Set(["scene-1", "scene-2"]), null, geometry,
  );
  let requestedIds: string[] = [];
  let reviewed: NisarSubsetWorkspaceDraft | null = null;
  const view = renderWorkspace(draft, {
    rows: batchRows,
    onPreflight: async (requested) => {
      requestedIds = [...requested.selectedItemIds];
      return {
        sourceBatchId: "batch-1",
        workbenchRevision: 7,
        eligibleItems: [{
          sourceItemId: "scene-1",
          effectiveGeometry: geometry,
          availableSelections: [{ frequency: "A" as const, polarizations: ["HH"] }],
        }],
        excludedItems: [{ sourceItemId: "scene-2", reason: "no_overlap" as const }],
        commonSelections: [{ frequency: "A" as const, polarizations: ["HH"] }],
      };
    },
    onReview: (value) => { reviewed = value; },
  });

  fireEvent.click(view.getByRole("button", { name: "检查相交与能力" }));
  await view.findByText("与 AOI 无交集，将排除");
  assert.deepEqual(requestedIds, ["scene-1", "scene-2"]);
  assert.ok(view.getByText("可执行 1"));
  assert.ok(view.getByText("排除 1"));
  fireEvent.click(view.getByRole("radio", { name: "Frequency A" }));
  fireEvent.click(view.getByRole("checkbox", { name: "Polarization HH" }));
  fireEvent.click(view.getByRole("button", { name: "检查并下载" }));
  assert.deepEqual(reviewed?.selectedItemIds, ["scene-1", "scene-2"]);
  cleanup();
});

test("workspace map distinguishes footprints, requested AOI, and prepared intersections", () => {
  const draft = createNisarSubsetWorkspaceDraft(
    "batch-1", rows, new Set(["scene-1"]), null, geometry,
  );
  const view = renderWorkspace(draft, { effectiveGeometries: [geometry] });

  const legend = view.getByRole("list", { name: "裁剪地图图层" });
  assert.match(legend.textContent ?? "", /影像覆盖/);
  assert.match(legend.textContent ?? "", /请求范围/);
  assert.match(legend.textContent ?? "", /实际相交范围/);
  cleanup();
});


test("multi-feature import applies selected exact polygons only to the local workspace draft", async () => {
  const initialDraft = createNisarSubsetWorkspaceDraft(
    "batch-1", rows, new Set(["scene-1"]), null, geometry,
  );
  const polygonA = {
    type: "Polygon",
    coordinates: [[[120, 20], [121, 20], [121, 21], [120, 20]]],
  };
  const polygonB = {
    type: "MultiPolygon",
    coordinates: [[[[122, 20], [123, 20], [123, 21], [122, 20]]]],
  };
  const preview: AoiPreviewOk = {
    ok: true,
    path: "multi.geojson",
    file_name: "multi.geojson",
    total_features: 3,
    fields: ["name"],
    features: [
      { id: "a", index: 1, source_index: 0, name: "边界 A", properties: { name: "边界 A" } },
      { id: "b", index: 2, source_index: 1, name: "边界 B", properties: { name: "边界 B" } },
      { id: "line", index: 3, source_index: 2, name: "线", properties: { name: "线" } },
    ],
    geojson: {
      type: "FeatureCollection",
      features: [
        { type: "Feature", id: "a", properties: {}, geometry: polygonA },
        { type: "Feature", id: "b", properties: {}, geometry: polygonB },
        { type: "Feature", id: "line", properties: {}, geometry: { type: "LineString", coordinates: [[0, 0], [1, 1]] } },
      ],
    },
  };
  const mainAoi = structuredClone(geometry);
  const mainSelection = ["scene-1"];
  let reviewed: NisarSubsetWorkspaceDraft | null = null;
  const view = renderWorkspace(initialDraft, {
    onImportBoundary: async () => ({ ok: true, preview }),
    onPreflight: async (draft) => ({
      sourceBatchId: "batch-1",
      workbenchRevision: 7,
      eligibleItems: [{
        sourceItemId: "scene-1",
        effectiveGeometry: draft.requestedGeometry!,
        availableSelections: [{ frequency: "A", polarizations: ["HH", "HV"] }],
      }],
      excludedItems: [],
      commonSelections: [{ frequency: "A", polarizations: ["HH", "HV"] }],
    }),
    onReview: (draft) => { reviewed = draft; },
  });

  fireEvent.click(view.getByRole("button", { name: "导入边界" }));
  await view.findByRole("dialog", { name: "选择裁剪边界要素" });
  fireEvent.click(view.getByRole("checkbox", { name: /边界 A/ }));
  fireEvent.click(view.getByRole("checkbox", { name: /边界 B/ }));
  fireEvent.click(view.getByRole("button", { name: "应用所选边界" }));
  fireEvent.click(view.getByRole("button", { name: "检查相交与能力" }));
  await view.findByRole("radio", { name: "Frequency A" });
  fireEvent.click(view.getByRole("radio", { name: "Frequency A" }));
  fireEvent.click(view.getByRole("checkbox", { name: "Polarization HH" }));
  fireEvent.click(view.getByRole("button", { name: "检查并下载" }));

  assert.ok(reviewed);
  assert.deepEqual(reviewed.requestedGeometry, {
    type: "MultiPolygon",
    coordinates: [polygonA.coordinates, ...polygonB.coordinates],
  });
  assert.deepEqual(mainAoi, geometry);
  assert.deepEqual(mainSelection, ["scene-1"]);
  assert.deepEqual(initialDraft.requestedGeometry, geometry);
  cleanup();
});

function loadedScene(itemId: string, provider = "asf.nasa.earthdata"): SceneRow {
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
    sar_product: product(itemId),
  };
}

function frozenPage(
  pageIndex: number,
  itemIds: string[],
  checkedItemIds: string[] = [],
  revision = 7,
): BoundedWorkbenchPageResult {
  return {
    ok: true,
    page: {
      batch_id: "batch-nisar",
      revision,
      page_index: pageIndex,
      page_size: 2,
      total_count: 3,
      checked_count: 1,
      render: null,
      items: itemIds.map((itemId, offset) => ({
        ordinal: pageIndex * 2 + offset,
        checked: checkedItemIds.includes(itemId),
        item: {
          item_id: itemId,
          provider_id: "asf.nasa.earthdata",
          display_name: itemId,
          acquisition_datetime: "2026-08-01T00:00:00Z",
          footprint_geojson: null,
          metadata: {},
          payload: {},
          asset_access: null,
          execution_policy: null,
        },
      })),
    },
    scenes: itemIds.map((itemId) => loadedScene(itemId)),
    checked_item_ids: checkedItemIds,
  };
}

const frozenBatchContext: DownloadBatchContext = {
  batch_id: "batch-nisar",
  created_at: "2026-08-01T00:00:00Z",
  aoi_name: "AOI",
  aoi_geojson: null,
  query: { provider: "asf.nasa.earthdata", dataset: "sar.nisar.l.rslc.provisional.v1" },
  page_index: 0,
  page_size: 2,
  revision: 7,
  total_count: 3,
};

test("production frozen-batch loader exposes later pages in exact Store order and binding", async () => {
  const requests: Array<{ batch_id: string; expected_revision: number; page_index: number; page_size: number }> = [];
  const laterPage = frozenPage(1, ["scene-3"], ["scene-3"]);
  const result = await loadCompleteNisarSubsetBatch({
    baseContext: frozenBatchContext,
    aoiAuthority: null,
    pageSize: 2,
    loadPage: async (request) => {
      requests.push(request);
      return request.page_index === 0 ? frozenPage(0, ["scene-1", "scene-2"]) : laterPage;
    },
  });

  assert.equal(result.ok, true);
  if (!result.ok) return;
  assert.deepEqual(requests, [
    { batch_id: "batch-nisar", expected_revision: 7, page_index: 0, page_size: 2 },
    { batch_id: "batch-nisar", expected_revision: 7, page_index: 1, page_size: 2 },
  ]);
  assert.deepEqual(result.items.map((item) => item.itemId), ["scene-1", "scene-2", "scene-3"]);
  assert.deepEqual(result.items.map((item) => item.providerId), [
    "asf.nasa.earthdata",
    "asf.nasa.earthdata",
    "asf.nasa.earthdata",
  ]);
  assert.deepEqual(result.checkedItemIds, ["scene-3"]);
  assert.notEqual(result.items[2]?.scene, laterPage.scenes[0]);
});

test("production frozen-batch loader fails closed on a later-page revision mismatch", async () => {
  const result = await loadCompleteNisarSubsetBatch({
    baseContext: frozenBatchContext,
    aoiAuthority: null,
    pageSize: 2,
    loadPage: async (request) => request.page_index === 0
      ? frozenPage(0, ["scene-1", "scene-2"])
      : frozenPage(1, ["scene-3"], ["scene-3"], 8),
  });

  assert.equal(result.ok, false);
  if (result.ok) return;
  assert.match(result.error, /批次、revision|版本/);
  assert.equal("items" in result, false);
});


test("automatic preflight follows AOI changes, ignores a late response, and preserves chosen science", async () => {
  const stale = deferred<NisarSubsetWorkspacePreflight>();
  const requests: NisarSubsetWorkspaceDraft[] = [];
  const initial = createNisarSubsetWorkspaceDraft("batch-1", rows, new Set(["scene-1"]), null, geometry);
  const view = renderWorkspace(initial, {
    effectiveGeometries: [geometry],
    onPreflight: (draft) => {
      requests.push(structuredClone(draft));
      if (requests.length === 1) return stale.promise;
      return Promise.resolve({
        sourceBatchId: "batch-1", workbenchRevision: 7,
        eligibleItems: [{ sourceItemId: "scene-1", effectiveGeometry: geometry,
          availableSelections: [{ frequency: "B", polarizations: ["HH", "HV"] }] }],
        excludedItems: [], commonSelections: [{ frequency: "B", polarizations: ["HH", "HV"] }],
      });
    },
  });
  try {
    await act(async () => { await new Promise((resolve) => setTimeout(resolve, 240)); });
    assert.equal(requests.length, 1);
    fireEvent.click(view.getByRole("button", { name: "清空范围" }));
    assert.equal(view.queryByText("实际相交范围"), null);
    fireEvent.click(view.getByRole("button", { name: "使用当前 AOI" }));
    await view.findByRole("radio", { name: "Frequency B" });
    assert.equal(requests.length, 2);
    await act(async () => {
      stale.resolve({ sourceBatchId: "batch-1", workbenchRevision: 7,
        eligibleItems: [{ sourceItemId: "scene-1", effectiveGeometry: geometry,
          availableSelections: [{ frequency: "A", polarizations: ["HH"] }] }],
        excludedItems: [], commonSelections: [{ frequency: "A", polarizations: ["HH"] }] });
      await stale.promise;
    });
    assert.equal(view.queryByRole("radio", { name: "Frequency A" }), null);
    fireEvent.click(view.getByRole("radio", { name: "Frequency B" }));
    fireEvent.click(view.getByRole("checkbox", { name: "Polarization HH" }));
    fireEvent.click(view.getByRole("checkbox", { name: "Polarization HV" }));
    await act(async () => { await new Promise((resolve) => setTimeout(resolve, 240)); });
    assert.equal(requests.length, 2);
    assert.equal((view.getByRole("button", { name: "检查并下载" }) as HTMLButtonElement).disabled, false);
  } finally { cleanup(); }
});

test("candidate status filters combine with product search independently of administrative search", async () => {
  const initial = createNisarSubsetWorkspaceDraft("batch-1", rows, new Set(["scene-1", "scene-2"]), null, geometry);
  const view = renderWorkspace(initial, {
    onPreflight: async () => ({ sourceBatchId: "batch-1", workbenchRevision: 7,
      eligibleItems: [{ sourceItemId: "scene-1", effectiveGeometry: geometry,
        availableSelections: [{ frequency: "A", polarizations: ["HH"] }] }],
      excludedItems: [{ sourceItemId: "scene-2", reason: "no_overlap" }],
      commonSelections: [{ frequency: "A", polarizations: ["HH"] }],
    }),
  });
  try {
    await view.findByText("与 AOI 无交集，将排除");
    fireEvent.click(view.getByRole("button", { name: "可执行", exact: true }));
    assert.ok(view.getByRole("checkbox", { name: /scene-1 选择影像/ }));
    assert.equal(view.queryByRole("checkbox", { name: /scene-2 选择影像/ }), null);
    fireEvent.click(view.getByRole("button", { name: "已排除", exact: true }));
    assert.ok(view.getByRole("checkbox", { name: /scene-2 选择影像/ }));
    assert.equal(view.queryByRole("checkbox", { name: /scene-1 选择影像/ }), null);
    fireEvent.click(view.getByRole("button", { name: "已选择", exact: true }));
    assert.equal(view.getAllByRole("checkbox", { name: /选择影像/ }).length, 2);
    fireEvent.change(view.getByRole("textbox", { name: "搜索影像" }), { target: { value: "scene-1" } });
    fireEvent.click(view.getByRole("button", { name: "选择行政区" }));
    fireEvent.change(view.getByRole("textbox", { name: "行政区关键词" }), { target: { value: "武汉" } });
    assert.equal(view.getAllByRole("checkbox", { name: /选择影像/ }).length, 1);
    assert.equal((view.getByRole("textbox", { name: "搜索影像" }) as HTMLInputElement).value, "scene-1");
    assert.ok(view.getByText("asf.nasa.earthdata"));
    assert.ok(view.getByText("A: HH"));
  } finally { cleanup(); }
});

test("S-band source notice follows frozen rows and permits reviewed crop with the known range difference", async () => {
  const sourceRows = [row("s-scene", ["RH"])];
  sourceRows[0].product = {
    ...sourceRows[0].product,
    provider: BHOONIDHI_PROVIDER_ID,
    dataset: BHOONIDHI_LOGICAL_DATASET,
    instrument: "SSAR",
    native_metadata: {
      logical_dataset: BHOONIDHI_LOGICAL_DATASET,
      native_collection_id: BHOONIDHI_COLLECTION_ID,
      collection_profile_id: BHOONIDHI_PROFILE_ID,
    },
  };
  const draft = createNisarSubsetWorkspaceDraft("s-batch", sourceRows, new Set(["s-scene"]), null, geometry);
  let reviews = 0;
  const view = renderWorkspace(draft, {
    rows: sourceRows,
    onPreflight: async () => ({
      sourceBatchId: "s-batch", workbenchRevision: 7,
      eligibleItems: [{ sourceItemId: "s-scene", effectiveGeometry: geometry,
        availableSelections: [{ frequency: "A", polarizations: ["RH"] }] }],
      excludedItems: [], commonSelections: [{ frequency: "A", polarizations: ["RH"] }],
    }),
    onReview: () => { reviews += 1; },
  });
  const notice = view.getByRole("note", { name: "S-Band 已知差异" });
  assert.match(notice.textContent ?? "", /S-band · Bhoonidhi/);
  assert.match(notice.textContent ?? "", /约 9\.6 cm/);
  const preflight = view.getByRole("button", { name: "检查相交与能力" }) as HTMLButtonElement;
  assert.equal(preflight.disabled, false);
  await act(async () => {
    fireEvent.click(preflight);
    await new Promise<void>((resolve) => setTimeout(resolve, 0));
  });
  fireEvent.click(view.getByRole("radio", { name: "Frequency A" }));
  fireEvent.click(view.getByRole("checkbox", { name: "Polarization RH" }));
  const review = view.getByRole("button", { name: "检查并下载" }) as HTMLButtonElement;
  assert.equal(review.disabled, false);
  fireEvent.click(review);
  assert.equal(reviews, 1);
  sourceRows[0].product = product("l-scene");
  fireEvent.click(view.getByRole("button", { name: "选择行政区" }));
  assert.ok(view.getByRole("note", { name: "S-Band 已知差异" }));
  cleanup();
});

test("L-band Frequency B never shows the S-band source notice", () => {
  const sourceRows = [row("l-frequency-b")];
  sourceRows[0].availableSelections = [{ frequency: "B", polarizations: ["HH"] }];
  const draft = createNisarSubsetWorkspaceDraft("l-batch", sourceRows, new Set(["l-frequency-b"]), null, geometry);
  const view = renderWorkspace(draft, { rows: sourceRows });
  assert.equal(view.queryByRole("note", { name: "S-Band 已知差异" }), null);
  cleanup();
});

test("clicking preview overlays the scene inside the map footprint frame without hiding WorkbenchMap", () => {
  const draft = createNisarSubsetWorkspaceDraft("batch-1", rows, new Set(["scene-1"]), null, geometry);
  let previewedItemId: string | null = null;
  const view = renderWorkspace(draft, {
    onPreviewItem: (itemId) => {
      previewedItemId = itemId;
    },
  });

  assert.ok(view.container.querySelector('[data-testid="map-toolbar"]'));
  assert.ok(view.getByRole("button", { name: "切换底图" }));
  assert.ok(view.container.querySelector('[data-testid="map-coordinate-overlay"]'));

  fireEvent.click(view.getByRole("button", { name: "预览影像 scene-1" }));
  assert.equal(previewedItemId, "scene-1");

  const legend = view.getByRole("list", { name: "裁剪地图图层" });
  assert.match(legend.textContent ?? "", /当前预览影像框/);
  assert.ok(view.getByText(/框内预览：scene-1/));
  assert.ok(view.container.querySelector('[data-testid="map-toolbar"]'));

  fireEvent.click(view.getByRole("button", { name: "关闭产品预览" }));
  assert.equal(view.queryByText(/框内预览：scene-1/), null);
  cleanup();
});

test("B7 studio workspace uses monochrome chrome without backdrop blur and locks Escape/close while reviewBusy", () => {
  const draft = createNisarSubsetWorkspaceDraft("batch-1", rows, new Set(["scene-1"]), null, geometry);
  let closeCalls = 0;
  const view = renderWorkspace(draft, {
    reviewBusy: true,
    onClose: () => {
      closeCalls += 1;
    },
  });

  const dialog = view.getByRole("dialog", { name: "NISAR RSLC AOI 裁剪下载" });
  assert.equal(dialog.parentElement?.className.includes("backdrop-blur"), false);
  assert.equal(dialog.querySelector(".bg-sp-accent-subtle"), null);
  assert.equal(dialog.querySelector(".text-blue-500"), null);
  assert.equal(dialog.querySelector(".text-blue-600"), null);

  const backBtn = view.getByRole("button", { name: "返回" }) as HTMLButtonElement;
  const closeBtn = view.getByRole("button", { name: "关闭裁剪工作区" }) as HTMLButtonElement;
  const cancelBtn = view.getByRole("button", { name: "取消" }) as HTMLButtonElement;
  assert.equal(backBtn.disabled, true);
  assert.equal(closeBtn.disabled, true);
  assert.equal(cancelBtn.disabled, true);

  fireEvent.keyDown(window, { key: "Escape" });
  fireEvent.click(backBtn);
  fireEvent.click(closeBtn);
  fireEvent.click(cancelBtn);
  assert.equal(closeCalls, 0);
  cleanup();
});

test("P2-1 subset workspace suppresses duplicate dock legend and disables draw toolbar buttons when AOI is task-frozen", () => {
  const draft = createNisarSubsetWorkspaceDraft("batch-1", rows, new Set(["scene-1"]), null, geometry);
  const view = renderWorkspace(draft, {
    taskContext: {
      taskId: "task-frozen-1",
      outputRoot: "/tmp/nisar-out",
      successor: false,
    },
  });

  assert.ok(view.getByRole("list", { name: "裁剪地图图层" }));
  assert.equal(view.container.querySelector('[data-testid="map-dock-legend"]'), null);

  const polygonBtn = view.container.querySelector('[data-testid="map-tool-btn-draw-polygon"]') as HTMLButtonElement;
  const rectBtn = view.container.querySelector('[data-testid="map-tool-btn-draw-rect"]') as HTMLButtonElement;
  const clearBtn = view.container.querySelector('[data-testid="map-tool-btn-clear"]') as HTMLButtonElement;
  const layersBtn = view.container.querySelector('[data-testid="map-tool-btn-layers"]') as HTMLButtonElement;

  assert.equal(polygonBtn.disabled, true);
  assert.equal(polygonBtn.getAttribute("aria-disabled"), "true");
  assert.equal(rectBtn.disabled, true);
  assert.equal(rectBtn.getAttribute("aria-disabled"), "true");
  assert.equal(clearBtn.disabled, true);
  assert.equal(clearBtn.getAttribute("aria-disabled"), "true");
  assert.equal(layersBtn.disabled, false);

  fireEvent.click(clearBtn);
  assert.ok(view.getByText("已选择 AOI"));
  cleanup();
});

test("Item 4b subset workspace renders live GB estimate (>=12px) instead of deferred placeholder", () => {
  const draft = createNisarSubsetWorkspaceDraft("batch-1", rows, new Set(["scene-1"]), null, geometry);
  const view = renderWorkspace(draft);
  const summaryEl = view.getByTestId("nisar-workspace-estimate-summary");
  assert.ok(summaryEl.textContent?.includes("预计裁剪后约"), "must display live GB estimate summary");
  assert.ok(summaryEl.textContent?.includes("GB"), "must include GB units");
  assert.equal(summaryEl.textContent?.includes("将在读取产品结构后确定"), false);
  const wrapperClass = summaryEl.parentElement?.className || "";
  assert.equal(wrapperClass.includes("text-[11px]"), false);
  assert.ok(wrapperClass.includes("text-xs"), "estimate block must use >=12px text-xs");
  cleanup();
});
