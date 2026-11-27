import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
Object.defineProperty(window, "innerWidth", { configurable: true, value: 1440 });
Object.assign(window, {
  requestAnimationFrame: (callback: FrameRequestCallback) => window.setTimeout(() => callback(Date.now()), 0),
  cancelAnimationFrame: (handle: number) => window.clearTimeout(handle),
});
import React from "react";
import { act } from "@testing-library/react";
import { cleanup, fireEvent, render } from "../../test-utils/render-harness";
import { I18nProvider, LOCALE_STORAGE_KEY, setCurrentLocale, translate, useI18n, type Locale } from "../../i18n";
import { clearTestDesktopShellBridge, registerTestDesktopShellBridge } from "../../lib/desktopShellBridge";
import { ManualWorkbench, type ManualWorkbenchProps } from "./ManualWorkbench";
import { WaybackPanel } from "./WaybackPanel";
import type { Bbox } from "../../lib/bridge";

const geometry = {
  type: "Polygon",
  coordinates: [[[110, 30], [111, 30], [111, 31], [110, 31], [110, 30]]],
};
const bbox: Bbox = { west: 110, south: 30, east: 111, north: 31, crs: "EPSG:4326" };

const props: ManualWorkbenchProps = {
  scientificMode: "sar",
  onScientificModeChange: () => {},
  onResetParams: () => {},
  availableSources: [{ resource_id: "science.sentinel-1.standard.v1", display_name_zh: "Sentinel-1", family: "sar" } as ManualWorkbenchProps["availableSources"][number]],
  activeResourceId: "science.sentinel-1.standard.v1",
  onSelectSource: () => {},
  productType: "",
  onProductTypeChange: () => {},
  workingAoi: bbox,
  workingGeometry: geometry,
  aoiLabel: "武汉市 · 湖北省",
  onReplaceAoi: async () => {},
  onImportVectorSuccess: () => {},
  startDate: null,
  endDate: null,
  onStartDateChange: () => {},
  onEndDateChange: () => {},
  orbitDirection: "ascending",
  onOrbitDirectionChange: () => {},
  relativeOrbit: null,
  onRelativeOrbitChange: () => {},
  frame: null,
  onFrameChange: () => {},
  searching: false,
  onTriggerSearch: () => {},
  frozenSearchRequest: {
    resource_id: "science.sentinel-1.standard.v1",
    aoi: geometry,
    aoi_name: "武汉市 · 湖北省",
    time_range: null,
    filters: { dataset: "SENTINEL-1", flight_direction: "ASCENDING" },
    request_id: "request-language",
  },
  scenes: [{
    scene_id: "scene-current",
    file_name: "S1A_IW_SLC__1SDV_TEST_PRODUCT.SAFE",
    platform: "Sentinel-1A",
    product_type: "SLC",
    beam_mode: "IW",
    polarization: "VV+VH",
    acquisition_datetime: "2024-04-08T10:01:32Z",
    absolute_orbit: null,
    relative_orbit: 69,
    path: 69,
    frame: 450,
    orbit_direction: "ascending",
    has_url: true,
  }],
  resultTotalCount: 128,
  selectedSceneId: null,
  onSelectScene: () => {},
  checkedSceneIds: new Set(),
  onToggleCheckScene: () => {},
  onToggleCheckAll: () => {},
  onDownloadChecked: () => {},
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
};

function renderIn(locale: Locale) {
  window.localStorage.setItem(LOCALE_STORAGE_KEY, locale);
  return render(
    <I18nProvider>
      <div style={{ width: 1440 }}>
        <ManualWorkbench {...props} layoutWidth={1440} />
      </div>
    </I18nProvider>,
  );
}

test.afterEach(() => {
  cleanup();
  window.localStorage.clear();
  setCurrentLocale("zh-CN");
  document.documentElement.lang = "zh-CN";
});

