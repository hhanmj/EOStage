import test from "node:test";
import assert from "node:assert/strict";
import {
  getPerformanceOnlyPrefs,
  setPerformanceOnlyPrefs,
  getWorkStepsPrefs,
  setWorkStepsPrefs,
  subscribeWorkStepsPrefs,
  shouldExpandToolGroupByDefault,
  shouldHideIntermediateReply,
  computeContextTelemetry,
  compareRunRecency,
  pickLatestUsageRun,
  formatCompactTokens,
} from "../../lib/performanceAndUsagePrefs.ts";
import {
  SETTINGS_CATEGORIES,
  SETTINGS_GROUPS,
  normalizeSettingsCategory,
} from "./settingsEntries.ts";
import {
  resolveAdminBoundaryAuthority,
  getAoiBookmarks,
  saveAoiBookmark,
  removeAoiBookmark,
  clearAoiBookmarks,
  mergeAoiRegions,
} from "../../lib/aoiBookmarksAndMerge.ts";
import {
  buildDownloadFolderCatalogGeoJson,
  buildDownloadFolderReadmeMarkdown,
  buildDownloadFolderCatalogBundle,
} from "../../lib/downloadFolderCatalog.ts";
import {
  getToolsForContext,
  summarizeToolCatalog,
  searchRegisteredTools,
  getRegisteredToolInfo,
  buildBrokeredAgentTools,
  OAUTH_PKCE_PROVIDERS,
  generatePkcePair,
  generateOAuthState,
  buildOAuthAuthorizationUrl,
  extractOAuthAuthorizationCode,
  extractChatGptAccountId,
  isOAuthTokenExpiringSoon,
  isLowCloudRequest,
  computeCandidatePoolLimit,
  geometryContainsPoint,
  computeBboxOverlapRatio,
  rankScenesBySpatialFit,
  buildSoftCancelToolFeedback,
  compactToolResultForContext,
  validateGeoScriptSafety,
  exportReproduciblePythonRecipe,
  OLMOEARTH_V13_SPECS,
  OLMOEARTH_ARTIFACT_LICENSE_NOTICE_ZH,
  planOlmoEarthUtmShardWindow,
  dequantizeOlmoEarthInt8,
  dequantizeOlmoEarthVector,
  buildOlmoEarthObservationCrosswalk,
  exportOlmoEarthCompanionPythonRecipe,
} from "../../lib/eoAgentSuite.ts";
import { REGISTERED_AGENT_TOOLS } from "../../lib/agentRegisteredTools.ts";
import type { ApplicationTask } from "./types.ts";

