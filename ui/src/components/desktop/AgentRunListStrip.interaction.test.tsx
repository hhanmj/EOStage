import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { render, cleanup, fireEvent } from "../../test-utils/render-harness";
import { AgentRunListStrip } from "./AgentRunListStrip";
import type { AgentRunSummary } from "../../lib/generated/v3-contracts.ts";

test.afterEach(() => {
  cleanup();
});

test("AgentRunListStrip 运行记录条折叠展开与徽章交互测试套件", async (suite) => {
  const mockRuns: AgentRunSummary[] = [
    {
      run_id: "fixture-run-01-secret-id",
      session_id: "test-sess",
      status: "completed",
      started_at: "2026-10-24 20:00:00",
      ended_at: "2026-10-24 20:01:15",
      model_route: "model-route-secret",
    },
    {
      run_id: "fixture-run-02-secret-id",
      session_id: "test-sess",
      status: "failed",
      started_at: "2026-10-24 20:02:00",
      ended_at: "2026-10-24 20:02:45",
      model_route: "model-route-secret",
      reasoning_off_refused: true,
      truncated: true,
    },
  ];

  await suite.test("1. runs 为空时不渲染任何内容", () => {
    const view = render(<AgentRunListStrip runs={[]} />);
    assert.strictEqual(view.container.firstChild, null);
  });

  await suite.test(
    "2. 默认收起显示「运行记录 2」且无徽章；点击切换按钮展开后显示两个状态词与两个徽章文案，不显示 run_id 原文",
    () => {
      const view = render(<AgentRunListStrip runs={mockRuns} />);

      // 默认收起：显示「运行记录 2」
      assert.ok(view.getByText("运行记录 2"));
      // 徽章与状态词在收起态不可见
      assert.strictEqual(view.queryByText("关闭思考被拒"), null);
      assert.strictEqual(view.queryByText("回复被截断"), null);
      assert.strictEqual(view.queryByText("已完成"), null);
      assert.strictEqual(view.queryByText("已失败"), null);

      // 点击切换按钮（可访问名称带条数，title 为「运行记录」，aria-expanded 反映展开态）
      const toggleBtn = view.getByRole("button", { name: "运行记录 2" });
      assert.strictEqual(toggleBtn.getAttribute("title"), "运行记录");
      assert.strictEqual(toggleBtn.getAttribute("aria-expanded"), "false");
      fireEvent.click(toggleBtn);
      assert.strictEqual(toggleBtn.getAttribute("aria-expanded"), "true");

      // 展开后：显示状态词
      assert.ok(view.getByText("已完成"));
      assert.ok(view.getByText("已失败"));

      // 展开后：显示两个徽章
      assert.ok(view.getByText("关闭思考被拒"));
      assert.ok(view.getByText("回复被截断"));

      // 绝不显示 run_id 与 model_route 原文
      assert.strictEqual(view.queryByText("fixture-run-01-secret-id"), null);
      assert.strictEqual(view.queryByText("fixture-run-02-secret-id"), null);
      assert.strictEqual(view.queryByText("model-route-secret"), null);
      assert.ok(
        !view.container.textContent?.includes("fixture-run-01-secret-id"),
        "DOM 文本中不得包含 run_id 原文"
      );
      assert.ok(
        !view.container.textContent?.includes("fixture-run-02-secret-id"),
        "DOM 文本中不得包含 run_id 原文"
      );
    }
  );
});
