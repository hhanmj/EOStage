import "../../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React, { act } from "react";
import { render, cleanup, fireEvent } from "../../../test-utils/render-harness";
import { AgentMessageActionBar } from "./AgentMessageActionBar";
import { AgentThinkingCard, UnifiedTurnProcessHeader } from "./AgentThinkingCard";
import { AgentTaskProgressCard } from "./AgentTaskProgressCard";
import {
  renderToolSemanticGlyph,
  resolveToolSemanticMeta,
  EO_MONOCHROME_ICON_CLASS,
} from "./agentToolSemantics";
import { AgentComposerEyebrow } from "./AgentComposerEyebrow";
import {
  AgentMapSceneDrawer,
  inferResourceIdFromSceneId,
  matchSceneIdFromList,
} from "./AgentMapSceneDrawer";
import { AgentConclusionCard } from "../AgentConclusionCard";
import { AgentToolGroupCard } from "../AgentToolGroupCard";
import { AgentComposer } from "../AgentComposer";
import { AgentContextHud } from "../AgentContextHud";
import { toggleSkillEnabled } from "../../../lib/agentSkills";
import { SettingsModal } from "../../SettingsModal";
import {
  getBusySendPrefs,
  setBusySendPrefs,
  setPerformanceOnlyPrefs,
} from "../../../lib/performanceAndUsagePrefs";
import {
  classifyStreamEvents,
  type StreamConclusionItem,
  type StreamPlanCardItem,
  type StreamToolGroupItem,
} from "../../../shell/api/agentStreamLogic";
import {
  clearAgentSessionWorkspaceCache,
  extractSessionBatchesFromEvents,
  upsertAgentSessionBatch,
} from "../../../shell/api/agentSessionWorkspace";
import { resolveAgentToolDisplayName } from "../../../shell/api/agentLogic";

test.afterEach(() => {
  cleanup();
});

