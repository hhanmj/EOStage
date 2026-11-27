import test from "node:test";
import assert from "node:assert/strict";
import {
  isTaskInQueue,
  isTaskInHistory,
  isGacosWebformTask,
  isTaskHistoryRow,
  partitionTasks,
  getAvailableTaskActions,
  isTaskFallbackEligible,
  sortTasksByPinAndCreatedAt,
  formatTaskTimestamp,
  RETRY_FALLBACK_DIALOG_TEXT,
  CLEAR_HISTORY_DIALOG_TEXT,
  isHarnessAccessible,
} from "./taskLogic.ts";
import { effectiveGroupStatus } from "./orbitChildRows.ts";
import { isFixtureBackend, __fixture } from "./index.ts";
import type { TaskLifecycleStatus, ApplicationTask } from "./types.ts";

test("Task Center and C1 Task Logic Suite", async (t) => {
  await t.test("1. Queue vs History task partition rules", () => {
    const queueStatuses: TaskLifecycleStatus[] = ["queued", "running", "paused"];
    const historyStatuses: TaskLifecycleStatus[] = ["completed", "failed", "cancelled", "interrupted"];

    for (const status of queueStatuses) {
      assert.equal(isTaskInQueue(status), true, `${status} should be in queue`);
      assert.equal(isTaskInHistory(status), false, `${status} should not be in history`);
    }

    for (const status of historyStatuses) {
      assert.equal(isTaskInQueue(status), false, `${status} should not be in queue`);
      assert.equal(isTaskInHistory(status), true, `${status} should be in history`);
    }

    const sampleTasks: ApplicationTask[] = [
      { task_id: "t1", revision: 1, category: "scene_download", name: "Task 1", status: "running", done_bytes: 100 },
      { task_id: "t2", revision: 1, category: "dem_convert", name: "Task 2", status: "queued", done_bytes: 0 },
      { task_id: "t3", revision: 1, category: "scene_download", name: "Task 3", status: "paused", done_bytes: 50 },
      { task_id: "t4", revision: 1, category: "scene_download", name: "Task 4", status: "completed", done_bytes: 200 },
      { task_id: "t5", revision: 1, category: "dem_convert", name: "Task 5", status: "failed", done_bytes: 30 },
      { task_id: "t6", revision: 1, category: "scene_download", name: "Task 6", status: "cancelled", done_bytes: 10 },
      { task_id: "t7", revision: 1, category: "scene_download", name: "Task 7", status: "interrupted", done_bytes: 20 },
    ];

    const { queue, history } = partitionTasks(sampleTasks);
    assert.deepEqual(queue.map((t) => t.task_id), ["t1", "t2", "t3"]);
    assert.deepEqual(history.map((t) => t.task_id), ["t4", "t5", "t6", "t7"]);
  });

  await t.test("2. Task action availability derived strictly from TaskLifecycleStatus and outputDir", () => {
    // queued -> pause, cancel
    assert.deepEqual(getAvailableTaskActions("queued", false), ["pause", "cancel"]);
    assert.deepEqual(getAvailableTaskActions("queued", true), ["pause", "cancel", "open_folder"]);

    // running -> pause, cancel
    assert.deepEqual(getAvailableTaskActions("running", false), ["pause", "cancel"]);
    assert.deepEqual(getAvailableTaskActions("running", true), ["pause", "cancel", "open_folder"]);

    // paused -> resume, cancel
    assert.deepEqual(getAvailableTaskActions("paused", false), ["resume", "cancel"]);
    assert.deepEqual(getAvailableTaskActions("paused", true), ["resume", "cancel", "open_folder"]);

    // failed -> retry, retry_fallback, details, (open_folder if outputDir exists)
    assert.deepEqual(
      getAvailableTaskActions("failed", false),
      ["retry", "retry_fallback", "details"]
    );
    assert.deepEqual(
      getAvailableTaskActions("failed", true),
      ["retry", "retry_fallback", "details", "open_folder"]
    );

    // completed -> details, (open_folder if outputDir exists)
    assert.deepEqual(getAvailableTaskActions("completed", false), ["details"]);
    assert.deepEqual(
      getAvailableTaskActions("completed", true),
      ["details", "open_folder"]
    );

    // cancelled / interrupted -> retry, details, (open_folder if outputDir exists; W5-10 R4)
    assert.deepEqual(getAvailableTaskActions("cancelled", false), ["retry", "details"]);
    assert.deepEqual(getAvailableTaskActions("cancelled", true), ["retry", "details", "open_folder"]);
    assert.deepEqual(getAvailableTaskActions("interrupted", false), ["retry", "details"]);
    assert.deepEqual(getAvailableTaskActions("interrupted", true), ["retry", "details", "open_folder"]);
  });

  await t.test("3. Confirmation dialog texts and safety assertions", () => {
    // 重新检索后重试文案断言
    const fallback = RETRY_FALLBACK_DIALOG_TEXT;
    assert.ok(fallback.title.includes("重新检索后重试"), "Title matches");
    assert.ok(fallback.content.includes("原检索条件"), "Must mention original search params");
    assert.ok(fallback.content.includes("按产品编号"), "Must mention matching by product ID");
    assert.ok(fallback.content.includes("原任务"), "Must mention reconstructing in original task");
    assert.ok(fallback.content.includes("差异"), "Must mention listing differences");
    assert.ok(fallback.content.includes("确认"), "Must mention user confirmation");

    // 严禁承诺续传或复用已下载内容
    assert.equal(fallback.content.includes("续传"), false, "Must not promise resume/continuation");
    assert.equal(fallback.content.includes("断点续传"), false, "Must not promise breakpoint continuation");
    assert.equal(fallback.content.includes("复用已下载"), false, "Must not promise reusing downloaded data");

    // 清除历史文案断言
    const clearHistory = CLEAR_HISTORY_DIALOG_TEXT;
    assert.ok(clearHistory.title.includes("清除历史"), "Clear history title matches");
    assert.ok(clearHistory.content.includes("只清除历史记录"), "Must clarify only clearing record");
    assert.ok(clearHistory.content.includes("不会删除已下载的文件"), "Must clarify files are not deleted");
  });

  await t.test("4. Fixture backend switch and Harness guard predicates", () => {
    // 在 node 单测环境下（import.meta.env 不存在），isFixtureBackend() 必须安全回退并返回 true
    assert.equal(isFixtureBackend(), true, "Node test environment defaults to fixture backend");
    assert.equal(__fixture, true, "__fixture marker must be truthy in fixture environment");

    // Harness 守卫：仅 DEV 开启时可访问
    assert.equal(isHarnessAccessible(true), true, "Harness is accessible when DEV=true");
    assert.equal(isHarnessAccessible(false), false, "Harness is strictly forbidden when DEV=false");
  });

  await t.test("5. Reverse-chronological and pinned sorting + timestamp formatting", () => {
    const tasks: ApplicationTask[] = [
      {
        task_id: "t-old",
        revision: 1,
        category: "scene_download",
        name: "Oldest",
        status: "queued",
        done_bytes: 0,
        created_at: "2026-11-10T08:00:00Z",
        updated_at: "2026-11-10T09:00:00Z",
      },
      {
        task_id: "t-mid",
        revision: 1,
        category: "scene_download",
        name: "Middle",
        status: "running",
        done_bytes: 10,
        created_at: "2026-11-12T08:00:00Z",
        updated_at: "2026-11-12T09:00:00Z",
      },
      {
        task_id: "t-new",
        revision: 1,
        category: "scene_download",
        name: "Newest",
        status: "queued",
        done_bytes: 0,
        created_at: "2026-11-15T08:00:00Z",
        updated_at: "2026-11-15T08:01:00Z",
      },
    ];

    assert.deepEqual(
      sortTasksByPinAndCreatedAt(tasks, []).map((item) => item.task_id),
      ["t-new", "t-mid", "t-old"],
      "Unpinned tasks must be reverse-chronological by created_at (newest on top)"
    );
    assert.deepEqual(
      sortTasksByPinAndCreatedAt(tasks, ["t-old"]).map((item) => item.task_id),
      ["t-old", "t-new", "t-mid"],
      "Pinned task must appear first before remaining reverse-chronological tasks"
    );
    assert.ok(
      formatTaskTimestamp("2026-11-15T08:00:00Z")?.match(/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}$/),
      "Valid RFC3339 timestamp must format cleanly"
    );
    assert.equal(formatTaskTimestamp("not-a-date"), null);
  });

  await t.test("6. Fallback eligibility gates retry_fallback availability", () => {
    assert.equal(
      getAvailableTaskActions("failed", true, false).includes("retry_fallback"),
      false,
      "Non-replayable failed task must not advertise retry_fallback"
    );
    assert.equal(
      isTaskFallbackEligible({
        task_id: "dem-fail",
        status: "failed",
        category: "dem_convert",
      }),
      false
    );
    assert.equal(
      isTaskFallbackEligible({
        task_id: "nisar-fail",
        status: "failed",
        category: "scene_download",
        task_type: "nisar_subset",
      }),
      false
    );
    assert.equal(
      isTaskFallbackEligible(
        { task_id: "cat-fail", status: "failed", category: "scene_download" },
        { "cat-fail": false }
      ),
      false
    );
    assert.equal(
      isTaskFallbackEligible(
        { task_id: "cat-fail", status: "failed", category: "scene_download" },
        { "cat-fail": true }
      ),
      true
    );
  });

  await t.test("7. Submitted GACOS webform requests partition into history and count as completed in filter bar", () => {
    const gacosSubmitted: ApplicationTask = {
      task_id: "gacos-request-abc123",
      revision: 2,
      category: "atmospheric_request",
      name: "GACOS 大气延迟申请",
      status: "paused",
      source_provider_id: "gacos.webform",
      done_bytes: 0,
    };
    const normalPaused: ApplicationTask = {
      task_id: "task-normal-paused",
      revision: 1,
      category: "scene_download",
      name: "Normal Paused",
      status: "paused",
      done_bytes: 100,
    };
    assert.equal(isGacosWebformTask(gacosSubmitted), true);
    assert.equal(isTaskHistoryRow(gacosSubmitted), true);
    assert.equal(isTaskHistoryRow(normalPaused), false);

    const { queue, history } = partitionTasks([gacosSubmitted, normalPaused]);
    assert.deepEqual(queue.map((x) => x.task_id), ["task-normal-paused"]);
    assert.deepEqual(history.map((x) => x.task_id), ["gacos-request-abc123"]);
    assert.equal(effectiveGroupStatus({ task: gacosSubmitted, companions: [] }), "completed");
    assert.equal(effectiveGroupStatus({ task: normalPaused, companions: [] }), "paused");
  });
});
