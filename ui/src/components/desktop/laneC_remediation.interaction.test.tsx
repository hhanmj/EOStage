import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import React, { act } from "react";
import { render, fireEvent, cleanup } from "@/test-utils/render-harness";

import {
  ManualConditionsPanel,
  getWorkbenchToolOptions,
  updateWorkbenchToolOptions,
} from "./ManualConditionsPanel";
import { DataSourceCatalogModal } from "./DataSourceCatalogModal";
import { PluginsAndSkillsPage, MARKETPLACE_PLUGIN_SUITES } from "./PluginsAndSkillsPage";
import {
  WorkbenchMap,
  extractGeodataRasterOverlayFromResponse,
} from "@/components/WorkbenchMap";
import {
  GeeLayerStackHud,
  formatCleanSwipeLayerTitle,
} from "./GeeLayerStackAndSwipe";
import { ModalOverlay } from "@/components/ui/ModalOverlay";
import { buildUnifiedSearchRequest } from "@/shell/api/searchRequest";
import type { SourceDescriptor } from "@/shell/api/types";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const uiSrcDir = path.resolve(__dirname, "../..");

if (typeof window !== "undefined") {
  if (typeof window.requestAnimationFrame !== "function") {
    window.requestAnimationFrame = (cb: FrameRequestCallback) =>
      setTimeout(() => cb(Date.now()), 0) as unknown as number;
  }
  if (typeof window.cancelAnimationFrame !== "function") {
    window.cancelAnimationFrame = (id: number) => clearTimeout(id);
  }
}

const mockSources: SourceDescriptor[] = [
  {
    resource_id: "science.sentinel-1.standard.v1",
    display_name_zh: "Sentinel-1 C波段 SAR",
    source_family: "sar",
    availability: "available",
  },
  {
    resource_id: "science.sentinel-2.l1c.v1",
    display_name_zh: "Sentinel-2 L1C 大气表观反射率",
    source_family: "optical",
    availability: "available",
  },
  {
    resource_id: "science.sentinel-3.standard.v1",
    display_name_zh: "Sentinel-3 OLCI/SLSTR",
    source_family: "optical",
    availability: "available",
  },
  {
    resource_id: "auxiliary.xyz-tiles.v1",
    display_name_zh: "在线卫星底图瓦片下载",
    source_family: "auxiliary",
    availability: "available",
  },
  {
    resource_id: "science.gee.catalog.v1",
    display_name_zh: "Google Earth Engine (GEE) 全球公共数据与云端算子库",
    source_family: "optical",
    availability: "available",
  },
];

test("[HIGH-01] ManualConditionsPanel exposes mbtiles, zip, and WEBP options for XYZ satellite tiles", () => {
  updateWorkbenchToolOptions({
    xyzTileStitchMosaic: true,
    xyzTileExportFormat: "gtiff",
    xyzTileCompression: "DEFLATE",
  });
  const view = render(
    <ManualConditionsPanel
      scientificMode="sar"
      onScientificModeChange={() => {}}
      onResetParams={() => {}}
      availableSources={mockSources}
      activeResourceId="auxiliary.xyz-tiles.v1"
      onSelectSource={() => {}}
      productType="SLC"
      onProductTypeChange={() => {}}
      workingAoi={{ west: 117.1, south: 31.7, east: 117.4, north: 31.9 }}
      workingGeometry={{
        type: "Polygon",
        coordinates: [[[117.1, 31.7], [117.4, 31.7], [117.4, 31.9], [117.1, 31.9], [117.1, 31.7]]],
      }}
      aoiLabel="合肥市"
      onReplaceAoi={async () => {}}
      onImportVectorSuccess={() => {}}
      startDate="2025-01-01"
      endDate="2025-02-01"
      onStartDateChange={() => {}}
      onEndDateChange={() => {}}
      orbitDirection="all"
      onOrbitDirectionChange={() => {}}
      relativeOrbit=""
      onRelativeOrbitChange={() => {}}
      frame=""
      onFrameChange={() => {}}
      searching={false}
      onTriggerSearch={() => {}}
    />,
  );
  try {
    const formatTrigger = view.getByRole("combobox", { name: "导出栅格格式" });
    fireEvent.click(formatTrigger);
    const mbtilesOption = view.getByRole("option", { name: "MBTiles 离线瓦片库 (.mbtiles)" });
    const zipOption = view.getByRole("option", { name: "ZIP 标准瓦片包 (.zip)" });
    assert.ok(mbtilesOption);
    assert.ok(zipOption);
    fireEvent.click(mbtilesOption);
    assert.equal(getWorkbenchToolOptions().xyzTileExportFormat, "mbtiles");

    fireEvent.click(view.getByRole("combobox", { name: "导出栅格格式" }));
    fireEvent.click(view.getByRole("option", { name: "ZIP 标准瓦片包 (.zip)" }));
    assert.equal(getWorkbenchToolOptions().xyzTileExportFormat, "zip");

    const compTrigger = view.getByRole("combobox", { name: "压缩算法" });
    fireEvent.click(compTrigger);
    const webpOption = view.getByRole("option", { name: "WEBP 高倍率有损/无损压缩" });
    assert.ok(webpOption);
    fireEvent.click(webpOption);
    assert.equal(getWorkbenchToolOptions().xyzTileCompression, "WEBP");

    // Verify multi-zoom presets, live estimator card native title & layout, and hidden irrelevant Time group
    assert.equal(view.container.querySelector('[data-condition-group="time"]'), null);
    const estimatorCard = view.getByTestId("xyz-tile-estimator-card");
    assert.ok(estimatorCard);
    assert.ok(estimatorCard.textContent?.includes("瓦片体积预估"));
    assert.equal(
      estimatorCard.textContent?.includes("ThirdParty"),
      false,
      "Unexpected third-party prefix must not appear in XYZ tile estimator card"
    );
    assert.ok(estimatorCard.textContent?.includes("瓦片数"));
    assert.ok(estimatorCard.textContent?.includes("网格 W × H"));
    assert.ok(estimatorCard.textContent?.includes("预计下载流量"));
    assert.ok(estimatorCard.textContent?.includes("预计输出文件大小"));

    const formatField = view.getByTestId("xyz-export-format-field");
    assert.equal(formatField.getAttribute("title"), "导出栅格格式：ZIP 标准瓦片包 (.zip)");
    const compField = view.getByTestId("xyz-compression-field");
    assert.equal(compField.getAttribute("title"), "压缩算法：WEBP 高倍率有损/无损压缩");

    fireEvent.click(view.getByTestId("xyz-zoom-preset-z10-14"));
    assert.deepEqual(getWorkbenchToolOptions().xyzTileZooms, [10, 11, 12, 13, 14]);
    assert.equal(getWorkbenchToolOptions().xyzTileZoom, 10);
  } finally {
    updateWorkbenchToolOptions({
      xyzTileExportFormat: "gtiff",
      xyzTileCompression: "DEFLATE",
      xyzTileZoom: 15,
      xyzTileZooms: [15],
    });
    cleanup();
  }
});

