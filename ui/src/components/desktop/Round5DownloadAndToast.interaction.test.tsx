import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { fireEvent } from "@testing-library/react";
import { render, cleanup } from "../../test-utils/render-harness";
import { I18nProvider, setCurrentLocale } from "../../i18n";
import type { ApplicationTask } from "../../shell/api/types";
import { DownloadFilterBar } from "./DownloadFilterBar";
import { taskMatchesStatusFilter } from "./downloadFilterLogic";
import { sortTaskSceneItemsByPriority } from "./DownloadTaskRow";
import {
  clearSubsetSizeEstimatesForTest,
  lookupSubsetSizeEstimate,
  recordSubsetSizeEstimate,
} from "../../lib/subsetSizeEstimateRegistry";
import { taskCardSizeLine } from "../../shell/api/taskCardLogic";
import { formatBytes } from "../../shell/api/rowProjection";
import * as nodeFs from "node:fs";
import * as nodePath from "node:path";
import { ProgressNotificationHost } from "../ui/ProgressNotification";
import { TopCenterToastHost, clearAllTopToastsForTest } from "../ui/TopCenterToast";
import { TransferEngineSettingsSection } from "../settings/TransferEngineSettingsSection";
import {
  DEFAULT_TRANSFER_ENGINE_PREFS,
  getTransferEnginePrefs,
  MAX_DOWNLOAD_SPEED_KIB_S_LIMIT,
  normalizeMaxDownloadSpeedKibS,
  normalizeTransferEnginePrefs,
  resolveSsdStagingMode,
  setTransferEnginePrefs,
  ssdStagingPatchForMode,
} from "../../lib/transferEnginePrefs";

test.afterEach(() => {
  cleanup();
  clearSubsetSizeEstimatesForTest();
  clearAllTopToastsForTest();
  setTransferEnginePrefs({ ...DEFAULT_TRANSFER_ENGINE_PREFS });
  if (typeof window !== "undefined") {
    window.localStorage?.clear?.();
  }
  setCurrentLocale("zh-CN");
});

function makeTask(overrides: Partial<ApplicationTask>): ApplicationTask {
  return {
    task_id: "t-1",
    revision: 1,
    category: "scene_download",
    name: "Sentinel-1 下载",
    provider: "ASF",
    dataset: "SENTINEL-1",
    status: "running",
    done_bytes: 0,
    total_bytes: null,
    created_at: "2026-11-16T01:00:00Z",
    updated_at: "2026-11-16T01:00:00Z",
    selected_count: 1,
    completed_count: 0,
    failed_count: 0,
    paused_count: 0,
    queued_count: 0,
    skipped_count: 0,
    cancelled_count: 0,
    ...overrides,
  };
}

test("Round 5.1: DownloadFilterBar 渲染状态 + 数据源双筛选并精准计算交集计数", () => {
  const tasks: ApplicationTask[] = [
    makeTask({ task_id: "t1", dataset: "optical.landsat.c2.l2", status: "running" }),
    makeTask({ task_id: "t2", dataset: "optical.landsat.c2.l2", status: "completed" }),
    makeTask({ task_id: "t3", dataset: "NISAR L", status: "paused" }),
    makeTask({ task_id: "t4", dataset: "NISAR S", status: "failed" }),
  ];

  assert.equal(taskMatchesStatusFilter("running", "active"), true);
  assert.equal(taskMatchesStatusFilter("queued", "active"), true);
  assert.equal(taskMatchesStatusFilter("paused", "paused"), true);
  assert.equal(taskMatchesStatusFilter("completed", "completed"), true);
  assert.equal(taskMatchesStatusFilter("cancelled", "failed"), true);
  assert.equal(taskMatchesStatusFilter("failed", "failed"), true);

  const selectedStatuses: string[] = [];
  const selectedTypes: string[] = [];

  const view = render(
    <I18nProvider>
      <DownloadFilterBar
        tasks={tasks}
        selectedType="all"
        onSelectType={(tp) => selectedTypes.push(tp)}
        selectedStatus="all"
        onSelectStatus={(st) => selectedStatuses.push(st)}
      />
    </I18nProvider>
  );

  assert.ok(view.getByTestId("download-status-filter-bar"));
  assert.ok(view.getByTestId("download-dataset-filter-bar"));
  assert.match(view.getByTestId("download-status-filter-active").textContent ?? "", /进行中.*1/);
  assert.match(view.getByTestId("download-status-filter-completed").textContent ?? "", /已完成.*1/);
  assert.match(view.getByTestId("download-status-filter-failed").textContent ?? "", /失败\/已取消.*1/);

  fireEvent.click(view.getByTestId("download-status-filter-paused"));
  assert.deepEqual(selectedStatuses, ["paused"]);
});

