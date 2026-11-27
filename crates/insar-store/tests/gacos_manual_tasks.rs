use insar_core::{
    DownloadLayout, FrozenSearchBatch, SearchWorkbenchState, SnapshotItem, StoreCreateTaskRequest,
    StoreSearchBatchRequest, StoreTaskCommand, StoreUpdateTaskRequest, TaskLifecycleStatus,
    TaskOperationDescriptor, TaskStateCommandKind,
};
use insar_store::{
    GacosSubmissionAttemptIdentity, GacosSubmissionOutcome, GacosSubmissionOutcomeRequest,
    GacosSubmissionReserveRequest, GacosSubmissionState, GacosSubmissionSubmittingRequest,
    StoreError, TaskStore,
};
use serde_json::{json, Map};
use std::path::PathBuf;

struct Database(PathBuf);
impl Database {
    fn new() -> Self {
        static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "insar-gacos-manual-{}-{nonce}-{seq}.sqlite3",
            std::process::id()
        )))
    }
}
impl Drop for Database {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{}", self.0.display(), suffix));
        }
    }
}

fn fixture(
    database: &Database,
) -> (
    StoreSearchBatchRequest,
    StoreCreateTaskRequest,
    TaskOperationDescriptor,
) {
    let geometry = json!({"type":"Polygon","coordinates":[[[100.0,30.0],[100.1,30.0],[100.1,30.1],[100.0,30.1],[100.0,30.0]]]}).as_object().unwrap().clone();
    let operation = TaskOperationDescriptor {
        operation_id: "gacos-task:request".to_owned(),
        kind: "gacos".to_owned(),
        phase: "request".to_owned(),
        region_id: "gacos-region".to_owned(),
        approved_worker_operation: "gacos_manual_request".to_owned(),
        payload: json!({"manual_action_required":true,"automatic_retry_allowed":false})
            .as_object()
            .unwrap()
            .clone(),
    };
    let item = SnapshotItem {
        item_id: "request-1".to_owned(),
        provider_id: "gacos.webform".to_owned(),
        display_name: "GACOS request 1".to_owned(),
        acquisition_datetime: None,
        footprint_geojson: Some(geometry.clone()),
        metadata: Map::new(),
        payload: json!({"dates":["2024-01-02"],"hour":7,"minute":8})
            .as_object()
            .unwrap()
            .clone(),
        asset_access: None,
        execution_policy: None,
    };
    let batch = StoreSearchBatchRequest {
        batch: FrozenSearchBatch {
            schema_version: 1,
            batch_id: "gacos-batch".to_owned(),
            provider_id: "gacos.webform".to_owned(),
            created_at: "2026-09-06T00:00:00Z".to_owned(),
            aoi_name: "Frozen AOI".to_owned(),
            aoi_geojson: Some(geometry),
            query: Map::new(),
            search_stats: None,
            candidates: vec![item],
        },
        workbench: SearchWorkbenchState {
            batch_id: "gacos-batch".to_owned(),
            checked_item_ids: vec!["request-1".to_owned()],
            page_index: 0,
            page_size: 1,
            revision: 0,
        },
        activate_workbench: false,
    };
    let task = StoreCreateTaskRequest {
        task_id: "gacos-task".to_owned(),
        batch_id: "gacos-batch".to_owned(),
        selected_item_ids: vec!["request-1".to_owned()],
        checked_item_ids: vec!["request-1".to_owned()],
        page_index: 0,
        page_size: 1,
        output_root: database.0.parent().unwrap().join("gacos-output"),
        layout: DownloadLayout::Direct,
        concurrency: 1,
        created_at: "2026-09-06T00:00:00Z".to_owned(),
    };
    (batch, task, operation)
}

fn attempt_identity(attempt_id: &str, attempt_number: u64) -> GacosSubmissionAttemptIdentity {
    GacosSubmissionAttemptIdentity {
        task_id: "gacos-task".to_owned(),
        item_id: "request-1".to_owned(),
        attempt_id: attempt_id.to_owned(),
        fingerprint_sha256: "a".repeat(64),
        account_identity_hash_sha256: "b".repeat(64),
        attempt_number,
    }
}

fn reserve_request(attempt_id: &str, attempt_number: u64) -> GacosSubmissionReserveRequest {
    GacosSubmissionReserveRequest {
        identity: attempt_identity(attempt_id, attempt_number),
        occurred_at: "2026-09-06T00:00:01Z".to_owned(),
        force_reapply: false,
    }
}

