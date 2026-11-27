import assert from "node:assert/strict";
import test from "node:test";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { JSDOM } from "jsdom";

import { clearTaskDetailTestBridge, registerTaskDetailTestBridge } from "../src/lib/taskDetailTestBridge.ts";

test("floating task workbench owns a dense live overview, close, output, files, and useful event history", async (t) => {
  const dom = new JSDOM("<div id='root'></div>", { url: "http://127.0.0.1/" });
  Object.assign(globalThis, {
    window: dom.window,
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    Node: dom.window.Node,
    MutationObserver: dom.window.MutationObserver,
    getComputedStyle: dom.window.getComputedStyle,
    IS_REACT_ACT_ENVIRONMENT: true,
  });
  Object.defineProperty(globalThis, "navigator", { configurable: true, value: dom.window.navigator });
  let root: ReturnType<typeof createRoot> | undefined;
  t.after(async () => {
    if (root) await act(async () => root!.unmount());
    clearTaskDetailTestBridge();
    dom.window.close();
  });
  // Leaflet is loaded by the production component's dependency graph; install DOM first.
  const { TaskDetailWorkbench } = await import("../src/components/TaskDetailWorkbench.tsx");
  registerTaskDetailTestBridge({
    invoke: async (command) => {
      assert.equal(command, "application_load_task_source_batch_page");
      return {
        task_id: "task-A", source_batch_id: "batch-A", task_revision: 7,
        task_status: "running", page_index: 0, page_size: 24, total_count: 0, items: [],
      };
    },
    getCurrentWindow: () => ({
      minimize: async () => undefined, toggleMaximize: async () => undefined,
      close: async () => undefined, innerSize: async () => ({ width: 100, height: 100 }),
      startResizeDragging: async () => undefined,
    }),
    open: async () => null,
  });
  let closes = 0;
  let opens = 0;
  root = createRoot(dom.window.document.getElementById("root")!);
  const renderTask = async ({
    status = "running",
    transferredBytes = 1_024,
    bytesPerSecond = 512,
    transferActiveMillis = 3_210,
    itemStatus = "active",
  }: {
    status?: string;
    transferredBytes?: number;
    bytesPerSecond?: number;
    transferActiveMillis?: number;
    itemStatus?: string;
  } = {}) => {
    await act(async () => root!.render(<TaskDetailWorkbench
      taskId="task-A"
      revision={7}
      title="Sentinel-1 下载任务"
      status={status}
      datasetLabel="Sentinel-1 SLC"
      concurrency={1}
      resumeSupported
      outputDir="E:\\downloads\\task-A"
      completedCount={1}
      activeCount={itemStatus === "active" ? 1 : 0}
      waitingCount={2}
      pausedCount={itemStatus === "paused" ? 1 : 0}
      failedCount={0}
      transferredBytes={transferredBytes}
      totalBytes={4_096}
      bytesPerSecond={bytesPerSecond}
      transferActiveMillis={transferActiveMillis}
      page={{
        taskId: "task-A", revision: 7, pageIndex: 0, pageSize: 40, totalCount: 5,
        items: [
          {
            itemId: "ITEM-A", displayName: "ITEM-A.zip", status: itemStatus,
            bytesCompleted: transferredBytes, totalBytes: 4_096,
            percentage: Math.round((transferredBytes / 4_096) * 100),
            bytesPerSecond, destination: "E:\\downloads\\task-A\\ITEM-A.zip",
            failure: null, retryable: false, terminal: false,
          },
          {
            itemId: "ITEM-B", displayName: "ITEM-B.zip", status: "queued",
            bytesCompleted: 0, totalBytes: 8_192, percentage: 0, bytesPerSecond: 0,
            destination: "E:\\downloads\\task-A\\ITEM-B.zip",
            failure: null, retryable: false, terminal: false,
          },
          {
            itemId: "ITEM-C", displayName: "ITEM-C.zip", status: "paused",
            bytesCompleted: 512, totalBytes: 2_048, percentage: 25, bytesPerSecond: 0,
            destination: "E:\\downloads\\task-A\\ITEM-C.zip",
            failure: null, retryable: false, terminal: false,
          },
          {
            itemId: "ITEM-D", displayName: "ITEM-D.zip", status: "failed",
            bytesCompleted: 256, totalBytes: 1_024, percentage: 25, bytesPerSecond: 0,
            destination: "E:\\downloads\\task-A\\ITEM-D.zip",
            failure: { code: "HTTP_TIMEOUT", reasonZh: "请求超时", retryable: true },
            retryable: true, terminal: true,
          },
          {
            itemId: "ITEM-E", displayName: "ITEM-E.zip", status: "completed",
            bytesCompleted: 1_024, totalBytes: 1_024, percentage: 100, bytesPerSecond: 0,
            destination: "E:\\downloads\\task-A\\ITEM-E.zip",
            failure: null, retryable: false, terminal: true,
          },
        ],
      }}
      events={[
        {
          sequence: 10, occurred_at: "2026-08-19T00:00:10Z", level: "info",
          event_kind: "started", item_label: "ITEM-A", code: null, message_zh: "任务已创建",
        },
        {
          sequence: 9, occurred_at: "2026-08-19T00:00:09Z", level: "error",
          event_kind: "failed", item_label: "ITEM-A", code: "HTTP_TIMEOUT", message_zh: "下载失败",
        },
      ]}
      onClose={() => { closes += 1; }}
      onOpenOutput={() => { opens += 1; }}
      onPageIndexChange={() => undefined}
    />));
  };
  await renderTask();

  const dialog = dom.window.document.querySelector("[role=dialog]");
  assert.ok(dialog);
  const tabs = [...dialog.querySelectorAll("nav[aria-label='任务详情分区'] button")] as HTMLButtonElement[];
  // Owner acceptance restores the dense overview as the default first screen.
  assert.deepEqual(tabs.map((button) => button.textContent), ["概览", "下载文件", "来源结果", "事件"]);
  assert.ok(dialog.querySelector("[data-task-overview-summary]"));
  assert.match(dialog.textContent ?? "", /真实传输(?:用时)?3\.2 秒/);
  assert.match(dialog.textContent ?? "", /25%/);
  // Shared decimal SI presentation was recorded in the 2026-09-05 handoff P0 work log.
  // Raw counters and percentage/ETA expectations remain unchanged.
  assert.match(dialog.textContent ?? "", /1 KB \/ 4\.1 KB/);
  assert.match(dialog.textContent ?? "", /ITEM-D\.zip/);
  assert.match(dialog.textContent ?? "", /HTTP_TIMEOUT/);
  assert.match(dialog.textContent ?? "", /预计剩余/);
  assert.match(dialog.textContent ?? "", /并发\s*1/);
  assert.match(dialog.textContent ?? "", /Sentinel-1 SLC/);
  assert.doesNotMatch(dialog.textContent ?? "", /说明/);
  // Owner decision 2026-09-08 supersedes both three-item and all-item overviews.
  assert.equal(dialog.querySelector("[data-task-overview-current-items]"), null);
  assert.equal(dialog.querySelector("[data-task-page-detail]"), null);

  await renderTask({
    transferredBytes: 2_048,
    bytesPerSecond: 1_024,
    transferActiveMillis: 4_321,
  });
  assert.match(dialog.textContent ?? "", /50%/);
  assert.match(dialog.textContent ?? "", /2 KB \/ 4\.1 KB/);
  assert.match(dialog.textContent ?? "", /真实传输(?:用时)?4\.3 秒/);
  assert.match(dialog.querySelector("[data-task-overview-eta]")?.textContent ?? "", /2\.0 秒/);

  await renderTask({
    status: "paused",
    transferredBytes: 2_048,
    bytesPerSecond: 0,
    transferActiveMillis: 4_321,
    itemStatus: "paused",
  });
  assert.match(dialog.querySelector("[data-task-overview-eta]")?.textContent ?? "", /不可用/);
  assert.doesNotMatch(dialog.querySelector("[data-task-overview-eta]")?.textContent ?? "", /2\.0 秒/);

  const output = [...dom.window.document.querySelectorAll("button")]
    .find((button) => button.textContent?.includes("打开输出目录")) as HTMLButtonElement;
  output.click();
  assert.equal(opens, 1);

  const files = [...dom.window.document.querySelectorAll("button")]
    .find((button) => button.textContent === "下载文件") as HTMLButtonElement;
  assert.ok(files);
  await act(async () => files.click());
  assert.match(dom.window.document.querySelector("[role=dialog]")?.textContent ?? "", /ITEM-A\.zip/);
  const details = dialog.querySelector("[data-task-page-detail]");
  assert.ok(details);
  for (const id of ["A", "B", "C", "D", "E"]) assert.ok(details.textContent?.includes(`ITEM-${id}.zip`));
  assert.match(details.textContent ?? "", /50%/);
  assert.match(details.textContent ?? "", /HTTP_TIMEOUT/);
  assert.match(details.textContent ?? "", /100%/);

  const events = [...dom.window.document.querySelectorAll("button")]
    .find((button) => button.textContent === "事件") as HTMLButtonElement;
  assert.ok(events);
  await act(async () => events.click());
  assert.match(dom.window.document.querySelector("[role=dialog]")?.textContent ?? "", /HTTP_TIMEOUT/);
  assert.match(dom.window.document.querySelector("[role=dialog]")?.textContent ?? "", /下载失败/);
  assert.match(dom.window.document.querySelector("[role=dialog]")?.textContent ?? "", /任务已创建/);
  assert.doesNotMatch(
    dom.window.document.querySelector("[role=dialog]")?.textContent ?? "",
    /(?:info|started|failed|updated)/,
  );

  (dialog.parentElement as HTMLElement).click();
  assert.equal(closes, 0, "backdrop never closes or mutates the task");
  dom.window.document.dispatchEvent(new dom.window.KeyboardEvent("keydown", { key: "Escape" }));
  assert.equal(closes, 1);
});
