import assert from "node:assert/strict";
import test from "node:test";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { JSDOM } from "jsdom";
import { DownloadHistoryCard } from "../src/components/DownloadHistoryCard.tsx";

test("history card keeps terminal status and actions readable and isolated", async (context) => {
  const dom = new JSDOM("<div id='root'></div>");
  Object.assign(globalThis, {
    window: dom.window,
    document: dom.window.document,
    IS_REACT_ACT_ENVIRONMENT: true,
  });
  const calls: Array<[string, string | undefined, number | undefined]> = [];
  const row = {
    id: "history-A",
    task_id: "task-A",
    revision: 9,
    name: "Sentinel-1 下载任务",
    status: "completed",
    detail: "不应出现在卡片头部的日志",
  };
  const root = createRoot(dom.window.document.getElementById("root")!);
  context.after(async () => {
    await act(async () => { root.unmount(); });
    dom.window.close();
  });
  await act(async () => {
    root.render(
      <DownloadHistoryCard
        row={row}
        title="Sentinel-1 下载任务 · 08/19 11:50"
        statusLabel="已结束"
        statusPresentation={{ variant: "success" }}
        onOpen={(taskId, revision) => calls.push(["detail", taskId, revision])}
        onOpenOutput={(taskId, revision) => calls.push(["output", taskId, revision])}
        onRestore={(value) => calls.push(["restore", value.task_id, value.revision])}
        onDelete={(value) => calls.push(["delete", value.task_id, value.revision])}
      />,
    );
  });

  const card = dom.window.document.querySelector("[data-testid='download-history-card']");
  assert.ok(card, "React must commit the history card before assertions");
  const header = card.querySelector("[data-history-header]")!;
  const status = card.querySelector("[data-history-status]")!;
  const actions = card.querySelector("[data-history-actions]")!;
  assert.match(header.className, /grid-cols-\[minmax\(0,1fr\)_auto_auto\]/);
  assert.match(status.className, /whitespace-nowrap/);
  assert.match(status.className, /shrink-0/);
  assert.match(actions.className, /flex-wrap|grid/);
  assert.doesNotMatch(header.textContent ?? "", /详情|不应出现在卡片头部的日志/);

  const actionButtons = [...actions.querySelectorAll("button")] as HTMLButtonElement[];
  assert.equal(actionButtons.length, 3);
  assert.deepEqual(actionButtons.map((button) => button.textContent?.trim()), ["详情", "打开目录", "恢复"]);
  for (const button of actionButtons) {
    await act(async () => { button.click(); });
  }
  assert.deepEqual(calls, [
    ["detail", "task-A", 9],
    ["output", "task-A", 9],
    ["restore", "task-A", 9],
  ]);

  const deleteButton = card.querySelector("button[aria-label='删除历史记录']") as HTMLButtonElement;
  await act(async () => { deleteButton.click(); });
  assert.deepEqual(calls.at(-1), ["delete", "task-A", 9]);
  assert.equal(calls.length, 4, "each control fires only its own callback once");
});
