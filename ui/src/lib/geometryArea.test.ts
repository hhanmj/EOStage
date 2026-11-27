import assert from "node:assert/strict";
import test from "node:test";

import { geometryAreaKm2 } from "./geometryArea.ts";

test("geometry area uses polygon rings and subtracts holes", () => {
  const outer = geometryAreaKm2({
    type: "Polygon",
    coordinates: [[[0, 0], [1, 0], [1, 1], [0, 1], [0, 0]]],
  });
  const withHole = geometryAreaKm2({
    type: "Polygon",
    coordinates: [
      [[0, 0], [1, 0], [1, 1], [0, 1], [0, 0]],
      [[0.25, 0.25], [0.75, 0.25], [0.75, 0.75], [0.25, 0.75], [0.25, 0.25]],
    ],
  });
  assert.ok(outer > 12_000 && outer < 13_000);
  assert.ok(withHole > 0 && withHole < outer);
});

test("geometry area handles multipolygons across the antimeridian without bbox inflation", () => {
  const area = geometryAreaKm2({
    type: "MultiPolygon",
    coordinates: [
      [[[179.5, 0], [-179.5, 0], [-179.5, 1], [179.5, 1], [179.5, 0]]],
      [[[10, 0], [10.5, 0], [10.5, 0.5], [10, 0.5], [10, 0]]],
    ],
  });
  assert.ok(area > 12_000 && area < 20_000);
});