test("[XYZ-LIMITS] ManualConditionsPanel disables unsupported zoom buttons per source, clips zooms on source change, and blocks over-limit search", () => {
  updateWorkbenchToolOptions({
    xyzTileSource: "google_satellite",
    xyzTileStitchMosaic: true,
    xyzTileExportFormat: "gtiff",
    xyzTileCompression: "DEFLATE",
    xyzTileZoom: 21,
    xyzTileZooms: [18, 21],
  });
  let draftState: Record<string, unknown> | undefined = undefined;
  const makeView = () => (
    <ManualConditionsPanel
      scientificMode="sar"
      onScientificModeChange={() => {}}
      onResetParams={() => {}}
      availableSources={mockSources}
      activeResourceId="auxiliary.xyz-tiles.v1"
      onSelectSource={() => {}}
      productType="SLC"
      onProductTypeChange={() => {}}
      opticalDraft={draftState as any}
      onOpticalDraftChange={(next) => {
        draftState = next as Record<string, unknown>;
        view.rerender(makeView());
      }}
      workingAoi={{ west: 117.0, south: 31.6, east: 117.5, north: 32.0 }}
      workingGeometry={{
        type: "Polygon",
        coordinates: [[[117.0, 31.6], [117.5, 31.6], [117.5, 32.0], [117.0, 32.0], [117.0, 31.6]]],
      }}
      aoiLabel="合肥市域"
      onReplaceAoi={async () => {}}
      onImportVectorSuccess={() => {}}
      startDate="2025-01-01"
      endDate="2025-02-01"
      onStartDateChange={() => {}}
      onEndDateChange={() => {}}
      orbitDirection="all"
      onOrbitDirectionChange={() => {}}
      relativeOrbit=""
      onRelativeOrbitChange={() => {}}
      frame=""
      onFrameChange={() => {}}
      searching={false}
      onTriggerSearch={() => {}}
    />
  );
  const view = render(makeView());
  try {
    const btnZ21 = view.getByTestId("xyz-zoom-btn-21") as HTMLButtonElement;
    const btnZ22 = view.getByTestId("xyz-zoom-btn-22") as HTMLButtonElement;
    assert.equal(btnZ21.disabled, false);
    assert.equal(btnZ22.disabled, true);

    // 1. Switch source while opticalDraft is undefined (must NOT throw TypeError)
    fireEvent.click(view.getByRole("combobox", { name: "图源选择" }));
    fireEvent.click(view.getByRole("option", { name: "天地图卫星影像 · 国家地理信息公共服务平台" }));
    assert.deepEqual(getWorkbenchToolOptions().xyzTileZooms, [18]);

    // 2. Switch back to Google, click Z14 & Z21 (populates opticalDraft.xyz_zooms + xyz_zoom="14"),
    //    then switch to Tianditu -> must preserve [14, 18] rather than collapsing to stale xyz_zoom=[14]
    fireEvent.click(view.getByRole("combobox", { name: "图源选择" }));
    fireEvent.click(view.getByRole("option", { name: "Google Satellite · 全球高清卫星影像" }));
    fireEvent.click(view.getByTestId("xyz-zoom-btn-14"));
    fireEvent.click(view.getByTestId("xyz-zoom-btn-21"));
    assert.ok(draftState && Array.isArray((draftState as any).xyz_zooms));
    assert.equal((draftState as any).xyz_zoom, "14");

    fireEvent.click(view.getByRole("combobox", { name: "图源选择" }));
    fireEvent.click(view.getByRole("option", { name: "天地图卫星影像 · 国家地理信息公共服务平台" }));
    assert.deepEqual(getWorkbenchToolOptions().xyzTileZooms, [14, 18]);
    assert.equal((draftState as any).xyz_zoom, undefined);
    assert.equal((draftState as any).xyz_zooms, undefined);
    assert.equal((view.getByTestId("xyz-zoom-btn-14") as HTMLButtonElement).getAttribute("aria-pressed"), "true");
    assert.equal((view.getByTestId("xyz-zoom-btn-18") as HTMLButtonElement).getAttribute("aria-pressed"), "true");
    assert.equal((view.getByTestId("xyz-zoom-btn-19") as HTMLButtonElement).disabled, true);
    assert.equal((view.getByTestId("xyz-zoom-preset-z15-19") as HTMLButtonElement).disabled, true);

    // 0.5° x 0.4° at Z18 (~125k tiles): exceeds 50k stitched gtiff limit and disables search button
    const limitAlert = view.getByTestId("xyz-tile-hard-limit-notice");
    assert.ok(limitAlert.textContent?.includes("50,000"));
    const startBtn = view.getByTestId("btn-start-search") as HTMLButtonElement;
    assert.equal(startBtn.disabled, true);

    // Switching to MBTiles hides raster compression/pyramids, raises limit to 200,000, and re-enables search
    fireEvent.click(view.getByRole("combobox", { name: "导出栅格格式" }));
    fireEvent.click(view.getByRole("option", { name: "MBTiles 离线瓦片库 (.mbtiles)" }));
    assert.equal(view.queryByTestId("xyz-compression-field"), null);
    assert.equal(view.queryByTestId("xyz-tile-hard-limit-notice"), null);
    assert.equal((view.getByTestId("btn-start-search") as HTMLButtonElement).disabled, false);
  } finally {
    updateWorkbenchToolOptions({
      xyzTileSource: "arcgis_satellite",
      xyzTileStitchMosaic: true,
      xyzTileExportFormat: "gtiff",
      xyzTileCompression: "DEFLATE",
      xyzTileZoom: 15,
      xyzTileZooms: [15],
    });
    cleanup();
  }
});

