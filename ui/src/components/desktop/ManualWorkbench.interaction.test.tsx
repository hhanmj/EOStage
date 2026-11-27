import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
Object.defineProperty(window, "innerWidth", { configurable: true, value: 1440 });
Object.assign(window, {
  requestAnimationFrame: (callback: FrameRequestCallback) => window.setTimeout(() => callback(Date.now()), 0),
  cancelAnimationFrame: (handle: number) => window.clearTimeout(handle),
});
import React, { useState } from "react";
import { act, waitFor, within } from "@testing-library/react";
import { cleanup, fireEvent, render } from "../../test-utils/render-harness";
import { ManualWorkbench, type ManualWorkbenchProps } from "./ManualWorkbench";
import { MANUAL_COLUMNS_STORAGE_KEY } from "./manualColumnState";
import type { Bbox } from "../../lib/bridge";

const geometry = {
  type: "Polygon",
  coordinates: [[[110, 30], [111, 30], [111, 31], [110, 31], [110, 30]]],
};
const bbox: Bbox = { west: 110, south: 30, east: 111, north: 31, crs: "EPSG:4326" };

function Harness({ width = 1440, ...overrides }: Partial<ManualWorkbenchProps> & { width?: number }) {
  const widthRef = React.useRef(width);
  widthRef.current = width;
  const setContainerNode = React.useCallback((node: HTMLDivElement | null) => {
    if (node) Object.defineProperty(node, "clientWidth", { configurable: true, value: widthRef.current });
  }, []);
  const [aoi, setAoi] = useState({ geometry, bbox, label: "武汉市 · 湖北省" });
  const [orbitDirection, setOrbitDirection] = useState("ascending");
  const [selectedSceneId, setSelectedSceneId] = useState<string | null>(null);
  const props: ManualWorkbenchProps = {
    scientificMode: "sar",
    onScientificModeChange: () => {},
    onResetParams: () => {},
    availableSources: [{ resource_id: "science.sentinel-1.standard.v1", display_name_zh: "Sentinel-1", family: "sar" } as ManualWorkbenchProps["availableSources"][number]],
    activeResourceId: "science.sentinel-1.standard.v1",
    onSelectSource: () => {},
    productType: "",
    onProductTypeChange: () => {},
    workingAoi: aoi.bbox,
    workingGeometry: aoi.geometry,
    aoiLabel: aoi.label,
    onReplaceAoi: async (nextGeometry, label, tuple) => {
      setAoi(tuple ? {
        geometry: nextGeometry!,
        bbox: { west: tuple[0], south: tuple[1], east: tuple[2], north: tuple[3], crs: "EPSG:4326" },
        label,
      } : { geometry: null!, bbox: null!, label });
    },
    onImportVectorSuccess: () => {},
    startDate: null,
    endDate: null,
    onStartDateChange: () => {},
    onEndDateChange: () => {},
    orbitDirection,
    onOrbitDirectionChange: setOrbitDirection,
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
      request_id: "request-test",
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
    selectedSceneId,
    onSelectScene: setSelectedSceneId,
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
    ...overrides,
  };
  return (
    <div style={{ width }}>
      <ManualWorkbench
        {...props}
        layoutWidth={width}
        onContainerRef={setContainerNode}
        mapFootprintControls={(selectScene) => (
          <>
            <button type="button" aria-label="地图覆盖框 scene-current" onClick={() => selectScene("scene-current")} />
            <button type="button" aria-label="地图覆盖框 scene-other-page" onClick={() => selectScene("scene-other-page")} />
          </>
        )}
      />
    </div>
  );
}

test.beforeEach(() => {
  window.localStorage.clear();
});
test.afterEach(() => cleanup());

