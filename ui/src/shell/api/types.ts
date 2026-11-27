/**
 * C1 数据源 / 检索批次 / 任务 统一接口契约 TypeScript 类型定义
 * 唯一来源为 ui/src/lib/generated/v3-contracts.ts
 */

import type {
  SourceDescriptor as GeneratedSourceDescriptor,
  SourceView as GeneratedSourceView,
  SearchProgress as GeneratedSearchProgress,
  SearchProgressStage,
  TypedError as GeneratedTypedError,
  TaskLifecycleStatus as GeneratedTaskLifecycleStatus,
  RowTaskState as GeneratedRowTaskState,
  RowTaskAction,
  SearchBatchProgressState,
  SearchCancelOutcome as GeneratedSearchCancelOutcome,
  TaskRef as GeneratedTaskRef,
  SourceFamily as GeneratedSourceFamily,
  SourceSearchMode as GeneratedSourceSearchMode,
  SourceNetworkDefault as GeneratedSourceNetworkDefault,
  SourceAvailability as GeneratedSourceAvailability,
  UnifiedSearchRequest as GeneratedUnifiedSearchRequest,
  SnapshotItem as GeneratedSnapshotItem,
  DownloadLayout as GeneratedDownloadLayout,
  AccountId as GeneratedAccountId,
  DownloadItemStatus as GeneratedDownloadItemStatus,
  TaskRetryFallbackOutcome as GeneratedTaskRetryFallbackOutcome,
  TaskFailureSummary as GeneratedTaskFailureSummary,
  TaskPauseReason as GeneratedTaskPauseReason,
  OrbitRecheckInfo as GeneratedOrbitRecheckInfo,
  DownloadFailure as GeneratedDownloadFailure,
} from "../../lib/generated/v3-contracts.ts";
import type { AccountSessionStatus as BridgeAccountSessionStatus } from "../../lib/accountSessions.ts";

export type Bbox = {
  west: number;
  south: number;
  east: number;
  north: number;
  crs?: string;
};

export interface DisplayBboxProjection {
  west: number;
  south: number;
  east: number;
  north: number;
}

export interface SceneRow {
  scene_id: string;
  product_key?: string;
  product_id?: string;
  provider?: string;
  dataset?: string;
  instrument?: string | null;
  processing_level?: string | null;
  product_family?: string;
  file_name?: string | null;
  native_metadata?: Record<string, unknown>;
  platform: string;
  product_type: string;
  beam_mode: string;
  polarization: string;
  acquisition_datetime: string;
  absolute_orbit: number | null;
  relative_orbit: number | null;
  path: number | null;
  frame: number | null;
  orbit_direction: string;
  file_size_remote?: number | null;
  footprint_bbox?: Bbox | null;
  footprint_geojson?: Record<string, unknown> | null;
  render_geometry?: Record<string, unknown> | null;
  render_bbox?: DisplayBboxProjection | null;
  render_authority?: { region_id: string; revision: number } | null;
  has_url: boolean;
  download_url?: string;
  task_item_ids?: string[];
  assets?: any[];
}

// 从生成类型直接再导出（禁止维护第二份同名手写定义）
export type SourceDescriptor = GeneratedSourceDescriptor;
export type SourceView = GeneratedSourceView;
export type SearchProgress = GeneratedSearchProgress;
export type SearchStage = SearchProgressStage;
export type SearchProgressStageType = SearchProgressStage;
export type TypedError = GeneratedTypedError;
export type TaskLifecycleStatus = GeneratedTaskLifecycleStatus;
export type RowTaskState = GeneratedRowTaskState;
export type RowAction = RowTaskAction;
export type RowTaskActionType = RowTaskAction;
export type TaskRef = GeneratedTaskRef;
export type SearchCancelOutcome = GeneratedSearchCancelOutcome;
export type { SearchBatchProgressState };
export type TaskRetryFallbackOutcome = GeneratedTaskRetryFallbackOutcome;
export type SourceFamily = GeneratedSourceFamily;
export type SearchMode = GeneratedSourceSearchMode;
export type SourceSearchMode = GeneratedSourceSearchMode;
export type NetworkDefault = GeneratedSourceNetworkDefault;
export type SourceNetworkDefault = GeneratedSourceNetworkDefault;
export type SourceAvailability = GeneratedSourceAvailability;
export type UnifiedSearchRequest = GeneratedUnifiedSearchRequest;
export type SnapshotItem = GeneratedSnapshotItem;
export type DownloadLayout = GeneratedDownloadLayout;
export type AccountId = GeneratedAccountId;
export type DownloadItemStatus = GeneratedDownloadItemStatus;
export type TaskFailureSummary = GeneratedTaskFailureSummary;

