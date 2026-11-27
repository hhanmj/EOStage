/**
 * agentHistoryLogic.ts
 * Agent 历史分组与会话标题纯函数逻辑模块（W5-2d / D1）
 *
 * 遵循无状态纯逻辑设计：无 React、无 DOM、仅导入契约类型。
 */

import type { AgentSessionSummary } from "../../lib/generated/v3-contracts.ts";
import { t } from "../../i18n/index.ts";

export type HistoryGroupKey = "today" | "yesterday" | "earlier";
export type HistoryGroupLabel = string;

export interface HistoryItem {
  sessionId: string;
  title: string;
  timeLabel: string;
  updatedAt: string;
}

export interface HistoryGroup {
  key: HistoryGroupKey;
  label: HistoryGroupLabel;
  items: HistoryItem[];
}

interface DateClassification {
  bucket: HistoryGroupKey;
  time: number | null;
}

function classifyDate(iso: string, now: Date): DateClassification {
  if (typeof iso !== "string" || !iso) {
    return { bucket: "earlier", time: null };
  }
  const timestamp = Date.parse(iso);
  if (Number.isNaN(timestamp)) {
    return { bucket: "earlier", time: null };
  }

  // 相对 now 处于未来的时间戳视为今天
  if (timestamp > now.getTime()) {
    return { bucket: "today", time: timestamp };
  }

  const itemDate = new Date(timestamp);
  const nowYear = now.getFullYear();
  const nowMonth = now.getMonth();
  const nowDate = now.getDate();

  // 本地当天
  if (
    itemDate.getFullYear() === nowYear &&
    itemDate.getMonth() === nowMonth &&
    itemDate.getDate() === nowDate
  ) {
    return { bucket: "today", time: timestamp };
  }

  // 本地昨天（自动处理跨月跨年边界）
  const yesterday = new Date(nowYear, nowMonth, nowDate - 1);
  if (
    itemDate.getFullYear() === yesterday.getFullYear() &&
    itemDate.getMonth() === yesterday.getMonth() &&
    itemDate.getDate() === yesterday.getDate()
  ) {
    return { bucket: "yesterday", time: timestamp };
  }

  return { bucket: "earlier", time: timestamp };
}

/**
 * 格式化历史条目展示时间：
 * - 今天或昨天：24 小时制补零 "HH:mm"（如 "08:05"）
 * - 更早且与 now 同年："M月D日 HH:mm"（月、日不补零，时间补零，如 "9月3日 16:05"）
 * - 更早且跨年："YYYY年M月D日"（如 "2025年12月30日"）
 * - 无法解析的时间戳："时间未知"
 */
export function formatHistoryTime(iso: string, now: Date): string {
  const { bucket, time } = classifyDate(iso, now);
  if (time === null) {
    return t("agent.history.unknownTime");
  }

  const itemDate = new Date(time);
  const hours = String(itemDate.getHours()).padStart(2, "0");
  const minutes = String(itemDate.getMinutes()).padStart(2, "0");

  if (bucket === "today" || bucket === "yesterday") {
    return `${hours}:${minutes}`;
  }

  const itemYear = itemDate.getFullYear();
  const nowYear = now.getFullYear();
  const month = itemDate.getMonth() + 1;
  const day = itemDate.getDate();

  if (itemYear === nowYear) {
    return t("agent.history.dateTime", { month, day, time: `${hours}:${minutes}` });
  }

  return t("agent.history.date", { year: itemYear, month, day });
}

/**
 * 规整会话标题：
 * - 去除首尾空白
 * - 若为空/全空白/null/undefined 则回退为 "未命名会话"
 */
export function historyTitle(title: string | null | undefined): string {
  if (title == null) {
    return t("agent.history.untitled");
  }
  const trimmed = title.trim();
  if (trimmed.length === 0) {
    return t("agent.history.untitled");
  }
  return trimmed;
}

/**
 * 从首条用户需求生成会话标题：
 * - 去除首尾空白，将所有连续空白（包括换行与制表符）折叠为一个空格
 * - 若结果为空返回 null
 * - 若字数大于 40（使用 Array.from 计数，使 CJK 和单个 emoji 均计为 1 字符），截取前 40 字符并追加 "…"
 * - 否则原样返回
 */
export function sessionTitleFromRequest(text: string): string | null {
  if (typeof text !== "string") {
    return null;
  }
  const collapsed = text.trim().replace(/\s+/g, " ");
  if (collapsed.length === 0) {
    return null;
  }
  const chars = Array.from(collapsed);
  if (chars.length > 40) {
    return chars.slice(0, 40).join("") + "…";
  }
  return collapsed;
}

