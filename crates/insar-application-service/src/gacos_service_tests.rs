use super::*;
use crate::{ApplicationServiceConfig, ApplicationSetWorkingAoiRequest};
use insar_core::{
    AsfRemoteSearchRequest, AsfRemoteSearchResponse, DownloadItemProgress, DownloadItemResult,
    ProviderResult, StoreArchiveTaskRequest, StoreTaskCommand, StoreUpdateTaskRequest,
    TaskStateCommandKind,
};
use insar_download::{DownloadWork, DownloadWorker, WorkerControl};
use insar_provider::gacos_delivery::{GacosDeliveryError, GacosFormPost, GacosHttpResponse};
use insar_service::LiveAsfSearch;
use insar_store::{
    GacosSubmissionAttemptIdentity, GacosSubmissionOutcome, GacosSubmissionOutcomeRequest,
    GacosSubmissionReserveRequest, GacosSubmissionState, GacosSubmissionSubmittingRequest,
};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize},
    Arc, Mutex,
};
use std::thread;
use std::time::{Duration, Instant};
struct NeverSearch;
impl LiveAsfSearch for NeverSearch {
    fn search(
        &self,
        _: &AsfRemoteSearchRequest,
        _: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<AsfRemoteSearchResponse> {
        panic!("GACOS must not perform catalogue search during submission")
    }
}
struct NeverWorker;
impl DownloadWorker for NeverWorker {
    fn execute(
        &self,
        _: DownloadWork,
        _: WorkerControl,
        _: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        panic!("waiting GACOS request must never reach scheduler worker")
    }
}
struct Fixture {
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "gacos-native-request-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        Self { root }
    }
    fn service(&self) -> ApplicationService {
        ApplicationService::with_fixture_business_runtime(
            ApplicationServiceConfig::business_store(self.root.join("tasks.sqlite3")),
            Arc::new(NeverSearch),
            Arc::new(NeverWorker),
        )
    }
    fn prepare(&self, service: &ApplicationService, days: usize) -> GacosConfirmRequest {
        self.prepare_with_format(service, days, None)
    }

    fn prepare_with_format(
        &self,
        service: &ApplicationService,
        days: usize,
        output_format: Option<&str>,
    ) -> GacosConfirmRequest {
        let geometry = json!({"type":"Polygon","coordinates":[[[100.,30.],[100.1,30.],[100.1,30.1],[100.,30.1],[100.,30.]]]}).as_object().unwrap().clone();
        service
            .set_working_aoi(&ApplicationSetWorkingAoiRequest {
                source_kind: "manual_bbox".to_owned(),
                geometry: geometry.clone(),
                bbox: vec![100., 30., 100.1, 30.1],
                crs: "EPSG:4326".to_owned(),
                display_name: "Fixture AOI".to_owned(),
            })
            .unwrap();
        let scenes = (1..=days)
            .map(|day| {
                json!({"scene_id": format!("scene-{day}"),
            "acquisition_datetime": format!("2024-01-{day:02}T07:08:00Z")})
            })
            .collect::<Vec<_>>();
        let mut payload = json!({"request_id":"fixture-plan", "aoi_name":"Fixture AOI", "aoi_geojson":geometry,
            "output_dir":self.root.join("output"), "selected_scene_ids":scenes.iter().map(|scene|scene["scene_id"].clone()).collect::<Vec<_>>(),
            "scenes":scenes});
        if let Some(output_format) = output_format {
            payload["output_format"] = json!(output_format);
        }
        let planned = service.gacos_plan(&payload).unwrap();
        assert_eq!(planned["report"]["valid"], true);
        GacosConfirmRequest {
            prepared_batch_id: planned["plan"]["prepared_batch_id"]
                .as_str()
                .unwrap()
                .to_owned(),
            plan_fingerprint_sha256: planned["plan"]["plan_fingerprint_sha256"]
                .as_str()
                .unwrap()
                .to_owned(),
            task_id: "gacos-task".to_owned(),
            confirm: true,
            force_reapply: false,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
struct Executor {
    calls: Arc<AtomicUsize>,
    uncertain: bool,
}
impl GacosFormPostExecutor for Executor {
    fn execute_form_post(
        &self,
        post: &GacosFormPost,
        _: &dyn Fn() -> bool,
    ) -> Result<GacosHttpResponse, GacosDeliveryError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert!(post
            .body()
            .windows(10)
            .any(|window| window == b"email=test"));
        if self.uncertain {
            return Err(GacosDeliveryError::SubmissionUncertain);
        }
        GacosHttpResponse::new(200, b"Your request is being reviewed".to_vec())
    }
}
struct SubmittedExecutor {
    calls: Arc<AtomicUsize>,
}
impl GacosFormPostExecutor for SubmittedExecutor {
    fn execute_form_post(
        &self,
        post: &GacosFormPost,
        _: &dyn Fn() -> bool,
    ) -> Result<GacosHttpResponse, GacosDeliveryError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert!(post
            .body()
            .windows(10)
            .any(|window| window == b"email=test"));
        GacosHttpResponse::new(200, b"Request received".to_vec())
    }
}
struct FormTypeExecutor {
    calls: Arc<AtomicUsize>,
    expected_type: &'static [u8],
}
impl GacosFormPostExecutor for FormTypeExecutor {
    fn execute_form_post(
        &self,
        post: &GacosFormPost,
        _: &dyn Fn() -> bool,
    ) -> Result<GacosHttpResponse, GacosDeliveryError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert!(post
            .body()
            .windows(10)
            .any(|window| window == b"email=test"));
        assert!(post
            .body()
            .windows(self.expected_type.len())
            .any(|window| window == self.expected_type));
        GacosHttpResponse::new(200, b"Request received".to_vec())
    }
}
fn execute(
    service: &ApplicationService,
    request: &GacosConfirmRequest,
    calls: &Arc<AtomicUsize>,
    uncertain: bool,
) -> Result<Value, ApplicationServiceError> {
    service.submit_gacos_with_executor(
        request,
        "test@example.invalid",
        Executor {
            calls: Arc::clone(calls),
            uncertain,
        },
        Duration::ZERO,
    )
}

