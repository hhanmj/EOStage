import assert from "node:assert/strict";
import test from "node:test";

import { JSDOM } from "jsdom";
import React, { useState } from "react";

import { SarResultLimitInput, SarSearchFilterInputs } from "./SarSearchFilterInputs";
import {
  buildSarSearchQuery,
  createSarSearchFormState,
  type SarSearchFormState,
} from "../lib/sarSearchForm";
import type { DatasetCapabilities } from "../lib/generated/v3-contracts";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://127.0.0.1/",
});
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
Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  value: dom.window.navigator,
});

const { cleanup, render } = await import("@testing-library/react");
const userEvent = (await import("@testing-library/user-event")).default;

const capabilities: DatasetCapabilities = {
  dataset: "SENTINEL-1",
  display_name: "Sentinel-1",
  platforms: ["Sentinel-1A", "Sentinel-1B"],
  instruments: ["C-SAR"],
  processing_levels: ["SLC"],
  product_families: ["SLC"],
  beam_modes: ["IW"],
  polarizations: ["VV+VH"],
  supports_flight_direction: true,
  supports_look_direction: true,
  supports_relative_orbit: true,
  supports_frame: true,
  supports_path: true,
  supports_burst: false,
  track_filter: {
    supported: true,
    parameter: "relativeOrbit",
    label: "Path / Relative Orbit",
    value_type: "integer",
    pattern: "^[0-9]+$",
    minimum: 1,
    maximum: 175,
    exact_width: null,
    platform_limits: [],
  },
  frame_filter: {
    supported: true,
    parameter: "frame",
    label: "Frame",
    value_type: "integer",
    pattern: "^[0-9]+$",
    minimum: 1,
    maximum: 10000,
    exact_width: null,
    platform_limits: [],
  },
  supported_actions: [],
};

function InteractionHarness() {
  const [form, setForm] = useState<SarSearchFormState>(() => createSarSearchFormState(capabilities));
  const [maxResults, setMaxResults] = useState("300");
  const [benignRender, setBenignRender] = useState(0);
  let query: unknown;
  try {
    query = buildSarSearchQuery(form, {
      provider: "ASF",
      start: null,
      end: null,
      bbox: null,
      aoiGeojson: null,
      maxResults: maxResults.trim() ? Number(maxResults) : null,
    });
  } catch (error) {
    query = { projection_error: error instanceof Error ? error.message : "invalid draft" };
  }
  return (
    <div data-benign-render={benignRender}>
      <SarSearchFilterInputs
        trackDescriptor={capabilities.track_filter}
        frameDescriptor={capabilities.frame_filter}
        trackRange={form.trackRange}
        frameRange={form.frameRange}
        onRangeChange={(kind, endpoint, value) => {
          setForm((current) => kind === "track"
            ? { ...current, trackRange: { ...current.trackRange, [endpoint]: value } }
            : { ...current, frameRange: { ...current.frameRange, [endpoint]: value } });
        }}
      />
      <SarResultLimitInput value={maxResults} onChange={setMaxResults} />
      <button type="button" onClick={() => setBenignRender((value) => value + 1)}>benign rerender</button>
      <output data-testid="outgoing-query">{JSON.stringify(query)}</output>
    </div>
  );
}

test("packaged-equivalent SAR inputs keep focus, React state, edits, and typed query projection", async () => {
  const user = userEvent.setup({ document: dom.window.document });
  const view = render(<InteractionHarness />);
  const pathStart = view.getByTestId("sar-path-start") as HTMLInputElement;
  const pathEnd = view.getByTestId("sar-path-end") as HTMLInputElement;
  const frameStart = view.getByTestId("sar-frame-start") as HTMLInputElement;
  const frameEnd = view.getByTestId("sar-frame-end") as HTMLInputElement;
  const imageCount = view.getByTestId("sar-max-results") as HTMLInputElement;

  await user.click(pathStart);
  await user.keyboard("118");
  assert.equal(dom.window.document.activeElement, pathStart);
  assert.equal(pathStart.value, "118");
  await user.keyboard("{Backspace}");
  assert.equal(pathStart.value, "11");
  await user.keyboard("{Control>}a{/Control}118");
  assert.equal(pathStart.value, "118");
  await user.type(pathEnd, "118");

  await user.type(frameStart, "42");
  await user.type(frameEnd, "42");
  await user.click(frameStart);
  await user.keyboard("{Control>}a{/Control}43");
  assert.equal(frameStart.value, "43");
  await user.keyboard("{Control>}a{/Control}42");

  await user.click(imageCount);
  await user.keyboard("{Control>}a{/Control}20");
  assert.equal(imageCount.value, "20");
  await user.keyboard("{Backspace}");
  assert.equal(imageCount.value, "2");
  await user.keyboard("{Control>}a{/Control}20");

  await user.click(view.getByRole("button", { name: "benign rerender" }));
  assert.equal(pathStart.value, "118");
  assert.equal(pathEnd.value, "118");
  assert.equal(frameStart.value, "42");
  assert.equal(frameEnd.value, "42");
  assert.equal(imageCount.value, "20");

  const outgoing = JSON.parse(view.getByTestId("outgoing-query").textContent ?? "null");
  assert.deepEqual(outgoing.track_range, { start: "118", end: "118" });
  assert.deepEqual(outgoing.frame_range, { start: "42", end: "42" });
  assert.equal(outgoing.max_results, 20);

  await user.click(pathStart);
  await user.keyboard("{Control>}a{/Control}{Backspace}");
  await user.click(pathEnd);
  await user.keyboard("{Control>}a{/Control}{Backspace}");
  await user.click(imageCount);
  await user.keyboard("{Control>}a{/Control}{Backspace}");
  assert.equal(pathStart.value, "");
  assert.equal(pathEnd.value, "");
  assert.equal(imageCount.value, "");
  assert.equal(imageCount.placeholder, "留空");
  assert.match(imageCount.closest("label")?.textContent ?? "", /留空默认请求 250 景；显式填写可超过 250/);
  assert.match(imageCount.title, /250 仅为单页容量/);
  const emptyOutgoing = JSON.parse(view.getByTestId("outgoing-query").textContent ?? "null");
  assert.equal(emptyOutgoing.track_range, null);
  assert.equal(emptyOutgoing.max_results, null);

  cleanup();
});

test("one populated Path and Frame endpoint dispatches the Golden scalar query", async () => {
  const user = userEvent.setup({ document: dom.window.document });
  const view = render(<InteractionHarness />);
  const pathStart = view.getByTestId("sar-path-start") as HTMLInputElement;
  const frameStart = view.getByTestId("sar-frame-start") as HTMLInputElement;
  const imageCount = view.getByTestId("sar-max-results") as HTMLInputElement;

  await user.type(pathStart, "118");
  await user.type(frameStart, "72");
  await user.click(imageCount);
  await user.keyboard("{Control>}a{/Control}30");

  assert.equal(pathStart.value, "118");
  assert.equal(frameStart.value, "72");
  assert.equal(imageCount.value, "30");
  const outgoing = JSON.parse(view.getByTestId("outgoing-query").textContent ?? "null");
  assert.equal(outgoing.relative_orbit, 118);
  assert.equal(outgoing.frame, 72);
  assert.equal(outgoing.track_range, null);
  assert.equal(outgoing.frame_range, null);
  assert.equal(outgoing.max_results, 30);

  cleanup();
});
