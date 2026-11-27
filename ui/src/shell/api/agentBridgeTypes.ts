/**
 * Agent 桌面桥接类型定义
 * 复用 generated/v3-contracts.ts 中的生成契约，杜绝类型漂移
 */

import type {
  AgentSessionSummary,
  AgentSendContext,
  AgentStreamPage,
  AgentStreamEvent,
  AgentStreamEventKind,
  AgentRunSummary,
  AgentRunDetail,
  AgentChannelView,
  AgentChannelSave,
  AgentChannelTest,
  AgentReasoningMode,
  AgentPermissionMode,
  AgentPermissionModeView,
  AgentStopAllResult,
  AgentActivityStatus,
  AgentWorkspace,
  AgentSessionExport,
  AgentSessionExportFormat,
  AgentWorkItem,
} from "../../lib/generated/v3-contracts.ts";

export type {
  AgentWorkItem,
  AgentWorkspace,
  AgentSessionExport,
  AgentSessionExportFormat,
  AgentSessionSummary,
  AgentSendContext,
  AgentStreamPage,
  AgentStreamEvent,
  AgentStreamEventKind,
  AgentRunSummary,
  AgentRunDetail,
  AgentChannelView,
  AgentChannelSave,
  AgentChannelTest,
  AgentReasoningMode,
  AgentPermissionMode,
  AgentPermissionModeView,
  AgentStopAllResult,
  AgentActivityStatus,
};

export interface AgentChannelDraft {
  id?: string | null;
  protocol: string;
  base_url: string;
  api_key?: string | null;
  model?: string | null;
}

export interface AgentSessionTruncateResult {
  session: AgentSessionSummary;
  cursor: number;
  aoi_restored: boolean;
}

export interface AgentDisabledItems {
  disabled_providers: string[];
  disabled_models: string[];
  disabled_skills: string[];
}

export interface AgentLiveDeltaPayload {
  session_id: string;
  run_id: string;
  step_index: number;
  kind: "thinking_start" | "thinking" | "thinking_end" | "text" | string;
  delta: string;
  started_at_ms?: number | null;
  duration_ms?: number | null;
}

export type DesktopAgentBridge = {
  /** Not archived sessions, pinned first; `includeArchived` adds archived ones. */
  sessionsList(options?: { includeArchived?: boolean }): Promise<AgentSessionSummary[]>;
  sessionsCreate(title?: string | null, workspaceId?: string | null): Promise<AgentSessionSummary>;
  sessionsDelete(sessionId: string): Promise<void>;
  sessionsRename(sessionId: string, title: string): Promise<AgentSessionSummary>;
  sessionsSetPinned(sessionId: string, pinned: boolean): Promise<AgentSessionSummary>;
  sessionsSetArchived(sessionId: string, archived: boolean): Promise<AgentSessionSummary>;
  /** `null` moves the session back to the unfiled group. */
  sessionsMove(sessionId: string, workspaceId: string | null): Promise<AgentSessionSummary>;
  /** File content only; the caller asks the user where to save it. */
  sessionsExport(sessionId: string, format: AgentSessionExportFormat): Promise<AgentSessionExport>;
  /** Writes the export into a folder the user just picked (never overwrites); returns the file path. */
  sessionsExportTo(sessionId: string, format: AgentSessionExportFormat, directory: string): Promise<string>;
  workspacesList(): Promise<AgentWorkspace[]>;
  workspacesCreate(name: string, folder?: string | null): Promise<AgentWorkspace>;
  /** `folder: null` unbinds the folder; an absent field is left unchanged. */
  workspacesUpdate(workspaceId: string, change: { name?: string; folder?: string | null }): Promise<AgentWorkspace>;
  /** Sessions in the workspace return to the unfiled group; files are untouched. */
  workspacesDelete(workspaceId: string): Promise<void>;
  send(sessionId: string, text: string, context?: AgentSendContext | null): Promise<string>;
  streamPoll(sessionId: string, cursor: number): Promise<AgentStreamPage>;
  planApprove(planId: string, snapshot: unknown, outputDir?: string | null): Promise<void>;
  planReject(planId: string): Promise<void>;
  runCancel(runId: string): Promise<boolean>;
  runsList(sessionId: string): Promise<AgentRunSummary[]>;
  runsOpen(runId: string): Promise<AgentRunDetail>;
  /** Runs of a session (newest first) with their request, plans and created download tasks. */
  sessionWork(sessionId: string): Promise<AgentWorkItem[]>;
  /** Truncates session stream before `beforeSeq` (for turn edit/revert) and optionally restores pre-turn AOI. */
  sessionTruncate?(sessionId: string, beforeSeq: number, restoreAoi?: boolean): Promise<AgentSessionTruncateResult>;
  channelsGet(): Promise<AgentChannelView[]>;
  channelsSave(channel: AgentChannelSave): Promise<AgentChannelView[]>;
  channelsTest(channelId: string): Promise<AgentChannelTest>;
  channelsModels(channelId: string): Promise<string[]>;
  channelsTestDraft(draft: AgentChannelDraft): Promise<AgentChannelTest>;
  channelsProbeDraft(draft: AgentChannelDraft): Promise<{ result: AgentChannelTest; models: string[] }>;
  channelsSetDefault(channelId: string): Promise<AgentChannelView[]>;
  channelsDelete(channelId: string): Promise<AgentChannelView[]>;
  channelsRename?(oldId: string, newId: string, apiKey?: string | null): Promise<AgentChannelView[]>;
  disabledItemsGet?(): Promise<AgentDisabledItems>;
  disabledItemsSet?(items: AgentDisabledItems): Promise<AgentDisabledItems>;
  permissionModeGet(sessionId: string | null): Promise<AgentPermissionModeView>;
  permissionModeSetGlobal(mode: AgentPermissionMode): Promise<AgentPermissionModeView>;
  permissionModeSetSession(sessionId: string, mode: AgentPermissionMode | null): Promise<AgentPermissionModeView>;
  stopAll(): Promise<AgentStopAllResult>;
  activityStatus(): Promise<AgentActivityStatus>;
};
