//! Isolated real Application/Store/Scheduler; the raster port is synthetic.
use insar_application_service::{
    ApplicationService, ApplicationServiceConfig, CapabilityRuntimeObservations, DemOperationPort,
    DemTaskSettings, DemTransferEvent, StandaloneDemTaskRequest,
};
use insar_core::{
    AsfRemoteSearchRequest, AsfRemoteSearchResponse, BuildCapabilityState, ContractError,
    DownloadItemProgress, DownloadItemResult, DownloadOutcome, ProviderResult,
    RuntimeReadinessState, StoreTaskCommand, StoreUpdateTaskRequest, TaskLifecycleStatus,
    TaskStateCommandKind,
};
use insar_download::{DownloadWork, DownloadWorker, WorkerControl};
use insar_service::LiveAsfSearch;
use serde_json::{json, Map, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

struct Temp {
    root: PathBuf,
    database: PathBuf,
}
impl Temp {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("sceneport-dem-limits-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        Self {
            database: root.join("tasks.sqlite3"),
            root,
        }
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
struct EmptySearch;
impl LiveAsfSearch for EmptySearch {
    fn search(
        &self,
        _: &AsfRemoteSearchRequest,
        _: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<AsfRemoteSearchResponse> {
        Err(ContractError::new("FIXTURE_NO_SEARCH", "synthetic", Map::new()).unwrap())
    }
}
struct UnusedWorker;
impl DownloadWorker for UnusedWorker {
    fn execute(
        &self,
        work: DownloadWork,
        _: WorkerControl,
        _: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        DownloadItemResult {
            task_id: work.task.task_id,
            item_id: work.item.item_id,
            outcome: DownloadOutcome::Completed,
            destination: work.task.output_root,
            bytes_written: 0,
            attempt_count: 1,
            failure: None,
            finished_at: "2026-09-30T00:00:05Z".into(),
        }
    }
}
#[derive(Default)]
struct State {
    active: BTreeSet<String>,
    starts: BTreeMap<String, usize>,
    finish: BTreeSet<String>,
    hold_exit: BTreeSet<String>,
    interrupted: BTreeSet<String>,
    max_active: usize,
}
#[derive(Default)]
struct Port(Mutex<State>);
impl Port {
    fn started(&self, id: &str) -> usize {
        *self.0.lock().unwrap().starts.get(id).unwrap_or(&0)
    }
    fn finish(&self, id: &str) {
        self.0.lock().unwrap().finish.insert(id.into());
    }
}
impl DemOperationPort for Port {
    fn call(&self, _: &str, _: Value) -> Result<Value, String> {
        Err("unexpected unbounded port call".into())
    }
    fn call_with_transfer_progress(
        &self,
        operation: &str,
        payload: Value,
        interrupted: &(dyn Fn() -> bool + Sync),
        _: &mut dyn FnMut(DemTransferEvent),
    ) -> Result<Value, String> {
        assert_eq!(operation, "dem_download");
        let id = payload["task_id"].as_str().unwrap().to_owned();
        {
            let mut state = self.0.lock().unwrap();
            assert!(
                state.active.insert(id.clone()),
                "one worker per synthetic DEM task"
            );
            *state.starts.entry(id.clone()).or_default() += 1;
            state.max_active = state.max_active.max(state.active.len());
        }
        let deadline = Instant::now() + Duration::from_secs(12);
        let cancelled = loop {
            let stop = interrupted();
            let mut state = self.0.lock().unwrap();
            if stop {
                state.interrupted.insert(id.clone());
            }
            if stop && !state.hold_exit.contains(&id) {
                break true;
            }
            if !stop && state.finish.contains(&id) {
                break false;
            }
            assert!(
                Instant::now() < deadline,
                "synthetic DEM worker was never released"
            );
            drop(state);
            thread::sleep(Duration::from_millis(3));
        };
        self.0.lock().unwrap().active.remove(&id);
        if cancelled {
            return Err("synthetic interrupted".into());
        }
        let destination = PathBuf::from(payload["input_path"].as_str().unwrap());
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::write(&destination, b"isolated DEM fixture").unwrap();
        Ok(json!({"ok":true,"bytes_written":20}))
    }
}
fn app(temp: &Temp, port: &Arc<Port>) -> ApplicationService {
    ApplicationService::with_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&temp.database)
            .with_capability_runtime_observations(
                CapabilityRuntimeObservations::default()
                    .with_build(
                        "dem-provider.copernicus-aws.v1",
                        BuildCapabilityState::Available,
                    )
                    .with_runtime(
                        "dem-provider.copernicus-aws.v1",
                        RuntimeReadinessState::Ready,
                    ),
            ),
        Arc::new(EmptySearch),
        Arc::new(UnusedWorker),
    )
    .attach_dem_operation_port(port.clone())
}
fn limit(app: &ApplicationService, n: u64) {
    app.save_dem_task_settings(&DemTaskSettings {
        max_concurrent_tasks: n,
        max_concurrent_conversions: 1,
    })
    .unwrap();
}
fn create(app: &ApplicationService, temp: &Temp, id: &str, index: u32) {
    app.create_standalone_dem_task(&StandaloneDemTaskRequest {
        request_id: format!("request-{id}"),
        task_id: id.into(),
        dataset: "COP30".into(),
        provider: "COPERNICUS_AWS".into(),
        bbox: vec![
            100.0 + f64::from(index) * 0.1,
            30.0,
            100.05 + f64::from(index) * 0.1,
            30.05,
        ],
        output_root: temp.root.join("output"),
        convert: false,
        concurrency: 10,
        created_at: format!("2026-09-30T00:00:{index:02}Z"),
    })
    .unwrap();
}
fn wait(mut yes: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !yes() {
        assert!(Instant::now() < deadline, "DEM state did not settle");
        thread::sleep(Duration::from_millis(5));
    }
}
fn status(app: &ApplicationService, id: &str) -> TaskLifecycleStatus {
    app.core_restore(false)
        .unwrap()
        .core
        .tasks
        .into_iter()
        .find(|t| t.task_id == id)
        .unwrap()
        .status
}
fn control(app: &ApplicationService, id: &str, kind: TaskStateCommandKind) {
    let task = app
        .core_restore(false)
        .unwrap()
        .core
        .tasks
        .into_iter()
        .find(|t| t.task_id == id)
        .unwrap();
    app.update_download_task(&StoreUpdateTaskRequest {
        task_id: id.into(),
        expected_revision: task.revision,
        command: StoreTaskCommand {
            kind,
            item_ids: vec![],
        },
        event_id: uuid::Uuid::new_v4().to_string(),
        occurred_at: "2026-09-30T00:01:00Z".into(),
    })
    .unwrap();
}

#[test]
fn saved_dem_limit_controls_whole_tasks_and_live_changes_not_per_task_items() {
    let temp = Temp::new();
    let port = Arc::new(Port::default());
    let app = app(&temp, &port);
    limit(&app, 1);
    for (i, id) in ["dem-a", "dem-b", "dem-c"].iter().enumerate() {
        create(&app, &temp, id, i as u32);
    }
    wait(|| port.started("dem-a") == 1);
    thread::sleep(Duration::from_millis(180));
    assert_eq!(
        port.started("dem-b"),
        0,
        "limit 1 must queue the second single-item DEM task"
    );
    assert_eq!(status(&app, "dem-b"), TaskLifecycleStatus::Queued);
    limit(&app, 2);
    wait(|| port.started("dem-b") == 1);
    assert_eq!(port.started("dem-c"), 0);
    limit(&app, 1);
    assert_eq!(
        port.0.lock().unwrap().active.len(),
        2,
        "lowering does not abort existing work"
    );
    port.finish("dem-a");
    wait(|| status(&app, "dem-a") == TaskLifecycleStatus::Completed);
    thread::sleep(Duration::from_millis(100));
    assert_eq!(port.started("dem-c"), 0);
    port.finish("dem-b");
    wait(|| port.started("dem-c") == 1);
    port.finish("dem-c");
    wait(|| status(&app, "dem-c") == TaskLifecycleStatus::Completed);
    assert_eq!(port.0.lock().unwrap().max_active, 2);
    app.shutdown().unwrap();
    let reopened = app_fn(&temp, &port);
    assert_eq!(
        reopened
            .get_dem_task_settings()
            .unwrap()
            .max_concurrent_tasks,
        1
    );
    for id in ["dem-a", "dem-b", "dem-c"] {
        assert_eq!(status(&reopened, id), TaskLifecycleStatus::Completed);
    }
    assert_eq!(
        fs::read_dir(temp.root.join("output"))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().is_some_and(|x| x == "tif"))
            .count(),
        3
    );
    reopened.shutdown().unwrap();
}
fn app_fn(temp: &Temp, port: &Arc<Port>) -> ApplicationService {
    app(temp, port)
}

