/**
 * DataSourceCatalogModal.interaction.test.tsx
 * 数据源目录弹窗交互测试套件（地图工作台改版第 5 步）
 */

import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { render, cleanup, fireEvent } from "../../test-utils/render-harness";
import { within } from "@testing-library/react";
import { DataSourceCatalogModal } from "./DataSourceCatalogModal";
import { ManualWorkbench, type ManualWorkbenchProps } from "./ManualWorkbench";
import type { SourceDescriptor } from "@/shell/api/types";
import { clearRecentSources, getRecentSourceIds } from "@/lib/dataSourceCatalogLogic";
import { getWorkbenchToolOptions, updateWorkbenchToolOptions } from "./ManualConditionsPanel";

test.afterEach(() => {
  cleanup();
  clearRecentSources();
  updateWorkbenchToolOptions({
    geeAssetId: "ALL",
    geeTheme: "all",
    geeSelectedPresetId: "science.gee.catalog.v1",
  });
});

const mockSources: SourceDescriptor[] = [
  {
    resource_id: "science.sentinel-1.standard.v1",
    group_id: "sentinel1",
    display_name_zh: "Sentinel-1",
    family: "sar",
    search_mode: "sar",
    filter_schema_id: null,
    filter_schema_revision: null,
    result_cap: 1000,
    accounts: ["asf.earthdata"],
    network_default: "direct",
    availability: "available",
    availability_reason_zh: null,
  },
  {
    resource_id: "science.sentinel-1.slc-burst.v1",
    group_id: "sentinel1",
    display_name_zh: "Sentinel-1 SLC Burst",
    family: "sar",
    search_mode: "sar",
    filter_schema_id: null,
    filter_schema_revision: null,
    result_cap: 1000,
    accounts: ["asf.earthdata"],
    network_default: "direct",
    availability: "available",
    availability_reason_zh: null,
  },
  {
    resource_id: "science.sentinel-2.l2a.v1",
    group_id: "sentinel2",
    display_name_zh: "Sentinel-2 MSI",
    family: "optical",
    search_mode: "catalogue_intent",
    filter_schema_id: "copernicus.s2.v1",
    filter_schema_revision: 1,
    result_cap: 500,
    accounts: ["copernicus.cdse"],
    network_default: "direct",
    availability: "available",
    availability_reason_zh: null,
  },
  {
    resource_id: "science.landsat.c2-l2.v1",
    group_id: "landsat",
    display_name_zh: "Landsat 8-9 C2-L2",
    family: "optical",
    search_mode: "catalogue_intent",
    filter_schema_id: "usgs.landsat.v1",
    filter_schema_revision: 1,
    result_cap: 500,
    accounts: ["usgs.m2m"],
    network_default: "direct",
    availability: "available",
    availability_reason_zh: null,
  },
  {
    resource_id: "science.envisat.reserved.v1",
    group_id: "envisat",
    display_name_zh: "ENVISAT ASAR",
    family: "sar",
    search_mode: "sar",
    filter_schema_id: null,
    filter_schema_revision: null,
    result_cap: 200,
    accounts: ["esa"],
    network_default: "direct",
    availability: "unavailable",
    availability_reason_zh: "接入中",
  },
];

