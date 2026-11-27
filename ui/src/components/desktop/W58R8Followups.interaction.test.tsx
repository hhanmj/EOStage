import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { render, cleanup, fireEvent } from "../../test-utils/render-harness";
import { AgentConfirmCard } from "./AgentConfirmCard";
import { DesktopDownloadDrawer } from "./DesktopDownloadDrawer";
import { SettingsModal } from "../SettingsModal";
import { ScenePopup } from "../WorkbenchMap";
import { confirmCardSourceLabel } from "../../shell/api/searchCardSource";
import type {
  StreamConfirmCardItem,
  StreamItem,
  StreamPlanCardItem,
  StreamProgressCardItem,
} from "../../shell/api/agentStreamLogic";
import type { SceneRow } from "../../lib/bridge";

test.afterEach(() => {
  cleanup();
});

const RUN_ID = "run-r8-1";

const confirmItem: StreamConfirmCardItem = {
  kind: "confirm_card",
  seq: 50,
  run_id: RUN_ID,
  plan_id: "plan-r8-1",
  plan_hash: "hash-r8-1",
  snapshot: {
    source: "asf.catalog",
    range: {
      aoi_name: "地图绘制矩形",
      area_km2: 125.6,
      product: "1 个产品",
      time_range: { start: "2026-07-26T00:00:00Z", end: "2026-10-26T23:59:59Z" },
    },
    estimated_bytes: null,
    output_dir: "D:\\ScenePort\\downloads",
    output_dir_source: "default",
    already_queued_count: 0,
  },
  approval: { decision: "manual", mode: "confirm_changes", card_reasons: [] },
  auto_executed: false,
};

const planWithDisclosure: StreamPlanCardItem = {
  kind: "plan_card",
  seq: 7,
  run_id: RUN_ID,
  plan_id: "plan-r8-0",
  plan_hash: "hash-r8-0",
  max_tier: 1,
  steps: [
    {
      tool_id: "search_start",
      label_zh: "检索数据",
      tier: 1,
      status: "done",
      disclosure: {
        resource_id: "science.sentinel-1.standard.v1",
        aoi_name: "地图绘制矩形",
        display_name_zh: "Sentinel-1",
        time_range: { start: "2026-07-26T00:00:00Z", end: "2026-10-26T23:59:59Z" },
      } as unknown as StreamPlanCardItem["steps"][number]["disclosure"],
    },
  ],
};

function sourceRowText(view: ReturnType<typeof render>): string {
  const dt = view.getByText("来源");
  return dt.nextElementSibling?.textContent?.trim() ?? "";
}

const doneProgress: StreamProgressCardItem = {
  kind: "progress_card",
  seq: 53,
  run_id: RUN_ID,
  plan_id: "plan-r8-1",
  step_index: 0,
  tool_id: "create_task",
  status: "done",
  code: null,
  created_task_ids: ["task-r8-1"],
};

