import React, { useEffect, useState, useMemo, useRef } from "react";
import { Pin } from "lucide-react";
import { useDelayedFlag } from "@/hooks/useLoadingPrimitives";
import { SessionRowSkeletons } from "@/components/ui/Skeleton";
import { OptimisticInlineStatus } from "@/components/ui/OptimisticInlineStatus";
import type { AgentSessionSummary } from "../../lib/generated/v3-contracts.ts";
import {
  groupSessionsByDay,
  nextHistoryDeleteState,
  type HistoryDeleteState,
} from "../../shell/api/agentHistoryLogic.ts";
import {
  buildSessionMenu,
  filterHistorySessions,
  isArchived,
  isPinned,
  normalizeRenameInput,
  orderSessions,
  type HistoryArchiveFilter,
  type SessionMenuAction,
} from "../../shell/api/sessionOrganize.ts";
import { getDesktopShellBridge, isTauriDesktopHost } from "@/lib/desktopShellBridge";
import { EmptyState } from "@/components/ui/EmptyState";
import { EmptyStateIcon, EmptyStateWireframe } from "@/components/ui/EmptyStateVisuals";
import { Popconfirm } from "@/components/ui/Popconfirm";
import { InlineRenameInput, OrganizeNoteBar, SessionActionMenu } from "./SessionActionMenu";
import { useSessionOrganizer } from "./useSessionOrganizer";
import { useI18n } from "@/i18n";

export interface AgentHistoryPageProps {
  onOpenSession: (sessionId: string) => void;
  onSessionDeleted?: (sessionId: string) => void;
}

function isDevBuild(): boolean {
  return typeof import.meta.env !== "undefined" && Boolean(import.meta.env.DEV);
}

/** 开发模式演示数据按需加载，避免本页静态依赖整个对话工作台模块 */
async function loadFixtureSessions(): Promise<AgentSessionSummary[]> {
  const mod = await import("./DesktopAgentWorkbench");
  return mod.historyFixtureSessionsFactory ? mod.historyFixtureSessionsFactory() : [];
}

