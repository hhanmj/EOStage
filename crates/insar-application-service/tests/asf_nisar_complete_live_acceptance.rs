#![cfg(feature = "nisar-subset-live-acceptance")]

use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use insar_application_service::{
    default_store_path, ApplicationBindRegionAoiRequest, ApplicationCatalogSearchAndFreezeRequest,
    ApplicationService, ApplicationServiceConfig, ApplicationServiceError,
};
use insar_core::{
    CatalogSearchQuery, DownloadItemStatus, DownloadLayout, DownloadOutcome,
    StoreCreateTaskRequest, StoreTaskCommand, StoreTaskPage, StoreTaskPageRequest,
    StoreTaskSourceBatchPageRequest, StoreUpdateTaskRequest, StoreUpdateWorkbenchRequest,
    TaskLifecycleStatus, TaskRuntimeSnapshot, TaskStateCommandKind,
};
use insar_download::{
    establish_hdf5_plugin_isolation, part_path, resolve_earthdata_credential,
    EarthdataCredentialSource,
};
use insar_provider::catalog::{
    NISAR_LOGICAL_DATASET, NISAR_NATIVE_COLLECTION_ID, NISAR_PROVIDER_ID,
};
use serde::Serialize;
use serde_json::{Map, Value};

mod common;

const LIVE_FLAG: &str = "INSAR_NISAR_COMPLETE_REAL_BYTE_LIVE";
const EVIDENCE_ROOT: &str = "_verification/nisar-complete-real-byte-20260826";
const TASK_ID: &str = "nisar-complete-real-byte-task";
const TRANSFER_DEADLINE: Duration = Duration::from_secs(120);
const CONTROL_DEADLINE: Duration = Duration::from_secs(30);

#[derive(Debug, Serialize)]
struct CompleteCanaryEvidence {
    schema_version: u64,
    attempt_id: String,
    outcome: String,
    stop_code: Option<String>,
    chain: Vec<String>,
    credential_available: bool,
    credential_material_written: bool,
    source_batch_id: Option<String>,
    source_product_id: Option<String>,
    primary_file_name: Option<String>,
    expected_size_bytes: Option<u64>,
    task_id: String,
    task_status: Option<String>,
    item_status: Option<String>,
    failure_code: Option<String>,
    failure_retryable: Option<bool>,
    observed_body_bytes: u64,
    paused_bytes: u64,
    resumed_bytes: u64,
    partial_bytes: u64,
    final_promoted: bool,
    source_page_verified: bool,
    task_page_verified: bool,
    restart_history_verified: bool,
}

impl CompleteCanaryEvidence {
    fn new(attempt_id: String) -> Self {
        Self {
            schema_version: 1,
            attempt_id,
            outcome: "blocked".to_owned(),
            stop_code: None,
            chain: [
                "ApplicationService",
                "production ASF CMR",
                "immutable SearchBatch",
                "Store",
                "scheduler",
                "ProductionWorker",
                "Earthdata HTTPS",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            credential_available: false,
            credential_material_written: false,
            source_batch_id: None,
            source_product_id: None,
            primary_file_name: None,
            expected_size_bytes: None,
            task_id: TASK_ID.to_owned(),
            task_status: None,
            item_status: None,
            failure_code: None,
            failure_retryable: None,
            observed_body_bytes: 0,
            paused_bytes: 0,
            resumed_bytes: 0,
            partial_bytes: 0,
            final_promoted: false,
            source_page_verified: false,
            task_page_verified: false,
            restart_history_verified: false,
        }
    }
}

fn require(condition: bool, code: &'static str) -> Result<(), String> {
    condition.then_some(()).ok_or_else(|| code.to_owned())
}

fn application_stop_code(error: ApplicationServiceError) -> String {
    match error {
        ApplicationServiceError::Provider(source) => source.code,
        other => other.code().to_owned(),
    }
}

fn task_status_label(status: &TaskLifecycleStatus) -> &'static str {
    match status {
        TaskLifecycleStatus::Queued => "queued",
        TaskLifecycleStatus::Running => "running",
        TaskLifecycleStatus::Paused => "paused",
        TaskLifecycleStatus::Completed => "completed",
        TaskLifecycleStatus::Failed => "failed",
        TaskLifecycleStatus::Cancelled => "cancelled",
        TaskLifecycleStatus::Interrupted => "interrupted",
    }
}

