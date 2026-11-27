import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { render, cleanup } from "../../test-utils/render-harness";
import { TaskCard } from "./DesktopDownloadDrawer";
import { ScenePopup } from "../WorkbenchMap";
import type { ApplicationTask } from "../../shell/api/types";
import type { SceneRow } from "../../lib/bridge";

test.afterEach(() => {
  cleanup();
});

function archivedTask(status: ApplicationTask["status"]): ApplicationTask {
  return {
    task_id: "task-h-1",
    revision: 4,
    category: "scene_download",
    name: "Sentinel-1 SLC 单景下载",
    provider: "ASF",
    dataset: "Sentinel-1",
    status,
    done_bytes: 0,
    total_bytes: null,
    source_total_bytes: null,
    updated_at: "2026-10-26T23:00:00Z",
    bytes_reported: false,
  } as ApplicationTask;
}

test("W5-8 R6 条目 11：历史区 TaskCard 不显示「0 B」", async (t) => {
  for (const status of ["completed", "cancelled"] as const) {
    await t.test(`${status}：显示「大小未知」`, () => {
      const view = render(<TaskCard task={archivedTask(status)} />);
      // 断言布尔值：直接比较 DOM 节点时 assert 的差异输出会深度展开 jsdom 对象
      assert.equal(
        view.queryByText(/(^|\s)0 B/) === null,
        true,
        `归档 ${status} 任务卡不得出现「0 B」（后端未上报字节），实际：${view.container.textContent}`,
      );
      assert.ok(view.getByText("大小未知"));
    });
  }
});

const nisarScene: SceneRow = {
  scene_id: "G1234567890-ASF",
  product_key: "G1234567890-ASF",
  product_id: "G1234567890-ASF",
  provider: "asf.nisar",
  file_name: "NISAR_L1_PR_RSLC_001_077_A_012_2005_DHDH_A_20261020T010203_20261020T010233_P05000_N_F_J_001.h5",
  acquisition_datetime: "2026-10-20T01:02:03Z",
  orbit_direction: "ascending",
  relative_orbit: 77,
  frame: 12,
  has_url: false,
} as SceneRow;

test("W5-8 R6 条目 12：足迹弹窗标题与下载入口", async (t) => {
  await t.test("display_name 模式显示文件名、不显示裸 provider ID 与「下载入口」", () => {
    const view = render(
      <ScenePopup scene={nisarScene} index={0} titleMode="display_name" hideDownloadEntry />,
    );
    assert.ok(view.getByText(nisarScene.file_name!), "弹窗标题必须显示文件名");
    assert.equal(view.queryByText("G1234567890-ASF") === null, true, "弹窗不得显示裸 provider ID");
    assert.equal(view.queryByText("下载入口") === null, true, "新外壳弹窗不得显示「下载入口」行");
  });

  await t.test("display_name 模式下文件名为空 →「未命名」，不回退裸 provider ID", () => {
    const view = render(
      <ScenePopup scene={{ ...nisarScene, file_name: "  " }} index={0} titleMode="display_name" hideDownloadEntry />,
    );
    assert.ok(view.getByText("未命名"));
    assert.equal(view.queryByText("G1234567890-ASF") === null, true, "弹窗不得显示裸 provider ID");
  });

  await t.test("默认模式与原来一致：标题 scene_id，有「下载入口」", () => {
    const view = render(<ScenePopup scene={nisarScene} index={0} />);
    assert.ok(view.getByText("G1234567890-ASF"));
    assert.ok(view.getByText("下载入口"));
    assert.equal(view.queryByText(nisarScene.file_name!) === null, true);
  });
});
