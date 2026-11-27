import assert from "node:assert/strict";
import test from "node:test";
import "../test-utils/setup-dom";
import { opticalTaskReceiptFixture } from "../test-utils/optical-task-fixture";
import { clearTestDesktopShellBridge, registerTestDesktopShellBridge } from "./desktopShellBridge";
import { createBrowserFixtureDesktopShellBridge } from "./desktopShellBridgeFixture";
import { startLandsatAssetBatchDownload, startLandsatAssetDownload, startLandsatBundleDownload,
  startOrbitDownloadSnapshot,
  startSentinel2ArchiveDownload, startSentinel2ArchiveBatchDownload, startSentinel2BandDownload,
  type DownloadBatchContext } from "./bridge";

test.afterEach(() => clearTestDesktopShellBridge());

const taskId = "optical-task-fixture";
const source: DownloadBatchContext = { batch_id: "source-batch", revision: 7, page_index: 0,
  page_size: 250, total_count: 2, checked_count: 1, aoi_name: "Fixture AOI" };
const output = "C:\\fixtures\\downloads";
const routes = [
  ["Landsat archive", "createLandsatBundleTask", "landsat-delivery-",
    () => startLandsatBundleDownload(output, "scene-a", 2, source, taskId, "request-a")],
  ["Landsat files", "createLandsatAssetTask", "landsat-assets-",
    () => startLandsatAssetDownload(output, "scene-a", 2, source, taskId, "request-a", ["red"])],
  ["Sentinel-2 SAFE", "createSentinel2ArchiveTask", "sentinel2-archive-",
    () => startSentinel2ArchiveDownload(output, "scene-a", source, taskId, "request-a")],
  ["Sentinel-2 SAFE batch", "createSentinel2ArchiveBatchTask", "sentinel2-archive-batch-",
    () => startSentinel2ArchiveBatchDownload(output, ["scene-a", "scene-b"], source, taskId, "request-a")],
  ["Sentinel-2 bands", "createSentinel2BandTask", "sentinel2-bands-",
    () => startSentinel2BandDownload(output, "scene-a", ["B02"], source, taskId, "request-a")],
] as const;

for (const [label, method, prefix, create] of routes) {
  test(`${label}: native creation receipt does not depend on the active search page`, async () => {
    let writes = 0;
    let reads = 0;
    const base = createBrowserFixtureDesktopShellBridge();
    registerTestDesktopShellBridge({ ...base, kind: "tauri", application: { ...base.application,
      [method]: async (request: { task_id: string; output_root: string; created_at: string }) => {
        writes += 1;
        const snapshot = opticalTaskReceiptFixture(request, prefix);
        // An unrelated search is active by the time the mutation returns.
        (snapshot.core.workbench as Record<string, unknown>).batch_id = "newer-search";
        return method === "createSentinel2ArchiveBatchTask" ? { ok: true, snapshot, admission: {
          requested_count: 2, admitted_count: 1, rejected_count: 1,
          rejections: [{ source_item_id: "scene-b", reason_code: "SENTINEL2_ARCHIVE_UNAVAILABLE" }],
        } } : snapshot;
      },
      workbenchPage: async () => { reads += 1; throw new Error("page read failed after committed creation"); },
    } });
    const result = await create();
    assert.equal(result.ok, true, "a committed task must not become a failed create because a page read failed");
    assert.equal(result.task_id, taskId);
    assert.equal(result.download_status?.task_id, taskId);
    assert.equal(result.download_status?.total, 1, "use admitted Store items, not requested products");
    assert.equal(reads, 0, "task creation must never hydrate the active search");
    assert.equal(writes, 1);
    assert.equal(result.search_workbench, undefined, "do not overwrite newer search state with a task receipt");
  });
}

