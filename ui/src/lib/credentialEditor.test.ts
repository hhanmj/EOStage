import assert from "node:assert/strict";
import test from "node:test";
import { setTimeout as delay } from "node:timers/promises";
import { CredentialEditorWriter, isCredentialEditorField, validCredentialEditorValue } from "./credentialEditor.ts";
import { createTauriDesktopShellBridge } from "./desktopShellBridgeTauri.ts";

test("field edits serialize, coalesce only unsent values, and reads wait for the final persisted text", async () => {
  const values = new Map<string,string>(); const calls: string[] = [];
  let active = 0, maxActive = 0;
  const writer = new CredentialEditorWriter({
    updateField: async (field,value) => { active++;maxActive=Math.max(active,maxActive);calls.push(value);
      await delay(5); if(value) values.set(field,value); else values.delete(field); active--; return {ok:true}; },
    readForEdit: async field => ({ok:true,value:values.get(field) ?? null}),
  });
  const first=writer.write("cdse_username","测");await delay(1);
  const skipped=writer.write("cdse_username","测试");
  const last=writer.write("cdse_username","测试 user ");
  const secret=writer.write("cdse_password","fixture secret ");
  assert.equal((await skipped).ok,true);await Promise.all([first,last,secret]);
  assert.equal(maxActive,1); assert.deepEqual(calls,["测","测试 user ","fixture secret "]);
  await writer.write("cdse_username","");
  assert.deepEqual(await writer.read(["cdse_username","cdse_password"]),{cdse_username:"",cdse_password:"fixture secret "});
});

test("a new edit from a save acknowledgement is not stranded between drain and finalization", async () => {
  const values:string[]=[];
  const writer=new CredentialEditorWriter({ updateField:async(_field,value)=>{values.push(value);return {ok:true};},readForEdit:async()=>({ok:true,value:null}) });
  let completed=false;
  const next=writer.write("usgs_username","first").then(()=>writer.write("usgs_username","last")).then(()=>{completed=true;});
  await Promise.race([next,delay(50)]);
  assert.equal(completed,true); assert.deepEqual(values,["first","last"]);
});

test("autosave failures remain failures and a late check never validates newer edits", async () => {
  let fail=true;
  const writer=new CredentialEditorWriter({updateField:async()=>{if(fail)throw new Error("do not reflect fixture-secret");return {ok:true};},readForEdit:async()=>({ok:true,value:null})});
  const result=await writer.write("earthdata_token","fixture");assert.equal(result.ok,false);assert.ok(!JSON.stringify(result).includes("fixture-secret"));
  const epoch=writer.revision("earthdata");fail=false;await writer.write("earthdata_token","fixture-next");
  writer.checked("earthdata",epoch,true);assert.equal(writer.needsCheck("earthdata"),true);
  writer.checked("earthdata",writer.revision("earthdata"),true);assert.equal(writer.needsCheck("earthdata"),false);
});

test("failed-save metadata survives reads without retaining values and old acknowledgements cannot erase a newer failure", async () => {
  let finish:(()=>void)|undefined;let fail=true;
  const writer=new CredentialEditorWriter({updateField:async()=>{await new Promise<void>(resolve=>{finish=resolve;});return fail?{ok:false,error:"failed",code:"FIXTURE"}:{ok:true};},readForEdit:async()=>({ok:true,value:"old-saved"})});
  const first=writer.write("cdse_password","synthetic-failed-secret");
  await new Promise(resolve=>setTimeout(resolve,0));finish!();assert.equal((await first).ok,false);
  assert.deepEqual(writer.failedFields(["cdse_password"]),["cdse_password"]);
  assert.equal((await writer.read(["cdse_password"])).cdse_password,"old-saved");
  assert.ok(!JSON.stringify(writer).includes("synthetic-failed-secret"));
  fail=false;const valid=writer.write("cdse_password","synthetic-new-value");
  await new Promise(resolve=>setTimeout(resolve,0));
  const invalid=await writer.write("cdse_password","invalid\u0000value");assert.equal(invalid.ok,false);
  finish!();await valid;assert.deepEqual(writer.failedFields(["cdse_password"]),["cdse_password"]);
  const corrected=writer.write("cdse_password","synthetic-corrected");await new Promise(resolve=>setTimeout(resolve,0));finish!();await corrected;
  assert.deepEqual(writer.failedFields(["cdse_password"]),[]);
});

test("explicit check observations are bound to the current field revision and non-secret source", async () => {
  const writer=new CredentialEditorWriter({updateField:async()=>({ok:true}),readForEdit:async()=>({ok:true,value:null})});
  writer.checked("earthdata",0,"valid","netrc");
  assert.equal(writer.checkedState("earthdata","netrc"),"valid");
  assert.equal(writer.checkedState("earthdata","env-token"),null);
  await writer.write("earthdata_token","synthetic");
  assert.equal(writer.checkedState("earthdata","netrc"),null);
  writer.checked("earthdata",0,"valid","token");
  assert.equal(writer.checkedState("earthdata","token"),null);
  writer.checked("earthdata",1,"unknown","token");
  assert.equal(writer.checkedState("earthdata","token"),"unknown");
  assert.equal(writer.needsCheck("earthdata"),true);
  writer.checked("earthdata",1,"unavailable","token");
  assert.equal(writer.checkedState("earthdata","token"),"unavailable");
});

test("credential editor IPC stays closed-field and never reflects material through mutation or errors", async () => {
  const calls:Array<[string,unknown]>=[];let mode="valid";
  const bridge=createTauriDesktopShellBridge({invoke:async(command,args)=>{
    calls.push([command,args]);
    if(mode==="throw")throw new Error("fixture-secret-should-not-escape");
    return command==="credential_read_for_edit"?{ok:true,value:"synthetic editable value"}:{ok:true,...(mode==="extra"?{value:"fixture-secret-should-not-escape"}:{})};
  }, getCurrentWindow:()=>({minimize:async()=>{},toggleMaximize:async()=>{},close:async()=>{},innerSize:async()=>({width:920,height:620}),startResizeDragging:async()=>{}}),open:async()=>null},"tauri://localhost/",false);
  const port=bridge.credentials!;
  assert.deepEqual(await port.updateField("cdse_password",""),{ok:true});
  assert.deepEqual(calls[0],["credential_update_field",{field:"cdse_password",value:""}]);
  assert.deepEqual(await port.readForEdit("cdse_username"),{ok:true,value:"synthetic editable value"});
  mode="extra";assert.equal((await port.updateField("cdse_password","fixture")).ok,false);
  mode="throw";assert.ok(!JSON.stringify(await port.updateField("cdse_password","fixture")).includes("fixture-secret"));
  const count=calls.length;
  assert.equal((await port.updateField("not_a_field" as never,"fixture")).ok,false);
  assert.equal((await port.updateField("cdse_password","a\0b")).ok,false);assert.equal(calls.length,count);
  assert.equal(isCredentialEditorField("gacos_email"),true);assert.equal(validCredentialEditorValue(""),true);
  assert.equal(validCredentialEditorValue("😀".repeat(641)),false);
});
