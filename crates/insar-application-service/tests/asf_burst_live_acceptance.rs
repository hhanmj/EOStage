use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use insar_application_service::{
    default_store_path, ApplicationSarSearchAndFreezeRequest, ApplicationService,
    ApplicationServiceConfig, ApplicationServiceError, BurstAssetPolicy,
    BurstCreateAssetTaskRequest, ASF_BURST_COLLECTION_PROFILE_ID,
};
use insar_core::{
    DownloadItemStatus, DownloadLayout, DownloadOutcome, FrozenSearchBatch, SnapshotItem,
    StoreTaskCommand, StoreTaskPage, StoreTaskPageRequest, StoreTaskSourceBatchPageRequest,
    StoreUpdateTaskRequest, TaskLifecycleStatus, TaskRuntimeSnapshot, TaskStateCommandKind,
};
use insar_download::{part_path, resolve_earthdata_burst_credential};
use serde::Serialize;
use serde_json::{json, Value};

const LIVE_FLAG: &str = "INSAR_BURST_REAL_BYTE_LIVE";
const ADMIN_SEARCH_LIVE_FLAG: &str = "INSAR_BURST_ADMIN_SEARCH_LIVE";
const LIVE_EVIDENCE_ROOT: &str = "_verification/burst-real-byte-20260826";
const TASK_ID: &str = "burst-real-byte-task";
const BATCH_ID: &str = "burst-real-byte-batch";
const SEARCH_DEADLINE: Duration = Duration::from_secs(90);
const TRANSFER_DEADLINE: Duration = Duration::from_secs(120);
const CANCEL_DEADLINE: Duration = Duration::from_secs(30);
const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(15);

#[derive(Clone, Debug, Serialize)]
struct FrozenAssetEvidence {
    role: String,
    file_name: String,
    size_bytes: Option<u64>,
    stable_asset_key: String,
}

#[derive(Clone, Debug, Serialize)]
struct BurstCanaryEvidence {
    schema_version: u64,
    attempt_id: String,
    outcome: String,
    stop_code: Option<String>,
    chain: Vec<String>,
    credential_available: bool,
    credential_material_written: bool,
    source_batch_id: Option<String>,
    source_product_id: Option<String>,
    provider_product_key: Option<String>,
    cmr_granule_concept_id: Option<String>,
    cmr_native_id: Option<String>,
    full_burst_id: Option<String>,
    frozen_assets: Vec<FrozenAssetEvidence>,
    asset_policy: Option<String>,
    execution_policy_id: Option<String>,
    collection_profile_id: Option<String>,
    task_id: String,
    task_status: Option<String>,
    item_status: Option<String>,
    failure_class: Option<String>,
    failure_retryable: Option<bool>,
    rejection_class: Option<String>,
    path_segment_count: Option<String>,
    path_terminal: Option<String>,
    path_has_encoded_separator_or_dot: Option<bool>,
    path_trailing_slash: Option<bool>,
    query_keys: Vec<String>,
    integrity_subtype: Option<String>,
    response_status_class: Option<String>,
    resume_state: Option<String>,
    remote_total_presence: Option<String>,
    size_relation: Option<String>,
    observed_body_bytes: u64,
    partial_bytes: u64,
    final_promoted: bool,
    search_elapsed_ms: u128,
    transfer_elapsed_ms: u128,
    shutdown_elapsed_ms: u128,
}

impl BurstCanaryEvidence {
    fn new(attempt_id: String) -> Self {
        Self {
            schema_version: 5,
            attempt_id,
            outcome: "blocked".to_owned(),
            stop_code: None,
            chain: [
                "ApplicationService",
                "formal Rust Burst CMR",
                "SearchBatch",
                "Store",
                "scheduler",
                "ProductionWorker",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            credential_available: false,
            credential_material_written: false,
            source_batch_id: None,
            source_product_id: None,
            provider_product_key: None,
            cmr_granule_concept_id: None,
            cmr_native_id: None,
            full_burst_id: None,
            frozen_assets: Vec::new(),
            asset_policy: None,
            execution_policy_id: None,
            collection_profile_id: None,
            task_id: TASK_ID.to_owned(),
            task_status: None,
            item_status: None,
            failure_class: None,
            failure_retryable: None,
            rejection_class: None,
            path_segment_count: None,
            path_terminal: None,
            path_has_encoded_separator_or_dot: None,
            path_trailing_slash: None,
            query_keys: Vec::new(),
            integrity_subtype: None,
            response_status_class: None,
            resume_state: None,
            remote_total_presence: None,
            size_relation: None,
            observed_body_bytes: 0,
            partial_bytes: 0,
            final_promoted: false,
            search_elapsed_ms: 0,
            transfer_elapsed_ms: 0,
            shutdown_elapsed_ms: 0,
        }
    }

    fn record_failure(&mut self, code: &str, reason: &str, retryable: bool) {
        self.failure_class = Some(failure_class(code, reason).to_owned());
        self.failure_retryable = Some(retryable);
        self.rejection_class = None;
        self.path_segment_count = None;
        self.path_terminal = None;
        self.path_has_encoded_separator_or_dot = None;
        self.path_trailing_slash = None;
        self.query_keys.clear();
        self.integrity_subtype = None;
        self.response_status_class = None;
        self.resume_state = None;
        self.remote_total_presence = None;
        self.size_relation = None;
        if let Some(diagnostic) = safe_extractor_diagnostic(reason) {
            self.rejection_class = Some(diagnostic.rejection_class.to_owned());
            self.path_segment_count = Some(diagnostic.path_segment_count.to_owned());
            self.path_terminal = Some(diagnostic.path_terminal.to_owned());
            self.path_has_encoded_separator_or_dot =
                Some(diagnostic.path_has_encoded_separator_or_dot);
            self.path_trailing_slash = Some(diagnostic.path_trailing_slash);
            self.query_keys = diagnostic.query_keys;
        }
        if let Some(diagnostic) = safe_integrity_diagnostic(reason) {
            self.integrity_subtype = Some(diagnostic.subtype.to_owned());
            self.response_status_class = Some(diagnostic.response_status.to_owned());
            self.resume_state = Some(diagnostic.resume.to_owned());
            self.remote_total_presence = Some(diagnostic.remote_total.to_owned());
            self.size_relation = Some(diagnostic.size_relation.to_owned());
        }
    }
}

struct SafeIntegrityDiagnostic {
    subtype: &'static str,
    response_status: &'static str,
    resume: &'static str,
    remote_total: &'static str,
    size_relation: &'static str,
}

fn safe_integrity_diagnostic(reason: &str) -> Option<SafeIntegrityDiagnostic> {
    const MARKER: &str = "INTEGRITY_DIAGNOSTIC ";
    const SUBTYPES: &[&str] = &[
        "range_not_satisfiable",
        "response_preparation",
        "expected_remote_total_mismatch",
        "validator_mismatch",
        "body_final_size_mismatch",
        "checksum_mismatch",
        "resolved_identity_mismatch",
        "resolved_checksum_invalid",
    ];
    const RESPONSE_STATUSES: &[&str] = &["2xx", "206", "416", "other"];
    const RESUME_STATES: &[&str] = &["fresh", "resume"];
    const REMOTE_TOTALS: &[&str] = &["present", "absent", "unknown"];
    const SIZE_RELATIONS: &[&str] = &["equal", "mismatch", "unknown"];

    let diagnostic = reason.split_once(MARKER)?.1;
    let mut fields = diagnostic.split_ascii_whitespace();
    let subtype = fields.next()?.strip_prefix("subtype=")?;
    let response_status = fields.next()?.strip_prefix("response_status=")?;
    let resume = fields.next()?.strip_prefix("resume=")?;
    let remote_total = fields.next()?.strip_prefix("remote_total=")?;
    let size_relation = fields.next()?.strip_prefix("size_relation=")?;
    let canonical = |value: &str, allowed: &'static [&'static str]| {
        allowed
            .iter()
            .copied()
            .find(|candidate| *candidate == value)
    };
    Some(SafeIntegrityDiagnostic {
        subtype: canonical(subtype, SUBTYPES)?,
        response_status: canonical(response_status, RESPONSE_STATUSES)?,
        resume: canonical(resume, RESUME_STATES)?,
        remote_total: canonical(remote_total, REMOTE_TOTALS)?,
        size_relation: canonical(size_relation, SIZE_RELATIONS)?,
    })
}

