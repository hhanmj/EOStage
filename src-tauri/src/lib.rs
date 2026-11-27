mod agent;
mod app_info;
mod app_update;
mod application;
mod cli_console;
mod commands;
mod compat_worker;
mod desktop_ui_prefs;
mod diagnostics;
mod eostage_account;
mod error;
mod event_target_guard;
pub mod external_gis_bridge;
mod external_url;
pub mod gee_cloud_suite;
pub mod geodata_tools;
pub mod log_dehydrator;
#[cfg(target_os = "macos")]
mod macos_cdp_bridge;
pub mod mcp_client;
mod oauth_login;
mod transfer_engine_commands;
pub mod ui_smoke;

use tauri::{Emitter, Manager};

pub use app_info::{desktop_app_info, AppInfo};
pub use cli_console::attach_parent_console_for_cli;
pub use error::ShellError;

pub(crate) fn emit_host_event(app: &tauri::AppHandle, event: &str, payload: serde_json::Value) {
    if std::env::var_os("RUST_LOG").is_some() && event != "agent:live-delta" {
        let task_id = payload
            .get("task_id")
            .and_then(|v| v.as_str())
            .or_else(|| payload.get("session_id").and_then(|v| v.as_str()))
            .unwrap_or("-");
        let status = payload
            .get("status")
            .and_then(|v| v.as_str())
            .or_else(|| payload.get("kind").and_then(|v| v.as_str()))
            .or_else(|| payload.get("state").and_then(|v| v.as_str()))
            .unwrap_or("-");
        eprintln!("[EOStage][event] name={event} id={task_id} state={status}");
    }
    let _ = app.emit(event, payload.clone());
    #[cfg(target_os = "macos")]
    macos_cdp_bridge::push_event(event, payload);
}

/// Narrow, test-only access to the normal desktop Application Service runtime.
///
/// The live DEM acceptance test is an integration test, so it cannot reach the
/// private Tauri application module directly. Keeping this adapter under a
/// hidden module lets that test construct the same product-owned native GIS
/// state as desktop startup without exposing a second runtime or worker path.
#[doc(hidden)]
pub mod dem_live_acceptance_support {
    pub use super::application::ApplicationServiceState;

    use super::application::native_gis_capability_observations;
    use insar_gis::LazyNativeGisEngine;
    use std::path::PathBuf;

    pub fn state(
        store_path: PathBuf,
        runtime_root: PathBuf,
    ) -> Result<ApplicationServiceState, String> {
        if !runtime_root.is_absolute() {
            return Err("DEM_LIVE_NATIVE_GIS_ROOT_NOT_ABSOLUTE".to_owned());
        }
        let engine = LazyNativeGisEngine::from_product_root(&runtime_root).map_err(|error| {
            format!(
                "DEM_LIVE_NATIVE_GIS_DISCOVERY_{}: {error}",
                error.code().as_str()
            )
        })?;
        let observations = native_gis_capability_observations(&engine);
        Ok(
            ApplicationServiceState::with_store_path_and_runtime_observations_and_lazy_native_gis(
                store_path,
                observations,
                engine,
            ),
        )
    }
}

pub fn is_allowed_desktop_window_label(label: &str) -> bool {
    label == "main"
}

pub fn is_allowed_desktop_origin(url: &tauri::Url, development: bool) -> bool {
    let is_packaged_tauri_origin =
        (url.scheme() == "tauri" && url.host_str() == Some("localhost") && url.port().is_none())
            || (url.host_str() == Some("tauri.localhost")
                && matches!(url.scheme(), "http" | "https")
                && url.port().is_none());
    if is_packaged_tauri_origin {
        return true;
    }

    development
        && url.scheme() == "http"
        && url.host_str() == Some("127.0.0.1")
        && url.port() == Some(5173)
}

