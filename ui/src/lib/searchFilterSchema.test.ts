import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { buildLandsatC2L2CatalogQuery, MPC_LANDSAT_COLLECTION_ID, MPC_LANDSAT_PROVIDER_ID } from "./landsatCatalog.ts";
import { buildOptionalOpticalDateRange } from "./opticalCatalogSearch.ts";
import { buildLandsatC2L2CatalogQueryFromDraft } from "./landsatCatalog.ts";
import {
  SEARCH_FILTER_SCHEMA_CATALOG,
  buildSchemaCatalogFilters,
  conditionFilters,
  draftDateRange,
  draftSelection,
  draftText,
  facetFilters,
  initialSearchFilterDraft,
  multiEnumMenuValues,
  searchFilterSchema,
  updateMultiEnumSelection,
  type MultiEnumSearchFilter,
  type SearchFilterSchema,
} from "./searchFilterSchema.ts";
import {
  buildSentinel2L2aCatalogQueryFromDraft,
  buildSentinel2L2aCatalogQuery,
  SENTINEL2_L2A_COLLECTION_ID,
  SENTINEL2_L2A_PRODUCT_TYPE,
  SENTINEL2_L2A_PROVIDER_ID,
} from "./sentinel2Catalog.ts";

const S2 = "search.optical.sentinel-2.l2a.v1";
const LANDSAT = "search.optical.landsat.c2-l2.v1";
const aoi = { type: "Polygon", coordinates: [[[110, 30], [111, 30], [111, 31], [110, 30]]] };

// ---------------------------------------------------------------------------
// Legacy references: the filter logic of the hand-written optical builders
// before they delegated to the schema (copied from sentinel2Catalog.ts and
// landsatCatalog.ts at 234a3da1). Kept only to prove behavioural parity.
type LegacyResult = { ok: true; filters: Record<string, unknown> } | { ok: false; error: string };

function legacyCloud(value: string): number | null | "invalid" {
  const trimmed = value.trim();
  if (!trimmed) return null;
  if (!/^(?:0|[1-9]\d*)(?:\.\d+)?$/.test(trimmed)) return "invalid";
  const parsed = Number(trimmed);
  return Number.isFinite(parsed) && parsed >= 0 && parsed <= 100 ? parsed : "invalid";
}

function legacySentinel2Filters(start: string, end: string, cloudText: string, spacecraftInput?: readonly string[]): LegacyResult {
  const dateRange = buildOptionalOpticalDateRange(start, end);
  if (!dateRange.ok) return dateRange;
  const cloud = legacyCloud(cloudText);
  if (cloud === "invalid") return { ok: false, error: "最大云量必须是 0 到 100 之间的数字。" };
  const filters: Record<string, unknown> = { product_type: "S2MSI2A" };
  const spacecraft = [...new Set(spacecraftInput ?? [])].sort();
  if (spacecraft.some((value) => !["sentinel_2a", "sentinel_2b", "sentinel_2c"].includes(value))) {
    return { ok: false, error: "请选择有效的 Sentinel-2A、2B 或 2C 平台。" };
  }
  if (spacecraft.length > 0) filters.spacecraft = spacecraft;
  if (dateRange.datetime) filters.datetime = dateRange.datetime;
  if (cloud !== null) filters.cloud_cover_lte = cloud;
  return { ok: true, filters };
}

function legacyClosed(values: readonly string[], allowed: ReadonlySet<string>): string[] | null {
  const unique = [...new Set(values)];
  return unique.length > 0 && unique.every((value) => allowed.has(value)) ? unique.sort() : null;
}

