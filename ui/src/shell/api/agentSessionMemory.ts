/**
 * agentSessionMemory.ts
 * 桌面外壳级活动会话内存记忆与活动栏导航决策模块（W5-8 P2-6 / R4b）
 * 仅在当前进程内存中记住最后活动的 Agent 会话 ID，从下载中心或其他路由返回对话页时恢复原会话；
 * 不跨应用重启持久化。
 */

import type { ShellRoute } from "./shellRoutes.ts";

export interface AgentSessionMemory {
  remember: (sessionId: string | null | undefined) => void;
  recall: () => string | null;
  clear: () => void;
}

export type AgentRailNavigationAction =
  | { kind: "stay" }
  | { kind: "restore"; sessionId: string }
  | { kind: "open_empty" };

export interface ResolveAgentRailNavigationInput {
  currentPageRoute: ShellRoute;
  rememberedSessionId: string | null | undefined;
}

function normalizeSessionId(sessionId: string | null | undefined): string | null {
  if (typeof sessionId !== "string") {
    return null;
  }
  const trimmed = sessionId.trim();
  return trimmed.length > 0 ? trimmed : null;
}

export function createAgentSessionMemory(
  initialSessionId: string | null = null
): AgentSessionMemory {
  let currentSessionId: string | null = normalizeSessionId(initialSessionId);

  return {
    remember: (sessionId: string | null | undefined) => {
      currentSessionId = normalizeSessionId(sessionId);
    },
    recall: () => currentSessionId,
    clear: () => {
      currentSessionId = null;
    },
  };
}

/**
 * 决策点击左侧活动栏「对话」项时的导航动作（W5-8 R4b）：
 * - 当前已在 agent 页面 → stay（不做新建或清空）
 * - 当前在非 agent 页面且有记住的活动会话 → restore
 * - 当前在非 agent 页面且无记住的会话 → open_empty
 */
export function resolveAgentRailNavigation(
  input: ResolveAgentRailNavigationInput
): AgentRailNavigationAction {
  const { currentPageRoute, rememberedSessionId } = input;
  if (currentPageRoute === "agent") {
    return { kind: "stay" };
  }
  const normalized = normalizeSessionId(rememberedSessionId);
  if (normalized) {
    return { kind: "restore", sessionId: normalized };
  }
  return { kind: "open_empty" };
}
