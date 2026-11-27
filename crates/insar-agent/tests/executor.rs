use insar_agent::{
    executor::{
        check_file, classify, recovery, verify_task_file, ErrorClass, ExecutionError, Executor,
        Recovery, ToolPort, ToolResult,
    },
    plan::{approve, build_plan},
    run::{read_sealed, RunWriter},
};
use insar_core::{
    DownloadItemResult, DownloadLayout, DownloadOutcome, FrozenSearchBatch, SearchWorkbenchState,
    SnapshotItem, StoreCreateTaskRequest, StoreSearchBatchRequest, StoreTaskCommand,
    StoreUpdateTaskRequest, TaskStateCommandKind,
};
use insar_store::{AgentApprovalRow, AgentPlanRow, AgentRunRow, TaskStore};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

fn setup(label: &str) -> (PathBuf, TaskStore, RunWriter) {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "agent-executor-{}-{label}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    let mut store = TaskStore::open(root.join("store.db")).unwrap();
    let row = AgentRunRow {
        run_id: "run-execution".into(),
        session_id: "session-test".into(),
        origin: "manual".into(),
        client_label: None,
        started_at: "2026-10-23T09:00:00Z".into(),
        ended_at: None,
        status: "running".into(),
        model_route: None,
        catalog_version: 1,
        catalog_sha256: "a".repeat(64),
        app_version: "test".into(),
        source_commit: "fixture".into(),
        store_revision_start: 0,
        store_revision_end: None,
        record_dir: "agent/runs/run-execution".into(),
        trace_sha256: None,
        reproduces_run_id: None,
        reasoning_off_refused: false,
        truncated: false,
        usage_prompt: None,
        usage_completion: None,
    };
    let writer = RunWriter::start(&root, &mut store, &row).unwrap();
    (root, store, writer)
}
fn plan(store: &mut TaskStore, input: &Value) -> String {
    let intent = json!({"steps":[{"tool_id":"update_download_task","input":input,"event_id":input["event_id"]} ],"card_snapshot":{"source":"fixture","range":"one item","estimated_bytes":4,"output":"relative"}});
    let row = build_plan(
        AgentPlanRow {
            plan_id: "plan-execution".into(),
            session_id: "session-test".into(),
            run_id: "run-execution".into(),
            created_at: "2026-10-23T09:00:00Z".into(),
            model_route: "manual".into(),
            plan_json: String::new(),
            plan_hash: String::new(),
            max_tier: 2,
            status: "draft".into(),
            expires_at: "2026-10-23T09:30:00Z".into(),
            store_revision_seen: 0,
        },
        &intent,
    )
    .unwrap();
    store.save_agent_plan(&row).unwrap();
    let receipt = AgentApprovalRow {
        approval_id: "approval-execution".into(),
        plan_id: row.plan_id.clone(),
        plan_hash_seen: row.plan_hash.clone(),
        snapshot_json:
            json!({"source":"fixture","range":"one item","estimated_bytes":4,"output":"relative"})
                .to_string(),
        decision: "approved".into(),
        decided_at: "2026-10-23T09:01:00Z".into(),
        decided_via: "desktop_card".into(),
        mode: None,
        rewritten_from_plan_hash: None,
    };
    approve(store, &row.plan_id, &receipt, 0, "2026-10-23T09:01:00Z").unwrap();
    row.plan_hash
}
struct OfflinePort {
    submits: usize,
    reads: usize,
    store_path: PathBuf,
    unknown: bool,
    found: bool,
}
impl ToolPort for OfflinePort {
    fn read(&mut self, command: &str, _: &Value) -> Result<ToolResult, ErrorClass> {
        self.reads += 1;
        assert!(matches!(
            command,
            "application_service_readiness" | "credential_status"
        ));
        Ok(ToolResult {
            summary: if command == "credential_status" {
                json!({"cdse":"configured","earthdata":"login:private-name","token":"not-a-real-token"})
            } else {
                json!({"ready":true})
            },
            observed_at: "2026-10-23T09:00:00Z".into(),
            source: "local".into(),
        })
    }
    fn submit(&mut self, command: &str, input: &Value) -> Result<ToolResult, ErrorClass> {
        self.submits += 1;
        assert_eq!(command, "application_update_download_task");
        let mut store = TaskStore::open_existing(&self.store_path).unwrap();
        let request: StoreUpdateTaskRequest = serde_json::from_value(input.clone()).unwrap();
        let updated = store
            .update_task(&request)
            .map_err(|_| ErrorClass::InvalidParameters)?;
        if self.unknown {
            return Err(ErrorClass::ResultUnknown);
        }
        Ok(ToolResult {
            summary: json!({"task_id":request.task_id,"status":updated.runtime.status,"revision":updated.runtime.revision}),
            observed_at: "2026-10-23T09:01:30Z".into(),
            source: "store".into(),
        })
    }
    fn task_state(&mut self, task_id: &str, event_id: &str) -> Result<Option<Value>, ErrorClass> {
        let store = TaskStore::open_existing(&self.store_path).unwrap();
        let task = store.load_task(task_id).unwrap();
        let events = store.list_task_events(task_id).unwrap();
        if !self.found || !events.iter().any(|event| event.event_id == event_id) {
            return Ok(None);
        }
        Ok(task.map(|task|json!({"task_id":task_id,"last_event_id":event_id,"status":task.runtime.status,"revision":task.runtime.revision})))
    }
}
fn seed_task(store: &mut TaskStore, root: &Path) {
    let item = SnapshotItem {
        item_id: "ITEM-A".into(),
        provider_id: "asf.sentinel1".into(),
        display_name: "A.zip".into(),
        acquisition_datetime: None,
        footprint_geojson: None,
        metadata: Default::default(),
        payload: json!({"file_name":"A.zip"}).as_object().unwrap().clone(),
        asset_access: None,
        execution_policy: None,
    };
    store.save_search_batch(&StoreSearchBatchRequest{activate_workbench:true,batch:FrozenSearchBatch{schema_version:1,batch_id:"batch-a".into(),provider_id:"asf.sentinel1".into(),created_at:"2026-10-23T09:00:00Z".into(),aoi_name:"fixture".into(),aoi_geojson:Some(json!({"type":"Polygon","coordinates":[[[100,30],[101,30],[101,31],[100,31],[100,30]]]}).as_object().unwrap().clone()),query:Default::default(),search_stats:None,candidates:vec![item]},workbench:SearchWorkbenchState{batch_id:"batch-a".into(),checked_item_ids:vec!["ITEM-A".into()],page_index:0,page_size:24,revision:0}}).unwrap();
    store
        .create_task(&StoreCreateTaskRequest {
            task_id: "task-a".into(),
            batch_id: "batch-a".into(),
            selected_item_ids: vec!["ITEM-A".into()],
            checked_item_ids: vec!["ITEM-A".into()],
            page_index: 0,
            page_size: 24,
            output_root: root.to_path_buf(),
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-10-23T09:00:01Z".into(),
        })
        .unwrap();
}

