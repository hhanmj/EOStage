import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import React from "react";
import { act, within } from "@testing-library/react";
import { render, cleanup, fireEvent } from "../../test-utils/render-harness";
import { I18nProvider, LOCALE_STORAGE_KEY, setCurrentLocale, useI18n } from "../../i18n";
import { resolveAgentToolDisplayName } from "../../shell/api/agentLogic";
import { classifyStreamEvents } from "../../shell/api/agentStreamLogic";
import { AccountsSettingsTab } from "../../shell/components/settings/AccountsSettingsTab";
import {
  clearTestDesktopShellBridge,
  registerTestDesktopShellBridge,
} from "../../lib/desktopShellBridge";
import {
  GEE_QUOTA_CONFIGURATION_URL,
  clearUnpinnedGeeMapLayers,
  formatLocalizedGeeTierLabel,
  getGeeLayerStoreSnapshot,
  pushGeeMapLayer,
  resetGeeLayerStoreForTest,
  setGeeQueryInvokerForTest,
  setGeeSwipeState,
} from "../../lib/geeCloudSuiteClient";
import { GeeScriptConfirmCard } from "./GeeScriptConfirmCard";
import { GeeLayerStackHud, GeeMapViewportAndSwipeSync } from "./GeeLayerStackAndSwipe";
import { MapScopeContext } from "../map/leafletBindings";
import { GeeWorkbenchPanel, extractGeeErrorMessage } from "./GeeCatalogAutocomplete";
import { updateWorkbenchToolOptions, getWorkbenchToolOptions } from "./ManualConditionsPanel";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

test.afterEach(() => {
  cleanup();
  resetGeeLayerStoreForTest();
  setGeeQueryInvokerForTest(null);
  clearTestDesktopShellBridge();
  updateWorkbenchToolOptions({
    geeAssetId: "ALL",
    geeTheme: "all",
    geeSelectedPresetId: "science.gee.catalog.v1",
  });
  try {
    window.localStorage.removeItem(LOCALE_STORAGE_KEY);
  } catch {
    // Ignore storage errors in synthetic environments
  }
  setCurrentLocale("zh-CN");
});

async function settle(ms = 40) {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, ms));
  });
}

test("1. classifyStreamEvents classifies gee_artifact stream events cleanly", () => {
  const items = classifyStreamEvents([
    {
      seq: 1,
      kind: "gee_artifact" as any,
      payload: {
        run_id: "run-gee-1",
        call_id: "call-1",
        tool_id: "gee_script_inspect",
        tool_args: { script: "RESULT = {'ok': 1}" },
        summary: { ok: true, approval_id: "gee-appr-123", scientific_warnings: ["定标提醒"] },
        script_preview: "RESULT = {'ok': 1}",
      },
    },
  ]);
  assert.equal(items.length, 1);
  assert.equal(items[0].kind, "gee_artifact");
  if (items[0].kind === "gee_artifact") {
    assert.equal(items[0].tool_id, "gee_script_inspect");
    assert.equal(items[0].summary.approval_id, "gee-appr-123");
    assert.equal(items[0].script_preview, "RESULT = {'ok': 1}");
  }
});

test("2. GeeScriptConfirmCard approves custom Python ee script and renders executed artifact", async () => {
  const calls: Array<Record<string, any>> = [];
  setGeeQueryInvokerForTest(async (req) => {
    calls.push(req);
    if (req.dataset === "gee_script_execute") {
      return {
        ok: true,
        artifact_id: "gee-art-script1",
        result: { mean: 0.5123, threshold_area_km2: 18.42 },
      };
    }
    if (req.dataset === "gee_artifact_get") {
      return {
        artifact_id: "gee-art-script1",
        title: "GEE 自定义脚本计算结果",
        dataset_id: "CUSTOM_EE_SCRIPT",
        operation: "script_execute",
        bbox: [117.2, 31.8, 117.3, 31.9],
        preview_data_url:
          "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==",
        vis_min: 0,
        vis_max: 1,
        palette: ["#000000", "#00ff00"],
        unit: "NDVI",
        stats: { mean: 0.5123, threshold_area_km2: 18.42 },
        created_at: 1700000000,
      };
    }
    return { ok: true };
  });

  const view = render(
    <GeeScriptConfirmCard
      toolId="gee_script_inspect"
      toolArgs={{ bbox: [117.2, 31.8, 117.3, 31.9] }}
      summary={{
        ok: true,
        approval_id: "gee-appr-demo99",
        scientific_warnings: ["检测到 Landsat Collection 2 数据集：请确认尺度因子"],
        blocked_reasons: [],
        env_ready: true,
      }}
      scriptPreview={"img = ee.Image('LANDSAT/LC09/C02/T1_L2/001')\nRESULT = {'mean': 0.5}"}
    />
  );

  await settle(30);
  assert.ok(view.getByTestId("gee-script-confirm-card"));
  assert.ok(view.getByText(/检测到 Landsat Collection 2 数据集/));

  const approveBtn = view.getByTestId("gee-approve-run-script-btn");
  await act(async () => {
    fireEvent.click(approveBtn);
  });
  await settle(60);

  const execCall = calls.find((c) => c.dataset === "gee_script_execute");
  assert.ok(execCall);
  assert.equal(execCall.gee_params.approval_id, "gee-appr-demo99");
  assert.ok(view.getByTestId("gee-artifact-card"));
  assert.ok(view.getByText(/18\.420 km²/));
});

test("3. Quota guardrail card never silently downsamples and dispatches full-resolution mosaic choice", async () => {
  const calls: Array<Record<string, any>> = [];
  setGeeQueryInvokerForTest(async (req) => {
    calls.push(req);
    return {
      ok: true,
      quota_confirm_required: false,
      file_name: "gee_COPERNICUS_S2_NDVI_10m.tif",
      bytes_written: 4_194_304,
      chunks_stitched: 16,
      stats: { mean: 0.44 },
    };
  });

  const view = render(
    <GeeScriptConfirmCard
      toolId="gee_chunked_download"
      toolArgs={{ asset_id: "COPERNICUS/S2_SR_HARMONIZED", index_name: "NDVI" }}
      summary={{
        ok: true,
        quota_confirm_required: true,
        asset_id: "COPERNICUS/S2_SR_HARMONIZED",
        grid_plan: {
          bbox: [117.0, 31.6, 117.4, 32.0],
          requested_scale_m: 10,
          suggested_preview_scale_m: 30,
          full_width: 2048,
          full_height: 2048,
          total_chunks: 16,
          estimated_mb: 16.0,
        },
      }}
    />
  );

  assert.ok(view.getByTestId("gee-quota-confirm-card"));
  const fullBtn = view.getByTestId("gee-confirm-full-res-btn");
  await act(async () => {
    fireEvent.click(fullBtn);
  });
  await settle(50);

  assert.equal(calls.length, 1);
  assert.equal(calls[0].dataset, "gee_chunked_download");
  assert.equal(calls[0].gee_params.confirm_full_resolution, true);
  assert.equal(calls[0].gee_params.use_preview_scale, false);
  assert.ok(view.getByTestId("gee-artifact-card"));
});

test("4. GeeLayerStackHud manages multi-layer stack pinning and split-screen swipe comparison", async () => {
  pushGeeMapLayer(
    {
      id: "layer-before",
      title: "巢湖 NDWI · 2024 枯水期",
      datasetId: "COPERNICUS/S2_SR_HARMONIZED",
      operation: "before",
      bbox: [117.2, 31.4, 117.9, 31.8],
      tileUrlTemplate: "https://earthengine.googleapis.com/v1/projects/demo/maps/m1/tiles/{z}/{x}/{y}",
      visMin: -1,
      visMax: 1,
      palette: ["#d73027", "#1a9850"],
      unit: "NDWI",
      pinned: true,
    },
    false
  );
  pushGeeMapLayer(
    {
      id: "layer-after",
      title: "巢湖 NDWI · 2025 丰水期",
      datasetId: "COPERNICUS/S2_SR_HARMONIZED",
      operation: "after",
      bbox: [117.2, 31.4, 117.9, 31.8],
      tileUrlTemplate: "https://earthengine.googleapis.com/v1/projects/demo/maps/m2/tiles/{z}/{x}/{y}",
      visMin: -1,
      visMax: 1,
      palette: ["#d73027", "#1a9850"],
      unit: "NDWI",
      pinned: false,
    },
    false
  );
  setGeeSwipeState({
    enabled: true,
    positionPct: 50,
    leftLayerId: "layer-before",
    rightLayerId: "layer-after",
  });

  const view = render(<GeeLayerStackHud />);
  const divider = view.getByTestId("gee-swipe-divider");
  assert.equal(divider.getAttribute("aria-valuenow"), "50");

  await act(async () => {
    fireEvent.keyDown(divider, { key: "ArrowRight" });
  });
  assert.equal(getGeeLayerStoreSnapshot().swipe.positionPct, 55);

  // Source switch clears unpinned layer while keeping pinned layer intact
  await act(async () => {
    clearUnpinnedGeeMapLayers();
  });
  const snap = getGeeLayerStoreSnapshot();
  assert.equal(snap.layers.length, 1);
  assert.equal(snap.layers[0].id, "layer-before");
  assert.equal(snap.swipe.enabled, false);
});

test("5. GeeWorkbenchPanel supports bilingual search and live pre-flight scene check", async () => {
  let reqCount = 2;
  setGeeQueryInvokerForTest(async (req) => {
    if (req.dataset === "gee_quota_status") {
      return {
        ok: true,
        configured: true,
        project_id: "ee-demo-workbench",
        registration_state: "REGISTERED_NOT_COMMERCIALLY",
        account_tier: "noncommercial_community",
        account_tier_label: "非商业科研档 (Community)",
        monthly_limit_eecu_seconds: 540000,
        monthly_limit_eecu_hours: 150,
        daily_limit_eecu_seconds: null,
        read_requests_per_minute: 6000,
        parallelism_restricted: false,
        last_quota_status_header: null,
        cloud_monitoring: {
          unlocked: false,
          reason: "billing_not_enabled",
          monthly_used_eecu_seconds: null,
          monthly_used_eecu_hours: null,
          monthly_remaining_eecu_hours: null,
        },
        local_ledger: {
          month_utc: "2026-11",
          total_requests: reqCount,
          analysis_runs: 1,
          chunk_downloads: 0,
          script_runs: 0,
          profiled_eecu_seconds: 1.8,
          has_eecu_profile: true,
          total_wall_seconds: 3.2,
          last_call_eecu_seconds: 1.8,
          last_call_wall_seconds: 3.2,
          last_updated_at_ms: 1794492000000,
        },
        fetched_at_ms: 1794492000000,
      };
    }
    if (req.dataset === "gee_catalog_search") {
      return {
        ok: true,
        items: [
          {
            id: "COPERNICUS/S2_SR_HARMONIZED",
            title_zh: "Sentinel-2 地表反射率",
            title_en: "Harmonized Sentinel-2 MSI",
            source: "official",
            asset_type: "image_collection",
            native_scale_m: 10,
          },
          {
            id: "projects/sat-io/open-datasets/ORNL/LANDSCAN_GLOBAL",
            title_zh: "LandScan 全球人口分布 (社区库)",
            title_en: "LandScan Global Population",
            source: "community",
            asset_type: "image_collection",
            native_scale_m: 1000,
          },
        ],
      };
    }
    if (req.dataset === "gee_check_availability") {
      reqCount += 1;
      return {
        ok: true,
        asset_id: req.gee_params.asset_id,
        title_zh: "Sentinel-2 地表反射率",
        asset_type: "image_collection",
        total_scenes: 28,
        cloud_filtered_scenes: 11,
        native_scale_m: 10,
        grid_plan: {
          bbox: [117.2, 31.8, 117.3, 31.9],
          requested_scale_m: 10,
          effective_scale_m: 10,
          suggested_preview_scale_m: 20,
          scale_x_deg: 0.00009,
          scale_y_deg: -0.00009,
          full_width: 800,
          full_height: 800,
          chunk_size_px: 512,
          chunks_x: 2,
          chunks_y: 2,
          total_chunks: 4,
          estimated_mb: 2.5,
          quota_confirm_required: false,
        },
      };
    }
    return { ok: true };
  });

  const view = render(
    <GeeWorkbenchPanel
      workingAoi={{ west: 117.2, south: 31.8, east: 117.3, north: 31.9, crs: "EPSG:4326" }}
      aoiLabel="合肥市蜀山区"
      startDate="2025-05-01"
      endDate="2025-08-31"
      onStartDateChange={() => {}}
      onEndDateChange={() => {}}
    />
  );

  await settle(180);
  assert.ok(view.getByTestId("gee-workbench-quota-strip"));
  assert.ok(view.getByText(/非商业科研档 \(Community\) · 上限 150h\/月/));
  assert.ok(view.getByText(/软件内本月 2 次 \/ 1\.8 EECU-s/));
  assert.ok(view.getByText("社区库 · 1000m"));

  const preflightBtn = view.getByTestId("gee-preflight-btn");
  await act(async () => {
    fireEvent.click(preflightBtn);
  });
  await settle(60);

  assert.ok(view.getByText(/总计 28 景，云量过滤后可用 11 景/));
  assert.ok(view.getByText(/软件内本月 3 次 \/ 1\.8 EECU-s/));
});

