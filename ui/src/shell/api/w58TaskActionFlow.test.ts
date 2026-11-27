import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createTaskActionRunner } from "./taskActionFlow.ts";
import type { ApplicationTask } from "./types.ts";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

function makeTask(taskId: string, revision: number): ApplicationTask {
  return {
    task_id: taskId,
    revision,
    category: "scene_download",
    name: "Sentinel-1 测试任务",
    status: "paused",
    done_bytes: 1024,
    total_bytes: 2048,
  };
}

test("W5-8 R1 - TaskActionFlow 动作最新修订与单次冲突重试测试套件", async (suite) => {
  await suite.test(
    "1. 旧 revision 触发 V3_STORE_REVISION_CONFLICT → 自动拉取最新 revision 重试一次并成功",
    async () => {
      const actionCalls: Array<{ taskId: string; revision: number; action: string }> = [];
      let fetchCalls = 0;

      const runner = createTaskActionRunner({
        taskAction: async (taskId, revision, action) => {
          actionCalls.push({ taskId, revision, action });
          if (revision === 2) {
            const err = Object.assign(new Error("store revision conflict"), {
              code: "V3_STORE_REVISION_CONFLICT",
            });
            throw err;
          }
        },
        fetchTasks: async () => {
          fetchCalls += 1;
          return [makeTask("task-001", 3)];
        },
      });

      await runner.run("task-001", "cancel", 2);

      assert.strictEqual(fetchCalls, 1, "发生冲突后必须拉取一次最新任务列表");
      assert.deepStrictEqual(actionCalls, [
        { taskId: "task-001", revision: 2, action: "cancel" },
        { taskId: "task-001", revision: 3, action: "cancel" },
      ]);
    }
  );

  await suite.test(
    "2. 字符串形态的 V3_STORE_REVISION_CONFLICT 同样触发刷新并单次重试成功",
    async () => {
      const revisionsUsed: number[] = [];

      const runner = createTaskActionRunner({
        taskAction: async (_taskId, revision) => {
          revisionsUsed.push(revision);
          if (revisionsUsed.length === 1) {
            throw "Error: V3_STORE_REVISION_CONFLICT (expected 5, got 4)";
          }
        },
        fetchTasks: async () => [makeTask("task-002", 5)],
      });

      await runner.run("task-002", "resume", 4);
      assert.deepStrictEqual(revisionsUsed, [4, 5]);
    }
  );

  await suite.test(
    "3. 连续两次 V3_STORE_REVISION_CONFLICT → 仅重试一次后抛出中文错误，不死循环且不暴露裸实现码",
    async () => {
      let actionCount = 0;
      let fetchCount = 0;

      const rawConflictErr = {
        code: "V3_STORE_REVISION_CONFLICT",
        message: "V3_STORE_REVISION_CONFLICT raw",
      };
      const runner = createTaskActionRunner({
        taskAction: async () => {
          actionCount += 1;
          throw rawConflictErr;
        },
        fetchTasks: async () => {
          fetchCount += 1;
          return [makeTask("task-003", 10 + fetchCount)];
        },
      });

      await assert.rejects(
        async () => {
          await runner.run("task-003", "pause", 10);
        },
        (err: any) => {
          assert.strictEqual(err?.code, "V3_STORE_REVISION_CONFLICT");
          assert.strictEqual(
            err?.cause,
            rawConflictErr,
            "包装错误必须保留原始异常作为 cause"
          );
          assert.strictEqual(typeof err?.message_zh, "string");
          assert.ok(err.message_zh.length > 0);
          assert.ok(
            !err.message_zh.includes("V3_"),
            "向用户展示的中文错误文案不得暴露 V3_ 实现码"
          );
          assert.ok(
            !String(err.message || "").includes("V3_"),
            "Error.message 同样应为中文文案，不得暴露 V3_ 实现码"
          );
          return true;
        }
      );

      assert.strictEqual(actionCount, 2, "初始调用 1 次 + 冲突重试 1 次 = 严格 2 次");
      assert.strictEqual(fetchCount, 1, "只刷新 1 次");
    }
  );

  await suite.test(
    "4. 已知 revision 0（新建排队或建即暂停任务）→ 直接以 revision 0 调用 taskAction，不触发 fetchTasks 刷新",
    async () => {
      const actionCalls: Array<{ taskId: string; revision: number; action: string }> = [];
      let fetchCalls = 0;

      const runner = createTaskActionRunner({
        taskAction: async (taskId, revision, action) => {
          actionCalls.push({ taskId, revision, action });
        },
        fetchTasks: async () => {
          fetchCalls += 1;
          return [makeTask("task-rev0-direct", 0)];
        },
      });

      await runner.run("task-rev0-direct", "pause", 0);
      assert.strictEqual(fetchCalls, 0, "已知 revision 0 为合法修订号，不应触发额外刷新");
      assert.deepStrictEqual(actionCalls, [
        { taskId: "task-rev0-direct", revision: 0, action: "pause" },
      ]);
    }
  );

  await suite.test(
    "4b. knownRevision 缺失、刷新后找到任务且 revision 为 0 → 以 revision 0 调用 taskAction",
    async () => {
      const actionCalls: Array<{ taskId: string; revision: number; action: string }> = [];
      let fetchCalls = 0;

      const runner = createTaskActionRunner({
        taskAction: async (taskId, revision, action) => {
          actionCalls.push({ taskId, revision, action });
        },
        fetchTasks: async () => {
          fetchCalls += 1;
          return [makeTask("task-rev0-fresh", 0)];
        },
      });

      await runner.run("task-rev0-fresh", "cancel");
      assert.strictEqual(fetchCalls, 1, "knownRevision 缺失时应刷新一次");
      assert.deepStrictEqual(actionCalls, [
        { taskId: "task-rev0-fresh", revision: 0, action: "cancel" },
      ]);
    }
  );

  await suite.test(
    "4c. knownRevision 缺失或为负数/小数/NaN 时 → 先通过 fetchTasks 获取最新非负整数 revision 再调用 taskAction",
    async () => {
      const actionCalls: Array<{ taskId: string; revision: number; action: string }> = [];
      let fetchCalls = 0;

      const runner = createTaskActionRunner({
        taskAction: async (taskId, revision, action) => {
          actionCalls.push({ taskId, revision, action });
        },
        fetchTasks: async () => {
          fetchCalls += 1;
          return [makeTask("task-004", 7)];
        },
      });

      await runner.run("task-004", "pause");
      await runner.run("task-004", "pause", -1);
      await runner.run("task-004", "pause", 1.5);
      await runner.run("task-004", "pause", Number.NaN);

      assert.strictEqual(fetchCalls, 4, "缺失、负数、小数、NaN 均应先刷新获取真实修订号");
      assert.deepStrictEqual(actionCalls, [
        { taskId: "task-004", revision: 7, action: "pause" },
        { taskId: "task-004", revision: 7, action: "pause" },
        { taskId: "task-004", revision: 7, action: "pause" },
        { taskId: "task-004", revision: 7, action: "pause" },
      ]);
    }
  );

  await suite.test(
    "5. knownRevision 缺失且新鲜列表找不到该任务 → 抛出 TASK_NOT_FOUND 且绝不调用 taskAction（绝不编造 revision 1）",
    async () => {
      let actionCalled = false;

      const runner = createTaskActionRunner({
        taskAction: async () => {
          actionCalled = true;
        },
        fetchTasks: async () => [],
      });

      await assert.rejects(
        async () => {
          await runner.run("task-missing", "cancel");
        },
        (err: any) => {
          assert.strictEqual(err?.code, "TASK_NOT_FOUND");
          assert.strictEqual(err?.message_zh, "任务不存在或已结束");
          return true;
        }
      );

      assert.strictEqual(actionCalled, false, "任务不存在时绝不可调用 taskAction");
    }
  );

  await suite.test(
    "6. 源码接线断言：taskActionFlow 使用字面量 V3_STORE_REVISION_CONFLICT，DesktopWorkbench 行动作 try 只包 runner 调用",
    () => {
      const flowPath = path.resolve(__dirname, "./taskActionFlow.ts");
      const flowContent = fs.readFileSync(flowPath, "utf-8");
      assert.ok(
        flowContent.includes('"V3_STORE_REVISION_CONFLICT"'),
        "taskActionFlow.ts 必须直接使用字面量 \"V3_STORE_REVISION_CONFLICT\""
      );

      const wbPath = path.resolve(__dirname, "../../components/desktop/DesktopWorkbench.tsx");
      const wbContent = fs.readFileSync(wbPath, "utf-8");

      assert.doesNotMatch(
        wbContent,
        /\.revision\s*\?\?\s*1/,
        "DesktopWorkbench.tsx 严禁包含修订号硬编码编造兜底"
      );
      assert.match(
        wbContent,
        /createTaskActionRunner/,
        "DesktopWorkbench.tsx 必须引入并使用 createTaskActionRunner"
      );

      // 检查 handleRowAction 的 pause/resume/retry 分支：setTaskOutcomeNotice 的 catch 出现在 getBatchPage 之前
      const branchIdx = wbContent.indexOf(
        'action === "pause" || action === "resume" || action === "retry"'
      );
      const branchEndIdx = wbContent.indexOf(
        'else if (action === "open_folder")',
        branchIdx
      );
      assert.ok(branchIdx !== -1 && branchEndIdx > branchIdx);
      const branchSnippet = wbContent.slice(branchIdx, branchEndIdx);
      const noticeIdx = branchSnippet.indexOf("setTaskOutcomeNotice");
      const batchPageIdx = branchSnippet.indexOf("getBatchPage");
      assert.ok(
        noticeIdx !== -1 && batchPageIdx > noticeIdx,
        "handleRowAction 中 setTaskOutcomeNotice 应仅捕获 taskActionRunner.run 错误，之后的 getBatchPage 刷新在独立 try 中不报操作未能完成"
      );

      const drawerPath = path.resolve(
        __dirname,
        "../../components/desktop/DesktopDownloadDrawer.tsx"
      );
      const drawerContent = fs.readFileSync(drawerPath, "utf-8");
      assert.match(
        drawerContent,
        /createTaskActionRunner/,
        "DesktopDownloadDrawer.tsx 必须引入并使用 createTaskActionRunner"
      );
    }
  );
});
