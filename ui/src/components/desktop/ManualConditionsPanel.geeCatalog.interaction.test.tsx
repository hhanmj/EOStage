import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { act } from "@testing-library/react";
import { cleanup, fireEvent, render } from "../../test-utils/render-harness";
import {
  ManualConditionsPanel,
  type ManualConditionsPanelProps,
  getWorkbenchToolOptions,
  inspectGacosWorkbenchInputs,
  isDerivedSarInterferogram,
  updateWorkbenchToolOptions,
} from "./ManualConditionsPanel";
import { classifyGeeDatasetKind } from "./GeeCatalogAutocomplete";
import { DataSourceCatalogModal } from "./DataSourceCatalogModal";
import { normalizeGeeCatalogItem } from "../../lib/geeCloudSuiteClient";
import { parseGacosManualDatesList } from "../../lib/companionGacosFlow";
import { sarFilterProfile, cleanStaleFilterValues } from "../../shell/api/sarFilterProfile";
import type { SourceDescriptor } from "../../shell/api/types";
import type { Bbox } from "../../lib/bridge";

const bbox: Bbox = { west: 117.2, south: 31.8, east: 117.3, north: 31.9, crs: "EPSG:4326" };
const geometry = {
  type: "Polygon",
  coordinates: [[[117.2, 31.8], [117.3, 31.8], [117.3, 31.9], [117.2, 31.9], [117.2, 31.8]]],
};

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
    resource_id: "science.nisar.l.rslc.provisional.v1",
    group_id: "nisar",
    display_name_zh: "NISAR L 波段 SAR",
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
    resource_id: "science.aria.s1-gunw.v1",
    group_id: "aria",
    display_name_zh: "NASA ARIA S1-GUNW",
    family: "sar",
    search_mode: "open_geodata",
    filter_schema_id: null,
    filter_schema_revision: null,
    result_cap: 500,
    accounts: ["asf.earthdata"],
    network_default: "direct",
    availability: "available",
    availability_reason_zh: null,
  },
  {
    resource_id: "science.opera.disp-s1.v1",
    group_id: "opera",
    display_name_zh: "NASA OPERA DISP-S1",
    family: "sar",
    search_mode: "open_geodata",
    filter_schema_id: null,
    filter_schema_revision: null,
    result_cap: 500,
    accounts: ["asf.earthdata"],
    network_default: "direct",
    availability: "available",
    availability_reason_zh: null,
  },
  {
    resource_id: "science.comet.licsar.v1",
    group_id: "comet",
    display_name_zh: "COMET-LiCSAR",
    family: "sar",
    search_mode: "open_geodata",
    filter_schema_id: null,
    filter_schema_revision: null,
    result_cap: 500,
    accounts: [],
    network_default: "direct",
    availability: "available",
    availability_reason_zh: null,
  },
  {
    resource_id: "orbit.sentinel-1.poeorb.v1",
    group_id: "orbit",
    display_name_zh: "Sentinel-1 精密轨道 (POEORB/RESORB)",
    family: "auxiliary",
    search_mode: "orbit",
    filter_schema_id: null,
    filter_schema_revision: null,
    result_cap: 500,
    accounts: [],
    network_default: "direct",
    availability: "available",
    availability_reason_zh: null,
  },
  {
    resource_id: "atmospheric.gacos.ztd.v1",
    group_id: "gacos",
    display_name_zh: "GACOS 大气延迟改正",
    family: "auxiliary",
    search_mode: "gacos",
    filter_schema_id: null,
    filter_schema_revision: null,
    result_cap: 500,
    accounts: [],
    network_default: "direct",
    availability: "available",
    availability_reason_zh: null,
  },
];

