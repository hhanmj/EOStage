import assert from "node:assert/strict";
import test from "node:test";

import type { DemTaskStatus } from "./bridge.ts";
import {
  clampDockHeight,
  DEFAULT_DOCK_PREFERENCES,
  parseDockPreferences,
  projectDemTasksToDock,
  scrollTopForProduct,
  scopeSelectionKeys,
  virtualRowWindow,
} from "./catalogDock.ts";

test("dock preferences read old tabs but normalize to the single result surface", () => {
  assert.deepEqual(parseDockPreferences(null), DEFAULT_DOCK_PREFERENCES);
  assert.deepEqual(
    parseDockPreferences({ tab: "assets", heightMode: "expanded", context: "dem" }),
    { tab: "results", heightMode: "normal", context: "dem", heightPx: 300 },
  );
  for (const tab of ["results", "preview", "assets", "metadata"]) {
    assert.equal(parseDockPreferences({ ...DEFAULT_DOCK_PREFERENCES, tab }).tab, "results");
  }
  for (const heightMode of ["collapsed", "normal"] as const) {
    assert.equal(
      parseDockPreferences({ ...DEFAULT_DOCK_PREFERENCES, heightMode }).heightMode,
      heightMode,
    );
  }
  assert.deepEqual(
    parseDockPreferences({ tab: "download", heightMode: "huge", context: "queue" }),
    DEFAULT_DOCK_PREFERENCES,
  );
});

test("dock height preferences migrate safely and preserve map space", () => {
  assert.deepEqual(
    parseDockPreferences({ tab: "results", heightMode: "normal", context: "catalog" }),
    { tab: "results", heightMode: "normal", context: "catalog", heightPx: 300 },
  );
  assert.equal(
    parseDockPreferences({
      tab: "results",
      heightMode: "normal",
      context: "catalog",
      heightPx: 372.4,
    }).heightPx,
    372,
  );
  assert.equal(clampDockHeight(900, 620), 460);
  assert.equal(clampDockHeight(20, 620), 180);
});

test("virtual result reveal returns the exact row offset without mutating rows", () => {
  const rows = [
    { provider_product_key: "P1" },
    { provider_product_key: "P2" },
    { provider_product_key: "P3" },
  ];

  assert.equal(scrollTopForProduct(rows, "P3", 72), 144);
  assert.equal(scrollTopForProduct(rows, "missing", 72), null);
  assert.deepEqual(rows.map((row) => row.provider_product_key), ["P1", "P2", "P3"]);
});

test("virtual windows are bounded for empty and ten-thousand-row catalogues", () => {
  assert.deepEqual(virtualRowWindow(0, 200, 300), {
    start: 0,
    end: 0,
    offset: 0,
    totalHeight: 0,
  });
  assert.deepEqual(virtualRowWindow(10_000, 4_800, 240, 48, 2), {
    start: 98,
    end: 107,
    offset: 4_704,
    totalHeight: 480_000,
  });
  assert.deepEqual(virtualRowWindow(10, Number.NaN, -1, 0, -5), {
    start: 0,
    end: 1,
    offset: 0,
    totalHeight: 10,
  });
});

test("catalogue selection projection excludes keys from another workbench scope", () => {
  const selected = scopeSelectionKeys(
    ["ASF::SENTINEL-1::ONE"],
    ["ASF::SENTINEL-1::ONE", "ASF::SENTINEL-1::STALE-A", "ASF::ERS::STALE-B"],
  );

  assert.deepEqual([...selected], ["ASF::SENTINEL-1::ONE"]);
});

function demTask(overrides: Partial<DemTaskStatus> = {}): DemTaskStatus {
  return {
    ok: true,
    task_id: "dem-task-1",
    name: "COP30 test",
    state: "running",
    total: 4,
    done: 1,
    current_scene: "N30E110",
    output_dir: "E:/dem/output",
    dataset: "COP30",
    convert: true,
    raw_dem_path: "E:/dem/output/cop30.tif",
    ellipsoid_dem_path: "E:/dem/output/cop30_ellipsoid.tif",
    sarscape_ready_dem_path: "E:/dem/output/cop30_dem",
    results_path: "E:/dem/output/download-results.json",
    conversion_results_path: "E:/dem/output/conversion-results.json",
    done_bytes: 25,
    total_bytes: 100,
    bytes_per_second: 10,
    elapsed_seconds: 2.5,
    cancelled: false,
    error: null,
    summary_line: "Downloading tile N30E110",
    succeeded: 1,
    skipped: 0,
    failed: 0,
    interrupted: 0,
    has_failures: false,
    worker_alive: true,
    queue_position: 0,
    created_sequence: 1,
    max_concurrent_tasks: 10,
    max_concurrent_conversions: 2,
    resume_supported: true,
    task_snapshot: {
      task_id: "dem-task-1",
      provider: "COPERNICUS_AWS",
      dataset: "COP30",
      bbox: { west: 110, east: 111, south: 30, north: 31, crs: "EPSG:4326" },
      source_vertical_datum: "EGM2008",
      target_vertical_datum: "WGS84_ELLIPSOID",
      output_root: "E:/dem/output",
      convert_policy: "sarscape",
      source_policy: "public-cog",
      proxy_policy: "system",
      schema_version: 1,
      created_at: 1,
      resume_supported: true,
    },
    results: [],
    log: [],
    ...overrides,
  };
}

test("DEM dock projection derives request, honest progress, and local artifacts without mutating task truth", () => {
  const task = demTask();
  const before = JSON.stringify(task);

  const [projection] = projectDemTasksToDock([task]);

  assert.equal(JSON.stringify(task), before);
  assert.equal(projection.task_id, "dem-task-1");
  assert.equal(projection.provider, "COPERNICUS_AWS");
  assert.equal(projection.dataset, "COP30");
  assert.equal(projection.state, "running");
  assert.equal(projection.bbox_summary, "W110.000 S30.000 E111.000 N31.000");
  assert.deepEqual(projection.progress, {
    determinate: true,
    completed: 25,
    total: 100,
    percent: 25,
    unit: "bytes",
  });
  assert.equal(projection.remote_source_summary, "COPERNICUS_AWS · COP30 · public-cog");
  assert.deepEqual(projection.artifacts.map((item) => item.kind), [
    "source_dem",
    "ellipsoid_dem",
    "sarscape_ready_dem",
    "results_table",
    "conversion_results_table",
  ]);
});

test("DEM projection keeps unknown totals indeterminate and omits unreported artifacts", () => {
  const [projection] = projectDemTasksToDock([
    demTask({
      state: "queued",
      total: 0,
      done: 0,
      done_bytes: undefined,
      total_bytes: undefined,
      raw_dem_path: undefined,
      ellipsoid_dem_path: undefined,
      sarscape_ready_dem_path: undefined,
      results_path: undefined,
      conversion_results_path: undefined,
      summary_line: "Waiting for capacity",
    }),
  ]);

  assert.deepEqual(projection.progress, {
    determinate: false,
    completed: 0,
    total: null,
    percent: null,
    unit: "tasks",
  });
  assert.deepEqual(projection.artifacts, []);
  assert.equal(projection.current_stage, "Waiting for capacity");
});