struct SafeExtractorDiagnostic {
    rejection_class: &'static str,
    path_segment_count: &'static str,
    path_terminal: &'static str,
    path_has_encoded_separator_or_dot: bool,
    path_trailing_slash: bool,
    query_keys: Vec<String>,
}

fn canonical_safe_query_key(key: &str) -> Option<&str> {
    (!key.is_empty()
        && key.len() <= 64
        && key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')))
    .then_some(key)
}

fn safe_extractor_diagnostic(reason: &str) -> Option<SafeExtractorDiagnostic> {
    const MARKER: &str = "ASF_EXTRACTOR_DIAGNOSTIC ";
    const REJECTIONS: &[&str] = &[
        "approved",
        "path",
        "query_keys",
        "algorithm",
        "credential_scope",
        "date",
        "expires",
        "signed_headers",
        "token_presence",
        "signature",
        "asf_user_id",
        "content_disposition",
    ];
    const SEGMENT_COUNTS: &[&str] = &["0", "1", "2", "3plus"];
    const TERMINALS: &[&str] = &[
        "exact_0_tiff",
        "other_tiff",
        "exact_0_xml",
        "other_xml",
        "unsupported",
    ];

    let diagnostic = reason.split_once(MARKER)?.1;
    let mut fields = diagnostic.split_ascii_whitespace();
    let rejection = fields.next()?.strip_prefix("rejection=")?;
    let path_segment_count = fields.next()?.strip_prefix("path_segment_count=")?;
    let path_terminal = fields.next()?.strip_prefix("path_terminal=")?;
    let path_encoded = fields.next()?.strip_prefix("path_encoded=")?;
    let path_trailing_slash = fields.next()?.strip_prefix("path_trailing_slash=")?;
    let query_keys = fields.next()?.strip_prefix("query_keys=")?;
    let rejection_class = REJECTIONS
        .iter()
        .copied()
        .find(|candidate| *candidate == rejection)?;
    let path_segment_count = SEGMENT_COUNTS
        .iter()
        .copied()
        .find(|candidate| *candidate == path_segment_count)?;
    let path_terminal = TERMINALS
        .iter()
        .copied()
        .find(|candidate| *candidate == path_terminal)?;
    let path_has_encoded_separator_or_dot = match path_encoded {
        "true" => true,
        "false" => false,
        _ => return None,
    };
    let path_trailing_slash = match path_trailing_slash {
        "true" => true,
        "false" => false,
        _ => return None,
    };
    let query_keys = if query_keys == "none" {
        Vec::new()
    } else {
        let parsed = query_keys.split(',').collect::<Vec<_>>();
        if parsed.is_empty() || parsed.len() > 32 {
            return None;
        }
        let canonical = parsed
            .iter()
            .map(|key| canonical_safe_query_key(key).map(str::to_owned))
            .collect::<Option<Vec<_>>>()?;
        let mut ordered = canonical.clone();
        ordered.sort_unstable();
        ordered.dedup();
        if ordered != canonical {
            return None;
        }
        canonical
    };
    Some(SafeExtractorDiagnostic {
        rejection_class,
        path_segment_count,
        path_terminal,
        path_has_encoded_separator_or_dot,
        path_trailing_slash,
        query_keys,
    })
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

fn frozen_asset_file_name(item: &SnapshotItem) -> Option<&str> {
    item.metadata
        .get("asset_basename")
        .or_else(|| item.payload.get("file_name"))
        .and_then(Value::as_str)
}

fn failure_class(code: &str, reason: &str) -> &'static str {
    if code == "DL005" && reason.starts_with("重定向目标主机 ") {
        "redirect_policy"
    } else {
        match code {
            "DL001" => "cancelled",
            "DL002" => "integrity",
            "DL004" => "credential",
            "ASF_BURST_COOKIE_REQUIRED" => "cookie_required",
            "DL005" => "remote",
            _ => "other",
        }
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
    ]
    .into_iter()
    .all(|forbidden| !lowered.contains(forbidden))
}

fn live_search_request() -> ApplicationSarSearchAndFreezeRequest {
    serde_json::from_value(json!({
        "request_id": "burst-real-byte-search",
        "search": {
            "query": {
                "provider": "ASF",
                "dataset": "SLC-BURST",
                "start": "2014-10-03T00:00:00Z",
                "end": "2014-10-04T00:00:00Z",
                "max_results": 1,
                "burst": {
                    "full_burst_id": "141_301495_IW3",
                    "relative_burst_id": 301495,
                    "absolute_burst_id": 5714273
                }
            }
        },
        "batch_id": BATCH_ID,
        "created_at": "2026-08-26T12:00:00Z",
        "aoi_name": "未限定 AOI",
        "aoi_geojson": null,
        "page_size": 24
    }))
    .expect("bounded Burst live request is a valid typed contract")
}

#[test]
#[ignore = "explicit production Burst search using the persisted desktop AOI, all time, and a 100-result limit"]
fn production_burst_search_accepts_the_live_desktop_admin_aoi_without_a_time_filter() {
    run_production_admin_search("all-time-100", None, None, Some(100), None);
}

#[test]
#[ignore = "explicit production Burst search using the persisted desktop AOI, three months, and all results"]
fn production_burst_search_accepts_three_months_without_a_result_limit() {
    run_production_admin_search(
        "three-months-all",
        Some("2026-05-26"),
        Some("2026-08-26"),
        None,
        None,
    );
}

#[test]
#[ignore = "explicit production Burst search using the persisted desktop AOI, three months, descending, and 100 results"]
fn production_burst_search_accepts_three_months_of_descending_results() {
    run_production_admin_search(
        "three-months-descending-100",
        Some("2026-05-26"),
        Some("2026-08-26"),
        Some(100),
        Some("DESCENDING"),
    );
}

fn run_production_admin_search(
    case: &str,
    start: Option<&str>,
    end: Option<&str>,
    max_results: Option<u64>,
    flight_direction: Option<&str>,
) {
    assert_eq!(
        std::env::var(ADMIN_SEARCH_LIVE_FLAG).ok().as_deref(),
        Some("1"),
        "BURST_ADMIN_SEARCH outcome=blocked code=LIVE_INPUT_REQUIRED"
    );

    let desktop = ApplicationService::new(ApplicationServiceConfig::store(default_store_path()));
    let aoi = desktop
        .working_aoi()
        .expect("desktop working AOI remains readable")
        .expect("desktop working AOI exists");
    assert!(
        aoi.display_name.contains("西藏"),
        "expected the current 西藏自治区 desktop AOI, found {}",
        aoi.display_name
    );
    let geometry = Value::Object(aoi.geometry.clone());
    drop(desktop);

    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root");
    let evidence_root = repository.join("_verification/burst-computer-use-20260826");
    fs::create_dir_all(&evidence_root).expect("Burst live evidence root");
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_millis();
    let store_path = evidence_root.join(format!("tibet-{case}-{stamp}.sqlite3"));
    let service = ApplicationService::new(ApplicationServiceConfig::business_store(&store_path));
    let mut search_query = json!({
        "provider": "ASF",
        "dataset": "SLC-BURST",
        "processing_levels": ["BURST"],
        "aoi_geojson": geometry.clone(),
        "bbox": {
            "west": aoi.bbox[0],
            "south": aoi.bbox[1],
            "east": aoi.bbox[2],
            "north": aoi.bbox[3],
            "crs": "EPSG:4326"
        }
    });
    let query = search_query
        .as_object_mut()
        .expect("diagnostic search query object");
    if let Some(start) = start {
        query.insert("start".to_owned(), json!(start));
    }
    if let Some(end) = end {
        query.insert("end".to_owned(), json!(end));
    }
    if let Some(max_results) = max_results {
        query.insert("max_results".to_owned(), json!(max_results));
    }
    if let Some(flight_direction) = flight_direction {
        query.insert("flight_direction".to_owned(), json!(flight_direction));
    }
    let request: ApplicationSarSearchAndFreezeRequest = serde_json::from_value(json!({
        "request_id": format!("burst-tibet-{case}-{stamp}"),
        "search": {"query": search_query},
        "batch_id": format!("burst-tibet-{case}-batch-{stamp}"),
        "created_at": "2026-08-26T12:00:00Z",
        "aoi_name": aoi.display_name,
        "aoi_geojson": geometry,
        "page_size": 24
    }))
    .expect("live desktop Burst request remains contract-valid");

    let result = service.sar_search_and_freeze_with_progress(&request, |progress| {
        eprintln!(
            "BURST_ADMIN_SEARCH_PROGRESS case={case} sequence={} stage={:?} requests={:?} raw={:?} mapped={:?} exact={:?} admitted={:?} requested={:?}",
            progress.sequence,
            progress.stage,
            progress.provider_request_count,
            progress.raw_fetched_count,
            progress.mapped_count,
            progress.exact_matched_count,
            progress.admitted_count,
            progress.requested_limit
        );
    });
    if let Err(error) = result {
        panic!(
            "BURST_ADMIN_SEARCH case={case} outcome=failed error={error:?} store={}",
            store_path.display()
        );
    }
    service.shutdown().expect("diagnostic service shutdown");
}

fn create_attempt() -> Result<(PathBuf, String), String> {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .map_err(|_| "LIVE_REPOSITORY_ROOT_UNAVAILABLE".to_owned())?;
    let root = repository.join(LIVE_EVIDENCE_ROOT);
    fs::create_dir_all(&root).map_err(|_| "LIVE_EVIDENCE_ROOT_UNAVAILABLE".to_owned())?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "LIVE_CLOCK_INVALID".to_owned())?
        .as_millis();
    let attempt_id = format!("attempt-{stamp}-{}", std::process::id());
    let attempt = root.join(&attempt_id);
    fs::create_dir(&attempt).map_err(|_| "LIVE_ATTEMPT_ALREADY_EXISTS".to_owned())?;
    fs::create_dir(attempt.join("downloads"))
        .map_err(|_| "LIVE_OUTPUT_ROOT_UNAVAILABLE".to_owned())?;
    Ok((attempt, attempt_id))
}