fn item_status_label(status: &DownloadItemStatus) -> &'static str {
    match status {
        DownloadItemStatus::Queued => "queued",
        DownloadItemStatus::Active => "active",
        DownloadItemStatus::Paused => "paused",
        DownloadItemStatus::Completed => "completed",
        DownloadItemStatus::Failed => "failed",
        DownloadItemStatus::Skipped => "skipped",
        DownloadItemStatus::Cancelled => "cancelled",
        DownloadItemStatus::Interrupted => "interrupted",
    }
}

fn evidence_is_secret_free(serialized: &str) -> bool {
    let lowered = serialized.to_ascii_lowercase();
    [
        "authorization:",
        "cookie:",
        "set-cookie",
        "username",
        "password",
        "bearer ",
        "basic ",
        "x-amz-",
        "signature=",
    ]
    .into_iter()
    .all(|forbidden| !lowered.contains(forbidden))
}

fn create_attempt() -> Result<(PathBuf, String), String> {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .map_err(|_| "NISAR_COMPLETE_REPOSITORY_ROOT_UNAVAILABLE".to_owned())?;
    let root = repository.join(EVIDENCE_ROOT);
    fs::create_dir_all(&root).map_err(|_| "NISAR_COMPLETE_EVIDENCE_ROOT_UNAVAILABLE".to_owned())?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "NISAR_COMPLETE_CLOCK_INVALID".to_owned())?
        .as_millis();
    let attempt_id = format!("attempt-{stamp}-{}", std::process::id());
    let attempt = root.join(&attempt_id);
    fs::create_dir(&attempt).map_err(|_| "NISAR_COMPLETE_ATTEMPT_ALREADY_EXISTS".to_owned())?;
    fs::create_dir(attempt.join("downloads"))
        .map_err(|_| "NISAR_COMPLETE_OUTPUT_ROOT_UNAVAILABLE".to_owned())?;
    Ok((attempt, attempt_id))
}

fn write_evidence(attempt: &Path, evidence: &CompleteCanaryEvidence) -> Result<(), String> {
    let serialized = serde_json::to_string_pretty(evidence)
        .map_err(|_| "NISAR_COMPLETE_EVIDENCE_SERIALIZATION_FAILED".to_owned())?;
    require(
        evidence_is_secret_free(&serialized),
        "NISAR_COMPLETE_EVIDENCE_SECRET_REJECTED",
    )?;
    fs::write(attempt.join("result.json"), format!("{serialized}\n"))
        .map_err(|_| "NISAR_COMPLETE_EVIDENCE_WRITE_FAILED".to_owned())?;
    let markdown = format!(
        "# NISAR Complete RSLC real-byte canary\n\n- Attempt: `{}`\n- Outcome: `{}`\n- Stop code: `{}`\n- Source batch: `{}`\n- Source product: `{}`\n- Primary file: `{}`\n- Expected size: `{}`\n- Task status: `{}`\n- Item status: `{}`\n- Failure code: `{}`\n- Failure retryable: `{}`\n- Observed bytes: `{}`\n- Paused bytes: `{}`\n- Resumed bytes: `{}`\n- Safe partial bytes: `{}`\n- Final promoted: `{}`\n- Source page verified: `{}`\n- Task page verified: `{}`\n- Restart history verified: `{}`\n- Credential material written: `false`\n",
        evidence.attempt_id,
        evidence.outcome,
        evidence.stop_code.as_deref().unwrap_or("none"),
        evidence.source_batch_id.as_deref().unwrap_or("unavailable"),
        evidence.source_product_id.as_deref().unwrap_or("unavailable"),
        evidence.primary_file_name.as_deref().unwrap_or("unavailable"),
        evidence
            .expected_size_bytes
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unavailable".to_owned()),
        evidence.task_status.as_deref().unwrap_or("unavailable"),
        evidence.item_status.as_deref().unwrap_or("unavailable"),
        evidence.failure_code.as_deref().unwrap_or("none"),
        evidence
            .failure_retryable
            .map(|value| if value { "true" } else { "false" })
            .unwrap_or("none"),
        evidence.observed_body_bytes,
        evidence.paused_bytes,
        evidence.resumed_bytes,
        evidence.partial_bytes,
        evidence.final_promoted,
        evidence.source_page_verified,
        evidence.task_page_verified,
        evidence.restart_history_verified,
    );
    require(
        evidence_is_secret_free(&markdown),
        "NISAR_COMPLETE_EVIDENCE_SECRET_REJECTED",
    )?;
    fs::write(attempt.join("README.md"), markdown)
        .map_err(|_| "NISAR_COMPLETE_EVIDENCE_WRITE_FAILED".to_owned())
}