test("6. AccountsSettingsTab (#account-card-gee) opens portal cleanly with or without Tauri bridge and saves + verifies credentials", async () => {
  const calls: Array<Record<string, any>> = [];
  let configured = false;

  const handleGeodataQuery = async (req: Record<string, any>) => {
    calls.push(req);
    if (req.dataset === "gee_account_status") {
      return {
        ok: true,
        configured,
        gee_ready: configured,
        project_id: configured ? "ee-demo-proj-01" : null,
        client_email: configured ? "runner@ee-demo-proj-01.iam.gserviceaccount.com" : null,
      };
    }
    if (req.dataset === "gee_quota_status") {
      return {
        ok: true,
        configured,
        project_id: configured ? "ee-demo-proj-01" : null,
        registration_state: configured ? "REGISTERED_NOT_COMMERCIALLY" : null,
        account_tier: configured ? "noncommercial_community" : "unconfigured",
        account_tier_label: configured ? "非商业科研档 (Community)" : "未配置",
        monthly_limit_eecu_seconds: configured ? 540000 : null,
        monthly_limit_eecu_hours: configured ? 150 : null,
        daily_limit_eecu_seconds: null,
        read_requests_per_minute: configured ? 6000 : null,
        parallelism_restricted: false,
        last_quota_status_header: null,
        cloud_monitoring: {
          unlocked: false,
          reason: configured ? "billing_not_enabled" : "unconfigured",
          monthly_used_eecu_seconds: null,
          monthly_used_eecu_hours: null,
          monthly_remaining_eecu_hours: null,
        },
        local_ledger: {
          month_utc: "2026-11",
          total_requests: 0,
          analysis_runs: 0,
          chunk_downloads: 0,
          script_runs: 0,
          profiled_eecu_seconds: 0,
          has_eecu_profile: false,
          total_wall_seconds: 0,
          last_call_eecu_seconds: null,
          last_call_wall_seconds: null,
          last_updated_at_ms: null,
        },
        fetched_at_ms: 1794492000000,
      };
    }
    if (req.dataset === "gee_account_save") {
      configured = true;
      return {
        ok: true,
        project_id: req.code,
        message: `凭据已加密保存（项目 ${req.code}）`,
      };
    }
    if (req.dataset === "gee_account_verify") {
      return {
        ok: true,
        latency_ms: 82,
        message_zh: "连通成功（延迟 82 ms）",
      };
    }
    return { ok: true };
  };

  // (a) Unregistered desktop bridge: clicking「获取密钥官网」never throws and falls back to window.open
  clearTestDesktopShellBridge();
  const openedUrls: Array<{ url: string; target?: string; features?: string }> = [];
  const origOpen = window.open;
  window.open = ((url?: string | URL, target?: string, features?: string) => {
    openedUrls.push({ url: String(url ?? ""), target, features });
    return null;
  }) as typeof window.open;

  try {
    const view = render(<AccountsSettingsTab />);
    await settle(40);
    const geeCard = view.container.querySelector("#account-card-gee") as HTMLElement;
    assert.ok(geeCard, "expected #account-card-gee in AccountsSettingsTab");

    const portalBtn = within(geeCard).getByTestId("account-portal-link-gee");
    await act(async () => {
      fireEvent.click(portalBtn);
    });
    assert.equal(openedUrls.length, 1);
    assert.equal(openedUrls[0].url, "https://code.earthengine.google.com/");
    assert.equal(openedUrls[0].target, "_blank");
    assert.equal(openedUrls[0].features, "noopener,noreferrer");

    // (b) Registered desktop bridge: clicking「account-portal-link-gee」calls bridge.application.openExternalUrl
    const bridgeOpened: string[] = [];
    registerTestDesktopShellBridge({
      kind: "tauri",
      application: {
        openExternalUrl: async (url: string) => {
          bridgeOpened.push(url);
          return { ok: true, url };
        },
        openGeodataQuery: handleGeodataQuery,
      },
    } as any);

    await act(async () => {
      fireEvent.click(portalBtn);
    });
    await settle(20);
    assert.deepEqual(bridgeOpened, ["https://code.earthengine.google.com/"]);
    assert.equal(openedUrls.length, 1);

    // (c) Real click flow on production #account-card-gee to expand, fill Project ID + JSON, save, and verify
    const expandBtn = within(geeCard).getByTestId("gee-account-expand-btn");
    await act(async () => {
      fireEvent.click(expandBtn);
    });

    const projectInput = within(geeCard).getByTestId("gee-project-id") as HTMLInputElement;
    const secretTextarea = within(geeCard).getByTestId("gee-secret-input") as HTMLTextAreaElement;
    await act(async () => {
      fireEvent.change(projectInput, { target: { value: "ee-demo-proj-01" } });
      fireEvent.change(secretTextarea, {
        target: { value: '{"type":"service_account","project_id":"ee-demo-proj-01"}' },
      });
    });

    const saveBtn = within(geeCard).getByTestId("gee-account-save-btn");
    await act(async () => {
      fireEvent.click(saveBtn);
    });
    await settle(60);

    const saveCall = calls.find((c) => c.dataset === "gee_account_save");
    assert.ok(saveCall, "expected gee_account_save call");
    assert.equal(saveCall.code, "ee-demo-proj-01");
    assert.ok(within(geeCard).getByText("已配置"));
    assert.ok(within(geeCard).getByText(/凭据已加密保存（项目 ee-demo-proj-01）/));

    const verifyBtn = within(geeCard).getByTestId("gee-account-verify-btn");
    await act(async () => {
      fireEvent.click(verifyBtn);
    });
    await settle(60);

    const verifyCall = calls.find((c) => c.dataset === "gee_account_verify");
    assert.ok(verifyCall, "expected gee_account_verify call");
    assert.equal(verifyCall.code, "ee-demo-proj-01");
    assert.ok(within(geeCard).getByText(/连通成功（延迟 82 ms）/));
  } finally {
    window.open = origOpen;
  }
});

test("7. GeeScriptConfirmCard renders time_series windows with dates, scene counts, bars, and mean values", async () => {
  setGeeQueryInvokerForTest(async (req) => {
    if (req.dataset === "gee_artifact_get") {
      return {
        artifact_id: "gee-ts-art-01",
        title: "合肥蜀山区 NDVI 时间序列",
        dataset_id: "COPERNICUS/S2_SR_HARMONIZED",
        operation: "time_series",
        bbox: [117.2, 31.8, 117.3, 31.9],
        vis_min: -1,
        vis_max: 1,
        palette: [],
        unit: "NDVI",
        stats: {
          mean: 0.5118,
          series: [
            {
              window_start: "2025-04-01",
              window_end: "2025-05-01",
              scene_count: 8,
              mean: 0.382,
            },
            {
              window_start: "2025-05-01",
              window_end: "2025-06-01",
              scene_count: 12,
              stats: { mean: 0.6415, min: 0.12, max: 0.89 },
            },
            {
              window_start: "2025-06-01",
              window_end: "2025-07-01",
              scene_count: 0,
              mean: null,
            },
          ],
        },
        created_at: 1700000100,
      };
    }
    return { ok: true };
  });

  const view = render(
    <GeeScriptConfirmCard
      toolId="gee_cloud_analyze"
      toolArgs={{ asset_id: "COPERNICUS/S2_SR_HARMONIZED", operation: "time_series", index_name: "NDVI" }}
      summary={{
        ok: true,
        artifact_id: "gee-ts-art-01",
        operation: "time_series",
      }}
    />
  );

  await settle(60);

  assert.ok(view.getByTestId("gee-artifact-card"));
  assert.ok(view.getByTestId("gee-time-series-table"));
  assert.ok(view.getByText("时间序列分段统计（共 3 期）"));
  assert.ok(view.getByText("2025-04-01 ~ 2025-05-01"));
  assert.ok(view.getByText("8 景"));
  assert.ok(view.getByText("0.3820"));
  assert.ok(view.getByText("2025-05-01 ~ 2025-06-01"));
  assert.ok(view.getByText("12 景"));
  assert.ok(view.getByText("0.6415"));
  assert.ok(view.getByText("2025-06-01 ~ 2025-07-01"));
  assert.ok(view.getByText("0 景"));
  assert.ok(view.getByText("无影像"));
  assert.ok(view.getByTestId("gee-time-series-bar-0"));
  assert.ok(view.getByTestId("gee-time-series-bar-1"));
});

