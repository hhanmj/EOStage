import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { render, cleanup } from "../../test-utils/render-harness";
import { AgentPlanCard } from "./AgentPlanCard";
import type { StreamPlanCardItem } from "../../shell/api/agentStreamLogic";

test.afterEach(() => {
  cleanup();
});

test("AgentPlanCard 交互测试套件", async (suite) => {
  const sourceNames: Record<string, string> = {
    "science.sentinel-1.standard.v1": "Sentinel-1 雷达卫星",
  };

  await suite.test(
    "1. search_start 步骤显示「将向 Sentinel-1 雷达卫星 检索 大同盆地试验区 2024-04-01 至 2024-04-30」",
    async () => {
      const item: StreamPlanCardItem = {
        kind: "plan_card",
        seq: 1,
        run_id: "run-plan-1",
        plan_id: "plan-s1-disc",
        plan_hash: "hash111",
        steps: [
          {
            tool_id: "search_start",
            label_zh: "检索 Sentinel-1 数据",
            tier: 1,
            status: "done",
            disclosure: {
              resource_id: "science.sentinel-1.standard.v1",
              aoi_name: "大同盆地试验区",
              time_range: { start: "2024-04-01", end: "2024-04-30" },
            },
          },
        ],
        approval: {
          decision: "auto_by_mode",
          mode: "confirm_changes",
          card_reasons: [],
        },
        auto_executed: true,
      };

      const { getByText } = render(
        <AgentPlanCard item={item} sourceNames={sourceNames} />
      );

      assert.ok(
        getByText("将向 Sentinel-1 雷达卫星 检索 大同盆地试验区 2024-04-01 至 2024-04-30")
      );
    }
  );

  await suite.test(
    "2. sourceNames 不含时显示「未知数据源」且卡片文字不含 resource_id 原文",
    async () => {
      const item: StreamPlanCardItem = {
        kind: "plan_card",
        seq: 2,
        run_id: "run-plan-2",
        plan_id: "plan-s1-disc-2",
        plan_hash: "hash222",
        steps: [
          {
            tool_id: "search_start",
            label_zh: "检索 Sentinel-1 数据",
            tier: 1,
            status: "done",
            disclosure: {
              resource_id: "secret.unregistered.dataset",
              aoi_name: "大同盆地试验区",
              time_range: { start: "2024-04-01", end: "2024-04-30" },
            },
          },
        ],
      };

      const { getByText, container } = render(
        <AgentPlanCard item={item} sourceNames={{}} />
      );

      assert.ok(
        getByText("将向 未知数据源 检索 大同盆地试验区 2024-04-01 至 2024-04-30")
      );
      assert.ok(!container.textContent?.includes("secret.unregistered.dataset"));
    }
  );

  await suite.test("3. 无 disclosure 的步骤没有披露行", async () => {
    const item: StreamPlanCardItem = {
      kind: "plan_card",
      seq: 3,
      run_id: "run-plan-3",
      plan_id: "plan-s1-disc-3",
      plan_hash: "hash333",
      steps: [
        {
          tool_id: "create_task",
          label_zh: "创建下载任务",
          tier: 2,
          status: "pending",
        },
      ],
    };

    const { getByText, container } = render(
      <AgentPlanCard item={item} sourceNames={sourceNames} />
    );

    assert.ok(getByText("创建下载任务"));
    assert.ok(getByText("待执行"));
    assert.ok(!container.textContent?.includes("将向"));
  });

  await suite.test(
    "4. 「已自动执行」徽章与模式名照旧（模式名为「变更前确认」）",
    async () => {
      const item: StreamPlanCardItem = {
        kind: "plan_card",
        seq: 4,
        run_id: "run-plan-4",
        plan_id: "plan-s1-disc-4",
        plan_hash: "hash444",
        steps: [
          {
            tool_id: "search_start",
            label_zh: "检索 Sentinel-1 数据",
            tier: 1,
            status: "done",
          },
        ],
        approval: {
          decision: "auto_by_mode",
          mode: "confirm_changes",
          card_reasons: [],
        },
        auto_executed: true,
      };

      const { getByText } = render(
        <AgentPlanCard item={item} sourceNames={sourceNames} />
      );

      assert.ok(getByText("已自动执行"));
      assert.ok(getByText("变更前确认"));
    }
  );
  // 5. disclosure 带 display_name_zh
  await suite.test(
    "5. disclosure 带 display_name_zh: 优先使用后端名称显示「将向 Sentinel-1 雷达卫星 检索 大同盆地试验区 2024-04-01 至 2024-04-30」，不含「未知数据源」，不含 resource_id 原文",
    async () => {
      const item: StreamPlanCardItem = {
        kind: "plan_card",
        seq: 5,
        run_id: "run-plan-5",
        plan_id: "plan-s1-disc-5",
        plan_hash: "hash555",
        steps: [
          {
            tool_id: "search_start",
            label_zh: "检索 Sentinel-1 数据",
            tier: 1,
            status: "done",
            disclosure: {
              resource_id: "science.sentinel-1.standard.v1",
              display_name_zh: "Sentinel-1 雷达卫星",
              aoi_name: "大同盆地试验区",
              time_range: { start: "2024-04-01", end: "2024-04-30" },
            },
          },
        ],
      };

      const { getByText, container } = render(
        <AgentPlanCard
          item={item}
          sourceNames={{ "science.sentinel-1.standard.v1": "旧前端映射名" }}
        />
      );

      assert.ok(
        getByText("将向 Sentinel-1 雷达卫星 检索 大同盆地试验区 2024-04-01 至 2024-04-30")
      );
      assert.ok(!container.textContent?.includes("未知数据源"));
      assert.ok(!container.textContent?.includes("science.sentinel-1.standard.v1"));
      assert.ok(!container.textContent?.includes("旧前端映射名"));

    }
  );
  await suite.test(
    "6. W5-8 P2-1: 计划步骤原状态为 pending 时，随终态 progressStatus=done 渲染「已完成」（W5-8 R9，原「已就绪」）而非「待执行」，progressStatus=running 渲染「执行中」",
    async () => {
      const item: StreamPlanCardItem = {
        kind: "plan_card",
        seq: 49,
        run_id: "run-1a17a54dea1-3",
        plan_id: "plan-1a17a56f09d-5",
        plan_hash: "b060b8ab2b17d7fe94bead369c285edfb72a0d4a109bfe60fb6324ce26f69703",
        steps: [
          {
            tool_id: "create_task",
            label_zh: "为最新一景 S1D_IW_SLC__1SDV_20261025T230339 创建下载任务",
            tier: 2,
            status: "pending",
          },
        ],
      };

      const doneRender = render(
        <AgentPlanCard item={item} sourceNames={sourceNames} progressStatus="done" />
      );
      assert.ok(doneRender.getByText("已完成"));
      assert.strictEqual(doneRender.queryByText("待执行"), null);
      doneRender.unmount();

      const runningRender = render(
        <AgentPlanCard item={item} sourceNames={sourceNames} progressStatus="running" />
      );
      assert.ok(runningRender.getByText("执行中"));
      assert.strictEqual(runningRender.queryByText("待执行"), null);
    }
  );

  await suite.test("7. W5-8 R7: 计划卡不显示内部计划 ID", async () => {
    const item: StreamPlanCardItem = {
      kind: "plan_card",
      seq: 6,
      run_id: "run-plan-6",
      plan_id: "plan-s1-disc",
      plan_hash: "hash666",
      steps: [
        {
          tool_id: "search_start",
          label_zh: "检索 Sentinel-1 数据",
          tier: 1,
          status: "done",
        },
      ],
      approval: {
        decision: "auto_by_mode",
        mode: "confirm_changes",
        card_reasons: [],
      },
      auto_executed: true,
    };

    const { container } = render(
      <AgentPlanCard item={item} sourceNames={sourceNames} />
    );

    const text = container.textContent ?? "";
    assert.ok(
      !text.includes("plan-"),
      `计划卡不得显示内部计划 ID，实际文本：${text}`
    );
    assert.ok(!text.includes("run-plan-6"), "计划卡不得显示运行 ID");
  });
});
