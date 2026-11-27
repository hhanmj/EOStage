import assert from "node:assert/strict";
import test from "node:test";
import type { AgentWorkItem } from "../../lib/generated/v3-contracts.ts";
import type { ApplicationTask } from "./types.ts";
import {
  createdTaskCount,
  filterWork,
  formatDuration,
  joinRunTasks,
  runDurationMs,
  sortWorkNewestFirst,
  taskProgressPercent,
  taskStatusLabel,
  workCounts,
  workTitle,
} from "./agentWork.ts";

function work(id: string, status: AgentWorkItem["run"]["status"], started: string, ended: string | null, tasks: string[] = [], request: string | null = `请求 ${id}`): AgentWorkItem {
  return {
    run: { run_id: id, session_id: "s1", status, started_at: started, ended_at: ended, model_route: "ch/model" },
    request,
    plan_ids: [],
    created_task_ids: tasks,
  };
}

function task(id: string, status: ApplicationTask["status"], done = 0, total: number | null = null): ApplicationTask {
  return { task_id: id, revision: 1, category: "scene_download", name: id, status, done_bytes: done, total_bytes: total };
}

const items = [
  work("r1", "completed", "2026-10-30T10:00:00Z", "2026-10-30T10:01:05Z", ["t1", "t2"]),
  work("r3", "running", "2026-10-30T12:00:00Z", null, ["t3"]),
  work("r2", "failed", "2026-10-30T11:00:00Z", "2026-10-30T11:00:30Z", ["t2"]),
];

test("sortWorkNewestFirst 按开始时间新的在前", () => {
  assert.deepEqual(sortWorkNewestFirst(items).map((i) => i.run.run_id), ["r3", "r2", "r1"]);
});

test("sortWorkNewestFirst 时间无法解析时保持原顺序", () => {
  const bad = [work("a", "completed", "x", null), work("b", "completed", "y", null)];
  assert.deepEqual(sortWorkNewestFirst(bad).map((i) => i.run.run_id), ["a", "b"]);
});

test("filterWork 与 workCounts：进行中只算 running", () => {
  assert.deepEqual(filterWork(items, "active").map((i) => i.run.run_id), ["r3"]);
  assert.deepEqual(filterWork(items, "ended").map((i) => i.run.run_id), ["r1", "r2"]);
  assert.equal(filterWork(items, "all").length, 3);
  assert.deepEqual(workCounts(items), { all: 3, active: 1, ended: 2 });
});

test("createdTaskCount 跨运行去重", () => {
  assert.equal(createdTaskCount(items), 3);
  assert.equal(createdTaskCount([]), 0);
  assert.equal(createdTaskCount(null), 0);
});

test("joinRunTasks 按 task_id 关联，缺失为 null", () => {
  const rows = joinRunTasks(items[0], [task("t1", "running", 5, 10)]);
  assert.equal(rows.length, 2);
  assert.equal(rows[0].task?.status, "running");
  assert.equal(rows[1].task, null);
  assert.equal(taskStatusLabel(rows[1].task), "已清理");
  assert.equal(taskStatusLabel(rows[0].task), "下载中");
});

test("joinRunTasks 去除重复 id", () => {
  const rows = joinRunTasks(work("x", "completed", "2026-10-30T10:00:00Z", null, ["t1", "t1"]), []);
  assert.equal(rows.length, 1);
});

test("runDurationMs 与 formatDuration", () => {
  assert.equal(runDurationMs(items[0], 0), 65_000);
  assert.equal(formatDuration(65_000), "1 分 05 秒");
  assert.equal(formatDuration(30_000), "30 秒");
  assert.equal(formatDuration(3_725_000), "1 时 02 分");
  const now = Date.parse("2026-10-30T12:00:10Z");
  assert.equal(runDurationMs(items[1], now), 10_000);
  assert.equal(runDurationMs(work("i", "interrupted", "2026-10-30T10:00:00Z", null), now), null);
  assert.equal(formatDuration(null), null);
});

test("taskProgressPercent 仅在有总量时确定", () => {
  assert.equal(taskProgressPercent(task("a", "running", 25, 100)), 25);
  assert.equal(taskProgressPercent(task("a", "running", 25, null)), null);
  assert.equal(taskProgressPercent(task("a", "completed", 0, null)), 100);
  assert.equal(taskProgressPercent(null), null);
});

test("workTitle 无请求时用占位", () => {
  assert.equal(workTitle(work("a", "completed", "", null, [], "  ")), "未记录请求");
  assert.equal(workTitle(work("a", "completed", "", null, [], " 下载 ")), "下载");
});
