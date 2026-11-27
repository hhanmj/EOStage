import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import {
  parseShellRoute,
  resolveInitialRoute,
  routeMeta,
  readLastRoute,
  writeLastRoute,
  SHELL_LAST_ROUTE_KEY,
  type ShellRoute,
  type SimpleStorage,
} from "./shellRoutes.ts";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const uiSrcDir = path.resolve(__dirname, "../..");
const desktopDir = path.resolve(uiSrcDir, "components/desktop");

test("W5-1a Shell Navigation & Titlebar Specification", async (t) => {
  // ==========================================================================
  // 1. parseShellRoute
  // ==========================================================================
  await t.test("1. parseShellRoute: parses strict 5 shell routes, rejects other hashes", () => {
    assert.strictEqual(parseShellRoute("#/agent"), "agent");
    assert.strictEqual(parseShellRoute("#/history"), "history");
    assert.strictEqual(parseShellRoute("#/workbench"), "workbench");
    assert.strictEqual(parseShellRoute("#/downloads"), "downloads");
    assert.strictEqual(parseShellRoute("#/settings"), "settings");

    // 不合规或未知 hash 必须严格返回 null
    assert.strictEqual(parseShellRoute(""), null);
    assert.strictEqual(parseShellRoute("#"), null);
    assert.strictEqual(parseShellRoute("#agent"), null);
    assert.strictEqual(parseShellRoute("#/unknown"), null);
    assert.strictEqual(parseShellRoute("#harness"), null);
    assert.strictEqual(parseShellRoute("#/agent?x=1"), null);
  });

  // ==========================================================================
  // 2. resolveInitialRoute
  // ==========================================================================
  await t.test("2. resolveInitialRoute: prioritizes currentHash over stored, falls back to agent", () => {
    // 优先从有效 hash 解析
    assert.strictEqual(resolveInitialRoute("#/workbench", "history"), "workbench");
    assert.strictEqual(resolveInitialRoute("#/history", "downloads"), "history");
    assert.strictEqual(resolveInitialRoute("#/settings", "agent"), "settings");

    // hash 无效时回退到 stored (仅限 4 个页面路由)
    assert.strictEqual(resolveInitialRoute("", "workbench"), "workbench");
    assert.strictEqual(resolveInitialRoute("#", "history"), "history");
    assert.strictEqual(resolveInitialRoute("#/unknown", "downloads"), "downloads");
    assert.strictEqual(resolveInitialRoute("", "agent"), "agent");

    // stored 为 "settings" 视为无效，必须回退到默认 "agent"
    assert.strictEqual(resolveInitialRoute("", "settings"), "agent");
    assert.strictEqual(resolveInitialRoute("#unknown", "settings"), "agent");

    // stored 为空或未知值时回退到 "agent"
    assert.strictEqual(resolveInitialRoute("", null), "agent");
    assert.strictEqual(resolveInitialRoute("", undefined), "agent");
    assert.strictEqual(resolveInitialRoute("", "invalid_route"), "agent");
  });

  // ==========================================================================
  // 3. routeMeta
  // ==========================================================================
  await t.test("3. routeMeta: carries pure labels for all 5 routes", () => {
    assert.deepStrictEqual(routeMeta("agent"), { label: "对话" });
    assert.deepStrictEqual(routeMeta("history"), { label: "全部对话" });
    assert.deepStrictEqual(routeMeta("workbench"), { label: "地图工作台" });
    assert.deepStrictEqual(routeMeta("downloads"), { label: "下载中心" });
    assert.deepStrictEqual(routeMeta("settings"), { label: "设置" });
  });

  // ==========================================================================
  // 4. Persistence helpers (readLastRoute, writeLastRoute)
  // ==========================================================================
  await t.test("4. readLastRoute & writeLastRoute: injectable storage, settings is no-op", () => {
    const memory = new Map<string, string>();
    const mockStorage: SimpleStorage = {
      getItem(key: string): string | null {
        return memory.has(key) ? memory.get(key)! : null;
      },
      setItem(key: string, value: string): void {
        memory.set(key, value);
      },
    };

    // 初始读取为空
    assert.strictEqual(readLastRoute(mockStorage), null);

    // 正常写入 4 种页面路由
    writeLastRoute(mockStorage, "workbench");
    assert.strictEqual(memory.get(SHELL_LAST_ROUTE_KEY), "workbench");
    assert.strictEqual(readLastRoute(mockStorage), "workbench");

    writeLastRoute(mockStorage, "history");
    assert.strictEqual(readLastRoute(mockStorage), "history");

    writeLastRoute(mockStorage, "downloads");
    assert.strictEqual(readLastRoute(mockStorage), "downloads");

    writeLastRoute(mockStorage, "agent");
    assert.strictEqual(readLastRoute(mockStorage), "agent");

    // 写入 settings 必须为 no-op，保持原持久化值不变
    writeLastRoute(mockStorage, "settings");
    assert.strictEqual(readLastRoute(mockStorage), "agent");

    // 伪造非法值存储时读取返回 null
    memory.set(SHELL_LAST_ROUTE_KEY, "settings");
    assert.strictEqual(readLastRoute(mockStorage), null);

    memory.set(SHELL_LAST_ROUTE_KEY, "nonsense");
    assert.strictEqual(readLastRoute(mockStorage), null);

    // 容错: 传入 null / undefined storage
    assert.strictEqual(readLastRoute(null), null);
    assert.doesNotThrow(() => writeLastRoute(null, "workbench"));
  });

  // ==========================================================================
  // 5. Source assertions on DesktopTitlebar.tsx
  // ==========================================================================
  await t.test("5. DesktopTitlebar source assertions: removed obsolete elements", () => {
    const titlebarPath = path.join(desktopDir, "DesktopTitlebar.tsx");
    const content = fs.readFileSync(titlebarPath, "utf8");

    // 必须不再包含旧顶栏标签与胶囊文案
    assert.strictEqual(
      content.includes("科学参数工作台"),
      false,
      "DesktopTitlebar.tsx must NOT contain 科学参数工作台"
    );
    assert.strictEqual(
      content.includes("ScenePort Agent 助手"),
      false,
      "DesktopTitlebar.tsx must NOT contain ScenePort Agent 助手"
    );
    assert.strictEqual(
      content.includes("任务中心"),
      false,
      "DesktopTitlebar.tsx must NOT contain 任务中心"
    );

    // 必须不包含 Sun 或 Moon 图标引用（深浅色切换由设置卡负责）
    const lucideImportMatch = content.match(/import\s*\{([^}]+)\}\s*from\s*["']lucide-react["']/);
    const importedIcons = lucideImportMatch ? lucideImportMatch[1] : "";
    assert.strictEqual(
      /\bSun\b/.test(importedIcons),
      false,
      "DesktopTitlebar.tsx must NOT import Sun from lucide-react"
    );
    assert.strictEqual(
      /\bMoon\b/.test(importedIcons),
      false,
      "DesktopTitlebar.tsx must NOT import Moon from lucide-react"
    );
  });
});

test("W5-1b Shell Navigation & Dedicated Downloads Page Specification", async (t) => {
  // ==========================================================================
  // 1. resolveInitialRoute deep link support for downloads
  // ==========================================================================
  await t.test("1. resolveInitialRoute: parses #/downloads directly from hash over stored route", () => {
    assert.strictEqual(resolveInitialRoute("#/downloads", null), "downloads");
    assert.strictEqual(resolveInitialRoute("#/downloads", "agent"), "downloads");
    assert.strictEqual(resolveInitialRoute("#/downloads", "workbench"), "downloads");
  });

  // ==========================================================================
  // 2. DesktopWorkbench.tsx routing wiring & dedicated downloads branch
  // ==========================================================================
  await t.test("2. DesktopWorkbench.tsx: wires routing lifecycle and dedicated downloads page branch", () => {
    const workbenchPath = path.join(desktopDir, "DesktopWorkbench.tsx");
    const content = fs.readFileSync(workbenchPath, "utf8");

    // 状态初始化与生命周期接线验证
    assert.ok(
      content.includes("resolveInitialRoute"),
      "DesktopWorkbench.tsx must use resolveInitialRoute to initialize route"
    );
    assert.ok(
      content.includes("writeLastRoute"),
      "DesktopWorkbench.tsx must call writeLastRoute to persist route changes"
    );
    assert.ok(
      content.includes("hashchange"),
      "DesktopWorkbench.tsx must listen to hashchange events"
    );

    // 消除 W5-1a 临时过渡行为：downloads 不得与 workbench 共享条件分支
    assert.strictEqual(
      content.includes('effectivePageRoute === "workbench" || effectivePageRoute === "downloads"'),
      false,
      "DesktopWorkbench.tsx must not share workbench and downloads in one conditional branch"
    );
    assert.strictEqual(
      content.includes('route === "workbench" || route === "downloads"'),
      false,
      "DesktopWorkbench.tsx must not share workbench and downloads in one route branch"
    );

    // 必须有独立的 downloads 分支渲染下载中心页面
    assert.ok(
      content.includes('effectivePageRoute === "downloads"') || content.includes('route === "downloads"'),
      "DesktopWorkbench.tsx must have an explicit branch for downloads route"
    );
    assert.ok(
      content.includes("<DownloadCenterPage") || content.includes('variant="page"'),
      "DesktopWorkbench.tsx must render dedicated download center page component"
    );
  });

  // ==========================================================================
  // 3. DownloadCenterPage reuses DesktopDownloadDrawer without modal overlay
  // ==========================================================================
  await t.test("3. DownloadCenterPage: reuses drawer logic and does NOT render modal overlay", () => {
    const pagePath = path.join(desktopDir, "DownloadCenterPage.tsx");
    assert.ok(fs.existsSync(pagePath), "DownloadCenterPage.tsx must exist");

    const content = fs.readFileSync(pagePath, "utf8");

    // 复用已有的下载抽屉组件/逻辑
    assert.ok(
      content.includes("DesktopDownloadDrawer"),
      "DownloadCenterPage.tsx must reuse DesktopDownloadDrawer"
    );

    // 页面变体不得渲染角色为 dialog 或 aria-modal 模态遮罩
    assert.strictEqual(
      content.includes("aria-modal"),
      false,
      "DownloadCenterPage.tsx must NOT contain aria-modal"
    );
    assert.strictEqual(
      content.includes('role="dialog"'),
      false,
      "DownloadCenterPage.tsx must NOT contain role=\"dialog\""
    );
  });

  // ==========================================================================
  // 4. DesktopWorkbench settings modal routing transition guard
  // ==========================================================================
  await t.test("4. DesktopWorkbench: transition away from settings route closes modal and preserves legacy DEV init", () => {
    const workbenchPath = path.join(desktopDir, "DesktopWorkbench.tsx");
    const content = fs.readFileSync(workbenchPath, "utf8");

    // 路由从 settings 转移离开时自动关闭弹窗的守卫断言
    assert.ok(
      content.includes("prevRouteRef"),
      "DesktopWorkbench.tsx must use prevRouteRef to track route transitions"
    );
    assert.ok(
      content.includes('prevRouteRef.current === "settings" && route !== "settings"'),
      "DesktopWorkbench.tsx must guard transition away from settings to reset settingsOpen"
    );

    // 保留旧有 DEV hash 快捷方式 (#settings=<cat>) 的初始化与监听
    assert.ok(
      content.includes('window.location.hash.includes("settings")'),
      "DesktopWorkbench.tsx must preserve legacy window.location.hash.includes('settings') in settingsOpen initializer"
    );
    assert.ok(
      content.includes("/#settings=([a-z_]+)/"),
      "DesktopWorkbench.tsx must preserve legacy #settings= regex pattern for category deep-linking"
    );
  });
});