pub fn selftest_payload() -> AppInfo {
    desktop_app_info(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"))
}

#[cfg(feature = "s1-live-acceptance")]
struct S1LiveNoopWorker;

#[cfg(feature = "s1-live-acceptance")]
impl insar_download::DownloadWorker for S1LiveNoopWorker {
    fn execute(
        &self,
        work: insar_download::DownloadWork,
        _control: insar_download::WorkerControl,
        _progress: &mut dyn FnMut(insar_core::DownloadItemProgress),
    ) -> insar_core::DownloadItemResult {
        insar_core::DownloadItemResult {
            task_id: work.task.task_id,
            item_id: work.item.item_id,
            outcome: insar_core::DownloadOutcome::Failed,
            destination: work.task.output_root,
            bytes_written: 0,
            attempt_count: 1,
            failure: Some(insar_core::DownloadFailure {
                code: "S1_LIVE_ACCEPTANCE_NO_DOWNLOAD".to_owned(),
                reason_zh: "S1 实机检索验收不执行下载".to_owned(),
                retryable: false,
            }),
            finished_at: "2026-08-22T00:00:00Z".to_owned(),
        }
    }
}

#[cfg(feature = "s1-live-acceptance")]
pub fn s1_live_acceptance_payload() -> Result<serde_json::Value, String> {
    use insar_application_service::{
        AdminBoundaryOptionsRequest, AdminBoundaryResolveRequest, AdminBoundarySearchRequest,
        ApplicationSarSearchAndFreezeRequest, ApplicationService, ApplicationServiceConfig,
        ApplicationServiceError, ApplicationSetWorkingAoiRequest,
    };
    use insar_core::{
        CreateTaskFromWorkbenchRequest, DownloadLayout, StoreTaskCommand, StoreUpdateTaskRequest,
        TaskLifecycleStatus, TaskStateCommandKind, WorkbenchSelectionCommand,
        WorkbenchSelectionCommandKind,
    };

    fn coordinate_count(value: &serde_json::Value) -> usize {
        match value {
            serde_json::Value::Array(items)
                if items.len() >= 2 && items[..2].iter().all(serde_json::Value::is_number) =>
            {
                1
            }
            serde_json::Value::Array(items) => items.iter().map(coordinate_count).sum(),
            serde_json::Value::Object(object) => object
                .get("coordinates")
                .map(coordinate_count)
                .unwrap_or_default(),
            _ => 0,
        }
    }

    fn antimeridian_edge_count(value: &serde_json::Value) -> usize {
        let serde_json::Value::Array(items) = value else {
            return value
                .as_object()
                .and_then(|object| object.get("coordinates"))
                .map(antimeridian_edge_count)
                .unwrap_or_default();
        };
        let is_ring = items.len() >= 4
            && items.iter().all(|item| {
                item.as_array().is_some_and(|position| {
                    position.len() >= 2
                        && position[0].as_f64().is_some_and(f64::is_finite)
                        && position[1].as_f64().is_some_and(f64::is_finite)
                })
            });
        if is_ring {
            return items
                .windows(2)
                .filter(|pair| {
                    let left = pair[0].as_array().and_then(|position| position[0].as_f64());
                    let right = pair[1].as_array().and_then(|position| position[0].as_f64());
                    matches!((left, right), (Some(left), Some(right)) if (right - left).abs() > 180.0)
                })
                .count();
        }
        items.iter().map(antimeridian_edge_count).sum()
    }

    fn error_evidence(error: &ApplicationServiceError) -> serde_json::Value {
        let provider = match error {
            ApplicationServiceError::Provider(source) => Some(serde_json::json!({
                "provider_code": source.code,
                "provider_debug_context": source.debug_context,
            })),
            _ => None,
        };
        serde_json::json!({
            "application_code": error.code(),
            "provider": provider,
        })
    }

    fn run_search(
        service: &ApplicationService,
        geometry: &serde_json::Map<String, serde_json::Value>,
        bbox: &[f64],
        suffix: &str,
        max_results: u64,
        filters: serde_json::Value,
        start: &str,
        end: &str,
    ) -> serde_json::Value {
        let mut query = serde_json::json!({
            "provider": "ASF",
            "dataset": "SENTINEL-1",
            "processing_levels": ["SLC"],
            "aoi_geojson": geometry,
            "bbox": {
                "west": bbox[0],
                "south": bbox[1],
                "east": bbox[2],
                "north": bbox[3],
                "crs": "EPSG:4326"
            },
            "start": start,
            "end": end,
            "max_results": max_results
        });
        if let (Some(query), Some(filters)) = (query.as_object_mut(), filters.as_object()) {
            query.extend(filters.clone());
        }
        let request: ApplicationSarSearchAndFreezeRequest =
            serde_json::from_value(serde_json::json!({
                "request_id": format!("s1-live-{suffix}"),
                "search": {"query": query},
                "batch_id": format!("s1-live-batch-{suffix}"),
                "created_at": "2026-08-22T00:00:00Z",
                "aoi_name": suffix,
                "aoi_geojson": geometry,
                "page_size": 24
            }))
            .expect("feature-gated live request is statically valid JSON");
        let plan = insar_provider::asf::query::plan_sar_query(&request.search.query)
            .expect("feature-gated live query uses the production ASF planner");
        let should_count = plan.should_count;
        let aoi_mode = plan.aoi_mode.as_str();
        let wkt_bytes = plan.params.get("intersectsWith").map(String::len);
        let stages = std::sync::Mutex::new(Vec::new());
        let started = std::time::Instant::now();
        match service.sar_search_and_freeze_with_progress(&request, |update| {
            stages.lock().expect("live progress lock").push(update);
        }) {
            Ok(core_snapshot) => {
                let full_snapshot = service.snapshot(true).ok();
                let batch = full_snapshot
                    .as_ref()
                    .and_then(|snapshot| snapshot.restore.workbench_batch.as_ref())
                    .filter(|batch| batch.batch_id == request.batch_id);
                let stats = batch.and_then(|batch| batch.search_stats.as_ref());
                let first_item_id = batch
                    .and_then(|batch| batch.candidates.first())
                    .map(|item| item.item_id.clone());
                let workbench = core_snapshot
                    .core
                    .workbench
                    .as_ref()
                    .filter(|workbench| workbench.batch_id == request.batch_id);
                let terminal = stages.lock().expect("live progress lock").last().cloned();
                serde_json::json!({
                    "outcome": "success",
                    "elapsed_ms": started.elapsed().as_millis(),
                    "should_count": should_count,
                    "aoi_mode": aoi_mode,
                    "intersects_with_bytes": wkt_bytes,
                    "matched_count": stats.and_then(|stats| stats.matched_count),
                    "fetched_record_count": stats.and_then(|stats| stats.fetched_record_count),
                    "mapped_record_count": stats.and_then(|stats| stats.mapped_record_count),
                    "provider_returned_count": stats.map(|stats| stats.provider_returned_count),
                    "frozen_candidate_count": stats.map(|stats| stats.frozen_candidate_count),
                    "batch_id": workbench.map(|workbench| workbench.batch_id.clone()),
                    "first_item_id": first_item_id,
                    "workbench_revision": workbench.map(|workbench| workbench.revision),
                    "terminal_progress": terminal,
                    "final_busy": false,
                })
            }
            Err(error) => {
                let terminal = stages.lock().expect("live progress lock").last().cloned();
                serde_json::json!({
                    "outcome": "error",
                    "elapsed_ms": started.elapsed().as_millis(),
                    "should_count": should_count,
                    "aoi_mode": aoi_mode,
                    "intersects_with_bytes": wkt_bytes,
                    "error": error_evidence(&error),
                    "terminal_progress": terminal,
                    "final_busy": false,
                })
            }
        }
    }

    fn bind_datav_boundary(
        service: &ApplicationService,
        province: &str,
        city: &str,
        district: &str,
        adcode: &str,
    ) -> Result<
        (
            serde_json::Map<String, serde_json::Value>,
            Vec<f64>,
            serde_json::Value,
        ),
        String,
    > {
        let started = std::time::Instant::now();
        let search = service
            .admin_boundary_search(&AdminBoundarySearchRequest {
                province: province.to_owned(),
                city: city.to_owned(),
                district: district.to_owned(),
                provider_object_id: Some(adcode.to_owned()),
                ..Default::default()
            })
            .map_err(|error| format!("{province}/{city}/{district}: {}", error.code()))?;
        let boundary = search
            .results
            .into_iter()
            .find(|result| result.provider == "datav" && result.provider_object_id == adcode)
            .ok_or_else(|| format!("DataV {adcode} result is missing"))?;
        let geometry = boundary
            .geometry
            .as_ref()
            .and_then(serde_json::Value::as_object)
            .cloned()
            .ok_or_else(|| format!("DataV {adcode} geometry is missing"))?;
        insar_core::validated_authoritative_polygonal_geojson(&geometry)
            .ok_or_else(|| format!("DataV {adcode} geometry is invalid"))?;
        let bbox = vec![
            boundary.bbox.west,
            boundary.bbox.south,
            boundary.bbox.east,
            boundary.bbox.north,
        ];
        service
            .set_working_aoi(&ApplicationSetWorkingAoiRequest {
                source_kind: "administrative_boundary".to_owned(),
                geometry: geometry.clone(),
                bbox: bbox.clone(),
                crs: "EPSG:4326".to_owned(),
                display_name: boundary.display_name.clone(),
            })
            .map_err(|error| format!("DataV {adcode} working AOI: {}", error.code()))?;
        let evidence = serde_json::json!({
            "provider": boundary.provider,
            "provider_object_id": boundary.provider_object_id,
            "display_name": boundary.display_name,
            "admin_level": boundary.admin_level,
            "geometry_type": geometry.get("type"),
            "coordinate_count": coordinate_count(&serde_json::Value::Object(geometry.clone())),
            "bbox": bbox,
            "cache_status": boundary.cache_status,
            "elapsed_ms": started.elapsed().as_millis(),
            "working_aoi_bound": true,
        });
        Ok((geometry, bbox, evidence))
    }

    fn bind_china_osm_gap(service: &ApplicationService) -> Result<serde_json::Value, String> {
        let started = std::time::Instant::now();
        let discovery = service
            .admin_boundary_search(&AdminBoundarySearchRequest {
                province: "湖北省".to_owned(),
                city: "荆州市".to_owned(),
                district: "监利市".to_owned(),
                provider_object_id: Some("421088".to_owned()),
                limit: Some(8),
                ..Default::default()
            })
            .map_err(|error| format!("监利市 fallback discovery: {}", error.code()))?;
        if discovery.provider != "nominatim"
            || discovery.results.is_empty()
            || discovery
                .results
                .iter()
                .any(|result| result.geometry.is_some())
        {
            return Err(
                "监利市 did not produce an explicit-selection OSM candidate set".to_owned(),
            );
        }
        let candidate_count = discovery.results.len();
        let candidate = discovery
            .results
            .into_iter()
            .find(|result| {
                result.provider == "nominatim"
                    && result
                        .country_code
                        .as_deref()
                        .is_some_and(|value| value.eq_ignore_ascii_case("cn"))
            })
            .ok_or_else(|| "监利市 China-restricted OSM candidate is missing".to_owned())?;
        let selected_identity = candidate.provider_object_id.clone();
        let resolved = service
            .admin_boundary_resolve(&AdminBoundaryResolveRequest {
                provider: candidate.provider,
                provider_object_id: selected_identity.clone(),
            })
            .map_err(|error| format!("监利市 OSM resolve: {}", error.code()))?;
        if resolved.provider_object_id != selected_identity {
            return Err("监利市 OSM selected identity changed during resolve".to_owned());
        }
        let geometry = resolved
            .geometry
            .as_ref()
            .and_then(serde_json::Value::as_object)
            .cloned()
            .ok_or_else(|| "监利市 OSM geometry is missing".to_owned())?;
        insar_core::validated_authoritative_polygonal_geojson(&geometry)
            .ok_or_else(|| "监利市 OSM geometry is invalid".to_owned())?;
        let bbox = vec![
            resolved.bbox.west,
            resolved.bbox.south,
            resolved.bbox.east,
            resolved.bbox.north,
        ];
        service
            .set_working_aoi(&ApplicationSetWorkingAoiRequest {
                source_kind: "administrative_boundary".to_owned(),
                geometry: geometry.clone(),
                bbox: bbox.clone(),
                crs: "EPSG:4326".to_owned(),
                display_name: resolved.display_name.clone(),
            })
            .map_err(|error| format!("监利市 OSM working AOI: {}", error.code()))?;
        Ok(serde_json::json!({
            "datav_provider_object_id": "421088",
            "datav_outcome": "unavailable",
            "fallback_provider": discovery.provider,
            "warning": discovery.warning,
            "candidate_count": candidate_count,
            "auto_selected": false,
            "explicit_selected_provider_object_id": selected_identity,
            "resolved_provider_object_id": resolved.provider_object_id,
            "geometry_type": geometry.get("type"),
            "coordinate_count": coordinate_count(&serde_json::Value::Object(geometry)),
            "bbox": bbox,
            "elapsed_ms": started.elapsed().as_millis(),
            "working_aoi_bound": true,
        }))
    }

    #[derive(Clone, Debug)]
    struct DownloadTaskProbe {
        status: TaskLifecycleStatus,
        revision: u64,
        transferred_bytes: u64,
        total_bytes: Option<u64>,
        bytes_per_second: f64,
        active_count: usize,
        completed_count: usize,
        failed_count: usize,
        cancelled_count: usize,
    }

    fn download_task_probe(
        service: &ApplicationService,
        task_id: &str,
    ) -> Result<DownloadTaskProbe, String> {
        let snapshot = service
            .core_restore(true)
            .map_err(|error| format!("download task restore: {}", error.code()))?;
        let task = snapshot
            .core
            .tasks
            .into_iter()
            .find(|task| task.task_id == task_id)
            .ok_or_else(|| format!("download task {task_id} is missing"))?;
        Ok(DownloadTaskProbe {
            status: task.status,
            revision: task.revision,
            transferred_bytes: task.transferred_bytes,
            total_bytes: task.total_bytes,
            bytes_per_second: task.bytes_per_second,
            active_count: task.active_count,
            completed_count: task.completed_count,
            failed_count: task.failed_count,
            cancelled_count: task.cancelled_count,
        })
    }

    fn wait_for_download_task(
        service: &ApplicationService,
        task_id: &str,
        timeout: std::time::Duration,
        mut predicate: impl FnMut(&DownloadTaskProbe) -> bool,
    ) -> Result<DownloadTaskProbe, String> {
        let started = std::time::Instant::now();
        loop {
            let probe = download_task_probe(service, task_id)?;
            if predicate(&probe) {
                return Ok(probe);
            }
            if started.elapsed() >= timeout {
                return Err(format!(
                    "download task {task_id} timed out after {} ms at {:?} with {} bytes",
                    started.elapsed().as_millis(),
                    probe.status,
                    probe.transferred_bytes
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }

    fn update_download_task_state(
        service: &ApplicationService,
        task_id: &str,
        revision: u64,
        kind: TaskStateCommandKind,
        event_id: &str,
        occurred_at: &str,
    ) -> Result<DownloadTaskProbe, String> {
        service
            .update_download_task(&StoreUpdateTaskRequest {
                task_id: task_id.to_owned(),
                expected_revision: revision,
                command: StoreTaskCommand {
                    kind,
                    item_ids: Vec::new(),
                },
                event_id: event_id.to_owned(),
                occurred_at: occurred_at.to_owned(),
            })
            .map_err(|error| format!("download task state update: {}", error.code()))?;
        download_task_probe(service, task_id)
    }

    fn wait_for_paused_transfer_to_settle(
        service: &ApplicationService,
        task_id: &str,
        timeout: std::time::Duration,
    ) -> Result<DownloadTaskProbe, String> {
        let started = std::time::Instant::now();
        let mut previous = download_task_probe(service, task_id)?;
        let mut stable_samples = 0_u8;
        loop {
            std::thread::sleep(std::time::Duration::from_millis(250));
            let current = download_task_probe(service, task_id)?;
            if current.status != TaskLifecycleStatus::Paused {
                return Err(format!(
                    "paused S1 transfer changed to {:?} before settling",
                    current.status
                ));
            }
            if current.transferred_bytes == previous.transferred_bytes {
                stable_samples += 1;
                if stable_samples >= 4 {
                    return Ok(current);
                }
            } else {
                stable_samples = 0;
            }
            previous = current;
            if started.elapsed() >= timeout {
                return Err(format!(
                    "paused S1 transfer did not settle within {} ms",
                    started.elapsed().as_millis()
                ));
            }
        }
    }

    fn run_real_s1_download(
        service: &ApplicationService,
        search: &serde_json::Value,
        output_root: &std::path::Path,
    ) -> serde_json::Value {
        let started = std::time::Instant::now();
        let execute = || -> Result<serde_json::Value, String> {
            if !output_root.is_absolute() {
                return Err("INSAR_S1_LIVE_DOWNLOAD_ROOT must be absolute".to_owned());
            }
            let batch_id = search
                .get("batch_id")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "successful S1 search has no batch_id".to_owned())?;
            let first_item_id = search
                .get("first_item_id")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "successful S1 search has no first_item_id".to_owned())?;
            let workbench_revision = search
                .get("workbench_revision")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| "successful S1 search has no workbench revision".to_owned())?;
            let selection = service
                .update_workbench_selection(&WorkbenchSelectionCommand {
                    batch_id: batch_id.to_owned(),
                    expected_revision: workbench_revision,
                    kind: WorkbenchSelectionCommandKind::CheckItems,
                    item_ids: vec![first_item_id.to_owned()],
                    page_index: 0,
                    page_size: 24,
                    updated_at: "2026-08-22T00:00:01Z".to_owned(),
                })
                .map_err(|error| format!("select S1 result: {}", error.code()))?;
            if selection.checked_count != 1 {
                return Err(format!(
                    "S1 result selection admitted {} checked items instead of one",
                    selection.checked_count
                ));
            }

            let task_id = "s1-live-china-core-download";
            service
                .create_task_from_workbench(&CreateTaskFromWorkbenchRequest {
                    task_id: task_id.to_owned(),
                    batch_id: batch_id.to_owned(),
                    expected_workbench_revision: selection.revision,
                    exact_item_id: None,
                    output_root: output_root.to_path_buf(),
                    layout: DownloadLayout::Sentinel1Slc,
                    concurrency: 1,
                    created_at: "2026-08-22T00:00:02Z".to_owned(),
                })
                .map_err(|error| format!("create S1 download task: {}", error.code()))?;

            let advancing = wait_for_download_task(
                service,
                task_id,
                std::time::Duration::from_secs(120),
                |probe| {
                    probe.transferred_bytes >= 1_048_576
                        || matches!(
                            probe.status,
                            TaskLifecycleStatus::Completed
                                | TaskLifecycleStatus::Failed
                                | TaskLifecycleStatus::Cancelled
                                | TaskLifecycleStatus::Interrupted
                        )
                },
            )?;
            if advancing.transferred_bytes == 0 {
                let snapshot = service.download_snapshot(task_id).ok();
                return Err(format!(
                    "S1 production transfer reached {:?} before bytes advanced: {:?}",
                    advancing.status,
                    snapshot.and_then(|snapshot| snapshot.results.into_iter().next())
                ));
            }
            if matches!(advancing.status, TaskLifecycleStatus::Completed) {
                let snapshot = service
                    .download_snapshot(task_id)
                    .map_err(|error| format!("completed download snapshot: {}", error.code()))?;
                return Ok(serde_json::json!({
                    "outcome": "completed_before_pause",
                    "task_id": task_id,
                    "item_id": first_item_id,
                    "layout": "sentinel1_slc",
                    "bytes_after_start": advancing.transferred_bytes,
                    "total_bytes": advancing.total_bytes,
                    "terminal_status": advancing.status,
                    "results": snapshot.results,
                    "elapsed_ms": started.elapsed().as_millis(),
                }));
            }

            let pause_requested = update_download_task_state(
                service,
                task_id,
                advancing.revision,
                TaskStateCommandKind::PauseTask,
                "s1-live-pause",
                "2026-08-22T00:00:03Z",
            )?;
            let paused = wait_for_download_task(
                service,
                task_id,
                std::time::Duration::from_secs(30),
                |probe| probe.status == TaskLifecycleStatus::Paused && probe.active_count == 0,
            )?;
            let paused_stable = wait_for_paused_transfer_to_settle(
                service,
                task_id,
                std::time::Duration::from_secs(30),
            )?;

            let resume_requested = update_download_task_state(
                service,
                task_id,
                paused_stable.revision,
                TaskStateCommandKind::ResumeTask,
                "s1-live-resume",
                "2026-08-22T00:00:04Z",
            )?;
            let resumed = wait_for_download_task(
                service,
                task_id,
                std::time::Duration::from_secs(120),
                |probe| {
                    probe.transferred_bytes >= paused_stable.transferred_bytes + 1_048_576
                        || matches!(
                            probe.status,
                            TaskLifecycleStatus::Completed
                                | TaskLifecycleStatus::Failed
                                | TaskLifecycleStatus::Cancelled
                                | TaskLifecycleStatus::Interrupted
                        )
                },
            )?;
            if resumed.transferred_bytes <= paused_stable.transferred_bytes {
                return Err(format!(
                    "resumed S1 transfer reached {:?} without additional bytes",
                    resumed.status
                ));
            }

            let terminal = if resumed.status == TaskLifecycleStatus::Completed {
                resumed.clone()
            } else {
                update_download_task_state(
                    service,
                    task_id,
                    resumed.revision,
                    TaskStateCommandKind::CancelTask,
                    "s1-live-cancel-after-resume",
                    "2026-08-22T00:00:05Z",
                )?;
                wait_for_download_task(
                    service,
                    task_id,
                    std::time::Duration::from_secs(30),
                    |probe| {
                        matches!(
                            probe.status,
                            TaskLifecycleStatus::Completed
                                | TaskLifecycleStatus::Failed
                                | TaskLifecycleStatus::Cancelled
                                | TaskLifecycleStatus::Interrupted
                        )
                    },
                )?
            };
            let snapshot = service
                .download_snapshot(task_id)
                .map_err(|error| format!("terminal download snapshot: {}", error.code()))?;
            Ok(serde_json::json!({
                "outcome": "terminal",
                "task_id": task_id,
                "batch_id": batch_id,
                "item_id": first_item_id,
                "selected_via_workbench": true,
                "layout": "sentinel1_slc",
                "bytes_after_start": advancing.transferred_bytes,
                "start_bytes_per_second": advancing.bytes_per_second,
                "pause_request_status": pause_requested.status,
                "bytes_when_pause_observed": paused.transferred_bytes,
                "paused_bytes": paused_stable.transferred_bytes,
                "pause_settle_overshoot_bytes": paused_stable
                    .transferred_bytes
                    .saturating_sub(pause_requested.transferred_bytes),
                "resume_request_status": resume_requested.status,
                "bytes_after_resume": resumed.transferred_bytes,
                "terminal_status": terminal.status,
                "terminal_bytes": terminal.transferred_bytes,
                "terminal_completed_count": terminal.completed_count,
                "terminal_failed_count": terminal.failed_count,
                "terminal_cancelled_count": terminal.cancelled_count,
                "results": snapshot.results,
                "elapsed_ms": started.elapsed().as_millis(),
            }))
        };
        match execute() {
            Ok(evidence) => evidence,
            Err(reason) => serde_json::json!({
                "outcome": "blocked_or_failed",
                "reason": reason,
                "elapsed_ms": started.elapsed().as_millis(),
            }),
        }
    }

    fn download_file_evidence(root: &std::path::Path) -> Vec<serde_json::Value> {
        let mut evidence = Vec::new();
        let mut pending = vec![root.to_path_buf()];
        while let Some(path) = pending.pop() {
            let Ok(entries) = std::fs::read_dir(path) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    pending.push(path);
                } else if let Ok(metadata) = entry.metadata() {
                    evidence.push(serde_json::json!({
                        "path": path,
                        "bytes": metadata.len(),
                    }));
                    if evidence.len() >= 20 {
                        return evidence;
                    }
                }
            }
        }
        evidence
    }

    let store_path = std::env::var_os("INSAR_S1_LIVE_STORE_PATH")
        .map(std::path::PathBuf::from)
        .ok_or_else(|| "INSAR_S1_LIVE_STORE_PATH is required".to_owned())?;
    if store_path.exists() || !store_path.is_absolute() {
        return Err("S1 live Store path must be absolute and absent".to_owned());
    }
    let selected_case = std::env::var("INSAR_S1_LIVE_CASE").ok();
    let service = if selected_case.as_deref() == Some("china-core") {
        ApplicationService::new(ApplicationServiceConfig::business_store(&store_path))
    } else {
        let live_asf = insar_provider::asf::search::AsfProvider::production()
            .map_err(|error| format!("ASF production client: {}", error.code))?;
        ApplicationService::with_fixture_business_runtime(
            ApplicationServiceConfig::business_store(&store_path),
            std::sync::Arc::new(live_asf),
            std::sync::Arc::new(S1LiveNoopWorker),
        )
    };
    if selected_case.as_deref() == Some("china-core") {
        let city_options_first = service
            .admin_options(&AdminBoundaryOptionsRequest {
                province: "湖北省".to_owned(),
                city: "全部".to_owned(),
            })
            .map_err(|error| format!("湖北省 city options: {}", error.code()))?;
        let city_options_second = service
            .admin_options(&AdminBoundaryOptionsRequest {
                province: "湖北省".to_owned(),
                city: "全部".to_owned(),
            })
            .map_err(|error| format!("湖北省 repeated city options: {}", error.code()))?;
        let city_order_first = city_options_first
            .cities
            .iter()
            .map(|option| option.provider_object_id.clone())
            .collect::<Vec<_>>();
        let city_order_second = city_options_second
            .cities
            .iter()
            .map(|option| option.provider_object_id.clone())
            .collect::<Vec<_>>();

        let (hubei_geometry, hubei_bbox, hubei_boundary) =
            bind_datav_boundary(&service, "湖北省", "全部", "全部", "420000")?;
        let (_, _, wuhan_boundary) =
            bind_datav_boundary(&service, "湖北省", "武汉市", "全部", "420100")?;
        let (badong_geometry, badong_bbox, badong_boundary) = bind_datav_boundary(
            &service,
            "湖北省",
            "恩施土家族苗族自治州",
            "巴东县",
            "422823",
        )?;
        let gap_boundary = bind_china_osm_gap(&service)?;

        service
            .set_working_aoi(&ApplicationSetWorkingAoiRequest {
                source_kind: "administrative_boundary".to_owned(),
                geometry: hubei_geometry.clone(),
                bbox: hubei_bbox.clone(),
                crs: "EPSG:4326".to_owned(),
                display_name: "湖北省".to_owned(),
            })
            .map_err(|error| format!("湖北省 S1 working AOI: {}", error.code()))?;
        let hubei_standard = run_search(
            &service,
            &hubei_geometry,
            &hubei_bbox,
            "china-core-hubei-standard",
            300,
            serde_json::json!({}),
            "2024-01-01",
            "2024-12-31",
        );
        let earthdata_readiness = insar_download::desktop_earthdata_readiness();
        let earthdata = serde_json::json!({
            "readiness": format!("{earthdata_readiness:?}"),
            "source": insar_download::desktop_earthdata_status(),
        });
        let download_root =
            std::env::var_os("INSAR_S1_LIVE_DOWNLOAD_ROOT").map(std::path::PathBuf::from);
        let download = if earthdata_readiness != insar_download::EarthdataCredentialReadiness::Ready
        {
            serde_json::json!({
                "outcome": "blocked_or_failed",
                "reason": "Earthdata credential readiness is not Ready",
            })
        } else if let Some(download_root) = download_root.as_deref() {
            run_real_s1_download(&service, &hubei_standard, download_root)
        } else {
            serde_json::json!({
                "outcome": "blocked_or_failed",
                "reason": "INSAR_S1_LIVE_DOWNLOAD_ROOT is required",
            })
        };

        let hubei_filtered = run_search(
            &service,
            &hubei_geometry,
            &hubei_bbox,
            "china-core-hubei-filtered",
            30,
            serde_json::json!({
                "beam_modes": ["IW"],
                "polarizations": ["VV+VH"],
                "flight_direction": "DESCENDING"
            }),
            "2024-01-01",
            "2024-12-31",
        );

        service
            .set_working_aoi(&ApplicationSetWorkingAoiRequest {
                source_kind: "administrative_boundary".to_owned(),
                geometry: badong_geometry.clone(),
                bbox: badong_bbox.clone(),
                crs: "EPSG:4326".to_owned(),
                display_name: "巴东县".to_owned(),
            })
            .map_err(|error| format!("巴东县 S1 working AOI: {}", error.code()))?;
        let badong_search = run_search(
            &service,
            &badong_geometry,
            &badong_bbox,
            "china-core-badong",
            30,
            serde_json::json!({"beam_modes": ["IW"]}),
            "2024-01-01",
            "2024-12-31",
        );

        let failure_started = std::time::Instant::now();
        let failure_client = insar_provider::asf::client::AsfHttpClient::production()
            .map_err(|error| format!("ASF failure-simulation client: {}", error.code))?;
        let failure_query: insar_core::SarSearchQuery = serde_json::from_value(serde_json::json!({
            "provider": "ASF",
            "dataset": "SENTINEL-1",
            "processing_levels": ["SLC"],
            "aoi_geojson": hubei_geometry,
            "bbox": {
                "west": hubei_bbox[0],
                "south": hubei_bbox[1],
                "east": hubei_bbox[2],
                "north": hubei_bbox[3],
                "crs": "EPSG:4326"
            },
            "start": "2024-01-01",
            "end": "2024-01-31",
            "max_results": 1
        }))
        .expect("feature-gated failure simulation query is valid");
        let mut failure_params = insar_provider::asf::query::plan_sar_query(&failure_query)
            .map_err(|error| format!("ASF failure-simulation plan: {}", error.code))?
            .params;
        failure_params.insert("output".to_owned(), "unsupported-output".to_owned());
        let bounded_failure = match failure_client.asf_geojson(&failure_params, &|| false) {
            Ok(_) => serde_json::json!({
                "outcome": "unexpected_success",
                "elapsed_ms": failure_started.elapsed().as_millis(),
                "final_busy": false,
            }),
            Err(error) => serde_json::json!({
                "outcome": "error",
                "elapsed_ms": failure_started.elapsed().as_millis(),
                "error_code": error.code,
                "debug_context": error.debug_context,
                "final_busy": false,
            }),
        };
        let city_order_stable = city_order_first == city_order_second;
        let shutdown = service.shutdown().is_ok();
        let persisted_after_restart = if download.get("task_id").is_some() && shutdown {
            let readonly = ApplicationService::new(ApplicationServiceConfig::store(&store_path));
            let restored = readonly.core_restore(true).ok();
            let task = restored
                .as_ref()
                .and_then(|snapshot| {
                    snapshot
                        .core
                        .tasks
                        .iter()
                        .find(|task| task.task_id == "s1-live-china-core-download")
                })
                .and_then(|task| serde_json::to_value(task).ok());
            let readonly_shutdown = readonly.shutdown().is_ok();
            serde_json::json!({
                "task": task,
                "readonly_shutdown": readonly_shutdown,
            })
        } else {
            serde_json::Value::Null
        };
        let download_files = download_root
            .as_deref()
            .map(download_file_evidence)
            .unwrap_or_default();
        return Ok(serde_json::json!({
            "schema_version": 2,
            "evidence_kind": "china_core_boundary_s1_download_live_acceptance",
            "store_path": store_path,
            "admin_option_order": {
                "provider": city_options_first.provider,
                "first": city_order_first,
                "second": city_order_second,
                "stable": city_order_stable,
            },
            "boundaries": {
                "hubei": hubei_boundary,
                "wuhan": wuhan_boundary,
                "badong": badong_boundary,
                "datav_gap_osm": gap_boundary,
            },
            "searches": {
                "hubei_standard": hubei_standard,
                "hubei_filtered": hubei_filtered,
                "badong": badong_search,
                "bounded_failure": bounded_failure,
            },
            "earthdata": earthdata,
            "download": download,
            "download_persisted_after_restart": persisted_after_restart,
            "download_files": download_files,
            "shutdown": shutdown,
        }));
    }
    if let Some(case) = selected_case {
        let (query, expected_country, suffix) = match case.as_str() {
            "us-provider-wait" => ("United States", "us", "united-states"),
            "japan-provider-wait" => ("Japan", "jp", "japan"),
            _ => return Err("INSAR_S1_LIVE_CASE is unsupported".to_owned()),
        };
        let discovery_started = std::time::Instant::now();
        let discovery = service
            .admin_boundary_search(&AdminBoundarySearchRequest {
                query: query.to_owned(),
                limit: Some(8),
                ..Default::default()
            })
            .map_err(|error| format!("{query} discovery: {}", error.code()))?;
        let candidate = discovery
            .results
            .into_iter()
            .find(|result| {
                result.provider == "nominatim"
                    && result
                        .country_code
                        .as_deref()
                        .is_some_and(|value| value.eq_ignore_ascii_case(expected_country))
            })
            .ok_or_else(|| format!("{query} Nominatim candidate is missing"))?;
        let boundary = if candidate.geometry.is_some() {
            candidate
        } else {
            service
                .admin_boundary_resolve(&AdminBoundaryResolveRequest {
                    provider: candidate.provider,
                    provider_object_id: candidate.provider_object_id,
                })
                .map_err(|error| format!("{query} resolve: {}", error.code()))?
        };
        let geometry = boundary
            .geometry
            .as_ref()
            .and_then(serde_json::Value::as_object)
            .cloned()
            .ok_or_else(|| format!("{query} authoritative geometry is missing"))?;
        let validated = insar_core::validated_authoritative_polygonal_geojson(&geometry)
            .ok_or_else(|| format!("{query} authoritative geometry is invalid"))?;
        let envelope = insar_core::wrapped_aoi_envelope(&validated.geometry);
        let antimeridian_edges =
            antimeridian_edge_count(&serde_json::Value::Object(geometry.clone()));
        let bbox = vec![
            boundary.bbox.west,
            boundary.bbox.south,
            boundary.bbox.east,
            boundary.bbox.north,
        ];
        service
            .set_working_aoi(&ApplicationSetWorkingAoiRequest {
                source_kind: "administrative_boundary".to_owned(),
                geometry: geometry.clone(),
                bbox: bbox.clone(),
                crs: "EPSG:4326".to_owned(),
                display_name: boundary.display_name.clone(),
            })
            .map_err(|error| format!("{query} working AOI: {}", error.code()))?;
        let planned_query: insar_core::SarSearchQuery = serde_json::from_value(serde_json::json!({
            "provider": "ASF",
            "dataset": "SENTINEL-1",
            "processing_levels": ["SLC"],
            "aoi_geojson": geometry,
            "bbox": {
                "west": bbox[0], "south": bbox[1], "east": bbox[2], "north": bbox[3], "crs": "EPSG:4326"
            },
            "start": "2026-08-04",
            "end": "2026-08-22",
            "max_results": 30
        }))
        .map_err(|error| format!("{query} diagnostic query: {error}"))?;
        let plan = insar_provider::asf::query::plan_sar_query(&planned_query)
            .map_err(|error| format!("{query} ASF plan: {}", error.code))?;
        let wkt_bytes = plan
            .params
            .get("intersectsWith")
            .map(String::len)
            .ok_or_else(|| format!("{query} ASF plan has no intersectsWith"))?;
        let live = run_search(
            &service,
            &geometry,
            &bbox,
            suffix,
            30,
            serde_json::json!({}),
            "2026-08-04",
            "2026-08-22",
        );
        let shutdown = service.shutdown().is_ok();
        return Ok(serde_json::json!({
            "schema_version": 1,
            "evidence_kind": "s1_provider_wait_live_diagnostic",
            "case": case,
            "identity": {
                "provider": boundary.provider,
                "provider_object_id": boundary.provider_object_id,
                "display_name": boundary.display_name,
                "geometry_type": geometry.get("type"),
                "coordinate_count": coordinate_count(&serde_json::Value::Object(geometry)),
                "bbox": bbox,
                "crosses_antimeridian": antimeridian_edges > 0,
                "antimeridian_edge_count": antimeridian_edges,
                "wrapped_envelope_available": envelope.is_some(),
                "request_window_count": envelope.as_ref().map(|value| value.request_windows.len()),
                "wkt_bytes": wkt_bytes,
                "boundary_elapsed_ms": discovery_started.elapsed().as_millis(),
            },
            "search": live,
            "shutdown": shutdown,
        }));
    }
    let hubei_started = std::time::Instant::now();
    let hubei_search = service
        .admin_boundary_search(&AdminBoundarySearchRequest {
            province: "湖北省".to_owned(),
            city: "全部".to_owned(),
            district: "全部".to_owned(),
            provider_object_id: Some("420000".to_owned()),
            ..Default::default()
        })
        .map_err(|error| format!("湖北省 boundary: {}", error.code()))?;
    let hubei = hubei_search
        .results
        .into_iter()
        .find(|result| result.provider == "datav" && result.provider_object_id == "420000")
        .ok_or_else(|| "湖北省 DataV 420000 result is missing".to_owned())?;
    let hubei_geometry = hubei
        .geometry
        .as_ref()
        .and_then(serde_json::Value::as_object)
        .cloned()
        .ok_or_else(|| "湖北省 authoritative geometry is missing".to_owned())?;
    let hubei_bbox = vec![
        hubei.bbox.west,
        hubei.bbox.south,
        hubei.bbox.east,
        hubei.bbox.north,
    ];
    service
        .set_working_aoi(&ApplicationSetWorkingAoiRequest {
            source_kind: "administrative_boundary".to_owned(),
            geometry: hubei_geometry.clone(),
            bbox: hubei_bbox.clone(),
            crs: "EPSG:4326".to_owned(),
            display_name: hubei.display_name.clone(),
        })
        .map_err(|error| format!("湖北省 working AOI: {}", error.code()))?;

    let standard = run_search(
        &service,
        &hubei_geometry,
        &hubei_bbox,
        "hubei-standard",
        300,
        serde_json::json!({}),
        "2023-01-01",
        "2025-01-01",
    );
    let filtered = run_search(
        &service,
        &hubei_geometry,
        &hubei_bbox,
        "hubei-filtered",
        60,
        serde_json::json!({
            "beam_modes": ["IW"],
            "polarizations": ["VV+VH"],
            "flight_direction": "DESCENDING"
        }),
        "2023-01-01",
        "2025-01-01",
    );

    let new_zealand_search = service
        .admin_boundary_search(&AdminBoundarySearchRequest {
            query: "New Zealand".to_owned(),
            limit: Some(8),
            ..Default::default()
        })
        .map_err(|error| format!("New Zealand discovery: {}", error.code()))?;
    let candidate = new_zealand_search
        .results
        .into_iter()
        .find(|result| {
            result.provider == "nominatim"
                && result
                    .country_code
                    .as_deref()
                    .is_some_and(|value| value.eq_ignore_ascii_case("nz"))
        })
        .ok_or_else(|| "New Zealand Nominatim candidate is missing".to_owned())?;
    let new_zealand = if candidate.geometry.is_some() {
        candidate
    } else {
        service
            .admin_boundary_resolve(&AdminBoundaryResolveRequest {
                provider: candidate.provider,
                provider_object_id: candidate.provider_object_id,
            })
            .map_err(|error| format!("New Zealand resolve: {}", error.code()))?
    };
    let new_zealand_geometry = new_zealand
        .geometry
        .as_ref()
        .and_then(serde_json::Value::as_object)
        .cloned()
        .ok_or_else(|| "New Zealand authoritative geometry is missing".to_owned())?;
    let new_zealand_bbox = vec![
        new_zealand.bbox.west,
        new_zealand.bbox.south,
        new_zealand.bbox.east,
        new_zealand.bbox.north,
    ];
    service
        .set_working_aoi(&ApplicationSetWorkingAoiRequest {
            source_kind: "administrative_boundary".to_owned(),
            geometry: new_zealand_geometry.clone(),
            bbox: new_zealand_bbox.clone(),
            crs: "EPSG:4326".to_owned(),
            display_name: new_zealand.display_name.clone(),
        })
        .map_err(|error| format!("New Zealand working AOI: {}", error.code()))?;
    let deterministic_4xx = run_search(
        &service,
        &new_zealand_geometry,
        &new_zealand_bbox,
        "new-zealand-4xx",
        20,
        serde_json::json!({}),
        "2024-01-01",
        "2025-01-01",
    );

    let shutdown = service.shutdown().is_ok();
    Ok(serde_json::json!({
        "schema_version": 1,
        "evidence_kind": "s1_live_application_service_native_trace",
        "store_path": store_path,
        "hubei_identity": {
            "provider": hubei.provider,
            "provider_object_id": hubei.provider_object_id,
            "display_name": hubei.display_name,
            "geometry_type": hubei_geometry.get("type"),
            "coordinate_count": coordinate_count(&serde_json::Value::Object(hubei_geometry.clone())),
            "bbox": hubei_bbox,
            "boundary_elapsed_ms": hubei_started.elapsed().as_millis(),
        },
        "hubei_standard": standard,
        "hubei_filtered": filtered,
        "deterministic_4xx": deterministic_4xx,
        "new_zealand_identity": {
            "provider": new_zealand.provider,
            "provider_object_id": new_zealand.provider_object_id,
            "geometry_type": new_zealand_geometry.get("type"),
            "coordinate_count": coordinate_count(&serde_json::Value::Object(new_zealand_geometry)),
        },
        "shutdown": shutdown,
    }))
}

