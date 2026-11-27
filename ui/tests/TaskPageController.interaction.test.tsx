import assert from "node:assert/strict";
import test from "node:test";
import React, { useEffect, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { JSDOM } from "jsdom";

import {
  DownloadTaskHistory,
  type DownloadTaskHistoryRow,
} from "../src/components/DownloadTaskHistory.tsx";
import { useTaskPageController } from "../src/lib/useTaskPageController.ts";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((nextResolve, nextReject) => {
    resolve = nextResolve;
    reject = nextReject;
  });
  return { promise, resolve, reject };
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

type PageValue = { taskId: string; revision: number; pageIndex: number };

function Harness({
  rows,
  read,
  publish,
}: {
  rows: readonly DownloadTaskHistoryRow[];
  read(taskId: string, revision: number, pageIndex: number): Promise<PageValue>;
  publish(value: PageValue): void;
}) {
  const controller = useTaskPageController();
  const [busy, setBusy] = useState<Record<string, string | undefined>>({});
  const [errors, setErrors] = useState<Record<string, string | undefined>>({});
  const [active, setActive] = useState<PageValue | null>(null);
  useEffect(() => {
    for (const row of rows) {
      if (row.task_id && row.revision !== undefined) controller.observeRevision(row.task_id, row.revision);
    }
  }, [controller, rows]);

  async function request(taskId: string, revision: number, pageIndex: number) {
    setBusy((value) => ({ ...value, [taskId]: "open" }));
    setErrors((value) => ({ ...value, [taskId]: undefined }));
    try {
      const value = await controller.request(taskId, revision, pageIndex, () => read(taskId, revision, pageIndex));
      if (!value) return;
      setActive(value);
      publish(value);
    } catch {
      setErrors((value) => ({ ...value, [taskId]: "typed page error" }));
    } finally {
      setBusy((value) => ({ ...value, [taskId]: undefined }));
    }
  }

  return (
    <div>
      <DownloadTaskHistory
        rows={rows}
        busyByTask={busy}
        errorByTask={errors}
        onOpen={(taskId, revision) => {
          if (revision !== undefined) void request(taskId, revision, 0);
        }}
        hasMore={active !== null}
        onNext={active ? () => void request(active.taskId, active.revision, active.pageIndex + 1) : undefined}
        onPrevious={active && active.pageIndex > 0 ? () => void request(active.taskId, active.revision, active.pageIndex - 1) : undefined}
      />
      <button data-close-task-page onClick={() => { controller.invalidate(); setActive(null); }}>close</button>
      <output>{active ? `${active.taskId}:${active.revision}:${active.pageIndex}` : "closed"}</output>
    </div>
  );
}

function row(taskId: string, revision: number): DownloadTaskHistoryRow {
  return { id: `${taskId}:${revision}`, task_id: taskId, revision, name: taskId, status: "completed", detail: "1/1" };
}

function rowButton(dom: JSDOM, taskId: string) {
  return dom.window.document.querySelector(`[data-task-id="${taskId}"] button`) as HTMLButtonElement;
}

async function render(root: Root, rows: readonly DownloadTaskHistoryRow[], read: Parameters<typeof Harness>[0]["read"], publish: (value: PageValue) => void) {
  root.render(<Harness rows={rows} read={read} publish={publish} />);
  await flush();
}

test("production task-page controller keeps exact task/revision/page and suppresses switch, revision, close, and unmount staleness", async () => {
  const dom = installDom();
  const root = createRoot(dom.window.document.getElementById("root")!);
  const calls: PageValue[] = [];
  const published: PageValue[] = [];
  const pending: Array<ReturnType<typeof deferred<PageValue>>> = [];
  const read = (taskId: string, revision: number, pageIndex: number) => {
    calls.push({ taskId, revision, pageIndex });
    const request = deferred<PageValue>();
    pending.push(request);
    return request.promise;
  };

  await render(root, [row("A", 4), row("B", 2)], read, (value) => published.push(value));
  rowButton(dom, "A").click();
  await flush();
  rowButton(dom, "B").click();
  await flush();
  assert.deepEqual(calls.slice(0, 2), [
    { taskId: "A", revision: 4, pageIndex: 0 },
    { taskId: "B", revision: 2, pageIndex: 0 },
  ]);
  pending[0].resolve(calls[0]);
  pending[1].resolve(calls[1]);
  await flush();
  assert.deepEqual(published, [{ taskId: "B", revision: 2, pageIndex: 0 }]);

  const next = [...dom.window.document.querySelectorAll("button")].find((button) => button.textContent === "下一页") as HTMLButtonElement;
  next.click();
  await flush();
  assert.deepEqual(calls.at(-1), { taskId: "B", revision: 2, pageIndex: 1 });
  await render(root, [row("B", 3)], read, (value) => published.push(value));
  pending[2].resolve(calls[2]);
  await flush();
  assert.equal(published.length, 1, "same-task revision advance invalidates the old page");

  rowButton(dom, "B").click();
  await flush();
  assert.deepEqual(calls.at(-1), { taskId: "B", revision: 3, pageIndex: 0 });
  pending[3].reject(new Error("typed failure"));
  await flush();
  assert.equal(dom.window.document.querySelector("[data-task-id='B'] [role=alert]")?.textContent, "typed page error");

  rowButton(dom, "B").click();
  await flush();
  (dom.window.document.querySelector("[data-close-task-page]") as HTMLButtonElement).click();
  pending[4].resolve(calls[4]);
  await flush();
  assert.equal(dom.window.document.querySelector("output")?.textContent, "closed");
  assert.equal(published.length, 1);

  rowButton(dom, "B").click();
  await flush();
  root.unmount();
  pending[5].resolve(calls[5]);
  await flush();
  assert.equal(published.length, 1, "unmount invalidates the final request");
});

test("production history controls expose cursor next and previous without fabricating an index", async () => {
  const dom = installDom();
  const root = createRoot(dom.window.document.getElementById("root")!);
  const actions: string[] = [];
  root.render(
    <DownloadTaskHistory
      rows={[row("terminal", 8)]}
      busyByTask={{ terminal: "open" }}
      errorByTask={{ terminal: "pending read" }}
      onOpen={() => undefined}
      hasMore
      onNext={() => actions.push("next")}
      onPrevious={() => actions.push("previous")}
    />,
  );
  await flush();
  const buttons = [...dom.window.document.querySelectorAll("button")];
  (buttons.find((button) => button.textContent === "上一页") as HTMLButtonElement).click();
  (buttons.find((button) => button.textContent === "下一页") as HTMLButtonElement).click();
  assert.deepEqual(actions, ["previous", "next"]);
  assert.equal(dom.window.document.querySelector("[data-task-id='terminal'] button")?.hasAttribute("disabled"), true);
  assert.equal(dom.window.document.querySelector("[role=alert]")?.textContent, "pending read");
  root.unmount();
});
