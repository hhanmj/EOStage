//! The service side of the engine's free-space pause (PM decision
//! 2026-10-26): the overview explains the pause and a resume re-checks the
//! drive first.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use insar_application_service::{ApplicationService, ApplicationServiceConfig, UnifiedTaskAction};
use insar_core::{
    DownloadLayout, FrozenSearchBatch, SearchWorkbenchState, SnapshotItem, StoreCreateTaskRequest,
    StoreSearchBatchRequest, StoreTaskCommand, StoreUpdateTaskRequest, TaskLifecycleStatus,
    TaskPauseReason, TaskPauseReasonKind, TaskStateCommandKind,
};
use insar_download::disk_guard::{drive_label, FreeSpaceProbe, VolumeSpace};
use insar_store::TaskStore;
use serde_json::Map;

const GIB: u64 = 1 << 30;
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "insar-disk-low-resume-{label}-{}-{id}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        Self { dir }
    }

    fn database(&self) -> PathBuf {
        self.dir.join("store.sqlite3")
    }

    fn output_root(&self) -> PathBuf {
        self.dir.join("out")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// A 20 GiB drive with a fixed free size.
struct Volume(u64);

impl FreeSpaceProbe for Volume {
    fn volume_space(&self, _path: &Path) -> Option<VolumeSpace> {
        Some(VolumeSpace {
            free_bytes: self.0,
            total_bytes: 20 * GIB,
        })
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

fn pause(store: &mut TaskStore, task_id: &str, reason: Option<&TaskPauseReason>) -> u64 {
    let request = StoreUpdateTaskRequest {
        task_id: task_id.to_owned(),
        expected_revision: store.load_task(task_id).unwrap().unwrap().runtime.revision,
        command: StoreTaskCommand {
            kind: TaskStateCommandKind::PauseTask,
            item_ids: vec![],
        },
        event_id: format!("{task_id}-pause"),
        occurred_at: "2026-10-26T00:02:00Z".to_owned(),
    };
    match reason {
        Some(reason) => store.pause_task_with_reason(&request, reason),
        None => store.update_task(&request),
    }
    .unwrap()
    .runtime
    .revision
}

fn disk_low(drive: Option<String>) -> TaskPauseReason {
    TaskPauseReason {
        kind: TaskPauseReasonKind::DiskLow,
        free_bytes: GIB,
        threshold_bytes: 2 * GIB,
        drive,
    }
}

#[test]
fn task_overview_exposes_the_engine_pause_reason_after_restart() {
    let fixture = Fixture::new("overview");
    let reason = disk_low(drive_label(&fixture.output_root()));
    {
        let mut store = TaskStore::open(fixture.database()).unwrap();
        create_task(&mut store, "disk-task", &fixture.output_root());
        create_task(&mut store, "user-task", &fixture.output_root());
        create_task(&mut store, "queued-task", &fixture.output_root());
        pause(&mut store, "disk-task", Some(&reason));
        pause(&mut store, "user-task", None);
    }
    let service = ApplicationService::new(ApplicationServiceConfig::store(fixture.database()));
    let overview = service.task_overview().unwrap();
    let find = |task_id: &str| {
        overview
            .queue
            .iter()
            .find(|task| task.task_id == task_id)
            .unwrap()
    };
    let disk = find("disk-task");
    assert_eq!(disk.status, TaskLifecycleStatus::Paused);
    assert_eq!(disk.pause_reason, Some(reason.clone()));
    assert_eq!(find("user-task").pause_reason, None);
    assert_eq!(find("queued-task").pause_reason, None);

    let wire = serde_json::to_value(&overview).unwrap();
    let wire_task = |task_id: &str| {
        wire["queue"]
            .as_array()
            .unwrap()
            .iter()
            .find(|task| task["task_id"] == task_id)
            .unwrap()
            .clone()
    };
    assert_eq!(
        wire_task("disk-task")["pause_reason"],
        serde_json::json!({
            "kind": "disk_low",
            "free_bytes": GIB,
            "threshold_bytes": 2 * GIB,
            "drive": reason.drive,
        })
    );
    assert!(wire_task("user-task")["pause_reason"].is_null());
    service.shutdown().unwrap();
}

#[test]
fn resume_is_refused_with_disk_low_while_the_drive_is_low_and_allowed_once_it_frees() {
    let fixture = Fixture::new("resume");
    let drive = drive_label(&fixture.output_root());
    let revision = {
        let mut store = TaskStore::open(fixture.database()).unwrap();
        create_task(&mut store, "disk-task", &fixture.output_root());
        pause(&mut store, "disk-task", Some(&disk_low(drive.clone())))
    };
    let service =
        ApplicationService::new(ApplicationServiceConfig::business_store(fixture.database()));

    let refused = service
        .task_action_with_free_space_probe(
            "disk-task".to_owned(),
            revision,
            UnifiedTaskAction::Resume,
            &Volume(GIB + GIB / 2),
        )
        .expect_err("a resume on a low drive is refused");
    assert_eq!(refused.code, "DISK_LOW");
    assert_eq!(
        refused.message_zh,
        format!(
            "磁盘剩余空间不足，无法继续下载：{} 剩余 1.5 GB",
            drive.as_deref().unwrap()
        )
    );
    assert!(refused.retryable);
    assert_eq!(refused.needs_account, None);
    let overview = service.task_overview().unwrap();
    let task = overview
        .queue
        .iter()
        .find(|task| task.task_id == "disk-task")
        .unwrap();
    assert_eq!(task.status, TaskLifecycleStatus::Paused, "nothing changed");
    assert_eq!(task.revision, revision);

    let resumed = service
        .task_action_with_free_space_probe(
            "disk-task".to_owned(),
            revision,
            UnifiedTaskAction::Resume,
            &Volume(10 * GIB),
        )
        .expect("a resume with enough space proceeds");
    assert_eq!(resumed.task_id, "disk-task");
    assert_eq!(resumed.revision, revision + 1);
    assert_ne!(resumed.status, TaskLifecycleStatus::Paused);
    service.shutdown().unwrap();
}

/// Counts measurements, so a resume can be shown to measure exactly once.
struct CountingVolume {
    free: u64,
    probes: std::sync::atomic::AtomicUsize,
}

impl FreeSpaceProbe for CountingVolume {
    fn volume_space(&self, _path: &Path) -> Option<VolumeSpace> {
        self.probes.fetch_add(1, Ordering::SeqCst);
        Some(VolumeSpace {
            free_bytes: self.free,
            total_bytes: 20 * GIB,
        })
    }
}

/// The engine paused while writing an 8 GiB file with 6 GiB free: it recorded
/// threshold 9 GiB (remaining + 1 GiB). 6 GiB is above the 2 GiB drive floor,
/// but a resume must meet the recorded threshold while it names the same
/// drive, measured once with the injected probe.
#[test]
fn resume_keeps_the_recorded_file_threshold_and_measures_once() {
    let fixture = Fixture::new("file-rule");
    let drive = drive_label(&fixture.output_root());
    let recorded = TaskPauseReason {
        kind: TaskPauseReasonKind::DiskLow,
        free_bytes: 6 * GIB,
        threshold_bytes: 9 * GIB,
        drive: drive.clone(),
    };
    let (revision, other_drive_revision) = {
        let mut store = TaskStore::open(fixture.database()).unwrap();
        create_task(&mut store, "file-task", &fixture.output_root());
        create_task(&mut store, "other-drive-task", &fixture.output_root());
        let revision = pause(&mut store, "file-task", Some(&recorded));
        let elsewhere = TaskPauseReason {
            drive: Some("Z:".to_owned()),
            ..recorded.clone()
        };
        (
            revision,
            pause(&mut store, "other-drive-task", Some(&elsewhere)),
        )
    };
    let service =
        ApplicationService::new(ApplicationServiceConfig::business_store(fixture.database()));

    let six = CountingVolume {
        free: 6 * GIB,
        probes: Default::default(),
    };
    let refused = service
        .task_action_with_free_space_probe(
            "file-task".to_owned(),
            revision,
            UnifiedTaskAction::Resume,
            &six,
        )
        .expect_err("6 GiB free cannot hold the recorded 9 GiB threshold");
    assert_eq!(refused.code, "DISK_LOW");
    assert_eq!(
        six.probes.load(Ordering::SeqCst),
        1,
        "one measurement per resume"
    );

    // A recorded threshold for another drive does not apply: the drive floor
    // (2 GiB) does, and 6 GiB passes it.
    let resumed_elsewhere = service
        .task_action_with_free_space_probe(
            "other-drive-task".to_owned(),
            other_drive_revision,
            UnifiedTaskAction::Resume,
            &six,
        )
        .expect("a reason recorded for another drive is not applied");
    assert_ne!(resumed_elsewhere.status, TaskLifecycleStatus::Paused);
    assert_eq!(six.probes.load(Ordering::SeqCst), 2);

    let ten = CountingVolume {
        free: 10 * GIB,
        probes: Default::default(),
    };
    let resumed = service
        .task_action_with_free_space_probe(
            "file-task".to_owned(),
            revision,
            UnifiedTaskAction::Resume,
            &ten,
        )
        .expect("10 GiB free meets the recorded threshold");
    assert_ne!(resumed.status, TaskLifecycleStatus::Paused);
    assert_eq!(
        ten.probes.load(Ordering::SeqCst),
        1,
        "one measurement per resume"
    );
    service.shutdown().unwrap();
}
