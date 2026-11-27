import { readFileSync } from "node:fs";
import assert from "node:assert/strict";
import test from "node:test";
import { parseCoreRestore, parseCoreTaskSummary } from "./coreRestoreProjection.ts";
import { createTauriDesktopShellBridge, type TauriBridgeDependencies } from "./desktopShellBridgeTauri.ts";
import { taskSessionProvider } from "./providerSessionGate.ts";
import { taskDeliveryProvider } from "./sarTaskDisplayName.ts";
import { opticalTaskReceiptFixture } from "../test-utils/optical-task-fixture.ts";

const request = {
  request_id: "hls-band-request",
  source_batch_id: "source-batch",
  expected_workbench_revision: 7,
  item_ids: ["scene-a", "scene-b"],
  asset_keys: ["B02", "B03", "B04", "Fmask"],
  task_id: "hls-band-task",
  output_root: "C:\\fixtures",
  layout: "direct" as const,
  concurrency: 2,
  created_at: "2026-11-13T00:00:00Z",
};

function response(
  rejected = 0,
  datasetId: "optical.hls.s30.v2" | "optical.hls.l30.v2" = "optical.hls.s30.v2",
) {
  const receipt = opticalTaskReceiptFixture(request, "hls-bands-batch-", (2 - rejected) * 4);
  Object.assign((receipt.core.tasks as Record<string, unknown>[])[0], {
    dataset_id: datasetId,
    delivery_requested_count: 2,
    delivery_rejected_count: rejected,
  });
  return {
    snapshot: rejected === 2 ? null : { readiness: receipt.readiness, core: receipt.core, aoi_display: null },
    admission: {
      requested_count: 2,
      admitted_count: 2 - rejected,
      rejected_count: rejected,
      rejections: request.item_ids.slice(2 - rejected).map((source_item_id) => ({
        source_item_id,
        reason_code: "STAC_TIMEOUT",
      })),
    },
  };
}

function fixture(raw: unknown) {
  const calls: unknown[] = [];
  const deps: TauriBridgeDependencies = {
    invoke: async (command, args) => {
      calls.push([command, args]);
      return raw;
    },
    getCurrentWindow: () => ({
      minimize: async () => undefined,
      toggleMaximize: async () => undefined,
      close: async () => undefined,
      innerSize: async () => ({ width: 920, height: 620 }),
      startResizeDragging: async () => undefined,
    }),
    open: async () => null,
  };
  return { bridge: createTauriDesktopShellBridge(deps, "tauri://localhost/", false), calls };
}

test("HLS native band batches accept complete, partial and all-rejected receipts for L30 and S30", async () => {
  for (const datasetId of ["optical.hls.s30.v2", "optical.hls.l30.v2"] as const) {
    for (const rejected of [0, 1, 2]) {
      const raw = response(rejected, datasetId);
      const f = fixture(raw);
      const result = await f.bridge.application.createHlsBandBatchTask(request);
      assert.equal(result.ok, true, `${datasetId} rejected=${rejected}`);
      if (result.ok) {
        assert.deepEqual(result.admission, raw.admission);
        assert.equal(result.snapshot === null, rejected === 2);
        if (result.snapshot && result.snapshot.ok) {
          const task = result.snapshot.core.tasks[0];
          assert.equal(task.provider_id, "microsoft.planetary-computer");
          assert.equal(task.dataset_id, datasetId);
          assert.equal(task.selected_count, (2 - rejected) * 4);
          assert.equal(task.delivery_requested_count, 2);
          assert.equal(task.delivery_rejected_count, rejected);
        }
      }
      assert.deepEqual(f.calls, [["application_create_hls_band_batch_task", { request }]]);
    }
  }
});

test("empty HLS band item IDs preserve Store-owned checked selection", async () => {
  const storeRequest = { ...request, item_ids: [] as string[] };
  const raw = response(0);
  const f = fixture(raw);
  const result = await f.bridge.application.createHlsBandBatchTask(storeRequest);
  assert.equal(result.ok, true);
  assert.deepEqual(f.calls, [["application_create_hls_band_batch_task", { request: storeRequest }]]);
});

test("HLS receipt validation rejects mismatched provider, dataset, batch_id and admission counts", async () => {
  for (const change of [
    { task_id: "foreign" },
    { provider_id: "microsoft.planetary-computer.hls" },
    { dataset_id: "optical.sentinel-2.l2a" },
    { selected_count: 1 },
    { delivery_requested_count: 3 },
    { delivery_rejected_count: 1 },
    { batch_id: "foreign-batch" },
  ]) {
    const raw = response(0);
    Object.assign((raw.snapshot!.core.tasks as Record<string, unknown>[])[0], change);
    const result = await fixture(raw).bridge.application.createHlsBandBatchTask(request);
    assert.equal(result.ok, false, JSON.stringify(change));
  }
});

test("coreRestoreProjection and providerSessionGate support multi-band HLS batches without session gate", () => {
  for (const datasetId of ["optical.hls.s30.v2", "optical.hls.l30.v2"] as const) {
    const raw = response(0, datasetId);
    const taskPayload = raw.snapshot!.core.tasks[0];
    const parsedSummary = parseCoreTaskSummary(taskPayload);
    assert.notEqual(parsedSummary, null, `parseCoreTaskSummary must accept ${datasetId}`);
    assert.equal(parsedSummary!.delivery_requested_count, 2);
    assert.equal(parsedSummary!.delivery_rejected_count, 0);
    assert.equal(parsedSummary!.selected_count, 8);
    const parsedRestore = parseCoreRestore(raw.snapshot!.core);
    assert.notEqual(parsedRestore, null, `parseCoreRestore must accept ${datasetId}`);
    assert.equal(parsedRestore!.tasks[0].delivery_requested_count, 2);
    assert.equal(parsedRestore!.tasks[0].delivery_rejected_count, 0);
    assert.equal(parsedRestore!.tasks[0].selected_count, 8);
  }
  assert.equal(taskDeliveryProvider({ batch_id: "hls-bands-batch-x" }).id, "microsoft.planetary-computer");
  assert.equal(taskSessionProvider({ batch_id: "hls-bands-batch-x" }), null);
});

test("DesktopWorkbench optical band batch taskRef satisfies hasTaskReceipt without falling through to taskReceiptUnknown", () => {
  const source = readFileSync(new URL("../components/desktop/DesktopWorkbench.tsx", import.meta.url), "utf8");
  const start = source.indexOf("if (useOpticalBandDelivery || useMpcSafeArchive)");
  const end = source.indexOf("if (!hasTaskReceipt(taskRef))", start);
  assert.notEqual(start, -1, "DesktopWorkbench should contain useOpticalBandDelivery creation branch");
  assert.notEqual(end, -1, "DesktopWorkbench should validate optical batch taskRef via hasTaskReceipt");
  const branch = source.slice(start, end);
  assert.match(branch, /startHlsBandBatchDownload\(/);
  assert.match(branch, /startSentinel2BandBatchDownload\(/);
  assert.match(branch, /startSentinel2L1cBandBatchDownload\(/);
  assert.match(branch, /created:\s*true/);
  assert.match(branch, /status:\s*normalizedStatus/);
  assert.match(
    source,
    /const normalizedStatus =\s*typeof rawState === "string"[\s\S]*\["queued", "running", "paused", "completed", "failed", "cancelled", "interrupted"\]\.includes\(rawState\)[\s\S]*\? rawState\s*: "queued";/,
  );
});
