import assert from "node:assert/strict";
import test from "node:test";

import type { OrbitRecheckInfo } from "../../lib/generated/v3-contracts.ts";
import type { ApplicationTask, TaskSceneItem } from "./types.ts";
import {
  formatOrbitCheckTime,
  getAvailableOrbitRowActions,
  groupOrbitCompanionTasks,
  isOrbitPublicationLate,
  orbitFileInfoLines,
  orbitRowStatus,
  resolvedOrbitType,
} from "./orbitChildRows.ts";

function task(taskId: string, overrides: Partial<ApplicationTask> = {}): ApplicationTask {
  return {
    task_id: taskId,
    revision: 1,
    category: "scene_download",
    name: taskId,
    status: "running",
    done_bytes: 0,
    total_bytes: null,
    ...overrides,
  };
}

function companion(taskId: string, sourceTaskId: string | null): ApplicationTask {
  return task(taskId, {
    auxiliary_kind: "AUX_POEORB",
    source_task_id: sourceTaskId,
  });
}

function recheck(overrides: Partial<OrbitRecheckInfo> = {}): OrbitRecheckInfo {
  return {
    orbit_type: "POEORB",
    state: "waiting",
    attempt: 1,
    next_check_at: "2026-11-22T00:00:00Z",
    deadline_at: "2026-12-25T00:00:00Z",
    expected_file_name: null,
    ...overrides,
  };
}

function orbitItem(
  status: TaskSceneItem["status"],
  orbitRecheck: OrbitRecheckInfo | null = null,
): TaskSceneItem {
  return {
    item_id: "orbit-item-1",
    scene_id: "S1A_SCENE_001",
    status,
    done_bytes: 0,
    total_bytes: null,
    orbit_recheck: orbitRecheck,
  };
}

test("orbit grouping uses projected identity, preserves companion order and never counts children twice", () => {
  const source = task("image", { items_count: 3, completed_items_count: 2, total_bytes: 300 });
  const first = companion("unrelated-prefix", "image");
  const second = companion("image-orbit-not-an-identity-key", "image");
  const idLookalike = task("image-orbit", { auxiliary_kind: null, source_task_id: null });
  const rows = groupOrbitCompanionTasks([first, source, second, idLookalike]);

  assert.deepEqual(rows.map(({ task: entry }) => entry.task_id), [
    "image",
    "image-orbit",
  ]);
  assert.deepEqual(rows[0].companions.map((entry) => entry.task_id), [
    "unrelated-prefix",
    "image-orbit-not-an-identity-key",
  ]);
  assert.equal(rows[0].task, source);
  assert.equal(rows[0].task.items_count, 3);
  assert.equal(rows[0].task.total_bytes, 300);
  assert.deepEqual(rows[1].companions, []);
});

test("missing, archived or filtered sources leave orbit tasks as independent rows", () => {
  const orphan = companion("orbit", "not-in-this-partition");
  const rows = groupOrbitCompanionTasks([orphan]);
  assert.deepEqual(rows, [{ task: orphan, companions: [] }]);

  const otherPartition = groupOrbitCompanionTasks([
    task("image", { status: "completed" }),
    orphan,
  ]);
  assert.equal(otherPartition.length, 2);
  assert.deepEqual(otherPartition[1], { task: orphan, companions: [] });
});

test("orbit row state priority preserves real transfer state and uses a separate light-weight vocabulary", () => {
  const waiting = recheck();
  const cases: Array<[TaskSceneItem, string]> = [
    [orbitItem("completed", waiting), "已下载"],
    [orbitItem("running", waiting), "下载中"],
    [orbitItem("paused", waiting), "已暂停"],
    [orbitItem("cancelled", waiting), "已取消"],
    [orbitItem("queued", recheck({ state: "cancelled" })), "已取消"],
    [orbitItem("failed", waiting), "失败"],
    [orbitItem("queued", recheck({ state: "exhausted" })), "失败"],
    [orbitItem("queued", waiting), "待补"],
    [orbitItem("queued", recheck({ state: "waiting", next_check_at: "" })), "待补"],
    [orbitItem("queued", recheck({ state: "cancelled", next_check_at: "" })), "已取消"],
    [orbitItem("queued", null), "待检查"],
    [orbitItem("interrupted", waiting), "已中断"],
  ];
  for (const [item, expected] of cases) {
    assert.equal(orbitRowStatus(item).label, expected);
  }
});

test("next check uses the backend UTC value in local time without a countdown or invented value", () => {
  const expected = new Date("2026-11-22T00:00:00Z");
  const pad = (value: number) => String(value).padStart(2, "0");
  const local = `${pad(expected.getMonth() + 1)}-${pad(expected.getDate())} ${pad(expected.getHours())}:${pad(expected.getMinutes())} 检查`;
  assert.equal(formatOrbitCheckTime(orbitItem("queued", recheck())), local);
  assert.equal(formatOrbitCheckTime(orbitItem("running", recheck())), null);
  assert.equal(formatOrbitCheckTime(orbitItem("queued", recheck({ state: "exhausted" }))), null);
  assert.equal(formatOrbitCheckTime(orbitItem("queued", recheck({ next_check_at: "" }))), null);
  assert.equal(formatOrbitCheckTime(orbitItem("queued", null)), null);
});

