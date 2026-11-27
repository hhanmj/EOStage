import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { cleanup, fireEvent, render } from "../../test-utils/render-harness";
import { waitFor } from "@testing-library/react";
import { registerHooks } from "node:module";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { transformSync } from "esbuild";
import {
  clearTestDesktopShellBridge,
  registerTestDesktopShellBridge,
} from "@/lib/desktopShellBridge";
import { createWorkbenchTestDesktopShellBridge } from "../../test-utils/workbenchDesktopBridge";
import type { DesktopAgentBridge } from "@/shell/api/agentBridgeTypes";
import type { AgentChannelSave, AgentChannelView, AgentSendContext } from "@/lib/generated/v3-contracts";
import { __setEOStageAccountTestAdapter } from "@/shell/api/eostageAccount";

// tsx does not provide Vite's import.meta.env. Patch only the modules loaded by this test.
const scopedSource = (url: string) => `import.meta.env ??= { DEV: true };\n${readFileSync(fileURLToPath(url), "utf8")}`;
const hooks = registerHooks({
  load(url, context, nextLoad) {
    if (url.endsWith("/ModelChannelsSettingsTab.tsx")) {
      return { format: "module", shortCircuit: true, source: transformSync(scopedSource(url), { loader: "tsx", format: "esm" }).code };
    }
    if (url.endsWith("/DesktopAgentWorkbench.tsx")) {
      return { format: "module", shortCircuit: true, source: transformSync(scopedSource(url), { loader: "tsx", format: "esm" }).code };
    }
    return nextLoad(url, context);
  },
});
const { ModelChannelsSettingsTab } = await import("@/shell/components/settings/ModelChannelsSettingsTab");
const { DesktopAgentWorkbench } = await import("./DesktopAgentWorkbench");
hooks.deregister();

const channel: AgentChannelView = {
  id: "main",
  protocol: "chat_completions",
  base_url: "https://example.invalid/v1",
  model: "gpt-default",
  reasoning_mode: "default",
  models: [
    { id: "gpt-default", name: "GPT", context_window: 128000, input_types: ["text"], reasoning_levels: [] },
    { id: "o3", input_types: ["text", "image"], reasoning_levels: [{ level: "low", value: "low" }, { level: "high", value: "high" }] },
  ],
  key_ready: true,
  is_default: true,
};

function installBridge(
  overrides: Partial<DesktopAgentBridge>,
  credentialOverrides?: Partial<ReturnType<typeof createWorkbenchTestDesktopShellBridge>["credentials"]>
) {
  window.localStorage.setItem("eostage.modelSettings.lastChannelId", "main");
  const base = createWorkbenchTestDesktopShellBridge();
  const agent = {
    channelsGet: async () => [channel],
    channelsModels: async () => ["gpt-default", "gpt-new"],
    channelsProbeDraft: async () => ({ result: { ok: true, code: null, status: null, model_count: 2 }, models: ["gpt-default", "gpt-new"] }),
    channelsSave: async () => [channel],
    sessionsList: async () => [{ session_id: "session-1", title: "模型测试", created_at: "2026-10-30T00:00:00Z", updated_at: "2026-10-30T00:00:00Z", run_ids: [] }],
    streamPoll: async () => ({ events: [], cursor: 0 }),
    runsList: async () => [{ run_id: "finished-run", session_id: "session-1", status: "completed", started_at: "2026-10-30T00:00:00Z", ended_at: "2026-10-30T00:00:01Z" }],
    permissionModeGet: async () => ({ effective: "confirm_changes", session_override: null, global_default: "confirm_changes" }),
    ...overrides,
  } as DesktopAgentBridge;
  registerTestDesktopShellBridge({
    ...base,
    agent,
    credentials: {
      ...base.credentials,
      ...credentialOverrides,
    },
  });
}

test.afterEach(() => {
  cleanup();
  __setEOStageAccountTestAdapter(null);
  clearTestDesktopShellBridge();
  window.localStorage.clear();
});

test("unsaved draft probes the visible URL and key without saving", async () => {
  const probes: Parameters<DesktopAgentBridge["channelsProbeDraft"]>[0][] = [];
  let saves = 0;
  installBridge({
    channelsProbeDraft: async (draft) => {
      probes.push(draft);
      return { result: { ok: true, code: null, status: null, model_count: 1 }, models: ["fixture-model"] };
    },
    channelsSave: async () => { saves++; return [channel]; },
  });
  const view = render(<ModelChannelsSettingsTab />);
  fireEvent.click(await waitFor(() => view.getByRole("button", { name: "新建渠道" })));
  fireEvent.change(view.getByRole("textbox", { name: "服务基地址" }), { target: { value: "https://draft.example.invalid/v1" } });
  fireEvent.change(view.getByLabelText("API 密钥"), { target: { value: "fixture-only" } });
  fireEvent.click(view.getByRole("button", { name: "更多供应商操作" }));
  fireEvent.click(view.getByRole("button", { name: "测试连接端点" }));
  await waitFor(() => assert.equal(probes.length, 1));
  assert.equal(probes[0].id, null);
  assert.equal(probes[0].base_url, "https://draft.example.invalid/v1");
  assert.equal(probes[0].api_key, "fixture-only");
  fireEvent.click(view.getByRole("button", { name: "获取可用模型" }));
  await waitFor(() => assert.equal(probes.length, 2));
  assert.ok(await waitFor(() => view.getByRole("checkbox", { name: "选择模型 fixture-model" })));
  assert.equal(saves, 0);
});

test("late draft probes do not update an edited channel", async () => {
  let finish!: (value: Awaited<ReturnType<DesktopAgentBridge["channelsProbeDraft"]>>) => void;
  installBridge({ channelsProbeDraft: () => new Promise((resolve) => { finish = resolve; }) });
  const view = render(<ModelChannelsSettingsTab />);
  const url = await waitFor(() => view.getByRole("textbox", { name: "服务基地址" }));
  fireEvent.click(view.getByRole("button", { name: "更多供应商操作" }));
  fireEvent.click(view.getByRole("button", { name: "测试连接端点" }));
  fireEvent.change(url, { target: { value: "https://other.example.invalid/v1" } });
  finish({ result: { ok: true, code: null, status: null, model_count: 1 }, models: ["stale-model"] });
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(view.queryByText(/端点探测成功/), null);

  fireEvent.click(view.getByRole("button", { name: "获取可用模型" }));
  fireEvent.change(url, { target: { value: "https://new.example.invalid/v1" } });
  finish({ result: { ok: true, code: null, status: null, model_count: 1 }, models: ["stale-model"] });
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(view.queryByText("stale-model"), null);
  assert.equal(view.queryByRole("dialog", { name: "获取可用模型" }), null);
});

