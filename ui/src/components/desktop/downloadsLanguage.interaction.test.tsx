import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { cleanup, fireEvent, render } from "../../test-utils/render-harness";
import { I18nProvider, LOCALE_STORAGE_KEY, useI18n } from "../../i18n";
import type { ApplicationTask, TaskSceneItem } from "../../shell/api/types";
import { DownloadFilterBar } from "./DownloadFilterBar";
import { StatusBadge } from "./DesktopDownloadDrawer";
import { OrbitChildRow } from "./OrbitChildRow";

function LanguageControls() {
  const { setLocale } = useI18n();
  return <button type="button" onClick={() => setLocale("en")}>English</button>;
}

function JapaneseLanguageControls() {
  const { setLocale } = useI18n();
  return <button type="button" onClick={() => setLocale("ja")}>日本語</button>;
}

const tasks: ApplicationTask[] = [
  { task_id: "dem", revision: 1, category: "dem_convert", name: "DEM", status: "running", done_bytes: 0, total_bytes: null },
  { task_id: "other", revision: 1, category: "scene_download", name: "Other", status: "queued", done_bytes: 0, total_bytes: null },
];
const orbitTask: ApplicationTask = { ...tasks[1], auxiliary_kind: "AUX_POEORB" };
const orbitItem: TaskSceneItem = {
  item_id: "orbit-1",
  scene_id: "S1A_SCENE_001",
  status: "queued",
  done_bytes: 0,
  total_bytes: null,
  orbit_recheck: {
    orbit_type: "POEORB",
    state: "waiting",
    attempt: 1,
    next_check_at: "2026-11-22T00:00:00Z",
    deadline_at: "2026-12-25T00:00:00Z",
    expected_file_name: null,
  },
};

function DownloadLabels({ controls }: { controls: React.ReactNode }) {
  return (
    <I18nProvider>
      {controls}
      <DownloadFilterBar tasks={tasks} selectedType="all" onSelectType={() => {}} />
      <StatusBadge status="running" />
      <OrbitChildRow task={orbitTask} item={orbitItem} sourcePaused={false} revisionChanged={false} action={null} error={null} onAction={() => {}} />
    </I18nProvider>
  );
}

test.afterEach(() => {
  cleanup();
  window.localStorage.removeItem(LOCALE_STORAGE_KEY);
});

test("English switches task status, filters, and orbit child-row labels", () => {
  const view = render(<DownloadLabels controls={<LanguageControls />} />);
  assert.ok(view.getByText("下载中"));
  assert.ok(view.getByText("待补"));
  fireEvent.click(view.getByRole("button", { name: "English" }));
  assert.ok(view.getByText("Downloading"));
  assert.ok(view.getByRole("toolbar", { name: "Filter by data type" }));
  assert.ok(view.getByText("All"));
  assert.ok(view.getByText("DEM conversion"));
  assert.ok(view.getByText("Other"));
  assert.ok(view.getByText("Awaiting file"));
  assert.ok(view.getByRole("button", { name: "Check now" }));
  assert.equal(document.documentElement.lang, "en");
});

test("Japanese switches task status, filters, and orbit child-row labels", () => {
  const view = render(<DownloadLabels controls={<JapaneseLanguageControls />} />);
  fireEvent.click(view.getByRole("button", { name: "日本語" }));
  assert.ok(view.getByText("ダウンロード中"));
  assert.ok(view.getByRole("toolbar", { name: "データ種別で絞り込み" }));
  assert.ok(view.getByText("すべて"));
  assert.ok(view.getByText("DEM 変換"));
  assert.ok(view.getByText("その他"));
  assert.ok(view.getByText("公開待ち"));
  assert.ok(view.getByRole("button", { name: "今すぐチェック" }));
  assert.equal(document.documentElement.lang, "ja");
});
