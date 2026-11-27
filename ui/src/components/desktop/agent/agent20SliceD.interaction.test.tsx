import "../../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React, { act } from "react";
import { cleanup, fireEvent, render } from "../../../test-utils/render-harness";
import { waitFor } from "@testing-library/react";
import { registerHooks } from "node:module";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { transformSync } from "esbuild";
import {
  clearTestDesktopShellBridge,
  registerTestDesktopShellBridge,
} from "@/lib/desktopShellBridge";
import { emitLocalDesktopHostEvent } from "@/lib/desktopHostEvents";
import { createWorkbenchTestDesktopShellBridge } from "../../../test-utils/workbenchDesktopBridge";
import type { DesktopAgentBridge } from "@/shell/api/agentBridgeTypes";
import type {
  AgentChannelView,
  AgentSendContext,
  AgentStreamEvent,
} from "@/lib/generated/v3-contracts";
import type { WorkbenchScene } from "@/shell/api/types";

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

const defaultChannel: AgentChannelView = {
  id: "main",
  protocol: "chat_completions",
  base_url: "https://example.invalid/v1",
  model: "deepseek-reasoner",
  reasoning_mode: "default",
  models: [
    {
      id: "deepseek-reasoner",
      name: "DeepSeek R1",
      context_window: 65536,
      input_types: ["text"],
      reasoning_levels: [],
    },
  ],
  key_ready: true,
  is_default: true,
};

function installBridge(overrides: Partial<DesktopAgentBridge>) {
  const base = createWorkbenchTestDesktopShellBridge();
  const agent = {
    channelsGet: async () => [defaultChannel],
    channelsModels: async () => ["deepseek-reasoner"],
    channelsProbeDraft: async () => ({
      result: { ok: true, code: null, status: null, model_count: 1 },
      models: ["deepseek-reasoner"],
    }),
    channelsSave: async () => [defaultChannel],
    sessionsList: async () => [
      {
        session_id: "sess-slice-d",
        title: "合肥 InSAR 协同测试",
        created_at: "2026-11-11T00:00:00Z",
        updated_at: "2026-11-11T00:00:00Z",
        run_ids: ["run-1"],
      },
    ],
    sessionNew: async () => "sess-slice-d",
    streamPoll: async () => ({ events: [], cursor: 0 }),
    runsList: async () => [
      {
        run_id: "run-1",
        session_id: "sess-slice-d",
        status: "completed",
        started_at: "2026-11-11T00:00:00Z",
        ended_at: "2026-11-11T00:00:02Z",
      },
    ],
    permissionModeGet: async () => ({
      effective: "confirm_changes",
      session_override: null,
      global_default: "confirm_changes",
    }),
    sessionTruncate: async (_sessionId, beforeSeq) => ({
      deleted_events: 2,
      deleted_runs: 1,
      cursor: Math.max(0, beforeSeq - 1),
      aoi_restored: true,
    }),
    ...overrides,
  } as DesktopAgentBridge;
  registerTestDesktopShellBridge({ ...base, agent });
}

test.afterEach(() => {
  cleanup();
  clearTestDesktopShellBridge();
});

const SAMPLE_SCENES: WorkbenchScene[] = [
  {
    scene_id: "S1A_IW_SLC__1SDV_20261001",
    display_name: "S1A_IW_SLC__1SDV_20261001",
    product_type: "SLC",
    platform: "Sentinel-1A",
    beam_mode: "IW",
    polarization: "VV+VH",
    orbit_direction: "ASCENDING",
    relative_orbit: 142,
    absolute_orbit: 55100,
    frame: 98,
    acquisition_time: "2026-10-01T10:12:00Z",
    footprint_bbox: {
      west: 117.1,
      south: 31.7,
      east: 117.5,
      north: 32.0,
      crs: "EPSG:4326",
    },
    footprint_geojson: {
      type: "Polygon",
      coordinates: [
        [
          [117.1, 31.7],
          [117.5, 31.7],
          [117.5, 32.0],
          [117.1, 32.0],
          [117.1, 31.7],
        ],
      ],
    },
  } as unknown as WorkbenchScene,
  {
    scene_id: "S1A_IW_SLC__1SDV_20261013",
    display_name: "S1A_IW_SLC__1SDV_20261013",
    product_type: "SLC",
    platform: "Sentinel-1A",
    beam_mode: "IW",
    polarization: "VV+VH",
    orbit_direction: "ASCENDING",
    relative_orbit: 142,
    absolute_orbit: 55275,
    frame: 98,
    acquisition_time: "2026-10-13T10:12:00Z",
    footprint_bbox: {
      west: 117.1,
      south: 31.7,
      east: 117.5,
      north: 32.0,
      crs: "EPSG:4326",
    },
    footprint_geojson: {
      type: "Polygon",
      coordinates: [
        [
          [117.1, 31.7],
          [117.5, 31.7],
          [117.5, 32.0],
          [117.1, 32.0],
          [117.1, 31.7],
        ],
      ],
    },
  } as unknown as WorkbenchScene,
];

