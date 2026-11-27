import { useCallback, useEffect, useRef, useState } from "react";
import type {
  AgentSessionSummary,
  AgentWorkspace,
  AgentSessionExportFormat,
} from "../../lib/generated/v3-contracts.ts";
import type { DesktopAgentBridge } from "../../shell/api/agentBridgeTypes.ts";
import { getDesktopShellBridge, isTauriDesktopHost } from "../../lib/desktopShellBridge";
import {
  SESSION_ORGANIZE_ERRORS,
  detachWorkspaceSessions,
  exportNoteText,
  removeSession,
  replaceSession,
} from "../../shell/api/sessionOrganize.ts";
import { completeLongWork, failLongWork, startLongWork } from "../../lib/longWorkRegistry.ts";

export interface InlineMutationStatus {
  state: "idle" | "saving" | "saved" | "error";
  errorMessage?: string | null;
  retry?: () => void;
}

export interface FailedRenameDraft {
  kind: "session" | "workspace";
  targetId: string;
  draft: string;
}

type Listener = (sourceId: number) => void;
const listeners = new Set<Listener>();
let nextInstanceId = 1;

/** 侧栏、历史页与对话区各自持有会话列表；任一处改动后通知其余视图重新读取。 */
export function subscribeSessionsChanged(listener: () => void): () => void {
  const wrapped: Listener = () => listener();
  listeners.add(wrapped);
  return () => {
    listeners.delete(wrapped);
  };
}

function emitSessionsChanged(sourceId: number): void {
  for (const listener of [...listeners]) listener(sourceId);
}

function subscribeInternal(listener: Listener): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function shellBridge() {
  if (!isTauriDesktopHost()) return null;
  try {
    return getDesktopShellBridge();
  } catch {
    return null;
  }
}

export interface OrganizeNote {
  kind: "ok" | "error";
  text: string;
  /** 递增序号，相同文本重复出现时仍重新计时 */
  seq: number;
}

export interface UseSessionOrganizerOptions {
  enabled?: boolean;
  /** 历史页读取含已归档的完整列表 */
  includeArchived?: boolean;
  refreshKey?: string | null;
  /** 无桌面宿主时的演示数据（开发模式）；为空则列表为空 */
  fixtureSessions?: (() => AgentSessionSummary[] | Promise<AgentSessionSummary[]>) | null;
}

export interface SessionOrganizer {
  sessions: AgentSessionSummary[];
  workspaces: AgentWorkspace[];
  loading: boolean;
  loadError: boolean;
  /** 桌面宿主可用；演示模式下整理操作禁用 */
  canOrganize: boolean;
  note: OrganizeNote | null;
  clearNote: () => void;
  inlineMutation: InlineMutationStatus;
  failedRename: FailedRenameDraft | null;
  clearFailedRename: () => void;
  reload: () => Promise<void>;
  removeLocal: (sessionId: string) => void;
  renameSession: (sessionId: string, title: string) => Promise<boolean>;
  setPinned: (sessionId: string, pinned: boolean) => Promise<boolean>;
  setArchived: (sessionId: string, archived: boolean) => Promise<boolean>;
  moveSession: (sessionId: string, workspaceId: string | null) => Promise<boolean>;
  exportSession: (sessionId: string, format: AgentSessionExportFormat) => Promise<boolean>;
  createSession: (workspaceId: string | null) => Promise<AgentSessionSummary | null>;
  createWorkspace: (name: string) => Promise<AgentWorkspace | null>;
  renameWorkspace: (workspaceId: string, name: string) => Promise<boolean>;
  pickWorkspaceFolder: (workspaceId: string) => Promise<boolean>;
  unbindWorkspaceFolder: (workspaceId: string) => Promise<boolean>;
  deleteWorkspace: (workspaceId: string) => Promise<boolean>;
}

