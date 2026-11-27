import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { cleanup, fireEvent, render } from "../../test-utils/render-harness";
import { waitFor } from "@testing-library/react";
import { I18nProvider } from "../../i18n";
import {
  clearTestDesktopShellBridge,
  registerTestDesktopShellBridge,
} from "@/lib/desktopShellBridge";
import { AgentUnifiedResultsDeck } from "./agent/AgentMultiBatchResultsView";
import { AgentThinkingCard } from "./agent/AgentThinkingCard";
import { AgentComposer } from "./AgentComposer";
import { NotificationPopover } from "./NotificationPopover";
import { ProgressNotificationHost } from "../ui/ProgressNotification";
import { DownloadFilterBar } from "./DownloadFilterBar";
import { DownloadTaskRow } from "./DownloadTaskRow";
import { TaskArtifactsVerificationPanel } from "./TaskArtifactsVerificationPanel";
import { AgentErrorCard } from "./AgentErrorCard";
import { ChannelListItem } from "../../shell/components/settings/ChannelListItem";
import { ModelChannelsSettingsTab } from "../../shell/components/settings/ModelChannelsSettingsTab";
import { ChannelModelsEditor } from "../../shell/components/settings/ChannelModelsEditor";
import {
  clearChannelHealth,
  setChannelHealth,
} from "../../shell/api/channelHealth";
import { projectAgentSessionBatchCollection } from "../../shell/api/agentSessionWorkspace";
import type { AgentChannelView } from "../../lib/generated/v3-contracts";
import type { ApplicationTask, SceneRow } from "../../shell/api/types";
import type { StreamErrorItem } from "../../shell/api/agentStreamLogic";

if (typeof globalThis.localStorage === "undefined" && typeof window !== "undefined") {
  (globalThis as any).localStorage = window.localStorage;
}

test.afterEach(() => {
  cleanup();
  clearTestDesktopShellBridge();
  if (typeof localStorage !== "undefined") {
    localStorage.clear();
  }
});

function makeScene(overrides: Partial<SceneRow> = {}): SceneRow {
  return {
    scene_id: "S1A_IW_SLC__1SDV_20261101T104925_001",
    acquisition_time: "2026-11-01T10:49:25Z",
    platform: "Sentinel-1A",
    cloud_cover: 0,
    orbit_direction: "Ascending",
    track: 84,
    frame: 70,
    size_bytes: 4_500_000_000,
    bbox: [103.8, 30.8, 104.2, 31.1],
    preview_url: null,
    selected: false,
    ...overrides,
  };
}

function makeTask(overrides: Partial<ApplicationTask> = {}): ApplicationTask {
  return {
    task_id: "task-main-1",
    revision: 1,
    category: "scene_download",
    name: "Sentinel-1 合肥监测",
    status: "running",
    done_bytes: 128 * 1024 * 1024,
    total_bytes: 1024 * 1024 * 1024,
    provider: "asf",
    dataset: "Sentinel-1",
    output_dir: "/tmp/eostage/s1",
    items_count: 2,
    completed_items_count: 1,
    skipped_count: 0,
    cancelled_count: 0,
    ...overrides,
  };
}

function makeChannel(overrides: Partial<AgentChannelView> = {}): AgentChannelView {
  return {
    id: "openai-main",
    protocol: "responses",
    base_url: "https://api.openai.com/v1",
    model: "gpt-5.4",
    reasoning_mode: "off",
    reasoning_switch_unsupported: false,
    models: [{ id: "gpt-5.4", protocol: "responses" }],
    key_ready: true,
    is_default: true,
    ...overrides,
  };
}

