import test from "node:test";
import assert from "node:assert/strict";
import {
  XYZ_JPEG_MAX_DIMENSION,
  XYZ_MAX_TILES_ARCHIVE,
  XYZ_MAX_TILES_MOSAIC,
  estimateXyzTilesForBbox,
  formatTileCount,
  formatXyzByteSize,
  getXyzMaxTilesLimit,
  getXyzSourceMaxZoom,
  lat2tile,
  lon2tile,
  normalizeXyzZooms,
} from "./xyzTileEstimator.ts";

test("lon2tile and lat2tile compute exact Web Mercator Z15 slippy coordinates", () => {
  // Hefei center (~117.28, 31.86) at Z15 (n = 32768)
  const x = lon2tile(117.28, 15);
  const y = lat2tile(31.86, 15);
  assert.equal(x, Math.floor(((117.28 + 180) / 360) * 32768));
  assert.ok(y > 0 && y < 32768);
  assert.equal(lon2tile(-180, 10), 0);
  assert.equal(lon2tile(180, 10), 1023);
});

test("normalizeXyzZooms deduplicates, sorts 1..22, and falls back cleanly", () => {
  assert.deepEqual(normalizeXyzZooms([18, 15, 15, 25, 0, 10]), [10, 15, 18]);
  assert.deepEqual(normalizeXyzZooms([], 16), [16]);
});

test("estimateXyzTilesForBbox sums tiles and scales output bytes across zoom levels & formats", () => {
  const bbox = { west: 117.15, south: 31.78, east: 117.35, north: 31.94 };
  const deflateEst = estimateXyzTilesForBbox(bbox, [12, 14], {
    format: "gtiff",
    compression: "DEFLATE",
    includeOverviews: false,
  });
  const noneEst = estimateXyzTilesForBbox(bbox, [12, 14], {
    format: "gtiff",
    compression: "NONE",
    includeOverviews: true,
  });
  const mbtilesEst = estimateXyzTilesForBbox(bbox, [12, 14], {
    format: "mbtiles",
  });

  assert.ok(deflateEst && noneEst && mbtilesEst);
  assert.equal(deflateEst.perZoom.length, 2);
  assert.equal(
    deflateEst.totalTiles,
    deflateEst.perZoom[0].tileCount + deflateEst.perZoom[1].tileCount,
  );
  assert.equal(deflateEst.maxWidthPx, deflateEst.perZoom[1].widthPx);
  assert.equal(deflateEst.maxHeightPx, deflateEst.perZoom[1].heightPx);
  assert.ok(deflateEst.totalEstimatedOutputBytes < noneEst.totalEstimatedOutputBytes);
  assert.ok(mbtilesEst.totalEstimatedOutputBytes > 0);
  assert.equal(deflateEst.exceedsHardLimit, false);
  assert.equal(deflateEst.exceedsSafeLimit, false);
  assert.equal(deflateEst.needsCogStagingDisk, false);
  assert.ok(deflateEst.estimatedPeakMemoryBytes >= 32 * 1024 * 1024 && deflateEst.estimatedPeakMemoryBytes <= 128 * 1024 * 1024);

  const cogEst = estimateXyzTilesForBbox(bbox, [12, 14], {
    format: "cog",
    compression: "DEFLATE",
    includeOverviews: true,
  });
  assert.ok(cogEst);
  assert.equal(cogEst.needsCogStagingDisk, true);
  assert.equal(mbtilesEst.estimatedPeakMemoryBytes, 24 * 1024 * 1024);
});

test("formatTileCount and formatXyzByteSize format KB, MB, and GB readably", () => {
  assert.equal(formatTileCount(12345), "12,345");
  assert.equal(formatXyzByteSize(512 * 1024), "512 KB");
  assert.equal(formatXyzByteSize(24.5 * 1024 * 1024), "24.5 MB");
  assert.equal(formatXyzByteSize(2.35 * 1024 * 1024 * 1024), "2.35 GB");
});