#[test]
fn ten_dem_tasks_run_but_the_eleventh_waits_and_invalid_limits_never_replace_the_setting() {
    let temp = Temp::new();
    let port = Arc::new(Port::default());
    let app = app(&temp, &port);
    limit(&app, 10);
    for invalid in [0, 11] {
        assert!(app
            .save_dem_task_settings(&DemTaskSettings {
                max_concurrent_tasks: invalid,
                max_concurrent_conversions: 1
            })
            .is_err());
    }
    assert_eq!(
        app.get_dem_task_settings().unwrap().max_concurrent_tasks,
        10
    );
    for i in 0..11 {
        create(&app, &temp, &format!("dem-{i:02}"), i);
    }
    wait(|| port.0.lock().unwrap().active.len() == 10);
    assert_eq!(port.started("dem-10"), 0);
    port.finish("dem-00");
    wait(|| port.started("dem-10") == 1);
    for i in 1..11 {
        port.finish(&format!("dem-{i:02}"));
    }
    wait(|| {
        app.core_restore(false)
            .unwrap()
            .core
            .tasks
            .iter()
            .all(|t| t.status == TaskLifecycleStatus::Completed)
    });
    assert_eq!(port.0.lock().unwrap().max_active, 10);
    app.shutdown().unwrap();
}

