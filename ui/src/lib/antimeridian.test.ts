import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import type { Bbox, Json } from "./bridge.ts";
import { DEM_BBOX_BUFFER_DEGREES } from "./demSources.ts";
import {
  continuousDisplayBbox,
  expandedDemDisplayBbox,
  deriveWrappedAoiEnvelope,
  shiftPolygonalGeojsonToLongitudeReference,
  shiftPolygonalGeojsonToDisplayDomain,
} from "./antimeridian.ts";

type FixtureCase = {
  id: string;
  geometry: Json;
  crosses_antimeridian: boolean;
  display: [number, number, number, number];
  windows: [number, number, number, number][];
};

const fixtureCases = JSON.parse(
  readFileSync(new URL("../../../tests/fixtures/geospatial/antimeridian_aoi_cases.json", import.meta.url), "utf8"),
) as { cases: FixtureCase[] };

function fixture(id: string): FixtureCase {
  const value = fixtureCases.cases.find((item) => item.id === id);
  assert.ok(value, `missing shared fixture ${id}`);
  return value;
}

function expectedBbox([west, south, east, north]: [number, number, number, number]): Bbox {
  return { west, south, east, north, crs: "EPSG:4326" };
}

test("New Zealand derives the native two-window contract", () => {
  const value = deriveWrappedAoiEnvelope(fixture("new-zealand").geometry);
  assert.deepEqual(value?.requestWindows, fixture("new-zealand").windows.map(expectedBbox));
  assert.deepEqual(
    [value?.displayWest, value?.south, value?.displayEast, value?.north],
    fixture("new-zealand").display,
  );
  assert.equal(value?.crossesAntimeridian, true);
  assert.deepEqual(continuousDisplayBbox(value!), expectedBbox(fixture("new-zealand").display));
});

test("realistic Nominatim decimal extrema keep New Zealand in one local display domain", () => {
  // Reduced from the production Nominatim New Zealand relation. The decimal
  // extrema are intentional: modulo normalization must not move an already
  // canonical coordinate outside the render domain by a floating-point ulp.
  const geometry = {
    type: "MultiPolygon",
    coordinates: [
      [[
        [165.5533714, -52.8213687],
        [179.3643594, -52.8213687],
        [179.3643594, -29.0303303],
        [165.5533714, -29.0303303],
        [165.5533714, -52.8213687],
      ]],
      [[
        [-179.059153, -44.6349947],
        [-175.541422, -44.6349947],
        [-175.541422, -43.3108013],
        [-179.059153, -43.3108013],
        [-179.059153, -44.6349947],
      ]],
    ],
  };

  const value = deriveWrappedAoiEnvelope(geometry);
  assert.ok(value, "the real provider decimal extrema must remain renderable");
  assert.equal(value.crossesAntimeridian, true);
  assert.ok(value.displayEast - value.displayWest < 30);
  assert.deepEqual(value.requestWindows, [
    { west: 165.5533714, south: -52.8213687, east: 180, north: -29.0303303, crs: "EPSG:4326" },
    { west: -180, south: -52.8213687, east: -175.541422, north: -29.0303303, crs: "EPSG:4326" },
  ]);
});

test("ordinary Hubei remains one unchanged native window", () => {
  const value = deriveWrappedAoiEnvelope(fixture("hubei").geometry);
  assert.deepEqual(value?.requestWindows, fixture("hubei").windows.map(expectedBbox));
  assert.deepEqual(
    [value?.displayWest, value?.south, value?.displayEast, value?.north],
    fixture("hubei").display,
  );
  assert.equal(value?.crossesAntimeridian, false);
});