test("manual columns collapse to 0px with floating expand handles and map remains open", async () => {
  const view = render(<Harness />);
  assert.equal(view.getByRole("region", { name: "检索" }).className.includes("w-0"), false);
  fireEvent.click(view.getByRole("button", { name: "收起检索" }));
  assert.equal(view.queryByRole("region", { name: "检索（已收起）" }), null);
  const expandSearchBtn = view.getByRole("button", { name: "展开检索" });
  assert.ok(expandSearchBtn);

  fireEvent.click(view.getByRole("button", { name: "收起结果" }));
  assert.equal(view.queryByRole("region", { name: "结果（已收起）" }), null);
  const expandResultsBtn = view.getByRole("button", { name: "展开结果" });
  assert.ok(expandResultsBtn);

  assert.ok(view.getByRole("region", { name: "地图" }));

  fireEvent.click(expandSearchBtn);
  assert.ok(view.getByRole("region", { name: "检索" }));
  assert.equal(view.queryByRole("button", { name: "展开检索" }), null);

  fireEvent.click(expandResultsBtn);
  assert.ok(view.getByRole("region", { name: "结果" }));
  assert.equal(view.queryByRole("button", { name: "展开结果" }), null);
});
test("splitters support keyboard resize, persisted widths, and double-click reset", async () => {
  const view = render(<Harness />);
  const splitter = view.getByRole("separator", { name: "调整检索与地图宽度" });
  assert.equal(splitter.getAttribute("aria-valuenow"), "320");
  fireEvent.keyDown(splitter, { key: "ArrowRight" });
  assert.equal(splitter.getAttribute("aria-valuenow"), "336");
  assert.ok(window.localStorage.getItem(MANUAL_COLUMNS_STORAGE_KEY)?.includes('"search":336'));
  fireEvent.doubleClick(splitter);
  assert.equal(splitter.getAttribute("aria-valuenow"), "320");

  const resultSplitter = view.getByRole("separator", { name: "调整地图与结果宽度" });
  fireEvent.keyDown(resultSplitter, { key: "ArrowRight" });
  assert.equal(resultSplitter.getAttribute("aria-valuenow"), "424");
  fireEvent.doubleClick(resultSplitter);
  assert.equal(resultSplitter.getAttribute("aria-valuenow"), "440");
});

test("condition tags focus their group, remove only the draft filter, and mark changed conditions", async () => {
  const view = render(<Harness />);
  const tag = view.getByRole("button", { name: "ASCENDING", exact: true });
  fireEvent.click(tag);
  await act(async () => { await new Promise((resolve) => setTimeout(resolve, 0)); });
  await waitFor(() => {
    assert.equal(view.container.querySelector('[data-condition-group="orbit"]')?.getAttribute("data-condition-focused"), "true");
  });
  fireEvent.click(view.getByRole("button", { name: "移除轨道方向" }));
  assert.ok(view.getByText("条件已更改"));
  assert.ok(view.getByRole("button", { name: "ASCENDING", exact: true }));
});

test("narrow windows retain the map dialog and Escape dismissal", () => {
  const originalWidth = window.innerWidth;
  Object.defineProperty(window, "innerWidth", { configurable: true, value: 920 });
  try {
    const view = render(<Harness width={920} />);
    const mapButton = view.getByRole("button", { name: "地图", exact: true });
    fireEvent.click(mapButton);
    assert.ok(view.getByRole("dialog", { name: "地图" }));
    fireEvent.keyDown(window, { key: "Escape" });
    assert.equal(view.queryByRole("dialog", { name: "地图" }), null);
  } finally {
    Object.defineProperty(window, "innerWidth", { configurable: true, value: originalWidth });
  }
});

test("map footprint selection stays within the current result page", async () => {
  const view = render(<Harness />);
  const footprint = view.getByRole("button", { name: "地图覆盖框 scene-current" });
  const row = view.container.querySelector('[data-scene-id="scene-current"]') as HTMLElement & { scrollIntoView: () => void };
  let scrollCount = 0;
  row.scrollIntoView = () => { scrollCount += 1; };
  fireEvent.click(footprint);
  assert.equal(row.className.includes("bg-sp-selected"), true);
  await act(async () => { await new Promise((resolve) => setTimeout(resolve, 60)); });
  assert.equal(scrollCount, 1);
  fireEvent.click(view.getByRole("button", { name: "地图覆盖框 scene-other-page" }));
  assert.equal(view.container.querySelector('[data-scene-id="scene-other-page"]'), null);
  assert.equal(row.className.includes("bg-sp-selected"), true);
});

