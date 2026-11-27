import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { mapTaskOverviewToApplicationTasks } from "./production.ts";
import { taskCardSizeLine } from "./taskCardLogic.ts";
import { formatBytes, projectSnapshotItemToSceneRow } from "./rowProjection.ts";
import type {
  ApplicationTask,
  SnapshotItem,
  StoreTaskHistoryItem,
  StoreTaskSummary,
  TaskOverview,
} from "./types.ts";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

function summary(overrides: Partial<StoreTaskSummary> = {}): StoreTaskSummary {
  return {
    task_id: "task-q-1",
    batch_id: "batch-1",
    provider_id: "asf",
    dataset_id: "science.sentinel-1.standard.v1",
    concurrency: 2,
    task_type: null,
    created_at: "2026-10-26T22:31:00Z",
    status: "cancelled",
    revision: 3,
    updated_at: "2026-10-26T22:40:00Z",
    selected_count: 1,
    delivery_requested_count: null,
    delivery_rejected_count: null,
    checked_count: 1,
    active_count: 0,
    paused_count: 0,
    queued_count: 0,
    completed_count: 0,
    failed_count: 0,
    skipped_count: 0,
    cancelled_count: 1,
    terminal_bytes: 210_000_000,
    active_bytes: 0,
    other_bytes: 0,
    transferred_bytes: 210_000_000,
    total_bytes: null,
    source_total_bytes: null,
    ...overrides,
  };
}

// 归档历史摘要：后端 history_summary_from_sql 把字节字段写死为 0 / None（core_restore.rs）
function historyItem(overrides: Partial<StoreTaskHistoryItem> = {}): StoreTaskHistoryItem {
  return {
    ...summary({
      task_id: "task-h-1",
      status: "completed",
      terminal_bytes: 0,
      active_bytes: 0,
      other_bytes: 0,
      transferred_bytes: 0,
      total_bytes: null,
      source_total_bytes: null,
    }),
    archived_at: "2026-10-26T23:00:00Z",
    ...overrides,
  } as StoreTaskHistoryItem;
}

function overview(queue: StoreTaskSummary[], history: StoreTaskHistoryItem[]): TaskOverview {
  return {
    queue,
    history_page: { page_size: 20, has_more: false, next_cursor: null, items: history },
  };
}

const fmt = formatBytes;

test("W5-8 R6 条目 11：历史卡字节未上报", async (t) => {
  await t.test("history_page 条目标记 bytes_reported:false，queue 条目不带", () => {
    const tasks = mapTaskOverviewToApplicationTasks(
      overview([summary()], [historyItem()]),
    );
    const queued = tasks.find((x) => x.task_id === "task-q-1")!;
    const archived = tasks.find((x) => x.task_id === "task-h-1")!;
    assert.equal(
      (archived as { bytes_reported?: unknown }).bytes_reported,
      false,
      "归档历史的字节字段是后端未提供（写死 0），映射必须标记 bytes_reported:false",
    );
    assert.equal("bytes_reported" in queued, false, "队列条目不得带 bytes_reported");
  });

  await t.test("completed + bytes_reported:false → 「大小未知」而非「0 B」", () => {
    const tasks = mapTaskOverviewToApplicationTasks(overview([], [historyItem()]));
    const line = taskCardSizeLine(tasks[0], fmt);
    assert.equal(line, "大小未知", `归档的已完成任务不得显示「0 B」，实际：${line}`);
  });

  await t.test("cancelled + bytes_reported:false → 「大小未知」", () => {
    const tasks = mapTaskOverviewToApplicationTasks(
      overview([], [historyItem({ status: "cancelled" })]),
    );
    assert.equal(taskCardSizeLine(tasks[0], fmt), "大小未知");
  });

  await t.test("bytes_reported:false 但后端给出总量 → 显示总量，不显示已传输 0 B", () => {
    const task = {
      status: "completed",
      done_bytes: 0,
      total_bytes: 4_700_000_000,
      source_total_bytes: null,
      bytes_reported: false,
    } as Pick<ApplicationTask, "status" | "done_bytes" | "total_bytes" | "source_total_bytes">;
    const line = taskCardSizeLine(task, fmt);
    assert.ok(!/0 B/.test(line), `字节未上报时不得出现「0 B」，实际：${line}`);
    assert.equal(line, `总量 ${fmt(4_700_000_000)}`);
  });

  await t.test("队列 cancelled 且 done_bytes>0、无总量 → 「已传输 X · 总量未知」不变", () => {
    const tasks = mapTaskOverviewToApplicationTasks(overview([summary()], []));
    assert.equal(
      taskCardSizeLine(tasks[0], fmt),
      `已传输 ${fmt(210_000_000)} · 总量未知`,
    );
  });

  await t.test("getTasks 经 mapTaskOverviewToApplicationTasks 映射（源码断言）", () => {
    const src = fs.readFileSync(path.join(__dirname, "production.ts"), "utf-8");
    const body = src.slice(src.indexOf("export async function getTasks"));
    assert.ok(/mapTaskOverviewToApplicationTasks\(overview\)/.test(body.slice(0, 400)));
  });
});

