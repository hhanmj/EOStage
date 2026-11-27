import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { render, cleanup, fireEvent } from "../../test-utils/render-harness";
import { AgentRunListStrip } from "./AgentRunListStrip";
import { AgentComposer } from "./AgentComposer";
import { AgentToolGroupCard } from "./AgentToolGroupCard";
import { AgentPlanCard } from "./AgentPlanCard";
import { AgentWorkPanel } from "./AgentWorkPanel";
import { DesktopTitlebar } from "./DesktopTitlebar";
import { AdminRegionSearch } from "./AdminRegionSearch";
import { AdminBoundaryModal } from "./AdminBoundaryModal";
import { DataSourceCatalogModal } from "./DataSourceCatalogModal";
import {
  saveAoiSearchHistory,
  clearAoiSearchHistory,
  getAoiSearchHistory,
} from "../../lib/aoiBookmarksAndMerge";
import { WorkStepsSettingsTab } from "../../shell/components/settings/WorkStepsSettingsTab";
import { PerformanceUsageSettingsTab } from "../../shell/components/settings/PerformanceUsageSettingsTab";
import type {
  AgentRunSummary,
  AgentPermissionModeView,
  AgentWorkItem,
} from "../../lib/generated/v3-contracts";
import type {
  StreamToolGroupItem,
  StreamPlanCardItem,
} from "../../shell/api/agentStreamLogic";
import type { ApplicationTask } from "../../shell/api/types";

test.afterEach(() => {
  cleanup();
});

const mockPermissionView: AgentPermissionModeView = {
  effective: "confirm_changes",
  session_override: null,
  global_default: "confirm_changes",
};

const mockChannelReadiness = {
  ready: true,
  disabled: false,
  reason_zh: "",
};

