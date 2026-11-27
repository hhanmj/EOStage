import assert from "node:assert/strict";
import test from "node:test";
import { createTauriDesktopShellBridge } from "./desktopShellBridgeTauri.ts";

const request = {
  task_id: "task-gacos-result", source_request_task_id: "task-gacos-request",
  source_request_item_id: "gacos-group-1",
  result_url: "https://www.gacos.net/result/fixture-private.tar?signature=fixture-only",
};
const created = { ok: true, task_id: request.task_id, output_root: "F:/fixtures/gacos-results", status: "queued", source: "native-gacos-manual-result" };
function fixture(response: unknown = created) {
  const calls: Array<[string, Record<string, unknown> | undefined]> = [];
  const bridge = createTauriDesktopShellBridge({
    invoke: async (command, args) => { calls.push([command, args]); return response; },
    getCurrentWindow: () => ({ minimize: async () => undefined, toggleMaximize: async () => undefined,
      close: async () => undefined, innerSize: async () => ({ width: 100, height: 100 }), startResizeDragging: async () => undefined }),
    open: async () => null,
  }, "tauri://localhost/", false);
  return { bridge, calls };
}
test("GACOS result bridge sends exact native commands and does not echo links", async () => {
  const { bridge, calls } = fixture();
  const result = await bridge.application.gacosCreateResultTask?.(request);
  assert.deepEqual(result, created);
  assert.deepEqual(calls, [["application_gacos_create_result_task", { request }]]);
  assert(!JSON.stringify(result).includes("fixture-private"));
  const supply = { task_id: request.task_id, result_url: request.result_url };
  const expected = { ok: true, task_id: request.task_id, source: created.source };
  const second = fixture(expected);
  assert.deepEqual(await second.bridge.application.gacosSupplyResultLink?.(supply), expected);
  assert.deepEqual(second.calls, [["application_gacos_supply_result_link", { request: supply }]]);
});
test("GACOS result bridge rejects foreign hosts and excess context before IPC", async () => {
  for (const value of [
    { ...request, result_url: "https://unreviewed.example.invalid/data.tar" },
    { ...request, result_url: "https://www.gacos.net/data.tar#fragment" },
    { ...request, result_url: "https://user:secret@www.gacos.net/data.tar" },
    { ...request, result_url: "ftp://www.gacos.net/data.tar" },
    { ...request, task_id: request.source_request_task_id },
    { ...request, email: "fixture@example.invalid" },
  ]) {
    const { bridge, calls } = fixture();
    const result = await bridge.application.gacosCreateResultTask?.(value);
    assert.equal(result?.ok, false); assert.equal(calls.length, 0);
    assert(!JSON.stringify(result).includes(request.result_url));
  }
});
test("GACOS result bridge rejects wrong task and provider identities in responses", async () => {
  for (const response of [
    { ...created, task_id: "wrong-task" },
    { ...created, source: "native-gacos-local-import" },
    { ...created, result_url: request.result_url },
  ]) {
    const { bridge } = fixture(response);
    const result = await bridge.application.gacosCreateResultTask?.(request);
    assert.equal(result?.ok, false); assert(!JSON.stringify(result).includes(request.result_url));
  }
  const { bridge } = fixture({ ok: true, task_id: "wrong-task", source: created.source });
  assert.equal((await bridge.application.gacosSupplyResultLink?.({ task_id: request.task_id, result_url: request.result_url }))?.ok, false);
});
