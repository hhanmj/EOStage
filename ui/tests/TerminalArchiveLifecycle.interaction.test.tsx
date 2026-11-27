import assert from "node:assert/strict";
import test from "node:test";
import React, { StrictMode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { JSDOM } from "jsdom";

import { DownloadTaskHistory } from "../src/components/DownloadTaskHistory.tsx";
import { useDownloadStatusLifecycle } from "../src/lib/downloadTaskRuntime.ts";
import {
  TerminalTaskArchiveRuntime,
  useServerObservedArchiveKeys,
  useTerminalTaskArchiver,
  type TerminalArchiveResult,
  type TerminalArchiveTask,
  type TimeoutHost,
} from "../src/lib/terminalTaskArchiver.ts";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((nextResolve, nextReject) => {
    resolve = nextResolve;
    reject = nextReject;
  });
  return { promise, resolve, reject };
}

function fakeTimeouts(): TimeoutHost & { advance(ms: number): void; count(): number } {
  let now = 0;
  let nextId = 0;
  const timers = new Map<number, { at: number; callback: () => void }>();
  return {
    now: () => now,
    setTimeout(callback, delayMs) {
      const id = ++nextId;
      timers.set(id, { at: now + delayMs, callback });
      return id;
    },
    clearTimeout(id) {
      timers.delete(id);
    },
    advance(ms) {
      now += ms;
      let ready = [...timers.entries()].filter(([, timer]) => timer.at <= now);
      while (ready.length) {
        for (const [id, timer] of ready) {
          timers.delete(id);
          timer.callback();
        }
        ready = [...timers.entries()].filter(([, timer]) => timer.at <= now);
      }
    },
    count: () => timers.size,
  };
}

async function flush() {
  await Promise.resolve();
  await new Promise<void>((resolve) => setImmediate(resolve));
  await new Promise<void>((resolve) => setTimeout(resolve, 20));
}

function installDom() {
  const dom = new JSDOM("<div id='root'></div>");
  Object.assign(globalThis, {
    window: dom.window,
    document: dom.window.document,
    IS_REACT_ACT_ENVIRONMENT: false,
  });
  return dom;
}

type HarnessProps = {
  tasks: readonly TerminalArchiveTask[];
  observedKeys: readonly string[];
  archive(taskId: string, revision: number): Promise<TerminalArchiveResult>;
  onSuccess(taskId: string, revision: number): void;
  onError(taskId: string, revision: number, error: unknown): void;
  timers: TimeoutHost;
  lifecycle: { hydrate(): void; dispose(): void; setTaskStates(states: readonly string[]): void };
};

type WorkbenchArchiveEvidenceHarnessProps = {
  tasks: readonly TerminalArchiveTask[];
  displayRows: readonly TerminalArchiveTask[];
  onEvidence(owner: ReturnType<typeof useServerObservedArchiveKeys>): void;
  archive(taskId: string, revision: number): Promise<TerminalArchiveResult>;
  onSuccess(taskId: string, revision: number): void;
  onError(taskId: string, revision: number, error: unknown): void;
  timers: TimeoutHost;
};

function WorkbenchArchiveEvidenceHarness(props: WorkbenchArchiveEvidenceHarnessProps) {
  const evidence = useServerObservedArchiveKeys();
  props.onEvidence(evidence);
  useTerminalTaskArchiver({
    enabled: true,
    tasks: props.tasks,
    observedKeys: evidence.observedKeys,
    archive: props.archive,
    onSuccess: (taskId, revision) => {
      evidence.observeAck(taskId, revision);
      props.onSuccess(taskId, revision);
    },
    onError: props.onError,
    timers: props.timers,
    retryMs: 1_000,
  });
  return (
    <DownloadTaskHistory
      rows={props.displayRows.map((row) => ({
        id: `${row.task_id}:${row.revision}`,
        task_id: row.task_id,
        revision: row.revision,
        name: "Local terminal row",
        status: row.state,
        detail: "1/1",
      }))}
      busyByTask={{}}
      errorByTask={{}}
      onOpen={() => undefined}
    />
  );
}

function Harness(props: HarnessProps) {
  useDownloadStatusLifecycle(props.lifecycle, props.tasks.map((task) => task.state));
  useTerminalTaskArchiver({
    enabled: true,
    tasks: props.tasks,
    observedKeys: props.observedKeys,
    archive: props.archive,
    onSuccess: props.onSuccess,
    onError: props.onError,
    timers: props.timers,
    retryMs: 1_000,
  });
  return (
    <DownloadTaskHistory
      rows={props.tasks.map((task, index) => ({
        id: `${task.task_id}:${task.revision}`,
        task_id: task.task_id,
        revision: task.revision,
        name: `Task ${index}`,
        status: task.state,
        detail: "1/1",
      }))}
      busyByTask={{}}
      errorByTask={{}}
      onOpen={() => undefined}
    />
  );
}

