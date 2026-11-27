/**
 * agentSearchViewSync.ts
 * Agent 检索完成后同步外壳源视图与地图面板「结果清单」冻结批次纯逻辑模块（W5-8 P2-8 / R4b）
 */

import type { AgentStreamEvent } from "./agentBridgeTypes.ts";
import type { SourceView } from "./types.ts";

/**
 * 从事件流中提取尚未处理过的、已完成的 search_progress 工具回合 call_id 列表。
 * 不修改入参 events 或 seenCallIds。
 */
export function newlySettledSearchCallIds(
  events: ReadonlyArray<AgentStreamEvent>,
  seenCallIds: ReadonlySet<string>
): string[] {
  if (!Array.isArray(events) || events.length === 0) {
    return [];
  }
  const result: string[] = [];
  const added = new Set<string>();

  for (const event of events) {
    if (!event || event.kind !== "tool_round") continue;
    const payload = (event.payload || {}) as Record<string, unknown>;
    if (payload.tool_id !== "search_progress") continue;
    if (payload.status !== "done") continue;

    const resultCount = payload.result_count;
    if (
      typeof resultCount !== "number" ||
      !Number.isInteger(resultCount) ||
      resultCount < 0
    ) {
      continue;
    }

    const rawCallId = payload.call_id;
    if (typeof rawCallId !== "string") continue;
    const callId = rawCallId.trim();
    if (!callId) continue;

    if (seenCallIds.has(callId) || added.has(callId)) {
      continue;
    }
    added.add(callId);
    result.push(callId);
  }

  return result;
}

/**
 * 根据已完成检索回合的 call_id，从同一 run 的 plan_card 步骤 disclosure 中提取目标 resource_id。
 * 若未找到或无 disclosure.resource_id 则返回 null。
 */
export function extractSearchResourceIdFromEvents(
  events: ReadonlyArray<AgentStreamEvent>,
  callId: string
): string | null {
  if (!Array.isArray(events) || events.length === 0 || !callId) {
    return null;
  }

  let targetRunId = "";
  let roundSeq = Number.MAX_SAFE_INTEGER;
  for (const event of events) {
    if (!event || event.kind !== "tool_round") continue;
    const payload = (event.payload || {}) as Record<string, unknown>;
    if (payload.call_id === callId && typeof payload.run_id === "string") {
      targetRunId = payload.run_id;
      if (typeof event.seq === "number") {
        roundSeq = event.seq;
      }
      break;
    }
  }

  let matchedResourceId: string | null = null;
  for (const event of events) {
    if (!event || event.kind !== "plan_card") continue;
    if (typeof event.seq === "number" && event.seq > roundSeq) continue;
    const payload = (event.payload || {}) as Record<string, unknown>;
    if (targetRunId && payload.run_id !== targetRunId) continue;

    const steps = Array.isArray(payload.steps) ? payload.steps : [];
    for (const step of steps) {
      if (!step || typeof step !== "object") continue;
      const stepRec = step as Record<string, unknown>;
      const disclosure = stepRec.disclosure;
      if (disclosure && typeof disclosure === "object") {
        const resId = (disclosure as Record<string, unknown>).resource_id;
        if (typeof resId === "string" && resId.trim().length > 0) {
          matchedResourceId = resId.trim();
        }
      }
    }
  }

  return matchedResourceId;
}

function parseSearchedAtMs(searchedAt: string | null | undefined): number {
  if (typeof searchedAt !== "string" || !searchedAt.trim()) {
    return -1;
  }
  const parsed = Date.parse(searchedAt);
  return Number.isNaN(parsed) ? -1 : parsed;
}

function getSafeRevision(revision: number | null | undefined): number {
  return typeof revision === "number" && Number.isFinite(revision) ? revision : -1;
}

/**
 * 对比检索前后的 SourceView 列表，挑选出因本次 Agent 检索而产生新批次（batch_id 变化或 revision 增大）的源视图。
 * - 无 batch_id 或无变化时返回 null（绝不编造批次）
 * - 若提供了 hintResourceId 且该源视图发生变化，优先返回该视图；否则返回 searched_at 最新的变化视图
 */
export function pickAgentSearchedView(
  prevViews: ReadonlyArray<SourceView>,
  nextViews: ReadonlyArray<SourceView>,
  hintResourceId?: string | null
): SourceView | null {
  if (!Array.isArray(nextViews) || nextViews.length === 0) {
    return null;
  }

  const prevByResource = new Map<string, SourceView>();
  if (Array.isArray(prevViews)) {
    for (const pv of prevViews) {
      if (pv && typeof pv.resource_id === "string") {
        prevByResource.set(pv.resource_id, pv);
      }
    }
  }

  const changedViews: SourceView[] = [];
  for (const next of nextViews) {
    if (!next || typeof next.resource_id !== "string") continue;
    if (typeof next.batch_id !== "string" || next.batch_id.trim().length === 0) {
      continue;
    }

    const prev = prevByResource.get(next.resource_id);
    if (!prev) {
      changedViews.push(next);
      continue;
    }

    const batchChanged = prev.batch_id !== next.batch_id;
    const revIncreased = getSafeRevision(next.revision) > getSafeRevision(prev.revision);
    if (batchChanged || revIncreased) {
      changedViews.push(next);
    }
  }

  if (changedViews.length === 0) {
    return null;
  }

  const trimmedHint =
    typeof hintResourceId === "string" && hintResourceId.trim().length > 0
      ? hintResourceId.trim()
      : null;

  if (trimmedHint) {
    const hinted = changedViews.find((v) => v.resource_id === trimmedHint);
    if (hinted) {
      return hinted;
    }
  }

  let best = changedViews[0];
  let bestTime = parseSearchedAtMs(best.searched_at);

  for (let i = 1; i < changedViews.length; i++) {
    const candidate = changedViews[i];
    const candidateTime = parseSearchedAtMs(candidate.searched_at);
    if (candidateTime >= bestTime) {
      best = candidate;
      bestTime = candidateTime;
    }
  }

  return best;
}
