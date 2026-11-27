import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  findTerminalProgressForPlan,
  projectConfirmCard,
  buildConfirmCardTaskModels,
  projectPlanStepStatus,
  planStepStatusLabelZh,
  progressStatusLabelZh,
  progressStatusLineZh,
} from "./agentConversationLogic.ts";
import { classifyStreamEvents } from "./agentStreamLogic.ts";
import type {
  StreamConfirmCardItem,
  StreamProgressCardItem,
} from "./agentStreamLogic.ts";
import type { AgentStreamEvent } from "./agentBridgeTypes.ts";
import type { ApplicationTask } from "./types.ts";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

function makeSampleTask(overrides: Partial<ApplicationTask> = {}): ApplicationTask {
  const base: ApplicationTask = {
    task_id: "task-unified-65046dba-c55e-4a01-a49f-728eac13d277",
    name: "Sentinel-1D SLC 下载任务",
    provider: "asf",
    dataset: "s1_slc",
    status: "running",
    items_count: 1,
    total_bytes: 4_700_000_000,
    done_bytes: 210_000_000,
    output_dir: "D:/ScenePort/downloads",
    created_at: "2026-10-26T22:31:00Z",
    updated_at: "2026-10-26T22:31:10Z",
    revision: 2,
  };
  return { ...base, ...overrides };
}