#[test]
fn a_non_dem_task_can_resume_while_the_dem_task_pool_is_full() {
    use insar_core::{
        DownloadLayout, FrozenSearchBatch, SearchWorkbenchState, SnapshotItem,
        StoreCreateTaskRequest, StoreSearchBatchRequest,
    };
    let temp = Temp::new();
    {
        let mut store = insar_store::TaskStore::open(&temp.database).unwrap();
        store
            .save_search_batch(&StoreSearchBatchRequest {
                activate_workbench: false,
                batch: FrozenSearchBatch {
                    schema_version: 1,
                    batch_id: "sar-source".into(),
                    provider_id: "asf.sentinel1".into(),
                    created_at: "2026-09-30T00:00:00Z".into(),
                    aoi_name: "Synthetic".into(),
                    aoi_geojson: None,
                    query: Map::new(),
                    search_stats: None,
                    candidates: vec![SnapshotItem {
                        item_id: "sar-item".into(),
                        provider_id: "asf.sentinel1".into(),
                        display_name: "fixture.zip".into(),
                        acquisition_datetime: None,
                        footprint_geojson: None,
                        metadata: Map::new(),
                        payload: Map::new(),
                        asset_access: None,
                        execution_policy: None,
                    }],
                },
                workbench: SearchWorkbenchState {
                    batch_id: "sar-source".into(),
                    checked_item_ids: vec![],
                    page_index: 0,
                    page_size: 24,
                    revision: 0,
                },
            })
            .unwrap();
        let created = store
            .create_task(&StoreCreateTaskRequest {
                task_id: "sar-task".into(),
                batch_id: "sar-source".into(),
                selected_item_ids: vec!["sar-item".into()],
                checked_item_ids: vec![],
                page_index: 0,
                page_size: 24,
                output_root: temp.root.join("sar"),
                layout: DownloadLayout::Direct,
                concurrency: 1,
                created_at: "2026-09-30T00:00:00Z".into(),
            })
            .unwrap();
        store
            .update_task(&StoreUpdateTaskRequest {
                task_id: "sar-task".into(),
                expected_revision: created.runtime.revision,
                command: StoreTaskCommand {
                    kind: TaskStateCommandKind::PauseTask,
                    item_ids: vec![],
                },
                event_id: "pause-sar".into(),
                occurred_at: "2026-09-30T00:00:01Z".into(),
            })
            .unwrap();
    }
    let port = Arc::new(Port::default());
    let app = app(&temp, &port);
    limit(&app, 1);
    create(&app, &temp, "dem-a", 0);
    create(&app, &temp, "dem-b", 1);
    wait(|| port.started("dem-a") == 1);
    control(&app, "sar-task", TaskStateCommandKind::ResumeTask);
    wait(|| status(&app, "sar-task") == TaskLifecycleStatus::Completed);
    assert_eq!(port.0.lock().unwrap().active.len(), 1);
    assert_eq!(port.started("dem-b"), 0);
    port.finish("dem-a");
    wait(|| port.started("dem-b") == 1);
    port.finish("dem-b");
    wait(|| status(&app, "dem-b") == TaskLifecycleStatus::Completed);
    app.shutdown().unwrap();
}

