import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  stackKey,
  describeSceneStack,
  buildStacks,
  stackFor,
  pageForScene,
  mapIdentityOf,
  buildIdentityIndex,
} from "./mapLinkLogic.ts";
import { shortSourceLabel } from "./manualConditionsLogic.ts";
import {
  parseManualWorkbenchSubscene,
  getSourceViews,
  getBatchPage,
  getTasks,
  __setManualWorkbenchSubsceneForTest,
  __resetFixtureState,
} from "./fixture.ts";
import { rowProgressPercent, failureSummaryText } from "./resultListLogic.ts";
import type { SceneRow } from "../../lib/bridge.ts";
import { assertLocalizedSource } from "./i18nSourceTestUtils.test.ts";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

test("W5-3e: Result Rows & Map Two-way Linkage Suite", async (t) => {
  await t.test("1. stackKey: rows missing relative_orbit or frame never stack", () => {
    assert.equal(stackKey(null), null);
    assert.equal(stackKey(undefined), null);
    assert.equal(stackKey({ relative_orbit: null, frame: 100 }), null);
    assert.equal(stackKey({ relative_orbit: 69, frame: null }), null);
    assert.equal(stackKey({ relative_orbit: undefined, frame: 100 }), null);
    assert.equal(stackKey({ relative_orbit: 69, frame: undefined }), null);
    assert.equal(stackKey({ relative_orbit: Number.NaN, frame: 100 }), null);
    assert.equal(stackKey({ relative_orbit: 69, frame: Number.POSITIVE_INFINITY }), null);
    assert.equal(stackKey({ relative_orbit: 69, frame: 450 }), "69:450");
    assert.equal(
      stackKey({ orbit_direction: "ASCENDING", relative_orbit: 69, frame: 450 }),
      "ASC:69:450",
    );
    assert.equal(
      stackKey({ orbit_direction: "DESCENDING", path: 142, burst_id: "071_151224_IW2" }),
      "DESC:142:071_151224_IW2",
    );
    assert.equal(
      stackKey({
        orbit_direction: "ASC",
        track: 12,
        footprint_bbox: { west: 117.1, south: 31.7, east: 117.3, north: 31.9 },
      }),
      "ASC:12:L318",
    );

    // 光学 Sentinel-2 MGRS 图幅、Landsat WRS-2 Path/Row 与空间范围兜底归组
    assert.equal(
      stackKey({
        file_name: "S2A_MSIL2A_20260726T030511_N0511_R075_T50RKU_20260726T061200.SAFE.zip",
      }),
      "MGRS:T50RKU",
    );
    assert.equal(
      describeSceneStack({
        file_name: "S2A_MSIL1C_20260726T030511_N0511_R075_T50RKU_20260726T061200.SAFE",
      }).groupLabel,
      "MGRS 图幅 T50RKU",
    );
    assert.equal(
      stackKey({
        scene_id: "LC09_L2SP_121038_20260401_20260403_02_T1",
      }),
      "WRS:121:038",
    );
    assert.equal(
      describeSceneStack({
        scene_id: "LC09_L2SP_121038_20260401_20260403_02_T1",
      }).shortTag,
      "121/038",
    );
    assert.equal(
      stackKey({
        footprint_bbox: { west: 117.02, south: 31.58, east: 117.81, north: 32.19 },
      }),
      "BBOX:117.0:31.6:117.8:32.2",
    );
  });

  await t.test("2. buildStacks and stackFor: same RO+Frame from different dates stack", () => {
    const rows: SceneRow[] = [
      {
        scene_id: "scene-1",
        relative_orbit: 69,
        frame: 450,
        acquisition_datetime: "2024-04-08T10:01:32Z",
      } as SceneRow,
      {
        scene_id: "scene-2",
        relative_orbit: 69,
        frame: 450,
        acquisition_datetime: "2024-03-27T10:01:32Z",
      } as SceneRow,
      {
        scene_id: "scene-solo",
        relative_orbit: 84,
        frame: 100,
        acquisition_datetime: "2024-04-09T22:08:15Z",
      } as SceneRow,
      {
        scene_id: "scene-no-frame",
        relative_orbit: 69,
        frame: null,
        acquisition_datetime: "2024-04-01T10:01:32Z",
      } as SceneRow,
    ];

    const stacks = buildStacks(rows);
    assert.equal(stacks.size, 1, "只有成员数量 >= 2 的键才能存入 stacks");
    assert.deepEqual(stacks.get("69:450"), ["scene-1", "scene-2"]);

    // stackFor
    assert.deepEqual(stackFor(rows, "scene-1"), ["scene-1", "scene-2"]);
    assert.deepEqual(stackFor(rows, "scene-2"), ["scene-1", "scene-2"]);
    assert.deepEqual(stackFor(rows, "scene-solo"), ["scene-solo"], "单成员场景 stackFor 返回自身");
    assert.deepEqual(stackFor(rows, "scene-no-frame"), ["scene-no-frame"], "无分堆键场景 stackFor 返回自身");
  });

  await t.test("3. pageForScene: index 0, 49, 50 and missing handling", () => {
    const sortedIds = Array.from({ length: 120 }, (_, i) => `scene-${i}`);

    assert.equal(pageForScene(sortedIds, "scene-0", 50), 1, "第 0 个场景属于第 1 页");
    assert.equal(pageForScene(sortedIds, "scene-49", 50), 1, "第 49 个场景属于第 1 页");
    assert.equal(pageForScene(sortedIds, "scene-50", 50), 2, "第 50 个场景属于第 2 页");
    assert.equal(pageForScene(sortedIds, "scene-119", 50), 3, "第 119 个场景属于第 3 页");
    assert.equal(pageForScene(sortedIds, "scene-non-existent", 50), null, "不存在的场景返回 null");
  });

  await t.test("4. Source Guard: Map tools rely on in-map floating toolbar and share unified AOI handlers", () => {
    const manualWorkbenchPath = path.resolve(__dirname, "../../components/desktop/ManualWorkbench.tsx");
    const manualWorkbenchSource = fs.readFileSync(manualWorkbenchPath, "utf-8");

    // 已删除重复的面板头工具栏 renderMapPanelHead（Owner 要求空间让给地图，绘图经由地图内工具条）
    assert.strictEqual(
      manualWorkbenchSource.includes("renderMapPanelHead"),
      false,
      "已删除地图列顶部重复的标题栏与绘制按钮行，空间让给地图"
    );

    // renderWorkbenchMap 直接使用外部传入的 onRectDraw / onPolygonDraw，无第二套 AOI 状态
    assert.match(
      manualWorkbenchSource,
      /onRectDraw=\{onRectDraw\}[\s\S]*?onPolygonDraw=\{onPolygonDraw\}/,
      "WorkbenchMap 绘制回调直接连接顶层统一 AOI 处理函数，不存在独立或分叉的 AOI 状态"
    );
  });

  await t.test("5. Source Guard: Two-way linkage props, hover style, map hint, and stack popover", () => {
    const workbenchMapPath = path.resolve(__dirname, "../../components/WorkbenchMap.tsx");
    const workbenchMapSource = fs.readFileSync(workbenchMapPath, "utf-8");
    const catalogResultRowPath = path.resolve(__dirname, "../../components/desktop/CatalogResultRow.tsx");
    const catalogResultRowSource = fs.readFileSync(catalogResultRowPath, "utf-8");
    const manualWorkbenchPath = path.resolve(__dirname, "../../components/desktop/ManualWorkbench.tsx");
    const manualWorkbenchSource = fs.readFileSync(manualWorkbenchPath, "utf-8");

    // 1. CatalogResultRow 包含 data-scene-id 和 onHoverChange
    assert.match(
      catalogResultRowSource,
      /data-scene-id=\{scene\.scene_id\}/,
      "CatalogResultRow 必须设置 data-scene-id 以便点选定位与滚动"
    );
    assert.match(
      catalogResultRowSource,
      /onMouseEnter=\{[^{}]*onHoverChange\?\.[\s\S]*?onMouseLeave=\{[^{}]*onHoverChange\?\./,
      "CatalogResultRow 必须在鼠标悬停时触发 onHoverChange"
    );

    // 2. WorkbenchMap 支持 hoveredSceneId 并提供加粗 hover 描边
    assert.match(
      workbenchMapSource,
      /hovered\s*&&\s*!partSelected/,
      "MapSceneFootprintEntry 必须在悬停且未选中时应用高亮悬停样式"
    );
    assert.match(
      workbenchMapSource,
      /3\.2/,
      "悬停足迹描边权重必须为 3.2 (介于 normal 1.8 与 selected 4.8 之间)"
    );

    // 3. Map hint 必须存在且仅在有足迹时显示
    assertLocalizedSource(workbenchMapSource, "panels.map.footprintHoverHint", "悬停行高亮足迹，点足迹定位到行");

    // 4. Stacks popover 必须渲染同幅清单（Owner 2026-11-17 /grill-me Q1 统一口径：覆盖 SAR 与光学 MGRS/WRS）
    assert.match(
      workbenchMapSource,
      /activeStackScenes[\s\S]*?panels\.map\.sameTrackStack/,
      "WorkbenchMap 必须为多成员分堆提供同幅清单弹层"
    );
    assertLocalizedSource(workbenchMapSource, "panels.map.sameTrackStack", "{scenes} 景同幅清单");

    // 5. ManualWorkbench 维护联动状态
    assert.match(
      manualWorkbenchSource,
      /hoveredSceneId=\{[\s\S]*?hoveredSceneId[\s\S]*?\}[\s\S]*?stackScenesFor=\{stackScenesFor\}/,
      "ManualWorkbench 必须向 WorkbenchMap 传入 hoveredSceneId 与 stackScenesFor"
    );
  });

  await t.test("6. Identity Mapping: mapIdentityOf and buildIdentityIndex round trip with disparate scene_id and productKey", () => {
    const s1Row: SceneRow = {
      scene_id: "S1A_IW_SLC__1SDV_20240408T100132_20240408T100159_053341_067676_8A12",
      platform: "Sentinel-1A",
      beam_mode: "IW",
      product_type: "SLC",
      polarization: "VV+VH",
      acquisition_datetime: "2024-04-08T10:01:32Z",
      absolute_orbit: 53341,
      relative_orbit: 69,
      path: 69,
      frame: 450,
      orbit_direction: "ascending",
    } as SceneRow;

    const identity = mapIdentityOf(s1Row);
    // identity 来源于 sceneProductKey，必须是非空字符串且不等于原生 scene_id
    assert.ok(identity && identity.length > 0, "mapIdentityOf 必须返回非空 identity");
    assert.notEqual(identity, s1Row.scene_id, "S1 的 sceneIdentity 绝不等于原生 scene_id");

    const index = buildIdentityIndex([s1Row]);
    assert.equal(index.toIdentity.get(s1Row.scene_id), identity, "toIdentity 映射正确");
    assert.equal(index.toSceneId.get(identity), s1Row.scene_id, "toSceneId 映射正确");

    // 从 identity 反查 stack 和 page
    const resolvedSceneId = index.toSceneId.get(identity)!;
    assert.equal(resolvedSceneId, s1Row.scene_id);
    const stack = stackFor([s1Row], resolvedSceneId);
    assert.deepEqual(stack, [s1Row.scene_id]);
    const page = pageForScene([s1Row.scene_id], resolvedSceneId, 50);
    assert.equal(page, 1);
  });

  await t.test("7. Coordinator P1: ManualWorkbench no longer renders DesktopAssistantWidget", () => {
    const manualWorkbenchPath = path.resolve(__dirname, "../../components/desktop/ManualWorkbench.tsx");
    const manualWorkbenchSource = fs.readFileSync(manualWorkbenchPath, "utf-8");

    // 不再导入与渲染 DesktopAssistantWidget
    assert.doesNotMatch(
      manualWorkbenchSource,
      /DesktopAssistantWidget/,
      "ManualWorkbench 严禁导入或渲染 DesktopAssistantWidget"
    );

    // 不再包含右下角浮动固定助手容器
    assert.doesNotMatch(
      manualWorkbenchSource,
      /absolute\s+bottom-4\s+right-4/,
      "ManualWorkbench 严禁在右下角保留浮动固定助手按钮容器"
    );
  });

  await t.test("8. Coordinator P2: shortSourceLabel drops a space-separated Chinese description, keeps attached CJK, and trims", () => {
    // 示例断言
    assert.equal(shortSourceLabel("Sentinel-1 雷达卫星"), "Sentinel-1");
    // W5-8 R7：汉字紧贴在拉丁词后（「L波段」）属于名称本身，不截断
    assert.equal(shortSourceLabel("NISAR L波段 (ASF)"), "NISAR L波段 (ASF)");
    assert.equal(shortSourceLabel("Sentinel-2 多光谱"), "Sentinel-2");
    assert.equal(shortSourceLabel("ALOS-2 PALSAR-2"), "ALOS-2 PALSAR-2");

    // 边界条件：纯中文回退全名，空值回退空字符串
    assert.equal(shortSourceLabel("高分一号"), "高分一号");
    assert.equal(shortSourceLabel(""), "");
    assert.equal(shortSourceLabel(null), "");
    assert.equal(shortSourceLabel(undefined), "");

    // 源码断言：ManualConditionsPanel 不再使用 max-w-[90px]
    const panelPath = path.resolve(__dirname, "../../components/desktop/ManualConditionsPanel.tsx");
    const panelSource = fs.readFileSync(panelPath, "utf-8");
    assert.doesNotMatch(
      panelSource,
      /max-w-\[90px\]/,
      "ManualConditionsPanel 来源摘要严禁限制为固定 90px 宽度"
    );
  });

  await t.test("9. Source Guard: WorkbenchMap fitScope unions AOI with displaySceneUnion and leaves default unchanged", () => {
    const workbenchMapPath = path.resolve(__dirname, "../../components/WorkbenchMap.tsx");
    const workbenchMapSource = fs.readFileSync(workbenchMapPath, "utf-8");
    const mapLogicPath = path.resolve(__dirname, "./mapLinkLogic.ts");
    const mapLogicSource = fs.readFileSync(mapLogicPath, "utf-8");

    // mapLinkLogic 中严禁包含死代码 unionFitBbox
    assert.doesNotMatch(
      mapLogicSource,
      /unionFitBbox/,
      "mapLinkLogic 严禁保留未使用的死代码 unionFitBbox"
    );

    // aoi-and-results 分支合并 baseAoiBbox 与 displaySceneUnion
    assert.match(
      workbenchMapSource,
      /fitScope === ["']aoi-and-results["'][\s\S]*?unionBbox\(\[baseAoiBbox,\s*sceneUnion\]\)/,
      "WorkbenchMap 必须在 aoi-and-results 模式下将 AOI 与足迹并集"
    );

    // 默认行为保持 strictWorkspaceAoi 与原有 AOI 优先逻辑
    assert.match(
      workbenchMapSource,
      /if \(strictWorkspaceAoi\) return DEFAULT_BBOX;/,
      "WorkbenchMap 默认模式必须保留原有严格工作区 AOI 优先级不变"
    );
  });

  await t.test("10. Pure Helper & Safety: parseManualWorkbenchSubscene and production gating", () => {
    // 纯函数解析测试
    assert.equal(parseManualWorkbenchSubscene("#/?subscene=empty"), "empty");
    assert.equal(parseManualWorkbenchSubscene("#/?subscene=results-8&other=1"), "results-8");
    assert.equal(parseManualWorkbenchSubscene("?subscene=results-tasks"), "results-tasks");
    assert.equal(parseManualWorkbenchSubscene("#/workbench?subscene=results-8"), "results-8");

    // 无效或未知值
    assert.equal(parseManualWorkbenchSubscene("#/?subscene=unknown"), null);
    assert.equal(parseManualWorkbenchSubscene(""), null);
    assert.equal(parseManualWorkbenchSubscene(null), null);
    assert.equal(parseManualWorkbenchSubscene(undefined), null);

    // 生产安全性断言：index.ts 严禁导出无门控的 manual subscene，所有 fixture 必须受 isFixtureBackend 保护
    const indexPath = path.resolve(__dirname, "./index.ts");
    const indexSource = fs.readFileSync(indexPath, "utf-8");
    assert.doesNotMatch(
      indexSource,
      /export const [a-zA-Z0-9_]*ManualWorkbenchSubscene/,
      "index.ts 严禁暴露无门控的 subscene 常量或类型"
    );
    assert.match(
      indexSource,
      /isFixtureBackend\(\)\s*&&\s*fixture/,
      "index.ts 必须通过 isFixtureBackend() 严格门控 fixture"
    );
  });

  await t.test("11. Results-tasks consistency: row task_id resolves in getTasks with exact percent and failure first sentence, and disabled checkbox visible", async () => {
    // 1. 设置 subscene 为 results-tasks 并重置状态
    __setManualWorkbenchSubsceneForTest("results-tasks");
    __resetFixtureState();

    try {
      const views = await getSourceViews();
      const s1View = views.find((v) => v.resource_id === "science.sentinel-1.standard.v1");
      assert.ok(s1View && s1View.batch_id, "results-tasks 必须初始化 Sentinel-1 批次");

      const page = await getBatchPage(s1View.batch_id, s1View.revision);
      const rowsWithTask = page.rows.filter((r) => r.task_id !== null);
      assert.equal(rowsWithTask.length, 4, "results-tasks 必须恰好有 4 行处于任务中");

      const tasks = await getTasks();
      const taskMap = new Map(tasks.map((t) => [t.task_id, t]));

      // 验证每一个带 task_id 的行都在 getTasks 中有匹配任务
      for (const row of rowsWithTask) {
        assert.ok(row.task_id && taskMap.has(row.task_id), `行状态中的 task_id ${row.task_id} 必须在 getTasks() 中存在`);
      }

      // 验证 running 任务：items_count === 1，且精确为 46%
      const runningRow = rowsWithTask.find((r) => r.task_status === "running")!;
      assert.ok(runningRow, "必须包含 running 行");
      const runningTask = taskMap.get(runningRow.task_id!)!;
      assert.equal(runningTask.items_count, 1, "running 任务 items_count 必须为 1");
      assert.equal(rowProgressPercent(runningTask), 46, "running 任务进度必须精确为 46%");

      // 验证 paused 任务：items_count === 1，且精确为 31%
      const pausedRow = rowsWithTask.find((r) => r.task_status === "paused")!;
      assert.ok(pausedRow, "必须包含 paused 行");
      const pausedTask = taskMap.get(pausedRow.task_id!)!;
      assert.equal(pausedTask.items_count, 1, "paused 任务 items_count 必须为 1");
      assert.equal(rowProgressPercent(pausedTask), 31, "paused 任务进度必须精确为 31%");

      // 验证 failed 任务：failure 字段与第一句消息
      const failedRow = rowsWithTask.find((r) => r.task_status === "failed")!;
      assert.ok(failedRow, "必须包含 failed 行");
      const failedTask = taskMap.get(failedRow.task_id!)!;
      assert.ok(failedTask.failure, "failed 任务必须包含 failure 结构体");
      assert.equal(failedTask.failure?.failed_items, 1);
      assert.equal(failedTask.failure?.total_items, 1);
      assert.equal(
        failureSummaryText(failedTask.failure),
        "下载连接中断，网络请求超时。",
        "failureSummaryText 必须为多句错误消息的第一句话"
      );

      // 验证 completed 任务
      const completedRow = rowsWithTask.find((r) => r.task_status === "completed")!;
      assert.ok(completedRow, "必须包含 completed 行");
      const completedTask = taskMap.get(completedRow.task_id!)!;
      assert.equal(completedTask.status, "completed");

      // 2. 源码守卫：CatalogResultRow 的禁用复选框可见性
      const rowPath = path.resolve(__dirname, "../../components/desktop/CatalogResultRow.tsx");
      const rowSource = fs.readFileSync(rowPath, "utf-8");
      const cbBlockMatch = rowSource.match(/\[grid-area:cb\][\s\S]*?<\/button>/);
      assert.ok(cbBlockMatch, "CatalogResultRow 必须包含复选框容器");
      const cbBlock = cbBlockMatch[0];
      // 严禁对 disabled checkbox 使用 opacity-50
      assert.doesNotMatch(
        cbBlock,
        /opacity-50/,
        "CatalogResultRow 复选框容器严禁包含 opacity-50"
      );
      // 必须使用可见空框样式的类名
      assert.match(
        cbBlock,
        /!selectable[\s\S]*?bg-sp-surface-muted[\s\S]*?ring-1\s+ring-sp-line[\s\S]*?cursor-not-allowed/,
        "CatalogResultRow 必须为 disabled checkbox 渲染可见的空框样式"
      );
    } finally {
      __setManualWorkbenchSubsceneForTest(null);
      __resetFixtureState();
    }
  });
});
