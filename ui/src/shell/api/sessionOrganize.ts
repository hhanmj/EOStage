/**
 * sessionOrganize.ts
 * 会话归档/置顶/工作区归类的纯逻辑模块（侧栏「工作区 · 会话」与历史页共用）
 *
 * 无 React、无 DOM、不访问 import.meta.env；仅导入生成契约类型。
 */

import { t } from "../../i18n/index.ts";

import type {
  AgentSessionSummary,
  AgentWorkspace,
  AgentSessionExportFormat,
} from "../../lib/generated/v3-contracts.ts";

export const WORKSPACE_DELETE_CONFIRM_TEXT = {
  get title() { return t("shell.sessions.deleteWorkspaceTitle"); },
  get body() { return t("shell.sessions.deleteWorkspaceBody"); },
  get confirm() { return t("shell.sessions.delete"); },
  get cancel() { return t("shell.sessions.cancel"); },
} as const;

export const SESSION_ORGANIZE_ERRORS = {
  get rename() { return t("shell.sessions.errors.rename"); },
  get pin() { return t("shell.sessions.errors.pin"); },
  get move() { return t("shell.sessions.errors.move"); },
  get archive() { return t("shell.sessions.errors.archive"); },
  get export() { return t("shell.sessions.errors.export"); },
  get workspaceCreate() { return t("shell.sessions.errors.workspaceCreate"); },
  get workspaceUpdate() { return t("shell.sessions.errors.workspaceUpdate"); },
  get workspaceDelete() { return t("shell.sessions.errors.workspaceDelete"); },
} as const;

export type SessionMenuActionKind =
  | "rename"
  | "pin"
  | "unpin"
  | "move"
  | "archive"
  | "unarchive"
  | "export"
  | "delete";

export type SessionMenuAction =
  | { kind: "rename" }
  | { kind: "pin" }
  | { kind: "unpin" }
  | { kind: "move"; workspaceId: string | null }
  | { kind: "archive" }
  | { kind: "unarchive" }
  | { kind: "export"; format: AgentSessionExportFormat }
  | { kind: "delete" };

export type WorkspaceMenuAction =
  | { kind: "rename" }
  | { kind: "pin" }
  | { kind: "unpin" }
  | { kind: "set-folder" }
  | { kind: "unbind-folder" }
  | { kind: "manage-all" }
  | { kind: "delete" };

export interface MenuNode<A> {
  id: string;
  label: string;
  action?: A;
  items?: MenuNode<A>[];
  danger?: boolean;
}

export interface WorkspaceGroup {
  workspace: AgentWorkspace;
  sessions: AgentSessionSummary[];
}

export interface SessionTreeModel {
  groups: WorkspaceGroup[];
  unfiled: AgentSessionSummary[];
  unfiledLabel: string;
}

function timeOf(iso: string | null | undefined): number {
  if (typeof iso !== "string" || iso.length === 0) return Number.NEGATIVE_INFINITY;
  const parsed = Date.parse(iso);
  return Number.isNaN(parsed) ? Number.NEGATIVE_INFINITY : parsed;
}

export function isPinned(session: AgentSessionSummary): boolean {
  return typeof session.pinned_at === "string" && session.pinned_at.length > 0;
}

export function isArchived(session: AgentSessionSummary): boolean {
  return typeof session.archived_at === "string" && session.archived_at.length > 0;
}

/**
 * 会话排序：置顶在前（置顶时间新者更前），其余按 updated_at 倒序；
 * 时间不可解析的条目排在同组末尾，顺序稳定。
 */
export function compareSessions(a: AgentSessionSummary, b: AgentSessionSummary): number {
  const pinnedA = isPinned(a);
  const pinnedB = isPinned(b);
  if (pinnedA !== pinnedB) return pinnedA ? -1 : 1;
  if (pinnedA && pinnedB) {
    const diff = timeOf(b.pinned_at) - timeOf(a.pinned_at);
    if (diff !== 0) return diff;
  }
  return timeOf(b.updated_at) - timeOf(a.updated_at);
}

export function orderSessions(sessions: AgentSessionSummary[]): AgentSessionSummary[] {
  if (!Array.isArray(sessions)) return [];
  return [...sessions].sort(compareSessions);
}

