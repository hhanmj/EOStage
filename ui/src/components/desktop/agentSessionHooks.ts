/**
 * agentSessionHooks.ts
 * 外壳与对话工作台之间的活动会话协作（W5-8 R4b 会话记忆 / R9 新对话回弹修复）。
 * 父：useAgentSessionShell 持有「打开的会话」「会话记忆」「新对话 nonce」；
 * 子：useAgentActiveSession 持有活动会话，响应 openSessionId 与新对话 nonce，并向上报告变化。
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { Dispatch, SetStateAction } from "react";
import {
  createAgentSessionMemory,
  type AgentSessionMemory,
} from "@/shell/api/agentSessionMemory";

export interface AgentSessionShell {
  agentOpenSessionId: string | null;
  setAgentOpenSessionId: Dispatch<SetStateAction<string | null>>;
  agentSessionMemory: AgentSessionMemory;
  hasActiveAgentSession: boolean;
  setHasActiveAgentSession: Dispatch<SetStateAction<boolean>>;
  newChatNonce: number;
  startNewChat: () => void;
  handleActiveSessionChange: (sessionId: string | null) => void;
}

export function useAgentSessionShell(onStartNewChat?: () => void): AgentSessionShell {
  const [agentOpenSessionId, setAgentOpenSessionId] = useState<string | null>(null);
  const agentSessionMemory = useMemo(() => createAgentSessionMemory(), []);
  const [hasActiveAgentSession, setHasActiveAgentSession] = useState<boolean>(false);
  const [newChatNonce, setNewChatNonce] = useState(0);
  const onStartNewChatRef = useRef(onStartNewChat);
  onStartNewChatRef.current = onStartNewChat;

  const startNewChat = useCallback(() => {
    setAgentOpenSessionId(null);
    agentSessionMemory.clear();
    setHasActiveAgentSession(false);
    setNewChatNonce((prev) => prev + 1);
    onStartNewChatRef.current?.();
  }, [agentSessionMemory]);

  const handleActiveSessionChange = useCallback(
    (sessionId: string | null) => {
      agentSessionMemory.remember(sessionId);
      setHasActiveAgentSession(Boolean(sessionId && sessionId.trim().length > 0));
    },
    [agentSessionMemory]
  );

  return {
    agentOpenSessionId,
    setAgentOpenSessionId,
    agentSessionMemory,
    hasActiveAgentSession,
    setHasActiveAgentSession,
    newChatNonce,
    startNewChat,
    handleActiveSessionChange,
  };
}

export interface UseAgentActiveSessionOptions {
  openSessionId: string | null;
  newChatNonce?: number;
  onActiveSessionChange?: (sessionId: string | null) => void;
  /** openSessionId 打开某会话时 */
  onOpenSession?: () => void;
  /** 新对话 nonce 变化、活动会话已清空时（重置事件流等） */
  onNewChat?: () => void;
}

/** 异步结果写入前的会话核对凭据 */
export interface SessionToken {
  sessionId: string;
  epoch: number;
}

export interface SessionGuard {
  /** 发起异步请求前记下当前会话与「新对话」代数 */
  capture: () => SessionToken;
  /** 会话与代数都没变：结果仍属于当前对话 */
  stillCurrent: (token: SessionToken) => boolean;
  /** 期间没有点过「新对话」 */
  noNewChatSince: (token: SessionToken) => boolean;
}

export function useAgentActiveSession({
  openSessionId,
  newChatNonce,
  onActiveSessionChange,
  onOpenSession,
  onNewChat,
}: UseAgentActiveSessionOptions): [string, Dispatch<SetStateAction<string>>, SessionGuard] {
  const [activeSessionId, setActiveSessionId] = useState<string>(() => openSessionId || "");
  // 异步结果（会话列表、历史、轮询）返回时据此判断是否仍属于当前对话
  const currentSessionRef = useRef(activeSessionId);
  currentSessionRef.current = activeSessionId;
  const newChatEpochRef = useRef(0);
  const onOpenSessionRef = useRef(onOpenSession);
  onOpenSessionRef.current = onOpenSession;
  const onNewChatRef = useRef(onNewChat);
  onNewChatRef.current = onNewChat;

  const onActiveSessionChangeRef = useRef(onActiveSessionChange);
  onActiveSessionChangeRef.current = onActiveSessionChange;

  // 只在活动会话真正变化时上报。回调不进依赖：父组件每次渲染若给新回调，旧写法会在
  // 「新对话」清空前用旧会话再上报一次，父组件据此重新记住并把旧会话作为 openSessionId 传回（R9 回弹）。
  const lastReportedRef = useRef<string | null | undefined>(undefined);
  useEffect(() => {
    const sessionId = activeSessionId || null;
    if (lastReportedRef.current === sessionId) return;
    lastReportedRef.current = sessionId;
    onActiveSessionChangeRef.current?.(sessionId);
  }, [activeSessionId]);

  useEffect(() => {
    if (openSessionId) {
      setActiveSessionId(openSessionId);
      onOpenSessionRef.current?.();
    }
  }, [openSessionId]);

  const initialNonceRef = useRef(newChatNonce);
  useEffect(() => {
    if (newChatNonce !== undefined && newChatNonce !== initialNonceRef.current) {
      initialNonceRef.current = newChatNonce;
      newChatEpochRef.current += 1;
      currentSessionRef.current = "";
      setActiveSessionId("");
      onNewChatRef.current?.();
    }
  }, [newChatNonce]);

  const guard = useMemo<SessionGuard>(
    () => ({
      capture: () => ({ sessionId: currentSessionRef.current, epoch: newChatEpochRef.current }),
      stillCurrent: (token) =>
        token.epoch === newChatEpochRef.current && token.sessionId === currentSessionRef.current,
      noNewChatSince: (token) => token.epoch === newChatEpochRef.current,
    }),
    []
  );

  return [activeSessionId, setActiveSessionId, guard];
}