/// Explicit, offline packaged smoke for the product-owned native GIS runtime.
/// This path is a test hook only: it constructs no Store, Scheduler,
/// Dispatcher, Provider, credentials, or compatibility Worker.
pub fn native_gis_smoke_payload() -> Result<serde_json::Value, String> {
    let total_started = std::time::Instant::now();
    let input = std::env::var_os("INSAR_NATIVE_GIS_SMOKE_INPUT")
        .map(std::path::PathBuf::from)
        .ok_or_else(|| "INSAR_NATIVE_GIS_SMOKE_INPUT is required".to_owned())?;
    let output_root = std::env::var_os("INSAR_NATIVE_GIS_SMOKE_OUTPUT")
        .map(std::path::PathBuf::from)
        .ok_or_else(|| "INSAR_NATIVE_GIS_SMOKE_OUTPUT is required".to_owned())?;
    let runtime_root = std::env::current_exe()
        .map_err(|error| format!("current executable cannot be resolved: {error}"))?
        .parent()
        .map(|parent| {
            let direct = parent.join("runtime/gis");
            if direct.is_dir() {
                return direct;
            }
            if let Some(contents) = parent.parent() {
                let bundled = contents.join("Resources/runtime/gis");
                if bundled.is_dir() {
                    return bundled;
                }
            }
            direct
        })
        .ok_or_else(|| "current executable has no parent directory".to_owned())?;
    let engine_started = std::time::Instant::now();
    let engine = insar_gis::NativeGisEngine::from_product_root(runtime_root)
        .map_err(|error| format!("{}: {error}", error.code().as_str()))?;
    let engine_initialization_ms = engine_started.elapsed().as_secs_f64() * 1_000.0;
    let inspect_started = std::time::Instant::now();
    let inspect = engine
        .inspect_dem(&input)
        .map_err(|error| format!("{}: {error}", error.code().as_str()))?;
    let inspect_ms = inspect_started.elapsed().as_secs_f64() * 1_000.0;
    std::fs::create_dir_all(&output_root)
        .map_err(|error| format!("native GIS smoke output directory: {error}"))?;
    let egm96_output = output_root.join("packaged_egm96_ellipsoid.tif");
    let egm2008_output = output_root.join("packaged_egm2008_ellipsoid.tif");
    let egm96_started = std::time::Instant::now();
    let egm96 = engine
        .convert_dem(&input, &egm96_output, None, "EGM96")
        .map_err(|error| format!("{}: {error}", error.code().as_str()))?;
    let egm96_ms = egm96_started.elapsed().as_secs_f64() * 1_000.0;
    let egm2008_started = std::time::Instant::now();
    let egm2008 = engine
        .convert_dem(&input, &egm2008_output, None, "EGM2008")
        .map_err(|error| format!("{}: {error}", error.code().as_str()))?;
    let egm2008_ms = egm2008_started.elapsed().as_secs_f64() * 1_000.0;
    // Exercise the user's second conversion action through the same packaged native engine.
    let sarscape_output = output_root.join("packaged_sarscape_dem");
    let copied_ellipsoid = output_root.join("packaged_wgs84_copy.tif");
    let sarscape = engine
        .convert_dem(
            &egm96_output,
            &copied_ellipsoid,
            Some(&sarscape_output),
            "WGS84_ELLIPSOID",
        )
        .map_err(|error| format!("{}: {error}", error.code().as_str()))?;
    let sarscape_inspection = engine
        .inspect_dem(&sarscape_output)
        .map_err(|error| format!("{}: {error}", error.code().as_str()))?;
    if sarscape_inspection["metadata"]["driver"] != "ENVI"
        || sarscape_inspection["metadata"]["count"] != 1
        || sarscape_inspection["metadata"]["dtype"] != serde_json::json!(["float32"])
        || sarscape_inspection["metadata"]["width"] != inspect["metadata"]["width"]
        || sarscape_inspection["metadata"]["height"] != inspect["metadata"]["height"]
        || sarscape_inspection["metadata"]["geotransform"] != inspect["metadata"]["geotransform"]
    {
        return Err(
            "SARSCAPE_ARTIFACT_INVALID: native reopen did not preserve the source grid".to_owned(),
        );
    }
    let total_ms = total_started.elapsed().as_secs_f64() * 1_000.0;
    Ok(serde_json::json!({
        "ok": true,
        "worker": false,
        "store": false,
        "scheduler": false,
        "dispatcher": false,
        "provider": false,
        "credentials": false,
        "runtime_manifest": engine.runtime_manifest(),
        "inspect": inspect,
        "egm96": egm96,
        "egm2008": egm2008,
        "sarscape": sarscape,
        "sarscape_inspection": sarscape_inspection,
        "timings_ms": {
            "engine_initialization": engine_initialization_ms,
            "inspect": inspect_ms,
            "egm96_first_operation": egm96_ms,
            "egm2008_first_operation": egm2008_ms,
            "total": total_ms,
        },
    }))
}

