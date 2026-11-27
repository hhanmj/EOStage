import assert from "node:assert/strict";
import test from "node:test";
import { observeOpticalPreparation, parseOpticalPreparation, type OpticalPreparationResult } from "./opticalPreparation.ts";
import { createTauriDesktopShellBridge, type TauriBridgeDependencies } from "./desktopShellBridgeTauri.ts";
import { opticalTaskReceiptFixture } from "../test-utils/optical-task-fixture.ts";

const progress = { request_id: "request", requested_count: 100, prepared_count: 50, admitted_count: 49, rejected_count: 1, phase: "preparing" as const };
const delay = (ms: number) => new Promise(resolve => setTimeout(resolve, ms));

test("preparation wire counts remain scene-scoped and reject forged or premature completion", () => {
  assert.deepEqual(parseOpticalPreparation(progress,"request"),progress);
  for (const value of [null, {...progress,request_id:"other"}, {...progress,requested_count:251},
    {...progress,prepared_count:51}, {...progress,admitted_count:-1}, {...progress,phase:"completed"},
    {...progress,phase:"committing"}, {...progress,authorization:"must-not-cross"}]) {
    assert.equal(parseOpticalPreparation(value,"request"),null);
  }
});

test("preparation observer has one pending read and ignores a late result after teardown", async () => {
  let calls=0, resolve!: (result:OpticalPreparationResult)=>void;
  const observed: unknown[]=[];
  const stop=observeOpticalPreparation("request",()=>{calls++;return new Promise(done=>{resolve=done;});},
    (...value)=>observed.push(value),1);
  await delay(20);assert.equal(calls,1);
  stop();resolve({ok:true,progress});await delay(10);
  assert.deepEqual(observed,[]);assert.equal(calls,1);
});

test("progress read failure or backwards counts cannot cancel creation or manufacture success", async () => {
  const inputs: OpticalPreparationResult[]=[{ok:true,progress},{ok:true,progress:{...progress,prepared_count:49,admitted_count:48}},
    {ok:false,code:"TEST_READ",error:"read unavailable"},
    {ok:true,progress:{...progress,prepared_count:100,admitted_count:99,phase:"committing"}}];
  const observed: Array<[unknown,boolean]>=[];
  let finish!: ()=>void;const done=new Promise<void>(resolve=>{finish=resolve;});
  const stop=observeOpticalPreparation("request",async()=>inputs.shift()!, (p,error)=>{
    observed.push([p,error]);if(observed.length===4)finish();
  },1);
  try { await done;assert.deepEqual(observed.map(row=>row[1]),[false,true,true,false]); }
  finally { stop(); }
});

const request={request_id:"request",source_batch_id:"source",expected_workbench_revision:7,item_ids:["scene-a","scene-b"],task_id:"task",output_root:"C:\\fixtures",
  layout:"direct" as const,concurrency:2,created_at:"2026-09-30T00:00:00Z"};
function native(raw:unknown) {
  const calls: unknown[]=[];
  const deps:TauriBridgeDependencies={invoke:async(command,args)=>{calls.push([command,args]);return raw;},open:async()=>null,
    getCurrentWindow:()=>({minimize:async()=>undefined,toggleMaximize:async()=>undefined,close:async()=>undefined,
      innerSize:async()=>({width:920,height:620}),startResizeDragging:async()=>undefined})};
  return {bridge:createTauriDesktopShellBridge(deps,"tauri://localhost/",false),calls};
}
test("SAFE all-rejected response preserves exact source IDs but does not claim a task",async()=>{
  const admission={requested_count:2,admitted_count:0,rejected_count:2,
    rejections:request.item_ids.map(source_item_id=>({source_item_id,reason_code:"SENTINEL2_ARCHIVE_DELIVERY_UNAVAILABLE"}))};
  const fixture=native({snapshot:null,admission});
  assert.deepEqual(await fixture.bridge.application.createSentinel2ArchiveBatchTask(request),{ok:true,snapshot:null,admission});
  for(const changed of [{...admission,rejections:[admission.rejections[0],admission.rejections[0]]},
    {...admission,rejections:[admission.rejections[0],{source_item_id:"foreign",reason_code:"FAIL"}]}]) {
    assert.equal((await native({snapshot:null,admission:changed}).bridge.application.createSentinel2ArchiveBatchTask(request)).ok,false);
  }
});
test("SAFE receipt binds the admitted count, task, provider and derived batch",async()=>{
  const admission={requested_count:2,admitted_count:1,rejected_count:1,rejections:[{source_item_id:"scene-b",reason_code:"LOOKUP_FAILED"}]};
  const snapshot=opticalTaskReceiptFixture(request,"sentinel2-archive-batch-",1);
  Object.assign((snapshot.core.tasks as Record<string,unknown>[])[0],{delivery_requested_count:2,delivery_rejected_count:1});
  const raw={snapshot,admission};
  assert.equal((await native(raw).bridge.application.createSentinel2ArchiveBatchTask(request)).ok,true);
  for(const update of [{task_id:"foreign"},{selected_count:2},{batch_id:"wrong"},{provider_id:"microsoft.planetary-computer"}]) {
    const changed=structuredClone(raw);Object.assign((changed.snapshot.core.tasks as Record<string,unknown>[])[0],update);
    assert.equal((await native(changed).bridge.application.createSentinel2ArchiveBatchTask(request)).ok,false);
  }
});
test("preparation IPC is a bounded read and rejects unrelated progress",async()=>{
  const fixture=native(progress);
  assert.deepEqual(await fixture.bridge.application.opticalPreparationProgress("request"),{ok:true,progress});
  assert.deepEqual(fixture.calls,[["application_optical_preparation_progress",{requestId:"request"}]]);
  assert.equal((await fixture.bridge.application.opticalPreparationProgress("other")).ok,false);
});
