/**
 * agentRunBadges.ts
 * 纯逻辑模块：AgentRunSummary 徽章标记判定、徽章文案与开始时间格式化。
 */

import { t } from "../../i18n/index.ts";

function pad2(n: number): string {
  return n < 10 ? `0${n}` : String(n);
}

/**
 * 严格布尔判定：仅字段显式 === true 算真
 * 缺省 / false / null / "true" / 1 等皆算 false（不猜）
 */
export function runBadgeFlags(run: unknown): {
  reasoningOffRefused: boolean;
  truncated: boolean;
} {
  if (!run || typeof run !== "object") {
    return { reasoningOffRefused: false, truncated: false };
  }
  const r = run as Record<string, unknown>;
  return {
    reasoningOffRefused: r.reasoning_off_refused === true,
    truncated: r.truncated === true,
  };
}

/**
 * 返回徽章文本列表
 * 顺序固定为：「关闭思考被拒」在先，「回复被截断」在后，无重复
 */
export function runBadgeLabels(flags: {
  reasoningOffRefused: boolean;
  truncated: boolean;
}): string[] {
  const labels: string[] = [];
  if (flags?.reasoningOffRefused === true) {
    labels.push(t("shared.agentRunBadges.thinkingDisableRejected"));
  }
  if (flags?.truncated === true) {
    labels.push(t("shared.agentRunBadges.replyTruncated"));
  }
  return labels;
}

/**
 * 开始时间格式化
 * - 若解析为有效 Date，格式化为本地时间 `YYYY-MM-DD HH:mm`
 * - 若无法解析为有效 Date，但为非空字符串，返回其修整（trimmed）后的文本
 * - 其余情况（空串、纯空白、null、undefined 或非字符串）返回「时间未知」
 * - 绝不编造日期
 */
export function formatRunStarted(startedAt: unknown): string {
  if (typeof startedAt === "string") {
    const trimmed = startedAt.trim();
    if (!trimmed) {
      return t("shared.agentRunBadges.unknownTime");
    }
    const d = new Date(trimmed);
    if (!isNaN(d.getTime())) {
      const yyyy = d.getFullYear();
      const mm = pad2(d.getMonth() + 1);
      const dd = pad2(d.getDate());
      const hh = pad2(d.getHours());
      const min = pad2(d.getMinutes());
      return `${yyyy}-${mm}-${dd} ${hh}:${min}`;
    }
    return trimmed;
  }
  if (startedAt instanceof Date && !isNaN(startedAt.getTime())) {
    const yyyy = startedAt.getFullYear();
    const mm = pad2(startedAt.getMonth() + 1);
    const dd = pad2(startedAt.getDate());
    const hh = pad2(startedAt.getHours());
    const min = pad2(startedAt.getMinutes());
    return `${yyyy}-${mm}-${dd} ${hh}:${min}`;
  }
  return t("shared.agentRunBadges.unknownTime");
}