function legacyLandsatFilters(start: string, end: string, cloudText: string, spacecraftInput: readonly string[], levelsInput: readonly string[]): LegacyResult {
  const dateRange = buildOptionalOpticalDateRange(start, end);
  if (!dateRange.ok) return dateRange;
  const cloud = legacyCloud(cloudText);
  if (cloud === "invalid") return { ok: false, error: "最大云量必须是 0 到 100 之间的数字。" };
  const spacecraft = legacyClosed(spacecraftInput, new Set(["landsat_4", "landsat_5", "landsat_7", "landsat_8", "landsat_9"]));
  const levels = legacyClosed(levelsInput, new Set(["l2sp", "l2sr"]));
  if (!spacecraft) return { ok: false, error: "请选择 Landsat 平台和处理级别的有效组合。" };
  if (!levels) return { ok: false, error: "请选择 Landsat 8/9 和 L2SP/L2SR 的有效组合。" };
  const filters: Record<string, unknown> = { spacecraft, processing_levels: levels };
  if (dateRange.datetime) filters.datetime = dateRange.datetime;
  if (cloud !== null) filters.cloud_cover_lte = cloud;
  return { ok: true, filters };
}
// ---------------------------------------------------------------------------

const DATES: Array<[string, string]> = [
  ["", ""],
  ["2026-07-01", "2026-07-02"],
  ["2026-07-01T00:00:00Z", "2026-07-03T12:00:00.5Z"],
  ["2026-07-01", ""],
  ["", "2026-07-02"],
  ["2026-07-02", "2026-07-01"],
  ["2026-02-30", "2026-03-01"],
];
const CLOUDS = ["", "0", "12.5", "100", " 30 ", "100.5", "-1", "1e2", "abc", "07", "0.0"];

test("schema-driven Sentinel-2 filters match the hand-written builder on every input", () => {
  const schema = searchFilterSchema(S2);
  const spacecraftCases: Array<string[] | undefined> = [
    undefined, [], ["sentinel_2b", "sentinel_2a", "sentinel_2a"], ["sentinel_2a", "sentinel_2b", "sentinel_2c"],
    ["sentinel_2x"], ["landsat_8"],
  ];
  let cases = 0;
  for (const [start, end] of DATES) for (const cloud of CLOUDS) for (const spacecraft of spacecraftCases) {
    const expected = legacySentinel2Filters(start, end, cloud, spacecraft);
    const actual = buildSchemaCatalogFilters(schema, {
      platform: spacecraft ?? [],
      acquisition_time: { start, end },
      cloud_cover: cloud,
    });
    // String comparison also pins key order, which serialized queries depend on.
    assert.equal(JSON.stringify(actual), JSON.stringify(expected), JSON.stringify({ start, end, cloud, spacecraft }));
    cases += 1;
  }
  assert.equal(cases, DATES.length * CLOUDS.length * spacecraftCases.length);
});

test("schema-driven Landsat filters match the hand-written builder on every input", () => {
  const schema = searchFilterSchema(LANDSAT);
  const spacecraftCases = [[], ["landsat_9"], ["landsat_9", "landsat_8", "landsat_8"], ["landsat_7"]];
  const levelCases = [[], ["l2sr"], ["l2sr", "l2sp"], ["l2xx"]];
  for (const [start, end] of DATES) for (const cloud of CLOUDS) for (const spacecraft of spacecraftCases) for (const levels of levelCases) {
    const expected = legacyLandsatFilters(start, end, cloud, spacecraft, levels);
    const actual = buildSchemaCatalogFilters(schema, {
      platform: spacecraft,
      product_type: levels,
      acquisition_time: { start, end },
      cloud_cover: cloud,
    });
    assert.equal(JSON.stringify(actual), JSON.stringify(expected), JSON.stringify({ start, end, cloud, spacecraft, levels }));
  }
});