test("missing or mismatched task acknowledgement never fabricates a successful creation", async () => {
  for (const invalid of ["missing", "wrong-task", "wrong-batch"] as const) {
    const base = createBrowserFixtureDesktopShellBridge();
    registerTestDesktopShellBridge({ ...base, kind: "tauri", application: { ...base.application,
      createLandsatAssetTask: async request => {
        const snapshot = opticalTaskReceiptFixture(request);
        const tasks = snapshot.core.tasks as Array<Record<string, unknown>>;
        if (invalid === "missing") snapshot.core.tasks = [];
        if (invalid === "wrong-task") tasks[0]!.task_id = "another-task";
        if (invalid === "wrong-batch") tasks[0]!.batch_id = "unrelated-batch";
        return snapshot;
      },
    } });
    assert.equal((await startLandsatAssetDownload(output, "scene-a", 2, source, taskId, "request-a")).ok, false);
  }
});

test("derived output and an immediate transfer failure still describe a created task", async () => {
  const productOutput = `${output}\\LC08_L2SP_123032_20260703_20260715_02_T1`;
  const base = createBrowserFixtureDesktopShellBridge();
  registerTestDesktopShellBridge({ ...base, kind: "tauri", application: { ...base.application,
    createLandsatAssetTask: async request => {
      const snapshot = opticalTaskReceiptFixture({ ...request, output_root: productOutput });
      const task = (snapshot.core.tasks as Array<Record<string, unknown>>)[0]!;
      task.status = "failed";
      task.queued_count = 0;
      task.failed_count = 1;
      return snapshot;
    },
    workbenchPage: async () => new Promise(() => undefined),
  } });
  const result = await startLandsatAssetDownload(output, "scene-a", 2, source, taskId, "request-a");
  assert.equal(result.ok, true);
  assert.equal(result.download_status?.state, "failed", "transfer failure is not creation failure");
  assert.equal(result.download_status?.output_dir, productOutput, "retain the authoritative product directory");
});

test("an outcome-unknown single create reconciles the committed Store task without replaying the mutation", async () => {
  let writes = 0, reads = 0;
  let committed: ReturnType<typeof opticalTaskReceiptFixture> | null = null;
  const base = createBrowserFixtureDesktopShellBridge();
  registerTestDesktopShellBridge({ ...base, kind: "tauri", application: { ...base.application,
    createLandsatAssetTask: async request => {
      writes += 1;
      committed = opticalTaskReceiptFixture(request, "landsat-assets-");
      return { ok: false, code: "V3_APPLICATION_UNAVAILABLE", error: "mutation reply lost" };
    },
    coreRestore: async () => {
      reads += 1;
      assert.ok(committed, "the synthetic Store task is committed before reconciliation");
      return committed;
    },
  } });
  const result = await startLandsatAssetDownload(output, "scene-a", 2, source, taskId, "request-a");
  assert.equal(result.ok, true);
  assert.equal(result.task_id, taskId);
  assert.equal(result.download_status?.task_id, taskId);
  assert.equal(writes, 1, "outcome reconciliation must not replay create");
  assert.equal(reads, 1, "read Store once by the stable task identity");
});

test("a malformed creation acknowledgement reconciles a matching committed Store task", async () => {
  let writes = 0, reads = 0;
  let committed: ReturnType<typeof opticalTaskReceiptFixture> | null = null;
  const base = createBrowserFixtureDesktopShellBridge();
  registerTestDesktopShellBridge({ ...base, kind: "tauri", application: { ...base.application,
    createLandsatBundleTask: async request => {
      writes += 1;
      committed = opticalTaskReceiptFixture(request, "landsat-delivery-");
      const malformed = opticalTaskReceiptFixture(request, "landsat-delivery-");
      malformed.core.tasks = [];
      return malformed;
    },
    coreRestore: async () => {
      reads += 1;
      assert.ok(committed);
      return committed;
    },
  } });
  const result = await startLandsatBundleDownload(output, "scene-a", 2, source, taskId, "request-a");
  assert.equal(result.ok, true);
  assert.equal(result.task_id, taskId);
  assert.equal(writes, 1);
  assert.equal(reads, 1);
});

