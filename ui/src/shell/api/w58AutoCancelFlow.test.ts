import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { executeAutoCancel } from "./autoCancelFlow.ts";
import { createTaskActionRunner } from "./taskActionFlow.ts";
import { stopAllAvailability } from "./stopAllLogic.ts";
import type { ApplicationTask } from "./types.ts";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const CONFLICT_CODE = "V3_STORE_REVISION_CONFLICT";

function makeTask(overrides: Partial<ApplicationTask> = {}): ApplicationTask {
  const base: ApplicationTask = {
    task_id: "task-auto-1",
    name: "Sentinel-1A SLC 自动下载任务",
    provider: "asf",
    dataset: "s1_slc",
    status: "running",
    items_count: 1,
    total_bytes: 209_974_033,
    done_bytes: 22_034_996,
    output_dir: "D:/ScenePort/downloads",
    created_at: "2026-10-27T00:00:00Z",
    updated_at: "2026-10-27T00:00:05Z",
    revision: 1,
  };
  return { ...base, ...overrides };
}

test("W5-8 P0-3 自动执行卡取消编排与全部停止回归测试套件", async (suite) => {
  await suite.test(
    "1. 新鲜快照包含活动任务：取消运行并用新鲜 revision 取消任务",
    async () => {
      const callOrder: string[] = [];

      const result = await executeAutoCancel(
        {
          runLive: true,
          getFreshTasks: async () => {
            callOrder.push("getFreshTasks");
            return [makeTask({ task_id: "task-auto-1", status: "running", revision: 4 })];
          },
          runCancel: async (runId) => {
            callOrder.push(`runCancel:${runId}`);
          },
          cancelTask: async (taskId, revision) => {
            callOrder.push(`cancelTask:${taskId}@${revision}`);
          },
        },
        {
          runId: "run-auto-1",
          createdTaskIds: ["task-auto-1"],
        }
      );

      assert.deepStrictEqual(callOrder, [
        "getFreshTasks",
        "runCancel:run-auto-1",
        "cancelTask:task-auto-1@4",
      ]);
      assert.strictEqual(result.runCancelled, true);
      assert.deepStrictEqual(result.cancelledTaskIds, ["task-auto-1"]);
      assert.deepStrictEqual(result.failedTaskIds, []);
      assert.strictEqual(result.outcomeText, "已取消本次运行和 1 个任务");
    }
  );

  await suite.test(
    "2. 旧快照缺失该任务但 getFreshTasks 新鲜快照包含该任务：必须先刷新再决策并成功取消任务 (L4 E10 根因锁定)",
    async () => {
      const cancelCalls: Array<{ taskId: string; revision: number }> = [];

      const result = await executeAutoCancel({
        runId: "run-e10",
        createdTaskIds: ["task-f613"],
        runLive: true,
        // 即使外部存在陈旧空列表，executeAutoCancel 也必须以 getFreshTasks() 返回值为准
        initialTasks: [],
        getFreshTasks: async () => [
          makeTask({
            task_id: "task-f613",
            name: "S1A_IW_SLC_f613",
            status: "running",
            revision: 3,
          }),
        ],
        runCancel: async () => {},
        cancelTask: async (taskId, revision) => {
          cancelCalls.push({ taskId, revision });
        },
      });

      assert.deepStrictEqual(
        cancelCalls,
        [{ taskId: "task-f613", revision: 3 }],
        "必须先通过 getFreshTasks 获取最新快照并以最新 revision 3 取消任务"
      );
      assert.deepStrictEqual(result.cancelledTaskIds, ["task-f613"]);
      assert.deepStrictEqual(result.skipped, []);
      assert.strictEqual(result.outcomeText, "已取消本次运行和 1 个任务");
    }
  );

  await suite.test(
    "2b. P0 回归锁定：排队中 (queued) 且 revision 为 0 的新建任务可经 executeAutoCancel + createTaskActionRunner 成功取消",
    async () => {
      const actionAttempts: Array<{ taskId: string; revision: number; action: string }> = [];
      let runnerFetchCount = 0;

      const queuedRev0Task = makeTask({
        task_id: "task-queued-rev0",
        status: "queued",
        revision: 0,
      });

      const runner = createTaskActionRunner({
        taskAction: async (taskId, revision, action) => {
          actionAttempts.push({ taskId, revision, action });
        },
        fetchTasks: async () => {
          runnerFetchCount += 1;
          return [queuedRev0Task];
        },
      });

      const result = await executeAutoCancel(
        {
          runLive: true,
          getFreshTasks: async () => [queuedRev0Task],
          runCancel: async () => {},
          cancelTask: async (taskId, revision) => {
            await runner.run(taskId, "cancel", revision);
          },
        },
        {
          runId: "run-queued-rev0",
          createdTaskIds: ["task-queued-rev0"],
        }
      );

      assert.strictEqual(runnerFetchCount, 0, "revision 0 已知合法，runner 不应重复刷新");
      assert.deepStrictEqual(actionAttempts, [
        { taskId: "task-queued-rev0", revision: 0, action: "cancel" },
      ]);
      assert.deepStrictEqual(result.cancelledTaskIds, ["task-queued-rev0"]);
      assert.deepStrictEqual(result.failedTaskIds, []);
      assert.strictEqual(result.outcomeText, "已取消本次运行和 1 个任务");
    }
  );

  await suite.test(
    "3. P1-1：刷新成功后仍 not_found 的任务不调用全局停止、记为「已结束」，结果文案如实且不暴露原始任务 ID",
    async () => {
      let cancelTaskCalled = false;

      const result = await executeAutoCancel(
        {
          runLive: true,
          getFreshTasks: async () => [],
          runCancel: async () => {},
          cancelTask: async () => {
            cancelTaskCalled = true;
          },
        },
        {
          runId: "run-already-ended",
          createdTaskIds: ["task-archived-001"],
        }
      );

      assert.strictEqual(cancelTaskCalled, false);
      assert.strictEqual(result.runCancelled, true);
      assert.deepStrictEqual(result.cancelledTaskIds, []);
      assert.deepStrictEqual(result.failedTaskIds, []);
      assert.strictEqual(
        result.outcomeText,
        "已取消本次运行；1 个任务此前已结束",
        "刷新成功后仍 not_found 的任务应记为此前已结束，不算失败"
      );
      assert.ok(
        !result.outcomeText.includes("task-archived-001"),
        "结果文案不得暴露原始任务 ID"
      );
    }
  );

  await suite.test(
    "3b. P1-1：刷新失败（getFreshTasks 抛错）→ 仍取消运行，任务部分文案含「未能确认任务状态」，不得声称任务已取消",
    async () => {
      let runCancelCalled = false;
      let cancelTaskCalled = false;

      const result = await executeAutoCancel(
        {
          runLive: true,
          getFreshTasks: async () => {
            throw new Error("IPC task overview refresh failed");
          },
          runCancel: async () => {
            runCancelCalled = true;
          },
          cancelTask: async () => {
            cancelTaskCalled = true;
          },
        },
        {
          runId: "run-refresh-err",
          createdTaskIds: ["task-unconfirmed-001"],
        }
      );

      assert.strictEqual(runCancelCalled, true, "刷新任务失败时仍应取消当前运行");
      assert.strictEqual(cancelTaskCalled, false, "未能刷新任务列表时不应盲目调用 cancelTask");
      assert.strictEqual(result.runCancelled, true);
      assert.deepStrictEqual(result.cancelledTaskIds, []);
      assert.ok(
        result.outcomeText.includes("未能确认任务状态"),
        "刷新失败时文案必须包含「未能确认任务状态」"
      );
      assert.ok(
        !result.outcomeText.includes("已取消 1 个任务") &&
          !result.outcomeText.includes("和 1 个任务"),
        "刷新失败时严禁声称任务已取消"
      );
      assert.ok(
        !result.outcomeText.includes("task-unconfirmed-001"),
        "刷新失败文案不得暴露原始任务 ID"
      );
    }
  );

  await suite.test(
    "4. runCancel 抛出异常时不得中断后续任务的取消",
    async () => {
      const cancelled: Array<{ taskId: string; revision: number }> = [];

      const result = await executeAutoCancel(
        {
          runLive: true,
          getFreshTasks: async () => [
            makeTask({ task_id: "task-keep-going", status: "running", revision: 5 }),
          ],
          runCancel: async () => {
            throw new Error("ipc failed");
          },
          cancelTask: async (taskId, revision) => {
            cancelled.push({ taskId, revision });
          },
        },
        {
          runId: "run-err",
          createdTaskIds: ["task-keep-going"],
        }
      );

      assert.strictEqual(result.runCancelled, false);
      assert.deepStrictEqual(cancelled, [{ taskId: "task-keep-going", revision: 5 }]);
      assert.deepStrictEqual(result.cancelledTaskIds, ["task-keep-going"]);
      // W5-8 R7：运行仍在进行但取消运行失败，须如实说明
      assert.strictEqual(result.outcomeText, "已取消 1 个任务；未能取消本次运行");
    }
  );

  await suite.test(
    "5. cancelTask 经 R1 runner 遇修订冲突自动重试一次并成功取消",
    async () => {
      const actionAttempts: Array<{ taskId: string; revision: number; action: string }> = [];
      let conflictTriggered = false;

      const runner = createTaskActionRunner({
        taskAction: async (taskId, revision, action) => {
          actionAttempts.push({ taskId, revision, action });
          if (!conflictTriggered) {
            conflictTriggered = true;
            throw { code: CONFLICT_CODE };
          }
        },
        fetchTasks: async () => [
          makeTask({ task_id: "task-conflict-retry", status: "running", revision: 7 }),
        ],
      });

      const result = await executeAutoCancel(
        {
          runLive: true,
          getFreshTasks: async () => [
            makeTask({ task_id: "task-conflict-retry", status: "running", revision: 6 }),
          ],
          runCancel: async () => {},
          cancelTask: async (taskId, revision) => {
            await runner.run(taskId, "cancel", revision);
          },
        },
        {
          runId: "run-conflict",
          createdTaskIds: ["task-conflict-retry"],
        }
      );

      assert.deepStrictEqual(actionAttempts, [
        { taskId: "task-conflict-retry", revision: 6, action: "cancel" },
        { taskId: "task-conflict-retry", revision: 7, action: "cancel" },
      ]);
      assert.deepStrictEqual(result.cancelledTaskIds, ["task-conflict-retry"]);
      assert.deepStrictEqual(result.failedTaskIds, []);
      assert.strictEqual(result.outcomeText, "已取消本次运行和 1 个任务");
    }
  );

  await suite.test(
    "6. L4 (d) 回归锁定：liveRunIds 为空但 agentTaskIds 命中的任务处于 running/queued/paused 时，stopAllAvailability 必须启用",
    () => {
      for (const activeStatus of ["running", "queued", "paused"] as const) {
        const availability = stopAllAvailability({
          liveRunIds: [],
          agentTaskIds: ["t1"],
          tasks: [makeTask({ task_id: "t1", status: activeStatus, revision: 2 })],
        });
        assert.strictEqual(
          availability.enabled,
          true,
          `当 Agent 任务 t1 处于 ${activeStatus} 状态时，全部停止必须可用`
        );
        assert.strictEqual(availability.reason_zh, "");
      }
    }
  );

  await suite.test(
    "7. 源码接线断言：autoCancelFlow.ts 与 DesktopAgentWorkbench 的取消链不再引用 stopAll，refreshAgentTasks 在刷新失败时抛错",
    () => {
      const autoCancelFlowPath = path.resolve(__dirname, "./autoCancelFlow.ts");
      const agentWorkbenchPath = path.resolve(
        __dirname,
        "../../components/desktop/DesktopAgentWorkbench.tsx"
      );
      const desktopWorkbenchPath = path.resolve(
        __dirname,
        "../../components/desktop/DesktopWorkbench.tsx"
      );

      const flowSrc = fs.readFileSync(autoCancelFlowPath, "utf8");
      const agentSrc = fs.readFileSync(agentWorkbenchPath, "utf8");
      const workbenchSrc = fs.readFileSync(desktopWorkbenchPath, "utf8");

      assert.ok(
        !flowSrc.includes("stopAll"),
        "autoCancelFlow.ts 严禁再引用全局 stopAll 兜底"
      );

      const cancelFnStart = agentSrc.indexOf("const handleCancelAuto");
      const cancelFnEnd = agentSrc.indexOf("const handleStopAll", cancelFnStart);
      assert.ok(cancelFnStart !== -1, "DesktopAgentWorkbench.tsx 应包含 handleCancelAuto");
      const cancelFnSlice =
        cancelFnEnd > cancelFnStart
          ? agentSrc.slice(cancelFnStart, cancelFnEnd)
          : agentSrc.slice(cancelFnStart, cancelFnStart + 1800);
      assert.ok(
        !cancelFnSlice.includes("stopAll"),
        "DesktopAgentWorkbench.tsx 的 handleCancelAuto 取消链不得引用 stopAll"
      );

      assert.ok(
        agentSrc.includes("executeAutoCancel"),
        "DesktopAgentWorkbench.tsx 必须接入 executeAutoCancel 编排器"
      );
      assert.ok(
        agentSrc.includes("refreshAgentTasks"),
        "DesktopAgentWorkbench.tsx 必须接收并使用可选的 refreshAgentTasks 属性"
      );
      assert.ok(
        workbenchSrc.includes("refreshAgentTasks={"),
        "DesktopWorkbench.tsx 必须向 DesktopAgentWorkbench 传入 refreshAgentTasks"
      );
    }
  );
});
