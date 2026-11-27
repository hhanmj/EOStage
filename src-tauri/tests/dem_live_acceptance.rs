//! Opt-in end-to-end acceptance for one real, anonymous Copernicus DEM task.
//!
//! The test deliberately uses the normal desktop `ApplicationServiceState` and
//! product-owned lazy native GIS engine. It is ignored so ordinary test runs do
//! not contact a provider. Run it only with a fresh absolute workspace root and
//! an explicit packaged GIS runtime, for example in PowerShell:
//!
//! ```text
//! $env:INSAR_DEM_LIVE_ACCEPTANCE = "1"
//! $env:INSAR_DEM_LIVE_ROOT = Join-Path $env:TEMP "insar-dem-cop90-live"
//! $env:INSAR_NATIVE_GIS_RUNTIME_ROOT = "<absolute packaged native GIS runtime root>"
//! rtk cargo test -p eostage-desktop --test dem_live_acceptance -- --ignored --nocapture
//! ```
//!
//! `INSAR_DEM_LIVE_ROOT` must not exist before the run. Set
//! `INSAR_NATIVE_GIS_RUNTIME_ROOT` to the absolute product-owned GIS runtime
//! directory whose immediate child is `manifest.json`; this is the packaged
//! runtime root discovered beside the desktop distribution, not a Python
//! environment or a `PATH` lookup. The harness never starts a compatibility
//! worker. The isolated root and redacted JSON
//! evidence are preserved on both success and failure. Set
//! `INSAR_DEM_LIVE_CLEANUP=1` only when a successful run should remove the
//! isolated root after its evidence has been written. Otherwise inspect the
//! root's `evidence/dem-live-acceptance.json` together with `tasks.sqlite3` and
//! `output/COP90_dem.tif`.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use eostage_desktop::dem_live_acceptance_support::{
    state as new_application_state, ApplicationServiceState,
};
use insar_application_service::{
    ApplicationServiceError, ApplicationSetWorkingAoiRequest, CoreServiceSnapshot,
    DemTaskOperation, ServiceStatus, StandaloneDemAoiTaskRequest, StandaloneDemRequest,
    StoreDemConversionSourceRequest, StoreTaskHistoryPageRequest,
};
use insar_core::{
    DownloadItemStatus, StoreArchiveTaskRequest, StoreTaskPageRequest, TaskLifecycleStatus,
};
use serde::Serialize;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

const ENABLE_ENV: &str = "INSAR_DEM_LIVE_ACCEPTANCE";
const ROOT_ENV: &str = "INSAR_DEM_LIVE_ROOT";
const RUNTIME_ENV: &str = "INSAR_NATIVE_GIS_RUNTIME_ROOT";
const CLEANUP_ENV: &str = "INSAR_DEM_LIVE_CLEANUP";

const DATASET: &str = "COP90";
const PROVIDER: &str = "COPERNICUS_AWS";
const RESOURCE_ID: &str = "dem.cop90.v1";
const CAPABILITY_ID: &str = "capability.dem.cop90.aws-download.v1";
const REGION_ID: &str = "standalone-dem-aoi";
const AOI_NAME: &str = "DEM live acceptance AOI";
const TASK_ID: &str = "dem-live-cop90";
const REQUEST_ID: &str = "dem-live-cop90-create";
const CREATED_AT: &str = "2026-09-05T00:00:00Z";
const ARCHIVED_AT: &str = "2026-09-05T00:05:00Z";
const BBOX: [f64; 4] = [100.0, 30.0, 100.01, 30.01];
const POLL_INTERVAL: Duration = Duration::from_secs(2);
const TASK_TIMEOUT: Duration = Duration::from_secs(45 * 60);

