/**
 * CompanionAndTools.interaction.test.tsx
 * 配套下载弹层（精密轨道接通，DEM/GACOS 未接通禁用）交互测试套件
 */

import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React, { useState } from "react";
import { render, cleanup, fireEvent } from "../../test-utils/render-harness";
import { screen } from "@testing-library/react";
import { CompanionDownloadPopover } from "./CompanionDownloadPopover";
import { DownloadTaskRow } from "./DownloadTaskRow";
import type { SceneRow } from "@/lib/bridge";
import {
  extractSceneBbox,
  geojsonToBbox,
  resolveAuxiliaryExtentGeojson,
  type CompanionState,
} from "@/lib/companionDownloadLogic";
import { canonicalSentinel1SceneId } from "@/lib/companionGacosFlow";
import type { ApplicationTask } from "@/shell/api/types";

test.afterEach(() => {
  cleanup();
});

const mockScenes: SceneRow[] = [
  {
    scene_id: "S1A_IW_SLC__1SDV_20240101T000000_20240101T000030_051900_064400_0001",
    product_name: "S1A_IW_SLC__1SDV_20240101T000000_20240101T000030_051900_064400_0001",
    acquisition_datetime: "2024-01-01T00:00:00Z",
    footprint_bbox: [114.0, 30.0, 115.0, 31.0],
    orbit_direction: "ascending",
    path: 100,
    frame: 200,
  },
  {
    scene_id: "S1A_IW_SLC__1SDV_20240113T000000_20240113T000030_051900_064400_0002",
    product_name: "S1A_IW_SLC__1SDV_20240113T000000_20240113T000030_051900_064400_0002",
    acquisition_datetime: "2024-01-13T00:00:00Z",
    footprint_bbox: [114.2, 30.2, 115.2, 31.2],
    orbit_direction: "ascending",
    path: 100,
    frame: 200,
  },
];

test("1. CompanionDownloadPopover renders S1 Orbit/DEM/GACOS, removes placeholders, and triggers standalone dispatch", () => {
  let currentState: CompanionState = {
    preciseOrbit: true,
    dem: false,
    gacos: false,
    extentMode: "study_area",
  };
  let standaloneCalls = 0;

  function TestHarness({ resourceId = "science.sentinel-1.standard.v1" }: { resourceId?: string }) {
    const [state, setState] = useState<CompanionState>(currentState);
    return (
      <CompanionDownloadPopover
        isOpen={true}
        onClose={() => {}}
        activeResourceId={resourceId}
        selectedScenes={mockScenes}
        workingAoiBbox={[114.0, 30.0, 115.0, 31.0]}
        companionState={state}
        onCompanionStateChange={(next) => {
          setState(next);
          currentState = next;
        }}
        onTriggerStandaloneCompanion={() => {
          standaloneCalls += 1;
        }}
      />
    );
  }

  render(<TestHarness />);

  const popover = screen.getByTestId("companion-download-popover");
  assert.ok(popover);

  // 验证精密轨道复选框及其默认提示
  const orbitCheckbox = screen.getByTestId("checkbox-precise-orbit") as HTMLInputElement;
  assert.strictEqual(orbitCheckbox.checked, true);

  // 验证已移除 ERA5 与电离层占位项
  assert.strictEqual(screen.queryByTestId("companion-item-era5"), null);
  assert.strictEqual(screen.queryByTestId("companion-item-ionosphere"), null);

  // 取消精密轨道勾选
  fireEvent.click(orbitCheckbox);
  assert.strictEqual(currentState.preciseOrbit, false);

  // 验证 30m DEM 已真实接通，不再标注暂未接通，且可点击勾选并估算瓦片数
  const demCheckbox = screen.getByTestId("checkbox-dem") as HTMLInputElement;
  assert.strictEqual(demCheckbox.disabled, false);
  const demItem = screen.getByTestId("companion-item-dem");
  assert.ok(!demItem.textContent?.includes("暂未接通"));
  assert.ok(screen.getByTestId("dem-estimate-badge").textContent?.includes("1 个瓦片"));
  fireEvent.click(demCheckbox);
  assert.strictEqual(currentState.dem, true);

  // 验证 GACOS 大气延迟已接通、显示申请批次估算并可勾选切换范围模式
  const gacosCheckbox = screen.getByTestId("checkbox-gacos") as HTMLInputElement;
  assert.strictEqual(gacosCheckbox.disabled, false);
  assert.strictEqual(gacosCheckbox.checked, false);
  const gacosBadge = screen.getByTestId("gacos-estimate-badge");
  assert.ok(gacosBadge.textContent?.includes("2 个日期"));
  assert.ok(gacosBadge.textContent?.includes("1 次申请"));

  fireEvent.click(gacosCheckbox);
  assert.strictEqual(currentState.gacos, true);

  const extentSelector = screen.getByTestId("companion-extent-selector");
  assert.ok(extentSelector);
  const sceneCoverageBtn = screen.getByTestId("extent-option-scene-coverage");
  fireEvent.click(sceneCoverageBtn);
  assert.strictEqual(currentState.extentMode, "scene_coverage");

  // 验证「仅下载已选配套」按钮可用并触发回调
  const standaloneBtn = screen.getByTestId("btn-standalone-companion-download") as HTMLButtonElement;
  assert.strictEqual(standaloneBtn.disabled, false);
  fireEvent.click(standaloneBtn);
  assert.strictEqual(standaloneCalls, 1);

  // 切换到非 S1 的 SAR 光源（如 NISAR）：隐藏精密轨道，保留 DEM 与 GACOS 可选
  cleanup();
  render(<TestHarness resourceId="science.nisar.l.rslc.provisional.v1" />);
  assert.strictEqual(screen.queryByTestId("companion-item-precise-orbit"), null);
  assert.strictEqual((screen.getByTestId("checkbox-dem") as HTMLInputElement).disabled, false);
  assert.strictEqual((screen.getByTestId("checkbox-gacos") as HTMLInputElement).disabled, false);
});

