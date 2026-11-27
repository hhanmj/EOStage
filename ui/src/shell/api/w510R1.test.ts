import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { evidenceSummary, evidenceRefReasonZh, projectEvidence } from "./agentConversationLogic.ts";
import { localMarkersToZh } from "./modelTextMarkers.ts";
import { openTaskOutputErrorText } from "./taskActionFlow.ts";
import { getAvailableTaskActions } from "./taskLogic.ts";
import type { StreamConclusionItem } from "./agentStreamLogic.ts";

const here = path.dirname(fileURLToPath(import.meta.url));

function read(rel: string): string {
  return fs.readFileSync(path.resolve(here, rel), "utf-8");
}

type Claim = StreamConclusionItem["claims"][number];
type Ref = Claim["refs"][number];

const thisRunBatch: Ref = { kind: "batch", id: "batch-1", label: "批次", trace_sequence: 22, response_count: 7 };
const taskRef: Ref = { kind: "task", id: "task-1", label: "任务", trace_sequence: null, response_count: null };

function conclusion(claims: Claim[]): StreamConclusionItem {
  return { kind: "conclusion", seq: 30, run_id: "run-c", text: "结论", claims, reasoning_off_refused: false };
}

test("复核三：多级容器中的正文、代码与转义隔离", () => {
  const cases: Array<[string, string]> = [
    ["- > 内容 [local]\n- - 内容 [redacted]", "- > 内容 （本机内容）\n- - 内容 （已脱敏）"],
    ["- > 正文 [local] `代码 [redacted]` \\`转义 [local]", "- > 正文 （本机内容） `代码 [redacted]` \\`转义 （本机内容）"],
    ["- > ```json\n  > {\"x\":\"[redacted]\"}\n  > ```\n  > 结束 [local]", "- > ```json\n  > {\"x\":\"[redacted]\"}\n  > ```\n  > 结束 （本机内容）"],
    ["- - ```ts\n    [local]\n    ```\n    后 [redacted]", "- - ```ts\n    [local]\n    ```\n    后 （已脱敏）"],
    ["- >     [local]\n  > 普通 [redacted]", "- >     [local]\n  > 普通 （已脱敏）"],
  ];
  for (const [input, expected] of cases) assert.equal(localMarkersToZh(input), expected, input);
});

test("嵌套组合保持代码与不确定的 fence 原文", () => {
  const cases: Array<[string, string]> = [
    ["> 1. - 段落 [local] `片段 [redacted]`\n>    - 下一项 [redacted]", "> 1. - 段落 （本机内容） `片段 [redacted]`\n>    - 下一项 （已脱敏）"],
    ["- > - ~~~ts\n  >   [local]\n  >   ~~~\n  >   后文 [redacted]", "- > - ~~~ts\n  >   [local]\n  >   ~~~\n  >   后文 （已脱敏）"],
    ["- >     [local]\n  > 结束 [redacted]", "- >     [local]\n  > 结束 （已脱敏）"],
    ["- > ```\n  > [local]\n```\n[redacted]", "- > ```\n  > [local]\n```\n[redacted]"],
    ["- - 前 [local] `未闭合 [redacted]\n    [local]", "- - 前 （本机内容） `未闭合 [redacted]\n    [local]"],
  ];
  for (const [input, expected] of cases) assert.equal(localMarkersToZh(input), expected, input);
});

test("复核二：单引用及非布尔核验均不得推断逐条结果", () => {
  for (const verified of [false, undefined, "false", null]) {
    const item = { ...conclusion([]), claims: [{ statement: "任务", verified, refs: [taskRef] }] } as unknown as StreamConclusionItem;
    assert.equal(evidenceSummary(item).status, "unverified");
    assert.equal(projectEvidence(item).refs[0].verified, null);
  }
  const multi = conclusion([{ statement: "任务", verified: false, refs: [taskRef, thisRunBatch] }]);
  assert.deepEqual(projectEvidence(multi).refs.map((r) => r.verified), [null, null]);
});

test("复核二：列表引用正文、嵌套代码与转义反引号", () => {
  const cases = [
    ["- 目录 [local]\n> 内容 [redacted]", "- 目录 （本机内容）\n> 内容 （已脱敏）"],
    ["> - 内容 [local] `代码 [redacted]`\n>   ```js\n>   [local]\n>   ```\n> 后续 [redacted]", "> - 内容 （本机内容） `代码 [redacted]`\n>   ```js\n>   [local]\n>   ```\n> 后续 （已脱敏）"],
    ["- 列表\n      [local]\n  说明 [redacted]", "- 列表\n      [local]\n  说明 （已脱敏）"],
    ["转义 \\` 正文 [local] 然后 `代码 [redacted]` 结束 [local]", "转义 \\` 正文 （本机内容） 然后 `代码 [redacted]` 结束 （本机内容）"],
    ["\\\\`代码 [local]` 后 [redacted]", "\\\\`代码 [local]` 后 （已脱敏）"],
    ["``代码 ` [local]\n继续 [redacted]`` 外 [local]", "``代码 ` [local]\n继续 [redacted]`` 外 （本机内容）"],
    ["\\`文字 [local] `未闭合 [redacted]\n[local]", "\\`文字 （本机内容） `未闭合 [redacted]\n[local]"],
  ];
  for (const [input, expected] of cases) assert.equal(localMarkersToZh(input), expected, input);
});

