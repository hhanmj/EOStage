import test from "node:test";
import assert from "node:assert/strict";
import {
  registerTestDesktopShellBridge,
  clearTestDesktopShellBridge,
  type DesktopShellBridge,
} from "../../lib/desktopShellBridge.ts";
import {
  isFixtureBackend,
  listSources,
  getAgentSessionsSync,
  getTasks,
} from "./index.ts";

test("W4 Slice 8 P0: Host-based Backend Selection Test Suite", async (t) => {
  // 确保测试开始时无桥
  clearTestDesktopShellBridge();

  await t.test("1. Tauri 宿主 + DEV 环境 ⇒ 必须走生产后端，绝不返回 fixture 数据", async () => {
    let stubCalled = false;
    let overviewCalled = false;
    const stubSources = [
      {
        resource_id: "stub-real-sentinel-1",
        group_id: "sentinel",
        display_name_zh: "真实生产哨兵数据源",
        family: "sar" as const,
        search_mode: "sar" as const,
        filter_schema_id: null,
        filter_schema_revision: null,
        result_cap: 250,
        accounts: [],
        network_default: "direct" as const,
        availability: "available" as const,
        availability_reason_zh: null,
      },
    ];

    const stubBridge: DesktopShellBridge = {
      kind: "tauri",
      application: {
        unified: {
          listSources: async () => {
            stubCalled = true;
            return stubSources;
          },
          taskOverview: async () => {
            overviewCalled = true;
            return {
              queue: [],
              history_page: {
                page_size: 50,
                has_more: false,
                next_cursor: null,
                items: [],
              },
            };
          },
        } as any,
      } as any,
    } as any;

    try {
      // 注入 Tauri 桥桩
      registerTestDesktopShellBridge(stubBridge);

      // a) isFixtureBackend 在 Tauri 宿主下必须返回 false
      assert.equal(
        isFixtureBackend(),
        false,
        "当宿主为 Tauri 时，无论是否处于 DEV 环境，isFixtureBackend() 必须为 false"
      );

      // b) listSources 必须路由到生产后端桩，绝不返回 fixture 数据
      const sources = await listSources();
      assert.equal(stubCalled, true, "必须调用到 Tauri 生产后端的 listSources 接口");
      assert.equal(sources.length, 1);
      assert.equal(sources[0].resource_id, "stub-real-sentinel-1");

      // c) 同步导出方法在 Tauri 宿主下必须走 production 实现（如未接入的 Agent 会话返回空数组）
      const sessions = getAgentSessionsSync();
      assert.deepEqual(sessions, [], "Tauri 宿主下未接入的会话同步方法应返回空数组而非 fixture 数据");

      // d) getTasks 经生产后端调用
      const tasks = await getTasks();
      assert.equal(overviewCalled, true, "必须调用到 Tauri 生产后端的 taskOverview 接口");
      assert.deepEqual(tasks, [], "Tauri 宿主下空队列返回空数组");
    } finally {
      clearTestDesktopShellBridge();
    }
  });

  await t.test("2. 无桥（浏览器预览）+ DEV 环境 ⇒ 走 fixture 演示数据（保持现状语义）", async () => {
    clearTestDesktopShellBridge();

    // a) 无桥时 isFixtureBackend 必须为 true
    assert.equal(
      isFixtureBackend(),
      true,
      "无桥环境下在 DEV/Node 中 isFixtureBackend() 必须为 true"
    );

    // b) listSources 必须返回 fixture 演示数据
    const sources = await listSources();
    assert.ok(sources.length > 0, "fixture 数据源列表不能为空");
    assert.ok(
      sources.some((s) => s.resource_id === "science.sentinel-1.standard.v1"),
      "fixture 应包含 science.sentinel-1.standard.v1 演示数据源"
    );

    // c) 同步导出方法在 fixture 下返回 fixture 数据
    const sessions = getAgentSessionsSync();
    assert.ok(sessions.length > 0, "fixture 下同步会话数据应不为空");
  });

  await t.test("3. 切片 8 审计项回归: 守卫防重入、杜绝策略伪造、存储保存结果诚实校验", async () => {
    const fs = await import("node:fs");
    const path = await import("node:path");
    const { fileURLToPath } = await import("node:url");

    const currentDir = path.dirname(fileURLToPath(import.meta.url));
    const mapSrc = fs.readFileSync(path.resolve(currentDir, "../../components/WorkbenchMap.tsx"), "utf-8");
    const dwbSrc = fs.readFileSync(path.resolve(currentDir, "../../components/desktop/DesktopWorkbench.tsx"), "utf-8");
    const sstSrc = fs.readFileSync(path.resolve(currentDir, "../components/settings/StorageSettingsTab.tsx"), "utf-8");

    // 1. DrawLayer 一次性守卫
    assert.ok(mapSrc.includes("isFinishedRef = useRef(false)"), "DrawLayer 必须包含 isFinishedRef 守卫标志");
    assert.ok(mapSrc.includes("if (isFinishedRef.current) return"), "finishRect 必须有一次性守卫阻断逻辑");
    assert.ok(mapSrc.includes("isFinishedRef.current = true"), "finishRect 执行时必须先置位守卫");

    // 2. 切片 9 契约对齐：使用 buildStorageSavePayload 纯函数安全剥离 ok 并支持默认载荷构造
    assert.ok(
      dwbSrc.includes("buildStorageSavePayload"),
      "DesktopWorkbench 必须通过 buildStorageSavePayload 纯函数构造严格四字段保存载荷"
    );
    assert.ok(
      dwbSrc.includes("formatStorageSaveError"),
      "DesktopWorkbench 必须使用专用的 formatStorageSaveError 进行真实中文错误呈现"
    );
    assert.ok(
      dwbSrc.includes("Promise<boolean>"),
      "handleSaveStoragePath 必须显式声明 Promise<boolean> 返回类型"
    );

    // 3. 存储设置页只在 true 时显示已保存
    assert.ok(
      sstSrc.includes("const ok = await onSaveStoragePath(trimmed)"),
      "StorageSettingsTab 必须接收并判断 onSaveStoragePath 的返回值"
    );
    assert.ok(
      sstSrc.includes("if (ok === true)"),
      "StorageSettingsTab 只有当返回显式 true 时才判定保存成功"
    );
  });
});