/// Run a bounded, offline smoke path for the packaged host and its managed
/// compatibility Worker. This bypasses the GUI so build verification can
/// exercise the same Application Service and Worker lifecycle without opening
/// a window or issuing provider requests.
pub fn smoke_test_payload() -> Result<serde_json::Value, String> {
    let smoke_store = std::env::temp_dir().join(format!(
        "insar-studio-tauri-smoke-{}.sqlite3",
        std::process::id()
    ));
    let worker = std::sync::Arc::new(compat_worker::CompatWorkerState::from_environment());
    // The packaged smoke is intentionally offline. Keep the live Earthdata
    // credential axis explicitly missing so a host user's local credentials
    // cannot turn this deterministic boundary probe into a NISAR operation.
    let smoke_capability_observations = worker.capability_runtime_observations().with_credential(
        "route.asf.nisar-rslc.remote-subset.v1",
        insar_core::CredentialReadinessState::Missing,
    );
    let state = application::ApplicationServiceState::with_offline_smoke_store_path_and_runtime_observations_and_compat_worker(
        smoke_store.clone(),
        smoke_capability_observations,
        std::sync::Arc::clone(&worker),
    );

    let result: Result<serde_json::Value, String> = (|| {
        let readiness = state
            .readiness()
            .map_err(|error| format!("application_service_readiness:{}", error.code()))?;
        let provider_capabilities = state.provider_capabilities().map_err(|error| {
            format!("application_service_provider_capabilities:{}", error.code())
        })?;
        // This internal smoke path may exercise the legacy compatibility reader,
        // but it is not registered as a normal renderer command.
        let restore = state
            .snapshot(false)
            .map_err(|error| format!("internal_smoke_legacy_restore:{}", error.code()))?;
        let nisar_subset_capability = state
            .nisar_remote_subset_capability()
            .map_err(|error| format!("application_service_nisar_capability:{}", error.code()))?;
        let nisar_subset_task_rejection = match state.create_nisar_remote_subset_task(
            &insar_application_service::NisarCreateRemoteSubsetTaskRequest {
                request_id: "tauri-smoke-nisar-request".to_owned(),
                source_batch_id: "tauri-smoke-batch".to_owned(),
                item_id: "tauri-smoke-item".to_owned(),
                task_id: "tauri-smoke-task".to_owned(),
                output_root: std::env::temp_dir().join("insar-studio-tauri-smoke-nisar"),
                layout: insar_core::DownloadLayout::Direct,
                concurrency: 2,
                frequency: insar_core::NisarSubsetFrequency::A,
                polarization: "HH".to_owned(),
                created_at: "2026-08-05T00:00:00Z".to_owned(),
            },
        ) {
            Ok(_) => return Err("application_service_nisar_task:unexpectedly_enabled".to_owned()),
            Err(error) => error.code().to_owned(),
        };
        if nisar_subset_task_rejection != "NISAR_SUBSET_NOT_ENABLED" {
            return Err(format!(
                "application_service_nisar_task:unexpected_code:{nisar_subset_task_rejection}"
            ));
        }
        let worker_capabilities = worker.capabilities();
        let mut worker_probe = serde_json::Value::Null;
        let mut worker_process_ids = Vec::with_capacity(100);
        for index in 0..100 {
            let probe = worker
                .call("worker_capabilities", serde_json::json!({}))
                .map_err(|error| format!("compatibility_worker_probe:{}", error.code))?;
            if index == 0 {
                worker_probe = probe;
            }
            worker_process_ids.push(
                worker
                    .process_id()
                    .ok_or_else(|| "compatibility_worker_probe:missing_process_id".to_owned())?,
            );
        }
        let mut unique_worker_process_ids = worker_process_ids.clone();
        unique_worker_process_ids.sort_unstable();
        unique_worker_process_ids.dedup();
        if worker_probe.get("public_requests") != Some(&serde_json::json!(0))
            || worker_probe.get("production_remote_subset") != Some(&serde_json::json!(false))
        {
            return Err("compatibility_worker_probe:unsafe_capabilities".to_owned());
        }
        let application_admin_probe = state
            .admin_boundary_search(&insar_application_service::AdminBoundarySearchRequest {
                province: "全国".to_owned(),
                city: "全部".to_owned(),
                district: "全部".to_owned(),
                provider_object_id: Some("100000".to_owned()),
                ..Default::default()
            })
            .map_err(|error| format!("application_service_admin_probe:{}", error.code()))?;
        if application_admin_probe.provider == "datav"
            && application_admin_probe.results.iter().any(|result| {
                matches!(
                    result.geometry_kind.as_deref(),
                    Some("polygon" | "multipolygon")
                ) && result.geometry.is_some()
                    && result.source == "DataV areas_v3"
            })
        {
        } else {
            return Err("application_service_admin_probe:invalid_result".to_owned());
        }
        let application_aoi_probe = state
            .aoi_import(&insar_application_service::ApplicationAoiImportRequest {
                source: "text".to_owned(),
                file_name: Some("application-中文.geojson".to_owned()),
                text: Some(
                    "{\"type\":\"FeatureCollection\",\"features\":[{\"type\":\"Feature\",\"properties\":{\"name\":\"测试区\"},\"geometry\":{\"type\":\"Polygon\",\"coordinates\":[[[100,30],[101,30],[101,31],[100,30]]]}}]}"
                        .to_owned(),
                ),
                ..Default::default()
            })
            .map_err(|error| format!("application_service_aoi_probe:{}", error.code()))?;
        if application_aoi_probe.get("ok") != Some(&serde_json::json!(true))
            || application_aoi_probe.get("file_name")
                != Some(&serde_json::json!("application-中文.geojson"))
        {
            return Err("application_service_aoi_probe:invalid_result".to_owned());
        }
        let smoke_gacos_geometry = serde_json::json!({
            "type": "Polygon",
            "coordinates": [[[100.0, 30.0], [101.0, 30.0], [101.0, 31.0], [100.0, 30.0]]]
        });
        state
            .set_working_aoi(
                &insar_application_service::ApplicationSetWorkingAoiRequest {
                    source_kind: "manual_polygon".to_owned(),
                    geometry: smoke_gacos_geometry
                        .as_object()
                        .cloned()
                        .expect("smoke GACOS geometry object"),
                    bbox: vec![100.0, 30.0, 101.0, 31.0],
                    crs: "EPSG:4326".to_owned(),
                    display_name: "Smoke GACOS AOI".to_owned(),
                },
            )
            .map_err(|error| format!("application_service_gacos_aoi:{}", error.code()))?;
        let application_gacos_probe = state
            .gacos_plan(&serde_json::json!({
                "request_id": "tauri-smoke-gacos",
                "aoi_geojson": smoke_gacos_geometry,
                "aoi_name": "Smoke GACOS AOI",
                "source_kind": "online_selected",
                "selected_scene_ids": ["TAURI_SMOKE_SCENE"],
                "output_dir": std::env::temp_dir()
                    .join("insar-studio-tauri-smoke-gacos")
                    .to_string_lossy()
                    .to_string(),
                "scenes": [{
                    "scene_id": "TAURI_SMOKE_SCENE",
                    "acquisition_datetime": "2024-01-02T03:04:05Z"
                }]
            }))
            .map_err(|error| format!("application_service_gacos_probe:{}", error.code()))?;
        if application_gacos_probe.get("ok") != Some(&serde_json::json!(true))
            || application_gacos_probe.get("offline") != Some(&serde_json::json!(true))
        {
            return Err("application_service_gacos_probe:invalid_result".to_owned());
        }
        let application_gacos_dry_run = state
            .gacos_dry_run(&serde_json::json!({
                "submission": {
                    "provider": "gacos",
                    "batches": [{
                        "batch_id": "tauri-smoke-batch",
                        "batch_index": 1,
                        "form_fields": {"N": "31", "S": "30", "W": "100", "E": "101"}
                    }]
                }
            }))
            .map_err(|error| format!("application_service_gacos_dry_run:{}", error.code()))?;
        if application_gacos_dry_run.get("ok") != Some(&serde_json::json!(true))
            || application_gacos_dry_run.get("offline") != Some(&serde_json::json!(true))
        {
            return Err("application_service_gacos_dry_run:invalid_result".to_owned());
        }
        let worker_aoi_probe = worker
            .call(
                "aoi_import",
                serde_json::json!({
                    "source": "text",
                    "file_name": "tauri-中文.geojson",
                    "text": "{\"type\":\"FeatureCollection\",\"features\":[{\"type\":\"Feature\",\"properties\":{\"name\":\"测试区\"},\"geometry\":{\"type\":\"Polygon\",\"coordinates\":[[[100,30],[101,30],[101,31],[100,30]]]}}]}",
                }),
            )
            .map_err(|error| format!("compatibility_worker_aoi_probe:{}", error.code))?;
        if worker_aoi_probe.get("ok") != Some(&serde_json::json!(true))
            || worker_aoi_probe.get("file_name") != Some(&serde_json::json!("tauri-中文.geojson"))
        {
            return Err("compatibility_worker_aoi_probe:invalid_result".to_owned());
        }
        let worker_dem_probe = worker
            .call(
                "dem_inspect",
                serde_json::json!({
                    "path": std::env::current_exe()
                        .map_err(|_| "compatibility_worker_dem_probe:host_path".to_owned())?,
                }),
            )
            .map_err(|error| format!("compatibility_worker_dem_probe:{}", error.code))?;
        if worker_dem_probe.get("ok") != Some(&serde_json::json!(true)) {
            return Err("compatibility_worker_dem_probe:invalid_result".to_owned());
        }
        let mut operation_process_ids = vec![worker
            .process_id()
            .ok_or_else(|| "compatibility_worker_operation_probe:aoi_pid".to_owned())?];
        for label in ["aoi_import", "dem_inspect"] {
            operation_process_ids.push(
                worker
                    .process_id()
                    .ok_or_else(|| format!("compatibility_worker_operation_probe:{label}_pid"))?,
            );
        }
        let mut unique_operation_process_ids = operation_process_ids.clone();
        unique_operation_process_ids.sort_unstable();
        unique_operation_process_ids.dedup();
        let worker_lifecycle_snapshot = worker.lifecycle_snapshot();
        let worker_lifecycle = serde_json::json!({
            "request_count": 100,
            "process_ids": worker_process_ids,
            "unique_process_ids": unique_worker_process_ids,
            "operation_process_ids": operation_process_ids,
            "unique_operation_process_ids": unique_operation_process_ids,
            "process_id": worker_lifecycle_snapshot.get("process_id"),
            "process_generation": worker_lifecycle_snapshot.get("process_generation"),
            "restart_count": worker_lifecycle_snapshot.get("restart_count"),
            "pending_count": worker_lifecycle_snapshot.get("pending_count"),
        });
        let application_service_ready = matches!(
            readiness.status,
            insar_application_service::ServiceStatus::StoreReady
        );
        let worker_protocol_ready =
            worker_probe.get("protocol") == Some(&serde_json::json!("eostage.compat-worker.v1"));

        // The packaged host must establish HDF5 dynamic plugin isolation
        // itself BEFORE HDF5 initialises: it sanitises the inherited
        // HDF5_PLUGIN_PATH / VOL / driver environment, sets the H5PL
        // sentinel, then verifies the live control mask is zero. The
        // structured status below is the actual evidence (parent sentinel
        // absence is captured before the application sanitises its own
        // environment, so the packaging harness can prove it did not inject
        // HDF5_PLUGIN_PRELOAD).
        let isolation_status = insar_download::hdf5_process_isolation_status();
        let hdf5_plugin_isolation = serde_json::json!({
            "established": isolation_status.isolation_established,
            "loading_disabled": isolation_status.loading_mask_zero,
            "parent_env_preload_absent": isolation_status.parent_env_preload_absent,
            "startup_environment_sanitized": isolation_status.startup_environment_sanitized,
            "preload_sentinel_set_by_application": isolation_status.preload_sentinel_set_by_application,
            "plugin_path_removed": isolation_status.plugin_path_removed,
            "vol_connector_removed": isolation_status.vol_connector_removed,
            "driver_removed": isolation_status.driver_removed,
            "driver_config_removed": isolation_status.driver_config_removed,
            "mask_zero": isolation_status.loading_mask_zero,
        });
        // The security property is the live H5PL mask: the host must have
        // disabled dynamic plugin loading itself through the API before any
        // HDF5 call. The packaging harness separately asserts that the
        // packaged EXE was launched without a parent-injected
        // HDF5_PLUGIN_PRELOAD and with hostile HDF5 startup values that the
        // app must sanitise.
        if hdf5_plugin_isolation.get("established") != Some(&serde_json::json!(true))
            || hdf5_plugin_isolation.get("loading_disabled") != Some(&serde_json::json!(true))
            || hdf5_plugin_isolation.get("startup_environment_sanitized")
                != Some(&serde_json::json!(true))
        {
            return Err("hdf5_plugin_isolation:not_established_by_host".to_owned());
        }

        Ok(serde_json::json!({
            "ok": true,
            "runtime_smoke": true,
            "application_service_ready": application_service_ready,
            "worker_protocol_ready": worker_protocol_ready,
            "hdf5_plugin_isolation": hdf5_plugin_isolation,
            "application_service": {
                "readiness": readiness,
                "provider_capabilities": provider_capabilities,
                "restore_completed": true,
                "restore": restore,
                "capability_checks": {
                    "aoi_import": true,
                    "administrative_boundary_search": true,
                    "gacos_plan": true,
                    "gacos_dry_run": true,
                },
                "nisar_subset_capability": nisar_subset_capability,
                "nisar_subset_task_rejected": {
                    "rejected": true,
                    "reason_code": nisar_subset_task_rejection,
                },
            },
            "compatibility_worker": {
                "declared_capabilities": worker_capabilities,
                "probe": worker_probe,
                "operation_checks": {
                    "aoi_import": true,
                    "dem_inspect": true,
                },
                "lifecycle": worker_lifecycle,
            },
        }))
    })();

    let worker_shutdown = worker.shutdown().is_ok();
    let application_shutdown = state.shutdown().is_ok();
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{}", smoke_store.display(), suffix));
    }

    result.map(|mut payload| {
        payload["shutdown"] = serde_json::json!({
            "application_service": application_shutdown,
            "compatibility_worker": worker_shutdown,
        });
        payload
    })
}