#[test]
#[ignore = "requires explicit anonymous Copernicus AWS DEM transfer and a packaged native GIS runtime"]
fn dem_cop90_native_live_acceptance() {
    match run_live() {
        Ok(report) => {
            eprintln!("DEM_COP90_NATIVE_LIVE outcome=completed");
            eprintln!("dataset={DATASET}");
            eprintln!("provider={PROVIDER}");
            eprintln!("input_bytes={}", report.input_bytes);
            eprintln!("width={}", report.width);
            eprintln!("height={}", report.height);
            eprintln!("task_revision={}", report.task_revision);
            eprintln!("native_inspect=true");
            eprintln!("artifact_sha256={}", report.artifact_sha256);
            eprintln!("active_restore_excludes=true");
            eprintln!("history_contains=true");
            eprintln!("conversion_source_exact=true");
            eprintln!(
                "acceptance_workspace_preserved={}",
                !report.workspace_cleaned
            );
            eprintln!("acceptance_workspace_cleaned={}", report.workspace_cleaned);
        }
        Err(code) => {
            eprintln!("DEM_COP90_NATIVE_LIVE outcome=stopped");
            eprintln!("code={code}");
            panic!("DEM_COP90_NATIVE_LIVE_STOPPED");
        }
    }
}

struct LiveReport {
    input_bytes: u64,
    width: u64,
    height: u64,
    task_revision: u64,
    artifact_sha256: String,
    workspace_cleaned: bool,
}

/// Owns the caller-designated, previously absent live-acceptance root.
///
/// The root is an evidence boundary. It is retained after both successful and
/// failed runs so Store/output state and the compact redacted report remain
/// inspectable. Cleanup is an explicit success-only opt-in.
struct LiveAcceptanceWorkspace {
    root: PathBuf,
}

impl LiveAcceptanceWorkspace {
    fn create(root: PathBuf) -> Result<Self, String> {
        if !root.is_absolute() {
            return Err("DEM_COP90_LIVE_ROOT_NOT_ABSOLUTE".to_owned());
        }
        if root.exists() {
            return Err("DEM_COP90_LIVE_ROOT_OCCUPIED".to_owned());
        }
        fs::create_dir_all(root.join("output"))
            .map_err(|_| "DEM_COP90_LIVE_ROOT_UNAVAILABLE".to_owned())?;
        Ok(Self { root })
    }

    fn output_root(&self) -> PathBuf {
        self.root.join("output")
    }

    fn database(&self) -> PathBuf {
        self.root.join("tasks.sqlite3")
    }

    fn evidence_path(&self) -> PathBuf {
        self.root.join("evidence").join("dem-live-acceptance.json")
    }

    fn write_evidence(&self, evidence: &LiveEvidence) -> Result<(), String> {
        fs::create_dir_all(self.root.join("evidence"))
            .map_err(|_| "DEM_COP90_LIVE_EVIDENCE_DIRECTORY_FAILED".to_owned())?;
        let bytes = serde_json::to_vec(evidence)
            .map_err(|_| "DEM_COP90_LIVE_EVIDENCE_SERIALIZE_FAILED".to_owned())?;
        fs::write(self.evidence_path(), bytes)
            .map_err(|_| "DEM_COP90_LIVE_EVIDENCE_WRITE_FAILED".to_owned())
    }

    fn cleanup_now(&self) -> Result<(), String> {
        fs::remove_dir_all(&self.root).map_err(|_| "DEM_COP90_LIVE_CLEANUP_FAILED".to_owned())?;
        if self.root.exists() {
            return Err("DEM_COP90_LIVE_CLEANUP_INCOMPLETE".to_owned());
        }
        Ok(())
    }
}

#[derive(Debug, Serialize)]
struct LiveEvidence {
    schema_version: u8,
    outcome: String,
    stage: String,
    failure_code: Option<String>,
    dataset: String,
    provider: String,
    resource_id: String,
    capability_id: String,
    region_id: String,
    task_id: String,
    aoi_bbox: [f64; 4],
    antimeridian: bool,
    task: Option<TaskEvidenceReport>,
    artifact: Option<ArtifactEvidence>,
    result: Option<NativeResultEvidence>,
    archive: ArchiveEvidence,
    reopen: ReopenEvidence,
    workspace_cleanup: String,
}

#[derive(Debug, Serialize)]
struct TaskEvidenceReport {
    status: String,
    revision: u64,
    selected_count: usize,
    completed_count: usize,
    operation_id: String,
    kind: String,
    phase: String,
    region_id: String,
    approved_worker_operation: String,
    output_name: String,
}