test("an antimeridian United States-style AOI keeps a bounded visible DEM envelope", () => {
  const geometry: Json = {
    type: "MultiPolygon",
    coordinates: [
      [[[-125, 25], [-66, 25], [-66, 49], [-125, 49], [-125, 25]]],
      [[[170, 51], [-170, 51], [-170, 66], [170, 66], [170, 51]]],
    ],
  };
  const envelope = deriveWrappedAoiEnvelope(geometry);
  assert.ok(envelope);

  const dem = expandedDemDisplayBbox(envelope, DEM_BBOX_BUFFER_DEGREES);
  assert.deepEqual(dem, continuousDisplayBbox(envelope));
  assert.ok(dem.east - dem.west < 180);
  assert.ok(dem.west <= envelope.displayWest && dem.east >= envelope.displayEast);
});

test("production-precision Hubei extrema survive the ordinary display interval unchanged", () => {
  // Reduced from the DataV 420000 geometry after the provider's reviewed
  // GCJ-02 -> EPSG:4326 normalization. A +360/-360 round-trip moves the west
  // endpoint by one ulp and must not make this ordinary China AOI disappear.
  const west = 108.36246974074933;
  const geometry = {
    type: "Polygon",
    coordinates: [[
      [west, 29.822904140486955],
      [111.99619022065494, 32.461191334560155],
      [114.71388869786355, 29.3999513022449],
      [west, 29.822904140486955],
    ]],
  };

  const value = deriveWrappedAoiEnvelope(geometry);
  assert.ok(value, "the production-precision DataV boundary must remain renderable");
  assert.equal(value.crossesAntimeridian, false);
  assert.equal(value.displayWest, west);
  assert.deepEqual(shiftPolygonalGeojsonToDisplayDomain(geometry, value), geometry);
});

test("render projection shifts only whole world copies and never mutates source GeoJSON", () => {
  const source = fixture("new-zealand").geometry;
  const before = JSON.parse(JSON.stringify(source)) as Json;
  const envelope = deriveWrappedAoiEnvelope(source);
  assert.ok(envelope);

  const rendered = shiftPolygonalGeojsonToDisplayDomain(source, envelope);
  assert.ok(rendered);
  assert.deepEqual(source, before);

  const coordinates = (rendered as { coordinates: number[][][][] }).coordinates;
  assert.deepEqual(coordinates[0]![0]![0], [166, -47]);
  assert.deepEqual(coordinates[1]![0]![0], [181, -45]);
  assert.deepEqual(coordinates[1]![0]![1], [185, -45]);
});

test("render projection keeps a seam-spanning scene ring in the same world copy", () => {
  const envelope = deriveWrappedAoiEnvelope(fixture("new-zealand").geometry);
  assert.ok(envelope);
  const scene: Json = {
    type: "Polygon",
    coordinates: [[
      [179, -45],
      [-179, -45],
      [-179, -35],
      [179, -35],
      [179, -45],
    ]],
  };
  const rendered = shiftPolygonalGeojsonToDisplayDomain(scene, envelope);
  assert.ok(rendered);
  const coordinates = (rendered as { coordinates: number[][][] }).coordinates;
  assert.deepEqual(coordinates[0]!.map(([longitude]) => longitude), [179, 181, 181, 179, 179]);
});

test("scene render-reference projection permits a valid footprint to extend beyond a Beijing AOI", () => {
  const beijing: Json = {
    type: "Polygon",
    coordinates: [[[115.4, 39.4], [117.6, 39.4], [117.6, 41.1], [115.4, 41.1], [115.4, 39.4]]],
  };
  const scene: Json = {
    type: "Polygon",
    coordinates: [[[114, 38], [119, 38], [119, 43], [114, 43], [114, 38]]],
  };
  const envelope = deriveWrappedAoiEnvelope(beijing);
  assert.ok(envelope);

  assert.equal(
    shiftPolygonalGeojsonToDisplayDomain(scene, envelope),
    null,
    "the strict AOI projection still requires containment",
  );
  assert.deepEqual(
    shiftPolygonalGeojsonToLongitudeReference(scene, envelope),
    scene,
    "a scene chooses the same longitude world without being clipped to the AOI",
  );
});