pub fn run() -> tauri::Result<()> {
    let native_gis = insar_gis::LazyNativeGisEngine::discover_product();
    let capability_observations = application::native_gis_capability_observations(&native_gis);
    run_desktop_with_application_service(
        application::ApplicationServiceState::with_store_path_and_runtime_observations_and_lazy_native_gis(
            insar_application_service::default_store_path(),
            capability_observations,
            native_gis,
        ),
        None,
        ui_smoke::UiSmokeState::disabled(),
    )
}

/// Run one phase of the packaged two-phase UI smoke against an isolated
/// Store supplied by the packaging harness.
///
/// Phase A binds the AOI through the formal Tauri command, confirms the Store
/// write and shuts down. Phase B restarts on the same Store, restores the AOI
/// without rebinding, runs a real loopback catalogue search, and submits the
/// complete report. The harness drives both phases as separate packaged EXE
/// launches.
pub fn run_ui_smoke_test(
    phase: ui_smoke::UiSmokePhase,
    smoke_store: std::path::PathBuf,
) -> Result<(), String> {
    let loopback = insar_application_service::UiSmokeLoopbackCatalog::new()
        .map_err(|error| format!("tauri_ui_smoke_loopback:{error}"))?;
    let ui_smoke_state = match phase {
        ui_smoke::UiSmokePhase::PhaseA => ui_smoke::UiSmokeState::phase_a(),
        ui_smoke::UiSmokePhase::PhaseB => ui_smoke::UiSmokeState::enabled(),
    };
    let compat_worker = std::sync::Arc::new(compat_worker::CompatWorkerState::ui_smoke());
    let application_state =
        application::ApplicationServiceState::with_ui_smoke_loopback_and_compat_worker(
            smoke_store.clone(),
            loopback,
            std::sync::Arc::clone(&compat_worker),
        );
    run_desktop_with_application_service(
        application_state,
        Some(compat_worker),
        ui_smoke_state.clone(),
    )
    .map_err(|error| format!("tauri_ui_smoke_runtime:{error}"))?;
    match phase {
        ui_smoke::UiSmokePhase::PhaseA => {
            if !ui_smoke_state.phase_a_completed() {
                return Err("tauri_ui_smoke_runtime:UI_SMOKE_PHASE_A_NO_COMPLETION".to_owned());
            }
        }
        ui_smoke::UiSmokePhase::PhaseB => {
            if ui_smoke_state.report().is_none() {
                return Err("tauri_ui_smoke_runtime:UI_SMOKE_NO_REPORT".to_owned());
            }
        }
    }
    Ok(())
}

