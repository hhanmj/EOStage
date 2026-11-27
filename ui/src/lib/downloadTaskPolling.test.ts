import assert from "node:assert/strict";
import test from "node:test";

import { LatestStatusCoordinator } from "./latestStatusCoordinator.ts";
import { ActiveOnlyStatusPolling } from "./downloadTaskPolling.ts";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((nextResolve) => {
    resolve = nextResolve;
  });
  return { promise, resolve };
}

function fakeTimers() {
  let nextId = 0;
  const callbacks = new Map<number, () => void>();
  return {
    setInterval(callback: () => void) {
      const id = ++nextId;
      callbacks.set(id, callback);
      return id;
    },
    clearInterval(id: number) {
      callbacks.delete(id);
    },
    advance() {
      for (const callback of [...callbacks.values()]) callback();
    },
    get count() {
      return callbacks.size;
    },
  };
}

test("startup hydrates immediately, then polls only mutable tasks at the bounded cadence", async () => {
  const reads = ["startup", "tick"];
  const published: string[] = [];
  const coordinator = new LatestStatusCoordinator<string>({
    read: async () => reads.shift()!,
    publish: (value) => published.push(value),
  });
  const timers = fakeTimers();
  const polling = new ActiveOnlyStatusPolling(coordinator, timers, 900);

  await polling.hydrate();
  assert.deepEqual(published, ["startup"]);
  assert.equal(timers.count, 0);

  polling.setTaskStates(["running"]);
  assert.equal(timers.count, 1);
  timers.advance();
  await new Promise<void>((resolve) => setImmediate(resolve));
  assert.deepEqual(published, ["startup", "tick"]);

  polling.setTaskStates(["completed", "cancelled"]);
  assert.equal(timers.count, 0);
});

test("timer ticks coalesce an in-flight request and a disposed lifecycle cannot publish stale data", async () => {
  const pending = deferred<string>();
  const published: string[] = [];
  let reads = 0;
  const coordinator = new LatestStatusCoordinator<string>({
    read: () => {
      reads += 1;
      return pending.promise;
    },
    publish: (value) => published.push(value),
  });
  const timers = fakeTimers();
  const polling = new ActiveOnlyStatusPolling(coordinator, timers, 900);

  polling.setTaskStates(["running"]);
  timers.advance();
  timers.advance();
  await Promise.resolve();
  assert.equal(reads, 1);

  polling.dispose();
  assert.equal(timers.count, 0);
  pending.resolve("late");
  await Promise.resolve();
  assert.deepEqual(published, []);
});

test("an action refresh waits for the active poll and publishes the newer response last", async () => {
  const inFlight = deferred<string>();
  const afterAction = deferred<string>();
  const values: string[] = [];
  const coordinator = new LatestStatusCoordinator<string>({
    read: () => (values.length === 0 ? inFlight.promise : afterAction.promise),
    publish: (value) => values.push(value),
  });
  const timers = fakeTimers();
  const polling = new ActiveOnlyStatusPolling(coordinator, timers, 900);

  polling.setTaskStates(["running"]);
  timers.advance();
  const refresh = polling.refreshAfterAction();
  inFlight.resolve("before-action");
  await Promise.resolve();
  afterAction.resolve("after-action");
  await refresh;
  assert.deepEqual(values, ["before-action", "after-action"]);
});

test("only Store mutable states schedule polling; unknown and restore errors stop it", () => {
  const coordinator = new LatestStatusCoordinator<string>({ read: async () => "unused", publish: () => undefined });
  const timers = fakeTimers();
  const polling = new ActiveOnlyStatusPolling(coordinator, timers, 900);

  for (const state of ["queued", "running", "paused", "cancelling"]) {
    polling.setTaskStates([state]);
    assert.equal(timers.count, 1, state);
  }
  for (const state of ["restore_error", "unknown", "idle", "completed", "failed", "cancelled", "interrupted", "timeout"]) {
    polling.setTaskStates([state]);
    assert.equal(timers.count, 0, state);
  }
});
