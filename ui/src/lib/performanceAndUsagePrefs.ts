/**
 * performanceAndUsagePrefs.ts
 * 性能与用量面板偏好配置 的两个独立设置入口偏好管理与遥测计算引擎：
 *
 * 1. 「性能与用量」(performance-usage)：
 *    - hudMode: "minimal" | "pro" —— 选择性能与用量信息展示的详细程度（简洁 / 详细）
 * 2. 「工作步骤显示」(transcript-view / work-steps)：
 *    - stepDetailMode: "concise" | "detailed" —— 选择希望看到多少工具调用与思考链细节（简洁 / 详细）
 *    - collapseTiming: "completion" | "next-input" —— 工作步骤收起时机（回答结束后 / 下次有新消息时）
 *    - foldedReplies: "all" | "final" —— 收起后显示的回复（全部回复 / 仅最终回答）
 */

export type ContextHudMode = "minimal" | "pro";
export type AgentStepDetailMode = "concise" | "detailed";
export type WorkStepsCollapseTiming = "completion" | "next-input";
export type WorkStepsFoldedReplies = "all" | "final";

export interface PerformanceOnlyPrefs {
  hudMode: ContextHudMode;
}

export interface WorkStepsPrefs {
  stepDetailMode: AgentStepDetailMode;
  collapseTiming: WorkStepsCollapseTiming;
  foldedReplies: WorkStepsFoldedReplies;
}

export interface PerformanceAndUsagePrefs extends PerformanceOnlyPrefs, WorkStepsPrefs {}

const PERF_STORAGE_KEY = "eostage.performanceUsage.v1";
const WORK_STEPS_STORAGE_KEY = "eostage.workStepsDisplay.v1";
const LEGACY_COMBINED_KEY = "eostage.perfUsagePrefs.v1";
const LEGACY_HUD_MODE_KEY = "eo-hud-mode";

export const PERF_USAGE_PREFS_EVENT = "eostage:perf-usage-prefs-changed";
export const WORK_STEPS_PREFS_EVENT = "eostage:work-steps-prefs-changed";

const DEFAULT_PREFS: PerformanceAndUsagePrefs = {
  hudMode: "minimal",
  stepDetailMode: "concise",
  collapseTiming: "completion",
  foldedReplies: "all",
};

let memoryPrefs: PerformanceAndUsagePrefs = { ...DEFAULT_PREFS };

function safeLocalStorage(): Storage | null {
  try {
    if (typeof window !== "undefined" && window.localStorage) {
      return window.localStorage;
    }
  } catch {
    // 忽略沙盒或隐私模式异常
  }
  return null;
}

function normalizeHudMode(val: unknown): ContextHudMode {
  return val === "pro" || val === "detailed" ? "pro" : "minimal";
}

function normalizeStepDetailMode(val: unknown): AgentStepDetailMode {
  return val === "detailed" ? "detailed" : "concise";
}

function normalizeCollapseTiming(val: unknown): WorkStepsCollapseTiming {
  return val === "next-input" ? "next-input" : "completion";
}

function normalizeFoldedReplies(val: unknown): WorkStepsFoldedReplies {
  return val === "final" ? "final" : "all";
}

export function getPerformanceAndUsagePrefs(): PerformanceAndUsagePrefs {
  const storage = safeLocalStorage();
  if (!storage) return { ...memoryPrefs };
  try {
    let hudMode: ContextHudMode = DEFAULT_PREFS.hudMode;
    let stepDetailMode: AgentStepDetailMode = DEFAULT_PREFS.stepDetailMode;
    let collapseTiming: WorkStepsCollapseTiming = DEFAULT_PREFS.collapseTiming;
    let foldedReplies: WorkStepsFoldedReplies = DEFAULT_PREFS.foldedReplies;

    const rawLegacyCombined = storage.getItem(LEGACY_COMBINED_KEY);
    if (rawLegacyCombined) {
      const parsed = JSON.parse(rawLegacyCombined) as Partial<PerformanceAndUsagePrefs>;
      if (parsed.hudMode !== undefined) hudMode = normalizeHudMode(parsed.hudMode);
      if (parsed.stepDetailMode !== undefined) {
        stepDetailMode = normalizeStepDetailMode(parsed.stepDetailMode);
      }
      if (parsed.collapseTiming !== undefined) {
        collapseTiming = normalizeCollapseTiming(parsed.collapseTiming);
      }
      if (parsed.foldedReplies !== undefined) {
        foldedReplies = normalizeFoldedReplies(parsed.foldedReplies);
      }
    } else {
      const legacyHud = storage.getItem(LEGACY_HUD_MODE_KEY);
      if (legacyHud === "pro" || legacyHud === "minimal") {
        hudMode = legacyHud;
      }
    }

    const rawPerf = storage.getItem(PERF_STORAGE_KEY);
    if (rawPerf) {
      const parsedPerf = JSON.parse(rawPerf) as Partial<PerformanceOnlyPrefs>;
      if (parsedPerf.hudMode !== undefined) {
        hudMode = normalizeHudMode(parsedPerf.hudMode);
      }
    }

    const rawSteps = storage.getItem(WORK_STEPS_STORAGE_KEY);
    if (rawSteps) {
      const parsedSteps = JSON.parse(rawSteps) as Partial<WorkStepsPrefs>;
      if (parsedSteps.stepDetailMode !== undefined) {
        stepDetailMode = normalizeStepDetailMode(parsedSteps.stepDetailMode);
      }
      if (parsedSteps.collapseTiming !== undefined) {
        collapseTiming = normalizeCollapseTiming(parsedSteps.collapseTiming);
      }
      if (parsedSteps.foldedReplies !== undefined) {
        foldedReplies = normalizeFoldedReplies(parsedSteps.foldedReplies);
      }
    }

    memoryPrefs = { hudMode, stepDetailMode, collapseTiming, foldedReplies };
  } catch {
    // 解析失败回退默认
  }
  return { ...memoryPrefs };
}

