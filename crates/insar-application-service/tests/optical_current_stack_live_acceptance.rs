//! Explicit, opt-in acceptance for the current anonymous Landsat/Planetary
//! Computer route.
//!
//! The test intentionally exercises the production `ApplicationService` route:
//! public PC catalogue search, the shared frozen Preview command, Workbench
//! selection, the derived 22-file L2SP task, the existing scheduler, and Store
//! history/reopen.  It never accepts a locator, token, or credential from the
//! test environment.  A caller-designated root is required and is preserved so
//! a stopped run leaves its evidence and any partial output available for
//! inspection.

use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use insar_application_service::{
    ApplicationCatalogSearchAndFreezeRequest, ApplicationService, ApplicationServiceConfig,
    LandsatCreateBundleTaskRequest, PreviewAssetRequest, StoreTaskHistoryPageRequest,
};
use insar_core::{
    CatalogSearchQuery, DownloadEngineSnapshot, DownloadItemStatus, DownloadLayout,
    DownloadOutcome, PreviewAssetRole, SearchWorkbenchState, StoreArchiveTaskRequest,
    StoreTaskCommand, StoreTaskPageRequest, StoreTaskSourceBatchPageRequest,
    StoreUpdateTaskRequest, StoreUpdateWorkbenchRequest, TaskLifecycleStatus, TaskRuntimeSnapshot,
    TaskStateCommandKind,
};
use insar_download::part_path;
use insar_store::{StoreWorkbenchPageRequest, TaskStore};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

const LIVE_ROOT_ENV: &str = "INSAR_LANDSAT_PC_CURRENT_STACK_LIVE_ROOT";
const PRESERVED_ROOT_ENV: &str = "INSAR_LANDSAT_PC_PRESERVED_LIVE_ROOT";
const PRESERVED_RECEIPT_ENV: &str = "INSAR_LANDSAT_PC_PRESERVED_LIVE_RECEIPT";
const SOURCE_BATCH_ID: &str = "landsat-pc-current-stack-source";
const TASK_ID: &str = "landsat-pc-current-stack-task";
const ASSET_TASK_BATCH_ID: &str = "landsat-assets-landsat-pc-current-stack-task";
const ENTITY_ID: &str = "LC90130352023085LGN00";
const DISPLAY_ID: &str = "LC09_L2SP_013035_20230326_20230328_02_T1";
const DATASET_ALIAS: &str = "landsat_ot_c2_l2";
const EXPECTED_ASSET_COUNT: usize = 22;
const MAX_CANDIDATES: u64 = 10;
const MAX_TOTAL_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const SEARCH_WAIT: Duration = Duration::from_secs(5 * 60);
const TRANSFER_WAIT: Duration = Duration::from_secs(45 * 60);
const STATE_WAIT: Duration = Duration::from_secs(45);
const PAUSE_STABILITY_WAIT: Duration = Duration::from_secs(2);
const POLL_INTERVAL: Duration = Duration::from_millis(250);
const HASH_BUFFER_BYTES: usize = 1024 * 1024;

#[test]
#[ignore = "requires explicit opt-in and an empty caller-designated root for a public 22-file transfer"]
fn optical_current_stack_planetary_computer_live_acceptance() {
    match run_live() {
        Ok(report) => {
            eprintln!("LANDSAT_PC_CURRENT_STACK_LIVE outcome=completed");
            eprintln!("candidate_count={}", report.candidate_count);
            eprintln!("asset_count={}", report.asset_count);
            eprintln!("expected_bytes={}", report.expected_bytes);
            eprintln!("actual_bytes={}", report.actual_bytes);
            eprintln!("active_observed={}", report.active_observed);
            eprintln!("part_observed={}", report.part_observed);
            eprintln!("paused_bytes={}", report.paused_bytes);
            eprintln!("resumed_bytes={}", report.resumed_bytes);
            eprintln!("pause_revision={}", report.pause_revision);
            eprintln!("resume_revision={}", report.resume_revision);
            eprintln!("history_revision={}", report.history_revision);
            eprintln!("hashes_verified=true");
            eprintln!("store_history_reopen_verified=true");
            eprintln!("acceptance_workspace_preserved=true");
        }
        Err(code) => {
            eprintln!("LANDSAT_PC_CURRENT_STACK_LIVE outcome=stopped");
            eprintln!("code={code}");
            panic!("LANDSAT_PC_CURRENT_STACK_LIVE_STOPPED");
        }
    }
}

/// Read-only follow-up for an already preserved live run. This intentionally
/// stays separate from the transfer acceptance so a history-contract failure
/// never causes an 819 MB re-download or rewrites the original evidence.
#[test]
#[ignore = "requires an existing preserved live root and a new receipt path"]
fn optical_current_stack_preserved_history_receipt() {
    match verify_preserved_history_receipt() {
        Ok(path) => eprintln!(
            "LANDSAT_PC_PRESERVED_HISTORY_RECEIPT outcome=passed receipt={}",
            path.display()
        ),
        Err(code) => panic!("LANDSAT_PC_PRESERVED_HISTORY_RECEIPT_STOPPED:{code}"),
    }
}

struct LiveReport {
    candidate_count: usize,
    asset_count: usize,
    expected_bytes: u64,
    actual_bytes: u64,
    active_observed: bool,
    part_observed: bool,
    paused_bytes: u64,
    resumed_bytes: u64,
    pause_revision: u64,
    resume_revision: u64,
    history_revision: u64,
}

/// The root is deliberately caller-owned and is never removed by this harness.
/// Requiring it to be absent prevents a prior run, unrelated files, or a stale
/// SQLite database from being mistaken for current acceptance evidence.
struct AcceptanceWorkspace {
    output_root: PathBuf,
    database: PathBuf,
    evidence: PathBuf,
}

impl AcceptanceWorkspace {
    fn create(root: PathBuf) -> Result<Self, String> {
        if root.as_os_str().is_empty() || !root.is_absolute() {
            return Err("LANDSAT_PC_CURRENT_STACK_ROOT_MUST_BE_ABSOLUTE".to_owned());
        }
        if root.exists() {
            return Err("LANDSAT_PC_CURRENT_STACK_ROOT_MUST_BE_ABSENT".to_owned());
        }
        let output_root = root.join("output");
        fs::create_dir_all(&output_root)
            .map_err(|_| "LANDSAT_PC_CURRENT_STACK_ROOT_UNAVAILABLE".to_owned())?;
        Ok(Self {
            database: root.join("tasks.sqlite3"),
            evidence: root.join("acceptance-evidence.json"),
            output_root,
        })
    }
}

/// Ensures the production scheduler and Store lease are stopped during every
/// early return or panic.  The live harness intentionally keeps its files, but
/// it must not leave a worker thread running against them.
struct ServiceShutdownGuard<'a> {
    service: &'a ApplicationService,
    armed: bool,
}

impl<'a> ServiceShutdownGuard<'a> {
    fn new(service: &'a ApplicationService) -> Self {
        Self {
            service,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ServiceShutdownGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.service.shutdown();
        }
    }
}

/// Small redacted evidence journal.  Each completed phase is flushed so a
/// transport failure, panic, or interrupted test still leaves the last known
/// Store revision and lifecycle state in the caller-owned root.
struct EvidenceJournal {
    path: PathBuf,
    document: Map<String, Value>,
    sequence: u64,
    finalized: bool,
}

impl EvidenceJournal {
    fn new(workspace: &AcceptanceWorkspace) -> Result<Self, String> {
        let document = Map::from_iter([
            ("schema_version".to_owned(), json!(1)),
            ("outcome".to_owned(), json!("started")),
            (
                "identity".to_owned(),
                json!({
                    "provider": "planetary_computer",
                    "logical_dataset": "optical.landsat.c2.l2",
                    "native_collection_id": DATASET_ALIAS,
                    "entity_id": ENTITY_ID,
                    "display_id": DISPLAY_ID,
                    "source_batch_id": SOURCE_BATCH_ID,
                    "task_id": TASK_ID
                }),
            ),
            ("events".to_owned(), Value::Array(Vec::new())),
        ]);
        let journal = Self {
            path: workspace.evidence.clone(),
            document,
            sequence: 0,
            finalized: false,
        };
        journal.flush()?;
        Ok(journal)
    }

    fn record(&mut self, label: &str, details: Value) -> Result<(), String> {
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or_else(|| "LANDSAT_PC_CURRENT_STACK_EVIDENCE_SEQUENCE_OVERFLOW".to_owned())?;
        let events = self
            .document
            .get_mut("events")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| "LANDSAT_PC_CURRENT_STACK_EVIDENCE_INVALID".to_owned())?;
        events.push(json!({
            "sequence": self.sequence,
            "label": label,
            "details": details
        }));
        self.flush()
    }

    fn finish(&mut self, outcome: &str, code: Option<&str>) -> Result<(), String> {
        self.document
            .insert("outcome".to_owned(), Value::String(outcome.to_owned()));
        if let Some(code) = code {
            self.document
                .insert("code".to_owned(), Value::String(code.to_owned()));
        } else {
            self.document.remove("code");
        }
        let result = self.flush();
        if result.is_ok() {
            self.finalized = true;
        }
        result
    }

    fn flush(&self) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(&self.document)
            .map_err(|_| "LANDSAT_PC_CURRENT_STACK_EVIDENCE_SERIALIZE_FAILED".to_owned())?;
        fs::write(&self.path, bytes)
            .map_err(|_| "LANDSAT_PC_CURRENT_STACK_EVIDENCE_WRITE_FAILED".to_owned())
    }
}

impl Drop for EvidenceJournal {
    fn drop(&mut self) {
        if !self.finalized {
            self.document.insert(
                "outcome".to_owned(),
                Value::String("interrupted".to_owned()),
            );
            self.document.insert(
                "code".to_owned(),
                Value::String("LANDSAT_PC_CURRENT_STACK_INTERRUPTED".to_owned()),
            );
        }
        let _ = self.flush();
    }
}

