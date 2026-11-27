import assert from "node:assert/strict";
import test from "node:test";
import React, { act, StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { JSDOM } from "jsdom";
import {
  publishCreatedDownloadStatus,
  useDownloadStatusLifecycle,
} from "./downloadTaskRuntime.ts";
import { ActiveOnlyStatusPolling } from "./downloadTaskPolling.ts";
import { LatestStatusCoordinator } from "./latestStatusCoordinator.ts";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((nextResolve) => { resolve = nextResolve; });
  return { promise, resolve };
}

test("created task status publishes from the mutation response without a racing restore", async () => {
  const published: string[] = [];
  let refreshes = 0;
  const usedMutation = await publishCreatedDownloadStatus(
    { download_status: "queued-task" },
    (status) => published.push(status),
    async () => { refreshes += 1; },
  );
  assert.equal(usedMutation, true);
  assert.deepEqual(published, ["queued-task"]);
  assert.equal(refreshes, 0);

  const usedFallback = await publishCreatedDownloadStatus(
    {},
    (status: string) => published.push(status),
    async () => { refreshes += 1; },
  );
  assert.equal(usedFallback, false);
  assert.equal(refreshes, 1);
});

test("production lifecycle hydrates once per StrictMode setup and stops after terminal state", async () => {
  const dom = new JSDOM("<div id='root'></div>");
  Object.assign(globalThis, { window: dom.window, document: dom.window.document });
  const calls: string[] = [];
  const lifecycle = { hydrate: () => { calls.push("hydrate"); }, dispose: () => { calls.push("dispose"); }, setTaskStates: (states: string[]) => calls.push(states.join(",")) };
  function Harness() { useDownloadStatusLifecycle(lifecycle, ["running"]); return null; }
  const root = createRoot(dom.window.document.getElementById("root")!);
  await act(async () => { root.render(React.createElement(StrictMode, null, React.createElement(Harness))); });
  await act(async () => { root.unmount(); });
  dom.window.close();
  assert.ok(calls.filter((value) => value === "hydrate").length >= 1);
  assert.ok(calls.includes("running"));
  assert.ok(calls.includes("dispose"));
});

test("production lifecycle keeps DEM manager polling active-only and stops when no mutable task remains", async () => {
  const dom = new JSDOM("<div id='root'></div>");
  Object.assign(globalThis, { window: dom.window, document: dom.window.document });
  let nextId = 0;
  const intervals = new Map<number, () => void>();
  const timers = {
    setInterval(callback: () => void) { const id = ++nextId; intervals.set(id, callback); return id; },
    clearInterval(id: number) { intervals.delete(id); },
  };
  let reads = 0;
  let active = true;
  const coordinator = {
    activate: () => { active = true; },
    dispose: () => { active = false; },
    poll: () => active ? Promise.resolve(++reads) : null,
    refresh: () => Promise.resolve(++reads),
  };
  let polling: ActiveOnlyStatusPolling | null = null;
  const lifecycle = {
    hydrate: () => {
      polling = new ActiveOnlyStatusPolling(coordinator, timers, 750);
      return polling.hydrate();
    },
    dispose: () => { polling?.dispose(); polling = null; },
    setTaskStates: (states: readonly string[]) => polling?.setTaskStates(states),
  };
  function Harness({ states }: { states: readonly string[] }) {
    useDownloadStatusLifecycle(lifecycle, states);
    return null;
  }
  const root = createRoot(dom.window.document.getElementById("root")!);
  await act(async () => { root.render(React.createElement(StrictMode, null, React.createElement(Harness, { states: [] }))); });
  assert.equal(intervals.size, 0, "startup hydrates once per setup but does not poll an empty DEM manager forever");

  await act(async () => { root.render(React.createElement(StrictMode, null, React.createElement(Harness, { states: ["running"] }))); });
  assert.equal(intervals.size, 1);
  for (const callback of intervals.values()) callback();
  await Promise.resolve();
  const readsWhileActive = reads;
  assert.ok(readsWhileActive >= 1);

  await act(async () => { root.render(React.createElement(StrictMode, null, React.createElement(Harness, { states: ["completed"] }))); });
  assert.equal(intervals.size, 0);
  await act(async () => { for (const callback of intervals.values()) callback(); });
  assert.equal(reads, readsWhileActive);
  await act(async () => { root.unmount(); });
  dom.window.close();
  assert.equal(intervals.size, 0);
});

test("StrictMode production lifecycle keeps the real latest-status coordinator single-flight and eventually publishes fresh data", async () => {
  const dom = new JSDOM("<div id='root'></div>");
  Object.assign(globalThis, { window: dom.window, document: dom.window.document });
  let nextTimer = 0;
  const intervals = new Map<number, () => void>();
  const timers = {
    setInterval(callback: () => void) { const id = ++nextTimer; intervals.set(id, callback); return id; },
    clearInterval(id: number) { intervals.delete(id); },
  };
  const requests = [deferred<string>(), deferred<string>(), deferred<string>()];
  let readIndex = 0;
  let activeReads = 0;
  let maxConcurrency = 0;
  const published: string[] = [];
  const coordinator = new LatestStatusCoordinator<string>({
    read: () => {
      const request = requests[readIndex++];
      activeReads += 1;
      maxConcurrency = Math.max(maxConcurrency, activeReads);
      return request.promise.finally(() => { activeReads -= 1; });
    },
    publish: (value) => published.push(value),
  });
  let polling: ActiveOnlyStatusPolling | null = null;
  const lifecycle = {
    hydrate: () => {
      polling = new ActiveOnlyStatusPolling(coordinator, timers, 900);
      return polling.hydrate();
    },
    dispose: () => { polling?.dispose(); polling = null; },
    setTaskStates: (states: readonly string[]) => polling?.setTaskStates(states),
  };
  function Harness({ state }: { state: string }) {
    useDownloadStatusLifecycle(lifecycle, [state]);
    return null;
  }
  const root = createRoot(dom.window.document.getElementById("root")!);
  await act(async () => { root.render(React.createElement(StrictMode, null, React.createElement(Harness, { state: "running" }))); });
  assert.equal(maxConcurrency, 1, "StrictMode replay must not start a second transport read");
  assert.equal(readIndex, 1);

  await act(async () => { requests[0].resolve("stale-first-lifecycle"); });
  assert.deepEqual(published, [], "disposed generation is suppressed");
  assert.equal(readIndex, 2, "replayed lifecycle eventually performs a fresh read");
  assert.equal(maxConcurrency, 1);
  await act(async () => { requests[1].resolve("fresh"); });
  assert.deepEqual(published, ["fresh"]);
  assert.equal(intervals.size, 1);

  for (const callback of intervals.values()) callback();
  await Promise.resolve();
  assert.equal(readIndex, 3);
  await act(async () => { requests[2].resolve("tick"); });
  assert.deepEqual(published, ["fresh", "tick"]);
  await act(async () => { root.render(React.createElement(StrictMode, null, React.createElement(Harness, { state: "completed" }))); });
  assert.equal(intervals.size, 0);
  await act(async () => { root.unmount(); });
  dom.window.close();
  assert.equal(intervals.size, 0);
});