// 账号状态类型：复用 ui/src/lib/accountSessions.ts
export type AccountSessionStatus = BridgeAccountSessionStatus;
export type AccountState = AccountSessionStatus["state"];

export interface AccountStatus {
  account_id: AccountId;
  state: AccountState;
  checked_at?: string | null;
  message_zh: string;
}

// 网络与代理配置
export type GlobalProxyMode = "direct" | "system" | "manual";
export type SourceProxyOverride = "follow_global" | "system" | "direct" | "manual";

export interface NetworkProxySettings {
  global_mode: GlobalProxyMode;
  manual_host?: string;
  manual_port?: number | null;
  source_overrides: Record<string, SourceProxyOverride>;
}

export interface EndpointConnectionResult {
  id: string;
  name: string;
  endpoint: string;
  ok: boolean;
  latency_ms: number;
  message_zh: string;
}

// 模型渠道配置
export type ModelProtocol =
  | "openai_chat_completions"
  | "openai_responses"
  | "anthropic_messages";

export interface ModelChannelConfig {
  id: string;
  name: string;
  protocol: ModelProtocol;
  base_url: string;
  api_key_configured: boolean;
  models: string[];
}

export type TaskPauseReason = GeneratedTaskPauseReason;
export type OrbitRecheckInfo = GeneratedOrbitRecheckInfo;
export type DownloadFailure = GeneratedDownloadFailure;

// 后端未生成、但经由 crates/insar-store/src/core_restore.rs 83-128 行与 436-448 行定义的投影
export interface StoreTaskSummary {
  task_id: string;
  batch_id: string;
  provider_id: string;
  dataset_id: string | null;
  concurrency: number | null;
  task_type: string | null;
  created_at: string;
  status: TaskLifecycleStatus;
  revision: number;
  updated_at: string;
  selected_count: number;
  delivery_requested_count: number | null;
  delivery_rejected_count: number | null;
  checked_count: number;
  active_count: number;
  paused_count: number;
  queued_count: number;
  completed_count: number;
  failed_count: number;
  skipped_count: number;
  cancelled_count: number;
  terminal_bytes: number;
  active_bytes: number;
  other_bytes: number;
  transferred_bytes: number;
  total_bytes: number | null;
  source_total_bytes?: number | null;
  bytes_per_second?: number;
  transfer_active_millis?: number;
  failure?: TaskFailureSummary | null;
  pause_reason?: TaskPauseReason | null;
  /** 保存目录（契约 TaskOverviewTask / TaskOverviewHistoryItem.output_root）；本机路径，只显示与打开，不外发 */
  output_root?: string;
  /**
   * 轨道定时获取 S4：该任务派生自哪个影像任务（配套精轨任务），
   * 由后端从冻结 operation payload 投影；普通任务与旧数据为 null。
   */
  source_task_id?: string | null;
  /**
   * 轨道定时获取 S4：配套任务行类型（如 AUX_POEORB）；
   * 普通任务与旧数据为 null。
   */
  auxiliary_kind?: string | null;
}

export interface StoreTaskHistoryCursor {
  archived_at_epoch_ms: number;
  task_id: string;
}

export interface StoreTaskHistoryItem extends StoreTaskSummary {
  archived_at: string;
}

export interface StoreTaskHistoryPage {
  page_size: number;
  has_more: boolean;
  next_cursor: StoreTaskHistoryCursor | null;
  items: StoreTaskHistoryItem[];
}

export interface TaskOverview {
  queue: StoreTaskSummary[];
  history_page: StoreTaskHistoryPage;
}

export interface StoreBatchPage {
  batch_id: string;
  revision: number;
  page: number;
  page_size: number;
  total_count: number;
  items: SnapshotItem[];
  rows: RowTaskState[];
  /** #22 B slice 3(c), contract 3.41.0: delivery state; a loading batch keeps growing. */
  state: SearchBatchProgressState;
  /** #22 B slice 3(c): rows that have arrived (equals total_count). */
  loaded_count: number;
  /** #22 B slice 3(c): provider-reported total when the provider offers one. */
  remote_total?: number | null;
}

export interface BatchPageResult {
  batch_id: string;
  revision: number;
  page: number;
  page_size: number;
  total_count: number;
  scenes: SceneRow[];
  rows: RowTaskState[];
  /** #22 B slice 3(c): delivery state of the batch (absent from older fixtures: complete). */
  state?: SearchBatchProgressState;
  /** #22 B slice 3(c): rows that have arrived. */
  loaded_count?: number;
  /** #22 B slice 3(c): provider total when known. */
  remote_total?: number | null;
}