/// The main window's handle, where a close aimed at tao's event target window
/// is forwarded (downloader enhancement D1d, see event_target_guard).
fn main_window_close_target<R: tauri::Runtime>(app: &tauri::App<R>) -> Option<isize> {
    #[cfg(windows)]
    {
        app.get_webview_window("main")
            .and_then(|window| window.hwnd().ok())
            .map(|hwnd| hwnd.0 as isize)
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        None
    }
}

/// Exit code of a host whose main window could not be created.
const MAIN_WINDOW_UNAVAILABLE_EXIT_CODE: i32 = 4;

/// Environment switch for automation: a startup failure is reported on stderr
/// and in the log only, never in a modal dialog.
const NO_STARTUP_ERROR_DIALOG_ENV: &str = "EOSTAGE_NO_STARTUP_ERROR_DIALOG";

/// What the setup can observe about the main window once the configured windows
/// were created.
trait MainWindowProbe {
    fn present(&self) -> bool;
    fn window_answers(&self) -> Result<(), String>;
    fn webview_answers(&self) -> Result<(), String>;
}

struct TauriMainWindowProbe<R: tauri::Runtime>(Option<tauri::WebviewWindow<R>>);

impl<R: tauri::Runtime> MainWindowProbe for TauriMainWindowProbe<R> {
    fn present(&self) -> bool {
        self.0.is_some()
    }

    fn window_answers(&self) -> Result<(), String> {
        self.0.as_ref().map_or(Ok(()), |window| {
            window
                .inner_size()
                .map(|_| ())
                .map_err(|error| error.to_string())
        })
    }

    fn webview_answers(&self) -> Result<(), String> {
        self.0.as_ref().map_or(Ok(()), |window| {
            window.url().map(|_| ()).map_err(|error| error.to_string())
        })
    }
}

/// L5 recheck 2026-10-19 (experiment C): when WebView2 cannot be initialised
/// (for example its data folder cannot be written), tauri-runtime-wry only logs
/// the window creation error and the application keeps running without any
/// window: an invisible process that holds the Store lock (the main window and
/// WebView2 disappeared after about 7 s; process 17224 stayed idle with an empty
/// stderr). After the configured windows were created, the main window and its
/// webview must answer.
fn main_window_failure(probe: &dyn MainWindowProbe) -> Option<String> {
    if !probe.present() {
        return Some("the main window was not created".to_owned());
    }
    if let Err(error) = probe.window_answers() {
        return Some(format!("the main window is not available: {error}"));
    }
    if let Err(error) = probe.webview_answers() {
        return Some(format!("the main webview is not available: {error}"));
    }
    None
}

/// Where a startup failure is reported.
trait StartupFailureSink {
    fn stderr(&mut self, line: &str);
    fn log(&mut self, line: &str);
    fn dialog(&mut self, message: &str);
}

/// The modal dialog is for a person at the real desktop only: automation (the
/// environment switch) and the UI smoke get stderr and the log.
fn startup_error_dialog_allowed(switch: Option<std::ffi::OsString>, ui_smoke: bool) -> bool {
    switch.is_none() && !ui_smoke
}

fn startup_failure_dialog_text(reason: &str) -> String {
    format!(
        "EOStage 无法打开主窗口：WebView2 初始化失败（例如 WebView2 数据目录无法写入或没有权限）。程序将退出，已有任务与断点均已保留。\n\n详细信息：{reason}"
    )
}

/// Reports a missing main window and returns the exit code of the process.
fn report_main_window_failure(
    reason: &str,
    dialog_allowed: bool,
    sink: &mut dyn StartupFailureSink,
) -> i32 {
    let detail = format!("DESKTOP_MAIN_WINDOW_UNAVAILABLE {reason}");
    sink.stderr(&detail);
    sink.log(&detail);
    if dialog_allowed {
        sink.dialog(&startup_failure_dialog_text(reason));
    }
    MAIN_WINDOW_UNAVAILABLE_EXIT_CODE
}

/// The desktop sink: stderr, `startup-failure.log` in the application log
/// directory and a native message box.
struct DesktopStartupFailureSink {
    log_path: Option<std::path::PathBuf>,
}

impl StartupFailureSink for DesktopStartupFailureSink {
    fn stderr(&mut self, line: &str) {
        eprintln!("{line}");
    }

    fn log(&mut self, line: &str) {
        let Some(path) = self.log_path.as_ref() else {
            return;
        };
        if path
            .parent()
            .is_some_and(|directory| std::fs::create_dir_all(directory).is_err())
        {
            return;
        }
        use std::io::Write as _;
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let unix_seconds = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_secs())
                .unwrap_or(0);
            let _ = writeln!(file, "unix={unix_seconds} {line}");
        }
    }

    fn dialog(&mut self, message: &str) {
        show_startup_error_dialog(message);
    }
}

/// Reports the failure (see `report_main_window_failure`), releases the
/// Application Service with its Store lease and the compatibility Worker as a
/// normal close does, and ends the process with a non-zero code.
fn end_without_main_window<R: tauri::Runtime>(app: &tauri::App<R>, reason: &str) -> ! {
    let ui_smoke = app
        .try_state::<ui_smoke::UiSmokeState>()
        .is_some_and(|state| state.is_enabled());
    let dialog_allowed =
        startup_error_dialog_allowed(std::env::var_os(NO_STARTUP_ERROR_DIALOG_ENV), ui_smoke);
    let mut sink = DesktopStartupFailureSink {
        log_path: app
            .path()
            .app_log_dir()
            .ok()
            .map(|directory| directory.join("startup-failure.log")),
    };
    let exit_code = report_main_window_failure(reason, dialog_allowed, &mut sink);
    if let Some(state) = app.try_state::<application::ApplicationServiceState>() {
        let _ = state.shutdown();
    }
    if let Some(worker) = app.try_state::<std::sync::Arc<compat_worker::CompatWorkerState>>() {
        let _ = worker.shutdown();
    }
    std::process::exit(exit_code);
}

fn show_startup_error_dialog(message: &str) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
        let text: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
        let title: Vec<u16> = "EOStage 启动失败".encode_utf16().chain(Some(0)).collect();
        // SAFETY: both strings are NUL-terminated and outlive the modal call;
        // a null owner window is allowed.
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                text.as_ptr(),
                title.as_ptr(),
                MB_OK | MB_ICONERROR,
            );
        }
    }
    #[cfg(not(windows))]
    let _ = message;
}

#[cfg(target_os = "macos")]
fn hide_macos_standard_window_buttons(ns_window: &objc2_app_kit::NSWindow) {
    use objc2_app_kit::NSWindowButton;
    for kind in [
        NSWindowButton::CloseButton,
        NSWindowButton::MiniaturizeButton,
        NSWindowButton::ZoomButton,
    ] {
        if let Some(button) = ns_window.standardWindowButton(kind) {
            button.setHidden(true);
        }
    }
}

#[cfg(target_os = "macos")]
fn hide_macos_standard_window_buttons_for_window(window: &tauri::Window) {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSView;

    if MainThreadMarker::new().is_none() {
        return;
    }
    if let Ok(ptr) = window.ns_view() {
        if !ptr.is_null() {
            // SAFETY: Non-null AppKit NSView pointer on main thread during window resize/fullscreen exit.
            let view: &NSView = unsafe { &*(ptr as *const NSView) };
            if let Some(win) = view.window() {
                hide_macos_standard_window_buttons(&win);
            }
        }
    }
}