fn task_runtime(
    service: &ApplicationService,
    task_id: &str,
) -> Result<TaskRuntimeSnapshot, String> {
    service
        .snapshot(true)
        .map_err(application_stop_code)?
        .restore
        .runtimes
        .into_iter()
        .find(|runtime| runtime.task_id == task_id)
        .ok_or_else(|| "NISAR_COMPLETE_TASK_RUNTIME_MISSING".to_owned())
}

fn task_page_latest(service: &ApplicationService, task_id: &str) -> Result<StoreTaskPage, String> {
    for _ in 0..100 {
        let revision = task_runtime(service, task_id)?.revision;
        match service.task_page(&StoreTaskPageRequest {
            task_id: task_id.to_owned(),
            expected_revision: revision,
            page_index: 0,
            page_size: 24,
        }) {
            Ok(page) => return Ok(page),
            Err(error) if error.code() == "V3_STORE_REVISION_CONFLICT" => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(application_stop_code(error)),
        }
    }
    Err("NISAR_COMPLETE_TASK_PAGE_REVISION_CONFLICT".to_owned())
}

fn update_item(
    service: &ApplicationService,
    item_id: &str,
    kind: TaskStateCommandKind,
    event: &str,
) -> Result<(), String> {
    for attempt in 0..200_u64 {
        let revision = task_runtime(service, TASK_ID)?.revision;
        match service.update_download_task(&StoreUpdateTaskRequest {
            task_id: TASK_ID.to_owned(),
            expected_revision: revision,
            command: StoreTaskCommand {
                kind: kind.clone(),
                item_ids: vec![item_id.to_owned()],
            },
            event_id: format!("nisar-complete-{event}-{attempt}"),
            occurred_at: "2026-08-26T13:00:00Z".to_owned(),
        }) {
            Ok(_) => return Ok(()),
            Err(error) if error.code() == "V3_STORE_REVISION_CONFLICT" => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(application_stop_code(error)),
        }
    }
    Err("NISAR_COMPLETE_UPDATE_REVISION_CONFLICT".to_owned())
}

fn wait_until(
    deadline: Duration,
    code: &'static str,
    mut condition: impl FnMut() -> Result<bool, String>,
) -> Result<(), String> {
    let started = Instant::now();
    loop {
        if condition()? {
            return Ok(());
        }
        require(started.elapsed() <= deadline, code)?;
        thread::sleep(Duration::from_millis(25));
    }
}

fn primary_asset_fields(item: &insar_core::SnapshotItem) -> Result<(String, Option<u64>), String> {
    let primary = item
        .metadata
        .get("assets")
        .and_then(Value::as_array)
        .and_then(|assets| {
            assets
                .iter()
                .find(|asset| asset.get("role").and_then(Value::as_str) == Some("primary"))
        })
        .ok_or_else(|| "NISAR_COMPLETE_PRIMARY_ASSET_MISSING".to_owned())?;
    let file_name = primary
        .get("file_name")
        .and_then(Value::as_str)
        .filter(|name| name.ends_with(".h5"))
        .map(str::to_owned)
        .ok_or_else(|| "NISAR_COMPLETE_PRIMARY_FILE_INVALID".to_owned())?;
    Ok((file_name, primary.get("size_bytes").and_then(Value::as_u64)))
}