test("Round 5.2: sortTaskSceneItemsByPriority 将活跃下载置顶，已完成置于排队之后", () => {
  const sorted = sortTaskSceneItemsByPriority([
    { scene_id: "done-1", status: "completed" },
    { scene_id: "cancel-1", status: "cancelled" },
    { scene_id: "wait-1", status: "queued" },
    { scene_id: "pause-1", status: "paused" },
    { scene_id: "err-1", status: "failed" },
    { scene_id: "active-1", status: "running" },
    { scene_id: "done-2", status: "completed" },
    { scene_id: "active-2", status: "subsetting" },
  ]);

  assert.deepEqual(
    sorted.map((s) => s.scene_id),
    ["active-1", "active-2", "err-1", "pause-1", "wait-1", "done-1", "done-2", "cancel-1"]
  );
});

test("Round 5.3: 裁剪预估体积在运行中显示「预计 ~」，完成后切换真实磁盘字节", () => {
  const est = 120 * 1024 * 1024;
  recordSubsetSizeEstimate({
    taskId: "nisar-sub-42",
    totalEstimatedBytes: est,
    sceneEstimates: { "NISAR_GSLC_1": est },
  });
  assert.equal(lookupSubsetSizeEstimate("nisar-sub-42"), est);

  const runningLine = taskCardSizeLine(
    makeTask({
      task_id: "nisar-sub-42",
      status: "running",
      done_bytes: 0,
      total_bytes: null,
    }),
    formatBytes
  );
  assert.match(runningLine, /^预计 ~120(\.0)? MB$/);

  const completedLine = taskCardSizeLine(
    makeTask({
      task_id: "nisar-sub-42",
      status: "completed",
      done_bytes: 98 * 1024 * 1024,
      total_bytes: null,
    }),
    formatBytes
  );
  assert.match(completedLine, /^98(\.0)? MB$/);

  const toastView = render(
    <ProgressNotificationHost
      tasks={[
        makeTask({
          task_id: "nisar-sub-42",
          name: "NISAR S 波段 GSLC · 裁剪",
          status: "running",
          done_bytes: 24 * 1024 * 1024,
          total_bytes: null,
          created_at: new Date(Date.now() - 3000).toISOString(),
        }),
      ]}
      pendingPreparations={[]}
    />
  );
  assert.match(
    toastView.getByTestId("progress-notification-card").textContent ?? "",
    /20% · 预计 ~120(\.0)? MB/
  );
});

test("Round 5.4: 三级并发控制归一化与 UI 同步 DEM 上限", async () => {
  const norm = normalizeTransferEnginePrefs({
    max_concurrent_tasks: 12,
    per_file_segments: 8,
    tile_chunk_concurrency: 64,
  });
  assert.equal(norm.max_concurrent_tasks, 8);
  assert.equal(norm.tile_chunk_concurrency, 64);
  assert.equal(DEFAULT_TRANSFER_ENGINE_PREFS.max_concurrent_tasks, 2);
  assert.equal(DEFAULT_TRANSFER_ENGINE_PREFS.tile_chunk_concurrency, 16);

  const demSynced: number[] = [];
  const view = render(
    <I18nProvider>
      <TransferEngineSettingsSection onSyncDemConcurrency={(n) => demSynced.push(n)} />
    </I18nProvider>
  );

  assert.ok(view.getByTestId("transfer-tier1-max-tasks"));
  assert.ok(view.getByTestId("transfer-tier2-segments"));
  assert.ok(view.getByTestId("transfer-tier3-tile-concurrency"));

  fireEvent.click(view.getByTestId("transfer-tier1-tasks-6"));
  await new Promise((resolve) => setTimeout(resolve, 25));
  assert.deepEqual(demSynced, [6]);
});

