import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  mapPanelMode,
  showMapButton,
  nextOverlayStateOnEscape,
  type MapPanelMode,
} from "./mapPanelLogic.ts";
import { assertLocalizedSource } from "./i18nSourceTestUtils.test.ts";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

test("W5-3d: Map Panel Logic & Layout Suite", async (t) => {
  // 1. mapPanelMode 表格测试
  await t.test("1. mapPanelMode maps viewport width, userCollapsed, and overlayOpen to correct modes", () => {
    const cases: Array<{
      width: number;
      collapsed: boolean;
      overlay: boolean;
      expected: MapPanelMode;
      label: string;
    }> = [
      // 宽屏 (>= 1600)
      { width: 1920, collapsed: false, overlay: false, expected: "docked-wide", label: "1920px 正常模式" },
      { width: 1600, collapsed: false, overlay: false, expected: "docked-wide", label: "1600px 临界正常模式" },
      { width: 1920, collapsed: true, overlay: false, expected: "collapsed", label: "1920px 用户折叠" },
      { width: 1600, collapsed: true, overlay: false, expected: "collapsed", label: "1600px 用户折叠" },

      // 中屏 (1280 - 1599)
      { width: 1599, collapsed: false, overlay: false, expected: "docked", label: "1599px docked 模式" },
      { width: 1440, collapsed: false, overlay: false, expected: "docked", label: "1440px docked 模式" },
      { width: 1280, collapsed: false, overlay: false, expected: "docked", label: "1280px 临界 docked 模式" },
      { width: 1440, collapsed: true, overlay: false, expected: "collapsed", label: "1440px 用户折叠" },
      { width: 1280, collapsed: true, overlay: false, expected: "collapsed", label: "1280px 用户折叠" },

      // 窄屏 (< 1280)
      { width: 1279, collapsed: false, overlay: true, expected: "overlay", label: "1279px 浮层打开" },
      { width: 1024, collapsed: false, overlay: true, expected: "overlay", label: "1024px 浮层打开" },
      { width: 800, collapsed: false, overlay: true, expected: "overlay", label: "800px 浮层打开" },
      { width: 1279, collapsed: false, overlay: false, expected: "hidden", label: "1279px 浮层关闭（隐藏）" },
      { width: 1024, collapsed: false, overlay: false, expected: "hidden", label: "1024px 浮层关闭（隐藏）" },
      { width: 800, collapsed: true, overlay: false, expected: "hidden", label: "800px 关闭（隐藏）" },
    ];

    for (const c of cases) {
      const actual = mapPanelMode(c.width, c.collapsed, c.overlay);
      assert.equal(
        actual,
        c.expected,
        `${c.label} (width: ${c.width}, collapsed: ${c.collapsed}, overlay: ${c.overlay}) 期望 ${c.expected}，实得 ${actual}`
      );
    }
  });

  // 2. showMapButton
  await t.test("2. showMapButton returns true only for collapsed and hidden modes", () => {
    assert.equal(showMapButton("collapsed"), true, "collapsed 模式必须展示地图按钮");
    assert.equal(showMapButton("hidden"), true, "hidden 模式必须展示地图按钮");
    assert.equal(showMapButton("docked-wide"), false, "docked-wide 模式不可展示地图按钮");
    assert.equal(showMapButton("docked"), false, "docked 模式不可展示地图按钮");
    assert.equal(showMapButton("overlay"), false, "overlay 模式不可展示地图按钮");
  });

  // 3. nextOverlayStateOnEscape
  await t.test("3. nextOverlayStateOnEscape always returns false", () => {
    assert.equal(nextOverlayStateOnEscape(true), false);
    assert.equal(nextOverlayStateOnEscape(false), false);
  });

  // 4. Source Assertions
  await t.test("4. Source Assertions: WorkbenchMap opt-in chrome prop and ManualWorkbench usage", () => {
    const workbenchMapPath = path.resolve(__dirname, "../../components/WorkbenchMap.tsx");
    const workbenchMapSource = fs.readFileSync(workbenchMapPath, "utf-8");

    // WorkbenchMap 必须具有 chrome 可选 prop，默认是 "default"
    assert.match(
      workbenchMapSource,
      /chrome\s*:\s*["']default["']\s*\|\s*["']panel["']|chrome\s*\?:\s*["']default["']\s*\|\s*["']panel["']/,
      "WorkbenchMap 必须声明 chrome?: 'default' | 'panel' 属性"
    );
    assert.match(
      workbenchMapSource,
      /chrome\s*=\s*["']default["']/,
      "WorkbenchMap chrome 参数默认值必须为 'default'"
    );

    // 默认分支必须原样保留原有控件与标记（防止影响 legacy 页面及现有单测）
    assert.match(
      workbenchMapSource,
      /<MapToolbar/,
      "WorkbenchMap 源码中必须保留原版 <MapToolbar 渲染调用"
    );

    // ManualWorkbench 必须向 WorkbenchMap 传入 chrome="panel"
    const manualWorkbenchPath = path.resolve(__dirname, "../../components/desktop/ManualWorkbench.tsx");
    const manualWorkbenchSource = fs.readFileSync(manualWorkbenchPath, "utf-8");

    assert.match(
      manualWorkbenchSource,
      /<WorkbenchMap[\s\S]*?chrome=["']panel["']/,
      "ManualWorkbench 必须显式传入 chrome='panel'"
    );

    // ManualWorkbench 必须包含地图标题与展开地图把手
    assert.match(
      manualWorkbenchSource,
      /地图/,
      "ManualWorkbench 必须包含地图面板标题"
    );
  });

  // 5. Defect Guards: no map top-right collapse button, true AOI legend only, no bottom-left overlap
  await t.test("5. Defect Guards: no map top-right collapse button, true AOI legend only, no bottom-left overlap", () => {
    const workbenchMapPath = path.resolve(__dirname, "../../components/WorkbenchMap.tsx");
    const workbenchMapSource = fs.readFileSync(workbenchMapPath, "utf-8");
    const manualWorkbenchPath = path.resolve(__dirname, "../../components/desktop/ManualWorkbench.tsx");
    const manualWorkbenchSource = fs.readFileSync(manualWorkbenchPath, "utf-8");

    // 1. 地图右上角不设收纳标志（仅保留图例与快视图开关）
    assert.doesNotMatch(
      manualWorkbenchSource,
      /topControls=\{[\s\S]*?aria-label=\{t\("mapWorkbench\.map\.collapse"\)\}/,
      "地图框右上角不应显示收纳标志",
    );

    // 2. Legend: ManualWorkbench 必须传 aoiName={aoiLabel}
    assert.match(
      manualWorkbenchSource,
      /<WorkbenchMap[\s\S]*?aoiName=\{aoiLabel\}/,
      "ManualWorkbench 必须向 WorkbenchMap 传入真实的 aoiName={aoiLabel}"
    );
    // WorkbenchMap 在 panel 模式下必须依据真实 AOI (aoiBbox || aoiGeometry) 条件渲染
    assert.match(
      workbenchMapSource,
      /aoiBbox\s*\|\|\s*aoiGeometry/,
      "WorkbenchMap 必须依据 aoiBbox || aoiGeometry 判定真实 AOI 存在，绝不虚构 AOI 图例"
    );

    // 3. Bottom-left coordinate capsule enabled in declutter mode, suppressed only in non-declutter panel mode:
    assert.match(
      workbenchMapSource,
      /if\s*\(\s*chrome\s*===\s*["']panel["'](?:\s*&&\s*!declutter)?\s*\)\s*return\s+null/,
      "MapStatusOverlay 在非 declutter 的 chrome === 'panel' 时返回 null，在 declutter 地图工作台保留左下角经纬度胶囊"
    );
    // MapProviderStatusOverlay 在 panel 模式下 ready 时不渲染，且定位在 top-left
    assert.match(
      workbenchMapSource,
      /chrome\s*===\s*["']panel["'][\s\S]*?top-3[\s\S]*?left-3/,
      "MapProviderStatusOverlay 在 panel 模式下必须定位在 top-left (top-3 left-3)"
    );
  });
});
