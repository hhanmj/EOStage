import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { JSDOM } from "jsdom";
import { createElement } from "react";
import type { SceneFootprintPart } from "./SceneFootprintLayer.tsx";
import type { AoiOverlayPart } from "./AoiOverlay.tsx";

const dom = new JSDOM("<!doctype html><html><body></body></html>", { url: "http://127.0.0.1/" });
Object.assign(globalThis, {
  window: dom.window,
  document: dom.window.document,
  HTMLElement: dom.window.HTMLElement,
  HTMLInputElement: dom.window.HTMLInputElement,
  Node: dom.window.Node,
  MutationObserver: dom.window.MutationObserver,
  getComputedStyle: dom.window.getComputedStyle,
  IS_REACT_ACT_ENVIRONMENT: true,
});
Object.defineProperty(globalThis, "navigator", { configurable: true, value: dom.window.navigator });
class TestResizeObserver {
  observe() {}
  disconnect() {}
}
Object.assign(globalThis, { ResizeObserver: TestResizeObserver });

const { fireEvent, render, waitFor } = await import("@testing-library/react");
const { CatalogWorkbenchDock } = await import("./CatalogWorkbenchDock.tsx");
const { SceneFootprintLayer } = await import("./SceneFootprintLayer.tsx");
const { AoiOverlay } = await import("./AoiOverlay.tsx");
const { PreviewImageOverlay, computeMercatorPolygonClipPath } = await import("./PreviewImageOverlay.tsx");
const { MapContainer, ImageOverlay } = await import("./map/leafletBindings.tsx");
const { AdminBoundaryBindButton, AdminBoundarySearchButton } = await import("./AdminBoundaryActions.tsx");

const noop = () => undefined;
const emptyKeys = () => new Set<string>();
const mapCss = readFileSync(new URL("../index.css", import.meta.url), "utf8");

function mountSceneFocusRule(css: string): HTMLStyleElement {
  const rule = css.match(/\.scene-footprint:focus\s*\{[^}]*\}/s)?.[0];
  if (!rule) throw new Error("scene focus rule is missing");
  const style = document.createElement("style");
  style.textContent = rule;
  document.head.append(style);
  return style;
}
const row = {
  provider_product_key: "row-1",
  display_name: "scene-1",
  collection: "S1",
  provider: "ASF",
  platform: "S1A",
  processing_level: "SLC",
  acquisition_time: "2026-08-17T00:00:00Z",
  footprint_available: true,
  preview_status: "not_requested" as const,
  file_size_label: "",
  asset_summary: "",
  kind: "sar" as const,
  domain_field_cells: [],
  selected: true,
  active: false,
};

