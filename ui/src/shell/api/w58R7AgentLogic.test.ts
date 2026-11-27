import test from "node:test";
import assert from "node:assert/strict";
import { computeTaskOverviewSignature, taskSnapshotChanged } from "./taskOverviewSource.ts";
import {
  buildConfirmCardTaskModels,
  buildTaskCardModels,
} from "./agentConversationLogic.ts";
import { rowProgressPercent } from "./resultListLogic.ts";
import { executeAutoCancel } from "./autoCancelFlow.ts";
import { searchCardContext } from "./searchCardSource.ts";
import type { StreamItem } from "./agentStreamLogic.ts";
import type { ApplicationTask } from "./types.ts";

function makeTask(overrides: Partial<ApplicationTask> = {}): ApplicationTask {
  const base: ApplicationTask = {
    task_id: "task-unified-r7-0001",
    name: "Sentinel-1A SLC 下载",
    provider: "asf",
    dataset: "s1_slc",
    status: "running",
    items_count: 1,
    total_bytes: 1_000_000,
    done_bytes: 250_000,
    output_dir: "D:/ScenePort/downloads",
    created_at: "2026-10-27T00:00:00Z",
    updated_at: "2026-10-27T00:00:05Z",
    revision: 3,
  };
  return { ...base, ...overrides };
}

const RAW_PROVIDER_IDS = ["asf", "asf.sentinel1", "G3210000001-ASF", "copernicus_dataspace", "Sentinel"];

