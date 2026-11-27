import assert from "node:assert/strict";
import test from "node:test";

import {
  DemManagerStatusCoordinator,
  DemManagerStatusTimeoutError,
} from "./demManagerStatusCoordinator.ts";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((nextResolve, nextReject) => {
    resolve = nextResolve;
    reject = nextReject;
  });
  return { promise, resolve, reject };
}

async function waitFor(predicate: () => boolean, timeoutMs = 250) {
  const started = Date.now();
  while (!predicate()) {
    if (Date.now() - started > timeoutMs) throw new Error("timed out waiting for condition");
    await new Promise((resolve) => setTimeout(resolve, 1));
  }
}

test("a rejected manager read publishes an error and releases the shared flight", async () => {
  const failure = new Error("bridge rejected");
  const values: number[] = [];
  const errors: unknown[] = [];
  const coordinator = new DemManagerStatusCoordinator<number>({
    read: async () => Promise.reject(failure),
    onValue: (value) => values.push(value),
    onError: (error) => errors.push(error),
    timeoutMs: 50,
  });

  assert.equal(await coordinator.poll(), null);
  assert.deepEqual(values, []);
  assert.deepEqual(errors, [failure]);
  assert.equal(coordinator.hasInFlight, false);
});

test("a timed out read cannot publish late or release a newer request", async () => {
  const first = deferred<number>();
  const second = deferred<number>();
  const reads = [first, second];
  const values: number[] = [];
  const errors: unknown[] = [];
  const coordinator = new DemManagerStatusCoordinator<number>({
    read: () => reads.shift()!.promise,
    onValue: (value) => values.push(value),
    onError: (error) => errors.push(error),
    timeoutMs: 15,
  });

  const timedOut = coordinator.poll();
  assert.ok(timedOut);
  assert.equal(await timedOut, null);
  assert.equal(errors.length, 1);
  assert.ok(errors[0] instanceof DemManagerStatusTimeoutError);
  assert.equal(errors[0].code, "DEM_MANAGER_STATUS_TIMEOUT");
  assert.equal(errors[0].message, "DEM 任务状态读取超过 15 毫秒，已停止等待；可稍后重试。");
  assert.equal(coordinator.hasInFlight, false);

  const current = coordinator.poll();
  assert.ok(current);
  assert.equal(coordinator.hasInFlight, true);
  first.resolve(1);
  await Promise.resolve();
  await Promise.resolve();
  assert.deepEqual(values, []);
  assert.equal(coordinator.hasInFlight, true);

  second.resolve(2);
  assert.equal(await current, 2);
  assert.deepEqual(values, [2]);
  assert.equal(coordinator.hasInFlight, false);
});

test("concurrent command refreshes serialize and each receives a fresh snapshot", async () => {
  const first = deferred<number>();
  const second = deferred<number>();
  const reads = [first, second];
  let readCount = 0;
  const values: number[] = [];
  const coordinator = new DemManagerStatusCoordinator<number>({
    read: () => {
      readCount += 1;
      return reads.shift()!.promise;
    },
    onValue: (value) => values.push(value),
    onError: (error) => assert.fail(`unexpected read error: ${String(error)}`),
    timeoutMs: 100,
  });

  const firstRefresh = coordinator.refresh();
  const secondRefresh = coordinator.refresh();
  assert.equal(readCount, 1);

  first.resolve(1);
  assert.equal(await firstRefresh, 1);
  await waitFor(() => readCount === 2);
  assert.equal(coordinator.hasInFlight, true);

  second.resolve(2);
  assert.equal(await secondRefresh, 2);
  assert.deepEqual(values, [1, 2]);
  assert.equal(coordinator.hasInFlight, false);
});

test("poll skips instead of joining or duplicating an in-flight read", async () => {
  const pending = deferred<number>();
  let readCount = 0;
  const coordinator = new DemManagerStatusCoordinator<number>({
    read: () => {
      readCount += 1;
      return pending.promise;
    },
    onValue: () => undefined,
    onError: (error) => assert.fail(`unexpected read error: ${String(error)}`),
    timeoutMs: 100,
  });

  const firstPoll = coordinator.poll();
  assert.ok(firstPoll);
  assert.equal(coordinator.poll(), null);
  assert.equal(readCount, 1);
  pending.resolve(3);
  assert.equal(await firstPoll, 3);
});

test("dispose settles the waiter and prevents reads or late publication while unmounted", async () => {
  const pending = deferred<number>();
  const values: number[] = [];
  const errors: unknown[] = [];
  let readCount = 0;
  const coordinator = new DemManagerStatusCoordinator<number>({
    read: () => {
      readCount += 1;
      return pending.promise;
    },
    onValue: (value) => values.push(value),
    onError: (error) => errors.push(error),
    timeoutMs: 100,
  });

  const current = coordinator.refresh();
  assert.equal(readCount, 1);
  coordinator.dispose();
  assert.equal(await current, null);
  assert.equal(coordinator.hasInFlight, false);
  assert.equal(await coordinator.refresh(), null);
  assert.equal(coordinator.poll(), null);
  assert.equal(readCount, 1);

  pending.resolve(9);
  await Promise.resolve();
  await Promise.resolve();
  assert.deepEqual(values, []);
  assert.deepEqual(errors, []);
});

test("a refresh queued before unmount cannot start a read in the next lifecycle", async () => {
  const oldRead = deferred<number>();
  const newRead = deferred<number>();
  const reads = [oldRead, newRead];
  let readCount = 0;
  const values: number[] = [];
  const coordinator = new DemManagerStatusCoordinator<number>({
    read: () => {
      readCount += 1;
      return reads.shift()!.promise;
    },
    onValue: (value) => values.push(value),
    onError: (error) => assert.fail(`unexpected read error: ${String(error)}`),
    timeoutMs: 100,
  });

  const oldPoll = coordinator.poll();
  assert.ok(oldPoll);
  const oldRefresh = coordinator.refresh();
  coordinator.dispose();
  coordinator.activate();

  assert.equal(await oldPoll, null);
  assert.equal(await oldRefresh, null);
  assert.equal(readCount, 1);

  const currentRefresh = coordinator.refresh();
  assert.equal(readCount, 2);
  oldRead.resolve(1);
  await Promise.resolve();
  assert.equal(coordinator.hasInFlight, true);
  assert.deepEqual(values, []);
  newRead.resolve(2);
  assert.equal(await currentRefresh, 2);
  assert.deepEqual(values, [2]);
});