test("rendered dock keeps target, candidate total, loaded page, and Store selection clear without provider jargon", () => {
  const view = render(createElement(CatalogWorkbenchDock, {
    rows: [row], products: [{}] as never, selectedProductKeys: new Set(["row-1"]),
    activeDownloadProductKeys: emptyKeys(), pausedDownloadProductKeys: emptyKeys(), taskProductKeys: emptyKeys(),
    demTasks: [], preview: { status: "not_requested", product_key: null, descriptor: null, url: null, message: "" },
    activePreviewProductKey: null, previewProductLabel: null, filterQuery: "filtered", pageIndex: 0, pageSize: 100,
    boundedPageMode: true, totalCount: 40, checkedCount: 3,
    searchCounts: { targetCount: 40, providerMatchedCount: 120, providerReturnedCount: 40, frozenCandidateCount: 40 },
    sourceAttempts: [
      {
        provider_id: "microsoft.planetary-computer",
        native_collection_id: "landsat-c2-l2",
        outcome: "fallback" as const,
        fallback_reason: "transient_network" as const,
      },
      {
        provider_id: "element84.earth-search",
        native_collection_id: "landsat-c2-l2",
        outcome: "succeeded" as const,
        fallback_reason: null,
      },
    ],
    preferences: { tab: "results", heightMode: "normal", context: "catalog", heightPx: 300 },
    onPreferencesChange: noop, onFilterQueryChange: noop, onPageIndexChange: noop, onActivateProduct: noop,
    onToggleProduct: noop, onShowProductInfo: noop, onHideProductInfo: noop, onCopyProductInfo: noop,
    onPauseProduct: noop, onResumeProduct: noop, onDownloadProduct: noop, onSelectAll: noop, onClearSelection: noop,
    onDownloadSelected: noop, onClosePreview: noop, onCancelPreview: noop,
    onSavePreview: noop, onOpenPreviewSaveDirectory: noop, onPauseSelected: noop, onResumeSelected: noop,
  }));

  assert.equal(view.getByTestId("scope-target").textContent, "目标 40");
  assert.equal(view.getByTestId("scope-matched").textContent, "候选总量 120");
  assert.equal(view.getByTestId("scope-frozen").textContent, "已载入 40");
  assert.equal(view.getByTestId("scope-current").textContent, "当前显示 1");
  assert.equal(view.getByTestId("scope-selected").textContent, "已选择 3");
  assert.equal(view.getByTestId("source-attempts-summary").textContent, "来源切换 1 次");
  fireEvent.click(view.getByTestId("source-attempts-summary"));
  assert.ok(view.getByText("Microsoft Planetary Computer · landsat-c2-l2"));
  assert.ok(view.getByText("网络异常，已自动切换"));
  assert.ok(view.getByText("Element 84 Earth Search · landsat-c2-l2"));
  assert.ok(view.getByText("成功"));
  assert.equal(view.queryByTestId("scope-source"), null);

  view.rerender(createElement(CatalogWorkbenchDock, {
    rows: [row], products: [{}] as never, selectedProductKeys: emptyKeys(),
    activeDownloadProductKeys: emptyKeys(), pausedDownloadProductKeys: emptyKeys(), taskProductKeys: emptyKeys(),
    demTasks: [], preview: { status: "not_requested", product_key: null, descriptor: null, url: null, message: "" },
    activePreviewProductKey: null, previewProductLabel: null, filterQuery: "", pageIndex: 0, pageSize: 100,
    boundedPageMode: true, totalCount: 8, checkedCount: 0,
    searchCounts: { targetCount: null, providerMatchedCount: null, providerReturnedCount: null, frozenCandidateCount: 8 },
    preferences: { tab: "results", heightMode: "normal", context: "dem", heightPx: 300 },
    onPreferencesChange: noop, onFilterQueryChange: noop, onPageIndexChange: noop, onActivateProduct: noop,
    onToggleProduct: noop, onShowProductInfo: noop, onHideProductInfo: noop, onCopyProductInfo: noop,
    onPauseProduct: noop, onResumeProduct: noop, onDownloadProduct: noop, onSelectAll: noop, onClearSelection: noop,
    onDownloadSelected: noop, onClosePreview: noop, onCancelPreview: noop,
    onSavePreview: noop, onOpenPreviewSaveDirectory: noop, onPauseSelected: noop, onResumeSelected: noop,
  }));
  assert.equal(view.queryByTestId("source-attempts-summary"), null);
  assert.equal(view.queryByTestId("scope-target"), null);
  assert.equal(view.getByTestId("scope-matched").textContent, "候选总量 未知");
  // A legacy DEM dock preference must not hide catalogue rows after disconnection.
  assert.ok(view.getByRole("textbox", { name: "筛选冻结结果" }));
  assert.equal(view.queryByRole("button", { name: "DEM", exact: true }), null);
  view.unmount();
});

test("bulk selection and row activation never preview; the preview control locates and previews", () => {
  const events: string[] = [];
  const view = render(createElement(CatalogWorkbenchDock, {
    rows: [row], products: [{}] as never, selectedProductKeys: new Set(["row-1"]),
    activeDownloadProductKeys: emptyKeys(), pausedDownloadProductKeys: emptyKeys(), taskProductKeys: emptyKeys(),
    demTasks: [], preview: { status: "not_requested", product_key: null, descriptor: null, url: null, message: "" },
    activePreviewProductKey: null, previewProductLabel: null, filterQuery: "", pageIndex: 0, pageSize: 100,
    boundedPageMode: true, totalCount: 1, checkedCount: 1,
    searchCounts: { targetCount: 1, providerMatchedCount: 1, providerReturnedCount: 1, frozenCandidateCount: 1 },
    preferences: { tab: "results", heightMode: "normal", context: "catalog", heightPx: 300 },
    onPreferencesChange: noop, onFilterQueryChange: noop, onPageIndexChange: noop,
    onActivateProduct: () => events.push("activate"),
    onPreviewProduct: () => events.push("preview"),
    onToggleProduct: noop, onShowProductInfo: noop, onHideProductInfo: noop, onCopyProductInfo: noop,
    onPauseProduct: noop, onResumeProduct: noop, onDownloadProduct: noop,
    onSelectAll: () => events.push("select-all"), onClearSelection: () => events.push("clear-all"),
    onDownloadSelected: noop, onClosePreview: noop, onCancelPreview: noop,
    onSavePreview: noop, onOpenPreviewSaveDirectory: noop, onPauseSelected: noop, onResumeSelected: noop,
  }));

  fireEvent.click(view.getByRole("button", { name: "全选" }));
  fireEvent.click(view.getByRole("button", { name: "重置" }));
  assert.deepEqual(events, ["select-all", "clear-all"]);

  const resultRow = view.getByText("scene-1").closest('[role="button"]');
  if (!resultRow) throw new Error("rendered catalogue row is missing");
  fireEvent.click(resultRow);
  assert.deepEqual(events, ["select-all", "clear-all", "activate"]);

  const previewControl = view.getByRole("button", { name: "定位并预览 scene-1" });
  fireEvent.click(previewControl);
  assert.deepEqual(events, ["select-all", "clear-all", "activate", "preview"]);
  previewControl.focus();
  assert.equal(view.container.ownerDocument.activeElement, previewControl);
  assert.match(previewControl.className, /focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/);
});