#[test]
fn gacos_submission_reopen_recovery_prevents_reclaim_and_resubmit() {
    let db = Database::new();
    let (batch, task, operation) = fixture(&db);
    let request = reserve_request("gacos-attempt-1", 0);
    {
        let mut store = TaskStore::open(&db.0).unwrap();
        store
            .create_paused_gacos_request_task(&batch, &task, &operation)
            .unwrap();
        let reserved = store.reserve_gacos_submission_attempt(&request).unwrap();
        assert_eq!(reserved.state, GacosSubmissionState::Reserved);
        let claimed = store
            .mark_gacos_submission_submitting(&GacosSubmissionSubmittingRequest {
                identity: request.identity.clone(),
                occurred_at: "2026-09-06T00:00:02Z".to_owned(),
            })
            .unwrap();
        assert_eq!(claimed.state, GacosSubmissionState::Submitting);
        let second_claim =
            store.mark_gacos_submission_submitting(&GacosSubmissionSubmittingRequest {
                identity: request.identity.clone(),
                occurred_at: "2026-09-06T00:00:03Z".to_owned(),
            });
        assert!(matches!(second_claim, Err(StoreError::AlreadyExists)));
    }

    let mut store = TaskStore::open_existing(&db.0).unwrap();
    assert_eq!(
        store
            .recover_gacos_submitting_attempts(Some(&task.task_id), "2026-09-06T00:00:04Z",)
            .unwrap(),
        1
    );
    assert_eq!(
        store
            .recover_gacos_submitting_attempts(Some(&task.task_id), "2026-09-06T00:00:05Z",)
            .unwrap(),
        0
    );
    let recovered = store
        .read_gacos_submission_attempts(&task.task_id, Some("request-1"), 8)
        .unwrap();
    assert_eq!(recovered.len(), 1);
    assert_eq!(
        recovered[0].state,
        GacosSubmissionState::SubmissionUncertain
    );
    assert!(!recovered[0].automatic_retry_allowed);
    assert_eq!(
        recovered[0].reason_code.as_deref(),
        Some("RECOVERED_AFTER_RESTART")
    );

    let replay = store
        .reserve_gacos_submission_attempt(&GacosSubmissionReserveRequest {
            occurred_at: "2026-09-06T00:00:06Z".to_owned(),
            ..request.clone()
        })
        .unwrap();
    assert_eq!(replay, recovered[0]);
    let reclaim = store.mark_gacos_submission_submitting(&GacosSubmissionSubmittingRequest {
        identity: request.identity,
        occurred_at: "2026-09-06T00:00:07Z".to_owned(),
    });
    assert!(matches!(reclaim, Err(StoreError::AlreadyExists)));
    assert_eq!(
        store
            .read_gacos_submission_attempts(&task.task_id, Some("request-1"), 8)
            .unwrap()
            .len(),
        1,
        "recovery must not create a second POST attempt"
    );
}

#[test]
fn gacos_submission_outcome_is_recorded_after_cancel_without_completing_task() {
    let db = Database::new();
    let (batch, task, operation) = fixture(&db);
    let request = reserve_request("gacos-attempt-cancelled", 0);
    let outcome = GacosSubmissionOutcomeRequest {
        identity: request.identity.clone(),
        occurred_at: "2026-09-06T00:00:04Z".to_owned(),
        outcome: GacosSubmissionOutcome::ManualVerificationRequired {
            reason_code: "RESULT_LINK_REQUIRED".to_owned(),
            http_status: 202,
        },
    };
    let stored = {
        let mut store = TaskStore::open(&db.0).unwrap();
        store
            .create_paused_gacos_request_task(&batch, &task, &operation)
            .unwrap();
        store.reserve_gacos_submission_attempt(&request).unwrap();
        store
            .mark_gacos_submission_submitting(&GacosSubmissionSubmittingRequest {
                identity: request.identity.clone(),
                occurred_at: "2026-09-06T00:00:02Z".to_owned(),
            })
            .unwrap();
        let cancelled = store
            .update_task(&StoreUpdateTaskRequest {
                task_id: task.task_id.clone(),
                expected_revision: 0,
                command: StoreTaskCommand {
                    kind: TaskStateCommandKind::CancelTask,
                    item_ids: vec![],
                },
                event_id: "cancel-before-result".to_owned(),
                occurred_at: "2026-09-06T00:00:03Z".to_owned(),
            })
            .unwrap();
        assert_eq!(cancelled.runtime.status, TaskLifecycleStatus::Cancelled);
        let stored = store.record_gacos_submission_outcome(&outcome).unwrap();
        assert_eq!(
            stored.state,
            GacosSubmissionState::ManualVerificationRequired
        );
        assert!(!stored.automatic_retry_allowed);
        let task_after = store.load_task(&task.task_id).unwrap().unwrap();
        assert_eq!(task_after.runtime.status, TaskLifecycleStatus::Cancelled);
        assert!(task_after.runtime.completed_item_ids.is_empty());
        stored
    };

    let mut store = TaskStore::open_existing(&db.0).unwrap();
    let replay = store
        .record_gacos_submission_outcome(&GacosSubmissionOutcomeRequest {
            occurred_at: "2026-09-06T00:00:05Z".to_owned(),
            ..outcome
        })
        .unwrap();
    assert_eq!(replay, stored);
    assert_eq!(
        store
            .read_gacos_submission_attempts(&task.task_id, Some("request-1"), 8)
            .unwrap(),
        vec![stored]
    );
    let task_after_reopen = store.load_task(&task.task_id).unwrap().unwrap();
    assert_eq!(
        task_after_reopen.runtime.status,
        TaskLifecycleStatus::Cancelled
    );
    assert!(task_after_reopen.runtime.completed_item_ids.is_empty());
}