test("2. CompanionDownloadPopover closes on Esc key and outside click", () => {
  let closeCount = 0;
  const state: CompanionState = {
    preciseOrbit: false,
    dem: false,
    gacos: false,
    extentMode: "study_area",
  };

  render(
    <div>
      <div data-testid="outside-element">Outside</div>
      <CompanionDownloadPopover
        isOpen={true}
        onClose={() => { closeCount += 1; }}
        activeResourceId="science.sentinel-1.standard.v1"
        selectedScenes={mockScenes}
        companionState={state}
        onCompanionStateChange={() => {}}
      />
    </div>
  );

  // 按 Esc 触发关闭
  fireEvent.keyDown(window, { key: "Escape" });
  assert.strictEqual(closeCount, 1);

  // 点击外部触发关闭
  const outsideEl = screen.getByTestId("outside-element");
  fireEvent.mouseDown(outsideEl);
  assert.strictEqual(closeCount, 2);
});

test("3. resolveAuxiliaryExtentGeojson and DownloadTaskRow GACOS submitted status", () => {
  const providerProjectedScene: SceneRow = {
    scene_id:
      "PROVIDER::asf.catalog::COLLECTION::SENTINEL-1::ITEM::S1D_IW_SLC__1SDV_20261019T235337_20261019T235404_005091_009A2D_26FC-SLC",
    file_name:
      "S1D_IW_SLC__1SDV_20261019T235337_20261019T235404_005091_009A2D_26FC.zip",
    acquisition_datetime: "2026-10-19T23:53:37Z",
    footprint_geojson: {
      type: "Polygon",
      coordinates: [[[91.0, 29.0], [92.0, 29.0], [92.0, 30.0], [91.0, 30.0], [91.0, 29.0]]],
    },
    native_metadata: {
      scene_id: "S1D_IW_SLC__1SDV_20261019T235337_20261019T235404_005091_009A2D_26FC",
    },
  };
  assert.strictEqual(
    canonicalSentinel1SceneId(providerProjectedScene),
    "S1D_IW_SLC__1SDV_20261019T235337_20261019T235404_005091_009A2D_26FC"
  );
  const extractedBbox = extractSceneBbox(providerProjectedScene);
  assert.ok(extractedBbox);
  assert.strictEqual(extractedBbox.west, 91.0);
  assert.strictEqual(extractedBbox.east, 92.0);
  const polyBbox = geojsonToBbox(providerProjectedScene.footprint_geojson as Record<string, unknown>);
  assert.ok(polyBbox);

  const studyGeo = resolveAuxiliaryExtentGeojson(
    "study_area",
    { type: "Polygon", coordinates: [[[114, 30], [115, 30], [115, 31], [114, 31], [114, 30]]] },
    [114, 30, 115, 31],
    mockScenes
  );
  assert.ok(studyGeo);
  assert.strictEqual(studyGeo?.type, "Polygon");

  const coverageGeo = resolveAuxiliaryExtentGeojson(
    "scene_coverage",
    null,
    [114, 30, 115, 31],
    mockScenes
  );
  assert.ok(coverageGeo);
  assert.strictEqual(coverageGeo?.type, "Polygon");

  const gacosTask: ApplicationTask = {
    task_id: "gacos-request-abc123456789",
    revision: 1,
    name: "合肥市 · GACOS 大气延迟申请",
    category: "scene_download",
    status: "queued",
    provider: "GACOS Webform",
    source_provider_id: "gacos.webform",
    dataset: "atmospheric.gacos.ztd.v1",
    items_count: 1,
    completed_items_count: 0,
    failed_items_count: 0,
    done_bytes: 0,
    total_bytes: 0,
    bytes_reported: false,
    output_dir: "/tmp/gacos",
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    scene_ids: ["gacos-request-abc123456789:group-1"],
  };

  render(<DownloadTaskRow task={gacosTask} />);
  const submittedBadge = screen.getByTestId("gacos-submitted-badge");
  assert.strictEqual(submittedBadge.textContent, "已提交申请");
  assert.strictEqual(screen.queryByTestId("gacos-toggle-followup-btn"), null);
  assert.strictEqual(screen.queryByTestId("gacos-followup-panel"), null);
});