#[test]
fn verified_delivery_rejects_wrong_hash_even_when_file_exists_and_has_right_size() {
    let (root, _, _) = setup("file");
    let path = root.join("artifact.bin");
    fs::write(&path, b"test payload").unwrap();
    assert!(check_file(&path, 12, &"0".repeat(64)).is_err());
    let sha = format!("{:x}", Sha256::digest(b"test payload"));
    assert_eq!(check_file(&path, 12, &sha).unwrap().sha256, sha);
}
#[test]
fn credential_readiness_never_puts_raw_service_status_into_trace() {
    let (root, mut store, mut writer) = setup("credential");
    let port = OfflinePort {
        submits: 0,
        reads: 0,
        store_path: root.join("store.db"),
        unknown: false,
        found: false,
    };
    let mut executor = Executor::new(port);
    let projected = executor
        .call_read("credential_readiness", &json!({}), &mut writer)
        .unwrap();
    assert_eq!(projected["earthdata"], "ready");
    let text = fs::read_to_string(root.join("agent/runs/run-execution/trace.jsonl")).unwrap();
    assert!(!text.contains("private-name"));
    assert!(!text.contains("not-a-real-token"));
    writer
        .finish(
            &mut store,
            "run-execution",
            "completed",
            "2026-10-23T09:03:00Z",
            0,
        )
        .unwrap();
}

#[test]
fn agent_read_does_not_forward_unrecognized_arguments_to_the_service() {
    let (root, _store, mut writer) = setup("read-input");
    let mut executor = Executor::new(OfflinePort {
        submits: 0,
        reads: 0,
        store_path: root.join("store.db"),
        unknown: false,
        found: false,
    });
    assert!(matches!(
        executor.call_read(
            "service_readiness",
            &json!({"unreviewed":"value"}),
            &mut writer
        ),
        Err(ExecutionError::ToolDenied)
    ));
    assert_eq!(
        executor.into_port().reads,
        0,
        "reject before crossing the host boundary"
    );
}

