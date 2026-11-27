import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { act, within } from "@testing-library/react";
import { render, cleanup, fireEvent, setupUser } from "../../test-utils/render-harness";
import { DesktopDownloadDrawer } from "./DesktopDownloadDrawer";
import type { ApplicationTask } from "../../shell/api/types";
import {
  clearTestDesktopShellBridge,
  registerTestDesktopShellBridge,
  type DesktopShellBridge,
  type DesktopTaskPageRequest,
} from "../../lib/desktopShellBridge";
import { MAX_UI_TASK_PAGE_SIZE } from "../../lib/coreRestoreProjection";
import { formatOrbitCheckTime as formatOrbitTime } from "../../shell/api/orbitChildRows";

test.afterEach(() => {
  cleanup();
  clearTestDesktopShellBridge();
});

async function settle() {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
}

function task(revision = 7): ApplicationTask {
  return {
    task_id: "task-scene-progress",
    revision,
    category: "scene_download",
    name: "Sentinel-1 批次",
    provider: "asf",
    dataset: "s1_slc",
    status: "running",
    items_count: 104,
    completed_items_count: 0,
    failed_items_count: 0,
    done_bytes: 512,
    total_bytes: 2048,
    scene_ids: ["FALLBACK_A", "FALLBACK_B"],
  };
}

function pageItem(
  displayName: string,
  bytesCompleted: number,
  totalBytes = 1024,
) {
  return {
    ordinal: 0,
    item: {
      item_id: `item-${displayName}`,
      provider_id: "asf",
      display_name: displayName,
      acquisition_datetime: null,
      footprint_geojson: null,
      metadata: {},
      payload: {},
      asset_access: null,
      execution_policy: null,
    },
    checked: true,
    status: "active",
    bytes_completed: bytesCompleted,
    total_bytes: totalBytes,
    source_total_bytes: totalBytes,
    bytes_per_second: 0,
    attempt: 0,
    destination: null,
    failure: null,
    finished_at: null,
    retryable: false,
    local_reuse: null,
  };
}

function taskPage(
  revision: number,
  items: ReturnType<typeof pageItem>[],
  totalCount = items.length,
) {
  return {
    task_id: "task-scene-progress",
    revision,
    page_index: 0,
    page_size: MAX_UI_TASK_PAGE_SIZE,
    total_count: totalCount,
    source_total_bytes: 2048,
    network_bytes_received: 512,
    items,
  };
}

function installPageBridge(
  loadPage: (request: DesktopTaskPageRequest) => Promise<ReturnType<typeof taskPage>>,
) {
  const calls: DesktopTaskPageRequest[] = [];
  registerTestDesktopShellBridge({
    kind: "browser_fixture",
    application: {
      taskPage: async (request: DesktopTaskPageRequest) => {
        calls.push(request);
        return { ok: true, page: await loadPage(request) };
      },
    },
  } as unknown as DesktopShellBridge);
  return calls;
}

test("展开分景进度后加载首页并显示每景字节和未显示数量", async () => {
  const calls = installPageBridge(async () => taskPage(7, [
    pageItem("SCENE_A", 512),
    pageItem("SCENE_B", 256),
  ], 104));
  const initial = task();
  const view = render(<DesktopDownloadDrawer open variant="page" tasks={[initial]} />);

  fireEvent.click(view.getByRole("button", { name: "展开分景进度" }));
  assert.ok(view.getByText("FALLBACK_A"), "加载期间保留原有场景清单");
  await settle();

  assert.deepEqual(calls, [{
    task_id: initial.task_id,
    expected_revision: 7,
    page_index: 0,
    page_size: MAX_UI_TASK_PAGE_SIZE,
  }]);
  assert.ok(view.getByText("SCENE_A"));
  assert.ok(view.getByText("512 B / 1.0 KB"));
  assert.ok(view.getByText("256 B / 1.0 KB"));
  assert.ok(view.getByText("还有 102 景未显示"));
  assert.equal(view.queryByText("FALLBACK_A"), null);
});