#[derive(Debug, Serialize)]
struct ArtifactEvidence {
    name: String,
    relative_path: String,
    bytes: u64,
    sha256: String,
    native_inspect: NativeInspectEvidence,
}

#[derive(Debug, Serialize)]
struct NativeResultEvidence {
    ok: bool,
    native_inspect: NativeInspectEvidence,
}

#[derive(Clone, Debug, Serialize)]
struct NativeInspectEvidence {
    readable: bool,
    size_bytes: u64,
    driver: String,
    count: u64,
    width: u64,
    height: u64,
    crs: String,
}

#[derive(Debug, Default, Serialize)]
struct ArchiveEvidence {
    attempted: bool,
    archived: bool,
    revision: Option<u64>,
    archived_at: Option<String>,
}

#[derive(Debug, Default, Serialize)]
struct ReopenEvidence {
    attempted: bool,
    store_ready: Option<bool>,
    active_restore_excludes: Option<bool>,
    history_contains: Option<bool>,
    conversion_source_exact: Option<bool>,
}

impl LiveEvidence {
    fn new() -> Self {
        Self {
            schema_version: 1,
            outcome: "running".to_owned(),
            stage: "startup".to_owned(),
            failure_code: None,
            dataset: DATASET.to_owned(),
            provider: PROVIDER.to_owned(),
            resource_id: RESOURCE_ID.to_owned(),
            capability_id: CAPABILITY_ID.to_owned(),
            region_id: REGION_ID.to_owned(),
            task_id: TASK_ID.to_owned(),
            aoi_bbox: BBOX,
            antimeridian: false,
            task: None,
            artifact: None,
            result: None,
            archive: ArchiveEvidence::default(),
            reopen: ReopenEvidence::default(),
            workspace_cleanup: "preserved".to_owned(),
        }
    }

    fn set_stage(&mut self, stage: &str) {
        self.stage = stage.to_owned();
    }

    fn mark_failed(&mut self, code: &str) {
        self.outcome = "failed".to_owned();
        self.failure_code = Some(code.to_owned());
        self.workspace_cleanup = "preserved_after_failure".to_owned();
    }

    fn mark_completed(&mut self) {
        self.outcome = "completed".to_owned();
        self.stage = "completed".to_owned();
        self.failure_code = None;
    }
}

#[derive(Debug)]
struct TaskEvidence {
    status: TaskLifecycleStatus,
    revision: u64,
    selected_count: usize,
    completed_count: usize,
    output_root: PathBuf,
    operation_id: String,
    kind: String,
    phase: String,
    region_id: String,
    approved_worker_operation: String,
}

fn run_live() -> Result<LiveReport, String> {
    require_live_gate()?;
    let runtime_root = native_runtime_root()?;
    let root = std::env::var_os(ROOT_ENV)
        .map(PathBuf::from)
        .ok_or_else(|| "DEM_COP90_LIVE_ROOT_REQUIRED".to_owned())?;
    let workspace = LiveAcceptanceWorkspace::create(root)?;
    let output_root = workspace.output_root();
    let database = workspace.database();
    let geometry = acceptance_geometry();
    let mut evidence = LiveEvidence::new();
    workspace.write_evidence(&evidence)?;

    let result = (|| -> Result<LiveReport, String> {
        evidence.set_stage("initial_service_start");
        workspace.write_evidence(&evidence)?;
        let (initial_result, initial_shutdown) = {
            let state = new_application_state(database.clone(), runtime_root)?;
            let result = run_download_and_archive(
                &state,
                &output_root,
                &geometry,
                &workspace,
                &mut evidence,
            );
            let shutdown = state
                .shutdown()
                .map_err(|error| service_error("DEM_COP90_LIVE_INITIAL_SHUTDOWN", &error));
            (result, shutdown)
        };
        initial_shutdown?;
        let report = initial_result?;

        evidence.set_stage("reopen_service_start");
        workspace.write_evidence(&evidence)?;
        let (reopen_result, reopen_shutdown) = {
            let state = new_application_state(database, native_runtime_root()?)?;
            let result =
                verify_reopened_state(&state, &output_root, &report, &workspace, &mut evidence);
            let shutdown = state
                .shutdown()
                .map_err(|error| service_error("DEM_COP90_LIVE_REOPEN_SHUTDOWN", &error));
            (result, shutdown)
        };
        reopen_shutdown?;
        reopen_result?;
        Ok(report)
    })();

    match result {
        Ok(mut report) => {
            evidence.mark_completed();
            if cleanup_requested() {
                evidence.workspace_cleanup = "cleaned".to_owned();
                if let Err(error) = workspace.write_evidence(&evidence) {
                    evidence.mark_failed(&redacted_failure_code(&error));
                    let _ = workspace.write_evidence(&evidence);
                    return Err(error);
                }
                if let Err(error) = workspace.cleanup_now() {
                    evidence.mark_failed(&redacted_failure_code(&error));
                    let _ = workspace.write_evidence(&evidence);
                    return Err(error);
                }
                report.workspace_cleaned = true;
            } else {
                evidence.workspace_cleanup = "preserved".to_owned();
                if let Err(error) = workspace.write_evidence(&evidence) {
                    evidence.mark_failed(&redacted_failure_code(&error));
                    let _ = workspace.write_evidence(&evidence);
                    return Err(error);
                }
                report.workspace_cleaned = false;
            }
            Ok(report)
        }
        Err(error) => {
            evidence.mark_failed(&redacted_failure_code(&error));
            let _ = workspace.write_evidence(&evidence);
            Err(error)
        }
    }
}

