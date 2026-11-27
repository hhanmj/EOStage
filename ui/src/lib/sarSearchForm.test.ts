import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import type { DatasetCapabilities } from "./generated/v3-contracts.ts";
import {
  buildSarSearchQuery,
  completeSarSearchQuery,
  createSarSearchFormState,
  parseSarMaxResultsInput,
  reconcileSarSearchForm,
} from "./sarSearchForm.ts";

const sentinelCapabilities: DatasetCapabilities = {
  dataset: "SENTINEL-1",
  display_name: "Sentinel-1",
  platforms: ["S1A", "S1B"],
  instruments: ["C-SAR"],
  processing_levels: [
    "SLC",
    "GRD_HD",
    "GRD_HS",
    "GRD_MD",
    "GRD_MS",
    "GRD_FD",
    "RAW",
    "OCN",
  ],
  product_families: ["SLC", "GRD", "RAW", "OCN"],
  beam_modes: ["SM", "IW", "EW", "WV"],
  polarizations: ["VV", "VH", "HH", "HV", "VV+VH", "HH+HV"],
  supports_flight_direction: true,
  supports_look_direction: false,
  supports_relative_orbit: true,
  supports_frame: true,
  supports_path: true,
  supports_burst: false,
  track_filter: {
    supported: true,
    parameter: "relativeOrbit",
    label: "Relative orbit",
    value_type: "integer",
    pattern: "^\\d{1,3}$",
  },
  frame_filter: {
    supported: true,
    parameter: "frame",
    label: "Frame",
    value_type: "integer",
    pattern: "^\\d+$",
  },
  supported_actions: [],
};

const commonInputs = {
  provider: "ASF",
  start: "2024-01-01T00:00:00Z",
  end: "2024-01-31T23:59:59Z",
  bbox: {
    west: 100,
    east: 101,
    south: 30,
    north: 31,
    crs: "EPSG:4326",
  },
  aoiGeojson: null,
  maxResults: 100,
};

test("result limit accepts finite 1 through 10000 while blank safely defaults to 250", () => {
  for (const value of [1, 17, 250, 251, 617, 3000, 10_000]) {
    assert.equal(parseSarMaxResultsInput(String(value)), value);
  }
  assert.equal(parseSarMaxResultsInput(""), 250);
  assert.equal(parseSarMaxResultsInput("   "), 250);
  for (const invalid of ["0", "-1", "1.5", "1e3", "unsafe", "10001", "9007199254740992"]) {
    assert.throws(() => parseSarMaxResultsInput(invalid), /1 到 10000/, invalid);
  }
});

test("empty mission filters remain unrestricted", () => {
  const state = createSarSearchFormState(sentinelCapabilities);
  const query = buildSarSearchQuery(state, commonInputs);

  assert.deepEqual(query.platforms, []);
  assert.deepEqual(query.beam_modes, []);
  assert.deepEqual(query.polarizations, []);
  assert.equal(query.processing_levels[0], "SLC");
});

test("multiple native processing levels are sent without broad product families", () => {
  const state = createSarSearchFormState(sentinelCapabilities);
  const query = buildSarSearchQuery(
    {
      ...state,
      processingLevels: ["GRD_HD", "GRD_HS"],
    },
    commonInputs,
  );

  assert.deepEqual(query.product_families, []);
  assert.deepEqual(query.processing_levels, ["GRD_HD", "GRD_HS"]);
});

test("multiple imaging modes and polarizations remain arrays", () => {
  const state = createSarSearchFormState(sentinelCapabilities);
  const query = buildSarSearchQuery(
    {
      ...state,
      beamModes: ["IW", "EW"],
      polarizations: ["VV", "VH", "VV+VH"],
    },
    commonInputs,
  );

  assert.deepEqual(query.beam_modes, ["IW", "EW"]);
  assert.deepEqual(query.polarizations, ["VV", "VH", "VV+VH"]);
});

