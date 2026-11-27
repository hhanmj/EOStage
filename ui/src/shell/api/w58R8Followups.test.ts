import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { confirmCardSourceLabel } from "./searchCardSource.ts";
import { projectConfirmCard } from "./agentConversationLogic.ts";
import { executeAutoCancel } from "./autoCancelFlow.ts";
import type {
  StreamConfirmCardItem,
  StreamItem,
  StreamPlanCardItem,
} from "./agentStreamLogic.ts";
import type { ApplicationTask } from "./types.ts";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

function read(rel: string): string {
  return fs.readFileSync(path.resolve(__dirname, rel), "utf-8");
}

function planCard(runId: string, disclosure: Record<string, unknown> | null, seq = 7): StreamPlanCardItem {
  return {
    kind: "plan_card",
    seq,
    run_id: runId,
    plan_id: `plan-${seq}`,
    plan_hash: `hash-${seq}`,
    max_tier: 1,
    steps: [
      {
        tool_id: "search_start",
        label_zh: "检索数据",
        tier: 1,
        status: "pending",
        disclosure: disclosure as StreamPlanCardItem["steps"][number]["disclosure"],
      },
    ],
  };
}

function confirmCard(runId: string): StreamConfirmCardItem {
  return {
    kind: "confirm_card",
    seq: 50,
    run_id: runId,
    plan_id: "plan-confirm",
    plan_hash: "hash-confirm",
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
    },
  };
}

const s1Disclosure = {
  resource_id: "science.sentinel-1.standard.v1",
  aoi_name: "地图绘制矩形",
  display_name_zh: "Sentinel-1",
  time_range: { start: "2026-07-26T00:00:00Z", end: "2026-10-26T23:59:59Z" },
};

test("W5-8 R8 条目 1：确认卡来源只用同 run 计划卡披露的显示名", async (t) => {
  await t.test("同 run 计划卡披露「Sentinel-1」→ 来源为 Sentinel-1，不是 asf.catalog", () => {
    const items: StreamItem[] = [planCard("run-a", s1Disclosure), confirmCard("run-a")];
    assert.equal(confirmCardSourceLabel("run-a", items, {}), "Sentinel-1");
  });

  await t.test("没有披露 → 「未知」，不显示快照 source 原文", () => {
    const items: StreamItem[] = [confirmCard("run-a")];
    assert.equal(confirmCardSourceLabel("run-a", items, {}), "未知");
  });

  await t.test("别的 run 的披露不算", () => {
    const items: StreamItem[] = [planCard("run-other", s1Disclosure), confirmCard("run-a")];
    assert.equal(confirmCardSourceLabel("run-a", items, {}), "未知");
  });

  await t.test("同 run 披露了两个不同 resource_id → 「未知」", () => {
    const items: StreamItem[] = [
      planCard("run-a", s1Disclosure, 7),
      planCard(
        "run-a",
        { ...s1Disclosure, resource_id: "science.nisar.l.v1", display_name_zh: "NISAR L波段 (ASF)" },
        9,
      ),
      confirmCard("run-a"),
    ];
    assert.equal(confirmCardSourceLabel("run-a", items, {}), "未知");
  });

  await t.test("同一 resource_id 重复披露仍取显示名", () => {
    const items: StreamItem[] = [
      planCard("run-a", s1Disclosure, 7),
      planCard("run-a", s1Disclosure, 9),
      confirmCard("run-a"),
    ];
    assert.equal(confirmCardSourceLabel("run-a", items, {}), "Sentinel-1");
  });

  await t.test("披露无显示名时经数据源名称映射（键为 resource_id）", () => {
    const items: StreamItem[] = [
      planCard("run-a", { ...s1Disclosure, display_name_zh: "" }),
      confirmCard("run-a"),
    ];
    assert.equal(
      confirmCardSourceLabel("run-a", items, { "science.sentinel-1.standard.v1": "Sentinel-1 雷达卫星" }),
      "Sentinel-1 雷达卫星",
    );
  });

  await t.test("projectConfirmCard 与快照不变（审批需要原快照）", () => {
    const item = confirmCard("run-a");
    const before = JSON.stringify(item.snapshot);
    projectConfirmCard(item, null);
    confirmCardSourceLabel("run-a", [item], {});
    assert.equal(JSON.stringify(item.snapshot), before);
  });
});

test("W5-8 R8 条目 2：确认卡「条件」时间只显示日期", () => {
  const proj = projectConfirmCard(confirmCard("run-a"), null);
  assert.ok(
    proj.conditionBadges.includes("2026-07-26 至 2026-10-26"),
    `条件应为「2026-07-26 至 2026-10-26」，实际：${JSON.stringify(proj.conditionBadges)}`,
  );
  assert.equal(
    proj.conditionBadges.some((b) => b.includes("T00:00:00Z")),
    false,
    "条件不得出现 ISO 时间戳",
  );
});