test("[HIGH-02] WorkbenchMap renders real ImageOverlay & TileLayer for geodata rasters and supports toggle chip", () => {
  const sampleDataUrl =
    "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
  const extracted = extractGeodataRasterOverlayFromResponse(
    {
      ok: true,
      dataset: "xyz_satellite_tiles",
      preview_data_url: sampleDataUrl,
      mosaic_bbox: [117.1, 31.7, 117.4, 31.9],
      tile_url_template: "https://earthengine.googleapis.com/v1/projects/earthengine-public/maps/demo/tiles/{z}/{x}/{y}",
      exported_mosaic_path: "/tmp/xyz_mosaic.tif",
    },
    null,
    "Z15 拼接大图",
  );
  assert.ok(extracted);
  assert.equal(extracted.previewDataUrl, sampleDataUrl);
  assert.deepEqual(extracted.bbox, { west: 117.1, south: 31.7, east: 117.4, north: 31.9 });
  assert.ok(extracted.tileUrlTemplate?.includes("{z}/{x}/{y}"));

  const view = render(
    <div style={{ width: 800, height: 600 }}>
      <WorkbenchMap
        declutter
        chrome="panel"
        bbox={{ west: 117.1, south: 31.7, east: 117.4, north: 31.9 }}
        aoiBbox={{ west: 117.1, south: 31.7, east: 117.4, north: 31.9 }}
        geodataRasterOverlay={extracted}
        layerKey="arcgisWorldImagery"
        drawMode="rect"
        drawActive={false}
        onLayerChange={() => {}}
        onDrawModeChange={() => {}}
        onDrawActiveChange={() => {}}
        onClearLayers={() => {}}
        onRectDraw={() => {}}
        onPolygonDraw={() => {}}
        onPointDraw={() => {}}
      />
    </div>,
  );
  try {
    const toggleBtn = view.getByTestId("btn-toggle-geodata-raster-overlay");
    assert.equal(toggleBtn.getAttribute("aria-pressed"), "true");

    const overlayImg = view.container.querySelector("img.geodata-raster-image-overlay") as HTMLImageElement | null;
    assert.ok(overlayImg, "Leaflet ImageOverlay must mount img.geodata-raster-image-overlay");
    assert.equal(overlayImg.getAttribute("src"), sampleDataUrl);
    assert.equal(overlayImg.style.opacity, "0.88");

    fireEvent.click(toggleBtn);
    assert.equal(toggleBtn.getAttribute("aria-pressed"), "false");
    assert.equal(view.container.querySelector("img.geodata-raster-image-overlay"), null);

    fireEvent.click(toggleBtn);
    assert.equal(toggleBtn.getAttribute("aria-pressed"), "true");
    assert.ok(view.container.querySelector("img.geodata-raster-image-overlay"));
  } finally {
    cleanup();
  }
});