test("DataSourceCatalogModal 交互测试套件", async (suite) => {
  // 1. 弹窗打开并展示三栏布局
  await suite.test("1. 弹窗打开后展示三栏布局、左栏分类、中栏列表与右栏当前源详情", () => {
    const { getByRole, getByTestId, getByText, getByPlaceholderText } = render(
      <DataSourceCatalogModal
        isOpen={true}
        onClose={() => {}}
        availableSources={mockSources}
        activeResourceId="science.sentinel-1.standard.v1"
        onSelectSource={() => {}}
      />
    );

    const dialog = getByRole("dialog", { name: "选择数据源" });
    assert.ok(dialog);
    assert.ok(getByTestId("data-source-catalog-modal"));

    // 搜索输入框
    assert.ok(getByPlaceholderText("搜索名称、波段或关键词"));

    // 左栏分类导航
    assert.ok(getByTestId("catalog-nav-recent"));
    assert.ok(getByTestId("catalog-nav-primary-sar"));
    assert.ok(getByTestId("catalog-nav-primary-optical"));
    assert.ok(getByTestId("catalog-nav-primary-dem"));
    assert.ok(getByTestId("catalog-nav-auxiliary-orbit"));

    // 右栏详情展示当前源（Sentinel-1）
    assert.ok(getByText("C 波段雷达，不受云雨影响，常用于形变、洪涝和变化监测。"));
    assert.ok(getByText(/S1A/));
    assert.ok(getByText("使用此数据源"));
  });

  // 2. 左栏分类切换
  await suite.test("2. 点击左栏分类（光学、高程 DEM），中栏与右栏联动切换对应条目", () => {
    const { getByTestId, container } = render(
      <DataSourceCatalogModal
        isOpen={true}
        onClose={() => {}}
        availableSources={mockSources}
        activeResourceId="science.sentinel-1.standard.v1"
        onSelectSource={() => {}}
      />
    );

    // 点击光学分类导航
    fireEvent.click(getByTestId("catalog-nav-primary-optical"));

    // 中栏应展示 Sentinel-2 MSI 与 Landsat 8-9
    assert.ok(container.querySelector('[data-resource-id="science.sentinel-2.l2a.v1"]'));
    assert.ok(container.querySelector('[data-resource-id="science.landsat.c2-l2.v1"]'));

    // 点击高程 DEM 分类导航
    fireEvent.click(getByTestId("catalog-nav-primary-dem"));
    assert.ok(container.querySelector('[data-resource-id="dem.cop30.v1"]'));
    assert.strictEqual(container.querySelector('[data-resource-id="science.sentinel-2.l2a.v1"]'), null);
  });

  // 3. 关键词过滤
  await suite.test("3. 输入关键词搜索实时过滤中栏条目，清空后恢复", () => {
    const { getByPlaceholderText, container } = render(
      <DataSourceCatalogModal
        isOpen={true}
        onClose={() => {}}
        availableSources={mockSources}
        activeResourceId="science.sentinel-1.standard.v1"
        onSelectSource={() => {}}
      />
    );

    const input = getByPlaceholderText("搜索名称、波段或关键词");

    // 输入 "burst"
    fireEvent.change(input, { target: { value: "burst" } });
    assert.ok(container.querySelector('[data-resource-id="science.sentinel-1.slc-burst.v1"]'));
    assert.strictEqual(container.querySelector('[data-resource-id="science.sentinel-1.standard.v1"]'), null);

    // 清空搜索
    fireEvent.change(input, { target: { value: "" } });
    assert.ok(container.querySelector('[data-resource-id="science.sentinel-1.standard.v1"]'));
    assert.ok(container.querySelector('[data-resource-id="science.sentinel-1.slc-burst.v1"]'));
  });

  // 4. 快捷标签（波段与账号）筛选
  await suite.test("4. 快捷标签筛选（波段 C，免账号）准确缩小结果集", () => {
    const { getByTestId, container, getByRole } = render(
      <DataSourceCatalogModal
        isOpen={true}
        onClose={() => {}}
        availableSources={mockSources}
        activeResourceId="science.sentinel-1.standard.v1"
        onSelectSource={() => {}}
      />
    );

    // 切换到全部主数据
    fireEvent.click(getByTestId("catalog-nav-primary-all"));

    // 点击波段标签 "C"
    fireEvent.click(getByRole("button", { name: "C" }));
    assert.ok(container.querySelector('[data-resource-id="science.sentinel-1.standard.v1"]'));
    assert.strictEqual(container.querySelector('[data-resource-id="science.nisar.l.rslc.provisional.v1"]'), null);
  });

  // 5. 已下架数据源与本地 DEM 均已移除，在线裁剪标识与三档凭据状态（B9）动态生效
  await suite.test("5. 已下架数据源与本地 DEM 不再展示，在线裁剪角标与三档凭据状态动态联动", () => {
    let openedAccountSettings = 0;
    const { getByTestId, container, rerender } = render(
      <DataSourceCatalogModal
        isOpen={true}
        onClose={() => {}}
        availableSources={mockSources}
        activeResourceId="science.sentinel-1.standard.v1"
        onSelectSource={() => {}}
        onOpenAccountSettings={() => {
          openedAccountSettings += 1;
        }}
        accountStatuses={{ bhoonidhi: "logged_in", earthdata: "logged_in", gee: "credentials_missing" }}
      />
    );

    // 已下架数据集不得出现在清单中
    assert.strictEqual(container.querySelector('[data-resource-id="science.envisat.reserved.v1"]'), null);
    assert.strictEqual(container.querySelector('[data-resource-id="science.ers-1-2.sar.v1"]'), null);
    assert.strictEqual(container.querySelector('[data-resource-id="science.jers-1.sar.v1"]'), null);
    assert.strictEqual(container.querySelector('[data-resource-id="science.radarsat-1.sar.v1"]'), null);

    // 支持在线裁剪的数据源右上角应渲染剪刀标识，且「当前数据源」与「在线裁剪」角标保持黑白灰中性色（无蓝色高亮类）
    assert.ok(getByTestId("catalog-crop-badge-science.nisar.l.rslc.provisional.v1"));
    assert.ok(getByTestId("catalog-crop-badge-science.nisar.s.ssar.rslc.v1"));
    const currentBadge = getByTestId("catalog-current-source-badge-science.sentinel-1.standard.v1");
    assert.ok(currentBadge.className.includes("bg-sp-surface-muted"));
    assert.ok(!currentBadge.className.includes("sp-accent") && !currentBadge.className.includes("blue"));

    // (a2) Bhoonidhi 已登录时，NISAR S-band 显示「已配置可用」
    const sbandRow = container.querySelector('[data-resource-id="science.nisar.s.ssar.rslc.v1"]') as HTMLElement;
    assert.ok(sbandRow);
    assert.ok(within(sbandRow).getByText("已配置可用"));
    fireEvent.click(sbandRow);
    const detailCropBadge = getByTestId("detail-online-crop-badge");
    assert.ok(detailCropBadge.className.includes("bg-sp-surface-muted"));
    assert.ok(!detailCropBadge.className.includes("sp-accent") && !detailCropBadge.className.includes("blue"));

    // (a1) 免账号直连源（COMET-LiCSAR）显示「免账号可用」
    const licsarRow = container.querySelector('[data-resource-id="science.comet.licsar.v1"]') as HTMLElement;
    assert.ok(licsarRow);
    assert.ok(within(licsarRow).getByText("免账号可用"));

    // (c) Bhoonidhi 未填写凭据时，NISAR S-band 显示「待配置凭据」；CDSE 已保存（login_required）时 Sentinel-2 显示「已配置可用」
    rerender(
      <DataSourceCatalogModal
        isOpen={true}
        onClose={() => {}}
        availableSources={mockSources}
        activeResourceId="science.sentinel-1.standard.v1"
        onSelectSource={() => {}}
        onOpenAccountSettings={() => {
          openedAccountSettings += 1;
        }}
        accountStatuses={{ bhoonidhi: "credentials_missing", cdse: "login_required", earthdata: "logged_in", gee: "credentials_missing" }}
      />
    );
    const sbandRowMissing = container.querySelector('[data-resource-id="science.nisar.s.ssar.rslc.v1"]') as HTMLElement;
    assert.ok(within(sbandRowMissing).getByText("待配置凭据"));
    fireEvent.click(getByTestId("catalog-nav-primary-optical"));
    const s2RowReady = container.querySelector('[data-resource-id="science.sentinel-2.l2a.v1"]') as HTMLElement;
    assert.ok(s2RowReady);
    assert.ok(within(s2RowReady).getByText("已配置可用"));
    assert.strictEqual(within(s2RowReady).queryByText("待配置凭据"), null);

    // (b) 未配置 GEE Project ID 时，GEE 数据源显示「仅检索 · 下载需配项目」，且右栏提供直达配置按钮
    fireEvent.click(getByTestId("catalog-nav-gee"));
    const geeRow = container.querySelector('[data-resource-id="science.gee.catalog.v1"]') as HTMLElement;
    assert.ok(geeRow);
    assert.ok(within(geeRow).getByText("仅检索 · 下载需配项目"));
    fireEvent.click(geeRow);
    const jumpBtn = getByTestId("catalog-open-account-settings-btn");
    assert.ok(jumpBtn.textContent?.includes("前往配置 GEE 项目凭据"));
    fireEvent.click(jumpBtn);
    assert.equal(openedAccountSettings, 1);
    assert.ok(window.location.hash.includes("scroll=gee"));
  });

  // 6. 选择数据源与最近使用持久化
  await suite.test("6. 点击选择条目并确认，调用 onSelectSource、关闭弹窗并写入最近使用", () => {
    let selectedId: string | null = null;
    let closed = false;

    const { getByTestId, getByText, container } = render(
      <DataSourceCatalogModal
        isOpen={true}
        onClose={() => {
          closed = true;
        }}
        availableSources={mockSources}
        activeResourceId="science.sentinel-1.standard.v1"
        onSelectSource={(id) => {
          selectedId = id;
        }}
      />
    );

    // 切换到光学并选中 Sentinel-2 MSI
    fireEvent.click(getByTestId("catalog-nav-primary-optical"));
    const s2Row = container.querySelector('[data-resource-id="science.sentinel-2.l2a.v1"]') as HTMLElement;
    assert.ok(s2Row);
    fireEvent.click(s2Row);

    // 点击「使用此数据源」
    const useBtn = getByText("使用此数据源");
    fireEvent.click(useBtn);

    assert.strictEqual(selectedId, "science.sentinel-2.l2a.v1");
    assert.strictEqual(closed, true);

    // 验证写入最近使用存储
    const recents = getRecentSourceIds();
    assert.ok(recents.includes("science.sentinel-2.l2a.v1"));
  });

  // 7. 键盘交互（ArrowDown, ArrowUp, Enter, Escape）
  await suite.test("7. 键盘 ArrowDown / ArrowUp 切换高亮，Enter 确认，Escape 关闭", () => {
    let selectedId: string | null = null;
    let closed = false;

    render(
      <DataSourceCatalogModal
        isOpen={true}
        onClose={() => {
          closed = true;
        }}
        availableSources={mockSources}
        activeResourceId="science.sentinel-1.standard.v1"
        onSelectSource={(id) => {
          selectedId = id;
        }}
      />
    );

    // 按下 ArrowDown 移动到下一个可选条目
    fireEvent.keyDown(window, { key: "ArrowDown" });

    // 按下 Enter 确认
    fireEvent.keyDown(window, { key: "Enter" });
    assert.ok(selectedId !== null, "Enter 应确认选择当前高亮项");
    assert.strictEqual(closed, true);

    // 测试 Escape
    closed = false;
    fireEvent.keyDown(window, { key: "Escape" });
    assert.strictEqual(closed, true);
  });

  // 8. 工作台集成：数据源卡片展示、点击打开弹窗及 Ctrl+K 全局快捷键唤起
  await suite.test("8. ManualWorkbench 数据源卡片展示快捷键提示，点击或按下 Ctrl+K 唤起弹窗", () => {
    let selectedSource: string | null = null;

    const props: ManualWorkbenchProps = {
      scientificMode: "sar",
      onScientificModeChange: () => {},
      onResetParams: () => {},
      availableSources: mockSources,
      activeResourceId: "science.sentinel-1.standard.v1",
      onSelectSource: (id) => {
        selectedSource = id;
      },
      productType: "SLC",
      onProductTypeChange: () => {},
      workingAoi: null,
      workingGeometry: null,
      aoiLabel: "武汉市",
      onReplaceAoi: async () => {},
      onImportVectorSuccess: () => {},
      startDate: null,
      endDate: null,
      onStartDateChange: () => {},
      onEndDateChange: () => {},
      orbitDirection: "all",
      onOrbitDirectionChange: () => {},
      relativeOrbit: null,
      onRelativeOrbitChange: () => {},
      frame: null,
      onFrameChange: () => {},
      path: null,
      onPathChange: () => {},
      polarizations: [],
      onPolarizationsChange: () => {},
      onSearchStart: () => {},
      searching: false,
      searchError: null,
      searchProgress: null,
      onCancelSearch: () => {},
      onResetFilters: () => {},
      displayedBatch: null,
      batchRevision: 0,
      activeBatchItemCount: 0,
      checkedSceneIds: new Set(),
      onCheckScene: () => {},
      onCheckAllMatching: () => {},
      onClearCheckedScenes: () => {},
      onRowDownload: async () => {},
      onBatchDownload: async () => {},
      rowTaskStates: {},
      isTaskActive: () => false,
      onRetryRowTask: async () => {},
      onRowAction: () => {},
      taskErrorExplanation: () => null,
      onOpenTaskOutput: () => {},
      onSelectScene: () => {},
      selectedSceneId: null,
      onPageChange: () => {},
      pageIndex: 0,
      pageSize: 50,
      onPageSizeChange: () => {},
      scenes: [],
      resultTotalCount: 0,
      downloadBusy: false,
      drawMode: "rect",
      drawActive: false,
      onDrawModeChange: () => {},
      onDrawActiveChange: () => {},
      onClearLayers: () => {},
      onRectDraw: () => {},
      onPolygonDraw: () => {},
      onPointDraw: () => {},
      mapLayerKey: "arcgisSatellite",
      onLayerChange: () => {},
      onToggleCheckScene: () => {},
      onToggleCheckAll: () => {},
      onDownloadChecked: () => {},
    };

    const { getByTestId, queryByTestId, getByText } = render(<ManualWorkbench {...props} />);

    // 数据源卡片存在且包含 Ctrl+K
    const card = getByTestId("data-source-card");
    assert.ok(card);
    assert.ok(getByText("Ctrl+K"));

    // 默认弹窗未打开
    assert.strictEqual(queryByTestId("data-source-catalog-modal"), null);

    // 点击卡片打开弹窗
    fireEvent.click(card);
    assert.ok(getByTestId("data-source-catalog-modal"));

    // 按 Escape 关闭
    fireEvent.keyDown(window, { key: "Escape" });
    assert.strictEqual(queryByTestId("data-source-catalog-modal"), null);

    // 按 Ctrl+K 全局快捷键打开
    fireEvent.keyDown(window, { key: "k", ctrlKey: true });
    assert.ok(getByTestId("data-source-catalog-modal"));
  });

  // 9. 提升后的 GACOS/精密轨道在「全部主数据」中可选，且 GEE 公共数据集双向同步至工作台
  await suite.test("9. 精密轨道/GACOS 在全部主数据中可选，GEE 数据集选用联动 workbenchToolOptions", () => {
    const selectedIds: string[] = [];
    const view = render(
      <DataSourceCatalogModal
        isOpen={true}
        onClose={() => {}}
        availableSources={mockSources}
        activeResourceId="science.sentinel-1.standard.v1"
        onSelectSource={(id) => selectedIds.push(id)}
      />
    );

    // 切换到全部主数据，应能直接找到精密轨道和 GACOS
    fireEvent.click(view.getByTestId("catalog-nav-primary-all"));
    const orbitRow = view.container.querySelector('[data-resource-id="orbit.sentinel-1.poeorb.v1"]') as HTMLElement;
    const gacosRow = view.container.querySelector('[data-resource-id="atmospheric.gacos.ztd.v1"]') as HTMLElement;
    assert.ok(orbitRow, "全部主数据分类应展示 Sentinel-1 精密轨道");
    assert.ok(gacosRow, "全部主数据分类应展示 GACOS 大气延迟改正");

    fireEvent.click(gacosRow);
    fireEvent.click(view.getByTestId("catalog-confirm-use-btn"));
    assert.strictEqual(selectedIds[0], "atmospheric.gacos.ztd.v1");

    // 切换到 GEE 目录分类并选用特定数据集（如 COPERNICUS/DEM/GLO30），同步至 workbenchToolOptions
    fireEvent.click(view.getByTestId("catalog-nav-gee"));
    const demEntry = view.getByTestId("gee-public-entry-COPERNICUS_DEM_GLO30");
    fireEvent.click(demEntry);
    assert.strictEqual(getWorkbenchToolOptions().geeAssetId, "COPERNICUS/DEM/GLO30");
    assert.strictEqual(selectedIds[1], "science.gee.catalog.v1");

    // 再次打开弹窗，该数据集按钮应显示为「已选用」
    view.rerender(
      <DataSourceCatalogModal
        isOpen={true}
        onClose={() => {}}
        availableSources={mockSources}
        activeResourceId="science.gee.catalog.v1"
        onSelectSource={(id) => selectedIds.push(id)}
      />
    );
    fireEvent.click(view.getByTestId("catalog-nav-gee"));
    assert.ok(within(view.getByTestId("gee-public-entry-COPERNICUS_DEM_GLO30")).getByText("已选用"));
  });

  await suite.test("10. 分类审计：精密轨道仅含三大星历、形变与事件/专题与瓦片独立分类且无 OlmoEarth", () => {
    const view = render(
      <DataSourceCatalogModal
        isOpen={true}
        onClose={() => {}}
        availableSources={mockSources}
        activeResourceId="science.sentinel-1.standard.v1"
        onSelectSource={() => {}}
      />
    );

    assert.strictEqual(view.container.textContent?.includes("OlmoEarth"), false);

    fireEvent.click(view.getByTestId("catalog-nav-auxiliary-orbit"));
    assert.ok(view.container.querySelector('[data-resource-id="orbit.sentinel-1.poeorb.v1"]'));
    assert.ok(view.container.querySelector('[data-resource-id="orbit.nisar.oe.v1"]'));
    assert.ok(view.container.querySelector('[data-resource-id="orbit.copernicus.pod.v1"]'));
    assert.strictEqual(view.container.querySelector('[data-resource-id="event.usgs.earthquake.v1"]'), null);
    assert.strictEqual(view.container.querySelector('[data-resource-id="geodesy.ngl.gnss.v1"]'), null);

    fireEvent.click(view.getByTestId("catalog-nav-auxiliary-geodesy"));
    assert.ok(view.container.querySelector('[data-resource-id="event.usgs.earthquake.v1"]'));
    assert.ok(view.container.querySelector('[data-resource-id="geodesy.ngl.gnss.v1"]'));

    fireEvent.click(view.getByTestId("catalog-nav-primary-thematic"));
    assert.ok(view.container.querySelector('[data-resource-id="auxiliary.xyz-tiles.v1"]'));
    assert.ok(view.container.querySelector('[data-resource-id="science.esa.worldcover.v1"]'));
    assert.ok(view.container.querySelector('[data-resource-id="science.jrc.gsw.v1"]'));
    assert.ok(view.container.querySelector('[data-resource-id="science.modis.ndvi.v1"]'));
    assert.ok(view.container.querySelector('[data-resource-id="atmospheric.sentinel-5p.l2.v1"]'));
  });

  // 11. 数据缺失与许可证信息仅收敛展示在右栏底部注意事项中，顶部介绍不重复堆砌
  await suite.test("11. 数据缺失与许可证信息仅收敛展示在右栏底部注意事项中，顶部介绍不重复堆砌", () => {
    const { getByTestId, queryByTestId, container } = render(
      <DataSourceCatalogModal
        isOpen={true}
        onClose={() => {}}
        availableSources={mockSources}
        activeResourceId="science.sentinel-1.standard.v1"
        onSelectSource={() => {}}
      />
    );

    const expectedGapKeywords = [
      "中国大陆",
      "俄罗斯",
      "蒙古",
      "朝鲜",
      "格陵兰",
      "南极洲",
      "公海",
      "高纬部分海域",
    ];

    fireEvent.click(getByTestId("catalog-nav-primary-dem"));
    for (const id of ["dem.gem15-dtm.v1", "dem.gem15-dsm.v1"]) {
      const row = container.querySelector(`[data-resource-id="${id}"]`) as HTMLElement;
      assert.ok(row, `${id} should appear under elevation DEM category`);
      fireEvent.click(row);

      const notesBox = getByTestId("catalog-detail-usage-notes");
      assert.ok(notesBox.textContent?.includes("注意事项"), `${id} 必须渲染注意事项区块`);
      for (const kw of expectedGapKeywords) {
        assert.ok(notesBox.textContent?.includes(kw), `${id} 注意事项应包含「${kw}」`);
      }
      assert.ok(notesBox.textContent?.includes("CC BY 4.0"), `${id} 注意事项应包含 CC BY 4.0 许可说明`);

      const rightPane = notesBox.closest("div.w-\\[280px\\]");
      const introParagraph = rightPane?.querySelector("p")?.textContent ?? "";
      assert.ok(!introParagraph.includes("不包含"), `${id} 顶部介绍不应重复堆砌缺口清单`);
      assert.ok(!introParagraph.includes("CC BY"), `${id} 顶部介绍不应重复堆砌许可证`);
    }

    const fabdemRow = container.querySelector('[data-resource-id="dem.fabdem.v1"]') as HTMLElement;
    assert.ok(fabdemRow);
    fireEvent.click(fabdemRow);
    const fabdemNotes = getByTestId("catalog-detail-usage-notes");
    assert.ok(fabdemNotes.textContent?.includes("CC BY-NC-SA 4.0"), "FABDEM 注意事项应包含非商用许可限制");
    const fabdemIntro = fabdemNotes.closest("div.w-\\[280px\\]")?.querySelector("p")?.textContent ?? "";
    assert.ok(!fabdemIntro.includes("CC BY-NC-SA"), "FABDEM 顶部介绍不应重复堆砌许可证");

    const cop30Row = container.querySelector('[data-resource-id="dem.cop30.v1"]') as HTMLElement;
    assert.ok(cop30Row);
    fireEvent.click(cop30Row);
    assert.strictEqual(queryByTestId("catalog-detail-usage-notes"), null);

    fireEvent.click(getByTestId("catalog-nav-gee"));
    const geeGem15Row = container.querySelector('[data-resource-id="science.gee.gem-15.v1"]') as HTMLElement;
    assert.ok(geeGem15Row, "science.gee.gem-15.v1 should appear under GEE category");
    fireEvent.click(geeGem15Row);
    const geeNotesBox = getByTestId("catalog-detail-usage-notes");
    for (const kw of expectedGapKeywords) {
      assert.ok(geeNotesBox.textContent?.includes(kw), `science.gee.gem-15.v1 注意事项应包含「${kw}」`);
    }
    assert.ok(geeNotesBox.textContent?.includes("CC BY 4.0"), "science.gee.gem-15.v1 注意事项应包含 CC BY 4.0 许可说明");
    const geeIntro = geeNotesBox.closest("div.w-\\[280px\\]")?.querySelector("p")?.textContent ?? "";
    assert.ok(!geeIntro.includes("不包含"), "science.gee.gem-15.v1 顶部介绍不应重复堆砌缺口清单");
    assert.ok(!geeIntro.includes("CC BY"), "science.gee.gem-15.v1 顶部介绍不应重复堆砌许可证");
  });

  await suite.test("12. Item 9: 无暗色蒙版 (bg-transparent)、点击外部不误关、搜索框聚焦时 Esc 先清空再失焦", () => {
    let closeCount = 0;
    const { container, getByPlaceholderText } = render(
      <DataSourceCatalogModal
        isOpen={true}
        onClose={() => {
          closeCount += 1;
        }}
        availableSources={mockSources}
        activeResourceId="science.sentinel-1.standard.v1"
        onSelectSource={() => {}}
      />
    );
    const overlay = container.querySelector("[data-overlay-layer='base']") as HTMLElement;
    assert.ok(overlay, "overlay must exist");
    assert.ok(overlay.className.includes("bg-transparent"), "no dim mask on data catalog");
    assert.equal(overlay.className.includes("bg-black/45"), false);

    fireEvent.click(overlay);
    assert.equal(closeCount, 0, "clicking outside catalog must not dismiss");

    const searchInput = getByPlaceholderText(/搜索|检索/) as HTMLInputElement;
    searchInput.focus();
    fireEvent.change(searchInput, { target: { value: "Landsat" } });
    assert.equal(searchInput.value, "Landsat");

    // First Escape clears non-empty input instead of closing
    fireEvent.keyDown(document, { key: "Escape" });
    assert.equal(closeCount, 0);
    assert.equal(searchInput.value, "");

    // Second Escape while focused but empty blurs input instead of closing
    fireEvent.keyDown(document, { key: "Escape" });
    assert.equal(closeCount, 0);
    assert.notEqual(document.activeElement, searchInput);

    // Controlled GEE custom Asset ID input also clears React state cleanly on Escape
    fireEvent.click(container.querySelector("[data-testid='catalog-nav-gee']") as HTMLElement);
    const geeInput = container.querySelector("[data-testid='gee-custom-asset-id-input']") as HTMLInputElement;
    geeInput.focus();
    fireEvent.change(geeInput, { target: { value: "COPERNICUS/S2" } });
    assert.equal(geeInput.value, "COPERNICUS/S2");
    fireEvent.keyDown(document, { key: "Escape" });
    assert.equal(closeCount, 0);
    assert.equal(geeInput.value, "");
    fireEvent.keyDown(document, { key: "Escape" });
    assert.equal(closeCount, 0);
    assert.notEqual(document.activeElement, geeInput);

    // Final Escape with no input focused closes catalog
    fireEvent.keyDown(document, { key: "Escape" });
    assert.equal(closeCount, 1);
  });

  await suite.test("13. Issue #53: 从雷达 SAR 分类进入时直接搜索 Landsat 应跨分类命中，空结果支持一键重置", () => {
    let selectedId: string | null = null;
    const { getByPlaceholderText, getByTestId, container } = render(
      <DataSourceCatalogModal
        isOpen={true}
        onClose={() => {}}
        availableSources={mockSources}
        activeResourceId="science.sentinel-1.standard.v1"
        onSelectSource={(id) => {
          selectedId = id;
        }}
      />
    );

    const searchInput = getByPlaceholderText(/搜索名称|Search|検索/) as HTMLInputElement;
    fireEvent.change(searchInput, { target: { value: "Landsat" } });
    assert.ok(getByTestId("catalog-cross-category-banner"));
    const landsatRow = container.querySelector('[data-resource-id="science.landsat.c2-l2.v1"]') as HTMLElement;
    assert.ok(landsatRow, "Searching Landsat from primary-sar should return Landsat across categories");
    fireEvent.click(landsatRow);
    fireEvent.click(getByTestId("catalog-confirm-use-btn"));
    assert.equal(selectedId, "science.landsat.c2-l2.v1");

    // Non-matching keyword shows actionable reset button and restores primary-all
    fireEvent.change(searchInput, { target: { value: "nonexistent-satellite-xyz" } });
    const resetBtn = getByTestId("catalog-reset-search-btn");
    fireEvent.click(resetBtn);
    assert.equal(searchInput.value, "");
    assert.ok(container.querySelector('[data-resource-id="science.sentinel-1.standard.v1"]'));
    assert.ok(container.querySelector('[data-resource-id="science.landsat.c2-l2.v1"]'));
  });
});