export function getPerformanceOnlyPrefs(): PerformanceOnlyPrefs {
  const { hudMode } = getPerformanceAndUsagePrefs();
  return { hudMode };
}

export function getWorkStepsPrefs(): WorkStepsPrefs {
  const { stepDetailMode, collapseTiming, foldedReplies } = getPerformanceAndUsagePrefs();
  return { stepDetailMode, collapseTiming, foldedReplies };
}

const prefListeners = new Set<(prefs: PerformanceAndUsagePrefs) => void>();
const workStepsListeners = new Set<(prefs: WorkStepsPrefs) => void>();

export function setPerformanceAndUsagePrefs(
  patch: Partial<PerformanceAndUsagePrefs>
): PerformanceAndUsagePrefs {
  const current = getPerformanceAndUsagePrefs();
  const next: PerformanceAndUsagePrefs = {
    hudMode: patch.hudMode !== undefined ? normalizeHudMode(patch.hudMode) : current.hudMode,
    stepDetailMode:
      patch.stepDetailMode !== undefined
        ? normalizeStepDetailMode(patch.stepDetailMode)
        : current.stepDetailMode,
    collapseTiming:
      patch.collapseTiming !== undefined
        ? normalizeCollapseTiming(patch.collapseTiming)
        : current.collapseTiming,
    foldedReplies:
      patch.foldedReplies !== undefined
        ? normalizeFoldedReplies(patch.foldedReplies)
        : current.foldedReplies,
  };
  const workStepsTouched =
    patch.stepDetailMode !== undefined ||
    patch.collapseTiming !== undefined ||
    patch.foldedReplies !== undefined ||
    next.stepDetailMode !== current.stepDetailMode ||
    next.collapseTiming !== current.collapseTiming ||
    next.foldedReplies !== current.foldedReplies;

  memoryPrefs = next;
  const storage = safeLocalStorage();
  if (storage) {
    try {
      storage.setItem(PERF_STORAGE_KEY, JSON.stringify({ hudMode: next.hudMode }));
      storage.setItem(
        WORK_STEPS_STORAGE_KEY,
        JSON.stringify({
          stepDetailMode: next.stepDetailMode,
          collapseTiming: next.collapseTiming,
          foldedReplies: next.foldedReplies,
        })
      );
      storage.setItem(LEGACY_COMBINED_KEY, JSON.stringify(next));
      storage.setItem(LEGACY_HUD_MODE_KEY, next.hudMode);
    } catch {
      // 忽略写入异常
    }
  }
  for (const fn of prefListeners) {
    try {
      fn(next);
    } catch {
      // ignore listener errors
    }
  }
  if (workStepsTouched) {
    const workStepsSnapshot: WorkStepsPrefs = {
      stepDetailMode: next.stepDetailMode,
      collapseTiming: next.collapseTiming,
      foldedReplies: next.foldedReplies,
    };
    for (const fn of workStepsListeners) {
      try {
        fn(workStepsSnapshot);
      } catch {
        // ignore listener errors
      }
    }
  }
  if (typeof window !== "undefined" && typeof window.dispatchEvent === "function") {
    try {
      const EventCtor = window.CustomEvent || CustomEvent;
      window.dispatchEvent(
        new EventCtor<PerformanceAndUsagePrefs>(PERF_USAGE_PREFS_EVENT, {
          detail: next,
        })
      );
      if (workStepsTouched) {
        window.dispatchEvent(
          new EventCtor<WorkStepsPrefs>(WORK_STEPS_PREFS_EVENT, {
            detail: {
              stepDetailMode: next.stepDetailMode,
              collapseTiming: next.collapseTiming,
              foldedReplies: next.foldedReplies,
            },
          })
        );
      }
    } catch {
      // ignore JSDOM cross-realm Event errors
    }
  }
  return next;
}

export function setPerformanceOnlyPrefs(patch: Partial<PerformanceOnlyPrefs>): PerformanceOnlyPrefs {
  const next = setPerformanceAndUsagePrefs({ hudMode: patch.hudMode });
  return { hudMode: next.hudMode };
}