#[test]
fn prepared_confirmation_uses_store_identity_and_submits_each_form_only_once() {
    let fixture = Fixture::new();
    let service = fixture.service();
    let request = fixture.prepare(&service, 1);
    assert!(service.restore(true).unwrap().tasks.is_empty());
    let calls = Arc::new(AtomicUsize::new(0));
    let result = execute(&service, &request, &calls, false).unwrap();
    assert_eq!(
        result["records"][0]["state"],
        "manual_verification_required"
    );
    assert_eq!(result["automatic_retry_allowed"], false);
    execute(&service, &request, &calls, false).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let store = service.store_handle().unwrap();
    let task = store
        .lock()
        .unwrap()
        .load_task(&request.task_id)
        .unwrap()
        .unwrap();
    assert_eq!(task.snapshot.output_root, fixture.root.join("output"));
    assert_eq!(task.runtime.status, insar_core::TaskLifecycleStatus::Paused);
    assert!(task.runtime.completed_item_ids.is_empty());
    drop(store);
    service.shutdown().unwrap();
    drop(service);
    let reopened = fixture.service();
    execute(&reopened, &request, &calls, false).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    reopened.shutdown().unwrap();
    drop(reopened);
    for suffix in ["", "-wal"] {
        if let Ok(bytes) = std::fs::read(fixture.root.join(format!("tasks.sqlite3{suffix}"))) {
            assert!(!bytes
                .windows(b"test@example.invalid".len())
                .any(|chunk| chunk == b"test@example.invalid"));
            assert!(!bytes
                .windows(b"email=test".len())
                .any(|chunk| chunk == b"email=test"));
        }
    }
}

#[test]
fn gacos_output_format_preserves_legacy_geotiff_identity_and_separates_binary() {
    let fixture = Fixture::new();
    let service = fixture.service();
    let legacy = fixture.prepare_with_format(&service, 1, None);
    let mut explicit_geotiff = fixture.prepare_with_format(&service, 1, Some("geotiff"));
    explicit_geotiff.task_id = "gacos-geotiff-explicit".to_owned();
    let legacy_task = service.create_gacos_request_task(&legacy).unwrap();
    let explicit_task = service
        .create_gacos_request_task(&explicit_geotiff)
        .unwrap();
    assert_eq!(legacy.prepared_batch_id, explicit_geotiff.prepared_batch_id);
    assert_eq!(
        legacy.plan_fingerprint_sha256,
        explicit_geotiff.plan_fingerprint_sha256
    );
    assert_eq!(
        legacy_task.snapshot.candidates[0].item_id,
        explicit_task.snapshot.candidates[0].item_id
    );
    assert_eq!(
        legacy_task.snapshot.candidates[0].payload,
        explicit_task.snapshot.candidates[0].payload
    );
    assert!(!legacy_task.snapshot.candidates[0]
        .payload
        .contains_key("output_format"));
    let buffered_west = 100.0_f64 - 0.05;
    let normalized_west = (buffered_west + 180.0).rem_euclid(360.0) - 180.0;
    let world_shift = buffered_west - normalized_west;
    let expected_legacy_payload = json!({
        "bbox": [
            normalized_west,
            (30.0_f64 - 0.05).max(-90.0),
            (100.1_f64 + 0.05) - world_shift,
            (30.1_f64 + 0.05).min(90.0)
        ],
        "dates": ["2024-01-01"],
        "hour": 7,
        "minute": 8
    })
    .as_object()
    .unwrap()
    .clone();
    assert_eq!(
        legacy_task.snapshot.candidates[0].payload,
        expected_legacy_payload
    );

    let mut binary = fixture.prepare_with_format(&service, 1, Some("binary"));
    binary.task_id = "gacos-binary".to_owned();
    let binary_task = service.create_gacos_request_task(&binary).unwrap();
    assert_ne!(legacy.prepared_batch_id, binary.prepared_batch_id);
    assert_ne!(
        legacy_task.snapshot.candidates[0].item_id,
        binary_task.snapshot.candidates[0].item_id
    );
    assert_ne!(
        legacy_task.snapshot.candidates[0].payload,
        binary_task.snapshot.candidates[0].payload
    );
    assert_eq!(
        binary_task.snapshot.candidates[0].payload["output_format"],
        "binary"
    );
    service.shutdown().unwrap();
}

#[test]
fn gacos_output_format_controls_provider_form_type_for_default_geotiff_and_binary() {
    let fixture = Fixture::new();
    let service = fixture.service();
    let default_geotiff = fixture.prepare_with_format(&service, 1, None);
    let mut explicit_geotiff = fixture.prepare_with_format(&service, 1, Some("geotiff"));
    explicit_geotiff.task_id = "gacos-geotiff-explicit".to_owned();
    explicit_geotiff.force_reapply = true;
    let mut binary = fixture.prepare_with_format(&service, 1, Some("binary"));
    binary.task_id = "gacos-binary".to_owned();

    let calls = Arc::new(AtomicUsize::new(0));
    for (request, expected_type) in [
        (&default_geotiff, b"type=2".as_slice()),
        (&explicit_geotiff, b"type=2".as_slice()),
        (&binary, b"type=1".as_slice()),
    ] {
        let result = service
            .submit_gacos_with_executor(
                request,
                "test@example.invalid",
                FormTypeExecutor {
                    calls: Arc::clone(&calls),
                    expected_type,
                },
                Duration::ZERO,
            )
            .unwrap();
        assert_eq!(result["records"][0]["state"], "submitted");
    }
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    service.shutdown().unwrap();
}

#[test]
fn invalid_gacos_output_format_is_rejected_before_request_task_creation() {
    let fixture = Fixture::new();
    let service = fixture.service();
    let _ = fixture.prepare(&service, 1);
    let geometry = json!({"type":"Polygon","coordinates":[[[100.,30.],[100.1,30.],[100.1,30.1],[100.,30.1],[100.,30.]]]});
    let scenes = vec![json!({
        "scene_id": "scene-1",
        "acquisition_datetime": "2024-01-01T07:08:00Z"
    })];
    let error = service
        .gacos_plan(&json!({
            "request_id": "invalid-format-plan",
            "aoi_name": "Fixture AOI",
            "aoi_geojson": geometry,
            "output_dir": fixture.root.join("output"),
            "selected_scene_ids": ["scene-1"],
            "scenes": scenes,
            "output_format": "unsupported"
        }))
        .unwrap_err();
    assert_eq!(error.code(), "V3_APPLICATION_INVALID_REQUEST");
    assert!(service.restore(true).unwrap().tasks.is_empty());
    service.shutdown().unwrap();
}