async function render(root: Root, props: HarnessProps) {
  root.render(<StrictMode><Harness {...props} /></StrictMode>);
  await flush();
}

test("production history/archive lifecycle is revision-exact, bounded-retry, and StrictMode safe", async () => {
  const dom = installDom();
  const root = createRoot(dom.window.document.getElementById("root")!);
  const timers = fakeTimeouts();
  const lifecycleCalls: string[] = [];
  const lifecycle = {
    hydrate: () => { lifecycleCalls.push("hydrate"); },
    dispose: () => { lifecycleCalls.push("dispose"); },
    setTaskStates: (states: readonly string[]) => { lifecycleCalls.push(`states:${states.length}`); },
  };
  const archiveCalls: Array<[string, number]> = [];
  const successes: string[] = [];
  const errors: string[] = [];
  const requests: Array<ReturnType<typeof deferred<TerminalArchiveResult>>> = [];
  const archive = (taskId: string, revision: number) => {
    archiveCalls.push([taskId, revision]);
    const request = deferred<TerminalArchiveResult>();
    requests.push(request);
    return request.promise;
  };
  const known = Array.from({ length: 45 }, (_, index) => ({
    task_id: `known-${String(index).padStart(2, "0")}`,
    revision: 7,
    state: "completed",
  }));
  const base = {
    timers,
    lifecycle,
    archive,
    onSuccess: (taskId: string, revision: number) => successes.push(`${taskId}:${revision}`),
    onError: (taskId: string, revision: number) => errors.push(`${taskId}:${revision}`),
  };

  await render(root, {
    ...base,
    tasks: known,
    observedKeys: known.map((task) => `${task.task_id}:${task.revision}`),
  });
  assert.equal(dom.window.document.querySelectorAll("[data-task-id]").length, 45);
  assert.deepEqual(archiveCalls, [], ">40 already archived rows never dispatch archive writes");

  const pending = { task_id: "new-terminal", revision: 11, state: "failed" };
  await render(root, { ...base, tasks: [pending], observedKeys: [] });
  assert.deepEqual(archiveCalls, [["new-terminal", 11]]);
  await render(root, { ...base, tasks: [{ ...pending }], observedKeys: [] });
  assert.equal(archiveCalls.length, 1, "repeat publications and StrictMode replay coalesce");

  requests[0].resolve({ ok: false, code: "V3_STORE_BUSY", error: "retry" });
  await flush();
  assert.deepEqual(errors, ["new-terminal:11"]);
  assert.equal(timers.count(), 1);
  timers.advance(999);
  assert.equal(archiveCalls.length, 1);
  timers.advance(1);
  assert.deepEqual(archiveCalls, [["new-terminal", 11], ["new-terminal", 11]]);
  requests[1].reject(new Error("transient rejection"));
  await flush();
  assert.deepEqual(errors, ["new-terminal:11", "new-terminal:11"]);
  timers.advance(1_999);
  assert.equal(archiveCalls.length, 2);
  timers.advance(1);
  assert.equal(archiveCalls.length, 3);
  requests[2].resolve({ ok: true });
  await flush();
  assert.deepEqual(successes, ["new-terminal:11"]);

  await render(root, { ...base, tasks: [pending], observedKeys: ["new-terminal:11"] });
  assert.equal(archiveCalls.length, 3, "observed success prunes the attempt key");
  await render(root, {
    ...base,
    tasks: [{ ...pending, revision: 12 }],
    observedKeys: ["new-terminal:11"],
  });
  assert.deepEqual(archiveCalls.at(-1), ["new-terminal", 12], "newer revision is independently eligible");
  requests[3].resolve({ ok: true });
  await flush();

  root.unmount();
  await flush();
  assert.ok(lifecycleCalls.includes("hydrate"));
  assert.ok(lifecycleCalls.includes("dispose"));
});