test("旧修订的迟到响应不会覆盖当前分景进度", async () => {
  const pending: Array<{
    request: DesktopTaskPageRequest;
    resolve: (page: ReturnType<typeof taskPage>) => void;
  }> = [];
  installPageBridge((request) => new Promise((resolve) => {
    pending.push({ request, resolve });
  }));
  const initial = task();
  const view = render(<DesktopDownloadDrawer open variant="page" tasks={[initial]} />);
  fireEvent.click(view.getByRole("button", { name: "展开分景进度" }));
  await settle();
  assert.equal(pending.length, 1);
  assert.equal(pending[0].request.expected_revision, 7);

  view.rerender(<DesktopDownloadDrawer open variant="page" tasks={[{ ...initial, revision: 8 }]} />);
  await settle();
  assert.equal(pending.length, 2);
  assert.equal(pending[1].request.expected_revision, 8);

  await act(async () => {
    pending[1].resolve(taskPage(8, [pageItem("SCENE_NEW", 768)]));
    await Promise.resolve();
  });
  await settle();
  assert.ok(view.getByText("SCENE_NEW"));
  assert.ok(view.getByText("768 B / 1.0 KB"));

  await act(async () => {
    pending[0].resolve(taskPage(7, [pageItem("SCENE_OLD", 128)]));
    await Promise.resolve();
  });
  await settle();
  assert.equal(view.queryByText("SCENE_OLD"), null);
  assert.ok(view.getByText("SCENE_NEW"));
  assert.ok(view.getByText("768 B / 1.0 KB"));
});

function orbitSourceTask(revision = 3): ApplicationTask {
  return {
    ...task(revision),
    task_id: "source-image",
    items_count: 3,
    scene_ids: ["SCENE_A", "SCENE_B", "SCENE_C"],
    status: "running",
  };
}

function orbitCompanionTask(revision = 5, sourceTaskId: string | null = "source-image"): ApplicationTask {
  return {
    ...task(revision),
    task_id: "companion-without-source-prefix",
    items_count: 3,
    scene_ids: ["SCENE_A", "SCENE_B", "SCENE_C"],
    status: "queued",
    auxiliary_kind: "AUX_POEORB",
    source_task_id: sourceTaskId,
  };
}

function orbitPage(taskId: string, revision: number, statuses = ["completed", "completed", "queued"]) {
  return {
    ...taskPage(revision, statuses.map((status, index) => ({
      ...pageItem(["SCENE_A", "SCENE_B", "SCENE_C"][index], 0),
      ordinal: index,
      status,
      orbit_recheck: {
        orbit_type: "POEORB",
        state: "waiting",
        attempt: 1,
        next_check_at: "2026-11-22T00:00:00Z",
        deadline_at: "2026-12-25T00:00:00Z",
        expected_file_name: status === "completed" ? `ORBIT_${index}.EOF` : null,
      },
    })), 3),
    task_id: taskId,
  } as ReturnType<typeof taskPage> & { task_id: string };
}

const ORBIT_CHECK_TIME = formatOrbitTime({
  scene_id: "SCENE_C",
  status: "queued",
  done_bytes: 0,
  orbit_recheck: {
    orbit_type: "POEORB",
    state: "waiting",
    attempt: 1,
    next_check_at: "2026-11-22T00:00:00Z",
    deadline_at: "2026-12-25T00:00:00Z",
  },
})!;

function installOrbitBridge({
  pages = new Map<string, ReturnType<typeof taskPage>>(),
  update,
}: {
  pages?: Map<string, ReturnType<typeof taskPage>>;
  update?: (request: { task_id: string; expected_revision: number; command: { kind: string; item_ids: string[] } }) => Promise<unknown>;
} = {}) {
  const pageCalls: DesktopTaskPageRequest[] = [];
  const updates: Array<{ task_id: string; expected_revision: number; command: { kind: string; item_ids: string[] } }> = [];
  registerTestDesktopShellBridge({
    kind: "browser_fixture",
    application: {
      taskPage: async (request: DesktopTaskPageRequest) => {
        pageCalls.push(request);
        const page = pages.get(request.task_id);
        if (!page) throw new Error("page unavailable");
        return { ok: true, page: { ...page, task_id: request.task_id } };
      },
      updateDownloadTask: async (request: { task_id: string; expected_revision: number; command: { kind: string; item_ids: string[] } }) => {
        updates.push(request);
        return update ? update(request) : { ok: true };
      },
    },
  } as unknown as DesktopShellBridge);
  return { pageCalls, updates };
}