test("flight direction is unrestricted unless exactly one direction is selected", () => {
  const state = createSarSearchFormState(sentinelCapabilities);

  assert.equal(buildSarSearchQuery(state, commonInputs).flight_direction, null);
  assert.equal(
    buildSarSearchQuery(
      { ...state, flightDirections: ["ASCENDING"] },
      commonInputs,
    ).flight_direction,
    "ASCENDING",
  );
  assert.equal(
    buildSarSearchQuery(
      { ...state, flightDirections: ["ASCENDING", "DESCENDING"] },
      commonInputs,
    ).flight_direction,
    null,
  );
});

test("dataset reconciliation removes selections illegal for the new dataset", () => {
  const current = {
    ...createSarSearchFormState(sentinelCapabilities),
    platforms: ["S1A", "S1B"],
    processingLevels: ["GRD_HD"],
    beamModes: ["IW", "EW"],
    polarizations: ["VV", "VV+VH"],
    trackRange: { start: "10", end: "20" },
    frameRange: { start: "100", end: "200" },
  };
  const alosCapabilities: DatasetCapabilities = {
    ...sentinelCapabilities,
    dataset: "ALOS PALSAR",
    display_name: "ALOS PALSAR",
    platforms: ["ALOS"],
    instruments: ["PALSAR"],
    processing_levels: ["L1.0", "L1.1", "L1.5"],
    product_families: ["SLC"],
    beam_modes: ["FBS", "FBD"],
    polarizations: ["HH", "HH+HV"],
    track_filter: null,
    frame_filter: null,
  };

  const reconciled = reconcileSarSearchForm(current, alosCapabilities);

  assert.equal(reconciled.dataset, "ALOS PALSAR");
  assert.deepEqual(reconciled.platforms, ["ALOS"]);
  assert.deepEqual(reconciled.processingLevels, []);
  assert.deepEqual(reconciled.beamModes, []);
  assert.deepEqual(reconciled.polarizations, []);
  assert.deepEqual(reconciled.trackRange, { start: "", end: "" });
  assert.deepEqual(reconciled.frameRange, { start: "", end: "" });
});

test("a dataset with one legal value selects it instead of showing an ambiguous all value", () => {
  const singletonCapabilities: DatasetCapabilities = {
    ...sentinelCapabilities,
    dataset: "SEASAT",
    display_name: "SEASAT",
    platforms: ["SEASAT 1"],
    instruments: [],
    processing_levels: ["L1"],
    product_families: ["SLC"],
    beam_modes: ["STD"],
    polarizations: ["HH"],
  };

  const state = createSarSearchFormState(singletonCapabilities);

  assert.deepEqual(state.platforms, ["SEASAT 1"]);
  assert.deepEqual(state.processingLevels, ["L1"]);
  assert.deepEqual(state.beamModes, ["STD"]);
  assert.deepEqual(state.polarizations, ["HH"]);
});

test("switching datasets applies singleton defaults and clears stale mission choices", () => {
  const current = {
    ...createSarSearchFormState(sentinelCapabilities),
    platforms: ["S1A"],
    processingLevels: ["GRD_HD"],
    beamModes: ["IW"],
    polarizations: ["VV+VH"],
  };
  const singletonCapabilities: DatasetCapabilities = {
    ...sentinelCapabilities,
    dataset: "JERS-1",
    display_name: "JERS-1",
    platforms: ["JERS-1"],
    instruments: [],
    processing_levels: ["L1"],
    product_families: ["SLC"],
    beam_modes: [],
    polarizations: ["HH"],
  };

  const state = reconcileSarSearchForm(current, singletonCapabilities);

  assert.deepEqual(state.platforms, ["JERS-1"]);
  assert.deepEqual(state.processingLevels, ["L1"]);
  assert.deepEqual(state.beamModes, []);
  assert.deepEqual(state.polarizations, ["HH"]);
});

test("datasets without a reviewed beam mode do not submit an invented default", () => {
  for (const dataset of ["ERS", "JERS-1"]) {
    const capabilities: DatasetCapabilities = {
      ...sentinelCapabilities,
      dataset,
      display_name: dataset,
      platforms: dataset === "ERS" ? ["ERS-1", "ERS-2"] : ["JERS-1"],
      instruments: dataset === "ERS" ? ["C-SAR"] : [],
      processing_levels: ["L0", "L1"],
      product_families: ["RAW", "SLC"],
      beam_modes: [],
      polarizations: [dataset === "ERS" ? "VV" : "HH"],
    };

    const query = buildSarSearchQuery(
      createSarSearchFormState(capabilities),
      commonInputs,
    );

    assert.deepEqual(query.beam_modes, [], dataset);
  }
});