test("获取可用模型 merges a checked model and save sends models", async () => {
  const saves: AgentChannelSave[] = [];
  installBridge({ channelsSave: async (payload) => { saves.push(payload); return [{ ...channel, models: payload.models ?? [] }]; } });
  const view = render(<ModelChannelsSettingsTab />);
  const fetch = await waitFor(() => view.getByRole("button", { name: "获取可用模型" }));
  fireEvent.click(fetch);
  const checkbox = await waitFor(() => view.getByRole("checkbox", { name: "选择模型 gpt-new" }));
  await waitFor(() => assert.equal((view.getByRole("button", { name: "更新列表" }) as HTMLButtonElement).disabled, false));
  await waitFor(() => assert.equal((checkbox as HTMLInputElement).checked, true));
  fireEvent.click(view.getByRole("button", { name: "更新列表" }));
  assert.ok(view.getByText("gpt-new"));
  await waitFor(() => assert.equal(saves.length, 1));
  assert.equal(saves[0].model, "gpt-default");
  assert.deepEqual(saves[0].models?.map((model) => model.id), ["gpt-default", "o3", "gpt-new"]);
  assert.equal(saves[0].models?.find((model) => model.id === "gpt-default")?.context_window, 128000);
});

test("model dialog applies parameters to the first model, presets, and keeps the default payload", async () => {
  const saves: AgentChannelSave[] = [];
  installBridge({ channelsSave: async (payload) => { saves.push(payload); return [{ ...channel, model: payload.model, models: payload.models ?? [] }]; } });
  const view = render(<ModelChannelsSettingsTab />);
  const gear = await waitFor(() => view.getByRole("button", { name: "设置模型 GPT" }));
  fireEvent.click(gear);
  const dialog = view.getByRole("dialog", { name: "GPT" });
  assert.ok(dialog);
  const context = view.getByRole("spinbutton", { name: "模型 1 上下文窗口" });
  fireEvent.change(context, { target: { value: "256000" } });
  fireEvent.change(view.getByRole("textbox", { name: "模型 1 显示名称" }), { target: { value: "GPT 5" } });
  fireEvent.click(view.getByRole("checkbox", { name: "模型 1 图像输入" }));
  fireEvent.click(view.getByRole("button", { name: "应用预设" }));
  fireEvent.click(view.getByRole("button", { name: "完成" }));
  assert.equal(view.queryByRole("dialog", { name: "GPT 5" }), null);
  assert.equal(document.activeElement, gear);
  await waitFor(() => assert.equal(saves.length, 1));
  assert.equal(saves[0].model, "gpt-default");
  assert.equal(saves[0].models?.[0].name, "GPT 5");
  assert.equal(saves[0].models?.[0].context_window, 256000);
  assert.deepEqual(saves[0].models?.[0].input_types, ["text", "image"]);
  assert.deepEqual(saves[0].models?.[0].reasoning_levels, [
    { level: "off", value: null },
    { level: "low", value: "low" },
    { level: "medium", value: "medium" },
    { level: "high", value: "high" },
  ]);
});

test("model dialog cancel and Escape discard edits, and moving the first model updates the default", async () => {
  const saves: AgentChannelSave[] = [];
  installBridge({ channelsSave: async (payload) => { saves.push(payload); return [{ ...channel, model: payload.model, models: payload.models ?? [] }]; } });
  const view = render(<ModelChannelsSettingsTab />);
  const gear = await waitFor(() => view.getByRole("button", { name: "设置模型 GPT" }));
  fireEvent.click(gear);
  fireEvent.change(view.getByRole("textbox", { name: "模型 1 标识" }), { target: { value: "discarded" } });
  fireEvent.keyDown(document, { key: "Escape" });
  assert.ok(view.getByTestId("model-unsaved-confirm-dialog"));
  fireEvent.click(view.getByTestId("model-unsaved-confirm-discard"));
  assert.equal(view.queryByRole("dialog", { name: "GPT" }), null);
  assert.equal(document.activeElement, gear);
  assert.ok(view.queryByText("discarded") === null);

  fireEvent.click(gear);
  fireEvent.change(view.getByRole("textbox", { name: "模型 1 标识" }), { target: { value: "also-discarded" } });
  fireEvent.click(view.getByRole("button", { name: "取消" }));
  assert.ok(view.getByTestId("model-unsaved-confirm-dialog"));
  fireEvent.click(view.getByTestId("model-unsaved-confirm-discard"));
  assert.ok(view.queryByText("also-discarded") === null);

  fireEvent.click(view.getByRole("button", { name: "设置模型 o3" }));
  fireEvent.click(view.getByRole("button", { name: "上移模型 2" }));
  fireEvent.click(view.getByRole("button", { name: "完成" }));
  await waitFor(() => assert.equal(saves.length, 1));
  assert.equal(saves[0].model, "o3");
  assert.deepEqual(saves[0].models?.map((model) => model.id), ["o3", "gpt-default"]);
});

