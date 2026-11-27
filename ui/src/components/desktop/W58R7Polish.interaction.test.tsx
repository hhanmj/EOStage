import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { waitFor } from "@testing-library/react";
import { render, cleanup, fireEvent } from "../../test-utils/render-harness";
import { SettingsModal } from "../SettingsModal";
import { DesktopDownloadDrawer } from "./DesktopDownloadDrawer";
import { DesktopTitlebar } from "./DesktopTitlebar";
import { ScenePopup } from "../WorkbenchMap";
import { routeMeta } from "../../shell/api/shellRoutes";
import type { ApplicationTask } from "../../shell/api/types";
import type { SceneRow } from "../../lib/bridge";

test.afterEach(() => {
  cleanup();
});

function historyTask(): ApplicationTask {
  return {
    task_id: "task-r7-history-1",
    revision: 2,
    category: "scene_download",
    name: "Sentinel-1 SLC 单景下载",
    provider: "ASF",
    dataset: "Sentinel-1",
    status: "completed",
    done_bytes: 0,
    total_bytes: null,
    updated_at: "2026-10-27T00:00:00Z",
    bytes_reported: false,
  } as ApplicationTask;
}

const popupScene: SceneRow = {
  scene_id: "G3210000001-ASF",
  product_key: "G3210000001-ASF",
  product_id: "G3210000001-ASF",
  provider: "asf.nisar",
  file_name: "NISAR_L1_PR_RSLC_001_077_A_012_2005_DHDH_A_20261020T010203_20261020T010233_P05000_N_F_J_001.h5",
  acquisition_datetime: "2026-10-20T01:02:03Z",
  orbit_direction: "ascending",
  relative_orbit: 77,
  frame: 12,
  file_size_remote: 4_400_000_000,
  has_url: false,
} as SceneRow;

test("W5-8 R7 设置/地图/历史细节", async (t) => {
  await t.test("偏好设置打开时按 Esc 只关闭设置，下层下载抽屉不响应", async () => {
    let settingsCloses = 0;
    let drawerCloses = 0;
    render(
      <>
        <DesktopDownloadDrawer
          open
          tasks={[]}
          onClose={() => {
            drawerCloses += 1;
          }}
        />
        <SettingsModal
          open
          initialCategory="storage"
          storagePanel={<div />}
          onClose={() => {
            settingsCloses += 1;
          }}
        />
      </>,
    );
    fireEvent.keyDown(document.body, { key: "Escape" });
    assert.equal(settingsCloses, 1, "Esc 必须关闭偏好设置");
    assert.equal(drawerCloses, 0, "偏好设置打开时，下层下载抽屉不得响应 Esc");
  });

  await t.test("没有偏好设置时，下载抽屉照常响应 Esc", () => {
    let drawerCloses = 0;
    render(
      <DesktopDownloadDrawer
        open
        tasks={[]}
        onClose={() => {
          drawerCloses += 1;
        }}
      />,
    );
    fireEvent.keyDown(document.body, { key: "Escape" });
    assert.equal(drawerCloses, 1);
  });

  await t.test("「清除历史」按钮文字完整、不换行、无省略号", async () => {
    const view = render(<DesktopDownloadDrawer open tasks={[historyTask()]} variant="page" />);
    const btn = await waitFor(() => view.getByRole("button", { name: "清除历史任务" }));
    assert.equal(btn.textContent?.trim(), "清除历史", `实际：${btn.textContent}`);
    assert.ok(btn.className.includes("whitespace-nowrap"), `实际 class：${btn.className}`);
    assert.ok(btn.className.includes("shrink-0"), `实际 class：${btn.className}`);
  });

  await t.test("新外壳足迹弹窗：轨道方向用 Ascending 格式、底部留白", () => {
    const view = render(
      <ScenePopup scene={popupScene} index={0} titleMode="display_name" hideDownloadEntry />,
    );
    assert.equal(view.queryByText("ascending") === null, true, "弹窗不得显示小写 ascending");
    assert.ok(view.getByText("Ascending"));
    const root = view.container.firstElementChild as HTMLElement;
    assert.ok(/\bpb-\d/.test(root.className), `弹窗根节点需要底部留白，实际 class：${root.className}`);
    assert.ok(view.getByText("大小"));
  });

  await t.test("空对话时标题栏中间标题为「对话」，右侧按钮保留「新对话」", () => {
    const view = render(
      <DesktopTitlebar
        title={routeMeta("agent").label}
        newChat={{ enabled: false, onClick: () => {} }}
      />,
    );
    assert.equal(view.getAllByText("新对话").length, 1, "「新对话」只应出现在右侧按钮上");
    assert.ok(view.getByText("对话"));

    cleanup();
    const defaultView = render(<DesktopTitlebar newChat={{ enabled: false, onClick: () => {} }} />);
    assert.equal(defaultView.getAllByText("新对话").length, 1);
  });
});