#[derive(Clone, Debug)]
struct FrozenAsset {
    item_id: String,
    file_name: String,
    expected_size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct HashedAsset {
    item_id: String,
    file_name: String,
    expected_size: u64,
    actual_size: u64,
    sha256: String,
}

/// Filesystem evidence is limited to the frozen asset names.  It records both
/// resumable `.part` lengths and the set/lengths of promoted final files.
#[derive(Clone, Debug, PartialEq, Eq)]
struct FileProgressSnapshot {
    part_sizes: Vec<(String, u64)>,
    final_sizes: Vec<(String, u64)>,
}

#[derive(Clone, Debug)]
struct PreservedEvidence {
    candidate_count: u64,
    expected_bytes: u64,
    archive_revision: u64,
    hashes: Vec<HashedAsset>,
    original_outcome: String,
    original_code: String,
    paused_bytes: u64,
    resumed_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileFingerprint {
    size_bytes: u64,
    sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SourceDatabaseSnapshot {
    database: FileFingerprint,
    wal: Option<FileFingerprint>,
    shm: Option<FileFingerprint>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CopySidecarSnapshot {
    wal: Option<FileFingerprint>,
    shm: Option<FileFingerprint>,
}

fn verify_preserved_history_receipt() -> Result<PathBuf, String> {
    let root = std::env::var_os(PRESERVED_ROOT_ENV)
        .map(PathBuf::from)
        .ok_or_else(|| "LANDSAT_PC_PRESERVED_ROOT_REQUIRED".to_owned())?;
    if !root.is_absolute() || !root.is_dir() {
        return Err("LANDSAT_PC_PRESERVED_ROOT_INVALID".to_owned());
    }
    let receipt_path = std::env::var_os(PRESERVED_RECEIPT_ENV)
        .map(PathBuf::from)
        .ok_or_else(|| "LANDSAT_PC_PRESERVED_RECEIPT_REQUIRED".to_owned())?;
    if !receipt_path.is_absolute() {
        return Err("LANDSAT_PC_PRESERVED_RECEIPT_MUST_BE_ABSOLUTE".to_owned());
    }
    if fs::symlink_metadata(&receipt_path).is_ok() {
        return Err("LANDSAT_PC_PRESERVED_RECEIPT_MUST_BE_NEW".to_owned());
    }
    if !receipt_path.parent().is_some_and(|parent| parent.is_dir()) {
        return Err("LANDSAT_PC_PRESERVED_RECEIPT_PARENT_INVALID".to_owned());
    }

    let database = root.join("tasks.sqlite3");
    let evidence_path = root.join("acceptance-evidence.json");
    let output_root = root.join("output");
    if !evidence_path.is_file() || !output_root.is_dir() {
        return Err("LANDSAT_PC_PRESERVED_ARTIFACTS_MISSING".to_owned());
    }
    if receipt_path == database
        || receipt_path == evidence_path
        || receipt_path == output_root
        || receipt_path.starts_with(&output_root)
    {
        return Err("LANDSAT_PC_PRESERVED_RECEIPT_OVERLAPS_SOURCE".to_owned());
    }

    let verification_database = receipt_path.with_extension("sqlite3");
    if verification_database == receipt_path
        || verification_database == database
        || verification_database == evidence_path
        || verification_database == output_root
        || verification_database.starts_with(&output_root)
    {
        return Err("LANDSAT_PC_PRESERVED_COPY_OVERLAPS_SOURCE".to_owned());
    }
    if fs::symlink_metadata(&verification_database).is_ok() {
        return Err("LANDSAT_PC_PRESERVED_COPY_MUST_BE_NEW".to_owned());
    }

    let evidence = read_preserved_evidence(&evidence_path)?;
    let source_before_copy = source_database_snapshot(&root)?;
    let verification_fingerprint = create_database_copy_new(
        &database,
        &verification_database,
        &source_before_copy.database,
    )?;
    let source_after_copy = source_database_snapshot(&root)?;
    if source_after_copy != source_before_copy {
        return Err("LANDSAT_PC_PRESERVED_SOURCE_CHANGED_AFTER_COPY".to_owned());
    }
    if verification_fingerprint != source_before_copy.database {
        return Err("LANDSAT_PC_PRESERVED_COPY_HASH_MISMATCH".to_owned());
    }
    ensure_copy_has_no_sidecars(&verification_database)?;

    let actual_hashes = hash_outputs_again(&output_root, &evidence.hashes)?;
    if actual_hashes != evidence.hashes {
        return Err("LANDSAT_PC_PRESERVED_OUTPUT_HASH_MISMATCH".to_owned());
    }
    let actual_bytes = actual_hashes
        .iter()
        .map(|asset| asset.actual_size)
        .try_fold(0_u64, u64::checked_add)
        .ok_or_else(|| "LANDSAT_PC_PRESERVED_SIZE_OVERFLOW".to_owned())?;
    if actual_bytes != evidence.expected_bytes {
        return Err("LANDSAT_PC_PRESERVED_OUTPUT_TOTAL_MISMATCH".to_owned());
    }

    let store = TaskStore::open_verification_read_only(&verification_database)
        .map_err(|error| format!("LANDSAT_PC_PRESERVED_STORE_{}", error.code()))?;
    let copy_sidecars_after_open = copy_sidecar_snapshot(&verification_database)?;
    let source_after_open = source_database_snapshot(&root)?;
    if source_after_open != source_before_copy {
        return Err("LANDSAT_PC_PRESERVED_SOURCE_CHANGED_AFTER_READ_ONLY_OPEN".to_owned());
    }
    let copy_after_open =
        required_file_fingerprint(&verification_database, "LANDSAT_PC_PRESERVED_COPY_INVALID")?;
    if copy_after_open != verification_fingerprint {
        return Err("LANDSAT_PC_PRESERVED_COPY_CHANGED_AFTER_READ_ONLY_OPEN".to_owned());
    }
    let restored = store
        .core_restore(true)
        .map_err(|error| format!("LANDSAT_PC_PRESERVED_CORE_RESTORE_{}", error.code()))?;
    let archived = restored
        .tasks
        .iter()
        .filter(|task| task.task_id == TASK_ID)
        .collect::<Vec<_>>();
    if archived.len() != 1 {
        return Err("LANDSAT_PC_PRESERVED_ARCHIVED_TASK_COUNT_INVALID".to_owned());
    }
    let archived = archived[0];
    if archived.batch_id != ASSET_TASK_BATCH_ID
        || archived.status != TaskLifecycleStatus::Completed
        || archived.selected_count != EXPECTED_ASSET_COUNT
        || archived.completed_count != EXPECTED_ASSET_COUNT
        || archived.revision != evidence.archive_revision
        || archived.total_bytes != Some(evidence.expected_bytes)
        || archived.transferred_bytes != evidence.expected_bytes
    {
        return Err("LANDSAT_PC_PRESERVED_RUNTIME_SUMMARY_INVALID".to_owned());
    }

    let active_restore = store
        .core_restore(false)
        .map_err(|error| format!("LANDSAT_PC_PRESERVED_ACTIVE_RESTORE_{}", error.code()))?;
    if active_restore
        .tasks
        .iter()
        .any(|task| task.task_id == TASK_ID)
    {
        return Err("LANDSAT_PC_PRESERVED_ARCHIVED_TASK_STILL_ACTIVE".to_owned());
    }

    let workbench = restored
        .workbench
        .as_ref()
        .ok_or_else(|| "LANDSAT_PC_PRESERVED_WORKBENCH_MISSING".to_owned())?;
    let candidate_count = usize::try_from(evidence.candidate_count)
        .map_err(|_| "LANDSAT_PC_PRESERVED_CANDIDATE_COUNT_INVALID".to_owned())?;
    if workbench.batch_id != SOURCE_BATCH_ID
        || workbench.candidate_count != candidate_count
        || workbench.checked_count != 1
    {
        return Err("LANDSAT_PC_PRESERVED_WORKBENCH_SUMMARY_INVALID".to_owned());
    }
    let workbench_page = store
        .load_workbench_page(&StoreWorkbenchPageRequest {
            batch_id: SOURCE_BATCH_ID.to_owned(),
            expected_revision: workbench.revision,
            page_index: 0,
            page_size: MAX_CANDIDATES,
        })
        .map_err(|error| format!("LANDSAT_PC_PRESERVED_WORKBENCH_PAGE_{}", error.code()))?;
    if workbench_page.batch_id != SOURCE_BATCH_ID
        || workbench_page.total_count != candidate_count
        || workbench_page.items.len() != candidate_count
        || workbench_page.checked_count != 1
    {
        return Err("LANDSAT_PC_PRESERVED_WORKBENCH_PAGE_INVALID".to_owned());
    }

    let checked_workbench_items = workbench_page
        .items
        .iter()
        .filter(|item| item.checked)
        .collect::<Vec<_>>();
    if checked_workbench_items.len() != 1 {
        return Err("LANDSAT_PC_PRESERVED_WORKBENCH_SELECTION_INVALID".to_owned());
    }
    let source_item = &checked_workbench_items[0].item;
    let source_processing_level = source_item
        .metadata
        .get("properties")
        .and_then(Value::as_object)
        .and_then(|properties| properties.get("processing_level"))
        .and_then(Value::as_str);
    if source_item.display_name != DISPLAY_ID
        || source_item
            .metadata
            .get("provider_product_id")
            .and_then(Value::as_str)
            != Some(ENTITY_ID)
        || source_processing_level != Some("l2sp")
    {
        return Err("LANDSAT_PC_PRESERVED_WORKBENCH_SOURCE_INVALID".to_owned());
    }
    let source_item_id = source_item.item_id.clone();

    let source_page = store
        .load_task_source_batch_page(&StoreTaskSourceBatchPageRequest {
            task_id: TASK_ID.to_owned(),
            page_index: 0,
            page_size: EXPECTED_ASSET_COUNT as u64,
        })
        .map_err(|error| format!("LANDSAT_PC_PRESERVED_SOURCE_BATCH_{}", error.code()))?;
    if source_page.source_batch_id != ASSET_TASK_BATCH_ID
        || source_page.total_count != EXPECTED_ASSET_COUNT as u64
        || source_page.page_size < EXPECTED_ASSET_COUNT as u64
        || source_page.items.len() != EXPECTED_ASSET_COUNT
        || source_page.task_revision != archived.revision
        || source_page.task_status != TaskLifecycleStatus::Completed
    {
        return Err("LANDSAT_PC_PRESERVED_SOURCE_BATCH_INVALID".to_owned());
    }
    let selected_source = source_page
        .items
        .iter()
        .filter(|item| item.belongs_to_task)
        .collect::<Vec<_>>();
    if selected_source.len() != EXPECTED_ASSET_COUNT
        || selected_source
            .iter()
            .any(|item| item.task_status != Some(DownloadItemStatus::Completed))
    {
        return Err("LANDSAT_PC_PRESERVED_SOURCE_MEMBERSHIP_INVALID".to_owned());
    }
    let mut source_asset_ids = BTreeSet::new();
    let mut source_parent_ids = BTreeSet::new();
    let mut source_page_bytes = 0_u64;
    let mut source_page_total_bytes = 0_u64;
    for item in &selected_source {
        if !source_asset_ids.insert(item.item.item_id.as_str()) {
            return Err("LANDSAT_PC_PRESERVED_SOURCE_ASSET_DUPLICATE".to_owned());
        }
        let parent_item_id = item
            .item
            .metadata
            .get("source_item_id")
            .and_then(Value::as_str)
            .ok_or_else(|| "LANDSAT_PC_PRESERVED_SOURCE_ITEM_ID_MISSING".to_owned())?;
        if parent_item_id != source_item_id {
            return Err("LANDSAT_PC_PRESERVED_SOURCE_ITEM_ID_MISMATCH".to_owned());
        }
        source_parent_ids.insert(parent_item_id.to_owned());
        let expected = evidence
            .hashes
            .iter()
            .find(|asset| asset.item_id == item.item.item_id)
            .ok_or_else(|| "LANDSAT_PC_PRESERVED_SOURCE_ASSET_MISSING".to_owned())?;
        if item.bytes_completed != expected.actual_size
            || item.total_bytes != Some(expected.expected_size)
        {
            return Err("LANDSAT_PC_PRESERVED_SOURCE_ASSET_BYTES_INVALID".to_owned());
        }
        source_page_bytes = source_page_bytes
            .checked_add(item.bytes_completed)
            .ok_or_else(|| "LANDSAT_PC_PRESERVED_SOURCE_BYTES_OVERFLOW".to_owned())?;
        source_page_total_bytes = source_page_total_bytes
            .checked_add(expected.expected_size)
            .ok_or_else(|| "LANDSAT_PC_PRESERVED_SOURCE_TOTAL_BYTES_OVERFLOW".to_owned())?;
    }
    if source_asset_ids.len() != EXPECTED_ASSET_COUNT
        || source_parent_ids.len() != 1
        || source_page_bytes != evidence.expected_bytes
        || source_page_total_bytes != evidence.expected_bytes
    {
        return Err("LANDSAT_PC_PRESERVED_SOURCE_MEMBERSHIP_INVALID".to_owned());
    }

    let task_page = store
        .load_task_page(&StoreTaskPageRequest {
            task_id: TASK_ID.to_owned(),
            expected_revision: archived.revision,
            page_index: 0,
            page_size: EXPECTED_ASSET_COUNT as u64,
        })
        .map_err(|error| format!("LANDSAT_PC_PRESERVED_TASK_PAGE_{}", error.code()))?;
    if task_page.task_id != TASK_ID
        || task_page.revision != archived.revision
        || task_page.total_count != EXPECTED_ASSET_COUNT as u64
        || task_page.items.len() != EXPECTED_ASSET_COUNT
    {
        return Err("LANDSAT_PC_PRESERVED_FROZEN_ASSET_PAGE_INVALID".to_owned());
    }
    let mut seen_items = BTreeSet::new();
    for item in &task_page.items {
        if !seen_items.insert(item.item.item_id.as_str()) {
            return Err("LANDSAT_PC_PRESERVED_FROZEN_ASSET_DUPLICATE".to_owned());
        }
        let expected = evidence
            .hashes
            .iter()
            .find(|asset| asset.item_id == item.item.item_id)
            .ok_or_else(|| "LANDSAT_PC_PRESERVED_FROZEN_ASSET_MISSING".to_owned())?;
        let access = item
            .item
            .asset_access
            .as_ref()
            .ok_or_else(|| "LANDSAT_PC_PRESERVED_FROZEN_ACCESS_MISSING".to_owned())?;
        let expected_destination = output_root.join(&expected.file_name);
        if item.status != DownloadItemStatus::Completed
            || item.item.display_name != expected.file_name
            || access.file_name.as_deref() != Some(expected.file_name.as_str())
            || access.expected_size_bytes != Some(expected.expected_size)
            || item.bytes_completed != expected.actual_size
            || item.total_bytes != Some(expected.expected_size)
            || item.destination.as_ref() != Some(&expected_destination)
        {
            return Err("LANDSAT_PC_PRESERVED_FROZEN_ASSET_INVALID".to_owned());
        }
    }
    if seen_items.len() != evidence.hashes.len() {
        return Err("LANDSAT_PC_PRESERVED_FROZEN_ASSET_COUNT_INVALID".to_owned());
    }

    let histories = store
        .list_history()
        .map_err(|error| format!("LANDSAT_PC_PRESERVED_HISTORY_RECORD_{}", error.code()))?;
    let history_record = histories
        .iter()
        .find(|history| history.task.runtime.task_id == TASK_ID)
        .ok_or_else(|| "LANDSAT_PC_PRESERVED_HISTORY_RECORD_MISSING".to_owned())?;
    if history_record.task.snapshot.batch_id != ASSET_TASK_BATCH_ID
        || history_record.task.runtime.status != TaskLifecycleStatus::Completed
        || history_record.task.runtime.revision != evidence.archive_revision
        || history_record.task.runtime.selected_item_ids.len() != EXPECTED_ASSET_COUNT
        || history_record.task.runtime.completed_item_ids.len() != EXPECTED_ASSET_COUNT
    {
        return Err("LANDSAT_PC_PRESERVED_HISTORY_RECORD_INVALID".to_owned());
    }
    let history_page = store
        .load_task_history_page(&StoreTaskHistoryPageRequest {
            page_size: 100,
            after_archived_at_epoch_ms: None,
            after_task_id: None,
        })
        .map_err(|error| format!("LANDSAT_PC_PRESERVED_HISTORY_PAGE_{}", error.code()))?;
    let history_item = history_page
        .items
        .iter()
        .find(|item| item.task.task_id == TASK_ID)
        .ok_or_else(|| "LANDSAT_PC_PRESERVED_HISTORY_PAGE_ITEM_MISSING".to_owned())?;
    if history_item.task.batch_id != ASSET_TASK_BATCH_ID
        || history_item.task.status != TaskLifecycleStatus::Completed
        || history_item.task.selected_count != EXPECTED_ASSET_COUNT
        || history_item.task.completed_count != EXPECTED_ASSET_COUNT
        || history_item.task.revision != evidence.archive_revision
    {
        return Err("LANDSAT_PC_PRESERVED_HISTORY_PAGE_INVALID".to_owned());
    }

    let events = store
        .list_task_events(TASK_ID)
        .map_err(|error| format!("LANDSAT_PC_PRESERVED_EVENTS_{}", error.code()))?;
    if events
        .iter()
        .enumerate()
        .any(|(index, event)| event.sequence != index as u64)
    {
        return Err("LANDSAT_PC_PRESERVED_EVENT_SEQUENCE_INVALID".to_owned());
    }
    let event_types = events
        .iter()
        .map(|event| event.event_type.as_str())
        .collect::<Vec<_>>();
    let count_events = |event_type: &str| {
        event_types
            .iter()
            .filter(|candidate| **candidate == event_type)
            .count()
    };
    let pause_index = event_types
        .iter()
        .position(|event_type| *event_type == "pause_task")
        .ok_or_else(|| "LANDSAT_PC_PRESERVED_PAUSE_EVENT_MISSING".to_owned())?;
    let resume_index = event_types
        .iter()
        .position(|event_type| *event_type == "resume_task")
        .ok_or_else(|| "LANDSAT_PC_PRESERVED_RESUME_EVENT_MISSING".to_owned())?;
    let completed_index = event_types
        .iter()
        .position(|event_type| *event_type == "mark_completed")
        .ok_or_else(|| "LANDSAT_PC_PRESERVED_COMPLETION_EVENT_MISSING".to_owned())?;
    if event_types.first().copied() != Some("task_created")
        || events.len() != 1 + 1 + 1 + EXPECTED_ASSET_COUNT + EXPECTED_ASSET_COUNT
        || count_events("task_created") != 1
        || count_events("activate_items") != EXPECTED_ASSET_COUNT
        || count_events("pause_task") != 1
        || count_events("resume_task") != 1
        || count_events("mark_completed") != EXPECTED_ASSET_COUNT
        || !(pause_index < resume_index && resume_index < completed_index)
    {
        return Err("LANDSAT_PC_PRESERVED_EVENT_TRANSITION_INVALID".to_owned());
    }

    let source_after_read = source_database_snapshot(&root)?;
    if source_after_read != source_before_copy {
        return Err("LANDSAT_PC_PRESERVED_SOURCE_CHANGED_DURING_READ_ONLY".to_owned());
    }
    let verification_after_read =
        required_file_fingerprint(&verification_database, "LANDSAT_PC_PRESERVED_COPY_INVALID")?;
    if verification_after_read != verification_fingerprint {
        return Err("LANDSAT_PC_PRESERVED_COPY_CHANGED_DURING_READ_ONLY".to_owned());
    }
    let copy_sidecars_after_read = copy_sidecar_snapshot(&verification_database)?;

    let receipt = json!({
        "schema_version": 1,
        "receipt_kind": "preserved_current_stack_history",
        "outcome": "passed",
        "original_harness": {
            "outcome": evidence.original_outcome,
            "code": evidence.original_code
        },
        "verification": {
            "store_read_only": true,
            "store_schema_version": restored.readiness.schema_version,
            "active_scheduler_task_absent": true,
            "source_workbench_retained": true,
            "history_summary_total_bytes_projected": history_item.task.total_bytes.is_some(),
            "source_database": source_database_snapshot_value(&source_before_copy),
            "source_database_stable_after_copy": source_after_copy == source_before_copy,
            "source_database_stable_after_read_only_open":
                source_after_open == source_before_copy,
            "source_database_stable_after_read_only": source_after_read == source_before_copy,
            "read_only_database_copy": {
                "path": verification_database
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("preserved.sqlite3"),
                "identity": "receipt_sibling_sqlite3",
                "created_with_create_new": true,
                "opened_with": "TaskStore::open_verification_read_only",
                "size_bytes": verification_fingerprint.size_bytes,
                "sha256": verification_fingerprint.sha256,
                "matches_source_database": true,
                "sidecars_copied": false,
                "generated_sidecars_after_open":
                    copy_sidecars_value(&verification_database, &copy_sidecars_after_open),
                "generated_sidecars_after_read_only":
                    copy_sidecars_value(&verification_database, &copy_sidecars_after_read)
            }
        },
        "task": {
            "task_id": TASK_ID,
            "batch_id": ASSET_TASK_BATCH_ID,
            "source_batch_id": SOURCE_BATCH_ID,
            "status": "completed",
            "revision": archived.revision,
            "selected_count": EXPECTED_ASSET_COUNT,
            "completed_count": EXPECTED_ASSET_COUNT,
            "expected_bytes": evidence.expected_bytes,
            "actual_bytes": actual_bytes
        },
        "store_events": {
            "task_created": count_events("task_created"),
            "activate_items": count_events("activate_items"),
            "pause_task": count_events("pause_task"),
            "resume_task": count_events("resume_task"),
            "mark_completed": count_events("mark_completed"),
            "pause_before_resume": true,
            "resume_before_completion": true,
            "archive_after_completion": true
        },
        "archive": {
            "status": "completed",
            "revision": evidence.archive_revision,
            "output_retained": true
        },
        "parent_workbench": {
            "batch_id": SOURCE_BATCH_ID,
            "candidate_count": evidence.candidate_count,
            "selected_scene_count": 1,
            "selected_scene_item_id": source_item_id
        },
        "source_batch": {
            "batch_id": ASSET_TASK_BATCH_ID,
            "candidate_count": source_page.total_count,
            "selected_asset_count": selected_source.len(),
            "completed_asset_count": selected_source.len(),
            "parent_source_item_id": source_item_id,
            "source_bytes": source_page_bytes,
            "source_total_bytes": source_page_total_bytes
        },
        "outputs": {
            "asset_count": evidence.hashes.len(),
            "expected_bytes": evidence.expected_bytes,
            "actual_bytes": actual_bytes,
            "hashes_verified": true,
            "hashes": evidence
                .hashes
                .iter()
                .map(|asset| json!({
                    "item_id": asset.item_id,
                    "file_name": asset.file_name,
                    "expected_size": asset.expected_size,
                    "actual_size": asset.actual_size,
                    "sha256": asset.sha256
                }))
                .collect::<Vec<_>>()
        },
        "evidence_transitions": {
            "paused_bytes": evidence.paused_bytes,
            "resumed_bytes": evidence.resumed_bytes,
            "positive_progress": evidence.resumed_bytes > evidence.paused_bytes
        }
    });
    let receipt_bytes = serde_json::to_vec_pretty(&receipt)
        .map_err(|_| "LANDSAT_PC_PRESERVED_RECEIPT_SERIALIZE_FAILED".to_owned())?;
    let mut receipt_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&receipt_path)
        .map_err(|_| "LANDSAT_PC_PRESERVED_RECEIPT_CREATE_NEW_FAILED".to_owned())?;
    receipt_file
        .write_all(&receipt_bytes)
        .map_err(|_| "LANDSAT_PC_PRESERVED_RECEIPT_WRITE_FAILED".to_owned())?;
    receipt_file
        .sync_all()
        .map_err(|_| "LANDSAT_PC_PRESERVED_RECEIPT_SYNC_FAILED".to_owned())?;
    Ok(receipt_path)
}

fn source_database_snapshot(root: &Path) -> Result<SourceDatabaseSnapshot, String> {
    let database = root.join("tasks.sqlite3");
    let wal = root.join("tasks.sqlite3-wal");
    let shm = root.join("tasks.sqlite3-shm");
    let database =
        required_file_fingerprint(&database, "LANDSAT_PC_PRESERVED_SOURCE_DATABASE_INVALID")?;
    let wal = optional_file_fingerprint(&wal, "LANDSAT_PC_PRESERVED_SOURCE_WAL_INVALID")?;
    if wal
        .as_ref()
        .is_some_and(|fingerprint| fingerprint.size_bytes != 0)
    {
        return Err("LANDSAT_PC_PRESERVED_SOURCE_WAL_NONEMPTY".to_owned());
    }
    let shm = optional_file_fingerprint(&shm, "LANDSAT_PC_PRESERVED_SOURCE_SHM_INVALID")?;
    Ok(SourceDatabaseSnapshot { database, wal, shm })
}

fn required_file_fingerprint(path: &Path, code: &str) -> Result<FileFingerprint, String> {
    let metadata = fs::symlink_metadata(path).map_err(|_| code.to_owned())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(code.to_owned());
    }
    let (size_bytes, sha256) = hash_regular_file(path).map_err(|_| code.to_owned())?;
    Ok(FileFingerprint { size_bytes, sha256 })
}

fn optional_file_fingerprint(path: &Path, code: &str) -> Result<Option<FileFingerprint>, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(code.to_owned());
            }
            let (size_bytes, sha256) = hash_regular_file(path).map_err(|_| code.to_owned())?;
            Ok(Some(FileFingerprint { size_bytes, sha256 }))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(code.to_owned()),
    }
}