function createBaseProps(overrides: Partial<ManualConditionsPanelProps> = {}): ManualConditionsPanelProps {
  return {
    scientificMode: "sar",
    onScientificModeChange: () => {},
    onResetParams: () => {},
    availableSources: mockSources,
    activeResourceId: "science.sentinel-1.standard.v1",
    onSelectSource: () => {},
    productType: "SLC",
    onProductTypeChange: () => {},
    workingAoi: bbox,
    workingGeometry: geometry,
    aoiLabel: "合肥市",
    onReplaceAoi: async () => {},
    onPreviewAdminGeometry: () => {},
    onImportVectorSuccess: () => {},
    startDate: "2025-04-01",
    endDate: "2025-06-30",
    onStartDateChange: () => {},
    onEndDateChange: () => {},
    orbitDirection: "all",
    onOrbitDirectionChange: () => {},
    relativeOrbit: null,
    onRelativeOrbitChange: () => {},
    frame: null,
    onFrameChange: () => {},
    searching: false,
    searchError: null,
    onTriggerSearch: () => {},
    ...overrides,
  };
}

test.afterEach(() => {
  cleanup();
  updateWorkbenchToolOptions({
    convertOutputMode: "sarscape",
    orbitActiveSubMode: "current",
    gacosUtcTime: "all",
    geeAssetId: "ALL",
    geeSelectedPresetId: "science.gee.catalog.v1",
  });
});

test("1. Derived SAR interferograms (ARIA / OPERA / LiCSAR) display '雷达 SAR' header badge and suppress raw-SLC POEORB wait warnings", () => {
  assert.strictEqual(isDerivedSarInterferogram("science.aria.s1-gunw.v1"), true);
  assert.strictEqual(isDerivedSarInterferogram("science.opera.disp-s1.v1"), true);
  assert.strictEqual(isDerivedSarInterferogram("science.comet.licsar.v1"), true);
  assert.strictEqual(isDerivedSarInterferogram("science.sentinel-1.standard.v1"), false);

  for (const id of [
    "science.aria.s1-gunw.v1",
    "science.opera.disp-s1.v1",
    "science.comet.licsar.v1",
  ]) {
    // Even if descriptor is marked optical or missing, header MUST remain 雷达 SAR
    const pollutedSources = [
      {
        resource_id: id,
        group_id: "derived",
        display_name_zh: id,
        family: "optical",
        search_mode: "open_geodata",
        filter_schema_id: null,
        filter_schema_revision: null,
        result_cap: 100,
        accounts: [],
        network_default: "direct",
        availability: "available",
        availability_reason_zh: null,
      } as SourceDescriptor,
    ];
    const view = render(
      <ManualConditionsPanel
        {...createBaseProps({
          availableSources: pollutedSources,
          activeResourceId: id,
        })}
      />
    );
    assert.strictEqual(view.getByTestId("conditions-header-category-badge").textContent, "雷达 SAR");
    assert.notStrictEqual(view.getByTestId("conditions-header-category-badge").textContent, "光学");
    assert.ok(view.getByTestId("derived-sar-product-note"));
    assert.strictEqual(view.container.textContent?.includes("POEORB 约在采集后 20 天发布"), false);
    view.unmount();
  }

  // Raw NISAR keeps concise SAR badge
  const nisarView = render(
    <ManualConditionsPanel
      {...createBaseProps({ activeResourceId: "science.nisar.l.rslc.provisional.v1" })}
    />
  );
  assert.strictEqual(nisarView.getByTestId("conditions-header-category-badge").textContent, "SAR");
});

