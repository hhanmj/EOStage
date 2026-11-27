//! Opt-in current ApplicationService/Store/Worker acceptance. No fixture ports.
//! Each stage is a new process; credentials stay in the native OS-keyring path.
//! Cleanup is a separate, explicit stage and retains the small evidence journal.
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

use insar_application_service::{
    ApplicationCatalogSearchAndFreezeRequest, ApplicationService, ApplicationServiceConfig,
    ApplicationSetWorkingAoiRequest, LandsatCreateAssetBatchTaskRequest, UsgsM2mLoginRequest,
};
use insar_core::{
    DownloadLayout, DownloadOutcome, StoreTaskCommand, StoreUpdateTaskRequest, TaskStateCommandKind,
};
use insar_download::{part_path, resolve_desktop_usgs_login};
use insar_store::TaskStore;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const OWNER_MARKER: &str = "ScenePort explicit isolated Landsat live acceptance v1";
const TASK: &str = "landsat-current-live-batch";
const BATCH: &str = "landsat-current-live-source";
const MAX_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const PAUSE_BYTES: u64 = 32 * 1024 * 1024;
type Check<T = ()> = Result<T, String>;

// Match the desktop's Date.toISOString UTC wire representation, not +00:00.
fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn require(ok: bool, code: &str) -> Check {
    if ok {
        Ok(())
    } else {
        Err(code.to_owned())
    }
}

fn journal(root: &Path, phase: &str, details: Value) -> Check {
    let event = json!({"at":now(), "phase":phase, "details":details});
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("events.jsonl"))
        .map_err(|_| "LIVE_EVIDENCE_WRITE")?;
    writeln!(file, "{event}").map_err(|_| "LIVE_EVIDENCE_WRITE")?;
    file.sync_data().map_err(|_| "LIVE_EVIDENCE_SYNC")?;
    println!("LANDSAT_LIVE {event}");
    Ok(())
}

fn record_new(path: &Path, value: &Value) -> Check {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|_| "LIVE_RECORD_ALREADY_EXISTS_OR_UNAVAILABLE")?;
    file.write_all(value.to_string().as_bytes())
        .map_err(|_| "LIVE_EVIDENCE_WRITE")?;
    file.sync_all().map_err(|_| "LIVE_EVIDENCE_SYNC")?;
    Ok(())
}

fn read_json(path: &Path) -> Check<Value> {
    serde_json::from_slice(&fs::read(path).map_err(|_| "LIVE_RECORD_MISSING")?)
        .map_err(|_| "LIVE_RECORD_INVALID".to_owned())
}

fn plain_path(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| {
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            !m.file_type().is_symlink() && m.file_attributes() & 0x400 == 0
        }
        #[cfg(not(windows))]
        {
            !m.file_type().is_symlink()
        }
    })
}

fn hash_prefix(path: &Path, limit: u64) -> Check<String> {
    let mut file = File::open(path).map_err(|_| "LIVE_FILE_OPEN")?.take(limit);
    let mut hash = Sha256::new();
    let mut buffer = vec![0; 1024 * 1024];
    let mut read = 0_u64;
    loop {
        let n = file.read(&mut buffer).map_err(|_| "LIVE_FILE_READ")?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
        read += n as u64;
    }
    require(read == limit, "LIVE_FILE_LENGTH_CHANGED")?;
    Ok(format!("{:x}", hash.finalize()))
}

fn command(service: &ApplicationService, database: &Path, kind: TaskStateCommandKind) -> Check {
    let store = TaskStore::open(database).map_err(|_| "LIVE_STORE_OPEN")?;
    let task = store
        .load_task_record(TASK)
        .map_err(|_| "LIVE_TASK_READ")?
        .ok_or("LIVE_TASK_MISSING")?;
    service
        .update_download_task(&StoreUpdateTaskRequest {
            task_id: TASK.into(),
            expected_revision: task.runtime.revision,
            command: StoreTaskCommand {
                kind,
                item_ids: vec![],
            },
            event_id: format!("live-control-{}", uuid::Uuid::new_v4()),
            occurred_at: now(),
        })
        .map_err(|e| e.code().to_owned())?;
    Ok(())
}