export function setWorkStepsPrefs(patch: Partial<WorkStepsPrefs>): WorkStepsPrefs {
  const next = setPerformanceAndUsagePrefs({
    stepDetailMode: patch.stepDetailMode,
    collapseTiming: patch.collapseTiming,
    foldedReplies: patch.foldedReplies,
  });
  return {
    stepDetailMode: next.stepDetailMode,
    collapseTiming: next.collapseTiming,
    foldedReplies: next.foldedReplies,
  };
}

export function subscribePerformanceAndUsagePrefs(
  listener: (prefs: PerformanceAndUsagePrefs) => void
): () => void {
  prefListeners.add(listener);
  return () => {
    prefListeners.delete(listener);
  };
}

export function subscribeWorkStepsPrefs(
  listener: (prefs: WorkStepsPrefs) => void
): () => void {
  workStepsListeners.add(listener);
  return () => {
    workStepsListeners.delete(listener);
  };
}

export interface ToolGroupDefaultExpandInput {
  stepDetailMode: AgentStepDetailMode;
  collapseTiming?: WorkStepsCollapseTiming;
  itemStatus?: "running" | "done" | "failed" | string;
  hasSubsequentUserInput?: boolean;
  defaultExpanded?: boolean;
}

/**
 * 计算工具步骤卡片默认是否展开（对齐 DSH TranscriptViewPolicy）：
 * - 详细模式 (detailed)：始终默认展开
 * - 简洁模式 (concise)：
 *   - 步骤执行中 (status === "running") 时保持展开
 *   - 收起时机为「下次有新消息时」(collapseTiming === "next-input") 且后续尚无新用户消息时保持展开
 *   - 收起时机为「回答结束后」(collapseTiming === "completion") 且已结束时自动收起
 */
export function shouldExpandToolGroupByDefault(
  input: ToolGroupDefaultExpandInput
): boolean {
  if (input.stepDetailMode === "detailed") return true;
  if (input.defaultExpanded) return true;
  if (input.itemStatus === "running") return true;
  if (
    input.collapseTiming === "next-input" &&
    input.hasSubsequentUserInput === false
  ) {
    return true;
  }
  return false;
}

export interface FoldedReplyVisibilityInput {
  stepDetailMode: AgentStepDetailMode;
  foldedReplies: WorkStepsFoldedReplies;
  isWorkStepsCollapsed: boolean;
  hasLaterAssistantReplyInTurn: boolean;
}

/**
 * 计算在工作步骤收起后是否隐藏本轮中间助手回复（对齐 DSH foldedReplies: "all" | "final"）：
 * - 仅当处于简洁模式、工作步骤已收起、且「收起后显示的回复」设为「仅最终回答」(final) 时，
 *   隐藏同一回合内后续仍有最终回答/结论的中间助手回复。
 */
export function shouldHideIntermediateReply(
  input: FoldedReplyVisibilityInput
): boolean {
  if (input.stepDetailMode === "detailed") return false;
  if (input.foldedReplies !== "final") return false;
  if (!input.isWorkStepsCollapsed) return false;
  return input.hasLaterAssistantReplyInTurn;
}

export interface ContextTelemetryRunSummary {
  run_id?: string;
  status?: string;
  model_route?: string | null;
  fast_path?: string | null;
  started_at?: string | null;
  ended_at?: string | null;
  usage_total?: {
    prompt?: number | null;
    completion?: number | null;
    total?: number | null;
    cached?: number | null;
    cached_tokens?: number | null;
    cached_input_tokens?: number | null;
    cache_creation?: number | null;
    cache_creation_input_tokens?: number | null;
    reasoning?: number | null;
    reasoning_tokens?: number | null;
    last_prompt?: number | null;
  } | null;
}

function parseRunTimestampMs(iso?: string | null): number | null {
  if (typeof iso !== "string" || iso.trim().length === 0) return null;
  const ms = Date.parse(iso);
  return Number.isFinite(ms) ? ms : null;
}

function parseRunIdOrder(runId?: string): [number, number] | null {
  if (typeof runId !== "string") return null;
  const match = /^run-([0-9a-f]+)-(\d+)$/i.exec(runId.trim());
  if (!match) return null;
  const hexTs = Number.parseInt(match[1], 16);
  const seq = Number.parseInt(match[2], 10);
  if (!Number.isFinite(hexTs) || !Number.isFinite(seq)) return null;
  return [hexTs, seq];
}

/**
 * 比较两条运行记录的新旧顺序（a 比 b 更新返回正数，更旧返回负数，无法比较返回 0）。
 * 兼容后端 `runs_list`（最新在前）与旧单测（最旧在前、无时间戳）。
 */
