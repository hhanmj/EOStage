import "../../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React, { useState } from "react";
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
import { createWorkbenchTestDesktopShellBridge } from "../../../test-utils/workbenchDesktopBridge";
import {
  clearAgentSessionWorkspaceCache,
  filterBatchCollectionByAoi,
  getAgentSessionWorkspaceState,
  peekAgentSessionWorkspaceState,
  projectAgentSessionBatchCollection,
  setAgentSessionWorkspaceState,
  upsertAgentSessionBatch,
  type AgentSessionWorkspaceState,
} from "@/shell/api/agentSessionWorkspace";
import type { DesktopAgentBridge } from "@/shell/api/agentBridgeTypes";
import type { AgentChannelView } from "@/lib/generated/v3-contracts";
import type { Bbox, SceneRow } from "@/lib/bridge";

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

const HEFEI_BBOX: Bbox = {
  west: 117.0,
  south: 31.5,
  east: 117.6,
  north: 32.1,
  crs: "EPSG:4326",
};

const BEIJING_BBOX: Bbox = {
  west: 116.1,
  south: 39.7,
  east: 116.7,
  north: 40.2,
  crs: "EPSG:4326",
};

function makeScene(sceneId: string, bbox: Bbox): SceneRow {
  return {
    scene_id: sceneId,
    display_name: sceneId,
    product_key: sceneId,
    acquisition_time: "2026-10-15T10:00:00Z",
    satellite: "Sentinel-1A",
    platform: "S1A",
    sensor: "SAR",
    product_type: "SLC",
    orbit_direction: "ASCENDING",
    relative_orbit: 42,
    polarization: "VV VH",
    beam_mode: "IW",
    footprint_bbox: bbox,
    footprint_geojson: null,
    file_size_bytes: 1_200_000_000,
    cloud_cover: null,
    native_metadata: {},
  } as unknown as SceneRow;
}

const HEFEI_SCENE = makeScene("S1_HEFEI_20261015", HEFEI_BBOX);
const BEIJING_SCENE = makeScene("S1_BEIJING_20261020", BEIJING_BBOX);

