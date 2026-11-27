/**
 * 模型用量的界面文案（契约 3.34.0：AgentRunSummary.usage_total、model_reply.payload.usage）。
 * 渠道没报的部分写「不完整」，整条没报就不显示——不当作 0。
 */

import { t } from "../../i18n/index.ts";

interface UsageParts {
  prompt: number | null;
  completion: number | null;
}

function count(value: unknown): number | null {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0 ? value : null;
}

function formatCount(value: number | null): string {
  return value === null ? t("shared.agentUsage.incomplete") : String(value).replace(/\B(?=(\d{3})+(?!\d))/g, ",");
}

function usageParts(usage: unknown): UsageParts | null {
  if (usage === null || typeof usage !== "object" || Array.isArray(usage)) return null;
  const obj = usage as Record<string, unknown>;
  return { prompt: count(obj.prompt), completion: count(obj.completion) };
}

function partsText(parts: UsageParts): string {
  return t("shared.agentUsage.inputAndOutput", {
    prompt: formatCount(parts.prompt),
    completion: formatCount(parts.completion),
  });
}

/** 运行记录条：「用量 输入 X · 输出 Y」；usage_total 为 null 或缺省时不显示。 */
export function runUsageText(usageTotal: unknown): string | null {
  const parts = usageParts(usageTotal);
  return parts ? t("shared.agentUsage.usagePrefix", { parts: partsText(parts) }) : null;
}

/** 对话里一轮运行的用量行：与运行记录条同源（usage_total，即各 model_reply.usage 之和），不另读运行记录。 */
export function turnUsageText(usageTotal: unknown): string | null {
  const parts = usageParts(usageTotal);
  return parts ? t("shared.agentUsage.turnUsagePrefix", { parts: partsText(parts) }) : null;
}

/** 逐次明细：运行记录里每个带 usage 的 model_reply 一行（只在用户悬停/展开用量行时读取）。 */
export function turnUsageDetailLines(events: unknown): string[] {
  if (!Array.isArray(events)) return [];
  const lines: string[] = [];
  for (const event of events) {
    if (event === null || typeof event !== "object") continue;
    const { kind, payload } = event as { kind?: unknown; payload?: unknown };
    if (kind !== "model_reply" || payload === null || typeof payload !== "object") continue;
    const parts = usageParts((payload as Record<string, unknown>).usage);
    if (parts) lines.push(t("shared.agentUsage.replyLine", { index: lines.length + 1, parts: partsText(parts) }));
  }
  return lines;
}