fn write_evidence(attempt: &Path, evidence: &BurstCanaryEvidence) -> Result<(), String> {
    let serialized = serde_json::to_string_pretty(evidence)
        .map_err(|_| "LIVE_EVIDENCE_SERIALIZATION_FAILED".to_owned())?;
    require(
        evidence_is_secret_free(&serialized),
        "LIVE_EVIDENCE_SECRET_REJECTED",
    )?;
    fs::write(attempt.join("result.json"), format!("{serialized}\n"))
        .map_err(|_| "LIVE_EVIDENCE_WRITE_FAILED".to_owned())?;
    let markdown = format!(
        "# Sentinel-1 Burst real-byte canary\n\n- Attempt: `{}`\n- Outcome: `{}`\n- Stop code: `{}`\n- Source batch: `{}`\n- Source product: `{}`\n- Task status: `{}`\n- Item status: `{}`\n- Failure class: `{}`\n- Failure retryable: `{}`\n- Extractor rejection class: `{}`\n- Extractor path segment count: `{}`\n- Extractor path terminal: `{}`\n- Extractor path encoded separator or dot: `{}`\n- Extractor path trailing slash: `{}`\n- Extractor query keys: `{}`\n- Integrity subtype: `{}`\n- Response status class: `{}`\n- Resume state: `{}`\n- Remote total presence: `{}`\n- Size relation: `{}`\n- Observed body bytes: `{}`\n- Safe partial bytes: `{}`\n- Final promoted: `{}`\n- Credential material written: `false`\n",
        evidence.attempt_id,
        evidence.outcome,
        evidence.stop_code.as_deref().unwrap_or("none"),
        evidence.source_batch_id.as_deref().unwrap_or("unavailable"),
        evidence.source_product_id.as_deref().unwrap_or("unavailable"),
        evidence.task_status.as_deref().unwrap_or("unavailable"),
        evidence.item_status.as_deref().unwrap_or("unavailable"),
        evidence.failure_class.as_deref().unwrap_or("none"),
        evidence
            .failure_retryable
            .map(|value| if value { "true" } else { "false" })
            .unwrap_or("none"),
        evidence.rejection_class.as_deref().unwrap_or("none"),
        evidence.path_segment_count.as_deref().unwrap_or("none"),
        evidence.path_terminal.as_deref().unwrap_or("none"),
        evidence
            .path_has_encoded_separator_or_dot
            .map(|value| if value { "true" } else { "false" })
            .unwrap_or("none"),
        evidence
            .path_trailing_slash
            .map(|value| if value { "true" } else { "false" })
            .unwrap_or("none"),
        if evidence.query_keys.is_empty() {
            "none".to_owned()
        } else {
            evidence.query_keys.join(",")
        },
        evidence.integrity_subtype.as_deref().unwrap_or("none"),
        evidence.response_status_class.as_deref().unwrap_or("none"),
        evidence.resume_state.as_deref().unwrap_or("none"),
        evidence.remote_total_presence.as_deref().unwrap_or("none"),
        evidence.size_relation.as_deref().unwrap_or("none"),
        evidence.observed_body_bytes,
        evidence.partial_bytes,
        evidence.final_promoted,
    );
    require(
        evidence_is_secret_free(&markdown),
        "LIVE_EVIDENCE_SECRET_REJECTED",
    )?;
    fs::write(attempt.join("README.md"), markdown)
        .map_err(|_| "LIVE_EVIDENCE_WRITE_FAILED".to_owned())
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
        .ok_or_else(|| "LIVE_TASK_RUNTIME_MISSING".to_owned())
}

