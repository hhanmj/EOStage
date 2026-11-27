import assert from "node:assert/strict";
import test from "node:test";
import { networkProxyPreference, testNetworkConnection } from "./networkSettings.ts";

test("network preference maps persisted modes and omits proxy credentials", () => {
  assert.deepEqual(networkProxyPreference({ proxy_enabled: false, proxy_url: "" }), { mode: "direct" });
  assert.deepEqual(networkProxyPreference({ proxy_enabled: true, proxy_url: "" }), { mode: "auto" });
  assert.deepEqual(networkProxyPreference({ proxy_enabled: true, proxy_url: "http://127.0.0.1:7897" }),
    { mode: "manual", host: "127.0.0.1", port: 7897 });
  assert.throws(() => networkProxyPreference({ proxy_enabled: true, proxy_url: "http://user:secret@127.0.0.1:7897" }));
});

test("connection test sends no credentials and accepts only a redacted transport stage", async () => {
  const calls: string[] = [];
  assert.equal(await testNetworkConnection(async (command, payload) => {
    calls.push(command);
    assert.equal(payload, undefined);
    return "http_reached";
  }), "http_reached");
  assert.deepEqual(calls, ["application_test_network_connection"]);
  await assert.rejects(testNetworkConnection(async () => ({ stage: "http_reached", token: "fixture-secret" })));
});
