import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { permissionScopeBadge } from "./agentPermissionLogic.ts";
import type {
  AgentPermissionMode,
  AgentPermissionModeView,
} from "../../lib/generated/v3-contracts.ts";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

test("W5-6 R5 (C): 待应用权限模式徽章纯逻辑与透传接线测试", async (t) => {
  const viewOverride: AgentPermissionModeView = {
    effective: "auto_approve",
    session_override: "auto_approve",
    global_default: "confirm_changes",
  };
  const viewGlobal: AgentPermissionModeView = {
    effective: "confirm_changes",
    session_override: null,
    global_default: "confirm_changes",
  };

  await t.test("1. pendingSessionMode 非 null 时返回「将在发送时应用」", () => {
    const callBadge = permissionScopeBadge as (
      v: AgentPermissionModeView,
      pending?: AgentPermissionMode | null
    ) => string;
    assert.strictEqual(callBadge(viewOverride, "auto_approve"), "将在发送时应用");
    assert.strictEqual(callBadge(viewGlobal, "full_access"), "将在发送时应用");
  });

  await t.test("2. pendingSessionMode 为 null 或缺省时保持「会话设置」/「全局默认」现状", () => {
    const callBadge = permissionScopeBadge as (
      v: AgentPermissionModeView,
      pending?: AgentPermissionMode | null
    ) => string;
    assert.strictEqual(callBadge(viewOverride, null), "会话设置");
    assert.strictEqual(callBadge(viewOverride), "会话设置");
    assert.strictEqual(callBadge(viewGlobal, null), "全局默认");
    assert.strictEqual(callBadge(viewGlobal), "全局默认");
  });

  await t.test("3. DesktopAgentWorkbench 两处 AgentComposer 传入 permissionPending={pendingSessionMode !== null}", () => {
    const wbPath = path.resolve(
      __dirname,
      "../../components/desktop/DesktopAgentWorkbench.tsx"
    );
    const src = fs.readFileSync(wbPath, "utf-8");
    const matches = src.match(/permissionPending=\{pendingSessionMode !== null\}/g) || [];
    assert.strictEqual(
      matches.length,
      2,
      "DesktopAgentWorkbench.tsx 两处 AgentComposer 必须传 permissionPending={pendingSessionMode !== null}"
    );
  });
});