test("[HIGH-07] & [HIGH-08] Authoritative Sentinel-2 L1C / Sentinel-3 routing, honest failover copy, and GEE preset tracking", () => {
  const workbenchSrc = fs.readFileSync(
    path.resolve(uiSrcDir, "components/desktop/DesktopWorkbench.tsx"),
    "utf-8",
  );
  assert.equal(
    workbenchSrc.includes('"science.sentinel-2.l1c.v1": "sentinel_2_l1c"'),
    false,
    "science.sentinel-2.l1c.v1 must NOT be diverted to OPEN_GEODATA_RESOURCE_TO_DATASET",
  );
  assert.equal(
    workbenchSrc.includes('"science.sentinel-3.standard.v1": "sentinel_3_standard"'),
    false,
    "science.sentinel-3.standard.v1 must NOT be diverted to OPEN_GEODATA_RESOURCE_TO_DATASET",
  );
  assert.equal(
    workbenchSrc.includes("均有该区域"),
    false,
    "Failover banner must never falsely claim backup providers have scenes without probing",
  );

  const aoi = {
    type: "Polygon",
    coordinates: [[[117.1, 31.7], [117.4, 31.7], [117.4, 31.9], [117.1, 31.9], [117.1, 31.7]]],
  };
  const s2L1cReq = buildUnifiedSearchRequest({
    resourceId: "science.sentinel-2.l1c.v1",
    aoiGeometry: aoi,
    aoiName: "合肥市",
    startDate: "2025-01-01",
    endDate: "2025-01-31",
  });
  assert.equal(s2L1cReq.resource_id, "science.sentinel-2.l1c.v1");

  const s3Req = buildUnifiedSearchRequest({
    resourceId: "science.sentinel-3.standard.v1",
    aoiGeometry: aoi,
    aoiName: "合肥市",
    startDate: "2025-01-01",
    endDate: "2025-01-31",
  });
  assert.equal(s3Req.resource_id, "science.sentinel-3.standard.v1");

  let selectedResourceId = "";
  const modalView = render(
    <DataSourceCatalogModal
      isOpen={true}
      onClose={() => {}}
      onSelectSource={(id) => {
        selectedResourceId = id;
      }}
      availableSources={mockSources}
      activeResourceId="science.sentinel-1.standard.v1"
    />,
  );
  try {
    act(() => {
      fireEvent.click(modalView.getByTestId("catalog-nav-gee"));
    });
    const dynamicRow = modalView.container.querySelector(
      '[data-resource-id="science.gee.dynamic-world.v1"]',
    ) as HTMLElement;
    assert.ok(dynamicRow);
    act(() => {
      fireEvent.doubleClick(dynamicRow);
    });
    assert.equal(selectedResourceId, "science.gee.catalog.v1");
    assert.equal(getWorkbenchToolOptions().geeAssetId, "GOOGLE/DYNAMICWORLD/V1");
    assert.equal(getWorkbenchToolOptions().geeSelectedPresetId, "science.gee.dynamic-world.v1");
  } finally {
    cleanup();
  }
});

test("[HIGH-09] Bundled eostage-gee-plugin is visible in PluginsAndSkillsPage without manual zip import", () => {
  window.localStorage.clear();
  const bundledGee = MARKETPLACE_PLUGIN_SUITES.find((p) => p.id === "eostage-gee-plugin");
  assert.ok(bundledGee, "eostage-gee-plugin must be registered in MARKETPLACE_PLUGIN_SUITES");
  assert.equal(bundledGee.name, "Google Earth Engine 科学目录与云端计算插件");

  const view = render(<PluginsAndSkillsPage initialTab="plugins" />);
  try {
    fireEvent.click(view.getByTestId("plugins-subtab-browse"));
    const browseCard = view.getByTestId("browse-plugin-card-eostage-gee-plugin");
    assert.ok(browseCard);
    assert.ok(browseCard.textContent?.includes("Google Earth Engine 科学目录与云端计算插件"));

    fireEvent.click(view.getByTestId("browse-install-plugin-btn-eostage-gee-plugin"));
    assert.equal(view.getByTestId("plugins-subtab-added").textContent, "已添加 (1)");
    fireEvent.click(view.getByTestId("plugins-subtab-added"));
    assert.ok(view.getByTestId("plugin-suite-card-eostage-gee-plugin"));
  } finally {
    cleanup();
    window.localStorage.clear();
  }
});

test("[6.2] buildManualConditionTags formats cloud_cover_lte as 最大云量 ≤ X% and omits blank", async () => {
  const { buildManualConditionTags } = await import("./manualSearchConditions");
  const tagsWithCloud = buildManualConditionTags({
    resource_id: "science.sentinel-2.l2a.v1",
    aoi: {
      type: "Polygon",
      coordinates: [[[117.1, 31.7], [117.4, 31.7], [117.4, 31.9], [117.1, 31.9], [117.1, 31.7]]],
    },
    aoi_name: "合肥",
    time_range: { start: "2025-01-01", end: "2025-02-01" },
    filters: { cloud_cover_lte: 15 },
    limit: 50,
  });
  const cloudTag = tagsWithCloud.find((item) => item.id === "optical:cloud_cover");
  assert.ok(cloudTag);
  assert.equal(cloudTag.text, "最大云量 ≤ 15%");

  const tagsBlankCloud = buildManualConditionTags({
    resource_id: "science.sentinel-2.l2a.v1",
    aoi: null,
    aoi_name: "",
    time_range: { start: "2025-01-01", end: "2025-02-01" },
    filters: { cloud_cover_lte: "留空不限制" },
    limit: 50,
  });
  assert.equal(tagsBlankCloud.find((item) => item.id === "optical:cloud_cover"), undefined);

  const tagsEmptyCloud = buildManualConditionTags({
    resource_id: "science.sentinel-2.l2a.v1",
    aoi: null,
    aoi_name: "",
    time_range: { start: "2025-01-01", end: "2025-02-01" },
    filters: { cloud_cover_lte: "" },
    limit: 50,
  });
  assert.equal(tagsEmptyCloud.find((item) => item.id === "optical:cloud_cover"), undefined);
});