/**
 * 按本地日历日将历史会话分组：
 * - 今天、昨天、更早；未来时间戳计入今天；不可解析时间戳计入更早
 * - 组内按 updated_at 降序排列（最新在前），不可解析项排在所有有效项之后，timeLabel 为 "时间未知"
 * - 组顺序固定为今天、昨天、更早；空组不保留；空输入返回 []
 */
export function groupSessionsByDay(
  sessions: AgentSessionSummary[],
  now: Date
): HistoryGroup[] {
  if (!Array.isArray(sessions) || sessions.length === 0) {
    return [];
  }

  const todayItems: HistoryItem[] = [];
  const yesterdayItems: HistoryItem[] = [];
  const earlierItems: HistoryItem[] = [];

  for (const session of sessions) {
    const { bucket } = classifyDate(session.updated_at, now);
    const item: HistoryItem = {
      sessionId: session.session_id,
      title: historyTitle(session.title),
      timeLabel: formatHistoryTime(session.updated_at, now),
      updatedAt: session.updated_at,
    };

    if (bucket === "today") {
      todayItems.push(item);
    } else if (bucket === "yesterday") {
      yesterdayItems.push(item);
    } else {
      earlierItems.push(item);
    }
  }

  const sortItems = (items: HistoryItem[]) => {
    items.sort((a, b) => {
      const tA = Date.parse(a.updatedAt);
      const tB = Date.parse(b.updatedAt);
      const isNanA = Number.isNaN(tA);
      const isNanB = Number.isNaN(tB);
      if (isNanA && isNanB) {
        return 0;
      }
      if (isNanA) {
        return 1;
      }
      if (isNanB) {
        return -1;
      }
      return tB - tA;
    });
  };

  sortItems(todayItems);
  sortItems(yesterdayItems);
  sortItems(earlierItems);

  const groups: HistoryGroup[] = [];
  if (todayItems.length > 0) {
    groups.push({ key: "today", label: t("agent.history.today"), items: todayItems });
  }
  if (yesterdayItems.length > 0) {
    groups.push({ key: "yesterday", label: t("agent.history.yesterday"), items: yesterdayItems });
  }
  if (earlierItems.length > 0) {
    groups.push({ key: "earlier", label: t("agent.history.earlier"), items: earlierItems });
  }

  return groups;
}

export type HistoryDeleteState =
  | { phase: "idle" }
  | { phase: "confirming"; sessionId: string }
  | { phase: "deleting"; sessionId: string }
  | { phase: "failed"; sessionId: string; message: string };

export type HistoryDeleteEvent =
  | { type: "request"; sessionId: string }
  | { type: "cancel" }
  | { type: "confirm" }
  | { type: "succeeded" }
  | { type: "failed"; message: string };

export const HISTORY_DELETE_CONFIRM_TEXT = {
  title: t("agent.history.deleteTitle"),
  body: t("agent.history.deleteBody"),
  confirm: t("agent.history.delete"),
  cancel: t("agent.cancel"),
} as const;

/**
 * 解析当前活动会话：
 * - 仅当 activeSessionId 匹配到对应会话时返回该会话
 * - 当 activeSessionId 为 null/undefined/空串或在列表中未找到时，恒返回 null（绝不回退至 sessions[0]）
 */
export function resolveActiveSession(
  sessions: AgentSessionSummary[],
  activeSessionId: string | null | undefined
): AgentSessionSummary | null {
  if (!activeSessionId || !Array.isArray(sessions) || sessions.length === 0) {
    return null;
  }
  return sessions.find((s) => s.session_id === activeSessionId) ?? null;
}

/**
 * 历史删除状态机：确保删除前必须经过二次确认
 */
export function nextHistoryDeleteState(
  state: HistoryDeleteState,
  event: HistoryDeleteEvent
): HistoryDeleteState {
  switch (event.type) {
    case "request":
      if (state.phase === "idle" || state.phase === "failed") {
        return { phase: "confirming", sessionId: event.sessionId };
      }
      return state;

    case "cancel":
      if (state.phase === "confirming" || state.phase === "failed") {
        return { phase: "idle" };
      }
      return state;

    case "confirm":
      if (state.phase === "confirming") {
        return { phase: "deleting", sessionId: state.sessionId };
      }
      return state;

    case "succeeded":
      if (state.phase === "deleting") {
        return { phase: "idle" };
      }
      return state;

    case "failed":
      if (state.phase === "deleting") {
        return { phase: "failed", sessionId: state.sessionId, message: event.message };
      }
      return state;

    default:
      return state;
  }
}