test("R4 复核：摘要和投影只认合法引用及严格布尔的陈述级核验", async (t) => {
  await t.test("混合结果按陈述计数，失败的多引用均为未知", () => {
    const item = conclusion([
      { statement: "通过", verified: true, refs: [thisRunBatch, { ...thisRunBatch }] },
      { statement: "未通过", verified: false, refs: [taskRef, thisRunBatch] },
      { statement: "无引用", verified: false, refs: [] },
    ]);
    const summary = evidenceSummary(item);
    assert.equal(summary.status, "partial");
    assert.equal(summary.reasonZh, "2 条有引用的陈述中 1 条已核验；另有 1 条陈述没有引用");
    const refs = projectEvidence(item).refs;
    assert.deepEqual(refs.map((ref) => ref.verified), [true, true, null, null]);
  });
  await t.test("空、异常或非布尔值不能当作已核验", () => {
    const invalid = [null, { statement: "bad", verified: "false", refs: [taskRef] }, { statement: "bad ref", verified: true, refs: [null] }];
    for (const claims of [[], invalid, "broken", null]) {
      const item = { ...conclusion([]), claims } as unknown as StreamConclusionItem;
      assert.notEqual(evidenceSummary(item).status, "verified");
      assert.equal(projectEvidence(item).verified, false);
    }
    const item = { ...conclusion([]), claims: [{ statement: "x", verified: "false", refs: [taskRef] }] } as unknown as StreamConclusionItem;
    assert.notEqual(projectEvidence(item).refs[0].verified, true);
  });
});

test("R4 复核：prose 替换与代码原文隔离", () => {
  const text = '前 [local] `inline [redacted]` 后 [redacted]\r\n```json\r\n{"secret":"[redacted]","path":"[local-reference]"}\r\n```\r\n~~~ts\n[local]\n~~~\n末 [local]';
  assert.equal(localMarkersToZh(text), '前 （本机内容） `inline [redacted]` 后 （已脱敏）\r\n```json\r\n{"secret":"[redacted]","path":"[local-reference]"}\r\n```\r\n~~~ts\n[local]\n~~~\n末 （本机内容）');
  for (const code of ['```json\n[redacted]\n[local]', '`未闭合 [redacted]\n[local]', '~~~js\n[local-reference]\n[redacted]', '    [redacted]\n    [local]', '> ```\n> [redacted]\n> ```', '- ```\n  [local]\n  ```']) {
    assert.equal(localMarkersToZh(code), code);
  }
});

test("W5-10 条目 1：依据摘要由引用逐条状态推导", async (t) => {
  await t.test("R4：L4 r10 C2 形状（引用都已核验、另有一条无引用的陈述）→ 部分核验，原因含无引用陈述数", () => {
    const summary = evidenceSummary(
      conclusion([
        { statement: "7 景", verified: true, refs: [thisRunBatch, { ...thisRunBatch }] },
        { statement: "建议", verified: false, refs: [] },
      ]),
    );
    assert.equal(summary.status, "partial");
    assert.equal(summary.labelZh, "部分核验");
    assert.equal(summary.reasonZh, "1 条有引用的陈述中 1 条已核验；另有 1 条陈述没有引用");
    assert.equal(summary.uncitedClaims, 1);
  });

  await t.test("R4：所有陈述都已核验 → 已核验", () => {
    const summary = evidenceSummary(
      conclusion([
        { statement: "7 景", verified: true, refs: [thisRunBatch] },
        { statement: "最新 10-18", verified: true, refs: [{ ...thisRunBatch }] },
      ]),
    );
    assert.equal(summary.status, "verified");
    assert.equal(summary.reasonZh, null);
    assert.equal(summary.uncitedClaims, 0);
  });

  await t.test("无引用即使标记为已核验，也不能计入已核验", () => {
    const summary = evidenceSummary(conclusion([
      { statement: "已核验", verified: true, refs: [thisRunBatch] },
      { statement: "缺少引用", verified: true, refs: [] },
    ]));
    assert.equal(summary.status, "partial");
    assert.equal(summary.uncitedClaims, 1);
  });

  await t.test("部分引用已核验 → 部分核验，原因写明条数", () => {
    const summary = evidenceSummary(
      conclusion([
        { statement: "7 景", verified: true, refs: [thisRunBatch] },
        { statement: "任务已建", verified: false, refs: [taskRef] },
      ]),
    );
    assert.equal(summary.status, "partial");
    assert.equal(summary.labelZh, "部分核验");
    assert.equal(summary.reasonZh, "2 条有引用的陈述中 1 条已核验");
  });

  await t.test("引用都未核验 → 报告陈述级结果", () => {
    const summary = evidenceSummary(conclusion([{ statement: "任务已建", verified: false, refs: [taskRef] }]));
    assert.equal(summary.status, "unverified");
    assert.equal(summary.labelZh, "未核验");
    assert.equal(summary.reasonZh, "1 条有引用的陈述中 0 条已核验");
  });

  await t.test("R4：引用了本次检索但未核验 → 未核验，原因不再与「运行记录第 #N 条」矛盾", () => {
    const summary = evidenceSummary(conclusion([{ statement: "7 景", verified: false, refs: [thisRunBatch] }]));
    assert.equal(summary.labelZh, "未核验");
    assert.equal(summary.reasonZh, "1 条有引用的陈述中 0 条已核验");
  });

  await t.test("R4：未核验且另有无引用陈述 → 原因一并写出", () => {
    const summary = evidenceSummary(
      conclusion([
        { statement: "7 景", verified: false, refs: [thisRunBatch] },
        { statement: "建议", verified: false, refs: [] },
      ]),
    );
    assert.equal(summary.labelZh, "未核验");
    assert.equal(summary.reasonZh, "1 条有引用的陈述中 0 条已核验；另有 1 条陈述没有引用");
  });

  await t.test("有结论但没有任何引用 → 未核验并给原因", () => {
    const summary = evidenceSummary(conclusion([{ statement: "建议", verified: false, refs: [] }]));
    assert.equal(summary.labelZh, "未核验");
    assert.equal(summary.reasonZh, "1 条陈述没有引用");
  });

  await t.test("没有结论条目 → 未引用证据", () => {
    assert.equal(evidenceSummary(conclusion([])).status, "uncited");
  });

  await t.test("R4：逐条引用的原因按所在陈述给出", () => {
    const webRef: Ref = { kind: "web", id: "https://example.invalid", label: "网页", trace_sequence: null, response_count: null };
    const oldBatch: Ref = { kind: "batch", id: "batch-old", label: "批次", trace_sequence: null, response_count: null };
    // 后端只返回陈述级的 all(refs) 结果，不暴露具体失败引用或原因。
    for (const ref of [thisRunBatch, taskRef, webRef, oldBatch]) {
      for (const count of [1, 2, 3]) {
        assert.equal(evidenceRefReasonZh(ref, count), "该陈述未通过核验，无法确定具体引用");
      }
    }
  });
});

