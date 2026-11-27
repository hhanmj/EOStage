import assert from "node:assert/strict";
import test from "node:test";

import React from "react";

import { projectCatalogRows, type PreviewRuntimeState } from "@/lib/catalogPreview";
import type { SarProduct } from "@/lib/generated/v3-contracts";
import { cleanup, fireEvent, render } from "../test-utils/render-harness";
import { CatalogWorkbenchDock, type CatalogWorkbenchDockProps } from "./CatalogWorkbenchDock";

test.afterEach(() => cleanup());

function product(kind: "landsat" | "sentinel2" | "sar"): SarProduct {
  const optical = kind !== "sar";
  const provider = kind === "sentinel2" ? "copernicus.dataspace.odata" : kind === "landsat" ? "microsoft.planetary-computer" : "asf";
  return {
    product_key: `${kind}-fixture`, product_id: `${kind}-fixture`, scene_id: `${kind}-fixture`,
    provider, dataset: kind === "sentinel2" ? "SENTINEL-2 L2A" : kind === "landsat" ? "LANDSAT C2 L2" : "SENTINEL-1",
    platform: null, instrument: null, processing_level: null, product_family: "fixture",
    beam_mode: null, beam_swath: null, polarization: null, flight_direction: null, look_direction: null,
    relative_orbit: null, absolute_orbit: null, frame: null, asf_frame: null, path: null,
    acquisition_start: null, acquisition_stop: null, footprint_geojson: null, footprint_bbox: null,
    download_url: null, file_name: null, file_size_remote: null, checksum: null, local_path: null, burst: null,
    native_metadata: optical ? {
      logical_dataset: kind === "sentinel2" ? "optical.sentinel-2.l2a" : "optical.landsat.c2.l2",
      native_collection_id: kind === "sentinel2" ? "SENTINEL-2" : "landsat-c2-l2",
      source_kind: kind, product_kind: "optical_scene", properties: {},
    } : {},
    assets: [],
  };
}

function props(item: SarProduct, overrides: Partial<CatalogWorkbenchDockProps> = {}): CatalogWorkbenchDockProps {
  const preview: PreviewRuntimeState = {
    product_key: item.product_key, status: "loading", descriptor: null, url: null, message: "",
  };
  const noop = () => undefined;
  return {
    products: [item], rows: projectCatalogRows([item], new Set(), item.product_key),
    selectedProductKeys: new Set(), activeDownloadProductKeys: new Set(), pausedDownloadProductKeys: new Set(),
    taskProductKeys: new Set(), demTasks: [], preview, activePreviewProductKey: item.product_key,
    previewProductLabel: item.product_id, filterQuery: "", pageIndex: 0, pageSize: 100,
    preferences: { tab: "results", context: "catalog", heightMode: "normal", heightPx: 300 },
    onPreferencesChange: noop, onFilterQueryChange: noop, onPageIndexChange: noop,
    onActivateProduct: noop, onToggleProduct: noop, onShowProductInfo: noop, onHideProductInfo: noop,
    onCopyProductInfo: noop, onPreviewProduct: noop, onPauseProduct: noop, onResumeProduct: noop,
    onDownloadProduct: noop, onSelectAll: noop, onClearSelection: noop, onDownloadSelected: noop,
    onClosePreview: noop, onCancelPreview: noop, onSavePreview: noop, onOpenPreviewSaveDirectory: noop,
    onPauseSelected: noop, onResumeSelected: noop, ...overrides,
  };
}

for (const kind of ["landsat", "sentinel2"] as const) {
  test(`${kind} preview remains map-only while loading, failing, and becoming available`, () => {
    const item = product(kind);
    const initial = props(item);
    assert.equal(initial.rows[0]?.kind, "optical", "the fixture must enter the real optical projection");
    const view = render(<CatalogWorkbenchDock {...initial} />);
    for (const status of ["loading", "failed", "available", "cancelled"] as const) {
      const preview = { ...initial.preview, status, message: "fixture preview status" };
      view.rerender(<CatalogWorkbenchDock {...initial} preview={preview} />);
      assert.ok(view.queryByLabelText("当前产品预览") === null, "optical preview must not open a second viewer");
      view.rerender(<CatalogWorkbenchDock {...initial} rows={[]} filterQuery="not visible" preview={preview} />);
      assert.ok(view.queryByLabelText("当前产品预览") === null, "filtering must not restore the duplicate viewer");
    }
  });
}

