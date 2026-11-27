import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import {
  AlertCircle,
  Check,
  ChevronDown,
  ChevronRight,
  Clock,
  Folder,
  FolderOpen,
  FolderPlus,
  ListFilter,
  MoreHorizontal,
  Pin,
  Plus,
  Search,
  SlidersHorizontal,
  X,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { t, useI18n } from "@/i18n";
import type { ShellRoute } from "@/shell/api/shellRoutes";
import type { AgentSessionSummary, AgentWorkspace } from "../../lib/generated/v3-contracts.ts";
import { formatHistoryTime, historyTitle } from "../../shell/api/agentHistoryLogic.ts";
import {
  buildSessionMenu,
  buildSessionTree,
  buildWorkspaceMenu,
  defaultWorkspaceName,
  isPinned,
  normalizeRenameInput,
  sidebarSessions,
  type SessionMenuAction,
  type WorkspaceMenuAction,
} from "../../shell/api/sessionOrganize.ts";
import { InlineRenameInput, OrganizeNoteBar, SessionActionMenu } from "./SessionActionMenu";
import { useSessionOrganizer } from "./useSessionOrganizer";
import { useDelayedFlag } from "@/hooks/useLoadingPrimitives";
import { SessionRowSkeletons } from "@/components/ui/Skeleton";
import { OptimisticInlineStatus } from "@/components/ui/OptimisticInlineStatus";

const RECENT_SESSION_LIMIT = 30;
const COLLAPSED_STORAGE_KEY = "eostage.shell.collapsedWorkspaces";
const PINNED_WORKSPACES_STORAGE_KEY = "eostage.shell.pinnedWorkspaces";

export type SidebarSessionFilterMode = "by-project" | "recent" | "pinned" | "active";

function readPinnedWorkspaces(): Set<string> {
  try {
    const raw =
      typeof window !== "undefined" ? window.localStorage.getItem(PINNED_WORKSPACES_STORAGE_KEY) : null;
    const parsed = raw ? JSON.parse(raw) : [];
    return new Set(Array.isArray(parsed) ? parsed.filter((v) => typeof v === "string") : []);
  } catch {
    return new Set();
  }
}

function writePinnedWorkspaces(ids: Set<string>): void {
  try {
    window.localStorage.setItem(PINNED_WORKSPACES_STORAGE_KEY, JSON.stringify([...ids]));
  } catch {
    // ignore
  }
}

function readCollapsed(): Set<string> {
  try {
    const raw = typeof window !== "undefined" ? window.localStorage.getItem(COLLAPSED_STORAGE_KEY) : null;
    const parsed = raw ? JSON.parse(raw) : [];
    return new Set(Array.isArray(parsed) ? parsed.filter((v) => typeof v === "string") : []);
  } catch {
    return new Set();
  }
}

function writeCollapsed(ids: Set<string>): void {
  try {
    window.localStorage.setItem(COLLAPSED_STORAGE_KEY, JSON.stringify([...ids]));
  } catch {
    // ignore
  }
}

export type ResolvedSessionRunStatus =
  | "running"
  | "awaiting_approval"
  | "failed"
  | "cancelled"
  | "completed"
  | "idle";

export function resolveSessionRunDot(rawStatus?: string | null): {
  status: ResolvedSessionRunStatus;
  visible: boolean;
  className: string;
  label: string;
} {
  const normalized = rawStatus?.trim().toLowerCase() ?? "";
  switch (normalized) {
    case "running":
      return {
        status: "running",
        visible: true,
        className: "bg-[#2563eb] animate-pulse",
        label: t("shell.sidebar.statusRunning"),
      };
    case "awaiting_approval":
      return {
        status: "awaiting_approval",
        visible: true,
        className: "bg-[#d97706]",
        label: t("shell.sidebar.statusAwaitingApproval"),
      };
    case "failed":
    case "error":
      return {
        status: "failed",
        visible: true,
        className: "bg-[#dc2626]",
        label: t("shell.sidebar.statusFailed"),
      };
    case "cancelled":
    case "interrupted":
      return {
        status: "cancelled",
        visible: false,
        className: "",
        label: t("shell.sidebar.statusCancelled"),
      };
    case "completed":
      return {
        status: "completed",
        visible: false,
        className: "",
        label: t("shell.sidebar.statusCompleted"),
      };
    default:
      return {
        status: "idle",
        visible: false,
        className: "",
        label: t("shell.sidebar.statusIdle"),
      };
  }
}

export interface SidebarSessionTreeProps {
  activeRoute: ShellRoute | null;
  activeSessionId?: string | null;
  onOpenSession: (sessionId: string) => void;
  onNavigate: (route: ShellRoute) => void;
  refreshKey?: string | null;
  fixtureSessions?: (() => AgentSessionSummary[]) | null;
  onNewChat?: () => void;
  externalSearchFocusTrigger?: number;
  activeSessionRunStatus?: string | null;
  sessionRunStatuses?: Record<string, string>;
}

export function SidebarSessionTree({
  activeRoute,
  activeSessionId,
  onOpenSession,
  onNavigate,
  refreshKey,
  fixtureSessions,
  onNewChat,
  externalSearchFocusTrigger = 0,
  activeSessionRunStatus,
  sessionRunStatuses,
}: SidebarSessionTreeProps) {
  const { locale } = useI18n();
  const organizer = useSessionOrganizer({ refreshKey, fixtureSessions });
  const { workspaces, canOrganize } = organizer;
  const showLoadingSkeleton = useDelayedFlag(organizer.loading);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [renamingWorkspace, setRenamingWorkspace] = useState<string | null>(null);

  useEffect(() => {
    const failed = organizer.failedRename;
    if (!failed) return;
    if (failed.kind === "session") {
      setRenaming(failed.targetId);
    } else if (failed.kind === "workspace") {
      setRenamingWorkspace(failed.targetId);
    }
  }, [organizer.failedRename]);

  useEffect(() => {
    if (organizer.inlineMutation.state === "saved") {
      setRenaming(null);
      setRenamingWorkspace(null);
    }
  }, [organizer.inlineMutation.state]);
  const [collapsed, setCollapsed] = useState<Set<string>>(readCollapsed);
  const [pinnedWorkspaces, setPinnedWorkspaces] = useState<Set<string>>(readPinnedWorkspaces);
  const [confirmDelete, setConfirmDelete] = useState<AgentWorkspace | null>(null);
  const [projectsCollapsed, setProjectsCollapsed] = useState(false);
  const [recentCollapsed, setRecentCollapsed] = useState(false);
  const [showSearchInput, setShowSearchInput] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [showMoreMenu, setShowMoreMenu] = useState(false);
  const [showFilterMenu, setShowFilterMenu] = useState(false);
  const [sessionFilterMode, setSessionFilterMode] = useState<SidebarSessionFilterMode>("by-project");
  const [draggedSessionId, setDraggedSessionId] = useState<string | null>(null);
  const [dragOverTarget, setDragOverTarget] = useState<string | null>(null);
  const [pointerGhost, setPointerGhost] = useState<{
    sessionId: string;
    title: string;
    currentWorkspaceId: string | null;
    x: number;
    y: number;
  } | null>(null);
  const suppressNextClickRef = useRef(false);
  const pointerCleanupRef = useRef<(() => void) | null>(null);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const moreMenuRef = useRef<HTMLDivElement>(null);
  const filterMenuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    return () => {
      pointerCleanupRef.current?.();
      pointerCleanupRef.current = null;
    };
  }, []);

  useEffect(() => {
    if (externalSearchFocusTrigger > 0) {
      setShowSearchInput(true);
      setTimeout(() => searchInputRef.current?.focus(), 30);
    }
  }, [externalSearchFocusTrigger]);

  useEffect(() => {
    if (!showMoreMenu && !showFilterMenu) return;
    const handleClickOutside = (e: MouseEvent) => {
      if (moreMenuRef.current && !moreMenuRef.current.contains(e.target as Node)) {
        setShowMoreMenu(false);
      }
      if (filterMenuRef.current && !filterMenuRef.current.contains(e.target as Node)) {
        setShowFilterMenu(false);
      }
    };
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setShowMoreMenu(false);
        setShowFilterMenu(false);
      }
    };
    document.addEventListener("mousedown", handleClickOutside);
    window.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("mousedown", handleClickOutside);
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [showMoreMenu, showFilterMenu]);

  const tree = useMemo(
    () => buildSessionTree(sidebarSessions(organizer.sessions), workspaces),
    [organizer.sessions, workspaces, locale]
  );

  const orderedGroups = useMemo(() => {
    if (pinnedWorkspaces.size === 0) return tree.groups;
    const pinned: typeof tree.groups = [];
    const regular: typeof tree.groups = [];
    for (const g of tree.groups) {
      if (pinnedWorkspaces.has(g.workspace.workspace_id)) {
        pinned.push(g);
      } else {
        regular.push(g);
      }
    }
    return [...pinned, ...regular];
  }, [tree.groups, pinnedWorkspaces]);

  const unfiled = useMemo(() => {
    const q = searchQuery.trim().toLowerCase();
    const visibleAll = sidebarSessions(organizer.sessions);
    let sourceList: AgentSessionSummary[];
    if (sessionFilterMode === "recent") {
      sourceList = [...visibleAll].sort(
        (a, b) => Date.parse(b.updated_at || "") - Date.parse(a.updated_at || "")
      );
    } else if (sessionFilterMode === "pinned") {
      sourceList = visibleAll.filter((s) => isPinned(s));
    } else if (sessionFilterMode === "active") {
      sourceList = visibleAll.filter((s) => {
        const isCurrent = s.session_id === activeSessionId;
        const raw =
          sessionRunStatuses?.[s.session_id] ??
          (isCurrent ? activeSessionRunStatus : null) ??
          s.last_run_status;
        return resolveSessionRunDot(raw).visible;
      });
    } else {
      sourceList = tree.unfiled;
    }
    const bounded = sourceList.slice(0, RECENT_SESSION_LIMIT);
    if (!q) return bounded;
    return bounded.filter((s) => historyTitle(s.title).toLowerCase().includes(q));
  }, [
    activeSessionId,
    activeSessionRunStatus,
    organizer.sessions,
    searchQuery,
    sessionFilterMode,
    sessionRunStatuses,
    tree.unfiled,
  ]);

  const now = new Date();

  const toggleCollapsed = useCallback((id: string) => {
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      writeCollapsed(next);
      return next;
    });
  }, []);

  const handleSessionAction = (session: AgentSessionSummary, action: SessionMenuAction) => {
    switch (action.kind) {
      case "rename":
        setRenaming(session.session_id);
        break;
      case "pin":
      case "unpin":
        void organizer.setPinned(session.session_id, action.kind === "pin");
        break;
      case "archive":
      case "unarchive":
        void organizer.setArchived(session.session_id, action.kind === "archive");
        break;
      case "move":
        void organizer.moveSession(session.session_id, action.workspaceId);
        break;
      case "export":
        void organizer.exportSession(session.session_id, action.format);
        break;
      default:
        break;
    }
  };

  const toggleWorkspacePinned = useCallback((id: string, pin: boolean) => {
    setPinnedWorkspaces((prev) => {
      const next = new Set(prev);
      if (pin) next.add(id);
      else next.delete(id);
      writePinnedWorkspaces(next);
      return next;
    });
  }, []);

  const handleWorkspaceAction = (workspace: AgentWorkspace, action: WorkspaceMenuAction) => {
    switch (action.kind) {
      case "rename":
        setRenamingWorkspace(workspace.workspace_id);
        break;
      case "pin":
        toggleWorkspacePinned(workspace.workspace_id, true);
        break;
      case "unpin":
        toggleWorkspacePinned(workspace.workspace_id, false);
        break;
      case "set-folder":
        void organizer.pickWorkspaceFolder(workspace.workspace_id);
        break;
      case "unbind-folder":
        void organizer.unbindWorkspaceFolder(workspace.workspace_id);
        break;
      case "manage-all":
        onNavigate("history");
        break;
      case "delete":
        setConfirmDelete(workspace);
        break;
    }
  };

  const handleCreateWorkspace = async () => {
    const created = await organizer.createWorkspace(defaultWorkspaceName(workspaces));
    if (created) setRenamingWorkspace(created.workspace_id);
  };

  const handleNewSessionIn = async (workspaceId: string) => {
    const created = await organizer.createSession(workspaceId);
    if (created) {
      setCollapsed((prev) => {
        if (!prev.has(workspaceId)) return prev;
        const next = new Set(prev);
        next.delete(workspaceId);
        writeCollapsed(next);
        return next;
      });
      onOpenSession(created.session_id);
    }
  };

  const resolveDraggedSessionId = (e: React.DragEvent): string | null => {
    const fromTransfer = e.dataTransfer?.getData("text/plain")?.trim();
    return fromTransfer || draggedSessionId;
  };

  const handleDropOnWorkspace = (e: React.DragEvent, workspaceId: string) => {
    e.preventDefault();
    e.stopPropagation();
    setDragOverTarget(null);
    const sid = resolveDraggedSessionId(e);
    setDraggedSessionId(null);
    if (!sid || !canOrganize) return;
    setCollapsed((prev) => {
      if (!prev.has(workspaceId)) return prev;
      const next = new Set(prev);
      next.delete(workspaceId);
      writeCollapsed(next);
      return next;
    });
    void organizer.moveSession(sid, workspaceId);
  };

  const handleDropOnUnfiled = (e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setDragOverTarget(null);
    const sid = resolveDraggedSessionId(e);
    setDraggedSessionId(null);
    if (!sid || !canOrganize) return;
    setRecentCollapsed(false);
    void organizer.moveSession(sid, null);
  };

  const resolveDropTargetFromElement = (el: Element | null | undefined): string | null => {
    if (!el || typeof el.closest !== "function") return null;
    const wsEl = el.closest("[data-workspace]");
    if (wsEl) {
      const wsId = wsEl.getAttribute("data-workspace");
      if (wsId) return wsId;
    }
    if (el.closest("[data-unfiled-drop], [data-testid='sidebar-recent-section']")) {
      return "__unfiled__";
    }
    return null;
  };

  const handleSessionPointerDown = (
    e: React.PointerEvent<HTMLElement>,
    session: AgentSessionSummary,
    title: string
  ) => {
    if (!canOrganize || e.button !== 0) return;
    if ((e.target as Element | null)?.closest("[data-session-menu-wrap]")) return;
    pointerCleanupRef.current?.();

    const startX = e.clientX;
    const startY = e.clientY;
    const currentWsId = session.workspace_id ?? null;
    let active = false;
    let latestTarget: string | null = null;

    const hitAt = (clientX: number, clientY: number, fallbackTarget: EventTarget | null): string | null => {
      let fromPoint: Element | null = null;
      if (typeof document !== "undefined" && typeof document.elementFromPoint === "function") {
        fromPoint = document.elementFromPoint(clientX, clientY);
      }
      return (
        resolveDropTargetFromElement(fromPoint) ??
        resolveDropTargetFromElement(fallbackTarget as Element | null)
      );
    };

    const cleanup = () => {
      window.removeEventListener("pointermove", onMove, true);
      window.removeEventListener("pointerup", onUp, true);
      window.removeEventListener("pointercancel", onCancel, true);
      window.removeEventListener("keydown", onKeyDown, true);
      if (pointerCleanupRef.current === cleanup) {
        pointerCleanupRef.current = null;
      }
    };

    const resetVisual = () => {
      setDraggedSessionId(null);
      setDragOverTarget(null);
      setPointerGhost(null);
    };

    const onMove = (ev: PointerEvent) => {
      const dist = Math.hypot(ev.clientX - startX, ev.clientY - startY);
      if (!active) {
        if (dist < 5) return;
        active = true;
        suppressNextClickRef.current = true;
        setProjectsCollapsed(false);
        setDraggedSessionId(session.session_id);
      }
      ev.preventDefault();
      const target = hitAt(ev.clientX, ev.clientY, ev.target);
      latestTarget = target;
      setDragOverTarget(target);
      setPointerGhost({
        sessionId: session.session_id,
        title,
        currentWorkspaceId: currentWsId,
        x: ev.clientX,
        y: ev.clientY,
      });
    };

    const onUp = (ev: PointerEvent) => {
      const wasActive = active;
      const resolved = wasActive ? (hitAt(ev.clientX, ev.clientY, ev.target) ?? latestTarget) : null;
      cleanup();
      resetVisual();
      if (!wasActive) return;

      window.setTimeout(() => {
        suppressNextClickRef.current = false;
      }, 60);

      if (!resolved) return;
      if (resolved === "__unfiled__") {
        if (currentWsId !== null) {
          setRecentCollapsed(false);
          void organizer.moveSession(session.session_id, null);
        }
        return;
      }
      if (resolved !== currentWsId) {
        setCollapsed((prev) => {
          if (!prev.has(resolved)) return prev;
          const next = new Set(prev);
          next.delete(resolved);
          writeCollapsed(next);
          return next;
        });
        void organizer.moveSession(session.session_id, resolved);
      }
    };

    const onCancel = () => {
      cleanup();
      resetVisual();
      suppressNextClickRef.current = false;
    };

    const onKeyDown = (ev: KeyboardEvent) => {
      if (ev.key === "Escape" && active) {
        ev.preventDefault();
        onCancel();
      }
    };

    pointerCleanupRef.current = cleanup;
    window.addEventListener("pointermove", onMove, true);
    window.addEventListener("pointerup", onUp, true);
    window.addEventListener("pointercancel", onCancel, true);
    window.addEventListener("keydown", onKeyDown, true);
  };

  const renderSessionRow = (session: AgentSessionSummary, indent: boolean, rowIndex = 0) => {
    const title = historyTitle(session.title);
    const isActive = activeRoute === "agent" && session.session_id === activeSessionId;
    const isCurrentSession = session.session_id === activeSessionId;
    const runDot = resolveSessionRunDot(
      sessionRunStatuses?.[session.session_id] ??
        (isCurrentSession ? activeSessionRunStatus : null) ??
        session.last_run_status
    );
    if (renaming === session.session_id) {
      const initialTitle =
        organizer.failedRename?.kind === "session" &&
        organizer.failedRename.targetId === session.session_id
          ? organizer.failedRename.draft
          : title;
      return (
        <div key={session.session_id} className={cn("flex h-[30px] shrink-0 items-center px-1", indent && "pl-5")}>
          <InlineRenameInput
            initial={initialTitle}
            label={t("shell.sessions.name")}
            onCommit={(value) => {
              organizer.clearFailedRename();
              setRenaming(null);
              const change = normalizeRenameInput(value, title);
              if (change) void organizer.renameSession(session.session_id, change.commit);
            }}
            onCancel={() => {
              organizer.clearFailedRename();
              setRenaming(null);
            }}
          />
        </div>
      );
    }
    const menuItems = buildSessionMenu(session, workspaces, { organize: canOrganize });
    const isDragging = draggedSessionId === session.session_id;
    return (
      <div
        key={session.session_id}
        data-session-row={session.session_id}
        draggable={canOrganize}
        onPointerDown={(e) => handleSessionPointerDown(e, session, title)}
        onDragStart={(e) => {
          setDraggedSessionId(session.session_id);
          try {
            e.dataTransfer?.setData("text/plain", session.session_id);
            if (e.dataTransfer) e.dataTransfer.effectAllowed = "move";
          } catch {
            // ignore in synthetic test environments
          }
        }}
        onDragEnd={() => {
          setDraggedSessionId(null);
          setDragOverTarget(null);
        }}
        style={{ "--row-index": Math.min(rowIndex, 10) } as React.CSSProperties}
        className={cn(
          "sp-stagger-fade-row group relative flex h-[30px] w-full shrink-0 items-center rounded-[9px] transition-colors",
          isActive
            ? "bg-black/[0.072] font-medium text-[#18181b] dark:bg-white/[0.09] dark:text-white"
            : "text-[#3f3f46] hover:bg-black/[0.045] dark:text-[#a1a1aa] dark:hover:bg-white/[0.052]",
          isDragging && "opacity-50"
        )}
      >
        <button
          type="button"
          onClick={(e) => {
            if (suppressNextClickRef.current) {
              e.preventDefault();
              e.stopPropagation();
              suppressNextClickRef.current = false;
              return;
            }
            onOpenSession(session.session_id);
          }}
          title={title}
          aria-current={isActive ? "true" : undefined}
          className={cn(
            "flex h-full min-w-0 flex-1 items-center gap-2 rounded-[9px] pl-3 text-left text-[12.5px] outline-none cursor-pointer overflow-hidden whitespace-nowrap",
            menuItems.length > 0 ? "pr-8" : "pr-2.5",
            indent && "pl-6"
          )}
        >
          {runDot.visible && (
            <span
              data-testid={`session-status-dot-${session.session_id}`}
              data-run-status={runDot.status}
              title={runDot.label}
              aria-label={runDot.label}
              className={cn("h-1.5 w-1.5 shrink-0 rounded-full", runDot.className)}
            />
          )}
          {isPinned(session) && (
            <Pin className="h-3 w-3 shrink-0 text-[#71717a]" aria-label={t("shell.sessions.pinned")} />
          )}
          <span className="min-w-0 flex-1 truncate">{title}</span>
          <span
            className={cn(
              "shrink-0 text-xs text-[#a1a1aa] tabular-nums",
              menuItems.length > 0 && "group-hover:hidden group-focus-within:hidden"
            )}
          >
            {formatHistoryTime(session.updated_at, now)}
          </span>
        </button>
        {menuItems.length > 0 && (
          <div data-session-menu-wrap className="absolute right-1 top-1/2 -translate-y-1/2">
            <SessionActionMenu
              items={menuItems}
              label={t("shell.sessions.actions")}
              onSelect={(action) => handleSessionAction(session, action)}
            />
          </div>
        )}
      </div>
    );
  };

  return (
    <div className="mt-3 flex min-h-0 flex-1 flex-col overflow-y-auto select-none">
      {/* 1. 项目 (Projects) 独立分组（头部含 `•••` 与 `+` 新建项目，空状态不堆冗余占位行；按项目筛选外收起分组视图） */}
      {canOrganize && sessionFilterMode === "by-project" && (
        <div className="pb-2" data-testid="sidebar-projects-section">
          <div className="relative flex items-center justify-between px-3 pt-1 pb-1">
            <button
              type="button"
              onClick={() => setProjectsCollapsed((prev) => !prev)}
              className="flex items-center gap-1 text-[12px] font-medium text-[#71717a] transition-colors hover:text-[#18181b] cursor-pointer dark:text-[#a1a1aa] dark:hover:text-white"
            >
              <span>{t("shell.sidebar.projectsHeading")}</span>
              {orderedGroups.length > 0 &&
                (projectsCollapsed ? (
                  <ChevronRight className="h-3.5 w-3.5 text-[#a1a1aa]" />
                ) : (
                  <ChevronDown className="h-3.5 w-3.5 text-[#a1a1aa]" />
                ))}
            </button>

            <div className="flex items-center gap-0.5 text-[#71717a]">
              <div ref={moreMenuRef} className="relative">
                <button
                  type="button"
                  onClick={() => setShowMoreMenu((prev) => !prev)}
                  aria-haspopup="true"
                  aria-expanded={showMoreMenu}
                  title={t("shell.sidebar.moreOptionsAria")}
                  aria-label={t("shell.sidebar.moreOptionsAria")}
                  className="flex h-[22px] w-[22px] items-center justify-center rounded-[6px] transition-colors hover:bg-black/[0.06] hover:text-[#18181b] cursor-pointer dark:hover:bg-white/[0.07] dark:hover:text-white"
                >
                  <MoreHorizontal className="h-[14px] w-[14px] stroke-[1.75]" />
                </button>

                {showMoreMenu && (
                  <div className="sp-popover-blur-in origin-top-right absolute top-full right-0 z-50 mt-1 w-44 rounded-[12px] border border-[#e4e4e7] bg-white p-1 shadow-lg dark:border-[#27272a] dark:bg-[#1e1e22]">
                    <button
                      type="button"
                      onClick={() => {
                        setShowMoreMenu(false);
                        onNavigate("history");
                      }}
                      className="flex w-full items-center gap-2 rounded-[7px] px-2.5 py-1.5 text-left text-[12px] text-[#3f3f46] hover:bg-[#f4f4f5] cursor-pointer dark:text-[#a1a1aa] dark:hover:bg-[#27272a]"
                    >
                      <SlidersHorizontal className="h-3.5 w-3.5" />
                      <span>{t("shell.nav.allChats")}</span>
                    </button>
                  </div>
                )}
              </div>

              <button
                type="button"
                onClick={() => void handleCreateWorkspace()}
                title={t("shell.sessions.newWorkspace")}
                aria-label={t("shell.sessions.newWorkspace")}
                className="flex h-[22px] w-[22px] items-center justify-center rounded-[6px] transition-colors hover:bg-black/[0.06] hover:text-[#18181b] cursor-pointer dark:hover:bg-white/[0.07] dark:hover:text-white"
              >
                <Plus className="h-[14px] w-[14px] stroke-[1.75]" />
              </button>
            </div>
          </div>

          {!projectsCollapsed && orderedGroups.length > 0 && (
            <div className="px-2">
              <div className="flex flex-col gap-0.5" role="list" aria-label={t("shell.sessions.workspaces")}>
                {orderedGroups.map(({ workspace, sessions }) => {
                  const isCollapsed = collapsed.has(workspace.workspace_id);
                  const isWsPinned = pinnedWorkspaces.has(workspace.workspace_id);
                  const FolderIcon = isWsPinned ? Pin : isCollapsed ? Folder : FolderOpen;
                  const isDragTarget = dragOverTarget === workspace.workspace_id;
                  return (
                    <div
                      key={workspace.workspace_id}
                      role="listitem"
                      data-workspace={workspace.workspace_id}
                      data-workspace-pinned={isWsPinned ? "true" : undefined}
                      onDragOver={(e) => {
                        e.preventDefault();
                        if (dragOverTarget !== workspace.workspace_id) {
                          setDragOverTarget(workspace.workspace_id);
                        }
                      }}
                      onDragLeave={(e) => {
                        if (
                          dragOverTarget === workspace.workspace_id &&
                          !e.currentTarget.contains(e.relatedTarget as Node | null)
                        ) {
                          setDragOverTarget(null);
                        }
                      }}
                      onDrop={(e) => handleDropOnWorkspace(e, workspace.workspace_id)}
                      className={cn(
                        "flex flex-col gap-0.5 rounded-[9px] transition-colors",
                        isDragTarget && "bg-[#10b981]/10 ring-1 ring-[#10b981]"
                      )}
                    >
                      {renamingWorkspace === workspace.workspace_id ? (
                        <div className="flex h-[30px] shrink-0 items-center px-1">
                          <InlineRenameInput
                            initial={
                              organizer.failedRename?.kind === "workspace" &&
                              organizer.failedRename.targetId === workspace.workspace_id
                                ? organizer.failedRename.draft
                                : workspace.name
                            }
                            label={t("shell.sessions.workspaceName")}
                            onCommit={(value) => {
                              organizer.clearFailedRename();
                              setRenamingWorkspace(null);
                              const change = normalizeRenameInput(value, workspace.name);
                              if (change) void organizer.renameWorkspace(workspace.workspace_id, change.commit);
                            }}
                            onCancel={() => {
                              organizer.clearFailedRename();
                              setRenamingWorkspace(null);
                            }}
                          />
                        </div>
                      ) : (
                        <div className="group relative flex h-[30px] w-full shrink-0 items-center rounded-[9px] text-[#3f3f46] transition-colors hover:bg-black/[0.045] dark:text-[#a1a1aa] dark:hover:bg-white/[0.052]">
                          <button
                            type="button"
                            aria-expanded={!isCollapsed}
                            onClick={() => toggleCollapsed(workspace.workspace_id)}
                            title={workspace.folder ? `${workspace.name}\n${workspace.folder}` : workspace.name}
                            className="flex h-full min-w-0 flex-1 items-center gap-2 rounded-[9px] pl-2.5 pr-14 text-left text-[12.5px] outline-none cursor-pointer overflow-hidden whitespace-nowrap"
                          >
                            <FolderIcon className="h-3.5 w-3.5 shrink-0 text-[#52525b] dark:text-[#a1a1aa]" aria-hidden="true" />
                            <span className="min-w-0 flex-1 truncate font-medium text-[#18181b] dark:text-[#f4f4f5]">
                              {workspace.name}
                            </span>
                          </button>
                          <div className="absolute right-1 top-1/2 flex -translate-y-1/2 items-center gap-0.5">
                            <SessionActionMenu
                              items={buildWorkspaceMenu(workspace, {
                                pinned: isWsPinned,
                                includeManageAll: true,
                              })}
                              label={t("shell.sessions.workspaceActions")}
                              onSelect={(action) => handleWorkspaceAction(workspace, action)}
                            />
                            <button
                              type="button"
                              aria-label={t("shell.sessions.newChatInWorkspace", { name: workspace.name })}
                              title={t("shell.sessions.newChat")}
                              onClick={() => void handleNewSessionIn(workspace.workspace_id)}
                              className="flex h-6 w-6 shrink-0 items-center justify-center rounded text-[#71717a] opacity-0 transition-opacity hover:bg-[#e4e4e7] hover:text-[#18181b] group-hover:opacity-100 cursor-pointer dark:hover:bg-[#27272a] dark:hover:text-white"
                            >
                              <Plus className="h-3.5 w-3.5" aria-hidden="true" />
                            </button>
                          </div>
                        </div>
                      )}
                      {!isCollapsed && (
                        <div className="flex flex-col gap-0.5">
                          {sessions.map((s, idx) => renderSessionRow(s, true, idx))}
                        </div>
                      )}
                    </div>
                  );
                })}
              </div>
            </div>
          )}
        </div>
      )}

      {/* 2. 最近 (Recent Standalone Chats) 独立分组（会话侧栏组织：下方「最近」，展示未归入项目的独立对话，支持拖入上方项目或从项目拖回） */}
      {draggedSessionId &&
        organizer.sessions.some(
          (s) => s.session_id === draggedSessionId && Boolean(s.workspace_id)
        ) && (
          <div
            data-unfiled-drop="true"
            data-testid="sidebar-unfile-dropzone"
            onDragOver={(e) => {
              e.preventDefault();
              if (dragOverTarget !== "__unfiled__") setDragOverTarget("__unfiled__");
            }}
            onDrop={handleDropOnUnfiled}
            className={cn(
              "mx-2 mb-1 flex h-7 shrink-0 items-center justify-center rounded-[6px] border border-dashed px-2 text-xs transition-colors",
              dragOverTarget === "__unfiled__"
                ? "border-[#10b981] bg-[#10b981]/15 font-medium text-[#047857] dark:text-[#34d399]"
                : "border-[#d4d4d8] text-[#71717a] dark:border-[#3f3f46] dark:text-[#a1a1aa]"
            )}
          >
            {dragOverTarget === "__unfiled__"
              ? t("shell.sidebar.dropOutOfFolderActive")
              : t("shell.sidebar.dropOutOfFolderIdle")}
          </div>
        )}
      <div
        className={cn(
          "flex min-h-0 flex-1 flex-col rounded-[6px] transition-colors",
          dragOverTarget === "__unfiled__" && "bg-[#10b981]/10 ring-1 ring-[#10b981]"
        )}
        data-testid="sidebar-recent-section"
        onDragOver={(e) => {
          e.preventDefault();
          if (dragOverTarget !== "__unfiled__") {
            setDragOverTarget("__unfiled__");
          }
        }}
        onDragLeave={(e) => {
          if (
            dragOverTarget === "__unfiled__" &&
            !e.currentTarget.contains(e.relatedTarget as Node | null)
          ) {
            setDragOverTarget(null);
          }
        }}
        onDrop={handleDropOnUnfiled}
      >
        <div className="flex items-center justify-between px-3 py-1">
          <button
            type="button"
            onClick={() => setRecentCollapsed((prev) => !prev)}
            className="flex items-center gap-1 text-[12px] font-medium text-[#71717a] transition-colors hover:text-[#18181b] cursor-pointer dark:text-[#a1a1aa] dark:hover:text-white"
          >
            <Clock className="h-[13px] w-[13px] stroke-[1.75] hidden" />
            <span>
              {sessionFilterMode === "recent"
                ? t("shell.sidebar.filterModeRecent")
                : sessionFilterMode === "pinned"
                  ? t("shell.sidebar.filterModePinned")
                  : sessionFilterMode === "active"
                    ? t("shell.sidebar.filterModeActive")
                    : t("shell.sidebar.filterModeRecentDefault")}
            </span>
            <span className="sr-only">{tree.unfiledLabel}</span>
            {recentCollapsed ? (
              <ChevronRight className="h-3.5 w-3.5 text-[#a1a1aa]" />
            ) : (
              <ChevronDown className="h-3.5 w-3.5 text-[#a1a1aa]" />
            )}
          </button>

          <div className="flex items-center gap-0.5 text-[#71717a]">
            <button
              type="button"
              onClick={() => {
                setShowSearchInput((prev) => {
                  const next = !prev;
                  if (next) {
                    setTimeout(() => searchInputRef.current?.focus(), 30);
                  } else {
                    setSearchQuery("");
                  }
                  return next;
                });
              }}
              title={t("shell.sidebar.filterSessionsAria")}
              aria-label={t("shell.sidebar.filterSessionsAria")}
              className="flex h-[22px] w-[22px] items-center justify-center rounded-[6px] text-[#71717a] transition-colors hover:bg-black/[0.06] hover:text-[#18181b] cursor-pointer dark:hover:bg-white/[0.07] dark:hover:text-white"
            >
              <Search className="h-[12.5px] w-[12.5px] stroke-[1.75]" />
            </button>

            <div ref={filterMenuRef} className="relative">
              <button
                type="button"
                data-testid="sidebar-session-filter-btn"
                aria-haspopup="menu"
                aria-expanded={showFilterMenu}
                onClick={() => setShowFilterMenu((prev) => !prev)}
                title={t("shell.sidebar.viewAndSortAria")}
                aria-label={t("shell.sidebar.viewAndSortAria")}
                className={cn(
                  "flex h-[22px] w-[22px] items-center justify-center rounded-[6px] transition-colors hover:bg-black/[0.06] hover:text-[#18181b] cursor-pointer dark:hover:bg-white/[0.07] dark:hover:text-white",
                  sessionFilterMode !== "by-project" ? "bg-black/[0.06] text-[#18181b] dark:bg-white/[0.09] dark:text-white" : "text-[#71717a]"
                )}
              >
                <ListFilter className="h-[12.5px] w-[12.5px] stroke-[1.75]" />
              </button>

              {showFilterMenu && (
                <div
                  role="menu"
                  aria-label={t("shell.sidebar.viewAndSortAria")}
                  className="sp-popover-blur-in origin-top-right absolute top-full right-0 z-50 mt-1 w-36 rounded-[12px] border border-[#e4e4e7] bg-white p-1 shadow-lg dark:border-[#27272a] dark:bg-[#1e1e22]"
                >
                  {(
                    [
                      { id: "by-project", label: t("shell.sidebar.filterModeByProject"), icon: Folder },
                      { id: "recent", label: t("shell.sidebar.filterModeRecent"), icon: Clock },
                      { id: "pinned", label: t("shell.sidebar.filterModePinned"), icon: Pin },
                      { id: "active", label: t("shell.sidebar.filterModeActive"), icon: AlertCircle },
                    ] as const
                  ).map((opt) => {
                    const OptIcon = opt.icon;
                    const active = sessionFilterMode === opt.id;
                    return (
                      <button
                        key={opt.id}
                        type="button"
                        role="menuitemradio"
                        aria-checked={active}
                        onClick={() => {
                          setSessionFilterMode(opt.id);
                          setShowFilterMenu(false);
                        }}
                        className="flex w-full items-center gap-2 rounded-[7px] px-2.5 py-1.5 text-left text-[12px] text-[#3f3f46] hover:bg-[#f4f4f5] cursor-pointer dark:text-[#d4d4d8] dark:hover:bg-[#27272a]"
                      >
                        <OptIcon className="h-3.5 w-3.5 shrink-0 text-[#71717a]" />
                        <span className="flex-1 truncate">{opt.label}</span>
                        {active && <Check className="h-3.5 w-3.5 shrink-0 text-[#18181b] dark:text-white" />}
                      </button>
                    );
                  })}
                </div>
              )}
            </div>

            <button
              type="button"
              onClick={() => {
                if (onNewChat) onNewChat();
                else onNavigate("agent");
              }}
              title={t("shell.sidebar.newSessionAria")}
              aria-label={t("shell.sidebar.newSessionAria")}
              className="flex h-[22px] w-[22px] items-center justify-center rounded-[6px] text-[#71717a] transition-colors hover:bg-black/[0.06] hover:text-[#18181b] cursor-pointer dark:hover:bg-white/[0.07] dark:hover:text-white"
            >
              <Plus className="h-[13.5px] w-[13.5px] stroke-[1.75]" />
            </button>
          </div>
        </div>

        {showSearchInput && (
          <div className="px-3 py-1">
            <div className="flex h-[28px] items-center gap-1.5 rounded-[9px] border border-black/[0.08] bg-white/90 px-2.5 shadow-[0_1px_2px_rgba(0,0,0,0.03)] dark:border-white/[0.08] dark:bg-[#1e1e22]">
              <Search className="h-3 w-3 shrink-0 text-[#a1a1aa]" />
              <input
                ref={searchInputRef}
                type="text"
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
                placeholder={t("shell.sidebar.searchSessionsPlaceholder")}
                className="w-full bg-transparent text-[12px] text-[#18181b] placeholder-[#a1a1aa] focus:outline-none dark:text-[#f4f4f5]"
              />
              {searchQuery && (
                <button
                  type="button"
                  onClick={() => setSearchQuery("")}
                  className="text-[#a1a1aa] hover:text-[#52525b] cursor-pointer"
                >
                  <X className="h-3 w-3" />
                </button>
              )}
            </div>
          </div>
        )}

        {!recentCollapsed && (
          <div className="flex min-h-0 flex-1 flex-col overflow-y-auto px-2">
            {organizer.loading && organizer.sessions.length === 0 ? (
              <div className="min-h-[160px]" data-testid="sidebar-unfiled">
                {showLoadingSkeleton ? <SessionRowSkeletons rows={5} /> : null}
              </div>
            ) : unfiled.length === 0 ? (
              <div className="min-h-[24px] flex-1" data-testid="sidebar-unfiled" />
            ) : (
              <div className="flex flex-col gap-0.5" data-testid="sidebar-unfiled">
                {unfiled.map((s, idx) => renderSessionRow(s, false, idx))}
              </div>
            )}
          </div>
        )}
      </div>

      {organizer.inlineMutation.state !== "idle" && (
        <div className="px-2 pt-1">
          <OptimisticInlineStatus
            state={organizer.inlineMutation.state}
            errorMessage={organizer.inlineMutation.errorMessage}
            onRetry={organizer.inlineMutation.retry}
            testId="sidebar-session-mutation-status"
          />
        </div>
      )}

      {organizer.note && (
        <div className="px-1 pt-1">
          <OrganizeNoteBar note={organizer.note} onDone={organizer.clearNote} />
        </div>
      )}

      {pointerGhost &&
        typeof document !== "undefined" &&
        createPortal(
          <div
            data-testid="sidebar-drag-ghost"
            style={{
              left: `${pointerGhost.x + 12}px`,
              top: `${pointerGhost.y + 10}px`,
            }}
            className="pointer-events-none fixed z-[9999] flex max-w-[240px] items-center gap-1.5 rounded-[8px] border border-[#e4e4e7] bg-white/95 px-2.5 py-1.5 text-xs font-medium text-[#18181b] shadow-lg backdrop-blur-xs dark:border-[#27272a] dark:bg-[#1e1e22]/95 dark:text-[#f4f4f5]"
          >
            <FolderOpen className="h-3.5 w-3.5 shrink-0 text-[#52525b] dark:text-[#a1a1aa]" />
            <span className="truncate">{pointerGhost.title}</span>
            {dragOverTarget && (
              <span className="ml-1 shrink-0 rounded bg-[#10b981]/15 px-1.5 py-0.5 text-xs font-semibold text-[#047857] dark:text-[#34d399]">
                {dragOverTarget === "__unfiled__"
                  ? pointerGhost.currentWorkspaceId
                    ? t("shell.sidebar.dragMoveOut")
                    : t("shell.sidebar.dragRecent")
                  : t("shell.sidebar.dragMoveInto", {
                      name: workspaces.find((w) => w.workspace_id === dragOverTarget)?.name ?? t("shell.sidebar.dragFolderFallback"),
                    })}
              </span>
            )}
          </div>,
          document.body
        )}

      {confirmDelete && (
        <WorkspaceDeleteDialog
          workspace={confirmDelete}
          onCancel={() => setConfirmDelete(null)}
          onConfirm={async () => {
            const ok = await organizer.deleteWorkspace(confirmDelete.workspace_id);
            if (ok) setConfirmDelete(null);
            return ok;
          }}
        />
      )}
    </div>
  );
}

