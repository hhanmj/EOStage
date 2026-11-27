import assert from "node:assert/strict";
import test from "node:test";
import React, { act } from "react";
import { waitFor } from "@testing-library/react";
import { cleanup, fireEvent, render, setupUser } from "../test-utils/render-harness";
import { clearTestDesktopShellBridge, registerTestDesktopShellBridge } from "../lib/desktopShellBridge";
import { createTauriDesktopShellBridge } from "../lib/desktopShellBridgeTauri";
import { NisarArchivedRecoveryConfirmation } from "./NisarArchivedRecoveryConfirmation";

test.afterEach(() => { cleanup(); clearTestDesktopShellBridge(); window.localStorage.clear(); });

const context = { source_task_id: "archived-source", source_item_id: "frozen-scene", expected_source_revision: 25,
  new_task_id: "confirmed-successor", output_root: "F:/fixture-output/confirmed-successor" };
const created = { readiness: { contract: "application_service_readonly_v1", status: "store_ready", store_schema_version: 27 }, core: { tasks: [{ task_id: context.new_task_id }] } };

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

function harness(options: { read?: () => Promise<unknown>; create?: () => Promise<unknown>;
  readFresh?: () => Promise<unknown>; createFresh?: () => Promise<unknown> } = {}) {
  const calls: Array<{ command: string; args: unknown }> = [];
  const completed: string[] = [];
  let cancelled = 0;
  const bridge = createTauriDesktopShellBridge({
    invoke: async (command, args) => {
      calls.push({ command, args });
      if (command === "application_read_nisar_archived_recovery_context_v2") return options.read ? options.read() : context;
      if (command === "application_create_nisar_archived_recovery_v2") return options.create ? options.create() : created;
      if (command === "application_read_nisar_subset_append_context_v2") return options.readFresh ? options.readFresh() : Promise.reject(new Error("no eligible frozen source"));
      if (command === "application_create_nisar_subset_successor_task_v2") return options.createFresh ? options.createFresh() : created;
      throw new Error("Unexpected command in confirmation test");
    },
    getCurrentWindow: () => ({ minimize: async () => undefined, toggleMaximize: async () => undefined,
      close: async () => undefined, innerSize: async () => ({ width: 100, height: 100 }), startResizeDragging: async () => undefined }),
    open: async () => null,
  }, "tauri://localhost/", false);
  registerTestDesktopShellBridge(bridge);
  const props = { sourceTaskId: context.source_task_id, newTaskId: context.new_task_id,
    onClose: () => { cancelled++; }, onCreated: (id: string) => { completed.push(id); } };
  const view = render(<NisarArchivedRecoveryConfirmation {...props} />);
  return { view, props, calls, completed, cancelled: () => cancelled,
    writes: () => calls.filter((call) => call.command === "application_create_nisar_archived_recovery_v2"),
    freshWrites: () => calls.filter((call) => call.command === "application_create_nisar_subset_successor_task_v2") };
}

test("ineligible archived two-scene task offers explicit fresh reconstruction without touching its old checkpoint", async () => {
  const source = {
    task_id: context.source_task_id, source_batch_id: "frozen-batch", task_revision: 8,
    task_status: "cancelled", frozen_aoi_geojson: { type: "Polygon", coordinates: [] },
    selected_frequency: "A", selected_polarizations: ["HH", "HV"],
    output_root: "F:/fixture-output", layout: "direct", concurrency: 2,
    original_item_ids: ["frozen-scene-1", "frozen-scene-2"],
  };
  const pending = deferred<unknown>();
  const state = harness({
    read: async () => { throw { code: "NISAR_ARCHIVED_RECOVERY_SOURCE_INELIGIBLE", message_key: "application_service.unavailable" }; },
    readFresh: async () => source,
    createFresh: () => pending.promise,
  });
  await state.view.findByText(/这是从头开始下载，不会复用原任务的下载进度/);
  const confirm = state.view.getByRole("button", { name: "确认重新创建暂停任务（不继承旧断点）" });
  assert.equal((confirm as HTMLButtonElement).disabled, false);
  assert.equal(state.writes().length, 0);
  assert.equal(state.freshWrites().length, 0);
  fireEvent.click(confirm);
  fireEvent.click(confirm);
  await waitFor(() => assert.equal(state.freshWrites().length, 1));
  const request = (state.freshWrites()[0].args as { request: Record<string, unknown> }).request;
  assert.equal(request.recreate_original_items, true);
  assert.deepEqual(request.source_item_ids, source.original_item_ids);
  assert.equal(request.new_task_id, context.new_task_id);
  assert.equal(request.output_root, context.output_root);
  assert.equal(request.expected_task_revision, source.task_revision);
  assert.equal(state.writes().length, 0);
  await act(async () => { pending.resolve(created); });
  await waitFor(() => assert.deepEqual(state.completed, [context.new_task_id]));
});

