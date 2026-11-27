use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::{self, ErrorKind};
use std::path::{Component, Path, PathBuf};
use std::thread;
use std::time::Duration;

use insar_core::storage_guard::{disk_low_resume_error, DiskLowPolicy};
use insar_core::TaskPauseReason;
use insar_core::{
    DownloadLayout, FrozenSearchBatch, SearchBatchProgressState, SnapshotItem,
    StoreArchiveTaskRequest, StoreCreateTaskRequest, StoreDeleteHistoryRequest, StoreTaskCommand,
    StoreUpdateTaskRequest, TaskFailureSummary, TaskLifecycleStatus, TaskRef, TaskStateCommandKind,
    TypedError,
};
use insar_download::disk_guard::{
    default_free_space_probe, disk_low_resume_reason, FreeSpaceProbe,
};
use insar_store::{
    CompletedDownloadDestination, StoreBatchSelection, StoreTaskHistoryPage,
    StoreTaskHistoryPageRequest, StoreTaskSummary,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{hls_catalogue, ApplicationService, ApplicationServiceError};

pub const ALL_MATCH_PREDICATE_VERSION_V1: u32 = 1;
pub const MAX_ALL_MATCH_CANDIDATES: usize = 10_000;
pub const MAX_ALL_MATCH_EXCLUSIONS: usize = 250;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchSelectionPredicate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orbit_direction: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beam_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub polarization: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relative_orbit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_query: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchAllMatchPreviewRequest {
    pub batch_id: String,
    pub revision: u64,
    pub predicate_version: u32,
    #[serde(default)]
    pub predicate: BatchSelectionPredicate,
    #[serde(default)]
    pub exclusions: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchAllMatchPreview {
    pub batch_id: String,
    pub revision: u64,
    pub predicate_version: u32,
    pub matched_count: usize,
    pub admissible_count: usize,
    pub skipped_active_count: usize,
    pub rejected_count: usize,
    pub known_bytes: u64,
    pub unknown_size_count: usize,
    pub membership_digest: String,
    pub orbit_directions: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchAllMatchCreateTaskRequest {
    pub batch_id: String,
    pub revision: u64,
    pub predicate_version: u32,
    #[serde(default)]
    pub predicate: BatchSelectionPredicate,
    #[serde(default)]
    pub exclusions: Vec<String>,
    pub expected_membership_digest: String,
    pub options: UnifiedCreateTaskOptions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchAllMatchTaskOutcome {
    pub task: TaskRef,
    pub batch_id: String,
    pub revision: u64,
    pub membership_digest: String,
    pub matched_count: usize,
    pub created_count: usize,
    pub pending_count: usize,
    pub skipped_count: usize,
    pub rejected_count: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnifiedCreateTaskOptions {
    pub output_root: PathBuf,
    pub layout: DownloadLayout,
    pub concurrency: u64,
    /// Orbit scheduled-fetch S4: create the companion precise-orbit task for
    /// the created image task. `None` follows the per-source policy: SLC and
    /// Burst default to on, GRD defaults to off.
    #[serde(default)]
    pub include_precise_orbit: Option<bool>,
}

/// The seam W1 fills: `create_task` installs
/// `account_sessions::AccountSessionPreflight`, which establishes or refreshes
/// every required account session before a task row exists.
pub trait TaskAdmissionPreflight: Send + Sync {
    /// `batch` is the frozen batch header (#22 B slice 2: its candidate array
    /// is empty); `selected_items` are the frozen products the request names,
    /// in the batch's frozen order. Admission reads what it needs from the
    /// selection, never from the whole candidate array.
    fn check(
        &self,
        batch: &FrozenSearchBatch,
        selected_items: &[SnapshotItem],
        item_ids: &[String],
    ) -> Result<(), TypedError>;

    /// A caveat from the last successful `check`, returned as
    /// `TaskRef.warning` (e.g. a session the worker establishes itself).
    fn admission_warning(&self) -> Option<String> {
        None
    }
}

pub(crate) fn task_error(error: ApplicationServiceError) -> TypedError {
    if let ApplicationServiceError::DiskLow { reason } = &error {
        return disk_low_resume_error(reason);
    }
    let code = error.code();
    let (message_zh, retryable) = if code.contains("CREDENTIAL")
        || code.contains("UNAUTHORIZED")
        || code.contains("AUTH")
        || code.contains("SESSION")
    {
        ("建任务失败，请检查账号登录状态", false)
    } else if code.contains("NETWORK") || code.contains("TRANSPORT") || code.contains("TIMEOUT") {
        ("网络暂不可用，请稍后重试建任务", true)
    } else if code.contains("REVISION_CONFLICT") {
        ("任务修订冲突，请刷新任务状态后重试", false)
    } else if code.contains("ROWS_NOT_YET_LOADED") {
        ("所选的行尚未载入，请稍后再试", true)
    } else if code == "V3_STORE_BATCH_STILL_LOADING" {
        (
            "检索批次仍在载入中，请等待载入完成后再执行全选全部匹配",
            true,
        )
    } else if code == "V3_STORE_PARTIAL_ACTIVE_TASK_OVERLAP" {
        (
            "部分所选场景或资产已有进行中的任务，请刷新后重新选择",
            false,
        )
    } else if code.contains("NOT_FOUND") {
        ("所选任务或冻结批次不存在，请刷新后重试", false)
    } else if code.contains("ALREADY_EXISTS") {
        ("任务已存在，请刷新任务列表", false)
    } else {
        ("建任务失败，请检查所选数据及输出路径", false)
    };
    TypedError {
        code: code.to_owned(),
        message_zh: message_zh.to_owned(),
        retryable,
        needs_account: None,
        needs_user_confirmation: false,
    }
}

fn invalid(message: &str) -> TypedError {
    TypedError {
        code: "V3_APPLICATION_INVALID_REQUEST".to_owned(),
        message_zh: message.to_owned(),
        retryable: false,
        needs_account: None,
        needs_user_confirmation: false,
    }
}

/// Orbit scheduled-fetch S4: whether one frozen product can take a
/// companion precise orbit — the same SLC evidence
/// `create_companion_orbit_task` freezes (ASF Sentinel-1, SLC product
/// identifiers), plus the unified-search burst selection it expands from.
fn companion_orbit_supported_item(item: &SnapshotItem) -> bool {
    let asf_sentinel1 = matches!(item.provider_id.as_str(), "asf.sentinel1" | "asf.catalog");
    let burst = item
        .metadata
        .get("dataset")
        .and_then(|value| value.as_str())
        .is_some_and(|dataset| dataset.eq_ignore_ascii_case("SLC-BURST"));
    let slc = [
        item.payload
            .get("scene_id")
            .and_then(|value| value.as_str()),
        item.metadata
            .get("scene_id")
            .and_then(|value| value.as_str()),
        Some(item.display_name.as_str()),
        Some(item.item_id.as_str()),
    ]
    .into_iter()
    .flatten()
    .find_map(|name| insar_core::parse_sentinel1_identifier(name).ok())
    .is_some_and(|scene| scene.product_type == "SLC");
    asf_sentinel1 && (burst || slc)
}

/// Orbit scheduled-fetch S4: how many selected products can take a companion
/// precise orbit. A partial count means the companion covers the supported
/// scenes and the rest ride a typed warning (Owner 11-02 rule).
fn companion_orbit_supported_count(selected_products: &[SnapshotItem]) -> usize {
    selected_products
        .iter()
        .filter(|item| companion_orbit_supported_item(item))
        .count()
}

/// Orbit scheduled-fetch S4: the per-source default when the caller leaves
/// `include_precise_orbit` unset. SLC and Burst selections default to on;
/// anything else (GRD, non-Sentinel-1 sources) defaults to off. This is the
/// same SLC-evidence test the companion path itself freezes, so the default
/// never names a selection the companion would then reject.
fn companion_orbit_default_for_selection(
    selected_products: &[SnapshotItem],
    contains_burst: bool,
) -> bool {
    contains_burst || companion_orbit_supported_count(selected_products) > 0
}

/// Orbit scheduled-fetch S4: combine the account preflight warning and the
/// orbit warning on one TaskRef (preflight first, joined with 「；」), so
/// neither silently drops the other.
fn join_warnings(preflight: Option<String>, orbit: Option<String>) -> Option<String> {
    match (preflight, orbit) {
        (Some(preflight), Some(orbit)) => Some(format!("{preflight}；{orbit}")),
        (warning, None) | (None, warning) => warning,
    }
}

const STORAGE_ROOT_NOT_WRITABLE: &str = "STORAGE_ROOT_NOT_WRITABLE";

fn storage_root_not_writable(root: &Path) -> TypedError {
    TypedError {
        code: STORAGE_ROOT_NOT_WRITABLE.to_owned(),
        message_zh: format!(
            "存储根路径不可写：{}（请检查目录权限或更换路径）",
            root.display()
        ),
        retryable: false,
        needs_account: None,
        needs_user_confirmation: false,
    }
}

/// How a probe is removed; a test substitutes a failing removal.
#[derive(Clone, Copy)]
pub(crate) struct ProbeCleanup {
    pub(crate) remove_file: fn(&Path) -> io::Result<()>,
    pub(crate) remove_dir: fn(&Path) -> io::Result<()>,
}

fn remove_file_probe(path: &Path) -> io::Result<()> {
    fs::remove_file(path)
}

fn remove_dir_probe(path: &Path) -> io::Result<()> {
    fs::remove_dir(path)
}

const PROBE_CLEANUP: ProbeCleanup = ProbeCleanup {
    remove_file: remove_file_probe,
    remove_dir: remove_dir_probe,
};
const PROBE_CLEANUP_ATTEMPTS: u32 = 3;
const PROBE_CLEANUP_RETRY_DELAY: Duration = Duration::from_millis(50);

/// A probe that is already gone (Windows delete-on-close) counts as removed.
fn remove_probe(probe: &Path, remove: fn(&Path) -> io::Result<()>) -> bool {
    for attempt in 1..=PROBE_CLEANUP_ATTEMPTS {
        match remove(probe) {
            Ok(()) => return true,
            Err(error) if error.kind() == ErrorKind::NotFound => return true,
            Err(_) if attempt < PROBE_CLEANUP_ATTEMPTS => thread::sleep(PROBE_CLEANUP_RETRY_DELAY),
            Err(_) => {}
        }
    }
    false
}

/// Admission pre-flight for the task's storage root, so a root the user
/// cannot write is refused before a task row exists instead of failing every
/// item later (DL005). An existing directory must accept a new file; a root
/// that does not exist yet must be creatable, so its nearest existing
/// ancestor must accept a new directory (the engine creates the root). Each
/// probe has a unique name and must be removed again: a root where we cannot
/// delete our own probe would also fail the engine's `.part` rename, so a
/// removal that still fails after bounded retries refuses the root too.
pub(crate) fn probe_storage_root(root: &Path) -> Result<(), TypedError> {
    probe_storage_root_with(root, PROBE_CLEANUP)
}

pub(crate) fn probe_storage_root_with(
    root: &Path,
    cleanup: ProbeCleanup,
) -> Result<(), TypedError> {
    let probe_name = format!(".eostage-write-probe-{}", Uuid::new_v4());
    let refused = || storage_root_not_writable(root);
    match fs::metadata(root) {
        Ok(metadata) if metadata.is_dir() => {
            let probe = root.join(&probe_name);
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(windows)]
            {
                // Delete on close as well, so a crash mid-probe leaves nothing.
                use std::os::windows::fs::OpenOptionsExt;
                const DELETE: u32 = 0x0001_0000;
                const GENERIC_WRITE: u32 = 0x4000_0000;
                const FILE_FLAG_DELETE_ON_CLOSE: u32 = 0x0400_0000;
                options
                    .access_mode(GENERIC_WRITE | DELETE)
                    .custom_flags(FILE_FLAG_DELETE_ON_CLOSE);
            }
            let file = options.open(&probe).map_err(|_| refused())?;
            drop(file);
            if remove_probe(&probe, cleanup.remove_file) {
                Ok(())
            } else {
                Err(refused())
            }
        }
        Ok(_) => Err(refused()),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            let mut ancestor = root.parent();
            while let Some(parent) = ancestor {
                let parent = if parent.as_os_str().is_empty() {
                    Path::new(".")
                } else {
                    parent
                };
                match fs::metadata(parent) {
                    Ok(metadata) if metadata.is_dir() => {
                        let probe = parent.join(&probe_name);
                        fs::create_dir(&probe).map_err(|_| refused())?;
                        return if remove_probe(&probe, cleanup.remove_dir) {
                            Ok(())
                        } else {
                            Err(refused())
                        };
                    }
                    Err(error)
                        if error.kind() == ErrorKind::NotFound && parent != Path::new(".") =>
                    {
                        ancestor = parent.parent();
                    }
                    _ => return Err(refused()),
                }
            }
            Err(refused())
        }
        Err(_) => Err(refused()),
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetryFallbackComparison {
    pub task_id: String,
    pub source_batch_id: String,
    pub refreshed_batch_id: String,
    pub missing_product_ids: Vec<String>,
    pub added_product_ids: Vec<String>,
    pub needs_user_confirmation: bool,
}

/// Products of the refreshed batch that the original batch did not return
/// (refreshed batch minus original batch). Reported only, never added.
pub(crate) fn added_products(original: &BTreeSet<&str>, refreshed: &BTreeSet<&str>) -> Vec<String> {
    refreshed
        .difference(original)
        .map(|id| (*id).to_owned())
        .collect()
}

/// A comparison is read-only; changed membership is never widened silently.
/// Only products missing from the refreshed batch require confirmation; an
/// unchanged set is rebuilt through schema-33 snapshot revisions.
pub fn compare_retry_fallback(
    task: &insar_core::FrozenTaskSnapshot,
    refreshed: &FrozenSearchBatch,
) -> Result<RetryFallbackComparison, TypedError> {
    if task.provider_id != refreshed.provider_id
        || task.aoi_geojson != refreshed.aoi_geojson
        || task.query != refreshed.query
    {
        return Err(invalid("重检来源、真实范围或冻结条件与原任务不一致"));
    }
    let selected = task
        .initial_selected_item_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if selected.is_empty() || selected.len() != task.initial_selected_item_ids.len() {
        return Err(invalid("原任务的产品集合无效"));
    }
    let new_ids = refreshed
        .candidates
        .iter()
        .map(|item| item.item_id.as_str())
        .collect::<BTreeSet<_>>();
    if new_ids.len() != refreshed.candidates.len() {
        return Err(invalid("重检产品编号重复"));
    }
    let missing_product_ids = selected
        .difference(&new_ids)
        .map(|id| (*id).to_owned())
        .collect::<Vec<_>>();
    let original = task
        .candidates
        .iter()
        .map(|item| item.item_id.as_str())
        .collect::<BTreeSet<_>>();
    let added_product_ids = added_products(&original, &new_ids);
    Ok(RetryFallbackComparison {
        task_id: task.task_id.clone(),
        source_batch_id: task.batch_id.clone(),
        refreshed_batch_id: refreshed.batch_id.clone(),
        needs_user_confirmation: !missing_product_ids.is_empty(),
        missing_product_ids,
        added_product_ids,
    })
}

/// C1 task overview (contracts/v3 `TaskOverview`), generated for Rust,
/// TypeScript and Python. The Store's summaries are projected into it by
/// their identical serde form, so a Store field the contract lacks fails the
/// read instead of silently disappearing.
pub use insar_core::TaskOverview;

#[derive(Serialize)]
struct StoreTaskOverview<'a> {
    queue: &'a [StoreTaskSummary],
    history_page: &'a StoreTaskHistoryPage,
}

fn task_failure(summary: insar_store::StoreTaskFailureSummary) -> TaskFailureSummary {
    TaskFailureSummary {
        codes: summary.codes,
        message_zh: summary.message_zh,
        failed_items: summary.failed_items,
        total_items: summary.total_items,
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnifiedTaskAction {
    Pause,
    Resume,
    Retry,
    Cancel,
    Archive,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnifiedTaskItemAction {
    Pause,
    Resume,
    Retry,
    Cancel,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExistingDownloadCheckItem {
    pub scene_id: String,
    pub file_name: String,
    #[serde(default)]
    pub target_relpath: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExistingDownloadMatch {
    pub scene_id: String,
    pub path: String,
    pub size_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_title: Option<String>,
}

/// The frozen legacy terminal archiver's set: completed/finished, failed,
/// cancelled, interrupted and timeout (finished and timeout have no Store
/// lifecycle state; a timed-out item fails its task). A failed or interrupted
/// NISAR subset keeps its resumable checkpoint in the queue; a cancelled one is
/// archived, exactly as in `isNisarSubsetAwaitingRetry`.
fn auto_archive_candidate(status: &TaskLifecycleStatus, task_type: Option<&str>) -> bool {
    match status {
        TaskLifecycleStatus::Completed | TaskLifecycleStatus::Cancelled => true,
        TaskLifecycleStatus::Failed | TaskLifecycleStatus::Interrupted => {
            task_type != Some("nisar_subset")
        }
        TaskLifecycleStatus::Queued
        | TaskLifecycleStatus::Running
        | TaskLifecycleStatus::Paused => false,
    }
}

fn archive_failed_candidates(
    failures: impl IntoIterator<Item = (String, u64)>,
    mut archive: impl FnMut(&StoreArchiveTaskRequest) -> Result<(), insar_store::StoreError>,
) -> Result<(), insar_store::StoreError> {
    for (task_id, revision) in failures {
        let request = StoreArchiveTaskRequest {
            task_id,
            expected_revision: revision,
            archived_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        };
        match archive(&request) {
            Ok(_)
            | Err(insar_store::StoreError::RevisionConflict)
            | Err(insar_store::StoreError::GacosSubmissionInFlight)
            | Err(insar_store::StoreError::AlreadyExists) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn is_single_normal_filename(name: &str) -> bool {
    let mut components = Path::new(name).components();
    matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
}

fn validated_relpath(raw: &str) -> Result<PathBuf, ApplicationServiceError> {
    let mut rel = PathBuf::new();
    for component in Path::new(raw).components() {
        match component {
            Component::Normal(part) => rel.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(ApplicationServiceError::InvalidRequest {
                    reason: format!("invalid check target_relpath: {raw}"),
                });
            }
        }
    }
    Ok(rel)
}

impl ApplicationService {
    /// Only the new task overview performs automatic archival, over the legacy
    /// terminal archiver's set. The legacy core_restore(false) and history-page
    /// reads retain their original semantics.
    pub fn task_overview(&self) -> Result<TaskOverview, ApplicationServiceError> {
        self.startup()?;
        let store = self.store_handle()?;
        let mut store = store
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?;
        let failures = store
            .core_restore(false)?
            .tasks
            .into_iter()
            .filter(|task| auto_archive_candidate(&task.status, task.task_type.as_deref()))
            .map(|task| (task.task_id, task.revision))
            .collect::<Vec<_>>();
        archive_failed_candidates(failures, |request| store.archive_task(request).map(|_| ()))?;
        let queue = store.core_restore(false)?.tasks;
        let history_page = store.load_task_history_page(&StoreTaskHistoryPageRequest {
            page_size: insar_store::DEFAULT_TASK_HISTORY_PAGE_SIZE,
            after_archived_at_epoch_ms: None,
            after_task_id: None,
        })?;
        let tasks = queue
            .iter()
            .chain(history_page.items.iter().map(|item| &item.task));
        let (task_ids, nisar_subset_ids) =
            tasks.fold((Vec::new(), Vec::new()), |(mut all, mut nisar), task| {
                all.push(task.task_id.clone());
                if task.task_type.as_deref() == Some("nisar_subset") {
                    nisar.push(task.task_id.clone());
                }
                (all, nisar)
            });
        let mut failures = store.load_task_failure_summaries(&task_ids, &nisar_subset_ids)?;
        let queue_ids = queue
            .iter()
            .map(|task| task.task_id.clone())
            .collect::<Vec<_>>();
        let mut pause_reasons = store.load_task_pause_reasons(&queue_ids)?;
        drop(store);
        let mut projected = serde_json::to_value(StoreTaskOverview {
            queue: &queue,
            history_page: &history_page,
        })
        .and_then(serde_json::from_value::<TaskOverview>)
        .map_err(|error| ApplicationServiceError::InvalidRequest {
            reason: format!("task overview does not match the contract: {error}"),
        })?;
        for task in &mut projected.queue {
            task.failure = failures.remove(&task.task_id).map(task_failure);
            task.pause_reason = pause_reasons.remove(&task.task_id);
        }
        for item in &mut projected.history_page.items {
            item.failure = failures.remove(&item.task_id).map(task_failure);
        }
        projected
            .validate()
            .map_err(|reason| ApplicationServiceError::InvalidRequest { reason })?;
        Ok(projected)
    }

    pub fn task_action(
        &self,
        task_id: String,
        revision: u64,
        action: UnifiedTaskAction,
    ) -> Result<TaskRef, TypedError> {
        let probe = default_free_space_probe();
        self.task_action_with_free_space_probe(task_id, revision, action, probe.as_ref())
    }

    /// `task_action` with a substituted free-space probe (tests). A resume
    /// measures once, with this probe, inside `update_download_task`.
    pub fn task_action_with_free_space_probe(
        &self,
        task_id: String,
        revision: u64,
        action: UnifiedTaskAction,
        probe: &dyn FreeSpaceProbe,
    ) -> Result<TaskRef, TypedError> {
        let task = match action {
            UnifiedTaskAction::Archive => {
                let current = self.store_handle().map_err(task_error)?;
                let status = current
                    .lock()
                    .map_err(|_| invalid("暂时无法读取任务记录，请稍后重试"))?
                    .load_task_record(&task_id)
                    .map_err(|error| task_error(error.into()))?
                    .ok_or_else(|| invalid("任务不存在"))?
                    .runtime
                    .status;
                let task = self
                    .archive_download_task(&StoreArchiveTaskRequest {
                        task_id: task_id.clone(),
                        expected_revision: revision,
                        archived_at: chrono::Utc::now()
                            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                    })
                    .map_err(task_error)?;
                TaskRef {
                    task_id: task.task_id,
                    revision: task.revision,
                    status,
                    created: false,
                    warning: None,
                }
            }
            other => {
                if other == UnifiedTaskAction::Retry {
                    self.refuse_retry_without_files(&task_id, revision)?;
                }
                let kind = match other {
                    UnifiedTaskAction::Pause => TaskStateCommandKind::PauseTask,
                    UnifiedTaskAction::Resume => TaskStateCommandKind::ResumeTask,
                    UnifiedTaskAction::Retry => TaskStateCommandKind::RetryTask,
                    UnifiedTaskAction::Cancel => TaskStateCommandKind::CancelTask,
                    UnifiedTaskAction::Archive => unreachable!(),
                };
                let mut current_rev = revision;
                let mut last_err = None;
                let mut completed_snapshot = None;
                for attempt in 0..6 {
                    let request = StoreUpdateTaskRequest {
                        task_id: task_id.clone(),
                        expected_revision: current_rev,
                        command: StoreTaskCommand {
                            kind: kind.clone(),
                            item_ids: vec![],
                        },
                        event_id: format!("unified-action-{}", Uuid::new_v4()),
                        occurred_at: chrono::Utc::now()
                            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                    };
                    match self.update_download_task_with_free_space_probe(&request, probe) {
                        Ok(snapshot) => {
                            completed_snapshot = Some(snapshot);
                            break;
                        }
                        Err(err)
                            if attempt < 5
                                && other != UnifiedTaskAction::Retry
                                && err.code() == "V3_STORE_REVISION_CONFLICT" =>
                        {
                            let store = self.store_handle().map_err(task_error)?;
                            let record = store
                                .lock()
                                .map_err(|_| invalid("暂时无法读取任务记录，请稍后重试"))?
                                .load_task_record(&task_id)
                                .map_err(|error| task_error(error.into()))?;
                            let Some(record) = record else {
                                last_err = Some(err);
                                break;
                            };
                            let status = record.runtime.status;
                            let already_target = matches!(
                                (&other, &status),
                                (UnifiedTaskAction::Pause, TaskLifecycleStatus::Paused)
                                    | (UnifiedTaskAction::Resume, TaskLifecycleStatus::Running)
                                    | (UnifiedTaskAction::Cancel, TaskLifecycleStatus::Cancelled)
                            );
                            if already_target {
                                return Ok(TaskRef {
                                    task_id,
                                    revision: record.runtime.revision,
                                    status,
                                    created: false,
                                    warning: None,
                                });
                            }
                            let eligible = matches!(
                                (&other, &status),
                                (
                                    UnifiedTaskAction::Pause,
                                    TaskLifecycleStatus::Running | TaskLifecycleStatus::Queued
                                ) | (UnifiedTaskAction::Resume, TaskLifecycleStatus::Paused)
                                    | (
                                        UnifiedTaskAction::Cancel,
                                        TaskLifecycleStatus::Running
                                            | TaskLifecycleStatus::Queued
                                            | TaskLifecycleStatus::Paused
                                    )
                            );
                            if !eligible {
                                last_err = Some(err);
                                break;
                            }
                            current_rev = record.runtime.revision;
                        }
                        Err(err) => {
                            last_err = Some(err);
                            break;
                        }
                    }
                }
                let snapshot = match completed_snapshot {
                    Some(s) => s,
                    None => return Err(task_error(last_err.expect("attempt failed"))),
                };
                let task = snapshot
                    .core
                    .tasks
                    .into_iter()
                    .find(|task| task.task_id == task_id)
                    .ok_or_else(|| invalid("任务状态暂不可用"))?;
                TaskRef {
                    task_id,
                    revision: task.revision,
                    status: task.status,
                    created: false,
                    warning: None,
                }
            }
        };
        Ok(task)
    }

    pub fn task_item_action(
        &self,
        task_id: String,
        revision: Option<u64>,
        item_id: String,
        action: UnifiedTaskItemAction,
    ) -> Result<TaskRef, TypedError> {
        let probe = default_free_space_probe();
        self.task_item_action_with_free_space_probe(
            task_id,
            revision,
            item_id,
            action,
            probe.as_ref(),
        )
    }

    pub fn task_item_action_with_free_space_probe(
        &self,
        task_id: String,
        revision: Option<u64>,
        item_id: String,
        action: UnifiedTaskItemAction,
        probe: &dyn FreeSpaceProbe,
    ) -> Result<TaskRef, TypedError> {
        if task_id.trim().is_empty() || item_id.trim().is_empty() {
            return Err(invalid("任务或场景标识无效"));
        }
        self.startup().map_err(task_error)?;
        let mut current_rev_override = revision;

        for attempt in 0..6 {
            let store = self.store_handle().map_err(task_error)?;
            let (record, is_archived) = {
                let store = store
                    .lock()
                    .map_err(|_| invalid("暂时无法读取任务记录，请稍后重试"))?;
                let record = store
                    .load_task_record(&task_id)
                    .map_err(|error| task_error(error.into()))?
                    .ok_or_else(|| TypedError {
                        code: "V3_STORE_NOT_FOUND".to_owned(),
                        message_zh: "任务不存在或已被删除".to_owned(),
                        retryable: false,
                        needs_account: None,
                        needs_user_confirmation: false,
                    })?;
                let is_archived = store
                    .is_task_archived(&task_id)
                    .map_err(|error| task_error(error.into()))?;
                (record, is_archived)
            };

            let runtime = &record.runtime;
            if !runtime.selected_item_ids.iter().any(|id| id == &item_id) {
                return Err(invalid("该文件不属于当前下载任务"));
            }

            let in_active = runtime.active_item_ids.iter().any(|id| id == &item_id);
            let in_queued = runtime.queued_item_ids.iter().any(|id| id == &item_id);
            let in_paused = runtime.paused_item_ids.iter().any(|id| id == &item_id);
            let in_failed = runtime.failed_item_ids.iter().any(|id| id == &item_id);
            let in_cancelled = runtime.cancelled_item_ids.iter().any(|id| id == &item_id);
            let in_completed = runtime.completed_item_ids.iter().any(|id| id == &item_id);
            let in_skipped = runtime.skipped_item_ids.iter().any(|id| id == &item_id);

            let current_ref = TaskRef {
                task_id: task_id.clone(),
                revision: runtime.revision,
                status: runtime.status.clone(),
                created: false,
                warning: None,
            };

            match action {
                UnifiedTaskItemAction::Pause => {
                    if in_paused || (runtime.task_paused && (in_active || in_queued)) {
                        return Ok(current_ref);
                    }
                    if is_archived || (!in_active && !in_queued) {
                        return Err(invalid("只有等待中或下载中的文件可以暂停"));
                    }
                }
                UnifiedTaskItemAction::Resume => {
                    if !runtime.task_paused && (in_active || in_queued) {
                        return Ok(current_ref);
                    }
                    if is_archived
                        || !(in_paused || (runtime.task_paused && (in_active || in_queued)))
                    {
                        return Err(TypedError {
                            code: "TASK_RESUME_UNAVAILABLE".to_owned(),
                            message_zh: "只有已暂停的文件可以单独继续".to_owned(),
                            retryable: false,
                            needs_account: None,
                            needs_user_confirmation: false,
                        });
                    }
                    self.refuse_resume_on_low_disk(&task_id, probe)
                        .map_err(task_error)?;
                    self.ensure_task_session_for_admission(&task_id)
                        .map_err(task_error)?;
                }
                UnifiedTaskItemAction::Retry => {
                    if in_active || in_queued {
                        return Ok(current_ref);
                    }
                    if in_completed || in_skipped || in_paused || !(in_failed || in_cancelled) {
                        return Err(TypedError {
                            code: "TASK_RETRY_UNAVAILABLE".to_owned(),
                            message_zh: "仅失败或已取消的文件可以单独重试".to_owned(),
                            retryable: false,
                            needs_account: None,
                            needs_user_confirmation: false,
                        });
                    }
                    self.refuse_resume_on_low_disk(&task_id, probe)
                        .map_err(task_error)?;
                    self.ensure_task_session_for_admission(&task_id)
                        .map_err(task_error)?;
                }
                UnifiedTaskItemAction::Cancel => {
                    if in_cancelled {
                        return Ok(current_ref);
                    }
                    if is_archived || !(in_active || in_queued || in_paused) {
                        return Err(invalid("只有未完成的文件可以取消"));
                    }
                }
            }

            let expected_revision = current_rev_override.unwrap_or(runtime.revision);
            let kind = match action {
                UnifiedTaskItemAction::Pause => TaskStateCommandKind::PauseItems,
                UnifiedTaskItemAction::Resume => TaskStateCommandKind::ResumeItems,
                UnifiedTaskItemAction::Retry => TaskStateCommandKind::RetryItems,
                UnifiedTaskItemAction::Cancel => TaskStateCommandKind::CancelItems,
            };
            let request = StoreUpdateTaskRequest {
                task_id: task_id.clone(),
                expected_revision,
                command: StoreTaskCommand {
                    kind: kind.clone(),
                    item_ids: vec![item_id.clone()],
                },
                event_id: format!("unified-item-{}", Uuid::new_v4()),
                occurred_at: chrono::Utc::now()
                    .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            };

            let outcome = if action == UnifiedTaskItemAction::Resume && runtime.task_paused {
                self.mutate_business_store(|store| {
                    store.resume_paused_task_items(&request)?;
                    Ok(())
                })
                .map(|_| ())
            } else if action == UnifiedTaskItemAction::Retry && is_archived {
                self.mutate_business_store(|store| {
                    store.retry_archived_task_items(&request)?;
                    Ok(())
                })
                .map(|_| ())
            } else {
                self.update_download_task_with_free_space_probe(&request, probe)
                    .map(|_| ())
            };

            match outcome {
                Ok(()) => {
                    let store = self.store_handle().map_err(task_error)?;
                    let updated = store
                        .lock()
                        .map_err(|_| invalid("暂时无法读取任务记录，请稍后重试"))?
                        .load_task_record(&task_id)
                        .map_err(|error| task_error(error.into()))?
                        .ok_or_else(|| invalid("任务状态暂不可用"))?;
                    return Ok(TaskRef {
                        task_id,
                        revision: updated.runtime.revision,
                        status: updated.runtime.status,
                        created: false,
                        warning: None,
                    });
                }
                Err(err) if attempt < 5 && err.code() == "V3_STORE_REVISION_CONFLICT" => {
                    current_rev_override = None;
                    continue;
                }
                Err(err) => return Err(task_error(err)),
            }
        }

        Err(task_error(insar_store::StoreError::RevisionConflict.into()))
    }

    pub fn task_delete_history(&self, task_id: String) -> Result<(), TypedError> {
        if task_id.trim().is_empty() {
            return Err(invalid("任务编号无效"));
        }
        self.startup().map_err(task_error)?;
        let store = self.store_handle().map_err(task_error)?;
        let (record, is_archived, is_tombstoned) = {
            let store = store
                .lock()
                .map_err(|_| invalid("暂时无法读取任务记录，请稍后重试"))?;
            let is_tombstoned = store
                .is_task_tombstoned(&task_id)
                .map_err(|error| task_error(error.into()))?;
            let record = store
                .load_task_record(&task_id)
                .map_err(|error| task_error(error.into()))?;
            let is_archived = store
                .is_task_archived(&task_id)
                .map_err(|error| task_error(error.into()))?;
            (record, is_archived, is_tombstoned)
        };
        if is_tombstoned {
            return Ok(());
        }
        let record = record.ok_or_else(|| TypedError {
            code: "V3_STORE_NOT_FOUND".to_owned(),
            message_zh: "任务记录不存在或已删除".to_owned(),
            retryable: false,
            needs_account: None,
            needs_user_confirmation: false,
        })?;
        if matches!(
            record.runtime.status,
            TaskLifecycleStatus::Queued
                | TaskLifecycleStatus::Running
                | TaskLifecycleStatus::Paused
        ) {
            if record.snapshot.provider_id == "gacos.webform"
                && record.runtime.status == TaskLifecycleStatus::Paused
            {
                {
                    let store = store
                        .lock()
                        .map_err(|_| invalid("暂时无法读取任务记录，请稍后重试"))?;
                    store
                        .ensure_no_active_gacos_submission(&task_id)
                        .map_err(|error| task_error(error.into()))?;
                }
                let cancel_now =
                    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
                let _ = self
                    .update_download_task(&StoreUpdateTaskRequest {
                        task_id: task_id.clone(),
                        expected_revision: record.runtime.revision,
                        command: StoreTaskCommand {
                            kind: TaskStateCommandKind::CancelTask,
                            item_ids: Vec::new(),
                        },
                        event_id: format!(
                            "delete-gacos-history-cancel-{}-{}",
                            task_id, record.runtime.revision
                        ),
                        occurred_at: cancel_now,
                    })
                    .map_err(task_error)?;
            } else {
                return Err(TypedError {
                    code: "TASK_DELETE_HISTORY_ACTIVE".to_owned(),
                    message_zh: "进行中或暂停的任务不能删除历史记录，请先取消任务".to_owned(),
                    retryable: false,
                    needs_account: None,
                    needs_user_confirmation: false,
                });
            }
        }
        let latest_record = {
            let store = store
                .lock()
                .map_err(|_| invalid("暂时无法读取任务记录，请稍后重试"))?;
            store
                .load_task_record(&task_id)
                .map_err(|error| task_error(error.into()))?
                .ok_or_else(|| TypedError {
                    code: "V3_STORE_NOT_FOUND".to_owned(),
                    message_zh: "任务记录不存在或已删除".to_owned(),
                    retryable: false,
                    needs_account: None,
                    needs_user_confirmation: false,
                })?
        };
        if !is_archived {
            self.archive_download_task(&StoreArchiveTaskRequest {
                task_id: task_id.clone(),
                expected_revision: latest_record.runtime.revision,
                archived_at: chrono::Utc::now()
                    .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            })
            .map_err(task_error)?;
        }
        match self.delete_download_history(&StoreDeleteHistoryRequest {
            task_id,
            deleted_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        }) {
            Ok(_) => Ok(()),
            Err(ApplicationServiceError::Store(insar_store::StoreError::AlreadyExists)) => Ok(()),
            Err(error) => Err(task_error(error)),
        }
    }

    pub fn set_task_pinned(
        &self,
        task_id: String,
        pinned: bool,
    ) -> Result<Vec<String>, TypedError> {
        if task_id.trim().is_empty() {
            return Err(invalid("任务编号无效"));
        }
        self.startup().map_err(task_error)?;
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let store = self.store_handle().map_err(task_error)?;
        let mut store = store
            .lock()
            .map_err(|_| invalid("暂时无法读取任务记录，请稍后重试"))?;
        store
            .set_task_pinned(&task_id, pinned, &now)
            .map_err(|error| task_error(error.into()))
    }

    pub fn list_pinned_tasks(&self) -> Result<Vec<String>, ApplicationServiceError> {
        self.startup()?;
        let store = self.store_handle()?;
        let store = store
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?;
        store
            .list_pinned_task_ids()
            .map_err(ApplicationServiceError::from)
    }

    pub fn check_existing_downloaded_scenes(
        &self,
        items: Vec<ExistingDownloadCheckItem>,
        target_dir: Option<String>,
    ) -> Result<Vec<ExistingDownloadMatch>, ApplicationServiceError> {
        if items.len() > 500 {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: "too many scenes to check in one request".to_owned(),
            });
        }
        if items.is_empty() {
            return Ok(Vec::new());
        }
        self.startup()?;

        let completed_records: Vec<CompletedDownloadDestination> = {
            let store = self.store_handle()?;
            let store = store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            store.load_completed_download_destinations()?
        };

        let mut search_roots: Vec<PathBuf> = Vec::new();
        if let Some(dir) = target_dir
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            search_roots.push(PathBuf::from(dir));
        }
        if let Ok(settings) = self.get_sar_download_policy_settings() {
            let configured = settings.default_output_root.trim();
            if !configured.is_empty() {
                let path = PathBuf::from(configured);
                if !search_roots.iter().any(|existing| existing == &path) {
                    search_roots.push(path);
                }
            }
        }

        let canonical_roots: Vec<(PathBuf, PathBuf)> = search_roots
            .into_iter()
            .filter_map(|root| {
                fs::canonicalize(&root)
                    .ok()
                    .filter(|canon| canon.is_dir())
                    .map(|canon| (root, canon))
            })
            .collect();

        // Map canonical Store paths to their task title for attribution when
        // an existing file inside target_dir was created by an earlier task.
        let mut store_title_by_canon_path: BTreeMap<PathBuf, String> = BTreeMap::new();
        for rec in &completed_records {
            if let Some(title) = rec.task_title.as_ref() {
                if let Ok(canon) = fs::canonicalize(&rec.destination) {
                    store_title_by_canon_path
                        .entry(canon)
                        .or_insert_with(|| title.clone());
                }
            }
        }

        let mut matches = Vec::new();
        for item in items {
            let scene_id = item.scene_id.trim();
            let file_name = item.file_name.trim();
            if scene_id.is_empty() || file_name.is_empty() {
                continue;
            }
            if !is_single_normal_filename(file_name) {
                return Err(ApplicationServiceError::InvalidRequest {
                    reason: format!("invalid check file_name: {file_name}"),
                });
            }
            let relpath = match item.target_relpath.as_deref().map(str::trim) {
                Some(raw) if !raw.is_empty() => Some(validated_relpath(raw)?),
                _ => None,
            };

            let mut matched: Option<ExistingDownloadMatch> = None;

            // 1. Check files physically present under target_dir / default_output_root.
            'roots: for (root, canon_root) in &canonical_roots {
                let mut candidates = Vec::new();
                if let Some(rel) = relpath.as_ref() {
                    if rel.file_name() == Some(OsStr::new(file_name)) {
                        candidates.push(root.join(rel));
                    } else {
                        candidates.push(root.join(rel).join(file_name));
                    }
                }
                if let Ok(direct) =
                    insar_download::plan_download_target(root, &DownloadLayout::Direct, file_name)
                {
                    candidates.push(direct);
                } else {
                    candidates.push(root.join(file_name));
                }
                if let Ok(subdir) = insar_download::plan_download_target(
                    root,
                    &DownloadLayout::Sentinel1Slc,
                    file_name,
                ) {
                    candidates.push(subdir);
                }
                for candidate in candidates {
                    if let Ok(canon) = fs::canonicalize(&candidate) {
                        if canon.starts_with(canon_root) {
                            if let Ok(meta) = fs::metadata(&canon) {
                                if meta.is_file() && meta.len() > 0 {
                                    let task_title = store_title_by_canon_path.get(&canon).cloned();
                                    matched = Some(ExistingDownloadMatch {
                                        scene_id: scene_id.to_owned(),
                                        path: candidate.to_string_lossy().into_owned(),
                                        size_bytes: meta.len(),
                                        task_title,
                                    });
                                    break 'roots;
                                }
                            }
                        }
                    }
                }
            }

            // 2. Fall back to Store history completed destinations on disk (e.g. saved in another folder).
            if matched.is_none() {
                for rec in &completed_records {
                    let rec_name = rec.destination.file_name().and_then(|s| s.to_str());
                    let id_match = rec.item_id == scene_id || rec_name == Some(file_name);
                    if !id_match {
                        continue;
                    }
                    if let Ok(meta) = fs::metadata(&rec.destination) {
                        if meta.is_file() && meta.len() > 0 {
                            matched = Some(ExistingDownloadMatch {
                                scene_id: scene_id.to_owned(),
                                path: rec.destination.to_string_lossy().into_owned(),
                                size_bytes: meta.len(),
                                task_title: rec.task_title.clone(),
                            });
                            break;
                        }
                    }
                }
            }

            if let Some(hit) = matched {
                matches.push(hit);
            }
        }

        Ok(matches)
    }

    /// A retry queues the task's failed and cancelled files again; with none
    /// (a paused, queued or completed task) it is refused with
    /// `TASK_RETRY_UNAVAILABLE` and a Chinese reason (L4 r10 P1-1) instead
    /// of the store's internal invalid-snapshot code. An unknown task is left
    /// to the retry itself.
    fn refuse_retry_without_files(&self, task_id: &str, revision: u64) -> Result<(), TypedError> {
        let store = self.store_handle().map_err(task_error)?;
        let task = store
            .lock()
            .map_err(|_| invalid("暂时无法读取任务记录，请稍后重试"))?
            .load_task_record(task_id)
            .map_err(|error| task_error(error.into()))?;
        let Some(task) = task else {
            return Ok(());
        };
        if task.runtime.revision != revision {
            return Err(task_error(insar_store::StoreError::RevisionConflict.into()));
        }
        if !task.runtime.failed_item_ids.is_empty() || !task.runtime.cancelled_item_ids.is_empty() {
            return Ok(());
        }
        let message_zh = if task.runtime.status == TaskLifecycleStatus::Completed {
            "任务已全部完成，没有失败或已取消的文件需要重试"
        } else {
            "这个任务没有失败或已取消的文件，无需重试；已暂停的任务请用「继续」"
        };
        Err(TypedError {
            code: "TASK_RETRY_UNAVAILABLE".to_owned(),
            message_zh: message_zh.to_owned(),
            retryable: false,
            needs_account: None,
            needs_user_confirmation: false,
        })
    }

    /// A resume re-checks free space first (PM decision 2026-10-26): while the
    /// drive of the task's output root is below the engine's floor, or below
    /// the threshold recorded when the engine paused this task (which may
    /// include the file rule) and that pause names the same drive, the resume
    /// is refused with `DISK_LOW` instead of starting a download the engine
    /// would pause again at once. The task and its recorded reason are read
    /// under the Store lock; the volume is measured once, after releasing it.
    /// An unknown task or an unmeasurable drive is left to the resume itself.
    /// Used by every resume entry: `update_download_task` (legacy command,
    /// unified action, Agent) and the NISAR subset resume and start commands.
    pub(crate) fn refuse_resume_on_low_disk(
        &self,
        task_id: &str,
        probe: &dyn FreeSpaceProbe,
    ) -> Result<(), ApplicationServiceError> {
        match self.disk_low_resume_reason(task_id, probe)? {
            Some(reason) => Err(ApplicationServiceError::DiskLow { reason }),
            None => Ok(()),
        }
    }

    fn disk_low_resume_reason(
        &self,
        task_id: &str,
        probe: &dyn FreeSpaceProbe,
    ) -> Result<Option<TaskPauseReason>, ApplicationServiceError> {
        let (output_root, recorded) = {
            let store = self.store_handle()?;
            let store = store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            let Some(task) = store.load_task_record(task_id)? else {
                return Ok(None);
            };
            let output_root = task.snapshot.output_root;
            let recorded = store
                .load_task_pause_reasons(&[task_id.to_owned()])?
                .remove(task_id);
            (output_root, recorded)
        };
        Ok(disk_low_resume_reason(
            probe,
            &DiskLowPolicy::default(),
            &output_root,
            recorded.as_ref(),
        ))
    }

    /// Admit an exact frozen selection without reading the mutable Workbench
    /// slot. Assets and workers remain owned by the existing scheduler.
    pub fn create_task(
        &self,
        batch_id: String,
        revision: u64,
        item_ids: Vec<String>,
        options: UnifiedCreateTaskOptions,
    ) -> Result<TaskRef, TypedError> {
        self.create_task_with_preflight(
            batch_id,
            revision,
            item_ids,
            options,
            &crate::account_sessions::AccountSessionPreflight::new(self),
        )
    }

    pub fn create_task_with_preflight(
        &self,
        batch_id: String,
        revision: u64,
        item_ids: Vec<String>,
        options: UnifiedCreateTaskOptions,
        preflight: &dyn TaskAdmissionPreflight,
    ) -> Result<TaskRef, TypedError> {
        if batch_id.trim().is_empty()
            || revision != 0
            || item_ids.is_empty()
            || item_ids.len() > 250
            || item_ids.iter().any(|id| id.trim().is_empty())
            || item_ids.iter().collect::<HashSet<_>>().len() != item_ids.len()
            || options.output_root.as_os_str().is_empty()
            || options.concurrency == 0
        {
            return Err(invalid("冻结批次、版本、产品或输出设置无效"));
        }
        let store_handle = self.store_handle().map_err(task_error)?;
        // #22 B slice 2: the frozen batch header plus exactly the selected
        // products, from the bounded candidate rows (blob fallback for old
        // batches) — the whole candidate array is never materialized on the
        // admission path.
        // #22 B slice 5 (Owner 2026-11-03, option A): a batch that is still
        // loading admits exactly its arrived rows, frozen into a derived
        // admission batch the task references; the loading batch itself is
        // sealed once, later, and select-all over it keeps waiting.
        let (selection, admission) = {
            let store = store_handle
                .lock()
                .map_err(|_| invalid("暂时无法读取任务记录，请稍后重试"))?;
            hls_catalogue::reject_search_only_batch(&store, &batch_id).map_err(task_error)?;
            match store.load_batch_selection(&batch_id, &item_ids) {
                Ok(selection) => (selection.ok_or_else(|| invalid("冻结批次不存在"))?, None),
                Err(insar_store::StoreError::BatchStillLoading { .. }) => {
                    let admission = store
                        .loading_admission_batch(
                            &batch_id,
                            &item_ids,
                            &format!("progressive-admission-{}", Uuid::new_v4()),
                            &chrono::Utc::now()
                                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                        )
                        .map_err(|error| task_error(error.into()))?
                        .ok_or_else(|| invalid("冻结批次不存在"))?;
                    hls_catalogue::reject_search_only_query(&admission.query)
                        .map_err(task_error)?;
                    let mut header = admission.clone();
                    let items = std::mem::take(&mut header.candidates);
                    (
                        StoreBatchSelection {
                            batch: header,
                            items,
                        },
                        Some(admission),
                    )
                }
                Err(error) => return Err(task_error(error.into())),
            }
        };
        let StoreBatchSelection {
            batch,
            items: selected_products,
        } = selection;
        let selected: HashSet<&str> = selected_products
            .iter()
            .map(|item| item.item_id.as_str())
            .collect();
        if selected.len() != item_ids.len() {
            return Err(invalid("所选产品不在冻结批次中"));
        }
        // Unified-search burst and MPC Landsat C2 L2 freezes keep the parent
        // scene/burst product; the verified workers download per-asset items,
        // so expand the selection through their reviewed asset authorities
        // before creating the task.
        let burst_items =
            crate::expand_unified_burst_selection(&selected_products.iter().collect::<Vec<_>>())
                .map_err(task_error)?;
        let contains_burst = burst_items.is_some();
        // Apply a pending route change first: applied later, it would retire
        // the session the preflight or manifest fetch is about to establish.
        self.apply_pending_network_policy();
        let explicit_items = match burst_items {
            Some(items) => Some(items),
            None => self
                .expand_unified_landsat_selection(&batch, &selected_products)
                .map_err(task_error)?,
        };
        // Never hold Store or runtime locks while W1 checks session readiness.
        preflight.check(&batch, &selected_products, &item_ids)?;
        let warning = preflight.admission_warning();
        probe_storage_root(&options.output_root)?;
        // Orbit scheduled-fetch S4 (Owner 11-02 rule, review findings 2-4):
        // `None` follows the per-source policy (SLC and Burst default to on,
        // GRD defaults to off). The reuse check runs first — re-issuing
        // against a live task never errors, whatever the orbit options —
        // then the coarse pre-write refusal applies only to an explicitly
        // requested companion (`Some(true)`) with nothing supported; the
        // companion builder itself is the single source of truth for which
        // scenes it covered and how many it skipped, and the TaskRef warning
        // is built from that count. Never a silent skip.
        let include_precise_orbit = options.include_precise_orbit.unwrap_or_else(|| {
            companion_orbit_default_for_selection(&selected_products, contains_burst)
        });
        let supported_count = companion_orbit_supported_count(&selected_products);
        // Review finding 4: the capability refusal for an explicit request
        // names the orbit capability in Chinese instead of the generic
        // 「建任务失败」 catch-all.
        fn orbit_capability_refusal(error: ApplicationServiceError) -> TypedError {
            let code = error.code();
            if code == "V3_STORE_READINESS_UNSUPPORTED" || code == "RESOURCE_REGISTRY_UNAVAILABLE" {
                TypedError {
                    code: code.to_owned(),
                    message_zh: "精密轨道下载能力当前不可用，请稍后重试，或在选项中关闭精密轨道"
                        .to_owned(),
                    retryable: true,
                    needs_account: None,
                    needs_user_confirmation: false,
                }
            } else {
                task_error(error)
            }
        }
        // Review finding 2: the reuse check runs BEFORE the pre-write
        // refusals — re-issuing against a live task never errors, whatever
        // the orbit options or the capability state say.
        let reused_task_id = self
            .with_business_runtime(|runtime| {
                let store = runtime
                    .store
                    .lock()
                    .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                let existing = match explicit_items.as_deref() {
                    Some(items) => {
                        store.active_task_for_products_with_assets(&batch_id, &item_ids, items)?
                    }
                    None => store.active_task_for_products(&batch_id, &item_ids)?,
                };
                Ok(existing.map(|task| task.snapshot.task_id))
            })
            .map_err(task_error)?;
        if include_precise_orbit && options.include_precise_orbit == Some(true) {
            // Coarse pre-write refusals: only an explicit request with the
            // capability unavailable or nothing supported refuses — and
            // only for a task that is not being reused.
            if reused_task_id.is_none() && supported_count == 0 {
                return Err(invalid(
                    "精密轨道仅支持 Sentinel-1 SLC / Burst 场景，请在选项中关闭精轨后重试",
                ));
            }
            if reused_task_id.is_none() {
                if let Err(error) = self.ensure_formal_orbit_capability() {
                    return Err(orbit_capability_refusal(error));
                }
            }
        }
        let task = self
            .with_business_runtime(|runtime| {
                let mut store = runtime
                    .store
                    .lock()
                    .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                hls_catalogue::reject_search_only_batch(&store, &batch_id)?;
                let existing = match explicit_items.as_deref() {
                    Some(items) => {
                        store.active_task_for_products_with_assets(&batch_id, &item_ids, items)?
                    }
                    None => store.active_task_for_products(&batch_id, &item_ids)?,
                };
                if let Some(task) = existing {
                    // Idempotent reuse (review finding 2): the first call
                    // already created the companion or declined it. A reused
                    // task never gains one, never errors on the orbit
                    // options, and the ref names why no companion rides
                    // along when none exists.
                    let companion_exists = store
                        .load_task(&format!("{}-orbit", task.snapshot.task_id))?
                        .is_some();
                    let reuse_orbit_warning = if include_precise_orbit && !companion_exists {
                        Some("已复用现有任务，精密轨道未附带".to_owned())
                    } else {
                        None
                    };
                    return Ok(TaskRef {
                        task_id: task.snapshot.task_id,
                        revision: task.runtime.revision,
                        status: task.runtime.status,
                        created: false,
                        warning: join_warnings(warning, reuse_orbit_warning),
                    });
                }
                let mut request = StoreCreateTaskRequest {
                    task_id: format!("task-unified-{}", Uuid::new_v4()),
                    batch_id: admission
                        .as_ref()
                        .map_or_else(|| batch_id.clone(), |admission| admission.batch_id.clone()),
                    selected_item_ids: item_ids.clone(),
                    checked_item_ids: item_ids.clone(),
                    page_index: 0,
                    page_size: 100,
                    output_root: options.output_root,
                    layout: options.layout,
                    concurrency: options.concurrency,
                    created_at: chrono::Utc::now()
                        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                };
                let mut task = None;
                if let Some(task_items) = explicit_items {
                    // Explicit-asset tasks (ASF Burst per-burst folders and MPC
                    // Landsat C2 L2 per-scene file sets) use Direct layout, and
                    // every frozen asset item is initially selected.
                    request.layout = DownloadLayout::Direct;
                    let source_item_ids = item_ids.clone();
                    let asset_item_ids = task_items
                        .iter()
                        .map(|item| item.item_id.clone())
                        .collect::<Vec<_>>();
                    request.selected_item_ids = asset_item_ids.clone();
                    request.checked_item_ids = asset_item_ids;
                    task = Some(match &admission {
                        Some(admission) => store.create_task_from_loading_admission(
                            &batch_id,
                            admission,
                            &request,
                            Some((&source_item_ids, &task_items)),
                        )?,
                        None => store.create_task_with_explicit_items_from_frozen_batch(
                            &request,
                            &source_item_ids,
                            &task_items,
                        )?,
                    });
                } else {
                    request.checked_item_ids = item_ids;
                }
                let task = match (task, &admission) {
                    (Some(task), _) => task,
                    (None, Some(admission)) => store
                        .create_task_from_loading_admission(&batch_id, admission, &request, None)?,
                    (None, None) => store.create_task(&request)?,
                };
                drop(store);
                runtime
                    .scheduler
                    .as_ref()
                    .ok_or(ApplicationServiceError::BusinessRuntimeUnavailable)?
                    .wake();
                Ok(TaskRef {
                    task_id: task.snapshot.task_id.clone(),
                    status: task.runtime.status,
                    revision: task.runtime.revision,
                    created: true,
                    warning,
                })
            })
            .map_err(task_error)?;
        if !task.created {
            return Ok(task);
        }
        // Orbit scheduled-fetch S4 (review finding 3): the companion builder
        // is the single source of truth. On the unified path it always skips
        // unsupported scenes and reports how many it skipped; the TaskRef
        // warning is built from that count — the coarse parent-level check
        // never decides what the warning says. `Some(true)` with nothing
        // supported already refused above; a failure after the image row
        // exists (capability turned unavailable, or a defaulted-on request
        // with the capability unavailable) is a warning, never an error, so
        // the image task the caller already has is never poisoned.
        if include_precise_orbit {
            let orbit_warning =
                match self.create_companion_orbit_task_with_policy(&task.task_id, true) {
                    Ok(outcome) => {
                        if outcome.skipped_unsupported > 0 {
                            Some(format!(
                                "{} 景暂无法附带精密轨道",
                                outcome.skipped_unsupported
                            ))
                        } else {
                            None
                        }
                    }
                    Err(error) => {
                        eprintln!(
                            "[unified_tasks] companion orbit creation skipped: code={}",
                            error.code()
                        );
                        if options.include_precise_orbit == Some(true) {
                            Some(
                                "精密轨道下载能力当前不可用，请稍后重试，或在选项中关闭精密轨道"
                                    .to_owned(),
                            )
                        } else {
                            Some("精密轨道暂不可用，本次未附带".to_owned())
                        }
                    }
                };
            return Ok(TaskRef {
                warning: join_warnings(task.warning, orbit_warning),
                ..task
            });
        }
        Ok(task)
    }

    /// #22 B slice 4: preview a batch-bound all-match selection (`全选全部匹配`)
    /// without transporting all item IDs across IPC.
    pub fn preview_all_match_task(
        &self,
        request: BatchAllMatchPreviewRequest,
    ) -> Result<BatchAllMatchPreview, TypedError> {
        Ok(self
            .evaluate_batch_all_match(
                &request.batch_id,
                request.revision,
                request.predicate_version,
                &request.predicate,
                &request.exclusions,
            )?
            .preview)
    }

    /// #22 B slice 4: admit a batch-bound all-match selection (`全选全部匹配`)
    /// verified against `expected_membership_digest`, returning per-item
    /// accounting (`created_count` / `pending_count` / `skipped_count` /
    /// `rejected_count`) that sums to `matched_count`.
    pub fn create_all_match_task(
        &self,
        request: BatchAllMatchCreateTaskRequest,
    ) -> Result<BatchAllMatchTaskOutcome, TypedError> {
        self.create_all_match_task_with_preflight(
            request,
            &crate::account_sessions::AccountSessionPreflight::new(self),
        )
    }

    pub fn create_all_match_task_with_preflight(
        &self,
        request: BatchAllMatchCreateTaskRequest,
        preflight: &dyn TaskAdmissionPreflight,
    ) -> Result<BatchAllMatchTaskOutcome, TypedError> {
        if request.options.output_root.as_os_str().is_empty()
            || request.options.concurrency == 0
            || request.expected_membership_digest.trim().is_empty()
        {
            return Err(invalid("冻结批次、输出路径、并发度或确认摘要无效"));
        }
        let evaluation = self.evaluate_batch_all_match(
            &request.batch_id,
            request.revision,
            request.predicate_version,
            &request.predicate,
            &request.exclusions,
        )?;
        if evaluation.preview.membership_digest != request.expected_membership_digest.trim() {
            return Err(TypedError {
                code: "V3_TASK_SELECTION_CHANGED".to_owned(),
                message_zh: "检索批次或活动任务状态已变化，请重新确认下载范围".to_owned(),
                retryable: true,
                needs_account: None,
                needs_user_confirmation: false,
            });
        }

        let options = request.options;
        let batch_id = evaluation.preview.batch_id.clone();

        // Case 1: Every downloadable matched item is already covered by at least
        // one active task (`admissible_count == 0` and `skipped_active_count > 0`).
        if evaluation.admissible_items.is_empty() {
            let skipped_ids: Vec<String> = evaluation
                .skipped_active_items
                .iter()
                .map(|item| item.item_id.clone())
                .collect();
            let burst_items = crate::expand_unified_burst_selection(
                &evaluation.skipped_active_items.iter().collect::<Vec<_>>(),
            )
            .map_err(task_error)?;
            let contains_burst = burst_items.is_some();
            self.apply_pending_network_policy();
            let explicit_items = match burst_items {
                Some(items) => Some(items),
                None => self
                    .expand_unified_landsat_selection(
                        &evaluation.batch,
                        &evaluation.skipped_active_items,
                    )
                    .map_err(task_error)?,
            };
            let include_precise_orbit = options.include_precise_orbit.unwrap_or_else(|| {
                companion_orbit_default_for_selection(
                    &evaluation.skipped_active_items,
                    contains_burst,
                )
            });
            let task = self
                .with_business_runtime(|runtime| {
                    let store = runtime
                        .store
                        .lock()
                        .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                    let existing = match explicit_items.as_deref() {
                        Some(items) => store.active_task_for_products_with_assets(
                            &batch_id,
                            &skipped_ids,
                            items,
                        )?,
                        None => store.active_task_for_products(&batch_id, &skipped_ids)?,
                    };
                    let Some(task) = existing else {
                        return Err(insar_store::StoreError::PartialActiveTaskOverlap.into());
                    };
                    let companion_exists = store
                        .load_task(&format!("{}-orbit", task.snapshot.task_id))?
                        .is_some();
                    let reuse_orbit_warning = if include_precise_orbit && !companion_exists {
                        Some("已复用现有任务，精密轨道未附带".to_owned())
                    } else {
                        None
                    };
                    Ok(TaskRef {
                        task_id: task.snapshot.task_id,
                        revision: task.runtime.revision,
                        status: task.runtime.status,
                        created: false,
                        warning: reuse_orbit_warning,
                    })
                })
                .map_err(task_error)?;
            return Ok(BatchAllMatchTaskOutcome {
                task,
                batch_id,
                revision: evaluation.preview.revision,
                membership_digest: evaluation.preview.membership_digest,
                matched_count: evaluation.preview.matched_count,
                created_count: 0,
                pending_count: 0,
                skipped_count: evaluation.preview.skipped_active_count,
                rejected_count: evaluation.preview.rejected_count,
            });
        }

        // Case 2: Admit the uncovered downloadable matched items (`admissible_items`).
        let admissible_ids: Vec<String> = evaluation
            .admissible_items
            .iter()
            .map(|item| item.item_id.clone())
            .collect();
        let burst_items = crate::expand_unified_burst_selection(
            &evaluation.admissible_items.iter().collect::<Vec<_>>(),
        )
        .map_err(task_error)?;
        let contains_burst = burst_items.is_some();
        self.apply_pending_network_policy();
        let explicit_items = match burst_items {
            Some(items) => Some(items),
            None => self
                .expand_unified_landsat_selection(&evaluation.batch, &evaluation.admissible_items)
                .map_err(task_error)?,
        };
        preflight.check(
            &evaluation.batch,
            &evaluation.admissible_items,
            &admissible_ids,
        )?;
        let warning = preflight.admission_warning();
        probe_storage_root(&options.output_root)?;

        let include_precise_orbit = options.include_precise_orbit.unwrap_or_else(|| {
            companion_orbit_default_for_selection(&evaluation.admissible_items, contains_burst)
        });
        let supported_count = companion_orbit_supported_count(&evaluation.admissible_items);
        fn orbit_capability_refusal(error: ApplicationServiceError) -> TypedError {
            let code = error.code();
            if code == "V3_STORE_READINESS_UNSUPPORTED" || code == "RESOURCE_REGISTRY_UNAVAILABLE" {
                TypedError {
                    code: code.to_owned(),
                    message_zh: "精密轨道下载能力当前不可用，请稍后重试，或在选项中关闭精密轨道"
                        .to_owned(),
                    retryable: true,
                    needs_account: None,
                    needs_user_confirmation: false,
                }
            } else {
                task_error(error)
            }
        }
        if include_precise_orbit && options.include_precise_orbit == Some(true) {
            if supported_count == 0 {
                return Err(invalid(
                    "精密轨道仅支持 Sentinel-1 SLC / Burst 场景，请在选项中关闭精轨后重试",
                ));
            }
            if let Err(error) = self.ensure_formal_orbit_capability() {
                return Err(orbit_capability_refusal(error));
            }
        }

        let expected_active_covered = evaluation.active_covered;
        let task = self
            .with_business_runtime(|runtime| {
                let mut store = runtime
                    .store
                    .lock()
                    .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                hls_catalogue::reject_search_only_batch(&store, &batch_id)?;
                let current_active = store.active_covered_items_for_batch(&batch_id)?;
                if current_active != expected_active_covered {
                    return Err(ApplicationServiceError::InvalidRequest {
                        reason: "active task coverage changed before admission".to_owned(),
                    });
                }
                let mut create_request = StoreCreateTaskRequest {
                    task_id: format!("task-unified-{}", Uuid::new_v4()),
                    batch_id: batch_id.clone(),
                    selected_item_ids: admissible_ids.clone(),
                    checked_item_ids: admissible_ids.clone(),
                    page_index: 0,
                    page_size: 100,
                    output_root: options.output_root,
                    layout: options.layout,
                    concurrency: options.concurrency,
                    created_at: chrono::Utc::now()
                        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                };
                let task = if let Some(task_items) = explicit_items {
                    create_request.layout = DownloadLayout::Direct;
                    let asset_item_ids = task_items
                        .iter()
                        .map(|item| item.item_id.clone())
                        .collect::<Vec<_>>();
                    create_request.selected_item_ids = asset_item_ids.clone();
                    create_request.checked_item_ids = asset_item_ids;
                    store.create_task_with_explicit_items_from_frozen_batch(
                        &create_request,
                        &admissible_ids,
                        &task_items,
                    )?
                } else {
                    store.create_task(&create_request)?
                };
                drop(store);
                runtime
                    .scheduler
                    .as_ref()
                    .ok_or(ApplicationServiceError::BusinessRuntimeUnavailable)?
                    .wake();
                Ok(TaskRef {
                    task_id: task.snapshot.task_id,
                    status: task.runtime.status,
                    revision: task.runtime.revision,
                    created: true,
                    warning,
                })
            })
            .map_err(|error| {
                if matches!(
                    &error,
                    ApplicationServiceError::InvalidRequest { reason }
                        if reason == "active task coverage changed before admission"
                ) {
                    TypedError {
                        code: "V3_TASK_SELECTION_CHANGED".to_owned(),
                        message_zh: "检索批次或活动任务状态已变化，请重新确认下载范围".to_owned(),
                        retryable: true,
                        needs_account: None,
                        needs_user_confirmation: false,
                    }
                } else {
                    task_error(error)
                }
            })?;

        let final_task = if include_precise_orbit && task.created {
            let orbit_warning =
                match self.create_companion_orbit_task_with_policy(&task.task_id, true) {
                    Ok(outcome) => {
                        if outcome.skipped_unsupported > 0 {
                            Some(format!(
                                "{} 景暂无法附带精密轨道",
                                outcome.skipped_unsupported
                            ))
                        } else {
                            None
                        }
                    }
                    Err(error) => {
                        eprintln!(
                            "[unified_tasks] companion orbit creation skipped: code={}",
                            error.code()
                        );
                        if options.include_precise_orbit == Some(true) {
                            Some(
                                "精密轨道下载能力当前不可用，请稍后重试，或在选项中关闭精密轨道"
                                    .to_owned(),
                            )
                        } else {
                            Some("精密轨道暂不可用，本次未附带".to_owned())
                        }
                    }
                };
            TaskRef {
                warning: join_warnings(task.warning, orbit_warning),
                ..task
            }
        } else {
            task
        };

        let created_count = evaluation.preview.admissible_count;
        Ok(BatchAllMatchTaskOutcome {
            task: final_task,
            batch_id,
            revision: evaluation.preview.revision,
            membership_digest: evaluation.preview.membership_digest,
            matched_count: evaluation.preview.matched_count,
            created_count,
            pending_count: created_count,
            skipped_count: evaluation.preview.skipped_active_count,
            rejected_count: evaluation.preview.rejected_count,
        })
    }

    fn evaluate_batch_all_match(
        &self,
        batch_id: &str,
        revision: u64,
        predicate_version: u32,
        predicate: &BatchSelectionPredicate,
        exclusions: &[String],
    ) -> Result<AllMatchEvaluation, TypedError> {
        let batch_id = batch_id.trim();
        if batch_id.is_empty() || revision != 0 {
            return Err(invalid("冻结批次或版本号无效"));
        }
        if predicate_version != ALL_MATCH_PREDICATE_VERSION_V1 {
            return Err(invalid("全选过滤条件协议版本不受支持"));
        }
        let canonical_predicate = normalize_selection_predicate(predicate)?;
        if exclusions.len() > MAX_ALL_MATCH_EXCLUSIONS {
            return Err(invalid("全选排除项超出 250 景上限"));
        }
        let mut exclusion_set = BTreeSet::new();
        for id in exclusions {
            let trimmed = id.trim();
            if trimmed.is_empty()
                || trimmed.chars().count() > 256
                || trimmed.chars().any(char::is_control)
                || !exclusion_set.insert(trimmed.to_owned())
            {
                return Err(invalid("全选排除项 ID 无效或存在重复"));
            }
        }

        let store_handle = self.store_handle().map_err(task_error)?;
        let (selection, active_covered) = {
            let store = store_handle
                .lock()
                .map_err(|_| invalid("暂时无法读取任务记录，请稍后重试"))?;
            if let Some(progress) = store
                .search_batch_progress(batch_id)
                .map_err(|error| task_error(error.into()))?
            {
                match progress.state {
                    SearchBatchProgressState::Loading => {
                        return Err(task_error(
                            insar_store::StoreError::BatchStillLoading {
                                batch_id: batch_id.to_owned(),
                            }
                            .into(),
                        ));
                    }
                    SearchBatchProgressState::Cancelled
                    | SearchBatchProgressState::Failed
                    | SearchBatchProgressState::Interrupted => {
                        return Err(TypedError {
                            code: "V3_TASK_BATCH_INCOMPLETE".to_owned(),
                            message_zh: "当前检索已提前结束，结果未完整加载，无法执行全选全部匹配"
                                .to_owned(),
                            retryable: false,
                            needs_account: None,
                            needs_user_confirmation: false,
                        });
                    }
                    SearchBatchProgressState::Complete => {
                        if progress
                            .remote_total
                            .is_some_and(|total| total > MAX_ALL_MATCH_CANDIDATES as u64)
                        {
                            return Err(over_ceiling_error());
                        }
                    }
                }
            }
            hls_catalogue::reject_search_only_batch(&store, batch_id).map_err(task_error)?;
            let selection = store
                .load_all_batch_candidates(batch_id)
                .map_err(|error| task_error(error.into()))?
                .ok_or_else(|| invalid("冻结批次不存在"))?;
            let active_covered = store
                .active_covered_items_for_batch(batch_id)
                .map_err(|error| task_error(error.into()))?;
            (selection, active_covered)
        };

        let StoreBatchSelection { batch, items } = selection;
        if items.len() > MAX_ALL_MATCH_CANDIDATES
            || batch
                .search_stats
                .as_ref()
                .and_then(|stats| stats.matched_count)
                .is_some_and(|matched| matched > MAX_ALL_MATCH_CANDIDATES as u64)
        {
            return Err(over_ceiling_error());
        }

        let candidate_id_set: HashSet<&str> =
            items.iter().map(|item| item.item_id.as_str()).collect();
        for excluded_id in &exclusion_set {
            if !candidate_id_set.contains(excluded_id.as_str()) {
                return Err(TypedError {
                    code: "V3_TASK_SELECTION_INVALID".to_owned(),
                    message_zh: "全选排除项不在当前检索批次中".to_owned(),
                    retryable: false,
                    needs_account: None,
                    needs_user_confirmation: false,
                });
            }
        }

        let mut matched_ids = Vec::new();
        let mut admissible_items = Vec::new();
        let mut skipped_active_items = Vec::new();
        let mut rejected_ids = Vec::new();
        let mut orbit_directions = BTreeSet::new();

        for item in items {
            if exclusion_set.contains(item.item_id.as_str()) {
                continue;
            }
            if !matches_selection_predicate(&item, &canonical_predicate) {
                continue;
            }
            matched_ids.push(item.item_id.clone());
            if let Some(dir) = extract_item_orbit_direction(&item) {
                orbit_directions.insert(dir);
            }
            if !is_candidate_downloadable(&item) {
                rejected_ids.push(item.item_id);
            } else if active_covered.contains(item.item_id.as_str()) {
                skipped_active_items.push(item);
            } else {
                admissible_items.push(item);
            }
        }

        let matched_count = matched_ids.len();
        let admissible_count = admissible_items.len();
        let skipped_active_count = skipped_active_items.len();
        let rejected_count = rejected_ids.len();
        debug_assert_eq!(
            admissible_count + skipped_active_count + rejected_count,
            matched_count
        );

        if matched_count == 0 || (admissible_count == 0 && skipped_active_count == 0) {
            return Err(TypedError {
                code: "V3_TASK_SELECTION_INVALID".to_owned(),
                message_zh: "没有符合条件的可下载产品".to_owned(),
                retryable: false,
                needs_account: None,
                needs_user_confirmation: false,
            });
        }

        let size_reference_items = if !admissible_items.is_empty() {
            &admissible_items
        } else {
            &skipped_active_items
        };
        let mut known_bytes = 0_u64;
        let mut unknown_size_count = 0_usize;
        for item in size_reference_items {
            match extract_item_size_bytes(item) {
                Some(bytes) => {
                    known_bytes = known_bytes.saturating_add(bytes);
                }
                None => {
                    unknown_size_count += 1;
                }
            }
        }

        let admissible_ids: Vec<&str> = admissible_items
            .iter()
            .map(|item| item.item_id.as_str())
            .collect();
        let skipped_active_ids: Vec<&str> = skipped_active_items
            .iter()
            .map(|item| item.item_id.as_str())
            .collect();
        let digest_payload = serde_json::json!({
            "batch_id": batch_id,
            "revision": revision,
            "predicate_version": predicate_version,
            "predicate": &canonical_predicate,
            "exclusions": &exclusion_set,
            "matched_ids": &matched_ids,
            "admissible_ids": &admissible_ids,
            "skipped_active_ids": &skipped_active_ids,
            "rejected_ids": &rejected_ids,
        });
        let digest_bytes =
            Sha256::digest(serde_json::to_vec(&digest_payload).expect("canonical JSON serializes"));
        let membership_digest = format!("{digest_bytes:x}");

        let preview = BatchAllMatchPreview {
            batch_id: batch_id.to_owned(),
            revision,
            predicate_version,
            matched_count,
            admissible_count,
            skipped_active_count,
            rejected_count,
            known_bytes,
            unknown_size_count,
            membership_digest,
            orbit_directions: orbit_directions.into_iter().collect(),
        };

        Ok(AllMatchEvaluation {
            batch,
            preview,
            admissible_items,
            skipped_active_items,
            active_covered,
        })
    }

    fn expand_unified_landsat_selection(
        &self,
        batch: &FrozenSearchBatch,
        selected_products: &[SnapshotItem],
    ) -> Result<Option<Vec<SnapshotItem>>, ApplicationServiceError> {
        let contains_mpc_landsat = selected_products.iter().any(|item| {
            item.provider_id == insar_provider::catalog::MPC_LANDSAT_PROVIDER_ID
                && item
                    .metadata
                    .get("logical_dataset")
                    .and_then(serde_json::Value::as_str)
                    == Some(crate::LANDSAT_C2_L2_LOGICAL_DATASET)
                && item.execution_policy.is_none()
        });
        if !contains_mpc_landsat {
            return Ok(None);
        }
        if self
            .dispatcher_handle()?
            .catalogue_route_use()
            .map_err(ApplicationServiceError::Provider)?
            != crate::RouteUse::Production
        {
            return Err(ApplicationServiceError::LandsatAssetEvidenceInvalid);
        }
        let mut evidence_batch = batch.clone();
        evidence_batch.candidates = selected_products.to_vec();
        let cancelled = std::sync::atomic::AtomicBool::new(false);
        let mut expanded = Vec::new();
        for chunk in selected_products.chunks(2) {
            let outcomes = std::thread::scope(|scope| {
                let handles = chunk
                    .iter()
                    .map(|item| {
                        let evidence_batch = &evidence_batch;
                        let cancelled = &cancelled;
                        scope.spawn(move || {
                            let evidence =
                                crate::frozen_landsat_scene_evidence(evidence_batch, 1, item)?;
                            Ok::<Vec<SnapshotItem>, ApplicationServiceError>(
                                self.prepare_landsat_asset_items(&evidence, item, &[], cancelled)?
                                    .items,
                            )
                        })
                    })
                    .collect::<Vec<_>>();
                handles
                    .into_iter()
                    .map(|handle| {
                        handle
                            .join()
                            .map_err(|_| ApplicationServiceError::RuntimeInitialization)?
                    })
                    .collect::<Result<Vec<_>, _>>()
            })?;
            for scene_items in outcomes {
                expanded.extend(scene_items);
            }
        }
        if expanded.is_empty() {
            return Err(ApplicationServiceError::LandsatAssetEvidenceInvalid);
        }
        let unique_ids = expanded
            .iter()
            .map(|item| item.item_id.as_str())
            .collect::<BTreeSet<_>>();
        let unique_names = expanded
            .iter()
            .map(|item| item.display_name.to_ascii_lowercase())
            .collect::<BTreeSet<_>>();
        if unique_ids.len() != expanded.len() || unique_names.len() != expanded.len() {
            return Err(ApplicationServiceError::LandsatAssetEvidenceInvalid);
        }
        // Prioritize lightweight scene metadata files (MTL.txt, MTL.xml, ANG.txt)
        // ahead of multi-megabyte GeoTIFF bands while preserving relative order.
        expanded.sort_by_key(|item| {
            !matches!(
                item.asset_access.as_ref().map(|access| &access.role),
                Some(insar_core::AssetRole::Metadata)
            )
        });
        Ok(Some(expanded))
    }
}

struct AllMatchEvaluation {
    batch: FrozenSearchBatch,
    preview: BatchAllMatchPreview,
    admissible_items: Vec<SnapshotItem>,
    skipped_active_items: Vec<SnapshotItem>,
    active_covered: HashSet<String>,
}

fn over_ceiling_error() -> TypedError {
    TypedError {
        code: "V3_TASK_OVER_CEILING".to_owned(),
        message_zh: "匹配结果超过 10,000 景安全上限，请缩小范围或时间段后重试".to_owned(),
        retryable: false,
        needs_account: None,
        needs_user_confirmation: false,
    }
}

fn normalize_optional_scalar(
    value: &Option<String>,
    field: &str,
) -> Result<Option<String>, TypedError> {
    let Some(raw) = value.as_deref() else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.chars().count() > 128 || trimmed.chars().any(char::is_control) {
        return Err(invalid(&format!("全选过滤条件 {field} 格式无效")));
    }
    Ok(Some(trimmed.to_owned()))
}

fn normalize_selection_predicate(
    predicate: &BatchSelectionPredicate,
) -> Result<BatchSelectionPredicate, TypedError> {
    let orbit_direction =
        match normalize_optional_scalar(&predicate.orbit_direction, "orbit_direction")? {
            Some(raw) => {
                let lower = raw.to_ascii_lowercase();
                if lower == "all" {
                    None
                } else if lower == "ascending" || lower == "asc" {
                    Some("ascending".to_owned())
                } else if lower == "descending" || lower == "desc" {
                    Some("descending".to_owned())
                } else {
                    return Err(invalid("全选轨道方向过滤条件无效"));
                }
            }
            None => None,
        };
    Ok(BatchSelectionPredicate {
        orbit_direction,
        platform: normalize_optional_scalar(&predicate.platform, "platform")?,
        product_type: normalize_optional_scalar(&predicate.product_type, "product_type")?,
        beam_mode: normalize_optional_scalar(&predicate.beam_mode, "beam_mode")?,
        polarization: normalize_optional_scalar(&predicate.polarization, "polarization")?,
        relative_orbit: predicate.relative_orbit,
        frame: predicate.frame,
        text_query: normalize_optional_scalar(&predicate.text_query, "text_query")?,
    })
}

fn item_properties(item: &SnapshotItem) -> Option<&serde_json::Map<String, serde_json::Value>> {
    item.metadata
        .get("properties")
        .and_then(serde_json::Value::as_object)
}

fn extract_string_field<'a>(item: &'a SnapshotItem, keys: &[&str]) -> Option<&'a str> {
    let props = item_properties(item);
    for key in keys {
        if let Some(val) = item
            .metadata
            .get(*key)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return Some(val);
        }
        if let Some(val) = item
            .payload
            .get(*key)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return Some(val);
        }
        if let Some(val) = props
            .and_then(|p| p.get(*key))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return Some(val);
        }
    }
    None
}

fn parse_u32_value(value: &serde_json::Value) -> Option<u32> {
    if let Some(num) = value.as_u64() {
        return u32::try_from(num).ok();
    }
    if let Some(text) = value.as_str() {
        let trimmed = text.trim();
        if !trimmed.is_empty() && trimmed.chars().all(|c| c.is_ascii_digit()) {
            return trimmed.parse::<u32>().ok();
        }
    }
    None
}

fn extract_u32_field(item: &SnapshotItem, keys: &[&str]) -> Option<u32> {
    let props = item_properties(item);
    for key in keys {
        if let Some(val) = item.metadata.get(*key).and_then(parse_u32_value) {
            return Some(val);
        }
        if let Some(val) = item.payload.get(*key).and_then(parse_u32_value) {
            return Some(val);
        }
        if let Some(val) = props.and_then(|p| p.get(*key)).and_then(parse_u32_value) {
            return Some(val);
        }
    }
    None
}

fn extract_item_orbit_direction(item: &SnapshotItem) -> Option<String> {
    let raw = extract_string_field(item, &["flight_direction", "orbit_direction"])?;
    let lower = raw.to_ascii_lowercase();
    if lower.contains("asc") {
        Some("ascending".to_owned())
    } else if lower.contains("desc") {
        Some("descending".to_owned())
    } else {
        None
    }
}

fn matches_selection_predicate(item: &SnapshotItem, predicate: &BatchSelectionPredicate) -> bool {
    if let Some(expected_dir) = predicate.orbit_direction.as_deref() {
        if extract_item_orbit_direction(item).as_deref() != Some(expected_dir) {
            return false;
        }
    }
    if let Some(expected_platform) = predicate.platform.as_deref() {
        let actual = extract_string_field(item, &["platform", "spacecraft"])
            .unwrap_or(item.provider_id.as_str());
        if !actual.eq_ignore_ascii_case(expected_platform)
            && !item
                .display_name
                .to_ascii_uppercase()
                .starts_with(&expected_platform.to_ascii_uppercase())
        {
            return false;
        }
    }
    if let Some(expected_product_type) = predicate.product_type.as_deref() {
        let Some(actual) = extract_string_field(item, &["product_type"]) else {
            return false;
        };
        if !actual.eq_ignore_ascii_case(expected_product_type) {
            return false;
        }
    }
    if let Some(expected_beam) = predicate.beam_mode.as_deref() {
        let Some(actual) = extract_string_field(item, &["beam_mode"]) else {
            return false;
        };
        if !actual.eq_ignore_ascii_case(expected_beam) {
            return false;
        }
    }
    if let Some(expected_pol) = predicate.polarization.as_deref() {
        let Some(actual) = extract_string_field(item, &["polarization"]) else {
            return false;
        };
        if !actual.eq_ignore_ascii_case(expected_pol) {
            return false;
        }
    }
    if let Some(expected_orbit) = predicate.relative_orbit {
        if extract_u32_field(item, &["relative_orbit", "track"]) != Some(expected_orbit) {
            return false;
        }
    }
    if let Some(expected_frame) = predicate.frame {
        if extract_u32_field(item, &["frame"]) != Some(expected_frame) {
            return false;
        }
    }
    if let Some(query) = predicate.text_query.as_deref() {
        let q = query.to_ascii_lowercase();
        let in_id = item.item_id.to_ascii_lowercase().contains(&q);
        let in_name = item.display_name.to_ascii_lowercase().contains(&q);
        if !in_id && !in_name {
            return false;
        }
    }
    true
}

fn is_candidate_downloadable(item: &SnapshotItem) -> bool {
    if item
        .metadata
        .get("delivery_policy")
        .and_then(serde_json::Value::as_str)
        == Some("search_only")
        || item
            .metadata
            .get("search_only")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
        || item
            .metadata
            .get("downloadable")
            .and_then(serde_json::Value::as_bool)
            == Some(false)
    {
        return false;
    }
    if item.provider_id == "asf.catalog"
        && item
            .metadata
            .get("dataset")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|dataset| dataset.eq_ignore_ascii_case("SLC-BURST"))
    {
        return crate::expand_unified_burst_selection(&[item])
            .ok()
            .flatten()
            .is_some_and(|expanded| !expanded.is_empty());
    }
    true
}

fn parse_u64_size(value: &serde_json::Value) -> Option<u64> {
    if let Some(num) = value.as_u64() {
        return Some(num);
    }
    if let Some(num) = value.as_i64() {
        return u64::try_from(num).ok();
    }
    if let Some(text) = value.as_str() {
        let trimmed = text.trim();
        if !trimmed.is_empty() && trimmed.chars().all(|c| c.is_ascii_digit()) {
            return trimmed.parse::<u64>().ok();
        }
    }
    None
}

fn extract_item_size_bytes(item: &SnapshotItem) -> Option<u64> {
    if let Some(expected) = item
        .asset_access
        .as_ref()
        .and_then(|access| access.expected_size_bytes)
    {
        return Some(expected);
    }
    for key in ["size_bytes", "file_size_remote"] {
        if let Some(size) = item.metadata.get(key).and_then(parse_u64_size) {
            return Some(size);
        }
        if let Some(size) = item.payload.get(key).and_then(parse_u64_size) {
            return Some(size);
        }
    }
    for assets in [item.metadata.get("assets"), item.payload.get("assets")]
        .into_iter()
        .flatten()
    {
        if let Some(arr) = assets.as_array() {
            for entry in arr {
                if let Some(size) = entry
                    .as_object()
                    .and_then(|obj| obj.get("size_bytes"))
                    .and_then(parse_u64_size)
                {
                    return Some(size);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod archive_tests {
    use super::{archive_failed_candidates, auto_archive_candidate, task_error};
    use insar_core::TaskLifecycleStatus;

    #[test]
    fn auto_archive_matches_the_legacy_terminal_set_and_nisar_exclusion() {
        use TaskLifecycleStatus::*;
        for (status, plain, nisar) in [
            (Completed, true, true),
            (Cancelled, true, true),
            (Failed, true, false),
            (Interrupted, true, false),
            (Queued, false, false),
            (Running, false, false),
            (Paused, false, false),
        ] {
            assert_eq!(auto_archive_candidate(&status, None), plain, "{status:?}");
            assert_eq!(
                auto_archive_candidate(&status, Some("nisar_subset")),
                nisar,
                "nisar {status:?}"
            );
        }
    }
    use crate::ApplicationServiceError;
    use insar_core::ContractError;
    use insar_store::StoreError;

    #[test]
    fn concurrent_archive_conflicts_do_not_fail_the_whole_overview() {
        let mut visited = Vec::new();
        let failures = ["revision", "gacos", "already", "next"].map(|id| (id.to_owned(), 2));
        archive_failed_candidates(failures, |request| {
            visited.push(request.task_id.clone());
            match request.task_id.as_str() {
                "revision" => Err(StoreError::RevisionConflict),
                "gacos" => Err(StoreError::GacosSubmissionInFlight),
                "already" => Err(StoreError::AlreadyExists),
                _ => Ok(()),
            }
        })
        .expect("skippable races leave later tasks readable");
        assert_eq!(visited, ["revision", "gacos", "already", "next"]);
    }

    #[test]
    fn archive_conflicts_are_skipped_but_other_store_errors_propagate() {
        let errors = [
            (StoreError::GacosSubmissionInFlight, true),
            (StoreError::AlreadyExists, true),
            (StoreError::RevisionConflict, true),
            (StoreError::NotFound, false),
        ];
        for (error, should_succeed) in errors {
            let mut error = Some(error);
            let result =
                archive_failed_candidates([("task".to_owned(), 1)], |_| Err(error.take().unwrap()));
            assert_eq!(result.is_ok(), should_succeed);
        }
    }

    #[test]
    fn task_error_preserves_code_and_distinguishes_common_categories() {
        for (error, fragment, retryable) in [
            (
                ApplicationServiceError::from(StoreError::RevisionConflict),
                "修订",
                false,
            ),
            (
                ApplicationServiceError::from(StoreError::NotFound),
                "不存在",
                false,
            ),
            (
                ApplicationServiceError::from(StoreError::AlreadyExists),
                "已存在",
                false,
            ),
            (
                ApplicationServiceError::Provider(
                    ContractError::new(
                        "CATALOGUE_TRANSPORT_FAILURE",
                        "fixture",
                        serde_json::Map::new(),
                    )
                    .unwrap(),
                ),
                "网络",
                true,
            ),
        ] {
            let typed = task_error(error);
            assert!(typed.message_zh.contains(fragment));
            assert_eq!(typed.retryable, retryable);
        }
    }
}

#[cfg(test)]
mod storage_probe_tests {
    use super::{probe_storage_root_with, ProbeCleanup, PROBE_CLEANUP};
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU32, Ordering};

    fn temp_root(label: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("storage-probe-{label}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn probes(dir: &Path) -> Vec<PathBuf> {
        fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(".eostage-write-probe-")
            })
            .collect()
    }

    fn denied(_: &Path) -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::PermissionDenied))
    }

    #[test]
    fn a_file_probe_that_cannot_be_removed_refuses_the_root() {
        let root = temp_root("file");
        let error = probe_storage_root_with(
            &root,
            ProbeCleanup {
                remove_file: denied,
                ..PROBE_CLEANUP
            },
        )
        .expect_err("an undeletable probe refuses the root");
        assert_eq!(error.code, "STORAGE_ROOT_NOT_WRITABLE");
        assert!(!error.retryable);
        // Windows opens the probe delete-on-close, so even this path leaves nothing.
        #[cfg(windows)]
        assert!(probes(&root).is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_directory_probe_that_cannot_be_removed_refuses_the_root() {
        let root = temp_root("dir");
        let error = probe_storage_root_with(
            &root.join("missing").join("downloads"),
            ProbeCleanup {
                remove_dir: denied,
                ..PROBE_CLEANUP
            },
        )
        .expect_err("an undeletable probe refuses the root");
        assert_eq!(error.code, "STORAGE_ROOT_NOT_WRITABLE");
        assert_eq!(probes(&root).len(), 1, "the failing removal left the probe");
        let _ = fs::remove_dir_all(&root);
    }

    static TRANSIENT_CALLS: AtomicU32 = AtomicU32::new(0);

    fn removed_on_third_attempt(path: &Path) -> io::Result<()> {
        if TRANSIENT_CALLS.fetch_add(1, Ordering::SeqCst) < 2 {
            Err(io::Error::from(io::ErrorKind::PermissionDenied))
        } else {
            fs::remove_dir(path)
        }
    }

    #[test]
    fn a_transient_removal_failure_is_retried_within_the_bound() {
        let root = temp_root("transient");
        probe_storage_root_with(
            &root.join("missing"),
            ProbeCleanup {
                remove_dir: removed_on_third_attempt,
                ..PROBE_CLEANUP
            },
        )
        .expect("the third attempt removes the probe");
        assert_eq!(TRANSIENT_CALLS.load(Ordering::SeqCst), 3);
        assert!(probes(&root).is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn the_real_cleanup_leaves_no_file_or_directory_probe() {
        let root = temp_root("real");
        probe_storage_root_with(&root, PROBE_CLEANUP).expect("writable root");
        probe_storage_root_with(&root.join("missing"), PROBE_CLEANUP).expect("creatable root");
        assert!(probes(&root).is_empty());
        assert!(!root.join("missing").exists());
        let _ = fs::remove_dir_all(&root);
    }
}

#[cfg(test)]
mod all_match_tests {
    use super::*;
    use crate::{ApplicationService, ApplicationServiceConfig};
    use insar_core::{
        AsfRemoteSearchRequest, AsfRemoteSearchResponse, DownloadItemProgress, DownloadItemResult,
        DownloadOutcome, FrozenSearchStats, ProviderResult, SearchWorkbenchState,
        StoreSearchBatchRequest,
    };
    use insar_download::{DownloadWork, DownloadWorker, WorkerControl};
    use insar_service::LiveAsfSearch;
    use serde_json::{json, Map, Value};
    use std::sync::Arc;

    struct EmptySearch;
    impl LiveAsfSearch for EmptySearch {
        fn search(
            &self,
            _: &AsfRemoteSearchRequest,
            _: &(dyn Fn() -> bool + Sync),
        ) -> ProviderResult<AsfRemoteSearchResponse> {
            unreachable!()
        }
    }

    struct HoldingWorker;
    impl DownloadWorker for HoldingWorker {
        fn execute(
            &self,
            work: DownloadWork,
            control: WorkerControl,
            _: &mut dyn FnMut(DownloadItemProgress),
        ) -> DownloadItemResult {
            while !control.is_cancelled() {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            DownloadItemResult {
                task_id: work.task.task_id,
                item_id: work.item.item_id,
                outcome: DownloadOutcome::Cancelled,
                destination: work.task.output_root,
                bytes_written: 0,
                attempt_count: 1,
                failure: None,
                finished_at: "2026-11-05T00:00:00Z".to_owned(),
            }
        }
    }

    struct AllowPreflight;
    impl TaskAdmissionPreflight for AllowPreflight {
        fn check(
            &self,
            _batch: &FrozenSearchBatch,
            _selected_items: &[SnapshotItem],
            _item_ids: &[String],
        ) -> Result<(), TypedError> {
            Ok(())
        }
    }

    fn test_service(path: &Path) -> ApplicationService {
        ApplicationService::with_fixture_business_runtime(
            ApplicationServiceConfig::business_store(path),
            Arc::new(EmptySearch),
            Arc::new(HoldingWorker),
        )
    }

    fn candidate_item(
        index: usize,
        orbit_dir: &str,
        size: Option<u64>,
        search_only: bool,
    ) -> SnapshotItem {
        let mut metadata = Map::from_iter([
            (
                "flight_direction".to_owned(),
                Value::String(orbit_dir.to_owned()),
            ),
            (
                "platform".to_owned(),
                Value::String("Sentinel-1A".to_owned()),
            ),
            (
                "product_type".to_owned(),
                Value::String("GRD_HD".to_owned()),
            ),
            ("beam_mode".to_owned(), Value::String("IW".to_owned())),
            ("polarization".to_owned(), Value::String("VV+VH".to_owned())),
            ("relative_orbit".to_owned(), json!(128)),
            ("frame".to_owned(), json!(498)),
        ]);
        if search_only {
            metadata.insert(
                "delivery_policy".to_owned(),
                Value::String("search_only".to_owned()),
            );
        }
        let mut payload = Map::from_iter([(
            "url".to_owned(),
            Value::String(format!("https://example.invalid/product-{index}.zip")),
        )]);
        if let Some(bytes) = size {
            payload.insert("file_size_remote".to_owned(), json!(bytes));
        }
        SnapshotItem {
            item_id: format!("product-{index}"),
            provider_id: "asf.sentinel1".to_owned(),
            display_name: format!("S1A_IW_GRDH_{index:04}.zip"),
            acquisition_datetime: Some("2026-09-01T00:00:00Z".to_owned()),
            footprint_geojson: None,
            metadata,
            payload,
            asset_access: None,
            execution_policy: None,
        }
    }

    #[test]
    fn all_match_preview_and_admission_accounts_for_320_candidates_exclusions_active_and_rejected()
    {
        let root = std::env::temp_dir().join(format!("all-match-320-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let app = test_service(&root.join("tasks.sqlite3"));

        app.mutate_business_store(|store| {
            // 320 candidates:
            // - 0..310: ascending (index 0, 1 will be in an active task; index 2, 3 are search_only; index 4 will be excluded by user)
            // - 310..320: descending (filtered out by ascending predicate)
            let candidates: Vec<SnapshotItem> = (0..320)
                .map(|idx| {
                    let dir = if idx < 310 { "ASCENDING" } else { "DESCENDING" };
                    let size = if idx % 10 == 0 { None } else { Some(1_000_000) };
                    let search_only = idx == 2 || idx == 3;
                    candidate_item(idx, dir, size, search_only)
                })
                .collect();
            store.save_search_batch(&StoreSearchBatchRequest {
                batch: FrozenSearchBatch {
                    schema_version: 1,
                    batch_id: "batch-320".to_owned(),
                    provider_id: "asf.sentinel1".to_owned(),
                    created_at: "2026-11-05T00:00:00Z".to_owned(),
                    aoi_name: "Test AOI".to_owned(),
                    aoi_geojson: None,
                    query: Map::new(),
                    search_stats: None,
                    candidates,
                },
                workbench: SearchWorkbenchState {
                    batch_id: "batch-320".to_owned(),
                    checked_item_ids: Vec::new(),
                    page_index: 0,
                    page_size: 50,
                    revision: 0,
                },
                activate_workbench: true,
            })?;
            // Active task covering product-0 and product-1
            store.create_task(&StoreCreateTaskRequest {
                task_id: "existing-active-task".to_owned(),
                batch_id: "batch-320".to_owned(),
                selected_item_ids: vec!["product-0".to_owned(), "product-1".to_owned()],
                checked_item_ids: vec!["product-0".to_owned(), "product-1".to_owned()],
                page_index: 0,
                page_size: 50,
                output_root: root.join("existing"),
                layout: DownloadLayout::Direct,
                concurrency: 1,
                created_at: "2026-11-05T00:00:01Z".to_owned(),
            })?;
            Ok(())
        })
        .unwrap();

        let predicate = BatchSelectionPredicate {
            orbit_direction: Some("ascending".to_owned()),
            ..BatchSelectionPredicate::default()
        };
        let exclusions = vec!["product-4".to_owned()];

        let preview = app
            .preview_all_match_task(BatchAllMatchPreviewRequest {
                batch_id: "batch-320".to_owned(),
                revision: 0,
                predicate_version: ALL_MATCH_PREDICATE_VERSION_V1,
                predicate: predicate.clone(),
                exclusions: exclusions.clone(),
            })
            .expect("preview succeeds across 320 candidates");

        // 310 ascending - 1 excluded (product-4) = 309 matched
        // Out of 309 matched:
        // - 2 rejected (product-2, product-3)
        // - 2 skipped active (product-0, product-1)
        // - 305 admissible (product-5 .. product-309)
        assert_eq!(preview.matched_count, 309);
        assert_eq!(preview.rejected_count, 2);
        assert_eq!(preview.skipped_active_count, 2);
        assert_eq!(preview.admissible_count, 305);
        assert_eq!(
            preview.admissible_count + preview.skipped_active_count + preview.rejected_count,
            preview.matched_count
        );
        assert_eq!(preview.orbit_directions, vec!["ascending".to_owned()]);
        // In 5..310 (305 items), multiples of 10 are 10, 20, ..., 300 -> 30 items have None size; 275 items have 1_000_000 bytes.
        assert_eq!(preview.unknown_size_count, 30);
        assert_eq!(preview.known_bytes, 275 * 1_000_000);

        let outcome = app
            .create_all_match_task_with_preflight(
                BatchAllMatchCreateTaskRequest {
                    batch_id: "batch-320".to_owned(),
                    revision: 0,
                    predicate_version: ALL_MATCH_PREDICATE_VERSION_V1,
                    predicate,
                    exclusions,
                    expected_membership_digest: preview.membership_digest.clone(),
                    options: UnifiedCreateTaskOptions {
                        output_root: root.join("downloads"),
                        layout: DownloadLayout::Direct,
                        concurrency: 2,
                        include_precise_orbit: Some(false),
                    },
                },
                &AllowPreflight,
            )
            .expect("all-match admission succeeds for >250 items");

        assert!(outcome.task.created);
        assert_eq!(outcome.matched_count, 309);
        assert_eq!(outcome.created_count, 305);
        assert_eq!(outcome.pending_count, 305);
        assert_eq!(outcome.skipped_count, 2);
        assert_eq!(outcome.rejected_count, 2);
        assert_eq!(
            outcome.created_count + outcome.skipped_count + outcome.rejected_count,
            outcome.matched_count
        );

        app.with_business_runtime(|runtime| {
            let store = runtime.store.lock().unwrap();
            let created_task = store.load_task(&outcome.task.task_id)?.unwrap();
            assert_eq!(created_task.runtime.selected_item_ids.len(), 305);
            assert!(!created_task
                .runtime
                .selected_item_ids
                .contains(&"product-0".to_owned()));
            assert!(!created_task
                .runtime
                .selected_item_ids
                .contains(&"product-2".to_owned()));
            assert!(!created_task
                .runtime
                .selected_item_ids
                .contains(&"product-4".to_owned()));
            assert!(created_task
                .runtime
                .selected_item_ids
                .contains(&"product-309".to_owned()));
            Ok(())
        })
        .unwrap();

        let _ = app.shutdown();
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn all_match_refuses_over_ceiling_and_stale_digest() {
        let root = std::env::temp_dir().join(format!("all-match-guards-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let app = test_service(&root.join("tasks.sqlite3"));

        app.mutate_business_store(|store| {
            store.save_search_batch(&StoreSearchBatchRequest {
                batch: FrozenSearchBatch {
                    schema_version: 1,
                    batch_id: "batch-over-cap".to_owned(),
                    provider_id: "asf.sentinel1".to_owned(),
                    created_at: "2026-11-05T00:00:00Z".to_owned(),
                    aoi_name: "Test AOI".to_owned(),
                    aoi_geojson: None,
                    query: Map::new(),
                    search_stats: Some(FrozenSearchStats {
                        matched_count: Some(10_001),
                        fetched_record_count: Some(2),
                        mapped_record_count: Some(2),
                        provider_returned_count: 2,
                        frozen_candidate_count: 2,
                        truncated: true,
                        source: "ASF".to_owned(),
                        source_attempts: Vec::new(),
                    }),
                    candidates: vec![
                        candidate_item(0, "ASCENDING", Some(100), false),
                        candidate_item(1, "ASCENDING", Some(100), false),
                    ],
                },
                workbench: SearchWorkbenchState {
                    batch_id: "batch-over-cap".to_owned(),
                    checked_item_ids: Vec::new(),
                    page_index: 0,
                    page_size: 50,
                    revision: 0,
                },
                activate_workbench: true,
            })?;
            store.save_search_batch(&StoreSearchBatchRequest {
                batch: FrozenSearchBatch {
                    schema_version: 1,
                    batch_id: "batch-normal".to_owned(),
                    provider_id: "asf.sentinel1".to_owned(),
                    created_at: "2026-11-05T00:00:00Z".to_owned(),
                    aoi_name: "Test AOI".to_owned(),
                    aoi_geojson: None,
                    query: Map::new(),
                    search_stats: None,
                    candidates: vec![
                        candidate_item(0, "ASCENDING", Some(100), false),
                        candidate_item(1, "ASCENDING", Some(100), false),
                    ],
                },
                workbench: SearchWorkbenchState {
                    batch_id: "batch-normal".to_owned(),
                    checked_item_ids: Vec::new(),
                    page_index: 0,
                    page_size: 50,
                    revision: 0,
                },
                activate_workbench: true,
            })?;
            Ok(())
        })
        .unwrap();

        let over_err = app
            .preview_all_match_task(BatchAllMatchPreviewRequest {
                batch_id: "batch-over-cap".to_owned(),
                revision: 0,
                predicate_version: ALL_MATCH_PREDICATE_VERSION_V1,
                predicate: BatchSelectionPredicate::default(),
                exclusions: Vec::new(),
            })
            .expect_err("over 10,000 matched count must be refused");
        assert_eq!(over_err.code, "V3_TASK_OVER_CEILING");

        let stale_err = app
            .create_all_match_task_with_preflight(
                BatchAllMatchCreateTaskRequest {
                    batch_id: "batch-normal".to_owned(),
                    revision: 0,
                    predicate_version: ALL_MATCH_PREDICATE_VERSION_V1,
                    predicate: BatchSelectionPredicate::default(),
                    exclusions: Vec::new(),
                    expected_membership_digest: "stale-digest".to_owned(),
                    options: UnifiedCreateTaskOptions {
                        output_root: root.join("downloads"),
                        layout: DownloadLayout::Direct,
                        concurrency: 2,
                        include_precise_orbit: Some(false),
                    },
                },
                &AllowPreflight,
            )
            .expect_err("mismatched digest must be refused");
        assert_eq!(stale_err.code, "V3_TASK_SELECTION_CHANGED");

        let _ = app.shutdown();
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn refuse_resume_on_low_disk_allows_unknown_task_and_fails_closed_on_store_error() {
        let probe = default_free_space_probe();
        let no_store = ApplicationService::new(ApplicationServiceConfig::liveness_only());
        let store_err = no_store
            .refuse_resume_on_low_disk("task-missing", probe.as_ref())
            .expect_err("unavailable store must fail closed rather than bypass disk check");
        assert!(matches!(
            store_err,
            ApplicationServiceError::StorePathRequired | ApplicationServiceError::StoreNotReady
        ));

        let root = std::env::temp_dir().join(format!(
            "insar-disk-resume-unknown-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let db_path = root.join("tasks.sqlite3");
        let _store = insar_store::TaskStore::open(&db_path).unwrap();
        let app = ApplicationService::new(ApplicationServiceConfig::business_store(&db_path));
        app.refuse_resume_on_low_disk("task-does-not-exist", probe.as_ref())
            .expect("missing task must return Ok(()) so caller reaches canonical NotFound");
        let conn2 = rusqlite::Connection::open(&db_path).unwrap();
        conn2
            .execute_batch("PRAGMA foreign_keys = OFF; DROP TABLE task_tombstones;")
            .unwrap();
        drop(conn2);
        let db_err = app
            .refuse_resume_on_low_disk("task-does-not-exist", probe.as_ref())
            .expect_err(
                "SQLite error on task_tombstones table must propagate instead of bypassing disk check",
            );
        assert!(matches!(db_err, ApplicationServiceError::Store(_)));
        let _ = app.shutdown();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn task_delete_history_still_rejects_normal_paused_download_task() {
        let root =
            std::env::temp_dir().join(format!("normal-paused-delete-guard-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let app = test_service(&root.join("tasks.sqlite3"));

        app.mutate_business_store(|store| {
            store.save_search_batch(&StoreSearchBatchRequest {
                batch: FrozenSearchBatch {
                    schema_version: 1,
                    batch_id: "batch-normal-paused".to_owned(),
                    provider_id: "asf.sentinel1".to_owned(),
                    created_at: "2026-11-22T00:00:00Z".to_owned(),
                    aoi_name: "Test AOI".to_owned(),
                    aoi_geojson: None,
                    query: Map::new(),
                    search_stats: None,
                    candidates: vec![candidate_item(0, "ASCENDING", Some(100), false)],
                },
                workbench: SearchWorkbenchState {
                    batch_id: "batch-normal-paused".to_owned(),
                    checked_item_ids: Vec::new(),
                    page_index: 0,
                    page_size: 50,
                    revision: 0,
                },
                activate_workbench: true,
            })?;
            Ok(())
        })
        .unwrap();

        let preview = app
            .preview_all_match_task(BatchAllMatchPreviewRequest {
                batch_id: "batch-normal-paused".to_owned(),
                revision: 0,
                predicate_version: ALL_MATCH_PREDICATE_VERSION_V1,
                predicate: BatchSelectionPredicate::default(),
                exclusions: Vec::new(),
            })
            .unwrap();
        let created = app
            .create_all_match_task_with_preflight(
                BatchAllMatchCreateTaskRequest {
                    batch_id: "batch-normal-paused".to_owned(),
                    revision: 0,
                    predicate_version: ALL_MATCH_PREDICATE_VERSION_V1,
                    predicate: BatchSelectionPredicate::default(),
                    exclusions: Vec::new(),
                    expected_membership_digest: preview.membership_digest,
                    options: UnifiedCreateTaskOptions {
                        output_root: root.join("downloads"),
                        layout: DownloadLayout::Direct,
                        concurrency: 2,
                        include_precise_orbit: Some(false),
                    },
                },
                &AllowPreflight,
            )
            .unwrap();
        let _ = app
            .task_action(
                created.task.task_id.clone(),
                created.task.revision,
                UnifiedTaskAction::Pause,
            )
            .unwrap();

        let err = app
            .task_delete_history(created.task.task_id)
            .expect_err("normal paused download task must still reject task_delete_history");
        assert_eq!(err.code, "TASK_DELETE_HISTORY_ACTIVE");

        let _ = app.shutdown();
        let _ = fs::remove_dir_all(&root);
    }
}
