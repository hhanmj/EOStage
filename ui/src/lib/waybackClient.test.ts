import test from "node:test";
import assert from "node:assert/strict";
import {
  WAYBACK_TILE_ORIGIN,
  formatWaybackSubfolderName,
  isValidWaybackSourceKey,
  isValidWaybackVersionId,
  joinOutputSubdir,
  scaleWaybackEstimateForPeriods,
  waybackTileUrlTemplate,
} from "./waybackLogic.ts";
import { estimateXyzTilesForBbox } from "./xyzTileEstimator.ts";

test("waybackClient validates release IDs and emits exact CSP-allowlisted tile template", () => {
  assert.equal(isValidWaybackVersionId("3692"), true);
  assert.equal(isValidWaybackVersionId("25951"), true);
  assert.equal(isValidWaybackVersionId(""), false);
  assert.equal(isValidWaybackVersionId("3692/../etc"), false);
  assert.equal(isValidWaybackVersionId("abc"), false);

  assert.equal(isValidWaybackSourceKey("wayback_3692"), true);
  assert.equal(isValidWaybackSourceKey("WAYBACK_10"), true);
  assert.equal(isValidWaybackSourceKey("esri_wayback"), false);
  assert.equal(isValidWaybackSourceKey("wayback_bad"), false);

  const tpl = waybackTileUrlTemplate("3692");
  assert.equal(
    tpl,
    `${WAYBACK_TILE_ORIGIN}/arcgis/rest/services/world_imagery/mapserver/tile/3692/{z}/{y}/{x}`,
  );
  assert.throws(() => waybackTileUrlTemplate("../bad"), /INVALID_WAYBACK_VERSION_ID/);
});

test("formatWaybackSubfolderName and joinOutputSubdir produce safe Windows and POSIX paths", () => {
  assert.equal(
    formatWaybackSubfolderName("3692", "2026-11-18"),
    "wayback_2026-11-18_id3692",
  );
  assert.equal(
    formatWaybackSubfolderName("10", "../2014/02/20"),
    "wayback_20140220_id10",
  );
  assert.equal(
    joinOutputSubdir("D:\\Data\\Hefei\\", "wayback_2026-11-18_id3692"),
    "D:\\Data\\Hefei\\wayback_2026-11-18_id3692",
  );
  assert.equal(
    joinOutputSubdir("/data/Hefei/", "/wayback_2026-11-18_id3692/"),
    "/data/Hefei/wayback_2026-11-18_id3692",
  );
});

test("scaleWaybackEstimateForPeriods multiplies tile and byte totals cleanly", () => {
  const est = estimateXyzTilesForBbox(
    { west: 117.25, south: 31.84, east: 117.27, north: 31.86 },
    [16],
    { source: "esri_wayback", maxZoomOverride: 19 },
  );
  assert.ok(est);
  assert.equal(est.maxZoom, 19);

  const scaled = scaleWaybackEstimateForPeriods(est, 4);
  assert.equal(scaled.periodCount, 4);
  assert.equal(scaled.totalTiles, est.totalTiles * 4);
  assert.equal(scaled.totalNetworkDownloadBytes, est.totalNetworkDownloadBytes * 4);
  assert.equal(scaled.totalEstimatedOutputBytes, est.totalEstimatedOutputBytes * 4);
});
