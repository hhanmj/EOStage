import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React, { useState } from "react";
import { act, waitFor } from "@testing-library/react";
import { cleanup, fireEvent, render } from "../../test-utils/render-harness";
import { AdminRegionSearch } from "./AdminRegionSearch";
import type { AdminBoundary, AdminBoundaryResult, Bbox } from "../../lib/bridge";

const oldGeometry = {
  type: "Polygon",
  coordinates: [[[110, 30], [111, 30], [111, 31], [110, 31], [110, 30]]],
};
const oldBbox: Bbox = { west: 110, south: 30, east: 111, north: 31, crs: "EPSG:4326" };
const newGeometry = {
  type: "MultiPolygon",
  coordinates: [[[[116, 39], [117, 39], [117, 40], [116, 40], [116, 39]]]],
};
const candidate: AdminBoundary = {
  label: "朝阳区",
  display_name: "朝阳区, 北京市",
  admin_level: "区县",
  provider: "nominatim",
  provider_object_id: "relation:1",
  bbox: { west: 116, south: 39, east: 117, north: 40, crs: "EPSG:4326" },
};
const ok = (results: AdminBoundary[]): AdminBoundaryResult => ({ ok: true, query: "", results });
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

function Harness({ services, onApplied }: {
  services: {
    search: (query: string, limit: number) => Promise<AdminBoundaryResult>;
    resolve: (value: AdminBoundary) => Promise<AdminBoundary>;
  };
  onApplied?: (geometry: Record<string, unknown> | null, label: string) => void;
}) {
  const [aoi, setAoi] = useState({ geometry: oldGeometry, bbox: oldBbox, label: "武汉市" });
  const [preview, setPreview] = useState<Record<string, unknown> | null>(null);
  return <>
    <AdminRegionSearch
      workingAoi={aoi.bbox}
      workingGeometry={aoi.geometry}
      aoiLabel={aoi.label}
      areaText="8,569 km²"
      services={services}
      onPreviewGeometry={setPreview}
      onReplaceAoi={async (geometry, label, tuple) => {
        onApplied?.(geometry, label);
        setAoi(tuple ? {
          geometry: geometry!,
          bbox: { west: tuple[0], south: tuple[1], east: tuple[2], north: tuple[3], crs: "EPSG:4326" },
          label,
        } : { geometry: null!, bbox: null!, label });
      }}
    />
    <output data-testid="preview">{preview ? "虚线预览" : "无预览"}</output>
    <output data-testid="aoi">{aoi.label}</output>
  </>;
}

test.afterEach(() => cleanup());

test("IME never searches; composition end debounces 400ms and Enter searches immediately", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const calls: string[] = [];
  const view = render(<Harness services={{
    search: async (query) => { calls.push(query); return ok([]); },
    resolve: async () => candidate,
  }} />);
  const input = view.getByRole("textbox", { name: "空间范围" });
  fireEvent.compositionStart(input);
  fireEvent.change(input, { target: { value: "朝" } });
  await act(async () => { t.mock.timers.tick(500); });
  assert.deepEqual(calls, []);
  fireEvent.change(input, { target: { value: "朝阳区" } });
  fireEvent.compositionEnd(input);
  await act(async () => { t.mock.timers.tick(399); });
  assert.deepEqual(calls, []);
  await act(async () => { t.mock.timers.tick(1); });
  assert.deepEqual(calls, ["朝阳区"]);
  fireEvent.change(input, { target: { value: "北京市" } });
  fireEvent.keyDown(input, { key: "Enter" });
  await act(async () => {});
  assert.deepEqual(calls, ["朝阳区", "北京市"]);
  fireEvent.compositionStart(input);
  fireEvent.keyDown(input, { key: "Enter", isComposing: true, keyCode: 229 });
  assert.deepEqual(calls, ["朝阳区", "北京市"]);
  t.mock.timers.reset();
});