test("1. 设置三分组（基础设置 / Agent 功能 / 数据与统计）、外观合并（性能与用量 + 工作步骤显示）及真实遥测验证", async (suite) => {
  await suite.test("1.1 SETTINGS_GROUPS 划分为 basic / agent / data 三大类，且 performance_usage 与 work_steps 已合并入 appearance", () => {
    assert.deepEqual(
      SETTINGS_GROUPS.map((g) => g.key),
      ["basic", "agent", "data"],
      "SETTINGS_GROUPS 必须包含基础设置、Agent 功能、数据与统计三大分组"
    );
    const appearanceEntry = SETTINGS_CATEGORIES.find((c) => c.key === "appearance");
    assert.ok(appearanceEntry, "SETTINGS_CATEGORIES 必须包含 appearance 入口");
    assert.equal(appearanceEntry.group, "basic");
    assert.equal(
      SETTINGS_CATEGORIES.some((c) => c.key === "performance_usage" || c.key === "work_steps"),
      false,
      "performance_usage 与 work_steps 不得在侧栏单独列出，应合并在外观中"
    );
    assert.equal(
      SETTINGS_CATEGORIES.some((c) => c.key === "plugins_skills"),
      false,
      "plugins_skills 应统一收口到主侧栏，不得继续在设置侧栏重复列出"
    );
    assert.ok(SETTINGS_CATEGORIES.some((c) => c.key === "runtime_tools" && c.group === "agent"));
    assert.equal(normalizeSettingsCategory("plugins_skills"), "runtime_tools");
    assert.ok(SETTINGS_CATEGORIES.some((c) => c.key === "usage_stats" && c.group === "data"));
  });

  await suite.test("1.2 PerformanceOnlyPrefs 与 WorkStepsPrefs 支持互不干扰的独立读写与订阅", () => {
    setPerformanceOnlyPrefs({ hudMode: "minimal" });
    setWorkStepsPrefs({
      stepDetailMode: "concise",
      collapseTiming: "completion",
      foldedReplies: "all",
    });

    assert.deepEqual(getPerformanceOnlyPrefs(), { hudMode: "minimal" });
    assert.deepEqual(getWorkStepsPrefs(), {
      stepDetailMode: "concise",
      collapseTiming: "completion",
      foldedReplies: "all",
    });

    let workStepsNotifyCount = 0;
    const unsubscribeWorkSteps = subscribeWorkStepsPrefs(() => {
      workStepsNotifyCount += 1;
    });

    // 修改「性能与用量」不应触发「工作步骤显示」监听器
    setPerformanceOnlyPrefs({ hudMode: "pro" });
    assert.equal(workStepsNotifyCount, 0);
    assert.deepEqual(getPerformanceOnlyPrefs(), { hudMode: "pro" });

    setWorkStepsPrefs({
      stepDetailMode: "detailed",
      collapseTiming: "next-input",
      foldedReplies: "final",
    });
    assert.equal(workStepsNotifyCount, 1);
    unsubscribeWorkSteps();

    assert.deepEqual(getWorkStepsPrefs(), {
      stepDetailMode: "detailed",
      collapseTiming: "next-input",
      foldedReplies: "final",
    });

    // 非法输入值安全归一化回退到默认简洁态
    setPerformanceOnlyPrefs({ hudMode: "invalid" as any });
    setWorkStepsPrefs({
      stepDetailMode: "invalid" as any,
      collapseTiming: "invalid" as any,
      foldedReplies: "invalid" as any,
    });
    assert.deepEqual(getPerformanceOnlyPrefs(), { hudMode: "minimal" });
    assert.deepEqual(getWorkStepsPrefs(), {
      stepDetailMode: "concise",
      collapseTiming: "completion",
      foldedReplies: "all",
    });
  });

  await suite.test("1.2b 工作步骤收起时机 (collapseTiming) 与收起后显示的回复 (foldedReplies) 策略函数验证", () => {
    assert.equal(
      shouldExpandToolGroupByDefault({
        stepDetailMode: "detailed",
        collapseTiming: "completion",
        itemStatus: "done",
        hasSubsequentUserInput: true,
      }),
      true
    );

    assert.equal(
      shouldExpandToolGroupByDefault({
        stepDetailMode: "concise",
        collapseTiming: "completion",
        itemStatus: "running",
        hasSubsequentUserInput: false,
      }),
      true
    );
    assert.equal(
      shouldExpandToolGroupByDefault({
        stepDetailMode: "concise",
        collapseTiming: "completion",
        itemStatus: "done",
        hasSubsequentUserInput: false,
      }),
      false
    );

    assert.equal(
      shouldExpandToolGroupByDefault({
        stepDetailMode: "concise",
        collapseTiming: "next-input",
        itemStatus: "done",
        hasSubsequentUserInput: false,
      }),
      true
    );
    assert.equal(
      shouldExpandToolGroupByDefault({
        stepDetailMode: "concise",
        collapseTiming: "next-input",
        itemStatus: "done",
        hasSubsequentUserInput: true,
      }),
      false
    );

    assert.equal(
      shouldHideIntermediateReply({
        stepDetailMode: "concise",
        foldedReplies: "final",
        isWorkStepsCollapsed: true,
        hasLaterAssistantReplyInTurn: true,
      }),
      true
    );
    assert.equal(
      shouldHideIntermediateReply({
        stepDetailMode: "concise",
        foldedReplies: "all",
        isWorkStepsCollapsed: true,
        hasLaterAssistantReplyInTurn: true,
      }),
      false
    );
  });

  await suite.test("1.3 computeContextTelemetry 严格基于真实 runs / usage_total / events 计算，零伪造模拟数值", () => {
    // 空会话：所有计数、速率、Token 与上下文必须如实为 0（绝不伪造 68.4 tok/s 或 4200 基础 token）
    const emptySnap = computeContextTelemetry({
      runs: [],
      events: [],
      draftText: "",
      maxContextTokens: 1_000_000,
    });
    assert.equal(emptySnap.turns, 0);
    assert.equal(emptySnap.steps, 0);
    assert.equal(emptySnap.hasReportedUsage, false);
    assert.equal(emptySnap.totalTokens, 0);
    assert.equal(emptySnap.inputTokens, 0);
    assert.equal(emptySnap.outputTokens, 0);
    assert.equal(emptySnap.tps, 0);
    assert.equal(emptySnap.totalTimeSec, 0);
    assert.equal(emptySnap.currentContextTokens, 0);
    assert.equal(emptySnap.contextPct, 0);

    // 真实运行记录：带 usage_total 与真实起止时间戳
    const realSnap = computeContextTelemetry({
      runs: [
        {
          run_id: "r1",
          status: "completed",
          started_at: "2026-11-05T10:00:00.000Z",
          ended_at: "2026-11-05T10:00:05.000Z",
          usage_total: { prompt: 12_000, completion: 500 },
        },
      ],
      events: [
        { kind: "user_message", payload: { text: "查询北京 Sentinel-1 数据" } },
        { kind: "tool_call", payload: { tool_id: "search_start", duration_ms: 1400 } },
        { kind: "tool_result", payload: { result: { total: 8 } } },
      ],
      draftText: "继续筛选升轨数据",
      maxContextTokens: 1_000_000,
    });

    assert.equal(realSnap.turns, 1);
    assert.equal(realSnap.steps, 2);
    assert.equal(realSnap.hasReportedUsage, true);
    assert.equal(realSnap.reportedRunsCount, 1);
    assert.equal(realSnap.inputTokens, 12_000);
    assert.equal(realSnap.outputTokens, 500);
    assert.equal(realSnap.totalTokens, 12_500);
    assert.equal(realSnap.totalTimeSec, 5);
    assert.equal(realSnap.toolSec, 1.4);
    assert.equal(realSnap.tps, 100); // 500 completion tokens / 5.0 sec = 100 tok/s
    assert.ok(realSnap.currentContextTokens > 12_000);
    assert.equal(formatCompactTokens(12_500), "12.5k");
    assert.equal(formatCompactTokens(1_000_000), "1M");

    // 真实 v3-contracts 事件流契约回归：
    // 1) kind === "message" && payload.role === "user" 计入轮次；
    // 2) 同一 call_id 的 tool_round (running + done) 去重计为 1 步；
    // 3) usage_total.total > prompt + completion 时保留真实 total；无 completion 的 run 不稀释 tps。
    const contractSnap = computeContextTelemetry({
      runs: [
        {
          run_id: "r-with-total",
          status: "done",
          started_at: "2026-11-05T10:00:00.000Z",
          ended_at: "2026-11-05T10:00:04.000Z",
          usage_total: { prompt: 4_000, completion: 200, total: 4_500 },
        },
        {
          run_id: "r-prompt-only",
          status: "done",
          started_at: "2026-11-05T10:01:00.000Z",
          ended_at: "2026-11-05T10:01:10.000Z",
          usage_total: { prompt: 1_000, completion: 0 },
        },
      ],
      events: [
        { kind: "message", payload: { role: "user", text: "检索汶川 SAR 数据" } },
        { kind: "tool_round", payload: { call_id: "call-1", tool_id: "search_start", status: "running" } },
        { kind: "tool_round", payload: { call_id: "call-1", tool_id: "search_start", status: "done", duration_ms: 800 } },
      ],
    });
    assert.equal(contractSnap.steps, 1, "同一 call_id 的 running/done tool_round 必须去重计为 1 步");
    assert.equal(contractSnap.totalTokens, 5_500, "usage_total.total 应被如实保留");
    assert.equal(contractSnap.tps, 50, "无 completion 的 run 不应稀释输出速率 (200 tok / 4s = 50 tok/s)");

    const streamOnlyTurns = computeContextTelemetry({
      runs: [],
      events: [
        { kind: "message", payload: { role: "user", content: "第一轮提问" } },
        { kind: "message", payload: { role: "assistant", content: "第一轮回答" } },
        { kind: "message", payload: { role: "user", content: "第二轮提问" } },
      ],
    });
    assert.equal(streamOnlyTurns.turns, 2, "runs 为空时应从 kind=message, role=user 统计真实轮次");

    // 真实 Prompt Cache 命中率与四分类上下文明细计算（兼容 OpenAI/Responses 包含式与 Anthropic 附加式）
    const openaiCacheSnap = computeContextTelemetry({
      runs: [
        {
          run_id: "r-cache-oai",
          status: "done",
          usage_total: { prompt: 10_000, completion: 500, cached: 7_500 },
        },
      ],
      events: [
        { kind: "message", payload: { role: "user", text: "查询长三角区域 Sentinel-1 数据" } },
      ],
      maxContextTokens: 1_048_576,
    });
    assert.equal(openaiCacheSnap.cachedTokens, 7_500);
    assert.equal(openaiCacheSnap.hasCacheData, true);
    assert.equal(openaiCacheSnap.cacheHitPct, 75);
    assert.equal(Object.keys(openaiCacheSnap.categories).length, 4);
    assert.equal(formatCompactTokens(1_048_576), "1.05M");

    const anthropicCacheSnap = computeContextTelemetry({
      runs: [
        {
          run_id: "r-cache-anth",
          status: "done",
          usage_total: { prompt: 2_000, completion: 300, cached: 6_000, cache_creation: 2_000 },
        },
      ],
      events: [],
      maxContextTokens: 200_000,
    });
    assert.equal(anthropicCacheSnap.cachedTokens, 6_000);
    assert.equal(anthropicCacheSnap.cacheCreationTokens, 2_000);
    assert.equal(anthropicCacheSnap.cacheHitPct, 60); // 6000 / (2000 + 6000 + 2000) = 60%

    // 回归防护（576.7k 多子调用累加误报修复）：
    // 单轮内发生 8 次模型调用时，usage_total.prompt 为累计计费词数 (576,653)，而 last_prompt 为末次实际窗口水位 (102,371)；
    // 上下文容量面板必须采用 last_prompt (~102.4k / 10.2%)，且将超出的窗口体量如实归入「系统工具」而非「对话消息」！
    const multiCallRunSnap = computeContextTelemetry({
      runs: [
        {
          run_id: "run-1a1e3734f8d-2",
          status: "completed",
          usage_total: {
            prompt: 576_653,
            completion: 9_339,
            cached: 363_840,
            reasoning: 7_063,
            last_prompt: 102_371,
          },
        },
      ],
      events: [
        { kind: "message", payload: { role: "user", text: "今天有哪些最新的数据？" } },
        { kind: "conclusion", payload: { text: "最新可用为 Sentinel-2 (25 景) 与 Sentinel-1 (12 景)。" } },
      ],
      maxContextTokens: 1_000_000,
    });
    assert.equal(multiCallRunSnap.inputTokens, 576_653, "累计输入计费词数保持如实统计");
    assert.equal(multiCallRunSnap.cacheHitPct, 63.1, "平均缓存命中率保持 63.1%");
    assert.equal(multiCallRunSnap.currentContextTokens, 102_371, "当前窗口占用水位必须取末次 prompt 而非 8 次累加");
    assert.equal(multiCallRunSnap.contextPct, 10.2);
    assert.ok(
      multiCallRunSnap.categories.messages.tokens < 2_000,
      `9 字提问不应被虚报为数十万对话消息，实际：${multiCallRunSnap.categories.messages.tokens}`
    );
    assert.ok(
      multiCallRunSnap.categories.tools.tokens > 90_000,
      `大体积工具返回应如实计入系统工具分类，实际：${multiCallRunSnap.categories.tools.tokens}`
    );

    // Casual greeting fast-path (tools = [], request.fast_path = "casual_greeting"): tools/skills tokens must be 0
    const casualSnap = computeContextTelemetry({
      runs: [
        {
          run_id: "run-casual-hi",
          status: "completed",
          usage_total: { prompt: 220, completion: 18, last_prompt: 220 },
        },
      ],
      events: [
        {
          kind: "request",
          payload: { run_id: "run-casual-hi", fast_path: "casual_greeting", tool_count: 0 },
        },
        { kind: "message", payload: { role: "user", text: "hi" } },
      ],
      maxContextTokens: 131_072,
    });
    assert.equal(casualSnap.categories.tools.tokens, 0);
    assert.equal(casualSnap.categories.skills.tokens, 0);
    assert.ok(casualSnap.categories.messages.tokens > 0);

    // 未上报缓存字段的渠道必须返回 hasCacheData=false（不得混为 0%）；混合渠道仅用上报缓存的轮次作分母
    const unreportedSnap = computeContextTelemetry({
      runs: [
        {
          run_id: "r-no-cache",
          status: "completed",
          model_route: "chat_completions/agnes-3.0-flash",
          usage_total: { prompt: 12_000, completion: 300 },
        },
      ],
      events: [{ kind: "message", payload: { role: "user", text: "你好" } }],
    });
    assert.equal(unreportedSnap.hasCacheData, false);
    assert.equal(unreportedSnap.cacheHitPct, 0);

    const mixedSnap = computeContextTelemetry({
      runs: [
        {
          run_id: "r-unreported",
          status: "completed",
          model_route: "chat_completions/agnes-3.0-flash",
          usage_total: { prompt: 10_000, completion: 200 },
        },
        {
          run_id: "r-reported",
          status: "completed",
          model_route: "responses/glm-5.3-flash",
          usage_total: { prompt: 5_000, completion: 100, cached: 4_000 },
        },
      ],
      events: [],
    });
    assert.equal(mixedSnap.hasCacheData, true);
    assert.equal(mixedSnap.cacheHitPct, 80, "未上报缓存的轮次不应稀释已上报缓存轮次的分母");

    // Anthropic 加法计费（cached <= prompt 且无 cache_creation）必须通过 model_route 识别为 prompt + cached
    const anthropicAdditiveNoCreation = computeContextTelemetry({
      runs: [
        {
          run_id: "r-anth-route",
          status: "completed",
          model_route: "anthropic_messages/claude-sonnet-4",
          usage_total: { prompt: 2_000, completion: 300, cached: 1_500 },
        },
      ],
      events: [],
    });
    assert.equal(anthropicAdditiveNoCreation.hasCacheData, true);
    assert.equal(anthropicAdditiveNoCreation.cacheHitPct, 42.9);

    const anthropicEventsFallback = computeContextTelemetry({
      runs: [
        {
          run_id: "r-anth-ev-fallback",
          status: "completed",
          model_route: "anthropic_messages/claude-sonnet-4",
          usage_total: { prompt: 2_000, completion: 300 },
        },
      ],
      events: [
        {
          kind: "model_reply",
          payload: { usage: { prompt: 2_000, completion: 300, cached: 1_500 } },
        },
      ],
    });
    assert.equal(anthropicEventsFallback.hasCacheData, true);
    assert.equal(anthropicEventsFallback.cacheHitPct, 42.9);

    // 1.5 多轮会话 newest-first 排序（runs[0] 最新，runs[last] 为 Turn 0 仅 138 tok）：
    // 上下文窗口水位必须选取最新 run，绝不能被较老的 Turn 0 覆盖成 ~138！
    const newestFirstRuns = [
      {
        run_id: "run-1a20271b77d-4",
        status: "completed",
        model_route: "responses/glm-5.3-flash",
        started_at: "2026-11-22T08:48:30Z",
        ended_at: "2026-11-22T08:48:37Z",
        usage_total: { prompt: 13_314, completion: 307, cached: 12_544 },
      },
      {
        run_id: "run-1a20271364e-2",
        status: "completed",
        model_route: "responses/glm-5.3-flash",
        started_at: "2026-11-22T08:47:50Z",
        ended_at: "2026-11-22T08:48:22Z",
        usage_total: { prompt: 34_324, completion: 2_368, cached: 21_248 },
      },
      {
        run_id: "run-1a2027122c1-1",
        status: "completed",
        model_route: "responses/glm-5.3-flash",
        started_at: "2026-11-22T08:47:10Z",
        ended_at: "2026-11-22T08:47:14Z",
        usage_total: { prompt: 138, completion: 105, cached: 0 },
      },
    ];
    assert.equal(pickLatestUsageRun(newestFirstRuns)?.run_id, "run-1a20271b77d-4");
    const newestFirstSnap = computeContextTelemetry({
      runs: newestFirstRuns,
      events: [],
    });
    assert.equal(newestFirstSnap.currentContextTokens, 13_314);

    // 同秒级 started_at / ended_at 时按 run-<hex>-<seq> 决出最新一条
    const sameSecondRuns = [
      {
        run_id: "run-1a20271b77d-4",
        status: "completed",
        started_at: "2026-11-22T08:48:30Z",
        ended_at: "2026-11-22T08:48:31Z",
        usage_total: { prompt: 9_600, completion: 120 },
      },
      {
        run_id: "run-1a20271b77d-1",
        status: "completed",
        started_at: "2026-11-22T08:48:30Z",
        ended_at: "2026-11-22T08:48:31Z",
        usage_total: { prompt: 140, completion: 50 },
      },
    ];
    assert.ok(compareRunRecency(sameSecondRuns[0], sameSecondRuns[1]) > 0);
    assert.equal(pickLatestUsageRun(sameSecondRuns)?.run_id, "run-1a20271b77d-4");
    assert.equal(computeContextTelemetry({ runs: sameSecondRuns }).currentContextTokens, 9_600);

    // 无时间戳且按 oldest-first 排列的旧测试数据仍保持取末尾元素兼容语义
    const oldestFirstUntimed = [
      { run_id: "legacy-a", usage_total: { prompt: 500, completion: 20 } },
      { run_id: "legacy-b", usage_total: { prompt: 2_400, completion: 80 } },
    ];
    assert.equal(pickLatestUsageRun(oldestFirstUntimed)?.run_id, "legacy-b");
    assert.equal(computeContextTelemetry({ runs: oldestFirstUntimed }).currentContextTokens, 2_400);

    // newest-first 且仅最新 run 带 fast_path="casual_greeting" 时，工具/技能为 0；下一轮非 casual 则恢复四分类
    const fastPathRunOnly = computeContextTelemetry({
      runs: [
        {
          run_id: "run-1a2027122c1-1",
          fast_path: "casual_greeting",
          started_at: "2026-11-22T08:47:10Z",
          usage_total: { prompt: 138, completion: 105 },
        },
      ],
      events: [{ kind: "tool_call", payload: { call_id: "stale", text: "history" } }],
    });
    assert.equal(fastPathRunOnly.categories.tools.tokens, 0);
    assert.equal(fastPathRunOnly.categories.skills.tokens, 0);

    const switchedBackToToolRun = computeContextTelemetry({
      runs: [
        {
          run_id: "run-1a20271364e-2",
          fast_path: null,
          started_at: "2026-11-22T08:47:50Z",
          usage_total: { prompt: 12_000, completion: 400 },
        },
        {
          run_id: "run-1a2027122c1-1",
          fast_path: "casual_greeting",
          started_at: "2026-11-22T08:47:10Z",
          usage_total: { prompt: 138, completion: 105 },
        },
      ],
      events: [],
    });
    assert.ok(switchedBackToToolRun.categories.tools.tokens > 0);
    assert.ok(switchedBackToToolRun.categories.skills.tokens > 0);
  });
});