fn create_database_copy_new(
    source: &Path,
    destination: &Path,
    source_fingerprint: &FileFingerprint,
) -> Result<FileFingerprint, String> {
    if fs::symlink_metadata(destination).is_ok() {
        return Err("LANDSAT_PC_PRESERVED_COPY_MUST_BE_NEW".to_owned());
    }
    let mut source_file = File::open(source)
        .map_err(|_| "LANDSAT_PC_PRESERVED_SOURCE_DATABASE_OPEN_FAILED".to_owned())?;
    let mut destination_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|_| "LANDSAT_PC_PRESERVED_COPY_CREATE_NEW_FAILED".to_owned())?;
    io::copy(&mut source_file, &mut destination_file)
        .map_err(|_| "LANDSAT_PC_PRESERVED_COPY_WRITE_FAILED".to_owned())?;
    destination_file
        .sync_all()
        .map_err(|_| "LANDSAT_PC_PRESERVED_COPY_SYNC_FAILED".to_owned())?;
    drop(destination_file);
    let copied = required_file_fingerprint(destination, "LANDSAT_PC_PRESERVED_COPY_INVALID")?;
    if copied != *source_fingerprint {
        return Err("LANDSAT_PC_PRESERVED_SOURCE_CHANGED_DURING_COPY".to_owned());
    }
    Ok(copied)
}