#[test]
fn gacos_create_is_atomic_paused_and_survives_reopen_without_queueing() {
    let db = Database::new();
    let (batch, task, operation) = fixture(&db);
    {
        let mut store = TaskStore::open(&db.0).unwrap();
        let created = store
            .create_paused_gacos_request_task(&batch, &task, &operation)
            .unwrap();
        assert_eq!(created.runtime.status, TaskLifecycleStatus::Paused);
        assert!(created.runtime.task_paused);
        assert_eq!(created.runtime.paused_item_ids, task.selected_item_ids);
        assert!(created.runtime.active_item_ids.is_empty());
        assert!(created.runtime.queued_item_ids.is_empty());
        assert_eq!(created.runtime.revision, 0);
        assert_eq!(store.list_task_events(&task.task_id).unwrap().len(), 1);
        let replay = store
            .create_paused_gacos_request_task(&batch, &task, &operation)
            .unwrap();
        assert_eq!(replay, created);
    }
    let store = TaskStore::open_existing(&db.0).unwrap();
    let task = store.load_task("gacos-task").unwrap().unwrap();
    assert_eq!(task.runtime.status, TaskLifecycleStatus::Paused);
    assert_eq!(store.core_restore(false).unwrap().tasks.len(), 1);
}

#[test]
fn gacos_generic_resume_retry_activation_and_completion_are_rejected_without_events() {
    let db = Database::new();
    let (batch, task, operation) = fixture(&db);
    let mut store = TaskStore::open(&db.0).unwrap();
    store
        .create_paused_gacos_request_task(&batch, &task, &operation)
        .unwrap();
    for (index, kind) in [
        TaskStateCommandKind::ResumeTask,
        TaskStateCommandKind::RetryTask,
        TaskStateCommandKind::ResumeItems,
        TaskStateCommandKind::RetryItems,
        TaskStateCommandKind::ActivateItems,
        TaskStateCommandKind::MarkCompleted,
        TaskStateCommandKind::MarkSkipped,
    ]
    .into_iter()
    .enumerate()
    {
        let item_ids = if matches!(
            kind,
            TaskStateCommandKind::ResumeTask | TaskStateCommandKind::RetryTask
        ) {
            vec![]
        } else {
            vec!["request-1".to_owned()]
        };
        let error = store
            .update_task(&StoreUpdateTaskRequest {
                task_id: task.task_id.clone(),
                expected_revision: 0,
                command: StoreTaskCommand { kind, item_ids },
                event_id: format!("forbidden-{index}"),
                occurred_at: "2026-09-06T00:01:00Z".to_owned(),
            })
            .unwrap_err();
        assert_eq!(error.code(), "GACOS_MANUAL_ACTION_REQUIRED");
    }
    assert_eq!(store.list_task_events(&task.task_id).unwrap().len(), 1);
    assert_eq!(
        store
            .load_task(&task.task_id)
            .unwrap()
            .unwrap()
            .runtime
            .revision,
        0
    );
    let cancelled = store
        .update_task(&StoreUpdateTaskRequest {
            task_id: task.task_id.clone(),
            expected_revision: 0,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::CancelTask,
                item_ids: vec![],
            },
            event_id: "cancel-manual".to_owned(),
            occurred_at: "2026-09-06T00:02:00Z".to_owned(),
        })
        .unwrap();
    assert_eq!(cancelled.runtime.status, TaskLifecycleStatus::Cancelled);
}

#[test]
fn gacos_cannot_escape_into_generic_schedulable_creation() {
    let db = Database::new();
    let (batch, task, operation) = fixture(&db);
    let mut store = TaskStore::open(&db.0).unwrap();
    let error = store
        .save_search_batch_and_create_task_with_projection(&batch, &task, Some(&operation), None)
        .unwrap_err();
    assert!(matches!(error, StoreError::GacosManualActionRequired));
    assert!(store.load_task(&task.task_id).unwrap().is_none());
    assert!(store.core_restore(false).unwrap().tasks.is_empty());
}

#[test]
fn gacos_invalid_membership_or_provider_never_leaves_a_task() {
    for bad_provider in [false, true] {
        let db = Database::new();
        let (mut batch, mut task, operation) = fixture(&db);
        if bad_provider {
            batch.batch.candidates[0].provider_id = "other.provider".to_owned();
        } else {
            task.selected_item_ids = vec!["unrepresented-request".to_owned()];
        }
        let mut store = TaskStore::open(&db.0).unwrap();
        assert!(store
            .create_paused_gacos_request_task(&batch, &task, &operation)
            .is_err());
        assert!(store.load_task(&task.task_id).unwrap().is_none());
    }
}