test("add model opens its dialog and save retains the default model", async () => {
  const saves: AgentChannelSave[] = [];
  installBridge({ channelsSave: async (payload) => { saves.push(payload); return [{ ...channel, model: payload.model, models: payload.models ?? [] }]; } });
  const view = render(<ModelChannelsSettingsTab />);
  fireEvent.click(await waitFor(() => view.getByRole("button", { name: "添加模型" })));
  fireEvent.change(view.getByRole("textbox", { name: "模型 3 标识" }), { target: { value: "gpt-added" } });
  fireEvent.click(view.getByRole("button", { name: "完成" }));
  await waitFor(() => assert.equal(saves.length, 1));
  assert.equal(saves[0].model, "gpt-default");
  assert.deepEqual(saves[0].models?.map((model) => model.id), ["gpt-default", "o3", "gpt-added"]);
});
test("typing recognized model id auto-fills preset parameters into inline editor", async () => {
  const saves: AgentChannelSave[] = [];
  installBridge({
    channelsSave: async (payload) => {
      saves.push(payload);
      return [{ ...channel, model: payload.model, models: payload.models ?? [] }];
    },
  });
  const view = render(<ModelChannelsSettingsTab />);
  fireEvent.click(await waitFor(() => view.getByRole("button", { name: "添加模型" })));
  const idInput = view.getByRole("textbox", { name: "模型 3 标识" });
  fireEvent.change(idInput, { target: { value: "deepseek-reasoner" } });

  assert.equal((view.getByRole("spinbutton", { name: "模型 3 上下文窗口" }) as HTMLInputElement).value, "131072");
  assert.equal((view.getByRole("textbox", { name: "模型 3 显示名称" }) as HTMLInputElement).value, "DeepSeek R1");
  assert.equal((view.getByRole("checkbox", { name: "模型 3 支持工具调用" }) as HTMLInputElement).checked, true);

  fireEvent.click(view.getByRole("button", { name: "完成" }));
  await waitFor(() => assert.equal(saves.length, 1));
  const savedR1 = saves[0].models?.find((m) => m.id === "deepseek-reasoner");
  assert.ok(savedR1);
  assert.equal(savedR1?.context_window, 131072);
  assert.equal(savedR1?.name, "DeepSeek R1");
  assert.equal(savedR1?.supports_tools, true);
  assert.ok((savedR1?.reasoning_levels?.length ?? 0) >= 2);
});

test("A2 + B1: floating speed-test HUD distinguishes per-row model plug vs channel default test", async () => {
  const testedDrafts: Parameters<NonNullable<DesktopAgentBridge["channelsTestDraft"]>>[0][] = [];
  installBridge({
    channelsTestDraft: async (draft) => {
      testedDrafts.push(draft);
      return {
        ok: true,
        code: null,
        status: 200,
        model_count: 2,
        latency_ms: 142,
        message: `pong from ${draft.model}`,
      };
    },
  });
  const view = render(<ModelChannelsSettingsTab />);
  const rowPlug = await waitFor(() => view.getByRole("button", { name: "测试模型 o3" }));
  fireEvent.click(rowPlug);
  await waitFor(() => assert.equal(testedDrafts.length, 1));
  assert.equal(testedDrafts[0].model, "o3");

  const hud = await waitFor(() => view.getByTestId("channel-speed-test-hud"));
  assert.equal(hud.parentElement?.className.includes("fixed bottom-6 right-6"), true);
  assert.equal(view.getByTestId("speed-test-hud-model").textContent, "o3");
  assert.ok(hud.textContent?.includes("HTTP 200"));
  assert.ok(hud.textContent?.includes("142"));
  assert.ok(hud.textContent?.includes("pong from o3"));

  // Top-right `...` menu tests the channel default model (`gpt-default`)
  fireEvent.click(view.getByRole("button", { name: "更多供应商操作" }));
  fireEvent.click(view.getByTestId("provider-menu-test-connection-btn"));
  await waitFor(() => assert.equal(testedDrafts.length, 2));
  assert.equal(testedDrafts[1].model, "gpt-default");
  await waitFor(() => assert.equal(view.getByTestId("speed-test-hud-model").textContent, "默认 · GPT (gpt-default)"));

  fireEvent.click(view.getByTestId("close-speed-test-hud-btn"));
  assert.equal(view.queryByTestId("channel-speed-test-hud"), null);
});

test("A3-model: saved channel populates API key from credentials, supports eye reveal, and preserves key on save", async () => {
  const saves: AgentChannelSave[] = [];
  const readIds: string[] = [];
  installBridge(
    {
      channelsSave: async (payload) => {
        saves.push(payload);
        return [{ ...channel, base_url: payload.base_url }];
      },
    },
    {
      readAgentChannelKey: async (channelId) => {
        readIds.push(channelId);
        return { ok: true, value: "sk-persisted-secret-999" };
      },
    }
  );
  const view = render(<ModelChannelsSettingsTab />);
  const keyInput = (await waitFor(() => view.getByLabelText("API 密钥"))) as HTMLInputElement;
  await waitFor(() => assert.equal(keyInput.value, "sk-persisted-secret-999"));
  assert.deepEqual(readIds, ["main"]);
  assert.equal(keyInput.type, "password");
  assert.equal(keyInput.placeholder, "sk-...");

  // Eye button toggles masked dots <-> plaintext
  fireEvent.click(view.getByRole("button", { name: "显示 API Key" }));
  assert.equal(keyInput.type, "text");

  // Auto-save URL edit without touching API key passes api_key: null and keeps keyInput populated!
  const urlInput = view.getByRole("textbox", { name: "服务基地址" });
  fireEvent.change(urlInput, {
    target: { value: "https://updated.example.invalid/v1" },
  });
  fireEvent.blur(urlInput);
  await waitFor(() => assert.equal(saves.length, 1));
  assert.equal(saves[0].api_key, null);
  assert.equal(keyInput.value, "sk-persisted-secret-999");
});