/** 工作区排序：创建时间升序，时间相同或缺失时按名称本地序，保证列表稳定。 */
export function orderWorkspaces(workspaces: AgentWorkspace[]): AgentWorkspace[] {
  if (!Array.isArray(workspaces)) return [];
  return [...workspaces].sort((a, b) => {
    const diff = timeOf(a.created_at) - timeOf(b.created_at);
    if (diff !== 0 && Number.isFinite(diff)) return diff;
    return a.name.localeCompare(b.name, "zh-Hans-CN");
  });
}

/** 侧栏只显示未归档会话。 */
export function sidebarSessions(sessions: AgentSessionSummary[]): AgentSessionSummary[] {
  if (!Array.isArray(sessions)) return [];
  return sessions.filter((s) => !isArchived(s));
}

export type HistoryArchiveFilter = "all" | "archived";

/** 历史页筛选：「全部」含已归档（带标签），「已归档」只留已归档。 */
export function filterHistorySessions(
  sessions: AgentSessionSummary[],
  filter: HistoryArchiveFilter
): AgentSessionSummary[] {
  if (!Array.isArray(sessions)) return [];
  return filter === "archived" ? sessions.filter(isArchived) : [...sessions];
}

/**
 * 按工作区分组：已存在的工作区各成一组（空组保留，便于拖入与新建后可见），
 * 其余会话（含 workspace_id 指向已删除工作区的会话）进入未归入分组。
 */
export function buildSessionTree(
  sessions: AgentSessionSummary[],
  workspaces: AgentWorkspace[]
): SessionTreeModel {
  const orderedWorkspaces = orderWorkspaces(workspaces);
  const known = new Set(orderedWorkspaces.map((w) => w.workspace_id));
  const buckets = new Map<string, AgentSessionSummary[]>();
  for (const workspace of orderedWorkspaces) {
    buckets.set(workspace.workspace_id, []);
  }
  const unfiled: AgentSessionSummary[] = [];

  for (const session of Array.isArray(sessions) ? sessions : []) {
    const workspaceId = session.workspace_id;
    if (typeof workspaceId === "string" && known.has(workspaceId)) {
      buckets.get(workspaceId)!.push(session);
    } else {
      unfiled.push(session);
    }
  }

  return {
    groups: orderedWorkspaces.map((workspace) => ({
      workspace,
      sessions: orderSessions(buckets.get(workspace.workspace_id) ?? []),
    })),
    unfiled: orderSessions(unfiled),
    unfiledLabel: orderedWorkspaces.length > 0 ? t("shell.sessions.unfiled") : t("shell.sessions.recent"),
  };
}

/**
 * 会话行「⋯」菜单模型。
 * includeDelete：仅历史页为 true（删除走历史页的二次确认）。
 * organize=false：演示模式无后端，只保留删除（本地移除）。
 */
export function buildSessionMenu(
  session: AgentSessionSummary,
  workspaces: AgentWorkspace[],
  options: { includeDelete?: boolean; organize?: boolean } = {}
): MenuNode<SessionMenuAction>[] {
  if (options.organize === false) {
    return options.includeDelete
      ? [{ id: "delete", label: t("shell.sessions.delete"), action: { kind: "delete" }, danger: true }]
      : [];
  }
  const items: MenuNode<SessionMenuAction>[] = [
    { id: "rename", label: t("shell.sessions.rename"), action: { kind: "rename" } },
    isPinned(session)
      ? { id: "unpin", label: t("shell.sessions.unpin"), action: { kind: "unpin" } }
      : { id: "pin", label: t("shell.sessions.pin"), action: { kind: "pin" } },
  ];

  const moveItems: MenuNode<SessionMenuAction>[] = orderWorkspaces(workspaces)
    .filter((w) => w.workspace_id !== session.workspace_id)
    .map((w) => ({
      id: `move:${w.workspace_id}`,
      label: w.name,
      action: { kind: "move", workspaceId: w.workspace_id },
    }));
  if (typeof session.workspace_id === "string" && session.workspace_id.length > 0) {
    moveItems.push({ id: "move:none", label: t("shell.sessions.moveOut"), action: { kind: "move", workspaceId: null } });
  }
  if (moveItems.length > 0) {
    items.push({ id: "move", label: t("shell.sessions.move"), items: moveItems });
  }

  items.push(
    isArchived(session)
      ? { id: "unarchive", label: t("shell.sessions.unarchive"), action: { kind: "unarchive" } }
      : { id: "archive", label: t("shell.sessions.archive"), action: { kind: "archive" } },
    { id: "export:markdown", label: t("shell.sessions.exportMarkdown"), action: { kind: "export", format: "markdown" } },
    { id: "export:json", label: t("shell.sessions.exportJson"), action: { kind: "export", format: "json" } }
  );

  if (options.includeDelete) {
    items.push({ id: "delete", label: t("shell.sessions.delete"), action: { kind: "delete" }, danger: true });
  }
  return items;
}

