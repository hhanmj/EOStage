import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { render, cleanup, fireEvent } from "../../test-utils/render-harness";
import { AgentConclusionCard } from "./AgentConclusionCard";
import { AgentErrorCard } from "./AgentErrorCard";
import { AgentToolGroupCard } from "./AgentToolGroupCard";
import type {
  StreamConclusionItem,
  StreamErrorItem,
  StreamToolGroupItem,
} from "../../shell/api/agentStreamLogic";

test.afterEach(() => {
  cleanup();
});

test("W5-2g R6 (2g-9) 交互测试：结论卡提示、重发步骤与截断错误卡", async (suite) => {
  await suite.test(
    "1. 结论卡在 reasoning_off_refused: true 时显示「该渠道无法关闭思考」恰好一次；false/缺省时不显示",
    () => {
      const itemRefused: StreamConclusionItem = {
        kind: "conclusion",
        seq: 1,
        run_id: "run-c1",
        text: "已完成数据检索",
        claims: [],
        reasoning_off_refused: true,
      };

      const view1 = render(<AgentConclusionCard item={itemRefused} />);
      const noteMatches = view1.queryAllByText("该渠道无法关闭思考");
      assert.strictEqual(
        noteMatches.length,
        1,
        "reasoning_off_refused: true 时必须恰好显示一次提示"
      );

      cleanup();

      const itemFalse: StreamConclusionItem = {
        kind: "conclusion",
        seq: 2,
        run_id: "run-c2",
        text: "已完成数据检索",
        claims: [],
        reasoning_off_refused: false,
      };
      const view2 = render(<AgentConclusionCard item={itemFalse} />);
      assert.strictEqual(
        view2.queryByText("该渠道无法关闭思考"),
        null,
        "reasoning_off_refused: false 时不得显示提示"
      );

      cleanup();

      const itemDefault: StreamConclusionItem = {
        kind: "conclusion",
        seq: 3,
        run_id: "run-c3",
        text: "已完成数据检索",
        claims: [],
        reasoning_off_refused: false,
      };
      const view3 = render(<AgentConclusionCard item={itemDefault} />);
      assert.strictEqual(
        view3.queryByText("该渠道无法关闭思考"),
        null,
        "缺省时不显示提示"
      );
    }
  );

  await suite.test(
    "2. 执行过程组展开后最后一行为「已按渠道要求重发（关闭思考被拒绝）」",
    () => {
      const tgItem: StreamToolGroupItem = {
        kind: "tool_group",
        seq: 2,
        run_id: "run-tg-1",
        status: "done",
        rounds: [
          {
            tool_id: "resolve_aoi",
            status: "done",
            started_at: "2026-10-26T10:00:00Z",
            duration_ms: 500,
            result_count: 1,
            summary_zh: "AOI 解析完毕",
          },
        ],
      };

      const view = render(
        <AgentToolGroupCard
          item={tgItem}
          options={{ reasoningResent: true }}
          resolveToolName={() => "解析地理范围"}
        />
      );

      // 默认未展开时点击展开
      const expandBtn = view.getByRole("button", { name: /执行过程/ });
      assert.ok(expandBtn.textContent?.includes("执行过程 2 步"));
      fireEvent.click(expandBtn);

      // 展开后验证最后一行为重发行
      assert.ok(
        view.getByText("已按渠道要求重发（关闭思考被拒绝）"),
        "展开后必须包含重发行文字"
      );
    }
  );

  await suite.test(
    "3. AgentErrorCard 渲染 model_truncated 收起时只有中文解释、展开详情才有「错误代码: model_truncated」",
    () => {
      const errItem: StreamErrorItem = {
        kind: "error",
        seq: 5,
        run_id: "run-err-1",
        code: "model_truncated",
        status: "failed",
        plan_id: null,
      };

      const view = render(<AgentErrorCard item={errItem} />);
      const expectedZh =
        "模型回复超出长度上限，本次运行未生成结论；请缩小范围或换渠道";

      // 收起状态
      assert.ok(view.getByText(expectedZh));
      const cardText = view.container.textContent || "";
      assert.strictEqual(
        /[a-z]+_[a-z_]+/.test(cardText),
        false,
        "收起状态不得出现 snake_case 错误码"
      );

      // 展开详情
      const toggle = view.getByRole("button", { name: "展开详情" });
      fireEvent.click(toggle);

      assert.ok(
        view.getByText("错误代码: model_truncated"),
        "展开后应显示错误代码: model_truncated"
      );
    }
  );
});
