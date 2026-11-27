export * from "./types.ts";
export * from "./taskLogic.ts";
export * from "./agentLogic.ts";
export * from "./agentTypes.ts";
export * from "./startupLoad.ts";
export * from "./taskRowSync.ts";
export * from "./rowProjection.ts";
export * from "./taskOverviewSource.ts";
export * from "./taskActionFlow.ts";
export * from "./autoCancelFlow.ts";
export * from "./agentSessionMemory.ts";
export * from "./agentSearchViewSync.ts";

import { isTauriDesktopHost } from "../../lib/desktopShellBridge.ts";
import type { UnifiedSearchRequest, SearchStartParams } from "./types.ts";
import { RESULT_PAGE_SIZE } from "./batchDeliveryLogic.ts";
import * as prod from "./production.ts";

/**
 * 编译期常量折叠与 DCE：
 * - 在 Vite 生产构建时，import.meta.env.DEV 被替换为字面量 false，死代码消除彻底剔除 fixture 模块
 * - 在 Vite 开发环境中 (import.meta.env.DEV === true)，加载 fixture
 * - 在 Node 单测环境中 (import.meta.env 为 undefined)，回退并加载 fixture
 */
// @ts-ignore
const isDev = import.meta.env?.DEV !== false;
const fixture = isDev ? await import("./fixture.ts") : null;

/**
 * 宿主与环境驱动的后端决策（每次调用时动态评估）：
 * - isTauriDesktopHost() 为真 → 生产后端（真实命令），无论 DEV 与否
 * - 非 Tauri 宿主 + DEV 环境（浏览器预览、#harness、Node 单测）→ fixture 演示数据
 * - 否则 → 生产后端（生产浏览器构建显示诚实的不可用态）
 */
export const isFixtureBackend = (): boolean => !isTauriDesktopHost() && isDev;

export const __fixture = isDev;

export const listSources = async () =>
  isFixtureBackend() && fixture ? fixture.listSources() : prod.listSources();
export const getSourceDescriptors = async () =>
  isFixtureBackend() && fixture ? fixture.getSourceDescriptors() : prod.getSourceDescriptors();
export const getSourceViews = async () =>
  isFixtureBackend() && fixture ? fixture.getSourceViews() : prod.getSourceViews();
export const getSourceView = async (id: string) =>
  isFixtureBackend() && fixture ? fixture.getSourceView(id) : prod.getSourceView(id);
export const searchStart = async (
  p: UnifiedSearchRequest | SearchStartParams,
  options?: { bypassCache?: boolean },
) =>
  isFixtureBackend() && fixture ? fixture.searchStart(p as any, options) : prod.searchStart(p, options);
export const getSearchProgress = async (id: string) =>
  isFixtureBackend() && fixture ? fixture.getSearchProgress(id) : prod.getSearchProgress(id);
export const cancelSearch = async (id: string) =>
  isFixtureBackend() && fixture ? fixture.cancelSearch(id) : prod.cancelSearch(id);
export const cancelCatalogSearch = async (id: string) =>
  isFixtureBackend() && fixture ? fixture.cancelCatalogSearch(id) : prod.cancelCatalogSearch(id);
export const getBatchPage = async (b: string, r: number, p = 0, s = RESULT_PAGE_SIZE) =>
  isFixtureBackend() && fixture ? fixture.getBatchPage(b, r, p, s) : prod.getBatchPage(b, r, p, s);
export const createTask = async (b: string, r: number, ids: string[], opt?: any) =>
  isFixtureBackend() && fixture ? fixture.createTask(b, r, ids, opt) : prod.createTask(b, r, ids, opt);
export const previewAllMatchTask = async (req: import("./types.ts").BatchAllMatchPreviewRequest) =>
  isFixtureBackend() && fixture ? fixture.previewAllMatchTask(req) : prod.previewAllMatchTask(req);
export const createAllMatchTask = async (req: import("./types.ts").BatchAllMatchCreateTaskRequest) =>
  isFixtureBackend() && fixture ? fixture.createAllMatchTask(req) : prod.createAllMatchTask(req);
export const taskAction = async (id: string, r: number, a: any) =>
  isFixtureBackend() && fixture ? fixture.taskAction(id, r, a) : prod.taskAction(id, r, a);
export const isAccountStatusesWired = (): boolean =>
  isFixtureBackend() && fixture ? fixture.isAccountStatusesWired() : prod.isAccountStatusesWired();
