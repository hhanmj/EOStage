import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { confirmCardSource } from "./searchCardSource.ts";
import { parseTaskOverview } from "./taskOverviewParser.ts";
import { mapTaskOverviewToApplicationTasks } from "./production.ts";
import { resolveApprovedOutputDir } from "./approvedOutputDirMemory.ts";
import { taskOutputDir } from "./taskLogic.ts";
import { runUsageText, turnUsageDetailLines, turnUsageText } from "./agentUsage.ts";
import { runBadgeFlags, runBadgeLabels } from "./agentRunBadges.ts";
import type { StreamConfirmCardItem, StreamItem, StreamPlanCardItem } from "./agentStreamLogic.ts";
import type { ApplicationTask } from "./types.ts";

const here = path.dirname(fileURLToPath(import.meta.url));

function read(rel: string): string {
  return fs.readFileSync(path.resolve(here, rel), "utf-8");
}

function confirmCard(snapshotExtra: Record<string, unknown>): StreamConfirmCardItem {
  return {
    kind: "confirm_card",
    seq: 20,
    run_id: "run-w59",
    plan_id: "plan-w59",
    plan_hash: "hash-w59",
    snapshot: {
      source: "asf.catalog",
      range: { aoi_name: "茂县叠溪", area_km2: 125.6, product: "1 个产品", time_range: null },
      estimated_bytes: null,
      output_dir: null,
      ...snapshotExtra,
    } as StreamConfirmCardItem["snapshot"],
    approval: null,
  };
}

const disclosurePlan: StreamPlanCardItem = {
  kind: "plan_card",
  seq: 10,
  run_id: "run-w59",
  plan_id: "plan-search",
  plan_hash: "hash-search",
  max_tier: 0,
  steps: [
    {
      tool_id: "search_start",
      status: "pending",
      disclosure: { resource_id: "science.sentinel-1.standard.v1", display_name_zh: "披露的来源", aoi_name: "茂县叠溪" },
    } as unknown as StreamPlanCardItem["steps"][number],
  ],
};

test("W5-9 条目 1：确认卡「来源」读快照 source_display_name_zh", async (t) => {
  await t.test("快照带显示名 → 用快照的显示名，不用计划卡披露", () => {
    const card = confirmCard({ source_display_name_zh: "Sentinel-1" });
    const items: StreamItem[] = [disclosurePlan, card];
    assert.equal(confirmCardSource(card, items, null), "Sentinel-1");
  });

  await t.test("快照显示名为 null → 「未知」，不回退到披露，不显示 snapshot.source", () => {
    const card = confirmCard({ source_display_name_zh: null });
    const items: StreamItem[] = [disclosurePlan, card];
    const label = confirmCardSource(card, items, { "asf.catalog": "不应出现" });
    assert.equal(label, "未知", `实际：${label}`);
  });

  await t.test("快照显示名为空白 → 「未知」", () => {
    const card = confirmCard({ source_display_name_zh: "   " });
    assert.equal(confirmCardSource(card, [card], null), "未知");
  });

  await t.test("旧卡没有该键 → 回退到同 run 计划卡披露", () => {
    const card = confirmCard({});
    const items: StreamItem[] = [disclosurePlan, card];
    assert.equal(confirmCardSource(card, items, null), "披露的来源");
  });

  await t.test("快照只读：读取前后键集与值不变（计划哈希）", () => {
    const withName = confirmCard({ source_display_name_zh: null });
    const without = confirmCard({});
    const before = [JSON.stringify(withName.snapshot), JSON.stringify(without.snapshot)];
    confirmCardSource(withName, [disclosurePlan, withName], null);
    confirmCardSource(without, [disclosurePlan, without], null);
    assert.equal(JSON.stringify(withName.snapshot), before[0]);
    assert.equal(JSON.stringify(without.snapshot), before[1]);
    assert.equal(Object.prototype.hasOwnProperty.call(without.snapshot, "source_display_name_zh"), false);
  });
});

function overviewTask(taskId: string, outputRoot: unknown): Record<string, unknown> {
  return {
    task_id: taskId,
    batch_id: "batch-1",
    provider_id: "asf",
    dataset_id: "science.sentinel-1.standard.v1",
    concurrency: 2,
    task_type: null,
    created_at: "2026-10-27T04:05:31Z",
    status: "completed",
    revision: 3,
    updated_at: "2026-10-27T04:07:00Z",
    selected_count: 1,
    delivery_requested_count: null,
    delivery_rejected_count: null,
    checked_count: 1,
    active_count: 0,
    paused_count: 0,
    queued_count: 0,
    completed_count: 1,
    failed_count: 0,
    skipped_count: 0,
    cancelled_count: 0,
    terminal_bytes: 10,
    active_bytes: 0,
    other_bytes: 0,
    transferred_bytes: 10,
    total_bytes: 10,
    source_total_bytes: null,
    output_root: outputRoot,
  };
}