function runningTask(taskId: string): ApplicationTask {
  return {
    task_id: taskId,
    name: "Sentinel-1 下载任务",
    provider: "asf",
    dataset: "s1_slc",
    status: "running",
    items_count: 1,
    total_bytes: 100,
    done_bytes: 10,
    output_dir: "D:\\ScenePort\\downloads",
    created_at: "2026-10-27T00:00:00Z",
    updated_at: "2026-10-27T00:00:10Z",
    revision: 3,
  } as ApplicationTask;
}

test("W5-8 R8 条目 6：runCancel 返回 false 表示运行已结束", async (t) => {
  await t.test("返回 false 且有任务被取消 →「本次运行已结束；已取消 1 个任务」", async () => {
    const result = await executeAutoCancel(
      {
        runLive: true,
        getFreshTasks: async () => [runningTask("task-1")],
        runCancel: async () => false,
        cancelTask: async () => {},
      },
      { runId: "run-a", createdTaskIds: ["task-1"] },
    );
    assert.equal(result.runCancelled, false);
    assert.equal(result.outcomeText.includes("已取消本次运行"), false, `实际：${result.outcomeText}`);
    assert.equal(result.outcomeText.includes("未能取消本次运行"), false, `实际：${result.outcomeText}`);
    assert.equal(result.outcomeText, "本次运行已结束；已取消 1 个任务");
  });

  await t.test("返回 false 且没有任务 →「本次运行已结束」", async () => {
    const result = await executeAutoCancel(
      {
        runLive: true,
        getFreshTasks: async () => [],
        runCancel: async () => false,
        cancelTask: async () => {},
      },
      { runId: "run-a", createdTaskIds: [] },
    );
    assert.equal(result.outcomeText, "本次运行已结束");
  });

  await t.test("返回 false 且任务刷新失败 →「本次运行已结束；未能确认任务状态」", async () => {
    const result = await executeAutoCancel(
      {
        runLive: true,
        getFreshTasks: async () => {
          throw new Error("ipc failed");
        },
        runCancel: async () => false,
        cancelTask: async () => {},
      },
      { runId: "run-a", createdTaskIds: ["task-1"] },
    );
    assert.equal(result.outcomeText, "本次运行已结束；未能确认任务状态");
  });

  await t.test("返回 true 仍为「已取消本次运行」", async () => {
    const result = await executeAutoCancel(
      {
        runLive: true,
        getFreshTasks: async () => [],
        runCancel: async () => true,
        cancelTask: async () => {},
      },
      { runId: "run-a", createdTaskIds: [] },
    );
    assert.equal(result.runCancelled, true);
    assert.equal(result.outcomeText, "已取消本次运行");
  });

  await t.test("DesktopAgentWorkbench 把 runCancel 的布尔结果交给取消编排器", () => {
    const src = read("../../components/desktop/DesktopAgentWorkbench.tsx");
    // 布尔断言：assert.match 失败时会把整份源码打进日志
    assert.equal(
      /return await bridge\.agent\.runCancel\(targetRunId\)/.test(src),
      true,
      "自动执行卡取消不得丢弃 runCancel 的返回值",
    );
  });
});

test("W5-8 R8 条目 1：DesktopAgentWorkbench 给确认卡传来源显示名", () => {
  const src = read("../../components/desktop/DesktopAgentWorkbench.tsx");
  const start = src.indexOf('case "confirm_card": {');
  assert.ok(start >= 0);
  const branch = src.slice(start, src.indexOf('case "progress_card"', start));
  // W5-9：来源改读快照 source_display_name_zh，计划卡披露只作旧卡回退（confirmCardSource 内部）
  assert.equal(
    /sourceLabel=\{confirmCardSource\(item, classifiedItems, sourceNames\)\}/.test(branch),
    true,
    "确认卡分支必须传入 sourceLabel（快照显示名，旧卡回退到同 run 计划卡披露）",
  );
});

test("W5-8 R8 条目 5：地图浮层 Esc 处理跳过已被消费的事件", () => {
  const src = read("../../components/desktop/ManualWorkbench.tsx");
  const start = src.indexOf('if (panelMode !== "overlay") return;');
  assert.ok(start >= 0);
  const handler = src.slice(start, src.indexOf("window.addEventListener", start));
  assert.ok(
    handler.includes("event.defaultPrevented"),
    "地图浮层 Esc 处理遇 defaultPrevented 必须直接返回",
  );
});
