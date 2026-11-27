import "../../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React, { useState } from "react";
import { render, cleanup, fireEvent } from "../../../test-utils/render-harness";
import { ChannelReasoningField } from "./ChannelReasoningField";
import { ChannelReasoningNote } from "./ChannelReasoningNote";
import { ChannelListItem } from "./ChannelListItem";
import type { AgentReasoningMode, AgentChannelView } from "../../api/agentBridgeTypes";

test.afterEach(() => {
  cleanup();
});

test("ChannelReasoning 交互测试套件", async (suite) => {
  await suite.test("1. ChannelReasoningField 受控往返: 点击选项触发 onChange，受控重渲染后 aria-checked 更新", () => {
    let currentMode: AgentReasoningMode = "off";
    const onChangeCalls: AgentReasoningMode[] = [];

    function TestWrapper() {
      const [mode, setMode] = useState<AgentReasoningMode>("off");
      return (
        <ChannelReasoningField
          value={mode}
          onChange={(val) => {
            onChangeCalls.push(val);
            setMode(val);
            currentMode = val;
          }}
        />
      );
    }

    const { getByRole } = render(<TestWrapper />);

    // 验证 radiogroup 角色与可访问名
    const group = getByRole("radiogroup", { name: "思考" });
    assert.ok(group, "radiogroup 思考 存在");

    const radioOff = getByRole("radio", { name: "关" });
    const radioDefault = getByRole("radio", { name: "默认" });

    // 初始受控值 off
    assert.equal(radioOff.getAttribute("aria-checked"), "true");
    assert.equal(radioDefault.getAttribute("aria-checked"), "false");

    // 点击「默认」
    fireEvent.click(radioDefault);
    assert.equal(onChangeCalls.length, 1);
    assert.equal(onChangeCalls[0], "default");
    assert.equal(currentMode, "default");
    assert.equal(radioOff.getAttribute("aria-checked"), "false");
    assert.equal(radioDefault.getAttribute("aria-checked"), "true");

    // 点击「关」
    fireEvent.click(radioOff);
    assert.equal(onChangeCalls.length, 2);
    assert.equal(onChangeCalls[1], "off");
    assert.equal(currentMode, "off");
    assert.equal(radioOff.getAttribute("aria-checked"), "true");
    assert.equal(radioDefault.getAttribute("aria-checked"), "false");
  });

  await suite.test("2. ChannelReasoningField disabled 状态: 点击时不触发 onChange 回调", () => {
    let called = false;
    const { getByRole } = render(
      <ChannelReasoningField
        value="off"
        disabled={true}
        onChange={() => {
          called = true;
        }}
      />
    );

    const radioDefault = getByRole("radio", { name: "默认" });
    fireEvent.click(radioDefault);
    assert.equal(called, false, "disabled 时不应触发 onChange");
  });

  await suite.test("3. ChannelReasoningNote: refused=true 显示提示文本，refused=false 与缺省时不渲染", () => {
    // a) refused={true} 渲染提示
    const { getByText, unmount } = render(<ChannelReasoningNote refused={true} />);
    assert.ok(getByText("该渠道无法关闭思考"));
    unmount();

    // b) refused={false} 不渲染
    const { container: containerFalse } = render(<ChannelReasoningNote refused={false} />);
    assert.equal(containerFalse.textContent, "");

    // c) 缺省 refused 不渲染
    const { container: containerDefault } = render(<ChannelReasoningNote />);
    assert.equal(containerDefault.textContent, "");
  });

  // Owner 2026-10-29: Task C 将列表项精简为单行，协议/模型与思考行已移至详情面板展示，列表项不再渲染独立思考行与协议模型
  await suite.test("4. ChannelListItem: 单行精简展示，不包含截断的协议/模型与思考行", () => {
    const channel: AgentChannelView = {
      id: "chan-default",
      protocol: "chat_completions",
      base_url: "https://api.openai.com/v1",
      model: "gpt-5",
      reasoning_mode: "default",
      key_ready: true,
      is_default: false,
    };

    const { container, getByText, queryByTestId } = render(
      <ChannelListItem channel={channel} isSelected={false} onSelect={() => {}} />
    );

    const idEl = getByText("chan-default");
    assert.ok(idEl);
    assert.equal(queryByTestId("channel-reasoning-line"), null, "单行列表项中不渲染独立思考行");
    assert.equal(container.textContent?.includes("OpenAI 兼容对话补全"), false, "单行列表项中不渲染协议模型行");
    const cardRoot = container.firstElementChild as HTMLElement;
    assert.equal(cardRoot.classList.contains("rounded-sp-sm"), true, "列表项圆角为 rounded-sp-sm");
    assert.equal(cardRoot.classList.contains("h-9"), true, "列表项为 h-9 单行");
  });

  // Owner 2026-10-29: Task C 将列表项精简为单行，缺省 reasoning_mode 的旧渠道同样单行展示
  await suite.test("5. ChannelListItem: 缺省 reasoning_mode 的旧渠道单行渲染", () => {
    const legacyChannel: AgentChannelView = {
      id: "chan-legacy",
      protocol: "chat_completions",
      base_url: "https://api.openai.com/v1",
      model: "gpt-4o",
      key_ready: false,
      is_default: false,
    };

    const { getByText, queryByTestId } = render(
      <ChannelListItem channel={legacyChannel} isSelected={false} onSelect={() => {}} />
    );

    const idEl = getByText("chan-legacy");
    assert.ok(idEl);
    assert.equal(queryByTestId("channel-reasoning-line"), null);
  });

  // Owner 2026-10-29: Task C 要求单行 truncate 截断（保留 break-all 样式）
  await suite.test("6. ChannelListItem: 40 字符长 id 显示（break-all 与 truncate 单行截断）", () => {
    const longId = "chan-long-identifier-1234567890-abcdefghij";
    assert.ok(longId.length >= 40);
    const channel: AgentChannelView = {
      id: longId,
      protocol: "chat_completions",
      base_url: "https://api.openai.com/v1",
      model: "gpt-4o",
      reasoning_mode: "default",
      key_ready: true,
      is_default: true,
    };

    const { getByText } = render(
      <ChannelListItem channel={channel} isSelected={true} onSelect={() => {}} />
    );

    const idEl = getByText(longId);
    assert.ok(idEl, "应包含完整 40+ 字符长渠道 id");
    assert.equal(idEl.textContent, longId);
    assert.ok(
      idEl.className.includes("break-all"),
      "渠道 id 元素 className 必须包含 break-all"
    );
    assert.ok(
      idEl.className.includes("truncate"),
      "Task C 要求渠道 id 采用 truncate 单行截断"
    );
  });
});
