/**
 * agentActivityHooks.ts
 * 通知上层重读 agent_activity_status（全部停止以后端活动状态为准，C1 规则 12）。
 */

import { useEffect, useRef } from "react";
import type { AgentStreamEvent } from "@/lib/generated/v3-contracts";

export interface UseAgentActivityNotifierOptions {
  sessionId: string;
  activeRunId: string | null;
  events: ReadonlyArray<AgentStreamEvent>;
  onActivityChanged?: () => void;
}

/**
 * 三种时机各通知一次：会话切换、本地活动运行开始/结束（activeRunId 变化）、事件流里出现新的 activity_changed。
 * 事件来自哪条拉取路径（发送后立即拉取、批准后拉取、定时轮询、打开历史）都一样处理：按 seq 去重，同一批多条只通知一次，换会话重新计序号。
 */
export function useAgentActivityNotifier({
  sessionId,
  activeRunId,
  events,
  onActivityChanged,
}: UseAgentActivityNotifierOptions): void {
  const onActivityChangedRef = useRef(onActivityChanged);
  onActivityChangedRef.current = onActivityChanged;

  useEffect(() => {
    onActivityChangedRef.current?.();
  }, [sessionId, activeRunId]);

  const lastSeenRef = useRef<{ sessionId: string; seq: number }>({ sessionId, seq: -1 });
  useEffect(() => {
    if (lastSeenRef.current.sessionId !== sessionId) {
      lastSeenRef.current = { sessionId, seq: -1 };
    }
    let latest = -1;
    for (const event of events) {
      if (event && event.kind === "activity_changed" && typeof event.seq === "number" && event.seq > latest) {
        latest = event.seq;
      }
    }
    if (latest > lastSeenRef.current.seq) {
      lastSeenRef.current.seq = latest;
      onActivityChangedRef.current?.();
    }
  }, [sessionId, events]);
}