fn login(service: &ApplicationService, root: &Path) -> Check {
    let started = Instant::now();
    let (mut username, mut application_token) =
        resolve_desktop_usgs_login().map_err(|_| "LIVE_SAVED_USGS_CREDENTIAL_UNAVAILABLE")?;
    let result = service.usgs_m2m_login(UsgsM2mLoginRequest {
        username: std::mem::take(&mut *username),
        application_token: std::mem::take(&mut *application_token),
    });
    journal(
        root,
        "login",
        json!({"elapsed_ms":started.elapsed().as_millis(),
        "authenticated":result.as_ref().is_ok_and(|v| v.authenticated),
        "code":result.as_ref().err().map(|e| e.code())}),
    )?;
    require(
        result.map_err(|e| e.code().to_owned())?.authenticated,
        "LIVE_LOGIN_NOT_AUTHENTICATED",
    )
}

fn create(service: &ApplicationService, root: &Path, archive: bool, logged_in: &mut bool) -> Check {
    let bbox = [-78.2, 35.5, -77.8, 35.9];
    let geometry = json!({"type":"Polygon","coordinates":[[[bbox[0],bbox[1]],[bbox[2],bbox[1]],
        [bbox[2],bbox[3]],[bbox[0],bbox[3]],[bbox[0],bbox[1]]]]})
    .as_object()
    .unwrap()
    .clone();
    service
        .set_working_aoi(&ApplicationSetWorkingAoiRequest {
            source_kind: "manual_bbox".into(),
            geometry: geometry.clone(),
            bbox: bbox.to_vec(),
            crs: "EPSG:4326".into(),
            display_name: "Landsat live acceptance AOI".into(),
        })
        .map_err(|e| e.code().to_owned())?;
    let started = Instant::now();
    let search = service.catalog_search_and_freeze(&ApplicationCatalogSearchAndFreezeRequest {
        request_id: "landsat-live-search".into(),
        batch_id: BATCH.into(),
        created_at: now(),
        aoi_name: "Landsat live acceptance AOI".into(),
        aoi_geojson: Some(geometry.clone()),
        workbench_page_size: 250,
        search: serde_json::from_value(json!({"logical_dataset":"optical.landsat.c2.l2",
            "provider_id":"microsoft.planetary-computer","native_collection_id":"landsat-c2-l2",
            "aoi_geojson":geometry,"cursor":null,"result_limit":2,
            "filters":{"spacecraft":["landsat_8","landsat_9"],"processing_levels":["l2sp"],
                "datetime":{"start":"2023-03-01T00:00:00Z","end":"2023-04-01T00:00:00Z"}}}))
        .map_err(|_| "LIVE_QUERY_INVALID")?,
    });
    journal(
        root,
        "search",
        json!({"elapsed_ms":started.elapsed().as_millis(),"ok":search.is_ok(),
        "code":search.as_ref().err().map(|e| e.code())}),
    )?;
    let source = search
        .map_err(|e| e.code().to_owned())?
        .core
        .workbench
        .ok_or("LIVE_SOURCE_MISSING")?;
    require(source.candidate_count == 2, "LIVE_TWO_SCENES_REQUIRED")?;
    let database = root.join("data/tasks.sqlite3");
    let frozen = TaskStore::open(&database)
        .map_err(|_| "LIVE_STORE_OPEN")?
        .load_workbench()
        .map_err(|_| "LIVE_SOURCE_READ")?
        .ok_or("LIVE_SOURCE_MISSING")?
        .batch;
    record_new(
        &root.join("source.json"),
        &json!({"batch_id":frozen.batch_id,
        "aoi_sha256":format!("{:x}",Sha256::digest(serde_json::to_vec(&frozen.aoi_geojson).unwrap())),
        "items":frozen.candidates.iter().map(|i| json!({"id":i.item_id,"product":i.display_name})).collect::<Vec<_>>()}),
    )?;
    if archive {
        login(service, root)?;
        *logged_in = true;
    }
    let request = LandsatCreateAssetBatchTaskRequest {
        request_id: "landsat-live-create".into(),
        source_batch_id: source.batch_id,
        expected_workbench_revision: source.revision,
        item_ids: frozen
            .candidates
            .iter()
            .map(|i| i.item_id.clone())
            .collect(),
        asset_keys: Vec::new(),
        task_id: TASK.into(),
        output_root: root.join("data/output"),
        layout: DownloadLayout::Direct,
        concurrency: 1,
        created_at: now(),
    };
    let started = Instant::now();
    let created = if archive {
        service.create_landsat_bundle_batch_task(&request)
    } else {
        service.create_landsat_asset_batch_task(&request)
    };
    journal(
        root,
        "admission",
        json!({"elapsed_ms":started.elapsed().as_millis(),"ok":created.is_ok(),
        "code":created.as_ref().err().map(|e| e.code()),"report":created.as_ref().ok().map(|v| &v.admission)}),
    )?;
    let created = created.map_err(|e| e.code().to_owned())?;
    require(
        created.admission.admitted_count == 2 && created.admission.rejected_count == 0,
        "LIVE_BATCH_NOT_FULLY_ADMITTED",
    )?;
    let snapshot = created.snapshot.ok_or("LIVE_TASK_MISSING")?;
    require(snapshot.core.tasks.len() == 1, "LIVE_TASK_COUNT_MISMATCH")?;
    let task = TaskStore::open(&database)
        .map_err(|_| "LIVE_STORE_OPEN")?
        .load_task_record(TASK)
        .map_err(|_| "LIVE_TASK_READ")?
        .ok_or("LIVE_TASK_MISSING")?;
    let total = task
        .snapshot
        .candidates
        .iter()
        .try_fold(0_u64, |sum, item| {
            item.asset_access
                .as_ref()
                .and_then(|a| a.expected_size_bytes)
                .and_then(|n| sum.checked_add(n))
        })
        .ok_or("LIVE_EXPECTED_SIZE_REQUIRED")?;
    require(total > 0 && total <= MAX_BYTES, "LIVE_SIZE_BUDGET_EXCEEDED")?;
    let assets=task.snapshot.candidates.iter().map(|i| {
        let a=i.asset_access.as_ref().unwrap();json!({"id":i.item_id,"file_name":a.file_name,
            "expected_size":a.expected_size_bytes,"checksum_algorithm":a.checksum_algorithm,"checksum_value":a.checksum_value})
    }).collect::<Vec<_>>();
    record_new(
        &root.join("assets.json"),
        &json!({"assets":assets,"expected_bytes":total,"archive":archive}),
    )?;
    journal(
        root,
        "created",
        json!({"task_count":1,"scene_count":2,"file_count":task.snapshot.candidates.len(),"expected_bytes":total}),
    )
}