test("[SLICE-A] GeeLayerStackHud cleans PROVIDER:: raw IDs, avoids toolbar/legend overlap, and ModalOverlay renders single base backdrop when nested", () => {
  const cleanedS2 = formatCleanSwipeLayerTitle(
    "PROVIDER::copernicus.dataspace.odata::COLLECTION::science.sentinel-2.l2a.v1::ITEM::S2B_MSIL2A_20260410T025539_N0511_R032_T50RKU_20260410T054211",
    "2026-04-10"
  );
  assert.equal(cleanedS2.includes("PROVIDER::"), false);
  assert.equal(cleanedS2.includes("copernicus.dataspace"), false);
  assert.ok(cleanedS2.includes("S2B L2A T50RKU"));
  assert.ok(cleanedS2.includes("2026-04-10"));

  const cleanedTruncated = formatCleanSwipeLayerTitle(
    "PROVIDER::copernicus.dataspace...",
    "2026-04-12"
  );
  assert.equal(cleanedTruncated.includes("PROVIDER::"), false);
  assert.equal(cleanedTruncated.includes("copernicus.dataspace"), false);
  assert.ok(cleanedTruncated.includes("2026-04-12"));

  const cleanedS2c = formatCleanSwipeLayerTitle(
    "PROVIDER::copernicus.dataspace.odata::COLLECTION::science.sentinel-2.l2a.v1::ITEM::S2C_MSIL2A_20261104T024831_N0511_R132_T50SNA_20261104T055148.SAFE",
    "2026-11-04"
  );
  assert.equal(cleanedS2c, "S2C L2A T50SNA (2026-11-04)");

  const cleanedUuid = formatCleanSwipeLayerTitle(
    "PROVIDER::copernicus.dataspace.odata::COLLECTION::science.sentinel-2.l2a.v1::ITEM::1188d957-4f2b-4a8e-9c12-abcdef012345",
    "2026-11-04"
  );
  assert.equal(cleanedUuid, "1188d957 (2026-11-04)");

  const hudView = render(
    <aside>
      <div data-testid="agent-split-panel-header" />
      <div className="relative">
        <GeeLayerStackHud
          swipeLeftLayer={{
            id: "s2-left",
            title:
              "PROVIDER::copernicus.dataspace.odata::COLLECTION::science.sentinel-2.l2a.v1::ITEM::S2A_MSIL2A_20260301T025539_N0511_R032_T50RKU_20260301T054211",
            acquisitionDate: "2026-03-01",
            bbox: [117.1, 31.7, 117.4, 31.9],
            tileUrlTemplate: "https://example.com/left/{z}/{x}/{y}.png",
          }}
          swipeRightLayer={{
            id: "s2-right",
            title: "PROVIDER::copernicus.dataspace...",
            acquisitionDate: "2026-04-10",
            bbox: [117.1, 31.7, 117.4, 31.9],
            tileUrlTemplate: "https://example.com/right/{z}/{x}/{y}.png",
          }}
          swipePosition={50}
          onClearSwipe={() => {}}
        />
      </div>
    </aside>
  );
  try {
    const leftBadge = hudView.getByTestId("gee-swipe-left-badge");
    const rightBadge = hudView.getByTestId("gee-swipe-right-badge");
    const statusBar = hudView.getByTestId("scene-swipe-status-bar");
    const statusBarCorridor = statusBar.parentElement as HTMLElement;

    assert.ok(leftBadge.className.includes("left-16"));
    assert.ok(leftBadge.className.includes("max-w-[calc(50%-5rem)]"));
    assert.ok(leftBadge.className.includes("truncate"));
    assert.ok(rightBadge.className.includes("right-16"));
    assert.ok(rightBadge.className.includes("max-w-[calc(50%-5rem)]"));
    assert.ok(rightBadge.className.includes("truncate"));
    assert.ok(statusBarCorridor.className.includes("left-16"));
    assert.ok(statusBarCorridor.className.includes("right-36"));
    assert.ok(statusBarCorridor.className.includes("justify-center"));
    assert.ok(statusBarCorridor.className.includes("top-14"));
    assert.ok(statusBar.className.includes("max-w-full"));
    assert.ok(statusBar.className.includes("min-w-0"));
    assert.equal(statusBar.className.includes("right-14"), false);

    assert.equal(leftBadge.textContent?.includes("PROVIDER::"), false);
    assert.equal(rightBadge.textContent?.includes("PROVIDER::"), false);
    assert.equal(statusBar.textContent?.includes("PROVIDER::"), false);
  } finally {
    cleanup();
  }

  let outerClosed = false;
  let innerClosed = false;
  const modalView = render(
    <div>
      <ModalOverlay
        data-testid="modal-overlay-outer"
        onClose={() => {
          outerClosed = true;
        }}
      >
        <div data-testid="outer-dialog-body">Outer</div>
      </ModalOverlay>
      <ModalOverlay
        data-testid="modal-overlay-inner"
        onClose={() => {
          innerClosed = true;
        }}
      >
        <div data-testid="inner-dialog-body">Inner</div>
      </ModalOverlay>
    </div>
  );
  try {
    const outerOverlay = modalView.getByTestId("modal-overlay-outer");
    const innerOverlay = modalView.getByTestId("modal-overlay-inner");
    assert.equal(outerOverlay.getAttribute("data-overlay-layer"), "base");
    assert.equal(innerOverlay.getAttribute("data-overlay-layer"), "nested");
    assert.ok(outerOverlay.className.includes("bg-black/45"));
    assert.ok(outerOverlay.className.includes("backdrop-blur-[2px]"));
    assert.ok(innerOverlay.className.includes("bg-transparent"));
    assert.equal(innerOverlay.className.includes("bg-black/45"), false);
    assert.equal(innerOverlay.className.includes("backdrop-blur-[2px]"), false);

    fireEvent.click(outerOverlay);
    fireEvent.click(innerOverlay);
    assert.equal(outerClosed, false);
    assert.equal(innerClosed, false);
  } finally {
    cleanup();
  }
});