test("at most eight full-path candidates; hover lazy-loads and caches boundary preview", async () => {
  const many = Array.from({ length: 10 }, (_, index) => ({
    ...candidate,
    display_name: `朝阳区${index + 1}, 北京市`,
    provider_object_id: `relation:${index + 1}`,
  }));
  let searches = 0;
  let resolves = 0;
  let replacements = 0;
  const view = render(<Harness
    onApplied={() => { replacements += 1; }}
    services={{
      search: async () => { searches += 1; return ok(many); },
      resolve: async (value) => { resolves += 1; return { ...value, geometry: newGeometry }; },
    }}
  />);
  const input = view.getByRole("textbox", { name: "空间范围" });
  fireEvent.focus(input);
  fireEvent.change(input, { target: { value: "朝阳区" } });
  fireEvent.keyDown(input, { key: "Enter" });
  const options = await view.findAllByRole("option");
  assert.equal(options.length, 8);
  assert.equal(options[0].textContent, "朝阳区1 · 北京市 · 区县");
  assert.equal(replacements, 0);
  fireEvent.mouseEnter(options[0]);
  await view.findByText("虚线预览");
  assert.equal(resolves, 1);
  assert.equal(replacements, 0);
  fireEvent.mouseEnter(options[0]);
  await act(async () => {});
  assert.equal(resolves, 1);
  fireEvent.keyDown(input, { key: "Enter" });
  await act(async () => {});
  assert.equal(searches, 1, "repeated query uses cache");
});

test("click replaces AOI and undo restores previous geometry, label, and bbox", async () => {
  const binds: Array<{ geometry: Record<string, unknown> | null; label: string }> = [];
  const view = render(<Harness
    onApplied={(geometry, label) => binds.push({ geometry, label })}
    services={{
      search: async () => ok([candidate]),
      resolve: async (value) => ({ ...value, geometry: newGeometry }),
    }}
  />);
  assert.equal((view.getByRole("textbox", { name: "空间范围" }) as HTMLInputElement).value, "武汉市 · 8,569 km²");
  const input = view.getByRole("textbox", { name: "空间范围" });
  fireEvent.focus(input);
  fireEvent.change(input, { target: { value: "朝阳区" } });
  fireEvent.keyDown(input, { key: "Enter" });
  const option = await view.findByRole("option");
  fireEvent.mouseEnter(option);
  await view.findByText("虚线预览");
  fireEvent.click(option);
  const statusEl = await view.findByText("已替换范围");
  assert.ok(
    statusEl.className.includes("sr-only"),
    "Item 5: replaced notice must be sr-only and must not add a second top row above the search bar",
  );
  assert.equal(view.getByTestId("aoi").textContent, "朝阳区, 北京市");
  assert.equal(binds[0].geometry, newGeometry);
  const undoBtn = view.getByTestId("aoi-undo-replace-btn");
  assert.equal(undoBtn.getAttribute("aria-label"), "撤销");
  fireEvent.click(undoBtn);
  await view.findByText("已恢复范围");
  assert.equal(view.queryByTestId("aoi-undo-replace-btn"), null);
  assert.equal(view.getByTestId("aoi").textContent, "武汉市");
  assert.equal(binds[1].geometry, oldGeometry);
  assert.equal(binds[1].label, "武汉市");
});

test("late responses cannot replace newer candidates", async () => {
  const oldRequest = deferred<AdminBoundaryResult>();
  const newRequest = deferred<AdminBoundaryResult>();
  const beijing = { ...candidate, label: "北京市", display_name: "北京市", provider_object_id: "relation:2" };
  const view = render(<Harness services={{
    search: (query) => query === "朝阳区" ? oldRequest.promise : newRequest.promise,
    resolve: async () => candidate,
  }} />);
  const input = view.getByRole("textbox", { name: "空间范围" });
  fireEvent.focus(input);
  fireEvent.change(input, { target: { value: "朝阳区" } });
  fireEvent.keyDown(input, { key: "Enter" });
  fireEvent.change(input, { target: { value: "北京市" } });
  fireEvent.keyDown(input, { key: "Enter" });
  await act(async () => { newRequest.resolve(ok([beijing])); });
  await view.findByRole("option");
  await act(async () => { oldRequest.resolve(ok([candidate])); });
  await waitFor(() => assert.equal(view.getAllByRole("option").length, 1));
  assert.equal(view.getByRole("option").textContent, "北京市 · 区县");
});

test("clicking magnifying glass icon opens AdminBoundaryModal", async () => {
  const view = render(<Harness services={{
    search: async () => ok([candidate]),
    resolve: async (value) => ({ ...value, geometry: newGeometry }),
  }} />);
  const searchIconBtn = view.getByTestId("aoi-search-icon");
  assert.equal(searchIconBtn.tagName, "BUTTON");
  fireEvent.click(searchIconBtn);
  const dialog = await view.findByRole("dialog");
  assert.ok(dialog);
});