fn cancel_item(service: &ApplicationService, task_id: &str, item_id: &str) -> Result<(), String> {
    for attempt in 0..200_u64 {
        let revision = task_runtime(service, task_id)?.revision;
        match service.update_download_task(&StoreUpdateTaskRequest {
            task_id: task_id.to_owned(),
            expected_revision: revision,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::CancelItems,
                item_ids: vec![item_id.to_owned()],
            },
            event_id: format!("burst-real-byte-cancel-{attempt}"),
            occurred_at: "2026-08-26T12:02:00Z".to_owned(),
        }) {
            Ok(_) => return Ok(()),
            Err(error) if error.code() == "V3_STORE_REVISION_CONFLICT" => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(application_stop_code(error)),
        }
    }
    Err("LIVE_CANCEL_REVISION_CONFLICT".to_owned())
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
    Err("LIVE_TASK_PAGE_REVISION_CONFLICT".to_owned())
}

fn capture_task_evidence(
    service: &ApplicationService,
    attempt: &Path,
    evidence: &mut BurstCanaryEvidence,
) -> Result<(), String> {
    let runtime = task_runtime(service, TASK_ID)?;
    evidence.task_status = Some(task_status_label(&runtime.status).to_owned());
    let page = task_page_latest(service, TASK_ID)?;
    let item = page
        .items
        .iter()
        .find(|entry| {
            entry
                .item
                .metadata
                .get("asset_role")
                .and_then(Value::as_str)
                == Some("primary")
        })
        .ok_or_else(|| "LIVE_PRIMARY_TASK_ITEM_MISSING".to_owned())?;
    evidence.item_status = Some(item_status_label(&item.status).to_owned());
    evidence.observed_body_bytes = evidence.observed_body_bytes.max(item.bytes_completed);
    evidence.asset_policy = item
        .item
        .metadata
        .get("burst_asset_policy")
        .and_then(Value::as_str)
        .map(str::to_owned);
    if let Some(policy) = item.item.execution_policy.as_ref() {
        evidence.execution_policy_id = Some(policy.policy_id.clone());
        evidence.collection_profile_id = policy.collection_profile_id.clone();
    }
    if let Some(failure) = item.failure.as_ref() {
        evidence.record_failure(&failure.code, &failure.reason_zh, failure.retryable);
    }

    let file_name = frozen_asset_file_name(&item.item)
        .ok_or_else(|| "LIVE_PRIMARY_FILE_NAME_MISSING".to_owned())?;
    let destination = match item.destination.clone() {
        Some(destination) => destination,
        None => {
            let burst_folder = item
                .item
                .metadata
                .get("product_id")
                .and_then(Value::as_str)
                .ok_or_else(|| "LIVE_SOURCE_PRODUCT_ID_MISSING".to_owned())?;
            attempt.join("downloads").join(burst_folder).join(file_name)
        }
    };
    evidence.partial_bytes = fs::metadata(part_path(&destination))
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    evidence.final_promoted = destination.exists();
    Ok(())
}