test("English map workbench labels cover search, column strips, result rows, and condition tags", () => {
  const view = renderIn("en");
  assert.ok(view.getByRole("button", { name: "Search", exact: true }));
  assert.ok(view.getByRole("button", { name: "Remove Orbit direction" }));
  assert.ok(view.getByRole("button", { name: /^Date/ }));
  assert.ok(view.getByRole("button", { name: "Size" }));
  assert.ok(view.getByRole("button", { name: "Select", exact: true }));
  assert.ok(view.getByRole("button", { name: "Download" }));
  assert.ok(view.getByRole("region", { name: "Map" }));

  fireEvent.click(view.getByRole("button", { name: "Collapse results" }));
  assert.ok(view.getByRole("button", { name: "Expand results" }));
  assert.equal(document.documentElement.lang, "en");
});

test("Japanese map workbench labels cover search, column strips, result rows, and condition tags", () => {
  const view = renderIn("ja");
  assert.ok((view.getByTestId("btn-start-search").textContent || "").includes("検索"));
  assert.ok(view.getByRole("button", { name: "軌道方向を削除" }));
  assert.ok(view.getByRole("button", { name: /^日付/ }));
  assert.ok(view.getByRole("button", { name: "サイズ" }));
  assert.ok(view.getByRole("button", { name: "選択", exact: true }));
  assert.ok(view.getByRole("button", { name: "ダウンロード" }));
  assert.ok(view.getByRole("region", { name: "地図" }));

  fireEvent.click(view.getByRole("button", { name: "結果を折りたたむ" }));
  assert.ok(view.getByRole("button", { name: "結果を展開" }));
  assert.equal(document.documentElement.lang, "ja");
});

test("Switching locale live from zh-CN to ja updates mounted ManualWorkbench memos, empty states, NISAR actions, and source tag without Chinese leftovers", () => {
  window.localStorage.setItem(LOCALE_STORAGE_KEY, "zh-CN");
  const nisarSourceId = "science.nisar.s.ssar.rslc.v1";

  function Harness() {
    const { setLocale } = useI18n();
    return (
      <div style={{ width: 1440 }} className="flex">
        <button type="button" data-testid="switch-ja" onClick={() => setLocale("ja")}>
          ja
        </button>
        <ManualWorkbench
          {...props}
          layoutWidth={1440}
          availableSources={[
            { resource_id: nisarSourceId, display_name_zh: "NISAR S 波段 SAR", family: "sar" } as ManualWorkbenchProps["availableSources"][number],
          ]}
          activeResourceId={nisarSourceId}
          activeSourceView={{
            resource_id: nisarSourceId,
            batch_id: "batch-nisar-empty",
            searched_at: new Date().toISOString(),
            item_count: 0,
            last_error: null,
          } as ManualWorkbenchProps["activeSourceView"]}
          frozenSearchRequest={{
            resource_id: nisarSourceId,
            aoi: geometry,
            aoi_name: "合肥市",
            time_range: null,
            filters: { dataset: "NISAR" },
            request_id: "request-nisar-ja",
          }}
          scenes={[]}
          resultTotalCount={0}
          onOpenNisarSubsetWorkspace={() => {}}
        />
      </div>
    );
  }

  const view = render(
    <I18nProvider>
      <Harness />
    </I18nProvider>,
  );

  // Starts in Simplified Chinese
  assert.ok(view.getByText("当前条件下未找到匹配影像"));

  // Switch live to Japanese while mounted
  fireEvent.click(view.getByTestId("switch-ja"));
  assert.equal(document.documentElement.lang, "ja");

  const bodyText = view.container.textContent || "";
  for (const forbidden of [
    "当前条件下未找到匹配影像",
    "可尝试放宽云量上限",
    "重置筛选条件",
    "上次检索",
    "本次条件",
    "已勾选 0",
    "AOI 裁剪下载",
    "完整产品下载",
    "任务状态",
    "NISAR S 波段 SAR",
  ]) {
    assert.equal(bodyText.includes(forbidden), false, `must not keep Simplified Chinese leftover: ${forbidden}`);
  }

  assert.ok((view.getByTestId("btn-start-search").textContent || "").includes("検索"));
  assert.ok(bodyText.includes("前回の検索"));
  assert.ok(bodyText.includes("今回の条件"));
  assert.ok(bodyText.includes("0 件選択済み"));
  assert.ok(bodyText.includes("並べ替え"));
  assert.ok(bodyText.includes("タスク状態"));
  assert.ok(bodyText.includes("AOI 切り抜き"));
  assert.ok(bodyText.includes("完全版を取得"));
  assert.ok(bodyText.includes("条件に一致する画像が見つかりませんでした"));
  assert.ok(bodyText.includes("条件をリセット"));
  assert.ok(bodyText.includes("NISAR Sバンド SAR"));
});