test("B1 / P0-2a: rename channel uses atomic channelsRename without round-tripping unmodified secret or calling channelsDelete", async () => {
  const renameCalls: Array<{ oldId: string; newId: string; apiKey?: string | null }> = [];
  const saves: AgentChannelSave[] = [];
  const deletedIds: string[] = [];
  let storedChannels: AgentChannelView[] = [channel];
  window.localStorage.setItem(
    "eostage.agent.modelSelection",
    JSON.stringify({ channel_id: "main", model_id: "o3", reasoning_level: "high" })
  );
  installBridge(
    {
      channelsGet: async () => storedChannels,
      channelsSave: async (payload) => {
        saves.push(payload);
        return storedChannels;
      },
      channelsRename: async (oldId, newId, apiKey) => {
        renameCalls.push({ oldId, newId, apiKey });
        storedChannels = [{ ...channel, id: newId, is_default: true, key_ready: true }];
        return storedChannels;
      },
      channelsDelete: async (id) => {
        deletedIds.push(id);
        return storedChannels;
      },
    },
    {
      readAgentChannelKey: async () => ({ ok: true, value: "sk-migrate-key-123" }),
    }
  );
  const view = render(<ModelChannelsSettingsTab />);
  await waitFor(() =>
    assert.equal((view.getByLabelText("API 密钥") as HTMLInputElement).value, "sk-migrate-key-123")
  );
  fireEvent.click(view.getByRole("button", { name: "更多供应商操作" }));
  fireEvent.click(view.getByTestId("provider-menu-rename-btn"));
  assert.ok(view.getByTestId("channel-rename-dialog"));

  fireEvent.change(view.getByTestId("channel-rename-input"), { target: { value: "main-renamed" } });
  fireEvent.click(view.getByTestId("channel-rename-confirm-btn"));

  await waitFor(() => assert.equal(renameCalls.length, 1));
  assert.deepEqual(renameCalls[0], { oldId: "main", newId: "main-renamed", apiKey: null });
  assert.equal(saves.length, 0, "no redundant save needed when metadata unchanged");
  assert.equal(deletedIds.length, 0, "atomic rename never calls non-atomic channelsDelete");
  assert.equal(view.queryByTestId("channel-rename-dialog"), null);
  assert.deepEqual(
    JSON.parse(window.localStorage.getItem("eostage.agent.modelSelection") ?? "null"),
    { channel_id: "main-renamed", model_id: "o3", reasoning_level: "high" }
  );
  window.localStorage.removeItem("eostage.agent.modelSelection");
});

test("P0-2a: rename fallback aborts immediately without calling channelsSave or channelsDelete when key_ready is true and key readback fails", async () => {
  const saves: AgentChannelSave[] = [];
  const deletedIds: string[] = [];
  installBridge(
    {
      channelsGet: async () => [channel],
      channelsSave: async (payload) => {
        saves.push(payload);
        return [channel];
      },
      channelsDelete: async (id) => {
        deletedIds.push(id);
        return [];
      },
    },
    {
      // Simulate HTTP debug bridge or transient credential store returning no plaintext secret
      readAgentChannelKey: async () => ({ ok: false, value: null } as any),
    }
  );
  const view = render(<ModelChannelsSettingsTab />);
  await waitFor(() => view.getByRole("button", { name: "更多供应商操作" }));
  assert.equal((view.getByLabelText("API 密钥") as HTMLInputElement).value, "");

  fireEvent.click(view.getByRole("button", { name: "更多供应商操作" }));
  fireEvent.click(view.getByTestId("provider-menu-rename-btn"));
  assert.ok(view.getByTestId("channel-rename-dialog"));

  fireEvent.change(view.getByTestId("channel-rename-input"), { target: { value: "main-unsafe" } });
  fireEvent.click(view.getByTestId("channel-rename-confirm-btn"));

  await waitFor(() =>
    assert.ok(
      view.getByTestId("channel-rename-dialog").textContent?.includes("已中止重命名以防密钥丢失")
    )
  );
  assert.equal(saves.length, 0, "MUST NOT create blank channel when key readback fails");
  assert.equal(deletedIds.length, 0, "MUST NOT delete original channel or Keychain entry");
});

test("composer sends picked model and level without pseudo default-model row", async () => {
  Object.assign(window.HTMLElement.prototype, { scrollTo() {} });
  const contexts: (AgentSendContext | null | undefined)[] = [];
  installBridge({
    send: async (_sessionId, _text, context) => { contexts.push(context); return "run-1"; },
    runsList: async () => [{ run_id: "run-1", session_id: "session-1", status: "completed", started_at: "2026-10-30T00:00:00Z", ended_at: "2026-10-30T00:00:01Z" }],
  });
  const view = render(<DesktopAgentWorkbench openSessionId="session-1" />);
  const input = await waitFor(() => view.getByRole("textbox", { name: "科学需求输入框" }));
  await waitFor(() => assert.equal(view.getByRole("button", { name: "模型" }).textContent, "GPT"));
  fireEvent.change(input, { target: { value: "默认模型请求" } });
  fireEvent.click(view.getByRole("button", { name: "发送" }));
  await waitFor(() => assert.equal(contexts.length, 1));
  assert.equal(Object.prototype.hasOwnProperty.call(contexts[0], "model"), false, JSON.stringify(contexts[0]));

  fireEvent.click(view.getByRole("button", { name: "模型" }));
  assert.equal(view.queryByTestId("model-picker-default-option"), null);
  fireEvent.click(await waitFor(() => view.getByRole("menuitemradio", { name: "o3" })));
  fireEvent.click(view.getByRole("button", { name: "模型" }));
  fireEvent.click(view.getByRole("menuitemradio", { name: "o3 high" }));
  fireEvent.change(input, { target: { value: "指定模型请求" } });
  await waitFor(() => assert.equal((view.getByRole("button", { name: "发送" }) as HTMLButtonElement).disabled, false), { timeout: 3000 });
  fireEvent.click(view.getByRole("button", { name: "发送" }));
  await waitFor(() => assert.equal(contexts.length, 2));
  assert.deepEqual(contexts[1]?.model, { channel_id: "main", model_id: "o3", reasoning_level: "high" });

  fireEvent.click(view.getByRole("button", { name: "模型" }));
  fireEvent.click(view.getByRole("menuitemradio", { name: "GPT" }));
  fireEvent.change(input, { target: { value: "切换回主模型请求" } });
  await waitFor(() => assert.equal((view.getByRole("button", { name: "发送" }) as HTMLButtonElement).disabled, false), { timeout: 3000 });
  fireEvent.click(view.getByRole("button", { name: "发送" }));
  await waitFor(() => assert.equal(contexts.length, 3));
  assert.deepEqual(contexts[2]?.model, { channel_id: "main", model_id: "gpt-default" });
});

test("deleting the last channel clears initialDraftRef so next click does not trigger unsaved-changes dialog", async () => {
  installBridge(
    {
      channelsDelete: async () => [],
    },
    {
      readAgentChannelKey: async () => ({ ok: true, value: "sk-last-channel-key" }),
    }
  );
  const view = render(<ModelChannelsSettingsTab />);
  await waitFor(() =>
    assert.equal((view.getByLabelText("API 密钥") as HTMLInputElement).value, "sk-last-channel-key")
  );
  fireEvent.click(view.getByRole("button", { name: "更多供应商操作" }));
  fireEvent.click(view.getByRole("button", { name: "删除渠道" }));
  fireEvent.click(view.getByRole("button", { name: "确认删除" }));

  await waitFor(() => {
    assert.ok(view.queryByLabelText("API 密钥") === null);
    assert.ok(view.queryByRole("button", { name: "添加渠道" }) === null);
    assert.ok(view.getByText("暂无模型渠道"));
  });
  fireEvent.click(view.getByTestId("official-provider-eostage-official"));
  assert.equal(view.queryByTestId("channel-unsaved-confirm-dialog"), null);
  assert.ok(view.getByTestId("official-eostage-panel"));
});

