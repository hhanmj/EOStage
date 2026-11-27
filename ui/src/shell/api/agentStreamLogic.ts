/**
 * Agent 事件流与真实会话环纯函数逻辑模块
 * 规范：v3-contracts.ts、EOStage Agent UI 规范
 */

import type {
  AgentStreamEvent,
  AgentStreamEventKind,
  AgentMessagePayload,
  AgentMessageRole,
  AgentToolRoundPayload,
  AgentToolRoundStatus,
  AgentPlanCard,
  AgentPlanCardStep,
  AgentConfirmCard,
  AgentCardSnapshot,
  AgentCardRange,
  AgentProgressCard,
  AgentConclusion,
  AgentErrorPayload,
  AgentSendContext,
  AgentRunStatus,
  AgentChannelView,
  AgentStopAllResult,
  AgentCardApproval,
  AgentThinkingStatus,
  AgentQuestionOption,
  AgentWorkspaceSyncAction,
} from "../../lib/generated/v3-contracts.ts";
import { t } from "../../i18n/index.ts";
import { normalizeCardApproval } from "./approvalLogic.ts";
import { localMarkersToZh } from "./modelTextMarkers.ts";
import { splitInlineThink } from "./agentThinkSplitLogic.ts";

export type StreamMessageItem = {
  kind: "message";
  seq: number;
  run_id: string;
  role: AgentMessageRole;
  text: string;
  context: AgentSendContext | null;
  thinking_text?: string | null;
  thinking_ms?: number | null;
};

export type StreamToolGroupItem = {
  kind: "tool_group";
  seq: number;
  run_id: string;
  status: AgentToolRoundStatus;
  rounds: AgentToolRoundPayload[];
};

export type StreamPlanCardItem = {
  kind: "plan_card";
  seq: number;
  run_id: string;
  plan_id: string;
  plan_hash: string;
  max_tier: number;
  steps: AgentPlanCardStep[];
  approval?: AgentCardApproval | null;
  auto_executed?: boolean;
};

export type StreamConfirmCardItem = {
  kind: "confirm_card";
  seq: number;
  run_id: string;
  plan_id: string;
  plan_hash: string;
  snapshot: AgentCardSnapshot;
  approval?: AgentCardApproval | null;
  auto_executed?: boolean;
};

export type StreamProgressCardItem = {
  kind: "progress_card";
  seq: number;
  run_id: string;
  plan_id: string;
  step_index: number | null;
  tool_id: string | null;
  status: string;
  code: string | null;
  created_task_ids?: string[];
};

export type StreamConclusionItem = {
  kind: "conclusion";
  seq: number;
  run_id: string;
  text: string;
  claims: AgentConclusion["claims"];
  reasoning_off_refused: boolean;
  thinking_text?: string | null;
  thinking_ms?: number | null;
};

export type StreamErrorItem = {
  kind: "error";
  seq: number;
  run_id: string;
  code: string;
  status: string | null;
  plan_id: string | null;
  approval?: AgentCardApproval | null;
  /** 主机自己结束运行时的中文原因（契约 3.33.0 AgentErrorPayload.message_zh）。 */
  message_zh?: string | null;
};

export type StreamStopAllItem = {
  kind: "stop_all";
  seq: number;
  payload: AgentStopAllResult;
};

export type StreamThinkingItem = {
  kind: "thinking";
  seq: number;
  run_id: string;
  step_index: number;
  status: AgentThinkingStatus;
  text: string;
  started_at_ms: number | null;
  thinking_ms: number | null;
};

export type StreamQuestionCardItem = {
  kind: "question_card";
  seq: number;
  run_id: string;
  question_zh: string;
  options: AgentQuestionOption[];
};

export type StreamWorkspaceSyncItem = {
  kind: "workspace_sync";
  seq: number;
  run_id: string;
  action: AgentWorkspaceSyncAction;
  scene_id: string | null;
  enabled: boolean | null;
  layer_key: string | null;
  target: string | null;
  tab: string | null;
  batch_id: string | null;
  display_name: string | null;
  bbox: [number, number, number, number] | null;
};

export type StreamGeeArtifactItem = {
  kind: "gee_artifact";
  seq: number;
  run_id: string;
  call_id: string;
  tool_id: string;
  tool_args: Record<string, any>;
  summary: Record<string, any>;
  script_preview?: string | null;
};

