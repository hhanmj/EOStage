import test from "node:test";
import assert from "node:assert/strict";

import {
  projectToolGroup,
  projectConfirmCard,
  projectEvidence,
  taskStatusLabelZh,
  buildTaskCardModels,
  projectSearchCard,
} from "./agentConversationLogic.ts";
import type {
  StreamToolGroupItem,
  StreamConfirmCardItem,
  StreamProgressCardItem,
  StreamConclusionItem,
} from "./agentStreamLogic.ts";
import type { ApplicationTask } from "./types.ts";

test("W5-2b Agent Conversation Logic Specification", async (t) => {
  // ==========================================================================
  // 1. projectToolGroup: tool round mapping and name resolution
  // ==========================================================================
  await t.test("1. projectToolGroup: resolves tool display names, omits durations", () => {
    const toolGroupItem: StreamToolGroupItem = {
      kind: "tool_group",
      seq: 2,
      run_id: "run-001",
      status: "done",
      rounds: [
        { run_id: "run-001", call_id: "call-1", tool_id: "resolve_aoi", status: "done" },
        { run_id: "run-001", call_id: "call-2", tool_id: "application_sar_search_and_freeze", status: "done" },
      ],
    };

    const resolver = (id: string) => {
      if (id === "resolve_aoi") return "解析空间范围与投影";
      if (id === "application_sar_search_and_freeze") return "检索并冻结批次";
      return id;
    };

    const projected = projectToolGroup(toolGroupItem, resolver);
    assert.strictEqual(projected.count, 2);
    assert.strictEqual(projected.status, "done");
    assert.deepStrictEqual(projected.rows, [
      {
        name_zh: "解析空间范围与投影",
        status: "done",
        timeZh: null,
        durationZh: null,
        resultCountZh: null,
        summaryZh: null,
      },
      {
        name_zh: "检索并冻结批次",
        status: "done",
        timeZh: null,
        durationZh: null,
        resultCountZh: null,
        summaryZh: null,
      },
    ]);
  });

  // ==========================================================================
  // 2. projectConfirmCard: facts extraction, condition badges, state transitions
  // ==========================================================================
  await t.test("2. projectConfirmCard: facts extraction and honest field omissions", () => {
    const confirmItem: StreamConfirmCardItem = {
      kind: "confirm_card",
      seq: 4,
      run_id: "run-001",
      plan_id: "plan-s1-001",
      plan_hash: "hash-001",
      snapshot: {
        source: "Sentinel-1 SLC (ASF)",
        range: {
          aoi_name: "茂县叠溪",
          area_km2: 432.4,
          time_range: { start: "2026-09-20", end: "2026-10-22" },
          product: "IW SLC",
        },
        estimated_bytes: 31138512896,
        output_dir: "/data/sar/downloads/sentinel1",
      },
    };

    // 无后继事件 -> 待确认 (pending)
    const pendingCard = projectConfirmCard(confirmItem, null);
    assert.strictEqual(pendingCard.source, "Sentinel-1 SLC (ASF)");
    assert.deepStrictEqual(pendingCard.conditionBadges, [
      "IW SLC",
      "2026-09-20 至 2026-10-22",
    ]);
    assert.strictEqual(pendingCard.aoiLine, "茂县叠溪（约 432 km²）");
    assert.strictEqual(pendingCard.state, "pending");
    assert.strictEqual(pendingCard.outputDir, "/data/sar/downloads/sentinel1");
    assert.strictEqual(pendingCard.estimatedSizeZh.includes("GB"), true);

    // 后继 plan_approved -> approved
    const approvedProgress: StreamProgressCardItem = {
      kind: "progress_card",
      seq: 5,
      run_id: "run-001",
      plan_id: "plan-s1-001",
      step_index: 2,
      tool_id: "application_create_download_task",
      status: "done",
      code: "plan_approved",
    };
    const approvedCard = projectConfirmCard(confirmItem, approvedProgress);
    assert.strictEqual(approvedCard.state, "approved");

    // 后继 plan_rejected -> rejected
    const rejectedProgress: StreamProgressCardItem = {
      ...approvedProgress,
      code: "plan_rejected",
    };
    const rejectedCard = projectConfirmCard(confirmItem, rejectedProgress);
    assert.strictEqual(rejectedCard.state, "rejected");

    // 后继 plan_expired -> expired
    const expiredProgress: StreamProgressCardItem = {
      ...approvedProgress,
      code: "plan_expired",
    };
    const expiredCard = projectConfirmCard(confirmItem, expiredProgress);
    assert.strictEqual(expiredCard.state, "expired");

    // 缺少可选字段时诚实省略，不编造虚假值
    const minimalItem: StreamConfirmCardItem = {
      kind: "confirm_card",
      seq: 6,
      run_id: "run-002",
      plan_id: "plan-s1-002",
      plan_hash: "hash-002",
      snapshot: {
        source: "ASF",
        range: {
          aoi_name: "自定义区域",
          area_km2: null,
          time_range: null,
          product: "",
        },
        estimated_bytes: null,
        output_dir: "",
      },
    };
    const minimalCard = projectConfirmCard(minimalItem, null);
    assert.strictEqual(minimalCard.aoiLine, "自定义区域");
    assert.deepStrictEqual(minimalCard.conditionBadges, []);
    assert.strictEqual(minimalCard.outputDir, null);
  });

  // ==========================================================================
  // 3. projectEvidence: conclusion citations and verification aggregation
  // ==========================================================================
  await t.test("3. projectEvidence: aggregates references without fabricated step/response counters", () => {
    const conclusionItem: StreamConclusionItem = {
      kind: "conclusion",
      seq: 7,
      run_id: "run-20261023-0912",
      text: "覆盖 AOI 的有 3 条轨道，12 天重访，期间没有缺景。",
      claims: [
        {
          statement: "已完成数据批次检索与冻结",
          verified: true,
          refs: [
            { kind: "batch", id: "batch-s1-01", label: "批次 #batch-s1-01" },
          ],
        },
        {
          statement: "已建立下载任务",
          verified: true,
          refs: [
            { kind: "task", id: "mock-task-c1-running-1", label: "任务 #task-01" },
            { kind: "web", id: "web-ref-1", label: "ASF 数据源文档" },
          ],
        },
      ],
    };

    const evidence = projectEvidence(conclusionItem);
    assert.strictEqual(evidence.runId, "run-20261023-0912");
    assert.strictEqual(evidence.verified, true);
    assert.strictEqual(evidence.refCount, 3);
    assert.deepStrictEqual(evidence.refs, [
      { kind: "batch", kind_zh: "关联批次", id: "batch-s1-01", label: "批次 #batch-s1-01", verified: true, traceZh: null, responseZh: null },
      { kind: "task", kind_zh: "关联任务", id: "mock-task-c1-running-1", label: "任务 #task-01", verified: true, traceZh: null, responseZh: null },
      { kind: "web", kind_zh: "网络来源", id: "web-ref-1", label: "ASF 数据源文档", verified: true, traceZh: null, responseZh: null },
    ]);

    // 部分 claim 未核验 -> 全局 verified 为 false
    const unverifiedConclusion: StreamConclusionItem = {
      ...conclusionItem,
      claims: [
        { statement: "已核验结论", verified: true, refs: [] },
        { statement: "未核验结论", verified: false, refs: [] },
      ],
    };
    const unverifiedEvidence = projectEvidence(unverifiedConclusion);
    assert.strictEqual(unverifiedEvidence.verified, false);
  });

  // ==========================================================================
  // 4. taskStatusLabelZh: strict status Chinese mapping
  // ==========================================================================
  await t.test("4. taskStatusLabelZh: strict status mapping without implementation terms", () => {
    assert.strictEqual(taskStatusLabelZh("queued"), "排队中");
    assert.strictEqual(taskStatusLabelZh("running"), "传输中");
    assert.strictEqual(taskStatusLabelZh("paused"), "已暂停");
    assert.strictEqual(taskStatusLabelZh("completed"), "已完成");
    assert.strictEqual(taskStatusLabelZh("failed"), "已失败");
    assert.strictEqual(taskStatusLabelZh("cancelled"), "已取消");
    assert.strictEqual(taskStatusLabelZh("interrupted"), "已中断");
  });

  // ==========================================================================
  // 5. buildTaskCardModels: task projection join and missing task omission
  // ==========================================================================
  await t.test("5. buildTaskCardModels: joins matching tasks, omits missing refs, derives speeds", () => {
    const refs = [
      { id: "mock-task-1", label: "任务 #1" },
      { id: "missing-task-ref", label: "不存在的任务" },
      { id: "mock-task-2", label: "任务 #2" },
    ];

    const tasks: ApplicationTask[] = [
      {
        task_id: "mock-task-1",
        revision: 1,
        name: "下载 1 景 Sentinel-1 SLC",
        status: "running",
        scene_ids: ["S1A_IW_SLC__1SDV_20261022T103247_20261022T103314_066758_0849A2_3F1C"],
        done_bytes: 2040109465, // ~1.9 GB
        total_bytes: 4402341478, // ~4.1 GB
        bytes_per_second: 6553600, // ~6.25 MB/s -> remaining ~2.36 GB / ~6.25 MB/s = ~377s = ~6min
        output_dir: "/data/sar",
        created_at: "2026-10-25T10:00:00Z",
      } as ApplicationTask,
      {
        task_id: "mock-task-2",
        revision: 2,
        name: "下载 Sentinel-1 GRD 批次",
        status: "paused",
        scene_ids: [],
        done_bytes: 104857600,
        total_bytes: 209715200,
        bytes_per_second: 0, // 暂停无速率
        output_dir: "/data/sar",
        created_at: "2026-10-25T09:00:00Z",
      } as ApplicationTask,
    ];

    const models = buildTaskCardModels(refs, tasks);

    // missing-task-ref 绝不渲染任务卡，仅生成 2 张卡片
    assert.strictEqual(models.length, 2);

    // 任务 1: 传输中，有剩余时间预估，操作包含 pause/cancel
    const model1 = models[0];
    assert.strictEqual(model1.taskId, "mock-task-1");
    assert.strictEqual(model1.task.name, "下载 1 景 Sentinel-1 SLC");
    assert.strictEqual(model1.statusLabel, "传输中");
    assert.strictEqual(model1.percent, 46);
    assert.strictEqual(model1.remainingMinutesZh, "剩余约 6 分钟");
    // W5-10 R4: 带保存目录的任务映射保留打开目录能力。
    assert.deepStrictEqual(model1.actions, ["pause", "cancel", "open_folder"]);
    assert.strictEqual(model1.sizeLine.includes("1.9 GB / 4.1 GB"), true);

    // 任务 2: 暂停中，无速率 -> remainingMinutesZh 为 null，操作包含 resume/cancel
    const model2 = models[1];
    assert.strictEqual(model2.taskId, "mock-task-2");
    assert.strictEqual(model2.statusLabel, "已暂停");
    assert.strictEqual(model2.percent, 50);
    assert.strictEqual(model2.remainingMinutesZh, null);
    assert.deepStrictEqual(model2.actions, ["resume", "cancel", "open_folder"]);
  });

  // ==========================================================================
  // 6. projectSearchCard: maps search card facts from tool group and snapshot
  // ==========================================================================
  await t.test("6. projectSearchCard: status and snapshot projection, null handling", () => {
    const searchToolGroup: StreamToolGroupItem = {
      kind: "tool_group",
      seq: 3,
      run_id: "run-001",
      status: "done",
      rounds: [
        { run_id: "run-001", call_id: "c1", tool_id: "resolve_aoi", status: "done" },
        { run_id: "run-001", call_id: "c2", tool_id: "application_sar_search_and_freeze", status: "done" },
      ],
    };

    const snapshot = {
      source: "Sentinel-1 (ASF)",
      conditionBadges: ["IW SLC", "2026-09-20 至 2026-10-22"],
    };

    // 1) 带有 snapshot 的正常投射
    const projectedWithSnapshot = projectSearchCard(searchToolGroup, snapshot);
    assert.strictEqual(projectedWithSnapshot.status, "done");
    assert.strictEqual(projectedWithSnapshot.source, "Sentinel-1 (ASF)");
    assert.deepStrictEqual(projectedWithSnapshot.conditionBadges, [
      "IW SLC",
      "2026-09-20 至 2026-10-22",
    ]);

    // 2) snapshot 为 null 时的投射（字段为 null，不捏造来源和条件）
    const projectedWithoutSnapshot = projectSearchCard(searchToolGroup, null);
    assert.strictEqual(projectedWithoutSnapshot.status, "done");
    assert.strictEqual(projectedWithoutSnapshot.source, null);
    assert.deepStrictEqual(projectedWithoutSnapshot.conditionBadges, []);

    // 3) status 透传（running / failed）
    const runningGroup: StreamToolGroupItem = {
      ...searchToolGroup,
      status: "running",
    };
    const projectedRunning = projectSearchCard(runningGroup, snapshot);
    assert.strictEqual(projectedRunning.status, "running");
  });
});
