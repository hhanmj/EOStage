import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { waitFor } from "@testing-library/react";
import { cleanup, render } from "../test-utils/render-harness";
import { OpticalDeliveryReport } from "./OpticalDeliveryReport";
import { clearTestDesktopShellBridge, registerTestDesktopShellBridge } from "../lib/desktopShellBridge";
import { createBrowserFixtureDesktopShellBridge } from "../lib/desktopShellBridgeFixture";

test.afterEach(()=>{cleanup();clearTestDesktopShellBridge();});
test("task report shows refused scenes and file count without confusing them",async()=>{
  const base=createBrowserFixtureDesktopShellBridge();
  registerTestDesktopShellBridge({...base,application:{...base.application,taskDeliveryAdmission:async taskId=>{
    assert.equal(taskId,"task-batch");return {ok:true,admission:{requested_count:2,admitted_count:1,rejected_count:1,
      rejections:[{source_item_id:"PROVIDER::mpc::ITEM::scene-b",reason_code:"LOOKUP_FAILED"}]}};
  }}});
  const view=render(<OpticalDeliveryReport taskId="task-batch" fileCount={22}/>);
  await view.findByLabelText("下载准备记录");
  assert.match(view.container.textContent!,/已加入 1 景.*下载文件 22 个/);
  assert.ok(view.container.textContent?.includes("scene-b"));
});
test("task report keeps the MPC sign failure class and its detail",async()=>{
  const base=createBrowserFixtureDesktopShellBridge();
  registerTestDesktopShellBridge({...base,application:{...base.application,taskDeliveryAdmission:async()=>({ok:true,admission:{
    requested_count:2,admitted_count:1,rejected_count:1,rejections:[{source_item_id:"PROVIDER::mpc::ITEM::scene-b",
      reason_code:"MPC_ASSET_SIGN_RATE_LIMITED",reason_detail:"reason=http_status_transient;http_status=429;retry_after_seconds=3;attempts=4"}]}})}});
  const view=render(<OpticalDeliveryReport taskId="task-batch" fileCount={12}/>);
  await view.findByLabelText("下载准备记录");
  assert.match(view.container.textContent!,/scene-b：MPC 签名服务限流，请约 3 秒后重试/);
  assert.match(view.container.textContent!,/MPC_ASSET_SIGN_RATE_LIMITED；.*retry_after_seconds=3/);
});
test("late admission from a previously opened task cannot replace the current report",async()=>{
  const base=createBrowserFixtureDesktopShellBridge();let finish:((value:unknown)=>void)|undefined;
  registerTestDesktopShellBridge({...base,application:{...base.application,taskDeliveryAdmission:async taskId=>
    taskId==="old"?new Promise(resolve=>{finish=resolve;}):{ok:true,admission:null}}});
  const view=render(<OpticalDeliveryReport taskId="old" fileCount={22}/>);
  view.rerender(<OpticalDeliveryReport taskId="new" fileCount={44}/>);
  finish?.({ok:true,admission:{requested_count:1,admitted_count:1,rejected_count:0,rejections:[]}});
  await waitFor(()=>assert.equal(view.queryByLabelText("下载准备记录"),null));
});
