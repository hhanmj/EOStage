import assert from "node:assert/strict";
import test from "node:test";
import React from "react";

import { cleanup, render } from "../src/test-utils/render-harness";
import { loadApplicationTaskEventPage, loadApplicationTaskPage } from "../src/lib/bridge";
import { clearTaskDetailTestBridge, registerTaskDetailTestBridge } from "../src/lib/taskDetailTestBridge";
import { projectFrozenItemToScene } from "../src/lib/applicationServiceProjection";
import { projectTaskPagePresentation } from "../src/lib/taskPagePresentation";
import { TaskPageDetail } from "../src/components/TaskPageDetail";

const sourceKey = "PROVIDER::microsoft.planetary-computer::COLLECTION::landsat-c2-l2::ITEM::LC08_L2SP_127036_20250101_02_T1";
const fileName = "LC08_L2SP_127036_20250101_20250111_02_T1_SR_B1.TIF";

function filePage(status: "active" | "completed") {
  return {
    task_id: "landsat-files", revision: 8, page_index: 0, page_size: 100, total_count: 22,
    items: [{
      ordinal: 0, checked: true,
      item: {
        item_id: `${sourceKey}::ASSET::coastal`,
        provider_id: "microsoft.planetary-computer", display_name: fileName,
        acquisition_datetime: "2025-01-01T00:00:00Z", footprint_geojson: null,
        metadata: {
          logical_dataset: "optical.landsat.c2.l2", source_kind: "landsat", product_kind: "optical_scene",
          native_collection_id: "landsat-c2-l2", provider_product_key: sourceKey,
          source_item_id: sourceKey, delivery_asset_key: "coastal", delivery_asset_role: "primary",
          assets: [], properties: {},
        },
        payload: {}, asset_access: { expected_size_bytes: 1000 }, execution_policy: null,
      },
      status, bytes_completed: status === "completed" ? 1000 : 200, total_bytes: 1000,
      bytes_per_second: status === "active" ? 20 : 0, attempt: 1,
      destination: status === "completed" ? `C:/isolated/${fileName}` : null,
      failure: null, finished_at: status === "completed" ? "2025-01-01T00:01:00Z" : null,
      retryable: false,
    }],
  };
}

function install(response: (command: string) => unknown) {
  registerTaskDetailTestBridge({
    invoke: async (command) => response(command),
    getCurrentWindow: () => ({
      minimize: async () => undefined, toggleMaximize: async () => undefined,
      close: async () => undefined, innerSize: async () => ({ width: 100, height: 100 }),
      startResizeDragging: async () => undefined,
    }),
    open: async () => null,
  });
}

test.afterEach(() => { cleanup(); clearTaskDetailTestBridge(); });

for (const status of ["active", "completed"] as const) {
  test(`Landsat ${status} file detail crosses the real desktop bridge without being a catalogue scene`, async () => {
    const payload = filePage(status);
    assert.equal(projectFrozenItemToScene(payload.items[0].item), null,
      "a delivery file must not be invented as an independently searchable scene");
    install(() => payload);
    const result = await loadApplicationTaskPage({
      task_id: "landsat-files", expected_revision: 7, page_index: 0, page_size: 100,
    });
    assert.equal(result.ok, true, JSON.stringify(result));
    if (!result.ok) return;
    assert.equal(result.page.revision, 8);
    assert.equal(result.page.total_count, 22);
    assert.equal(result.page.items[0].item.display_name, fileName);
    assert.deepEqual(result.checked_item_ids, [`${sourceKey}::ASSET::coastal`]);
    assert.deepEqual(result.scenes, [], "task-only assets have no catalogue row projection");
    const view = render(<TaskPageDetail page={projectTaskPagePresentation(result.page)} onPageIndexChange={() => undefined} />);
    assert.match(view.container.textContent ?? "", /SR_B1\.TIF/);
    assert.doesNotMatch(view.container.textContent ?? "", /当前应用状态已变化/);
  });
}

test("read-only task events accept the Store's observed revision after progress advances", async () => {
  install(() => ({
    task_id: "landsat-files", revision: 9, page_size: 40,
    has_more: false, next_before_sequence: null,
    items: [{ sequence: 4, occurred_at: "2025-01-01T00:00:01Z", level: "info",
      event_kind: "progress", item_label: fileName, code: null, message_zh: "正在下载" }],
  }));
  const result = await loadApplicationTaskEventPage({ task_id: "landsat-files", expected_revision: 8, page_size: 40 });
  assert.equal(result.ok, true, JSON.stringify(result));
  if (result.ok) assert.equal(result.revision, 9);
});

test("task detail still rejects a different task or page even though runtime revisions are observed", async () => {
  for (const changed of [{ task_id: "other-task" }, { page_index: 1 }, { page_size: 101 }]) {
    install(() => ({ ...filePage("completed"), ...changed }));
    const result = await loadApplicationTaskPage({ task_id: "landsat-files", expected_revision: 7, page_index: 0, page_size: 100 });
    assert.equal(result.ok, false);
  }
});