test("ALOS-2 beam mode WBS restricts polarization to HH and condition tags reflect beam mode and look direction", async () => {
  function Alos2Harness() {
    const [beamModes, setBeamModes] = useState<string[]>(["WBD"]);
    const [lookDirection, setLookDirection] = useState<string | null>("RIGHT");
    const [polarizations, setPolarizations] = useState<string[]>(["HH+HV"]);
    return (
      <ManualWorkbench
        scientificMode="sar"
        onScientificModeChange={() => {}}
        onResetParams={() => {}}
        availableSources={[
          {
            resource_id: "science.alos-2.palsar-2.scansar-l1.1.v1",
            display_name_zh: "ALOS-2 ScanSAR",
            family: "sar",
          } as ManualWorkbenchProps["availableSources"][number],
        ]}
        activeResourceId="science.alos-2.palsar-2.scansar-l1.1.v1"
        onSelectSource={() => {}}
        productType=""
        onProductTypeChange={() => {}}
        polarizations={polarizations}
        onPolarizationsChange={setPolarizations}
        beamModes={beamModes}
        onBeamModesChange={setBeamModes}
        lookDirection={lookDirection}
        onLookDirectionChange={setLookDirection}
        workingAoi={bbox}
        workingGeometry={geometry}
        aoiLabel="武汉市 · 湖北省"
        onReplaceAoi={async () => {}}
        onImportVectorSuccess={() => {}}
        startDate={null}
        endDate={null}
        onStartDateChange={() => {}}
        onEndDateChange={() => {}}
        orbitDirection="all"
        onOrbitDirectionChange={() => {}}
        relativeOrbit={null}
        onRelativeOrbitChange={() => {}}
        path={null}
        onPathChange={() => {}}
        frame={null}
        onFrameChange={() => {}}
        searching={false}
        onTriggerSearch={() => {}}
        frozenSearchRequest={{
          resource_id: "science.alos-2.palsar-2.scansar-l1.1.v1",
          aoi: geometry,
          aoi_name: "武汉市 · 湖北省",
          time_range: null,
          filters: {
            beam_mode: "WBD",
            look_direction: "RIGHT",
            polarizations: ["HH", "HV"],
          },
          request_id: "req-alos2",
        }}
        scenes={[]}
        resultTotalCount={0}
        selectedSceneId={null}
        onSelectScene={() => {}}
        checkedSceneIds={new Set()}
        onToggleCheckScene={() => {}}
        onToggleCheckAll={() => {}}
        onDownloadChecked={() => {}}
        downloadBusy={false}
        drawMode="rect"
        drawActive={false}
        onDrawModeChange={() => {}}
        onDrawActiveChange={() => {}}
        onClearLayers={() => {}}
        onRectDraw={() => {}}
        onPolygonDraw={() => {}}
        onPointDraw={() => {}}
        mapLayerKey="arcgisSatellite"
        onLayerChange={() => {}}
      />
    );
  }

  const view = render(<Alos2Harness />);
  assert.ok(view.getByRole("button", { name: "移除波束模式" }));
  assert.ok(view.getByRole("button", { name: "移除视向" }));

  fireEvent.click(view.getByRole("button", { name: "WBS", exact: true }));
  assert.ok(view.getByText("WBS 单极化仅支持 HH"));
  assert.ok(view.getByText("条件已更改"));
});

test("manual column state survives remount and invalid persisted state fails open", async () => {
  const first = render(<Harness />);
  fireEvent.click(first.getByRole("button", { name: "收起检索" }));
  assert.ok(window.localStorage.getItem(MANUAL_COLUMNS_STORAGE_KEY)?.includes('"search":true'));
  first.unmount();
  const second = render(<Harness />);
  assert.ok(second.getByRole("button", { name: "展开检索" }));
  second.unmount();
  window.localStorage.setItem(MANUAL_COLUMNS_STORAGE_KEY, JSON.stringify({ collapsed: { search: true, map: true, results: true } }));
  const third = render(<Harness />);
  assert.ok(third.getByRole("region", { name: "检索" }));
  assert.ok(third.getByRole("region", { name: "地图" }));
  assert.ok(third.getByRole("region", { name: "结果" }));
});

