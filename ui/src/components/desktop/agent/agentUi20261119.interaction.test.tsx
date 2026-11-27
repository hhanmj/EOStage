import "../../../test-utils/setup-dom";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import React, { act } from "react";
import { cleanup, fireEvent, render } from "../../../test-utils/render-harness";
import { waitFor } from "@testing-library/react";
import { registerHooks } from "node:module";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { transformSync } from "esbuild";
import { I18nProvider } from "../../../i18n";
import {
  clearTestDesktopShellBridge,
  registerTestDesktopShellBridge,
} from "@/lib/desktopShellBridge";
import { emitLocalDesktopHostEvent } from "@/lib/desktopHostEvents";
import { createWorkbenchTestDesktopShellBridge } from "../../../test-utils/workbenchDesktopBridge";
import type { DesktopAgentBridge } from "@/shell/api/agentBridgeTypes";
import type {
  AgentChannelView,
  AgentModelSelection,
  AgentSendContext,
  AgentStreamEvent,
} from "@/lib/generated/v3-contracts";
import type { ApplicationTask } from "@/shell/api/types";
import { classifyStreamEvents, type StreamErrorItem, type StreamToolGroupItem } from "@/shell/api/agentStreamLogic";
import { AgentComposer } from "../AgentComposer";
import { AgentModelPicker } from "../AgentModelPicker";
import { AgentErrorCard } from "../AgentErrorCard";
import { AgentToolGroupCard } from "../AgentToolGroupCard";
import { AgentThinkingCard, UnifiedTurnProcessHeader } from "./AgentThinkingCard";
import {
  createImeEnterGuardState,
  onImeCompositionEnd,
  onImeCompositionStart,
  onImeKeyUp,
  shouldBlockKeyDownForIme,
} from "./imeEnterGuard";
import { toWireAttachments } from "./composerAttachments";
import { countSessionActiveDownloads } from "./sessionDownloadScope";

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
        url: new URL("../../../../tests/reactLeafletBoundaryStub.tsx", import.meta.url).href,
        shortCircuit: true,
      };
    }
    if (specifier === "leaflet") {
      return {
        url: new URL("../../../../tests/leafletBoundaryStub.ts", import.meta.url).href,
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

const { DesktopAgentWorkbench } = await import("../DesktopAgentWorkbench");
hooks.deregister();

const MULTI_CHANNELS: AgentChannelView[] = [
  {
    id: "AGY",
    protocol: "responses",
    base_url: "https://agy.example.invalid/v1",
    model: "gemini-4-carbon",
    reasoning_mode: "default",
    models: [
      {
        id: "gemini-4-argon",
        input_types: ["text", "image"],
        reasoning_levels: [
          { level: "off", value: null },
          { level: "low", value: "low" },
          { level: "medium", value: "medium" },
          { level: "high", value: "high" },
          { level: "xhigh", value: "xhigh" },
        ],
      },
      {
        id: "gemini-4-carbon",
        input_types: ["text", "image"],
        reasoning_levels: [
          { level: "off", value: null },
          { level: "low", value: "low" },
          { level: "medium", value: "medium" },
          { level: "high", value: "high" },
          { level: "xhigh", value: "xhigh" },
        ],
      },
      {
        id: "claude-sonnet-4-6",
        input_types: ["text", "image"],
        reasoning_levels: [],
      },
    ],
    key_ready: true,
    is_default: true,
  },
  {
    id: "openai-codex-oauth",
    protocol: "responses",
    base_url: "https://chatgpt.com/backend-api/codex",
    model: "gpt-5.4",
    reasoning_mode: "default",
    models: [
      {
        id: "gpt-5.4",
        name: "GPT-5.4 Codex",
        input_types: ["text", "image"],
        reasoning_levels: [
          { level: "low", value: "low" },
          { level: "medium", value: "medium" },
          { level: "high", value: "high" },
        ],
      },
    ],
    key_ready: true,
    is_default: false,
  },
];

function installBridge(overrides: Partial<DesktopAgentBridge>) {
  const base = createWorkbenchTestDesktopShellBridge();
  const agent = {
    channelsGet: async () => MULTI_CHANNELS,
    channelsModels: async () => ["gemini-4-carbon"],
    channelsProbeDraft: async () => ({
      result: { ok: true, code: null, status: 200, model_count: 1 },
      models: ["gemini-4-carbon"],
    }),
    channelsSave: async () => MULTI_CHANNELS,
    sessionsList: async () => [
      {
        session_id: "sess-ui-19",
        title: "2026-11-19 UI 验收",
        created_at: "2026-11-19T00:00:00Z",
        updated_at: "2026-11-19T00:00:00Z",
        run_ids: ["run-1"],
      },
    ],
    sessionsCreate: async () => ({
      session_id: "sess-ui-19",
      title: "新会话",
      created_at: "2026-11-19T00:00:00Z",
      updated_at: "2026-11-19T00:00:00Z",
      run_ids: [],
    }),
    streamPoll: async () => ({ events: [], cursor: 0 }),
    runsList: async () => [],
    sessionWork: async () => [],
    permissionModeGet: async () => ({
      effective: "confirm_changes",
      session_override: null,
      global_default: "confirm_changes",
    }),
    ...overrides,
  } as DesktopAgentBridge;
  registerTestDesktopShellBridge({ ...base, agent });
}

function makeTask(overrides: Partial<ApplicationTask> = {}): ApplicationTask {
  return {
    task_id: "task-1",
    revision: 1,
    category: "scene_download",
    name: "Sentinel-1 下载",
    status: "running",
    done_bytes: 1024,
    total_bytes: 4096,
    provider: "asf",
    dataset: "Sentinel-1",
    output_dir: "/tmp/eostage/s1",
    ...overrides,
  };
}

test.afterEach(() => {
  cleanup();
  clearTestDesktopShellBridge();
  if (typeof window !== "undefined" && window.localStorage) {
    window.localStorage.clear();
  }
});

test("Agent UI 2026-11-19 全量 10 项修复与交互回归套件 (Items 7/9/11/12/13/15.1)", async (suite) => {
  await suite.test(
    "Item 7e: 修复中文 IME 确认后快速按 Enter 变换行 Bug（活跃合成阻塞，确认后快速 Enter 立即发送）",
    () => {
      const guard = createImeEnterGuardState();
      // 1. 活跃 IME 合成期间按 Enter 必须拦截（不发送）
      onImeCompositionStart(guard);
      assert.equal(
        shouldBlockKeyDownForIme(guard, { key: "Enter", nativeIsComposing: true, keyCode: 229, nowMs: 1000 }),
        true
      );
      // 2. WebKit 同一按键同步派发的 compositionend -> keydown(Enter) (< 30ms, 无 keyup) 拦截一次，第二次立即放行
      onImeCompositionEnd(guard, 1010);
      assert.equal(
        shouldBlockKeyDownForIme(guard, { key: "Enter", nativeIsComposing: false, keyCode: 13, nowMs: 1015 }),
        true
      );
      assert.equal(
        shouldBlockKeyDownForIme(guard, { key: "Enter", nativeIsComposing: false, keyCode: 13, nowMs: 1020 }),
        false
      );
      // 3. 按空格选词上屏后抬起空格键 -> 10ms 后紧接按 Enter 必须立即放行发送！
      onImeCompositionStart(guard);
      onImeCompositionEnd(guard, 2000);
      onImeKeyUp(guard, " ");
      assert.equal(
        shouldBlockKeyDownForIme(guard, { key: "Enter", nativeIsComposing: false, keyCode: 13, nowMs: 2010 }),
        false
      );
      // 4. 即使未收到 keyup，60ms 后（远短于旧的 500ms 错误窗口）按 Enter 也必须立即发送！
      onImeCompositionStart(guard);
      onImeCompositionEnd(guard, 3000);
      assert.equal(
        shouldBlockKeyDownForIme(guard, { key: "Enter", nativeIsComposing: false, keyCode: 13, nowMs: 3060 }),
        false
      );

      // 组件级交互验证
      let submits = 0;
      const view = render(
        <I18nProvider>
          <AgentComposer
            value="查询合肥最新哨兵数据"
            onInputChange={() => {}}
            onSubmit={() => {
              submits += 1;
            }}
            channelReadiness={{ ready: true, disabled: false, reason_zh: "" }}
            channels={MULTI_CHANNELS}
            sessionId="sess-ime"
            permissionView={null}
            permissionStatus="ready"
            onPermissionSelect={() => {}}
            onPermissionSetGlobal={() => {}}
          />
        </I18nProvider>
      );
      const textarea = view.getByRole("textbox", { name: "科学需求输入框" });
      fireEvent.compositionStart(textarea);
      fireEvent.keyDown(textarea, { key: "Enter", isComposing: true, keyCode: 229 });
      assert.equal(submits, 0, "IME 合成期间 Enter 绝不能发送");

      fireEvent.compositionEnd(textarea);
      fireEvent.keyUp(textarea, { key: " " });
      fireEvent.keyDown(textarea, { key: "Enter" });
      assert.equal(submits, 1, "中文上屏释放空格后紧接按 Enter 必须立即发送，而非插入换行");
      cleanup();
    }
  );

  await suite.test(
    "Item 7c: `+` 附件菜单加宽并禁止标签折断截断（插件与技能及副标题完整可见，无「插...」或底部裁切）",
    () => {
      const view = render(
        <I18nProvider>
          <AgentComposer
            value=""
            onInputChange={() => {}}
            onSubmit={() => {}}
            channelReadiness={{ ready: true, disabled: false, reason_zh: "" }}
            channels={MULTI_CHANNELS}
            sessionId="sess-menu"
            permissionView={null}
            permissionStatus="ready"
            onPermissionSelect={() => {}}
            onPermissionSetGlobal={() => {}}
          />
        </I18nProvider>
      );

      fireEvent.click(view.getByRole("button", { name: "附加范围或文件" }));
      const menu = view.getByTestId("agent-attach-menu");
      assert.ok(menu.className.includes("w-max"));
      assert.ok(menu.className.includes("min-w-[296px]"));
      assert.equal(menu.className.includes("w-[250px]"), false);

      const skillsItem = view.getByTestId("attach-menu-item-skills-tools");
      const labelSpan = skillsItem.querySelector("span")!;
      assert.ok(labelSpan.className.includes("whitespace-nowrap"));
      assert.equal(labelSpan.className.includes("truncate"), false);
      assert.ok(skillsItem.textContent?.includes("插件与技能"));
      assert.ok(skillsItem.textContent?.includes("Plugin / Skill / MCP"));

      const mediaItem = view.getByTestId("attach-menu-item-media-upload");
      assert.ok(mediaItem.textContent?.includes("添加图片或文件"));
      cleanup();
    }
  );

  await suite.test(
    "Item 7d: EOStage 左右分栏渠道+模型选择器、模型实时搜索、底部思考强度滑块与 Ctrl+T 循环切换",
    async () => {
      const changes: Array<AgentModelSelection | null> = [];
      let settingsOpened = 0;

      const view = render(
        <I18nProvider>
          <AgentModelPicker
            channels={MULTI_CHANNELS}
            sessionId="sess-model-picker"
            ready={true}
            onGoSettings={() => {
              settingsOpened += 1;
            }}
            onChange={(sel) => {
              changes.push(sel);
            }}
          />
        </I18nProvider>
      );

      fireEvent.click(view.getByTestId("agent-model-picker-trigger"));
      assert.ok(view.getByTestId("model-picker-models-pane"));
      assert.equal(view.queryByTestId("model-picker-channels-pane"), null);
      fireEvent.click(view.getByTestId("model-picker-active-channel-header"));
      assert.ok(view.getByTestId("model-picker-channels-pane"));

      // 左侧展示 API 与 订阅 胶囊及底部「管理模型」
      const agyBtn = view.getByTestId("model-picker-channel-AGY");
      const oauthBtn = view.getByTestId("model-picker-channel-openai-codex-oauth");
      assert.ok(agyBtn.textContent?.includes("API"));
      assert.ok(oauthBtn.textContent?.includes("订阅"));
      assert.equal(oauthBtn.textContent?.includes("OAuth"), false);
      assert.ok(view.getByTestId("model-picker-manage-btn").textContent?.includes("管理模型"));

      // 右侧搜索框实时过滤模型
      const searchInput = view.getByTestId("model-picker-search-input");
      fireEvent.change(searchInput, { target: { value: "argon" } });
      assert.ok(view.getByTestId("model-picker-model-gemini-4-argon"));
      assert.equal(view.queryByTestId("model-picker-model-claude-sonnet-4-6"), null);
      fireEvent.change(searchInput, { target: { value: "" } });

      // 切换左侧渠道到 OAuth -> 右侧头部与模型列表即时切换
      fireEvent.click(oauthBtn);
      assert.ok(
        view.getByTestId("model-picker-active-channel-header").textContent?.includes("openai-codex-oauth")
      );
      assert.ok(view.getByTestId("model-picker-model-gpt-5.4"));

      // 切回 AGY 并测试底部思考强度滑块点击 + Ctrl+T
      fireEvent.click(agyBtn);
      const slider = view.getByTestId("reasoning-intensity-slider");
      assert.equal(slider.getAttribute("data-disabled"), "false");

      fireEvent.click(view.getByTestId("reasoning-level-step-high"));
      assert.equal(view.getByTestId("reasoning-intensity-label").textContent, "高");
      assert.deepEqual(changes[changes.length - 1], {
        channel_id: "AGY",
        model_id: "gemini-4-carbon",
        reasoning_level: "high",
      });

      // 点击 Ctrl+T 徽章 -> 从 high 循环至 xhigh（极高）
      fireEvent.click(view.getByTestId("reasoning-intensity-shortcut-btn"));
      assert.equal(view.getByTestId("reasoning-intensity-label").textContent, "极高");
      assert.deepEqual(changes[changes.length - 1], {
        channel_id: "AGY",
        model_id: "gemini-4-carbon",
        reasoning_level: "xhigh",
      });

      // 键盘 Ctrl+T -> 从 xhigh 回绕至 off（关闭）
      fireEvent.keyDown(document, { key: "t", ctrlKey: true });
      assert.equal(view.getByTestId("reasoning-intensity-label").textContent, "关闭");
      assert.deepEqual(changes[changes.length - 1], {
        channel_id: "AGY",
        model_id: "gemini-4-carbon",
        reasoning_level: "off",
      });

      // 直接按住白色圆形滑块拖拽（验证不再被 stopPropagation 吞掉 pointerDown）
      const track = view.getByTestId("reasoning-intensity-track");
      track.getBoundingClientRect = () =>
        ({
          left: 100,
          right: 300,
          width: 200,
          top: 0,
          bottom: 24,
          height: 24,
          x: 100,
          y: 0,
          toJSON: () => ({}),
        }) as DOMRect;
      const thumb = view.getByTestId("reasoning-intensity-thumb");
      fireEvent.pointerDown(thumb, { button: 0, clientX: 100, pointerId: 1 });
      fireEvent.pointerMove(track, { clientX: 250, pointerId: 1 }); // 75% -> index 3 ("high")
      fireEvent.pointerUp(track, { clientX: 250, pointerId: 1 });
      assert.equal(view.getByTestId("reasoning-intensity-label").textContent, "高");
      assert.deepEqual(changes[changes.length - 1], {
        channel_id: "AGY",
        model_id: "gemini-4-carbon",
        reasoning_level: "high",
      });

      // 点击右侧头部收起/展开渠道列表，点击「管理模型」打开模型设置
      const channelHeader = view.getByTestId("model-picker-active-channel-header");
      assert.equal(channelHeader.getAttribute("aria-expanded"), "true");
      fireEvent.click(channelHeader);
      assert.equal(view.queryByTestId("model-picker-channels-pane"), null);
      fireEvent.click(channelHeader);
      fireEvent.click(view.getByTestId("model-picker-manage-btn"));
      assert.equal(settingsOpened, 1);
      cleanup();

      // 分屏窄左栏 + 居中欢迎态校验：使用 fixed Portal 逃逸 section.chat overflow-hidden，防左裁切与顶栏裁切
      const prevInnerWidth = window.innerWidth;
      const prevInnerHeight = window.innerHeight;
      Object.defineProperty(window, "innerWidth", { value: 1440, configurable: true });
      Object.defineProperty(window, "innerHeight", { value: 900, configurable: true });
      try {
        const splitView = render(
          <I18nProvider>
            <section className="chat">
              <AgentModelPicker
                channels={MULTI_CHANNELS}
                sessionId="sess-eostage-clamp"
                ready={true}
                compact={true}
              />
            </section>
          </I18nProvider>
        );
        const chatEl = splitView.container.querySelector("section.chat") as HTMLElement;
        chatEl.getBoundingClientRect = () =>
          ({
            left: 240,
            right: 630,
            width: 390,
            top: 44,
            bottom: 900,
            height: 856,
            x: 240,
            y: 44,
            toJSON: () => ({}),
          }) as DOMRect;
        const trigger = splitView.getByTestId("agent-model-picker-trigger");
        trigger.getBoundingClientRect = () =>
          ({
            left: 500,
            right: 574,
            width: 74,
            top: 424,
            bottom: 450,
            height: 26,
            x: 500,
            y: 424,
            toJSON: () => ({}),
          }) as DOMRect;
        fireEvent.click(trigger);
        const popover = await waitFor(() => splitView.getByTestId("agent-model-picker-popover"));
        // Single-pane 260px fits cleanly inside [248..630]; expanding Box 2 (426px) clamps rightward
        assert.equal(Number(popover.getAttribute("data-clamped-shift") || "0"), 0);
        fireEvent.click(splitView.getByTestId("model-picker-active-channel-header"));
        await waitFor(() => assert.ok(splitView.getByTestId("model-picker-channels-pane")));
        const clampedShift = Number(popover.getAttribute("data-clamped-shift") || "0");
        assert.ok(clampedShift > 0, `expected positive right shift in narrow left split pane, got ${clampedShift}`);
        assert.equal(popover.style.position, "fixed");
        assert.ok(parseInt(popover.style.left, 10) >= 248, `popover left must stay clear of sidebar/chat left edge, got ${popover.style.left}`);
        assert.equal(popover.getAttribute("data-placement"), "down", "centered welcome composer must flip downward instead of clipping past title bar");
      } finally {
        Object.defineProperty(window, "innerWidth", { value: prevInnerWidth, configurable: true });
        Object.defineProperty(window, "innerHeight", { value: prevInnerHeight, configurable: true });
      }
      cleanup();
    }
  );

  await suite.test(
    "Item 7a: 支持剪贴板粘贴图片、拖拽文件、移除附件、空文本发送附件并严格匹配 SendContext.attachments 契约",
    async () => {
      const sentPayloads: Array<{ text: string; context: AgentSendContext | null | undefined }> = [];
      installBridge({
        send: async (_sid, text, context) => {
          sentPayloads.push({ text, context });
          return "run-att-1";
        },
      });

      const view = render(<DesktopAgentWorkbench openSessionId="sess-ui-19" />);
      const textarea = await waitFor(() => view.getByRole("textbox", { name: "科学需求输入框" }));

      // 1. 粘贴 PNG 图片
      const pngFile = new File([new Uint8Array([137, 80, 78, 71])], "radar-preview.png", {
        type: "image/png",
      });
      await act(async () => {
        fireEvent.paste(textarea, {
          clipboardData: {
            items: [{ kind: "file", getAsFile: () => pngFile }],
            files: [pngFile],
          },
        });
      });

      const strip = await waitFor(() => view.getByTestId("composer-attachments-strip"));
      assert.ok(strip.querySelector("img[alt='radar-preview.png']"));

      // 2. 拖拽 JSON 文件附加为文件芯片
      const jsonFile = new File(['{"bbox":[117,31,118,32]}'], "roi.json", {
        type: "application/json",
      });
      await act(async () => {
        fireEvent.drop(textarea.closest(".cds-composer-shell")!, {
          dataTransfer: { files: [jsonFile] },
        });
      });
      await waitFor(() => assert.ok(strip.textContent?.includes("roi.json")));

      // 3. 空文本状态下因为已有附件，发送按钮必须启用，且发出严格匹配 deny_unknown_fields 的 wire 载荷
      const sendBtn = view.getByTestId("agent-send-btn") as HTMLButtonElement;
      assert.equal(sendBtn.disabled, false);
      fireEvent.click(sendBtn);

      await waitFor(() => assert.equal(sentPayloads.length, 1));
      assert.equal(sentPayloads[0].text, "");
      const wireAttachments = (sentPayloads[0].context as any)?.attachments;
      assert.equal(Array.isArray(wireAttachments), true);
      assert.equal(wireAttachments.length, 2);
      assert.equal(wireAttachments[0].kind, "image");
      assert.equal(wireAttachments[0].name, "radar-preview.png");
      assert.equal(wireAttachments[0].mime_type, "image/png");
      assert.ok(String(wireAttachments[0].data_base64 || "").startsWith("data:image/png;base64,"));
      assert.equal(wireAttachments[1].kind, "file");
      assert.equal(wireAttachments[1].name, "roi.json");
      assert.ok(String(wireAttachments[1].text_content || "").includes("bbox"));

      // 严格校验无未知键（匹配后端 #[serde(deny_unknown_fields)]）
      const allowedKeys = new Set(["kind", "name", "mime_type", "data_base64", "text_content", "size_bytes"]);
      for (const att of wireAttachments) {
        for (const key of Object.keys(att)) {
          assert.ok(allowedKeys.has(key), `unexpected wire attachment key: ${key}`);
        }
      }

      // 校验 toWireAttachments 同样去除 UI 私有字段（id / previewUrl）
      const projected = toWireAttachments([
        {
          id: "ui-only-id",
          kind: "image",
          name: "/tmp/secret/path/shot.png",
          mimeType: "image/png",
          sizeBytes: 120,
          previewUrl: "data:image/png;base64,AA==",
          dataBase64: "data:image/png;base64,AA==",
        },
      ]);
      assert.deepEqual(Object.keys(projected[0]).sort(), ["data_base64", "kind", "mime_type", "name", "size_bytes"]);
      assert.equal(projected[0].name, "shot.png");
      cleanup();
    }
  );

  await suite.test(
    "Item 7b & Item 12: 彻底移除 EO 头像块；思考头无停止键，输入框单键就地变形为深色圆角方形停止键",
    async () => {
      const cancelledRuns: string[] = [];
      installBridge({
        runsList: async () => [
          {
            run_id: "run-live-12",
            session_id: "sess-ui-19",
            status: "running",
            started_at: "2026-11-19T00:00:00Z",
            ended_at: null,
          },
        ],
        runCancel: async (rid) => {
          cancelledRuns.push(rid);
          return true;
        },
      });

      const view = render(<DesktopAgentWorkbench openSessionId="sess-ui-19" />);
      await waitFor(() => assert.ok(view.getByTestId("agent-live-pending-thinking")));

      // 无 EO 头像块
      const allSpans = Array.from(view.container.querySelectorAll("span"));
      assert.equal(
        allSpans.some((el) => (el.textContent || "").trim() === "EO"),
        false,
        "思考等待态绝不应渲染 EO 黑色方块头像"
      );

      // 收到 live-delta 后依然无 EO 头像块、无思考卡停止键
      await act(async () => {
        emitLocalDesktopHostEvent("agent:live-delta", {
          session_id: "sess-ui-19",
          run_id: "run-live-12",
          step_index: 0,
          kind: "thinking",
          delta: "正在分析研究区轨道覆盖...",
          started_at_ms: Date.now() - 500,
        });
      });
      await waitFor(() => assert.ok(view.getByTestId("agent-live-delta-preview")));
      assert.equal(
        Array.from(view.container.querySelectorAll("span")).some(
          (el) => (el.textContent || "").trim() === "EO"
        ),
        false,
        "实时流式思考与正文预览绝不应渲染 EO 黑色方块头像"
      );

      assert.equal(view.queryByTestId("agent-thinking-stop-btn"), null);
      assert.equal(view.queryByTestId("agent-composer-stop-btn"), null);

      const morphStopBtn = view.getByTestId("agent-send-btn") as HTMLButtonElement;
      assert.equal(morphStopBtn.getAttribute("data-action-mode"), "stop");
      assert.equal(morphStopBtn.getAttribute("aria-label"), "停止当前运行");
      assert.equal(morphStopBtn.disabled, false);

      fireEvent.click(morphStopBtn);
      await waitFor(() => assert.deepEqual(cancelledRuns, ["run-live-12"]));

      cleanup();

      // 独立组件确认 UnifiedTurnProcessHeader 也不再渲染停止键
      const headerView = render(
        <I18nProvider>
          <UnifiedTurnProcessHeader
            turnKey="t-1"
            isActive={true}
            isExpanded={true}
            onToggle={() => {}}
            onStop={() => {}}
            toolStepCount={1}
          />
        </I18nProvider>
      );
      assert.equal(headerView.queryByTestId("agent-thinking-stop-btn"), null);
      cleanup();
    }
  );

  await suite.test(
    "Item 9: classifyStreamEvents 保留 target_label / input_summary / output_summary 并于工具展开抽屉真实展示",
    () => {
      const rawEvents: AgentStreamEvent[] = [
        {
          seq: 1,
          kind: "tool_round",
          payload: {
            run_id: "run-9",
            call_id: "c-1",
            tool_id: "source_list",
            status: "running",
            target_label: "卫星目录",
            input_summary: "category=all",
          },
        },
        {
          seq: 2,
          kind: "tool_round",
          payload: {
            run_id: "run-9",
            call_id: "c-1",
            tool_id: "source_list",
            status: "done",
            duration_ms: 25,
            result_count: 10,
            output_summary: "已返回 10 个可用数据源（Sentinel-1 / NISAR / Landsat / DEM 等）",
          },
        },
      ];

      const classified = classifyStreamEvents(rawEvents);
      assert.equal(classified.length, 1);
      const group = classified[0] as StreamToolGroupItem;
      assert.equal(group.kind, "tool_group");
      assert.equal(group.rounds[0].target_label, "卫星目录");
      assert.equal(group.rounds[0].input_summary, "category=all");
      assert.equal(
        group.rounds[0].output_summary,
        "已返回 10 个可用数据源（Sentinel-1 / NISAR / Landsat / DEM 等）"
      );

      const view = render(
        <I18nProvider>
          <AgentToolGroupCard item={group} isExpanded={true} />
        </I18nProvider>
      );
      fireEvent.click(view.getByTestId("agent-tool-row-trigger"));
      const drawer = view.getByTestId("agent-tool-row-detail-drawer");
      assert.ok(drawer.textContent?.includes("传入参数："));
      assert.ok(drawer.textContent?.includes("category=all"));
      assert.ok(drawer.textContent?.includes("返回结果："));
      assert.ok(drawer.textContent?.includes("已返回 10 个可用数据源"));
      cleanup();
    }
  );

  await suite.test(
    "Item 11: AgentErrorCard 采用单色极简卡片（无粉红底）、真实脱敏上游报错、且所有按钮单行不换行",
    () => {
      const errItem: StreamErrorItem = {
        kind: "error",
        seq: 5,
        run_id: "run-err-11",
        code: "model_unknown",
        status: "failed",
        plan_id: null,
        message_zh: "HTTP 502（上游网关暂时异常）：Bad Gateway",
      };

      const view = render(
        <I18nProvider>
          <AgentErrorCard
            item={errItem}
            onResumeTurn={() => {}}
            onRetry={() => {}}
            onOpenModelSettings={() => {}}
            alternateModels={[
              {
                channelId: "openai-codex-oauth",
                modelId: "gpt-5.4",
                label: "openai-codex-oauth · GPT-5.4",
              },
              {
                channelId: "AGY",
                modelId: "gemini-4-carbon",
                label: "AGY · gemini-4-carbon",
              },
            ]}
            onSwitchModelAndResume={() => {}}
          />
        </I18nProvider>
      );

      const card = view.getByTestId("agent-error-card");
      assert.equal(card.className.includes("bg-sp-danger-bg"), false, "错误卡不得使用刺眼粉红底 bg-sp-danger-bg");
      assert.ok(card.className.includes("bg-sp-surface"));
      assert.ok(card.className.includes("border-sp-line"));
      assert.ok(card.textContent?.includes("HTTP 502（上游网关暂时异常）：Bad Gateway"));
      assert.equal(card.textContent?.includes("模型服务返回未知错误"), false);

      // Owner 2026-11-21: 错误卡精简为只展示清晰报错理由与重试（认证类错误才显示前往模型设置）
      assert.equal(view.queryByTestId("agent-error-switch-model-btn"), null);
      assert.equal(view.queryByTestId("agent-error-resume-btn"), null);
      assert.equal(view.queryByTestId("btn-error-open-model-settings"), null);
      const retryBtn = view.getByTestId("btn-error-retry");
      assert.ok(retryBtn.className.includes("whitespace-nowrap"));
      cleanup();
    }
  );

  await suite.test(
    "Item 13: Agent 分屏地图右上角图例与定位范围上移至 top-3（移除遗留 top-14 偏移）",
    () => {
      const workbenchPath = path.resolve(
        path.dirname(fileURLToPath(import.meta.url)),
        "../DesktopAgentWorkbench.tsx"
      );
      const source = fs.readFileSync(workbenchPath, "utf8");
      assert.ok(source.includes('topOverlayOffsetClass="top-3"'));
      assert.equal(source.includes('topOverlayOffsetClass="top-14"'), false);
    }
  );

  await suite.test(
    "Item 15.1: 错误卡后台下载提示严格按当前会话隔离，零下载对话绝不误报其他会话或历史队列任务",
    async () => {
      const unrelatedRunning = makeTask({ task_id: "unrelated-running-1", status: "running" });
      const unrelatedQueued = makeTask({ task_id: "unrelated-queued-2", status: "queued" });
      const sessionPaused = makeTask({ task_id: "sess-paused-3", status: "paused" });
      const sessionRunning = makeTask({ task_id: "sess-running-4", status: "running" });

      // 1. 当前会话未创建任何任务 -> 即使全局存在运行中/排队中任务，也必须返回 0
      assert.equal(
        countSessionActiveDownloads({
          activeSessionId: "sess-empty",
          events: [],
          sessionWork: [],
          tasks: [unrelatedRunning, unrelatedQueued],
        }),
        0
      );

      // 2. 当前会话仅创建了已暂停任务 -> 返回 0
      const pausedEvents: AgentStreamEvent[] = [
        {
          seq: 1,
          kind: "progress_card",
          payload: {
            run_id: "r1",
            plan_id: "p1",
            status: "done",
            created_task_ids: ["sess-paused-3"],
          },
        },
      ];
      assert.equal(
        countSessionActiveDownloads({
          activeSessionId: "sess-p",
          events: pausedEvents,
          sessionWork: [],
          tasks: [unrelatedRunning, sessionPaused],
        }),
        0
      );

      // 3. 当前会话创建了真实运行中任务 -> 精确返回 1（不计入 unrelatedRunning）
      const runningEvents: AgentStreamEvent[] = [
        {
          seq: 1,
          kind: "progress_card",
          payload: {
            run_id: "r2",
            plan_id: "p2",
            status: "done",
            created_task_ids: ["sess-running-4"],
          },
        },
      ];
      assert.equal(
        countSessionActiveDownloads({
          activeSessionId: "sess-r",
          events: runningEvents,
          sessionWork: [],
          tasks: [unrelatedRunning, sessionPaused, sessionRunning],
        }),
        1
      );

      // 4. DesktopAgentWorkbench 集成验证：零下载会话出错时，绝不渲染后台下载提示横幅
      installBridge({
        streamPoll: async (_sid, cursor) =>
          cursor === 0
            ? {
                events: [
                  {
                    seq: 1,
                    kind: "message",
                    payload: { run_id: "run-err", role: "user", text: "为什么缓存命中率那么低？" },
                  },
                  {
                    seq: 2,
                    kind: "error",
                    payload: {
                      run_id: "run-err",
                      code: "model_transient",
                      status: "failed",
                      plan_id: null,
                      message_zh: "上游连接超时",
                    },
                  },
                ],
                cursor: 2,
              }
            : { events: [], cursor: 2 },
        runsList: async () => [
          {
            run_id: "run-err",
            session_id: "sess-ui-19",
            status: "failed",
            started_at: "2026-11-19T00:00:00Z",
            ended_at: "2026-11-19T00:00:02Z",
          },
        ],
      });

      const view = render(
        <DesktopAgentWorkbench
          openSessionId="sess-ui-19"
          agentTasks={[unrelatedRunning, unrelatedQueued]}
        />
      );
      await waitFor(() => assert.ok(view.getByTestId("agent-error-card")));
      assert.equal(
        view.queryByTestId("agent-error-active-downloads-banner"),
        null,
        "本会话未创建任何下载任务时，错误卡绝不能显示「当前有 N 个后台下载任务正在独立运行」"
      );
    }
  );
});
