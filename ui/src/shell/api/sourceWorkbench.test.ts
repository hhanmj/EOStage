import test from "node:test";
import assert from "node:assert/strict";
import {
  listSources,
  getSourceViews,
  getSourceView,
  searchStart,
  getSearchProgress,
  getBatchPage,
  createTask,
  taskAction,
  __resetFixtureState,
  __setSearchOutcome,
} from "./fixture.ts";
import type { TypedError } from "./types.ts";

test("C1 Data Workbench Contract Test Suite", async (t) => {
  t.beforeEach(() => {
    __resetFixtureState();
  });

  await t.test("Rule 1: Switching data sources switches views without clearing batches (Invariance)", async () => {
    const sources = await listSources();
    assert.ok(sources.length >= 3, "At least 3 sources available");

    const s1View = await getSourceView("science.sentinel-1.standard.v1");
    const nisarLView = await getSourceView("science.nisar.l.rslc.provisional.v1");
    const nisarSView = await getSourceView("science.nisar.s.ssar.rslc.v1");

    assert.ok(s1View && s1View.batch_id, "Sentinel-1 has active initial batch");
    assert.ok(nisarLView && nisarLView.batch_id, "NISAR L has active initial batch");
    assert.ok(nisarSView && nisarSView.batch_id, "NISAR S has active initial batch");

    // Sentinel-1 场景
    const s1Page = await getBatchPage(s1View.batch_id!, s1View.revision!);
    assert.equal(s1Page.total_count, 5, "S1 has 5 scenes");

    // NISAR L 场景
    const nisarLPage = await getBatchPage(nisarLView.batch_id!, nisarLView.revision!);
    assert.equal(nisarLPage.total_count, 2, "NISAR L has 2 scenes");

    // 切回 Sentinel-1，数据完整保留且不变
    const s1PageRevisit = await getBatchPage(s1View.batch_id!, s1View.revision!);
    assert.equal(s1PageRevisit.total_count, 5, "S1 scenes preserved byte-for-byte");
    assert.equal(s1PageRevisit.scenes[0].scene_id, s1Page.scenes[0].scene_id);
  });

  await t.test("Rule 2: Failed new search preserves the prior frozen batch and records last_error", async () => {
    const s1ViewBefore = await getSourceView("science.sentinel-1.standard.v1");
    assert.ok(s1ViewBefore?.batch_id);
    const oldBatchId = s1ViewBefore.batch_id;
    const oldItemCount = s1ViewBefore.item_count;

    // 配置下一次检索模拟失败
    __setSearchOutcome("failed");
    const searchReq = await searchStart({
      resource_id: "science.sentinel-1.standard.v1",
      aoi_name: "大同盆地测试区",
    });

    assert.equal(searchReq.terminal, false);
    assert.equal(searchReq.stage, "requesting");

    // 轮询 1: filtering
    const p1 = await getSearchProgress(searchReq.request_id);
    assert.equal(p1.stage, "filtering");

    // 轮询 2: terminal failed
    const p2 = await getSearchProgress(searchReq.request_id);
    assert.equal(p2.stage, "failed");
    assert.equal(p2.terminal, true);
    assert.ok(p2.error, "Progress has error details");

    // 关键断言：失败后该源视图的旧批次必须完整保留！
    const s1ViewAfter = await getSourceView("science.sentinel-1.standard.v1");
    assert.equal(s1ViewAfter?.batch_id, oldBatchId, "Old batch ID is untouched");
    assert.equal(s1ViewAfter?.item_count, oldItemCount, "Old item count is untouched");
    assert.ok(s1ViewAfter?.last_error, "View carries last_error for display banner");
    assert.equal(s1ViewAfter?.pending_request_id, null, "Pending request is cleared");
  });

  await t.test("Rule 3: Row task actions strictly originate from backend RowTaskState", async () => {
    const s1View = await getSourceView("science.sentinel-1.standard.v1");
    const pageBefore = await getBatchPage(s1View!.batch_id!, s1View!.revision!);
    const firstRow = pageBefore.rows[0];

    // 初始状态：未建任务，可用操作仅允许 "download"
    assert.equal(firstRow.task_id, null);
    assert.deepEqual(firstRow.actions, ["download"]);

    // 触发建任务
    const taskRef = await createTask(s1View!.batch_id!, s1View!.revision!, [firstRow.item_id]);
    assert.ok(taskRef.task_id);
    assert.equal(taskRef.status, "queued");

    // 再次获取批次行状态
    const pageAfter = await getBatchPage(s1View!.batch_id!, s1View!.revision!);
    const updatedRow = pageAfter.rows.find((r) => r.item_id === firstRow.item_id);
    assert.ok(updatedRow);
    assert.equal(updatedRow.task_status, "running");
    assert.ok(updatedRow.actions.includes("pause"), "Running task can be paused");
    assert.ok(updatedRow.actions.includes("open_task"), "Can inspect task");

    // 触发暂停任务
    await taskAction(taskRef.task_id, 1, "pause");
    const pagePaused = await getBatchPage(s1View!.batch_id!, s1View!.revision!);
    const pausedRow = pagePaused.rows.find((r) => r.item_id === firstRow.item_id);
    assert.equal(pausedRow?.task_status, "paused");
    assert.ok(pausedRow?.actions.includes("resume"), "Paused task can be resumed");
  });

  await t.test("Rule 4: Task creation returns TaskRef immediately without blocking subsequent search", async () => {
    const s1View = await getSourceView("science.sentinel-1.standard.v1");
    const batchId = s1View!.batch_id!;
    const revision = s1View!.revision!;

    const start = Date.now();
    const taskRef = await createTask(batchId, revision, ["item-test-1"]);
    const elapsed = Date.now() - start;

    assert.ok(elapsed < 100, "createTask returns immediately (<100ms)");
    assert.ok(taskRef.task_id);

    // 界面可无缝发起下一次检索
    const nextSearch = await searchStart({ resource_id: "science.sentinel-1.standard.v1" });
    assert.ok(nextSearch.request_id);
  });

  await t.test("Rule 5: Missing credentials return TypedError.needs_account for inline login", async () => {
    const s1View = await getSourceView("science.sentinel-1.standard.v1");
    try {
      await createTask(s1View!.batch_id!, s1View!.revision!, ["item-test-1"], {
        simulate_auth_error: true,
      });
      assert.fail("Should throw TypedError");
    } catch (err) {
      const typedErr = err as TypedError;
      assert.equal(typedErr.code, "AUTH_REQUIRED");
      assert.equal(typedErr.needs_account, "earthdata");
      assert.ok(typedErr.message_zh.includes("NASA Earthdata"));
    }
  });
});