test("三景附属行紧跟所属景，单景也可展开，独立回退不缩进且不收起任务徽章", async () => {
  const source = orbitSourceTask();
  const companion = orbitCompanionTask();
  installOrbitBridge({
    pages: new Map([
      [source.task_id, taskPage(source.revision, [pageItem("SCENE_A", 100), pageItem("SCENE_B", 100), pageItem("SCENE_C", 100)])],
      [companion.task_id, orbitPage(companion.task_id, companion.revision)],
    ]),
  });
  const view = render(<DesktopDownloadDrawer open variant="page" tasks={[source, companion]} />);
  const group = view.getByRole("group", { name: "Sentinel-1 批次 的任务组" });
  assert.ok(group);
  assert.match(group.textContent ?? "", /\(1\)|Sentinel-1 批次/);
  assert.ok(view.getByText("(1)"));
  fireEvent.click(view.getByRole("button", { name: "展开分景与配套数据" }));
  await settle();
  assert.equal(view.getAllByText("AUX_POEORB").length, 3);
  assert.ok(view.getByText("待补"));
  assert.ok(view.getByText(ORBIT_CHECK_TIME));
  const listItems = view.getAllByRole("listitem");
  assert.deepEqual(listItems.slice(0, 6).map((row) => row.querySelector('button[aria-label="AUX_POEORB，查看文件信息"]') !== null), [
    false,
    true,
    false,
    true,
    false,
    true,
  ]);
  assert.deepEqual(listItems.slice(0, 6).map((row) => row.querySelector('[title^="SCENE_"]')?.getAttribute("title") ?? null), [
    "SCENE_A",
    null,
    "SCENE_B",
    null,
    "SCENE_C",
    null,
  ]);
  const expand = view.getByRole("button", { name: "收起分景与配套数据" });
  assert.equal(expand.getAttribute("aria-controls"), view.getByRole("list", { name: "精密轨道" }).id);
  fireEvent.click(expand);
  assert.equal(view.queryByText("AUX_POEORB"), null);
  fireEvent.click(view.getByRole("button", { name: "展开分景与配套数据" }));
  await settle();
  assert.equal(view.getAllByText("AUX_POEORB").length, 3);

  view.rerender(<DesktopDownloadDrawer open variant="page" tasks={[orbitCompanionTask(5, "archived-image")]} />);
  await settle();
  assert.equal(view.queryByRole("group"), null);
  assert.ok(view.getByText("排队中"));
  fireEvent.click(view.getByRole("button", { name: "展开分景与配套数据" }));
  await settle();
  assert.equal(view.getAllByText("AUX_POEORB").length, 3);
});