test("malformed or unsupported geometry fails closed", () => {
  assert.equal(deriveWrappedAoiEnvelope({ type: "Point", coordinates: [180, 0] }), null);
  assert.equal(
    deriveWrappedAoiEnvelope({
      type: "Polygon",
      coordinates: [[[0, 0], [1, 0], [1, 1]]],
    }),
    null,
  );
  assert.equal(
    shiftPolygonalGeojsonToDisplayDomain({ type: "Point", coordinates: [180, 0] }, {
      south: -1,
      north: 1,
      crossesAntimeridian: false,
      displayWest: -1,
      displayEast: 1,
      requestWindows: [expectedBbox([-1, -1, 1, 1])],
    }),
    null,
  );
});

test("legacy envelope budget counts UTF-8 bytes before structural parsing", () => {
  const oversized = {
    ...fixture("hubei").geometry,
    label: "汉".repeat(4_500_000),
  };
  assert.equal(deriveWrappedAoiEnvelope(oversized), null);
});

test("an explicit full-world edge keeps the native global control", () => {
  const value = deriveWrappedAoiEnvelope({
    type: "Polygon",
    coordinates: [[[-180, -90], [180, -90], [180, 90], [-180, 90], [-180, -90]]],
  });
  assert.deepEqual(value?.requestWindows, [{ west: -180, south: -90, east: 180, north: 90, crs: "EPSG:4326" }]);
  assert.deepEqual([value?.displayWest, value?.displayEast], [-180, 180]);
});

test("unsupported exact-180 tie fails closed instead of rotating to a 90..270 copy", () => {
  const geometry: Json = {
    type: "Polygon",
    coordinates: [[[-90, 0], [90, 0], [90, 1], [-90, 1], [-90, 0]]],
  };
  assert.equal(deriveWrappedAoiEnvelope(geometry), null);
});

test("unsupported disconnected geometry fails closed instead of clipping a component", () => {
  const geometry: Json = {
    type: "MultiPolygon",
    coordinates: [
      [[[-177, 0], [-177, 1], [-135, 0], [-177, 0]]],
      [[[-171, 10], [15, 11], [27, 10], [-171, 10]]],
    ],
  };
  assert.equal(deriveWrappedAoiEnvelope(geometry), null);
});

test("unsupported wide disconnected geometry fails closed instead of shrinking -150/0/150", () => {
  const geometry: Json = {
    type: "MultiPolygon",
    coordinates: [
      [[[-150, 0], [-140, 0], [-140, 1], [-150, 1], [-150, 0]]],
      [[[-1, 10], [1, 10], [1, 11], [-1, 11], [-1, 10]]],
      [[[140, 20], [150, 20], [150, 21], [140, 21], [140, 20]]],
    ],
  };
  assert.equal(deriveWrappedAoiEnvelope(geometry), null);
});

test("global display authority rejects a seam ring that cannot fit one copy", () => {
  const envelope = {
    south: -90,
    north: 90,
    crossesAntimeridian: false,
    displayWest: -180,
    displayEast: 180,
    requestWindows: [expectedBbox([-180, -90, 180, 90])],
  } as const;
  const seamRing: Json = {
    type: "Polygon",
    coordinates: [[[179, -1], [-179, -1], [-179, 1], [179, 1], [179, -1]]],
  };
  assert.equal(shiftPolygonalGeojsonToDisplayDomain(seamRing, envelope), null);
});

test("legacy fallback bounds a large component collection without spread/recursion failure", () => {
  const geometry: Json = {
    type: "MultiPolygon",
    coordinates: Array.from({ length: 30_000 }, (_, index) => {
      const west = -179 + (index % 300) * 0.01;
      const south = -80 + Math.floor(index / 300) * 0.01;
      return [[[west, south], [west + 0.005, south], [west + 0.005, south + 0.005], [west, south + 0.005], [west, south]]];
    }),
  };
  assert.doesNotThrow(() => {
    const envelope = deriveWrappedAoiEnvelope(geometry);
    assert.ok(envelope === null || envelope.displayEast > envelope.displayWest);
  });
});
