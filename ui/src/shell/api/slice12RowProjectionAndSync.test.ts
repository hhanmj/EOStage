import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { deriveVisibleRowActions } from "./rowProjection.ts";
import { createTaskRowSyncer, computeTasksSignature } from "./taskRowSync.ts";
import type { RowTaskState } from "./types.ts";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

test("W4 Slice 12-1 - Row Projection and Row Inline Actions Suite", async (t) => {
  // --------------------------------------------------------------------------
  // 1. deriveVisibleRowActions 纯函数测试（六种输入形态）
  // --------------------------------------------------------------------------
  await t.test("1. deriveVisibleRowActions maps task_status to visible actions correctly", () => {
    // 1.1 running -> ["pause"]
    const runningState: RowTaskState = {
      item_id: "scene-1",
      task_id: "task-100",
      task_status: "running",
      item_status: null,
      actions: ["pause"],
    };
    assert.deepEqual(deriveVisibleRowActions(runningState), ["pause"]);

    // 1.2 paused -> ["resume"] (即使后端 actions 仍残留 pause，也强制覆盖为 resume)
    const pausedStateWithStaleBackendPause: RowTaskState = {
      item_id: "scene-2",
      task_id: "task-101",
      task_status: "paused",
      item_status: null,
      actions: ["pause", "resume"],
    };
    assert.deepEqual(deriveVisibleRowActions(pausedStateWithStaleBackendPause), ["resume"]);

    // 1.3 failed -> [] (结果卡不设重检入口，重试/重新检索后重试留在抽屉任务卡)
    const failedState: RowTaskState = {
      item_id: "scene-3",
      task_id: "task-102",
      task_status: "failed",
      item_status: null,
      actions: ["retry"],
    };
    assert.deepEqual(deriveVisibleRowActions(failedState), []);

    // 1.4 cancelled / completed / interrupted -> []
    const cancelledState: RowTaskState = {
      item_id: "scene-4",
      task_id: "task-103",
      task_status: "cancelled",
      item_status: null,
      actions: [],
    };
    assert.deepEqual(deriveVisibleRowActions(cancelledState), []);

    const completedState: RowTaskState = {
      item_id: "scene-5",
      task_id: "task-104",
      task_status: "completed",
      item_status: null,
      actions: ["open_folder"],
    };
    assert.deepEqual(deriveVisibleRowActions(completedState), []);

    const interruptedState: RowTaskState = {
      item_id: "scene-6",
      task_id: "task-105",
      task_status: "interrupted",
      item_status: null,
      actions: ["resume"],
    };
    assert.deepEqual(deriveVisibleRowActions(interruptedState), []);

    // 1.5 queued -> 原样返回后端 actions
    const queuedState: RowTaskState = {
      item_id: "scene-7",
      task_id: "task-106",
      task_status: "queued",
      item_status: null,
      actions: ["pause"],
    };
    assert.deepEqual(deriveVisibleRowActions(queuedState), ["pause"]);

    // 1.6 无行状态或 task_status 为 null/undefined -> 默认 ["download"]
    assert.deepEqual(deriveVisibleRowActions(null), ["download"]);
    assert.deepEqual(deriveVisibleRowActions(undefined), ["download"]);

    const noTaskStatusState: RowTaskState = {
      item_id: "scene-8",
      task_id: null,
      task_status: null,
      item_status: null,
      actions: ["download"],
    };
    assert.deepEqual(deriveVisibleRowActions(noTaskStatusState), ["download"]);
  });

  // --------------------------------------------------------------------------
  // 2. computeTasksSignature 快照签名纯函数测试
  // --------------------------------------------------------------------------
  await t.test("2. computeTasksSignature calculates order-independent signatures", () => {
    const listA = [
      { task_id: "t1", status: "running", revision: 1 },
      { task_id: "t2", status: "paused", revision: 3 },
    ];
    const listB = [
      { task_id: "t2", status: "paused", revision: 3 },
      { task_id: "t1", status: "running", revision: 1 },
    ];
    assert.equal(computeTasksSignature(listA), computeTasksSignature(listB));
    assert.equal(computeTasksSignature([]), "");
    assert.notEqual(
      computeTasksSignature(listA),
      computeTasksSignature([{ task_id: "t1", status: "paused", revision: 2 }])
    );
  });

  // --------------------------------------------------------------------------
  // 3. createTaskRowSyncer 行为测试（抽屉关闭下状态变动刷新 + 序列结束无多余请求）
  // --------------------------------------------------------------------------
  await t.test("3. createTaskRowSyncer behavior: refreshes rows on status changes even when drawer is closed", async () => {
    let batchPageCallCount = 0;
    const recordedBatchRequests: Array<{ batchId: string; revision: number }> = [];
    let stateMap = new Map<string, RowTaskState>();

    const rowStateTimeline: RowTaskState[][] = [
      // 第一次调用：返回 running 状态的 row
      [
        {
          item_id: "item-001",
          task_id: "task-1",
          task_status: "running",
          item_status: null,
          actions: ["pause"],
        },
      ],
      // 第二次调用：返回 paused 状态的 row
      [
        {
          item_id: "item-001",
          task_id: "task-1",
          task_status: "paused",
          item_status: null,
          actions: ["resume"],
        },
      ],
      // 第三次调用：返回 cancelled 状态的 row
      [
        {
          item_id: "item-001",
          task_id: "task-1",
          task_status: "cancelled",
          item_status: null,
          actions: [],
        },
      ],
    ];

    const syncer = createTaskRowSyncer({
      getActiveBatch: () => ({ batchId: "batch-xyz", revision: 1 }),
      getBatchPage: async (batchId, revision) => {
        const rows = rowStateTimeline[batchPageCallCount] || [];
        batchPageCallCount++;
        recordedBatchRequests.push({ batchId, revision });
        return { rows };
      },
      setRowStates: (updater) => {
        stateMap = updater(stateMap);
      },
      isMounted: () => true,
    });

    // Step A: 抽屉关闭，派发 running 快照
    const snapshotRunning = [{ task_id: "task-1", status: "running", revision: 1 }];
    const didFetch1 = await syncer.syncOnTaskSnapshot(snapshotRunning, { drawerOpen: false });
    assert.equal(didFetch1, true, "First snapshot must trigger row fetch even when drawer is closed");
    assert.equal(batchPageCallCount, 1);
    assert.equal(stateMap.get("item-001")?.task_status, "running");

    // Step B: 抽屉关闭，派发相同 running 快照（无状态变动）
    const didFetch2 = await syncer.syncOnTaskSnapshot(snapshotRunning, { drawerOpen: false });
    assert.equal(didFetch2, false, "Identical snapshot with drawer closed must NOT trigger row fetch");
    assert.equal(batchPageCallCount, 1);

    // Step C: 抽屉关闭，派发 paused 状态（状态迁移：running -> paused）
    const snapshotPaused = [{ task_id: "task-1", status: "paused", revision: 2 }];
    const didFetch3 = await syncer.syncOnTaskSnapshot(snapshotPaused, { drawerOpen: false });
    assert.equal(didFetch3, true, "State migration to paused must trigger row fetch even when drawer is closed");
    assert.equal(batchPageCallCount, 2);
    assert.equal(stateMap.get("item-001")?.task_status, "paused");

    // Step D: 抽屉关闭，再次派发相同 paused 快照
    const didFetch4 = await syncer.syncOnTaskSnapshot(snapshotPaused, { drawerOpen: false });
    assert.equal(didFetch4, false, "Identical paused snapshot with drawer closed must NOT trigger row fetch");
    assert.equal(batchPageCallCount, 2);

    // Step E: 抽屉关闭，派发 cancelled 状态（状态迁移：paused -> cancelled）
    const snapshotCancelled = [{ task_id: "task-1", status: "cancelled", revision: 3 }];
    const didFetch5 = await syncer.syncOnTaskSnapshot(snapshotCancelled, { drawerOpen: false });
    assert.equal(didFetch5, true, "State migration to cancelled must trigger row fetch even when drawer is closed");
    assert.equal(batchPageCallCount, 3);
    assert.equal(stateMap.get("item-001")?.task_status, "cancelled");

    // Step F: 抽屉关闭，再次派发相同 cancelled 快照
    const didFetch6 = await syncer.syncOnTaskSnapshot(snapshotCancelled, { drawerOpen: false });
    assert.equal(didFetch6, false, "No extra fetch after terminal state reached");
    assert.equal(batchPageCallCount, 3, "Total batchPage calls must be strictly 3");

    // Step G: 当抽屉打开时，既有连续刷新行为仍然保持
    const didFetch7 = await syncer.syncOnTaskSnapshot(snapshotCancelled, { drawerOpen: true });
    assert.equal(didFetch7, true, "Drawer open must maintain continuous refresh even with identical snapshot");
    assert.equal(batchPageCallCount, 4);
  });

  // --------------------------------------------------------------------------
  // 4. 源码级接线断言
  // --------------------------------------------------------------------------
  await t.test("4. source-level assertions for DesktopWorkbench and DesktopCatalogDock", () => {
    // 4.1 DesktopWorkbench.tsx 必须引入并接线 createTaskRowSyncer，不再以 pollActive 门禁行状态批次刷新
    const workbenchPath = path.resolve(__dirname, "../../components/desktop/DesktopWorkbench.tsx");
    const workbenchContent = fs.readFileSync(workbenchPath, "utf-8");

    assert.match(
      workbenchContent,
      /createTaskRowSyncer/,
      "DesktopWorkbench.tsx must import and use createTaskRowSyncer"
    );
    assert.doesNotMatch(
      workbenchContent,
      /if\s*\(\s*pollActive\s*&&\s*activeBatchRef\.current/,
      "DesktopWorkbench.tsx must NOT gate batch row fetching behind pollActive"
    );

    // 4.2 CatalogResultRow.tsx 必须使用 deriveVisibleRowActions 纯函数映射行操作，ManualWorkbench 无直接回退
    const rowPath = path.resolve(__dirname, "../../components/desktop/CatalogResultRow.tsx");
    const rowContent = fs.readFileSync(rowPath, "utf-8");
    assert.match(
      rowContent,
      /deriveVisibleRowActions/,
      "CatalogResultRow.tsx must import and use deriveVisibleRowActions"
    );

    const catalogDockPath = path.resolve(__dirname, "../../components/desktop/ManualWorkbench.tsx");
    const catalogDockContent = fs.readFileSync(catalogDockPath, "utf-8");
    assert.doesNotMatch(
      catalogDockContent,
      /const\s+actions\s*=\s*rowState\?\.actions\s*\|\|\s*\["download"\]/,
      "ManualWorkbench.tsx must NOT directly fallback to rowState?.actions"
    );
  });
});