test("2026-11-19 8项 /grill-me 全面回归套件", async (suite) => {
  await suite.test(
    "条目 1-3: Agent 结果清单支持可折叠「同幅归组」与对齐排序，且标准化 ASF 与 NISAR 官方/测试层级",
    () => {
      const s1a = makeScene({ scene_id: "S1A_P84_F70_1", track: 84, frame: 70 });
      const s1b = makeScene({ scene_id: "S1B_P84_F70_2", track: 84, frame: 70 });
      const nisarV1 = makeScene({
        scene_id: "NISAR_L2_PR_RSLC_001_V1",
        platform: "NISAR L-band",
        provider_id: "asf.nasa.earthdata",
        properties: { release_tier: "validated" },
      });
      const nisarProv = makeScene({
        scene_id: "NISAR_L2_PR_RSLC_002_PROV",
        platform: "NISAR L-band",
        provider_id: "asf.nasa.earthdata",
        properties: { release_tier: "provisional" },
      });

      const collection = projectAgentSessionBatchCollection(
        [
          {
            batchId: "b-s1",
            resourceId: "sar.sentinel1.slc.v1",
            revision: 1,
            totalCount: 2,
            scenes: [s1a, s1b],
            rowStates: new Map(),
            updatedAtMs: 1,
          },
          {
            batchId: "b-nisar",
            resourceId: "asf.nasa.earthdata",
            revision: 1,
            totalCount: 2,
            scenes: [nisarV1, nisarProv],
            rowStates: new Map(),
            updatedAtMs: 2,
          },
        ],
        "all"
      );

      assert.ok(
        collection.batches.every((b) => !(b.resourceId ?? "").toLowerCase().includes("asf.nasa.earthdata")),
        "不得裸露 asf.nasa.earthdata 内部标识符"
      );

      const view = render(
        <I18nProvider>
          <AgentUnifiedResultsDeck
            scenes={[s1a, s1b, nisarV1, nisarProv]}
            totalCount={4}
            batches={collection.batches}
            activeFilter="all"
            combinedCount={4}
            onSelectFilter={() => {}}
            checkedSceneIds={new Set()}
            creatingItemIds={new Set()}
          />
        </I18nProvider>
      );

      // 混合数据源模式下显示「数据源」排序，且「同幅归组」开关与折叠头真实生效
      assert.ok(view.getByText("数据源"));
      const stackBtn = view.getByTestId("btn-agent-dock-stack-toggle");
      fireEvent.click(stackBtn);
      const groupCards = view.getAllByTestId("agent-dock-stack-group");
      assert.ok(groupCards.length >= 1);
      const groupToggle = groupCards[0].querySelector("button[aria-expanded]") as HTMLButtonElement;
      assert.ok(groupToggle);
      assert.equal(groupToggle.getAttribute("aria-expanded"), "true");
      fireEvent.click(groupToggle);
      assert.equal(groupToggle.getAttribute("aria-expanded"), "false");
    }
  );

  await suite.test(
    "条目 4 / Item 12: 移除双停止按钮，思考卡无停止键，输入框发送按钮运行中就地变形为停止键并触发停止",
    () => {
      let composerStopped = 0;
      const view = render(
        <I18nProvider>
          <>
            <AgentThinkingCard
              status="thinking"
              thinkingText="正在核对研究区检索策略"
              startedAtMs={Date.now() - 2000}
              onStop={() => {}}
            />
            <AgentComposer
              value=""
              onInputChange={() => {}}
              onSubmit={() => {}}
              busy
              isRunning
              onStop={() => {
                composerStopped += 1;
              }}
              channelReadiness={{ ready: true, disabled: false, reason_zh: "就绪" }}
              channels={[makeChannel()]}
              sessionId="sess-1"
              permissionView={null}
              permissionStatus="ready"
            />
          </>
        </I18nProvider>
      );

      assert.equal(view.queryByTestId("agent-thinking-stop-btn"), null);
      assert.equal(view.queryByTestId("agent-composer-stop-btn"), null);
      const morphBtn = view.getByTestId("agent-send-btn") as HTMLButtonElement;
      assert.equal(morphBtn.getAttribute("data-action-mode"), "stop");
      assert.equal(morphBtn.disabled, false);
      fireEvent.click(morphBtn);
      assert.equal(composerStopped, 1);
    }
  );

  await suite.test(
    "条目 5: 左上角消息通知按钮（固定/筛选/更多/更新日志/关闭）真实交互与右上角悬浮互斥",
    () => {
      let pinnedStates: boolean[] = [];
      let markedRead = 0;
      let clearedRead = 0;
      let closed = 0;

      const view = render(
        <I18nProvider>
          <>
            <NotificationPopover
              open
              variant="side-drawer"
              onClose={() => {
                closed += 1;
              }}
              onPinnedChange={(next) => pinnedStates.push(next)}
              onMarkAllRead={() => {
                markedRead += 1;
              }}
              onClearRead={() => {
                clearedRead += 1;
              }}
              notifications={[
                {
                  id: "n-1",
                  kind: "completed",
                  title: "Sentinel-1 下载完成",
                  detail: ["2 景影像已落盘"],
                  occurred_at: Date.now() - 60_000,
                  route: "downloads",
                },
                {
                  id: "n-2",
                  kind: "completed",
                  title: "Landsat 历史任务",
                  detail: ["已完成归档"],
                  occurred_at: Date.now() - 120_000,
                  route: "downloads",
                },
              ]}
              unreadIds={["n-1"]}
              unreadCount={1}
            />
            <ProgressNotificationHost
              tasks={[
                makeTask({
                  task_id: "t-active",
                  name: "Landsat C2 L2",
                  status: "running",
                }),
              ]}
              pendingPreparations={[]}
              suppressAll
              onOpenDownloads={() => {}}
            />
          </>
        </I18nProvider>
      );

      // suppressAll=true 时右上角悬浮通知彻底隐藏
      assert.equal(view.queryByText("Landsat C2 L2"), null);

      fireEvent.click(view.getByTestId("notification-pin-btn"));
      assert.equal(pinnedStates[pinnedStates.length - 1], true);

      fireEvent.click(view.getByTestId("notification-more-btn"));
      fireEvent.click(view.getByTestId("notification-mark-all-read"));
      fireEvent.click(view.getByTestId("notification-more-btn"));
      fireEvent.click(view.getByTestId("notification-clear-read"));
      assert.equal(markedRead, 1);
      assert.equal(clearedRead, 1);

      fireEvent.click(view.getByTestId("notification-tab-changelog"));
      assert.ok(view.getByTestId("notification-changelog-list"));

      fireEvent.click(view.getByTestId("notification-close-btn"));
      assert.equal(closed, 1);
    }
  );

  await suite.test(
    "条目 6: 伴随精轨并组计数(1 而非 2)、折叠精轨胶囊、「成果与核验」与模型 3 层托底续跑",
    () => {
      const parent = makeTask({
        task_id: "parent-s1",
        status: "running",
        dataset: "Sentinel-1",
      });
      const companion = makeTask({
        task_id: "orbit-s1",
        status: "queued",
        auxiliary_kind: "AUX_POEORB",
        source_task_id: "parent-s1",
        dataset: "Sentinel-1",
        name: "伴随精密轨道",
      });
      const doneTask = makeTask({
        task_id: "done-l8",
        status: "completed",
        name: "Landsat 成都市",
        dataset: "Landsat C2 L2",
        output_dir: "/tmp/eostage/l8",
      });

      const barView = render(
        <I18nProvider>
          <DownloadFilterBar
            tasks={[parent, companion]}
            selectedType="all"
            onSelectType={() => {}}
            selectedStatus="all"
            onSelectStatus={() => {}}
          />
        </I18nProvider>
      );
      assert.match(
        barView.getByTestId("download-status-filter-all").textContent ?? "",
        /全部.*1/
      );
      assert.match(
        barView.getByTestId("download-status-filter-active").textContent ?? "",
        /进行中.*1/
      );
      cleanup();

      const rowView = render(
        <I18nProvider>
          <DownloadTaskRow
            task={parent}
            compact={false}
            companions={[companion]}
          />
        </I18nProvider>
      );
      const pill = rowView.getByTestId("task-companion-summary-pill");
      assert.ok(pill.textContent?.includes("含 1 项伴随精轨"));
      fireEvent.click(pill);
      assert.equal(rowView.queryByTestId("task-companion-summary-pill"), null);
      cleanup();

      let openedFolder = "";
      const artifactsView = render(
        <I18nProvider>
          <TaskArtifactsVerificationPanel
            tasks={[parent, companion, doneTask]}
            onOpenFolder={(task) => {
              openedFolder = task.task_id;
            }}
          />
        </I18nProvider>
      );
      assert.ok(artifactsView.getByTestId("artifacts-independent-transfer-banner"));
      assert.ok(artifactsView.getByTestId("artifact-status-verified"));
      fireEvent.click(artifactsView.getAllByTestId("artifact-open-folder-btn")[0]);
      assert.ok(openedFolder.length > 0);
      cleanup();

      let retriedRun = "";
      const errItem: StreamErrorItem = {
        kind: "error",
        seq: 12,
        run_id: "run-failed-9",
        code: "model_transient",
        status: "failed",
        plan_id: null,
        message_zh: "HTTP 429（触发限流或配额不足）：Rate limit reached",
      };
      // Owner 2026-11-21: 错误卡精简为报错理由与重试，不再渲染接着继续/换备用模型继续按钮
      const errView = render(
        <AgentErrorCard
          item={errItem}
          activeDownloadCount={1}
          onRetry={(rid) => {
            retriedRun = rid;
          }}
        />
      );

      assert.ok(errView.getByTestId("agent-error-active-downloads-banner"));
      assert.ok(!/[a-z]+_[a-z_]+/.test(errView.container.textContent || ""));
      assert.equal(errView.queryByTestId("agent-error-resume-btn"), null);
      assert.equal(errView.queryByTestId("agent-error-switch-model-btn"), null);
      fireEvent.click(errView.getByTestId("btn-error-retry"));
      assert.equal(retriedRun, "run-failed-9");
    }
  );

  await suite.test(
    "条目 7-8: 模型设置不弹出公开库下拉、移除假 Copilot Key 卡、官方渠道与自定义前置、4 态健康绿/红/灰/橙圆点",
    async () => {
      registerTestDesktopShellBridge({
        kind: "tauri",
        agent: {
          channelsGet: async () => [makeChannel()],
          channelsSave: async () => [makeChannel()],
          channelsSetDefault: async () => [makeChannel()],
          channelsDelete: async () => [],
          channelsModels: async () => ["gpt-5.4"],
          channelsProbeDraft: async () => ({
            result: { ok: true, code: null, status: 200, model_count: 1, latency_ms: 40, message: "ok" },
            models: ["gpt-5.4"],
          }),
          channelsTestDraft: async () => ({
            ok: true,
            code: null,
            status: 200,
            model_count: null,
            latency_ms: 50,
            message: "pong",
          }),
          permissionModeGet: async () => ({
            global_mode: "within_quota",
            session_mode: null,
            effective_mode: "within_quota",
          }),
          permissionModeSetGlobal: async () => ({
            global_mode: "within_quota",
            session_mode: null,
            effective_mode: "within_quota",
          }),
          permissionModeSetSession: async () => ({
            global_mode: "within_quota",
            session_mode: null,
            effective_mode: "within_quota",
          }),
          permissionModeClearSession: async () => ({
            global_mode: "within_quota",
            session_mode: null,
            effective_mode: "within_quota",
          }),
          sessionsList: async () => [],
          sessionsCreate: async () => ({
            session_id: "s1",
            title: "t",
            created_at: "",
            updated_at: "",
            run_ids: [],
          }),
          sessionsDelete: async () => {},
          sessionsDeleteMany: async () => ({ deleted_ids: [] }),
          runsList: async () => [],
          runOpen: async () => ({
            run: {
              run_id: "r1",
              session_id: "s1",
              started_at: "",
              ended_at: null,
              status: "completed",
              model_route: "responses/gpt-5.4",
            },
            events: [],
          }),
          streamPoll: async () => ({ cursor: 0, events: [] }),
          send: async () => "r1",
          runCancel: async () => {},
          planConfirm: async () => {},
          planReject: async () => {},
          sessionWork: async () => [],
          channelsRename: async () => [],
        },
        application: {},
      });

      const modelsEditorView = render(
        <I18nProvider>
          <ChannelModelsEditor
            channelId="openai-main"
            models={[{ id: "gpt-5.4", protocol: "responses" }]}
            protocol="responses"
            onFetch={() => {}}
            onChange={() => {}}
          />
        </I18nProvider>
      );
      assert.equal(modelsEditorView.queryByTestId("public-model-suggestions"), null);
      cleanup();

      const settingsView = render(
        <I18nProvider>
          <ModelChannelsSettingsTab />
        </I18nProvider>
      );
      await waitFor(() => settingsView.getByTestId("toggle-oauth-pkce-helper"));
      fireEvent.click(settingsView.getByTestId("toggle-oauth-pkce-helper"));
      const customCard = await waitFor(() =>
        settingsView.getByTestId("create-custom-provider-card")
      );
      const pkcePanel = settingsView.getByTestId("oauth-pkce-helper-panel");
      assert.equal(settingsView.queryByTestId("preset-provider-card-github-copilot"), null);
      assert.ok(
        (customCard.compareDocumentPosition(pkcePanel) & Node.DOCUMENT_POSITION_FOLLOWING) !== 0,
        "「官方渠道与自定义」必须排在「免 Key 浏览器授权」之前"
      );
      cleanup();

      // 3 态健康圆点规则：绿 (ok=已启用且凭据就绪)、红 (error)、灰 (disabled/missing_key/untested)
      setChannelHealth("ch-ok", "ok");
      setChannelHealth("ch-err", "error", "HTTP 401");
      clearChannelHealth("ch-untested");

      const dotsView = render(
        <I18nProvider>
          <>
            <ChannelListItem
              channel={makeChannel({ id: "ch-ok", key_ready: true })}
              isSelected={false}
              onSelect={() => {}}
            />
            <ChannelListItem
              channel={makeChannel({ id: "ch-err", key_ready: true })}
              isSelected={false}
              onSelect={() => {}}
            />
            <ChannelListItem
              channel={makeChannel({ id: "ch-untested", key_ready: true })}
              isSelected={false}
              onSelect={() => {}}
            />
            <ChannelListItem
              channel={makeChannel({ id: "ch-missing", key_ready: false })}
              isSelected={false}
              onSelect={() => {}}
            />
            <ChannelListItem
              channel={makeChannel({ id: "ch-disabled", key_ready: true })}
              enabled={false}
              isSelected={false}
              onSelect={() => {}}
            />
          </>
        </I18nProvider>
      );

      const states = dotsView
        .getAllByTestId("channel-health-dot")
        .map((el) => el.getAttribute("data-health-state"));
      assert.deepEqual(states, ["ok", "error", "ok", "missing_key", "disabled"]);
    }
  );
});
