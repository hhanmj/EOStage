import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { createTauriDesktopShellBridge, type TauriBridgeDependencies } from "./desktopShellBridgeTauri.ts";

function bridgeReturning(response: unknown) {
  const calls: string[] = [];
  const dependencies: TauriBridgeDependencies = {
    invoke: async (command) => { calls.push(command); return structuredClone(response); },
    getCurrentWindow: () => ({
      minimize: async () => undefined, toggleMaximize: async () => undefined,
      close: async () => undefined, innerSize: async () => ({ width: 920, height: 620 }),
      startResizeDragging: async () => undefined,
    }),
    open: async () => null,
  };
  return { bridge: createTauriDesktopShellBridge(dependencies, "tauri://localhost/", false), calls };
}

function nativeCredentialStatus() {
  return { ok: true, cdse: "login:c***", usgs: "login:u***",
    earthdata: "none", earthdata_input: { mode: "none", token: "", username: "", password: "" },
    bhoonidhi: "login:f***", opentopography: "none", gacos: "none" };
}

// Keep the fixture aligned with the actual Rust Serialize envelope, not a UI-invented mock.
test("native credential mutation fixture matches the nested Rust status field", () => {
  const source = readFileSync(new URL("../../../src-tauri/src/commands.rs", import.meta.url), "utf8");
  assert.match(source, /pub struct DesktopCredentialMutation\s*\{\s*pub ok: bool,\s*pub status: DesktopCredentialStatus,\s*pub removed: bool,/);
});

test("all credential mutations accept the native nested success envelope", async () => {
  const status = nativeCredentialStatus();
  const { bridge, calls } = bridgeReturning({ ok: true, status, removed: false });
  const credentials = bridge.credentials!;
  const results = [
    await credentials.saveCdseLogin("cdse-user", "fixture-only"),
    await credentials.clearCdse(),
    await credentials.saveUsgsLogin("usgs-user", "fixture-token"),
    await credentials.clearUsgs(),
    await credentials.saveBhoonidhiLogin("fixture-user", "fixture-only"),
    await credentials.clearBhoonidhi(),
    await credentials.saveEarthdataToken("fixture-only"),
    await credentials.saveEarthdataLogin("fixture-user", "fixture-only"),
    await credentials.clearEarthdata(),
    await credentials.saveOpentopographyKey("fixture-only"),
    await credentials.clearOpentopographyKey(),
    await credentials.saveGacosEmail("fixture@example.invalid"),
    await credentials.clearGacosEmail(),
  ];
  assert.equal(calls.length, 13);
  assert.deepEqual(calls.slice(0, 4), [
    "credential_save_cdse_login",
    "credential_clear_cdse",
    "credential_save_usgs_login",
    "credential_clear_usgs",
  ]);
  for (const result of results) {
    assert.equal(result.ok, true, JSON.stringify(result));
    if (result.ok) {
      assert.equal(result.status.bhoonidhi, "login:f***");
      assert.equal(result.status.cdse, "login:c***");
      assert.equal(result.status.usgs, "login:u***");
      assert.equal(result.status.earthdata_input.password, "");
      assert.equal(result.removed, false);
    }
  }
});

test("credential mutation rejects flattened, failed and malformed acknowledgements", async () => {
  for (const response of [
    { ...nativeCredentialStatus(), removed: false },
    { ok: false, status: nativeCredentialStatus(), removed: false },
    { ok: true, status: nativeCredentialStatus() },
    { ok: true, status: { ...nativeCredentialStatus(), ok: false }, removed: false },
  ]) {
    const { bridge } = bridgeReturning(response);
    const result = await bridge.credentials!.clearBhoonidhi();
    assert.equal(result.ok, false);
    if (!result.ok) assert.equal(result.code, "CREDENTIAL_RESPONSE_INVALID");
  }
});

const fingerprint = "a".repeat(64);
const itemId = `gacos-request-${"b".repeat(64)}`;
function nativeGacosResponse(forceReapply: boolean) {
  const taskId = `gacos-request-${fingerprint}${forceReapply ? "-00000000-0000-4000-8000-000000000000" : ""}`;
  return {
    ok: true, task_id: taskId, task_status: "completed", submitted_count: 1,
    application_completed: true, manual_action_required: false, automatic_retry_allowed: false,
    message: "申请已提交",
    request_groups: [{ item_id: itemId, dates: ["20260901"], hour: 1, minute: 2, bbox: [110, 20, 111, 21] }],
    records: [{ attempt_id: `${taskId}:${itemId}:0`, task_id: taskId, item_id: itemId,
      attempt_number: 0, state: "submitted", reason_code: "EXPLICIT_SUCCESS_EVIDENCE", http_status: 200,
      automatic_retry_allowed: false, created_at: "2026-09-07T00:00:00Z", updated_at: "2026-09-07T00:00:01Z" }],
  };
}

test("native GACOS fixture follows producer IDs and the Store's 256-byte attempt contract", () => {
  const service = readFileSync(new URL("../../../crates/insar-application-service/src/gacos_service.rs", import.meta.url), "utf8");
  const store = readFileSync(new URL("../../../crates/insar-store/src/gacos_submission.rs", import.meta.url), "utf8");
  assert.ok(service.includes('attempt_id: format!("{}:{}:0", request.task_id, item.item_id)'));
  assert.ok(service.includes('item_id: format!("gacos-request-{request_fingerprint}")'));
  assert.match(store, /fn canonical_attempt_id[\s\S]*?value\.len\(\) > 256/);
});

for (const forceReapply of [false, true]) {
  test(`GACOS submit and status accept real fingerprint-based attempt IDs (force=${forceReapply})`, async () => {
    const response = nativeGacosResponse(forceReapply);
    assert.equal(response.records[0].attempt_id.length, forceReapply ? 196 : 159);
    const { bridge, calls } = bridgeReturning(response);
    const result = await bridge.application.gacosSubmitRequest({
      prepared_batch_id: `gacos-plan-${fingerprint}`, plan_fingerprint_sha256: fingerprint,
      task_id: response.task_id, confirm: true, force_reapply: forceReapply,
    });
    assert.equal(result.ok, true, JSON.stringify(result));
    const status = await bridge.application.gacosRequestStatus(response.task_id);
    assert.equal(status.ok, true, JSON.stringify(status));
    if (status.ok) assert.equal(status.records[0].attempt_id, response.records[0].attempt_id);
    assert.deepEqual(calls, ["application_gacos_submit_request", "application_gacos_request_status"]);
  });
}

test("GACOS attempt validation retains the native bound, opaque alphabet and task identity", async () => {
  for (const invalidId of ["a".repeat(257), "has space", "含中文", "url/path", "\n", "a\n", "a".repeat(256) + "\n"]) {
    const response = nativeGacosResponse(false);
    response.records[0].attempt_id = invalidId;
    const { bridge } = bridgeReturning(response);
    assert.equal((await bridge.application.gacosRequestStatus(response.task_id)).ok, false);
  }
  const response = nativeGacosResponse(false);
  response.records[0].task_id = "different-task";
  const { bridge } = bridgeReturning(response);
  const invalid = await bridge.application.gacosRequestStatus(response.task_id);
  assert.equal(invalid.ok, false);
  if (!invalid.ok) assert.equal(invalid.code, "GACOS_RESPONSE_INVALID");
});

test("GACOS attempt IDs accept the exact 256-byte Store boundary", async () => {
  const response = nativeGacosResponse(false);
  response.records[0].attempt_id = "a".repeat(256);
  const { bridge } = bridgeReturning(response);
  assert.equal((await bridge.application.gacosRequestStatus(response.task_id)).ok, true);
});

test("valid rejected and uncertain GACOS records are readable without being turned into success or resent", async () => {
  for (const state of ["rejected", "submission_uncertain", "manual_verification_required", "reserved"]) {
    const response = nativeGacosResponse(false);
    response.records[0].state = state;
    response.records[0].reason_code = "FIXTURE_OUTCOME";
    response.task_status = "paused";
    response.submitted_count = 0;
    response.application_completed = false;
    response.manual_action_required = ["submission_uncertain", "manual_verification_required"].includes(state);
    response.message = "申请状态待核对";
    const { bridge, calls } = bridgeReturning(response);
    const result = await bridge.application.gacosRequestStatus(response.task_id);
    assert.equal(result.ok, true);
    if (result.ok) {
      assert.equal(result.records[0].state, state);
      assert.equal(result.application_completed, false);
      assert.equal(result.automatic_retry_allowed, false);
    }
    assert.deepEqual(calls, ["application_gacos_request_status"]);
  }
});