test("轨道文件 tooltip 支持点击、键盘 Escape 和真实 EOF，行级动作只发配套条目命令", async () => {
  const source = orbitSourceTask();
  const companion = orbitCompanionTask();
  let releaseUpdate: (() => void) | null = null;
  const { updates } = installOrbitBridge({
    pages: new Map([
      [source.task_id, taskPage(source.revision, [pageItem("SCENE_A", 100), pageItem("SCENE_B", 100), pageItem("SCENE_C", 100)])],
      [companion.task_id, orbitPage(companion.task_id, companion.revision)],
    ]),
    update: () => new Promise((resolve) => { releaseUpdate = () => resolve({ ok: true }); }),
  });
  const sourceActions: string[] = [];
  const user = setupUser();
  const view = render(<DesktopDownloadDrawer open variant="page" tasks={[source, companion]} onTaskAction={async (id) => { sourceActions.push(id); }} />);
  fireEvent.click(view.getByRole("button", { name: "展开分景与配套数据" }));
  await settle();

  const fileInfo = view.getAllByRole("button", { name: "AUX_POEORB，查看文件信息" })[0];
  fireEvent.click(fileInfo);
  const tooltipId = fileInfo.getAttribute("aria-describedby")!;
  assert.ok(document.getElementById(tooltipId)?.textContent?.includes("ORBIT_0.EOF"));
  assert.equal(document.getElementById(tooltipId) !== null, true);
  fireEvent.keyDown(fileInfo, { key: "Escape" });
  assert.equal(document.getElementById(tooltipId), null);
  assert.equal(fileInfo.getAttribute("aria-expanded"), "false");
  // Dual filter bars (status + dataset) both expose an "全部" pill; folded AUX_POEORB companion counts with its parent group so the dataset filter bar shows "全部1". Start keyboard tabbing from that pill preceding the task rows.
  within(view.getByTestId("download-dataset-filter-bar")).getByRole("button", { name: "全部1" }).focus();
  let reachedFileInfo = false;
  for (let index = 0; index < 8; index += 1) {
    await user.tab();
    if (fileInfo.matches(":focus")) {
      reachedFileInfo = true;
      break;
    }
  }
  assert.equal(reachedFileInfo, true);
  assert.equal(document.getElementById(tooltipId) !== null, true);
  assert.equal(fileInfo.getAttribute("aria-describedby"), tooltipId);
  fireEvent.keyDown(fileInfo, { key: "Escape" });

  const check = view.getByRole("button", { name: "立即检查" });
  fireEvent.click(check);
  const pendingCheck = view.getByRole("button", { name: "正在提交检查" });
  assert.equal(pendingCheck.disabled, true);
  assert.equal(view.getByRole("button", { name: "取消" }).disabled, true);
  assert.equal(pendingCheck.closest("[aria-busy]")?.getAttribute("aria-busy"), "true");
  await user.tab();
  assert.equal(pendingCheck.className.includes("focus:opacity-100"), true);
  assert.equal(updates.length, 1);
  assert.equal(updates[0].task_id, companion.task_id);
  assert.equal(updates[0].expected_revision, companion.revision);
  assert.deepEqual(updates[0].command, { kind: "retry_items", item_ids: ["item-SCENE_C"] });
  assert.deepEqual(sourceActions, []);
  await act(async () => { releaseUpdate?.(); await Promise.resolve(); });
  await settle();
  assert.ok(view.getByText("已提交立即检查"));

  fireEvent.click(view.getByRole("button", { name: "取消" }));
  assert.deepEqual(updates[1].command, { kind: "cancel_items", item_ids: ["item-SCENE_C"] });
  assert.equal(view.queryByRole("alertdialog"), null);
  await act(async () => { releaseUpdate?.(); await Promise.resolve(); });
  await settle();
  assert.ok(view.getByText("已提交取消"));
  assert.deepEqual(sourceActions, []);
});
test("轨道明细失败可重试，空明细不等于已下载，动作失败保留时间并显示行内错误", async () => {
  const source = orbitSourceTask();
  const companion = orbitCompanionTask();
  let allowOrbitPage = false;
  let empty = false;
  let orbitRevision = companion.revision;
  const { pageCalls } = installOrbitBridge({
    pages: new Map([[source.task_id, taskPage(source.revision, [pageItem("SCENE_A", 100), pageItem("SCENE_B", 100), pageItem("SCENE_C", 100)])]]),
    update: async () => ({ ok: false, error: "轨道服务连接失败", code: "ORB001" }),
  });
  const originalBridge = (await import("../../lib/desktopShellBridge")).getDesktopShellBridge;
  const originalApplication = originalBridge().application;
  const orbitUpdates: Array<{ task_id: string; expected_revision: number; command: { kind: string; item_ids: string[] } }> = [];
  registerTestDesktopShellBridge({
    kind: "browser_fixture",
    application: {
      ...originalApplication,
      updateDownloadTask: async (request: { task_id: string; expected_revision: number; command: { kind: string; item_ids: string[] } }) => {
        orbitUpdates.push(request);
        return { ok: false, error: "轨道服务连接失败", code: "ORB001" };
      },
      taskPage: async (request: DesktopTaskPageRequest) => {
        pageCalls.push(request);
        if (request.task_id === companion.task_id) {
          if (!allowOrbitPage) throw new Error("page unavailable");
          if (empty) return { ok: true, page: { ...taskPage(companion.revision, [], 0), task_id: request.task_id } };
          return { ok: true, page: { ...orbitPage(companion.task_id, orbitRevision), task_id: request.task_id } };
        }
        return { ok: true, page: { ...taskPage(source.revision, [pageItem("SCENE_A", 100), pageItem("SCENE_B", 100), pageItem("SCENE_C", 100)]), task_id: request.task_id } };
      },
    },
  } as unknown as DesktopShellBridge);
  const view = render(<DesktopDownloadDrawer open variant="page" tasks={[source, companion]} />);
  fireEvent.click(view.getByRole("button", { name: "展开分景与配套数据" }));
  await settle();
  assert.ok(view.getByText("精密轨道明细不可用"));
  assert.equal(view.queryByText("已下载"), null);
  allowOrbitPage = true;
  empty = true;
  fireEvent.click(view.getByRole("button", { name: "重试精密轨道明细" }));
  await settle();
  assert.ok(view.getByText("暂无精密轨道明细"));
  assert.equal(view.queryByText("已下载"), null);

  allowOrbitPage = true;
  empty = false;
  orbitRevision = 6;
  view.rerender(<DesktopDownloadDrawer open variant="page" tasks={[source, { ...companion, revision: 6 }]} />);
  await settle();
  assert.ok(view.getByText(ORBIT_CHECK_TIME));
  fireEvent.click(view.getByRole("button", { name: "立即检查" }));
  await settle();
  assert.equal(orbitUpdates.length, 1);
  assert.equal(orbitUpdates[0].expected_revision, 6);
  assert.ok(view.getByRole("alert").textContent?.includes("轨道服务连接失败"));
  assert.ok(view.getByText(ORBIT_CHECK_TIME));
  assert.equal(view.getByRole("button", { name: "立即检查" }).disabled, false);
  assert.equal(pageCalls.filter((call) => call.task_id === companion.task_id).length >= 3, true);
});

