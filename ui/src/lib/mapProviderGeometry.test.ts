import assert from "node:assert/strict";
import test from "node:test";

import {
  projectBboxForMapProvider,
  projectGeometryForMapProvider,
} from "./mapProviderGeometry.ts";

test("EPSG basemaps keep canonical coordinates in an independent render copy", () => {
  const geometry = {
    type: "Polygon",
    coordinates: [[[116.397, 39.908, 52], [116.407, 39.908, 52], [116.397, 39.908, 52]]],
  };
  const before = structuredClone(geometry);
  const rendered = projectGeometryForMapProvider(geometry, "EPSG:3857");
  assert.deepEqual(rendered, before);
  assert.notStrictEqual(rendered, geometry);
  assert.deepEqual(geometry, before);
});

test("GCJ-02 basemaps shift Beijing render coordinates without mutating scientific geometry", () => {
  const geometry = {
    type: "GeometryCollection",
    geometries: [
      { type: "Point", coordinates: [116.397, 39.908, 52] },
      { type: "Point", coordinates: [2.3522, 48.8566] },
    ],
  };
  const before = structuredClone(geometry);
  const rendered = projectGeometryForMapProvider(geometry, "GCJ-02") as typeof geometry;
  const beijing = rendered.geometries[0]!.coordinates;
  const paris = rendered.geometries[1]!.coordinates;
  assert.ok(Math.abs(beijing[0]! - 116.40324) < 0.002);
  assert.ok(Math.abs(beijing[1]! - 39.9094) < 0.002);
  assert.equal(beijing[2], 52);
  assert.deepEqual(paris, [2.3522, 48.8566]);
  assert.deepEqual(geometry, before);
});

test("bbox projection is render-only and keeps out-of-China windows exact", () => {
  const beijing = { west: 116.3, south: 39.8, east: 116.5, north: 40.0, crs: "EPSG:4326" };
  const before = { ...beijing };
  const shifted = projectBboxForMapProvider(beijing, "GCJ-02");
  assert.ok(shifted.west > beijing.west);
  assert.ok(shifted.east > beijing.east);
  assert.equal(shifted.crs, "GCJ-02");
  assert.deepEqual(beijing, before);

  const paris = { west: 2.2, south: 48.7, east: 2.5, north: 49.0, crs: "EPSG:4326" };
  assert.deepEqual(projectBboxForMapProvider(paris, "GCJ-02"), paris);
});
