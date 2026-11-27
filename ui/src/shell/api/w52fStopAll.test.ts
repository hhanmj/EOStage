import test from "node:test";
import assert from "node:assert/strict";
import type {
  AgentStopAllResult,
  AgentStreamEvent,
} from "../../lib/generated/v3-contracts.ts";
import type { ApplicationTask } from "./types.ts";
import {
  agentCreatedTaskIds,
  stopAllAvailability,
  stopAllSummary,
  stopAllNotification,
  shouldAddStopAllStreamEvent,
} from "./stopAllLogic.ts";

test("W5-2f 全部停止纯逻辑测试套件", async (suite) => {
  // 1. agentCreatedTaskIds 收集去重与保序
  await suite.test("1. agentCreatedTaskIds: 收集 progress_card 中的 created_task_ids，去重且保序", () => {
    const events: AgentStreamEvent[] = [
      {
        seq: 1,
        kind: "message",
        payload: { text: "hello" },
      },
      {
        seq: 2,
        kind: "progress_card",
        payload: {
          created_task_ids: ["task-1", "task-2", "task-1"],
        },
      },
      {
        seq: 3,
        kind: "tool_round",
        payload: {},
      },
      {
        seq: 4,
        kind: "progress_card",
        payload: {
          created_task_ids: ["task-3", "task-2", "task-4"],
        },
      },
    ];

    const taskIds = agentCreatedTaskIds(events);
    assert.deepStrictEqual(taskIds, ["task-1", "task-2", "task-3", "task-4"]);
  });

  // 2. stopAllAvailability 可用性判定
  await suite.test("2. stopAllAvailability: 只有进行中运行时可用", () => {
    const res = stopAllAvailability({
      liveRunIds: ["run-1"],
      agentTaskIds: [],
      tasks: [],
    });
    assert.strictEqual(res.enabled, true);
    assert.strictEqual(res.reason_zh, "");
  });

  await suite.test("2. stopAllAvailability: 只有活动任务 (queued/running/paused/interrupted) 时可用", () => {
    const mockTasks: ApplicationTask[] = [
      {
        task_id: "task-1",
        name: "任务 1",
        provider: "cdse",
        dataset: "s1_grd",
        status: "running",
        items_count: 1,
        total_bytes: 1000,
        done_bytes: 500,
        output_dir: "/out",
        created_at: "2026-10-26T00:00:00Z",
        updated_at: "2026-10-26T00:01:00Z",
      },
    ];
    const res = stopAllAvailability({
      liveRunIds: [],
      agentTaskIds: ["task-1"],
      tasks: mockTasks,
    });
    assert.strictEqual(res.enabled, true);
    assert.strictEqual(res.reason_zh, "");
  });

  await suite.test("2. stopAllAvailability: 任务已完成或非活动状态时禁用且给出说明", () => {
    const mockTasks: ApplicationTask[] = [
      {
        task_id: "task-completed",
        name: "已完成任务",
        provider: "cdse",
        dataset: "s1_grd",
        status: "completed",
        items_count: 1,
        total_bytes: 1000,
        done_bytes: 1000,
        output_dir: "/out",
        created_at: "2026-10-26T00:00:00Z",
        updated_at: "2026-10-26T00:01:00Z",
      },
    ];
    const res = stopAllAvailability({
      liveRunIds: [],
      agentTaskIds: ["task-completed"],
      tasks: mockTasks,
    });
    assert.strictEqual(res.enabled, false);
    assert.strictEqual(res.reason_zh, "没有进行中的 Agent 运行或 Agent 创建的进行中任务");
  });

  await suite.test("2. stopAllAvailability: agentTaskId 不在 tasks 列表时不计入活动任务", () => {
    const res = stopAllAvailability({
      liveRunIds: [],
      agentTaskIds: ["task-not-in-list"],
      tasks: [],
    });
    assert.strictEqual(res.enabled, false);
    assert.strictEqual(res.reason_zh, "没有进行中的 Agent 运行或 Agent 创建的进行中任务");
  });

  // 3. stopAllSummary 汇总文案与失败回退
  await suite.test("3. stopAllSummary: 全部成功汇总", () => {
    const result: AgentStopAllResult = {
      cancelled_runs: ["run-1"],
      failed_runs: [],
      cancelled_tasks: ["task-1", "task-2"],
      failed_tasks: [],
    };
    const summary = stopAllSummary(result, []);
    assert.strictEqual(summary.title_zh, "已全部停止");
    assert.strictEqual(summary.partial, false);
    assert.strictEqual(summary.lines.length, 1);
    assert.strictEqual(summary.lines[0], "取消了 1 个运行、2 个任务");
  });

  await suite.test("3. stopAllSummary: 部分失败列出任务名与回退 id", () => {
    const mockTasks: ApplicationTask[] = [
      {
        task_id: "task-f1",
        name: "哨兵1号下载",
        provider: "cdse",
        dataset: "s1_grd",
        status: "failed",
        items_count: 1,
        total_bytes: 1000,
        done_bytes: 0,
        output_dir: "/out",
        created_at: "2026-10-26T00:00:00Z",
        updated_at: "2026-10-26T00:01:00Z",
      },
    ];
    const result: AgentStopAllResult = {
      cancelled_runs: ["run-1"],
      failed_runs: ["run-fail-1"],
      cancelled_tasks: [],
      failed_tasks: ["task-f1", "task-f2-no-name"],
    };
    const summary = stopAllSummary(result, mockTasks);
    assert.strictEqual(summary.title_zh, "部分未能停止");
    assert.strictEqual(summary.partial, true);
    assert.strictEqual(summary.lines[0], "取消了 1 个运行、0 个任务");
    assert.ok(summary.lines[1].includes("未能停止的运行：run-fail-1"));
    assert.ok(summary.lines[2].includes("未能停止的任务：哨兵1号下载、task-f2-no-name"));
  });

  await suite.test("3. stopAllSummary: 全为 0 且无失败时输出特化无需要提示", () => {
    const result: AgentStopAllResult = {
      cancelled_runs: [],
      failed_runs: [],
      cancelled_tasks: [],
      failed_tasks: [],
    };
    const summary = stopAllSummary(result, []);
    assert.strictEqual(summary.title_zh, "已全部停止");
    assert.strictEqual(summary.partial, false);
    assert.strictEqual(summary.lines[0], "没有需要停止的运行或任务");
  });

  // 4. stopAllNotification
  await suite.test("4. stopAllNotification: 生成通知条目包含 detail 行", () => {
    const result: AgentStopAllResult = {
      cancelled_runs: ["run-1"],
      failed_runs: [],
      cancelled_tasks: ["task-1"],
      failed_tasks: [],
    };
    const notif = stopAllNotification(result, [], 1700000000000, "manual");
    assert.strictEqual(notif.kind, "stop_all");
    assert.strictEqual(notif.title, "已全部停止");
    assert.strictEqual(notif.occurred_at, 1700000000000);
    assert.strictEqual(notif.id, "notif-stop-all-1700000000000-manual");
    assert.deepStrictEqual(notif.detail, ["取消了 1 个运行、1 个任务"]);
  });

  // 5. shouldAddStopAllStreamEvent 去重规则
  await suite.test("5. shouldAddStopAllStreamEvent: 60 秒内相同 payload 不入铃，不同或超时入铃", () => {
    const resA: AgentStopAllResult = {
      cancelled_runs: ["run-1"],
      failed_runs: [],
      cancelled_tasks: ["task-1"],
      failed_tasks: [],
    };
    const resB: AgentStopAllResult = {
      cancelled_runs: ["run-2"],
      failed_runs: [],
      cancelled_tasks: ["task-1"],
      failed_tasks: [],
    };

    // 1) lastLocal 为 null -> 入铃
    assert.strictEqual(shouldAddStopAllStreamEvent(resA, null, 100_000), true);

    // 2) 60 秒内且相同结果 -> 不入铃
    assert.strictEqual(
      shouldAddStopAllStreamEvent(resA, { result: resA, at: 100_000 }, 130_000),
      false
    );

    // 3) 60 秒内但结果不同 -> 入铃
    assert.strictEqual(
      shouldAddStopAllStreamEvent(resA, { result: resB, at: 100_000 }, 130_000),
      true
    );

    // 4) 超过 60 秒（如 61 秒）-> 入铃
    assert.strictEqual(
      shouldAddStopAllStreamEvent(resA, { result: resA, at: 100_000 }, 161_000),
      true
    );
  });
});