fn string_field(value: &Value, field: &'static str) -> Result<String, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| "LIVE_SOURCE_IDENTITY_INVALID".to_owned())
}

fn validate_source_batch(
    batch: &FrozenSearchBatch,
    evidence: &mut BurstCanaryEvidence,
) -> Result<(SnapshotItem, String, String), String> {
    require(batch.batch_id == BATCH_ID, "LIVE_SOURCE_BATCH_ID_INVALID")?;
    require(
        batch.provider_id == "asf.catalog",
        "LIVE_SOURCE_PROVIDER_INVALID",
    )?;
    require(batch.candidates.len() == 1, "LIVE_SOURCE_COUNT_INVALID")?;
    let product = batch.candidates[0].clone();
    require(
        product.provider_id == "asf.catalog"
            && product.metadata["dataset"] == "SLC-BURST"
            && product.metadata["product_family"] == "BURST",
        "LIVE_SOURCE_PRODUCT_INVALID",
    )?;
    require(
        product.item_id.starts_with("ASF::SLC-BURST::")
            && product.metadata["product_key"] == product.item_id,
        "LIVE_SOURCE_PRODUCT_IDENTITY_INVALID",
    )?;
    let native = &product.metadata["native_metadata"];
    let concept_id = string_field(native, "cmr_granule_concept_id")?;
    let native_id = string_field(native, "cmr_native_id")?;
    require(
        concept_id.starts_with('G')
            && concept_id.ends_with("-ASF")
            && native_id.ends_with("-BURST"),
        "LIVE_CMR_IDENTITY_INVALID",
    )?;
    let full_burst_id = string_field(&product.metadata["burst"], "full_burst_id")?;
    require(
        full_burst_id == "141_301495_IW3",
        "LIVE_BURST_IDENTITY_INVALID",
    )?;
    let assets = product.metadata["assets"]
        .as_array()
        .ok_or_else(|| "LIVE_SOURCE_ASSETS_INVALID".to_owned())?;
    require(assets.len() == 2, "LIVE_SOURCE_ASSET_COUNT_INVALID")?;
    let mut primary = None;
    let mut metadata = None;
    let mut frozen_assets = Vec::new();
    for asset in assets {
        let role = string_field(asset, "role")?;
        let file_name = string_field(asset, "file_name")?;
        let stable_asset_key = string_field(asset, "stable_asset_key")?;
        let official_href = string_field(asset, "url")?;
        require(
            official_href.starts_with("https://sentinel1-burst.asf.alaska.edu/")
                && !official_href.contains('?')
                && !official_href.contains('#'),
            "LIVE_SOURCE_ASSET_HOST_INVALID",
        )?;
        let size_bytes = asset.get("size_bytes").and_then(Value::as_u64);
        match role.as_str() {
            "primary"
                if file_name.ends_with(".tiff")
                    && asset["media_type"] == "image/tiff"
                    && size_bytes.is_some_and(|size| size > 0) =>
            {
                require(primary.is_none(), "LIVE_SOURCE_ASSET_ROLE_DUPLICATE")?;
                primary = Some((stable_asset_key.clone(), file_name.clone()));
            }
            "metadata"
                if file_name.ends_with(".xml") && asset["media_type"] == "application/xml" =>
            {
                require(metadata.is_none(), "LIVE_SOURCE_ASSET_ROLE_DUPLICATE")?;
                metadata = Some(stable_asset_key.clone());
            }
            _ => return Err("LIVE_SOURCE_ASSET_ROLE_INVALID".to_owned()),
        }
        frozen_assets.push(FrozenAssetEvidence {
            role,
            file_name,
            size_bytes,
            stable_asset_key,
        });
    }
    let (primary_item_id, primary_file_name) =
        primary.ok_or_else(|| "LIVE_SOURCE_PRIMARY_MISSING".to_owned())?;
    require(metadata.is_some(), "LIVE_SOURCE_METADATA_MISSING")?;

    evidence.source_batch_id = Some(batch.batch_id.clone());
    evidence.source_product_id = Some(product.item_id.clone());
    evidence.provider_product_key = product.metadata["provider_product_key"]
        .as_str()
        .map(str::to_owned);
    evidence.cmr_granule_concept_id = Some(concept_id);
    evidence.cmr_native_id = Some(native_id);
    evidence.full_burst_id = Some(full_burst_id);
    evidence.frozen_assets = frozen_assets;
    Ok((product, primary_item_id, primary_file_name))
}

