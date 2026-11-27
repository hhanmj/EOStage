import assert from "node:assert/strict";
import test from "node:test";

import type { SarProduct } from "./generated/v3-contracts.ts";
import {
  availableCommonNisarSelections,
  combineNisarSubsetBoundaryFeatures,
  createNisarSubsetWorkspaceDraft,
  nisarSubsetWorkspaceReadiness,
  setNisarSubsetWorkspaceGeometry,
  toggleNisarSubsetWorkspaceItem,
  type NisarSubsetWorkspaceRow,
} from "./nisarSubsetWorkspace.ts";

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
    acquisition_start: null,
    acquisition_stop: null,
    footprint_geojson: null,
    footprint_bbox: null,
    download_url: null,
    file_name: `${itemId}.h5`,
    file_size_remote: 1_000,
    checksum: null,
    local_path: null,
    native_metadata: {},
    burst: null,
  };
}

const rows: NisarSubsetWorkspaceRow[] = [
  {
    itemId: "item-1",
    product: product("item-1"),
    subsetEligible: true,
    blockedReason: null,
    overlapState: "unchecked",
    availableSelections: [{ frequency: "A", polarizations: ["HH", "HV"] }],
  },
  {
    itemId: "item-2",
    product: product("item-2"),
    subsetEligible: true,
    blockedReason: null,
    overlapState: "unchecked",
    availableSelections: [{ frequency: "A", polarizations: ["HH"] }],
  },
];

test("workspace seeds checked NISAR rows without aliasing main selection or AOI", () => {
  const selected = new Set(["item-1"]);
  const geometry: Record<string, unknown> = {
    type: "Polygon",
    coordinates: [[[110, 30], [111, 30], [111, 31], [110, 30]]],
  };

  const draft = createNisarSubsetWorkspaceDraft(
    "batch-1",
    rows,
    selected,
    "item-2",
    geometry,
  );

  selected.clear();
  ((geometry.coordinates as number[][][])[0][0])[0] = 0;

  assert.deepEqual(draft.selectedItemIds, ["item-1"]);
  assert.equal(
    ((draft.requestedGeometry?.coordinates as number[][][])[0][0])[0],
    110,
  );
});

test("workspace starts empty when neither checked nor highlighted rows are eligible", () => {
  const ineligibleRows = rows.map((row) => ({
    ...row,
    subsetEligible: false,
    blockedReason: "当前影像不可裁剪",
  }));

  const draft = createNisarSubsetWorkspaceDraft(
    "batch-1",
    ineligibleRows,
    new Set(),
    null,
    null,
  );

  assert.deepEqual(draft.selectedItemIds, []);
  assert.equal(draft.requestedGeometry, null);
});


test("workspace draft preserves all checked scenes and toggles ordinary multi-select", () => {
  const initial = createNisarSubsetWorkspaceDraft(
    "batch-1", rows, new Set(["item-1", "item-2"]), null, null,
  );
  assert.deepEqual(initial.selectedItemIds, ["item-1", "item-2"]);
  const withoutSecond = toggleNisarSubsetWorkspaceItem(initial, rows, "item-2");
  assert.deepEqual(withoutSecond.selectedItemIds, ["item-1"]);
  const restored = toggleNisarSubsetWorkspaceItem(withoutSecond, rows, "item-2");
  assert.deepEqual(restored.selectedItemIds, ["item-1", "item-2"]);
});

test("workspace item and geometry edits return isolated drafts", () => {
  const initial = createNisarSubsetWorkspaceDraft(
    "batch-1", rows, new Set(["item-1"]), null, null,
  );
  const withSecondItem = toggleNisarSubsetWorkspaceItem(initial, rows, "item-2");
  const geometry: Record<string, unknown> = {
    type: "Polygon",
    coordinates: [[[110, 30], [111, 30], [111, 31], [110, 30]]],
  };
  const withGeometry = setNisarSubsetWorkspaceGeometry(withSecondItem, geometry);
  ((geometry.coordinates as number[][][])[0][0])[0] = 0;

  assert.deepEqual(initial.selectedItemIds, ["item-1"]);
  assert.deepEqual(withSecondItem.selectedItemIds, ["item-1", "item-2"]);
  assert.equal(((withGeometry.requestedGeometry?.coordinates as number[][][])[0][0])[0], 110);

  const withoutSecondItem = toggleNisarSubsetWorkspaceItem(withSecondItem, rows, "item-2");
  assert.deepEqual(withoutSecondItem.selectedItemIds, ["item-1"]);
  const blockedRows = rows.map((row) => row.itemId === "item-2"
    ? { ...row, subsetEligible: false, blockedReason: "当前影像不可裁剪" }
    : row);
  assert.deepEqual(
    toggleNisarSubsetWorkspaceItem(initial, blockedRows, "item-2").selectedItemIds,
    ["item-1"],
  );
});

test("multi-scene choices are the exact common frequency and polarizations", () => {
  const multiFrequencyRows: NisarSubsetWorkspaceRow[] = [
    {
      ...rows[0],
      availableSelections: [
        { frequency: "A", polarizations: ["HH", "HV"] },
        { frequency: "B", polarizations: ["HH"] },
      ],
    },
    {
      ...rows[1],
      availableSelections: [
        { frequency: "A", polarizations: ["HH"] },
        { frequency: "B", polarizations: ["HV"] },
      ],
    },
  ];

  assert.deepEqual(availableCommonNisarSelections(multiFrequencyRows), [
    { frequency: "A", polarizations: ["HH"] },
  ]);
});