test("2. 行政区境内外权威源判定、范围收藏夹与多区域一键合并验证", async (suite) => {
  await suite.test("2.1 行政区来源严格依据后端返回的真实 provider 提取", () => {
    const withProvider = resolveAdminBoundaryAuthority({
      provider: "DataV",
      display_name: "朝阳区 · 北京市",
    });
    assert.equal(withProvider.sourceKey, "datav");
    assert.equal(withProvider.badgeZh, "DataV");

    const defaultProvider = resolveAdminBoundaryAuthority({
      display_name: "拉萨市 · 西藏自治区",
    });
    assert.equal(defaultProvider.sourceKey, "admin_boundary");
    assert.equal(defaultProvider.badgeZh, "行政区划");

    const osm = resolveAdminBoundaryAuthority({
      provider: "OpenStreetMap",
      display_name: "California",
    });
    assert.equal(osm.sourceKey, "openstreetmap");
    assert.equal(osm.badgeZh, "OpenStreetMap");
  });

  await suite.test("2.2 范围收藏夹支持新增、去重更新、删除与清空", () => {
    clearAoiBookmarks();
    assert.equal(getAoiBookmarks().length, 0);

    const poly = {
      type: "Polygon",
      coordinates: [
        [
          [116.3, 39.8],
          [116.6, 39.8],
          [116.6, 40.1],
          [116.3, 40.1],
          [116.3, 39.8],
        ],
      ],
    };

    const list1 = saveAoiBookmark({
      label: "北京市朝阳区",
      bbox: { west: 116.3, south: 39.8, east: 116.6, north: 40.1 },
      geometry: poly,
    });
    assert.equal(list1.length, 1);
    assert.equal(list1[0].authoritySource, "admin_boundary");
    assert.equal(getAoiBookmarks().length, 1);

    saveAoiBookmark({
      label: "北京市朝阳区",
      bbox: { west: 116.31, south: 39.81, east: 116.61, north: 40.11 },
      geometry: poly,
    });
    assert.equal(getAoiBookmarks().length, 1);
    assert.equal(getAoiBookmarks()[0].bbox.west, 116.31);

    removeAoiBookmark(getAoiBookmarks()[0].id);
    assert.equal(getAoiBookmarks().length, 0);
  });

  await suite.test("2.3 多区域一键合并 (mergeAoiRegions) 正确合并 Polygon / MultiPolygon 与外接包络框", () => {
    assert.equal(mergeAoiRegions([]), null);

    const merged = mergeAoiRegions([
      {
        id: "110105",
        label: "朝阳区 · 北京市 · 区县",
        bbox: { west: 116.35, south: 39.85, east: 116.65, north: 40.05 },
        geometry: {
          type: "Polygon",
          coordinates: [
            [
              [116.35, 39.85],
              [116.65, 39.85],
              [116.65, 40.05],
              [116.35, 40.05],
              [116.35, 39.85],
            ],
          ],
        },
      },
      {
        id: "110108",
        label: "海淀区 · 北京市 · 区县",
        bbox: { west: 116.05, south: 39.9, east: 116.38, north: 40.15 },
        geometry: {
          type: "MultiPolygon",
          coordinates: [
            [
              [
                [116.05, 39.9],
                [116.38, 39.9],
                [116.38, 40.15],
                [116.05, 40.15],
                [116.05, 39.9],
              ],
            ],
          ],
        },
      },
    ]);

    assert.ok(merged);
    assert.equal(merged.regionCount, 2);
    assert.equal(merged.label, "朝阳区 + 海淀区");
    assert.deepEqual(merged.bbox, {
      west: 116.05,
      south: 39.85,
      east: 116.65,
      north: 40.15,
      crs: "EPSG:4326",
    });
    assert.equal(merged.geometry.type, "MultiPolygon");
    assert.equal((merged.geometry.coordinates as unknown[]).length, 2);
  });
});