fn frozen_files(root: &Path) -> Check<Vec<(PathBuf, u64)>> {
    let record = read_json(&root.join("assets.json"))?;
    record["assets"]
        .as_array()
        .ok_or("LIVE_ASSETS_INVALID")?
        .iter()
        .map(|a| {
            let name = a["file_name"].as_str().ok_or("LIVE_ASSET_NAME_INVALID")?;
            require(
                !name.is_empty()
                    && Path::new(name).components().count() == 1
                    && !name.contains(['/', '\\', ':']),
                "LIVE_ASSET_NAME_INVALID",
            )?;
            Ok((
                root.join("data/output").join(name),
                a["expected_size"].as_u64().ok_or("LIVE_SIZE_INVALID")?,
            ))
        })
        .collect()
}

fn capture_preserved_pause(root: &Path) -> Check<Value> {
    require(
        read_json(&root.join("prepare-result.json"))?["service_closed"] == true,
        "LIVE_PREVIOUS_SERVICE_MUST_BE_CLOSED",
    )?;
    // Normal restart must consume a valid WAL, not require the sidecar-free
    // historical snapshot API. Never remove a WAL to satisfy a test precondition.
    let store = TaskStore::open(root.join("data/tasks.sqlite3"))
        .map_err(|_| "LIVE_PAUSED_STORE_REOPEN_FAILED")?;
    let task = store
        .load_task_record(TASK)
        .map_err(|_| "LIVE_TASK_READ")?
        .ok_or("LIVE_TASK_MISSING")?;
    require(task.runtime.task_paused, "LIVE_PREVIOUS_TASK_NOT_PAUSED")?;
    let files = frozen_files(root)?;
    let (path, bytes) = files
        .iter()
        .filter_map(|(p, size)| {
            fs::metadata(part_path(p))
                .ok()
                .filter(|m| m.len() >= PAUSE_BYTES && m.len() < *size)
                .map(|m| (p, m.len()))
        })
        .next()
        .ok_or("LIVE_PRESERVED_PART_REQUIRED")?;
    let saved = json!({"file_name":path.file_name().unwrap().to_string_lossy(),
        "bytes":bytes,"sha256":hash_prefix(&part_path(path),bytes)?});
    record_new(&root.join("recheck-before.json"), &saved)?;
    journal(
        root,
        "preserved_partial",
        json!({"bytes":bytes,"task_reused":true,"prior_failure_retained":true}),
    )?;
    Ok(saved)
}

