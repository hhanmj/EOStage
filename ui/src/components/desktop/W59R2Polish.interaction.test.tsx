import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { render, cleanup } from "../../test-utils/render-harness";
import { AgentErrorCard } from "./AgentErrorCard";
import type { StreamErrorItem } from "../../shell/api/agentStreamLogic";

test.afterEach(() => {
  cleanup();
});

const HINT = "该渠道无法关闭思考，输出可能被思考占满；可改用支持关闭思考的模型";

const truncated: StreamErrorItem = {
  kind: "error",
  seq: 40,
  run_id: "run-t",
  code: "model_truncated",
  status: "failed",
  plan_id: null,
};

test("W5-9 条目 6：截断错误卡在渠道拒绝关闭思考时显示说明", async (t) => {
  await t.test("reasoningOffRefused → 显示说明，错误码仍只在详情里", () => {
    const view = render(<AgentErrorCard item={truncated} reasoningOffRefused />);
    assert.ok(view.getByText(HINT));
    assert.equal((view.container.textContent ?? "").includes("model_truncated"), false);
  });
  await t.test("未拒绝 → 不显示说明", () => {
    const view = render(<AgentErrorCard item={truncated} reasoningOffRefused={false} />);
    assert.equal(view.queryByText(HINT) === null, true);
  });
});

// 「未核验」悬停原因已由 W5-10 条目 1 取代（W510R1.interaction.test.tsx）