export const getAccountStatuses = async () =>
  isFixtureBackend() && fixture ? fixture.getAccountStatuses() : prod.getAccountStatuses();
export const getTasks = async () =>
  isFixtureBackend() && fixture ? fixture.getTasks() : prod.getTasks();
export const retryTaskFallback = async (id: string, confirmRefreshedBatchId: string | null = null) =>
  isFixtureBackend() && fixture ? fixture.retryTaskFallback(id, confirmRefreshedBatchId) : prod.retryTaskFallback(id, confirmRefreshedBatchId);
export const clearTaskHistory = async () =>
  isFixtureBackend() && fixture ? fixture.clearTaskHistory() : prod.clearTaskHistory();
export const openOutputDirectory = async (ref: { task_id: string; revision: number }) =>
  isFixtureBackend() && fixture ? fixture.openOutputDirectory(ref) : prod.openOutputDirectory(ref);
export const testNetworkConnection = async () =>
  prod.testNetworkConnection();
export const isAgentWired = (): boolean =>
  isFixtureBackend() && fixture ? fixture.isAgentWired() : prod.isAgentWired();
export const getModelChannelReadiness = async () =>
  isFixtureBackend() && fixture ? fixture.getModelChannelReadiness() : prod.getModelChannelReadiness();
export const getAgentSessions = async () =>
  isFixtureBackend() && fixture ? fixture.getAgentSessions() : prod.getAgentSessions();
export const getAgentSessionsSync = () =>
  isFixtureBackend() && fixture ? fixture.getAgentSessionsSync() : prod.getAgentSessionsSync();
export const getAgentSubsceneApproval = async (subscene: string | null) =>
  isFixtureBackend() && fixture ? fixture.getAgentSubsceneApproval(subscene) : prod.getAgentSubsceneApproval(subscene);
export const getAgentSubsceneApprovalSync = (subscene: string | null) =>
  isFixtureBackend() && fixture ? fixture.getAgentSubsceneApprovalSync(subscene) : prod.getAgentSubsceneApprovalSync(subscene);
export const getAgentConclusions = async () =>
  isFixtureBackend() && fixture ? fixture.getAgentConclusions() : prod.getAgentConclusions();
export const getAgentConclusionsSync = () =>
  isFixtureBackend() && fixture ? fixture.getAgentConclusionsSync() : prod.getAgentConclusionsSync();
export const getFixtureNewSessionInitialMessage = async () =>
  isFixtureBackend() && fixture ? fixture.getFixtureNewSessionInitialMessage() : prod.getFixtureNewSessionInitialMessage();
export const getFixtureNewSessionInitialMessageSync = () =>
  isFixtureBackend() && fixture ? fixture.getFixtureNewSessionInitialMessageSync() : prod.getFixtureNewSessionInitialMessageSync();
export const getFixtureAssistantGreeting = async () =>
  isFixtureBackend() && fixture ? fixture.getFixtureAssistantGreeting() : prod.getFixtureAssistantGreeting();
export const getFixtureAssistantGreetingSync = () =>
  isFixtureBackend() && fixture ? fixture.getFixtureAssistantGreetingSync() : prod.getFixtureAssistantGreetingSync();
export const getAgentPlanResourceSummary = async (sessionId?: string) =>
  isFixtureBackend() && fixture ? fixture.getAgentPlanResourceSummary(sessionId) : prod.getAgentPlanResourceSummary(sessionId);
export const getAgentPlanResourceSummarySync = (sessionId?: string) =>
  isFixtureBackend() && fixture ? fixture.getAgentPlanResourceSummarySync(sessionId) : prod.getAgentPlanResourceSummarySync(sessionId);
export const getFixtureWorkingAoi = () =>
  isFixtureBackend() && fixture ? fixture.getFixtureWorkingAoi() : prod.getFixtureWorkingAoi();
export const getFixtureStorageRoot = () =>
  isFixtureBackend() && fixture ? fixture.getFixtureStorageRoot() : prod.getFixtureStorageRoot();

export const __resetFixtureState = () => {
  if (fixture) fixture.__resetFixtureState();
};
export const __setSearchOutcome = (o: "done" | "failed") => {
  if (fixture) fixture.__setSearchOutcome(o);
};
export const __setFixtureChannelsConfigured = (c: boolean) => {
  if (fixture) fixture.__setFixtureChannelsConfigured(c);
};