test("Sentinel-2 L2A optical workbench & result rows translate filters, preview buttons, and AX attributes to ja with zero Simplified Chinese leftovers", () => {
  window.localStorage.setItem(LOCALE_STORAGE_KEY, "zh-CN");
  setCurrentLocale("zh-CN");
  const s2SourceId = "science.sentinel-2.l2a.v1";

  function OpticalHarness() {
    const { setLocale } = useI18n();
    return (
      <div style={{ width: 1440 }} className="flex">
        <button type="button" data-testid="switch-s2-ja" onClick={() => setLocale("ja")}>
          ja
        </button>
        <ManualWorkbench
          {...props}
          scientificMode="sar"
          layoutWidth={1440}
          availableSources={[
            {
              resource_id: s2SourceId,
              display_name_zh: "Sentinel-2 L2A",
              family: "optical",
              filter_schema_id: "search.optical.sentinel-2.l2a.v1",
            } as ManualWorkbenchProps["availableSources"][number],
          ]}
          activeResourceId={s2SourceId}
          activeSourceView={{
            resource_id: s2SourceId,
            batch_id: "batch-s2-ja",
            searched_at: new Date().toISOString(),
            item_count: 1,
            last_error: null,
          } as ManualWorkbenchProps["activeSourceView"]}
          frozenSearchRequest={{
            resource_id: s2SourceId,
            aoi: geometry,
            aoi_name: "合肥市",
            time_range: null,
            filters: { dataset: "SENTINEL-2-L2A", cloud_cover_lte: 20, spacecraft: ["SENTINEL-2A"] },
            request_id: "request-s2-ja",
          }}
          scenes={[
            {
              ...props.scenes[0],
              scene_id: "S2A_MSIL2A_20261126T025039_N0513_R132_T50SMA_20261126T050354.SAFE",
              file_name: "S2A_MSIL2A_20261126T025039_N0513_R132_T50SMA_20261126T050354.SAFE",
              dataset: s2SourceId,
              platform: "Sentinel-2A",
              product_type: "S2MSI2A",
              beam_mode: "",
              polarization: "",
              orbit_direction: "",
              relative_orbit: null,
              absolute_orbit: null,
              path: null,
              frame: null,
              acquisition_datetime: "2026-11-26T02:50:39Z",
              cloud_cover: 0,
              has_url: true,
              footprint_bbox: { west: 117.0, south: 31.0, east: 118.0, north: 32.0, crs: "EPSG:4326" },
              native_metadata: {
                preview_data_url: "data:image/png;base64,iVBORw0KGgo=",
              },
            },
          ]}
          resultTotalCount={1}
          geodataRasterOverlay={{
            id: "s1-grd-preview",
            label: "Sentinel-1 GRD (AWS Open Data)",
            previewDataUrl: "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQYV2P4z8BQDwAEgAF/pooBPQAAAABJRU5ErkJggg==",
            bbox,
          }}
        />
      </div>
    );
  }

  const view = render(
    <I18nProvider>
      <OpticalHarness />
    </I18nProvider>,
  );

  // Starts in Simplified Chinese
  assert.ok(view.getByText("最大云量"));
  assert.ok(view.getByText("平台"));
  assert.ok(view.getByText("预览"));

  // Switch live to Japanese while mounted
  fireEvent.click(view.getByTestId("switch-s2-ja"));
  assert.equal(document.documentElement.lang, "ja");

  const bodyText = view.container.textContent || "";
  const attrValues = Array.from(view.container.querySelectorAll("*"))
    .flatMap((el) => [
      el.getAttribute("aria-label") || "",
      el.getAttribute("title") || "",
      el.getAttribute("placeholder") || "",
    ])
    .filter(Boolean)
    .join("\n");

  for (const forbidden of [
    "预览",
    "最大云量",
    "留空不限制",
    "平台",
    "查看并复制影像元数据",
    "打开行政区划边界检索弹窗",
    "清除空间范围",
    "行政区检索历史",
    "范围收藏夹",
    "已替换范围",
  ]) {
    assert.equal(bodyText.includes(forbidden), false, `visible text leaked Simplified Chinese: ${forbidden}`);
    assert.equal(attrValues.includes(forbidden), false, `AX/attribute leaked Simplified Chinese: ${forbidden}`);
  }

  assert.ok(bodyText.includes("プレビュー"));
  assert.ok(bodyText.includes("最大雲量"));
  assert.ok(bodyText.includes("プラットフォーム"));
  assert.ok(bodyText.includes("空欄の場合は制限なし"));
  assert.ok(attrValues.includes("画像メタデータを表示・コピー"));
  assert.ok(attrValues.includes("空間範囲をクリア"));
  assert.ok(attrValues.includes("制限なし"));
  assert.equal(attrValues.includes("空欄の場合は制限なし"), false);
  assert.ok(attrValues.includes("最大雲量を削除"));
  assert.ok(attrValues.includes("プラットフォームを削除"));
  assert.equal(attrValues.includes("最大云量を削除"), false);
  assert.equal(attrValues.includes("平台を削除"), false);

  // Raster overlay toggle chip renders in Japanese (ラスタ表示中 -> ラスタ非表示)
  const rasterToggleBtn = view.getByTestId("btn-toggle-geodata-raster-overlay");
  assert.equal(rasterToggleBtn.textContent?.includes("ラスタ表示中"), true);
  assert.equal(rasterToggleBtn.textContent?.includes("栅格图层已开"), false);
  assert.equal((rasterToggleBtn.getAttribute("title") || "").includes("地図ラスタ表示を隠す"), true);
  assert.equal((rasterToggleBtn.getAttribute("title") || "").includes("隐藏地图栅格叠加"), false);
  fireEvent.click(rasterToggleBtn);
  assert.equal(rasterToggleBtn.textContent?.includes("ラスタ非表示"), true);
  assert.equal(rasterToggleBtn.textContent?.includes("栅格图层已关"), false);
  assert.equal((rasterToggleBtn.getAttribute("title") || "").includes("地図ラスタ表示をオン"), true);

  // Basemap switcher renders localized Japanese basemap & annotation titles and tooltips (zero Simplified Chinese)
  const layersBtn = view.getByTestId("map-tool-btn-layers");
  fireEvent.click(layersBtn);
  const fullTextAfterLayers = view.container.textContent || "";
  const fullTitlesAfterLayers = Array.from(view.container.querySelectorAll("*"))
    .map((el) => el.getAttribute("title") || "")
    .filter(Boolean)
    .join("\n");
  for (const expectedJaLabel of [
    "ArcGIS 衛星",
    "ArcGIS 道路",
    "Google 衛星",
    "Google マップ",
    "Google ハイブリッド",
    "高徳地図",
    "高徳衛星",
    "天地図 衛星",
    "天地図 ベクタ",
    "天地図 画像注記",
    "天地図 ベクタ注記",
  ]) {
    assert.equal(fullTextAfterLayers.includes(expectedJaLabel), true, `basemap switcher missing ja label: ${expectedJaLabel}`);
  }
  for (const forbiddenBasemapLeak of [
    "ArcGIS 卫星",
    "ArcGIS 街道",
    "Google 卫星",
    "Google 地图",
    "Google 混合",
    "高德地图",
    "高德卫星",
    "天地图卫星",
    "天地图矢量",
    "天地图影像注记",
    "天地图矢量注记",
    "部分网络不可达",
    "GCJ-02 图源",
  ]) {
    assert.equal(fullTextAfterLayers.includes(forbiddenBasemapLeak), false, `basemap switcher visible text leaked Simplified Chinese: ${forbiddenBasemapLeak}`);
    assert.equal(fullTitlesAfterLayers.includes(forbiddenBasemapLeak), false, `basemap switcher title leaked Simplified Chinese: ${forbiddenBasemapLeak}`);
  }
  // Clicking map attribution toggle (`地図ソース`) after selecting AMap never leaks Simplified Chinese
  fireEvent.click(view.getByText("高徳地図"));
  const attrToggleBtn = view.getByRole("button", { name: "地図ソース" });
  fireEvent.click(attrToggleBtn);
  const attrText = view.container.textContent || "";
  assert.ok(attrText.includes("Leaflet"));
  assert.equal(attrText.includes("天地图"), false, "map attribution leaked Simplified Chinese 天地图");
  assert.equal(attrText.includes("高德地图"), false, "map attribution leaked Simplified Chinese 高德地图");
  assert.equal(translate("ja", "panels.mapAttribution.tianditu"), "天地図");
  assert.equal(translate("ja", "panels.mapAttribution.amap"), "高徳地図");
  assert.equal(translate("en", "panels.mapAttribution.tianditu"), "Tianditu");
  assert.equal(translate("en", "panels.mapAttribution.amap"), "AMap");
  view.unmount();

  // Dedicated search summary & NISAR band-level strings under ja have zero Simplified Chinese
  const jaNisarTitle = translate("ja", "mapWorkbench.desktop.nisarBandLevelTitle", {
    band: translate("ja", "mapWorkbench.desktop.nisarLBandLabel"),
    level: "RSLC",
  });
  const jaNisarSummary = translate("ja", "mapWorkbench.desktop.dedicatedSearchMatched", {
    source: jaNisarTitle,
    count: 22,
    hint: "",
  });
  assert.equal(jaNisarSummary, "NISAR Lバンド RSLC：22 件一致");
  assert.equal(jaNisarSummary.includes("共检索匹配"), false);
  assert.equal(jaNisarSummary.includes("波段"), false);

  const jaS1Summary = translate("ja", "mapWorkbench.desktop.dedicatedSearchMatched", {
    source: "Sentinel-1 GRD (AWS Open Data)",
    count: 20,
    hint: "",
  });
  assert.equal(jaS1Summary, "Sentinel-1 GRD (AWS Open Data)：20 件一致");
  assert.equal(jaS1Summary.includes("共检索匹配"), false);

  // SAR row info popover (`i`) under ja uses Japanese summary labels & Ascending (not 升轨 / 轨道信息)
  const sarView = renderIn("ja");
  fireEvent.focus(sarView.getByTestId("row-info-copy"));
  const infoPopover = sarView.getByTestId("row-info-popover");
  const popoverText = infoPopover.textContent || "";
  assert.equal(popoverText.includes("升轨"), false, "SAR i popover must not leak 升轨");
  assert.equal(popoverText.includes("轨道信息"), false, "SAR i popover must not leak 轨道信息");
  assert.equal(popoverText.includes("点击 i 复制"), false, "SAR i popover footer must not leak Chinese");
  assert.ok(popoverText.includes("軌道情報"));
  assert.ok(popoverText.includes("Ascending"));
  assert.ok(popoverText.includes("i をクリックして標準英語メタデータをコピー"));
});

