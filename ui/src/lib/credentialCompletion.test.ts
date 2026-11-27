import assert from "node:assert/strict";
import test from "node:test";
import { createTauriDesktopShellBridge, type TauriBridgeDependencies } from "./desktopShellBridgeTauri.ts";

function bridge(invoke: TauriBridgeDependencies["invoke"]) {
  return createTauriDesktopShellBridge({
    invoke, open: async () => null,
    getCurrentWindow: () => ({
      minimize: async () => undefined, toggleMaximize: async () => undefined,
      close: async () => undefined, startDragging: async () => undefined,
      startResizeDragging: async () => undefined,
    }),
  }, "tauri://localhost/", false).credentials!;
}

test("native credential calls use exact commands and closed field argument without configured-auth conflation", async () => {
  const calls: unknown[] = [];
  const credentials = bridge(async (command, args) => {
    calls.push([command, args]);
    if (command === "credential_bhoonidhi_auth") return { ok: true, session_available: true, catalogue_verified: false };
    if (command === "credential_reveal") return { ok: true, value: "fixture-only" };
    throw new Error("unexpected command");
  });
  assert.deepEqual(await credentials.checkBhoonidhiAuth(), { ok: true, session_available: true, catalogue_verified: false });
  assert.deepEqual(await credentials.reveal("bhoonidhi_password"), { ok: true, value: "fixture-only" });
  assert.deepEqual(calls, [["credential_bhoonidhi_auth", undefined], ["credential_reveal", { field: "bhoonidhi_password" }]]);
});

test("authentication preserves timeout code; reveal failures discard raw payloads", async () => {
  const credentials = bridge(async () => { throw { code: "BHOONIDHI_AUTH_TIMEOUT", message_key: "fixture-sensitive" }; });
  const result = await credentials.checkBhoonidhiAuth();
  assert.equal(result.ok, false);
  if (!result.ok) assert.equal(result.code, "BHOONIDHI_AUTH_TIMEOUT");
  assert.ok(!JSON.stringify(await credentials.reveal("bhoonidhi_password")).includes("fixture-sensitive"));
});

test("configured-only, catalogue-verified and malformed replies cannot certify authentication", async () => {
  for (const response of [{ ok: true, configured: true }, { ok: true, session_available: true, catalogue_verified: true }, null]) {
    assert.equal((await bridge(async () => response).checkBhoonidhiAuth()).ok, false);
  }
  assert.deepEqual(await bridge(async () => ({ ok: true, value: null })).reveal("gacos_email"), { ok: true, value: null });
});
