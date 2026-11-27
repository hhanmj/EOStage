import assert from "node:assert/strict";
import test from "node:test";
import { createTauriDesktopShellBridge } from "./desktopShellBridgeTauri.ts";

test("archived recovery requires a bound source context and never sends paths or credentials", async () => {
  const calls: Array<[string, unknown]> = [];
  const context = { source_task_id: "old", source_item_id: "scene", expected_source_revision: 25, new_task_id: "new", output_root: "F:/output/new" };
  const bridge = createTauriDesktopShellBridge({
    invoke: async (command, args) => { calls.push([command, args]); return command.includes("context") ? context : { readiness: { contract: "application_service_readonly_v1", status: "store_ready", store_schema_version: 27 }, core: {} }; },
    getCurrentWindow: () => ({ minimize: async () => undefined, toggleMaximize: async () => undefined, close: async () => undefined, innerSize: async () => ({ width: 100, height: 100 }), startResizeDragging: async () => undefined }),
    open: async () => null,
  }, "tauri://localhost/", false);
  assert.deepEqual(await bridge.application.readNisarArchivedRecoveryContextV2("old", "new"), { ok: true, context });
  const request = { source_task_id: "old", source_item_id: "scene", expected_source_revision: 25, new_task_id: "new", created_at: "2026-10-13T00:00:00Z" };
  assert.equal((await bridge.application.createNisarArchivedRecoveryV2(request)).ok, true);
  for (const invalid of [{ ...request, output_root: "F:/other" }, { ...request, token: "placeholder" }, { ...request, new_task_id: "old" }, { ...request, new_task_id: "../escape" }]) {
    assert.equal((await bridge.application.createNisarArchivedRecoveryV2(invalid)).ok, false);
  }
  assert.deepEqual(calls, [["application_read_nisar_archived_recovery_context_v2", { sourceTaskId: "old", newTaskId: "new" }], ["application_create_nisar_archived_recovery_v2", { request }]]);
});
