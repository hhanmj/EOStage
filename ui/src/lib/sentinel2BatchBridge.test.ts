import assert from "node:assert/strict";
import test from "node:test";
import { createTauriDesktopShellBridge, type TauriBridgeDependencies } from "./desktopShellBridgeTauri.ts";
import { opticalTaskReceiptFixture } from "../test-utils/optical-task-fixture.ts";
const request = {request_id:"band-request",source_batch_id:"source-batch",expected_workbench_revision:7,
  item_ids:["scene-a","scene-b"],asset_keys:["B02","B03","B04"],task_id:"band-task",output_root:"C:\\fixtures",
  layout:"direct" as const,concurrency:2,created_at:"2026-09-30T00:00:00Z"};
function response(rejected = 0) {
  const receipt = opticalTaskReceiptFixture(request,"sentinel2-bands-batch-",(2-rejected)*3);
  Object.assign((receipt.core.tasks as Record<string,unknown>[])[0],{delivery_requested_count:2,delivery_rejected_count:rejected});
  return {snapshot: rejected === 2 ? null : {readiness:receipt.readiness,core:receipt.core,aoi_display:null},
    admission:{requested_count:2,admitted_count:2-rejected,rejected_count:rejected,
      rejections:request.item_ids.slice(2-rejected).map(source_item_id=>({source_item_id,reason_code:"STAC_TIMEOUT"}))}};
}
function fixture(raw: unknown) {
  const calls:unknown[]=[];
  const deps:TauriBridgeDependencies={invoke:async(command,args)=>{calls.push([command,args]);return raw;},
    getCurrentWindow:()=>({minimize:async()=>undefined,toggleMaximize:async()=>undefined,close:async()=>undefined,
      innerSize:async()=>({width:920,height:620}),startResizeDragging:async()=>undefined}),open:async()=>null};
  return {bridge:createTauriDesktopShellBridge(deps,"tauri://localhost/",false),calls};
}
test("Sentinel-2 native band batches preserve complete, partial and all-rejected receipts",async()=>{
  for(const rejected of [0,1,2]) {
    const raw=response(rejected),f=fixture(raw),r=await f.bridge.application.createSentinel2BandBatchTask(request);
    assert.equal(r.ok,true);if(r.ok){assert.deepEqual(r.admission,raw.admission);assert.equal(r.snapshot===null,rejected===2);}
    assert.deepEqual(f.calls,[["application_create_sentinel2_band_batch_task",{request}]]);
  }
});

test("empty Sentinel-2 band IDs preserve Store-owned checked selection on the native boundary", async () => {
  const storeRequest={...request,item_ids:[] as string[]};
  const raw=response(0);
  const f=fixture(raw);
  const result=await f.bridge.application.createSentinel2BandBatchTask(storeRequest);
  assert.equal(result.ok,true);
  assert.deepEqual(f.calls,[["application_create_sentinel2_band_batch_task",{request:storeRequest}]]);
});
test("Sentinel-2 native band batches accept Earth Search L2A provider receipts", async () => {
  const raw = response(0);
  Object.assign((raw.snapshot!.core.tasks as Record<string, unknown>[])[0], {
    provider_id: "element84.earth-search",
  });
  const f = fixture(raw);
  const r = await f.bridge.application.createSentinel2BandBatchTask(request);
  assert.equal(r.ok, true);
});
test("band-batch receipt requires exact task, provider, dataset and scene-to-file counts",async()=>{
  for(const change of [{task_id:"foreign"},{provider_id:"copernicus.dataspace.odata"},{dataset_id:"optical.landsat.c2.l2"},
    {selected_count:5},{delivery_requested_count:3},{delivery_rejected_count:1},{batch_id:"foreign-batch"}]) {
    const raw=response();Object.assign((raw.snapshot!.core.tasks as Record<string,unknown>[])[0],change);
    assert.equal((await fixture(raw).bridge.application.createSentinel2BandBatchTask(request)).ok,false,JSON.stringify(change));
  }
});
test("invalid band selection and revision never call native creation",async()=>{
  for(const change of [{asset_keys:["B10"]},{asset_keys:["B02","B02"]},{asset_keys:[]},{item_ids:["a","a"]},{expected_workbench_revision:-1}]) {
    const f=fixture(response());assert.equal((await f.bridge.application.createSentinel2BandBatchTask({...request,...change})).ok,false);assert.equal(f.calls.length,0);
  }
});