test("failed NISAR subsets remain actionable across remount and retry until completed or cancelled", async () => {
  const dom = installDom();
  let root = createRoot(dom.window.document.getElementById("root")!);
  const calls: Array<[string, number]> = [];
  const base = {
    observedKeys: [],
    timers: fakeTimeouts(),
    lifecycle: { hydrate() {}, dispose() {}, setTaskStates() {} },
    archive: async (taskId: string, revision: number) => {
      calls.push([taskId, revision]);
      return { ok: true };
    },
    onSuccess() {},
    onError() { assert.fail("archive must not fail"); },
  };
  const subset = { task_id: "nisar-failed", task_type: "nisar_subset", revision: 25, state: "failed" };
  await render(root, { ...base, tasks: [subset] });
  assert.deepEqual(calls, [], "failure must not freeze a resumable checkpoint in history");
  root.unmount();
  await flush();
  root = createRoot(dom.window.document.getElementById("root")!);
  await render(root, { ...base, tasks: [subset] });
  assert.deepEqual(calls, [], "a new desktop lifecycle must preserve manual retry");
  for (const state of ["interrupted", "timeout", "queued", "running", "paused"]) {
    await render(root, { ...base, tasks: [{ ...subset, state }] });
  }
  assert.deepEqual(calls, []);
  await render(root, { ...base, tasks: [{ ...subset, revision: 26, state: "completed" }] });
  assert.deepEqual(calls, [["nisar-failed", 26]]);
  await render(root, { ...base, tasks: [{ ...subset, task_id: "nisar-cancelled", cancelled: true }] });
  assert.deepEqual(calls.at(-1), ["nisar-cancelled", 25]);
  await render(root, { ...base, tasks: [{ ...subset, task_id: "ordinary-failed", task_type: "sar" }] });
  assert.deepEqual(calls.at(-1), ["ordinary-failed", 25], "ordinary failed tasks retain automatic archival");
  root.unmount();
  await flush();
});

test("unmount suppresses stale archive failures and clears bounded retry timers", async () => {
  const dom = installDom();
  const root = createRoot(dom.window.document.getElementById("root")!);
  const timers = fakeTimeouts();
  const request = deferred<TerminalArchiveResult>();
  let errorCount = 0;
  await render(root, {
    tasks: [{ task_id: "late", revision: 3, state: "interrupted" }],
    observedKeys: [],
    archive: async () => request.promise,
    onSuccess: () => assert.fail("unmounted success must be suppressed"),
    onError: () => { errorCount += 1; },
    timers,
    lifecycle: { hydrate: () => undefined, dispose: () => undefined, setTaskStates: () => undefined },
  });
  root.unmount();
  request.reject(new Error("late"));
  await flush();
  timers.advance(10_000);
  assert.equal(errorCount, 0);
  assert.equal(timers.count(), 0);
});

test("permanent revision and already-exists archive outcomes reconcile once without retry loops", async () => {
  const timers = fakeTimeouts();
  const calls: string[] = [];
  const reconciles: string[] = [];
  const errors: string[] = [];
  const results: TerminalArchiveResult[] = [
    { ok: false, code: "V3_STORE_REVISION_CONFLICT", error: "stale" },
    { ok: false, code: "V3_STORE_ALREADY_EXISTS", error: "response lost" },
    { ok: true },
  ];
  const runtime = new TerminalTaskArchiveRuntime({
    archive: async (taskId, revision) => {
      calls.push(`${taskId}:${revision}`);
      return results.shift()!;
    },
    onSuccess: () => undefined,
    onError: (taskId, revision) => { errors.push(`${taskId}:${revision}`); },
    onReconcile: (taskId, revision) => { reconciles.push(`${taskId}:${revision}`); },
  }, timers, 1_000);
  runtime.activate();
  runtime.publish(true, [{ task_id: "permanent", revision: 4, state: "failed" }], []);
  await flush();
  timers.advance(60_000);
  await flush();
  assert.deepEqual(calls, ["permanent:4"]);
  assert.deepEqual(reconciles, ["permanent:4"]);
  assert.deepEqual(errors, ["permanent:4"]);
  assert.equal(timers.count(), 0);

  runtime.publish(true, [{ task_id: "lost-response", revision: 8, state: "completed" }], []);
  await flush();
  timers.advance(60_000);
  await flush();
  assert.deepEqual(calls, ["permanent:4", "lost-response:8"]);
  assert.deepEqual(reconciles, ["permanent:4", "lost-response:8"]);
  assert.deepEqual(errors, ["permanent:4"], "already-exists is reconciled as response loss, not surfaced as a new failure");
  assert.equal(timers.count(), 0);

  runtime.publish(true, [{ task_id: "permanent", revision: 5, state: "failed" }], []);
  await flush();
  assert.deepEqual(calls.at(-1), "permanent:5", "newer revision remains independently eligible");
  runtime.deactivate();
});