test("3. 下载文件夹本地目录清单 (catalog.geojson) 与通用数据说明书 (README_数据说明.md) 生成验证", () => {
  const mockTask: ApplicationTask = {
    task_id: "task-s1-beijing-01",
    revision: 3,
    name: "Sentinel-1 北京形变监测序列",
    category: "scene_download",
    status: "completed",
    provider: "ASF Earthdata",
    dataset: "Sentinel-1 SLC",
    created_at: "2026-11-05T00:00:00Z",
    updated_at: "2026-11-05T01:00:00Z",
    done_bytes: 8_589_934_592,
    total_bytes: 8_589_934_592,
    items_count: 2,
    completed_items_count: 2,
    output_dir: "D:/SAR_Data/Beijing_S1",
    scene_items: [
      {
        item_id: "item-1",
        scene_id: "S1A_IW_SLC__1SDV_20261001T101230",
        status: "completed",
        done_bytes: 4_294_967_296,
        total_bytes: 4_294_967_296,
        bbox: [116.1, 39.7, 116.8, 40.2],
      } as any,
      {
        item_id: "item-2",
        scene_id: "S1A_IW_SLC__1SDV_20261013T101230",
        status: "completed",
        done_bytes: 4_294_967_296,
        total_bytes: 4_294_967_296,
      },
    ],
  } as any;

  const geojson = buildDownloadFolderCatalogGeoJson(mockTask);
  assert.equal(geojson.type, "FeatureCollection");
  assert.equal(geojson.features.length, 1);
  assert.equal(geojson.features[0].geometry.type, "Polygon");
  assert.deepEqual(geojson.features[0].geometry.coordinates[0][0], [116.1, 39.7]);
  assert.equal(
    geojson.features[0].properties.relative_file_path,
    "./S1A_IW_SLC__1SDV_20261001T101230.zip"
  );
  assert.equal(geojson.features[0].properties.crs, "EPSG:4326");

  const emptyCoordsTask: any = {
    ...mockTask,
    bbox: undefined,
    scene_items: [{ item_id: "x", scene_id: "s1", status: "completed" }],
  };
  const emptyGeoJson = buildDownloadFolderCatalogGeoJson(emptyCoordsTask);
  assert.equal(emptyGeoJson.features.length, 0);

  const readme = buildDownloadFolderReadmeMarkdown(mockTask);
  assert.ok(readme.includes("# 通用数据说明书"));
  assert.ok(readme.includes("catalog.geojson"));
  assert.ok(readme.includes("QGIS"));
  assert.ok(readme.includes("S1A_IW_SLC__1SDV_20261001T101230"));
  assert.ok(
    readme.includes("## 5. 零密钥可复现 Python 检索脚本 (Reproducible Recipe)"),
    "含有效 bbox 的任务说明书应自动嵌入零密钥可复现 Python 脚本章节"
  );
  assert.ok(readme.includes("from pystac_client import Client"));

  const bundle = buildDownloadFolderCatalogBundle(mockTask);
  assert.equal(bundle.geojsonFileName, "catalog.geojson");
  assert.equal(bundle.readmeFileName, "README_数据说明.md");
  assert.doesNotThrow(() => JSON.parse(bundle.geojsonText));
});

