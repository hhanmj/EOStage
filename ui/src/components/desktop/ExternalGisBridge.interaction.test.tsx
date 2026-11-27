import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { act } from "@testing-library/react";
import { render, cleanup, fireEvent } from "../../test-utils/render-harness";
import { I18nProvider } from "@/i18n";
import { createBrowserFixtureDesktopShellBridge } from "@/lib/desktopShellBridgeFixture";
import {
  registerTestDesktopShellBridge,
  clearTestDesktopShellBridge,
  type DesktopShellBridge,
} from "@/lib/desktopShellBridge";
import { RuntimeToolsSettingsSection } from "./PluginsAndSkillsPage";
import { DownloadTaskRow } from "./DownloadTaskRow";
import type { ApplicationTask } from "@/shell/api/types";

async function flushUiMicrotasks(): Promise<void> {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 15));
  });
}

test.afterEach(() => {
  cleanup();
  clearTestDesktopShellBridge();
  window.localStorage.clear();
});

test("1. Settings runtime_tools section renders external GIS toolchains, saves custom path, and registers RustGIS MCP connector", async () => {
  const savedPrefsCalls: Array<Record<string, unknown>> = [];
  const savedConnectors: Array<Record<string, unknown>> = [];

  const base = createBrowserFixtureDesktopShellBridge();
  let rustgisPath = "";
  const testBridge: DesktopShellBridge = {
    ...base,
    kind: "tauri",
    application: {
      ...base.application,
      getRuntimeComponents: async () => ({
        ok: true,
        install_root: "/app/runtime",
        manifest_found: true,
        app_version: "0.3.0",
        components: [
          {
            id: "eostage-core",
            name: "EOStage Core",
            kind: "core",
            version: "0.3.0",
            install_root: "/app",
            status: "ready",
            file_count: 1,
            total_bytes: 1024,
            verified: true,
            detail: "ok",
            capabilities: ["gdal"],
          },
        ],
      }),
      getPluginsStatus: async () => ({
        ok: true,
        plugins_dir: "/app/plugins",
        installed_plugins: [],
        available_plugins: [],
        custom_connectors: [],
        custom_skills: [],
      }),
      externalGisInspect: async () => ({
        ok: true,
        platform: "macos",
        prefs: {
          custom_paths: rustgisPath ? { rustgis: rustgisPath } : {},
          preferred_toolchain: rustgisPath ? "rustgis" : null,
        },
        toolchains: [
          {
            id: "rustgis",
            display_name: "RustGIS",
            installed: Boolean(rustgisPath),
            source: rustgisPath ? "custom_path" : "missing",
            executable_path: rustgisPath || null,
            gui_path: null,
            version: rustgisPath ? "rustgis 0.9.0" : null,
            mcp_capable: true,
            headless_capable: Boolean(rustgisPath),
            gui_capable: false,
            capabilities: ["version", "mcp_stdio"],
            detail: rustgisPath ? `Found ${rustgisPath}` : "Not detected",
          },
          {
            id: "geolibre",
            display_name: "GeoLibre",
            installed: false,
            source: "missing",
            executable_path: null,
            gui_path: null,
            version: null,
            mcp_capable: false,
            headless_capable: false,
            gui_capable: false,
            capabilities: ["version", "list_algorithms", "run_algorithm"],
            detail: "Not detected",
          },
          {
            id: "qgis",
            display_name: "QGIS",
            installed: false,
            source: "missing",
            executable_path: null,
            gui_path: null,
            version: null,
            mcp_capable: false,
            headless_capable: false,
            gui_capable: false,
            capabilities: ["deliver_project"],
            detail: "Not detected",
          },
          {
            id: "arcgis_pro",
            display_name: "ArcGIS Pro (arcpy)",
            installed: false,
            source: "unsupported_os",
            executable_path: null,
            gui_path: null,
            version: null,
            mcp_capable: false,
            headless_capable: false,
            gui_capable: false,
            capabilities: ["deliver_project"],
            detail: "Windows only",
          },
        ],
      }),
      externalGisSavePrefs: async (prefs: any) => {
        savedPrefsCalls.push(prefs);
        rustgisPath = prefs?.custom_paths?.rustgis || "";
        return {
          ok: true,
          platform: "macos",
          prefs: {
            custom_paths: { rustgis: rustgisPath },
            preferred_toolchain: "rustgis",
          },
          toolchains: [
            {
              id: "rustgis",
              display_name: "RustGIS",
              installed: true,
              source: "custom_path",
              executable_path: rustgisPath,
              gui_path: null,
              version: "rustgis 0.9.0",
              mcp_capable: true,
              headless_capable: true,
              gui_capable: false,
              capabilities: ["version", "mcp_stdio"],
              detail: `Found ${rustgisPath}`,
            },
          ],
        };
      },
      saveCustomConnector: async (connector: any) => {
        savedConnectors.push(connector);
        return { ok: true };
      },
    },
  };
  registerTestDesktopShellBridge(testBridge);

  const view = render(
    <I18nProvider>
      <RuntimeToolsSettingsSection
        fetchRegistryOverride={async () =>
          ({
            ok: true,
            source_url: "local",
            generated_at: "2026-11-14",
            entries: [],
          }) as any
        }
      />
    </I18nProvider>,
  );

  await flushUiMicrotasks();

  assert.ok(view.getByTestId("external-gis-panel"));
  assert.ok(view.getByTestId("external-gis-card-rustgis"));
  assert.ok(view.getByTestId("external-gis-card-qgis"));
  assert.ok(view.getByTestId("external-gis-card-arcgis_pro"));
  assert.equal(view.queryByTestId("register-rustgis-mcp-btn"), null);

  fireEvent.change(view.getByTestId("external-gis-path-input-rustgis"), {
    target: { value: "/opt/rustgis/bin/rustgis" },
  });
  fireEvent.click(view.getByTestId("external-gis-save-path-rustgis"));
  await flushUiMicrotasks();

  assert.equal(savedPrefsCalls.length, 1);
  assert.equal(
    (savedPrefsCalls[0] as any).custom_paths.rustgis,
    "/opt/rustgis/bin/rustgis",
  );

  const registerBtn = view.getByTestId("register-rustgis-mcp-btn");
  fireEvent.click(registerBtn);
  await flushUiMicrotasks();

  assert.equal(savedConnectors.length, 1);
  assert.equal(savedConnectors[0].id, "plugin-mcp-rustgis");
  assert.equal(savedConnectors[0].transport, "stdio");
  assert.equal(savedConnectors[0].endpoint, "/opt/rustgis/bin/rustgis mcp --yes");
  assert.ok(view.getByTestId("register-rustgis-mcp-btn").textContent?.includes("MCP"));
});