/** 工作区标题「⋯」菜单模型。 */
export function buildWorkspaceMenu(
  workspace: AgentWorkspace,
  options?: { pinned?: boolean; includeManageAll?: boolean }
): MenuNode<WorkspaceMenuAction>[] {
  const bound = typeof workspace.folder === "string" && workspace.folder.length > 0;
  const items: MenuNode<WorkspaceMenuAction>[] = [
    { id: "rename", label: t("shell.sessions.rename"), action: { kind: "rename" } },
  ];
  if (options && typeof options.pinned === "boolean") {
    items.push(
      options.pinned
        ? { id: "unpin", label: "取消置顶", action: { kind: "unpin" } }
        : { id: "pin", label: "置顶项目", action: { kind: "pin" } }
    );
  }
  items.push({
    id: "set-folder",
    label: bound ? t("shell.sessions.changeFolder") : t("shell.sessions.setFolder"),
    action: { kind: "set-folder" },
  });
  if (bound) {
    items.push({ id: "unbind-folder", label: t("shell.sessions.unbindFolder"), action: { kind: "unbind-folder" } });
  }
  items.push({ id: "delete", label: t("shell.sessions.deleteWorkspace"), action: { kind: "delete" }, danger: true });
  if (options?.includeManageAll) {
    items.push({ id: "manage-all", label: "管理全部对话…", action: { kind: "manage-all" } });
  }
  return items;
}

/** 菜单可聚焦项（含子菜单项）按出现顺序展开，供键盘导航使用。 */
export function flattenMenu<A>(items: MenuNode<A>[]): MenuNode<A>[] {
  const out: MenuNode<A>[] = [];
  for (const item of items) {
    out.push(item);
    if (item.items) out.push(...flattenMenu(item.items));
  }
  return out;
}

/** 列表内替换一条会话（后端返回更新后的快照）。 */
export function replaceSession(
  sessions: AgentSessionSummary[],
  updated: AgentSessionSummary
): AgentSessionSummary[] {
  let found = false;
  const next = sessions.map((s) => {
    if (s.session_id === updated.session_id) {
      found = true;
      return updated;
    }
    return s;
  });
  return found ? next : [...next, updated];
}

export function removeSession(
  sessions: AgentSessionSummary[],
  sessionId: string
): AgentSessionSummary[] {
  return sessions.filter((s) => s.session_id !== sessionId);
}

/** 工作区被删除后，其会话回到未归入分组（与后端一致，本地立即生效）。 */
export function detachWorkspaceSessions(
  sessions: AgentSessionSummary[],
  workspaceId: string
): AgentSessionSummary[] {
  return sessions.map((s) =>
    s.workspace_id === workspaceId ? { ...s, workspace_id: null } : s
  );
}

/** 重命名输入规整：去首尾空白；为空视为未改动（返回 null）。 */
export function normalizeRenameInput(
  draft: string,
  current: string
): { commit: string } | null {
  const trimmed = typeof draft === "string" ? draft.trim() : "";
  if (trimmed.length === 0) return null;
  if (trimmed === current.trim()) return null;
  return { commit: trimmed };
}

/** 导出成功提示：只报文件名，不复述操作。 */
export function exportNoteText(filePath: string): string {
  const name = String(filePath).split(/[\\/]/).filter(Boolean).pop() ?? String(filePath);
  return t("shell.sessions.exported", { name });
}

export function defaultWorkspaceName(existing: AgentWorkspace[]): string {
  const base = t("shell.sessions.defaultWorkspace");
  const names = new Set((Array.isArray(existing) ? existing : []).map((w) => w.name));
  if (!names.has(base)) return base;
  for (let i = 2; i < 1000; i += 1) {
    const candidate = `${base} ${i}`;
    if (!names.has(candidate)) return candidate;
  }
  return base;
}