#[test]
fn result_unknown_requires_task_readback_not_replay() {
    assert_eq!(classify("unknown"), ErrorClass::ResultUnknown);
    assert_eq!(recovery(classify("unknown")), Recovery::ReadTask);
    assert_eq!(classify("auth"), ErrorClass::Authentication);
}

#[test]
fn approved_update_unknown_result_reads_event_id_without_duplicate_submit_and_replays_same_plan_hash(
) {
    let (root, mut store, mut writer) = setup("replay");
    seed_task(&mut store, &root);
    let request = StoreUpdateTaskRequest {
        task_id: "task-a".into(),
        expected_revision: 0,
        command: StoreTaskCommand {
            kind: TaskStateCommandKind::PauseTask,
            item_ids: vec![],
        },
        event_id: "event-pause-a".into(),
        occurred_at: "2026-10-23T09:01:30Z".into(),
    };
    let input = serde_json::to_value(&request).unwrap();
    let hash = plan(&mut store, &input);
    let port = OfflinePort {
        submits: 0,
        reads: 0,
        store_path: root.join("store.db"),
        unknown: true,
        found: true,
    };
    let mut executor = Executor::new(port);
    assert!(matches!(
        executor.call_read("shell_exec", &json!({}), &mut writer),
        Err(ExecutionError::ToolDenied)
    ));
    let result = executor
        .submit_approved(
            &mut store,
            "plan-execution",
            0,
            "2026-10-23T09:02:00Z",
            "update_download_task",
            &input,
            &mut writer,
        )
        .unwrap();
    assert_eq!(result["last_event_id"], "event-pause-a");
    let port = executor.into_port();
    assert_eq!(port.submits, 1);
    assert_eq!(
        store
            .list_task_events("task-a")
            .unwrap()
            .iter()
            .filter(|event| event.event_id == "event-pause-a")
            .count(),
        1
    );
    assert_eq!(
        store.load_task("task-a").unwrap().unwrap().runtime.revision,
        1
    );
    let original = store.agent_plan("plan-execution").unwrap().unwrap();
    assert_eq!(hash, original.plan_hash);
    let recomputed=build_plan(AgentPlanRow{plan_id:"repeat".into(),..original},&json!({"steps":[{"tool_id":"update_download_task","input":input,"event_id":"event-pause-a"}],"card_snapshot":{"source":"fixture","range":"one item","estimated_bytes":4,"output":"relative"}})).unwrap();
    assert_eq!(hash, recomputed.plan_hash);
    writer
        .finish(
            &mut store,
            "run-execution",
            "completed",
            "2026-10-23T09:03:00Z",
            0,
        )
        .unwrap();
    let trace = read_sealed(&root, &store, "run-execution").unwrap();
    assert!(trace.iter().any(|e| e.kind == "execution"));
}

#[test]
fn unknown_without_confirmed_event_stops_and_does_not_resubmit() {
    let (root, mut store, mut writer) = setup("unknown");
    seed_task(&mut store, &root);
    let input = serde_json::to_value(StoreUpdateTaskRequest {
        task_id: "task-a".into(),
        expected_revision: 0,
        command: StoreTaskCommand {
            kind: TaskStateCommandKind::PauseTask,
            item_ids: vec![],
        },
        event_id: "event-pause-a".into(),
        occurred_at: "2026-10-23T09:01:30Z".into(),
    })
    .unwrap();
    plan(&mut store, &input);
    let mut executor = Executor::new(OfflinePort {
        submits: 0,
        reads: 0,
        store_path: root.join("store.db"),
        unknown: true,
        found: false,
    });
    assert!(matches!(
        executor.submit_approved(
            &mut store,
            "plan-execution",
            0,
            "2026-10-23T09:02:00Z",
            "update_download_task",
            &input,
            &mut writer
        ),
        Err(ExecutionError::ResultUnknown)
    ));
    assert_eq!(executor.into_port().submits, 1);
}