export interface SearchStartParams {
  resource_id: string;
  aoi?: Record<string, unknown> | Bbox | null;
  aoi_name: string; // 必填，后端 validate 拒绝空串
  start_date?: string;
  end_date?: string;
  filters?: Record<string, unknown>;
}

export interface CreateTaskOptions {
  output_root?: string;
  layout?: DownloadLayout;
  concurrency?: number;
  /**
   * 轨道定时获取 S4：同时创建配套精密轨道（POEORB）任务。
   * 省略时按数据源策略：SLC / Burst 默认创建，GRD 默认不创建。
   */
  include_precise_orbit?: boolean;
}

export type TaskCategory = "scene_download" | "dem_convert";

/** #22 B Slice 4: versioned predicate over a frozen search batch. */
export interface BatchSelectionPredicate {
  version: number;
  orbit_direction?: "ascending" | "descending" | null;
  relative_orbit?: number | null;
  min_acquisition_datetime?: string | null;
  max_acquisition_datetime?: string | null;
  max_cloud_cover?: number | null;
}

export interface BatchAllMatchPreviewRequest {
  batch_id: string;
  expected_revision: number;
  predicate?: BatchSelectionPredicate;
  excluded_item_ids?: string[];
}

export interface BatchAllMatchPreview {
  batch_id: string;
  revision: number;
  batch_total_count: number;
  matched_count: number;
  excluded_count: number;
  skipped_active_count: number;
  rejected_count: number;
  admissible_count: number;
  known_bytes: number;
  unknown_size_count: number;
  selection_digest: string;
}

export interface BatchAllMatchCreateTaskRequest {
  batch_id: string;
  expected_revision: number;
  predicate?: BatchSelectionPredicate;
  excluded_item_ids?: string[];
  expected_selection_digest: string;
  expected_admissible_count: number;
  output_root: string;
  layout?: DownloadLayout;
  concurrency?: number;
  include_precise_orbit?: boolean;
}

export interface BatchAllMatchTaskOutcome {
  task_id: string;
  revision: number;
  status: TaskLifecycleStatus;
  created: boolean;
  warning?: string | null;
  matched_count: number;
  created_count: number;
  pending_count: number;
  skipped_count: number;
  rejected_count: number;
}

/**
 * 分景任务进度详情（可选，后端未下发时前端退回至 scene_ids 列表）
 */
export interface TaskSceneItem {
  item_id?: string;
  scene_id: string;
  status: TaskLifecycleStatus;
  done_bytes: number;
  total_bytes?: number | null;
  destination?: string | null;
  resolved_orbit_type?: "POEORB" | "RESORB" | null;
  orbit_recheck?: OrbitRecheckInfo | null;
  failure?: DownloadFailure | null;
}

export interface ApplicationTask {
  task_id: string;
  revision: number;
  category: TaskCategory;
  name: string;
  provider?: string;
  source_provider_id?: string;
  task_type?: string | null;
  dataset?: string;
  status: TaskLifecycleStatus;
  done_bytes: number;
  total_bytes?: number | null;
  source_total_bytes?: number | null;
  /**
   * false：字节字段不是后端上报值。归档历史摘要的字节字段由后端写死为 0 / null
   * （core_restore.rs history_summary_from_sql），只在 history_page 条目上设置；队列条目不设。
   */
  bytes_reported?: false;
  bytes_per_second?: number;
  speed_formatted?: string;
  output_dir?: string;
  error?: TypedError | string | null;
  created_at?: string;
  updated_at?: string;
  frozen_search_params?: SearchStartParams | null;
  items_count?: number;
  items_unit?: "scenes" | "files";
  completed_items_count?: number;
  failed_items_count?: number;
  scene_ids?: string[];
  /**
   * 分景级任务进度详情（可选），后端未下发时前端退回至 scene_ids。
   */
  scene_items?: TaskSceneItem[] | null;
  failure?: TaskFailureSummary | null;
  pause_reason?: TaskPauseReason | null;
  source_task_id?: string | null;
  auxiliary_kind?: string | null;
}

export type TaskItemAction =
  | "pause"
  | "resume"
  | "retry"
  | "retry_fallback"
  | "details"
  | "open_folder"
  | "cancel";

export type UnifiedTaskItemAction = "pause" | "resume" | "retry" | "cancel";

export interface ExistingDownloadCheckItem {
  scene_id: string;
  file_name: string;
  target_relpath?: string | null;
}

export interface ExistingDownloadMatch {
  scene_id: string;
  path: string;
  size_bytes: number;
  task_title?: string | null;
}

export * from "./agentTypes.ts";