test("2. Top mode switcher renames 影像检索 -> 数据检索 and separates Processing & Cloud Tools cleanly", () => {
  const modeCalls: string[] = [];
  const view = render(
    <ManualConditionsPanel
      {...createBaseProps({
        scientificMode: "sar",
        onScientificModeChange: (m) => modeCalls.push(m),
      })}
    />
  );

  const selectTrigger = view.getByTestId("scientific-mode-select");
  assert.ok(selectTrigger.textContent?.includes("数据检索"));
  assert.strictEqual(selectTrigger.textContent?.includes("影像检索"), false);

  act(() => {
    fireEvent.click(selectTrigger);
  });

  // Assert reorganized peer options (Owner 2026-11-17 /grill-me #6: single merged DEM 转换 option)
  assert.ok(view.getByRole("option", { name: "数据检索" }));
  assert.ok(view.getByRole("option", { name: "DEM 转换" }));
  assert.strictEqual(view.queryByRole("option", { name: "DEM 转 SARscape 格式" }), null);
  assert.strictEqual(view.queryByRole("option", { name: "DEM 转椭球高（高程基准转换）" }), null);
  assert.ok(view.getByRole("option", { name: "GEE 云端计算与制图" }));

  // Switch to DEM 转换
  act(() => {
    fireEvent.click(view.getByRole("option", { name: "DEM 转换" }));
  });
  assert.deepStrictEqual(modeCalls, ["convert"]);

  // Switch to GEE 云端计算与制图
  act(() => {
    fireEvent.click(selectTrigger);
  });
  act(() => {
    fireEvent.click(view.getByRole("option", { name: "GEE 云端计算与制图" }));
  });
  assert.deepStrictEqual(modeCalls, ["convert", "gee"]);
});

test("3. Promoted 精密轨道 & GACOS sources expose standalone active retrieval + local folder matching controls", () => {
  let orbitMode: "current" | "local" = "current";
  const orbitView = render(
    <ManualConditionsPanel
      {...createBaseProps({
        activeResourceId: "orbit.sentinel-1.poeorb.v1",
        orbitSourceType: orbitMode,
        onOrbitSourceTypeChange: (m) => {
          orbitMode = m;
        },
        localSlcPath: "",
      })}
    />
  );

  assert.strictEqual(orbitView.getByTestId("conditions-header-category-badge").textContent, "轨道");
  assert.ok(orbitView.getByTestId("active-orbit-panel"));
  assert.ok(orbitView.getByTestId("orbit-source-mode-current"));
  assert.ok(orbitView.getByTestId("orbit-source-mode-local"));

  // Switch to local SAR folder orbit matching
  act(() => {
    fireEvent.click(orbitView.getByTestId("orbit-source-mode-local"));
  });
  assert.strictEqual(orbitMode, "local");
  orbitView.rerender(
    <ManualConditionsPanel
      {...createBaseProps({
        activeResourceId: "orbit.sentinel-1.poeorb.v1",
        orbitSourceType: "local",
        localSlcPath: "",
      })}
    />
  );
  assert.ok(orbitView.getByTestId("orbit-local-slc-browse-btn"));
  assert.strictEqual(
    (orbitView.getByTestId("btn-start-search") as HTMLButtonElement).disabled,
    true,
    "本地文件夹路径未选时检索/匹配按钮禁用"
  );
  orbitView.unmount();

  const gacosView = render(
    <ManualConditionsPanel
      {...createBaseProps({ activeResourceId: "atmospheric.gacos.ztd.v1" })}
    />
  );
  assert.strictEqual(gacosView.getByTestId("conditions-header-category-badge").textContent, "大气");
  assert.ok(gacosView.getByTestId("gacos-active-panel"));
  assert.ok(gacosView.getByTestId("gacos-aoi-span-check"));
  assert.ok(gacosView.getByTestId("gacos-dates-textarea"));
  assert.ok(gacosView.getByTestId("gacos-utc-hour"));
  assert.ok(gacosView.getByTestId("gacos-utc-minute"));
  assert.ok(gacosView.getByTestId("gacos-format-geotiff"));
  assert.ok(gacosView.getByTestId("gacos-format-binary"));
  assert.ok(gacosView.getByTestId("gacos-aoi-coords-grid"));
  assert.ok(gacosView.getByTestId("gacos-slc-dir-input"));
  assert.ok(gacosView.getByTestId("gacos-scan-slc-dir-btn"));
  assert.strictEqual(gacosView.container.querySelectorAll("select:not(.sr-only)").length, 0);
  assert.strictEqual(gacosView.container.querySelector('[data-condition-group="time"]'), null);
  act(() => {
    fireEvent.click(gacosView.getByTestId("gacos-format-binary"));
    fireEvent.click(gacosView.getByTestId("gacos-utc-hour"));
  });
  act(() => {
    fireEvent.click(gacosView.getByRole("option", { name: "06 时" }));
    fireEvent.click(gacosView.getByTestId("gacos-utc-minute"));
  });
  act(() => {
    fireEvent.click(gacosView.getByRole("option", { name: "35 分" }));
    fireEvent.change(gacosView.getByTestId("gacos-dates-textarea"), {
      target: { value: "20260501\n20260513" },
    });
  });
  assert.strictEqual(getWorkbenchToolOptions().gacosOutputFormat, "binary");
  assert.strictEqual(getWorkbenchToolOptions().gacosUtcHour, "6");
  assert.strictEqual(getWorkbenchToolOptions().gacosUtcMinute, "35");
  assert.strictEqual(getWorkbenchToolOptions().gacosDatesText, "20260501\n20260513");
});

