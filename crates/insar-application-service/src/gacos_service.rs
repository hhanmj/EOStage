//! Native GACOS request preparation and one-shot submission authority.
use super::{ApplicationService, ApplicationServiceError};
use chrono::{NaiveDate, SecondsFormat, Utc};
use insar_core::{
    DownloadLayout, FrozenSearchBatch, SearchWorkbenchState, SnapshotItem, StoreCreateTaskRequest,
    StoreSearchBatchRequest, TaskOperationDescriptor,
};
use insar_provider::gacos_delivery::{
    GacosBoundingBox, GacosOutputFormat, GacosRequestInput, GacosUtcTime,
};
use insar_store::StoredTask;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
const PROVIDER: &str = "gacos.webform";
const PREPARED_KEY: &str = "gacos_prepared_request_v1";
const MAX_REQUESTS: usize = 256;

#[derive(Clone, Copy, Debug, thiserror::Error)]
pub enum GacosRequestError {
    #[error("GACOS request parameters are invalid")]
    Invalid,
    #[error("GACOS requires final confirmation")]
    ConfirmationRequired,
    #[error("GACOS prepared plan is missing or changed")]
    PlanStale,
    #[error("GACOS email is not configured")]
    EmailNotConfigured,
    #[error("GACOS system credential store is unavailable")]
    CredentialsUnavailable,
    #[error("GACOS another confirmed submission is active")]
    SubmissionBusy,
    #[error("GACOS request was already submitted")]
    DuplicateRequest,
    #[error("GACOS native transport is unavailable")]
    TransportUnavailable,
    #[error("GACOS result import requires its frozen request task and item")]
    ImportContextRequired,
    #[error("GACOS result link is invalid")]
    ResultLinkInvalid,
    #[error("GACOS result link host is unreviewed")]
    ResultLinkHostUnreviewed,
    #[error("GACOS result link does not match its frozen identity")]
    ResultLinkMismatch,
    #[error("GACOS result link credential storage is unavailable")]
    ResultLinkCredentialUnavailable,
}
impl GacosRequestError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::Invalid => "GACOS_REQUEST_INVALID",
            Self::ConfirmationRequired => "GACOS_CONFIRMATION_REQUIRED",
            Self::PlanStale => "GACOS_PREPARED_PLAN_STALE",
            Self::EmailNotConfigured => "GACOS_EMAIL_NOT_CONFIGURED",
            Self::CredentialsUnavailable => "GACOS_CREDENTIALS_UNAVAILABLE",
            Self::SubmissionBusy => "GACOS_SUBMISSION_BUSY",
            Self::DuplicateRequest => "GACOS_DUPLICATE_REQUEST",
            Self::TransportUnavailable => "GACOS_TRANSPORT_UNAVAILABLE",
            Self::ImportContextRequired => "GACOS_IMPORT_CONTEXT_REQUIRED",
            Self::ResultLinkInvalid => "GACOS_RESULT_LINK_INVALID",
            Self::ResultLinkHostUnreviewed => "GACOS_RESULT_LINK_HOST_UNREVIEWED",
            Self::ResultLinkMismatch => "GACOS_RESULT_LINK_HASH_MISMATCH",
            Self::ResultLinkCredentialUnavailable => "GACOS_RESULT_LINK_CREDENTIAL_UNAVAILABLE",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GacosConfirmRequest {
    pub prepared_batch_id: String,
    pub plan_fingerprint_sha256: String,
    pub task_id: String,
    pub confirm: bool,
    #[serde(default)]
    pub force_reapply: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct FrozenRequest {
    bbox: [f64; 4],
    dates: Vec<String>,
    hour: u8,
    minute: u8,
    // Preserve the historical GeoTIFF payload/fingerprint for duplicate detection.
    #[serde(default, skip_serializing_if = "gacos_format_is_geotiff")]
    output_format: GacosOutputFormat,
}
fn gacos_format_is_geotiff(value: &GacosOutputFormat) -> bool {
    *value == GacosOutputFormat::GeoTiff
}
impl FrozenRequest {
    fn input(&self) -> Result<GacosRequestInput, ApplicationServiceError> {
        let dates = self
            .dates
            .iter()
            .map(|date| {
                NaiveDate::parse_from_str(date, "%Y-%m-%d")
                    .map_err(|_| invalid("GACOS prepared date is invalid"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let utc = GacosUtcTime::new(self.hour, self.minute)
            .map_err(|_| invalid("GACOS UTC time is invalid"))?;
        let input = GacosRequestInput::new(
            GacosBoundingBox::new(self.bbox[0], self.bbox[1], self.bbox[2], self.bbox[3]),
            dates,
            utc,
            self.output_format,
        );
        input
            .validate()
            .map_err(|_| invalid("GACOS prepared request is invalid"))?;
        Ok(input)
    }
}

#[derive(Clone, Debug)]
pub(super) struct GacosResultParentParameters {
    pub(super) bbox: [f64; 4],
    pub(super) dates: Vec<String>,
    pub(super) hour: u8,
    pub(super) minute: u8,
}

/// Parse the frozen request with its complete typed schema, then project only
/// the fields that belong in a result manifest.  Result tasks must not inherit
/// request-only fields such as output format, while malformed or unknown parent
/// payload fields remain rejected by FrozenRequest's strict schema.
pub(super) fn project_gacos_result_parent_parameters(
    payload: &Map<String, Value>,
) -> Result<GacosResultParentParameters, ApplicationServiceError> {
    let frozen: FrozenRequest = serde_json::from_value(Value::Object(payload.clone()))
        .map_err(|_| invalid("GACOS frozen form request is invalid"))?;
    frozen.input()?;
    Ok(GacosResultParentParameters {
        bbox: frozen.bbox,
        dates: frozen.dates,
        hour: frozen.hour,
        minute: frozen.minute,
    })
}

/// An empty native lock file coordinates senders of this Store across service
/// instances and processes. It stores no task state or credentials. Closing the
/// owned handle releases the OS lock, including after a process crash.
pub(crate) fn lock_gacos_submissions(
    store_path: &std::path::Path,
) -> Result<std::fs::File, ApplicationServiceError> {
    let canonical_store = store_path
        .canonicalize()
        .map_err(|_| ApplicationServiceError::from(GacosRequestError::SubmissionBusy))?;
    let mut lock_path = canonical_store.as_os_str().to_os_string();
    lock_path.push(".gacos-submit.lock");
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(PathBuf::from(lock_path))
        .map_err(|_| ApplicationServiceError::from(GacosRequestError::SubmissionBusy))?;
    file.try_lock()
        .map_err(|_| ApplicationServiceError::from(GacosRequestError::SubmissionBusy))?;
    Ok(file)
}

fn invalid(_reason: impl Into<String>) -> ApplicationServiceError {
    GacosRequestError::Invalid.into()
}
fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}
fn digest(value: &Value) -> Result<String, ApplicationServiceError> {
    let bytes = serde_json::to_vec(value).map_err(|_| invalid("GACOS identity encoding failed"))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}
fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str, ApplicationServiceError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| invalid(format!("GACOS {key} is required")))
}
fn opaque_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 120
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}
impl ApplicationService {
    /// Persist a non-active frozen batch after temporal and AOI validation.
    /// This operation never creates a task or sends a provider request.
    pub(crate) fn freeze_gacos_plan(
        &self,
        plan: &mut Value,
        selected_scene_ids: &[String],
    ) -> Result<(), ApplicationServiceError> {
        let output_root = PathBuf::from(text(plan, "output_directory")?);
        if !output_root.is_absolute() {
            return Err(invalid("GACOS output directory must be absolute"));
        }
        let geometry = plan
            .get("aoi_geojson")
            .and_then(Value::as_object)
            .ok_or_else(|| invalid("GACOS prepared AOI is missing"))?
            .clone();
        let output_format: GacosOutputFormat = serde_json::from_value(
            plan.get("output_format")
                .cloned()
                .unwrap_or_else(|| json!("geotiff")),
        )
        .map_err(|_| invalid("GACOS output format is invalid"))?;
        let mut requests = Vec::new();
        for batch in plan
            .get("batches")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid("GACOS prepared batches are missing"))?
        {
            let dates = batch
                .get("dates")
                .and_then(Value::as_array)
                .ok_or_else(|| invalid("GACOS prepared dates are missing"))?
                .iter()
                .map(|date| {
                    date.as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| invalid("GACOS date is invalid"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let time = |key| {
                batch
                    .get(key)
                    .and_then(Value::as_u64)
                    .and_then(|value| u8::try_from(value).ok())
                    .ok_or_else(|| invalid("GACOS explicit UTC time is required"))
            };
            for window in batch
                .get("request_windows")
                .and_then(Value::as_array)
                .ok_or_else(|| invalid("GACOS request windows are missing"))?
            {
                let coordinate = |key| {
                    window
                        .get(key)
                        .and_then(Value::as_f64)
                        .ok_or_else(|| invalid("GACOS request coordinate is invalid"))
                };
                let request = FrozenRequest {
                    bbox: [
                        coordinate("west")?,
                        coordinate("south")?,
                        coordinate("east")?,
                        coordinate("north")?,
                    ],
                    dates: dates.clone(),
                    hour: time("hour")?,
                    minute: time("minute")?,
                    output_format,
                };
                request.input()?;
                requests.push(request);
            }
        }
        if requests.is_empty() || requests.len() > MAX_REQUESTS {
            return Err(invalid("GACOS prepared request count is invalid"));
        }
        let mut frozen = json!({"schema_version": 1, "output_directory": output_root,
            "aoi_geojson": geometry, "aoi_name": text(plan, "aoi_name")?,
            "selected_scene_ids": selected_scene_ids, "requests": requests});
        // Freeze explicit imports for provenance without changing historical request identities.
        for key in ["source_kind", "source_batch_id", "aoi_source"] {
            if let Some(value) = plan.get(key).filter(|value| !value.is_null()) {
                frozen[key] = value.clone();
            }
        }
        let fingerprint = digest(&frozen)?;
        let batch_id = format!("gacos-plan-{fingerprint}");
        let candidates = requests
            .iter()
            .enumerate()
            .map(|(index, request)| {
                let payload = serde_json::to_value(request)
                    .map_err(|_| invalid("GACOS request encoding failed"))?;
                let request_fingerprint = digest(&payload)?;
                Ok(SnapshotItem {
                    item_id: format!("gacos-request-{request_fingerprint}"),
                    provider_id: PROVIDER.to_owned(),
                    display_name: format!("GACOS 请求 {}", index + 1),
                    acquisition_datetime: None,
                    footprint_geojson: Some(geometry.clone()),
                    metadata: Map::new(),
                    payload: payload.as_object().expect("typed request object").clone(),
                    asset_access: None,
                    execution_policy: None,
                })
            })
            .collect::<Result<Vec<_>, ApplicationServiceError>>()?;
        let created_at = now();
        let mut query = Map::new();
        query.insert(PREPARED_KEY.to_owned(), frozen);
        self.with_business_runtime(|runtime| {
            let mut store = runtime
                .store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            if let Some(existing) = store.load_search_batch(&batch_id)? {
                if existing.provider_id != PROVIDER
                    || existing.query != query
                    || existing.candidates != candidates
                {
                    return Err(invalid("GACOS prepared batch identity mismatch"));
                }
            } else {
                store.save_search_batch(&StoreSearchBatchRequest {
                    batch: FrozenSearchBatch {
                        schema_version: 1,
                        batch_id: batch_id.clone(),
                        provider_id: PROVIDER.to_owned(),
                        created_at,
                        aoi_name: text(plan, "aoi_name")?.to_owned(),
                        aoi_geojson: Some(geometry),
                        query,
                        search_stats: None,
                        candidates: candidates.clone(),
                    },
                    workbench: SearchWorkbenchState {
                        batch_id: batch_id.clone(),
                        checked_item_ids: candidates
                            .iter()
                            .map(|item| item.item_id.clone())
                            .collect(),
                        page_index: 0,
                        page_size: candidates.len() as u64,
                        revision: 0,
                    },
                    activate_workbench: false,
                })?;
            }
            Ok(())
        })?;
        plan["prepared_batch_id"] = json!(batch_id);
        plan["plan_fingerprint_sha256"] = json!(fingerprint);
        Ok(())
    }
    /// Create a waiting task using only the server-prepared immutable batch.
    fn create_gacos_request_task(
        &self,
        request: &GacosConfirmRequest,
    ) -> Result<StoredTask, ApplicationServiceError> {
        if !request.confirm {
            return Err(GacosRequestError::ConfirmationRequired.into());
        }
        if !opaque_id(&request.task_id) {
            return Err(invalid("GACOS task id is invalid"));
        }
        if request.plan_fingerprint_sha256.len() != 64
            || !request
                .plan_fingerprint_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || request.prepared_batch_id
                != format!("gacos-plan-{}", request.plan_fingerprint_sha256)
        {
            return Err(GacosRequestError::PlanStale.into());
        }
        self.with_business_runtime(|runtime| {
            let mut store = runtime.store.lock().map_err(|_| ApplicationServiceError::StatePoisoned)?;
            let batch = store.load_search_batch(&request.prepared_batch_id)?
                .ok_or_else(|| ApplicationServiceError::from(GacosRequestError::PlanStale))?;
            if batch.provider_id != PROVIDER { return Err(invalid("GACOS prepared provider mismatch")); }
            let frozen = batch.query.get(PREPARED_KEY).ok_or_else(|| invalid("GACOS prepared identity is missing"))?;
            if digest(frozen)? != request.plan_fingerprint_sha256 || batch.batch_id != format!("gacos-plan-{}", request.plan_fingerprint_sha256) {
                return Err(GacosRequestError::PlanStale.into());
            }
            let selected_item_ids = batch.candidates.iter().map(|item| item.item_id.clone()).collect::<Vec<_>>();
            let workbench = SearchWorkbenchState { batch_id: batch.batch_id.clone(),
                checked_item_ids: selected_item_ids.clone(), page_index: 0, page_size: selected_item_ids.len() as u64, revision: 0 };
            let task = StoreCreateTaskRequest { task_id: request.task_id.clone(), batch_id: batch.batch_id.clone(),
                selected_item_ids: selected_item_ids.clone(), checked_item_ids: selected_item_ids,
                page_index: 0, page_size: workbench.page_size,
                output_root: PathBuf::from(text(frozen, "output_directory")?), layout: DownloadLayout::Direct,
                concurrency: 1, created_at: batch.created_at.clone() };
            let operation = TaskOperationDescriptor { operation_id: format!("{}:request", request.task_id),
                kind: "gacos".to_owned(), phase: "request".to_owned(), region_id: batch.batch_id.clone(),
                approved_worker_operation: "gacos_manual_request".to_owned(),
                payload: json!({"prepared_batch_id": batch.batch_id, "plan_fingerprint_sha256": request.plan_fingerprint_sha256,
                    "manual_action_required": true, "automatic_retry_allowed": false}).as_object().unwrap().clone() };
            Ok(store.create_paused_gacos_request_task(&StoreSearchBatchRequest { batch, workbench, activate_workbench: false }, &task, &operation)?)
        })
    }
}

use insar_provider::gacos_delivery::{
    build_form_post, GacosDeliveryAdapter, GacosDeliveryError, GacosFormPostExecutor,
    GacosReqwestTransport, GacosSubmissionOutcome as ProviderOutcome,
};
use insar_store::{
    GacosSubmissionAttemptIdentity, GacosSubmissionOutcome, GacosSubmissionOutcomeRequest,
    GacosSubmissionReserveRequest, GacosSubmissionState, GacosSubmissionSubmittingRequest,
    StoreError,
};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

impl ApplicationService {
    pub fn gacos_submit_request(
        &self,
        request: &GacosConfirmRequest,
    ) -> Result<Value, ApplicationServiceError> {
        if !request.confirm {
            return Err(GacosRequestError::ConfirmationRequired.into());
        }
        // Read-only and verification runtimes have no credential/provider capability.
        self.with_business_runtime(|_| Ok(()))?;
        let transport = GacosReqwestTransport::production_with_route(
            self.network_policy(crate::network_routes::ServiceRoute::Ordinary)
                .route(),
        )
        .map_err(|_| ApplicationServiceError::from(GacosRequestError::TransportUnavailable))?;
        insar_download::with_desktop_gacos_email(|email| {
            self.submit_gacos_with_executor(request, email, transport, Duration::from_secs(20))
        })
        .map_err(|_| ApplicationServiceError::from(GacosRequestError::CredentialsUnavailable))?
        .ok_or_else(|| ApplicationServiceError::from(GacosRequestError::EmailNotConfigured))?
    }

    fn submit_gacos_with_executor<E: GacosFormPostExecutor>(
        &self,
        request: &GacosConfirmRequest,
        email: &str,
        executor: E,
        minimum_interval: Duration,
    ) -> Result<Value, ApplicationServiceError> {
        // One process owns this Store. Serialize form sends separately from its
        // lifecycle mutex so task cancellation remains available during HTTP.
        let mut last_finished = self
            .gacos_submission_gate
            .try_lock()
            .map_err(|_| ApplicationServiceError::from(GacosRequestError::SubmissionBusy))?;
        self.with_business_runtime(|_| Ok(()))?;
        let _process_lease = lock_gacos_submissions(
            self.config
                .store_path()
                .ok_or(ApplicationServiceError::StorePathRequired)?,
        )?;
        let store = self.store_handle()?;
        // With the cross-process lease held, no prior sender can still be
        // executing. A crash left in the attempt ledger now needs manual review.
        store
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?
            .recover_gacos_submitting_attempts(None, &now())?;
        let task = self.create_gacos_request_task(request)?;
        let cancelled = || {
            if self.shutting_down.load(Ordering::Acquire) {
                return true;
            }
            let Ok(store) = store.lock() else {
                return true;
            };
            match store.load_task(&request.task_id) {
                Ok(Some(current)) => {
                    !current.runtime.task_paused
                        || current.runtime.status != insar_core::TaskLifecycleStatus::Paused
                        || current.runtime.revision != task.runtime.revision
                }
                _ => true,
            }
        };
        let adapter = GacosDeliveryAdapter::new(executor);
        let mut account_digest = Sha256::new();
        for byte in email.trim().bytes() {
            account_digest.update([byte.to_ascii_lowercase()]);
        }
        let account_hash = format!("{:x}", account_digest.finalize());
        for item in &task.snapshot.candidates {
            if cancelled() {
                break;
            }
            let frozen: FrozenRequest = serde_json::from_value(Value::Object(item.payload.clone()))
                .map_err(|_| invalid("GACOS frozen form request is invalid"))?;
            let input = frozen.input()?;
            let post = build_form_post(&input, email)
                .map_err(|_| invalid("GACOS request or configured email is invalid"))?;
            let identity = GacosSubmissionAttemptIdentity {
                task_id: request.task_id.clone(),
                item_id: item.item_id.clone(),
                attempt_id: format!("{}:{}:0", request.task_id, item.item_id),
                fingerprint_sha256: digest(&Value::Object(item.payload.clone()))?,
                account_identity_hash_sha256: account_hash.clone(),
                attempt_number: 0,
            };
            let reserve = {
                let mut store = store
                    .lock()
                    .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                store.reserve_gacos_submission_attempt(&GacosSubmissionReserveRequest {
                    identity: identity.clone(),
                    occurred_at: now(),
                    force_reapply: request.force_reapply,
                })
            };
            match reserve {
                Ok(attempt)
                    if matches!(
                        attempt.state,
                        GacosSubmissionState::ManualVerificationRequired
                            | GacosSubmissionState::Submitted
                    ) =>
                {
                    continue
                }
                Ok(attempt) if attempt.state != GacosSubmissionState::Reserved => break,
                Err(StoreError::AlreadyExists) => {
                    return Err(GacosRequestError::DuplicateRequest.into())
                }
                Err(error) => return Err(error.into()),
                Ok(_) => {}
            }
            loop {
                if cancelled() {
                    break;
                }
                let persistent = store
                    .lock()
                    .map_err(|_| ApplicationServiceError::StatePoisoned)?
                    .last_gacos_submission_activity_at()?;
                let persistent_remaining = persistent
                    .as_deref()
                    .map(|timestamp| {
                        let parsed =
                            chrono::DateTime::parse_from_rfc3339(timestamp).map_err(|_| {
                                invalid("GACOS persisted submission timestamp is invalid")
                            })?;
                        let elapsed = Utc::now()
                            .signed_duration_since(parsed)
                            .to_std()
                            .unwrap_or_default();
                        Ok::<_, ApplicationServiceError>(minimum_interval.saturating_sub(elapsed))
                    })
                    .transpose()?
                    .unwrap_or_default();
                let monotonic_remaining = last_finished
                    .as_ref()
                    .map(|last: &Instant| minimum_interval.saturating_sub(last.elapsed()))
                    .unwrap_or_default();
                let remaining = persistent_remaining.max(monotonic_remaining);
                if remaining.is_zero() {
                    break;
                }
                std::thread::sleep(remaining.min(Duration::from_millis(100)));
            }
            if cancelled() {
                store
                    .lock()
                    .map_err(|_| ApplicationServiceError::StatePoisoned)?
                    .record_gacos_submission_outcome(&GacosSubmissionOutcomeRequest {
                        identity,
                        occurred_at: now(),
                        outcome: GacosSubmissionOutcome::Rejected {
                            reason_code: "CANCELLED_BEFORE_SEND".to_owned(),
                            http_status: None,
                        },
                    })?;
                break;
            }
            {
                let mut store = store
                    .lock()
                    .map_err(|_| ApplicationServiceError::StatePoisoned)?;
                // This transition grants exactly one sender. A concurrent or
                // replayed caller cannot obtain permission from Submitting.
                store.mark_gacos_submission_submitting(&GacosSubmissionSubmittingRequest {
                    identity: identity.clone(),
                    occurred_at: now(),
                })?;
            }
            let outcome = provider_outcome(adapter.submit(&post, &cancelled));
            *last_finished = Some(Instant::now());
            let stop = matches!(
                outcome,
                GacosSubmissionOutcome::SubmissionUncertain { .. }
                    | GacosSubmissionOutcome::Rejected { .. }
            );
            store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?
                .record_gacos_submission_outcome(&GacosSubmissionOutcomeRequest {
                    identity,
                    occurred_at: now(),
                    outcome,
                })?;
            if stop {
                break;
            }
        }
        self.gacos_request_status(&request.task_id)
    }

    pub fn gacos_request_status(&self, task_id: &str) -> Result<Value, ApplicationServiceError> {
        self.ensure_running()?;
        let store = self.store_handle()?;
        let store = store
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?;
        let task = store
            .load_task_record(task_id)?
            .ok_or_else(|| invalid("GACOS task is unavailable"))?;
        insar_core::validate_gacos_manual_request_operation(
            task.snapshot
                .operation
                .as_ref()
                .ok_or_else(|| invalid("GACOS request operation is missing"))?,
        )
        .map_err(|_| invalid("GACOS request operation is invalid"))?;
        let request_groups = task.snapshot.candidates.iter()
            .filter(|item| task.snapshot.initial_selected_item_ids.contains(&item.item_id))
            .map(|item| {
                let frozen: FrozenRequest = serde_json::from_value(Value::Object(item.payload.clone()))
                    .map_err(|_| invalid("GACOS frozen request is invalid"))?;
                frozen.input()?;
                Ok(json!({"item_id":item.item_id,"dates":frozen.dates,"hour":frozen.hour,"minute":frozen.minute,"bbox":frozen.bbox}))
            }).collect::<Result<Vec<_>, ApplicationServiceError>>()?;
        let attempts = store.read_gacos_submission_attempts(task_id, None, 256)?;
        let latest_attempts = store.read_latest_gacos_submission_attempts(task_id)?;
        let records = attempts.iter().map(|attempt| json!({
            "attempt_id": attempt.attempt_id, "task_id": attempt.task_id, "item_id": attempt.item_id,
            "attempt_number": attempt.attempt_number, "state": attempt.state,
            "reason_code": attempt.reason_code, "http_status": attempt.http_status,
            "automatic_retry_allowed": false, "created_at": attempt.created_at, "updated_at": attempt.updated_at,
        })).collect::<Vec<_>>();
        let latest_state = |item_id: &str| {
            latest_attempts
                .iter()
                .find(|attempt| attempt.item_id == item_id)
                .map(|attempt| attempt.state)
        };
        let submitted_count = request_groups
            .iter()
            .filter(|group| {
                group
                    .get("item_id")
                    .and_then(Value::as_str)
                    .and_then(&latest_state)
                    == Some(GacosSubmissionState::Submitted)
            })
            .count();
        let manual_action_required = request_groups.iter().any(|group| {
            group
                .get("item_id")
                .and_then(Value::as_str)
                .and_then(&latest_state)
                .is_some_and(|state| {
                    matches!(
                        state,
                        GacosSubmissionState::ManualVerificationRequired
                            | GacosSubmissionState::SubmissionUncertain
                    )
                })
        });
        let rejected_count = request_groups
            .iter()
            .filter(|group| {
                group
                    .get("item_id")
                    .and_then(Value::as_str)
                    .and_then(&latest_state)
                    == Some(GacosSubmissionState::Rejected)
            })
            .count();
        let not_sent_count = request_groups
            .iter()
            .filter(|group| {
                matches!(
                    group
                        .get("item_id")
                        .and_then(Value::as_str)
                        .and_then(&latest_state),
                    None | Some(GacosSubmissionState::Reserved)
                        | Some(GacosSubmissionState::Submitting)
                )
            })
            .count();
        let application_completed =
            !request_groups.is_empty() && submitted_count == request_groups.len();
        let message = if application_completed {
            "申请已提交"
        } else if manual_action_required && submitted_count > 0 {
            "部分申请已提交，仍有申请无法确认"
        } else if manual_action_required {
            "无法确认申请状态"
        } else if submitted_count > 0 && rejected_count > 0 {
            "部分申请已提交，部分申请被明确拒绝"
        } else if submitted_count > 0 && not_sent_count > 0 {
            "部分申请已提交，仍有批次尚未发送"
        } else if rejected_count > 0 && not_sent_count > 0 {
            "部分申请被拒绝，仍有批次尚未发送"
        } else if rejected_count == request_groups.len() && !request_groups.is_empty() {
            "申请被明确拒绝"
        } else {
            "申请尚未发送"
        };
        Ok(json!({
            "ok": true,
            "task_id": task_id,
            "records": records,
            "request_groups": request_groups,
            "task_status": task.runtime.status,
            "submitted_count": submitted_count,
            "application_completed": application_completed,
            "manual_action_required": manual_action_required,
            "automatic_retry_allowed": false,
            "message": message,
        }))
    }
}

fn provider_outcome(
    result: Result<insar_provider::gacos_delivery::GacosSubmissionReceipt, GacosDeliveryError>,
) -> GacosSubmissionOutcome {
    match result {
        Ok(receipt) => match receipt.outcome() {
            ProviderOutcome::Submitted => GacosSubmissionOutcome::Submitted {
                reason_code: "EXPLICIT_SUCCESS_EVIDENCE".to_owned(),
                http_status: receipt.http_status(),
            },
            ProviderOutcome::ManualVerificationRequired => {
                GacosSubmissionOutcome::ManualVerificationRequired {
                    reason_code: "NO_EXPLICIT_SUCCESS_EVIDENCE".to_owned(),
                    http_status: receipt.http_status(),
                }
            }
            ProviderOutcome::Rejected => GacosSubmissionOutcome::Rejected {
                reason_code: "PROVIDER_REJECTED".to_owned(),
                http_status: Some(receipt.http_status()),
            },
            ProviderOutcome::Uncertain => GacosSubmissionOutcome::SubmissionUncertain {
                reason_code: "ACCEPTANCE_UNCERTAIN".to_owned(),
                http_status: Some(receipt.http_status()),
            },
        },
        Err(GacosDeliveryError::CancelledBeforeSend) => GacosSubmissionOutcome::Rejected {
            reason_code: "CANCELLED_BEFORE_SEND".to_owned(),
            http_status: None,
        },
        Err(_) => GacosSubmissionOutcome::SubmissionUncertain {
            reason_code: "TRANSPORT_OUTCOME_UNCERTAIN".to_owned(),
            http_status: None,
        },
    }
}

#[cfg(test)]
#[path = "gacos_service_tests.rs"]
mod tests;
