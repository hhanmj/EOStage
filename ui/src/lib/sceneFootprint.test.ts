import assert from "node:assert/strict";
import test from "node:test";

import { sceneFootprintRenderModel } from "./sceneFootprint.ts";

test("map admission renders each authoritative Polygon and MultiPolygon part without a bbox substitute", () => {
  const polygon = {
    type: "Polygon",
    coordinates: [[[100, 30], [101, 30], [101, 31], [100, 31], [100, 30]]],
  };
  const multiPolygon = {
    type: "MultiPolygon",
    coordinates: [
      [[[110, 40], [111, 40], [111, 41], [110, 41], [110, 40]]],
      [[[112, 42], [113, 42], [113, 43], [112, 43], [112, 42]]],
    ],
  };

  assert.deepEqual(sceneFootprintRenderModel(polygon), {
    geojson: polygon,
    bbox: { west: 100, south: 30, east: 101, north: 31, crs: "EPSG:4326" },
    polygons: [[[[30, 100], [30, 101], [31, 101], [31, 100], [30, 100]]]],
  });
  assert.equal(sceneFootprintRenderModel(multiPolygon)?.polygons.length, 2);
  assert.equal(sceneFootprintRenderModel(null), null);
});

test("scene footprint preserves canonical bbox and foreign members but rejects extra render DTO keys", () => {
  const canonical = {
    type: "Polygon",
    bbox: [100, 30, 101, 31],
    "x-scene-owner": { source: "catalogue", version: 1 },
    coordinates: [[[100, 30], [101, 30], [101, 31], [100, 31], [100, 30]]],
  };
  const rendered = {
    type: "Polygon",
    coordinates: [[[100, 30], [101, 30], [101, 31], [100, 31], [100, 30]]],
  };
  const model = sceneFootprintRenderModel(canonical, rendered);
  assert.ok(model);
  assert.deepEqual(model?.geojson, canonical);

  assert.equal(sceneFootprintRenderModel(canonical, { ...rendered, "x-render-extra": true }), null);
});

test("scene footprint rejects non-JSON-safe canonical foreign values", () => {
  const canonical = {
    type: "Polygon",
    coordinates: [[[100, 30], [101, 30], [101, 31], [100, 31], [100, 30]]],
    "x-scene-owner": new Date("2026-08-20T00:00:00Z"),
  };
  assert.equal(sceneFootprintRenderModel(canonical), null);
});

test("map admission rejects malformed geometry instead of drawing a bbox-only rectangle", () => {
  assert.equal(
    sceneFootprintRenderModel({
      type: "Polygon",
      coordinates: [[[100, 30], [101, 30], [101, 31]]],
    }),
    null,
  );
});

test("each ring and every MultiPolygon member must be non-degenerate with entirely numeric positions", () => {
  const polygon = (rings: unknown[]) => ({ type: "Polygon", coordinates: rings });
  const validExterior = [[100, 30], [102, 30], [102, 32], [100, 32], [100, 30]];

  assert.equal(sceneFootprintRenderModel(polygon([
    [[100, 30], [101, 30], [100, 30], [101, 30], [100, 30]],
  ])), null, "exterior must have three distinct horizontal positions");
  assert.equal(sceneFootprintRenderModel(polygon([
    validExterior,
    [[100.5, 30.5], [101.5, 30.5], [100.5, 30.5], [101.5, 30.5], [100.5, 30.5]],
  ])), null, "a zero-area hole must reject the complete polygon");
  assert.equal(sceneFootprintRenderModel({
    type: "MultiPolygon",
    coordinates: [[validExterior], [[[110, 40], [111, 40], [110, 40], [111, 40], [110, 40]]]],
  }), null, "a degenerate member must not be hidden by a valid member");
  assert.equal(sceneFootprintRenderModel(polygon([
    [[100, 30, 1], [102, 30, "not-numeric"], [102, 32, 1], [100, 32, 1], [100, 30, 1]],
  ])), null, "all supplied ordinates must be finite numbers");
});