test("optical builders delegate filters to the schema and keep their routes", () => {
  const s2 = buildSentinel2L2aCatalogQuery({
    aoiGeojson: aoi, startUtc: "2026-07-01", endUtc: "2026-07-02", cloudCoverLte: "12.5",
    resultLimit: 100, spacecraft: ["sentinel_2c", "sentinel_2a"],
  });
  assert.equal(s2.ok, true);
  if (!s2.ok) return;
  assert.equal(s2.query.provider_id, SENTINEL2_L2A_PROVIDER_ID);
  assert.equal(s2.query.native_collection_id, SENTINEL2_L2A_COLLECTION_ID);
  assert.equal(JSON.stringify(s2.query.filters), JSON.stringify({
    product_type: SENTINEL2_L2A_PRODUCT_TYPE,
    spacecraft: ["sentinel_2a", "sentinel_2c"],
    datetime: { start: "2026-07-01T00:00:00Z", end: "2026-07-02T00:00:00Z" },
    cloud_cover_lte: 12.5,
  }));

  const landsat = buildLandsatC2L2CatalogQuery({
    aoiGeojson: aoi, startUtc: "", endUtc: "", cloudCoverLte: "",
    spacecraft: ["landsat_9", "landsat_8"], processingLevels: ["l2sr"], resultLimit: 100,
  });
  assert.equal(landsat.ok, true);
  if (!landsat.ok) return;
  assert.equal(landsat.query.provider_id, MPC_LANDSAT_PROVIDER_ID);
  assert.equal(landsat.query.native_collection_id, MPC_LANDSAT_COLLECTION_ID);
  assert.equal(JSON.stringify(landsat.query.filters), JSON.stringify({
    spacecraft: ["landsat_8", "landsat_9"],
    processing_levels: ["l2sr"],
  }));
});

test("initial drafts equal the Workbench defaults the hand-written state used", () => {
  assert.deepEqual(initialSearchFilterDraft(searchFilterSchema(S2)), {
    platform: [],
    acquisition_time: { start: "", end: "" },
    cloud_cover: "",
  });
  assert.deepEqual(initialSearchFilterDraft(searchFilterSchema(LANDSAT)), {
    platform: ["landsat_4", "landsat_5", "landsat_7", "landsat_8", "landsat_9"],
    product_type: ["l2sp", "l2sr"],
    acquisition_time: { start: "", end: "" },
    cloud_cover: "",
  });
});

test("changing the reviewed schema data changes behaviour without code changes", () => {
  // The Owner goal for #3: a filter edit is a contract edit, not a UI edit.
  const edited = structuredClone(searchFilterSchema(S2)) as SearchFilterSchema;
  for (const filter of edited.filters) {
    if (filter.control === "number" && filter.filter_id === "cloud_cover") filter.maximum = 50;
    if (filter.control === "multi_enum" && filter.filter_id === "platform") {
      filter.options.push({ value: "sentinel_2d", label: "Sentinel-2D", description: "test-only option" });
    }
  }
  const tooCloudy = buildSchemaCatalogFilters(edited, { cloud_cover: "60" });
  assert.deepEqual(tooCloudy, { ok: false, error: "最大云量必须是 0 到 100 之间的数字。" });
  const newPlatform = buildSchemaCatalogFilters(edited, { platform: ["sentinel_2d"], cloud_cover: "40" });
  assert.equal(newPlatform.ok, true);
  if (newPlatform.ok) assert.deepEqual(newPlatform.filters.spacecraft, ["sentinel_2d"]);
  // The shipped schema is untouched.
  assert.equal(buildSchemaCatalogFilters(searchFilterSchema(S2), { platform: ["sentinel_2d"] }).ok, false);
});

test("the schema contract carries no provider identity, native fields or secrets", () => {
  const text = readFileSync(new URL("../../../contracts/v3/search-filter-schemas.v1.json", import.meta.url), "utf8").toLowerCase();
  for (const forbidden of [
    "provider_id", "native_collection_id", "endpoint", "http", "credential", "token", "password",
    "copernicus.dataspace", "planetary-computer", "earth-search", "usgs.eros", "eo:cloud_cover",
  ]) {
    assert.ok(!text.includes(forbidden), forbidden);
  }
  // Issue #3 R2 B5 (2026-10-19): HLS L30/S30 v2.0 schemas added.
  assert.deepEqual(SEARCH_FILTER_SCHEMA_CATALOG.schemas.map((schema) => schema.schema_id), [
    S2, "search.optical.sentinel-2.l1c.v1", LANDSAT, "search.optical.sentinel-3.v1", "search.optical.hls.l30.v2", "search.optical.hls.s30.v2",
  ]);
  assert.throws(() => searchFilterSchema("search.unknown.v1"), /Unknown search filter schema/);
});

function multiEnum(schemaId: string, filterId: string): MultiEnumSearchFilter {
  const filter = searchFilterSchema(schemaId).filters.find((item) => item.filter_id === filterId);
  assert.ok(filter && filter.control === "multi_enum");
  return filter as MultiEnumSearchFilter;
}