fn task_evidence_report(evidence: &TaskEvidence) -> TaskEvidenceReport {
    TaskEvidenceReport {
        status: format!("{:?}", evidence.status),
        revision: evidence.revision,
        selected_count: evidence.selected_count,
        completed_count: evidence.completed_count,
        operation_id: evidence.operation_id.clone(),
        kind: evidence.kind.clone(),
        phase: evidence.phase.clone(),
        region_id: evidence.region_id.clone(),
        approved_worker_operation: evidence.approved_worker_operation.clone(),
        output_name: "COP90_dem.tif".to_owned(),
    }
}

fn record_stage(
    workspace: &LiveAcceptanceWorkspace,
    evidence: &mut LiveEvidence,
    stage: &str,
) -> Result<(), String> {
    evidence.set_stage(stage);
    workspace.write_evidence(evidence)
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file =
        fs::File::open(path).map_err(|_| "DEM_COP90_LIVE_OUTPUT_HASH_OPEN_FAILED".to_owned())?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|_| "DEM_COP90_LIVE_OUTPUT_HASH_READ_FAILED".to_owned())?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn cleanup_requested() -> bool {
    matches!(std::env::var(CLEANUP_ENV).as_deref(), Ok("1"))
}

fn redacted_failure_code(error: &str) -> String {
    let candidate = error
        .split(|character: char| {
            !(character.is_ascii_alphanumeric() || character == '_' || character == '-')
        })
        .next()
        .unwrap_or_default();
    if candidate.is_empty() {
        "DEM_COP90_LIVE_FAILED".to_owned()
    } else {
        candidate.to_owned()
    }
}

fn require_live_gate() -> Result<(), String> {
    match std::env::var(ENABLE_ENV) {
        Ok(value) if value == "1" => Ok(()),
        Ok(_) => Err("DEM_COP90_LIVE_ENABLE_MUST_EQUAL_1".to_owned()),
        Err(_) => Err("DEM_COP90_LIVE_ENABLE_REQUIRED".to_owned()),
    }
}

fn native_runtime_root() -> Result<PathBuf, String> {
    let configured = std::env::var_os(RUNTIME_ENV)
        .map(PathBuf::from)
        .ok_or_else(|| "DEM_COP90_LIVE_NATIVE_GIS_ROOT_REQUIRED".to_owned())?;
    if !configured.is_absolute() {
        return Err("DEM_COP90_LIVE_NATIVE_GIS_ROOT_NOT_ABSOLUTE".to_owned());
    }
    let root = fs::canonicalize(&configured)
        .map_err(|_| "DEM_COP90_LIVE_NATIVE_GIS_ROOT_UNREADABLE".to_owned())?;
    if !root.join("manifest.json").is_file() {
        return Err("DEM_COP90_LIVE_NATIVE_GIS_MANIFEST_REQUIRED".to_owned());
    }
    Ok(root)
}

