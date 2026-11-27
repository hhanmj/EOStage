import test from "node:test";
import assert from "node:assert/strict";
import { clampPanelWidth } from "./agentPanelLogic.ts";

test("agentPanelLogic clampPanelWidth Unit Tests", async (t) => {
  await t.test("1. 边界限制：面板宽度不小于 360px", () => {
    // 正常视口下，期望宽度过小直接 clamp 到 360
    assert.strictEqual(clampPanelWidth(200, 1920, 248), 360);
    assert.strictEqual(clampPanelWidth(0, 1920, 0), 360);
    assert.strictEqual(clampPanelWidth(-100, 1440, 52), 360);
    assert.strictEqual(clampPanelWidth(359, 1920, 0), 360);
    assert.strictEqual(clampPanelWidth(360, 1920, 0), 360);
  });

  // Owner 2026-10-29: 对话列最小宽度调整为 420px (maxPanel = available - 420)
  await t.test("2. 边界限制：对话列宽度不小于 420px (maxPanel = available - 420)", () => {
    // viewportWidth = 1920, sidebarWidth = 248 -> available = 1672 -> maxPanel = 1252
    assert.strictEqual(clampPanelWidth(1500, 1920, 248), 1252);
    assert.strictEqual(clampPanelWidth(1252, 1920, 248), 1252);
    assert.strictEqual(clampPanelWidth(1251, 1920, 248), 1251);

    // viewportWidth = 1280, sidebarWidth = 248 -> available = 1032 -> maxPanel = 612
    assert.strictEqual(clampPanelWidth(800, 1280, 248), 612);
    assert.strictEqual(clampPanelWidth(612, 1280, 248), 612);

    // viewportWidth = 1280, sidebarWidth = 0 / undefined -> available = 1280 -> maxPanel = 860
    assert.strictEqual(clampPanelWidth(1000, 1280), 860);
    assert.strictEqual(clampPanelWidth(1000, 1280, 0), 860);
    assert.strictEqual(clampPanelWidth(860, 1280), 860);
  });

  await t.test("3. 正常区间：返回整数四舍五入值", () => {
    assert.strictEqual(clampPanelWidth(500, 1920, 248), 500);
    assert.strictEqual(clampPanelWidth(600.4, 1920, 248), 600);
    assert.strictEqual(clampPanelWidth(600.6, 1920, 248), 601);
  });

  await t.test("4. 极端紧凑视口：保证不低于 360px", () => {
    // available = 700 -> available - 400 = 300 < 360 -> maxPanel clamped to 360
    assert.strictEqual(clampPanelWidth(500, 700, 0), 360);
  });

  await t.test("5. 非正常数值防守：NaN 或非有限值回退为 360px", () => {
    assert.strictEqual(clampPanelWidth(Number.NaN, 1920, 248), 360);
    assert.strictEqual(clampPanelWidth(Number.POSITIVE_INFINITY, 1920, 248), 360);
  });
});
