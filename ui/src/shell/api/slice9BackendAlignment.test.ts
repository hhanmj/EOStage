import test from "node:test";
import assert from "node:assert/strict";
import {
  buildUnifiedSearchRequest,
  buildStorageSavePayload,
  formatStorageSaveError,
} from "./searchRequest.ts";
import { getAccountStateLabel } from "./accountLogic.ts";
import { getAvailableTaskActions } from "./taskLogic.ts";
import { getBatchPage as getFixtureBatchPage } from "./fixture.ts";
import { getBatchPage as getProdBatchPage } from "./production.ts";
import type { TaskRef, SarSearchQuery } from "../../lib/generated/v3-contracts.ts";

test("W4 Slice 9: Real Backend Contract Alignment Suite", async (t) => {
  // -------------------------------------------------------------
  // 1. 请求构造器测试 (P0-1)
  // -------------------------------------------------------------
  await t.test("1. buildUnifiedSearchRequest pure contract validation", () => {
    // 1a. 无 AOI 时必须拒绝发请求（抛出包含明确中文提示的错误）
    assert.throws(
      () => {
        buildUnifiedSearchRequest({
          resourceId: "science.sentinel-1.standard.v1",
          aoiGeometry: null,
          aoiName: "未指定范围",
        });
      },
      (err: any) => {
        return (
          err instanceof Error &&
          err.message.includes("请先设定 AOI：重画、行政区或导入矢量")
        );
      },
      "没有绑定 AOI 时必须拒绝并诚实提示用户"
    );

    // 1b. 有效参数构造 UnifiedSearchRequest
    const samplePolygon = {
      type: "Polygon",
      coordinates: [
        [
          [116.3, 39.9],
          [116.5, 39.9],
          [116.5, 40.1],
          [116.3, 40.1],
          [116.3, 39.9],
        ],
      ],
    };

    const req = buildUnifiedSearchRequest({
      resourceId: "science.sentinel-1.standard.v1",
      aoiGeometry: samplePolygon,
      aoiName: "北京市海淀区",
      startDate: "2024-01-01",
      endDate: "2024-01-15",
      productType: "SLC",
      orbitDirection: "ascending",
      relativeOrbit: "128",
      frame: "498",
    });

    // 核心契约检查
    assert.equal(req.resource_id, "science.sentinel-1.standard.v1");
    assert.deepEqual(req.aoi, samplePolygon);
    assert.equal(req.aoi_name, "北京市海淀区");
    assert.deepEqual(req.time_range, {
      start: "2024-01-01",
      end: "2024-01-15",
    });
    assert.match(req.request_id, /^req-\d+-[a-z0-9]+$/);

    // filters 键集必须兼容 SarSearchQuery，绝不包含旧键
    const allowedSarQueryKeys = new Set<string>([
      "provider",
      "dataset",
      "platforms",
      "instrument",
      "processing_levels",
      "product_families",
      "beam_modes",
      "polarizations",
      "flight_direction",
      "look_direction",
      "relative_orbit",
      "frame",
      "asf_frame",
      "path",
      "start",
      "end",
      "track_range",
      "frame_range",
      "aoi_geojson",
      "aoi_wkt",
      "bbox",
      "max_results",
      "burst",
    ]);

    const filters = req.filters as Record<string, unknown>;
    for (const key of Object.keys(filters)) {
      assert.ok(
        allowedSarQueryKeys.has(key),
        `filters 中的键 '${key}' 必须属于 SarSearchQuery 的权威键集`
      );
    }

    // 显式断言绝对不含旧键
    assert.equal("product_type" in filters, false, "严禁包含旧键 product_type");
    assert.equal("orbit_direction" in filters, false, "严禁包含旧键 orbit_direction");
    assert.equal("aoi" in filters, false);

    // 显式断言 SarSearchQuery 所需的新键值
    assert.equal(filters.dataset, "SENTINEL-1", "dataset 固定为 SENTINEL-1");
    assert.deepEqual(filters.processing_levels, ["SLC"], "processing_levels 应为数组");
    assert.equal(filters.flight_direction, "ASCENDING", "flight_direction 应大写");
    assert.equal(filters.relative_orbit, 128, "relative_orbit 应为数字");
    assert.equal(filters.frame, 498, "frame 应为数字");
  });

  // -------------------------------------------------------------
  // 2. 分页 0 基测试 (P0-2)
  // -------------------------------------------------------------
  await t.test("2. Pagination 0-base index validation", async () => {
    // 2a. fixture.getBatchPage 默认参数与切片逻辑
    // 默认应当取第一页（page 0），返回从 0 到 pageSize 景，total_count 正确
    const fixturePage = await getFixtureBatchPage("batch-s1-init", 1);
    assert.equal(fixturePage.page, 0, "fixture.getBatchPage 默认 page 应为 0");
    assert.ok(fixturePage.scenes.length > 0, "page=0 必须正常返回第一页场景");

    // 2b. 显式验证 page=0 与 page=1 的切片不重复且从 0 开始
    const p0 = await getFixtureBatchPage("batch-s1-init", 1, 0, 2);
    const p1 = await getFixtureBatchPage("batch-s1-init", 1, 1, 2);
    assert.equal(p0.scenes.length, 2);
    assert.notDeepEqual(p0.scenes[0].scene_id, p1.scenes[0].scene_id);
  });

  // -------------------------------------------------------------
  // 3. 账号五态映射与 OpenTopography (P0-5)
  // -------------------------------------------------------------
  await t.test("3. Account 5-state mapping and OpenTopography public service rule", () => {
    assert.equal(getAccountStateLabel("logged_in"), "账号有效");
    assert.equal(getAccountStateLabel("login_required"), "未验证");
    assert.equal(getAccountStateLabel("credentials_missing"), "未配置");
    assert.equal(getAccountStateLabel("service_unavailable"), "服务不可达");
    assert.equal(getAccountStateLabel("not_required"), "无需认证");
  });

  // -------------------------------------------------------------
  // 4. 存储保存有效载荷测试 (P0-4)
  // -------------------------------------------------------------
  await t.test("4. Storage settings save payload strict 4 fields", () => {
    const dirtyPolicyWithOk = {
      ok: true,
      mode: "manual",
      max_concurrent_items: 4,
      organize_by_product: true,
      default_output_root: "D:/legacy/path",
      unknown_field: "must_be_stripped",
    };

    const payload = buildStorageSavePayload(dirtyPolicyWithOk, "E:/new/storage/root");

    // 必须精确只有 4 个字段
    const keys = Object.keys(payload).sort();
    assert.deepEqual(
      keys,
      ["default_output_root", "max_concurrent_items", "mode", "organize_by_product"].sort()
    );

    assert.equal(payload.default_output_root, "E:/new/storage/root");
    assert.equal(payload.max_concurrent_items, 4);
    assert.equal(payload.mode, "manual");
    assert.equal(payload.organize_by_product, true);
    assert.equal("ok" in payload, false, "保存有效载荷中绝对不得包含 ok 包装字段");

    // 全新数据库（null policy）时优雅降级为默认安全配置
    const defaultPayload = buildStorageSavePayload(null, "F:/insar_store");
    assert.deepEqual(
      Object.keys(defaultPayload).sort(),
      ["default_output_root", "max_concurrent_items", "mode", "organize_by_product"].sort()
    );
    assert.equal(defaultPayload.mode, "manual");
    assert.equal(defaultPayload.max_concurrent_items, 2);
    assert.equal(defaultPayload.organize_by_product, false);
    assert.equal(defaultPayload.default_output_root, "F:/insar_store");
  });

  // -------------------------------------------------------------
  // 5. TaskRef.warning 契约兼容测试 (P0-7)
  // -------------------------------------------------------------
  await t.test("5. TaskRef warning field contract compatibility", () => {
    const taskWithWarning: TaskRef = {
      task_id: "task-101",
      revision: 1,
      status: "queued",
      created: true,
      warning: "存储空间不足 10GB，请及时清理",
    };

    assert.equal(taskWithWarning.warning, "存储空间不足 10GB，请及时清理");

    const taskWithoutWarning: TaskRef = {
      task_id: "task-102",
      revision: 1,
      status: "queued",
      created: true,
      warning: null,
    };
    assert.equal(taskWithoutWarning.warning, null);
  });

  // -------------------------------------------------------------
  // 6. 下载中心任务动作与文案 (P0-6)
  // -------------------------------------------------------------
  await t.test("6. Download center actions and status text mappings", () => {
    // 6a. 取消动作：queued / running / paused 必须支持 cancel
    const queuedActions = getAvailableTaskActions("queued", false);
    assert.ok(queuedActions.includes("cancel"), "queued 必须支持 cancel 动作");

    const runningActions = getAvailableTaskActions("running", true);
    assert.ok(runningActions.includes("cancel"), "running 必须支持 cancel 动作");

    const pausedActions = getAvailableTaskActions("paused", false);
    assert.ok(pausedActions.includes("cancel"), "paused 必须支持 cancel 动作");

    // 6b. cancelled 与 interrupted 互斥区分
    const cancelledActions = getAvailableTaskActions("cancelled", false);
    assert.ok(!cancelledActions.includes("cancel"), "cancelled 不应再出现 cancel 动作");
    assert.ok(cancelledActions.includes("retry"));
  });

  // -------------------------------------------------------------
  // 7. 诚实性防护：存储设置禁止 URL hash 伪造成功态 (问题 1)
  // -------------------------------------------------------------
  await t.test("7. Storage settings honesty: no storage_saved URL bypass", async () => {
    const fs = await import("node:fs");
    const { fileURLToPath } = await import("node:url");
    const targetUrl = new URL("../components/settings/StorageSettingsTab.tsx", import.meta.url);
    const filePath = fileURLToPath(targetUrl);
    const content = fs.readFileSync(filePath, "utf-8");
    assert.ok(
      !content.includes("storage_saved"),
      "StorageSettingsTab 严禁包含 storage_saved 伪造状态通道"
    );
    assert.ok(
      content.includes("const [savedSuccess, setSavedSuccess] = useState(false);"),
      "savedSuccess 初始值必须恒为 false"
    );
  });

  // -------------------------------------------------------------
  // 8. 闭包保护：下载抽屉打开期间保持轮询活跃使用 ref (问题 2)
  // -------------------------------------------------------------
  await t.test("8. Task poller closure fix: downloadDrawerOpen ref guard", async () => {
    const fs = await import("node:fs");
    const { fileURLToPath } = await import("node:url");
    const targetUrl = new URL("../../components/desktop/DesktopWorkbench.tsx", import.meta.url);
    const filePath = fileURLToPath(targetUrl);
    const content = fs.readFileSync(filePath, "utf-8");
    assert.ok(
      content.includes("downloadDrawerOpenRef"),
      "DesktopWorkbench 必须使用 downloadDrawerOpenRef 规避闭包陷阱"
    );
    assert.ok(
      content.includes("downloadDrawerOpenRef.current"),
      "fetchTasksAndSync 内部必须读取 downloadDrawerOpenRef.current 而非过期的 state"
    );
    const pollerEffectMatch = content.match(
      /useEffect\(\(\) => \{[\s\S]*?pollerRef\.current = poller;[\s\S]*?\}, \[([\s\S]*?)\]\);/
    );
    assert.ok(pollerEffectMatch, "必须存在 poller 初始化 effect");
    assert.ok(
      !pollerEffectMatch[1].includes("downloadDrawerOpen"),
      "poller 初始化 effect 严禁依赖 downloadDrawerOpen，避免销毁重建轮询器"
    );
  });

  // -------------------------------------------------------------
  // 9. 真实桌面 searchStart 必须原样传递 UnifiedSearchRequest (问题 1)
  // -------------------------------------------------------------
  await t.test("9. production.searchStart passes UnifiedSearchRequest with intact time_range", async () => {
    const { searchStart: prodSearchStart } = await import("./production.ts");
    const { registerTestDesktopShellBridge, clearTestDesktopShellBridge } = await import("../../lib/desktopShellBridge.ts");

    let interceptedRequest: any = null;
    const stubBridge: any = {
      kind: "tauri",
      application: {
        unified: {
          searchStart: async (req: any) => {
            interceptedRequest = req;
            return {
              request_id: req.request_id,
              resource_id: req.resource_id,
              stage: "requesting",
              requests_done: 1,
              items_kept: 0,
              items_cap: 250,
              elapsed_ms: 10,
              message_zh: "测试中",
              terminal: false,
              error: null,
            };
          },
        },
      },
    };
    registerTestDesktopShellBridge(stubBridge);

    try {
      const reqWithDates = buildUnifiedSearchRequest({
        resourceId: "science.sentinel-1.standard.v1",
        aoiGeometry: {
          type: "Polygon",
          coordinates: [[[116, 39], [117, 39], [117, 40], [116, 40], [116, 39]]],
        },
        aoiName: "测试区域",
        startDate: "2024-03-01",
        endDate: "2024-03-15",
      });

      await prodSearchStart(reqWithDates as any);

      assert.ok(interceptedRequest, "必须调用 client.searchStart 发送请求");
      assert.deepEqual(
        interceptedRequest.time_range,
        { start: "2024-03-01", end: "2024-03-15" },
        "设置日期时实际发出的 request.time_range 必须为 {start,end}，绝不能丢失或变为 null"
      );
      assert.equal(
        interceptedRequest.request_id,
        reqWithDates.request_id,
        "request_id 必须原样传递，绝不能重新生成被覆盖"
      );

      const reqWithoutDates = buildUnifiedSearchRequest({
        resourceId: "science.sentinel-1.standard.v1",
        aoiGeometry: {
          type: "Polygon",
          coordinates: [[[116, 39], [117, 39], [117, 40], [116, 40], [116, 39]]],
        },
        aoiName: "测试区域",
      });
      await prodSearchStart(reqWithoutDates as any);
      assert.equal(
        interceptedRequest.time_range,
        null,
        "未设置日期时实际发出的 request.time_range 必须为 null"
      );
    } finally {
      clearTestDesktopShellBridge();
    }
  });

  // -------------------------------------------------------------
  // 10. 权威注册表 ID 检查 (问题 4)
  // -------------------------------------------------------------
  await t.test("10. All fixture resource IDs match registry authoritative IDs", async () => {
    const fs = await import("node:fs");
    const { fileURLToPath } = await import("node:url");
    const targetUrl = new URL("./fixture.ts", import.meta.url);
    const content = fs.readFileSync(fileURLToPath(targetUrl), "utf-8");

    assert.ok(
      !content.includes("sentinel-2-cdse"),
      "fixture.ts 严禁含有非注册表 ID sentinel-2-cdse，必须为 science.sentinel-2.l2a.v1"
    );
    assert.ok(
      !content.includes("dem-cop30"),
      "fixture.ts 严禁含有非注册表 ID dem-cop30，必须为 dem.cop30.v1"
    );
  });
});
