import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

import type { SceneRow } from "../../lib/bridge.ts";
import type { RowTaskState, ApplicationTask, TaskFailureSummary, StoreTaskSummary } from "./types.ts";
import {
  sortScenes,
  nextSort,
  DEFAULT_SORT_STATE,
  isRowSelectable,
  selectableIds,
  toggleAll,
  headerCheckState,
  checkedSummary,
  paginate,
  failureFirstSentence,
  failureSummaryText,
  rowProgressPercent,
  createTaskErrorText,
  createTaskOutcomeNotice,
  togglePageSelection,
  isRowCheckedInScope,
  toggleRowInAllMatch,
  togglePageInAllMatch,
  checkStateInScope,
  allMatchSummary,
} from "./resultListLogic.ts";
import { parseStoreTaskSummary, parseStoreTaskHistoryItem } from "./taskOverviewParser.ts";
import { ERROR_CODE_MAP } from "./errorExplanations.ts";
import {
  describeDownloadFailure,
  explainDownloadCode,
} from "./errorExplanations.ts";
import { projectSnapshotItemToSceneRow } from "./rowProjection.ts";
import { assertLocalizedSource } from "./i18nSourceTestUtils.test.ts";

function makeScene(id: string, overrides: Partial<SceneRow> = {}): SceneRow {
  return {
    scene_id: id,
    product_key: id,
    product_id: id,
    provider: "sentinel",
    file_name: `${id}.SAFE`,
    acquisition_datetime: "2026-10-20T10:00:00Z",
    platform: "Sentinel-1A",
    product_type: "SLC",
    beam_mode: "IW",
    polarization: "VV+VH",
    orbit_direction: "ascending",
    relative_orbit: 11,
    frame: 65,
    absolute_orbit: 50000,
    path: null,
    file_size_remote: 1024 * 1024 * 100, // 100 MB
    has_url: false,
    ...overrides,
  };
}

test("W5-3b (a) 场景排序与状态机 nextSort", async (t) => {
  await t.test("sortScenes 按照日期 asc/desc 排序，缺失值始终排在最后，ties 由 scene_id 升序决胜", () => {
    const s1 = makeScene("S1", { acquisition_datetime: "2026-10-21T00:00:00Z" });
    const s2 = makeScene("S2", { acquisition_datetime: "2026-10-19T00:00:00Z" });
    const s3 = makeScene("S3", { acquisition_datetime: "" }); // 缺失
    const s4 = makeScene("S4", { acquisition_datetime: "2026-10-21T00:00:00Z" }); // 与 S1 同分

    const list = [s3, s1, s2, s4];

    // desc: S1 与 S4 相同 -> 按 scene_id 升序 S1 前 S4 后; 然后 S2; 缺失的 S3 垫底
    const desc = sortScenes(list, "date", "desc");
    assert.deepStrictEqual(desc.map((s) => s.scene_id), ["S1", "S4", "S2", "S3"]);

    // asc: S2 最小; S1 与 S4 相同 -> 按 scene_id 升序 S1 前 S4 后; 缺失的 S3 仍然垫底
    const asc = sortScenes(list, "date", "asc");
    assert.deepStrictEqual(asc.map((s) => s.scene_id), ["S2", "S1", "S4", "S3"]);

    // 原数组不可变
    assert.strictEqual(list[0].scene_id, "S3");
  });

  await t.test("sortScenes 按照 relativeOrbit 与 frame 排序，缺失值始终垫底", () => {
    const s1 = makeScene("S1", { relative_orbit: 20 });
    const s2 = makeScene("S2", { relative_orbit: null });
    const s3 = makeScene("S3", { relative_orbit: 5 });

    const asc = sortScenes([s1, s2, s3], "relativeOrbit", "asc");
    assert.deepStrictEqual(asc.map((s) => s.scene_id), ["S3", "S1", "S2"]);

    const desc = sortScenes([s1, s2, s3], "relativeOrbit", "desc");
    assert.deepStrictEqual(desc.map((s) => s.scene_id), ["S1", "S3", "S2"]);
  });

  await t.test("sortScenes 按照 size 排序，缺失/无效值垫底", () => {
    const s1 = makeScene("S1", { file_size_remote: 1000 });
    const s2 = makeScene("S2", { file_size_remote: null });
    const s3 = makeScene("S3", { file_size_remote: 5000 });

    const desc = sortScenes([s1, s2, s3], "size", "desc");
    assert.deepStrictEqual(desc.map((s) => s.scene_id), ["S3", "S1", "S2"]);

    const asc = sortScenes([s1, s2, s3], "size", "asc");
    assert.deepStrictEqual(asc.map((s) => s.scene_id), ["S1", "S3", "S2"]);
  });

  await t.test("nextSort: 相同键翻转方向，新键按规范初始（date/size desc，relativeOrbit/frame asc）", () => {
    assert.deepStrictEqual(DEFAULT_SORT_STATE, { key: "date", dir: "desc" });

    // 相同键翻转
    assert.deepStrictEqual(nextSort({ key: "date", dir: "desc" }, "date"), { key: "date", dir: "asc" });
    assert.deepStrictEqual(nextSort({ key: "date", dir: "asc" }, "date"), { key: "date", dir: "desc" });

    // 切换到新键
    assert.deepStrictEqual(nextSort({ key: "date", dir: "desc" }, "relativeOrbit"), { key: "relativeOrbit", dir: "asc" });
    assert.deepStrictEqual(nextSort({ key: "date", dir: "desc" }, "frame"), { key: "frame", dir: "asc" });
    assert.deepStrictEqual(nextSort({ key: "relativeOrbit", dir: "asc" }, "size"), { key: "size", dir: "desc" });
    assert.deepStrictEqual(nextSort({ key: "relativeOrbit", dir: "asc" }, "date"), { key: "date", dir: "desc" });
  });
});