fn run_live_chain(
    service: &ApplicationService,
    attempt: &Path,
    evidence: &mut BurstCanaryEvidence,
) -> Result<(), String> {
    let credential =
        resolve_earthdata_burst_credential(None).map_err(|error| error.code().to_owned())?;
    drop(credential);
    evidence.credential_available = true;

    let search_started = Instant::now();
    service
        .sar_search_and_freeze_with_progress(&live_search_request(), |_| {})
        .map_err(application_stop_code)?;
    evidence.search_elapsed_ms = search_started.elapsed().as_millis();
    require(
        search_started.elapsed() <= SEARCH_DEADLINE,
        "LIVE_SEARCH_DEADLINE_EXCEEDED",
    )?;
    let frozen_snapshot = service.snapshot(true).map_err(application_stop_code)?;
    let batch = frozen_snapshot
        .restore
        .workbench_batch
        .as_ref()
        .ok_or_else(|| "LIVE_SOURCE_BATCH_MISSING".to_owned())?;
    let workbench = frozen_snapshot
        .restore
        .workbench
        .as_ref()
        .ok_or_else(|| "LIVE_WORKBENCH_MISSING".to_owned())?;
    let (product, primary_item_id, primary_file_name) = validate_source_batch(batch, evidence)?;

    service
        .create_burst_asset_task(&BurstCreateAssetTaskRequest {
            request_id: "burst-real-byte-create".to_owned(),
            source_batch_id: batch.batch_id.clone(),
            selected_product_ids: vec![product.item_id.clone()],
            asset_policy: BurstAssetPolicy::Primary,
            expected_workbench_revision: workbench.revision,
            task_id: TASK_ID.to_owned(),
            output_root: attempt.join("downloads"),
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-08-26T12:01:00Z".to_owned(),
        })
        .map_err(application_stop_code)?;

    let source_page = service
        .task_source_batch_page(&StoreTaskSourceBatchPageRequest {
            task_id: TASK_ID.to_owned(),
            page_index: 0,
            page_size: 24,
        })
        .map_err(application_stop_code)?;
    require(
        source_page.source_batch_id == batch.batch_id
            && source_page.total_count == 1
            && source_page.items.len() == 1
            && source_page.items[0].belongs_to_task
            && source_page.items[0].item.item_id == product.item_id,
        "LIVE_SOURCE_TASK_MEMBERSHIP_INVALID",
    )?;

    let burst_folder = product
        .metadata
        .get("product_id")
        .and_then(Value::as_str)
        .ok_or_else(|| "LIVE_SOURCE_PRODUCT_ID_MISSING".to_owned())?;
    let destination = attempt
        .join("downloads")
        .join(burst_folder)
        .join(&primary_file_name);
    let partial = part_path(&destination);
    let transfer_started = Instant::now();
    loop {
        let download = service
            .download_snapshot(TASK_ID)
            .map_err(application_stop_code)?;
        let observed = download
            .progress
            .iter()
            .filter(|progress| progress.item_id == primary_item_id)
            .map(|progress| progress.bytes_completed)
            .chain(
                download
                    .results
                    .iter()
                    .filter(|result| result.item_id == primary_item_id)
                    .map(|result| result.bytes_written),
            )
            .max()
            .unwrap_or(0);
        evidence.observed_body_bytes = evidence.observed_body_bytes.max(observed);
        if evidence.observed_body_bytes > 0 {
            break;
        }
        if let Some(result) = download
            .results
            .iter()
            .find(|result| result.item_id == primary_item_id)
        {
            return Err(result
                .failure
                .as_ref()
                .map(|failure| failure.code.clone())
                .unwrap_or_else(|| "LIVE_TRANSFER_TERMINATED_WITHOUT_BYTES".to_owned()));
        }
        require(
            transfer_started.elapsed() <= TRANSFER_DEADLINE,
            "LIVE_TRANSFER_DEADLINE_EXCEEDED",
        )?;
        thread::sleep(Duration::from_millis(25));
    }
    cancel_item(service, TASK_ID, &primary_item_id)?;

    let cancel_started = Instant::now();
    let terminal_result = loop {
        let download = service
            .download_snapshot(TASK_ID)
            .map_err(application_stop_code)?;
        if let Some(result) = download
            .results
            .into_iter()
            .find(|result| result.item_id == primary_item_id)
        {
            break result;
        }
        require(
            cancel_started.elapsed() <= CANCEL_DEADLINE,
            "LIVE_CANCEL_DEADLINE_EXCEEDED",
        )?;
        thread::sleep(Duration::from_millis(25));
    };
    evidence.transfer_elapsed_ms = transfer_started.elapsed().as_millis();
    require(
        terminal_result.outcome == DownloadOutcome::Cancelled,
        "LIVE_ITEM_NOT_CANCELLED",
    )?;
    evidence.observed_body_bytes = evidence
        .observed_body_bytes
        .max(terminal_result.bytes_written);

    let runtime = task_runtime(service, TASK_ID)?;
    require(
        runtime.status == TaskLifecycleStatus::Cancelled
            && runtime.cancelled_item_ids == [primary_item_id.clone()],
        "LIVE_TASK_STATE_INVALID",
    )?;
    let page = task_page_latest(service, TASK_ID)?;
    require(
        page.total_count == 1
            && page.items.len() == 1
            && page.items[0].item.item_id == primary_item_id
            && page.items[0].status == DownloadItemStatus::Cancelled
            && page.items[0].bytes_completed > 0,
        "LIVE_ITEM_STATE_INVALID",
    )?;
    let frozen = &page.items[0].item;
    require(
        frozen.metadata["source_item_id"] == product.item_id
            && frozen.metadata["asset_role"] == "primary"
            && frozen.metadata["burst_asset_policy"] == "primary"
            && frozen.payload["burst_asset"] == true,
        "LIVE_FROZEN_ITEM_INVALID",
    )?;
    let policy = frozen
        .execution_policy
        .as_ref()
        .ok_or_else(|| "LIVE_EXECUTION_POLICY_MISSING".to_owned())?;
    require(
        policy.policy_id == "asf.sentinel-1.burst.asset.v1"
            && policy.collection_profile_id.as_deref() == Some(ASF_BURST_COLLECTION_PROFILE_ID),
        "LIVE_EXECUTION_POLICY_INVALID",
    )?;

    let partial_bytes = fs::metadata(&partial)
        .map_err(|_| "LIVE_SAFE_PART_MISSING".to_owned())?
        .len();
    require(partial_bytes > 0, "LIVE_SAFE_PART_EMPTY")?;
    require(!destination.exists(), "LIVE_FINAL_PROMOTED")?;
    evidence.asset_policy = Some("primary".to_owned());
    evidence.execution_policy_id = Some(policy.policy_id.clone());
    evidence.collection_profile_id = policy.collection_profile_id.clone();
    evidence.task_status = Some("cancelled".to_owned());
    evidence.item_status = Some("cancelled".to_owned());
    evidence.partial_bytes = partial_bytes;
    evidence.final_promoted = false;
    Ok(())
}