fn wait_download(
    service: &ApplicationService,
    root: &Path,
    prepare: bool,
    baseline: Option<Value>,
) -> Check {
    let database = root.join("data/tasks.sqlite3");
    let files = frozen_files(root)?;
    let start = Instant::now();
    let mut last = Instant::now();
    let pause_after = baseline
        .as_ref()
        .and_then(|v| v["bytes"].as_u64())
        .map_or(PAUSE_BYTES, |bytes| bytes + 16 * 1024 * 1024);
    let saved = if prepare {
        baseline
    } else {
        Some(read_json(&root.join("paused.json"))?)
    };
    loop {
        let snap = service
            .download_snapshot(TASK)
            .map_err(|e| e.code().to_owned())?;
        if let Some(failed) = snap
            .results
            .iter()
            .find(|r| r.outcome == DownloadOutcome::Failed)
        {
            journal(
                root,
                "transfer_failed",
                json!({"item_id":failed.item_id,"bytes":failed.bytes_written,
                "code":failed.failure.as_ref().map(|e| &e.code)}),
            )?;
            return Err("LIVE_TRANSFER_FAILED".into());
        }
        let partials = files
            .iter()
            .filter_map(|(p, _)| {
                fs::metadata(part_path(p))
                    .ok()
                    .map(|m| (p.clone(), m.len()))
            })
            .collect::<Vec<_>>();
        if let Some(saved) = &saved {
            let name = saved["file_name"].as_str().ok_or("LIVE_PAUSED_INVALID")?;
            let (path, size) = files
                .iter()
                .find(|(p, _)| p.file_name().and_then(|n| n.to_str()) == Some(name))
                .ok_or("LIVE_PAUSED_INVALID")?;
            let current = fs::metadata(part_path(path))
                .or_else(|_| fs::metadata(path))
                .map_err(|_| "LIVE_PART_DISAPPEARED")?
                .len();
            require(
                current >= saved["bytes"].as_u64().ok_or("LIVE_PAUSED_INVALID")?
                    && current <= *size,
                "LIVE_PART_TRUNCATED_OR_OVERSIZED",
            )?;
        }
        if prepare && partials.iter().any(|(_, n)| *n >= pause_after) {
            let pause_started = Instant::now();
            command(service, &database, TaskStateCommandKind::PauseTask)?;
            let deadline = Instant::now() + Duration::from_secs(60);
            loop {
                let task = TaskStore::open(&database)
                    .map_err(|_| "LIVE_STORE_OPEN")?
                    .load_task_record(TASK)
                    .map_err(|_| "LIVE_TASK_READ")?
                    .ok_or("LIVE_TASK_MISSING")?;
                // Ordinary HTTP transfers retain the paused Worker and its destination
                // lease. Unlike the SSAR subset path, pause does not return Interrupted
                // until shutdown. Require the real paused progress, not zero live owners.
                let progress = service
                    .download_snapshot(TASK)
                    .map_err(|e| e.code().to_owned())?;
                if task.runtime.task_paused
                    && progress.progress.iter().any(|p| {
                        p.status == insar_core::DownloadItemStatus::Paused
                            && p.bytes_completed >= pause_after
                    })
                {
                    break;
                }
                require(Instant::now() < deadline, "LIVE_PAUSE_NOT_SETTLED")?;
                thread::sleep(Duration::from_millis(100));
            }
            let (path, bytes) = files
                .iter()
                .filter_map(|(p, _)| fs::metadata(part_path(p)).ok().map(|m| (p, m.len())))
                .find(|(_, n)| *n >= pause_after)
                .ok_or("LIVE_PART_NOT_RETAINED")?;
            let prefix = hash_prefix(&part_path(path), bytes)?;
            thread::sleep(Duration::from_secs(2));
            require(
                fs::metadata(part_path(path))
                    .map_err(|_| "LIVE_PART_MISSING")?
                    .len()
                    == bytes,
                "LIVE_PAUSE_FILE_STILL_CHANGING",
            )?;
            record_new(
                &root.join("paused.json"),
                &json!({"file_name":path.file_name().unwrap().to_string_lossy(),"bytes":bytes,"sha256":prefix}),
            )?;
            if let Some(before) = &saved {
                require(
                    hash_prefix(
                        &part_path(path),
                        before["bytes"].as_u64().ok_or("LIVE_PAUSED_INVALID")?,
                    )? == before["sha256"],
                    "LIVE_RESUMED_PREFIX_CHANGED",
                )?;
            }
            return journal(
                root,
                "paused",
                json!({"bytes":bytes,"elapsed_ms":start.elapsed().as_millis(),
                "pause_confirmation_and_2s_stability_ms":pause_started.elapsed().as_millis(),"stable":true}),
            );
        }
        if snap
            .results
            .iter()
            .filter(|r| r.outcome == DownloadOutcome::Completed)
            .count()
            == files.len()
        {
            require(!prepare, "LIVE_COMPLETED_BEFORE_PAUSE_EVIDENCE")?;
            let saved = saved.as_ref().ok_or("LIVE_PAUSED_INVALID")?;
            let p = root
                .join("data/output")
                .join(saved["file_name"].as_str().ok_or("LIVE_PAUSED_INVALID")?);
            require(
                hash_prefix(&p, saved["bytes"].as_u64().ok_or("LIVE_PAUSED_INVALID")?)?
                    == saved["sha256"],
                "LIVE_RESUMED_PREFIX_CHANGED",
            )?;
            for (path, size) in &files {
                require(
                    !part_path(path).exists()
                        && fs::metadata(path).map_err(|_| "LIVE_OUTPUT_MISSING")?.len() == *size,
                    "LIVE_OUTPUT_SIZE_OR_PART_INVALID",
                )?;
                let hash = hash_prefix(path, *size)?;
                journal(
                    root,
                    "file_verified",
                    json!({"file_name":path.file_name().unwrap().to_string_lossy(),"bytes":size,"sha256":hash}),
                )?;
            }
            return journal(
                root,
                "complete",
                json!({"files":files.len(),"elapsed_ms":start.elapsed().as_millis(),"resumed_prefix_verified":true}),
            );
        }
        if last.elapsed() > Duration::from_secs(10) {
            journal(
                root,
                "transfer",
                json!({"elapsed_ms":start.elapsed().as_millis(),"progress":snap.progress.iter()
                .map(|p|json!({"bytes":p.bytes_completed,"total":p.total_bytes,"status":p.status})).collect::<Vec<_>>()}),
            )?;
            last = Instant::now();
        }
        require(
            start.elapsed() < Duration::from_secs(if prepare { 240 } else { 1200 }),
            "LIVE_TRANSFER_WAIT_EXCEEDED",
        )?;
        thread::sleep(Duration::from_millis(100));
    }
}