const DEFAULT_CHANNEL: AgentChannelView = {
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

const RESPONSES_CHANNEL: AgentChannelView = {
  id: "responses",
  protocol: "responses",
  base_url: "https://example.invalid/v1",
  model: "gpt-5.4",
  reasoning_mode: "default",
  models: [
    {
      id: "egg-beta",
      name: "egg-beta",
      context_window: 1_000_000,
      input_types: ["text"],
      reasoning_levels: [],
    },
  ],
  key_ready: true,
  is_default: false,
};

function installSessionIsolationBridge() {
  const base = createWorkbenchTestDesktopShellBridge();
  const sessions = new Map<string, { session_id: string; title: string; created_at: string; updated_at: string; run_ids: string[] }>();
  const eventsBySession = new Map<string, any[]>();
  const sendCalls: Array<{ sessionId: string; text: string; context: any }> = [];

  const agent = {
    channelsGet: async () => [DEFAULT_CHANNEL, RESPONSES_CHANNEL],
    channelsModels: async () => ["deepseek-reasoner"],
    channelsProbeDraft: async () => ({
      result: { ok: true, code: null, status: null, model_count: 1 },
      models: ["deepseek-reasoner"],
    }),
    channelsSave: async () => [DEFAULT_CHANNEL],
    channelsTest: async () => ({ ok: true, code: null, status: null, model_count: 1 }),
    channelsTestDraft: async () => ({ ok: true, code: null, status: null, model_count: 1 }),
    channelsSetDefault: async () => [DEFAULT_CHANNEL],
    channelsDelete: async () => [DEFAULT_CHANNEL],
    sessionsList: async () => Array.from(sessions.values()),
    sessionsCreate: async (title?: string | null) => {
      const id = "sess-1";
      const created = {
        session_id: id,
        title: title || "合肥检索会话",
        created_at: "2026-11-14T05:00:00Z",
        updated_at: "2026-11-14T05:00:00Z",
        run_ids: ["run-1"],
      };
      sessions.set(id, created);
      eventsBySession.set(id, []);
      return created;
    },
    sessionsDelete: async () => {},
    sessionsRename: async (sessionId: string, title: string) => ({
      session_id: sessionId,
      title,
      created_at: "2026-11-14T05:00:00Z",
      updated_at: "2026-11-14T05:00:00Z",
      run_ids: [],
    }),
    sessionsSetPinned: async (sessionId: string) => sessions.get(sessionId)!,
    sessionsSetArchived: async (sessionId: string) => sessions.get(sessionId)!,
    sessionsMove: async (sessionId: string) => sessions.get(sessionId)!,
    sessionsExport: async () => ({ format: "markdown", content: "", filename: "export.md" }),
    sessionsExportTo: async () => "/tmp/export.md",
    workspacesList: async () => [],
    workspacesCreate: async (name: string) => ({ workspace_id: "ws-1", name, folder: null }),
    workspacesUpdate: async (workspaceId: string) => ({ workspace_id: workspaceId, name: "ws", folder: null }),
    workspacesDelete: async () => {},
    sessionTruncate: async (_sessionId: string, beforeSeq: number) => ({
      session: sessions.get("sess-1")!,
      cursor: Math.max(0, beforeSeq - 1),
      aoi_restored: false,
    }),
    send: async (sessionId: string, text: string, context?: any) => {
      sendCalls.push({ sessionId, text, context: context ?? null });
      const list = eventsBySession.get(sessionId) ?? [];
      list.push({
        seq: list.length + 1,
        ts: "2026-11-14T05:00:01Z",
        kind: "message",
        payload: {
          run_id: "run-1",
          role: "user",
          text,
          context: null,
        },
      });
      eventsBySession.set(sessionId, list);
      return "run-1";
    },
    runCancel: async () => true,
    stopAll: async () => ({
      requested_at: "2026-11-14T05:00:00Z",
      cancelled_runs: [],
      cancelled_tasks: [],
      errors: [],
    }),
    streamPoll: async (sessionId: string, cursor: number) => {
      const list = eventsBySession.get(sessionId) ?? [];
      const fresh = list.filter((e) => e.seq > cursor);
      return { events: fresh, cursor: list.length };
    },
    planApprove: async () => {},
    planReject: async () => {},
    runsList: async () => [],
    runsOpen: async () => ({
      run_id: "run-1",
      session_id: "sess-1",
      status: "completed",
      started_at: "2026-11-14T05:00:00Z",
      ended_at: "2026-11-14T05:00:02Z",
      steps: [],
    }),
    sessionWork: async () => [],
    permissionModeGet: async () => ({
      effective: "auto_approve",
      session_override: null,
      global_default: "auto_approve",
    }),
    permissionModeSetGlobal: async (mode: any) => ({
      effective: mode,
      session_override: null,
      global_default: mode,
    }),
    permissionModeSetSession: async (_sid: string, mode: any) => ({
      effective: mode ?? "auto_approve",
      session_override: mode,
      global_default: "auto_approve",
    }),
    activityStatus: async () => ({
      busy: false,
      active_runs: 0,
      queued_tasks: 0,
      running_tasks: 0,
      pausing_tasks: 0,
    }),
  } as unknown as DesktopAgentBridge;

  registerTestDesktopShellBridge({
    ...base,
    agent,
  });
  return { sendCalls };
}

function IsolationHarness() {
  const [manualAoi, setManualAoi] = useState<Bbox | null>(HEFEI_BBOX);
  const [manualAoiLabel, setManualAoiLabel] = useState<string>("安徽省合肥市");
  const [manualScenes, setManualScenes] = useState<SceneRow[]>([HEFEI_SCENE]);
  const [manualBatchId, setManualBatchId] = useState<string | null>("batch-hefei");

  const [openSessionId, setOpenSessionId] = useState<string | null>(null);
  const [newChatNonce, setNewChatNonce] = useState(0);
  const [activeSessionId, setActiveSessionId] = useState<string | null>(null);
  const [sessionWorkspace, setSessionWorkspace] = useState<AgentSessionWorkspaceState | null>(null);

  const isIsolated = activeSessionId !== null;
  const effectiveAoi = isIsolated ? (sessionWorkspace?.workingAoi ?? null) : manualAoi;
  const effectiveLabel = isIsolated ? (sessionWorkspace?.aoiLabel ?? "") : manualAoiLabel;
  const effectiveScenes = isIsolated ? (sessionWorkspace?.scenes ?? []) : manualScenes;
  const effectiveBatchId = isIsolated ? (sessionWorkspace?.batchId ?? null) : manualBatchId;

  return (
    <div>
      <button
        type="button"
        data-testid="harness-change-manual-beijing"
        onClick={() => {
          setManualAoi(BEIJING_BBOX);
          setManualAoiLabel("北京市");
          setManualScenes([BEIJING_SCENE]);
          setManualBatchId("batch-beijing");
        }}
      >
        Switch Manual Workbench to Beijing
      </button>
      <button
        type="button"
        data-testid="harness-new-chat"
        onClick={() => {
          setOpenSessionId(null);
          setActiveSessionId(null);
          setSessionWorkspace(null);
          setNewChatNonce((n) => n + 1);
        }}
      >
        + 新对话
      </button>
      <button
        type="button"
        data-testid="harness-open-sess-1"
        onClick={() => {
          setOpenSessionId("sess-1");
          setActiveSessionId("sess-1");
          setSessionWorkspace(peekAgentSessionWorkspaceState("sess-1"));
        }}
      >
        打开 Session 1
      </button>

      <DesktopAgentWorkbench
        workbenchAoi={effectiveAoi}
        aoiLabel={effectiveLabel}
        scenes={effectiveScenes}
        batchId={effectiveBatchId}
        sourceId="science.sentinel-1.standard.v1"
        sourceNames={{ "science.sentinel-1.standard.v1": "Sentinel-1 SAR" }}
        openSessionId={openSessionId}
        newChatNonce={newChatNonce}
        onActiveSessionChange={(sid) => {
          setActiveSessionId(sid);
          setSessionWorkspace(sid ? peekAgentSessionWorkspaceState(sid) : null);
        }}
        onSessionCreatedSnapshot={async (newSid) => {
          const saved = await setAgentSessionWorkspaceState(newSid, {
            workingAoi: manualAoi,
            workingGeometry: null,
            aoiLabel: manualAoiLabel,
            batchId: manualBatchId,
            resourceId: "science.sentinel-1.standard.v1",
            scenes: [...manualScenes],
            batchTotalCount: manualScenes.length,
          });
          setActiveSessionId(newSid);
          setOpenSessionId(newSid);
          setSessionWorkspace(saved);
        }}
      />
    </div>
  );
}

test("Requirement #2: 单向工作台继承与按会话严格隔离", async (suite) => {
  suite.afterEach(() => {
    cleanup();
    clearTestDesktopShellBridge();
    clearAgentSessionWorkspaceCache();
    window.localStorage.removeItem("eostage.agent.modelSelection");
  });

  await suite.test("agentSessionWorkspace 按 sessionId 独立缓存且互不串改", async () => {
    clearAgentSessionWorkspaceCache();
    await setAgentSessionWorkspaceState("sess-1", {
      workingAoi: HEFEI_BBOX,
      workingGeometry: null,
      aoiLabel: "安徽省合肥市",
      batchId: "batch-hefei",
      resourceId: "science.sentinel-1.standard.v1",
      scenes: [HEFEI_SCENE],
    });
    await setAgentSessionWorkspaceState("sess-2", {
      workingAoi: BEIJING_BBOX,
      workingGeometry: null,
      aoiLabel: "北京市",
      batchId: "batch-beijing",
      resourceId: "science.landsat.c2-l2.v1",
      scenes: [BEIJING_SCENE],
    });

    const s1 = await getAgentSessionWorkspaceState("sess-1");
    const s2 = await getAgentSessionWorkspaceState("sess-2");
    assert.equal(s1?.aoiLabel, "安徽省合肥市");
    assert.equal(s1?.batchId, "batch-hefei");
    assert.equal(s1?.scenes?.[0]?.scene_id, "S1_HEFEI_20261015");
    assert.equal(s2?.aoiLabel, "北京市");
    assert.equal(s2?.batchId, "batch-beijing");
    assert.equal(s2?.scenes?.[0]?.scene_id, "S1_BEIJING_20261020");
  });

  await suite.test(
    "Session 1 发送冻结快照 -> 地图工作台改北京并新搜 -> + 新对话继承北京 -> 切回 Session 1 仍保留合肥",
    async () => {
      clearAgentSessionWorkspaceCache();
      installSessionIsolationBridge();
      const view = render(<IsolationHarness />);

      // 1. 初始未发送草稿实时跟随地图工作台（合肥）
      await waitFor(() =>
        assert.ok(view.container.textContent?.includes("已绑定研究区：安徽省合肥市"))
      );
      await waitFor(() =>
        assert.equal(view.getByRole("button", { name: "模型" }).textContent, "DeepSeek R1")
      );

      // 2. 点击发送创建 Session 1，冻结当时合肥快照
      const input = view.getByRole("textbox", { name: "科学需求输入框" });
      fireEvent.change(input, { target: { value: "帮我下载合肥这景" } });
      await waitFor(() =>
        assert.equal((view.getByRole("button", { name: "发送" }) as HTMLButtonElement).disabled, false)
      );
      fireEvent.click(view.getByRole("button", { name: "发送" }));

      await waitFor(() => {
        const snap = peekAgentSessionWorkspaceState("sess-1");
        assert.equal(snap?.aoiLabel, "安徽省合肥市");
        assert.equal(snap?.scenes?.[0]?.scene_id, "S1_HEFEI_20261015");
      });
      await waitFor(() => {
        const bar = view.getByTestId("agent-live-context-bar");
        assert.ok(bar.textContent?.includes("研究区：安徽省合肥市"));
        assert.ok(view.getByTestId("agent-context-batch-pill").textContent?.includes("1 景"));
      });

      // 3. 在地图工作台将 AOI 改为北京市并更换结果：Session 1 不受任何影响
      fireEvent.click(view.getByTestId("harness-change-manual-beijing"));
      await waitFor(() => {
        const bar = view.getByTestId("agent-live-context-bar");
        assert.ok(bar.textContent?.includes("研究区：安徽省合肥市"));
        assert.equal(bar.textContent?.includes("北京市"), false);
      });

      // 4. 点击 + 新对话：立即继承地图工作台最新的北京市 AOI 与新结果
      fireEvent.click(view.getByTestId("harness-new-chat"));
      await waitFor(() => {
        assert.ok(view.container.textContent?.includes("已绑定研究区：北京市"));
      });
      assert.equal(peekAgentSessionWorkspaceState("sess-1")?.scenes?.[0]?.scene_id, "S1_HEFEI_20261015");

      // 5. 切回 Session 1：依然完整保留 Session 1 冻结的合肥市 AOI 与 S1_HEFEI 结果
      fireEvent.click(view.getByTestId("harness-open-sess-1"));
      await waitFor(() => {
        const bar = view.getByTestId("agent-live-context-bar");
        assert.ok(bar.textContent?.includes("研究区：安徽省合肥市"));
      });
      // 展开地图结果 Tab 验证景清单也保持合肥结果
      fireEvent.click(view.getByTestId("agent-context-batch-pill"));
      fireEvent.click(view.getByRole("tab", { name: /清单/ }));
      await waitFor(() => {
        assert.ok(view.container.textContent?.includes("S1_HEFEI_20261015"));
        assert.equal(view.container.textContent?.includes("S1_BEIJING_20261020"), false);
      });
    }
  );

  await suite.test(
    "remount / + 新对话时持久化 modelSelection (eostage.agent.modelSelection) 同步到 sendContext.model_selection",
    async () => {
      clearAgentSessionWorkspaceCache();
      window.localStorage.setItem(
        "eostage.agent.modelSelection",
        JSON.stringify({ channel_id: "responses", model_id: "egg-beta", reasoning_level: null })
      );
      const { sendCalls } = installSessionIsolationBridge();
      const view = render(<IsolationHarness />);

      await waitFor(() =>
        assert.equal(view.getByRole("button", { name: "模型" }).textContent, "egg-beta")
      );

      const input = view.getByRole("textbox", { name: "科学需求输入框" });
      fireEvent.change(input, { target: { value: "用 egg-beta 搜索吉隆 NISAR" } });
      await waitFor(() =>
        assert.equal((view.getByRole("button", { name: "发送" }) as HTMLButtonElement).disabled, false)
      );
      fireEvent.click(view.getByRole("button", { name: "发送" }));

      await waitFor(() => assert.equal(sendCalls.length, 1));
      assert.deepEqual(sendCalls[0].context?.model, {
        channel_id: "responses",
        model_id: "egg-beta",
      });
    }
  );

  await suite.test(
    "Issue #50: 继承工作台 Sentinel-1 批次后 Agent 检索 Sentinel-2，顶栏映射数量与分屏底栏一致显示新批次数",
    async () => {
      clearAgentSessionWorkspaceCache();
      const s1Scenes = Array.from({ length: 25 }, (_, i) =>
        makeScene(`S1_HEFEI_${String(i + 1).padStart(2, "0")}`, HEFEI_BBOX)
      );
      const s2Scenes = Array.from({ length: 11 }, (_, i) =>
        makeScene(`S2_HEFEI_${String(i + 1).padStart(2, "0")}`, HEFEI_BBOX)
      );

      // Step 1: New session inherits workbench S1 batch with activeBatchFilter: "all" (fixed)
      await setAgentSessionWorkspaceState("sess-50", {
        workingAoi: HEFEI_BBOX,
        workingGeometry: null,
        aoiLabel: "合肥市",
        batchId: "batch-s1-250",
        resourceId: "science.sentinel-1.standard.v1",
        scenes: s1Scenes,
        rowStates: new Map(),
        batchTotalCount: 250,
        selectedSceneId: null,
        batches: [
          {
            batchId: "batch-s1-250",
            resourceId: "science.sentinel-1.standard.v1",
            revision: 1,
            scenes: s1Scenes,
            rowStates: new Map(),
            totalCount: 250,
            updatedAtMs: 1000,
          },
        ],
        activeBatchFilter: "all",
      });

      // Step 2: Agent completes a Sentinel-2 search with selectThisBatch: true
      const updated = await upsertAgentSessionBatch("sess-50", {
        batchId: "batch-s2-11",
        resourceId: "science.sentinel-2.l2a.v1",
        revision: 1,
        scenes: s2Scenes,
        rowStates: new Map(),
        totalCount: 11,
        updatedAtMs: 2000,
        selectThisBatch: true,
      });

      assert.equal(updated.activeBatchFilter, "batch-s2-11");
      assert.equal(updated.scenes.length, 11);
      assert.equal(updated.batchTotalCount, 11);

      const projected = filterBatchCollectionByAoi(
        projectAgentSessionBatchCollection(updated.batches ?? [], updated.activeBatchFilter),
        updated.workingAoi
      );
      assert.equal(projected.scenes.length, 11);
      assert.equal(projected.activeBatchId, "batch-s2-11");

      // Step 3: Even if Agent searches in a new AOI (Beijing) without selecting batch explicitly
      const beijingS2 = Array.from({ length: 7 }, (_, i) =>
        makeScene(`S2_BEIJING_${String(i + 1).padStart(2, "0")}`, BEIJING_BBOX)
      );
      await setAgentSessionWorkspaceState("sess-50", {
        workingAoi: BEIJING_BBOX,
        aoiLabel: "北京市",
      });
      const updatedNewAoi = await upsertAgentSessionBatch("sess-50", {
        batchId: "batch-s2-beijing-7",
        resourceId: "science.sentinel-2.l2a.v1",
        revision: 2,
        scenes: beijingS2,
        rowStates: new Map(),
        totalCount: 7,
        updatedAtMs: 3000,
        selectThisBatch: true,
      });
      const projectedNewAoi = filterBatchCollectionByAoi(
        projectAgentSessionBatchCollection(
          updatedNewAoi.batches ?? [],
          updatedNewAoi.activeBatchFilter
        ),
        updatedNewAoi.workingAoi
      );
      assert.equal(projectedNewAoi.scenes.length, 7);
    }
  );
});