test("4. NISAR L/S-band exposes all 9 official product levels, keeps S-band non-RSLC selectable with pending notice, and cleanStaleFilterValues preserves level", () => {
  const levels: string[] = [];
  const lView = render(
    <ManualConditionsPanel
      {...createBaseProps({
        activeResourceId: "science.nisar.l.rslc.provisional.v1",
        productType: "RSLC",
        onProductTypeChange: (lvl) => levels.push(lvl),
      })}
    />
  );

  const selector = lView.getByTestId("nisar-product-level-selector");
  for (const code of ["RSLC", "GSLC", "GCOV", "GUNW", "GOFF", "RIFG", "RUNW", "ROFF", "SME2"]) {
    assert.ok(
      selector.querySelector(`[data-testid="nisar-level-pill-${code}"]`),
      `NISAR level ${code} must be rendered`
    );
  }
  act(() => {
    fireEvent.click(lView.getByTestId("nisar-level-pill-GCOV"));
  });
  assert.deepStrictEqual(levels, ["GCOV"]);
  assert.strictEqual(getWorkbenchToolOptions().nisarProductLevel, "GCOV");
  lView.rerender(
    <ManualConditionsPanel
      {...createBaseProps({
        activeResourceId: "science.nisar.l.rslc.provisional.v1",
        productType: "GCOV",
        onProductTypeChange: (lvl) => levels.push(lvl),
      })}
    />
  );
  assert.ok(lView.getByTestId("nisar-lband-level-route-note"));
  lView.unmount();

  const sBandDesc: SourceDescriptor = {
    resource_id: "science.nisar.s.ssar.rslc.v1",
    group_id: "nisar",
    display_name_zh: "NISAR S 波段",
    family: "sar",
    search_mode: "sar",
    filter_schema_id: null,
    filter_schema_revision: null,
    result_cap: 1000,
    accounts: ["bhoonidhi"],
    network_default: "direct",
    availability: "available",
    availability_reason_zh: null,
  };
  const sView = render(
    <ManualConditionsPanel
      {...createBaseProps({
        availableSources: [...mockSources, sBandDesc],
        activeResourceId: "science.nisar.s.ssar.rslc.v1",
        productType: "GUNW",
        onProductTypeChange: (lvl) => levels.push(lvl),
      })}
    />
  );
  const gunwBtn = sView.getByTestId("nisar-level-pill-GUNW") as HTMLButtonElement;
  assert.strictEqual(gunwBtn.disabled, false);
  assert.ok(sView.getByTestId("nisar-sband-level-route-note"));
  sView.unmount();

  const sSme2View = render(
    <ManualConditionsPanel
      {...createBaseProps({
        availableSources: [...mockSources, sBandDesc],
        activeResourceId: "science.nisar.s.ssar.rslc.v1",
        productType: "SME2",
        onProductTypeChange: (lvl) => levels.push(lvl),
      })}
    />
  );
  act(() => {
    fireEvent.click(sSme2View.getByTestId("nisar-level-pill-SME2"));
  });
  assert.ok(sSme2View.getByTestId("nisar-sband-level-pending-note"));
  sSme2View.unmount();

  const s1Profile = sarFilterProfile("science.sentinel-1.standard.v1");
  const nisarProfile = sarFilterProfile("science.nisar.l.rslc.provisional.v1");
  const preserved = cleanStaleFilterValues(s1Profile, nisarProfile, { productType: "GCOV" });
  assert.strictEqual(preserved.productType, "GCOV");
  const clearedSlc = cleanStaleFilterValues(s1Profile, nisarProfile, { productType: "SLC" });
  assert.strictEqual(clearedSlc.productType, null);
});