/// An independent full-delivery observation after a failed resume. This never
/// satisfies the stronger wait_download restart/prefix assertions above.
fn finish_delivery(service: &ApplicationService, root: &Path) -> Check {
    require(
        read_json(&root.join("owner.json"))?["delivery"] == "assets",
        "LIVE_FINISH_ANONYMOUS_ONLY",
    )?;
    let files = frozen_files(root)?;
    command(
        service,
        &root.join("data/tasks.sqlite3"),
        TaskStateCommandKind::ResumeTask,
    )?;
    journal(
        root,
        "full_delivery_observation",
        json!({"restart_resume_accepted":false,"task_reused":true}),
    )?;
    let started = Instant::now();
    let mut last_report = Instant::now();
    loop {
        let snapshot = service
            .download_snapshot(TASK)
            .map_err(|e| e.code().to_owned())?;
        if let Some(failed) = snapshot
            .results
            .iter()
            .find(|r| r.outcome == DownloadOutcome::Failed)
        {
            journal(
                root,
                "transfer_failed",
                json!({"item_id":failed.item_id,"bytes":failed.bytes_written,
                "code":failed.failure.as_ref().map(|e| &e.code)}),
            )?;
            return Err("LIVE_TRANSFER_FAILED".into());
        }
        let completed = snapshot
            .results
            .iter()
            .filter(|r| r.outcome == DownloadOutcome::Completed)
            .count();
        if completed == files.len() {
            for (path, size) in &files {
                require(
                    !part_path(path).exists()
                        && fs::metadata(path).map_err(|_| "LIVE_OUTPUT_MISSING")?.len() == *size,
                    "LIVE_OUTPUT_SIZE_OR_PART_INVALID",
                )?;
                journal(
                    root,
                    "file_verified",
                    json!({"file_name":path.file_name().unwrap().to_string_lossy(),
                    "bytes":size,"sha256":hash_prefix(path,*size)?}),
                )?;
            }
            return journal(
                root,
                "full_delivery_complete",
                json!({"files":files.len(),
                "elapsed_ms":started.elapsed().as_millis(),"restart_resume_accepted":false}),
            );
        }
        if last_report.elapsed() >= Duration::from_secs(15) {
            journal(
                root,
                "full_delivery_progress",
                json!({"completed_files":completed,
                "progress_bytes":snapshot.progress.iter().map(|p|p.bytes_completed).sum::<u64>(),
                "elapsed_ms":started.elapsed().as_millis()}),
            )?;
            last_report = Instant::now();
        }
        require(
            started.elapsed() < Duration::from_secs(1200),
            "LIVE_TRANSFER_WAIT_EXCEEDED",
        )?;
        thread::sleep(Duration::from_millis(250));
    }
}