fn apply_native_rounded_window_corners(window: &tauri::WebviewWindow) {
    #[cfg(target_os = "macos")]
    {
        use objc2::MainThreadMarker;
        use objc2_app_kit::{NSView, NSWindowStyleMask, NSWindowTitleVisibility};

        if MainThreadMarker::new().is_none() {
            return;
        }
        let Ok(ns_view_ptr) = window.ns_view() else {
            return;
        };
        if ns_view_ptr.is_null() {
            return;
        }
        // SAFETY: Non-null AppKit NSView pointer obtained from Tauri on the main thread.
        let ns_view: &NSView = unsafe { &*(ns_view_ptr as *const NSView) };
        let Some(ns_window) = ns_view.window() else {
            return;
        };
        let prev_responder = ns_window.firstResponder();
        ns_window.setStyleMask(
            NSWindowStyleMask::Titled
                | NSWindowStyleMask::FullSizeContentView
                | NSWindowStyleMask::Closable
                | NSWindowStyleMask::Miniaturizable
                | NSWindowStyleMask::Resizable,
        );
        ns_window.makeFirstResponder(prev_responder.as_deref().or(Some(ns_view)));
        ns_window.setTitlebarAppearsTransparent(true);
        ns_window.setTitleVisibility(NSWindowTitleVisibility::Hidden);
        hide_macos_standard_window_buttons(&ns_window);
        ns_window.setHasShadow(true);
        ns_window.invalidateShadow();
    }
    #[cfg(target_os = "windows")]
    {
        if let Ok(hwnd) = window.hwnd() {
            let preference: u32 = windows_sys::Win32::Graphics::Dwm::DWMWCP_ROUND as u32;
            let _ = unsafe {
                windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute(
                    hwnd.0 as windows_sys::Win32::Foundation::HWND,
                    windows_sys::Win32::Graphics::Dwm::DWMWA_WINDOW_CORNER_PREFERENCE as u32,
                    (&preference as *const u32).cast(),
                    std::mem::size_of::<u32>() as u32,
                )
            };
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = window;
    }
}

/// The only formal desktop runtime. It owns one business Application Service,
/// one Store lease, and the complete named command surface.
fn run_desktop_with_application_service(
    state: application::ApplicationServiceState,
    compat_worker: Option<std::sync::Arc<compat_worker::CompatWorkerState>>,
    ui_smoke_state: ui_smoke::UiSmokeState,
) -> tauri::Result<()> {
    let agent_state = agent::AgentHostState::new(state.agent_application());
    let mut builder = tauri::Builder::default()
        .manage(state)
        .manage(agent_state)
        .manage(commands::SavedWindowBounds::default());
    if let Some(compat_worker) = compat_worker {
        builder = builder.manage(compat_worker);
    }
    builder
        .manage(ui_smoke_state)
        .setup(|app| {
            // L5 recheck 2026-10-19 (experiment C): the configured windows were
            // just created; without a live main window, end instead of running
            // invisibly (see main_window_failure).
            if let Some(reason) =
                main_window_failure(&TauriMainWindowProbe(app.get_webview_window("main")))
            {
                end_without_main_window(app, &reason);
            }
            if let Some(main_window) = app.get_webview_window("main") {
                apply_native_rounded_window_corners(&main_window);
            }
            // Downloader enhancement D1c: an external WM_CLOSE must not destroy
            // tao's event target window; since D1d it closes the main window
            // through the normal close flow instead (see event_target_guard).
            if cfg!(windows)
                && event_target_guard::guard_event_target_windows(main_window_close_target(app))
                    == 0
            {
                eprintln!("DESKTOP_EVENT_TARGET_GUARD_MISSING no tao event target window found");
            }
            let startup_service_state = app.state::<application::ApplicationServiceState>();
            let startup_data_dir = application::resolve_app_data_dir(&startup_service_state);
            crate::diagnostics::install_panic_hook(
                startup_data_dir.clone(),
                app.package_info().version.to_string(),
            );
            if let Err(err) =
                crate::transfer_engine_commands::reconcile_startup_transfer_prefs_with_service(
                    &startup_data_dir,
                    Some(&*startup_service_state),
                )
            {
                eprintln!("DESKTOP_TRANSFER_PREFS_LOAD_WARNING {err}");
            }
            let ui_smoke_state = app.state::<ui_smoke::UiSmokeState>().inner().clone();
            if ui_smoke_state.is_enabled() {
                let phase = ui_smoke_state.phase();
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(60));
                    let completed = match phase {
                        ui_smoke::UiSmokePhase::PhaseA => ui_smoke_state.phase_a_completed(),
                        ui_smoke::UiSmokePhase::PhaseB => ui_smoke_state.report().is_some(),
                    };
                    if !completed {
                        eprintln!(
                            "Tauri UI smoke watchdog incomplete; last checkpoint: {}",
                            ui_smoke_state
                                .last_checkpoint()
                                .as_deref()
                                .unwrap_or("none")
                        );
                        handle.exit(1);
                        // Tauri's requested exit code is not reliably surfaced
                        // by every packaged WebView2 shutdown path. The smoke
                        // harness must see a truthful non-zero process result.
                        std::process::exit(1);
                    }
                });
            }
            #[cfg(target_os = "macos")]
            macos_cdp_bridge::start_if_requested(app.handle().clone());
            if std::env::var_os("RUST_LOG").is_some() {
                eprintln!(
                    "[EOStage][startup] pid={} version={}",
                    std::process::id(),
                    env!("CARGO_PKG_VERSION")
                );
            }
            let agent_emit_handle = app.handle().clone();
            app.state::<agent::AgentHostState>()
                .set_notifier(std::sync::Arc::new(
                    move |note: agent::AgentHostNotification| match note {
                        agent::AgentHostNotification::StreamEvent { session_id, event } => {
                            emit_host_event(
                                &agent_emit_handle,
                                "agent:stream-event",
                                serde_json::json!({
                                    "session_id": session_id,
                                    "seq": event.seq,
                                    "kind": event.kind,
                                    "event": event,
                                }),
                            );
                        }
                        agent::AgentHostNotification::LiveDelta {
                            session_id,
                            run_id,
                            step_index,
                            kind,
                            delta,
                            started_at_ms,
                            duration_ms,
                        } => {
                            emit_host_event(
                                &agent_emit_handle,
                                "agent:live-delta",
                                serde_json::json!({
                                    "session_id": session_id,
                                    "run_id": run_id,
                                    "step_index": step_index,
                                    "kind": kind,
                                    "delta": delta,
                                    "started_at_ms": started_at_ms,
                                    "duration_ms": duration_ms,
                                }),
                            );
                        }
                        agent::AgentHostNotification::AoiChanged {
                            session_id,
                            run_id,
                            state,
                            display_name,
                            bbox,
                        } => {
                            emit_host_event(
                                &agent_emit_handle,
                                "workbench:aoi-changed",
                                serde_json::json!({
                                    "session_id": session_id,
                                    "run_id": run_id,
                                    "state": state,
                                    "display_name": display_name,
                                    "bbox": bbox,
                                    "updated_by": "agent",
                                }),
                            );
                        }
                        agent::AgentHostNotification::SearchUpdated {
                            session_id,
                            run_id,
                            request_id,
                            batch_id,
                        } => {
                            emit_host_event(
                                &agent_emit_handle,
                                "workbench:search-updated",
                                serde_json::json!({
                                    "session_id": session_id,
                                    "run_id": run_id,
                                    "request_id": request_id,
                                    "batch_id": batch_id,
                                    "updated_by": "agent",
                                }),
                            );
                        }
                        agent::AgentHostNotification::WorkspaceSync {
                            session_id,
                            run_id,
                            payload,
                        } => {
                            if payload.get("action").and_then(|v| v.as_str()) == Some("search_updated") {
                                emit_host_event(
                                    &agent_emit_handle,
                                    "workbench:search-updated",
                                    serde_json::json!({
                                        "session_id": session_id,
                                        "run_id": run_id,
                                        "request_id": payload.get("request_id").cloned().unwrap_or(serde_json::Value::Null),
                                        "batch_id": payload.get("batch_id").cloned().unwrap_or(serde_json::Value::Null),
                                        "loaded_count": payload.get("loaded_count").cloned().unwrap_or(serde_json::Value::Null),
                                        "total_count": payload.get("total_count").cloned().unwrap_or(serde_json::Value::Null),
                                        "complete": payload.get("complete").cloned().unwrap_or(serde_json::Value::Null),
                                        "updated_by": "background_search",
                                    }),
                                );
                            }
                            emit_host_event(
                                &agent_emit_handle,
                                "agent:workspace-sync",
                                serde_json::json!({
                                    "session_id": session_id,
                                    "run_id": run_id,
                                    "payload": payload,
                                }),
                            );
                        }
                    },
                ));
            Ok(())
        })
        .on_window_event(|window, event| {
            match event {
                tauri::WindowEvent::CloseRequested { .. } => {
                    #[cfg(target_os = "macos")]
                    macos_cdp_bridge::shutdown();
                    if let Some(state) = window
                        .app_handle()
                        .try_state::<application::ApplicationServiceState>()
                    {
                        let _ = state.shutdown();
                    }
                    if let Some(worker) = window
                        .app_handle()
                        .try_state::<std::sync::Arc<compat_worker::CompatWorkerState>>()
                    {
                        let _ = worker.shutdown();
                    }
                }
                tauri::WindowEvent::DragDrop(drop_event) => {
                    let scale_factor = window.scale_factor().unwrap_or(1.0);
                    let payload = match drop_event {
                        tauri::DragDropEvent::Enter { paths, position } => Some(serde_json::json!({
                            "kind": "hover",
                            "paths": paths.iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>(),
                            "position": { "x": position.x, "y": position.y },
                            "scaleFactor": scale_factor,
                        })),
                        tauri::DragDropEvent::Over { position } => Some(serde_json::json!({
                            "kind": "over",
                            "paths": serde_json::Value::Array(Vec::new()),
                            "position": { "x": position.x, "y": position.y },
                            "scaleFactor": scale_factor,
                        })),
                        tauri::DragDropEvent::Drop { paths, position } => Some(serde_json::json!({
                            "kind": "drop",
                            "paths": paths.iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>(),
                            "position": { "x": position.x, "y": position.y },
                            "scaleFactor": scale_factor,
                        })),
                        tauri::DragDropEvent::Leave => Some(serde_json::json!({
                            "kind": "cancel",
                            "paths": serde_json::Value::Array(Vec::new()),
                            "position": serde_json::Value::Null,
                            "scaleFactor": scale_factor,
                        })),
                        _ => None,
                    };
                    if let Some(payload) = payload {
                        emit_host_event(window.app_handle(), "workbench:file-drop", payload);
                    }
                }
                tauri::WindowEvent::Resized(_) => {
                    #[cfg(target_os = "macos")]
                    hide_macos_standard_window_buttons_for_window(window);
                }
                _ => {}
            }
        })
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            commands::shell_app_info,
            commands::shell_window_toggle_maximize,
            commands::application_runtime_components_inspect,
            commands::application_extension_registry_fetch,
            commands::application_runtime_component_install,
            commands::application_plugins_status,
            commands::plugin_list,
            commands::application_plugin_install,
            commands::application_plugin_set_enabled,
            commands::application_plugin_uninstall,
            commands::application_custom_connector_save,
            commands::application_custom_connector_delete,
            commands::application_custom_connector_probe_tools,
            commands::application_custom_skill_save,
            commands::application_custom_skill_delete,
            commands::application_install_custom_skill_from_url,
            commands::application_install_custom_skill_from_folder,
            commands::application_usage_statistics,
            commands::get_cache_summary,
            commands::clear_disposable_caches,
            commands::application_gis_raster_inspect,
            commands::application_gis_raster_pixel_probe,
            commands::application_gis_raster_process,
            commands::application_gis_vector_process,
            commands::application_external_gis_inspect,
            commands::application_external_gis_save_prefs,
            commands::application_external_gis_execute,
            commands::application_conda_install_geo_package,
            commands::application_lbs_coord_transform,
            commands::application_mapgis_inspect,
            commands::application_mapgis_convert,
            commands::application_open_geodata_query,
            commands::geodata_fetch_preview_data_url,
            commands::application_experiment_project_get,
            commands::application_experiment_project_add_task,
            commands::application_experiment_project_remove_item,
            commands::application_experiment_project_export,
            commands::application_service_readiness,
            commands::application_core_restore,
            commands::application_task_overview,
            commands::application_workbench_page,
            commands::application_batch_page,
            commands::application_download_task_page,
            commands::application_load_task_source_batch_page,
            commands::application_download_history_page,
            commands::application_download_task_event_page,
            commands::application_open_download_task_output,
            commands::application_list_task_conflict_files,
            commands::application_delete_task_conflict_files,
            commands::application_open_external_url,
            commands::application_aoi_import,
            commands::application_aoi_normalize,
            commands::application_admin_boundary_search,
            commands::application_admin_boundary_resolve,
            commands::application_admin_options,
            commands::application_geo_boundary_countries,
            commands::application_geo_boundary_levels,
            commands::application_geo_boundary_units,
            commands::application_geo_boundary_resolve,
            commands::application_gacos_plan,
            commands::application_gacos_dry_run,
            commands::application_gacos_submit_request,
            commands::application_gacos_request_status,
            commands::application_gacos_ledger,
            commands::application_gacos_import_result,
            commands::application_gacos_create_import_task,
            commands::application_gacos_create_result_task,
            commands::application_gacos_supply_result_link,
            commands::application_report_generate,
            commands::application_update_workbench_selection,
            commands::application_workspace_open,
            commands::application_workspace_create,
            commands::application_workspace_prepare_location,
            commands::application_project_create,
            commands::application_project_select,
            commands::application_region_create,
            commands::application_region_select,
            commands::credential_status,
            commands::credential_bhoonidhi_auth,
            commands::credential_bhoonidhi_session_status,
            #[cfg(feature = "bhoonidhi-auth-revoke-acceptance")]
            commands::acceptance_bhoonidhi_auth_generation,
            #[cfg(feature = "bhoonidhi-auth-revoke-acceptance")]
            commands::acceptance_bhoonidhi_auth_revoke_once,
            commands::credential_reveal,
            commands::credential_read_for_edit,
            commands::credential_agent_channel_key_read,
            commands::credential_update_field,
            commands::credential_earthdata_auth,
            commands::credential_save_cdse_login,
            commands::credential_clear_cdse,
            commands::credential_save_usgs_login,
            commands::credential_clear_usgs,
            commands::credential_save_earthdata_token,
            commands::credential_save_earthdata_login,
            commands::credential_clear_earthdata,
            commands::credential_save_bhoonidhi_login,
            commands::credential_clear_bhoonidhi,
            commands::credential_save_opentopography_key,
            commands::credential_clear_opentopography_key,
            commands::credential_save_gacos_email,
            commands::credential_clear_gacos_email,
            commands::application_bind_region_aoi,
            commands::application_clear_region_aoi,
            commands::application_set_working_aoi,
            commands::application_clear_working_aoi,
            commands::application_working_aoi,
            commands::application_plan_dem_task,
            commands::application_create_dem_task,
            commands::application_set_dem_source_identity,
            commands::application_resolve_dem_conversion_source,
            commands::application_handoff_dem_conversion,
            commands::application_get_dem_task_settings,
            commands::application_save_dem_task_settings,
            commands::application_get_sar_download_policy_settings,
            commands::application_save_sar_download_policy_settings,
            commands::application_get_network_settings,
            commands::application_save_network_settings,
            commands::application_test_network_connection,
            commands::application_probe_provider_connectivity,
            commands::application_standalone_dem,
            commands::application_create_standalone_dem_task,
            commands::application_import_scenes,
            commands::application_match_orbits_directory,
            commands::application_create_orbit_task,
            commands::application_create_standalone_orbit_task,
            commands::application_create_companion_orbit_task,
            commands::application_list_sources,
            commands::application_source_views,
            commands::application_provider_capabilities,
            commands::application_effective_capability,
            commands::application_search_start,
            commands::application_search_progress,
            commands::application_search_cancel,
            commands::application_sar_search_and_freeze,
            commands::application_sar_search_progress,
            commands::application_sar_background_count,
            commands::application_catalog_search_and_freeze,
            commands::application_catalog_intent_search_and_freeze,
            commands::application_cancel_request,
            commands::application_cancel_catalog_search,
            commands::application_account_session_statuses,
            commands::application_cdse_session_status,
            commands::application_cdse_login,
            commands::application_cdse_logout,
            commands::application_cdse_network_policy,
            commands::application_set_cdse_network_policy,
            commands::application_cdse_network_diagnostic,
            commands::application_preview_asset,
            commands::application_save_preview_asset,
            commands::application_cdse_quicklook,
            commands::application_sentinel2_map_preview,
            commands::application_usgs_session_status,
            commands::application_usgs_login,
            commands::application_usgs_logout,
            commands::application_landsat_browse,
            commands::application_landsat_asset_details,
            commands::application_bhoonidhi_asset_size,
            commands::application_create_landsat_bundle_task,
            commands::application_create_landsat_asset_task,
            commands::application_create_landsat_asset_batch_task,
            commands::application_create_landsat_bundle_batch_task,
            commands::application_task_delivery_admission,
            commands::application_optical_preparation_progress,
            commands::application_create_reviewed_mpc_asset_task,
            commands::application_create_sentinel2_archive_batch_task,
            commands::application_create_sentinel2_archive_task,
            commands::application_create_sentinel2_band_task,
            commands::application_create_sentinel2_band_batch_task,
            commands::application_create_sentinel2_l1c_band_batch_task,
            commands::application_create_hls_band_batch_task,
            commands::application_precheck_optical_aoi_coverage,
            commands::application_create_optical_derived_task,
            commands::application_nisar_remote_subset_capability,
            commands::application_create_nisar_remote_subset_task,
            commands::application_prepare_nisar_subset_intent_v2,
            commands::application_confirm_nisar_subset_task_v2,
            commands::application_preflight_nisar_subset_batch_v2,
            commands::application_prepare_nisar_subset_batch_v2,
            commands::application_confirm_nisar_subset_batch_task_v2,
            commands::application_append_nisar_subset_batch_task_v2,
            commands::application_read_nisar_subset_append_context_v2,
            commands::application_preflight_nisar_subset_append_v2,
            commands::application_create_nisar_subset_successor_task_v2,
            commands::application_read_nisar_archived_recovery_context_v2,
            commands::application_create_nisar_archived_recovery_v2,
            commands::application_resume_nisar_subset_task,
            commands::application_start_newly_confirmed_task,
            commands::application_update_search_workbench,
            commands::application_create_task,
            commands::application_preview_all_match_task,
            commands::application_create_all_match_task,
            commands::application_create_download_task,
            commands::application_append_download_task,
            commands::application_create_task_from_workbench,
            commands::application_append_task_from_workbench,
            commands::application_query_source_append_status,
            commands::application_query_historical_successor_status,
            commands::application_query_burst_append_status,
            commands::application_query_burst_successor_status,
            commands::application_query_nisar_append_status,
            commands::application_query_nisar_successor_status,
            commands::application_query_nisar_archived_recovery_status,
            commands::application_query_reacquire_status,
            commands::application_append_task_from_source_batch,
            commands::application_create_task_from_historical_source_batch,
            commands::application_create_burst_asset_task,
            commands::application_append_burst_asset_task,
            commands::application_create_historical_burst_asset_task,
            commands::application_task_action,
            commands::application_task_retry_fallback,
            commands::application_update_download_task,
            commands::application_update_download_task_view,
            commands::application_archive_download_task,
            commands::application_delete_download_history,
            commands::application_task_item_action,
            commands::application_task_delete_history,
            commands::application_task_set_pinned,
            commands::application_list_pinned_tasks,
            commands::application_task_retry_fallback_eligibility,
            commands::check_existing_downloaded_scenes,
            commands::agent_sessions_list,
            commands::agent_sessions_create,
            commands::agent_sessions_delete,
            commands::agent_sessions_rename,
            commands::agent_sessions_set_pinned,
            commands::agent_sessions_set_archived,
            commands::agent_sessions_move,
            commands::agent_sessions_export,
            commands::agent_sessions_export_to,
            commands::agent_workspaces_list,
            commands::agent_workspaces_create,
            commands::agent_workspaces_update,
            commands::agent_workspaces_delete,
            commands::agent_send,
            commands::agent_stream_poll,
            commands::agent_plan_approve,
            commands::agent_plan_reject,
            commands::agent_run_cancel,
            commands::agent_runs_list,
            commands::agent_session_work,
            commands::agent_session_truncate,
            commands::agent_session_workspace_state,
            commands::agent_session_set_working_aoi,
            commands::agent_runs_open,
            commands::agent_channels_get,
            commands::agent_disabled_items_get,
            commands::agent_disabled_items_set,
            commands::agent_channels_save,
            commands::agent_channels_test,
            commands::agent_channels_models,
            commands::agent_channels_probe_draft,
            commands::agent_channels_test_draft,
            commands::agent_channels_set_default,
            commands::agent_channels_delete,
            commands::agent_channel_rename,
            commands::agent_sync_public_model_registry,
            commands::agent_oauth_open_browser,
            commands::agent_oauth_device_start,
            commands::agent_oauth_device_poll,
            commands::agent_oauth_pkce_start,
            commands::agent_oauth_pkce_complete,
            commands::agent_oauth_channels_meta,
            commands::agent_oauth_channel_quota,
            commands::agent_permission_mode_get,
            commands::agent_permission_mode_set_global,
            commands::agent_permission_mode_set_session,
            commands::agent_stop_all,
            commands::agent_activity_status,
            commands::eostage_account_status,
            commands::eostage_account_login_start,
            commands::eostage_account_login_poll,
            commands::eostage_account_login_complete_code,
            commands::eostage_account_logout,
            commands::eostage_account_sync_models,
            commands::eostage_billing_config,
            commands::eostage_billing_redeem,
            commands::eostage_billing_orders_list,
            commands::eostage_billing_order_create,
            commands::eostage_billing_order_check,
            commands::eostage_billing_order_cancel,
            commands::eostage_usage_list,
            commands::desktop_onboarding_state_get,
            commands::desktop_onboarding_state_set,
            commands::desktop_active_resource_get,
            commands::desktop_active_resource_set,
            commands::shell_diagnostics_get_state,
            commands::shell_diagnostics_set_enabled,
            commands::shell_diagnostics_record,
            commands::shell_diagnostics_list,
            commands::shell_diagnostics_clear,
            commands::shell_diagnostics_upload,
            commands::shell_check_app_update,
            transfer_engine_commands::transfer_engine_get_prefs,
            transfer_engine_commands::transfer_engine_save_prefs,
            commands::ui_smoke_status,
            commands::ui_smoke_report,
            commands::ui_smoke_phase,
            commands::ui_smoke_checkpoint,
            commands::ui_smoke_phase_a_complete,
            commands::ui_smoke_captured_search_query,
        ])
        .run(tauri::generate_context!())
}