fn ensure_copy_has_no_sidecars(database: &Path) -> Result<(), String> {
    for path in [
        database.with_extension("sqlite3-wal"),
        database.with_extension("sqlite3-shm"),
    ] {
        if fs::symlink_metadata(path).is_ok() {
            return Err("LANDSAT_PC_PRESERVED_COPY_SIDECAR_UNEXPECTED".to_owned());
        }
    }
    Ok(())
}

fn copy_sidecar_snapshot(database: &Path) -> Result<CopySidecarSnapshot, String> {
    let wal = optional_file_fingerprint(
        &database.with_extension("sqlite3-wal"),
        "LANDSAT_PC_PRESERVED_COPY_WAL_INVALID",
    )?;
    if wal
        .as_ref()
        .is_some_and(|fingerprint| fingerprint.size_bytes != 0)
    {
        return Err("LANDSAT_PC_PRESERVED_COPY_WAL_NONEMPTY".to_owned());
    }
    let shm = optional_file_fingerprint(
        &database.with_extension("sqlite3-shm"),
        "LANDSAT_PC_PRESERVED_COPY_SHM_INVALID",
    )?;
    Ok(CopySidecarSnapshot { wal, shm })
}

fn copy_sidecars_value(database: &Path, snapshot: &CopySidecarSnapshot) -> Value {
    json!({
        "wal": copy_sidecar_value(&database.with_extension("sqlite3-wal"), snapshot.wal.as_ref()),
        "shm": copy_sidecar_value(&database.with_extension("sqlite3-shm"), snapshot.shm.as_ref()),
        "wal_zero_bytes": snapshot
            .wal
            .as_ref()
            .is_none_or(|fingerprint| fingerprint.size_bytes == 0)
    })
}

fn copy_sidecar_value(path: &Path, fingerprint: Option<&FileFingerprint>) -> Value {
    let path = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("sqlite3-sidecar");
    match fingerprint {
        Some(fingerprint) => json!({
            "path": path,
            "present": true,
            "size_bytes": fingerprint.size_bytes,
            "sha256": fingerprint.sha256
        }),
        None => json!({
            "path": path,
            "present": false
        }),
    }
}

fn source_database_snapshot_value(snapshot: &SourceDatabaseSnapshot) -> Value {
    json!({
        "path": "tasks.sqlite3",
        "size_bytes": snapshot.database.size_bytes,
        "sha256": snapshot.database.sha256,
        "wal": source_sidecar_value("tasks.sqlite3-wal", snapshot.wal.as_ref()),
        "shm": source_sidecar_value("tasks.sqlite3-shm", snapshot.shm.as_ref())
    })
}

fn source_sidecar_value(path: &str, fingerprint: Option<&FileFingerprint>) -> Value {
    match fingerprint {
        Some(fingerprint) => json!({
            "path": path,
            "present": true,
            "size_bytes": fingerprint.size_bytes,
            "sha256": fingerprint.sha256
        }),
        None => json!({
            "path": path,
            "present": false
        }),
    }
}

fn read_preserved_evidence(path: &Path) -> Result<PreservedEvidence, String> {
    let bytes =
        fs::read(path).map_err(|_| "LANDSAT_PC_PRESERVED_EVIDENCE_READ_FAILED".to_owned())?;
    let evidence: Value = serde_json::from_slice(&bytes)
        .map_err(|_| "LANDSAT_PC_PRESERVED_EVIDENCE_JSON_INVALID".to_owned())?;
    if preserved_evidence_u64(&evidence, "schema_version")? != 1 {
        return Err("LANDSAT_PC_PRESERVED_EVIDENCE_SCHEMA_INVALID".to_owned());
    }
    let original_outcome = preserved_evidence_string(&evidence, "outcome")?.to_owned();
    let original_code = preserved_evidence_string(&evidence, "code")?.to_owned();
    if original_outcome != "stopped"
        || original_code != "LANDSAT_PC_CURRENT_STACK_HISTORY_TASK_INVALID"
    {
        return Err("LANDSAT_PC_PRESERVED_ORIGINAL_FAILURE_INVALID".to_owned());
    }
    let identity = preserved_evidence_object(&evidence, "identity")?;
    if preserved_evidence_string_from(identity, "task_id")? != TASK_ID
        || preserved_evidence_string_from(identity, "source_batch_id")? != SOURCE_BATCH_ID
    {
        return Err("LANDSAT_PC_PRESERVED_IDENTITY_INVALID".to_owned());
    }

    let search = preserved_event_details(&evidence, "search_completed")?;
    let candidate_count = preserved_evidence_u64_from(search, "candidate_count")?;
    if candidate_count == 0 || candidate_count > MAX_CANDIDATES {
        return Err("LANDSAT_PC_PRESERVED_CANDIDATE_COUNT_INVALID".to_owned());
    }
    let pause_requested_index = preserved_event_index(&evidence, "pause_requested")?;
    let paused_index = preserved_event_index(&evidence, "paused")?;
    let resume_requested_index = preserved_event_index(&evidence, "resume_requested")?;
    let resumed_index = preserved_event_index(&evidence, "resumed")?;
    let completed_index = preserved_event_index(&evidence, "completed")?;
    let hashes_index = preserved_event_index(&evidence, "output_hashes_verified")?;
    let archive_index = preserved_event_index(&evidence, "archived")?;
    if !(pause_requested_index < paused_index
        && paused_index < resume_requested_index
        && resume_requested_index < resumed_index
        && resumed_index < completed_index
        && completed_index < hashes_index
        && hashes_index < archive_index)
    {
        return Err("LANDSAT_PC_PRESERVED_EVIDENCE_ORDER_INVALID".to_owned());
    }

    let paused = preserved_event_details(&evidence, "paused")?;
    if preserved_evidence_string_from(paused, "status")? != "paused"
        || !preserved_evidence_bool_from(paused, "task_paused")?
    {
        return Err("LANDSAT_PC_PRESERVED_PAUSED_EVIDENCE_INVALID".to_owned());
    }
    let paused_extra = preserved_evidence_object_from(paused, "extra")?;
    let paused_bytes = preserved_evidence_u64_from(paused_extra, "bytes")?;
    if paused_bytes == 0 || !preserved_evidence_bool_from(paused_extra, "pause_before_complete")? {
        return Err("LANDSAT_PC_PRESERVED_PAUSE_EVIDENCE_INVALID".to_owned());
    }
    let resumed = preserved_event_details(&evidence, "resumed")?;
    if preserved_evidence_string_from(resumed, "status")? != "running"
        || !preserved_evidence_bool_from(
            preserved_evidence_object_from(resumed, "extra")?,
            "positive_progress",
        )?
    {
        return Err("LANDSAT_PC_PRESERVED_RESUMED_EVIDENCE_INVALID".to_owned());
    }
    let resumed_extra = preserved_evidence_object_from(resumed, "extra")?;
    let resumed_bytes = preserved_evidence_u64_from(resumed_extra, "bytes_after_resume")?;
    if preserved_evidence_u64_from(resumed_extra, "bytes_at_pause")? != paused_bytes
        || resumed_bytes <= paused_bytes
    {
        return Err("LANDSAT_PC_PRESERVED_RESUME_EVIDENCE_INVALID".to_owned());
    }
    let completed = preserved_event_details(&evidence, "completed")?;
    if preserved_evidence_string_from(completed, "status")? != "completed"
        || preserved_evidence_bool_from(completed, "task_paused")?
        || preserved_evidence_u64_from(completed, "active_count")? != 0
        || preserved_evidence_u64_from(completed, "completed_count")? != EXPECTED_ASSET_COUNT as u64
        || preserved_evidence_u64_from(completed, "queued_count")? != 0
    {
        return Err("LANDSAT_PC_PRESERVED_COMPLETED_EVIDENCE_INVALID".to_owned());
    }
    let completed_extra = preserved_evidence_object_from(completed, "extra")?;
    let expected_bytes = preserved_evidence_u64_from(completed_extra, "expected_bytes")?;
    if preserved_evidence_u64_from(completed_extra, "asset_count")? != EXPECTED_ASSET_COUNT as u64
        || preserved_evidence_u64_from(completed_extra, "observed_bytes")? != expected_bytes
    {
        return Err("LANDSAT_PC_PRESERVED_COMPLETED_BYTES_INVALID".to_owned());
    }
    let hashes_details = preserved_event_details(&evidence, "output_hashes_verified")?;
    let actual_bytes = preserved_evidence_u64_from(hashes_details, "actual_bytes")?;
    let hashes = preserved_hashes(hashes_details)?;
    if hashes.len() != EXPECTED_ASSET_COUNT
        || preserved_evidence_u64_from(hashes_details, "asset_count")?
            != EXPECTED_ASSET_COUNT as u64
        || actual_bytes != expected_bytes
    {
        return Err("LANDSAT_PC_PRESERVED_HASH_EVIDENCE_INVALID".to_owned());
    }
    let archive = preserved_event_details(&evidence, "archived")?;
    let archive_revision = preserved_evidence_u64_from(archive, "archive_revision")?;
    if preserved_evidence_u64_from(archive, "revision_before_archive")? != archive_revision
        || !preserved_evidence_bool_from(archive, "output_retained")?
    {
        return Err("LANDSAT_PC_PRESERVED_ARCHIVE_EVIDENCE_INVALID".to_owned());
    }
    Ok(PreservedEvidence {
        candidate_count,
        expected_bytes,
        archive_revision,
        hashes,
        original_outcome,
        original_code,
        paused_bytes,
        resumed_bytes,
    })
}

