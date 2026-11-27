/**
 * C1 契约生产环境真实接入实现
 * 依据任务书规范：
 * - 绝不直接或间接 import fixture.ts
 * - 经 desktopShellBridge 统一接入后端真实 Tauri 命令
 * - 浏览器预览守卫：非桌面环境时诚实提示「需在桌面应用中运行」，绝不盲目发起调用
 * - 审计 P0 清零：权威字段映射，无编造回退
 */

import { t } from "../../i18n/index.ts";
import { RESULT_PAGE_SIZE } from "./batchDeliveryLogic.ts";
import type {
  SourceDescriptor,
  SourceView,
  AccountStatus,
  AccountSessionStatus,
  SearchProgress,
  BatchPageResult,
  SearchStartParams,
  CreateTaskOptions,
  TaskRef,
  TaskRetryFallbackOutcome,
  ApplicationTask,
  UnifiedSearchRequest,
  StoreTaskSummary,
  TaskOverview,
  SearchCancelOutcome,
  BatchAllMatchPreviewRequest,
  BatchAllMatchPreview,
  BatchAllMatchCreateTaskRequest,
  BatchAllMatchTaskOutcome,
} from "./types.ts";
import type {
  AgentApprovalRecord,
  AgentSession,
  AgentMessage,
  AgentPlanResourceSummary,
} from "./agentTypes.ts";
import { defaultUnifiedClient } from "./unifiedClient.ts";
import { projectSnapshotItemToSceneRow, formatBytes } from "./rowProjection.ts";
import { projectFrozenItemToScene } from "../../lib/applicationServiceProjection.ts";
import { isTauriDesktopHost, getDesktopShellBridge } from "../../lib/desktopShellBridge.ts";
import { humanTaskLabel } from "./taskCardNaming.ts";
import { isTaskHistoryRow } from "./taskLogic.ts";

export const isFixtureBackend = (): boolean => false;

export const isUnifiedWired = (): boolean => true;

export async function listSources(): Promise<SourceDescriptor[]> {
  if (!isTauriDesktopHost()) {
    return [];
  }
  return defaultUnifiedClient.listSources();
}

export async function getSourceDescriptors(): Promise<SourceDescriptor[]> {
  return listSources();
}

export async function getSourceViews(): Promise<SourceView[]> {
  if (!isTauriDesktopHost()) {
    return [];
  }
  return defaultUnifiedClient.sourceViews();
}

export async function getSourceView(resourceId: string): Promise<SourceView | undefined> {
  const views = await getSourceViews();
  return views.find((v) => v.resource_id === resourceId);
}