test("B5/B6/B8: basemap loading pill sits above bottom-left coordinates and source switch clears old preview while keeping AOI and zoom steady", async () => {
  const chengduGeometry = {
    type: "Polygon",
    coordinates: [[[104.04, 30.58], [104.12, 30.58], [104.12, 30.66], [104.04, 30.66], [104.04, 30.58]]],
  };
  const chengduBbox: Bbox = { west: 104.04, south: 30.58, east: 104.12, north: 30.66, crs: "EPSG:4326" };
  const wideSceneFootprint = {
    type: "Polygon",
    coordinates: [[[103.5, 30.0], [105.5, 30.0], [105.5, 31.8], [103.5, 31.8], [103.5, 30.0]]],
  };

  function SourceSwitchHarness() {
    const [resourceId, setResourceId] = useState("science.gee.catalog.v1");
    const [currentAoi, setCurrentAoi] = useState({
      geometry: chengduGeometry,
      bbox: chengduBbox,
      label: "成都市锦江区 · 60.5 km²",
    });
    const isGee = resourceId === "science.gee.catalog.v1";
    return (
      <div style={{ width: 1440, height: 800 }}>
        <button
          type="button"
          onClick={() => setResourceId("insar.comet-licsar.v1")}
        >
          切换到 COMET-LiCSAR
        </button>
        <button
          type="button"
          onClick={() =>
            setCurrentAoi({
              geometry,
              bbox,
              label: "武汉市 · 湖北省",
            })
          }
        >
          切换新 AOI
        </button>
        <ManualWorkbench
          scientificMode="sar"
          onScientificModeChange={() => {}}
          onResetParams={() => {}}
          availableSources={[
            { resource_id: "science.gee.catalog.v1", display_name_zh: "GEE", family: "optical" } as ManualWorkbenchProps["availableSources"][number],
            { resource_id: "insar.comet-licsar.v1", display_name_zh: "COMET-LiCSAR", family: "sar" } as ManualWorkbenchProps["availableSources"][number],
          ]}
          activeResourceId={resourceId}
          onSelectSource={setResourceId}
          productType=""
          onProductTypeChange={() => {}}
          workingAoi={currentAoi.bbox}
          workingGeometry={currentAoi.geometry}
          aoiLabel={currentAoi.label}
          onReplaceAoi={async () => {}}
          onImportVectorSuccess={() => {}}
          startDate={null}
          endDate={null}
          onStartDateChange={() => {}}
          onEndDateChange={() => {}}
          orbitDirection="all"
          onOrbitDirectionChange={() => {}}
          relativeOrbit={null}
          onRelativeOrbitChange={() => {}}
          frame={null}
          onFrameChange={() => {}}
          searching={false}
          onTriggerSearch={() => {}}
          scenes={
            isGee
              ? [
                  {
                    scene_id: "S2C_T48RVU",
                    file_name: "S2C_MSIL2A_20261106_T48RVU",
                    platform: "Sentinel-2C",
                    product_type: "L2A",
                    beam_mode: "",
                    polarization: "",
                    acquisition_datetime: "2026-11-06T03:39:31Z",
                    absolute_orbit: null,
                    relative_orbit: 61,
                    path: 61,
                    frame: null,
                    orbit_direction: "descending",
                    has_url: true,
                    footprint_bbox: { west: 103.5, south: 30.0, east: 105.5, north: 31.8 },
                    footprint_geojson: wideSceneFootprint,
                  },
                ]
              : []
          }
          resultTotalCount={isGee ? 1 : 0}
          selectedSceneId={isGee ? "S2C_T48RVU" : null}
          onSelectScene={() => {}}
          checkedSceneIds={new Set()}
          onToggleCheckScene={() => {}}
          onToggleCheckAll={() => {}}
          onDownloadChecked={() => {}}
          downloadBusy={false}
          geodataRasterOverlay={
            isGee
              ? {
                  id: "S2C_T48RVU",
                  label: "S2C_MSIL2A_20261106_T48RVU",
                  previewDataUrl: "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQYV2P4z8BQDwAEgAF/pooBPQAAAABJRU5ErkJggg==",
                  bbox: { west: 104.2, south: 30.1, east: 105.3, north: 31.0 },
                }
              : null
          }
          taskOutcomeNotice={isGee ? { tone: "neutral", text: "GEE：共检索匹配 20 项" } : null}
          drawMode="rect"
          drawActive={false}
          onDrawModeChange={() => {}}
          onDrawActiveChange={() => {}}
          onClearLayers={() => {}}
          onRectDraw={() => {}}
          onPolygonDraw={() => {}}
          onPointDraw={() => {}}
          mapLayerKey="arcgisSatellite"
          onLayerChange={() => {}}
          layoutWidth={1440}
        />
      </div>
    );
  }

  const view = render(<SourceSwitchHarness />);
  const statusPill = view.container.querySelector('[role="status"][data-map-status]') as HTMLElement | null;
  assert.ok(statusPill, "basemap loading status pill renders initially");
  assert.equal(statusPill.className.includes("bottom-11"), true, "basemap status pill sits above bottom-left coordinates");
  assert.equal(statusPill.className.includes("top-3"), false, "basemap status pill never blocks top-left toolbar");

  assert.ok(view.getByTestId("btn-toggle-geodata-raster-overlay"));
  assert.ok(view.getByText("GEE：共检索匹配 20 项"));
  assert.ok(view.container.querySelector(".aoi-overlay-halo"), "bound AOI polygon renders on map");

  const coordBefore = view.getByTestId("map-coordinate-overlay").textContent;
  assert.match(coordBefore ?? "", /104\.0800,\s*30\.6200/);

  fireEvent.click(view.getByRole("button", { name: "切换到 COMET-LiCSAR" }));

  assert.equal(view.queryByTestId("btn-toggle-geodata-raster-overlay"), null, "previous source raster preview cleared");
  assert.equal(view.queryByText("GEE：共检索匹配 20 项"), null, "previous source notice cleared");
  assert.ok(view.container.querySelector(".aoi-overlay-halo"), "bound AOI polygon remains rendered after source switch");

  const coordAfter = view.getByTestId("map-coordinate-overlay").textContent;
  assert.equal(coordAfter, coordBefore, "map center and zoom stay locked across source switch");

  fireEvent.click(view.getByRole("button", { name: "切换新 AOI" }));
  await waitFor(() => {
    assert.match(view.getByTestId("map-coordinate-overlay").textContent ?? "", /110\.5000,\s*30\.50\d\d/);
  });
});

