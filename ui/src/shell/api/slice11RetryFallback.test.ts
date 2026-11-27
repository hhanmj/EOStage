import test from "node:test";
import assert from "node:assert/strict";
import { UnifiedClient } from "./unifiedClient.ts";
import * as prod from "./production.ts";
import * as fixture from "./fixture.ts";
import {
  getAvailableTaskActions,
  RETRY_FALLBACK_DIALOG_TEXT,
} from "./taskLogic.ts";
import type { TaskRetryFallbackOutcome } from "./types.ts";

test("W4 Slice 11 - Task Retry Fallback & Two-Step Confirmation Test Suite", async (t) => {
  // 1. UnifiedClient 桩测：两步确认流调用的命令名与参数断言（逐字相等）
  await t.test("1. UnifiedClient taskRetryFallback stub invoke assertions (two-step flow)", async () => {
    const invokedCalls: Array<{ cmd: string; args?: Record<string, unknown> }> = [];

    const mockInvoke = async (cmd: string, args?: Record<string, unknown>): Promise<unknown> => {
      invokedCalls.push({ cmd, args });
      if (cmd === "application_task_retry_fallback") {
        const taskId = args?.taskId as string;
        const confirmBatchId = args?.confirmRefreshedBatchId as string | null;

        if (confirmBatchId === null) {
          // 第一步：返回需要用户确认的差异结构
          const outcome: TaskRetryFallbackOutcome = {
            task: {
              task_id: taskId,
              revision: 1,
              status: "failed",
              created: false,
            },
            refreshed_batch_id: "batch-refreshed-12345",
            rebuilt_item_ids: ["item-rebuilt-001"],
            missing_product_ids: ["S1A_IW_SLC__1SDV_20240410_MISSING"],
            added_product_ids: ["S1A_IW_SLC__1SDV_20240410_NEW"],
            needs_user_confirmation: true,
          };
          return outcome;
        }

        if (confirmBatchId === "batch-refreshed-12345") {
          // 第二步：用户确认并回传同一 refreshed_batch_id
          const outcome: TaskRetryFallbackOutcome = {
            task: {
              task_id: taskId,
              revision: 2,
              status: "running",
              created: false,
            },
            refreshed_batch_id: confirmBatchId,
            rebuilt_item_ids: ["item-rebuilt-001"],
            missing_product_ids: [],
            added_product_ids: [],
            needs_user_confirmation: false,
          };
          return outcome;
        }

        throw new Error("RETRY_FALLBACK_CONFIRMATION_MISMATCH: 批次标识不匹配");
      }
      throw new Error(`Unexpected command: ${cmd}`);
    };

    const client = new UnifiedClient({ invoke: mockInvoke });

    // 第一次调用：confirmRefreshedBatchId 为 null
    const outcome1 = await client.taskRetryFallback("task-failed-001", null);
    assert.equal(invokedCalls.length, 1);
    assert.equal(invokedCalls[0].cmd, "application_task_retry_fallback");
    assert.deepEqual(invokedCalls[0].args, {
      taskId: "task-failed-001",
      confirmRefreshedBatchId: null,
    });
    assert.equal(outcome1.needs_user_confirmation, true);
    assert.equal(outcome1.refreshed_batch_id, "batch-refreshed-12345");
    assert.deepEqual(outcome1.missing_product_ids, ["S1A_IW_SLC__1SDV_20240410_MISSING"]);
    assert.deepEqual(outcome1.added_product_ids, ["S1A_IW_SLC__1SDV_20240410_NEW"]);

    // 第二次调用：回传 outcome1.refreshed_batch_id（逐字相等）
    const outcome2 = await client.taskRetryFallback(
      "task-failed-001",
      outcome1.refreshed_batch_id
    );
    assert.equal(invokedCalls.length, 2);
    assert.equal(invokedCalls[1].cmd, "application_task_retry_fallback");
    assert.deepEqual(invokedCalls[1].args, {
      taskId: "task-failed-001",
      confirmRefreshedBatchId: "batch-refreshed-12345",
    });
    assert.equal(outcome2.needs_user_confirmation, false);
    assert.equal(outcome2.task.status, "running");
  });

  // 2. production 路径：非 Tauri 宿主下诚实拒绝并返回 DESKTOP_HOST_REQUIRED
  await t.test("2. production retryTaskFallback rejects with DESKTOP_HOST_REQUIRED on non-Tauri host", async () => {
    await assert.rejects(
      async () => {
        await prod.retryTaskFallback("task-failed-001", null);
      },
      (err: any) => {
        const message = err?.message || String(err);
        assert.ok(
          message.includes("DESKTOP_HOST_REQUIRED"),
          `Expected DESKTOP_HOST_REQUIRED, got: ${message}`
        );
        // 绝不得包含已废弃的 NOT_IMPLEMENTED 假实现
        assert.equal(
          message.includes("TASK_RETRY_FALLBACK_NOT_IMPLEMENTED"),
          false,
          "Must not contain obsolete NOT_IMPLEMENTED text"
        );
        return true;
      }
    );
  });

  // 3. fixture 两步演示流与错误断言
  await t.test("3. fixture retryTaskFallback two-step flow and error guards", async () => {
    // 第一次调用：null -> needs_user_confirmation=true 且 refreshed_batch_id 非空
    const firstOutcome = await fixture.retryTaskFallback("mock-task-c1-failed-6", null);
    assert.equal(firstOutcome.needs_user_confirmation, true);
    assert.ok(
      typeof firstOutcome.refreshed_batch_id === "string" && firstOutcome.refreshed_batch_id.length > 0,
      "refreshed_batch_id must be non-empty string"
    );
    assert.ok(
      firstOutcome.missing_product_ids.length > 0,
      "fixture should provide at least one missing product for demonstration"
    );
    assert.ok(
      firstOutcome.added_product_ids.length > 0,
      "fixture should provide at least one added product for demonstration"
    );

    // 负向测试 A：在确认前若回传错误/过期的 refreshed_batch_id -> 拒绝
    await assert.rejects(
      async () => {
        await fixture.retryTaskFallback("mock-task-c1-failed-6", "invalid-batch-id-xyz");
      },
      (err: any) => {
        const code = err?.code || "";
        const msg = err?.message_zh || err?.message || "";
        return (
          code === "RETRY_FALLBACK_CONFIRMATION_MISMATCH" ||
          msg.includes("不匹配")
        );
      }
    );

    // 第二次调用：回传同一 refreshed_batch_id -> needs_user_confirmation=false，任务恢复
    const secondOutcome = await fixture.retryTaskFallback(
      "mock-task-c1-failed-6",
      firstOutcome.refreshed_batch_id
    );
    assert.equal(secondOutcome.needs_user_confirmation, false);
    assert.equal(secondOutcome.task.status, "running");

    // 负向测试 B：非 failed 状态的任务（如已恢复为 running 的任务，或 completed 任务）-> 拒绝并返回 RETRY_FALLBACK_TASK_NOT_FAILED
    await assert.rejects(
      async () => {
        await fixture.retryTaskFallback("mock-task-c1-failed-6", null);
      },
      (err: any) => {
        const code = err?.code || "";
        const msg = err?.message_zh || err?.message || "";
        return (
          code === "RETRY_FALLBACK_TASK_NOT_FAILED" ||
          msg.includes("只有失败的任务可以重检兜底")
        );
      }
    );

    // 负向测试 B：非 failed 状态的任务（如 completed 任务）-> 拒绝并返回 RETRY_FALLBACK_TASK_NOT_FAILED
    await assert.rejects(
      async () => {
        await fixture.retryTaskFallback("mock-task-c1-dem-completed-8", null);
      },
      (err: any) => {
        const code = err?.code || "";
        const msg = err?.message_zh || err?.message || "";
        return (
          code === "RETRY_FALLBACK_TASK_NOT_FAILED" ||
          msg.includes("只有失败的任务可以重检兜底")
        );
      }
    );

    // 负向测试 C：非 ASF 目录 SAR 产品任务（如 dem_convert 类型的任务）-> 拒绝并返回 RETRY_FALLBACK_UNSUPPORTED
    await assert.rejects(
      async () => {
        await fixture.retryTaskFallback("mock-task-c1-dem-failed-7", null);
      },
      (err: any) => {
        const code = err?.code || "";
        const msg = err?.message_zh || err?.message || "";
        return (
          code === "RETRY_FALLBACK_UNSUPPORTED" ||
          msg.includes("不支持")
        );
      }
    );
  });

  // 4. getAvailableTaskActions 负断言：cancelled/completed/interrupted 永不包含 retry_fallback
  await t.test("4. getAvailableTaskActions negative assertions for retry_fallback", () => {
    const nonFailedStatuses = ["queued", "running", "paused", "completed", "cancelled", "interrupted"] as const;

    for (const status of nonFailedStatuses) {
      const actionsWithoutDir = getAvailableTaskActions(status, false);
      const actionsWithDir = getAvailableTaskActions(status, true);

      assert.equal(
        actionsWithoutDir.includes("retry_fallback"),
        false,
        `${status} without output dir must NOT have retry_fallback`
      );
      assert.equal(
        actionsWithDir.includes("retry_fallback"),
        false,
        `${status} with output dir must NOT have retry_fallback`
      );
    }

    // 唯有 failed 且具备可重检检索条件时包含 retry_fallback
    assert.equal(getAvailableTaskActions("failed", false).includes("retry_fallback"), true);
    assert.equal(getAvailableTaskActions("failed", true).includes("retry_fallback"), true);
    assert.equal(getAvailableTaskActions("failed", true, false).includes("retry_fallback"), false);
  });

  // 5. 文案断言：对话框文案遵守安全事实，严禁出现「即将提供」与承诺续传
  await t.test("5. Dialog text safety assertions", () => {
    const dialog = RETRY_FALLBACK_DIALOG_TEXT;
    assert.ok(dialog.title.includes("重新检索后重试"), "Title matches");
    assert.ok(dialog.content.includes("原检索条件"), "Must mention original search params");
    assert.ok(dialog.content.includes("按产品编号"), "Must mention matching by product ID");
    assert.ok(dialog.content.includes("原任务"), "Must mention reconstructing in original task");
    assert.ok(dialog.content.includes("差异"), "Must mention listing differences");
    assert.ok(dialog.content.includes("确认"), "Must mention user confirmation");

    // 不得出现「即将提供」
    assert.equal(dialog.title.includes("即将提供"), false, "Title must not include '即将提供'");
    assert.equal(dialog.content.includes("即将提供"), false, "Content must not include '即将提供'");

    // 严禁承诺续传或复用已下载部分
    assert.equal(dialog.content.includes("续传"), false, "Must not promise resume/continuation");
    assert.equal(dialog.content.includes("断点续传"), false, "Must not promise breakpoint continuation");
    assert.equal(dialog.content.includes("复用已下载"), false, "Must not promise reusing downloaded data");
  });

  // 6. 审计 P0：验证 DesktopDownloadDrawer.tsx 组件源码绝不包含手工构造的 fixture-batch 假数据通路
  await t.test("6. DesktopDownloadDrawer source must NOT contain fixture-batch literal", async () => {
    const fs = await import("node:fs/promises");
    const drawerUrl = new URL("../../components/desktop/DesktopDownloadDrawer.tsx", import.meta.url);
    const content = await fs.readFile(drawerUrl, "utf-8");
    assert.equal(
      content.includes("fixture-batch"),
      false,
      "DesktopDownloadDrawer source must NOT contain fixture-batch literal (Audit P0)"
    );
  });

  // 7. UnifiedClient 新增下载中心命令（分景控制 / 单条删除 / 置顶 / 资格检查 / 磁盘重复检测）桩测
  await t.test("7. UnifiedClient stub invoke assertions for item actions, single delete, pins, eligibility, and duplicate check", async () => {
    const calls: Array<{ cmd: string; args?: Record<string, unknown> }> = [];
    const client = new UnifiedClient({
      invoke: async (cmd, args) => {
        calls.push({ cmd, args });
        switch (cmd) {
          case "application_task_item_action":
            return { task_id: args?.taskId, revision: 9, status: "running", created: false };
          case "application_task_delete_history":
            return null;
          case "application_task_set_pinned":
            return [args?.taskId];
          case "application_list_pinned_tasks":
            return ["task-pinned-1"];
          case "application_task_retry_fallback_eligibility":
            return { "t-cat": true, "t-non": false };
          case "check_existing_downloaded_scenes":
            return [
              {
                scene_id: "S1A_001",
                path: "/downloads/S1A_001.zip",
                size_bytes: 1024,
                task_title: "Prior Task",
              },
            ];
          default:
            throw new Error(`Unexpected command ${cmd}`);
        }
      },
    });

    await client.taskItemAction("task-1", 8, "item-a", "pause");
    await client.taskDeleteHistory("task-old");
    const pinnedAfterSet = await client.taskSetPinned("task-1", true);
    const listedPins = await client.listPinnedTasks();
    const eligibility = await client.taskRetryFallbackEligibility(["t-cat", "t-non"]);
    const duplicates = await client.checkExistingDownloadedScenes(
      [{ scene_id: "S1A_001", file_name: "S1A_001.zip" }],
      "/downloads"
    );

    assert.deepEqual(pinnedAfterSet, ["task-1"]);
    assert.deepEqual(listedPins, ["task-pinned-1"]);
    assert.deepEqual(eligibility, { "t-cat": true, "t-non": false });
    assert.equal(duplicates.length, 1);
    assert.equal(duplicates[0].scene_id, "S1A_001");
    assert.deepEqual(
      calls.map((c) => c.cmd),
      [
        "application_task_item_action",
        "application_task_delete_history",
        "application_task_set_pinned",
        "application_list_pinned_tasks",
        "application_task_retry_fallback_eligibility",
        "check_existing_downloaded_scenes",
      ]
    );
    assert.deepEqual(calls[0].args, {
      taskId: "task-1",
      revision: 8,
      itemId: "item-a",
      action: "pause",
    });
    assert.deepEqual(calls[5].args, {
      items: [{ scene_id: "S1A_001", file_name: "S1A_001.zip" }],
      targetDir: "/downloads",
    });
  });
});