test("asset detail control is explicit, isolated from row activation, and exposes loading state", () => {
  const events: string[] = [];
  const common = {
    rows: [row], products: [{}] as never, selectedProductKeys: emptyKeys(),
    activeDownloadProductKeys: emptyKeys(), pausedDownloadProductKeys: emptyKeys(), taskProductKeys: emptyKeys(),
    demTasks: [], preview: { status: "not_requested", product_key: null, descriptor: null, url: null, message: "" },
    activePreviewProductKey: null, previewProductLabel: null, filterQuery: "", pageIndex: 0, pageSize: 100,
    boundedPageMode: true, totalCount: 1, checkedCount: 0,
    searchCounts: { targetCount: 1, providerMatchedCount: 1, providerReturnedCount: 1, frozenCandidateCount: 1 },
    preferences: { tab: "results", heightMode: "normal", context: "catalog", heightPx: 300 } as const,
    onPreferencesChange: noop, onFilterQueryChange: noop, onPageIndexChange: noop,
    onActivateProduct: () => events.push("activate"), onToggleProduct: noop,
    onShowProductInfo: noop, onHideProductInfo: noop, onCopyProductInfo: noop,
    onPauseProduct: noop, onResumeProduct: noop, onDownloadProduct: noop,
    onSelectAll: noop, onClearSelection: noop, onDownloadSelected: noop,
    onClosePreview: noop, onCancelPreview: noop, onSavePreview: noop,
    onOpenPreviewSaveDirectory: noop, onPauseSelected: noop, onResumeSelected: noop,
    assetDetailProductKeys: new Set(["row-1"]),
    onAssetDetailsProduct: () => events.push("assets"),
  };
  const view = render(createElement(CatalogWorkbenchDock, common));

  const control = view.getByRole("button", { name: "查看 scene-1 波段与资产详情" });
  fireEvent.click(control);
  assert.deepEqual(events, ["assets"]);
  assert.equal(control.getAttribute("disabled"), null);

  view.rerender(createElement(CatalogWorkbenchDock, {
    ...common,
    assetDetailsLoadingProductKey: "row-1",
  }));
  assert.equal(view.getByRole("button", { name: "查看 scene-1 波段与资产详情" }).hasAttribute("disabled"), true);
  view.unmount();
});

test("rendered scene layer marks only authoritative footprints, exposes selection, and preserves click wiring", () => {
  const selected: string[] = [];
  const focusStyle = mountSceneFocusRule(mapCss);
  const scene = {
    product_key: "scene-key", scene_id: "scene-1", platform: "S1A", product_type: "SLC", beam_mode: "IW",
    polarization: "VV", acquisition_datetime: "2026-08-17T00:00:00Z", absolute_orbit: null, relative_orbit: null,
    path: null, frame: null, orbit_direction: "ASCENDING", has_url: false,
    footprint_geojson: { type: "Polygon", coordinates: [[[100, 30], [101, 30], [101, 31], [100, 31], [100, 30]]] },
    footprint_bbox: { west: 100, south: 30, east: 101, north: 31, crs: "EPSG:4326" },
  };
  const view = render(createElement("svg", {},
    createElement(SceneFootprintLayer, {
      scene, selected: true, drawActive: false, onSceneSelect: (key: string) => selected.push(key),
      renderPart: ({ className, onClick, positions, dataSceneFootprint }: SceneFootprintPart) => createElement("path", {
        tabIndex: 0, className, onClick, "data-scene-footprint": dataSceneFootprint,
        "data-testid": `scene-part-${positions.length}`,
      }),
    }),
    createElement(SceneFootprintLayer, {
      scene: { ...scene, product_key: "bbox-only", footprint_geojson: null }, selected: false, drawActive: false,
      onSceneSelect: (key: string) => selected.push(key), renderPart: () => createElement("path"),
    }),
  ));

  const footprint = view.getByTestId("scene-part-1");
  const footprintClass = footprint.getAttribute("class") ?? "";
  assert.match(footprintClass, /scene-footprint/);
  assert.match(footprintClass, /scene-footprint--selected/);
  assert.doesNotMatch(footprintClass, /leaflet-interactive/);
  assert.equal(footprint.getAttribute("data-scene-footprint"), "true");
  assert.equal(view.container.querySelectorAll(".scene-footprint").length, 1);
  footprint.focus();
  assert.equal(view.container.ownerDocument.activeElement, footprint);
  assert.equal(getComputedStyle(footprint).outline, "none");
  fireEvent.click(footprint);
  assert.deepEqual(selected, ["scene-key"]);
  view.unmount();
  focusStyle.remove();
});