test("EOStage Agent 工作台交互测试套件", async (suite) => {
  await suite.test(
    "1. InputShellBeardBand 同心圆包裹器与 InputShellEyebrow 任务追踪眉栏交互",
    () => {
      const runs: AgentRunSummary[] = [
        {
          run_id: "run-1",
          session_id: "sess-1",
          status: "completed",
          started_at: "2026-11-06 10:00:00",
          ended_at: "2026-11-06 10:01:00",
          model_route: null,
        },
        {
          run_id: "run-2",
          session_id: "sess-1",
          status: "running",
          started_at: "2026-11-06 10:02:00",
          ended_at: null,
          model_route: null,
        },
      ];

      const view = render(
        <AgentComposer
          compact
          eyebrow={<AgentRunListStrip runs={runs} variant="eyebrow" />}
          value=""
          onInputChange={() => {}}
          onSubmit={() => {}}
          channelReadiness={mockChannelReadiness}
          channels={[]}
          sessionId="sess-1"
          permissionView={mockPermissionView}
          permissionStatus="ready"
          onPermissionSelect={() => {}}
          onPermissionSetGlobal={() => {}}
        />
      );

      const band = view.getByTestId("eo-input-shell-band");
      const inner = view.getByTestId("eo-input-shell-inner");
      const eyebrow = view.getByTestId("task-tracker-eyebrow");
      const counter = view.getByTestId("task-tracker-eyebrow-counter");

      assert.ok(band.classList.contains("eo-concentric-band"));
      assert.ok(inner.classList.contains("eo-concentric-inner"));
      assert.strictEqual(eyebrow.getAttribute("data-variant"), "eyebrow");
      assert.strictEqual(counter.textContent, "1 / 2");

      // 默认未展开步骤行
      assert.strictEqual(view.queryByTestId("task-tracker-eyebrow-rows"), null);

      // 点击切换展开步骤行
      fireEvent.click(view.getByRole("button", { name: "运行记录 2" }));
      const rows = view.getByTestId("task-tracker-eyebrow-rows");
      assert.ok(rows);
      assert.strictEqual(view.getAllByTestId("task-tracker-eyebrow-row").length, 2);
    }
  );

  await suite.test(
    "2. ChatCollapsible 单行工具流与左侧 1px 折叠导轨点击收起",
    () => {
      let expandedState = true;
      const toolItem: StreamToolGroupItem = {
        kind: "tool_group",
        seq: 10,
        run_id: "run-1",
        status: "running",
        rounds: [
          {
            run_id: "run-1",
            call_id: "c1",
            tool_id: "resolve_aoi",
            status: "done",
          },
          {
            run_id: "run-1",
            call_id: "c2",
            tool_id: "search_start",
            status: "running",
          },
        ],
      };

      const view = render(
        <AgentToolGroupCard
          item={toolItem}
          resolveToolName={(id) => id}
          isExpanded={expandedState}
          onToggleExpand={() => {
            expandedState = !expandedState;
          }}
        />
      );

      assert.strictEqual(view.getAllByTestId("eo-chat-event-item").length, 2);
      const rail = view.getByTestId("eo-tool-collapse-rail");
      assert.ok(rail);

      // 点击左侧 1px 折叠导轨触发收起
      fireEvent.click(rail);
      assert.strictEqual(expandedState, false);
    }
  );

  await suite.test(
    "3. PlanCardUI 流式呼吸脉冲边框与步骤完成状态呈现",
    () => {
      const planItem: StreamPlanCardItem = {
        kind: "plan_card",
        seq: 12,
        run_id: "run-1",
        plan_id: "plan-1",
        plan_hash: "hash-1",
        steps: [
          {
            tool_id: "search_start",
            label_zh: "检索 Sentinel-1 SLC 影像",
            tier: 1,
            status: "running",
          },
        ],
      };

      const view = render(
        <AgentPlanCard item={planItem} progressStatus="running" />
      );
      const card = view.getByTestId("eo-plan-card");
      assert.ok(card.classList.contains("eo-concentric-band"));
      assert.ok(card.classList.contains("eo-artifact-card-pulse"));
      assert.ok(view.getByText("执行中"));
    }
  );

  await suite.test(
    "4. AgentWorkPanel 分段式进度条 (eo-segmented-progress) 与凹陷内槽",
    () => {
      const work: AgentWorkItem[] = [
        {
          run: {
            run_id: "r1",
            session_id: "s1",
            status: "completed",
            started_at: "2026-11-06T09:00:00Z",
            ended_at: "2026-11-06T09:01:00Z",
            model_route: null,
          },
          request: "检索大同盆地 Sentinel-1",
          plan_ids: [],
          created_task_ids: ["t1"],
        },
        {
          run: {
            run_id: "r2",
            session_id: "s1",
            status: "running",
            started_at: "2026-11-06T10:00:00Z",
            ended_at: null,
            model_route: null,
          },
          request: "下载九寨沟 ALOS-2",
          plan_ids: [],
          created_task_ids: [],
        },
      ];
      const tasks: ApplicationTask[] = [
        {
          task_id: "t1",
          revision: 1,
          category: "scene_download",
          name: "大同盆地 S1 批次",
          status: "running",
          done_bytes: 50,
          total_bytes: 100,
        },
      ];

      const view = render(<AgentWorkPanel work={work} tasks={tasks} />);
      const segBar = view.getByTestId("eo-segmented-progress");
      assert.strictEqual(segBar.children.length, 2);
    }
  );

  await suite.test(
    "5. SettingsCard + SettingsCardContent + SettingsCardBeard (#/ds/settings) 同心底托与分段胶囊切换",
    () => {
      const view = render(
        <div>
          <WorkStepsSettingsTab />
          <PerformanceUsageSettingsTab />
        </div>
      );
      const cards = view.container.querySelectorAll(".eo-settings-card");
      const contents = view.container.querySelectorAll(".eo-settings-card-content");
      const beards = view.container.querySelectorAll(".eo-settings-card-beard");
      assert.strictEqual(cards.length, 2);
      assert.strictEqual(contents.length, 2);
      assert.strictEqual(beards.length, 2);

      const detailedBtn = view.getByTestId("work-step-mode-detailed");
      fireEvent.click(detailedBtn);
      assert.strictEqual(detailedBtn.getAttribute("data-active"), "true");
    }
  );

  await suite.test(
    "6. DesktopTitlebar 与 AgentComposer 零假壳核验：移除伪菜单/伪语音按钮，真实绑定历史栈、AOI 胶囊与 / 技能呼出",
    () => {
      let aoiClicked = false;
      let rectClicked = false;
      const view = render(
        <div>
          <DesktopTitlebar
            activeRoute="workbench"
            contextSubtitle="Sentinel-1 SAR"
            canGoBack={false}
            canGoForward={true}
            aoiSummary="合肥市"
            onOpenAoiModal={() => {
              aoiClicked = true;
            }}
            panelToggle={{ open: false, onToggle: () => {}, sceneCount: 12 }}
          />
          <AgentComposer
            value=""
            onInputChange={() => {}}
            onSubmit={() => {}}
            channelReadiness={{ ready: true, disabled: false, reason_zh: "" }}
            channels={[]}
            sessionId={null}
            permissionView={mockPermissionView}
            permissionStatus="ready"
            onPermissionSelect={() => {}}
            onPermissionSetGlobal={() => {}}
            onDrawRect={() => {
              rectClicked = true;
            }}
          />
        </div>
      );

      // 确认伪 VS Code 菜单与伪更新胶囊已彻底移除
      assert.strictEqual(view.queryByText("文件"), null);
      assert.strictEqual(view.queryByText("编辑"), null);
      assert.strictEqual(view.queryByText("前往"), null);
      assert.strictEqual(view.queryByText("版本更新 v2.4"), null);
      assert.strictEqual(view.queryByLabelText("Voice input"), null);
      assert.strictEqual(view.queryByLabelText("Voice mode"), null);

      // 验证后退按钮在 canGoBack=false 时真实 disabled，前进在 canGoForward=true 时可用
      const backBtn = view.getByLabelText("后退") as HTMLButtonElement;
      const forwardBtn = view.getByLabelText("前进") as HTMLButtonElement;
      assert.strictEqual(backBtn.disabled, true);
      assert.strictEqual(forwardBtn.disabled, false);

      // 验证顶栏真实显示当前工作台与子上下文，且 AOI 胶囊可真实点击
      assert.ok(view.getByText("地图工作台"));
      assert.ok(view.getByText("Sentinel-1 SAR"));
      const aoiPill = view.getByTestId("titlebar-aoi-pill");
      fireEvent.click(aoiPill);
      assert.strictEqual(aoiClicked, true);

      // 验证 Composer 附加菜单空间框选与空输入框按 `/` 呼出技能面板
      const attachBtn = view.getByLabelText("附加范围或文件");
      fireEvent.click(attachBtn);
      const rectItem = view.getByTestId("attach-menu-item-rect-draw");
      fireEvent.click(rectItem);
      assert.strictEqual(rectClicked, true);

      const textarea = view.container.querySelector("textarea")!;
      fireEvent.keyDown(textarea, { key: "/" });
      assert.ok(view.getByTestId("agent-skill-picker"));
    }
  );

  await suite.test(
    "7. 顶栏右上角简洁下载浮窗（不直接跳下载中心）+ 右移模块切换器 + 行政区检索历史功能",
    async () => {
      clearAoiSearchHistory();
      let enteredDownloadCenter = false;
      let openedDrawerTaskId: string | undefined = undefined;

      const sampleTasks: ApplicationTask[] = [
        {
          task_id: "dl-task-1",
          revision: 1,
          category: "scene_download",
          name: "Sentinel-1 SLC 茂县批次",
          status: "running",
          done_bytes: 512 * 1024 * 1024,
          total_bytes: 1024 * 1024 * 1024,
        },
      ];

      const view = render(
        <div>
          <DesktopTitlebar
            activeRoute="workbench"
            contextSubtitle="Sentinel-1 雷达卫星"
            activeDownloadCount={1}
            downloadTasks={sampleTasks}
            onOpenDownloadCenter={() => {
              enteredDownloadCenter = true;
            }}
            onOpenDownloadDrawer={(taskId) => {
              openedDrawerTaskId = taskId ?? "drawer-root";
            }}
          />
        </div>
      );

      // 点击右上角下载按钮：打开简洁浮窗，不直接跳转下载中心
      const dlTrigger = view.getByTestId("titlebar-download-trigger");
      fireEvent.click(dlTrigger);
      assert.strictEqual(enteredDownloadCenter, false);
      assert.ok(view.getByTestId("titlebar-download-popover"));
      assert.ok(view.getByText("Sentinel-1 SLC 茂县批次"));

      // 点击浮窗内「侧弹窗查看」触发侧弹窗回调
      fireEvent.click(view.getByTestId("titlebar-open-download-drawer-btn"));
      assert.strictEqual(openedDrawerTaskId, "drawer-root");

      // 重新打开浮窗并点击「进入下载中心」才触发进入下载中心
      fireEvent.click(dlTrigger);
      fireEvent.click(view.getByTestId("titlebar-enter-download-center-btn"));
      assert.strictEqual(enteredDownloadCenter, true);

      // 验证行政区检索历史（AdminRegionSearch + AdminBoundaryModal）
      saveAoiSearchHistory({
        label: "茂县叠溪",
        bbox: { west: 103.5, south: 31.8, east: 103.75, north: 32.0, crs: "EPSG:4326" },
        geometry: {
          type: "Polygon",
          coordinates: [[[103.5, 31.8], [103.75, 31.8], [103.75, 32.0], [103.5, 32.0], [103.5, 31.8]]],
        },
        adminLevel: "县域",
      });
      assert.strictEqual(getAoiSearchHistory().length, 1);

      let appliedHistoryLabel = "";
      const searchView = render(
        <div>
          <AdminRegionSearch
            workingAoi={null}
            workingGeometry={null}
            aoiLabel="全球视图"
            areaText=""
            onPreviewGeometry={() => {}}
            onReplaceAoi={async (_geom, label) => {
              appliedHistoryLabel = label;
            }}
          />
          <AdminBoundaryModal
            open={true}
            onClose={() => {}}
            onSelectBoundary={async (_geom, label) => {
              appliedHistoryLabel = label;
            }}
          />
        </div>
      );

      // AdminBoundaryModal 默认展示最近检索历史
      assert.ok(searchView.getByTestId("admin-modal-history"));
      assert.strictEqual(searchView.getAllByTestId("admin-modal-history-item").length, 1);

      // AdminRegionSearch 点击历史按钮展开历史面板并可一键应用
      fireEvent.click(searchView.getByTestId("aoi-history-trigger"));
      assert.ok(searchView.getByTestId("aoi-history-popover"));
      fireEvent.click(searchView.getAllByText("茂县叠溪")[0]);
      assert.strictEqual(appliedHistoryLabel, "茂县叠溪");
      clearAoiSearchHistory();
    }
  );

  await suite.test(
    "8. 数据清单入口与行政区检索/数据清单统一圆角核验",
    () => {
      let catalogOpenedFromTitlebar = false;
      const view = render(
        <div>
          <DesktopTitlebar
            activeRoute="agent"
            onOpenDataCatalog={() => {
              catalogOpenedFromTitlebar = true;
            }}
          />
          <DataSourceCatalogModal
            isOpen={true}
            onClose={() => {}}
            availableSources={[]}
            activeResourceId="cdse:sentinel-1:slc"
            onSelectSource={() => {}}
            workingBbox={null}
          />
          <AdminBoundaryModal
            open={true}
            onClose={() => {}}
            onSelectBoundary={async () => {}}
          />
        </div>
      );

      // 顶栏下拉菜单包含「数据清单」且移除凑数项「技能与插件」
      fireEvent.click(view.getByTestId("titlebar-workspace-switcher"));
      const catalogMenuBtn = view.getByTestId("titlebar-menu-data-catalog");
      assert.ok(catalogMenuBtn);
      assert.strictEqual(view.queryByText("技能与插件"), null);
      fireEvent.click(catalogMenuBtn);
      assert.strictEqual(catalogOpenedFromTitlebar, true);

      // 数据清单弹窗与行政区检索弹窗均使用平滑圆角 (rounded-[14px])
      const catalogModal = view.getByTestId("data-source-catalog-modal");
      assert.ok(catalogModal.classList.contains("rounded-[14px]"));
      const adminDialog = view.getByRole("dialog", { name: "行政区划边界定位" });
      const adminCard = adminDialog.firstElementChild as HTMLElement;
      assert.ok(adminCard.classList.contains("rounded-[14px]"));
    }
  );

  await suite.test(
    "9. 行政区检索：最近检索历史位于候选列表下方，且输入后自动检索无需点击按钮",
    async () => {
      clearAoiSearchHistory();
      saveAoiSearchHistory({
        label: "汶川县, 阿坝州",
        query: "汶川",
        bbox: { west: 102.85, south: 30.75, east: 103.73, north: 31.72 },
        geometry: {
          type: "Polygon",
          coordinates: [
            [
              [102.85, 30.75],
              [103.73, 30.75],
              [103.73, 31.72],
              [102.85, 31.72],
              [102.85, 30.75],
            ],
          ],
        },
        adminLevel: "县级",
      });

      const autoSearchQueries: string[] = [];
      const view = render(
        <AdminBoundaryModal
          open={true}
          onClose={() => {}}
          onSelectBoundary={async () => {}}
          services={{
            search: async (q) => {
              autoSearchQueries.push(q);
              return {
                ok: true,
                query: q,
                results: [
                  {
                    boundary_id: "510100",
                    provider: "datav",
                    provider_object_id: "510100",
                    label: "成都市",
                    display_name: "成都市, 四川省",
                    admin_level: "地级市",
                    bbox: { west: 102.98, south: 30.09, east: 104.89, north: 31.43 },
                  },
                ],
              };
            },
          }}
        />
      );

      // 1. 候选结果列表在 DOM 顺序上位于最近检索历史上方（即最近检索历史在候选列表下方）
      const candidatesEl = view.getByTestId("admin-modal-candidates");
      const historyEl = view.getByTestId("admin-modal-history");
      assert.ok(
        Boolean(candidatesEl.compareDocumentPosition(historyEl) & Node.DOCUMENT_POSITION_FOLLOWING),
        "最近检索历史应位于候选列表下方"
      );

      // 2. 输入框输入关键词后无需点击按钮，自动防抖触发检索并渲染候选结果
      const input = view.getByTestId("admin-modal-search-input");
      fireEvent.change(input, { target: { value: "成都市" } });
      const candidateItem = await view.findByTestId("admin-modal-candidate-item");
      assert.deepStrictEqual(autoSearchQueries, ["成都市"]);
      assert.ok(candidateItem.textContent?.includes("成都市, 四川省"));
      clearAoiSearchHistory();
    }
  );
});