test("P1-3: hideMultiProviderSelector prevents duplicate provider selector and NISAR subset CTA uses neutral studio tokens", () => {
  const baseProps: ManualWorkbenchProps = {
    scientificMode: "sar",
    onScientificModeChange: () => {},
    onResetParams: () => {},
    availableSources: [
      { resource_id: "science.sentinel-2.l2a.v1", display_name_zh: "Sentinel-2 L2A", family: "optical" } as ManualWorkbenchProps["availableSources"][number],
      { resource_id: "science.nisar.l.rslc.provisional.v1", display_name_zh: "NISAR L-band RSLC", family: "sar" } as ManualWorkbenchProps["availableSources"][number],
    ],
    activeResourceId: "science.sentinel-2.l2a.v1",
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
    orbitDirection: "all",
    onOrbitDirectionChange: () => {},
    relativeOrbit: null,
    onRelativeOrbitChange: () => {},
    frame: null,
    onFrameChange: () => {},
    searching: false,
    onTriggerSearch: () => {},
    scenes: [],
    resultTotalCount: 0,
    selectedSceneId: null,
    onSelectScene: () => {},
    checkedSceneIds: new Set(),
    onToggleCheckScene: () => {},
    onToggleCheckAll: () => {},
    onDownloadChecked: () => {},
    downloadBusy: false,
    onOpenNisarSubsetWorkspace: () => {},
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
    layoutWidth: 1440,
  };

  const standaloneView = render(<ManualWorkbench {...baseProps} />);
  assert.ok(standaloneView.queryByTestId("multi-provider-selector"), "standalone ManualWorkbench renders multi-provider-selector");
  assert.equal(
    standaloneView.getByTestId("provider-option-cdse").className.includes("border-sp-accent"),
    false,
    "active provider option uses neutral studio border",
  );
  standaloneView.unmount();

  const hostedView = render(<ManualWorkbench {...baseProps} hideMultiProviderSelector />);
  assert.equal(hostedView.queryByTestId("multi-provider-selector"), null, "hosted ManualWorkbench suppresses duplicate inline selector");
  hostedView.unmount();

  const nisarView = render(
    <ManualWorkbench
      {...baseProps}
      activeResourceId="science.nisar.l.rslc.provisional.v1"
    />,
  );
  const subsetBtn = nisarView.getByTestId("btn-nisar-subset-download");
  assert.equal(subsetBtn.className.includes("border-sp-accent"), false);
  assert.equal(subsetBtn.innerHTML.includes("text-sp-accent"), false);
  assert.equal(nisarView.getByTestId("conditions-header-category-badge").textContent, "SAR");
  nisarView.unmount();

  const opticalView = render(
    <ManualWorkbench
      {...baseProps}
      activeResourceId="science.sentinel-2.l2a.v1"
      scientificMode="optical"
      canBatchOpticalAoiClip
      onOpenBatchOpticalAoiClip={() => {}}
    />,
  );
  assert.equal(opticalView.getByTestId("conditions-header-category-badge").textContent, "光学");
  assert.ok(opticalView.getByTestId("btn-optical-aoi-clip-batch"));
  opticalView.unmount();

  const geeView = render(
    <ManualWorkbench
      {...baseProps}
      activeResourceId="science.gee.catalog.v1"
    />,
  );
  assert.equal(geeView.getByTestId("conditions-header-category-badge").textContent, "GEE");
  geeView.unmount();

  const demView = render(
    <ManualWorkbench
      {...baseProps}
      scientificMode="dem"
      activeResourceId="dem.cop30.v1"
    />,
  );
  assert.equal(demView.getByTestId("conditions-header-category-badge").textContent, "DEM");
  demView.unmount();
});

