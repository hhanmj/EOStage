import assert from "node:assert/strict";
import test from "node:test";
import { classifyStreamEvents } from "./agentStreamLogic.ts";
import type { AgentStreamEvent } from "../../lib/generated/v3-contracts.ts";
import { explainErrorCode } from "./errorExplanations.ts";
import { projectErrorCard } from "./approvalLogic.ts";

// W3k (L4 r7 P1-4, review P1-2): the host ends some runs by its own decision
// and says why (contract 3.33.0 AgentErrorPayload.message_zh); these used to
// show 「模型服务拒绝了该请求」 or 「执行出错」.
test("W3k: 主机结束运行的错误码与 message_zh", async (suite) => {
  await suite.test("1. 五个主机错误码都有中文说明", () => {
    for (const code of [
      "step_limit",
      "repeated_tool_call",
      "tool_not_offered",
      "tool_repeated_result",
      "outbound_blocked",
    ]) {
      const text = explainErrorCode(code);
      assert.ok(text && /[\u4e00-\u9fff]/.test(text), `${code}: ${text}`);
      assert.notEqual(text, "模型服务拒绝了该请求", code);
    }
  });

  await suite.test("2. classifyStreamEvents 透传 message_zh", () => {
    const events: AgentStreamEvent[] = [
      {
        seq: 1,
        kind: "error",
        payload: {
          run_id: "run-1",
          code: "outbound_blocked",
          status: "failed",
          plan_id: null,
          message_zh: "请求里有无法安全发出的内容，没有发给模型服务。",
          field_class: "tool_result",
        },
      },
      {
        seq: 2,
        kind: "error",
        payload: { run_id: "run-2", code: "model_capability", status: "failed", plan_id: null },
      },
    ];
    const items = classifyStreamEvents(events);
    assert.equal(items[0].kind, "error");
    assert.equal(items[1].kind, "error");
    if (items[0].kind === "error" && items[1].kind === "error") {
      assert.equal(items[0].message_zh, "请求里有无法安全发出的内容，没有发给模型服务。");
      assert.equal(items[1].message_zh ?? null, null);
    }
  });

  await suite.test("3. 错误卡有 message_zh 时优先显示它", () => {
    const card = projectErrorCard({
      code: "some_future_host_code",
      status: "failed",
      message_zh: "主机给出的原因。",
    });
    assert.equal(card.wording, "主机给出的原因。");
    assert.equal(card.detailCode, "some_future_host_code");
    const plain = projectErrorCard({ code: "some_future_host_code", status: "failed" });
    assert.equal(plain.wording, "执行出错");
    const cancelled = projectErrorCard({ code: "cancelled", status: "cancelled", message_zh: "x" });
    assert.equal(cancelled.wording, "已取消");
  });
});