test("model dialog persists video, pdf, web_search, per-model protocol override, and avoids duplicate reasoning value box", async () => {
  const saves: AgentChannelSave[] = [];
  installBridge({
    channelsSave: async (payload) => {
      saves.push(payload);
      return [{ ...channel, model: payload.model, models: payload.models ?? [] }];
    },
  });
  const view = render(<ModelChannelsSettingsTab />);
  const gear = await waitFor(() => view.getByRole("button", { name: "设置模型 o3" }));
  fireEvent.click(gear);

  // Single-label reasoning chip: value inputs should NOT duplicate matching level names
  assert.equal(view.queryAllByRole("textbox", { name: "模型 2 推理参数值" }).length, 0);

  fireEvent.click(view.getByTestId("model-protocol-override-select"));
  fireEvent.click(view.getByRole("option", { name: /Responses/i }));
  fireEvent.click(view.getByRole("button", { name: "模型 2 视频输入" }));
  fireEvent.click(view.getByRole("button", { name: "模型 2 PDF 输入" }));
  fireEvent.click(view.getByRole("button", { name: "原生联网搜索" }));
  fireEvent.click(view.getByRole("button", { name: "完成" }));

  await waitFor(() => assert.equal(saves.length, 1));
  const savedO3 = saves[0].models?.find((m) => m.id === "o3");
  assert.ok(savedO3);
  assert.equal(savedO3?.protocol, "responses");
  assert.deepEqual(savedO3?.input_types, ["text", "image", "video", "pdf"]);
  assert.equal(savedO3?.supports_web_search, true);
});

test("Items 1-3 & 8a: defaults to EOStage 官方 before restoring last channel, removes reasoning & bottom save on existing channel, trims model badges, and makes header Credits non-clickable", async () => {
  const officialChannel: AgentChannelView = {
    ...channel,
    id: "eostage-official",
    is_default: false,
  };
  const oauthChannel: AgentChannelView = {
    ...channel,
    id: "openai-codex-oauth",
    is_default: true,
  };
  const customRelay: AgentChannelView = {
    ...channel,
    id: "tokenh",
    is_default: false,
  };
  __setEOStageAccountTestAdapter({
    status: async () => ({
      logged_in: true,
      email: "user@example.invalid",
      total_credits: 5754,
      trial_credits: 3000,
      trial_expires_at: null,
      trial_expire_days: 60,
      trial_remaining_seconds: null,
      paid_credits: 2754,
    }),
  });
  try {
    installBridge({
      channelsGet: async () => [oauthChannel, officialChannel, customRelay],
    });
    // First open with clean localStorage -> defaults to EOStage 官方 even though openai-codex-oauth has is_default: true
    window.localStorage.removeItem("eostage.modelSettings.lastChannelId");
    const view = render(<ModelChannelsSettingsTab />);
    const officialPanel = await waitFor(() => view.getByTestId("official-eostage-panel"));
    assert.ok(officialPanel);
    assert.equal(view.queryByRole("button", { name: "保存渠道配置" }), null);

    // Item 8a: header Credits pill is a non-clickable span; 查看余额与用量 remains a button
    const creditsPill = await waitFor(() => view.getByTestId("official-panel-credits-pill"));
    assert.notEqual(creditsPill.tagName.toLowerCase(), "button");
    const openBillingBtn = await waitFor(() => view.getByTestId("official-open-billing-btn"));
    assert.equal(openBillingBtn.tagName.toLowerCase(), "button");

    // Select custom channel -> remembers it and hides channel reasoning + bottom save; model row shows only context + vision
    fireEvent.click(await waitFor(() => view.getByRole("button", { name: "渠道 tokenh" })));
    await waitFor(() => assert.ok(view.getByRole("textbox", { name: "服务基地址" })));
    assert.equal(window.localStorage.getItem("eostage.modelSettings.lastChannelId"), "tokenh");
    assert.equal(view.queryByRole("button", { name: "保存渠道配置" }), null);
    assert.equal(view.queryByText("控制支持推理的模型是否输出思维链过程"), null);
    assert.ok(view.getByText("128K"));
    assert.ok(view.getByText("视觉"));
    assert.equal(view.queryByText("智能配置"), null);
    assert.equal(view.queryByText("工具"), null);
    assert.equal(view.queryByText("思考"), null);

    // Remount -> restores last viewed channel ("tokenh")
    view.unmount();
    const remounted = render(<ModelChannelsSettingsTab />);
    await waitFor(() =>
      assert.equal(remounted.getByRole("button", { name: "渠道 tokenh" }).getAttribute("aria-pressed"), "true")
    );

    // Brand-new custom draft still shows footer 取消 + 保存渠道配置, uses neutral custom cube logo (not OpenAI), and starts with gray missing_key dot
    fireEvent.click(remounted.getByRole("button", { name: "新建渠道" }));
    assert.ok(remounted.getByRole("button", { name: "保存渠道配置" }));
    const draftUrlInput = remounted.getByRole("textbox", { name: "服务基地址" }) as HTMLInputElement;
    assert.equal(draftUrlInput.value, "");
    const selectedDraftPill = remounted.container.querySelector('[aria-pressed="true"]');
    assert.ok(selectedDraftPill);
    assert.equal(selectedDraftPill.querySelector('[data-brand="openai"]'), null);
    assert.ok(selectedDraftPill.querySelector('[data-brand="custom"]'));

    // EOStage 官方 dot: green when logged in + enabled; toggling provider switch off flips it gray immediately
    const officialDot = await waitFor(() => remounted.getByTestId("official-channel-health-dot"));
    assert.equal(officialDot.getAttribute("data-health-state"), "ok");
    fireEvent.click(remounted.getByTestId("official-provider-eostage-official"));
    await waitFor(() => assert.ok(remounted.getByTestId("official-eostage-panel")));
    fireEvent.click(remounted.getByRole("switch", { name: "启用当前供应商" }));
    await waitFor(() =>
      assert.equal(
        remounted.getByTestId("official-channel-health-dot").getAttribute("data-health-state"),
        "disabled"
      )
    );
    await waitFor(() =>
      assert.equal(
        remounted.getByRole("switch", { name: "启用当前供应商" }).getAttribute("aria-checked"),
        "false"
      )
    );
    fireEvent.click(remounted.getByRole("switch", { name: "启用当前供应商" }));
    await waitFor(() =>
      assert.equal(
        remounted.getByTestId("official-channel-health-dot").getAttribute("data-health-state"),
        "ok"
      )
    );
  } finally {
    __setEOStageAccountTestAdapter(null);
  }
});