test("W5-3b (b) 勾选可选性与三态全选", async (t) => {
  await t.test("isRowSelectable: 运行中、排队中、暂停、完成、中断不可勾选；无状态、失败或取消可勾选", () => {
    assert.strictEqual(isRowSelectable(undefined), true);
    assert.strictEqual(isRowSelectable(null), true);
    assert.strictEqual(isRowSelectable({ task_id: "t1", item_status: "ok" } as any), true);

    assert.strictEqual(isRowSelectable({ task_status: "failed" } as any), true);
    assert.strictEqual(isRowSelectable({ task_status: "cancelled" } as any), true);

    assert.strictEqual(isRowSelectable({ task_status: "queued" } as any), false);
    assert.strictEqual(isRowSelectable({ task_status: "running" } as any), false);
    assert.strictEqual(isRowSelectable({ task_status: "paused" } as any), false);
    assert.strictEqual(isRowSelectable({ task_status: "completed" } as any), false);
    assert.strictEqual(isRowSelectable({ task_status: "interrupted" } as any), false);
  });

  await t.test("selectableIds: 仅提取未入任务的有效场景 id", () => {
    const s1 = makeScene("S1");
    const s2 = makeScene("S2");
    const s3 = makeScene("S3");
    const rowStates = new Map<string, RowTaskState>([
      ["S2", { task_status: "running", task_id: "t1", item_status: "active", actions: [] }],
      ["S3", { task_status: "failed", task_id: "t2", item_status: "failed", actions: [] }],
    ]);

    const ids = selectableIds([s1, s2, s3], rowStates);
    assert.deepStrictEqual(ids, ["S1", "S3"]);
  });

  await t.test("toggleAll 与 headerCheckState: 三态全选联动", () => {
    const selectable = ["S1", "S2", "S3"];

    // 1. 全部未勾选 -> "none"
    let checked = new Set<string>();
    assert.strictEqual(headerCheckState(checked, selectable), "none");

    // 点击 toggleAll -> 全部勾选
    checked = toggleAll(checked, selectable);
    assert.deepStrictEqual(Array.from(checked).sort(), ["S1", "S2", "S3"]);
    assert.strictEqual(headerCheckState(checked, selectable), "all");

    // 2. 部分勾选 -> "some"
    checked = new Set(["S1"]);
    assert.strictEqual(headerCheckState(checked, selectable), "some");

    // 点击 toggleAll -> 补齐全部
    checked = toggleAll(checked, selectable);
    assert.deepStrictEqual(Array.from(checked).sort(), ["S1", "S2", "S3"]);

    // 再次点击 toggleAll -> 全清空
    checked = toggleAll(checked, selectable);
    assert.strictEqual(checked.size, 0);
    assert.strictEqual(headerCheckState(checked, selectable), "none");

    // 无可选场景时 -> "none"
    assert.strictEqual(headerCheckState(new Set(["OTHER"]), []), "none");
  });
});