test("B8 polish: source switch from catalog modal swallows rejected async onSelectSource without unhandledRejection", async () => {
  const unhandled: unknown[] = [];
  const onUnhandled = (reason: unknown) => {
    unhandled.push(reason);
  };
  process.on("unhandledRejection", onUnhandled);
  try {
    let selectedId: string | null = null;
    const view = render(
      <ManualWorkbench
        scientificMode="sar"
        onScientificModeChange={() => {}}
        onResetParams={() => {}}
        availableSources={[
          { resource_id: "science.sentinel-1.standard.v1", display_name_zh: "Sentinel-1", family: "sar" } as ManualWorkbenchProps["availableSources"][number],
          { resource_id: "science.sentinel-2.l2a.v1", display_name_zh: "Sentinel-2 Level-2A", family: "optical" } as ManualWorkbenchProps["availableSources"][number],
        ]}
        activeResourceId="science.sentinel-1.standard.v1"
        onSelectSource={((id: string) => {
          selectedId = id;
          return Promise.reject(new Error("simulated bridge rejection"));
        }) as unknown as (id: string) => void}
        productType="SLC"
        onProductTypeChange={() => {}}
        workingAoi={bbox}
        workingGeometry={geometry}
        aoiLabel="四川省"
        onReplaceAoi={async () => {}}
        onImportVectorSuccess={() => {}}
        startDate={null}
        endDate={null}
        onStartDateChange={() => {}}
        onEndDateChange={() => {}}
        orbitDirection="all"
        onOrbitDirectionChange={() => {}}
        relativeOrbit={null}
        onRelativeOrbitChange={() => {}}
        frame={null}
        onFrameChange={() => {}}
        searching={false}
        onTriggerSearch={() => {}}
        scenes={[]}
        resultTotalCount={0}
        selectedSceneId={null}
        onSelectScene={() => {}}
        checkedSceneIds={new Set()}
        onToggleCheckScene={() => {}}
        onToggleCheckAll={() => {}}
        onDownloadChecked={() => {}}
        downloadBusy={false}
        drawMode="rect"
        drawActive={false}
        onDrawModeChange={() => {}}
        onDrawActiveChange={() => {}}
        onClearLayers={() => {}}
        onRectDraw={() => {}}
        onPolygonDraw={() => {}}
        onPointDraw={() => {}}
        mapLayerKey="arcgisSatellite"
        onLayerChange={() => {}}
        layoutWidth={1440}
      />,
    );

    fireEvent.keyDown(window, { key: "k", ctrlKey: true });
    const useBtn = await waitFor(() => view.getByRole("button", { name: "使用此数据源" }));
    fireEvent.click(useBtn);
    await new Promise((resolve) => setTimeout(resolve, 30));
    assert.ok(selectedId, "catalog modal invoked onSelectSource");
    assert.equal(unhandled.length, 0, "no unhandled promise rejection escaped");
    view.unmount();
  } finally {
    process.off("unhandledRejection", onUnhandled);
  }
});

test("R2: map remains mounted on narrow viewports down to 920px, expand handle has z-[1001], and re-search expands results", async () => {
  const originalWidth = window.innerWidth;
  Object.defineProperty(window, "innerWidth", { configurable: true, value: 1024 });
  try {
    const view = render(<Harness width={1024} />);
    // 1. Map remains mounted when viewport is 1024px (< 1280px)
    const mapSection = view.container.querySelector('section[data-column="map"]');
    assert.ok(mapSection, "map column must remain mounted across viewport < 1280px to prevent white screen");

    // 2. Collapse results and verify edge-docked arrow handle style, click-to-expand, and z-[1001]
    fireEvent.click(view.getByRole("button", { name: "收起结果" }));
    const expandResultsBtn = view.getByTestId("expand-results-column-btn");
    assert.ok(expandResultsBtn);
    assert.ok(expandResultsBtn.className.includes("z-[1001]"), "expand-results-column-btn must have z-[1001]");
    assert.ok(expandResultsBtn.className.includes("sp-magnetic-peek-left"), "expand-results-column-btn must use sp-magnetic-peek-left");
    assert.ok(expandResultsBtn.className.includes("right-0"), "expand-results-column-btn must dock to right-0 edge");
    assert.ok(!expandResultsBtn.textContent?.includes("结果列表"), "expand-results-column-btn must use arrow tab style without floating pill text");

    // 2b. Click edge arrow handle to re-expand results column, then collapse again
    fireEvent.click(expandResultsBtn);
    assert.ok(view.getByRole("region", { name: "结果" }), "clicking edge arrow handle must re-expand results column");
    fireEvent.click(view.getByRole("button", { name: "收起结果" }));
    assert.ok(view.getByTestId("expand-results-column-btn"));

    // 3. Trigger search while results is collapsed -> auto expands results
    const triggerSearchBtn = view.getByTestId("btn-start-search");
    fireEvent.click(triggerSearchBtn);
    await new Promise((r) => setTimeout(r, 20));
    assert.ok(view.getByRole("region", { name: "结果" }), "re-search must auto-expand results column");
  } finally {
    Object.defineProperty(window, "innerWidth", { configurable: true, value: originalWidth });
  }
});

