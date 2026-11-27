// Synthetic native adapter for the actual Workbench. No real Store, credentials or network.
import { createBrowserFixtureDesktopShellBridge } from "../lib/desktopShellBridgeFixture";
import { opticalTaskReceiptFixture } from "./optical-task-fixture";
import type { DesktopShellBridge } from "../lib/desktopShellBridge";

export function createDemTasksFixture() {
  const base = createBrowserFixtureDesktopShellBridge();
  const state = { limit: 2, reads: 0, settingsFail: false, controls: [] as string[], opened: [] as string[] };
  const filename = "COP30_EGM2008_20260930T120000_" + "long-original-product-".repeat(8) + "01234567.tif";
  const snapshot = opticalTaskReceiptFixture({ task_id: "dem-aws", output_root: "C:\\synthetic\\dem", created_at: "2026-09-30T12:00:00Z" });
  const template = (snapshot.core.tasks as Record<string, unknown>[])[0]!;
  snapshot.core.workbench = null;
  snapshot.core.tasks = ["COPERNICUS_AWS", "OPENTOPOGRAPHY"].map((provider, index) => ({
    ...template, task_id: index ? "dem-otg" : "dem-aws", batch_id: `dem-${index}`, dataset_id: "COP30",
    provider_id: provider, status: index ? "queued" : "running", revision: 3,
    selected_count: 1, checked_count: 1, active_count: index ? 0 : 1, queued_count: index ? 1 : 0,
    network_bytes_received: 1024, transferred_bytes: 1024, active_bytes: 1024,
    created_at: `2026-09-30T12:00:0${index}Z`,
    operation: { operation_id: `dem-${index}:dem_download`, kind: "dem", phase: "download",
      region_id: "standalone-dem", approved_worker_operation: "dem_download", source_provider: provider,
      input_path: `C:\\synthetic\\dem\\${filename}`, convert: false, source_vertical_datum: "EGM2008" },
  }));
  const bridge: DesktopShellBridge = { ...base, kind: "tauri", application: { ...base.application,
    coreRestore: async () => { state.reads++; return snapshot; },
    getDemTaskSettings: async () => state.settingsFail
      ? { ok: false, code: "DEM_SETTINGS_FIXTURE", error: "synthetic settings unavailable" }
      : { max_concurrent_tasks: state.limit, max_concurrent_conversions: 1 },
    saveDemTaskSettings: async settings => { state.limit = settings.max_concurrent_tasks; return settings; },
    openExternalUrl: async url => { state.opened.push(url); return { ok: true, url }; },
  } };
  return { bridge, state, snapshot, filename };
}
