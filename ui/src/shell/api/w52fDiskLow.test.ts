import test from "node:test";
import assert from "node:assert/strict";

import {
  parseTaskPauseReason,
  parseStoreTaskSummary,
} from "./taskOverviewParser.ts";
import { pauseReasonLine } from "./taskCardLogic.ts";
import { ERROR_CODE_MAP, explainErrorCode } from "./errorExplanations.ts";
import { deriveTaskEvents } from "./notificationBell.ts";
import type { ApplicationTask, TaskPauseReason } from "./types.ts";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  getTasks,
  __setManualWorkbenchSubsceneForTest,
  __resetFixtureState,
} from "./fixture.ts";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const readSrc = (rel: string) => fs.readFileSync(path.resolve(HERE, rel), "utf-8");

function makeMockTask(
  id: string,
  status: any,
  options?: {
    name?: string;
    pause_reason?: TaskPauseReason | null;
    updated_at?: string;
  }
): ApplicationTask {
  return {
    task_id: id,
    revision: 1,
    category: "scene_download",
    name: options?.name ?? `Task ${id}`,
    status,
    done_bytes: 100,
    total_bytes: 1000,
    pause_reason: options?.pause_reason ?? null,
    updated_at: options?.updated_at ?? "2026-10-26T08:00:00Z",
  };
}