test("W5-3b (c) 勾选摘要 checkedSummary", async (t) => {
  await t.test("计算已勾选数量、已知大小与未知大小，并生成规范文本", () => {
    const s1 = makeScene("S1", { file_size_remote: 1024 * 1024 * 1024 }); // 1.0 GB
    const s2 = makeScene("S2", { file_size_remote: 2 * 1024 * 1024 * 1024 }); // 2.0 GB
    const s3 = makeScene("S3", { file_size_remote: null }); // 未知大小
    const s4 = makeScene("S4", { file_size_remote: 0 });
    const scenes = [s1, s2, s3, s4];

    // 0 景
    assert.strictEqual(checkedSummary(scenes, new Set()).summaryText, "已勾选 0");

    // 全部已知
    const resAllKnown = checkedSummary(scenes, new Set(["S1", "S2"]));
    assert.strictEqual(resAllKnown.count, 2);
    assert.strictEqual(resAllKnown.unknownCount, 0);
    assert.strictEqual(resAllKnown.summaryText, "已勾选 2（3.0 GB）");

    // 部分未知
    const resPartial = checkedSummary(scenes, new Set(["S1", "S2", "S3"]));
    assert.strictEqual(resPartial.count, 3);
    assert.strictEqual(resPartial.unknownCount, 1);
    assert.strictEqual(resPartial.summaryText, "已勾选 3（3.0 GB，另 1 景大小未知）");

    // 全部未知
    const resAllUnknown = checkedSummary(scenes, new Set(["S3"]));
    assert.strictEqual(resAllUnknown.count, 1);
    assert.strictEqual(resAllUnknown.unknownCount, 1);
    assert.strictEqual(resAllUnknown.summaryText, "已勾选 1（大小未知）");
  });
});

test("W5-3b (d) 前端分页 paginate", async (t) => {
  await t.test("分页钳位与切片", () => {
    const list = Array.from({ length: 105 }, (_, i) => `item_${i}`);

    // 第 1 页
    const p1 = paginate(list, 1, 50);
    assert.strictEqual(p1.page, 1);
    assert.strictEqual(p1.pageCount, 3);
    assert.strictEqual(p1.items.length, 50);
    assert.strictEqual(p1.items[0], "item_0");

    // 第 3 页（最后一页 5 项）
    const p3 = paginate(list, 3, 50);
    assert.strictEqual(p3.page, 3);
    assert.strictEqual(p3.pageCount, 3);
    assert.strictEqual(p3.items.length, 5);

    // 越界自动钳位
    const pOverflow = paginate(list, 999, 50);
    assert.strictEqual(pOverflow.page, 3);

    const pUnderflow = paginate(list, 0, 50);
    assert.strictEqual(pUnderflow.page, 1);

    // 空列表安全
    const pEmpty = paginate([], 1, 50);
    assert.strictEqual(pEmpty.page, 1);
    assert.strictEqual(pEmpty.pageCount, 1);
    assert.strictEqual(pEmpty.items.length, 0);
  });
});