test("8. All GEE desktop UI surfaces comply 100% with Settings Token Rules A-J", () => {
  const targetFiles = [
    "GeeCatalogAutocomplete.tsx",
    "GeeScriptConfirmCard.tsx",
    "GeeLayerStackAndSwipe.tsx",
  ];

  const REGEX_A = /\brounded-control\b/g;
  const REGEX_B = /\btext-\[(?:9|10|10\.5|11|11\.5)px\](?![\w-])/g;
  const REGEX_C = /(?:#[0-9A-Fa-f]{6}\b|rounded-\[|shadow-\[|bg-\[#|text-\[#)/g;
  const REGEX_D =
    /(?:(?<![\w-])(?:[a-z0-9-]+:)*(?:(?:bg|text|border|ring|divide|from|to|via|placeholder|outline|fill|stroke)-(?:slate|gray|zinc|neutral|stone|blue|sky|indigo|red|rose|green|emerald|amber|orange|yellow|lime|teal|cyan|violet|purple|pink|fuchsia)-\d[\w\/-]*|bg-white\b|bg-black\b|text-black\b|border-white\b|border-black\b))/g;
  const REGEX_E_TRIGGER = /(?<![\w-])text-white\b/;
  const REGEX_E_ALLOWED = /\b(?:bg-sp-accent-fill|bg-sp-danger-strong)\b/;
  const REGEX_F =
    /(?<![\w-])(?:[a-z0-9-]+:)?(?:rounded-(?:lg|xl|2xl|3xl|md|sm)\b|rounded(?=[\s"'`]))/g;
  const REGEX_G =
    /(?<![\w-])(?:[a-z0-9-]+:)?(?:shadow-(?:sm|md|lg|xl|2xl|inner)\b|shadow(?=[\s"'`]))/g;
  const REGEX_H =
    /(?<=^|[\s:"'-])(?:gap|gap-x|gap-y|p|px|py|pt|pb|pl|pr|m|mx|my|mt|mb|ml|mr|space-x|space-y)-(?:0\.5|1\.5|2\.5|3\.5)\b/g;
  const REGEX_I =
    /(?<![\w-])(?:[a-z0-9-]+:)?font-(?:medium|light|extralight|thin|extrabold|black)\b/g;

  const violations: string[] = [];

  for (const fileName of targetFiles) {
    const targetFile = path.resolve(__dirname, fileName);
    const content = fs.readFileSync(targetFile, "utf8");
    const lines = content.split(/\r?\n/);

    lines.forEach((lineText, idx) => {
      const ln = idx + 1;
      for (const [rule, rx] of [
        ["A", REGEX_A],
        ["B", REGEX_B],
        ["C", REGEX_C],
        ["D", REGEX_D],
        ["F", REGEX_F],
        ["G", REGEX_G],
        ["H", REGEX_H],
        ["I", REGEX_I],
      ] as const) {
        rx.lastIndex = 0;
        let m: RegExpExecArray | null;
        while ((m = rx.exec(lineText)) !== null) {
          violations.push(`${fileName}:${ln} [Rule ${rule}]: ${m[0]}`);
        }
      }
      if (REGEX_E_TRIGGER.test(lineText) && !REGEX_E_ALLOWED.test(lineText)) {
        violations.push(`${fileName}:${ln} [Rule E]: text-white without allowed bg token`);
      }
    });

    // Rule J: control font size >= 13px on <label|input|select|textarea|button>
    const tagRegex = /<(label|input|select|textarea|button)\b/g;
    const sizeRegex = /\btext-(?:xs\b|\[(?:9|10|10\.5|11|11\.5|12)px\](?![\w-]))/g;
    let tm: RegExpExecArray | null;
    while ((tm = tagRegex.exec(content)) !== null) {
      let i = tm.index + tm[0].length;
      let inQuote: string | null = null;
      let braceDepth = 0;
      let tagEnd = -1;
      while (i < content.length) {
        const ch = content[i];
        if (inQuote) {
          if (ch === "\\") {
            i += 2;
            continue;
          }
          if (ch === inQuote) inQuote = null;
        } else {
          if (ch === '"' || ch === "'" || ch === "`") inQuote = ch;
          else if (ch === "{") braceDepth++;
          else if (ch === "}" && braceDepth > 0) braceDepth--;
          else if (ch === ">" && braceDepth === 0) {
            tagEnd = i;
            break;
          }
        }
        i++;
      }
      if (tagEnd !== -1) {
        const tagText = content.slice(tm.index, tagEnd + 1);
        sizeRegex.lastIndex = 0;
        let sm: RegExpExecArray | null;
        while ((sm = sizeRegex.exec(tagText)) !== null) {
          const ln = content.slice(0, tm.index + sm.index).split(/\r?\n/).length;
          violations.push(`${fileName}:${ln} [Rule J]: <${tm[1]} ... ${sm[0]}>`);
        }
      }
    }
  }

  // Also confirm dead duplicate GeeAccountSettingsCard is removed
  const catalogSrc = fs.readFileSync(
    path.resolve(__dirname, "GeeCatalogAutocomplete.tsx"),
    "utf8"
  );
  assert.ok(
    !catalogSrc.includes("GeeAccountSettingsCard"),
    "duplicate GeeAccountSettingsCard must be removed from GeeCatalogAutocomplete.tsx"
  );

  assert.deepEqual(violations, []);
});

test("9. GeeMapViewportAndSwipeSync clears both clip and clipPath when swipe is disabled or unmounted", () => {
  const leftPane = document.createElement("div");
  const rightPane = document.createElement("div");
  const fakeMap = {
    getPane: (name: string) => {
      if (name === "gee-swipe-left-pane") return leftPane;
      if (name === "gee-swipe-right-pane") return rightPane;
      return null;
    },
    getSize: () => {
      throw new Error("headless-dom-fallback");
    },
    on: () => {},
    off: () => {},
  };

  const view = render(
    <MapScopeContext.Provider value={{ map: fakeMap as any }}>
      <GeeMapViewportAndSwipeSync swipeEnabled={true} positionPct={40} zoomTarget={null} />
    </MapScopeContext.Provider>
  );

  assert.equal(leftPane.style.clipPath, "inset(0 60% 0 0)");
  assert.equal(rightPane.style.clipPath, "inset(0 0 0 40%)");

  view.rerender(
    <MapScopeContext.Provider value={{ map: fakeMap as any }}>
      <GeeMapViewportAndSwipeSync swipeEnabled={false} positionPct={40} zoomTarget={null} />
    </MapScopeContext.Provider>
  );

  assert.equal(leftPane.style.clip, "");
  assert.equal(rightPane.style.clip, "");
  assert.equal(leftPane.style.clipPath, "");
  assert.equal(rightPane.style.clipPath, "");
});

test("10. AccountsSettingsTab (#account-card-gee) renders cardless Community 150h badge, local EECU ledger, and live refresh", async () => {
  const calls: Array<Record<string, any>> = [];
  let refreshed = false;
  const openedUrls: string[] = [];

  registerTestDesktopShellBridge({
    kind: "tauri",
    application: {
      openExternalUrl: async (url: string) => {
        openedUrls.push(url);
        return { ok: true, url };
      },
      openGeodataQuery: async (req: Record<string, any>) => {
        calls.push(req);
        if (req.dataset === "gee_account_status") {
          return {
            ok: true,
            configured: true,
            gee_ready: true,
            project_id: "ee-demo-community",
          };
        }
        if (req.dataset === "gee_quota_status") {
          if (req.gee_params?.refresh === true) {
            refreshed = true;
          }
          return {
            ok: true,
            configured: true,
            project_id: "ee-demo-community",
            registration_state: "REGISTERED_NOT_COMMERCIALLY",
            account_tier: "noncommercial_community",
            account_tier_label: "非商业科研档 (Community)",
            monthly_limit_eecu_seconds: 540000,
            monthly_limit_eecu_hours: 150,
            daily_limit_eecu_seconds: null,
            read_requests_per_minute: 6000,
            parallelism_restricted: false,
            last_quota_status_header: null,
            cloud_monitoring: {
              unlocked: false,
              reason: "billing_not_enabled",
              monthly_used_eecu_seconds: null,
              monthly_used_eecu_hours: null,
              monthly_remaining_eecu_hours: null,
            },
            local_ledger: {
              month_utc: "2026-11",
              total_requests: refreshed ? 8 : 7,
              analysis_runs: 3,
              chunk_downloads: 1,
              script_runs: 0,
              profiled_eecu_seconds: refreshed ? 5.5 : 4.25,
              has_eecu_profile: true,
              total_wall_seconds: 12.4,
              last_call_eecu_seconds: 1.25,
              last_call_wall_seconds: 2.1,
              last_updated_at_ms: 1794492000000,
            },
            fetched_at_ms: 1794492000000,
          };
        }
        return { ok: true };
      },
    },
  } as any);

  const view = render(<AccountsSettingsTab />);
  await settle(60);

  const geeCard = view.container.querySelector("#account-card-gee") as HTMLElement;
  assert.ok(geeCard);

  // Header quota badge shows Community tier + 150 hours monthly cap
  const tierBadge = within(geeCard).getByTestId("gee-quota-tier-badge");
  assert.match(tierBadge.textContent || "", /非商业科研档 \(Community\) · 每月 150 小时/);

  // Quota & Compute Ledger panel shows official cap, read rate, normal status, and cardless safe-mode note
  assert.ok(within(geeCard).getByText(/每月 150 计算小时上限/));
  assert.ok(within(geeCard).getByText(/6000 次\/分钟/));
  assert.ok(within(geeCard).getByText("正常未限流"));
  const cardlessNote = within(geeCard).getByTestId("gee-cloud-quota-cardless-note");
  assert.equal(
    cardlessNote.textContent,
    "当前为免绑卡免费科研模式（零扣费风险）；若项目关联谷歌云结算账户，将自动解锁全账号实时剩余小时进度条。"
  );
  assert.equal(within(geeCard).queryByTestId("gee-cloud-quota-progressbar"), null);

  // Local ledger shows accurate profiled EECU-seconds + wall-clock seconds
  const localLedger = within(geeCard).getByTestId("gee-local-usage-ledger");
  assert.match(localLedger.textContent || "", /本月软件内统计 \(2026-11\)/);
  assert.match(localLedger.textContent || "", /已执行请求 7 次（分析 3 · 分块下载 1 · 脚本 0）/);
  assert.match(localLedger.textContent || "", /实测云端算力 4\.25 EECU-秒/);
  assert.match(localLedger.textContent || "", /累计计算耗时 12\.4 秒/);

  // Clicking 官方额度配置 opens official Google Cloud Earth Engine configuration URL
  const configBtn = within(geeCard).getByTestId("gee-open-quota-config-btn");
  assert.ok(configBtn.className.includes("bg-transparent"));
  await act(async () => {
    fireEvent.click(configBtn);
  });
  await settle(20);
  assert.deepEqual(openedUrls, [GEE_QUOTA_CONFIGURATION_URL]);

  // Clicking 刷新额度 dispatches gee_quota_status with { refresh: true } and updates DOM
  const refreshBtn = within(geeCard).getByTestId("gee-refresh-quota-btn");
  assert.ok(refreshBtn.className.includes("bg-transparent"));
  await act(async () => {
    fireEvent.click(refreshBtn);
  });
  await settle(60);

  const refreshCalls = calls.filter(
    (c) => c.dataset === "gee_quota_status" && c.gee_params?.refresh === true
  );
  assert.equal(refreshCalls.length, 1);
  assert.match(within(geeCard).getByTestId("gee-local-usage-ledger").textContent || "", /已执行请求 8 次/);
  assert.match(within(geeCard).getByTestId("gee-local-usage-ledger").textContent || "", /实测云端算力 5\.50 EECU-秒/);
});

test("11. AccountsSettingsTab (#account-card-gee) auto-unlocks real remaining-hours progress bar when Cloud Billing monitoring is active", async () => {
  registerTestDesktopShellBridge({
    kind: "tauri",
    application: {
      openGeodataQuery: async (req: Record<string, any>) => {
        if (req.dataset === "gee_account_status") {
          return {
            ok: true,
            configured: true,
            gee_ready: true,
            project_id: "ee-demo-contributor",
          };
        }
        if (req.dataset === "gee_quota_status") {
          return {
            ok: true,
            configured: true,
            project_id: "ee-demo-contributor",
            registration_state: "REGISTERED_NOT_COMMERCIALLY",
            account_tier: "noncommercial_contributor",
            account_tier_label: "进阶科研档 (Contributor)",
            monthly_limit_eecu_seconds: 3600000,
            monthly_limit_eecu_hours: 1000,
            daily_limit_eecu_seconds: null,
            read_requests_per_minute: 6000,
            parallelism_restricted: true,
            last_quota_status_header: "parallelism_restricted=true",
            cloud_monitoring: {
              unlocked: true,
              reason: null,
              monthly_used_eecu_seconds: 451800,
              monthly_used_eecu_hours: 125.5,
              monthly_remaining_eecu_hours: 874.5,
            },
            local_ledger: {
              month_utc: "2026-11",
              total_requests: 12,
              analysis_runs: 9,
              chunk_downloads: 2,
              script_runs: 1,
              profiled_eecu_seconds: 0,
              has_eecu_profile: false,
              total_wall_seconds: 38.6,
              last_call_eecu_seconds: null,
              last_call_wall_seconds: 4.2,
              last_updated_at_ms: 1794492000000,
            },
            fetched_at_ms: 1794492000000,
          };
        }
        return { ok: true };
      },
    },
  } as any);

  const view = render(<AccountsSettingsTab />);
  await settle(60);

  const geeCard = view.container.querySelector("#account-card-gee") as HTMLElement;
  assert.ok(geeCard);

  // Header shows Contributor 1000h badge and parallelism throttle warning badge
  assert.match(
    within(geeCard).getByTestId("gee-quota-tier-badge").textContent || "",
    /进阶科研档 \(Contributor\) · 每月 1000 小时/
  );
  assert.equal(within(geeCard).getByTestId("gee-quota-throttle-badge").textContent, "云端并发降速中");
  assert.ok(within(geeCard).getByText("超额降速保护中"));

  // Cardless note disappears and real remaining progress bar unlocks automatically
  assert.equal(within(geeCard).queryByTestId("gee-cloud-quota-cardless-note"), null);
  const unlockedBox = within(geeCard).getByTestId("gee-cloud-quota-unlocked");
  assert.match(
    unlockedBox.textContent || "",
    /全账号本月已用 125\.5 \/ 1000 小时 · 剩余 874\.5 小时 \(87\.5%\)/
  );
  const progressBar = within(geeCard).getByTestId("gee-cloud-quota-progressbar");
  assert.equal(progressBar.getAttribute("aria-valuemax"), "1000");
  assert.equal(progressBar.getAttribute("aria-valuenow"), "125.5");

  // When has_eecu_profile is false, local ledger honestly reports wall-clock seconds only (never fakes EECU-seconds)
  const localLedgerText = within(geeCard).getByTestId("gee-local-usage-ledger").textContent || "";
  assert.match(localLedgerText, /累计计算耗时 38\.6 秒/);
  assert.ok(!localLedgerText.includes("EECU-秒"));
});

test("12. GeeWorkbenchPanel quota strip displays unlocked remaining hours and triggers manual refresh", async () => {
  const calls: Array<Record<string, any>> = [];
  let remaining = 874.5;

  setGeeQueryInvokerForTest(async (req) => {
    calls.push(req);
    if (req.dataset === "gee_quota_status") {
      if (req.gee_params?.refresh === true) {
        remaining -= 2.5;
      }
      return {
        ok: true,
        configured: true,
        project_id: "ee-demo-wb-unlocked",
        registration_state: "REGISTERED_NOT_COMMERCIALLY",
        account_tier: "noncommercial_contributor",
        account_tier_label: "进阶科研档 (Contributor)",
        monthly_limit_eecu_seconds: 3600000,
        monthly_limit_eecu_hours: 1000,
        daily_limit_eecu_seconds: null,
        read_requests_per_minute: 6000,
        parallelism_restricted: false,
        last_quota_status_header: null,
        cloud_monitoring: {
          unlocked: true,
          reason: null,
          monthly_used_eecu_seconds: (1000 - remaining) * 3600,
          monthly_used_eecu_hours: 1000 - remaining,
          monthly_remaining_eecu_hours: remaining,
        },
        local_ledger: {
          month_utc: "2026-11",
          total_requests: 5,
          analysis_runs: 5,
          chunk_downloads: 0,
          script_runs: 0,
          profiled_eecu_seconds: 2.4,
          has_eecu_profile: true,
          total_wall_seconds: 9.1,
          last_call_eecu_seconds: 0.5,
          last_call_wall_seconds: 1.8,
          last_updated_at_ms: 1794492000000,
        },
        fetched_at_ms: 1794492000000,
      };
    }
    if (req.dataset === "gee_catalog_search") {
      return { ok: true, items: [] };
    }
    return { ok: true };
  });

  const view = render(
    <GeeWorkbenchPanel
      workingAoi={{ west: 117.2, south: 31.8, east: 117.3, north: 31.9, crs: "EPSG:4326" }}
      aoiLabel="合肥市蜀山区"
      startDate="2025-05-01"
      endDate="2025-08-31"
      onStartDateChange={() => {}}
      onEndDateChange={() => {}}
    />
  );

  await settle(60);
  const strip = view.getByTestId("gee-workbench-quota-strip");
  assert.match(strip.textContent || "", /进阶科研档 \(Contributor\) · 上限 1000h\/月/);
  assert.match(strip.textContent || "", /剩余 874\.5h/);

  const refreshBtn = view.getByTestId("gee-workbench-refresh-quota-btn");
  assert.ok(refreshBtn.className.includes("bg-transparent"));
  await act(async () => {
    fireEvent.click(refreshBtn);
  });
  await settle(60);

  const manualRefreshes = calls.filter(
    (c) => c.dataset === "gee_quota_status" && c.gee_params?.refresh === true
  );
  assert.equal(manualRefreshes.length, 1);
  assert.match(view.getByTestId("gee-workbench-quota-strip").textContent || "", /剩余 872h/);

  // Credential change event from Settings Modal refreshes workbench quota live
  await act(async () => {
    window.dispatchEvent(new window.Event("eostage:credentials-changed"));
  });
  await settle(60);
  const refreshesAfterCredEvent = calls.filter(
    (c) => c.dataset === "gee_quota_status" && c.gee_params?.refresh === true
  );
  assert.equal(refreshesAfterCredEvent.length, 2);
  assert.match(view.getByTestId("gee-workbench-quota-strip").textContent || "", /剩余 869\.5h/);
});

test("13. DesktopWorkbench clears unpinned GEE layers when switching out of gee scientific mode", () => {
  const wbFile = path.resolve(__dirname, "./DesktopWorkbench.tsx");
  const wbContent = fs.readFileSync(wbFile, "utf8");
  const modeHandlerIdx = wbContent.indexOf("onScientificModeChange={(nextMode) => {");
  assert.ok(modeHandlerIdx > 0, "DesktopWorkbench must wire onScientificModeChange");
  const modeHandlerSlice = wbContent.slice(modeHandlerIdx, modeHandlerIdx + 320);
  assert.ok(
    modeHandlerSlice.includes("clearUnpinnedGeeMapLayers();"),
    "onScientificModeChange must call clearUnpinnedGeeMapLayers() so unpinned GEE layers do not stay stuck when switching to DEM/SAR/Orbit/Convert"
  );
});

test("14. [P1-2] GeeScriptConfirmCard hydrates uncorrupted script via gee_script_preview, blocks premature redacted click, resets across approvalId updates, and executes without GEE_SCRIPT_HASH_MISMATCH", async () => {
  const canonicalScript =
    "# 光学/雷达融合\nb8 = 0.62\nb4 = 0.14\ndenom = b8 + b4\nndvi = (b8 - b4)/denom\nRESULT = {'ndvi': round(ndvi, 4)}";
  const secondCanonicalScript =
    "# 第二版水体/雷达脚本\nb3 = 0.41\nb8 = 0.12\nndwi = (b3 - b8)/(b3 + b8)\nRESULT = {'ndwi': round(ndwi, 4)}";
  // Mirrors insar_agent::history::outbound_text rewriting slash-containing tokens to `[local-reference]`
  const scrubbedByOutboundText =
    "# [local-reference]\nb8 = 0.62\nb4 = 0.14\ndenom = b8 + b4\nndvi = (b8 - [local-reference]\nRESULT = {'ndvi': round(ndvi, 4)}";

  let resolveFirstPreview: ((v: any) => void) | null = null;
  const calls: Array<Record<string, any>> = [];
  setGeeQueryInvokerForTest(async (req) => {
    calls.push(req);
    if (req.dataset === "gee_script_preview") {
      if (req.gee_params.approval_id === "gee-appr-slash-ndvi") {
        return await new Promise((resolve) => {
          resolveFirstPreview = resolve;
        });
      }
      if (req.gee_params.approval_id === "gee-appr-second-ndwi") {
        return {
          ok: true,
          approval_id: "gee-appr-second-ndwi",
          script: secondCanonicalScript,
          script_sha256: "mock-sha256-second",
        };
      }
    }
    if (req.dataset === "gee_script_execute") {
      const expected =
        req.gee_params.approval_id === "gee-appr-second-ndwi"
          ? secondCanonicalScript
          : canonicalScript;
      if (req.gee_params.script !== expected) {
        throw new Error("GEE_SCRIPT_HASH_MISMATCH: 待运行代码与通过安全审查的版本不一致");
      }
      return {
        ok: true,
        artifact_id: "gee-art-ndvi-ok",
        result: { mean: 0.5472 },
      };
    }
    return { ok: true };
  });

  const view = render(
    <GeeScriptConfirmCard
      toolId="gee_script_inspect"
      toolArgs={{ bbox: [117.2, 31.8, 117.3, 31.9] }}
      summary={{
        ok: true,
        approval_id: "gee-appr-slash-ndvi",
        scientific_warnings: [],
        blocked_reasons: [],
        env_ready: true,
      }}
      scriptPreview={scrubbedByOutboundText}
    />
  );

  // Synchronous frame-0 check (BEFORE passive effects settle): `[local-reference]` is NEVER flashed and button is disabled
  assert.ok(!view.getByTestId("gee-script-code-preview").textContent?.includes("[local-reference]"));
  assert.equal((view.getByTestId("gee-approve-run-script-btn") as HTMLButtonElement).disabled, true);

  await settle(20);
  // While gee_script_preview is still pending, `[local-reference]` remains hidden and button remains disabled
  assert.ok(!view.getByTestId("gee-script-code-preview").textContent?.includes("[local-reference]"));
  assert.equal((view.getByTestId("gee-approve-run-script-btn") as HTMLButtonElement).disabled, true);

  await act(async () => {
    resolveFirstPreview?.({
      ok: true,
      approval_id: "gee-appr-slash-ndvi",
      script: canonicalScript,
      script_sha256: "mock-sha256-canonical",
    });
  });
  await settle(30);

  // Human reviewer sees real `# 光学/雷达融合` and `ndvi = (b8 - b4)/denom`, never `[local-reference]`
  const codePre = view.getByTestId("gee-script-code-preview");
  assert.ok(codePre.textContent?.includes("# 光学/雷达融合"));
  assert.ok(codePre.textContent?.includes("ndvi = (b8 - b4)/denom"));
  assert.ok(!codePre.textContent?.includes("[local-reference]"));
  assert.equal((view.getByTestId("gee-approve-run-script-btn") as HTMLButtonElement).disabled, false);

  // Prop update to a new `approval_id` resets `resolvedScript` immediately and hydrates the new script
  view.rerender(
    <GeeScriptConfirmCard
      toolId="gee_script_inspect"
      toolArgs={{ bbox: [117.2, 31.8, 117.3, 31.9] }}
      summary={{
        ok: true,
        approval_id: "gee-appr-second-ndwi",
        scientific_warnings: [],
        blocked_reasons: [],
        env_ready: true,
      }}
      scriptPreview={scrubbedByOutboundText}
    />
  );
  await settle(40);
  assert.ok(view.getByTestId("gee-script-code-preview").textContent?.includes("ndwi = (b3 - b8)/(b3 + b8)"));

  const approveBtn = view.getByTestId("gee-approve-run-script-btn");
  await act(async () => {
    fireEvent.click(approveBtn);
  });
  await settle(50);

  const execCall = calls.find((c) => c.dataset === "gee_script_execute");
  assert.ok(execCall, "gee_script_execute must be invoked");
  assert.equal(execCall.gee_params.approval_id, "gee-appr-second-ndwi");
  assert.equal(execCall.gee_params.script, secondCanonicalScript);
  assert.equal(view.queryByRole("alert"), null, "zero GEE_SCRIPT_HASH_MISMATCH error");
  assert.ok(view.getByTestId("gee-artifact-card"));
  assert.ok(view.getByText(/0\.5472/));
});

test("15. [P2-2] GEE UI surfaces, all 8 backend quota tiers, #account-card-gee actions, and live locale switching localize cleanly across en and ja", async () => {
  // Verify all 8 backend `account_tier` variants never leak Chinese fallback strings in `en` or `ja`
  const allBackendTiers: Array<[string, string]> = [
    ["noncommercial_community", "非商业科研档 (Community)"],
    ["noncommercial_contributor", "进阶科研档 (Contributor)"],
    ["noncommercial_partner", "合作科研档 (Partner)"],
    ["noncommercial_unlimited", "非商业科研账号"],
    ["noncommercial_unknown", "非商业科研账号"],
    ["commercial", "商业计费档 (Commercial)"],
    ["unknown", "已配置 GEE 项目"],
    ["unconfigured", "未配置"],
  ];
  for (const [tier, zhFallback] of allBackendTiers) {
    const enLabel = formatLocalizedGeeTierLabel(tier, zhFallback, "en");
    const jaLabel = formatLocalizedGeeTierLabel(tier, zhFallback, "ja");
    const zhLabel = formatLocalizedGeeTierLabel(tier, zhFallback, "zh-CN");
    assert.equal(zhLabel, zhFallback);
    assert.ok(!/[\u4e00-\u9fff]/.test(enLabel), `en tier label leaked CJK for ${tier}: ${enLabel}`);
    assert.notEqual(jaLabel, zhFallback, `ja tier label fell back to raw zh-CN for ${tier}: ${jaLabel}`);
  }

  registerTestDesktopShellBridge({
    kind: "tauri",
    application: {
      openGeodataQuery: async (req: Record<string, any>) => {
        if (req.dataset === "gee_catalog_search") {
          return { ok: true, items: [] };
        }
        if (req.dataset === "gee_account_status") {
          return { ok: true, configured: true, gee_ready: true, project_id: "ee-i18n-demo" };
        }
        if (req.dataset === "gee_account_verify") {
          // Real Rust backend returns Chinese `message` & `message_zh`
          return {
            ok: true,
            message: "GEE 官方认证接口校验通过 (Project: ee-i18n-demo)",
            message_zh: "GEE 官方认证接口校验通过 (Project: ee-i18n-demo)",
          };
        }
        if (req.dataset === "gee_check_availability") {
          return {
            ok: true,
            total_scenes: 3,
            cloud_filtered_scenes: 3,
            bands: ["B4", "B3", "B2"],
            native_scale_m: 10,
            grid_plan: { total_chunks: 1, full_width: 256, full_height: 256, estimated_mb: 0.4 },
          };
        }
        if (req.dataset === "gee_cloud_analyze") {
          return {
            ok: true,
            artifact_id: "art-live-locale",
            bbox: [117.2, 31.8, 117.3, 31.9],
            tile_url_template: "https://earthengine.googleapis.com/v1/projects/demo/maps/m2/tiles/{z}/{x}/{y}",
          };
        }
        if (req.dataset === "gee_artifact_get") {
          throw new Error("force fallback title from workbench");
        }
        if (req.dataset === "gee_quota_status") {
          return {
            ok: true,
            configured: true,
            project_id: "ee-i18n-demo",
            account_tier: "noncommercial_partner",
            account_tier_label: "合作科研档 (Partner)",
            monthly_limit_eecu_hours: 150,
            read_requests_per_minute: 6000,
            parallelism_restricted: false,
            cloud_monitoring: { unlocked: false },
            local_ledger: {
              month_utc: "2026-11",
              total_requests: 2,
              analysis_runs: 2,
              chunk_downloads: 0,
              script_runs: 0,
              profiled_eecu_seconds: 1.5,
              has_eecu_profile: true,
              total_wall_seconds: 4.0,
            },
            fetched_at_ms: 1794492000000,
          };
        }
        return { ok: true };
      },
    },
  } as any);

  pushGeeMapLayer(
    {
      id: "layer-i18n-1",
      title: "Sentinel-2 NDVI",
      datasetId: "COPERNICUS/S2_SR_HARMONIZED",
      operation: "index",
      bbox: [117.2, 31.8, 117.3, 31.9],
      tileUrlTemplate: "https://earthengine.googleapis.com/v1/projects/demo/maps/m1/tiles/{z}/{x}/{y}",
      visMin: 0,
      visMax: 1,
      palette: ["#000000", "#00ff00"],
      unit: "NDVI",
      pinned: false,
    },
    false
  );

  function LocaleSwitchHarness() {
    const { setLocale } = useI18n();
    return (
      <div>
        <button type="button" data-testid="switch-to-en" onClick={() => setLocale("en")}>
          en
        </button>
        <button type="button" data-testid="switch-to-ja" onClick={() => setLocale("ja")}>
          ja
        </button>
        <GeeWorkbenchPanel
          workingAoi={null}
          aoiLabel=""
          startDate="2025-05-01"
          endDate="2025-06-01"
          onStartDateChange={() => {}}
          onEndDateChange={() => {}}
        />
        <GeeScriptConfirmCard
          toolId="gee_script_inspect"
          summary={{ ok: true, approval_id: "appr-en", env_ready: true, blocked_reasons: [] }}
          scriptPreview="RESULT = {'ok': 1}"
        />
        <GeeLayerStackHud />
        <AccountsSettingsTab />
      </div>
    );
  }

  // Mount in `zh-CN`, then live-switch to `en` and `ja` without unmounting!
  window.localStorage.setItem(LOCALE_STORAGE_KEY, "zh-CN");
  const liveView = render(
    <I18nProvider>
      <LocaleSwitchHarness />
    </I18nProvider>
  );
  await settle(60);

  // Switch live to English (`en`)
  await act(async () => {
    fireEvent.click(liveView.getByTestId("switch-to-en"));
  });
  await settle(40);

  assert.equal(resolveAgentToolDisplayName("gee_cloud_analyze"), "GEE Live Cloud Analysis");
  assert.ok(liveView.getByText("GEE Catalog Search (Official + Community)"));
  assert.ok(liveView.getByText("Approve & Run Script"));
  assert.ok(liveView.getByText("GEE Cloud Layer Stack (1)"));
  assert.ok(liveView.getByText(/Partner Research \(Partner\) · 150h \/ month/));
  assert.ok(liveView.getByText(/Cardless free research mode active/));

  // Verify #account-card-gee Test Connection feedback and status badge have zero leaked Chinese in `en`
  const geeCard = liveView.container.querySelector("#account-card-gee") as HTMLElement;
  assert.ok(within(geeCard).getByText("Configured"));
  await act(async () => {
    fireEvent.click(within(geeCard).getByTestId("gee-account-expand-btn"));
  });
  await act(async () => {
    fireEvent.click(within(geeCard).getByTestId("gee-account-verify-btn"));
  });
  await settle(40);
  assert.equal(within(geeCard).getByTestId("gee-feedback-msg").textContent, "GEE connection verified");

  // Owner Round-5 Item 8 (/grill-me): Analyze & Download require preflight pass first
  await act(async () => {
    fireEvent.click(liveView.getByTestId("gee-preflight-btn"));
  });
  await settle(30);

  // Verify live locale switch updated GeeWorkbenchPanel default dataset title dynamically (not frozen at zh-CN mount!)
  await act(async () => {
    fireEvent.click(liveView.getByTestId("gee-workbench-analyze-btn"));
  });
  await settle(40);
  const pushedEn = getGeeLayerStoreSnapshot().layers.find((l) => l.id === "art-live-locale");
  assert.equal(pushedEn?.title, "Harmonized Sentinel-2 Surface Reflectance");

  // Switch live to Japanese (`ja`)
  await act(async () => {
    fireEvent.click(liveView.getByTestId("switch-to-ja"));
  });
  await settle(40);

  assert.equal(resolveAgentToolDisplayName("gee_cloud_analyze"), "GEE クラウドリアルタイム解析");
  assert.ok(liveView.getByText("GEE 全カタログ検索（公式＋コミュニティ）"));
  assert.ok(liveView.getByText("承認してスクリプトを実行"));
  assert.ok(liveView.getByText("GEE クラウド解析レイヤースタック (2)"));
  assert.ok(liveView.getByText(/パートナー研究枠 \(Partner\) · 月間 150 時間/));
  assert.ok(liveView.getByText(/現在はカード登録不要の無料研究モード/));
  assert.ok(within(geeCard).getByText("設定済み"));
});

test("16. [P2-NEW-1] GeeScriptConfirmCard resets post-execution scriptDoneResult and cross-session artifactId/artifact state on prop updates, and DesktopAgentWorkbench keys cards by session/call identity", async () => {
  const calls: Array<Record<string, any>> = [];
  setGeeQueryInvokerForTest(async (req) => {
    calls.push(req);
    if (req.dataset === "gee_script_preview") {
      return {
        ok: true,
        approval_id: req.gee_params.approval_id,
        script:
          req.gee_params.approval_id === "gee-appr-session-b"
            ? "img_b = ee.Image(2)\nRESULT = {'session': 'B'}"
            : "img_a = ee.Image(1)\nRESULT = {'session': 'A'}",
        script_sha256: `sha-${req.gee_params.approval_id}`,
      };
    }
    if (req.dataset === "gee_script_execute") {
      return {
        ok: true,
        artifact_id: "art-script-a",
        result: { mean: 0.9123 },
      };
    }
    if (req.dataset === "gee_artifact_get") {
      if (req.gee_params.artifact_id === "art-hefei") {
        return {
          artifact_id: "art-hefei",
          title: "合肥 NDVI 分析结果",
          operation: "index",
          bbox: [117.1, 31.7, 117.4, 32.0],
          preview_data_url: "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=",
          tile_url_template: "https://earthengine.googleapis.com/v1/projects/p/maps/hefei/tiles/{z}/{x}/{y}",
          stats: { mean: 0.6111 },
        };
      }
      if (req.gee_params.artifact_id === "art-xinjiang") {
        return {
          artifact_id: "art-xinjiang",
          title: "新疆戈壁 NDVI 分析结果",
          operation: "index",
          bbox: [87.5, 43.7, 87.8, 44.0],
          preview_data_url: "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=",
          tile_url_template: "https://earthengine.googleapis.com/v1/projects/p/maps/xinjiang/tiles/{z}/{x}/{y}",
          stats: { mean: 0.1222 },
        };
      }
    }
    return { ok: true };
  });

  // Part A: Execute script in Session A (`scriptDoneResult` set), then update props to unapproved Session B script
  const view = render(
    <GeeScriptConfirmCard
      toolId="gee_script_inspect"
      toolArgs={{ bbox: [117.1, 31.7, 117.4, 32.0] }}
      summary={{ ok: true, approval_id: "gee-appr-session-a", env_ready: true }}
      scriptPreview="img_a = ee.Image(1)"
    />
  );
  await settle(25);
  await act(async () => {
    fireEvent.click(view.getByTestId("gee-approve-run-script-btn"));
  });
  await settle(30);
  assert.ok(view.getByTestId("gee-artifact-card"), "Session A executed result renders artifact card");

  // Rerender in-place to Session B's unapproved `gee_script_inspect` card
  view.rerender(
    <GeeScriptConfirmCard
      toolId="gee_script_inspect"
      toolArgs={{ bbox: [87.5, 43.7, 87.8, 44.0] }}
      summary={{ ok: true, approval_id: "gee-appr-session-b", env_ready: true }}
      scriptPreview="# [local-reference]"
    />
  );
  // Synchronously resets `scriptDoneResult` -> renders confirmation card (NOT Session A's artifact card) without flashing `[local-reference]`
  assert.ok(view.getByTestId("gee-script-confirm-card"));
  assert.equal(view.queryByTestId("gee-artifact-card"), null);
  assert.ok(!view.getByTestId("gee-script-code-preview").textContent?.includes("[local-reference]"));
  assert.equal((view.getByTestId("gee-approve-run-script-btn") as HTMLButtonElement).disabled, true);
  await settle(30);
  assert.ok(view.getByTestId("gee-script-code-preview").textContent?.includes("img_b = ee.Image(2)"));

  // Part B: Cross-session `gee_cloud_analyze` artifact_id switch (`art-hefei` -> `art-xinjiang`) never leaks Hefei data or map layer
  view.rerender(
    <GeeScriptConfirmCard
      toolId="gee_cloud_analyze"
      toolArgs={{ operation: "index" }}
      summary={{ ok: true, artifact_id: "art-hefei", operation: "index" }}
    />
  );
  await settle(30);
  assert.ok(view.getByText("合肥 NDVI 分析结果"));
  assert.ok(view.getByText(/0\.6111/));

  view.rerender(
    <GeeScriptConfirmCard
      toolId="gee_cloud_analyze"
      toolArgs={{ operation: "index" }}
      summary={{ ok: true, artifact_id: "art-xinjiang", operation: "index" }}
    />
  );
  // Synchronously drops stale `art-hefei` record on frame 0
  assert.equal(view.queryByText("合肥 NDVI 分析结果"), null);
  await settle(30);
  assert.ok(view.getByText("新疆戈壁 NDVI 分析结果"));
  assert.ok(view.getByText(/0\.1222/));

  resetGeeLayerStoreForTest();
  await act(async () => {
    fireEvent.click(view.getByTestId("gee-push-to-map-btn"));
  });
  const mapLayers = getGeeLayerStoreSnapshot().layers;
  assert.equal(mapLayers.length, 1);
  assert.equal(mapLayers[0].id, "art-xinjiang");

  // Part C: Source guard verifying DesktopAgentWorkbench keys GeeScriptConfirmCard by session + run + call + seq
  const workbenchSrc = fs.readFileSync(
    path.resolve(__dirname, "./DesktopAgentWorkbench.tsx"),
    "utf8"
  );
  assert.match(
    workbenchSrc,
    /<GeeScriptConfirmCard\s+key=\{`\$\{activeSessionId \|\| "local"\}:\$\{item\.run_id\}:\$\{item\.call_id\}:\$\{item\.seq\}`\}/,
    "DesktopAgentWorkbench must key GeeScriptConfirmCard by session/run/call/seq identity"
  );
});

test("17. GeeWorkbenchPanel unifies GEE dataset selection, adapts operators, offers QGIS Add Layer to Map & neutral preflight card", async () => {
  const calls: Array<Record<string, any>> = [];
  setGeeQueryInvokerForTest(async (req) => {
    calls.push(req);
    if (req.dataset === "gee_quota_status") {
      return { ok: true, configured: false, account_tier: "unconfigured", account_tier_label: "未配置", parallelism_restricted: false, cloud_monitoring: { unlocked: false }, local_ledger: { month_utc: "2026-11", total_requests: 0, analysis_runs: 0, chunk_downloads: 0, script_runs: 0, profiled_eecu_seconds: 0, has_eecu_profile: false, total_wall_seconds: 0 }, fetched_at_ms: 1794492000000 };
    }
    if (req.dataset === "gee_catalog_search") {
      return {
        ok: true,
        items: [
          { id: "COPERNICUS/S1_GRD", title_zh: "Sentinel-1 SAR GRD", title_en: "Sentinel-1 SAR GRD", source: "official", asset_type: "image_collection", native_scale_m: 10 },
          { id: "COPERNICUS/DEM/GLO30", title_zh: "Copernicus DEM 30m", title_en: "Copernicus DEM GLO30", source: "official", asset_type: "image_collection", native_scale_m: 30 },
        ],
      };
    }
    if (req.dataset === "gee_check_availability") {
      return {
        ok: true,
        asset_id: req.gee_params.asset_id,
        title_zh: "Sentinel-1 SAR GRD",
        asset_type: "image_collection",
        start_date: "2025-04-01",
        end_date: "2025-06-30",
        bands: ["VV", "VH", "angle"],
        total_scenes: 14,
        cloud_filtered_scenes: 14,
        native_scale_m: 10,
        grid_plan: {
          bbox: [117.2, 31.8, 117.3, 31.9],
          requested_scale_m: 10,
          effective_scale_m: 10,
          suggested_preview_scale_m: 20,
          scale_x_deg: 0.00009,
          scale_y_deg: -0.00009,
          full_width: 600,
          full_height: 600,
          chunk_size_px: 512,
          chunks_x: 2,
          chunks_y: 2,
          total_chunks: 4,
          estimated_mb: 1.8,
          quota_confirm_required: false,
        },
      };
    }
    if (req.dataset === "gee_cloud_analyze") {
      return {
        ok: true,
        artifact_id: "gee-art-tile-preview",
        title: `在线瓦片 · ${req.gee_params.asset_id}`,
        operation: req.gee_params.operation,
        bbox: req.gee_params.bbox,
        tile_url_template: "https://earthengine.googleapis.com/v1/projects/demo/maps/s1/tiles/{z}/{x}/{y}",
        vis_min: -25,
        vis_max: 0,
        palette: [],
        unit: "dB",
        stats: { mean: -11.2 },
      };
    }
    return { ok: true };
  });

  // Pre-seed workbenchToolOptions as if DataSourceCatalogModal selected COPERNICUS/DEM/GLO30
  updateWorkbenchToolOptions({ geeAssetId: "COPERNICUS/DEM/GLO30", geeSelectedPresetId: "science.gee.dem-glo30.v1" });

  const view = render(
    <GeeWorkbenchPanel
      workingAoi={{ west: 117.2, south: 31.8, east: 117.3, north: 31.9, crs: "EPSG:4326" }}
      aoiLabel="合肥市"
      startDate="2025-04-01"
      endDate="2025-06-30"
      onStartDateChange={() => {}}
      onEndDateChange={() => {}}
    />
  );
  await settle(160);

  // Active dataset card & adaptive DEM operators (no optical NDVI or maxCloud slider)
  const activeCard = view.getByTestId("gee-active-dataset-card");
  assert.ok(activeCard.textContent?.includes("COPERNICUS/DEM/GLO30"));
  assert.ok(within(activeCard).getByText("高程 DEM"));
  const opSelect = view.getByTestId("gee-analysis-mode-select") as HTMLButtonElement;
  assert.ok(opSelect.textContent?.includes("高程基准成图"));
  act(() => {
    fireEvent.click(opSelect);
  });
  const demOpts = view.getAllByRole("option");
  assert.strictEqual(demOpts.length, 1);
  act(() => {
    fireEvent.click(opSelect);
  });

  // External update to SAR updates selected asset & operators dynamically
  act(() => {
    updateWorkbenchToolOptions({ geeAssetId: "COPERNICUS/S1_GRD", geeSelectedPresetId: "science.gee.s1-grd.v1" });
  });
  await settle(30);
  const sarCard = view.getByTestId("gee-active-dataset-card");
  assert.ok(sarCard.textContent?.includes("COPERNICUS/S1_GRD"));
  assert.ok(within(sarCard).getByText("雷达 SAR"));
  act(() => {
    fireEvent.click(opSelect);
  });
  assert.ok(
    view.getAllByRole("option").some((o) => o.textContent?.includes("SAR") || o.textContent?.includes("变化"))
  );
  act(() => {
    fireEvent.click(opSelect);
  });

  // Neutral monochrome preflight icon (no Sparkles text-sp-accent blue accent)
  const preflightBtn = view.getByTestId("gee-preflight-btn");
  assert.strictEqual(preflightBtn.innerHTML.includes("text-sp-accent"), false);

  // Run preflight -> structured card shows date span, bands, and 1-click "叠加默认合成到地图"
  await act(async () => {
    fireEvent.click(preflightBtn);
  });
  await settle(40);
  const preflightCard = view.getByTestId("gee-preflight-card");
  assert.ok(preflightCard.textContent?.includes("VV, VH, angle"));
  assert.ok(preflightCard.textContent?.includes("2025-04-01 ~ 2025-06-30"));

  // Strict 3-step progression: composite / analyze calls operation="composite" and mounts layer onto map
  await act(async () => {
    fireEvent.click(view.getByTestId("gee-workbench-analyze-btn"));
  });
  await settle(40);
  const analyzeCall = calls.find((c) => c.dataset === "gee_cloud_analyze");
  assert.ok(analyzeCall);
  assert.strictEqual(analyzeCall.gee_params.asset_id, "COPERNICUS/S1_GRD");
  assert.strictEqual(analyzeCall.gee_params.operation, "composite");
  const mounted = getGeeLayerStoreSnapshot().layers;
  assert.strictEqual(mounted.length, 1);
  assert.strictEqual(
    mounted[0].tileUrlTemplate,
    "https://earthengine.googleapis.com/v1/projects/demo/maps/s1/tiles/{z}/{x}/{y}"
  );
});

test("18. extractGeeErrorMessage unpacks structured errors without '[object Object]' & >64 chunk error exposes inline preview-scale recovery", async () => {
  // Pure unpacking tests
  assert.strictEqual(
    extractGeeErrorMessage({ code: "GEE_AUTH", message_zh: "请先配置 GEE 服务账号密钥" }),
    "[GEE_AUTH] 请先配置 GEE 服务账号密钥"
  );
  assert.strictEqual(
    extractGeeErrorMessage('{"message_zh":"研究区超过 64 块","code":"GEE_AOI_TOO_LARGE_FOR_SINGLE_DOWNLOAD"}'),
    "[GEE_AOI_TOO_LARGE_FOR_SINGLE_DOWNLOAD] 研究区超过 64 块"
  );
  assert.strictEqual(extractGeeErrorMessage({ nested: { detail: "quota limit" } }).includes("[object Object]"), false);
  assert.strictEqual(extractGeeErrorMessage("[object Object]").includes("[object Object]"), false);

  const dlCalls: Array<Record<string, any>> = [];
  setGeeQueryInvokerForTest(async (req) => {
    if (req.dataset === "gee_check_availability") {
      return {
        ok: true,
        total_scenes: 4,
        cloud_filtered_scenes: 4,
        native_scale_m: 10,
        bands: ["B4", "B3", "B2"],
        grid_plan: { total_chunks: 144, full_width: 4000, full_height: 4000, estimated_mb: 120, suggested_preview_scale_m: 40 },
      };
    }
    if (req.dataset === "gee_chunked_download") {
      dlCalls.push(req.gee_params);
      if (!req.gee_params.use_preview_scale) {
        return Promise.reject({
          code: "GEE_AOI_TOO_LARGE_FOR_SINGLE_DOWNLOAD",
          message_zh: "网格块数 144 超过单次下载上限 64 块，请改用预览分辨率下载或缩小研究区",
          grid_plan: { total_chunks: 144, suggested_preview_scale_m: 40 },
        });
      }
      return {
        ok: true,
        quota_confirm_required: false,
        chunks_stitched: 9,
        file_name: "s2_preview_40m.tif",
        bytes_written: 1_024_000,
      };
    }
    return { ok: false, items: [] };
  });

  const view = render(
    <GeeWorkbenchPanel
      workingAoi={{ west: 116.0, south: 31.0, east: 118.0, north: 33.0, crs: "EPSG:4326" }}
      aoiLabel="大区域"
      startDate="2025-04-01"
      endDate="2025-06-30"
      onStartDateChange={() => {}}
      onEndDateChange={() => {}}
    />
  );

  await act(async () => {
    fireEvent.click(view.getByTestId("gee-preflight-btn"));
  });
  await settle(30);

  await act(async () => {
    fireEvent.click(view.getByTestId("gee-workbench-download-btn"));
  });
  await settle(40);

  // Must NEVER display [object Object]
  const note = view.getByTestId("gee-workbench-status-note");
  assert.strictEqual(note.textContent?.includes("[object Object]"), false);
  assert.ok(note.textContent?.includes("网格块数 144"));

  // Inline recovery card shows both preview-scale retry and narrow-AOI button
  const recoveryCard = view.getByTestId("gee-too-large-recovery-card");
  assert.ok(recoveryCard);
  const retryPreviewBtn = view.getByTestId("gee-retry-preview-scale-btn");
  assert.ok(retryPreviewBtn.textContent?.includes("按推荐预览分辨率下载"));

  await act(async () => {
    fireEvent.click(retryPreviewBtn);
  });
  await settle(40);
  assert.strictEqual(dlCalls.length, 2);
  assert.strictEqual(dlCalls[1].use_preview_scale, true);
  assert.strictEqual(view.queryByTestId("gee-too-large-recovery-card"), null);
});

test("19. Slow GEE requests show live elapsed timer after 1s and active 中止 cancel drops late results", async () => {
  let resolveSlow!: (v: unknown) => void;
  setGeeQueryInvokerForTest(async (req) => {
    if (req.dataset === "gee_check_availability") {
      return {
        ok: true,
        total_scenes: 2,
        cloud_filtered_scenes: 2,
        native_scale_m: 10,
        bands: ["B4", "B3", "B2"],
        grid_plan: { total_chunks: 1, full_width: 512, full_height: 512, estimated_mb: 1.5 },
      };
    }
    if (req.dataset === "gee_cloud_analyze") {
      return new Promise((resolve) => {
        resolveSlow = resolve;
      });
    }
    return { ok: false, items: [] };
  });

  const view = render(
    <GeeWorkbenchPanel
      workingAoi={{ west: 117.2, south: 31.8, east: 117.3, north: 31.9, crs: "EPSG:4326" }}
      aoiLabel="合肥"
      startDate="2025-04-01"
      endDate="2025-06-30"
      onStartDateChange={() => {}}
      onEndDateChange={() => {}}
    />
  );

  await act(async () => {
    fireEvent.click(view.getByTestId("gee-preflight-btn"));
  });
  await settle(30);

  await act(async () => {
    fireEvent.click(view.getByTestId("gee-workbench-analyze-btn"));
  });
  assert.ok(view.getByTestId("gee-action-progress-bar"));
  assert.ok(view.getByTestId("gee-action-cancel-btn"));

  // Wait > 1.05s so elapsed timer appears
  await settle(1120);
  const timerEl = view.getByTestId("gee-action-elapsed-timer");
  assert.ok(timerEl.textContent?.includes("已用时 1."));

  // Cancel active request
  await act(async () => {
    fireEvent.click(view.getByTestId("gee-action-cancel-btn"));
  });
  assert.strictEqual(view.queryByTestId("gee-action-progress-bar"), null);
  assert.ok(view.getByTestId("gee-workbench-status-note").textContent?.includes("已中止当前 GEE 操作"));

  // Late resolution after cancel must NOT mount a stale map layer or overwrite cancelled notice
  resolveSlow({
    ok: true,
    artifact_id: "late-cancelled-artifact",
    title: "Stale Late Result",
    operation: "composite",
    bbox: [117.2, 31.8, 117.3, 31.9],
    tile_url_template: "https://example.com/{z}/{x}/{y}",
  });
  await settle(40);
  assert.strictEqual(getGeeLayerStoreSnapshot().layers.length, 0);
  assert.ok(view.getByTestId("gee-workbench-status-note").textContent?.includes("已中止当前 GEE 操作"));
});

test("18. extractGeeErrorMessage prioritizes ShellError.context and triggers too-large recovery card with 30m retry", async () => {
  const errObj = {
    code: "V3_OPEN_GEODATA_QUERY_FAILED",
    message_key: "application_service.invalid_request",
    context:
      "GEE_AOI_TOO_LARGE_FOR_SINGLE_DOWNLOAD: 当前研究区按原生 10 米分辨率共需请求 82 个分块，请缩小研究区范围（AOI），或传 use_preview_scale: true 改用推荐的 30 米轻量预览分辨率下载。",
  };
  const extracted = extractGeeErrorMessage(errObj);
  assert.ok(extracted.includes("GEE_AOI_TOO_LARGE_FOR_SINGLE_DOWNLOAD"));
  assert.ok(extracted.includes("82 个分块"));

  setGeeQueryInvokerForTest(async (payload) => {
    const ds = payload.dataset;
    if (ds === "gee_quota_status") {
      return {
        ok: true,
        configured: false,
        account_tier: "free",
        account_tier_label: "Free",
        local_ledger: { total_requests: 0, total_wall_seconds: 0, profiled_eecu_seconds: 0, has_eecu_profile: false },
        cloud_monitoring: { unlocked: false },
      };
    }
    if (ds === "gee_catalog_search") {
      return { ok: true, items: [] };
    }
    if (ds === "gee_check_availability") {
      return {
        ok: true,
        total_scenes: 3,
        cloud_filtered_scenes: 3,
        native_scale_m: 10,
        bands: ["B4", "B3", "B2"],
        grid_plan: { total_chunks: 82, full_width: 4000, full_height: 4000, estimated_mb: 80, suggested_preview_scale_m: 30 },
      };
    }
    if (ds === "gee_chunked_download") {
      throw errObj;
    }
    return { ok: true };
  });

  const view = render(
    <GeeWorkbenchPanel
      workingAoi={{ west: 117.1, south: 31.7, east: 117.6, north: 32.1 }}
      aoiLabel="大范围测试"
      startDate="2025-04-01"
      endDate="2025-06-30"
      onStartDateChange={() => {}}
      onEndDateChange={() => {}}
    />
  );

  await act(async () => {
    fireEvent.click(view.getByTestId("gee-preflight-btn"));
  });
  await settle(30);

  await act(async () => {
    fireEvent.click(view.getByTestId("gee-workbench-download-btn"));
  });
  await settle(20);
  const recovery = view.getByTestId("gee-too-large-recovery-card");
  assert.ok(recovery);
  assert.ok(recovery.textContent?.includes("82 块"));
  assert.ok(view.getByTestId("gee-workbench-status-note").textContent?.includes("82 个分块"));
  assert.ok(view.getByTestId("gee-retry-preview-scale-btn").textContent?.includes("30m"));
});

test("19. Static DEM dataset (COPERNICUS/DEM/GLO30) omits temporal filters on preflight/analyze/download and truncates asset ID cleanly", async () => {
  const capturedCalls: Array<{ dataset: string; payload: Record<string, unknown> }> = [];
  updateWorkbenchToolOptions({ geeAssetId: "COPERNICUS/DEM/GLO30" });

  setGeeQueryInvokerForTest(async (payload) => {
    const ds = String(payload.dataset || "");
    capturedCalls.push({ dataset: ds, payload });
    if (ds === "gee_quota_status") {
      return {
        ok: true,
        configured: false,
        account_tier: "free",
        account_tier_label: "Free",
        local_ledger: { total_requests: 0, total_wall_seconds: 0, profiled_eecu_seconds: 0, has_eecu_profile: false },
        cloud_monitoring: { unlocked: false },
      };
    }
    if (ds === "gee_catalog_search") {
      return {
        ok: true,
        items: [
          {
            id: "COPERNICUS/DEM/GLO30",
            title_zh: "Copernicus DEM GLO-30",
            title_en: "Copernicus DEM GLO-30",
            asset_type: "image_collection",
            source: "official",
            theme: "dem",
            native_scale_m: 30,
          },
        ],
      };
    }
    if (ds === "gee_check_availability") {
      return {
        ok: true,
        total_scenes: 4,
        cloud_filtered_scenes: 4,
        native_scale_m: 30,
        grid_plan: { total_chunks: 1, quota_confirm_required: false, exceed_hard_limit: false, suggested_preview_scale_m: 30, estimated_mb: 1.2 },
      };
    }
    if (ds === "gee_cloud_analyze") {
      return {
        ok: true,
        artifact_id: "dem-art-1",
        title: "Copernicus DEM",
        operation: "composite",
        bbox: [117.2, 31.8, 117.3, 31.9],
        tile_url_template: "https://example.com/{z}/{x}/{y}",
      };
    }
    if (ds === "gee_chunked_download") {
      return {
        ok: true,
        chunks_stitched: 1,
        file_name: "COPERNICUS_DEM_GLO30.tif",
        bytes_written: 1024,
      };
    }
    return { ok: true };
  });

  const view = render(
    <GeeWorkbenchPanel
      workingAoi={{ west: 117.2, south: 31.8, east: 117.3, north: 31.9 }}
      aoiLabel="合肥"
      startDate="2025-04-01"
      endDate="2025-06-30"
      onStartDateChange={() => {}}
      onEndDateChange={() => {}}
    />
  );

  await settle(20);
  const chip = view.getByTestId("gee-active-dataset-card");
  const idSpan = within(chip).getByText("COPERNICUS/DEM/GLO30");
  assert.ok(idSpan.className.includes("truncate"), "asset ID must use truncate instead of break-all");
  assert.strictEqual(idSpan.className.includes("break-all"), false);
  assert.ok(view.getByTestId("gee-timeless-dataset-note"));

  await act(async () => {
    fireEvent.click(view.getByTestId("gee-preflight-btn"));
  });
  await settle(20);
  await act(async () => {
    fireEvent.click(view.getByTestId("gee-workbench-analyze-btn"));
  });
  await act(async () => {
    fireEvent.click(view.getByTestId("gee-workbench-download-btn"));
  });

  for (const ds of ["gee_check_availability", "gee_cloud_analyze", "gee_chunked_download"]) {
    const call = capturedCalls.find((c) => c.dataset === ds);
    assert.ok(call, `${ds} must be invoked`);
    assert.strictEqual(call.payload.start_date, undefined, `${ds} must omit start_date for static DEM`);
    assert.strictEqual(call.payload.end_date, undefined, `${ds} must omit end_date for static DEM`);
    assert.strictEqual(call.payload.max_cloud, undefined, `${ds} must omit max_cloud for static DEM`);
  }
});

test("20. [Owner Round-5 Item 8] Strict 2-row 3-step progression gates Analyze & Download behind non-zero preflight and drops duplicate layer buttons", async () => {
  let emptyMode = true;
  setGeeQueryInvokerForTest(async (payload) => {
    if (payload.dataset === "gee_check_availability") {
      return emptyMode
        ? {
            ok: true,
            total_scenes: 0,
            cloud_filtered_scenes: 0,
            bands: ["dtm", "dsm"],
            native_scale_m: 15,
            grid_plan: { total_chunks: 1, full_width: 256, full_height: 256, estimated_mb: 0.5 },
          }
        : {
            ok: true,
            total_scenes: 6,
            cloud_filtered_scenes: 4,
            bands: ["B4", "B3", "B2"],
            native_scale_m: 10,
            grid_plan: { total_chunks: 1, full_width: 256, full_height: 256, estimated_mb: 0.5 },
          };
    }
    return { ok: true, items: [] };
  });

  const view = render(
    <GeeWorkbenchPanel
      workingAoi={{ west: 117.2, south: 31.8, east: 117.3, north: 31.9 }}
      aoiLabel="合肥"
      startDate="2025-04-01"
      endDate="2025-06-30"
      onStartDateChange={() => {}}
      onEndDateChange={() => {}}
    />
  );

  assert.equal(view.queryByTestId("gee-workbench-add-layer-btn"), null);
  assert.equal(view.queryByTestId("gee-preflight-add-layer-btn"), null);

  const analyzeBtn = view.getByTestId("gee-workbench-analyze-btn") as HTMLButtonElement;
  const downloadBtn = view.getByTestId("gee-workbench-download-btn") as HTMLButtonElement;
  assert.equal(analyzeBtn.disabled, true);
  assert.equal(downloadBtn.disabled, true);

  await act(async () => {
    fireEvent.click(view.getByTestId("gee-preflight-btn"));
  });
  await settle(20);
  assert.equal(analyzeBtn.disabled, true);
  assert.equal(downloadBtn.disabled, true);

  emptyMode = false;
  await act(async () => {
    fireEvent.click(view.getByTestId("gee-preflight-btn"));
  });
  await settle(20);
  assert.equal(analyzeBtn.disabled, false);
  assert.equal(downloadBtn.disabled, false);

  view.rerender(
    <GeeWorkbenchPanel
      workingAoi={{ west: 117.2, south: 31.8, east: 117.3, north: 31.9 }}
      aoiLabel="合肥"
      startDate="2025-05-01"
      endDate="2025-06-30"
      onStartDateChange={() => {}}
      onEndDateChange={() => {}}
    />
  );
  await settle(10);
  assert.equal((view.getByTestId("gee-workbench-analyze-btn") as HTMLButtonElement).disabled, true);
});

test("21. GeeScriptConfirmCard renders structured findings with L{line} and severity badges without duplicating scientific_warnings", async () => {
  setGeeQueryInvokerForTest(async (req) => {
    if (req.dataset === "gee_script_preview") {
      return {
        ok: true,
        script: "ee.ImageCollection('ESA/WorldCover/v200').mean()",
      };
    }
    return { ok: true };
  });

  const view = render(
    <GeeScriptConfirmCard
      toolId="gee_script_inspect"
      toolArgs={{ bbox: [117.2, 31.8, 117.3, 31.9] }}
      summary={{
        ok: true,
        approval_id: "gee-appr-findings-1",
        scientific_warnings: ["LEGACY_FALLBACK_ONLY_SHOULD_NOT_DUPLICATE"],
        findings: [
          {
            severity: "ERROR",
            code: "categorical_mean_error",
            line: 4,
            message: "分类波段禁止使用 .mean()",
          },
          {
            severity: "WARN",
            code: "s2_qa60_deprecated",
            line: 2,
            message: "QA60 云掩膜存在时间缺口",
          },
        ],
        blocked_reasons: [],
      }}
      scriptPreview={"ee.ImageCollection('ESA/WorldCover/v200').mean()"}
    />
  );

  await settle(20);
  const findingsBox = view.getByTestId("gee-script-findings");
  assert.ok(within(findingsBox).getByText("严重"));
  assert.ok(within(findingsBox).getByText("L4"));
  assert.ok(within(findingsBox).getByText("分类波段禁止使用 .mean()"));
  assert.ok(within(findingsBox).getByText("提醒"));
  assert.ok(within(findingsBox).getByText("L2"));
  assert.ok(within(findingsBox).getByText("QA60 云掩膜存在时间缺口"));
  assert.equal(view.queryByText(/LEGACY_FALLBACK_ONLY_SHOULD_NOT_DUPLICATE/), null);
});

test("22. GeeScriptConfirmCard renders gee_export_preflight card even when quota_confirm_required is true, and surfaces tile_unsafe warnings", async () => {
  const view = render(
    <GeeScriptConfirmCard
      toolId="gee_export_preflight"
      toolArgs={{ asset_id: "COPERNICUS/S2_SR_HARMONIZED" }}
      summary={{
        ok: true,
        ready: true,
        asset_id: "COPERNICUS/S2_SR_HARMONIZED",
        scale_m: 10,
        algorithm_kind: "index",
        tile_safety: "tile_safe",
        recommended_route: "local_chunked_geotiff_confirm",
        quota_confirm_required: true,
        raw_width_px: 2048,
        raw_height_px: 2048,
        raw_total_chunks: 16,
        estimated_mb: 32.5,
        data_layer_record: {
          dataset_source: "official",
          dataset_id: "COPERNICUS/S2_SR_HARMONIZED",
          native_scale_m: 10,
          analysis_scale_m: 10,
          time_range: { start_date: "2025-04-01", end_date: "2025-06-30", is_timeless: false },
          verification_status: "catalog_curated",
        },
      }}
    />
  );

  const preflightCard = view.getByTestId("gee-export-preflight-card");
  assert.equal(view.queryByTestId("gee-quota-confirm-card"), null);
  assert.equal(view.queryByText(/大幅面科研分辨率与分块下载确认/), null);
  assert.ok(within(preflightCard).getByText("GEE 离线导出预演与安全评估"));
  assert.ok(within(preflightCard).getByText("支持独立分块"));
  assert.ok(within(preflightCard).getByText("推荐通道：本机分块下载（需确认分辨率）"));
  const provStrip = within(preflightCard).getByTestId("gee-provenance-strip");
  assert.ok(within(provStrip).getByText("COPERNICUS/S2_SR_HARMONIZED"));

  view.rerender(
    <GeeScriptConfirmCard
      toolId="gee_export_preflight"
      toolArgs={{ asset_id: "COPERNICUS/S2_SR_HARMONIZED" }}
      summary={{
        ok: true,
        ready: true,
        asset_id: "COPERNICUS/S2_SR_HARMONIZED",
        scale_m: 10,
        algorithm_kind: "cluster",
        tile_safety: "tile_unsafe",
        tile_safety_reason: "全局聚类无法安全分块",
        recommended_route: "external_batch_export",
        quota_confirm_required: false,
        raw_width_px: 4096,
        raw_height_px: 4096,
        raw_total_chunks: 64,
        estimated_mb: 128.0,
        warnings: ["全局聚类无法安全分块", "建议导出前复核波段投影"],
      }}
    />
  );

  const updatedCard = view.getByTestId("gee-export-preflight-card");
  assert.ok(within(updatedCard).getByText("禁止独立分块"));
  assert.equal(within(updatedCard).getAllByText(/全局聚类无法安全分块/).length, 1);
  assert.ok(within(updatedCard).getByText("· 建议导出前复核波段投影"));
  assert.ok(within(updatedCard).getByText("推荐通道：外部批量导出（Drive 或云存储）"));
});

test("23. GeeScriptConfirmCard Artifact card renders data_layer_record provenance strip", async () => {
  setGeeQueryInvokerForTest(async (req) => {
    if (req.dataset === "gee_artifact_get") {
      return {
        artifact_id: "gee-prov-art-1",
        title: "社区人口网格分析",
        dataset_id: "projects/sat-io/open-datasets/ORNL/LANDSCAN_GLOBAL",
        operation: "composite",
        bbox: [117.2, 31.8, 117.3, 31.9],
        stats: { mean: 120.5 },
        created_at: 1700000001,
        data_layer_record: {
          dataset_source: "community",
          dataset_id: "projects/sat-io/open-datasets/ORNL/LANDSCAN_GLOBAL",
          native_scale_m: 1000,
          analysis_scale_m: 1000,
          time_range: { start_date: "2022-01-01", end_date: "2022-12-31", is_timeless: false },
          verification_status: "community_unverified",
        },
      };
    }
    return { ok: true };
  });

  const view = render(
    <GeeScriptConfirmCard
      toolId="gee_cloud_analyze"
      toolArgs={{ asset_id: "projects/sat-io/open-datasets/ORNL/LANDSCAN_GLOBAL" }}
      summary={{
        ok: true,
        artifact_id: "gee-prov-art-1",
        operation: "composite",
      }}
    />
  );

  await settle(30);
  const strip = view.getByTestId("gee-provenance-strip");
  assert.ok(within(strip).getByText("社区目录（待核验）"));
  assert.ok(within(strip).getByText("projects/sat-io/open-datasets/ORNL/LANDSCAN_GLOBAL"));
  assert.ok(within(strip).getByText(/community_unverified/));
});

test("24. GeeWorkbenchPanel renders deprecated replacement and community verify hints while preserving 社区库 · 1000m text", async () => {
  setGeeQueryInvokerForTest(async (req) => {
    if (req.dataset === "gee_catalog_search") {
      return {
        ok: true,
        items: [
          {
            id: "projects/sat-io/open-datasets/test_1km",
            title_zh: "社区人口网格",
            title_en: "Community Pop",
            source: "community",
            native_scale_m: 1000,
            asset_type: "IMAGE",
            bands: ["b1"],
            match_reasons: ["关键词命中"],
          },
          {
            id: "USGS/SRTMGL1_003_OLD",
            title_zh: "旧高程数据集",
            title_en: "Legacy DEM",
            source: "official",
            native_scale_m: 30,
            asset_type: "IMAGE",
            bands: ["elevation"],
            deprecated: true,
            replacement_id: "COPERNICUS/DEM/GLO30_2024_1",
          },
        ],
      };
    }
    return { ok: true, items: [] };
  });

  const view = render(
    <GeeWorkbenchPanel
      workingAoi={{ west: 117.2, south: 31.8, east: 117.3, north: 31.9 }}
      aoiLabel="合肥"
      startDate="2025-04-01"
      endDate="2025-06-30"
      onStartDateChange={() => {}}
      onEndDateChange={() => {}}
    />
  );

  const deadline = Date.now() + 1500;
  while (
    Date.now() < deadline &&
    !view.queryByTestId("gee-catalog-option-projects/sat-io/open-datasets/test_1km")
  ) {
    await settle(25);
  }
  assert.ok(view.queryByTestId("gee-catalog-option-projects/sat-io/open-datasets/test_1km"));
  assert.ok(view.getByText("社区库 · 1000m"));
  assert.ok(view.getByText("社区目录仅作发现提示，使用前请核验"));
  assert.ok(view.getByText("关键词命中"));
  assert.ok(view.getByText("已退役"));
  assert.ok(view.getByText("建议改用 COPERNICUS/DEM/GLO30_2024_1"));
});

test("25. GeeScriptConfirmCard updates provenance strip from resolved download artifact after quota confirmation instead of showing stale initial summary provenance", async () => {
  setGeeQueryInvokerForTest(async (req) => {
    if (req.dataset === "gee_chunked_download") {
      return {
        ok: true,
        artifact_id: "gee-dl-preview-30m",
        operation: "chunked_download",
        scale_m: 30,
        data_layer_record: {
          dataset_source: "official",
          dataset_id: "COPERNICUS/S2_SR_HARMONIZED",
          native_scale_m: 10,
          analysis_scale_m: 30,
          output_target: "local_geotiff",
          verification_status: "preview_resolved",
        },
      };
    }
    if (req.dataset === "gee_artifact_get") {
      return {
        artifact_id: "gee-dl-preview-30m",
        title: "预览分辨率导出结果",
        dataset_id: "COPERNICUS/S2_SR_HARMONIZED",
        operation: "chunked_download",
        bbox: [117.2, 31.8, 117.3, 31.9],
        stats: { mean: 0.45 },
        created_at: 1700000002,
        data_layer_record: {
          dataset_source: "official",
          dataset_id: "COPERNICUS/S2_SR_HARMONIZED",
          native_scale_m: 10,
          analysis_scale_m: 30,
          output_target: "local_geotiff",
          verification_status: "preview_resolved",
        },
      };
    }
    return { ok: true };
  });

  const view = render(
    <GeeScriptConfirmCard
      toolId="gee_chunked_download"
      toolArgs={{
        asset_id: "COPERNICUS/S2_SR_HARMONIZED",
        bbox: [117.2, 31.8, 117.3, 31.9],
      }}
      summary={{
        ok: true,
        quota_confirm_required: true,
        asset_id: "COPERNICUS/S2_SR_HARMONIZED",
        grid_plan: {
          requested_scale_m: 10,
          suggested_preview_scale_m: 30,
          total_chunks: 16,
          full_width: 2048,
          full_height: 2048,
          estimated_mb: 32.0,
        },
        data_layer_record: {
          dataset_source: "official",
          dataset_id: "COPERNICUS/S2_SR_HARMONIZED",
          native_scale_m: 10,
          analysis_scale_m: 10,
          output_target: "local_chunked_geotiff_confirm",
          verification_status: "confirm_pending",
        },
      }}
    />
  );

  await act(async () => {
    fireEvent.click(view.getByTestId("gee-confirm-preview-res-btn"));
  });

  const deadline = Date.now() + 1500;
  while (Date.now() < deadline && !view.queryByTestId("gee-provenance-strip")) {
    await settle(25);
  }
  const strip = view.getByTestId("gee-provenance-strip");
  assert.ok(within(strip).getByText("30m"));
  assert.ok(within(strip).getByText(/preview_resolved/));
  assert.equal(within(strip).queryByText("10m"), null);
  assert.equal(within(strip).queryByText(/confirm_pending/), null);
});