test("[S5a] WorkbenchMap declutter top-right legend dock shifts to top-14 when topOverlayOffsetClass is supplied", () => {
  const defaultView = render(
    <div style={{ width: 800, height: 600 }}>
      <WorkbenchMap
        declutter
        chrome="panel"
        bbox={{ west: 117.1, south: 31.7, east: 117.4, north: 31.9 }}
        aoiBbox={{ west: 117.1, south: 31.7, east: 117.4, north: 31.9 }}
        layerKey="arcgisWorldImagery"
        drawMode="rect"
        drawActive={false}
        onLayerChange={() => {}}
        onDrawModeChange={() => {}}
        onDrawActiveChange={() => {}}
        onClearLayers={() => {}}
        onRectDraw={() => {}}
        onPolygonDraw={() => {}}
        onPointDraw={() => {}}
      />
    </div>,
  );
  try {
    const dock = defaultView.getByTestId("map-declutter-top-right-dock");
    assert.ok(dock.className.includes("top-3"));
    assert.ok(dock.querySelector('[data-testid="map-dock-legend"]'));
  } finally {
    cleanup();
  }

  const shiftedView = render(
    <div style={{ width: 800, height: 600 }}>
      <WorkbenchMap
        declutter
        chrome="panel"
        topOverlayOffsetClass="top-14"
        bbox={{ west: 117.1, south: 31.7, east: 117.4, north: 31.9 }}
        aoiBbox={{ west: 117.1, south: 31.7, east: 117.4, north: 31.9 }}
        layerKey="arcgisWorldImagery"
        drawMode="rect"
        drawActive={false}
        onLayerChange={() => {}}
        onDrawModeChange={() => {}}
        onDrawActiveChange={() => {}}
        onClearLayers={() => {}}
        onRectDraw={() => {}}
        onPolygonDraw={() => {}}
        onPointDraw={() => {}}
      />
    </div>,
  );
  try {
    const dock = shiftedView.getByTestId("map-declutter-top-right-dock");
    assert.ok(dock.className.includes("top-14"));
    assert.equal(dock.className.includes("top-3"), false);
    assert.ok(dock.querySelector('[data-testid="map-dock-legend"]'));
  } finally {
    cleanup();
  }
});