test("W5-3b (e) 失败首句截取 failureFirstSentence 与摘要 failureSummaryText", async (t) => {
  await t.test("failureFirstSentence: 首个句号或换行截取，120 字截断加省略号", () => {
    assert.strictEqual(failureFirstSentence(""), "");
    assert.strictEqual(failureFirstSentence(null), "");
    assert.strictEqual(failureFirstSentence("网络连接超时。请稍后重试。"), "网络连接超时。");
    assert.strictEqual(failureFirstSentence("第一行错误\n第二行详情"), "第一行错误");
    assert.strictEqual(failureFirstSentence("句号在前。第二行\n详情"), "句号在前。");

    const longMsg = "A".repeat(150);
    const capped = failureFirstSentence(longMsg);
    assert.strictEqual(capped.length, 121); // 120 + "…"
    assert.strictEqual(capped.endsWith("…"), true);
  });

  await t.test("failureSummaryText: message_zh 优先，错误码次之，前缀支持多项失败", () => {
    // 1. 单项任务携带 message_zh
    const f1: TaskFailureSummary = {
      codes: ["NET_ERR"],
      message_zh: "上游服务不可达。连接超时",
      failed_items: 1,
      total_items: 1,
    };
    assert.strictEqual(failureSummaryText(f1), "上游服务不可达。");

    // 2. 多项任务携带前缀
    const f2: TaskFailureSummary = {
      codes: ["NET_ERR"],
      message_zh: "部分切片下载失败。",
      failed_items: 2,
      total_items: 5,
    };
    assert.strictEqual(failureSummaryText(f2), "2/5 项失败：部分切片下载失败。");

    // 3. 无 message_zh，从 codes[0] 解析已知错误码
    const f3: TaskFailureSummary = {
      codes: ["model_timeout"],
      message_zh: null,
      failed_items: 1,
      total_items: 1,
    };
    assert.strictEqual(failureSummaryText(f3), "模型服务响应超时");

    // 4. 无 message_zh 且未知错误码
    const f4: TaskFailureSummary = {
      codes: ["UNKNOWN_CODE"],
      message_zh: null,
      failed_items: 1,
      total_items: 1,
    };
    assert.strictEqual(failureSummaryText(f4), "失败原因未返回");

    // 5. null failure
    assert.strictEqual(failureSummaryText(null), "失败原因未返回");
  });
});

test("W5-3b (f) 单景任务进度百分比 rowProgressPercent", async (t) => {
  await t.test("仅单景且 total_bytes > 0 时返回百分比整数，其余严格返回 null", () => {
    const singleTask: ApplicationTask = {
      task_id: "t1",
      revision: 1,
      category: "scene_download",
      name: "Task 1",
      status: "running",
      items_count: 1,
      done_bytes: 4600,
      total_bytes: 10000,
    };
    assert.strictEqual(rowProgressPercent(singleTask), 46);

    // 多景任务不可挪用为单行进度
    const multiTask: ApplicationTask = {
      ...singleTask,
      items_count: 3,
    };
    assert.strictEqual(rowProgressPercent(multiTask), null);

    // total_bytes <= 0 或缺失
    assert.strictEqual(rowProgressPercent({ ...singleTask, total_bytes: 0 }), null);
    assert.strictEqual(rowProgressPercent({ ...singleTask, total_bytes: null }), null);
    assert.strictEqual(rowProgressPercent(undefined), null);
  });
});