test("2. DownloadTaskRow delivers completed task bundle to QGIS/ArcGIS project via externalGisExecute", async () => {
  const executeCalls: Array<Record<string, unknown>> = [];
  const base = createBrowserFixtureDesktopShellBridge();
  registerTestDesktopShellBridge({
    ...base,
    kind: "tauri",
    application: {
      ...base.application,
      externalGisExecute: async (req: any) => {
        executeCalls.push(req);
        return {
          ok: true,
          toolchain: req.toolchain,
          action: req.action,
          exit_code: 0,
          stdout: "",
          stderr: "",
          output_path: "/tmp/sar_delivery/eostage_delivery.qgs",
          project_path: "/tmp/sar_delivery/eostage_delivery.qgs",
          catalog_path: "/tmp/sar_delivery/eostage_catalog.geojson",
          gui_launched: false,
          detail: "ok",
          artifacts: [
            "/tmp/sar_delivery/eostage_delivery.qgs",
            "/tmp/sar_delivery/eostage_catalog.geojson",
          ],
        };
      },
    },
  });

  const completedTask: ApplicationTask = {
    task_id: "task-ext-gis-deliver",
    revision: 1,
    name: "Sentinel-2 L2A Delivery",
    category: "scene_download",
    status: "completed",
    provider: "MPC",
    dataset: "Sentinel-2 L2A",
    created_at: "2026-11-14T00:00:00Z",
    updated_at: "2026-11-14T01:00:00Z",
    done_bytes: 1_000_000,
    total_bytes: 1_000_000,
    items_count: 1,
    completed_items_count: 1,
    output_dir: "/tmp/sar_delivery",
    scene_items: [
      {
        item_id: "i1",
        scene_id: "S2A_MSIL2A_20261101",
        status: "completed",
        done_bytes: 1_000_000,
        total_bytes: 1_000_000,
      },
    ],
  } as any;

  const rowView = render(
    <I18nProvider>
      <DownloadTaskRow task={completedTask} />
    </I18nProvider>,
  );

  fireEvent.click(rowView.getByRole("button", { name: "展开分景进度" }));
  fireEvent.click(rowView.getByTestId("deliver-external-gis-btn"));
  await flushUiMicrotasks();

  assert.equal(executeCalls.length, 1);
  assert.equal(executeCalls[0].toolchain, "qgis");
  assert.equal(executeCalls[0].action, "deliver_project");
  assert.deepEqual(executeCalls[0].input_paths, ["/tmp/sar_delivery"]);
  assert.equal(executeCalls[0].output_path, "/tmp/sar_delivery");
  assert.equal(executeCalls[0].open_gui, false);
  assert.ok(
    rowView.getByText("/tmp/sar_delivery/eostage_delivery.qgs"),
  );
});