function nisarItem(
  metadata: Record<string, unknown>,
  displayName = "NISAR_L1_PR_RSLC_001_077_A_012_2005_DHDH_A_20261020T010203_20261020T010233_P05000_N_F_J_001.h5",
): SnapshotItem {
  return {
    item_id: "G1234567890-ASF",
    provider_id: "asf.nisar",
    display_name: displayName,
    acquisition_datetime: "2026-10-20T01:02:03Z",
    footprint_geojson: null,
    metadata,
    payload: {},
  };
}

test("W5-8 R6 条目 12：NISAR 轨道方向/分幅读 metadata.properties", async (t) => {
  await t.test("顶层无字段、properties 有 flight_direction/relative_orbit/frame", () => {
    const row = projectSnapshotItemToSceneRow(
      nisarItem({
        dataset: "nisar",
        properties: { flight_direction: "ASCENDING", track: 77, relative_orbit: 77, frame: 12 },
      }),
    );
    assert.equal(row.orbit_direction, "ascending", "NISAR 轨道方向必须读后端 properties.flight_direction");
    assert.equal(row.relative_orbit, 77, "NISAR Relative orbit 必须读 properties.relative_orbit");
    assert.equal(row.frame, 12, "NISAR Frame 必须读 properties.frame");
  });

  await t.test("relative_orbit 缺失时用 track；DESCENDING → descending", () => {
    const row = projectSnapshotItemToSceneRow(
      nisarItem({ properties: { flight_direction: "DESCENDING", track: 150, frame: 33 } }),
    );
    assert.equal(row.orbit_direction, "descending");
    assert.equal(row.relative_orbit, 150);
    assert.equal(row.frame, 33);
  });

  await t.test("字符串整数接受，其他字符串忽略", () => {
    const row = projectSnapshotItemToSceneRow(
      nisarItem({ properties: { flight_direction: "ASCENDING", relative_orbit: "77", frame: "twelve" } }),
    );
    assert.equal(row.relative_orbit, 77);
    assert.equal(row.frame, null);
  });

  await t.test("顶层与 properties 同时存在时取顶层", () => {
    const row = projectSnapshotItemToSceneRow(
      nisarItem({
        flight_direction: "DESCENDING",
        relative_orbit: 5,
        frame: 6,
        properties: { flight_direction: "ASCENDING", relative_orbit: 77, frame: 12 },
      }),
    );
    assert.equal(row.orbit_direction, "descending");
    assert.equal(row.relative_orbit, 5);
    assert.equal(row.frame, 6);
  });

  await t.test("properties 缺失或不是对象 → 仍为空", () => {
    for (const metadata of [{}, { properties: "ASCENDING" }, { properties: ["ASCENDING"] }]) {
      const row = projectSnapshotItemToSceneRow(nisarItem(metadata));
      assert.equal(row.orbit_direction, "");
      assert.equal(row.relative_orbit, null);
      assert.equal(row.frame, null);
    }
  });

  await t.test("不从文件名推断：display_name 含 _A_ 但无属性 → 不得推出 ascending", () => {
    const row = projectSnapshotItemToSceneRow(nisarItem({}));
    assert.equal(row.orbit_direction, "");
    assert.equal(row.relative_orbit, null);
    assert.equal(row.frame, null);
  });
});

test("W5-8 R6 条目 12：新外壳地图弹窗用显示名且隐藏下载入口（源码断言）", () => {
  for (const rel of ["../../components/desktop/ManualWorkbench.tsx", "../../components/desktop/DesktopAgentWorkbench.tsx"]) {
    const src = fs.readFileSync(path.join(__dirname, rel), "utf-8");
    const start = src.indexOf("<WorkbenchMap");
    assert.ok(start >= 0, `${rel} 应渲染 WorkbenchMap`);
    const block = src.slice(start, src.indexOf("/>", start));
    assert.ok(/popupTitleMode="display_name"/.test(block), `${rel} 的 WorkbenchMap 必须传 popupTitleMode="display_name"`);
    assert.ok(/hideDownloadEntry\b/.test(block), `${rel} 的 WorkbenchMap 必须传 hideDownloadEntry`);
  }
});

test("Bhoonidhi S-SAR uppercase STAC properties project cleanly to SceneRow top-level fields", () => {
  const row = projectSnapshotItemToSceneRow(
    nisarItem({
      dataset: "sar.nisar.s.ssar.rslc.v1",
      properties: {
        Track: 26,
        Frame: 18,
        Node: "Ascending",
        Mode: "W",
        Polarization: "DH",
        Imaging_Orbit: 6944,
      },
    }),
  );
  assert.equal(row.relative_orbit, 26);
  assert.equal(row.path, 26);
  assert.equal(row.frame, 18);
  assert.equal(row.orbit_direction, "ascending");
  assert.equal(row.beam_mode, "W");
  assert.equal(row.polarization, "DH");
  assert.equal(row.absolute_orbit, 6944);
});