test("W5-3b (g) Parser 支持 failure 解析与容错", async (t) => {
  const baseRaw: Record<string, unknown> = {
    task_id: "task_1",
    batch_id: "batch_1",
    provider_id: "sentinel",
    dataset_id: "slc",
    concurrency: 2,
    task_type: null,
    created_at: "2026-10-20T00:00:00Z",
    status: "failed",
    revision: 1,
    updated_at: "2026-10-20T00:01:00Z",
    selected_count: 2,
    delivery_requested_count: null,
    delivery_rejected_count: null,
    checked_count: 2,
    active_count: 0,
    paused_count: 0,
    queued_count: 0,
    completed_count: 0,
    failed_count: 2,
    skipped_count: 0,
    cancelled_count: 0,
    terminal_bytes: 0,
    active_bytes: 0,
    other_bytes: 0,
    transferred_bytes: 0,
    total_bytes: 2000,
    source_total_bytes: null,
  };

  await t.test("正常 failure 结构解析", () => {
    const raw = {
      ...baseRaw,
      failure: {
        codes: ["NET_ERR"],
        message_zh: "网络错误",
        failed_items: 2,
        total_items: 2,
      },
    };
    const parsed = parseStoreTaskSummary(raw);
    assert.ok(parsed.failure);
    assert.deepStrictEqual(parsed.failure.codes, ["NET_ERR"]);
    assert.strictEqual(parsed.failure.message_zh, "网络错误");
    assert.strictEqual(parsed.failure.failed_items, 2);
    assert.strictEqual(parsed.failure.total_items, 2);
  });

  await t.test("failure 缺失或 null 返回 null", () => {
    const parsedNull = parseStoreTaskSummary({ ...baseRaw, failure: null });
    assert.strictEqual(parsedNull.failure, null);

    const parsedAbsent = parseStoreTaskSummary(baseRaw);
    assert.strictEqual(parsedAbsent.failure, null);
  });

  await t.test("failure 格式畸变时不抛出异常，安全回退为 null", () => {
    const parsedMalformed = parseStoreTaskSummary({
      ...baseRaw,
      failure: { codes: "not-an-array", total_items: "two" },
    });
    assert.strictEqual(parsedMalformed.failure, null);
  });

  await t.test("parseStoreTaskHistoryItem 同步解析 failure", () => {
    const historyRaw = {
      ...baseRaw,
      archived_at: "2026-10-20T01:00:00Z",
      failure: {
        codes: ["ERR_1"],
        message_zh: "已归档失败",
        failed_items: 1,
        total_items: 1,
      },
    };
    const parsed = parseStoreTaskHistoryItem(historyRaw);
    assert.ok(parsed.failure);
    assert.strictEqual(parsed.failure?.message_zh, "已归档失败");
  });
});

test("W5-3b (h) 错误码映射与 createTaskErrorText", async (t) => {
  await t.test("ERROR_CODE_MAP 包含 STORAGE_ROOT_NOT_WRITABLE", () => {
    assert.strictEqual(
      ERROR_CODE_MAP["STORAGE_ROOT_NOT_WRITABLE"],
      "存储根路径不可写，请检查目录权限或更换路径"
    );
  });

  await t.test("createTaskErrorText: STORAGE_ROOT_NOT_WRITABLE 带路径时优先返回 message_zh", () => {
    const errWithPath = {
      code: "STORAGE_ROOT_NOT_WRITABLE",
      message_zh: "存储根路径不可写：D:\\ScenePortData（请检查目录权限或更换路径）",
    };
    assert.strictEqual(
      createTaskErrorText(errWithPath),
      "存储根路径不可写：D:\\ScenePortData（请检查目录权限或更换路径）"
    );

    // 若无具体 message_zh，返回标准中文解释
    const errWithoutMsg = {
      code: "STORAGE_ROOT_NOT_WRITABLE",
    };
    assert.strictEqual(
      createTaskErrorText(errWithoutMsg),
      "存储根路径不可写，请检查目录权限或更换路径"
    );

    // 未知错误回退至「任务创建失败」
    assert.strictEqual(createTaskErrorText("other"), "任务创建失败");
  });

  await t.test("createTaskOutcomeNotice: 任务创建失败呈现 danger 通知，沿用未完成任务呈现 neutral 通知，正常新建返回 null", () => {
    // 1. STORAGE_ROOT_NOT_WRITABLE 带路径
    const errWithPath = {
      code: "STORAGE_ROOT_NOT_WRITABLE",
      message_zh: "存储根路径不可写：D:\\ScenePortData（请检查目录权限或更换路径）",
    };
    assert.deepStrictEqual(createTaskOutcomeNotice(errWithPath), {
      tone: "danger",
      text: "存储根路径不可写：D:\\ScenePortData（请检查目录权限或更换路径）",
    });

    // 2. 未知错误
    assert.deepStrictEqual(createTaskOutcomeNotice(new Error("network error")), {
      tone: "danger",
      text: "任务创建失败",
    });

    // 3. created = false (沿用未完成任务)
    assert.deepStrictEqual(createTaskOutcomeNotice({ created: false }), {
      tone: "neutral",
      text: "已有相同的未完成任务，已沿用该任务",
    });

    // 4. created = true (正常新建)
    assert.strictEqual(createTaskOutcomeNotice({ created: true }), null);
  });
});