fn acceptance_geometry() -> Map<String, Value> {
    json!({
        "type": "Polygon",
        "coordinates": [[
            [BBOX[0], BBOX[1]],
            [BBOX[2], BBOX[1]],
            [BBOX[2], BBOX[3]],
            [BBOX[0], BBOX[3]],
            [BBOX[0], BBOX[1]]
        ]]
    })
    .as_object()
    .expect("acceptance polygon is an object")
    .clone()
}

fn run_download_and_archive(
    state: &ApplicationServiceState,
    output_root: &Path,
    geometry: &Map<String, Value>,
    workspace: &LiveAcceptanceWorkspace,
    evidence: &mut LiveEvidence,
) -> Result<LiveReport, String> {
    let readiness = state
        .readiness()
        .map_err(|error| service_error("DEM_COP90_LIVE_READINESS", &error))?;
    if readiness.status != ServiceStatus::StoreReady || readiness.store_schema_version.is_none() {
        return Err("DEM_COP90_LIVE_STORE_NOT_READY".to_owned());
    }
    record_stage(workspace, evidence, "initial_store_ready")?;

    state
        .set_working_aoi(&ApplicationSetWorkingAoiRequest {
            source_kind: "manual_polygon".to_owned(),
            geometry: geometry.clone(),
            bbox: BBOX.to_vec(),
            crs: "EPSG:4326".to_owned(),
            display_name: AOI_NAME.to_owned(),
        })
        .map_err(|error| service_error("DEM_COP90_LIVE_AOI", &error))?;
    record_stage(workspace, evidence, "aoi_persisted")?;

    let created = state
        .create_standalone_dem_task(&StandaloneDemAoiTaskRequest {
            request_id: REQUEST_ID.to_owned(),
            task_id: TASK_ID.to_owned(),
            dataset: DATASET.to_owned(),
            provider: PROVIDER.to_owned(),
            aoi_geojson: geometry.clone(),
            aoi_name: AOI_NAME.to_owned(),
            output_root: output_root.to_path_buf(),
            convert: false,
            concurrency: 1,
            created_at: CREATED_AT.to_owned(),
        })
        .map_err(|error| service_error("DEM_COP90_LIVE_TASK_CREATE", &error))?;
    let created_task = task_evidence(&created)?;
    assert_task_operation(&created_task, output_root)?;
    if created_task.selected_count != 1 {
        return Err("DEM_COP90_LIVE_TASK_SELECTION_INVALID".to_owned());
    }
    evidence.task = Some(task_evidence_report(&created_task));
    record_stage(workspace, evidence, "task_created")?;

    let completed = wait_for_completion(state, output_root)?;
    if completed.selected_count != 1 || completed.completed_count != 1 {
        return Err("DEM_COP90_LIVE_TASK_COMPLETION_INVALID".to_owned());
    }
    evidence.task = Some(task_evidence_report(&completed));
    record_stage(workspace, evidence, "task_completed")?;
    let input_path = output_root.join("COP90_dem.tif");
    let input_bytes = fs::metadata(&input_path)
        .map_err(|_| "DEM_COP90_LIVE_OUTPUT_MISSING".to_owned())?
        .len();
    if input_bytes == 0 {
        return Err("DEM_COP90_LIVE_OUTPUT_EMPTY".to_owned());
    }
    record_stage(workspace, evidence, "artifact_downloaded")?;

    let page = state
        .task_page(&StoreTaskPageRequest {
            task_id: TASK_ID.to_owned(),
            expected_revision: completed.revision,
            page_index: 0,
            page_size: 10,
        })
        .map_err(|error| service_error("DEM_COP90_LIVE_TASK_PAGE", &error))?;
    if page.total_count != 1 || page.items.len() != 1 {
        return Err("DEM_COP90_LIVE_TASK_PAGE_COUNT_INVALID".to_owned());
    }
    let item = &page.items[0];
    if item.status != DownloadItemStatus::Completed
        || item.bytes_completed == 0
        || item.bytes_completed != input_bytes
        || item.destination.as_deref() != Some(input_path.as_path())
    {
        return Err("DEM_COP90_LIVE_TASK_PAGE_OUTPUT_INVALID".to_owned());
    }
    record_stage(workspace, evidence, "task_page_verified")?;

    let inspection = state
        .standalone_dem(&StandaloneDemRequest {
            request_id: "dem-live-cop90-inspect".to_owned(),
            dataset: DATASET.to_owned(),
            provider: PROVIDER.to_owned(),
            operation: DemTaskOperation::Inspect,
            input_path: input_path.clone(),
            output_root: output_root.to_path_buf(),
            bbox: Some(BBOX.to_vec()),
            source_vertical_datum: Some("EGM2008".to_owned()),
            target_vertical_datum: Some("WGS84_ELLIPSOID".to_owned()),
            output_mode: Some("sarscape".to_owned()),
        })
        .map_err(|error| service_error("DEM_COP90_LIVE_NATIVE_INSPECT", &error))?;
    let native_inspect = assert_native_inspection(&inspection, &input_path, input_bytes)?;
    let artifact_sha256 = sha256_file(&input_path)?;
    evidence.artifact = Some(ArtifactEvidence {
        name: "COP90_dem.tif".to_owned(),
        relative_path: "output/COP90_dem.tif".to_owned(),
        bytes: input_bytes,
        sha256: artifact_sha256.clone(),
        native_inspect: native_inspect.clone(),
    });
    evidence.result = Some(NativeResultEvidence {
        ok: true,
        native_inspect: native_inspect.clone(),
    });
    record_stage(workspace, evidence, "native_inspect_completed")?;

    evidence.archive.attempted = true;
    evidence.archive.revision = Some(completed.revision);
    evidence.archive.archived_at = Some(ARCHIVED_AT.to_owned());
    record_stage(workspace, evidence, "archive_requested")?;
    let archive = state
        .archive_download_task(&StoreArchiveTaskRequest {
            task_id: TASK_ID.to_owned(),
            expected_revision: completed.revision,
            archived_at: ARCHIVED_AT.to_owned(),
        })
        .map_err(|error| service_error("DEM_COP90_LIVE_ARCHIVE", &error))?;
    if archive.task_id != TASK_ID || archive.archived_at != ARCHIVED_AT {
        return Err("DEM_COP90_LIVE_ARCHIVE_ACK_INVALID".to_owned());
    }
    evidence.archive.archived = true;
    record_stage(workspace, evidence, "archived")?;

    Ok(LiveReport {
        input_bytes,
        width: native_inspect.width,
        height: native_inspect.height,
        task_revision: completed.revision,
        artifact_sha256,
        workspace_cleaned: false,
    })
}