#[test]
fn result_parent_projection_accepts_legacy_and_binary_but_rejects_unknown_or_malformed_payloads() {
    let legacy = json!({
        "bbox": [100.0, 30.0, 100.1, 30.1],
        "dates": ["2024-01-01"],
        "hour": 7,
        "minute": 8,
    });
    let projected = project_gacos_result_parent_parameters(legacy.as_object().unwrap()).unwrap();
    assert_eq!(projected.bbox, [100.0, 30.0, 100.1, 30.1]);
    assert_eq!(projected.dates, vec!["2024-01-01".to_owned()]);
    assert_eq!(projected.hour, 7);
    assert_eq!(projected.minute, 8);

    let mut binary = legacy.clone();
    binary["output_format"] = json!("binary");
    let binary_projected =
        project_gacos_result_parent_parameters(binary.as_object().unwrap()).unwrap();
    assert_eq!(binary_projected.bbox, projected.bbox);
    assert_eq!(binary_projected.dates, projected.dates);
    assert_eq!(binary_projected.hour, projected.hour);
    assert_eq!(binary_projected.minute, projected.minute);

    let mut unknown = legacy.clone();
    unknown["unexpected_parent_field"] = json!(true);
    assert!(project_gacos_result_parent_parameters(unknown.as_object().unwrap()).is_err());

    let mut malformed = legacy;
    malformed["output_format"] = json!("unsupported");
    assert!(project_gacos_result_parent_parameters(malformed.as_object().unwrap()).is_err());
}