#[test]
fn burst_live_request_and_evidence_contract_are_offline_secret_free() {
    let request = live_search_request();
    assert_eq!(request.search.query.provider, "ASF");
    assert_eq!(request.search.query.dataset, "SLC-BURST");
    assert_eq!(request.search.query.max_results, Some(1));
    let evidence = BurstCanaryEvidence::new("attempt-offline".to_owned());
    let serialized = serde_json::to_string(&evidence).unwrap();
    assert!(evidence_is_secret_free(&serialized));
    assert!(!evidence.credential_material_written);
}

#[test]
fn burst_failure_evidence_records_only_whitelisted_store_state() {
    let mut evidence = BurstCanaryEvidence::new("attempt-offline-failure".to_owned());
    evidence.record_failure(
        "DL005",
        "重定向目标主机 signed-host scheme=https port=443 userinfo=false 被冻结的远程主机策略拒绝。 ASF_EXTRACTOR_DIAGNOSTIC rejection=query_keys path_segment_count=2 path_terminal=exact_0_tiff path_encoded=false path_trailing_slash=false query_keys=X-Amz-Algorithm,X-Amz-Credential,X-Amz-Date,X-Amz-Expires,X-Amz-Security-Token,X-Amz-Signature,X-Amz-SignedHeaders,x-id RAW_PATH_SEGMENT RAW_QUERY_VALUE ASIASECRET 0123456789abcdef",
        false,
    );
    evidence.task_status = Some(task_status_label(&TaskLifecycleStatus::Failed).to_owned());
    evidence.item_status = Some(item_status_label(&DownloadItemStatus::Failed).to_owned());
    let serialized = serde_json::to_string(&evidence).unwrap();
    assert_eq!(evidence.failure_class.as_deref(), Some("redirect_policy"));
    assert_eq!(evidence.failure_retryable, Some(false));
    assert_eq!(evidence.rejection_class.as_deref(), Some("query_keys"));
    assert_eq!(evidence.path_segment_count.as_deref(), Some("2"));
    assert_eq!(evidence.path_terminal.as_deref(), Some("exact_0_tiff"));
    assert_eq!(evidence.path_has_encoded_separator_or_dot, Some(false));
    assert_eq!(evidence.path_trailing_slash, Some(false));
    assert_eq!(
        evidence.query_keys,
        [
            "X-Amz-Algorithm",
            "X-Amz-Credential",
            "X-Amz-Date",
            "X-Amz-Expires",
            "X-Amz-Security-Token",
            "X-Amz-Signature",
            "X-Amz-SignedHeaders",
            "x-id",
        ]
    );
    assert!(evidence_is_secret_free(&serialized));
    assert!(!serialized.contains("signed-host"));
    assert!(!serialized.contains("重定向目标主机"));
    for forbidden in [
        "RAW_PATH_SEGMENT",
        "RAW_QUERY_VALUE",
        "ASIASECRET",
        "0123456789abcdef",
    ] {
        assert!(!serialized.contains(forbidden));
    }
}

#[test]
fn burst_failure_evidence_rejects_unrecognized_diagnostic_tokens() {
    for reason in [
        "重定向目标主机 signed-host 被拒绝。 ASF_EXTRACTOR_DIAGNOSTIC rejection=raw-secret path_segment_count=2 path_terminal=exact_0_tiff path_encoded=false path_trailing_slash=false query_keys=X-Amz-Date",
        "重定向目标主机 signed-host 被拒绝。 ASF_EXTRACTOR_DIAGNOSTIC rejection=date path_segment_count=2 path_terminal=/raw/secret/path path_encoded=false path_trailing_slash=false query_keys=X-Amz-Date",
        "重定向目标主机 signed-host 被拒绝。 ASF_EXTRACTOR_DIAGNOSTIC rejection=date path_segment_count=2 path_terminal=other_tiff path_encoded=false path_trailing_slash=false query_keys=X-Amz-Date,Raw/SecretKey",
    ] {
        let mut evidence = BurstCanaryEvidence::new("attempt-offline-invalid".to_owned());
        evidence.record_failure("DL005", reason, false);
        let serialized = serde_json::to_string(&evidence).unwrap();
        assert_eq!(evidence.rejection_class, None);
        assert_eq!(evidence.path_segment_count, None);
        assert_eq!(evidence.path_terminal, None);
        assert_eq!(evidence.path_has_encoded_separator_or_dot, None);
        assert_eq!(evidence.path_trailing_slash, None);
        assert!(evidence.query_keys.is_empty());
        assert!(!serialized.contains("raw-secret"));
        assert!(!serialized.contains("/raw/secret/path"));
        assert!(!serialized.contains("RawSecretKey"));
    }
}

#[test]
fn burst_integrity_evidence_records_only_fixed_diagnostics() {
    let mut evidence = BurstCanaryEvidence::new("attempt-offline-integrity".to_owned());
    evidence.record_failure(
        "DL002",
        "下载文件不完整。 INTEGRITY_DIAGNOSTIC subtype=expected_remote_total_mismatch response_status=206 resume=resume remote_total=present size_relation=mismatch RAW_HEADER RAW_URL 171218848 SECRET_VALUE",
        true,
    );
    let serialized = serde_json::to_string(&evidence).unwrap();
    assert_eq!(
        evidence.integrity_subtype.as_deref(),
        Some("expected_remote_total_mismatch")
    );
    assert_eq!(evidence.response_status_class.as_deref(), Some("206"));
    assert_eq!(evidence.resume_state.as_deref(), Some("resume"));
    assert_eq!(evidence.remote_total_presence.as_deref(), Some("present"));
    assert_eq!(evidence.size_relation.as_deref(), Some("mismatch"));
    for forbidden in [
        "RAW_HEADER",
        "RAW_URL",
        "171218848",
        "SECRET_VALUE",
        "下载文件不完整",
    ] {
        assert!(!serialized.contains(forbidden));
    }
}

