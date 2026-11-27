use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use insar_core::{
    DownloadLayout, FrozenSearchBatch, SearchWorkbenchState, SidecarOperation, SidecarRequest,
    SidecarResponse, SnapshotItem, StoreArchiveTaskRequest, StoreCreateTaskRequest,
    StoreDeleteHistoryRequest, StoreSearchBatchRequest, StoreTaskCommand, StoreUpdateTaskRequest,
    TaskStateCommandKind,
};
use insar_service::Dispatcher;
use insar_store::TaskStore;
use serde_json::{json, Value};

struct TempStore(PathBuf);

impl TempStore {
    fn new() -> Self {
        static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "insar-service-store-{}-{nonce}-{seq}.sqlite3",
            std::process::id()
        )))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempStore {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{}", self.0.display(), suffix));
        }
    }
}

fn request(request_id: &str, operation: SidecarOperation, payload: Value) -> SidecarRequest {
    SidecarRequest {
        protocol_major: 1,
        protocol_minor: 6,
        request_id: request_id.to_string(),
        operation,
        payload: payload.as_object().cloned().unwrap_or_default(),
    }
}

fn batch_payload() -> Value {
    json!({
        "batch": {
            "schema_version": 1,
            "batch_id": "batch-sidecar",
            "provider_id": "asf.sentinel1",
            "created_at": "2024-01-15T00:00:00Z",
            "aoi_name": "四川省雅安市",
            "aoi_geojson": {
                "type": "Polygon",
                "coordinates": [[[102.2, 29.7], [103.2, 29.7], [103.2, 30.7], [102.2, 30.7], [102.2, 29.7]]]
            },
            "query": {"product_type": "SLC"},
            "candidates": [{
                "item_id": "S1-A",
                "provider_id": "asf.sentinel1",
                "display_name": "S1-A",
                "acquisition_datetime": null,
                "footprint_geojson": null,
                "metadata": {},
                "payload": {}
            }]
        },
        "workbench": {
            "batch_id": "batch-sidecar",
            "checked_item_ids": ["S1-A"],
            "page_index": 9,
            "page_size": 24,
            "revision": 0
        }
    })
}

fn create_payload(output_root: &Path) -> Value {
    json!({
        "task_id": "task-sidecar",
        "batch_id": "batch-sidecar",
        "selected_item_ids": ["S1-A"],
        "checked_item_ids": ["S1-A"],
        "page_index": 9,
        "page_size": 24,
        "output_root": output_root,
        "layout": "sentinel1_slc",
        "concurrency": 2,
        "created_at": "2024-01-15T01:00:00Z"
    })
}

fn pause_payload() -> Value {
    json!({
        "task_id": "task-sidecar",
        "expected_revision": 0,
        "command": {"kind": "pause_task", "item_ids": []},
        "event_id": "pause-sidecar",
        "occurred_at": "2024-01-15T01:00:01Z"
    })
}

fn complex_batch(batch_id: &str, aoi_name: &str) -> FrozenSearchBatch {
    FrozenSearchBatch {
        schema_version: 1,
        batch_id: batch_id.to_owned(),
        provider_id: "fixture.provider".to_owned(),
        created_at: format!("2024-02-01T00:00:00Z-{batch_id}"),
        aoi_name: aoi_name.to_owned(),
        aoi_geojson: serde_json::from_value(json!({
            "type": "Polygon",
            "coordinates": [[[100.0, 30.0], [101.0, 30.0], [101.0, 31.0], [100.0, 31.0], [100.0, 30.0]]]
        }))
        .unwrap(),
        query: json!({"dataset": "fixture", "aoi": aoi_name})
            .as_object()
            .unwrap()
            .clone(),
        search_stats: None,
        candidates: vec![
            SnapshotItem {
                item_id: format!("{batch_id}-selected"),
                provider_id: "fixture.provider".to_owned(),
                display_name: "selected fixture product".to_owned(),
                acquisition_datetime: Some("2024-02-01T00:00:00Z".to_owned()),
                footprint_geojson: None,
                metadata: json!({"orbit": "ascending"}).as_object().unwrap().clone(),
                payload: json!({"asset": "primary"}).as_object().unwrap().clone(),
                asset_access: None,
                execution_policy: None,
            },
            SnapshotItem {
                item_id: format!("{batch_id}-unselected"),
                provider_id: "fixture.provider".to_owned(),
                display_name: "unselected fixture product".to_owned(),
                acquisition_datetime: Some("2024-02-02T00:00:00Z".to_owned()),
                footprint_geojson: None,
                metadata: Default::default(),
                payload: Default::default(),
                asset_access: None,
                execution_policy: None,
            },
        ],
    }
}

