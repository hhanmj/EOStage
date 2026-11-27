import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { isSearchToolGroup, searchCardContext } from "./searchCardSource.ts";
import type {
  StreamConfirmCardItem,
  StreamItem,
  StreamPlanCardItem,
  StreamToolGroupItem,
} from "./agentStreamLogic.ts";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

function round(tool_id: string, status: "running" | "done" | "failed" = "done") {
  return { run_id: "run-1", call_id: `call-${tool_id}`, tool_id, status } as StreamToolGroupItem["rounds"][number];
}

function planCard(
  runId: string,
  disclosure: Record<string, unknown> | null,
): StreamPlanCardItem {
  return {
    kind: "plan_card",
    seq: 7,
    run_id: runId,
    plan_id: "plan-1",
    plan_hash: "hash-1",
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
    seq: 20,
    run_id: runId,
    plan_id: "plan-2",
    plan_hash: "hash-2",
    snapshot: {
      source: "asf.sentinel1",
      range: {
        aoi_name: "茂县叠溪",
        area_km2: 120,
        time_range: { start: "2026-07-26T00:00:00Z", end: "2026-08-25T23:59:59Z" },
        product: "1 个产品",
      },
      estimated_bytes: null,
      output_dir: null,
    },
  };
}

test("W5-8 R5 条目 10：检索卡按工具类型分派", async (t) => {
  await t.test("仅 batch_page 回合不算检索", () => {
    assert.equal(isSearchToolGroup([round("batch_page")]), false, "只含 batch_page 的工具轮不得渲染「检索」卡");
  });
  await t.test("仅 workbench_page 回合不算检索", () => {
    assert.equal(isSearchToolGroup([round("workbench_page")]), false, "只含 workbench_page 的工具轮不得渲染「检索」卡");
  });
  await t.test("其他工具不算检索", () => {
    assert.equal(isSearchToolGroup([round("capabilities_get"), round("source_list")]), false);
  });
  await t.test("含 search_progress / search_start 才算检索", () => {
    assert.equal(isSearchToolGroup([round("search_progress")]), true);
    assert.equal(isSearchToolGroup([round("search_start")]), true);
    assert.equal(isSearchToolGroup([round("search_progress"), round("workbench_page")]), true);
  });
});

test("W5-8 R5 条目 10：首张检索卡的来源与条件", async (t) => {
  await t.test("计划卡披露带 display_name_zh → 用它作来源，AOI 与日期作条件", () => {
    const items: StreamItem[] = [
      planCard("run-1", {
        resource_id: "science.sentinel-1.standard.v1",
        aoi_name: "茂县叠溪（地图绘制矩形）",
        display_name_zh: "Sentinel-1",
        time_range: { start: "2026-07-26T00:00:00Z", end: "2026-08-25T23:59:59Z" },
      }),
    ];
    const ctx = searchCardContext("run-1", items, { "science.sentinel-1.standard.v1": "映射名" });
    assert.equal(ctx.source, "Sentinel-1", "首张检索卡（仅有计划卡、无确认卡）必须显示披露的来源");
    assert.deepEqual(ctx.conditionBadges, ["茂县叠溪（地图绘制矩形）", "2026-07-26 至 2026-08-25"]);
  });

  await t.test("无 display_name_zh 时用 sourceNames[resource_id]", () => {
    const items: StreamItem[] = [
      planCard("run-1", {
        resource_id: "science.sentinel-1.standard.v1",
        aoi_name: "西宁北",
        time_range: { start: "2026-07-26T00:00:00Z", end: null },
      }),
    ];
    const ctx = searchCardContext("run-1", items, { "science.sentinel-1.standard.v1": "Sentinel-1 标准产品" });
    assert.equal(ctx.source, "Sentinel-1 标准产品");
    assert.deepEqual(ctx.conditionBadges, ["西宁北", "2026-07-26 起"]);
  });

  await t.test("两者都无 → 来源 null，不显示 resource_id 原文", () => {
    const items: StreamItem[] = [
      planCard("run-1", {
        resource_id: "science.sentinel-1.standard.v1",
        aoi_name: "",
        display_name_zh: "  ",
        time_range: { start: null, end: "2026-08-25T23:59:59Z" },
      }),
    ];
    const ctx = searchCardContext("run-1", items, {});
    assert.equal(ctx.source, null);
    assert.deepEqual(ctx.conditionBadges, ["至 2026-08-25"]);
  });

  await t.test("其他 run 的计划卡不被借用", () => {
    const items: StreamItem[] = [
      planCard("run-other", { resource_id: "r", aoi_name: "别处", display_name_zh: "别的来源", time_range: null }),
    ];
    assert.deepEqual(searchCardContext("run-1", items, {}), { source: null, conditionBadges: [] });
  });

  await t.test("无披露、有确认卡 → 去掉「1 个产品」，ISO 时间改日期", () => {
    const items: StreamItem[] = [confirmCard("run-1")];
    const ctx = searchCardContext("run-1", items, { "asf.sentinel1": "Sentinel-1" });
    assert.equal(ctx.source, "Sentinel-1");
    assert.ok(
      !ctx.conditionBadges.some((b) => /个产品/.test(b)),
      `检索条件不得包含选择计数「N 个产品」，实际：${JSON.stringify(ctx.conditionBadges)}`,
    );
    assert.ok(
      !ctx.conditionBadges.some((b) => /T\d{2}:\d{2}/.test(b)),
      `检索条件不得包含 ISO 时间串，实际：${JSON.stringify(ctx.conditionBadges)}`,
    );
    assert.deepEqual(ctx.conditionBadges, ["茂县叠溪", "2026-07-26 至 2026-08-25"]);
  });

  await t.test("确认卡来源未映射 → null（不显示提供方原文）", () => {
    const ctx = searchCardContext("run-1", [confirmCard("run-1")], {});
    assert.equal(ctx.source, null);
  });

  await t.test("都没有 → 如实留空", () => {
    assert.deepEqual(searchCardContext("run-1", [], {}), { source: null, conditionBadges: [] });
  });
});

test("W5-8 R5 条目 10：DesktopAgentWorkbench 接线（源码断言）", () => {
  const src = fs.readFileSync(
    path.join(__dirname, "../../components/desktop/DesktopAgentWorkbench.tsx"),
    "utf-8",
  );
  assert.ok(
    !/SEARCH_TOOL_IDS/.test(src) && !/"batch_page",\s*"workbench_page"/.test(src),
    "DesktopAgentWorkbench 不得再把 batch_page / workbench_page 算作检索",
  );
  assert.ok(/isSearchToolGroup\(/.test(src), "tool_group 分支必须用 isSearchToolGroup 分派");
  assert.ok(/searchCardContext\(/.test(src), "检索卡来源与条件必须来自 searchCardContext");
});

test("W5-8 R5 条目 9：渠道 id 不用等宽字体（源码断言）", () => {
  const src = fs.readFileSync(
    path.join(__dirname, "../components/settings/ModelChannelsSettingsTab.tsx"),
    "utf-8",
  );
  assert.ok(
    !/font-mono break-all">\{draft\.id\}/.test(src),
    "草稿行渠道 id 不得使用 font-mono",
  );
  // Owner 2026-10-29: 消除独立编辑页标题行，改用 DSH 单列卡片头部
  assert.ok(!src.includes("font-mono break-all"), "渠道 id 不得使用等宽字体");
});
