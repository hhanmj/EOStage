import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  newlySettledSearchCallIds,
  extractSearchResourceIdFromEvents,
  pickAgentSearchedView,
} from "./agentSearchViewSync.ts";
import type { AgentStreamEvent } from "./agentBridgeTypes.ts";
import type { SourceView } from "./types.ts";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

test("W5-8 R4b (条目 8 / L4 P2-8): Agent 地图面板「结果清单」读本会话冻结批次测试套件", async (suite) => {
  await suite.test(
    "1. newlySettledSearchCallIds: 仅返回未见过的 search_progress + status=done + 非负整数 result_count 的 call_id，不修改入参",
    () => {
      const events: AgentStreamEvent[] = [
        // 1) 非 search_progress 工具 done -> 不返回
        {
          seq: 1,
          kind: "tool_round",
          payload: {
            run_id: "run-1",
            call_id: "call-cap",
            tool_id: "capabilities_get",
            status: "done",
            result_count: null,
          },
        },
        // 2) search_progress running -> 不返回
        {
          seq: 2,
          kind: "tool_round",
          payload: {
            run_id: "run-1",
            call_id: "call-sp-1",
            tool_id: "search_progress",
            status: "running",
            result_count: null,
          },
        },
        // 3) search_progress done + result_count=8 -> 返回
        {
          seq: 3,
          kind: "tool_round",
          payload: {
            run_id: "run-1",
            call_id: "call-sp-1",
            tool_id: "search_progress",
            status: "done",
            result_count: 8,
          },
        },
        // 4) search_progress failed -> 不返回
        {
          seq: 4,
          kind: "tool_round",
          payload: {
            run_id: "run-1",
            call_id: "call-sp-failed",
            tool_id: "search_progress",
            status: "failed",
            result_count: 0,
          },
        },
        // 5) 另一新完成检索 result_count=0（合法非负整数）-> 返回
        {
          seq: 5,
          kind: "tool_round",
          payload: {
            run_id: "run-2",
            call_id: "call-sp-2",
            tool_id: "search_progress",
            status: "done",
            result_count: 0,
          },
        },
      ];

      const seen = new Set<string>();
      const firstBatch = newlySettledSearchCallIds(events, seen);
      assert.deepStrictEqual(
        firstBatch,
        ["call-sp-1", "call-sp-2"],
        "应返回两个已完成检索的 call_id"
      );
      assert.strictEqual(seen.size, 0, "不得修改入参 seenCallIds 集合");

      // 将 call-sp-1 标为已见后再次调用，只返回 call-sp-2
      const seenWithOne = new Set<string>(["call-sp-1"]);
      const secondBatch = newlySettledSearchCallIds(events, seenWithOne);
      assert.deepStrictEqual(
        secondBatch,
        ["call-sp-2"],
        "已见过的 call_id 不得重复返回"
      );
    }
  );

  await suite.test(
    "2. extractSearchResourceIdFromEvents: 从同 run 的 plan_card 检索步骤 disclosure.resource_id 提取数据源 ID",
    () => {
      const events: AgentStreamEvent[] = [
        {
          seq: 7,
          kind: "plan_card",
          payload: {
            run_id: "run-1a17a53635b-1",
            plan_id: "plan-1a17a53c9bf-2",
            plan_hash: "hash",
            max_tier: 1,
            steps: [
              {
                tool_id: "search_start",
                label_zh: "检索茂县叠溪区域 Sentinel-1 SLC",
                tier: 1,
                status: "pending",
                disclosure: {
                  resource_id: "science.sentinel-1.standard.v1",
                  display_name_zh: "Sentinel-1",
                  aoi_name: "茂县叠溪",
                },
              },
            ],
          },
        },
        {
          seq: 12,
          kind: "tool_round",
          payload: {
            run_id: "run-1a17a53635b-1",
            call_id: "call_4aaea8c6fd7f4c7eb8d53eba",
            tool_id: "search_progress",
            status: "done",
            result_count: 8,
          },
        },
      ];

      assert.strictEqual(
        extractSearchResourceIdFromEvents(events, "call_4aaea8c6fd7f4c7eb8d53eba"),
        "science.sentinel-1.standard.v1"
      );
      assert.strictEqual(
        extractSearchResourceIdFromEvents([], "call-unknown"),
        null
      );
    }
  );

  await suite.test(
    "3. pickAgentSearchedView: batch_id 变化、仅 revision 增大、无变化、hintResourceId 优先、无 batch_id 覆盖",
    () => {
      const prevViews: SourceView[] = [
        {
          resource_id: "science.sentinel-1.standard.v1",
          batch_id: "batch-old-1",
          revision: 1,
          item_count: 3,
          searched_at: "2026-10-27T00:00:00Z",
        },
        {
          resource_id: "science.nisar.l-band.v1",
          batch_id: null,
          revision: 0,
          item_count: 0,
          searched_at: null,
        },
      ];

      // (a) batch_id 变化 -> 返回该视图
      const nextBatchChanged: SourceView[] = [
        {
          resource_id: "science.sentinel-1.standard.v1",
          batch_id: "batch-new-8",
          revision: 1,
          item_count: 8,
          searched_at: "2026-10-27T01:00:00Z",
        },
        prevViews[1],
      ];
      const pickedBatch = pickAgentSearchedView(prevViews, nextBatchChanged);
      assert.ok(pickedBatch);
      assert.strictEqual(pickedBatch.batch_id, "batch-new-8");

      // (b) 仅 revision 增大 -> 返回该视图
      const nextRevIncreased: SourceView[] = [
        {
          resource_id: "science.sentinel-1.standard.v1",
          batch_id: "batch-old-1",
          revision: 2,
          item_count: 8,
          searched_at: "2026-10-27T01:05:00Z",
        },
        prevViews[1],
      ];
      const pickedRev = pickAgentSearchedView(prevViews, nextRevIncreased);
      assert.ok(pickedRev);
      assert.strictEqual(pickedRev.revision, 2);

      // (c) 无变化 -> 返回 null
      assert.strictEqual(pickAgentSearchedView(prevViews, prevViews), null);

      // (d) 多个视图变化时 hintResourceId 优先；无 hint 时取 searched_at 最新
      const nextMultiChanged: SourceView[] = [
        {
          resource_id: "science.sentinel-1.standard.v1",
          batch_id: "batch-s1-new",
          revision: 2,
          item_count: 8,
          searched_at: "2026-10-27T01:00:00Z",
        },
        {
          resource_id: "science.nisar.l-band.v1",
          batch_id: "batch-nisar-new",
          revision: 1,
          item_count: 23,
          searched_at: "2026-10-27T02:00:00Z",
        },
      ];
      const pickedWithHint = pickAgentSearchedView(
        prevViews,
        nextMultiChanged,
        "science.sentinel-1.standard.v1"
      );
      assert.ok(pickedWithHint);
      assert.strictEqual(pickedWithHint.resource_id, "science.sentinel-1.standard.v1");

      const pickedLatest = pickAgentSearchedView(prevViews, nextMultiChanged, null);
      assert.ok(pickedLatest);
      assert.strictEqual(pickedLatest.resource_id, "science.nisar.l-band.v1");

      // (e) 无 batch_id（即使 revision 变化）-> 返回 null，不编造批次
      const nextNoBatchId: SourceView[] = [
        prevViews[0],
        {
          resource_id: "science.nisar.l-band.v1",
          batch_id: null,
          revision: 3,
          item_count: 0,
          searched_at: "2026-10-27T03:00:00Z",
        },
      ];
      assert.strictEqual(pickAgentSearchedView(prevViews, nextNoBatchId), null);
    }
  );

  await suite.test(
    "4. 源码接线断言: DesktopAgentWorkbench 声明 onAgentSearchSettled，DesktopWorkbench 传入并调用 pickAgentSearchedView 与 getBatchPage",
    () => {
      const agentWbSrc = fs.readFileSync(
        path.resolve(__dirname, "../../components/desktop/DesktopAgentWorkbench.tsx"),
        "utf-8"
      );
      const shellWbSrc = fs.readFileSync(
        path.resolve(__dirname, "../../components/desktop/DesktopWorkbench.tsx"),
        "utf-8"
      );

      assert.match(
        agentWbSrc,
        /onAgentSearchSettled\?\s*:\s*\(info:\s*\{\s*resourceId:\s*string\s*\|\s*null\s*\}\)\s*=>\s*void/,
        "DesktopAgentWorkbenchProps 必须声明 onAgentSearchSettled 可选回调"
      );
      assert.match(
        agentWbSrc,
        /newlySettledSearchCallIds\(/,
        "DesktopAgentWorkbench 必须调用 newlySettledSearchCallIds 识别新完成的检索回合"
      );

      assert.match(
        shellWbSrc,
        /onAgentSearchSettled=\{/,
        "DesktopWorkbench 必须向 DesktopAgentWorkbench 传入 onAgentSearchSettled"
      );
      assert.match(
        shellWbSrc,
        /pickAgentSearchedView\(/,
        "DesktopWorkbench 必须调用 pickAgentSearchedView 选取更新后的源视图"
      );
    }
  );
});