#[test]
fn burst_integrity_evidence_rejects_unrecognized_diagnostic_tokens() {
    for reason in [
        "INTEGRITY_DIAGNOSTIC subtype=raw_secret response_status=206 resume=resume remote_total=present size_relation=mismatch",
        "INTEGRITY_DIAGNOSTIC subtype=checksum_mismatch response_status=raw_status resume=resume remote_total=present size_relation=mismatch",
        "INTEGRITY_DIAGNOSTIC subtype=checksum_mismatch response_status=2xx resume=raw_resume remote_total=present size_relation=mismatch",
        "INTEGRITY_DIAGNOSTIC subtype=checksum_mismatch response_status=2xx resume=fresh remote_total=raw_total size_relation=mismatch",
        "INTEGRITY_DIAGNOSTIC subtype=checksum_mismatch response_status=2xx resume=fresh remote_total=absent size_relation=raw_relation",
    ] {
        let mut evidence = BurstCanaryEvidence::new("attempt-offline-invalid-integrity".to_owned());
        evidence.record_failure("DL002", reason, true);
        let serialized = serde_json::to_string(&evidence).unwrap();
        assert_eq!(evidence.integrity_subtype, None);
        assert_eq!(evidence.response_status_class, None);
        assert_eq!(evidence.resume_state, None);
        assert_eq!(evidence.remote_total_presence, None);
        assert_eq!(evidence.size_relation, None);
        assert!(!serialized.contains("raw_"));
    }
}

#[test]
fn burst_failure_capture_never_indexes_missing_frozen_item_keys() {
    let item = |metadata: Value, payload: Value| SnapshotItem {
        item_id: "fixture-item".to_owned(),
        provider_id: "asf.catalog".to_owned(),
        display_name: "0.tiff".to_owned(),
        acquisition_datetime: None,
        footprint_geojson: None,
        metadata: metadata.as_object().unwrap().clone(),
        payload: payload.as_object().unwrap().clone(),
        asset_access: None,
        execution_policy: None,
    };
    let fixtures = [
        item(json!({"asset_basename": "0.tiff"}), json!({})),
        item(json!({}), json!({"file_name": "0.tiff"})),
        item(json!({}), json!({})),
    ];
    let captured = fixtures
        .iter()
        .map(frozen_asset_file_name)
        .collect::<Vec<_>>();
    assert_eq!(captured, [Some("0.tiff"), Some("0.tiff"), None]);
}

#[test]
#[ignore = "local-only explicit Earthdata account credential preflight; never contacts the network"]
fn burst_real_byte_credentials_are_locally_parseable() {
    let available = resolve_earthdata_burst_credential(None).map(drop).is_ok();
    eprintln!("BURST_REAL_BYTE_CREDENTIAL_AVAILABLE={available}");
    assert!(
        available,
        "BURST_REAL_BYTE_CANARY outcome=blocked code=EARTHDATA_BURST_CREDENTIAL_UNAVAILABLE"
    );
}

#[test]
#[ignore = "explicit bounded production ApplicationService/CMR/Store/scheduler/ProductionWorker Burst real-byte canary"]
fn production_burst_real_bytes_cancel_to_safe_partial_without_promotion() {
    assert_eq!(
        std::env::var(LIVE_FLAG).ok().as_deref(),
        Some("1"),
        "BURST_REAL_BYTE_CANARY outcome=blocked code=LIVE_INPUT_REQUIRED"
    );
    let (attempt, attempt_id) = create_attempt().expect("bounded new live evidence attempt");
    let mut evidence = BurstCanaryEvidence::new(attempt_id);
    let service = ApplicationService::new(ApplicationServiceConfig::business_store(
        attempt.join("tasks.sqlite3"),
    ));
    let result = run_live_chain(&service, &attempt, &mut evidence);
    if evidence.source_product_id.is_some()
        && capture_task_evidence(&service, &attempt, &mut evidence).is_err()
        && result.is_err()
    {
        evidence.failure_class = Some("store_readback_unavailable".to_owned());
        evidence.failure_retryable = None;
    }
    let shutdown_started = Instant::now();
    let shutdown_result = service.shutdown();
    evidence.shutdown_elapsed_ms = shutdown_started.elapsed().as_millis();
    let result = result.and_then(|_| {
        shutdown_result.map_err(application_stop_code)?;
        require(
            shutdown_started.elapsed() <= SHUTDOWN_DEADLINE,
            "LIVE_SHUTDOWN_DEADLINE_EXCEEDED",
        )
    });
    match &result {
        Ok(()) => {
            evidence.outcome = "passed".to_owned();
            evidence.stop_code = None;
        }
        Err(code) => {
            evidence.outcome = "blocked".to_owned();
            evidence.stop_code = Some(code.clone());
        }
    }
    write_evidence(&attempt, &evidence).expect("secret-free live evidence write");
    eprintln!(
        "BURST_REAL_BYTE_CANARY outcome={} code={} evidence={} observed_bytes={} partial_bytes={} final_promoted={}",
        evidence.outcome,
        evidence.stop_code.as_deref().unwrap_or("none"),
        attempt.display(),
        evidence.observed_body_bytes,
        evidence.partial_bytes,
        evidence.final_promoted,
    );
    assert!(
        result.is_ok(),
        "BURST_REAL_BYTE_CANARY outcome=blocked code={} evidence={}",
        evidence.stop_code.as_deref().unwrap_or("LIVE_UNKNOWN"),
        attempt.display(),
    );
}
