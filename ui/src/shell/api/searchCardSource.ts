import { t } from "../../i18n/index.ts";
import type {
  StreamConfirmCardItem,
  StreamItem,
  StreamToolGroupItem,
} from "./agentStreamLogic.ts";
import { formatOrbitTerms } from "./orbitTerms.ts";
import { mappedSourceName } from "./sourceDisplayName.ts";

export interface SearchCardContext {
  source: string | null;
  conditionBadges: string[];
}

const SEARCH_ROUND_TOOL_IDS: ReadonlySet<string> = new Set(["search_start", "search_progress"]);

/** 只有含检索回合的工具轮才渲染「检索」卡；只读结果页（batch_page/workbench_page）等不算。 */
export function isSearchToolGroup(rounds: StreamToolGroupItem["rounds"]): boolean {
  return (rounds || []).some((r) => SEARCH_ROUND_TOOL_IDS.has(r.tool_id));
}

function nonEmpty(value: unknown): string | null {
  if (typeof value !== "string") return null;
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
}

export function dateOnly(value: unknown): string | null {
  const text = nonEmpty(value);
  if (text === null) return null;
  const match = /^(\d{4}-\d{2}-\d{2})/.exec(text);
  return match ? match[1] : null;
}

function timeRangeBadge(range: unknown): string | null {
  if (range === null || typeof range !== "object") return null;
  const obj = range as Record<string, unknown>;
  const start = dateOnly(obj.start);
  const end = dateOnly(obj.end);
  if (start && end) return t("shared.searchCardSource.timeRange", { start, end });
  if (start) return t("shared.searchCardSource.timeFrom", { start });
  if (end) return t("shared.searchCardSource.timeUntil", { end });
  return null;
}

function badgesOf(aoiName: unknown, timeRange: unknown): string[] {
  const badges: string[] = [];
  const aoi = nonEmpty(aoiName);
  if (aoi !== null) badges.push(aoi);
  const time = timeRangeBadge(timeRange);
  if (time !== null) badges.push(time);
  return badges.map(formatOrbitTerms);
}

/**
 * 检索卡的来源与条件：优先取同一 run 计划卡 search_start 步骤的后端披露；
 * 没有披露时退回同一 run 确认卡快照的范围（只取 AOI 与时间，选择计数不是检索条件）；
 * 都没有则留空。来源只用显示名或数据源名称映射，不显示内部标识原文。
 */
export function searchCardContext(
  runId: string,
  items: readonly StreamItem[],
  sourceNames: Readonly<Record<string, string>> | null | undefined,
): SearchCardContext {
  for (let i = items.length - 1; i >= 0; i--) {
    const it = items[i];
    if (it.kind !== "plan_card" || it.run_id !== runId) continue;
    const step = (it.steps || []).find(
      (s) => s.tool_id === "search_start" && s.disclosure !== null && typeof s.disclosure === "object",
    );
    if (!step || !step.disclosure) continue;
    const disclosure = step.disclosure as unknown as Record<string, unknown>;
    return {
      source:
        nonEmpty(disclosure.display_name_zh) ?? mappedSourceName(disclosure.resource_id, sourceNames),
      conditionBadges: badgesOf(disclosure.aoi_name, disclosure.time_range),
    };
  }

  for (let i = items.length - 1; i >= 0; i--) {
    const it = items[i];
    if (it.kind !== "confirm_card" || it.run_id !== runId) continue;
    const snapshot = (it.snapshot ?? {}) as unknown as Record<string, unknown>;
    const range = (snapshot.range ?? {}) as Record<string, unknown>;
    return {
      source:
        nonEmpty(snapshot.source_display_name_zh) ?? mappedSourceName(snapshot.source, sourceNames),
      conditionBadges: badgesOf(range.aoi_name, range.time_range),
    };
  }

  return { source: null, conditionBadges: [] };
}

/**
 * 确认卡「来源」：只取同一 run 计划卡 search_start 披露的显示名（或经 sourceNames 映射 resource_id）。
 * 快照 source 是后端 provider id，不显示；没有披露、或同一 run 披露了不止一个 resource_id（无法确定是哪一个）时为「未知」。
 */
export function confirmCardSourceLabel(
  runId: string,
  items: readonly StreamItem[],
  sourceNames: Readonly<Record<string, string>> | null | undefined,
): string {
  const resourceIds = new Set<string>();
  let label: string | null = null;
  for (let i = items.length - 1; i >= 0; i--) {
    const it = items[i];
    if (it.kind !== "plan_card" || it.run_id !== runId) continue;
    for (const step of it.steps || []) {
      if (step.tool_id !== "search_start" || step.disclosure === null || typeof step.disclosure !== "object") continue;
      const disclosure = step.disclosure as unknown as Record<string, unknown>;
      const resourceId = nonEmpty(disclosure.resource_id);
      if (resourceId !== null) resourceIds.add(resourceId);
      if (label === null) {
        label = nonEmpty(disclosure.display_name_zh) ?? mappedSourceName(disclosure.resource_id, sourceNames);
      }
    }
  }
  if (resourceIds.size > 1) return t("shared.searchCardSource.unknownSource");
  return label ?? t("shared.searchCardSource.unknownSource");
}

/**
 * 确认卡「来源」：契约 3.34.0 起快照自带 source_display_name_zh（null 即后端也不知道 →「未知」）。
 * 只有更早的卡没有这个键，才回退到同 run 计划卡的披露。快照只读，不补写任何键（计划哈希）。
 */
export function confirmCardSource(
  item: StreamConfirmCardItem,
  items: readonly StreamItem[],
  sourceNames: Readonly<Record<string, string>> | null | undefined,
): string {
  const snapshot = item.snapshot as unknown;
  if (
    snapshot !== null &&
    typeof snapshot === "object" &&
    Object.prototype.hasOwnProperty.call(snapshot, "source_display_name_zh")
  ) {
    const rec = snapshot as Record<string, unknown>;
    return (
      nonEmpty(rec.source_display_name_zh) ??
      (nonEmpty(rec.source_resource_id) ? mappedSourceName(rec.source_resource_id, sourceNames) : null) ??
      t("shared.searchCardSource.unknownSource")
    );
  }
  if (snapshot !== null && typeof snapshot === "object") {
    const rec = snapshot as Record<string, unknown>;
    if (nonEmpty(rec.source_resource_id)) {
      const resolved = mappedSourceName(rec.source_resource_id, sourceNames);
      if (resolved) return resolved;
    }
  }
  return confirmCardSourceLabel(item.run_id, items, sourceNames);
}
