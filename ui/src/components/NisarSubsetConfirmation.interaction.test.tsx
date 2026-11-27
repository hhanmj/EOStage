import assert from "node:assert/strict";
import test from "node:test";

import { JSDOM } from "jsdom";
import React, { act } from "react";

import { NisarSubsetConfirmation } from "./NisarSubsetConfirmation";
import {
  BHOONIDHI_COLLECTION_ID, BHOONIDHI_PROFILE_ID, BHOONIDHI_PROVIDER_ID,
} from "@/lib/bhoonidhiSsarCatalog";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://127.0.0.1/",
});
Object.assign(globalThis, {
  window: dom.window,
  document: dom.window.document,
  HTMLElement: dom.window.HTMLElement,
  Node: dom.window.Node,
  MutationObserver: dom.window.MutationObserver,
  getComputedStyle: dom.window.getComputedStyle,
  IS_REACT_ACT_ENVIRONMENT: true,
});
Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  value: dom.window.navigator,
});

const { cleanup, fireEvent, render, screen } = await import("@testing-library/react");
test.afterEach(() => cleanup());

const geometry = {
  type: "Polygon",
  coordinates: [[[110, 30], [111, 30], [111, 31], [110, 30]]],
};

const effectiveGeometry = {
  crs: "EPSG:4326",
  canonical_geojson: geometry,
  component_count: 1,
  ring_count: 1,
  vertex_count: 4,
};

const prepared = {
  source_batch_id: "nisar-batch-2026-09",
  workbench_revision: 12,
  intents: [
    {
      source_batch_id: "nisar-batch-2026-09",
      source_item_id: "NISAR_L1_RSLC_SCENE_001",
      workbench_revision: 12,
      intent: {
        selected_frequency: "A",
        selected_polarizations: ["HH", "HV"],
        requested_geometry: geometry,
        effective_geometry: effectiveGeometry,
        output_authority: "client_derived",
        quality_status: "PROVISIONAL",
      },
    },
    {
      source_batch_id: "nisar-batch-2026-09",
      source_item_id: "NISAR_L1_RSLC_SCENE_002",
      workbench_revision: 12,
      intent: {
        selected_frequency: "A",
        selected_polarizations: ["HH", "HV"],
        requested_geometry: geometry,
        effective_geometry: effectiveGeometry,
        output_authority: "client_derived",
        quality_status: "PROVISIONAL",
      },
    },
  ],
  excluded_items: [
    { source_item_id: "NISAR_L1_RSLC_SCENE_NO_OVERLAP", reason: "no_overlap" },
  ],
};

function renderConfirmation(
  overrides: Partial<React.ComponentProps<typeof NisarSubsetConfirmation>> = {},
) {
  return render(
    <NisarSubsetConfirmation
      prepared={prepared}
      outputRoot="E:/nisar-output"
      taskRequest={{
        source_batch_id: prepared.source_batch_id,
        expected_workbench_revision: prepared.workbench_revision,
        items: prepared.intents.map((item) => ({
          source_item_id: item.source_item_id,
          prepared_intent: item.intent,
        })),
        task_id: "nisar-subset-task-fixture",
        output_root: "E:/nisar-output",
        layout: "direct",
        concurrency: 2,
        created_at: "2026-09-04T00:00:00Z",
      }}
      onChooseDirectory={async () => "E:/nisar-output"}
      onCancel={() => undefined}
      onConfirm={async () => undefined}
      {...overrides}
    />,
  );
}

test("confirmation summary shows trustworthy per-item intersection, selections, destination and provenance", () => {
  renderConfirmation();
  const dialog = screen.getByRole("dialog", { name: "确认 NISAR 裁剪任务" });
  assert.match(dialog.textContent ?? "", /nisar-batch-2026-09/);
  assert.match(dialog.textContent ?? "", /2 景/);
  assert.match(dialog.textContent ?? "", /NISAR_L1_RSLC_SCENE_001/);
  assert.match(dialog.textContent ?? "", /NISAR_L1_RSLC_SCENE_001[\s\S]*已确认相交/);
  assert.match(dialog.textContent ?? "", /NISAR_L1_RSLC_SCENE_002[\s\S]*已确认相交/);
  assert.doesNotMatch(dialog.textContent ?? "", /\d+%/);
  assert.match(dialog.textContent ?? "", /Frequency A/);
  assert.match(dialog.textContent ?? "", /HH/);
  assert.match(dialog.textContent ?? "", /HV/);
  assert.match(dialog.textContent ?? "", /E:\/nisar-output/);
  assert.match(dialog.textContent ?? "", /客户端派生/);
  assert.match(dialog.textContent ?? "", /PROVISIONAL/);
  assert.match(dialog.textContent ?? "", /请求 AOI/);
  assert.match(dialog.textContent ?? "", /Polygon/);
  assert.match(dialog.textContent ?? "", /2 景使用同一请求范围/);
  assert.match(dialog.textContent ?? "", /预计数据量/);
  assert.match(dialog.textContent ?? "", /将在读取产品结构后确定/);
  assert.doesNotMatch(dialog.textContent ?? "", /coordinates/);
});