test("3. RuntimeToolsSettingsSection renders log dehydrator + Conda environments, switches dehydrator mode, saves shared RTK path, sets default Conda env, and adds custom Conda env", async () => {
  const savedPrefsCalls: Array<Record<string, unknown>> = [];
  const base = createBrowserFixtureDesktopShellBridge();

  let currentPrefs: {
    custom_paths: Record<string, string>;
    preferred_toolchain: string | null;
    conda_custom_envs: Record<string, string>;
    preferred_conda_env: string | null;
    rtk_custom_path: string | null;
    dehydrator_mode: string;
  } = {
    custom_paths: { rustgis: "/opt/rustgis/bin/rustgis" },
    preferred_toolchain: "rustgis",
    conda_custom_envs: {},
    preferred_conda_env: null,
    rtk_custom_path: null,
    dehydrator_mode: "auto",
  };

  const buildReport = () => {
    const envs = [
      {
        id: "conda:insar",
        name: "insar",
        source: "auto",
        env_prefix: "/opt/miniforge3/envs/insar",
        python_path: "/opt/miniforge3/envs/insar/bin/python3",
        python_version: "Python 3.11.9",
        geo_packages: ["gdal", "rasterio", "h5py"],
        verified: true,
        detail: "ok",
      },
      ...Object.entries(currentPrefs.conda_custom_envs).map(([name, path]) => ({
        id: `conda:custom:${name}`,
        name,
        source: "custom",
        env_prefix: path,
        python_path: `${path}/bin/python`,
        python_version: "Python 3.12.1",
        geo_packages: ["gdal", "pyproj"],
        verified: true,
        detail: `Custom ${name}`,
      })),
    ];
    return {
      ok: true,
      platform: "macos",
      prefs: currentPrefs,
      toolchains: [
        {
          id: "rustgis",
          display_name: "RustGIS",
          installed: true,
          source: "custom_path",
          executable_path: "/opt/rustgis/bin/rustgis",
          gui_path: null,
          version: "rustgis 0.9.0",
          mcp_capable: true,
          headless_capable: true,
          gui_capable: false,
          capabilities: ["version", "mcp_stdio"],
          detail: "ok",
        },
      ],
      conda_environments: envs,
      log_dehydrator: {
        builtin_ready: true,
        mode: currentPrefs.dehydrator_mode,
        shared_rtk: {
          installed: true,
          source: currentPrefs.rtk_custom_path ? "custom_path" : "system_path",
          executable_path:
            currentPrefs.rtk_custom_path || "/opt/homebrew/bin/rtk",
          version: "rtk 0.24.0",
          verified: true,
          detail: "ok",
        },
        capabilities: [
          "strip_ansi_and_cr",
          "collapse_progress_bars",
          "fold_duplicate_warnings",
        ],
        detail: "ok",
      },
    };
  };

  registerTestDesktopShellBridge({
    ...base,
    kind: "tauri",
    application: {
      ...base.application,
      getRuntimeComponents: async () => ({
        ok: true,
        install_root: "/app/runtime",
        manifest_found: true,
        app_version: "0.3.0",
        components: [],
      }),
      getPluginsStatus: async () => ({
        ok: true,
        plugins_dir: "/app/plugins",
        installed_plugins: [],
        available_plugins: [],
        custom_connectors: [],
        custom_skills: [],
      }),
      externalGisInspect: async () => buildReport(),
      externalGisSavePrefs: async (prefs: any) => {
        savedPrefsCalls.push(prefs);
        currentPrefs = {
          custom_paths: prefs?.custom_paths || {},
          preferred_toolchain: prefs?.preferred_toolchain ?? null,
          conda_custom_envs: prefs?.conda_custom_envs || {},
          preferred_conda_env: prefs?.preferred_conda_env ?? null,
          rtk_custom_path: prefs?.rtk_custom_path ?? null,
          dehydrator_mode: prefs?.dehydrator_mode || "auto",
        };
        return buildReport();
      },
    },
  });

  const view = render(
    <I18nProvider>
      <RuntimeToolsSettingsSection
        fetchRegistryOverride={async () =>
          ({
            ok: true,
            source_url: "local",
            generated_at: "2026-11-16",
            entries: [],
          }) as any
        }
      />
    </I18nProvider>,
  );

  await flushUiMicrotasks();

  assert.ok(view.getByTestId("log-dehydrator-card"));
  assert.ok(view.getByTestId("external-conda-section"));
  assert.ok(view.getByTestId("conda-env-card-insar"));

  fireEvent.click(view.getByTestId("dehydrator-mode-builtin"));
  await flushUiMicrotasks();
  assert.equal(savedPrefsCalls[0].dehydrator_mode, "builtin_only");
  assert.equal(
    (savedPrefsCalls[0].custom_paths as Record<string, string>).rustgis,
    "/opt/rustgis/bin/rustgis",
  );

  fireEvent.change(view.getByTestId("shared-rtk-path-input"), {
    target: { value: "/usr/local/bin/rtk" },
  });
  fireEvent.click(view.getByTestId("shared-rtk-save-btn"));
  await flushUiMicrotasks();
  assert.equal(savedPrefsCalls[1].rtk_custom_path, "/usr/local/bin/rtk");

  fireEvent.click(view.getByTestId("conda-set-default-insar"));
  await flushUiMicrotasks();
  assert.equal(savedPrefsCalls[2].preferred_conda_env, "insar");
  assert.ok(view.getByTestId("conda-default-badge-insar"));

  fireEvent.change(view.getByTestId("conda-custom-alias-input"), {
    target: { value: "mintpy" },
  });
  fireEvent.change(view.getByTestId("conda-custom-path-input"), {
    target: { value: "/opt/conda/envs/mintpy" },
  });
  fireEvent.click(view.getByTestId("conda-add-custom-btn"));
  await flushUiMicrotasks();

  assert.deepEqual(savedPrefsCalls[3].conda_custom_envs, {
    mintpy: "/opt/conda/envs/mintpy",
  });
  assert.equal(savedPrefsCalls[3].preferred_conda_env, "insar");
  assert.equal(
    (savedPrefsCalls[3].custom_paths as Record<string, string>).rustgis,
    "/opt/rustgis/bin/rustgis",
  );
  assert.ok(view.getByTestId("conda-env-card-mintpy"));
});