fn seed_complex_store(path: &Path) {
    let mut store = TaskStore::open(path).expect("complex fixture Store opens");
    let first = complex_batch("batch-a", "AOI A");
    let second = complex_batch("batch-b", "AOI B");
    for (batch, activate_workbench) in [(first, true), (second, false)] {
        store
            .save_search_batch(&StoreSearchBatchRequest {
                workbench: SearchWorkbenchState {
                    batch_id: batch.batch_id.clone(),
                    checked_item_ids: vec![format!("{}-selected", batch.batch_id)],
                    page_index: 1,
                    page_size: 24,
                    revision: 0,
                },
                batch,
                activate_workbench,
            })
            .unwrap();
    }

    let output_root = path.parent().unwrap().join("complex-output");
    for (task_id, batch_id, command) in [
        (
            "task-running",
            "batch-a",
            TaskStateCommandKind::ActivateItems,
        ),
        ("task-paused", "batch-a", TaskStateCommandKind::PauseTask),
        ("task-failed", "batch-b", TaskStateCommandKind::MarkFailed),
        (
            "task-cancelled",
            "batch-b",
            TaskStateCommandKind::CancelItems,
        ),
        (
            "task-history",
            "batch-b",
            TaskStateCommandKind::MarkCompleted,
        ),
        (
            "task-tombstone",
            "batch-b",
            TaskStateCommandKind::MarkCompleted,
        ),
        ("task-late", "batch-a", TaskStateCommandKind::ActivateItems),
    ] {
        let item_id = format!("{batch_id}-selected");
        store
            .create_task(&StoreCreateTaskRequest {
                task_id: task_id.to_owned(),
                batch_id: batch_id.to_owned(),
                selected_item_ids: vec![item_id.clone()],
                checked_item_ids: vec![item_id.clone()],
                page_index: 3,
                page_size: 24,
                output_root: output_root.join(task_id),
                layout: DownloadLayout::Direct,
                concurrency: 1,
                created_at: "2024-02-01T01:00:00Z".to_owned(),
            })
            .unwrap();
        let command_item_ids = if matches!(&command, TaskStateCommandKind::PauseTask) {
            Vec::new()
        } else {
            vec![item_id]
        };
        store
            .update_task(&StoreUpdateTaskRequest {
                task_id: task_id.to_owned(),
                expected_revision: 0,
                command: StoreTaskCommand {
                    kind: command,
                    item_ids: command_item_ids,
                },
                event_id: format!("event-{task_id}"),
                occurred_at: "2024-02-01T01:00:01Z".to_owned(),
            })
            .unwrap();
    }

    let late_item = "batch-a-selected".to_owned();
    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "task-late".to_owned(),
            expected_revision: 1,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::PauseItems,
                item_ids: vec![late_item.clone()],
            },
            event_id: "event-task-late-newer".to_owned(),
            occurred_at: "2024-02-01T01:00:02Z".to_owned(),
        })
        .unwrap();
    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "task-late".to_owned(),
            expected_revision: 2,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::PauseTask,
                item_ids: Vec::new(),
            },
            event_id: "event-task-late-paused".to_owned(),
            occurred_at: "2024-02-01T01:00:03Z".to_owned(),
        })
        .unwrap();
    let stale = store.update_task(&StoreUpdateTaskRequest {
        task_id: "task-late".to_owned(),
        expected_revision: 2,
        command: StoreTaskCommand {
            kind: TaskStateCommandKind::ResumeItems,
            item_ids: vec![late_item],
        },
        event_id: "event-task-late-stale".to_owned(),
        occurred_at: "2024-02-01T01:00:04Z".to_owned(),
    });
    assert_eq!(stale.unwrap_err().code(), "V3_STORE_REVISION_CONFLICT");

    store
        .archive_task(&StoreArchiveTaskRequest {
            task_id: "task-history".to_owned(),
            expected_revision: 1,
            archived_at: "2024-02-01T02:00:00Z".to_owned(),
        })
        .unwrap();
    store
        .archive_task(&StoreArchiveTaskRequest {
            task_id: "task-tombstone".to_owned(),
            expected_revision: 1,
            archived_at: "2024-02-01T02:00:01Z".to_owned(),
        })
        .unwrap();
    store
        .delete_history(&StoreDeleteHistoryRequest {
            task_id: "task-tombstone".to_owned(),
            deleted_at: "2024-02-01T03:00:00Z".to_owned(),
        })
        .unwrap();
}

