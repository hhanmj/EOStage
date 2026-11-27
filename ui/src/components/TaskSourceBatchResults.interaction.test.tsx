import assert from "node:assert/strict";
import test from "node:test";

import { JSDOM } from "jsdom";
import React from "react";

import type { StoreTaskSourceBatchPage } from "@/lib/generated/v3-contracts";

import { TaskSourceBatchResults } from "./TaskSourceBatchResults";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://127.0.0.1/",
});
Object.assign(globalThis, {
  window: dom.window,
  document: dom.window.document,
  HTMLElement: dom.window.HTMLElement,
  Node: dom.window.Node,
  MutationObserver: dom.window.MutationObserver,
  getComputedStyle: dom.window.getComputedStyle,
  IS_REACT_ACT_ENVIRONMENT: true,
});
Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  value: dom.window.navigator,
});

const { cleanup, render, screen } = await import("@testing-library/react");
const userEvent = (await import("@testing-library/user-event")).default;

test.afterEach(() => cleanup());

function sourcePage(status: StoreTaskSourceBatchPage["task_status"]): StoreTaskSourceBatchPage {
  return {
    task_id: "task-1",
    source_batch_id: "batch-frozen",
    task_revision: 7,
    task_status: status,
    page_index: 0,
    page_size: 24,
    total_count: 2,
    items: [
      {
        ordinal: 0,
        item: {
          item_id: "already-member",
          provider_id: "ASF",
          display_name: "ALREADY_MEMBER.zip",
          acquisition_datetime: "2020-01-01T00:00:00Z",
          footprint_geojson: null,
          metadata: {},
          payload: {},
        },
        belongs_to_task: true,
        task_status: "completed",
        checked: true,
        bytes_completed: 100,
        total_bytes: 100,
        bytes_per_second: 0,
        failure: null,
      },
      {
        ordinal: 1,
        item: {
          item_id: "candidate",
          provider_id: "ASF",
          display_name: "CANDIDATE.zip",
          acquisition_datetime: "2020-01-02T00:00:00Z",
          footprint_geojson: null,
          metadata: {},
          payload: {},
        },
        belongs_to_task: false,
        task_status: null,
        checked: false,
        bytes_completed: 0,
        total_bytes: null,
        bytes_per_second: 0,
        failure: null,
      },
    ],
  };
}

test("queued task can select only new historical products and append them", async () => {
  const submitted: string[][] = [];
  render(
    <TaskSourceBatchResults
      page={sourcePage("queued")}
      selectedIds={[]}
      filter=""
      sort="source_order"
      busy={false}
      mutationBusy={false}
      onToggleItem={() => undefined}
      onFilterChange={() => undefined}
      onSortChange={() => undefined}
      onPageIndexChange={() => undefined}
      onSubmit={(ids) => { submitted.push(ids); }}
    />,
  );

  assert.equal(screen.getByRole("checkbox", { name: "选择 ALREADY_MEMBER.zip" }).hasAttribute("disabled"), true);
  assert.ok(screen.getByRole("button", { name: "追加到当前任务" }).hasAttribute("disabled"));

  cleanup();
  render(
    <TaskSourceBatchResults
      page={sourcePage("queued")}
      selectedIds={["candidate"]}
      filter=""
      sort="source_order"
      busy={false}
      mutationBusy={false}
      onToggleItem={() => undefined}
      onFilterChange={() => undefined}
      onSortChange={() => undefined}
      onPageIndexChange={() => undefined}
      onSubmit={(ids) => { submitted.push(ids); }}
    />,
  );
  await userEvent.click(screen.getByRole("button", { name: "追加到当前任务" }));
  assert.deepEqual(submitted, [["candidate"]]);
});

test("terminal task selection creates a new task instead of reviving the old task", async () => {
  const submitted: string[][] = [];
  render(
    <TaskSourceBatchResults
      page={sourcePage("failed")}
      selectedIds={["candidate"]}
      filter=""
      sort="source_order"
      busy={false}
      mutationBusy={false}
      onToggleItem={() => undefined}
      onFilterChange={() => undefined}
      onSortChange={() => undefined}
      onPageIndexChange={() => undefined}
      onSubmit={(ids) => { submitted.push(ids); }}
    />,
  );

  await userEvent.click(screen.getByRole("button", { name: "创建新下载任务" }));
  assert.deepEqual(submitted, [["candidate"]]);
});

test("non-active non-terminal task cannot append or create a successor", () => {
  render(
    <TaskSourceBatchResults
      page={sourcePage("interrupted")}
      selectedIds={["candidate"]}
      filter=""
      sort="source_order"
      busy={false}
      mutationBusy={false}
      onToggleItem={() => undefined}
      onFilterChange={() => undefined}
      onSortChange={() => undefined}
      onPageIndexChange={() => undefined}
      onSubmit={() => assert.fail("interrupted task must not mutate membership")}
    />,
  );

  assert.equal(
    screen.getByRole("button", { name: "当前状态不可变更" }).hasAttribute("disabled"),
    true,
  );
});
