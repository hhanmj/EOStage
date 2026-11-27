import assert from "node:assert/strict";
import test from "node:test";
import {
  availableBandsForOpticalFamily,
  bandPresetOrder,
  classifyOpticalDerivedScene,
  extractWgs84RingsAndBbox,
  resolveOpticalDerivedSelection,
  type OpticalAoiSceneLike,
} from "./opticalAoiClipLogic.ts";

test("classifyOpticalDerivedScene supports MPC/CDSE L2A Sentinel-2 and rejects USGS M2M / CDSE SAFE-only archives", () => {
  assert.deepEqual(
    classifyOpticalDerivedScene({
      scene_id: "s2-1",
      provider: "microsoft.planetary-computer",
      native_metadata: { logical_dataset: "optical.sentinel-2.l2a" },
    }),
    { eligible: true, family: "sentinel2_l2a" },
  );

  assert.deepEqual(
    classifyOpticalDerivedScene({
      scene_id: "s2-earth-search",
      provider: "element84.earth-search",
      native_metadata: { logical_dataset: "optical.sentinel-2.l2a" },
    }),
    { eligible: true, family: "sentinel2_l2a" },
  );

  assert.deepEqual(
    classifyOpticalDerivedScene({
      scene_id: "l8-earth-search",
      provider: "element84.earth-search",
      native_metadata: { logical_dataset: "optical.landsat.c2.l2" },
    }),
    { eligible: false, reasonCode: "UNSUPPORTED_OPTICAL_SOURCE" },
  );

  assert.deepEqual(
    classifyOpticalDerivedScene({
      scene_id: "l8-mpc",
      provider: "microsoft.planetary-computer",
      native_metadata: { logical_dataset: "optical.landsat.c2.l2" },
    }),
    { eligible: true, family: "landsat_c2_l2" },
  );

  assert.deepEqual(
    classifyOpticalDerivedScene({
      scene_id: "l8-usgs",
      provider: "usgs.eros.m2m",
      native_metadata: {
        logical_dataset: "optical.landsat.c2.l2",
        native_collection_id: "landsat_ot_c2_l2",
      },
    }),
    { eligible: false, reasonCode: "USGS_M2M_ARCHIVE_UNSUPPORTED" },
  );

  assert.deepEqual(
    classifyOpticalDerivedScene({
      scene_id: "s2-cdse",
      provider: "copernicus.dataspace.odata",
      native_metadata: { logical_dataset: "optical.sentinel-2.l2a" },
    }),
    { eligible: true, family: "sentinel2_l2a" },
  );

  assert.deepEqual(
    classifyOpticalDerivedScene({
      scene_id: "s2-cdse-l1c",
      provider: "copernicus.dataspace.odata",
      native_metadata: { logical_dataset: "optical.sentinel-2.l1c" },
    }),
    { eligible: true, family: "sentinel2_l1c" },
  );

  assert.deepEqual(
    classifyOpticalDerivedScene({
      scene_id: "S2A_MSIL1C_20250101T025041_N0511_R132_T50SNA_20250101T061234.SAFE",
      provider: "copernicus.dataspace.odata",
      native_metadata: { logical_dataset: "optical.sentinel-2.l2a" },
    }),
    { eligible: true, family: "sentinel2_l1c" },
  );

  assert.deepEqual(
    classifyOpticalDerivedScene({
      scene_id: "s3-cdse",
      provider: "copernicus.dataspace.odata",
      native_metadata: { logical_dataset: "optical.sentinel-3.standard" },
    }),
    { eligible: false, reasonCode: "CDSE_SAFE_ARCHIVE_UNSUPPORTED" },
  );

  assert.deepEqual(
    classifyOpticalDerivedScene({
      scene_id: "hls-s30",
      provider: "microsoft.planetary-computer",
      native_metadata: { logical_dataset: "optical.hls.s30.v2" },
    }),
    { eligible: true, family: "hls_s30" },
  );

  assert.deepEqual(
    classifyOpticalDerivedScene({
      scene_id: "hls-l30",
      provider: "microsoft.planetary-computer",
      native_metadata: { logical_dataset: "optical.hls.l30.v2" },
    }),
    { eligible: true, family: "hls_l30" },
  );
});

