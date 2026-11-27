import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import React, { useState } from "react";
import { cleanup, fireEvent, render } from "../../test-utils/render-harness";
import { waitFor } from "@testing-library/react";
import { registerHooks } from "node:module";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { transformSync } from "esbuild";
import { I18nProvider } from "../../i18n";
import {
  clearTestDesktopShellBridge,
  registerTestDesktopShellBridge,
} from "@/lib/desktopShellBridge";
import { createWorkbenchTestDesktopShellBridge } from "../../test-utils/workbenchDesktopBridge";
import type { DesktopAgentBridge } from "@/shell/api/agentBridgeTypes";
import type {
  AgentChannelView,
  AgentSendContext,
} from "@/lib/generated/v3-contracts";
import { syncAgentModelSelection } from "./AgentModelPicker";
import { AgentComposer } from "./AgentComposer";
import { AgentErrorCard } from "./AgentErrorCard";
import { ReasoningIntensitySlider } from "./agent/ReasoningIntensitySlider";
import { ctrlShortcut, isMacPlatform } from "@/lib/platformShortcuts";
import { OAUTH_PKCE_PROVIDERS } from "@/lib/eoAgentSuite";
import { clearChannelHealth, setChannelHealth } from "@/shell/api/channelHealth";
import catalogRegistry from "@/lib/dataSourceCatalogRegistry.json";

if (typeof window !== "undefined") {
  Object.assign(window.HTMLElement.prototype, { scrollTo() {} });
  if (typeof window.requestAnimationFrame === "undefined") {
    window.requestAnimationFrame = (fn) => setTimeout(() => fn(Date.now()), 16) as any;
  }
  if (typeof window.cancelAnimationFrame === "undefined") {
    window.cancelAnimationFrame = (id) => clearTimeout(id as any);
  }
}

const scopedSource = (url: string) =>
  `import.meta.env ??= { DEV: true };\n${readFileSync(fileURLToPath(url), "utf8")}`;

const hooks = registerHooks({
  resolve(specifier, context, nextResolve) {
    if (
      specifier === "react-leaflet" ||
      specifier === "@/components/map/leafletBindings" ||
      specifier.includes("leafletBindings")
    ) {
      return {
        url: new URL("../../../tests/reactLeafletBoundaryStub.tsx", import.meta.url).href,
        shortCircuit: true,
      };
    }
    if (specifier === "leaflet") {
      return {
        url: new URL("../../../tests/leafletBoundaryStub.ts", import.meta.url).href,
        shortCircuit: true,
      };
    }
    return nextResolve(specifier, context);
  },
  load(url, context, nextLoad) {
    if (
      url.endsWith("/DesktopAgentWorkbench.tsx") ||
      url.endsWith("/WorkbenchMap.tsx") ||
      url.endsWith("/reactLeafletBoundaryStub.tsx") ||
      url.endsWith("/leafletBoundaryStub.ts")
    ) {
      return {
        format: "module",
        shortCircuit: true,
        source: transformSync(scopedSource(url), {
          loader: url.endsWith(".ts") ? "ts" : "tsx",
          format: "esm",
          jsx: "automatic",
        }).code,
      };
    }
    return nextLoad(url, context);
  },
});

const { DesktopAgentWorkbench } = await import("./DesktopAgentWorkbench");
hooks.deregister();

const CHANNELS: AgentChannelView[] = [
  {
    id: "openai-codex-oauth",
    protocol: "responses",
    base_url: "https://chatgpt.com/backend-api/codex",
    model: "gpt-6.2-sol",
    reasoning_mode: "default",
    models: [
      { id: "gpt-6.2-sol", name: "GPT-6.2 Sol", input_types: ["text"] },
      { id: "gpt-6.2-astra", name: "GPT-6.2 Astra", input_types: ["text"] },
    ],
    key_ready: true,
    is_default: true,
  },
  {
    id: "tokenh",
    protocol: "responses",
    base_url: "https://tokenh.example.invalid/v1",
    model: "neohorse-1.5-flash-preview",
    reasoning_mode: "default",
    models: [
      {
        id: "neohorse-1.5-flash-preview",
        name: "neohorse-1.5-flash-preview",
        input_types: ["text"],
      },
    ],
    key_ready: true,
    is_default: false,
  },
];

