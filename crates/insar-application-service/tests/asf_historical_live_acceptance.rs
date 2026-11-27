use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use insar_application_service::{
    ApplicationSarSearchAndFreezeRequest, ApplicationService, ApplicationServiceConfig,
    ApplicationSetWorkingAoiRequest, SarSearchProgressUpdate,
};
use insar_core::{
    CreateTaskFromWorkbenchRequest, DownloadLayout, DownloadOutcome, FrozenSearchBatch,
    SearchWorkbenchState, StoreTaskCommand, StoreUpdateTaskRequest, StoreUpdateWorkbenchRequest,
    TaskLifecycleStatus, TaskStateCommandKind,
};
use insar_provider::boundary::{normalize_polygonal_geometry, DataVBoundaryProvider};
use serde_json::{json, Map, Value};

const LIVE_ROOT_ENV: &str = "INSAR_HISTORICAL_ASF_LIVE_ROOT";
const RESUME_ATTEMPT_ENV: &str = "INSAR_HISTORICAL_ASF_RESUME_ATTEMPT";

fn live_root(slug: &str) -> PathBuf {
    let root = std::env::var(LIVE_ROOT_ENV)
        .map(PathBuf::from)
        .expect("INSAR_HISTORICAL_ASF_LIVE_ROOT must point to an E: verification directory");
    assert!(root.is_absolute(), "live evidence root must be absolute");
    assert_eq!(
        root.components()
            .next()
            .map(|component| component.as_os_str()),
        Some(std::ffi::OsStr::new("E:")),
        "live evidence must remain on E:"
    );
    if let Ok(resume) = std::env::var(RESUME_ATTEMPT_ENV) {
        let attempt = PathBuf::from(resume);
        assert!(
            attempt.starts_with(&root),
            "resume attempt must remain under live evidence root"
        );
        assert!(
            attempt.is_dir(),
            "resume attempt directory must already exist"
        );
        return attempt;
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_millis();
    let attempt = root.join(format!("{slug}-{stamp}-{}", std::process::id()));
    fs::create_dir_all(&attempt).expect("live evidence directory");
    attempt
}

fn geometry_map(value: Value) -> Map<String, Value> {
    value
        .as_object()
        .cloned()
        .expect("authoritative boundary geometry is an object")
}

fn observed_download_bytes(service: &ApplicationService, task_id: &str) -> u64 {
    let download = service
        .download_snapshot(task_id)
        .expect("download snapshot");
    download
        .progress
        .iter()
        .map(|item| item.bytes_completed)
        .chain(download.results.iter().map(|item| item.bytes_written))
        .max()
        .unwrap_or(0)
}

fn observed_item_bytes(service: &ApplicationService, task_id: &str, item_id: &str) -> u64 {
    let download = service
        .download_snapshot(task_id)
        .expect("download item snapshot");
    download
        .progress
        .iter()
        .filter(|item| item.item_id == item_id)
        .map(|item| item.bytes_completed)
        .chain(
            download
                .results
                .iter()
                .filter(|item| item.item_id == item_id)
                .map(|item| item.bytes_written),
        )
        .max()
        .unwrap_or(0)
}

fn update_task_with_revision_retry(
    service: &ApplicationService,
    task_id: &str,
    kind: TaskStateCommandKind,
    item_ids: Vec<String>,
    event_prefix: &str,
) {
    for attempt in 0..200 {
        let latest_revision = task_runtime(service, task_id).revision;
        match service.update_download_task(&StoreUpdateTaskRequest {
            task_id: task_id.to_owned(),
            expected_revision: latest_revision,
            command: StoreTaskCommand {
                kind: kind.clone(),
                item_ids: item_ids.clone(),
            },
            event_id: format!("{task_id}-{event_prefix}-{attempt}"),
            occurred_at: "2026-08-24T00:00:03Z".to_owned(),
        }) {
            Ok(_) => return,
            Err(error) if error.code() == "V3_STORE_REVISION_CONFLICT" => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("{event_prefix} bounded live download failed: {error:?}"),
        }
    }
    panic!("{event_prefix} remained revision-conflicted");
}

fn task_runtime(service: &ApplicationService, task_id: &str) -> insar_core::TaskRuntimeSnapshot {
    service
        .snapshot(true)
        .expect("task lifecycle snapshot")
        .restore
        .runtimes
        .into_iter()
        .find(|runtime| runtime.task_id == task_id)
        .expect("live task runtime")
}

fn task_status(service: &ApplicationService, task_id: &str) -> TaskLifecycleStatus {
    task_runtime(service, task_id).status
}

fn prove_first_candidate_download(
    service: &ApplicationService,
    batch: &FrozenSearchBatch,
    output_root: PathBuf,
    task_id: &str,
    exercise_lifecycle: bool,
) -> (u64, Duration, Option<DownloadOutcome>) {
    fs::create_dir_all(&output_root).expect("download output directory");
    let selected_item_ids = batch
        .candidates
        .iter()
        .take(if exercise_lifecycle { 2 } else { 1 })
        .map(|candidate| candidate.item_id.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        selected_item_ids.len(),
        if exercise_lifecycle { 2 } else { 1 },
        "historical ASF batch has enough candidates for the requested acceptance"
    );
    let selected_item_id = selected_item_ids[0].clone();
    let current_workbench = service
        .snapshot(true)
        .expect("workbench snapshot before task creation")
        .restore
        .workbench
        .filter(|state| state.batch_id == batch.batch_id);
    let workbench_revision = if current_workbench
        .as_ref()
        .is_some_and(|state| state.checked_item_ids == selected_item_ids)
    {
        current_workbench.expect("matching workbench").revision
    } else {
        let expected_revision = current_workbench
            .as_ref()
            .map(|state| state.revision)
            .unwrap_or(0);
        let revision = expected_revision + 1;
        service
            .update_search_workbench(&StoreUpdateWorkbenchRequest {
                state: SearchWorkbenchState {
                    batch_id: batch.batch_id.clone(),
                    checked_item_ids: selected_item_ids.clone(),
                    page_index: 0,
                    page_size: 24,
                    revision,
                },
                expected_revision,
                updated_at: "2026-08-24T00:00:01Z".to_owned(),
            })
            .expect("select one historical ASF product");
        revision
    };
    service
        .create_task_from_workbench(&CreateTaskFromWorkbenchRequest {
            task_id: task_id.to_owned(),
            batch_id: batch.batch_id.clone(),
            expected_workbench_revision: workbench_revision,
            exact_item_id: if exercise_lifecycle {
                None
            } else {
                Some(selected_item_id.clone())
            },
            output_root,
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-08-24T00:00:02Z".to_owned(),
        })
        .expect("Store-owned historical ASF task starts through the Workbench seam");

    let download_started = Instant::now();
    let deadline = download_started + Duration::from_secs(90);
    let mut observed_bytes = 0_u64;
    while Instant::now() < deadline {
        let download = service
            .download_snapshot(task_id)
            .expect("download snapshot");
        observed_bytes = observed_bytes.max(
            download
                .progress
                .iter()
                .map(|item| item.bytes_completed)
                .max()
                .unwrap_or(0),
        );
        observed_bytes = observed_bytes.max(
            download
                .results
                .iter()
                .map(|item| item.bytes_written)
                .max()
                .unwrap_or(0),
        );
        if observed_bytes > 0 || !download.results.is_empty() {
            if observed_bytes == 0 {
                panic!(
                    "historical ASF download terminated before real bytes: {:?}",
                    download.results
                );
            }
            break;
        }
        thread::sleep(Duration::from_millis(250));
    }
    assert!(
        observed_bytes > 0,
        "no real historical ASF bytes arrived within 90 seconds"
    );

    if exercise_lifecycle {
        update_task_with_revision_retry(
            service,
            task_id,
            TaskStateCommandKind::PauseTask,
            vec![],
            "pause-after-bytes",
        );
        let pause_deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < pause_deadline {
            if task_status(service, task_id) == TaskLifecycleStatus::Paused {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(task_status(service, task_id), TaskLifecycleStatus::Paused);
        let paused_bytes = observed_download_bytes(service, task_id);
        update_task_with_revision_retry(
            service,
            task_id,
            TaskStateCommandKind::ResumeTask,
            vec![],
            "resume-after-pause",
        );
        let resume_deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < resume_deadline
            && observed_download_bytes(service, task_id) <= paused_bytes
        {
            thread::sleep(Duration::from_millis(100));
        }
        assert!(
            observed_download_bytes(service, task_id) > paused_bytes,
            "resumed live transfer did not advance bytes"
        );
    }

    update_task_with_revision_retry(
        service,
        task_id,
        TaskStateCommandKind::CancelItems,
        vec![selected_item_id.clone()],
        "cancel-after-bytes",
    );

    let cancel_deadline = Instant::now() + Duration::from_secs(30);
    let mut final_outcome = None;
    while Instant::now() < cancel_deadline {
        let download = service
            .download_snapshot(task_id)
            .expect("cancelled download snapshot");
        if let Some(result) = download.results.first() {
            final_outcome = Some(result.outcome.clone());
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    assert!(matches!(
        final_outcome,
        Some(DownloadOutcome::Cancelled | DownloadOutcome::Completed)
    ));

    if exercise_lifecycle && final_outcome == Some(DownloadOutcome::Cancelled) {
        let cancelled_bytes = observed_item_bytes(service, task_id, &selected_item_id);
        update_task_with_revision_retry(
            service,
            task_id,
            TaskStateCommandKind::RetryItems,
            vec![selected_item_id.clone()],
            "retry-cancelled-item",
        );
        update_task_with_revision_retry(
            service,
            task_id,
            TaskStateCommandKind::CancelItems,
            vec![selected_item_ids[1].clone()],
            "cancel-companion-before-retry",
        );
        let retry_deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < retry_deadline
            && observed_item_bytes(service, task_id, &selected_item_id) <= cancelled_bytes
        {
            thread::sleep(Duration::from_millis(100));
        }
        assert!(
            observed_item_bytes(service, task_id, &selected_item_id) > cancelled_bytes,
            "retried live transfer did not advance bytes"
        );
        update_task_with_revision_retry(
            service,
            task_id,
            TaskStateCommandKind::CancelItems,
            vec![selected_item_id],
            "cancel-after-retry",
        );
    }
    (observed_bytes, download_started.elapsed(), final_outcome)
}

struct GlobalHistoricalCanary {
    slug: &'static str,
    dataset: &'static str,
    platform: &'static str,
    processing_levels: &'static [&'static str],
    polarizations: &'static [&'static str],
    start: &'static str,
    end: &'static str,
    exercise_lifecycle: bool,
}

fn run_global_historical_canary(case: GlobalHistoricalCanary) {
    let attempt = live_root(case.slug);
    let store_path = attempt.join("tasks.sqlite3");
    let output_root = attempt.join("downloads");
    let service = ApplicationService::new(ApplicationServiceConfig::business_store(&store_path));
    let request_id = format!("{}-live-search", case.slug);
    let batch_id = format!("{}-live-batch", case.slug);
    let task_id = format!("{}-live-download", case.slug);
    let search_started = Instant::now();
    let request: ApplicationSarSearchAndFreezeRequest = serde_json::from_value(json!({
        "request_id": request_id,
        "search": {
            "query": {
                "provider": "asf",
                "dataset": case.dataset,
                "platforms": [case.platform],
                "processing_levels": case.processing_levels,
                "polarizations": case.polarizations,
                "start": case.start,
                "end": case.end,
                "max_results": 3
            }
        },
        "batch_id": batch_id,
        "created_at": "2026-08-24T00:00:00Z",
        "aoi_name": "未限定 AOI",
        "aoi_geojson": null,
        "page_size": 24
    }))
    .expect("historical ASF ApplicationService request");
    let progress = Mutex::new(Vec::<SarSearchProgressUpdate>::new());
    service
        .sar_search_and_freeze_with_progress(&request, |update| {
            progress.lock().expect("progress lock").push(update);
        })
        .expect("real historical ASF search freezes through ApplicationService");
    assert!(progress
        .lock()
        .expect("progress lock")
        .last()
        .is_some_and(|update| update.terminal));

    let snapshot = service
        .snapshot(true)
        .expect("frozen historical ASF batch snapshot");
    let batch = snapshot
        .restore
        .workbench_batch
        .expect("historical ASF SearchBatch is Store-owned");
    assert_eq!(batch.provider_id, "asf.catalog");
    assert_eq!(batch.aoi_geojson, None);
    assert!(!batch.candidates.is_empty());
    assert!(batch.candidates.len() <= 3);
    assert!(batch.candidates.iter().all(|candidate| {
        candidate.provider_id == "asf.catalog"
            && candidate.metadata["dataset"] == case.dataset
            && candidate.metadata["platform"] == case.platform
            && candidate.payload["download_url"].is_string()
            && candidate.payload["file_name"].is_string()
            && candidate.payload["file_size_remote"].is_u64()
    }));
    let (observed_bytes, download_elapsed, final_outcome) = prove_first_candidate_download(
        &service,
        &batch,
        output_root,
        &task_id,
        case.exercise_lifecycle,
    );
    eprintln!(
        "HISTORICAL_ASF_APPLICATION_CANARY dataset={} evidence={} search_ms={} candidates={} download_ms={} observed_bytes={} outcome={:?} lifecycle={}",
        case.dataset,
        attempt.display(),
        search_started.elapsed().as_millis(),
        batch.candidates.len(),
        download_elapsed.as_millis(),
        observed_bytes,
        final_outcome,
        case.exercise_lifecycle,
    );
    service.shutdown().expect("ApplicationService shutdown");
}

#[test]
#[ignore = "explicit live ApplicationService/Store/ASF/production-download ALOS PALSAR canary"]
fn hubei_alos_palsar_freezes_real_rows_and_starts_real_download_bytes() {
    let attempt = live_root("alos-palsar");
    let store_path = attempt.join("tasks.sqlite3");
    let output_root = attempt.join("downloads");
    fs::create_dir_all(&output_root).expect("download output directory");
    let resuming = store_path.is_file();
    let service = ApplicationService::new(if resuming {
        ApplicationServiceConfig::existing_business_store(&store_path)
    } else {
        ApplicationServiceConfig::business_store(&store_path)
    });
    let search_started = Instant::now();
    if !resuming {
        let hubei = DataVBoundaryProvider::production()
            .expect("DataV provider initializes")
            .resolve_exact_adcode("420000")
            .expect("Hubei exact DataV polygon resolves");
        let geometry = geometry_map(hubei.geometry);
        service
            .set_working_aoi(&ApplicationSetWorkingAoiRequest {
                source_kind: "administrative_boundary".to_owned(),
                geometry,
                bbox: hubei.bbox.to_vec(),
                crs: "EPSG:4326".to_owned(),
                display_name: "湖北省".to_owned(),
            })
            .expect("Hubei becomes the Store-owned working AOI");

        let request: ApplicationSarSearchAndFreezeRequest = serde_json::from_value(json!({
            "request_id": "alos-palsar-live-search",
            "search": {
                "query": {
                    "provider": "asf",
                    "dataset": "ALOS PALSAR",
                    "platforms": ["ALOS"],
                    "instrument": "PALSAR",
                    "processing_levels": ["L1.1"],
                    "start": "2007-01-01T00:00:00Z",
                    "end": "2007-12-31T23:59:59Z",
                    "max_results": 3
                }
            },
            "batch_id": "alos-palsar-live-batch",
            "created_at": "2026-08-24T00:00:00Z",
            "aoi_name": "由工作 AOI 解析",
            "aoi_geojson": null,
            "page_size": 24
        }))
        .expect("ALOS PALSAR ApplicationService request");
        let progress = Mutex::new(Vec::<SarSearchProgressUpdate>::new());
        service
            .sar_search_and_freeze_with_progress(&request, |update| {
                progress.lock().expect("progress lock").push(update);
            })
            .expect("real ALOS PALSAR search freezes through ApplicationService");
        assert!(progress
            .lock()
            .expect("progress lock")
            .last()
            .is_some_and(|update| update.terminal));
    }
    let search_elapsed_ms = if resuming {
        0
    } else {
        search_started.elapsed().as_millis()
    };

    let snapshot = service.snapshot(true).expect("frozen batch snapshot");
    let batch = snapshot
        .restore
        .workbench_batch
        .expect("ALOS PALSAR SearchBatch is Store-owned");
    let working_geometry = service
        .working_aoi()
        .expect("working AOI load")
        .expect("working AOI remains Store-owned")
        .geometry;
    assert_eq!(batch.aoi_name, "湖北省");
    let batch_geometry = batch
        .aoi_geojson
        .as_ref()
        .expect("frozen batch keeps the real AOI geometry");
    assert_eq!(
        batch.query.get("aoi_geojson"),
        Some(&Value::Object(batch_geometry.clone()))
    );
    let working_topology = normalize_polygonal_geometry(Value::Object(working_geometry))
        .expect("restored working AOI topology remains valid");
    let batch_topology = normalize_polygonal_geometry(Value::Object(batch_geometry.clone()))
        .expect("frozen search AOI topology remains valid");
    assert_eq!(working_topology.geometry_kind, batch_topology.geometry_kind);
    assert!(working_topology
        .bbox
        .iter()
        .zip(batch_topology.bbox)
        .all(|(working, frozen)| (working - frozen).abs() <= 1e-9));
    assert!(!batch.candidates.is_empty());
    assert!(batch.candidates.len() <= 3);
    assert!(batch
        .candidates
        .iter()
        .all(|candidate| candidate.footprint_geojson.is_some()));
    assert!(batch
        .candidates
        .iter()
        .all(|candidate| candidate.metadata["dataset"] == "ALOS PALSAR"));
    assert!(batch
        .candidates
        .iter()
        .all(|candidate| candidate.payload["download_url"].is_string()));
    let live_task_id = if resuming {
        format!(
            "alos-palsar-live-download-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time")
                .as_millis()
        )
    } else {
        "alos-palsar-live-download".to_owned()
    };
    let (observed_bytes, download_elapsed, final_outcome) =
        prove_first_candidate_download(&service, &batch, output_root, &live_task_id, true);
    eprintln!(
        "ALOS_PALSAR_APPLICATION_CANARY evidence={} resumed={} search_ms={} candidates={} download_ms={} observed_bytes={} outcome={:?}",
        attempt.display(),
        resuming,
        search_elapsed_ms,
        batch.candidates.len(),
        download_elapsed.as_millis(),
        observed_bytes,
        final_outcome,
    );
    service.shutdown().expect("ApplicationService shutdown");
}

#[test]
#[ignore = "explicit live ApplicationService/Store/ASF/production-download ALOS PALSAR global lifecycle canary"]
fn global_alos_palsar_proves_pause_resume_cancel_and_retry() {
    run_global_historical_canary(GlobalHistoricalCanary {
        slug: "alos-palsar-lifecycle",
        dataset: "ALOS PALSAR",
        platform: "ALOS",
        processing_levels: &["L1.1"],
        polarizations: &[],
        start: "2007-01-01T00:00:00Z",
        end: "2007-12-31T23:59:59Z",
        exercise_lifecycle: true,
    });
}

#[test]
#[ignore = "explicit live ApplicationService/Store/ASF/production-download ERS-1 canary"]
fn global_ers1_freezes_real_rows_and_starts_real_download_bytes() {
    run_global_historical_canary(GlobalHistoricalCanary {
        slug: "ers-1",
        dataset: "ERS",
        platform: "ERS-1",
        processing_levels: &["L1"],
        polarizations: &["VV"],
        start: "1991-01-01T00:00:00Z",
        end: "2011-12-31T23:59:59Z",
        exercise_lifecycle: false,
    });
}

#[test]
#[ignore = "explicit live ApplicationService/Store/ASF/production-download JERS-1 canary"]
fn global_jers1_freezes_real_rows_and_starts_real_download_bytes() {
    run_global_historical_canary(GlobalHistoricalCanary {
        slug: "jers-1",
        dataset: "JERS-1",
        platform: "JERS-1",
        processing_levels: &["L1"],
        polarizations: &["HH"],
        start: "1992-02-11T00:00:00Z",
        end: "1998-10-12T23:59:59Z",
        exercise_lifecycle: false,
    });
}

#[test]
#[ignore = "explicit live ApplicationService/Store/ASF/production-download RADARSAT-1 canary"]
fn global_radarsat1_freezes_real_rows_and_starts_real_download_bytes() {
    run_global_historical_canary(GlobalHistoricalCanary {
        slug: "radarsat-1",
        dataset: "RADARSAT-1",
        platform: "RADARSAT-1",
        processing_levels: &["L1"],
        polarizations: &["HH"],
        start: "1995-11-04T00:00:00Z",
        end: "2013-03-29T23:59:59Z",
        exercise_lifecycle: false,
    });
}

#[test]
#[ignore = "explicit live ApplicationService/Store/ASF/production-download SEASAT canary"]
fn global_seasat_freezes_real_rows_and_starts_real_download_bytes() {
    run_global_historical_canary(GlobalHistoricalCanary {
        slug: "seasat",
        dataset: "SEASAT",
        platform: "SEASAT 1",
        processing_levels: &[],
        polarizations: &["HH"],
        start: "1978-06-26T00:00:00Z",
        end: "1978-10-10T23:59:59Z",
        exercise_lifecycle: false,
    });
}
