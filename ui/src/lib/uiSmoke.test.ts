import assert from "node:assert/strict";
import test from "node:test";
import { configureUiSmoke, markUiSmoke, uiSmokeReportForTest } from "./uiSmoke.ts";

test("normal UI mode does not fabricate a smoke report", () => {
  configureUiSmoke(false);
  markUiSmoke("react_mounted", true);
  assert.equal(uiSmokeReportForTest().react_mounted, false);
});

function setupSmokeMarks(): void {
  markUiSmoke("react_mounted", true);
  markUiSmoke("workbench_rendered", true);
  markUiSmoke("application_service_ready", true);
  markUiSmoke("leaflet_initialized", true);
  markUiSmoke("map_width", 640);
  markUiSmoke("map_height", 480);
  markUiSmoke("map_resized", true);
  markUiSmoke("tile_error_handled", true);
  markUiSmoke("aoi_overlay_rendered", true);
  markUiSmoke("region_created_or_selected", true);
  markUiSmoke("aoi_bind_command_completed", true);
  markUiSmoke("aoi_persisted_in_snapshot", true);
  markUiSmoke("aoi_restored_after_restart", true);
  markUiSmoke("search_request_uses_persisted_aoi", true);
  markUiSmoke("workspace_open_completed", true);
  markUiSmoke("project_region_context_completed", true);
  markUiSmoke("per_region_aoi_completed", true);
  markUiSmoke("first_region_aoi_id", "region-a");
  markUiSmoke("second_region_aoi_id", "region-b");
  markUiSmoke("first_region_aoi_revision", 1);
  markUiSmoke("second_region_aoi_revision", 1);
  markUiSmoke("dem_plan_task_id", "dem-test");
  markUiSmoke("dem_task_control_completed", true);
  markUiSmoke("dem_task_control_revision", 2);
  markUiSmoke("dem_task_completed_id", "dem-test");
  markUiSmoke("dem_conversion_task_id", "dem-conversion-test");
  markUiSmoke("dem_conversion_completed_id", "dem-conversion-test");
  markUiSmoke("orbit_task_id", "orbit-test");
  markUiSmoke("orbit_completed_id", "orbit-test");
  markUiSmoke("gacos_plan_offline", true);
  markUiSmoke("gacos_dry_run_offline", true);
}

test("a complete browser report is submitted only after the map checks pass", async () => {
  const previousWindow = (globalThis as { window?: unknown }).window;
  const submitted: unknown[] = [];
  Object.defineProperty(globalThis, "window", {
    configurable: true,
    value: { setTimeout },
  });
  try {
    configureUiSmoke(true, async (report) => {
      submitted.push(report);
    });
    setupSmokeMarks();
    await new Promise((resolve) => setTimeout(resolve, 160));
    assert.equal(submitted.length, 1);
    assert.equal((submitted[0] as { csp_violations: number }).csp_violations, 0);
  } finally {
    configureUiSmoke(false);
    if (previousWindow === undefined) {
      delete (globalThis as { window?: unknown }).window;
    } else {
      Object.defineProperty(globalThis, "window", {
        configurable: true,
        value: previousWindow,
      });
    }
  }
});

test("region_created_or_selected is tracked and submitted", async () => {
  const previousWindow = (globalThis as { window?: unknown }).window;
  const submitted: unknown[] = [];
  Object.defineProperty(globalThis, "window", {
    configurable: true,
    value: { setTimeout },
  });
  try {
    configureUiSmoke(true, async (report) => {
      submitted.push(report);
    });
    setupSmokeMarks();
    await new Promise((resolve) => setTimeout(resolve, 160));
    assert.equal(submitted.length, 1);
    assert.equal(
      (submitted[0] as { region_created_or_selected: boolean }).region_created_or_selected,
      true,
    );
  } finally {
    configureUiSmoke(false);
    if (previousWindow === undefined) {
      delete (globalThis as { window?: unknown }).window;
    } else {
      Object.defineProperty(globalThis, "window", {
        configurable: true,
        value: previousWindow,
      });
    }
  }
});

test("AOI bind smoke fields are tracked through the full pipeline", async () => {
  const previousWindow = (globalThis as { window?: unknown }).window;
  const submitted: unknown[] = [];
  Object.defineProperty(globalThis, "window", {
    configurable: true,
    value: { setTimeout },
  });
  try {
    configureUiSmoke(true, async (report) => {
      submitted.push(report);
    });
    setupSmokeMarks();
    await new Promise((resolve) => setTimeout(resolve, 160));
    assert.equal(submitted.length, 1);
    const s = submitted[0] as Record<string, unknown>;
    assert.equal(s.aoi_bind_command_completed, true);
    assert.equal(s.aoi_persisted_in_snapshot, true);
    assert.equal(s.aoi_restored_after_restart, true);
    assert.equal(s.search_request_uses_persisted_aoi, true);
  } finally {
    configureUiSmoke(false);
    if (previousWindow === undefined) {
      delete (globalThis as { window?: unknown }).window;
    } else {
      Object.defineProperty(globalThis, "window", {
        configurable: true,
        value: previousWindow,
      });
    }
  }
});