fn wait_for_completion(
    state: &ApplicationServiceState,
    output_root: &Path,
) -> Result<TaskEvidence, String> {
    let deadline = Instant::now() + TASK_TIMEOUT;
    loop {
        let snapshot = state
            .core_restore(true)
            .map_err(|error| service_error("DEM_COP90_LIVE_CORE_RESTORE", &error))?;
        let evidence = task_evidence(&snapshot)?;
        assert_task_operation(&evidence, output_root)?;
        match evidence.status {
            TaskLifecycleStatus::Completed => return Ok(evidence),
            TaskLifecycleStatus::Failed
            | TaskLifecycleStatus::Cancelled
            | TaskLifecycleStatus::Interrupted => {
                return Err(format!(
                    "DEM_COP90_LIVE_TASK_TERMINAL_{:?}",
                    evidence.status
                ));
            }
            TaskLifecycleStatus::Queued
            | TaskLifecycleStatus::Running
            | TaskLifecycleStatus::Paused => {}
        }
        if Instant::now() >= deadline {
            return Err(format!("DEM_COP90_LIVE_TASK_TIMEOUT_{:?}", evidence.status));
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn assert_native_inspection(
    inspection: &Value,
    input_path: &Path,
    expected_bytes: u64,
) -> Result<NativeInspectEvidence, String> {
    if inspection.get("ok") != Some(&Value::Bool(true))
        || inspection.get("native") != Some(&Value::Bool(true))
    {
        return Err("DEM_COP90_LIVE_NATIVE_INSPECT_ENVELOPE_INVALID".to_owned());
    }
    let result = inspection
        .get("result")
        .and_then(Value::as_object)
        .ok_or_else(|| "DEM_COP90_LIVE_NATIVE_INSPECT_RESULT_MISSING".to_owned())?;
    if result.get("ok") != Some(&Value::Bool(true))
        || result.get("readable") != Some(&Value::Bool(true))
        || result.get("size_bytes").and_then(Value::as_u64) != Some(expected_bytes)
    {
        return Err("DEM_COP90_LIVE_NATIVE_INSPECT_RESULT_INVALID".to_owned());
    }
    let metadata = result
        .get("metadata")
        .and_then(Value::as_object)
        .ok_or_else(|| "DEM_COP90_LIVE_NATIVE_INSPECT_METADATA_MISSING".to_owned())?;
    let driver = metadata
        .get("driver")
        .and_then(Value::as_str)
        .filter(|value| *value == "GTiff")
        .ok_or_else(|| "DEM_COP90_LIVE_NATIVE_INSPECT_METADATA_INVALID".to_owned())?;
    let count = metadata
        .get("count")
        .and_then(Value::as_u64)
        .filter(|value| *value == 1)
        .ok_or_else(|| "DEM_COP90_LIVE_NATIVE_INSPECT_METADATA_INVALID".to_owned())?;
    let crs = metadata
        .get("crs")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "DEM_COP90_LIVE_NATIVE_INSPECT_METADATA_INVALID".to_owned())?;
    let width = metadata
        .get("width")
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .ok_or_else(|| "DEM_COP90_LIVE_NATIVE_INSPECT_WIDTH_INVALID".to_owned())?;
    let height = metadata
        .get("height")
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .ok_or_else(|| "DEM_COP90_LIVE_NATIVE_INSPECT_HEIGHT_INVALID".to_owned())?;
    let expected_path = input_path.to_string_lossy();
    if result.get("path").and_then(Value::as_str) != Some(expected_path.as_ref()) {
        return Err("DEM_COP90_LIVE_NATIVE_INSPECT_PATH_INVALID".to_owned());
    }
    Ok(NativeInspectEvidence {
        readable: true,
        size_bytes: expected_bytes,
        driver: driver.to_owned(),
        count,
        width,
        height,
        crs: crs.to_owned(),
    })
}

fn verify_reopened_state(
    state: &ApplicationServiceState,
    output_root: &Path,
    report: &LiveReport,
    workspace: &LiveAcceptanceWorkspace,
    evidence: &mut LiveEvidence,
) -> Result<(), String> {
    evidence.reopen.attempted = true;
    let readiness = state
        .readiness()
        .map_err(|error| service_error("DEM_COP90_LIVE_REOPEN_READINESS", &error))?;
    if readiness.status != ServiceStatus::StoreReady || readiness.store_schema_version.is_none() {
        return Err("DEM_COP90_LIVE_REOPEN_STORE_NOT_READY".to_owned());
    }
    evidence.reopen.store_ready = Some(true);
    record_stage(workspace, evidence, "reopen_store_ready")?;
    let active = state
        .core_restore(false)
        .map_err(|error| service_error("DEM_COP90_LIVE_REOPEN_CORE_RESTORE", &error))?;
    let active_restore_excludes = !active.core.tasks.iter().any(|task| task.task_id == TASK_ID);
    evidence.reopen.active_restore_excludes = Some(active_restore_excludes);
    if !active_restore_excludes {
        return Err("DEM_COP90_LIVE_ACTIVE_RESTORE_INCLUDES_ARCHIVED_TASK".to_owned());
    }
    record_stage(workspace, evidence, "reopen_active_restore_verified")?;
    let history = state
        .task_history_page(&StoreTaskHistoryPageRequest {
            page_size: 100,
            after_archived_at_epoch_ms: None,
            after_task_id: None,
        })
        .map_err(|error| service_error("DEM_COP90_LIVE_HISTORY", &error))?;
    let history_item = history
        .items
        .iter()
        .find(|item| item.task.task_id == TASK_ID)
        .ok_or_else(|| "DEM_COP90_LIVE_HISTORY_TASK_MISSING".to_owned())?;
    evidence.reopen.history_contains = Some(true);
    record_stage(workspace, evidence, "reopen_history_found")?;
    if history_item.task.status != TaskLifecycleStatus::Completed
        || history_item.task.revision != report.task_revision
    {
        return Err("DEM_COP90_LIVE_HISTORY_TASK_INVALID".to_owned());
    }
    let operation = history_item
        .task
        .operation
        .as_ref()
        .ok_or_else(|| "DEM_COP90_LIVE_HISTORY_OPERATION_MISSING".to_owned())?;
    let history_evidence = TaskEvidence {
        status: history_item.task.status.clone(),
        revision: history_item.task.revision,
        selected_count: history_item.task.selected_count,
        completed_count: history_item.task.completed_count,
        output_root: history_item.task.output_root.clone(),
        operation_id: operation.operation_id.clone(),
        kind: operation.kind.clone(),
        phase: operation.phase.clone(),
        region_id: operation.region_id.clone(),
        approved_worker_operation: operation.approved_worker_operation.clone(),
    };
    assert_task_operation(&history_evidence, output_root)?;
    record_stage(workspace, evidence, "reopen_history_verified")?;

    evidence.set_stage("conversion_source_resolve");
    workspace.write_evidence(evidence)?;
    let source = state
        .resolve_dem_conversion_source(&StoreDemConversionSourceRequest {
            region_id: REGION_ID.to_owned(),
            dataset: DATASET.to_owned(),
            provider: PROVIDER.to_owned(),
            resource_id: RESOURCE_ID.to_owned(),
            capability_id: CAPABILITY_ID.to_owned(),
        })
        .map_err(|error| service_error("DEM_COP90_LIVE_CONVERSION_SOURCE", &error))?
        .ok_or_else(|| "DEM_COP90_LIVE_CONVERSION_SOURCE_MISSING".to_owned())?;
    if source.task_id != TASK_ID
        || source.revision != report.task_revision
        || source.region_id != REGION_ID
        || source.dataset != DATASET
        || source.provider != PROVIDER
        || source.resource_id != RESOURCE_ID
        || source.capability_id != CAPABILITY_ID
        || source.input_path != output_root.join("COP90_dem.tif")
        || source.output_root.as_path() != output_root
    {
        return Err("DEM_COP90_LIVE_CONVERSION_SOURCE_IDENTITY_INVALID".to_owned());
    }
    evidence.reopen.conversion_source_exact = Some(true);
    record_stage(workspace, evidence, "reopen_conversion_source_verified")?;
    Ok(())
}

fn task_evidence(snapshot: &CoreServiceSnapshot) -> Result<TaskEvidence, String> {
    let task = snapshot
        .core
        .tasks
        .iter()
        .find(|task| task.task_id == TASK_ID)
        .ok_or_else(|| "DEM_COP90_LIVE_TASK_MISSING".to_owned())?;
    let operation = task
        .operation
        .as_ref()
        .ok_or_else(|| "DEM_COP90_LIVE_TASK_OPERATION_MISSING".to_owned())?;
    Ok(TaskEvidence {
        status: task.status.clone(),
        revision: task.revision,
        selected_count: task.selected_count,
        completed_count: task.completed_count,
        output_root: task.output_root.clone(),
        operation_id: operation.operation_id.clone(),
        kind: operation.kind.clone(),
        phase: operation.phase.clone(),
        region_id: operation.region_id.clone(),
        approved_worker_operation: operation.approved_worker_operation.clone(),
    })
}

fn assert_task_operation(evidence: &TaskEvidence, output_root: &Path) -> Result<(), String> {
    if evidence.output_root.as_path() != output_root
        || evidence.operation_id != format!("{TASK_ID}:dem_download")
        || evidence.kind != "dem"
        || evidence.phase != "download"
        || evidence.region_id != REGION_ID
        || evidence.approved_worker_operation != "dem_download"
    {
        return Err("DEM_COP90_LIVE_TASK_OPERATION_INVALID".to_owned());
    }
    Ok(())
}

fn service_error(prefix: &str, error: &ApplicationServiceError) -> String {
    format!("{prefix}_{}", error.code())
}
