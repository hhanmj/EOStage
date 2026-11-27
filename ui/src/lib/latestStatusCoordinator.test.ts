import assert from "node:assert/strict";
import test from "node:test";

import { LatestStatusCoordinator } from "./latestStatusCoordinator.ts";
import {
  ApplicationSearchProgressSequenceGuard,
  type ApplicationSearchProgress,
} from "./sarSearchStatusPresentation.ts";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((nextResolve) => {
    resolve = nextResolve;
  });
  return { promise, resolve };
}

test("a final refresh waits for an older poll and cannot be overwritten by it", async () => {
  const running = deferred<string>();
  const finished = deferred<string>();
  const reads = [running, finished];
  const values: string[] = [];
  const coordinator = new LatestStatusCoordinator<string>({
    read: () => reads.shift()!.promise,
    publish: (value) => values.push(value),
  });

  const oldPoll = coordinator.poll();
  assert.ok(oldPoll);
  const finalRefresh = coordinator.refresh();

  running.resolve("running");
  assert.equal(await oldPoll, "running");
  finished.resolve("finished");
  assert.equal(await finalRefresh, "finished");
  assert.deepEqual(values, ["running", "finished"]);
});

test("polling never starts overlapping reads", async () => {
  const pending = deferred<string>();
  let reads = 0;
  const coordinator = new LatestStatusCoordinator<string>({
    read: () => {
      reads += 1;
      return pending.promise;
    },
    publish: () => undefined,
  });

  const first = coordinator.poll();
  assert.ok(first);
  assert.equal(coordinator.poll(), null);
  await Promise.resolve();
  assert.equal(reads, 1);
  pending.resolve("running");
  assert.equal(await first, "running");
});

test("a response from a previous search lifecycle cannot overwrite a reactivated search", async () => {
  const previous = deferred<string>();
  const current = deferred<string>();
  const reads = [previous, current];
  const values: string[] = [];
  const coordinator = new LatestStatusCoordinator<string>({
    read: () => reads.shift()!.promise,
    publish: (value) => values.push(value),
  });

  const oldPoll = coordinator.poll();
  coordinator.dispose();
  coordinator.activate();
  const newPoll = coordinator.poll();

  previous.resolve("old finished");
  assert.equal(await oldPoll, null);
  current.resolve("new running");
  assert.equal(await newPoll, "new running");
  assert.deepEqual(values, ["new running"]);
});

function applicationProgress(
  sequence: number,
  stage: ApplicationSearchProgress["stage"],
  terminal: boolean,
): ApplicationSearchProgress {
  return {
    request_id: "search-1",
    sequence,
    stage,
    elapsed_ms: sequence,
    message: `progress ${sequence}`,
    terminal,
  };
}

test("application progress discards duplicate regression and post-terminal updates", () => {
  const guard = new ApplicationSearchProgressSequenceGuard();
  assert.equal(guard.accept(applicationProgress(1, "validating", false)), true);
  assert.equal(guard.accept(applicationProgress(1, "planning", false)), false);
  assert.equal(guard.accept(applicationProgress(0, "planning", false)), false);
  assert.equal(guard.accept(applicationProgress(2, "completed", true)), true);
  assert.equal(guard.accept(applicationProgress(3, "fetching", false)), false);

  guard.reset();
  assert.equal(guard.accept(applicationProgress(1, "validating", false)), true);
});
