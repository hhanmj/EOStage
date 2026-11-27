import assert from "node:assert/strict";
import { registerHooks } from "node:module";
import test from "node:test";
import { JSDOM } from "jsdom";
import { createElement, type ReactElement } from "react";

const dom = new JSDOM("<!doctype html><html><body></body></html>", { url: "http://127.0.0.1/" });
Object.assign(globalThis, {
  window: dom.window,
  document: dom.window.document,
  HTMLElement: dom.window.HTMLElement,
  Element: dom.window.Element,
  Node: dom.window.Node,
  MutationObserver: dom.window.MutationObserver,
  getComputedStyle: dom.window.getComputedStyle,
  ResizeObserver: class { observe() {} disconnect() {} },
  requestAnimationFrame: (callback: FrameRequestCallback) => setTimeout(() => callback(Date.now()), 0),
  cancelAnimationFrame: (handle: number) => clearTimeout(handle),
  IS_REACT_ACT_ENVIRONMENT: true,
});
Object.defineProperty(globalThis, "navigator", { configurable: true, value: dom.window.navigator });
Object.assign(dom.window, {
  requestAnimationFrame: (callback: FrameRequestCallback) => setTimeout(() => callback(Date.now()), 0),
  cancelAnimationFrame: (handle: number) => clearTimeout(handle),
});

const { cleanup, render, screen } = await import("@testing-library/react");
const { I18nProvider, LOCALE_STORAGE_KEY } = await import("../src/i18n/index.ts");
const { createDownloadAcquisitionDraft } = await import("../src/lib/downloadDestinationConfirmation.ts");
const { DownloadDestinationConfirmation } = await import("../src/components/DownloadDestinationConfirmation.tsx");
const { OpticalPreparationStatus } = await import("../src/components/OpticalPreparationStatus.tsx");

type Locale = "zh-CN" | "en" | "ja";

function renderIn(locale: Locale, ui: ReactElement) {
  window.localStorage.setItem(LOCALE_STORAGE_KEY, locale);
  return render(createElement(I18nProvider, null, ui));
}

test.afterEach(() => {
  cleanup();
  window.localStorage.clear();
});

function sentinel2Draft() {
  return createDownloadAcquisitionDraft({
    sourceBatchId: "batch-language",
    sourceBatchRevision: 1,
    selectionScope: "checked",
    selectedProductIds: ["PRODUCT-A", "PRODUCT-B"],
    selectedProductCount: 2,
    acquisition: {
      kind: "sentinel2",
      frozenParameters: {
        delivery_form: "safe_archive",
        safe_archive_available: true,
        analysis_bands_available: true,
        band_preset: "true_color",
        selected_band_keys: ["B02", "B03", "B04"],
      },
    },
    destinationPrefill: "F:/downloads/default",
    layout: "sentinel2_l2a",
    taskPolicy: "manual",
    concurrency: 2,
  });
}

function confirmation() {
  return createElement(DownloadDestinationConfirmation, {
    draft: sentinel2Draft(),
    chooseDirectory: async () => null,
    onCancel: () => undefined,
    onConfirm: async () => undefined,
  });
}

const mapProps = {
  layerKey: "osm",
  tiandituToken: "",
  drawMode: "rect",
  drawActive: false,
  chrome: "panel",
  onLayerChange: () => undefined,
  onSceneSelect: () => undefined,
  onDrawModeChange: () => undefined,
  onDrawActiveChange: () => undefined,
  onClearLayers: () => undefined,
  onRectDraw: () => undefined,
  onPolygonDraw: () => undefined,
  onPointDraw: () => undefined,
};

async function withStubbedMap(suffix: string, check: (module: typeof import("../src/components/WorkbenchMap.tsx")) => void) {
  const hooks = registerHooks({
    resolve(specifier, context, nextResolve) {
      if (specifier === "react-leaflet" || specifier === "@/components/map/leafletBindings" || specifier.includes("leafletBindings")) {
        return { url: new URL("./reactLeafletBoundaryStub.tsx", import.meta.url).href, shortCircuit: true };
      }
      if (specifier === "leaflet") {
        return { url: new URL("./leafletBoundaryStub.ts", import.meta.url).href, shortCircuit: true };
      }
      return nextResolve(specifier, context);
    },
  });
  try {
    await import("./reactLeafletBoundaryStub.tsx");
    const module = await import(`../src/components/WorkbenchMap.tsx?${suffix}`);
    check(module);
  } finally {
    hooks.deregister();
  }
}

