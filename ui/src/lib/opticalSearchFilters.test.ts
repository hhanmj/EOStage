import assert from "node:assert/strict";
import test from "node:test";

import { buildLandsatC2L2CatalogQueryFromDraft } from "./landsatCatalog.ts";
import {
  MAX_OPTICAL_INTENT_RESULT_LIMIT,
  OPTICAL_SEARCH_CAPABILITIES,
  buildOpticalCatalogIntent,
  initialOpticalFilterDrafts,
  opticalCapabilityForLogicalDataset,
  searchOnlyOpticalReason,
  opticalSearchCapability,
  opticalSearchSchema,
} from "./opticalSearchFilters.ts";
import { conditionFilters, facetFilters } from "./searchFilterSchema.ts";
import { buildSentinel2L2aCatalogQueryFromDraft } from "./sentinel2Catalog.ts";

// Issue #3 R2 B3 (2026-10-19): the optical Workbench no longer keeps a facet
// table or per-facet state bindings. Facets, dates and cloud cover come from the
// reviewed schemas; these tests keep UI-REG-004's invariants on that source.

test("optical products are every schema placed on the optical panel, in schema order", () => {
  // Issue #3 R2 B5 (2026-10-19): the list comes from the schema catalog; HLS
  // L30/S30 joined by contract alone and are search-only per the Registry.
  assert.deepEqual(
    OPTICAL_SEARCH_CAPABILITIES.map((item) => [item.key, item.schemaId, item.logicalDataset, item.searchOnly]),
    [
      ["sentinel2_l2a", "search.optical.sentinel-2.l2a.v1", "optical.sentinel-2.l2a", false],
      ["sentinel2_l1c", "search.optical.sentinel-2.l1c.v1", "optical.sentinel-2.l1c", false],
      ["landsat_c2_l2", "search.optical.landsat.c2-l2.v1", "optical.landsat.c2.l2", false],
      ["sentinel3", "search.optical.sentinel-3.v1", "optical.sentinel-3.standard", false],
      ["hls_l30", "search.optical.hls.l30.v2", "optical.hls.l30.v2", false],
      ["hls_s30", "search.optical.hls.s30.v2", "optical.hls.s30.v2", false],
    ],
  );
  assert.equal(searchOnlyOpticalReason({ logical_dataset: "optical.hls.l30.v2" }), null);
  assert.equal(searchOnlyOpticalReason({ logical_dataset: "optical.landsat.c2.l2" }), null);
  assert.equal(searchOnlyOpticalReason({ logical_dataset: "sar.unknown" }), null);
  assert.equal(searchOnlyOpticalReason(null), null);
  assert.equal(opticalCapabilityForLogicalDataset("optical.hls.s30.v2")?.key, "hls_s30");
  for (const capability of OPTICAL_SEARCH_CAPABILITIES) {
    assert.ok(capability.datasetLabel);
    assert.ok(capability.description);
    const schema = opticalSearchSchema(capability.key);
    assert.equal(schema.dataset_label, capability.datasetLabel);
    // Dataset, platform, instrument, level and type stay separate capabilities.
    assert.deepEqual(
      facetFilters(schema).map((filter) => filter.filter_id),
      capability.key === "sentinel3"
        ? ["platform", "instrument", "product_type"]
        : ["platform", "instrument", "product_level", "product_type"],
    );
    assert.ok(facetFilters(schema).every((filter) => filter.options.length > 0));
    assert.deepEqual(
      conditionFilters(schema).map((filter) => [filter.filter_id, filter.control]),
      [["acquisition_time", "date_range"], ["cloud_cover", "number"]],
    );
  }
});

test("optical facet values remain closed to implemented routes", () => {
  const sentinel2 = opticalSearchSchema("sentinel2_l2a");
  const s2Facets = Object.fromEntries(facetFilters(sentinel2).map((filter) => [filter.filter_id, filter]));
  assert.equal(opticalSearchCapability("sentinel2_l2a").datasetLabel, "Sentinel-2");
  assert.equal(s2Facets.platform.control, "multi_enum");
  assert.deepEqual(s2Facets.platform.options.map((item) => item.value), ["sentinel_2a", "sentinel_2b", "sentinel_2c"]);
  assert.equal(s2Facets.instrument.control, "fixed");
  assert.equal(s2Facets.product_level.control, "fixed");
  assert.equal(s2Facets.product_type.control, "fixed");
  assert.deepEqual(s2Facets.product_type.options.map((item) => item.value), ["S2MSI2A"]);

  const landsat = opticalSearchSchema("landsat_c2_l2");
  const lsFacets = Object.fromEntries(facetFilters(landsat).map((filter) => [filter.filter_id, filter]));
  assert.equal(opticalSearchCapability("landsat_c2_l2").datasetLabel, "Landsat Collection 2");
  assert.equal(lsFacets.platform.control, "multi_enum");
  assert.deepEqual(lsFacets.platform.options.map((item) => item.value), ["landsat_4", "landsat_5", "landsat_7", "landsat_8", "landsat_9"]);
  assert.equal(lsFacets.product_type.control, "multi_enum");
  assert.deepEqual(lsFacets.product_type.options.map((item) => item.value), ["l2sp", "l2sr"]);
  assert.doesNotMatch(landsat.dataset_label, /8\/9|OLI|TIRS|L2SP|L2SR/);
});

