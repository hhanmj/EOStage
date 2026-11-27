import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React, { useState } from "react";
import { act } from "@testing-library/react";
import { cleanup, fireEvent, render } from "../../test-utils/render-harness";
import { I18nProvider, LOCALE_STORAGE_KEY, useI18n } from "../../i18n/index.ts";
import {
  clearTestDesktopShellBridge,
  registerTestDesktopShellBridge,
  type DesktopDeleteTaskConflictFilesRequest,
  type DesktopShellBridge,
  type DesktopTaskConflictFileListResult,
  type DesktopTaskConflictFileDeleteResult,
} from "../../lib/desktopShellBridge.ts";
import type { TaskConflictFile } from "../../lib/generated/v3-contracts.ts";
import type { ApplicationTask } from "../../shell/api/types.ts";
import { TaskConflictCleanup } from "./TaskConflictCleanup.tsx";
import { DownloadTaskRow } from "./DownloadTaskRow.tsx";

const files: TaskConflictFile[] = Array.from({ length: 6 }, (_, index) => ({
  path: `nested/scene-${index}.zip.part.eostage-conflict-20261102`,
  size_bytes: 1_000_000 + index,
  modified_at_epoch_ms: 1000 + index,
  recorded: index % 2 === 0,
}));

function task(overrides: Partial<ApplicationTask> = {}): ApplicationTask {
  return {
    task_id: "b15-ui-task",
    revision: 1,
    category: "scene_download",
    name: "Conflict cleanup",
    status: "completed",
    done_bytes: 0,
    total_bytes: null,
    scene_items: [],
    ...overrides,
  };
}

function listing(totalCount = files.length): DesktopTaskConflictFileListResult {
  return {
    ok: true,
    list: {
      task_id: "b15-ui-task",
      files,
      total_count: totalCount,
      total_size_bytes: 6_000_015,
    },
  };
}

function installBridge({
  list = async () => listing(),
  remove = async (request: DesktopDeleteTaskConflictFilesRequest): Promise<DesktopTaskConflictFileDeleteResult> => ({
    ok: true,
    result: {
      task_id: request.task_id,
      outcomes: request.expected.map(({ path }) => ({ path, outcome: "deleted" })),
    },
  }),
}: {
  list?: () => Promise<DesktopTaskConflictFileListResult>;
  remove?: (request: DesktopDeleteTaskConflictFilesRequest) => Promise<DesktopTaskConflictFileDeleteResult>;
} = {}) {
  const listCalls: string[] = [];
  const deleteCalls: DesktopDeleteTaskConflictFilesRequest[] = [];
  registerTestDesktopShellBridge({
    kind: "browser_fixture",
    application: {
      listTaskConflictFiles: async (request: { task_id: string }) => {
        listCalls.push(request.task_id);
        return list();
      },
      deleteTaskConflictFiles: async (request: DesktopDeleteTaskConflictFilesRequest) => {
        deleteCalls.push(request);
        return remove(request);
      },
    },
  } as unknown as DesktopShellBridge);
  return { listCalls, deleteCalls };
}

function cleanupRow(overrides: Partial<ApplicationTask> = {}, expanded = true) {
  return (
    <I18nProvider>
      <TaskConflictCleanup task={task(overrides)} expanded={expanded} />
    </I18nProvider>
  );
}

async function settle() {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
}

test.afterEach(() => {
  cleanup();
  clearTestDesktopShellBridge();
  window.localStorage.removeItem(LOCALE_STORAGE_KEY);
});

test("does not query collapsed, active, or active-item tasks; empty listings stay silent", async () => {
  const bridge = installBridge({ list: async () => listing(0) });
  const view = render(cleanupRow({}, false));
  await settle();
  assert.equal(bridge.listCalls.length, 0);
  view.rerender(cleanupRow({ status: "running" }));
  await settle();
  assert.equal(bridge.listCalls.length, 0);
  view.rerender(cleanupRow({ status: "paused", scene_items: [{ scene_id: "a", status: "running", done_bytes: 0 }] }));
  await settle();
  assert.equal(bridge.listCalls.length, 0);
  view.rerender(cleanupRow());
  await settle();
  assert.deepEqual(bridge.listCalls, ["b15-ui-task"]);
  assert.equal(view.queryByRole("button"), null);
  assert.equal(view.queryByRole("alert"), null);
});