test("[2026-11-20 Items 1/2/6] Sticky stack popover, basemap jump + Carto fallback, and filter dir removal", async () => {
  const stackScenes = [
    {
      scene_id: "S1A-STACK-01",
      product_key: "S1A-STACK-01",
      dataset: "science.sentinel-1.standard.v1",
      platform: "Sentinel-1A",
      product_type: "SLC",
      beam_mode: "IW",
      polarization: "VV+VH",
      acquisition_datetime: "2026-10-01T10:00:00Z",
      orbit_direction: "ascending",
      footprint_geojson: {
        type: "Polygon",
        coordinates: [[[117.1, 31.7], [117.3, 31.7], [117.3, 31.9], [117.1, 31.9], [117.1, 31.7]]],
      },
    },
    {
      scene_id: "S1A-STACK-02",
      product_key: "S1A-STACK-02",
      dataset: "science.sentinel-1.standard.v1",
      platform: "Sentinel-1B",
      product_type: "SLC",
      beam_mode: "IW",
      polarization: "VV+VH",
      acquisition_datetime: "2026-10-13T10:00:00Z",
      orbit_direction: "ascending",
      footprint_geojson: {
        type: "Polygon",
        coordinates: [[[117.1, 31.7], [117.3, 31.7], [117.3, 31.9], [117.1, 31.9], [117.1, 31.7]]],
      },
    },
  ] as any[];

  const selectedIds: string[] = [];
  const jumpTargets: string[] = [];
  const view = render(
    <div style={{ width: 900, height: 640 }}>
      <WorkbenchMap
        chrome="panel"
        bbox={{ west: 117.0, south: 31.6, east: 117.5, north: 32.0 }}
        aoiBbox={{ west: 117.1, south: 31.7, east: 117.4, north: 31.9 }}
        layerKey="cartoLight"
        scenes={stackScenes}
        stackScenesFor={() => stackScenes}
        onSceneSelect={(id) => selectedIds.push(id)}
        onOpenTileDownloadWorkbench={(src) => jumpTargets.push(src)}
        drawMode="rect"
        drawActive={false}
        onLayerChange={() => {}}
        onDrawModeChange={() => {}}
        onDrawActiveChange={() => {}}
        onClearLayers={() => {}}
        onRectDraw={() => {}}
        onPolygonDraw={() => {}}
        onPointDraw={() => {}}
      />
    </div>,
  );

  try {
    // 1. Trigger stack popover by clicking footprint polygon
    const footprintPath = view.container.querySelector("path.scene-footprint");
    assert.ok(footprintPath, "footprint polygon path must render");
    fireEvent.click(footprintPath!);
    const stackDialog = await view.findByRole("dialog", { name: "同幅影像清单" });
    const rowBtns = Array.from(stackDialog.querySelectorAll("button")).filter((b) =>
      b.textContent?.includes("2026-10-"),
    );
    assert.equal(rowBtns.length, 2);

    // Click inside stack list AND fire mousedown outside -> must remain open
    fireEvent.click(rowBtns[0]!);
    assert.equal(selectedIds[0], "S1A-STACK-01");
    assert.ok(view.queryByRole("dialog", { name: "同幅影像清单" }), "clicking item inside 同幅影像清单 must keep it open");
    fireEvent.mouseDown(document.body);
    assert.ok(view.queryByRole("dialog", { name: "同幅影像清单" }), "outside mousedown must NOT close 同幅影像清单");

    // Explicit Close button closes it
    fireEvent.click(view.getByRole("button", { name: "关闭" }));
    assert.equal(view.queryByRole("dialog", { name: "同幅影像清单" }), null);

    // Reopen and verify Escape closes it
    fireEvent.click(footprintPath!);
    await view.findByRole("dialog", { name: "同幅影像清单" });
    fireEvent.keyDown(window, { key: "Escape" });
    assert.equal(view.queryByRole("dialog", { name: "同幅影像清单" }), null);

    // 2. Open basemap popover: concise descriptions + hover title + workbench jump button, zero inline download
    fireEvent.click(view.getByRole("button", { name: "切换底图" }));
    const jumpBtn = await view.findByTestId("map-open-tile-download-workbench-btn");
    assert.equal(view.queryByTestId("map-download-satellite-tiles-btn"), null);
    assert.equal(view.queryByTestId("map-tile-download-source-select"), null);
    assert.doesNotMatch(view.container.textContent ?? "", /审查后的预览来源/);
    const cartoBtn = view.getByText(/Carto Light/i).closest("button");
    assert.ok(cartoBtn?.getAttribute("title")?.includes("浅色极简底图（已切换 Esri 浅灰免密源）"));
    fireEvent.click(jumpBtn);
    assert.deepEqual(jumpTargets, ["carto_light"]);

    // 3. Keyless Carto routes immediately to Esri Gray Canvas without requesting CARTO API-key-required watermark tiles
    const mapRoot = view.container.querySelector('[data-base-layer="cartoLight"]');
    assert.ok(mapRoot);
    assert.equal(mapRoot.getAttribute("data-carto-fallback"), "true");
  } finally {
    cleanup();
  }

  // 4. Filter panel removes download dir pickers across XYZ / Orbit / Open Geodata / DEM, keeps convertOutputDir
  for (const [scientificMode, resourceId] of [
    ["sar", "auxiliary.xyz-tiles.v1"],
    ["orbit", "orbit.sentinel-1.poeorb.v1"],
    ["sar", "open.worldcover.v1"],
    ["dem", "dem.copernicus30.v1"],
  ] as const) {
    const panelView = render(
      <ManualConditionsPanel
        scientificMode={scientificMode}
        onScientificModeChange={() => {}}
        onResetParams={() => {}}
        availableSources={mockSources}
        activeResourceId={resourceId}
        onSelectSource={() => {}}
        productType="SLC"
        onProductTypeChange={() => {}}
        workingAoi={{ west: 117.1, south: 31.7, east: 117.4, north: 31.9 }}
        workingGeometry={null}
        aoiLabel="合肥市"
        startDate="2026-01-01"
        endDate="2026-10-01"
        onStartDateChange={() => {}}
        onEndDateChange={() => {}}
        cloudMax={20}
        onCloudMaxChange={() => {}}
        polarizations={["VV", "VH"]}
        onPolarizationsChange={() => {}}
        orbitDirection="all"
        onOrbitDirectionChange={() => {}}
        relativeOrbit=""
        onRelativeOrbitChange={() => {}}
        path=""
        onPathChange={() => {}}
        frame=""
        onFrameChange={() => {}}
        beamModes={[]}
        onBeamModesChange={() => {}}
        lookDirection="all"
        onLookDirectionChange={() => {}}
        demSource="COP30"
        onDemSourceChange={() => {}}
        autoConvertEllipsoid={false}
        onAutoConvertEllipsoidChange={() => {}}
        orbitSourceType="current"
        onOrbitSourceTypeChange={() => {}}
        localSlcPath=""
        onPickLocalSlc={() => {}}
        localDemPath=""
        onPickLocalDem={() => {}}
        lastSearchText=""
        searching={false}
        searchError={null}
        inlineToolResult={null}
        onTriggerSearch={() => {}}
      />,
    );
    assert.doesNotMatch(
      panelView.container.textContent ?? "",
      /输出目录|保存目录|自动补齐下载目录/,
      `${scientificMode}/${resourceId} must not expose download directory pickers in filter panel`,
    );
    cleanup();
  }

  const convertView = render(
    <ManualConditionsPanel
      scientificMode="convert"
      onScientificModeChange={() => {}}
      onResetParams={() => {}}
      availableSources={mockSources}
      activeResourceId="dem.copernicus30.v1"
      onSelectSource={() => {}}
      productType="SLC"
      onProductTypeChange={() => {}}
      workingAoi={null}
      workingGeometry={null}
      aoiLabel=""
      startDate="2026-01-01"
      endDate="2026-10-01"
      onStartDateChange={() => {}}
      onEndDateChange={() => {}}
      cloudMax={20}
      onCloudMaxChange={() => {}}
      polarizations={[]}
      onPolarizationsChange={() => {}}
      orbitDirection="all"
      onOrbitDirectionChange={() => {}}
      relativeOrbit=""
      onRelativeOrbitChange={() => {}}
      path=""
      onPathChange={() => {}}
      frame=""
      onFrameChange={() => {}}
      beamModes={[]}
      onBeamModesChange={() => {}}
      lookDirection="all"
      onLookDirectionChange={() => {}}
      demSource="COP30"
      onDemSourceChange={() => {}}
      autoConvertEllipsoid={false}
      onAutoConvertEllipsoidChange={() => {}}
      orbitSourceType="current"
      onOrbitSourceTypeChange={() => {}}
      localSlcPath=""
      onPickLocalSlc={() => {}}
      localDemPath="/data/raw_dem.tif"
      onPickLocalDem={() => {}}
      lastSearchText=""
      searching={false}
      searchError={null}
      inlineToolResult={null}
      onTriggerSearch={() => {}}
    />,
  );
  assert.match(convertView.container.textContent ?? "", /输出目录（留空默认输出至原文件所在目录）/);
  cleanup();
});

