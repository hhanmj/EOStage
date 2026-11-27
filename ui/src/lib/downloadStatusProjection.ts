import type {
  CoreTaskActiveProgressProjection,
  CoreTaskFailureCodeProjection,
  CoreTaskLocalScanProjection,
  CoreTaskSummaryProjection,
} from "./coreRestoreProjection.ts";

export type CoreTaskDownloadStatusProjection = {
  ok: true;
  task_id: string;
  batch_id: string;
  provider_id: string;
  approved_worker_operation: string | null;
  dataset_id: string | null;
  task_type: string;
  state: string;
  revision: number;
  concurrency: number | null;
  created_at: string;
  ended_at: string | null;
  total: number;
  admission_requested_count: number | null;
  admission_rejected_count: number | null;
  done: number;
  current_scene: string;
  active_downloads: Array<{ scene_id: string; bytes: number; expected_size: number | null }>;
  active_progress: CoreTaskActiveProgressProjection | null;
  failure_codes: CoreTaskFailureCodeProjection[] | null;
  local_scan: CoreTaskLocalScanProjection | null;
  total_bytes: number | null;
  source_total_bytes: number | null;
  done_bytes: number;
  current_bytes: number;
  transferred_bytes: number;
  network_bytes_received: number | null;
  reusable_verified_bytes: number | null;
  reused_verified_bytes: number | null;
  current_expected_size: number | null;
  bytes_per_second: number;
  transfer_active_millis: number;
  elapsed_seconds: number;
  paused: boolean;
  cancelled: boolean;
  error: null;
  summary_line: string;
  results_path: string;
  output_dir: string;
  download_layout: string;
  succeeded: number;
  skipped: number;
  failed: number;
  interrupted: number;
  has_failures: boolean;
  paused_scene_ids: string[];
  selected_scene_ids: string[];
  checked_scene_ids: string[];
  queued_scene_ids: string[];
  snapshot_scenes: never[];
  task_paused: boolean;
  resume_supported: boolean;
  retry_supported: boolean;
  log: Array<{ scene_id: string; outcome: string; bytes_written: number; detail: string; message?: string; ts?: number }>;
};

export function projectDownloadStatusFromCoreTask(
  task: CoreTaskSummaryProjection,
): CoreTaskDownloadStatusProjection {
  const done = task.completed_count + task.skipped_count;
  const failures = task.failed_count + task.cancelled_count;
  const active = task.active_item;
  const isManualGacosRequest = task.operation?.kind === "gacos"
    && task.operation.approved_worker_operation === "gacos_manual_request";
  // Tasks stored before the Store recorded item-only pauses as paused still say queued.
  const onlyPausedItemsRemain = task.status === "queued"
    && task.active_count === 0
    && task.queued_count === 0
    && task.paused_count > 0;
  const state = onlyPausedItemsRemain ? "paused" : task.status;
  return {
    ok: true,
    task_id: task.task_id,
    batch_id: task.batch_id,
    provider_id: task.provider_id,
    approved_worker_operation: task.operation?.approved_worker_operation ?? null,
    dataset_id: task.dataset_id,
    task_type: task.operation?.kind === "orbit" ? "orbit" : task.task_type ?? "",
    state,
    revision: task.revision,
    concurrency: task.concurrency ?? null,
    created_at: task.created_at,
    ended_at: ["completed", "failed", "cancelled", "interrupted"].includes(task.status)
      ? task.updated_at
      : null,
    total: task.selected_count,
    admission_requested_count: task.delivery_requested_count ?? null,
    admission_rejected_count: task.delivery_rejected_count ?? null,
    done,
    current_scene: active?.item_id ?? "",
    active_downloads: active
      ? [{ scene_id: active.item_id, bytes: active.bytes_completed, expected_size: active.total_bytes }]
      : [],
    active_progress: task.active_progress ?? null,
    failure_codes: task.failure_codes ?? null,
    local_scan: task.local_scan ?? null,
    total_bytes: task.total_bytes,
    source_total_bytes: task.source_total_bytes ?? null,
    done_bytes: task.terminal_bytes,
    current_bytes: task.active_bytes + task.other_bytes,
    transferred_bytes: task.transferred_bytes,
    network_bytes_received: task.network_bytes_received ?? null,
    reusable_verified_bytes: task.reusable_verified_bytes ?? null,
    reused_verified_bytes: task.reused_verified_bytes ?? null,
    current_expected_size: active?.total_bytes ?? null,
    bytes_per_second: task.bytes_per_second,
    transfer_active_millis: task.transfer_active_millis,
    elapsed_seconds: task.transfer_active_millis / 1_000,
    paused: state === "paused",
    cancelled: task.status === "cancelled",
    error: null,
    summary_line: `${done}/${task.selected_count}`,
    results_path: "",
    output_dir: task.output_root,
    download_layout: task.layout,
    succeeded: task.completed_count,
    skipped: task.skipped_count,
    failed: task.failed_count,
    interrupted: task.cancelled_count,
    has_failures: failures > 0,
    paused_scene_ids: [],
    selected_scene_ids: [],
    checked_scene_ids: [],
    queued_scene_ids: [],
    snapshot_scenes: [],
    task_paused: state === "paused",
    // This field describes transfer/recovery capability, not whether the
    // current lifecycle state happens to expose the Resume button.  Keeping
    // those concepts separate lets running tasks state their resume support
    // before a pause is requested.
    resume_supported: !isManualGacosRequest,
    retry_supported: !isManualGacosRequest && failures > 0,
    log: [],
  };
}