const popupScene = {
  scene_id: "S1A_IW_SLC__1SDV_LANGUAGE",
  file_name: "S1A_IW_SLC__1SDV_LANGUAGE.SAFE",
  platform: "Sentinel-1A",
  product_type: "SLC",
  beam_mode: "IW",
  polarization: "VV+VH",
  acquisition_datetime: "2024-04-08T10:01:32Z",
  absolute_orbit: 53341,
  relative_orbit: 69,
  path: 69,
  frame: 450,
  orbit_direction: "ascending",
  has_url: false,
};

test("English download-destination confirmation and preparation status labels", async () => {
  renderIn("en", confirmation());
  assert.ok(screen.getByRole("dialog", { name: "Confirm download task" }));
  assert.ok(screen.getByLabelText("Destination directory"));
  assert.ok(screen.getByRole("button", { name: "Create download task" }));
  assert.ok(screen.getByRole("button", { name: "Cancel" }));
  assert.ok(screen.getByRole("group", { name: "Delivery form" }));
  assert.ok(screen.getByText("Delivery: complete SAFE product"));
  assert.ok(screen.getByText("2 products"));
  cleanup();

  renderIn("en", createElement(OpticalPreparationStatus, { requestId: "prep-en", total: 3 }));
  assert.ok(await screen.findByText("Preparation progress is temporarily unavailable."));
  assert.equal(document.documentElement.lang, "en");
});

test("Japanese download-destination confirmation and preparation status labels", async () => {
  renderIn("ja", confirmation());
  assert.ok(screen.getByRole("dialog", { name: "ダウンロードタスクの確認" }));
  assert.ok(screen.getByLabelText("保存先ディレクトリ"));
  assert.ok(screen.getByRole("button", { name: "ダウンロードタスクを作成" }));
  assert.ok(screen.getByRole("button", { name: "キャンセル" }));
  assert.ok(screen.getByRole("group", { name: "納品形式" }));
  assert.ok(screen.getByText("納品：完全な SAFE プロダクト"));
  cleanup();

  renderIn("ja", createElement(OpticalPreparationStatus, { requestId: "prep-ja", total: 3 }));
  assert.ok(await screen.findByText("準備の進捗を一時的に読み取れません。"));
  assert.equal(document.documentElement.lang, "ja");
});

test("zh-CN download-destination confirmation keeps the original copy", () => {
  renderIn("zh-CN", confirmation());
  assert.ok(screen.getByRole("dialog", { name: "确认下载任务" }));
  assert.ok(screen.getByRole("button", { name: "创建下载任务" }));
  assert.ok(screen.getByText("交付：完整 SAFE 产品"));
});

test("English and Japanese map controls and footprint popup labels", async () => {
  await withStubbedMap("panels-language", ({ WorkbenchMap, ScenePopup }) => {
    renderIn("en", createElement(WorkbenchMap, mapProps));
    assert.ok(screen.getByRole("button", { name: "Zoom in" }));
    assert.ok(screen.getByRole("button", { name: "Zoom out" }));
    assert.ok(screen.getByRole("button", { name: "Switch basemap" }));
    cleanup();

    renderIn("en", createElement(ScenePopup, { scene: popupScene, index: 0 }));
    assert.ok(screen.getByText("Absolute orbit"));
    assert.ok(screen.getByText("Download entry"));
    assert.ok(screen.getByText("Not provided"));
    assert.ok(screen.getByText("Orbit direction"));
    cleanup();

    renderIn("ja", createElement(WorkbenchMap, mapProps));
    assert.ok(screen.getByRole("button", { name: "拡大" }));
    assert.ok(screen.getByRole("button", { name: "縮小" }));
    assert.ok(screen.getByRole("button", { name: "ベースマップを切り替え" }));
    cleanup();

    renderIn("ja", createElement(ScenePopup, { scene: popupScene, index: 0 }));
    assert.ok(screen.getByText("絶対軌道"));
    assert.ok(screen.getByText("ダウンロード入口"));
    assert.ok(screen.getByText("提供なし"));
  });
});