test("delete immediately after editing URL cancels pending auto-save timer and does not recreate deleted channel", async () => {
  const saves: AgentChannelSave[] = [];
  const deletedIds: string[] = [];
  installBridge({
    channelsSave: async (payload) => {
      saves.push(payload);
      return [channel];
    },
    channelsDelete: async (id) => {
      deletedIds.push(id);
      return [];
    },
  });
  const view = render(<ModelChannelsSettingsTab />);
  const urlInput = await waitFor(() => view.getByRole("textbox", { name: "服务基地址" }));
  fireEvent.change(urlInput, { target: { value: "https://edited-before-delete.example.invalid/v1" } });
  fireEvent.click(view.getByRole("button", { name: "更多供应商操作" }));
  fireEvent.click(view.getByRole("button", { name: "删除渠道" }));
  fireEvent.click(view.getByRole("button", { name: "确认删除" }));
  await waitFor(() => assert.deepEqual(deletedIds, ["main"]));
  // Wait beyond the 500ms auto-save debounce window to ensure the cancelled timer does not recreate the deleted channel
  await new Promise((resolve) => setTimeout(resolve, 600));
  assert.equal(saves.length, 0);
});

test("Free badge is read-only on official channel only, absent from edit modal, and never shown on custom channels", async () => {
  const officialChannel: AgentChannelView = {
    id: "eostage-official",
    protocol: "chat_completions",
    base_url: "https://api.eostage.org/eostage/v1",
    model: "neohorse-1.5-flash-preview",
    models: [
      { id: "neohorse-1.5-flash-preview", name: "NeoHorse 1.5 Flash Preview", is_free: true },
      { id: "glm-5.3-flash", name: "GLM-5.3 Flash", is_free: false },
    ],
    key_ready: true,
    is_default: true,
  };
  const customWithSpoof: AgentChannelView = {
    ...channel,
    models: [
      { id: "gpt-spoof", name: "Spoof", is_free: true },
      { id: "o3", name: "o3" },
    ],
  };

  // 1. Custom channel never renders Free badge or toggle, even if is_free is present on payload
  installBridge({
    channelsGet: async () => [customWithSpoof],
  });
  const customView = render(<ModelChannelsSettingsTab />);
  const customGear = await waitFor(() => customView.getByRole("button", { name: "设置模型 Spoof" }));
  assert.equal(customView.queryByTestId("model-free-badge-0"), null);
  fireEvent.click(customGear);
  assert.equal(customView.queryByTestId("model-modal-toggle-free"), null);
  fireEvent.click(customView.getByRole("button", { name: "完成" }));
  cleanup();

  // 2. Official channel renders green Free badge strictly on server-marked free models, with no user toggle in modal
  installBridge({
    channelsGet: async () => [officialChannel, customWithSpoof],
  });
  __setEOStageAccountTestAdapter({
    status: async () => ({
      logged_in: true,
      session_expired: false,
      email: "owner@example.test",
      display_name: null,
      status: "active",
      trial_granted: true,
      total_credits: 3000,
      remaining_credits: 3000,
      trial_credits: 3000,
      account_credits: 0,
      used_credits: 0,
      initial_trial_credits: 3000,
      total_recharged_credits: 0,
      trial_expires_at: null,
      active_sessions: 1,
      proxy_base_url: "https://api.eostage.org/eostage/v1",
    }),
  });
  const officialView = render(<ModelChannelsSettingsTab />);
  fireEvent.click(
    await waitFor(() => officialView.getByTestId("official-provider-eostage-official"))
  );
  const officialGear = await waitFor(() =>
    officialView.getByRole("button", { name: "设置模型 NeoHorse 1.5 Flash Preview" })
  );
  const freeBadge = officialView.getByTestId("model-free-badge-0");
  assert.equal(freeBadge.textContent, "Free");
  assert.ok(freeBadge.className.includes("eo-settings-badge-free"));
  assert.equal(officialView.queryByTestId("model-free-badge-1"), null);

  fireEvent.click(officialGear);
  assert.equal(officialView.queryByTestId("model-modal-toggle-free"), null);
  fireEvent.click(officialView.getByRole("button", { name: "完成" }));
});