test("4. EOStage 四大核心能力：P1 工具上下文代理、P2 OAuth PKCE 免密、P3 四维空间排序、P4 软取消与安全门禁", async (suite) => {
  await suite.test("4.1 P1: 500+ 工具防爆上下文与按需发现代理 (Fast/Full 模式 + 4 元工具 Broker)", () => {
    const fullTools = getToolsForContext(REGISTERED_AGENT_TOOLS, "full");
    const fastTools = getToolsForContext(REGISTERED_AGENT_TOOLS, "fast");
    assert.equal(fullTools.length, 18);
    assert.equal(fastTools.length, 13);
    assert.equal(
      fastTools.every((t) => t.effect === "read" && t.confirmation === "none"),
      true
    );

    const catalog = summarizeToolCatalog(REGISTERED_AGENT_TOOLS);
    assert.equal(catalog.totalCount, 18);
    assert.ok(catalog.catalogLines.every((line) => line.summary_line.length <= 160));

    const matched = searchRegisteredTools("create_task", { contextMode: "full" });
    assert.equal(matched[0]?.tool_id, "create_task");
    assert.equal(getRegisteredToolInfo("working_aoi_get")?.target_command, "application_working_aoi");
    assert.equal(getRegisteredToolInfo("admin_boundary_search")?.target_command, "application_admin_boundary_search");
    assert.equal(getRegisteredToolInfo("working_aoi_set")?.target_command, "application_set_working_aoi");
    assert.equal(getRegisteredToolInfo("non_existent_tool"), null);

    // Fast 模式下未超出 eagerLimit 时，仍相对全量 18 工具 Schema 节省 >20% Token
    const fastBroker = buildBrokeredAgentTools(REGISTERED_AGENT_TOOLS, {
      contextMode: "fast",
      eagerLimit: 13,
    });
    assert.equal(fastBroker.brokerActive, false);
    assert.ok(
      fastBroker.savedTokensPct >= 20,
      `Fast 模式相对于全量 18 工具应反映真实节省比例 (>20%)，实际 ${fastBroker.savedTokensPct}%`
    );

    // 500+ 扩展工具压测（simulatedExtensionToolCount 与真实 500 项数组双重验证）
    const broker500 = buildBrokeredAgentTools(REGISTERED_AGENT_TOOLS, {
      contextMode: "full",
      eagerLimit: 12,
      loadedToolIds: ["search_start"],
      simulatedExtensionToolCount: 500,
    });
    assert.equal(broker500.brokerActive, true);
    assert.equal(broker500.totalAvailableTools, 518);
    assert.equal(broker500.metaTools.length, 4);
    assert.ok(broker500.eagerSchemaTools.some((t) => t.tool_id === "search_start"));
    assert.ok(broker500.savedTokensPct > 90, `预期节省 >90%，实际 ${broker500.savedTokensPct}%`);

    const real500Tools = Array.from({ length: 500 }, (_, idx) => ({
      ...REGISTERED_AGENT_TOOLS[idx % REGISTERED_AGENT_TOOLS.length],
      tool_id: `whitebox_op_${idx}`,
    }));
    const real500Broker = buildBrokeredAgentTools(real500Tools, {
      contextMode: "full",
      eagerLimit: 12,
    });
    assert.equal(real500Broker.brokerActive, true);
    assert.ok(
      real500Broker.savedTokensPct > 90,
      `真实传入 500 个工具对象时同样应截断内联摘要并节省 >90% Token，实际 ${real500Broker.savedTokensPct}%`
    );
  });

  await suite.test("4.2 P2: 免 API Key 浏览器 OAuth PKCE S256 挑战生成、State 防伪校验与 JWT AccountID 提取", () => {
    // RFC 7636 Appendix B 标准测试向量验证：
    // verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"
    // S256 challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
    const rfcPair = generatePkcePair("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk");
    assert.equal(rfcPair.method, "S256");
    assert.equal(rfcPair.codeChallenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");

    const state = generateOAuthState();
    assert.ok(state.length >= 16);

    const codexPreset = OAUTH_PKCE_PROVIDERS[0];
    const authUrl = buildOAuthAuthorizationUrl({
      provider: codexPreset,
      codeChallenge: rfcPair.codeChallenge,
      state,
    });
    assert.ok(authUrl.startsWith("https://auth.openai.com/oauth/authorize?"));
    assert.ok(authUrl.includes("code_challenge=E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"));
    assert.ok(authUrl.includes("code_challenge_method=S256"));

    // 正常回调与 State 不匹配 / 缺失 State 参数拦截
    const validCb = extractOAuthAuthorizationCode(
      `http://localhost:1455/auth/callback?code=ac_998877&state=${state}`,
      state
    );
    assert.equal(validCb.ok, true);
    assert.equal(validCb.code, "ac_998877");

    const forgedCb = extractOAuthAuthorizationCode(
      "http://localhost:1455/auth/callback?code=ac_998877&state=wrong_state",
      state
    );
    assert.equal(forgedCb.ok, false);
    assert.ok(forgedCb.error?.includes("state"));

    const missingStateCb = extractOAuthAuthorizationCode(
      "http://localhost:1455/auth/callback?code=ac_998877",
      state
    );
    assert.equal(
      missingStateCb.ok,
      false,
      "当提供 expectedState 时，缺少 state 查询参数的回调 URL 必须被作为 CSRF 拒绝"
    );

    // JWT chatgpt_account_id 提取（嵌套 claim、扁平 claim 与 Bearer 前缀）及过期提前量 fail-closed 判定
    const fakePayload = {
      "https://api.openai.com/auth": {
        chatgpt_account_id: "acct_geolibre_001",
      },
    };
    assert.equal(extractChatGptAccountId(fakePayload), "acct_geolibre_001");
    assert.equal(
      extractChatGptAccountId({
        "https://api.openai.com/auth.chatgpt_account_id": "acct_flat_claim_002",
      }),
      "acct_flat_claim_002"
    );
    const encodedPayload = Buffer.from(
      JSON.stringify({
        "https://api.openai.com/auth": {
          chatgpt_account_id: "acct_pkce_999",
        },
      }),
      "utf8"
    ).toString("base64url");
    const bearerJwt = `Bearer eyJhbGciOiJSUzI1NiJ9.${encodedPayload}.sig`;
    assert.equal(extractChatGptAccountId(bearerJwt), "acct_pkce_999");

    assert.equal(isOAuthTokenExpiringSoon(1200, 1000, 300), true);
    assert.equal(isOAuthTokenExpiringSoon(2000, 1000, 300), false);
    assert.equal(isOAuthTokenExpiringSoon(null, 1000, 300), true, "缺失过期时间必须 fail-closed 返回 true");
    assert.equal(isOAuthTokenExpiringSoon(undefined, 1000, 300), true);
    assert.equal(isOAuthTokenExpiringSoon("", 1000, 300), true);
    assert.equal(isOAuthTokenExpiringSoon(0, 1000, 300), true);
  });

  await suite.test("4.3 P3: 多因子四维空间优先景排序 (中心点包含含洞多边形 + 重叠占比 + 云量意图 + 时间)", () => {
    assert.equal(isLowCloudRequest("查找北京无云的 Sentinel-2 影像"), true);
    assert.equal(isLowCloudRequest("find cloud-free optical scenes"), true);
    assert.equal(isLowCloudRequest("查找 Sentinel-1 雷达影像"), false);
    assert.equal(computeCandidatePoolLimit(5, true, 50), 50);
    assert.equal(computeCandidatePoolLimit(5, false, 50), 5);

    // 含内部孔洞 (Hole) 的 Polygon 测试
    const polygonWithHole = {
      type: "Polygon" as const,
      coordinates: [
        [
          [116.0, 39.0],
          [117.0, 39.0],
          [117.0, 40.0],
          [116.0, 40.0],
          [116.0, 39.0],
        ],
        [
          [116.4, 39.4],
          [116.6, 39.4],
          [116.6, 39.6],
          [116.4, 39.6],
          [116.4, 39.4],
        ],
      ],
    };
    assert.equal(geometryContainsPoint(polygonWithHole, [116.2, 39.2]), true);
    assert.equal(geometryContainsPoint(polygonWithHole, [116.5, 39.5]), false, "位于内部孔洞中的点不应判定为包含");

    const queryBbox: [number, number, number, number] = [116.0, 39.0, 117.0, 40.0];
    assert.equal(computeBboxOverlapRatio([116.0, 39.0, 117.0, 40.0], queryBbox), 1);
    assert.equal(computeBboxOverlapRatio([116.0, 39.0, 116.5, 40.0], queryBbox), 0.5);

    const ranked = rankScenesBySpatialFit(
      [
        {
          id: "edge-scene-zero-cloud",
          datetime: "2026-10-20T00:00:00Z",
          cloud_cover: 0,
          bbox: [116.0, 39.0, 116.2, 39.2], // 不包含中心点 [116.5, 39.5]
        },
        {
          id: "center-full-cloudy",
          datetime: "2026-10-25T00:00:00Z",
          properties: { "eo:cloud_cover": 15 },
          bbox: [116.0, 39.0, 117.0, 40.0], // 包含中心点且 100% 覆盖，时间更新但云量较高
        },
        {
          id: "center-full-clear",
          datetime: "2026-10-19T00:00:00Z",
          properties: { "eo:cloud_cover": 2 },
          bbox: [116.0, 39.0, 117.0, 40.0], // 包含中心点、100% 覆盖且低云量
        },
      ],
      { queryBbox, preferLowCloud: true }
    );

    assert.equal(ranked[0].scene.id, "center-full-clear");
    assert.equal(ranked[1].scene.id, "center-full-cloudy");
    assert.equal(ranked[2].scene.id, "edge-scene-zero-cloud");

    // 当 preferLowCloud = false 时，相同空间覆盖下按最新时间 (datetime DESC) 优先于云量排序
    const rankedByDate = rankScenesBySpatialFit(
      [
        {
          id: "older-clear",
          datetime: "2026-10-19T00:00:00Z",
          properties: { "eo:cloud_cover": 2 },
          bbox: [116.0, 39.0, 117.0, 40.0],
        },
        {
          id: "newer-cloudy",
          datetime: "2026-10-25T00:00:00Z",
          properties: { "eo:cloud_cover": 15 },
          bbox: [116.0, 39.0, 117.0, 40.0],
        },
      ],
      { queryBbox, preferLowCloud: false }
    );
    assert.equal(
      rankedByDate[0].scene.id,
      "newer-cloudy",
      "preferLowCloud=false 时，空间契合度相同应优先按最新时间排序"
    );
  });

  await suite.test("4.4 P4: 软取消重规划反馈、长结果截断、PyQGIS/GEE 静态安全门禁与零密钥复现脚本导出", () => {
    const softCancel = buildSoftCancelToolFeedback({
      toolId: "create_task",
      planId: "plan-101",
      reasonZh: "云量过高，希望改为 10% 以下",
    });
    assert.equal(softCancel.status, "cancelled_by_user");
    assert.ok(softCancel.cancel_tool_message.includes("create_task"));
    assert.ok(softCancel.replan_hint_zh.includes("云量过高"));

    const compacted = compactToolResultForContext(
      { items: Array.from({ length: 25 }, (_, i) => i) },
      10
    ) as { items: unknown[] };
    assert.equal(compacted.items.length, 11);
    assert.ok(String(compacted.items[10]).includes("15 more items truncated"));

    // 脚本安全校验：覆盖黑名单、多模块导入、非白名单顶层模块及独立 dunder 反射
    const unsafePy = validateGeoScriptSafety(
      "import subprocess\nsubprocess.Popen('rm -rf /')\nx = eval('1+1')\nobj.__subclasses__()",
      "pyqgis"
    );
    assert.equal(unsafePy.safe, false);
    assert.ok(unsafePy.violations.length >= 3);

    const multiImportAndDunder = validateGeoScriptSafety(
      "import math, subprocess\nimport pathlib\nx = __builtins__['open']('/etc/passwd')",
      "pyqgis"
    );
    assert.equal(multiImportAndDunder.safe, false);
    assert.ok(multiImportAndDunder.violations.some((v) => v.includes("subprocess")));
    assert.ok(multiImportAndDunder.violations.some((v) => v.includes("pathlib")));
    assert.ok(multiImportAndDunder.violations.some((v) => v.includes("__builtins__")));

    const unsafeGee = validateGeoScriptSafety(
      "const info = image.reduceRegion({reducer: ee.Reducer.mean()}).getInfo();",
      "gee"
    );
    assert.equal(unsafeGee.safe, false);
    assert.equal(unsafeGee.violations.length, 2);

    const safePy = validateGeoScriptSafety(
      "from qgis.core import QgsVectorLayer\nimport math\nlayer = QgsVectorLayer('path.geojson', 'aoi', 'ogr')\niface.mapCanvas().refresh()",
      "pyqgis"
    );
    assert.equal(safePy.safe, true);

    const recipe = exportReproduciblePythonRecipe({
      resourceId: "sentinel-2-l2a",
      bbox: [116.2, 39.8, 116.6, 40.1],
      timeRange: { start: "2026-09-01T00:00:00Z", end: "2026-10-01T00:00:00Z" },
      maxCloudCover: 10,
      selectedSceneIds: ["S2A_MSIL2A_20260915"],
      includeOlmoEarthCompanion: true,
    });
    assert.ok(recipe.includes("# EOStage Reproducible Geospatial Recipe"));
    assert.ok(recipe.includes("sentinel-2-l2a"));
    assert.ok(recipe.includes("S2A_MSIL2A_20260915"));
    assert.ok(recipe.includes("eo:cloud_cover"));
    assert.ok(recipe.includes("OlmoEarth Artifact License"));
    assert.ok(recipe.includes("dequantize_olmoearth_int8"));
  });

  await suite.test("4.5 P5: OlmoEarth v1.3 南半球 EPSG:326NN 负值北坐标、Zarr v3 2052B 尾索引、非线性反量化与跨源溯源", () => {
    // 1) 北半球 AOI (北京): EPSG:32650, centerNorthingMeters > 0
    const northPlan = planOlmoEarthUtmShardWindow([116.3, 39.85, 116.45, 39.98], {
      matryoshkaDim: 64,
    });
    assert.equal(northPlan.utmZone, 50);
    assert.equal(northPlan.epsgCode, "EPSG:32650");
    assert.equal(northPlan.hemisphere, "N");
    assert.equal(northPlan.usesNegativeNorthing, false);
    assert.ok(northPlan.centerNorthingMeters > 4_000_000);
    assert.equal(northPlan.channelInnerChunkSlices, 1);
    assert.equal(northPlan.rangeMergePolicy.maximum_gap_bytes, 0);
    assert.equal(northPlan.rangeMergePolicy.shard_tail_index_bytes, 2052);
    assert.ok(northPlan.savedBytesPctVs128D > 49);

    // 2) 南半球 AOI (悉尼): 全球统一使用北半球 EPSG:32656 代码 + 负值北坐标 (y < 0)，绝不切 EPSG:32756
    const southPlan = planOlmoEarthUtmShardWindow([151.15, -33.92, 151.25, -33.84], {
      matryoshkaDim: 128,
    });
    assert.equal(southPlan.utmZone, 56);
    assert.equal(southPlan.epsgCode, "EPSG:32656");
    assert.equal(southPlan.hemisphere, "S");
    assert.equal(southPlan.usesNegativeNorthing, true);
    assert.ok(
      southPlan.centerNorthingMeters < -3_500_000,
      `南半球必须使用负值北坐标 (y < 0)，实际 ${southPlan.centerNorthingMeters}`
    );
    assert.equal(southPlan.channelInnerChunkSlices, 2);
    assert.equal(southPlan.savedBytesPctVs128D, 0);
    assert.equal(
      southPlan.tailIndexBytesTotal,
      southPlan.outerShardCount * OLMOEARTH_V13_SPECS.shardTailIndexBytes
    );

    // 2.1) 跨赤道 AOI (肯尼亚: south = -0.5 < 0, north = +0.8 > 0, centerLat = +0.15 > 0):
    //      中心虽在北半球，但南侧延伸至赤道以南，在 EPSG:32637 下仍涉及负值北坐标 (usesNegativeNorthing = true)
    const equatorStraddlePlan = planOlmoEarthUtmShardWindow([36.0, -0.5, 36.5, 0.8]);
    assert.equal(equatorStraddlePlan.utmZone, 37);
    assert.equal(equatorStraddlePlan.epsgCode, "EPSG:32637");
    assert.equal(equatorStraddlePlan.hemisphere, "N");
    assert.equal(equatorStraddlePlan.usesNegativeNorthing, true);

    // 2.2) 跨国际日期变更线 AOI (斐济: west = 179.5 > east = -179.5, 真实跨度 1.0°):
    //      绝不能算成 centerLon = 0 (EPSG:32631) 或 359° 环球宽度
    const antimeridianPlan = planOlmoEarthUtmShardWindow([179.5, -18.0, -179.5, -17.0]);
    assert.equal(antimeridianPlan.utmZone, 60);
    assert.equal(antimeridianPlan.epsgCode, "EPSG:32660");
    assert.equal(antimeridianPlan.hemisphere, "S");
    assert.equal(antimeridianPlan.usesNegativeNorthing, true);
    assert.ok(
      antimeridianPlan.bboxSpanMeters.widthMeters > 95_000 &&
        antimeridianPlan.bboxSpanMeters.widthMeters < 120_000,
      `跨日界线 1° AOI 宽度应约为 106 km，实际 ${antimeridianPlan.bboxSpanMeters.widthMeters}`
    );
    assert.equal(antimeridianPlan.outerShardCount, 36);

    // 2.3) 异常/非有限坐标与倒置纬度防御：绝不输出 EPSG:326NaN
    const sanitizedPlan = planOlmoEarthUtmShardWindow([Number.NaN, -33.84, 151.25, -33.92]);
    assert.ok(!sanitizedPlan.epsgCode.includes("NaN"));
    assert.ok(Number.isFinite(sanitizedPlan.estimatedTransferBytes));

    // 3) 非线性有符号平方根反量化 ((abs(q) / 127.5) ** 2) * sign(q) 及 NoData = -128 边界验证
    assert.equal(Number.isNaN(dequantizeOlmoEarthInt8(-128)), true, "NoData = -128 必须映射为 NaN");
    assert.equal(Number.isNaN(dequantizeOlmoEarthInt8(-129)), true);
    assert.equal(Number.isNaN(dequantizeOlmoEarthInt8(128)), true);
    assert.equal(Number.isNaN(dequantizeOlmoEarthInt8(Number.NaN)), true);
    assert.equal(dequantizeOlmoEarthInt8(0), 0);

    const pos64 = dequantizeOlmoEarthInt8(64);
    const neg64 = dequantizeOlmoEarthInt8(-64);
    const expected64 = (64 / 127.5) ** 2;
    assert.ok(Math.abs(pos64 - expected64) < 1e-6);
    assert.ok(Math.abs(neg64 + expected64) < 1e-6);
    // 验证绝不等于错误线性缩放 (64 / 127.5 ≈ 0.50196 vs 非线性 ≈ 0.25196)
    assert.ok(Math.abs(pos64 - 64 / 127.5) > 0.2);

    // Matryoshka 64D 向量前缀切片与批量反量化（含非法维度防御）
    const raw128 = Array.from({ length: 128 }, (_, i) => (i === 0 ? -128 : (i % 127) + 1));
    const vec64 = dequantizeOlmoEarthVector(raw128, 64);
    assert.equal(vec64.length, 64);
    assert.equal(Number.isNaN(vec64[0]), true);
    assert.ok(Math.abs(vec64[1] - (2 / 127.5) ** 2) < 1e-6);
    assert.equal(dequantizeOlmoEarthVector(raw128, -5 as any).length, 0);
    assert.equal(dequantizeOlmoEarthVector(raw128, Number.NaN as any).length, 128);

    // 4) 观测级跨源溯源映射与许可证合规声明验证（即使超云量景面积覆盖率更高，也必须优先排入满足 maxCloudCover 的合格低云景）
    const crosswalk = buildOlmoEarthObservationCrosswalk({
      bbox: [151.15, -33.92, 151.25, -33.84],
      timeRange: { start: "2024-01-01T00:00:00Z", end: "2024-12-31T23:59:59Z" },
      maxCloudCover: 12,
      candidateScenes: [
        {
          id: "S2B_MSIL2A_20240710_CLOUDY_100PCT_OVERLAP",
          datetime: "2024-07-10T00:00:00Z",
          cloud_cover: 80,
          bbox: [151.0, -34.0, 151.4, -33.7], // 100% 覆盖 AOI，但云量 80% > 12%
        },
        {
          id: "S2A_MSIL2A_20240615_CLEAR_90PCT_OVERLAP",
          datetime: "2024-06-15T00:00:00Z",
          cloud_cover: 3,
          bbox: [151.16, -33.92, 151.25, -33.84], // 90% 覆盖 AOI 且包含中心，云量 3% <= 12%
        },
      ],
    });
    assert.equal(crosswalk.shardPlan.epsgCode, "EPSG:32656");
    assert.equal(
      crosswalk.rankedLowCloudCandidates[0].scene.id,
      "S2A_MSIL2A_20240615_CLEAR_90PCT_OVERLAP"
    );
    assert.equal(
      crosswalk.rankedLowCloudCandidates[1].scene.id,
      "S2B_MSIL2A_20240710_CLOUDY_100PCT_OVERLAP"
    );
    assert.equal(crosswalk.recommendedRawSearches.length, 3);
    assert.deepEqual(
      crosswalk.recommendedRawSearches.map((r) => r.resourceId),
      [
        "science.sentinel-2.l2a.v1",
        "science.landsat.c2-l2.v1",
        "science.sentinel-1.standard.v1",
      ]
    );
    assert.equal(crosswalk.licenseNoticeZh, OLMOEARTH_ARTIFACT_LICENSE_NOTICE_ZH);
    assert.ok(crosswalk.provenanceWarningZh.includes("scene_id"));

    const companionPy = exportOlmoEarthCompanionPythonRecipe({
      bbox: [151.15, -33.92, 151.25, -33.84],
      year: Number.NaN,
      matryoshkaDim: 64,
    });
    assert.ok(companionPy.includes("EPSG:32656"));
    assert.ok(companionPy.includes("NegativeNorthing=true"));
    assert.ok(companionPy.includes("/v1.3/2024/collection.json"));
    assert.ok(!companionPy.includes("/NaN/"));
    assert.ok(companionPy.includes("((np.abs(q) / 127.5) ** 2) * np.sign(q)"));
  });
});