test("rejected and explicitly transient archive failures use capped bounded backoff", async () => {
  const timers = fakeTimeouts();
  const calls: number[] = [];
  const outcomes: Array<() => Promise<TerminalArchiveResult>> = [
    async () => { throw new Error("transport rejected"); },
    async () => ({ ok: false, code: "V3_STORE_BUSY", error: "busy" }),
    async () => { throw new Error("transport rejected again"); },
  ];
  const runtime = new TerminalTaskArchiveRuntime({
    archive: async () => {
      calls.push(timers.now());
      return outcomes.shift()!();
    },
    onSuccess: () => undefined,
    onError: () => undefined,
    onReconcile: () => undefined,
  }, timers, 1_000);
  runtime.activate();
  runtime.publish(true, [{ task_id: "transient", revision: 2, state: "failed" }], []);
  await flush();
  assert.deepEqual(calls, [0]);
  timers.advance(999);
  assert.deepEqual(calls, [0]);
  timers.advance(1);
  await flush();
  assert.deepEqual(calls, [0, 1_000]);
  timers.advance(1_999);
  assert.deepEqual(calls, [0, 1_000]);
  timers.advance(1);
  await flush();
  assert.deepEqual(calls, [0, 1_000, 3_000]);
  assert.equal(timers.count(), 0, "retry cap leaves no timer for the exhausted revision");
  timers.advance(60_000);
  await flush();
  assert.equal(calls.length, 3);
  runtime.publish(true, [{ task_id: "transient", revision: 2, state: "failed" }], []);
  timers.advance(60_000);
  await flush();
  assert.equal(calls.length, 3, "same exhausted publication remains suppressed");
  runtime.deactivate();
});

test("Workbench archive evidence never treats a local terminal display row as server persistence", async () => {
  const dom = installDom();
  const root = createRoot(dom.window.document.getElementById("root")!);
  const timers = fakeTimeouts();
  const task = { task_id: "local-terminal", revision: 13, state: "failed" };
  const requests: Array<ReturnType<typeof deferred<TerminalArchiveResult>>> = [];
  const calls: string[] = [];
  let evidenceOwner: ReturnType<typeof useServerObservedArchiveKeys> | null = null;
  const archive = (taskId: string, revision: number) => {
    calls.push(`${taskId}:${revision}`);
    const request = deferred<TerminalArchiveResult>();
    requests.push(request);
    return request.promise;
  };
  const renderEvidence = async (
    tasks: readonly TerminalArchiveTask[],
    displayRows: readonly TerminalArchiveTask[],
  ) => {
    root.render(
      <StrictMode>
        <WorkbenchArchiveEvidenceHarness
          tasks={tasks}
          displayRows={displayRows}
          onEvidence={(owner) => { evidenceOwner = owner; }}
          archive={archive}
          onSuccess={() => undefined}
          onError={() => undefined}
          timers={timers}
        />
      </StrictMode>,
    );
    await flush();
  };

  await renderEvidence([task], []);
  assert.deepEqual(calls, ["local-terminal:13"]);
  await renderEvidence([task], [task]);
  requests[0].reject(new Error("transport rejected after local row insertion"));
  await flush();
  assert.equal(timers.count(), 1, "local display state must not cancel the bounded retry");
  timers.advance(1_000);
  await flush();
  assert.equal(calls.length, 2);
  requests[1].resolve({ ok: true });
  await flush();

  await renderEvidence([task], [task]);
  assert.equal(calls.length, 2, "successful fixed ACK remains confirmed when the current page is absent");

  const observedLater = { task_id: "server-observed-later", revision: 4, state: "completed" };
  await renderEvidence([observedLater], []);
  assert.equal(calls.filter((key) => key === "server-observed-later:4").length, 1);
  evidenceOwner!.observeHistoryRows([observedLater]);
  await flush();
  requests[2].reject(new Error("late failure after bounded history observation"));
  await flush();
  assert.equal(timers.count(), 0, "server history observation prunes the matching in-flight retry");
  await renderEvidence([observedLater], []);
  assert.equal(
    calls.filter((key) => key === "server-observed-later:4").length,
    1,
    "a successful bounded history observation survives later page absence",
  );
  await renderEvidence([], []);
  evidenceOwner!.observeHistoryRows(Array.from({ length: 600 }, (_, index) => ({
    task_id: `bulk-${String(index).padStart(3, "0")}`,
    revision: 1,
  })));
  await flush();
  assert.equal(evidenceOwner!.observedKeys.length, 512);
  assert.equal(evidenceOwner!.observedKeys[0], "bulk-088:1");
  assert.equal(evidenceOwner!.observedKeys.at(-1), "bulk-599:1");
  root.unmount();
});