fn preserved_hashes(details: &Map<String, Value>) -> Result<Vec<HashedAsset>, String> {
    let entries = details
        .get("sha256")
        .and_then(Value::as_array)
        .ok_or_else(|| "LANDSAT_PC_PRESERVED_HASH_LIST_MISSING".to_owned())?;
    let mut item_ids = BTreeSet::new();
    let mut file_names = BTreeSet::new();
    let mut hashes = Vec::with_capacity(entries.len());
    for entry in entries {
        let entry = entry
            .as_object()
            .ok_or_else(|| "LANDSAT_PC_PRESERVED_HASH_ENTRY_INVALID".to_owned())?;
        let item_id = preserved_evidence_string_from(entry, "item_id")?.to_owned();
        let file_name = preserved_evidence_string_from(entry, "file_name")?.to_owned();
        let expected_size = preserved_evidence_u64_from(entry, "expected_size")?;
        let actual_size = preserved_evidence_u64_from(entry, "actual_size")?;
        let sha256 = preserved_evidence_string_from(entry, "sha256")?.to_owned();
        if item_id.is_empty()
            || !item_ids.insert(item_id.clone())
            || !safe_file_name(&file_name)
            || !file_names.insert(file_name.clone())
            || expected_size == 0
            || actual_size != expected_size
            || sha256.len() != 64
            || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("LANDSAT_PC_PRESERVED_HASH_ENTRY_INVALID".to_owned());
        }
        hashes.push(HashedAsset {
            item_id,
            file_name,
            expected_size,
            actual_size,
            sha256,
        });
    }
    Ok(hashes)
}

fn preserved_event_index(value: &Value, label: &str) -> Result<usize, String> {
    let events = value
        .get("events")
        .and_then(Value::as_array)
        .ok_or_else(|| "LANDSAT_PC_PRESERVED_EVIDENCE_EVENTS_INVALID".to_owned())?;
    let mut found = None;
    for (index, event) in events.iter().enumerate() {
        if event.get("label").and_then(Value::as_str) == Some(label) {
            if found.is_some() {
                return Err("LANDSAT_PC_PRESERVED_EVIDENCE_EVENT_DUPLICATE".to_owned());
            }
            found = Some(index);
        }
    }
    found.ok_or_else(|| "LANDSAT_PC_PRESERVED_EVIDENCE_EVENT_MISSING".to_owned())
}

fn preserved_event_details<'a>(
    value: &'a Value,
    label: &str,
) -> Result<&'a Map<String, Value>, String> {
    let index = preserved_event_index(value, label)?;
    value
        .get("events")
        .and_then(Value::as_array)
        .and_then(|events| events.get(index))
        .and_then(|event| event.get("details"))
        .and_then(Value::as_object)
        .ok_or_else(|| "LANDSAT_PC_PRESERVED_EVIDENCE_DETAILS_INVALID".to_owned())
}

fn preserved_evidence_object<'a>(
    value: &'a Value,
    key: &str,
) -> Result<&'a Map<String, Value>, String> {
    value
        .as_object()
        .ok_or_else(|| "LANDSAT_PC_PRESERVED_EVIDENCE_OBJECT_INVALID".to_owned())
        .and_then(|object| preserved_evidence_object_from(object, key))
}

fn preserved_evidence_object_from<'a>(
    object: &'a Map<String, Value>,
    key: &str,
) -> Result<&'a Map<String, Value>, String> {
    object
        .get(key)
        .and_then(Value::as_object)
        .ok_or_else(|| "LANDSAT_PC_PRESERVED_EVIDENCE_FIELD_INVALID".to_owned())
}

fn preserved_evidence_string<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .as_object()
        .ok_or_else(|| "LANDSAT_PC_PRESERVED_EVIDENCE_OBJECT_INVALID".to_owned())
        .and_then(|object| preserved_evidence_string_from(object, key))
}

fn preserved_evidence_string_from<'a>(
    object: &'a Map<String, Value>,
    key: &str,
) -> Result<&'a str, String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| "LANDSAT_PC_PRESERVED_EVIDENCE_FIELD_INVALID".to_owned())
}

fn preserved_evidence_u64(value: &Value, key: &str) -> Result<u64, String> {
    value
        .as_object()
        .ok_or_else(|| "LANDSAT_PC_PRESERVED_EVIDENCE_OBJECT_INVALID".to_owned())
        .and_then(|object| preserved_evidence_u64_from(object, key))
}

fn preserved_evidence_u64_from(object: &Map<String, Value>, key: &str) -> Result<u64, String> {
    object
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| "LANDSAT_PC_PRESERVED_EVIDENCE_FIELD_INVALID".to_owned())
}

fn preserved_evidence_bool_from(object: &Map<String, Value>, key: &str) -> Result<bool, String> {
    object
        .get(key)
        .and_then(Value::as_bool)
        .ok_or_else(|| "LANDSAT_PC_PRESERVED_EVIDENCE_FIELD_INVALID".to_owned())
}

fn run_live() -> Result<LiveReport, String> {
    let root = std::env::var_os(LIVE_ROOT_ENV)
        .map(PathBuf::from)
        .ok_or_else(|| "LANDSAT_PC_CURRENT_STACK_ROOT_REQUIRED".to_owned())?;
    let workspace = AcceptanceWorkspace::create(root)?;
    let mut evidence = EvidenceJournal::new(&workspace)?;
    evidence.record(
        "workspace_created",
        json!({
            "root_absent_before_start": true,
            "output_root": "output",
            "database": "tasks.sqlite3",
            "evidence": "acceptance-evidence.json"
        }),
    )?;

    let result = run_live_inner(&workspace, &mut evidence);
    let finish_result = match &result {
        Ok(_) => evidence.finish("completed", None),
        Err(code) => evidence.finish("stopped", Some(code)),
    };
    match (result, finish_result) {
        (Ok(report), Ok(())) => Ok(report),
        (Err(code), Ok(())) => Err(code),
        (Ok(_), Err(_)) => Err("LANDSAT_PC_CURRENT_STACK_EVIDENCE_FINALIZE_FAILED".to_owned()),
        (Err(code), Err(_)) => Err(code),
    }
}

