import assert from "node:assert/strict";
import test from "node:test";
import { createTauriDesktopShellBridge, type TauriBridgeDependencies } from "./desktopShellBridgeTauri.ts";
import { opticalTaskReceiptFixture } from "../test-utils/optical-task-fixture.ts";

const request = {request_id:"request-batch",source_batch_id:"source-batch",expected_workbench_revision:7,
  item_ids:["scene-a","scene-b"],task_id:"task-batch",output_root:"C:\\fixtures",layout:"direct" as const,
  concurrency:2,created_at:"2026-09-29T00:00:00Z"};
const report = {requested_count:2,admitted_count:1,rejected_count:1,
  rejections:[{source_item_id:"scene-b",reason_code:"LANDSAT_PUBLIC_ASSET_FROZEN_EVIDENCE_INVALID"}]};
function bridge(raw: unknown) {
  const calls: Array<[string,unknown]> = [];
  const dependencies: TauriBridgeDependencies = {invoke:async(command,args)=>{calls.push([command,args]);return raw;},
    getCurrentWindow:()=>({minimize:async()=>undefined,toggleMaximize:async()=>undefined,close:async()=>undefined,
      innerSize:async()=>({width:920,height:620}),startResizeDragging:async()=>undefined}),open:async()=>null};
  return {bridge:createTauriDesktopShellBridge(dependencies,"tauri://localhost/",false),calls};
}
test("native Landsat batch envelope preserves partial and all-rejected outcomes without changing source IDs",async()=>{
  const receipt=opticalTaskReceiptFixture(request,"landsat-assets-batch-",22);
  Object.assign((receipt.core.tasks as Record<string, unknown>[])[0],{delivery_requested_count:2,delivery_rejected_count:1});
  const raw={snapshot:{readiness:receipt.readiness,core:receipt.core,aoi_display:null},admission:report};
  const fixture=bridge(raw);
  const result=await fixture.bridge.application.createLandsatAssetBatchTask(request);
  assert.equal(result.ok,true);
  assert.deepEqual(fixture.calls,[["application_create_landsat_asset_batch_task",{request}]]);
  if(result.ok)assert.deepEqual(result.admission,report);
  const rejected={...report,admitted_count:0,rejected_count:2,rejections:request.item_ids.map(source_item_id=>({source_item_id,reason_code:"LOOKUP_FAILED"}))};
  const all=await bridge({snapshot:null,admission:rejected}).bridge.application.createLandsatAssetBatchTask(request);
  assert.equal(all.ok,true);if(all.ok)assert.equal(all.snapshot,null,"all-rejected is not a created task");
});
test("native batch rejects count, foreign identity, missing snapshot and forbidden extra fields",async()=>{
  for(const raw of [
    {snapshot:null,admission:report},
    {snapshot:null,admission:{...report,requested_count:3}},
    {snapshot:null,admission:{...report,admitted_count:0,requested_count:1}},
    {snapshot:{},admission:report},
    {snapshot:{},admission:{...report,rejections:[{source_item_id:"foreign",reason_code:"FAIL"}]}},
    {snapshot:null,admission:report,secret:"not-allowed"},
  ])assert.equal((await bridge(raw).bridge.application.createLandsatAssetBatchTask(request)).ok,false);
});
test("persisted task admission is a bounded read and has no create side effect",async()=>{
  const fixture=bridge(report);
  assert.deepEqual(await fixture.bridge.application.taskDeliveryAdmission("task-batch"),{ok:true,admission:report});
  assert.deepEqual(fixture.calls,[["application_task_delivery_admission",{taskId:"task-batch"}]]);
});

test("empty Landsat batch IDs preserve Store-owned checked selection on the native boundary", async () => {
  const storeRequest = { ...request, item_ids: [] as string[] };
  const receipt = opticalTaskReceiptFixture(storeRequest, "landsat-assets-batch-", 22);
  Object.assign((receipt.core.tasks as Record<string, unknown>[])[0], {
    delivery_requested_count: 2, delivery_rejected_count: 1,
  });
  const f = bridge({ snapshot: { readiness: receipt.readiness, core: receipt.core, aoi_display: null }, admission: report });
  const result = await f.bridge.application.createLandsatAssetBatchTask(storeRequest);
  assert.equal(result.ok, true);
  assert.deepEqual(f.calls, [["application_create_landsat_asset_batch_task", { request: storeRequest }]]);
});

function archiveResult() {
  const snapshot = opticalTaskReceiptFixture(request, "landsat-bundles-batch-", 1);
  Object.assign((snapshot.core.tasks as Record<string, unknown>[])[0], { delivery_requested_count: 2, delivery_rejected_count: 1 });
  return { snapshot: { readiness: snapshot.readiness, core: snapshot.core, aoi_display: null }, admission: report };
}

test("official archive batch invokes one native command and keeps per-scene admission", async () => {
  const fixture = bridge(archiveResult());
  const result = await fixture.bridge.application.createLandsatBundleBatchTask(request);
  assert.equal(result.ok, true);
  if (result.ok) assert.deepEqual(result.admission, report);
  assert.deepEqual(fixture.calls, [["application_create_landsat_bundle_batch_task", { request }]]);
  const rejected = { requested_count: 2, admitted_count: 0, rejected_count: 2,
    rejections: request.item_ids.map(source_item_id => ({ source_item_id, reason_code: "USGS_M2M_AUTH_REQUIRED" })) };
  const all = await bridge({ snapshot: null, admission: rejected }).bridge.application.createLandsatBundleBatchTask(request);
  assert.equal(all.ok, true);
  if (all.ok) assert.equal(all.snapshot, null);
});

test("official archive batch rejects foreign receipts, asset counts and source channels", async () => {
  for (const mutation of [
    { task_id: "other" }, { batch_id: "landsat-assets-batch-task-batch" },
    { provider_id: "microsoft.planetary-computer" }, { dataset_id: "optical.sentinel-2.l2a" },
    { selected_count: 22, checked_count: 22, queued_count: 22 },
    { delivery_requested_count: 3 }, { delivery_rejected_count: 0 },
  ]) {
    const raw = archiveResult(); Object.assign(raw.snapshot.core.tasks[0], mutation);
    const fixture = bridge(raw);
    assert.equal((await fixture.bridge.application.createLandsatBundleBatchTask(request)).ok, false);
    assert.equal(fixture.calls.length, 1, "rejected receipt must not trigger another create or fallback");
  }
  const raw = archiveResult(); raw.snapshot.core.tasks.push(structuredClone(raw.snapshot.core.tasks[0]));
  assert.equal((await bridge(raw).bridge.application.createLandsatBundleBatchTask(request)).ok, false);
});

test("official archive batch rejects malformed selection before invoking native", async () => {
  for (const invalid of [{ ...request, item_ids: ["scene-a", "scene-a"] },
    { ...request, expected_workbench_revision: -1 }]) {
    const fixture = bridge(archiveResult());
    assert.equal((await fixture.bridge.application.createLandsatBundleBatchTask(invalid)).ok, false);
    assert.equal(fixture.calls.length, 0);
  }
});