export function useSessionOrganizer({
  enabled = true,
  includeArchived = false,
  refreshKey,
  fixtureSessions,
}: UseSessionOrganizerOptions): SessionOrganizer {
  const [sessions, setSessions] = useState<AgentSessionSummary[]>([]);
  const [workspaces, setWorkspaces] = useState<AgentWorkspace[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState(false);
  const [note, setNote] = useState<OrganizeNote | null>(null);
  const [inlineMutation, setInlineMutation] = useState<InlineMutationStatus>({ state: "idle" });
  const [failedRename, setFailedRename] = useState<FailedRenameDraft | null>(null);
  const instanceId = useRef(0);
  if (instanceId.current === 0) instanceId.current = nextInstanceId++;
  const noteSeq = useRef(0);
  const savedTimerRef = useRef<number | null>(null);
  const sessionsRef = useRef(sessions);
  sessionsRef.current = sessions;
  const workspacesRef = useRef(workspaces);
  workspacesRef.current = workspaces;
  const fixtureRef = useRef(fixtureSessions);
  fixtureRef.current = fixtureSessions;
  const mounted = useRef(true);

  const canOrganize = Boolean(shellBridge()?.agent);

  const showNote = useCallback((kind: OrganizeNote["kind"], text: string) => {
    noteSeq.current += 1;
    setNote({ kind, text, seq: noteSeq.current });
  }, []);

  const load = useCallback(
    async (quiet: boolean) => {
      const agent = shellBridge()?.agent;
      if (!agent) {
        const factory = fixtureRef.current;
        let demo: AgentSessionSummary[] = [];
        try {
          demo = factory ? await factory() : [];
        } catch {
          demo = [];
        }
        if (!mounted.current) return;
        setSessions(demo);
        setWorkspaces([]);
        setLoadError(false);
        setLoading(false);
        return;
      }
      if (!quiet) {
        setLoading(true);
        setLoadError(false);
      }
      try {
        const [list, spaces] = await Promise.all([
          agent.sessionsList(includeArchived ? { includeArchived: true } : undefined),
          agent.workspacesList().catch(() => null),
        ]);
        if (!mounted.current) return;
        setSessions(list);
        if (spaces) setWorkspaces(spaces);
        setLoadError(false);
      } catch {
        if (mounted.current && !quiet) setLoadError(true);
      } finally {
        if (mounted.current) setLoading(false);
      }
    },
    [includeArchived]
  );

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      if (savedTimerRef.current !== null && typeof window !== "undefined") {
        window.clearTimeout(savedTimerRef.current);
        savedTimerRef.current = null;
      }
    };
  }, []);

  useEffect(() => {
    if (!enabled) return;
    void load(false);
  }, [enabled, load, refreshKey]);

  useEffect(() => {
    if (!enabled) return;
    return subscribeInternal((sourceId) => {
      if (sourceId !== instanceId.current) void load(true);
    });
  }, [enabled, load]);

  const changed = useCallback(() => emitSessionsChanged(instanceId.current), []);

  const run = useCallback(
    async <T,>(
      errorText: string,
      op: (agent: DesktopAgentBridge) => Promise<T>
    ): Promise<{ ok: true; value: T } | { ok: false }> => {
      const agent = shellBridge()?.agent;
      if (!agent) return { ok: false };
      try {
        const value = await op(agent);
        return { ok: true, value };
      } catch {
        if (mounted.current) showNote("error", errorText);
        return { ok: false };
      }
    },
    [showNote]
  );

  const clearSavedTimer = useCallback(() => {
    if (savedTimerRef.current !== null && typeof window !== "undefined") {
      window.clearTimeout(savedTimerRef.current);
      savedTimerRef.current = null;
    }
  }, []);

  const markMutationSaved = useCallback(() => {
    clearSavedTimer();
    setInlineMutation({ state: "saved" });
    if (typeof window !== "undefined") {
      savedTimerRef.current = window.setTimeout(() => {
        savedTimerRef.current = null;
        if (mounted.current) {
          setInlineMutation({ state: "idle" });
        }
      }, 1500);
    }
  }, [clearSavedTimer]);

  const runOptimisticSession = useCallback(
    async (
      sessionId: string,
      errorText: string,
      optimisticPatch: (cur: AgentSessionSummary) => AgentSessionSummary,
      op: (agent: DesktopAgentBridge) => Promise<AgentSessionSummary>,
      options?: { failedRenameDraft?: string; retryFn?: () => Promise<boolean> }
    ): Promise<boolean> => {
      clearSavedTimer();
      const prevSnapshot = sessionsRef.current.find((s) => s.session_id === sessionId);
      setFailedRename(null);
      setInlineMutation({ state: "saving" });
      if (prevSnapshot) {
        setSessions((prev) => replaceSession(prev, optimisticPatch(prevSnapshot)));
      }
      const res = await run<AgentSessionSummary>(errorText, op);
      if (!res.ok) {
        if (mounted.current && prevSnapshot) {
          setSessions((prev) => replaceSession(prev, prevSnapshot));
        }
        if (mounted.current) {
          if (options?.failedRenameDraft !== undefined) {
            setFailedRename({
              kind: "session",
              targetId: sessionId,
              draft: options.failedRenameDraft,
            });
          }
          setInlineMutation({
            state: "error",
            errorMessage: errorText,
            retry: options?.retryFn
              ? () => {
                  void options.retryFn!();
                }
              : undefined,
          });
        }
        return false;
      }
      if (!mounted.current) return true;
      setSessions((prev) => replaceSession(prev, res.value));
      changed();
      markMutationSaved();
      return true;
    },
    [clearSavedTimer, run, changed, markMutationSaved]
  );

  const renameSession = useCallback(
    (sessionId: string, title: string): Promise<boolean> =>
      runOptimisticSession(
        sessionId,
        SESSION_ORGANIZE_ERRORS.rename,
        (cur) => ({ ...cur, title, updated_at: new Date().toISOString() }),
        (a) => a.sessionsRename(sessionId, title),
        { failedRenameDraft: title, retryFn: () => renameSession(sessionId, title) }
      ),
    [runOptimisticSession]
  );
  const setPinned = useCallback(
    (sessionId: string, pinned: boolean): Promise<boolean> =>
      runOptimisticSession(
        sessionId,
        SESSION_ORGANIZE_ERRORS.pin,
        (cur) => ({ ...cur, pinned_at: pinned ? new Date().toISOString() : null }),
        (a) => a.sessionsSetPinned(sessionId, pinned),
        { retryFn: () => setPinned(sessionId, pinned) }
      ),
    [runOptimisticSession]
  );
  const setArchived = useCallback(
    (sessionId: string, archived: boolean): Promise<boolean> =>
      runOptimisticSession(
        sessionId,
        SESSION_ORGANIZE_ERRORS.archive,
        (cur) => ({ ...cur, archived_at: archived ? new Date().toISOString() : null }),
        (a) => a.sessionsSetArchived(sessionId, archived),
        { retryFn: () => setArchived(sessionId, archived) }
      ),
    [runOptimisticSession]
  );
  const moveSession = useCallback(
    (sessionId: string, workspaceId: string | null): Promise<boolean> =>
      runOptimisticSession(
        sessionId,
        SESSION_ORGANIZE_ERRORS.move,
        (cur) => ({ ...cur, workspace_id: workspaceId }),
        (a) => a.sessionsMove(sessionId, workspaceId),
        { retryFn: () => moveSession(sessionId, workspaceId) }
      ),
    [runOptimisticSession]
  );

  const exportSession = useCallback(
    async (sessionId: string, format: AgentSessionExportFormat) => {
      const bridge = shellBridge();
      if (!bridge?.agent) return false;
      let directory: string | null = null;
      try {
        const picked = await bridge.dialog.pickDirectory("选择导出位置");
        directory = picked && picked.ok && picked.path && picked.path.trim().length > 0 ? picked.path : null;
      } catch {
        showNote("error", SESSION_ORGANIZE_ERRORS.export);
        return false;
      }
      if (!directory) return false;
      const target = directory;
      const workId = startLongWork({
        title: `导出会话 (${format.toUpperCase()})`,
        category: "export",
        stageLabel: "正在写入导出文件…",
      });
      const res = await run<string>(SESSION_ORGANIZE_ERRORS.export, (a) =>
        a.sessionsExportTo(sessionId, format, target)
      );
      if (!res.ok) {
        failLongWork(workId, SESSION_ORGANIZE_ERRORS.export);
        return false;
      }
      const summary = exportNoteText(res.value);
      completeLongWork(workId, { doneSubtitle: summary });
      showNote("ok", summary);
      return true;
    },
    [run, showNote]
  );

  const createSession = useCallback(
    async (workspaceId: string | null) => {
      const res = await run<AgentSessionSummary>("新建对话失败", (a) =>
        a.sessionsCreate(null, workspaceId)
      );
      if (!res.ok) return null;
      setSessions((prev) => replaceSession(prev, res.value));
      changed();
      return res.value;
    },
    [run, changed]
  );

  const replaceWorkspace = useCallback((updated: AgentWorkspace) => {
    setWorkspaces((prev) => {
      const exists = prev.some((w) => w.workspace_id === updated.workspace_id);
      return exists
        ? prev.map((w) => (w.workspace_id === updated.workspace_id ? updated : w))
        : [...prev, updated];
    });
  }, []);

  const createWorkspace = useCallback(
    async (name: string) => {
      const res = await run<AgentWorkspace>(SESSION_ORGANIZE_ERRORS.workspaceCreate, (a) =>
        a.workspacesCreate(name)
      );
      if (!res.ok) return null;
      replaceWorkspace(res.value);
      changed();
      return res.value;
    },
    [run, replaceWorkspace, changed]
  );

  const updateWorkspace = useCallback(
    async (workspaceId: string, change: { name?: string; folder?: string | null }): Promise<boolean> => {
      clearSavedTimer();
      const prevWorkspace = workspacesRef.current.find((w) => w.workspace_id === workspaceId);
      setFailedRename(null);
      setInlineMutation({ state: "saving" });
      if (prevWorkspace) {
        replaceWorkspace({
          ...prevWorkspace,
          ...(typeof change.name === "string" ? { name: change.name } : {}),
          ...(change.folder !== undefined ? { folder: change.folder } : {}),
        });
      }
      const res = await run<AgentWorkspace>(SESSION_ORGANIZE_ERRORS.workspaceUpdate, (a) =>
        a.workspacesUpdate(workspaceId, change)
      );
      if (!res.ok) {
        if (mounted.current && prevWorkspace) {
          replaceWorkspace(prevWorkspace);
        }
        if (mounted.current) {
          if (typeof change.name === "string") {
            setFailedRename({
              kind: "workspace",
              targetId: workspaceId,
              draft: change.name,
            });
          }
          setInlineMutation({
            state: "error",
            errorMessage: SESSION_ORGANIZE_ERRORS.workspaceUpdate,
            retry: () => {
              void updateWorkspace(workspaceId, change);
            },
          });
        }
        return false;
      }
      if (!mounted.current) return true;
      replaceWorkspace(res.value);
      changed();
      markMutationSaved();
      return true;
    },
    [clearSavedTimer, run, replaceWorkspace, changed, markMutationSaved]
  );

  const renameWorkspace = useCallback(
    (workspaceId: string, name: string) => updateWorkspace(workspaceId, { name }),
    [updateWorkspace]
  );
  const unbindWorkspaceFolder = useCallback(
    (workspaceId: string) => updateWorkspace(workspaceId, { folder: null }),
    [updateWorkspace]
  );
  const pickWorkspaceFolder = useCallback(
    async (workspaceId: string) => {
      const bridge = shellBridge();
      if (!bridge) return false;
      try {
        const picked = await bridge.dialog.pickDirectory("选择工作区文件夹");
        if (!picked || !picked.ok || !picked.path || picked.path.trim().length === 0) return false;
        return updateWorkspace(workspaceId, { folder: picked.path });
      } catch {
        showNote("error", SESSION_ORGANIZE_ERRORS.workspaceUpdate);
        return false;
      }
    },
    [updateWorkspace, showNote]
  );

  const deleteWorkspace = useCallback(
    async (workspaceId: string) => {
      const res = await run<void>(SESSION_ORGANIZE_ERRORS.workspaceDelete, (a) =>
        a.workspacesDelete(workspaceId)
      );
      if (!res.ok) return false;
      setWorkspaces((prev) => prev.filter((w) => w.workspace_id !== workspaceId));
      setSessions((prev) => detachWorkspaceSessions(prev, workspaceId));
      changed();
      return true;
    },
    [run, changed]
  );

  const removeLocal = useCallback(
    (sessionId: string) => {
      setSessions((prev) => removeSession(prev, sessionId));
      changed();
    },
    [changed]
  );

  return {
    sessions,
    workspaces,
    loading,
    loadError,
    canOrganize,
    note,
    clearNote: useCallback(() => setNote(null), []),
    inlineMutation,
    failedRename,
    clearFailedRename: useCallback(() => setFailedRename(null), []),
    reload: useCallback(() => load(false), [load]),
    removeLocal,
    renameSession,
    setPinned,
    setArchived,
    moveSession,
    exportSession,
    createSession,
    createWorkspace,
    renameWorkspace,
    pickWorkspaceFolder,
    unbindWorkspaceFolder,
    deleteWorkspace,
  };
}