fn execute_stage(root: &Path, stage: &str, archive: bool) -> Check {
    let database = root.join("data/tasks.sqlite3");
    let service = ApplicationService::new(ApplicationServiceConfig::business_store(&database));
    let mut logged_in = false;
    let outcome = (|| {
        if stage == "finish-delivery" {
            return finish_delivery(&service, root);
        }
        let mut baseline = None;
        if stage == "prepare" {
            create(&service, root, archive, &mut logged_in)?;
        } else {
            let saved = if stage == "recheck-pause" {
                capture_preserved_pause(root)?
            } else {
                read_json(&root.join("paused.json"))?
            };
            let name = saved["file_name"].as_str().ok_or("LIVE_PAUSED_INVALID")?;
            let files = frozen_files(root)?;
            let (path, _) = files
                .iter()
                .find(|(p, _)| p.file_name().and_then(|n| n.to_str()) == Some(name))
                .ok_or("LIVE_PAUSED_INVALID")?;
            require(
                hash_prefix(
                    &part_path(path),
                    saved["bytes"].as_u64().ok_or("LIVE_PAUSED_INVALID")?,
                )? == saved["sha256"],
                "LIVE_PART_CHANGED_BEFORE_RESTART",
            )?;
            if archive {
                login(&service, root)?;
                logged_in = true;
            }
            command(&service, &database, TaskStateCommandKind::ResumeTask)?;
            journal(
                root,
                "resumed_new_process",
                json!({"pid":std::process::id(),"saved_bytes":saved["bytes"]}),
            )?;
            baseline = Some(saved);
        }
        wait_download(&service, root, stage != "resume", baseline)
    })();
    if outcome.is_err() {
        let _ = command(&service, &database, TaskStateCommandKind::PauseTask);
    }
    let logout = if logged_in {
        service
            .usgs_m2m_logout()
            .map(|s| !s.authenticated)
            .map_err(|e| e.code().to_owned())
    } else {
        Ok(true)
    };
    let closed = service.shutdown().map_err(|e| e.code().to_owned());
    drop(service);
    journal(
        root,
        "stage_closed",
        json!({"stage":stage,"pid":std::process::id(),"ok":outcome.is_ok(),"code":outcome.as_ref().err(),
        "logout_ok":logout.as_ref().is_ok_and(|v|*v),"logout_code":logout.as_ref().err(),"service_closed":closed.is_ok()}),
    )?;
    let mut result_path = root.join(format!("{stage}-result.json"));
    if result_path.exists() {
        result_path = root.join(format!("{stage}-result-{}.json", uuid::Uuid::new_v4()));
    }
    record_new(
        &result_path,
        &json!({"ok":outcome.is_ok(),"code":outcome.as_ref().err(),"service_closed":closed.is_ok()}),
    )?;
    closed?;
    logout?;
    outcome
}