export type StreamItem =
  | StreamMessageItem
  | StreamToolGroupItem
  | StreamPlanCardItem
  | StreamConfirmCardItem
  | StreamProgressCardItem
  | StreamConclusionItem
  | StreamErrorItem
  | StreamStopAllItem
  | StreamThinkingItem
  | StreamQuestionCardItem
  | StreamWorkspaceSyncItem
  | StreamGeeArtifactItem;

/**
 * 格式化字节大小（1024 进制）
 */
export function formatBytes(bytes: number): string {
  if (isNaN(bytes) || bytes < 0) return "0 B";
  if (bytes === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB", "PB"];
  const i = Math.floor(Math.log(bytes) / Math.log(1024));
  const safeI = Math.min(i, units.length - 1);
  const val = bytes / Math.pow(1024, safeI);
  return `${val >= 10 || safeI === 0 ? Math.round(val) : val.toFixed(1)} ${units[safeI]}`;
}

/**
 * D2-1: 分类与归并事件流
 * - message：user 右侧气泡 / assistant 正文
 * - 连续同 run 的 tool_round 归并为一组折叠组（同 call_id 更新最新状态，不同追加；包含 running 组为 running，否则任一 failed 为 failed，全部 done 为 done）
 * - plan_card / confirm_card / progress_card / conclusion / error 各自成卡
 */
export function classifyStreamEvents(events: AgentStreamEvent[]): StreamItem[] {
  if (!Array.isArray(events)) return [];

  const items: StreamItem[] = [];

  for (const event of events) {
    if (!event || typeof event !== "object") continue;
    const kind = event.kind as AgentStreamEventKind | "gee_artifact";
    const seq = typeof event.seq === "number" ? event.seq : 0;
    const payload = (event.payload || {}) as Record<string, any>;

    switch (kind) {
      case "message": {
        const role = payload.role === "user" ? "user" : "assistant";
        const rawText = String(payload.text || "");
        const normalizedText = role === "assistant" ? localMarkersToZh(rawText) : rawText;
        const split =
          role === "assistant"
            ? splitInlineThink(normalizedText)
            : { thinkingText: "", answerMarkdown: normalizedText };
        const explicitThinking =
          typeof payload.thinking_text === "string" && payload.thinking_text.trim().length > 0
            ? payload.thinking_text.trim()
            : null;
        const mergedThinking =
          role === "assistant"
            ? explicitThinking || (split.thinkingText ? split.thinkingText : null)
            : null;
        const thinkingMs =
          role === "assistant" &&
          typeof payload.thinking_ms === "number" &&
          Number.isFinite(payload.thinking_ms) &&
          payload.thinking_ms >= 0
            ? payload.thinking_ms
            : null;
        items.push({
          kind: "message",
          seq,
          run_id: String(payload.run_id || ""),
          role,
          text: split.answerMarkdown,
          context: payload.context || null,
          ...(mergedThinking !== null ? { thinking_text: mergedThinking } : {}),
          ...(thinkingMs !== null ? { thinking_ms: thinkingMs } : {}),
        });
        break;
      }

      case "tool_round": {
        const runId = String(payload.run_id || "");
        const callId = String(payload.call_id || "");
        const toolId = String(payload.tool_id || "");
        const status = (payload.status || "running") as AgentToolRoundStatus;

        const started_at =
          typeof payload.started_at === "string" && payload.started_at.trim().length > 0
            ? payload.started_at
            : undefined;
        const duration_ms =
          typeof payload.duration_ms === "number" &&
          Number.isFinite(payload.duration_ms) &&
          payload.duration_ms >= 0
            ? payload.duration_ms
            : undefined;
        const result_count =
          typeof payload.result_count === "number" &&
          Number.isInteger(payload.result_count) &&
          payload.result_count >= 0
            ? payload.result_count
            : undefined;
        const summary_zh =
          typeof payload.summary_zh === "string" && payload.summary_zh.trim().length > 0
            ? payload.summary_zh
            : undefined;
        const target_label =
          typeof payload.target_label === "string" && payload.target_label.trim().length > 0
            ? payload.target_label.trim()
            : undefined;
        const input_summary =
          typeof payload.input_summary === "string" && payload.input_summary.trim().length > 0
            ? payload.input_summary.trim()
            : undefined;
        const output_summary =
          typeof payload.output_summary === "string" && payload.output_summary.trim().length > 0
            ? payload.output_summary.trim()
            : undefined;

        const roundPayload: AgentToolRoundPayload = {
          run_id: runId,
          call_id: callId,
          tool_id: toolId,
          status,
          ...(started_at !== undefined ? { started_at } : {}),
          ...(duration_ms !== undefined ? { duration_ms } : {}),
          ...(result_count !== undefined ? { result_count } : {}),
          ...(summary_zh !== undefined ? { summary_zh } : {}),
          ...(target_label !== undefined ? { target_label } : {}),
          ...(input_summary !== undefined ? { input_summary } : {}),
          ...(output_summary !== undefined ? { output_summary } : {}),
        };

        const recomputeGroupStatus = (group: StreamToolGroupItem) => {
          const hasRunning = group.rounds.some((r) => r.status === "running");
          const hasFailed = group.rounds.some((r) => r.status === "failed");
          if (hasRunning) {
            group.status = "running";
          } else if (hasFailed) {
            group.status = "failed";
          } else {
            group.status = "done";
          }
        };

        const existingGroupForCall = callId
          ? items.find(
              (it): it is StreamToolGroupItem =>
                it.kind === "tool_group" &&
                it.run_id === runId &&
                it.rounds.some((r) => r.call_id === callId)
            )
          : undefined;

        const lastItem = items[items.length - 1];
        if (existingGroupForCall) {
          const existingCallIdx = existingGroupForCall.rounds.findIndex(
            (r) => r.call_id === callId
          );
          if (existingCallIdx >= 0) {
            const prevRound = existingGroupForCall.rounds[existingCallIdx];
            existingGroupForCall.rounds[existingCallIdx] = {
              ...prevRound,
              ...roundPayload,
              ...(roundPayload.target_label === undefined && prevRound.target_label
                ? { target_label: prevRound.target_label }
                : {}),
              ...(roundPayload.input_summary === undefined && prevRound.input_summary
                ? { input_summary: prevRound.input_summary }
                : {}),
              ...(roundPayload.output_summary === undefined && prevRound.output_summary
                ? { output_summary: prevRound.output_summary }
                : {}),
            };
          }
          recomputeGroupStatus(existingGroupForCall);
        } else if (lastItem && lastItem.kind === "tool_group" && lastItem.run_id === runId) {
          // 同 run 连续 tool_round 归并
          lastItem.rounds.push(roundPayload);
          recomputeGroupStatus(lastItem);
        } else {
          // 开启新工具执行组
          items.push({
            kind: "tool_group",
            seq,
            run_id: runId,
            status,
            rounds: [roundPayload],
          });
        }
        break;
      }

      case "plan_card": {
        const approval =
          payload.approval !== undefined
            ? normalizeCardApproval(payload.approval)
            : undefined;
        const auto_executed = payload.auto_executed === true;
        items.push({
          kind: "plan_card",
          seq,
          run_id: String(payload.run_id || ""),
          plan_id: String(payload.plan_id || ""),
          plan_hash: String(payload.plan_hash || ""),
          max_tier: typeof payload.max_tier === "number" ? payload.max_tier : 0,
          steps: Array.isArray(payload.steps) ? payload.steps : [],
          approval,
          auto_executed,
        });
        break;
      }

      case "confirm_card": {
        const snapshot = (payload.snapshot || {}) as AgentCardSnapshot;
        const approval =
          payload.approval !== undefined
            ? normalizeCardApproval(payload.approval)
            : undefined;
        const auto_executed = payload.auto_executed === true;
        items.push({
          kind: "confirm_card",
          seq,
          run_id: String(payload.run_id || ""),
          plan_id: String(payload.plan_id || ""),
          plan_hash: String(payload.plan_hash || ""),
          snapshot,
          approval,
          auto_executed,
        });
        break;
      }

      case "progress_card": {
        let created_task_ids: string[] | undefined = undefined;
        if (Array.isArray(payload.created_task_ids)) {
          created_task_ids = payload.created_task_ids.filter(
            (id: unknown): id is string => typeof id === "string"
          );
        }
        items.push({
          kind: "progress_card",
          seq,
          run_id: String(payload.run_id || ""),
          plan_id: String(payload.plan_id || ""),
          step_index: typeof payload.step_index === "number" ? payload.step_index : null,
          tool_id: payload.tool_id ? String(payload.tool_id) : null,
          status: String(payload.status || ""),
          code: payload.code ? String(payload.code) : null,
          ...(created_task_ids !== undefined ? { created_task_ids } : {}),
        });
        break;
      }

      case "conclusion": {
        const normalizedConclusion = localMarkersToZh(String(payload.text || ""));
        const splitConclusion = splitInlineThink(normalizedConclusion);
        const explicitThinking =
          typeof payload.thinking_text === "string" && payload.thinking_text.trim().length > 0
            ? payload.thinking_text.trim()
            : null;
        const mergedThinking =
          explicitThinking || (splitConclusion.thinkingText ? splitConclusion.thinkingText : null);
        const thinkingMs =
          typeof payload.thinking_ms === "number" &&
          Number.isFinite(payload.thinking_ms) &&
          payload.thinking_ms >= 0
            ? payload.thinking_ms
            : null;
        items.push({
          kind: "conclusion",
          seq,
          run_id: String(payload.run_id || ""),
          text: splitConclusion.answerMarkdown,
          claims: Array.isArray(payload.claims) ? payload.claims : [],
          reasoning_off_refused: payload.reasoning_off_refused === true,
          ...(mergedThinking !== null ? { thinking_text: mergedThinking } : {}),
          ...(thinkingMs !== null ? { thinking_ms: thinkingMs } : {}),
        });
        break;
      }

      case "thinking": {
        const runId = String(payload.run_id || "");
        const stepIndex =
          typeof payload.step_index === "number" && Number.isInteger(payload.step_index) && payload.step_index >= 0
            ? payload.step_index
            : 0;
        const status: AgentThinkingStatus = payload.status === "done" ? "done" : "thinking";
        const rawThinkingText = typeof payload.text === "string" ? payload.text : "";
        const startedAtMs =
          typeof payload.started_at_ms === "number" && Number.isFinite(payload.started_at_ms) && payload.started_at_ms >= 0
            ? payload.started_at_ms
            : null;
        const thinkingMs =
          typeof payload.thinking_ms === "number" && Number.isFinite(payload.thinking_ms) && payload.thinking_ms >= 0
            ? payload.thinking_ms
            : null;

        const existingIdx = items.findIndex(
          (it): it is StreamThinkingItem =>
            it.kind === "thinking" && it.run_id === runId && it.step_index === stepIndex
        );
        if (existingIdx >= 0) {
          const prev = items[existingIdx] as StreamThinkingItem;
          items[existingIdx] = {
            ...prev,
            status,
            text: rawThinkingText.length > 0 ? rawThinkingText : prev.text,
            started_at_ms: startedAtMs ?? prev.started_at_ms,
            thinking_ms: thinkingMs ?? prev.thinking_ms,
          };
        } else {
          items.push({
            kind: "thinking",
            seq,
            run_id: runId,
            step_index: stepIndex,
            status,
            text: rawThinkingText,
            started_at_ms: startedAtMs,
            thinking_ms: thinkingMs,
          });
        }
        break;
      }

      case "question_card": {
        const rawOptions = Array.isArray(payload.options) ? payload.options : [];
        const options: AgentQuestionOption[] = rawOptions
          .filter((o: unknown): o is Record<string, unknown> => Boolean(o && typeof o === "object"))
          .map((o) => ({
            label_zh: String(o.label_zh || o.value || ""),
            value: String(o.value || o.label_zh || ""),
            ...(typeof o.recommended === "boolean" ? { recommended: o.recommended } : {}),
          }))
          .filter((o) => o.label_zh.length > 0 || o.value.length > 0);

        items.push({
          kind: "question_card",
          seq,
          run_id: String(payload.run_id || ""),
          question_zh: String(payload.question_zh || ""),
          options,
        });
        break;
      }

      case "workspace_sync": {
        const rawBbox = payload.bbox;
        let bbox: [number, number, number, number] | null = null;
        if (
          Array.isArray(rawBbox) &&
          rawBbox.length === 4 &&
          rawBbox.every((v: unknown) => typeof v === "number" && Number.isFinite(v))
        ) {
          bbox = [rawBbox[0], rawBbox[1], rawBbox[2], rawBbox[3]];
        }
        items.push({
          kind: "workspace_sync",
          seq,
          run_id: String(payload.run_id || ""),
          action: (payload.action || "aoi_changed") as AgentWorkspaceSyncAction,
          scene_id: typeof payload.scene_id === "string" ? payload.scene_id : null,
          enabled: typeof payload.enabled === "boolean" ? payload.enabled : null,
          layer_key: typeof payload.layer_key === "string" ? payload.layer_key : null,
          target: typeof payload.target === "string" ? payload.target : null,
          tab: typeof payload.tab === "string" ? payload.tab : null,
          batch_id: typeof payload.batch_id === "string" ? payload.batch_id : null,
          display_name: typeof payload.display_name === "string" ? payload.display_name : null,
          bbox,
        });
        break;
      }

      case "error": {
        const approval =
          payload.approval !== undefined
            ? normalizeCardApproval(payload.approval)
            : undefined;
        items.push({
          kind: "error",
          seq,
          run_id: String(payload.run_id || ""),
          code: String(payload.code || ""),
          status: payload.status ? String(payload.status) : null,
          plan_id: payload.plan_id ? String(payload.plan_id) : null,
          approval,
          message_zh:
            typeof payload.message_zh === "string" && payload.message_zh.trim()
              ? payload.message_zh
              : null,
        });
        break;
      }

      case "stop_all": {
        items.push({
          kind: "stop_all",
          seq,
          payload: {
            cancelled_runs: Array.isArray(payload.cancelled_runs) ? payload.cancelled_runs : [],
            failed_runs: Array.isArray(payload.failed_runs) ? payload.failed_runs : [],
            cancelled_tasks: Array.isArray(payload.cancelled_tasks) ? payload.cancelled_tasks : [],
            failed_tasks: Array.isArray(payload.failed_tasks) ? payload.failed_tasks : [],
          },
        });
        break;
      }

      case "gee_artifact": {
        items.push({
          kind: "gee_artifact",
          seq,
          run_id: String(payload.run_id || ""),
          call_id: String(payload.call_id || ""),
          tool_id: String(payload.tool_id || ""),
          tool_args:
            payload.tool_args && typeof payload.tool_args === "object"
              ? payload.tool_args
              : {},
          summary:
            payload.summary && typeof payload.summary === "object"
              ? payload.summary
              : {},
          script_preview:
            typeof payload.script_preview === "string" ? payload.script_preview : null,
        });
        break;
      }

      default: {
        // 未知或保留类型，不抛错，静默忽略
        break;
      }
    }
  }

  return items;
}

export type ExtractedConfirmSnapshot = {
  source: string;
  range: {
    aoi_name: string;
    area_km2: number | null;
    time_range: { start: string | null; end: string | null } | null;
    time_range_zh: string;
    product: string;
  };
  estimated_bytes: number | null;
  estimated_bytes_zh: string;
  output_dir: string;
  raw_snapshot: unknown;
};

/**
 * D3-1: 安全提取确认卡 snapshot 四项
 * 绝不编造任何科学值，缺失/null 显式显示为「未知」，预计大小显式带「估算」
 */
export function confirmSnapshotOf(eventOrPayload: unknown): ExtractedConfirmSnapshot {
  if (!eventOrPayload || typeof eventOrPayload !== "object") {
    return {
      source: t("agent.unknown"),
      range: {
        aoi_name: t("agent.unknown"),
        area_km2: null,
        time_range: null,
        time_range_zh: t("agent.unknown"),
        product: t("agent.unknown"),
      },
      estimated_bytes: null,
      estimated_bytes_zh: t("agent.unknown"),
      output_dir: t("agent.unknown"),
      raw_snapshot: null,
    };
  }

  const p = ("payload" in (eventOrPayload as any)
    ? (eventOrPayload as any).payload
    : eventOrPayload) as Record<string, any> || {};

  const rawSnapshot = ("snapshot" in p ? p.snapshot : p) as Record<string, any> || null;
  const s = rawSnapshot || {};

  const source =
    typeof s.source === "string" && s.source.trim().length > 0 ? s.source.trim() : t("agent.unknown");

  const rangeObj = (s.range || {}) as Record<string, any>;
  const aoiName =
    typeof rangeObj.aoi_name === "string" && rangeObj.aoi_name.trim().length > 0
      ? rangeObj.aoi_name.trim()
      : t("agent.unknown");

  const areaKm2 =
    typeof rangeObj.area_km2 === "number" && !isNaN(rangeObj.area_km2)
      ? rangeObj.area_km2
      : null;

  const timeRangeObj = rangeObj.time_range;
  let timeRange: { start: string | null; end: string | null } | null = null;
  let timeRangeZh = t("agent.unknown");

  if (timeRangeObj && typeof timeRangeObj === "object") {
    const start = timeRangeObj.start ? String(timeRangeObj.start) : null;
    const end = timeRangeObj.end ? String(timeRangeObj.end) : null;
    timeRange = { start, end };
    if (start || end) {
      timeRangeZh = `${start || t("agent.unknown")} ~ ${end || t("agent.unknown")}`;
    }
  }

  const product =
    typeof rangeObj.product === "string" && rangeObj.product.trim().length > 0
      ? rangeObj.product.trim()
      : t("agent.unknown");

  const estimatedBytes =
    typeof s.estimated_bytes === "number" && !isNaN(s.estimated_bytes)
      ? s.estimated_bytes
      : null;

  let estimatedBytesZh = t("agent.unknown");
  if (estimatedBytes !== null) {
    estimatedBytesZh = t("agent.conversation.estimatedBytes", { size: formatBytes(estimatedBytes) });
  }

  const outputDir =
    typeof s.output_dir === "string" && s.output_dir.trim().length > 0
      ? s.output_dir.trim()
      : t("agent.unknown");

  return {
    source,
    range: {
      aoi_name: aoiName,
      area_km2: areaKm2,
      time_range: timeRange,
      time_range_zh: timeRangeZh,
      product,
    },
    estimated_bytes: estimatedBytes,
    estimated_bytes_zh: estimatedBytesZh,
    output_dir: outputDir,
    raw_snapshot: rawSnapshot,
  };
}

/**
 * D-extra-2: 构造 SendContext 载荷；可选模型选择由发送方按最新渠道校验。
 */
export function buildSendContext(
  aoi: { label?: string; name?: string; geometry: unknown } | null,
  timeRange: { start: string | null; end: string | null } | null,
  source: string | null,
  model?: AgentSendContext["model"]
): AgentSendContext {
  return {
    aoi:
      aoi && aoi.geometry
        ? {
            name: aoi.label || aoi.name || "工作区", // i18n-exempt: model-facing send-context content remains unchanged
            geometry: aoi.geometry,
          }
        : null,
    time_range: timeRange
      ? {
          start: timeRange.start ?? null,
          end: timeRange.end ?? null,
        }
      : null,
    source: source ?? null,
    ...(model ? { model } : {}),
  };
}

/**
 * D4-1: Run 运行状态标签转换
 */
export function runStatusLabel(status: AgentRunStatus | string): string {
  switch (status) {
    case "running":
      return t("agent.status.running");
    case "completed":
      return t("agent.status.completed");
    case "failed":
      return t("agent.status.failed");
    case "cancelled":
      return t("agent.status.cancelled");
    case "interrupted":
      return t("agent.status.interrupted");
    default:
      return status ? String(status) : t("agent.unknown");
  }
}

/**
 * D4-1: 取消/中断文案区分（两者绝不混用）
 */
export function errorCancelledWording(status: string | null): string {
  if (status === "interrupted") {
    return t("agent.status.interrupted");
  }
  return t("agent.status.cancelled");
}

export type ChannelReadinessResult = {
  ready: boolean;
  disabled: boolean;
  reason_zh: string;
};

/**
 * D-extra-1: 模型渠道就绪判定
 * - 存在 is_default 且 key_ready 的渠道 → 就绪
 * - 没有 default 但存在任一 key_ready 渠道 → 就绪（后端 select_for_run 会选第一个有 Key 的）
 * - 否则禁用 + 一行提示 + 「前往设置 → 模型渠道」
 */
export function channelReadinessFromViews(views: AgentChannelView[]): ChannelReadinessResult {
  if (!Array.isArray(views) || views.length === 0) {
    return {
      ready: false,
      disabled: true,
      reason_zh: t("agent.channels.unconfigured"),
    };
  }

  const defaultChannel = views.find((v) => v.is_default);
  if (defaultChannel) {
    if (defaultChannel.key_ready) {
      return {
        ready: true,
        disabled: false,
        reason_zh: "",
      };
    } else {
      return {
        ready: false,
        disabled: true,
        reason_zh: t("agent.channels.defaultMissingKey"),
      };
    }
  }

  const hasAnyKeyReady = views.some((v) => v.key_ready);
  if (hasAnyKeyReady) {
    return {
      ready: true,
      disabled: false,
      reason_zh: "",
    };
  }

  return {
    ready: false,
    disabled: true,
    reason_zh: t("agent.channels.unconfigured"),
  };
}
