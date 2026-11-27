import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import {
  classifyStreamEvents,
  confirmSnapshotOf,
  buildSendContext,
  runStatusLabel,
  errorCancelledWording,
  channelReadinessFromViews,
} from "./agentStreamLogic.ts";
import type {
  AgentStreamEvent,
  AgentConfirmCard,
  AgentChannelView,
} from "../../lib/generated/v3-contracts.ts";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

test("W4 Slice 10 Subtask 2: Agent Stream Logic Suite", async (t) => {
  // -------------------------------------------------------------
  // D2-1: classifyStreamEvents 混合事件序列分类与归并
  // -------------------------------------------------------------
  await t.test(
    "D2-1: classifyStreamEvents groups tool_round and preserves item sequence and status",
    () => {
      const events: AgentStreamEvent[] = [
        {
          seq: 1,
          kind: "message",
          payload: {
            role: "user",
            run_id: "run-1",
            text: "检索大同区域 Sentinel-1",
            context: null,
          },
        },
        {
          seq: 2,
          kind: "tool_round",
          payload: {
            run_id: "run-1",
            call_id: "call-1",
            tool_id: "resolve_aoi",
            status: "running",
          },
        },
        {
          seq: 3,
          kind: "tool_round",
          payload: {
            run_id: "run-1",
            call_id: "call-1",
            tool_id: "resolve_aoi",
            status: "done",
          },
        },
        {
          seq: 4,
          kind: "tool_round",
          payload: {
            run_id: "run-1",
            call_id: "call-2",
            tool_id: "application_sar_search_and_freeze",
            status: "done",
          },
        },
        {
          seq: 5,
          kind: "plan_card",
          payload: {
            run_id: "run-1",
            plan_id: "plan-1",
            plan_hash: "hash-1",
            max_tier: 2,
            steps: [
              { tool_id: "resolve_aoi", label_zh: "解析空间范围", tier: 0, status: "done" },
              { tool_id: "application_create_download_task", label_zh: "创建下载任务", tier: 2, status: "pending" },
            ],
          },
        },
        {
          seq: 6,
          kind: "confirm_card",
          payload: {
            run_id: "run-1",
            plan_id: "plan-1",
            plan_hash: "hash-1",
            snapshot: {
              source: "Sentinel-1 (ASF)",
              range: {
                aoi_name: "大同盆地",
                area_km2: 1200.5,
                time_range: { start: "2024-04-01", end: "2024-04-30" },
                product: "SLC",
              },
              estimated_bytes: 5368709120, // 5 GB
              output_dir: "/data/sar",
            },
          },
        },
        {
          seq: 7,
          kind: "progress_card",
          payload: {
            run_id: "run-1",
            plan_id: "plan-1",
            step_index: 0,
            tool_id: "resolve_aoi",
            status: "done",
            code: "plan_approved",
          },
        },
        {
          seq: 8,
          kind: "conclusion",
          payload: {
            run_id: "run-1",
            text: "已完成批次建立与任务创建",
            claims: [
              {
                statement: "已创建下载任务",
                verified: true,
                refs: [{ kind: "task", id: "task-001", label: "任务 #task-001" }],
              },
            ],
          },
        },
        {
          seq: 9,
          kind: "error",
          payload: {
            run_id: "run-1",
            code: "cancelled",
            status: "cancelled",
            plan_id: null,
          },
        },
      ];

      const items = classifyStreamEvents(events);
      assert.equal(items.length, 7, "2条连续且同run的tool_round应归并为1个工具执行组，总共7项");

      assert.equal(items[0].kind, "message");
      assert.equal(items[0].seq, 1);
      if (items[0].kind === "message") {
        assert.equal(items[0].role, "user");
        assert.equal(items[0].text, "检索大同区域 Sentinel-1");
      }

      assert.equal(items[1].kind, "tool_group");
      if (items[1].kind === "tool_group") {
        assert.equal(items[1].run_id, "run-1");
        assert.equal(items[1].status, "done");
        assert.equal(items[1].rounds.length, 2, "call-1 更新状态后只保留最新一条，call-2 一条，共2条调用");
      }

      assert.equal(items[2].kind, "plan_card");
      assert.equal(items[3].kind, "confirm_card");
      assert.equal(items[4].kind, "progress_card");
      assert.equal(items[5].kind, "conclusion");
      assert.equal(items[6].kind, "error");
      if (items[6].kind === "error") {
        assert.equal(items[6].code, "cancelled");
        assert.equal(items[6].status, "cancelled");
      }
    }
  );

  // -------------------------------------------------------------
  // D2-2: 空/畸形 payload 不抛错，未知字段显示未知
  // -------------------------------------------------------------
  await t.test("D2-2: empty or malformed payload does not throw and displays unknown", () => {
    const malformedEvents: AgentStreamEvent[] = [
      { seq: 10, kind: "confirm_card", payload: {} },
      { seq: 11, kind: "message", payload: null as any },
      { seq: 12, kind: "error", payload: { run_id: "run-2" } as any },
    ];

    assert.doesNotThrow(() => {
      const items = classifyStreamEvents(malformedEvents);
      assert.equal(items.length, 3);
    });

    const snapshot = confirmSnapshotOf({
      seq: 1,
      kind: "confirm_card",
      payload: {},
    });
    assert.equal(snapshot.source, "未知");
    assert.equal(snapshot.range.aoi_name, "未知");
    assert.equal(snapshot.range.time_range_zh, "未知");
    assert.equal(snapshot.range.product, "未知");
    assert.equal(snapshot.estimated_bytes_zh, "未知");
    assert.equal(snapshot.output_dir, "未知");
  });

  // -------------------------------------------------------------
  // D3-1: confirmSnapshotOf 提取四项，估算语义，null → 未知
  // -------------------------------------------------------------
  await t.test("D3-1: confirmSnapshotOf extracts four items with estimation semantics and null as unknown", () => {
    const validCard: AgentConfirmCard = {
      run_id: "run-1",
      plan_id: "plan-1",
      plan_hash: "hash-123",
      snapshot: {
        source: "Sentinel-1 (ASF)",
        range: {
          aoi_name: "大同盆地试验区",
          area_km2: 1240.5,
          time_range: { start: "2024-04-01", end: "2024-04-30" },
          product: "SLC · 升轨",
        },
        estimated_bytes: 4 * 1024 * 1024 * 1024, // 4 GB
        output_dir: "/data/sar/downloads",
      },
    };

    const snapshot = confirmSnapshotOf({
      seq: 20,
      kind: "confirm_card",
      payload: validCard as any,
    });

    assert.equal(snapshot.source, "Sentinel-1 (ASF)");
    assert.equal(snapshot.range.aoi_name, "大同盆地试验区");
    assert.equal(snapshot.range.time_range_zh, "2024-04-01 ~ 2024-04-30");
    assert.equal(snapshot.range.product, "SLC · 升轨");
    assert.ok(snapshot.estimated_bytes_zh.includes("估算"), "预计大小必须带显式「估算」语义");
    assert.equal(snapshot.output_dir, "/data/sar/downloads");

    // null estimated_bytes -> 未知
    const nullBytesCard: AgentConfirmCard = {
      ...validCard,
      snapshot: {
        ...validCard.snapshot,
        estimated_bytes: null,
      },
    };
    const nullSnapshot = confirmSnapshotOf({
      seq: 21,
      kind: "confirm_card",
      payload: nullBytesCard as any,
    });
    assert.equal(nullSnapshot.estimated_bytes_zh, "未知");
  });

  // -------------------------------------------------------------
  // D3-2: 批准回传：原样回传 snapshot 对象
  // -------------------------------------------------------------
  await t.test("D3-2: planApprove receives exact original snapshot object without mutation or key reordering", async () => {
    const rawConfirmCardPayload = {
      run_id: "run-3",
      plan_id: "plan-approve-target",
      plan_hash: "phash-abc",
      snapshot: {
        source: "ALOS-2 PALSAR-2",
        range: {
          aoi_name: "青藏高原局部",
          area_km2: 560.2,
          time_range: { start: "2023-01-01", end: "2023-12-31" },
          product: "L1.1",
        },
        estimated_bytes: 1073741824,
        output_dir: "/archive/sar",
      },
    };

    let capturedPlanId = "";
    let capturedSnapshot: unknown = null;

    const mockBridge = {
      async planApprove(planId: string, snapshot: unknown): Promise<void> {
        capturedPlanId = planId;
        capturedSnapshot = snapshot;
      },
    };

    const event: AgentStreamEvent = {
      seq: 30,
      kind: "confirm_card",
      payload: rawConfirmCardPayload,
    };

    const extracted = confirmSnapshotOf(event);
    await mockBridge.planApprove(rawConfirmCardPayload.plan_id, extracted.raw_snapshot);

    assert.equal(capturedPlanId, "plan-approve-target");
    assert.equal(capturedSnapshot, rawConfirmCardPayload.snapshot, "必须直接传递原对象的引用或深相等");
    assert.deepEqual(capturedSnapshot, rawConfirmCardPayload.snapshot);
  });

  // -------------------------------------------------------------
  // D4-1: runStatusLabel 与 errorCancelledWording 互斥与文案
  // -------------------------------------------------------------
  await t.test("D4-1: runStatusLabel and errorCancelledWording map correctly and keep cancelled vs interrupted distinct", () => {
    assert.equal(runStatusLabel("running"), "执行中");
    assert.equal(runStatusLabel("completed"), "已完成");
    assert.equal(runStatusLabel("failed"), "已失败");
    assert.equal(runStatusLabel("cancelled"), "已取消");
    assert.equal(runStatusLabel("interrupted"), "已中断");

    assert.equal(errorCancelledWording("cancelled"), "已取消");
    assert.equal(errorCancelledWording("interrupted"), "已中断");
    assert.notEqual(errorCancelledWording("cancelled"), errorCancelledWording("interrupted"));
  });

  // -------------------------------------------------------------
  // D-extra-1: channelReadinessFromViews 就绪判定
  // -------------------------------------------------------------
  await t.test("D-extra-1: channelReadinessFromViews readiness logic", () => {
    // 1. is_default + key_ready -> ready
    const views1: AgentChannelView[] = [
      {
        id: "c1",
        protocol: "chat_completions",
        base_url: "https://api.openai.com/v1",
        model: "gpt-4o",
        key_ready: true,
        is_default: true,
      },
    ];
    const r1 = channelReadinessFromViews(views1);
    assert.equal(r1.ready, true);
    assert.equal(r1.disabled, false);

    // 2. 无 default 但有 key_ready -> ready
    const views2: AgentChannelView[] = [
      {
        id: "c2",
        protocol: "responses",
        base_url: "https://api.example.com",
        model: "model-a",
        key_ready: true,
        is_default: false,
      },
    ];
    const r2 = channelReadinessFromViews(views2);
    assert.equal(r2.ready, true);
    assert.equal(r2.disabled, false);

    // 3. default 无 key -> 不 ready
    const views3: AgentChannelView[] = [
      {
        id: "c3",
        protocol: "chat_completions",
        base_url: "https://api.openai.com/v1",
        model: "gpt-4o",
        key_ready: false,
        is_default: true,
      },
    ];
    const r3 = channelReadinessFromViews(views3);
    assert.equal(r3.ready, false);
    assert.equal(r3.disabled, true);
    assert.ok(r3.reason_zh.includes("模型渠道"), "未就绪提示必须含「模型渠道」");

    // 4. 空列表 -> 不 ready 且提示含「模型渠道」
    const r4 = channelReadinessFromViews([]);
    assert.equal(r4.ready, false);
    assert.equal(r4.disabled, true);
    assert.ok(r4.reason_zh.includes("模型渠道"));
  });

  // -------------------------------------------------------------
  // D-extra-2: buildSendContext 键集恰为 aoi/time_range/source
  // -------------------------------------------------------------
  await t.test("D-extra-2: buildSendContext has exact keys and wraps aoi with name and geometry", () => {
    const ctx = buildSendContext(
      {
        label: "大同盆地",
        geometry: { type: "Polygon", coordinates: [[[0, 0], [1, 0], [1, 1], [0, 0]]] },
      },
      { start: "2024-01-01", end: "2024-06-01" },
      "asf_sentinel1"
    );

    assert.deepEqual(Object.keys(ctx).sort(), ["aoi", "source", "time_range"].sort());
    assert.deepEqual(ctx.aoi, {
      name: "大同盆地",
      geometry: { type: "Polygon", coordinates: [[[0, 0], [1, 0], [1, 1], [0, 0]]] },
    });
    assert.deepEqual(ctx.time_range, { start: "2024-01-01", end: "2024-06-01" });
    assert.equal(ctx.source, "asf_sentinel1");

    // null input -> 各字段为 null
    const nullCtx = buildSendContext(null, null, null);
    assert.deepEqual(Object.keys(nullCtx).sort(), ["aoi", "source", "time_range"].sort());
    assert.equal(nullCtx.aoi, null);
    assert.equal(nullCtx.time_range, null);
    assert.equal(nullCtx.source, null);
  });

  // -------------------------------------------------------------
  // D-extra-3: 源码检查：DCE 门控、无「修订版本」、fixture 标记与唯一引用面
  // -------------------------------------------------------------
  await t.test("D-extra-3: DesktopAgentWorkbench has no '修订版本', gates fixture with import.meta.env.DEV, and agentWorkbenchFixture is isolated", () => {
    const workbenchPath = path.resolve(__dirname, "../../components/desktop/DesktopAgentWorkbench.tsx");
    const workbenchSrc = fs.readFileSync(workbenchPath, "utf-8");
    assert.ok(!workbenchSrc.includes("修订版本"), "DesktopAgentWorkbench 源码严禁出现「修订版本」");

    // 门控检查：必须经过 import.meta.env.DEV 门控常量折叠引用
    assert.ok(
      workbenchSrc.includes("import.meta.env.DEV ? createFixtureEvents__fixture : null"),
      "DesktopAgentWorkbench 必须使用 import.meta.env.DEV 门控引用 createFixtureEvents__fixture"
    );
    assert.ok(
      !workbenchSrc.includes("fixture-run-01"),
      "DesktopAgentWorkbench 源码不应直接硬编码 fixture-run-01"
    );
    assert.ok(
      !workbenchSrc.includes("fixture-sess-01"),
      "DesktopAgentWorkbench 源码不应直接硬编码 fixture-sess-01"
    );

    // fixture 文件检查
    const fixturePath = path.resolve(__dirname, "agentWorkbenchFixture.ts");
    const fixtureSrc = fs.readFileSync(fixturePath, "utf-8");
    assert.ok(fixtureSrc.includes("__fixture"), "agentWorkbenchFixture.ts 必须带 __fixture 标记");
    assert.ok(fixtureSrc.includes("createFixtureEvents__fixture"), "必须导出带 __fixture 后缀的工厂函数");
    assert.ok(fixtureSrc.includes("fixture-run-01"), "演示数据应集中于 agentWorkbenchFixture.ts 中");

    // 引用面检查：agentWorkbenchFixture 只被 DesktopAgentWorkbench.tsx 引用
    const srcDir = path.resolve(__dirname, "../..");
    const allFiles: string[] = [];
    function walk(dir: string) {
      for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
        const full = path.join(dir, entry.name);
        if (entry.isDirectory()) {
          walk(full);
        } else if (entry.isFile() && (entry.name.endsWith(".ts") || entry.name.endsWith(".tsx"))) {
          allFiles.push(full);
        }
      }
    }
    walk(srcDir);

    const referencingFiles = allFiles.filter((filePath) => {
      if (filePath === fixturePath) return false;
      if (filePath === __filename) return false; // 排除当前测试文件
      const content = fs.readFileSync(filePath, "utf-8");
      return content.includes("agentWorkbenchFixture");
    });

    assert.equal(
      referencingFiles.length,
      1,
      `agentWorkbenchFixture 只能且仅被 DesktopAgentWorkbench.tsx 引用，实际引用者: ${referencingFiles.map((f) => path.basename(f)).join(", ")}`
    );
    assert.equal(path.basename(referencingFiles[0]), "DesktopAgentWorkbench.tsx");
  });
});