#[test]
fn explicit_success_marks_submitted_without_completing_the_request_task() {
    let fixture = Fixture::new();
    let service = fixture.service();
    let request = fixture.prepare(&service, 1);
    let calls = Arc::new(AtomicUsize::new(0));
    let result = service
        .submit_gacos_with_executor(
            &request,
            "test@example.invalid",
            SubmittedExecutor {
                calls: Arc::clone(&calls),
            },
            Duration::ZERO,
        )
        .unwrap();
    assert_eq!(result["records"][0]["state"], "submitted");
    assert_eq!(
        result["records"][0]["reason_code"],
        "EXPLICIT_SUCCESS_EVIDENCE"
    );
    assert_eq!(result["records"][0]["http_status"], 200);
    assert_eq!(result["submitted_count"], 1);
    assert_eq!(result["application_completed"], true);
    assert_eq!(result["manual_action_required"], false);
    assert_eq!(result["message"], "申请已提交");
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    let replay = service
        .submit_gacos_with_executor(
            &request,
            "test@example.invalid",
            SubmittedExecutor {
                calls: Arc::clone(&calls),
            },
            Duration::ZERO,
        )
        .unwrap();
    assert_eq!(replay["application_completed"], true);
    assert_eq!(replay["submitted_count"], 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    let task = service
        .store_handle()
        .unwrap()
        .lock()
        .unwrap()
        .load_task(&request.task_id)
        .unwrap()
        .unwrap();
    assert_eq!(task.runtime.status, insar_core::TaskLifecycleStatus::Paused);
    assert!(task.runtime.completed_item_ids.is_empty());
    service.shutdown().unwrap();
}

#[test]
fn submitted_success_continues_through_all_frozen_batches() {
    let fixture = Fixture::new();
    let service = fixture.service();
    let request = fixture.prepare(&service, 21);
    let calls = Arc::new(AtomicUsize::new(0));
    let result = service
        .submit_gacos_with_executor(
            &request,
            "test@example.invalid",
            SubmittedExecutor {
                calls: Arc::clone(&calls),
            },
            Duration::ZERO,
        )
        .unwrap();

    assert_eq!(result["records"].as_array().unwrap().len(), 2);
    assert!(result["records"]
        .as_array()
        .unwrap()
        .iter()
        .all(|record| record["state"] == "submitted"));
    assert_eq!(result["submitted_count"], 2);
    assert_eq!(result["application_completed"], true);
    assert_eq!(result["manual_action_required"], false);
    assert_eq!(result["message"], "申请已提交");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    service.shutdown().unwrap();
}

#[test]
fn empty_or_partially_submitted_request_is_not_application_complete() {
    let fixture = Fixture::new();
    let service = fixture.service();
    let request = fixture.prepare(&service, 21);
    let task = service.create_gacos_request_task(&request).unwrap();

    let empty = service.gacos_request_status(&request.task_id).unwrap();
    assert_eq!(empty["submitted_count"], 0);
    assert_eq!(empty["application_completed"], false);
    assert_eq!(empty["message"], "申请尚未发送");

    let first = task.snapshot.candidates.first().unwrap();
    let identity = GacosSubmissionAttemptIdentity {
        task_id: request.task_id.clone(),
        item_id: first.item_id.clone(),
        attempt_id: format!("{}:{}:0", request.task_id, first.item_id),
        fingerprint_sha256: digest(&Value::Object(first.payload.clone())).unwrap(),
        account_identity_hash_sha256: "b".repeat(64),
        attempt_number: 0,
    };
    let store = service.store_handle().unwrap();
    let mut store = store.lock().unwrap();
    store
        .reserve_gacos_submission_attempt(&GacosSubmissionReserveRequest {
            identity: identity.clone(),
            occurred_at: "2026-09-06T00:00:00Z".to_owned(),
            force_reapply: false,
        })
        .unwrap();
    store
        .mark_gacos_submission_submitting(&GacosSubmissionSubmittingRequest {
            identity: identity.clone(),
            occurred_at: "2026-09-06T00:00:01Z".to_owned(),
        })
        .unwrap();
    store
        .record_gacos_submission_outcome(&GacosSubmissionOutcomeRequest {
            identity,
            occurred_at: "2026-09-06T00:00:02Z".to_owned(),
            outcome: GacosSubmissionOutcome::Submitted {
                reason_code: "EXPLICIT_SUCCESS_EVIDENCE".to_owned(),
                http_status: 200,
            },
        })
        .unwrap();
    drop(store);

    let partial = service.gacos_request_status(&request.task_id).unwrap();
    assert_eq!(partial["request_groups"].as_array().unwrap().len(), 2);
    assert_eq!(partial["submitted_count"], 1);
    assert_eq!(partial["application_completed"], false);
    assert_eq!(partial["manual_action_required"], false);
    assert_eq!(partial["message"], "部分申请已提交，仍有批次尚未发送");
    service.shutdown().unwrap();
}

#[test]
fn partially_rejected_request_reports_unsubmitted_batches() {
    let fixture = Fixture::new();
    let service = fixture.service();
    let request = fixture.prepare(&service, 21);
    let task = service.create_gacos_request_task(&request).unwrap();
    let first = task.snapshot.candidates.first().unwrap();
    let identity = GacosSubmissionAttemptIdentity {
        task_id: request.task_id.clone(),
        item_id: first.item_id.clone(),
        attempt_id: format!("{}:{}:0", request.task_id, first.item_id),
        fingerprint_sha256: digest(&Value::Object(first.payload.clone())).unwrap(),
        account_identity_hash_sha256: "b".repeat(64),
        attempt_number: 0,
    };
    let store = service.store_handle().unwrap();
    let mut store = store.lock().unwrap();
    store
        .reserve_gacos_submission_attempt(&GacosSubmissionReserveRequest {
            identity: identity.clone(),
            occurred_at: "2026-09-06T00:00:00Z".to_owned(),
            force_reapply: false,
        })
        .unwrap();
    store
        .mark_gacos_submission_submitting(&GacosSubmissionSubmittingRequest {
            identity: identity.clone(),
            occurred_at: "2026-09-06T00:00:01Z".to_owned(),
        })
        .unwrap();
    store
        .record_gacos_submission_outcome(&GacosSubmissionOutcomeRequest {
            identity,
            occurred_at: "2026-09-06T00:00:02Z".to_owned(),
            outcome: GacosSubmissionOutcome::Rejected {
                reason_code: "PROVIDER_REJECTED".to_owned(),
                http_status: Some(400),
            },
        })
        .unwrap();
    drop(store);

    let status = service.gacos_request_status(&request.task_id).unwrap();
    assert_eq!(status["submitted_count"], 0);
    assert_eq!(status["application_completed"], false);
    assert_eq!(status["manual_action_required"], false);
    assert_eq!(status["message"], "部分申请被拒绝，仍有批次尚未发送");
    service.shutdown().unwrap();
}

#[test]
fn unconfirmed_or_tampered_plan_cannot_create_task_or_send() {
    let fixture = Fixture::new();
    let service = fixture.service();
    let mut request = fixture.prepare(&service, 1);
    let calls = Arc::new(AtomicUsize::new(0));
    request.confirm = false;
    assert!(execute(&service, &request, &calls, false).is_err());
    request.confirm = true;
    request.plan_fingerprint_sha256 = "0".repeat(64);
    assert!(execute(&service, &request, &calls, false).is_err());
    assert!(service.restore(true).unwrap().tasks.is_empty());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    service.shutdown().unwrap();
}
#[test]
fn uncertain_submission_stops_the_batch_and_reconfirmation_never_sends_more() {
    let fixture = Fixture::new();
    let service = fixture.service();
    let request = fixture.prepare(&service, 21);
    let calls = Arc::new(AtomicUsize::new(0));
    let result = execute(&service, &request, &calls, true).unwrap();
    assert_eq!(result["records"].as_array().unwrap().len(), 1);
    assert_eq!(result["records"][0]["state"], "submission_uncertain");
    execute(&service, &request, &calls, false).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    service.shutdown().unwrap();
}
#[test]
fn duplicate_across_tasks_needs_explicit_reapply_and_keeps_both_receipts() {
    let fixture = Fixture::new();
    let service = fixture.service();
    let mut request = fixture.prepare(&service, 1);
    let calls = Arc::new(AtomicUsize::new(0));
    execute(&service, &request, &calls, false).unwrap();
    request.task_id = "gacos-second".to_owned();
    assert!(execute(&service, &request, &calls, false).is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    request.force_reapply = true;
    execute(&service, &request, &calls, false).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        service.gacos_request_status("gacos-task").unwrap()["records"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        service.gacos_request_status("gacos-second").unwrap()["records"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    service.shutdown().unwrap();
}

struct GateExecutor {
    calls: Arc<AtomicUsize>,
    entered: Arc<AtomicBool>,
    release: Arc<AtomicBool>,
    uncertain_on_release: bool,
}
impl GacosFormPostExecutor for GateExecutor {
    fn execute_form_post(
        &self,
        post: &GacosFormPost,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<GacosHttpResponse, GacosDeliveryError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert!(post
            .body()
            .windows(10)
            .any(|window| window == b"email=test"));
        self.entered.store(true, Ordering::SeqCst);
        while !self.release.load(Ordering::SeqCst) {
            if cancelled() {
                return Err(GacosDeliveryError::SubmissionUncertain);
            }
            thread::sleep(Duration::from_millis(1));
        }
        if self.uncertain_on_release {
            Err(GacosDeliveryError::SubmissionUncertain)
        } else {
            GacosHttpResponse::new(200, b"Your request is being reviewed".to_vec())
        }
    }
}

struct TimingExecutor {
    calls: Arc<Mutex<Vec<Instant>>>,
}
impl GacosFormPostExecutor for TimingExecutor {
    fn execute_form_post(
        &self,
        post: &GacosFormPost,
        _: &dyn Fn() -> bool,
    ) -> Result<GacosHttpResponse, GacosDeliveryError> {
        assert!(post
            .body()
            .windows(10)
            .any(|window| window == b"email=test"));
        self.calls.lock().unwrap().push(Instant::now());
        GacosHttpResponse::new(200, b"Your request is being reviewed".to_vec())
    }
}

#[test]
fn concurrent_confirmation_has_one_post_claim() {
    let fixture = Fixture::new();
    let service = Arc::new(fixture.service());
    let request = fixture.prepare(&service, 1);
    let calls = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let worker_service = Arc::clone(&service);
    let worker_request = request.clone();
    let worker_calls = Arc::clone(&calls);
    let worker_entered = Arc::clone(&entered);
    let worker_release = Arc::clone(&release);
    let handle = thread::spawn(move || {
        worker_service.submit_gacos_with_executor(
            &worker_request,
            "test@example.invalid",
            GateExecutor {
                calls: worker_calls,
                entered: worker_entered,
                release: worker_release,
                uncertain_on_release: false,
            },
            Duration::ZERO,
        )
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    while !entered.load(Ordering::SeqCst) {
        assert!(
            Instant::now() < deadline,
            "first submission did not enter executor"
        );
        thread::sleep(Duration::from_millis(1));
    }
    let duplicate = service.submit_gacos_with_executor(
        &request,
        "test@example.invalid",
        Executor {
            calls: Arc::clone(&calls),
            uncertain: false,
        },
        Duration::ZERO,
    );
    assert_eq!(duplicate.unwrap_err().code(), "GACOS_SUBMISSION_BUSY");
    release.store(true, Ordering::SeqCst);
    handle.join().unwrap().unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    service.shutdown().unwrap();
}

#[test]
fn cancelled_active_submission_preserves_outcome_and_defers_archive_until_after_record() {
    let fixture = Fixture::new();
    let service = Arc::new(fixture.service());
    let request = fixture.prepare(&service, 1);
    let calls = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let worker_service = Arc::clone(&service);
    let worker_request = request.clone();
    let worker_calls = Arc::clone(&calls);
    let worker_entered = Arc::clone(&entered);
    let worker_release = Arc::clone(&release);
    let handle = thread::spawn(move || {
        worker_service.submit_gacos_with_executor(
            &worker_request,
            "test@example.invalid",
            GateExecutor {
                calls: worker_calls,
                entered: worker_entered,
                release: worker_release,
                uncertain_on_release: true,
            },
            Duration::ZERO,
        )
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    while !entered.load(Ordering::SeqCst) {
        assert!(
            Instant::now() < deadline,
            "submission did not enter executor"
        );
        thread::sleep(Duration::from_millis(1));
    }
    let blocked_archive = service.archive_download_task(&StoreArchiveTaskRequest {
        task_id: request.task_id.clone(),
        expected_revision: 0,
        archived_at: "2026-09-06T00:01:00Z".to_owned(),
    });
    assert_eq!(
        blocked_archive.unwrap_err().code(),
        "GACOS_SUBMISSION_IN_FLIGHT"
    );
    let cancelled = service
        .update_download_task(&StoreUpdateTaskRequest {
            task_id: request.task_id.clone(),
            expected_revision: 0,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::CancelTask,
                item_ids: vec![],
            },
            event_id: "gacos-service-test-cancel".to_owned(),
            occurred_at: "2026-09-06T00:01:01Z".to_owned(),
        })
        .unwrap();
    assert_eq!(
        cancelled.core.tasks[0].status,
        insar_core::TaskLifecycleStatus::Cancelled
    );
    release.store(true, Ordering::SeqCst);
    let result = handle.join().unwrap().unwrap();
    assert_eq!(result["records"][0]["state"], "submission_uncertain");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let archived = service
        .archive_download_task(&StoreArchiveTaskRequest {
            task_id: request.task_id,
            expected_revision: 1,
            archived_at: "2026-09-06T00:01:02Z".to_owned(),
        })
        .unwrap();
    assert_eq!(archived.task_id, "gacos-task");
    service.shutdown().unwrap();
}

#[test]
fn injected_minimum_interval_spaces_multiple_frozen_forms() {
    let fixture = Fixture::new();
    let service = fixture.service();
    let request = fixture.prepare(&service, 21);
    let calls = Arc::new(Mutex::new(Vec::new()));
    service
        .submit_gacos_with_executor(
            &request,
            "test@example.invalid",
            TimingExecutor {
                calls: Arc::clone(&calls),
            },
            Duration::from_millis(20),
        )
        .unwrap();
    let times = calls.lock().unwrap().clone();
    assert_eq!(times.len(), 2);
    assert!(times[1].duration_since(times[0]) >= Duration::from_millis(20));
    service.shutdown().unwrap();
}

#[test]
fn verification_only_submit_is_rejected_before_provider_capability() {
    let path = std::env::temp_dir().join(format!(
        "gacos-verification-only-missing-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let service = ApplicationService::new(ApplicationServiceConfig::verification_only(path));
    let error = service
        .gacos_submit_request(&GacosConfirmRequest {
            prepared_batch_id: "gacos-plan-invalid".to_owned(),
            plan_fingerprint_sha256: "0".repeat(64),
            task_id: "gacos-task".to_owned(),
            confirm: true,
            force_reapply: false,
        })
        .unwrap_err();
    assert_eq!(error.code(), "V3_APPLICATION_BUSINESS_RUNTIME_UNAVAILABLE");
    service.shutdown().unwrap();
}

#[test]
fn persisted_activity_spaces_submission_after_service_reopen() {
    let fixture = Fixture::new();
    let service = fixture.service();
    let first = fixture.prepare(&service, 1);
    let calls = Arc::new(Mutex::new(Vec::new()));
    service
        .submit_gacos_with_executor(
            &first,
            "test@example.invalid",
            TimingExecutor {
                calls: Arc::clone(&calls),
            },
            Duration::ZERO,
        )
        .unwrap();
    service.shutdown().unwrap();

    let reopened = fixture.service();
    let mut second = fixture.prepare(&reopened, 1);
    second.task_id = "gacos-second".to_owned();
    second.force_reapply = true;
    reopened
        .submit_gacos_with_executor(
            &second,
            "test@example.invalid",
            TimingExecutor {
                calls: Arc::clone(&calls),
            },
            Duration::from_millis(40),
        )
        .unwrap();
    let times = calls.lock().unwrap().clone();
    assert_eq!(times.len(), 2);
    assert!(
        times[1].duration_since(times[0]) >= Duration::from_millis(40),
        "reopened submission bypassed the persisted pacing interval"
    );
    reopened.shutdown().unwrap();
}

#[test]
fn cross_service_send_lease_preserves_live_submission_and_blocks_second_confirmation() {
    let fixture = Fixture::new();
    let first = Arc::new(fixture.service());
    let request = fixture.prepare(&first, 1);
    let calls = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let worker_service = Arc::clone(&first);
    let worker_request = request.clone();
    let worker_calls = Arc::clone(&calls);
    let worker_entered = Arc::clone(&entered);
    let worker_release = Arc::clone(&release);
    let handle = thread::spawn(move || {
        worker_service.submit_gacos_with_executor(
            &worker_request,
            "test@example.invalid",
            GateExecutor {
                calls: worker_calls,
                entered: worker_entered,
                release: worker_release,
                uncertain_on_release: false,
            },
            Duration::ZERO,
        )
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    while !entered.load(Ordering::SeqCst) {
        assert!(
            Instant::now() < deadline,
            "first service did not enter the blocked executor"
        );
        thread::sleep(Duration::from_millis(1));
    }

    let second = fixture.service();
    let second_startup = second.startup();
    let attempts_before = second.store_handle().ok().and_then(|store| {
        store.lock().ok().and_then(|store| {
            store
                .read_gacos_submission_attempts(&request.task_id, None, 8)
                .ok()
        })
    });
    let second_result = second.submit_gacos_with_executor(
        &request,
        "test@example.invalid",
        Executor {
            calls: Arc::clone(&calls),
            uncertain: false,
        },
        Duration::ZERO,
    );
    let attempts_after = second.store_handle().ok().and_then(|store| {
        store.lock().ok().and_then(|store| {
            store
                .read_gacos_submission_attempts(&request.task_id, None, 8)
                .ok()
        })
    });

    release.store(true, Ordering::SeqCst);
    let first_result = handle.join().unwrap().unwrap();

    assert!(second_startup.is_ok());
    assert_eq!(second_result.unwrap_err().code(), "GACOS_SUBMISSION_BUSY");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let attempts_before = attempts_before.expect("second service could read live attempt");
    assert_eq!(attempts_before.len(), 1);
    assert_eq!(attempts_before[0].state, GacosSubmissionState::Submitting);
    let attempts_after = attempts_after.expect("second service could reread live attempt");
    assert_eq!(attempts_after.len(), 1);
    assert_eq!(attempts_after[0].state, GacosSubmissionState::Submitting);
    assert_eq!(
        first_result["records"][0]["state"],
        "manual_verification_required"
    );
    assert_eq!(
        second.gacos_request_status(&request.task_id).unwrap()["records"][0]["state"],
        "manual_verification_required"
    );
    second.shutdown().unwrap();
    first.shutdown().unwrap();
}

fn import_fixture_pair(fixture: &Fixture, date: &str) -> Vec<PathBuf> {
    let data = fixture.root.join(format!("{date}.ztd"));
    let header = fixture.root.join(format!("{date}.ztd.rsc"));
    std::fs::write(&data, vec![0u8; 64]).unwrap();
    std::fs::write(
        &header,
        b"WIDTH 4
FILE_LENGTH 4
X_FIRST 99
Y_FIRST 32
X_STEP 1
Y_STEP -1
",
    )
    .unwrap();
    vec![data, header]
}
fn await_import_terminal(service: &ApplicationService, task_id: &str) -> insar_store::StoredTask {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let task = service
            .store_handle()
            .unwrap()
            .lock()
            .unwrap()
            .load_task_record(task_id)
            .unwrap()
            .unwrap();
        if matches!(
            task.runtime.status,
            insar_core::TaskLifecycleStatus::Completed | insar_core::TaskLifecycleStatus::Failed
        ) {
            return task;
        }
        assert!(
            Instant::now() < deadline,
            "import did not reach terminal status"
        );
        thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn native_import_completes_in_store_and_survives_archive_reopen_without_completing_request() {
    let fixture = Fixture::new();
    let service = fixture.service();
    let request = fixture.prepare(&service, 1);
    let parent = service.create_gacos_request_task(&request).unwrap();
    let import = crate::GacosCreateImportTaskRequest {
        task_id: "gacos-import-test".to_owned(),
        source_request_task_id: request.task_id.clone(),
        source_request_item_id: parent.snapshot.candidates[0].item_id.clone(),
        source_paths: import_fixture_pair(&fixture, "20240101"),
    };
    service.gacos_create_import_task(&import).unwrap();
    let terminal = await_import_terminal(&service, &import.task_id);
    assert_eq!(
        terminal.runtime.status,
        insar_core::TaskLifecycleStatus::Completed,
        "{:?}",
        service
            .store_handle()
            .unwrap()
            .lock()
            .unwrap()
            .load_download_snapshot(&import.task_id)
            .unwrap()
    );
    assert_eq!(
        terminal.snapshot.output_root,
        fixture.root.join("output/gacos-results")
    );
    let replay = service.gacos_create_import_task(&import).unwrap();
    assert_eq!(replay["status"], "completed");
    let store = service.store_handle().unwrap();
    let download = store
        .lock()
        .unwrap()
        .load_download_snapshot(&import.task_id)
        .unwrap();
    assert_eq!(download.results.len(), 1);
    assert_eq!(
        download.results[0].bytes_written,
        import
            .source_paths
            .iter()
            .map(|path| std::fs::metadata(path).unwrap().len())
            .sum::<u64>()
    );
    let final_path = download.results[0].destination.clone();
    assert_eq!(
        std::fs::read(final_path.join("20240101.ztd")).unwrap(),
        vec![0u8; 64]
    );
    assert_eq!(
        store
            .lock()
            .unwrap()
            .load_task(&request.task_id)
            .unwrap()
            .unwrap()
            .runtime
            .status,
        insar_core::TaskLifecycleStatus::Paused
    );
    store
        .lock()
        .unwrap()
        .archive_task(&StoreArchiveTaskRequest {
            task_id: import.task_id.clone(),
            expected_revision: terminal.runtime.revision,
            archived_at: "2026-09-06T10:00:00Z".to_owned(),
        })
        .unwrap();
    drop(store);
    service.shutdown().unwrap();
    let reopened = fixture.service();
    let reopened_store = reopened.store_handle().unwrap();
    assert!(reopened_store
        .lock()
        .unwrap()
        .load_scheduler_tasks()
        .unwrap()
        .iter()
        .all(|task| task.snapshot.task_id != import.task_id));
    assert!(reopened_store
        .lock()
        .unwrap()
        .list_history()
        .unwrap()
        .iter()
        .any(|entry| entry.task.snapshot == terminal.snapshot));
    let historical = reopened_store
        .lock()
        .unwrap()
        .load_task_record(&import.task_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        historical.runtime.status,
        insar_core::TaskLifecycleStatus::Completed
    );
    assert_eq!(historical.snapshot, terminal.snapshot);
    assert!(!fixture.root.join("gacos-ledger.json").exists());
    reopened.shutdown().unwrap();
}
#[test]
fn native_import_accepts_binary_parent_without_copying_request_only_fields() {
    let fixture = Fixture::new();
    let service = fixture.service();
    let request = fixture.prepare_with_format(&service, 1, Some("binary"));
    let parent = service.create_gacos_request_task(&request).unwrap();
    let import = crate::GacosCreateImportTaskRequest {
        task_id: "gacos-binary-import".to_owned(),
        source_request_task_id: request.task_id.clone(),
        source_request_item_id: parent.snapshot.candidates[0].item_id.clone(),
        source_paths: import_fixture_pair(&fixture, "20240101"),
    };
    let created = service.gacos_create_import_task(&import).unwrap();
    assert_eq!(created["source"], "native-gacos-local-import");
    let child = service
        .store_handle()
        .unwrap()
        .lock()
        .unwrap()
        .load_task_record(&import.task_id)
        .unwrap()
        .unwrap();
    let manifest = child.snapshot.candidates[0]
        .payload
        .get("gacos_import_manifest")
        .unwrap();
    assert!(manifest.get("output_format").is_none());
    assert_eq!(
        manifest["source_request_task_id"].as_str(),
        Some(import.source_request_task_id.as_str())
    );
    assert_eq!(
        manifest["source_request_item_id"].as_str(),
        Some(import.source_request_item_id.as_str())
    );
    service.shutdown().unwrap();
}

#[test]
fn native_import_wrong_date_fails_without_publishing_or_mutating_request() {
    let fixture = Fixture::new();
    let service = fixture.service();
    let request = fixture.prepare(&service, 1);
    let parent = service.create_gacos_request_task(&request).unwrap();
    let mut import = crate::GacosCreateImportTaskRequest {
        task_id: "gacos-invalid-import".to_owned(),
        source_request_task_id: request.task_id.clone(),
        source_request_item_id: "not-selected".to_owned(),
        source_paths: import_fixture_pair(&fixture, "20240102"),
    };
    assert_eq!(
        service
            .gacos_create_import_task(&import)
            .unwrap_err()
            .code(),
        "GACOS_REQUEST_INVALID"
    );
    import.source_request_item_id = parent.snapshot.candidates[0].item_id.clone();
    service.gacos_create_import_task(&import).unwrap();
    let terminal = await_import_terminal(&service, &import.task_id);
    assert_eq!(
        terminal.runtime.status,
        insar_core::TaskLifecycleStatus::Failed
    );
    let store = service.store_handle().unwrap();
    let download = store
        .lock()
        .unwrap()
        .load_download_snapshot(&import.task_id)
        .unwrap();
    assert!(!download.results[0].destination.exists());
    assert!(download.results[0]
        .failure
        .as_ref()
        .unwrap()
        .code
        .starts_with("NATIVE_GACOS_"));
    assert_eq!(
        store
            .lock()
            .unwrap()
            .load_task(&request.task_id)
            .unwrap()
            .unwrap()
            .runtime,
        parent.runtime
    );
    service.shutdown().unwrap();
}

struct ImportInspectionGate {
    entered: Arc<AtomicBool>,
    release: Arc<AtomicBool>,
}
impl crate::DemOperationPort for ImportInspectionGate {
    fn call(&self, operation: &str, _: Value) -> Result<Value, String> {
        assert_eq!(operation, "dem_inspect");
        self.entered.store(true, Ordering::SeqCst);
        let deadline = Instant::now() + Duration::from_secs(5);
        while !self.release.load(Ordering::SeqCst) {
            if Instant::now() > deadline {
                return Err("fixture gate timed out".to_owned());
            }
            thread::sleep(Duration::from_millis(1));
        }
        Ok(
            json!({"ok":true,"metadata":{"driver":"GTiff","width":2,"height":2,"count":1,"dtype":["float32"],"crs":"EPSG:4326","bounds":[99.,29.,102.,32.],"geotransform":[99.,1.5,0.,32.,0.,-1.5]}}),
        )
    }
}
fn import_command(service: &ApplicationService, task_id: &str, kind: TaskStateCommandKind) {
    let revision = service
        .store_handle()
        .unwrap()
        .lock()
        .unwrap()
        .load_task_record(task_id)
        .unwrap()
        .unwrap()
        .runtime
        .revision;
    service
        .update_download_task(&StoreUpdateTaskRequest {
            task_id: task_id.to_owned(),
            expected_revision: revision,
            event_id: format!("import-test-{kind:?}-{revision}"),
            occurred_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            command: StoreTaskCommand {
                kind,
                item_ids: vec![],
            },
        })
        .unwrap();
}
#[test]
fn native_import_pause_resume_and_cancel_keep_atomic_publication_under_store_control() {
    for cancel in [false, true] {
        let fixture = Fixture::new();
        let entered = Arc::new(AtomicBool::new(false));
        let release = Arc::new(AtomicBool::new(false));
        let service = fixture
            .service()
            .attach_dem_operation_port(Arc::new(ImportInspectionGate {
                entered: Arc::clone(&entered),
                release: Arc::clone(&release),
            }));
        let request = fixture.prepare(&service, 1);
        let parent = service.create_gacos_request_task(&request).unwrap();
        // Explicit synthetic bytes and fixture inspection facts exercise lifecycle only.
        let tiff = fixture.root.join("20240101.tif");
        std::fs::write(&tiff, [73u8, 73, 42, 0, 8, 0, 0, 0]).unwrap();
        let import = crate::GacosCreateImportTaskRequest {
            task_id: "gacos-gated-import".to_owned(),
            source_request_task_id: request.task_id.clone(),
            source_request_item_id: parent.snapshot.candidates[0].item_id.clone(),
            source_paths: vec![tiff],
        };
        service.gacos_create_import_task(&import).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !entered.load(Ordering::SeqCst) {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        import_command(
            &service,
            &import.task_id,
            if cancel {
                TaskStateCommandKind::CancelTask
            } else {
                TaskStateCommandKind::PauseTask
            },
        );
        // Allow two coordinator polls before releasing the deliberately blocked port.
        thread::sleep(Duration::from_millis(250));
        release.store(true, Ordering::SeqCst);
        thread::sleep(Duration::from_millis(250));
        let destination = fixture
            .root
            .join("output/gacos-results")
            .join(format!("{:x}", Sha256::digest(import.task_id.as_bytes())));
        assert!(
            !destination.exists(),
            "paused/cancelled import published its final directory"
        );
        if cancel {
            service.shutdown().unwrap();
            let current = service.store_handle();
            assert!(current.is_err());
            let reopened = fixture.service();
            assert_eq!(
                reopened
                    .store_handle()
                    .unwrap()
                    .lock()
                    .unwrap()
                    .load_task_record(&import.task_id)
                    .unwrap()
                    .unwrap()
                    .runtime
                    .status,
                insar_core::TaskLifecycleStatus::Cancelled
            );
            reopened.shutdown().unwrap();
        } else {
            import_command(&service, &import.task_id, TaskStateCommandKind::ResumeTask);
            assert_eq!(
                await_import_terminal(&service, &import.task_id)
                    .runtime
                    .status,
                insar_core::TaskLifecycleStatus::Completed
            );
            assert!(destination.exists());
            service.shutdown().unwrap();
        }
    }
}

#[test]
fn native_import_failed_inspection_reopens_and_retries_existing_staged_bytes() {
    let fixture = Fixture::new();
    let service = fixture.service();
    let request = fixture.prepare(&service, 1);
    let parent = service.create_gacos_request_task(&request).unwrap();
    // Synthetic TIFF signature; the injected port below supplies fixture facts only.
    let tiff = fixture.root.join("20240101.tif");
    std::fs::write(&tiff, [73u8, 73, 42, 0, 8, 0, 0, 0]).unwrap();
    let import = crate::GacosCreateImportTaskRequest {
        task_id: "gacos-retry-import".to_owned(),
        source_request_task_id: request.task_id.clone(),
        source_request_item_id: parent.snapshot.candidates[0].item_id.clone(),
        source_paths: vec![tiff],
    };
    service.gacos_create_import_task(&import).unwrap();
    let failed = await_import_terminal(&service, &import.task_id);
    assert_eq!(
        failed.runtime.status,
        insar_core::TaskLifecycleStatus::Failed
    );
    let download = service
        .store_handle()
        .unwrap()
        .lock()
        .unwrap()
        .load_download_snapshot(&import.task_id)
        .unwrap();
    let failure = download.results[0].failure.as_ref().unwrap();
    assert_eq!(failure.code, "NATIVE_GACOS_RASTER_INSPECTION_UNAVAILABLE");
    assert!(failure.retryable);
    let key = format!("{:x}", Sha256::digest(import.task_id.as_bytes()));
    let output = fixture.root.join("output/gacos-results");
    let staging = output.join(format!(".gacos-{key}.partial"));
    let staged_modified = std::fs::metadata(staging.join("20240101.tif"))
        .unwrap()
        .modified()
        .unwrap();
    assert!(!output.join(&key).exists());
    service.shutdown().unwrap();
    let entered = Arc::new(AtomicBool::new(false));
    let reopened = fixture
        .service()
        .attach_dem_operation_port(Arc::new(ImportInspectionGate {
            entered: Arc::clone(&entered),
            release: Arc::new(AtomicBool::new(true)),
        }));
    let restored = reopened
        .store_handle()
        .unwrap()
        .lock()
        .unwrap()
        .load_task_record(&import.task_id)
        .unwrap()
        .unwrap();
    assert_eq!(restored.snapshot, failed.snapshot);
    assert_eq!(
        restored.runtime.status,
        insar_core::TaskLifecycleStatus::Failed
    );
    reopened
        .update_download_task(&StoreUpdateTaskRequest {
            task_id: import.task_id.clone(),
            expected_revision: restored.runtime.revision,
            event_id: "gacos-import-retry-after-reopen".to_owned(),
            occurred_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::RetryItems,
                item_ids: restored.snapshot.initial_selected_item_ids.clone(),
            },
        })
        .unwrap();
    let completed = await_import_terminal(&reopened, &import.task_id);
    assert_eq!(
        completed.runtime.status,
        insar_core::TaskLifecycleStatus::Completed
    );
    assert!(completed.runtime.revision > restored.runtime.revision);
    assert!(entered.load(Ordering::SeqCst));
    assert!(!staging.exists());
    assert_eq!(
        std::fs::metadata(output.join(&key).join("20240101.tif"))
            .unwrap()
            .modified()
            .unwrap(),
        staged_modified
    );
    assert_eq!(
        reopened
            .store_handle()
            .unwrap()
            .lock()
            .unwrap()
            .load_task_record(&request.task_id)
            .unwrap()
            .unwrap()
            .runtime,
        parent.runtime
    );
    reopened.shutdown().unwrap();
}

#[test]
fn task_delete_history_cancels_idle_submitted_gacos_task_and_blocks_inflight() {
    let fixture = Fixture::new();
    let service = Arc::new(fixture.service());

    // 1. Submitted idle GACOS task remains Paused per GACOS-REG-001, yet user deleting it from History succeeds
    let idle_request = fixture.prepare(&service, 1);
    let calls = Arc::new(AtomicUsize::new(0));
    service
        .submit_gacos_with_executor(
            &idle_request,
            "test@example.invalid",
            SubmittedExecutor {
                calls: Arc::clone(&calls),
            },
            Duration::ZERO,
        )
        .unwrap();
    let before = service
        .store_handle()
        .unwrap()
        .lock()
        .unwrap()
        .load_task(&idle_request.task_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        before.runtime.status,
        insar_core::TaskLifecycleStatus::Paused
    );

    service
        .task_delete_history(idle_request.task_id.clone())
        .expect("idle submitted GACOS request task should be deletable from UI history");

    let store_handle = service.store_handle().unwrap();
    let guard = store_handle.lock().unwrap();
    assert!(guard.is_task_tombstoned(&idle_request.task_id).unwrap());
    assert!(guard
        .load_task_record(&idle_request.task_id)
        .unwrap()
        .is_none());
    drop(guard);

    // 2. In-flight submitting GACOS attempt must refuse history deletion with GACOS_SUBMISSION_IN_FLIGHT without cancelling
    let inflight_fixture = Fixture::new();
    let inflight_service = Arc::new(inflight_fixture.service());
    let inflight_req = inflight_fixture.prepare(&inflight_service, 1);
    let entered = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let worker_service = Arc::clone(&inflight_service);
    let worker_req = inflight_req.clone();
    let worker_entered = Arc::clone(&entered);
    let worker_release = Arc::clone(&release);
    let worker_calls = Arc::new(AtomicUsize::new(0));
    let handle = thread::spawn(move || {
        worker_service.submit_gacos_with_executor(
            &worker_req,
            "test@example.invalid",
            GateExecutor {
                calls: worker_calls,
                entered: worker_entered,
                release: worker_release,
                uncertain_on_release: false,
            },
            Duration::ZERO,
        )
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    while !entered.load(Ordering::SeqCst) {
        assert!(
            Instant::now() < deadline,
            "submission did not enter executor"
        );
        thread::sleep(Duration::from_millis(1));
    }
    let inflight_err = inflight_service
        .task_delete_history(inflight_req.task_id.clone())
        .expect_err("in-flight GACOS submission must block history deletion");
    assert_eq!(inflight_err.code, "GACOS_SUBMISSION_IN_FLIGHT");
    let still_paused = inflight_service
        .store_handle()
        .unwrap()
        .lock()
        .unwrap()
        .load_task(&inflight_req.task_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        still_paused.runtime.status,
        insar_core::TaskLifecycleStatus::Paused
    );
    release.store(true, Ordering::SeqCst);
    let _ = handle.join().unwrap();

    service.shutdown().unwrap();
    inflight_service.shutdown().unwrap();
}

#[path = "gacos_result_service_tests.rs"]
mod result_tests;
