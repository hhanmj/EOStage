import test from "node:test";
import assert from "node:assert";
import type {
  AgentCardApproval,
  AgentStreamEvent,
} from "../../lib/generated/v3-contracts.ts";
import type { ApplicationTask } from "./types.ts";
import {
  projectApproval,
  planAutoCancel,
  autoCancelOutcomeText,
  normalizeCardApproval,
  CARD_REASON_ZH,
} from "./approvalLogic.ts";
import { classifyStreamEvents } from "./agentStreamLogic.ts";
import { explainErrorCode } from "./errorExplanations.ts";

test("W5-2f/W5-2g 审批决策、确认原因与自动取消纯逻辑测试套件", async (t) => {
  // ==========================================================================
  // 1. projectApproval 审批投影
  // ==========================================================================
  await t.test("1.1 projectApproval: auto 模式（decision 为 auto_by_mode 且 autoExecuted 为 true）", () => {
    const approval: AgentCardApproval = { decision: "auto_by_mode", mode: "full_access", card_reasons: [] };
    const res = projectApproval(approval, true);
    assert.strictEqual(res.kind, "auto");
    assert.strictEqual(res.modeLabel, "完全访问");
    assert.deepStrictEqual(res.reasonsZh, []);
  });

  await t.test("1.2 projectApproval: reasons 模式（decision 为 manual 且 card_reasons 非空）", () => {
    const approval: AgentCardApproval = {
      decision: "manual",
      mode: "auto_approve",
      card_reasons: ["output_dir_unknown", "size_unknown"],
    };
    const res = projectApproval(approval, false);
    assert.strictEqual(res.kind, "reasons");
    assert.strictEqual(res.modeLabel, "自主审批");
    assert.deepStrictEqual(res.reasonsZh, ["下载目录未知，请在卡上选择", "预计大小未知"]);
  });

  await t.test("1.3 projectApproval: forbidden 模式（decision 为 refused_forbidden 或 errorCode 为 plan_refused_forbidden）", () => {
    const approval: AgentCardApproval = { decision: "refused_forbidden", mode: "full_access", card_reasons: [] };
    const res = projectApproval(approval, false);
    assert.strictEqual(res.kind, "forbidden");
    assert.strictEqual(res.modeLabel, "完全访问");

    const resErr = projectApproval(null, false, "plan_refused_forbidden");
    assert.strictEqual(resErr.kind, "forbidden");
    assert.strictEqual(resErr.modeLabel, null);
  });

  await t.test("1.4 projectApproval: manual 模式（变更前确认、无 approval 或不一致情况）", () => {
    const res1 = projectApproval({ decision: "manual", mode: "confirm_changes", card_reasons: [] }, false);
    assert.strictEqual(res1.kind, "manual");
    assert.strictEqual(res1.modeLabel, "变更前确认");
    assert.deepStrictEqual(res1.reasonsZh, []);

    const res2 = projectApproval(null, false);
    assert.strictEqual(res2.kind, "manual");
    assert.strictEqual(res2.modeLabel, null);

    // 不一致情况：decision 为 auto_by_mode 但 autoExecuted 不为 true -> 保守回退为 manual
    const res3 = projectApproval({ decision: "auto_by_mode", mode: "auto_approve" }, false);
    assert.strictEqual(res3.kind, "manual");
    assert.strictEqual(res3.modeLabel, "自主审批");
  });

  await t.test("2. card_reasons 映射保持后端顺序，未知值丢弃", () => {
    const res = projectApproval(
      {
        decision: "manual",
        mode: "confirm_changes",
        card_reasons: ["ledger_edit_requires_confirm", "unknown_reason" as any, "account_not_ready"],
      },
      false
    );
    assert.deepStrictEqual(res.reasonsZh, ["修改任务记录需要确认", "所需账号未就绪"]);
    assert.strictEqual(CARD_REASON_ZH.archive_requires_confirm, "归档任务需要确认");
  });

  // ==========================================================================
  // 3. planAutoCancel 计算
  // ==========================================================================
  await t.test("3. planAutoCancel: runLive 判定与任务状态分类", () => {
    const tasks: ApplicationTask[] = [
      {
        task_id: "task-running",
        revision: 2,
        category: "scene_download",
        name: "下载景1",
        status: "running",
        done_bytes: 100,
      },
      {
        task_id: "task-paused",
        revision: 1,
        category: "scene_download",
        name: "下载景2",
        status: "paused",
        done_bytes: 50,
      },
      {
        task_id: "task-finished",
        revision: 4,
        category: "scene_download",
        name: "下载景3",
        status: "completed",
        done_bytes: 200,
      },
    ];

    // runLive 为 true
    const resLive = planAutoCancel({
      runId: "run-1",
      runLive: true,
      createdTaskIds: ["task-running", "task-paused", "task-finished", "task-not-exist"],
      tasks,
    });
    assert.strictEqual(resLive.cancelRun, true);
    assert.deepStrictEqual(resLive.cancelTasks, [
      { taskId: "task-running", revision: 2 },
      { taskId: "task-paused", revision: 1 },
    ]);
    assert.deepStrictEqual(resLive.skipped, [
      { taskId: "task-finished", reason: "not_active" },
      { taskId: "task-not-exist", reason: "not_found" },
    ]);

    // runLive 为 false
    const resNotLive = planAutoCancel({
      runId: "run-1",
      runLive: false,
      createdTaskIds: ["task-running"],
      tasks,
    });
    assert.strictEqual(resNotLive.cancelRun, false);
    assert.deepStrictEqual(resNotLive.cancelTasks, [
      { taskId: "task-running", revision: 2 },
    ]);
  });

  // ==========================================================================
  // 4. autoCancelOutcomeText 文案分支
  // ==========================================================================
  await t.test("4. autoCancelOutcomeText: 结果文案各分支", () => {
    const tasks: ApplicationTask[] = [
      {
        task_id: "task-1",
        revision: 1,
        category: "scene_download",
        name: "任务一",
        status: "cancelled",
        done_bytes: 0,
      },
      {
        task_id: "task-2",
        revision: 1,
        category: "scene_download",
        name: "任务二",
        status: "failed",
        done_bytes: 0,
      },
    ];

    // 分支 1：全部成功（runCancelled 为真，取消了任务）
    const textAllSuccess = autoCancelOutcomeText(
      {
        runCancelled: true,
        cancelledTaskIds: ["task-1"],
        failedTaskIds: [],
        skipped: [],
      },
      tasks
    );
    assert.strictEqual(textAllSuccess, "已取消本次运行和 1 个任务");

    // 分支 2：没有运行可取消时（仅取消任务）
    const textOnlyTasks = autoCancelOutcomeText(
      {
        runCancelled: false,
        cancelledTaskIds: ["task-1", "task-2"],
        failedTaskIds: [],
        skipped: [],
      },
      tasks
    );
    assert.strictEqual(textOnlyTasks, "已取消 2 个任务");

    // 分支 3：什么都不用取消
    const textNothing = autoCancelOutcomeText(
      {
        runCancelled: false,
        cancelledTaskIds: [],
        failedTaskIds: [],
        skipped: [{ taskId: "task-1", reason: "not_active" }],
      },
      tasks
    );
    assert.strictEqual(textNothing, "本次运行与任务都已结束，无需取消");

    // 分支 4：有失败或 not_found 时追加「未能取消：<任务名或 id>」
    const textWithFailure = autoCancelOutcomeText(
      {
        runCancelled: true,
        cancelledTaskIds: ["task-1"],
        failedTaskIds: ["task-2"],
        skipped: [{ taskId: "task-unknown-99", reason: "not_found" }],
      },
      tasks
    );
    assert.strictEqual(
      textWithFailure,
      "已取消本次运行和 1 个任务，未能取消：任务二、task-unknown-99"
    );
  });

  // ==========================================================================
  // 5. 新错误码映射
  // ==========================================================================
  await t.test("5. errorExplanations 包含三个错误码规范中文解释（quota_exceeded_actual 只出现在 W3e 旧事件里）", () => {
    assert.strictEqual(
      explainErrorCode("plan_refused_forbidden"),
      "该操作属于禁止档，任何模式都不执行"
    );
    assert.strictEqual(
      explainErrorCode("quota_exceeded_actual"),
      "实际下载量超过本次运行字节上限的 110%，Agent 建的任务已暂停"
    );
    assert.strictEqual(
      explainErrorCode("record_failed"),
      "运行记录写入失败，本次运行已终止"
    );
  });

  // ==========================================================================
  // 6. classifyStreamEvents 保留 approval, auto_executed, created_task_ids
  // ==========================================================================
  await t.test("6. classifyStreamEvents: 规范化保留 approval、auto_executed 与 created_task_ids", () => {
    const rawEvents: AgentStreamEvent[] = [
      {
        seq: 1,
        kind: "plan_card",
        payload: {
          run_id: "r1",
          plan_id: "p1",
          plan_hash: "h1",
          max_tier: 1,
          steps: [],
          approval: {
            decision: "auto_by_mode",
            mode: "full_access",
            card_reasons: [],
          },
          auto_executed: true,
        },
      },
      {
        seq: 2,
        kind: "confirm_card",
        payload: {
          run_id: "r1",
          plan_id: "p1",
          plan_hash: "h1",
          snapshot: { source: "test" } as any,
          approval: {
            decision: "manual",
            mode: "auto_approve",
            card_reasons: ["size_unknown", "invalid_reason"],
          },
          auto_executed: false,
        },
      },
      {
        seq: 3,
        kind: "progress_card",
        payload: {
          run_id: "r1",
          plan_id: "p1",
          status: "running",
          created_task_ids: ["task-a", 123 as any, "task-b", null as any],
        },
      },
      {
        seq: 4,
        kind: "error",
        payload: {
          run_id: "r1",
          code: "plan_refused_forbidden",
          approval: {
            decision: "refused_forbidden",
            mode: "full_access",
            card_reasons: [],
          },
        },
      },
    ];

    const classified = classifyStreamEvents(rawEvents);
    assert.strictEqual(classified.length, 4);

    const planCard = classified[0] as any;
    assert.strictEqual(planCard.kind, "plan_card");
    assert.deepStrictEqual(planCard.approval, {
      decision: "auto_by_mode",
      mode: "full_access",
      card_reasons: [],
    });
    assert.strictEqual(planCard.auto_executed, true);

    const confirmCard = classified[1] as any;
    assert.strictEqual(confirmCard.kind, "confirm_card");
    assert.deepStrictEqual(confirmCard.approval, {
      decision: "manual",
      mode: "auto_approve",
      card_reasons: ["size_unknown"],
    });
    assert.strictEqual(confirmCard.auto_executed, false);

    const progressCard = classified[2] as any;
    assert.strictEqual(progressCard.kind, "progress_card");
    assert.deepStrictEqual(progressCard.created_task_ids, ["task-a", "task-b"]);

    const errorItem = classified[3] as any;
    assert.strictEqual(errorItem.kind, "error");
    assert.deepStrictEqual(errorItem.approval, {
      decision: "refused_forbidden",
      mode: "full_access",
      card_reasons: [],
    });

    // 负例测试：decision 不合法时整个 approval 置为 null，auto_executed 非 true 时为 false
    const invalidEvents: AgentStreamEvent[] = [
      {
        seq: 5,
        kind: "plan_card",
        payload: {
          run_id: "r2",
          plan_id: "p2",
          steps: [],
          approval: {
            decision: "invalid_decision" as any,
            mode: "confirm_changes",
          },
          auto_executed: "true" as any, // 非 strict boolean true
        },
      },
    ];
    const invalidClassified = classifyStreamEvents(invalidEvents);
    const invalidPlan = invalidClassified[0] as any;
    assert.strictEqual(invalidPlan.approval, null);
    assert.strictEqual(invalidPlan.auto_executed, false);
  });
});