test("W5-8 R7 Agent 卡片与逻辑修复", async (suite) => {
  await suite.test("1. 任务总览签名区分 pause_reason 内容与 failure", () => {
    const pausedA = makeTask({
      status: "paused",
      pause_reason: { kind: "disk_low", free_bytes: 1_000, threshold_bytes: 5_000, drive: "D:" },
    });
    const pausedB = makeTask({
      status: "paused",
      pause_reason: { kind: "disk_low", free_bytes: 9_000, threshold_bytes: 5_000, drive: "E:" },
    });
    assert.notStrictEqual(
      computeTaskOverviewSignature([pausedA]),
      computeTaskOverviewSignature([pausedB]),
      "pause_reason 内容变化必须改变签名（不得序列化成 [object Object]）"
    );
    assert.ok(
      !computeTaskOverviewSignature([pausedA]).includes("[object Object]"),
      "签名不得含 [object Object]"
    );

    const noFailure = makeTask({ status: "failed", failure: null });
    const withFailure = makeTask({
      status: "failed",
      failure: { codes: ["HTTP_504"], message_zh: "下载端点响应超时", failed_items: 1, total_items: 1 },
    });
    assert.ok(
      taskSnapshotChanged([noFailure], [withFailure]),
      "仅 failure 变化也必须判为快照变化"
    );
  });

  await suite.test("2. 已匹配真实任务的确认卡任务卡不以任务 ID 充当产品行", () => {
    const taskId = "task-unified-65046dba-c55e-4a01-a49f-728eac13d277";
    const models = buildConfirmCardTaskModels(
      [taskId],
      [makeTask({ task_id: taskId, name: "Sentinel-1D 茂县叠溪下载" })],
      { source: "asf" }
    );
    assert.strictEqual(models.length, 1);
    assert.strictEqual(
      models[0].productLine,
      null,
      `未知景名时 productLine 必须为 null，实际：${models[0].productLine}`
    );

    const withScene = buildConfirmCardTaskModels(
      [taskId],
      [makeTask({ task_id: taskId, scene_ids: ["S1A_IW_SLC__1SDV_20261020T000000"] })]
    );
    assert.strictEqual(withScene[0].productLine, "S1A_IW_SLC__1SDV_20261020T000000");
  });

  await suite.test("3. 运行仍在进行但取消运行失败时，文案如实说明未能取消本次运行", async () => {
    const noTasks = await executeAutoCancel({
      runLive: true,
      runId: "run-r7-1",
      createdTaskIds: [],
      getFreshTasks: async () => [],
      runCancel: async () => {
        throw new Error("bridge down");
      },
      cancelTask: async () => {},
    });
    assert.strictEqual(noTasks.runCancelled, false);
    assert.ok(
      noTasks.outcomeText.includes("未能取消本次运行"),
      `实际：${noTasks.outcomeText}`
    );
    assert.ok(!noTasks.outcomeText.includes("无需取消"), `实际：${noTasks.outcomeText}`);

    const endedTasks = await executeAutoCancel({
      runLive: true,
      runId: "run-r7-2",
      createdTaskIds: ["task-gone-1"],
      getFreshTasks: async () => [],
      runCancel: async () => {
        throw new Error("bridge down");
      },
      cancelTask: async () => {},
    });
    assert.ok(endedTasks.outcomeText.includes("未能取消本次运行"), `实际：${endedTasks.outcomeText}`);
    assert.ok(!endedTasks.outcomeText.includes("无需取消"), `实际：${endedTasks.outcomeText}`);

    const refreshFailed = await executeAutoCancel({
      runLive: true,
      runId: "run-r7-3",
      createdTaskIds: ["task-x"],
      getFreshTasks: async () => {
        throw new Error("overview down");
      },
      runCancel: async () => {
        throw new Error("bridge down");
      },
      cancelTask: async () => {},
    });
    assert.ok(refreshFailed.outcomeText.includes("未能取消本次运行"), `实际：${refreshFailed.outcomeText}`);
    assert.ok(refreshFailed.outcomeText.includes("未能确认任务状态"), `实际：${refreshFailed.outcomeText}`);

    const notLive = await executeAutoCancel({
      runLive: false,
      runId: "run-r7-4",
      createdTaskIds: [],
      getFreshTasks: async () => [],
      runCancel: async () => {
        throw new Error("must not be called");
      },
      cancelTask: async () => {},
    });
    assert.strictEqual(notLive.outcomeText, "本次运行与任务都已结束，无需取消");
  });

  await suite.test("4. 字节未上报（bytes_reported:false）时不计算进度", () => {
    const unreported = makeTask({
      task_id: "task-unreported",
      status: "done",
      total_bytes: 4_000_000,
      done_bytes: 0,
      bytes_reported: false,
    });
    const models = buildTaskCardModels([{ id: "task-unreported" }], [unreported]);
    assert.strictEqual(models.length, 1);
    assert.strictEqual(models[0].percent, null, `实际：${models[0].percent}`);
    assert.strictEqual(rowProgressPercent(unreported), null);

    const reported = makeTask({ task_id: "task-reported" });
    assert.strictEqual(buildTaskCardModels([{ id: "task-reported" }], [reported])[0].percent, 25);
    assert.strictEqual(rowProgressPercent(reported), 25);
  });

  await suite.test("5. 来源名只经映射：确认卡回退任务卡与检索卡规则一致，绝不显示原始来源标识", () => {
    const sourceNames = { "science.sentinel-1.standard.v1": "Sentinel-1" };
    for (const raw of RAW_PROVIDER_IDS) {
      const models = buildConfirmCardTaskModels(["task-fallback-1"], [], {
        source: raw,
        sourceNames,
      });
      assert.strictEqual(
        models[0].task.name,
        "下载任务",
        `未映射的来源标识「${raw}」不得出现在任务卡标题，实际：${models[0].task.name}`
      );

      const items = [
        {
          kind: "confirm_card",
          seq: 1,
          run_id: "run-src",
          plan_id: "plan-src",
          plan_hash: "h",
          snapshot: { source: raw, range: {} },
        },
      ] as unknown as StreamItem[];
      assert.strictEqual(searchCardContext("run-src", items, sourceNames).source, null);
    }

    const mapped = buildConfirmCardTaskModels(["task-fallback-2"], [], {
      source: "science.sentinel-1.standard.v1",
      sourceNames,
    });
    assert.strictEqual(mapped[0].task.name, "Sentinel-1 下载任务");
  });
});
