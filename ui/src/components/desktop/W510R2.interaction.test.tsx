import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { act } from "@testing-library/react";
import { render, cleanup, fireEvent } from "../../test-utils/render-harness";
import { ChannelListItem } from "../../shell/components/settings/ChannelListItem";
import { ChannelReasoningField } from "../../shell/components/settings/ChannelReasoningField";
import { TaskDetailView } from "./DesktopDownloadDrawer";
import { AgentConclusionCard } from "./AgentConclusionCard";
import { AgentRunListStrip } from "./AgentRunListStrip";
import type { AgentChannelView, AgentRunSummary } from "../../lib/generated/v3-contracts";
import type { ApplicationTask } from "../../shell/api/types";
import type { StreamConclusionItem } from "../../shell/api/agentStreamLogic";

test.afterEach(() => {
  cleanup();
});

async function settle(ms = 30) {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, ms));
  });
}

function channel(overrides: Partial<AgentChannelView> = {}): AgentChannelView {
  return {
    id: "l4-vectorport-glm",
    protocol: "chat_completions",
    base_url: "https://example.invalid/v1",
    model: "glm-5.3-flash",
    reasoning_mode: "off",
    key_ready: true,
    is_default: true,
    ...overrides,
  };
}

const NOTE = "该渠道不支持关闭思考，之后的请求不再发送思考设置，选「关」也按服务商默认处理；更改协议、地址或模型并保存后该标志会清除";

test("W5-10 条目 4：渠道卡与思考开关说明「不支持关闭思考」", async (t) => {
  // Owner 2026-10-29: Task C 将列表项精简为单行，从列表项中移除了思考行与 chips（在右侧详情中展示），验证列表项单行渲染且不包含思考标签
  await t.test("渠道卡：Task C 单行精简，移除了思考行与 chips", () => {
    const view = render(
      <ChannelListItem channel={channel({ reasoning_switch_unsupported: true })} isSelected={false} onSelect={() => {}} />,
    );
    assert.equal(view.queryByText("该渠道不支持关闭思考"), null);
    assert.ok(view.getByText("l4-vectorport-glm"));
  });

  // Owner 2026-10-29: Task C 单行模式下，卡片由原三行精简为单行，不再包含 channel-reasoning-line
  await t.test("R3：列表项为单行圆角卡片，不包含 channel-reasoning-line", () => {
    const view = render(
      <ChannelListItem
        channel={channel({ reasoning_mode: "default", reasoning_switch_unsupported: true })}
        isSelected={false}
        onSelect={() => {}}
      />,
    );
    assert.equal(view.queryByTestId("channel-reasoning-line"), null);
    const cardRoot = view.container.firstElementChild as HTMLElement;
    assert.equal(cardRoot.classList.contains("rounded-sp-sm"), true);
    assert.equal(cardRoot.classList.contains("h-9"), true);
  });

  await t.test("渠道卡：未标记 → 没有标签", () => {
    const view = render(
      <ChannelListItem channel={channel({ reasoning_switch_unsupported: false })} isSelected={false} onSelect={() => {}} />,
    );
    assert.equal(view.queryByText("该渠道不支持关闭思考") === null, true);
  });

  await t.test("思考开关旁说明", () => {
    const view = render(<ChannelReasoningField value="off" onChange={() => {}} switchUnsupported />);
    assert.ok(view.getByText(NOTE));
  });

  await t.test("未标记 → 不显示说明", () => {
    const view = render(<ChannelReasoningField value="off" onChange={() => {}} />);
    assert.equal(view.queryByText(NOTE) === null, true);
  });
});

function task(): ApplicationTask {
  return {
    task_id: "task-unified-6adce9d6-d405-4caa-aaab-c9be6b2f3016",
    revision: 3,
    category: "scene_download",
    name: "Sentinel-1",
    provider: "asf",
    dataset: "s1_slc",
    status: "cancelled",
    items_count: 1,
    done_bytes: 0,
    total_bytes: null,
    created_at: "2026-10-27T04:05:31Z",
    updated_at: "2026-10-27T04:07:00Z",
  } as ApplicationTask;
}

test("W5-10 条目 5：任务标识收进「技术信息」并可复制", async (t) => {
  await t.test("默认收起：看不到任务标识", () => {
    const view = render(<TaskDetailView task={task()} />);
    const toggle = view.getByRole("button", { name: /技术信息/ });
    assert.equal(toggle.getAttribute("aria-expanded"), "false");
    assert.equal(view.queryByText(task().task_id) === null, true);
  });

  await t.test("展开后显示并可复制", async () => {
    const copied: string[] = [];
    const nav = globalThis.navigator as Navigator & { clipboard?: unknown };
    const original = Object.getOwnPropertyDescriptor(nav, "clipboard");
    Object.defineProperty(nav, "clipboard", {
      configurable: true,
      value: {
        writeText: async (text: string) => {
          copied.push(text);
        },
      },
    });
    try {
      const view = render(<TaskDetailView task={task()} />);
      fireEvent.click(view.getByRole("button", { name: /技术信息/ }));
      assert.ok(view.getByText("任务标识"));
      assert.ok(view.getByText(task().task_id));
      fireEvent.click(view.getByRole("button", { name: "复制任务标识" }));
      await settle();
      assert.deepEqual(copied, [task().task_id]);
      assert.ok(view.getByText("已复制"));
    } finally {
      if (original) Object.defineProperty(nav, "clipboard", original);
      else delete (nav as { clipboard?: unknown }).clipboard;
    }
  });
});

function conclusion(refused: boolean): StreamConclusionItem {
  return { kind: "conclusion", seq: 30, run_id: "run-c", text: "结论", claims: [], reasoning_off_refused: refused };
}

function run(refused: boolean): AgentRunSummary {
  return {
    run_id: "run-c",
    session_id: "s",
    status: "completed",
    started_at: "2026-10-27 15:00:00",
    ended_at: "2026-10-27 15:01:00",
    model_route: null,
    reasoning_off_refused: refused,
    truncated: false,
  };
}

test("W5-10 条目 6：「无法关闭思考」只在运行标记为 true 时出现", async (t) => {
  await t.test("结论卡", () => {
    const off = render(<AgentConclusionCard item={conclusion(false)} />);
    assert.equal(off.queryByText("该渠道无法关闭思考") === null, true);
    cleanup();
    const on = render(<AgentConclusionCard item={conclusion(true)} />);
    assert.ok(on.getByText("该渠道无法关闭思考"));
  });

  await t.test("运行记录条", () => {
    const off = render(<AgentRunListStrip defaultExpanded runs={[run(false)]} />);
    assert.equal(off.queryByText("关闭思考被拒") === null, true);
    cleanup();
    const on = render(<AgentRunListStrip defaultExpanded runs={[run(true)]} />);
    assert.ok(on.getByText("关闭思考被拒"));
  });
});
