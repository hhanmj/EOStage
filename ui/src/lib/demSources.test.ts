import assert from "node:assert/strict";
import test from "node:test";

import {
  DEM_DATASET_OPTIONS,
  DEM_DATASET_GROUPS,
  compatibleDemProviders,
  downloadableDemDatasets,
  demDatasetRequiresOpenTopographyKey,
  expandedDemRequestBbox,
  formatDemBboxCoordinate,
  resolvedDemProvider,
} from "./demSources.ts";

test("Copernicus datasets resolve to anonymous AWS without an API key", () => {
  for (const dataset of ["COP30", "COP90"]) {
    assert.equal(resolvedDemProvider(dataset), "COPERNICUS_AWS");
    assert.equal(demDatasetRequiresOpenTopographyKey(dataset), false);
    assert.equal(downloadableDemDatasets.has(dataset), true);
  }
});

test("user-facing DEM products are unique and hide provider duplication", () => {
  const values = DEM_DATASET_OPTIONS.map((option) => option.value);
  assert.equal(new Set(values).size, values.length);
  assert.equal(values.filter((value) => value === "COP30").length, 1);
  assert.equal(values.filter((value) => value === "COP90").length, 1);
  assert.deepEqual(compatibleDemProviders("COP30"), ["COPERNICUS_AWS", "OPENTOPOGRAPHY"]);
  assert.deepEqual(compatibleDemProviders("NASADEM"), ["OPENTOPOGRAPHY"]);
});

test("Copernicus datasets remain selectable through the OpenTopography provider", () => {
  const openTopography = DEM_DATASET_GROUPS.find((group) =>
    group.options.some((option) => option.provider === "OPENTOPOGRAPHY"),
  );
  const values = openTopography?.options
    .filter((option) => option.provider === "OPENTOPOGRAPHY")
    .map((option) => option.value);

  assert.ok(values?.includes("COP30"));
  assert.ok(values?.includes("COP90"));
  assert.equal(resolvedDemProvider("COP30", "OPENTOPOGRAPHY"), "OPENTOPOGRAPHY");
  assert.equal(demDatasetRequiresOpenTopographyKey("COP90", "OPENTOPOGRAPHY"), true);
});

test("NASADEM and ordinary AW3D30 remain explicit OpenTopography products", () => {
  for (const dataset of ["NASADEM", "AW3D30", "SRTM_GL1"]) {
    assert.equal(resolvedDemProvider(dataset), "OPENTOPOGRAPHY");
    assert.equal(demDatasetRequiresOpenTopographyKey(dataset), true);
    assert.equal(downloadableDemDatasets.has(dataset), true);
  }
});

test("NISAR DEM resolves to its own Earthdata-authenticated provider", () => {
  assert.equal(resolvedDemProvider("NISAR_DEM"), "NISAR_DEM_ASF");
  assert.equal(demDatasetRequiresOpenTopographyKey("NISAR_DEM"), false);
  assert.equal(downloadableDemDatasets.has("NISAR_DEM"), true);
  assert.deepEqual(compatibleDemProviders("NISAR_DEM"), ["NISAR_DEM_ASF"]);

  const option = DEM_DATASET_OPTIONS.find((item) => item.value === "NISAR_DEM");
  assert.ok(option, "NISAR_DEM must be a user-facing product");
  assert.equal(option.label, "NISAR DEM 30m");
  assert.equal(option.hint, "WGS84 椭球高 · 支持断点续传");

  const group = DEM_DATASET_GROUPS.find((candidate) =>
    candidate.options.some((item) => item.value === "NISAR_DEM"),
  );
  assert.ok(group, "NISAR_DEM must belong to a source group");
  assert.equal(group.label, "ASF NISAR DEM：需要 Earthdata 账号");
});

test("FABDEM V1-2 resolves to its anonymous CloudFront route with a non-commercial CC BY-NC-SA 4.0 hint", () => {
  assert.equal(resolvedDemProvider("FABDEM"), "FABDEM_PUBLIC");
  assert.equal(demDatasetRequiresOpenTopographyKey("FABDEM"), false);
  assert.equal(downloadableDemDatasets.has("FABDEM"), true);
  assert.deepEqual(compatibleDemProviders("FABDEM"), ["FABDEM_PUBLIC"]);

  const option = DEM_DATASET_OPTIONS.find((item) => item.value === "FABDEM");
  assert.ok(option, "FABDEM must be a user-facing product");
  assert.equal(option.label, "FABDEM V1-2 30m (去树冠/建筑裸地)");
  assert.match(option.hint ?? "", /CC BY-NC-SA 4\.0/);
});

test("Google GEM-15 DTM and DSM 15m resolve to GEM15_PUBLIC with CC BY 4.0 and regional coverage notice", () => {
  for (const dataset of ["GEM15_DTM", "GEM15_DSM"]) {
    assert.equal(resolvedDemProvider(dataset), "GEM15_PUBLIC");
    assert.equal(demDatasetRequiresOpenTopographyKey(dataset), false);
    assert.equal(downloadableDemDatasets.has(dataset), true);
    assert.deepEqual(compatibleDemProviders(dataset), ["GEM15_PUBLIC"]);
    const option = DEM_DATASET_OPTIONS.find((item) => item.value === dataset);
    assert.ok(option, `${dataset} must be a user-facing product`);
    assert.match(option.hint ?? "", /EGM96/);
    assert.match(option.hint ?? "", /CC BY 4\.0/);
  }
});

test("orthometric and ellipsoidal products have unique provider-dataset identities", () => {
  const values = DEM_DATASET_GROUPS.flatMap((group) =>
    group.options.map((option) => `${option.provider}:${option.value}`),
  );

  assert.ok(values.includes("OPENTOPOGRAPHY:AW3D30"));
  assert.ok(values.includes("OPENTOPOGRAPHY:AW3D30_ELLIPSOIDAL"));
  assert.equal(new Set(values).size, values.length);
});

test("unknown DEM values are not submitted through a fabricated provider", () => {
  assert.equal(resolvedDemProvider("FUTURE_DEM"), null);
  assert.equal(downloadableDemDatasets.has("FUTURE_DEM"), false);
});

test("DEM uses the exact AOI rectangle without padding or coordinate rounding", () => {
  assert.deepEqual(
    expandedDemRequestBbox({ west: 104.4485163, east: 112.0563984, south: 20.9018167, north: 26.3890405 }),
    {
      west: 104.4485163,
      east: 112.0563984,
      south: 20.9018167,
      north: 26.3890405,
      crs: "EPSG:4326",
    },
  );
  assert.equal(formatDemBboxCoordinate(104.4), "104.400");
  assert.deepEqual(
    expandedDemRequestBbox({ west: -179.99, east: -179, south: 89, north: 89.99 }),
    { west: -179.99, east: -179, south: 89, north: 89.99, crs: "EPSG:4326" },
  );
});

test("a sub-millidegree DEM rectangle is not collapsed by display precision", () => {
  const bbox = { west: 100.0001, east: 100.0002, south: 30.0001, north: 30.0002 };
  assert.deepEqual(expandedDemRequestBbox(bbox), { ...bbox, crs: "EPSG:4326" });
  assert.equal(formatDemBboxCoordinate(bbox.west), "100.000");
});