test("Round 5.5: 右上角进度悬浮卡片支持向右滑关闭且不触发取消下载", () => {
  const cancelledIds: string[] = [];
  const view = render(
    <ProgressNotificationHost
      tasks={[
        makeTask({
          task_id: "task-swipe-1",
          name: "全球瓦片下载 ·合肥",
          status: "running",
          done_bytes: 512 * 1024,
          total_bytes: 2048 * 1024,
          created_at: new Date(Date.now() - 3000).toISOString(),
        }),
      ]}
      pendingPreparations={[]}
      onCancelTask={(task) => cancelledIds.push(task.task_id)}
    />
  );

  const card = view.getByTestId("progress-notification-card");
  fireEvent.pointerDown(card, { clientX: 100 });
  fireEvent.pointerMove(card, { clientX: 185 });
  fireEvent.pointerUp(card, { clientX: 185 });

  assert.equal(view.queryByTestId("progress-notification-card"), null);
  assert.deepEqual(cancelledIds, []);
});

test("Round 5.6: SSD 暂存目录提供文件夹选择按钮，且进度卡片剥离 INTEGRITY_DIAGNOSTIC 内部诊断串", () => {
  const engineView = render(
    <I18nProvider>
      <TransferEngineSettingsSection />
    </I18nProvider>
  );
  assert.ok(engineView.getByTestId("transfer-ssd-staging-dir-input"));
  assert.ok(engineView.getByTestId("transfer-ssd-staging-dir-browse"));
  cleanup();

  const failToast = render(
    <ProgressNotificationHost
      tasks={[
        makeTask({
          task_id: "task-err-strip-1",
          name: "Sentinel-2 L2A",
          status: "queued",
          created_at: new Date(Date.now() - 3000).toISOString(),
        }),
      ]}
      pendingPreparations={[]}
    />
  );
  failToast.rerender(
    <ProgressNotificationHost
      tasks={[
        makeTask({
          task_id: "task-err-strip-1",
          name: "Sentinel-2 L2A",
          status: "failed",
          error: {
            code: "DL001",
            message_zh: "服务器拒绝区间探测；已记录断点 INTEGRITY_DIAGNOSTIC subtype=RangeNotSatisfiable status=416",
          } as any,
          created_at: new Date(Date.now() - 3000).toISOString(),
        }),
      ]}
      pendingPreparations={[]}
    />
  );
  const cardText = failToast.getByTestId("progress-notification-card").textContent ?? "";
  assert.match(cardText, /服务器拒绝区间探测/);
  assert.equal(cardText.includes("INTEGRITY_DIAGNOSTIC"), false);
});

test("Round 5.7: 传输与并发设置无装饰行图标、对齐一致且支持三段式 SSD 暂存模式", async () => {
  const flushAsync = () => new Promise((resolve) => setTimeout(resolve, 25));
  assert.equal(
    resolveSsdStagingMode({ ssd_staging_enabled: false, ssd_staging_auto_slow_disk: false }),
    "off"
  );
  assert.equal(
    resolveSsdStagingMode({ ssd_staging_enabled: false, ssd_staging_auto_slow_disk: true }),
    "auto"
  );
  assert.equal(
    resolveSsdStagingMode({ ssd_staging_enabled: true, ssd_staging_auto_slow_disk: false }),
    "always"
  );
  assert.equal(
    resolveSsdStagingMode({ ssd_staging_enabled: true, ssd_staging_auto_slow_disk: true }),
    "always"
  );

  assert.deepEqual(ssdStagingPatchForMode("off"), {
    ssd_staging_enabled: false,
    ssd_staging_auto_slow_disk: false,
  });
  assert.deepEqual(ssdStagingPatchForMode("auto"), {
    ssd_staging_enabled: false,
    ssd_staging_auto_slow_disk: true,
  });
  assert.deepEqual(ssdStagingPatchForMode("always"), {
    ssd_staging_enabled: true,
    ssd_staging_auto_slow_disk: false,
  });

  const view = render(
    <I18nProvider>
      <TransferEngineSettingsSection />
    </I18nProvider>
  );

  const section = view.getByTestId("transfer-engine-settings-section");
  // 所有行标题不应有左侧装饰 SVG 图标，也不应有历史遗留的 pl-6 缩进
  assert.equal(section.querySelectorAll(".pl-6").length, 0);
  const nonButtonSvgs = Array.from(section.querySelectorAll("svg")).filter(
    (svg) => !svg.closest("button")
  );
  assert.equal(nonButtonSvgs.length, 0);

  // 默认 auto 模式下，显示三段式控件与 SSD 暂存路径行
  assert.ok(view.getByTestId("transfer-ssd-mode-group"));
  assert.ok(view.getByTestId("transfer-ssd-staging-dir-row"));

  // 切换至 off 隐藏路径行，再切换回 always 显示路径行
  fireEvent.click(view.getByTestId("transfer-ssd-mode-off"));
  await flushAsync();
  assert.equal(view.queryByTestId("transfer-ssd-staging-dir-row"), null);

  fireEvent.click(view.getByTestId("transfer-ssd-mode-always"));
  await flushAsync();
  assert.ok(view.getByTestId("transfer-ssd-staging-dir-row"));
});

