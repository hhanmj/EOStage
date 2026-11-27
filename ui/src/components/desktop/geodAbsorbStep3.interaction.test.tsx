import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import React from "react";
import { render, cleanup, fireEvent } from "../../test-utils/render-harness";
import { waitFor } from "@testing-library/react";
import { I18nProvider } from "../../i18n/index.ts";
import {
  clearTestDesktopShellBridge,
  registerTestDesktopShellBridge,
} from "../../lib/desktopShellBridge.ts";
import { ManualConditionsPanel } from "./ManualConditionsPanel.tsx";
import { RasterPixelProbePanel } from "./RasterPixelProbePanel.tsx";
import { DesktopDownloadDrawer } from "./DesktopDownloadDrawer.tsx";
import { DownloadTaskRow } from "./DownloadTaskRow.tsx";
import type { ApplicationTask } from "../../shell/api/types.ts";

test.afterEach(() => {
  cleanup();
  clearTestDesktopShellBridge();
});

test("1. 选择 FABDEM 数据源时必须渲染 CC BY-NC-SA 4.0 非商用许可提醒横幅", () => {
  const noop = () => {};
  const view = render(
    <I18nProvider>
      <ManualConditionsPanel
        scientificMode="dem"
        onScientificModeChange={noop}
        onResetParams={noop}
        availableSources={[]}
        activeResourceId="dem.fabdem.v1"
        onSelectSource={noop}
        productType=""
        onProductTypeChange={noop}
        workingAoi={null}
        workingGeometry={null}
        aoiLabel="蜀山区"
        onReplaceAoi={async () => {}}
        onPreviewAdminGeometry={noop}
        onImportVectorSuccess={noop}
        startDate="2026-01-01"
        endDate="2026-02-01"
        onStartDateChange={noop}
        onEndDateChange={noop}
        orbitDirection={null}
        onOrbitDirectionChange={noop}
        relativeOrbit={null}
        onRelativeOrbitChange={noop}
        frame={null}
        onFrameChange={noop}
        searching={false}
        onTriggerSearch={noop}
        demSource="FABDEM"
        onDemSourceChange={noop}
      />
    </I18nProvider>,
  );

  const banner = view.getByTestId("fabdem-noncommercial-license-banner");
  assert.ok(banner, "选择 FABDEM 时必须显示非商用许可提醒横幅");
  assert.ok(
    banner.textContent?.includes("CC BY-NC-SA 4.0"),
    "横幅内容必须包含 CC BY-NC-SA 4.0 许可声明",
  );
});

test("1b. 选择 Google GEM-15 DTM 或 DSM 15m 时展示不含中印韩及南极洲的覆盖范围提示横幅", () => {
  const noop = () => {};
  for (const dataset of ["GEM15_DTM", "GEM15_DSM"] as const) {
    const view = render(
      <I18nProvider>
        <ManualConditionsPanel
          scientificMode="dem"
          onScientificModeChange={noop}
          onResetParams={noop}
          availableSources={[
            {
              resource_id: dataset === "GEM15_DTM" ? "dem.gem15-dtm.v1" : "dem.gem15-dsm.v1",
              family: "dem",
              group_id: "dem",
              display_name_zh: `Google GEM-15 ${dataset === "GEM15_DTM" ? "DTM" : "DSM"} 15m`,
              display_name_en: `Google GEM-15 ${dataset === "GEM15_DTM" ? "DTM" : "DSM"} 15m`,
              availability: "available",
              accounts: [],
            },
          ]}
          activeResourceId={dataset === "GEM15_DTM" ? "dem.gem15-dtm.v1" : "dem.gem15-dsm.v1"}
          onSelectSource={noop}
          productType=""
          onProductTypeChange={noop}
          workingAoi={null}
          workingGeometry={null}
          aoiLabel="San Francisco"
          onReplaceAoi={async () => {}}
          onPreviewAdminGeometry={noop}
          onImportVectorSuccess={noop}
          startDate="2026-01-01"
          endDate="2026-02-01"
          onStartDateChange={noop}
          onEndDateChange={noop}
          orbitDirection={null}
          onOrbitDirectionChange={noop}
          relativeOrbit={null}
          onRelativeOrbitChange={noop}
          frame={null}
          onFrameChange={noop}
          searching={false}
          onTriggerSearch={noop}
          demSource={dataset}
          onDemSourceChange={noop}
        />
      </I18nProvider>,
    );

    const gemBanner = view.getByTestId("gem15-coverage-notice-banner");
    assert.ok(gemBanner, `选择 ${dataset} 时必须显示覆盖范围提示横幅`);
    assert.ok(
      gemBanner.textContent?.includes("不包含中国") || gemBanner.textContent?.includes("Excludes Mainland China"),
      "横幅内容必须提示 GEM-15 v1 不含中国区域",
    );
    view.unmount();
  }
});