test("production AOI overlay primitives stay distinct from scene markers for geometry and bbox paths", () => {
  const view = render(createElement("div", {},
    createElement(AoiOverlay, {
      geometry: { type: "Polygon", coordinates: [[[100, 30], [101, 30], [101, 31], [100, 31], [100, 30]]] },
      bbox: { west: 100, south: 30, east: 101, north: 31, crs: "EPSG:4326" },
      renderPolygon: ({ pathOptions }: AoiOverlayPart) => createElement("output", {
        className: pathOptions.className, "data-testid": "aoi-polygon", "data-color": pathOptions.color,
        "data-weight": pathOptions.weight, "data-fill": pathOptions.fillColor, "data-fill-opacity": pathOptions.fillOpacity,
      }),
      renderRectangle: ({ pathOptions }: AoiOverlayPart) => createElement("output", {
        className: pathOptions.className, "data-testid": "aoi-rectangle", "data-color": pathOptions.color,
      }),
    }),
    createElement(AoiOverlay, {
      geometry: null,
      bbox: { west: 110, south: 40, east: 111, north: 41, crs: "EPSG:4326" },
      renderPolygon: () => createElement("output"),
      renderRectangle: ({ pathOptions }: AoiOverlayPart) => createElement("output", {
        className: pathOptions.className, "data-testid": "aoi-bbox", "data-color": pathOptions.color,
      }),
    }),
    createElement(AoiOverlay, {
      geometry: null,
      bbox: { west: 120, south: 20, east: 121, north: 21, crs: "EPSG:4326" },
      variant: "smoke",
      renderPolygon: () => createElement("output"),
      renderRectangle: ({ pathOptions }: AoiOverlayPart) => createElement("output", {
        className: pathOptions.className, "data-testid": "aoi-smoke", "data-color": pathOptions.color,
        "data-weight": pathOptions.weight, "data-fill": pathOptions.fillColor, "data-fill-opacity": pathOptions.fillOpacity,
      }),
    }),
  ));

  for (const element of [view.getByTestId("aoi-polygon"), view.getByTestId("aoi-bbox")]) {
    assert.match(element.className, /aoi-overlay/);
    assert.doesNotMatch(element.className, /scene-footprint/);
    assert.equal(element.getAttribute("data-color"), "#eab308");
  }
  const smoke = view.getByTestId("aoi-smoke");
  assert.match(smoke.className, /aoi-overlay/);
  assert.doesNotMatch(smoke.className, /scene-footprint/);
  assert.equal(smoke.getAttribute("data-color"), "#eab308");
  assert.equal(smoke.getAttribute("data-weight"), "2.5");
  assert.equal(smoke.getAttribute("data-fill"), "#facc15");
  assert.equal(smoke.getAttribute("data-fill-opacity"), "0.18");
  assert.equal(view.queryByTestId("aoi-rectangle"), null);
});

