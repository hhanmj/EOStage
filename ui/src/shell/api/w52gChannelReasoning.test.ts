import assert from "node:assert/strict";
import test from "node:test";
import {
  newChannelDraft,
  channelDraftFromView,
  buildChannelSavePayload,
  projectChannelView,
  reasoningOffRefusedNote,
} from "./agentChannelsLogic.ts";
import type { AgentChannelView } from "./agentBridgeTypes.ts";

test("W5-2g R4: 模型渠道思考开关（纯逻辑与往返测试）", async (suite) => {
  await suite.test("1. newChannelDraft: 新建草稿的 reasoning_mode 默认为 'off'", () => {
    const draft = newChannelDraft("test-new");
    assert.equal(draft.reasoning_mode, "off");
  });

  await suite.test("2. 往返链路 A: 渠道视图 default -> 草稿改为 off -> 保存载荷 off -> 回显视图 -> 显示「关」", () => {
    const viewDefault: AgentChannelView = {
      id: "chan-default",
      protocol: "chat_completions",
      base_url: "https://api.openai.com/v1",
      model: "gpt-5",
      reasoning_mode: "default",
      key_ready: true,
      is_default: false,
    };

    // 从视图读入草稿
    const draft = channelDraftFromView(viewDefault);
    assert.equal(draft.reasoning_mode, "default");

    // 用户在界面切换为 off
    draft.reasoning_mode = "off";

    // 构建保存载荷
    const payload = buildChannelSavePayload(draft);
    assert.equal(payload.reasoning_mode, "off");

    // 假装后端保存成功后返回回显视图
    const echoedView: AgentChannelView = {
      ...viewDefault,
      reasoning_mode: payload.reasoning_mode!,
    };

    const display = projectChannelView(echoedView);
    assert.equal(display.reasoning_mode, "off");
    assert.equal(display.reasoning_mode_zh, "关");
  });

  await suite.test("3. 往返链路 B: 渠道视图 off -> 草稿改为 default -> 保存载荷 default -> 回显视图 -> 显示「默认」", () => {
    const viewOff: AgentChannelView = {
      id: "chan-off",
      protocol: "chat_completions",
      base_url: "https://api.openai.com/v1",
      model: "glm-4-plus",
      reasoning_mode: "off",
      key_ready: true,
      is_default: false,
    };

    const draft = channelDraftFromView(viewOff);
    assert.equal(draft.reasoning_mode, "off");

    draft.reasoning_mode = "default";

    const payload = buildChannelSavePayload(draft);
    assert.equal(payload.reasoning_mode, "default");

    const echoedView: AgentChannelView = {
      ...viewOff,
      reasoning_mode: payload.reasoning_mode!,
    };

    const display = projectChannelView(echoedView);
    assert.equal(display.reasoning_mode, "default");
    assert.equal(display.reasoning_mode_zh, "默认");
  });

  await suite.test("4. 旧渠道视图容错: 无 reasoning_mode 字段时，草稿读作 off，列表显示「关」", () => {
    const legacyView: AgentChannelView = {
      id: "chan-legacy",
      protocol: "chat_completions",
      base_url: "https://api.openai.com/v1",
      model: "gpt-4o",
      key_ready: true,
      is_default: true,
    };

    const draft = channelDraftFromView(legacyView);
    assert.equal(draft.reasoning_mode, "off");

    const display = projectChannelView(legacyView);
    assert.equal(display.reasoning_mode, "off");
    assert.equal(display.reasoning_mode_zh, "关");
  });

  await suite.test("5. buildChannelSavePayload 非法值抛错: 不支持的思考设置", () => {
    const base = {
      id: "chan-val",
      protocol: "chat_completions",
      base_url: "https://api.openai.com/v1",
      model: "gpt-4o",
    };

    assert.throws(
      () => buildChannelSavePayload({ ...base, reasoning_mode: "invalid" as any }),
      /不支持的思考设置/
    );
    assert.throws(
      () => buildChannelSavePayload({ ...base, reasoning_mode: "on" as any }),
      /不支持的思考设置/
    );
    assert.throws(
      () => buildChannelSavePayload({ ...base, reasoning_mode: "" as any }),
      /不支持的思考设置/
    );
    assert.throws(
      () => buildChannelSavePayload({ ...base, reasoning_mode: null as any }),
      /不支持的思考设置/
    );
    assert.throws(
      () => buildChannelSavePayload({ ...base, reasoning_mode: undefined as any }),
      /不支持的思考设置/
    );
  });

  await suite.test("6. reasoningOffRefusedNote: true 返回提示，false/undefined/null 返回 null", () => {
    assert.equal(reasoningOffRefusedNote(true), "该渠道无法关闭思考");
    assert.equal(reasoningOffRefusedNote(false), null);
    assert.equal(reasoningOffRefusedNote(undefined), null);
    assert.equal(reasoningOffRefusedNote(null), null);
  });
});
