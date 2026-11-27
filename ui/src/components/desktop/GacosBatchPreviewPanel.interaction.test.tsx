import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { cleanup, fireEvent, render } from "../../test-utils/render-harness";
import { GacosBatchPreviewPanel } from "./GacosBatchPreviewPanel";
import { updateWorkbenchToolOptions } from "./ManualConditionsPanel";
import type { GacosBatchSubmitState } from "../../lib/companionGacosFlow";

test.afterEach(() => {
  cleanup();
  updateWorkbenchToolOptions({
    gacosDatesText: "",
    gacosUtcHour: "10",
    gacosUtcMinute: "0",
    gacosOutputFormat: "geotiff",
  });
});

test("GacosBatchPreviewPanel reactively splits 25 dates into 2 batches (20 + 5), tracks selection, and updates status cards in place", () => {
  const dates25 = Array.from({ length: 25 }, (_, i) => {
    const day = String(i + 1).padStart(2, "0");
    return `202605${day}`;
  }).join("\n");

  updateWorkbenchToolOptions({
    gacosDatesText: dates25,
    gacosUtcHour: "06",
    gacosUtcMinute: "35",
    gacosOutputFormat: "binary",
  });

  const toggled: number[] = [];
  const retried: number[] = [];
  const statuses = new Map<number, GacosBatchSubmitState>();

  const view = render(
    <GacosBatchPreviewPanel
      workingAoi={{ west: 116.5, south: 31.2, east: 118.1, north: 32.6, crs: "EPSG:4326" }}
      aoiLabel="合肥"
      startDate="2026-05-01"
      endDate="2026-05-25"
      gacosEmail="user@example.org"
      batchStatuses={statuses}
      selectedBatchIndices={[0, 1]}
      onToggleBatchIndex={(idx) => toggled.push(idx)}
      onRetryBatch={(idx) => retried.push(idx)}
    />
  );

  assert.ok(view.getByTestId("gacos-batch-preview-panel"));
  assert.match(view.getByTestId("gacos-batch-count-badge").textContent ?? "", /共 2 批 · 25 个日期/);
  assert.ok(view.getByText(/第 1 \/ 2 批（20 个日期）/));
  assert.ok(view.getByText(/第 2 \/ 2 批（5 个日期）/));
  assert.ok(view.getAllByText(/06:35 UTC/).length >= 1);
  assert.ok(view.getAllByText(/Binary Grid/).length >= 1);
  assert.ok(view.getByText(/user@example\.org/));

  // Uncheck batch 2
  const cb2 = view.getByRole("checkbox", { name: "选择第 2 批" }) as HTMLInputElement;
  fireEvent.click(cb2);
  assert.deepEqual(toggled, [1]);

  // Update batch statuses in place: batch 0 submitted, batch 1 failed with retry
  statuses.set(0, { status: "submitted", taskId: "gacos-task-1" });
  statuses.set(1, { status: "failed", error: "GACOS 服务器超时" });
  view.rerender(
    <GacosBatchPreviewPanel
      workingAoi={{ west: 116.5, south: 31.2, east: 118.1, north: 32.6, crs: "EPSG:4326" }}
      aoiLabel="合肥"
      startDate="2026-05-01"
      endDate="2026-05-25"
      gacosEmail="user@example.org"
      batchStatuses={new Map(statuses)}
      selectedBatchIndices={[0, 1]}
      onToggleBatchIndex={(idx) => toggled.push(idx)}
      onRetryBatch={(idx) => retried.push(idx)}
    />
  );

  assert.equal(view.getByTestId("gacos-batch-status-0").textContent?.trim(), "已提交申请");
  assert.equal(view.getByTestId("gacos-batch-status-1").textContent?.trim(), "提交失败");
  fireEvent.click(view.getByTestId("gacos-batch-retry-1"));
  assert.deepEqual(retried, [1]);
});
