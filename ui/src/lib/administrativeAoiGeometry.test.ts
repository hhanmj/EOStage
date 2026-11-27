import assert from "node:assert/strict";
import test from "node:test";

import { canonicalAdministrativeAoiGeometry } from "./administrativeAoiGeometry.ts";
import { catalogSearchDecision } from "./operationPrerequisites.ts";
import { buildSarSearchQuery, createSarSearchFormState } from "./sarSearchForm.ts";
import type { DatasetCapabilities } from "./generated/v3-contracts.ts";

const polygon = {
  type: "Polygon",
  coordinates: [
    [
      [113.9, 30.4],
      [114.5, 30.4],
      [114.5, 30.8],
      [113.9, 30.4],
    ],
  ],
} as const;

const capabilities: DatasetCapabilities = {
  dataset: "SENTINEL-1",
  display_name: "Sentinel-1",
  platforms: ["S1A", "S1B"],
  instruments: ["C-SAR"],
  processing_levels: ["SLC"],
  product_families: ["SLC"],
  beam_modes: ["IW"],
  polarizations: ["VV", "VV+VH"],
  supports_flight_direction: true,
  supports_look_direction: false,
  supports_relative_orbit: true,
  supports_frame: true,
  supports_path: true,
  supports_burst: false,
  track_filter: { supported: true, parameter: "relativeOrbit", label: "Relative orbit", value_type: "integer", pattern: "^\\d{1,3}$" },
  frame_filter: { supported: true, parameter: "frame", label: "Frame", value_type: "integer", pattern: "^\\d+$" },
  supported_actions: [],
};

test("administrative AOI binds the direct polygon geometry used by SAR search", () => {
  const geometry = canonicalAdministrativeAoiGeometry({
    type: "Feature",
    properties: { name: "Wuhan" },
    geometry: polygon,
  });

  assert.deepEqual(geometry, polygon);
  assert.equal(geometry?.type, "Polygon");
  assert.equal("properties" in (geometry ?? {}), false);

  const scope = catalogSearchDecision({
    provider: "asf",
    aoiGeojson: geometry,
    filtersValid: true,
  });
  assert.equal(scope.ok, true);
  if (!scope.ok) return;
  assert.equal(scope.request.aoi_geojson, geometry);

  const query = buildSarSearchQuery(createSarSearchFormState(capabilities), {
    provider: "ASF",
    bbox: null,
    aoiGeojson: geometry,
    start: null,
    end: null,
    maxResults: 20,
  });
  assert.equal(query.aoi_geojson, geometry);
  assert.equal(query.max_results, 20);
});

test("administrative AOI keeps direct MultiPolygon and rejects collection wrappers", () => {
  const multiPolygon = {
    type: "MultiPolygon",
    coordinates: [polygon.coordinates],
  } as const;

  assert.equal(canonicalAdministrativeAoiGeometry(multiPolygon), multiPolygon);
  assert.equal(
    canonicalAdministrativeAoiGeometry({
      type: "FeatureCollection",
      features: [{ type: "Feature", properties: {}, geometry: polygon }],
    }),
    null,
  );
  assert.equal(
    canonicalAdministrativeAoiGeometry({
      type: "GeometryCollection",
      geometries: [polygon],
    }),
    null,
  );
});
