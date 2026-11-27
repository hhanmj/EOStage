/**
 * 每轮用量的逐次明细：只在用户悬停或展开某一运行的用量行时，才读那一个运行的记录（agent_runs_open），
 * 结果按 run_id 缓存；请求串行，一次只发一个。读取失败记为不可用，不重试。
 */

import { useCallback, useEffect, useRef, useState } from "react";
import type { AgentRunDetail } from "@/lib/generated/v3-contracts";
import { turnUsageDetailLines } from "@/shell/api/agentUsage";

export type RunUsageDetailState =
  | { status: "loading" }
  | { status: "loaded"; lines: string[] }
  | { status: "failed" };

export function useRunUsageDetail(
  openRun: ((runId: string) => Promise<AgentRunDetail>) | null,
): [Readonly<Record<string, RunUsageDetailState>>, (runId: string) => void] {
  const [details, setDetails] = useState<Record<string, RunUsageDetailState>>({});
  const requestedRef = useRef<Set<string>>(new Set());
  const queueRef = useRef<Promise<void>>(Promise.resolve());
  const mountedRef = useRef(true);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const request = useCallback(
    (runId: string) => {
      if (!openRun || typeof runId !== "string" || !runId || requestedRef.current.has(runId)) return;
      requestedRef.current.add(runId);
      setDetails((prev) => ({ ...prev, [runId]: { status: "loading" } }));
      queueRef.current = queueRef.current.then(async () => {
        let next: RunUsageDetailState;
        try {
          const detail = await openRun(runId);
          next = { status: "loaded", lines: turnUsageDetailLines(detail?.events) };
        } catch {
          next = { status: "failed" };
        }
        if (mountedRef.current) setDetails((prev) => ({ ...prev, [runId]: next }));
      });
    },
    [openRun]
  );

  return [details, request];
}
