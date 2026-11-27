import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import {
  CHANNEL_PROTOCOLS,
  newChannelDraft,
  buildChannelSavePayload,
  projectChannelView,
  formatChannelTestResult,
  fetchChannelModels,
} from "./agentChannelsLogic.ts";
import type { AgentChannelView, AgentChannelTest } from "./agentBridgeTypes.ts";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

test("W4 Slice 10 Channels: Real Backend Command Alignment Suite", async (t) => {
  // -------------------------------------------------------------
  // D1-1: buildChannelSavePayload 校验与载荷构造
  // -------------------------------------------------------------
  await t.test("D1-1: buildChannelSavePayload key input, omission and dirty field stripping", () => {
    // a) 有 Key 输入：带 api_key 且值正确
    const draftWithKey = {
      id: "chan-test-1",
      protocol: "chat_completions",
      base_url: "https://api.openai.com/v1",
      model: "gpt-4o",
      reasoning_mode: "off",
      apiKeyInput: "sk-live-secret-test-key",
      apiKeySaved: false,
      extra_unknown_field: "should_be_stripped",
    };
    const payloadWithKey = buildChannelSavePayload(draftWithKey);
    assert.equal(payloadWithKey.id, "chan-test-1");
    assert.equal(payloadWithKey.protocol, "chat_completions");
    assert.equal(payloadWithKey.base_url, "https://api.openai.com/v1");
    assert.equal(payloadWithKey.model, "gpt-4o");
    assert.equal(payloadWithKey.reasoning_mode, "off");
    assert.equal(payloadWithKey.api_key, "sk-live-secret-test-key");
    assert.equal("extra_unknown_field" in payloadWithKey, false);
    assert.equal("apiKeySaved" in payloadWithKey, false);
    assert.deepEqual(
      Object.keys(payloadWithKey).sort(),
      ["api_key", "base_url", "id", "model", "protocol", "reasoning_mode"].sort()
    );

    // b) 空输入：无 api_key 键（保留后端已存 Key 的语义）
    const draftWithoutKey = {
      id: "chan-test-2",
      protocol: "responses",
      base_url: "https://api.example.com",
      model: "gpt-4o-mini",
      reasoning_mode: "default",
      apiKeyInput: "",
      extra_junk: 123,
    };
    const payloadWithoutKey = buildChannelSavePayload(draftWithoutKey);
    assert.equal(payloadWithoutKey.reasoning_mode, "default");
    assert.equal("api_key" in payloadWithoutKey, false, "未输入密钥时，payload 严禁携带 api_key 键");
    assert.deepEqual(
      Object.keys(payloadWithoutKey).sort(),
      ["base_url", "id", "model", "protocol", "reasoning_mode"].sort()
    );

    // 空格输入也视为无 Key 输入
    const draftWithWhitespaceKey = {
      id: "chan-test-3",
      protocol: "anthropic_messages",
      base_url: "https://api.anthropic.com",
      model: "claude-3-5-sonnet",
      reasoning_mode: "off",
      apiKeyInput: "   ",
    };
    const payloadWhitespace = buildChannelSavePayload(draftWithWhitespaceKey);
    assert.equal("api_key" in payloadWhitespace, false);

    // c) 边界校验返回明确中文错误
    assert.throws(
      () => buildChannelSavePayload({ id: "", protocol: "chat_completions", base_url: "https://a.com", model: "m", reasoning_mode: "off" }),
      /渠道标识不能为空/
    );
    assert.throws(
      () => buildChannelSavePayload({ id: "invalid id with spaces", protocol: "chat_completions", base_url: "https://a.com", model: "m", reasoning_mode: "off" }),
      /渠道标识只能包含英文字母、数字、短横线与下划线/
    );
    assert.throws(
      () => buildChannelSavePayload({ id: "c".repeat(65), protocol: "chat_completions", base_url: "https://a.com", model: "m", reasoning_mode: "off" }),
      /渠道标识长度不能超过 64 个字符/
    );
    assert.throws(
      () => buildChannelSavePayload({ id: "valid-id", protocol: "invalid_proto", base_url: "https://a.com", model: "m", reasoning_mode: "off" }),
      /不支持的接口协议/
    );
    assert.throws(
      () => buildChannelSavePayload({ id: "valid-id", protocol: "chat_completions", base_url: "   ", model: "m", reasoning_mode: "off" }),
      /服务基地址不能为空/
    );
    assert.throws(
      () => buildChannelSavePayload({ id: "valid-id", protocol: "chat_completions", base_url: "https://a.com", model: "   ", reasoning_mode: "off" }),
      /模型标识不能为空/
    );
  });

  // -------------------------------------------------------------
  // D1-2: 协议映射：值集恰为后端支持的四种协议字符串（含 contract 3.43.0 新增 google_gemini）
  // -------------------------------------------------------------
  await t.test("D1-2: protocol mapping exact backend values and label_zh", () => {
    const protocolValues = CHANNEL_PROTOCOLS.map((p) => p.value);
    assert.deepEqual(
      protocolValues.sort(),
      ["anthropic_messages", "chat_completions", "google_gemini", "responses"].sort(),
      "协议四档 value 恰为后端字符串"
    );

    // 严禁存在 openai_chat_completions 等旧前端值
    assert.equal(protocolValues.includes("openai_chat_completions" as any), false);
    assert.equal(protocolValues.includes("openai_responses" as any), false);

    // 协议选项标签需严格带端点路径括号
    assert.deepEqual(
      CHANNEL_PROTOCOLS.map((p) => p.label_zh),
      [
        "Anthropic Messages (/v1/messages)",
        "Chat Completions (/chat/completions)",
        "Google Gemini API (/v1beta)",
        "Responses (/responses)",
      ]
    );
  });

  // -------------------------------------------------------------
  // D1-3: projectChannelView 绝不泄漏/携带任何 Key 字段
  // -------------------------------------------------------------
  await t.test("D1-3: projectChannelView projections and zero-key leakage", () => {
    const backendView1: AgentChannelView = {
      id: "chan-default-1",
      protocol: "chat_completions",
      base_url: "https://api.openai.com/v1",
      model: "gpt-4o",
      key_ready: true,
      is_default: true,
    };
    const projected1 = projectChannelView(backendView1);
    assert.equal(projected1.id, "chan-default-1");
    assert.equal(projected1.is_default, true);
    assert.equal(projected1.key_ready, true);
    assert.equal(projected1.protocol_zh, "Chat Completions (/chat/completions)");
    assert.equal(projected1.key_ready_zh, "已配置密钥");
    assert.equal(projected1.is_default_zh, "当前默认");

    const backendView2: AgentChannelView = {
      id: "chan-standby-2",
      protocol: "anthropic_messages",
      base_url: "https://api.anthropic.com",
      model: "claude-3-5-sonnet",
      key_ready: false,
      is_default: false,
    };
    const projected2 = projectChannelView(backendView2);
    assert.equal(projected2.id, "chan-standby-2");
    assert.equal(projected2.is_default, false);
    assert.equal(projected2.key_ready, false);
    assert.equal(projected2.key_ready_zh, "未配置密钥");
    assert.equal(projected2.is_default_zh, "备用渠道");

    // 结果对象绝对不能存在任何 api_key 字段
    for (const key of Object.keys(projected1)) {
      assert.notEqual(key, "api_key");
      assert.notEqual(key, "apiKey");
      assert.notEqual(key, "key");
    }
    for (const key of Object.keys(projected2)) {
      assert.notEqual(key, "api_key");
      assert.notEqual(key, "apiKey");
      assert.notEqual(key, "key");
    }
  });

  // -------------------------------------------------------------
  // D1-4: formatChannelTestResult 真实后端探测结果映射与无假延迟
  // -------------------------------------------------------------
  await t.test("D1-4: formatChannelTestResult truthfulness and no fake latency", () => {
    // 成功且带模型数
    const okWithCount: AgentChannelTest = {
      ok: true,
      code: null,
      status: null,
      model_count: 8,
    };
    const msgOkCount = formatChannelTestResult(okWithCount);
    assert.match(msgOkCount, /探测成功/);
    assert.match(msgOkCount, /8/);
    assert.equal(msgOkCount.includes("ms"), false, "绝不得编造伪造延迟毫秒数");

    // 成功但无模型数
    const okNoCount: AgentChannelTest = {
      ok: true,
      code: null,
      status: null,
      model_count: null,
    };
    const msgOkNoCount = formatChannelTestResult(okNoCount);
    assert.match(msgOkNoCount, /探测成功/);
    assert.equal(msgOkNoCount.includes("ms"), false);

    // 失败情况 1：缺失密钥
    const failMissingKey: AgentChannelTest = {
      ok: false,
      code: "AGENT_CHANNEL_KEY_MISSING",
      status: null,
      model_count: null,
    };
    const msgFailKey = formatChannelTestResult(failMissingKey);
    assert.match(msgFailKey, /密钥/);
    assert.equal(msgFailKey.includes("ms"), false);

    // 失败情况 2：模型服务不可达并带有 HTTP 状态码
    const failStatus: AgentChannelTest = {
      ok: false,
      code: "AGENT_MODEL_UNAVAILABLE",
      status: 503,
      model_count: null,
    };
    const msgFailStatus = formatChannelTestResult(failStatus);
    assert.match(msgFailStatus, /不可达/);
    assert.match(msgFailStatus, /503/);
    assert.equal(msgFailStatus.includes("ms"), false);
  });

  // -------------------------------------------------------------
  // D1-5: 源码级防回显与无假接入横幅
  // -------------------------------------------------------------
  await t.test("D1-5: source-level honesty checks in ModelChannelsSettingsTab.tsx", () => {
    const tabPath = path.resolve(__dirname, "../components/settings/ModelChannelsSettingsTab.tsx");
    const content = fs.readFileSync(tabPath, "utf-8");

    // 不包含假掩码字符串
    assert.equal(content.includes("•••••"), false, "严禁出现假掩码回显 •••••");
    assert.equal(content.includes("••••••••"), false);

    // 必须清空 Key 输入框
    assert.ok(
      content.includes('setApiKeyInput("")') || content.includes("setApiKeyInput('')"),
      "保存密钥或保存配置成功后必须清空 API Key 输入框"
    );

    // 绝不包含后端接入中旧 stub
    assert.equal(content.includes("后端接入中"), false, "严禁出现后端接入中假 stub");

    // 不做渠道删除按钮（后端无删除命令）
    assert.equal(content.includes("handleDeleteChannel"), false, "严禁提供伪造的前端删除渠道操作");
  });

  // -------------------------------------------------------------
  // D1-6: 桥接实现源码级检查（15 个 agent 命令完整性交叉比对）
  // -------------------------------------------------------------
  await t.test("D1-6: 15 agent commands cross check between tauri bridge and lib.rs", () => {
    const expectedCommands = [
      "agent_sessions_list",
      "agent_sessions_create",
      "agent_sessions_delete",
      "agent_send",
      "agent_stream_poll",
      "agent_plan_approve",
      "agent_plan_reject",
      "agent_run_cancel",
      "agent_runs_list",
      "agent_runs_open",
      "agent_channels_get",
      "agent_channels_save",
      "agent_channels_test",
      "agent_channels_models",
      "agent_channels_set_default",
    ];

    const tauriBridgePath = path.resolve(__dirname, "../../lib/desktopShellBridgeTauri.ts");
    const tauriBridgeContent = fs.readFileSync(tauriBridgePath, "utf-8");

    const libRsPath = path.resolve(__dirname, "../../../../src-tauri/src/lib.rs");
    const libRsContent = fs.readFileSync(libRsPath, "utf-8");

    for (const cmd of expectedCommands) {
      assert.ok(
        tauriBridgeContent.includes(`"${cmd}"`),
        `desktopShellBridgeTauri.ts 必须 invoke "${cmd}"`
      );
      assert.ok(
        libRsContent.includes(cmd),
        `src-tauri/src/lib.rs 必须注册命令 ${cmd}`
      );
    }
  });

  // -------------------------------------------------------------
  // 辅助测试：8 秒超时竞速助手
  // -------------------------------------------------------------
  await t.test("D1-extra: fetchChannelModels timeout and fallback", async () => {
    // 快速成功的调用
    const fastFetcher = async (id: string) => ["gpt-4o", "gpt-4o-mini"];
    const fastRes = await fetchChannelModels(fastFetcher, "chan-1", 1000);
    assert.equal(fastRes.fallbackToManual, false);
    assert.deepEqual(fastRes.models, ["gpt-4o", "gpt-4o-mini"]);

    // 超时的调用
    const slowFetcher = async (id: string) => {
      await new Promise((r) => setTimeout(r, 200));
      return ["slow-model"];
    };
    const slowRes = await fetchChannelModels(slowFetcher, "chan-1", 50);
    assert.equal(slowRes.fallbackToManual, true);
    assert.match(slowRes.message_zh || "", /超时/);
  });
});
