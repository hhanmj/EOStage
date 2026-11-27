//! Owner-authorized real S-band transfer; ordinary test runs never authenticate.
use eostage_desktop::dem_live_acceptance_support::{state, ApplicationServiceState};
use insar_application_service::{
    ApplicationServiceError, NisarConfirmSubsetTaskV2Request, NisarPreflightSubsetBatchV2Request,
    NisarPrepareSubsetIntentV2Request,
};
use insar_core::{
    CreateTaskFromWorkbenchRequest, DownloadItemStatus, DownloadLayout, StoreTaskCommand,
    StoreTaskPage, StoreTaskPageRequest, StoreUpdateTaskRequest, TaskRuntimeSnapshot,
    TaskStateCommandKind, WorkbenchSelectionCommand, WorkbenchSelectionCommandKind,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::atomic::Ordering;
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

#[path = "support/bhoonidhi_encrypted_relay.rs"]
mod encrypted_relay;
const TASK: &str = "sband-download-acceptance";
const SUBSET_TASK: &str = "sband-remote-subset-acceptance";
const MAX_BYTES: u64 = 40 * 1024 * 1024 * 1024;
const CONTROL_WAIT: Duration = Duration::from_secs(45);

fn record(root: &Path, value: Value) {
    let line = serde_json::to_string(&value).unwrap();
    for forbidden in [
        "bearer ",
        "password",
        "access_token",
        "refresh_token",
        "authorization:",
    ] {
        assert!(!line.to_ascii_lowercase().contains(forbidden));
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("download-events.jsonl"))
        .unwrap();
    writeln!(file, "{line}").unwrap();
    println!("{line}");
}
fn call<T>(value: Result<T, ApplicationServiceError>) -> Result<T, String> {
    value.map_err(|error| error.code().to_owned())
}
fn runtime_for(
    service: &ApplicationServiceState,
    task_id: &str,
) -> Result<TaskRuntimeSnapshot, String> {
    call(service.snapshot(true))?
        .restore
        .runtimes
        .into_iter()
        .find(|runtime| runtime.task_id == task_id)
        .ok_or("TASK_RUNTIME_MISSING".into())
}
fn runtime(service: &ApplicationServiceState) -> Result<TaskRuntimeSnapshot, String> {
    runtime_for(service, TASK)
}
fn page_for(service: &ApplicationServiceState, task_id: &str) -> Result<StoreTaskPage, String> {
    for _ in 0..40 {
        let revision = runtime_for(service, task_id)?.revision;
        match service.task_page(&StoreTaskPageRequest {
            task_id: task_id.into(),
            expected_revision: revision,
            page_index: 0,
            page_size: 1,
        }) {
            Ok(page) => return Ok(page),
            Err(error) if error.code() == "V3_STORE_REVISION_CONFLICT" => {
                thread::sleep(Duration::from_millis(25))
            }
            Err(error) => return Err(error.code().into()),
        }
    }
    Err("TASK_PAGE_BUSY".into())
}
fn page(service: &ApplicationServiceState) -> Result<StoreTaskPage, String> {
    page_for(service, TASK)
}
fn control(
    service: &ApplicationServiceState,
    kind: TaskStateCommandKind,
    event: &str,
) -> Result<(), String> {
    for attempt in 0..40 {
        let revision = runtime(service)?.revision;
        match service.update_download_task(&StoreUpdateTaskRequest {
            task_id: TASK.into(),
            expected_revision: revision,
            command: StoreTaskCommand {
                kind: kind.clone(),
                item_ids: vec![],
            },
            event_id: format!("sband-{event}-{attempt}"),
            occurred_at: "2026-09-08T08:00:00Z".into(),
        }) {
            Ok(_) => return Ok(()),
            Err(error) if error.code() == "V3_STORE_REVISION_CONFLICT" => {
                thread::sleep(Duration::from_millis(25))
            }
            Err(error) => return Err(error.code().into()),
        }
    }
    Err("CONTROL_BUSY".into())
}
fn file_len(path: &Path) -> u64 {
    fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0)
}
fn sha256_prefix(path: &Path, length: u64) -> Result<String, String> {
    let mut file = fs::File::open(path)
        .map_err(|_| "PREFIX_OPEN_FAILED")?
        .take(length);
    let mut hash = Sha256::new();
    let mut count = 0;
    let mut buffer = [0; 64 * 1024];
    loop {
        let n = file.read(&mut buffer).map_err(|_| "PREFIX_READ_FAILED")?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
        count += n as u64;
    }
    if count != length {
        return Err("PREFIX_SIZE_MISMATCH".into());
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn wait_bytes(
    service: &ApplicationServiceState,
    root: &Path,
    threshold: u64,
    timeout: Duration,
) -> Result<StoreTaskPage, String> {
    let start = Instant::now();
    let mut reported = Instant::now() - Duration::from_secs(20);
    loop {
        let page = page(service)?;
        let item = page.items.first().ok_or("TASK_ITEM_MISSING")?;
        if item.total_bytes.is_some_and(|size| size > MAX_BYTES) {
            return Err("ACCEPTANCE_SIZE_BUDGET_EXCEEDED".into());
        }
        if reported.elapsed() >= Duration::from_secs(15) {
            record(
                root,
                json!({"phase":"transfer", "status":item.status,"bytes":item.bytes_completed,"total":item.total_bytes,"rate":item.bytes_per_second,"attempt":item.attempt}),
            );
            reported = Instant::now();
        }
        if matches!(
            item.status,
            DownloadItemStatus::Failed
                | DownloadItemStatus::Cancelled
                | DownloadItemStatus::Interrupted
        ) {
            return Err(item
                .failure
                .as_ref()
                .map(|f| f.code.clone())
                .unwrap_or("TRANSFER_STOPPED".into()));
        }
        if item.bytes_completed >= threshold
            || matches!(
                item.status,
                DownloadItemStatus::Completed | DownloadItemStatus::Skipped
            )
        {
            return Ok(page);
        }
        if start.elapsed() >= timeout {
            return Err("TRANSFER_DEADLINE".into());
        }
        thread::sleep(Duration::from_millis(100));
    }
}
fn pause_stable(service: &ApplicationServiceState, partial: &Path) -> Result<u64, String> {
    control(service, TaskStateCommandKind::PauseTask, "pause")?;
    let start = Instant::now();
    let mut previous = file_len(partial);
    let mut stable = Instant::now();
    loop {
        let current = file_len(partial);
        if current != previous {
            previous = current;
            stable = Instant::now();
        }
        if runtime(service)?.task_paused
            && current > 0
            && stable.elapsed() >= Duration::from_secs(2)
        {
            return Ok(current);
        }
        if start.elapsed() > CONTROL_WAIT {
            return Err("PAUSE_NOT_STABLE".into());
        }
        thread::sleep(Duration::from_millis(100));
    }
}
fn prepare(
    service: &ApplicationServiceState,
    root: &Path,
    relay: &encrypted_relay::Relay,
) -> Result<(), String> {
    if std::env::var("INSAR_SBAND_ACCEPTANCE_SELECT_CP").as_deref() == Ok("1") {
        let geometry = json!({"type":"Polygon","coordinates":[[[68.0,6.0],[98.0,6.0],[98.0,38.0],[68.0,38.0],[68.0,6.0]]]});
        let request = serde_json::from_value::<insar_application_service::ApplicationCatalogSearchAndFreezeRequest>(json!({
            "request_id":"sband-cp-download-catalogue","batch_id":"sband-cp-download-batch",
            "created_at":"2026-09-08T08:00:00Z","aoi_name":"bounded CP full-product acceptance",
            "aoi_geojson":geometry,"workbench_page_size":1,
            "search":{"logical_dataset":"sar.nisar.s.ssar.rslc.v1","provider_id":"isro.nrsc.bhoonidhi",
                "native_collection_id":"NISAR_SSAR_RSLC","aoi_geojson":geometry,"cursor":null,"result_limit":1,
                "filters":{"online":"Y","polarization":"CP","mode":"W","datetime":{"start":"2026-07-08T00:00:00Z","end":"2026-09-07T23:59:59Z"}}}
        })).map_err(|_| "CP_QUERY_CONSTRUCTION_INVALID")?;
        record(
            root,
            json!({"phase":"cp_catalogue_start","limit":1,"downloads_started":false}),
        );
        let started = Instant::now();
        let found = service.catalog_search_and_freeze(&request);
        record(
            root,
            json!({"phase":"cp_catalogue_result","ok":found.is_ok(),"code":found.as_ref().err().map(|error|error.code()),"elapsed_ms":started.elapsed().as_millis(),"downloads_started":false}),
        );
        call(found)?;
    }
    let snapshot = call(service.snapshot(true))?;
    if !snapshot.restore.runtimes.is_empty() {
        return Err("ACCEPTANCE_STORE_NOT_EMPTY".into());
    }
    let batch = snapshot
        .restore
        .workbench_batch
        .ok_or("SOURCE_BATCH_MISSING")?;
    let workbench = snapshot.restore.workbench.ok_or("WORKBENCH_MISSING")?;
    if batch.candidates.len() != 1 {
        return Err("SOURCE_COUNT_INVALID".into());
    }
    let product = &batch.candidates[0];
    if product.provider_id != "isro.nrsc.bhoonidhi" {
        return Err("SOURCE_PROVIDER_MISMATCH".into());
    }
    let asset = product
        .asset_access
        .as_ref()
        .ok_or("SOURCE_ASSET_MISSING")?;
    let name = asset
        .file_name
        .as_deref()
        .ok_or("SOURCE_FILENAME_MISSING")?;
    if Path::new(name).components().count() != 1 || name.contains(['/', '\\', ':']) {
        return Err("SOURCE_FILENAME_UNSAFE".into());
    }
    let destination = root.join("downloads").join(name);
    let partial = insar_download::part_path(&destination);
    let subset = service.preflight_nisar_subset_batch_v2(&NisarPreflightSubsetBatchV2Request {
        source_batch_id: batch.batch_id.clone(),
        source_item_ids: vec![product.item_id.clone()],
        expected_workbench_revision: workbench.revision,
        subset_draft_geometry: product
            .footprint_geojson
            .clone()
            .ok_or("SOURCE_GEOMETRY_MISSING")?,
    });
    record(
        root,
        json!({"phase":"subset_preflight", "ok":subset.is_ok(), "code":subset.as_ref().err().map(|e|e.code()),"download_started":false,"source_item_id_chars":product.item_id.chars().count(),"batch_id_chars":batch.batch_id.chars().count(),"source_id_trimmed":product.item_id == product.item_id.trim(),"source_id_has_control":product.item_id.chars().any(char::is_control)}),
    );
    call(
        service.create_task_from_workbench(&CreateTaskFromWorkbenchRequest {
            task_id: TASK.into(),
            batch_id: batch.batch_id,
            expected_workbench_revision: workbench.revision,
            exact_item_id: Some(product.item_id.clone()),
            output_root: root.join("downloads"),
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-09-08T08:00:00Z".into(),
        }),
    )?;
    record(
        root,
        json!({"phase":"task_created", "task_id":TASK,"file_name":name,"pid":std::process::id()}),
    );
    // Complete-image creation already wakes the normal scheduler. The separate
    // start_newly_confirmed_task method admits only remote-derived subset tasks.
    let first = wait_bytes(service, root, 64 * 1024 * 1024, Duration::from_secs(600))?;
    if matches!(
        first.items[0].status,
        DownloadItemStatus::Completed | DownloadItemStatus::Skipped
    ) {
        return Err("FILE_FINISHED_BEFORE_LIFECYCLE_EXERCISE".into());
    }
    let paused = pause_stable(service, &partial)?;
    if !relay.interrupted.load(Ordering::SeqCst) || first.items[0].attempt < 2 {
        return Err("NETWORK_INTERRUPTION_NOT_PROVEN".into());
    }
    record(
        root,
        json!({"phase":"paused", "part_bytes":paused,"network_cut":relay.interrupted.load(Ordering::SeqCst),"attempt":first.items[0].attempt}),
    );
    control(service, TaskStateCommandKind::ResumeTask, "resume")?;
    wait_bytes(
        service,
        root,
        paused + 8 * 1024 * 1024,
        Duration::from_secs(180),
    )?;
    let checkpoint = pause_stable(service, &partial)?;
    let hash = sha256_prefix(&partial, checkpoint)?;
    let evidence = json!({"phase":"restart_checkpoint","part_bytes":checkpoint,"prefix_sha256":hash,"destination":destination,
        "same_process_resume":true,"network_cut":relay.interrupted.load(Ordering::SeqCst),"relay_bytes":relay.bytes.load(Ordering::SeqCst),"pid":std::process::id()});
    fs::write(
        root.join("restart-checkpoint.json"),
        serde_json::to_vec_pretty(&evidence).unwrap(),
    )
    .map_err(|_| "CHECKPOINT_WRITE_FAILED")?;
    record(root, evidence);
    Ok(())
}
fn finish(service: &ApplicationServiceState, root: &Path) -> Result<(), String> {
    let checkpoint: Value = serde_json::from_slice(
        &fs::read(root.join("restart-checkpoint.json")).map_err(|_| "CHECKPOINT_MISSING")?,
    )
    .map_err(|_| "CHECKPOINT_INVALID")?;
    if checkpoint["pid"].as_u64() == Some(std::process::id() as u64) {
        return Err("RESTART_REQUIRES_NEW_PROCESS".into());
    }
    let destination = PathBuf::from(
        checkpoint["destination"]
            .as_str()
            .ok_or("DESTINATION_MISSING")?,
    );
    if !destination.starts_with(root.join("downloads")) {
        return Err("DESTINATION_OUTSIDE_ACCEPTANCE".into());
    }
    let partial = insar_download::part_path(&destination);
    let offset = checkpoint["part_bytes"].as_u64().ok_or("OFFSET_MISSING")?;
    if file_len(&partial) != offset
        || sha256_prefix(&partial, offset)? != checkpoint["prefix_sha256"].as_str().unwrap_or("")
    {
        return Err("RESTART_PREFIX_CHANGED".into());
    }
    if !runtime(service)?.task_paused {
        return Err("RESTART_PAUSED_STATE_LOST".into());
    }
    record(
        root,
        json!({"phase":"process_restored","pid":std::process::id(),"part_bytes":offset,"prefix_verified":true}),
    );
    control(service, TaskStateCommandKind::ResumeTask, "restart-resume")?;
    let started = Instant::now();
    let mut report_at = Instant::now() - Duration::from_secs(20);
    let mut reset = false;
    loop {
        let page = page(service)?;
        let item = &page.items[0];
        let bytes = file_len(&partial);
        if bytes < offset && !destination.exists() && !reset {
            reset = true;
            record(
                root,
                json!({"phase":"restart_resume_failed","code":"PART_RESTARTED_FROM_ZERO","previous_bytes":offset,"current_bytes":bytes}),
            );
        }
        if report_at.elapsed() >= Duration::from_secs(15) {
            record(
                root,
                json!({"phase":"finishing","bytes":item.bytes_completed,"part_bytes":bytes,"total":item.total_bytes,"rate":item.bytes_per_second,"attempt":item.attempt,"status":item.status}),
            );
            report_at = Instant::now();
        }
        if item.total_bytes.is_some_and(|size| size > MAX_BYTES) {
            return Err("ACCEPTANCE_SIZE_BUDGET_EXCEEDED".into());
        }
        if matches!(
            item.status,
            DownloadItemStatus::Failed
                | DownloadItemStatus::Cancelled
                | DownloadItemStatus::Interrupted
        ) {
            return Err(item
                .failure
                .as_ref()
                .map(|f| f.code.clone())
                .unwrap_or("TRANSFER_STOPPED".into()));
        }
        if matches!(
            item.status,
            DownloadItemStatus::Completed | DownloadItemStatus::Skipped
        ) {
            let size = file_len(&destination);
            if size == 0 || item.total_bytes != Some(size) || partial.exists() {
                return Err("FINAL_SIZE_OR_PROMOTION_INVALID".into());
            }
            let hash = sha256_prefix(&destination, size)?;
            let prefix = sha256_prefix(&destination, offset)?
                == checkpoint["prefix_sha256"].as_str().unwrap_or("");
            record(
                root,
                json!({"phase":"complete_artifact","bytes":size,"sha256":hash,"prefix_unchanged":prefix,"cross_process_resume":!reset&&prefix,"production_validator_passed":true,"destination":destination}),
            );
            if reset {
                return Err("COMPLETE_DOWNLOAD_PASSED_BUT_RESTART_RESUME_FAILED".into());
            }
            return Ok(());
        }
        if started.elapsed() > Duration::from_secs(7200) {
            return Err("COMPLETE_DOWNLOAD_DEADLINE".into());
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn small_subset_geometry(
    product: &insar_core::SnapshotItem,
) -> Result<serde_json::Map<String, Value>, String> {
    fn visit_coordinates(
        value: &Value,
        bounds: &mut Option<(f64, f64, f64, f64)>,
    ) -> Result<(), String> {
        let values = value.as_array().ok_or("SOURCE_GEOMETRY_INVALID")?;
        if values.len() >= 2 {
            if let (Some(x), Some(y)) = (values[0].as_f64(), values[1].as_f64()) {
                if !x.is_finite() || !y.is_finite() {
                    return Err("SOURCE_GEOMETRY_INVALID".into());
                }
                *bounds = Some(match *bounds {
                    Some((west, south, east, north)) => {
                        (west.min(x), south.min(y), east.max(x), north.max(y))
                    }
                    None => (x, y, x, y),
                });
                return Ok(());
            }
        }
        for child in values {
            visit_coordinates(child, bounds)?;
        }
        Ok(())
    }

    let footprint = product
        .footprint_geojson
        .as_ref()
        .ok_or("SOURCE_GEOMETRY_MISSING")?;
    let coordinates = footprint
        .get("coordinates")
        .ok_or("SOURCE_GEOMETRY_INVALID")?;
    let mut bounds = None;
    visit_coordinates(coordinates, &mut bounds)?;
    let (west, south, east, north) = bounds.ok_or("SOURCE_GEOMETRY_INVALID")?;
    if !(west < east && south < north) {
        return Err("SOURCE_GEOMETRY_INVALID".into());
    }
    let center_x = (west + east) / 2.0;
    let center_y = (south + north) / 2.0;
    let half_x = ((east - west) * 0.01).clamp(0.001, 0.05);
    let half_y = ((north - south) * 0.01).clamp(0.001, 0.05);
    json!({
        "type": "Polygon",
        "coordinates": [[
            [center_x - half_x, center_y - half_y],
            [center_x + half_x, center_y - half_y],
            [center_x + half_x, center_y + half_y],
            [center_x - half_x, center_y + half_y],
            [center_x - half_x, center_y - half_y]
        ]]
    })
    .as_object()
    .cloned()
    .ok_or("SUBSET_GEOMETRY_INVALID".into())
}

#[test]
#[ignore = "explicit isolated frozen-store preflight only; no credentials, task creation or network transfer"]
fn existing_frozen_sband_preflight_only() {
    assert_eq!(
        std::env::var("INSAR_SBAND_PREFLIGHT_ONLY").as_deref(),
        Ok("1")
    );
    let root =
        PathBuf::from(std::env::var_os("INSAR_SBAND_PREFLIGHT_ROOT").expect("fresh isolated root"));
    assert!(root.is_absolute() && root.join("tasks.sqlite3").is_file());

    let service = state(
        root.join("tasks.sqlite3"),
        PathBuf::from(std::env::var_os("INSAR_COMPLETION_GIS_ROOT").expect("product runtime")),
    )
    .expect("open isolated frozen Store");
    let snapshot = service.snapshot(true).expect("read frozen source");
    assert!(
        snapshot.restore.runtimes.is_empty(),
        "preflight fixture must contain no download runtime"
    );
    let batch = snapshot
        .restore
        .workbench_batch
        .expect("frozen Bhoonidhi batch");
    let workbench = snapshot.restore.workbench.expect("frozen workbench");
    assert_eq!(batch.candidates.len(), 1);
    let product = &batch.candidates[0];
    assert_eq!(product.provider_id, "isro.nrsc.bhoonidhi");
    let provider_product_id = product
        .metadata
        .get("provider_product_id")
        .and_then(Value::as_str)
        .expect("frozen provider product id");
    let polarization_code = provider_product_id
        .split('_')
        .nth(9)
        .expect("frozen SSAR filename polarization code");
    assert_eq!(
        polarization_code, "DHNA",
        "preflight fixture must stay bound to the frozen DHNA source; observed {polarization_code}"
    );

    let result = service.preflight_nisar_subset_batch_v2(&NisarPreflightSubsetBatchV2Request {
        source_batch_id: batch.batch_id.clone(),
        source_item_ids: vec![product.item_id.clone()],
        expected_workbench_revision: workbench.revision,
        subset_draft_geometry: small_subset_geometry(product).expect("bounded subset geometry"),
    });
    println!(
        "{}",
        json!({
            "phase":"dhna_preflight_only",
            "ok":result.is_ok(),
            "code":result.as_ref().err().map(|error| error.code()),
            "source_kind":"DHNA",
            "candidate_count":batch.candidates.len(),
            "downloads_started":false
        })
    );
    let shutdown = service.shutdown();
    assert!(shutdown.is_ok(), "preflight service shutdown failed");
    result.expect("real frozen DHNA must pass local subset preflight");
}

#[test]
#[ignore = "explicit owner-authorized real Bhoonidhi SSAR remote-derived subset; requires isolated copied Store"]
fn native_sband_remote_subset_completion() {
    assert_eq!(
        std::env::var("INSAR_SBAND_SUBSET_ACCEPTANCE").as_deref(),
        Ok("1")
    );
    let root =
        PathBuf::from(std::env::var_os("INSAR_SBAND_SUBSET_ROOT").expect("isolated subset root"));
    assert!(root.is_absolute() && root.join("tasks.sqlite3").is_file());
    let output_root = root.join("subset-output");
    assert!(!output_root.exists(), "subset output root must be fresh");
    fs::create_dir_all(&output_root).unwrap();

    let service = state(
        root.join("tasks.sqlite3"),
        PathBuf::from(std::env::var_os("INSAR_COMPLETION_GIS_ROOT").unwrap()),
    )
    .unwrap();
    let snapshot = call(service.snapshot(true)).unwrap();
    assert!(
        snapshot.restore.runtimes.is_empty(),
        "subset source Store must contain no prior task runtime"
    );
    let batch = snapshot
        .restore
        .workbench_batch
        .as_ref()
        .expect("frozen Bhoonidhi workbench");
    let workbench = snapshot
        .restore
        .workbench
        .as_ref()
        .expect("workbench summary");
    let product = batch
        .candidates
        .iter()
        .find(|candidate| candidate.provider_id == "isro.nrsc.bhoonidhi")
        .expect("one frozen Bhoonidhi S-band source");
    let provider_product_id = product
        .metadata
        .get("provider_product_id")
        .and_then(Value::as_str)
        .expect("frozen provider product id");
    let polarization_code = provider_product_id
        .split('_')
        .nth(9)
        .expect("frozen SSAR filename polarization code");
    let expected_polarizations = match polarization_code {
        "SHNA" => vec!["HH".to_owned()],
        "SVNA" => vec!["VV".to_owned()],
        "DHNA" => vec!["HH".to_owned(), "HV".to_owned()],
        "DVNA" => vec!["VV".to_owned(), "VH".to_owned()],
        "CLNA" => vec!["LH".to_owned(), "LV".to_owned()],
        "CRNA" => vec!["RH".to_owned(), "RV".to_owned()],
        other => panic!("unsupported frozen SSAR polarization code: {other}"),
    };
    assert!(
        product.item_id.len() > 128 && product.item_id.len() <= 1024,
        "real Bhoonidhi source identity must exercise the widened V2 identity envelope"
    );
    let resolver_eligibility = insar_download::evaluate_resolver_cutover_eligibility(
        product,
        &insar_download::AsfCollectionProfileRegistry,
        &insar_download::AssetExecutionPolicyRegistry,
    );
    let (resolver_class, resolver_code) = match &resolver_eligibility {
        insar_download::ResolverCutoverEligibility::Eligible(_) => ("eligible", None),
        insar_download::ResolverCutoverEligibility::Ineligible(reason) => {
            ("ineligible", Some(reason.code()))
        }
        insar_download::ResolverCutoverEligibility::Invalid(error) => {
            ("invalid", Some(error.code()))
        }
    };
    record(
        &root,
        json!({
            "event":"source_resolver_eligibility",
            "class":resolver_class,
            "code":resolver_code,
            "downloads_started":false
        }),
    );
    assert!(
        matches!(
            &resolver_eligibility,
            insar_download::ResolverCutoverEligibility::Eligible(_)
        ),
        "frozen S-band source is not eligible under the current production resolver policy: {resolver_class}:{resolver_code:?}"
    );

    let workbench = call(
        service.update_workbench_selection(&WorkbenchSelectionCommand {
            batch_id: batch.batch_id.clone(),
            expected_revision: workbench.revision,
            kind: WorkbenchSelectionCommandKind::CheckItems,
            item_ids: vec![product.item_id.clone()],
            page_index: workbench.page_index,
            page_size: workbench.page_size,
            updated_at: "2026-10-05T02:50:00Z".to_owned(),
        }),
    )
    .unwrap();

    let subset_geometry = small_subset_geometry(product).expect("bounded subset geometry");
    let preflight = call(service.preflight_nisar_subset_batch_v2(
        &NisarPreflightSubsetBatchV2Request {
            source_batch_id: batch.batch_id.clone(),
            source_item_ids: vec![product.item_id.clone()],
            expected_workbench_revision: workbench.revision,
            subset_draft_geometry: subset_geometry.clone(),
        },
    ))
    .unwrap();
    assert_eq!(preflight.eligible_items.len(), 1);
    assert!(preflight.excluded_items.is_empty());
    let selection = preflight
        .common_selections
        .iter()
        .find(|selection| selection.frequency == "A")
        .expect("SSAR Frequency A subset selection");
    let selected_frequency = selection.frequency.clone();
    let selected_polarizations = selection.polarizations.clone();
    let mut observed_polarizations = selected_polarizations.clone();
    let mut expected_polarization_set = expected_polarizations.clone();
    observed_polarizations.sort();
    expected_polarization_set.sort();
    assert_eq!(
        observed_polarizations, expected_polarization_set,
        "preflight selection must exactly match the frozen SSAR product polarization set"
    );

    let prepared = call(service.prepare_nisar_subset_intent_v2(
        &NisarPrepareSubsetIntentV2Request {
            source_batch_id: batch.batch_id.clone(),
            source_item_id: product.item_id.clone(),
            expected_workbench_revision: workbench.revision,
            subset_draft_geometry: subset_geometry,
            selected_frequency: selected_frequency.clone(),
            selected_polarizations: selected_polarizations.clone(),
        },
    ))
    .unwrap();
    assert_eq!(prepared.source_item_id, product.item_id);
    assert_eq!(
        prepared.intent.selected_polarizations,
        selected_polarizations
    );

    call(
        service.confirm_nisar_subset_task_v2(&NisarConfirmSubsetTaskV2Request {
            source_batch_id: batch.batch_id.clone(),
            source_item_id: product.item_id.clone(),
            expected_workbench_revision: workbench.revision,
            prepared_intent: prepared.intent,
            task_id: SUBSET_TASK.into(),
            output_root: output_root.clone(),
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-10-05T02:50:00Z".to_owned(),
        }),
    )
    .unwrap();
    let confirmed_runtime = runtime_for(&service, SUBSET_TASK).expect("confirmed subset runtime");
    assert_eq!(
        confirmed_runtime.status,
        insar_core::TaskLifecycleStatus::Paused
    );
    assert!(confirmed_runtime.task_paused);
    let confirmed_page = page_for(&service, SUBSET_TASK).unwrap();
    let frozen_item = &confirmed_page.items[0].item;
    assert_eq!(
        frozen_item
            .metadata
            .get("delivery_kind")
            .and_then(Value::as_str),
        Some("remote_derived_subset_v2")
    );

    call(service.start_newly_confirmed_task(SUBSET_TASK)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(30 * 60);
    let mut last_report = Instant::now() - Duration::from_secs(30);
    let completed = loop {
        let page = page_for(&service, SUBSET_TASK).unwrap();
        let item = page.items.first().expect("one subset item");
        if last_report.elapsed() >= Duration::from_secs(15) {
            record(
                &root,
                json!({
                    "event":"subset_progress",
                    "task_id":SUBSET_TASK,
                    "status":format!("{:?}", item.status),
                    "bytes":item.bytes_completed,
                    "attempt":item.attempt
                }),
            );
            last_report = Instant::now();
        }
        match item.status {
            DownloadItemStatus::Completed => break item.clone(),
            DownloadItemStatus::Failed
            | DownloadItemStatus::Cancelled
            | DownloadItemStatus::Skipped => {
                let code = item
                    .failure
                    .as_ref()
                    .map(|failure| failure.code.as_str())
                    .unwrap_or("NO_FAILURE_DETAIL");
                panic!("real S-band subset ended terminally: {code}");
            }
            _ => {}
        }
        assert!(Instant::now() < deadline, "real S-band subset timed out");
        thread::sleep(Duration::from_millis(250));
    };

    let destination = completed.destination.expect("derived subset destination");
    let canonical_destination = fs::canonicalize(&destination).unwrap();
    let canonical_output_root = fs::canonicalize(&output_root).unwrap();
    assert!(canonical_destination.starts_with(&canonical_output_root));
    assert!(canonical_destination.is_file());
    assert_eq!(
        destination.extension().and_then(|value| value.to_str()),
        Some("h5")
    );
    let artifact_bytes = fs::metadata(&destination).unwrap().len();
    assert!(artifact_bytes > 0 && artifact_bytes < MAX_BYTES);
    let artifact_name = destination.file_name().unwrap().to_string_lossy();
    let sidecar_path =
        destination.with_file_name(format!("{artifact_name}.artifact-manifest-v2.json"));
    let sidecar: Value = serde_json::from_slice(&fs::read(&sidecar_path).unwrap()).unwrap();
    assert_eq!(sidecar["complete"], json!(true));
    assert_eq!(sidecar["output_authority"], json!("client_derived"));
    assert_eq!(sidecar["selected_frequency"], json!(selected_frequency));
    assert_eq!(
        sidecar["selected_polarizations"],
        serde_json::to_value(&selected_polarizations).unwrap()
    );
    let source_file_name = product
        .asset_access
        .as_ref()
        .and_then(|access| access.file_name.as_deref())
        .expect("source filename");
    assert!(
        !output_root.join(source_file_name).exists(),
        "remote subset acceptance must not materialize the complete source product"
    );

    record(
        &root,
        json!({
            "event":"subset_completed",
            "task_id":SUBSET_TASK,
            "source_item_id_bytes":product.item_id.len(),
            "selected_frequency":selected_frequency,
            "source_polarization_code":polarization_code,
            "selected_polarizations":selected_polarizations,
            "artifact_bytes":artifact_bytes,
            "derived_artifact":destination.file_name().unwrap().to_string_lossy(),
            "sidecar_complete":true,
            "output_authority":"client_derived",
            "complete_source_materialized":false
        }),
    );
    call(service.shutdown()).unwrap();
}

#[test]
#[ignore = "explicit owner-authorized restart of an existing real S-band subset task"]
fn native_sband_remote_subset_resume_existing() {
    assert_eq!(
        std::env::var("INSAR_SBAND_SUBSET_RESUME_ACCEPTANCE").as_deref(),
        Ok("1")
    );
    let root = PathBuf::from(
        std::env::var_os("INSAR_SBAND_SUBSET_RESUME_ROOT").expect("existing isolated subset root"),
    );
    assert!(root.is_absolute() && root.join("tasks.sqlite3").is_file());
    let output_root = root.join("subset-output");
    let recovery_root = output_root.join(".nisar-subset-v2-recovery");
    let checkpoint_paths = fs::read_dir(&recovery_root)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("science-checkpoint-v2.json"))
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    assert_eq!(
        checkpoint_paths.len(),
        1,
        "resume acceptance requires one durable science checkpoint"
    );
    let checkpoint_path = checkpoint_paths[0].clone();
    let initial_checkpoint: Value =
        serde_json::from_slice(&fs::read(&checkpoint_path).unwrap()).unwrap();
    let initial_completed = initial_checkpoint["completed_ranges"]
        .as_array()
        .expect("completed science ranges")
        .clone();
    let initial_planned = initial_checkpoint["planned_ranges"]
        .as_array()
        .expect("planned science ranges")
        .len();
    let initial_committed_requests = initial_checkpoint["committed_science_request_count"]
        .as_u64()
        .expect("committed science request count");
    assert!(
        !initial_completed.is_empty() && initial_completed.len() < initial_planned,
        "resume acceptance must start from a real partial science checkpoint"
    );
    assert!(
        initial_checkpoint["pending_attempt"].is_null(),
        "hard interruption evidence must reopen from a committed boundary"
    );
    record(
        &root,
        json!({
            "event":"subset_resume_reopen",
            "task_id":SUBSET_TASK,
            "completed_ranges":initial_completed.len(),
            "planned_ranges":initial_planned,
            "committed_science_requests":initial_committed_requests,
            "checkpoint_state":initial_checkpoint["state"]
        }),
    );

    let service = state(
        root.join("tasks.sqlite3"),
        PathBuf::from(std::env::var_os("INSAR_COMPLETION_GIS_ROOT").unwrap()),
    )
    .unwrap();
    let startup = call(service.snapshot(true)).unwrap();
    let reopened_runtime = startup
        .restore
        .runtimes
        .iter()
        .find(|runtime| runtime.task_id == SUBSET_TASK)
        .expect("persisted subset runtime");
    assert!(
        !reopened_runtime.active_item_ids.is_empty(),
        "restart must recover the previously active subset item"
    );

    let deadline = Instant::now() + Duration::from_secs(30 * 60);
    let mut last_report = Instant::now() - Duration::from_secs(30);
    let completed = loop {
        let page = page_for(&service, SUBSET_TASK).unwrap();
        let item = page.items.first().expect("one subset item");
        if last_report.elapsed() >= Duration::from_secs(15) {
            record(
                &root,
                json!({
                    "event":"subset_resume_progress",
                    "task_id":SUBSET_TASK,
                    "status":format!("{:?}", item.status),
                    "bytes":item.bytes_completed,
                    "attempt":item.attempt
                }),
            );
            last_report = Instant::now();
        }
        match item.status {
            DownloadItemStatus::Completed => break item.clone(),
            DownloadItemStatus::Failed
            | DownloadItemStatus::Cancelled
            | DownloadItemStatus::Skipped => {
                let code = item
                    .failure
                    .as_ref()
                    .map(|failure| failure.code.as_str())
                    .unwrap_or("NO_FAILURE_DETAIL");
                panic!("real S-band subset resume ended terminally: {code}");
            }
            _ => {}
        }
        assert!(
            Instant::now() < deadline,
            "real S-band subset resume timed out"
        );
        thread::sleep(Duration::from_millis(250));
    };

    let final_checkpoint: Value =
        serde_json::from_slice(&fs::read(&checkpoint_path).unwrap()).unwrap();
    let final_completed = final_checkpoint["completed_ranges"]
        .as_array()
        .expect("final completed science ranges");
    assert_eq!(final_completed.len(), initial_planned);
    assert_eq!(
        &final_completed[..initial_completed.len()],
        initial_completed.as_slice(),
        "restart must preserve already verified science ranges"
    );
    let final_committed_requests = final_checkpoint["committed_science_request_count"]
        .as_u64()
        .expect("final committed science request count");
    let remaining_ranges = u64::try_from(initial_planned - initial_completed.len()).unwrap();
    assert!(
        final_committed_requests >= initial_committed_requests
            && final_committed_requests - initial_committed_requests <= remaining_ranges + 1,
        "restart must not replay the already completed science ranges"
    );

    let destination = completed.destination.expect("derived subset destination");
    let canonical_destination = fs::canonicalize(&destination).unwrap();
    let canonical_output_root = fs::canonicalize(&output_root).unwrap();
    assert!(
        canonical_destination.starts_with(&canonical_output_root)
            && canonical_destination.is_file()
    );
    assert_eq!(
        destination.extension().and_then(|value| value.to_str()),
        Some("h5")
    );
    let artifact_bytes = fs::metadata(&destination).unwrap().len();
    assert!(artifact_bytes > 0 && artifact_bytes < MAX_BYTES);
    let artifact_name = destination.file_name().unwrap().to_string_lossy();
    let sidecar_path =
        destination.with_file_name(format!("{artifact_name}.artifact-manifest-v2.json"));
    let sidecar: Value = serde_json::from_slice(&fs::read(&sidecar_path).unwrap()).unwrap();
    assert_eq!(sidecar["complete"], json!(true));
    assert_eq!(sidecar["output_authority"], json!("client_derived"));
    record(
        &root,
        json!({
            "event":"subset_resume_completed",
            "task_id":SUBSET_TASK,
            "initial_completed_ranges":initial_completed.len(),
            "final_completed_ranges":final_completed.len(),
            "initial_committed_science_requests":initial_committed_requests,
            "final_committed_science_requests":final_committed_requests,
            "artifact_bytes":artifact_bytes,
            "derived_artifact":destination.file_name().unwrap().to_string_lossy(),
            "sidecar_complete":true,
            "output_authority":"client_derived"
        }),
    );
    call(service.shutdown()).unwrap();
}

#[test]
#[ignore = "explicit owner-authorized writer retry on an existing completed science checkpoint"]
fn native_sband_remote_subset_retry_writer_existing() {
    assert_eq!(
        std::env::var("INSAR_SBAND_SUBSET_WRITER_RETRY_ACCEPTANCE").as_deref(),
        Ok("1")
    );
    let root = PathBuf::from(
        std::env::var_os("INSAR_SBAND_SUBSET_WRITER_RETRY_ROOT")
            .expect("existing isolated subset root"),
    );
    assert!(root.is_absolute() && root.join("tasks.sqlite3").is_file());
    let output_root = root.join("subset-output");
    let recovery_root = output_root.join(".nisar-subset-v2-recovery");
    let checkpoint_paths = fs::read_dir(&recovery_root)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("science-checkpoint-v2.json"))
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    assert_eq!(
        checkpoint_paths.len(),
        1,
        "writer retry acceptance requires one durable science checkpoint"
    );
    let checkpoint_path = checkpoint_paths[0].clone();
    let initial_checkpoint: Value =
        serde_json::from_slice(&fs::read(&checkpoint_path).unwrap()).unwrap();
    let initial_completed = initial_checkpoint["completed_ranges"]
        .as_array()
        .expect("completed science ranges")
        .clone();
    let initial_planned = initial_checkpoint["planned_ranges"]
        .as_array()
        .expect("planned science ranges")
        .len();
    let initial_committed_requests = initial_checkpoint["committed_science_request_count"]
        .as_u64()
        .expect("committed science request count");
    assert_eq!(initial_checkpoint["state"], json!("Complete"));
    assert_eq!(initial_completed.len(), initial_planned);
    assert!(initial_checkpoint["pending_attempt"].is_null());

    let service = state(
        root.join("tasks.sqlite3"),
        PathBuf::from(std::env::var_os("INSAR_COMPLETION_GIS_ROOT").unwrap()),
    )
    .unwrap();
    let runtime = runtime_for(&service, SUBSET_TASK).expect("failed subset runtime");
    assert!(matches!(
        runtime.status,
        insar_core::TaskLifecycleStatus::Failed
    ));
    assert_eq!(runtime.failed_item_ids.len(), 1);
    let item_id = runtime.failed_item_ids[0].clone();
    let page = page_for(&service, SUBSET_TASK).unwrap();
    let failed = page.items.first().expect("one failed subset item");
    assert!(matches!(failed.status, DownloadItemStatus::Failed));
    assert_eq!(
        failed.failure.as_ref().map(|failure| failure.code.as_str()),
        Some("NISAR_SUBSET_V2_WRITER_BINDING")
    );
    record(
        &root,
        json!({
            "event":"subset_writer_retry_start",
            "task_id":SUBSET_TASK,
            "completed_ranges":initial_completed.len(),
            "committed_science_requests":initial_committed_requests
        }),
    );

    call(service.update_download_task(&StoreUpdateTaskRequest {
        task_id: SUBSET_TASK.into(),
        expected_revision: runtime.revision,
        command: StoreTaskCommand {
            kind: TaskStateCommandKind::RetryItems,
            item_ids: vec![item_id],
        },
        event_id: format!("sband-subset-writer-retry-{}", runtime.revision),
        occurred_at: "2026-10-06T00:10:00Z".into(),
    }))
    .unwrap();

    let deadline = Instant::now() + Duration::from_secs(15 * 60);
    let mut last_report = Instant::now() - Duration::from_secs(30);
    let completed = loop {
        let page = page_for(&service, SUBSET_TASK).unwrap();
        let item = page.items.first().expect("one subset item");
        if last_report.elapsed() >= Duration::from_secs(15) {
            record(
                &root,
                json!({
                    "event":"subset_writer_retry_progress",
                    "task_id":SUBSET_TASK,
                    "status":format!("{:?}", item.status),
                    "bytes":item.bytes_completed,
                    "attempt":item.attempt
                }),
            );
            last_report = Instant::now();
        }
        match item.status {
            DownloadItemStatus::Completed => break item.clone(),
            DownloadItemStatus::Failed
            | DownloadItemStatus::Cancelled
            | DownloadItemStatus::Skipped => {
                let code = item
                    .failure
                    .as_ref()
                    .map(|failure| failure.code.as_str())
                    .unwrap_or("NO_FAILURE_DETAIL");
                panic!("real S-band subset writer retry ended terminally: {code}");
            }
            _ => {}
        }
        assert!(
            Instant::now() < deadline,
            "real S-band subset writer retry timed out"
        );
        thread::sleep(Duration::from_millis(250));
    };

    let final_checkpoint: Value =
        serde_json::from_slice(&fs::read(&checkpoint_path).unwrap()).unwrap();
    assert_eq!(
        final_checkpoint["completed_ranges"].as_array().unwrap(),
        initial_completed.as_slice(),
        "writer retry must not replay completed science ranges"
    );
    assert_eq!(
        final_checkpoint["committed_science_request_count"]
            .as_u64()
            .unwrap(),
        initial_committed_requests,
        "writer retry must not issue additional science requests"
    );

    let destination = completed.destination.expect("derived subset destination");
    let canonical_destination = fs::canonicalize(&destination).unwrap();
    let canonical_output_root = fs::canonicalize(&output_root).unwrap();
    assert!(
        canonical_destination.starts_with(&canonical_output_root)
            && canonical_destination.is_file()
    );
    assert_eq!(
        destination.extension().and_then(|value| value.to_str()),
        Some("h5")
    );
    let artifact_bytes = fs::metadata(&destination).unwrap().len();
    assert!(artifact_bytes > 0 && artifact_bytes < MAX_BYTES);
    let artifact_name = destination.file_name().unwrap().to_string_lossy();
    let sidecar_path =
        destination.with_file_name(format!("{artifact_name}.artifact-manifest-v2.json"));
    let sidecar: Value = serde_json::from_slice(&fs::read(&sidecar_path).unwrap()).unwrap();
    assert_eq!(sidecar["complete"], json!(true));
    assert_eq!(sidecar["output_authority"], json!("client_derived"));
    record(
        &root,
        json!({
            "event":"subset_writer_retry_completed",
            "task_id":SUBSET_TASK,
            "science_ranges_reused":initial_completed.len(),
            "science_requests_unchanged":initial_committed_requests,
            "artifact_bytes":artifact_bytes,
            "derived_artifact":destination.file_name().unwrap().to_string_lossy(),
            "sidecar_complete":true,
            "output_authority":"client_derived"
        }),
    );
    call(service.shutdown()).unwrap();
}

#[test]
#[ignore = "explicit owner-authorized real download; process phases require isolated copied Store"]
fn native_sband_download_lifecycle() {
    assert_eq!(
        std::env::var("INSAR_SBAND_DOWNLOAD_ACCEPTANCE").as_deref(),
        Ok("1")
    );
    let root = PathBuf::from(std::env::var_os("INSAR_SBAND_DOWNLOAD_ROOT").expect("isolated root"));
    let phase = std::env::var("INSAR_SBAND_DOWNLOAD_PHASE").unwrap();
    assert!(root.is_absolute() && root.join("tasks.sqlite3").is_file());
    let relay = (phase == "prepare").then(encrypted_relay::Relay::start);
    if let Some(relay) = &relay {
        // A fresh, single-test process only; never writes Windows/user proxy settings.
        std::env::set_var("HTTPS_PROXY", format!("http://127.0.0.1:{}", relay.port));
    }
    let service = state(
        root.join("tasks.sqlite3"),
        PathBuf::from(std::env::var_os("INSAR_COMPLETION_GIS_ROOT").unwrap()),
    )
    .unwrap();
    struct Cleanup<'a>(Option<&'a ApplicationServiceState>);
    impl Drop for Cleanup<'_> {
        fn drop(&mut self) {
            if let Some(service) = self.0.take() {
                if runtime(service).is_ok() {
                    let _ = control(service, TaskStateCommandKind::PauseTask, "panic-pause");
                }
                let _ = service.shutdown();
            }
        }
    }
    let mut cleanup = Cleanup(Some(&service));
    let result = match phase.as_str() {
        "prepare" => prepare(&service, &root, relay.as_ref().unwrap()),
        "finish" => finish(&service, &root),
        _ => Err("UNKNOWN_PHASE".into()),
    };
    if result.is_err() && runtime(&service).is_ok() {
        let _ = control(&service, TaskStateCommandKind::PauseTask, "failure-pause");
    }
    let shutdown = service.shutdown();
    cleanup.0 = None;
    record(
        &root,
        json!({"phase":"phase_finished","name":phase,"ok":result.is_ok(),"code":result.as_ref().err(),"shutdown_ok":shutdown.is_ok(),"shutdown_code":shutdown.as_ref().err().map(|e|e.code())}),
    );
    assert!(shutdown.is_ok(), "native shutdown failed");
    assert!(
        result.is_ok(),
        "native acceptance failed: {}",
        result.unwrap_err()
    );
}

#[test]
fn checkpoint_hash_is_bounded_and_rejects_a_short_file() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/bhoonidhi_download_acceptance.rs");
    let source = include_bytes!("bhoonidhi_download_acceptance.rs");
    assert_eq!(
        sha256_prefix(&path, 17).unwrap(),
        format!("{:x}", Sha256::digest(&source[..17]))
    );
    assert_eq!(
        sha256_prefix(&path, file_len(&path) + 1).unwrap_err(),
        "PREFIX_SIZE_MISMATCH"
    );
}

#[test]
#[ignore = "explicit anonymous transport diagnostic only; no saved credential access"]
fn anonymous_relay_route_check() {
    let relay = encrypted_relay::Relay::start();
    let mut statuses = Vec::new();
    for (route, port) in [("owner_proxy", 7897), ("test_relay", relay.port)] {
        let client = reqwest::blocking::Client::builder()
            .proxy(reqwest::Proxy::https(format!("http://127.0.0.1:{port}")).unwrap())
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let started = Instant::now();
        match client
            .get("https://bhoonidhi-api.nrsc.gov.in/auth/token")
            .send()
        {
            Ok(response) => {
                statuses.push(response.status().as_u16());
                println!(
                    "{}",
                    json!({"route":route,"http":response.status().as_u16(),"elapsed_ms":started.elapsed().as_millis(),"relay_encrypted_bytes":relay.bytes.load(Ordering::SeqCst),"credentials_used":false})
                );
            }
            Err(error) => println!(
                "{}",
                json!({"route":route,"connect_error":error.is_connect(),"timeout":error.is_timeout(),"error":format!("{error:?}"),"elapsed_ms":started.elapsed().as_millis(),"relay_encrypted_bytes":relay.bytes.load(Ordering::SeqCst),"credentials_used":false})
            ),
        }
    }
    assert_eq!(
        statuses,
        vec![401, 401],
        "anonymous relay must match the owner's real proxy before any credential use"
    );
}