test("channel list & detail badges distinguish API vs 订阅 and subscription channels render quota card without reading raw secret", async () => {
  const codexChannel: AgentChannelView = {
    id: "openai-codex-oauth",
    protocol: "responses",
    base_url: "https://chatgpt.com/backend-api/codex/responses",
    model: "gpt-5.4",
    models: [{ id: "gpt-5.4", name: "GPT-5.4" }],
    key_ready: true,
    is_default: false,
  };
  const openrouterOauthChannel: AgentChannelView = {
    id: "openrouter-oauth",
    protocol: "chat_completions",
    base_url: "https://openrouter.ai/api/v1",
    model: "openai/gpt-4o",
    models: [{ id: "openai/gpt-4o", name: "GPT-4o" }],
    key_ready: true,
    is_default: false,
  };
  const renamedCodexChannel: AgentChannelView = {
    id: "my-work-sub",
    protocol: "responses",
    base_url: "https://custom.relay.invalid/v1",
    model: "gpt-5.4",
    models: [{ id: "gpt-5.4", name: "GPT-5.4" }],
    key_ready: true,
    is_default: false,
  };

  const readKeyIds: string[] = [];
  const quotaCalls: string[] = [];
  const futureReset = Math.floor(Date.now() / 1000) + 7200;

  const prevTauriInternals = (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  Object.defineProperty(window, "__TAURI_INTERNALS__", {
    configurable: true,
    writable: true,
    value: {
      invoke: async (cmd: string, args?: Record<string, unknown>) => {
        if (cmd === "agent_oauth_channels_meta") {
          return [
            { channel_id: "main", auth_kind: "api", oauth_provider: null, account_id: null },
            {
              channel_id: "openai-codex-oauth",
              auth_kind: "subscription",
              oauth_provider: "openai_codex",
              account_id: "acct_1",
            },
            { channel_id: "openrouter-oauth", auth_kind: "api", oauth_provider: null, account_id: null },
            {
              channel_id: "my-work-sub",
              auth_kind: "subscription",
              oauth_provider: "openai_codex",
              account_id: "acct_2",
            },
          ];
        }
        if (cmd === "agent_oauth_channel_quota") {
          const cid = String(args?.channelId ?? args?.channel_id ?? "");
          quotaCalls.push(cid);
          return {
            channel_id: cid,
            oauth_provider: "openai_codex",
            plan_type: "pro",
            account_id: "acct_1",
            primary_window: {
              used_percent: 22,
              remaining_percent: 78,
              window_minutes: 300,
              reset_at_secs: futureReset,
            },
            secondary_window: {
              used_percent: 65,
              remaining_percent: 35,
              window_minutes: 10080,
              reset_at_secs: futureReset + 86400,
            },
            reset_credits_available: 2,
            credits_has_credits: true,
            credits_unlimited: false,
            credits_balance: "12.50",
            fetched_at_ms: Date.now(),
          };
        }
        return null;
      },
    },
  });

  try {
    installBridge(
      {
        channelsGet: async () => [channel, codexChannel, openrouterOauthChannel, renamedCodexChannel],
      },
      {
        readAgentChannelKey: async (channelId) => {
          readKeyIds.push(channelId);
          return { ok: true, value: "sk-api-key" };
        },
      }
    );

    const view = render(<ModelChannelsSettingsTab />);
    await waitFor(() => assert.equal(view.getAllByTestId("channel-auth-kind-badge").length, 4));
    const railBadges = view.getAllByTestId("channel-auth-kind-badge");
    assert.deepEqual(
      railBadges.map((el) => `${el.getAttribute("data-auth-kind")}:${el.textContent}`),
      ["api:API", "subscription:订阅", "api:API", "subscription:订阅"]
    );

    // Selecting OpenRouter PKCE keeps API badge & API key input
    fireEvent.click(view.getByRole("button", { name: /openrouter-oauth/ }));
    const detailBadgeOr = await waitFor(() => view.getByTestId("channel-detail-auth-kind-badge"));
    assert.equal(detailBadgeOr.getAttribute("data-auth-kind"), "api");
    assert.equal(detailBadgeOr.textContent, "API");
    assert.equal(view.queryByTestId("subscription-quota-card"), null);

    // Selecting renamed Codex subscription channel shows 订阅 badge & Quota Card, skips reading raw key
    fireEvent.click(view.getByRole("button", { name: /my-work-sub/ }));
    const quotaCard = await waitFor(() => view.getByTestId("subscription-quota-card"));
    assert.equal(view.getByTestId("channel-detail-auth-kind-badge").textContent, "订阅");
    assert.equal(view.queryByLabelText("API 密钥"), null);
    await waitFor(() => assert.ok(quotaCalls.includes("my-work-sub")));
    assert.ok(!readKeyIds.includes("my-work-sub"));
    assert.equal(view.getByTestId("subscription-plan-badge").textContent, "pro");
    assert.ok(view.getByTestId("subscription-quota-primary").textContent?.includes("78%"));
    assert.ok(view.getByTestId("subscription-quota-secondary").textContent?.includes("35%"));
    assert.ok(view.getByTestId("subscription-credits-balance").textContent?.includes("$12.50"));
    assert.ok(view.getByTestId("subscription-reset-credits").textContent?.includes("2"));

    // Refresh button re-queries quota
    const prevCount = quotaCalls.length;
    fireEvent.click(view.getByTestId("subscription-quota-refresh-btn"));
    await waitFor(() => assert.equal(quotaCalls.length, prevCount + 1));
    assert.ok(quotaCard.isConnected);
  } finally {
    if (prevTauriInternals === undefined) {
      delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
    } else {
      Object.defineProperty(window, "__TAURI_INTERNALS__", {
        configurable: true,
        writable: true,
        value: prevTauriInternals,
      });
    }
  }
});

test("re-clicking open-pkce-browser-btn starts a fresh native PKCE session after completion stops the listener", async () => {
  let pkceStartCount = 0;
  let openBrowserCalls = 0;

  const prevTauriInternals = (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  Object.defineProperty(window, "__TAURI_INTERNALS__", {
    configurable: true,
    writable: true,
    value: {
      invoke: async (cmd: string) => {
        if (cmd === "agent_oauth_channels_meta") return [];
        if (cmd === "agent_oauth_pkce_start") {
          pkceStartCount += 1;
          return {
            session_id: `pkce_sess_${pkceStartCount}`,
            provider_id: "openai_codex",
            authorize_url: `https://auth.openai.com/oauth/authorize?state=st_${pkceStartCount}`,
            redirect_uri: "http://localhost:1455/auth/callback",
            state: `st_${pkceStartCount}`,
            expires_in_secs: 600,
          };
        }
        if (cmd === "agent_oauth_open_browser") {
          openBrowserCalls += 1;
          return { url: "https://auth.openai.com/oauth/authorize" };
        }
        if (cmd === "agent_oauth_pkce_complete") {
          return {
            status: "error",
            channel_id: null,
            message: "回调兑换失败：授权码仅能使用一次",
          };
        }
        return null;
      },
    },
  });

  try {
    installBridge({ channelsGet: async () => [channel] });
    const view = render(<ModelChannelsSettingsTab />);
    fireEvent.click(await waitFor(() => view.getByRole("button", { name: /添加供应商/ })));
    fireEvent.click(await waitFor(() => view.getByTestId("generate-pkce-auth-url-btn")));
    await waitFor(() => assert.equal(pkceStartCount, 1));
    await waitFor(() => assert.equal(openBrowserCalls, 1));

    // Wait for poller to receive terminal error & stop listener cleanly
    await waitFor(() =>
      assert.ok(view.getByText(/授权码仅能使用一次/))
    );

    // Re-clicking "打开授权网页" MUST restart a fresh live listener session instead of reopening dead URL
    fireEvent.click(view.getByTestId("open-pkce-browser-btn"));
    await waitFor(() => assert.equal(pkceStartCount, 2));
    await waitFor(() => assert.equal(openBrowserCalls, 2));
  } finally {
    if (prevTauriInternals === undefined) {
      delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
    } else {
      Object.defineProperty(window, "__TAURI_INTERNALS__", {
        configurable: true,
        writable: true,
        value: prevTauriInternals,
      });
    }
  }
});

test("Copilot quota labels, Device Flow UI, and 3-dot callback URL guard against JWT misparse", async () => {
  const copilotChannel: AgentChannelView = {
    id: "github-copilot",
    protocol: "chat_completions",
    base_url: "https://api.githubcopilot.com",
    model: "gpt-4o",
    reasoning_mode: "default",
    key_ready: true,
    is_default: false,
  };
  const completeCalls: Array<{ sessionId: string; callbackInput: string | null }> = [];
  const deviceStarts: string[] = [];

  const prevTauriInternals = (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  Object.defineProperty(window, "__TAURI_INTERNALS__", {
    configurable: true,
    writable: true,
    value: {
      invoke: async (cmd: string, args?: Record<string, unknown>) => {
        if (cmd === "agent_oauth_channels_meta") {
          return [
            {
              channel_id: "github-copilot",
              auth_kind: "subscription",
              oauth_provider: "github_copilot",
              account_id: "octocat",
            },
          ];
        }
        if (cmd === "agent_oauth_channel_quota") {
          return {
            channel_id: "github-copilot",
            oauth_provider: "github_copilot",
            plan_type: "individual",
            account_id: "octocat",
            primary_window: {
              used_percent: 10,
              remaining_percent: 90,
              window_minutes: 43200,
              reset_at_secs: Math.floor(Date.now() / 1000) + 7200,
            },
            secondary_window: {
              used_percent: 5,
              remaining_percent: 95,
              window_minutes: 43200,
              reset_at_secs: Math.floor(Date.now() / 1000) + 7200,
            },
            reset_credits_available: null,
            credits_has_credits: false,
            credits_unlimited: false,
            credits_balance: null,
            fetched_at_ms: Date.now(),
          };
        }
        if (cmd === "agent_oauth_device_start") {
          deviceStarts.push(String(args?.providerId ?? ""));
          return {
            session_id: "dev_sess_1",
            provider_id: "github_copilot",
            user_code: "ABCD-1234",
            verification_uri: "https://github.com/login/device",
            interval_secs: 5,
            expires_in_secs: 900,
          };
        }
        if (cmd === "agent_oauth_device_poll") {
          return { status: "pending" };
        }
        if (cmd === "agent_oauth_pkce_start") {
          return {
            session_id: "pkce_dot_1",
            provider_id: "openai_codex",
            authorize_url: "https://auth.openai.com/oauth/authorize?state=st_dot",
            redirect_uri: "http://localhost:1455/auth/callback",
            state: "st_dot",
            expires_in_secs: 600,
          };
        }
        if (cmd === "agent_oauth_pkce_complete") {
          const entry = {
            sessionId: String(args?.sessionId ?? ""),
            callbackInput: (args?.callbackInput as string | null) ?? null,
          };
          completeCalls.push(entry);
          if (entry.callbackInput) {
            return {
              status: "authorized",
              channel_id: "openai-codex",
              message: "已完成回调兑换并保存",
            };
          }
          return { status: "pending" };
        }
        if (cmd === "agent_oauth_open_browser") {
          return { url: String(args?.url ?? "") };
        }
        return null;
      },
    },
  });

  try {
    installBridge({ channelsGet: async () => [channel, copilotChannel] });
    const view = render(<ModelChannelsSettingsTab />);

    // 1. Copilot channel renders Copilot-specific quota labels
    fireEvent.click(await waitFor(() => view.getByRole("button", { name: /github-copilot/ })));
    await waitFor(() => assert.ok(view.getByTestId("subscription-quota-primary")));
    assert.ok(view.getByTestId("subscription-quota-primary").textContent?.includes("高级交互额度"));
    assert.ok(view.getByTestId("subscription-quota-secondary").textContent?.includes("对话额度"));

    // 2. Switching to GitHub Copilot Device Flow starts device auth, displays code, hides callback input
    fireEvent.click(view.getByRole("button", { name: /添加供应商/ }));
    fireEvent.click(await waitFor(() => view.getByTestId("pkce-provider-github_copilot_device")));
    fireEvent.click(view.getByTestId("generate-pkce-auth-url-btn"));
    await waitFor(() => assert.deepEqual(deviceStarts, ["github_copilot"]));
    assert.equal(view.getByTestId("device-user-code-display").textContent, "ABCD-1234");
    assert.ok(view.getByTestId("copy-device-code-btn"));
    assert.equal(view.queryByTestId("pkce-callback-input"), null);

    // 3. Callback URL containing 3+ dots (e.g. code=ac.1.AABa...) MUST NOT be parsed as JWT
    fireEvent.click(view.getByTestId("pkce-provider-openai_codex_pkce"));
    fireEvent.click(view.getByTestId("generate-pkce-auth-url-btn"));
    const cbInput = await waitFor(() => view.getByTestId("pkce-callback-input"));
    const dotCallback = "http://localhost:1455/auth/callback?code=ac.1.AABaFooBarBaz&state=st_dot";
    fireEvent.change(cbInput, { target: { value: dotCallback } });
    fireEvent.click(view.getByTestId("verify-pkce-callback-btn"));
    await waitFor(() =>
      assert.ok(completeCalls.some((c) => c.callbackInput === dotCallback))
    );
    assert.equal(view.queryByTestId("pkce-jwt-parsed-toast"), null);
  } finally {
    if (prevTauriInternals === undefined) {
      delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
    } else {
      Object.defineProperty(window, "__TAURI_INTERNALS__", {
        configurable: true,
        writable: true,
        value: prevTauriInternals,
      });
    }
  }
});