#[test]
#[cfg(windows)]
fn failed_settings_replacement_preserves_the_file_and_the_live_limit() {
    use std::os::windows::fs::OpenOptionsExt;
    let temp = Temp::new();
    let port = Arc::new(Port::default());
    let app = app(&temp, &port);
    limit(&app, 1);
    create(&app, &temp, "dem-a", 0);
    create(&app, &temp, "dem-b", 1);
    wait(|| port.started("dem-a") == 1);
    let path = temp.root.join("dem-task-settings.json");
    let original = fs::read(&path).unwrap();
    let locked = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&path)
        .unwrap();
    assert!(app
        .save_dem_task_settings(&DemTaskSettings {
            max_concurrent_tasks: 2,
            max_concurrent_conversions: 1
        })
        .is_err());
    drop(locked);
    assert_eq!(fs::read(&path).unwrap(), original);
    thread::sleep(Duration::from_millis(120));
    assert_eq!(port.started("dem-b"), 0);
    assert!(!fs::read_dir(&temp.root)
        .unwrap()
        .filter_map(Result::ok)
        .any(|entry| entry.file_name().to_string_lossy().ends_with(".tmp")));
    port.finish("dem-a");
    wait(|| port.started("dem-b") == 1);
    port.finish("dem-b");
    wait(|| status(&app, "dem-b") == TaskLifecycleStatus::Completed);
    app.shutdown().unwrap();
}

#[test]
fn paused_dem_keeps_slot_until_worker_exits_then_resume_uses_same_task() {
    let temp = Temp::new();
    let port = Arc::new(Port::default());
    let app = app(&temp, &port);
    limit(&app, 1);
    port.0.lock().unwrap().hold_exit.insert("dem-a".into());
    create(&app, &temp, "dem-a", 0);
    create(&app, &temp, "dem-b", 1);
    wait(|| port.started("dem-a") == 1);
    control(&app, "dem-a", TaskStateCommandKind::PauseTask);
    wait(|| port.0.lock().unwrap().interrupted.contains("dem-a"));
    thread::sleep(Duration::from_millis(120));
    assert_eq!(
        port.started("dem-b"),
        0,
        "pause acknowledgment alone cannot release a running native worker slot"
    );
    port.0.lock().unwrap().hold_exit.remove("dem-a");
    wait(|| port.started("dem-b") == 1);
    assert_eq!(status(&app, "dem-a"), TaskLifecycleStatus::Paused);
    control(&app, "dem-a", TaskStateCommandKind::ResumeTask);
    thread::sleep(Duration::from_millis(100));
    assert_eq!(port.started("dem-a"), 1);
    port.finish("dem-b");
    wait(|| port.started("dem-a") == 2);
    port.finish("dem-a");
    wait(|| status(&app, "dem-a") == TaskLifecycleStatus::Completed);
    assert_eq!(port.0.lock().unwrap().max_active, 1);
    assert_eq!(app.core_restore(false).unwrap().core.tasks.len(), 2);
    app.shutdown().unwrap();
}

#[test]
fn restart_loads_limit_before_recovering_queued_dem_tasks() {
    let temp = Temp::new();
    let port = Arc::new(Port::default());
    let first = app(&temp, &port);
    limit(&first, 1);
    for (i, id) in ["dem-a", "dem-b", "dem-c"].iter().enumerate() {
        create(&first, &temp, id, i as u32);
    }
    wait(|| port.started("dem-a") == 1);
    first.shutdown().unwrap();
    let next_port = Arc::new(Port::default());
    let next = app(&temp, &next_port);
    next.startup().unwrap();
    wait(|| next_port.started("dem-a") == 1);
    thread::sleep(Duration::from_millis(120));
    assert_eq!(next_port.started("dem-b"), 0);
    for id in ["dem-a", "dem-b", "dem-c"] {
        wait(|| next_port.started(id) == 1);
        next_port.finish(id);
        wait(|| status(&next, id) == TaskLifecycleStatus::Completed);
    }
    assert_eq!(next_port.0.lock().unwrap().max_active, 1);
    next.shutdown().unwrap();
}