fn bind_aoi(
    service: &ApplicationService,
    geometry: Map<String, Value>,
    bbox: Vec<f64>,
    display_name: String,
) -> Result<(), String> {
    let region_id = common::ensure_fixture_region(service, "nisar-complete-live");
    service
        .bind_region_aoi(&ApplicationBindRegionAoiRequest {
            request_id: "nisar-complete-live-bind-aoi".to_owned(),
            region_id,
            expected_revision: None,
            source_kind: "manual_bbox".to_owned(),
            geometry,
            bbox,
            crs: "EPSG:4326".to_owned(),
            display_name,
            bound_at: "2026-08-26T12:59:00Z".to_owned(),
        })
        .map(drop)
        .map_err(|error| format!("NISAR_COMPLETE_BIND_{}", application_stop_code(error)))
}

fn capture_terminal_evidence(
    service: &ApplicationService,
    attempt: &Path,
    evidence: &mut CompleteCanaryEvidence,
) -> Result<(), String> {
    let runtime = task_runtime(service, TASK_ID)?;
    evidence.task_status = Some(task_status_label(&runtime.status).to_owned());
    let page = task_page_latest(service, TASK_ID)?;
    let item = page
        .items
        .first()
        .ok_or_else(|| "NISAR_COMPLETE_TASK_ITEM_MISSING".to_owned())?;
    evidence.item_status = Some(item_status_label(&item.status).to_owned());
    evidence.observed_body_bytes = evidence.observed_body_bytes.max(item.bytes_completed);
    if let Some(failure) = item.failure.as_ref() {
        evidence.failure_code = Some(failure.code.clone());
        evidence.failure_retryable = Some(failure.retryable);
    }
    let destination = item.destination.clone().unwrap_or_else(|| {
        attempt.join("downloads").join(
            evidence
                .primary_file_name
                .as_deref()
                .unwrap_or("unknown.h5"),
        )
    });
    evidence.partial_bytes = fs::metadata(part_path(&destination))
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    evidence.final_promoted = destination.exists();
    evidence.task_page_verified = page.total_count == 1
        && page.items.len() == 1
        && item.item.item_id == evidence.source_product_id.as_deref().unwrap_or_default();
    Ok(())
}