test("padded frame ranges preserve leading zeroes", () => {
  const seasatCapabilities: DatasetCapabilities = {
    ...sentinelCapabilities,
    dataset: "SEASAT",
    display_name: "SEASAT",
    platforms: ["SEASAT"],
    instruments: ["SAR"],
    processing_levels: [],
    product_families: [],
    beam_modes: [],
    polarizations: [],
    track_filter: null,
    frame_filter: {
      supported: true,
      parameter: "asfframe",
      label: "ASF frame",
      value_type: "padded_string",
      pattern: "^\\d{4}$",
    },
  };
  const state = {
    ...createSarSearchFormState(seasatCapabilities),
    frameRange: { start: "0017", end: "0021" },
  };

  const query = buildSarSearchQuery(state, commonInputs);

  assert.deepEqual(query.frame_range, { start: "0017", end: "0021" });
  assert.equal(query.asf_frame, null);
});

test("a single Path or Frame endpoint preserves the Golden scalar filter semantics", () => {
  const state = {
    ...createSarSearchFormState(sentinelCapabilities),
    trackRange: { start: "118", end: "" },
    frameRange: { start: "", end: "72" },
  };

  const query = buildSarSearchQuery(state, commonInputs);

  assert.equal(query.relative_orbit, 118);
  assert.equal(query.frame, 72);
  assert.equal(query.asf_frame, null);
  assert.equal(query.track_range, null);
  assert.equal(query.frame_range, null);
});

test("a single padded ASF Frame endpoint remains a string without losing zeroes", () => {
  const seasatCapabilities: DatasetCapabilities = {
    ...sentinelCapabilities,
    dataset: "SEASAT",
    display_name: "SEASAT",
    platforms: ["SEASAT 1"],
    instruments: [],
    processing_levels: ["L1"],
    product_families: ["SLC"],
    beam_modes: ["STD"],
    polarizations: ["HH"],
    frame_filter: {
      supported: true,
      parameter: "asfframe",
      label: "ASF Frame",
      value_type: "padded_string",
      pattern: "^[0-9]+$",
      minimum: 208,
      maximum: 3458,
      exact_width: 4,
      platform_limits: [],
    },
  };
  const state = {
    ...createSarSearchFormState(seasatCapabilities),
    frameRange: { start: "0208", end: "" },
  };

  const query = buildSarSearchQuery(state, commonInputs);

  assert.equal(query.frame, null);
  assert.equal(query.asf_frame, "0208");
  assert.equal(query.frame_range, null);
});

test("capability bounds and fixed-width frames reject invalid mission ranges", () => {
  const seasatCapabilities: DatasetCapabilities = {
    ...sentinelCapabilities,
    dataset: "SEASAT",
    display_name: "SEASAT",
    platforms: ["SEASAT 1"],
    instruments: [],
    processing_levels: ["L1"],
    product_families: ["SLC"],
    beam_modes: ["STD"],
    polarizations: ["HH"],
    frame_filter: {
      supported: true,
      parameter: "asfframe",
      label: "ASF Frame",
      value_type: "padded_string",
      pattern: "^[0-9]+$",
      minimum: 208,
      maximum: 3458,
      exact_width: 4,
      platform_limits: [],
    },
  };
  const base = createSarSearchFormState(seasatCapabilities);

  assert.throws(
    () => buildSarSearchQuery(
      { ...base, frameRange: { start: "208", end: "0999" } },
      commonInputs,
    ),
    /ASF Frame.*invalid/i,
  );
  assert.deepEqual(
    buildSarSearchQuery(
      { ...base, frameRange: { start: "0208", end: "3458" } },
      commonInputs,
    ).frame_range,
    { start: "0208", end: "3458" },
  );
});