test("W5-10 条目 3：模型正文的本机标记换成中文占位", async (t) => {
  await t.test("各种标记", () => {
    assert.equal(localMarkersToZh("凭据引用标记为 [local-reference]，说明"), "凭据引用标记为 （本机内容），说明");
    assert.equal(localMarkersToZh("目录 [local-reference:message_dir] 下"), "目录 （消息中指定的目录） 下");
    assert.equal(localMarkersToZh("日志在[local-reference:message_dir]下"), "日志在（消息中指定的目录）下");
    assert.equal(localMarkersToZh("令牌【[local-reference]]"), "令牌【（本机内容）]");
    assert.equal(localMarkersToZh("见 [local] 与 [已省略]"), "见 （本机内容） 与 （已省略）");
    assert.equal(localMarkersToZh("[local-reference:other_kind]"), "（本机内容）");
    assert.equal(localMarkersToZh("密钥 [redacted] 已隐藏"), "密钥 （已脱敏） 已隐藏");
  });
  await t.test("没有标记 → 原样；普通方括号不动", () => {
    assert.equal(localMarkersToZh("共 7 景 [1] 与 [local-ish]"), "共 7 景 [1] 与 [local-ish]");
  });
  await t.test("R3：只在事件分类一处替换（助手消息与结论），旧的 message_dir 专用替换已删除，渲染层不再重复", () => {
    const stream = read("./agentStreamLogic.ts");
    assert.equal(stream.includes("replaceMessageDirPlaceholder"), false, "旧的 message_dir 替换路径必须删除");
    assert.equal((stream.match(/localMarkersToZh\(/g) || []).length, 2, "助手消息与结论各替换一次");
    const workbench = read("../../components/desktop/DesktopAgentWorkbench.tsx");
    assert.equal(workbench.includes("localMarkersToZh"), false, "渲染层不得再替换");
    const card = read("../../components/desktop/AgentConclusionCard.tsx");
    assert.equal(card.includes("localMarkersToZh"), false, "渲染层不得再替换");
  });
});

test("W5-10 条目 2：「打开目录」失败的中文提示", async (t) => {
  await t.test("修订号不符 → 说明任务状态刚变化", () => {
    assert.equal(
      openTaskOutputErrorText({ ok: false, error: "Tauri application service operation failed", code: "V3_STORE_REVISION_CONFLICT" }),
      "任务状态刚刚变化，请再点一次「打开目录」",
    );
  });
  await t.test("R4：详情页按钮字是「打开」，提示与之一致", () => {
    assert.equal(
      openTaskOutputErrorText({ ok: false, error: "x", code: "V3_STORE_REVISION_CONFLICT" }, "打开"),
      "任务状态刚刚变化，请再点一次「打开」",
    );
  });
  await t.test("后端给了中文原因 → 用它", () => {
    assert.equal(openTaskOutputErrorText({ code: "X", message_zh: "保存目录不存在" }), "保存目录不存在");
  });
  await t.test("其他 → 「未能打开保存目录」", () => {
    assert.equal(openTaskOutputErrorText({ ok: false, error: "x", code: null }), "未能打开保存目录");
  });
});

test("W5-10 R4 P3：进行中/排队/暂停/取消/中断任务有保存目录时动作映射含 open_folder，抽屉按钮由映射派生", async (t) => {
  for (const status of ["queued", "running", "paused", "cancelled", "interrupted"] as const) {
    await t.test(status, () => {
      assert.equal(getAvailableTaskActions(status, true).includes("open_folder"), true);
      assert.equal(getAvailableTaskActions(status, false).includes("open_folder"), false);
    });
  }
  await t.test("抽屉按钮由映射派生", () => {
    const drawer = read("../../components/desktop/DesktopDownloadDrawer.tsx");
    assert.equal(drawer.includes('actions.includes("open_folder") && outputDir !== null'), true);
  });
});

// ============================================================================
// W5-10 R4 单元1：兜底请求 gate 的跨修订与快照协调
// ============================================================================

import type { TaskRetryFallbackOutcome } from "./types.ts";
import * as gateModule from "./taskFallbackRequestGate.ts";

const {
  getTaskFallbackRequestGate,
  startTaskFallbackRequest,
  settleTaskFallbackRequest,
  markTaskFallbackRequestUncertain,
  clearTaskFallbackRequest,
  reconcileTaskFallbackRequest,
  resetTaskFallbackRequestGatesForTests,
} = gateModule;

const reconcileSnapshot: (
  taskId: string,
  task: { revision: number; status: string } | null,
) => gateModule.TaskFallbackRequestGate | null =
  (gateModule as any).reconcileTaskFallbackRequestSnapshot ??
  ((taskId: string, task: { revision: number; status: string } | null) => {
    return (reconcileTaskFallbackRequest as any)(taskId, task);
  });

function makeFallbackOutcome(
  id: string,
  revision: number,
  needsConfirmation = true,
): TaskRetryFallbackOutcome {
  return {
    task: {
      task_id: id,
      revision: needsConfirmation ? revision : revision + 1,
      status: needsConfirmation ? "failed" : "queued",
      created: false,
    },
    refreshed_batch_id: "batch-x",
    rebuilt_item_ids: [],
    missing_product_ids: ["m"],
    added_product_ids: [],
    needs_user_confirmation: needsConfirmation,
  };
}

test("W5-10 R4 单元1：兜底请求 gate 跨修订与快照协调", async (t) => {
  await t.test("1. start while pending (same revision) → null，原 gate 保持 unchanged (同 generation, 仍 pending)", () => {
    resetTaskFallbackRequestGatesForTests();
    const g1 = startTaskFallbackRequest({ taskId: "t1", taskRevision: 7, step: "first" });
    assert.ok(g1);
    assert.equal(g1.generation, 1);
    assert.equal(g1.pending, true);

    const g2 = startTaskFallbackRequest({ taskId: "t1", taskRevision: 7, step: "first" });
    assert.equal(g2, null);

    const current = getTaskFallbackRequestGate("t1", 7);
    assert.ok(current);
    assert.equal(current.generation, 1);
    assert.equal(current.pending, true);
  });

  await t.test("2. start while pending with changed revision → null，原 gate 保持 intact (getter 返回原 gate，修订号仍为旧)", () => {
    resetTaskFallbackRequestGatesForTests();
    const g1 = startTaskFallbackRequest({ taskId: "t1", taskRevision: 7, step: "first" });
    assert.ok(g1);

    const g2 = startTaskFallbackRequest({ taskId: "t1", taskRevision: 8, step: "first" });
    assert.equal(g2, null);

    const current = getTaskFallbackRequestGate("t1", 7);
    assert.ok(current);
    assert.equal(current.taskRevision, 7);
    assert.equal(current.generation, 1);
    assert.equal(current.pending, true);
  });

  await t.test("3. start while uncertain with changed revision → null，原 gate 保持 intact", () => {
    resetTaskFallbackRequestGatesForTests();
    const g1 = startTaskFallbackRequest({ taskId: "t1", taskRevision: 7, step: "first" });
    assert.ok(g1);
    markTaskFallbackRequestUncertain(
      { taskId: "t1", taskRevision: 7, step: "first", generation: g1.generation },
      "网络异常",
    );
    const u1 = getTaskFallbackRequestGate("t1", 7);
    assert.equal(u1?.uncertain, true);

    const g2 = startTaskFallbackRequest({ taskId: "t1", taskRevision: 8, step: "first" });
    assert.equal(g2, null);

    const current = getTaskFallbackRequestGate("t1", 7);
    assert.ok(current);
    assert.equal(current.uncertain, true);
    assert.equal(current.taskRevision, 7);
    assert.equal(current.generation, 1);
  });

  await t.test("4. start for a DIFFERENT task while task A pending → 成功创建（任务隔离）", () => {
    resetTaskFallbackRequestGatesForTests();
    const gA = startTaskFallbackRequest({ taskId: "task-A", taskRevision: 7, step: "first" });
    assert.ok(gA);

    const gB = startTaskFallbackRequest({ taskId: "task-B", taskRevision: 7, step: "first" });
    assert.ok(gB);
    assert.equal(gB.taskId, "task-B");
    assert.equal(getTaskFallbackRequestGate("task-A", 7)?.pending, true);
    assert.equal(getTaskFallbackRequestGate("task-B", 7)?.pending, true);
  });

  await t.test("5. 回归：settled first-step (needs_user_confirmation true) + 同修订号 → 第二步启动；同修订号单纯重新发起第一步 → null", () => {
    resetTaskFallbackRequestGatesForTests();
    const g1 = startTaskFallbackRequest({ taskId: "t1", taskRevision: 7, step: "first" });
    assert.ok(g1);
    const outcome = makeFallbackOutcome("t1", 7, true);
    const settled = settleTaskFallbackRequest(
      { taskId: "t1", taskRevision: 7, step: "first", generation: g1.generation },
      outcome,
    );
    assert.ok(settled);

    const restartFirst = startTaskFallbackRequest({ taskId: "t1", taskRevision: 7, step: "first" });
    assert.equal(restartFirst, null);

    const second = startTaskFallbackRequest({ taskId: "t1", taskRevision: 7, step: "second", outcome });
    assert.ok(second);
    assert.equal(second.step, "second");
    assert.equal(second.pending, true);
  });

  await t.test("6. settled gate at revision 7 + new start at revision 8 → 成功启动新 gate，修订号 8，generation 递增，outcome 置为 null（stale diff 不泄露）", () => {
    resetTaskFallbackRequestGatesForTests();
    const g1 = startTaskFallbackRequest({ taskId: "t1", taskRevision: 7, step: "first" });
    assert.ok(g1);
    const outcome = makeFallbackOutcome("t1", 7, true);
    settleTaskFallbackRequest(
      { taskId: "t1", taskRevision: 7, step: "first", generation: g1.generation },
      outcome,
    );
    const oldGate = getTaskFallbackRequestGate("t1", 7);
    assert.ok(oldGate?.outcome);

    const g2 = startTaskFallbackRequest({ taskId: "t1", taskRevision: 8, step: "first" });
    assert.ok(g2);
    assert.equal(g2.taskRevision, 8);
    assert.ok(g2.generation > g1.generation);
    assert.equal(g2.outcome, null, "旧的 diff outcome 不得泄露给新的 gate");
    assert.equal(g2.pending, true);
  });

  await t.test("7. getter revision-agnostic: gate at revision 7, 查询传入 revision 8 → 返回该 gate", () => {
    resetTaskFallbackRequestGatesForTests();
    const g1 = startTaskFallbackRequest({ taskId: "t1", taskRevision: 7, step: "first" });
    assert.ok(g1);
    const queried = getTaskFallbackRequestGate("t1", 8);
    assert.ok(queried);
    assert.equal(queried.taskId, "t1");
    assert.equal(queried.taskRevision, 7);
  });

  await t.test("8. reconcile 匹配 snapshot → gate 保持 unchanged (仍 pending)", () => {
    resetTaskFallbackRequestGatesForTests();
    assert.equal(typeof (gateModule as any).reconcileTaskFallbackRequestSnapshot, "function");
    const g1 = startTaskFallbackRequest({ taskId: "t1", taskRevision: 7, step: "first" });
    assert.ok(g1);
    const reconciled = reconcileSnapshot("t1", { revision: 7, status: "failed" });
    assert.ok(reconciled);
    assert.equal(reconciled.pending, true);
    assert.equal(reconciled.uncertain, false);
    assert.equal(reconciled.generation, g1.generation);
  });

  await t.test("9. reconcile 分歧 snapshot (status 或 revision) → pending 变为 uncertain 并带有精准中文提示；第二次 reconcile 保持 untouched", () => {
    resetTaskFallbackRequestGatesForTests();
    // 9a: 修订号不符
    const g1 = startTaskFallbackRequest({ taskId: "t1", taskRevision: 7, step: "first" });
    assert.ok(g1);
    const r1 = reconcileSnapshot("t1", { revision: 8, status: "failed" });
    assert.ok(r1);
    assert.equal(r1.pending, false);
    assert.equal(r1.uncertain, true);
    assert.equal(r1.error, "请求已发出，但任务状态发生变化，结果无法安全确认；请刷新后核对任务记录");

    // 第二次 reconcile（已经是 uncertain）不受影响
    const r2 = reconcileSnapshot("t1", { revision: 9, status: "failed" });
    assert.ok(r2);
    assert.equal(r2.pending, false);
    assert.equal(r2.uncertain, true);
    assert.equal(r2.error, "请求已发出，但任务状态发生变化，结果无法安全确认；请刷新后核对任务记录");

    // 9b: 状态不符 (running)
    resetTaskFallbackRequestGatesForTests();
    const g2 = startTaskFallbackRequest({ taskId: "t2", taskRevision: 7, step: "first" });
    assert.ok(g2);
    const rStatus = reconcileSnapshot("t2", { revision: 7, status: "running" });
    assert.ok(rStatus);
    assert.equal(rStatus.pending, false);
    assert.equal(rStatus.uncertain, true);
    assert.equal(rStatus.error, "请求已发出，但任务状态发生变化，结果无法安全确认；请刷新后核对任务记录");
  });

  await t.test("10. reconcile 传入 null snapshot (任务消失) → pending 变为 uncertain；settled 与 uncertain gates 绝不被删除或修改", () => {
    resetTaskFallbackRequestGatesForTests();
    // 10a: pending 在 snapshot 为 null 时变为 uncertain
    const g1 = startTaskFallbackRequest({ taskId: "t1", taskRevision: 7, step: "first" });
    assert.ok(g1);
    const rNull = reconcileSnapshot("t1", null);
    assert.ok(rNull);
    assert.equal(rNull.pending, false);
    assert.equal(rNull.uncertain, true);
    assert.equal(rNull.error, "请求已发出，但任务状态发生变化，结果无法安全确认；请刷新后核对任务记录");

    // 10b: 无 gate 时返回 null
    assert.equal(reconcileSnapshot("non-existent", null), null);

    // 10c: settled gate 不被 null 或 diverged snapshot 篡改或删除
    const g2 = startTaskFallbackRequest({ taskId: "t2", taskRevision: 7, step: "first" });
    assert.ok(g2);
    settleTaskFallbackRequest(
      { taskId: "t2", taskRevision: 7, step: "first", generation: g2.generation },
      makeFallbackOutcome("t2", 7, true),
    );
    const rSettledNull = reconcileSnapshot("t2", null);
    assert.ok(rSettledNull);
    assert.equal(rSettledNull.pending, false);
    assert.equal(rSettledNull.uncertain, false);
    assert.equal(rSettledNull.outcome !== null, true);

    const rSettledDiverged = reconcileSnapshot("t2", { revision: 8, status: "completed" });
    assert.ok(rSettledDiverged);
    assert.equal(rSettledDiverged.pending, false);
    assert.equal(rSettledDiverged.uncertain, false);

    // 10d: uncertain gate 绝不被 reconcile 释放或覆盖错误
    const g3 = startTaskFallbackRequest({ taskId: "t3", taskRevision: 7, step: "first" });
    assert.ok(g3);
    markTaskFallbackRequestUncertain(
      { taskId: "t3", taskRevision: 7, step: "first", generation: g3.generation },
      "自定义错误",
    );
    const rUncertainNull = reconcileSnapshot("t3", null);
    assert.equal(rUncertainNull?.error, "自定义错误");
  });

  await t.test("11. mark uncertain 带有 dropOutcome → outcome 置为 null；无 dropOutcome → outcome 保留", () => {
    resetTaskFallbackRequestGatesForTests();
    // 带有 dropOutcome: true
    const gFirst1 = startTaskFallbackRequest({ taskId: "t1", taskRevision: 7, step: "first" });
    assert.ok(gFirst1);
    const outcome1 = makeFallbackOutcome("t1", 7, true);
    settleTaskFallbackRequest(
      { taskId: "t1", taskRevision: 7, step: "first", generation: gFirst1.generation },
      outcome1,
    );
    const g1 = startTaskFallbackRequest({
      taskId: "t1",
      taskRevision: 7,
      step: "second",
      outcome: outcome1,
    });
    assert.ok(g1?.outcome);
    const uDrop = markTaskFallbackRequestUncertain(
      { taskId: "t1", taskRevision: 7, step: "second", generation: g1.generation },
      "操作异常",
      { dropOutcome: true },
    );
    assert.ok(uDrop);
    assert.equal(uDrop.uncertain, true);
    assert.equal(uDrop.outcome, null, "dropOutcome 为 true 时 outcome 应置为 null");

    // 默认行为（无 dropOutcome）
    const gFirst2 = startTaskFallbackRequest({ taskId: "t2", taskRevision: 7, step: "first" });
    assert.ok(gFirst2);
    const outcome2 = makeFallbackOutcome("t2", 7, true);
    settleTaskFallbackRequest(
      { taskId: "t2", taskRevision: 7, step: "first", generation: gFirst2.generation },
      outcome2,
    );
    const g2 = startTaskFallbackRequest({
      taskId: "t2",
      taskRevision: 7,
      step: "second",
      outcome: outcome2,
    });
    assert.ok(g2?.outcome);
    const uKeep = markTaskFallbackRequestUncertain(
      { taskId: "t2", taskRevision: 7, step: "second", generation: g2.generation },
      "操作异常",
    );
    assert.ok(uKeep);
    assert.equal(uKeep.uncertain, true);
    assert.ok(uKeep.outcome !== null, "默认行为应保留 outcome");
  });

  await t.test("12. 回归：settle 使用错误标识 (generation / step / revision 不符) → 返回 null 且 gate 不被篡改", () => {
    resetTaskFallbackRequestGatesForTests();
    const g1 = startTaskFallbackRequest({ taskId: "t1", taskRevision: 7, step: "first" });
    assert.ok(g1);
    const outcome = makeFallbackOutcome("t1", 7, true);

    // 错误 generation
    assert.equal(
      settleTaskFallbackRequest(
        { taskId: "t1", taskRevision: 7, step: "first", generation: g1.generation + 99 },
        outcome,
      ),
      null,
    );
    // 错误 step
    assert.equal(
      settleTaskFallbackRequest(
        { taskId: "t1", taskRevision: 7, step: "second", generation: g1.generation },
        outcome,
      ),
      null,
    );
    // 错误 revision
    assert.equal(
      settleTaskFallbackRequest(
        { taskId: "t1", taskRevision: 99, step: "first", generation: g1.generation },
        outcome,
      ),
      null,
    );

    const current = getTaskFallbackRequestGate("t1", 7);
    assert.ok(current);
    assert.equal(current.pending, true);
    assert.equal(current.outcome, null);
  });

  await t.test("13. 回归：clear 传入错误 revision → 返回 false 且 gate 保留；正确 revision 清除成功", () => {
    resetTaskFallbackRequestGatesForTests();
    const g1 = startTaskFallbackRequest({ taskId: "t1", taskRevision: 7, step: "first" });
    assert.ok(g1);

    assert.equal(clearTaskFallbackRequest({ taskId: "t1", taskRevision: 99 }), false);
    assert.ok(getTaskFallbackRequestGate("t1", 7));

    assert.equal(clearTaskFallbackRequest({ taskId: "t1", taskRevision: 7 }), true);
    assert.equal(getTaskFallbackRequestGate("t1", 7), null);
  });

  await t.test("兼容性：reconcileTaskFallbackRequest 单对象参数旧调用点正常工作", () => {
    resetTaskFallbackRequestGatesForTests();
    const gOld = startTaskFallbackRequest({ taskId: "t-compat", taskRevision: 7, step: "first" });
    assert.ok(gOld);
    const rOld = reconcileTaskFallbackRequest({ task_id: "t-compat", revision: 8, status: "failed" });
    assert.ok(rOld);
    assert.equal(rOld.uncertain, true);
  });
});

test("W5-10 R4：gate 校验与防注入：只接受同任务同修订原第一步 offer，拒绝伪造与跨修订 outcome", async (t) => {
  await t.test("rev7 已结清 offer 存在时，rev8 传入旧 outcome 启动第二步必须返回 null 且不注入旧结果", () => {
    resetTaskFallbackRequestGatesForTests();
    const g1 = startTaskFallbackRequest({ taskId: "t1", taskRevision: 7, step: "first" });
    assert.ok(g1);
    const offer7 = makeFallbackOutcome("t1", 7, true);
    settleTaskFallbackRequest(
      { taskId: "t1", taskRevision: 7, step: "first", generation: g1.generation },
      offer7,
    );
    const settled = getTaskFallbackRequestGate("t1", 7);
    assert.ok(settled?.outcome);

    // rev8 尝试调用第二步，传入 rev7 的 outcome
    const gRev8 = startTaskFallbackRequest({
      taskId: "t1",
      taskRevision: 8,
      step: "second",
      outcome: offer7,
    });
    // 契约：第二步确认必须严格对应同修订号已 settled 的合法 offer，rev8 尚未经过第一步，绝不能启动第二步
    assert.equal(gRev8, null, "未经过 rev8 第一步的确认请求必须返回 null");

    // 检查 gate，不得为 rev8 创建带有旧 offer 的第二步 gate
    const current = getTaskFallbackRequestGate("t1", 8);
    assert.equal(current?.step !== "second", true, "不得为 rev8 建立 step 为 second 的 gate");
    assert.equal(current === null || current.outcome === null, true, "不得向 rev8 注入旧 revision 的 outcome");
  });

  await t.test("第一步请求必须为全新检索，不接受外部传入或旧有的 outcome", () => {
    resetTaskFallbackRequestGatesForTests();
    const staleOutcome = makeFallbackOutcome("t1", 7, true);
    const g1 = startTaskFallbackRequest({
      taskId: "t1",
      taskRevision: 7,
      step: "first",
      outcome: staleOutcome,
    });
    assert.ok(g1);
    assert.equal(g1.step, "first");
    // 第一步开始时处于 pending，尚未拿到服务端返回，outcome 必须严格为 null
    assert.equal(g1.outcome, null, "第一步启动时 outcome 必须强制为 null，不得由外部注入");
  });

  await t.test("第二步确认必须严格校验 task ID，拒绝来自其它任务的伪造 outcome", () => {
    resetTaskFallbackRequestGatesForTests();
    const gA = startTaskFallbackRequest({ taskId: "task-A", taskRevision: 7, step: "first" });
    assert.ok(gA);
    const outcomeA = makeFallbackOutcome("task-A", 7, true);
    settleTaskFallbackRequest(
      { taskId: "task-A", taskRevision: 7, step: "first", generation: gA.generation },
      outcomeA,
    );

    // 针对 task-B 发起第二步，传入 task-A 的 outcome
    const gB = startTaskFallbackRequest({
      taskId: "task-B",
      taskRevision: 7,
      step: "second",
      outcome: outcomeA,
    });
    assert.equal(gB, null, "传入不匹配的 task outcome 必须返回 null");
    assert.equal(getTaskFallbackRequestGate("task-B", 7), null);
  });

  await t.test("第二步确认必须与已结清的真实 offer 严格匹配，拒绝篡改 batch 的 outcome", () => {
    resetTaskFallbackRequestGatesForTests();
    const g1 = startTaskFallbackRequest({ taskId: "t1", taskRevision: 7, step: "first" });
    assert.ok(g1);
    const officialOffer = makeFallbackOutcome("t1", 7, true);
    officialOffer.refreshed_batch_id = "batch-official-123";
    settleTaskFallbackRequest(
      { taskId: "t1", taskRevision: 7, step: "first", generation: g1.generation },
      officialOffer,
    );

    // 篡改 batch 伪造第二步确认
    const forgedOffer = { ...officialOffer, refreshed_batch_id: "batch-forged-999" };
    const gSecond = startTaskFallbackRequest({
      taskId: "t1",
      taskRevision: 7,
      step: "second",
      outcome: forgedOffer,
    });
    assert.equal(gSecond, null, "批次与第一步结果不符时必须拒绝启动第二步");

    // 原合法 gate 不受影响，仍为 settled
    const current = getTaskFallbackRequestGate("t1", 7);
    assert.ok(current);
    assert.equal(current.outcome?.refreshed_batch_id, "batch-official-123");
  });

  await t.test("getTaskFallbackRequestGate 查询新 revision 时不得返回旧 revision 已结清的 diff outcome", () => {
    resetTaskFallbackRequestGatesForTests();
    const g1 = startTaskFallbackRequest({ taskId: "t1", taskRevision: 7, step: "first" });
    assert.ok(g1);
    const offer7 = makeFallbackOutcome("t1", 7, true);
    settleTaskFallbackRequest(
      { taskId: "t1", taskRevision: 7, step: "first", generation: g1.generation },
      offer7,
    );

    // 当任务快照更新为 rev8 时，以 rev8 查询 gate
    const gateForRev8 = getTaskFallbackRequestGate("t1", 8);
    // 已经 settled 的旧 revision 差异结果绝不能作为 rev8 的有效 gate/outcome 返回给上层
    assert.equal(
      gateForRev8 === null || gateForRev8.outcome === null,
      true,
      "查询 rev8 时不得提供 rev7 的差异 outcome",
    );
  });

  await t.test("reconcileTaskFallbackRequestSnapshot 在快照修订变更时清理已结清的旧 diff offer", () => {
    resetTaskFallbackRequestGatesForTests();
    const g1 = startTaskFallbackRequest({ taskId: "t1", taskRevision: 7, step: "first" });
    assert.ok(g1);
    const offer7 = makeFallbackOutcome("t1", 7, true);
    settleTaskFallbackRequest(
      { taskId: "t1", taskRevision: 7, step: "first", generation: g1.generation },
      offer7,
    );

    // 任务快照变为 rev8 failed，协调快照
    const reconciled = reconcileSnapshot("t1", { revision: 8, status: "failed" });
    // 跨修订后旧的差异 offer 不得继续保留在 gate 中
    assert.equal(
      reconciled === null || reconciled.outcome === null,
      true,
      "跨修订快照对齐后旧 diff offer 必须被清理",
    );
  });

  await t.test("防回归：pending 与 uncertain 门禁跨修订依然禁止释放与重复发送", () => {
    resetTaskFallbackRequestGatesForTests();
    // 处于 pending 时
    const gPending = startTaskFallbackRequest({ taskId: "t1", taskRevision: 7, step: "first" });
    assert.ok(gPending);
    assert.equal(startTaskFallbackRequest({ taskId: "t1", taskRevision: 8, step: "first" }), null);

    // 处于 uncertain 时
    markTaskFallbackRequestUncertain(
      { taskId: "t1", taskRevision: 7, step: "first", generation: gPending.generation },
      "网络超时",
    );
    assert.equal(startTaskFallbackRequest({ taskId: "t1", taskRevision: 8, step: "first" }), null);
    const current = getTaskFallbackRequestGate("t1", 8);
    assert.equal(current?.uncertain, true);
  });

  await t.test("跨任务伪造确认被拒绝后，原合法任务已结清的 offer 保持完整且可正常完成第二步", () => {
    resetTaskFallbackRequestGatesForTests();
    const gA = startTaskFallbackRequest({ taskId: "task-A", taskRevision: 7, step: "first" });
    assert.ok(gA);
    const outcomeA = makeFallbackOutcome("task-A", 7, true);
    settleTaskFallbackRequest(
      { taskId: "task-A", taskRevision: 7, step: "first", generation: gA.generation },
      outcomeA,
    );

    // 尝试用 task-A 的 offer 去为 task-B 发起第二步
    const forgedB = startTaskFallbackRequest({
      taskId: "task-B",
      taskRevision: 7,
      step: "second",
      outcome: outcomeA,
    });
    assert.equal(forgedB, null);

    // 检查 task-A：offer 依然完整保留
    const gateA = getTaskFallbackRequestGate("task-A", 7);
    assert.ok(gateA);
    assert.equal(gateA.outcome?.refreshed_batch_id, outcomeA.refreshed_batch_id);

    // task-A 依然可以正常进行合法的第二步
    const legalSecondA = startTaskFallbackRequest({
      taskId: "task-A",
      taskRevision: 7,
      step: "second",
      outcome: outcomeA,
    });
    assert.ok(legalSecondA);
    assert.equal(legalSecondA.step, "second");
    assert.equal(legalSecondA.pending, true);
  });

  await t.test("settle 后外部篡改原始 outcome 对象（如修改 batch、task 属性或推入 product），内部 gate 与第二步使用的 offer 保持不可篡改", () => {
    resetTaskFallbackRequestGatesForTests();
    const g1 = startTaskFallbackRequest({ taskId: "t1", taskRevision: 7, step: "first" });
    assert.ok(g1);
    const originalOffer = makeFallbackOutcome("t1", 7, true);
    originalOffer.refreshed_batch_id = "batch-original";
    originalOffer.missing_product_ids = ["p-1"];
    settleTaskFallbackRequest(
      { taskId: "t1", taskRevision: 7, step: "first", generation: g1.generation },
      originalOffer,
    );

    // 外部在 settle 后修改原始传入对象
    originalOffer.refreshed_batch_id = "batch-mutated-hacked";
    originalOffer.task.revision = 999;
    originalOffer.missing_product_ids.push("p-injected");

    // 检查内部 gate 的 outcome 保持不可篡改
    const gate = getTaskFallbackRequestGate("t1", 7);
    assert.ok(gate?.outcome);
    assert.equal(gate.outcome.refreshed_batch_id, "batch-original");
    assert.equal(gate.outcome.task.revision, 7);
    assert.deepEqual(gate.outcome.missing_product_ids, ["p-1"]);

    // 启动第二步，其自动绑定的 offer 也完全是原始干净的 offer
    const g2 = startTaskFallbackRequest({ taskId: "t1", taskRevision: 7, step: "second" });
    assert.ok(g2?.outcome);
    assert.equal(g2.outcome.refreshed_batch_id, "batch-original");
    assert.equal(g2.outcome.task.revision, 7);
    assert.deepEqual(g2.outcome.missing_product_ids, ["p-1"]);
  });
});