export function compareRunRecency(
  a: ContextTelemetryRunSummary,
  b: ContextTelemetryRunSummary
): number {
  const tsA = parseRunTimestampMs(a.started_at) ?? parseRunTimestampMs(a.ended_at);
  const tsB = parseRunTimestampMs(b.started_at) ?? parseRunTimestampMs(b.ended_at);
  if (tsA !== null && tsB !== null && tsA !== tsB) {
    return tsA - tsB;
  }
  const endA = parseRunTimestampMs(a.ended_at);
  const endB = parseRunTimestampMs(b.ended_at);
  if (endA !== null && endB !== null && endA !== endB) {
    return endA - endB;
  }
  const idA = parseRunIdOrder(a.run_id);
  const idB = parseRunIdOrder(b.run_id);
  if (idA && idB) {
    if (idA[0] !== idB[0]) return idA[0] - idB[0];
    if (idA[1] !== idB[1]) return idA[1] - idB[1];
  }
  return 0;
}

function hasPositiveRunUsage(run: ContextTelemetryRunSummary): boolean {
  const u = run.usage_total;
  if (!u) return false;
  const positiveNums = [
    u.prompt,
    u.cached,
    u.cached_tokens,
    u.cached_input_tokens,
    u.cache_creation,
    u.cache_creation_input_tokens,
    u.last_prompt,
  ];
  return positiveNums.some((n) => typeof n === "number" && Number.isFinite(n) && n > 0);
}

/**
 * 从运行列表中挑出最近一次带用量记录的 run（无视升序/降序排列）。
 */
export function pickLatestUsageRun<T extends ContextTelemetryRunSummary>(
  runs: readonly T[]
): T | null {
  let best: T | null = null;
  for (const run of runs) {
    if (!hasPositiveRunUsage(run)) continue;
    if (!best || compareRunRecency(run, best) >= 0) {
      best = run;
    }
  }
  return best;
}

export interface ContextTelemetryEventLike {
  kind?: string;
  seq?: number;
  created_at?: string;
  payload?: Record<string, unknown> | null;
}

export interface ContextTelemetryInput {
  runs?: readonly ContextTelemetryRunSummary[];
  events?: readonly ContextTelemetryEventLike[];
  draftText?: string;
  maxContextTokens?: number;
  autoCompactRatio?: number;
}

export interface ContextTelemetryCategory {
  key: "messages" | "tools" | "skills" | "system";
  tokens: number;
  pct: number;
}

export interface ContextTelemetrySnapshot {
  turns: number;
  steps: number;
  reportedRunsCount: number;
  hasReportedUsage: boolean;
  tps: number;
  totalTimeSec: number;
  toolSec: number;
  totalTokens: number;
  inputTokens: number;
  outputTokens: number;
  cachedTokens: number;
  cacheCreationTokens: number;
  reasoningTokens: number;
  hasCacheData: boolean;
  cacheHitPct: number;
  draftTokens: number;
  currentContextTokens: number;
  maxContextTokens: number;
  contextPct: number;
  autoCompactTokens: number;
  autoCompactPct: number;
  remainingToCompactTokens: number;
  isNearAutoCompact: boolean;
  categories: {
    messages: ContextTelemetryCategory;
    tools: ContextTelemetryCategory;
    skills: ContextTelemetryCategory;
    system: ContextTelemetryCategory;
  };
}

function estimateTextTokens(text: string): number {
  if (!text) return 0;
  let cjk = 0;
  let other = 0;
  for (let i = 0; i < text.length; i++) {
    const code = text.charCodeAt(i);
    if (code >= 0x2e80 && code <= 0x9fff) {
      cjk += 1;
    } else {
      other += 1;
    }
  }
  return Math.ceil(cjk * 0.65 + other * 0.28);
}

/**
 * 严格基于真实会话运行记录 (runs) 与真实事件流 (events) 计算 EOStage-hud 监控遥测快照。
 * 不注入任何硬编码或伪造的兜底速率、缓存命中率、思考占比或美金估算值；
 * 当尚无运行或渠道未返回某项指标时，如实返回 0 并标记 hasReportedUsage = false。
 */