test("valid holes, disjoint members, finite 3D positions, and antimeridian rings remain renderable", () => {
  const withHole = sceneFootprintRenderModel({
    type: "Polygon",
    coordinates: [
      [[100, 30], [104, 30], [104, 34], [100, 34], [100, 30]],
      [[101, 31], [103, 31], [103, 33], [101, 33], [101, 31]],
    ],
  });
  const disjoint = sceneFootprintRenderModel({
    type: "MultiPolygon",
    coordinates: [
      [[[100, 30], [101, 30], [101, 31], [100, 31], [100, 30]]],
      [[[110, 40], [111, 40], [111, 41], [110, 41], [110, 40]]],
    ],
  });
  const threeDimensional = sceneFootprintRenderModel({
    type: "Polygon",
    coordinates: [[[100, 30, 50], [101, 30, 51], [101, 31, 52], [100, 31, 53], [100, 30, 50]]],
  });
  const antimeridian = sceneFootprintRenderModel({
    type: "Polygon",
    coordinates: [[[179, 10], [-179, 10], [-179, 11], [179, 11], [179, 10]]],
  });

  assert.equal(withHole?.polygons[0]?.length, 2);
  assert.equal(disjoint?.polygons.length, 2);
  assert.ok(threeDimensional);
  assert.ok(antimeridian);
});

test("scene footprint preserves bit-exact full-world edges in canonical and native render copies", () => {
  const canonical = {
    type: "Polygon",
    coordinates: [[[-180, 0], [180, 0], [180, 1], [-180, 1], [-180, 0]]],
  };
  const rendered = {
    type: "Polygon",
    coordinates: [[[180, 0], [540, 0], [540, 1], [180, 1], [180, 0]]],
  };
  assert.ok(sceneFootprintRenderModel(canonical));
  const model = sceneFootprintRenderModel(canonical, rendered);
  assert.ok(model);
  assert.deepEqual(model?.bbox, { west: 180, south: 0, east: 540, north: 1, crs: "EPSG:4326" });
  assert.equal(model?.polygons[0]?.[0]?.[1]?.[1], 540);
});

test("scene footprint accepts a visible full-world render subset when a far canonical component is omitted", () => {
  const canonical = {
    type: "MultiPolygon",
    coordinates: [
      [[[-180, 0], [180, 0], [180, 1], [-180, 1], [-180, 0]]],
      [[[10, 40], [11, 40], [11, 41], [10, 41], [10, 40]]],
    ],
  };
  const rendered = {
    type: "Polygon",
    coordinates: [[[180, 0], [540, 0], [540, 1], [180, 1], [180, 0]]],
  };
  const model = sceneFootprintRenderModel(canonical, rendered);
  assert.ok(model);
  assert.deepEqual(model?.bbox, { west: 180, south: 0, east: 540, north: 1, crs: "EPSG:4326" });
  assert.equal(model?.polygons.length, 1);
});

test("scene footprint accepts exact adjacent render copies of one seam polygon", () => {
  const canonical = {
    type: "Polygon",
    coordinates: [[[170, -1], [-170, -1], [-170, 1], [170, 1], [170, -1]]],
  };
  const rendered = {
    type: "MultiPolygon",
    coordinates: [
      [[[-190, -1], [-170, -1], [-170, 1], [-190, 1], [-190, -1]]],
      [[[170, -1], [190, -1], [190, 1], [170, 1], [170, -1]]],
    ],
  };
  const model = sceneFootprintRenderModel(canonical, rendered);
  assert.ok(model);
  assert.equal(model?.polygons.length, 2);
  assert.deepEqual(model?.bbox, { west: -190, south: -1, east: 190, north: 1, crs: "EPSG:4326" });
});

test("scene footprint uses the shared matcher for a 30,000-component full-world subset", () => {
  const smallComponents = Array.from({ length: 30_000 }, (_, index) => {
    const west = -179 + (index % 300) * 0.01;
    const south = -80 + Math.floor(index / 300) * 0.01;
    return [[[west, south], [west + 0.005, south], [west + 0.005, south + 0.005], [west, south + 0.005], [west, south]]];
  });
  const canonical = {
    type: "MultiPolygon",
    coordinates: [
      [[[-180, 0], [180, 0], [180, 1], [-180, 1], [-180, 0]]],
      ...smallComponents,
    ],
  };
  const rendered = {
    type: "Polygon",
    coordinates: [[[180, 0], [540, 0], [540, 1], [180, 1], [180, 0]]],
  };
  assert.doesNotThrow(() => {
    const model = sceneFootprintRenderModel(canonical, rendered);
    assert.equal(model?.polygons.length, 1);
  });
});