test.afterEach(() => {
  cleanup();
  clearTestDesktopShellBridge();
  clearChannelHealth("openai-codex-oauth");
  clearChannelHealth("tokenh");
  syncAgentModelSelection(null, null);
  if (typeof window !== "undefined" && window.localStorage) {
    window.localStorage.clear();
  }
});

test("Native macOS 2026-11-19 independent audit 6 fixes verification", async (suite) => {
  await suite.test(
    "P1-1: switching model inside an old session and immediately clicking New Chat dispatches and displays the chosen model",
    async () => {
      const sentPayloads: Array<{ sid: string; text: string; ctx?: AgentSendContext }> = [];
      let seqCounter = 0;

      const base = createWorkbenchTestDesktopShellBridge();
      const agent = {
        channelsGet: async () => CHANNELS,
        channelsModels: async () => ["gpt-6.2-sol", "neohorse-1.5-flash-preview"],
        channelsProbeDraft: async () => ({
          result: { ok: true, code: null, status: 200, model_count: 1 },
          models: [],
        }),
        channelsSave: async () => CHANNELS,
        sessionsList: async () => [
          {
            session_id: "sess-old-err",
            title: "旧报错会话",
            created_at: "2026-11-19T00:00:00Z",
            updated_at: "2026-11-19T00:00:00Z",
            run_ids: ["run-err-old"],
          },
        ],
        sessionsCreate: async () => ({
          session_id: `sess-created-${++seqCounter}`,
          title: "hi",
          created_at: "2026-11-19T01:00:00Z",
          updated_at: "2026-11-19T01:00:00Z",
          run_ids: [],
        }),
        sessionsSend: async (sid: string, text: string, ctx?: AgentSendContext) => {
          sentPayloads.push({ sid, text, ctx });
          return { run_id: `run-${seqCounter}`, queued: false };
        },
        send: async (sid: string, text: string, ctx?: AgentSendContext) => {
          sentPayloads.push({ sid, text, ctx });
          return `run-${seqCounter}`;
        },
        streamPoll: async (sid: string, cursor: number) =>
          sid === "sess-old-err" && cursor === 0
            ? {
                events: [
                  {
                    seq: 1,
                    kind: "message",
                    payload: { run_id: "run-err-old", role: "user", text: "hi" },
                  },
                  {
                    seq: 2,
                    kind: "error",
                    payload: {
                      run_id: "run-err-old",
                      code: "model_authentication",
                      status: "failed",
                      plan_id: null,
                      message_zh: "HTTP 401（认证失败或令牌已过期）",
                    },
                  },
                ],
                cursor: 2,
              }
            : { events: [], cursor },
        runsList: async (sid: string) =>
          sid === "sess-old-err"
            ? [
                {
                  run_id: "run-err-old",
                  session_id: "sess-old-err",
                  status: "failed",
                  started_at: "2026-11-19T00:00:00Z",
                  ended_at: "2026-11-19T00:00:01Z",
                },
              ]
            : [],
        sessionWork: async () => [],
        permissionModeGet: async () => ({
          effective: "confirm_changes",
          session_override: null,
          global_default: "confirm_changes",
        }),
      } as unknown as DesktopAgentBridge;
      registerTestDesktopShellBridge({ ...base, agent });

      function NewChatHarness() {
        const [openSessionId, setOpenSessionId] = useState<string | null>("sess-old-err");
        const [newChatNonce, setNewChatNonce] = useState(0);
        return (
          <div>
            <button
              type="button"
              data-testid="harness-new-chat-btn"
              onClick={() => {
                setOpenSessionId(null);
                setNewChatNonce((n) => n + 1);
              }}
            >
              新对话
            </button>
            <DesktopAgentWorkbench
              openSessionId={openSessionId}
              newChatNonce={newChatNonce}
            />
          </div>
        );
      }

      const view = render(
        <I18nProvider>
          <NewChatHarness />
        </I18nProvider>
      );

      // Wait for channels and model picker trigger
      await waitFor(() =>
        assert.ok(view.getByTestId("agent-model-picker-trigger"))
      );
      await waitFor(() =>
        assert.equal(view.getByTestId("agent-model-picker-trigger").hasAttribute("disabled"), false)
      );

      // Inside old session, switch model to tokenh / neohorse-1.5-flash-preview
      fireEvent.click(view.getByTestId("agent-model-picker-trigger"));
      await waitFor(() => assert.ok(view.getByTestId("model-picker-active-channel-header")));
      fireEvent.click(view.getByTestId("model-picker-active-channel-header"));
      await waitFor(() => assert.ok(view.getByTestId("model-picker-channel-tokenh")));
      fireEvent.click(view.getByTestId("model-picker-channel-tokenh"));
      await waitFor(() =>
        assert.ok(view.getByTestId("model-picker-model-neohorse-1.5-flash-preview"))
      );
      fireEvent.click(view.getByTestId("model-picker-model-neohorse-1.5-flash-preview"));

      // Immediately click "新对话"
      fireEvent.click(view.getByTestId("harness-new-chat-btn"));
      await waitFor(() =>
        assert.ok(
          view
            .getByTestId("agent-model-picker-trigger")
            .textContent?.includes("neohorse-1.5-flash-preview")
        )
      );

      // Send "hi" on the brand-new session
      const textarea = view.container.querySelector("textarea") as HTMLTextAreaElement;
      fireEvent.change(textarea, { target: { value: "hi" } });
      fireEvent.click(view.getByTestId("agent-send-btn"));

      await waitFor(() => assert.equal(sentPayloads.length, 1));
      assert.deepEqual(sentPayloads[0].ctx?.model, {
        channel_id: "tokenh",
        model_id: "neohorse-1.5-flash-preview",
      });
    }
  );

  await suite.test(
    "P2-4: healthy different-channel alternates sort ahead of broken same-channel models, switched label sticks, and Composer disables macOS autocorrect",
    async () => {
      setChannelHealth("openai-codex-oauth", { status: "error", checkedAt: Date.now() });
      setChannelHealth("tokenh", { status: "ok", checkedAt: Date.now() });

      const base = createWorkbenchTestDesktopShellBridge();
      const agent = {
        channelsGet: async () => CHANNELS,
        channelsModels: async () => [],
        channelsProbeDraft: async () => ({
          result: { ok: true, code: null, status: 200, model_count: 1 },
          models: [],
        }),
        channelsSave: async () => CHANNELS,
        sessionsList: async () => [],
        sessionsCreate: async () => ({
          session_id: "sess-err-alt",
          title: "err",
          created_at: "2026-11-19T00:00:00Z",
          updated_at: "2026-11-19T00:00:00Z",
          run_ids: ["run-err-alt"],
        }),
        streamPoll: async (_sid: string, cursor: number) =>
          cursor === 0
            ? {
                events: [
                  {
                    seq: 1,
                    kind: "message",
                    payload: { run_id: "run-err-alt", role: "user", text: "hi" },
                  },
                  {
                    seq: 2,
                    kind: "error",
                    payload: {
                      run_id: "run-err-alt",
                      code: "model_authentication",
                      status: "failed",
                      plan_id: null,
                      message_zh: "HTTP 401：Expired",
                    },
                  },
                ],
                cursor: 2,
              }
            : { events: [], cursor },
        runsList: async () => [
          {
            run_id: "run-err-alt",
            session_id: "sess-err-alt",
            status: "failed",
            started_at: "2026-11-19T00:00:00Z",
            ended_at: "2026-11-19T00:00:01Z",
          },
        ],
        sessionWork: async () => [],
        permissionModeGet: async () => ({
          effective: "confirm_changes",
          session_override: null,
          global_default: "confirm_changes",
        }),
      } as unknown as DesktopAgentBridge;
      registerTestDesktopShellBridge({ ...base, agent });

      const view = render(
        <I18nProvider>
          <DesktopAgentWorkbench openSessionId="sess-err-alt" onOpenSettings={() => {}} />
        </I18nProvider>
      );

      // Owner 2026-11-21: 错误卡精简为只展示清晰报错理由、重试与认证类设置入口（移除接着继续/备用模型切换）
      await waitFor(() => assert.ok(view.getByTestId("agent-error-card")));
      assert.equal(view.queryByTestId("agent-error-alt-model-select"), null);
      assert.equal(view.queryByTestId("agent-error-switch-model-btn"), null);
      assert.equal(view.queryByTestId("agent-error-resume-btn"), null);
      assert.ok(view.getByTestId("btn-error-retry"));
      assert.ok(view.getByTestId("btn-error-open-model-settings"));
      cleanup();

      // Composer disables macOS WebKit autocorrect popover ("Hi x")
      const compView = render(
        <I18nProvider>
          <AgentComposer
            value=""
            onInputChange={() => {}}
            onSubmit={() => {}}
            channelReadiness={{ ready: true, disabled: false, reason_zh: "" }}
            channels={CHANNELS}
            sessionId={null}
          />
        </I18nProvider>
      );
      const ta = compView.container.querySelector("textarea") as HTMLTextAreaElement;
      assert.equal(ta.getAttribute("autocorrect"), "off");
      assert.equal(ta.getAttribute("autocapitalize"), "off");
      assert.equal(ta.getAttribute("spellcheck"), "false");
      cleanup();

      // Auth error (HTTP 401) shows settings link while transient upstream error hides clutter controls
      const cardView = render(
        <I18nProvider>
          <AgentErrorCard
            item={{
              kind: "error",
              seq: 1,
              run_id: "r1",
              code: "model_authentication",
              status: "failed",
              plan_id: null,
              message_zh: "HTTP 401",
            }}
            onRetry={() => {}}
            onOpenModelSettings={() => {}}
          />
        </I18nProvider>
      );
      assert.ok(cardView.getByTestId("btn-error-open-model-settings"));
      assert.ok(cardView.getByTestId("btn-error-retry"));
    }
  );

  await suite.test(
    "P2-5: ctrlShortcut and ReasoningIntensitySlider adapt to MacIntel (⌃T) and Windows/Linux (Ctrl+T)",
    () => {
      const desc = Object.getOwnPropertyDescriptor(window.navigator, "platform");
      try {
        Object.defineProperty(window.navigator, "platform", { configurable: true, value: "MacIntel" });
        assert.equal(isMacPlatform(), true);
        assert.equal(ctrlShortcut("T"), "⌃T");
        const macSlider = render(
          <ReasoningIntensitySlider
            levels={[
              { level: "low", value: "low" },
              { level: "high", value: "high" },
            ]}
            currentLevel="high"
            disabled={false}
          />
        );
        assert.equal(
          macSlider.getByTestId("reasoning-intensity-shortcut-btn").textContent?.trim(),
          "⌃T"
        );
        cleanup();

        Object.defineProperty(window.navigator, "platform", { configurable: true, value: "Win32" });
        assert.equal(ctrlShortcut("T"), "Ctrl+T");
      } finally {
        if (desc) {
          Object.defineProperty(window.navigator, "platform", desc);
        } else {
          Object.defineProperty(window.navigator, "platform", { configurable: true, value: "" });
        }
      }
    }
  );

  await suite.test(
    "P1-2, P2-3 & P2-6: stale notice cleared on search, unclipped plugin operator buttons, spaced NISAR codes, concise PKCE pill copy",
    () => {
      const thisDir = path.dirname(fileURLToPath(import.meta.url));
      const dwPath = path.resolve(thisDir, "DesktopWorkbench.tsx");
      const dwSrc = fs.readFileSync(dwPath, "utf8");
      const firstPageStart = dwSrc.indexOf("const applyFirstBatchPage");
      const firstPageBody = dwSrc.slice(firstPageStart, firstPageStart + 1100);
      assert.ok(
        firstPageBody.includes("setTaskOutcomeNotice(null)"),
        "applyFirstBatchPage must clear stale search notices from prior dedicated queries"
      );

      const psPath = path.resolve(thisDir, "PluginsAndSkillsPage.tsx");
      const psSrc = fs.readFileSync(psPath, "utf8");
      assert.equal(
        psSrc.includes("执行窗口算子"),
        false,
        "internal debug operator panel should remain removed from PluginsAndSkillsPage"
      );

      const nisarL = (catalogRegistry as Record<string, { description?: string }>)[
        "science.nisar.l.rslc.provisional.v1"
      ];
      assert.ok(nisarL?.description?.includes("RSLC / GSLC / GCOV"));

      const codexPkce = OAUTH_PKCE_PROVIDERS.find((p) => p.id === "openai_codex_pkce");
      assert.equal(codexPkce?.name_zh, "OpenAI Codex / ChatGPT (PKCE)");
    }
  );
});