test("W5-9 条目 2：任务总览 output_root 映射为 ApplicationTask.output_dir", async (t) => {
  const raw = {
    queue: [overviewTask("task-q", "D:\\ScenePort\\downloads")],
    history_page: {
      page_size: 20,
      has_more: false,
      next_cursor: null,
      items: [{ ...overviewTask("task-h", "E:\\archive"), archived_at: "2026-10-27T05:00:00Z" }],
    },
  };

  await t.test("解析保留 output_root，映射到队列与历史条目的 output_dir", () => {
    const tasks = mapTaskOverviewToApplicationTasks(parseTaskOverview(raw));
    const byId = new Map(tasks.map((task) => [task.task_id, task]));
    assert.equal(byId.get("task-q")?.output_dir, "D:\\ScenePort\\downloads");
    assert.equal(byId.get("task-h")?.output_dir, "E:\\archive");
  });

  await t.test("空字符串目录 → 视为未知（taskOutputDir 为 null）", () => {
    const tasks = mapTaskOverviewToApplicationTasks(
      parseTaskOverview({ queue: [overviewTask("task-e", "")], history_page: raw.history_page }),
    );
    const task = tasks.find((x) => x.task_id === "task-e");
    assert.equal(taskOutputDir(task), null);
  });

  await t.test("taskOutputDir：空白目录为 null，非空原样（不改写路径）", () => {
    assert.equal(taskOutputDir({ output_dir: "   " }), null);
    assert.equal(taskOutputDir({ output_dir: undefined }), null);
    assert.equal(taskOutputDir({ output_dir: "D:\\x" }), "D:\\x");
  });

  await t.test("旧数据缺 output_root 时不报错、不编造目录", () => {
    const legacy = overviewTask("task-old", undefined);
    delete legacy.output_root;
    const tasks = mapTaskOverviewToApplicationTasks(
      parseTaskOverview({ queue: [legacy], history_page: { ...raw.history_page, items: [] } }),
    );
    assert.equal(taskOutputDir(tasks[0]), null);
  });
});

function appTask(taskId: string, outputDir: string | undefined): ApplicationTask {
  return {
    task_id: taskId,
    revision: 1,
    category: "scene_download",
    name: "Sentinel-1",
    status: "cancelled",
    done_bytes: 0,
    output_dir: outputDir,
  } as ApplicationTask;
}

test("W5-9 条目 2：已批准卡目录 = 批准记录，否则已建任务的保存目录", async (t) => {
  await t.test("有批准记录 → 用记录", () => {
    assert.equal(resolveApprovedOutputDir("D:\\chosen", ["t1"], [appTask("t1", "D:\\task")]), "D:\\chosen");
  });
  await t.test("记录为空 → 用本卡已建任务的 output_dir", () => {
    const dir = resolveApprovedOutputDir(null, ["t1"], [appTask("t0", "D:\\other"), appTask("t1", "D:\\task")]);
    assert.equal(dir, "D:\\task", `实际：${dir}`);
  });
  await t.test("任务目录为空字符串 → null（卡上写「未知」）", () => {
    assert.equal(resolveApprovedOutputDir(null, ["t1"], [appTask("t1", "")]), null);
  });
  await t.test("不属于本卡的任务不参与", () => {
    assert.equal(resolveApprovedOutputDir(null, ["t1"], [appTask("t2", "D:\\other")]), null);
  });
});

test("W5-9 条目 2：本机目录只显示，不进发给模型的请求", () => {
  const stream = read("./agentStreamLogic.ts");
  const start = stream.indexOf("export function buildSendContext(");
  assert.ok(start >= 0, "找不到 buildSendContext");
  const end = stream.indexOf("\nexport ", start + 10);
  const body = stream.slice(start, end > 0 ? end : undefined);
  assert.equal(/output_dir|output_root/.test(body), false, "buildSendContext 不得读取目录字段");

  const workbench = read("../../components/desktop/DesktopAgentWorkbench.tsx");
  const sendAt = workbench.indexOf("bridge.agent.send(");
  assert.ok(sendAt >= 0, "找不到 agent_send 调用");
  const ctxAt = workbench.lastIndexOf("buildSendContext(", sendAt);
  const sendRegion = workbench.slice(ctxAt, sendAt + 80);
  assert.equal(/output_dir|output_root|agentTasks/.test(sendRegion), false, "发送请求的构造不得读取任务目录");
});

test("W5-9 条目 3：运行用量文案", async (t) => {
  await t.test("usage_total 为 null / 缺省 → 不显示（不是 0）", () => {
    assert.equal(runUsageText(null), null);
    assert.equal(runUsageText(undefined), null);
  });
  await t.test("完整 → 「用量 输入 X · 输出 Y」（千分位）", () => {
    assert.equal(runUsageText({ prompt: 12345, completion: 678, total: 13023 }), "用量 输入 12,345 · 输出 678");
  });
  await t.test("一部分为 null → 该部分「不完整」", () => {
    assert.equal(runUsageText({ prompt: null, completion: 678, total: 678 }), "用量 输入 不完整 · 输出 678");
    assert.equal(runUsageText({ prompt: 900, completion: null, total: 900 }), "用量 输入 900 · 输出 不完整");
  });
  await t.test("文案不含英文 token", () => {
    const text = runUsageText({ prompt: 1, completion: 2, total: 3 }) ?? "";
    assert.equal(/token/i.test(text), false, `实际：${text}`);
  });
});

