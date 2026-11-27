import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { render, cleanup, fireEvent } from "../../test-utils/render-harness";
import { registerHooks } from "node:module";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { transformSync } from "esbuild";
import { AgentToolGroupCard } from "./AgentToolGroupCard";
import { AgentComposer } from "./AgentComposer";
import { AgentPermissionMenu } from "./AgentPermissionMenu";
import { AgentPlanCard } from "./AgentPlanCard";
import { AgentConfirmCard } from "./AgentConfirmCard";
import { CatalogResultRow } from "./CatalogResultRow";
import type { AgentPermissionModeView } from "../../lib/generated/v3-contracts";

if (typeof window !== "undefined") {
  if (typeof window.requestAnimationFrame === "undefined") {
    window.requestAnimationFrame = (fn) => setTimeout(fn, 16) as any;
  }
  if (typeof window.cancelAnimationFrame === "undefined") {
    window.cancelAnimationFrame = (id) => clearTimeout(id as any);
  }
}

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const scopedSource = (url: string) => `import.meta.env ??= { DEV: true };\n${readFileSync(fileURLToPath(url), "utf8")}`;
const hooks = registerHooks({
  load(url, context, nextLoad) {
    if (url.endsWith("/DesktopAgentWorkbench.tsx") || url.endsWith("/WorkbenchMap.tsx")) {
      return {
        format: "module",
        shortCircuit: true,
        source: transformSync(scopedSource(url), { loader: "tsx", format: "esm", jsx: "automatic" }).code,
      };
    }
    return nextLoad(url, context);
  },
});

const { DesktopAgentWorkbench } = await import("./DesktopAgentWorkbench");
const { WorkbenchMap } = await import("../WorkbenchMap");
hooks.deregister();

test.afterEach(() => {
  cleanup();
});

const mockChannelReadiness = {
  ready: true,
  disabled: false,
  reason_zh: "",
};

const mockPermissionView: AgentPermissionModeView = {
  current_mode: "confirm_each",
  source: "default",
  is_global: true,
  can_clear_session: false,
};