test("Agent 8 项体验打磨与排队/立即引导交互测试套件", async (suite) => {
  await suite.test(
    "1 & 4. 纯图标轮次操作栏 + 回复顶部思考耗时（含无 <think> 回退）+ 底部同行 EOStage 悬停本轮用量浮层",
    async () => {
      // 用户轮：复制 / 编辑 / 回撤均为纯图标按钮（无可见文字标注，保留 aria-label / title）
      const userView = render(
        <AgentMessageActionBar
          role="user"
          content="检索四川省近一个月的 Sentinel-1 影像"
          turnIndex={0}
          onEditResend={() => {}}
          onRevertTurn={() => {}}
        />
      );
      const userCopyBtn = userView.getByTestId("btn-copy-user-msg");
      const userEditBtn = userView.getByTestId("btn-edit-resend-msg");
      const userRevertBtn = userView.getByTestId("btn-revert-turn");
      assert.strictEqual(userCopyBtn.textContent, "");
      assert.strictEqual(userEditBtn.textContent, "");
      assert.strictEqual(userRevertBtn.textContent, "");
      assert.ok(userCopyBtn.getAttribute("aria-label"));
      assert.ok(userEditBtn.getAttribute("aria-label"));
      assert.ok(userRevertBtn.getAttribute("aria-label"));
      cleanup();

      // 助手结论卡：当模型未输出 <think> 文本但存在本轮耗时时，顶部展示「已思考 X.X 秒」；
      // 底部复制/重试无文字标注，且同一行提供 BarChart2 用量图标，鼠标移入展开本轮用量明细；
      // 无真实引用时绝不渲染「未引用证据」。
      let detailRequested = 0;
      const concItem: StreamConclusionItem = {
        kind: "conclusion",
        seq: 10,
        run_id: "run-agent-1",
        text: "已完成四川省研究区绑定与检索，共检出 **12 景** VV+VH 数据。",
        claims: [],
        thinking_text: "",
        thinking_ms: null,
      };

      const concView = render(
        <AgentConclusionCard
          item={concItem}
          showActionBar
          fallbackDurationMs={2640}
          onRetryTurn={() => {}}
          usageTotal={{ prompt: 4200, completion: 380, total: 4580, cached: 3000 }}
          usageDetail={{
            status: "loaded",
            lines: ["第 1 次模型调用：输入 4,200 · 输出 380"],
          }}
          onRequestUsageDetail={() => {
            detailRequested += 1;
          }}
        />
      );

      // 顶部思考时间（无 <think> 文本时渲染静态耗时标题，无展开箭头）
      const elapsedHeader = concView.getByTestId("agent-thinking-card");
      assert.ok((elapsedHeader.textContent || "").includes("已思考 2.6 秒"));
      assert.strictEqual(concView.queryByTestId("agent-thinking-toggle"), null);

      // 无「未引用证据」
      assert.ok(!(concView.container.textContent || "").includes("未引用证据"));

      // 底部操作栏：纯图标复制 + 纯图标重试 + 同行用量图标
      const actionBar = concView.getByTestId("agent-assistant-turn-actions");
      const asstCopyBtn = concView.getByTestId("btn-copy-assistant-msg");
      const asstRetryBtn = concView.getByTestId("btn-retry-turn");
      const usageIconBtn = concView.getByTestId("btn-turn-usage-icon");

      assert.strictEqual(asstCopyBtn.textContent, "");
      assert.strictEqual(asstRetryBtn.textContent, "");
      assert.strictEqual(usageIconBtn.textContent, "");
      assert.ok(actionBar.contains(asstCopyBtn));
      assert.ok(actionBar.contains(asstRetryBtn));
      assert.ok(actionBar.contains(usageIconBtn));

      // 默认不展示浮窗，鼠标悬停在用量图标上弹出单行本轮汇总（Owner 2026-11-16：不再拆分「第 1 次/第 2 次回复」）
      assert.strictEqual(concView.queryByTestId("turn-usage-popover"), null);
      await act(async () => {
        fireEvent.mouseEnter(usageIconBtn.parentElement!);
      });
      assert.strictEqual(detailRequested, 1);
      const popover = concView.getByTestId("turn-usage-popover");
      assert.ok((popover.textContent || "").includes("本轮模型用量"));
      assert.ok((popover.textContent || "").includes("4,200"));
      assert.ok((popover.textContent || "").includes("380"));
      assert.ok((popover.textContent || "").includes("命中缓存 3,000"), "本轮模型用量展示本轮缓存 Token 数而非会话平均百分比");
      assert.ok(!(popover.textContent || "").includes("%"), "本轮模型用量严禁与会话平均缓存命中率(%)混淆");
      assert.ok(!(popover.textContent || "").includes("第 1 次"), "不得再罗列第 1 次/第 2 次子调用拆分");

      // 同时验证「依据」对裸 UUID 批次引用自动转为人话标题且支持点击回调，绝不向用户裸露 batch-unified-<uuid>
      let jumpedBatchId = "";
      const uuidConcView = render(
        <AgentConclusionCard
          item={{
            ...concItem,
            claims: [
              {
                statement: "本次检索结果批次",
                verified: true,
                refs: [
                  {
                    kind: "batch",
                    id: "batch-unified-38ce5bac-0b71-4391-8033-bbb9fbb7cf7c",
                    label: "批次 batch-unified-38ce5bac-0b71-4391-8033-bbb9fbb7cf7c",
                    trace_sequence: 20,
                    response_count: 12,
                  },
                ],
              },
            ],
          }}
          isEvidenceExpanded
          batchLabelsById={{
            "batch-unified-38ce5bac-0b71-4391-8033-bbb9fbb7cf7c": "Sentinel-1 IW SLC · 12 景",
          }}
          onSelectBatchRef={(bid) => {
            jumpedBatchId = bid;
          }}
        />
      );
      const uuidText = uuidConcView.container.textContent || "";
      assert.ok(!uuidText.includes("38ce5bac"), `不得向用户暴露内部批次 UUID，实际：${uuidText}`);
      assert.ok(uuidText.includes("Sentinel-1 IW SLC · 12 景"), `应展示人话数据源与景数，实际：${uuidText}`);
      fireEvent.click(uuidConcView.getByText("Sentinel-1 IW SLC · 12 景"));
      assert.strictEqual(jumpedBatchId, "batch-unified-38ce5bac-0b71-4391-8033-bbb9fbb7cf7c");
    }
  );

  await suite.test(
    "2. 聊天工具步骤卡片内彻底移除「工作步骤显示设置」入口按钮",
    () => {
      const group: StreamToolGroupItem = {
        kind: "tool_group",
        seq: 2,
        run_id: "run-2",
        status: "done",
        rounds: [
          {
            run_id: "run-2",
            call_id: "c1",
            tool_id: "working_aoi_set",
            status: "done",
            summary_zh: "已绑定四川省",
          },
        ],
      };
      const view = render(<AgentToolGroupCard item={group} isExpanded={true} />);
      assert.strictEqual(view.queryByTestId("open-work-steps-settings-btn"), null);
      assert.ok(!(view.container.textContent || "").includes("工作步骤显示设置"));
    }
  );

  await suite.test(
    "3 & 8a. 设置—外观更名为「通用设置」，并包含「回复进行中发送新消息（排队等待 / 立即引导）」设置项",
    () => {
      setBusySendPrefs({ busySendMode: "queue" });
      const view = render(
        <SettingsModal
          open
          onClose={() => {}}
          initialCategory="appearance"
          theme="light"
          onThemeChange={() => {}}
          cdsePanel={<div />}
          earthdataPanel={<div />}
          bhoonidhiPanel={<div />}
          usgsPanel={<div />}
          storagePanel={<div />}
          concurrencyPanel={<div />}
        />
      );

      assert.ok((view.container.textContent || "").includes("通用设置"));
      assert.ok(view.getByTestId("busy-send-settings-section"));
      const queueBtn = view.getByTestId("busy-send-mode-queue");
      const steerBtn = view.getByTestId("busy-send-mode-steer");
      assert.strictEqual(queueBtn.getAttribute("aria-pressed"), "true");

      fireEvent.click(steerBtn);
      assert.strictEqual(getBusySendPrefs().busySendMode, "steer");
      assert.strictEqual(steerBtn.getAttribute("aria-pressed"), "true");

      fireEvent.click(queueBtn);
      assert.strictEqual(getBusySendPrefs().busySendMode, "queue");
    }
  );

  await suite.test(
    "5 & 7. 输入框单个发送按钮（无快捷键下拉菜单）+ 极简模式单胶囊同行 vs 详细模式底部独立状态条防换行",
    () => {
      setPerformanceOnlyPrefs({ hudMode: "minimal" });

      const view = render(
        <AgentComposer
          compact
          value="继续查询降轨数据"
          onInputChange={() => {}}
          onSubmit={() => {}}
          busy={true}
          allowSendWhileBusy={true}
          channelReadiness={{ ready: true, disabled: false, reason_zh: null }}
          channels={[]}
          sessionId="sess-1"
          contextRuns={[
            {
              run_id: "r1",
              session_id: "sess-1",
              status: "completed",
              started_at: "2026-11-11T10:00:00.000Z",
              ended_at: "2026-11-11T10:00:04.000Z",
              step_count: 2,
              usage_total: { prompt: 8000, completion: 400, total: 8400 },
            },
          ]}
          contextEvents={[]}
          permissionView={null}
          permissionStatus="ready"
          permissionPending={false}
          onPermissionSelect={() => {}}
          onPermissionSetGlobal={() => {}}
          onPermissionClearSession={() => {}}
        />
      );

      // 7: 单个发送按钮，无发送快捷键下拉菜单，且在 busy + allowSendWhileBusy 下保持可点
      const sendBtn = view.getByTestId("agent-send-btn") as HTMLButtonElement;
      assert.strictEqual(sendBtn.disabled, false);
      assert.strictEqual(view.queryByLabelText("切换发送快捷键模式"), null);

      // 5a: 极简模式工具栏触发器只渲染圆圈 SVG 进度环（不含旁侧文字），悬停或点击弹出图 5 详细卡片
      const contextBtn = view.getByTestId("pulse-pill-context");
      assert.strictEqual((contextBtn.textContent || "").trim(), "");
      assert.ok(view.getByTestId("pulse-context-ring-svg"));
      assert.strictEqual(view.queryByTestId("pulse-pill-activity"), null);
      assert.strictEqual(view.queryByTestId("pulse-pill-usage"), null);
      assert.strictEqual(view.queryByTestId("composer-pro-telemetry-bar"), null);

      fireEvent.click(contextBtn);
      const contextPanel = view.getByTestId("pulse-panel-context");
      assert.ok((contextPanel.textContent || "").includes("上下文容量"));
      assert.ok((contextPanel.textContent || "").includes("已用 0.8%"));
      assert.ok(!(contextPanel.textContent || "").includes("瘦身"), "产品界面严禁出现论文式自夸标签");
      assert.ok(view.getByTestId("pulse-context-segmented-bar"));
      assert.ok(view.getByTestId("pulse-context-legend"));
      assert.ok(view.getByTestId("pulse-context-cache-hit-rate"));

      // 5b: 切换到详细模式 -> 遥测胶囊收敛在输入框内部工具栏 agent-composer-toolbar 内，移除外部 composer-pro-telemetry-bar
      act(() => {
        setPerformanceOnlyPrefs({ hudMode: "pro" });
      });
      assert.strictEqual(view.queryByTestId("composer-pro-telemetry-bar"), null);
      const toolbar = view.getByTestId("agent-composer-toolbar");
      assert.ok(toolbar.contains(view.getByTestId("pulse-pill-activity")));
      assert.ok(toolbar.contains(view.getByTestId("pulse-pill-usage")));
      assert.ok(toolbar.contains(view.getByTestId("pulse-pill-context")));

      act(() => {
        setPerformanceOnlyPrefs({ hudMode: "minimal" });
      });
    }
  );

  await suite.test(
    "6c & 8b. AgentComposerEyebrow 默认收拢空间/检索/下载速览，有排队消息时自动切换为待发队列条并支持一键「立即引导」",
    () => {
      const actions: string[] = [];
      const view = render(
        <AgentComposerEyebrow
          hasBoundAoi={true}
          aoiLabel="四川省 (510000)"
          onOpenMap={() => actions.push("map")}
          onClearAoi={() => actions.push("clear-aoi")}
          sourceLabel="Sentinel-1 SAR"
          scenesCount={18}
          checkedCount={3}
          onOpenResults={() => actions.push("results")}
          activeTaskCount={1}
          completedTaskCount={2}
          onOpenTasks={() => actions.push("tasks")}
          queuedMessages={[]}
          onSteerQueued={(id) => actions.push(`steer:${id}`)}
          onEditQueued={(id) => actions.push(`edit:${id}`)}
          onRemoveQueued={(id) => actions.push(`rm:${id}`)}
        />
      );

      // 默认态：工作区速览眉栏
      assert.ok(view.getByTestId("agent-live-context-bar"));
      assert.ok((view.getByTestId("agent-context-aoi-pill").textContent || "").includes("四川省"));
      assert.ok((view.getByTestId("agent-context-batch-pill").textContent || "").includes("18 景"));
      assert.ok((view.getByTestId("agent-context-checked-pill").textContent || "").includes("已勾选 3 景"));
      assert.ok(view.getByTestId("agent-context-tasks-pill"));

      // 存在排队消息时自动切换为待发队列眉栏
      view.rerender(
        <AgentComposerEyebrow
          hasBoundAoi={true}
          aoiLabel="四川省 (510000)"
          onOpenMap={() => {}}
          scenesCount={18}
          checkedCount={3}
          onOpenResults={() => {}}
          activeTaskCount={1}
          completedTaskCount={2}
          onOpenTasks={() => {}}
          queuedMessages={[{ id: "q-1", text: "改成检索成都市 2025 年 10 月升轨 SLC" }]}
          onSteerQueued={(id) => actions.push(`steer:${id}`)}
          onEditQueued={(id) => actions.push(`edit:${id}`)}
          onRemoveQueued={(id) => actions.push(`rm:${id}`)}
        />
      );

      assert.strictEqual(view.queryByTestId("agent-live-context-bar"), null);
      assert.ok(view.getByTestId("agent-queued-messages-eyebrow"));
      assert.ok((view.getByTestId("agent-queued-message-row").textContent || "").includes("成都市"));

      fireEvent.click(view.getByTestId("btn-steer-queued-msg"));
      fireEvent.click(view.getByTestId("btn-edit-queued-msg"));
      fireEvent.click(view.getByTestId("btn-remove-queued-msg"));
      assert.deepStrictEqual(actions, ["steer:q-1", "edit:q-1", "rm:q-1"]);
    }
  );

  await suite.test(
    "5c & 地图抽屉净名：空会话上下文胶囊仅显示 0%（绝不显示 ~0 · 0%），且地图景抽屉剥离内部 PROVIDER:: 前缀",
    () => {
      setPerformanceOnlyPrefs({ hudMode: "minimal" });
      const emptyView = render(
        <AgentComposer
          value=""
          onInputChange={() => {}}
          onSubmit={() => {}}
          busy={false}
          channelReadiness={{ ready: true, disabled: false, reason_zh: "" }}
          channels={[]}
          sessionId={null}
          contextRuns={[]}
          contextEvents={[]}
          permissionView={null}
          permissionStatus="ready"
          permissionPending={false}
          onPermissionSelect={() => {}}
          onPermissionSetGlobal={() => {}}
          onPermissionClearSession={() => {}}
        />
      );
      const pillText = (emptyView.getByTestId("pulse-pill-context").textContent || "").trim();
      assert.strictEqual(pillText, "", "极简模式只渲染圆圈 SVG 进度环，严禁在旁边渲染多余文字");
      assert.ok(emptyView.getByTestId("pulse-context-ring-svg"));
      cleanup();

      const drawerView = render(
        <AgentMapSceneDrawer
          scene={{
            scene_id:
              "PROVIDER::asf.catalog::COLLECTION::SENTINEL-1::ITEM::S1D_IW_SLC__1SDV_20260831T231204_FB7C-SLC",
          }}
        />
      );
      assert.strictEqual(
        drawerView.getByTestId("agent-scene-drawer-title").textContent,
        "S1D_IW_SLC__1SDV_20260831T231204_FB7C-SLC"
      );

      // select_scene 归一化匹配：兼容模型拼接 resource_id + display_name（含处理日期）的场景
      const sampleScenes = [
        {
          scene_id:
            "PROVIDER::microsoft.planetary-computer::COLLECTION::landsat-c2-l2::ITEM::LC08_L2SP_129039_20241027_02_T1",
          product_key:
            "PROVIDER::microsoft.planetary-computer::COLLECTION::landsat-c2-l2::ITEM::LC08_L2SP_129039_20241027_02_T1",
          product_id: "LC08_L2SP_129039_20241027_20241104_02_T1",
          file_name: "LC08_L2SP_129039_20241027_20241104_02_T1",
        },
      ];
      assert.strictEqual(
        matchSceneIdFromList(
          "PROVIDER::science.landsat.c2-l2.v1::COLLECTION::landsat-c2-l2::ITEM::LC08_L2SP_129039_20241027_20241104_02_T1",
          sampleScenes
        ),
        "PROVIDER::microsoft.planetary-computer::COLLECTION::landsat-c2-l2::ITEM::LC08_L2SP_129039_20241027_02_T1"
      );
      assert.strictEqual(
        matchSceneIdFromList("LC08_L2SP_129039_20241027_20241104_02_T1", sampleScenes),
        "PROVIDER::microsoft.planetary-computer::COLLECTION::landsat-c2-l2::ITEM::LC08_L2SP_129039_20241027_02_T1"
      );
      assert.strictEqual(
        matchSceneIdFromList("#1", sampleScenes),
        "PROVIDER::microsoft.planetary-computer::COLLECTION::landsat-c2-l2::ITEM::LC08_L2SP_129039_20241027_02_T1"
      );
      assert.strictEqual(
        inferResourceIdFromSceneId(
          "PROVIDER::microsoft.planetary-computer::COLLECTION::landsat-c2-l2::ITEM::LC08_L2SP_129039_20241027_02_T1"
        ),
        "science.landsat.c2-l2.v1"
      );
      assert.strictEqual(
        inferResourceIdFromSceneId(
          "PROVIDER::asf.catalog::COLLECTION::SENTINEL-1::ITEM::S1A_IW_SLC__1SDV_20241009T110904_F964-SLC"
        ),
        "sar.sentinel1.slc.v1"
      );

      // 当 tool_round(running) 与 tool_round(done) 之间穿插 thinking 事件时，原位归并状态，绝不残留孤立「执行中」
      const mergedItems = classifyStreamEvents([
        {
          seq: 1,
          kind: "tool_round",
          payload: { run_id: "r1", call_id: "call-a", tool_id: "map_workspace_action", status: "running" },
        },
        {
          seq: 2,
          kind: "thinking",
          payload: { run_id: "r1", step_index: 1, status: "done", text: "", thinking_ms: 420 },
        },
        {
          seq: 3,
          kind: "tool_round",
          payload: { run_id: "r1", call_id: "call-a", tool_id: "map_workspace_action", status: "done", duration_ms: 15 },
        },
      ]);
      const toolGroups = mergedItems.filter((it): it is StreamToolGroupItem => it.kind === "tool_group");
      assert.strictEqual(toolGroups.length, 1);
      assert.strictEqual(toolGroups[0].status, "done");
      assert.strictEqual(toolGroups[0].rounds[0].status, "done");
    }
  );

  await suite.test(
    "Batch 2: 思考链逐字波浪动效 + 全工具专属语义图标与简短说明 + 右上角按需任务进度卡 (AITaskList)",
    () => {
      // 1. 思考链：进行中标题渲染逐字波浪动画 (.sp-ai-char-wave) 且保持精确文本「思考中 · X.X 秒」
      const thinkView = render(
        <AgentThinkingCard
          status="thinking"
          thinkingText={"第一步：读取四川行政区边界\n第二步：并发查询 Sentinel-1 与 Sentinel-2"}
          startedAtMs={Date.now() - 1400}
        />
      );
      const toggle = thinkView.getByTestId("agent-thinking-toggle");
      assert.match(toggle.textContent || "", /思考中 · \d+\.\d 秒/);
      assert.ok(toggle.querySelectorAll(".sp-ai-char-wave").length >= 3, "逐字波浪节点已挂载");
      // 折叠进行中思考时显示最新一行预览
      fireEvent.click(toggle);
      assert.ok((thinkView.container.textContent || "").includes("第二步：并发查询 Sentinel-1 与 Sentinel-2"));
      cleanup();

      // 2. 全工具语义映射与简短人话说明（含 GIS / NISAR / 技能 / MCP 外部连接器，绝不泄露裸 snake_case）
      assert.strictEqual(resolveAgentToolDisplayName("nisar_subset_preflight"), "NISAR 范围裁剪预检");
      assert.strictEqual(resolveAgentToolDisplayName("gis_local_process"), "本地栅格与矢量空间处理");
      assert.strictEqual(resolveAgentToolDisplayName("skill_read"), "加载遥感领域技能包");
      assert.strictEqual(resolveAgentToolDisplayName("mcp__web_reader"), "MCP · web reader");
      assert.strictEqual(resolveToolSemanticMeta("working_aoi_set").category, "spatial");
      assert.strictEqual(resolveToolSemanticMeta("search_start").category, "sensing");
      assert.strictEqual(resolveToolSemanticMeta("create_task").category, "delivery");
      assert.strictEqual(resolveToolSemanticMeta("mcp__web_reader").category, "system");

      // 统一交互布局 单色纪律：白底灰黑线、黑底灰白线，严禁给工具图标加彩色染色或彩色底框（即使失败态也保持原工具单色图标）
      for (const tid of ["working_aoi_set", "search_start", "create_task", "mcp__web_reader"]) {
        const meta = resolveToolSemanticMeta(tid);
        assert.strictEqual(meta.iconToneClass, EO_MONOCHROME_ICON_CLASS);
        assert.strictEqual(meta.nodeBgClass, "bg-transparent border-transparent");
      }
      const failedGlyphView = render(
        <div>
          {renderToolSemanticGlyph("mcp__web_reader", { status: "failed" })}
          {renderToolSemanticGlyph("search_start", { status: "done", resultCount: 0 })}
        </div>
      );
      const glyphSvgs = failedGlyphView.container.querySelectorAll("svg");
      assert.strictEqual(glyphSvgs.length, 2);
      glyphSvgs.forEach((svgEl) => {
        const cls = svgEl.getAttribute("class") || "";
        assert.ok(cls.includes("text-[#52525b]") && cls.includes("dark:text-[#a1a1aa]"));
        assert.ok(
          !cls.includes("sky") &&
            !cls.includes("indigo") &&
            !cls.includes("emerald") &&
            !cls.includes("danger") &&
            !cls.includes("warning"),
          `工具图标不得包含任何彩色类: ${cls}`
        );
      });
      cleanup();

      const richToolGroup: StreamToolGroupItem = {
        kind: "tool_group",
        seq: 10,
        run_id: "r-rich",
        status: "running",
        rounds: [
          {
            run_id: "r-rich",
            call_id: "c1",
            tool_id: "skill_read",
            status: "done",
            duration_ms: 45,
          },
          {
            run_id: "r-rich",
            call_id: "c2",
            tool_id: "nisar_subset_preflight",
            status: "running",
          },
        ],
      };
      const tgView = render(<AgentToolGroupCard item={richToolGroup} headless />);
      assert.strictEqual(
        tgView.queryByRole("button", { name: /执行过程/ }),
        null,
        "headless 模式下隐藏二级「执行过程 N 步」折叠头"
      );
      assert.ok((tgView.container.textContent || "").includes("按需载入遥感专业工作流技能规范"));
      assert.ok((tgView.container.textContent || "").includes("预估 NISAR 研究区云端裁剪体积"));
      assert.strictEqual(
        tgView.container.querySelector(".sp-check-draw"),
        null,
        "初始已完成的步骤挂载/展开时绝不重播 sp-check-draw 对勾闪烁"
      );
      tgView.rerender(
        <AgentToolGroupCard
          item={{
            ...richToolGroup,
            status: "done",
            rounds: richToolGroup.rounds.map((r) => ({ ...r, status: "done" })),
          }}
          headless
        />
      );
      assert.ok(
        tgView.container.querySelector(".sp-check-draw"),
        "仅在步骤由 running 实时跃迁为 done 时播放一次自绘对勾"
      );
      cleanup();

      // 3. 右上角任务进度卡：单步简单问答隐藏，多步计划展开展示进度并在完成后支持胶囊收拢/展开/关闭
      const singleStepView = render(
        <AgentTaskProgressCard
          classifiedItems={[
            { kind: "message", seq: 1, role: "user", text: "你好", run_id: "r-1" },
            {
              kind: "tool_group",
              seq: 2,
              run_id: "r-1",
              status: "done",
              rounds: [{ run_id: "r-1", call_id: "c1", tool_id: "source_list", status: "done" }],
            },
          ]}
          activeRunId={null}
        />
      );
      assert.strictEqual(singleStepView.queryByTestId("agent-task-progress-card"), null);
      assert.strictEqual(singleStepView.queryByTestId("agent-task-progress-pill"), null);
      cleanup();

      const multiStepPlan: StreamPlanCardItem = {
        kind: "plan_card",
        seq: 2,
        run_id: "r-multi",
        plan_id: "p-multi",
        plan_hash: "h-multi",
        max_tier: 1,
        steps: [
          { tool_id: "working_aoi_set", label_zh: "锁定四川茂县研究区", tier: 1, status: "done" },
          { tool_id: "search_start", label_zh: "检索 Sentinel-1 IW SLC", tier: 1, status: "running" },
          { tool_id: "search_start", label_zh: "检索 Sentinel-2 低云影像", tier: 1, status: "pending" },
        ],
      };
      const liveCardView = render(
        <AgentTaskProgressCard
          classifiedItems={[
            { kind: "message", seq: 1, role: "user", text: "今天有哪些新数据", run_id: "r-multi" },
            multiStepPlan,
          ]}
          activeRunId="r-multi"
        />
      );
      const card = liveCardView.getByTestId("agent-task-progress-card");
      assert.ok((card.textContent || "").includes("1/3"));
      assert.ok((card.textContent || "").includes("检索 Sentinel-1 IW SLC"));
      const rows = liveCardView.getAllByTestId("agent-task-progress-row");
      assert.strictEqual(rows.length, 3);
      assert.ok(rows[0].className.includes("opacity-65"), "已完成步骤自动降对比度退后");
      assert.ok(rows[1].className.includes("sp-task-shimmer-line"), "执行中步骤带呼吸流光");

      // 已完成历史轮默认以紧凑胶囊呈现，点击可展开，点关闭可隐藏
      liveCardView.rerender(
        <AgentTaskProgressCard
          classifiedItems={[
            { kind: "message", seq: 1, role: "user", text: "今天有哪些新数据", run_id: "r-settled" },
            {
              ...multiStepPlan,
              run_id: "r-settled",
              plan_id: "p-settled",
              steps: multiStepPlan.steps.map((s) => ({ ...s, status: "done" })),
            },
            {
              kind: "conclusion",
              seq: 9,
              run_id: "r-settled",
              text: "已完成全部检索",
              claims: [],
              reasoning_off_refused: false,
            },
          ]}
          activeRunId={null}
        />
      );
      const pill = liveCardView.getByTestId("agent-task-progress-pill");
      assert.strictEqual(pill.getAttribute("data-spring-card"), "true");
      assert.ok((pill.textContent || "").includes("已完成 3/3 步"));
      fireEvent.click(pill.querySelector("button")!);
      const expandedTaskCard = liveCardView.getByTestId("agent-task-progress-card");
      assert.strictEqual(expandedTaskCard.getAttribute("data-spring-card"), "true");
      assert.strictEqual(
        expandedTaskCard.querySelector(".sp-check-draw"),
        null,
        "从胶囊展开已完成清单卡时，已完成对勾保持稳定静止，绝不重播闪烁"
      );
      fireEvent.click(liveCardView.getByLabelText("关闭任务进度卡"));
      assert.strictEqual(liveCardView.queryByTestId("agent-task-progress-card"), null);
    }
  );

  await suite.test(
    "Batch 3: 会话级多数据源批次并存（4 源合并 62 景不覆盖 + 放宽重查原位更新 + 历史事件流恢复）",
    async () => {
      clearAgentSessionWorkspaceCache();
      const mkScenes = (prefix: string, n: number) =>
        Array.from({ length: n }, (_, i) => ({
          scene_id: `${prefix}-${i + 1}`,
          file_name: `${prefix}-${i + 1}.zip`,
          acquisition_datetime: `2026-11-${String(14 - (i % 5)).padStart(2, "0")}T10:00:00Z`,
          footprint_bbox: { west: 103.5, south: 31.5, east: 104.2, north: 32.1 },
        })) as any[];

      await upsertAgentSessionBatch("sess-4src", {
        batchId: "b-s1",
        resourceId: "science.sentinel-1.standard.v1",
        revision: 1,
        scenes: mkScenes("S1", 12),
        rowStates: new Map(),
        totalCount: 12,
      });
      await upsertAgentSessionBatch("sess-4src", {
        batchId: "b-s2",
        resourceId: "science.sentinel-2.l2a.v1",
        revision: 1,
        scenes: mkScenes("S2", 25),
        rowStates: new Map(),
        totalCount: 25,
      });
      await upsertAgentSessionBatch("sess-4src", {
        batchId: "b-nisar",
        resourceId: "science.nisar.l2.v1",
        revision: 1,
        scenes: mkScenes("NISAR", 3),
        rowStates: new Map(),
        totalCount: 3,
      });
      // 第 4 源初查云量过严返回 0 景，不得冲掉前 3 源
      const afterZeroProbe = await upsertAgentSessionBatch("sess-4src", {
        batchId: "b-ls-zero",
        resourceId: "science.landsat.c2-l2.v1",
        revision: 1,
        scenes: [],
        rowStates: new Map(),
        totalCount: 0,
      });
      assert.strictEqual(afterZeroProbe.scenes?.length, 40);

      // 放宽条件重查返回 22 景后，4 类数据并存共 62 景（默认 activeBatchFilter === "all"）
      const finalWs = await upsertAgentSessionBatch("sess-4src", {
        batchId: "b-ls-22",
        resourceId: "science.landsat.c2-l2.v1",
        revision: 2,
        scenes: mkScenes("LC09", 22),
        rowStates: new Map(),
        totalCount: 22,
      });
      assert.strictEqual(finalWs.batches?.length, 4, "4 类数据源必须全部并存，不得只剩最后一类");
      assert.strictEqual(finalWs.activeBatchFilter, "all");
      assert.strictEqual(finalWs.scenes?.length, 62, "全部视图合并 12+25+3+22 = 62 景");

      // 历史事件流提取：过滤掉被后续同源非空批次取代的 0 景试探批次，完整保留 4 个有效批次
      const discovered = extractSessionBatchesFromEvents([
        {
          seq: 1,
          ts: "2026-11-16T00:22:01Z",
          kind: "workspace_sync",
          payload: { action: "search_updated", batch_id: "b-s1", resource_id: "science.sentinel-1.standard.v1", total_count: 12 },
        },
        {
          seq: 2,
          ts: "2026-11-16T00:22:02Z",
          kind: "workspace_sync",
          payload: { action: "search_updated", batch_id: "b-s2", resource_id: "science.sentinel-2.l2a.v1", total_count: 25 },
        },
        {
          seq: 3,
          ts: "2026-11-16T00:22:03Z",
          kind: "workspace_sync",
          payload: { action: "search_updated", batch_id: "b-nisar", resource_id: "science.nisar.l2.v1", total_count: 3 },
        },
        {
          seq: 4,
          ts: "2026-11-16T00:22:04Z",
          kind: "workspace_sync",
          payload: { action: "search_updated", batch_id: "b-ls-zero", resource_id: "science.landsat.c2-l2.v1", total_count: 0 },
        },
        {
          seq: 5,
          ts: "2026-11-16T00:22:08Z",
          kind: "workspace_sync",
          payload: { action: "search_updated", batch_id: "b-ls-22", resource_id: "science.landsat.c2-l2.v1", total_count: 22 },
        },
      ] as any);
      assert.deepEqual(
        discovered.map((d) => d.batchId),
        ["b-s1", "b-s2", "b-nisar", "b-ls-22"]
      );
      clearAgentSessionWorkspaceCache();
    }
  );

  await suite.test(
    "Batch 2.5: 思考计时秒/分/小时多级格式、等宽定宽盒防整行抖动、执行清单外层弹簧与即刻灰态隔离",
    () => {
      // 1. 秒 / 分钟 / 小时三级时长展示 + 数字定宽盒 `[data-timer-digits]` 防抖动
      const secHeader = render(
        <UnifiedTurnProcessHeader
          turnKey="t-sec"
          isActive={true}
          isExpanded={true}
          onToggle={() => {}}
          startedAtMs={Date.now() - 4_200}
          toolStepCount={2}
          runningToolHint="启动数据检索"
        />
      );
      const secBtn = secHeader.getByTestId("agent-turn-process-toggle");
      assert.ok((secBtn.textContent || "").includes("秒"));
      assert.ok((secBtn.textContent || "").includes("正在执行：启动数据检索"));
      const digitBoxes = secBtn.querySelectorAll<HTMLElement>("[data-timer-digits]");
      assert.ok(digitBoxes.length >= 1, "计时数字必须包裹在定宽等宽盒中防止秒数跳字抖动");
      assert.ok(digitBoxes[0].className.includes("font-mono"));
      assert.ok(digitBoxes[0].style.width.includes("ch"));
      cleanup();

      const minHeader = render(
        <UnifiedTurnProcessHeader
          turnKey="t-min"
          isActive={false}
          isExpanded={false}
          onToggle={() => {}}
          durationMs={1_967_000} // 32 分 47 秒
          toolStepCount={4}
        />
      );
      assert.strictEqual(
        minHeader.getByTestId("agent-turn-process-toggle").textContent,
        "已思考 32 分 47 秒 · 4 个步骤"
      );
      cleanup();

      const hourHeader = render(
        <UnifiedTurnProcessHeader
          turnKey="t-hour"
          isActive={false}
          isExpanded={false}
          onToggle={() => {}}
          durationMs={3_725_000} // 1 小时 02 分 05 秒
          toolStepCount={6}
        />
      );
      assert.strictEqual(
        hourHeader.getByTestId("agent-turn-process-toggle").textContent,
        "已思考 1 小时 02 分 05 秒 · 6 个步骤"
      );
      cleanup();

      const inlineHourCard = render(
        <AgentThinkingCard
          status="done"
          thinkingText="长时多源对比推理"
          durationMs={3_725_000}
          embeddedInTurn={true}
        />
      );
      assert.ok(
        (inlineHourCard.getByTestId("agent-thinking-card").textContent || "").includes(
          "思考 · 持续了 1 小时 02 分 05 秒"
        )
      );
      cleanup();

      // 2. 执行清单小卡：外层 `[data-spring-item]` 与内层 `opacity-65 text-sp-text-muted` 严格分离（无 transition-opacity 滞后）
      const taskView = render(
        <AgentTaskProgressCard
          classifiedItems={[
            { kind: "message", seq: 1, role: "user", text: "测试灰态", run_id: "r-gray" },
            {
              kind: "plan_card",
              seq: 2,
              run_id: "r-gray",
              plan_id: "p-gray",
              plan_hash: "h-gray",
              max_tier: 1,
              steps: [
                { tool_id: "working_aoi_set", label_zh: "已完成锁定范围", tier: 1, status: "done" },
                { tool_id: "search_start", label_zh: "正在检索影像", tier: 1, status: "running" },
              ],
            },
          ]}
          activeRunId="r-gray"
        />
      );
      const taskRows = taskView.getAllByTestId("agent-task-progress-row");
      const doneRow = taskRows[0];
      assert.strictEqual(
        doneRow.hasAttribute("data-spring-item"),
        false,
        "内层已完成行本身不得挂载 data-spring-item，避免内联 style.opacity 覆盖 opacity-65"
      );
      assert.ok(
        doneRow.parentElement?.hasAttribute("data-spring-item"),
        "外层包裹节点负责弹簧交错入场"
      );
      assert.ok(doneRow.className.includes("opacity-65"));
      assert.ok(doneRow.className.includes("text-sp-text-muted"));
      assert.strictEqual(
        doneRow.className.includes("transition-opacity"),
        false,
        "不得残留 transition-opacity 导致弹簧落定后再滞后 150ms 变灰"
      );
    }
  );

  await suite.test(
    "Batch 2: 工具步骤左侧定宽耗时列、显式对象标签与点击展开传入参数/返回结果抽屉 + Composer 停用技能过滤与 @ 数据源引用",
    () => {
      const toolItem: StreamToolGroupItem = {
        kind: "tool_group",
        seq: 12,
        run_id: "r-tool-detail",
        status: "done",
        rounds: [
          {
            run_id: "r-tool-detail",
            call_id: "c-sr-1",
            tool_id: "skill_read",
            status: "done",
            duration_ms: 12,
            target_label: "dem-datum-conversion / references/vertical-datum-protocol.md",
            input_summary: "skill_id: dem-datum-conversion · path: references/vertical-datum-protocol.md",
            output_summary: "dem-datum-conversion/references/vertical-datum-protocol.md (4120 B)",
          } as any,
          {
            run_id: "r-tool-detail",
            call_id: "c-gee-1",
            tool_id: "gee_check_availability",
            status: "done",
            duration_ms: 2200,
            target_label: "COPERNICUS/DEM/GLO30",
            input_summary: "dataset_id: COPERNICUS/DEM/GLO30",
            output_summary: "共 126 个云端分块可按需读取",
          } as any,
        ],
      };

      const toolView = render(
        <AgentToolGroupCard item={toolItem} headless={true} />
      );
      const durations = toolView.getAllByTestId("agent-tool-left-duration");
      assert.strictEqual(durations.length, 2);
      assert.strictEqual(durations[0].textContent, "12 毫秒");
      assert.strictEqual(durations[1].textContent, "2.2 秒");
      assert.ok(durations[0].className.includes("w-[58px]"));

      const targets = toolView.getAllByTestId("agent-tool-target-label");
      assert.strictEqual(targets.length, 2);
      assert.strictEqual(
        targets[0].textContent,
        "dem-datum-conversion / references/vertical-datum-protocol.md"
      );
      assert.strictEqual(targets[1].textContent, "COPERNICUS/DEM/GLO30");

      assert.strictEqual(toolView.queryByTestId("agent-tool-row-detail-drawer"), null);
      const triggers = toolView.getAllByTestId("agent-tool-row-trigger");
      fireEvent.click(triggers[0]);
      const drawer = toolView.getByTestId("agent-tool-row-detail-drawer");
      assert.ok(drawer.textContent?.includes("传入参数："));
      assert.ok(drawer.textContent?.includes("references/vertical-datum-protocol.md"));
      assert.ok(drawer.textContent?.includes("返回结果："));
      assert.ok(drawer.textContent?.includes("4120 B"));
      cleanup();

      // `/` 过滤已停用技能 & `@` 弹出数据源引用菜单
      toggleSkillEnabled("radar-satellite-analysis", false);
      try {
        let draftVal = "";
        function ComposerHarness() {
          const [val, setVal] = React.useState("");
          draftVal = val;
          return (
            <AgentComposer
              value={val}
              onInputChange={setVal}
              onSubmit={() => {}}
              channelReadiness={{ ready: true, disabled: false, reason_zh: "" }}
              channels={[]}
              sessionId="sess-mention"
              permissionView={null}
              permissionStatus="ready"
              onPermissionSelect={() => {}}
              onPermissionSetGlobal={() => {}}
            />
          );
        }
        const compView = render(<ComposerHarness />);
        const input = compView.getByRole("textbox", { name: "科学需求输入框" });
        fireEvent.change(input, { target: { value: "/" } });
        assert.ok(compView.getByTestId("agent-skill-picker"));
        assert.ok(
          compView.queryByTestId("skill-picker-item-radar-satellite-analysis") === null
        );
        fireEvent.change(input, { target: { value: "请帮我检索 @sen" } });
        const mentionPicker = compView.getByTestId("agent-dataset-mention-picker");
        assert.ok(mentionPicker);
        fireEvent.click(compView.getByTestId("mention-dataset-Sentinel-1"));
        assert.ok(draftVal.includes("@Sentinel-1 "));
      } finally {
        toggleSkillEnabled("radar-satellite-analysis", true);
        cleanup();
      }
    }
  );

  await suite.test(
    "Batch 4 (Solid Glass + Context Window Meter + Morph Island + Status Glyph): 实体倒角质感、双层同心圆上下文托盘、视口边界夹紧、压缩脉冲与单色点阵动效",
    () => {
      // 1. AgentContextHud：双层同心圆托盘 (.sp-solid-glass-popover) + 内舱 (.sp-solid-glass-card) + 内凹槽 (.sp-field-well / .sp-meter-track) + 压缩脉冲
      const hudView = render(
        <AgentContextHud
          runs={[
            {
              run_id: "r-cwm",
              session_id: "s-cwm",
              status: "completed",
              started_at: "2026-11-18T00:00:00Z",
              ended_at: "2026-11-18T00:00:03Z",
              step_count: 3,
              usage_total: { prompt: 12000, completion: 600, total: 12600 },
            },
          ]}
          events={[]}
          compactionPulse={true}
        />
      );
      const trigger = hudView.getByTestId("pulse-pill-context");
      assert.ok(trigger.className.includes("sp-morph-island-shell"));
      fireEvent.click(trigger);

      const popover = hudView.getByTestId("pulse-hud-popover");
      assert.strictEqual(popover.getAttribute("data-morph-island"), "true");
      assert.strictEqual(popover.getAttribute("data-compaction-pulse"), "true");
      assert.ok(popover.className.includes("sp-solid-glass-popover"));
      assert.ok(popover.className.includes("sp-morph-island-shell"));
      assert.ok(popover.querySelector(".sp-solid-glass-card"));
      assert.ok(hudView.getByTestId("pulse-mode-toggle").className.includes("sp-field-well"));
      assert.ok(
        hudView.getByTestId("pulse-context-segmented-bar").className.includes("sp-meter-track")
      );
      cleanup();

      // 1b. 验证 narrow split-pane 下 AgentContextHud 浮层与 Solid Glass 样式共存并保留水平视口夹紧 (translateX / maxWidth)
      const composerShell = document.createElement("div");
      composerShell.className = "cds-composer-shell";
      composerShell.getBoundingClientRect = () =>
        ({ left: 10, right: 300, top: 400, bottom: 500, width: 290, height: 100, x: 10, y: 400, toJSON() {} }) as DOMRect;
      document.body.appendChild(composerShell);
      try {
        const clampedHud = render(
          <AgentContextHud runs={[]} events={[]} />,
          { container: composerShell }
        );
        const proto = window.Element.prototype;
        const origGetBCR = proto.getBoundingClientRect;
        proto.getBoundingClientRect = function (this: Element) {
          if (this.getAttribute("data-testid") === "pulse-hud-popover") {
            const match = (this as HTMLElement).style.transform.match(/translateX\(([-\d.]+)px\)/);
            const dx = match ? Number(match[1]) : 0;
            return { left: -40 + dx, right: 300 + dx, top: 100, bottom: 380, width: 340, height: 280, x: -40 + dx, y: 100, toJSON() {} } as DOMRect;
          }
          return origGetBCR.call(this);
        };
        try {
          fireEvent.click(clampedHud.getByTestId("pulse-pill-context"));
          const clampedPop = clampedHud.getByTestId("pulse-hud-popover");
          assert.ok(
            clampedPop.style.transform.includes("translateX(") || clampedPop.style.maxWidth.length > 0,
            "narrow split-pane 下脉冲浮层须保留内联视口夹紧样式"
          );
        } finally {
          proto.getBoundingClientRect = origGetBCR;
        }
      } finally {
        cleanup();
        composerShell.remove();
      }

      // 2. Solid Glass Status Glyph：按工作类型区分单色几何运行态点阵 (wave / circuit / orbit)
      const thinkView = render(
        <UnifiedTurnProcessHeader
          turnKey="t-glyph"
          isActive={true}
          isExpanded={false}
          onToggle={() => {}}
          startedAtMs={Date.now() - 1200}
          toolStepCount={2}
        />
      );
      assert.strictEqual(
        thinkView.getByTestId("turn-process-quiet-dots").getAttribute("data-glyph-mode"),
        "wave"
      );
      cleanup();

      const toolView = render(
        <AgentToolGroupCard
          item={{
            kind: "tool_group",
            seq: 1,
            run_id: "r-glyph",
            status: "running",
            rounds: [{ run_id: "r-glyph", call_id: "c1", tool_id: "search_start", status: "running" }],
          }}
        />
      );
      assert.strictEqual(
        toolView.getByTestId("tool-group-quiet-dots").getAttribute("data-glyph-mode"),
        "circuit"
      );
      cleanup();

      const taskProgressView = render(
        <AgentTaskProgressCard
          classifiedItems={[
            { kind: "message", seq: 1, role: "user", text: "多步测试", run_id: "r-orb" },
            {
              kind: "plan_card",
              seq: 2,
              run_id: "r-orb",
              plan_id: "p-orb",
              plan_hash: "h-orb",
              max_tier: 1,
              steps: [
                { tool_id: "working_aoi_set", label_zh: "步骤1", tier: 1, status: "done" },
                { tool_id: "search_start", label_zh: "步骤2", tier: 1, status: "running" },
              ],
            },
          ]}
          activeRunId="r-orb"
        />
      );
      assert.strictEqual(
        taskProgressView.getByTestId("task-progress-quiet-dots").getAttribute("data-glyph-mode"),
        "orbit"
      );
      cleanup();
    }
  );

  await suite.test(
    "Issue #47: single-batch Agent search overrides Map Workbench fallback source label cleanly",
    async () => {
      const mod = await import("@/shell/api/agentSessionWorkspace");
      mod.clearAgentSessionWorkspaceCache();
      const s2Scenes = Array.from({ length: 11 }, (_, i) => ({
        scene_id: `S2A_MSIL2A_202510${String(10 + i).padStart(2, "0")}T024551`,
        file_name: `S2A_MSIL2A_202510${String(10 + i).padStart(2, "0")}.SAFE`,
        product_type: "S2MSI2A",
      })) as any[];
      const ws = await mod.upsertAgentSessionBatch("sess-s2-over-s1", {
        batchId: "b-s2-hefei",
        resourceId: "science.sentinel-2.l2a.v1",
        revision: 1,
        scenes: s2Scenes,
        rowStates: new Map(),
        totalCount: 11,
      });
      const resolvedSingle = mod.resolveAgentSessionSourceLabel({
        batches: ws.batches ?? [],
        activeFilter: ws.activeBatchFilter ?? "all",
        scenes: ws.scenes ?? [],
        effectiveResourceId: ws.resourceId,
        fallbackSourceId: "science.sentinel-1.standard.v1",
        sourceNames: {
          "science.sentinel-1.standard.v1": "Sentinel-1",
          "science.sentinel-2.l2a.v1": "Sentinel-2 L2A",
        },
        fallbackLabel: "Sentinel-1",
      });
      assert.strictEqual(resolvedSingle, "Sentinel-2 L2A");

      // Multi-batch session: "all" reports dataset count, specific batchId resolves that batch's label
      const wsMulti = await mod.upsertAgentSessionBatch("sess-s2-over-s1", {
        batchId: "b-hls-hefei",
        resourceId: "asf.nasa.earthdata",
        revision: 2,
        scenes: [
          { scene_id: "HLS.L30.T50RMA.2025200T023511.v2.0", file_name: "HLS.L30.v2.0" },
        ] as any[],
        rowStates: new Map(),
        totalCount: 1,
      });
      assert.strictEqual(
        mod.resolveAgentSessionSourceLabel({
          batches: wsMulti.batches ?? [],
          activeFilter: "all",
          scenes: wsMulti.scenes ?? [],
          fallbackSourceId: "science.sentinel-1.standard.v1",
          fallbackLabel: "Sentinel-1",
        }),
        "2 类数据源"
      );
      assert.strictEqual(
        mod.resolveAgentSessionSourceLabel({
          batches: wsMulti.batches ?? [],
          activeFilter: "b-hls-hefei",
          scenes: wsMulti.scenes ?? [],
          fallbackSourceId: "science.sentinel-1.standard.v1",
          fallbackLabel: "Sentinel-1",
        }),
        "HLS L30"
      );

      // DEM scene-ID inference resolves canonical labels when raw provider ID is used
      for (const [sceneId, expectedLabel] of [
        ["ASTGTM_NC.003_N31E117", "ASTER GDEM 30m"],
        ["ALPSMLC30_N031E117_DSM", "ALOS World 3D 30m"],
        ["SRTMGL1_N31E117.SRTMGL1", "SRTM GL1 30m"],
      ] as const) {
        assert.strictEqual(
          mod.resolveAgentSessionSourceLabel({
            batches: [
              {
                batchId: `b-${sceneId}`,
                resourceId: "usgs.eros.m2m",
                revision: 1,
                scenes: [{ scene_id: sceneId }] as any[],
                rowStates: new Map(),
                totalCount: 1,
                updatedAt: 1,
              },
            ],
            activeFilter: "all",
            scenes: [{ scene_id: sceneId }] as any[],
            fallbackSourceId: "science.sentinel-1.standard.v1",
            fallbackLabel: "Sentinel-1",
          }),
          expectedLabel
        );
      }

      // Empty Agent draft still falls back to Map Workbench label
      const fallbackEmpty = mod.resolveAgentSessionSourceLabel({
        batches: [],
        activeFilter: "all",
        scenes: [],
        fallbackSourceId: "science.sentinel-1.standard.v1",
        sourceNames: { "science.sentinel-1.standard.v1": "Sentinel-1" },
        fallbackLabel: "Sentinel-1",
      });
      assert.strictEqual(fallbackEmpty, "Sentinel-1");
    }
  );
});
