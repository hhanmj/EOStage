import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

// 命名空间导入：缺失的导出在断言里失败，而不是在模块链接时失败（RED 必须是行为断言）
import * as perm from "./agentPermissionLogic.ts";
import * as approval from "./approvalLogic.ts";
import { classifyStreamEvents } from "./agentStreamLogic.ts";
import { MESSAGES } from "../../i18n/locales.ts";
import { assertLocalizedSource } from "./i18nSourceTestUtils.test.ts";

const here = path.dirname(fileURLToPath(import.meta.url));

test("W5-2g 权限三档与确认卡原因（纯逻辑）", async (t) => {
  await t.test("1. 三档中文名：confirm_changes / auto_approve / full_access", () => {
    assert.equal(perm.permissionModeLabelZh("confirm_changes"), "变更前确认");
    assert.equal(perm.permissionModeLabelZh("auto_approve"), "自主审批");
    assert.equal(perm.permissionModeLabelZh("full_access"), "完全访问");
  });

  await t.test("2. W3e 旧值与未知值不再有中文名（选择器不编造）", () => {
    assert.equal(perm.permissionModeLabelZh("per_item"), null);
    assert.equal(perm.permissionModeLabelZh("within_quota"), null);
    assert.equal(perm.permissionModeLabelZh(""), null);
    assert.equal(perm.permissionModeLabelZh(null), null);
  });

  await t.test("3. 选择器三项与说明（简报 2g-1 原文），顺序固定", () => {
    const rows = (perm as Record<string, unknown>).PERMISSION_MODE_ROWS as
      | ReadonlyArray<{ mode: string; labelKey: string; descriptionKey: string }>
      | undefined;
    assert.ok(Array.isArray(rows), "PERMISSION_MODE_ROWS 必须导出");
    const source = fs.readFileSync(path.resolve(here, "agentPermissionLogic.ts"), "utf8");
    const permissionMessages = MESSAGES["zh-CN"].agent.permission;
    assert.deepEqual(
      rows!.map((r) => [r.mode, r.labelKey, permissionMessages[r.labelKey.replace("agent.permission.", "") as keyof typeof permissionMessages]]),
      [
        ["confirm_changes", "agent.permission.confirmChanges", "变更前确认"],
        ["auto_approve", "agent.permission.autoApprove", "自主审批"],
        ["full_access", "agent.permission.fullAccess", "完全访问"],
      ],
    );
    for (const row of rows!) {
      assertLocalizedSource(source, row.labelKey, permissionMessages[row.labelKey.replace("agent.permission.", "") as keyof typeof permissionMessages]);
      assertLocalizedSource(source, row.descriptionKey, permissionMessages[row.descriptionKey.replace("agent.permission.", "") as keyof typeof permissionMessages]);
    }
    assert.equal(permissionMessages.confirmChangesDescription, "创建或修改任务前需经确认");
    assert.equal(permissionMessages.autoApproveDescription, "信息完整时自动执行");
    assert.equal(permissionMessages.fullAccessDescription, "全部自动执行");
    for (const description of [permissionMessages.confirmChangesDescription, permissionMessages.autoApproveDescription, permissionMessages.fullAccessDescription]) {
      assert.ok(!description.includes("额度"), "说明里不得出现额度");
    }
  });

  await t.test("4. 线格式只是三个字符串之一；旧值、对象与未知值返回 null", () => {
    assert.equal(perm.permissionModeWire("confirm_changes"), "confirm_changes");
    assert.equal(perm.permissionModeWire("auto_approve"), "auto_approve");
    assert.equal(perm.permissionModeWire("full_access"), "full_access");
    assert.equal(perm.permissionModeWire("per_item"), null);
    assert.equal(perm.permissionModeWire("within_quota"), null);
    assert.equal(perm.permissionModeWire({ kind: "full_access" } as unknown as string), null);
  });

  await t.test("5. 额度表单相关导出已删除", () => {
    const exported = Object.keys(perm);
    for (const name of [
      "WITHIN_QUOTA_DEFAULT_MAX_BYTES",
      "WITHIN_QUOTA_DEFAULT_MAX_TASKS",
      "quotaDraftFrom",
      "validateQuotaDraft",
    ]) {
      assert.ok(!exported.includes(name), `${name} 应删除`);
    }
  });

  await t.test("6. 风险提示只在切到 full_access 且未提示过时出现", () => {
    assert.equal(perm.needsFullAccessRiskPrompt("full_access", false), true);
    assert.equal(perm.needsFullAccessRiskPrompt("full_access", true), false);
    assert.equal(perm.needsFullAccessRiskPrompt("auto_approve", false), false);
    assert.equal(perm.needsFullAccessRiskPrompt("confirm_changes", false), false);
  });

  await t.test("7. 待用会话模式覆盖显示视图（字符串枚举）", () => {
    const view = { effective: "confirm_changes", session_override: null, global_default: "confirm_changes" } as const;
    assert.deepEqual(perm.displayedPermissionView(view as never, "auto_approve" as never), {
      effective: "auto_approve",
      session_override: "auto_approve",
      global_default: "confirm_changes",
    });
  });

  await t.test("8. card_reasons 十值中文（简报三条 + 其余七条）", () => {
    const map = (approval as Record<string, unknown>).CARD_REASON_ZH as Record<string, string> | undefined;
    assert.ok(map, "CARD_REASON_ZH 必须导出");
    assert.equal(map!.output_dir_unverified, "模型建议的目录需要你确认");
    assert.equal(map!.ledger_edit_requires_confirm, "修改任务记录需要确认");
    assert.equal(map!.tool_requires_confirm, "该操作需要确认");
    assert.deepEqual(Object.keys(map!).sort(), [
      "account_not_ready",
      "archive_requires_confirm",
      "ledger_edit_requires_confirm",
      "output_dir_unknown",
      "output_dir_unverified",
      "reopens_existing_task",
      "selection_not_from_run",
      "size_unknown",
      "targets_user_task",
      "tool_requires_confirm",
    ]);
    for (const zh of Object.values(map!)) {
      assert.ok(!/[a-z]_[a-z]/.test(zh), `中文说明不得含实现术语：${zh}`);
      assert.ok(!zh.includes("额度"), `不得出现额度：${zh}`);
    }
  });

  await t.test("9. projectApproval：manual + card_reasons 按卡片顺序列中文原因，未知原因丢弃", () => {
    const p = approval.projectApproval(
      {
        decision: "manual",
        mode: "auto_approve",
        card_reasons: ["output_dir_unverified", "bogus_reason", "targets_user_task"],
      } as never,
      false,
    ) as unknown as { kind: string; modeLabel: string | null; reasonsZh?: string[] };
    assert.equal(p.kind, "reasons");
    assert.equal(p.modeLabel, "自主审批");
    assert.deepEqual(p.reasonsZh, ["模型建议的目录需要你确认", "涉及你自己建的任务，需要确认"]);
  });

  await t.test("10. 自动执行与禁止档投影沿用", () => {
    const auto = approval.projectApproval({ decision: "auto_by_mode", mode: "full_access", card_reasons: [] } as never, true);
    assert.equal(auto.kind, "auto");
    assert.equal(auto.modeLabel, "完全访问");
    const forbidden = approval.projectApproval({ decision: "refused_forbidden", mode: "confirm_changes" } as never, false);
    assert.equal(forbidden.kind, "forbidden");
    assert.equal(forbidden.modeLabel, "变更前确认");
    const plain = approval.projectApproval({ decision: "manual", mode: "confirm_changes", card_reasons: [] } as never, false);
    assert.equal(plain.kind, "manual");
  });

  await t.test("11. 旧事件（W3e）：mode 为 per_item / within_quota 显示「未知模式」，exceeded 忽略，不抛错", () => {
    const legacyFallback = approval.projectApproval(
      { decision: "manual", mode: "within_quota", exceeded: ["max_bytes", "output_dir"] } as never,
      false,
    ) as unknown as { kind: string; modeLabel: string | null; reasonsZh?: string[] };
    assert.equal(legacyFallback.kind, "manual", "旧 exceeded 不再产生原因块");
    assert.equal(legacyFallback.modeLabel, "未知模式");
    assert.deepEqual(legacyFallback.reasonsZh, []);

    const legacyAuto = approval.projectApproval({ decision: "auto_by_mode", mode: "per_item", exceeded: [] } as never, true);
    assert.equal(legacyAuto.kind, "auto");
    assert.equal(legacyAuto.modeLabel, "未知模式");

    const normalized = approval.normalizeCardApproval({ decision: "manual", mode: "within_quota", exceeded: ["max_bytes"] });
    assert.ok(normalized);
    assert.ok(!("exceeded" in (normalized as object)), "规范化后不保留 exceeded");

    const noMode = approval.projectApproval({ decision: "manual" } as never, false);
    assert.equal(noMode.modeLabel, null, "没有 mode 字段时不显示模式");
  });

  await t.test("12. 旧事件流整体可分类：带旧 approval 的确认卡与旧 quota_exceeded_actual 错误不抛错", () => {
    const items = classifyStreamEvents([
      {
        seq: 1,
        kind: "confirm_card",
        payload: {
          run_id: "r1",
          plan_id: "p1",
          plan_hash: "h",
          snapshot: { source: "x", range: {}, estimated_bytes: 1, output_dir: "D:\\d" },
          approval: { decision: "manual", mode: "within_quota", exceeded: ["max_bytes"] },
          auto_executed: false,
        },
      },
      {
        seq: 2,
        kind: "error",
        payload: {
          run_id: "r1",
          code: "quota_exceeded_actual",
          status: "failed",
          plan_id: "p1",
          approval: { decision: "auto_by_mode", mode: "within_quota", exceeded: ["max_bytes_actual"] },
        },
      },
    ] as never);
    const err = items.find((i) => i.kind === "error") as never;
    const card = approval.projectErrorCard(err) as unknown as Record<string, unknown>;
    assert.ok(!("exceededZh" in card), "错误卡不再列超额项");
    assert.equal(typeof card.wording, "string");
    assert.ok(!/[a-z]_[a-z]/.test(String(card.wording)));
  });

  await t.test("13. 桥接 planApprove 透传可选 outputDir（agent_plan_approve 的 output_dir）", () => {
    const src = fs.readFileSync(path.resolve(here, "../../lib/desktopShellBridgeTauri.ts"), "utf-8");
    assert.match(src, /invoke\("agent_plan_approve", \{ planId, snapshot, outputDir: outputDir \?\? null \}\)/);
    const types = fs.readFileSync(path.resolve(here, "agentBridgeTypes.ts"), "utf-8");
    assert.match(types, /planApprove\(planId: string, snapshot: unknown, outputDir\?: string \| null\): Promise<void>;/);
  });
});