function traceEvent(sequence: number, kind: string, payload: Record<string, unknown>) {
  return { sequence, previous_sha256: "0", kind, payload, sha256: "1" };
}

test("W5-9 条目 3（R3）：本轮用量读 usage_total，逐次明细才读 model_reply.usage", async (t) => {
  const flagsOff = { reasoning_off_refused: false, truncated: false, think_block_unclosed: false };

  await t.test("本轮用量文案来自 usage_total（与运行记录条同源），null 不显示", () => {
    assert.equal(turnUsageText({ prompt: 3000, completion: 120, total: 3120 }), "本轮用量 输入 3,000 · 输出 120");
    assert.equal(turnUsageText({ prompt: 3000, completion: null, total: 3000 }), "本轮用量 输入 3,000 · 输出 不完整");
    assert.equal(turnUsageText(null), null);
    assert.equal(turnUsageText(undefined), null);
  });

  await t.test("逐次明细：每个带用量的 model_reply 一行", () => {
    const lines = turnUsageDetailLines([
      traceEvent(0, "model_call", { file: "a.json", sha256: "x" }),
      traceEvent(1, "model_reply", { ...flagsOff, usage: { prompt: 1000, completion: 50, total: 1050 } }),
      traceEvent(2, "model_reply", { ...flagsOff, truncated: true }),
      traceEvent(3, "model_reply", { ...flagsOff, usage: { prompt: 2000, completion: null, total: 2000 } }),
    ]);
    assert.deepEqual(lines, ["第 1 次回复：输入 1,000 · 输出 50", "第 2 次回复：输入 2,000 · 输出 不完整"]);
  });

  await t.test("没有带用量的 model_reply 或不是数组 → 空", () => {
    assert.deepEqual(turnUsageDetailLines([traceEvent(1, "model_reply", { ...flagsOff })]), []);
    assert.deepEqual(turnUsageDetailLines(null), []);
  });
});

test("W5-9 条目 3：带用量的运行本身不是异常（徽章只看标记）", () => {
  const run = {
    run_id: "r",
    session_id: "s",
    status: "completed",
    started_at: "2026-10-27T05:00:00Z",
    ended_at: null,
    model_route: null,
    reasoning_off_refused: false,
    truncated: false,
    usage_total: { prompt: 10, completion: 2, total: 12 },
  };
  assert.deepEqual(runBadgeLabels(runBadgeFlags(run)), []);
});

test("W5-9 接线：确认卡来源、每轮用量行、运行条用量", () => {
  const workbench = read("../../components/desktop/DesktopAgentWorkbench.tsx");
  assert.equal(
    workbench.includes("sourceLabel={confirmCardSource(item, classifiedItems, sourceNames)}"),
    true,
    "确认卡分支必须用 confirmCardSource 读快照显示名",
  );
  assert.equal(workbench.includes("useRunTurnUsage("), false, "打开会话时不得逐个读取已结束运行的运行记录（R3 P1-2）");
  // Owner UX 决策 #4 + 2026-11-16 明确决策：回复底栏悬停用量图标仅直接展示本轮汇总（usage_total），不再拆分或请求「第 1/2 次回复」逐次明细
  assert.equal(workbench.includes("<AgentTurnUsageLine"), false, "不得在回复下方单独堆叠额外用量文字行");
  assert.equal(workbench.includes("usageTotalByRun.get(item.run_id)"), true, "回复底栏必须挂载本轮总用量（usage_total）");
  assert.equal(workbench.includes("requestUsageDetail("), false, "悬停本轮用量不再触发逐次子调用明细请求");

  const strip = read("../../components/desktop/AgentRunListStrip.tsx");
  assert.equal(strip.includes("runUsageText("), true, "运行记录条必须显示用量");

  const card = read("../../components/desktop/AgentConfirmCard.tsx");
  assert.equal(card.includes("resolveApprovedOutputDir("), true, "已批准卡目录必须回退到已建任务的保存目录");

  const drawer = read("../../components/desktop/DesktopDownloadDrawer.tsx");
  assert.equal(/task\.output_dir/.test(drawer), false, "下载抽屉的目录行与「打开目录」必须经 taskOutputDir 判空");
});

test("W5-9 R3 P1-1：桌面版「打开目录」走任务保存目录命令，不再用 open_path", () => {
  const shell = read("../../components/desktop/DesktopWorkbench.tsx");
  assert.equal(/openPath/.test(shell), false, "DesktopWorkbench 不得再引用 openPath（桌面宿主上它直接返回不可用）");
  assert.equal(shell.includes("openApplicationTaskOutput("), true, "必须调用 openApplicationTaskOutput");
  assert.equal(shell.includes("expected_revision"), true, "必须带任务修订号");
});