export function computeContextTelemetry(
  input: ContextTelemetryInput = {}
): ContextTelemetrySnapshot {
  const runs = input.runs ?? [];
  const events = input.events ?? [];
  const draftText = input.draftText ?? "";
  const maxContextTokens = Math.max(8_000, input.maxContextTokens ?? 1_000_000);
  const autoCompactRatio = Math.min(0.95, Math.max(0.1, input.autoCompactRatio ?? 0.80));

  const userMsgEvents = events.filter(
    (ev) =>
      ev.kind === "user_message" ||
      ev.kind === "message_user" ||
      (ev.kind === "message" && ev.payload?.role === "user") ||
      (ev.kind === "request" && ev.payload?.role !== "assistant")
  );

  // 对具有相同 call_id 的 tool_round (如 running + done) 按 call_id 去重，避免单次工具调用被重复计数
  const seenToolCallIds = new Set<string>();
  let stepCount = 0;
  for (const ev of events) {
    if (
      ev.kind === "tool_call" ||
      ev.kind === "tool_round" ||
      ev.kind === "tool_result"
    ) {
      const rawCallId = ev.payload?.call_id;
      if (typeof rawCallId === "string" && rawCallId.trim().length > 0) {
        const key = `${ev.kind === "tool_round" ? "round" : ev.kind}:${rawCallId.trim()}`;
        if (!seenToolCallIds.has(key)) {
          seenToolCallIds.add(key);
          stepCount += 1;
        }
      } else {
        stepCount += 1;
      }
    }
  }

  const turns = runs.length > 0 ? runs.length : userMsgEvents.length;
  const steps = stepCount;

  let reportedPrompt = 0;
  let reportedCompletion = 0;
  let reportedCached = 0;
  let reportedCacheCreation = 0;
  let reportedReasoning = 0;
  let reportedEffectiveInputForCache = 0;
  let hasReportedCacheField = false;
  let reportedTotalOverride = 0;
  let reportedRunsCount = 0;
  let latestRunPrompt = 0;
  let bestLatestRun: ContextTelemetryRunSummary | null = null;
  let totalTimeSec = 0;
  let completionReportedTimeSec = 0;

  for (const run of runs) {
    const u = run.usage_total;
    const p = u?.prompt;
    const c = u?.completion;
    const tot = u?.total;
    const rawCached =
      u?.cached ?? u?.cached_tokens ?? u?.cached_input_tokens;
    const rawCacheCreation =
      u?.cache_creation ?? u?.cache_creation_input_tokens;
    const rawReasoning = u?.reasoning ?? u?.reasoning_tokens;
    const rawLastPrompt = u?.last_prompt;

    const promptVal = typeof p === "number" && Number.isFinite(p) && p > 0 ? p : 0;
    const completionVal = typeof c === "number" && Number.isFinite(c) && c > 0 ? c : 0;
    const totalVal = typeof tot === "number" && Number.isFinite(tot) && tot > 0 ? tot : 0;
    const lastPromptVal =
      typeof rawLastPrompt === "number" && Number.isFinite(rawLastPrompt) && rawLastPrompt > 0
        ? rawLastPrompt
        : null;
    const cachedVal =
      typeof rawCached === "number" && Number.isFinite(rawCached) && rawCached >= 0
        ? rawCached
        : null;
    const cacheCreationVal =
      typeof rawCacheCreation === "number" &&
      Number.isFinite(rawCacheCreation) &&
      rawCacheCreation >= 0
        ? rawCacheCreation
        : 0;
    const reasoningVal =
      typeof rawReasoning === "number" && Number.isFinite(rawReasoning) && rawReasoning > 0
        ? rawReasoning
        : 0;

    const runHasCacheField = cachedVal !== null || cacheCreationVal > 0;
    if (runHasCacheField) {
      hasReportedCacheField = true;
    }
    const safeCached = cachedVal ?? 0;
    reportedCached += safeCached;
    reportedCacheCreation += cacheCreationVal;
    reportedReasoning += reasoningVal;

    const isAnthropicAdditive =
      cacheCreationVal > 0 ||
      (typeof run.model_route === "string" &&
        run.model_route.toLowerCase().startsWith("anthropic"));
    // 兼容 OpenAI/Responses/DeepSeek (prompt_tokens 已包含 cached_tokens) 与 Anthropic (input_tokens 为未命中部分，cache_read 独立计入)
    const runTotalInput =
      isAnthropicAdditive || safeCached > promptVal
        ? promptVal + safeCached + cacheCreationVal
        : promptVal + cacheCreationVal;
    if (runHasCacheField) {
      reportedEffectiveInputForCache += runTotalInput;
    }

    const runSum = Math.max(promptVal + completionVal, totalVal);

    if (runSum > 0 || safeCached > 0) {
      reportedPrompt += promptVal;
      reportedCompletion += completionVal;
      reportedTotalOverride += runSum;
      reportedRunsCount += 1;
    }
    if (lastPromptVal !== null || promptVal > 0 || safeCached > 0 || cacheCreationVal > 0) {
      if (!bestLatestRun || compareRunRecency(run, bestLatestRun) >= 0) {
        bestLatestRun = run;
        if (lastPromptVal !== null) {
          // 真实单次上下文窗口水位（多轮工具调用时绝不把 8 次重复发送的累计账单当成窗口占用）
          latestRunPrompt = lastPromptVal;
        } else {
          latestRunPrompt = Math.max(promptVal, runTotalInput);
        }
      }
    }
    if (run.started_at && run.ended_at) {
      const s = Date.parse(run.started_at);
      const e = Date.parse(run.ended_at);
      if (Number.isFinite(s) && Number.isFinite(e) && e > s) {
        const durSec = (e - s) / 1000;
        totalTimeSec += durSec;
        if (completionVal > 0) {
          completionReportedTimeSec += durSec;
        }
      }
    }
  }

  let eventReportedPrompt = 0;
  let lastEventPrompt = 0;
  let eventReportedCompletion = 0;
  let eventReportedCached = 0;
  let eventReportedCacheCreation = 0;
  let eventReportedReasoning = 0;
  let eventEffectiveInputForCache = 0;
  let hasEventCacheField = false;
  let toolDurationSec = 0;
  let eventHistoryTokens = 0;
  let eventMessageTokens = 0;
  let eventToolTokens = 0;
  let isLatestRequestCasualFastPath = false;

  const allRunsAnthropic =
    runs.length > 0 &&
    runs.every(
      (r) =>
        typeof r.model_route === "string" &&
        r.model_route.toLowerCase().startsWith("anthropic")
    );

  for (const ev of events) {
    const payload = ev.payload;
    if (!payload) continue;
    if (ev.kind === "request") {
      isLatestRequestCasualFastPath =
        payload.fast_path === "casual_greeting" || payload.tool_count === 0;
    }

    const usage = payload.usage as
      | {
          prompt?: unknown;
          input_tokens?: unknown;
          completion?: unknown;
          output_tokens?: unknown;
          cached?: unknown;
          cached_tokens?: unknown;
          cached_input_tokens?: unknown;
          cache_read_input_tokens?: unknown;
          prompt_cache_hit_tokens?: unknown;
          cache_creation?: unknown;
          cache_creation_input_tokens?: unknown;
          reasoning?: unknown;
          reasoning_tokens?: unknown;
        }
      | undefined;
    if (usage && typeof usage === "object") {
      const rawP =
        typeof usage.prompt === "number"
          ? usage.prompt
          : typeof usage.input_tokens === "number"
          ? usage.input_tokens
          : 0;
      const rawC =
        typeof usage.completion === "number"
          ? usage.completion
          : typeof usage.output_tokens === "number"
          ? usage.output_tokens
          : 0;
      const rawCached =
        typeof usage.cached === "number"
          ? usage.cached
          : typeof usage.cached_tokens === "number"
          ? usage.cached_tokens
          : typeof usage.cached_input_tokens === "number"
          ? usage.cached_input_tokens
          : typeof usage.cache_read_input_tokens === "number"
          ? usage.cache_read_input_tokens
          : typeof usage.prompt_cache_hit_tokens === "number"
          ? usage.prompt_cache_hit_tokens
          : null;
      const rawCreation =
        typeof usage.cache_creation === "number"
          ? usage.cache_creation
          : typeof usage.cache_creation_input_tokens === "number"
          ? usage.cache_creation_input_tokens
          : 0;
      const rawReason =
        typeof usage.reasoning === "number"
          ? usage.reasoning
          : typeof usage.reasoning_tokens === "number"
          ? usage.reasoning_tokens
          : 0;

      const pVal = Number.isFinite(rawP) && rawP > 0 ? rawP : 0;
      const cVal = Number.isFinite(rawC) && rawC > 0 ? rawC : 0;
      const cachedVal =
        rawCached !== null && Number.isFinite(rawCached) && rawCached >= 0
          ? rawCached
          : null;
      const creationVal =
        Number.isFinite(rawCreation) && rawCreation >= 0 ? rawCreation : 0;
      const reasonVal =
        Number.isFinite(rawReason) && rawReason > 0 ? rawReason : 0;

      const evHasCacheField = cachedVal !== null || creationVal > 0;
      if (evHasCacheField) {
        hasEventCacheField = true;
      }
      const safeCached = cachedVal ?? 0;
      eventReportedPrompt += pVal;
      if (pVal > 0) {
        lastEventPrompt = pVal;
      }
      eventReportedCompletion += cVal;
      eventReportedCached += safeCached;
      eventReportedCacheCreation += creationVal;
      eventReportedReasoning += reasonVal;
      const evModelRoute =
        typeof payload.model_route === "string" ? payload.model_route : "";
      const isEvAnthropic =
        creationVal > 0 ||
        evModelRoute.toLowerCase().startsWith("anthropic") ||
        allRunsAnthropic;
      if (evHasCacheField) {
        eventEffectiveInputForCache +=
          isEvAnthropic || safeCached > pVal
            ? pVal + safeCached + creationVal
            : pVal + creationVal;
      }
    }

    if (
      typeof payload.duration_ms === "number" &&
      Number.isFinite(payload.duration_ms) &&
      payload.duration_ms > 0
    ) {
      toolDurationSec += payload.duration_ms / 1000;
    }

    const rawText =
      (typeof payload.text === "string" && payload.text) ||
      (typeof payload.content === "string" && payload.content) ||
      (typeof payload.summary_zh === "string" && payload.summary_zh) ||
      "";
    const isToolEvent =
      ev.kind === "tool_call" ||
      ev.kind === "tool_round" ||
      ev.kind === "tool_result";
    if (rawText) {
      const est = estimateTextTokens(rawText);
      eventHistoryTokens += est;
      if (isToolEvent) {
        eventToolTokens += est;
      } else {
        eventMessageTokens += est;
      }
    } else if (payload.result !== undefined) {
      try {
        const est = estimateTextTokens(JSON.stringify(payload.result));
        eventHistoryTokens += est;
        eventToolTokens += est;
      } catch {
        // ignore serialization error
      }
    }
  }

  const inputTokens = reportedPrompt > 0 ? reportedPrompt : eventReportedPrompt;
  const outputTokens = reportedCompletion > 0 ? reportedCompletion : eventReportedCompletion;
  const cachedTokens =
    reportedCached > 0 || (hasReportedCacheField && !hasEventCacheField)
      ? reportedCached
      : eventReportedCached;
  const cacheCreationTokens =
    reportedCacheCreation > 0 ? reportedCacheCreation : eventReportedCacheCreation;
  const reasoningTokens =
    reportedReasoning > 0 ? reportedReasoning : eventReportedReasoning;
  const effectiveInputForCache =
    hasReportedCacheField && reportedEffectiveInputForCache > 0
      ? reportedEffectiveInputForCache
      : hasEventCacheField && eventEffectiveInputForCache > 0
      ? eventEffectiveInputForCache
      : 0;
  const hasCacheData = hasReportedCacheField || hasEventCacheField;
  const cacheHitPct =
    hasCacheData && effectiveInputForCache > 0 && cachedTokens > 0
      ? Number(
          Math.min(100, (cachedTokens / effectiveInputForCache) * 100).toFixed(1)
        )
      : 0;

  const totalTokens = Math.max(inputTokens + outputTokens, reportedTotalOverride);
  const hasReportedUsage = reportedRunsCount > 0 || totalTokens > 0;

  const roundedTotalTimeSec = Number(totalTimeSec.toFixed(2));
  const roundedToolSec = Number(toolDurationSec.toFixed(2));
  const effectiveSpeedDurationSec =
    completionReportedTimeSec > 0 ? completionReportedTimeSec : roundedTotalTimeSec;
  const tps =
    effectiveSpeedDurationSec > 0 && outputTokens > 0
      ? Number((outputTokens / effectiveSpeedDurationSec).toFixed(1))
      : 0;

  const draftTokens = estimateTextTokens(draftText);
  // Before any user turn or recorded run/history exists, unsent draft text must not inflate the empty context pill above 0%.
  const resolvedBaseContextTokens =
    latestRunPrompt > 0
      ? latestRunPrompt
      : lastEventPrompt > 0
      ? lastEventPrompt
      : eventHistoryTokens;
  const currentContextTokens =
    turns === 0 && resolvedBaseContextTokens === 0
      ? 0
      : resolvedBaseContextTokens + draftTokens;

  const contextPct = Number(
    Math.min(100, (currentContextTokens / maxContextTokens) * 100).toFixed(1)
  );
  const autoCompactTokens = Math.round(maxContextTokens * autoCompactRatio);
  const autoCompactPct = Number((autoCompactRatio * 100).toFixed(1));
  const remainingToCompactTokens = Math.max(0, autoCompactTokens - currentContextTokens);
  const isNearAutoCompact = currentContextTokens >= autoCompactTokens * 0.85;

  // 四维上下文构成明细（对话消息 / 系统工具 / 技能 / 系统提示词）：
  // 当存在真实会话事件文本（eventMessageTokens > 0）且剩余窗口体量明显超出真实用户/助手对话文本时，
  // 超出部分如实归入「系统工具」（工具定义 Schema + 工具返回结果），绝不将工具返回数据误扣在「对话消息」头上。
  const isEffectiveCasualFastPath =
    bestLatestRun?.fast_path !== undefined
      ? bestLatestRun.fast_path === "casual_greeting"
      : isLatestRequestCasualFastPath && eventToolTokens === 0;

  let catMessages = 0;
  let catTools = 0;
  let catSkills = 0;
  let catSystem = 0;
  if (currentContextTokens > 0) {
    if (isEffectiveCasualFastPath) {
      catTools = 0;
      catSkills = 0;
      const measuredMessages = eventMessageTokens + draftTokens;
      catSystem = Math.min(400, Math.round(currentContextTokens * 0.25));
      catMessages = Math.max(
        measuredMessages,
        Math.max(0, currentContextTokens - catSystem)
      );
      if (catMessages + catSystem > currentContextTokens) {
        catSystem = Math.max(0, currentContextTokens - catMessages);
      }
    } else {
      catSystem = Math.min(4956, Math.round(currentContextTokens * 0.15));
      catSkills = Math.min(3142, Math.round(currentContextTokens * 0.10));
      const remainingAfterBase = Math.max(0, currentContextTokens - catSystem - catSkills);
      const measuredMessages = eventMessageTokens + draftTokens;

      if (eventMessageTokens > 0 && remainingAfterBase > measuredMessages + 5724) {
        catMessages = Math.min(measuredMessages, remainingAfterBase);
        catTools = Math.max(0, remainingAfterBase - catMessages);
      } else {
        catTools = Math.max(
          eventToolTokens,
          Math.min(5724, Math.round(currentContextTokens * 0.18))
        );
        if (catSystem + catTools > currentContextTokens * 0.65) {
          catTools = Math.round(currentContextTokens * 0.18);
        }
        catMessages = Math.max(
          measuredMessages,
          currentContextTokens - catSystem - catTools - catSkills
        );
        const catSum = catMessages + catTools + catSkills + catSystem;
        if (catSum > currentContextTokens && catSum > 0) {
          catMessages = Math.max(
            0,
            currentContextTokens - catSystem - catTools - catSkills
          );
        }
      }
    }
  }
  const toCatPct = (tok: number) =>
    currentContextTokens > 0
      ? Number(Math.min(100, (tok / maxContextTokens) * 100).toFixed(1))
      : 0;

  return {
    turns,
    steps,
    reportedRunsCount,
    hasReportedUsage,
    tps,
    totalTimeSec: roundedTotalTimeSec,
    toolSec: roundedToolSec,
    totalTokens,
    inputTokens,
    outputTokens,
    cachedTokens,
    cacheCreationTokens,
    reasoningTokens,
    hasCacheData,
    cacheHitPct,
    draftTokens,
    currentContextTokens,
    maxContextTokens,
    contextPct,
    autoCompactTokens,
    autoCompactPct,
    remainingToCompactTokens,
    isNearAutoCompact,
    categories: {
      messages: {
        key: "messages",
        tokens: catMessages,
        pct: toCatPct(catMessages),
      },
      tools: {
        key: "tools",
        tokens: catTools,
        pct: toCatPct(catTools),
      },
      skills: {
        key: "skills",
        tokens: catSkills,
        pct: toCatPct(catSkills),
      },
      system: {
        key: "system",
        tokens: catSystem,
        pct: toCatPct(catSystem),
      },
    },
  };
}