export const AgentHistoryPage: React.FC<AgentHistoryPageProps> = ({
  onOpenSession,
  onSessionDeleted,
}) => {
  const { t } = useI18n();
  const filters: ReadonlyArray<{ key: HistoryArchiveFilter; label: string }> = [
    { key: "all", label: t("agent.history.all") },
    { key: "archived", label: t("agent.history.archived") },
  ];
  const deleteConfirmText = {
    title: t("agent.history.deleteTitle"),
    body: t("agent.history.deleteBody"),
    confirm: t("agent.history.delete"),
    cancel: t("agent.cancel"),
  };
  const isDesktop = isTauriDesktopHost();
  const isDev = isDevBuild();

  const organizer = useSessionOrganizer({
    includeArchived: true,
    fixtureSessions: !isDesktop && isDev ? loadFixtureSessions : null,
  });
  const { sessions, workspaces, loading, loadError, canOrganize } = organizer;
  const showLoadingSkeleton = useDelayedFlag(loading);
  const [filter, setFilter] = useState<HistoryArchiveFilter>("all");
  const [renaming, setRenaming] = useState<string | null>(null);
  const [deleteState, setDeleteState] = useState<HistoryDeleteState>({ phase: "idle" });
  const [deleteAnchorEl, setDeleteAnchorEl] = useState<HTMLElement | null>(null);
  const containerRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    const failed = organizer.failedRename;
    if (failed?.kind === "session") {
      setRenaming(failed.targetId);
    }
  }, [organizer.failedRename]);

  useEffect(() => {
    if (organizer.inlineMutation.state === "saved") {
      setRenaming(null);
    }
  }, [organizer.inlineMutation.state]);

  const handleRequestDelete = (sessionId: string) => {
    const rows = Array.from(containerRef.current?.querySelectorAll("[data-history-row]") ?? []);
    const matchedRow = rows.find((el) => el.getAttribute("data-history-row") === sessionId);
    const btnEl = (matchedRow?.querySelector('button[aria-haspopup]') ?? matchedRow?.querySelector("button") ?? matchedRow ?? null) as HTMLElement | null;
    setDeleteAnchorEl(btnEl);
    setDeleteState((prev) => nextHistoryDeleteState(prev, { type: "request", sessionId }));
  };

  const handleConfirmDelete = async () => {
    if (deleteState.phase !== "confirming") return;
    const targetId = deleteState.sessionId;
    setDeleteState((prev) => nextHistoryDeleteState(prev, { type: "confirm" }));

    // 演示模式下不触碰后端真实服务，仅本地列表移除
    if (!isTauriDesktopHost() && isDevBuild()) {
      organizer.removeLocal(targetId);
      onSessionDeleted?.(targetId);
      setDeleteState((prev) => nextHistoryDeleteState(prev, { type: "succeeded" }));
      return;
    }

    try {
      const bridge = getDesktopShellBridge();
      if (!bridge?.agent) {
        throw new Error("unavailable");
      }
      await bridge.agent.sessionsDelete(targetId);
      organizer.removeLocal(targetId);
      onSessionDeleted?.(targetId);
      setDeleteState((prev) => nextHistoryDeleteState(prev, { type: "succeeded" }));
    } catch {
      setDeleteState((prev) =>
        nextHistoryDeleteState(prev, { type: "failed", message: t("agent.history.deleteFailed") })
      );
    }
  };

  const handleAction = (session: AgentSessionSummary, action: SessionMenuAction) => {
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
      case "delete":
        handleRequestDelete(session.session_id);
        break;
    }
  };

  const visible = useMemo(() => filterHistorySessions(sessions, filter), [sessions, filter]);
  const byId = useMemo(() => new Map(sessions.map((s) => [s.session_id, s])), [sessions]);
  const workspaceNames = useMemo(
    () => new Map(workspaces.map((w) => [w.workspace_id, w.name])),
    [workspaces]
  );

  // 生产环境且非桌面宿主
  if (!isDesktop && !isDev) {
    return (
      <div className="flex flex-1 h-full w-full items-center justify-center p-6 bg-sp-surface">
        <EmptyState
          kind="no-access"
          size="page"
          visual={<EmptyStateWireframe variant="no-access" />}
          title={t("agent.history.desktopOnly")}
          description="当前处于浏览器预览环境，请在 EOStage 桌面客户端中打开以访问本地历史记录。"
        />
      </div>
    );
  }

  const now = new Date();
  const pinned = !loading && !loadError && sessions.length > 0 && filter === "all"
    ? orderSessions(visible.filter(isPinned))
    : [];
  const rest = !loading && !loadError && sessions.length > 0
    ? (filter === "all" ? visible.filter((s) => !isPinned(s)) : visible)
    : [];
  const groups = [
    ...(pinned.length > 0
      ? [{ key: "pinned", label: t("agent.history.pinned"), ids: pinned.map((s) => s.session_id) }]
      : []),
    ...groupSessionsByDay(rest, now).map((g) => ({
      key: g.key,
      label: g.label,
      ids: g.items.map((i) => i.sessionId),
    })),
  ];
  const timeLabels = new Map(
    groupSessionsByDay(visible, now).flatMap((g) => g.items.map((i) => [i.sessionId, i] as const))
  );

  return (
    <div ref={containerRef} className="flex flex-1 h-full w-full flex-col overflow-hidden bg-sp-surface">
      {/* 顶部页面栏：骨架/加载态先显示固定标题与筛选栏 */}
      <header className="flex h-[46px] shrink-0 items-center justify-between border-b border-sp-line bg-sp-canvas px-7 select-none">
        <div className="flex items-center gap-2.5">
          <h1 className="text-[14px] font-semibold text-sp-text tracking-[-0.01em]">{t("agent.history.title")}</h1>
          <span className="rounded-[5px] bg-sp-surface-muted px-1.5 py-0.5 text-xs font-medium text-sp-text-secondary">
            {sessions.length}
          </span>
        </div>
        <div
          role="radiogroup"
          aria-label={t("agent.history.filterAria")}
          className="eo-segmented-track"
        >
          {filters.map((f) => (
            <button
              key={f.key}
              type="button"
              role="radio"
              aria-checked={filter === f.key}
              onClick={() => setFilter(f.key)}
              className="eo-segmented-pill h-[24px] px-3 text-[12px] cursor-pointer outline-none"
              data-active={filter === f.key}
            >
              {f.label}
            </button>
          ))}
        </div>
      </header>

      <div className="flex-1 overflow-y-auto">
        {loading ? (
          <div
            data-testid="agent-history-loading"
            className="w-full max-w-[760px] mx-auto pt-5 px-7 pb-12 min-h-[260px]"
          >
            {showLoadingSkeleton ? <SessionRowSkeletons rows={6} /> : null}
          </div>
        ) : loadError ? (
          <div className="flex h-full items-center justify-center p-6">
            <EmptyState
              kind="failed"
              size="page"
              visual={<EmptyStateIcon name="alert" size={48} />}
              title={t("agent.history.loadFailed")}
              description="本地会话数据完好无损，请稍后重试读取。"
              primaryAction={{
                label: t("agent.history.retry"),
                onClick: () => void organizer.reload(),
                loading,
              }}
            />
          </div>
        ) : sessions.length === 0 ? (
          <div className="flex h-full items-center justify-center p-6">
            <EmptyState
              kind="first-use"
              size="page"
              visual={<EmptyStateIcon name="history" size={48} />}
              title={t("agent.history.emptyTitle")}
              description={t("agent.history.emptyDescription")}
            />
          </div>
        ) : (
        <div className="w-full max-w-[760px] mx-auto pt-5 px-7 pb-12">

        {organizer.inlineMutation.state !== "idle" && (
          <div className="mb-2 flex justify-end">
            <OptimisticInlineStatus
              state={organizer.inlineMutation.state}
              errorMessage={organizer.inlineMutation.errorMessage}
              onRetry={organizer.inlineMutation.retry}
              testId="history-session-mutation-status"
            />
          </div>
        )}

        {organizer.note && (
          <OrganizeNoteBar note={organizer.note} onDone={organizer.clearNote} className="mb-3" />
        )}

        {groups.length === 0 && (
          <EmptyState
            kind="no-results"
            size="row"
            title={t(filter === "archived" ? "agent.history.emptyArchived" : "agent.history.empty")}
            primaryAction={
              filter !== "all"
                ? {
                    label: "查看全部会话",
                    onClick: () => setFilter("all"),
                  }
                : undefined
            }
            className="rounded-[11px] border border-sp-line bg-sp-canvas px-4 py-4"
          />
        )}

        {/* 分组会话列表 */}
        <div className="space-y-5">
          {groups.map((group) => (
            <div key={group.key} className="space-y-1.5">
              <div className="text-xs font-medium text-sp-text-muted py-1 px-1 select-none">
                {group.label}
              </div>
              <div className="divide-y divide-sp-divider rounded-[11px] border border-sp-line bg-sp-card overflow-hidden">
                {group.ids.map((id, rowIdx) => {
                  const session = byId.get(id);
                  const item = timeLabels.get(id);
                  if (!session || !item) return null;
                  const workspaceName = session.workspace_id
                    ? workspaceNames.get(session.workspace_id)
                    : undefined;
                  if (renaming === id) {
                    const initialDraft =
                      organizer.failedRename?.kind === "session" &&
                      organizer.failedRename.targetId === id
                        ? organizer.failedRename.draft
                        : item.title;
                    return (
                      <div key={id} className="flex items-center p-2.5 bg-sp-canvas">
                        <InlineRenameInput
                          initial={initialDraft}
                          label={t("agent.history.sessionName")}
                          onCommit={(value) => {
                            organizer.clearFailedRename();
                            setRenaming(null);
                            const change = normalizeRenameInput(value, item.title);
                            if (change) void organizer.renameSession(id, change.commit);
                          }}
                          onCancel={() => {
                            organizer.clearFailedRename();
                            setRenaming(null);
                          }}
                        />
                      </div>
                    );
                  }
                  return (
                    <div
                      key={id}
                      data-history-row={id}
                      style={{ "--row-index": Math.min(rowIdx, 12) } as React.CSSProperties}
                      className="sp-stagger-fade-row group relative flex items-center hover:bg-sp-canvas transition-colors duration-fast ease-sp"
                    >
                      <div
                        role="button"
                        tabIndex={0}
                        onClick={() => onOpenSession(id)}
                        onKeyDown={(e) => {
                          if (e.key === "Enter" || e.key === " ") {
                            e.preventDefault();
                            onOpenSession(id);
                          }
                        }}
                        className="flex-1 min-w-0 px-3.5 py-2.5 pr-10 cursor-pointer outline-none focus-visible:ring-1 focus-visible:ring-sp-accent"
                      >
                        <div className="flex min-w-0 items-center gap-1.5">
                          {isPinned(session) && (
                            <Pin className="h-3 w-3 shrink-0 text-sp-text-muted" aria-label={t("agent.history.pinnedAria")} />
                          )}
                          <div
                            className="min-w-0 text-[13px] text-sp-text truncate font-medium leading-snug"
                            title={item.title}
                          >
                            {item.title}
                          </div>
                          {isArchived(session) && (
                            <span className="shrink-0 rounded-[4px] bg-sp-surface-muted px-1.5 py-0.5 text-xs text-sp-text-secondary leading-normal">
                              {t("agent.history.archived")}
                            </span>
                          )}
                        </div>
                        <div className="flex min-w-0 items-center gap-2 text-xs text-sp-text-muted tabular-nums mt-0.5">
                          <span className="shrink-0">{item.timeLabel}</span>
                          {workspaceName && (
                            <span className="min-w-0 truncate" title={workspaceName}>
                              · {workspaceName}
                            </span>
                          )}
                        </div>
                      </div>
                      <div className="absolute right-2.5 top-1/2 -translate-y-1/2">
                        <SessionActionMenu
                          items={buildSessionMenu(session, workspaces, {
                            includeDelete: true,
                            organize: canOrganize,
                          })}
                          label={t("agent.history.sessionActions")}
                          onSelect={(action) => handleAction(session, action)}
                        />
                      </div>
                    </div>
                  );
                })}
              </div>
            </div>
          ))}
        </div>
        </div>
        )}
      </div>

      <Popconfirm
        open={deleteState.phase !== "idle"}
        anchorEl={deleteAnchorEl}
        title={deleteConfirmText.title}
        description={deleteConfirmText.body}
        error={deleteState.phase === "failed" ? t("agent.history.deleteFailed") : null}
        extra={
          !isTauriDesktopHost() && isDevBuild() ? (
            <div className="text-xs text-sp-text-muted">{t("agent.history.demoDelete")}</div>
          ) : null
        }
        confirmLabel={deleteConfirmText.confirm}
        cancelLabel={deleteConfirmText.cancel}
        tone="danger"
        busy={deleteState.phase === "deleting"}
        onCancel={() => {
          if (deleteState.phase !== "deleting") {
            setDeleteState((prev) => nextHistoryDeleteState(prev, { type: "cancel" }));
          }
        }}
        onConfirm={() => void handleConfirmDelete()}
      />
    </div>
  );
};