test("scene footprint closes rings by horizontal coordinates while render copies remain 2D", () => {
  const canonical = {
    type: "Polygon",
    coordinates: [[
      [-180, 0, 10, 100], [180, 0, 11, 101], [180, 1, 12, 102],
      [-180, 1, 13, 103], [-180, 0],
    ]],
  };
  const rendered = {
    type: "Polygon",
    coordinates: [[[180, 0], [540, 0], [540, 1], [180, 1], [180, 0]]],
  };
  assert.ok(sceneFootprintRenderModel(canonical, rendered));
});

test("scene footprint rejects a near-global pseudo-full render copy", () => {
  const canonical = {
    type: "Polygon",
    coordinates: [[[-179, 0], [179, 0], [179, 1], [-179, 1], [-179, 0]]],
  };
  const pseudoFull = {
    type: "Polygon",
    coordinates: [[[181, 0], [539, 0], [539, 1], [181, 1], [181, 0]]],
  };
  assert.equal(sceneFootprintRenderModel(canonical, pseudoFull), null);
});

test("scene footprint rejects a non-degenerate self-intersecting ring", () => {
  assert.equal(sceneFootprintRenderModel({
    type: "Polygon",
    coordinates: [[[0, 0], [4, 4], [0, 3], [4, 0], [0, 0]]],
  }), null);
});

test("scene footprint rejects a full-world copy with unrelated latitude", () => {
  const canonical = {
    type: "Polygon",
    coordinates: [[[-180, 0], [180, 0], [180, 1], [-180, 1], [-180, 0]]],
  };
  const shiftedLatitude = {
    type: "Polygon",
    coordinates: [[[180, 0], [540, 0], [540, 2], [180, 2], [180, 0]]],
  };
  assert.equal(sceneFootprintRenderModel(canonical, shiftedLatitude), null);
});

test("scene footprint applies one shared self-intersection budget across canonical and render geometry", () => {
  const ring = Array.from({ length: 4_095 }, (_, index) => {
    const angle = (index / 4_095) * Math.PI * 2;
    return [Number((Math.cos(angle) * 0.5).toFixed(6)), Number((Math.sin(angle) * 0.5).toFixed(6))];
  });
  ring.push(ring[0]!);
  const geometry = {
    type: "MultiPolygon",
    coordinates: Array.from({ length: 50 }, () => [ring]),
  };
  assert.equal(sceneFootprintRenderModel(geometry, structuredClone(geometry)), null);
});

test("scene footprint bbox remains bounded for a 30,000-component native scene", () => {
  const geometry = {
    type: "MultiPolygon",
    coordinates: Array.from({ length: 30_000 }, (_, index) => {
      const west = -179 + (index % 300) * 0.01;
      const south = -80 + Math.floor(index / 300) * 0.01;
      return [[[west, south], [west + 0.005, south], [west + 0.005, south + 0.005], [west, south + 0.005], [west, south]]];
    }),
  };
  assert.doesNotThrow(() => {
    const model = sceneFootprintRenderModel(geometry);
    assert.equal(model?.polygons.length, 30_000);
  });
});

test("scene footprint bbox remains bounded for a 30,000-component native scene", () => {
  const geometry = {
    type: "MultiPolygon",
    coordinates: Array.from({ length: 30_000 }, (_, index) => {
      const west = -179 + (index % 300) * 0.01;
      const south = -80 + Math.floor(index / 300) * 0.01;
      return [[[west, south], [west + 0.005, south], [west + 0.005, south + 0.005], [west, south + 0.005], [west, south]]];
    }),
  };
  assert.doesNotThrow(() => {
    const model = sceneFootprintRenderModel(geometry);
    assert.equal(model?.polygons.length, 30_000);
  });
});