test("Item 14 & ARIA/NISAR polish: sumbar stats nowrap, notice break-words, results scrollTop resets on batch switch, non-RSLC disables subset button", () => {
  const sampleScene = {
    product_key: "aria-scene-1",
    product_id: "aria-scene-1",
    provider: "ASF",
    dataset: "science.aria.s1-gunw.v1",
    file_name: "S1-GUNW-A-R-040-tops-20250101.nc",
    scene_id: "aria-scene-1",
    platform: "Sentinel-1",
    product_type: "GUNW",
    beam_mode: "IW",
    polarization: "VV",
    acquisition_datetime: "2025-01-01T10:00:00Z",
    absolute_orbit: null,
    relative_orbit: 40,
    path: 40,
    frame: null,
    orbit_direction: "ASCENDING",
    has_url: true,
    download_url: "https://example.com/gunw.nc",
    footprint_bbox: { west: -118.4, south: 33.9, east: -118.1, north: 34.1 },
    footprint_geojson: null,
    assets: [],
  };

  const view = render(
    <Harness
      activeResourceId="science.nisar.l.rslc.provisional.v1"
      scenes={[sampleScene as any]}
      resultTotalCount={1}
      checkedSceneIds={new Set(["aria-scene-1"])}
      selectedSceneId="aria-scene-1"
      onOpenNisarSubsetWorkspace={() => {}}
      nisarSubsetDisabledReason="AOI 在线子集裁剪当前仅支持 NISAR L-Band RSLC"
      taskOutcomeNotice={{ tone: "neutral", text: "Copernicus DEM GLO-30 检索完成" }}
    />
  );

  const sceneCountEl = view.getByText("1 景");
  const checkedCountEl = view.getByText(/已勾选 1/);
  const noticeSpan = view.getByText("Copernicus DEM GLO-30 检索完成");
  assert.ok(sceneCountEl.className.includes("whitespace-nowrap"));
  assert.ok(checkedCountEl.className.includes("whitespace-nowrap"));
  assert.ok(noticeSpan.className.includes("break-words"));
  assert.equal(noticeSpan.className.includes("break-all"), false);

  const subsetBtn = view.getByTestId("btn-nisar-subset-download") as HTMLButtonElement;
  assert.equal(subsetBtn.disabled, true);

  const scrollBox = view.getByTestId("results-scroll-container");
  scrollBox.scrollTop = 120;
  view.rerender(
    <Harness
      activeResourceId="science.aria.s1-gunw.v1"
      scenes={[{ ...sampleScene, scene_id: "aria-scene-2", product_key: "aria-scene-2" } as any]}
      resultTotalCount={1}
      checkedSceneIds={new Set()}
      selectedSceneId="aria-scene-2"
    />
  );
  assert.equal(view.getByTestId("results-scroll-container").scrollTop, 0, "new batch must reset results scrollTop to 0 so Row 1 is never clipped");
  view.unmount();
});

test("GEE scientific mode hides stale NISAR subset and SAR stack grouping controls in results header", () => {
  const view = render(
    <Harness
      scientificMode="gee"
      activeResourceId="science.nisar.l.rslc.provisional.v1"
      scenes={[]}
      resultTotalCount={0}
      checkedSceneIds={new Set()}
      selectedSceneId={null}
      onOpenNisarSubsetWorkspace={() => {}}
    />
  );
  assert.equal(view.queryByTestId("btn-nisar-subset-download"), null);
  assert.equal(view.queryByTestId("toggle-sar-stack-grouping"), null);
  view.unmount();
});

test("Issue #40: narrow viewport (788px container / 920x620 window) auto-collapses search without polluting localStorage and wraps sumbar/sorthead", async () => {
  const originalWidth = window.innerWidth;
  Object.defineProperty(window, "innerWidth", { configurable: true, value: 920 });
  try {
    const view = render(<Harness width={788} />);
    await waitFor(() => {
      assert.equal(view.getByTestId("manual-workbench").getAttribute("data-search-collapsed"), "true");
      assert.equal(view.getByTestId("manual-workbench").getAttribute("data-results-collapsed"), "false");
    });
    assert.ok(view.getByRole("region", { name: "地图" }));
    assert.ok(view.getByRole("region", { name: "结果" }));
    assert.equal(window.localStorage.getItem(MANUAL_COLUMNS_STORAGE_KEY), null, "narrow auto-collapse must not pollute localStorage");

    // Expanding search in narrow mode mutually excludes results
    fireEvent.click(view.getByTestId("expand-search-column-btn"));
    await waitFor(() => {
      assert.equal(view.getByTestId("manual-workbench").getAttribute("data-search-collapsed"), "false");
      assert.equal(view.getByTestId("manual-workbench").getAttribute("data-results-collapsed"), "true");
    });
    assert.equal(window.localStorage.getItem(MANUAL_COLUMNS_STORAGE_KEY), null, "narrow expand mutual exclusion must not pollute localStorage");

    // Triggering search in narrow mode expands results and collapses search
    fireEvent.click(view.getByTestId("btn-start-search"));
    await waitFor(() => {
      assert.equal(view.getByTestId("manual-workbench").getAttribute("data-search-collapsed"), "true");
      assert.equal(view.getByTestId("manual-workbench").getAttribute("data-results-collapsed"), "false");
    });

    // Widening back >= 960px restores 3-column layout
    view.rerender(<Harness width={1280} />);
    await waitFor(() => {
      assert.equal(view.getByTestId("manual-workbench").getAttribute("data-search-collapsed"), "false");
      assert.equal(view.getByTestId("manual-workbench").getAttribute("data-results-collapsed"), "false");
    });

    // Verify sumbar & sorthead wrap rather than clip
    const addQueueBtn = view.getByRole("button", { name: /加入下载队列/ });
    const sumbar = addQueueBtn.closest("div.flex-wrap");
    assert.ok(sumbar, "sumbar must allow wrapping on narrow width");
    view.unmount();
  } finally {
    Object.defineProperty(window, "innerWidth", { configurable: true, value: originalWidth });
  }
});