test("selected platform limits narrow a dataset filter range", () => {
  const ersCapabilities: DatasetCapabilities = {
    ...sentinelCapabilities,
    dataset: "ERS",
    display_name: "ERS-1 / ERS-2",
    platforms: ["ERS-1", "ERS-2"],
    track_filter: {
      supported: true,
      parameter: "relativeOrbit",
      label: "Path / Track",
      value_type: "integer",
      pattern: "^[0-9]+$",
      minimum: 0,
      maximum: 2410,
      exact_width: null,
      platform_limits: [
        { platform: "ERS-1", minimum: 0, maximum: 2410 },
        { platform: "ERS-2", minimum: 0, maximum: 500 },
      ],
    },
  };
  const base = createSarSearchFormState(ersCapabilities);

  assert.throws(
    () => buildSarSearchQuery(
      {
        ...base,
        platforms: ["ERS-2"],
        trackRange: { start: "501", end: "600" },
      },
      commonInputs,
    ),
    /Path \/ Track.*invalid/i,
  );
  assert.deepEqual(
    buildSarSearchQuery(
      {
        ...base,
        platforms: ["ERS-1"],
        trackRange: { start: "501", end: "600" },
      },
      commonInputs,
    ).track_range,
    { start: "501", end: "600" },
  );
  assert.throws(
    () => buildSarSearchQuery(
      {
        ...base,
        platforms: ["ERS-1"],
        trackRange: { start: "600", end: "501" },
      },
      commonInputs,
    ),
    /Path \/ Track.*start.*end/i,
  );
});

test("a range with one invalid endpoint is rejected", () => {
  const state = {
    ...createSarSearchFormState(sentinelCapabilities),
    trackRange: { start: "10", end: "invalid" },
  };

  assert.throws(
    () => buildSarSearchQuery(state, commonInputs),
    /Relative orbit.*invalid/i,
  );
});

test("canonical query defaults complete a legacy browser-search draft", () => {
  assert.deepEqual(
    completeSarSearchQuery({
      dataset: "SENTINEL-1",
      processing_levels: ["SLC"],
      beam_modes: [],
      polarizations: [],
      bbox: { west: 100, east: 101, south: 30, north: 31 },
    }),
    {
      provider: "ASF",
      dataset: "SENTINEL-1",
      platforms: [],
      instrument: null,
      processing_levels: ["SLC"],
      product_families: [],
      beam_modes: [],
      polarizations: [],
      flight_direction: null,
      look_direction: null,
      relative_orbit: null,
      frame: null,
      asf_frame: null,
      path: null,
      start: null,
      end: null,
      track_range: null,
      frame_range: null,
      aoi_geojson: null,
      aoi_wkt: null,
      bbox: {
        west: 100,
        east: 101,
        south: 30,
        north: 31,
        crs: "EPSG:4326",
      },
      max_results: null,
      burst: null,
    },
  );
});

test("browser search mock constructs rather than casts its executed query", () => {
  const bridgeSource = readFileSync(new URL("./bridge.ts", import.meta.url), "utf8");

  assert.match(
    bridgeSource,
    /executed_query:\s*completeSarSearchQuery\(params\)/,
  );
  assert.doesNotMatch(bridgeSource, /params as SarSearchQuery/);
});

test("SLC-BURST exposes only official searchable IDs in the query contract", () => {
  const capabilities: DatasetCapabilities = {
    ...sentinelCapabilities,
    dataset: "SLC-BURST",
    display_name: "Sentinel-1 SLC Burst",
    processing_levels: ["BURST"],
    product_families: ["BURST"],
    supports_frame: false,
    supports_path: false,
    supports_relative_orbit: false,
    supports_burst: true,
    track_filter: null,
    frame_filter: null,
  };
  const state = {
    ...createSarSearchFormState(capabilities),
    burstAbsoluteId: "101784",
    burstRelativeId: "32480",
    burstFullId: "101784-IW2",
  };

  const query = buildSarSearchQuery(state, commonInputs);

  assert.deepEqual(query.burst, {
    burst_id: null,
    full_burst_id: "101784-IW2",
    relative_burst_id: 32480,
    absolute_burst_id: 101784,
    subswath: null,
    burst_index: null,
  });
  assert.throws(
    () => buildSarSearchQuery({ ...state, burstAbsoluteId: "IW2" }, commonInputs),
    /Absolute Burst ID.*integer/,
  );
});