test("dialog lists five relative paths and total; confirmation sends every expectation and reports skips", async () => {
  let refreshed = false;
  let releaseDelete: (() => void) | undefined;
  const deletePending = new Promise<void>((resolve) => { releaseDelete = () => { refreshed = true; resolve(); }; });
  const bridge = installBridge({
    list: async () => refreshed ? {
      ok: true,
      list: { task_id: "b15-ui-task", files: [], total_count: 0, total_size_bytes: 0 },
    } : listing(),
    remove: async (request) => {
      await deletePending;
      return {
        ok: true,
        result: {
          task_id: request.task_id,
          outcomes: request.expected.map(({ path }, index) => ({
            path,
            outcome: index === 1 ? "skipped_missing" : index === 2 ? "skipped_changed" : "deleted",
          })),
        },
      };
    },
  });
  const view = render(cleanupRow());
  await settle();
  assert.ok(view.getByText("6 个旧临时文件 · 6 MB"));
  fireEvent.click(view.getByRole("button", { name: "清理" }));
  await settle();
  assert.ok(view.getByRole("dialog", { name: "永久删除旧临时文件？" }));
  for (const file of files.slice(0, 5)) assert.ok(view.getByText(file.path));
  assert.equal(view.queryByText(files[5].path), null);
  assert.ok(view.getByText("等 1 个"));
  assert.ok(view.getByText("共 6 MB"));
  assert.equal(document.activeElement, view.getByRole("button", { name: "取消" }));

  fireEvent.click(view.getByRole("button", { name: "删除", exact: true }));
  await act(async () => { await Promise.resolve(); });
  assert.equal(view.getAllByRole("button", { name: "删除中…" }).every((button) => button.hasAttribute("disabled")), true);
  await act(async () => { releaseDelete?.(); await Promise.resolve(); });
  await settle();
  assert.deepEqual(bridge.deleteCalls, [{
    task_id: "b15-ui-task",
    expected: files.map(({ path, size_bytes, modified_at_epoch_ms }) => ({ path, size_bytes, modified_at_epoch_ms })),
  }]);
  assert.equal(view.queryByRole("dialog"), null);
  assert.ok(view.getByText(/已删除 4 个/));
  assert.ok(view.getByText(/2 个已变化或不存在，未删除/));
  assert.equal(bridge.listCalls.length, 2, "a successful deletion re-queries the listing");
  assert.equal(view.queryByRole("button", { name: "清理" }), null);
});

test("Escape and backdrop cancel without deleting and restore trigger focus", async () => {
  const bridge = installBridge();
  const view = render(cleanupRow());
  await settle();
  fireEvent.click(view.getByRole("button", { name: "清理" }));
  await settle();
  fireEvent.keyDown(window, { key: "Escape" });
  await settle();
  assert.equal(view.queryByRole("dialog"), null);
  assert.equal(document.activeElement, view.getByRole("button", { name: "清理" }));
  fireEvent.click(view.getByRole("button", { name: "清理" }));
  await settle();
  fireEvent.click(view.getByRole("dialog").querySelector('[aria-hidden="true"]')!);
  assert.equal(view.queryByRole("dialog"), null);
  assert.equal(bridge.deleteCalls.length, 0);
});

test("delete errors remain inline and allow retry", async () => {
  const bridge = installBridge({
    remove: async () => ({ ok: false, error: "permission denied", code: "IO" }),
  });
  const view = render(cleanupRow());
  await settle();
  fireEvent.click(view.getByRole("button", { name: "清理" }));
  fireEvent.click(view.getByRole("button", { name: "删除", exact: true }));
  await settle();
  assert.ok(view.getByRole("dialog"));
  assert.equal(view.getByRole("alert").textContent, "旧临时文件删除失败: permission denied");
  assert.equal(bridge.deleteCalls.length, 1);
  fireEvent.click(view.getByRole("button", { name: "取消" }));
  assert.ok(view.getByRole("alert"), "the error stays visible after cancelling");
});

test("listing errors are inline instead of a toast", async () => {
  installBridge({ list: async () => ({ ok: false, error: "listing unavailable", code: "IO" }) });
  const view = render(cleanupRow());
  await settle();
  assert.equal(view.getByRole("alert").textContent, "旧临时文件加载失败: listing unavailable");
});

test("download row lazily reveals cleanup for a completed single-item task only", async () => {
  const bridge = installBridge();
  const renderRow = (status: ApplicationTask["status"]) => (
    <I18nProvider><DownloadTaskRow task={task({ status })} /></I18nProvider>
  );
  const view = render(renderRow("completed"));
  assert.equal(bridge.listCalls.length, 0);
  fireEvent.click(view.getByRole("button", { name: "展开分景进度" }));
  await settle();
  assert.deepEqual(bridge.listCalls, ["b15-ui-task"]);
  assert.ok(view.getByText("6 个旧临时文件 · 6 MB"));
  fireEvent.click(view.getByRole("button", { name: "清理" }));
  await settle();
  assert.ok(view.getByRole("dialog", { name: "永久删除旧临时文件？" }));

  cleanup();
  bridge.listCalls.length = 0;
  const activeView = render(renderRow("running"));
  assert.equal(activeView.queryByRole("button", { name: "清理" }), null);
  assert.equal(bridge.listCalls.length, 0);
});

test("English switches cleanup summary and dialog labels", async () => {
  installBridge();
  function EnglishCleanup() {
    const { setLocale } = useI18n();
    const [english, setEnglish] = useState(false);
    return (
      <>
        <button type="button" onClick={() => { setLocale("en"); setEnglish(true); }}>English</button>
        <TaskConflictCleanup task={task()} expanded={english} />
      </>
    );
  }
  const view = render(<I18nProvider><EnglishCleanup /></I18nProvider>);
  fireEvent.click(view.getByRole("button", { name: "English" }));
  await settle();
  assert.ok(view.getByText("6 old temporary files · 6 MB"));
  fireEvent.click(view.getByRole("button", { name: "Clean up" }));
  await settle();
  assert.ok(view.getByRole("dialog", { name: "Permanently delete old temporary files?" }));
  assert.ok(view.getByRole("button", { name: "Delete", exact: true }));
  assert.ok(view.getByRole("button", { name: "Cancel", exact: true }));
  assert.equal(document.documentElement.lang, "en");
});