test("loading archived recovery context performs only the read and never creates a task", async () => {
  const read = deferred<unknown>();
  const state = harness({ read: () => read.promise });
  assert.equal((state.view.getByRole("button", { name: "确认继承并创建暂停任务" }) as HTMLButtonElement).disabled, true);
  assert.deepEqual(state.calls, [{ command: "application_read_nisar_archived_recovery_context_v2", args: { sourceTaskId: context.source_task_id, newTaskId: context.new_task_id } }]);
  assert.equal(state.writes().length, 0);
  await act(async () => { read.resolve(context); });
  await state.view.findByText(`新输出目录：${context.output_root}`);
  assert.equal(state.writes().length, 0);
  assert.deepEqual(state.completed, []);
});

test("cancel after context confirmation creates nothing and preserves the source", async () => {
  const state = harness();
  await state.view.findByText(`新输出目录：${context.output_root}`);
  await setupUser().click(state.view.getByRole("button", { name: "取消" }));
  assert.equal(state.cancelled(), 1);
  assert.equal(state.writes().length, 0);
  assert.deepEqual(state.completed, []);
});

test("explicit confirmation submits once while busy and sends only frozen identities and timestamp", async () => {
  const create = deferred<unknown>();
  const state = harness({ create: () => create.promise });
  await state.view.findByText(`新输出目录：${context.output_root}`);
  const button = state.view.getByRole("button", { name: "确认继承并创建暂停任务" });
  fireEvent.click(button);
  fireEvent.click(button);
  assert.equal(state.writes().length, 1);
  const args = state.writes()[0].args as { request: Record<string, unknown> };
  assert.deepEqual(Object.keys(args), ["request"]);
  assert.deepEqual(Object.keys(args.request).sort(), ["created_at", "expected_source_revision", "new_task_id", "source_item_id", "source_task_id"]);
  assert.equal(args.request.source_task_id, context.source_task_id);
  assert.equal(args.request.source_item_id, context.source_item_id);
  assert.equal(args.request.new_task_id, context.new_task_id);
  assert.equal(args.request.expected_source_revision, context.expected_source_revision);
  assert.match(String(args.request.created_at), /^\d{4}-\d{2}-\d{2}T/);
  assert.equal(JSON.stringify(args).includes(context.output_root), false);
  assert.equal((state.view.getByRole("button", { name: "取消" }) as HTMLButtonElement).disabled, true);
  await act(async () => { create.resolve(created); });
  await waitFor(() => assert.deepEqual(state.completed, [context.new_task_id]));
});

test("an ambiguous creation failure retries the same task identity and timestamp without duplicate intent", async () => {
  let attempts = 0;
  const state = harness({ create: async () => { if (++attempts === 1) throw new Error("fixture transport uncertainty"); return created; } });
  await state.view.findByText(`新输出目录：${context.output_root}`);
  await setupUser().click(state.view.getByRole("button", { name: "确认继承并创建暂停任务" }));
  await state.view.findByRole("alert");
  assert.deepEqual(state.completed, []);
  await setupUser().click(state.view.getByRole("button", { name: "重试原请求" }));
  await waitFor(() => assert.deepEqual(state.completed, [context.new_task_id]));
  assert.equal(state.writes().length, 2);
  assert.deepEqual(state.writes()[0].args, state.writes()[1].args);
});

test("a failed context lookup leaves confirmation disabled and cannot create", async () => {
  const state = harness({ read: async () => { throw new Error("fixture unavailable"); } });
  await state.view.findByRole("alert");
  const button = state.view.getByRole("button", { name: "确认继承并创建暂停任务" }) as HTMLButtonElement;
  assert.equal(button.disabled, true);
  fireEvent.click(button);
  assert.equal(state.writes().length, 0);
});

test("an ineligible archived multi-scene task shows the protected reason instead of a service outage", async () => {
  const state = harness({
    read: async () => { throw { code: "NISAR_ARCHIVED_RECOVERY_SOURCE_INELIGIBLE", message_key: "application_service.unavailable" }; },
  });
  const alert = await state.view.findByRole("alert");
  assert.match(alert.textContent ?? "", /多景或取消的旧任务/);
  assert.doesNotMatch(alert.textContent ?? "", /application_service\.unavailable/);
  assert.equal((state.view.getByRole("button", { name: "确认继承并创建暂停任务" }) as HTMLButtonElement).disabled, true);
  assert.equal(state.writes().length, 0);
});

test("changing the source invalidates previous consent until the new context is loaded", async () => {
  const nextRead = deferred<unknown>();
  let reads = 0;
  const state = harness({ read: async () => ++reads === 1 ? context : nextRead.promise });
  await state.view.findByText(`新输出目录：${context.output_root}`);
  state.view.rerender(<NisarArchivedRecoveryConfirmation {...state.props} sourceTaskId="different-source" newTaskId="different-successor" />);
  assert.equal((state.view.getByRole("button", { name: "确认继承并创建暂停任务" }) as HTMLButtonElement).disabled, true);
  assert.equal(state.writes().length, 0);
  await act(async () => { nextRead.resolve({ ...context, source_task_id: "different-source", new_task_id: "different-successor", output_root: "F:/fixture-output/different-successor" }); });
});