fn run_live_inner(
    workspace: &AcceptanceWorkspace,
    evidence: &mut EvidenceJournal,
) -> Result<LiveReport, String> {
    let request = live_search_request()?;
    evidence.record(
        "search_requested",
        json!({
            "route": "application_service_production",
            "provider": "planetary_computer",
            "request_id": "landsat-pc-current-stack-search",
            "result_limit": MAX_CANDIDATES,
            "datetime_start": "2023-03-26T00:00:00Z",
            "datetime_end": "2023-03-27T00:00:00Z",
            "aoi_source": "known scene source metadata bbox",
            "aoi_bbox": [-76.9196, 35.5713, -75.446, 36.3793]
        }),
    )?;

    // Production construction wires the real PC catalogue and existing
    // scheduler. No fixture Provider, custom HTTP client, or credential input
    // is injected into this acceptance path.
    let service = ApplicationService::new(ApplicationServiceConfig::business_store(
        &workspace.database,
    ));
    let mut service_shutdown = ServiceShutdownGuard::new(&service);
    let search_snapshot = service
        .catalog_search_and_freeze(&request)
        .map_err(|error| format!("LANDSAT_PC_CURRENT_STACK_SEARCH_{}", error.code()))?;
    let candidate_count = search_snapshot
        .core
        .workbench
        .as_ref()
        .map(|workbench| workbench.candidate_count)
        .unwrap_or(0);
    if candidate_count == 0 || candidate_count > MAX_CANDIDATES as usize {
        return Err("LANDSAT_PC_CURRENT_STACK_SEARCH_RESULT_COUNT_INVALID".to_owned());
    }
    evidence.record(
        "search_completed",
        json!({
            "candidate_count": candidate_count,
            "workbench_revision": search_snapshot
                .core
                .workbench
                .as_ref()
                .map(|workbench| workbench.revision)
                .unwrap_or(0),
            "source_workbench_retained": true
        }),
    )?;

    let page = service
        .workbench_page(&StoreWorkbenchPageRequest {
            batch_id: SOURCE_BATCH_ID.to_owned(),
            expected_revision: 0,
            page_index: 0,
            page_size: MAX_CANDIDATES,
        })
        .map_err(|error| format!("LANDSAT_PC_CURRENT_STACK_WORKBENCH_PAGE_{}", error.code()))?;
    if page.page.batch_id != SOURCE_BATCH_ID
        || page.page.total_count != candidate_count
        || page.page.items.len() != candidate_count
    {
        return Err("LANDSAT_PC_CURRENT_STACK_WORKBENCH_PAGE_INVALID".to_owned());
    }
    let matching = page
        .page
        .items
        .iter()
        .filter(|candidate| {
            candidate.item.metadata.get("provider_product_id")
                == Some(&Value::String(ENTITY_ID.to_owned()))
                && candidate.item.display_name == DISPLAY_ID
                && candidate.item.metadata["properties"]["processing_level"] == "l2sp"
        })
        .collect::<Vec<_>>();
    if matching.len() != 1 {
        return Err("LANDSAT_PC_CURRENT_STACK_SCENE_IDENTITY_NOT_UNIQUE".to_owned());
    }
    let source_item_id = matching[0].item.item_id.clone();
    evidence.record(
        "scene_selected_for_preview",
        json!({
            "candidate_count": candidate_count,
            "item_id": source_item_id,
            "display_id": DISPLAY_ID,
            "selection_source": "frozen_workbench_page"
        }),
    )?;

    let preview = service
        .preview_asset(PreviewAssetRequest {
            request_id: "landsat-pc-current-stack-preview".to_owned(),
            batch_id: SOURCE_BATCH_ID.to_owned(),
            item_id: source_item_id.clone(),
        })
        .map_err(|error| format!("LANDSAT_PC_CURRENT_STACK_PREVIEW_{}", error.code()))?;
    if preview.source_asset_role != PreviewAssetRole::Browse
        || preview.byte_length == 0
        || !preview.media_type.starts_with("image/")
        || !preview
            .data_url
            .starts_with(&format!("data:{};base64,", preview.media_type))
    {
        return Err("LANDSAT_PC_CURRENT_STACK_PREVIEW_INVALID".to_owned());
    }
    evidence.record(
        "shared_preview_completed",
        json!({
            "source_asset_role": "browse",
            "media_type": preview.media_type,
            "byte_length": preview.byte_length,
            "locator_persisted": false
        }),
    )?;

    let selection_revision = page
        .page
        .revision
        .checked_add(1)
        .ok_or_else(|| "LANDSAT_PC_CURRENT_STACK_WORKBENCH_REVISION_OVERFLOW".to_owned())?;
    service
        .update_search_workbench(&StoreUpdateWorkbenchRequest {
            state: SearchWorkbenchState {
                batch_id: SOURCE_BATCH_ID.to_owned(),
                checked_item_ids: vec![source_item_id.clone()],
                page_index: 0,
                page_size: MAX_CANDIDATES,
                revision: selection_revision,
            },
            expected_revision: page.page.revision,
            updated_at: "2026-09-06T00:00:01Z".to_owned(),
        })
        .map_err(|error| format!("LANDSAT_PC_CURRENT_STACK_SELECTION_{}", error.code()))?;
    evidence.record(
        "scene_selection_persisted",
        json!({
            "checked_count": 1,
            "selection_revision": selection_revision,
            "source_batch_id": SOURCE_BATCH_ID
        }),
    )?;

    let created = service
        .create_landsat_asset_task(&LandsatCreateBundleTaskRequest {
            request_id: "landsat-pc-current-stack-assets".to_owned(),
            source_batch_id: SOURCE_BATCH_ID.to_owned(),
            item_id: source_item_id,
            task_id: TASK_ID.to_owned(),
            output_root: workspace.output_root.clone(),
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-09-06T00:00:02Z".to_owned(),
        })
        .map_err(|error| format!("LANDSAT_PC_CURRENT_STACK_TASK_CREATE_{}", error.code()))?;
    let task_summary = created
        .core
        .tasks
        .iter()
        .find(|task| task.task_id == TASK_ID)
        .ok_or_else(|| "LANDSAT_PC_CURRENT_STACK_TASK_MISSING".to_owned())?;
    if task_summary.batch_id != ASSET_TASK_BATCH_ID
        || task_summary.selected_count != EXPECTED_ASSET_COUNT
        || task_summary.layout != DownloadLayout::Direct
        || task_summary.output_root != workspace.output_root
    {
        return Err("LANDSAT_PC_CURRENT_STACK_TASK_ASSET_SET_INVALID".to_owned());
    }
    let assets = load_frozen_task_assets(&service)?;
    let expected_bytes = assets
        .iter()
        .map(|asset| asset.expected_size)
        .try_fold(0_u64, u64::checked_add)
        .ok_or_else(|| "LANDSAT_PC_CURRENT_STACK_SIZE_OVERFLOW".to_owned())?;
    if expected_bytes == 0 || expected_bytes > MAX_TOTAL_BYTES {
        return Err("LANDSAT_PC_CURRENT_STACK_EXPECTED_SIZE_INVALID".to_owned());
    }
    if task_summary
        .total_bytes
        .is_some_and(|value| value != expected_bytes)
    {
        return Err("LANDSAT_PC_CURRENT_STACK_TASK_TOTAL_MISMATCH".to_owned());
    }
    evidence.record(
        "asset_set_frozen",
        json!({
            "asset_count": assets.len(),
            "expected_bytes": expected_bytes,
            "task_revision": task_summary.revision,
            "concurrency": task_summary.concurrency,
            "signed_material_persisted": false
        }),
    )?;

    let mut active_observed = false;
    let mut part_observed = false;
    let pause_deadline = Instant::now() + SEARCH_WAIT;
    let (paused_bytes, pause_revision, resume_revision, resumed_bytes) = loop {
        let download = service
            .download_snapshot(TASK_ID)
            .map_err(|error| format!("LANDSAT_PC_CURRENT_STACK_PROGRESS_{}", error.code()))?;
        active_observed |= download
            .progress
            .iter()
            .any(|progress| progress.status == DownloadItemStatus::Active);
        part_observed |= any_part_file(&workspace.output_root, &assets);
        let bytes = observed_bytes(&download);
        if !download.results.is_empty()
            && download.results.iter().any(|result| {
                !matches!(
                    result.outcome,
                    DownloadOutcome::Completed | DownloadOutcome::Skipped
                )
            })
        {
            return Err("LANDSAT_PC_CURRENT_STACK_TRANSFER_FAILED_BEFORE_PAUSE".to_owned());
        }
        if bytes > 0 {
            let before_pause = task_runtime(&service)?;
            if before_pause.status == TaskLifecycleStatus::Completed
                || before_pause.completed_item_ids.len() == assets.len()
            {
                return Err("LANDSAT_PC_CURRENT_STACK_PAUSE_WINDOW_MISSED".to_owned());
            }
            evidence.record(
                "pause_requested",
                runtime_details(
                    &before_pause,
                    &download,
                    json!({"bytes_before_pause": bytes}),
                ),
            )?;
            update_task_with_revision_retry(
                &service,
                TaskStateCommandKind::PauseTask,
                "landsat-pc-current-stack-pause",
                "2026-09-06T00:00:03Z",
            )?;
            wait_for_task_paused(&service)?;
            let (paused, paused_download, stable_before, stable_after, stable_for_ms) =
                wait_for_paused_files_stable(&service, &workspace.output_root, &assets)?;
            part_observed |=
                !stable_before.part_sizes.is_empty() || !stable_after.part_sizes.is_empty();
            let paused_bytes = observed_bytes(&paused_download);
            if paused_bytes == 0 {
                return Err("LANDSAT_PC_CURRENT_STACK_PAUSED_BYTES_INVALID".to_owned());
            }
            evidence.record(
                "paused",
                runtime_details(
                    &paused,
                    &paused_download,
                    json!({
                        "bytes": paused_bytes,
                        "pause_before_complete": true,
                        "filesystem_before": file_progress_snapshot_value(&stable_before),
                        "filesystem_after": file_progress_snapshot_value(&stable_after),
                        "filesystem_stable_for_ms": stable_for_ms
                    }),
                ),
            )?;

            let before_resume = task_runtime(&service)?;
            evidence.record(
                "resume_requested",
                json!({
                    "revision": before_resume.revision,
                    "bytes_at_pause": paused_bytes
                }),
            )?;
            update_task_with_revision_retry(
                &service,
                TaskStateCommandKind::ResumeTask,
                "landsat-pc-current-stack-resume",
                "2026-09-06T00:00:04Z",
            )?;
            let resumed = wait_for_task_resumed(&service)?;
            let resume_deadline = Instant::now() + STATE_WAIT;
            let mut resumed_bytes = 0_u64;
            while Instant::now() < resume_deadline {
                let resumed_download = service.download_snapshot(TASK_ID).map_err(|error| {
                    format!("LANDSAT_PC_CURRENT_STACK_RESUMED_SNAPSHOT_{}", error.code())
                })?;
                active_observed |= resumed_download
                    .progress
                    .iter()
                    .any(|progress| progress.status == DownloadItemStatus::Active);
                part_observed |= any_part_file(&workspace.output_root, &assets);
                resumed_bytes = resumed_bytes.max(observed_bytes(&resumed_download));
                if resumed_bytes > paused_bytes {
                    break;
                }
                thread::sleep(POLL_INTERVAL);
            }
            if resumed_bytes <= paused_bytes {
                return Err("LANDSAT_PC_CURRENT_STACK_RESUME_NO_PROGRESS".to_owned());
            }
            evidence.record(
                "resumed",
                runtime_details(
                    &resumed,
                    &service.download_snapshot(TASK_ID).map_err(|error| {
                        format!(
                            "LANDSAT_PC_CURRENT_STACK_RESUMED_FINAL_SNAPSHOT_{}",
                            error.code()
                        )
                    })?,
                    json!({
                        "bytes_at_pause": paused_bytes,
                        "bytes_after_resume": resumed_bytes,
                        "positive_progress": true
                    }),
                ),
            )?;
            break (
                paused_bytes,
                paused.revision,
                resumed.revision,
                resumed_bytes,
            );
        }
        if Instant::now() >= pause_deadline {
            return Err("LANDSAT_PC_CURRENT_STACK_NO_POSITIVE_BYTES_BEFORE_PAUSE".to_owned());
        }
        thread::sleep(POLL_INTERVAL);
    };

    let completed_download = wait_for_completion(
        &service,
        &workspace.output_root,
        &assets,
        &mut active_observed,
        &mut part_observed,
    )?;
    let completed_runtime = task_runtime(&service)?;
    evidence.record(
        "completed",
        runtime_details(
            &completed_runtime,
            &completed_download,
            json!({
                "asset_count": completed_download.results.len(),
                "expected_bytes": expected_bytes,
                "observed_bytes": observed_bytes(&completed_download)
            }),
        ),
    )?;

    let hashed = hash_outputs(&workspace.output_root, &assets, &completed_download)?;
    let actual_bytes = hashed
        .iter()
        .map(|asset| asset.actual_size)
        .try_fold(0_u64, u64::checked_add)
        .ok_or_else(|| "LANDSAT_PC_CURRENT_STACK_SIZE_OVERFLOW".to_owned())?;
    if actual_bytes != expected_bytes {
        return Err("LANDSAT_PC_CURRENT_STACK_TOTAL_SIZE_MISMATCH".to_owned());
    }
    evidence.record(
        "output_hashes_verified",
        json!({
            "asset_count": hashed.len(),
            "actual_bytes": actual_bytes,
            "sha256": hashed
                .iter()
                .map(|asset| {
                    json!({
                        "item_id": asset.item_id,
                        "file_name": asset.file_name,
                        "expected_size": asset.expected_size,
                        "actual_size": asset.actual_size,
                        "sha256": asset.sha256
                    })
                })
                .collect::<Vec<_>>()
        }),
    )?;

    let archive_before = task_runtime(&service)?;
    let archive = service
        .archive_download_task(&StoreArchiveTaskRequest {
            task_id: TASK_ID.to_owned(),
            expected_revision: archive_before.revision,
            archived_at: "2026-09-06T00:10:00Z".to_owned(),
        })
        .map_err(|error| format!("LANDSAT_PC_CURRENT_STACK_ARCHIVE_{}", error.code()))?;
    evidence.record(
        "archived",
        json!({
            "revision_before_archive": archive_before.revision,
            "archive_revision": archive.revision,
            "output_retained": true
        }),
    )?;
    service
        .shutdown()
        .map_err(|error| format!("LANDSAT_PC_CURRENT_STACK_SHUTDOWN_{}", error.code()))?;
    service_shutdown.disarm();

    let reopened = ApplicationService::new(ApplicationServiceConfig::existing_business_store(
        &workspace.database,
    ));
    let mut reopened_shutdown = ServiceShutdownGuard::new(&reopened);
    let history = reopened
        .task_history_page(&StoreTaskHistoryPageRequest {
            page_size: 100,
            after_archived_at_epoch_ms: None,
            after_task_id: None,
        })
        .map_err(|error| format!("LANDSAT_PC_CURRENT_STACK_HISTORY_{}", error.code()))?;
    let history_item = history
        .items
        .iter()
        .find(|item| item.task.task_id == TASK_ID)
        .ok_or_else(|| "LANDSAT_PC_CURRENT_STACK_HISTORY_TASK_MISSING".to_owned())?;
    if history_item.task.batch_id != ASSET_TASK_BATCH_ID
        || history_item.task.status != TaskLifecycleStatus::Completed
        || history_item.task.selected_count != EXPECTED_ASSET_COUNT
        || history_item.task.completed_count != EXPECTED_ASSET_COUNT
    {
        return Err("LANDSAT_PC_CURRENT_STACK_HISTORY_TASK_INVALID".to_owned());
    }
    let restored = reopened
        .snapshot(true)
        .map_err(|error| format!("LANDSAT_PC_CURRENT_STACK_REOPEN_{}", error.code()))?;
    if restored
        .restore
        .tasks
        .iter()
        .any(|task| task.task_id == TASK_ID)
        || restored
            .restore
            .workbench_batch
            .as_ref()
            .map(|batch| batch.batch_id.as_str())
            != Some(SOURCE_BATCH_ID)
    {
        return Err("LANDSAT_PC_CURRENT_STACK_REOPEN_AUTHORITY_INVALID".to_owned());
    }
    let restored_hashes = hash_outputs_again(&workspace.output_root, &hashed)?;
    if restored_hashes != hashed {
        return Err("LANDSAT_PC_CURRENT_STACK_OUTPUT_CHANGED_AFTER_REOPEN".to_owned());
    }
    evidence.record(
        "history_reopened",
        json!({
            "history_status": "completed",
            "history_revision": history_item.task.revision,
            "active_task_absent": true,
            "source_workbench_retained": true,
            "output_hashes_rechecked": true
        }),
    )?;
    reopened
        .shutdown()
        .map_err(|error| format!("LANDSAT_PC_CURRENT_STACK_REOPEN_SHUTDOWN_{}", error.code()))?;
    reopened_shutdown.disarm();

    Ok(LiveReport {
        candidate_count,
        asset_count: assets.len(),
        expected_bytes,
        actual_bytes,
        active_observed,
        part_observed,
        paused_bytes,
        resumed_bytes,
        pause_revision,
        resume_revision,
        history_revision: history_item.task.revision,
    })
}

