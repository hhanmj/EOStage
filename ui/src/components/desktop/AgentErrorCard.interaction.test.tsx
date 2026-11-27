import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { render, cleanup, fireEvent } from "../../test-utils/render-harness";
import { AgentErrorCard } from "./AgentErrorCard";
import type { StreamErrorItem } from "../../shell/api/agentStreamLogic";

test.afterEach(() => {
  cleanup();
});

const SNAKE = /[a-z]+_[a-z_]+/;

function errorItem(code: string, approval: StreamErrorItem["approval"] = null, status: string | null = null): StreamErrorItem {
  return { kind: "error", seq: 9, run_id: "run-1", code, status, plan_id: null, approval };
}

test("W5-2f R7 错误卡不在可见文字里显示错误码", async (suite) => {
  const cases: Array<[string, StreamErrorItem, string]> = [
    [
      "禁止档",
      errorItem("plan_refused_forbidden", { decision: "refused_forbidden", mode: "full_access", card_reasons: [] }),
      "该操作属于禁止档，任何模式都不执行",
    ],
    [
      "W3e 旧实际超额事件",
      errorItem("quota_exceeded_actual", { decision: "auto_by_mode", mode: "within_quota", exceeded: ["max_bytes_actual"] } as never),
      "实际下载量超过本次运行字节上限的 110%，Agent 建的任务已暂停",
    ],
    ["记录失败", errorItem("record_failed", null, "failed"), "运行记录写入失败，本次运行已终止"],
    ["未知码", errorItem("some_unknown_code"), "执行出错"],
  ];

  for (const [name, item, wording] of cases) {
    await suite.test(`${name}：收起时只有中文说明，展开详情才显示错误代码`, () => {
      const view = render(<AgentErrorCard item={item} />);
      const card = view.container;
      assert.ok(view.getByText(wording));
      assert.ok(!SNAKE.test(card.textContent || ""), `收起状态的卡片文字含 snake_case：${card.textContent}`);
      const toggle = view.getByRole("button", { name: "展开详情" });
      assert.strictEqual(toggle.getAttribute("aria-expanded"), "false");
      fireEvent.click(toggle);
      assert.ok(view.getByText(`错误代码: ${item.code}`));
      assert.ok(view.getByRole("button", { name: "收起详情" }));
    });
  }

  await suite.test("禁止档显示当前模式中文名；W3e 旧实际超额事件不再列超额项（W5-2g 删除额度）", () => {
    const v1 = render(<AgentErrorCard item={cases[0][1]} />);
    assert.ok(v1.getByText("当前模式：完全访问"));
    cleanup();
    const v2 = render(<AgentErrorCard item={cases[1][1]} />);
    assert.strictEqual(v2.queryByText("实际超额 110%"), null);
    assert.ok(!(v2.container.textContent || "").includes("额度"));
  });

  await suite.test("已取消：无错误代码，也无展开详情", () => {
    const view = render(<AgentErrorCard item={errorItem("cancelled", null, "cancelled")} />);
    assert.ok(view.getByText("已取消"));
    assert.strictEqual(view.queryByRole("button", { name: "展开详情" }), null);
  });
});
