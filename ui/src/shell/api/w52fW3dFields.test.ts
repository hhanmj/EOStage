import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { classifyStreamEvents } from "./agentStreamLogic.ts";
import type {
  StreamToolGroupItem,
  StreamConfirmCardItem,
  StreamConclusionItem,
  StreamThinkingItem,
  StreamQuestionCardItem,
  StreamWorkspaceSyncItem,
  StreamMessageItem,
} from "./agentStreamLogic.ts";
import { AGENT_TOOL_CATALOG, resolveAgentToolDisplayName } from "./agentLogic.ts";
import { REGISTERED_AGENT_TOOLS } from "../../lib/agentRegisteredTools.ts";
import { splitInlineThink, cleanCopyableMarkdown } from "./agentThinkSplitLogic.ts";
import { assertLocalizedSource } from "./i18nSourceTestUtils.test.ts";
import { MESSAGES } from "../../i18n/locales.ts";
import {
  projectToolGroup,
  projectSearchCard,
  projectConfirmCard,
  projectEvidence,
} from "./agentConversationLogic.ts";

test("W5-2f R5 D1: W3d 字段接线与工具目录纯逻辑套件", async (suite) => {
  // 1. classifyStreamEvents 保留并校验 4 个新字段
  await suite.test("1. classifyStreamEvents 保留 tool_round 的 4 个新字段并校验类型与同 call_id 更新", () => {
    // 1.1 合法字段保留
    const validEvents = [
      {
        seq: 1,
        kind: "tool_round",
        payload: {
          run_id: "run-1",
          call_id: "call-1",
          tool_id: "search_start",
          status: "done",
          started_at: "2026-10-26T10:15:30Z",
          duration_ms: 1250,
          result_count: 10,
          summary_zh: "检索完成",
        },
      },
    ];
    const items1 = classifyStreamEvents(validEvents);
    assert.strictEqual(items1.length, 1);
    const grp1 = items1[0] as StreamToolGroupItem;
    assert.strictEqual(grp1.rounds[0].started_at, "2026-10-26T10:15:30Z");
    assert.strictEqual(grp1.rounds[0].duration_ms, 1250);
    assert.strictEqual(grp1.rounds[0].result_count, 10);
    assert.strictEqual(grp1.rounds[0].summary_zh, "检索完成");

    // 1.2 非法字段丢弃（置为 undefined 或省略，不猜）
    const invalidEvents = [
      {
        seq: 2,
        kind: "tool_round",
        payload: {
          run_id: "run-1",
          call_id: "call-2",
          tool_id: "search_progress",
          status: "done",
          started_at: "   ",
          duration_ms: -50,
          result_count: 3.14,
          summary_zh: "",
        },
      },
      {
        seq: 3,
        kind: "tool_round",
        payload: {
          run_id: "run-1",
          call_id: "call-3",
          tool_id: "batch_page",
          status: "done",
          duration_ms: NaN,
          result_count: -1,
        },
      },
    ];
    const items2 = classifyStreamEvents(invalidEvents);
    const grp2 = items2[0] as StreamToolGroupItem;
    assert.ok(grp2.rounds[0].started_at == null);
    assert.ok(grp2.rounds[0].duration_ms == null);
    assert.ok(grp2.rounds[0].result_count == null);
    assert.ok(grp2.rounds[0].summary_zh == null);

    assert.ok(grp2.rounds[1].duration_ms == null);
    assert.ok(grp2.rounds[1].result_count == null);

    // 1.3 同 call_id 更新用新事件字段覆盖
    const updateEvents = [
      {
        seq: 4,
        kind: "tool_round",
        payload: {
          run_id: "run-1",
          call_id: "call-up",
          tool_id: "search_progress",
          status: "running",
          started_at: "2026-10-26T10:20:00Z",
        },
      },
      {
        seq: 5,
        kind: "tool_round",
        payload: {
          run_id: "run-1",
          call_id: "call-up",
          tool_id: "search_progress",
          status: "done",
          started_at: "2026-10-26T10:20:00Z",
          duration_ms: 2000,
          result_count: 42,
          summary_zh: "完成检索更新",
        },
      },
    ];
    const items3 = classifyStreamEvents(updateEvents);
    const grp3 = items3[0] as StreamToolGroupItem;
    assert.strictEqual(grp3.rounds.length, 1);
    assert.strictEqual(grp3.rounds[0].duration_ms, 2000);
    assert.strictEqual(grp3.rounds[0].result_count, 42);
    assert.strictEqual(grp3.rounds[0].summary_zh, "完成检索更新");
  });

  // 2. AGENT_TOOL_CATALOG 与 agent-tool-catalog.v1.json 键集合完全相等
  await suite.test("2. AGENT_TOOL_CATALOG 键集合与 contracts/v3/agent-tool-catalog.v1.json 完全相等且定义准确", () => {
    const __filename = fileURLToPath(import.meta.url);
    const catalogPath = path.resolve(__filename, "../../../../../contracts/v3/agent-tool-catalog.v1.json");
    const jsonContent = fs.readFileSync(catalogPath, "utf-8");
    const catalogJson = JSON.parse(jsonContent);
    const expectedToolIds = catalogJson.tools.map((t: { tool_id: string }) => t.tool_id);

    const actualKeys = Object.keys(AGENT_TOOL_CATALOG);
    assert.deepStrictEqual(actualKeys.sort(), expectedToolIds.sort());

    const registeredIds = REGISTERED_AGENT_TOOLS.map((t) => t.tool_id);
    assert.deepStrictEqual([...registeredIds].sort(), [...expectedToolIds].sort());

    // 验证 18 个真实工具逐字名称与 tier
    const expectations: Record<string, { name_zh: string; tier: number }> = {
      service_readiness: { name_zh: "检查服务状态", tier: 0 },
      capabilities_get: { name_zh: "读取可用能力", tier: 0 },
      working_aoi_get: { name_zh: "读取工作区 AOI", tier: 0 },
      admin_boundary_search: { name_zh: "检索行政区边界", tier: 0 },
      working_aoi_set: { name_zh: "设置工作区 AOI", tier: 1 },
      workbench_page: { name_zh: "读取工作台结果", tier: 0 },
      task_page: { name_zh: "读取任务列表", tier: 0 },
      task_event_page: { name_zh: "读取任务事件", tier: 0 },
      credential_readiness: { name_zh: "检查凭据状态", tier: 0 },
      update_download_task: { name_zh: "更新下载任务", tier: 2 },
      source_list: { name_zh: "读取数据源列表", tier: 0 },
      account_readiness: { name_zh: "检查账户状态", tier: 0 },
      task_overview: { name_zh: "读取任务概览", tier: 0 },
      batch_page: { name_zh: "读取检索批次", tier: 0 },
      search_progress: { name_zh: "查询检索进度", tier: 0 },
      search_start: { name_zh: "发起检索", tier: 1 },
      create_task: { name_zh: "创建下载任务", tier: 2 },
      task_action: { name_zh: "任务操作", tier: 2 },
    };

    const source = fs.readFileSync(path.resolve(__filename, "../agentLogic.ts"), "utf8");
    for (const [toolId, exp] of Object.entries(expectations)) {
      const item = AGENT_TOOL_CATALOG[toolId];
      assert.ok(item, `工具 ${toolId} 必须存在`);
      const key = Object.entries(MESSAGES["zh-CN"].agent.tools).find(([, value]) => value === exp.name_zh)?.[0];
      assert.ok(key, `工具 ${toolId} 的 zh-CN 文案必须保留在字典中`);
      assertLocalizedSource(source, `agent.tools.${key}`, exp.name_zh);
      assert.strictEqual(item.name_zh, exp.name_zh, `工具 ${toolId} 的 name_zh 不符`);
      assert.strictEqual(item.tier, exp.tier, `工具 ${toolId} 的 tier 不符`);
    }

    assert.strictEqual(resolveAgentToolDisplayName("ask_user_question"), "向用户确认参数");
    assert.strictEqual(resolveAgentToolDisplayName("map_workspace_action"), "同步地图工作区");
  });

  // 3. projectToolGroup 每行投影 timeZh、durationZh、resultCountZh、summaryZh
  await suite.test("3. projectToolGroup 输出 timeZh, durationZh, resultCountZh, summaryZh", () => {
    const item: StreamToolGroupItem = {
      kind: "tool_group",
      seq: 1,
      run_id: "run-1",
      status: "done",
      rounds: [
        {
          run_id: "run-1",
          call_id: "c1",
          tool_id: "search_start",
          status: "done",
          started_at: "2026-10-26T10:15:30Z",
          duration_ms: 850,
          result_count: 5,
          summary_zh: "已发起检索",
        },
        {
          run_id: "run-1",
          call_id: "c2",
          tool_id: "search_progress",
          status: "done",
          started_at: "2026-10-26T10:15:32Z",
          duration_ms: 15400,
          result_count: 0,
        },
        {
          run_id: "run-1",
          call_id: "c3",
          tool_id: "batch_page",
          status: "done",
          started_at: "2026-10-26T10:16:00Z",
          duration_ms: 125000,
        },
        {
          run_id: "run-1",
          call_id: "c4",
          tool_id: "create_task",
          status: "failed",
          started_at: "invalid-time",
          duration_ms: -5,
          result_count: -1,
        },
      ],
    };

    const proj = projectToolGroup(item);
    assert.strictEqual(proj.rows.length, 4);

    // c1: 850ms -> 850 毫秒, result_count 5 -> 返回 5 条
    assert.match(proj.rows[0].timeZh || "", /^\d{2}:\d{2}:\d{2}$/);
    assert.strictEqual(proj.rows[0].durationZh, "850 毫秒");
    assert.strictEqual(proj.rows[0].resultCountZh, "返回 5 条");
    assert.strictEqual(proj.rows[0].summaryZh, "已发起检索");

    // c2: 15400ms -> 15.4 秒, result_count 0 -> 返回 0 条, summaryZh -> null
    assert.strictEqual(proj.rows[1].durationZh, "15.4 秒");
    assert.strictEqual(proj.rows[1].resultCountZh, "返回 0 条");
    assert.strictEqual(proj.rows[1].summaryZh, null);

    // c3: 125000ms -> 2 分 5 秒
    assert.strictEqual(proj.rows[2].durationZh, "2 分 5 秒");
    assert.strictEqual(proj.rows[2].resultCountZh, null);
    assert.strictEqual(proj.rows[2].summaryZh, null);

    // c4: 非法时间与负数丢弃为 null
    assert.strictEqual(proj.rows[3].timeZh, null);
    assert.strictEqual(proj.rows[3].durationZh, null);
    assert.strictEqual(proj.rows[3].resultCountZh, null);
    assert.strictEqual(proj.rows[3].summaryZh, null);

    // c5: 小时级长时任务 -> H 小时 M 分 S 秒
    const hourProj = projectToolGroup({
      kind: "tool_group",
      seq: 2,
      run_id: "run-hour",
      status: "done",
      rounds: [
        {
          run_id: "run-hour",
          call_id: "ch1",
          tool_id: "gee_chunked_download",
          status: "done",
          duration_ms: 3_725_000,
        },
      ],
    });
    assert.strictEqual(hourProj.rows[0].durationZh, "1 小时 2 分 5 秒");
  });

  // 4. projectSearchCard 只取 search_progress 的 result_count
  await suite.test("4. projectSearchCard 只提取 search_progress 最后一个非空 result_count 为 N 景", () => {
    const groupWithSearch: StreamToolGroupItem = {
      kind: "tool_group",
      seq: 1,
      run_id: "run-1",
      status: "done",
      rounds: [
        {
          run_id: "run-1",
          call_id: "c1",
          tool_id: "search_start",
          status: "done",
          result_count: 99,
        },
        {
          run_id: "run-1",
          call_id: "c2",
          tool_id: "search_progress",
          status: "done",
          result_count: 12,
        },
        {
          run_id: "run-1",
          call_id: "c3",
          tool_id: "batch_page",
          status: "done",
          result_count: 50, // 单页条数，不可当作结果数
        },
      ],
    };

    const proj = projectSearchCard(groupWithSearch, {
      source: "ASF",
      conditionBadges: ["SLC"],
    });
    assert.strictEqual(proj.resultCountZh, "12 景");

    // 无 search_progress 结果数的情况
    const groupWithoutCount: StreamToolGroupItem = {
      kind: "tool_group",
      seq: 2,
      run_id: "run-2",
      status: "done",
      rounds: [
        {
          run_id: "run-2",
          call_id: "c4",
          tool_id: "batch_page",
          status: "done",
          result_count: 50,
        },
      ],
    };
    const proj2 = projectSearchCard(groupWithoutCount, null);
    assert.strictEqual(proj2.resultCountZh, null);
  });

  // 5. projectConfirmCard 投影 resultLine 与 orbitBadges
  await suite.test("5. projectConfirmCard 投影 resultLine 与 orbitBadges", () => {
    // 5.1 already_queued_count > 0 -> 重新打开已有任务
    const itemReopen: StreamConfirmCardItem = {
      kind: "confirm_card",
      seq: 1,
      run_id: "run-1",
      plan_id: "p1",
      plan_hash: "h1",
      snapshot: {
        source: "ASF",
        range: { aoi_name: "茂县", area_km2: 100, time_range: null, product: "SLC" },
        estimated_bytes: 1000,
        output_dir: "/data",
        result_count: 5,
        already_queued_count: 2,
        orbit_directions: ["Ascending", "Descending", "Ascending", "CustomTrack"],
      },
    };
    const proj1 = projectConfirmCard(itemReopen, null);
    assert.strictEqual(proj1.resultLine, "5 景，将重新打开已有任务（含 2 景）");
    assert.deepStrictEqual(proj1.orbitBadges, [
      { label: "Ascending", tone: "asc" },
      { label: "Descending", tone: "desc" },
      { label: "CustomTrack", tone: "other" },
    ]);

    // 5.2 already_queued_count === 0 -> 新增 N 景
    const itemNew: StreamConfirmCardItem = {
      ...itemReopen,
      snapshot: {
        ...itemReopen.snapshot,
        result_count: 8,
        already_queued_count: 0,
        orbit_directions: ["Descending"],
      },
    };
    const proj2 = projectConfirmCard(itemNew, null);
    assert.strictEqual(proj2.resultLine, "新增 8 景");
    assert.deepStrictEqual(proj2.orbitBadges, [
      { label: "Descending", tone: "desc" },
    ]);

    // 5.3 already_queued_count 为 null/undefined -> N 景
    const itemPlain: StreamConfirmCardItem = {
      ...itemReopen,
      snapshot: {
        ...itemReopen.snapshot,
        result_count: 3,
        already_queued_count: null,
      },
    };
    const proj3 = projectConfirmCard(itemPlain, null);
    assert.strictEqual(proj3.resultLine, "3 景");

    // 5.4 result_count 为 null/undefined -> null
    const itemNull: StreamConfirmCardItem = {
      ...itemReopen,
      snapshot: {
        ...itemReopen.snapshot,
        result_count: null,
      },
    };
    const proj4 = projectConfirmCard(itemNull, null);
    assert.strictEqual(proj4.resultLine, null);
  });

  // 6. projectEvidence 投影 traceZh, responseZh 以及摘要行汇总
  await suite.test("6. projectEvidence 投影 traceZh, responseZh 与摘要行", () => {
    const itemWithEvidence: StreamConclusionItem = {
      kind: "conclusion",
      seq: 1,
      run_id: "run-1",
      text: "测试结论",
      claims: [
        {
          statement: "声明 1",
          verified: true,
          refs: [
            {
              kind: "task",
              id: "t1",
              label: "任务 1",
              trace_sequence: 3,
              response_count: 5,
            },
            {
              kind: "batch",
              id: "b1",
              label: "批次 1",
              trace_sequence: 4,
              response_count: 0,
            },
          ],
        },
        {
          statement: "声明 2",
          verified: false,
          refs: [
            {
              kind: "web",
              id: "w1",
              label: "网络 1",
              trace_sequence: -1, // 非法
              response_count: -2, // 非法
            },
          ],
        },
      ],
    };

    const proj = projectEvidence(itemWithEvidence);
    assert.strictEqual(proj.refs.length, 3);

    // 第一个引用
    assert.strictEqual(proj.refs[0].traceZh, "运行记录第 #3 条");
    assert.strictEqual(proj.refs[0].responseZh, "检索响应 5 条");

    // 第二个引用
    assert.strictEqual(proj.refs[1].traceZh, "运行记录第 #4 条");
    assert.strictEqual(proj.refs[1].responseZh, "检索响应 0 条");

    // 第三个引用（非法值置为 null）
    assert.strictEqual(proj.refs[2].traceZh, null);
    assert.strictEqual(proj.refs[2].responseZh, null);

    // 摘要行取第一个非空
    assert.strictEqual(proj.summaryTraceZh, "运行记录第 #3 条");
    assert.strictEqual(proj.summaryResponseZh, "检索响应 5 条");

    // 无引用或无新字段的情况
    const itemEmpty: StreamConclusionItem = {
      kind: "conclusion",
      seq: 2,
      run_id: "run-2",
      text: "空结论",
      claims: [],
    };
    const projEmpty = projectEvidence(itemEmpty);
    assert.strictEqual(projEmpty.summaryTraceZh, null);
    assert.strictEqual(projEmpty.summaryResponseZh, null);
  });

  // 7. splitInlineThink 与 cleanCopyableMarkdown 全状态机边界校验
  await suite.test("7. splitInlineThink 剥离 <think>/<thinking>、孤立闭合、未闭合尾部并保护代码块", () => {
    // 7.1 标准 <think>...</think>
    const s1 = splitInlineThink("<think>先检索合肥市行政区边界</think>\n\n已定位**合肥市**，共 12 景。");
    assert.strictEqual(s1.thinkingText, "先检索合肥市行政区边界");
    assert.strictEqual(s1.answerMarkdown, "已定位**合肥市**，共 12 景。");

    // 7.2 多段 <think> 与 <thinking>
    const s2 = splitInlineThink("<think>步骤一</think>中间说明\n<thinking>步骤二</thinking>\n最终结论");
    assert.strictEqual(s2.thinkingText, "步骤一\n\n步骤二");
    assert.ok(!s2.answerMarkdown.includes("<think"));
    assert.ok(s2.answerMarkdown.includes("中间说明"));
    assert.ok(s2.answerMarkdown.includes("最终结论"));

    // 7.3 孤立闭合 </think>
    const s3 = splitInlineThink("隐式前置思考文本\n</think>\n公开答复正文");
    assert.strictEqual(s3.thinkingText, "隐式前置思考文本");
    assert.strictEqual(s3.answerMarkdown, "公开答复正文");

    // 7.4 未闭合 <think> 尾部：隔离至 thinkingText，正文不泄露 <think>
    const s4 = splitInlineThink("<think>正在分析轨道方向与极化方式...");
    assert.strictEqual(s4.thinkingText, "正在分析轨道方向与极化方式...");
    assert.strictEqual(s4.answerMarkdown, "");

    // 7.5 围栏代码块与行内代码免疫
    const fenced = "示例说明：\n```xml\n<think>示例代码内标签</think>\n```\n以及行内 `<think>` 标签。";
    const s5 = splitInlineThink(fenced);
    assert.strictEqual(s5.thinkingText, "");
    assert.strictEqual(s5.answerMarkdown, fenced);

    // 7.6 cleanCopyableMarkdown 解包裸结论 JSON 并剥离 <think>
    const dirtyCopy = '<think>内部推理</think>\n{"text":"检索完成，共 **8** 景。","claims":[]}';
    assert.strictEqual(cleanCopyableMarkdown(dirtyCopy), "检索完成，共 **8** 景。");
  });

  // 8. classifyStreamEvents 支持 Agent 2.0 thinking 合并、question_card、workspace_sync 与内联思考剥离
  await suite.test("8. classifyStreamEvents 投影 thinking / question_card / workspace_sync 并净化结论", () => {
    const events = [
      {
        seq: 1,
        kind: "thinking",
        payload: {
          run_id: "run-20",
          step_index: 0,
          status: "thinking",
          text: "正在检查合肥范围...",
          started_at_ms: 1000,
        },
      },
      {
        seq: 2,
        kind: "thinking",
        payload: {
          run_id: "run-20",
          step_index: 0,
          status: "done",
          text: "正在检查合肥范围，准备绑定 340100。",
          thinking_ms: 1850,
        },
      },
      {
        seq: 3,
        kind: "workspace_sync",
        payload: {
          run_id: "run-20",
          action: "aoi_changed",
          display_name: "安徽省合肥市",
          bbox: [116.68, 30.95, 117.99, 32.53],
        },
      },
      {
        seq: 4,
        kind: "question_card",
        payload: {
          run_id: "run-20",
          question_zh: "请选择 Sentinel-1 轨道方向：",
          options: [
            { label_zh: "升轨 (Ascending)", value: "ASCENDING", recommended: true },
            { label_zh: "降轨 (Descending)", value: "DESCENDING" },
          ],
        },
      },
      {
        seq: 5,
        kind: "message",
        payload: {
          run_id: "run-20",
          role: "assistant",
          text: "<think>内部思考不泄露</think>已将工作区设置为**合肥市**。",
          thinking_ms: 920,
        },
      },
      {
        seq: 6,
        kind: "conclusion",
        payload: {
          run_id: "run-20",
          text: "<think>总结前核对景数</think>共检索到 6 景 Sentinel-1 SLC 数据。",
          claims: [],
          thinking_ms: 1400,
        },
      },
    ];

    const items = classifyStreamEvents(events as any);
    assert.strictEqual(items.length, 5);

    const thinkItem = items[0] as StreamThinkingItem;
    assert.strictEqual(thinkItem.kind, "thinking");
    assert.strictEqual(thinkItem.status, "done");
    assert.strictEqual(thinkItem.started_at_ms, 1000);
    assert.strictEqual(thinkItem.thinking_ms, 1850);
    assert.strictEqual(thinkItem.text, "正在检查合肥范围，准备绑定 340100。");

    const syncItem = items[1] as StreamWorkspaceSyncItem;
    assert.strictEqual(syncItem.kind, "workspace_sync");
    assert.strictEqual(syncItem.action, "aoi_changed");
    assert.strictEqual(syncItem.display_name, "安徽省合肥市");
    assert.deepStrictEqual(syncItem.bbox, [116.68, 30.95, 117.99, 32.53]);

    const qItem = items[2] as StreamQuestionCardItem;
    assert.strictEqual(qItem.kind, "question_card");
    assert.strictEqual(qItem.question_zh, "请选择 Sentinel-1 轨道方向：");
    assert.strictEqual(qItem.options.length, 2);
    assert.strictEqual(qItem.options[0].recommended, true);

    const msgItem = items[3] as StreamMessageItem;
    assert.strictEqual(msgItem.text, "已将工作区设置为**合肥市**。");
    assert.strictEqual(msgItem.thinking_text, "内部思考不泄露");
    assert.strictEqual(msgItem.thinking_ms, 920);

    const concItem = items[4] as StreamConclusionItem;
    assert.strictEqual(concItem.text, "共检索到 6 景 Sentinel-1 SLC 数据。");
    assert.strictEqual(concItem.thinking_text, "总结前核对景数");
    assert.strictEqual(concItem.thinking_ms, 1400);
  });
});