test("readiness excludes a no-overlap source scene and blocks an empty executable set", () => {
  const noOverlapRows = rows.map((row) => ({ ...row, overlapState: "no_overlap" as const }));
  const draft = {
    ...createNisarSubsetWorkspaceDraft(
      "batch-1", rows, new Set(["item-1"]), null,
      { type: "Polygon", coordinates: [[[110, 30], [111, 30], [111, 31], [110, 30]]] },
    ),
    selectedFrequency: "A" as const,
    selectedPolarizations: ["HH"],
  };

  const result = nisarSubsetWorkspaceReadiness(draft, noOverlapRows);

  assert.equal(result.ready, false);
  assert.equal(result.code, "no_overlap");
  assert.equal(result.reason, "当前 AOI 与所选影像均不相交");
  assert.deepEqual(result.executableItemIds, []);
  assert.deepEqual(result.excludedItemIds, ["item-1"]);
});

test("readiness accepts a preflighted multi-scene draft with a common selection", () => {
  const conflictingRows: NisarSubsetWorkspaceRow[] = [
    { ...rows[0], overlapState: "intersects" },
    { ...rows[1], overlapState: "intersects" },
  ];
  const draft = {
    ...createNisarSubsetWorkspaceDraft(
      "batch-1", conflictingRows, new Set(["item-1"]), null,
      { type: "Polygon", coordinates: [[[110, 30], [111, 30], [111, 31], [110, 30]]] },
    ),
    selectedItemIds: ["item-1", "item-2"],
    selectedFrequency: "A" as const,
    selectedPolarizations: ["HH"],
  };

  const result = nisarSubsetWorkspaceReadiness(draft, conflictingRows);

  assert.equal(result.ready, true);
  assert.equal(result.code, "ready");
  assert.equal(result.reason, null);
  assert.deepEqual(result.executableItemIds, ["item-1", "item-2"]);
  assert.deepEqual(result.conflictingItemIds, []);
});

test("readiness never treats unchecked overlap as executable", () => {
  const draft = {
    ...createNisarSubsetWorkspaceDraft(
      "batch-1", rows, new Set(["item-1", "item-2"]), null,
      { type: "Polygon", coordinates: [[[110, 30], [111, 30], [111, 31], [110, 30]]] },
    ),
    selectedFrequency: "A" as const,
    selectedPolarizations: ["HH"],
  };

  const result = nisarSubsetWorkspaceReadiness(draft, rows);

  assert.equal(result.ready, false);
  assert.equal(result.code, "preflight_required");
  assert.deepEqual(result.executableItemIds, []);
  assert.equal(result.overlapVerified, false);
});


test("selected Polygon and MultiPolygon preview features combine without bbox fabrication", () => {
  const polygon = {
    type: "Polygon",
    coordinates: [[[110, 30], [111, 30], [111, 31], [110, 30]]],
  };
  const multiPolygon = {
    type: "MultiPolygon",
    coordinates: [[[[112, 30], [113, 30], [113, 31], [112, 30]]]],
  };
  const preview = {
    ok: true as const,
    path: "boundary.geojson",
    file_name: "boundary.geojson",
    total_features: 3,
    fields: ["name"],
    features: [
      { id: "polygon-a", index: 1, source_index: 0, name: "A", properties: { name: "A" } },
      { id: "multi-b", index: 2, source_index: 1, name: "B", properties: { name: "B" } },
      { id: "line-c", index: 3, source_index: 2, name: "C", properties: { name: "C" } },
    ],
    geojson: {
      type: "FeatureCollection",
      features: [
        { type: "Feature", id: "polygon-a", properties: { name: "A" }, geometry: polygon },
        { type: "Feature", id: "multi-b", properties: { name: "B" }, geometry: multiPolygon },
        { type: "Feature", id: "line-c", properties: { name: "C" }, geometry: { type: "LineString", coordinates: [[0, 0], [1, 1]] } },
      ],
    },
  };

  const result = combineNisarSubsetBoundaryFeatures(preview, new Set(["polygon-a", "multi-b"]));

  assert.equal(result.ok, true);
  if (!result.ok) return;
  assert.deepEqual(result.geometry, {
    type: "MultiPolygon",
    coordinates: [polygon.coordinates, ...multiPolygon.coordinates],
  });
  assert.notEqual(result.geometry, preview.geojson);
  assert.deepEqual(preview.geojson.features[0].geometry, polygon);
});

test("boundary feature combination fails closed for empty or unsupported selections", () => {
  const preview = {
    ok: true as const,
    path: "boundary.geojson",
    file_name: "boundary.geojson",
    total_features: 1,
    fields: [],
    features: [{ id: "line-only", index: 1, source_index: 0, name: "line", properties: {} }],
    geojson: {
      type: "FeatureCollection",
      features: [{
        type: "Feature",
        id: "line-only",
        properties: {},
        geometry: { type: "LineString", coordinates: [[0, 0], [1, 1]] },
      }],
    },
  };

  assert.deepEqual(combineNisarSubsetBoundaryFeatures(preview, new Set()), {
    ok: false,
    error: "请至少选择一个可用的多边形要素。",
  });
  assert.deepEqual(combineNisarSubsetBoundaryFeatures(preview, new Set(["line-only"])), {
    ok: false,
    error: "所选要素不包含可用的 Polygon 或 MultiPolygon 几何。",
  });
});