test("WaybackPanel translates visible controls, headers, ARIA/title attributes, error fallbacks, and keeps currentRelease non-breaking across ja and en", async () => {
  cleanup();
  window.localStorage.setItem(LOCALE_STORAGE_KEY, "zh-CN");
  setCurrentLocale("zh-CN");

  let failVersionsFetch = false;
  registerTestDesktopShellBridge({
    kind: "tauri",
    application: {
      openGeodataQuery: async (req: Record<string, unknown>) => {
        if (req?.dataset === "esri_wayback_versions") {
          if (failVersionsFetch) return { ok: false };
          return {
            ok: true,
            versions: [
              {
                release_num: "3692",
                release_date: "2026-11-18",
                label: "Wayback 2026-11-18",
                tile_url_template: "https://example.test/{z}/{y}/{x}",
              },
            ],
            cache_source: "memory",
          };
        }
        return { ok: true };
      },
    },
  } as any);

  function WaybackHarness() {
    const { setLocale } = useI18n();
    return (
      <div>
        <button type="button" data-testid="wb-switch-ja" onClick={() => setLocale("ja")}>
          ja
        </button>
        <button type="button" data-testid="wb-switch-en" onClick={() => setLocale("en")}>
          en
        </button>
        <WaybackPanel
          workingAoi={null}
          selectedZooms={[18]}
          activeVersionId="3692"
          selectedPeriods={[{ releaseNum: "3692", dateLabel: "2026-11-18", label: "Wayback 2026-11-18" }]}
          probedMaxZoom={19}
          onChangeActiveVersionId={() => {}}
          onChangeSelectedPeriods={() => {}}
          onChangeProbedMaxZoom={() => {}}
        />
      </div>
    );
  }

  try {
    let view!: ReturnType<typeof render>;
    await act(async () => {
      view = render(
        <I18nProvider>
          <WaybackHarness />
        </I18nProvider>,
      );
      await new Promise((resolve) => setTimeout(resolve, 0));
    });

    // Initial zh-CN labels and non-breaking currentReleaseLabel
    assert.ok((view.getByTestId("wayback-panel").textContent || "").includes("Wayback 历史档案"));
    assert.ok((view.getByTestId("wayback-start-scan").textContent || "").includes("扫描当地变更"));
    const zhReleaseWrap = view.getByTestId("wayback-probed-max-zoom").querySelector("span.whitespace-nowrap");
    assert.ok(zhReleaseWrap !== null, "zh currentRelease span must be mounted");
    assert.ok((zhReleaseWrap?.textContent || "").includes("当前期次 2026-11-18"));

    // Switch live to Japanese and verify mounted controls + currentRelease + error fallback
    await act(async () => {
      fireEvent.click(view.getByTestId("wb-switch-ja"));
    });
    const jaPanel = view.getByTestId("wayback-panel");
    const jaText = jaPanel.textContent || "";
    const jaAttrs = Array.from(jaPanel.querySelectorAll("[title], [aria-label]"))
      .map((el) => `${el.getAttribute("title") || ""} ${el.getAttribute("aria-label") || ""}`)
      .join("\n");

    const jaReleaseWrap = view.getByTestId("wayback-probed-max-zoom").querySelector("span.whitespace-nowrap");
    assert.ok(jaReleaseWrap !== null, "ja currentRelease span must be mounted");
    assert.ok((jaReleaseWrap?.textContent || "").includes("選択期 2026-11-18"));

    failVersionsFetch = true;
    const jaRefreshBtn = view.getByTestId("wayback-refresh-versions");
    assert.ok((jaRefreshBtn.getAttribute("title") || "").includes("リリース一覧ソース：memory"));
    await act(async () => {
      fireEvent.click(jaRefreshBtn);
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    const jaErrText = view.getByTestId("wayback-versions-error").textContent || "";
    assert.strictEqual(jaErrText, "Esri Wayback リリース一覧の取得に失敗しました");

    const jaFullText = jaPanel.textContent || "";
    for (const forbiddenZh of [
      "Wayback 历史档案",
      "刷新",
      "预览",
      "扫描当地变更",
      "快速",
      "精细",
      "官方全量",
      "已选下载期次",
      "清空期次",
      "研究区实测最高可用",
      "绑定研究区后自动探测最高可用切片层级",
      "选择 Esri Wayback 历史发布期次",
      "Esri Wayback 历史版本档案",
      "在主地图叠加预览",
      "获取 Esri Wayback 版本清单失败",
    ]) {
      assert.equal(jaText.includes(forbiddenZh), false, `WaybackPanel ja text leaked zh-CN: ${forbiddenZh}`);
      assert.equal(jaFullText.includes(forbiddenZh), false, `WaybackPanel ja error text leaked zh-CN: ${forbiddenZh}`);
      assert.equal(jaAttrs.includes(forbiddenZh), false, `WaybackPanel ja attrs leaked zh-CN: ${forbiddenZh}`);
    }
    assert.ok(jaFullText.includes("Wayback 履歴アーカイブ"));
    assert.ok(jaFullText.includes("現地変化をスキャン"));
    assert.ok(jaFullText.includes("選択中の取得期："));
    assert.ok(jaAttrs.includes("Esri Wayback リリース期を選択"));

    // Switch live to English and verify mounted controls + currentRelease + error fallback
    await act(async () => {
      fireEvent.click(view.getByTestId("wb-switch-en"));
    });
    const enPanel = view.getByTestId("wayback-panel");
    const enReleaseWrap = view.getByTestId("wayback-probed-max-zoom").querySelector("span.whitespace-nowrap");
    assert.ok(enReleaseWrap !== null, "en currentRelease span must be mounted");
    assert.ok((enReleaseWrap?.textContent || "").includes("Release 2026-11-18"));

    const enRefreshBtn = view.getByTestId("wayback-refresh-versions");
    assert.ok((enRefreshBtn.getAttribute("title") || "").includes("Release list source: memory"));
    await act(async () => {
      fireEvent.click(enRefreshBtn);
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    const enErrText = view.getByTestId("wayback-versions-error").textContent || "";
    assert.strictEqual(enErrText, "Failed to load Esri Wayback release list");

    const enText = enPanel.textContent || "";
    const enAttrs = Array.from(enPanel.querySelectorAll("[title], [aria-label]"))
      .map((el) => `${el.getAttribute("title") || ""} ${el.getAttribute("aria-label") || ""}`)
      .join("\n");
    assert.equal(/[\u4e00-\u9fff]/.test(enText), false, `WaybackPanel en text leaked CJK: ${enText}`);
    assert.equal(/[\u4e00-\u9fff]/.test(enAttrs), false, `WaybackPanel en attrs leaked CJK: ${enAttrs}`);
    assert.ok(enText.includes("Wayback Archive"));
    assert.ok(enText.includes("Scan Local Changes"));
    assert.ok(enText.includes("Selected releases:"));
  } finally {
    clearTestDesktopShellBridge();
    cleanup();
    setCurrentLocale("zh-CN");
    window.localStorage.setItem(LOCALE_STORAGE_KEY, "zh-CN");
  }
});

test("Admin modal examples/placeholders in en and ja never leak Simplified Chinese place names", () => {
  const enPlaceholder = translate("en", "mapWorkbench.admin.placeholder");
  const enRequired = translate("en", "mapWorkbench.admin.queryRequired");
  assert.equal(/[\u4e00-\u9fff]/.test(enPlaceholder), false, `en admin placeholder leaked CJK: ${enPlaceholder}`);
  assert.equal(/[\u4e00-\u9fff]/.test(enRequired), false, `en admin queryRequired leaked CJK: ${enRequired}`);

  const jaPlaceholder = translate("ja", "mapWorkbench.admin.placeholder");
  const jaRequired = translate("ja", "mapWorkbench.admin.queryRequired");
  assert.equal(/[云冈东]/.test(jaPlaceholder), false, `ja admin placeholder leaked Simplified Chinese: ${jaPlaceholder}`);
  assert.equal(/[云冈东]/.test(jaRequired), false, `ja admin queryRequired leaked Simplified Chinese: ${jaRequired}`);
});