#[test]
fn sidecar_restore_matches_the_canonical_store_projection_for_non_empty_multi_state_fixture() {
    let database = TempStore::new();
    seed_complex_store(database.path());

    let dispatcher = Dispatcher::try_new_with_store_path(database.path().to_path_buf()).unwrap();
    let sidecar = dispatcher.dispatch(request(
        "complex-sidecar-restore",
        SidecarOperation::StoreRestore,
        json!({"include_terminal": true}),
    ));
    assert!(sidecar.ok, "sidecar restore failed: {:?}", sidecar.error);

    let store = TaskStore::open_existing(database.path()).expect("canonical Store opens");
    let tauri_restore = serde_json::to_value(
        store
            .restore(&insar_core::StoreRestoreRequest {
                include_terminal: true,
            })
            .expect("canonical Store restore succeeds"),
    )
    .expect("canonical Store restore serializes")
    .as_object()
    .cloned()
    .expect("canonical Store restore is an object");

    assert_eq!(sidecar.result, tauri_restore);
    assert_eq!(sidecar.result["workbench"]["batch_id"], "batch-a");
    assert_eq!(sidecar.result["workbench_batch"]["aoi_name"], "AOI A");
    assert_eq!(sidecar.result["tasks"].as_array().unwrap().len(), 6);
    assert_eq!(sidecar.result["runtimes"].as_array().unwrap().len(), 6);
    assert!(sidecar.result["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|task| task["task_id"] == "task-history"));
    assert!(!sidecar.result["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|task| task["task_id"] == "task-tombstone"));
    let late_runtime = sidecar.result["runtimes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|runtime| runtime["task_id"] == "task-late")
        .expect("late task runtime is restored");
    assert_eq!(late_runtime["revision"], 3);
    assert_eq!(late_runtime["status"], "paused");
}

#[test]
fn startup_health_does_not_report_ready_before_the_store_opens() {
    let database = TempStore::new();
    fs::write(database.path(), b"not a sqlite database").unwrap();
    let dispatcher = Dispatcher::try_new_with_store_path(database.path().to_path_buf()).unwrap();

    let mut health_request = request(
        "startup-store-readiness",
        SidecarOperation::Health,
        json!({"probe": "startup", "require_store": true}),
    );
    health_request.protocol_minor = 16;
    let health = dispatcher.dispatch(health_request);

    assert!(!health.ok);
    assert_eq!(
        health.error.expect("store readiness error").code,
        "V3_STORE_DATABASE_ERROR"
    );
}

#[test]
fn startup_health_without_store_requirement_remains_a_liveness_probe() {
    let database = TempStore::new();
    fs::write(database.path(), b"not a sqlite database").unwrap();
    let dispatcher = Dispatcher::try_new_with_store_path(database.path().to_path_buf()).unwrap();

    let health = dispatcher.dispatch(request(
        "startup-liveness-only",
        SidecarOperation::Health,
        json!({"probe": "startup", "require_store": false}),
    ));

    assert!(health.ok);
    assert_eq!(health.result["status"], "healthy");
}

#[test]
fn protocol_1_6_store_restores_after_dispatcher_restart_and_older_capabilities_stay_bounded() {
    let database = TempStore::new();
    let output_root = database.path().parent().unwrap().join("sidecar-output");
    {
        let dispatcher =
            Dispatcher::try_new_with_store_path(database.path().to_path_buf()).unwrap();
        for request_id in ["save-batch", "save-batch-replay"] {
            assert!(
                dispatcher
                    .dispatch(request(
                        request_id,
                        SidecarOperation::StoreSearchBatch,
                        batch_payload(),
                    ))
                    .ok
            );
        }
        for request_id in ["create-task", "create-task-replay"] {
            assert!(
                dispatcher
                    .dispatch(request(
                        request_id,
                        SidecarOperation::StoreCreateTask,
                        create_payload(&output_root),
                    ))
                    .ok
            );
        }
        assert!(
            dispatcher
                .dispatch(request(
                    "pause-task",
                    SidecarOperation::StoreUpdateTask,
                    pause_payload(),
                ))
                .ok
        );
        let mut update_task_view = request(
            "update-task-view",
            SidecarOperation::StoreUpdateTaskView,
            json!({
                "task_id": "task-sidecar",
                "checked_item_ids": [],
                "page_index": 4,
                "page_size": 24,
                "updated_at": "2024-01-15T01:00:03Z"
            }),
        );
        update_task_view.protocol_minor = 10;
        assert!(dispatcher.dispatch(update_task_view).ok);
        assert!(
            dispatcher
                .dispatch(request(
                    "update-workbench",
                    SidecarOperation::StoreUpdateWorkbench,
                    json!({
                        "state": {
                            "batch_id": "batch-sidecar",
                            "checked_item_ids": [],
                            "page_index": 2,
                            "page_size": 24,
                            "revision": 1
                        },
                        "expected_revision": 0,
                        "updated_at": "2024-01-15T01:00:02Z"
                    }),
                ))
                .ok
        );
    }

    let dispatcher = Dispatcher::try_new_with_store_path(database.path().to_path_buf()).unwrap();
    let restored = dispatcher.dispatch(request(
        "restore",
        SidecarOperation::StoreRestore,
        json!({"include_terminal": false}),
    ));
    assert!(restored.ok);
    assert_eq!(restored.result["tasks"][0]["task_id"], "task-sidecar");
    assert_eq!(restored.result["tasks"][0]["aoi_name"], "四川省雅安市");
    assert_eq!(
        restored.result["tasks"][0]["output_root"],
        json!(output_root)
    );
    assert_eq!(restored.result["runtimes"][0]["status"], "paused");
    assert_eq!(restored.result["runtimes"][0]["page_index"], 4);
    assert_eq!(
        restored.result["runtimes"][0]["checked_item_ids"],
        json!([])
    );
    assert_eq!(restored.result["runtimes"][0]["revision"], 1);
    assert_eq!(restored.result["workbench"]["page_index"], 2);

    let mut old = request(
        "old-store",
        SidecarOperation::StoreRestore,
        json!({"include_terminal": false}),
    );
    old.protocol_minor = 4;
    let rejected = dispatcher.dispatch(old);
    assert_eq!(
        rejected.error.unwrap().code,
        "V3_SIDECAR_CAPABILITY_UNAVAILABLE"
    );

    let mut protocol_1_5 = request(
        "old-workbench-update",
        SidecarOperation::StoreUpdateWorkbench,
        json!({}),
    );
    protocol_1_5.protocol_minor = 5;
    assert_eq!(
        dispatcher.dispatch(protocol_1_5).error.unwrap().code,
        "V3_SIDECAR_CAPABILITY_UNAVAILABLE"
    );
}

fn run_process_session(database: &Path, requests: Vec<SidecarRequest>) -> Vec<SidecarResponse> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_insar-service"))
        .arg("--store-path")
        .arg(database)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("sidecar starts");
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut responses = Vec::new();
    for request in requests {
        writeln!(stdin, "{}", serde_json::to_string(&request).unwrap()).unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        responses.push(serde_json::from_str(&line).unwrap());
    }
    drop(stdin);
    drop(stdout);
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    responses
}

#[test]
fn real_sidecar_process_restores_the_same_paused_task_after_restart() {
    let database = TempStore::new();
    let output_root = database
        .path()
        .parent()
        .unwrap()
        .join("real-process-output");
    let first = run_process_session(
        database.path(),
        vec![
            request(
                "process-batch",
                SidecarOperation::StoreSearchBatch,
                batch_payload(),
            ),
            request(
                "process-create",
                SidecarOperation::StoreCreateTask,
                create_payload(&output_root),
            ),
            request(
                "process-pause",
                SidecarOperation::StoreUpdateTask,
                pause_payload(),
            ),
            request("process-shutdown", SidecarOperation::Shutdown, json!({})),
        ],
    );
    assert!(first.iter().all(|response| response.ok));

    let second = run_process_session(
        database.path(),
        vec![
            request(
                "process-restore",
                SidecarOperation::StoreRestore,
                json!({"include_terminal": false}),
            ),
            request("process-shutdown-2", SidecarOperation::Shutdown, json!({})),
        ],
    );
    assert!(second.iter().all(|response| response.ok));
    assert_eq!(second[0].result["tasks"][0]["task_id"], "task-sidecar");
    assert_eq!(second[0].result["runtimes"][0]["status"], "paused");
    assert_eq!(second[0].result["runtimes"][0]["revision"], 1);
}

#[test]
fn rust_download_capability_is_explicitly_gated_and_exposes_persisted_snapshot() {
    let database = TempStore::new();
    let output_root = database
        .path()
        .parent()
        .unwrap()
        .join("download-gate-output");
    let disabled = Dispatcher::try_new_with_store_path(database.path().to_path_buf()).unwrap();
    assert!(
        disabled
            .dispatch(request(
                "gate-batch",
                SidecarOperation::StoreSearchBatch,
                batch_payload(),
            ))
            .ok
    );
    assert!(
        disabled
            .dispatch(request(
                "gate-task",
                SidecarOperation::StoreCreateTask,
                create_payload(&output_root),
            ))
            .ok
    );
    assert!(
        disabled
            .dispatch(request(
                "gate-pause",
                SidecarOperation::StoreUpdateTask,
                pause_payload(),
            ))
            .ok
    );
    let mut disabled_handshake =
        request("disabled-handshake", SidecarOperation::Handshake, json!({}));
    disabled_handshake.protocol_minor = 14;
    let handshake = disabled.dispatch(disabled_handshake);
    assert!(!handshake.result["capabilities"]
        .as_array()
        .unwrap()
        .contains(&json!("download_snapshot")));
    assert!(handshake.result["capabilities"]
        .as_array()
        .unwrap()
        .contains(&json!("asset_execution_policy_v1")));
    assert!(!handshake.result["capabilities"]
        .as_array()
        .unwrap()
        .contains(&json!("resolved_asset_transfer_v1")));
    assert_eq!(
        disabled
            .dispatch(request(
                "disabled-snapshot",
                SidecarOperation::DownloadSnapshot,
                json!({"task_id": "task-sidecar"}),
            ))
            .error
            .unwrap()
            .code,
        "V3_SIDECAR_CAPABILITY_UNAVAILABLE"
    );
    drop(disabled);

    let enabled = Dispatcher::try_new_with_downloads(database.path().to_path_buf()).unwrap();
    let mut enabled_handshake =
        request("enabled-handshake", SidecarOperation::Handshake, json!({}));
    enabled_handshake.protocol_minor = 14;
    let handshake = enabled.dispatch(enabled_handshake);
    assert!(handshake.result["capabilities"]
        .as_array()
        .unwrap()
        .contains(&json!("download_snapshot")));
    assert!(handshake.result["capabilities"]
        .as_array()
        .unwrap()
        .contains(&json!("asset_execution_policy_v1")));
    assert!(handshake.result["capabilities"]
        .as_array()
        .unwrap()
        .contains(&json!("resolved_asset_transfer_v1")));
    let snapshot = enabled.dispatch(request(
        "enabled-snapshot",
        SidecarOperation::DownloadSnapshot,
        json!({"task_id": "task-sidecar"}),
    ));
    assert!(snapshot.ok);
    assert_eq!(snapshot.result["revision"], 0);
    assert!(
        enabled
            .dispatch(request(
                "enabled-wake",
                SidecarOperation::DownloadWake,
                json!({}),
            ))
            .ok
    );
}

#[test]
fn protocol_1_14_restores_missing_legacy_and_resolver_policies_from_snapshot_json() {
    let database = TempStore::new();
    let output_root = database.path().parent().unwrap().join("policy-output");
    let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures/asset_execution_policy/sentinel1_standard_cases.json");
    let fixture: Value = serde_json::from_str(&fs::read_to_string(fixture_path).unwrap()).unwrap();
    let resolver = fixture["base_item"].clone();
    let item_ids = json!([
        "missing-policy",
        "explicit-legacy",
        resolver["item_id"].as_str().unwrap()
    ]);
    let batch = json!({
        "batch": {
            "schema_version": 1,
            "batch_id": "policy-sidecar-batch",
            "provider_id": "asf.catalog",
            "created_at": "2026-07-18T00:00:00Z",
            "aoi_name": "fixture-aoi",
            "aoi_geojson": {"type": "Polygon", "coordinates": []},
            "query": {"dataset": "SENTINEL-1"},
            "candidates": [
                {
                    "item_id": "missing-policy",
                    "provider_id": "asf.catalog",
                    "display_name": "missing-policy.zip",
                    "metadata": {},
                    "payload": {}
                },
                {
                    "item_id": "explicit-legacy",
                    "provider_id": "asf.catalog",
                    "display_name": "explicit-legacy.zip",
                    "metadata": {},
                    "payload": {},
                    "execution_policy": {
                        "authority": "legacy",
                        "policy_id": "legacy.v1",
                        "collection_profile_id": null
                    }
                },
                resolver
            ]
        },
        "workbench": {
            "batch_id": "policy-sidecar-batch",
            "checked_item_ids": item_ids,
            "page_index": 0,
            "page_size": 24,
            "revision": 0
        },
        "activate_workbench": false
    });
    {
        let dispatcher =
            Dispatcher::try_new_with_store_path(database.path().to_path_buf()).unwrap();
        let mut save = request(
            "save-policy-batch",
            SidecarOperation::StoreSearchBatch,
            batch,
        );
        save.protocol_minor = 14;
        assert!(dispatcher.dispatch(save).ok);
        let mut create = request(
            "create-policy-task",
            SidecarOperation::StoreCreateTask,
            json!({
                "task_id": "policy-sidecar-task",
                "batch_id": "policy-sidecar-batch",
                "selected_item_ids": item_ids,
                "checked_item_ids": item_ids,
                "page_index": 0,
                "page_size": 24,
                "output_root": output_root,
                "layout": "direct",
                "concurrency": 2,
                "created_at": "2026-07-18T00:00:01Z"
            }),
        );
        create.protocol_minor = 14;
        assert!(dispatcher.dispatch(create).ok);
    }

    let dispatcher = Dispatcher::try_new_with_store_path(database.path().to_path_buf()).unwrap();
    let mut restore = request(
        "restore-policy-task",
        SidecarOperation::StoreRestore,
        json!({"include_terminal": false}),
    );
    restore.protocol_minor = 14;
    let restored = dispatcher.dispatch(restore);
    assert!(restored.ok);
    let candidates = restored.result["tasks"][0]["candidates"]
        .as_array()
        .unwrap();
    assert!(candidates[0]["execution_policy"].is_null());
    assert_eq!(candidates[1]["execution_policy"]["policy_id"], "legacy.v1");
    assert_eq!(
        candidates[2]["execution_policy"]["policy_id"],
        "asf.sentinel-1.standard.resolver-cutover.v1"
    );
}