test("Round 5.8: 二/三级与所有传输开关变更均触发统一底部提示且不出现孤立 DEM 提示", async () => {
  const flushAsync = () => new Promise((resolve) => setTimeout(resolve, 30));
  const demSynced: number[] = [];

  const view = render(
    <I18nProvider>
      <>
        <TopCenterToastHost />
        <TransferEngineSettingsSection onSyncDemConcurrency={(n) => demSynced.push(n)} />
      </>
    </I18nProvider>
  );

  const controlsToTest: Array<{ testId: string; label: string }> = [
    { testId: "transfer-tier1-tasks-6", label: "Tier-1" },
    { testId: "transfer-smart-concurrency-switch", label: "SmartConcurrency" },
    { testId: "transfer-segments-8", label: "Tier-2" },
    { testId: "transfer-tile-concurrency-32", label: "Tier-3" },
    { testId: "transfer-stall-reconnect-switch", label: "StallReconnect" },
    { testId: "transfer-ssd-mode-off", label: "SSDModeOff" },
    { testId: "transfer-ssd-mode-always", label: "SSDModeAlways" },
  ];

  for (const item of controlsToTest) {
    clearAllTopToastsForTest();
    fireEvent.click(view.getByTestId(item.testId));
    await flushAsync();
    const toasts = view.queryAllByTestId("top-center-toast");
    assert.ok(toasts.length >= 1, `${item.label} 应触发统一底部提示`);
    const toastText = toasts.map((el) => el.textContent ?? "").join(" | ");
    assert.equal(
      toastText.includes("DEM 任务并发上限已更新"),
      false,
      `${item.label} 不得触发孤立的 DEM 并发提示`
    );
  }

  assert.deepEqual(demSynced, [6]);

  // 模拟同步失败：onSyncDemConcurrency 返回 false 时只出现失败 toast，不得弹出双 toast
  cleanup();
  clearAllTopToastsForTest();
  setTransferEnginePrefs({ ...DEFAULT_TRANSFER_ENGINE_PREFS });
  const failView = render(
    <I18nProvider>
      <>
        <TopCenterToastHost />
        <TransferEngineSettingsSection onSyncDemConcurrency={async () => false} />
      </>
    </I18nProvider>
  );
  fireEvent.click(failView.getByTestId("transfer-tier1-tasks-6"));
  await flushAsync();
  const failToasts = failView.queryAllByTestId("top-center-toast");
  assert.equal(failToasts.length, 1, "同步失败时只能出现 1 条失败提示");
  const failText = failToasts[0]?.textContent ?? "";
  assert.match(failText, /同步传输引擎配置失败/);
  assert.equal(failText.includes("已更新为"), false, "同步失败时不得出现绿色成功 toast");

  // 核对 DesktopWorkbench.tsx 在 handleSaveDemConcurrency 中先拉取新鲜策略再同步 SAR 策略且无孤立 DEM 成功 toast
  const workbenchPath = nodePath.resolve(
    process.cwd(),
    "src/components/desktop/DesktopWorkbench.tsx"
  );
  const workbenchSrc = nodeFs.readFileSync(workbenchPath, "utf8");
  const handlerMatch = workbenchSrc.match(
    /const handleSaveDemConcurrency = useCallback\([\s\S]*?\n\s*\}, \[[^\]]*\]\);/
  );
  assert.ok(handlerMatch, "应能定位 handleSaveDemConcurrency");
  const handlerBody = handlerMatch[0];
  assert.match(handlerBody, /getSarDownloadPolicySettings/);
  assert.match(handlerBody, /saveSarDownloadPolicySettings/);
  assert.equal(handlerBody.includes("DEM 任务并发上限已更新"), false);
  assert.equal(handlerBody.includes("demConcurrencyUpdated"), false);
});

