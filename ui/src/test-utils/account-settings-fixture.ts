// Synthetic desktop only. Never resolves native credentials or contacts a provider.
import { createBrowserFixtureDesktopShellBridge } from "../lib/desktopShellBridgeFixture";
import { createTauriDesktopShellBridge } from "../lib/desktopShellBridgeTauri";
import type { DesktopCredentialBridge, DesktopCredentialField, DesktopShellBridge } from "../lib/desktopShellBridge";

export function createAccountSettingsFixture() {
  const values = new Map<DesktopCredentialField,string>([
    ["cdse_username","fixture-user"],["cdse_password","synthetic password"],
    ["usgs_username","fixture-user"],["usgs_application_token","synthetic app token"],
    ["bhoonidhi_username","fixture-user"],["bhoonidhi_password","synthetic password"],
    ["earthdata_username","fixture-user"],["earthdata_password","synthetic password"],
    ["opentopography_key","synthetic key"],["gacos_email","fixture@example.invalid"],
  ]);
  const state = { writes: 0, reads: 0, checks: [] as string[], statusReads: 0, activeWrites: 0, maxWrites: 0,
    writeFailure: false, statusFailure: false, authenticated: true, readDelay: 0, writeDelay: 2, holdLogin: false,
    opened: [] as string[], forbidden: 0 };
  let releaseLogin: (() => void) | undefined;
  const pause = (ms:number) => new Promise<void>(resolve => setTimeout(resolve,ms));
  const base = createBrowserFixtureDesktopShellBridge();
  const unavailable = { ok:false as const,code:"FIXTURE_UNAVAILABLE",error:"synthetic unavailable" };
  const template = createTauriDesktopShellBridge({invoke:async()=>{state.forbidden++;return unavailable;},
    getCurrentWindow:()=>({minimize:async()=>{},toggleMaximize:async()=>{},close:async()=>{},innerSize:async()=>({width:920,height:620}),startResizeDragging:async()=>{}}),open:async()=>null},"tauri://localhost/",false);
  const pair = (user:DesktopCredentialField,secret:DesktopCredentialField) => values.get(user) && values.get(secret) ? "login:f***" : values.has(user)||values.has(secret)?"incomplete":"none";
  const writeValues = async (entries: [DesktopCredentialField, string][], remove: DesktopCredentialField[] = []) => {
    state.writes++;
    state.activeWrites++;
    state.maxWrites = Math.max(state.activeWrites, state.maxWrites);
    await pause(state.writeDelay);
    state.activeWrites--;
    if (state.writeFailure) return unavailable;
    for (const field of remove) values.delete(field);
    for (const [field, value] of entries) values.set(field, value);
    const current = await credentialPort.getStatus();
    return current.ok ? { ok: true as const, status: current.status, removed: false } : current;
  };
  const credentialPort:DesktopCredentialBridge = { ...template.credentials!,
    readForEdit:async field=>{state.reads++;const value=values.get(field)??null;await pause(state.readDelay);return {ok:true,value};},
    reveal:async field=>{state.reads++;const value=values.get(field)??null;await pause(state.readDelay);return {ok:true,value};},
    updateField:async(field,value)=>{state.writes++;state.activeWrites++;state.maxWrites=Math.max(state.activeWrites,state.maxWrites);
      await pause(state.writeDelay);state.activeWrites--;if(state.writeFailure)return unavailable;
      if(value)values.set(field,value);else values.delete(field);
      if(value && field==="earthdata_token"){values.delete("earthdata_username");values.delete("earthdata_password");}
      if(value && (field==="earthdata_username"||field==="earthdata_password"))values.delete("earthdata_token");
      return {ok:true};},
    getStatus:async()=>{state.statusReads++;if(state.statusFailure)return unavailable;return {ok:true,status:{
      cdse:pair("cdse_username","cdse_password"),usgs:pair("usgs_username","usgs_application_token"),
      bhoonidhi:pair("bhoonidhi_username","bhoonidhi_password"),
      earthdata:values.has("earthdata_token")?"token":pair("earthdata_username","earthdata_password"),
      opentopography:values.has("opentopography_key")?"set":"none",gacos:values.has("gacos_email")?"email:f***":"none",
      earthdata_input:{mode:values.has("earthdata_token")?"token":"login",token:"",username:"",password:""},
    }};},
    getBhoonidhiSessionStatus:async()=>state.statusFailure?unavailable:{ok:true,status:{authenticated:state.authenticated,refresh_available:true,expires_in_seconds:60}},
    checkBhoonidhiAuth:async()=>{state.checks.push("bhoonidhi");return {ok:true,session_available:true,catalogue_verified:false};},
    checkEarthdataAuth:async(force)=>{if(force)state.checks.push("earthdata");return {ok:true,configured:true,status:force?"valid":"unknown",message:""};},
    saveCdseLogin:async(username,password)=>writeValues([["cdse_username",username],["cdse_password",password]]),
    saveUsgsLogin:async(username,token)=>writeValues([["usgs_username",username],["usgs_application_token",token]]),
    saveBhoonidhiLogin:async(username,password)=>writeValues([["bhoonidhi_username",username],["bhoonidhi_password",password]]),
    saveEarthdataToken:async(token)=>writeValues([["earthdata_token",token]],["earthdata_username","earthdata_password"]),
    saveEarthdataLogin:async(username,password)=>writeValues([["earthdata_username",username],["earthdata_password",password]],["earthdata_token"]),
    saveOpentopographyKey:async(key)=>writeValues([["opentopography_key",key]]),
    saveGacosEmail:async(email)=>writeValues([["gacos_email",email]]),
  };
  const cdseStatus = () => ({network_mode:"system" as const,authenticated:state.authenticated,refresh_available:true,expires_in_seconds:60});
  const bridge:DesktopShellBridge = {...base,kind:"tauri",credentials:credentialPort,application:{...base.application,
    getSarDownloadPolicySettings:async()=>({mode:"manual",max_concurrent_items:2,organize_by_product:false,default_output_root:"C:\\synthetic\\downloads"}),
    cdseSessionStatus:async()=>state.statusFailure?unavailable:{ok:true,status:cdseStatus()},
    usgsSessionStatus:async()=>state.statusFailure?unavailable:{ok:true,status:{state:state.authenticated?"authenticated":"ready",authenticated:state.authenticated}},
    cdseLogin:async request=>{state.checks.push("cdse");if(request.username||request.password)throw new Error("use saved native material");
      if(state.holdLogin)await new Promise<void>(resolve=>{releaseLogin=resolve;}); return {ok:true,status:cdseStatus()};},
    usgsLogin:async request=>{state.checks.push("usgs");if(request.username||request.application_token)throw new Error("use saved native material");return {ok:true,status:{state:"authenticated",authenticated:true}};},
    openExternalUrl:async url=>{state.opened.push(url);return {ok:true,url};},
  }};
  return {bridge,state,values,releaseLogin:()=>releaseLogin?.()};
}
