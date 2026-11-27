import assert from "node:assert/strict";
import test from "node:test";

import { JSDOM } from "jsdom";
import { createElement } from "react";

const dom = new JSDOM("<!doctype html><html><body></body></html>", { url: "http://127.0.0.1/" });
Object.assign(globalThis, {
  window: dom.window,
  document: dom.window.document,
  HTMLElement: dom.window.HTMLElement,
  HTMLButtonElement: dom.window.HTMLButtonElement,
  Node: dom.window.Node,
  MutationObserver: dom.window.MutationObserver,
  getComputedStyle: dom.window.getComputedStyle,
  IS_REACT_ACT_ENVIRONMENT: true,
});
Object.defineProperty(globalThis, "navigator", { configurable: true, value: dom.window.navigator });

const { fireEvent, render } = await import("@testing-library/react");
const { AoiCandidatePanel } = await import("../src/components/AoiCandidatePanel.tsx");

test("AOI bind action uses the approved label and remains a keyboard button", () => {
  const calls: string[] = [];
  const candidate = {
    label: "候选 1",
    bbox: { west: 100, east: 101, south: 30, north: 31, crs: "EPSG:4326" },
    boundary_id: "boundary-1",
    source: "test-provider",
  };
  const view = render(createElement(AoiCandidatePanel, {
    candidates: [candidate],
    selected: candidate,
    detailBusy: false,
    bindBusy: false,
    onPreview: () => undefined,
    onBind: (boundary) => calls.push(boundary.label),
  }));

  const button = view.getByRole("button", { name: "设置为AOI" });
  assert.equal(button.textContent?.replace(/\s+/g, "").includes("设置为AOI"), true);
  assert.equal((button as HTMLButtonElement).type, "button");
  button.focus();
  assert.equal(view.container.ownerDocument.activeElement, button);
  fireEvent.keyDown(button, { key: "Enter" });
  fireEvent.click(button);
  assert.deepEqual(calls, ["候选 1"]);
  view.unmount();
});

test("AOI candidates scroll independently while the bind footer stays reachable", () => {
  const previews: string[] = [];
  const binds: string[] = [];
  const results = Array.from({ length: 12 }, (_, index) => ({
    label: `候选 ${index + 1}`,
    bbox: { west: 100 + index, east: 101 + index, south: 30, north: 31, crs: "EPSG:4326" },
    boundary_id: `boundary-${index + 1}`,
    source: "test-provider",
  }));
  const view = render(createElement(AoiCandidatePanel, {
    candidates: results,
    selected: results[11],
    detailBusy: false,
    bindBusy: false,
    onPreview: (boundary) => previews.push(boundary.label),
    onBind: () => binds.push("bind"),
  }));

  const scroll = view.getByTestId("aoi-candidate-scroll");
  const footer = view.getByTestId("aoi-candidate-footer");
  assert.match(scroll.className, /max-h-36/);
  assert.match(scroll.className, /overflow-y-auto/);
  assert.equal(scroll.contains(footer), false);

  const candidate = view.getByRole("option", { name: /候选 12/ });
  candidate.focus();
  assert.equal(view.container.ownerDocument.activeElement, candidate);
  fireEvent.click(candidate);
  fireEvent.click(view.getByRole("button", { name: "设置为AOI" }));
  assert.deepEqual(previews, ["候选 12"]);
  assert.deepEqual(binds, ["bind"]);
  view.unmount();
});