test("5. DEM parameters header and channel use whitespace-nowrap to prevent orphan character wraps at narrow width", () => {
  const demView = render(
    <ManualConditionsPanel
      {...createBaseProps({
        scientificMode: "dem",
        demSource: "COP30",
      })}
    />
  );
  const titleEl = demView.getByText("DEM 参数配置");
  const channelEl = demView.getByText("AWS 公开桶 · 免账号直连");
  assert.ok(titleEl.className.includes("whitespace-nowrap"));
  assert.ok(channelEl.className.includes("whitespace-nowrap"));
  assert.ok(demView.getByTestId("data-source-card"));
  assert.ok(demView.queryByText(/93 天/) === null);
  demView.unmount();
});

test("6. GEE static catalog assets hide Time/Cloud filters + clarify compute jump, and GACOS validates official span/date/batch rules", () => {
  const geeDesc: SourceDescriptor = {
    resource_id: "science.gee.catalog.v1",
    group_id: "gee",
    display_name_zh: "Google Earth Engine (GEE) 全球公共数据目录",
    family: "optical",
    search_mode: "optical",
    filter_schema_id: null,
    filter_schema_revision: null,
    result_cap: 1000,
    accounts: ["google_earth_engine"],
    network_default: "proxy",
    availability: "available",
    availability_reason_zh: null,
  };
  updateWorkbenchToolOptions({ geeAssetId: "COPERNICUS/DEM/GLO30" });
  const geeView = render(
    <ManualConditionsPanel
      {...createBaseProps({
        scientificMode: "sar",
        activeResourceId: "science.gee.catalog.v1",
        availableSources: [...mockSources, geeDesc],
      })}
    />
  );
  assert.strictEqual(geeView.container.querySelector('[data-condition-group="time"]'), null);
  assert.strictEqual(geeView.container.querySelector("#group-quality-body"), null);
  assert.strictEqual(
    geeView.getByTestId("gee-enter-workbench-btn").textContent?.trim(),
    "前往 GEE 云端计算与制图 →"
  );
  geeView.unmount();

  const tooLargeAudit = inspectGacosWorkbenchInputs(
    { west: 100, south: 20, east: 112.5, north: 35, crs: "EPSG:4326" },
    "20260501",
    "10",
    "15"
  );
  assert.strictEqual(tooLargeAudit.spanStatus, "too_large");

  const tooSmallAudit = inspectGacosWorkbenchInputs(
    { west: 117.1, south: 31.8, east: 117.12, north: 31.81, crs: "EPSG:4326" },
    "20260501",
    "10",
    "15"
  );
  assert.strictEqual(tooSmallAudit.spanStatus, "too_small");

  const parsedDates = parseGacosManualDatesList("20260501\n20260513\n2026-05-25\n20260230\nabc123");
  assert.deepStrictEqual(parsedDates.validDates, ["20260501", "20260513"]);
  assert.deepStrictEqual(parsedDates.invalidTokens, ["2026-05-25", "20260230", "abc123"]);

  const dates21 = Array.from({ length: 21 }, (_, idx) =>
    `202605${String(idx + 1).padStart(2, "0")}`
  ).join("\n");
  const batchAudit = inspectGacosWorkbenchInputs(
    { west: 117, south: 31, east: 118, north: 32, crs: "EPSG:4326" },
    dates21,
    "6",
    "42"
  );
  assert.strictEqual(batchAudit.spanStatus, "ok");
  assert.strictEqual(batchAudit.validDates.length, 21);
  assert.strictEqual(batchAudit.batchCount, 2);
  assert.strictEqual(batchAudit.utcHourNum, 6);
  assert.strictEqual(batchAudit.utcMinuteNum, 42);
});