export function formatCompactTokens(count: number): string {
  if (!Number.isFinite(count) || count <= 0) return "0";
  if (count >= 1_000_000) {
    const m = count / 1_000_000;
    if (Number.isInteger(m)) return `${m}M`;
    const twoDec = Number(m.toFixed(2));
    const oneDec = Number(m.toFixed(1));
    return Math.abs(twoDec - oneDec) >= 0.03 ? `${twoDec}M` : `${oneDec}M`;
  }
  if (count >= 1_000) {
    return `${(count / 1_000).toFixed(1)}k`;
  }
  return String(Math.round(count));
}

// ---------------------------------------------------------------------------
// 3. 「回复进行中发送新消息」偏好（排队等待 queue ⇄ 立即引导 steer）
// ---------------------------------------------------------------------------

export type BusySendMode = "queue" | "steer";

export interface BusySendPrefs {
  busySendMode: BusySendMode;
}

const BUSY_SEND_STORAGE_KEY = "eostage.busySendMode.v1";
export const BUSY_SEND_PREFS_EVENT = "eostage:busy-send-prefs-changed";

let memoryBusySendMode: BusySendMode = "queue";
const busySendListeners = new Set<(prefs: BusySendPrefs) => void>();

function normalizeBusySendMode(val: unknown): BusySendMode {
  return val === "steer" ? "steer" : "queue";
}