#[test]
fn stale_task_revision_cannot_be_bypassed_by_spoofed_revision_argument() {
    let (root, mut store, mut writer) = setup("stale-revision");
    seed_task(&mut store, &root);
    let request = StoreUpdateTaskRequest {
        task_id: "task-a".into(),
        expected_revision: 0,
        command: StoreTaskCommand {
            kind: TaskStateCommandKind::PauseTask,
            item_ids: vec![],
        },
        event_id: "agent-event".into(),
        occurred_at: "2026-10-23T09:01:30Z".into(),
    };
    let input = serde_json::to_value(request).unwrap();
    plan(&mut store, &input);
    let old_revision = store.load_task("task-a").unwrap().unwrap().runtime.revision;
    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "task-a".into(),
            expected_revision: old_revision,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::PauseTask,
                item_ids: vec![],
            },
            event_id: "external-event".into(),
            occurred_at: "2026-10-23T09:01:31Z".into(),
        })
        .unwrap();
    let mut executor = Executor::new(OfflinePort {
        submits: 0,
        reads: 0,
        store_path: root.join("store.db"),
        unknown: false,
        found: false,
    });
    assert!(executor
        .submit_approved(
            &mut store,
            "plan-execution",
            0,
            "2026-10-23T09:02:00Z",
            "update_download_task",
            &input,
            &mut writer
        )
        .is_err());
    assert_eq!(
        executor.into_port().submits,
        0,
        "stale approval must stop before invoking ApplicationService"
    );
}

#[test]
fn task_and_actual_bytes_and_hash_must_all_agree() {
    let (root, mut store, _) = setup("verify");
    seed_task(&mut store, &root);
    let path = root.join("artifact.bin");
    fs::write(&path, b"test payload").unwrap();
    let sha = format!("{:x}", Sha256::digest(b"test payload"));
    assert!(verify_task_file(&store, "task-a", &path, 12, &sha).is_err());
    let result = DownloadItemResult {
        task_id: "task-a".into(),
        item_id: "ITEM-A".into(),
        outcome: DownloadOutcome::Completed,
        destination: path.clone(),
        bytes_written: 12,
        attempt_count: 1,
        failure: None,
        finished_at: "2026-10-23T09:04:00Z".into(),
    };
    store.save_scheduler_download_result(&result).unwrap();
    let revision = store.load_task("task-a").unwrap().unwrap().runtime.revision;
    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "task-a".into(),
            expected_revision: revision,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::MarkCompleted,
                item_ids: vec!["ITEM-A".into()],
            },
            event_id: "complete-a".into(),
            occurred_at: "2026-10-23T09:05:00Z".into(),
        })
        .unwrap();
    assert_eq!(
        verify_task_file(&store, "task-a", &path, 12, &sha)
            .unwrap()
            .bytes,
        12
    );
    fs::write(&path, b"test paylOad").unwrap();
    assert!(verify_task_file(&store, "task-a", &path, 12, &sha).is_err());
}

#[test]
fn non_terminal_search_progress_polls_do_not_stall_executor_while_terminal_repeats_still_stall() {
    struct SearchPollPort {
        calls: usize,
    }
    impl ToolPort for SearchPollPort {
        fn read(&mut self, command: &str, _: &Value) -> Result<ToolResult, ErrorClass> {
            assert_eq!(command, "application_search_progress");
            self.calls += 1;
            let summary = if self.calls <= 4 {
                json!({"stage": "requesting", "terminal": false})
            } else {
                json!({"stage": "done", "terminal": true, "items_kept": 250})
            };
            Ok(ToolResult {
                summary,
                observed_at: "2026-11-11T09:50:00Z".into(),
                source: "local".into(),
            })
        }
        fn submit(&mut self, _: &str, _: &Value) -> Result<ToolResult, ErrorClass> {
            unreachable!()
        }
        fn task_state(&mut self, _: &str, _: &str) -> Result<Option<Value>, ErrorClass> {
            Ok(None)
        }
    }

    let (_, _, mut writer) = setup("search-poll-stall");
    let mut executor = Executor::new(SearchPollPort { calls: 0 });
    let input = json!({"request_id": "plan-live-1"});

    // Four identical non-terminal polls do NOT trigger ExecutionError::Stalled
    for _ in 0..4 {
        let out = executor
            .call_read("search_progress", &input, &mut writer)
            .expect("non-terminal search_progress poll must not stall");
        assert_eq!(out["terminal"], false);
    }

    // Terminal polls DO still stall on the 3rd identical terminal read
    assert_eq!(
        executor
            .call_read("search_progress", &input, &mut writer)
            .unwrap()["terminal"],
        true
    );
    assert_eq!(
        executor
            .call_read("search_progress", &input, &mut writer)
            .unwrap()["terminal"],
        true
    );
    assert!(matches!(
        executor.call_read("search_progress", &input, &mut writer),
        Err(ExecutionError::Stalled)
    ));
}