test("batch outcome reconciliation restores persisted admission without creating a second batch", async () => {
  let writes = 0, reads = 0, admissionReads = 0;
  let committed: ReturnType<typeof opticalTaskReceiptFixture> | null = null;
  const admission = {
    requested_count: 2, admitted_count: 1, rejected_count: 1,
    rejections: [{ source_item_id: "scene-b", reason_code: "STAC_TIMEOUT" }],
  };
  const base = createBrowserFixtureDesktopShellBridge();
  registerTestDesktopShellBridge({ ...base, kind: "tauri", application: { ...base.application,
    createLandsatAssetBatchTask: async request => {
      writes += 1;
      committed = opticalTaskReceiptFixture(request, "landsat-assets-batch-", 22);
      return { ok: false, code: "V3_APPLICATION_UNAVAILABLE", error: "mutation reply lost" };
    },
    coreRestore: async () => {
      reads += 1;
      assert.ok(committed);
      return committed;
    },
    taskDeliveryAdmission: async id => {
      admissionReads += 1;
      assert.equal(id, taskId);
      return { ok: true, admission };
    },
  } });
  const result = await startLandsatAssetBatchDownload(output, ["scene-a", "scene-b"], source, taskId, "request-a");
  assert.equal(result.ok, true);
  assert.equal(result.task_id, taskId);
  assert.deepEqual(result.admission, admission);
  assert.equal(result.download_status?.total, 22);
  assert.equal(writes, 1);
  assert.equal(reads, 1);
  assert.equal(admissionReads, 1);
});

test("batch reconciliation still returns the committed task when admission detail cannot be re-read", async () => {
  let writes = 0, admissionReads = 0;
  let committed: ReturnType<typeof opticalTaskReceiptFixture> | null = null;
  const base = createBrowserFixtureDesktopShellBridge();
  registerTestDesktopShellBridge({ ...base, kind: "tauri", application: { ...base.application,
    createLandsatAssetBatchTask: async request => {
      writes += 1;
      committed = opticalTaskReceiptFixture(request, "landsat-assets-batch-", 44);
      return { ok: false, code: "V3_APPLICATION_UNAVAILABLE", error: "mutation reply lost" };
    },
    coreRestore: async () => {
      assert.ok(committed);
      return committed;
    },
    taskDeliveryAdmission: async () => {
      admissionReads += 1;
      return { ok: true, admission: null };
    },
  } });
  const result = await startLandsatAssetBatchDownload(output, ["scene-a", "scene-b"], source, taskId, "request-a");
  assert.equal(result.ok, true);
  assert.equal(result.task_id, taskId);
  assert.equal(result.download_status?.total, 44);
  assert.equal(result.admission, undefined);
  assert.equal(writes, 1);
  assert.equal(admissionReads, 1);
});

test("a native admission failure remains a failure and does not reconcile or retry", async () => {
  let writes = 0, reads = 0;
  const base = createBrowserFixtureDesktopShellBridge();
  registerTestDesktopShellBridge({ ...base, kind: "tauri", application: { ...base.application,
    createLandsatAssetTask: async () => { writes += 1; return { ok: false, code: "ADMISSION_REJECTED", error: "准入失败" }; },
    coreRestore: async () => { reads += 1; return opticalTaskReceiptFixture({
      task_id: taskId, output_root: output, created_at: new Date().toISOString(),
    }); },
  } });
  const result = await startLandsatAssetDownload(output, "scene-a", 2, source, taskId, "request-a");
  assert.equal(result.ok, false);
  assert.equal(result.code, "ADMISSION_REJECTED");
  assert.equal(writes, 1);
  assert.equal(reads, 0, "known business failures are final and must not probe Store as a success path");
});