fn clean_payload(root: &Path) -> Check {
    let mut closed = Vec::new();
    for entry in fs::read_dir(root).map_err(|_| "LIVE_CLEANUP_DIRECTORY")? {
        let entry = entry.map_err(|_| "LIVE_CLEANUP_DIRECTORY")?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.ends_with(".json")
            && [
                "prepare-result",
                "recheck-pause-result",
                "resume-result",
                "finish-delivery-result",
            ]
            .iter()
            .any(|prefix| name.starts_with(prefix))
        {
            closed.push(read_json(&entry.path())?);
        }
    }
    require(
        !closed.is_empty() && closed.iter().all(|v| v["service_closed"] == true),
        "LIVE_CLEANUP_REQUIRES_CLOSED_SERVICES",
    )?;
    fn inventory(path: &Path) -> Check<(u64, u64)> {
        require(plain_path(path), "LIVE_CLEANUP_LINK_REJECTED")?;
        let m = fs::metadata(path).map_err(|_| "LIVE_CLEANUP_METADATA")?;
        if m.is_file() {
            return Ok((1, m.len()));
        }
        let mut totals = (0, 0);
        for entry in fs::read_dir(path).map_err(|_| "LIVE_CLEANUP_DIRECTORY")? {
            let (n, b) = inventory(&entry.map_err(|_| "LIVE_CLEANUP_DIRECTORY")?.path())?;
            totals.0 += n;
            totals.1 += b;
        }
        Ok(totals)
    }
    let data = root.join("data");
    let (files, bytes) = inventory(&data)?;
    fs::remove_dir_all(&data).map_err(|_| "LIVE_CLEANUP_FAILED")?;
    journal(
        root,
        "cleanup",
        json!({"files_removed":files,"bytes_removed":bytes,"payload_absent":!data.exists(),"evidence_retained":true}),
    )
}

#[test]
#[ignore = "explicit Owner authorization, native saved USGS credentials for archives, and a unique test root required"]
fn landsat_current_batch_real_transfer_and_restart() {
    assert_eq!(
        std::env::var("EOSTAGE_LANDSAT_BATCH_LIVE").as_deref(),
        Ok("1")
    );
    let stage = std::env::var("EOSTAGE_LANDSAT_BATCH_STAGE").unwrap();
    assert!([
        "prepare",
        "recheck-pause",
        "resume",
        "finish-delivery",
        "cleanup"
    ]
    .contains(&stage.as_str()));
    let root = PathBuf::from(std::env::var_os("EOSTAGE_LANDSAT_BATCH_ROOT").unwrap());
    assert!(root.is_absolute());
    if stage == "prepare" {
        let mode = std::env::var("EOSTAGE_LANDSAT_BATCH_DELIVERY").unwrap();
        assert!(["archive", "assets"].contains(&mode.as_str()));
        fs::create_dir(&root).expect("test root must be new with an existing parent");
        record_new(
            &root.join("owner.json"),
            &json!({"marker":OWNER_MARKER,"delivery":mode}),
        )
        .unwrap();
        fs::create_dir(root.join("data")).unwrap();
        fs::create_dir(root.join("data/output")).unwrap();
    }
    assert!(plain_path(&root));
    let owner = read_json(&root.join("owner.json")).unwrap();
    assert_eq!(owner["marker"], OWNER_MARKER);
    // A crash leaves this marker for explicit reconciliation instead of letting
    // a second stage or cleanup race an outcome-unknown transfer.
    let lock_path = root.join("active-stage");
    let lock = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock_path)
        .expect("a previous stage is still active or requires reconciliation");
    let result = if stage == "cleanup" {
        clean_payload(&root)
    } else {
        execute_stage(&root, &stage, owner["delivery"] == "archive")
    };
    drop(lock);
    fs::remove_file(lock_path).unwrap();
    if let Err(code) = result {
        panic!("LANDSAT_CURRENT_BATCH_LIVE_STOPPED:{code}");
    }
}

#[test]
fn live_harness_uses_desktop_utc_and_rejects_unowned_cleanup() {
    assert!(now().ends_with('Z'));
    let root = std::env::temp_dir().join(format!("sceneport-live-safety-{}", uuid::Uuid::new_v4()));
    assert!(!plain_path(&root));
    fs::create_dir(&root).unwrap();
    assert_eq!(
        clean_payload(&root).unwrap_err(),
        "LIVE_CLEANUP_REQUIRES_CLOSED_SERVICES"
    );
    fs::remove_dir(root).unwrap();
}