export async function searchStart(
  request: UnifiedSearchRequest | SearchStartParams,
  options?: { bypassCache?: boolean },
): Promise<SearchProgress> {
  if (!isTauriDesktopHost()) {
    return {
      request_id: "req-browser-guard",
      resource_id: request.resource_id,
      stage: "failed",
      requests_done: 0,
      items_kept: 0,
      items_cap: 0,
      elapsed_ms: 0,
      message_zh: t("shared.production.requireDesktopHost"),
      terminal: true,
      error: {
        code: "DESKTOP_HOST_REQUIRED",
        message_zh: t("shared.production.requireDesktopHost"),
        retryable: false,
        needs_account: null,
        needs_user_confirmation: false,
      },
    };
  }

  // 若已经是权威 UnifiedSearchRequest，直接原样传递给 defaultUnifiedClient（保留 time_range、request_id、filters）
  if ("request_id" in request && "time_range" in request) {
    return defaultUnifiedClient.searchStart(request as UnifiedSearchRequest, options);
  }

  // 兼容遗留 SearchStartParams
  const params = request as SearchStartParams;
  const req: UnifiedSearchRequest = {
    resource_id: params.resource_id,
    aoi: (params.aoi as any) || {},
    aoi_name: params.aoi_name,
    time_range:
      params.start_date || params.end_date
        ? {
            start: params.start_date ?? "",
            end: params.end_date ?? "",
          }
        : null,
    filters: params.filters || {},
    request_id: `req-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
  };

  return defaultUnifiedClient.searchStart(req, options);
}

export async function getSearchProgress(requestId: string): Promise<SearchProgress> {
  if (!isTauriDesktopHost()) {
    return {
      request_id: requestId,
      resource_id: "unknown",
      stage: "failed",
      requests_done: 0,
      items_kept: 0,
      items_cap: 0,
      elapsed_ms: 0,
      message_zh: t("shared.production.requireDesktopHost"),
      terminal: true,
      error: null,
    };
  }

  const progress = await defaultUnifiedClient.searchProgress(requestId);
  if (!progress) {
    return {
      request_id: requestId,
      resource_id: "unknown",
      stage: "failed",
      requests_done: 0,
      items_kept: 0,
      items_cap: 0,
      elapsed_ms: 0,
      message_zh: t("shared.production.noActiveRequest"),
      terminal: true,
      error: null,
    };
  }
  return progress;
}

export async function cancelSearch(requestId: string): Promise<void> {
  if (!isTauriDesktopHost()) return;
  await defaultUnifiedClient.searchCancel(requestId);
}

/**
 * #22 B slice 3(c): cancel a catalogue search. A search that already showed
 * its first page keeps the rows that arrived (possibly incomplete).
 */
export async function cancelCatalogSearch(requestId: string): Promise<SearchCancelOutcome | null> {
  if (!isTauriDesktopHost()) return null;
  return defaultUnifiedClient.cancelCatalogSearch(requestId);
}

export async function getBatchPage(
  batchId: string,
  revision: number,
  page = 0,
  pageSize = RESULT_PAGE_SIZE
): Promise<BatchPageResult> {
  if (!isTauriDesktopHost()) {
    return {
      batch_id: batchId,
      revision,
      page,
      page_size: pageSize,
      total_count: 0,
      scenes: [],
      rows: [],
    };
  }

  const storePage = await defaultUnifiedClient.batchPage(batchId, revision, page, pageSize);
  // 权威 SnapshotItem 投影为展示行，未知字段绝不伪造；缺失顶层字段从权威 frozenProduct 补齐，保持列表行与信息弹窗一致。
  const scenes = (storePage.items || []).map((item) => {
    const scene = projectSnapshotItemToSceneRow(item);
    const frozenProduct = projectFrozenItemToScene(item)?.sar_product;
    if (!frozenProduct) return scene;
    return {
      ...scene,
      platform: scene.platform || frozenProduct.platform || "",
      instrument: scene.instrument ?? frozenProduct.instrument ?? null,
      processing_level: scene.processing_level ?? frozenProduct.processing_level ?? null,
      product_type: scene.product_type || frozenProduct.processing_level || frozenProduct.product_family || "",
      beam_mode: scene.beam_mode || frozenProduct.beam_mode || "",
      polarization: scene.polarization || frozenProduct.polarization || "",
      orbit_direction: scene.orbit_direction || frozenProduct.flight_direction || "",
      relative_orbit: scene.relative_orbit ?? frozenProduct.relative_orbit ?? null,
      path: scene.path ?? frozenProduct.path ?? null,
      frame: scene.frame ?? frozenProduct.frame ?? null,
      absolute_orbit: scene.absolute_orbit ?? frozenProduct.absolute_orbit ?? null,
      file_size_remote: scene.file_size_remote ?? frozenProduct.file_size_remote ?? null,
      footprint_bbox: scene.footprint_bbox ?? frozenProduct.footprint_bbox ?? null,
      dataset: scene.dataset || frozenProduct.dataset,
      sar_product: frozenProduct,
    };
  });

  return {
    batch_id: storePage.batch_id,
    revision: storePage.revision,
    page: storePage.page,
    page_size: storePage.page_size,
    total_count: storePage.total_count,
    scenes,
    rows: storePage.rows || [],
    state: storePage.state ?? "complete",
    loaded_count: storePage.loaded_count ?? storePage.total_count,
    remote_total: storePage.remote_total ?? null,
  };
}

export async function createTask(
  batchId: string,
  revision: number,
  itemIds: string[],
  options?: CreateTaskOptions
): Promise<TaskRef> {
  if (!isTauriDesktopHost()) {
    throw new Error(t("shared.production.requireDesktopHost"));
  }

  const outputRoot = options?.output_root?.trim();
  if (!outputRoot) {
    const err: any = new Error(t("shared.production.requireStorageLocation"));
    err.code = "OUTPUT_ROOT_REQUIRED";
    err.message_zh = t("shared.production.requireStorageLocation");
    throw err;
  }

  return defaultUnifiedClient.createTask(batchId, revision, itemIds, {
    output_root: outputRoot,
    concurrency: options?.concurrency || 2,
    layout: options?.layout || "direct",
    // Omitted unless the caller chose; the service then applies the
    // per-source default.
    ...(options?.include_precise_orbit === undefined
      ? {}
      : { include_precise_orbit: options.include_precise_orbit }),
  });
}

export async function previewAllMatchTask(
  request: BatchAllMatchPreviewRequest
): Promise<BatchAllMatchPreview> {
  if (!isTauriDesktopHost()) {
    throw new Error(t("shared.production.requireDesktopHost"));
  }
  return defaultUnifiedClient.previewAllMatchTask(request);
}

export async function createAllMatchTask(
  request: BatchAllMatchCreateTaskRequest
): Promise<BatchAllMatchTaskOutcome> {
  if (!isTauriDesktopHost()) {
    throw new Error(t("shared.production.requireDesktopHost"));
  }
  const outputRoot = request.output_root?.trim();
  if (!outputRoot) {
    const err: any = new Error(t("shared.production.requireStorageLocation"));
    err.code = "OUTPUT_ROOT_REQUIRED";
    err.message_zh = t("shared.production.requireStorageLocation");
    throw err;
  }
  return defaultUnifiedClient.createAllMatchTask({
    ...request,
    output_root: outputRoot,
    concurrency: request.concurrency || 2,
    layout: request.layout || "direct",
  });
}

export async function taskAction(
  taskId: string,
  revision: number,
  action: "pause" | "resume" | "retry" | "cancel" | "archive"
): Promise<void> {
  if (!isTauriDesktopHost()) return;
  await defaultUnifiedClient.taskAction(taskId, revision, action);
}

export const isAccountStatusesWired = (): boolean => isTauriDesktopHost();

const ACCOUNT_STATUS_ZH: Record<string, string> = {
  get logged_in() { return t("shared.production.authStates.logged_in"); },
  get login_required() { return t("shared.production.authStates.login_required"); },
  get credentials_missing() { return t("shared.production.authStates.credentials_missing"); },
  get service_unavailable() { return t("shared.production.authStates.service_unavailable"); },
  get not_required() { return t("shared.production.authStates.not_required"); },
};

export async function getAccountStatuses(): Promise<AccountStatus[]> {
  if (!isTauriDesktopHost()) {
    return [];
  }

  const rawStatuses: AccountSessionStatus[] = await defaultUnifiedClient.accountSessionStatuses();
  const nowStr = new Date().toLocaleTimeString("zh-CN", { hour12: false });

  return rawStatuses.map((st) => ({
    account_id: st.provider,
    state: st.state,
    checked_at: nowStr,
    message_zh: ACCOUNT_STATUS_ZH[st.state] || st.state,
  }));
}

/**
 * 将 StoreTaskSummary 投影为前端呈现的 ApplicationTask
 * 依据 crates/insar-store/src/core_restore.rs 的 task_type_for：
 * 后端 task_type 仅对 NISAR 数据集给值（nisar_subset / nisar_complete），其他如 DEM 转换任务均为 null。
 * 依据 contracts/v3/resource-registry.v1.json：真实 DEM 数据集的 resource_id 均以 "dem." 开头（如 dem.cop30.v1、dem.nasadem.v1 等）。
 * 因此依 dataset_id 前缀推导类目：dataset_id 以 "dem." 开头为 dem_convert，否则为 scene_download。
 */
export function mapStoreTaskSummaryToApplicationTask(summary: StoreTaskSummary): ApplicationTask {
  const isDem = typeof summary.dataset_id === "string" && summary.dataset_id.startsWith("dem.");
  const batchId = (summary.batch_id || "").trim();
  const isExpandedAssetBatch =
    batchId.startsWith("landsat-assets-batch-")
    || batchId.startsWith("sentinel2-bands-batch-")
    || batchId.startsWith("hls-bands-batch-")
    || batchId.startsWith("landsat-assets-")
    || batchId.startsWith("sentinel2-bands-");
  const admittedSceneCount =
    isExpandedAssetBatch && typeof summary.delivery_requested_count === "number"
      ? Math.max(1, summary.delivery_requested_count - (summary.delivery_rejected_count ?? 0))
      : isExpandedAssetBatch && (batchId.startsWith("landsat-assets-") || batchId.startsWith("sentinel2-bands-")) && !batchId.includes("-batch-")
        ? 1
        : null;
  const { title, subtitle } = humanTaskLabel({
    provider_id: summary.provider_id,
    dataset_id: summary.dataset_id,
    selected_count: summary.selected_count,
    scene_count: admittedSceneCount,
    task_id: summary.task_id,
    task_type: summary.task_type,
  });
  return {
    task_id: summary.task_id,
    revision: summary.revision,
    category: isDem ? "dem_convert" : "scene_download",
    name: title,
    provider: subtitle,
    source_provider_id: summary.provider_id,
    task_type: summary.task_type,
    dataset: summary.dataset_id ?? "—",
    status: summary.status,
    done_bytes: summary.transferred_bytes,
    total_bytes: summary.total_bytes,
    source_total_bytes: summary.source_total_bytes ?? null,
    speed_formatted: summary.bytes_per_second ? `${formatBytes(summary.bytes_per_second)}/s` : undefined,
    items_count: summary.selected_count,
    ...(isExpandedAssetBatch ? { items_unit: "files" as const } : {}),
    completed_items_count: summary.completed_count,
    failed_items_count: summary.failed_count,
    created_at: summary.created_at,
    updated_at: summary.updated_at,
    failure: summary.failure ?? null,
    pause_reason: summary.pause_reason ?? null,
    ...(typeof summary.output_root === "string" ? { output_dir: summary.output_root } : {}),
    source_task_id: summary.source_task_id ?? null,
    auxiliary_kind: summary.auxiliary_kind ?? null,
  };
}

export function mapTaskOverviewToApplicationTasks(overview: TaskOverview): ApplicationTask[] {
  const queueTasks = (overview.queue || []).map(mapStoreTaskSummaryToApplicationTask);
  // 归档历史摘要的字节字段是后端写死的 0 / null，不是「传了 0 字节」
  const historyTasks = (overview.history_page?.items || []).map(
    (item): ApplicationTask => ({ ...mapStoreTaskSummaryToApplicationTask(item), bytes_reported: false })
  );
  // 合并队列与历史第一页
  return [...queueTasks, ...historyTasks];
}

export async function getTasks(): Promise<ApplicationTask[]> {
  if (!isTauriDesktopHost()) {
    return [];
  }

  const overview = await defaultUnifiedClient.taskOverview();
  return mapTaskOverviewToApplicationTasks(overview);
}

export async function retryTaskFallback(
  taskId: string,
  confirmRefreshedBatchId: string | null = null
): Promise<TaskRetryFallbackOutcome> {
  return defaultUnifiedClient.taskRetryFallback(taskId, confirmRefreshedBatchId);
}

export async function clearTaskHistory(): Promise<void> {
  if (!isTauriDesktopHost()) return;
  const overview = await defaultUnifiedClient.taskOverview();
  const seen = new Set<string>();
  const targets = [
    ...(overview.history_page?.items || []),
    ...(overview.queue || []).filter((item) => isTaskHistoryRow(item)),
  ].filter((item) => {
    if (!item?.task_id || seen.has(item.task_id)) return false;
    seen.add(item.task_id);
    return true;
  });
  for (const item of targets) {
    try {
      await defaultUnifiedClient.taskDeleteHistory(item.task_id);
    } catch (_e) {
      // 容错继续
    }
  }
}

export async function openOutputDirectory(ref: {
  task_id: string;
  revision: number;
}): Promise<void> {
  if (!isTauriDesktopHost()) return;
  const bridge = getDesktopShellBridge();
  const result = await bridge.application.openTaskOutput({
    task_id: ref.task_id,
    expected_revision: ref.revision,
  });
  if (result && typeof result === "object" && "ok" in result && !(result as { ok: boolean }).ok) {
    throw result;
  }
}

export async function testNetworkConnection(): Promise<string> {
  if (!isTauriDesktopHost()) {
    return "transport_unavailable";
  }
  return defaultUnifiedClient.testNetworkConnection();
}

// 模型渠道与智能助手保持诚实未接入状态（W3c 后端未合入）
export const isAgentWired = (): boolean => false;

export async function getModelChannelReadiness() {
  return {
    ready: false,
    disabled: true,
    reason_zh: t("shared.production.agentNotConnectedReason"),
  };
}

export async function getAgentSessions(): Promise<AgentSession[]> {
  return [];
}

export function getAgentSessionsSync(): AgentSession[] {
  return [];
}

export async function getAgentSubsceneApproval(
  _subscene: string | null
): Promise<AgentApprovalRecord | null> {
  return null;
}

export function getAgentSubsceneApprovalSync(
  _subscene: string | null
): AgentApprovalRecord | null {
  return null;
}

export async function getAgentConclusions() {
  return {
    claims: [],
    knowledge: { taskIds: [], batchIds: [], verifiedWebDomains: [] },
  };
}

export function getAgentConclusionsSync() {
  return {
    claims: [],
    knowledge: { taskIds: [], batchIds: [], verifiedWebDomains: [] },
  };
}

export async function getFixtureNewSessionInitialMessage(): Promise<AgentMessage | null> {
  return null;
}

export function getFixtureNewSessionInitialMessageSync(): AgentMessage | null {
  return null;
}

export async function getFixtureAssistantGreeting() {
  return null;
}

export function getFixtureAssistantGreetingSync() {
  return null;
}

export function getAgentPlanResourceSummarySync(
  _sessionId?: string
): AgentPlanResourceSummary | null {
  return null;
}

export async function getAgentPlanResourceSummary(
  _sessionId?: string
): Promise<AgentPlanResourceSummary | null> {
  return null;
}

export function getFixtureWorkingAoi() {
  return null;
}

export function getFixtureStorageRoot(): string | null {
  return null;
}
