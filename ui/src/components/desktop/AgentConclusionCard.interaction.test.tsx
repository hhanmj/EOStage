import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { render, cleanup, fireEvent } from "../../test-utils/render-harness";
import { AgentConclusionCard } from "./AgentConclusionCard";
import type { StreamConclusionItem } from "../../shell/api/agentStreamLogic";

test.afterEach(() => {
  cleanup();
});

test("W5-6 R5 (A): AgentConclusionCard 折叠重复 json 代码块交互测试", async (suite) => {
  await suite.test(
    "1. 带 ```json 块：正文不含 ```json，默认显示「展开详情」(aria-expanded=false)，点击展开显示原始 JSON（含 claims），再点收起",
    () => {
      const itemWithJson: StreamConclusionItem = {
        kind: "conclusion",
        seq: 1,
        run_id: "run-w56-c1",
        text: '检索已完成，共命中 3 景影像。\n```json\n{"text":"检索已完成","claims":[]}\n```',
        claims: [],
      };

      const view = render(<AgentConclusionCard item={itemWithJson} />);
      assert.ok(view.getByText("检索已完成，共命中 3 景影像。"));
      assert.ok(
        !view.container.textContent?.includes("```json"),
        "默认收起时卡片文本不得包含 ```json"
      );

      const expandBtn = view.getByRole("button", { name: "展开详情" });
      assert.strictEqual(expandBtn.getAttribute("aria-expanded"), "false");

      fireEvent.click(expandBtn);

      const collapseBtn = view.getByRole("button", { name: "收起详情" });
      assert.strictEqual(collapseBtn.getAttribute("aria-expanded"), "true");
      assert.ok(
        view.container.textContent?.includes('"claims"'),
        "展开详情后应包含原始 JSON 内容中的 claims"
      );

      fireEvent.click(collapseBtn);
      assert.ok(view.getByRole("button", { name: "展开详情" }));
      assert.ok(
        !view.container.textContent?.includes('"claims"'),
        "再次收起后不应显示原始 JSON 内容"
      );
    }
  );

  await suite.test("2. 无 ```json 块：不渲染「展开详情」按钮，正文原样显示", () => {
    const itemPlain: StreamConclusionItem = {
      kind: "conclusion",
      seq: 2,
      run_id: "run-w56-c2",
      text: "无代码块的普通结论正文",
      claims: [],
    };

    const view = render(<AgentConclusionCard item={itemPlain} />);
    assert.ok(view.getByText("无代码块的普通结论正文"));
    assert.strictEqual(view.queryByRole("button", { name: "展开详情" }), null);
    assert.strictEqual(view.queryByRole("button", { name: "收起详情" }), null);
  });
});
