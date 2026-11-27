import assert from "node:assert/strict";
import test from "node:test";
import { TaskPageRequestCoordinator } from "./taskPageRequestCoordinator.ts";

function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>((r) => { resolve = r; }); return { promise, resolve }; }

test("read-only detail can open while live revisions advance, without weakening close or switch isolation", async () => {
  const coordinator = new TaskPageRequestCoordinator();
  const pending = deferred<string>();
  coordinator.observeRevision("A", 5);
  const read = coordinator.request("A", 4, 0, () => pending.promise, "observed");
  coordinator.observeRevision("A", 6);
  pending.resolve("coherent-read-snapshot");
  assert.equal(await read, "coherent-read-snapshot");

  const late = deferred<string>();
  const closed = coordinator.request("A", 6, 1, () => late.promise, "observed");
  coordinator.invalidate();
  late.resolve("after-close");
  assert.equal(await closed, null);

  const switched = deferred<string>();
  const first = coordinator.request("A", 6, 0, () => switched.promise, "observed");
  const second = coordinator.request("B", 1, 0, async () => "B", "observed");
  switched.resolve("after-switch");
  assert.equal(await first, null);
  assert.equal(await second, "B");
});

test("a newer task page request invalidates an older task response", async () => {
  const first = deferred<string>();
  const second = deferred<string>();
  const coordinator = new TaskPageRequestCoordinator();
  const a = coordinator.request("A", 1, 0, () => first.promise);
  const b = coordinator.request("B", 2, 0, () => second.promise);
  first.resolve("old");
  assert.equal(await a, null);
  second.resolve("new");
  assert.equal(await b, "new");
});

test("a newer observed revision invalidates an in-flight page for the same task", async () => {
  const pending = deferred<string>();
  const coordinator = new TaskPageRequestCoordinator();
  coordinator.observeRevision("A", 4);
  const read = coordinator.request("A", 4, 0, () => pending.promise);
  coordinator.observeRevision("A", 5);
  pending.resolve("stale-page");
  assert.equal(await read, null);
});

test("a request establishes its revision floor and an already stale request never reads", async () => {
  const pending = deferred<string>();
  const coordinator = new TaskPageRequestCoordinator();
  const read = coordinator.request("A", 4, 0, () => pending.promise);
  coordinator.observeRevision("A", 5);
  pending.resolve("stale-page");
  assert.equal(await read, null);
  let calls = 0;
  assert.equal(await coordinator.request("A", 4, 1, async () => { calls += 1; return "must-not-read"; }), null);
  assert.equal(calls, 0);
});