test("preview image stays invisible until native image load completes, renders at opacity 1, and clips to polygon", () => {
  const rotatedFootprint = {
    type: "Polygon",
    coordinates: [[
      [100.2, 31.0],
      [101.0, 30.8],
      [100.8, 30.0],
      [100.0, 30.2],
      [100.2, 31.0],
    ]],
  };
  const view = render(createElement(PreviewImageOverlay, {
    identity: "scene-a",
    url: "https://example.invalid/preview-a.jpg",
    bounds: [[30, 100], [31, 101]],
    footprintGeojson: rotatedFootprint,
    renderImage: ({ opacity, className, clipPath, onLoad, onError }) => createElement("img", {
      alt: "preview",
      className,
      "data-opacity": String(opacity),
      "data-clip-path": clipPath ?? "",
      onLoad,
      onError,
    }),
  }));

  const image = view.getByAltText("preview");
  assert.equal(image.getAttribute("data-opacity"), "0");
  assert.match(image.className, /catalog-preview-image/);
  assert.match(image.getAttribute("data-clip-path") ?? "", /^polygon\(/);
  assert.ok(computeMercatorPolygonClipPath(rotatedFootprint, [[30, 100], [31, 101]]));
  fireEvent.load(image);
  // Owner Round-5 Item 2: map quicklook previews render at 100% opacity (1.0), never translucent 0.82.
  assert.equal(view.getByAltText("preview").getAttribute("data-opacity"), "1");

  view.rerender(createElement(PreviewImageOverlay, {
    identity: "scene-b",
    url: "https://example.invalid/preview-b.jpg",
    bounds: [[30, 100], [31, 101]],
    renderImage: ({ opacity, className, onLoad, onError }) => createElement("img", {
      alt: "preview",
      className,
      "data-opacity": String(opacity),
      onLoad,
      onError,
    }),
  }));
  assert.equal(view.getByAltText("preview").getAttribute("data-opacity"), "0");
  view.unmount();
});

test("administrative candidate search and AOI bind expose independent native button states", () => {
  let searchCalls = 0;
  let bindCalls = 0;
  const controls = (searchBusy: boolean, bindBusy: boolean) => createElement("div", {},
    createElement(AdminBoundarySearchButton, {
      busy: searchBusy,
      onSearch: () => { searchCalls += 1; },
    }),
    createElement(AdminBoundaryBindButton, {
      busy: bindBusy,
      onBind: () => { bindCalls += 1; },
    }),
  );
  const view = render(controls(false, false));
  const search = view.getByRole("button", { name: "搜索行政边界候选" });
  const bind = view.getByRole("button", { name: "加载并绑定边界" });

  fireEvent.click(search);
  assert.equal(searchCalls, 1);
  assert.equal(bindCalls, 0);

  view.rerender(controls(true, false));
  assert.equal(search.getAttribute("aria-busy"), "true");
  assert.equal(bind.getAttribute("aria-busy"), "false");
  assert.equal(bind.getAttribute("data-state"), "idle");
  assert.equal((bind as HTMLButtonElement).disabled, false);
  fireEvent.click(bind);
  assert.equal(bindCalls, 1);

  view.rerender(controls(false, true));
  assert.equal(search.getAttribute("aria-busy"), "false");
  assert.equal((search as HTMLButtonElement).disabled, false);
  assert.equal(bind.getAttribute("aria-busy"), "true");
  assert.equal(bind.getAttribute("data-state"), "binding");
});

test("ImageOverlay immediately dispatches load for already-cached complete images so PreviewImageOverlay opacity reaches 1", async () => {
  const completeDesc = Object.getOwnPropertyDescriptor(dom.window.HTMLImageElement.prototype, "complete");
  const natWidthDesc = Object.getOwnPropertyDescriptor(dom.window.HTMLImageElement.prototype, "naturalWidth");
  Object.defineProperty(dom.window.HTMLImageElement.prototype, "complete", {
    configurable: true,
    get: () => true,
  });
  Object.defineProperty(dom.window.HTMLImageElement.prototype, "naturalWidth", {
    configurable: true,
    get: () => 256,
  });

  try {
    const view = render(
      createElement(
        MapContainer,
        { center: [30.5, 100.5], zoom: 6, style: { width: 320, height: 240 } },
        createElement(PreviewImageOverlay, {
          identity: "cached-scene-1",
          url: "https://bhoonidhi.nrsc.gov.in/thumb-cached.jpg",
          bounds: [[30, 100], [31, 101]],
          renderImage: ({ url, bounds, opacity, className, clipPath, onLoad, onError }) =>
            createElement(ImageOverlay, {
              url,
              bounds,
              opacity,
              className,
              clipPath,
              eventHandlers: { load: onLoad, error: onError },
            }),
        }),
      ),
    );

    const imgEl = await waitFor(() => {
      const node = view.container.querySelector("img.catalog-preview-image") as HTMLImageElement | null;
      assert.ok(node !== null);
      assert.equal(node.style.opacity, "1");
      return node;
    });
    assert.equal(imgEl.style.opacity, "1");
    view.unmount();
  } finally {
    if (completeDesc) Object.defineProperty(dom.window.HTMLImageElement.prototype, "complete", completeDesc);
    if (natWidthDesc) Object.defineProperty(dom.window.HTMLImageElement.prototype, "naturalWidth", natWidthDesc);
  }
});