fn run_live_chain(
    service: &ApplicationService,
    attempt: &Path,
    evidence: &mut CompleteCanaryEvidence,
    geometry: Map<String, Value>,
    bbox: Vec<f64>,
    display_name: String,
) -> Result<(), String> {
    resolve_earthdata_credential(EarthdataCredentialSource::Auto, None)
        .map(drop)
        .map_err(|error| error.code().to_owned())?;
    evidence.credential_available = true;
    establish_hdf5_plugin_isolation()
        .map_err(|_| "NISAR_COMPLETE_HDF5_ISOLATION_FAILED".to_owned())?;
    bind_aoi(service, geometry.clone(), bbox, display_name.clone())?;

    let request = ApplicationCatalogSearchAndFreezeRequest {
        request_id: "nisar-complete-live-search".to_owned(),
        search: CatalogSearchQuery {
            logical_dataset: NISAR_LOGICAL_DATASET.to_owned(),
            provider_id: NISAR_PROVIDER_ID.to_owned(),
            native_collection_id: NISAR_NATIVE_COLLECTION_ID.to_owned(),
            aoi_geojson: Some(geometry.clone()),
            cursor: None,
            result_limit: Some(1),
            filters: Map::new(),
        },
        batch_id: "nisar-complete-live-batch".to_owned(),
        created_at: "2026-08-26T13:00:00Z".to_owned(),
        aoi_name: display_name,
        aoi_geojson: Some(geometry),
        workbench_page_size: 24,
    };
    let frozen = service
        .catalog_search_and_freeze_legacy(&request)
        .map_err(|error| format!("NISAR_COMPLETE_SEARCH_{}", application_stop_code(error)))?;
    let batch = frozen
        .restore
        .workbench_batch
        .as_ref()
        .ok_or_else(|| "NISAR_COMPLETE_SOURCE_BATCH_MISSING".to_owned())?;
    let workbench = frozen
        .restore
        .workbench
        .as_ref()
        .ok_or_else(|| "NISAR_COMPLETE_WORKBENCH_MISSING".to_owned())?;
    require(
        batch.candidates.len() == 1,
        "NISAR_COMPLETE_SOURCE_COUNT_INVALID",
    )?;
    let product = batch.candidates[0].clone();
    require(
        product.provider_id == NISAR_PROVIDER_ID
            && product.asset_access.is_some()
            && product
                .metadata
                .get("logical_dataset")
                .and_then(Value::as_str)
                == Some(NISAR_LOGICAL_DATASET),
        "NISAR_COMPLETE_SOURCE_PRODUCT_INVALID",
    )?;
    let (file_name, size_bytes) = primary_asset_fields(&product)?;
    require(
        size_bytes.is_some_and(|size| size > 0),
        "NISAR_COMPLETE_PRIMARY_SIZE_INVALID",
    )?;
    evidence.source_batch_id = Some(batch.batch_id.clone());
    evidence.source_product_id = Some(product.item_id.clone());
    evidence.primary_file_name = Some(file_name);
    evidence.expected_size_bytes = size_bytes;

    service
        .update_search_workbench(&StoreUpdateWorkbenchRequest {
            state: insar_core::SearchWorkbenchState {
                batch_id: batch.batch_id.clone(),
                checked_item_ids: vec![product.item_id.clone()],
                page_index: 0,
                page_size: 24,
                revision: workbench.revision + 1,
            },
            expected_revision: workbench.revision,
            updated_at: "2026-08-26T13:00:01Z".to_owned(),
        })
        .map_err(application_stop_code)?;

    service
        .create_download_task_legacy(&StoreCreateTaskRequest {
            task_id: TASK_ID.to_owned(),
            batch_id: batch.batch_id.clone(),
            selected_item_ids: vec![product.item_id.clone()],
            checked_item_ids: vec![product.item_id.clone()],
            page_index: 0,
            page_size: 24,
            output_root: attempt.join("downloads"),
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-08-26T13:00:02Z".to_owned(),
        })
        .map_err(application_stop_code)?;

    let source_page = service
        .task_source_batch_page(&StoreTaskSourceBatchPageRequest {
            task_id: TASK_ID.to_owned(),
            page_index: 0,
            page_size: 24,
        })
        .map_err(application_stop_code)?;
    evidence.source_page_verified = source_page.source_batch_id == batch.batch_id
        && source_page.total_count == 1
        && source_page.items.len() == 1
        && source_page.items[0].belongs_to_task
        && source_page.items[0].item.item_id == product.item_id;
    require(
        evidence.source_page_verified,
        "NISAR_COMPLETE_SOURCE_PAGE_INVALID",
    )?;

    wait_until(
        TRANSFER_DEADLINE,
        "NISAR_COMPLETE_TRANSFER_DEADLINE_EXCEEDED",
        || {
            let download = service
                .download_snapshot(TASK_ID)
                .map_err(application_stop_code)?;
            if let Some(result) = download.results.first() {
                if result.outcome != DownloadOutcome::Completed {
                    if let Some(failure) = result.failure.as_ref() {
                        evidence.failure_code = Some(failure.code.clone());
                        evidence.failure_retryable = Some(failure.retryable);
                        return Err(failure.code.clone());
                    }
                    return Err("NISAR_COMPLETE_TRANSFER_TERMINATED_WITHOUT_BYTES".to_owned());
                }
            }
            let observed = download
                .progress
                .iter()
                .filter(|progress| progress.item_id == product.item_id)
                .map(|progress| progress.bytes_completed)
                .max()
                .unwrap_or(0);
            evidence.observed_body_bytes = evidence.observed_body_bytes.max(observed);
            Ok(evidence.observed_body_bytes > 0)
        },
    )?;

    update_item(
        service,
        &product.item_id,
        TaskStateCommandKind::PauseItems,
        "pause",
    )?;
    wait_until(
        CONTROL_DEADLINE,
        "NISAR_COMPLETE_PAUSE_DEADLINE_EXCEEDED",
        || {
            Ok(task_runtime(service, TASK_ID)?
                .paused_item_ids
                .contains(&product.item_id))
        },
    )?;
    let paused_page = task_page_latest(service, TASK_ID)?;
    let destination = paused_page.items[0].destination.clone().unwrap_or_else(|| {
        attempt.join("downloads").join(
            evidence
                .primary_file_name
                .as_deref()
                .unwrap_or("unknown.h5"),
        )
    });
    let partial = part_path(&destination);
    wait_until(
        CONTROL_DEADLINE,
        "NISAR_COMPLETE_PAUSED_PART_MISSING",
        || Ok(fs::metadata(&partial).is_ok_and(|metadata| metadata.len() > 0)),
    )?;
    thread::sleep(Duration::from_millis(250));
    let paused_bytes = fs::metadata(&partial)
        .map_err(|_| "NISAR_COMPLETE_PAUSED_PART_MISSING".to_owned())?
        .len();
    thread::sleep(Duration::from_millis(250));
    require(
        fs::metadata(&partial)
            .map(|metadata| metadata.len())
            .unwrap_or(0)
            == paused_bytes,
        "NISAR_COMPLETE_PAUSE_NOT_STABLE",
    )?;
    evidence.paused_bytes = paused_bytes;

    update_item(
        service,
        &product.item_id,
        TaskStateCommandKind::ResumeItems,
        "resume",
    )?;
    wait_until(
        TRANSFER_DEADLINE,
        "NISAR_COMPLETE_RESUME_DEADLINE_EXCEEDED",
        || {
            let bytes = fs::metadata(&partial)
                .map(|metadata| metadata.len())
                .unwrap_or(0);
            evidence.resumed_bytes = evidence.resumed_bytes.max(bytes);
            Ok(bytes > paused_bytes)
        },
    )?;

    update_item(
        service,
        &product.item_id,
        TaskStateCommandKind::CancelItems,
        "cancel",
    )?;
    wait_until(
        CONTROL_DEADLINE,
        "NISAR_COMPLETE_CANCEL_DEADLINE_EXCEEDED",
        || {
            let download = service
                .download_snapshot(TASK_ID)
                .map_err(application_stop_code)?;
            Ok(download
                .results
                .iter()
                .any(|result| result.outcome == DownloadOutcome::Cancelled))
        },
    )?;

    let runtime = task_runtime(service, TASK_ID)?;
    require(
        runtime.status == TaskLifecycleStatus::Cancelled
            && runtime.cancelled_item_ids == [product.item_id.clone()],
        "NISAR_COMPLETE_CANCELLED_RUNTIME_INVALID",
    )?;
    let page = task_page_latest(service, TASK_ID)?;
    require(
        page.total_count == 1
            && page.items.len() == 1
            && page.items[0].status == DownloadItemStatus::Cancelled
            && page.items[0].bytes_completed > 0,
        "NISAR_COMPLETE_CANCELLED_TASK_PAGE_INVALID",
    )?;
    let partial_bytes = fs::metadata(&partial)
        .map_err(|_| "NISAR_COMPLETE_SAFE_PART_MISSING".to_owned())?
        .len();
    require(partial_bytes > 0, "NISAR_COMPLETE_SAFE_PART_EMPTY")?;
    require(!destination.exists(), "NISAR_COMPLETE_FINAL_PROMOTED")?;
    evidence.observed_body_bytes = evidence
        .observed_body_bytes
        .max(page.items[0].bytes_completed);
    evidence.partial_bytes = partial_bytes;
    evidence.final_promoted = false;
    evidence.task_status = Some("cancelled".to_owned());
    evidence.item_status = Some("cancelled".to_owned());
    evidence.task_page_verified = true;
    Ok(())
}

