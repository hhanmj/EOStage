import test from "node:test";
import assert from "node:assert/strict";
import { UnifiedClient } from "./unifiedClient.ts";
import { parseTaskOverview, parseStoreTaskSummary } from "./taskOverviewParser.ts";
import { projectSnapshotItemToSceneRow, truncateMiddle } from "./rowProjection.ts";
import { UnifiedPoller, type PollerTimerDependencies } from "./poller.ts";
import type {
  SnapshotItem,
  UnifiedSearchRequest,
  TypedError,
  StoreTaskSummary,
  StoreTaskHistoryPage,
} from "./types.ts";
import * as prod from "./production.ts";
import { registerTestDesktopShellBridge, clearTestDesktopShellBridge, type DesktopShellBridge } from "../../lib/desktopShellBridge.ts";

test("W4 Slice 7 - Unified Wiring & Real Commands Test Suite", async (t) => {
  // a) 命令封装测试（桩 invoke 逐命令断言命令名与载荷、TypedError 透传）
  await t.test("a) Command encapsulation and payload assertions with stub invoke", async () => {
    const invokedCalls: Array<{ cmd: string; args?: Record<string, unknown> }> = [];

    const mockInvoke = async (cmd: string, args?: Record<string, unknown>): Promise<unknown> => {
      invokedCalls.push({ cmd, args });

      switch (cmd) {
        case "application_list_sources":
          return [
            {
              resource_id: "science.sentinel-1.standard.v1",
              group_id: "sentinel",
              display_name_zh: "哨兵1号",
              family: "sar",
              search_mode: "sar",
              filter_schema_id: null,
              filter_schema_revision: null,
              result_cap: 250,
              accounts: ["earthdata"],
              network_default: "direct",
              availability: "available",
              availability_reason_zh: null,
            },
          ];
        case "application_source_views":
          return [
            {
              resource_id: "science.sentinel-1.standard.v1",
              batch_id: "b-1",
              revision: 1,
              item_count: 10,
              searched_at: "2026-10-24T00:00:00Z",
              pending_request_id: null,
              last_error: null,
            },
          ];
        case "application_search_start":
          return {
            request_id: (args?.request as any)?.request_id || "req-1",
            resource_id: (args?.request as any)?.resource_id || "science.sentinel-1.standard.v1",
            stage: "planning",
            requests_done: 0,
            items_kept: 0,
            items_cap: 250,
            elapsed_ms: 0,
            message_zh: "检索已启动",
            terminal: false,
            error: null,
          };
        case "application_search_progress":
          return {
            request_id: args?.requestId,
            resource_id: "science.sentinel-1.standard.v1",
            stage: "done",
            requests_done: 2,
            items_kept: 15,
            items_cap: 250,
            elapsed_ms: 250,
            message_zh: "检索完成",
            terminal: true,
            error: null,
          };
        case "application_search_cancel":
          return true;
        case "application_batch_page":
          return {
            batch_id: args?.batchId,
            revision: args?.revision,
            page: args?.page,
            page_size: args?.pageSize,
            total_count: 1,
            items: [],
            rows: [],
          };
        case "application_create_task": {
          if ((args?.options as any)?.output_root === "/unauthorized") {
            const err: TypedError = {
              code: "AUTH_REQUIRED",
              message_zh: "需要登录账户",
              retryable: false,
              needs_account: "earthdata",
              needs_user_confirmation: false,
            };
            return Promise.reject(err);
          }
          return {
            task_id: "task-100",
            revision: 1,
            status: "queued",
            created: true,
          };
        }
        case "application_task_action":
          return {
            task_id: args?.taskId,
            revision: (args?.revision as number) + 1,
            status: "paused",
            created: false,
          };
        case "application_task_overview":
          return {
            queue: [],
            history_page: {
              page_size: 50,
              has_more: false,
              next_cursor: null,
              items: [],
            },
          };
        case "application_account_session_statuses":
          return [
            { provider: "cdse", state: "logged_in" },
            { provider: "usgs_m2m", state: "login_required" },
            { provider: "earthdata", state: "logged_in" },
            { provider: "bhoonidhi", state: "credentials_missing" },
            { provider: "mpc", state: "not_required" },
            { provider: "opentopography", state: "not_required" },
            { provider: "gacos", state: "credentials_missing" },
          ];
        case "application_test_network_connection":
          return "http_reached";
        default:
          throw new Error(`Unexpected command: ${cmd}`);
      }
    };

    const client = new UnifiedClient({ invoke: mockInvoke });

    // 1. listSources
    const sources = await client.listSources();
    assert.equal(sources.length, 1);
    assert.equal(invokedCalls[0].cmd, "application_list_sources");

    // 2. sourceViews
    const views = await client.sourceViews();
    assert.equal(views.length, 1);
    assert.equal(invokedCalls[1].cmd, "application_source_views");

    // 3. searchStart 必填 aoi_name 校验
    await assert.rejects(
      async () => {
        await client.searchStart({
          resource_id: "s1",
          aoi: {},
          aoi_name: "", // 空串必须被前端拒绝
          time_range: null,
          filters: {},
          request_id: "r-1",
        });
      },
      /AOI_NAME_REQUIRED/
    );

    // 3b. searchStart 正常调用
    const searchReq: UnifiedSearchRequest = {
      resource_id: "science.sentinel-1.standard.v1",
      aoi: { type: "Polygon" },
      aoi_name: "大同盆地真实区域",
      time_range: null,
      filters: {},
      request_id: "req-1",
    };
    const progress = await client.searchStart(searchReq);
    assert.equal(progress.request_id, "req-1");
    assert.equal(invokedCalls[2].cmd, "application_search_start");
    assert.deepEqual(invokedCalls[2].args, { request: searchReq });

    // 4. searchProgress
    const p = await client.searchProgress("req-1");
    assert.equal(p?.stage, "done");
    assert.equal(invokedCalls[3].cmd, "application_search_progress");
    assert.deepEqual(invokedCalls[3].args, { requestId: "req-1" });

    // 5. searchCancel
    const cancelled = await client.searchCancel("req-1");
    assert.equal(cancelled, true);
    assert.equal(invokedCalls[4].cmd, "application_search_cancel");
    assert.deepEqual(invokedCalls[4].args, { requestId: "req-1" });

    // 6. batchPage
    const batchPage = await client.batchPage("b-1", 1, 1, 50);
    assert.equal(batchPage.batch_id, "b-1");
    assert.equal(invokedCalls[5].cmd, "application_batch_page");
    assert.deepEqual(invokedCalls[5].args, { batchId: "b-1", revision: 1, page: 1, pageSize: 50 });

    // 7. createTask 正常
    const task = await client.createTask("b-1", 1, ["item-1"], {
      output_root: "F:/InSAR_Data",
      concurrency: 2,
      layout: "sentinel1_slc",
    });
    assert.equal(task.task_id, "task-100");
    assert.equal(invokedCalls[6].cmd, "application_create_task");

    // 7b. createTask TypedError 透传 (needs_account 可被捕获)
    try {
      await client.createTask("b-1", 1, ["item-1"], {
        output_root: "/unauthorized",
        concurrency: 2,
        layout: "sentinel1_slc",
      });
      assert.fail("Should have thrown TypedError");
    } catch (err: any) {
      assert.equal(err.code, "AUTH_REQUIRED");
      assert.equal(err.needs_account, "earthdata");
    }

    // 8. taskAction
    const actionResult = await client.taskAction("task-100", 1, "pause");
    assert.equal(actionResult.status, "paused");
    assert.equal(invokedCalls[8].cmd, "application_task_action");
    assert.deepEqual(invokedCalls[8].args, { taskId: "task-100", revision: 1, action: "pause" });

    // 9. taskOverview
    const overview = await client.taskOverview();
    assert.equal(overview.queue.length, 0);
    assert.equal(invokedCalls[9].cmd, "application_task_overview");

    // 10. accountSessionStatuses
    const statuses = await client.accountSessionStatuses();
    assert.equal(statuses.length, 7);
    assert.equal(invokedCalls[10].cmd, "application_account_session_statuses");

    // 11. testNetworkConnection
    const netStatus = await client.testNetworkConnection();
    assert.equal(netStatus, "http_reached");
    assert.equal(invokedCalls[11].cmd, "application_test_network_connection");

    // 12. searchStart with bypassCache passes bypassCache: true only when enabled
    await client.searchStart(searchReq, { bypassCache: true });
    assert.equal(invokedCalls[12].cmd, "application_search_start");
    assert.deepEqual(invokedCalls[12].args, { request: searchReq, bypassCache: true });
  });

  // b) TaskOverview 解析器测试（合法、缺字段、非法类型的严格拒绝）
  await t.test("b) TaskOverview parser strict validation", () => {
    const validSummary: StoreTaskSummary = {
      task_id: "t-1",
      batch_id: "b-1",
      provider_id: "cdse",
      dataset_id: "s1",
      concurrency: 2,
      task_type: "sar_slc",
      created_at: "2026-10-24T00:00:00Z",
      status: "running",
      revision: 1,
      updated_at: "2026-10-24T00:01:00Z",
      selected_count: 5,
      delivery_requested_count: 5,
      delivery_rejected_count: 0,
      checked_count: 5,
      active_count: 1,
      paused_count: 0,
      queued_count: 4,
      completed_count: 0,
      failed_count: 0,
      skipped_count: 0,
      cancelled_count: 0,
      terminal_bytes: 0,
      active_bytes: 1048576,
      other_bytes: 0,
      transferred_bytes: 1048576,
      total_bytes: 5242880,
      source_total_bytes: 5242880,
      bytes_per_second: 500000,
      transfer_active_millis: 2000,
    };

    // 1. 合法 summary 解析
    const parsed = parseStoreTaskSummary(validSummary);
    assert.equal(parsed.task_id, "t-1");
    assert.equal(parsed.transferred_bytes, 1048576);

    // 2. 缺少必填字段 status 抛错
    assert.throws(() => {
      const { status: _, ...rest } = validSummary;
      parseStoreTaskSummary(rest);
    }, /TASK_OVERVIEW_INVALID/);

    // 3. 非法 status 枚举抛错
    assert.throws(() => {
      parseStoreTaskSummary({ ...validSummary, status: "unknown_status" });
    }, /TASK_OVERVIEW_INVALID/);

    // 4. 非对象拒绝
    assert.throws(() => {
      parseStoreTaskSummary(null);
    }, /TASK_OVERVIEW_INVALID/);
    assert.throws(() => {
      parseStoreTaskSummary("string");
    }, /TASK_OVERVIEW_INVALID/);

    // 5. 合法 overview 解析
    const validOverview = {
      queue: [validSummary],
      history_page: {
        page_size: 20,
        has_more: false,
        next_cursor: null,
        items: [{ ...validSummary, archived_at: "2026-10-24T01:00:00Z" }],
      },
    };
    const parsedOverview = parseTaskOverview(validOverview);
    assert.equal(parsedOverview.queue.length, 1);
    assert.equal(parsedOverview.history_page.items.length, 1);
    assert.equal(parsedOverview.history_page.items[0].archived_at, "2026-10-24T01:00:00Z");

    // 6. overview 队列缺少数组拒绝
    assert.throws(() => {
      parseTaskOverview({ queue: "not_array", history_page: validOverview.history_page });
    }, /TASK_OVERVIEW_INVALID/);
  });

  // c) SnapshotItem → 显示行投影与诚实性断言
  await t.test("c) SnapshotItem to SceneRow projection honesty (Audit P0-02)", () => {
    // 1. 存在字段映射
    const itemFull: SnapshotItem = {
      item_id: "S1A_IW_SLC__1SDV_20261024T060000_050000_1234.SAFE",
      provider_id: "cdse",
      display_name: "S1A_IW_SLC__1SDV_20261024T060000_050000_1234.SAFE",
      acquisition_datetime: "2026-10-24T06:00:00Z",
      footprint_geojson: { type: "Polygon", coordinates: [] },
      metadata: {
        orbit_direction: "ascending",
        relative_orbit: 128,
        frame: 498,
        polarization: "VV+VH",
        size_bytes: 4200000000,
        platform: "Sentinel-1A",
      },
      payload: {},
    };

    const rowFull = projectSnapshotItemToSceneRow(itemFull);
    assert.equal(rowFull.scene_id, itemFull.item_id);
    assert.equal(rowFull.orbit_direction, "ascending");
    assert.equal(rowFull.relative_orbit, 128);
    assert.equal(rowFull.frame, 498);
    assert.equal(rowFull.polarization, "VV+VH");
    assert.equal(rowFull.file_size_remote, 4200000000);

    // 2. 字段缺失时：绝不伪造回退值！
    const itemEmpty: SnapshotItem = {
      item_id: "UNKNOWN_ITEM",
      provider_id: "asf",
      display_name: "UNKNOWN_ITEM",
      acquisition_datetime: null,
      footprint_geojson: null,
      metadata: {}, // 空元数据
      payload: {},
    };

    const rowEmpty = projectSnapshotItemToSceneRow(itemEmpty);
    assert.equal(rowEmpty.orbit_direction, "", "未提供方向时必须为空串（UI呈现未知），不可猜测");
    assert.equal(rowEmpty.relative_orbit, null, "未提供轨道号时绝不可填补 128");
    assert.equal(rowEmpty.frame, null, "未提供分幅号时绝不可填补 498");
    assert.equal(rowEmpty.polarization, "");
    assert.equal(rowEmpty.file_size_remote, null);

    // 3. 长文件名中间截断 (视觉基线 §4b-6)
    const longName = "S1A_IW_SLC__1SDV_20261024T060000_20261024T060030_050000_05FFFF_1234.SAFE";
    const truncated = truncateMiddle(longName, 20, 14);
    assert.ok(truncated.includes("..."));
    assert.ok(truncated.startsWith("S1A_IW_SLC__1SDV_202"));
    assert.ok(truncated.endsWith("FFFF_1234.SAFE"));
  });

  // d) 检索轮询器测试（检索 250ms / 检索完成停止）
  await t.test("d) UnifiedPoller cadence and stopSearch cleanup", () => {
    const activeIntervals = new Map<number, { callback: () => void; timeout: number }>();
    let nextTimerId = 1;

    const mockTimers: PollerTimerDependencies = {
      setInterval: (cb, ms) => {
        const id = nextTimerId++;
        activeIntervals.set(id, { callback: cb, timeout: ms });
        return id;
      },
      clearInterval: (id) => {
        activeIntervals.delete(id);
      },
    };

    let searchPollCount = 0;

    const poller = new UnifiedPoller({
      onPollSearch: () => {
        searchPollCount++;
      },
      timers: mockTimers,
    });

    // 初始状态应处于空闲
    assert.equal(poller.isIdle(), true);
    assert.equal(activeIntervals.size, 0);

    // 启动检索轮询 (250ms)
    poller.startSearch("req-test-1");
    assert.equal(poller.isSearching(), true);
    assert.equal(activeIntervals.size, 1);
    const searchTimer = Array.from(activeIntervals.values()).find((v) => v.timeout === 250);
    assert.ok(searchTimer, "检索轮询定时器必须为 250ms");

    // 模拟时钟触发 2 次检索轮询
    searchTimer.callback();
    searchTimer.callback();
    assert.equal(searchPollCount, 2);

    // 检索完成：停止检索轮询
    poller.stopSearch();
    assert.equal(poller.isSearching(), false);
    assert.equal(poller.isIdle(), true);
    assert.equal(activeIntervals.size, 0, "无进行中检索时必须停止检索定时器");
  });

  // e) 验证 production.ts 真实接线与浏览器守卫断言 (GREEN 验收)
  await t.test("e) production.ts wiring verification and browser guard", async () => {
    assert.equal(prod.isFixtureBackend(), false, "production 环境不得标记为 fixture");
    assert.equal(prod.isUnifiedWired(), true, "production 必须标记为已接通真实命令");
    assert.equal(prod.isAgentWired(), false, "智能助手后端未合入，必须保持诚实未接入状态");

    // 浏览器预览守卫：非 Tauri 环境下返回诚实提示，不发起后端调用
    const searchRes = await prod.searchStart({
      resource_id: "science.sentinel-1.standard.v1",
      aoi_name: "测试AOI",
    });
    assert.equal(searchRes.stage, "failed");
    assert.equal(searchRes.error?.code, "DESKTOP_HOST_REQUIRED");
    assert.equal(searchRes.error?.message_zh, "需在桌面应用中运行");

    // 重试兜底非桌面宿主断言：真实命令要求桌面应用环境，拒绝且包含 DESKTOP_HOST_REQUIRED
    await assert.rejects(
      async () => {
        await prod.retryTaskFallback("task-1");
      },
      /DESKTOP_HOST_REQUIRED/
    );

    // 非 Tauri 环境获取任务列表诚实返回空数组
    const tasks = await prod.getTasks();
    assert.deepEqual(tasks, []);
  });

  // f) 修正项验收测试：createTask output_root 校验与 DEM 转换类目推导
  await t.test("f) production.ts createTask guards and DEM category derivation (Fix verification)", async () => {
    // 1. DEM 类目推导契约 (contracts/v3/resource-registry.v1.json: dem.* 为 DEM 转换，后端 task_type 此时为 null)
    const baseSummary: StoreTaskSummary = {
      task_id: "task-dem-1",
      batch_id: "batch-dem",
      provider_id: "copernicus",
      dataset_id: "dem.cop30.v1",
      concurrency: 2,
      output_root: "/data/dem",
      selected_count: 1,
      completed_count: 1,
      failed_count: 0,
      delivery_rejected_count: 0,
      transferred_bytes: 1024,
      total_bytes: 1024,
      status: "completed",
      task_type: null, // 后端 core_restore.rs 对于 DEM 为 null
      created_at: "2026-10-24T00:00:00Z",
      updated_at: "2026-10-24T00:01:00Z",
    };

    const appTaskDem = prod.mapStoreTaskSummaryToApplicationTask(baseSummary);
    assert.equal(appTaskDem.category, "dem_convert", "dataset_id 以 dem. 开头必须推导为 dem_convert");

    const appTaskNasar = prod.mapStoreTaskSummaryToApplicationTask({
      ...baseSummary,
      dataset_id: "dem.nasadem.v1",
    });
    assert.equal(appTaskNasar.category, "dem_convert");

    const appTaskSar = prod.mapStoreTaskSummaryToApplicationTask({
      ...baseSummary,
      dataset_id: "sentinel-1-slc",
    });
    assert.equal(appTaskSar.category, "scene_download", "普通 SAR 数据集必须归入 scene_download");

    // 2. createTask 的 output_root 必填守卫与 layout 默认 direct
    let createdCall: any = null;
    const mockTauriBridge: DesktopShellBridge = {
      kind: "tauri",
      platform: "windows",
      application: {
        unified: {
          listSources: async () => [],
          sourceViews: async () => [],
          searchStart: async () => ({} as any),
          searchProgress: async () => ({} as any),
          searchCancel: async () => {},
          batchPage: async () => ({} as any),
          createTask: async (_b, _r, _ids, opt) => {
            createdCall = opt;
            return { task_id: "t-1", batch_id: _b, revision: _r, created: true };
          },
          taskAction: async () => {},
          taskOverview: async () => ({} as any),
          accountSessionStatuses: async () => [],
          getNetworkSettings: async () => ({} as any),
          saveNetworkSettings: async () => {},
          testNetworkConnection: async () => "http_reached",
        },
      } as any,
    } as any;

    registerTestDesktopShellBridge(mockTauriBridge);

    try {
      // a) output_root 缺失时必须拒绝并抛出 OUTPUT_ROOT_REQUIRED，绝不把空路径传给后端或写死路径
      await assert.rejects(
        async () => {
          await prod.createTask("b-1", 1, ["item-1"], { output_root: "" });
        },
        (err: any) => {
          return err?.code === "OUTPUT_ROOT_REQUIRED" && err?.message_zh?.includes("需先配置存储位置");
        }
      );

      // b) layout 未传时必须默认为中性档 "direct"，绝不编造特定产品的 sentinel1_slc
      createdCall = null;
      await prod.createTask("b-1", 1, ["item-1"], { output_root: "D:/insar_store" });
      assert.ok(createdCall, "必须成功调用底层 createTask");
      assert.equal(createdCall.layout, "direct", "未显式指定 layout 时必须默认 direct");
      assert.equal(createdCall.output_root, "D:/insar_store");

      // c) 调用方显式传 layout 时保持透传
      createdCall = null;
      await prod.createTask("b-1", 1, ["item-1"], {
        output_root: "D:/insar_store",
        layout: "sar_dataset_processing",
      });
      assert.equal(createdCall.layout, "sar_dataset_processing", "显式传 layout 必须透传");
    } finally {
      clearTestDesktopShellBridge();
    }
  });
});