test("S7：待补行提供「临时用 RESORB」显式手动选项，完成后标明「已用 RESORB」并支持「升级为精轨」", async () => {
  const source = orbitSourceTask();
  const companion = orbitCompanionTask();
  const resorbDest =
    "E:\\Downloads\\Sentinel_Orbit\\AUX_RESORB\\S1A_OPER_AUX_RESORB_OPOD_20260312T140000_V20260312T095942_20260312T131712.EOF";
  const pageWithResorb = {
    ...taskPage(
      companion.revision,
      [
        {
          ...pageItem("SCENE_A", 100),
          ordinal: 0,
          status: "completed" as const,
          destination: resorbDest,
        },
        {
          ...pageItem("SCENE_B", 0),
          ordinal: 1,
          status: "queued" as const,
          orbit_recheck: {
            orbit_type: "POEORB",
            state: "waiting",
            attempt: 1,
            next_check_at: "2026-11-22T00:00:00Z",
            deadline_at: "2026-12-25T00:00:00Z",
            expected_file_name: null,
          },
        },
      ],
      2
    ),
    task_id: companion.task_id,
  };
  const updates: Array<{
    task_id: string;
    expected_revision: number;
    command: { kind: string; item_ids: string[] };
    event_id?: string;
  }> = [];
  registerTestDesktopShellBridge({
    kind: "browser_fixture",
    application: {
      taskPage: async (request: DesktopTaskPageRequest) => {
        if (request.task_id === companion.task_id) {
          return { ok: true, page: pageWithResorb };
        }
        return {
          ok: true,
          page: {
            ...taskPage(source.revision, [pageItem("SCENE_A", 100), pageItem("SCENE_B", 100)], 2),
            task_id: source.task_id,
          },
        };
      },
      updateDownloadTask: async (request: any) => {
        updates.push(request);
        return { ok: true };
      },
    },
  } as unknown as DesktopShellBridge);

  const view = render(<DesktopDownloadDrawer open variant="page" tasks={[source, companion]} />);
  fireEvent.click(view.getByRole("button", { name: "展开分景与配套数据" }));
  await settle();

  // SCENE_A 显示 AUX_RESORB 与「已用 RESORB」状态，并提供「升级为精轨」按钮
  assert.ok(view.getByText("AUX_RESORB"));
  assert.ok(view.getByText("已用 RESORB"));
  const upgradeBtn = view.getByRole("button", { name: "升级为精轨" });
  assert.ok(upgradeBtn);

  // SCENE_B 处于待补，提供「临时用 RESORB」按钮
  const useResorbBtn = view.getByRole("button", { name: "临时用 RESORB" });
  fireEvent.click(useResorbBtn);
  await settle();
  assert.equal(updates.length, 1);
  assert.deepEqual(updates[0].command, { kind: "retry_items", item_ids: ["item-SCENE_B"] });
  assert.equal(updates[0].event_id?.startsWith("orbit-resorb-"), true);
  assert.ok(view.getByText("已提交临时 RESORB 获取"));

  // 点击 SCENE_A 的「升级为精轨」按钮
  fireEvent.click(upgradeBtn);
  await settle();
  assert.equal(updates.length, 2);
  assert.deepEqual(updates[1].command, { kind: "retry_items", item_ids: ["item-SCENE_A"] });
  assert.equal(updates[1].event_id?.startsWith("orbit-upgrade-"), true);
  assert.ok(view.getByText("已提交精轨升级请求"));
});