test("W5-8 R8 确认卡来源与条件", async (t) => {
  await t.test("同 run 计划卡披露「Sentinel-1」→ 来源显示 Sentinel-1，不显示 asf.catalog", () => {
    const items: StreamItem[] = [planWithDisclosure, confirmItem];
    const view = render(
      <AgentConfirmCard
        item={confirmItem}
        sourceLabel={confirmCardSourceLabel(RUN_ID, items, {})}
      />,
    );
    assert.equal(sourceRowText(view), "Sentinel-1");
    assert.equal(view.queryByText("asf.catalog") === null, true, "确认卡不得显示 provider id 原文");
    assert.equal(
      (view.container.textContent ?? "").includes("asf.catalog"),
      false,
      `实际：${view.container.textContent}`,
    );
  });

  await t.test("没有披露 → 来源显示「未知」", () => {
    const items: StreamItem[] = [confirmItem];
    const view = render(
      <AgentConfirmCard
        item={confirmItem}
        sourceLabel={confirmCardSourceLabel(RUN_ID, items, {})}
      />,
    );
    assert.equal(sourceRowText(view), "未知");
    assert.equal(
      (view.container.textContent ?? "").includes("asf.catalog"),
      false,
      `实际：${view.container.textContent}`,
    );
  });

  await t.test("未传 sourceLabel 时也不显示快照 source", () => {
    const view = render(<AgentConfirmCard item={confirmItem} />);
    assert.equal(
      (view.container.textContent ?? "").includes("asf.catalog"),
      false,
      `实际：${view.container.textContent}`,
    );
    assert.equal(sourceRowText(view), "未知");
  });

  await t.test("「条件」只显示日期", () => {
    const view = render(<AgentConfirmCard item={confirmItem} sourceLabel="Sentinel-1" />);
    assert.ok(view.getByText("2026-07-26 至 2026-10-26"));
    assert.equal(
      (view.container.textContent ?? "").includes("T00:00:00Z"),
      false,
      `实际：${view.container.textContent}`,
    );
  });

  await t.test("已批准卡的回退任务卡名称用同一来源显示名", () => {
    const view = render(
      <AgentConfirmCard
        item={confirmItem}
        sourceLabel="Sentinel-1"
        subsequentProgress={doneProgress}
        createdTaskIds={["task-r8-1"]}
        agentTasks={[]}
      />,
    );
    assert.ok(view.getByText("Sentinel-1 下载任务"));
    assert.equal(view.queryByText("task-r8-1") === null, true);
  });

  await t.test("来源未知时回退任务卡名称为「下载任务」", () => {
    const view = render(
      <AgentConfirmCard
        item={confirmItem}
        sourceLabel="未知"
        sourceNames={{ "asf.catalog": "不应出现" }}
        subsequentProgress={doneProgress}
        createdTaskIds={["task-r8-1"]}
        agentTasks={[]}
      />,
    );
    assert.ok(view.getByText("下载任务"));
    assert.equal(view.queryByText("未知 下载任务") === null, true);
  });
});

const bareScene: SceneRow = {
  scene_id: "G1234567890-ASF",
  product_key: "G1234567890-ASF",
  product_id: "G1234567890-ASF",
  provider: "asf.nisar",
  file_name: "",
  acquisition_datetime: "2026-10-20T01:02:03Z",
  has_url: false,
} as SceneRow;

test("W5-8 R8 足迹弹窗产品行", async (t) => {
  await t.test("新外壳：产品/波束/极化全空时不显示「- · - · -」", () => {
    const view = render(
      <ScenePopup scene={bareScene} index={0} titleMode="display_name" hideDownloadEntry />,
    );
    assert.equal(
      (view.container.textContent ?? "").includes("- · - · -"),
      false,
      `实际：${view.container.textContent}`,
    );
  });

  await t.test("新外壳：有部分字段时保留产品行", () => {
    const view = render(
      <ScenePopup
        scene={{ ...bareScene, product_type: "RSLC" }}
        index={0}
        titleMode="display_name"
        hideDownloadEntry
      />,
    );
    const text = view.container.textContent ?? "";
    // W5-8 R10 F5：新外壳模式下缺失字段写「未知」，不写「-」
    assert.ok(text.includes("RSLC · 未知 · 未知"), `实际：${text}`);
  });

  await t.test("旧界面默认模式不变", () => {
    const view = render(<ScenePopup scene={bareScene} index={0} />);
    assert.ok((view.container.textContent ?? "").includes("- · - · -"));
  });
});

test("W5-8 R8 Esc 层级", async (t) => {
  await t.test("偏好设置处理 Esc 时标记 defaultPrevented", () => {
    let closes = 0;
    render(
      <SettingsModal
        open
        initialCategory="storage"
        storagePanel={<div />}
        onClose={() => {
          closes += 1;
        }}
      />,
    );
    const notCancelled = fireEvent.keyDown(document.body, { key: "Escape" });
    assert.equal(closes, 1);
    assert.equal(notCancelled, false, "偏好设置处理 Esc 后必须 preventDefault");
  });

  await t.test("下载抽屉忽略已被消费（defaultPrevented）的 Esc", () => {
    let drawerCloses = 0;
    const consume = (e: KeyboardEvent) => {
      if (e.key === "Escape") e.preventDefault();
    };
    window.addEventListener("keydown", consume, true);
    try {
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
    } finally {
      window.removeEventListener("keydown", consume, true);
    }
    assert.equal(drawerCloses, 0, "已被上层消费的 Esc 不得再关闭下载抽屉");
  });
});