test("7. Multi-mission global orbits (NISAR OE / CDSE POD) hide AOI box + stay searchable, and geodesy/thematic header badges match taxonomy", () => {
  const orbitNisarView = render(
    <ManualConditionsPanel
      {...createBaseProps({
        activeResourceId: "orbit.nisar.oe.v1",
        workingAoi: null,
        workingGeometry: null,
      })}
    />
  );
  assert.strictEqual(orbitNisarView.getByTestId("conditions-header-category-badge").textContent, "轨道");
  assert.ok(orbitNisarView.getByTestId("nisar-orbit-oe-panel"));
  assert.strictEqual(orbitNisarView.container.querySelector('[data-condition-group="aoi"]'), null);
  assert.ok(orbitNisarView.getByTestId("btn-start-search").textContent?.includes("检索并下载精密轨道"));
  assert.strictEqual((orbitNisarView.getByTestId("btn-start-search") as HTMLButtonElement).disabled, false);
  orbitNisarView.unmount();

  const orbitPodView = render(
    <ManualConditionsPanel
      {...createBaseProps({
        activeResourceId: "orbit.copernicus.pod.v1",
        workingAoi: null,
        workingGeometry: null,
      })}
    />
  );
  assert.strictEqual(orbitPodView.getByTestId("conditions-header-category-badge").textContent, "轨道");
  assert.ok(orbitPodView.getByTestId("copernicus-pod-orbit-panel"));
  assert.strictEqual(orbitPodView.container.querySelector('[data-condition-group="aoi"]'), null);
  orbitPodView.unmount();

  const eqView = render(
    <ManualConditionsPanel
      {...createBaseProps({ activeResourceId: "event.usgs.earthquake.v1" })}
    />
  );
  assert.strictEqual(eqView.getByTestId("conditions-header-category-badge").textContent, "形变/事件");
  eqView.unmount();

  const wcView = render(
    <ManualConditionsPanel
      {...createBaseProps({ activeResourceId: "science.esa.worldcover.v1" })}
    />
  );
  assert.strictEqual(wcView.getByTestId("conditions-header-category-badge").textContent, "专题/瓦片");
  wcView.unmount();
});

test("8. [Owner Round-5 Item 7] GEE catalog switches across all 4 dynamic filter templates without native select", () => {
  const cases: Array<{ assetId: string; expectedTemplate: string; expectDateBox: boolean }> = [
    { assetId: "COPERNICUS/S2_SR_HARMONIZED", expectedTemplate: "optical", expectDateBox: true },
    { assetId: "COPERNICUS/S1_GRD", expectedTemplate: "sar", expectDateBox: true },
    { assetId: "ECMWF/ERA5_LAND/HOURLY", expectedTemplate: "climate_env", expectDateBox: true },
    { assetId: "projects/climate-and-sustainability/assets/gem-15_v1", expectedTemplate: "static", expectDateBox: false },
  ];

  for (const tc of cases) {
    updateWorkbenchToolOptions({ geeAssetId: tc.assetId });
    const view = render(
      <ManualConditionsPanel
        {...createBaseProps({ activeResourceId: "science.gee.catalog.v1" })}
      />
    );
    const tpl = view.getByTestId("gee-dynamic-filter-template");
    assert.equal(tpl.getAttribute("data-template"), tc.expectedTemplate);
    assert.equal(
      Boolean(view.container.querySelector('[data-condition-group="time"]')),
      tc.expectDateBox,
      `time group mismatch for ${tc.assetId}`
    );
    assert.equal(tpl.querySelectorAll("select").length, 0, "GEE dynamic filter template must use SpSelect, never native <select>");
    view.unmount();
  }
  updateWorkbenchToolOptions({ geeAssetId: "ALL" });
});

