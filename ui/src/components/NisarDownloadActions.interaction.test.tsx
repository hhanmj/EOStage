import assert from "node:assert/strict";
import test from "node:test";

import React from "react";

import { cleanup, fireEvent, render } from "../test-utils/render-harness";
import { NisarDownloadActions } from "./NisarDownloadActions";
import { DownloadHistoryCard } from "./DownloadHistoryCard";

test("NISAR results expose complete and AOI subset actions without row activation", () => {
  let completeDownloads = 0;
  let opened = 0;
  const view = render(
    <NisarDownloadActions
      completeEnabled
      subsetState="ready"
      onCompleteDownload={() => { completeDownloads += 1; }}
      onOpenSubsetWorkspace={() => { opened += 1; }}
    />,
  );

  fireEvent.click(view.getByRole("button", { name: "完整产品" }));
  fireEvent.click(view.getByRole("button", { name: "AOI 裁剪" }));

  assert.equal(completeDownloads, 1);
  assert.equal(opened, 1);
  cleanup();
});

test("subset entry stays openable and explains a blocked submission", () => {
  let opened = 0;
  const view = render(
    <NisarDownloadActions
      completeEnabled={false}
      subsetState="blocked"
      subsetReason="需要配置 Earthdata 账号"
      onCompleteDownload={() => undefined}
      onOpenSubsetWorkspace={() => { opened += 1; }}
    />,
  );

  assert.equal((view.getByRole("button", { name: "完整产品" }) as HTMLButtonElement).disabled, true);
  fireEvent.click(view.getByRole("button", { name: "AOI 裁剪" }));
  assert.equal(opened, 1);
  assert.ok(view.getByText("需要配置 Earthdata 账号"));
  cleanup();
});

test("busy complete download is disabled with a readable reason while AOI stays openable", () => {
  let opened = 0;
  const view = render(
    <NisarDownloadActions
      completeEnabled
      completeBusy
      completeDisabledReason="正在准备完整产品"
      subsetState="ready"
      onCompleteDownload={() => undefined}
      onOpenSubsetWorkspace={() => { opened += 1; }}
    />,
  );

  const complete = view.getByRole("button", { name: /正在下载完整产品/ }) as HTMLButtonElement;
  assert.equal(complete.disabled, true);
  assert.ok(view.getByText("正在准备完整产品"));
  fireEvent.click(view.getByRole("button", { name: "AOI 裁剪" }));
  assert.equal(opened, 1);
  cleanup();
});

test("complete download disabled reason is shown when complete download is not enabled", () => {
  const view = render(
    <NisarDownloadActions
      completeEnabled={false}
      completeDisabledReason="完整产品尚未准备好"
      subsetState="ready"
      onCompleteDownload={() => undefined}
      onOpenSubsetWorkspace={() => undefined}
    />
  );
  assert.equal((view.getByRole("button", { name: "完整产品" }) as HTMLButtonElement).disabled, true);
  assert.ok(view.getByText("完整产品尚未准备好"));
  cleanup();
});

test("download history card shows task type, authoritative concurrency, and full lifecycle timestamps", () => {
  const view = render(
    <DownloadHistoryCard
      row={{
        id: "history-nisar-subset",
        task_id: "task-nisar-subset",
        revision: 4,
        name: "NISAR RSLC AOI 裁剪下载任务",
        status: "finished",
        detail: "1/1",
        task_type: "NISAR RSLC · AOI 裁剪",
        concurrency: 2,
        created_at: "2026-09-04T07:42:18.000Z",
        ended_at: "2026-09-04T07:45:21.000Z",
      }}
      title="NISAR RSLC AOI 裁剪下载任务"
      statusLabel="已完成"
    />,
  );
  const card = view.getByTestId("download-history-card");
  const facts = card.querySelector("[data-history-facts]");
  const metadata = card.querySelector("[data-history-metadata]");
  assert.match(facts?.textContent ?? "", /任务类型：NISAR RSLC · AOI 裁剪/);
  assert.match(facts?.textContent ?? "", /并发：2/);
  assert.doesNotMatch(facts?.textContent ?? "", /创建：|结束：/);
  assert.match(metadata?.textContent ?? "", /创建：\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}/);
  assert.match(metadata?.textContent ?? "", /结束：\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}/);
  assert.doesNotMatch(metadata?.textContent ?? "", /任务类型：|并发：/);
  cleanup();
});

test("Bhoonidhi S-RSLC keeps complete download and AOI subset both available", () => {
  let opened = 0;
  const view = render(
    <NisarDownloadActions
      completeEnabled
      subsetState="ready"
      bandLabel="S-band"
      sourceLabel="Bhoonidhi"
      subsetReason="可进入裁剪工作区"
      onCompleteDownload={() => undefined}
      onOpenSubsetWorkspace={() => { opened += 1; }}
    />,
  );

  assert.equal((view.getByRole("button", { name: "完整产品" }) as HTMLButtonElement).disabled, false);
  assert.equal((view.getByRole("button", { name: "AOI 裁剪" }) as HTMLButtonElement).disabled, false);
  assert.match(view.getByText(/S-band · Bhoonidhi/).textContent ?? "", /S-band · Bhoonidhi/);
  fireEvent.click(view.getByRole("button", { name: "AOI 裁剪" }));
  assert.equal(opened, 1);
  assert.ok(view.getByText(/可进入裁剪工作区/));
  cleanup();
});