test("Round 5.9: 全局下载限速预设、自定义小数与清零恢复均触发统一底部提示并正确归一化", async () => {
  const flushAsync = () => new Promise((resolve) => setTimeout(resolve, 25));

  assert.equal(normalizeMaxDownloadSpeedKibS(undefined), 0);
  assert.equal(normalizeMaxDownloadSpeedKibS(-50), 0);
  assert.equal(normalizeMaxDownloadSpeedKibS(9_999_999), MAX_DOWNLOAD_SPEED_KIB_S_LIMIT);
  assert.equal(normalizeMaxDownloadSpeedKibS(5120), 5120);
  assert.equal(normalizeTransferEnginePrefs({ max_download_speed_kib_s: 2048 }).max_download_speed_kib_s, 2048);

  const view = render(
    <I18nProvider>
      <>
        <TopCenterToastHost />
        <TransferEngineSettingsSection />
      </>
    </I18nProvider>
  );

  // 确保无装饰 SVG 与无 .pl-6 的结构约束保持成立
  const section = view.getByTestId("transfer-engine-settings-section");
  assert.equal(section.querySelectorAll(".pl-6").length, 0);
  const nonButtonSvgs = Array.from(section.querySelectorAll("svg")).filter(
    (svg) => !svg.closest("button")
  );
  assert.equal(nonButtonSvgs.length, 0);

  // 点击预设 5 MB/s (5120 KiB/s)
  clearAllTopToastsForTest();
  fireEvent.click(view.getByTestId("transfer-speed-limit-5120"));
  await flushAsync();
  assert.equal(getTransferEnginePrefs().max_download_speed_kib_s, 5120);
  assert.equal((view.getByTestId("transfer-speed-limit-input") as HTMLInputElement).value, "5");
  assert.match(view.getAllByTestId("top-center-toast")[0]?.textContent ?? "", /5 MB\/s/);

  // 输入自定义小数 0.5 MB/s -> 512 KiB/s
  clearAllTopToastsForTest();
  fireEvent.change(view.getByTestId("transfer-speed-limit-input"), { target: { value: "0.5" } });
  fireEvent.click(view.getByTestId("transfer-speed-limit-apply"));
  await flushAsync();
  assert.equal(getTransferEnginePrefs().max_download_speed_kib_s, 512);
  assert.match(view.getAllByTestId("top-center-toast")[0]?.textContent ?? "", /512 KiB\/s/);

  // 输入无效负数应被拒绝且不覆盖当前生效值
  clearAllTopToastsForTest();
  fireEvent.change(view.getByTestId("transfer-speed-limit-input"), { target: { value: "-2" } });
  fireEvent.click(view.getByTestId("transfer-speed-limit-apply"));
  await flushAsync();
  assert.equal(getTransferEnginePrefs().max_download_speed_kib_s, 512);
  assert.match(view.getAllByTestId("top-center-toast")[0]?.textContent ?? "", /请输入不小于 0/);

  // 输入极小正小数 0.0001 MB/s -> 归一化 1 KiB/s 且显示非零 "0.001"，再次点应用不重置为 0
  clearAllTopToastsForTest();
  fireEvent.change(view.getByTestId("transfer-speed-limit-input"), { target: { value: "0.0001" } });
  fireEvent.click(view.getByTestId("transfer-speed-limit-apply"));
  await flushAsync();
  assert.equal(getTransferEnginePrefs().max_download_speed_kib_s, 1);
  assert.equal((view.getByTestId("transfer-speed-limit-input") as HTMLInputElement).value, "0.001");
  fireEvent.click(view.getByTestId("transfer-speed-limit-apply"));
  await flushAsync();
  assert.equal(getTransferEnginePrefs().max_download_speed_kib_s, 1);

  // 输入 0.25 MB/s 验证 step="any" 小数输入与回显
  clearAllTopToastsForTest();
  fireEvent.change(view.getByTestId("transfer-speed-limit-input"), { target: { value: "0.25" } });
  fireEvent.click(view.getByTestId("transfer-speed-limit-apply"));
  await flushAsync();
  assert.equal(getTransferEnginePrefs().max_download_speed_kib_s, 256);
  assert.equal((view.getByTestId("transfer-speed-limit-input") as HTMLInputElement).value, "0.25");

  // 留空或点击不限速恢复 0
  clearAllTopToastsForTest();
  fireEvent.change(view.getByTestId("transfer-speed-limit-input"), { target: { value: "" } });
  fireEvent.click(view.getByTestId("transfer-speed-limit-apply"));
  await flushAsync();
  assert.equal(getTransferEnginePrefs().max_download_speed_kib_s, 0);
  assert.match(view.getAllByTestId("top-center-toast")[0]?.textContent ?? "", /不限速/);
});
