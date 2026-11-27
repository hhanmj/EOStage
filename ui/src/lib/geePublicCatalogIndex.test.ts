import test from "node:test";
import assert from "node:assert/strict";
import {
  GEE_CATALOG_THEMES,
  GEE_PUBLIC_CATALOG_INDEX,
  filterGeePublicCatalog,
  isStaticGeeAssetId,
  lookupGeePublicDataset,
} from "./geePublicCatalogIndex.ts";

/**
 * Owner Round-5 Item 7 & 8 decision:
 * Prune redundant/stale entries from 55 down to 24 curated GEE-unique verified datasets
 * with explicit `assetKind` (`IMAGE` | `IMAGE_COLLECTION`), `filterTemplate`
 * (`optical` | `sar` | `climate_env` | `static`), and selectable band choices.
 */
test("Curated GEE public catalog holds 22–26 unique verified datasets covering all themes with templates", () => {
  assert.ok(
    GEE_PUBLIC_CATALOG_INDEX.length >= 22 && GEE_PUBLIC_CATALOG_INDEX.length <= 26,
    `Expected curated 22–26 GEE datasets, got ${GEE_PUBLIC_CATALOG_INDEX.length}`
  );
  const uniqueIds = new Set(GEE_PUBLIC_CATALOG_INDEX.map((d) => d.assetId));
  assert.equal(uniqueIds.size, GEE_PUBLIC_CATALOG_INDEX.length, "assetIds must be unique");

  for (const theme of GEE_CATALOG_THEMES) {
    if (theme.id === "all") continue;
    const subset = filterGeePublicCatalog(theme.id, "");
    assert.ok(subset.length >= 2, `theme ${theme.id} should have >= 2 curated datasets`);
  }

  for (const item of GEE_PUBLIC_CATALOG_INDEX) {
    assert.ok(
      item.assetKind === "IMAGE" || item.assetKind === "IMAGE_COLLECTION",
      `missing assetKind on ${item.assetId}`
    );
    assert.ok(
      ["optical", "sar", "climate_env", "static"].includes(item.filterTemplate),
      `missing filterTemplate on ${item.assetId}`
    );
    assert.ok(
      Array.isArray(item.selectableBands) && item.selectableBands.length >= 1,
      `selectableBands required on ${item.assetId}`
    );
  }
});

test("filterGeePublicCatalog and lookupGeePublicDataset resolve GEM-15, static templates, and keywords", () => {
  const sarMatches = filterGeePublicCatalog("sar", "palsar");
  assert.ok(sarMatches.some((d) => d.assetId === "JAXA/ALOS/PALSAR/YEARLY/SAR_EPOCH"));

  const rainMatches = filterGeePublicCatalog("all", "chirps");
  assert.ok(rainMatches.some((d) => d.assetId === "UCSB-CHG/CHIRPS/DAILY"));

  const nightMatches = filterGeePublicCatalog("atmosphere", "夜光");
  assert.ok(nightMatches.some((d) => d.assetId.includes("VIIRS/DNB")));

  const gem15Matches = filterGeePublicCatalog("dem", "gem-15");
  assert.ok(
    gem15Matches.some(
      (d) =>
        d.assetId === "projects/climate-and-sustainability/assets/gem-15_v1" &&
        d.presetResourceId === "science.gee.gem-15.v1" &&
        d.filterTemplate === "static"
    ),
    "GEM-15 must be discoverable in DEM theme of GEE public catalog as static template"
  );

  assert.equal(lookupGeePublicDataset("ESA/WorldCover/v200")?.filterTemplate, "static");
  assert.equal(lookupGeePublicDataset("COPERNICUS/S2_SR_HARMONIZED")?.filterTemplate, "optical");
  assert.equal(lookupGeePublicDataset("COPERNICUS/S1_GRD")?.filterTemplate, "sar");
  assert.equal(lookupGeePublicDataset("ECMWF/ERA5_LAND/HOURLY")?.filterTemplate, "climate_env");
  assert.equal(isStaticGeeAssetId("projects/climate-and-sustainability/assets/gem-15_v1"), true);
  assert.equal(isStaticGeeAssetId("COPERNICUS/S2_SR_HARMONIZED"), false);
});