test("menu changes follow each filter's empty-selection rule", () => {
  // Sentinel-2 platforms: empty means all and is omitted from the query.
  const s2Platform = multiEnum(S2, "platform");
  assert.deepEqual(updateMultiEnumSelection(s2Platform, []), []);
  assert.deepEqual(updateMultiEnumSelection(s2Platform, ["sentinel_2c"]), ["sentinel_2c"]);
  // Landsat platforms and types are never sent empty: clearing restores all.
  const lsPlatform = multiEnum(LANDSAT, "platform");
  assert.deepEqual(updateMultiEnumSelection(lsPlatform, []), ["landsat_4", "landsat_5", "landsat_7", "landsat_8", "landsat_9"]);
  assert.deepEqual(updateMultiEnumSelection(multiEnum(LANDSAT, "product_type"), []), ["l2sp", "l2sr"]);
  // The shared menu shows "全部" (empty) when every option is selected.
  assert.deepEqual(multiEnumMenuValues(lsPlatform, ["landsat_4", "landsat_5", "landsat_7", "landsat_8", "landsat_9"]), []);
  assert.deepEqual(multiEnumMenuValues(lsPlatform, ["landsat_9"]), ["landsat_9"]);
});

test("facet and condition groups keep schema order and draft accessors are defensive", () => {
  const schema = searchFilterSchema(LANDSAT);
  assert.deepEqual(facetFilters(schema).map((filter) => filter.filter_id), ["platform", "instrument", "product_level", "product_type"]);
  assert.deepEqual(conditionFilters(schema).map((filter) => filter.filter_id), ["acquisition_time", "cloud_cover"]);
  const draft = { platform: ["landsat_9"], acquisition_time: { start: "2026-07-01", end: "2026-07-02" }, cloud_cover: "30" };
  assert.deepEqual(draftSelection(draft, "platform"), ["landsat_9"]);
  assert.deepEqual(draftSelection(draft, "cloud_cover"), []);
  assert.deepEqual(draftDateRange(draft, "acquisition_time"), { start: "2026-07-01", end: "2026-07-02" });
  assert.deepEqual(draftDateRange(draft, "missing"), { start: "", end: "" });
  assert.equal(draftText(draft, "cloud_cover"), "30");
  assert.equal(draftText(draft, "platform"), "");
  const broken = structuredClone(schema);
  broken.filters[0] = { control: "date_range", filter_id: "platform", label: "x", group: "facet", query_key: "spacecraft" };
  assert.throws(() => facetFilters(broken), /must be a multi-select or fixed fact/);
});

test("draft builders and field builders produce identical queries", () => {
  const draftS2 = { platform: ["sentinel_2b"], acquisition_time: { start: "2026-07-01", end: "2026-07-02" }, cloud_cover: "15" };
  const fromDraft = buildSentinel2L2aCatalogQueryFromDraft({ aoiGeojson: aoi, draft: draftS2, resultLimit: 100 });
  const fromFields = buildSentinel2L2aCatalogQuery({
    aoiGeojson: aoi, startUtc: "2026-07-01", endUtc: "2026-07-02", cloudCoverLte: "15", spacecraft: ["sentinel_2b"], resultLimit: 100,
  });
  assert.equal(JSON.stringify(fromDraft), JSON.stringify(fromFields));
  const draftLs = initialSearchFilterDraft(searchFilterSchema(LANDSAT));
  const ls = buildLandsatC2L2CatalogQueryFromDraft({ aoiGeojson: aoi, draft: draftLs, resultLimit: 100 });
  assert.equal(ls.ok, true);
  if (ls.ok) assert.equal(JSON.stringify(ls.query.filters), JSON.stringify({ spacecraft: ["landsat_4", "landsat_5", "landsat_7", "landsat_8", "landsat_9"], processing_levels: ["l2sp", "l2sr"] }));
  assert.deepEqual(
    buildLandsatC2L2CatalogQueryFromDraft({ aoiGeojson: null, draft: draftLs, resultLimit: 100 }),
    { ok: false, error: "请先提供真实 AOI 几何；Landsat 检索不会用边界框代替。" },
  );
});