test("Issue #51: startOrbitDownloadSnapshot uses createStandaloneOrbitTask with bounded task_id/request_id on real S1 scenes and zero workspace calls", async () => {
  const base = createBrowserFixtureDesktopShellBridge();
  let calledStandalone = 0;
  let calledCoreRestore = 0;
  let capturedReq: Record<string, unknown> | null = null;
  registerTestDesktopShellBridge({
    ...base,
    kind: "tauri",
    application: {
      ...base.application,
      coreRestore: async () => {
        calledCoreRestore += 1;
        return base.application.coreRestore();
      },
      createStandaloneOrbitTask: async (request) => {
        calledStandalone += 1;
        capturedReq = request as unknown as Record<string, unknown>;
        return opticalTaskReceiptFixture(
          { task_id: request.task_id, output_root: request.output_root, created_at: request.created_at },
          "orbit-standalone-",
          1,
        );
      },
    },
  });
  const longS1Id =
    "S1A_IW_SLC__1SDV_20240312T100000_20240312T100027_052946_066912_ABCD";
  const result = await startOrbitDownloadSnapshot(
    "/tmp/orbit-standalone-out",
    [
      {
        scene_id: longS1Id,
        product_id: longS1Id,
        acquisition_datetime: "2024-03-12T10:00:00Z",
      } as any,
    ],
    [longS1Id],
    3,
    true,
  );
  assert.equal(result.ok, true);
  assert.equal(calledStandalone, 1);
  assert.equal(calledCoreRestore, 0, "standalone orbit must not require legacy workspace context when outputDir is given");
  assert.ok(capturedReq);
  const taskIdVal = String((capturedReq as Record<string, unknown>).task_id ?? "");
  const requestIdVal = String((capturedReq as Record<string, unknown>).request_id ?? "");
  assert.ok(taskIdVal.length >= 8 && taskIdVal.length <= 96, `task_id length ${taskIdVal.length} must satisfy Rust <= 96 bound`);
  assert.ok(requestIdVal.length >= 8 && requestIdVal.length <= 128, `request_id length ${requestIdVal.length} must satisfy Rust <= 128 bound`);
  assert.equal((capturedReq as Record<string, unknown>).output_root, "/tmp/orbit-standalone-out");
  assert.equal((capturedReq as Record<string, unknown>).use_orbit_subdir, true);
});

test("Bug C: startOrbitDownloadSnapshot strips PROVIDER::...::ITEM:: prefix and ASF suffix from S1D canonical IDs", async () => {
  const base = createBrowserFixtureDesktopShellBridge();
  let capturedScenes: Array<{ scene_id: string; acquisition_datetime?: string | null }> = [];
  registerTestDesktopShellBridge({
    ...base,
    kind: "tauri",
    application: {
      ...base.application,
      createStandaloneOrbitTask: async (request) => {
        capturedScenes = request.scenes;
        return opticalTaskReceiptFixture(
          { task_id: request.task_id, output_root: request.output_root, created_at: request.created_at },
          "orbit-standalone-",
          1,
        );
      },
    },
  });
  const canonicalS1d =
    "PROVIDER::asf.catalog::COLLECTION::SENTINEL-1::ITEM::S1D_IW_SLC__1SDV_20260825T101857_20260825T101925_004281_007E0D_FB72-SLC";
  const result = await startOrbitDownloadSnapshot(
    "/tmp/orbit-standalone-s1d",
    [
      {
        scene_id: canonicalS1d,
        product_id: canonicalS1d,
        acquisition_datetime: "2026-08-25T10:18:57Z",
      } as any,
    ],
    [canonicalS1d],
    2,
    false,
  );
  assert.equal(result.ok, true);
  assert.equal(capturedScenes.length, 1);
  assert.equal(
    capturedScenes[0]?.scene_id,
    "S1D_IW_SLC__1SDV_20260825T101857_20260825T101925_004281_007E0D_FB72",
  );
});
