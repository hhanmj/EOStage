import assert from "node:assert/strict";
import test from "node:test";
import { MESSAGES } from "../i18n/locales.ts";

import {
  selectDesktopShellBridge,
  type DesktopCatalogSearchAndFreezeRequest,
  type DesktopShellBridge,
} from "./desktopShellBridge.ts";
import { createBrowserFixtureDesktopShellBridge } from "./desktopShellBridgeFixture.ts";
import {
  createTauriDesktopShellBridge,
  isAllowedTauriOrigin,
  type TauriBridgeDependencies,
} from "./desktopShellBridgeTauri.ts";
import {
  createPywebviewDesktopShellBridge,
  type PywebviewShellApi,
} from "./desktopShellBridgePywebview.ts";


// A ShellError message_key is projected to its zh-CN errors-namespace text.
const APPLICATION_UNAVAILABLE_TEXT = MESSAGES["zh-CN"].errors.messageKeys.application_service.unavailable;
function stubBridge(kind: DesktopShellBridge["kind"]): DesktopShellBridge {
  return {
    kind,
    ready: async () => kind !== "unavailable",
    application: {
      getReadiness: async () => ({
        ok: false,
        error: "stub",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      restore: async () => ({
        ok: false,
        error: "stub",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      coreRestore: async () => ({
        ok: false,
        error: "stub",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      workbenchPage: async () => ({
        ok: false,
        error: "stub",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      taskPage: async () => ({
        ok: false,
        error: "stub",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      taskSourceBatchPage: async () => ({
        ok: false,
        error: "stub",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      updateWorkbenchSelection: async () => ({
        ok: false,
        error: "stub",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      createTaskFromWorkbench: async () => ({
        ok: false,
        error: "stub",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      appendTaskFromWorkbench: async () => ({
        ok: false,
        error: "stub",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      appendTaskFromSourceBatch: async () => ({
        ok: false,
        error: "stub",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      createTaskFromHistoricalSourceBatch: async () => ({
        ok: false,
        error: "stub",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      workspacePrepareLocation: async () => ({
        ok: false,
        error: "stub",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      workspaceOpen: async () => ({
        ok: false,
        error: "stub",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      workspaceCreate: async () => ({
        ok: false,
        error: "stub",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      projectCreate: async () => ({
        ok: false,
        error: "stub",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      projectSelect: async () => ({
        ok: false,
        error: "stub",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      regionCreate: async () => ({
        ok: false,
        error: "stub",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      regionSelect: async () => ({
        ok: false,
        error: "stub",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      providerCapabilities: async () => ({
        ok: false,
        error: "stub",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      sarSearchAndFreeze: async () => ({
        ok: false,
        error: "stub",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      catalogSearchAndFreeze: async () => ({
        ok: false,
        error: "stub",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      cancelRequest: async () => ({
        ok: false,
        error: "stub",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      cdseSessionStatus: async () => ({
        ok: false,
        error: "stub",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      cdseLogin: async () => ({
        ok: false,
        error: "stub",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      cdseLogout: async () => ({
        ok: false,
        error: "stub",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      cdseNetworkPolicy: async () => ({
        ok: false,
        error: "stub",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      setCdseNetworkPolicy: async () => ({
        ok: false,
        error: "stub",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      cdseNetworkDiagnostic: async () => ({
        ok: false,
        error: "stub",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      previewAsset: async () => ({
        ok: false,
        error: "stub",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      cdseQuicklook: async () => ({
        ok: false,
        error: "stub",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      usgsSessionStatus: async () => ({
        ok: false,
        error: "stub",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      usgsLogin: async () => ({
        ok: false,
        error: "stub",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      usgsLogout: async () => ({
        ok: false,
        error: "stub",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      landsatBrowse: async () => ({
        ok: false,
        error: "stub",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      createLandsatBundleTask: async () => ({
        ok: false,
        error: "stub",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      createLandsatAssetTask: async () => ({
        ok: false,
        error: "stub",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      createSentinel2ArchiveTask: async () => ({
        ok: false,
        error: "stub",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      createSentinel2ArchiveBatchTask: async () => ({
        ok: false,
        error: "stub",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      createSentinel2BandTask: async () => ({
        ok: false,
        error: "stub",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      precheckOpticalAoiCoverage: async () => ({
        ok: false,
        error: "stub",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      createOpticalDerivedTask: async () => ({
        ok: false,
        error: "stub",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      updateSearchWorkbench: async () => ({
        ok: false,
        error: "stub",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      createDownloadTask: async () => ({
        ok: false,
        error: "stub",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      appendDownloadTask: async () => ({
        ok: false,
        error: "stub",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      updateDownloadTask: async () => ({
        ok: false,
        error: "stub",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      updateDownloadTaskView: async () => ({
        ok: false,
        error: "stub",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      archiveDownloadTask: async () => ({
        ok: false,
        error: "stub",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      deleteDownloadHistory: async () => ({
        ok: false,
        error: "stub",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
    },
    compatibility: {
      capabilities: async () => ({ ok: true }),
      callLegacy: async () => ({ ok: false, error: "stub", code: "COMPAT_WORKER_UNAVAILABLE" }),
    },
    app: {
      getInfo: async () => ({ name: "stub", version: "1", offline: true }),
    },
    window: {
      minimize: async () => ({ ok: true }),
      toggleMaximize: async () => ({ ok: true }),
      close: async () => ({ ok: true }),
      getSize: async () => ({ ok: true, width: 100, height: 100 }),
      resizeFromEdge: async () => ({ ok: true }),
    },
    dialog: {
      pickOpenFile: async () => ({ ok: true, path: "" }),
      pickDirectory: async () => ({ ok: true, path: "" }),
    },
  };
}

function catalogueFreezeRequest(): DesktopCatalogSearchAndFreezeRequest {
  return {
    request_id: "catalogue-freeze-request",
    search: {
      logical_dataset: "optical.sentinel-2.l2a",
      provider_id: "copernicus.dataspace.odata",
      native_collection_id: "SENTINEL-2",
      aoi_geojson: {
        type: "Polygon",
        coordinates: [[[110, 30], [111, 30], [111, 31], [110, 30]]],
      },
      cursor: null,
      page_size: 24,
      filters: {
        product_type: "S2MSI2A",
        cloud_cover_lte: 4.2,
        datetime: {
          start: "2026-01-01T00:00:00.120Z",
          end: "2026-01-02T00:00:00.120Z",
        },
      },
    },
    batch_id: "catalogue-freeze-batch",
    created_at: "2026-07-26T00:00:00.120Z",
    aoi_name: "offline catalogue fixture",
    aoi_geojson: {
      type: "Polygon",
      coordinates: [[[110, 30], [111, 30], [111, 31], [110, 30]]],
    },
    workbench_page_size: 24,
  };
}

test("desktop shell selection order is deterministic and test injection is test-only", () => {
  const testAdapter = stubBridge("unavailable");
  const tauriAdapter = stubBridge("tauri");
  const fixture = stubBridge("browser_fixture");

  assert.equal(
    selectDesktopShellBridge({
      testMode: true,
      testAdapter,
      tauriAdapter,
      tauriOriginValidated: true,
      browserFixtureAllowed: true,
      browserFixture: fixture,
    }),
    testAdapter,
  );
  assert.equal(
    selectDesktopShellBridge({
      testMode: false,
      testAdapter,
      tauriAdapter,
      tauriOriginValidated: true,
      browserFixtureAllowed: true,
      browserFixture: fixture,
    }),
    tauriAdapter,
  );
  assert.equal(
    selectDesktopShellBridge({
      tauriAdapter,
      tauriOriginValidated: false,
      browserFixtureAllowed: true,
      browserFixture: fixture,
    }),
    fixture,
  );
  assert.equal(
    selectDesktopShellBridge({
      browserFixtureAllowed: true,
      browserFixture: fixture,
    }),
    fixture,
  );
  assert.equal(selectDesktopShellBridge({}).kind, "unavailable");
});

test("pywebview adapter preserves existing Python method names and arguments", async () => {
  const calls: Array<[string, ...unknown[]]> = [];
  const api: PywebviewShellApi = {
    get_app_info: async () => {
      calls.push(["get_app_info"]);
      return { name: "InSAR Studio", version: "2.2.0", offline: true };
    },
    pick_open_file: async (title, filters) => {
      calls.push(["pick_open_file", title, filters]);
      return { ok: true, path: "D:/selected/file.txt" };
    },
    pick_directory: async (title) => {
      calls.push(["pick_directory", title]);
      return { ok: true, path: "D:/selected" };
    },
    window_minimize: async () => {
      calls.push(["window_minimize"]);
      return { ok: true };
    },
    window_toggle_maximize: async (workArea) => {
      calls.push(["window_toggle_maximize", workArea]);
      return { ok: true };
    },
    window_close: async () => {
      calls.push(["window_close"]);
      return { ok: true };
    },
    window_get_size: async () => {
      calls.push(["window_get_size"]);
      return { ok: true, width: 1320, height: 880 };
    },
    window_resize_from_edge: async (...args) => {
      calls.push(["window_resize_from_edge", ...args]);
      return { ok: true };
    },
  };
  const workArea = { x: 0, y: 0, width: 1920, height: 1039 };
  const bridge = createPywebviewDesktopShellBridge(api, {
    getWorkArea: () => workArea,
  });

  await bridge.app.getInfo();
  await bridge.dialog.pickOpenFile("Open", ["Text (*.txt)"]);
  await bridge.dialog.pickDirectory("Directory");
  await bridge.window.minimize();
  await bridge.window.toggleMaximize();
  await bridge.window.close();
  await bridge.window.getSize();
  await bridge.window.resizeFromEdge("right", 1000, 700, 25, 0);
  assert.deepEqual(await bridge.application.catalogSearchAndFreeze(catalogueFreezeRequest()), {
    ok: false,
    error: "pywebview application service route is not enabled",
    code: "V3_APPLICATION_UNAVAILABLE",
  });
  assert.deepEqual(await bridge.application.cdseSessionStatus(), {
    ok: false,
    error: "pywebview application service route is not enabled",
    code: "V3_APPLICATION_UNAVAILABLE",
  });
  assert.deepEqual(await bridge.application.cdseLogin({
    username: "fixture-user",
    password: "fixture-password",
  }), {
    ok: false,
    error: "pywebview application service route is not enabled",
    code: "V3_APPLICATION_UNAVAILABLE",
  });
  assert.deepEqual(await bridge.application.cdseQuicklook({
    request_id: "quicklook-request",
    batch_id: "quicklook-batch",
    item_id: "quicklook-item",
  }), {
    ok: false,
    error: "pywebview application service route is not enabled",
    code: "V3_APPLICATION_UNAVAILABLE",
  });

  assert.deepEqual(calls, [
    ["get_app_info"],
    ["pick_open_file", "Open", ["Text (*.txt)"]],
    ["pick_directory", "Directory"],
    ["window_minimize"],
    ["window_toggle_maximize", workArea],
    ["window_close"],
    ["window_get_size"],
    ["window_resize_from_edge", "right", 1000, 700, 25, 0],
  ]);
});

test("Browser Fixture is deterministic, cancellable, and shell-only", async () => {
  let closed = 0;
  const bridge = createBrowserFixtureDesktopShellBridge({
    close: () => {
      closed += 1;
    },
    getSize: () => ({ width: 1440, height: 900 }),
  });

  assert.equal(bridge.kind, "browser_fixture");
  assert.equal(await bridge.ready(), true);
  assert.deepEqual(await bridge.app.getInfo(), {
    name: "EOStage",
    version: "2.2.0-beta.0",
    offline: true,
  });
  assert.deepEqual(await bridge.dialog.pickOpenFile("Open", ["Any (*.*)"]), {
    ok: true,
    path: "",
  });
  assert.deepEqual(await bridge.dialog.pickDirectory("Directory"), {
    ok: true,
    path: "",
  });
  assert.deepEqual(await bridge.window.getSize(), {
    ok: true,
    width: 1440,
    height: 900,
  });
  assert.deepEqual(await bridge.application.catalogSearchAndFreeze(catalogueFreezeRequest()), {
    ok: false,
    error: "Browser Fixture has no application Store",
    code: "V3_APPLICATION_UNAVAILABLE",
  });
  assert.deepEqual(await bridge.application.cdseNetworkDiagnostic("system"), {
    ok: false,
    error: "Browser Fixture has no application Store",
    code: "V3_APPLICATION_UNAVAILABLE",
  });
  assert.deepEqual(await bridge.application.cdseQuicklook({
    request_id: "quicklook-request",
    batch_id: "quicklook-batch",
    item_id: "quicklook-item",
  }), {
    ok: false,
    error: "Browser Fixture has no application Store",
    code: "V3_APPLICATION_UNAVAILABLE",
  });
  await bridge.window.close();
  assert.equal(closed, 1);
  assert.deepEqual(Object.keys(bridge).sort(), [
    "app",
    "application",
    "compatibility",
    "dialog",
    "kind",
    "ready",
    "window",
  ]);
});

test("unvalidated Tauri registration and production unknown origin fail closed", async () => {
  const selected = selectDesktopShellBridge({
    tauriAdapter: stubBridge("tauri"),
    tauriOriginValidated: false,
    browserFixtureAllowed: false,
  });

  assert.equal(selected.kind, "unavailable");
  assert.equal(await selected.ready(), false);
  assert.deepEqual(await selected.dialog.pickDirectory("Directory"), {
    ok: false,
    path: "",
    error: "Desktop shell bridge is unavailable",
    code: "GUI003",
  });
  assert.deepEqual(await selected.application.catalogSearchAndFreeze(catalogueFreezeRequest()), {
    ok: false,
    error: "Desktop application service is unavailable",
    code: "V3_APPLICATION_UNAVAILABLE",
  });
});

test("Tauri origin policy accepts only exact packaged and development origins", () => {
  for (const origin of [
    "tauri://localhost/",
    "http://tauri.localhost/",
    "https://tauri.localhost/",
  ]) {
    assert.equal(isAllowedTauriOrigin(origin, false), true, origin);
  }
  assert.equal(isAllowedTauriOrigin("http://127.0.0.1:5173/", true), true);

  for (const [origin, development] of [
    ["tauri://localhost:443/", false],
    ["http://tauri.localhost:443/", false],
    ["https://tauri.localhost.evil.example/", false],
    ["http://localhost:5173/", true],
    ["http://127.0.0.1:5174/", true],
    ["https://127.0.0.1:5173/", true],
    ["https://example.com/", true],
  ] as const) {
    assert.equal(isAllowedTauriOrigin(origin, development), false, origin);
  }
});

test("Tauri adapter preserves Unicode paths and translates reviewed filters", async () => {
  const calls: string[] = [];
  const opened: Array<Record<string, unknown>> = [];
  const window = {
    minimize: async () => {
      calls.push("minimize");
    },
    toggleMaximize: async () => {
      calls.push("toggleMaximize");
    },
    close: async () => {
      calls.push("close");
    },
    innerSize: async () => ({ width: 1320, height: 880 }),
    startResizeDragging: async (direction: string) => {
      calls.push(`resize:${direction}`);
    },
  };
  const dependencies: TauriBridgeDependencies = {
    invoke: async (command) => {
      calls.push(`invoke:${command}`);
      return { name: "InSAR Studio", version: "2.2.0", shell_kind: "tauri_single_desktop_host" };
    },
    getCurrentWindow: () => window,
    open: async (options) => {
      opened.push(options);
      return "E:/测试资料/长路径/影像.tar.gz";
    },
  };
  const bridge = createTauriDesktopShellBridge(
    dependencies,
    "http://127.0.0.1:5173/",
    true,
  );

  assert.equal(bridge.kind, "tauri");
  assert.deepEqual(await bridge.app.getInfo(), {
    name: "InSAR Studio",
    version: "2.2.0",
    offline: true,
  });
  assert.deepEqual(await bridge.dialog.pickOpenFile("打开影像", [
    "SAR (*.zip;*.SAFE)",
    "All files (*.*)",
  ]), {
    ok: true,
    path: "E:/测试资料/长路径/影像.tar.gz",
  });
  assert.deepEqual(opened, [{
    title: "打开影像",
    multiple: false,
    directory: false,
    filters: [
      { name: "SAR", extensions: ["zip", "SAFE"] },
      { name: "All files", extensions: ["*"] },
    ],
  }]);
  assert.deepEqual(await bridge.window.resizeFromEdge("right", 1, 1, 1, 1), { ok: true });
  assert.deepEqual(await bridge.window.getSize(), { ok: true, width: 1320, height: 880 });
  assert.deepEqual(calls, ["invoke:shell_app_info", "resize:East"]);
});

test("Tauri application adapter invokes only named typed application commands", async () => {
  const commands: Array<[string, Record<string, unknown> | undefined]> = [];
  const nativeAoiDisplay = {
    region_id: "region-1",
    revision: 3,
    envelope: {
      south: -47,
      north: -34,
      crosses_antimeridian: true,
      display_west: 166,
      display_east: 185,
      request_windows: [
        { west: 166, south: -47, east: 180, north: -34, crs: "EPSG:4326" },
        { west: -180, south: -47, east: -175, north: -34, crs: "EPSG:4326" },
      ],
    },
  };
  const dependencies: TauriBridgeDependencies = {
    invoke: async (command, args) => {
      commands.push([command, args]);
      if (command === "application_service_readiness") {
        return {
          contract: "application_service_readonly_v1",
          status: "store_ready",
          store_schema_version: 4,
        };
      }
      if (command === "application_provider_capabilities") {
        return {
          provider_id: "asf.catalog",
          available: true,
          reason: null,
          datasets: [],
          source_version: "fixture",
        };
      }
      if (
        [
          "application_cdse_session_status",
          "application_cdse_login",
          "application_cdse_logout",
          "application_set_cdse_network_policy",
        ].includes(command)
      ) {
        return {
          network_mode: command === "application_set_cdse_network_policy" ? "direct" : "system",
          authenticated: command === "application_cdse_login",
          refresh_available: command === "application_cdse_login",
          expires_in_seconds: command === "application_cdse_login" ? 60 : null,
          access_token: "must-not-cross-the-typed-response-boundary",
        };
      }
      if (command === "application_cdse_network_policy") return "system";
      if (command === "application_cdse_network_diagnostic") {
        return {
          network_mode: "direct",
          stage: "http_reached",
          authorization: "must-not-cross-the-typed-response-boundary",
        };
      }
      if (command === "application_preview_asset") {
        return {
          product_key: "preview-product",
          descriptor: {
            preview_kind: "quicklook",
            source_asset_keys: ["preview-key"],
            render_profile_id: "asf.sentinel-1.grd-browse-geographic.v1",
            media_type: "image/jpeg",
            credential_requirement: "none",
            cache_policy: "session",
            provenance: "ASF official Browse",
          },
          source_asset_role: "browse",
          source_asset_key: "preview-key",
          media_type: "image/jpeg",
          data_url: "data:image/jpeg;base64,AQID",
          byte_length: 3,
          href: "https://untrusted.example/preview.jpg",
          authorization: "must-not-cross-the-typed-response-boundary",
        };
      }
      if (command === "application_cdse_quicklook") {
        return {
          media_type: "image/png",
          data_url: "data:image/png;base64,AQID",
          byte_length: 3,
          href: "https://catalogue.dataspace.copernicus.eu/odata/v1/Assets(untrusted)/$value",
          access_token: "must-not-cross-the-typed-response-boundary",
        };
      }
      if (
        [
          "application_usgs_session_status",
          "application_usgs_login",
          "application_usgs_logout",
        ].includes(command)
      ) {
        const state = command === "application_usgs_login"
          ? "authenticated"
          : command === "application_usgs_logout"
            ? "logged_out"
            : "ready";
        return {
          state,
          authenticated: state === "authenticated",
          runtime_api_key: "must-not-cross-the-typed-response-boundary",
        };
      }
      if (command === "application_landsat_browse") {
        return {
          media_type: "image/webp",
          data_url: "data:image/webp;base64,AQID",
          byte_length: 3,
          href: "https://landsatlook.usgs.gov/untrusted-runtime-locator",
          authorization: "must-not-cross-the-typed-response-boundary",
        };
      }
      if (command === "application_landsat_asset_details") {
        return {
          product_key: "landsat-item",
          display_id: "LC09_L2SP_123045_20260101_20260103_02_T1",
          processing_level: "l2sp",
          total_size_bytes: 30,
          assets: [
            { asset_key: "red", file_name: "red.tif", role: "primary", size_bytes: 20 },
            { asset_key: "mtl.xml", file_name: "mtl.xml", role: "metadata", size_bytes: 10 },
          ],
        };
      }
      if (command === "application_nisar_remote_subset_capability") {
        return {
          feature_id: "nisar_remote_subset_v1",
          visible: true,
          enabled: false,
          implementation_scope: "offline_only",
          offline_verified: true,
          live_verified: false,
          production_enabled: false,
          default_enabled: false,
          output_authority: "client_derived",
          supported_product_type: "RSLC",
          supported_specification_versions: ["1.5.0"],
          supported_frequencies: ["A", "B"],
          supported_polarizations: { A: ["HH", "HV"], B: ["HH", "HV", "VV"] },
          reason_code: "NISAR_SUBSET_NOT_ENABLED",
        };
      }
      if (command === "application_prepare_nisar_subset_intent_v2") {
        return {
          source_batch_id: "nisar-batch",
          source_item_id: "nisar-item",
          workbench_revision: 7,
          intent: {
            selected_frequency: "A",
            selected_polarizations: ["HH", "HV"],
          },
        };
      }
      if (command === "application_effective_capability") {
        return {
          resource_id: "science.nisar.l.rslc.provisional.v1",
          capability: {
            capability_id: "capability.nisar-rslc.remote-subset.v1",
            kind: "remote_subset",
            lifecycle: "admission_controlled",
          },
          inputs: {
            build: "available",
            runtime: "ready",
            credential: "missing",
            admission: "accepted",
            policy: "allowed",
          },
          projection: {
            state: "blocked_credential",
            reason: "credential_missing_or_unknown",
          },
        };
      }
      if (command === "application_cancel_request") return true;
      if (command === "application_core_restore") {
        return {
          readiness: {
            contract: "application_service_readonly_v1",
            status: "store_ready",
            store_schema_version: 4,
          },
          core: {},
          aoi_display: args?.includeTerminal === false ? [] : nativeAoiDisplay,
        };
      }
      if (
        [
          "application_sar_search_and_freeze",
          "application_catalog_search_and_freeze",
          "application_create_landsat_bundle_task",
          "application_create_landsat_asset_task",
          "application_create_nisar_remote_subset_task",
          "application_prepare_nisar_subset_intent_v2",
          "application_confirm_nisar_subset_task_v2",
          "application_update_search_workbench",
          "application_create_download_task",
          "application_append_download_task",
          "application_update_download_task",
          "application_update_download_task_view",
          "application_archive_download_task",
          "application_delete_download_history",
        ].includes(command)
      ) {
        return {
          readiness: {
            contract: "application_service_readonly_v1",
            status: "store_ready",
            store_schema_version: 4,
          },
          core: {},
        };
      }
      return {};
    },
    getCurrentWindow: () => ({
      minimize: async () => undefined,
      toggleMaximize: async () => undefined,
      close: async () => undefined,
      innerSize: async () => ({ width: 100, height: 100 }),
      startResizeDragging: async () => undefined,
    }),
    open: async () => null,
  };
  const bridge = createTauriDesktopShellBridge(dependencies, "tauri://localhost/", false);

  assert.deepEqual(await bridge.application.getReadiness(), {
    ok: true,
    contract: "application_service_readonly_v1",
    status: "store_ready",
    store_schema_version: 4,
  });
  assert.deepEqual(await bridge.application.coreRestore(true), {
    ok: true,
    readiness: {
      contract: "application_service_readonly_v1",
      status: "store_ready",
      store_schema_version: 4,
    },
    core: {},
    aoi_display: nativeAoiDisplay,
  });
  assert.deepEqual(await bridge.application.providerCapabilities(), {
    ok: true,
    capabilities: {
      provider_id: "asf.catalog",
      available: true,
      reason: null,
      datasets: [],
      source_version: "fixture",
    },
  });
  const search = {
    request_id: "search-1",
    search: { query: { provider: "asf", dataset: "SENTINEL-1" } },
    batch_id: "batch-1",
    created_at: "2026-07-25T00:00:00Z",
    aoi_name: "fixture",
    aoi_geojson: { type: "Polygon", coordinates: [] },
    page_size: 24,
  };
  assert.equal((await bridge.application.sarSearchAndFreeze(search)).ok, true);
  const catalogue = catalogueFreezeRequest();
  assert.equal((await bridge.application.catalogSearchAndFreeze(catalogue)).ok, true);
  assert.deepEqual(await bridge.application.cancelRequest("search-1"), {
    ok: true,
    cancelled: true,
  });
  const previewRequest = {
    request_id: "preview-request",
    batch_id: "preview-batch",
    item_id: "preview-item",
  };
  const previewResult = await bridge.application.previewAsset(previewRequest);
  assert.deepEqual(previewResult, {
    ok: true,
    preview: {
      product_key: "preview-product",
      descriptor: {
        preview_kind: "quicklook",
        source_asset_keys: ["preview-key"],
        render_profile_id: "asf.sentinel-1.grd-browse-geographic.v1",
        media_type: "image/jpeg",
        credential_requirement: "none",
        cache_policy: "session",
        provenance: "ASF official Browse",
      },
      source_asset_role: "browse",
      source_asset_key: "preview-key",
      media_type: "image/jpeg",
      data_url: "data:image/jpeg;base64,AQID",
      byte_length: 3,
    },
  });
  assert(!JSON.stringify(previewResult).includes("untrusted.example"));
  assert(!JSON.stringify(previewResult).includes("authorization"));
  const login = {
    username: "fixture-user",
    password: "fixture-password",
    totp: "123456",
  };
  assert.deepEqual(await bridge.application.cdseSessionStatus(), {
    ok: true,
    status: {
      network_mode: "system",
      authenticated: false,
      refresh_available: false,
      expires_in_seconds: null,
    },
  });
  assert.deepEqual(await bridge.application.cdseLogin(login), {
    ok: true,
    status: {
      network_mode: "system",
      authenticated: true,
      refresh_available: true,
      expires_in_seconds: 60,
    },
  });
  assert.deepEqual(await bridge.application.cdseLogout(), {
    ok: true,
    status: {
      network_mode: "system",
      authenticated: false,
      refresh_available: false,
      expires_in_seconds: null,
    },
  });
  assert.deepEqual(await bridge.application.cdseNetworkPolicy(), {
    ok: true,
    network_mode: "system",
  });
  assert.deepEqual(await bridge.application.setCdseNetworkPolicy("direct"), {
    ok: true,
    status: {
      network_mode: "direct",
      authenticated: false,
      refresh_available: false,
      expires_in_seconds: null,
    },
  });
  assert.deepEqual(await bridge.application.cdseNetworkDiagnostic("direct"), {
    ok: true,
    diagnostic: {
      network_mode: "direct",
      stage: "http_reached",
    },
  });
  const quicklook = {
    request_id: "quicklook-request",
    batch_id: "quicklook-batch",
    item_id: "quicklook-item",
  };
  assert.deepEqual(await bridge.application.cdseQuicklook(quicklook), {
    ok: true,
    quicklook: {
      media_type: "image/png",
      data_url: "data:image/png;base64,AQID",
      byte_length: 3,
    },
  });
  const usgsLogin = {
    username: "fixture-user",
    application_token: "fixture-token-placeholder",
  };
  assert.deepEqual(await bridge.application.usgsSessionStatus(), {
    ok: true,
    status: { state: "ready", authenticated: false },
  });
  const usgsLoginResult = await bridge.application.usgsLogin(usgsLogin);
  assert.deepEqual(usgsLoginResult, {
    ok: true,
    status: { state: "authenticated", authenticated: true },
  });
  assert(!JSON.stringify(usgsLoginResult).includes("runtime_api_key"));
  assert.deepEqual(await bridge.application.usgsLogout(), {
    ok: true,
    status: { state: "logged_out", authenticated: false },
  });
  const landsatBrowse = {
    request_id: "landsat-browse-request",
    batch_id: "landsat-batch",
    item_id: "landsat-item",
  };
  const landsatBrowseResult = await bridge.application.landsatBrowse(landsatBrowse);
  assert.deepEqual(landsatBrowseResult, {
    ok: true,
    browse: {
      media_type: "image/webp",
      data_url: "data:image/webp;base64,AQID",
      byte_length: 3,
    },
  });
  assert(!JSON.stringify(landsatBrowseResult).includes("landsatlook.usgs.gov"));
  const landsatAssetDetails = {
    request_id: "landsat-asset-details-request",
    batch_id: "landsat-batch",
    item_id: "landsat-item",
  };
  assert.deepEqual(await bridge.application.landsatAssetDetails(landsatAssetDetails), {
    ok: true,
    details: {
      product_key: "landsat-item",
      display_id: "LC09_L2SP_123045_20260101_20260103_02_T1",
      processing_level: "l2sp",
      total_size_bytes: 30,
      assets: [
        { asset_key: "red", file_name: "red.tif", role: "primary", size_bytes: 20 },
        { asset_key: "mtl.xml", file_name: "mtl.xml", role: "metadata", size_bytes: 10 },
      ],
    },
  });
  const landsatBundleTask = {
    request_id: "landsat-bundle-request",
    source_batch_id: "landsat-batch",
    item_id: "landsat-item",
    task_id: "landsat-task",
    output_root: "D:/fixture",
    layout: "direct" as const,
    concurrency: 1,
    created_at: "2026-07-25T00:00:01Z",
  };
  assert.equal((await bridge.application.createLandsatBundleTask(landsatBundleTask)).ok, true);
  assert.equal((await bridge.application.createLandsatAssetTask({
    ...landsatBundleTask,
    asset_keys: ["red", "nir08"],
  })).ok, true);
  const nisarSubsetTask = {
    request_id: "nisar-subset-request",
    source_batch_id: "nisar-batch",
    item_id: "nisar-item",
    task_id: "nisar-subset-task",
    output_root: "D:/fixture",
    layout: "direct" as const,
    concurrency: 2,
    frequency: "A" as const,
    polarization: "HH",
    created_at: "2026-08-01T00:00:00Z",
  };
  const subsetCapability = await bridge.application.nisarRemoteSubsetCapability();
  assert.equal(subsetCapability.ok, true);
  assert.equal(subsetCapability.ok && subsetCapability.capability.visible, true);
  const effectiveCapability = await bridge.application.effectiveCapability(
    "capability.nisar-rslc.remote-subset.v1",
  );
  assert.equal(effectiveCapability.ok, true);
  assert.equal(
    effectiveCapability.ok && effectiveCapability.status.projection.state,
    "blocked_credential",
  );
  assert.equal((await bridge.application.createNisarRemoteSubsetTask(nisarSubsetTask)).ok, true);
  const nisarPrepare = {
    source_batch_id: "nisar-batch",
    source_item_id: "nisar-item",
    expected_workbench_revision: 7,
    subset_draft_geometry: { type: "Polygon", coordinates: [] },
    selected_frequency: "A" as const,
    selected_polarizations: ["HH", "HV"],
  };
  const prepared = await bridge.application.prepareNisarSubsetIntentV2(nisarPrepare);
  assert.equal(prepared.ok, true);
  assert.equal(prepared.ok && prepared.prepared.intent.selected_frequency, "A");
  const nisarConfirm = {
    source_batch_id: "nisar-batch",
    source_item_id: "nisar-item",
    expected_workbench_revision: 7,
    prepared_intent: prepared.ok ? prepared.prepared.intent : {},
    task_id: "nisar-v2-task",
    output_root: "D:/fixture",
    layout: "direct" as const,
    concurrency: 1,
    created_at: "2026-08-01T00:00:00Z",
  };
  assert.equal((await bridge.application.confirmNisarSubsetTaskV2(nisarConfirm)).ok, true);
  const workbench = {
    state: { batch_id: "batch-1", checked_item_ids: [], page_index: 0, page_size: 24, revision: 1 },
    expected_revision: 0,
    updated_at: "2026-07-25T00:00:01Z",
  };
  assert.equal((await bridge.application.updateSearchWorkbench(workbench)).ok, true);
  const task = {
    task_id: "task-1",
    batch_id: "batch-1",
    selected_item_ids: ["item-1"],
    checked_item_ids: [],
    page_index: 0,
    page_size: 24,
    output_root: "D:/fixture",
    layout: "direct" as const,
    concurrency: 1,
    created_at: "2026-07-25T00:00:02Z",
  };
  assert.equal((await bridge.application.createDownloadTask(task)).ok, true);
  const update = {
    task_id: "task-1",
    expected_revision: 0,
    command: { kind: "pause_task" as const, item_ids: [] },
    event_id: "event-1",
    occurred_at: "2026-07-25T00:00:03Z",
  };
  assert.equal((await bridge.application.updateDownloadTask(update)).ok, true);
  assert.deepEqual(commands, [
    ["application_service_readiness", undefined],
    ["application_core_restore", { includeTerminal: true }],
    ["application_provider_capabilities", undefined],
    ["application_sar_search_and_freeze", { request: search }],
    ["application_catalog_search_and_freeze", { request: catalogue }],
    ["application_cancel_request", { requestId: "search-1" }],
    ["application_preview_asset", { request: previewRequest }],
    ["application_cdse_session_status", undefined],
    ["application_cdse_login", { request: login }],
    ["application_cdse_logout", undefined],
    ["application_cdse_network_policy", undefined],
    ["application_set_cdse_network_policy", { mode: "direct" }],
    ["application_cdse_network_diagnostic", { mode: "direct" }],
    ["application_cdse_quicklook", { request: quicklook }],
    ["application_usgs_session_status", undefined],
    ["application_usgs_login", { request: usgsLogin }],
    ["application_usgs_logout", undefined],
    ["application_landsat_browse", { request: landsatBrowse }],
    ["application_landsat_asset_details", { request: landsatAssetDetails }],
    ["application_create_landsat_bundle_task", { request: landsatBundleTask }],
    ["application_create_landsat_asset_task", { request: { ...landsatBundleTask, asset_keys: ["red", "nir08"] } }],
    ["application_nisar_remote_subset_capability", undefined],
    ["application_effective_capability", { capabilityId: "capability.nisar-rslc.remote-subset.v1" }],
    ["application_create_nisar_remote_subset_task", { request: nisarSubsetTask }],
    ["application_prepare_nisar_subset_intent_v2", { request: nisarPrepare }],
    ["application_confirm_nisar_subset_task_v2", { request: nisarConfirm }],
    ["application_update_search_workbench", { request: workbench }],
    ["application_create_download_task", { request: task }],
    ["application_update_download_task", { request: update }],
  ]);
  assert.equal(commands[4]?.[1]?.request, catalogue);
  assert.equal(commands[6]?.[1]?.request, previewRequest);
  assert.equal(commands[8]?.[1]?.request, login);
  assert.equal(commands[13]?.[1]?.request, quicklook);
  assert.equal(commands[15]?.[1]?.request, usgsLogin);
  assert.equal(commands[17]?.[1]?.request, landsatBrowse);
  assert.equal(commands[18]?.[1]?.request, landsatAssetDetails);
  assert.equal(commands[19]?.[1]?.request, landsatBundleTask);
  assert.deepEqual(commands[20]?.[1]?.request, { ...landsatBundleTask, asset_keys: ["red", "nir08"] });
  assert.equal(commands[23]?.[1]?.request, nisarSubsetTask);
  assert.equal(commands[24]?.[1]?.request, nisarPrepare);
  assert.equal(commands[25]?.[1]?.request, nisarConfirm);
  assert.equal((await bridge.application.coreRestore(false)).ok, false);
});

test("Tauri reviewed MPC asset task uses the exact command and preserves the request", async () => {
  const calls: Array<[string, Record<string, unknown> | undefined]> = [];
  const dependencies: TauriBridgeDependencies = {
    invoke: async (command, args) => {
      calls.push([command, args]);
      if (command === "application_create_reviewed_mpc_asset_task") {
        return {
          readiness: {
            contract: "application_service_readonly_v1",
            status: "store_ready",
            store_schema_version: 4,
          },
          core: {},
        };
      }
      return {};
    },
    getCurrentWindow: () => ({
      minimize: async () => undefined,
      toggleMaximize: async () => undefined,
      close: async () => undefined,
      innerSize: async () => ({ width: 1, height: 1 }),
      startResizeDragging: async () => undefined,
    }),
    open: async () => null,
  };
  const bridge = createTauriDesktopShellBridge(dependencies, "tauri://localhost/", false);
  const request = {
    request_id: "reviewed-mpc-request",
    source_batch_id: "reviewed-mpc-batch",
    item_id: "reviewed-mpc-item",
    asset_keys: ["vv", "vh"],
    task_id: "reviewed-mpc-task",
    output_root: "D:/fixture",
    layout: "direct" as const,
    concurrency: 2,
    created_at: "2026-09-20T00:00:01Z",
  };

  assert.equal((await bridge.application.createReviewedMpcAssetTask(request)).ok, true);
  assert.deepEqual(calls, [[
    "application_create_reviewed_mpc_asset_task",
    { request },
  ]]);
});

test("Tauri Sentinel-2 archive batch preserves admission truth and rejects malformed summaries", async () => {
  const calls: Array<[string, Record<string, unknown> | undefined]> = [];
  let responseMode: "valid" | "count_mismatch" | "extra_field" = "valid";
  const dependencies: TauriBridgeDependencies = {
    invoke: async (command, args) => {
      calls.push([command, args]);
      if (command !== "application_create_sentinel2_archive_batch_task") return {};
      const rejection = responseMode === "extra_field"
        ? {
            source_item_id: "source-2",
            reason_code: "SENTINEL2_ARCHIVE_LOOKUP_FAILED",
            authorization: "must-not-cross",
          }
        : {
            source_item_id: "source-2",
            reason_code: "SENTINEL2_ARCHIVE_LOOKUP_FAILED",
          };
      return {
        snapshot: {
          readiness: {
            contract: "application_service_readonly_v1",
            status: "store_ready",
            store_schema_version: 4,
          },
          core: { tasks: [{ task_id: "sentinel2-archive-batch-task",
            batch_id: "sentinel2-archive-batch-sentinel2-archive-batch-task",
            provider_id: "copernicus.dataspace.odata", dataset_id: "optical.sentinel-2.l2a",
            selected_count: 1, delivery_requested_count: 2, delivery_rejected_count: 1 }] },
        },
        admission: {
          requested_count: 2,
          admitted_count: 1,
          rejected_count: responseMode === "count_mismatch" ? 0 : 1,
          rejections: [rejection],
        },
      };
    },
    getCurrentWindow: () => ({
      minimize: async () => undefined,
      toggleMaximize: async () => undefined,
      close: async () => undefined,
      innerSize: async () => ({ width: 1, height: 1 }),
      startResizeDragging: async () => undefined,
    }),
    open: async () => null,
  };
  const bridge = createTauriDesktopShellBridge(dependencies, "tauri://localhost/", false);
  const request = {
    request_id: "sentinel2-archive-batch-request",
    source_batch_id: "sentinel2-source-batch",
    expected_workbench_revision: 7,
    item_ids: ["source-1", "source-2"],
    task_id: "sentinel2-archive-batch-task",
    output_root: "D:/fixture",
    layout: "direct" as const,
    concurrency: 2,
    created_at: "2026-09-20T00:00:02Z",
  };

  const valid = await bridge.application.createSentinel2ArchiveBatchTask(request);
  assert.equal(valid.ok, true);
  if (!valid.ok) assert.fail("valid admission summary should pass");
  assert.deepEqual(valid.admission, {
    requested_count: 2,
    admitted_count: 1,
    rejected_count: 1,
    rejections: [{
      source_item_id: "source-2",
      reason_code: "SENTINEL2_ARCHIVE_LOOKUP_FAILED",
    }],
  });
  assert.deepEqual(calls[0], [
    "application_create_sentinel2_archive_batch_task",
    { request },
  ]);

  const storeOwnedRequest = { ...request, item_ids: [] as string[] };
  const storeOwned = await bridge.application.createSentinel2ArchiveBatchTask(storeOwnedRequest);
  assert.equal(storeOwned.ok, true, "empty IDs must preserve the Store-owned checked selection");
  assert.deepEqual(calls[1], [
    "application_create_sentinel2_archive_batch_task",
    { request: storeOwnedRequest },
  ]);

  responseMode = "count_mismatch";
  assert.equal((await bridge.application.createSentinel2ArchiveBatchTask(request)).ok, false);

  responseMode = "extra_field";
  const extraField = await bridge.application.createSentinel2ArchiveBatchTask(request);
  assert.equal(extraField.ok, false);
  assert(!JSON.stringify(extraField).includes("must-not-cross"));
});

test("Tauri Landsat asset details reject runtime locators and credential-shaped fields", async () => {
  const bridge = createTauriDesktopShellBridge({
    invoke: async (command) => {
      if (command !== "application_landsat_asset_details") return {};
      return {
        product_key: "landsat-item",
        display_id: "LC09_L2SP_123045_20260101_20260103_02_T1",
        processing_level: "l2sp",
        total_size_bytes: 20,
        assets: [{
          asset_key: "red",
          file_name: "red.tif",
          role: "primary",
          size_bytes: 20,
          stable_href: "https://example.invalid/red.tif",
          sas: "must-not-cross",
          authorization: "must-not-cross",
        }],
      };
    },
    getCurrentWindow: () => ({
      minimize: async () => undefined,
      toggleMaximize: async () => undefined,
      close: async () => undefined,
      innerSize: async () => ({ width: 1, height: 1 }),
      startResizeDragging: async () => undefined,
    }),
    open: async () => null,
  }, "tauri://localhost/", false);

  const result = await bridge.application.landsatAssetDetails({
    request_id: "asset-details-secret-rejection",
    batch_id: "landsat-batch",
    item_id: "landsat-item",
  });
  assert.equal(result.ok, false);
  assert(!JSON.stringify(result).includes("example.invalid"));
  assert(!JSON.stringify(result).includes("must-not-cross"));
});

test("Tauri Preview save stays inside the typed application boundary", async () => {
  const calls: Array<[string, Record<string, unknown> | undefined]> = [];
  const dependencies: TauriBridgeDependencies = {
    invoke: async (command, args) => {
      calls.push([command, args]);
      if (command === "application_save_preview_asset") {
        return {
          path: "D:/preview/S1A_IW_P1__preview.jpg",
          file_name: "S1A_IW_P1__preview.jpg",
          bytes_written: 3,
        };
      }
      return {};
    },
    getCurrentWindow: () => ({
      minimize: async () => undefined,
      toggleMaximize: async () => undefined,
      close: async () => undefined,
      innerSize: async () => ({ width: 1, height: 1 }),
      startResizeDragging: async () => undefined,
    }),
    open: async () => null,
  };
  const bridge = createTauriDesktopShellBridge(dependencies, "tauri://localhost/", false);
  const result = await bridge.application.savePreview!({
    data_url: "data:image/jpeg;base64,AQID",
    output_dir: "D:/preview",
    file_name: "S1A/IW:P1.zip",
  });
  assert.deepEqual(result, {
    ok: true,
    path: "D:/preview/S1A_IW_P1__preview.jpg",
    file_name: "S1A_IW_P1__preview.jpg",
    bytes_written: 3,
  });
  assert.deepEqual(calls, [[
    "application_save_preview_asset",
    {
      request: {
        data_url: "data:image/jpeg;base64,AQID",
        output_dir: "D:/preview",
        file_name: "S1A/IW:P1.zip",
      },
    },
  ]]);
  const rejected = await bridge.application.savePreview!({
    data_url: "https://example.org/preview.jpg",
    output_dir: "D:/preview",
    file_name: "P1",
  });
  assert.equal(rejected.ok, false);
  assert.equal(calls.length, 1);
});

test("Tauri bounded bridge keeps page and Store selection requests typed and candidate-free", async () => {
  const calls: Array<[string, Record<string, unknown> | undefined]> = [];
  const core = {
    readiness: { schema_version: 9, ready: true },
    workspace_context: null,
    region_aoi: null,
    region_aois: [],
    tasks: [],
    legacy_task_warnings: [],
    workbench: null,
  };
  const page = {
    batch_id: "batch-1",
    revision: 4,
    page_index: 0,
    page_size: 250,
    total_count: 975,
    checked_count: 0,
    items: Array.from({ length: 250 }, (_, ordinal) => ({ ordinal })),
  };
  const render = { region_id: "region-1", revision: 3, items: [] };
  const dependencies: TauriBridgeDependencies = {
    invoke: async (command, args) => {
      calls.push([command, args]);
      if (
        [
          "application_core_restore",
          "application_create_task_from_workbench",
          "application_append_task_from_workbench",
        ].includes(command)
      ) {
        return {
          readiness: {
            contract: "application_service_readonly_v1",
            status: "store_ready",
            store_schema_version: 9,
          },
          core,
        };
      }
      if (command === "application_workbench_page") {
        return {
          readiness: {
            contract: "application_service_readonly_v1",
            status: "store_ready",
            store_schema_version: 9,
          },
          page,
          render,
        };
      }
      if (command === "application_update_workbench_selection") {
        return {
          batch_id: "batch-1",
          revision: 5,
          checked_count: 1,
          page_index: 0,
          page_size: 250,
        };
      }
      throw new Error(`unexpected command ${command}`);
    },
    getCurrentWindow: () => ({
      minimize: async () => undefined,
      toggleMaximize: async () => undefined,
      close: async () => undefined,
      innerSize: async () => ({ width: 100, height: 100 }),
      startResizeDragging: async () => undefined,
    }),
    open: async () => null,
  };
  const bridge = createTauriDesktopShellBridge(dependencies, "tauri://localhost/", false);
  const pageRequest = {
    batch_id: "batch-1",
    expected_revision: 4,
    page_index: 0,
    page_size: 250,
  };
  const selection = {
    batch_id: "batch-1",
    expected_revision: 4,
    kind: "check_items" as const,
    item_ids: ["item-1"],
    page_index: 0,
    page_size: 250,
    updated_at: "2026-08-13T00:00:00Z",
  };
  const createRequest = {
    task_id: "task-1",
    batch_id: "batch-1",
    expected_workbench_revision: 5,
    output_root: "D:/fixture",
    layout: "direct" as const,
    concurrency: 1,
    created_at: "2026-08-13T00:00:01Z",
  };
  const appendRequest = {
    task_id: "task-1",
    batch_id: "batch-1",
    expected_workbench_revision: 5,
    event_id: "event-1",
    occurred_at: "2026-08-13T00:00:02Z",
  };

  assert.equal((await bridge.application.coreRestore(true)).ok, true);
  const boundedPage = await bridge.application.workbenchPage(pageRequest);
  assert.equal(boundedPage.ok, true);
  if (boundedPage.ok) assert.deepEqual(boundedPage.render, render);
  assert.deepEqual(await bridge.application.updateWorkbenchSelection(selection), {
    ok: true,
    summary: { batch_id: "batch-1", revision: 5, checked_count: 1, page_index: 0, page_size: 250 },
  });
  assert.equal((await bridge.application.createTaskFromWorkbench(createRequest)).ok, true);
  assert.equal((await bridge.application.appendTaskFromWorkbench(appendRequest)).ok, true);
  assert.deepEqual(calls, [
    ["application_core_restore", { includeTerminal: true }],
    ["application_workbench_page", { request: { request: pageRequest } }],
    ["application_update_workbench_selection", { command: selection }],
    ["application_create_task_from_workbench", { request: createRequest }],
    ["application_append_task_from_workbench", { request: appendRequest }],
  ]);
});

test("Tauri AOI boundary routes use named search and selector commands", async () => {
  const calls: Array<[string, Record<string, unknown> | undefined]> = [];
  const bridge = createTauriDesktopShellBridge({
    invoke: async (command, args) => {
      calls.push([command, args]);
      if (command === "application_admin_boundary_search") {
        return {
          ok: true,
          query: "California",
          provider: "nominatim",
          warning: null,
          results: [],
        };
      }
      if (command === "application_admin_boundary_resolve") {
        return {
          boundary_id: "nominatim:relation:123",
          provider: "nominatim",
          provider_object_id: "relation:123",
          display_name: "Wuhan",
          country_code: "cn",
          admin_level: "5",
          bbox: { west: 113.7, east: 115.1, south: 29.9, north: 31.4, crs: "EPSG:4326" },
          geometry: { type: "Polygon", coordinates: [] },
          geometry_kind: "polygon",
          source: "nominatim",
          attribution: "OpenStreetMap contributors",
          cache_status: "miss",
        };
      }
      if (command === "application_admin_options") {
        return {
          ok: true,
          provider: "datav",
          warning: null,
          provinces: [{ name: "全国", provider_object_id: "100000" }],
          cities: [{ name: "全部", provider_object_id: null }],
          districts: [{ name: "全部", provider_object_id: null }],
        };
      }
      throw new Error(`unexpected command: ${command}`);
    },
    getCurrentWindow: () => ({
      minimize: async () => undefined,
      toggleMaximize: async () => undefined,
      close: async () => undefined,
      innerSize: async () => ({ width: 100, height: 100 }),
      startResizeDragging: async () => undefined,
    }),
    open: async () => null,
  }, "tauri://localhost/", false);

  assert.equal((await bridge.application.adminBoundarySearch?.({
    query: "DataV测试新区",
    provider_object_id: "659012",
  })).ok, true);
  assert.equal((await bridge.application.adminBoundaryResolve?.({
    provider: "nominatim",
    provider_object_id: "relation:123",
  })).provider_object_id, "relation:123");
  assert.equal((await bridge.application.adminOptions?.({ province: "全国", city: "全部" })).ok, true);
  assert.deepEqual(calls, [
    ["application_admin_boundary_search", { request: {
      query: "DataV测试新区",
      provider_object_id: "659012",
    } }],
    ["application_admin_boundary_resolve", { request: {
      provider: "nominatim",
      provider_object_id: "relation:123",
    } }],
    ["application_admin_options", { request: { province: "全国", city: "全部" } }],
  ]);
});

test("Tauri catalogue freeze preserves the typed application error", async () => {
  const request = catalogueFreezeRequest();
  const bridge = createTauriDesktopShellBridge({
    invoke: async (command, args) => {
      assert.equal(command, "application_catalog_search_and_freeze");
      assert.deepEqual(args, { request });
      throw {
        code: "CATALOGUE_CAPABILITY_UNAVAILABLE",
        message_key: "application_service.unavailable",
      };
    },
    getCurrentWindow: () => ({
      minimize: async () => undefined,
      toggleMaximize: async () => undefined,
      close: async () => undefined,
      innerSize: async () => ({ width: 100, height: 100 }),
      startResizeDragging: async () => undefined,
    }),
    open: async () => null,
  }, "tauri://localhost/", false);

  assert.deepEqual(await bridge.application.catalogSearchAndFreeze(request), {
    ok: false,
    error: APPLICATION_UNAVAILABLE_TEXT,
    code: "CATALOGUE_CAPABILITY_UNAVAILABLE",
  });
});

test("Tauri SAR download policy routes exact manual settings through named commands", async () => {
  const calls: Array<[string, Record<string, unknown> | undefined]> = [];
  const bridge = createTauriDesktopShellBridge({
    invoke: async (command, args) => {
      calls.push([command, args]);
      if (command === "application_get_sar_download_policy_settings") {
        return { mode: "manual", max_concurrent_items: 2, organize_by_product: false, default_output_root: "" };
      }
      if (command === "application_save_sar_download_policy_settings") {
        return args?.settings;
      }
      throw new Error(`unexpected command: ${command}`);
    },
    getCurrentWindow: () => ({
      minimize: async () => undefined,
      toggleMaximize: async () => undefined,
      close: async () => undefined,
      innerSize: async () => ({ width: 100, height: 100 }),
      startResizeDragging: async () => undefined,
    }),
    open: async () => null,
  }, "tauri://localhost/", false);

  assert.deepEqual(await bridge.application.getSarDownloadPolicySettings?.(), {
    mode: "manual",
    max_concurrent_items: 2,
    organize_by_product: false,
    default_output_root: "",
  });
  const saved = {
    mode: "manual" as const,
    max_concurrent_items: 5,
    organize_by_product: true,
    default_output_root: "E:/InSAR Downloads",
  };
  assert.deepEqual(await bridge.application.saveSarDownloadPolicySettings?.(saved), saved);
  assert.deepEqual(calls, [
    ["application_get_sar_download_policy_settings", undefined],
    ["application_save_sar_download_policy_settings", { settings: saved }],
  ]);
});

test("Tauri external URLs use the named native opener command", async () => {
  const calls: Array<[string, Record<string, unknown> | undefined]> = [];
  const bridge = createTauriDesktopShellBridge({
    invoke: async (command, args) => {
      calls.push([command, args]);
      if (command === "application_open_external_url") return { url: args?.url };
      throw new Error(`unexpected command: ${command}`);
    },
    getCurrentWindow: () => ({
      minimize: async () => undefined,
      toggleMaximize: async () => undefined,
      close: async () => undefined,
      innerSize: async () => ({ width: 100, height: 100 }),
      startResizeDragging: async () => undefined,
    }),
    open: async () => null,
  }, "tauri://localhost/", false);

  assert.deepEqual(await bridge.application.openExternalUrl?.("https://leafletjs.com/"), {
    ok: true,
    url: "https://leafletjs.com/",
  });
  assert.deepEqual(calls, [[
    "application_open_external_url",
    { url: "https://leafletjs.com/" },
  ]]);
});

test("Tauri network settings preserve the user-configured Tianditu token", async () => {
  const calls: Array<[string, Record<string, unknown> | undefined]> = [];
  const settings = {
    proxy_enabled: false,
    proxy_url: "",
    cache_enabled: true,
    cache_dir: "E:/cache",
    cache_limit_mb: 2048,
    tianditu_token: "user-configured-tianditu-token",
    asf_ssl_verify: true,
  };
  const bridge = createTauriDesktopShellBridge({
    invoke: async (command, args) => {
      calls.push([command, args]);
      if (command === "application_get_network_settings") return settings;
      if (command === "application_save_network_settings") return args?.settings;
      throw new Error(`unexpected command: ${command}`);
    },
    getCurrentWindow: () => ({
      minimize: async () => undefined,
      toggleMaximize: async () => undefined,
      close: async () => undefined,
      innerSize: async () => ({ width: 100, height: 100 }),
      startResizeDragging: async () => undefined,
    }),
    open: async () => null,
  }, "tauri://localhost/", false);

  assert.deepEqual(await bridge.application.getNetworkSettings?.(), settings);
  assert.deepEqual(await bridge.application.saveNetworkSettings?.(settings), settings);
  assert.deepEqual(calls, [
    ["application_get_network_settings", undefined],
    ["application_save_network_settings", { settings }],
  ]);
});

test("Tauri numeric SAR background count carries exact batch revision and returns bounded core facts", async () => {
  const request = {
    request_id: "m1-search-1-count",
    batch_id: "search-1",
    expected_workbench_revision: 4,
  };
  const bridge = createTauriDesktopShellBridge({
    invoke: async (command, args) => {
      assert.equal(command, "application_sar_background_count");
      assert.deepEqual(args, { request });
      return {
        applied: true,
        total_count: 11_976,
        snapshot: {
          readiness: { contract: "v3", status: "store_ready", store_schema_version: 18 },
          core: { workbench: { batch_id: "search-1", revision: 4 } },
        },
      };
    },
    getCurrentWindow: () => ({
      minimize: async () => undefined,
      toggleMaximize: async () => undefined,
      close: async () => undefined,
      innerSize: async () => ({ width: 100, height: 100 }),
      startResizeDragging: async () => undefined,
    }),
    open: async () => null,
  }, "tauri://localhost/", false);

  const result = await bridge.application.sarBackgroundCount(request);
  assert.equal(result.ok, true);
  if (!result.ok) return;
  assert.equal(result.applied, true);
  assert.equal(result.total_count, 11_976);
  assert.equal(
    result.snapshot?.core.workbench
      && (result.snapshot.core.workbench as Record<string, unknown>).batch_id,
    "search-1",
  );
});

test("Tauri search progress preserves the generated finite observation", async () => {
  const bridge = createTauriDesktopShellBridge({
    invoke: async (command, args) => {
      assert.equal(command, "application_sar_search_progress");
      assert.deepEqual(args, { requestId: "m1-search-progress" });
      return {
        request_id: "m1-search-progress",
        sequence: 4,
        stage: "fetching",
        provider_request_count: 2,
        raw_fetched_count: 240,
        mapped_count: 220,
        exact_matched_count: 210,
        admitted_count: 200,
        requested_limit: 250,
        remote_total: 1248,
        elapsed_ms: 1500,
        message: "已获取 200 / 250",
        terminal: false,
      };
    },
    getCurrentWindow: () => ({
      minimize: async () => undefined,
      toggleMaximize: async () => undefined,
      close: async () => undefined,
      innerSize: async () => ({ width: 100, height: 100 }),
      startResizeDragging: async () => undefined,
    }),
    open: async () => null,
  }, "tauri://localhost/", false);

  assert.deepEqual(await bridge.application.sarSearchProgress("m1-search-progress"), {
    ok: true,
    progress: {
      request_id: "m1-search-progress",
      sequence: 4,
      stage: "fetching",
      provider_request_count: 2,
      raw_fetched_count: 240,
      mapped_count: 220,
      exact_matched_count: 210,
      admitted_count: 200,
      requested_limit: 250,
      remote_total: 1248,
      elapsed_ms: 1500,
      message: "已获取 200 / 250",
      terminal: false,
    },
  });
});

test("Tauri search progress preserves real zero separately from unknown for ALL", async () => {
  const bridge = createTauriDesktopShellBridge({
    invoke: async () => ({
      request_id: "all-transport-only",
      sequence: 3,
      stage: "fetching",
      provider_request_count: 2,
      raw_fetched_count: 400,
      mapped_count: 0,
      exact_matched_count: 0,
      admitted_count: 0,
      requested_limit: null,
      remote_total: 900,
      elapsed_ms: 1200,
      message: "已完成 2 次请求，获取原始记录 400 条",
      terminal: false,
    }),
    getCurrentWindow: () => ({
      minimize: async () => undefined,
      toggleMaximize: async () => undefined,
      close: async () => undefined,
      innerSize: async () => ({ width: 100, height: 100 }),
      startResizeDragging: async () => undefined,
    }),
    open: async () => null,
  }, "tauri://localhost/", false);
  const result = await bridge.application.sarSearchProgress("all-transport-only");
  assert.equal(result.ok, true);
  if (result.ok) {
    assert.equal(result.progress?.sequence, 3);
    assert.equal(result.progress?.admitted_count, 0);
    assert.equal(result.progress?.requested_limit, null);
  }
});

test("Tauri search progress rejects generated count-order violations", async () => {
  const bridge = createTauriDesktopShellBridge({
    invoke: async () => ({
      request_id: "invalid-count-order",
      sequence: 2,
      stage: "fetching",
      provider_request_count: 1,
      raw_fetched_count: 1,
      mapped_count: 2,
      exact_matched_count: 1,
      admitted_count: 1,
      requested_limit: null,
      remote_total: 10,
      elapsed_ms: 10,
      message: "invalid",
      terminal: false,
    }),
    getCurrentWindow: () => ({
      minimize: async () => undefined,
      toggleMaximize: async () => undefined,
      close: async () => undefined,
      innerSize: async () => ({ width: 100, height: 100 }),
      startResizeDragging: async () => undefined,
    }),
    open: async () => null,
  }, "tauri://localhost/", false);
  assert.deepEqual(await bridge.application.sarSearchProgress("invalid-count-order"), {
    ok: false,
    error: "Tauri application service operation failed",
    code: "V3_APPLICATION_UNAVAILABLE",
  });
});

test("Tauri SAR freeze preserves the sanitized typed search diagnostic", async () => {
  const request = {
    request_id: "m1-search-diagnostic",
    search: { query: { provider: "ASF", dataset: "SENTINEL-1", max_results: 20 } },
    batch_id: "search-diagnostic",
    created_at: "2026-08-16T00:00:00Z",
    aoi_name: "unbounded AOI",
    aoi_geojson: null,
    page_size: 24,
  };
  const diagnostic = {
    operation: "sar_search_and_freeze",
    route: "legacy_asf_search",
    sanitized_reason: "M1 accepts only mapped ASF catalogue products",
    dataset: "SENTINEL-1",
    query_shape: {
      platform_count: 0,
      processing_level_count: 1,
      product_family_count: 0,
      beam_mode_count: 1,
      polarization_count: 1,
      has_instrument: false,
      has_relative_orbit: false,
      has_frame: false,
      has_asf_frame: false,
      has_path: false,
      has_track_range: true,
      has_frame_range: true,
      has_aoi_geojson: false,
      has_aoi_wkt: false,
      has_bbox: false,
      has_burst: false,
    },
    aoi_state: "none",
    date_range: { start: "2026-08-01T00:00:00Z", end: "2026-08-02T23:59:59Z" },
    filter_state: {
      platforms: [],
      instrument: null,
      processing_levels: ["SLC"],
      product_families: [],
      beam_modes: ["IW"],
      polarizations: ["VV+VH"],
      flight_direction: "ASCENDING",
      look_direction: "RIGHT",
      relative_orbit: null,
      frame: null,
      asf_frame: null,
      path: null,
      track_range: { start: "118", end: "118" },
      frame_range: { start: "42", end: "42" },
    },
    max_results: 20,
  } as const;
  const bridge = createTauriDesktopShellBridge({
    invoke: async (command, args) => {
      assert.equal(command, "application_sar_search_and_freeze");
      assert.deepEqual(args, { request });
      throw {
        code: "V3_APPLICATION_INVALID_REQUEST",
        message_key: "application_service.unavailable",
        context: diagnostic.sanitized_reason,
        diagnostic,
      };
    },
    getCurrentWindow: () => ({
      minimize: async () => undefined,
      toggleMaximize: async () => undefined,
      close: async () => undefined,
      innerSize: async () => ({ width: 100, height: 100 }),
      startResizeDragging: async () => undefined,
    }),
    open: async () => null,
  }, "tauri://localhost/", false);

  assert.deepEqual(await bridge.application.sarSearchAndFreeze(request), {
    ok: false,
    error: APPLICATION_UNAVAILABLE_TEXT,
    code: "V3_APPLICATION_INVALID_REQUEST",
    diagnostic,
  });
});

test("Tauri SAR freeze drops a malformed or sensitive diagnostic", async () => {
  const request = {
    request_id: "m1-search-redaction",
    search: { query: { provider: "ASF", dataset: "SENTINEL-1" } },
    batch_id: "search-redaction",
    created_at: "2026-08-16T00:00:00Z",
    aoi_name: "unbounded AOI",
    aoi_geojson: null,
    page_size: 24,
  };
  const bridge = createTauriDesktopShellBridge({
    invoke: async () => {
      throw {
        code: "V3_APPLICATION_INVALID_REQUEST",
        message_key: "application_service.unavailable",
        context: "password=fixture-secret",
        diagnostic: {
          operation: "sar_search_and_freeze",
          route: "legacy_asf_search",
          sanitized_reason: "password=fixture-secret",
        },
      };
    },
    getCurrentWindow: () => ({
      minimize: async () => undefined,
      toggleMaximize: async () => undefined,
      close: async () => undefined,
      innerSize: async () => ({ width: 100, height: 100 }),
      startResizeDragging: async () => undefined,
    }),
    open: async () => null,
  }, "tauri://localhost/", false);

  const result = await bridge.application.sarSearchAndFreeze(request);
  assert.deepEqual(result, {
    ok: false,
    error: APPLICATION_UNAVAILABLE_TEXT,
    code: "V3_APPLICATION_INVALID_REQUEST",
  });
  assert(!JSON.stringify(result).includes("fixture-secret"));
});

test("Tauri provider failures preserve bounded Unicode user copy", async () => {
  const bridge = createTauriDesktopShellBridge({
    invoke: async () => {
      throw {
        code: "V3_PROVIDER_OPERATION_FAILED",
        message_key: "application_service.unavailable",
        context: "Bhoonidhi 网络请求失败",
      };
    },
    getCurrentWindow: () => ({
      minimize: async () => undefined,
      toggleMaximize: async () => undefined,
      close: async () => undefined,
      innerSize: async () => ({ width: 100, height: 100 }),
      startResizeDragging: async () => undefined,
    }),
    open: async () => null,
  }, "tauri://localhost/", false);

  assert.deepEqual(await bridge.application.catalogSearchAndFreeze(catalogueFreezeRequest()), {
    ok: false,
    error: APPLICATION_UNAVAILABLE_TEXT,
    code: "V3_PROVIDER_OPERATION_FAILED",
    context: "Bhoonidhi 网络请求失败",
  });
});

test("Tauri Bhoonidhi catalogue error carries only a bounded HTTP status", async () => {
  const bridge = createTauriDesktopShellBridge({
    invoke: async () => {
      throw {
        code: "BHOONIDHI_HTTP_ERROR",
        message_key: "application_service.unavailable",
        context: "http_status:422",
      };
    },
    getCurrentWindow: () => ({
      minimize: async () => undefined,
      toggleMaximize: async () => undefined,
      close: async () => undefined,
      innerSize: async () => ({ width: 100, height: 100 }),
      startResizeDragging: async () => undefined,
    }),
    open: async () => null,
  }, "tauri://localhost/", false);
  assert.deepEqual(await bridge.application.catalogSearchAndFreeze(catalogueFreezeRequest()), {
    ok: false,
    code: "BHOONIDHI_HTTP_ERROR",
    error: APPLICATION_UNAVAILABLE_TEXT,
    context: "http_status:422",
  });
});

test("Tauri compatibility bridge rejects legacy production calls without invoking the Worker", async () => {
  const bridge = createTauriDesktopShellBridge({
    invoke: async () => {
      throw new Error("normal Tauri must not invoke a legacy production route");
    },
    getCurrentWindow: () => ({
      minimize: async () => undefined,
      toggleMaximize: async () => undefined,
      close: async () => undefined,
      innerSize: async () => ({ width: 100, height: 100 }),
      startResizeDragging: async () => undefined,
    }),
    open: async () => null,
  }, "tauri://localhost/", false);

  const result = await bridge.compatibility.callLegacy(
    "search_admin_boundaries",
    ["北京", "北京市", "", "", 1],
  );
  assert.deepEqual(result, {
    ok: false,
    error: "当前单一 Tauri 宿主不接受旧生产接口：search_admin_boundaries",
    code: "COMPAT_WORKER_OPERATION_UNSUPPORTED",
  });
});

test("Tauri CDSE login preserves only the typed error projection", async () => {
  const request = {
    username: "fixture-user",
    password: "fixture-password",
    totp: "123456",
  };
  const bridge = createTauriDesktopShellBridge({
    invoke: async (command, args) => {
      assert.equal(command, "application_cdse_login");
      assert.deepEqual(args, { request });
      throw {
        code: "CDSE_OAUTH_REJECTED",
        message_key: "application_service.unavailable",
        context: request.password,
      };
    },
    getCurrentWindow: () => ({
      minimize: async () => undefined,
      toggleMaximize: async () => undefined,
      close: async () => undefined,
      innerSize: async () => ({ width: 100, height: 100 }),
      startResizeDragging: async () => undefined,
    }),
    open: async () => null,
  }, "tauri://localhost/", false);

  const result = await bridge.application.cdseLogin(request);
  assert.deepEqual(result, {
    ok: false,
    error: APPLICATION_UNAVAILABLE_TEXT,
    code: "CDSE_OAUTH_REJECTED",
  });
  assert!(!JSON.stringify(result).includes(request.password));
  assert!(!JSON.stringify(result).includes(request.totp));
});

test("Tauri CDSE quicklook accepts only bounded canonical image data URLs", async () => {
  const request = {
    request_id: "quicklook-request",
    batch_id: "quicklook-batch",
    item_id: "quicklook-item",
  };
  const rejected = [
    {
      media_type: "image/png",
      data_url: "https://catalogue.dataspace.copernicus.eu/odata/v1/Assets(untrusted)/$value",
      byte_length: 3,
    },
    {
      media_type: "image/svg+xml",
      data_url: "data:image/svg+xml;base64,AQID",
      byte_length: 3,
    },
    {
      media_type: "image/png",
      data_url: "data:image/png;base64,AQJ=",
      byte_length: 2,
    },
    {
      media_type: "image/png",
      data_url: "data:image/png;base64,AQID",
      byte_length: 16 * 1024 * 1024 + 1,
    },
  ];

  for (const response of rejected) {
    const bridge = createTauriDesktopShellBridge({
      invoke: async (command, args) => {
        assert.equal(command, "application_cdse_quicklook");
        assert.deepEqual(args, { request });
        return response;
      },
      getCurrentWindow: () => ({
        minimize: async () => undefined,
        toggleMaximize: async () => undefined,
        close: async () => undefined,
        innerSize: async () => ({ width: 100, height: 100 }),
        startResizeDragging: async () => undefined,
      }),
      open: async () => null,
    }, "tauri://localhost/", false);

    assert.deepEqual(await bridge.application.cdseQuicklook(request), {
      ok: false,
      error: "Tauri application service operation failed",
      code: "V3_APPLICATION_UNAVAILABLE",
    });
  }
});

test("Tauri shared Preview bridge rejects malformed generated descriptors", async () => {
  const request = {
    request_id: "preview-request",
    batch_id: "preview-batch",
    item_id: "preview-item",
  };
  const valid = {
    product_key: "preview-product",
    descriptor: {
      preview_kind: "quicklook",
      source_asset_keys: ["preview-key"],
      render_profile_id: "asf.sentinel-1.grd-browse-geographic.v1",
      media_type: "image/jpeg",
      credential_requirement: "none",
      cache_policy: "session",
      provenance: "ASF official Browse",
    },
    source_asset_role: "browse",
    source_asset_key: "preview-key",
    media_type: "image/jpeg",
    data_url: "data:image/jpeg;base64,AQID",
    byte_length: 3,
  };
  const rejected = [
    { ...valid, source_asset_role: "primary" },
    { ...valid, source_asset_role: "metadata" },
    { ...valid, source_asset_role: "auxiliary" },
    { ...valid, source_asset_role: "derived" },
    { ...valid, source_asset_key: "different-key" },
    { ...valid, media_type: "image/png" },
    { ...valid, product_key: "" },
    { ...valid, data_url: "https://untrusted.example/preview.jpg" },
  ];

  for (const response of rejected) {
    const bridge = createTauriDesktopShellBridge({
      invoke: async (command, args) => {
        assert.equal(command, "application_preview_asset");
        assert.deepEqual(args, { request });
        return response;
      },
      getCurrentWindow: () => ({
        minimize: async () => undefined,
        toggleMaximize: async () => undefined,
        close: async () => undefined,
        innerSize: async () => ({ width: 100, height: 100 }),
        startResizeDragging: async () => undefined,
      }),
      open: async () => null,
    }, "tauri://localhost/", false);

    assert.deepEqual(await bridge.application.previewAsset(request), {
      ok: false,
      error: "Tauri application service operation failed",
      code: "V3_APPLICATION_UNAVAILABLE",
    });
  }
});

test("Tauri adapter maps cancellation, malformed filters, and native failures safely", async () => {
  let openCalls = 0;
  const window = {
    minimize: async () => undefined,
    toggleMaximize: async () => undefined,
    close: async () => undefined,
    innerSize: async () => {
      throw new Error("native failure");
    },
    startResizeDragging: async () => undefined,
  };
  const dependencies: TauriBridgeDependencies = {
    invoke: async () => ({}),
    getCurrentWindow: () => window,
    open: async () => {
      openCalls += 1;
      return null;
    },
  };
  const bridge = createTauriDesktopShellBridge(
    dependencies,
    "tauri://localhost/",
    false,
  );

  assert.deepEqual(await bridge.dialog.pickDirectory("取消"), { ok: true, path: "" });
  assert.deepEqual(await bridge.window.getSize(), {
    ok: false,
    error: "Tauri shell operation failed",
    code: "GUI003",
  });
  assert.deepEqual(await bridge.window.resizeFromEdge("invalid", 1, 1, 1, 1), {
    ok: false,
    error: "Tauri shell operation failed",
    code: "GUI003",
  });
  assert.deepEqual(await bridge.dialog.pickOpenFile("坏过滤器", ["not a filter"]), {
    ok: false,
    path: "",
    error: "Tauri dialog operation failed",
    code: "GUI003",
  });
  assert.equal(openCalls, 1);
});

test("Tauri DEM and GACOS transports carry exact AOI identity without client windows", async () => {
  const calls: Array<[string, Record<string, unknown> | undefined]> = [];
  const bridge = createTauriDesktopShellBridge({
    invoke: async (command, args) => {
      calls.push([command, args]);
      return command === "application_gacos_plan"
        ? { ok: true, plan: {}, report: {}, offline: true }
        : {};
    },
    getCurrentWindow: () => ({
      minimize: async () => undefined,
      toggleMaximize: async () => undefined,
      close: async () => undefined,
      innerSize: async () => ({ width: 100, height: 100 }),
      startResizeDragging: async () => undefined,
    }),
    open: async () => null,
  }, "tauri://localhost/", false);
  const geometry = {
    type: "MultiPolygon",
    coordinates: [
      [[[[166, -47], [179, -47], [179, -34], [166, -34], [166, -47]]]],
      [[[[-179, -45], [-175, -45], [-175, -35], [-179, -35], [-179, -45]]]],
    ],
  };
  const demRequest = {
    request_id: "dem-nz-request",
    task_id: "dem-nz-task",
    dataset: "COP30",
    provider: "COPERNICUS_AWS",
    aoi_geojson: geometry,
    aoi_name: "New Zealand",
    output_root: "F:/InSAR_Studio/_verification/dem-nz",
    convert: false,
    concurrency: 2,
    created_at: "2026-08-19T00:00:00Z",
  };
  await bridge.application.createStandaloneDemTask?.(demRequest);
  await bridge.application.gacosPlan?.({
    request_id: "gacos-nz-request",
    aoi_geojson: geometry,
    aoi_name: "New Zealand",
    scenes: [],
  });

  assert.deepEqual(calls, [
    ["application_create_standalone_dem_task", { request: demRequest }],
    ["application_gacos_plan", { payload: {
      request_id: "gacos-nz-request",
      aoi_geojson: geometry,
      aoi_name: "New Zealand",
      scenes: [],
    } }],
  ]);
  assert(!JSON.stringify(calls).includes("processing_windows"));
  assert(!JSON.stringify(calls).includes('"bbox"'));
});

test("Tauri GACOS request bridge preserves opaque handles and durable status shape", async () => {
  const calls: Array<[string, Record<string, unknown> | undefined]> = [];
  const fingerprint = "a".repeat(64);
  const taskId = "gacos-request-" + fingerprint;
  const request = {
    prepared_batch_id: "prepared-gacos-batch",
    plan_fingerprint_sha256: fingerprint,
    task_id: taskId,
    confirm: true,
    force_reapply: false,
  };
  const response = {
    ok: true,
    task_id: taskId,
    records: [{
      attempt_id: "attempt-1",
      task_id: taskId,
      item_id: "batch-1",
      attempt_number: 1,
      state: "manual_verification_required",
      reason_code: null,
      http_status: 202,
      automatic_retry_allowed: false,
      created_at: "2026-09-06T00:00:00Z",
      updated_at: "2026-09-06T00:00:00Z",
    }],
    request_groups: [{
      item_id: "batch-1",
      dates: ["2026-09-06"],
      hour: 12,
      minute: 30,
      bbox: [110, 30, 111, 31],
    }],
    task_status: "paused",
    manual_action_required: true,
    automatic_retry_allowed: false,
    message: "request recorded",
  };
  const bridge = createTauriDesktopShellBridge({
    invoke: async (command, args) => {
      calls.push([command, args]);
      return response;
    },
    getCurrentWindow: () => ({
      minimize: async () => undefined,
      toggleMaximize: async () => undefined,
      close: async () => undefined,
      innerSize: async () => ({ width: 100, height: 100 }),
      startResizeDragging: async () => undefined,
    }),
    open: async () => null,
  }, "tauri://localhost/", false);

  const submitted = await bridge.application.gacosSubmitRequest?.(request);
  const status = await bridge.application.gacosRequestStatus?.(taskId);
  assert.equal(submitted?.ok, true);
  assert.equal(status?.ok, true);
  assert.deepEqual(calls, [
    ["application_gacos_submit_request", { request }],
    ["application_gacos_request_status", { taskId }],
  ]);
  assert(!JSON.stringify(calls).includes("account_identity_hash"));
  if (status?.ok) {
    assert.equal(status.application_completed, false);
    assert.equal(status.submitted_count, 0);
  }
  response.records[0].state = "submitted";
  Object.assign(response, { submitted_count: 1, application_completed: true, manual_action_required: false });
  const accepted = await bridge.application.gacosRequestStatus?.(taskId);
  assert.equal(accepted?.ok, true);
  if (accepted?.ok) {
    assert.equal(accepted.records[0].state, "submitted");
    assert.equal(accepted.application_completed, true);
    assert.equal(accepted.manual_action_required, false);
    assert.equal(accepted.automatic_retry_allowed, false);
  }
  Object.assign(response, { submitted_count: 0 });
  assert.equal((await bridge.application.gacosRequestStatus?.(taskId))?.ok, false);
});

test("Tauri GACOS local result bridge preserves bound request identity and native task result", async () => {
  const calls: Array<[string, Record<string, unknown> | undefined]> = [];
  const request = {
    task_id: "task-ui-gacos-import",
    source_request_task_id: "gacos-request-task",
    source_request_item_id: "gacos-item-1",
    source_paths: ["F:/results/gacos/result.tif"],
  };
  const response = {
    ok: true,
    task_id: request.task_id,
    output_root: "F:/results/gacos-results",
    status: "queued",
    source: "native-gacos-local-import",
  };
  const bridge = createTauriDesktopShellBridge({
    invoke: async (command, args) => {
      calls.push([command, args]);
      return response;
    },
    getCurrentWindow: () => ({
      minimize: async () => undefined,
      toggleMaximize: async () => undefined,
      close: async () => undefined,
      innerSize: async () => ({ width: 100, height: 100 }),
      startResizeDragging: async () => undefined,
    }),
    open: async () => null,
  }, "tauri://localhost/", false);

  const created = await bridge.application.gacosCreateImportTask?.(request);
  assert.deepEqual(created, response);
  assert.deepEqual(calls, [
    ["application_gacos_create_import_task", { request }],
  ]);
});

// TASK 5 RED contract: the desktop adapter must expose one typed batch prepare
// and one typed batch confirm operation, with no transport credentials or URLs.

test("Tauri batch bridge uses exact prepare, confirm, append, resume, and start commands", async () => {
  const calls: Array<[string, Record<string, unknown> | undefined]> = [];
  const geometry = {
    type: "Polygon",
    coordinates: [[[110, 30], [111, 30], [111, 31], [110, 30]]],
  };
  const preparedIntent = {
    selected_frequency: "A",
    selected_polarizations: ["HH", "HV"],
    requested_geometry: geometry,
    effective_geometry: geometry,
    output_authority: "client_derived",
    quality_status: "PROVISIONAL",
  };
  const preflightBatch = {
    source_batch_id: "nisar-batch",
    workbench_revision: 12,
    eligible_items: [{
      source_item_id: "scene-1",
      effective_geometry: geometry,
      available_selections: [{ frequency: "A", polarizations: ["HH", "HV"] }],
    }],
    excluded_items: [{ source_item_id: "scene-no-overlap", reason: "no_overlap" }],
    common_selections: [{ frequency: "A", polarizations: ["HH", "HV"] }],
  };
  const preparedBatch = {
    source_batch_id: "nisar-batch",
    workbench_revision: 12,
    intents: [{
      source_batch_id: "nisar-batch",
      source_item_id: "scene-1",
      workbench_revision: 12,
      intent: preparedIntent,
    }],
    excluded_items: [{ source_item_id: "scene-no-overlap", reason: "no_overlap" }],
  };
  const core = {
    readiness: {
      contract: "application_service_readonly_v1",
      status: "store_ready",
      store_schema_version: 4,
    },
    core: {},
  };
  const dependencies: TauriBridgeDependencies = {
    invoke: async (command, args) => {
      calls.push([command, args]);
      if (command === "application_preflight_nisar_subset_batch_v2") return preflightBatch;
      if (command === "application_prepare_nisar_subset_batch_v2") return preparedBatch;
      if (command === "application_confirm_nisar_subset_batch_task_v2") return core;
      if (command === "application_append_nisar_subset_batch_task_v2") return core;
      if (command === "application_resume_nisar_subset_task") return core;
      if (command === "application_start_newly_confirmed_task") return core;
      return {};
    },
    getCurrentWindow: () => ({
      minimize: async () => undefined,
      toggleMaximize: async () => undefined,
      close: async () => undefined,
      innerSize: async () => ({ width: 100, height: 100 }),
      startResizeDragging: async () => undefined,
    }),
    open: async () => null,
  };
  const bridge = createTauriDesktopShellBridge(dependencies, "tauri://localhost/", false);
  const preflightRequest = {
    source_batch_id: "nisar-batch",
    source_item_ids: ["scene-1", "scene-no-overlap"],
    expected_workbench_revision: 12,
    subset_draft_geometry: geometry,
  };
  const prepareRequest = {
    ...preflightRequest,
    selected_frequency: "A",
    selected_polarizations: ["HH", "HV"],
  };
  const confirmRequest = {
    source_batch_id: "nisar-batch",
    expected_workbench_revision: 12,
    items: [{ source_item_id: "scene-1", prepared_intent: preparedIntent }],
    task_id: "nisar-batch-task",
    output_root: "E:/nisar-output",
    layout: "direct" as const,
    concurrency: 2,
    created_at: "2026-09-04T00:00:00Z",
  };
  const appendRequest = {
    task_id: confirmRequest.task_id,
    expected_task_revision: 14,
    source_item_ids: ["scene-2"],
    event_id: "append-scene-2",
    occurred_at: "2026-09-04T00:00:01Z",
  };

  const preflight = await bridge.application.preflightNisarSubsetBatchV2(preflightRequest);
  assert.deepEqual(preflight, { ok: true, preflight: preflightBatch });
  const prepared = await bridge.application.prepareNisarSubsetBatchV2(prepareRequest);
  assert.deepEqual(prepared, { ok: true, prepared: preparedBatch });
  const confirmed = await bridge.application.confirmNisarSubsetBatchTaskV2(confirmRequest);
  assert.equal(confirmed.ok, true);
  const appended = await bridge.application.appendNisarSubsetBatchTaskV2(appendRequest);
  assert.equal(appended.ok, true);
  const resumed = await bridge.application.resumeNisarSubsetTask(confirmRequest.task_id);
  assert.equal(resumed.ok, true);
  const started = await bridge.application.startNewlyConfirmedTask(confirmRequest.task_id);
  assert.equal(started.ok, true);
  assert.deepEqual(calls, [
    ["application_preflight_nisar_subset_batch_v2", { request: preflightRequest }],
    ["application_prepare_nisar_subset_batch_v2", { request: prepareRequest }],
    ["application_confirm_nisar_subset_batch_task_v2", { request: confirmRequest }],
    ["application_append_nisar_subset_batch_task_v2", { request: appendRequest }],
    ["application_resume_nisar_subset_task", { taskId: confirmRequest.task_id }],
    ["application_start_newly_confirmed_task", { taskId: confirmRequest.task_id }],
  ]);
  assert.doesNotMatch(JSON.stringify(calls), /(?:url|secret|token|authorization|password)/i);
});

test("Tauri batch bridge rejects malformed preflight batches at the strict boundary", async () => {
  const geometry = { type: "Polygon", coordinates: [[[110, 30], [111, 30], [111, 31], [110, 30]]] };
  const selection = { frequency: "A", polarizations: ["HH"] };
  const eligible = {
    source_item_id: "scene-1",
    effective_geometry: geometry,
    available_selections: [selection],
  };
  const valid = {
    source_batch_id: "nisar-batch",
    workbench_revision: 12,
    eligible_items: [eligible],
    excluded_items: [],
    common_selections: [selection],
  };
  const malformed = [
    { ...valid, eligible_items: [] },
    { ...valid, source_batch_id: "other-batch" },
    { ...valid, workbench_revision: 13 },
    { ...valid, eligible_items: [eligible, eligible] },
    { ...valid, unknown_transport_field: true },
    { ...valid, resolved_url: "https://untrusted.example/data", access_token: "must-not-cross" },
  ];

  for (const response of malformed) {
    const dependencies: TauriBridgeDependencies = {
      invoke: async () => response,
      getCurrentWindow: () => ({
        minimize: async () => undefined,
        toggleMaximize: async () => undefined,
        close: async () => undefined,
        innerSize: async () => ({ width: 100, height: 100 }),
        startResizeDragging: async () => undefined,
      }),
      open: async () => null,
    };
    const bridge = createTauriDesktopShellBridge(dependencies, "tauri://localhost/", false);
    const result = await bridge.application.preflightNisarSubsetBatchV2({
      source_batch_id: "nisar-batch",
      source_item_ids: ["scene-1"],
      expected_workbench_revision: 12,
      subset_draft_geometry: geometry,
    });
    assert.equal(result.ok, false);
  }
});

test("Tauri batch bridge rejects malformed prepared batches at the strict boundary", async () => {
  const geometry = { type: "Polygon", coordinates: [[[110, 30], [111, 30], [111, 31], [110, 30]]] };
  const intent = {
    selected_frequency: "A",
    selected_polarizations: ["HH"],
    requested_geometry: geometry,
    effective_geometry: geometry,
  };
  const valid = {
    source_batch_id: "nisar-batch",
    workbench_revision: 12,
    intents: [{ source_batch_id: "nisar-batch", source_item_id: "scene-1", workbench_revision: 12, intent }],
    excluded_items: [],
  };
  const malformed = [
    { ...valid, intents: [] },
    { ...valid, source_batch_id: "other-batch" },
    { ...valid, workbench_revision: 13 },
    { ...valid, intents: [{ ...valid.intents[0], source_batch_id: "other-batch" }] },
    { ...valid, intents: [valid.intents[0], valid.intents[0]] },
    { ...valid, unknown_transport_field: true },
    { ...valid, resolved_url: "https://untrusted.example/data", access_token: "must-not-cross" },
  ];

  for (const response of malformed) {
    const dependencies: TauriBridgeDependencies = {
      invoke: async () => response,
      getCurrentWindow: () => ({
        minimize: async () => undefined,
        toggleMaximize: async () => undefined,
        close: async () => undefined,
        innerSize: async () => ({ width: 100, height: 100 }),
        startResizeDragging: async () => undefined,
      }),
      open: async () => null,
    };
    const bridge = createTauriDesktopShellBridge(dependencies, "tauri://localhost/", false);
    const result = await bridge.application.prepareNisarSubsetBatchV2({
      source_batch_id: "nisar-batch",
      source_item_ids: ["scene-1"],
      expected_workbench_revision: 12,
      subset_draft_geometry: geometry,
      selected_frequency: "A",
      selected_polarizations: ["HH"],
    });
    assert.equal(result.ok, false);
  }
});

test("Tauri NISAR batch bridge accepts 200 selected scenes without applying the retired 100-scene product limit", async () => {
  const geometry = { type: "Polygon", coordinates: [[[110, 30], [111, 30], [111, 31], [110, 30]]] };
  const sourceItemIds = Array.from({ length: 200 }, (_, index) => `scene-${index + 1}`);
  const selection = { frequency: "A" as const, polarizations: ["HH"] };
  const preflightBatch = {
    source_batch_id: "nisar-batch-200",
    workbench_revision: 12,
    eligible_items: sourceItemIds.map((source_item_id) => ({
      source_item_id,
      effective_geometry: geometry,
      available_selections: [selection],
    })),
    excluded_items: [],
    common_selections: [selection],
  };
  const dependencies: TauriBridgeDependencies = {
    invoke: async (command) => {
      assert.equal(command, "application_preflight_nisar_subset_batch_v2");
      return preflightBatch;
    },
    getCurrentWindow: () => ({
      minimize: async () => undefined,
      toggleMaximize: async () => undefined,
      close: async () => undefined,
      innerSize: async () => ({ width: 100, height: 100 }),
      startResizeDragging: async () => undefined,
    }),
    open: async () => null,
  };
  const bridge = createTauriDesktopShellBridge(dependencies, "tauri://localhost/", false);

  const result = await bridge.application.preflightNisarSubsetBatchV2({
    source_batch_id: "nisar-batch-200",
    source_item_ids: sourceItemIds,
    expected_workbench_revision: 12,
    subset_draft_geometry: geometry,
  });

  assert.equal(result.ok, true);
  assert.equal(result.ok && result.preflight.eligible_items.length, 200);
  assert.doesNotMatch(JSON.stringify(result), /Application Service|Sentinel-2|Landsat/);
});

test("Tauri optical derived bridge validates precheck (including AOI_SCENE_DISJOINT) and task creation", async () => {
  const precheckReport = {
    ok: false,
    recommended_mode: "single_scene_clip",
    aoi_bbox_wgs84: [114.2, 30.5, 114.4, 30.7],
    aoi_center_wgs84: [114.3, 30.6],
    recommended_utm_epsg: 32650,
    crosses_utm_zones: false,
    distinct_utm_zones: [32650],
    scene_count: 1,
    intersecting_scene_count: 0,
    combined_aoi_coverage_fraction: 0,
    temporal_span_days: null,
    within_near_coincident_limit: true,
    estimated_output_pixels_per_band: 0,
    warnings: [],
    blocking_reasons: ["AOI_SCENE_DISJOINT: selected scenes do not intersect AOI"],
    scene_coverages: [
      {
        scene_id: "s2-1",
        intersects_aoi: false,
        aoi_coverage_fraction: 0,
        scene_epsg: 32650,
        acquired_at: "2026-01-01T03:00:00Z",
        cloud_cover_pct: 4.5,
      },
    ],
  };

  let lastCommand = "";
  const dependencies: TauriBridgeDependencies = {
    invoke: async (command) => {
      lastCommand = command;
      if (command === "application_precheck_optical_aoi_coverage") {
        return precheckReport;
      }
      if (command === "application_create_optical_derived_task") {
        return {
          precheck: { ...precheckReport, ok: true, intersecting_scene_count: 1, combined_aoi_coverage_fraction: 1, blocking_reasons: [] },
          task_id: "task-opt-1",
          derived_batch_id: "optderived-task-opt-1",
          derived_item_id: "item-derived-1",
          output_tif_path: "/tmp/out/S2_clip.tif",
          snapshot: {
            readiness: { contract: "v3", status: "ready", store_schema_version: 5 },
            core: { tasks: [], active_task_id: null },
          },
        };
      }
      throw new Error(`unexpected command ${command}`);
    },
    getCurrentWindow: () => ({
      minimize: async () => undefined,
      toggleMaximize: async () => undefined,
      close: async () => undefined,
      innerSize: async () => ({ width: 100, height: 100 }),
      startResizeDragging: async () => undefined,
    }),
    open: async () => null,
  };
  const bridge = createTauriDesktopShellBridge(dependencies, "tauri://localhost/", false);

  const precheckRes = await bridge.application.precheckOpticalAoiCoverage({
    source_batch_id: "batch-s2",
    expected_workbench_revision: 1,
    item_ids: ["s2-1"],
    aoi_bbox_wgs84: [114.2, 30.5, 114.4, 30.7],
  });
  assert.equal(lastCommand, "application_precheck_optical_aoi_coverage");
  assert.equal(precheckRes.ok, true);
  assert.equal(precheckRes.ok && precheckRes.report.ok, false);
  assert.equal(precheckRes.ok && precheckRes.report.recommended_utm_epsg, 32650);

  const createdRes = await bridge.application.createOpticalDerivedTask({
    request_id: "req-opt-1",
    task_id: "task-opt-1",
    source_batch_id: "batch-s2",
    expected_workbench_revision: 1,
    item_ids: ["s2-1"],
    mode: "single_scene_clip",
    aoi_bbox_wgs84: [114.2, 30.5, 114.4, 30.7],
    band_order: ["B04", "B03", "B02"],
    resampling: "bilinear",
    cloud_mask: "cloud_to_nodata",
    mosaic_strategy: "quality_first",
    overwrite: true,
    write_lineage_sidecar: true,
    output_root: "/tmp/out",
    concurrency: 2,
    created_at: "2026-11-14T00:00:00Z",
  });
  assert.equal(lastCommand, "application_create_optical_derived_task");
  assert.equal(createdRes.ok, true);
  assert.equal(createdRes.ok && createdRes.derived_batch_id, "optderived-task-opt-1");
});

test("Issue #51: Tauri bridge creates standalone orbit tasks and preserves formal_dem_download error context", async () => {
  let capturedCmd = "";
  let capturedArgs: Record<string, unknown> | undefined;
  const dependencies: TauriBridgeDependencies = {
    invoke: async (command, args) => {
      capturedCmd = command;
      capturedArgs = args;
      if (command === "application_create_standalone_orbit_task") {
        return {
          readiness: {
            contract: "application_service_readonly_v1",
            status: "store_ready",
            store_schema_version: 4,
          },
          core: {},
          aoi_display: null,
        };
      }
      if (command === "application_plan_dem_task") {
        throw {
          code: "V3_STORE_READINESS_UNSUPPORTED",
          message_key: "application_service.dem_native_gis_unavailable",
          context: "formal_dem_download",
        };
      }
      throw new Error(`unexpected command: ${command}`);
    },
    getCurrentWindow: () => ({
      minimize: async () => undefined,
      toggleMaximize: async () => undefined,
      close: async () => undefined,
      innerSize: async () => ({ width: 100, height: 100 }),
      startResizeDragging: async () => undefined,
    }),
    open: async () => null,
  };
  const bridge = createTauriDesktopShellBridge(dependencies, "tauri://localhost/", false);

  const res = await bridge.application.createStandaloneOrbitTask!({
    request_id: "orbit-req-standalone-1",
    task_id: "orbit-task-standalone-1",
    scenes: [
      {
        scene_id: "S1A_IW_SLC__1SDV_20240312T100000_20240312T100027_052946_066912_ABCD",
        acquisition_datetime: "2024-03-12T10:00:00Z",
      },
    ],
    output_root: "/tmp/orbit-out",
    use_orbit_subdir: true,
    created_at: "2026-11-24T00:00:00Z",
  });
  assert.equal(res.ok, true);
  assert.equal(capturedCmd, "application_create_standalone_orbit_task");
  assert.equal((capturedArgs as { request: { use_orbit_subdir: boolean } }).request.use_orbit_subdir, true);

  const demErr = await bridge.application.planDemTask!({
    request_id: "dem-plan-1",
    task_id: "dem-task-1",
    region_id: "r1",
    resource_id: "dem.cop30.v1",
    capability_id: "capability.dem.cop30.download.v1",
    dataset: "COP30",
    provider: "COPERNICUS_AWS",
    operation: "prepare_sarscape",
    input_path: "/tmp/dem.tif",
    output_root: "/tmp",
    created_at: "2026-11-24T00:00:00Z",
  });
  assert.equal(demErr.ok, false);
  if (!demErr.ok) {
    assert.equal(demErr.code, "V3_STORE_READINESS_UNSUPPORTED");
    assert.equal(demErr.context, "formal_dem_download");
    assert.match(demErr.error, /Native GIS Engine/);
  }
});