test("confirmation reports unavailable intersection when prepared intent lacks effective geometry", () => {
  renderConfirmation({
    prepared: {
      ...prepared,
      intents: [{
        ...prepared.intents[0],
        intent: {
          selected_frequency: "A",
          selected_polarizations: ["HH"],
          requested_geometry: geometry,
        },
      }],
    },
  });
  assert.ok(screen.getByText("相交状态不可用"));
  assert.equal(screen.queryByText("已确认相交"), null);
});

test("confirmation does not report intersection for a malformed effective geometry wrapper", () => {
  renderConfirmation({
    prepared: {
      ...prepared,
      intents: [{
        ...prepared.intents[0],
        intent: {
          ...prepared.intents[0].intent,
          effective_geometry: {
            ...effectiveGeometry,
            type: "Polygon",
            canonical_geojson: [],
          },
        },
      }],
    },
  });
  assert.ok(screen.getByText("相交状态不可用"));
  assert.equal(screen.queryByText("已确认相交"), null);
});

test("confirmation lists typed no-overlap exclusion with the exact conflicting image name", () => {
  renderConfirmation();
  const dialog = screen.getByRole("dialog", { name: "确认 NISAR 裁剪任务" });
  assert.match(dialog.textContent ?? "", /排除/);
  assert.match(dialog.textContent ?? "", /no_overlap/);
  assert.match(dialog.textContent ?? "", /NISAR_L1_RSLC_SCENE_NO_OVERLAP/);
});

test("cancel returns without submitting or mutating the prepared batch", () => {
  let cancelled = 0;
  const confirmed: unknown[] = [];
  const before = structuredClone(prepared);
  renderConfirmation({
    onCancel: () => { cancelled += 1; },
    onConfirm: (value: unknown) => { confirmed.push(value); },
  });
  fireEvent.click(screen.getByRole("button", { name: "取消" }));
  assert.equal(cancelled, 1);
  assert.equal(confirmed.length, 0);
  assert.deepEqual(prepared, before);
});

test("S-band confirmation uses prepared source identity and permits crop with the known range-reference difference", async () => {
  const confirmed: unknown[] = [];
  renderConfirmation({
    prepared: {
      ...prepared,
      intents: [{
        ...prepared.intents[0],
        intent: {
          ...prepared.intents[0].intent,
          source_product_identity: {
            provider_id: BHOONIDHI_PROVIDER_ID,
            collection_id: BHOONIDHI_COLLECTION_ID,
            collection_profile_id: BHOONIDHI_PROFILE_ID,
          },
        },
      }],
    },
    onConfirm: (request) => { confirmed.push(request); },
  });
  const notice = screen.getByRole("note", { name: "S-Band 已知差异" });
  assert.match(notice.textContent ?? "", /S-band · Bhoonidhi/);
  assert.match(notice.textContent ?? "", /约 9\.6 cm/);
  const button = screen.getByRole("button", { name: "确认创建裁剪任务" }) as HTMLButtonElement;
  assert.equal(button.disabled, false);
  await act(async () => { fireEvent.click(button); });
  assert.equal(confirmed.length, 1);
});

test("Frequency B alone does not identify an S-band confirmation", () => {
  renderConfirmation({
    prepared: {
      ...prepared,
      intents: [{
        ...prepared.intents[0],
        intent: { ...prepared.intents[0].intent, selected_frequency: "B" },
      }],
    },
  });
  assert.ok(screen.getByText("Frequency B"));
  assert.equal(screen.queryByRole("note", { name: "S-Band 已知差异" }), null);
});

test("B7 confirmation uses clean non-blur monochrome modal and blocks Escape/Cancel while submission is in flight", async () => {
  let cancelled = 0;
  let resolveConfirm!: () => void;
  const inFlight = new Promise<void>((resolve) => {
    resolveConfirm = resolve;
  });

  renderConfirmation({
    onCancel: () => {
      cancelled += 1;
    },
    onConfirm: () => inFlight,
  });

  const dialog = screen.getByRole("dialog", { name: "确认 NISAR 裁剪任务" });
  assert.equal(dialog.parentElement?.className.includes("backdrop-blur"), false);
  assert.equal(dialog.querySelector(".bg-sp-accent-subtle"), null);

  fireEvent.keyDown(window, { key: "Escape" });
  assert.equal(cancelled, 1);

  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "确认创建裁剪任务" }));
  });

  const cancelBtn = screen.getByRole("button", { name: "取消" }) as HTMLButtonElement;
  assert.equal(cancelBtn.disabled, true);
  fireEvent.keyDown(window, { key: "Escape" });
  fireEvent.click(cancelBtn);
  assert.equal(cancelled, 1);

  await act(async () => {
    resolveConfirm();
    await inFlight;
  });
});