test("[WAYBACK-FULL] Esri Wayback source selection, AOI maxZoom clipping, preview overlay lifecycle, and multi-period scaling", async () => {
  updateWorkbenchToolOptions({
    xyzTileSource: "arcgis_satellite",
    xyzTileStitchMosaic: true,
    xyzTileExportFormat: "gtiff",
    xyzTileCompression: "DEFLATE",
    xyzTileZoom: 20,
    xyzTileZooms: [18, 20],
    xyzWaybackVersionId: "3692",
    xyzWaybackSelectedPeriods: [
      { releaseNum: "3692", dateLabel: "2026-11-18", label: "World Imagery (Wayback 2026-11-18)" },
      { releaseNum: "25951", dateLabel: "2024-06-15", label: "World Imagery (Wayback 2024-06-15)" },
    ],
    xyzWaybackProbedMaxZoom: null,
  });

  const emittedOverlays: Array<{ id: string; tileUrlTemplate?: string | null } | null> = [];
  let draftState: Record<string, unknown> = { xyz_zooms: [18, 20], xyz_zoom: "20" };

  const view = render(
    <ManualConditionsPanel
      scientificMode="sar"
      onScientificModeChange={() => {}}
      onResetParams={() => {}}
      availableSources={mockSources}
      activeResourceId="auxiliary.xyz-tiles.v1"
      onSelectSource={() => {}}
      productType="SLC"
      onProductTypeChange={() => {}}
      opticalDraft={draftState}
      onOpticalDraftChange={(next) => {
        draftState = next as Record<string, unknown>;
      }}
      workingAoi={{ west: 117.25, south: 31.82, east: 117.30, north: 31.87 }}
      workingGeometry={{
        type: "Polygon",
        coordinates: [[[117.25, 31.82], [117.30, 31.82], [117.30, 31.87], [117.25, 31.87], [117.25, 31.82]]],
      }}
      aoiLabel="合肥市"
      onReplaceAoi={async () => {}}
      onImportVectorSuccess={() => {}}
      startDate="2025-01-01"
      endDate="2025-02-01"
      onStartDateChange={() => {}}
      onEndDateChange={() => {}}
      orbitDirection="all"
      onOrbitDirectionChange={() => {}}
      relativeOrbit=""
      onRelativeOrbitChange={() => {}}
      path=""
      onPathChange={() => {}}
      frame=""
      onFrameChange={() => {}}
      searching={false}
      onTriggerSearch={() => {}}
      onPreviewWaybackOverlay={(overlay) => {
        emittedOverlays.push(overlay ? { id: overlay.id, tileUrlTemplate: overlay.tileUrlTemplate } : null);
      }}
    />,
  );

  try {
    // 1. Select Esri Wayback from source selector
    fireEvent.click(view.getByRole("combobox", { name: "图源选择" }));
    fireEvent.click(view.getByRole("option", { name: /Esri Wayback/ }));
    assert.equal(getWorkbenchToolOptions().xyzTileSource, "esri_wayback");
    assert.ok(view.queryByTestId("wayback-panel") !== null);

    // 2. Apply AOI probed maxZoom=19 and verify Z20 disables & opticalDraft + toolOptions clip to <=19
    await act(async () => {
      updateWorkbenchToolOptions({ xyzWaybackProbedMaxZoom: 19 });
    });
    const z20Btn = view.getByTestId("xyz-zoom-btn-20") as HTMLButtonElement;
    assert.equal(z20Btn.disabled, true);
    assert.deepEqual(getWorkbenchToolOptions().xyzTileZooms, [18]);
    assert.deepEqual(draftState.xyz_zooms, [18]);
    assert.equal(draftState.xyz_zoom, "18");

    // 3. Multi-period estimate & button label update
    const estimatorCard = view.getByTestId("xyz-tile-estimator-card");
    assert.match(estimatorCard.textContent ?? "", /2 期累计/);
    const searchBtn = view.getByTestId("btn-start-search") as HTMLButtonElement;
    assert.match(searchBtn.textContent ?? "", /批量下载 2 期历史影像/);
    assert.equal(searchBtn.disabled, false);

    // 4. Live preview toggle emits exact CSP-allowlisted wayback-a URL template
    fireEvent.click(view.getByTestId("wayback-toggle-preview"));
    const lastOverlay = emittedOverlays.at(-1);
    assert.ok(lastOverlay !== null && lastOverlay !== undefined);
    assert.equal(
      lastOverlay?.tileUrlTemplate,
      "https://wayback-a.maptiles.arcgis.com/arcgis/rest/services/world_imagery/mapserver/tile/3692/{z}/{y}/{x}",
    );

    // 5. Clearing selected periods disables search
    updateWorkbenchToolOptions({ xyzWaybackVersionId: "", xyzWaybackSelectedPeriods: [] });
    await act(async () => {});
    assert.equal((view.getByTestId("btn-start-search") as HTMLButtonElement).disabled, true);

    // 6. Switching away from esri_wayback resets preview overlay and probed maxZoom
    fireEvent.click(view.getByRole("combobox", { name: "图源选择" }));
    fireEvent.click(view.getByRole("option", { name: /Esri World Imagery/ }));
    assert.equal(getWorkbenchToolOptions().xyzWaybackProbedMaxZoom, null);
    assert.equal(emittedOverlays.at(-1), null);
  } finally {
    updateWorkbenchToolOptions({
      xyzTileSource: "arcgis_satellite",
      xyzTileStitchMosaic: true,
      xyzTileExportFormat: "gtiff",
      xyzTileCompression: "DEFLATE",
      xyzTileZoom: 15,
      xyzTileZooms: [15],
      xyzWaybackVersionId: "",
      xyzWaybackSelectedPeriods: [],
      xyzWaybackProbedMaxZoom: null,
    });
    cleanup();
  }
});