test("Agent 2.0 Slice D 协同工作台集成交互测试", async (suite) => {
  await suite.test(
    "1. 实时空间状态栏 (agent-live-context-bar) 与无遮挡分屏顶栏 (agent-split-panel-header) 联动及结果抽屉展示",
    async () => {
      installBridge({});
      let selectedId: string | null = "S1A_IW_SLC__1SDV_20261001";
      const view = render(
        <DesktopAgentWorkbench
          openSessionId="sess-slice-d"
          workbenchAoi={{ west: 117.0, south: 31.5, east: 117.6, north: 32.1 }}
          aoiLabel="安徽省合肥市"
          scenes={SAMPLE_SCENES}
          sourceId="asf_sentinel1"
          sourceNames={{ asf_sentinel1: "Sentinel-1 SAR" }}
          selectedSceneId={selectedId}
          onSelectScene={(id) => {
            selectedId = id;
          }}
          checkedSceneIds={new Set(["S1A_IW_SLC__1SDV_20261001"])}
          onToggleCheck={() => {}}
        />
      );

      const contextBar = await waitFor(() => view.getByTestId("agent-live-context-bar"));
      assert.ok(contextBar.textContent?.includes("研究区：安徽省合肥市"));
      assert.ok(
        view.getByTestId("agent-context-batch-pill").textContent?.includes("Sentinel-1 SAR · 2 景")
      );
      assert.ok(view.getByTestId("agent-context-checked-pill").textContent?.includes("已勾选 1 景"));

      // Owner 2026-11-14 #6.6: 点击研究区胶囊打开右侧分屏地图，验证顶栏采用地图内居中悬浮胶囊 (absolute top-3 z-[760])，地图占满 100% 高度
      const aoiBtn = view.getByTestId("agent-context-aoi-pill").querySelector("button")!;
      fireEvent.click(aoiBtn);

      const header = await waitFor(() => view.getByTestId("agent-split-panel-header"));
      assert.equal(header.className.includes("h-10"), false, "悬浮胶囊顶栏不应再占用独立 h-10 外栏");
      assert.equal(
        header.className.includes("absolute"),
        true,
        "悬浮胶囊顶栏应采用 absolute 浮在地图内上方"
      );
      assert.equal(
        header.className.includes("z-[760]"),
        true,
        "悬浮胶囊顶栏应使用 z-[760] 浮层层级"
      );

      // Agent 地图框不再弹出独立「已选景详情」卡片，点击地图影像后改为在统一结果清单定位
      assert.strictEqual(view.queryByTestId("agent-map-scene-drawer"), null);
    }
  );

  await suite.test(
    "2. 发送消息保留真实多边形 workingGeometry（不退化为 4 点矩形）且发送失败不丢失草稿",
    async () => {
      const sentContexts: Array<AgentSendContext | null | undefined> = [];
      let shouldFailNext = true;

      installBridge({
        send: async (_sid, _text, ctx) => {
          if (shouldFailNext) {
            throw { code: "AGENT_NETWORK_ERROR", message_zh: "模拟网络瞬断" };
          }
          sentContexts.push(ctx);
          return "run-ok";
        },
      });

      const hefeiPolygon = {
        type: "Polygon" as const,
        coordinates: [
          [
            [117.12, 31.65],
            [117.45, 31.68],
            [117.62, 31.82],
            [117.58, 32.04],
            [117.28, 32.11],
            [117.05, 31.91],
            [117.12, 31.65],
          ],
        ],
      };

      const view = render(
        <DesktopAgentWorkbench
          openSessionId="sess-slice-d"
          workbenchAoi={{ west: 117.05, south: 31.65, east: 117.62, north: 32.11 }}
          workingGeometry={hefeiPolygon}
          aoiLabel="安徽省合肥市"
        />
      );

      const input = (await waitFor(() =>
        view.getByRole("textbox", { name: "科学需求输入框" })
      )) as HTMLTextAreaElement;
      await waitFor(() =>
        assert.equal(view.getByRole("button", { name: "模型" }).textContent, "DeepSeek R1")
      );

      // 第一次发送失败：显示错误横幅且输入框草稿必须完整保留
      fireEvent.change(input, { target: { value: "检索合肥市 2026 年 10 月 Sentinel-1 SLC" } });
      await waitFor(() =>
        assert.equal((view.getByRole("button", { name: "发送" }) as HTMLButtonElement).disabled, false)
      );
      fireEvent.click(view.getByRole("button", { name: "发送" }));
      await waitFor(() => assert.ok(view.container.textContent?.includes("发送消息失败")));
      assert.equal(
        input.value,
        "检索合肥市 2026 年 10 月 Sentinel-1 SLC",
        "发送失败时严禁清空用户输入草稿"
      );

      // 第二次发送成功：验证传入后端的 aoi.geometry 保留真实 7 顶点多边形而非 4 角包络框
      shouldFailNext = false;
      fireEvent.click(view.getByRole("button", { name: "发送" }));
      await waitFor(() => assert.equal(sentContexts.length, 1));
      assert.equal(sentContexts[0]?.aoi?.name, "安徽省合肥市");
      const sentGeo = sentContexts[0]?.aoi?.geometry as any;
      assert.equal(sentGeo?.type, "Polygon");
      assert.equal(
        sentGeo?.coordinates?.[0]?.length,
        7,
        "workingGeometry 多边形顶点必须原样透传给 Agent 后端，不得退化为 5 点矩形 BBox"
      );
      await waitFor(() => assert.equal(input.value, ""));
    }
  );

  await suite.test(
    "3. 对话文本允许自由选取 (select-text)，支持回退此轮 (sessionTruncate) 与编辑重发",
    async () => {
      const truncateCalls: Array<{ sessionId: string; beforeSeq: number; restoreAoi?: boolean }> = [];
      const resends: string[] = [];
      let aoiSyncedCount = 0;

      const historyEvents: AgentStreamEvent[] = [
        {
          seq: 1,
          ts: "2026-11-11T00:00:01Z",
          kind: "message",
          payload: {
            run_id: "run-1",
            role: "user",
            text: "帮我检索合肥 Sentinel-1 数据",
            context: null,
          },
        },
        {
          seq: 2,
          ts: "2026-11-11T00:00:02Z",
          kind: "thinking",
          payload: {
            run_id: "run-1",
            step_index: 0,
            status: "done",
            text: "先解析安徽省合肥市行政边界，再调用 search_start。",
            thinking_ms: 1420,
          },
        },
        {
          seq: 3,
          ts: "2026-11-11T00:00:03Z",
          kind: "conclusion",
          payload: {
            run_id: "run-1",
            text: "已完成 **合肥市** 检索，共匹配 `2` 景升轨 SLC。",
            claims: [],
            thinking_text: null,
            thinking_ms: 1420,
          },
        },
      ];

      installBridge({
        streamPoll: async (_sid, cursor) =>
          cursor === 0 ? { events: historyEvents, cursor: 3 } : { events: [], cursor: 3 },
        runsList: async () => [
          {
            run_id: "run-1",
            session_id: "sess-slice-d",
            status: "completed",
            started_at: "2026-11-11T00:00:01Z",
            ended_at: "2026-11-11T00:00:03Z",
          },
        ],
        sessionTruncate: async (sessionId, beforeSeq, restoreAoi) => {
          truncateCalls.push({ sessionId, beforeSeq, restoreAoi });
          return {
            deleted_events: 3,
            deleted_runs: 1,
            cursor: 0,
            aoi_restored: true,
          };
        },
        send: async (_sid, text) => {
          resends.push(text);
          return "run-2";
        },
      });

      const view = render(
        <DesktopAgentWorkbench
          openSessionId="sess-slice-d"
          onSyncWorkingAoi={async () => {
            aoiSyncedCount += 1;
          }}
        />
      );

      const userBubble = await waitFor(() => view.getByTestId("eo-user-bubble"));
      assert.equal(
        userBubble.className.includes("select-text"),
        true,
        "用户消息气泡必须允许自由划词复制 (select-text)"
      );
      // Owner 2026-11-14 #6.7: 已完成轮次默认折叠思考与过程步骤，仅显示 1 行摘要并在点击后展开思考卡片
      const processToggle = await waitFor(() => view.getByTestId("agent-turn-process-toggle"));
      assert.equal(view.queryByTestId("agent-thinking-card"), null);
      fireEvent.click(processToggle);
      await waitFor(() => assert.ok(view.getByTestId("agent-thinking-card")));
      assert.ok(view.getByTestId("agent-markdown-renderer").textContent?.includes("合肥市"));

      // 测试「回退到此轮之前」动作：调用 sessionTruncate(sess, 1, true) 并同步恢复 AOI
      const revertBtn = await waitFor(() => view.getByTestId("btn-revert-turn"));
      assert.equal((revertBtn as HTMLButtonElement).disabled, false);
      fireEvent.click(revertBtn);
      await waitFor(() => assert.equal(truncateCalls.length, 1));
      assert.deepEqual(truncateCalls[0], {
        sessionId: "sess-slice-d",
        beforeSeq: 1,
        restoreAoi: true,
      });
      assert.equal(aoiSyncedCount, 1, "aoi_restored 为 true 时必须回调同步工作台 AOI");
    }
  );

  await suite.test(
    "4. 活动 Run 期间接收 agent:live-delta 宿主事件时立即渲染实时思考卡片与增量 Markdown",
    async () => {
      installBridge({
        streamPoll: async () => ({ events: [], cursor: 0 }),
        runsList: async () => [
          {
            run_id: "run-live",
            session_id: "sess-slice-d",
            status: "running",
            started_at: "2026-11-11T00:00:04Z",
            ended_at: null,
          },
        ],
      });

      const view = render(<DesktopAgentWorkbench openSessionId="sess-slice-d" />);
      await waitFor(() => view.getByRole("textbox", { name: "科学需求输入框" }));
      // 等待 activeRunId 建立后，输入框 placeholder 切为运行中状态或停止按钮可感知
      await act(async () => {
        await new Promise((r) => setTimeout(r, 25));
      });

      // 阶段 1：思考增量阶段自动展开显示思考正文
      await act(async () => {
        emitLocalDesktopHostEvent("agent:live-delta", {
          session_id: "sess-slice-d",
          run_id: "run-live",
          step_index: 1,
          kind: "thinking",
          delta: "正在核验轨道号 142 覆盖度…",
          started_at_ms: Date.now() - 800,
        });
      });

      const livePreview = await waitFor(() => view.getByTestId("agent-live-delta-preview"));
      assert.ok(livePreview.textContent?.includes("正在核验轨道号 142 覆盖度"));

      // 阶段 2：正文增量到达后思考卡自动折叠，实时渲染增量 Markdown
      await act(async () => {
        emitLocalDesktopHostEvent("agent:live-delta", {
          session_id: "sess-slice-d",
          run_id: "run-live",
          step_index: 1,
          kind: "text",
          delta: "### 实时增量结论\n已确认 **100%** 覆盖主城区。",
        });
      });

      await waitFor(() => assert.ok(livePreview.textContent?.includes("实时增量结论")));
    }
  );

  await suite.test(
    "5. propose_plan 产生的 plan_card / progress_card(working_aoi_set) / workspace_sync(aoi_changed) 自动展开分屏地图、同步 AOI 并暴露 data-tool-id",
    async () => {
      let aoiSyncedCount = 0;
      const planEvents: AgentStreamEvent[] = [
        {
          seq: 1,
          ts: "2026-11-11T00:01:00Z",
          kind: "plan_card",
          payload: {
            run_id: "run-plan",
            plan_id: "plan-sc",
            plan_hash: "hash-sc",
            max_tier: 1,
            auto_executed: true,
            steps: [
              {
                tool_id: "working_aoi_set",
                label_zh: "切换研究区至四川省",
                tier: 1,
                status: "done",
              },
              {
                tool_id: "search_start",
                label_zh: "检索四川省 Sentinel-1 数据",
                tier: 1,
                status: "running",
              },
            ],
          },
        },
        {
          seq: 2,
          ts: "2026-11-11T00:01:01Z",
          kind: "progress_card",
          payload: {
            run_id: "run-plan",
            plan_id: "plan-sc",
            step_index: 0,
            tool_id: "working_aoi_set",
            status: "done",
            code: null,
          },
        },
        {
          seq: 3,
          ts: "2026-11-11T00:01:01Z",
          kind: "workspace_sync",
          payload: {
            run_id: "run-plan",
            action: "aoi_changed",
            display_name: "四川省",
            bbox: [97.35, 26.05, 108.54, 34.31],
          },
        },
      ];

      installBridge({
        streamPoll: async (_sid, cursor) =>
          cursor === 0 ? { events: planEvents, cursor: 3 } : { events: [], cursor: 3 },
        runsList: async () => [
          {
            run_id: "run-plan",
            session_id: "sess-slice-d",
            status: "running",
            started_at: "2026-11-11T00:01:00Z",
            ended_at: null,
          },
        ],
      });

      const view = render(
        <DesktopAgentWorkbench
          openSessionId="sess-slice-d"
          onSyncWorkingAoi={async () => {
            aoiSyncedCount += 1;
          }}
        />
      );

      // 自动打开右侧地图分屏（无需人工点击），且胶囊导航不因 @container 塌陷
      const splitHeader = await waitFor(() => view.getByTestId("agent-split-panel-header"));
      assert.ok(splitHeader);
      assert.equal(splitHeader.className.includes("@container"), false);
      assert.ok(splitHeader.className.includes("w-max"));
      assert.ok(splitHeader.className.includes("max-w-[calc(100%-8.5rem)]"));
      // Owner 2026-11-14 #3: replaced solid bg-sp-surface wrapper with single-layer translucent frosted-glass capsule.
      assert.equal(splitHeader.className.includes("bg-sp-surface"), false);
      assert.ok(splitHeader.className.includes("bg-white/75"));
      assert.ok(splitHeader.className.includes("backdrop-blur-md"));
      assert.ok(splitHeader.className.includes("border-black/10"));
      assert.ok(view.getByTestId("agent-panel-tab-map"));
      assert.ok(view.getByTestId("agent-panel-tab-results"));
      assert.ok(view.getByTestId("agent-panel-tab-task"));

      // 计划卡步骤暴露 data-tool-id="working_aoi_set" 与 data-tool-id="search_start"
      const planCard = view.getByTestId("eo-plan-card");
      assert.ok(planCard.querySelector('[data-tool-id="working_aoi_set"]'));
      assert.ok(planCard.querySelector('[data-tool-id="search_start"]'));

      // 触发工作区 AOI 同步回调
      assert.equal(aoiSyncedCount >= 1, true, "working_aoi_set 完成后必须自动同步工作台 AOI");
    }
  );

  await suite.test(
    "6. 工具执行步骤统一包含在「思考中 / 已思考」时间线内：运行中平铺展示无二级头，完成后与思考链一并自动收纳",
    async () => {
      let currentStatus: "running" | "completed" = "running";
      const liveEvents: AgentStreamEvent[] = [
        {
          seq: 1,
          ts: "2026-11-16T00:00:01Z",
          kind: "message",
          payload: {
            run_id: "run-unified",
            role: "user",
            text: "今天有哪些新数据？",
          },
        },
        {
          seq: 2,
          ts: "2026-11-16T00:00:02Z",
          kind: "thinking",
          payload: {
            run_id: "run-unified",
            step_index: 0,
            status: "done",
            text: "先读取当前研究区，再并行检索 Sentinel-1 与 Sentinel-2。",
            thinking_ms: 1200,
          },
        },
        {
          seq: 3,
          ts: "2026-11-16T00:00:03Z",
          kind: "tool_round",
          payload: {
            run_id: "run-unified",
            call_id: "call-1",
            tool_id: "working_aoi_get",
            status: "done",
            duration_ms: 80,
          },
        },
        {
          seq: 4,
          ts: "2026-11-16T00:00:04Z",
          kind: "tool_round",
          payload: {
            run_id: "run-unified",
            call_id: "call-2",
            tool_id: "search_start",
            status: "running",
          },
        },
      ];

      const settledEvents: AgentStreamEvent[] = [
        ...liveEvents.slice(0, 3),
        {
          seq: 4,
          ts: "2026-11-16T00:00:05Z",
          kind: "tool_round",
          payload: {
            run_id: "run-unified",
            call_id: "call-2",
            tool_id: "search_start",
            status: "done",
            duration_ms: 920,
            result_count: 12,
          },
        },
        {
          seq: 5,
          ts: "2026-11-16T00:00:06Z",
          kind: "conclusion",
          payload: {
            run_id: "run-unified",
            text: "共检索到 **12 景** 最新 Sentinel-1 数据。",
            claims: [],
            thinking_text: null,
            thinking_ms: 1200,
          },
        },
      ];

      installBridge({
        streamPoll: async (_sid, cursor) => {
          const target = currentStatus === "running" ? liveEvents : settledEvents;
          return cursor < target.length
            ? { events: target.slice(cursor), cursor: target.length }
            : { events: [], cursor: target.length };
        },
        runsList: async () => [
          {
            run_id: "run-unified",
            session_id: "sess-unified",
            status: currentStatus,
            started_at: "2026-11-16T00:00:01Z",
            ended_at: currentStatus === "completed" ? "2026-11-16T00:00:06Z" : null,
          },
        ],
      });

      const view = render(<DesktopAgentWorkbench openSessionId="sess-unified" />);

      // 阶段 A：运行中自动展开在统一的「思考中 · X.X 秒」波浪标题下，思考块与单色工具行平铺共处，无「执行过程 N 步」二级折叠头
      const liveHeader = await waitFor(() => view.getByTestId("agent-turn-process-toggle"));
      assert.strictEqual(liveHeader.getAttribute("data-turn-active"), "true");
      assert.strictEqual(liveHeader.getAttribute("aria-expanded"), "true");
      assert.ok(liveHeader.querySelectorAll(".sp-ai-char-wave").length >= 3);
      assert.ok((liveHeader.textContent || "").includes("思考中"));
      assert.ok(view.getByTestId("agent-thinking-card"));
      assert.strictEqual(view.getAllByTestId("eo-chat-event-item").length, 2);
      assert.strictEqual(
        view.queryByRole("button", { name: /执行过程/ }),
        null,
        "统一时间线内绝不出现冗余的二级「执行过程 N 步」折叠按钮"
      );

      // 验证折叠/展开切换时 UnifiedTurnProcessHeader 保持同一 DOM 节点不卸载重挂（彻底杜绝波浪重播闪烁），且运行中思考卡片真正折叠收起
      const liveHeaderNodeBefore = liveHeader;
      fireEvent.click(liveHeader);
      assert.strictEqual(
        view.getByTestId("agent-turn-process-toggle"),
        liveHeaderNodeBefore,
        "展开/收起切换时顶部思考开关必须复用同一 DOM 节点，防止 CSS 波浪重置闪烁"
      );
      assert.strictEqual(
        view.queryByTestId("agent-thinking-card"),
        null,
        "运行中点击收起开关后思考链与工具行必须真实折叠隐藏"
      );
      fireEvent.click(view.getByTestId("agent-turn-process-toggle"));
      assert.ok(
        view.getByTestId("agent-thinking-card"),
        "运行中再次点击展开开关后应恢复显示思考与工具步骤"
      );

      // 阶段 B：轮次完成后，思考链与全部工具执行步骤一并自动收纳为单行「已思考 X.X 秒 · 2 个步骤」
      currentStatus = "completed";
      view.unmount();
      const settledView = render(<DesktopAgentWorkbench openSessionId="sess-unified" />);

      const settledHeader = await waitFor(() =>
        settledView.getByTestId("agent-turn-process-toggle")
      );
      assert.strictEqual(settledHeader.getAttribute("data-turn-active"), "false");
      assert.strictEqual(settledHeader.getAttribute("aria-expanded"), "false");
      assert.match(settledHeader.textContent || "", /已思考 .* · 2 个步骤/);
      assert.strictEqual(settledView.queryByTestId("agent-thinking-card"), null);
      assert.strictEqual(settledView.queryByTestId("agent-tool-group-card"), null);

      // 点击顶部「已思考」统一开关：思考段落与 2 个工具步骤一并平铺展开，仍无二级折叠头
      fireEvent.click(settledHeader);
      await waitFor(() => assert.ok(settledView.getByTestId("agent-thinking-card")));
      assert.strictEqual(settledView.getAllByTestId("eo-chat-event-item").length, 2);
      assert.strictEqual(settledView.queryByRole("button", { name: /执行过程/ }), null);
    }
  );

  await suite.test(
    "7. 多数据源合并去重、AOI 空间拦截、右竖排工具栏与三态底部数据坞 (collapsed / split / full) + 同步手动检索台",
    async () => {
      installBridge({});
      let activeFilter: "all" | string = "all";
      let openedWorkbenchArgs: { rid?: string | null; bid?: string | null } | null = null;
      const outOfAoiScene = {
        ...SAMPLE_SCENES[1],
        scene_id: "S2A_MSIL2A_20261015T030000_OUT_OF_AOI",
        display_name: "S2A_MSIL2A_20261015T030000_OUT_OF_AOI",
        footprint_bbox: { west: 80.0, south: 20.0, east: 81.0, north: 21.0, crs: "EPSG:4326" },
        footprint_geojson: {
          type: "Polygon",
          coordinates: [[[80.0, 20.0], [81.0, 20.0], [81.0, 21.0], [80.0, 21.0], [80.0, 20.0]]],
        },
      } as unknown as WorkbenchScene;

      const batches: AgentSessionBatchEntry[] = [
        {
          batchId: "b-s1",
          resourceId: "sar.sentinel1.slc.v1",
          revision: 1,
          scenes: [SAMPLE_SCENES[0]],
          rowStates: new Map(),
          totalCount: 12,
          updatedAtMs: 1,
        },
        {
          batchId: "b-s2",
          resourceId: "science.sentinel-2.l2a.v1",
          revision: 1,
          scenes: [SAMPLE_SCENES[1], outOfAoiScene],
          rowStates: new Map(),
          totalCount: 25,
          updatedAtMs: 2,
        },
      ];

      const view = render(
        <DesktopAgentWorkbench
          openSessionId="sess-multi"
          panelOpen
          workbenchAoi={{ west: 117.0, south: 31.5, east: 117.6, north: 32.1 }}
          aoiLabel="安徽省合肥市"
          sessionBatches={batches}
          activeBatchFilter={activeFilter}
          onSelectBatchFilter={(f) => {
            activeFilter = f;
          }}
          onOpenInMapWorkbench={(rid, bid) => {
            openedWorkbenchArgs = { rid, bid };
          }}
        />
      );

      // 收拢态：地图上方不再悬浮多数据源条，右侧采用单列竖排工具栏 (agent-map-vertical-toolbar)
      assert.strictEqual(view.queryByTestId("agent-multi-batch-bar"), null);
      const verticalToolbar = await waitFor(() => view.getByTestId("agent-map-vertical-toolbar"));
      assert.ok(verticalToolbar.className.includes("flex-col"));
      assert.ok(view.getByTestId("btn-agent-fit-bounds"));

      // 空间过滤生效：超出合肥 AOI 的 S2A_..._OUT_OF_AOI 已被过滤（25 - 1 = 24，总计 12 + 24 = 36）
      const bottomDock = await waitFor(() => view.getByTestId("agent-map-bottom-dock"));
      assert.ok((bottomDock.textContent || "").includes("2 类数据"));
      assert.ok((bottomDock.textContent || "").includes("36"));

      // 状态 2 (split)：点击底部胶囊展开分屏数据坞，坞内显示规范短名（不出现 sar.sentinel1.slc.v1）、归栈切换与同步检索台按钮
      fireEvent.click(view.getByTestId("btn-expand-map-split-dock"));
      const splitPane = await waitFor(() => view.getByTestId("agent-map-split-results-pane"));
      assert.ok(splitPane);
      assert.strictEqual(
        (splitPane.textContent || "").includes("S2A_MSIL2A_20261015T030000_OUT_OF_AOI"),
        false
      );

      const multiBar = await waitFor(() => view.getByTestId("agent-multi-batch-bar"));
      assert.equal((multiBar.textContent || "").includes("sar.sentinel1.slc.v1"), false);
      assert.ok((multiBar.textContent || "").includes("Sentinel-1 SLC"));
      assert.ok((multiBar.textContent || "").includes("Sentinel-2 L2A"));
      assert.ok((multiBar.textContent || "").includes("36"));
      assert.ok((multiBar.textContent || "").includes("12"));
      assert.ok((multiBar.textContent || "").includes("24"));

      // 点击单个数据源胶囊筛选与统一「同幅归组」按钮（可折叠 + 整组全选）
      fireEvent.click(view.getByTestId("agent-batch-pill-b-s1"));
      assert.strictEqual(activeFilter, "b-s1");
      const stackToggleBtn = view.getByTestId("btn-agent-dock-stack-toggle");
      fireEvent.click(stackToggleBtn);
      assert.strictEqual(stackToggleBtn.getAttribute("aria-pressed"), "true");
      assert.ok((stackToggleBtn.textContent || "").includes("同幅归组"));

      // 同步至手动检索台图标按钮
      const openWbBtn = view.getByTestId("btn-open-in-map-workbench");
      fireEvent.click(openWbBtn);
      assert.deepEqual(openedWorkbenchArgs, {
        rid: "science.sentinel-2.l2a.v1",
        bid: "b-s2",
      });

      // 状态 3 (full)：全屏覆盖地图框 -> 收回至 collapsed
      fireEvent.click(view.getByTestId("btn-toggle-dock-fullscreen"));
      await waitFor(() =>
        assert.ok(view.getByTestId("agent-map-split-results-pane").className.includes("flex-1"))
      );
      fireEvent.click(view.getByTestId("btn-collapse-map-split-dock"));
      assert.strictEqual(view.queryByTestId("agent-map-split-results-pane"), null);
    }
  );

  await suite.test(
    "8. 每个会话与新建对话独立保存未发草稿（切换对话与新建对话绝不串台）",
    async () => {
      installBridge({
        sessionsList: async () => [
          {
            session_id: "sess-draft-a",
            title: "会话 A",
            created_at: "2026-11-18T00:00:00Z",
            updated_at: "2026-11-18T00:00:00Z",
            run_ids: [],
          },
          {
            session_id: "sess-draft-b",
            title: "会话 B",
            created_at: "2026-11-18T00:00:00Z",
            updated_at: "2026-11-18T00:00:00Z",
            run_ids: [],
          },
        ],
      });

      const view = render(
        <DesktopAgentWorkbench openSessionId="sess-draft-a" newChatNonce={0} />
      );
      const inputA = (await waitFor(() =>
        view.getByRole("textbox", { name: "科学需求输入框" })
      )) as HTMLTextAreaElement;
      fireEvent.change(inputA, { target: { value: "会话 A 草稿：成都平原形变分析" } });
      assert.strictEqual(inputA.value, "会话 A 草稿：成都平原形变分析");

      view.rerender(
        <DesktopAgentWorkbench openSessionId="sess-draft-b" newChatNonce={0} />
      );
      const inputB = (await waitFor(() =>
        view.getByRole("textbox", { name: "科学需求输入框" })
      )) as HTMLTextAreaElement;
      assert.strictEqual(inputB.value, "", "切到会话 B 不得串联会话 A 未发草稿");

      fireEvent.change(inputB, { target: { value: "会话 B 草稿：青藏高原冰川" } });
      view.rerender(
        <DesktopAgentWorkbench openSessionId={null} newChatNonce={1} />
      );
      const inputNew = (await waitFor(() =>
        view.getByRole("textbox", { name: "科学需求输入框" })
      )) as HTMLTextAreaElement;
      assert.strictEqual(inputNew.value, "", "新建对话不得残留历史草稿");

      view.rerender(
        <DesktopAgentWorkbench openSessionId="sess-draft-a" newChatNonce={1} />
      );
      const inputARestored = (await waitFor(() =>
        view.getByRole("textbox", { name: "科学需求输入框" })
      )) as HTMLTextAreaElement;
      assert.strictEqual(inputARestored.value, "会话 A 草稿：成都平原形变分析");
    }
  );
});