test("2. RasterPixelProbePanel 点击查询后正确展示像元行列号、定标物理值与 SCL 语义", async () => {
  let calledWith: { source: string; lon: number; lat: number } | null = null;
  registerTestDesktopShellBridge({
    kind: "tauri",
    application: {
      gisRasterPixelProbe: async (req: { source: string; lon: number; lat: number }) => {
        calledWith = req;
        return {
          ok: true,
          source: req.source,
          lon: req.lon,
          lat: req.lat,
          projected_coord: [521500.0, 3520500.0],
          crs: 'PROJCS["WGS 84 / UTM zone 50N"]',
          width: 128,
          height: 128,
          in_bounds: true,
          col: 42,
          row: 19,
          bands: [
            {
              band: 1,
              name: "B04",
              dtype: "uint16",
              raw_value: 1500,
              scale: 0.0001,
              offset: 0,
              physical_value: 0.15,
              nodata: -9999,
              is_nodata: false,
              qa_semantics: null,
            },
            {
              band: 2,
              name: "SCL",
              dtype: "uint8",
              raw_value: 4,
              scale: 1,
              offset: 0,
              physical_value: 4,
              nodata: null,
              is_nodata: false,
              qa_semantics: "SCL=4: Vegetation (植被)",
            },
          ],
        };
      },
    },
  } as any);

  const view = render(
    <RasterPixelProbePanel
      sourcePath="/tmp/hefei_s2.tif"
      defaultLon={117.2272}
      defaultLat={31.8206}
    />,
  );

  fireEvent.click(view.getByTestId("pixel-probe-run-btn"));

  const results = await waitFor(() => view.getByTestId("pixel-probe-results"));
  assert.ok(calledWith);
  assert.equal(calledWith!.source, "/tmp/hefei_s2.tif");
  assert.ok(results.textContent?.includes("(42, 19)"));
  assert.ok(results.textContent?.includes("0.1500"));
  assert.ok(results.textContent?.includes("SCL=4: Vegetation (植被)"));
});

test("3. 下载中心页面与已完成任务行不得渲染未经批准的实验工程资产包或加入实验工程按钮", async () => {
  const completedTask: ApplicationTask = {
    task_id: "task-clean-download",
    revision: 1,
    name: "Sentinel-2 L2A Delivery",
    category: "scene_download",
    status: "completed",
    provider: "MPC",
    dataset: "Sentinel-2 L2A",
    created_at: "2026-11-15T00:00:00Z",
    updated_at: "2026-11-15T01:00:00Z",
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

  const drawerView = render(
    <I18nProvider>
      <DesktopDownloadDrawer open variant="page" tasks={[completedTask]} />
    </I18nProvider>,
  );
  await new Promise((resolve) => setTimeout(resolve, 25));
  assert.equal(drawerView.queryByTestId("experiment-project-drawer-bar"), null);
  assert.equal(drawerView.queryByText("实验工程资产包"), null);

  const expandButtons = drawerView.getAllByRole("button", { name: "展开分景进度" });
  fireEvent.click(expandButtons[0]);
  await new Promise((resolve) => setTimeout(resolve, 15));
  assert.equal(drawerView.queryByTestId("add-to-experiment-project-btn"), null);
  assert.equal(drawerView.queryByText("加入实验工程"), null);

  cleanup();

  const rowView = render(
    <I18nProvider>
      <DownloadTaskRow task={completedTask} />
    </I18nProvider>,
  );
  fireEvent.click(rowView.getByRole("button", { name: "展开分景进度" }));
  await new Promise((resolve) => setTimeout(resolve, 15));
  assert.equal(rowView.queryByTestId("add-to-experiment-project-btn"), null);
  assert.equal(rowView.queryByText("加入实验工程"), null);
});

test("4. DesktopWorkbench finishSearchBusy 必须调用 setSearching(false) 且不得自我递归", () => {
  const __dirname = path.dirname(fileURLToPath(import.meta.url));
  const workbenchPath = path.resolve(__dirname, "./DesktopWorkbench.tsx");
  const workbenchSrc = fs.readFileSync(workbenchPath, "utf-8");

  const match = workbenchSrc.match(/const\s+finishSearchBusy\s*=\s*\(\)\s*=>\s*\{([\s\S]*?)\};/);
  assert.ok(match, "DesktopWorkbench.tsx 必须定义 finishSearchBusy");
  const body = match[1].trim();
  assert.ok(
    body.includes("setSearching(false)"),
    `finishSearchBusy 必须调用 setSearching(false)，实际实现: ${body}`,
  );
  assert.equal(
    /\bfinishSearchBusy\s*\(/.test(body),
    false,
    `finishSearchBusy 不得自我递归调用，实际实现: ${body}`,
  );

  const runner = new Function(
    "isExplicitDownload",
    "setSearching",
    `const finishSearchBusy = () => { ${body} }; finishSearchBusy();`,
  ) as (isExplicitDownload: boolean, setSearching: (value: boolean) => void) => void;

  const calls: boolean[] = [];
  runner(false, (value) => calls.push(value));
  assert.deepEqual(calls, [false]);

  const explicitCalls: boolean[] = [];
  runner(true, (value) => explicitCalls.push(value));
  assert.deepEqual(explicitCalls, []);
});
