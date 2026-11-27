import assert from "node:assert/strict";
import test from "node:test";

import { createTauriDesktopShellBridge, type TauriBridgeDependencies } from "./desktopShellBridgeTauri.ts";
import type { DesktopArchiveDownloadTaskRequest } from "./desktopShellBridge.ts";

function dependencies(result: unknown, calls: Array<[string, unknown]>): TauriBridgeDependencies {
  return {
    invoke: async (command, args) => {
      calls.push([command, args]);
      return result;
    },
    getCurrentWindow: () => ({
      minimize: async () => undefined,
      toggleMaximize: async () => undefined,
      close: async () => undefined,
      innerSize: async () => ({ width: 100, height: 100 }),
      startResizeDragging: async () => undefined,
    }),
    open: async () => null,
  };
}

test("automatic archive returns only a fixed acknowledgement and ignores no terminal restore payload", async () => {
  const request: DesktopArchiveDownloadTaskRequest = {
    task_id: "terminal-041",
    expected_revision: 9,
    archived_at: "2026-08-17T00:00:00.000Z",
  };
  const ack = {
    task_id: request.task_id,
    revision: request.expected_revision,
    archived_at: "2026-08-17T00:00:00Z",
  };
  const calls: Array<[string, unknown]> = [];
  const bridge = createTauriDesktopShellBridge(dependencies(ack, calls), "tauri://localhost/", false);
  assert.deepEqual(await bridge.application.archiveDownloadTask(request), { ok: true, archive: ack });
  assert.deepEqual(calls, [["application_archive_download_task", { request }]]);

  for (const unbounded of [
    { ...ack, tasks: Array.from({ length: 41 }, (_, index) => ({ task_id: `terminal-${index}` })) },
    { ...ack, core: { tasks: Array.from({ length: 41 }, () => ({})) } },
    { ...ack, snapshot_json: "{}" },
  ]) {
    const rejected = createTauriDesktopShellBridge(dependencies(unbounded, []), "tauri://localhost/", false);
    assert.equal((await rejected.application.archiveDownloadTask(request)).ok, false);
  }
});