fn live_search_request() -> Result<ApplicationCatalogSearchAndFreezeRequest, String> {
    // These four extrema are the bounding envelope recorded by the existing
    // accepted source metadata for ENTITY_ID.  The query AOI is intentionally
    // the source bbox envelope; the authoritative PC footprint is still read
    // from the live STAC result and frozen by ApplicationService.
    let aoi_geojson: Map<String, Value> = serde_json::from_value(json!({
        "type": "Polygon",
        "coordinates": [[
            [-75.446, 36.3793],
            [-76.9196, 36.3793],
            [-76.9196, 35.5713],
            [-75.446, 35.5713],
            [-75.446, 36.3793]
        ]]
    }))
    .map_err(|_| "LANDSAT_PC_CURRENT_STACK_AOI_INVALID".to_owned())?;
    let search: CatalogSearchQuery = serde_json::from_value(json!({
        "logical_dataset": "optical.landsat.c2.l2",
        "provider_id": "usgs.eros.m2m",
        "native_collection_id": DATASET_ALIAS,
        "aoi_geojson": aoi_geojson,
        "result_limit": MAX_CANDIDATES,
        "filters": {
            "datetime": {
                "start": "2023-03-26T00:00:00Z",
                "end": "2023-03-27T00:00:00Z"
            },
            "cloud_cover_lte": 100.0,
            "spacecraft": ["landsat_9"],
            "processing_levels": ["l2sp"]
        }
    }))
    .map_err(|_| "LANDSAT_PC_CURRENT_STACK_QUERY_INVALID".to_owned())?;
    search
        .validate()
        .map_err(|_| "LANDSAT_PC_CURRENT_STACK_QUERY_INVALID".to_owned())?;
    Ok(ApplicationCatalogSearchAndFreezeRequest {
        request_id: "landsat-pc-current-stack-search".to_owned(),
        search,
        batch_id: SOURCE_BATCH_ID.to_owned(),
        created_at: "2026-09-06T00:00:00Z".to_owned(),
        aoi_name: "Known Landsat scene source metadata bbox".to_owned(),
        aoi_geojson: Some(aoi_geojson),
        workbench_page_size: MAX_CANDIDATES,
    })
}

fn load_frozen_task_assets(service: &ApplicationService) -> Result<Vec<FrozenAsset>, String> {
    for _ in 0..200 {
        let runtime = task_runtime(service)?;
        match service.task_page(&StoreTaskPageRequest {
            task_id: TASK_ID.to_owned(),
            expected_revision: runtime.revision,
            page_index: 0,
            page_size: EXPECTED_ASSET_COUNT as u64,
        }) {
            Ok(page) => {
                if page.total_count != EXPECTED_ASSET_COUNT as u64
                    || page.items.len() != EXPECTED_ASSET_COUNT
                {
                    return Err("LANDSAT_PC_CURRENT_STACK_ASSET_PAGE_INVALID".to_owned());
                }
                let mut assets = Vec::with_capacity(page.items.len());
                for entry in page.items {
                    let access = entry.item.asset_access.as_ref().ok_or_else(|| {
                        "LANDSAT_PC_CURRENT_STACK_ASSET_ACCESS_MISSING".to_owned()
                    })?;
                    let expected_size = access
                        .expected_size_bytes
                        .filter(|size| *size > 0)
                        .ok_or_else(|| "LANDSAT_PC_CURRENT_STACK_ASSET_SIZE_MISSING".to_owned())?;
                    if access.file_name.as_deref() != Some(entry.item.display_name.as_str())
                        || !safe_file_name(&entry.item.display_name)
                    {
                        return Err("LANDSAT_PC_CURRENT_STACK_ASSET_FILENAME_INVALID".to_owned());
                    }
                    assets.push(FrozenAsset {
                        item_id: entry.item.item_id,
                        file_name: entry.item.display_name,
                        expected_size,
                    });
                }
                assets.sort_by(|left, right| left.file_name.cmp(&right.file_name));
                if assets
                    .iter()
                    .map(|asset| asset.file_name.as_str())
                    .collect::<BTreeSet<_>>()
                    .len()
                    != assets.len()
                {
                    return Err("LANDSAT_PC_CURRENT_STACK_ASSET_FILENAME_DUPLICATE".to_owned());
                }
                return Ok(assets);
            }
            Err(error) if error.code() == "V3_STORE_REVISION_CONFLICT" => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => {
                return Err(format!(
                    "LANDSAT_PC_CURRENT_STACK_ASSET_PAGE_{}",
                    error.code()
                ));
            }
        }
    }
    Err("LANDSAT_PC_CURRENT_STACK_ASSET_PAGE_REVISION_UNSTABLE".to_owned())
}

fn safe_file_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 240
        && value != "."
        && value != ".."
        && !value.contains('/')
        && !value.contains('\\')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        && Path::new(value).file_name().and_then(|name| name.to_str()) == Some(value)
}

fn task_runtime(service: &ApplicationService) -> Result<TaskRuntimeSnapshot, String> {
    service
        .snapshot(true)
        .map_err(|error| format!("LANDSAT_PC_CURRENT_STACK_RUNTIME_{}", error.code()))?
        .restore
        .runtimes
        .into_iter()
        .find(|runtime| runtime.task_id == TASK_ID)
        .ok_or_else(|| "LANDSAT_PC_CURRENT_STACK_RUNTIME_MISSING".to_owned())
}

fn update_task_with_revision_retry(
    service: &ApplicationService,
    kind: TaskStateCommandKind,
    event_prefix: &str,
    occurred_at: &str,
) -> Result<(), String> {
    for attempt in 0..200 {
        let runtime = task_runtime(service)?;
        match service.update_download_task(&StoreUpdateTaskRequest {
            task_id: TASK_ID.to_owned(),
            expected_revision: runtime.revision,
            command: StoreTaskCommand {
                kind: kind.clone(),
                item_ids: Vec::new(),
            },
            event_id: format!("{event_prefix}-{attempt}"),
            occurred_at: occurred_at.to_owned(),
        }) {
            Ok(_) => return Ok(()),
            Err(error) if error.code() == "V3_STORE_REVISION_CONFLICT" => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(format!("LANDSAT_PC_CURRENT_STACK_STATE_{}", error.code())),
        }
    }
    Err("LANDSAT_PC_CURRENT_STACK_STATE_REVISION_UNSTABLE".to_owned())
}

