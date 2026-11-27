import assert from "node:assert/strict";
import test from "node:test";

import type { SnapshotItem } from "./types.ts";
import {
  buildResultRowBadges,
  opticalTileLabel,
  projectSnapshotItemToSceneRow,
} from "./rowProjection.ts";
import { sortScenes } from "./resultListLogic.ts";

function snapshotItem(
  itemId: string,
  providerId: string,
  displayName: string,
  logicalDataset: string,
  properties: Record<string, unknown>,
  extras: Record<string, unknown> = {},
): SnapshotItem {
  return {
    item_id: itemId,
    provider_id: providerId,
    display_name: displayName,
    acquisition_datetime: "2026-10-04T03:26:01Z",
    footprint_geojson: null,
    metadata: {
      logical_dataset: logicalDataset,
      product_kind: "optical_scene",
      properties,
      assets: [],
    },
    payload: {},
    ...extras,
  } as SnapshotItem;
}

function badgeValues(item: SnapshotItem) {
  const row = projectSnapshotItemToSceneRow(item);
  return Object.fromEntries(buildResultRowBadges(row, "optical").map((badge) => [badge.key, badge.value]));
}

test("CDSE Sentinel-2 rows project platform, product type, MGRS tile, cloud and asset size", () => {
  const item = snapshotItem(
    "s2c-item",
    "copernicus.dataspace.odata",
    "S2C_MSIL2A_20261004T032601_N0513_R018_T48RXQ_20261004T091007.SAFE",
    "optical.sentinel-2.l2a",
    { cloud_cover: 99.85, product_type: "S2MSI2A" },
    {
      metadata: {
        logical_dataset: "optical.sentinel-2.l2a",
        product_kind: "optical_scene",
        properties: { cloud_cover: 99.85, product_type: "S2MSI2A" },
        assets: [{ asset_key: "archive", size_bytes: 733061241 }],
      },
      asset_access: { expected_size_bytes: 733061241 },
    },
  );

  const row = projectSnapshotItemToSceneRow(item);
  assert.equal(row.platform, "Sentinel-2C");
  assert.equal(row.product_type, "S2MSI2A");
  assert.equal(row.file_size_remote, 733061241);
  assert.equal(opticalTileLabel(row), "Tile T48RXQ");
  assert.deepEqual(badgeValues(item), {
    date: "2026-10-04",
    platform: "Sentinel-2C",
    cloud: "云量 99.9%",
    level: "S2MSI2A",
    size: "699.1 MB",
  });
});

test("Planetary Computer Landsat rows project Landsat platform, WRS path/row and cloud", () => {
  const item = snapshotItem(
    "landsat-item",
    "microsoft.planetary-computer",
    "LC09_L2SP_123045_20261004_20261005_02_T1",
    "optical.landsat.c2.l2",
    {
      spacecraft: "landsat_9",
      processing_level: "l2sp",
      data_type_l2: "OLI_TIRS_L2SP",
      cloud_cover: 12.5,
      wrs_path: "123",
      wrs_row: "045",
    },
    {
      metadata: {
        logical_dataset: "optical.landsat.c2.l2",
        product_kind: "optical_scene",
        properties: {
          spacecraft: "landsat_9",
          processing_level: "l2sp",
          data_type_l2: "OLI_TIRS_L2SP",
          cloud_cover: 12.5,
          wrs_path: "123",
          wrs_row: "045",
        },
        assets: [{ asset_key: "bundle", size_bytes: 1200000000 }],
      },
    },
  );

  const row = projectSnapshotItemToSceneRow(item);
  assert.equal(row.platform, "Landsat 9");
  assert.equal(row.processing_level, "l2sp");
  assert.equal(row.file_size_remote, 1200000000);
  assert.equal(opticalTileLabel(row), "WRS 123/045");
  assert.equal(badgeValues(item).cloud, "云量 12.5%");
});

test("HLS L30/S30 rows retain their own platform labels and MGRS tiles", () => {
  for (const [token, platform] of [["L30", "Landsat 9"], ["S30", "Sentinel-2C"]] as const) {
    const item = snapshotItem(
      `hls-${token}`,
      "microsoft.planetary-computer",
      `HLS.${token}.T49RDP.2024177T030756.v2.0`,
      `optical.hls.${token.toLowerCase()}.v2`,
      { platform: token === "L30" ? "landsat_9" : "sentinel_2c", cloud_cover: 4.25 },
    );
    const row = projectSnapshotItemToSceneRow(item);
    assert.equal(row.platform, platform);
    assert.equal(opticalTileLabel(row), "Tile T49RDP");
    assert.equal(badgeValues(item).cloud, "云量 4.3%");
    assert.equal(badgeValues(item).size, undefined);
  }
});

test("optical cloud and size sorting use the projected snapshot values", () => {
  const cloudy = projectSnapshotItemToSceneRow(snapshotItem(
    "cloudy",
    "copernicus.dataspace.odata",
    "S2A_MSIL2A_20261004T032601_N0513_R018_T48RXQ_20261004T091007.SAFE",
    "optical.sentinel-2.l2a",
    { cloud_cover: 90 },
    { asset_access: { expected_size_bytes: 100 } },
  ));
  const clear = projectSnapshotItemToSceneRow(snapshotItem(
    "clear",
    "copernicus.dataspace.odata",
    "S2B_MSIL2A_20261004T032601_N0513_R018_T48RXQ_20261004T091007.SAFE",
    "optical.sentinel-2.l2a",
    { cloud_cover: 2 },
    { asset_access: { expected_size_bytes: 900 } },
  ));

  assert.deepEqual(sortScenes([clear, cloudy], "cloud", "asc").map((row) => row.scene_id), ["clear", "cloudy"]);
  assert.deepEqual(sortScenes([clear, cloudy], "size", "desc").map((row) => row.scene_id), ["clear", "cloudy"]);
});