test("xyzTileEstimator enforces per-source maxZoom, 50k vs 200k limits, and JPEG 65535 px guard", () => {
  assert.equal(getXyzSourceMaxZoom("carto_light"), 16);
  assert.equal(getXyzSourceMaxZoom("opentopomap"), 17);
  assert.equal(getXyzSourceMaxZoom("tianditu_satellite"), 18);
  assert.equal(getXyzSourceMaxZoom("tianditu_vector"), 18);
  assert.equal(getXyzSourceMaxZoom("gaode_satellite"), 18);
  assert.equal(getXyzSourceMaxZoom("osm"), 19);
  assert.equal(getXyzSourceMaxZoom("arcgis_topo"), 19);
  assert.equal(getXyzSourceMaxZoom("arcgis_satellite"), 20);
  assert.equal(getXyzSourceMaxZoom("esri_wayback"), 20);
  assert.equal(getXyzSourceMaxZoom("wayback_3692"), 20);
  assert.equal(getXyzSourceMaxZoom("google_satellite"), 21);

  const probedOver = estimateXyzTilesForBbox(
    { west: 117.27, south: 31.85, east: 117.28, north: 31.86 },
    [20],
    { source: "esri_wayback", maxZoomOverride: 19 },
  );
  assert.ok(probedOver);
  assert.equal(probedOver.maxZoom, 19);
  assert.equal(probedOver.exceedsSourceMaxZoom, true);
  assert.equal(getXyzSourceMaxZoom("custom"), 22);

  assert.deepEqual(normalizeXyzZooms([15, 19, 22], 15, "tianditu_satellite"), [15]);
  assert.deepEqual(normalizeXyzZooms([20], 15, "tianditu_satellite"), [15]);
  assert.deepEqual(normalizeXyzZooms([18, 20], 15, "arcgis_satellite"), [18, 20]);

  assert.equal(getXyzMaxTilesLimit("gtiff", true), XYZ_MAX_TILES_MOSAIC);
  assert.equal(getXyzMaxTilesLimit("vrt", false), XYZ_MAX_TILES_MOSAIC);
  assert.equal(getXyzMaxTilesLimit("mbtiles", true), XYZ_MAX_TILES_ARCHIVE);
  assert.equal(getXyzMaxTilesLimit("zip", false), XYZ_MAX_TILES_ARCHIVE);
  assert.equal(getXyzMaxTilesLimit("zip", true), XYZ_MAX_TILES_MOSAIC);

  // ~125k tiles at Z18: rejected under stitched GeoTIFF (50k) but accepted under MBTiles archive (200k)
  const midBbox = { west: 117.0, south: 31.6, east: 117.5, north: 32.0 };
  const gtiffMid = estimateXyzTilesForBbox(midBbox, [18], {
    source: "arcgis_satellite",
    exportFormat: "gtiff",
    stitchMosaic: true,
  });
  const mbtMid = estimateXyzTilesForBbox(midBbox, [18], {
    source: "arcgis_satellite",
    exportFormat: "mbtiles",
    stitchMosaic: true,
  });
  assert.ok(gtiffMid && mbtMid);
  assert.equal(gtiffMid.totalTiles, mbtMid.totalTiles);
  assert.ok(gtiffMid.totalTiles > 50_000 && gtiffMid.totalTiles <= 200_000);
  assert.equal(gtiffMid.exceedsHardLimit, true);
  assert.equal(mbtMid.exceedsHardLimit, false);

  // > 200k tiles fails even in archive mode
  const hugeEst = estimateXyzTilesForBbox(
    { west: 115.0, south: 30.0, east: 118.0, north: 33.0 },
    [18],
    { source: "tianditu_satellite", exportFormat: "mbtiles", stitchMosaic: false },
  );
  assert.ok(hugeEst);
  assert.equal(hugeEst.maxTilesLimit, 200_000);
  assert.equal(hugeEst.exceedsHardLimit, true);

  // Out-of-ceiling zoom flagged explicitly
  const overZoom = estimateXyzTilesForBbox(
    { west: 117.27, south: 31.85, east: 117.28, north: 31.86 },
    [20],
    { source: "tianditu_satellite", exportFormat: "gtiff", stitchMosaic: true },
  );
  assert.ok(overZoom);
  assert.equal(overZoom.maxZoom, 18);
  assert.equal(overZoom.exceedsSourceMaxZoom, true);

  // Wide strip < 50k tiles but > 65,535 px triggers JPEG dimension guard
  const jpgStrip = estimateXyzTilesForBbox(
    { west: 117.0, south: 31.85, east: 117.45, north: 31.8505 },
    [18],
    { source: "arcgis_satellite", exportFormat: "jpg", stitchMosaic: true },
  );
  assert.ok(jpgStrip);
  assert.equal(jpgStrip.exceedsHardLimit, false);
  assert.ok(jpgStrip.maxGridWidthPx > XYZ_JPEG_MAX_DIMENSION);
  assert.equal(jpgStrip.exceedsJpegDimensionLimit, true);
});