test("W5-3b 源码级门禁断言", async (t) => {
  const desktopDir = path.resolve(__dirname, "../../components/desktop");

  await t.test("ManualWorkbench 渲染四项排序键与三态复选框", () => {
    const workbenchPath = path.join(desktopDir, "ManualWorkbench.tsx");
    const content = fs.readFileSync(workbenchPath, "utf8");

    // 四项排序键
    assert.ok(content.includes('key: "date", labelKey: "mapWorkbench.results.date"'), "Must include date sort button");
    assertLocalizedSource(content, "mapWorkbench.results.date", "日期");
    assert.ok(content.includes('key: "relativeOrbit", label: "Path"'), "Must include relativeOrbit sort button");
    assert.ok(content.includes('key: "frame", label: "Frame"'), "Must include frame sort button");
    assert.ok(content.includes('key: "size", labelKey: "mapWorkbench.results.size"'), "Must include size sort button");
    assertLocalizedSource(content, "mapWorkbench.results.size", "大小");

    // 三态复选框
    assert.ok(
      content.includes('aria-checked={checkState === "some" ? "mixed" : checkState === "all" ? "true" : "false"}'),
      "Must have tri-state aria-checked"
    );
    assert.ok(content.includes('aria-label={t("mapWorkbench.results.selectAllLoaded")}'), "Must make loaded-row select-all scope explicit");
    assertLocalizedSource(content, "mapWorkbench.results.selectAllLoaded", "全选已加载");
  });

  await t.test("CatalogResultRow 使用 deriveVisibleRowActions 与 ThumbnailSlot，且不包含平台徽标与 font-mono 徽章", () => {
    const rowPath = path.join(desktopDir, "CatalogResultRow.tsx");
    const content = fs.readFileSync(rowPath, "utf8");

    assert.ok(content.includes("deriveVisibleRowActions("), "CatalogResultRow must call deriveVisibleRowActions");
    assert.ok(content.includes("<ThumbnailSlot"), "CatalogResultRow must mount <ThumbnailSlot");
    assert.strictEqual(content.includes("scene.platform"), false, "CatalogResultRow must drop platform badge");

    // Owner 2026-10-29: task C switches CatalogResultRow to two-line layout (Line 1 date/orbit, Line 2 Path/Frame/pol/size with font-mono for numbers)
    assert.ok(content.includes("acqDate") || content.includes("badges.map"), "Must render acquisition date or badges");

    // 支持失败展开与错误码格式化
    assert.ok(content.includes("isFailureExpanded"), "Must support isFailureExpanded");
    assert.ok(content.includes("canExpandFailure"), "Must support canExpandFailure");
    assert.ok(content.includes("formatErrorWithCode"), "Must format error codes via formatErrorWithCode");
  });

  await t.test("ManualWorkbench 支持就地任务通知条与展开状态传递", () => {
    const workbenchPath = path.join(desktopDir, "ManualWorkbench.tsx");
    const content = fs.readFileSync(workbenchPath, "utf8");

    assert.ok(content.includes("taskOutcomeNotice"), "ManualWorkbench must render taskOutcomeNotice");
    assert.ok(content.includes("onDismissTaskOutcomeNotice"), "ManualWorkbench must support dismissing notice");
    assert.ok(content.includes("expandedFailureSceneIds"), "ManualWorkbench must manage failure expansion state");
  });

  await t.test("#22 B Slice 4: 本页 / 已加载 / 全部匹配 三态选择范围与全匹配预检归账逻辑", () => {
    const initialChecked = new Set(["other-page-1"]);
    const pageSelected = togglePageSelection(initialChecked, ["p1-a", "p1-b"]);
    assert.deepEqual(Array.from(pageSelected).sort(), ["other-page-1", "p1-a", "p1-b"]);
    const pageUnselected = togglePageSelection(pageSelected, ["p1-a", "p1-b"]);
    assert.deepEqual(Array.from(pageUnselected), ["other-page-1"]);

    const rowStates = new Map<string, RowTaskState>([
      ["s-running", { item_id: "s-running", task_id: "t-1", task_status: "running", item_status: "running", actions: ["pause"] }],
      ["s-failed", { item_id: "s-failed", task_id: "t-0", task_status: "failed", item_status: "failed", actions: ["retry"] }],
    ]);
    let excluded = new Set<string>();
    assert.equal(isRowCheckedInScope("s-1", undefined, new Set(), { active: true, excludedIds: excluded }), true);
    assert.equal(isRowCheckedInScope("s-failed", rowStates.get("s-failed"), new Set(), { active: true, excludedIds: excluded }), true);
    assert.equal(isRowCheckedInScope("s-running", rowStates.get("s-running"), new Set(), { active: true, excludedIds: excluded }), false);
    assert.equal(checkStateInScope(["s-1", "s-failed"], new Set(), { active: true, excludedIds: excluded }), "all");

    excluded = toggleRowInAllMatch(excluded, "s-1");
    assert.equal(excluded.has("s-1"), true);
    assert.equal(isRowCheckedInScope("s-1", undefined, new Set(), { active: true, excludedIds: excluded }), false);
    assert.equal(checkStateInScope(["s-1", "s-failed"], new Set(), { active: true, excludedIds: excluded }), "some");

    excluded = togglePageInAllMatch(excluded, ["s-1", "s-failed"]);
    assert.equal(excluded.size, 0);
    assert.equal(checkStateInScope(["s-1", "s-failed"], new Set(), { active: true, excludedIds: excluded }), "all");

    excluded = togglePageInAllMatch(excluded, ["s-1", "s-failed"]);
    assert.deepEqual(Array.from(excluded).sort(), ["s-1", "s-failed"]);
    assert.equal(checkStateInScope(["s-1", "s-failed"], new Set(), { active: true, excludedIds: excluded }), "none");

    const loadedScenes = [
      makeScene("s-1", { size_bytes: 1_000_000_000 }),
      makeScene("s-running", { size_bytes: 2_000_000_000 }),
      makeScene("s-failed", { size_bytes: 500_000_000 }),
    ];
    const summaryFull = allMatchSummary(120, loadedScenes, rowStates, new Set());
    assert.equal(summaryFull.count, 119);
    assert.equal(summaryFull.summaryText, "已选全部匹配 119 景");

    const summaryExcl = allMatchSummary(120, loadedScenes, rowStates, new Set(["s-1"]));
    assert.equal(summaryExcl.count, 118);
    assert.equal(summaryExcl.summaryText, "已选全部匹配 118 景（已排除 1 景）");

    const outcomeNotice = createTaskOutcomeNotice({
      task_id: "task-all-1",
      revision: 1,
      status: "queued",
      created: true,
      matched_count: 120,
      created_count: 115,
      pending_count: 115,
      skipped_count: 4,
      rejected_count: 1,
    });
    assert.deepEqual(outcomeNotice, {
      tone: "neutral",
      text: "已创建全部匹配任务：入队 115 景，跳过进行中 4 景，不可下载拒绝 1 景",
    });
    assert.equal(
      createTaskErrorText({ code: "V3_TASK_BATCH_INCOMPLETE" }),
      "当前检索批次未完整结束，不能按全部匹配创建任务",
    );
    assert.equal(
      createTaskErrorText({ code: "V3_TASK_OVER_CEILING" }),
      "匹配结果超过单任务安全上限（10000 景），请缩小范围或时间后再试",
    );
    assert.equal(
      createTaskErrorText({ code: "V3_TASK_SELECTION_CHANGED" }),
      "全部匹配选择状态或进行中任务已变化，请重新确认后再创建任务",
    );
    assert.equal(
      createTaskErrorText({ code: "V3_TASK_EXCLUSIONS_TOO_MANY" }),
      "排除项超过上限（250 景），请缩小筛选条件后再试",
    );
  });

  await t.test("Item 1: projectSnapshotItemToSceneRow preserves authoritative NISAR release_tier and collection metadata", () => {
    const validatedProjected = projectSnapshotItemToSceneRow({
      item_id: "NISAR_L1_PR_RSLC_001_055_A_001_2005_DHDH_A_20261101T120000_20261101T120030_X05007_N_F_J_001",
      provider_id: "asf.nasa.earthdata",
      display_name: "NISAR_L1_PR_RSLC_001_055_A_001_2005_DHDH_A_20261101T120000_20261101T120030_X05007_N_F_J_001.h5",
      acquisition_datetime: "2026-11-01T12:00:00Z",
      footprint_geojson: null,
      metadata: {
        platform: "NISAR",
        product_type: "RSLC",
      },
      payload: {
        release_tier: "validated",
        tier: "Validated",
        collection_concept_id: "C3622236985-ASF",
        collection_short_name: "NISAR_L1_RSLC_V1",
      },
    } as any);

    const meta = validatedProjected.native_metadata as Record<string, unknown>;
    assert.equal(meta.release_tier, "validated");
    assert.equal(meta.tier, "Validated");
    assert.equal(meta.collection_concept_id, "C3622236985-ASF");
    assert.equal(meta.collection_short_name, "NISAR_L1_RSLC_V1");
  });

  await t.test("Item 10: describeDownloadFailure & failureSummaryText surface real sanitized failure causes and guidance", () => {
    const http503 = describeDownloadFailure({
      codes: ["DL005"],
      messageZh: "上游服务返回 HTTP 503（服务暂时不可用），已按重试策略处理。",
    });
    assert.equal(http503.cause, "上游服务返回 HTTP 503（服务暂时不可用），已按重试策略处理。");
    assert.ok(http503.guidance?.includes("繁忙或触发限流"));
    assert.deepEqual(http503.extraExplanations, []);
    assert.deepEqual(http503.diagnosticCodes, ["DL005"]);

    const tlsErr = describeDownloadFailure({
      codes: ["DL005"],
      messageZh: "TLS 握手失败：证书校验异常。",
    });
    assert.ok(tlsErr.guidance?.includes("TLS/证书"));

    const legacyBare = describeDownloadFailure({
      codes: ["DL005"],
      messageZh: "下载连接暂时失败，已按重试策略处理。",
    });
    assert.ok(legacyBare.extraExplanations.some((line) => line.includes("下载传输连接中断")));
    assert.ok(legacyBare.guidance?.includes("网络与代理"));

    const codeOnlyDl005: TaskFailureSummary = {
      codes: ["DL005"],
      message_zh: null,
      failed_items: 1,
      total_items: 1,
    };
    assert.equal(failureSummaryText(codeOnlyDl005), explainDownloadCode("DL005"));
    assert.ok(explainDownloadCode("DL005", "en")?.includes("Download connection interrupted"));
    assert.ok(explainDownloadCode("DL005", "ja")?.includes("ダウンロード接続が中断"));
  });
});

import {
  staleSearchBatchAgeDays,
  staleSearchBatchAgeLabel,
} from "./manualConditionsLogic.ts";

test("staleSearchBatchAgeDays & staleSearchBatchAgeLabel detect batches older than 24h", () => {
  const now = new Date("2026-11-25T12:00:00Z");
  assert.equal(staleSearchBatchAgeDays(null, now), null);
  assert.equal(staleSearchBatchAgeDays("invalid", now), null);
  assert.equal(staleSearchBatchAgeDays("2026-11-25T00:00:00Z", now), null);
  assert.equal(staleSearchBatchAgeDays("2026-11-24T12:00:01Z", now), null);
  assert.equal(staleSearchBatchAgeDays("2026-11-24T12:00:00Z", now), 1);
  assert.equal(staleSearchBatchAgeDays("2026-11-16T12:00:00Z", now), 9);
  assert.equal(staleSearchBatchAgeLabel("2026-11-16T12:00:00Z", now), "9 天前");
});
