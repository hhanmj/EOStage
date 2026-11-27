import { t } from "../../i18n/index.ts";

function formatEndpoint(val: unknown): string | null {
  if (typeof val !== "string") {
    return null;
  }
  const trimmed = val.trim();
  if (trimmed.length === 0) {
    return null;
  }
  // 形如 YYYY-MM-DD... 时只取前 10 个字符，其余原样
  if (/^\d{4}-\d{2}-\d{2}/.test(trimmed)) {
    return trimmed.slice(0, 10);
  }
  return trimmed;
}

/**
 * 计划卡 search_start 步骤的检索披露文本格式化。
 * 格式：「将向 ${来源} 检索 ${AOI} ${时间}」
 * 若 disclosure 缺失或无 resource_id 返回 null（不显示披露行）。
 */
export function disclosureLine(
  disclosure: unknown,
  sourceNames: Readonly<Record<string, string>> | null | undefined,
): string | null {
  if (disclosure === null || typeof disclosure !== "object") {
    return null;
  }
  const obj = disclosure as Record<string, unknown>;
  const resourceId = obj.resource_id;
  if (typeof resourceId !== "string" || resourceId.trim().length === 0) {
    return null;
  }

  // 来源：优先 disclosure.display_name_zh 去空白非空原样使用；次选 sourceNames[resource_id] 去空白非空；否则「未知数据源」（不显示 resource_id 原文）
  let source = t("shared.planDisclosure.unknownSource");
  const rawDisplayNameZh = obj.display_name_zh;
  if (typeof rawDisplayNameZh === "string" && rawDisplayNameZh.trim().length > 0) {
    source = rawDisplayNameZh;
  } else {
    const mappedSource = sourceNames?.[resourceId];
    if (typeof mappedSource === "string" && mappedSource.trim().length > 0) {
      source = mappedSource.trim();
    }
  }

  // AOI：aoi_name 去空白后非空用它；否则「未命名范围」
  const rawAoi = obj.aoi_name;
  const aoi =
    typeof rawAoi === "string" && rawAoi.trim().length > 0
      ? rawAoi.trim()
      : t("shared.planDisclosure.unnamedAoi");

  // 时间：time_range 为对象时取 start/end
  let timeText = t("shared.planDisclosure.anyTime");
  const rawTimeRange = obj.time_range;
  if (rawTimeRange !== null && typeof rawTimeRange === "object") {
    const timeObj = rawTimeRange as Record<string, unknown>;
    const start = formatEndpoint(timeObj.start);
    const end = formatEndpoint(timeObj.end);
    if (start && end) {
      timeText = t("shared.planDisclosure.timeRange", { start, end });
    } else if (start) {
      timeText = t("shared.planDisclosure.timeFrom", { start });
    } else if (end) {
      timeText = t("shared.planDisclosure.timeUntil", { end });
    }
  }

  return t("shared.planDisclosure.searchSummary", { source, aoi, timeText });
}