test("W5-8 R3 (条目 4+5): 已批准确认卡终态投影、任务卡模型与计划步骤/状态行中文测试套件", async (suite) => {
  await suite.test(
    "1. (a) findTerminalProgressForPlan: L4 D4 真实事件序列 [running, done+created_task_ids] 必须返回终态 done 项，projectConfirmCard 投影为 approved",
    () => {
      // 按 L4 D4-session-after-approve.json seq 49-53 构造真实事件流
      const d4Events: AgentStreamEvent[] = [
        {
          seq: 49,
          kind: "plan_card",
          payload: {
            approval: {
              card_reasons: ["size_unknown"],
              decision: "manual",
              mode: "confirm_changes",
            },
            auto_executed: false,
            max_tier: 2,
            plan_hash: "b060b8ab2b17d7fe94bead369c285edfb72a0d4a109bfe60fb6324ce26f69703",
            plan_id: "plan-1a17a56f09d-5",
            run_id: "run-1a17a54dea1-3",
            steps: [
              {
                label_zh: "为最新一景 S1D_IW_SLC__1SDV_20261025T230339 创建下载任务",
                status: "pending",
                tier: 2,
                tool_id: "create_task",
              },
            ],
          },
        },
        {
          seq: 50,
          kind: "confirm_card",
          payload: {
            approval: {
              card_reasons: ["size_unknown"],
              decision: "manual",
              mode: "confirm_changes",
            },
            auto_executed: false,
            plan_hash: "b060b8ab2b17d7fe94bead369c285edfb72a0d4a109bfe60fb6324ce26f69703",
            plan_id: "plan-1a17a56f09d-5",
            run_id: "run-1a17a54dea1-3",
            snapshot: {
              already_queued_count: 0,
              estimated_bytes: null,
              orbit_directions: ["Descending"],
              output_dir: "D:\\ScenePort\\downloads",
              output_dir_source: "default",
              range: {
                aoi_name: "地图绘制矩形",
                area_km2: 125.6,
                product: "1 个产品",
                time_range: {
                  end: "2026-10-26T23:59:59Z",
                  start: "2026-07-26T00:00:00Z",
                },
              },
              result_count: 1,
              source: "asf.catalog",
            },
          },
        },
        {
          seq: 51,
          kind: "progress_card",
          payload: {
            code: null,
            created_task_ids: [],
            plan_id: "plan-1a17a56f09d-5",
            run_id: "run-1a17a54dea1-3",
            status: "running",
            step_index: 0,
            tool_id: "create_task",
          },
        },
        {
          seq: 53,
          kind: "progress_card",
          payload: {
            code: null,
            created_task_ids: ["task-unified-65046dba-c55e-4a01-a49f-728eac13d277"],
            plan_id: "plan-1a17a56f09d-5",
            run_id: "run-1a17a54dea1-3",
            status: "done",
            step_index: 0,
            tool_id: "create_task",
          },
        },
      ];

      const classified = classifyStreamEvents(d4Events);
      const confirmIdx = classified.findIndex((it) => it.kind === "confirm_card");
      assert.ok(confirmIdx >= 0);
      const confirmItem = classified[confirmIdx] as StreamConfirmCardItem;

      const selected = findTerminalProgressForPlan(
        classified.slice(confirmIdx + 1),
        confirmItem.plan_id
      );
      assert.ok(selected, "应找到该 plan_id 的后续 progress_card");
      assert.strictEqual(
        selected.status,
        "done",
        "存在 [running, done] 时必须选取终态 done 而非第一张 running"
      );
      assert.deepStrictEqual(selected.created_task_ids, [
        "task-unified-65046dba-c55e-4a01-a49f-728eac13d277",
      ]);

      const projected = projectConfirmCard(confirmItem, selected);
      assert.strictEqual(projected.state, "approved");

      // 确认卡之后出现的非 rejected/expired 进度卡（含 running/failed/cancelled/interrupted）均按「已批准只读」处理
      const runningOnly: StreamProgressCardItem[] = [
        {
          kind: "progress_card",
          seq: 51,
          run_id: "run-1a17a54dea1-3",
          plan_id: "plan-1a17a56f09d-5",
          step_index: 0,
          tool_id: "create_task",
          status: "running",
          code: null,
          created_task_ids: [],
        },
      ];
      const selectedRunning = findTerminalProgressForPlan(
        runningOnly,
        "plan-1a17a56f09d-5"
      );
      assert.ok(selectedRunning);
      assert.strictEqual(selectedRunning.status, "running");
      const projectedRunning = projectConfirmCard(confirmItem, selectedRunning);
      assert.strictEqual(
        projectedRunning.state,
        "approved",
        "确认卡之后已出现 running 进度卡说明计划已批准执行，必须投影为 approved 只读态"
      );

      for (const postApproveStatus of ["failed", "cancelled", "interrupted"] as const) {
        const projectedPost = projectConfirmCard(confirmItem, {
          ...runningOnly[0],
          status: postApproveStatus,
        });
        assert.strictEqual(
          projectedPost.state,
          "approved",
          `确认卡之后出现 ${postApproveStatus} 进度卡同样按已批准只读处理`
        );
      }
    }
  );

  await suite.test(
    "2. (b) buildConfirmCardTaskModels: 已批准确认卡根据 createdTaskIds 生成任务卡模型（匹配任务与尚未同步时的诚实回退）",
    () => {
      const taskId = "task-unified-65046dba-c55e-4a01-a49f-728eac13d277";
      const matchedModels = buildConfirmCardTaskModels(
        [taskId],
        [makeSampleTask({ task_id: taskId, name: "Sentinel-1D 茂县叠溪下载" })],
        { source: "asf.catalog", estimatedSizeZh: "4.4 GB" }
      );
      assert.strictEqual(matchedModels.length, 1);
      assert.strictEqual(matchedModels[0].taskId, taskId);
      assert.strictEqual(matchedModels[0].task.name, "Sentinel-1D 茂县叠溪下载");
      assert.strictEqual(matchedModels[0].statusLabel, "传输中");

      // 当总览列表尚未同步或单独渲染确认卡时，生成真正的「状态未知」只读回退模型（不猜 status、不带 revision、不显示任务 ID / 原始 resource_id）
      const fallbackModels = buildConfirmCardTaskModels(
        [taskId],
        [],
        { source: "asf.catalog", estimatedSizeZh: "大小未知" }
      );
      assert.strictEqual(fallbackModels.length, 1);
      assert.strictEqual(fallbackModels[0].taskId, taskId);
      assert.strictEqual(
        (fallbackModels[0].task as any).status,
        undefined,
        "回退任务卡模型不得猜测或硬编码 status"
      );
      assert.strictEqual(
        (fallbackModels[0].task as any).revision,
        undefined,
        "回退任务卡模型不得携带编造的 revision"
      );
      assert.deepStrictEqual(
        fallbackModels[0].actions,
        [],
        "回退任务卡模型 actions 必须为空"
      );
      assert.strictEqual(
        fallbackModels[0].productLine,
        null,
        "回退任务卡模型 productLine 不得放置任务 ID"
      );
      assert.strictEqual(
        fallbackModels[0].task.name,
        "下载任务",
        "原始 source ID (asf.catalog) 无显示名映射时标题应只写「下载任务」，不露出 resource_id 或任务 ID"
      );
      assert.ok(
        !fallbackModels[0].task.name.includes(taskId),
        "回退任务卡标题严禁包含任务 ID 原文"
      );
      assert.strictEqual(
        fallbackModels[0].statusLabel,
        "状态未知",
        "回退任务卡模型必须展示「状态未知」"
      );

      // 有来源显示名时：标题 = 「<来源显示名> 下载任务」
      const fallbackWithDisplay = buildConfirmCardTaskModels(
        [taskId],
        [],
        {
          source: "science.sentinel-1.standard.v1",
          sourceNames: { "science.sentinel-1.standard.v1": "Sentinel-1" },
          estimatedSizeZh: "大小未知",
        }
      );
      assert.strictEqual(fallbackWithDisplay[0].task.name, "Sentinel-1 下载任务");
      assert.strictEqual(fallbackWithDisplay[0].productLine, null);
    }
  );

  await suite.test(
    "3. (c) projectPlanStepStatus 与 planStepStatusLabelZh: 终态 done 时步骤显示「已完成」（W5-8 R9，原「已就绪」）而非「待执行」，running 显示「执行中」",
    () => {
      const doneStepStatus = projectPlanStepStatus("pending", "done");
      assert.strictEqual(doneStepStatus, "done");
      assert.strictEqual(planStepStatusLabelZh(doneStepStatus), "已完成");

      const runningStepStatus = projectPlanStepStatus("pending", "running");
      assert.strictEqual(runningStepStatus, "running");
      assert.strictEqual(planStepStatusLabelZh(runningStepStatus), "执行中");

      const failedStepStatus = projectPlanStepStatus("pending", "failed");
      assert.strictEqual(failedStepStatus, "failed");
      assert.strictEqual(planStepStatusLabelZh(failedStepStatus), "失败");

      const cancelledStepStatus = projectPlanStepStatus("pending", "cancelled");
      assert.strictEqual(cancelledStepStatus, "cancelled");
      assert.strictEqual(planStepStatusLabelZh(cancelledStepStatus), "已取消");

      const interruptedStepStatus = projectPlanStepStatus("pending", "interrupted");
      assert.strictEqual(interruptedStepStatus, "interrupted");
      assert.strictEqual(planStepStatusLabelZh(interruptedStepStatus), "已中断");

      const approvedByCodeStepStatus = projectPlanStepStatus("pending", "approved", "plan_approved");
      assert.strictEqual(approvedByCodeStepStatus, "approved");
      assert.strictEqual(planStepStatusLabelZh(approvedByCodeStepStatus), "已批准");

      const rejectedStepStatus = projectPlanStepStatus("pending", "rejected", "plan_rejected");
      assert.strictEqual(rejectedStepStatus, "rejected");
      assert.strictEqual(planStepStatusLabelZh(rejectedStepStatus), "已拒绝");

      const expiredStepStatus = projectPlanStepStatus("pending", "expired", "plan_expired");
      assert.strictEqual(expiredStepStatus, "expired");
      assert.strictEqual(planStepStatusLabelZh(expiredStepStatus), "已失效");

      // 无后续进度时保持原步骤状态
      assert.strictEqual(projectPlanStepStatus("pending", null), "pending");
      assert.strictEqual(planStepStatusLabelZh("pending"), "待执行");
      assert.strictEqual(projectPlanStepStatus("done", null), "done");
      assert.strictEqual(planStepStatusLabelZh("done"), "已完成");
    }
  );

  await suite.test(
    "4. (d) progressStatusLabelZh 与 progressStatusLineZh: 状态映射表全中文，严禁露出英文枚举",
    () => {
      assert.strictEqual(progressStatusLabelZh("running"), "执行中");
      assert.strictEqual(progressStatusLabelZh("done"), "已完成");
      assert.strictEqual(progressStatusLabelZh("failed"), "失败");
      assert.strictEqual(progressStatusLabelZh("cancelled"), "已取消");
      assert.strictEqual(progressStatusLabelZh("interrupted"), "已中断");
      assert.strictEqual(progressStatusLabelZh("queued"), "排队中");
      assert.strictEqual(progressStatusLabelZh("some_unknown_enum"), "未知");
      assert.strictEqual(progressStatusLabelZh(null), "未知");

      assert.strictEqual(
        progressStatusLineZh({ status: "running", code: null }),
        "状态：执行中"
      );
      assert.strictEqual(
        progressStatusLineZh({ status: "done", code: null }),
        "状态：已完成"
      );
      assert.strictEqual(
        progressStatusLineZh({ status: "done", code: "plan_approved" }),
        "执行计划已获批准，正在建立下载任务。"
      );
      assert.strictEqual(
        progressStatusLineZh({ status: "rejected", code: "plan_rejected" }),
        "用户已拒绝执行该计划。"
      );
      assert.strictEqual(
        progressStatusLineZh({ status: "expired", code: "plan_expired" }),
        "计划已过期失效，请重新生成。"
      );
      assert.strictEqual(
        progressStatusLineZh({ status: "unexpected_raw_value", code: null }),
        "状态：未知"
      );
    }
  );

  await suite.test(
    "5. 源码接线断言：DesktopAgentWorkbench 使用 findTerminalProgressForPlan 与 progressStatusLineZh，AgentConfirmCard 渲染 AgentTaskCard",
    () => {
      const agentWorkbenchSrc = fs.readFileSync(
        path.resolve(__dirname, "../../components/desktop/DesktopAgentWorkbench.tsx"),
        "utf8"
      );
      const confirmCardSrc = fs.readFileSync(
        path.resolve(__dirname, "../../components/desktop/AgentConfirmCard.tsx"),
        "utf8"
      );
      const planCardSrc = fs.readFileSync(
        path.resolve(__dirname, "../../components/desktop/AgentPlanCard.tsx"),
        "utf8"
      );

      assert.ok(
        agentWorkbenchSrc.includes("findTerminalProgressForPlan"),
        "DesktopAgentWorkbench.tsx 必须使用 findTerminalProgressForPlan 选取后续进度卡"
      );
      assert.ok(
        !agentWorkbenchSrc.includes("`状态：${item.status}`"),
        "DesktopAgentWorkbench.tsx 严禁直接拼接英文 item.status"
      );
      assert.ok(
        confirmCardSrc.includes("AgentTaskCard"),
        "AgentConfirmCard.tsx 在已批准且有 createdTaskIds 时必须渲染 AgentTaskCard"
      );
      assert.ok(
        planCardSrc.includes("projectPlanStepStatus"),
        "AgentPlanCard.tsx 必须根据 progressStatus 投影步骤完成态"
      );
    }
  );
});