function WorkspaceDeleteDialog({
  workspace,
  onCancel,
  onConfirm,
}: {
  workspace: AgentWorkspace;
  onCancel: () => void;
  onConfirm: () => Promise<boolean>;
}) {
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);
  const cancelRef = React.useRef<HTMLButtonElement | null>(null);
  const returnFocus = React.useRef<HTMLElement | null>(
    typeof document !== "undefined" ? (document.activeElement as HTMLElement | null) : null
  );

  useEffect(() => {
    cancelRef.current?.focus();
    const previous = returnFocus.current;
    return () => {
      if (previous && previous.isConnected) previous.focus();
    };
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !busy) onCancel();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [busy, onCancel]);

  if (typeof document === "undefined") return null;
  return createPortal(
    <div
      role="alertdialog"
      aria-modal="true"
      aria-label={t("shell.sessions.deleteWorkspaceTitle")}
      className="fixed inset-0 z-[1300] flex items-center justify-center bg-black/45 dark:bg-black/65 backdrop-blur-[2px] p-4"
    >
      <div
        className="w-full max-w-sm space-y-4 rounded-modal border border-sp-line bg-sp-surface p-5 shadow-modal dark:bg-sp-card"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center gap-2 text-sm font-semibold text-sp-text">
          <AlertCircle className="h-4 w-4 shrink-0 text-sp-danger-text" aria-hidden="true" />
          <h3 className="min-w-0 truncate" title={workspace.name}>
            {t("shell.sessions.deleteWorkspaceTitle")}
          </h3>
        </div>
        <p className="text-xs leading-relaxed text-sp-text">{t("shell.sessions.deleteWorkspaceBody")}</p>
        {failed && (
          <div role="alert" className="rounded-sp-sm bg-sp-danger-bg p-2 text-xs text-sp-danger-text">
            {t("shell.sessions.deleteWorkspaceFailed")}
          </div>
        )}
        <div className="flex items-center justify-end gap-2 pt-1">
          <button
            ref={cancelRef}
            type="button"
            disabled={busy}
            onClick={onCancel}
            className="flex h-8 items-center rounded-sp-sm bg-sp-hover px-3 text-xs font-medium text-sp-text transition-colors hover:bg-sp-selected disabled:opacity-50 cursor-pointer"
          >
            {t("shell.sessions.cancel")}
          </button>
          <button
            type="button"
            disabled={busy}
            onClick={async () => {
              setBusy(true);
              setFailed(false);
              const ok = await onConfirm();
              setBusy(false);
              if (!ok) setFailed(true);
            }}
            className="flex h-8 items-center rounded-sp-sm bg-sp-danger-strong px-3 text-xs font-medium text-white transition-opacity hover:opacity-90 disabled:opacity-50 cursor-pointer"
          >
            {t("shell.sessions.delete")}
          </button>
        </div>
      </div>
    </div>,
    document.body
  );
}