test("publication warning starts at deadline minus 35 days and never turns an item failed in the UI", () => {
  const value = recheck({ deadline_at: "2026-12-25T00:00:00Z" });
  const threshold = Date.parse(value.deadline_at) - 35 * 24 * 60 * 60 * 1000;
  assert.equal(isOrbitPublicationLate(value, threshold - 1), false);
  assert.equal(isOrbitPublicationLate(value, threshold), true);
  assert.equal(isOrbitPublicationLate(recheck({ state: "exhausted", deadline_at: value.deadline_at }), threshold + 1), false);
  assert.equal(orbitRowStatus(orbitItem("queued", recheck({ deadline_at: "2020-01-01T00:00:00Z" }))).label, "待补");
});

test("file information uses only the real EOF, scene fallback and backend failure explanation", () => {
  const eof = "S1A_OPER_AUX_POEORB_OPOD_20261122T080000_V20261025T000000_20261027T000000.EOF";
  assert.deepEqual(orbitFileInfoLines(orbitItem("completed", recheck({ expected_file_name: eof }))), [eof]);
  assert.deepEqual(orbitFileInfoLines(orbitItem("queued", recheck())), [
    "S1A_SCENE_001",
    "完整文件名在发布后可见",
  ]);
  assert.deepEqual(orbitFileInfoLines(orbitItem("failed", recheck({ state: "exhausted" }))), [
    "S1A_SCENE_001",
    "完整文件名在发布后可见",
    "超过 60 天仍未发布，自动检查已停止（ORB003）",
  ]);
  assert.deepEqual(orbitFileInfoLines({
    ...orbitItem("failed", recheck()),
    failure: { code: "ORB001", reason_zh: "轨道服务连接失败", retryable: true },
  }), [
    "S1A_SCENE_001",
    "完整文件名在发布后可见",
    "轨道服务连接失败",
  ]);
});

test("row action matrix preserves single-item scopes across pause, stale pages, loading failure and terminal states", () => {
  const normal = { sourcePaused: false, companionPaused: false, revisionChanged: false, inFlight: false, loadFailed: false };
  const waiting = orbitItem("queued", recheck());
  assert.deepEqual(getAvailableOrbitRowActions(waiting, normal), [
    { kind: "check", disabled: false },
    { kind: "use_resorb", disabled: false },
    { kind: "cancel", disabled: false },
  ]);
  assert.deepEqual(getAvailableOrbitRowActions(waiting, { ...normal, sourcePaused: true }), [
    { kind: "check", disabled: true },
    { kind: "use_resorb", disabled: true },
    { kind: "cancel", disabled: false },
  ]);
  assert.deepEqual(getAvailableOrbitRowActions(waiting, { ...normal, companionPaused: true }), [
    { kind: "check", disabled: true },
    { kind: "use_resorb", disabled: true },
    { kind: "cancel", disabled: false },
  ]);
  for (const blocked of ["revisionChanged", "inFlight", "loadFailed"] as const) {
    assert.deepEqual(getAvailableOrbitRowActions(waiting, { ...normal, [blocked]: true }), [
      { kind: "check", disabled: true },
      { kind: "use_resorb", disabled: true },
      { kind: "cancel", disabled: true },
    ]);
  }
  assert.deepEqual(getAvailableOrbitRowActions(orbitItem("queued"), normal), [
    { kind: "cancel", disabled: false },
  ]);
  assert.deepEqual(getAvailableOrbitRowActions(orbitItem("running", recheck()), normal), [
    { kind: "cancel", disabled: false },
  ]);
  assert.deepEqual(getAvailableOrbitRowActions(orbitItem("paused", recheck()), normal), [
    { kind: "cancel", disabled: false },
  ]);
  assert.deepEqual(getAvailableOrbitRowActions(orbitItem("failed", recheck()), normal), [
    { kind: "check", disabled: false },
    { kind: "use_resorb", disabled: false },
  ]);
  assert.deepEqual(getAvailableOrbitRowActions(orbitItem("queued", recheck({ state: "exhausted" })), normal), [
    { kind: "check", disabled: false },
    { kind: "use_resorb", disabled: false },
  ]);
  for (const status of ["completed", "cancelled", "interrupted"] as const) {
    assert.deepEqual(getAvailableOrbitRowActions(orbitItem(status, recheck()), normal), []);
  }
  // S7: completed RESORB item marks status as "已用 RESORB" and exposes "upgrade_poeorb"
  const completedResorb: TaskSceneItem = {
    ...orbitItem("completed", null),
    destination: "E:\\Downloads\\Sentinel_Orbit\\AUX_RESORB\\S1A_OPER_AUX_RESORB_OPOD_20260312T140000_V20260312T095942_20260312T131712.EOF",
  };
  assert.equal(resolvedOrbitType(completedResorb), "RESORB");
  assert.equal(orbitRowStatus(completedResorb).label, "已用 RESORB");
  assert.deepEqual(
    orbitFileInfoLines(completedResorb),
    ["S1A_OPER_AUX_RESORB_OPOD_20260312T140000_V20260312T095942_20260312T131712.EOF"]
  );
  assert.deepEqual(getAvailableOrbitRowActions(completedResorb, normal), [
    { kind: "upgrade_poeorb", disabled: false },
  ]);
});