#[cfg(test)]
mod tests {
    use super::is_allowed_desktop_origin;

    /// What the setup observes after tauri-runtime-wry swallowed a failed
    /// window or WebView2 creation (L5 recheck 2026-10-19, experiment C).
    struct MainWindow {
        present: bool,
        window: Result<(), String>,
        webview: Result<(), String>,
    }

    impl super::MainWindowProbe for MainWindow {
        fn present(&self) -> bool {
            self.present
        }

        fn window_answers(&self) -> Result<(), String> {
            self.window.clone()
        }

        fn webview_answers(&self) -> Result<(), String> {
            self.webview.clone()
        }
    }

    #[test]
    fn a_main_window_that_was_not_created_or_does_not_answer_is_a_startup_failure() {
        let live = MainWindow {
            present: true,
            window: Ok(()),
            webview: Ok(()),
        };
        assert_eq!(super::main_window_failure(&live), None);
        let missing = MainWindow {
            present: false,
            ..live
        };
        assert_eq!(
            super::main_window_failure(&missing).as_deref(),
            Some("the main window was not created")
        );
        let destroyed = MainWindow {
            present: true,
            window: Err("failed to receive message from webview".to_owned()),
            webview: Ok(()),
        };
        assert_eq!(
            super::main_window_failure(&destroyed).as_deref(),
            Some("the main window is not available: failed to receive message from webview")
        );
        let no_webview = MainWindow {
            present: true,
            window: Ok(()),
            webview: Err("webview not found".to_owned()),
        };
        assert_eq!(
            super::main_window_failure(&no_webview).as_deref(),
            Some("the main webview is not available: webview not found")
        );
    }

    #[derive(Default)]
    struct RecordedReport {
        stderr: Vec<String>,
        log: Vec<String>,
        dialogs: Vec<String>,
    }

    impl super::StartupFailureSink for RecordedReport {
        fn stderr(&mut self, line: &str) {
            self.stderr.push(line.to_owned());
        }

        fn log(&mut self, line: &str) {
            self.log.push(line.to_owned());
        }

        fn dialog(&mut self, message: &str) {
            self.dialogs.push(message.to_owned());
        }
    }

    #[test]
    fn a_missing_main_window_is_logged_and_ends_with_code_4_and_only_people_get_a_dialog() {
        let mut automated = RecordedReport::default();
        assert_eq!(
            super::report_main_window_failure(
                "the main window was not created",
                false,
                &mut automated
            ),
            4
        );
        let detail = "DESKTOP_MAIN_WINDOW_UNAVAILABLE the main window was not created";
        assert_eq!(automated.stderr, [detail]);
        assert_eq!(automated.log, [detail]);
        assert!(automated.dialogs.is_empty());

        let mut person = RecordedReport::default();
        assert_eq!(
            super::report_main_window_failure("the main window was not created", true, &mut person),
            4
        );
        assert_eq!(person.stderr, [detail]);
        assert_eq!(person.dialogs.len(), 1);
        assert!(person.dialogs[0].starts_with("EOStage 无法打开主窗口：WebView2 初始化失败"));
        assert!(person.dialogs[0].contains("已有任务与断点均已保留"));
        assert!(person.dialogs[0].ends_with("详细信息：the main window was not created"));
    }

    #[test]
    fn the_startup_dialog_is_only_shown_at_the_real_desktop() {
        assert!(super::startup_error_dialog_allowed(None, false));
        assert!(!super::startup_error_dialog_allowed(
            Some("1".into()),
            false
        ));
        assert!(!super::startup_error_dialog_allowed(Some("".into()), false));
        assert!(!super::startup_error_dialog_allowed(None, true));
    }

    fn url(value: &str) -> tauri::Url {
        value.parse().expect("valid test URL")
    }

    #[test]
    fn packaged_origins_are_accepted_without_development_mode() {
        for value in [
            "tauri://localhost/",
            "http://tauri.localhost/",
            "https://tauri.localhost/",
        ] {
            assert!(is_allowed_desktop_origin(&url(value), false));
        }
    }

    #[test]
    fn packaged_origins_require_the_exact_main_window_label() {
        assert!(super::is_allowed_desktop_window_label("main"));
        for label in ["", "settings", "main/other", "MAIN"] {
            assert!(!super::is_allowed_desktop_window_label(label));
        }
        assert!(!is_allowed_desktop_origin(
            &url("tauri://localhost:443/"),
            false
        ));
    }

    #[test]
    fn only_the_reviewed_loopback_origin_is_accepted_in_development() {
        assert!(is_allowed_desktop_origin(
            &url("http://127.0.0.1:5173/"),
            true
        ));
        for value in [
            "http://127.0.0.1:5174/",
            "http://localhost:5173/",
            "https://127.0.0.1:5173/",
            "http://0.0.0.0:5173/",
            "https://example.com/",
        ] {
            assert!(!is_allowed_desktop_origin(&url(value), true));
        }
    }

    #[test]
    fn loopback_is_not_accepted_in_production() {
        assert!(!is_allowed_desktop_origin(
            &url("http://127.0.0.1:5173/"),
            false
        ));
    }
}