test("each optical product starts with its own draft defaulting to all", () => {
  const drafts = initialOpticalFilterDrafts();
  assert.deepEqual(drafts.sentinel2_l2a, {
    platform: [],
    acquisition_time: { start: "", end: "" },
    cloud_cover: "",
  });
  assert.deepEqual(drafts.landsat_c2_l2, {
    platform: ["landsat_4", "landsat_5", "landsat_7", "landsat_8", "landsat_9"],
    product_type: ["l2sp", "l2sr"],
    acquisition_time: { start: "", end: "" },
    cloud_cover: "",
  });
  // Drafts are independent objects, so switching products cannot leak values.
  assert.notEqual(drafts.sentinel2_l2a, drafts.landsat_c2_l2);
  assert.notEqual(initialOpticalFilterDrafts().landsat_c2_l2.platform, drafts.landsat_c2_l2.platform);
});

const intentAoi = { type: "Polygon", coordinates: [[[110, 30], [111, 30], [111, 31], [110, 30]]] };

test("optical intents name the product and schema, never a provider", () => {
  const drafts = initialOpticalFilterDrafts();
  drafts.sentinel2_l2a = { ...drafts.sentinel2_l2a, platform: ["sentinel_2b"], cloud_cover: "25" };
  const s2 = buildOpticalCatalogIntent("sentinel2_l2a", { aoiGeojson: intentAoi, draft: drafts.sentinel2_l2a, resultLimit: 100 });
  assert.equal(s2.ok, true);
  if (!s2.ok) return;
  assert.deepEqual(Object.keys(s2.intent).sort(), ["aoi_geojson", "filters", "resource_id", "result_limit", "schema_revision", "search_schema_id"]);
  assert.equal(s2.intent.resource_id, "science.sentinel-2.l2a.v1");
  assert.equal(s2.intent.search_schema_id, "search.optical.sentinel-2.l2a.v1");
  assert.equal(s2.intent.schema_revision, 1);
  assert.deepEqual(s2.intent.aoi_geojson, intentAoi);
  // Predicates are byte-identical to the seed-query builder the intent replaces.
  const legacy = buildSentinel2L2aCatalogQueryFromDraft({ aoiGeojson: intentAoi, draft: drafts.sentinel2_l2a, resultLimit: 100 });
  assert.equal(legacy.ok, true);
  if (legacy.ok) assert.equal(JSON.stringify(s2.intent.filters), JSON.stringify(legacy.query.filters));

  const landsat = buildOpticalCatalogIntent("landsat_c2_l2", { aoiGeojson: intentAoi, draft: drafts.landsat_c2_l2, resultLimit: 100 });
  const landsatLegacy = buildLandsatC2L2CatalogQueryFromDraft({ aoiGeojson: intentAoi, draft: drafts.landsat_c2_l2, resultLimit: 100 });
  assert.equal(landsat.ok && landsatLegacy.ok, true);
  if (landsat.ok && landsatLegacy.ok) {
    assert.equal(landsat.intent.resource_id, "science.landsat.c2-l2.v1");
    assert.equal(JSON.stringify(landsat.intent.filters), JSON.stringify(landsatLegacy.query.filters));
  }
});

test("optical intents reject missing geometry, invalid drafts and unsafe limits", () => {
  const drafts = initialOpticalFilterDrafts();
  assert.deepEqual(
    buildOpticalCatalogIntent("landsat_c2_l2", { aoiGeojson: null, draft: drafts.landsat_c2_l2, resultLimit: 100 }),
    { ok: false, error: "请先提供真实 AOI 几何；Landsat Collection 2 检索不会用边界框代替。" },
  );
  assert.deepEqual(
    buildOpticalCatalogIntent("sentinel2_l2a", { aoiGeojson: intentAoi, draft: { ...drafts.sentinel2_l2a, cloud_cover: "101" }, resultLimit: 100 }),
    { ok: false, error: "最大云量必须是 0 到 100 之间的数字。" },
  );
  for (const resultLimit of [0, MAX_OPTICAL_INTENT_RESULT_LIMIT + 1, 1.5]) {
    assert.deepEqual(
      buildOpticalCatalogIntent("sentinel2_l2a", { aoiGeojson: intentAoi, draft: drafts.sentinel2_l2a, resultLimit }),
      { ok: false, error: "检索结果上限无效。" },
    );
  }
});