test("9. [Owner Round-6 #7] Community GEE dataset (Wuhan Univ. CLCD) with Rust string tags never crashes and DataSourceCatalogModal stays locked in GEE mode", () => {
  // 1. Raw Rust GeeDatasetProfile.tags is a space-separated String; normalizeGeeCatalogItem & classifyGeeDatasetKind must handle both (including unindexed community IDs)
  const clcdRaw = {
    id: "projects/sat-io/open-datasets/CLCD",
    title: "China Land Cover Dataset (CLCD) Annual 30m (1985-2023)",
    title_zh: "武汉大学中国逐年 30米土地覆盖数据集 CLCD (社区精选)",
    source: "community",
    asset_type: "image_collection",
    native_scale_m: 30,
    tags: "china land cover clCD 30m annual 中国土地覆盖 武汉大学 土地利用 社区",
  };
  const unindexedCommunityRaw = {
    ...clcdRaw,
    id: "projects/sat-io/open-datasets/CommunityLulcTest",
  };
  const normalized = normalizeGeeCatalogItem(clcdRaw);
  const normalizedUnindexed = normalizeGeeCatalogItem(unindexedCommunityRaw);
  assert.ok(Array.isArray(normalized.tags));
  assert.strictEqual(classifyGeeDatasetKind(clcdRaw.id, clcdRaw as any), "lulc");
  assert.strictEqual(classifyGeeDatasetKind(normalized.id, normalized), "lulc");
  assert.strictEqual(classifyGeeDatasetKind(unindexedCommunityRaw.id, unindexedCommunityRaw as any), "lulc");
  assert.strictEqual(classifyGeeDatasetKind(normalizedUnindexed.id, normalizedUnindexed), "lulc");

  // 2. lockedToGee=true hides non-GEE catalog nav items & updates geeAssetId without switching out of GEE mode
  const selectedSources: string[] = [];
  let closed = false;
  updateWorkbenchToolOptions({ geeAssetId: "COPERNICUS/S2_SR_HARMONIZED" });
  const modalView = render(
    <DataSourceCatalogModal
      isOpen={true}
      lockedToGee={true}
      onClose={() => {
        closed = true;
      }}
      availableSources={mockSources}
      activeResourceId="science.gee.catalog.v1"
      onSelectSource={(id) => {
        selectedSources.push(id);
      }}
      accountStatuses={{ gee: "logged_in", asf: "logged_in" }}
    />
  );

  assert.strictEqual(modalView.queryByTestId("catalog-nav-primary-sar"), null);
  assert.ok(modalView.getByTestId("catalog-nav-gee"));
  const clcdCard = modalView.getByTestId("gee-public-entry-projects_sat-io_open-datasets_CLCD");
  fireEvent.click(clcdCard);
  assert.strictEqual(getWorkbenchToolOptions().geeAssetId, "projects/sat-io/open-datasets/CLCD");
  assert.strictEqual(closed, true);
  assert.deepStrictEqual(selectedSources, []);
  modalView.unmount();

  // 3. GeeWorkbenchPanel renders cleanly with projects/sat-io/open-datasets/CLCD active & emits geeOnly on catalog btn click
  const openCatalogArgs: Array<unknown> = [];
  const geeView = render(
    <ManualConditionsPanel
      {...createBaseProps({
        scientificMode: "gee",
        activeResourceId: "science.gee.catalog.v1",
        onOpenCatalog: ((opts?: unknown) => {
          openCatalogArgs.push(opts);
        }) as any,
      })}
    />
  );
  const activeCardText = geeView.getByTestId("gee-active-dataset-card").textContent || "";
  assert.ok(activeCardText.includes("CLCD"));
  assert.ok(activeCardText.includes("地表专题"));
  fireEvent.click(geeView.getByTestId("gee-open-catalog-btn"));
  assert.deepStrictEqual(openCatalogArgs, [{ geeOnly: true }]);
  geeView.unmount();
  updateWorkbenchToolOptions({ geeAssetId: "ALL" });
});