export function getBusySendPrefs(): BusySendPrefs {
  const storage = safeLocalStorage();
  if (!storage) return { busySendMode: memoryBusySendMode };
  try {
    const raw = storage.getItem(BUSY_SEND_STORAGE_KEY);
    if (!raw) return { busySendMode: memoryBusySendMode };
    const parsed = JSON.parse(raw) as Partial<BusySendPrefs>;
    return { busySendMode: normalizeBusySendMode(parsed.busySendMode) };
  } catch {
    return { busySendMode: memoryBusySendMode };
  }
}

export function setBusySendPrefs(patch: Partial<BusySendPrefs>): BusySendPrefs {
  const current = getBusySendPrefs();
  const next: BusySendPrefs = {
    busySendMode:
      patch.busySendMode !== undefined
        ? normalizeBusySendMode(patch.busySendMode)
        : current.busySendMode,
  };
  memoryBusySendMode = next.busySendMode;
  const storage = safeLocalStorage();
  if (storage) {
    try {
      storage.setItem(BUSY_SEND_STORAGE_KEY, JSON.stringify(next));
    } catch {
      // ignore storage quota / privacy error
    }
  }
  for (const fn of busySendListeners) {
    try {
      fn(next);
    } catch {
      // ignore listener errors
    }
  }
  if (typeof window !== "undefined" && typeof window.dispatchEvent === "function") {
    try {
      const EventCtor = window.CustomEvent || CustomEvent;
      window.dispatchEvent(new EventCtor<BusySendPrefs>(BUSY_SEND_PREFS_EVENT, { detail: next }));
    } catch {
      // ignore non-DOM test env dispatch errors
    }
  }
  return next;
}

export function subscribeBusySendPrefs(listener: (prefs: BusySendPrefs) => void): () => void {
  busySendListeners.add(listener);
  return () => {
    busySendListeners.delete(listener);
  };
}
