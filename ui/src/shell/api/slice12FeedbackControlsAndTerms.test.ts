import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import {
  getSettingsBubbleEntries,
  type SettingsBubbleEntry,
} from "./settingsEntries.ts";
import {
  formatOrbitDirection,
  formatOrbitBadge,
  formatOrbitFrameLine,
} from "./rowProjection.ts";
import type { SettingsCategory } from "../../components/SettingsModal";
import { assertLocalizedSource } from "./i18nSourceTestUtils.test.ts";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const uiSrcDir = path.resolve(__dirname, "../..");
const desktopDir = path.resolve(uiSrcDir, "components/desktop");

test("W4 Slice 12-5a - Owner Feedback (a) Window Controls, (b) Unified Settings Bubble, (c) English Orbit Terms", async (t) => {
  // ==========================================================================
  // (a) 移除 macOS 交通灯，右上角添加 Windows 窗口控制按钮
  // ==========================================================================
  await t.test("1. (a) 窗口控制：DesktopTitlebar 不再包含 macOS 红黄绿交通灯类名与三圆点结构", () => {
    const titlebarPath = path.join(desktopDir, "DesktopTitlebar.tsx");
    const content = fs.readFileSync(titlebarPath, "utf8");

    // 不包含 macOS 彩色三圆点色值类名
    assert.strictEqual(
      content.includes("bg-sp-win-close"),
      false,
      "DesktopTitlebar.tsx must not contain bg-sp-win-close"
    );
    assert.strictEqual(
      content.includes("bg-sp-win-min"),
      false,
      "DesktopTitlebar.tsx must not contain bg-sp-win-min"
    );
    assert.strictEqual(
      content.includes("bg-sp-win-max"),
      false,
      "DesktopTitlebar.tsx must not contain bg-sp-win-max"
    );

    // 不包含左侧 macOS 风格圆点胶囊结构
    assert.strictEqual(
      content.includes("macOS 风格窗口胶囊"),
      false,
      "DesktopTitlebar.tsx must remove macOS window control capsule"
    );
  });

  await t.test("2. (a) 窗口控制：DesktopTitlebar 右上角贴缘提供 Windows 风格窗口控制按钮 (Minus/Square/X)", () => {
    const titlebarPath = path.join(desktopDir, "DesktopTitlebar.tsx");
    const content = fs.readFileSync(titlebarPath, "utf8");

    // 检查线条图标引用
    assert.ok(
      /\bMinus\b/.test(content),
      "DesktopTitlebar.tsx must import and use Minus icon from lucide-react"
    );
    assert.ok(
      /\bSquare\b/.test(content),
      "DesktopTitlebar.tsx must import and use Square icon from lucide-react"
    );
    assert.ok(
      /\bX\b/.test(content),
      "DesktopTitlebar.tsx must import and use X icon from lucide-react"
    );

    // 检查对应 aria-label 或 title
    assertLocalizedSource(content, "shell.titlebar.minimize", "最小化");
    assertLocalizedSource(content, "shell.titlebar.maximizeRestore", "最大化 / 还原");
    assertLocalizedSource(content, "shell.titlebar.closeWindow", "关闭窗口");

    // 检查右上窗口控制区域具有 no-drag 属性
    assert.ok(
      content.includes('WebkitAppRegion: "no-drag"'),
      "Window controls must have no-drag style"
    );
  });

  await t.test("3. (a)/(b) 移除右上角设置齿轮按钮与 onOpenSettings prop", () => {
    const titlebarPath = path.join(desktopDir, "DesktopTitlebar.tsx");
    const content = fs.readFileSync(titlebarPath, "utf8");

    // 不再接收 onOpenSettings
    assert.strictEqual(
      content.includes("onOpenSettings"),
      false,
      "DesktopTitlebar.tsx must not have onOpenSettings prop"
    );
    // 不再包含右上设置齿轮
    assert.strictEqual(
      content.includes("打开偏好设置"),
      false,
      "DesktopTitlebar.tsx must remove settings gear button"
    );
  });

  // ==========================================================================
  // (b) 设置入口唯一化：左下直达设置页（Owner 2026-10-29: 应用没有用户账号，左下角改为直达设置页，外观与字号移入设置）
  // ==========================================================================
  // Owner 2026-10-29: 应用没有用户账号，左下角改为直达设置页，外观与字号移入设置
  await t.test("4. (b) 触发器无障碍：DesktopActivityRail 底部设置触发器为直达设置按钮", () => {
    const railPath = path.join(desktopDir, "DesktopActivityRail.tsx");
    const content = fs.readFileSync(railPath, "utf8");

    assertLocalizedSource(content, "shell.nav.settings", "设置");
  });

  // Owner 2026-10-29: 应用没有用户账号，左下角改为直达设置页，外观与字号移入设置
  await t.test("5. (b) 纯函数清单：getSettingsBubbleEntries 恰 6 行，6 个真实分类，包含外观，无语言", () => {
    const entries = getSettingsBubbleEntries();

    assert.strictEqual(entries.length, 6, "Settings entries must have exactly 6 rows");

    // 6 个弹窗分类
    const modalCategories = entries
      .filter((e) => e.kind === "modal")
      .map((e) => e.category);
    assert.strictEqual(modalCategories.length, 6, "Must have exactly 6 modal categories");

    const validCategories: readonly SettingsCategory[] = [
      "appearance",
      "auth",
      "network_proxy",
      "models",
      "storage",
      "network",
    ];

    for (const cat of modalCategories) {
      assert.ok(
        cat && validCategories.includes(cat),
        `Category ${cat} must be within SettingsCategory valid values`
      );
    }

    // 无语言条目（死控件禁止）
    const languageEntry = entries.find(
      (e) => e.label.includes("语言") || e.key.includes("language") || e.key.includes("locale")
    );
    assert.strictEqual(languageEntry, undefined, "Must NOT have any language entry");

    // 外观条目为 appearance 弹窗分类
    const appearanceEntry = entries.find((e) => e.label === "外观" || e.key === "appearance");
    assert.ok(appearanceEntry, "Must contain appearance entry");
    assert.strictEqual(appearanceEntry?.kind, "modal", "Appearance entry kind must be 'modal'");
    assert.strictEqual(appearanceEntry?.category, "appearance", "Appearance entry category must be 'appearance'");
  });

  // ==========================================================================
  // (c) 领域术语英文化
  // ==========================================================================
  await t.test("6. (c) 纯函数：formatOrbitDirection 正确映射完整英文方向", () => {
    assert.strictEqual(formatOrbitDirection("ASCENDING"), "Ascending");
    assert.strictEqual(formatOrbitDirection("ascending"), "Ascending");
    assert.strictEqual(formatOrbitDirection("DESCENDING"), "Descending");
    assert.strictEqual(formatOrbitDirection("descending"), "Descending");
    assert.strictEqual(formatOrbitDirection(null), "Unknown");
    assert.strictEqual(formatOrbitDirection(undefined), "Unknown");
    assert.strictEqual(formatOrbitDirection(""), "Unknown");
    assert.strictEqual(formatOrbitDirection("other"), "other");
  });

  await t.test("7. (c) 纯函数：formatOrbitBadge 正确映射紧凑英文徽章", () => {
    assert.strictEqual(formatOrbitBadge("ASCENDING"), "ASC");
    assert.strictEqual(formatOrbitBadge("ascending"), "ASC");
    assert.strictEqual(formatOrbitBadge("DESCENDING"), "DESC");
    assert.strictEqual(formatOrbitBadge("descending"), "DESC");
    assert.strictEqual(formatOrbitBadge(null), "Unknown");
    assert.strictEqual(formatOrbitBadge(undefined), "Unknown");
    assert.strictEqual(formatOrbitBadge(""), "Unknown");
  });

  await t.test("8. (c) 纯函数：formatOrbitFrameLine 正确组合轨道与分幅信息", () => {
    // 相对轨道 + 分幅
    assert.strictEqual(
      formatOrbitFrameLine("ascending", 11, null, 65),
      "Ascending · Path 11 · Frame 65"
    );
    // Path + 分幅
    assert.strictEqual(
      formatOrbitFrameLine("descending", null, 42, 100),
      "Descending · Path 42 · Frame 100"
    );
    // 缺失全未知
    assert.strictEqual(
      formatOrbitFrameLine(null, null, null, null),
      "Unknown · Path Unknown · Frame Unknown"
    );
  });

  await t.test("9. (c) 源码断言：ManualWorkbench.tsx 不再包含中文领域术语 (升轨/降轨/相对轨道/分幅编号)", () => {
    const dockPath = path.join(desktopDir, "ManualWorkbench.tsx");
    const content = fs.readFileSync(dockPath, "utf8");

    // 排除纯代码注释后的检查，但组件内即使注释也应避免这些旧词
    assert.strictEqual(
      content.includes("升轨"),
      false,
      "ManualWorkbench.tsx must not contain '升轨'"
    );
    assert.strictEqual(
      content.includes("降轨"),
      false,
      "ManualWorkbench.tsx must not contain '降轨'"
    );
    assert.strictEqual(
      content.includes("相对轨道"),
      false,
      "ManualWorkbench.tsx must not contain '相对轨道'"
    );
    assert.strictEqual(
      content.includes("分幅编号"),
      false,
      "ManualWorkbench.tsx must not contain '分幅编号'"
    );
  });

  await t.test("10. (c) 源码断言：ManualConditionsPanel.tsx 不再包含中文领域术语 (升轨/降轨/相对轨道/分幅编号)", () => {
    const panelPath = path.join(desktopDir, "ManualConditionsPanel.tsx");
    const content = fs.readFileSync(panelPath, "utf8");

    assert.strictEqual(
      content.includes("升轨"),
      false,
      "ManualConditionsPanel.tsx must not contain '升轨'"
    );
    assert.strictEqual(
      content.includes("降轨"),
      false,
      "ManualConditionsPanel.tsx must not contain '降轨'"
    );
    assert.strictEqual(
      content.includes("相对轨道"),
      false,
      "ManualConditionsPanel.tsx must not contain '相对轨道'"
    );
    assert.strictEqual(
      content.includes("分幅编号"),
      false,
      "ManualConditionsPanel.tsx must not contain '分幅编号'"
    );
  });
});