test("bandPresetOrder and availableBandsForOpticalFamily preserve sensor-native band names", () => {
  assert.deepEqual(bandPresetOrder("sentinel2_l2a", "true_color"), ["B04", "B03", "B02"]);
  assert.deepEqual(bandPresetOrder("sentinel2_l1c", "true_color"), ["B04", "B03", "B02"]);
  assert.deepEqual(bandPresetOrder("landsat_c2_l2", "true_color"), ["red", "green", "blue"]);
  assert.deepEqual(bandPresetOrder("hls_l30", "false_color_nir"), ["B05", "B04", "B03"]);
  assert.deepEqual(bandPresetOrder("hls_s30", "swir"), ["B12", "B8A", "B04"]);

  const landsatBands = availableBandsForOpticalFamily("landsat_c2_l2");
  assert.ok(landsatBands.every((band) => band.resolutionM === 30));
  assert.ok(availableBandsForOpticalFamily("sentinel2_l2a").some((band) => band.id === "B8A"));
  const l1cBands = availableBandsForOpticalFamily("sentinel2_l1c");
  assert.equal(l1cBands.length, 13);
  assert.ok(l1cBands.some((band) => band.id === "B10" && band.resolutionM === 60));
});

test("resolveOpticalDerivedSelection infers single-scene vs batch single-scene clip and blocks >64 or mixed scenes", () => {
  const mkS2 = (scene_id: string): OpticalAoiSceneLike => ({
    scene_id,
    provider: "microsoft.planetary-computer",
    native_metadata: { logical_dataset: "optical.sentinel-2.l2a" },
  });

  assert.deepEqual(resolveOpticalDerivedSelection([mkS2("s2-1")]), {
    ok: true,
    family: "sentinel2_l2a",
    mode: "single_scene_clip",
    sceneIds: ["s2-1"],
  });

  const twoScenes = resolveOpticalDerivedSelection([mkS2("s2-1"), mkS2("s2-2")]);
  assert.deepEqual(twoScenes, {
    ok: true,
    family: "sentinel2_l2a",
    mode: "batch_single_scene_clip",
    sceneIds: ["s2-1", "s2-2"],
  });

  const sixtyFourScenes = Array.from({ length: 64 }, (_, i) => mkS2(`s2-${i}`));
  assert.equal(resolveOpticalDerivedSelection(sixtyFourScenes).ok, true);

  const sixtyFiveScenes = Array.from({ length: 65 }, (_, i) => mkS2(`s2-${i}`));
  assert.deepEqual(resolveOpticalDerivedSelection(sixtyFiveScenes), {
    ok: false,
    blockCode: "TOO_MANY_SCENES_FOR_MOSAIC",
  });

  assert.deepEqual(
    resolveOpticalDerivedSelection([
      mkS2("s2-1"),
      {
        scene_id: "l8-1",
        provider: "microsoft.planetary-computer",
        native_metadata: { logical_dataset: "optical.landsat.c2.l2" },
      },
    ]),
    { ok: false, blockCode: "MIXED_SENSOR_FAMILIES" },
  );
});

test("extractWgs84RingsAndBbox normalizes open Polygon rings and falls back to bbox", () => {
  const fromPoly = extractWgs84RingsAndBbox({
    type: "Polygon",
    coordinates: [[[114.2, 30.5], [114.4, 30.5], [114.4, 30.7], [114.2, 30.7]]],
  });
  assert.deepEqual(fromPoly, {
    aoiBboxWgs84: [114.2, 30.5, 114.4, 30.7],
    aoiRingsWgs84: [
      [
        [114.2, 30.5],
        [114.4, 30.5],
        [114.4, 30.7],
        [114.2, 30.7],
        [114.2, 30.5],
      ],
    ],
  });

  const fromFallback = extractWgs84RingsAndBbox(null, {
    west: 114.0,
    south: 30.0,
    east: 114.5,
    north: 30.5,
    crs: "EPSG:4326",
  });
  assert.equal(fromFallback?.aoiBboxWgs84[0], 114.0);
  assert.equal(fromFallback?.aoiRingsWgs84[0].length, 5);
  assert.equal(extractWgs84RingsAndBbox(null, null), null);
});
