import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

import {
  resolvePanelOpen,
  resolvePanelMode,
  resolvePanelTab,
  panelWidth,
} from "./agentPanelLogic.ts";

test("W5-2c Agent Right-Side Panel Specification", async (t) => {
  // ==========================================================================
  // 1. resolvePanelOpen: default closed, open intents, overlay persistence
  // ==========================================================================
  await t.test("1. resolvePanelOpen: default closed, intent override, responsive persistence", () => {
    // 默认关闭
    assert.strictEqual(
      resolvePanelOpen({ requested: false, viewportWidth: 1920, openIntent: null }),
      false,
      "panel must default to closed when not requested and no intent"
    );

    // 用户请求打开
    assert.strictEqual(
      resolvePanelOpen({ requested: true, viewportWidth: 1920, openIntent: null }),
      true,
      "panel opens when requested is true"
    );

    // 地图查看意图强制打开（即使用户请求为 false）
    assert.strictEqual(
      resolvePanelOpen({ requested: false, viewportWidth: 1920, openIntent: "map-view" }),
      true,
      "map-view intent forces panel open"
    );

    // 绘制矩形意图强制打开（即使用户请求为 false）
    assert.strictEqual(
      resolvePanelOpen({ requested: false, viewportWidth: 1920, openIntent: "rect-draw" }),
      true,
      "rect-draw intent forces panel open"
    );

    // 小于 1280 宽度下，满足打开条件时依然返回 true（模式由 resolvePanelMode 处理为 overlay）
    assert.strictEqual(
      resolvePanelOpen({ requested: true, viewportWidth: 1024, openIntent: null }),
      true,
      "panel can remain open under 1280px in overlay mode"
    );
    assert.strictEqual(
      resolvePanelOpen({ requested: false, viewportWidth: 920, openIntent: "map-view" }),
      true,
      "intent forces panel open on narrow viewport"
    );
  });

  // ==========================================================================
  // 2. resolvePanelMode: 1280px boundary arbitration
  // ==========================================================================
  await t.test("2. resolvePanelMode: docked >= 1280px, overlay < 1280px", () => {
    assert.strictEqual(resolvePanelMode(1920), "docked");
    assert.strictEqual(resolvePanelMode(1440), "docked");
    assert.strictEqual(resolvePanelMode(1280), "docked", "1280px exactly is docked");
    assert.strictEqual(resolvePanelMode(1279), "overlay", "1279px falls below boundary to overlay");
    assert.strictEqual(resolvePanelMode(1024), "overlay");
    assert.strictEqual(resolvePanelMode(920), "overlay");
    assert.strictEqual(resolvePanelMode(600), "overlay");
  });

  // ==========================================================================
  // 3. resolvePanelTab: guards tabs to map | results | task
  // ==========================================================================
  await t.test("3. resolvePanelTab: guards valid tabs and falls back to map", () => {
    assert.strictEqual(resolvePanelTab("map"), "map");
    assert.strictEqual(resolvePanelTab("results"), "results");
    assert.strictEqual(resolvePanelTab("task"), "task");

    assert.strictEqual(resolvePanelTab("invalid"), "map");
    assert.strictEqual(resolvePanelTab(null), "map");
    assert.strictEqual(resolvePanelTab(undefined), "map");
    assert.strictEqual(resolvePanelTab(123), "map");
  });

  // ==========================================================================
  // 4. panelWidth: docked 42% capped at 760px, overlay returns null
  // ==========================================================================
  await t.test("4. panelWidth: min(round(w * 0.42), 760) when >= 1280, null below 1280", () => {
    // 1920: 1920 * 0.42 = 806.4 -> round 806 -> min(806, 760) = 760
    assert.strictEqual(panelWidth(1920), 760);

    // 1440: 1440 * 0.42 = 604.8 -> round 605 -> min(605, 760) = 605
    assert.strictEqual(panelWidth(1440), 605);

    // 1280: 1280 * 0.42 = 537.6 -> round 538 -> min(538, 760) = 538
    assert.strictEqual(panelWidth(1280), 538);

    // < 1280: 由 CSS / overlay 机制控制，函数返回 null
    assert.strictEqual(panelWidth(1279), null);
    assert.strictEqual(panelWidth(1024), null);
    assert.strictEqual(panelWidth(920), null);
  });

  // ==========================================================================
  // 5. DesktopAgentWorkbench.tsx 源码断言：必须调用 resolvePanelOpen 裁决面板开合
  // ==========================================================================
  await t.test("5. DesktopAgentWorkbench.tsx 源码断言：必须调用 resolvePanelOpen 裁决面板开合", () => {
    const workbenchPath = path.resolve(__dirname, "../../components/desktop/DesktopAgentWorkbench.tsx");
    const content = fs.readFileSync(workbenchPath, "utf-8");
    assert.match(
      content,
      /resolvePanelOpen\(\s*\{/,
      "DesktopAgentWorkbench.tsx must invoke resolvePanelOpen with parameters"
    );
  });
});
