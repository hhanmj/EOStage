import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { waitFor } from "@testing-library/react";
import { render, cleanup, fireEvent } from "../../test-utils/render-harness";
import { TaskCard } from "./DesktopDownloadDrawer";
import type { ApplicationTask } from "../../shell/api/types";

test.afterEach(() => {
  cleanup();
});

function pausedTask(withReason: boolean): ApplicationTask {
  return {
    task_id: "task-disk-1",
    revision: 3,
    category: "scene_download",
    name: "Sentinel-1 SLC 单景下载",
    provider: "ASF",
    dataset: "Sentinel-1",
    status: "paused",
    done_bytes: 1_000_000_000,
    total_bytes: 4_000_000_000,
    output_dir: "D:/InSAR_Data/s1",
    updated_at: "2026-10-26T08:05:00Z",
    pause_reason: withReason
      ? { kind: "disk_low", free_bytes: 1288490188, threshold_bytes: 2147483648, drive: "D:" }
      : null,
  };
}

test("W5-2f R6 TaskCard 磁盘空间不足暂停", async (suite) => {
  await suite.test("1. 有 disk_low 原因：显示原因行且保留「继续」按钮", () => {
    const view = render(<TaskCard task={pausedTask(true)} onResume={() => {}} />);
    assert.ok(view.getByText("磁盘空间不足已暂停（D: 剩余 1.2 GB）"));
    assert.ok(view.getByRole("button", { name: "继续任务" }));
  });

  await suite.test("2. 无原因：不显示原因行", () => {
    const view = render(<TaskCard task={pausedTask(false)} onResume={() => {}} />);
    assert.strictEqual(view.queryByText(/磁盘空间不足已暂停/), null);
    assert.ok(view.getByRole("button", { name: "继续任务" }));
  });

  await suite.test("3. 点「继续」被后端以 DISK_LOW 拒绝：卡内出现中文解释，不出现错误码原文", async () => {
    let calls = 0;
    const view = render(
      <TaskCard
        task={pausedTask(true)}
        onResume={async () => {
          calls += 1;
          throw { code: "DISK_LOW", message_key: "disk_low", context: {} };
        }}
      />
    );
    fireEvent.click(view.getByRole("button", { name: "继续任务" }));
    await waitFor(() =>
      assert.ok(view.getByText("磁盘剩余空间不足，无法继续下载，请先清理该盘空间"))
    );
    assert.strictEqual(calls, 1);
    assert.strictEqual(view.queryByText(/DISK_LOW/), null);
  });

  await suite.test("4. 未知错误：显示固定中文句", async () => {
    const view = render(
      <TaskCard
        task={pausedTask(false)}
        onResume={async () => {
          throw new Error("boom");
        }}
      />
    );
    fireEvent.click(view.getByRole("button", { name: "继续任务" }));
    await waitFor(() => assert.ok(view.getByText("操作未能完成")));
    assert.strictEqual(view.queryByText(/boom/), null);
  });
});