test("8. Item 1 & Item 8: NISAR Release Tier uses official Validated/Provisional dark pills (no V1/V2 jargon) and Time has 5 balanced presets (3m/6m/1y/3y/5y)", () => {
  updateWorkbenchToolOptions({ nisarReleaseTiers: ["validated", "provisional"], nisarProductLevel: "RSLC" });
  const starts: string[] = [];
  const ends: string[] = [];
  const view = render(
    <ManualConditionsPanel
      {...createBaseProps({
        activeResourceId: "science.nisar.l.rslc.provisional.v1",
        productType: "RSLC",
        onStartDateChange: (d) => starts.push(d),
        onEndDateChange: (d) => ends.push(d),
      })}
    />
  );

  // Item 1: Release Tier pills match Product Level style, official NASA/ASF names, zero V1/V2/checkbox glyphs
  const tierSection = view.getByTestId("nisar-release-tier-selector");
  const validBtn = view.getByTestId("nisar-tier-checkbox-validated");
  const provBtn = view.getByTestId("nisar-tier-checkbox-provisional");
  assert.strictEqual(validBtn.textContent?.trim(), "Validated");
  assert.strictEqual(provBtn.textContent?.trim(), "Provisional");
  assert.strictEqual(validBtn.getAttribute("aria-checked"), "true");
  assert.strictEqual(provBtn.getAttribute("aria-checked"), "true");
  assert.ok(validBtn.className.includes("bg-sp-accent-fill"));
  assert.ok(provBtn.className.includes("bg-sp-accent-fill"));
  assert.strictEqual(tierSection.textContent?.includes("☑"), false);
  assert.strictEqual(tierSection.textContent?.includes("V1"), false);
  assert.strictEqual(view.container.textContent?.includes("Range V2"), false);
  assert.ok(view.container.textContent?.includes("HDF5 HTTP Range"));

  // Deselecting Provisional keeps Validated active; attempting to deselect the last active tier is ignored
  act(() => {
    fireEvent.click(provBtn);
  });
  assert.deepStrictEqual(getWorkbenchToolOptions().nisarReleaseTiers, ["validated"]);
  act(() => {
    fireEvent.click(validBtn);
  });
  assert.deepStrictEqual(getWorkbenchToolOptions().nisarReleaseTiers, ["validated"]);

  // Item 8: 5 date presets in flex-wrap row (近3月, 近半年, 近1年, 近3年, 近5年) so localized labels never clip
  const presetsBar = view.getByTestId("time-quick-presets");
  assert.ok(presetsBar.className.includes("flex-wrap"));
  const expectedPresets = [
    ["3m", "近3月"],
    ["6m", "近半年"],
    ["1y", "近1年"],
    ["3y", "近3年"],
    ["5y", "近5年"],
  ] as const;
  for (const [kind, label] of expectedPresets) {
    const btn = view.getByTestId(`time-quick-${kind}`);
    assert.strictEqual(btn.textContent?.trim(), label);
    assert.ok(btn.className.includes("whitespace-nowrap"));
  }
  act(() => {
    fireEvent.click(view.getByTestId("time-quick-3y"));
    fireEvent.click(view.getByTestId("time-quick-5y"));
  });
  assert.strictEqual(starts.length, 2);
  assert.strictEqual(ends.length, 2);
  const yNow = new Date().getFullYear();
  assert.strictEqual(Number(starts[0].slice(0, 4)), yNow - 3);
  assert.strictEqual(Number(starts[1].slice(0, 4)), yNow - 5);

  view.unmount();
  updateWorkbenchToolOptions({ nisarReleaseTiers: ["validated", "provisional"] });
});