test("W5-2f R6: 磁盘空间不足暂停契约与逻辑测试套件", async (suite) => {
  await suite.test("1. parseTaskPauseReason: 规范对象解析与容错处理", () => {
    // 正常有 drive
    const valid = parseTaskPauseReason({
      kind: "disk_low",
      free_bytes: 1288490188,
      threshold_bytes: 2147483648,
      drive: "D:",
    });
    assert.deepStrictEqual(valid, {
      kind: "disk_low",
      free_bytes: 1288490188,
      threshold_bytes: 2147483648,
      drive: "D:",
    });

    // drive 为 null 或缺省
    const nullDrive = parseTaskPauseReason({
      kind: "disk_low",
      free_bytes: 1073741824,
      threshold_bytes: 2147483648,
      drive: null,
    });
    assert.deepStrictEqual(nullDrive, {
      kind: "disk_low",
      free_bytes: 1073741824,
      threshold_bytes: 2147483648,
      drive: null,
    });

    // drive 为空字符串 -> 规范化为 null
    const emptyDrive = parseTaskPauseReason({
      kind: "disk_low",
      free_bytes: 1073741824,
      threshold_bytes: 2147483648,
      drive: "",
    });
    assert.deepStrictEqual(emptyDrive, {
      kind: "disk_low",
      free_bytes: 1073741824,
      threshold_bytes: 2147483648,
      drive: null,
    });

    // 畸形与非法输入一律返回 null，绝不抛错
    assert.strictEqual(parseTaskPauseReason(null), null);
    assert.strictEqual(parseTaskPauseReason(undefined), null);
    assert.strictEqual(parseTaskPauseReason("disk_low"), null);
    assert.strictEqual(parseTaskPauseReason([]), null);
    assert.strictEqual(parseTaskPauseReason({ kind: "other_kind" }), null);
    assert.strictEqual(parseTaskPauseReason({ kind: "disk_low", free_bytes: -1, threshold_bytes: 100 }), null);
    assert.strictEqual(parseTaskPauseReason({ kind: "disk_low", free_bytes: NaN, threshold_bytes: 100 }), null);
    assert.strictEqual(parseTaskPauseReason({ kind: "disk_low", free_bytes: 100, threshold_bytes: -1 }), null);
    assert.strictEqual(parseTaskPauseReason({ kind: "disk_low", free_bytes: 100, threshold_bytes: 200, drive: 123 }), null);
  });

  await suite.test("2. parseStoreTaskSummary: 正确解析 pause_reason", () => {
    const raw = {
      task_id: "task-001",
      batch_id: "batch-001",
      provider_id: "ASF",
      created_at: "2026-10-26T08:00:00Z",
      status: "paused",
      revision: 1,
      updated_at: "2026-10-26T08:01:00Z",
      selected_count: 1,
      checked_count: 1,
      active_count: 0,
      paused_count: 1,
      queued_count: 0,
      completed_count: 0,
      failed_count: 0,
      skipped_count: 0,
      cancelled_count: 0,
      terminal_bytes: 0,
      active_bytes: 0,
      other_bytes: 0,
      transferred_bytes: 1000,
      total_bytes: 5000,
      pause_reason: {
        kind: "disk_low",
        free_bytes: 1288490188,
        threshold_bytes: 2147483648,
        drive: "D:",
      },
    };
    const parsed = parseStoreTaskSummary(raw);
    assert.deepStrictEqual(parsed.pause_reason, {
      kind: "disk_low",
      free_bytes: 1288490188,
      threshold_bytes: 2147483648,
      drive: "D:",
    });
  });

  await suite.test("3. pauseReasonLine: 盘符格式化与 Windows 资源管理器 2^30 口径", () => {
    // null 或非 disk_low
    assert.strictEqual(pauseReasonLine(null), null);
    assert.strictEqual(pauseReasonLine(undefined), null);

    // 有盘符 D:，free_bytes = 1288490188 (1288490188 / 2^30 = 1.19999998... -> 1.2 GB)
    const lineWithDrive = pauseReasonLine({
      kind: "disk_low",
      free_bytes: 1288490188,
      threshold_bytes: 2147483648,
      drive: "D:",
    });
    assert.strictEqual(lineWithDrive, "磁盘空间不足已暂停（D: 剩余 1.2 GB）");

    // 无盘符（null 或空）-> 未知盘符
    const lineWithoutDrive = pauseReasonLine({
      kind: "disk_low",
      free_bytes: 1288490188,
      threshold_bytes: 2147483648,
      drive: null,
    });
    assert.strictEqual(lineWithoutDrive, "磁盘空间不足已暂停（未知盘符 剩余 1.2 GB）");
  });

  await suite.test("4. ERROR_CODE_MAP: DISK_LOW 错误解释映射", () => {
    assert.strictEqual(
      ERROR_CODE_MAP["DISK_LOW"],
      "磁盘剩余空间不足，无法继续下载，请先清理该盘空间"
    );
    assert.strictEqual(
      explainErrorCode("DISK_LOW"),
      "磁盘剩余空间不足，无法继续下载，请先清理该盘空间"
    );
  });

  await suite.test("5. deriveTaskEvents: 磁盘空间不足通知事件派生与防抖去重", () => {
    const diskLowReason: TaskPauseReason = {
      kind: "disk_low",
      free_bytes: 1288490188,
      threshold_bytes: 2147483648,
      drive: "D:",
    };

    // a. 任务从 running 进入 disk_low 暂停 -> 发一条通知
    const prevRunning = [makeMockTask("t1", "running")];
    const nextPaused = [
      makeMockTask("t1", "paused", {
        name: "测试任务 1",
        pause_reason: diskLowReason,
        updated_at: "2026-10-26T08:05:00Z",
      }),
    ];
    const events1 = deriveTaskEvents(prevRunning, nextPaused);
    assert.strictEqual(events1.length, 1);
    assert.strictEqual(events1[0].task_id, "t1");
    assert.strictEqual(events1[0].kind, "disk_low");
    assert.strictEqual(events1[0].id, "notif-t1-disk_low-2026-10-26T08:05:00Z");
    assert.strictEqual(events1[0].title, "测试任务 1");
    assert.strictEqual(events1[0].route, "downloads");
    assert.deepStrictEqual(events1[0].detail, ["磁盘空间不足已暂停（D: 剩余 1.2 GB）"]);

    // b. 重复快照 -> 不发通知
    const events2 = deriveTaskEvents(nextPaused, nextPaused);
    assert.strictEqual(events2.length, 0);

    // c. 普通暂停（无 reason）-> 不发通知
    const normalPaused = [makeMockTask("t2", "paused", { name: "普通暂停任务" })];
    const events3 = deriveTaskEvents(prevRunning, normalPaused);
    assert.strictEqual(events3.length, 0);

    // d. 首次观察到已处于 disk_low 暂停 -> 发一次通知
    const events4 = deriveTaskEvents([], nextPaused);
    assert.strictEqual(events4.length, 1);
    assert.strictEqual(events4[0].task_id, "t1");
    assert.strictEqual(events4[0].kind, "disk_low");

    // e. 任务恢复为 running 后再次因磁盘不足暂停 -> 再发一条新通知
    const resumed = [makeMockTask("t1", "running")];
    const secondPaused = [
      makeMockTask("t1", "paused", {
        name: "测试任务 1",
        pause_reason: diskLowReason,
        updated_at: "2026-10-26T08:10:00Z",
      }),
    ];
    const events5 = deriveTaskEvents(resumed, secondPaused);
    assert.strictEqual(events5.length, 1);
    assert.strictEqual(events5[0].id, "notif-t1-disk_low-2026-10-26T08:10:00Z");
  });

  await suite.test("6. 下载中心任务卡的动作回调必须把 Promise 交回 TaskCard（否则继续失败的 DISK_LOW 无处显示）", () => {
    const src = readSrc("../../components/desktop/DesktopDownloadDrawer.tsx");
    const discarded = src.match(/=\{\(\) => void handle(Pause|Resume|Retry|Cancel)\(/g) || [];
    assert.deepStrictEqual(
      discarded,
      [],
      "onPause/onResume/onRetry/onCancel 不得写成 () => void handleX(...)：TaskCard 的 runAction 需要拿到被拒绝的 Promise"
    );
  });

  await suite.test("7. Agent 任务卡组件不得夹在 import 语句之间定义", () => {
    const src = readSrc("../../components/desktop/DesktopAgentWorkbench.tsx");
    const firstFunction = src.search(/\n(export )?function /);
    const lastImport = src.lastIndexOf("\nimport ");
    assert.ok(firstFunction > lastImport, "DesktopAgentWorkbench.tsx 中在最后一个 import 之前出现了函数定义");
  });

  await suite.test("8. 演示任务：默认列表含一个 disk_low 暂停任务与一个无原因暂停任务；results-tasks 的暂停行带 disk_low", async () => {
    __setManualWorkbenchSubsceneForTest(null);
    __resetFixtureState();
    try {
      const tasks = await getTasks();
      const paused = tasks.filter((t) => t.status === "paused");
      const diskLow = paused.filter((t) => t.pause_reason?.kind === "disk_low");
      const plain = paused.filter((t) => !t.pause_reason);
      assert.ok(diskLow.length >= 1, "默认演示任务必须含 disk_low 暂停任务");
      assert.ok(plain.length >= 1, "默认演示任务必须保留一个无原因的暂停任务作对照");
      assert.strictEqual(pauseReasonLine(diskLow[0].pause_reason), "磁盘空间不足已暂停（D: 剩余 1.2 GB）");
    } finally {
      __setManualWorkbenchSubsceneForTest(null);
      __resetFixtureState();
    }

    __setManualWorkbenchSubsceneForTest("results-tasks");
    __resetFixtureState();
    try {
      await (await import("./fixture.ts")).getSourceViews();
      const tasks = await getTasks();
      const pausedTask = tasks.find((t) => t.task_id === "task-s1-paused-31");
      assert.ok(pausedTask, "results-tasks 必须含 task-s1-paused-31");
      assert.strictEqual(pausedTask!.pause_reason?.kind, "disk_low");
      assert.strictEqual(pauseReasonLine(pausedTask!.pause_reason), "磁盘空间不足已暂停（F: 剩余 1.2 GB）");
    } finally {
      __setManualWorkbenchSubsceneForTest(null);
      __resetFixtureState();
    }
  });
});