#[test]
fn nisar_complete_live_evidence_contract_is_offline_and_secret_free() {
    let evidence = CompleteCanaryEvidence::new("attempt-offline".to_owned());
    let serialized = serde_json::to_string(&evidence).unwrap();
    assert!(evidence_is_secret_free(&serialized));
    assert!(!evidence.credential_material_written);
    assert_eq!(
        evidence.chain.last().map(String::as_str),
        Some("Earthdata HTTPS")
    );
}

#[test]
#[ignore = "explicit bounded production ApplicationService/CMR/Store/scheduler/ProductionWorker NISAR Complete real-byte canary"]
fn production_nisar_complete_real_bytes_pause_resume_cancel_and_restore_history() {
    assert_eq!(
        std::env::var(LIVE_FLAG).ok().as_deref(),
        Some("1"),
        "NISAR_COMPLETE_REAL_BYTE outcome=blocked code=LIVE_INPUT_REQUIRED"
    );
    let (attempt, attempt_id) = create_attempt().expect("bounded new NISAR Complete attempt");
    let mut evidence = CompleteCanaryEvidence::new(attempt_id);

    let desktop = ApplicationService::new(ApplicationServiceConfig::store(default_store_path()));
    let aoi = desktop
        .working_aoi()
        .expect("desktop working AOI remains readable")
        .expect("desktop working AOI exists");
    desktop
        .shutdown()
        .expect("desktop read-only service shutdown");

    let store_path = attempt.join("tasks.sqlite3");
    let service = ApplicationService::new(ApplicationServiceConfig::business_store(&store_path));
    let result = run_live_chain(
        &service,
        &attempt,
        &mut evidence,
        aoi.geometry,
        aoi.bbox.to_vec(),
        aoi.display_name,
    );
    if evidence.source_product_id.is_some() {
        let _ = capture_terminal_evidence(&service, &attempt, &mut evidence);
    }
    let shutdown_result = service.shutdown().map_err(application_stop_code);
    let result = result.and(shutdown_result);

    if result.is_ok() {
        let restored =
            ApplicationService::new(ApplicationServiceConfig::business_store(&store_path));
        let snapshot = restored
            .snapshot(true)
            .map_err(application_stop_code)
            .expect("cancelled NISAR Complete history remains readable after restart");
        evidence.restart_history_verified = snapshot
            .restore
            .tasks
            .iter()
            .any(|task| task.task_id == TASK_ID)
            && snapshot.restore.runtimes.iter().any(|runtime| {
                runtime.task_id == TASK_ID && runtime.status == TaskLifecycleStatus::Cancelled
            });
        restored.shutdown().expect("restored service shutdown");
    }

    match &result {
        Ok(()) if evidence.restart_history_verified => {
            evidence.outcome = "passed".to_owned();
            evidence.stop_code = None;
        }
        Ok(()) => {
            evidence.outcome = "blocked".to_owned();
            evidence.stop_code = Some("NISAR_COMPLETE_RESTART_HISTORY_INVALID".to_owned());
        }
        Err(code) => {
            evidence.outcome = "blocked".to_owned();
            evidence.stop_code = Some(code.clone());
        }
    }
    write_evidence(&attempt, &evidence).expect("secret-free NISAR Complete evidence write");
    eprintln!(
        "NISAR_COMPLETE_REAL_BYTE outcome={} code={} evidence={} paused_bytes={} resumed_bytes={} partial_bytes={} final_promoted={}",
        evidence.outcome,
        evidence.stop_code.as_deref().unwrap_or("none"),
        attempt.display(),
        evidence.paused_bytes,
        evidence.resumed_bytes,
        evidence.partial_bytes,
        evidence.final_promoted,
    );
    assert_eq!(
        evidence.outcome,
        "passed",
        "NISAR_COMPLETE_REAL_BYTE outcome=blocked code={} evidence={}",
        evidence
            .stop_code
            .as_deref()
            .unwrap_or("NISAR_COMPLETE_UNKNOWN"),
        attempt.display(),
    );
}
