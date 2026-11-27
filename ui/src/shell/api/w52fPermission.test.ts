import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import {
  permissionModeLabelZh,
  permissionScopeBadge,
  FULL_ACCESS_RISK_ACK_KEY,
  readFullAccessRiskAck,
  writeFullAccessRiskAck,
  needsFullAccessRiskPrompt,
  applyPendingSessionMode,
  popoverPlacement,
  DESKTOP_TITLEBAR_HEIGHT,
  displayedPermissionView,
} from "./agentPermissionLogic.ts";
import type {
  AgentPermissionMode,
  AgentPermissionModeView,
} from "../../lib/generated/v3-contracts.ts";

test("W5-2f 权限模式逻辑与桥接测试套件（W5-2g 改为三档字符串）", async (t) => {
  // ==========================================================================
  // 1. permissionModeLabelZh: 权限模式中文标签
  // ==========================================================================
  await t.test("1. permissionModeLabelZh: 正例：已知三种权限模式返回对应中文标签", () => {
    assert.strictEqual(permissionModeLabelZh("confirm_changes"), "变更前确认");
    assert.strictEqual(permissionModeLabelZh("auto_approve"), "自主审批");
    assert.strictEqual(permissionModeLabelZh("full_access"), "完全访问");
  });

  await t.test("1. permissionModeLabelZh: 反例：未知值、空串、null与undefined一律返回null（不编造）", () => {
    assert.strictEqual(permissionModeLabelZh(null), null);
    assert.strictEqual(permissionModeLabelZh(undefined), null);
    assert.strictEqual(permissionModeLabelZh(""), null);
    assert.strictEqual(permissionModeLabelZh("unknown_mode"), null);
  });

  // ==========================================================================
  // 2. permissionScopeBadge: 作用域徽章说明
  // ==========================================================================
  await t.test("2. permissionScopeBadge: 正例：session_override非null时返回「会话设置」", () => {
    const viewWithOverride: AgentPermissionModeView = {
      effective: "auto_approve",
      session_override: "auto_approve",
      global_default: "confirm_changes",
    };
    assert.strictEqual(permissionScopeBadge(viewWithOverride), "会话设置");
  });

  await t.test("2. permissionScopeBadge: 反例：session_override为null时返回「全局默认」，即使effective为auto_approve也不推算", () => {
    const viewWithoutOverride: AgentPermissionModeView = {
      effective: "auto_approve",
      session_override: null,
      global_default: "auto_approve",
    };
    assert.strictEqual(permissionScopeBadge(viewWithoutOverride), "全局默认");
  });

  // ==========================================================================
  // 6. 源码断言：desktopShellBridgeTauri.ts 必须挂载四个 invoke 命令
  // ==========================================================================
  await t.test("6. 源码断言：desktopShellBridgeTauri.ts 包含四个权限与停止命令的 invoke 调用", () => {
    const __filename = fileURLToPath(import.meta.url);
    const bridgeFile = path.resolve(path.dirname(__filename), "../../lib/desktopShellBridgeTauri.ts");
    const content = fs.readFileSync(bridgeFile, "utf-8");

    assert.ok(
      content.includes('"agent_permission_mode_get"'),
      'desktopShellBridgeTauri.ts 必须包含 "agent_permission_mode_get" invoke'
    );
    assert.ok(
      content.includes('"agent_permission_mode_set_global"'),
      'desktopShellBridgeTauri.ts 必须包含 "agent_permission_mode_set_global" invoke'
    );
    assert.ok(
      content.includes('"agent_permission_mode_set_session"'),
      'desktopShellBridgeTauri.ts 必须包含 "agent_permission_mode_set_session" invoke'
    );
    assert.ok(
      content.includes('"agent_stop_all"'),
      'desktopShellBridgeTauri.ts 必须包含 "agent_stop_all" invoke'
    );
  });

  // ==========================================================================
  // R2 缺陷修复与风险确认测试
  // ==========================================================================
  await t.test("R2-7. readFullAccessRiskAck / writeFullAccessRiskAck: 写入1并正确读取，非1、null或抛错返回false", () => {
    let storedValue: string | null = null;
    const memStorage = {
      getItem: (k: string) => (k === FULL_ACCESS_RISK_ACK_KEY ? storedValue : null),
      setItem: (k: string, v: string) => {
        if (k === FULL_ACCESS_RISK_ACK_KEY) storedValue = v;
      },
    };

    assert.strictEqual(readFullAccessRiskAck(memStorage), false);
    writeFullAccessRiskAck(memStorage);
    assert.strictEqual(storedValue, "1");
    assert.strictEqual(readFullAccessRiskAck(memStorage), true);

    // 异常或非 1
    storedValue = "0";
    assert.strictEqual(readFullAccessRiskAck(memStorage), false);
    const brokenStorage = {
      getItem: () => { throw new Error("storage failure"); },
      setItem: () => { throw new Error("storage failure"); },
    };
    assert.strictEqual(readFullAccessRiskAck(brokenStorage), false);
    // write 不抛错
    assert.doesNotThrow(() => writeFullAccessRiskAck(brokenStorage));
  });

  await t.test("R2-7. needsFullAccessRiskPrompt: 正例与反例：仅 targetKind 为 full_access 且未确认时返回 true", () => {
    assert.strictEqual(needsFullAccessRiskPrompt("full_access", false), true);
    assert.strictEqual(needsFullAccessRiskPrompt("full_access", true), false);
    assert.strictEqual(needsFullAccessRiskPrompt("confirm_changes", false), false);
    assert.strictEqual(needsFullAccessRiskPrompt("auto_approve", false), false);
    assert.strictEqual(needsFullAccessRiskPrompt(null, false), false);
  });

  // ==========================================================================
  // R2b: applyPendingSessionMode 发送前应用待用会话模式纯函数测试
  // ==========================================================================
  await t.test("R2b. applyPendingSessionMode: pending 为 null 时不调用 setSession 且返回 { ok: true }", async () => {
    let called = false;
    const fakeSetSession = async () => {
      called = true;
      throw new Error("should not be called");
    };
    const res = await applyPendingSessionMode(fakeSetSession, "session-1", null);
    assert.strictEqual(called, false);
    assert.strictEqual(res.ok, true);
  });

  await t.test("R2b. applyPendingSessionMode: pending 非 null 时成功调用并返回新视图", async () => {
    let calledWith: { sessionId: string; mode: AgentPermissionMode } | null = null;
    const mockView: AgentPermissionModeView = {
      effective: "auto_approve",
      session_override: "auto_approve",
      global_default: "confirm_changes",
    };
    const fakeSetSession = async (sessionId: string, mode: AgentPermissionMode) => {
      calledWith = { sessionId, mode };
      return mockView;
    };
    const pendingMode: AgentPermissionMode = "auto_approve";
    const res = await applyPendingSessionMode(fakeSetSession, "session-2", pendingMode);
    assert.deepStrictEqual(calledWith, { sessionId: "session-2", mode: pendingMode });
    assert.strictEqual(res.ok, true);
    if (res.ok) {
      assert.deepStrictEqual(res.view, mockView);
    }
  });

  await t.test("R2b. applyPendingSessionMode: setSession 抛错时捕获异常并返回 { ok: false, error }", async () => {
    const errorInst = new Error("backend unavailable");
    const failingSetSession = async () => {
      throw errorInst;
    };
    const pendingMode: AgentPermissionMode = "confirm_changes";
    const res = await applyPendingSessionMode(failingSetSession, "session-3", pendingMode);
    assert.strictEqual(res.ok, false);
    if (!res.ok) {
      assert.strictEqual(res.error, errorInst);
    }
  });

  // ==========================================================================
  // R3a. popoverPlacement 弹层方向与最大高度纯函数测试
  // ==========================================================================
  await t.test("R3a. popoverPlacement: 下方空间充足时选择向下展开", () => {
    // 视口高 800，触发按钮 top: 100, bottom: 132，下方空间 800 - 132 = 668，期望高 300
    const res = popoverPlacement({
      triggerTop: 100,
      triggerBottom: 132,
      viewportHeight: 800,
      desiredHeight: 300,
    });
    assert.strictEqual(res.side, "down");
    assert.strictEqual(res.maxHeight, 660); // 668 - 8
  });

  await t.test("R3a. popoverPlacement: 下方空间不足但上方空间充足时选择向上展开", () => {
    // 视口高 600，触发按钮 top: 450, bottom: 482，下方空间 118 < 300，上方空间 450 >= 300
    const res = popoverPlacement({
      triggerTop: 450,
      triggerBottom: 482,
      viewportHeight: 600,
      desiredHeight: 300,
    });
    assert.strictEqual(res.side, "up");
    assert.strictEqual(res.maxHeight, 442); // 450 - 8
  });

  await t.test("R3a. popoverPlacement: 上下方均不足时取较大一侧且 maxHeight = 较大侧空间 - 8", () => {
    // 视口高 500，期望高 400。
    // 情况 1: 上方较大（top: 300, bottom: 332 -> 上方 300, 下方 168）
    const resUp = popoverPlacement({
      triggerTop: 300,
      triggerBottom: 332,
      viewportHeight: 500,
      desiredHeight: 400,
    });
    assert.strictEqual(resUp.side, "up");
    assert.strictEqual(resUp.maxHeight, 292); // 300 - 8

    // 情况 2: 下方较大（top: 150, bottom: 182 -> 上方 150, 下方 318）
    const resDown = popoverPlacement({
      triggerTop: 150,
      triggerBottom: 182,
      viewportHeight: 500,
      desiredHeight: 400,
    });
    assert.strictEqual(resDown.side, "down");
    assert.strictEqual(resDown.maxHeight, 310); // 318 - 8
  });

  await t.test("R4-a2. popoverPlacement: topInset 扣除标题栏高度，防遮挡标题栏", () => {
    // DESKTOP_TITLEBAR_HEIGHT 常量值验证为 40px
    assert.strictEqual(DESKTOP_TITLEBAR_HEIGHT, 40);

    // 视口高 600，按钮 top: 300, bottom: 332，下方空间 600 - 332 = 268
    // 未设 topInset: 上方空间 300 >= 280 -> 向上展开，maxHeight = 300 - 8 = 292
    const resNoInset = popoverPlacement({
      triggerTop: 300,
      triggerBottom: 332,
      viewportHeight: 600,
      desiredHeight: 280,
    });
    assert.strictEqual(resNoInset.side, "up");
    assert.strictEqual(resNoInset.maxHeight, 292);

    // 传入 topInset = 40 (标题栏高度):
    // 上方空间 = 300 - 40 = 260 < 280 (不够 desiredHeight)
    // 下方空间 = 268 < 280 (也不够 desiredHeight)
    // 比较两者：下方 268 > 上方 260 -> 转向下方展开！maxHeight = 268 - 8 = 260
    const resWithInset = popoverPlacement({
      triggerTop: 300,
      triggerBottom: 332,
      viewportHeight: 600,
      desiredHeight: 280,
      topInset: 40,
    });
    assert.strictEqual(resWithInset.side, "down");
    assert.strictEqual(resWithInset.maxHeight, 260);

    // 当上方即便扣除 topInset 后依然大于下方时，选向上且 maxHeight 扣除 topInset
    // 视口高 500，按钮 top: 320, bottom: 352，下方 148
    // 上方扣除 topInset 40 后为 280 > 148 -> side: "up", maxHeight = 280 - 8 = 272
    const resUpWithInset = popoverPlacement({
      triggerTop: 320,
      triggerBottom: 352,
      viewportHeight: 500,
      desiredHeight: 400,
      topInset: 40,
    });
    assert.strictEqual(resUpWithInset.side, "up");
    assert.strictEqual(resUpWithInset.maxHeight, 272);
  });

  // ==========================================================================
  // R3a / R4-a1. displayedPermissionView 纯函数测试
  // ==========================================================================
  await t.test("R3a. displayedPermissionView: pending 为 null 时原样返回 backendView", () => {
    const backendView: AgentPermissionModeView = {
      effective: "confirm_changes",
      session_override: null,
      global_default: "confirm_changes",
    };
    const res = displayedPermissionView(backendView, null);
    assert.deepStrictEqual(res, backendView);
    assert.strictEqual(displayedPermissionView(null, null), null);
  });

  await t.test("R4-a1. displayedPermissionView: backendView 为 null 时直接返回 null，绝不编造兜底", () => {
    const pending: AgentPermissionMode = "auto_approve";
    // 无论 pending 是否有值，backendView 为 null/undefined 时必须返回 null
    assert.strictEqual(displayedPermissionView(null, pending), null);
    assert.strictEqual(displayedPermissionView(undefined, pending), null);
  });

  await t.test("R3a. displayedPermissionView: pending 非 null 时覆盖 effective 与 session_override", () => {
    const backendView: AgentPermissionModeView = {
      effective: "confirm_changes",
      session_override: null,
      global_default: "confirm_changes",
    };
    const pending: AgentPermissionMode = "auto_approve";
    const res = displayedPermissionView(backendView, pending);
    assert.deepStrictEqual(res, {
      effective: pending,
      session_override: pending,
      global_default: "confirm_changes",
    });
  });
});
