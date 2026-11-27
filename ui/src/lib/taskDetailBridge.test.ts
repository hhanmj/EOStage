import assert from "node:assert/strict";
import test from "node:test";

import { createTauriDesktopShellBridge, type TauriBridgeDependencies } from "./desktopShellBridgeTauri.ts";

function dependencies(
  responder: (command: string, args: unknown) => unknown,
  calls: Array<[string, unknown]>,
): TauriBridgeDependencies {
  return {
    invoke: async (command, args) => {
      calls.push([command, args]);
      return responder(command, args);
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

test("Tauri task detail uses bounded events and Store-authorized open-output commands", async () => {
  const calls: Array<[string, unknown]> = [];
  const eventPage = {
    task_id: "task-detail",
    revision: 7,
    page_size: 40,
    has_more: false,
    next_before_sequence: null,
    items: [{
      sequence: 9,
      occurred_at: "2026-08-19T00:00:09Z",
      level: "error",
      event_kind: "failed",
      item_label: "ITEM-A",
      code: "HTTP_TIMEOUT",
      message_zh: "下载失败",
    }],
  };
  const bridge = createTauriDesktopShellBridge(
    dependencies((command) => command === "application_download_task_event_page"
      ? eventPage
      : { task_id: "task-detail", revision: 7 }, calls),
    "tauri://localhost/",
    false,
  );

  assert.deepEqual(await bridge.application.taskEventPage({
    task_id: "task-detail",
    expected_revision: 7,
    before_sequence: null,
    page_size: 40,
  }), { ok: true, page: eventPage });
  assert.deepEqual(await bridge.application.openTaskOutput({
    task_id: "task-detail",
    expected_revision: 7,
  }), { ok: true, task_id: "task-detail", revision: 7 });
  assert.deepEqual(calls, [
    ["application_download_task_event_page", { request: {
      task_id: "task-detail",
      expected_revision: 7,
      before_sequence: null,
      page_size: 40,
    } }],
    ["application_open_download_task_output", { request: {
      task_id: "task-detail",
      expected_revision: 7,
    } }],
  ]);
});

test("Tauri task event parser rejects raw payloads, oversized pages, and malformed output acknowledgements", async () => {
  for (const invalid of [
    {
      task_id: "task-detail", revision: 7, page_size: 40, has_more: false,
      next_before_sequence: null, items: [{
        sequence: 1, occurred_at: "2026-08-19T00:00:01Z", level: "info",
        event_kind: "updated", item_label: null, code: null, message_zh: "状态更新",
        payload_json: "{secret}",
      }],
    },
    {
      task_id: "task-detail", revision: 7, page_size: 40, has_more: false,
      next_before_sequence: null, items: Array.from({ length: 41 }, () => ({
        sequence: 1, occurred_at: "2026-08-19T00:00:01Z", level: "info",
        event_kind: "updated", item_label: null, code: null, message_zh: "状态更新",
      })),
    },
  ]) {
    const bridge = createTauriDesktopShellBridge(
      dependencies(() => invalid, []),
      "tauri://localhost/",
      false,
    );
    assert.equal((await bridge.application.taskEventPage({
      task_id: "task-detail", expected_revision: 7, page_size: 40,
    })).ok, false);
  }

  const bridge = createTauriDesktopShellBridge(
    dependencies(() => ({ task_id: "different-task", revision: 7 }), []),
    "tauri://localhost/",
    false,
  );
  assert.equal((await bridge.application.openTaskOutput({
    task_id: "task-detail", expected_revision: 7,
  })).ok, false);
});