fn wait_for_task_paused(service: &ApplicationService) -> Result<TaskRuntimeSnapshot, String> {
    let deadline = Instant::now() + STATE_WAIT;
    loop {
        let runtime = task_runtime(service)?;
        if runtime.task_paused && runtime.status == TaskLifecycleStatus::Paused {
            return Ok(runtime);
        }
        if runtime.status == TaskLifecycleStatus::Completed {
            return Err("LANDSAT_PC_CURRENT_STACK_PAUSE_WINDOW_MISSED".to_owned());
        }
        if Instant::now() >= deadline {
            return Err("LANDSAT_PC_CURRENT_STACK_PAUSE_NOT_OBSERVED".to_owned());
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn wait_for_paused_files_stable(
    service: &ApplicationService,
    output_root: &Path,
    assets: &[FrozenAsset],
) -> Result<
    (
        TaskRuntimeSnapshot,
        DownloadEngineSnapshot,
        FileProgressSnapshot,
        FileProgressSnapshot,
        u64,
    ),
    String,
> {
    let deadline = Instant::now() + STATE_WAIT;
    let mut stable_since = Instant::now();
    let mut stable_before = filesystem_progress_snapshot(output_root, assets)?;
    let mut stable_after = stable_before.clone();
    loop {
        let runtime = task_runtime(service)?;
        if runtime.status == TaskLifecycleStatus::Completed {
            return Err("LANDSAT_PC_CURRENT_STACK_PAUSE_WINDOW_MISSED".to_owned());
        }
        if runtime.status != TaskLifecycleStatus::Paused || !runtime.task_paused {
            return Err("LANDSAT_PC_CURRENT_STACK_PAUSE_STATE_LOST".to_owned());
        }
        let download = service.download_snapshot(TASK_ID).map_err(|error| {
            format!("LANDSAT_PC_CURRENT_STACK_PAUSED_SNAPSHOT_{}", error.code())
        })?;
        let current = filesystem_progress_snapshot(output_root, assets)?;
        if current != stable_after {
            stable_before = current.clone();
            stable_after = current;
            stable_since = Instant::now();
        }
        if stable_since.elapsed() >= PAUSE_STABILITY_WAIT {
            let stable_for_ms =
                u64::try_from(stable_since.elapsed().as_millis()).unwrap_or(u64::MAX);
            return Ok((
                runtime,
                download,
                stable_before,
                stable_after,
                stable_for_ms,
            ));
        }
        if Instant::now() >= deadline {
            return Err("LANDSAT_PC_CURRENT_STACK_PAUSE_FILES_NOT_STABLE".to_owned());
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn filesystem_progress_snapshot(
    output_root: &Path,
    assets: &[FrozenAsset],
) -> Result<FileProgressSnapshot, String> {
    let mut part_sizes = Vec::new();
    let mut final_sizes = Vec::new();
    for asset in assets {
        let destination = output_root.join(&asset.file_name);
        if let Some(size) = regular_file_size(&part_path(&destination))? {
            part_sizes.push((asset.file_name.clone(), size));
        }
        if let Some(size) = regular_file_size(&destination)? {
            final_sizes.push((asset.file_name.clone(), size));
        }
    }
    Ok(FileProgressSnapshot {
        part_sizes,
        final_sizes,
    })
}

fn regular_file_size(path: &Path) -> Result<Option<u64>, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err("LANDSAT_PC_CURRENT_STACK_OUTPUT_SNAPSHOT_INVALID".to_owned());
            }
            Ok(Some(metadata.len()))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("LANDSAT_PC_CURRENT_STACK_OUTPUT_SNAPSHOT_READ_FAILED".to_owned()),
    }
}

fn file_progress_snapshot_value(snapshot: &FileProgressSnapshot) -> Value {
    json!({
        "part_sizes": snapshot
            .part_sizes
            .iter()
            .map(|(file_name, bytes)| json!({"file_name": file_name, "bytes": bytes}))
            .collect::<Vec<_>>(),
        "final_sizes": snapshot
            .final_sizes
            .iter()
            .map(|(file_name, bytes)| json!({"file_name": file_name, "bytes": bytes}))
            .collect::<Vec<_>>()
    })
}

fn wait_for_task_resumed(service: &ApplicationService) -> Result<TaskRuntimeSnapshot, String> {
    let deadline = Instant::now() + STATE_WAIT;
    loop {
        let runtime = task_runtime(service)?;
        if !runtime.task_paused
            && matches!(
                runtime.status,
                TaskLifecycleStatus::Running | TaskLifecycleStatus::Completed
            )
        {
            return Ok(runtime);
        }
        if runtime.status == TaskLifecycleStatus::Failed
            || runtime.status == TaskLifecycleStatus::Cancelled
        {
            return Err("LANDSAT_PC_CURRENT_STACK_RESUME_TERMINAL".to_owned());
        }
        if Instant::now() >= deadline {
            return Err("LANDSAT_PC_CURRENT_STACK_RESUME_NOT_OBSERVED".to_owned());
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn wait_for_completion(
    service: &ApplicationService,
    output_root: &Path,
    assets: &[FrozenAsset],
    active_observed: &mut bool,
    part_observed: &mut bool,
) -> Result<DownloadEngineSnapshot, String> {
    let deadline = Instant::now() + TRANSFER_WAIT;
    loop {
        let download = service
            .download_snapshot(TASK_ID)
            .map_err(|error| format!("LANDSAT_PC_CURRENT_STACK_COMPLETION_{}", error.code()))?;
        *active_observed |= download
            .progress
            .iter()
            .any(|progress| progress.status == DownloadItemStatus::Active);
        *part_observed |= any_part_file(output_root, assets);
        if download.results.iter().any(|result| {
            matches!(
                result.outcome,
                DownloadOutcome::Failed | DownloadOutcome::Cancelled | DownloadOutcome::Interrupted
            )
        }) {
            return Err("LANDSAT_PC_CURRENT_STACK_TRANSFER_FAILED".to_owned());
        }
        if download.results.len() == assets.len() {
            let runtime = task_runtime(service)?;
            if runtime.status == TaskLifecycleStatus::Completed
                && download
                    .results
                    .iter()
                    .all(|result| result.outcome == DownloadOutcome::Completed)
            {
                return Ok(download);
            }
        }
        if Instant::now() >= deadline {
            return Err("LANDSAT_PC_CURRENT_STACK_TRANSFER_TIMEOUT".to_owned());
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn observed_bytes(download: &DownloadEngineSnapshot) -> u64 {
    let progress = download
        .progress
        .iter()
        .map(|item| item.bytes_completed)
        .try_fold(0_u64, u64::checked_add)
        .unwrap_or(u64::MAX);
    let results = download
        .results
        .iter()
        .map(|item| item.bytes_written)
        .try_fold(0_u64, u64::checked_add)
        .unwrap_or(u64::MAX);
    progress.max(results)
}

fn any_part_file(output_root: &Path, assets: &[FrozenAsset]) -> bool {
    assets
        .iter()
        .any(|asset| part_path(&output_root.join(&asset.file_name)).is_file())
}

fn hash_outputs(
    output_root: &Path,
    assets: &[FrozenAsset],
    download: &DownloadEngineSnapshot,
) -> Result<Vec<HashedAsset>, String> {
    let expected_names = assets
        .iter()
        .map(|asset| asset.file_name.as_str())
        .collect::<BTreeSet<_>>();
    for entry in fs::read_dir(output_root)
        .map_err(|_| "LANDSAT_PC_CURRENT_STACK_OUTPUT_READ_FAILED".to_owned())?
    {
        let entry = entry.map_err(|_| "LANDSAT_PC_CURRENT_STACK_OUTPUT_READ_FAILED".to_owned())?;
        let name = entry
            .file_name()
            .to_str()
            .map(str::to_owned)
            .ok_or_else(|| "LANDSAT_PC_CURRENT_STACK_OUTPUT_FILENAME_INVALID".to_owned())?;
        if !expected_names.contains(name.as_str()) {
            return Err("LANDSAT_PC_CURRENT_STACK_OUTPUT_HAS_EXTRA_ENTRY".to_owned());
        }
    }
    let mut hashed = Vec::with_capacity(assets.len());
    for asset in assets {
        let result = download
            .results
            .iter()
            .find(|result| result.item_id == asset.item_id)
            .ok_or_else(|| "LANDSAT_PC_CURRENT_STACK_RESULT_MISSING".to_owned())?;
        let destination = output_root.join(&asset.file_name);
        if result.outcome != DownloadOutcome::Completed
            || result.bytes_written != asset.expected_size
            || result.destination != destination
            || part_path(&destination).exists()
        {
            return Err("LANDSAT_PC_CURRENT_STACK_OUTPUT_PROMOTION_INVALID".to_owned());
        }
        let (actual_size, sha256) = hash_regular_file(&destination)?;
        if actual_size != asset.expected_size {
            return Err("LANDSAT_PC_CURRENT_STACK_OUTPUT_SIZE_INVALID".to_owned());
        }
        hashed.push(HashedAsset {
            item_id: asset.item_id.clone(),
            file_name: asset.file_name.clone(),
            expected_size: asset.expected_size,
            actual_size,
            sha256,
        });
    }
    Ok(hashed)
}

fn hash_outputs_again(
    output_root: &Path,
    expected: &[HashedAsset],
) -> Result<Vec<HashedAsset>, String> {
    let expected_names = expected
        .iter()
        .map(|asset| asset.file_name.as_str())
        .collect::<BTreeSet<_>>();
    for entry in fs::read_dir(output_root)
        .map_err(|_| "LANDSAT_PC_CURRENT_STACK_OUTPUT_READ_FAILED".to_owned())?
    {
        let entry = entry.map_err(|_| "LANDSAT_PC_CURRENT_STACK_OUTPUT_READ_FAILED".to_owned())?;
        let name = entry
            .file_name()
            .to_str()
            .map(str::to_owned)
            .ok_or_else(|| "LANDSAT_PC_CURRENT_STACK_OUTPUT_FILENAME_INVALID".to_owned())?;
        if !expected_names.contains(name.as_str()) {
            return Err("LANDSAT_PC_CURRENT_STACK_OUTPUT_HAS_EXTRA_ENTRY".to_owned());
        }
    }
    let mut observed = Vec::with_capacity(expected.len());
    for asset in expected {
        let (actual_size, sha256) = hash_regular_file(&output_root.join(&asset.file_name))?;
        observed.push(HashedAsset {
            item_id: asset.item_id.clone(),
            file_name: asset.file_name.clone(),
            expected_size: asset.expected_size,
            actual_size,
            sha256,
        });
    }
    Ok(observed)
}

fn hash_regular_file(path: &Path) -> Result<(u64, String), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| "LANDSAT_PC_CURRENT_STACK_OUTPUT_MISSING".to_owned())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("LANDSAT_PC_CURRENT_STACK_OUTPUT_NOT_REGULAR".to_owned());
    }
    let mut file =
        File::open(path).map_err(|_| "LANDSAT_PC_CURRENT_STACK_OUTPUT_OPEN_FAILED".to_owned())?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; HASH_BUFFER_BYTES];
    let mut size = 0_u64;
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| "LANDSAT_PC_CURRENT_STACK_OUTPUT_READ_FAILED".to_owned())?;
        if read == 0 {
            break;
        }
        size = size
            .checked_add(read as u64)
            .ok_or_else(|| "LANDSAT_PC_CURRENT_STACK_SIZE_OVERFLOW".to_owned())?;
        hasher.update(&buffer[..read]);
    }
    Ok((size, format!("{:x}", hasher.finalize())))
}

fn runtime_details(
    runtime: &TaskRuntimeSnapshot,
    download: &DownloadEngineSnapshot,
    extra: Value,
) -> Value {
    json!({
        "status": serde_json::to_value(&runtime.status).unwrap_or(Value::String("unknown".to_owned())),
        "revision": runtime.revision,
        "task_paused": runtime.task_paused,
        "active_count": runtime.active_item_ids.len(),
        "paused_count": runtime.paused_item_ids.len(),
        "queued_count": runtime.queued_item_ids.len(),
        "completed_count": runtime.completed_item_ids.len(),
        "download_revision": download.revision,
        "download_result_count": download.results.len(),
        "observed_bytes": observed_bytes(download),
        "extra": extra
    })
}
