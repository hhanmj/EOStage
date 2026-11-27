import assert from "node:assert/strict";
import test from "node:test";

import { ensureProviderSessionForTask, taskSessionProvider } from "./providerSessionGate.ts";

const cdseTask = { batch_id: "sentinel2-archive-x", provider_id: "copernicus.dataspace.odata" };
const usgsTask = { batch_id: "landsat-delivery-x", provider_id: "usgs.eros.m2m" };
const formatError = (error: unknown) => (error instanceof Error ? error.message : String(error));

function ensurers(results: {
  cdse?: () => Promise<any>;
  usgs?: () => Promise<any>;
}) {
  const calls: string[] = [];
  return {
    calls,
    ensure: {
      cdse: async () => {
        calls.push("cdse");
        return results.cdse ? results.cdse() : { ok: true, value: { authenticated: true }, note: "" };
      },
      usgs: async () => {
        calls.push("usgs");
        return results.usgs ? results.usgs() : { ok: true, value: { authenticated: true }, note: "" };
      },
    },
  };
}

test("official-archive tasks need their provider session; other deliveries do not", () => {
  assert.deepEqual(taskSessionProvider(cdseTask), { key: "cdse", label: "CDSE" });
  assert.deepEqual(taskSessionProvider(usgsTask), { key: "usgs", label: "USGS" });
  assert.equal(taskSessionProvider({ batch_id: "sentinel2-bands-x" }), null);
  assert.equal(taskSessionProvider({ batch_id: "optderived-task-opt-1", provider_id: "copernicus.dataspace.odata" }), null);
  assert.equal(taskSessionProvider({ batch_id: "landsat-assets-x" }), null);
  assert.equal(taskSessionProvider({ provider_id: "asf.catalog" }), null);
  assert.equal(taskSessionProvider({ provider_id: "isro.bhoonidhi" }), null);
  assert.equal(taskSessionProvider(null), null);
});

test("an authenticated session lets the task continue", async () => {
  const { calls, ensure } = ensurers({});
  assert.deepEqual(await ensureProviderSessionForTask(cdseTask, ensure, formatError), { ok: true, provider: "cdse" });
  assert.deepEqual(calls, ["cdse"]);
});

test("a session that stays unauthenticated blocks the task with a login prompt", async () => {
  const { ensure } = ensurers({ usgs: async () => ({ ok: true, value: { authenticated: false }, note: "" }) });
  assert.deepEqual(await ensureProviderSessionForTask(usgsTask, ensure, formatError), {
    ok: false,
    provider: "usgs",
    label: "USGS",
    message: "需要登录 USGS 后才能继续",
    reason: null,
  });
});

test("a failed login keeps its real reason", async () => {
  const { ensure } = ensurers({ cdse: async () => ({ ok: false, error: "CDSE 用户名或密码不正确" }) });
  const result = await ensureProviderSessionForTask(cdseTask, ensure, formatError);
  assert.equal(result.ok, false);
  assert.equal(result.ok ? null : result.reason, "CDSE 用户名或密码不正确");
});

test("a thrown session error blocks the task instead of resuming it", async () => {
  const { ensure } = ensurers({ cdse: async () => { throw new Error("网络不可用"); } });
  const result = await ensureProviderSessionForTask(cdseTask, ensure, formatError);
  assert.equal(result.ok, false);
  assert.equal(result.ok ? null : result.reason, "网络不可用");
});

test("tasks without a provider session never start a login", async () => {
  const { calls, ensure } = ensurers({});
  for (const task of [{ batch_id: "sentinel2-bands-x" }, { provider_id: "asf.catalog" }, undefined]) {
    assert.deepEqual(await ensureProviderSessionForTask(task, ensure, formatError), { ok: true, provider: null });
  }
  assert.deepEqual(calls, []);
});