test("catalog row actions keep one shared preview-metadata-details order", () => {
  const item = product("landsat");
  const calls: string[] = [];
  const initial = props(item, {
    assetDetailProductKeys: new Set([item.product_key]),
    onPreviewProduct: (key) => calls.push(`preview:${key}`),
    onActivateProduct: (key) => calls.push(`activate:${key}`),
    onToggleProduct: (key) => calls.push(`select:${key}`),
    onCopyProductInfo: (key) => calls.push(`metadata:${key}`),
    onAssetDetailsProduct: (key) => calls.push(`assets:${key}`),
  });
  const view = render(<CatalogWorkbenchDock {...initial} />);
  const preview = view.getByRole("button", { name: `定位并预览 ${item.product_id}` });
  const metadata = view.getByRole("button", { name: `查看并复制 ${item.product_id} 元数据` });
  const details = view.getByRole("button", { name: `查看 ${item.product_id} 波段与资产详情` });
  assert.ok(preview.compareDocumentPosition(metadata) & Node.DOCUMENT_POSITION_FOLLOWING);
  assert.ok(metadata.compareDocumentPosition(details) & Node.DOCUMENT_POSITION_FOLLOWING);

  fireEvent.click(preview);
  fireEvent.click(view.getByRole("checkbox", { name: `选择下载 ${item.product_id}` }));
  fireEvent.click(metadata);
  fireEvent.click(details);
  assert.deepEqual(calls, [
    `preview:${item.product_key}`,
    `select:${item.product_key}`,
    `metadata:${item.product_key}`,
    `assets:${item.product_key}`,
  ]);

  const sentinel = product("sentinel2");
  const sentinelView = render(<CatalogWorkbenchDock {...props(sentinel)} />);
  const sentinelPreview = sentinelView.getByRole("button", { name: `定位并预览 ${sentinel.product_id}` });
  const sentinelMetadata = sentinelView.getByRole("button", { name: `查看并复制 ${sentinel.product_id} 元数据` });
  assert.ok(sentinelPreview.compareDocumentPosition(sentinelMetadata) & Node.DOCUMENT_POSITION_FOLLOWING);
});

test("metadata intent follows only visible rows and stays stable when their sizes change", () => {
  const products = Array.from({ length: 100 }, (_, index) => ({ ...product("landsat"),
    product_key: `landsat-${index}`, product_id: `landsat-${index}`, scene_id: `landsat-${index}` }));
  const changes: string[][] = [];
  const initial = props(products[0], { products, rows: projectCatalogRows(products, new Set(), null),
    activePreviewProductKey: null, boundedPageMode: true,
    onVisibleProductKeysChange: keys => changes.push([...keys]) });
  const view = render(<CatalogWorkbenchDock {...initial} />);
  const first = changes.at(-1)!;
  assert.ok(first.length > 0 && first.length < products.length);
  const before = changes.length;
  view.rerender(<CatalogWorkbenchDock {...initial} rows={initial.rows.map(row => ({ ...row, file_size_label: "3.0 GB" }))} />);
  assert.equal(changes.length, before, "a returned size must not restart viewport requests");
  const scroller = view.container.querySelector(".overflow-auto") as HTMLDivElement;
  fireEvent.scroll(scroller, { target: { scrollTop: 72 * 40 } });
  assert.ok(changes.at(-1)!.length < 64);
  assert.ok(!changes.at(-1)!.includes(first[0]), "off-screen products must leave the requested set");
  view.rerender(<CatalogWorkbenchDock {...initial} searchPending />);
  assert.deepEqual(changes.at(-1), []);
  view.rerender(<CatalogWorkbenchDock {...initial} preferences={{ ...initial.preferences, heightMode: "collapsed" }} />);
  assert.deepEqual(changes.at(-1), []);
  view.unmount();
  assert.deepEqual(changes.at(-1), []);
});

test("SAR preview stays map-only and never restores a duplicate bottom-dock viewer", () => {
  const sar = product("sar");
  const initial = props(sar);
  const view = render(<CatalogWorkbenchDock {...initial} />);
  for (const status of ["loading", "failed", "available", "cancelled"] as const) {
    view.rerender(<CatalogWorkbenchDock {...initial} preview={{ ...initial.preview, status }} />);
    assert.equal(view.queryByLabelText("当前产品预览"), null);
    assert.equal(view.queryByRole("button", { name: "重试预览" }), null);
  }
});