test("Issue #42: DEM planning mode empty state shows DEM planning copy instead of image search copy", () => {
  const view = render(
    <Harness
      scientificMode="dem"
      activeResourceId="elevation.copernicus.glo30.v1"
      availableSources={[
        {
          resource_id: "elevation.copernicus.glo30.v1",
          display_name_zh: "Copernicus DEM GLO-30",
          family: "dem",
        } as ManualWorkbenchProps["availableSources"][number],
      ]}
      frozenSearchRequest={null}
      scenes={[]}
      resultTotalCount={0}
      workingAoi={null}
      workingGeometry={null}
      aoiLabel=""
    />
  );
  assert.ok(view.getByText("尚未开始 DEM 规划"));
  assert.ok(view.queryByText("尚未开始影像检索") === null);

  view.rerender(
    <Harness
      scientificMode="dem"
      activeResourceId="elevation.copernicus.glo30.v1"
      availableSources={[
        {
          resource_id: "elevation.copernicus.glo30.v1",
          display_name_zh: "Copernicus DEM GLO-30",
          family: "dem",
        } as ManualWorkbenchProps["availableSources"][number],
      ]}
      frozenSearchRequest={null}
      scenes={[]}
      resultTotalCount={0}
    />
  );
  assert.ok(view.getByText(/确认左侧 DEM 数据源与高程基准设置后/));
  assert.ok(view.queryByTestId("toggle-sar-stack-grouping") === null);
  const resultsPane = view.container.querySelector('[data-column="results"]') as HTMLElement;
  assert.ok(within(resultsPane).getByRole("button", { name: "规划 DEM" }));
  view.unmount();
});

test("stale restored search batch (>24h) shows refresh banner and disables refresh without AOI", () => {
  let refreshCalls = 0;
  const staleIso = new Date(Date.now() - 3 * 24 * 60 * 60 * 1000).toISOString();
  const staleView = {
    resource_id: "isro.nrsc.bhoonidhi.nisar.s.ssar.rslc.v1",
    batch_id: "batch-stale-1",
    searched_at: staleIso,
    item_count: 1,
  } as ManualWorkbenchProps["activeSourceView"];

  const view = render(
    <Harness
      activeSourceView={staleView}
      onRefreshSearch={() => {
        refreshCalls += 1;
      }}
    />,
  );
  const banner = view.getByTestId("stale-batch-banner");
  assert.ok(banner.textContent?.includes("3 天前"));
  const refreshBtn = view.getByTestId("stale-batch-refresh-btn") as HTMLButtonElement;
  assert.equal(refreshBtn.disabled, false);
  fireEvent.click(refreshBtn);
  assert.equal(refreshCalls, 1);

  // Disabled when workingAoi is null
  view.rerender(
    <Harness
      activeSourceView={staleView}
      workingAoi={null}
      onRefreshSearch={() => {
        refreshCalls += 1;
      }}
    />,
  );
  assert.equal((view.getByTestId("stale-batch-refresh-btn") as HTMLButtonElement).disabled, true);

  // Hidden when batch is fresh (<24h)
  const freshView = {
    ...staleView,
    searched_at: new Date(Date.now() - 2 * 60 * 60 * 1000).toISOString(),
  } as ManualWorkbenchProps["activeSourceView"];
  view.rerender(<Harness activeSourceView={freshView} />);
  assert.ok(view.queryByTestId("stale-batch-banner") === null);
  view.unmount();
});