test("Idea Spark (灵光一闪) 动效套件：Dock Menu 地图图例 + Adaptive Pill Navbar + 科研微动效", async (t) => {
  await t.test("1. Dock Menu 地图图例：单容器原位形变、色点微缩预览、展开双列网格与 Escape/外部点击收起", () => {
    const sampleBbox = { west: 103.8, south: 30.5, east: 104.3, north: 31.0 };
    const sampleScenes: any[] = [
      {
        scene_id: "S1A_ASC_001",
        dataset_id: "SENTINEL-1",
        platform: "Sentinel-1A",
        product_type: "SLC",
        beam_mode: "IW",
        polarization: "VV+VH",
        orbit_direction: "ASCENDING",
        acquisition_datetime: "2025-05-01T00:00:00Z",
        footprint_bbox: sampleBbox,
      },
      {
        scene_id: "S1A_DESC_002",
        dataset_id: "SENTINEL-1",
        platform: "Sentinel-1A",
        product_type: "SLC",
        beam_mode: "IW",
        polarization: "VV+VH",
        orbit_direction: "DESCENDING",
        acquisition_datetime: "2025-05-02T00:00:00Z",
        footprint_bbox: sampleBbox,
      },
      {
        scene_id: "S2A_OPT_003",
        dataset_id: "SENTINEL-2",
        platform: "Sentinel-2A",
        product_type: "S2MSI2A",
        beam_mode: "MSI",
        polarization: "",
        orbit_direction: "DESCENDING",
        sensor: "MSI",
        acquisition_datetime: "2025-05-03T00:00:00Z",
        footprint_bbox: sampleBbox,
      },
    ];

    const { getByTestId } = render(
      <WorkbenchMap
        declutter
        aoiBbox={sampleBbox}
        demEnvelopeBbox={sampleBbox}
        scenes={sampleScenes}
        topControls={<button type="button" data-testid="custom-top-control">顶栏附加按钮</button>}
      />
    );

    // 验证 topControls 与 Dock Legend 并排存在
    const customTopCtrl = getByTestId("custom-top-control");
    const dockLegend = getByTestId("map-dock-legend");
    const toggleBtn = getByTestId("map-dock-legend-toggle");
    assert.ok(customTopCtrl, "topControls 应与图例并排渲染");
    assert.ok(dockLegend, "应渲染单容器形变图例 map-dock-legend");

    // 初始收起态
    assert.equal(dockLegend.getAttribute("data-legend-state"), "closed");
    assert.equal(dockLegend.classList.contains("is-open"), false);
    assert.equal(toggleBtn.getAttribute("aria-expanded"), "false");

    // 包含色点微缩预览与双列网格容器
    const dotsPreview = dockLegend.querySelector(".sp-dock-dots");
    const gridWrap = dockLegend.querySelector(".sp-dock-grid-wrap");
    assert.ok(dotsPreview, "应包含收起态色点微缩预览 .sp-dock-dots");
    assert.ok(gridWrap, "应包含双列网格折叠容器 .sp-dock-grid-wrap");

    // 点击展开
    fireEvent.click(toggleBtn);
    assert.equal(dockLegend.getAttribute("data-legend-state"), "open");
    assert.equal(dockLegend.classList.contains("is-open"), true);
    assert.equal(toggleBtn.getAttribute("aria-expanded"), "true");

    const dockItems = dockLegend.querySelectorAll(".sp-dock-item");
    assert.ok(dockItems.length >= 4, "展开态应包含 AOI、DEM、升轨、降轨、光学等双列图例条目");

    // 按下 Escape 收起
    fireEvent.keyDown(document, { key: "Escape" });
    assert.equal(dockLegend.getAttribute("data-legend-state"), "closed");
    assert.equal(toggleBtn.getAttribute("aria-expanded"), "false");

    // 再次点击展开并通过外部 pointerdown 收起
    fireEvent.click(toggleBtn);
    assert.equal(dockLegend.getAttribute("data-legend-state"), "open");
    assert.ok(
      dockLegend.style.getPropertyValue("--sp-dock-closed-w").endsWith("px") &&
        dockLegend.style.getPropertyValue("--sp-dock-open-w").endsWith("px") &&
        dockLegend.style.getPropertyValue("--sp-dock-open-h").endsWith("px"),
      "Dock 图例应注入 --sp-dock-closed-w / --sp-dock-open-w / --sp-dock-open-h 2D 宽高同步形变变量"
    );
    fireEvent.pointerDown(document.body);
    assert.equal(dockLegend.getAttribute("data-legend-state"), "closed");
    cleanup();

    // 验证按需显示：仅有矩形 AOI (aoiGeometry) 但未显式传入 demEnvelopeBbox 时，绝不伪造「DEM 下载范围」图例
    const rectAoiGeojson = {
      type: "Polygon",
      coordinates: [
        [
          [103.5, 31.8],
          [103.75, 31.8],
          [103.75, 32.0],
          [103.5, 32.0],
          [103.5, 31.8],
        ],
      ],
    };
    const { getByTestId: getByTestIdNoDem } = render(
      <WorkbenchMap
        declutter
        aoiBbox={sampleBbox}
        aoiGeometry={rectAoiGeojson as any}
        scenes={sampleScenes.slice(0, 2)}
      />
    );
    const noDemDock = getByTestIdNoDem("map-dock-legend");
    assert.equal(
      noDemDock.textContent?.includes("DEM") ?? false,
      false,
      "未传入 demEnvelopeBbox 时，矩形 AOI 下图例不得出现 DEM 下载范围"
    );
  });

  await t.test("2. Adaptive Pill Navbar：激活项常驻展开、非激活项默认收起仅图标、悬停丝滑去模糊展开、点击切换滑块", () => {
    const { getByTestId } = render(
      <DesktopAgentWorkbench
        panelOpen={true}
        channelReadiness={mockChannelReadiness}
        channels={[]}
        permissionView={mockPermissionView}
      />
    );

    const pillNav = getByTestId("agent-adaptive-pill-nav");
    const indicator = getByTestId("agent-panel-pill-indicator");
    const mapTab = getByTestId("agent-panel-tab-map");
    const resultsTab = getByTestId("agent-panel-tab-results");
    const taskTab = getByTestId("agent-panel-tab-task");

    assert.ok(pillNav, "应渲染自适应胶囊导航栏");
    assert.ok(indicator, "应渲染底层自适应弹簧白色滑块");

    // 默认激活「地图」标签，仅「地图」展开文字，其余两项默认收起为仅图标
    assert.equal(pillNav.getAttribute("data-active-tab"), "map");
    assert.equal(indicator.getAttribute("data-active-tab"), "map");
    assert.equal(mapTab.getAttribute("data-expanded"), "true");
    assert.equal(resultsTab.getAttribute("data-expanded"), "false");
    assert.equal(taskTab.getAttribute("data-expanded"), "false");

    // 悬停非激活的「结果清单」标签：文字顺滑去模糊展开
    fireEvent.mouseEnter(resultsTab);
    assert.equal(resultsTab.getAttribute("data-expanded"), "true", "悬停非激活标签时应展开文字");
    assert.equal(mapTab.getAttribute("data-expanded"), "true", "激活标签在悬停其他标签时仍保持展开");

    // 鼠标移出胶囊导航栏：非激活标签恢复仅图标收起态
    fireEvent.mouseLeave(pillNav);
    assert.equal(resultsTab.getAttribute("data-expanded"), "false", "移出后非激活标签应恢复收起");

    // 点击切换到「结果清单」标签
    fireEvent.click(resultsTab);
    assert.equal(pillNav.getAttribute("data-active-tab"), "results");
    assert.equal(indicator.getAttribute("data-active-tab"), "results");
    assert.equal(resultsTab.getAttribute("data-expanded"), "true");
    assert.equal(mapTab.getAttribute("data-expanded"), "false");

    // 验证当激活项为「结果清单」时，悬停左侧「地图」标签会同步在第 0 帧推进滑块坐标（单阶段几何锁定）
    const leftBeforeHoverMap = indicator.style.transform;
    fireEvent.mouseEnter(mapTab);
    assert.notEqual(
      indicator.style.transform,
      leftBeforeHoverMap,
      "悬停左侧非激活标签展开时，右侧激活滑块应在第 0 帧同步位移"
    );
    fireEvent.mouseLeave(pillNav);

    // 点击切换到「任务与成果」标签
    fireEvent.click(taskTab);
    assert.equal(pillNav.getAttribute("data-active-tab"), "task");
    assert.equal(indicator.getAttribute("data-active-tab"), "task");
    assert.equal(taskTab.getAttribute("data-expanded"), "true");
    assert.equal(resultsTab.getAttribute("data-expanded"), "false");
  });

  await t.test("3. 科研级微动效与无障碍降级：边缘把手 Magnetic Peek、图层面板 Blur-to-Sharp 与 reduced-motion 守卫", () => {
    const cssPath = path.resolve(__dirname, "../../index.css");
    const cssSource = readFileSync(cssPath, "utf-8");

    assert.match(cssSource, /--sp-spring-dock:/, "index.css 应定义 Dock 形变物理阻尼曲线");
    assert.match(cssSource, /--sp-spring-pill:/, "index.css 应定义 Adaptive Pill 弹簧物理曲线");
    assert.match(cssSource, /\.sp-popover-blur-in\b/, "index.css 应定义 Blur-to-Sharp 浮层入场动效");
    assert.match(cssSource, /\.sp-magnetic-peek-left\b/, "index.css 应定义左侧磁吸探出微动效");
    assert.match(cssSource, /\.sp-magnetic-peek-right\b/, "index.css 应定义右侧磁吸探出微动效");
    assert.match(
      cssSource,
      /@media\s*\(prefers-reduced-motion:\s*reduce\)[\s\S]*?\.sp-dock-legend[\s\S]*?\.sp-adaptive-pill-indicator/,
      "prefers-reduced-motion: reduce 必须覆盖新增的 Dock 与 Adaptive Pill 动效类"
    );

    const { getByTestId } = render(
      <DesktopAgentWorkbench
        panelOpen={false}
        channelReadiness={mockChannelReadiness}
        channels={[]}
        permissionView={mockPermissionView}
      />
    );
    const expandHandle = getByTestId("agent-expand-map-handle");
    assert.equal(expandHandle.classList.contains("sp-magnetic-peek-left"), true, "展开把手应具备磁吸探出类");
  });

  await t.test("4. Inspora 第二弹 · 地图左侧工具栏「共享流体提示胶囊」(Fluid Shared Tooltip)：沿 Y 轴弹簧跟随滑移", () => {
    const { getByTestId } = render(<WorkbenchMap declutter />);

    const toolbar = getByTestId("map-toolbar");
    const zoomInBtn = getByTestId("map-tool-btn-zoom-in");
    const drawRectBtn = getByTestId("map-tool-btn-draw-rect");
    const fluidTooltip = getByTestId("map-toolbar-fluid-tooltip");

    assert.equal(fluidTooltip.getAttribute("data-visible"), "false");

    // 悬停「放大」按钮：流体胶囊浮现并定位到第 0 项高度
    fireEvent.mouseEnter(zoomInBtn);
    assert.equal(fluidTooltip.getAttribute("data-visible"), "true");
    assert.equal(fluidTooltip.getAttribute("data-tooltip-label"), "放大");
    const firstY = fluidTooltip.style.getPropertyValue("--sp-tooltip-y");

    // 滑向「矩形框选」按钮：同一共享胶囊沿 Y 轴平滑滑移并切换文字
    fireEvent.mouseEnter(drawRectBtn);
    assert.equal(fluidTooltip.getAttribute("data-visible"), "true");
    const secondY = fluidTooltip.style.getPropertyValue("--sp-tooltip-y");
    assert.notEqual(firstY, secondY, "移动到不同工具按钮时，共享胶囊 Y 坐标应更新");

    // 鼠标移出工具栏：流体胶囊原位隐退
    fireEvent.mouseLeave(toolbar);
    assert.equal(fluidTooltip.getAttribute("data-visible"), "false");
  });

  await t.test("5. Inspora 第二弹 · 工具调用卡「叠片暗示与抽屉级联展开」(Folder Stack Peek & Staggered Drawer)", () => {
    const { getByTestId, queryByTestId, getByRole } = render(
      <AgentToolGroupCard
        item={{
          kind: "tool_group",
          seq: 1,
          run_id: "r-inspora",
          status: "done",
          rounds: [
            {
              run_id: "r-inspora",
              call_id: "c1",
              tool_id: "resolve_aoi",
              status: "done",
              summary_zh: "成功绑定研究区边界",
            },
            {
              run_id: "r-inspora",
              call_id: "c2",
              tool_id: "search_scenes",
              status: "done",
              summary_zh: "检索到 8 景 Sentinel-1 SLC 影像",
            },
          ],
        }}
        defaultExpanded={false}
      />
    );

    // 收起态：渲染双层微缩叠片边缘暗示，不渲染抽屉详情
    const card = getByTestId("agent-tool-group-card");
    assert.equal(card.getAttribute("data-expanded"), "false");
    assert.ok(getByTestId("agent-tool-folder-peek"), "收起态应展示叠片边缘暗示 .sp-tool-folder-peek");
    assert.equal(queryByTestId("agent-tool-drawer"), null);

    // 点击展开：叠片切换为抽屉详情，步骤卡带级联延迟 --sp-step-idx
    const triggerBtn = getByRole("button", { name: /执行过程 2 步/ });
    fireEvent.click(triggerBtn);
    assert.equal(card.getAttribute("data-expanded"), "true");
    assert.equal(queryByTestId("agent-tool-folder-peek"), null);

    const drawer = getByTestId("agent-tool-drawer");
    const stepCards = drawer.querySelectorAll(".sp-tool-step-card");
    assert.equal(stepCards.length, 2);
    assert.equal((stepCards[0] as HTMLElement).style.getPropertyValue("--sp-step-idx"), "0");
    assert.equal((stepCards[1] as HTMLElement).style.getPropertyValue("--sp-step-idx"), "1");
  });

  await t.test("6. Inspora 第二弹 · 输入台「+ 附加菜单」与「权限菜单」流体跟随高亮 (Fluid Menu Highlight)", () => {
    const { getByRole, getByTestId } = render(
      <div>
        <AgentComposer
          value=""
          onInputChange={() => {}}
          onSubmit={() => {}}
          channelReadiness={mockChannelReadiness}
          channels={[]}
          sessionId="test-session"
          permissionView={{
            ...mockPermissionView,
            effective: "confirm_each",
          } as any}
          permissionStatus="ready"
          onPermissionSelect={() => {}}
          onPermissionSetGlobal={() => {}}
        />
      </div>
    );

    // 1) 打开「+ 附加」菜单并验证流体跟随高亮
    const attachBtn = getByRole("button", { name: "附加范围或文件" });
    fireEvent.click(attachBtn);
    const attachHighlight = getByTestId("attach-menu-fluid-highlight");
    assert.equal(attachHighlight.getAttribute("data-active-item"), "");

    const rectItem = getByTestId("attach-menu-item-rect-draw");
    const workflowItem = getByTestId("attach-menu-item-workflow");
    fireEvent.mouseEnter(rectItem);
    assert.equal(attachHighlight.getAttribute("data-active-item"), "rect-draw");
    assert.equal(attachHighlight.classList.contains("is-active"), true);

    fireEvent.mouseEnter(workflowItem);
    assert.equal(attachHighlight.getAttribute("data-active-item"), "workflow");

    // 2) 打开「权限」菜单并验证流体跟随高亮
    const permBtn = getByRole("button", { name: "权限模式" });
    fireEvent.click(permBtn);
    const permHighlight = getByTestId("permission-menu-fluid-highlight");
    const autoApproveItem = getByTestId("permission-menu-item-auto_approve");
    fireEvent.mouseEnter(autoApproveItem);
    assert.equal(permHighlight.getAttribute("data-active-mode"), "auto_approve");
    assert.equal(permHighlight.classList.contains("is-active"), true);
  });

  await t.test("7. Inspora 第二弹 · 结果清单行与下载操作「纵向焦点游标 + 原位图标形变」(Tactile Row & In-place Morph)", () => {
    const sampleScene: any = {
      scene_id: "S1A_IW_SLC_20250501",
      file_name: "S1A_IW_SLC__1SDV_20250501T102030_058999_074F1A_1234.zip",
      dataset_id: "SENTINEL-1",
      orbit_direction: "ascending",
      acquisition_datetime: "2025-05-01 10:20:30",
      polarization: "VV+VH",
      path: 142,
      frame: 95,
      file_size_remote: 4294967296,
    };

    const { getByTestId, rerender } = render(
      <CatalogResultRow
        scene={sampleScene}
        isSelected={true}
        isChecked={true}
        isCreating={false}
      />
    );

    const cursor = getByTestId("result-row-cursor");
    const morphWrap = getByTestId("row-download-icon-morph");
    assert.ok(cursor, "结果行应包含左侧纵向焦点游标 .sp-result-row-cursor");
    assert.equal(morphWrap.getAttribute("data-state"), "idle");

    // 切换为 isCreating=true 时，原位图标容器状态切换为 creating，不卸载容器节点
    rerender(
      <CatalogResultRow
        scene={sampleScene}
        isSelected={true}
        isChecked={true}
        isCreating={true}
      />
    );
    assert.equal(getByTestId("row-download-icon-morph").getAttribute("data-state"), "creating");
  });

  await t.test("8. Open Claude UI (CDS) 第三弹：输入台 20px 双环阴影、MCP 双色决策卡片头、3 点思考波浪与双纹胶囊分栏手柄", () => {
    const cssPath = path.resolve(__dirname, "../../index.css");
    const cssSource = readFileSync(cssPath, "utf-8");

    assert.match(cssSource, /\.cds-composer-shell\b/, "index.css 应定义 20px 双环弥散阴影输入台 .cds-composer-shell");
    assert.match(cssSource, /\.cds-btn-squish\b/, "index.css 应定义 15 阶弹簧按压 .cds-btn-squish");
    assert.match(cssSource, /\.cds-decision-card\b/, "index.css 应定义 MCP 决策式卡片 .cds-decision-card");
    assert.match(cssSource, /\.timeline-quiet-dots\b/, "index.css 应定义 3 点静默思考波浪 .timeline-quiet-dots");
    assert.match(cssSource, /\.cds-split-pill\b/, "index.css 应定义双纹胶囊分栏手柄 .cds-split-pill");

    // 验证运行态工具组渲染 3 点静默思考波浪，并渲染计划卡与确认卡 MCP 双色顶栏
    window.innerWidth = 1440;
    const { getByTestId } = render(
      <div>
        <AgentToolGroupCard
          item={{
            kind: "tool_group",
            seq: 2,
            run_id: "r-cds",
            status: "running",
            rounds: [
              {
                run_id: "r-cds",
                call_id: "c-run",
                tool_id: "search_scenes",
                status: "running",
                summary_zh: "正在检索影像",
              },
            ],
          }}
        />
        <AgentPlanCard
          item={{
            kind: "plan_card",
            seq: 3,
            run_id: "r-cds",
            plan_id: "p-cds",
            auto_executed: true,
            steps: [{ step_id: "s1", label_zh: "步骤一：解析研究区", status: "done" }],
          }}
        />
        <AgentConfirmCard
          item={{
            kind: "confirm_card",
            seq: 4,
            run_id: "r-cds",
            plan_id: "p-cds",
            auto_executed: false,
            snapshot: {
              action: "create_task",
              source: "copernicus_dataspace",
              scene_ids: ["S2A_001"],
              total_bytes: 1024 * 1024 * 120,
              output_dir: "D:/data",
              output_dir_origin: "default",
            } as any,
          }}
        />
        <DesktopAgentWorkbench
          panelOpen={true}
          sidebarWidth={0}
          channelReadiness={mockChannelReadiness}
          channels={[]}
          permissionView={mockPermissionView}
        />
      </div>
    );

    assert.ok(getByTestId("tool-group-quiet-dots"), "运行态工具组应显示 3 点静默思考波浪");
    assert.ok(getByTestId("cds-split-pill"), "分栏拖拽条应包含 CDS 双纹胶囊手柄");
    const planCard = document.querySelector('[aria-label="计划卡"]');
    assert.ok(planCard?.classList.contains("cds-decision-card"), "AgentPlanCard 应使用 .cds-decision-card");
    const confirmCard = document.getElementById("agent-confirm-card");
    assert.ok(confirmCard?.classList.contains("cds-decision-card"), "AgentConfirmCard 应使用 .cds-decision-card");
    assert.ok(confirmCard?.classList.contains("is-pending-approval"), "待确认态 AgentConfirmCard 应具备 .is-pending-approval");
  });
});
