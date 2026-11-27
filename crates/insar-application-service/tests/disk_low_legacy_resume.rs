//! The legacy `update_download_task` path (old command and the Agent's
//! resume) and the NISAR subset resume/start commands refuse a resume on a
//! low drive with an additive DISK_LOW error and leave every other command
//! as it was. The default probe is driven through a process-wide test knob,
//! so every test here holds `KNOB` for its whole run.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use insar_agent::executor::ErrorClass;
use insar_agent::host::UnifiedApplication;
use insar_application_service::{
    AgentApplication, ApplicationService, ApplicationServiceConfig, ApplicationServiceError,
    UnifiedTaskAction,
};
use insar_core::{
    DownloadLayout, FrozenSearchBatch, SearchWorkbenchState, SnapshotItem, StoreCreateTaskRequest,
    StoreSearchBatchRequest, StoreTaskCommand, StoreUpdateTaskRequest, TaskLifecycleStatus,
    TaskStateCommandKind,
};
use insar_download::disk_guard::{drive_label, test_support};
use insar_store::TaskStore;
use serde_json::Map;

const GIB: u64 = 1 << 30;
static KNOB: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct Fixture(PathBuf);

impl Drop for Fixture {
    fn drop(&mut self) {
        test_support::set_default_probe_free_bytes(None);
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn create_task(store: &mut TaskStore, task_id: &str, output_root: &Path) {
    let batch_id = format!("{task_id}-batch");
    let item_id = format!("{task_id}-item");
    store
        .save_search_batch(&StoreSearchBatchRequest {
            batch: FrozenSearchBatch {
                schema_version: 1,
                batch_id: batch_id.clone(),
                provider_id: "fixture.provider".to_owned(),
                created_at: "2026-10-26T00:00:00Z".to_owned(),
                aoi_name: "fixture AOI".to_owned(),
                aoi_geojson: None,
                query: Map::new(),
                search_stats: None,
                candidates: vec![SnapshotItem {
                    item_id: item_id.clone(),
                    provider_id: "fixture.provider".to_owned(),
                    display_name: item_id.clone(),
                    acquisition_datetime: None,
                    footprint_geojson: None,
                    metadata: Map::new(),
                    payload: Map::new(),
                    asset_access: None,
                    execution_policy: None,
                }],
            },
            workbench: SearchWorkbenchState {
                batch_id: batch_id.clone(),
                checked_item_ids: vec![],
                page_index: 0,
                page_size: 25,
                revision: 0,
            },
            activate_workbench: false,
        })
        .unwrap();
    store
        .create_task(&StoreCreateTaskRequest {
            task_id: task_id.to_owned(),
            batch_id,
            selected_item_ids: vec![item_id.clone()],
            checked_item_ids: vec![item_id],
            page_index: 0,
            page_size: 25,
            output_root: output_root.to_path_buf(),
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-10-26T00:00:01Z".to_owned(),
        })
        .unwrap();
}

fn command(
    task_id: &str,
    revision: u64,
    kind: TaskStateCommandKind,
    event: &str,
) -> StoreUpdateTaskRequest {
    StoreUpdateTaskRequest {
        task_id: task_id.to_owned(),
        expected_revision: revision,
        command: StoreTaskCommand {
            kind,
            item_ids: vec![],
        },
        event_id: event.to_owned(),
        occurred_at: "2026-10-26T00:02:00Z".to_owned(),
    }
}

fn revision(service: &ApplicationService, task_id: &str) -> (u64, TaskLifecycleStatus) {
    let task = service
        .task_overview()
        .unwrap()
        .queue
        .into_iter()
        .find(|task| task.task_id == task_id)
        .unwrap();
    (task.revision, task.status)
}

/// Read from the command's own snapshot: the overview archives terminal tasks.
fn status_in(
    snapshot: &insar_application_service::CoreServiceSnapshot,
    task_id: &str,
) -> TaskLifecycleStatus {
    snapshot
        .core
        .tasks
        .iter()
        .find(|task| task.task_id == task_id)
        .unwrap()
        .status
        .clone()
}

#[test]
fn legacy_and_agent_resume_are_refused_on_a_low_drive_and_other_commands_are_unchanged() {
    let _knob = KNOB.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = std::env::temp_dir().join(format!("insar-disk-low-legacy-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let fixture = Fixture(dir);
    let output_root = fixture.0.join("out");
    let database = fixture.0.join("store.sqlite3");
    {
        let mut store = TaskStore::open(&database).unwrap();
        for task_id in ["legacy-task", "agent-task", "unified-task", "other-task"] {
            create_task(&mut store, task_id, &output_root);
            store
                .update_task(&command(
                    task_id,
                    0,
                    TaskStateCommandKind::PauseTask,
                    &format!("{task_id}-pause"),
                ))
                .unwrap();
        }
    }
    let service = Arc::new(ApplicationService::new(
        ApplicationServiceConfig::business_store(&database),
    ));
    test_support::set_default_probe_free_bytes(Some(GIB + GIB / 2));

    // Legacy command path: an additive, typed refusal; the task is unchanged.
    let (legacy_revision, _) = revision(&service, "legacy-task");
    let error = service
        .update_download_task(&command(
            "legacy-task",
            legacy_revision,
            TaskStateCommandKind::ResumeTask,
            "legacy-resume-low",
        ))
        .expect_err("a legacy resume on a low drive is refused");
    assert_eq!(error.code(), "DISK_LOW");
    match &error {
        ApplicationServiceError::DiskLow { reason } => {
            assert_eq!(reason.free_bytes, GIB + GIB / 2);
            assert_eq!(reason.threshold_bytes, 2 * GIB);
            assert_eq!(reason.drive, drive_label(&output_root));
        }
        other => panic!("unexpected error {other:?}"),
    }
    assert_eq!(
        revision(&service, "legacy-task"),
        (legacy_revision, TaskLifecycleStatus::Paused)
    );
    // Item-level resume is a resume too.
    let mut items = command(
        "legacy-task",
        legacy_revision,
        TaskStateCommandKind::ResumeItems,
        "legacy-resume-items-low",
    );
    items.command.item_ids = vec!["legacy-task-item".to_owned()];
    assert_eq!(
        service.update_download_task(&items).unwrap_err().code(),
        "DISK_LOW"
    );

    // Every other command keeps its semantics on a low drive.
    let (other_revision, _) = revision(&service, "other-task");
    let cancelled = service
        .update_download_task(&command(
            "other-task",
            other_revision,
            TaskStateCommandKind::CancelTask,
            "other-cancel-low",
        ))
        .expect("cancel needs no space");
    assert_eq!(
        status_in(&cancelled, "other-task"),
        TaskLifecycleStatus::Cancelled
    );

    // The Agent's resume goes through the same path.
    let agent = AgentApplication(Arc::clone(&service));
    let (agent_revision, _) = revision(&service, "agent-task");
    assert_eq!(
        agent
            .application_task_action("agent-task", agent_revision, "resume", "agent-resume-low")
            .unwrap_err(),
        ErrorClass::Business
    );
    assert_eq!(
        revision(&service, "agent-task").1,
        TaskLifecycleStatus::Paused
    );

    // The unified action, with its default probe, words the refusal.
    let (unified_revision, _) = revision(&service, "unified-task");
    let typed = service
        .task_action(
            "unified-task".to_owned(),
            unified_revision,
            UnifiedTaskAction::Resume,
        )
        .unwrap_err();
    assert_eq!(typed.code, "DISK_LOW");
    assert!(
        typed.message_zh.contains("剩余 1.5 GB"),
        "{}",
        typed.message_zh
    );

    // Space frees: the same legacy resume proceeds.
    test_support::set_default_probe_free_bytes(None);
    let resumed = service
        .update_download_task(&command(
            "legacy-task",
            legacy_revision,
            TaskStateCommandKind::ResumeTask,
            "legacy-resume-free",
        ))
        .expect("a resume with space proceeds");
    assert_ne!(
        status_in(&resumed, "legacy-task"),
        TaskLifecycleStatus::Paused
    );
    service.shutdown().unwrap();
}

fn paused_service(label: &str, task_id: &str) -> (Fixture, Arc<ApplicationService>, PathBuf) {
    let dir = std::env::temp_dir().join(format!("insar-disk-low-{label}-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let fixture = Fixture(dir);
    let output_root = fixture.0.join("out");
    let database = fixture.0.join("store.sqlite3");
    {
        let mut store = TaskStore::open(&database).unwrap();
        create_task(&mut store, task_id, &output_root);
        store
            .update_task(&command(
                task_id,
                0,
                TaskStateCommandKind::PauseTask,
                "pause",
            ))
            .unwrap();
    }
    let service = Arc::new(ApplicationService::new(
        ApplicationServiceConfig::business_store(&database),
    ));
    (fixture, service, output_root)
}

/// `resume_nisar_subset_task` measures before touching the task: a low drive
/// refuses with DISK_LOW and leaves it paused.
#[test]
fn nisar_subset_resume_is_refused_on_a_low_drive() {
    let _knob = KNOB.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let (_fixture, service, output_root) = paused_service("nisar-resume", "nisar-task");
    test_support::set_default_probe_free_bytes(Some(GIB));
    let error = service
        .resume_nisar_subset_task("nisar-task")
        .expect_err("a NISAR resume on a low drive is refused");
    assert_eq!(error.code(), "DISK_LOW");
    match &error {
        ApplicationServiceError::DiskLow { reason } => {
            assert_eq!(reason.free_bytes, GIB);
            assert_eq!(reason.drive, drive_label(&output_root));
        }
        other => panic!("unexpected error {other:?}"),
    }
    assert_eq!(
        revision(&service, "nisar-task").1,
        TaskLifecycleStatus::Paused
    );
    test_support::set_default_probe_free_bytes(None);
    // With space the check passes and the command's own validation answers
    // (this fixture is not a NISAR subset task).
    assert_ne!(
        service
            .resume_nisar_subset_task("nisar-task")
            .unwrap_err()
            .code(),
        "DISK_LOW"
    );
    service.shutdown().unwrap();
}

/// `start_newly_confirmed_task` measures before starting a confirmed task.
#[test]
fn newly_confirmed_task_start_is_refused_on_a_low_drive() {
    let _knob = KNOB.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let (_fixture, service, _output_root) = paused_service("nisar-start", "confirmed-task");
    test_support::set_default_probe_free_bytes(Some(GIB));
    let error = service
        .start_newly_confirmed_task("confirmed-task")
        .expect_err("starting a confirmed task on a low drive is refused");
    assert_eq!(error.code(), "DISK_LOW");
    assert_eq!(
        revision(&service, "confirmed-task").1,
        TaskLifecycleStatus::Paused
    );
    test_support::set_default_probe_free_bytes(None);
    assert_ne!(
        service
            .start_newly_confirmed_task("confirmed-task")
            .unwrap_err()
            .code(),
        "DISK_LOW"
    );
    service.shutdown().unwrap();
}

struct PlentyVolume;

impl insar_download::disk_guard::FreeSpaceProbe for PlentyVolume {
    fn volume_space(&self, _path: &Path) -> Option<insar_download::disk_guard::VolumeSpace> {
        Some(insar_download::disk_guard::VolumeSpace {
            free_bytes: 10 * GIB,
            total_bytes: 20 * GIB,
        })
    }
}

/// One measurement per resume, with the injected probe throughout: a unified
/// resume measured as plenty is not refused by a second, default-probe check
/// further down the same call.
#[test]
fn a_unified_resume_uses_only_the_injected_probe() {
    let _knob = KNOB.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let (_fixture, service, _output_root) = paused_service("injected", "injected-task");
    test_support::set_default_probe_free_bytes(Some(GIB));
    let (revision, _) = revision(&service, "injected-task");
    let resumed = service
        .task_action_with_free_space_probe(
            "injected-task".to_owned(),
            revision,
            UnifiedTaskAction::Resume,
            &PlentyVolume,
        )
        .expect("the injected probe's measurement decides");
    assert_ne!(resumed.status, TaskLifecycleStatus::Paused);
    test_support::set_default_probe_free_bytes(None);
    service.shutdown().unwrap();
}
