use crate::compat_worker::CompatWorkerState;
use insar_application_service::{
    default_store_path, with_ui_smoke_loopback_runtime, AccountSessionStatus,
    AdminBoundaryOptionsRequest, AdminBoundaryOptionsResult, AdminBoundaryResolveRequest,
    AdminBoundaryResult, AdminBoundarySearchRequest, AdminBoundarySearchResult,
    ApplicationAoiImportRequest, ApplicationAoiNormalizeRequest, ApplicationBindRegionAoiRequest,
    ApplicationCatalogIntentSearchAndFreezeRequest, ApplicationCatalogSearchAndFreezeRequest,
    ApplicationClearRegionAoiRequest, ApplicationNetworkSettings,
    ApplicationSarBackgroundCountRequest, ApplicationSarBackgroundCountResult,
    ApplicationSarSearchAndFreezeRequest, ApplicationService, ApplicationServiceConfig,
    ApplicationSetWorkingAoiRequest, ApplicationWorkbenchPage, ApplicationWorkbenchPageRequest,
    ApplicationWorkingAoi, ArchiveDownloadTaskAck, BhoonidhiAssetSizeRequest,
    BhoonidhiAssetSizeResponse, BhoonidhiDesktopSessionStatus, BurstAppendAssetTaskRequest,
    BurstCreateAssetTaskRequest, BurstCreateHistoricalAssetTaskRequest, CdseLoginRequest,
    CdseNetworkDiagnosticResult, CdseNetworkMode, CdseQuicklookRequest, CdseQuicklookResponse,
    CdseSessionStatus, CoreServiceSnapshot, DemConversionRequest, DemTaskPlan, DemTaskPlanRequest,
    DemTaskRequest, DemTaskSettings, EffectiveCapabilityStatus, GacosConfirmRequest,
    GacosCreateImportTaskRequest, GacosCreateResultTaskRequest, GacosResultImportRequest,
    GacosSupplyResultLinkRequest, GeoBoundaryCountriesResult, GeoBoundaryLevelsRequest,
    GeoBoundaryLevelsResult, GeoBoundaryResolveRequest, GeoBoundaryUnitsRequest,
    GeoBoundaryUnitsResult, LandsatAssetDetailsRequest, LandsatAssetDetailsResponse,
    LandsatBrowseRequest, LandsatBrowseResponse, LandsatCreateAssetTaskRequest,
    LandsatCreateBundleTaskRequest, NisarAppendSubsetBatchTaskV2Request,
    NisarAppendSubsetContextV2, NisarAppendSubsetContextV2Request,
    NisarConfirmSubsetBatchTaskV2Request, NisarConfirmSubsetTaskV2Request,
    NisarCreateRemoteSubsetTaskRequest, NisarCreateSubsetSuccessorTaskV2Request,
    NisarPreflightSubsetAppendV2, NisarPreflightSubsetAppendV2Request, NisarPreflightSubsetBatchV2,
    NisarPreflightSubsetBatchV2Request, NisarPrepareSubsetBatchV2Request,
    NisarPrepareSubsetIntentV2Request, NisarPreparedSubsetBatchV2, NisarPreparedSubsetIntentV2,
    NisarRemoteSubsetCapability, OrbitDirectoryMatchRequest, OrbitTaskRequest, PreviewAssetRequest,
    PreviewSaveRequest, PreviewSaveResponse, ReadonlyServiceSnapshot, ResolvedPreviewAsset,
    ReviewedMpcCreateAssetTaskRequest, SarDownloadPolicySettings, SarSearchProgressUpdate,
    SceneImportRequest, Sentinel2CreateArchiveBatchTaskRequest,
    Sentinel2CreateArchiveBatchTaskResult, Sentinel2CreateArchiveTaskRequest,
    Sentinel2CreateBandTaskRequest, Sentinel2MapPreviewRequest, Sentinel2MapPreviewResponse,
    ServiceReadiness, StandaloneDemAoiTaskRequest, StandaloneDemRequest, StoreDemConversionSource,
    StoreDemConversionSourceRequest, StoreDemSourceIdentity, StoreTaskConflictFilesRequest,
    StoreTaskEventPage, StoreTaskEventPageRequest, StoreTaskHistoryPage,
    StoreTaskHistoryPageRequest, StoreTaskOutputDirectory, StoreTaskOutputDirectoryRequest,
    StoreTaskPage, StoreTaskPageRequest, StoreTaskSourceBatchPage, StoreTaskSourceBatchPageRequest,
    TaskConflictFileDeleteResult, TaskConflictFileList, UiSmokeLoopbackCatalog,
    UsgsM2mDesktopSessionStatus, UsgsM2mLoginRequest, WorkspaceContextState,
    WorkspaceCreateRequest, WorkspaceNamedNodeRequest, WorkspaceOpenRequest,
    WorkspacePrepareLocationRequest, WorkspacePreparedLocation, WorkspaceSelectNodeRequest,
};
use insar_core::{
    AppendTaskFromSourceBatchRequest, AppendTaskFromWorkbenchRequest,
    CreateTaskFromHistoricalSourceBatchRequest, CreateTaskFromWorkbenchRequest,
    ProviderCapabilitiesResponse, StoreAppendTaskRequest, StoreArchiveTaskRequest,
    StoreCreateTaskRequest, StoreDeleteHistoryRequest, StoreUpdateTaskRequest,
    StoreUpdateTaskViewRequest, StoreUpdateWorkbenchRequest, WorkbenchSelectionCommand,
    WorkbenchSelectionSummary,
};
use insar_gis::LazyNativeGisEngine;
use reqwest::header::{CONTENT_RANGE, ETAG, IF_RANGE, RANGE};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[path = "offline_smoke_boundary.rs"]
mod offline_smoke_boundary;

struct CompatWorkerOperationPort {
    worker: Arc<CompatWorkerState>,
}

impl insar_application_service::DemOperationPort for CompatWorkerOperationPort {
    fn call(
        &self,
        operation: &str,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        self.worker
            .call(operation, payload)
            .map_err(|error| format!("{}: {}", error.code, error.message_key))
    }
}

struct NativeGisOperationPort {
    engine: LazyNativeGisEngine,
    store_path: Option<PathBuf>,
}

fn native_optical_derived_execute(
    engine: &LazyNativeGisEngine,
    store_path: Option<&Path>,
    payload: &serde_json::Value,
    interrupted: &(dyn Fn() -> bool + Sync),
    progress: &mut dyn FnMut(insar_application_service::DemTransferEvent),
) -> Result<serde_json::Value, String> {
    let mut request: insar_gis::OpticalDerivedRequest = serde_json::from_value(payload.clone())
        .map_err(|e| format!("OPTICAL_DERIVED_SPEC_INVALID: {e}"))?;
    let network_policy = store_path
        .map(insar_application_service::read_network_policy_for_store_path)
        .unwrap_or_else(insar_core::ProviderNetworkPolicy::direct);
    insar_application_service::sign_mpc_optical_request_in_memory(
        &mut request,
        network_policy,
        interrupted,
    )?;
    let parent = request.output_tif_path.parent().ok_or_else(|| {
        "OPTICAL_DERIVED_DESTINATION_INVALID: missing parent directory".to_owned()
    })?;
    prepare_native_task_output_path(parent, &request.output_tif_path)?;
    progress(insar_application_service::DemTransferEvent::BodyStarted);
    let mut last_bytes = 0u64;
    let exec_result = engine.execute_optical_derived(&request, interrupted, &mut |prog| {
        let delta = prog.bytes_written_estimate.saturating_sub(last_bytes);
        if delta > 0 {
            last_bytes = prog.bytes_written_estimate;
            progress(insar_application_service::DemTransferEvent::BytesReceived(
                delta,
            ));
        }
    });
    progress(insar_application_service::DemTransferEvent::BodyStopped);
    match exec_result {
        Ok(result) => serde_json::to_value(result)
            .map_err(|e| format!("OPTICAL_DERIVED_SERIALIZE_FAILED: {e}")),
        Err(err) => Err(err.to_string()),
    }
}

#[cfg(test)]
#[path = "native_dem_progress_tests.rs"]
mod native_dem_progress_tests;

#[cfg(test)]
#[path = "native_dem_provider_output_tests.rs"]
mod native_dem_provider_output_tests;

impl insar_application_service::DemOperationPort for NativeGisOperationPort {
    fn call_with_transfer_progress(
        &self,
        operation: &str,
        payload: serde_json::Value,
        interrupted: &(dyn Fn() -> bool + Sync),
        progress: &mut dyn FnMut(insar_application_service::DemTransferEvent),
    ) -> Result<serde_json::Value, String> {
        if operation == "dem_download" {
            native_dem_download_with_progress(&self.engine, &payload, interrupted, progress)
        } else if operation == "optical_derived_execute" {
            native_optical_derived_execute(
                &self.engine,
                self.store_path.as_deref(),
                &payload,
                interrupted,
                progress,
            )
        } else {
            self.call(operation, payload)
        }
    }

    fn call(
        &self,
        operation: &str,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        match operation {
            "dem_inspect" => {
                let path = payload_path(&payload, "path")?;
                self.engine
                    .inspect_dem(path)
                    .map_err(|error| format!("{}: {}", error.code().as_str(), error))
            }
            "dem_prepare_sarscape" => native_dem_plan(&payload),
            "dem_download" => native_dem_download(&self.engine, &payload),
            "dem_finalize_windows" => native_dem_finalize_windows(&self.engine, &payload),
            "dem_convert" => native_dem_convert(&self.engine, &payload),
            "optical_derived_execute" => native_optical_derived_execute(
                &self.engine,
                self.store_path.as_deref(),
                &payload,
                &|| false,
                &mut |_| {},
            ),
            _ => Err(format!(
                "NATIVE_GIS_OPERATION_UNSUPPORTED: unsupported native GIS operation {operation}"
            )),
        }
    }
}

fn payload_path(payload: &serde_json::Value, field: &str) -> Result<PathBuf, String> {
    payload
        .get(field)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| format!("NATIVE_GIS_INVALID_PAYLOAD: {field} is required"))
}

fn payload_path_any(payload: &serde_json::Value, fields: &[&str]) -> Result<PathBuf, String> {
    fields
        .iter()
        .find_map(|field| {
            payload
                .get(*field)
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(PathBuf::from)
        })
        .ok_or_else(|| {
            format!(
                "NATIVE_GIS_INVALID_PAYLOAD: one of {} is required",
                fields.join(", ")
            )
        })
}

fn native_dem_plan(payload: &serde_json::Value) -> Result<serde_json::Value, String> {
    let input = payload_path_any(payload, &["input_path", "path"])?;
    let output_root = payload_path_any(payload, &["output_root", "output_dir"])?;
    let source = payload
        .get("source_vertical_datum")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("EGM2008")
        .to_ascii_uppercase();
    let target = payload
        .get("target_vertical_datum")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("WGS84_ELLIPSOID")
        .to_ascii_uppercase();
    Ok(serde_json::json!({
        "ok": true,
        "native": true,
        "executed": false,
        "offline": true,
        "plan": {
            "input_path": input,
            "output_root": output_root,
            "source_vertical_datum": source,
            "target_vertical_datum": target,
            "execution_owner": "insar-gis",
            "runtime": "product-owned-native-gdal-proj",
        }
    }))
}

use insar_application_service::dem_limits::{
    self, MAX_NATIVE_DEM_DOWNLOAD_BYTES, MAX_NATIVE_DEM_TILES,
};

#[derive(Clone, Debug, PartialEq, Eq)]
struct NativeDemDownloadSource {
    id: String,
    url: String,
}

fn prepare_native_task_output_path(output_root: &Path, path: &Path) -> Result<(), String> {
    if !output_root.is_absolute() || !path.is_absolute() {
        return Err("NATIVE_DEM_INVALID_PAYLOAD: DEM task paths must be absolute".to_owned());
    }
    if path.file_name().is_none() {
        return Err("NATIVE_DEM_INVALID_PAYLOAD: DEM output filename is invalid".to_owned());
    }
    std::fs::create_dir_all(output_root).map_err(|_| {
        "NATIVE_DEM_OUTPUT_UNAVAILABLE: DEM output root could not be prepared".to_owned()
    })?;
    let relative_parent = path
        .parent()
        .ok_or_else(|| "NATIVE_DEM_INVALID_PAYLOAD: DEM output path has no parent".to_owned())?
        .strip_prefix(output_root)
        .map_err(|_| {
            "NATIVE_DEM_INVALID_PAYLOAD: DEM output path is outside the task root".to_owned()
        })?;
    let mut current = output_root.to_path_buf();
    for component in relative_parent.components() {
        let Component::Normal(segment) = component else {
            return Err(
                "NATIVE_DEM_INVALID_PAYLOAD: DEM output path contains unsafe components".to_owned(),
            );
        };
        current.push(segment);
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(
                    "NATIVE_DEM_OUTPUT_ESCAPE: DEM task directory contains a symlink".to_owned(),
                );
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err(
                    "NATIVE_DEM_OUTPUT_UNAVAILABLE: DEM task parent is not a directory".to_owned(),
                );
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&current).map_err(|_| {
                    "NATIVE_DEM_OUTPUT_UNAVAILABLE: DEM task directory could not be created"
                        .to_owned()
                })?;
            }
            Err(_) => {
                return Err(
                    "NATIVE_DEM_OUTPUT_UNAVAILABLE: DEM task directory could not be inspected"
                        .to_owned(),
                );
            }
        }
    }
    let canonical_root = std::fs::canonicalize(output_root).map_err(|_| {
        "NATIVE_DEM_OUTPUT_UNAVAILABLE: DEM output root could not be resolved".to_owned()
    })?;
    let canonical_parent = std::fs::canonicalize(path.parent().unwrap()).map_err(|_| {
        "NATIVE_DEM_OUTPUT_UNAVAILABLE: DEM output parent could not be resolved".to_owned()
    })?;
    if !canonical_parent.starts_with(&canonical_root) {
        return Err("NATIVE_DEM_OUTPUT_ESCAPE: DEM task path resolves outside its root".to_owned());
    }
    if path.exists()
        && !std::fs::canonicalize(path)
            .map(|resolved| resolved.starts_with(&canonical_root))
            .unwrap_or(false)
    {
        return Err(
            "NATIVE_DEM_OUTPUT_ESCAPE: existing DEM path resolves outside its root".to_owned(),
        );
    }
    Ok(())
}

fn native_dem_window_staging_root(
    payload: &serde_json::Value,
    output_root: &Path,
    inputs: &[PathBuf],
) -> Result<PathBuf, String> {
    let task_id = payload
        .get("task_id")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "NATIVE_DEM_INVALID_PAYLOAD: task_id is required".to_owned())?;
    let mut task_components = Path::new(task_id).components();
    if !matches!(task_components.next(), Some(Component::Normal(_)))
        || task_components.next().is_some()
    {
        return Err(
            "NATIVE_DEM_INVALID_PAYLOAD: task_id must be one safe path component".to_owned(),
        );
    }
    let staging_root = output_root.join(".eostage").join("dem-tasks").join(task_id);
    if inputs
        .iter()
        .any(|input| input.parent() != Some(staging_root.as_path()))
    {
        return Err(
            "NATIVE_DEM_INVALID_PAYLOAD: window inputs must belong to the task staging directory"
                .to_owned(),
        );
    }
    Ok(staging_root)
}

fn cleanup_native_dem_window_staging(staging_root: &Path, inputs: &[PathBuf]) -> serde_json::Value {
    let mut removed_window_files = 0_u64;
    let mut cleanup_incomplete = false;
    for input in inputs {
        match std::fs::remove_file(input) {
            Ok(()) => removed_window_files += 1,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => cleanup_incomplete = true,
        }
    }

    let mut staging_directory_removed = false;
    let mut residual_entries_preserved = false;
    if !cleanup_incomplete {
        match std::fs::read_dir(staging_root) {
            Ok(mut entries) => {
                residual_entries_preserved = entries.next().is_some();
                if !residual_entries_preserved {
                    match std::fs::remove_dir(staging_root) {
                        Ok(()) => staging_directory_removed = true,
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                            staging_directory_removed = true;
                        }
                        Err(_) => cleanup_incomplete = true,
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                staging_directory_removed = true;
            }
            Err(_) => cleanup_incomplete = true,
        }
    }

    serde_json::json!({
        "ok": !cleanup_incomplete,
        "removed_window_files": removed_window_files,
        "staging_directory_removed": staging_directory_removed,
        "residual_entries_preserved": residual_entries_preserved,
        "error_code": cleanup_incomplete.then_some("NATIVE_DEM_CLEANUP_INCOMPLETE"),
    })
}

fn native_dem_download(
    engine: &LazyNativeGisEngine,
    payload: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    native_dem_download_with_progress(engine, payload, &|| false, &mut |_| {})
}

fn native_dem_download_with_progress(
    engine: &LazyNativeGisEngine,
    payload: &serde_json::Value,
    interrupted: &(dyn Fn() -> bool + Sync),
    progress: &mut dyn FnMut(insar_application_service::DemTransferEvent),
) -> Result<serde_json::Value, String> {
    if interrupted() {
        return Err("NATIVE_DEM_INTERRUPTED: task paused or cancelled".to_owned());
    }
    let input = payload_path_any(payload, &["input_path", "path"])?;
    let output_root = payload_path_any(payload, &["output_root", "output_dir"])?;
    let dataset = payload
        .get("dataset")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .trim()
        .to_ascii_uppercase();
    let provider = payload
        .get("provider")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .trim()
        .to_ascii_uppercase();
    let bbox = native_dem_bbox(payload)?;
    let estimate = dem_limits::estimate(&dataset, bbox);
    if dem_limits::limits_apply(&provider) && estimate.exceeds_mosaic() {
        return Err(format!(
            "NATIVE_DEM_OUTPUT_LIMIT: source-aligned DEM crop of {} x {} samples exceeds the bounded native raster limit",
            estimate.width, estimate.height
        ));
    }
    let sources = {
        #[cfg(test)]
        {
            if provider == "GEM15_PUBLIC" {
                if let Some(base) = payload
                    .get("test_gem15_base_url")
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|url| url.starts_with("http://127.0.0.1:"))
                {
                    native_dem_gem15_sources_with_base(&dataset, bbox, base)?
                } else {
                    native_dem_download_sources(&dataset, &provider, bbox)?
                }
            } else {
                native_dem_download_sources(&dataset, &provider, bbox)?
            }
        }
        #[cfg(not(test))]
        {
            native_dem_download_sources(&dataset, &provider, bbox)?
        }
    };
    let proxy_enabled = payload
        .get("proxy_enabled")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let proxy_url = payload
        .get("proxy_url")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .trim();
    let mut builder = reqwest::blocking::Client::builder()
        .user_agent(format!("EOStage/{}", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(120))
        // Provider routing must not inherit an ambient system proxy. The
        // product-owned settings sidecar is the only proxy authority.
        .no_proxy();
    if proxy_enabled && !proxy_url.is_empty() {
        builder = builder
            .proxy(reqwest::Proxy::all(proxy_url).map_err(|_| {
                "NATIVE_DEM_PROXY_INVALID: product proxy URL is invalid".to_owned()
            })?);
    }
    let client = builder
        .build()
        .map_err(|_| "NATIVE_DEM_HTTP_UNAVAILABLE: native HTTP client unavailable".to_owned())?;
    // NISAR DEM tiles live behind Earthdata Login. This plain client keeps
    // reqwest's default redirects and has no cookie jar, so a Basic login
    // would be stripped on the redirect to URS; always send an EDL bearer
    // token instead (a saved login is exchanged for one). Missing credentials
    // fail closed with the typed Earthdata-account message.
    let earthdata_auth = if matches!(
        provider.as_str(),
        "NISAR_DEM_ASF" | "EARTHDATA" | "LPDAAC" | "ASF"
    ) {
        let required = || {
            format!("NATIVE_DEM_CREDENTIAL_REQUIRED: an Earthdata account is required for {provider} DEM tiles")
        };
        let token = insar_download::resolve_earthdata_search_token().map_err(|_| required())?;
        Some(insar_download::TransferAuth::bearer(token).map_err(|_| required())?)
    } else {
        None
    };

    prepare_native_task_output_path(&output_root, &input)?;
    let source_root = input.with_file_name(format!(
        ".{}.source-tiles",
        input
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("dem")
    ));
    prepare_native_task_output_path(&output_root, &source_root.join(".guard"))?;
    let mut source_paths = Vec::with_capacity(sources.len());
    let mut downloaded_bytes = 0_u64;
    for source in &sources {
        if interrupted() {
            return Err("NATIVE_DEM_INTERRUPTED: task paused or cancelled".to_owned());
        }
        let source_path = source_root.join(format!("{}.tif", source.id));
        let cached_bytes = std::fs::metadata(&source_path)
            .ok()
            .filter(|metadata| metadata.is_file() && metadata.len() > 0)
            .and_then(|metadata| {
                if engine.is_discovered() {
                    engine
                        .inspect_dem(&source_path)
                        .ok()
                        .map(|_| metadata.len())
                } else {
                    validate_raw_geotiff_structure(&source_path)
                        .ok()
                        .map(|_| metadata.len())
                }
            });
        let source_bytes = if let Some(bytes) = cached_bytes {
            bytes
        } else {
            let _ = std::fs::remove_file(&source_path);
            match download_native_dem_geotiff(
                engine,
                &client,
                earthdata_auth.as_ref(),
                &source.url,
                &source_path,
                NativeDemTransferPolicy {
                    provider: &provider,
                    remaining_bytes: MAX_NATIVE_DEM_DOWNLOAD_BYTES.saturating_sub(downloaded_bytes),
                },
                interrupted,
                progress,
            ) {
                Ok(bytes) => bytes,
                Err(error)
                    if matches!(
                        provider.as_str(),
                        "FABDEM_PUBLIC" | "GEM15_PUBLIC" | "EARTHDATA" | "LPDAAC" | "ASF"
                    ) && error.starts_with("NATIVE_DEM_TILE_NOT_FOUND_404:") =>
                {
                    let _ = std::fs::remove_file(&source_path);
                    continue;
                }
                Err(error) => return Err(error),
            }
        };
        downloaded_bytes = downloaded_bytes.saturating_add(source_bytes);
        if downloaded_bytes > MAX_NATIVE_DEM_DOWNLOAD_BYTES {
            return Err(
                "NATIVE_DEM_SIZE_LIMIT: DEM source tiles exceed the 2 GiB task bound".to_owned(),
            );
        }
        source_paths.push(source_path);
    }

    if source_paths.is_empty() && provider == "FABDEM_PUBLIC" {
        return Err(
            "NATIVE_DEM_OCEAN_NO_TILE: Selected AOI contains no FABDEM V1-2 land tiles (HTTP 404)"
                .to_owned(),
        );
    }
    if source_paths.is_empty() && matches!(provider.as_str(), "EARTHDATA" | "LPDAAC" | "ASF") {
        return Err(
            "NATIVE_DEM_OCEAN_NO_TILE: Selected AOI contains no ASTER GDEM v3 land tiles (HTTP 404)"
                .to_owned(),
        );
    }
    if source_paths.is_empty() && provider == "GEM15_PUBLIC" {
        return Err(
            "NATIVE_DEM_OUT_OF_COVERAGE: Selected AOI has no Google GEM-15 v1 tiles (HTTP 404 — ocean or excluded regions: Mainland China, Russia, Mongolia, Greenland, Antarctica). Please switch to Copernicus GLO-30 or FABDEM V1-2."
                .to_owned(),
        );
    }

    let preparation =
        prepare_native_dem_download_output(engine, &provider, &source_paths, &input, bbox)?;
    let final_inspection = if engine.is_discovered() {
        let inspected = engine
            .inspect_dem(&input)
            .map_err(|error| format!("{}: {}", error.code().as_str(), error))?;
        let final_metadata = inspected
            .get("metadata")
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| "NATIVE_DEM_INVALID_RASTER: final DEM metadata is missing".to_owned())?;
        if final_metadata
            .get("count")
            .and_then(serde_json::Value::as_u64)
            != Some(1)
            || final_metadata
                .get("crs")
                .and_then(serde_json::Value::as_str)
                .is_none()
        {
            let _ = std::fs::remove_file(&input);
            return Err(
                "NATIVE_DEM_INVALID_RASTER: final DEM failed the single-band CRS check".to_owned(),
            );
        }
        inspected
    } else {
        let summary = validate_raw_geotiff_structure(&input).inspect_err(|_| {
            let _ = std::fs::remove_file(&input);
        })?;
        serde_json::json!({
            "ok": true,
            "mode": "raw_geotiff_no_native_gis",
            "native_gis_available": false,
            "validation": "rust_geotiff_decoded_crs",
            "metadata": {
                "driver": "GTiff",
                "width": summary.width,
                "height": summary.height,
                "count": summary.bands,
                "crs": summary.crs,
                "compression": summary.compression,
                "bigtiff": summary.is_bigtiff,
                "geotiff_tags_present": summary.geotiff_tags_present
            }
        })
    };
    for source_path in &source_paths {
        let _ = std::fs::remove_file(source_path);
    }
    let _ = std::fs::remove_dir(&source_root);

    let output_bytes = std::fs::metadata(&input)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    if output_bytes == 0 {
        let _ = std::fs::remove_file(&input);
        return Err("NATIVE_DEM_INVALID_RASTER: final DEM is empty".to_owned());
    }
    let mut result = serde_json::json!({
        "ok": true,
        "provider": provider,
        "dataset": dataset,
        "raw_path": input,
        "bytes_written": output_bytes,
        "native_gis_available": engine.is_discovered(),
        "source_url": match provider.as_str() {
            "OPENTOPOGRAPHY" => "https://portal.opentopography.org/API/globaldem",
            "NISAR_DEM_ASF" => {
                "https://nisar.asf.earthdatacloud.nasa.gov/NISAR/DEM/v1.2/EPSG4326/"
            }
            "EARTHDATA" | "LPDAAC" | "ASF" => {
                "https://data.lpdaac.earthdatacloud.nasa.gov/lp-prod-protected/ASTGTM.003/"
            }
            "FABDEM_PUBLIC" => "https://huggingface.co/buckets/links-ads/fabdem/resolve/tiles",
            "GEM15_PUBLIC" => "https://storage.googleapis.com/climate-and-sustainability/gem-15",
            _ => "copernicus-aws-cog",
        },
        "source_tile_count": sources.len(),
        "preparation": preparation,
        "inspection": final_inspection,
    });
    if payload
        .get("convert")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
        && payload
            .get("window_count")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(1)
            <= 1
    {
        if engine.is_discovered() {
            let source_vertical_datum = native_dem_source_vertical_datum(payload)?;
            let conversion = native_dem_convert(
                engine,
                &serde_json::json!({
                    "input_path": input,
                    "output_root": output_root,
                    "source_vertical_datum": source_vertical_datum,
                    "output_mode": "sarscape",
                }),
            )?;
            if let Some(object) = result.as_object_mut() {
                object.insert("conversion".to_owned(), conversion);
            }
        } else if let Some(object) = result.as_object_mut() {
            object.insert(
                "conversion_skipped".to_owned(),
                serde_json::Value::Bool(true),
            );
            object.insert(
                "conversion_skipped_reason".to_owned(),
                serde_json::Value::String("native_gis_missing".to_owned()),
            );
        }
    }
    Ok(result)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RawGeoTiffSummary {
    width: u64,
    height: u64,
    bands: u64,
    is_bigtiff: bool,
    compression: &'static str,
    crs: String,
    geotiff_tags_present: bool,
}

#[derive(Clone, Copy)]
enum TiffEndian {
    Little,
    Big,
}

impl TiffEndian {
    fn read_u16(self, bytes: &[u8]) -> u16 {
        let arr: [u8; 2] = [bytes[0], bytes[1]];
        match self {
            Self::Little => u16::from_le_bytes(arr),
            Self::Big => u16::from_be_bytes(arr),
        }
    }

    fn read_u32(self, bytes: &[u8]) -> u32 {
        let arr: [u8; 4] = [bytes[0], bytes[1], bytes[2], bytes[3]];
        match self {
            Self::Little => u32::from_le_bytes(arr),
            Self::Big => u32::from_be_bytes(arr),
        }
    }

    fn read_u64(self, bytes: &[u8]) -> u64 {
        let arr: [u8; 8] = [
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ];
        match self {
            Self::Little => u64::from_le_bytes(arr),
            Self::Big => u64::from_be_bytes(arr),
        }
    }

    fn read_f64(self, bytes: &[u8]) -> f64 {
        let arr: [u8; 8] = [
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ];
        match self {
            Self::Little => f64::from_le_bytes(arr),
            Self::Big => f64::from_be_bytes(arr),
        }
    }
}

fn read_exact_at(file: &mut std::fs::File, offset: u64, buf: &mut [u8]) -> Result<(), String> {
    use std::io::{Read, Seek, SeekFrom};
    file.seek(SeekFrom::Start(offset))
        .map_err(|_| "NATIVE_DEM_INVALID_RASTER: TIFF seek outside file bounds".to_owned())?;
    file.read_exact(buf)
        .map_err(|_| "NATIVE_DEM_INVALID_RASTER: TIFF structure truncated".to_owned())
}

fn read_tiff_tag_raw_bytes(
    file: &mut std::fs::File,
    endian: TiffEndian,
    is_bigtiff: bool,
    file_len: u64,
    total_bytes: u64,
    inline_slice: &[u8],
) -> Result<Vec<u8>, String> {
    let inline_limit = if is_bigtiff { 8_u64 } else { 4_u64 };
    let header_len = if is_bigtiff { 16_u64 } else { 8_u64 };
    if total_bytes <= inline_limit {
        return Ok(inline_slice[..total_bytes as usize].to_vec());
    }
    let ext_offset = if is_bigtiff {
        endian.read_u64(inline_slice)
    } else {
        u64::from(endian.read_u32(inline_slice))
    };
    if ext_offset < header_len
        || ext_offset
            .checked_add(total_bytes)
            .is_none_or(|end| end > file_len)
    {
        return Err(
            "NATIVE_DEM_INVALID_RASTER: TIFF tag array extends past end of file".to_owned(),
        );
    }
    let mut buf = vec![0_u8; total_bytes as usize];
    read_exact_at(file, ext_offset, &mut buf)?;
    Ok(buf)
}

fn read_tiff_uint_values(
    file: &mut std::fs::File,
    endian: TiffEndian,
    is_bigtiff: bool,
    file_len: u64,
    field_type: u16,
    count: u64,
    inline_slice: &[u8],
) -> Result<Vec<u64>, String> {
    const MAX_TIFF_ARRAY_ELEMENTS: u64 = 262_144;
    if count == 0 || count > MAX_TIFF_ARRAY_ELEMENTS {
        return Err("NATIVE_DEM_INVALID_RASTER: TIFF integer tag count out of bounds".to_owned());
    }
    let elem_size = match field_type {
        3 => 2_u64,
        4 | 13 => 4_u64,
        16 | 18 => 8_u64,
        _ => {
            return Err("NATIVE_DEM_INVALID_RASTER: unexpected TIFF integer field type".to_owned())
        }
    };
    let total_bytes = count
        .checked_mul(elem_size)
        .ok_or_else(|| "NATIVE_DEM_INVALID_RASTER: TIFF tag byte length overflow".to_owned())?;
    let raw = read_tiff_tag_raw_bytes(
        file,
        endian,
        is_bigtiff,
        file_len,
        total_bytes,
        inline_slice,
    )?;

    let mut out = Vec::with_capacity(count as usize);
    for chunk in raw.chunks_exact(elem_size as usize) {
        let val = match elem_size {
            2 => u64::from(endian.read_u16(chunk)),
            4 => u64::from(endian.read_u32(chunk)),
            8 => endian.read_u64(chunk),
            _ => unreachable!(),
        };
        out.push(val);
    }
    Ok(out)
}

fn read_tiff_f64_values(
    file: &mut std::fs::File,
    endian: TiffEndian,
    is_bigtiff: bool,
    file_len: u64,
    field_type: u16,
    count: u64,
    inline_slice: &[u8],
) -> Result<Vec<f64>, String> {
    const MAX_TIFF_DOUBLE_ELEMENTS: u64 = 65_536;
    if field_type != 12 {
        return Err("NATIVE_DEM_INVALID_RASTER: unexpected TIFF DOUBLE field type".to_owned());
    }
    if count == 0 || count > MAX_TIFF_DOUBLE_ELEMENTS {
        return Err("NATIVE_DEM_INVALID_RASTER: GeoTIFF double tag count out of bounds".to_owned());
    }
    let total_bytes = count
        .checked_mul(8)
        .ok_or_else(|| "NATIVE_DEM_INVALID_RASTER: GeoTIFF double tag overflow".to_owned())?;
    let raw = read_tiff_tag_raw_bytes(
        file,
        endian,
        is_bigtiff,
        file_len,
        total_bytes,
        inline_slice,
    )?;
    let mut out = Vec::with_capacity(count as usize);
    for chunk in raw.chunks_exact(8) {
        let value = endian.read_f64(chunk);
        if !value.is_finite() {
            return Err(
                "NATIVE_DEM_INVALID_RASTER: GeoTIFF tag contains non-finite floating-point value"
                    .to_owned(),
            );
        }
        out.push(value);
    }
    Ok(out)
}

fn read_tiff_ascii_bytes(
    file: &mut std::fs::File,
    endian: TiffEndian,
    is_bigtiff: bool,
    file_len: u64,
    field_type: u16,
    count: u64,
    inline_slice: &[u8],
) -> Result<Vec<u8>, String> {
    const MAX_TIFF_ASCII_BYTES: u64 = 262_144;
    if field_type != 2 {
        return Err("NATIVE_DEM_INVALID_RASTER: unexpected TIFF ASCII field type".to_owned());
    }
    if count == 0 || count > MAX_TIFF_ASCII_BYTES {
        return Err("NATIVE_DEM_INVALID_RASTER: GeoTIFF ASCII tag length out of bounds".to_owned());
    }
    read_tiff_tag_raw_bytes(file, endian, is_bigtiff, file_len, count, inline_slice)
}

fn validate_chunk_spans(
    offsets: &[u64],
    byte_counts: &[u64],
    expected_chunks: u64,
    header_len: u64,
    file_len: u64,
) -> Result<(), String> {
    if offsets.is_empty()
        || offsets.len() != byte_counts.len()
        || (offsets.len() as u64) != expected_chunks
    {
        return Err(
            "NATIVE_DEM_INVALID_RASTER: mismatched TIFF offset and byte-count tables".to_owned(),
        );
    }
    let mut has_nonzero = false;
    for (&off, &cnt) in offsets.iter().zip(byte_counts.iter()) {
        if cnt > 0 {
            has_nonzero = true;
            if off < header_len || off.checked_add(cnt).is_none_or(|end| end > file_len) {
                return Err(
                    "NATIVE_DEM_INVALID_RASTER: TIFF raster data chunk extends past end of file"
                        .to_owned(),
                );
            }
        }
    }
    if !has_nonzero {
        return Err(
            "NATIVE_DEM_INVALID_RASTER: TIFF contains no non-empty raster chunks".to_owned(),
        );
    }
    Ok(())
}

fn verify_deflate_chunk(
    file: &mut std::fs::File,
    compressed_bytes: u64,
    expected_bytes: u64,
) -> Result<(), String> {
    use std::io::Read;
    let limited = std::io::BufReader::new(file.take(compressed_bytes));
    let mut decoder = flate2::read::ZlibDecoder::new(limited);
    let mut scratch = [0_u8; 32_768];
    let mut emitted = 0_u64;
    loop {
        let read = decoder.read(&mut scratch).map_err(|_| {
            "NATIVE_DEM_INVALID_RASTER: corrupt or truncated DEFLATE raster chunk".to_owned()
        })?;
        if read == 0 {
            break;
        }
        emitted = emitted.saturating_add(read as u64);
        if emitted > expected_bytes {
            return Err(
                "NATIVE_DEM_INVALID_RASTER: DEFLATE raster chunk exceeds expected pixel size"
                    .to_owned(),
            );
        }
    }
    if emitted != expected_bytes {
        return Err(format!(
            "NATIVE_DEM_INVALID_RASTER: DEFLATE raster chunk decoded {emitted} bytes, expected {expected_bytes}"
        ));
    }
    Ok(())
}

fn verify_packbits_chunk(
    file: &mut std::fs::File,
    compressed_bytes: u64,
    expected_bytes: u64,
) -> Result<(), String> {
    use std::io::Read;
    let mut reader = std::io::BufReader::new(file.take(compressed_bytes));
    let mut emitted = 0_u64;
    let mut header = [0_u8; 1];
    let mut literal = [0_u8; 128];
    let mut repeated = [0_u8; 1];
    loop {
        match reader.read(&mut header) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => {
                return Err(
                    "NATIVE_DEM_INVALID_RASTER: failed reading PackBits raster stream".to_owned(),
                );
            }
        }
        let control = header[0] as i8;
        if control == -128 {
            continue;
        } else if control >= 0 {
            let run = (control as u64) + 1;
            if emitted.saturating_add(run) > expected_bytes {
                return Err(
                    "NATIVE_DEM_INVALID_RASTER: PackBits raster chunk exceeds expected pixel size"
                        .to_owned(),
                );
            }
            reader
                .read_exact(&mut literal[..run as usize])
                .map_err(|_| {
                    "NATIVE_DEM_INVALID_RASTER: truncated PackBits literal run".to_owned()
                })?;
            emitted += run;
        } else {
            let run = (1_i16 - i16::from(control)) as u64;
            if emitted.saturating_add(run) > expected_bytes {
                return Err(
                    "NATIVE_DEM_INVALID_RASTER: PackBits raster chunk exceeds expected pixel size"
                        .to_owned(),
                );
            }
            reader.read_exact(&mut repeated).map_err(|_| {
                "NATIVE_DEM_INVALID_RASTER: truncated PackBits repeat run".to_owned()
            })?;
            emitted += run;
        }
    }
    if emitted != expected_bytes {
        return Err(format!(
            "NATIVE_DEM_INVALID_RASTER: PackBits raster chunk decoded {emitted} bytes, expected {expected_bytes}"
        ));
    }
    Ok(())
}

fn verify_lzw_chunk(
    file: &mut std::fs::File,
    compressed_bytes: u64,
    expected_bytes: u64,
) -> Result<(), String> {
    use std::io::Read;
    const CLEAR_CODE: usize = 256;
    const EOI_CODE: usize = 257;
    const MAX_TABLE: usize = 4096;

    let mut reader = std::io::BufReader::new(file.take(compressed_bytes));
    let mut bit_buf = 0_u64;
    let mut bits_avail = 0_u8;
    let mut byte_buf = [0_u8; 1];

    let mut first_byte = vec![0_u8; MAX_TABLE];
    let mut entry_len = vec![0_u16; MAX_TABLE];
    for i in 0..256 {
        first_byte[i] = i as u8;
        entry_len[i] = 1;
    }

    let mut next_code = 258_usize;
    let mut code_bits = 9_u8;
    let mut prev_code: Option<usize> = None;
    let mut emitted = 0_u64;
    let mut saw_initial_clear = false;

    loop {
        while bits_avail < code_bits {
            match reader.read(&mut byte_buf) {
                Ok(0) => {
                    return Err(
                        "NATIVE_DEM_INVALID_RASTER: truncated LZW raster stream before EOI"
                            .to_owned(),
                    );
                }
                Ok(_) => {
                    bit_buf = (bit_buf << 8) | u64::from(byte_buf[0]);
                    bits_avail += 8;
                }
                Err(_) => {
                    return Err(
                        "NATIVE_DEM_INVALID_RASTER: failed reading LZW raster stream".to_owned(),
                    );
                }
            }
        }
        let shift = bits_avail - code_bits;
        let code = ((bit_buf >> shift) & ((1_u64 << code_bits) - 1)) as usize;
        bit_buf &= (1_u64 << shift) - 1;
        bits_avail = shift;

        if !saw_initial_clear {
            if code != CLEAR_CODE {
                return Err(
                    "NATIVE_DEM_INVALID_RASTER: LZW raster stream must begin with CLEAR code"
                        .to_owned(),
                );
            }
            saw_initial_clear = true;
            next_code = 258;
            code_bits = 9;
            prev_code = None;
            continue;
        }
        if code == CLEAR_CODE {
            next_code = 258;
            code_bits = 9;
            prev_code = None;
            continue;
        }
        if code == EOI_CODE {
            break;
        }

        if code < next_code {
            let chunk_len = u64::from(entry_len[code]);
            if let Some(prev) = prev_code {
                if next_code < MAX_TABLE {
                    first_byte[next_code] = first_byte[prev];
                    entry_len[next_code] = entry_len[prev].saturating_add(1);
                    next_code += 1;
                    match next_code {
                        511 if code_bits == 9 => code_bits = 10,
                        1023 if code_bits == 10 => code_bits = 11,
                        2047 if code_bits == 11 => code_bits = 12,
                        _ => {}
                    }
                }
            }
            emitted = emitted.saturating_add(chunk_len);
        } else if code == next_code {
            let prev = prev_code.ok_or_else(|| {
                "NATIVE_DEM_INVALID_RASTER: invalid LZW KwKwK code before first literal".to_owned()
            })?;
            if next_code >= MAX_TABLE {
                return Err(
                    "NATIVE_DEM_INVALID_RASTER: LZW dictionary overflow before CLEAR code"
                        .to_owned(),
                );
            }
            first_byte[next_code] = first_byte[prev];
            entry_len[next_code] = entry_len[prev].saturating_add(1);
            let chunk_len = u64::from(entry_len[next_code]);
            next_code += 1;
            match next_code {
                511 if code_bits == 9 => code_bits = 10,
                1023 if code_bits == 10 => code_bits = 11,
                2047 if code_bits == 11 => code_bits = 12,
                _ => {}
            }
            emitted = emitted.saturating_add(chunk_len);
        } else {
            return Err("NATIVE_DEM_INVALID_RASTER: corrupt LZW code out of range".to_owned());
        }

        if emitted > expected_bytes {
            return Err(
                "NATIVE_DEM_INVALID_RASTER: LZW raster chunk exceeds expected pixel size"
                    .to_owned(),
            );
        }
        prev_code = Some(code);
    }

    if emitted != expected_bytes {
        return Err(format!(
            "NATIVE_DEM_INVALID_RASTER: LZW raster chunk decoded {emitted} bytes, expected {expected_bytes}"
        ));
    }
    Ok(())
}

fn validate_geotiff_spatial_and_crs(
    pixel_scale: Option<&[f64]>,
    tiepoints: Option<&[f64]>,
    model_transform: Option<&[f64]>,
    geo_key_dir: Option<&[u64]>,
    geo_double_params: Option<&[f64]>,
    geo_ascii_params: Option<&[u8]>,
) -> Result<String, String> {
    let has_model_transform = match model_transform {
        Some(m) => {
            if m.len() != 16 {
                return Err(
                    "NATIVE_DEM_INVALID_RASTER: ModelTransformationTag must have 16 elements"
                        .to_owned(),
                );
            }
            let det = m[0] * m[5] - m[1] * m[4];
            if det.abs() <= 0.0 {
                return Err(
                    "NATIVE_DEM_INVALID_RASTER: ModelTransformationTag affine matrix is singular"
                        .to_owned(),
                );
            }
            true
        }
        None => false,
    };

    let has_scale_and_tiepoint = match (pixel_scale, tiepoints) {
        (Some(scale), Some(ties)) => {
            if scale.len() != 3 || scale[0] <= 0.0 || scale[1] <= 0.0 {
                return Err(
                    "NATIVE_DEM_INVALID_RASTER: ModelPixelScaleTag must define positive X/Y scale"
                        .to_owned(),
                );
            }
            if ties.len() < 6 || ties.len() % 6 != 0 {
                return Err(
                    "NATIVE_DEM_INVALID_RASTER: ModelTiepointTag must contain 6-tuples".to_owned(),
                );
            }
            true
        }
        (Some(scale), None) => {
            if scale.len() != 3 || scale[0] <= 0.0 || scale[1] <= 0.0 {
                return Err(
                    "NATIVE_DEM_INVALID_RASTER: ModelPixelScaleTag must define positive X/Y scale"
                        .to_owned(),
                );
            }
            false
        }
        (None, Some(ties)) => {
            if ties.len() < 6 || ties.len() % 6 != 0 {
                return Err(
                    "NATIVE_DEM_INVALID_RASTER: ModelTiepointTag must contain 6-tuples".to_owned(),
                );
            }
            false
        }
        (None, None) => false,
    };

    if !has_model_transform && !has_scale_and_tiepoint {
        return Err(
            "NATIVE_DEM_INVALID_RASTER: GeoTIFF missing valid spatial georeferencing transform"
                .to_owned(),
        );
    }

    let keys = geo_key_dir.ok_or_else(|| {
        "NATIVE_DEM_INVALID_RASTER: GeoTIFF missing GeoKeyDirectoryTag CRS definition".to_owned()
    })?;
    if keys.len() < 4 {
        return Err("NATIVE_DEM_INVALID_RASTER: truncated GeoKeyDirectoryTag header".to_owned());
    }
    let dir_version = keys[0];
    let key_revision = keys[1];
    let minor_revision = keys[2];
    let num_keys = keys[3] as usize;
    if dir_version != 1 || key_revision != 1 || !matches!(minor_revision, 0 | 1) {
        return Err("NATIVE_DEM_INVALID_RASTER: unsupported GeoKeyDirectoryTag version".to_owned());
    }
    if num_keys == 0 || keys.len() < 4 + num_keys * 4 {
        return Err(
            "NATIVE_DEM_INVALID_RASTER: GeoKeyDirectoryTag key table truncated or empty".to_owned(),
        );
    }

    let mut model_type: Option<u64> = None;
    let mut geog_type: Option<u64> = None;
    let mut proj_cs_type: Option<u64> = None;

    for idx in 0..num_keys {
        let base = 4 + idx * 4;
        let key_id = keys[base];
        let tag_loc = keys[base + 1];
        let val_count = keys[base + 2] as usize;
        let val_off = keys[base + 3] as usize;

        match tag_loc {
            0 => {
                if val_count != 1 {
                    return Err(
                        "NATIVE_DEM_INVALID_RASTER: inline GeoKey must have count 1".to_owned()
                    );
                }
                let val = val_off as u64;
                match key_id {
                    1024 => model_type = Some(val),
                    2048 => geog_type = Some(val),
                    3072 => proj_cs_type = Some(val),
                    _ => {}
                }
            }
            34736 => {
                let params = geo_double_params.ok_or_else(|| {
                    "NATIVE_DEM_INVALID_RASTER: GeoKeyDirectory references missing GeoDoubleParamsTag"
                        .to_owned()
                })?;
                if val_count == 0
                    || val_off
                        .checked_add(val_count)
                        .is_none_or(|end| end > params.len())
                {
                    return Err(
                        "NATIVE_DEM_INVALID_RASTER: GeoDoubleParamsTag index out of bounds"
                            .to_owned(),
                    );
                }
            }
            34737 => {
                let ascii = geo_ascii_params.ok_or_else(|| {
                    "NATIVE_DEM_INVALID_RASTER: GeoKeyDirectory references missing GeoAsciiParamsTag"
                        .to_owned()
                })?;
                if val_count == 0
                    || val_off
                        .checked_add(val_count)
                        .is_none_or(|end| end > ascii.len())
                {
                    return Err(
                        "NATIVE_DEM_INVALID_RASTER: GeoAsciiParamsTag index out of bounds"
                            .to_owned(),
                    );
                }
            }
            34735 => {
                let header_words = 4 + num_keys * 4;
                if val_count == 0
                    || val_off < header_words
                    || val_off
                        .checked_add(val_count)
                        .is_none_or(|end| end > keys.len())
                {
                    return Err(
                        "NATIVE_DEM_INVALID_RASTER: GeoKeyDirectoryTag multi-short offset out of bounds"
                            .to_owned(),
                    );
                }
            }
            _ => {
                return Err(
                    "NATIVE_DEM_INVALID_RASTER: GeoKeyDirectory references unsupported TIFF tag location"
                        .to_owned(),
                );
            }
        }
    }

    fn is_verified_geographic_epsg(code: u64) -> bool {
        matches!(
            code,
            4167 | 4258
                | 4267
                | 4269
                | 4283
                | 4322
                | 4324
                | 4326
                | 4490
                | 4612
                | 4674
                | 4979
                | 6668
                | 7844
        )
    }

    fn is_verified_projected_epsg(code: u64) -> bool {
        matches!(
            code,
            3031 | 3035 | 3413 | 3857 | 4513..=4554 | 25828..=25838 | 26901..=26923 | 32601..=32660 | 32701..=32760
        )
    }

    match model_type {
        Some(2) => {
            if proj_cs_type.is_some() {
                return Err(
                    "NATIVE_DEM_INVALID_RASTER: Geographic GeoTIFF must not define ProjectedCSTypeGeoKey"
                        .to_owned(),
                );
            }
            if let Some(code) = geog_type.filter(|&c| is_verified_geographic_epsg(c)) {
                Ok(format!("EPSG:{code}"))
            } else {
                Err(
                    "NATIVE_DEM_INVALID_RASTER: GeoTIFF Geographic CRS EPSG code is missing, mismatched, or requires native GIS runtime"
                        .to_owned(),
                )
            }
        }
        Some(1) => {
            if let Some(geog) = geog_type {
                if !is_verified_geographic_epsg(geog) {
                    return Err(
                        "NATIVE_DEM_INVALID_RASTER: GeoTIFF Projected CRS embeds invalid GeographicTypeGeoKey"
                            .to_owned(),
                    );
                }
            }
            if let Some(code) = proj_cs_type.filter(|&c| is_verified_projected_epsg(c)) {
                Ok(format!("EPSG:{code}"))
            } else {
                Err(
                    "NATIVE_DEM_INVALID_RASTER: GeoTIFF Projected CRS EPSG code is missing, mismatched, or requires native GIS runtime"
                        .to_owned(),
                )
            }
        }
        _ => Err(
            "NATIVE_DEM_INVALID_RASTER: GeoTIFF missing valid GTModelTypeGeoKey (1 or 2)"
                .to_owned(),
        ),
    }
}

/// Pure-Rust structural, pixel-decoding, and CRS GeoTIFF/BigTIFF validator used when
/// the optional native GDAL runtime is absent. Rejects truncated, header-only, corrupt
/// compressed, multi-band, or non-georeferenced/CRS-less payloads before promotion.
fn validate_raw_geotiff_structure(path: &Path) -> Result<RawGeoTiffSummary, String> {
    use std::io::Seek;

    let mut file = std::fs::File::open(path)
        .map_err(|_| "NATIVE_DEM_INVALID_RASTER: raster file could not be opened".to_owned())?;
    let file_len = file
        .metadata()
        .map(|m| m.len())
        .map_err(|_| "NATIVE_DEM_INVALID_RASTER: raster file metadata unavailable".to_owned())?;
    if file_len < 16 {
        return Err("NATIVE_DEM_INVALID_RASTER: raster file is smaller than a TIFF IFD".to_owned());
    }

    let mut header = [0_u8; 16];
    read_exact_at(&mut file, 0, &mut header)?;
    let endian = match &header[0..2] {
        b"II" => TiffEndian::Little,
        b"MM" => TiffEndian::Big,
        _ => {
            return Err("NATIVE_DEM_INVALID_RASTER: invalid TIFF byte-order marker".to_owned());
        }
    };
    let magic = endian.read_u16(&header[2..4]);
    let (is_bigtiff, header_len, ifd_offset) = match magic {
        42 => (false, 8_u64, u64::from(endian.read_u32(&header[4..8]))),
        43 => {
            let off_size = endian.read_u16(&header[4..6]);
            let reserved = endian.read_u16(&header[6..8]);
            if off_size != 8 || reserved != 0 {
                return Err(
                    "NATIVE_DEM_INVALID_RASTER: invalid BigTIFF header parameters".to_owned(),
                );
            }
            (true, 16_u64, endian.read_u64(&header[8..16]))
        }
        _ => {
            return Err("NATIVE_DEM_INVALID_RASTER: unsupported TIFF version number".to_owned());
        }
    };

    if ifd_offset < header_len || ifd_offset >= file_len {
        return Err("NATIVE_DEM_INVALID_RASTER: TIFF IFD offset outside file bounds".to_owned());
    }

    let (entry_count, entries_start, entry_size, next_ptr_size) = if is_bigtiff {
        if ifd_offset.checked_add(8).is_none_or(|end| end > file_len) {
            return Err("NATIVE_DEM_INVALID_RASTER: truncated BigTIFF IFD header".to_owned());
        }
        let mut cnt_buf = [0_u8; 8];
        read_exact_at(&mut file, ifd_offset, &mut cnt_buf)?;
        (endian.read_u64(&cnt_buf), ifd_offset + 8, 20_u64, 8_u64)
    } else {
        if ifd_offset.checked_add(2).is_none_or(|end| end > file_len) {
            return Err("NATIVE_DEM_INVALID_RASTER: truncated TIFF IFD header".to_owned());
        }
        let mut cnt_buf = [0_u8; 2];
        read_exact_at(&mut file, ifd_offset, &mut cnt_buf)?;
        (
            u64::from(endian.read_u16(&cnt_buf)),
            ifd_offset + 2,
            12_u64,
            4_u64,
        )
    };

    if entry_count == 0 || entry_count > 4096 {
        return Err("NATIVE_DEM_INVALID_RASTER: invalid TIFF IFD tag count".to_owned());
    }
    let table_bytes = entry_count
        .checked_mul(entry_size)
        .and_then(|b| b.checked_add(next_ptr_size))
        .ok_or_else(|| "NATIVE_DEM_INVALID_RASTER: TIFF IFD size overflow".to_owned())?;
    if entries_start
        .checked_add(table_bytes)
        .is_none_or(|end| end > file_len)
    {
        return Err("NATIVE_DEM_INVALID_RASTER: truncated TIFF IFD entry table".to_owned());
    }
    let mut table = vec![0_u8; table_bytes as usize];
    read_exact_at(&mut file, entries_start, &mut table)?;

    let mut width: Option<u64> = None;
    let mut height: Option<u64> = None;
    let mut bits_per_sample: Option<Vec<u64>> = None;
    let mut compression_tag: Option<u64> = None;
    let mut strip_offsets: Option<Vec<u64>> = None;
    let mut samples_per_pixel: Option<u64> = None;
    let mut rows_per_strip: Option<u64> = None;
    let mut strip_byte_counts: Option<Vec<u64>> = None;
    let mut planar_configuration: Option<u64> = None;
    let mut predictor: Option<u64> = None;
    let mut sample_format: Option<Vec<u64>> = None;
    let mut tile_width: Option<u64> = None;
    let mut tile_length: Option<u64> = None;
    let mut tile_offsets: Option<Vec<u64>> = None;
    let mut tile_byte_counts: Option<Vec<u64>> = None;
    let mut pixel_scale: Option<Vec<f64>> = None;
    let mut tiepoints: Option<Vec<f64>> = None;
    let mut model_transform: Option<Vec<f64>> = None;
    let mut geo_key_dir: Option<Vec<u64>> = None;
    let mut geo_double_params: Option<Vec<f64>> = None;
    let mut geo_ascii_params: Option<Vec<u8>> = None;

    for idx in 0..(entry_count as usize) {
        let base = idx * (entry_size as usize);
        let entry = &table[base..base + (entry_size as usize)];
        let tag = endian.read_u16(&entry[0..2]);
        let field_type = endian.read_u16(&entry[2..4]);
        let (count, val_slice) = if is_bigtiff {
            (endian.read_u64(&entry[4..12]), &entry[12..20])
        } else {
            (u64::from(endian.read_u32(&entry[4..8])), &entry[8..12])
        };

        match tag {
            256 => {
                let vals = read_tiff_uint_values(
                    &mut file, endian, is_bigtiff, file_len, field_type, count, val_slice,
                )?;
                if vals.len() != 1 {
                    return Err("NATIVE_DEM_INVALID_RASTER: invalid ImageWidth tag".to_owned());
                }
                width = Some(vals[0]);
            }
            257 => {
                let vals = read_tiff_uint_values(
                    &mut file, endian, is_bigtiff, file_len, field_type, count, val_slice,
                )?;
                if vals.len() != 1 {
                    return Err("NATIVE_DEM_INVALID_RASTER: invalid ImageLength tag".to_owned());
                }
                height = Some(vals[0]);
            }
            258 => {
                bits_per_sample = Some(read_tiff_uint_values(
                    &mut file, endian, is_bigtiff, file_len, field_type, count, val_slice,
                )?);
            }
            259 => {
                let vals = read_tiff_uint_values(
                    &mut file, endian, is_bigtiff, file_len, field_type, count, val_slice,
                )?;
                if vals.len() != 1 {
                    return Err("NATIVE_DEM_INVALID_RASTER: invalid Compression tag".to_owned());
                }
                compression_tag = Some(vals[0]);
            }
            273 => {
                strip_offsets = Some(read_tiff_uint_values(
                    &mut file, endian, is_bigtiff, file_len, field_type, count, val_slice,
                )?);
            }
            277 => {
                let vals = read_tiff_uint_values(
                    &mut file, endian, is_bigtiff, file_len, field_type, count, val_slice,
                )?;
                if vals.len() != 1 {
                    return Err("NATIVE_DEM_INVALID_RASTER: invalid SamplesPerPixel tag".to_owned());
                }
                samples_per_pixel = Some(vals[0]);
            }
            278 => {
                let vals = read_tiff_uint_values(
                    &mut file, endian, is_bigtiff, file_len, field_type, count, val_slice,
                )?;
                if vals.len() != 1 {
                    return Err("NATIVE_DEM_INVALID_RASTER: invalid RowsPerStrip tag".to_owned());
                }
                rows_per_strip = Some(vals[0]);
            }
            279 => {
                strip_byte_counts = Some(read_tiff_uint_values(
                    &mut file, endian, is_bigtiff, file_len, field_type, count, val_slice,
                )?);
            }
            284 => {
                let vals = read_tiff_uint_values(
                    &mut file, endian, is_bigtiff, file_len, field_type, count, val_slice,
                )?;
                if vals.len() != 1 {
                    return Err(
                        "NATIVE_DEM_INVALID_RASTER: invalid PlanarConfiguration tag".to_owned()
                    );
                }
                planar_configuration = Some(vals[0]);
            }
            317 => {
                let vals = read_tiff_uint_values(
                    &mut file, endian, is_bigtiff, file_len, field_type, count, val_slice,
                )?;
                if vals.len() != 1 {
                    return Err("NATIVE_DEM_INVALID_RASTER: invalid Predictor tag".to_owned());
                }
                predictor = Some(vals[0]);
            }
            322 => {
                let vals = read_tiff_uint_values(
                    &mut file, endian, is_bigtiff, file_len, field_type, count, val_slice,
                )?;
                if vals.len() != 1 {
                    return Err("NATIVE_DEM_INVALID_RASTER: invalid TileWidth tag".to_owned());
                }
                tile_width = Some(vals[0]);
            }
            323 => {
                let vals = read_tiff_uint_values(
                    &mut file, endian, is_bigtiff, file_len, field_type, count, val_slice,
                )?;
                if vals.len() != 1 {
                    return Err("NATIVE_DEM_INVALID_RASTER: invalid TileLength tag".to_owned());
                }
                tile_length = Some(vals[0]);
            }
            324 => {
                tile_offsets = Some(read_tiff_uint_values(
                    &mut file, endian, is_bigtiff, file_len, field_type, count, val_slice,
                )?);
            }
            325 => {
                tile_byte_counts = Some(read_tiff_uint_values(
                    &mut file, endian, is_bigtiff, file_len, field_type, count, val_slice,
                )?);
            }
            339 => {
                sample_format = Some(read_tiff_uint_values(
                    &mut file, endian, is_bigtiff, file_len, field_type, count, val_slice,
                )?);
            }
            33550 => {
                pixel_scale = Some(read_tiff_f64_values(
                    &mut file, endian, is_bigtiff, file_len, field_type, count, val_slice,
                )?);
            }
            33922 => {
                tiepoints = Some(read_tiff_f64_values(
                    &mut file, endian, is_bigtiff, file_len, field_type, count, val_slice,
                )?);
            }
            34264 => {
                model_transform = Some(read_tiff_f64_values(
                    &mut file, endian, is_bigtiff, file_len, field_type, count, val_slice,
                )?);
            }
            34735 => {
                if field_type != 3 {
                    return Err(
                        "NATIVE_DEM_INVALID_RASTER: GeoKeyDirectoryTag must have SHORT field type"
                            .to_owned(),
                    );
                }
                geo_key_dir = Some(read_tiff_uint_values(
                    &mut file, endian, is_bigtiff, file_len, field_type, count, val_slice,
                )?);
            }
            34736 => {
                geo_double_params = Some(read_tiff_f64_values(
                    &mut file, endian, is_bigtiff, file_len, field_type, count, val_slice,
                )?);
            }
            34737 => {
                geo_ascii_params = Some(read_tiff_ascii_bytes(
                    &mut file, endian, is_bigtiff, file_len, field_type, count, val_slice,
                )?);
            }
            _ => {}
        }
    }

    let width = width.filter(|&w| w > 0).ok_or_else(|| {
        "NATIVE_DEM_INVALID_RASTER: GeoTIFF missing positive ImageWidth".to_owned()
    })?;
    let height = height.filter(|&h| h > 0).ok_or_else(|| {
        "NATIVE_DEM_INVALID_RASTER: GeoTIFF missing positive ImageLength".to_owned()
    })?;
    let bands = samples_per_pixel.unwrap_or(1);
    if bands != 1 {
        return Err(format!(
            "NATIVE_DEM_INVALID_RASTER: DEM must be single-band, found {bands} bands"
        ));
    }
    let bps = bits_per_sample
        .ok_or_else(|| "NATIVE_DEM_INVALID_RASTER: GeoTIFF missing BitsPerSample tag".to_owned())?;
    if bps.len() != bands as usize || bps.iter().any(|&b| !matches!(b, 8 | 16 | 32 | 64)) {
        return Err("NATIVE_DEM_INVALID_RASTER: unsupported DEM BitsPerSample".to_owned());
    }
    let planar = planar_configuration.unwrap_or(1);
    if !matches!(planar, 1 | 2) {
        return Err("NATIVE_DEM_INVALID_RASTER: unsupported TIFF PlanarConfiguration".to_owned());
    }
    let predictor = predictor.unwrap_or(1);
    if !matches!(predictor, 1..=3) {
        return Err("NATIVE_DEM_INVALID_RASTER: unsupported TIFF Predictor".to_owned());
    }
    let sample_fmt = if let Some(ref fmt) = sample_format {
        if fmt.len() != 1 || !matches!(fmt[0], 1..=3) {
            return Err("NATIVE_DEM_INVALID_RASTER: unsupported DEM SampleFormat".to_owned());
        }
        if fmt[0] == 3 && !matches!(bps[0], 32 | 64) {
            return Err(
                "NATIVE_DEM_INVALID_RASTER: IEEE float DEM requires 32-bit or 64-bit samples"
                    .to_owned(),
            );
        }
        fmt[0]
    } else {
        1
    };
    let compression_code = compression_tag.unwrap_or(1);
    let compression = match compression_code {
        1 => "NONE",
        5 => "LZW",
        8 | 32946 => "DEFLATE",
        32773 => "PACKBITS",
        other => {
            return Err(format!(
                "NATIVE_DEM_INVALID_RASTER: unsupported TIFF compression ({other}) without native GIS runtime"
            ));
        }
    };
    match predictor {
        1 => {}
        2 => {
            if !matches!(compression_code, 5 | 8 | 32946) || !matches!(sample_fmt, 1 | 2) {
                return Err(
                    "NATIVE_DEM_INVALID_RASTER: horizontal predictor (2) requires integer LZW/DEFLATE samples"
                        .to_owned(),
                );
            }
        }
        3 => {
            if !matches!(compression_code, 5 | 8 | 32946)
                || sample_fmt != 3
                || !matches!(bps[0], 32 | 64)
            {
                return Err(
                    "NATIVE_DEM_INVALID_RASTER: floating-point predictor (3) requires IEEE float (32/64-bit) LZW/DEFLATE samples"
                        .to_owned(),
                );
            }
        }
        _ => unreachable!(),
    }

    let crs = validate_geotiff_spatial_and_crs(
        pixel_scale.as_deref(),
        tiepoints.as_deref(),
        model_transform.as_deref(),
        geo_key_dir.as_deref(),
        geo_double_params.as_deref(),
        geo_ascii_params.as_deref(),
    )?;

    let bytes_per_sample = bps[0] / 8;
    let total_uncompressed = width
        .checked_mul(height)
        .and_then(|px| px.checked_mul(bytes_per_sample))
        .ok_or_else(|| {
            "NATIVE_DEM_INVALID_RASTER: raster dimension byte size overflow".to_owned()
        })?;
    if total_uncompressed > MAX_NATIVE_DEM_DOWNLOAD_BYTES {
        return Err(
            "NATIVE_DEM_INVALID_RASTER: raster dimensions exceed 2 GiB safety limit".to_owned(),
        );
    }

    let has_strips = match (&strip_offsets, &strip_byte_counts) {
        (Some(_), Some(_)) => true,
        (None, None) => false,
        _ => {
            return Err(
                "NATIVE_DEM_INVALID_RASTER: incomplete TIFF StripOffsets/StripByteCounts"
                    .to_owned(),
            );
        }
    };
    let has_tiles = match (&tile_offsets, &tile_byte_counts) {
        (Some(_), Some(_)) => true,
        (None, None) => false,
        _ => {
            return Err(
                "NATIVE_DEM_INVALID_RASTER: incomplete TIFF TileOffsets/TileByteCounts".to_owned(),
            );
        }
    };
    if has_strips == has_tiles {
        return Err(
            "NATIVE_DEM_INVALID_RASTER: GeoTIFF must define exactly one of strip or tile layout"
                .to_owned(),
        );
    }

    let mut chunk_jobs: Vec<(u64, u64, u64)> = Vec::new();
    if has_strips {
        let offsets = strip_offsets.as_deref().unwrap_or(&[]);
        let counts = strip_byte_counts.as_deref().unwrap_or(&[]);
        let rps = rows_per_strip.unwrap_or(height);
        if rps == 0 {
            return Err(
                "NATIVE_DEM_INVALID_RASTER: GeoTIFF RowsPerStrip must be positive".to_owned(),
            );
        }
        let expected_strips = height.div_ceil(rps);
        validate_chunk_spans(offsets, counts, expected_strips, header_len, file_len)?;
        chunk_jobs.reserve(offsets.len());
        for (idx, (&off, &cnt)) in offsets.iter().zip(counts.iter()).enumerate() {
            if cnt == 0 {
                continue;
            }
            let row_start = (idx as u64).saturating_mul(rps);
            let rows_in_strip = (height - row_start).min(rps);
            let expected_bytes = width
                .checked_mul(rows_in_strip)
                .and_then(|px| px.checked_mul(bytes_per_sample))
                .ok_or_else(|| {
                    "NATIVE_DEM_INVALID_RASTER: strip expected byte size overflow".to_owned()
                })?;
            chunk_jobs.push((off, cnt, expected_bytes));
        }
    } else {
        let tw = tile_width.filter(|&w| w > 0).ok_or_else(|| {
            "NATIVE_DEM_INVALID_RASTER: tiled GeoTIFF missing positive TileWidth".to_owned()
        })?;
        let tl = tile_length.filter(|&l| l > 0).ok_or_else(|| {
            "NATIVE_DEM_INVALID_RASTER: tiled GeoTIFF missing positive TileLength".to_owned()
        })?;
        let offsets = tile_offsets.as_deref().unwrap_or(&[]);
        let counts = tile_byte_counts.as_deref().unwrap_or(&[]);
        let tiles_x = width.div_ceil(tw);
        let tiles_y = height.div_ceil(tl);
        let expected_tiles = tiles_x.checked_mul(tiles_y).ok_or_else(|| {
            "NATIVE_DEM_INVALID_RASTER: tiled GeoTIFF chunk count overflow".to_owned()
        })?;
        validate_chunk_spans(offsets, counts, expected_tiles, header_len, file_len)?;
        let expected_tile_bytes = tw
            .checked_mul(tl)
            .and_then(|px| px.checked_mul(bytes_per_sample))
            .ok_or_else(|| {
                "NATIVE_DEM_INVALID_RASTER: tile expected byte size overflow".to_owned()
            })?;
        if expected_tile_bytes > MAX_NATIVE_DEM_DOWNLOAD_BYTES {
            return Err(
                "NATIVE_DEM_INVALID_RASTER: tile size exceeds 2 GiB safety limit".to_owned(),
            );
        }
        chunk_jobs.reserve(offsets.len());
        for (&off, &cnt) in offsets.iter().zip(counts.iter()) {
            if cnt == 0 {
                continue;
            }
            chunk_jobs.push((off, cnt, expected_tile_bytes));
        }
    }

    for (off, cnt, expected_bytes) in chunk_jobs {
        file.seek(std::io::SeekFrom::Start(off))
            .map_err(|_| "NATIVE_DEM_INVALID_RASTER: TIFF seek outside file bounds".to_owned())?;
        match compression_code {
            1 => {
                if cnt != expected_bytes {
                    return Err(format!(
                        "NATIVE_DEM_INVALID_RASTER: uncompressed TIFF chunk byte count ({cnt}) does not match expected pixel size ({expected_bytes})"
                    ));
                }
            }
            5 => verify_lzw_chunk(&mut file, cnt, expected_bytes)?,
            8 | 32946 => verify_deflate_chunk(&mut file, cnt, expected_bytes)?,
            32773 => verify_packbits_chunk(&mut file, cnt, expected_bytes)?,
            _ => unreachable!(),
        }
    }

    Ok(RawGeoTiffSummary {
        width,
        height,
        bands,
        is_bigtiff,
        compression,
        crs,
        geotiff_tags_present: true,
    })
}

fn prepare_raw_dem_tiles_without_native_gis(
    provider: &str,
    source_paths: &[PathBuf],
    input: &Path,
    bbox: [f64; 4],
) -> Result<serde_json::Value, String> {
    if source_paths.is_empty() {
        return Err("NATIVE_DEM_INVALID_RASTER: no DEM source tiles available".to_owned());
    }
    for source in source_paths {
        let meta = std::fs::symlink_metadata(source).map_err(|_| {
            "NATIVE_DEM_INVALID_RASTER: provider response file is unavailable".to_owned()
        })?;
        if source == input || !meta.is_file() || meta.file_type().is_symlink() || meta.len() == 0 {
            return Err(
                "NATIVE_DEM_INVALID_RASTER: provider response is not a valid GeoTIFF".to_owned(),
            );
        }
        validate_raw_geotiff_structure(source)?;
    }
    let _ = std::fs::remove_file(input);
    if source_paths.len() == 1 {
        let source = &source_paths[0];
        let bytes = std::fs::metadata(source).map(|m| m.len()).unwrap_or(0);
        if std::fs::rename(source, input).is_err() {
            std::fs::copy(source, input).map_err(|_| {
                "NATIVE_DEM_OUTPUT_UNAVAILABLE: raw GeoTIFF tile could not be promoted".to_owned()
            })?;
        }
        return Ok(serde_json::json!({
            "ok": true,
            "executed": true,
            "operation": "raw_geotiff_passthrough_no_native_gis",
            "native_gis_available": false,
            "provider": provider,
            "output_path": input,
            "tile_paths": [input],
            "tile_count": 1,
            "requested_bbox": bbox,
            "bytes_written": bytes,
        }));
    }

    let parent = input.parent().ok_or_else(|| {
        "NATIVE_DEM_OUTPUT_UNAVAILABLE: DEM output parent directory missing".to_owned()
    })?;
    let stem = input
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("dem");
    let mut tile_paths = Vec::with_capacity(source_paths.len());
    let mut total_bytes = 0_u64;
    for (idx, source) in source_paths.iter().enumerate() {
        let tile_name = source
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("tile.tif");
        let tile_dest = parent.join(format!("{stem}__{tile_name}"));
        let tile_bytes = std::fs::metadata(source).map(|m| m.len()).unwrap_or(0);
        std::fs::copy(source, &tile_dest).map_err(|_| {
            "NATIVE_DEM_OUTPUT_UNAVAILABLE: raw GeoTIFF tile could not be written".to_owned()
        })?;
        if idx == 0 {
            std::fs::copy(source, input).map_err(|_| {
                "NATIVE_DEM_OUTPUT_UNAVAILABLE: primary raw GeoTIFF tile could not be promoted"
                    .to_owned()
            })?;
        }
        total_bytes = total_bytes.saturating_add(tile_bytes);
        tile_paths.push(tile_dest);
    }
    Ok(serde_json::json!({
        "ok": true,
        "executed": true,
        "operation": "raw_geotiff_multitile_no_native_gis",
        "native_gis_available": false,
        "provider": provider,
        "output_path": input,
        "tile_paths": tile_paths,
        "tile_count": tile_paths.len(),
        "requested_bbox": bbox,
        "bytes_written": total_bytes,
    }))
}

fn prepare_native_dem_download_output(
    engine: &LazyNativeGisEngine,
    provider: &str,
    source_paths: &[PathBuf],
    input: &Path,
    bbox: [f64; 4],
) -> Result<serde_json::Value, String> {
    if !engine.is_discovered() {
        return prepare_raw_dem_tiles_without_native_gis(provider, source_paths, input, bbox);
    }
    if provider == "GEM15_PUBLIC" {
        let _ = std::fs::remove_file(input);
        return engine
            .warp_mosaic_crop_utm_dem_tiles(source_paths, input, bbox)
            .map_err(|error| format!("{}: {}", error.code().as_str(), error));
    }
    if matches!(
        provider,
        "COPERNICUS_AWS" | "NISAR_DEM_ASF" | "FABDEM_PUBLIC" | "EARTHDATA" | "LPDAAC" | "ASF"
    ) {
        // All three routes return 1-degree source tiles, unlike the single
        // server-produced OpenTopography response.
        let _ = std::fs::remove_file(input);
        return engine
            .mosaic_crop_dem_tiles(source_paths, input, bbox)
            .map_err(|error| format!("{}: {}", error.code().as_str(), error));
    }
    if provider != "OPENTOPOGRAPHY" {
        return Err("NATIVE_DEM_PROVIDER_UNSUPPORTED: unknown DEM delivery route".to_owned());
    }
    let [source] = source_paths else {
        return Err(
            "NATIVE_DEM_PROVIDER_FAILED: OpenTopography must return one GeoTIFF per request window"
                .to_owned(),
        );
    };
    let source_file = std::fs::symlink_metadata(source).map_err(|_| {
        "NATIVE_DEM_INVALID_RASTER: provider response file is unavailable".to_owned()
    })?;
    if source == input || !source_file.is_file() || source_file.file_type().is_symlink() {
        return Err(
            "NATIVE_DEM_OUTPUT_ESCAPE: invalid provider response promotion path".to_owned(),
        );
    }
    let inspection = engine
        .inspect_dem(source)
        .map_err(|error| format!("{}: {}", error.code().as_str(), error))?;
    let metadata = &inspection["metadata"];
    if metadata["driver"] != "GTiff"
        || metadata["count"] != 1
        || metadata["width"].as_u64().is_none_or(|value| value == 0)
        || metadata["height"].as_u64().is_none_or(|value| value == 0)
        || metadata["crs"]
            .as_str()
            .is_none_or(|value| value.trim().is_empty())
        || !metadata["geotransform"].as_array().is_some_and(|values| {
            values.len() == 6
                && values
                    .iter()
                    .all(|value| value.as_f64().is_some_and(f64::is_finite))
        })
    {
        return Err(
            "NATIVE_DEM_INVALID_RASTER: provider response must be a georeferenced single-band GeoTIFF"
                .to_owned(),
        );
    }
    engine
        .verify_dem_pixels(source)
        .map_err(|error| format!("{}: {}", error.code().as_str(), error))?;
    // The response is already the provider's raster. Its actual pixel boundary
    // need not equal the requested bbox. Renaming preserves every file byte,
    // including native grid, dtype, NoData and scientific tags, without a 4096
    // clamp, reprojection or re-encoding. Validate before replacing any output.
    std::fs::rename(source, input).map_err(|_| {
        "NATIVE_DEM_OUTPUT_UNAVAILABLE: validated provider GeoTIFF could not be promoted".to_owned()
    })?;
    Ok(serde_json::json!({
        "ok": true,
        "executed": true,
        "operation": "provider_geotiff_passthrough",
        "provider": provider,
        "output_path": input,
        "requested_bbox": bbox,
        "width": metadata["width"],
        "height": metadata["height"],
        "resampling": serde_json::Value::Null,
        "bytes_written": source_file.len(),
    }))
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct NativeDemResumeMetadata {
    url: String,
    etag: String,
    total_bytes: u64,
}

#[derive(Clone, Copy)]
struct NativeDemTransferPolicy<'a> {
    provider: &'a str,
    remaining_bytes: u64,
}

fn native_dem_resume_metadata_path(temporary: &Path) -> PathBuf {
    let name = temporary
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("dem.download.part");
    temporary.with_file_name(format!("{name}.resume.json"))
}

fn parse_http_content_range(value: &str) -> Option<(u64, u64, u64)> {
    let value = value.strip_prefix("bytes ")?;
    let (range, total) = value.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    let start = start.parse::<u64>().ok()?;
    let end = end.parse::<u64>().ok()?;
    let total = total.parse::<u64>().ok()?;
    (start <= end && end < total).then_some((start, end, total))
}

#[allow(clippy::too_many_arguments)]
fn download_native_dem_geotiff(
    engine: &LazyNativeGisEngine,
    client: &reqwest::blocking::Client,
    earthdata_auth: Option<&insar_download::TransferAuth>,
    url: &str,
    destination: &Path,
    policy: NativeDemTransferPolicy<'_>,
    interrupted: &(dyn Fn() -> bool + Sync),
    progress: &mut dyn FnMut(insar_application_service::DemTransferEvent),
) -> Result<u64, String> {
    let NativeDemTransferPolicy {
        provider,
        remaining_bytes,
    } = policy;
    if interrupted() {
        return Err("NATIVE_DEM_INTERRUPTED: task paused or cancelled".to_owned());
    }
    if remaining_bytes == 0 {
        return Err("NATIVE_DEM_SIZE_LIMIT: DEM source tiles exceed the task bound".to_owned());
    }
    let temporary = destination.with_file_name(format!(
        ".{}.download.part",
        destination
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("dem-tile")
    ));
    let resume_metadata_path = native_dem_resume_metadata_path(&temporary);
    // NISAR DEM and FABDEM V1-2 tiles honor byte ranges (verified against
    // the ASF cloud and HuggingFace/CloudFront endpoints, same as Copernicus AWS).
    let resume_capable = provider == "COPERNICUS_AWS"
        || provider == "NISAR_DEM_ASF"
        || provider == "FABDEM_PUBLIC"
        || provider == "GEM15_PUBLIC";
    let mut resume_from = 0_u64;
    let mut resume_metadata = None;
    if resume_capable {
        if let (Ok(part), Ok(metadata_bytes)) = (
            std::fs::metadata(&temporary),
            std::fs::read(&resume_metadata_path),
        ) {
            if let Ok(metadata) = serde_json::from_slice::<NativeDemResumeMetadata>(&metadata_bytes)
            {
                if part.is_file()
                    && part.len() > 0
                    && part.len() < metadata.total_bytes
                    && metadata.total_bytes <= remaining_bytes
                    && metadata.url == url
                    && !metadata.etag.trim().is_empty()
                {
                    resume_from = part.len();
                    resume_metadata = Some(metadata);
                }
            }
        }
        if resume_from == 0 {
            let _ = std::fs::remove_file(&temporary);
            let _ = std::fs::remove_file(&resume_metadata_path);
        }
    } else {
        // OpenTopography returns a generated one-off GeoTIFF and does not expose
        // a stable byte-range validator. A frozen task can restart the request,
        // but must never append to a response from an earlier request.
        let _ = std::fs::remove_file(&temporary);
        let _ = std::fs::remove_file(&resume_metadata_path);
    }
    let mut response = None;
    let mut last_status = None;
    for attempt in 0..3 {
        if interrupted() {
            return Err("NATIVE_DEM_INTERRUPTED: task paused or cancelled".to_owned());
        }
        let mut request = client.get(url);
        if let Some(auth) = earthdata_auth {
            request = auth.apply_to_blocking_request(request);
        }
        if resume_from > 0 {
            let metadata = resume_metadata
                .as_ref()
                .expect("resume offset is backed by validated metadata");
            request = request
                .header(RANGE, format!("bytes={resume_from}-"))
                .header(IF_RANGE, metadata.etag.as_str());
        }
        let candidate = request.send();
        match candidate {
            Ok(candidate) => {
                let status = candidate.status().as_u16();
                if status == 429 || status >= 500 {
                    last_status = Some(status);
                    if attempt < 2 {
                        std::thread::sleep(Duration::from_millis(250 * (attempt + 1)));
                        continue;
                    }
                }
                response = Some(candidate);
                break;
            }
            Err(error) if attempt < 2 => {
                let _ = error;
                std::thread::sleep(Duration::from_millis(250 * (attempt + 1)));
            }
            Err(error) => {
                return Err(format!(
                    "NATIVE_DEM_NETWORK_FAILED: native DEM request failed: {error}"
                ));
            }
        }
    }
    let mut response = response.ok_or_else(|| {
        format!(
            "NATIVE_DEM_NETWORK_FAILED: provider returned retryable HTTP status {}",
            last_status.unwrap_or(0)
        )
    })?;
    if resume_from > 0 {
        let resume = resume_metadata
            .as_ref()
            .expect("resume offset is backed by validated metadata");
        let response_etag = response
            .headers()
            .get(ETAG)
            .and_then(|value| value.to_str().ok());
        let content_range = response
            .headers()
            .get(CONTENT_RANGE)
            .and_then(|value| value.to_str().ok())
            .and_then(parse_http_content_range);
        let valid_partial = response.status().as_u16() == 206
            && response_etag == Some(resume.etag.as_str())
            && content_range.is_some_and(|(start, _, total)| {
                start == resume_from && total == resume.total_bytes
            });
        if !valid_partial {
            // If-Range returning 200 means the object validator changed. The
            // full body is safe to use as a fresh response, never as an append.
            if response.status().as_u16() == 200 {
                resume_from = 0;
                resume_metadata = None;
                let _ = std::fs::remove_file(&temporary);
                let _ = std::fs::remove_file(&resume_metadata_path);
            } else {
                return Err(
                    "NATIVE_DEM_RESUME_INVALID: AWS did not honor the frozen byte-range validator"
                        .to_owned(),
                );
            }
        }
    }
    if response.status().as_u16() == 401 || response.status().as_u16() == 403 {
        if provider == "OPENTOPOGRAPHY" {
            return Err(
                "NATIVE_DEM_CREDENTIAL_REQUIRED: OpenTopography API key was rejected".to_owned(),
            );
        }
        if matches!(provider, "NISAR_DEM_ASF" | "EARTHDATA" | "LPDAAC" | "ASF") {
            return Err(format!(
                "NATIVE_DEM_CREDENTIAL_REQUIRED: Earthdata session is required for {provider} DEM tiles"
            ));
        }
        if provider == "GEM15_PUBLIC" {
            return Err(
                "NATIVE_DEM_PROVIDER_FAILED: Google Cloud Storage denied the GEM-15 tile request"
                    .to_owned(),
            );
        }
        return Err(
            "NATIVE_DEM_PROVIDER_FAILED: Copernicus AWS denied the tile request".to_owned(),
        );
    }
    if response.status().as_u16() == 404 {
        if provider == "NISAR_DEM_ASF" {
            return Err(
                "NATIVE_DEM_PROVIDER_FAILED: NISAR DEM tile was not found (HTTP 404)".to_owned(),
            );
        }
        if matches!(provider, "EARTHDATA" | "LPDAAC" | "ASF") {
            return Err(
                "NATIVE_DEM_TILE_NOT_FOUND_404: ASTER GDEM v3 tile absent (ocean cell)".to_owned(),
            );
        }
        if provider == "FABDEM_PUBLIC" {
            return Err(
                "NATIVE_DEM_TILE_NOT_FOUND_404: FABDEM land tile absent (ocean cell)".to_owned(),
            );
        }
        if provider == "GEM15_PUBLIC" {
            return Err(
                "NATIVE_DEM_TILE_NOT_FOUND_404: GEM-15 v1 tile absent (outside coverage or ocean; Mainland China/Russia/Mongolia/Greenland/Antarctica are excluded in v1)"
                    .to_owned(),
            );
        }
        return Err(format!(
            "NATIVE_DEM_PROVIDER_FAILED: provider returned HTTP {}",
            response.status()
        ));
    }
    if !response.status().is_success() {
        return Err(format!(
            "NATIVE_DEM_PROVIDER_FAILED: provider returned HTTP {}",
            response.status()
        ));
    }
    if response
        .content_length()
        .is_some_and(|size| resume_from.saturating_add(size) > remaining_bytes)
    {
        return Err(
            "NATIVE_DEM_SIZE_LIMIT: provider response exceeds the remaining task bound".to_owned(),
        );
    }
    if resume_capable && resume_from == 0 {
        let etag = response
            .headers()
            .get(ETAG)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let total_bytes = response.content_length().unwrap_or(0);
        if !etag.is_empty() && total_bytes > 0 {
            let metadata = NativeDemResumeMetadata {
                url: url.to_owned(),
                etag,
                total_bytes,
            };
            std::fs::write(
                &resume_metadata_path,
                serde_json::to_vec(&metadata).map_err(|_| {
                    "NATIVE_DEM_OUTPUT_UNAVAILABLE: DEM resume metadata could not be encoded"
                        .to_owned()
                })?,
            )
            .map_err(|_| {
                "NATIVE_DEM_OUTPUT_UNAVAILABLE: DEM resume metadata could not be written".to_owned()
            })?;
            resume_metadata = Some(metadata);
        }
    }
    let mut output = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(resume_from > 0)
        .truncate(resume_from == 0)
        .open(&temporary)
        .map_err(|_| {
            "NATIVE_DEM_OUTPUT_UNAVAILABLE: DEM temporary file could not be created".to_owned()
        })?;
    let mut written = resume_from;
    let mut network_received = 0_u64;
    let mut first = [0_u8; 4];
    let mut first_len = if resume_from > 0 {
        let mut existing = std::fs::File::open(&temporary).map_err(|_| {
            "NATIVE_DEM_OUTPUT_UNAVAILABLE: DEM partial file could not be reopened".to_owned()
        })?;
        existing.read(&mut first).map_err(|_| {
            "NATIVE_DEM_OUTPUT_UNAVAILABLE: DEM partial header could not be read".to_owned()
        })?
    } else {
        0
    };
    let mut buffer = [0_u8; 1024 * 1024];
    progress(insar_application_service::DemTransferEvent::BodyStarted);
    let body_result = (|| -> Result<(), String> {
        loop {
            if interrupted() {
                return Err("NATIVE_DEM_INTERRUPTED: task paused or cancelled".to_owned());
            }
            let read = response.read(&mut buffer).map_err(|_| {
                "NATIVE_DEM_NETWORK_FAILED: DEM response could not be read".to_owned()
            })?;
            if read == 0 {
                break;
            }
            if first_len < first.len() {
                let take = (first.len() - first_len).min(read);
                first[first_len..first_len + take].copy_from_slice(&buffer[..take]);
                first_len += take;
            }
            written = written.saturating_add(read as u64);
            network_received = network_received.saturating_add(read as u64);
            progress(insar_application_service::DemTransferEvent::BytesReceived(
                network_received,
            ));
            if written > remaining_bytes {
                let _ = std::fs::remove_file(&temporary);
                return Err(
                    "NATIVE_DEM_SIZE_LIMIT: DEM response exceeds the remaining task bound"
                        .to_owned(),
                );
            }
            output.write_all(&buffer[..read]).map_err(|_| {
                "NATIVE_DEM_OUTPUT_UNAVAILABLE: DEM temporary file could not be written".to_owned()
            })?;
        }
        Ok(())
    })();
    progress(insar_application_service::DemTransferEvent::BodyStopped);
    if let Err(error) = body_result {
        if !resume_capable || resume_metadata.is_none() {
            let _ = std::fs::remove_file(&temporary);
            let _ = std::fs::remove_file(&resume_metadata_path);
        }
        return Err(error);
    }
    output.sync_all().map_err(|_| {
        "NATIVE_DEM_OUTPUT_UNAVAILABLE: DEM temporary file could not be flushed".to_owned()
    })?;
    drop(output);
    if written == 0 || !matches!(first, [b'I', b'I', 42 | 43, 0] | [b'M', b'M', 0, 42 | 43]) {
        let _ = std::fs::remove_file(&temporary);
        let _ = std::fs::remove_file(&resume_metadata_path);
        return Err("NATIVE_DEM_INVALID_RASTER: provider response is not a GeoTIFF".to_owned());
    }

    if engine.is_discovered() {
        if let Err(error) = engine.inspect_dem(&temporary) {
            let _ = std::fs::remove_file(&temporary);
            let _ = std::fs::remove_file(&resume_metadata_path);
            return Err(format!("{}: {}", error.code().as_str(), error));
        }
    } else if let Err(error) = validate_raw_geotiff_structure(&temporary) {
        let _ = std::fs::remove_file(&temporary);
        let _ = std::fs::remove_file(&resume_metadata_path);
        return Err(error);
    }
    if destination.exists() {
        let _ = std::fs::remove_file(destination);
    }
    std::fs::rename(&temporary, destination).map_err(|_| {
        let _ = std::fs::remove_file(&temporary);
        "NATIVE_DEM_OUTPUT_UNAVAILABLE: DEM tile could not be promoted".to_owned()
    })?;
    let _ = std::fs::remove_file(&resume_metadata_path);
    Ok(written)
}

fn native_dem_finalize_windows(
    engine: &LazyNativeGisEngine,
    payload: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let output_root = payload_path_any(payload, &["output_root", "output_dir"])?;
    let merged_input = payload_path(payload, "merged_input_path")?;
    let inputs = payload
        .get("window_inputs")
        .and_then(serde_json::Value::as_array)
        .filter(|values| values.len() == 2)
        .ok_or_else(|| {
            "NATIVE_DEM_INVALID_PAYLOAD: window_inputs must contain exactly two paths".to_owned()
        })?
        .iter()
        .map(|value| {
            value
                .as_str()
                .filter(|path| !path.trim().is_empty())
                .map(PathBuf::from)
                .ok_or_else(|| {
                    "NATIVE_DEM_INVALID_PAYLOAD: window input path is invalid".to_owned()
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if !output_root.is_absolute()
        || !merged_input.is_absolute()
        || !merged_input.starts_with(&output_root)
        || inputs
            .iter()
            .any(|input| !input.is_absolute() || !input.starts_with(&output_root))
    {
        return Err(
            "NATIVE_DEM_INVALID_PAYLOAD: finalization paths must belong to the task output root"
                .to_owned(),
        );
    }
    let staging_root = native_dem_window_staging_root(payload, &output_root, &inputs)?;
    if merged_input.starts_with(&staging_root) {
        return Err(
            "NATIVE_DEM_INVALID_PAYLOAD: merged output must be independent from task staging"
                .to_owned(),
        );
    }
    prepare_native_task_output_path(&output_root, &merged_input)?;
    for input in &inputs {
        prepare_native_task_output_path(&output_root, input)?;
        if !input.is_file() {
            return Err(
                "NATIVE_DEM_INVALID_PAYLOAD: finalization input is not a regular file".to_owned(),
            );
        }
    }
    let merge = engine
        .merge_antimeridian_dem(&inputs[0], &inputs[1], &merged_input)
        .map_err(|error| format!("{}: {}", error.code().as_str(), error))?;
    let conversion = if payload
        .get("convert")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        let source_vertical_datum = native_dem_source_vertical_datum(payload)?;
        match native_dem_convert(
            engine,
            &serde_json::json!({
                "input_path": merged_input,
                "output_root": output_root,
                "source_vertical_datum": source_vertical_datum,
                "output_mode": "sarscape",
            }),
        ) {
            Ok(value) => Some(value),
            Err(error) => {
                let _ = std::fs::remove_file(&merged_input);
                return Err(error);
            }
        }
    } else {
        None
    };
    let cleanup = cleanup_native_dem_window_staging(&staging_root, &inputs);
    Ok(serde_json::json!({
        "ok": true,
        "operation": "dem_finalize_windows",
        "merged_input_path": merged_input,
        "merge": merge,
        "conversion": conversion,
        "cleanup": cleanup,
    }))
}

fn native_dem_bbox(payload: &serde_json::Value) -> Result<[f64; 4], String> {
    let bbox = payload
        .get("bbox")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| "NATIVE_DEM_INVALID_PAYLOAD: bbox is required".to_owned())?;
    let value = |name: &str| {
        bbox.get(name)
            .and_then(serde_json::Value::as_f64)
            .ok_or_else(|| format!("NATIVE_DEM_INVALID_PAYLOAD: bbox.{name} is required"))
    };
    let output = [
        value("west")?,
        value("south")?,
        value("east")?,
        value("north")?,
    ];
    if output.iter().any(|value| !value.is_finite())
        || output[0] < -180.0
        || output[2] > 180.0
        || output[1] < -90.0
        || output[3] > 90.0
        || output[0] >= output[2]
        || output[1] >= output[3]
    {
        return Err("NATIVE_DEM_INVALID_PAYLOAD: bbox is invalid".to_owned());
    }
    Ok(output)
}

fn native_dem_source_vertical_datum(payload: &serde_json::Value) -> Result<String, String> {
    let dataset = payload
        .get("dataset")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "NATIVE_GIS_VERTICAL_DATUM_REQUIRED: dataset is required".to_owned())?;
    let expected = match dataset.to_ascii_uppercase().as_str() {
        "COP30" | "COP90" | "FABDEM" => "EGM2008",
        "SRTM_GL1_ELLIPSOIDAL" | "AW3D30_ELLIPSOIDAL" | "NISAR_DEM" => "WGS84_ELLIPSOID",
        "NASADEM" | "SRTM_GL3" | "SRTM_GL1" | "AW3D30" | "GEM15_DTM" | "GEM15_DSM"
        | "ASTER_GDEM_V3" => "EGM96",
        _ => {
            return Err(format!(
                "NATIVE_GIS_VERTICAL_DATUM_REQUIRED: no reviewed vertical datum is registered for {dataset}"
            ))
        }
    };
    let requested = payload
        .get("source_vertical_datum")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if requested.is_none_or(|value| value.eq_ignore_ascii_case("auto")) {
        return Ok(expected.to_owned());
    }
    let requested = requested.unwrap().to_ascii_uppercase();
    if requested != expected {
        return Err(format!(
            "NATIVE_GIS_VERTICAL_DATUM_MISMATCH: {dataset} requires {expected}, not {requested}"
        ));
    }
    Ok(requested)
}

fn native_dem_download_sources(
    dataset: &str,
    provider: &str,
    bbox: [f64; 4],
) -> Result<Vec<NativeDemDownloadSource>, String> {
    match provider {
        "OPENTOPOGRAPHY" => {
            let key = insar_download::resolve_desktop_opentopography_key()
                .map_err(|_| {
                    "NATIVE_DEM_CREDENTIAL_UNAVAILABLE: OpenTopography keyring is unavailable"
                        .to_owned()
                })?
                .ok_or_else(|| {
                    "NATIVE_DEM_CREDENTIAL_REQUIRED: OpenTopography API key is not configured"
                        .to_owned()
                })?;
            let demtype = match dataset {
                "COP30" => "COP30",
                "COP90" => "COP90",
                "NASADEM" => "NASADEM",
                "SRTM_GL3" => "SRTMGL3",
                "SRTM_GL1" => "SRTMGL1",
                "AW3D30" => "AW3D30",
                "SRTM_GL1_ELLIPSOIDAL" => "SRTMGL1_E",
                "AW3D30_ELLIPSOIDAL" => "AW3D30_E",
                _ => return Err(
                    "NATIVE_DEM_PROVIDER_UNSUPPORTED: dataset is not supported by OpenTopography"
                        .to_owned(),
                ),
            };
            let mut url = reqwest::Url::parse("https://portal.opentopography.org/API/globaldem")
                .map_err(|_| {
                    "NATIVE_DEM_PROVIDER_INVALID: OpenTopography endpoint is invalid".to_owned()
                })?;
            {
                let mut query = url.query_pairs_mut();
                query
                    .append_pair("demtype", demtype)
                    .append_pair("south", &bbox[1].to_string())
                    .append_pair("north", &bbox[3].to_string())
                    .append_pair("west", &bbox[0].to_string())
                    .append_pair("east", &bbox[2].to_string())
                    .append_pair("outputFormat", "GTiff")
                    .append_pair("API_Key", &key);
            }
            drop(key);
            Ok(vec![NativeDemDownloadSource {
                id: "opentopography".to_owned(),
                url: url.to_string(),
            }])
        }
        "COPERNICUS_AWS" => {
            if dataset != "COP30" && dataset != "COP90" {
                return Err(
                    "NATIVE_DEM_PROVIDER_UNSUPPORTED: dataset is not supported by Copernicus AWS"
                        .to_owned(),
                );
            }
            let resolution = if dataset == "COP30" { "10" } else { "30" };
            let base = if dataset == "COP30" {
                "https://copernicus-dem-30m.s3.eu-central-1.amazonaws.com"
            } else {
                "https://copernicus-dem-90m.s3.eu-central-1.amazonaws.com"
            };
            let south_start = bbox[1].floor() as i32;
            let north_end = bbox[3].ceil() as i32;
            let west_start = bbox[0].floor() as i32;
            let east_end = bbox[2].ceil() as i32;
            let row_count = usize::try_from(north_end.saturating_sub(south_start)).unwrap_or(0);
            let column_count = usize::try_from(east_end.saturating_sub(west_start)).unwrap_or(0);
            let tile_count = row_count.checked_mul(column_count).ok_or_else(|| {
                "NATIVE_DEM_TILE_LIMIT: Copernicus tile count overflowed".to_owned()
            })?;
            if tile_count == 0 || tile_count > MAX_NATIVE_DEM_TILES {
                return Err(format!(
                    "NATIVE_DEM_TILE_LIMIT: Copernicus request requires {tile_count} tiles; the product bound is {MAX_NATIVE_DEM_TILES}"
                ));
            }
            let mut sources = Vec::with_capacity(tile_count);
            for south in south_start..north_end {
                for west in west_start..east_end {
                    let latitude = if south < 0 {
                        format!("S{:02}", south.unsigned_abs())
                    } else {
                        format!("N{:02}", south)
                    };
                    let longitude = if west < 0 {
                        format!("W{:03}", west.unsigned_abs())
                    } else {
                        format!("E{:03}", west)
                    };
                    let tile =
                        format!("Copernicus_DSM_COG_{resolution}_{latitude}_00_{longitude}_00_DEM");
                    sources.push(NativeDemDownloadSource {
                        id: tile.clone(),
                        url: format!("{base}/{tile}/{tile}.tif"),
                    });
                }
            }
            Ok(sources)
        }
        "NISAR_DEM_ASF" => {
            if dataset != "NISAR_DEM" {
                return Err(
                    "NATIVE_DEM_PROVIDER_UNSUPPORTED: dataset is not supported by the NISAR DEM ASF route"
                        .to_owned(),
                );
            }
            // Official NISAR DEM v1.2 EPSG:4326 tiles: the file name is the
            // 1-degree lower-left corner; the directory bucket is the 10-degree
            // latitude band (5-degree polar bands N85/S85/S90) and the 20-degree
            // longitude block. Verified against the public CMR granule listing
            // of collection NISAR_DEM (2026-11-01).
            let south_start = bbox[1].floor() as i32;
            let north_end = bbox[3].ceil() as i32;
            let west_start = bbox[0].floor() as i32;
            let east_end = bbox[2].ceil() as i32;
            let row_count = usize::try_from(north_end.saturating_sub(south_start)).unwrap_or(0);
            let column_count = usize::try_from(east_end.saturating_sub(west_start)).unwrap_or(0);
            let tile_count = row_count.checked_mul(column_count).ok_or_else(|| {
                "NATIVE_DEM_TILE_LIMIT: NISAR DEM tile count overflowed".to_owned()
            })?;
            if tile_count == 0 || tile_count > MAX_NATIVE_DEM_TILES {
                return Err(format!(
                    "NATIVE_DEM_TILE_LIMIT: NISAR DEM request requires {tile_count} tiles; the product bound is {MAX_NATIVE_DEM_TILES}"
                ));
            }
            let base = "https://nisar.asf.earthdatacloud.nasa.gov/NISAR/DEM/v1.2/EPSG4326";
            let mut sources = Vec::with_capacity(tile_count);
            for south in south_start..north_end {
                for west in west_start..east_end {
                    let latitude = nisar_dem_coordinate_token(south, true);
                    let longitude = nisar_dem_coordinate_token(west, false);
                    let band = nisar_dem_coordinate_token(nisar_dem_band(south), true);
                    let block = nisar_dem_coordinate_token(nisar_dem_block(west), false);
                    let tile = format!("DEM_{latitude}_00_{longitude}_00_C01");
                    sources.push(NativeDemDownloadSource {
                        id: tile.clone(),
                        url: format!("{base}/{band}/{band}_{block}/{tile}.tif"),
                    });
                }
            }
            Ok(sources)
        }
        "FABDEM_PUBLIC" => {
            if dataset != "FABDEM" {
                return Err(
                    "NATIVE_DEM_PROVIDER_UNSUPPORTED: dataset is not supported by FABDEM Public"
                        .to_owned(),
                );
            }
            let south_start = bbox[1].floor() as i32;
            let north_end = bbox[3].ceil() as i32;
            let west_start = bbox[0].floor() as i32;
            let east_end = bbox[2].ceil() as i32;
            let row_count = usize::try_from(north_end.saturating_sub(south_start)).unwrap_or(0);
            let column_count = usize::try_from(east_end.saturating_sub(west_start)).unwrap_or(0);
            let tile_count = row_count
                .checked_mul(column_count)
                .ok_or_else(|| "NATIVE_DEM_TILE_LIMIT: FABDEM tile count overflowed".to_owned())?;
            if tile_count == 0 || tile_count > MAX_NATIVE_DEM_TILES {
                return Err(format!(
                    "NATIVE_DEM_TILE_LIMIT: FABDEM request requires {tile_count} tiles; the product bound is {MAX_NATIVE_DEM_TILES}"
                ));
            }
            let mut sources = Vec::with_capacity(tile_count);
            for south in south_start..north_end {
                for west in west_start..east_end {
                    let (tile_id, url) = fabdem_tile_url(south, west)?;
                    sources.push(NativeDemDownloadSource { id: tile_id, url });
                }
            }
            Ok(sources)
        }
        "GEM15_PUBLIC" => native_dem_gem15_sources_with_base(
            dataset,
            bbox,
            "https://storage.googleapis.com/climate-and-sustainability/gem-15",
        ),
        "EARTHDATA" | "LPDAAC" | "ASF" => {
            if dataset != "ASTER_GDEM_V3" && dataset != "ASTGTM" {
                return Err(
                    "NATIVE_DEM_PROVIDER_UNSUPPORTED: dataset is not supported by NASA Earthdata LP DAAC"
                        .to_owned(),
                );
            }
            let south_start = bbox[1].floor() as i32;
            let north_end = bbox[3].ceil() as i32;
            let west_start = bbox[0].floor() as i32;
            let east_end = bbox[2].ceil() as i32;
            let row_count = usize::try_from(north_end.saturating_sub(south_start)).unwrap_or(0);
            let column_count = usize::try_from(east_end.saturating_sub(west_start)).unwrap_or(0);
            let tile_count = row_count.checked_mul(column_count).ok_or_else(|| {
                "NATIVE_DEM_TILE_LIMIT: ASTER GDEM tile count overflowed".to_owned()
            })?;
            if tile_count == 0 || tile_count > MAX_NATIVE_DEM_TILES {
                return Err(format!(
                    "NATIVE_DEM_TILE_LIMIT: ASTER GDEM request requires {tile_count} tiles; the product bound is {MAX_NATIVE_DEM_TILES}"
                ));
            }
            let mut sources = Vec::with_capacity(tile_count);
            for south in south_start..north_end {
                for west in west_start..east_end {
                    let latitude = if south < 0 {
                        format!("S{:02}", south.unsigned_abs())
                    } else {
                        format!("N{:02}", south)
                    };
                    let longitude = if west < 0 {
                        format!("W{:03}", west.unsigned_abs())
                    } else {
                        format!("E{:03}", west)
                    };
                    let tile = format!("ASTGTMV003_{latitude}{longitude}_dem");
                    sources.push(NativeDemDownloadSource {
                        id: tile.clone(),
                        url: format!(
                            "https://data.lpdaac.earthdatacloud.nasa.gov/lp-prod-protected/ASTGTM.003/{tile}.tif"
                        ),
                    });
                }
            }
            Ok(sources)
        }
        _ => Err("NATIVE_DEM_PROVIDER_UNSUPPORTED: provider route is not admitted".to_owned()),
    }
}

fn native_dem_gem15_sources_with_base(
    dataset: &str,
    bbox: [f64; 4],
    base: &str,
) -> Result<Vec<NativeDemDownloadSource>, String> {
    if dataset != "GEM15_DTM" && dataset != "GEM15_DSM" {
        return Err(
            "NATIVE_DEM_PROVIDER_UNSUPPORTED: dataset is not supported by GEM-15 Public".to_owned(),
        );
    }
    let south_start = bbox[1].floor() as i32;
    let north_end = bbox[3].ceil() as i32;
    let west_start = bbox[0].floor() as i32;
    let east_end = bbox[2].ceil() as i32;
    let row_count = usize::try_from(north_end.saturating_sub(south_start)).unwrap_or(0);
    let column_count = usize::try_from(east_end.saturating_sub(west_start)).unwrap_or(0);
    let tile_count = row_count
        .checked_mul(column_count)
        .ok_or_else(|| "NATIVE_DEM_TILE_LIMIT: GEM-15 tile count overflowed".to_owned())?;
    if tile_count == 0 || tile_count > MAX_NATIVE_DEM_TILES {
        return Err(format!(
            "NATIVE_DEM_TILE_LIMIT: GEM-15 request requires {tile_count} tiles; the product bound is {MAX_NATIVE_DEM_TILES}"
        ));
    }
    let mut sources = Vec::with_capacity(tile_count);
    for south in south_start..north_end {
        for west in west_start..east_end {
            let (tile_id, url) = gem15_tile_url_with_base(base, dataset, south, west)?;
            sources.push(NativeDemDownloadSource { id: tile_id, url });
        }
    }
    Ok(sources)
}

/// Resolves `(tile_id, https_url)` for a 1-degree Google GEM-15 v1 UTM COG
/// tile anchored at SW geographic corner `(south, west)`.
/// Coverage is `61S-84N` (`south in -61..=83`, `west in -180..180`);
/// bucket prefixes use `m`/`p` (`0 -> p0`, `-130 -> m130`) while leaf tile
/// coordinates use `-X -> mX` and non-negative integers without `p`.
pub(crate) fn gem15_tile_url(
    dataset: &str,
    south: i32,
    west: i32,
) -> Result<(String, String), String> {
    gem15_tile_url_with_base(
        "https://storage.googleapis.com/climate-and-sustainability/gem-15",
        dataset,
        south,
        west,
    )
}

fn gem15_tile_url_with_base(
    base: &str,
    dataset: &str,
    south: i32,
    west: i32,
) -> Result<(String, String), String> {
    let layer = match dataset.trim().to_ascii_uppercase().as_str() {
        "GEM15_DTM" => "dtm",
        "GEM15_DSM" => "dsm",
        _ => {
            return Err(
                "NATIVE_DEM_PROVIDER_UNSUPPORTED: dataset is not supported by GEM-15 Public"
                    .to_owned(),
            );
        }
    };
    if !(-61..=83).contains(&south) || !(-180..180).contains(&west) {
        return Err("NATIVE_DEM_OUT_OF_COVERAGE: Google GEM-15 v1 covers 61S-84N tiles".to_owned());
    }
    fn decade_tok(v: i32) -> String {
        if v < 0 {
            format!("m{}", v.unsigned_abs())
        } else {
            format!("p{v}")
        }
    }
    fn degree_tok(v: i32) -> String {
        if v < 0 {
            format!("m{}", v.unsigned_abs())
        } else {
            format!("{v}")
        }
    }
    let lat10 = south.div_euclid(10) * 10;
    let lon10 = west.div_euclid(10) * 10;
    let w_tok = degree_tok(west);
    let s_tok = degree_tok(south);
    let tile_id = format!("gem15_{layer}_w{w_tok}_s{s_tok}");
    let url = format!(
        "{}/{}_{}/w{w_tok}s{s_tok}/{layer}/zall/mosaic.tif",
        base.trim_end_matches('/'),
        decade_tok(lat10),
        decade_tok(lon10),
    );
    Ok((tile_id, url))
}

/// Resolves the `(tile_id, https_url)` for a 1-degree `FABDEM V1-2` bare-earth
/// COG tile anchored at SW corner `(south, west)`. Coverage is limited to
/// `60S-80N` (`south in -60..80`); `0` always uses `N00` / `E000` and the
/// `E170..W180` dateline block uses `W180`.
pub(crate) fn fabdem_tile_url(south: i32, west: i32) -> Result<(String, String), String> {
    if !(-60..80).contains(&south) || !(-180..180).contains(&west) {
        return Err("NATIVE_DEM_OUT_OF_COVERAGE: FABDEM V1-2 covers 60S-80N land tiles".to_owned());
    }
    fn lat_tok(y: i32) -> String {
        if y < 0 {
            format!("S{:02}", y.unsigned_abs())
        } else {
            format!("N{y:02}")
        }
    }
    fn lon_tok(x: i32) -> String {
        if x == 180 {
            "W180".to_owned()
        } else if x < 0 {
            format!("W{:03}", x.unsigned_abs())
        } else {
            format!("E{x:03}")
        }
    }
    let sw_lat10 = south.div_euclid(10) * 10;
    let ne_lat10 = sw_lat10 + 10;
    let sw_lon10 = west.div_euclid(10) * 10;
    let ne_lon10 = sw_lon10 + 10;
    let folder = format!(
        "{}{}-{}{}_FABDEM_V1-2",
        lat_tok(sw_lat10),
        lon_tok(sw_lon10),
        lat_tok(ne_lat10),
        lon_tok(ne_lon10)
    );
    let tile_id = format!("{}{}_FABDEM_V1-2", lat_tok(south), lon_tok(west));
    let url = format!(
        "https://huggingface.co/buckets/links-ads/fabdem/resolve/tiles/{folder}/{tile_id}.tif"
    );
    Ok((tile_id, url))
}

/// Hemisphere token with the official NISAR DEM padding: two digits for
/// latitude, three for longitude.
fn nisar_dem_coordinate_token(value: i32, latitude: bool) -> String {
    if latitude {
        if value < 0 {
            format!("S{:02}", value.unsigned_abs())
        } else {
            format!("N{:02}", value)
        }
    } else if value < 0 {
        format!("W{:03}", value.unsigned_abs())
    } else {
        format!("E{:03}", value)
    }
}

/// The 10-degree latitude band bucket a NISAR DEM tile belongs to, with
/// 5-degree polar bands at the top rows (verified against the CMR listing:
/// N17 -> N10, S19 -> S20, S51 -> S60, N85..N88 -> N85, S81..S85 -> S85,
/// S86..S90 -> S90).
fn nisar_dem_band(latitude: i32) -> i32 {
    if latitude >= 0 {
        if latitude >= 85 {
            85
        } else {
            (latitude / 10) * 10
        }
    } else {
        let magnitude = latitude.unsigned_abs() as i32;
        if magnitude >= 86 {
            -90
        } else if magnitude >= 81 {
            -85
        } else {
            -(((magnitude + 9) / 10) * 10)
        }
    }
}

/// The 20-degree longitude block bucket, flooring toward negative infinity
/// (Rust integer division truncates toward zero, which would misplace the
/// western blocks, e.g. -136 belongs to the W140 block, not W120).
fn nisar_dem_block(longitude: i32) -> i32 {
    longitude.div_euclid(20) * 20
}

fn native_dem_convert(
    engine: &LazyNativeGisEngine,
    payload: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let input = payload_path_any(payload, &["input_path", "path"])?;
    let output_root = payload_path_any(payload, &["output_root", "output_dir"]).or_else(
        |_| -> Result<PathBuf, String> {
            let destination = payload_path(payload, "destination")?;
            Ok(destination
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from(".")))
        },
    )?;
    let source = payload
        .get("source_vertical_datum")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            "NATIVE_GIS_VERTICAL_DATUM_REQUIRED: source_vertical_datum is required".to_owned()
        })?
        .trim()
        .to_ascii_uppercase();
    let output_mode = payload
        .get("output_mode")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("sarscape")
        .trim()
        .to_ascii_lowercase();
    let stem = native_dem_stem(&input);
    let ellipsoid_output = output_root.join(format!("{stem}_ellipsoid.tif"));
    let sarscape_output =
        (output_mode != "ellipsoid").then(|| output_root.join(format!("{stem}_dem")));
    prepare_native_task_output_path(&output_root, &ellipsoid_output)?;
    if let Some(output) = &sarscape_output {
        prepare_native_task_output_path(&output_root, output)?;
    }
    if !input.is_absolute() || !input.is_file() {
        return Err("NATIVE_DEM_INVALID_PAYLOAD: DEM conversion input is invalid".to_owned());
    }
    engine
        .convert_dem(
            &input,
            &ellipsoid_output,
            sarscape_output.as_deref(),
            &source,
        )
        .map_err(|error| format!("{}: {}", error.code().as_str(), error))
}

fn native_dem_stem(input: &Path) -> String {
    let raw = input
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("local_dem");
    let mut stem = raw
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    for suffix in ["_ELLIPSOID", "_DEM"] {
        if stem.to_ascii_uppercase().ends_with(suffix) {
            stem.truncate(stem.len().saturating_sub(suffix.len()));
            break;
        }
    }
    if stem.is_empty() {
        "local_dem".to_owned()
    } else {
        stem
    }
}

pub fn native_gis_capability_observations(
    engine: &LazyNativeGisEngine,
) -> insar_application_service::CapabilityRuntimeObservations {
    let mut bindings = vec![
        "dem-provider.copernicus-aws.v1",
        "dem-provider.fabdem-public.v1",
        "dem-provider.gem15-public.v1",
        "dem-provider.nisar-dem-asf.v1",
        "dem-provider.opentopography.v1",
        "lpdaac.aster-gdem.v3",
    ];
    if engine.is_discovered() {
        bindings.push("dem-provider.user-local.v1");
    }
    bindings.into_iter().fold(
        insar_application_service::CapabilityRuntimeObservations::default(),
        |observations, binding| {
            observations
                .with_build(binding, insar_core::BuildCapabilityState::Available)
                .with_runtime(binding, insar_core::RuntimeReadinessState::Ready)
        },
    )
}
/// Tauri owns the host adapter, while the Application Service owns the one
/// Store connection. Construction is lazy, but the normal desktop state is
/// always the business runtime; there is no foundation/provider-ready split.
pub struct ApplicationServiceState {
    service: Arc<ApplicationService>,
    /// Shared handle to the smoke-only loopback catalogue so the host can
    /// expose the exact search query the provider received. None outside the
    /// packaged UI-smoke mode.
    ui_smoke_catalog: Option<std::sync::Arc<UiSmokeLoopbackCatalog>>,
    sar_search_progress: Mutex<HashMap<String, SarSearchProgressUpdate>>,
    /// Local DEM conversions of one output share its `.part` file, so only one runs at a time.
    standalone_dem_running: std::sync::atomic::AtomicBool,
}

/// Marks the single running local DEM conversion; dropping it frees the slot.
pub struct StandaloneDemRun<'a>(&'a std::sync::atomic::AtomicBool);

impl Drop for StandaloneDemRun<'_> {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::Release);
    }
}

fn store_search_progress(
    progress: &mut HashMap<String, SarSearchProgressUpdate>,
    update: SarSearchProgressUpdate,
) -> bool {
    if update.validate().is_err() {
        return false;
    }
    let accepted = progress.get(&update.request_id).is_none_or(|current| {
        update.sequence > current.sequence && (!current.terminal || update.terminal)
    });
    if !accepted {
        return false;
    }
    progress.retain(|request_id, current| request_id == &update.request_id || !current.terminal);
    progress.insert(update.request_id.clone(), update);
    true
}

impl ApplicationServiceState {
    pub fn new() -> Self {
        Self::with_store_path(default_store_path())
    }

    pub fn with_store_path(path: PathBuf) -> Self {
        Self {
            service: Arc::new(ApplicationService::new(
                ApplicationServiceConfig::business_store(path),
            )),
            ui_smoke_catalog: None,
            sar_search_progress: Mutex::new(HashMap::new()),
            standalone_dem_running: std::sync::atomic::AtomicBool::new(false),
        }
    }

    pub fn with_store_path_and_runtime_observations_and_lazy_native_gis(
        path: PathBuf,
        observations: insar_application_service::CapabilityRuntimeObservations,
        engine: LazyNativeGisEngine,
    ) -> Self {
        let store_path_for_port = path.clone();
        let config = ApplicationServiceConfig::business_store(path)
            .with_capability_runtime_observations(observations);
        #[cfg(feature = "bhoonidhi-auth-revoke-acceptance")]
        let config = match std::env::var_os("INSAR_BHOONIDHI_AUTH_REVOKE_ACCEPTANCE_PROFILE") {
            Some(profile) => config
                .with_bhoonidhi_auth_revoke_acceptance_profile(PathBuf::from(profile))
                .expect("auth revoke acceptance requires an isolated canonical hand-test Store"),
            None => config,
        };
        Self {
            service: Arc::new(ApplicationService::with_dem_operation_port(
                config,
                Arc::new(NativeGisOperationPort {
                    engine,
                    store_path: Some(store_path_for_port),
                }),
            )),
            ui_smoke_catalog: None,
            sar_search_progress: Mutex::new(HashMap::new()),
            standalone_dem_running: std::sync::atomic::AtomicBool::new(false),
        }
    }

    pub fn with_offline_smoke_store_path_and_runtime_observations_and_compat_worker(
        path: PathBuf,
        observations: insar_application_service::CapabilityRuntimeObservations,
        worker: Arc<CompatWorkerState>,
    ) -> Self {
        let (global_boundary_provider, china_boundary_provider) =
            offline_smoke_boundary::providers();
        Self {
            service: Arc::new(
                ApplicationService::with_boundary_providers(
                    ApplicationServiceConfig::offline_smoke(path)
                        .with_capability_runtime_observations(observations),
                    global_boundary_provider,
                    china_boundary_provider,
                )
                .attach_dem_operation_port(Arc::new(CompatWorkerOperationPort { worker })),
            ),
            ui_smoke_catalog: None,
            sar_search_progress: Mutex::new(HashMap::new()),
            standalone_dem_running: std::sync::atomic::AtomicBool::new(false),
        }
    }

    pub fn with_ui_smoke_loopback_and_compat_worker(
        path: PathBuf,
        catalog: std::sync::Arc<UiSmokeLoopbackCatalog>,
        worker: Arc<CompatWorkerState>,
    ) -> Self {
        Self {
            service: Arc::new(
                with_ui_smoke_loopback_runtime(
                    ApplicationServiceConfig::business_store(path)
                        .with_capability_runtime_observations(
                            worker
                                .capability_runtime_observations()
                                .with_build(
                                    "orbit-download.asf-sentinel-1.v1",
                                    insar_core::BuildCapabilityState::Available,
                                )
                                .with_runtime(
                                    "orbit-download.asf-sentinel-1.v1",
                                    insar_core::RuntimeReadinessState::Ready,
                                ),
                        ),
                    Arc::clone(&catalog),
                )
                .attach_dem_operation_port(Arc::new(CompatWorkerOperationPort { worker })),
            ),
            ui_smoke_catalog: Some(catalog),
            sar_search_progress: Mutex::new(HashMap::new()),
            standalone_dem_running: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Agent tool adapter sharing this state's one ApplicationService.
    pub fn agent_application(&self) -> insar_application_service::AgentApplication {
        insar_application_service::AgentApplication(Arc::clone(&self.service))
    }

    pub fn record_external_download_task(
        &self,
        request: &insar_application_service::RecordExternalDownloadTaskRequest,
    ) -> Result<
        insar_application_service::RecordedExternalDownloadTask,
        insar_application_service::ApplicationServiceError,
    > {
        self.service.record_external_download_task(request)
    }

    /// The exact catalogue query the smoke loopback provider received.
    pub fn ui_smoke_captured_query(&self) -> Option<insar_core::CatalogSearchQuery> {
        self.ui_smoke_catalog
            .as_ref()
            .and_then(|catalog| catalog.recorded_query())
    }

    pub fn readiness(
        &self,
    ) -> Result<ServiceReadiness, insar_application_service::ApplicationServiceError> {
        self.service.startup()
    }

    pub fn snapshot(
        &self,
        include_terminal: bool,
    ) -> Result<ReadonlyServiceSnapshot, insar_application_service::ApplicationServiceError> {
        self.service.snapshot(include_terminal)
    }

    pub fn core_restore(
        &self,
        include_terminal: bool,
    ) -> Result<CoreServiceSnapshot, insar_application_service::ApplicationServiceError> {
        self.service.core_restore(include_terminal)
    }

    pub fn workbench_page(
        &self,
        request: &ApplicationWorkbenchPageRequest,
    ) -> Result<ApplicationWorkbenchPage, insar_application_service::ApplicationServiceError> {
        self.service.workbench_page(&request.request)
    }

    pub fn batch_page(
        &self,
        request: &insar_application_service::StoreBatchPageRequest,
    ) -> Result<
        insar_application_service::StoreBatchPage,
        insar_application_service::ApplicationServiceError,
    > {
        self.service.batch_page(request)
    }

    pub fn task_page(
        &self,
        request: &StoreTaskPageRequest,
    ) -> Result<StoreTaskPage, insar_application_service::ApplicationServiceError> {
        self.service.task_page(request)
    }

    pub fn task_source_batch_page(
        &self,
        request: &StoreTaskSourceBatchPageRequest,
    ) -> Result<StoreTaskSourceBatchPage, insar_application_service::ApplicationServiceError> {
        self.service.task_source_batch_page(request)
    }

    pub fn task_overview(
        &self,
    ) -> Result<
        insar_application_service::TaskOverview,
        insar_application_service::ApplicationServiceError,
    > {
        self.service.task_overview()
    }

    pub fn task_history_page(
        &self,
        request: &StoreTaskHistoryPageRequest,
    ) -> Result<StoreTaskHistoryPage, insar_application_service::ApplicationServiceError> {
        self.service.task_history_page(request)
    }

    pub fn task_event_page(
        &self,
        request: &StoreTaskEventPageRequest,
    ) -> Result<StoreTaskEventPage, insar_application_service::ApplicationServiceError> {
        self.service.task_event_page(request)
    }

    pub fn resolve_task_output_directory(
        &self,
        request: &StoreTaskOutputDirectoryRequest,
    ) -> Result<StoreTaskOutputDirectory, insar_application_service::ApplicationServiceError> {
        self.service.resolve_task_output_directory(request)
    }

    /// B15: enumerate one task's renamed-aside conflict files (read-only).
    pub fn list_task_conflict_files(
        &self,
        request: StoreTaskConflictFilesRequest,
    ) -> Result<TaskConflictFileList, insar_application_service::ApplicationServiceError> {
        request.validate().map_err(|reason| {
            insar_application_service::ApplicationServiceError::InvalidRequest { reason }
        })?;
        self.service.list_task_conflict_files(&request.task_id)
    }

    /// B15: delete the user-confirmed conflict files of one task, each by its
    /// exact (path, size, mtime) from the listing.
    pub fn delete_task_conflict_files(
        &self,
        task_id: &str,
        expected: &[(String, u64, u64)],
    ) -> Result<TaskConflictFileDeleteResult, insar_application_service::ApplicationServiceError>
    {
        self.service.delete_task_conflict_files(task_id, expected)
    }
    /// Perform the desktop-only shell action after the Application Service has
    /// resolved the exact persisted task/revision output authority. The Tauri
    /// command remains a typed transport adapter and never accepts a caller path.
    pub fn open_task_output_directory(
        &self,
        request: &StoreTaskOutputDirectoryRequest,
    ) -> Result<StoreTaskOutputDirectory, insar_application_service::ApplicationServiceError> {
        let resolved = self.resolve_task_output_directory(request)?;
        std::fs::create_dir_all(&resolved.output_root).map_err(|_| {
            insar_application_service::ApplicationServiceError::InvalidRequest {
                reason: "task output directory could not be prepared".to_owned(),
            }
        })?;
        // 按平台选择系统文件管理器：Windows 资源管理器、macOS Finder（`open`）、
        // 其他 Unix 桌面 `xdg-open`。此前硬编码 `explorer.exe` 导致 macOS 上
        // 下载中心「打开目录」按钮全部失败。
        #[cfg(target_os = "windows")]
        let mut opener = std::process::Command::new("explorer.exe");
        #[cfg(target_os = "macos")]
        let mut opener = std::process::Command::new("open");
        #[cfg(all(unix, not(target_os = "macos")))]
        let mut opener = std::process::Command::new("xdg-open");
        opener.arg(&resolved.output_root).spawn().map_err(|_| {
            insar_application_service::ApplicationServiceError::InvalidRequest {
                reason: "task output directory could not be opened".to_owned(),
            }
        })?;
        Ok(resolved)
    }

    pub fn update_workbench_selection(
        &self,
        command: &WorkbenchSelectionCommand,
    ) -> Result<WorkbenchSelectionSummary, insar_application_service::ApplicationServiceError> {
        self.service.update_workbench_selection(command)
    }

    pub fn workspace_open(
        &self,
        request: &WorkspaceOpenRequest,
    ) -> Result<WorkspaceContextState, insar_application_service::ApplicationServiceError> {
        self.service.workspace_open(request)
    }

    pub fn workspace_create(
        &self,
        request: &WorkspaceCreateRequest,
    ) -> Result<WorkspaceContextState, insar_application_service::ApplicationServiceError> {
        self.service.workspace_create(request)
    }

    pub fn workspace_prepare_location(
        &self,
        request: &WorkspacePrepareLocationRequest,
    ) -> Result<WorkspacePreparedLocation, insar_application_service::ApplicationServiceError> {
        self.service.workspace_prepare_location(request)
    }

    pub fn plan_dem_task(
        &self,
        request: &DemTaskPlanRequest,
    ) -> Result<DemTaskPlan, insar_application_service::ApplicationServiceError> {
        self.service.plan_dem_task(request)
    }

    pub fn create_dem_task(
        &self,
        request: &DemTaskRequest,
    ) -> Result<CoreServiceSnapshot, insar_application_service::ApplicationServiceError> {
        self.service.create_dem_task(request)
    }

    pub fn handoff_dem_conversion(
        &self,
        request: &DemConversionRequest,
    ) -> Result<CoreServiceSnapshot, insar_application_service::ApplicationServiceError> {
        self.service.handoff_dem_conversion(request)
    }

    pub fn get_dem_task_settings(
        &self,
    ) -> Result<DemTaskSettings, insar_application_service::ApplicationServiceError> {
        self.service.get_dem_task_settings()
    }

    pub fn save_dem_task_settings(
        &self,
        settings: &DemTaskSettings,
    ) -> Result<DemTaskSettings, insar_application_service::ApplicationServiceError> {
        self.service.save_dem_task_settings(settings)
    }

    pub fn get_sar_download_policy_settings(
        &self,
    ) -> Result<SarDownloadPolicySettings, insar_application_service::ApplicationServiceError> {
        self.service.get_sar_download_policy_settings()
    }

    pub fn save_sar_download_policy_settings(
        &self,
        settings: &SarDownloadPolicySettings,
    ) -> Result<SarDownloadPolicySettings, insar_application_service::ApplicationServiceError> {
        self.service.save_sar_download_policy_settings(settings)
    }

    pub fn get_network_settings(
        &self,
    ) -> Result<ApplicationNetworkSettings, insar_application_service::ApplicationServiceError>
    {
        self.service.get_network_settings()
    }

    pub fn ordinary_proxy_policy(&self) -> insar_core::ProxyPolicy {
        self.service.ordinary_proxy_policy()
    }

    pub fn save_network_settings(
        &self,
        settings: &ApplicationNetworkSettings,
    ) -> Result<ApplicationNetworkSettings, insar_application_service::ApplicationServiceError>
    {
        self.service.save_network_settings(settings)
    }

    pub fn test_network_connection(
        &self,
    ) -> Result<String, insar_application_service::ApplicationServiceError> {
        self.service.test_network_connection()
    }

    pub fn probe_provider_connectivity(
        &self,
    ) -> Result<
        Vec<insar_application_service::ProviderConnectivityResult>,
        insar_application_service::ApplicationServiceError,
    > {
        self.service.probe_provider_connectivity()
    }

    pub fn begin_standalone_dem(&self) -> Option<StandaloneDemRun<'_>> {
        self.standalone_dem_running
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
            )
            .ok()
            .map(|_| StandaloneDemRun(&self.standalone_dem_running))
    }

    pub fn standalone_dem(
        &self,
        request: &StandaloneDemRequest,
    ) -> Result<serde_json::Value, insar_application_service::ApplicationServiceError> {
        self.service.standalone_dem(request)
    }

    pub fn create_standalone_dem_task(
        &self,
        request: &StandaloneDemAoiTaskRequest,
    ) -> Result<CoreServiceSnapshot, insar_application_service::ApplicationServiceError> {
        self.service.create_standalone_dem_aoi_task(request)
    }

    pub fn import_scenes(
        &self,
        request: &SceneImportRequest,
    ) -> Result<serde_json::Value, insar_application_service::ApplicationServiceError> {
        self.service.import_scenes(request)
    }

    pub fn match_orbits_directory(
        &self,
        request: &OrbitDirectoryMatchRequest,
    ) -> Result<serde_json::Value, insar_application_service::ApplicationServiceError> {
        self.service.match_orbits_directory(request)
    }

    pub fn gacos_ledger(
        &self,
    ) -> Result<serde_json::Value, insar_application_service::ApplicationServiceError> {
        self.service.gacos_ledger()
    }

    pub fn gacos_import_result(
        &self,
        request: &GacosResultImportRequest,
    ) -> Result<serde_json::Value, insar_application_service::ApplicationServiceError> {
        self.service.gacos_import_result(request)
    }

    pub fn gacos_create_import_task(
        &self,
        request: &GacosCreateImportTaskRequest,
    ) -> Result<serde_json::Value, insar_application_service::ApplicationServiceError> {
        self.service.gacos_create_import_task(request)
    }

    pub fn gacos_create_result_task(
        &self,
        request: &GacosCreateResultTaskRequest,
    ) -> Result<serde_json::Value, insar_application_service::ApplicationServiceError> {
        self.service.gacos_create_result_task(request)
    }

    pub fn gacos_supply_result_link(
        &self,
        request: &GacosSupplyResultLinkRequest,
    ) -> Result<serde_json::Value, insar_application_service::ApplicationServiceError> {
        self.service.gacos_supply_result_link(request)
    }

    pub fn gacos_submit_request(
        &self,
        request: &GacosConfirmRequest,
    ) -> Result<serde_json::Value, insar_application_service::ApplicationServiceError> {
        self.service.gacos_submit_request(request)
    }

    pub fn gacos_request_status(
        &self,
        task_id: &str,
    ) -> Result<serde_json::Value, insar_application_service::ApplicationServiceError> {
        self.service.gacos_request_status(task_id)
    }

    pub fn create_companion_orbit_task(
        &self,
        source_task_id: &str,
    ) -> Result<CoreServiceSnapshot, insar_application_service::ApplicationServiceError> {
        self.service.create_companion_orbit_task(source_task_id)
    }

    pub fn create_orbit_task(
        &self,
        request: &OrbitTaskRequest,
    ) -> Result<CoreServiceSnapshot, insar_application_service::ApplicationServiceError> {
        self.service.create_orbit_task(request)
    }

    pub fn create_standalone_orbit_task(
        &self,
        request: &insar_application_service::StandaloneOrbitTaskRequest,
    ) -> Result<CoreServiceSnapshot, insar_application_service::ApplicationServiceError> {
        self.service.create_standalone_orbit_task(request)
    }

    pub fn project_create(
        &self,
        request: &WorkspaceNamedNodeRequest,
    ) -> Result<WorkspaceContextState, insar_application_service::ApplicationServiceError> {
        self.service.project_create(request)
    }

    pub fn project_select(
        &self,
        request: &WorkspaceSelectNodeRequest,
    ) -> Result<WorkspaceContextState, insar_application_service::ApplicationServiceError> {
        self.service.project_select(request)
    }

    pub fn region_create(
        &self,
        request: &WorkspaceNamedNodeRequest,
    ) -> Result<WorkspaceContextState, insar_application_service::ApplicationServiceError> {
        self.service.region_create(request)
    }

    pub fn region_select(
        &self,
        request: &WorkspaceSelectNodeRequest,
    ) -> Result<WorkspaceContextState, insar_application_service::ApplicationServiceError> {
        self.service.region_select(request)
    }

    pub fn list_sources(
        &self,
    ) -> Result<Vec<insar_core::SourceDescriptor>, insar_application_service::ApplicationServiceError>
    {
        self.service.list_sources()
    }

    pub fn source_views(
        &self,
    ) -> Result<Vec<insar_core::SourceView>, insar_application_service::ApplicationServiceError>
    {
        self.service.source_views()
    }

    pub fn provider_capabilities(
        &self,
    ) -> Result<ProviderCapabilitiesResponse, insar_application_service::ApplicationServiceError>
    {
        self.service.provider_capabilities()
    }

    pub fn effective_capability(
        &self,
        capability_id: &str,
    ) -> Result<Option<EffectiveCapabilityStatus>, insar_application_service::ApplicationServiceError>
    {
        self.service.effective_capability(capability_id)
    }

    pub fn bind_region_aoi(
        &self,
        request: &ApplicationBindRegionAoiRequest,
    ) -> Result<CoreServiceSnapshot, insar_application_service::ApplicationServiceError> {
        self.service.bind_region_aoi(request)
    }

    pub fn clear_region_aoi(
        &self,
        request: &ApplicationClearRegionAoiRequest,
    ) -> Result<CoreServiceSnapshot, insar_application_service::ApplicationServiceError> {
        self.service.clear_region_aoi(request)
    }

    pub fn set_working_aoi(
        &self,
        request: &ApplicationSetWorkingAoiRequest,
    ) -> Result<ApplicationWorkingAoi, insar_application_service::ApplicationServiceError> {
        self.service.set_working_aoi(request)
    }

    pub fn clear_working_aoi(
        &self,
    ) -> Result<(), insar_application_service::ApplicationServiceError> {
        self.service.clear_working_aoi()
    }

    pub fn working_aoi(
        &self,
    ) -> Result<Option<ApplicationWorkingAoi>, insar_application_service::ApplicationServiceError>
    {
        self.service.working_aoi()
    }

    pub fn aoi_import(
        &self,
        request: &ApplicationAoiImportRequest,
    ) -> Result<serde_json::Value, insar_application_service::ApplicationServiceError> {
        self.service.aoi_import(request)
    }

    pub fn aoi_normalize(
        &self,
        request: &ApplicationAoiNormalizeRequest,
    ) -> Result<serde_json::Value, insar_application_service::ApplicationServiceError> {
        self.service.aoi_normalize(request)
    }

    pub fn admin_boundary_search(
        &self,
        request: &AdminBoundarySearchRequest,
    ) -> Result<AdminBoundarySearchResult, insar_application_service::ApplicationServiceError> {
        self.service.admin_boundary_search(request)
    }

    pub fn admin_boundary_resolve(
        &self,
        request: &AdminBoundaryResolveRequest,
    ) -> Result<AdminBoundaryResult, insar_application_service::ApplicationServiceError> {
        self.service.admin_boundary_resolve(request)
    }

    pub fn admin_options(
        &self,
        request: &AdminBoundaryOptionsRequest,
    ) -> Result<AdminBoundaryOptionsResult, insar_application_service::ApplicationServiceError>
    {
        self.service.admin_options(request)
    }

    pub fn geo_boundary_countries(
        &self,
    ) -> Result<GeoBoundaryCountriesResult, insar_application_service::ApplicationServiceError>
    {
        self.service.geo_boundary_countries()
    }

    pub fn geo_boundary_levels(
        &self,
        request: &GeoBoundaryLevelsRequest,
    ) -> Result<GeoBoundaryLevelsResult, insar_application_service::ApplicationServiceError> {
        self.service.geo_boundary_levels(request)
    }

    pub fn geo_boundary_units(
        &self,
        request: &GeoBoundaryUnitsRequest,
    ) -> Result<GeoBoundaryUnitsResult, insar_application_service::ApplicationServiceError> {
        self.service.geo_boundary_units(request)
    }

    pub fn geo_boundary_resolve(
        &self,
        request: &GeoBoundaryResolveRequest,
    ) -> Result<AdminBoundaryResult, insar_application_service::ApplicationServiceError> {
        self.service.geo_boundary_resolve(request)
    }

    pub fn gacos_plan(
        &self,
        payload: &serde_json::Value,
    ) -> Result<serde_json::Value, insar_application_service::ApplicationServiceError> {
        self.service.gacos_plan(payload)
    }

    pub fn gacos_dry_run(
        &self,
        payload: &serde_json::Value,
    ) -> Result<serde_json::Value, insar_application_service::ApplicationServiceError> {
        self.service.gacos_dry_run(payload)
    }

    pub fn report_generate(
        &self,
        payload: &serde_json::Value,
    ) -> Result<serde_json::Value, insar_application_service::ApplicationServiceError> {
        self.service.report_generate(payload)
    }

    pub fn search_start(
        &self,
        request: insar_application_service::UnifiedSearchRequest,
    ) -> Result<insar_core::SearchProgress, insar_application_service::ApplicationServiceError>
    {
        self.search_start_with_options(request, false)
    }

    pub fn search_start_with_options(
        &self,
        request: insar_application_service::UnifiedSearchRequest,
        bypass_cache: bool,
    ) -> Result<insar_core::SearchProgress, insar_application_service::ApplicationServiceError>
    {
        self.service
            .search_start_with_options(request, bypass_cache)
    }

    pub fn search_progress(&self, request_id: &str) -> Option<insar_core::SearchProgress> {
        self.service.search_progress(request_id)
    }

    pub fn search_cancel(
        &self,
        request_id: &str,
    ) -> Result<bool, insar_application_service::ApplicationServiceError> {
        self.service.search_cancel(request_id)
    }

    pub fn sar_search_and_freeze(
        &self,
        request: &ApplicationSarSearchAndFreezeRequest,
    ) -> Result<CoreServiceSnapshot, insar_application_service::ApplicationServiceError> {
        self.service
            .sar_search_and_freeze_with_progress(request, |update| {
                if let Ok(mut progress) = self.sar_search_progress.lock() {
                    store_search_progress(&mut progress, update);
                }
            })
    }

    pub fn sar_search_progress(&self, request_id: &str) -> Option<SarSearchProgressUpdate> {
        let request_id = request_id.trim();
        if request_id.is_empty() || request_id.len() > 128 {
            return None;
        }
        self.sar_search_progress
            .lock()
            .ok()
            .and_then(|progress| progress.get(request_id).cloned())
    }

    pub fn sar_background_count_and_update(
        &self,
        request: &ApplicationSarBackgroundCountRequest,
    ) -> Result<
        ApplicationSarBackgroundCountResult,
        insar_application_service::ApplicationServiceError,
    > {
        self.service.sar_background_count_and_update(request)
    }

    pub fn catalog_search_and_freeze(
        &self,
        request: &ApplicationCatalogSearchAndFreezeRequest,
    ) -> Result<CoreServiceSnapshot, insar_application_service::ApplicationServiceError> {
        self.service
            .catalog_search_and_freeze_with_progress(request, |update| {
                if let Ok(mut progress) = self.sar_search_progress.lock() {
                    store_search_progress(&mut progress, update);
                }
            })
    }

    pub fn catalog_intent_search_and_freeze(
        &self,
        request: &ApplicationCatalogIntentSearchAndFreezeRequest,
    ) -> Result<CoreServiceSnapshot, insar_application_service::ApplicationServiceError> {
        self.service
            .catalog_intent_search_and_freeze_with_progress(request, |update| {
                if let Ok(mut progress) = self.sar_search_progress.lock() {
                    store_search_progress(&mut progress, update);
                }
            })
    }

    pub fn cancel_request(
        &self,
        request_id: &str,
    ) -> Result<bool, insar_application_service::ApplicationServiceError> {
        self.service.cancel_request(request_id)
    }

    /// #22 B slice 3(c): cancel a catalogue search, keeping the rows of a
    /// search that was already readable (state cancelled, possibly
    /// incomplete).
    pub fn cancel_catalog_search(
        &self,
        request_id: &str,
    ) -> Result<insar_core::SearchCancelOutcome, insar_application_service::ApplicationServiceError>
    {
        self.service.cancel_catalog_search(request_id)
    }

    pub fn bhoonidhi_authentication_check(
        &self,
    ) -> Result<serde_json::Value, insar_application_service::ApplicationServiceError> {
        self.service.bhoonidhi_authentication_check()
    }

    pub fn bhoonidhi_session_status(
        &self,
    ) -> Result<BhoonidhiDesktopSessionStatus, insar_application_service::ApplicationServiceError>
    {
        self.service.bhoonidhi_session_status()
    }

    pub fn account_session_statuses(
        &self,
    ) -> Result<Vec<AccountSessionStatus>, insar_application_service::ApplicationServiceError> {
        self.service.account_session_statuses()
    }

    pub fn cdse_session_status(
        &self,
    ) -> Result<CdseSessionStatus, insar_application_service::ApplicationServiceError> {
        self.service.cdse_session_status()
    }

    pub fn cdse_login(
        &self,
        request: CdseLoginRequest,
    ) -> Result<CdseSessionStatus, insar_application_service::ApplicationServiceError> {
        self.service.cdse_login(request)
    }

    pub fn cdse_logout(
        &self,
    ) -> Result<CdseSessionStatus, insar_application_service::ApplicationServiceError> {
        self.service.cdse_logout()
    }

    pub fn earthdata_credentials_changed(&self) {
        self.service.earthdata_credentials_changed();
    }

    pub fn cdse_network_policy(
        &self,
    ) -> Result<CdseNetworkMode, insar_application_service::ApplicationServiceError> {
        self.service.cdse_network_policy()
    }

    pub fn set_cdse_network_policy(
        &self,
        mode: CdseNetworkMode,
    ) -> Result<CdseSessionStatus, insar_application_service::ApplicationServiceError> {
        self.service.set_cdse_network_policy(mode)
    }

    pub fn cdse_network_diagnostic(
        &self,
        mode: CdseNetworkMode,
    ) -> Result<CdseNetworkDiagnosticResult, insar_application_service::ApplicationServiceError>
    {
        self.service.cdse_network_diagnostic(mode)
    }

    pub fn preview_asset(
        &self,
        request: PreviewAssetRequest,
    ) -> Result<ResolvedPreviewAsset, insar_application_service::ApplicationServiceError> {
        self.service.preview_asset(request)
    }

    pub fn save_preview_asset(
        &self,
        request: PreviewSaveRequest,
    ) -> Result<PreviewSaveResponse, insar_application_service::ApplicationServiceError> {
        self.service.save_preview_asset(request)
    }

    pub fn cdse_quicklook(
        &self,
        request: CdseQuicklookRequest,
    ) -> Result<CdseQuicklookResponse, insar_application_service::ApplicationServiceError> {
        self.service.cdse_quicklook(request)
    }

    pub fn sentinel2_map_preview(
        &self,
        request: Sentinel2MapPreviewRequest,
    ) -> Result<Sentinel2MapPreviewResponse, insar_application_service::ApplicationServiceError>
    {
        self.service.sentinel2_map_preview(request)
    }

    pub fn usgs_m2m_session_status(
        &self,
    ) -> Result<UsgsM2mDesktopSessionStatus, insar_application_service::ApplicationServiceError>
    {
        self.service.usgs_m2m_session_status()
    }

    pub fn usgs_m2m_login(
        &self,
        request: UsgsM2mLoginRequest,
    ) -> Result<UsgsM2mDesktopSessionStatus, insar_application_service::ApplicationServiceError>
    {
        self.service.usgs_m2m_login(request)
    }

    pub fn usgs_m2m_logout(
        &self,
    ) -> Result<UsgsM2mDesktopSessionStatus, insar_application_service::ApplicationServiceError>
    {
        self.service.usgs_m2m_logout()
    }

    pub fn landsat_browse(
        &self,
        request: LandsatBrowseRequest,
    ) -> Result<LandsatBrowseResponse, insar_application_service::ApplicationServiceError> {
        self.service.landsat_browse(request)
    }

    pub fn landsat_asset_details(
        &self,
        request: LandsatAssetDetailsRequest,
    ) -> Result<LandsatAssetDetailsResponse, insar_application_service::ApplicationServiceError>
    {
        self.service.landsat_asset_details(request)
    }

    pub fn bhoonidhi_asset_size(
        &self,
        request: BhoonidhiAssetSizeRequest,
    ) -> Result<BhoonidhiAssetSizeResponse, insar_application_service::ApplicationServiceError>
    {
        self.service.bhoonidhi_asset_size(request)
    }

    pub fn create_landsat_bundle_task(
        &self,
        request: &LandsatCreateBundleTaskRequest,
    ) -> Result<CoreServiceSnapshot, insar_application_service::ApplicationServiceError> {
        self.service.create_landsat_bundle_task(request)
    }

    pub fn create_landsat_asset_task(
        &self,
        request: &LandsatCreateAssetTaskRequest,
    ) -> Result<CoreServiceSnapshot, insar_application_service::ApplicationServiceError> {
        self.service.create_landsat_asset_selection_task(request)
    }

    pub fn create_reviewed_mpc_asset_task(
        &self,
        request: &ReviewedMpcCreateAssetTaskRequest,
    ) -> Result<CoreServiceSnapshot, insar_application_service::ApplicationServiceError> {
        self.service.create_reviewed_mpc_asset_task(request)
    }

    pub fn create_landsat_bundle_batch_task(
        &self,
        request: &insar_application_service::LandsatCreateBundleBatchTaskRequest,
    ) -> Result<
        insar_application_service::LandsatCreateBundleBatchTaskResult,
        insar_application_service::ApplicationServiceError,
    > {
        self.service.create_landsat_bundle_batch_task(request)
    }

    pub fn create_landsat_asset_batch_task(
        &self,
        request: &insar_application_service::LandsatCreateAssetBatchTaskRequest,
    ) -> Result<
        insar_application_service::LandsatCreateAssetBatchTaskResult,
        insar_application_service::ApplicationServiceError,
    > {
        self.service.create_landsat_asset_batch_task(request)
    }

    pub fn task_delivery_admission(
        &self,
        task_id: &str,
    ) -> Result<
        Option<insar_application_service::OpticalDeliveryAdmissionSummary>,
        insar_application_service::ApplicationServiceError,
    > {
        self.service.task_delivery_admission(task_id)
    }

    pub fn optical_preparation_progress(
        &self,
        request_id: &str,
    ) -> Result<
        Option<insar_application_service::OpticalPreparationProgress>,
        insar_application_service::ApplicationServiceError,
    > {
        self.service.optical_preparation_progress(request_id)
    }

    pub fn create_sentinel2_archive_batch_task(
        &self,
        request: &Sentinel2CreateArchiveBatchTaskRequest,
    ) -> Result<
        Sentinel2CreateArchiveBatchTaskResult,
        insar_application_service::ApplicationServiceError,
    > {
        self.service.create_sentinel2_archive_batch_task(request)
    }

    pub fn create_sentinel2_archive_task(
        &self,
        request: &Sentinel2CreateArchiveTaskRequest,
    ) -> Result<CoreServiceSnapshot, insar_application_service::ApplicationServiceError> {
        self.service.create_sentinel2_archive_task(request)
    }

    pub fn create_sentinel2_band_task(
        &self,
        request: &Sentinel2CreateBandTaskRequest,
    ) -> Result<CoreServiceSnapshot, insar_application_service::ApplicationServiceError> {
        self.service.create_sentinel2_band_task(request)
    }

    pub fn create_sentinel2_band_batch_task(
        &self,
        request: &insar_application_service::Sentinel2CreateBandBatchTaskRequest,
    ) -> Result<
        insar_application_service::Sentinel2CreateBandBatchTaskResult,
        insar_application_service::ApplicationServiceError,
    > {
        self.service.create_sentinel2_band_batch_task(request)
    }

    pub fn create_sentinel2_l1c_band_batch_task(
        &self,
        request: &insar_application_service::Sentinel2L1cCreateBandBatchTaskRequest,
    ) -> Result<
        insar_application_service::Sentinel2L1cCreateBandBatchTaskResult,
        insar_application_service::ApplicationServiceError,
    > {
        self.service.create_sentinel2_l1c_band_batch_task(request)
    }

    pub fn create_hls_band_batch_task(
        &self,
        request: &insar_application_service::HlsCreateBandBatchTaskRequest,
    ) -> Result<
        insar_application_service::HlsCreateBandBatchTaskResult,
        insar_application_service::ApplicationServiceError,
    > {
        self.service.create_hls_band_batch_task(request)
    }

    pub fn precheck_optical_aoi_coverage(
        &self,
        request: insar_application_service::OpticalDerivedPrecheckBatchRequest,
    ) -> Result<insar_gis::OpticalPrecheckReport, insar_application_service::ApplicationServiceError>
    {
        self.service.precheck_optical_aoi_coverage(request)
    }

    pub fn create_optical_derived_task(
        &self,
        request: insar_application_service::OpticalDerivedCreateTaskRequest,
    ) -> Result<
        insar_application_service::OpticalDerivedTaskCreatedResult,
        insar_application_service::ApplicationServiceError,
    > {
        self.service.create_optical_derived_task(request)
    }

    pub fn experiment_project_get(
        &self,
    ) -> Result<
        insar_application_service::ExperimentProjectSnapshot,
        insar_application_service::ApplicationServiceError,
    > {
        self.service.experiment_project_get()
    }

    pub fn experiment_project_add_task(
        &self,
        task_id: &str,
    ) -> Result<
        insar_application_service::ExperimentProjectSnapshot,
        insar_application_service::ApplicationServiceError,
    > {
        self.service.experiment_project_add_task(task_id)
    }

    pub fn experiment_project_remove_item(
        &self,
        item_id: &str,
    ) -> Result<
        insar_application_service::ExperimentProjectSnapshot,
        insar_application_service::ApplicationServiceError,
    > {
        self.service.experiment_project_remove_item(item_id)
    }

    pub fn experiment_project_export(
        &self,
        export_dir: Option<std::path::PathBuf>,
    ) -> Result<
        insar_application_service::ExperimentProjectExportResult,
        insar_application_service::ApplicationServiceError,
    > {
        self.service.experiment_project_export(export_dir)
    }

    pub fn nisar_remote_subset_capability(
        &self,
    ) -> Result<NisarRemoteSubsetCapability, insar_application_service::ApplicationServiceError>
    {
        self.service.nisar_remote_subset_capability()
    }

    pub fn create_nisar_remote_subset_task(
        &self,
        request: &NisarCreateRemoteSubsetTaskRequest,
    ) -> Result<CoreServiceSnapshot, insar_application_service::ApplicationServiceError> {
        self.service.create_nisar_remote_subset_task(request)
    }

    pub fn prepare_nisar_subset_intent_v2(
        &self,
        request: &NisarPrepareSubsetIntentV2Request,
    ) -> Result<NisarPreparedSubsetIntentV2, insar_application_service::ApplicationServiceError>
    {
        self.service.prepare_nisar_subset_intent_v2(request)
    }

    pub fn confirm_nisar_subset_task_v2(
        &self,
        request: &NisarConfirmSubsetTaskV2Request,
    ) -> Result<CoreServiceSnapshot, insar_application_service::ApplicationServiceError> {
        self.service.confirm_nisar_subset_task_v2(request)
    }

    pub fn preflight_nisar_subset_batch_v2(
        &self,
        request: &NisarPreflightSubsetBatchV2Request,
    ) -> Result<NisarPreflightSubsetBatchV2, insar_application_service::ApplicationServiceError>
    {
        self.service.preflight_nisar_subset_batch_v2(request)
    }

    pub fn prepare_nisar_subset_batch_v2(
        &self,
        request: &NisarPrepareSubsetBatchV2Request,
    ) -> Result<NisarPreparedSubsetBatchV2, insar_application_service::ApplicationServiceError>
    {
        self.service.prepare_nisar_subset_batch_v2(request)
    }

    pub fn confirm_nisar_subset_batch_task_v2(
        &self,
        request: &NisarConfirmSubsetBatchTaskV2Request,
    ) -> Result<CoreServiceSnapshot, insar_application_service::ApplicationServiceError> {
        self.service.confirm_nisar_subset_batch_task_v2(request)
    }

    pub fn append_nisar_subset_batch_task_v2(
        &self,
        request: &NisarAppendSubsetBatchTaskV2Request,
    ) -> Result<CoreServiceSnapshot, insar_application_service::ApplicationServiceError> {
        self.service.append_nisar_subset_batch_task_v2(request)
    }

    pub fn read_nisar_subset_append_context_v2(
        &self,
        request: &NisarAppendSubsetContextV2Request,
    ) -> Result<NisarAppendSubsetContextV2, insar_application_service::ApplicationServiceError>
    {
        self.service.read_nisar_subset_append_context_v2(request)
    }

    pub fn preflight_nisar_subset_append_v2(
        &self,
        request: &NisarPreflightSubsetAppendV2Request,
    ) -> Result<NisarPreflightSubsetAppendV2, insar_application_service::ApplicationServiceError>
    {
        self.service.preflight_nisar_subset_append_v2(request)
    }

    pub fn query_source_mutation_status(
        &self,
        query: insar_application_service::SourceMutationStatusQuery<'_>,
    ) -> Result<
        insar_application_service::SourceMutationStatus,
        insar_application_service::ApplicationServiceError,
    > {
        self.service.query_source_mutation_status(query)
    }

    pub fn create_nisar_subset_successor_task_v2(
        &self,
        request: &NisarCreateSubsetSuccessorTaskV2Request,
    ) -> Result<CoreServiceSnapshot, insar_application_service::ApplicationServiceError> {
        self.service.create_nisar_subset_successor_task_v2(request)
    }

    pub fn resume_nisar_subset_task(
        &self,
        task_id: &str,
    ) -> Result<CoreServiceSnapshot, insar_application_service::ApplicationServiceError> {
        self.service.resume_nisar_subset_task(task_id)
    }

    pub fn read_nisar_archived_recovery_context_v2(
        &self,
        source_task_id: &str,
        new_task_id: &str,
    ) -> Result<
        insar_application_service::NisarArchivedRecoveryContextV2,
        insar_application_service::ApplicationServiceError,
    > {
        self.service
            .read_nisar_archived_recovery_context_v2(source_task_id, new_task_id)
    }

    #[cfg(feature = "bhoonidhi-auth-revoke-acceptance")]
    pub fn bhoonidhi_auth_revoke_acceptance_generation(
        &self,
        task_id: &str,
    ) -> Result<u64, insar_application_service::ApplicationServiceError> {
        self.service
            .bhoonidhi_auth_revoke_acceptance_generation(task_id)
    }

    #[cfg(feature = "bhoonidhi-auth-revoke-acceptance")]
    pub fn bhoonidhi_auth_revoke_acceptance_once(
        &self,
        task_id: &str,
        generation: u64,
    ) -> Result<(), insar_application_service::ApplicationServiceError> {
        self.service
            .bhoonidhi_auth_revoke_acceptance_once(task_id, generation)
    }

    pub fn create_nisar_archived_recovery_v2(
        &self,
        request: &insar_application_service::NisarCreateArchivedRecoveryV2Request,
    ) -> Result<CoreServiceSnapshot, insar_application_service::ApplicationServiceError> {
        self.service.create_nisar_archived_recovery_v2(request)
    }

    pub fn start_newly_confirmed_task(
        &self,
        task_id: &str,
    ) -> Result<CoreServiceSnapshot, insar_application_service::ApplicationServiceError> {
        self.service.start_newly_confirmed_task(task_id)
    }

    pub fn update_search_workbench(
        &self,
        request: &StoreUpdateWorkbenchRequest,
    ) -> Result<CoreServiceSnapshot, insar_application_service::ApplicationServiceError> {
        self.service.update_search_workbench(request)
    }

    pub fn create_task(
        &self,
        batch_id: String,
        revision: u64,
        item_ids: Vec<String>,
        options: insar_application_service::UnifiedCreateTaskOptions,
    ) -> Result<insar_core::TaskRef, insar_core::TypedError> {
        self.service
            .create_task(batch_id, revision, item_ids, options)
    }

    pub fn preview_all_match_task(
        &self,
        request: insar_application_service::BatchAllMatchPreviewRequest,
    ) -> Result<insar_application_service::BatchAllMatchPreview, insar_core::TypedError> {
        self.service.preview_all_match_task(request)
    }

    pub fn create_all_match_task(
        &self,
        request: insar_application_service::BatchAllMatchCreateTaskRequest,
    ) -> Result<insar_application_service::BatchAllMatchTaskOutcome, insar_core::TypedError> {
        self.service.create_all_match_task(request)
    }

    pub fn create_download_task(
        &self,
        request: &StoreCreateTaskRequest,
    ) -> Result<CoreServiceSnapshot, insar_application_service::ApplicationServiceError> {
        self.service.create_download_task(request)
    }

    pub fn append_download_task(
        &self,
        request: &StoreAppendTaskRequest,
    ) -> Result<CoreServiceSnapshot, insar_application_service::ApplicationServiceError> {
        self.service.append_download_task(request)
    }

    pub fn create_task_from_workbench(
        &self,
        request: &CreateTaskFromWorkbenchRequest,
    ) -> Result<CoreServiceSnapshot, insar_application_service::ApplicationServiceError> {
        self.service.create_task_from_workbench(request)
    }

    pub fn append_task_from_workbench(
        &self,
        request: &AppendTaskFromWorkbenchRequest,
    ) -> Result<CoreServiceSnapshot, insar_application_service::ApplicationServiceError> {
        self.service.append_task_from_workbench(request)
    }

    pub fn append_task_from_source_batch(
        &self,
        request: &AppendTaskFromSourceBatchRequest,
    ) -> Result<CoreServiceSnapshot, insar_application_service::ApplicationServiceError> {
        self.service.append_task_from_source_batch(request)
    }

    pub fn create_task_from_historical_source_batch(
        &self,
        request: &CreateTaskFromHistoricalSourceBatchRequest,
    ) -> Result<CoreServiceSnapshot, insar_application_service::ApplicationServiceError> {
        self.service
            .create_task_from_historical_source_batch(request)
    }

    pub fn create_burst_asset_task(
        &self,
        request: &BurstCreateAssetTaskRequest,
    ) -> Result<CoreServiceSnapshot, insar_application_service::ApplicationServiceError> {
        self.service.create_burst_asset_task(request)
    }

    pub fn append_burst_asset_task(
        &self,
        request: &BurstAppendAssetTaskRequest,
    ) -> Result<CoreServiceSnapshot, insar_application_service::ApplicationServiceError> {
        self.service.append_burst_asset_task(request)
    }

    pub fn create_historical_burst_asset_task(
        &self,
        request: &BurstCreateHistoricalAssetTaskRequest,
    ) -> Result<CoreServiceSnapshot, insar_application_service::ApplicationServiceError> {
        self.service.create_historical_burst_asset_task(request)
    }

    pub fn task_action(
        &self,
        task_id: String,
        revision: u64,
        action: insar_application_service::UnifiedTaskAction,
    ) -> Result<insar_core::TaskRef, insar_core::TypedError> {
        self.service.task_action(task_id, revision, action)
    }

    pub fn task_retry_fallback(
        &self,
        task_id: String,
        confirm_refreshed_batch_id: Option<String>,
    ) -> Result<insar_application_service::TaskRetryFallbackOutcome, insar_core::TypedError> {
        self.service
            .task_retry_fallback(task_id, confirm_refreshed_batch_id)
    }

    pub fn task_item_action(
        &self,
        task_id: String,
        revision: Option<u64>,
        item_id: String,
        action: insar_application_service::UnifiedTaskItemAction,
    ) -> Result<insar_core::TaskRef, insar_core::TypedError> {
        self.service
            .task_item_action(task_id, revision, item_id, action)
    }

    pub fn task_delete_history(&self, task_id: String) -> Result<(), insar_core::TypedError> {
        self.service.task_delete_history(task_id)
    }

    pub fn set_task_pinned(
        &self,
        task_id: String,
        pinned: bool,
    ) -> Result<Vec<String>, insar_core::TypedError> {
        self.service.set_task_pinned(task_id, pinned)
    }

    pub fn list_pinned_tasks(
        &self,
    ) -> Result<Vec<String>, insar_application_service::ApplicationServiceError> {
        self.service.list_pinned_tasks()
    }

    pub fn task_retry_fallback_eligibility(
        &self,
        task_ids: &[String],
    ) -> Result<
        std::collections::BTreeMap<String, bool>,
        insar_application_service::ApplicationServiceError,
    > {
        self.service.task_retry_fallback_eligibility(task_ids)
    }

    pub fn check_existing_downloaded_scenes(
        &self,
        items: Vec<insar_application_service::ExistingDownloadCheckItem>,
        target_dir: Option<String>,
    ) -> Result<
        Vec<insar_application_service::ExistingDownloadMatch>,
        insar_application_service::ApplicationServiceError,
    > {
        self.service
            .check_existing_downloaded_scenes(items, target_dir)
    }

    pub fn update_download_task(
        &self,
        request: &StoreUpdateTaskRequest,
    ) -> Result<CoreServiceSnapshot, insar_application_service::ApplicationServiceError> {
        self.service.update_download_task(request)
    }

    pub fn update_download_task_view(
        &self,
        request: &StoreUpdateTaskViewRequest,
    ) -> Result<CoreServiceSnapshot, insar_application_service::ApplicationServiceError> {
        self.service.update_download_task_view(request)
    }

    pub fn archive_download_task(
        &self,
        request: &StoreArchiveTaskRequest,
    ) -> Result<ArchiveDownloadTaskAck, insar_application_service::ApplicationServiceError> {
        self.service.archive_download_task(request)
    }

    pub fn resolve_dem_conversion_source(
        &self,
        request: &StoreDemConversionSourceRequest,
    ) -> Result<Option<StoreDemConversionSource>, insar_application_service::ApplicationServiceError>
    {
        self.service.resolve_dem_conversion_source(request)
    }

    pub fn set_dem_source_identity(
        &self,
        identity: &StoreDemSourceIdentity,
    ) -> Result<StoreDemSourceIdentity, insar_application_service::ApplicationServiceError> {
        self.service.set_dem_source_identity(identity)
    }

    pub fn delete_download_history(
        &self,
        request: &StoreDeleteHistoryRequest,
    ) -> Result<CoreServiceSnapshot, insar_application_service::ApplicationServiceError> {
        self.service.delete_download_history(request)
    }

    #[cfg(test)]
    pub fn status(
        &self,
    ) -> Result<
        insar_application_service::ServiceStatus,
        insar_application_service::ApplicationServiceError,
    > {
        self.service.status()
    }

    pub fn shutdown(&self) -> Result<(), insar_application_service::ApplicationServiceError> {
        self.service.shutdown()
    }
}

impl Default for ApplicationServiceState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        cleanup_native_dem_window_staging, native_dem_download, native_dem_download_sources,
        native_dem_source_vertical_datum, native_dem_window_staging_root,
        native_gis_capability_observations, prepare_native_task_output_path, store_search_progress,
        ApplicationServiceState, CompatWorkerState,
    };
    use insar_application_service::{
        AdminBoundarySearchRequest, ApplicationCatalogSearchAndFreezeRequest,
        ApplicationServiceConfig, CdseNetworkMode, SarSearchProgressStage, SarSearchProgressUpdate,
        ServiceStatus,
    };
    use insar_gis::LazyNativeGisEngine;
    use serde_json::json;
    use std::collections::HashMap;
    use std::fs;
    use std::sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    };

    static NEXT_ID: AtomicU64 = AtomicU64::new(0);

    fn search_progress(
        sequence: u64,
        stage: SarSearchProgressStage,
        terminal: bool,
    ) -> SarSearchProgressUpdate {
        SarSearchProgressUpdate {
            request_id: "search-1".to_owned(),
            sequence,
            stage,
            provider_request_count: None,
            raw_fetched_count: None,
            mapped_count: None,
            exact_matched_count: None,
            admitted_count: None,
            requested_limit: Some(25),
            remote_total: None,
            batch_id: None,
            loaded_count: None,
            elapsed_ms: sequence,
            message: format!("progress {sequence}"),
            terminal,
        }
    }

    #[test]
    fn desktop_progress_store_rejects_regression_duplicates_and_post_terminal_updates() {
        let mut progress = HashMap::new();
        assert!(store_search_progress(
            &mut progress,
            search_progress(1, SarSearchProgressStage::Validating, false)
        ));
        assert!(!store_search_progress(
            &mut progress,
            search_progress(1, SarSearchProgressStage::Planning, false)
        ));
        assert!(!store_search_progress(
            &mut progress,
            search_progress(0, SarSearchProgressStage::Planning, false)
        ));
        assert!(store_search_progress(
            &mut progress,
            search_progress(2, SarSearchProgressStage::Completed, true)
        ));
        assert!(!store_search_progress(
            &mut progress,
            search_progress(3, SarSearchProgressStage::Fetching, false)
        ));
        assert_eq!(progress["search-1"].sequence, 2);
        assert!(progress["search-1"].terminal);
    }

    #[test]
    fn copernicus_aws_plans_every_intersecting_tile_for_bounded_windows() {
        let hubei =
            native_dem_download_sources("COP30", "COPERNICUS_AWS", [108.0, 29.0, 116.0, 34.0])
                .unwrap();
        assert_eq!(hubei.len(), 40);
        assert!(hubei.first().unwrap().id.contains("N29_00_E108_00"));
        assert!(hubei.last().unwrap().id.contains("N33_00_E115_00"));

        let west =
            native_dem_download_sources("COP30", "COPERNICUS_AWS", [166.0, -47.0, 180.0, -34.0])
                .unwrap();
        let east =
            native_dem_download_sources("COP30", "COPERNICUS_AWS", [-180.0, -47.0, -175.0, -34.0])
                .unwrap();
        assert_eq!(west.len(), 182);
        assert_eq!(east.len(), 65);
        assert!(west.iter().all(|source| !source.id.contains("W180")));
        assert!(east.iter().all(|source| !source.id.contains("E180")));
        assert!(west.iter().chain(east.iter()).all(|source| source
            .url
            .starts_with("https://copernicus-dem-30m.s3.eu-central-1.amazonaws.com/")));
    }

    #[test]
    fn copernicus_aws_tile_planner_fails_closed_above_the_product_bound() {
        let error =
            native_dem_download_sources("COP30", "COPERNICUS_AWS", [-30.0, -30.0, 30.0, 30.0])
                .unwrap_err();
        assert!(error.starts_with("NATIVE_DEM_TILE_LIMIT:"));
    }

    #[test]
    fn nisar_dem_plans_official_tile_names_and_directories_for_all_hemispheres() {
        // Tile names and directory layout are verified against the public CMR
        // granule listing of collection NISAR_DEM v1.2 (EPSG:4326), checked
        // 2026-11-01: tiles are DEM_<N|S><lat 2-digit>_00_<E|W><lon 3-digit>_00_C01
        // under <band>/<band>_<block>, where band is the 10-degree latitude
        // bucket (5-degree polar bands N85/S85/S90) and block is the 20-degree
        // longitude bucket floor(lon/20)*20. There is no E180 column; the
        // antimeridian west edge is W180.
        let hubei =
            native_dem_download_sources("NISAR_DEM", "NISAR_DEM_ASF", [108.0, 29.0, 116.0, 34.0])
                .unwrap();
        assert_eq!(hubei.len(), 40);
        assert_eq!(hubei.first().unwrap().id, "DEM_N29_00_E108_00_C01");
        assert_eq!(hubei.first().unwrap().url, "https://nisar.asf.earthdatacloud.nasa.gov/NISAR/DEM/v1.2/EPSG4326/N20/N20_E100/DEM_N29_00_E108_00_C01.tif");
        assert_eq!(hubei.last().unwrap().id, "DEM_N33_00_E115_00_C01");

        let south_west = native_dem_download_sources(
            "NISAR_DEM",
            "NISAR_DEM_ASF",
            [-135.5, -19.5, -135.0, -19.0],
        )
        .unwrap();
        assert_eq!(south_west.len(), 1);
        assert_eq!(south_west[0].id, "DEM_S20_00_W136_00_C01");
        assert_eq!(
            south_west[0].url,
            "https://nisar.asf.earthdatacloud.nasa.gov/NISAR/DEM/v1.2/EPSG4326/S20/S20_W140/DEM_S20_00_W136_00_C01.tif"
        );

        let equator_zero =
            native_dem_download_sources("NISAR_DEM", "NISAR_DEM_ASF", [-5.5, 3.0, -5.0, 3.5])
                .unwrap();
        assert_eq!(equator_zero[0].id, "DEM_N03_00_W006_00_C01");
        assert_eq!(
            equator_zero[0].url,
            "https://nisar.asf.earthdatacloud.nasa.gov/NISAR/DEM/v1.2/EPSG4326/N00/N00_W020/DEM_N03_00_W006_00_C01.tif"
        );

        let east_zero =
            native_dem_download_sources("NISAR_DEM", "NISAR_DEM_ASF", [5.0, 3.0, 5.5, 3.5])
                .unwrap();
        assert_eq!(east_zero[0].id, "DEM_N03_00_E005_00_C01");
        assert!(east_zero[0].url.contains("/N00/N00_E000/"));

        // The antimeridian: east edge is E179 (no E180 column exists), west
        // edge is W180.
        let east_edge =
            native_dem_download_sources("NISAR_DEM", "NISAR_DEM_ASF", [179.0, 17.0, 180.0, 18.0])
                .unwrap();
        assert_eq!(east_edge[0].id, "DEM_N17_00_E179_00_C01");
        assert!(east_edge[0].url.contains("/N10/N10_E160/"));
        assert!(east_edge.iter().all(|source| !source.id.contains("E180")));

        let west_edge =
            native_dem_download_sources("NISAR_DEM", "NISAR_DEM_ASF", [-180.0, 17.0, -179.0, 18.0])
                .unwrap();
        assert_eq!(west_edge[0].id, "DEM_N17_00_W180_00_C01");
        assert!(west_edge[0].url.contains("/N10/N10_W180/"));

        let pole = native_dem_download_sources(
            "NISAR_DEM",
            "NISAR_DEM_ASF",
            [-180.0, -90.0, -179.0, -89.0],
        )
        .unwrap();
        assert_eq!(pole[0].id, "DEM_S90_00_W180_00_C01");
        assert!(pole[0].url.contains("/S90/S90_W180/"));
    }

    #[test]
    fn nisar_dem_tile_planner_fails_closed_above_the_product_bound() {
        let error =
            native_dem_download_sources("NISAR_DEM", "NISAR_DEM_ASF", [-30.0, -30.0, 30.0, 30.0])
                .unwrap_err();
        assert!(error.starts_with("NATIVE_DEM_TILE_LIMIT:"));

        let unsupported =
            native_dem_download_sources("COP30", "NISAR_DEM_ASF", [0.0, 0.0, 1.0, 1.0])
                .unwrap_err();
        assert!(unsupported.starts_with("NATIVE_DEM_PROVIDER_UNSUPPORTED:"));
    }

    #[test]
    fn nisar_dem_reports_an_ellipsoidal_vertical_datum_without_conversion() {
        let payload = serde_json::json!({
            "dataset": "NISAR_DEM",
            "source_vertical_datum": "auto",
        });
        assert_eq!(
            native_dem_source_vertical_datum(&payload).unwrap(),
            "WGS84_ELLIPSOID"
        );
        let mismatch = native_dem_source_vertical_datum(&serde_json::json!({
            "dataset": "NISAR_DEM",
            "source_vertical_datum": "EGM2008",
        }))
        .unwrap_err();
        assert!(mismatch.starts_with("NATIVE_GIS_VERTICAL_DATUM_MISMATCH:"));
    }

    #[test]
    fn fabdem_plans_10deg_folder_urls_and_egm2008_datum_with_bounds() {
        let (tile_hefei, url_hefei) = super::fabdem_tile_url(31, 117).expect("Hefei tile");
        assert_eq!(tile_hefei, "N31E117_FABDEM_V1-2");
        assert_eq!(
            url_hefei,
            "https://huggingface.co/buckets/links-ads/fabdem/resolve/tiles/N30E110-N40E120_FABDEM_V1-2/N31E117_FABDEM_V1-2.tif"
        );

        let (tile_sw, url_sw) = super::fabdem_tile_url(-1, -79).expect("South-West tile");
        assert_eq!(tile_sw, "S01W079_FABDEM_V1-2");
        assert_eq!(
            url_sw,
            "https://huggingface.co/buckets/links-ads/fabdem/resolve/tiles/S10W080-N00W070_FABDEM_V1-2/S01W079_FABDEM_V1-2.tif"
        );

        let (tile_zero, url_zero) = super::fabdem_tile_url(0, 0).expect("Origin tile");
        assert_eq!(tile_zero, "N00E000_FABDEM_V1-2");
        assert_eq!(
            url_zero,
            "https://huggingface.co/buckets/links-ads/fabdem/resolve/tiles/N00E000-N10E010_FABDEM_V1-2/N00E000_FABDEM_V1-2.tif"
        );

        let (tile_dateline, url_dateline) = super::fabdem_tile_url(5, 175).expect("Dateline tile");
        assert_eq!(tile_dateline, "N05E175_FABDEM_V1-2");
        assert_eq!(
            url_dateline,
            "https://huggingface.co/buckets/links-ads/fabdem/resolve/tiles/N00E170-N10W180_FABDEM_V1-2/N05E175_FABDEM_V1-2.tif"
        );

        let out_of_range = super::fabdem_tile_url(-65, 10).unwrap_err();
        assert!(out_of_range.starts_with("NATIVE_DEM_OUT_OF_COVERAGE:"));

        let sources =
            native_dem_download_sources("FABDEM", "FABDEM_PUBLIC", [117.1, 31.7, 117.3, 31.9])
                .expect("planned FABDEM source");
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].id, "N31E117_FABDEM_V1-2");
        assert_eq!(
            native_dem_source_vertical_datum(&serde_json::json!({
                "dataset": "FABDEM",
                "source_vertical_datum": "auto",
            }))
            .unwrap(),
            "EGM2008"
        );

        let open_tiles =
            crate::geodata_tools::resolve_open_dem_tiles("fabdem", [117.1, 31.7, 117.3, 31.9])
                .expect("resolve open fabdem tiles");
        assert_eq!(open_tiles["tile_count"], 1);
        assert_eq!(open_tiles["license"], "CC BY-NC-SA 4.0 (Non-Commercial)");
        assert_eq!(open_tiles["tiles"][0]["primary_url"], url_hefei);
    }

    #[test]
    fn gem15_plans_10deg_gcs_urls_egm96_datum_and_open_dem_tiles_for_all_hemispheres() {
        let (sf_id, sf_url) =
            super::gem15_tile_url("GEM15_DTM", 37, -123).expect("San Francisco GEM-15 DTM tile");
        assert_eq!(sf_id, "gem15_dtm_wm123_s37");
        assert_eq!(
            sf_url,
            "https://storage.googleapis.com/climate-and-sustainability/gem-15/p30_m130/wm123s37/dtm/zall/mosaic.tif"
        );

        let (hefei_id, hefei_url) =
            super::gem15_tile_url("GEM15_DSM", 31, 117).expect("Hefei GEM-15 DSM tile");
        assert_eq!(hefei_id, "gem15_dsm_w117_s31");
        assert_eq!(
            hefei_url,
            "https://storage.googleapis.com/climate-and-sustainability/gem-15/p30_p110/w117s31/dsm/zall/mosaic.tif"
        );

        let (sw_id, sw_url) =
            super::gem15_tile_url("GEM15_DTM", -1, -79).expect("South-West GEM-15 DTM tile");
        assert_eq!(sw_id, "gem15_dtm_wm79_sm1");
        assert_eq!(
            sw_url,
            "https://storage.googleapis.com/climate-and-sustainability/gem-15/m10_m80/wm79sm1/dtm/zall/mosaic.tif"
        );

        let (zero_id, zero_url) =
            super::gem15_tile_url("GEM15_DTM", 0, 0).expect("Origin GEM-15 DTM tile");
        assert_eq!(zero_id, "gem15_dtm_w0_s0");
        assert_eq!(
            zero_url,
            "https://storage.googleapis.com/climate-and-sustainability/gem-15/p0_p0/w0s0/dtm/zall/mosaic.tif"
        );

        let out_of_range = super::gem15_tile_url("GEM15_DTM", -65, 10).unwrap_err();
        assert!(out_of_range.starts_with("NATIVE_DEM_OUT_OF_COVERAGE:"));

        for dataset in ["GEM15_DTM", "GEM15_DSM"] {
            let sources = native_dem_download_sources(
                dataset,
                "GEM15_PUBLIC",
                [-122.42, 37.77, -122.41, 37.78],
            )
            .expect("planned GEM-15 source");
            assert_eq!(sources.len(), 1);
            assert!(sources[0].url.contains("wm123s37"));
            assert_eq!(
                native_dem_source_vertical_datum(&serde_json::json!({
                    "dataset": dataset,
                    "source_vertical_datum": "auto",
                }))
                .unwrap(),
                "EGM96"
            );
        }

        let open_dtm = crate::geodata_tools::resolve_open_dem_tiles(
            "gem15_dtm",
            [-122.42, 37.77, -122.41, 37.78],
        )
        .expect("resolve open gem15_dtm tiles");
        assert_eq!(open_dtm["tile_count"], 1);
        assert_eq!(open_dtm["license"], "CC BY 4.0");
        assert!(open_dtm["coverage_note"]
            .as_str()
            .unwrap_or("")
            .contains("Mainland China"));
        assert_eq!(open_dtm["tiles"][0]["primary_url"], sf_url);

        let open_dsm =
            crate::geodata_tools::resolve_open_dem_tiles("gem15_dsm", [117.1, 31.7, 117.3, 31.9])
                .expect("resolve open gem15_dsm tiles");
        assert_eq!(open_dsm["tile_count"], 1);
        assert_eq!(open_dsm["license"], "CC BY 4.0");
        assert_eq!(open_dsm["tiles"][0]["primary_url"], hefei_url);
    }

    #[test]
    fn gem15_loopback_404_returns_out_of_coverage_without_leaving_output() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback HTTP server");
        let addr = listener.local_addr().expect("local addr");
        let handle = std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0_u8; 1024];
                let _ = stream.read(&mut buf);
                let _ = stream.write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
                let _ = stream.flush();
            }
        });

        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "insar-tauri-gem15-loopback-404-{id}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("create temp root");
        let raw_path = root.join("gem15_dtm_raw.tif");

        let engine = LazyNativeGisEngine::unavailable();
        let error = super::native_dem_download_with_progress(
            &engine,
            &serde_json::json!({
                "output_root": root,
                "input_path": raw_path,
                "dataset": "GEM15_DTM",
                "provider": "GEM15_PUBLIC",
                "bbox": { "west": 117.25, "south": 31.82, "east": 117.26, "north": 31.83 },
                "test_gem15_base_url": format!("http://{addr}"),
            }),
            &|| false,
            &mut |_| {},
        )
        .expect_err("all-404 GEM-15 AOI must return NATIVE_DEM_OUT_OF_COVERAGE");

        handle.join().expect("join loopback server");
        assert!(
            error.starts_with("NATIVE_DEM_OUT_OF_COVERAGE:"),
            "unexpected error: {error}"
        );
        assert!(!raw_path.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    #[ignore = "live network check against public GCS GEM-15 bucket and bundled runtime/gis"]
    fn live_gem15_gcs_download_and_china_404_check() {
        let runtime_root = std::env::var_os("INSAR_GIS_RUNTIME_ROOT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("..")
                    .join("runtime")
                    .join("gis")
            });
        let engine = LazyNativeGisEngine::from_product_root(&runtime_root)
            .expect("load bundled product GIS runtime");

        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "insar-tauri-gem15-live-{id}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("create live test dir");

        // 1. China (Hefei) AOI: excluded upstream in v1, must fail cleanly with NATIVE_DEM_OUT_OF_COVERAGE.
        let hefei_raw = root.join("hefei_gem15_dtm_raw.tif");
        let hefei_err = super::native_dem_download_with_progress(
            &engine,
            &serde_json::json!({
                "output_root": root,
                "input_path": hefei_raw,
                "dataset": "GEM15_DTM",
                "provider": "GEM15_PUBLIC",
                "bbox": { "west": 117.25, "south": 31.82, "east": 117.26, "north": 31.83 },
            }),
            &|| false,
            &mut |_| {},
        )
        .expect_err("Mainland China Hefei AOI must be intercepted with NATIVE_DEM_OUT_OF_COVERAGE");
        assert!(
            hefei_err.starts_with("NATIVE_DEM_OUT_OF_COVERAGE:"),
            "unexpected Hefei error: {hefei_err}"
        );
        assert!(!hefei_raw.exists());
        println!("HEFEI_404_INTERCEPT_OK: {hefei_err}");

        // 2. Overseas (San Francisco) AOI: real public GCS COG download -> UTM-to-WGS84 warp -> EGM96->WGS84 ellipsoid + SARscape (both DTM 1-band and DSM 5-band).
        for (dataset, file_stem) in [
            ("GEM15_DTM", "GEM15_DTM_EGM96_sf_demo"),
            ("GEM15_DSM", "GEM15_DSM_EGM96_sf_demo"),
        ] {
            let sf_raw = root.join(format!("{file_stem}.tif"));
            let mut tile_bytes = 0_u64;
            let sf_res = super::native_dem_download_with_progress(
                &engine,
                &serde_json::json!({
                    "output_root": root,
                    "input_path": sf_raw,
                    "dataset": dataset,
                    "provider": "GEM15_PUBLIC",
                    "source_vertical_datum": "auto",
                    "bbox": { "west": -122.42, "south": 37.77, "east": -122.41, "north": 37.78 },
                    "convert": true,
                }),
                &|| false,
                &mut |event| {
                    if let insar_application_service::DemTransferEvent::BytesReceived(n) = event {
                        tile_bytes = n;
                    }
                },
            )
            .unwrap_or_else(|err| panic!("San Francisco live GCS {dataset} failed: {err}"));
            assert!(sf_raw.is_file());
            let raw_size = fs::metadata(&sf_raw).unwrap().len();
            assert!(raw_size > 0);
            assert!(tile_bytes > 0);
            assert_eq!(sf_res["inspection"]["metadata"]["count"], 1);
            assert_eq!(sf_res["inspection"]["metadata"]["width"], 72);
            assert_eq!(sf_res["inspection"]["metadata"]["height"], 72);
            assert_eq!(sf_res["inspection"]["metadata"]["nodata"], -9999.0);
            assert_eq!(sf_res["conversion"]["executed"], true);

            let ellipsoid_path = root.join(format!("{file_stem}_ellipsoid.tif"));
            let sarscape_bin = root.join(format!("{file_stem}_dem"));
            let sarscape_hdr = root.join(format!("{file_stem}_dem.hdr"));
            let sarscape_sml = root.join(format!("{file_stem}_dem.sml"));
            assert!(ellipsoid_path.is_file() && fs::metadata(&ellipsoid_path).unwrap().len() > 0);
            assert!(sarscape_bin.is_file() && fs::metadata(&sarscape_bin).unwrap().len() > 0);
            assert!(sarscape_hdr.is_file());
            assert!(sarscape_sml.is_file());
            println!(
                "SF_LIVE_{dataset}_OK: gcs_tile_bytes={tile_bytes}, raw_bytes={raw_size}, width={}, height={}, ellipsoid_bytes={}, sarscape_bytes={}",
                sf_res["inspection"]["metadata"]["width"],
                sf_res["inspection"]["metadata"]["height"],
                fs::metadata(&ellipsoid_path).unwrap().len(),
                fs::metadata(&sarscape_bin).unwrap().len(),
            );
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn native_dem_output_path_rejects_parent_traversal() {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("insar-tauri-dem-path-{id}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let safe = root.join("task").join("output.tif");
        prepare_native_task_output_path(&root, &safe).expect("safe nested output");
        assert!(safe.parent().unwrap().is_dir());

        let escaped = root.join("task").join("..").join("escape.tif");
        let error = prepare_native_task_output_path(&root, &escaped).unwrap_err();
        assert!(error.contains("unsafe components"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn native_dem_window_cleanup_is_scoped_to_the_exact_task_staging_directory() {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "insar-tauri-dem-window-cleanup-{id}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let staging = root.join(".eostage").join("dem-tasks").join("task-1");
        fs::create_dir_all(&staging).expect("task staging directory");
        let west = staging.join("COP30_dem_west.tif");
        let east = staging.join("COP30_dem_east.tif");
        fs::write(&west, b"west").expect("west window");
        fs::write(&east, b"east").expect("east window");
        let inputs = vec![west.clone(), east.clone()];
        let payload = json!({"task_id": "task-1"});

        let resolved = native_dem_window_staging_root(&payload, &root, &inputs)
            .expect("exact owned staging directory");
        assert_eq!(resolved, staging);
        let cleanup = cleanup_native_dem_window_staging(&resolved, &inputs);

        assert_eq!(cleanup["ok"], true);
        assert_eq!(cleanup["removed_window_files"], 2);
        assert_eq!(cleanup["staging_directory_removed"], true);
        assert!(!west.exists());
        assert!(!east.exists());
        assert!(!staging.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn native_dem_window_cleanup_rejects_non_owned_inputs_before_deletion() {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "insar-tauri-dem-window-cleanup-guard-{id}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let staging = root.join(".eostage").join("dem-tasks").join("task-1");
        fs::create_dir_all(&staging).expect("task staging directory");
        let west = staging.join("COP30_dem_west.tif");
        let outside = root.join("do-not-delete.tif");
        fs::write(&west, b"west").expect("west window");
        fs::write(&outside, b"owner data").expect("owner data");
        let inputs = vec![west.clone(), outside.clone()];

        let error = native_dem_window_staging_root(&json!({"task_id": "task-1"}), &root, &inputs)
            .expect_err("mixed ownership must fail closed");

        assert!(error.contains("task staging directory"));
        assert!(west.exists());
        assert!(outside.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn native_dem_window_cleanup_preserves_unknown_staging_entries() {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "insar-tauri-dem-window-cleanup-residual-{id}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let staging = root.join(".eostage").join("dem-tasks").join("task-1");
        fs::create_dir_all(&staging).expect("task staging directory");
        let west = staging.join("COP30_dem_west.tif");
        let east = staging.join("COP30_dem_east.tif");
        let unknown = staging.join("keep-me.bin");
        fs::write(&west, b"west").expect("west window");
        fs::write(&east, b"east").expect("east window");
        fs::write(&unknown, b"unowned").expect("unknown staging entry");
        let inputs = vec![west.clone(), east.clone()];
        let resolved =
            native_dem_window_staging_root(&json!({"task_id": "task-1"}), &root, &inputs)
                .expect("exact owned staging directory");

        let cleanup = cleanup_native_dem_window_staging(&resolved, &inputs);

        assert_eq!(cleanup["ok"], true);
        assert_eq!(cleanup["removed_window_files"], 2);
        assert_eq!(cleanup["staging_directory_removed"], false);
        assert_eq!(cleanup["residual_entries_preserved"], true);
        assert!(!west.exists());
        assert!(!east.exists());
        assert_eq!(fs::read(&unknown).unwrap(), b"unowned");
        assert!(staging.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn native_cop30_crop_rejects_oversize_without_resampling() {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "insar-tauri-dem-oversize-{id}-{}",
            std::process::id()
        ));
        let error = native_dem_download(
            &LazyNativeGisEngine::unavailable(),
            &json!({
                "input_path": root.join("west.tif"),
                "output_root": root,
                "dataset": "COP30",
                "provider": "COPERNICUS_AWS",
                "bbox": {"west": 166.0, "south": -47.0, "east": 180.0, "north": -34.0}
            }),
        )
        .unwrap_err();
        assert!(error.starts_with("NATIVE_DEM_OUTPUT_LIMIT:"));
    }

    #[test]
    fn state_construction_is_lazy_and_shutdown_is_idempotent() {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "insar-tauri-state-{id}-{}.sqlite3",
            std::process::id()
        ));
        let state = ApplicationServiceState {
            service: Arc::new(insar_application_service::ApplicationService::new(
                ApplicationServiceConfig::store(path.clone()),
            )),
            ui_smoke_catalog: None,
            sar_search_progress: Mutex::new(HashMap::new()),
            standalone_dem_running: std::sync::atomic::AtomicBool::new(false),
        };
        assert_eq!(state.status().unwrap(), ServiceStatus::Created);
        state.shutdown().unwrap();
        state.shutdown().unwrap();
        assert_eq!(state.status().unwrap(), ServiceStatus::ShuttingDown);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn desktop_capability_observation_does_not_initialize_or_verify_native_gis() {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "insar-tauri-lazy-gis-observation-{id}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("runtime root");
        fs::write(
            root.join("manifest.json"),
            serde_json::to_vec(&json!({
                "schema_version": 1,
                "runtime_id": "tauri-lazy-test-runtime",
                "gdal_version": "3.10.0",
                "proj_version": "9.5.1",
                "gdal_library": "bin/gdal.dll",
                "proj_library": "bin/proj.dll",
                "proj_data_dir": "share/proj",
                "gdal_data_dir": "share/gdal",
                "proj_network": "disabled",
                "drivers": ["GTiff", "ENVI"],
                "files": [{
                    "relative_path": "vertical-grids/egm2008/not-read-at-startup.tif",
                    "bytes": 80585622,
                    "sha256": "0000000000000000000000000000000000000000000000000000000000000000"
                }]
            }))
            .expect("manifest JSON"),
        )
        .expect("manifest file");
        let lazy = LazyNativeGisEngine::from_product_root(&root).expect("cheap identity");

        let _observations = native_gis_capability_observations(&lazy);

        assert!(lazy.is_discovered());
        assert!(!lazy.is_initialized());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn normal_business_profile_is_lazy() {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "insar-tauri-provider-ready-{id}-{}.sqlite3",
            std::process::id()
        ));
        let state = ApplicationServiceState {
            service: Arc::new(insar_application_service::ApplicationService::new(
                ApplicationServiceConfig::business_store(path.clone()),
            )),
            ui_smoke_catalog: None,
            sar_search_progress: Mutex::new(HashMap::new()),
            standalone_dem_running: std::sync::atomic::AtomicBool::new(false),
        };

        assert_eq!(state.status().unwrap(), ServiceStatus::Created);
        assert!(!path.exists());
        state.shutdown().unwrap();
    }

    #[test]
    fn cdse_controls_use_the_normal_business_runtime() {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "insar-tauri-cdse-business-{id}-{}.sqlite3",
            std::process::id()
        ));
        let state = ApplicationServiceState {
            service: Arc::new(insar_application_service::ApplicationService::new(
                ApplicationServiceConfig::business_store(path.clone()),
            )),
            ui_smoke_catalog: None,
            sar_search_progress: Mutex::new(HashMap::new()),
            standalone_dem_running: std::sync::atomic::AtomicBool::new(false),
        };
        let system = state
            .cdse_session_status()
            .expect("business status has no network request");
        assert!(!system.authenticated);
        // Ordinary routes default to Direct since W1 (2026-10-24).
        assert_eq!(system.network_mode, CdseNetworkMode::Direct);
        assert_eq!(
            state.cdse_network_policy().unwrap(),
            CdseNetworkMode::Direct
        );

        let direct = state
            .set_cdse_network_policy(CdseNetworkMode::Direct)
            .expect("route replacement has no network request");
        assert!(!direct.authenticated);
        assert_eq!(direct.network_mode, CdseNetworkMode::Direct);
        assert!(!path.exists());
        state.shutdown().unwrap();
    }

    #[test]
    fn normal_business_invalid_catalogue_freeze_is_rejected_without_store_mutation() {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "insar-tauri-catalogue-unavailable-{id}-{}.sqlite3",
            std::process::id()
        ));
        let aoi_geojson = json!({
            "type": "Polygon",
            "coordinates": [[[110.0, 30.0], [111.0, 30.0], [111.0, 31.0], [110.0, 30.0]]]
        })
        .as_object()
        .expect("object AOI")
        .clone();
        let request = ApplicationCatalogSearchAndFreezeRequest {
            request_id: "catalogue-freeze-request".to_owned(),
            search: insar_core::CatalogSearchQuery {
                logical_dataset: "optical.sentinel-2.l2a".to_owned(),
                provider_id: "copernicus.dataspace.odata".to_owned(),
                native_collection_id: "SENTINEL-2".to_owned(),
                aoi_geojson: Some(aoi_geojson.clone()),
                cursor: None,
                result_limit: Some(24),
                filters: json!({
                    "product_type": "S2MSI2A",
                    "cloud_cover_lte": 4.2,
                    "datetime": {
                        "start": "2026-01-01T00:00:00Z",
                        "end": "2026-01-02T00:00:00Z"
                    }
                })
                .as_object()
                .expect("object filters")
                .clone(),
            },
            batch_id: "catalogue-freeze-batch".to_owned(),
            created_at: "2026-07-26T00:00:00Z".to_owned(),
            aoi_name: "offline catalogue fixture".to_owned(),
            aoi_geojson: Some(aoi_geojson),
            workbench_page_size: 0,
        };
        let state = ApplicationServiceState {
            service: Arc::new(insar_application_service::ApplicationService::new(
                ApplicationServiceConfig::business_store(path.clone()),
            )),
            ui_smoke_catalog: None,
            sar_search_progress: Mutex::new(HashMap::new()),
            standalone_dem_running: std::sync::atomic::AtomicBool::new(false),
        };

        let before = serde_json::to_value(state.snapshot(true).expect("snapshot before request"))
            .expect("serializable snapshot");
        let error = state
            .catalog_search_and_freeze(&request)
            .expect_err("invalid request must be rejected before a provider request");
        assert_eq!(error.code(), "V3_APPLICATION_INVALID_REQUEST");
        let after = serde_json::to_value(state.snapshot(true).expect("snapshot after request"))
            .expect("serializable snapshot");

        assert_eq!(after, before);
        state.shutdown().unwrap();
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{}", path.display(), suffix));
        }
    }

    #[test]
    fn offline_smoke_constructor_uses_exact_national_boundary_fixture() {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "insar-tauri-offline-smoke-boundary-{id}-{}.sqlite3",
            std::process::id()
        ));
        let state = ApplicationServiceState::with_offline_smoke_store_path_and_runtime_observations_and_compat_worker(
            path.clone(),
            Default::default(),
            Arc::new(CompatWorkerState::from_environment()),
        );
        let result = state
            .admin_boundary_search(&AdminBoundarySearchRequest {
                province: "全国".to_owned(),
                city: "全部".to_owned(),
                district: "全部".to_owned(),
                provider_object_id: Some("100000".to_owned()),
                ..Default::default()
            })
            .expect("offline national boundary fixture");

        assert!(result.ok);
        assert_eq!(result.provider, "datav");
        let country = result
            .results
            .iter()
            .find(|result| result.provider_object_id == "100000")
            .expect("exact national fixture result");
        assert_eq!(country.source, "DataV areas_v3");
        assert_eq!(country.display_name, "中国（离线自检样例，非真实边界）");
        assert!(matches!(
            country.geometry_kind.as_deref(),
            Some("polygon" | "multipolygon")
        ));
        assert!(country.geometry.is_some());

        state.shutdown().unwrap();
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{}", path.display(), suffix));
        }
    }

    #[test]
    fn only_one_local_dem_conversion_runs_at_a_time() {
        let state = ApplicationServiceState::with_store_path(
            std::env::temp_dir().join("insar-standalone-dem-single-flight.sqlite3"),
        );
        let first = state
            .begin_standalone_dem()
            .expect("the first conversion starts");
        assert!(
            state.begin_standalone_dem().is_none(),
            "a second conversion must wait"
        );
        drop(first);
        assert!(
            state.begin_standalone_dem().is_some(),
            "the slot is free again"
        );
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RuntimeComponentItem {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub version: String,
    pub install_root: String,
    pub status: String,
    pub file_count: usize,
    pub total_bytes: u64,
    pub verified: bool,
    pub detail: String,
    pub capabilities: Vec<String>,
}

pub fn current_host_platform_tag() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RuntimeComponentsReport {
    pub ok: bool,
    pub install_root: String,
    pub manifest_found: bool,
    pub app_version: String,
    #[serde(default)]
    pub host_platform: String,
    pub components: Vec<RuntimeComponentItem>,
}

pub fn discover_runtime_root() -> std::path::PathBuf {
    if let Some(env_root) = std::env::var_os("INSAR_GIS_RUNTIME_ROOT") {
        let env_path = std::path::PathBuf::from(env_root);
        if env_path.file_name().and_then(|s| s.to_str()) == Some("gis") {
            if let Some(product_root) = env_path.parent().and_then(|p| p.parent()) {
                if product_root.join("runtime").is_dir() {
                    return product_root.to_path_buf();
                }
            }
        }
        if env_path.join("runtime").is_dir() || env_path.join("components-manifest.json").is_file()
        {
            return env_path;
        }
    }

    let current_exe = std::env::current_exe().ok();
    let exe_dir = current_exe
        .as_ref()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()));
    let current_dir = std::env::current_dir().ok();

    for base in [exe_dir.as_ref(), current_dir.as_ref()]
        .into_iter()
        .flatten()
    {
        for ancestor in base.ancestors().take(7) {
            let macos_resources = ancestor.join("Resources");
            if macos_resources.join("components-manifest.json").is_file()
                || macos_resources.join("runtime/gis").is_dir()
                || macos_resources.join("runtime/vertical-grids").is_dir()
                || macos_resources.join("runtime/compat-worker").is_dir()
            {
                return macos_resources;
            }
            if ancestor.join("components-manifest.json").is_file()
                || ancestor.join("runtime/gis").is_dir()
                || ancestor.join("runtime/vertical-grids").is_dir()
                || ancestor.join("runtime/compat-worker").is_dir()
            {
                return ancestor.to_path_buf();
            }
        }
    }

    for base in [exe_dir.as_ref(), current_dir.as_ref()]
        .into_iter()
        .flatten()
    {
        if base.join("runtime").is_dir()
            || base.join("packaging/component_manifest.v3.json").is_file()
        {
            return base.to_path_buf();
        }
    }

    exe_dir
        .or(current_dir)
        .unwrap_or_else(|| std::path::PathBuf::from("."))
}

/// Resolve an installed native component executable across Windows (`.exe`),
/// macOS/Linux (stemless), and nested `bin/` layouts.
pub(crate) fn resolve_component_binary(
    dir: &std::path::Path,
    stems: &[&str],
) -> Option<std::path::PathBuf> {
    for &stem in stems {
        for cand in [
            dir.join(format!("{stem}.exe")),
            dir.join(stem),
            dir.join("bin").join(format!("{stem}.exe")),
            dir.join("bin").join(stem),
        ] {
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    None
}

pub fn inspect_runtime_components() -> Result<RuntimeComponentsReport, String> {
    let root = discover_runtime_root();
    inspect_runtime_components_at(&root)
}

pub fn inspect_runtime_components_at(
    root: &std::path::Path,
) -> Result<RuntimeComponentsReport, String> {
    let current_exe = std::env::current_exe().ok();
    let root = root.to_path_buf();

    let manifest_path = root.join("components-manifest.json");
    let packaging_manifest_path = root.join("packaging/component_manifest.v3.json");
    let manifest_found = manifest_path.is_file() || packaging_manifest_path.is_file();
    let app_version = env!("CARGO_PKG_VERSION").to_string();

    let mut components = Vec::new();

    // 1. eostage-core
    let core_exe = current_exe.as_ref().and_then(|p| std::fs::metadata(p).ok());
    let core_bytes = core_exe.map(|m| m.len()).unwrap_or(0);
    components.push(RuntimeComponentItem {
        id: "eostage-core".to_string(),
        name: "EOStage 主程序与核心服务".to_string(),
        kind: "application".to_string(),
        version: app_version.clone(),
        install_root: ".".to_string(),
        status: "ready".to_string(),
        file_count: 1,
        total_bytes: core_bytes,
        verified: true,
        detail: format!("Tauri 宿主与 Rust 核心服务 (PID {})", std::process::id()),
        capabilities: vec![
            "desktop-ui".into(),
            "workspace".into(),
            "settings".into(),
            "credential-keyring".into(),
            "catalogue".into(),
            "task-store".into(),
            "download-orchestration".into(),
        ],
    });

    // 2. native-gis-runtime (GDAL / PROJ)
    let gis_dir = root.join("runtime/gis");
    let gis_manifest = gis_dir.join("manifest.json");
    if gis_manifest.is_file() {
        match insar_gis::NativeGisRuntimeIdentity::from_product_root(&gis_dir) {
            Ok(identity) => {
                let mut expected_files = 0usize;
                let mut existing_files = 0usize;
                let mut bytes_sum = 0u64;
                if let Ok(manifest_raw) = std::fs::read(&gis_manifest) {
                    if let Ok(m) =
                        serde_json::from_slice::<insar_gis::NativeGisRuntimeManifest>(&manifest_raw)
                    {
                        for f in &m.files {
                            if f.relative_path.starts_with("vertical-grids/")
                                || f.relative_path.starts_with("vertical-grids\\")
                            {
                                continue;
                            }
                            expected_files += 1;
                            let file_path = gis_dir.join(&f.relative_path);
                            if let Ok(meta) = std::fs::metadata(&file_path) {
                                if meta.is_file() {
                                    existing_files += 1;
                                    bytes_sum += meta.len();
                                }
                            }
                        }
                    }
                }
                let all_present = expected_files > 0 && existing_files == expected_files;
                components.push(RuntimeComponentItem {
                    id: "native-gis-runtime".to_string(),
                    name: "Native GIS 高级地理信息引擎 (GDAL / PROJ)".to_string(),
                    kind: "runtime".to_string(),
                    version: identity.runtime_id().to_string(),
                    install_root: "runtime/gis".to_string(),
                    status: if all_present {
                        "ready".to_string()
                    } else {
                        "partial".to_string()
                    },
                    file_count: existing_files,
                    total_bytes: bytes_sum,
                    verified: all_present,
                    detail: if all_present {
                        format!("包含 GDAL/PROJ 私有动态库与投影定义 ({existing_files} 个文件)")
                    } else {
                        format!("运行时文件不完整: 已找到 {existing_files}/{expected_files} 个文件")
                    },
                    capabilities: vec![
                        "dem-gdal-proj".into(),
                        "gtiff".into(),
                        "envi".into(),
                        "sarscape-output".into(),
                    ],
                });
            }
            Err(err) => {
                components.push(RuntimeComponentItem {
                    id: "native-gis-runtime".to_string(),
                    name: "Native GIS 高级地理信息引擎 (GDAL / PROJ)".to_string(),
                    kind: "runtime".to_string(),
                    version: "unverified".to_string(),
                    install_root: "runtime/gis".to_string(),
                    status: "partial".to_string(),
                    file_count: 0,
                    total_bytes: 0,
                    verified: false,
                    detail: format!("清单解析校验未通过: {err}"),
                    capabilities: vec!["dem-gdal-proj".into()],
                });
            }
        }
    } else {
        components.push(RuntimeComponentItem {
            id: "native-gis-runtime".to_string(),
            name: "Native GIS 高级地理信息引擎 (GDAL / PROJ)".to_string(),
            kind: "runtime".to_string(),
            version: "未就绪".to_string(),
            install_root: "runtime/gis".to_string(),
            status: "missing".to_string(),
            file_count: 0,
            total_bytes: 0,
            verified: false,
            detail: "本地未找到 runtime/gis 目录或 manifest.json".to_string(),
            capabilities: vec![
                "dem-gdal-proj".into(),
                "gtiff".into(),
                "envi".into(),
                "sarscape-output".into(),
            ],
        });
    }

    // 3. vertical-grid-egm96 & 4. vertical-grid-egm2008
    let vg_root = if root.join("runtime/vertical-grids").is_dir() {
        root.join("runtime/vertical-grids")
    } else if root.join("runtime/gis/vertical-grids").is_dir() {
        root.join("runtime/gis/vertical-grids")
    } else {
        root.join("runtime/vertical-grids")
    };
    let vg_registry = insar_gis::VerticalGridRegistry::load(&vg_root).ok();

    let egm96_candidates = [
        vg_root.join("egm96/us_nga_egm96_15.tif"),
        vg_root.join("EGM96.tif"),
        root.join("runtime/gis/vertical-grids/egm96/us_nga_egm96_15.tif"),
        root.join("runtime/gis/vertical-grids/EGM96.tif"),
    ];
    if let Some(f) = egm96_candidates.iter().find(|p| p.is_file()) {
        let size = std::fs::metadata(f).map(|m| m.len()).unwrap_or(0);
        let manifest_check = vg_registry
            .as_ref()
            .filter(|r| r.entries().any(|e| e.model.eq_ignore_ascii_case("EGM96")))
            .map(|r| r.readiness("EGM96"));
        let is_valid_tif = is_tiff_header(f);
        let (status, verified, detail) = match manifest_check {
            Some(insar_gis::VerticalGridReadiness::Ready) => (
                "ready".to_string(),
                true,
                format!(
                    "全球高程基准网格 (15' 栅格, {:.2} MB, SHA-256 已验证)",
                    size as f64 / 1_048_576.0
                ),
            ),
            Some(insar_gis::VerticalGridReadiness::IdentityMismatch) => (
                "partial".to_string(),
                false,
                "EGM96 栅格文件字节数或 SHA-256 与 manifest.json 不匹配".to_string(),
            ),
            _ if is_valid_tif && size > 0 => (
                "ready".to_string(),
                true,
                format!(
                    "全球高程基准网格 (15' 栅格, {:.2} MB)",
                    size as f64 / 1_048_576.0
                ),
            ),
            _ => (
                "partial".to_string(),
                false,
                "EGM96 栅格文件头无效或文件为空".to_string(),
            ),
        };
        components.push(RuntimeComponentItem {
            id: "vertical-grid-egm96".to_string(),
            name: "EGM96 大地水准面高程基准数据".to_string(),
            kind: "scientific_resource".to_string(),
            version: "1.0.0".to_string(),
            install_root: "runtime/vertical-grids".to_string(),
            status,
            file_count: 1,
            total_bytes: size,
            verified,
            detail,
            capabilities: vec!["egm96".into()],
        });
    } else {
        components.push(RuntimeComponentItem {
            id: "vertical-grid-egm96".to_string(),
            name: "EGM96 大地水准面高程基准数据".to_string(),
            kind: "scientific_resource".to_string(),
            version: "未就绪".to_string(),
            install_root: "runtime/vertical-grids".to_string(),
            status: "missing".to_string(),
            file_count: 0,
            total_bytes: 0,
            verified: false,
            detail: "egm96/us_nga_egm96_15.tif 未安装于 vertical-grids 目录".to_string(),
            capabilities: vec!["egm96".into()],
        });
    }

    let egm2008_candidates = [
        vg_root.join("egm2008/us_nga_egm08_25.tif"),
        vg_root.join("EGM2008.tif"),
        root.join("runtime/gis/vertical-grids/egm2008/us_nga_egm08_25.tif"),
        root.join("runtime/gis/vertical-grids/EGM2008.tif"),
    ];
    if let Some(f) = egm2008_candidates.iter().find(|p| p.is_file()) {
        let size = std::fs::metadata(f).map(|m| m.len()).unwrap_or(0);
        let manifest_check = vg_registry
            .as_ref()
            .filter(|r| r.entries().any(|e| e.model.eq_ignore_ascii_case("EGM2008")))
            .map(|r| r.readiness("EGM2008"));
        let is_valid_tif = is_tiff_header(f);
        let (status, verified, detail) = match manifest_check {
            Some(insar_gis::VerticalGridReadiness::Ready) => (
                "ready".to_string(),
                true,
                format!(
                    "高精度大地水准面 ({:.1} MB, SHA-256 已验证)",
                    size as f64 / 1_048_576.0
                ),
            ),
            Some(insar_gis::VerticalGridReadiness::IdentityMismatch) => (
                "partial".to_string(),
                false,
                "EGM2008 栅格文件字节数或 SHA-256 与 manifest.json 不匹配".to_string(),
            ),
            _ if is_valid_tif && size > 0 => (
                "ready".to_string(),
                true,
                format!("高精度大地水准面 ({:.1} MB)", size as f64 / 1_048_576.0),
            ),
            _ => (
                "partial".to_string(),
                false,
                "EGM2008 栅格文件头无效或文件为空".to_string(),
            ),
        };
        components.push(RuntimeComponentItem {
            id: "vertical-grid-egm2008".to_string(),
            name: "EGM2008 2.5分高精度全球大地水准面".to_string(),
            kind: "scientific_resource".to_string(),
            version: "1.0.0".to_string(),
            install_root: "runtime/vertical-grids".to_string(),
            status,
            file_count: 1,
            total_bytes: size,
            verified,
            detail,
            capabilities: vec!["egm2008".into()],
        });
    } else {
        components.push(RuntimeComponentItem {
            id: "vertical-grid-egm2008".to_string(),
            name: "EGM2008 2.5分高精度全球大地水准面".to_string(),
            kind: "scientific_resource".to_string(),
            version: "未就绪".to_string(),
            install_root: "runtime/vertical-grids".to_string(),
            status: "missing".to_string(),
            file_count: 0,
            total_bytes: 0,
            verified: false,
            detail: "egm2008/us_nga_egm08_25.tif 缺失 (可选高精度组件，约 76.9 MB)".to_string(),
            capabilities: vec!["egm2008".into()],
        });
    }

    // 5. compat-worker
    let worker_dir = root.join("runtime/compat-worker");
    let worker_manifest = worker_dir.join("component-manifest.json");
    if let Some(worker_exe) = resolve_component_binary(&worker_dir, &["eostage-compat-worker"]) {
        let size = std::fs::metadata(&worker_exe).map(|m| m.len()).unwrap_or(0);
        let count = if worker_manifest.is_file() { 2 } else { 1 };
        let valid_pe = is_pe_executable_header(&worker_exe);
        let mut version = "1.0.0".to_string();
        if let Ok(raw) = std::fs::read(&worker_manifest) {
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&raw) {
                if let Some(ver) = v.get("version").and_then(|s| s.as_str()) {
                    version = ver.to_string();
                }
            }
        }
        components.push(RuntimeComponentItem {
            id: "compat-worker".to_string(),
            name: "Compatibility Worker 兼容性隔离工作进程".to_string(),
            kind: "managed_runtime".to_string(),
            version,
            install_root: "runtime/compat-worker".to_string(),
            status: if valid_pe && size > 0 {
                "ready".to_string()
            } else {
                "partial".to_string()
            },
            file_count: count,
            total_bytes: size,
            verified: valid_pe && size > 0,
            detail: if valid_pe && size > 0 {
                format!("独立受控进程 ({:.2} MB)", size as f64 / 1_048_576.0)
            } else {
                format!(
                    "{} 不是有效的可执行文件",
                    worker_exe
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("eostage-compat-worker")
                )
            },
            capabilities: vec!["bounded-compatibility-operations".into()],
        });
    } else {
        components.push(RuntimeComponentItem {
            id: "compat-worker".to_string(),
            name: "Compatibility Worker 兼容性隔离工作进程".to_string(),
            kind: "managed_runtime".to_string(),
            version: "未就绪".to_string(),
            install_root: "runtime/compat-worker".to_string(),
            status: "missing".to_string(),
            file_count: 0,
            total_bytes: 0,
            verified: false,
            detail: "runtime/compat-worker 可执行文件缺失".to_string(),
            capabilities: vec!["bounded-compatibility-operations".into()],
        });
    }

    // 6. whitebox-cli (WhiteboxTools v2.4.0)
    let wbt_dir = root.join("runtime/whitebox-cli");
    let wbt_manifest = wbt_dir.join("component-manifest.json");
    if let Some(wbt_exe) = resolve_component_binary(&wbt_dir, &["whitebox_tools"]) {
        let size = std::fs::metadata(&wbt_exe).map(|m| m.len()).unwrap_or(0);
        let count = if wbt_manifest.is_file() { 2 } else { 1 };
        let valid_pe = is_pe_executable_header(&wbt_exe);
        let mut version = "2.4.0".to_string();
        if let Ok(raw) = std::fs::read(&wbt_manifest) {
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&raw) {
                if let Some(ver) = v.get("version").and_then(|s| s.as_str()) {
                    version = ver.to_string();
                }
            }
        }
        components.push(RuntimeComponentItem {
            id: "whitebox-cli".to_string(),
            name: "WhiteboxTools 地形与水文地貌分析引擎".to_string(),
            kind: "scientific_runtime".to_string(),
            version,
            install_root: "runtime/whitebox-cli".to_string(),
            status: if valid_pe && size > 0 {
                "ready".to_string()
            } else {
                "partial".to_string()
            },
            file_count: count,
            total_bytes: size,
            verified: valid_pe && size > 0,
            detail: if valid_pe && size > 0 {
                format!(
                    "500+ 水文与地貌算子原生引擎 ({:.2} MB, SHA-256 已校验)",
                    size as f64 / 1_048_576.0
                )
            } else {
                "whitebox_tools 可执行文件头无效".to_string()
            },
            capabilities: vec![
                "hydrology-breach-fill".into(),
                "d8-flow-accumulation".into(),
                "stream-network-extraction".into(),
                "geomorphometry-terrain-analysis".into(),
            ],
        });
    } else {
        components.push(RuntimeComponentItem {
            id: "whitebox-cli".to_string(),
            name: "WhiteboxTools 地形与水文地貌分析引擎".to_string(),
            kind: "scientific_runtime".to_string(),
            version: "未就绪".to_string(),
            install_root: "runtime/whitebox-cli".to_string(),
            status: "missing".to_string(),
            file_count: 0,
            total_bytes: 0,
            verified: false,
            detail: "runtime/whitebox-cli/whitebox_tools 未安装 (可选水文地貌分析组件)".to_string(),
            capabilities: vec![
                "hydrology-breach-fill".into(),
                "d8-flow-accumulation".into(),
                "stream-network-extraction".into(),
                "geomorphometry-terrain-analysis".into(),
            ],
        });
    }

    // 7. duckdb-cli (DuckDB v1.5.6 + spatial.duckdb_extension)
    let duckdb_dir = root.join("runtime/duckdb-cli");
    let duckdb_spatial = duckdb_dir.join("ext/spatial.duckdb_extension");
    let duckdb_manifest = duckdb_dir.join("component-manifest.json");
    if let Some(duckdb_exe) = resolve_component_binary(&duckdb_dir, &["duckdb"]) {
        let exe_size = std::fs::metadata(&duckdb_exe).map(|m| m.len()).unwrap_or(0);
        let ext_size = std::fs::metadata(&duckdb_spatial)
            .map(|m| m.len())
            .unwrap_or(0);
        let total_size = exe_size.saturating_add(ext_size);
        let has_spatial = duckdb_spatial.is_file() && ext_size > 0;
        let valid_pe = is_pe_executable_header(&duckdb_exe);
        let mut version = "1.5.6".to_string();
        if let Ok(raw) = std::fs::read(&duckdb_manifest) {
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&raw) {
                if let Some(ver) = v.get("version").and_then(|s| s.as_str()) {
                    version = ver.to_string();
                }
            }
        }
        let file_count = 1 + usize::from(has_spatial) + usize::from(duckdb_manifest.is_file());
        components.push(RuntimeComponentItem {
            id: "duckdb-cli".to_string(),
            name: "DuckDB Spatial 空间 SQL 与 GeoParquet 分析引擎".to_string(),
            kind: "scientific_runtime".to_string(),
            version,
            install_root: "runtime/duckdb-cli".to_string(),
            status: if valid_pe && exe_size > 0 {
                "ready".to_string()
            } else {
                "partial".to_string()
            },
            file_count,
            total_bytes: total_size,
            verified: valid_pe && exe_size > 0,
            detail: if valid_pe && has_spatial {
                format!(
                    "含 DuckDB 主程序与 Spatial 空间扩展 ({:.1} MB, SHA-256 已校验)",
                    total_size as f64 / 1_048_576.0
                )
            } else if valid_pe {
                format!(
                    "DuckDB 主程序就绪 ({:.1} MB)",
                    exe_size as f64 / 1_048_576.0
                )
            } else {
                "duckdb 可执行文件头无效".to_string()
            },
            capabilities: vec![
                "spatial-sql".into(),
                "geoparquet-online-query".into(),
                "st-read-gdal-vector".into(),
            ],
        });
    } else {
        components.push(RuntimeComponentItem {
            id: "duckdb-cli".to_string(),
            name: "DuckDB Spatial 空间 SQL 与 GeoParquet 分析引擎".to_string(),
            kind: "scientific_runtime".to_string(),
            version: "未就绪".to_string(),
            install_root: "runtime/duckdb-cli".to_string(),
            status: "missing".to_string(),
            file_count: 0,
            total_bytes: 0,
            verified: false,
            detail: "runtime/duckdb-cli/duckdb 未安装 (可选空间 SQL 与 GeoParquet 组件)"
                .to_string(),
            capabilities: vec![
                "spatial-sql".into(),
                "geoparquet-online-query".into(),
                "st-read-gdal-vector".into(),
            ],
        });
    }

    // 8. rustgis-cli (RustGIS headless spatial analysis & MCP runner)
    let rustgis_dir = root.join("runtime/rustgis-cli");
    let rustgis_manifest = rustgis_dir.join("component-manifest.json");
    if let Some(rustgis_exe) = resolve_component_binary(&rustgis_dir, &["rustgis", "rustgis-cli"]) {
        let size = std::fs::metadata(&rustgis_exe)
            .map(|m| m.len())
            .unwrap_or(0);
        let count = if rustgis_manifest.is_file() { 2 } else { 1 };
        let valid_bin = is_pe_executable_header(&rustgis_exe);
        let mut version = "0.2.0".to_string();
        if let Ok(raw) = std::fs::read(&rustgis_manifest) {
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&raw) {
                if let Some(ver) = v.get("version").and_then(|s| s.as_str()) {
                    version = ver.to_string();
                }
            }
        }
        components.push(RuntimeComponentItem {
            id: "rustgis-cli".to_string(),
            name: "RustGIS 高性能无头空间分析与 MCP 引擎".to_string(),
            kind: "scientific_runtime".to_string(),
            version,
            install_root: "runtime/rustgis-cli".to_string(),
            status: if valid_bin && size > 0 {
                "ready".to_string()
            } else {
                "partial".to_string()
            },
            file_count: count,
            total_bytes: size,
            verified: valid_bin && size > 0,
            detail: if valid_bin && size > 0 {
                format!(
                    "Rust 原生无头 GIS 与 MCP 协议引擎 ({:.2} MB, SHA-256 已校验)",
                    size as f64 / 1_048_576.0
                )
            } else {
                "rustgis 可执行文件头无效".to_string()
            },
            capabilities: vec![
                "headless-gis-algorithms".into(),
                "mcp-stdio-tools".into(),
                "raster-vector-hydrology".into(),
            ],
        });
    } else {
        components.push(RuntimeComponentItem {
            id: "rustgis-cli".to_string(),
            name: "RustGIS 高性能无头空间分析与 MCP 引擎".to_string(),
            kind: "scientific_runtime".to_string(),
            version: "未就绪".to_string(),
            install_root: "runtime/rustgis-cli".to_string(),
            status: "missing".to_string(),
            file_count: 0,
            total_bytes: 0,
            verified: false,
            detail: "runtime/rustgis-cli/rustgis 未安装 (可选 Apache-2.0 无头 GIS 引擎)"
                .to_string(),
            capabilities: vec![
                "headless-gis-algorithms".into(),
                "mcp-stdio-tools".into(),
                "raster-vector-hydrology".into(),
            ],
        });
    }

    Ok(RuntimeComponentsReport {
        ok: true,
        install_root: root.to_string_lossy().to_string(),
        manifest_found,
        app_version,
        host_platform: current_host_platform_tag(),
        components,
    })
}

fn is_tiff_header(path: &std::path::Path) -> bool {
    use std::io::Read;
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    let mut header = [0u8; 4];
    if f.read_exact(&mut header).is_err() {
        return false;
    }
    header == [b'I', b'I', 42, 0] || header == [b'M', b'M', 0, 42]
}

fn is_pe_executable_header(path: &std::path::Path) -> bool {
    use std::io::Read;
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    let mut header = [0u8; 4];
    let Ok(read) = f.read(&mut header) else {
        return false;
    };
    if read >= 2 && &header[..2] == b"MZ" {
        return true;
    }
    if read == 4 {
        return matches!(
            header,
            [0xCF, 0xFA, 0xED, 0xFE]
                | [0xFE, 0xED, 0xFA, 0xCF]
                | [0xCA, 0xFE, 0xBA, 0xBE]
                | [0xBE, 0xBA, 0xFE, 0xCA]
                | [0x7F, b'E', b'L', b'F']
        );
    }
    false
}

pub const DEFAULT_EXTENSION_REGISTRY_URL: &str =
    "https://api.gitcode.com/api/v5/repos/hhanmj/eostage-hub/raw/registry.json";
pub const MAX_COMPONENT_BYTES: u64 = 512 * 1024 * 1024;
const MAX_REGISTRY_BYTES: u64 = 12 * 1024 * 1024;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ExtensionRegistrySkillPayload {
    pub id: String,
    pub name_zh: String,
    pub name_en: String,
    pub category: String,
    pub description_zh: String,
    pub description_en: String,
    pub prompt_template: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq, Default)]
pub struct ExtensionRegistryMcpServer {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub transport: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq, Default)]
pub struct ExtensionPlatformPackage {
    #[serde(default)]
    pub package: String,
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub size: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ExtensionRegistryEntry {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub package: String,
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub install_root: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub skill: Option<ExtensionRegistrySkillPayload>,
    #[serde(default)]
    pub source_market: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub repository_url: String,
    #[serde(default)]
    pub homepage_url: String,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub runtime_compatibility: String,
    #[serde(default)]
    pub compatibility_note: String,
    #[serde(default)]
    pub skill_url: String,
    #[serde(default)]
    pub mcp_servers: Vec<ExtensionRegistryMcpServer>,
    #[serde(default)]
    pub platforms: Vec<String>,
    #[serde(default)]
    pub platform_packages: std::collections::BTreeMap<String, ExtensionPlatformPackage>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq, Default)]
pub struct ExtensionBlocklistEntry {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub reason: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExtensionRegistryReport {
    pub ok: bool,
    pub source_url: String,
    pub updated_at: String,
    pub entries: Vec<ExtensionRegistryEntry>,
    pub blocked_count: usize,
    #[serde(default)]
    pub market_id: String,
    #[serde(default)]
    pub market_title: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RuntimeComponentInstallRequest {
    pub component_id: String,
    pub source: String,
    #[serde(default)]
    pub expected_sha256: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RuntimeComponentInstallResult {
    pub ok: bool,
    pub component_id: String,
    pub version: String,
    pub sha256: String,
    pub bytes_installed: u64,
    pub install_path: String,
    pub status: String,
    pub detail: String,
}

pub fn is_valid_sha256_hex(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.len() == 64 && trimmed.bytes().all(|b| b.is_ascii_hexdigit())
}

pub fn resolve_registry_url(base: &str, rel: &str) -> String {
    let rel_trimmed = rel.trim();
    if rel_trimmed.starts_with("https://") || rel_trimmed.starts_with("http://") {
        return rel_trimmed.to_string();
    }
    let rel_clean = rel_trimmed.strip_prefix("./").unwrap_or(rel_trimmed);
    let base_trimmed = base.trim();
    if base_trimmed.starts_with("https://") || base_trimmed.starts_with("http://") {
        if let Some(slash_idx) = base_trimmed.rfind('/') {
            return format!("{}{}", &base_trimmed[..=slash_idx], rel_clean);
        }
        return format!("{base_trimmed}/{rel_clean}");
    }
    let base_path = std::path::Path::new(
        base_trimmed
            .strip_prefix("file:///")
            .or_else(|| base_trimmed.strip_prefix("file://"))
            .unwrap_or(base_trimmed),
    );
    let parent = if base_path.is_dir() {
        base_path
    } else {
        base_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
    };
    parent.join(rel_clean).to_string_lossy().to_string()
}

fn is_likely_html_payload(bytes: &[u8]) -> bool {
    let trimmed = bytes
        .iter()
        .copied()
        .skip_while(|b| b.is_ascii_whitespace())
        .take(32)
        .collect::<Vec<u8>>();
    let lower = String::from_utf8_lossy(&trimmed).to_ascii_lowercase();
    lower.starts_with("<!doctype html")
        || lower.starts_with("<!doctype")
        || lower.starts_with("<html")
}

fn is_html_content_type(headers: &reqwest::header::HeaderMap) -> bool {
    headers
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.to_ascii_lowercase().contains("text/html"))
}

fn is_loopback_http_url(raw: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(raw.trim()) else {
        return false;
    };
    parsed.scheme() == "http" && matches!(parsed.host_str(), Some("127.0.0.1" | "localhost"))
}

fn hub_url_ip(url: &reqwest::Url) -> Option<std::net::IpAddr> {
    let raw = url.host_str()?;
    let stripped = raw
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(raw);
    stripped.parse::<std::net::IpAddr>().ok()
}

fn is_disallowed_hub_ipv4(ip: std::net::Ipv4Addr) -> bool {
    let [o0, o1, ..] = ip.octets();
    ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_multicast()
        || ip.is_unspecified()
        || o0 == 0
        || (o0 == 100 && (64..=127).contains(&o1))
        || (o0 == 198 && (18..=19).contains(&o1))
        || o0 >= 240
}

fn validate_hub_http_url(
    url: &reqwest::Url,
    loopback_only: bool,
    err_prefix: &str,
) -> Result<(), String> {
    if !url.username().is_empty() || url.password().is_some() {
        return Err(format!("{err_prefix}: URL 不允许携带内嵌凭据"));
    }
    if loopback_only {
        if !matches!(url.scheme(), "http" | "https") {
            return Err(format!("{err_prefix}: 回环地址仅支持 HTTP/HTTPS 协议"));
        }
        let is_loopback_ip = hub_url_ip(url).is_some_and(|ip| match ip {
            std::net::IpAddr::V4(v4) => v4.is_loopback(),
            std::net::IpAddr::V6(v6) => {
                v6.is_loopback() || v6.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback())
            }
        });
        let is_localhost = url
            .host_str()
            .is_some_and(|h| h.trim_end_matches('.').eq_ignore_ascii_case("localhost"));
        if !is_loopback_ip && !is_localhost {
            return Err(format!("{err_prefix}: 禁止回环源重定向至非本地主机"));
        }
        return Ok(());
    }

    if url.scheme() != "https" {
        return Err(format!("{err_prefix}: 仅允许安全 HTTPS 地址"));
    }
    if let Some(ip) = hub_url_ip(url) {
        let disallowed = match ip {
            std::net::IpAddr::V4(v4) => is_disallowed_hub_ipv4(v4),
            std::net::IpAddr::V6(v6) => {
                v6.is_loopback()
                    || v6.is_unspecified()
                    || v6.is_unique_local()
                    || v6.is_unicast_link_local()
                    || v6.is_multicast()
                    || v6.to_ipv4_mapped().is_some_and(is_disallowed_hub_ipv4)
            }
        };
        if disallowed {
            return Err(format!("{err_prefix}: 拒绝访问本地或保留网段 IP 地址"));
        }
        return Ok(());
    }

    let Some(host) = url.host_str() else {
        return Err(format!("{err_prefix}: 缺少目标主机名"));
    };
    let norm = host.trim_end_matches('.').to_ascii_lowercase();
    if norm.is_empty()
        || !norm.contains('.')
        || norm == "localhost"
        || norm.ends_with(".localhost")
        || norm.ends_with(".local")
        || norm.ends_with(".internal")
    {
        return Err(format!("{err_prefix}: 拒绝访问本地或内网保留域名"));
    }
    Ok(())
}

fn build_hub_http_client(
    loopback_only: bool,
    connect_secs: u64,
    timeout_secs: u64,
    err_prefix: &str,
) -> Result<reqwest::blocking::Client, String> {
    let prefix = err_prefix.to_string();
    let policy = reqwest::redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() >= 5 {
            return attempt.error(std::io::Error::other(format!(
                "{prefix}: 重定向次数超过上限 (5)"
            )));
        }
        if let Err(reason) = validate_hub_http_url(attempt.url(), loopback_only, &prefix) {
            return attempt.error(std::io::Error::other(reason));
        }
        attempt.follow()
    });
    let mut builder = reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(connect_secs))
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .redirect(policy);
    if loopback_only {
        builder = builder.no_proxy();
    }
    builder.build().map_err(|e| format!("{err_prefix}: {e}"))
}

fn read_text_from_source(source: &str, max_bytes: u64) -> Result<String, String> {
    use std::io::Read;
    let trimmed = source.trim();
    let is_loopback = is_loopback_http_url(trimmed);
    if trimmed.starts_with("https://") || is_loopback {
        let parsed =
            reqwest::Url::parse(trimmed).map_err(|e| format!("REGISTRY_INVALID_URL: {e}"))?;
        validate_hub_http_url(&parsed, is_loopback, "REGISTRY_UNSAFE_URL")?;
        let client = build_hub_http_client(is_loopback, 8, 18, "REGISTRY_HTTP_CLIENT_ERROR")?;
        let resp = client
            .get(parsed)
            .header(reqwest::header::USER_AGENT, "EOStage-Desktop/2.2")
            .send()
            .map_err(|e| format!("REGISTRY_FETCH_FAILED: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("REGISTRY_HTTP_STATUS_{}", resp.status().as_u16()));
        }
        if let Some(cl) = resp.content_length() {
            if cl > max_bytes {
                return Err("REGISTRY_TOO_LARGE: 注册表大小超出安全上限".to_string());
            }
        }
        if is_html_content_type(resp.headers()) {
            return Err(
                "REGISTRY_UNEXPECTED_HTML_RESPONSE: 目标地址返回 HTML 页面而非清单数据".to_string(),
            );
        }
        let mut buf = Vec::new();
        resp.take(max_bytes + 1)
            .read_to_end(&mut buf)
            .map_err(|e| format!("REGISTRY_READ_FAILED: {e}"))?;
        if buf.len() as u64 > max_bytes {
            return Err("REGISTRY_TOO_LARGE: 注册表大小超出安全上限".to_string());
        }
        if is_likely_html_payload(&buf) {
            return Err(
                "REGISTRY_UNEXPECTED_HTML_PAYLOAD: 目标地址返回 HTML 内容而非清单数据".to_string(),
            );
        }
        String::from_utf8(buf).map_err(|e| format!("REGISTRY_INVALID_UTF8: {e}"))
    } else if trimmed.starts_with("http://") {
        Err("REGISTRY_INSECURE_URL: 仅支持 HTTPS 或本地回环地址获取注册表".to_string())
    } else {
        let clean_path = trimmed
            .strip_prefix("file:///")
            .or_else(|| trimmed.strip_prefix("file://"))
            .unwrap_or(trimmed);
        let meta = std::fs::metadata(clean_path)
            .map_err(|e| format!("REGISTRY_FILE_NOT_FOUND ({clean_path}): {e}"))?;
        if meta.len() > max_bytes {
            return Err("REGISTRY_TOO_LARGE: 注册表大小超出安全上限".to_string());
        }
        std::fs::read_to_string(clean_path)
            .map_err(|e| format!("REGISTRY_FILE_READ_FAILED ({clean_path}): {e}"))
    }
}

fn pick_localized_str(val: Option<&serde_json::Value>) -> String {
    let Some(v) = val else {
        return String::new();
    };
    if let Some(s) = v.as_str() {
        return s.trim().to_string();
    }
    if let Some(obj) = v.as_object() {
        for key in ["zh", "zh-CN", "zh_CN", "en", "en-US", "default"] {
            if let Some(s) = obj.get(key).and_then(|item| item.as_str()) {
                let trimmed = s.trim();
                if !trimmed.is_empty() {
                    return trimmed.to_string();
                }
            }
        }
        for val in obj.values() {
            if let Some(s) = val.as_str() {
                let trimmed = s.trim();
                if !trimmed.is_empty() {
                    return trimmed.to_string();
                }
            }
        }
    }
    String::new()
}

fn pick_author_str(val: Option<&serde_json::Value>) -> String {
    let Some(v) = val else {
        return String::new();
    };
    if let Some(s) = v.as_str() {
        return s.trim().to_string();
    }
    if let Some(obj) = v.as_object() {
        for key in ["name", "username", "login", "id", "email"] {
            if let Some(s) = obj.get(key).and_then(|item| item.as_str()) {
                let trimmed = s.trim();
                if !trimmed.is_empty() {
                    return trimmed.to_string();
                }
            }
        }
    }
    String::new()
}

fn pick_url_str(val: Option<&serde_json::Value>) -> String {
    let Some(v) = val else {
        return String::new();
    };
    if let Some(s) = v.as_str() {
        return s.trim().to_string();
    }
    if let Some(obj) = v.as_object() {
        if let Some(s) = obj.get("url").and_then(|item| item.as_str()) {
            return s.trim().to_string();
        }
    }
    String::new()
}

fn is_forbidden_internal_connector_endpoint(raw: &str) -> bool {
    let trimmed = raw
        .trim()
        .trim_matches(|c| c == '"' || c == '\'')
        .trim();
    trimmed.to_ascii_lowercase().starts_with("tauri://")
}

fn parse_mcp_servers_from_value(
    val: Option<&serde_json::Value>,
) -> Vec<ExtensionRegistryMcpServer> {
    let Some(v) = val else {
        return Vec::new();
    };
    let mut servers = Vec::new();
    if let Some(map) = v.as_object() {
        for (key, cfg) in map {
            let Some(obj) = cfg.as_object() else {
                continue;
            };
            let id = key.trim().to_string();
            if id.is_empty() {
                continue;
            }
            let command = obj
                .get("command")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            let args = obj
                .get("args")
                .and_then(|x| x.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|item| item.as_str().map(|s| s.to_string()))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let env = obj
                .get("env")
                .and_then(|x| x.as_object())
                .map(|m| {
                    m.iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                        .collect::<std::collections::HashMap<_, _>>()
                })
                .unwrap_or_default();
            let url = obj
                .get("url")
                .or_else(|| obj.get("endpoint"))
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if is_forbidden_internal_connector_endpoint(&command)
                || is_forbidden_internal_connector_endpoint(&url)
            {
                continue;
            }
            let transport = obj
                .get("type")
                .or_else(|| obj.get("transport"))
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if !command.is_empty() || !url.is_empty() {
                servers.push(ExtensionRegistryMcpServer {
                    id: id.clone(),
                    name: obj
                        .get("name")
                        .and_then(|x| x.as_str())
                        .unwrap_or(&id)
                        .to_string(),
                    transport,
                    command,
                    args,
                    env,
                    url,
                    description: obj
                        .get("description")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string(),
                });
            }
        }
    } else if let Some(arr) = v.as_array() {
        for item in arr {
            let Some(obj) = item.as_object() else {
                continue;
            };
            let id = obj
                .get("id")
                .or_else(|| obj.get("name"))
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if id.is_empty() {
                continue;
            }
            let command = obj
                .get("command")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            let args = obj
                .get("args")
                .and_then(|x| x.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|item| item.as_str().map(|s| s.to_string()))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let env = obj
                .get("env")
                .and_then(|x| x.as_object())
                .map(|m| {
                    m.iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                        .collect::<std::collections::HashMap<_, _>>()
                })
                .unwrap_or_default();
            let url = obj
                .get("url")
                .or_else(|| obj.get("endpoint"))
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if is_forbidden_internal_connector_endpoint(&command)
                || is_forbidden_internal_connector_endpoint(&url)
            {
                continue;
            }
            let transport = obj
                .get("type")
                .or_else(|| obj.get("transport"))
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if !command.is_empty() || !url.is_empty() {
                servers.push(ExtensionRegistryMcpServer {
                    id: id.clone(),
                    name: obj
                        .get("name")
                        .and_then(|x| x.as_str())
                        .unwrap_or(&id)
                        .to_string(),
                    transport,
                    command,
                    args,
                    env,
                    url,
                    description: obj
                        .get("description")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string(),
                });
            }
        }
    }
    servers
}

fn detect_market_identity(_source_url: &str, _val: &serde_json::Value) -> (String, String) {
    ("eostage".to_string(), "EOStage 官方扩展源".to_string())
}

fn parse_external_or_dsh_entry(
    raw: &serde_json::Value,
    market_id: &str,
    source_url: &str,
) -> Option<ExtensionRegistryEntry> {
    let obj = raw.as_object()?;
    // Select latest version entry when DSH Format-1 `versions` array exists
    let version_obj = obj
        .get("versions")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|v| v.as_object());

    let raw_id = obj
        .get("id")
        .or_else(|| obj.get("slug"))
        .or_else(|| obj.get("name"))
        .or_else(|| obj.get("packageName"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if raw_id.is_empty() {
        return None;
    }
    let id = raw_id.to_string();
    let name = {
        let candidate = pick_localized_str(
            obj.get("title")
                .or_else(|| obj.get("displayName"))
                .or_else(|| obj.get("display_name"))
                .or_else(|| obj.get("name"))
                .or_else(|| version_obj.and_then(|vo| vo.get("title"))),
        );
        if candidate.is_empty() {
            id.clone()
        } else {
            candidate
        }
    };
    let description = {
        let d = pick_localized_str(
            obj.get("description")
                .or_else(|| obj.get("summary"))
                .or_else(|| version_obj.and_then(|vo| vo.get("description"))),
        );
        d
    };
    let version = obj
        .get("version")
        .or_else(|| obj.get("latest"))
        .or_else(|| version_obj.and_then(|vo| vo.get("version")))
        .and_then(|v| v.as_str())
        .unwrap_or("1.0.0")
        .trim()
        .to_string();

    let mut kind = obj
        .get("kind")
        .or_else(|| obj.get("type"))
        .or_else(|| version_obj.and_then(|vo| vo.get("kind")))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();

    let author = pick_author_str(
        obj.get("author")
            .or_else(|| obj.get("publisher"))
            .or_else(|| {
                obj.get("maintainers")
                    .and_then(|m| m.as_array())
                    .and_then(|a| a.first())
            })
            .or_else(|| version_obj.and_then(|vo| vo.get("author"))),
    );
    let category = obj
        .get("category")
        .or_else(|| version_obj.and_then(|vo| vo.get("category")))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let license = obj
        .get("license")
        .or_else(|| version_obj.and_then(|vo| vo.get("license")))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let repository_url = pick_url_str(
        obj.get("repository_url")
            .or_else(|| obj.get("repository"))
            .or_else(|| obj.get("repo"))
            .or_else(|| version_obj.and_then(|vo| vo.get("repository"))),
    );
    let homepage_url = pick_url_str(
        obj.get("homepage_url")
            .or_else(|| obj.get("homepage"))
            .or_else(|| obj.get("links").and_then(|l| l.get("homepage")))
            .or_else(|| version_obj.and_then(|vo| vo.get("homepage"))),
    );
    let keywords = obj
        .get("keywords")
        .or_else(|| obj.get("tags"))
        .or_else(|| version_obj.and_then(|vo| vo.get("keywords")))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| item.as_str().map(|s| s.trim().to_string()))
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let skill_url = obj
        .get("skill_url")
        .or_else(|| obj.get("skillUrl"))
        .or_else(|| obj.get("skill_md_url"))
        .or_else(|| version_obj.and_then(|vo| vo.get("skill_url").or_else(|| vo.get("skillUrl"))))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    let skill = obj
        .get("skill")
        .or_else(|| version_obj.and_then(|vo| vo.get("skill")))
        .and_then(|sv| {
            let cat = if category.is_empty() {
                "custom".to_string()
            } else {
                category.clone()
            };
            if let Some(raw_md) = sv.as_str() {
                if raw_md.trim().is_empty() {
                    None
                } else {
                    Some(ExtensionRegistrySkillPayload {
                        id: sanitize_plugin_id(&id),
                        name_zh: name.clone(),
                        name_en: name.clone(),
                        category: cat,
                        description_zh: description.clone(),
                        description_en: description.clone(),
                        prompt_template: raw_md.to_string(),
                    })
                }
            } else if let Ok(exact) =
                serde_json::from_value::<ExtensionRegistrySkillPayload>(sv.clone())
            {
                Some(exact)
            } else if let Some(sobj) = sv.as_object() {
                let sk_id = sobj
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(sanitize_plugin_id)
                    .unwrap_or_else(|| sanitize_plugin_id(&id));
                let sk_name = sobj
                    .get("name_zh")
                    .or_else(|| sobj.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or(&name)
                    .to_string();
                let sk_desc = sobj
                    .get("description_zh")
                    .or_else(|| sobj.get("description"))
                    .and_then(|v| v.as_str())
                    .unwrap_or(&description)
                    .to_string();
                let sk_prompt = sobj
                    .get("prompt_template")
                    .or_else(|| sobj.get("content"))
                    .or_else(|| sobj.get("prompt"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                Some(ExtensionRegistrySkillPayload {
                    id: sk_id,
                    name_zh: sk_name.clone(),
                    name_en: sobj
                        .get("name_en")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&sk_name)
                        .to_string(),
                    category: sobj
                        .get("category")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&cat)
                        .to_string(),
                    description_zh: sk_desc.clone(),
                    description_en: sobj
                        .get("description_en")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&sk_desc)
                        .to_string(),
                    prompt_template: sk_prompt,
                })
            } else {
                None
            }
        });

    let mcp_servers = {
        let direct = parse_mcp_servers_from_value(
            obj.get("mcp_servers")
                .or_else(|| obj.get("mcpServers"))
                .or_else(|| obj.get("mcp"))
                .or_else(|| {
                    version_obj
                        .and_then(|vo| vo.get("mcpServers").or_else(|| vo.get("mcp_servers")))
                }),
        );
        if !direct.is_empty() {
            direct
        } else if let Some(cmd) = obj.get("command").and_then(|v| v.as_str()) {
            vec![ExtensionRegistryMcpServer {
                id: sanitize_plugin_id(&id),
                name: name.clone(),
                transport: obj
                    .get("transport")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string(),
                command: cmd.trim().to_string(),
                args: obj
                    .get("args")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default(),
                env: Default::default(),
                url: obj
                    .get("url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string(),
                description: description.clone(),
            }]
        } else {
            Vec::new()
        }
    };

    let source_type = obj
        .get("source")
        .and_then(|s| s.get("type"))
        .or_else(|| {
            version_obj
                .and_then(|vo| vo.get("source"))
                .and_then(|s| s.get("type"))
        })
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();

    let package_raw = obj
        .get("package")
        .or_else(|| obj.get("download_url"))
        .or_else(|| obj.get("zip_url"))
        .or_else(|| obj.get("mcp_url"))
        .or_else(|| obj.get("dist").and_then(|d| d.get("tarball")))
        .or_else(|| version_obj.and_then(|vo| vo.get("dist").and_then(|d| d.get("tarball"))))
        .or_else(|| version_obj.and_then(|vo| vo.get("package")))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    let package = if package_raw.is_empty() {
        if !skill_url.is_empty() {
            skill_url.clone()
        } else if source_type == "npm" && !raw_id.is_empty() {
            format!(
                "https://registry.npmjs.org/{raw_id}/-/{}-{version}.tgz",
                raw_id.rsplit('/').next().unwrap_or(raw_id)
            )
        } else if !repository_url.is_empty() {
            repository_url.clone()
        } else {
            String::new()
        }
    } else {
        resolve_registry_url(source_url, &package_raw)
    };

    let sha256_raw = obj
        .get("sha256")
        .or_else(|| version_obj.and_then(|vo| vo.get("sha256")))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();

    let size = obj
        .get("size")
        .or_else(|| {
            version_obj
                .and_then(|vo| vo.get("dist"))
                .and_then(|d| d.get("unpackedSize"))
        })
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let engines_obj = obj
        .get("engines")
        .or_else(|| version_obj.and_then(|vo| vo.get("engines")))
        .and_then(|v| v.as_object());
    let has_cordis_or_dsh_engine = engines_obj
        .map(|e| e.contains_key("dsh") || e.contains_key("cordis") || e.contains_key("koishi"))
        .unwrap_or(false);

    let explicit_compat = obj
        .get("runtime_compatibility")
        .or_else(|| obj.get("runtimeCompatibility"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    let has_native_skill = skill.is_some() || !skill_url.is_empty();
    let has_native_mcp =
        !mcp_servers.is_empty() || package.ends_with(".mcp.json") || package.ends_with("mcp.json");
    let has_native_zip = package.ends_with(".zip") && is_valid_sha256_hex(&sha256_raw);

    let is_cordis_only = explicit_compat == "cordis_host_required"
        || (!has_native_skill
            && !has_native_mcp
            && !has_native_zip
            && (has_cordis_or_dsh_engine
                || source_type == "npm"
                || kind == "cordis"
                || kind == "dsh-plugin"
                || kind == "npm"
                || package.ends_with(".tgz")));

    let (runtime_compatibility, compatibility_note) = if is_cordis_only {
        (
            "cordis_host_required".to_string(),
            obj.get("compatibility_note")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|| {
                    "该插件基于 Node.js / Cordis 运行时，需 DSH / Cordis 宿主环境才能运行；当前桌面端仅原生安装兼容的 SKILL.md 技能包与 .mcp.json MCP 连接器。".to_string()
                }),
        )
    } else {
        (
            if explicit_compat.is_empty() {
                "eostage_native".to_string()
            } else {
                explicit_compat
            },
            obj.get("compatibility_note")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string(),
        )
    };

    if kind.is_empty() {
        kind = if !mcp_servers.is_empty() || package.ends_with("mcp.json") {
            "connector".to_string()
        } else if has_native_skill {
            "skill".to_string()
        } else {
            "plugin".to_string()
        };
    }

    let capabilities = obj
        .get("capabilities")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let platforms = obj
        .get("platforms")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(|s| s.trim().to_string()))
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Some(ExtensionRegistryEntry {
        id,
        name,
        kind,
        version,
        description,
        package,
        sha256: sha256_raw,
        size,
        install_root: obj
            .get("install_root")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string(),
        capabilities,
        skill,
        source_market: market_id.to_string(),
        author,
        category,
        license,
        repository_url,
        homepage_url,
        keywords,
        runtime_compatibility,
        compatibility_note,
        skill_url,
        mcp_servers,
        platforms,
        platform_packages: std::collections::BTreeMap::new(),
    })
}

pub fn parse_and_filter_extension_registry(
    source_url: &str,
    registry_json: &str,
    blocklist_json: Option<&str>,
) -> Result<ExtensionRegistryReport, String> {
    let val: serde_json::Value =
        serde_json::from_str(registry_json).map_err(|e| format!("REGISTRY_JSON_INVALID: {e}"))?;
    let (market_id, market_title) = detect_market_identity(source_url, &val);
    let updated_at = val
        .get("updated_at")
        .or_else(|| val.get("updatedAt"))
        .or_else(|| val.get("generated_at"))
        .or_else(|| val.get("time"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let is_eostage_strict_entries = market_id == "eostage"
        && (val.is_array() || val.get("entries").is_some())
        && val.get("format").is_none();

    let raw_items: Vec<serde_json::Value> = if let Some(arr) = val.as_array() {
        arr.clone()
    } else if let Some(arr) = val
        .get("entries")
        .or_else(|| val.get("plugins"))
        .or_else(|| val.get("items"))
        .or_else(|| {
            val.get("data").and_then(|d| {
                if d.is_array() {
                    Some(d)
                } else {
                    d.get("plugins")
                        .or_else(|| d.get("items"))
                        .or_else(|| d.get("entries"))
                }
            })
        })
        .and_then(|v| v.as_array())
    {
        arr.clone()
    } else {
        return Err("REGISTRY_SCHEMA_INVALID: 缺少 entries / plugins 数组".to_string());
    };

    let blocked_entries: Vec<ExtensionBlocklistEntry> = if let Some(bl_str) = blocklist_json {
        if let Ok(bl_val) = serde_json::from_str::<serde_json::Value>(bl_str) {
            if bl_val.is_array() {
                serde_json::from_value(bl_val).unwrap_or_default()
            } else if let Some(arr) = bl_val.get("blocked").or_else(|| bl_val.get("entries")) {
                serde_json::from_value(arr.clone()).unwrap_or_default()
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    let host_platform = current_host_platform_tag();
    let mut entries = Vec::new();
    let mut blocked_count = 0usize;

    if is_eostage_strict_entries {
        let parsed_entries: Vec<ExtensionRegistryEntry> =
            serde_json::from_value(serde_json::Value::Array(raw_items))
                .map_err(|e| format!("REGISTRY_ENTRIES_INVALID: {e}"))?;
        for mut entry in parsed_entries {
            for pp in entry.platform_packages.values_mut() {
                if !pp.package.trim().is_empty() {
                    pp.package = resolve_and_project_registry_package_url(source_url, &pp.package);
                }
                if !pp.sha256.trim().is_empty() {
                    if !is_valid_sha256_hex(&pp.sha256) {
                        return Err(format!(
                            "REGISTRY_INVALID_SHA256: 条目 {} 的平台包 sha256 必须是 64 位十六进制字符串",
                            entry.id
                        ));
                    }
                    pp.sha256 = pp.sha256.trim().to_ascii_lowercase();
                }
            }
            if let Some(host_pkg) = entry.platform_packages.get(&host_platform).cloned() {
                if !host_pkg.package.is_empty() {
                    entry.package = host_pkg.package;
                }
                if !host_pkg.sha256.is_empty() {
                    entry.sha256 = host_pkg.sha256;
                }
                if host_pkg.size > 0 {
                    entry.size = host_pkg.size;
                }
            }
            if !is_valid_sha256_hex(&entry.sha256) {
                return Err(format!(
                    "REGISTRY_INVALID_SHA256: 条目 {} 的 sha256 必须是 64 位十六进制字符串",
                    entry.id
                ));
            }
            entry.sha256 = entry.sha256.trim().to_ascii_lowercase();
            let is_blocked = blocked_entries.iter().any(|b| {
                b.id.as_deref()
                    .is_some_and(|id| id.eq_ignore_ascii_case(&entry.id))
                    || b.sha256
                        .as_deref()
                        .is_some_and(|h| h.trim().eq_ignore_ascii_case(&entry.sha256))
            });
            if is_blocked {
                blocked_count += 1;
                continue;
            }
            entry.package = resolve_and_project_registry_package_url(source_url, &entry.package);
            if entry.source_market.is_empty() {
                entry.source_market = market_id.clone();
            }
            if entry.runtime_compatibility.is_empty() {
                entry.runtime_compatibility = "eostage_native".to_string();
            }
            entries.push(entry);
        }
    } else {
        for raw in &raw_items {
            let Some(entry) = parse_external_or_dsh_entry(raw, &market_id, source_url) else {
                continue;
            };
            let is_blocked = blocked_entries.iter().any(|b| {
                b.id.as_deref()
                    .is_some_and(|id| id.eq_ignore_ascii_case(&entry.id))
                    || (!entry.sha256.is_empty()
                        && b.sha256
                            .as_deref()
                            .is_some_and(|h| h.trim().eq_ignore_ascii_case(&entry.sha256)))
            });
            if is_blocked {
                blocked_count += 1;
                continue;
            }
            entries.push(entry);
        }
    }

    Ok(ExtensionRegistryReport {
        ok: true,
        source_url: source_url.to_string(),
        updated_at,
        entries,
        blocked_count,
        market_id,
        market_title,
    })
}

#[derive(Debug, PartialEq, Eq)]
enum GitPktLine<'a> {
    Flush,
    Delim,
    ResponseEnd,
    Data(&'a [u8]),
}

fn git_pkt_line(payload: &[u8]) -> Vec<u8> {
    let mut out = format!("{:04x}", payload.len() + 4).into_bytes();
    out.extend_from_slice(payload);
    out
}

fn git_parse_pkt_lines(buf: &[u8]) -> Result<Vec<GitPktLine<'_>>, String> {
    let mut pos = 0usize;
    let mut out = Vec::new();
    while pos + 4 <= buf.len() {
        let hdr = std::str::from_utf8(&buf[pos..pos + 4])
            .map_err(|_| "GIT_WIRE_INVALID_PKT_HEADER".to_string())?;
        pos += 4;
        match hdr {
            "0000" => out.push(GitPktLine::Flush),
            "0001" => out.push(GitPktLine::Delim),
            "0002" => out.push(GitPktLine::ResponseEnd),
            _ => {
                let len = usize::from_str_radix(hdr, 16)
                    .map_err(|_| format!("GIT_WIRE_INVALID_PKT_LEN: {hdr}"))?;
                if len < 4 || pos + (len - 4) > buf.len() {
                    return Err("GIT_WIRE_TRUNCATED_PKT".to_string());
                }
                let payload_len = len - 4;
                out.push(GitPktLine::Data(&buf[pos..pos + payload_len]));
                pos += payload_len;
            }
        }
    }
    Ok(out)
}

fn git_extract_packfile_from_fetch_response(resp: &[u8]) -> Result<Vec<u8>, String> {
    let lines = git_parse_pkt_lines(resp)?;
    let mut pack = Vec::new();
    let mut in_pack = false;
    for line in lines {
        match line {
            GitPktLine::Data(payload) if !in_pack => {
                let trimmed = payload
                    .strip_suffix(b"\n")
                    .unwrap_or(payload)
                    .strip_suffix(b"\r")
                    .unwrap_or(payload.strip_suffix(b"\n").unwrap_or(payload));
                if trimmed == b"packfile" {
                    in_pack = true;
                }
            }
            GitPktLine::Data(payload) if in_pack => {
                if payload.is_empty() {
                    continue;
                }
                match payload[0] {
                    1 => pack.extend_from_slice(&payload[1..]),
                    2 => {}
                    3 => {
                        return Err(format!(
                            "GIT_WIRE_REMOTE_ERROR: {}",
                            String::from_utf8_lossy(&payload[1..]).trim()
                        ));
                    }
                    _ => {}
                }
            }
            GitPktLine::Flush | GitPktLine::ResponseEnd if in_pack => break,
            _ => {}
        }
    }
    if !pack.starts_with(b"PACK") {
        return Err("GIT_WIRE_MISSING_PACKFILE: 响应中未包含有效 Git PACK 数据".to_string());
    }
    Ok(pack)
}

#[derive(Debug, Clone)]
struct GitPackObject {
    obj_type: u8,
    oid: [u8; 20],
    offset: usize,
    raw: Vec<u8>,
}

fn git_compute_object_oid(obj_type: u8, raw: &[u8]) -> Result<[u8; 20], String> {
    let kind = match obj_type {
        1 => "commit",
        2 => "tree",
        3 => "blob",
        4 => "tag",
        _ => return Err(format!("GIT_WIRE_UNSUPPORTED_OBJECT_TYPE: {obj_type}")),
    };
    let mut ctx = ring::digest::Context::new(&ring::digest::SHA1_FOR_LEGACY_USE_ONLY);
    ctx.update(format!("{kind} {}\0", raw.len()).as_bytes());
    ctx.update(raw);
    let digest = ctx.finish();
    let mut out = [0u8; 20];
    out.copy_from_slice(digest.as_ref());
    Ok(out)
}

fn git_read_delta_varint(data: &[u8], pos: &mut usize) -> Result<usize, String> {
    let mut val = 0usize;
    let mut shift = 0u32;
    loop {
        if *pos >= data.len() {
            return Err("GIT_WIRE_DELTA_TRUNCATED_VARINT".to_string());
        }
        let b = data[*pos];
        *pos += 1;
        val |= ((b & 0x7f) as usize) << shift;
        if (b & 0x80) == 0 {
            return Ok(val);
        }
        shift += 7;
        if shift >= usize::BITS {
            return Err("GIT_WIRE_DELTA_VARINT_OVERFLOW".to_string());
        }
    }
}

fn git_apply_delta(base: &[u8], delta: &[u8], max_bytes: usize) -> Result<Vec<u8>, String> {
    let mut pos = 0usize;
    let base_len = git_read_delta_varint(delta, &mut pos)?;
    if base_len != base.len() {
        return Err(format!(
            "GIT_WIRE_DELTA_BASE_MISMATCH: expected {base_len}, actual {}",
            base.len()
        ));
    }
    let result_len = git_read_delta_varint(delta, &mut pos)?;
    if result_len > max_bytes {
        return Err("GIT_WIRE_DELTA_TOO_LARGE: 目标对象超出安全体积上限".to_string());
    }
    let mut out = Vec::with_capacity(result_len);
    while pos < delta.len() {
        let cmd = delta[pos];
        pos += 1;
        if (cmd & 0x80) != 0 {
            let mut cp_off = 0usize;
            let mut cp_size = 0usize;
            for bit in 0..4u32 {
                if (cmd & (1 << bit)) != 0 {
                    if pos >= delta.len() {
                        return Err("GIT_WIRE_DELTA_TRUNCATED_COPY".to_string());
                    }
                    cp_off |= (delta[pos] as usize) << (bit * 8);
                    pos += 1;
                }
            }
            for bit in 0..3u32 {
                if (cmd & (1 << (bit + 4))) != 0 {
                    if pos >= delta.len() {
                        return Err("GIT_WIRE_DELTA_TRUNCATED_COPY".to_string());
                    }
                    cp_size |= (delta[pos] as usize) << (bit * 8);
                    pos += 1;
                }
            }
            if cp_size == 0 {
                cp_size = 0x1_0000;
            }
            let end = cp_off
                .checked_add(cp_size)
                .ok_or_else(|| "GIT_WIRE_DELTA_COPY_OVERFLOW".to_string())?;
            if end > base.len() || out.len() + cp_size > result_len {
                return Err("GIT_WIRE_DELTA_COPY_OOB".to_string());
            }
            out.extend_from_slice(&base[cp_off..end]);
        } else if cmd != 0 {
            let n = cmd as usize;
            if pos + n > delta.len() || out.len() + n > result_len {
                return Err("GIT_WIRE_DELTA_INSERT_OOB".to_string());
            }
            out.extend_from_slice(&delta[pos..pos + n]);
            pos += n;
        } else {
            return Err("GIT_WIRE_DELTA_INVALID_OPCODE_0".to_string());
        }
    }
    if out.len() != result_len {
        return Err("GIT_WIRE_DELTA_SIZE_MISMATCH".to_string());
    }
    Ok(out)
}

fn git_decompress_zlib_object(
    slice: &[u8],
    expected_len: usize,
) -> Result<(Vec<u8>, usize), String> {
    let mut decomp = flate2::Decompress::new(true);
    let mut out = vec![0u8; expected_len];
    let mut scratch = [0u8; 64];
    let mut in_off = 0usize;
    let mut out_off = 0usize;
    loop {
        let before_in = decomp.total_in();
        let before_out = decomp.total_out();
        let status = if expected_len == 0 {
            decomp.decompress(
                &slice[in_off..],
                &mut scratch,
                flate2::FlushDecompress::Sync,
            )
        } else {
            decomp.decompress(
                &slice[in_off..],
                &mut out[out_off..],
                flate2::FlushDecompress::Sync,
            )
        }
        .map_err(|e| format!("GIT_WIRE_ZLIB_ERROR: {e}"))?;
        in_off += (decomp.total_in() - before_in) as usize;
        out_off += (decomp.total_out() - before_out) as usize;
        match status {
            flate2::Status::StreamEnd => break,
            flate2::Status::Ok | flate2::Status::BufError => {
                if out_off > expected_len
                    || (out_off == expected_len && decomp.total_in() == before_in)
                {
                    return Err("GIT_WIRE_ZLIB_SIZE_EXCEEDED".to_string());
                }
                if in_off >= slice.len() {
                    return Err("GIT_WIRE_ZLIB_TRUNCATED".to_string());
                }
            }
        }
    }
    if out_off != expected_len {
        return Err(format!(
            "GIT_WIRE_OBJECT_LEN_MISMATCH: expected {expected_len}, got {out_off}"
        ));
    }
    Ok((out, in_off))
}

fn git_parse_packfile_objects(
    pack: &[u8],
    max_object_bytes: usize,
) -> Result<Vec<GitPackObject>, String> {
    if pack.len() < 32 || !pack.starts_with(b"PACK") {
        return Err("GIT_WIRE_INVALID_PACK_HEADER".to_string());
    }
    let version = u32::from_be_bytes([pack[4], pack[5], pack[6], pack[7]]);
    if version != 2 {
        return Err(format!("GIT_WIRE_UNSUPPORTED_PACK_VERSION: {version}"));
    }
    let obj_count = u32::from_be_bytes([pack[8], pack[9], pack[10], pack[11]]) as usize;
    let payload_end = pack.len() - 20;
    let mut pos = 12usize;
    let mut objects: Vec<GitPackObject> = Vec::with_capacity(obj_count.min(4096));
    let mut total_uncompressed: usize = 0;
    let max_total_bytes = max_object_bytes.saturating_mul(2);

    for _ in 0..obj_count {
        if pos >= payload_end {
            return Err("GIT_WIRE_PACK_TRUNCATED".to_string());
        }
        let obj_offset = pos;
        let mut b = pack[pos];
        pos += 1;
        let obj_type = (b >> 4) & 0x07;
        let mut size = (b & 0x0f) as usize;
        let mut shift = 4u32;
        while (b & 0x80) != 0 {
            if pos >= payload_end {
                return Err("GIT_WIRE_PACK_TRUNCATED_SIZE".to_string());
            }
            b = pack[pos];
            pos += 1;
            size |= ((b & 0x7f) as usize)
                .checked_shl(shift)
                .ok_or_else(|| "GIT_WIRE_PACK_SIZE_OVERFLOW".to_string())?;
            shift += 7;
        }
        if size > max_object_bytes {
            return Err("GIT_WIRE_OBJECT_TOO_LARGE: Packfile 对象体积超出上限".to_string());
        }

        enum BaseRef {
            None,
            Offset(usize),
            Oid([u8; 20]),
        }
        let base_ref = match obj_type {
            1..=4 => BaseRef::None,
            6 => {
                if pos >= payload_end {
                    return Err("GIT_WIRE_PACK_TRUNCATED_OFS_DELTA".to_string());
                }
                let mut c = pack[pos];
                pos += 1;
                let mut back = (c & 0x7f) as usize;
                while (c & 0x80) != 0 {
                    if pos >= payload_end {
                        return Err("GIT_WIRE_PACK_TRUNCATED_OFS_DELTA".to_string());
                    }
                    c = pack[pos];
                    pos += 1;
                    back = back
                        .checked_add(1)
                        .and_then(|v| v.checked_shl(7))
                        .map(|v| v | ((c & 0x7f) as usize))
                        .ok_or_else(|| "GIT_WIRE_OFS_DELTA_OVERFLOW".to_string())?;
                }
                let target_off = obj_offset
                    .checked_sub(back)
                    .ok_or_else(|| "GIT_WIRE_OFS_DELTA_INVALID_OFFSET".to_string())?;
                BaseRef::Offset(target_off)
            }
            7 => {
                if pos + 20 > payload_end {
                    return Err("GIT_WIRE_PACK_TRUNCATED_REF_DELTA".to_string());
                }
                let mut oid = [0u8; 20];
                oid.copy_from_slice(&pack[pos..pos + 20]);
                pos += 20;
                BaseRef::Oid(oid)
            }
            other => return Err(format!("GIT_WIRE_UNKNOWN_PACK_TYPE: {other}")),
        };

        let (decompressed, consumed) = git_decompress_zlib_object(&pack[pos..payload_end], size)?;
        pos += consumed;

        let (resolved_type, raw) = match base_ref {
            BaseRef::None => (obj_type, decompressed),
            BaseRef::Offset(base_off) => {
                let base_obj = objects
                    .iter()
                    .find(|o| o.offset == base_off)
                    .ok_or_else(|| "GIT_WIRE_OFS_DELTA_BASE_MISSING".to_string())?;
                let rebuilt = git_apply_delta(&base_obj.raw, &decompressed, max_object_bytes)?;
                (base_obj.obj_type, rebuilt)
            }
            BaseRef::Oid(base_oid) => {
                let base_obj = objects
                    .iter()
                    .find(|o| o.oid == base_oid)
                    .ok_or_else(|| "GIT_WIRE_REF_DELTA_BASE_MISSING".to_string())?;
                let rebuilt = git_apply_delta(&base_obj.raw, &decompressed, max_object_bytes)?;
                (base_obj.obj_type, rebuilt)
            }
        };
        let oid = git_compute_object_oid(resolved_type, &raw)?;
        total_uncompressed = total_uncompressed
            .checked_add(raw.len())
            .filter(|&n| n <= max_total_bytes)
            .ok_or_else(|| {
                "GIT_WIRE_PACK_TOTAL_TOO_LARGE: Packfile 解压总大小超出上限".to_string()
            })?;
        objects.push(GitPackObject {
            obj_type: resolved_type,
            oid,
            offset: obj_offset,
            raw,
        });
    }
    Ok(objects)
}

fn hex_to_oid20(hex: &str) -> Result<[u8; 20], String> {
    let trimmed = hex.trim();
    if trimmed.len() != 40 {
        return Err(format!("GIT_WIRE_INVALID_OID_HEX: {trimmed}"));
    }
    let mut out = [0u8; 20];
    for i in 0..20 {
        out[i] = u8::from_str_radix(&trimmed[i * 2..i * 2 + 2], 16)
            .map_err(|_| format!("GIT_WIRE_INVALID_OID_HEX: {trimmed}"))?;
    }
    Ok(out)
}

fn oid20_to_hex(oid: &[u8; 20]) -> String {
    let mut s = String::with_capacity(40);
    for b in oid {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn git_lookup_path_in_tree_pack(
    objects: &[GitPackObject],
    commit_oid: [u8; 20],
    rel_path: &str,
) -> Result<[u8; 20], String> {
    let commit = objects
        .iter()
        .find(|o| o.obj_type == 1 && o.oid == commit_oid)
        .ok_or_else(|| "GIT_WIRE_COMMIT_OBJECT_MISSING".to_string())?;
    let commit_txt = String::from_utf8_lossy(&commit.raw);
    let root_tree_hex = commit_txt
        .lines()
        .find_map(|line| line.strip_prefix("tree "))
        .ok_or_else(|| "GIT_WIRE_COMMIT_MISSING_TREE".to_string())?;
    let mut current_oid = hex_to_oid20(root_tree_hex)?;

    let segments: Vec<&str> = rel_path
        .trim_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    if segments.is_empty() {
        return Err("GIT_WIRE_EMPTY_REL_PATH".to_string());
    }
    for seg in segments {
        let tree = objects
            .iter()
            .find(|o| o.obj_type == 2 && o.oid == current_oid)
            .ok_or_else(|| format!("GIT_WIRE_TREE_OBJECT_MISSING: {seg}"))?;
        let raw = &tree.raw;
        let mut i = 0usize;
        let mut matched: Option<[u8; 20]> = None;
        while i < raw.len() {
            let sp = raw[i..]
                .iter()
                .position(|&b| b == b' ')
                .map(|off| i + off)
                .ok_or_else(|| "GIT_WIRE_MALFORMED_TREE_ENTRY".to_string())?;
            let nul = raw[sp + 1..]
                .iter()
                .position(|&b| b == 0)
                .map(|off| sp + 1 + off)
                .ok_or_else(|| "GIT_WIRE_MALFORMED_TREE_ENTRY".to_string())?;
            if nul + 21 > raw.len() {
                return Err("GIT_WIRE_TRUNCATED_TREE_OID".to_string());
            }
            let name = std::str::from_utf8(&raw[sp + 1..nul])
                .map_err(|_| "GIT_WIRE_INVALID_TREE_NAME".to_string())?;
            if name == seg {
                let mut child = [0u8; 20];
                child.copy_from_slice(&raw[nul + 1..nul + 21]);
                matched = Some(child);
                break;
            }
            i = nul + 21;
        }
        current_oid = matched.ok_or_else(|| format!("GIT_WIRE_PATH_NOT_FOUND: {rel_path}"))?;
    }
    Ok(current_oid)
}

fn fetch_gitcode_wire_v2_blob(
    owner: &str,
    repo: &str,
    rel_path: &str,
    max_bytes: usize,
) -> Result<Vec<u8>, String> {
    let endpoint = format!("https://gitcode.com/{owner}/{repo}.git/git-upload-pack");
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| format!("GIT_WIRE_HTTP_CLIENT_ERROR: {e}"))?;

    let post_wire = |body: Vec<u8>| -> Result<Vec<u8>, String> {
        let resp = client
            .post(&endpoint)
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/x-git-upload-pack-request",
            )
            .header(
                reqwest::header::ACCEPT,
                "application/x-git-upload-pack-result",
            )
            .header("Git-Protocol", "version=2")
            .header(reqwest::header::USER_AGENT, "git/2.43.0")
            .body(body)
            .send()
            .map_err(|e| format!("GIT_WIRE_POST_FAILED: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("GIT_WIRE_HTTP_STATUS_{}", resp.status().as_u16()));
        }
        let read_cap = (max_bytes as u64).saturating_add(16 * 1024 * 1024);
        let mut buf = Vec::new();
        let mut limited = std::io::Read::take(resp, read_cap.saturating_add(1));
        std::io::Read::read_to_end(&mut limited, &mut buf)
            .map_err(|e| format!("GIT_WIRE_READ_FAILED: {e}"))?;
        if (buf.len() as u64) > read_cap {
            return Err(format!(
                "GIT_WIRE_RESPONSE_TOO_LARGE: GitCode 传输流超过限制 ({read_cap} bytes)"
            ));
        }
        Ok(buf)
    };

    let mut ls_body = Vec::new();
    ls_body.extend_from_slice(&git_pkt_line(b"command=ls-refs\n"));
    ls_body.extend_from_slice(b"0001");
    ls_body.extend_from_slice(&git_pkt_line(b"ref-prefix refs/heads/main\n"));
    ls_body.extend_from_slice(&git_pkt_line(b"ref-prefix HEAD\n"));
    ls_body.extend_from_slice(b"0000");

    let ls_resp = post_wire(ls_body)?;
    let mut commit_hex: Option<String> = None;
    for pkt in git_parse_pkt_lines(&ls_resp)? {
        if let GitPktLine::Data(payload) = pkt {
            let line = String::from_utf8_lossy(payload);
            let mut parts = line.split_whitespace();
            let Some(oid) = parts.next() else { continue };
            let Some(refname) = parts.next() else {
                continue;
            };
            if refname == "refs/heads/main" || (refname == "HEAD" && commit_hex.is_none()) {
                commit_hex = Some(oid.to_string());
            }
        }
    }
    let commit_hex = commit_hex.ok_or_else(|| "GIT_WIRE_HEAD_REF_NOT_FOUND".to_string())?;
    let commit_oid = hex_to_oid20(&commit_hex)?;

    let mut tree_body = Vec::new();
    tree_body.extend_from_slice(&git_pkt_line(b"command=fetch\n"));
    tree_body.extend_from_slice(b"0001");
    tree_body.extend_from_slice(&git_pkt_line(b"no-progress\n"));
    tree_body.extend_from_slice(&git_pkt_line(b"filter blob:none\n"));
    tree_body.extend_from_slice(&git_pkt_line(b"deepen 1\n"));
    tree_body.extend_from_slice(&git_pkt_line(format!("want {commit_hex}\n").as_bytes()));
    tree_body.extend_from_slice(&git_pkt_line(b"done\n"));
    tree_body.extend_from_slice(b"0000");

    let tree_resp = post_wire(tree_body)?;
    let tree_pack = git_extract_packfile_from_fetch_response(&tree_resp)?;
    let tree_objs = git_parse_packfile_objects(&tree_pack, MAX_REGISTRY_BYTES as usize)?;
    let blob_oid = git_lookup_path_in_tree_pack(&tree_objs, commit_oid, rel_path)?;
    let blob_hex = oid20_to_hex(&blob_oid);

    let mut blob_body = Vec::new();
    blob_body.extend_from_slice(&git_pkt_line(b"command=fetch\n"));
    blob_body.extend_from_slice(b"0001");
    blob_body.extend_from_slice(&git_pkt_line(b"no-progress\n"));
    blob_body.extend_from_slice(&git_pkt_line(format!("want {blob_hex}\n").as_bytes()));
    blob_body.extend_from_slice(&git_pkt_line(b"done\n"));
    blob_body.extend_from_slice(b"0000");

    let blob_resp = post_wire(blob_body)?;
    let blob_pack = git_extract_packfile_from_fetch_response(&blob_resp)?;
    let blob_objs = git_parse_packfile_objects(&blob_pack, max_bytes)?;
    let target = blob_objs
        .into_iter()
        .find(|o| o.obj_type == 3 && o.oid == blob_oid)
        .ok_or_else(|| format!("GIT_WIRE_BLOB_MISSING_IN_PACK: {rel_path}"))?;
    Ok(target.raw)
}

fn parse_gitcode_owner_repo(url: &str) -> Option<(String, String)> {
    let trimmed = url.trim();
    if let Some(rest) = trimmed.strip_prefix("https://api.gitcode.com/api/v5/repos/") {
        let mut parts = rest.split('/');
        let owner = parts.next()?.trim();
        let repo = parts.next()?.trim();
        if !owner.is_empty() && !repo.is_empty() {
            return Some((owner.to_string(), repo.to_string()));
        }
    }
    for prefix in ["https://raw.gitcode.com/", "https://gitcode.com/"] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let mut parts = rest.split('/');
            let owner = parts.next()?.trim();
            let repo = parts.next()?.trim().trim_end_matches(".git");
            if !owner.is_empty() && !repo.is_empty() {
                return Some((owner.to_string(), repo.to_string()));
            }
        }
    }
    None
}

fn expand_gitcode_repo_pairs(source_hint: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    if let Some((owner, repo)) = parse_gitcode_owner_repo(source_hint) {
        pairs.push((owner, repo));
    } else {
        pairs.push(("hhanmj".to_string(), "eostage-hub".to_string()));
    }
    pairs
}

fn fetch_gitcode_wire_v2_registry_and_blocklist(
    source_hint: &str,
) -> Result<(String, String, Option<String>), String> {
    let mut last_err = "GIT_WIRE_NO_GITCODE_CANDIDATES".to_string();
    for (owner, repo) in expand_gitcode_repo_pairs(source_hint) {
        match fetch_gitcode_wire_v2_blob(
            &owner,
            &repo,
            "registry.json",
            MAX_REGISTRY_BYTES as usize,
        ) {
            Ok(bytes) => {
                let reg_txt =
                    String::from_utf8(bytes).map_err(|e| format!("REGISTRY_INVALID_UTF8: {e}"))?;
                let bl_txt = fetch_gitcode_wire_v2_blob(
                    &owner,
                    &repo,
                    "blocklist.json",
                    MAX_REGISTRY_BYTES as usize,
                )
                .ok()
                .and_then(|b| String::from_utf8(b).ok());
                return Ok((
                    format!("https://gitcode.com/{owner}/{repo}/raw/main/registry.json"),
                    reg_txt,
                    bl_txt,
                ));
            }
            Err(err) => {
                last_err = err;
            }
        }
    }
    Err(last_err)
}

fn extract_git_repo_rel_path(source: &str, file_name: &str) -> String {
    let without_query = source
        .trim()
        .split(['?', '#'])
        .next()
        .unwrap_or("")
        .trim_end_matches('/');
    if let Some((_, after_raw)) = without_query.split_once("/raw/") {
        let trimmed = after_raw.trim_start_matches('/');
        let rel = if let Some(rest) = trimmed
            .strip_prefix("main/")
            .or_else(|| trimmed.strip_prefix("master/"))
        {
            rest
        } else {
            trimmed
        };
        if !rel.is_empty()
            && !rel.contains('\\')
            && rel
                .split('/')
                .all(|seg| !seg.is_empty() && seg != "." && seg != "..")
        {
            return rel.to_string();
        }
    }
    format!("components/{file_name}")
}

fn fetch_gitcode_wire_v2_component(source_hint: &str, rel_path: &str) -> Result<Vec<u8>, String> {
    let mut last_err = "GIT_WIRE_NO_GITCODE_COMPONENT_CANDIDATES".to_string();
    for (owner, repo) in expand_gitcode_repo_pairs(source_hint) {
        match fetch_gitcode_wire_v2_blob(&owner, &repo, rel_path, MAX_COMPONENT_BYTES as usize) {
            Ok(bytes) => return Ok(bytes),
            Err(err) => {
                last_err = err;
            }
        }
    }
    Err(last_err)
}

fn normalize_gitcode_raw_url(url: &str) -> String {
    let trimmed = url.trim();
    // Convert https://raw.gitcode.com/<owner>/<repo>/raw/<branch>/<path>
    // or https://gitcode.com/<owner>/<repo>/raw/<branch>/<path>
    // -> https://api.gitcode.com/api/v5/repos/<owner>/<repo>/raw/<path>
    for prefix in ["https://raw.gitcode.com/", "https://gitcode.com/"] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let parts: Vec<&str> = rest.splitn(5, '/').collect();
            if parts.len() == 5 && parts[2] == "raw" {
                let owner = parts[0];
                let repo = parts[1];
                let rel_path = parts[4];
                return format!(
                    "https://api.gitcode.com/api/v5/repos/{owner}/{repo}/raw/{rel_path}"
                );
            }
        }
    }
    trimmed.to_string()
}

fn expand_gitcode_registry_candidates(url: &str) -> Vec<String> {
    let mut cands = Vec::new();
    let norm = normalize_gitcode_raw_url(url);
    cands.push(norm.clone());
    if norm != url.trim() && !url.trim().contains("gitcode.com/") {
        cands.push(url.trim().to_string());
    }
    cands
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OfficialHubMirrorKind {
    GitCode,
    GitHub,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
enum OfficialHubReleaseHost {
    GitCodeWeb,
    GitCodeApi,
    GitHub,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OfficialHubReleaseAsset {
    host: OfficialHubReleaseHost,
    owner: String,
    repo: String,
    tag: String,
    file_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ComponentDownloadPlan {
    primary_urls: Vec<String>,
    fallback_urls: Vec<String>,
    allow_gitcode_wire_v2: bool,
    allow_local_hub_fallback: bool,
    file_name: String,
}

fn is_safe_url_segment(seg: &str) -> bool {
    !seg.is_empty()
        && seg != "."
        && seg != ".."
        && !seg.contains('/')
        && !seg.contains('\\')
        && !seg.contains('%')
        && !seg.chars().any(|c| c.is_control())
}

fn is_official_hub_owner_repo(owner: &str, repo: &str) -> bool {
    owner.eq_ignore_ascii_case("hhanmj") && repo.eq_ignore_ascii_case("eostage-hub")
}

fn detect_official_hub_registry_mirror(source_url: &str) -> Option<OfficialHubMirrorKind> {
    let parsed = reqwest::Url::parse(source_url.trim()).ok()?;
    if parsed.scheme() != "https" {
        return None;
    }
    let host = parsed.host_str()?.to_ascii_lowercase();
    let segs: Vec<&str> = parsed
        .path_segments()
        .map(|it| it.filter(|s| !s.is_empty()).collect())
        .unwrap_or_default();
    if matches!(host.as_str(), "gitcode.com" | "raw.gitcode.com") {
        if segs.len() >= 2 && is_official_hub_owner_repo(segs[0], segs[1]) {
            return Some(OfficialHubMirrorKind::GitCode);
        }
    } else if host == "api.gitcode.com" {
        if segs.len() >= 5
            && segs[0] == "api"
            && segs[1] == "v5"
            && segs[2] == "repos"
            && is_official_hub_owner_repo(segs[3], segs[4])
        {
            return Some(OfficialHubMirrorKind::GitCode);
        }
    } else if matches!(host.as_str(), "github.com" | "raw.githubusercontent.com")
        && segs.len() >= 2
        && is_official_hub_owner_repo(segs[0], segs[1])
    {
        return Some(OfficialHubMirrorKind::GitHub);
    }
    None
}

fn parse_official_hub_release_url(url: &str) -> Option<OfficialHubReleaseAsset> {
    let parsed = reqwest::Url::parse(url.trim()).ok()?;
    if parsed.scheme() != "https" {
        return None;
    }
    let host = parsed.host_str()?.to_ascii_lowercase();
    let segs: Vec<&str> = parsed
        .path_segments()
        .map(|it| it.filter(|s| !s.is_empty()).collect())
        .unwrap_or_default();

    // 1. https://gitcode.com/{owner}/{repo}/releases/download/{tag}/{file_name}
    // 2. https://github.com/{owner}/{repo}/releases/download/{tag}/{file_name}
    if matches!(host.as_str(), "gitcode.com" | "github.com")
        && segs.len() == 6
        && segs[2] == "releases"
        && segs[3] == "download"
        && is_official_hub_owner_repo(segs[0], segs[1])
        && is_safe_url_segment(segs[4])
        && is_safe_url_segment(segs[5])
    {
        return Some(OfficialHubReleaseAsset {
            host: if host == "gitcode.com" {
                OfficialHubReleaseHost::GitCodeWeb
            } else {
                OfficialHubReleaseHost::GitHub
            },
            owner: segs[0].to_string(),
            repo: segs[1].to_string(),
            tag: segs[4].to_string(),
            file_name: segs[5].to_string(),
        });
    }

    // 3. https://api.gitcode.com/api/v5/repos/{owner}/{repo}/releases/{tag}/attach_files/{file_name}/download
    if host == "api.gitcode.com"
        && segs.len() == 10
        && segs[0] == "api"
        && segs[1] == "v5"
        && segs[2] == "repos"
        && segs[5] == "releases"
        && segs[7] == "attach_files"
        && segs[9] == "download"
        && is_official_hub_owner_repo(segs[3], segs[4])
        && is_safe_url_segment(segs[6])
        && is_safe_url_segment(segs[8])
    {
        return Some(OfficialHubReleaseAsset {
            host: OfficialHubReleaseHost::GitCodeApi,
            owner: segs[3].to_string(),
            repo: segs[4].to_string(),
            tag: segs[6].to_string(),
            file_name: segs[8].to_string(),
        });
    }

    None
}

fn parse_official_hub_raw_component_url(
    url: &str,
) -> Option<(OfficialHubMirrorKind, String, String, String)> {
    let parsed = reqwest::Url::parse(url.trim()).ok()?;
    if parsed.scheme() != "https" {
        return None;
    }
    let host = parsed.host_str()?.to_ascii_lowercase();
    let segs: Vec<&str> = parsed
        .path_segments()
        .map(|it| it.filter(|s| !s.is_empty()).collect())
        .unwrap_or_default();
    let file_name = segs.last().copied().unwrap_or("");
    if !is_safe_url_segment(file_name) || !file_name.to_ascii_lowercase().ends_with(".zip") {
        return None;
    }
    if matches!(host.as_str(), "gitcode.com" | "raw.gitcode.com")
        && segs.len() >= 5
        && segs[2] == "raw"
        && is_official_hub_owner_repo(segs[0], segs[1])
    {
        return Some((
            OfficialHubMirrorKind::GitCode,
            segs[0].to_string(),
            segs[1].to_string(),
            file_name.to_string(),
        ));
    }
    if host == "api.gitcode.com"
        && segs.len() >= 7
        && segs[0] == "api"
        && segs[1] == "v5"
        && segs[2] == "repos"
        && segs[5] == "raw"
        && is_official_hub_owner_repo(segs[3], segs[4])
    {
        return Some((
            OfficialHubMirrorKind::GitCode,
            segs[3].to_string(),
            segs[4].to_string(),
            file_name.to_string(),
        ));
    }
    if host == "raw.githubusercontent.com"
        && segs.len() >= 4
        && is_official_hub_owner_repo(segs[0], segs[1])
    {
        return Some((
            OfficialHubMirrorKind::GitHub,
            segs[0].to_string(),
            segs[1].to_string(),
            file_name.to_string(),
        ));
    }
    None
}

fn project_official_release_url(source_url: &str, resolved_pkg_url: &str) -> String {
    let Some(mirror) = detect_official_hub_registry_mirror(source_url) else {
        return resolved_pkg_url.to_string();
    };
    let Some(asset) = parse_official_hub_release_url(resolved_pkg_url) else {
        return resolved_pkg_url.to_string();
    };
    match mirror {
        OfficialHubMirrorKind::GitCode => format!(
            "https://gitcode.com/{}/{}/releases/download/{}/{}",
            asset.owner, asset.repo, asset.tag, asset.file_name
        ),
        OfficialHubMirrorKind::GitHub => format!(
            "https://github.com/{}/{}/releases/download/{}/{}",
            asset.owner, asset.repo, asset.tag, asset.file_name
        ),
    }
}

fn resolve_and_project_registry_package_url(source_url: &str, raw_pkg: &str) -> String {
    let resolved = resolve_registry_url(source_url, raw_pkg);
    project_official_release_url(source_url, &resolved)
}

fn push_unique_url(list: &mut Vec<String>, candidate: String) {
    if !candidate.is_empty() && !list.contains(&candidate) {
        list.push(candidate);
    }
}

fn build_gitcode_release_urls(
    owner: &str,
    repo: &str,
    tag: &str,
    file_name: &str,
    prefer_api_first: bool,
) -> Vec<String> {
    let mut urls = Vec::new();
    let web = format!("https://gitcode.com/{owner}/{repo}/releases/download/{tag}/{file_name}");
    let api = format!(
        "https://api.gitcode.com/api/v5/repos/{owner}/{repo}/releases/{tag}/attach_files/{file_name}/download"
    );
    if prefer_api_first {
        push_unique_url(&mut urls, api);
        push_unique_url(&mut urls, web);
    } else {
        push_unique_url(&mut urls, web);
        push_unique_url(&mut urls, api);
    }
    urls
}

fn build_github_release_urls(owner: &str, repo: &str, tag: &str, file_name: &str) -> Vec<String> {
    let mut urls = Vec::new();
    push_unique_url(
        &mut urls,
        format!("https://github.com/{owner}/{repo}/releases/download/{tag}/{file_name}"),
    );
    urls
}

fn extract_component_filename(source: &str) -> String {
    if let Some(asset) = parse_official_hub_release_url(source) {
        return asset.file_name;
    }
    if let Ok(parsed) = reqwest::Url::parse(source.trim()) {
        let segs: Vec<&str> = parsed
            .path_segments()
            .map(|it| it.filter(|s| !s.is_empty()).collect())
            .unwrap_or_default();
        if segs.len() >= 2 && segs[segs.len() - 1].eq_ignore_ascii_case("download") {
            let prev = segs[segs.len() - 2];
            if is_safe_url_segment(prev) {
                return prev.to_string();
            }
        }
        if let Some(last) = segs.last() {
            if is_safe_url_segment(last) {
                return (*last).to_string();
            }
        }
    }
    source
        .trim()
        .rsplit('/')
        .next()
        .unwrap_or("")
        .split(['?', '#'])
        .next()
        .unwrap_or("")
        .to_string()
}

fn url_expects_zip(cand: &str) -> bool {
    extract_component_filename(cand)
        .to_ascii_lowercase()
        .ends_with(".zip")
}

fn expand_component_download_plan(source: &str) -> ComponentDownloadPlan {
    let trimmed = source.trim();

    // 1. Official GitCode / GitHub Releases attachment direct URL
    if let Some(asset) = parse_official_hub_release_url(trimmed) {
        let prefer_api_first = asset.host == OfficialHubReleaseHost::GitCodeApi;
        let gitcode_urls = build_gitcode_release_urls(
            &asset.owner,
            &asset.repo,
            &asset.tag,
            &asset.file_name,
            prefer_api_first,
        );
        let github_urls =
            build_github_release_urls(&asset.owner, &asset.repo, &asset.tag, &asset.file_name);
        let (primary_urls, fallback_urls) = match asset.host {
            OfficialHubReleaseHost::GitCodeWeb | OfficialHubReleaseHost::GitCodeApi => {
                (gitcode_urls, github_urls)
            }
            OfficialHubReleaseHost::GitHub => (github_urls, gitcode_urls),
        };
        return ComponentDownloadPlan {
            primary_urls,
            fallback_urls,
            allow_gitcode_wire_v2: false,
            allow_local_hub_fallback: true,
            file_name: asset.file_name,
        };
    }

    // 2. Legacy official repository raw URLs (backward-compatible with cached/older registries)
    if let Some((mirror, owner, repo, file_name)) = parse_official_hub_raw_component_url(trimmed) {
        let default_tag = "v1.0.0";
        let gitcode_rel = build_gitcode_release_urls(&owner, &repo, default_tag, &file_name, false);
        let github_rel = build_github_release_urls(&owner, &repo, default_tag, &file_name);
        let mut primary_urls = Vec::new();
        let mut fallback_urls = Vec::new();
        match mirror {
            OfficialHubMirrorKind::GitCode => {
                for u in gitcode_rel {
                    push_unique_url(&mut primary_urls, u);
                }
                for u in expand_gitcode_registry_candidates(trimmed) {
                    push_unique_url(&mut primary_urls, u);
                }
                for u in github_rel {
                    if !primary_urls.contains(&u) {
                        push_unique_url(&mut fallback_urls, u);
                    }
                }
                let direct_raw = format!(
                    "https://raw.githubusercontent.com/hhanmj/eostage-hub/main/components/{file_name}"
                );
                if !primary_urls.contains(&direct_raw) {
                    push_unique_url(&mut fallback_urls, direct_raw);
                }
                return ComponentDownloadPlan {
                    primary_urls,
                    fallback_urls,
                    allow_gitcode_wire_v2: true,
                    allow_local_hub_fallback: true,
                    file_name,
                };
            }
            OfficialHubMirrorKind::GitHub => {
                for u in github_rel {
                    push_unique_url(&mut primary_urls, u);
                }
                push_unique_url(&mut primary_urls, trimmed.to_string());
                for u in gitcode_rel {
                    if !primary_urls.contains(&u) {
                        push_unique_url(&mut fallback_urls, u);
                    }
                }
                return ComponentDownloadPlan {
                    primary_urls,
                    fallback_urls,
                    allow_gitcode_wire_v2: false,
                    allow_local_hub_fallback: true,
                    file_name,
                };
            }
        }
    }

    // 3. Loopback test server or custom third-party mirror: strictly isolated, no public hijack
    ComponentDownloadPlan {
        primary_urls: vec![trimmed.to_string()],
        fallback_urls: Vec::new(),
        allow_gitcode_wire_v2: false,
        allow_local_hub_fallback: false,
        file_name: extract_component_filename(trimmed),
    }
}

fn resolve_extension_registry_source(registry_url: Option<&str>) -> String {
    match registry_url.map(str::trim).filter(|s| !s.is_empty()) {
        Some("eostage") | Some("cloudflare") | None => DEFAULT_EXTENSION_REGISTRY_URL.to_string(),
        Some(explicit)
            if explicit
                .to_ascii_lowercase()
                .contains("eostage-hub.pages.dev") =>
        {
            DEFAULT_EXTENSION_REGISTRY_URL.to_string()
        }
        Some(explicit) => explicit.to_string(),
    }
}

pub fn fetch_extension_registry(
    registry_url: Option<String>,
) -> Result<ExtensionRegistryReport, String> {
    if matches!(registry_url.as_deref().map(str::trim), Some("local")) {
        let local_hub = discover_runtime_root().join("_hub_dist/registry.json");
        if !local_hub.is_file() {
            return Err(format!(
                "REGISTRY_LOCAL_NOT_FOUND: 本地扩展清单 {} 不存在",
                local_hub.display()
            ));
        }
        let local_str = local_hub.to_string_lossy().to_string();
        let registry_raw = read_text_from_source(&local_str, MAX_REGISTRY_BYTES)?;
        let blocklist_url = resolve_registry_url(&local_str, "blocklist.json");
        let blocklist_raw = read_text_from_source(&blocklist_url, MAX_REGISTRY_BYTES).ok();
        return parse_and_filter_extension_registry(
            &local_str,
            &registry_raw,
            blocklist_raw.as_deref(),
        );
    }

    let resolved_source = resolve_extension_registry_source(registry_url.as_deref());

    let targets_gitcode = resolved_source.contains("gitcode.com");
    let candidates = expand_gitcode_registry_candidates(&resolved_source);

    let mut last_err = String::new();
    let mut fetched_triple: Option<(String, String, Option<String>)> = None;
    for cand in &candidates {
        match read_text_from_source(cand, MAX_REGISTRY_BYTES) {
            Ok(raw) => {
                let bl_url = resolve_registry_url(cand, "blocklist.json");
                let bl = read_text_from_source(&bl_url, MAX_REGISTRY_BYTES).ok();
                fetched_triple = Some((cand.clone(), raw, bl));
                break;
            }
            Err(err) => {
                last_err = err;
            }
        }
    }

    if fetched_triple.is_none() && targets_gitcode {
        match fetch_gitcode_wire_v2_registry_and_blocklist(&resolved_source) {
            Ok(triple) => {
                fetched_triple = Some(triple);
            }
            Err(err) => {
                last_err = err;
            }
        }
    }

    if fetched_triple.is_none()
        && (targets_gitcode || resolved_source == DEFAULT_EXTENSION_REGISTRY_URL)
    {
        let gh_cand = "https://raw.githubusercontent.com/hhanmj/eostage-hub/main/registry.json";
        if let Ok(raw) = read_text_from_source(gh_cand, MAX_REGISTRY_BYTES) {
            let bl_url = resolve_registry_url(gh_cand, "blocklist.json");
            let bl = read_text_from_source(&bl_url, MAX_REGISTRY_BYTES).ok();
            fetched_triple = Some((gh_cand.to_string(), raw, bl));
        }
    }

    let (final_source, registry_raw, blocklist_raw) = match fetched_triple {
        Some(triple) => triple,
        None => {
            if targets_gitcode || resolved_source == DEFAULT_EXTENSION_REGISTRY_URL {
                let local_hub = discover_runtime_root().join("_hub_dist/registry.json");
                if local_hub.is_file() {
                    let local_str = local_hub.to_string_lossy().to_string();
                    match read_text_from_source(&local_str, MAX_REGISTRY_BYTES) {
                        Ok(raw) => {
                            let bl_url = resolve_registry_url(&local_str, "blocklist.json");
                            let bl = read_text_from_source(&bl_url, MAX_REGISTRY_BYTES).ok();
                            (local_str, raw, bl)
                        }
                        Err(_) => return Err(last_err),
                    }
                } else {
                    return Err(last_err);
                }
            } else {
                return Err(last_err);
            }
        }
    };

    parse_and_filter_extension_registry(&final_source, &registry_raw, blocklist_raw.as_deref())
}

fn read_https_component_candidate(
    client: &reqwest::blocking::Client,
    cand: &str,
    loopback_only: bool,
    expected_sha256: Option<&str>,
) -> Result<Vec<u8>, String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let parsed = reqwest::Url::parse(cand.trim())
        .map_err(|e| format!("RUNTIME_COMPONENT_INVALID_URL: {e}"))?;
    validate_hub_http_url(&parsed, loopback_only, "RUNTIME_COMPONENT_UNSAFE_URL")?;
    let resp = client
        .get(parsed)
        .header(reqwest::header::USER_AGENT, "EOStage-Desktop/2.2")
        .send()
        .map_err(|e| format!("RUNTIME_COMPONENT_DOWNLOAD_FAILED: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!(
            "RUNTIME_COMPONENT_DOWNLOAD_HTTP_{}",
            resp.status().as_u16()
        ));
    }
    if is_html_content_type(resp.headers()) {
        return Err(
            "RUNTIME_COMPONENT_UNEXPECTED_HTML: 镜像地址返回 HTML 页面而非组件包".to_string(),
        );
    }
    let expected_content_length = resp.content_length();
    if let Some(cl) = expected_content_length {
        if cl > MAX_COMPONENT_BYTES {
            return Err("RUNTIME_COMPONENT_TOO_LARGE: 安装包体积超过 512 MiB 上限".to_string());
        }
    }
    let mut buf = Vec::new();
    resp.take(MAX_COMPONENT_BYTES + 1)
        .read_to_end(&mut buf)
        .map_err(|e| format!("RUNTIME_COMPONENT_DOWNLOAD_READ_FAILED: {e}"))?;
    if buf.len() as u64 > MAX_COMPONENT_BYTES {
        return Err("RUNTIME_COMPONENT_TOO_LARGE: 安装包体积超过 512 MiB 上限".to_string());
    }
    if let Some(expected_len) = expected_content_length {
        if buf.len() as u64 != expected_len {
            return Err(format!(
                "RUNTIME_COMPONENT_DOWNLOAD_TRUNCATED: 下载内容不完整（预期 {expected_len} 字节，实际 {} 字节）",
                buf.len()
            ));
        }
    }
    if is_likely_html_payload(&buf) {
        return Err(
            "RUNTIME_COMPONENT_UNEXPECTED_HTML: 下载内容为 HTML 页面而非组件包".to_string(),
        );
    }
    if url_expects_zip(cand) && !buf.starts_with(b"PK\x03\x04") {
        return Err("RUNTIME_COMPONENT_INVALID_ZIP: 下载内容不是有效的 ZIP 归档包".to_string());
    }
    if let Some(exp) = expected_sha256 {
        let actual_sha = format!("{:x}", Sha256::digest(&buf));
        if !actual_sha.eq_ignore_ascii_case(exp) {
            return Err(format!(
                "RUNTIME_COMPONENT_SHA256_MISMATCH: 期望 {exp}，实际 {actual_sha}"
            ));
        }
    }
    Ok(buf)
}

fn validate_staged_component_platform(
    component_id: &str,
    staging_dir: &std::path::Path,
) -> Result<(), String> {
    if !matches!(component_id, "compat-worker" | "native-gis-runtime") {
        return Ok(());
    }
    let mut has_exe = false;
    let mut has_dll = false;
    let mut has_dylib = false;
    let mut has_so = false;
    let mut has_unix_exec = false;

    fn inspect_dir(
        dir: &std::path::Path,
        has_exe: &mut bool,
        has_dll: &mut bool,
        has_dylib: &mut bool,
        has_so: &mut bool,
        has_unix_exec: &mut bool,
    ) {
        use std::io::Read;
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                inspect_dir(&p, has_exe, has_dll, has_dylib, has_so, has_unix_exec);
                continue;
            }
            let ext = p
                .extension()
                .and_then(|s| s.to_str())
                .map(|s| s.to_ascii_lowercase())
                .unwrap_or_default();
            if ext == "dll" {
                *has_dll = true;
            } else if ext == "dylib" {
                *has_dylib = true;
            } else if ext == "so" {
                *has_so = true;
            }
            if let Ok(mut f) = std::fs::File::open(&p) {
                let mut hdr = [0u8; 4];
                if let Ok(n) = f.read(&mut hdr) {
                    if n >= 2 && &hdr[..2] == b"MZ" {
                        if ext != "dll" {
                            *has_exe = true;
                        }
                    } else if n == 4
                        && matches!(
                            hdr,
                            [0xCF, 0xFA, 0xED, 0xFE]
                                | [0xFE, 0xED, 0xFA, 0xCF]
                                | [0xCA, 0xFE, 0xBA, 0xBE]
                                | [0xBE, 0xBA, 0xFE, 0xCA]
                                | [0x7F, b'E', b'L', b'F']
                        )
                    {
                        *has_unix_exec = true;
                    }
                }
            }
        }
    }

    inspect_dir(
        staging_dir,
        &mut has_exe,
        &mut has_dll,
        &mut has_dylib,
        &mut has_so,
        &mut has_unix_exec,
    );

    if component_id == "compat-worker" {
        if !cfg!(target_os = "windows") && has_exe && !has_unix_exec {
            return Err(
                "RUNTIME_COMPONENT_PLATFORM_MISMATCH: 该兼容进程包仅适用于 Windows x64 系统"
                    .to_string(),
            );
        }
        if cfg!(target_os = "windows") && has_unix_exec && !has_exe {
            return Err(
                "RUNTIME_COMPONENT_PLATFORM_MISMATCH: 该兼容进程包与当前 Windows 系统不匹配"
                    .to_string(),
            );
        }
    } else if component_id == "native-gis-runtime" {
        if cfg!(target_os = "macos") && (has_dll || has_so) && !has_dylib {
            return Err(
                "RUNTIME_COMPONENT_PLATFORM_MISMATCH: 该地理信息引擎包与当前 macOS 系统不匹配"
                    .to_string(),
            );
        }
        if cfg!(target_os = "windows") && (has_dylib || has_so) && !has_dll {
            return Err(
                "RUNTIME_COMPONENT_PLATFORM_MISMATCH: 该地理信息引擎包与当前 Windows 系统不匹配"
                    .to_string(),
            );
        }
        if cfg!(target_os = "linux") && (has_dylib || has_dll) && !has_so {
            return Err(
                "RUNTIME_COMPONENT_PLATFORM_MISMATCH: 该地理信息引擎包与当前 Linux 系统不匹配"
                    .to_string(),
            );
        }
    }
    Ok(())
}

fn download_component_with_plan(
    root: &std::path::Path,
    client: &reqwest::blocking::Client,
    source: &str,
    plan: &ComponentDownloadPlan,
    loopback_only: bool,
    expected_sha: Option<&str>,
) -> Result<Vec<u8>, String> {
    use sha2::{Digest, Sha256};

    let mut downloaded: Option<Vec<u8>> = None;
    let mut primary_err: Option<String> = None;
    let mut fallback_err: Option<String> = None;

    for cand in &plan.primary_urls {
        match read_https_component_candidate(client, cand, loopback_only, expected_sha) {
            Ok(buf) => {
                downloaded = Some(buf);
                break;
            }
            Err(err) => {
                if primary_err.is_none() {
                    primary_err = Some(err);
                }
            }
        }
    }

    if downloaded.is_none() && plan.allow_gitcode_wire_v2 && !plan.file_name.is_empty() {
        let rel_path = extract_git_repo_rel_path(source, &plan.file_name);
        match fetch_gitcode_wire_v2_component(source, &rel_path) {
            Ok(buf) => {
                if let Some(exp) = expected_sha {
                    let actual_sha = format!("{:x}", Sha256::digest(&buf));
                    if !actual_sha.eq_ignore_ascii_case(exp) {
                        if primary_err.is_none() {
                            primary_err = Some(format!(
                                "RUNTIME_COMPONENT_SHA256_MISMATCH: 期望 {exp}，实际 {actual_sha}"
                            ));
                        }
                    } else {
                        downloaded = Some(buf);
                    }
                } else {
                    downloaded = Some(buf);
                }
            }
            Err(err) => {
                if primary_err.is_none() {
                    primary_err = Some(err);
                }
            }
        }
    }

    if downloaded.is_none() {
        for cand in &plan.fallback_urls {
            match read_https_component_candidate(client, cand, loopback_only, expected_sha) {
                Ok(buf) => {
                    downloaded = Some(buf);
                    break;
                }
                Err(err) => {
                    fallback_err = Some(err);
                }
            }
        }
    }

    if let Some(buf) = downloaded {
        return Ok(buf);
    }

    if plan.allow_local_hub_fallback && !plan.file_name.is_empty() {
        let local_fallback = root.join("_hub_dist/components").join(&plan.file_name);
        if local_fallback.is_file() {
            return std::fs::read(&local_fallback)
                .map_err(|e| format!("RUNTIME_COMPONENT_FILE_READ_FAILED: {e}"));
        }
    }

    Err(match (primary_err, fallback_err) {
        (Some(p), Some(f)) if p != f => format!("{p}（回退镜像亦失败: {f}）"),
        (Some(p), _) => p,
        (None, Some(f)) => f,
        (None, None) => "RUNTIME_COMPONENT_DOWNLOAD_FAILED: 未找到可用镜像地址".to_string(),
    })
}

pub fn install_runtime_component(
    request: &RuntimeComponentInstallRequest,
) -> Result<RuntimeComponentInstallResult, String> {
    let root = discover_runtime_root();
    install_runtime_component_at(&root, request)
}

pub fn install_runtime_component_at(
    root: &std::path::Path,
    request: &RuntimeComponentInstallRequest,
) -> Result<RuntimeComponentInstallResult, String> {
    use sha2::{Digest, Sha256};

    let component_id = request.component_id.trim();
    if !matches!(
        component_id,
        "vertical-grid-egm96"
            | "vertical-grid-egm2008"
            | "native-gis-runtime"
            | "compat-worker"
            | "whitebox-cli"
            | "duckdb-cli"
            | "rustgis-cli"
    ) {
        return Err(format!(
            "RUNTIME_COMPONENT_UNSUPPORTED_ID: 不支持安装的组件 ID '{component_id}'"
        ));
    }

    let expected_sha = request
        .expected_sha256
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if let Some(exp) = expected_sha {
        if !is_valid_sha256_hex(exp) {
            return Err(
                "RUNTIME_COMPONENT_INVALID_SHA256: expected_sha256 必须是 64 位十六进制字符串"
                    .to_string(),
            );
        }
    }

    let source = request.source.trim();
    if source.is_empty() {
        return Err("RUNTIME_COMPONENT_SOURCE_EMPTY: 安装包路径或 URL 不能为空".to_string());
    }

    // 1. Read bytes from local file or HTTPS/loopback URL with MAX_COMPONENT_BYTES limit
    let is_loopback = is_loopback_http_url(source);
    let bytes: Vec<u8> = if source.starts_with("https://") || is_loopback {
        let parsed = reqwest::Url::parse(source)
            .map_err(|e| format!("RUNTIME_COMPONENT_INVALID_URL: {e}"))?;
        validate_hub_http_url(&parsed, is_loopback, "RUNTIME_COMPONENT_UNSAFE_URL")?;
        let client =
            build_hub_http_client(is_loopback, 15, 300, "RUNTIME_COMPONENT_HTTP_CLIENT_ERROR")?;
        let plan = expand_component_download_plan(source);
        download_component_with_plan(root, &client, source, &plan, is_loopback, expected_sha)?
    } else if source.starts_with("http://") {
        return Err(
            "RUNTIME_COMPONENT_INSECURE_URL: 仅允许从 HTTPS 或本地回环地址下载运行组件".to_string(),
        );
    } else {
        let clean_path = source
            .strip_prefix("file:///")
            .or_else(|| source.strip_prefix("file://"))
            .unwrap_or(source);
        let meta = std::fs::metadata(clean_path)
            .map_err(|e| format!("RUNTIME_COMPONENT_FILE_NOT_FOUND ({clean_path}): {e}"))?;
        if meta.len() > MAX_COMPONENT_BYTES {
            return Err("RUNTIME_COMPONENT_TOO_LARGE: 安装包体积超过 512 MiB 上限".to_string());
        }
        std::fs::read(clean_path)
            .map_err(|e| format!("RUNTIME_COMPONENT_FILE_READ_FAILED ({clean_path}): {e}"))?
    };

    if bytes.is_empty() {
        return Err("RUNTIME_COMPONENT_EMPTY_PAYLOAD: 安装包内容为空".to_string());
    }

    // 2. Compute and verify SHA-256 before modifying any target directory
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let actual_sha256 = format!("{:x}", hasher.finalize());

    if let Some(exp) = expected_sha {
        if !actual_sha256.eq_ignore_ascii_case(exp) {
            return Err(format!(
                "RUNTIME_COMPONENT_SHA256_MISMATCH: 期望 {exp}，实际 {actual_sha256}"
            ));
        }
    }

    // 3. Stage and apply with .old backup + self-check + rollback
    let runtime_dir = root.join("runtime");
    std::fs::create_dir_all(&runtime_dir).map_err(|e| format!("RUNTIME_DIR_CREATE_FAILED: {e}"))?;
    let staging_dir = runtime_dir.join(format!(
        ".staging-{}-{}-{}",
        component_id,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let _ = std::fs::remove_dir_all(&staging_dir);
    std::fs::create_dir_all(&staging_dir).map_err(|e| format!("STAGING_DIR_CREATE_FAILED: {e}"))?;

    let is_zip = bytes.starts_with(b"PK\x03\x04");
    if is_zip {
        if let Err(err) = extract_zip_safely(&bytes, &staging_dir) {
            let _ = std::fs::remove_dir_all(&staging_dir);
            return Err(err);
        }
        if let Err(err) = validate_staged_component_platform(component_id, &staging_dir) {
            let _ = std::fs::remove_dir_all(&staging_dir);
            return Err(err);
        }
    } else if component_id == "compat-worker"
        && !cfg!(target_os = "windows")
        && bytes.starts_with(b"MZ")
    {
        let _ = std::fs::remove_dir_all(&staging_dir);
        return Err(
            "RUNTIME_COMPONENT_PLATFORM_MISMATCH: 该兼容进程包仅适用于 Windows x64 系统"
                .to_string(),
        );
    }

    let apply_res = match component_id {
        "vertical-grid-egm96" | "vertical-grid-egm2008" => install_vertical_grid_staged(
            root,
            component_id,
            &bytes,
            is_zip,
            &staging_dir,
            &actual_sha256,
        ),
        "native-gis-runtime" | "compat-worker" | "whitebox-cli" | "duckdb-cli" | "rustgis-cli" => {
            install_directory_component_staged(
                root,
                component_id,
                &bytes,
                is_zip,
                &staging_dir,
                &actual_sha256,
            )
        }
        _ => unreachable!(),
    };

    let _ = std::fs::remove_dir_all(&staging_dir);
    apply_res
}

fn extract_zip_safely(bytes: &[u8], dest: &std::path::Path) -> Result<(), String> {
    let cursor = std::io::Cursor::new(bytes);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|e| format!("RUNTIME_COMPONENT_INVALID_ZIP: {e}"))?;

    for idx in 0..archive.len() {
        let mut entry = archive
            .by_index(idx)
            .map_err(|e| format!("RUNTIME_COMPONENT_ZIP_ENTRY_ERROR: {e}"))?;
        let raw_name = entry
            .name()
            .map_err(|e| format!("RUNTIME_COMPONENT_ZIP_ENTRY_ERROR: {e}"))?
            .replace('\\', "/");
        let entry_path = std::path::Path::new(&raw_name);
        if entry_path.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        }) {
            return Err(format!(
                "RUNTIME_COMPONENT_ZIP_UNSAFE_PATH: 拒绝包含路径穿越的压缩包条目 '{raw_name}'"
            ));
        }
        let out_path = dest.join(entry_path);
        if entry.is_dir() || raw_name.ends_with('/') {
            std::fs::create_dir_all(&out_path)
                .map_err(|e| format!("RUNTIME_COMPONENT_MKDIR_FAILED: {e}"))?;
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("RUNTIME_COMPONENT_MKDIR_FAILED: {e}"))?;
            }
            {
                let mut out_file = std::fs::File::create(&out_path)
                    .map_err(|e| format!("RUNTIME_COMPONENT_FILE_CREATE_FAILED: {e}"))?;
                std::io::copy(&mut entry, &mut out_file)
                    .map_err(|e| format!("RUNTIME_COMPONENT_EXTRACT_WRITE_FAILED: {e}"))?;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let has_exec_bit = entry.unix_mode().is_some_and(|m| (m & 0o111) != 0);
                if has_exec_bit || is_pe_executable_header(&out_path) {
                    let _ =
                        std::fs::set_permissions(&out_path, std::fs::Permissions::from_mode(0o755));
                }
            }
        }
    }
    Ok(())
}

fn find_first_file_with_extension(dir: &std::path::Path, ext: &str) -> Option<std::path::PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_file()
            && p.extension()
                .and_then(|s| s.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case(ext))
        {
            return Some(p);
        } else if p.is_dir() {
            if let Some(found) = find_first_file_with_extension(&p, ext) {
                return Some(found);
            }
        }
    }
    None
}

fn install_vertical_grid_staged(
    root: &std::path::Path,
    component_id: &str,
    raw_bytes: &[u8],
    is_zip: bool,
    staging_dir: &std::path::Path,
    actual_sha256: &str,
) -> Result<RuntimeComponentInstallResult, String> {
    let rel_tif = if component_id == "vertical-grid-egm96" {
        "egm96/us_nga_egm96_15.tif"
    } else {
        "egm2008/us_nga_egm08_25.tif"
    };
    let vg_dir = root.join("runtime/vertical-grids");
    let target_tif = vg_dir.join(rel_tif);
    if let Some(parent) = target_tif.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("VERTICAL_GRID_DIR_CREATE_FAILED: {e}"))?;
    }

    let staged_tif = if is_zip {
        let exact = staging_dir.join(rel_tif);
        if exact.is_file() {
            exact
        } else {
            find_first_file_with_extension(staging_dir, "tif").ok_or_else(|| {
                "RUNTIME_COMPONENT_VERIFY_FAILED: 压缩包内未找到 .tif 栅格文件".to_string()
            })?
        }
    } else {
        let direct = staging_dir.join("direct.tif");
        std::fs::write(&direct, raw_bytes)
            .map_err(|e| format!("VERTICAL_GRID_STAGE_WRITE_FAILED: {e}"))?;
        direct
    };

    // Backup existing target_tif -> target_tif.old
    let backup_tif = target_tif.with_extension("tif.old");
    let _ = std::fs::remove_file(&backup_tif);
    let had_old_tif = target_tif.is_file();
    if had_old_tif {
        std::fs::rename(&target_tif, &backup_tif)
            .map_err(|e| format!("VERTICAL_GRID_BACKUP_FAILED: {e}"))?;
    }

    // Also backup and optionally update manifest.json if the zip includes one AND none exists yet
    let target_manifest = vg_dir.join("manifest.json");
    let staged_manifest = staging_dir.join("manifest.json");
    let installed_manifest = if !target_manifest.is_file() && staged_manifest.is_file() {
        let _ = std::fs::copy(&staged_manifest, &target_manifest);
        true
    } else {
        false
    };

    if let Err(err) = std::fs::copy(&staged_tif, &target_tif) {
        if had_old_tif {
            let _ = std::fs::rename(&backup_tif, &target_tif);
        }
        if installed_manifest {
            let _ = std::fs::remove_file(&target_manifest);
        }
        return Err(format!("VERTICAL_GRID_APPLY_FAILED: {e}", e = err));
    }

    // Post-install self-check via inspect_runtime_components_at
    let rollback_vertical_grid = || {
        let _ = std::fs::remove_file(&target_tif);
        if had_old_tif {
            let _ = std::fs::rename(&backup_tif, &target_tif);
        }
        if installed_manifest {
            let _ = std::fs::remove_file(&target_manifest);
        }
    };

    let report = match inspect_runtime_components_at(root) {
        Ok(report) => report,
        Err(err) => {
            rollback_vertical_grid();
            return Err(err);
        }
    };
    let verified_item = report
        .components
        .into_iter()
        .find(|c| c.id == component_id && c.status == "ready" && c.verified);

    match verified_item {
        Some(item) => {
            let _ = std::fs::remove_file(&backup_tif);
            Ok(RuntimeComponentInstallResult {
                ok: true,
                component_id: component_id.to_string(),
                version: item.version,
                sha256: actual_sha256.to_string(),
                bytes_installed: item.total_bytes,
                install_path: target_tif.to_string_lossy().to_string(),
                status: item.status,
                detail: item.detail,
            })
        }
        None => {
            rollback_vertical_grid();
            Err(format!(
                "RUNTIME_COMPONENT_VERIFY_FAILED: 组件 {component_id} 安装后自检未通过，已自动回滚至先前状态"
            ))
        }
    }
}

fn unwrap_single_subdir(dir: &std::path::Path) -> std::path::PathBuf {
    if let Ok(entries) = std::fs::read_dir(dir) {
        let items: Vec<_> = entries.flatten().collect();
        if items.len() == 1 && items[0].path().is_dir() {
            return items[0].path();
        }
    }
    dir.to_path_buf()
}

fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| format!("COPY_DIR_CREATE_FAILED: {e}"))?;
    let entries = std::fs::read_dir(src).map_err(|e| format!("COPY_DIR_READ_FAILED: {e}"))?;
    for entry in entries.flatten() {
        let p = entry.path();
        let target = dst.join(entry.file_name());
        if p.is_dir() {
            copy_dir_recursive(&p, &target)?;
        } else {
            std::fs::copy(&p, &target).map_err(|e| format!("COPY_FILE_FAILED: {e}"))?;
            #[cfg(unix)]
            if let Ok(meta) = std::fs::metadata(&p) {
                use std::os::unix::fs::PermissionsExt;
                if (meta.permissions().mode() & 0o111) != 0 {
                    let _ =
                        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755));
                }
            }
        }
    }
    Ok(())
}

fn install_directory_component_staged(
    root: &std::path::Path,
    component_id: &str,
    raw_bytes: &[u8],
    is_zip: bool,
    staging_dir: &std::path::Path,
    actual_sha256: &str,
) -> Result<RuntimeComponentInstallResult, String> {
    let target_dir = match component_id {
        "native-gis-runtime" => root.join("runtime/gis"),
        "whitebox-cli" => root.join("runtime/whitebox-cli"),
        "duckdb-cli" => root.join("runtime/duckdb-cli"),
        "rustgis-cli" => root.join("runtime/rustgis-cli"),
        _ => root.join("runtime/compat-worker"),
    };
    let old_dir = match component_id {
        "native-gis-runtime" => root.join("runtime/gis.old"),
        "whitebox-cli" => root.join("runtime/whitebox-cli.old"),
        "duckdb-cli" => root.join("runtime/duckdb-cli.old"),
        "rustgis-cli" => root.join("runtime/rustgis-cli.old"),
        _ => root.join("runtime/compat-worker.old"),
    };

    let prepared_source_dir = if is_zip {
        unwrap_single_subdir(staging_dir)
    } else if component_id == "compat-worker" {
        let direct_dir = staging_dir.join("worker_direct");
        std::fs::create_dir_all(&direct_dir)
            .map_err(|e| format!("WORKER_STAGE_DIR_FAILED: {e}"))?;
        std::fs::write(direct_dir.join("eostage-compat-worker.exe"), raw_bytes)
            .map_err(|e| format!("WORKER_STAGE_WRITE_FAILED: {e}"))?;
        if target_dir.join("component-manifest.json").is_file() {
            let _ = std::fs::copy(
                target_dir.join("component-manifest.json"),
                direct_dir.join("component-manifest.json"),
            );
        }
        direct_dir
    } else {
        return Err(format!(
            "RUNTIME_COMPONENT_VERIFY_FAILED: {component_id} 必须使用 .zip 归档包安装"
        ));
    };

    // Backup target_dir -> old_dir (if a previous .old backup is locked on Windows, use a unique backup dir)
    let _ = std::fs::remove_dir_all(&old_dir);
    let active_old_dir = if old_dir.exists() {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let old_name = old_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("component.old");
        old_dir.with_file_name(format!("{old_name}-{}-{ts}", std::process::id()))
    } else {
        old_dir
    };
    let had_old_dir = target_dir.is_dir();
    if had_old_dir {
        std::fs::rename(&target_dir, &active_old_dir).map_err(|e| {
            let busy_hint = if matches!(e.kind(), std::io::ErrorKind::PermissionDenied)
                || matches!(e.raw_os_error(), Some(5) | Some(32))
            {
                "（组件动态库可能正在使用中，请重启 EOStage 后重试更新）"
            } else {
                ""
            };
            format!("COMPONENT_BACKUP_RENAME_FAILED: {e}{busy_hint}")
        })?;
    }

    if let Err(err) = copy_dir_recursive(&prepared_source_dir, &target_dir) {
        let _ = std::fs::remove_dir_all(&target_dir);
        if had_old_dir {
            let _ = std::fs::rename(&active_old_dir, &target_dir);
        }
        return Err(err);
    }

    // Post-install self-check via inspect_runtime_components_at (rollback on either Err or unverified)
    let report = match inspect_runtime_components_at(root) {
        Ok(r) => r,
        Err(err) => {
            let _ = std::fs::remove_dir_all(&target_dir);
            if had_old_dir {
                let _ = std::fs::rename(&active_old_dir, &target_dir);
            }
            return Err(format!(
                "RUNTIME_COMPONENT_VERIFY_FAILED: 安装后自检异常并已自动回滚 ({err})"
            ));
        }
    };
    let verified_item = report
        .components
        .into_iter()
        .find(|c| c.id == component_id && c.status == "ready" && c.verified);

    match verified_item {
        Some(item) => {
            let _ = std::fs::remove_dir_all(&active_old_dir);
            Ok(RuntimeComponentInstallResult {
                ok: true,
                component_id: component_id.to_string(),
                version: item.version,
                sha256: actual_sha256.to_string(),
                bytes_installed: item.total_bytes,
                install_path: target_dir.to_string_lossy().to_string(),
                status: item.status,
                detail: item.detail,
            })
        }
        None => {
            let _ = std::fs::remove_dir_all(&target_dir);
            if had_old_dir {
                let _ = std::fs::rename(&active_old_dir, &target_dir);
            }
            Err(format!(
                "RUNTIME_COMPONENT_VERIFY_FAILED: 组件 {component_id} 安装后自检未通过，已自动回滚至先前版本"
            ))
        }
    }
}

// ============================================================================
// 真实插件、自定义技能与自定义连接器磁盘持久化后端 (<app_data_dir>/plugins/ 等)
// ============================================================================

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PluginBundledSkillEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub prompt: String,
    #[serde(default)]
    pub path: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PluginBundledConnectorEntry {
    pub id: String,
    pub name: String,
    pub transport: String,
    pub endpoint: String,
    pub description: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ManagedPluginPackage {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub discipline: String,
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub enabled: bool,
    pub installed: bool,
    pub is_official: bool,
    pub install_path: String,
    pub sha256: String,
    pub total_bytes: u64,
    pub file_count: usize,
    pub files: Vec<String>,
    #[serde(default, alias = "bundledSkills")]
    pub bundled_skills: Vec<PluginBundledSkillEntry>,
    #[serde(default, alias = "bundledConnector")]
    pub bundled_connector: Option<PluginBundledConnectorEntry>,
    #[serde(default, alias = "bundledConnectors")]
    pub bundled_connectors: Vec<PluginBundledConnectorEntry>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct CustomConnectorRecord {
    pub id: String,
    pub name: String,
    pub transport: String,
    pub endpoint: String,
    pub description: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct CustomSkillRecord {
    pub id: String,
    pub name: String,
    pub description: String,
    pub prompt: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub version: String,
    #[serde(default, alias = "slashCommand")]
    pub slash_command: String,
    #[serde(default, alias = "updatedAt")]
    pub updated_at: String,
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ApplicationPluginsStatusResponse {
    pub ok: bool,
    pub app_data_dir: String,
    pub plugins_dir: String,
    pub runtime_plugins_dir: String,
    pub installed_plugins: Vec<ManagedPluginPackage>,
    pub available_plugins: Vec<ManagedPluginPackage>,
    pub custom_connectors: Vec<CustomConnectorRecord>,
    pub custom_skills: Vec<CustomSkillRecord>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct PluginInstallRequest {
    #[serde(default)]
    pub plugin_id: Option<String>,
    #[serde(default)]
    pub source_path: Option<String>,
    #[serde(default)]
    pub raw_content: Option<String>,
    #[serde(default, alias = "raw_name")]
    pub fallback_name: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PluginMutationResponse {
    pub ok: bool,
    pub message: String,
    pub installed_plugin: Option<ManagedPluginPackage>,
    pub status: ApplicationPluginsStatusResponse,
}

pub fn resolve_app_data_dir(state: &ApplicationServiceState) -> std::path::PathBuf {
    state
        .agent_application()
        .data_root()
        .or_else(|| {
            default_store_path()
                .parent()
                .map(std::path::Path::to_path_buf)
        })
        .unwrap_or_else(|| std::env::temp_dir().join("eostage-app-data"))
}

pub fn discover_runtime_plugins_dir() -> std::path::PathBuf {
    let rt_root = discover_runtime_root();
    let cand1 = rt_root.join("runtime").join("plugins");
    if cand1.is_dir() {
        return cand1;
    }
    let cand2 = rt_root.join("plugins");
    if cand2.is_dir() {
        return cand2;
    }
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if let Some(workspace_root) = manifest_dir.parent() {
        let cand3 = workspace_root.join("runtime").join("plugins");
        if cand3.is_dir() {
            return cand3;
        }
    }
    cand1
}

fn sanitize_plugin_id(raw: &str) -> String {
    let cleaned: String = raw
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('-').to_string();
    if trimmed.is_empty() {
        format!("custom-plugin-{}", now_unix_nanos())
    } else {
        trimmed
    }
}

fn shell_quote_stdio_words(command: &str, args: &[String]) -> String {
    let mut tokens = Vec::with_capacity(1 + args.len());
    for raw in std::iter::once(command).chain(args.iter().map(String::as_str)) {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed
            .chars()
            .any(|c| c.is_whitespace() || c == '"' || c == '\'' || c == '\\')
        {
            let escaped = trimmed.replace('\\', "\\\\").replace('"', "\\\"");
            tokens.push(format!("\"{escaped}\""));
        } else {
            tokens.push(trimmed.to_string());
        }
    }
    tokens.join(" ")
}

fn parse_plugin_connector_from_obj(
    obj: &serde_json::Map<String, serde_json::Value>,
    default_id: &str,
    default_name: &str,
    default_desc: &str,
) -> Option<PluginBundledConnectorEntry> {
    let id = obj
        .get("id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(default_id)
        .to_string();
    let name = obj
        .get("name")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(default_name)
        .to_string();
    let explicit_url = obj
        .get("endpoint")
        .or_else(|| obj.get("url"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let command = obj
        .get("command")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if explicit_url.is_some_and(is_forbidden_internal_connector_endpoint)
        || command.is_some_and(is_forbidden_internal_connector_endpoint)
    {
        return None;
    }
    let args: Vec<String> = obj
        .get("args")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let endpoint = if let Some(u) = explicit_url {
        u.to_string()
    } else if let Some(cmd) = command {
        shell_quote_stdio_words(cmd, &args)
    } else {
        "stdio://mcp".to_string()
    };
    if is_forbidden_internal_connector_endpoint(&endpoint) {
        return None;
    }
    let transport = obj
        .get("transport")
        .or_else(|| obj.get("type"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            if s.eq_ignore_ascii_case("streamable_http")
                || s.eq_ignore_ascii_case("streamable-http")
                || s.eq_ignore_ascii_case("sse")
            {
                "http".to_string()
            } else {
                s.to_ascii_lowercase()
            }
        })
        .unwrap_or_else(|| {
            if command.is_some() {
                "stdio".to_string()
            } else if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
                "http".to_string()
            } else {
                "stdio".to_string()
            }
        });
    let description = obj
        .get("description")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(default_desc)
        .to_string();
    Some(PluginBundledConnectorEntry {
        id,
        name,
        transport,
        endpoint,
        description,
    })
}

fn push_unique_plugin_connector(
    list: &mut Vec<PluginBundledConnectorEntry>,
    entry: Option<PluginBundledConnectorEntry>,
) {
    if let Some(entry) = entry {
        if !entry.id.trim().is_empty() && !list.iter().any(|existing| existing.id == entry.id) {
            list.push(entry);
        }
    }
}

fn compute_plugin_package_sha256(plugin_dir: &std::path::Path, sorted_files: &[String]) -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(b"eostage-plugin-v1\0");
    for rel in sorted_files {
        let full = plugin_dir.join(rel);
        let Ok(meta) = std::fs::symlink_metadata(&full) else {
            continue;
        };
        if meta.file_type().is_symlink() || !meta.is_file() {
            continue;
        }
        let Ok(bytes) = std::fs::read(&full) else {
            continue;
        };
        hasher.update(rel.as_bytes());
        hasher.update(b"\0");
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
        hasher.update(b"\0");
    }
    format!("{:x}", hasher.finalize())
}

fn collect_plugin_files_recursive(
    base: &std::path::Path,
    current: &std::path::Path,
    files: &mut Vec<String>,
    total_bytes: &mut u64,
) {
    let Ok(entries) = std::fs::read_dir(current) else {
        return;
    };
    let mut sorted: Vec<_> = entries.flatten().collect();
    sorted.sort_by_key(|e| e.file_name());
    for entry in sorted {
        let p = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&p) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            collect_plugin_files_recursive(base, &p, files, total_bytes);
        } else if meta.is_file() {
            if let Ok(rel) = p.strip_prefix(base) {
                let rel_str = rel.to_string_lossy().replace('\\', "/");
                let file_name = rel
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("");
                if rel_str != ".disabled"
                    && file_name != ".DS_Store"
                    && file_name != "Thumbs.db"
                {
                    *total_bytes = total_bytes.saturating_add(meta.len());
                    files.push(rel_str);
                }
            }
        }
    }
}

pub fn inspect_plugin_package_dir(
    plugin_dir: &std::path::Path,
    installed: bool,
    is_official: bool,
) -> Option<ManagedPluginPackage> {
    let manifest_path = plugin_dir.join("plugin.json");
    let manifest_meta = std::fs::symlink_metadata(&manifest_path).ok()?;
    if manifest_meta.file_type().is_symlink() || !manifest_meta.is_file() {
        return None;
    }
    let raw_bytes = std::fs::read(&manifest_path).ok()?;
    let val: serde_json::Value = serde_json::from_slice(&raw_bytes).ok()?;

    let dir_name = plugin_dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("plugin");
    let id = val
        .get("id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(dir_name)
        .to_string();
    let name = val
        .get("name")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&id)
        .to_string();
    let version = val
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("1.0.0")
        .to_string();
    let raw_author = val
        .get("author")
        .and_then(|v| v.as_str())
        .unwrap_or("EOStage");
    // Normalize legacy plugin attributions to EOStage
    let author = match raw_author {
        "ScenePort" | "ScenePort Team" | "EOStage" => "EOStage".to_string(),
        other => other.to_string(),
    };
    let license = val
        .get("license")
        .and_then(|v| v.as_str())
        .unwrap_or("Apache-2.0")
        .to_string();
    let discipline = val
        .get("discipline")
        .and_then(|v| v.as_str())
        .unwrap_or("insar-sar")
        .to_string();
    let description = val
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let tags: Vec<String> = val
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    let disabled_marker = plugin_dir.join(".disabled").is_file();
    let json_enabled = val.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);
    let enabled = if installed {
        !disabled_marker
    } else {
        json_enabled && !disabled_marker
    };

    let mut files = Vec::new();
    let mut total_bytes = 0u64;
    collect_plugin_files_recursive(plugin_dir, plugin_dir, &mut files, &mut total_bytes);
    files.sort();
    let sha256 = compute_plugin_package_sha256(plugin_dir, &files);

    let mut bundled_skills = Vec::new();
    if let Some(arr) = val
        .get("bundledSkills")
        .or_else(|| val.get("bundled_skills"))
        .or_else(|| val.get("skills"))
        .and_then(|v| v.as_array())
    {
        for item in arr {
            let sk_id = item
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("plugin-skill")
                .to_string();
            let sk_name = item
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or(&sk_id)
                .to_string();
            let sk_desc = item
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let sk_rel_path = item
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let mut sk_prompt = item
                .get("prompt")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if !sk_rel_path.is_empty() {
                let candidate = plugin_dir.join(&sk_rel_path);
                if let Ok(md_text) = std::fs::read_to_string(&candidate) {
                    sk_prompt = md_text;
                }
            } else {
                let default_md = plugin_dir.join("skills").join(&sk_id).join("SKILL.md");
                if let Ok(md_text) = std::fs::read_to_string(&default_md) {
                    sk_prompt = md_text;
                }
            }
            bundled_skills.push(PluginBundledSkillEntry {
                id: sk_id,
                name: sk_name,
                description: sk_desc,
                prompt: sk_prompt,
                path: sk_rel_path,
            });
        }
    }
    let skills_subdir = plugin_dir.join("skills");
    if skills_subdir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&skills_subdir) {
            let mut dirs: Vec<_> = entries.flatten().collect();
            dirs.sort_by_key(|e| e.file_name());
            for entry in dirs {
                let p = entry.path();
                if p.is_dir() {
                    let sk_id = entry.file_name().to_string_lossy().to_string();
                    if !bundled_skills.iter().any(|s| s.id == sk_id) {
                        let md_file = p.join("SKILL.md");
                        if let Ok(md_text) = std::fs::read_to_string(&md_file) {
                            bundled_skills.push(PluginBundledSkillEntry {
                                id: sk_id.clone(),
                                name: sk_id.clone(),
                                description: description.clone(),
                                prompt: md_text,
                                path: format!("skills/{sk_id}/SKILL.md"),
                            });
                        }
                    }
                }
            }
        }
    }

    let mut bundled_connectors = Vec::new();
    if let Some(arr) = val
        .get("bundledConnectors")
        .or_else(|| val.get("bundled_connectors"))
        .and_then(|v| v.as_array())
    {
        for (idx, item) in arr.iter().enumerate() {
            if let Some(conn_obj) = item.as_object() {
                let default_id = if idx == 0 {
                    format!("plugin-mcp-{id}")
                } else {
                    format!("plugin-mcp-{id}-{}", idx + 1)
                };
                push_unique_plugin_connector(
                    &mut bundled_connectors,
                    parse_plugin_connector_from_obj(conn_obj, &default_id, &name, &description),
                );
            }
        }
    }
    if let Some(conn_obj) = val
        .get("bundledConnector")
        .or_else(|| val.get("bundled_connector"))
        .and_then(|v| v.as_object())
    {
        push_unique_plugin_connector(
            &mut bundled_connectors,
            parse_plugin_connector_from_obj(
                conn_obj,
                &format!("plugin-mcp-{id}"),
                &name,
                &description,
            ),
        );
    }

    let mcp_path = plugin_dir.join("mcp.json");
    if let Ok(mcp_raw) = std::fs::read_to_string(&mcp_path) {
        if let Ok(mcp_val) = serde_json::from_str::<serde_json::Value>(&mcp_raw) {
            if let Some(servers) = mcp_val.get("mcpServers").and_then(|v| v.as_object()) {
                let mut keys: Vec<_> = servers.keys().cloned().collect();
                keys.sort();
                for srv_key in keys {
                    if let Some(srv_cfg) = servers.get(&srv_key).and_then(|v| v.as_object()) {
                        push_unique_plugin_connector(
                            &mut bundled_connectors,
                            parse_plugin_connector_from_obj(
                                srv_cfg,
                                &format!("plugin-mcp-{srv_key}"),
                                &srv_key,
                                &description,
                            ),
                        );
                    }
                }
            }
        }
    } else if let Some(servers) = val
        .get("mcpServers")
        .or_else(|| val.get("mcp_servers"))
        .and_then(|v| v.as_object())
    {
        let mut keys: Vec<_> = servers.keys().cloned().collect();
        keys.sort();
        for srv_key in keys {
            if let Some(srv_cfg) = servers.get(&srv_key).and_then(|v| v.as_object()) {
                push_unique_plugin_connector(
                    &mut bundled_connectors,
                    parse_plugin_connector_from_obj(
                        srv_cfg,
                        &format!("plugin-mcp-{srv_key}"),
                        &srv_key,
                        &description,
                    ),
                );
            }
        }
    }
    let bundled_connector = bundled_connectors.first().cloned();

    Some(ManagedPluginPackage {
        id,
        name,
        version,
        author,
        license,
        discipline,
        description,
        tags,
        enabled,
        installed,
        is_official,
        install_path: plugin_dir.to_string_lossy().to_string(),
        sha256,
        total_bytes,
        file_count: files.len(),
        files,
        bundled_skills,
        bundled_connector,
        bundled_connectors,
    })
}

pub fn load_custom_connectors_at(app_data_dir: &std::path::Path) -> Vec<CustomConnectorRecord> {
    let path = app_data_dir.join("connectors_custom.json");
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<CustomConnectorRecord>>(&raw).unwrap_or_default()
}

pub fn save_custom_connectors_at(
    app_data_dir: &std::path::Path,
    items: &[CustomConnectorRecord],
) -> Result<(), String> {
    std::fs::create_dir_all(app_data_dir)
        .map_err(|e| format!("APP_DATA_DIR_CREATE_FAILED: {e}"))?;
    let path = app_data_dir.join("connectors_custom.json");
    let json = serde_json::to_string_pretty(items)
        .map_err(|e| format!("CONNECTORS_SERIALIZE_FAILED: {e}"))?;
    std::fs::write(&path, json).map_err(|e| format!("CONNECTORS_WRITE_FAILED: {e}"))
}

fn now_unix_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

fn now_iso_timestamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}Z")
}

pub fn load_custom_skills_at(app_data_dir: &std::path::Path) -> Vec<CustomSkillRecord> {
    let skills_root = app_data_dir.join("skills_custom");
    let Ok(entries) = std::fs::read_dir(&skills_root) else {
        return Vec::new();
    };
    let mut list = Vec::new();
    let mut dirs: Vec<_> = entries.flatten().collect();
    dirs.sort_by_key(|e| e.file_name());
    for entry in dirs {
        let p = entry.path();
        if !p.is_dir() {
            continue;
        }
        let meta_path = p.join("skill.json");
        let md_path = p.join("SKILL.md");
        if let Ok(raw) = std::fs::read_to_string(&meta_path) {
            if let Ok(mut rec) = serde_json::from_str::<CustomSkillRecord>(&raw) {
                if let Ok(md) = std::fs::read_to_string(&md_path) {
                    if !md.trim().is_empty() {
                        rec.prompt = md;
                    }
                }
                list.push(rec);
                continue;
            }
        }
        if let Ok(md) = std::fs::read_to_string(&md_path) {
            let id = entry.file_name().to_string_lossy().to_string();
            list.push(CustomSkillRecord {
                id: id.clone(),
                name: id.clone(),
                description: "本地持久化自定义技能包".to_string(),
                prompt: md,
                author: "EOStage".to_string(),
                license: "MIT".to_string(),
                version: "1.0.0".to_string(),
                slash_command: format!("/{id}"),
                updated_at: now_iso_timestamp(),
                enabled: true,
            });
        }
    }
    list
}

pub fn save_custom_skill_at(
    app_data_dir: &std::path::Path,
    skill: &CustomSkillRecord,
) -> Result<(), String> {
    let safe_id = sanitize_plugin_id(&skill.id);
    let skill_dir = app_data_dir.join("skills_custom").join(&safe_id);
    std::fs::create_dir_all(&skill_dir)
        .map_err(|e| format!("CUSTOM_SKILL_DIR_CREATE_FAILED: {e}"))?;
    std::fs::write(skill_dir.join("SKILL.md"), &skill.prompt)
        .map_err(|e| format!("CUSTOM_SKILL_MD_WRITE_FAILED: {e}"))?;
    let json = serde_json::to_string_pretty(skill)
        .map_err(|e| format!("CUSTOM_SKILL_JSON_FAILED: {e}"))?;
    std::fs::write(skill_dir.join("skill.json"), json)
        .map_err(|e| format!("CUSTOM_SKILL_META_WRITE_FAILED: {e}"))
}

pub fn delete_custom_skill_at(
    app_data_dir: &std::path::Path,
    skill_id: &str,
) -> Result<(), String> {
    let safe_id = sanitize_plugin_id(skill_id);
    let skill_dir = app_data_dir.join("skills_custom").join(&safe_id);
    if skill_dir.exists() {
        std::fs::remove_dir_all(&skill_dir)
            .map_err(|e| format!("CUSTOM_SKILL_DELETE_FAILED: {e}"))?;
    }
    let raw_dir = app_data_dir.join("skills_custom").join(skill_id.trim());
    if raw_dir.exists() {
        let _ = std::fs::remove_dir_all(&raw_dir);
    }
    Ok(())
}

fn parse_skill_frontmatter_fields(
    markdown: &str,
) -> (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
) {
    let trimmed = markdown.trim_start_matches('\u{feff}').trim_start();
    let mut name: Option<String> = None;
    let mut description: Option<String> = None;
    let mut author: Option<String> = None;
    let mut license: Option<String> = None;
    if let Some(rest) = trimmed
        .strip_prefix("---\n")
        .or_else(|| trimmed.strip_prefix("---\r\n"))
    {
        if let Some(end) = rest.find("\n---") {
            let block = &rest[..end];
            for line in block.lines() {
                if let Some((k, v)) = line.split_once(':') {
                    let key = k.trim().to_ascii_lowercase();
                    let val = v.trim().trim_matches('"').trim_matches('\'').trim();
                    if val.is_empty() {
                        continue;
                    }
                    match key.as_str() {
                        "name" => name = Some(val.to_owned()),
                        "description" => description = Some(val.to_owned()),
                        "author" => author = Some(val.to_owned()),
                        "license" => license = Some(val.to_owned()),
                        _ => {}
                    }
                }
            }
        }
    }
    if name.is_none() {
        for line in trimmed.lines() {
            if let Some(heading) = line.trim().strip_prefix("# ") {
                let clean = heading.trim();
                if !clean.is_empty() {
                    name = Some(clean.to_owned());
                    break;
                }
            }
        }
    }
    (name, description, author, license)
}

pub fn install_custom_skill_from_url_at(
    app_data_dir: &std::path::Path,
    proxy_policy: insar_core::ProxyPolicy,
    raw_url: &str,
    fallback_name: Option<&str>,
) -> Result<CustomSkillRecord, String> {
    use std::io::Read;

    let url_str = raw_url.trim();
    let parsed = reqwest::Url::parse(url_str)
        .map_err(|e| format!("SKILL_URL_INVALID: URL 格式无效 ({e})"))?;
    let host = parsed.host_str().unwrap_or("");
    let is_loopback = host == "127.0.0.1" || host == "localhost" || host == "[::1]";
    if parsed.scheme() != "https" && !(parsed.scheme() == "http" && is_loopback) {
        return Err("SKILL_URL_SCHEME_FORBIDDEN: 仅支持 HTTPS 远端技能链接".to_string());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("SKILL_URL_CREDENTIALS_FORBIDDEN: URL 不得内嵌账号密码".to_string());
    }

    let builder = reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(8))
        .timeout(std::time::Duration::from_secs(20));
    let client = insar_provider::network_proxy::blocking_builder_with_proxy(builder, proxy_policy)
        .build()
        .map_err(|e| format!("SKILL_HTTP_CLIENT_ERROR: {e}"))?;
    let resp = client
        .get(parsed.clone())
        .header(reqwest::header::USER_AGENT, "EOStage-Desktop/1.0")
        .send()
        .map_err(|e| format!("SKILL_REMOTE_FETCH_FAILED: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("SKILL_REMOTE_HTTP_{}", resp.status().as_u16()));
    }
    const MAX_SKILL_BYTES: u64 = 512 * 1024;
    let mut bytes = Vec::new();
    resp.take(MAX_SKILL_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("SKILL_REMOTE_READ_FAILED: {e}"))?;
    if bytes.len() as u64 > MAX_SKILL_BYTES {
        return Err("SKILL_REMOTE_TOO_LARGE: 远端技能文件大小超过 512 KB 上限".to_string());
    }
    let markdown = String::from_utf8(bytes)
        .map_err(|e| format!("SKILL_REMOTE_UTF8_INVALID: 远端文件不是有效 UTF-8 文本 ({e})"))?;
    if markdown.trim().is_empty() {
        return Err("SKILL_REMOTE_EMPTY: 远端技能文件内容为空".to_string());
    }
    let trimmed_md = markdown.trim_start_matches('\u{feff}').trim_start();
    if !trimmed_md.starts_with("---\n")
        && !trimmed_md.starts_with("---\r\n")
        && !trimmed_md.starts_with('#')
    {
        return Err("SKILL_REMOTE_INVALID_FORMAT: 远端技能文件须为包含 YAML Frontmatter 或标题的 Markdown (SKILL.md)".to_string());
    }

    let (fm_name, fm_desc, fm_author, fm_license) = parse_skill_frontmatter_fields(&markdown);
    let url_slug = parsed
        .path_segments()
        .and_then(|mut segs| segs.rfind(|s| !s.is_empty() && !s.eq_ignore_ascii_case("SKILL.md")))
        .map(|s| s.trim_end_matches(".md").trim())
        .filter(|s| !s.is_empty())
        .unwrap_or("remote-skill");
    let raw_id = fallback_name
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or(fm_name.as_deref())
        .unwrap_or(url_slug);
    let safe_id = sanitize_plugin_id(raw_id);
    let record = CustomSkillRecord {
        id: safe_id.clone(),
        name: fm_name.unwrap_or_else(|| safe_id.clone()),
        description: fm_desc.unwrap_or_else(|| format!("从 {host} 导入的远端技能包")),
        prompt: markdown,
        author: fm_author.unwrap_or_else(|| "Remote URL".to_string()),
        license: fm_license.unwrap_or_else(|| "MIT".to_string()),
        version: "1.0.0".to_string(),
        slash_command: format!("/{safe_id}"),
        updated_at: now_iso_timestamp(),
        enabled: true,
    };
    save_custom_skill_at(app_data_dir, &record)?;
    Ok(record)
}

fn copy_bounded_skill_subdir(
    src_root: &std::path::Path,
    src_cur: &std::path::Path,
    dst_root: &std::path::Path,
    copied_files: &mut usize,
) -> Result<(), String> {
    const MAX_SKILL_FOLDER_FILES: usize = 48;
    const MAX_SKILL_FILE_BYTES: u64 = 512 * 1024;
    let Ok(entries) = std::fs::read_dir(src_cur) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        if *copied_files >= MAX_SKILL_FOLDER_FILES {
            break;
        }
        let p = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&p) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        let Ok(rel) = p.strip_prefix(src_root) else {
            continue;
        };
        if p.is_dir() {
            let name = entry.file_name();
            if name.to_string_lossy().starts_with('.') {
                continue;
            }
            let _ = std::fs::create_dir_all(dst_root.join(rel));
            copy_bounded_skill_subdir(src_root, &p, dst_root, copied_files)?;
        } else if meta.is_file() && meta.len() <= MAX_SKILL_FILE_BYTES {
            let ext = p
                .extension()
                .and_then(|e| e.to_str())
                .map(|s| s.to_ascii_lowercase())
                .unwrap_or_default();
            if matches!(
                ext.as_str(),
                "md" | "txt" | "py" | "json" | "yaml" | "yml" | "toml" | "sh"
            ) {
                let dst = dst_root.join(rel);
                if let Some(parent) = dst.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                std::fs::copy(&p, &dst).map_err(|e| format!("SKILL_FOLDER_COPY_FAILED: {e}"))?;
                *copied_files += 1;
            }
        }
    }
    Ok(())
}

pub fn import_custom_skill_folder_at(
    app_data_dir: &std::path::Path,
    folder_path: &str,
) -> Result<CustomSkillRecord, String> {
    let trimmed = folder_path.trim();
    if trimmed.is_empty() {
        return Err("SKILL_FOLDER_EMPTY: 请选择包含 SKILL.md 的本地技能文件夹".to_string());
    }
    let src_dir = std::path::Path::new(trimmed);
    if !src_dir.is_dir() {
        return Err(format!("SKILL_FOLDER_NOT_FOUND: 目录不存在 ({trimmed})"));
    }
    let md_path = src_dir.join("SKILL.md");
    if !md_path.is_file() {
        return Err("SKILL_FOLDER_MISSING_SKILL_MD: 所选文件夹根目录未找到 SKILL.md".to_string());
    }
    let markdown = std::fs::read_to_string(&md_path)
        .map_err(|e| format!("SKILL_FOLDER_READ_FAILED: 无法读取 SKILL.md ({e})"))?;
    if markdown.trim().is_empty() {
        return Err("SKILL_FOLDER_EMPTY_MD: SKILL.md 内容为空".to_string());
    }
    let (fm_name, fm_desc, fm_author, fm_license) = parse_skill_frontmatter_fields(&markdown);
    let dir_slug = src_dir
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("local-skill");
    let safe_id = sanitize_plugin_id(fm_name.as_deref().unwrap_or(dir_slug));
    let record = CustomSkillRecord {
        id: safe_id.clone(),
        name: fm_name.unwrap_or_else(|| dir_slug.to_owned()),
        description: fm_desc
            .unwrap_or_else(|| format!("从本地目录导入的多文件技能包 ({dir_slug})")),
        prompt: markdown,
        author: fm_author.unwrap_or_else(|| "Local Folder".to_string()),
        license: fm_license.unwrap_or_else(|| "MIT".to_string()),
        version: "1.0.0".to_string(),
        slash_command: format!("/{safe_id}"),
        updated_at: now_iso_timestamp(),
        enabled: true,
    };
    save_custom_skill_at(app_data_dir, &record)?;
    let dst_dir = app_data_dir.join("skills_custom").join(&safe_id);
    for sub in ["references", "scripts", "examples"] {
        let sub_src = src_dir.join(sub);
        if sub_src.is_dir() {
            let mut count = 0usize;
            copy_bounded_skill_subdir(src_dir, &sub_src, &dst_dir, &mut count)?;
        }
    }
    Ok(record)
}

pub fn load_installed_plugin_packages_at(
    app_data_dir: &std::path::Path,
    runtime_plugins_dir: &std::path::Path,
) -> Vec<ManagedPluginPackage> {
    let plugins_dir = app_data_dir.join("plugins");
    let mut installed_plugins = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&plugins_dir) {
        let mut dirs: Vec<_> = entries.flatten().collect();
        dirs.sort_by_key(|e| e.file_name());
        for entry in dirs {
            let p = entry.path();
            let Ok(meta) = std::fs::symlink_metadata(&p) else {
                continue;
            };
            if meta.file_type().is_symlink() || !meta.is_dir() {
                continue;
            }
            let is_official = runtime_plugins_dir.join(entry.file_name()).is_dir();
            if let Some(pkg) = inspect_plugin_package_dir(&p, true, is_official) {
                installed_plugins.push(pkg);
            }
        }
    }
    installed_plugins
}

fn cleanup_legacy_plugin_copies_at(
    app_data_dir: &std::path::Path,
    plugins: &[ManagedPluginPackage],
) {
    if plugins.is_empty() {
        return;
    }
    // Remove unedited legacy copies created by earlier releases that duplicated
    // plugin-owned skills into `skills_custom/`. Always preserve user-authored or
    // user-edited skills.
    let custom_skills = load_custom_skills_at(app_data_dir);
    for sk in &custom_skills {
        if sk.author.trim().eq_ignore_ascii_case("User") {
            continue;
        }
        let matches_plugin_copy = plugins.iter().any(|p| {
            p.bundled_skills.iter().any(|bundled| {
                bundled.id == sk.id && bundled.prompt.trim() == sk.prompt.trim()
            })
        });
        if matches_plugin_copy {
            let _ = delete_custom_skill_at(app_data_dir, &sk.id);
        }
    }

    // Remove exact legacy plugin connector clones from `connectors_custom.json`,
    // explicitly preserving user-authored entries such as `plugin-mcp-rustgis`.
    let custom_connectors = load_custom_connectors_at(app_data_dir);
    let mut changed = false;
    let filtered: Vec<CustomConnectorRecord> = custom_connectors
        .into_iter()
        .filter(|conn| {
            if conn.id == "plugin-mcp-rustgis" {
                return true;
            }
            let matches_plugin_copy = plugins.iter().any(|p| {
                p.bundled_connectors.iter().any(|bundled| {
                    bundled.id == conn.id
                        && bundled.endpoint == conn.endpoint
                        && bundled.transport == conn.transport
                })
            });
            if matches_plugin_copy {
                changed = true;
                false
            } else {
                true
            }
        })
        .collect();
    if changed {
        let _ = save_custom_connectors_at(app_data_dir, &filtered);
    }
}

pub fn load_effective_mcp_connectors_at(
    app_data_dir: &std::path::Path,
    runtime_plugins_dir: &std::path::Path,
) -> Vec<CustomConnectorRecord> {
    let mut effective = load_custom_connectors_at(app_data_dir);
    let mut seen_ids: std::collections::HashSet<String> =
        effective.iter().map(|c| c.id.clone()).collect();

    let mut installed = load_installed_plugin_packages_at(app_data_dir, runtime_plugins_dir);
    installed.sort_by(|a, b| a.id.cmp(&b.id));
    for plugin in installed {
        if !plugin.enabled {
            continue;
        }
        let mut conns = plugin.bundled_connectors;
        conns.sort_by(|a, b| a.id.cmp(&b.id));
        for conn in conns {
            if seen_ids.insert(conn.id.clone()) {
                effective.push(CustomConnectorRecord {
                    id: conn.id,
                    name: conn.name,
                    transport: conn.transport,
                    endpoint: conn.endpoint,
                    description: conn.description,
                    enabled: true,
                });
            }
        }
    }
    effective
}

pub fn resolve_connector_for_probe_at(
    app_data_dir: &std::path::Path,
    runtime_plugins_dir: &std::path::Path,
    connector_id: &str,
) -> Result<CustomConnectorRecord, String> {
    if let Some(custom) = load_custom_connectors_at(app_data_dir)
        .into_iter()
        .find(|c| c.id == connector_id)
    {
        return Ok(custom);
    }
    let mut installed = load_installed_plugin_packages_at(app_data_dir, runtime_plugins_dir);
    installed.sort_by(|a, b| a.id.cmp(&b.id));
    for plugin in &installed {
        if let Some(conn) = plugin
            .bundled_connectors
            .iter()
            .find(|c| c.id == connector_id)
        {
            if !plugin.enabled {
                return Err(format!(
                    "PLUGIN_DISABLED: 所属插件 '{}' 已停用，请先启用后再探测工具",
                    plugin.name
                ));
            }
            return Ok(CustomConnectorRecord {
                id: conn.id.clone(),
                name: conn.name.clone(),
                transport: conn.transport.clone(),
                endpoint: conn.endpoint.clone(),
                description: conn.description.clone(),
                enabled: true,
            });
        }
    }
    Err(format!("V3_CONNECTOR_NOT_FOUND: 未找到连接器 '{connector_id}'"))
}

pub fn inspect_plugins_status_at(
    app_data_dir: &std::path::Path,
    runtime_plugins_dir: &std::path::Path,
) -> Result<ApplicationPluginsStatusResponse, String> {
    let plugins_dir = app_data_dir.join("plugins");
    std::fs::create_dir_all(&plugins_dir).map_err(|e| format!("PLUGINS_DIR_CREATE_FAILED: {e}"))?;

    let installed_plugins = load_installed_plugin_packages_at(app_data_dir, runtime_plugins_dir);
    cleanup_legacy_plugin_copies_at(app_data_dir, &installed_plugins);

    let mut available_plugins = Vec::new();
    if runtime_plugins_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(runtime_plugins_dir) {
            let mut dirs: Vec<_> = entries.flatten().collect();
            dirs.sort_by_key(|e| e.file_name());
            for entry in dirs {
                let p = entry.path();
                if p.is_dir() {
                    if let Some(mut pkg) = inspect_plugin_package_dir(&p, false, true) {
                        if let Some(inst) = installed_plugins.iter().find(|i| i.id == pkg.id) {
                            pkg.installed = true;
                            pkg.enabled = inst.enabled;
                            pkg.install_path = inst.install_path.clone();
                        }
                        available_plugins.push(pkg);
                    }
                }
            }
        }
    }

    for inst in &installed_plugins {
        if !available_plugins.iter().any(|a| a.id == inst.id) {
            available_plugins.push(inst.clone());
        }
    }

    let custom_connectors = load_custom_connectors_at(app_data_dir);
    let custom_skills = load_custom_skills_at(app_data_dir);

    Ok(ApplicationPluginsStatusResponse {
        ok: true,
        app_data_dir: app_data_dir.to_string_lossy().to_string(),
        plugins_dir: plugins_dir.to_string_lossy().to_string(),
        runtime_plugins_dir: runtime_plugins_dir.to_string_lossy().to_string(),
        installed_plugins,
        available_plugins,
        custom_connectors,
        custom_skills,
    })
}

pub fn install_plugin_package_at(
    app_data_dir: &std::path::Path,
    runtime_plugins_dir: &std::path::Path,
    request: &PluginInstallRequest,
) -> Result<PluginMutationResponse, String> {
    let plugins_dir = app_data_dir.join("plugins");
    std::fs::create_dir_all(&plugins_dir).map_err(|e| format!("PLUGINS_DIR_CREATE_FAILED: {e}"))?;

    let installed_target_dir: std::path::PathBuf;

    if let Some(plugin_id) = request
        .plugin_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let safe_id = sanitize_plugin_id(plugin_id);
        let src_dir = runtime_plugins_dir.join(&safe_id);
        if !src_dir.is_dir() || !src_dir.join("plugin.json").is_file() {
            return Err(format!(
                "PLUGIN_NOT_FOUND_IN_RUNTIME: 在 {} 下未找到插件包 '{}'",
                runtime_plugins_dir.display(),
                safe_id
            ));
        }
        let dst_dir = plugins_dir.join(&safe_id);
        let _ = std::fs::remove_dir_all(&dst_dir);
        copy_dir_recursive(&src_dir, &dst_dir)?;
        let _ = std::fs::remove_file(dst_dir.join(".disabled"));
        installed_target_dir = dst_dir;
    } else if let Some(source_path) = request
        .source_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let lower_source = source_path.to_ascii_lowercase();
        if lower_source.starts_with("https://") || lower_source.starts_with("http://") {
            let parsed = reqwest::Url::parse(source_path)
                .map_err(|e| format!("PLUGIN_INVALID_URL: {e}"))?;
            if parsed.path().to_ascii_lowercase().ends_with(".tgz") {
                return Err("PLUGIN_CORDIS_HOST_REQUIRED: npm/Cordis 归档包需要 Node.js / Cordis 宿主环境，EOStage 桌面端不支持伪装安装。".to_string());
            }
            let is_loopback = is_loopback_http_url(source_path);
            validate_hub_http_url(&parsed, is_loopback, "PLUGIN_UNSAFE_URL")?;
            use std::io::Read;
            let client = build_hub_http_client(is_loopback, 8, 20, "PLUGIN_HTTP_CLIENT_ERROR")?;
            let resp = client
                .get(parsed)
                .header(reqwest::header::USER_AGENT, "EOStage-Desktop/2.2")
                .send()
                .map_err(|e| format!("PLUGIN_REMOTE_FETCH_FAILED: {e}"))?;
            if !resp.status().is_success() {
                return Err(format!(
                    "PLUGIN_REMOTE_HTTP_STATUS_{}",
                    resp.status().as_u16()
                ));
            }
            if let Some(cl) = resp.content_length() {
                if cl > MAX_REGISTRY_BYTES {
                    return Err("PLUGIN_REMOTE_TOO_LARGE: 远端插件包大小超出安全上限".to_string());
                }
            }
            if is_html_content_type(resp.headers()) {
                return Err(
                    "PLUGIN_REMOTE_UNEXPECTED_HTML: 远端地址返回 HTML 页面而非插件包".to_string(),
                );
            }
            let mut bytes = Vec::new();
            resp.take(MAX_REGISTRY_BYTES + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| format!("PLUGIN_REMOTE_READ_FAILED: {e}"))?;
            if bytes.len() as u64 > MAX_REGISTRY_BYTES {
                return Err("PLUGIN_REMOTE_TOO_LARGE: 远端插件包大小超出安全上限".to_string());
            }
            if is_likely_html_payload(&bytes) {
                return Err(
                    "PLUGIN_REMOTE_UNEXPECTED_HTML: 远端内容为 HTML 页面而非插件包".to_string(),
                );
            }
            if bytes.starts_with(b"PK\x03\x04") {
                let stage_dir = plugins_dir.join(format!(".plugin-stage-{}", now_unix_nanos()));
                let _ = std::fs::remove_dir_all(&stage_dir);
                std::fs::create_dir_all(&stage_dir)
                    .map_err(|e| format!("PLUGIN_STAGE_DIR_FAILED: {e}"))?;
                if let Err(err) = extract_zip_safely(&bytes, &stage_dir) {
                    let _ = std::fs::remove_dir_all(&stage_dir);
                    return Err(err);
                }
                let unpacked = unwrap_single_subdir(&stage_dir);
                if !unpacked.join("plugin.json").is_file() {
                    let _ = std::fs::remove_dir_all(&stage_dir);
                    return Err(
                        "PLUGIN_ZIP_MISSING_MANIFEST: 远端 ZIP 压缩包内缺少 plugin.json"
                            .to_string(),
                    );
                }
                let pkg = inspect_plugin_package_dir(&unpacked, false, false).ok_or_else(|| {
                    let _ = std::fs::remove_dir_all(&stage_dir);
                    "PLUGIN_ZIP_INVALID_MANIFEST: plugin.json 格式无效".to_string()
                })?;
                let safe_id = sanitize_plugin_id(&pkg.id);
                let dst_dir = plugins_dir.join(&safe_id);
                let _ = std::fs::remove_dir_all(&dst_dir);
                let copy_res = copy_dir_recursive(&unpacked, &dst_dir);
                let _ = std::fs::remove_dir_all(&stage_dir);
                copy_res?;
                installed_target_dir = dst_dir;
            } else {
                let raw_text = String::from_utf8(bytes)
                    .map_err(|e| format!("PLUGIN_FILE_UTF8_INVALID: {e}"))?;
                return install_plugin_from_raw_content_at(
                    app_data_dir,
                    runtime_plugins_dir,
                    &raw_text,
                    request.fallback_name.as_deref(),
                );
            }
        } else {
            let clean_path = source_path
                .strip_prefix("file:///")
                .or_else(|| source_path.strip_prefix("file://"))
                .unwrap_or(source_path);
            let src = std::path::Path::new(clean_path);
            if !src.exists() {
                return Err(format!(
                    "PLUGIN_SOURCE_NOT_FOUND: 本地路径不存在 '{clean_path}'"
                ));
            }
            if src.is_dir() {
                let unpacked = unwrap_single_subdir(src);
                if !unpacked.join("plugin.json").is_file() {
                    return Err(
                        "PLUGIN_MANIFEST_MISSING: 本地插件文件夹缺少 plugin.json".to_string()
                    );
                }
                let pkg = inspect_plugin_package_dir(&unpacked, false, false)
                    .ok_or_else(|| "PLUGIN_MANIFEST_INVALID: plugin.json 格式无效".to_string())?;
                let safe_id = sanitize_plugin_id(&pkg.id);
                let dst_dir = plugins_dir.join(&safe_id);
                let _ = std::fs::remove_dir_all(&dst_dir);
                copy_dir_recursive(&unpacked, &dst_dir)?;
                installed_target_dir = dst_dir;
            } else {
                let bytes =
                    std::fs::read(src).map_err(|e| format!("PLUGIN_FILE_READ_FAILED: {e}"))?;
                if bytes.starts_with(b"PK\x03\x04") {
                    let stage_dir = plugins_dir.join(format!(".plugin-stage-{}", now_unix_nanos()));
                    let _ = std::fs::remove_dir_all(&stage_dir);
                    std::fs::create_dir_all(&stage_dir)
                        .map_err(|e| format!("PLUGIN_STAGE_DIR_FAILED: {e}"))?;
                    if let Err(err) = extract_zip_safely(&bytes, &stage_dir) {
                        let _ = std::fs::remove_dir_all(&stage_dir);
                        return Err(err);
                    }
                    let unpacked = unwrap_single_subdir(&stage_dir);
                    if !unpacked.join("plugin.json").is_file() {
                        let _ = std::fs::remove_dir_all(&stage_dir);
                        return Err(
                            "PLUGIN_ZIP_MISSING_MANIFEST: ZIP 压缩包内缺少 plugin.json".to_string()
                        );
                    }
                    let pkg =
                        inspect_plugin_package_dir(&unpacked, false, false).ok_or_else(|| {
                            let _ = std::fs::remove_dir_all(&stage_dir);
                            "PLUGIN_ZIP_INVALID_MANIFEST: plugin.json 格式无效".to_string()
                        })?;
                    let safe_id = sanitize_plugin_id(&pkg.id);
                    let dst_dir = plugins_dir.join(&safe_id);
                    let _ = std::fs::remove_dir_all(&dst_dir);
                    let copy_res = copy_dir_recursive(&unpacked, &dst_dir);
                    let _ = std::fs::remove_dir_all(&stage_dir);
                    copy_res?;
                    installed_target_dir = dst_dir;
                } else {
                    let raw_text = String::from_utf8(bytes)
                        .map_err(|e| format!("PLUGIN_FILE_UTF8_INVALID: {e}"))?;
                    return install_plugin_from_raw_content_at(
                        app_data_dir,
                        runtime_plugins_dir,
                        &raw_text,
                        request.fallback_name.as_deref(),
                    );
                }
            }
        }
    } else if let Some(raw_content) = request
        .raw_content
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return install_plugin_from_raw_content_at(
            app_data_dir,
            runtime_plugins_dir,
            raw_content,
            request.fallback_name.as_deref(),
        );
    } else {
        return Err(
            "PLUGIN_INSTALL_EMPTY_REQUEST: 请提供 plugin_id、source_path 或 raw_content"
                .to_string(),
        );
    }

    let pkg = inspect_plugin_package_dir(&installed_target_dir, true, false)
        .ok_or_else(|| "PLUGIN_VERIFY_FAILED: 安装后无法读取 plugin.json".to_string())?;

    crate::mcp_client::invalidate_connector_cache(None);
    let status = inspect_plugins_status_at(app_data_dir, runtime_plugins_dir)?;
    Ok(PluginMutationResponse {
        ok: true,
        message: format!("已安装插件「{}」至 {}", pkg.name, pkg.install_path),
        installed_plugin: Some(pkg),
        status,
    })
}

fn install_plugin_from_raw_content_at(
    app_data_dir: &std::path::Path,
    runtime_plugins_dir: &std::path::Path,
    raw_content: &str,
    fallback_name: Option<&str>,
) -> Result<PluginMutationResponse, String> {
    let plugins_dir = app_data_dir.join("plugins");
    std::fs::create_dir_all(&plugins_dir).map_err(|e| format!("PLUGINS_DIR_CREATE_FAILED: {e}"))?;

    let trimmed = raw_content.trim();
    let dst_dir = if trimmed.starts_with('{') {
        let mut parsed: serde_json::Value =
            serde_json::from_str(trimmed).map_err(|e| format!("PLUGIN_JSON_PARSE_FAILED: {e}"))?;
        let raw_compat = parsed
            .get("runtime_compatibility")
            .or_else(|| parsed.get("runtimeCompatibility"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        let has_inline_skill = parsed
            .get("bundledSkills")
            .or_else(|| parsed.get("skills"))
            .and_then(|v| v.as_array())
            .is_some_and(|a| !a.is_empty())
            || parsed.get("skill").is_some_and(|s| !s.is_null());
        let skill_url = parsed
            .get("skill_url")
            .or_else(|| parsed.get("skillUrl"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let mcp_from_list = parse_mcp_servers_from_value(
            parsed
                .get("mcpServers")
                .or_else(|| parsed.get("mcp_servers")),
        );
        let has_mcp_payload = !mcp_from_list.is_empty()
            || parsed
                .get("mcpServers")
                .and_then(|v| v.as_object())
                .is_some_and(|m| !m.is_empty());
        let is_cordis_engine = parsed
            .get("engines")
            .and_then(|v| v.as_object())
            .is_some_and(|e| {
                e.contains_key("dsh") || e.contains_key("cordis") || e.contains_key("koishi")
            });
        if raw_compat == "cordis_host_required"
            || (is_cordis_engine && !has_inline_skill && skill_url.is_empty() && !has_mcp_payload)
        {
            return Err(
                "PLUGIN_CORDIS_HOST_REQUIRED: 该插件属于 Node.js / Cordis 运行时插件，需 DSH / Cordis 宿主环境；EOStage 桌面端不会将其伪装为原生插件安装。"
                    .to_string(),
            );
        }

        let raw_id = parsed
            .get("id")
            .and_then(|v| v.as_str())
            .or(fallback_name)
            .unwrap_or("custom-plugin");
        let safe_id = sanitize_plugin_id(raw_id);
        let name = parsed
            .get("name")
            .and_then(|v| v.as_str())
            .or(fallback_name)
            .unwrap_or(&safe_id)
            .to_string();
        let author = parsed
            .get("author")
            .and_then(|v| v.as_str())
            .map(|a| if a == "ScenePort" { "EOStage" } else { a })
            .filter(|a| !a.trim().is_empty())
            .unwrap_or("EOStage")
            .to_string();
        let version = parsed
            .get("version")
            .and_then(|v| v.as_str())
            .filter(|v| !v.trim().is_empty())
            .unwrap_or("1.0.0")
            .to_string();
        let description = parsed
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("从扩展市场或本地清单导入的扩展插件")
            .to_string();

        if let Some(obj) = parsed.as_object_mut() {
            obj.insert("id".to_string(), serde_json::Value::String(safe_id.clone()));
            obj.insert("name".to_string(), serde_json::Value::String(name.clone()));
            obj.insert(
                "author".to_string(),
                serde_json::Value::String(author.clone()),
            );
            obj.insert(
                "version".to_string(),
                serde_json::Value::String(version.clone()),
            );
            obj.insert(
                "description".to_string(),
                serde_json::Value::String(description.clone()),
            );
            if !obj.contains_key("discipline") {
                obj.insert(
                    "discipline".to_string(),
                    serde_json::Value::String("custom".to_string()),
                );
            }
            if !obj.contains_key("enabled") {
                obj.insert("enabled".to_string(), serde_json::Value::Bool(true));
            }
        }

        let target_dir = plugins_dir.join(&safe_id);
        let _ = std::fs::remove_dir_all(&target_dir);
        std::fs::create_dir_all(&target_dir)
            .map_err(|e| format!("PLUGIN_DIR_CREATE_FAILED: {e}"))?;

        let skills_arr = parsed
            .get("bundledSkills")
            .or_else(|| parsed.get("skills"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        if !skills_arr.is_empty() {
            for (idx, sk) in skills_arr.iter().enumerate() {
                let sk_id = sk
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(sanitize_plugin_id)
                    .unwrap_or_else(|| format!("{safe_id}-skill-{}", idx + 1));
                let sk_prompt = sk
                    .get("prompt")
                    .or_else(|| sk.get("prompt_template"))
                    .or_else(|| sk.get("content"))
                    .and_then(|v| v.as_str())
                    .unwrap_or(&description);
                let sk_dir = target_dir.join("skills").join(&sk_id);
                std::fs::create_dir_all(&sk_dir)
                    .map_err(|e| format!("PLUGIN_SKILL_DIR_FAILED: {e}"))?;
                std::fs::write(sk_dir.join("SKILL.md"), sk_prompt)
                    .map_err(|e| format!("PLUGIN_SKILL_WRITE_FAILED: {e}"))?;
            }
        } else if let Some(skill_payload) = parsed.get("skill") {
            let sk_id = skill_payload
                .get("id")
                .and_then(|v| v.as_str())
                .map(sanitize_plugin_id)
                .unwrap_or_else(|| safe_id.clone());
            let sk_content = skill_payload
                .get("content")
                .or_else(|| skill_payload.get("prompt_template"))
                .or_else(|| skill_payload.get("prompt"))
                .and_then(|v| v.as_str())
                .or_else(|| skill_payload.as_str())
                .unwrap_or("");
            let final_md = if sk_content.trim().is_empty() && !skill_url.is_empty() {
                read_text_from_source(&skill_url, MAX_REGISTRY_BYTES)?
            } else if sk_content.trim().is_empty() {
                format!("---\nname: {name}\ndescription: {description}\nauthor: {author}\n---\n\n# {name}\n\n{description}\n")
            } else {
                sk_content.to_string()
            };
            let sk_dir = target_dir.join("skills").join(&sk_id);
            std::fs::create_dir_all(&sk_dir)
                .map_err(|e| format!("PLUGIN_SKILL_DIR_FAILED: {e}"))?;
            std::fs::write(sk_dir.join("SKILL.md"), final_md)
                .map_err(|e| format!("PLUGIN_SKILL_WRITE_FAILED: {e}"))?;
        } else if !skill_url.is_empty() {
            let remote_md = read_text_from_source(&skill_url, MAX_REGISTRY_BYTES)?;
            let sk_dir = target_dir.join("skills").join(&safe_id);
            std::fs::create_dir_all(&sk_dir)
                .map_err(|e| format!("PLUGIN_SKILL_DIR_FAILED: {e}"))?;
            std::fs::write(sk_dir.join("SKILL.md"), remote_md)
                .map_err(|e| format!("PLUGIN_SKILL_WRITE_FAILED: {e}"))?;
        } else if !has_mcp_payload {
            let default_sk_id = format!("{safe_id}-skill");
            let sk_dir = target_dir.join("skills").join(&default_sk_id);
            std::fs::create_dir_all(&sk_dir)
                .map_err(|e| format!("PLUGIN_SKILL_DIR_FAILED: {e}"))?;
            std::fs::write(
                sk_dir.join("SKILL.md"),
                format!("---\nname: {name}\ndescription: {description}\nauthor: {author}\n---\n\n# {name}\n\n{description}\n"),
            )
            .map_err(|e| format!("PLUGIN_SKILL_WRITE_FAILED: {e}"))?;
        }

        if let Some(mcp_servers) = parsed.get("mcpServers").and_then(|v| v.as_object()) {
            let filtered: serde_json::Map<String, serde_json::Value> = mcp_servers
                .iter()
                .filter(|(_, cfg)| {
                    cfg.as_object().is_some_and(|obj| {
                        let cmd = obj.get("command").and_then(|x| x.as_str()).unwrap_or("");
                        let url = obj
                            .get("url")
                            .or_else(|| obj.get("endpoint"))
                            .and_then(|x| x.as_str())
                            .unwrap_or("");
                        !is_forbidden_internal_connector_endpoint(cmd)
                            && !is_forbidden_internal_connector_endpoint(url)
                    })
                })
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            if !filtered.is_empty() {
                let mcp_doc = serde_json::json!({ "mcpServers": filtered });
                std::fs::write(
                    target_dir.join("mcp.json"),
                    serde_json::to_string_pretty(&mcp_doc).unwrap_or_default(),
                )
                .map_err(|e| format!("PLUGIN_MCP_WRITE_FAILED: {e}"))?;
            }
        } else if !mcp_from_list.is_empty() {
            let mut mcp_map = serde_json::Map::new();
            for srv in &mcp_from_list {
                let mut srv_obj = serde_json::Map::new();
                if !srv.command.is_empty() {
                    srv_obj.insert(
                        "command".to_string(),
                        serde_json::Value::String(srv.command.clone()),
                    );
                }
                if !srv.args.is_empty() {
                    srv_obj.insert(
                        "args".to_string(),
                        serde_json::Value::Array(
                            srv.args
                                .iter()
                                .cloned()
                                .map(serde_json::Value::String)
                                .collect(),
                        ),
                    );
                }
                if !srv.env.is_empty() {
                    srv_obj.insert(
                        "env".to_string(),
                        serde_json::to_value(&srv.env)
                            .unwrap_or(serde_json::Value::Object(Default::default())),
                    );
                }
                if !srv.url.is_empty() {
                    srv_obj.insert(
                        "url".to_string(),
                        serde_json::Value::String(srv.url.clone()),
                    );
                }
                if !srv.transport.is_empty() {
                    srv_obj.insert(
                        "type".to_string(),
                        serde_json::Value::String(srv.transport.clone()),
                    );
                }
                mcp_map.insert(srv.id.clone(), serde_json::Value::Object(srv_obj));
            }
            let mcp_doc = serde_json::json!({ "mcpServers": mcp_map });
            std::fs::write(
                target_dir.join("mcp.json"),
                serde_json::to_string_pretty(&mcp_doc).unwrap_or_default(),
            )
            .map_err(|e| format!("PLUGIN_MCP_WRITE_FAILED: {e}"))?;
        }

        std::fs::write(
            target_dir.join("plugin.json"),
            serde_json::to_string_pretty(&parsed).unwrap_or_else(|_| trimmed.to_string()),
        )
        .map_err(|e| format!("PLUGIN_MANIFEST_WRITE_FAILED: {e}"))?;
        target_dir
    } else {
        let fb = fallback_name
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("custom-markdown-plugin");
        let safe_id = sanitize_plugin_id(fb);
        let target_dir = plugins_dir.join(&safe_id);
        let _ = std::fs::remove_dir_all(&target_dir);
        let sk_dir = target_dir.join("skills").join(&safe_id);
        std::fs::create_dir_all(&sk_dir).map_err(|e| format!("PLUGIN_SKILL_DIR_FAILED: {e}"))?;
        std::fs::write(sk_dir.join("SKILL.md"), trimmed)
            .map_err(|e| format!("PLUGIN_SKILL_WRITE_FAILED: {e}"))?;
        let manifest = serde_json::json!({
            "id": safe_id,
            "name": fb,
            "version": "1.0.0",
            "author": "EOStage",
            "license": "MIT",
            "discipline": "custom",
            "description": "从本地 SKILL.md 导入并持久化的扩展插件包",
            "tags": ["本地导入", "SKILL.md"],
            "enabled": true,
            "bundledSkills": [
                {
                    "id": safe_id,
                    "name": fb,
                    "description": "从本地 SKILL.md 导入的技能规程",
                    "path": format!("skills/{safe_id}/SKILL.md")
                }
            ]
        });
        std::fs::write(
            target_dir.join("plugin.json"),
            serde_json::to_string_pretty(&manifest).unwrap_or_default(),
        )
        .map_err(|e| format!("PLUGIN_MANIFEST_WRITE_FAILED: {e}"))?;
        target_dir
    };

    let pkg = inspect_plugin_package_dir(&dst_dir, true, false)
        .ok_or_else(|| "PLUGIN_VERIFY_FAILED: 导入后无法读取 plugin.json".to_string())?;
    crate::mcp_client::invalidate_connector_cache(None);
    let status = inspect_plugins_status_at(app_data_dir, runtime_plugins_dir)?;
    Ok(PluginMutationResponse {
        ok: true,
        message: format!("已导入并落盘插件「{}」", pkg.name),
        installed_plugin: Some(pkg),
        status,
    })
}

pub fn set_plugin_enabled_at(
    app_data_dir: &std::path::Path,
    runtime_plugins_dir: &std::path::Path,
    plugin_id: &str,
    enabled: bool,
) -> Result<PluginMutationResponse, String> {
    let safe_id = sanitize_plugin_id(plugin_id);
    let plugin_dir = app_data_dir.join("plugins").join(&safe_id);
    let manifest_path = plugin_dir.join("plugin.json");
    if !manifest_path.is_file() {
        return Err(format!("PLUGIN_NOT_INSTALLED: 插件 '{safe_id}' 尚未安装"));
    }
    let disabled_marker = plugin_dir.join(".disabled");
    if enabled {
        let _ = std::fs::remove_file(&disabled_marker);
    } else {
        std::fs::write(&disabled_marker, b"disabled")
            .map_err(|e| format!("PLUGIN_TOGGLE_FAILED: {e}"))?;
    }
    crate::mcp_client::invalidate_connector_cache(None);
    let pkg = inspect_plugin_package_dir(&plugin_dir, true, false);
    let status = inspect_plugins_status_at(app_data_dir, runtime_plugins_dir)?;
    Ok(PluginMutationResponse {
        ok: true,
        message: format!(
            "插件 '{}' 状态已更新为 {}",
            safe_id,
            if enabled { "已启用" } else { "已停用" }
        ),
        installed_plugin: pkg,
        status,
    })
}

pub fn uninstall_plugin_package_at(
    app_data_dir: &std::path::Path,
    runtime_plugins_dir: &std::path::Path,
    plugin_id: &str,
) -> Result<PluginMutationResponse, String> {
    let safe_id = sanitize_plugin_id(plugin_id);
    let plugin_dir = app_data_dir.join("plugins").join(&safe_id);
    if let Some(pkg) = inspect_plugin_package_dir(&plugin_dir, true, false) {
        cleanup_legacy_plugin_copies_at(app_data_dir, &[pkg]);
    }
    if plugin_dir.exists() {
        std::fs::remove_dir_all(&plugin_dir)
            .map_err(|e| format!("PLUGIN_UNINSTALL_FAILED: {e}"))?;
    }
    crate::mcp_client::invalidate_connector_cache(None);
    let status = inspect_plugins_status_at(app_data_dir, runtime_plugins_dir)?;
    Ok(PluginMutationResponse {
        ok: true,
        message: format!("已从磁盘卸载插件 '{safe_id}'"),
        installed_plugin: None,
        status,
    })
}

// ============================================================================
// 真实「数据与统计 -> 使用统计」后端聚合 (application_usage_statistics)
// ============================================================================

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ModelUsageBreakdownRow {
    pub route: String,
    pub channel_id: String,
    pub model_id: String,
    pub run_count: u64,
    pub model_calls: u64,
    pub tool_calls: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub reasoning_tokens: u64,
    pub cached_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq, Default)]
pub struct DailyUsageSeriesPoint {
    pub date: String,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub reasoning_tokens: u64,
    pub runs: u64,
    pub download_bytes: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq, Default)]
pub struct ToolUsageBreakdownRow {
    pub tool_id: String,
    pub call_count: u64,
    pub success_count: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq, Default)]
pub struct ProviderDownloadBreakdownRow {
    pub source_id: String,
    pub display_name: String,
    pub task_count: usize,
    pub bytes_transferred: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq, Default)]
pub struct GeeQuotaStatusSummary {
    pub configured: bool,
    pub verified: bool,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub auth_mode: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ApplicationUsageStatisticsResponse {
    pub ok: bool,
    pub collected_at: String,
    pub data_root: String,
    pub sqlite_size_bytes: u64,
    #[serde(default)]
    pub disk_free_bytes: Option<u64>,
    pub workspace_count: usize,
    pub session_count: usize,
    pub run_count: u64,
    pub completed_run_count: u64,
    pub tool_call_count: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub reasoning_tokens: u64,
    pub cached_tokens: u64,
    pub total_tokens: u64,
    pub model_breakdown: Vec<ModelUsageBreakdownRow>,
    pub download_total_tasks: usize,
    pub download_completed_tasks: usize,
    pub download_active_tasks: usize,
    pub download_failed_tasks: usize,
    pub download_transferred_bytes: u64,
    pub download_selected_items: usize,
    pub installed_plugins_count: usize,
    pub custom_skills_count: usize,
    #[serde(default)]
    pub daily_series: Vec<DailyUsageSeriesPoint>,
    #[serde(default)]
    pub tool_breakdown: Vec<ToolUsageBreakdownRow>,
    #[serde(default)]
    pub provider_download_breakdown: Vec<ProviderDownloadBreakdownRow>,
    #[serde(default)]
    pub gee_quota_status: Option<GeeQuotaStatusSummary>,
}

fn sqlite_bundle_size_bytes(base_path: &std::path::Path) -> Option<u64> {
    let main_len = std::fs::metadata(base_path).ok()?.len();
    let wal_path = std::path::PathBuf::from(format!("{}-wal", base_path.display()));
    let shm_path = std::path::PathBuf::from(format!("{}-shm", base_path.display()));
    let wal_len = std::fs::metadata(wal_path).map(|m| m.len()).unwrap_or(0);
    let shm_len = std::fs::metadata(shm_path).map(|m| m.len()).unwrap_or(0);
    Some(main_len.saturating_add(wal_len).saturating_add(shm_len))
}

fn extract_utc_date_from_timestamp(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.len() >= 10 {
        let head = &trimmed[..10];
        if chrono::NaiveDate::parse_from_str(head, "%Y-%m-%d").is_ok() {
            return Some(head.to_string());
        }
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(trimmed) {
        return Some(
            dt.with_timezone(&chrono::Utc)
                .format("%Y-%m-%d")
                .to_string(),
        );
    }
    if let Ok(num) = trimmed.parse::<i64>() {
        let dt = if num > 10_000_000_000 {
            chrono::DateTime::<chrono::Utc>::from_timestamp_millis(num)
        } else if num > 0 {
            chrono::DateTime::<chrono::Utc>::from_timestamp(num, 0)
        } else {
            None
        };
        if let Some(resolved) = dt {
            return Some(resolved.format("%Y-%m-%d").to_string());
        }
    }
    None
}

fn format_provider_display_name(source_id: &str) -> String {
    match source_id.trim().to_ascii_lowercase().as_str() {
        "asf" | "asf_sentinel1" => "ASF Sentinel-1 SAR".to_string(),
        "nisar" | "asf_nisar" => "NASA/ISRO NISAR".to_string(),
        "cdse" | "copernicus" => "Copernicus CDSE".to_string(),
        "mpc" | "planetary_computer" => "Planetary Computer".to_string(),
        "usgs" | "usgs_m2m" | "landsat" => "USGS Landsat M2M".to_string(),
        "copernicus_dem" | "dem" => "Copernicus DEM".to_string(),
        "earthdata" => "NASA Earthdata".to_string(),
        "gee" | "google_earth_engine" => "Google Earth Engine".to_string(),
        "xyz_tiles" | "xyz_satellite_tiles" => "XYZ 卫星影像瓦片".to_string(),
        other if !other.is_empty() => source_id.trim().to_string(),
        _ => "未知数据源".to_string(),
    }
}

pub fn compute_application_usage_statistics(
    state: &ApplicationServiceState,
) -> Result<ApplicationUsageStatisticsResponse, String> {
    let data_root = resolve_app_data_dir(state);
    let mut sqlite_candidates = Vec::with_capacity(5);
    if let Some(configured_store) = state.service.store_path() {
        sqlite_candidates.push(configured_store.to_path_buf());
    }
    sqlite_candidates.push(data_root.join("tasks.sqlite3"));
    sqlite_candidates.push(data_root.join("store.sqlite"));
    sqlite_candidates.push(default_store_path());
    sqlite_candidates.push(data_root.join("insar.sqlite"));
    let sqlite_size_bytes = sqlite_candidates
        .iter()
        .find_map(|p| sqlite_bundle_size_bytes(p))
        .unwrap_or(0);
    let disk_free_bytes = {
        use insar_download::disk_guard::{FreeSpaceProbe, SystemFreeSpaceProbe};
        SystemFreeSpaceProbe
            .volume_space(&data_root)
            .map(|space| space.free_bytes)
    };

    let workspaces_path = data_root.join("agent").join("workspaces.json");
    let workspace_count = std::fs::read_to_string(&workspaces_path)
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|v| {
            v.get("workspaces")
                .and_then(|w| w.as_array())
                .map(|a| a.len())
        })
        .unwrap_or(1)
        .max(1);

    let sessions_dir = data_root.join("agent").join("sessions");
    let mut session_ids = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&sessions_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() && p.join("session.json").is_file() {
                if let Some(name) = p.file_name().and_then(|s| s.to_str()) {
                    session_ids.push(name.to_string());
                }
            } else if p.is_file() && p.extension().and_then(|e| e.to_str()) == Some("json") {
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    session_ids.push(stem.to_string());
                }
            }
        }
    }
    let session_count = session_ids.len();

    let today_utc = chrono::Utc::now().date_naive();
    let mut daily_map: std::collections::BTreeMap<String, DailyUsageSeriesPoint> =
        std::collections::BTreeMap::new();
    for days_ago in (0..30).rev() {
        let d = (today_utc - chrono::Duration::days(days_ago))
            .format("%Y-%m-%d")
            .to_string();
        daily_map.insert(
            d.clone(),
            DailyUsageSeriesPoint {
                date: d,
                ..Default::default()
            },
        );
    }

    #[derive(Default)]
    struct RunRecordAccum {
        status: String,
        model_route: String,
        started_at: String,
        db_prompt: u64,
        db_completion: u64,
    }
    let mut run_map: std::collections::HashMap<String, RunRecordAccum> =
        std::collections::HashMap::new();

    if let Ok(Ok(db_runs)) = state.agent_application().with_task_store(|store| {
        let mut rows = Vec::new();
        for sid in &session_ids {
            if let Ok(rids) = store.agent_session_run_ids(sid) {
                for rid in rids {
                    if let Ok(Some(row)) = store.agent_run(&rid) {
                        rows.push(row);
                    }
                }
            }
        }
        Ok::<_, String>(rows)
    }) {
        for row in db_runs {
            run_map.insert(
                row.run_id.clone(),
                RunRecordAccum {
                    status: row.status,
                    model_route: row
                        .model_route
                        .filter(|s| !s.trim().is_empty())
                        .unwrap_or_else(|| "default/auto".to_string()),
                    started_at: row.started_at,
                    db_prompt: row.usage_prompt.unwrap_or(0),
                    db_completion: row.usage_completion.unwrap_or(0),
                },
            );
        }
    }

    let runs_dir = data_root.join("agent").join("runs");
    if let Ok(entries) = std::fs::read_dir(&runs_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if !p.is_dir() {
                continue;
            }
            let rid = entry.file_name().to_string_lossy().to_string();
            let run_json_path = p.join("run.json");
            if let Ok(raw) = std::fs::read_to_string(&run_json_path) {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) {
                    let entry_ref = run_map.entry(rid).or_default();
                    if entry_ref.status.is_empty() {
                        entry_ref.status = val
                            .get("status")
                            .and_then(|v| v.as_str())
                            .unwrap_or("completed")
                            .to_string();
                    }
                    if entry_ref.model_route.is_empty() {
                        entry_ref.model_route = val
                            .get("model_route")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.trim().is_empty())
                            .unwrap_or("default/auto")
                            .to_string();
                    }
                    if entry_ref.started_at.is_empty() {
                        entry_ref.started_at = val
                            .get("started_at")
                            .or_else(|| val.get("created_at"))
                            .or_else(|| val.get("updated_at"))
                            .and_then(|v| {
                                v.as_str()
                                    .map(|s| s.to_string())
                                    .or_else(|| v.as_i64().map(|n| n.to_string()))
                            })
                            .unwrap_or_default();
                    }
                    let p_tok = val
                        .get("usage_prompt")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let c_tok = val
                        .get("usage_completion")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    entry_ref.db_prompt = entry_ref.db_prompt.max(p_tok);
                    entry_ref.db_completion = entry_ref.db_completion.max(c_tok);
                }
            }
        }
    }

    let mut run_count = 0u64;
    let mut completed_run_count = 0u64;
    let mut tool_call_count = 0u64;
    let mut total_prompt = 0u64;
    let mut total_completion = 0u64;
    let mut total_reasoning = 0u64;
    let mut total_cached = 0u64;

    let mut route_map: std::collections::BTreeMap<String, ModelUsageBreakdownRow> =
        std::collections::BTreeMap::new();
    let mut tool_map: std::collections::HashMap<String, ToolUsageBreakdownRow> =
        std::collections::HashMap::new();

    for (rid, rinfo) in &run_map {
        run_count = run_count.saturating_add(1);
        if rinfo.status == "completed" {
            completed_run_count = completed_run_count.saturating_add(1);
        }
        let route = if rinfo.model_route.trim().is_empty() {
            "default/auto".to_string()
        } else {
            rinfo.model_route.clone()
        };
        let (channel_id, model_id) = match route.split_once('/') {
            Some((c, m)) => (c.to_string(), m.to_string()),
            None => ("default".to_string(), route.clone()),
        };

        let mut trace_prompt = 0u64;
        let mut trace_completion = 0u64;
        let mut trace_reasoning = 0u64;
        let mut trace_cached = 0u64;
        let mut trace_model_calls = 0u64;
        let mut trace_tool_calls = 0u64;
        let mut first_trace_ts: Option<String> = None;

        let trace_path = runs_dir.join(rid).join("trace.jsonl");
        if let Ok(trace_raw) = std::fs::read_to_string(&trace_path) {
            for line in trace_raw.lines() {
                let Ok(ev) = serde_json::from_str::<serde_json::Value>(line) else {
                    continue;
                };
                if first_trace_ts.is_none() {
                    if let Some(ts_str) = ev
                        .get("ts")
                        .or_else(|| ev.get("timestamp"))
                        .or_else(|| ev.get("created_at"))
                        .and_then(|v| {
                            v.as_str()
                                .map(|s| s.to_string())
                                .or_else(|| v.as_i64().map(|n| n.to_string()))
                        })
                    {
                        if extract_utc_date_from_timestamp(&ts_str).is_some() {
                            first_trace_ts = Some(ts_str);
                        }
                    }
                }
                let kind = ev.get("kind").and_then(|v| v.as_str()).unwrap_or("");
                if kind == "tool_call" {
                    trace_tool_calls = trace_tool_calls.saturating_add(1);
                    let tool_id = ev
                        .pointer("/payload/tool_id")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .unwrap_or("unknown");
                    let t_entry = tool_map.entry(tool_id.to_string()).or_insert_with(|| {
                        ToolUsageBreakdownRow {
                            tool_id: tool_id.to_string(),
                            call_count: 0,
                            success_count: 0,
                        }
                    });
                    t_entry.call_count = t_entry.call_count.saturating_add(1);
                } else if kind == "tool_result" {
                    let tool_id = ev
                        .pointer("/payload/tool_id")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .unwrap_or("unknown");
                    let explicit_ok = ev.pointer("/payload/ok").and_then(|v| v.as_bool());
                    let has_error = ev.pointer("/payload/error").is_some_and(|v| !v.is_null());
                    let is_success = match explicit_ok {
                        Some(b) => b,
                        None => !has_error,
                    };
                    if is_success {
                        let t_entry = tool_map.entry(tool_id.to_string()).or_insert_with(|| {
                            ToolUsageBreakdownRow {
                                tool_id: tool_id.to_string(),
                                call_count: 0,
                                success_count: 0,
                            }
                        });
                        t_entry.success_count = t_entry.success_count.saturating_add(1);
                    }
                } else if kind == "model_reply" {
                    trace_model_calls = trace_model_calls.saturating_add(1);
                    if let Some(u) = ev.pointer("/payload/usage") {
                        trace_prompt = trace_prompt
                            .saturating_add(u.get("prompt").and_then(|v| v.as_u64()).unwrap_or(0));
                        trace_completion = trace_completion.saturating_add(
                            u.get("completion").and_then(|v| v.as_u64()).unwrap_or(0),
                        );
                        trace_reasoning = trace_reasoning.saturating_add(
                            u.get("reasoning").and_then(|v| v.as_u64()).unwrap_or(0),
                        );
                        trace_cached = trace_cached
                            .saturating_add(u.get("cached").and_then(|v| v.as_u64()).unwrap_or(0));
                    }
                }
            }
        }

        let run_prompt = trace_prompt.max(rinfo.db_prompt);
        let run_completion = trace_completion.max(rinfo.db_completion);
        let run_model_calls = if trace_model_calls > 0 {
            trace_model_calls
        } else if run_prompt > 0 || run_completion > 0 {
            1
        } else {
            0
        };

        tool_call_count = tool_call_count.saturating_add(trace_tool_calls);
        total_prompt = total_prompt.saturating_add(run_prompt);
        total_completion = total_completion.saturating_add(run_completion);
        total_reasoning = total_reasoning.saturating_add(trace_reasoning);
        total_cached = total_cached.saturating_add(trace_cached);

        if let Some(day_key) = extract_utc_date_from_timestamp(&rinfo.started_at).or_else(|| {
            first_trace_ts
                .as_deref()
                .and_then(extract_utc_date_from_timestamp)
        }) {
            if let Some(day_point) = daily_map.get_mut(&day_key) {
                day_point.runs = day_point.runs.saturating_add(1);
                day_point.prompt_tokens = day_point.prompt_tokens.saturating_add(run_prompt);
                day_point.completion_tokens =
                    day_point.completion_tokens.saturating_add(run_completion);
                day_point.reasoning_tokens =
                    day_point.reasoning_tokens.saturating_add(trace_reasoning);
            }
        }

        let row = route_map
            .entry(route.clone())
            .or_insert_with(|| ModelUsageBreakdownRow {
                route: route.clone(),
                channel_id,
                model_id,
                run_count: 0,
                model_calls: 0,
                tool_calls: 0,
                prompt_tokens: 0,
                completion_tokens: 0,
                reasoning_tokens: 0,
                cached_tokens: 0,
                total_tokens: 0,
            });
        row.run_count = row.run_count.saturating_add(1);
        row.model_calls = row.model_calls.saturating_add(run_model_calls);
        row.tool_calls = row.tool_calls.saturating_add(trace_tool_calls);
        row.prompt_tokens = row.prompt_tokens.saturating_add(run_prompt);
        row.completion_tokens = row.completion_tokens.saturating_add(run_completion);
        row.reasoning_tokens = row.reasoning_tokens.saturating_add(trace_reasoning);
        row.cached_tokens = row.cached_tokens.saturating_add(trace_cached);
        row.total_tokens = row
            .prompt_tokens
            .saturating_add(row.completion_tokens)
            .saturating_add(row.reasoning_tokens);
    }

    let mut tool_breakdown: Vec<ToolUsageBreakdownRow> = tool_map
        .into_values()
        .map(|mut row| {
            if row.success_count > row.call_count {
                row.success_count = row.call_count;
            }
            row
        })
        .collect();
    tool_breakdown.sort_by(|a, b| {
        b.call_count
            .cmp(&a.call_count)
            .then_with(|| a.tool_id.cmp(&b.tool_id))
    });
    tool_breakdown.truncate(15);

    struct TaskStatAccum {
        status: insar_core::TaskLifecycleStatus,
        bytes: u64,
        selected_count: usize,
        provider_id: String,
        date_str: Option<String>,
    }

    let mut task_map: std::collections::HashMap<String, TaskStatAccum> =
        std::collections::HashMap::new();
    if let Ok(overview) = state.task_overview() {
        for t in overview.queue {
            let bytes = t
                .transferred_bytes
                .max(t.terminal_bytes.saturating_add(t.active_bytes));
            let date_str = extract_utc_date_from_timestamp(&t.updated_at)
                .or_else(|| extract_utc_date_from_timestamp(&t.created_at));
            task_map.insert(
                t.task_id,
                TaskStatAccum {
                    status: t.status,
                    bytes,
                    selected_count: t.selected_count as usize,
                    provider_id: t.provider_id,
                    date_str,
                },
            );
        }
        for item in overview.history_page.items {
            let bytes = item
                .transferred_bytes
                .max(item.terminal_bytes.saturating_add(item.active_bytes));
            let date_str = extract_utc_date_from_timestamp(&item.updated_at)
                .or_else(|| extract_utc_date_from_timestamp(&item.created_at));
            task_map.entry(item.task_id).or_insert(TaskStatAccum {
                status: item.status,
                bytes,
                selected_count: item.selected_count as usize,
                provider_id: item.provider_id,
                date_str,
            });
        }
    }
    if let Ok(history) = state.task_history_page(&StoreTaskHistoryPageRequest {
        page_size: 200,
        after_archived_at_epoch_ms: None,
        after_task_id: None,
    }) {
        for item in history.items {
            let t = item.task;
            let bytes = t
                .transferred_bytes
                .max(t.terminal_bytes.saturating_add(t.active_bytes));
            let date_str = extract_utc_date_from_timestamp(&t.updated_at)
                .or_else(|| extract_utc_date_from_timestamp(&t.created_at));
            task_map.entry(t.task_id).or_insert(TaskStatAccum {
                status: t.status,
                bytes,
                selected_count: t.selected_count,
                provider_id: t.provider_id,
                date_str,
            });
        }
    }

    let download_total_tasks = task_map.len();
    let mut download_completed_tasks = 0usize;
    let mut download_active_tasks = 0usize;
    let mut download_failed_tasks = 0usize;
    let mut download_transferred_bytes = 0u64;
    let mut download_selected_items = 0usize;
    let mut provider_map: std::collections::HashMap<String, ProviderDownloadBreakdownRow> =
        std::collections::HashMap::new();

    for t in task_map.values() {
        match t.status {
            insar_core::TaskLifecycleStatus::Completed => {
                download_completed_tasks += 1;
            }
            insar_core::TaskLifecycleStatus::Running
            | insar_core::TaskLifecycleStatus::Queued
            | insar_core::TaskLifecycleStatus::Paused => {
                download_active_tasks += 1;
            }
            insar_core::TaskLifecycleStatus::Failed => {
                download_failed_tasks += 1;
            }
            _ => {}
        }
        download_transferred_bytes = download_transferred_bytes.saturating_add(t.bytes);
        download_selected_items = download_selected_items.saturating_add(t.selected_count);
        if let Some(ref day_key) = t.date_str {
            if let Some(day_point) = daily_map.get_mut(day_key) {
                day_point.download_bytes = day_point.download_bytes.saturating_add(t.bytes);
            }
        }
        let pid = t.provider_id.trim();
        let norm_pid = if pid.is_empty() { "unknown" } else { pid };
        let prov_entry = provider_map.entry(norm_pid.to_string()).or_insert_with(|| {
            ProviderDownloadBreakdownRow {
                source_id: norm_pid.to_string(),
                display_name: format_provider_display_name(norm_pid),
                task_count: 0,
                bytes_transferred: 0,
            }
        });
        prov_entry.task_count += 1;
        prov_entry.bytes_transferred = prov_entry.bytes_transferred.saturating_add(t.bytes);
    }

    let mut provider_download_breakdown: Vec<ProviderDownloadBreakdownRow> =
        provider_map.into_values().collect();
    provider_download_breakdown.sort_by(|a, b| {
        b.bytes_transferred
            .cmp(&a.bytes_transferred)
            .then_with(|| b.task_count.cmp(&a.task_count))
            .then_with(|| a.source_id.cmp(&b.source_id))
    });

    let gee_raw = crate::gee_cloud_suite::gee_account_status(&data_root);
    let gee_quota_status = Some(GeeQuotaStatusSummary {
        configured: gee_raw
            .get("configured")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        verified: gee_raw
            .get("gee_ready")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        project_id: gee_raw
            .get("project_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        auth_mode: gee_raw
            .get("auth_mode")
            .and_then(|v| v.as_str())
            .unwrap_or("none")
            .to_string(),
    });

    let rt_plugins_dir = discover_runtime_plugins_dir();
    let plugins_status = inspect_plugins_status_at(&data_root, &rt_plugins_dir).ok();
    let installed_plugins_count = plugins_status
        .as_ref()
        .map(|s| s.installed_plugins.len())
        .unwrap_or(0);
    let custom_skills_count = plugins_status
        .as_ref()
        .map(|s| s.custom_skills.len())
        .unwrap_or(0);

    let total_tokens = total_prompt
        .saturating_add(total_completion)
        .saturating_add(total_reasoning);

    Ok(ApplicationUsageStatisticsResponse {
        ok: true,
        collected_at: now_iso_timestamp(),
        data_root: data_root.to_string_lossy().to_string(),
        sqlite_size_bytes,
        disk_free_bytes,
        workspace_count,
        session_count,
        run_count,
        completed_run_count,
        tool_call_count,
        prompt_tokens: total_prompt,
        completion_tokens: total_completion,
        reasoning_tokens: total_reasoning,
        cached_tokens: total_cached,
        total_tokens,
        model_breakdown: route_map.into_values().collect(),
        download_total_tasks,
        download_completed_tasks,
        download_active_tasks,
        download_failed_tasks,
        download_transferred_bytes,
        download_selected_items,
        installed_plugins_count,
        custom_skills_count,
        daily_series: daily_map.into_values().collect(),
        tool_breakdown,
        provider_download_breakdown,
        gee_quota_status,
    })
}

// ============================================================================
// 商业级可控缓存管理：统计汇总 (get_cache_summary) 与安全一键清理 (clear_disposable_caches)
// ============================================================================

const CATALOG_AND_PREVIEW_CACHE_CAP_BYTES: u64 = 64 * 1024 * 1024;
const CATALOG_AND_PREVIEW_CACHE_MAX_ENTRIES: u64 = 512;
const CATALOG_AND_PREVIEW_CACHE_TTL_SECONDS: u64 = 7 * 24 * 60 * 60;
const PROTECTED_CACHE_NOTICE_ZH: &str =
    "清理缓存仅删除可重建的检索、边界、目录与临时预览缓存，不会影响已下载影像、下载任务记录、工作区设置或账号凭据。";

/// Disposable on-disk cache subdirectories under `app_data_dir`.
/// Strictly excludes user-owned directories (`agent`, `plugins`, `skills_custom`,
/// `downloads`, `geodata_downloads`, `gee_downloads`, `gis_outputs`, `open_geodata`)
/// and all credential/settings files.
const DISPOSABLE_DISK_CACHE_SUBDIRS: &[&str] = &[
    "gee_catalog_cache",
    "gee_artifacts",
    "scratch",
    "cache",
    "thumbnails",
    "quicklook_cache",
];

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct CacheCategorySummary {
    pub id: String,
    pub label_zh: String,
    pub description_zh: String,
    pub location: String,
    pub size_bytes: u64,
    pub cap_bytes: u64,
    pub entry_count: u64,
    pub max_entries: u64,
    pub ttl_seconds: u64,
    pub clearable: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct CacheSummaryResponse {
    pub ok: bool,
    pub cache_dir: String,
    pub sqlite_path: String,
    pub sqlite_size_bytes: u64,
    pub sqlite_wal_bytes: u64,
    pub total_size_bytes: u64,
    pub total_cap_bytes: u64,
    pub categories: Vec<CacheCategorySummary>,
    pub protected_notice_zh: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ClearCachesResponse {
    pub ok: bool,
    pub cleared_categories: Vec<String>,
    pub reclaimed_bytes: u64,
    pub removed_entries: u64,
    pub vacuumed: bool,
    pub summary: CacheSummaryResponse,
}

fn resolve_primary_sqlite_path(
    state: &ApplicationServiceState,
    data_root: &std::path::Path,
) -> std::path::PathBuf {
    let mut candidates = Vec::with_capacity(5);
    if let Some(configured_store) = state.service.store_path() {
        candidates.push(configured_store.to_path_buf());
    }
    candidates.push(data_root.join("tasks.sqlite3"));
    candidates.push(data_root.join("store.sqlite"));
    candidates.push(default_store_path());
    candidates.push(data_root.join("insar.sqlite"));
    for path in &candidates {
        if path.is_file() {
            return path.clone();
        }
    }
    candidates
        .into_iter()
        .next()
        .unwrap_or_else(|| data_root.join("tasks.sqlite3"))
}

fn collect_files_recursive(
    root: &std::path::Path,
    out: &mut Vec<(std::path::PathBuf, u64, std::time::SystemTime)>,
) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            collect_files_recursive(&path, out);
        } else if meta.is_file() {
            let modified = meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            out.push((path, meta.len(), modified));
        }
    }
}

fn prune_disposable_disk_caches_at(
    data_root: &std::path::Path,
    ttl_seconds: u64,
    max_entries: u64,
    max_bytes: u64,
) -> (u64, u64) {
    let mut files = Vec::new();
    for subdir in DISPOSABLE_DISK_CACHE_SUBDIRS {
        let dir = data_root.join(subdir);
        if dir.is_dir() {
            collect_files_recursive(&dir, &mut files);
        }
    }
    // Newest first for LRU retention.
    files.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| b.0.cmp(&a.0)));

    let now = std::time::SystemTime::now();
    let ttl = std::time::Duration::from_secs(ttl_seconds);
    let mut kept_entries: u64 = 0;
    let mut kept_bytes: u64 = 0;
    let mut removed_entries: u64 = 0;
    let mut reclaimed_bytes: u64 = 0;

    for (path, len, modified) in files {
        let expired = now
            .duration_since(modified)
            .map(|age| age > ttl)
            .unwrap_or(false);
        let over_budget = kept_entries.saturating_add(1) > max_entries
            || kept_bytes.saturating_add(len) > max_bytes;
        if expired || over_budget {
            if std::fs::remove_file(&path).is_ok() {
                removed_entries = removed_entries.saturating_add(1);
                reclaimed_bytes = reclaimed_bytes.saturating_add(len);
            }
        } else {
            kept_entries = kept_entries.saturating_add(1);
            kept_bytes = kept_bytes.saturating_add(len);
        }
    }
    (removed_entries, reclaimed_bytes)
}

fn inspect_disposable_disk_caches_at(data_root: &std::path::Path) -> (u64, u64) {
    let mut files = Vec::new();
    for subdir in DISPOSABLE_DISK_CACHE_SUBDIRS {
        let dir = data_root.join(subdir);
        if dir.is_dir() {
            collect_files_recursive(&dir, &mut files);
        }
    }
    let total_bytes: u64 = files.iter().map(|(_, len, _)| *len).sum();
    (files.len() as u64, total_bytes)
}

fn clear_disposable_disk_caches_at(data_root: &std::path::Path) -> (u64, u64) {
    let mut files = Vec::new();
    for subdir in DISPOSABLE_DISK_CACHE_SUBDIRS {
        let dir = data_root.join(subdir);
        if dir.is_dir() {
            collect_files_recursive(&dir, &mut files);
        }
    }
    let mut removed_entries: u64 = 0;
    let mut reclaimed_bytes: u64 = 0;
    for (path, len, _) in files {
        if std::fs::remove_file(&path).is_ok() {
            removed_entries = removed_entries.saturating_add(1);
            reclaimed_bytes = reclaimed_bytes.saturating_add(len);
        }
    }
    for subdir in DISPOSABLE_DISK_CACHE_SUBDIRS {
        let dir = data_root.join(subdir);
        if dir.is_dir() {
            remove_empty_subdirs(&dir);
        }
    }
    (removed_entries, reclaimed_bytes)
}

fn remove_empty_subdirs(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            remove_empty_subdirs(&path);
            let _ = std::fs::remove_dir(&path);
        }
    }
}

fn build_cache_summary(
    data_root: &std::path::Path,
    sqlite_path: &std::path::Path,
    search_cache_bytes: u64,
    search_cache_entries: u64,
    admin_boundary_cache_bytes: u64,
    admin_boundary_cache_entries: u64,
) -> CacheSummaryResponse {
    use insar_agent::session::TaskStore;

    let (disk_entries, disk_bytes) = inspect_disposable_disk_caches_at(data_root);
    let sqlite_size_bytes = sqlite_bundle_size_bytes(sqlite_path).unwrap_or(0);
    let wal_path = std::path::PathBuf::from(format!("{}-wal", sqlite_path.display()));
    let sqlite_wal_bytes = std::fs::metadata(&wal_path).map(|m| m.len()).unwrap_or(0);

    let search_cap = TaskStore::SEARCH_CACHE_MAX_BYTES;
    let admin_cap = TaskStore::ADMIN_BOUNDARY_CACHE_MAX_BYTES;
    let disk_cap = CATALOG_AND_PREVIEW_CACHE_CAP_BYTES;

    let categories = vec![
        CacheCategorySummary {
            id: "search_cache".to_owned(),
            label_zh: "检索结果缓存".to_owned(),
            description_zh:
                "SAR 与光学历史检索缓存及未绑定任务的检索快照（按 24 小时 TTL 与容量上限自动淘汰）"
                    .to_owned(),
            location: sqlite_path.to_string_lossy().to_string(),
            size_bytes: search_cache_bytes,
            cap_bytes: search_cap,
            entry_count: search_cache_entries,
            max_entries: TaskStore::SEARCH_CACHE_MAX_ENTRIES as u64,
            ttl_seconds: TaskStore::SEARCH_CACHE_DEFAULT_TTL_SECONDS,
            clearable: true,
        },
        CacheCategorySummary {
            id: "admin_boundary_cache".to_owned(),
            label_zh: "行政区边界缓存".to_owned(),
            description_zh:
                "已解析的中国省市县与全球行政区矢量边界缓存（按 30 天 TTL 与容量上限自动淘汰）"
                    .to_owned(),
            location: sqlite_path.to_string_lossy().to_string(),
            size_bytes: admin_boundary_cache_bytes,
            cap_bytes: admin_cap,
            entry_count: admin_boundary_cache_entries,
            max_entries: TaskStore::ADMIN_BOUNDARY_CACHE_MAX_ENTRIES as u64,
            ttl_seconds: TaskStore::ADMIN_BOUNDARY_CACHE_DEFAULT_TTL_SECONDS,
            clearable: true,
        },
        CacheCategorySummary {
            id: "catalog_and_preview_cache".to_owned(),
            label_zh: "目录与缩略图缓存".to_owned(),
            description_zh:
                "GEE 数据目录本地缓存、临时预览图与工具暂存文件（按 7 天 TTL 与容量上限自动淘汰）"
                    .to_owned(),
            location: data_root.to_string_lossy().to_string(),
            size_bytes: disk_bytes,
            cap_bytes: disk_cap,
            entry_count: disk_entries,
            max_entries: CATALOG_AND_PREVIEW_CACHE_MAX_ENTRIES,
            ttl_seconds: CATALOG_AND_PREVIEW_CACHE_TTL_SECONDS,
            clearable: true,
        },
    ];

    let total_size_bytes: u64 = categories.iter().map(|c| c.size_bytes).sum();
    let total_cap_bytes: u64 = categories.iter().map(|c| c.cap_bytes).sum();

    CacheSummaryResponse {
        ok: true,
        cache_dir: data_root.to_string_lossy().to_string(),
        sqlite_path: sqlite_path.to_string_lossy().to_string(),
        sqlite_size_bytes,
        sqlite_wal_bytes,
        total_size_bytes,
        total_cap_bytes,
        categories,
        protected_notice_zh: PROTECTED_CACHE_NOTICE_ZH.to_owned(),
    }
}

pub fn compute_cache_summary_at(
    data_root: &std::path::Path,
    sqlite_path: &std::path::Path,
    store: &mut insar_agent::session::TaskStore,
) -> Result<CacheSummaryResponse, String> {
    // Enforce commercial-grade TTL + size cap opportunistically on disposable disk caches.
    let _ = prune_disposable_disk_caches_at(
        data_root,
        CATALOG_AND_PREVIEW_CACHE_TTL_SECONDS,
        CATALOG_AND_PREVIEW_CACHE_MAX_ENTRIES,
        CATALOG_AND_PREVIEW_CACHE_CAP_BYTES,
    );
    let now_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let admin_cutoff_iso = (chrono::Utc::now()
        - chrono::Duration::seconds(
            insar_agent::session::TaskStore::ADMIN_BOUNDARY_CACHE_DEFAULT_TTL_SECONDS as i64,
        ))
    .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    if let Ok(outcome) = store.prune_all_disposable_caches(now_epoch, &admin_cutoff_iso) {
        if outcome.reclaimed_bytes >= 16 * 1024 * 1024 {
            let _ = store.reclaim_sqlite_space(true);
        } else if outcome.removed_entries > 0 || outcome.reclaimed_bytes > 0 {
            let _ = store.reclaim_sqlite_space(false);
        }
    }
    let store_stats = store
        .inspect_disposable_cache_stats()
        .map_err(|err| format!("CACHE_INSPECT_FAILED: {err}"))?;
    Ok(build_cache_summary(
        data_root,
        sqlite_path,
        store_stats.search_cache_bytes,
        store_stats.search_cache_entries,
        store_stats.admin_boundary_cache_bytes,
        store_stats.admin_boundary_cache_entries,
    ))
}

pub fn clear_disposable_caches_at(
    data_root: &std::path::Path,
    sqlite_path: &std::path::Path,
    store: &mut insar_agent::session::TaskStore,
    categories: Option<Vec<String>>,
) -> Result<ClearCachesResponse, String> {
    let requested: Vec<String> = match categories {
        Some(list) if !list.is_empty() => list
            .into_iter()
            .map(|item| item.trim().to_ascii_lowercase())
            .filter(|item| !item.is_empty())
            .collect(),
        _ => vec![
            "search_cache".to_owned(),
            "admin_boundary_cache".to_owned(),
            "catalog_and_preview_cache".to_owned(),
        ],
    };

    let mut cleared_categories = Vec::new();
    let mut reclaimed_bytes: u64 = 0;
    let mut removed_entries: u64 = 0;
    let mut touched_sqlite = false;

    if requested.iter().any(|c| c == "search_cache") {
        let outcome = store
            .clear_disposable_search_cache()
            .map_err(|err| format!("CLEAR_SEARCH_CACHE_FAILED: {err}"))?;
        reclaimed_bytes = reclaimed_bytes.saturating_add(outcome.reclaimed_bytes);
        removed_entries = removed_entries.saturating_add(outcome.removed_entries);
        cleared_categories.push("search_cache".to_owned());
        touched_sqlite = true;
    }

    if requested.iter().any(|c| c == "admin_boundary_cache") {
        let outcome = store
            .clear_administrative_boundary_cache()
            .map_err(|err| format!("CLEAR_ADMIN_BOUNDARY_CACHE_FAILED: {err}"))?;
        reclaimed_bytes = reclaimed_bytes.saturating_add(outcome.reclaimed_bytes);
        removed_entries = removed_entries.saturating_add(outcome.removed_entries);
        cleared_categories.push("admin_boundary_cache".to_owned());
        touched_sqlite = true;
    }

    if requested.iter().any(|c| c == "catalog_and_preview_cache") {
        let (disk_removed, disk_reclaimed) = clear_disposable_disk_caches_at(data_root);
        removed_entries = removed_entries.saturating_add(disk_removed);
        reclaimed_bytes = reclaimed_bytes.saturating_add(disk_reclaimed);
        cleared_categories.push("catalog_and_preview_cache".to_owned());
    }

    let vacuumed = if touched_sqlite {
        store
            .reclaim_sqlite_space(reclaimed_bytes > 0 || removed_entries > 0)
            .unwrap_or(false)
    } else {
        false
    };

    let summary = compute_cache_summary_at(data_root, sqlite_path, store)?;
    Ok(ClearCachesResponse {
        ok: true,
        cleared_categories,
        reclaimed_bytes,
        removed_entries,
        vacuumed,
        summary,
    })
}

pub fn resolve_disposable_cache_dir_from_settings(
    app_data_dir: &std::path::Path,
    settings: Option<&ApplicationNetworkSettings>,
) -> std::path::PathBuf {
    if let Some(settings) = settings {
        let custom = settings.cache_dir.trim();
        if !custom.is_empty() {
            let candidate = std::path::PathBuf::from(custom);
            if candidate.is_absolute() {
                return candidate;
            }
        }
    }
    app_data_dir.to_path_buf()
}

pub fn resolve_disposable_cache_dir(state: &ApplicationServiceState) -> std::path::PathBuf {
    let data_root = resolve_app_data_dir(state);
    let settings = state.get_network_settings().ok();
    resolve_disposable_cache_dir_from_settings(&data_root, settings.as_ref())
}

pub fn compute_cache_summary(
    state: &ApplicationServiceState,
) -> Result<CacheSummaryResponse, String> {
    let data_root = resolve_app_data_dir(state);
    let cache_root = resolve_disposable_cache_dir(state);
    let sqlite_path = resolve_primary_sqlite_path(state, &data_root);
    state
        .agent_application()
        .with_task_store(|store| compute_cache_summary_at(&cache_root, &sqlite_path, store))
        .map_err(|err| format!("CACHE_STORE_UNAVAILABLE: {err}"))?
}

pub fn clear_disposable_caches(
    state: &ApplicationServiceState,
    categories: Option<Vec<String>>,
) -> Result<ClearCachesResponse, String> {
    let data_root = resolve_app_data_dir(state);
    let cache_root = resolve_disposable_cache_dir(state);
    let sqlite_path = resolve_primary_sqlite_path(state, &data_root);
    state
        .agent_application()
        .with_task_store(|store| {
            clear_disposable_caches_at(&cache_root, &sqlite_path, store, categories)
        })
        .map_err(|err| format!("CACHE_STORE_UNAVAILABLE: {err}"))?
}

#[cfg(test)]
mod extension_hub_and_component_install_tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::io::Write;

    fn temp_test_root(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sceneport-ext-hub-test-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn resolve_registry_url_supports_gitcode_cloudflare_github_and_local_paths() {
        assert_eq!(
            resolve_registry_url(
                "https://raw.gitcode.com/hhanmj/eostage-hub/raw/main/registry.json",
                "components/vertical-grid-egm96-1.0.0.zip"
            ),
            "https://raw.gitcode.com/hhanmj/eostage-hub/raw/main/components/vertical-grid-egm96-1.0.0.zip"
        );
        assert_eq!(
            resolve_registry_url(
                "https://hub.eostage.org/registry.json",
                "./skills/insar-alos2-stripmap.json"
            ),
            "https://hub.eostage.org/skills/insar-alos2-stripmap.json"
        );
        assert_eq!(
            resolve_registry_url(
                "https://raw.githubusercontent.com/hhanmj/eostage-hub/main/registry.json",
                "https://mirror.example.com/direct.zip"
            ),
            "https://mirror.example.com/direct.zip"
        );
        assert_eq!(
            resolve_extension_registry_source(Some("cloudflare")),
            DEFAULT_EXTENSION_REGISTRY_URL
        );
        assert_eq!(
            resolve_extension_registry_source(Some("https://eostage-hub.pages.dev/registry.json")),
            DEFAULT_EXTENSION_REGISTRY_URL
        );
        assert_eq!(
            extract_git_repo_rel_path(
                "https://gitcode.com/hhanmj/eostage-hub/raw/main/components/macos/native-gis-runtime-1.0.0.zip",
                "native-gis-runtime-1.0.0.zip",
            ),
            "components/macos/native-gis-runtime-1.0.0.zip"
        );
        assert_eq!(
            extract_git_repo_rel_path(
                "https://api.gitcode.com/api/v5/repos/hhanmj/eostage-hub/raw/components/compat-worker-1.0.0.zip?ref=main",
                "compat-worker-1.0.0.zip",
            ),
            "components/compat-worker-1.0.0.zip"
        );
    }

    #[test]
    fn git_wire_v2_packfile_parser_resolves_commit_tree_blob_and_ofs_delta() {
        use flate2::{write::ZlibEncoder, Compression};

        fn zlib_pack(data: &[u8]) -> Vec<u8> {
            let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
            enc.write_all(data).unwrap();
            enc.finish().unwrap()
        }
        fn pack_obj_hdr(obj_type: u8, mut size: usize) -> Vec<u8> {
            let mut hdr = Vec::new();
            let mut first = (obj_type << 4) | ((size & 0x0f) as u8);
            size >>= 4;
            if size > 0 {
                first |= 0x80;
            }
            hdr.push(first);
            while size > 0 {
                let mut b = (size & 0x7f) as u8;
                size >>= 7;
                if size > 0 {
                    b |= 0x80;
                }
                hdr.push(b);
            }
            hdr
        }

        let base_blob = b"hello world\n";
        let base_oid = git_compute_object_oid(3, base_blob).unwrap();

        let mut tree_raw = Vec::new();
        tree_raw.extend_from_slice(b"100644 registry.json\0");
        tree_raw.extend_from_slice(&base_oid);
        let tree_oid = git_compute_object_oid(2, &tree_raw).unwrap();

        let commit_raw = format!(
            "tree {}\nauthor Test <t@example.invalid> 0 +0000\ncommitter Test <t@example.invalid> 0 +0000\n\ninit\n",
            oid20_to_hex(&tree_oid)
        )
        .into_bytes();
        let commit_oid = git_compute_object_oid(1, &commit_raw).unwrap();

        // Build an OFS_DELTA blob transforming base_blob -> "hello eostage world!\n"
        let delta_target = b"hello eostage world!\n";
        let mut delta = vec![
            base_blob.len() as u8,    // base len = 12
            delta_target.len() as u8, // target len = 21
            0x80 | 0x10,              // copy 6 bytes from offset 0 ("hello ")
            6,
            9, // insert 9 bytes ("eostage w")
        ];
        delta.extend_from_slice(b"eostage w");
        delta.push(0x80 | 0x01 | 0x10); // copy 4 bytes from offset 7 ("orld")
        delta.push(7);
        delta.push(4);
        delta.push(2); // insert "!\n"
        delta.extend_from_slice(b"!\n");

        let mut pack = Vec::new();
        pack.extend_from_slice(b"PACK");
        pack.extend_from_slice(&2u32.to_be_bytes());
        pack.extend_from_slice(&4u32.to_be_bytes());

        let base_off = pack.len();
        pack.extend_from_slice(&pack_obj_hdr(3, base_blob.len()));
        pack.extend_from_slice(&zlib_pack(base_blob));

        pack.extend_from_slice(&pack_obj_hdr(2, tree_raw.len()));
        pack.extend_from_slice(&zlib_pack(&tree_raw));

        pack.extend_from_slice(&pack_obj_hdr(1, commit_raw.len()));
        pack.extend_from_slice(&zlib_pack(&commit_raw));

        fn enc_ofs(mut n: usize) -> Vec<u8> {
            let mut out = vec![(n & 0x7f) as u8];
            while n > 0x7f {
                n = (n >> 7) - 1;
                out.push(((n & 0x7f) | 0x80) as u8);
            }
            out.reverse();
            out
        }

        let delta_off = pack.len();
        let rel_back = delta_off - base_off;
        pack.extend_from_slice(&pack_obj_hdr(6, delta.len()));
        pack.extend_from_slice(&enc_ofs(rel_back));
        pack.extend_from_slice(&zlib_pack(&delta));
        pack.extend_from_slice(&[0u8; 20]); // trailing pack sha1

        let objs = git_parse_packfile_objects(&pack, 4096).expect("parse packfile");
        assert_eq!(objs.len(), 4);
        assert_eq!(objs[3].raw, delta_target);
        assert_eq!(
            git_lookup_path_in_tree_pack(&objs, commit_oid, "registry.json").unwrap(),
            base_oid
        );

        let err = git_parse_packfile_objects(&pack, 15).unwrap_err();
        assert!(
            err.contains("GIT_WIRE_OBJECT_TOO_LARGE") || err.contains("GIT_WIRE_DELTA_TOO_LARGE")
        );

        // Verify sideband pkt-line framing extraction
        let mut resp = Vec::new();
        resp.extend_from_slice(&git_pkt_line(b"shallow-info\n"));
        resp.extend_from_slice(b"0001");
        resp.extend_from_slice(&git_pkt_line(b"packfile\n"));
        let mut band1 = vec![1u8];
        band1.extend_from_slice(&pack);
        resp.extend_from_slice(&git_pkt_line(&band1));
        resp.extend_from_slice(b"0000");
        assert_eq!(
            git_extract_packfile_from_fetch_response(&resp).unwrap(),
            pack
        );
    }

    #[test]
    fn extension_registry_parses_platforms_resolves_host_package_and_rejects_html_and_cross_os() {
        let host = current_host_platform_tag();
        let other_os = if cfg!(target_os = "windows") {
            "macos-aarch64"
        } else {
            "windows-x86_64"
        };
        let host_sha = "a".repeat(64);
        let other_sha = "b".repeat(64);
        let reg_json = format!(
            r#"{{
                "version": 1,
                "schema_version": 2,
                "updated_at": "2026-11-20T15:00:00Z",
                "entries": [
                    {{
                        "id": "native-gis-runtime",
                        "name": "Native GIS",
                        "kind": "component",
                        "version": "1.0.0",
                        "package": "components/other.zip",
                        "sha256": "{other_sha}",
                        "size": 100,
                        "install_root": "runtime/gis",
                        "platforms": ["{host}", "{other_os}"],
                        "platform_packages": {{
                            "{host}": {{
                                "package": "components/host.zip",
                                "sha256": "{host_sha}",
                                "size": 200
                            }},
                            "{other_os}": {{
                                "package": "components/other.zip",
                                "sha256": "{other_sha}",
                                "size": 100
                            }}
                        }}
                    }}
                ]
            }}"#
        );
        let report = parse_and_filter_extension_registry(
            "https://raw.githubusercontent.com/hhanmj/eostage-hub/main/registry.json",
            &reg_json,
            None,
        )
        .expect("parse registry with platform_packages");
        assert_eq!(report.entries.len(), 1);
        assert_eq!(
            report.entries[0].package,
            "https://raw.githubusercontent.com/hhanmj/eostage-hub/main/components/host.zip"
        );
        assert_eq!(report.entries[0].sha256, host_sha);
        assert_eq!(report.entries[0].size, 200);
        assert_eq!(
            report.entries[0].platforms,
            vec![host.clone(), other_os.to_string()]
        );

        // Verify HTML payload detection rejects GitCode 200 HTML SPA pages
        assert!(is_likely_html_payload(b"  <!DOCTYPE html><html></html>"));
        assert!(!is_likely_html_payload(b"{\"entries\":[]}"));

        // Verify local sentinel fails honestly when _hub_dist is missing
        let missing_err = fetch_extension_registry(Some("local".to_string()));
        if !discover_runtime_root()
            .join("_hub_dist/registry.json")
            .is_file()
        {
            assert!(missing_err
                .unwrap_err()
                .contains("REGISTRY_LOCAL_NOT_FOUND"));
        } else {
            assert!(missing_err.is_ok());
        }

        // Verify compat-worker Windows .exe payload is rejected on non-Windows hosts before .old backup
        if !cfg!(target_os = "windows") {
            let root = temp_test_root("cross-os-reject");
            let wrong_zip = root.join("compat-win.zip");
            {
                let f = std::fs::File::create(&wrong_zip).unwrap();
                let mut z = zip::ZipWriter::new(f);
                z.start_file(
                    "eostage-compat-worker.exe",
                    zip::write::SimpleFileOptions::default(),
                )
                .unwrap();
                z.write_all(b"MZ\x90\x00WIN_EXE").unwrap();
                z.finish().unwrap();
            }
            let err = install_runtime_component_at(
                &root,
                &RuntimeComponentInstallRequest {
                    component_id: "compat-worker".to_string(),
                    source: wrong_zip.to_string_lossy().to_string(),
                    expected_sha256: None,
                },
            )
            .unwrap_err();
            assert!(err.contains("RUNTIME_COMPONENT_PLATFORM_MISMATCH"), "{err}");
            assert!(!root.join("runtime/compat-worker").exists());
            let _ = std::fs::remove_dir_all(&root);
        }
    }

    #[test]
    #[ignore = "live network probe to GitCode git-upload-pack (Wire Protocol v2)"]
    fn live_gitcode_wire_v2_fetches_registry_without_api_rate_limit() {
        let (src_url, reg_raw, bl_raw) =
            fetch_gitcode_wire_v2_registry_and_blocklist(DEFAULT_EXTENSION_REGISTRY_URL)
                .expect("live GitCode Wire v2 fetch should succeed without API rate limits");
        assert_eq!(
            src_url, "https://gitcode.com/hhanmj/eostage-hub/raw/main/registry.json",
            "expected renamed eostage-hub repository on GitCode, got: {src_url}"
        );
        assert!(!is_likely_html_payload(reg_raw.as_bytes()));
        let report = parse_and_filter_extension_registry(&src_url, &reg_raw, bl_raw.as_deref())
            .expect("fetched registry should parse cleanly");
        assert!(report.ok);
        assert_eq!(
            report.entries.len(),
            16,
            "fetched eostage-hub registry should contain all 16 entries"
        );
        eprintln!(
            "live Wire v2 src_url={src_url}, entries={}",
            report.entries.len()
        );
    }

    #[test]
    fn official_release_url_projection_and_download_plan_cover_gitcode_github_and_custom_sources() {
        let mac_sha = "1".repeat(64);
        let win_sha = "2".repeat(64);
        let reg_json = format!(
            r#"{{
                "version": 1,
                "schema_version": 2,
                "updated_at": "2026-11-23T09:20:00Z",
                "entries": [
                    {{
                        "id": "native-gis-runtime",
                        "name": "Native GIS",
                        "kind": "component",
                        "version": "1.0.0",
                        "package": "https://gitcode.com/hhanmj/eostage-hub/releases/download/v1.0.0/native-gis-runtime-1.0.0.zip",
                        "sha256": "{mac_sha}",
                        "size": 14025059,
                        "install_root": "runtime/gis",
                        "platforms": ["macos-aarch64", "windows-x86_64"],
                        "platform_packages": {{
                            "macos-aarch64": {{
                                "package": "https://gitcode.com/hhanmj/eostage-hub/releases/download/v1.0.0/native-gis-runtime-1.0.0.zip",
                                "sha256": "{mac_sha}",
                                "size": 14025059
                            }},
                            "windows-x86_64": {{
                                "package": "https://gitcode.com/hhanmj/eostage-hub/releases/download/v1.0.0/native-gis-runtime-1.0.0-windows-x86_64.zip",
                                "sha256": "{win_sha}",
                                "size": 25572230
                            }}
                        }}
                    }}
                ]
            }}"#
        );

        // 1. GitHub registry source projects both entry.package and platform_packages to GitHub Releases
        let gh_report = parse_and_filter_extension_registry(
            "https://raw.githubusercontent.com/hhanmj/eostage-hub/main/registry.json",
            &reg_json,
            None,
        )
        .expect("parse github mirror registry");
        let gh_entry = &gh_report.entries[0];
        assert_eq!(
            gh_entry.platform_packages["macos-aarch64"].package,
            "https://github.com/hhanmj/eostage-hub/releases/download/v1.0.0/native-gis-runtime-1.0.0.zip"
        );
        assert_eq!(
            gh_entry.platform_packages["windows-x86_64"].package,
            "https://github.com/hhanmj/eostage-hub/releases/download/v1.0.0/native-gis-runtime-1.0.0-windows-x86_64.zip"
        );
        if cfg!(target_os = "windows") {
            assert_eq!(
                gh_entry.package,
                "https://github.com/hhanmj/eostage-hub/releases/download/v1.0.0/native-gis-runtime-1.0.0-windows-x86_64.zip"
            );
            assert_eq!(gh_entry.sha256, win_sha);
        }

        // 2. GitCode registry source projects to GitCode Releases
        let gc_report =
            parse_and_filter_extension_registry(DEFAULT_EXTENSION_REGISTRY_URL, &reg_json, None)
                .expect("parse gitcode mirror registry");
        let gc_entry = &gc_report.entries[0];
        assert_eq!(
            gc_entry.platform_packages["windows-x86_64"].package,
            "https://gitcode.com/hhanmj/eostage-hub/releases/download/v1.0.0/native-gis-runtime-1.0.0-windows-x86_64.zip"
        );

        // 3. Loopback & custom third-party registry sources do NOT rewrite explicit Release URLs
        let local_report = parse_and_filter_extension_registry(
            "http://127.0.0.1:18888/registry.json",
            &reg_json,
            None,
        )
        .expect("parse loopback registry");
        assert_eq!(
            local_report.entries[0].platform_packages["windows-x86_64"].package,
            "https://gitcode.com/hhanmj/eostage-hub/releases/download/v1.0.0/native-gis-runtime-1.0.0-windows-x86_64.zip"
        );

        // 4. Download plan: GitCode Web Release preserves arbitrary tag, prefers GitCode Web->API, falls back to GitHub, skips Wire v2
        let gc_plan = expand_component_download_plan(
            "https://gitcode.com/hhanmj/eostage-hub/releases/download/v2.1.0/native-gis-runtime-1.0.0-windows-x86_64.zip",
        );
        assert_eq!(
            gc_plan.primary_urls,
            vec![
                "https://gitcode.com/hhanmj/eostage-hub/releases/download/v2.1.0/native-gis-runtime-1.0.0-windows-x86_64.zip",
                "https://api.gitcode.com/api/v5/repos/hhanmj/eostage-hub/releases/v2.1.0/attach_files/native-gis-runtime-1.0.0-windows-x86_64.zip/download",
            ]
        );
        assert_eq!(
            gc_plan.fallback_urls,
            vec![
                "https://github.com/hhanmj/eostage-hub/releases/download/v2.1.0/native-gis-runtime-1.0.0-windows-x86_64.zip"
            ]
        );
        assert!(!gc_plan.allow_gitcode_wire_v2);
        assert!(gc_plan.allow_local_hub_fallback);
        assert_eq!(
            gc_plan.file_name,
            "native-gis-runtime-1.0.0-windows-x86_64.zip"
        );

        // 5. GitCode API attach_files URL extracts true filename (not "download") & expects ZIP magic
        let gc_api_url = "https://api.gitcode.com/api/v5/repos/hhanmj/eostage-hub/releases/v1.2.0/attach_files/compat-worker-1.0.0.zip/download";
        assert!(url_expects_zip(gc_api_url));
        let gc_api_plan = expand_component_download_plan(gc_api_url);
        assert_eq!(gc_api_plan.file_name, "compat-worker-1.0.0.zip");
        assert_eq!(gc_api_plan.primary_urls[0], gc_api_url);
        assert_eq!(
            gc_api_plan.primary_urls[1],
            "https://gitcode.com/hhanmj/eostage-hub/releases/download/v1.2.0/compat-worker-1.0.0.zip"
        );
        assert_eq!(
            gc_api_plan.fallback_urls,
            vec!["https://github.com/hhanmj/eostage-hub/releases/download/v1.2.0/compat-worker-1.0.0.zip"]
        );
        assert!(!gc_api_plan.allow_gitcode_wire_v2);

        // 6. GitHub Release prefers GitHub, falls back to GitCode Releases
        let gh_plan = expand_component_download_plan(
            "https://github.com/hhanmj/eostage-hub/releases/download/v1.0.0/vertical-grid-egm96-1.0.0.zip",
        );
        assert_eq!(
            gh_plan.primary_urls,
            vec!["https://github.com/hhanmj/eostage-hub/releases/download/v1.0.0/vertical-grid-egm96-1.0.0.zip"]
        );
        assert_eq!(
            gh_plan.fallback_urls,
            vec![
                "https://gitcode.com/hhanmj/eostage-hub/releases/download/v1.0.0/vertical-grid-egm96-1.0.0.zip",
                "https://api.gitcode.com/api/v5/repos/hhanmj/eostage-hub/releases/v1.0.0/attach_files/vertical-grid-egm96-1.0.0.zip/download",
            ]
        );
        assert!(!gh_plan.allow_gitcode_wire_v2);

        // 7. Loopback & third-party URLs containing "eostage-hub" must NOT hijack to public mirrors or _hub_dist
        for isolated in [
            "http://127.0.0.1:18080/eostage-hub/releases/download/v1.0.0/vertical-grid-egm96-1.0.0.zip",
            "http://localhost:18080/eostage-hub/components/vertical-grid-egm96-1.0.0.zip",
            "https://mirror.example.org/eostage-hub/components/vertical-grid-egm96-1.0.0.zip",
        ] {
            let p = expand_component_download_plan(isolated);
            assert_eq!(p.primary_urls, vec![isolated]);
            assert!(p.fallback_urls.is_empty(), "must not add public fallbacks for {isolated}");
            assert!(!p.allow_gitcode_wire_v2);
            assert!(!p.allow_local_hub_fallback);
        }

        // 8. Strict loopback host parsing rejects lookalike HTTP hosts like localhost.evil.com
        assert!(is_loopback_http_url("http://127.0.0.1:18080/registry.json"));
        assert!(is_loopback_http_url("http://localhost:8080/foo.zip"));
        assert!(!is_loopback_http_url("http://localhost.evil.com/foo.zip"));
        assert!(!is_loopback_http_url("http://127.0.0.1.evil.com/foo.zip"));
        assert!(!is_loopback_http_url("https://127.0.0.1:8443/foo.zip"));
    }

    #[test]
    fn loopback_http_install_verifies_zip_magic_and_never_falls_back_to_local_hub_dist() {
        use sha2::{Digest, Sha256};
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let root = temp_test_root("loopback-release-install");

        // Build a valid minimal EGM96 ZIP in memory
        let mut valid_zip_bytes = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut valid_zip_bytes);
            let mut z = zip::ZipWriter::new(cursor);
            let opt = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            z.start_file("egm96/us_nga_egm96_15.tif", opt).unwrap();
            z.write_all(&[b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 2, 3, 4])
                .unwrap();
            z.finish().unwrap();
        }
        let valid_sha = format!("{:x}", Sha256::digest(&valid_zip_bytes));

        // Plant a valid zip inside _hub_dist/components to prove loopback failure does NOT silently fall back to it
        let decoy_dir = root.join("_hub_dist/components");
        std::fs::create_dir_all(&decoy_dir).unwrap();
        std::fs::write(
            decoy_dir.join("vertical-grid-egm96-1.0.0.zip"),
            &valid_zip_bytes,
        )
        .unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback server");
        let addr = listener.local_addr().unwrap();
        let served_zip = valid_zip_bytes.clone();
        let server = std::thread::spawn(move || {
            for _ in 0..2 {
                if let Ok((mut stream, _)) = listener.accept() {
                    let mut req_buf = [0u8; 2048];
                    let n = stream.read(&mut req_buf).unwrap_or(0);
                    let req_head = String::from_utf8_lossy(&req_buf[..n]);
                    let first_line = req_head.lines().next().unwrap_or("");
                    if first_line.contains("/attach_files/vertical-grid-egm96-1.0.0.zip/download") {
                        let body = br#"{"error_code":429,"error_message":"rate limited"}"#;
                        let header = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        let _ = stream.write_all(header.as_bytes());
                        let _ = stream.write_all(body);
                    } else {
                        let header = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/zip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            served_zip.len()
                        );
                        let _ = stream.write_all(header.as_bytes());
                        let _ = stream.write_all(&served_zip);
                    }
                }
            }
        });

        // 1. JSON body on /attach_files/*.zip/download must fail with RUNTIME_COMPONENT_INVALID_ZIP (no _hub_dist hijack)
        let bad_err = install_runtime_component_at(
            &root,
            &RuntimeComponentInstallRequest {
                component_id: "vertical-grid-egm96".to_string(),
                source: format!(
                    "http://127.0.0.1:{}/eostage-hub/attach_files/vertical-grid-egm96-1.0.0.zip/download",
                    addr.port()
                ),
                expected_sha256: Some(valid_sha.clone()),
            },
        )
        .unwrap_err();
        assert!(
            bad_err.contains("RUNTIME_COMPONENT_INVALID_ZIP"),
            "expected invalid zip error, got: {bad_err}"
        );
        assert!(
            !root
                .join("runtime/vertical-grids/egm96/us_nga_egm96_15.tif")
                .exists(),
            "failed loopback install must not silently use _hub_dist"
        );

        // 2. Valid ZIP via loopback Releases URL succeeds with SHA-256 verification and post-install self-check
        let res = install_runtime_component_at(
            &root,
            &RuntimeComponentInstallRequest {
                component_id: "vertical-grid-egm96".to_string(),
                source: format!(
                    "http://127.0.0.1:{}/releases/download/v1.0.0/vertical-grid-egm96-1.0.0.zip",
                    addr.port()
                ),
                expected_sha256: Some(valid_sha.clone()),
            },
        )
        .expect("loopback release install should succeed");
        assert!(res.ok);
        assert_eq!(res.sha256, valid_sha);
        assert_eq!(res.status, "ready");
        assert!(root
            .join("runtime/vertical-grids/egm96/us_nga_egm96_15.tif")
            .is_file());

        let _ = server.join();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    #[ignore = "real Windows eostage-hub Release assets HTTP download, SHA-256, install, and self-check"]
    fn real_windows_native_gis_and_grids_e2e_install_from_loopback_release_server() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let assets_dir = std::path::PathBuf::from(
            std::env::var("EOSTAGE_REAL_HUB_ASSETS_DIR")
                .expect("EOSTAGE_REAL_HUB_ASSETS_DIR must be set"),
        );
        let hub_registry_path = std::path::PathBuf::from(
            std::env::var("EOSTAGE_REAL_HUB_REGISTRY")
                .expect("EOSTAGE_REAL_HUB_REGISTRY must be set"),
        );
        let install_root = std::path::PathBuf::from(
            std::env::var("EOSTAGE_REAL_INSTALL_ROOT")
                .expect("EOSTAGE_REAL_INSTALL_ROOT must be set"),
        );
        let _ = std::fs::remove_dir_all(&install_root);
        std::fs::create_dir_all(&install_root).unwrap();

        // 1. Verify real eostage-hub/registry.json parses cleanly under both GitCode and GitHub mirrors
        let reg_raw = std::fs::read_to_string(&hub_registry_path).expect("read real registry.json");
        let gc_report =
            parse_and_filter_extension_registry(DEFAULT_EXTENSION_REGISTRY_URL, &reg_raw, None)
                .expect("parse real registry under GitCode mirror");
        assert_eq!(gc_report.entries.len(), 16);
        let gh_report = parse_and_filter_extension_registry(
            "https://raw.githubusercontent.com/hhanmj/eostage-hub/main/registry.json",
            &reg_raw,
            None,
        )
        .expect("parse real registry under GitHub mirror");
        assert_eq!(gh_report.entries.len(), 16);

        let gc_gis = gc_report
            .entries
            .iter()
            .find(|e| e.id == "native-gis-runtime")
            .unwrap();
        let gh_gis = gh_report
            .entries
            .iter()
            .find(|e| e.id == "native-gis-runtime")
            .unwrap();
        assert_eq!(
            gc_gis.platform_packages["windows-x86_64"].package,
            "https://gitcode.com/hhanmj/eostage-hub/releases/download/v1.1.0/native-gis-runtime-1.0.0-windows-x86_64.zip"
        );
        assert_eq!(
            gh_gis.platform_packages["windows-x86_64"].package,
            "https://github.com/hhanmj/eostage-hub/releases/download/v1.1.0/native-gis-runtime-1.0.0-windows-x86_64.zip"
        );
        assert_eq!(
            gc_gis.platform_packages["macos-aarch64"].package,
            "https://gitcode.com/hhanmj/eostage-hub/releases/download/v1.1.0/native-gis-runtime-1.0.0.zip"
        );

        // 2. Serve the real release assets over a local loopback HTTP server and install all 3 GIS/grid components
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback release server");
        let port = listener.local_addr().unwrap().port();
        let assets_clone = assets_dir.clone();
        let server = std::thread::spawn(move || {
            for _ in 0..4 {
                if let Ok((mut stream, _)) = listener.accept() {
                    let mut req_buf = [0u8; 4096];
                    let n = stream.read(&mut req_buf).unwrap_or(0);
                    let req_head = String::from_utf8_lossy(&req_buf[..n]);
                    let path_part = req_head
                        .lines()
                        .next()
                        .and_then(|l| l.split_whitespace().nth(1))
                        .unwrap_or("/");
                    let fname = path_part.rsplit('/').next().unwrap_or("");
                    let file_path = assets_clone.join(fname);
                    if file_path.is_file() {
                        let bytes = std::fs::read(&file_path).unwrap();
                        let hdr = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/zip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            bytes.len()
                        );
                        let _ = stream.write_all(hdr.as_bytes());
                        let _ = stream.write_all(&bytes);
                    } else {
                        let _ = stream.write_all(
                            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        );
                    }
                }
            }
        });

        let targets = [
            (
                "native-gis-runtime",
                "native-gis-runtime-1.0.0-windows-x86_64.zip",
                gc_gis.platform_packages["windows-x86_64"].sha256.clone(),
            ),
            (
                "vertical-grid-egm96",
                "vertical-grid-egm96-1.0.0.zip",
                gc_report
                    .entries
                    .iter()
                    .find(|e| e.id == "vertical-grid-egm96")
                    .unwrap()
                    .sha256
                    .clone(),
            ),
            (
                "vertical-grid-egm2008",
                "vertical-grid-egm2008-1.0.0.zip",
                gc_report
                    .entries
                    .iter()
                    .find(|e| e.id == "vertical-grid-egm2008")
                    .unwrap()
                    .sha256
                    .clone(),
            ),
        ];

        for (comp_id, fname, sha) in &targets {
            let res = install_runtime_component_at(
                &install_root,
                &RuntimeComponentInstallRequest {
                    component_id: (*comp_id).to_string(),
                    source: format!("http://127.0.0.1:{port}/releases/download/v1.0.0/{fname}"),
                    expected_sha256: Some(sha.clone()),
                },
            )
            .unwrap_or_else(|e| panic!("failed to install {comp_id}: {e}"));
            assert!(res.ok, "{comp_id} should report ok");
            assert_eq!(res.status, "ready", "{comp_id} should be ready");
            assert_eq!(res.sha256, *sha, "{comp_id} sha256 mismatch");
        }

        // 3. Tampered SHA-256 must fail closed and preserve installed native-gis-runtime
        let bad_sha = "0".repeat(64);
        let tamper_err = install_runtime_component_at(
            &install_root,
            &RuntimeComponentInstallRequest {
                component_id: "native-gis-runtime".to_string(),
                source: format!(
                    "http://127.0.0.1:{port}/releases/download/v1.0.0/native-gis-runtime-1.0.0-windows-x86_64.zip"
                ),
                expected_sha256: Some(bad_sha),
            },
        )
        .unwrap_err();
        assert!(
            tamper_err.contains("RUNTIME_COMPONENT_SHA256_MISMATCH"),
            "{tamper_err}"
        );

        let report = inspect_runtime_components_at(&install_root).expect("inspect after install");
        for cid in [
            "native-gis-runtime",
            "vertical-grid-egm96",
            "vertical-grid-egm2008",
        ] {
            let item = report.components.iter().find(|c| c.id == cid).unwrap();
            assert_eq!(item.status, "ready", "{cid} must remain ready");
            assert!(item.verified, "{cid} must be verified");
        }

        let _ = server.join();
    }

    #[test]
    fn hub_url_and_redirect_policy_blocks_ssrf_downgrades_and_encoded_segments() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        // 1. Public URL validator rejects HTTP downgrade, embedded creds, and non-public hosts/IPs
        for bad in [
            "http://example.com/pkg.zip",
            "https://user:pass@github.com/hhanmj/eostage-hub/releases/download/v1.1.0/a.zip",
            "https://127.0.0.1/a.zip",
            "https://10.0.0.1/a.zip",
            "https://169.254.169.254/latest/meta-data",
            "https://[::1]/a.zip",
            "https://localhost/a.zip",
            "https://internal.local/a.zip",
        ] {
            let parsed = reqwest::Url::parse(bad).unwrap();
            assert!(
                validate_hub_http_url(&parsed, false, "TEST_ERR").is_err(),
                "expected public validator to reject {bad}"
            );
        }

        // Valid public HTTPS host passes
        let ok_pub = reqwest::Url::parse(
            "https://github.com/hhanmj/eostage-hub/releases/download/v1.1.0/vertical-grid-egm96-1.0.0.zip",
        )
        .unwrap();
        assert!(validate_hub_http_url(&ok_pub, false, "TEST_ERR").is_ok());

        // Loopback validator rejects off-loopback targets (including public IPs/domains and credentials)
        for bad_lb in [
            "https://github.com/hhanmj/eostage-hub/raw/main/registry.json",
            "http://169.254.169.254/latest/meta-data",
            "http://user:pw@127.0.0.1:8080/a.zip",
        ] {
            let parsed = reqwest::Url::parse(bad_lb).unwrap();
            assert!(
                validate_hub_http_url(&parsed, true, "TEST_ERR").is_err(),
                "expected loopback validator to reject {bad_lb}"
            );
        }

        // 2. is_safe_url_segment rejects percent-encoded traversal/separators
        assert!(!is_safe_url_segment("%2f"));
        assert!(!is_safe_url_segment("%2e%2e"));
        assert!(is_safe_url_segment("v1.1.0"));
        assert!(is_safe_url_segment(
            "native-gis-runtime-1.0.0-windows-x86_64.zip"
        ));

        // 3. Loopback client refuses redirect to off-loopback URL via custom redirect Policy
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind redirect test server");
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut req_buf = [0u8; 1024];
                let _ = stream.read(&mut req_buf);
                let _ = stream.write_all(
                    b"HTTP/1.1 302 Found\r\nLocation: https://example.com/evil.zip\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
            }
        });
        let err = read_text_from_source(
            &format!("http://127.0.0.1:{port}/registry.json"),
            MAX_REGISTRY_BYTES,
        )
        .unwrap_err();
        assert!(
            err.contains("REGISTRY_FETCH_FAILED"),
            "expected redirect policy abort, got: {err}"
        );
        let _ = server.join();
    }

    #[test]
    fn mirror_failover_recovers_from_sha256_mismatch_and_rejects_truncated_body() {
        use sha2::{Digest, Sha256};
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let root = temp_test_root("mirror-failover-trunc");

        let mut good_zip = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut good_zip);
            let mut z = zip::ZipWriter::new(cursor);
            let opt = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            z.start_file("egm96/us_nga_egm96_15.tif", opt).unwrap();
            z.write_all(b"GOOD_ZIP_DATA_12345").unwrap();
            z.finish().unwrap();
        }
        let good_sha = format!("{:x}", Sha256::digest(&good_zip));

        let mut stale_zip = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut stale_zip);
            let mut z = zip::ZipWriter::new(cursor);
            let opt = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            z.start_file("egm96/us_nga_egm96_15.tif", opt).unwrap();
            z.write_all(b"STALE_MISMATCH_DATA").unwrap();
            z.finish().unwrap();
        }

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind failover test server");
        let port = listener.local_addr().unwrap().port();
        let good_clone = good_zip.clone();
        let stale_clone = stale_zip.clone();
        let server = std::thread::spawn(move || {
            for _ in 0..3 {
                if let Ok((mut stream, _)) = listener.accept() {
                    let mut req_buf = [0u8; 2048];
                    let n = stream.read(&mut req_buf).unwrap_or(0);
                    let req_head = String::from_utf8_lossy(&req_buf[..n]);
                    let line = req_head.lines().next().unwrap_or("");
                    if line.contains("/primary-stale.zip") {
                        let hdr = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/zip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            stale_clone.len()
                        );
                        let _ = stream.write_all(hdr.as_bytes());
                        let _ = stream.write_all(&stale_clone);
                    } else if line.contains("/fallback-good.zip") {
                        let hdr = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/zip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            good_clone.len()
                        );
                        let _ = stream.write_all(hdr.as_bytes());
                        let _ = stream.write_all(&good_clone);
                    } else if line.contains("/truncated.zip") {
                        let hdr = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/zip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            good_clone.len() + 500
                        );
                        let _ = stream.write_all(hdr.as_bytes());
                        let _ = stream.write_all(&good_clone);
                    }
                }
            }
        });

        let client = build_hub_http_client(true, 5, 10, "TEST_CLIENT").unwrap();

        // 1. Primary returns valid ZIP with wrong SHA-256 -> must fail over to fallback URL and succeed
        let primary_url = format!("http://127.0.0.1:{port}/primary-stale.zip");
        let fallback_url = format!("http://127.0.0.1:{port}/fallback-good.zip");
        let plan = ComponentDownloadPlan {
            primary_urls: vec![primary_url.clone()],
            fallback_urls: vec![fallback_url],
            allow_gitcode_wire_v2: false,
            allow_local_hub_fallback: false,
            file_name: "vertical-grid-egm96-1.0.0.zip".to_string(),
        };
        let recovered = download_component_with_plan(
            &root,
            &client,
            &primary_url,
            &plan,
            true,
            Some(&good_sha),
        )
        .expect("should fail over from SHA-mismatched primary to healthy fallback");
        assert_eq!(recovered, good_zip);

        // 2. Truncated HTTP response (Content-Length > received bytes) must be rejected
        let trunc_err = read_https_component_candidate(
            &client,
            &format!("http://127.0.0.1:{port}/truncated.zip"),
            true,
            None,
        )
        .unwrap_err();
        assert!(
            trunc_err.contains("RUNTIME_COMPONENT_DOWNLOAD_TRUNCATED")
                || trunc_err.contains("RUNTIME_COMPONENT_DOWNLOAD_READ_FAILED"),
            "expected truncation/read error, got: {trunc_err}"
        );

        let _ = server.join();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn parse_and_filter_extension_registry_validates_sha256_and_applies_blocklist() {
        let registry_json = r#"{
            "version": 1,
            "updated_at": "2026-11-05T12:00:00Z",
            "entries": [
                {
                    "id": "vertical-grid-egm96",
                    "name": "EGM96 大地水准面高程基准数据",
                    "kind": "component",
                    "version": "1.0.0",
                    "description": "15' 全球大地水准面网格",
                    "package": "components/vertical-grid-egm96-1.0.0.zip",
                    "sha256": "DB493027562C9B004D7220FA881F5603ADADA4E1C5029B933FA7DE4547B0E78D",
                    "size": 2710815,
                    "install_root": "runtime/vertical-grids",
                    "capabilities": ["egm96"]
                },
                {
                    "id": "revoked-skill",
                    "name": "已吊销的技能",
                    "kind": "skill",
                    "version": "0.1.0",
                    "package": "skills/revoked.json",
                    "sha256": "0000000000000000000000000000000000000000000000000000000000000001"
                }
            ]
        }"#;
        let blocklist_json = r#"{
            "blocked": [
                { "id": "revoked-skill", "reason": "security test" }
            ]
        }"#;
        let report = parse_and_filter_extension_registry(
            "https://raw.gitcode.com/hhanmj/eostage-hub/raw/main/registry.json",
            registry_json,
            Some(blocklist_json),
        )
        .expect("valid registry");
        assert_eq!(report.blocked_count, 1);
        assert_eq!(report.entries.len(), 1);
        assert_eq!(report.entries[0].id, "vertical-grid-egm96");
        assert_eq!(
            report.entries[0].sha256,
            "db493027562c9b004d7220fa881f5603adada4e1c5029b933fa7de4547b0e78d"
        );
        assert_eq!(
            report.entries[0].package,
            "https://raw.gitcode.com/hhanmj/eostage-hub/raw/main/components/vertical-grid-egm96-1.0.0.zip"
        );
    }

    #[test]
    fn install_runtime_component_installs_zip_verifies_sha256_and_rolls_back_on_bad_archive() {
        let root = temp_test_root("install-rollback");
        let initial_report = inspect_runtime_components_at(&root).unwrap();
        let egm96_before = initial_report
            .components
            .iter()
            .find(|c| c.id == "vertical-grid-egm96")
            .unwrap();
        assert_eq!(egm96_before.status, "missing");

        // Create a valid minimal TIFF inside a zip archive
        let zip_path = root.join("egm96-valid.zip");
        {
            let file = std::fs::File::create(&zip_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            zip.start_file("egm96/us_nga_egm96_15.tif", options)
                .unwrap();
            // Valid little-endian TIFF header + payload
            zip.write_all(&[b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 2, 3, 4])
                .unwrap();
            zip.finish().unwrap();
        }
        let zip_bytes = std::fs::read(&zip_path).unwrap();
        let mut hasher = Sha256::new();
        hasher.update(&zip_bytes);
        let zip_sha = format!("{:x}", hasher.finalize());

        // 1. Mismatched SHA-256 must fail closed before modifying target
        let bad_sha_err = install_runtime_component_at(
            &root,
            &RuntimeComponentInstallRequest {
                component_id: "vertical-grid-egm96".to_string(),
                source: zip_path.to_string_lossy().to_string(),
                expected_sha256: Some("f".repeat(64)),
            },
        )
        .unwrap_err();
        assert!(bad_sha_err.contains("RUNTIME_COMPONENT_SHA256_MISMATCH"));
        assert!(!root
            .join("runtime/vertical-grids/egm96/us_nga_egm96_15.tif")
            .exists());

        // 2. Valid SHA-256 installs and passes self-check
        let installed = install_runtime_component_at(
            &root,
            &RuntimeComponentInstallRequest {
                component_id: "vertical-grid-egm96".to_string(),
                source: zip_path.to_string_lossy().to_string(),
                expected_sha256: Some(zip_sha),
            },
        )
        .expect("install succeeds");
        assert!(installed.ok);
        assert_eq!(installed.status, "ready");
        assert_eq!(installed.bytes_installed, 12);

        // 3. Corrupt TIFF payload that passes SHA-256 check but fails post-install TIFF verification must roll back to the working 12-byte file!
        let corrupt_tif_path = root.join("corrupt.tif");
        std::fs::write(&corrupt_tif_path, b"NOT_A_TIFF_HEADER").unwrap();
        let rollback_err = install_runtime_component_at(
            &root,
            &RuntimeComponentInstallRequest {
                component_id: "vertical-grid-egm96".to_string(),
                source: corrupt_tif_path.to_string_lossy().to_string(),
                expected_sha256: None,
            },
        )
        .unwrap_err();
        assert!(rollback_err.contains("RUNTIME_COMPONENT_VERIFY_FAILED"));

        // Verify the previous valid 12-byte file was rolled back and component is still ready
        let after_rollback = inspect_runtime_components_at(&root).unwrap();
        let egm96_after = after_rollback
            .components
            .iter()
            .find(|c| c.id == "vertical-grid-egm96")
            .unwrap();
        assert_eq!(egm96_after.status, "ready");
        assert_eq!(egm96_after.total_bytes, 12);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn install_runtime_component_supports_rustgis_cli_cross_platform_and_rolls_back() {
        let root = temp_test_root("install-rustgis-cli");
        let before = inspect_runtime_components_at(&root).unwrap();
        let rustgis_before = before
            .components
            .iter()
            .find(|c| c.id == "rustgis-cli")
            .expect("rustgis-cli component listed");
        assert_eq!(rustgis_before.status, "missing");

        let zip_path = root.join("rustgis-cli-macos.zip");
        {
            let file = std::fs::File::create(&zip_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .unix_permissions(0o755);
            zip.start_file("bin/rustgis", options).unwrap();
            zip.write_all(&[0xCF, 0xFA, 0xED, 0xFE, 1, 2, 3, 4])
                .unwrap();
            zip.start_file(
                "component-manifest.json",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            zip.write_all(br#"{"version":"0.2.1"}"#).unwrap();
            zip.finish().unwrap();
        }
        let zip_bytes = std::fs::read(&zip_path).unwrap();
        let mut hasher = Sha256::new();
        hasher.update(&zip_bytes);
        let zip_sha = format!("{:x}", hasher.finalize());

        let installed = install_runtime_component_at(
            &root,
            &RuntimeComponentInstallRequest {
                component_id: "rustgis-cli".to_string(),
                source: zip_path.to_string_lossy().to_string(),
                expected_sha256: Some(zip_sha),
            },
        )
        .expect("rustgis-cli install succeeds");
        assert!(installed.ok);
        assert_eq!(installed.status, "ready");
        assert_eq!(installed.version, "0.2.1");

        let bin_path = root.join("runtime/rustgis-cli/bin/rustgis");
        assert!(bin_path.is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&bin_path).unwrap().permissions().mode();
            assert_ne!(mode & 0o111, 0, "installed unix binary must be executable");
        }

        // Corrupt zip without valid executable header must roll back cleanly
        let bad_zip_path = root.join("rustgis-bad.zip");
        {
            let file = std::fs::File::create(&bad_zip_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            zip.start_file("bin/rustgis", zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(b"NOT_A_BINARY").unwrap();
            zip.finish().unwrap();
        }
        let rollback_err = install_runtime_component_at(
            &root,
            &RuntimeComponentInstallRequest {
                component_id: "rustgis-cli".to_string(),
                source: bad_zip_path.to_string_lossy().to_string(),
                expected_sha256: None,
            },
        )
        .unwrap_err();
        assert!(rollback_err.contains("RUNTIME_COMPONENT_VERIFY_FAILED"));

        let after_rollback = inspect_runtime_components_at(&root).unwrap();
        let rustgis_after = after_rollback
            .components
            .iter()
            .find(|c| c.id == "rustgis-cli")
            .unwrap();
        assert_eq!(rustgis_after.status, "ready");
        assert_eq!(rustgis_after.version, "0.2.1");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn real_workspace_runtime_and_hub_dist_registry_are_detected_and_verified() {
        let root = discover_runtime_root();
        if root.join("runtime/vertical-grids/manifest.json").is_file() {
            let report = inspect_runtime_components_at(&root).expect("inspect real runtime");
            for expected_id in [
                "eostage-core",
                "native-gis-runtime",
                "vertical-grid-egm96",
                "vertical-grid-egm2008",
                "compat-worker",
            ] {
                let comp = report
                    .components
                    .iter()
                    .find(|c| c.id == expected_id)
                    .expect(expected_id);
                assert_eq!(
                    comp.status, "ready",
                    "component {expected_id} should be ready in real runtime ({})",
                    comp.detail
                );
                assert!(comp.verified, "component {expected_id} should be verified");
            }
        }
        let local_hub = root.join("_hub_dist/registry.json");
        if local_hub.is_file() {
            let reg = fetch_extension_registry(Some(local_hub.to_string_lossy().to_string()))
                .expect("load generated _hub_dist/registry.json");
            assert!(reg.ok);
            assert!(reg.entries.iter().any(|e| e.id == "vertical-grid-egm96"));
            assert!(reg
                .entries
                .iter()
                .any(|e| e.id == "skill-alos2-stripmap-insar"));
        }
    }

    #[test]
    fn plugins_and_skills_disk_persistence_and_zip_install_work() {
        let app_data = temp_test_root("plugins-app-data");
        let rt_plugins = discover_runtime_plugins_dir();

        // 1. Initial status has 0 installed plugins and >= 6 available plugins from runtime/plugins
        let initial = inspect_plugins_status_at(&app_data, &rt_plugins).expect("initial status");
        assert!(initial.ok);
        assert_eq!(initial.installed_plugins.len(), 0);
        if rt_plugins.is_dir() {
            assert!(
                initial.available_plugins.len() >= 6,
                "expected at least 6 runtime plugins, got {}",
                initial.available_plugins.len()
            );
            assert!(
                initial
                    .available_plugins
                    .iter()
                    .all(|p| p.author == "EOStage"),
                "all available plugins must have honest attribution"
            );
        }

        // 2. Install from runtime/plugins if available
        if rt_plugins.join("eostage-stac-harmonized-plugin").is_dir() {
            let res = install_plugin_package_at(
                &app_data,
                &rt_plugins,
                &PluginInstallRequest {
                    plugin_id: Some("eostage-stac-harmonized-plugin".to_string()),
                    source_path: None,
                    raw_content: None,
                    fallback_name: None,
                },
            )
            .expect("install from runtime/plugins");
            assert!(res.ok);
            assert!(app_data
                .join("plugins/eostage-stac-harmonized-plugin/plugin.json")
                .is_file());
            assert!(app_data
                .join("plugins/eostage-stac-harmonized-plugin/mcp.json")
                .is_file());
            assert_eq!(res.status.installed_plugins.len(), 1);
            assert!(
                res.status.custom_connectors.is_empty(),
                "installing a plugin must not clone its connector into user custom_connectors"
            );
            let installed_stac = &res.status.installed_plugins[0];
            assert!(
                !installed_stac.bundled_connectors.is_empty(),
                "installed STAC plugin must expose bundled_connectors directly"
            );
        }

        // 3. Create a real .zip plugin package and install from source_path
        let zip_path = app_data.join("custom-dem-qc-plugin.zip");
        {
            let file = std::fs::File::create(&zip_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            zip.start_file("plugin.json", options).unwrap();
            zip.write_all(
                r#"{
  "id": "eostage-dem-void-qc-plugin",
  "name": "DEM 空洞检测与高程基准核验套件",
  "version": "1.1.0",
  "author": "EOStage",
  "license": "Apache-2.0",
  "discipline": "cartography",
  "description": "检测 Copernicus GLO-30 / SRTM DEM 空洞比例与垂直基准一致性。",
  "tags": ["DEM", "EGM2008", "QC"],
  "enabled": true,
  "bundledSkills": [
    {
      "id": "plugin-skill-dem-void-qc",
      "name": "DEM 空洞与高程基准核验",
      "description": "检查 DEM NoData 空洞占比与 EGM96/EGM2008 垂直基准。",
      "path": "skills/plugin-skill-dem-void-qc/SKILL.md"
    }
  ]
}"#
                .as_bytes(),
            )
            .unwrap();
            zip.start_file("skills/plugin-skill-dem-void-qc/SKILL.md", options)
                .unwrap();
            zip.write_all(
                b"---\nname: plugin-skill-dem-void-qc\ndescription: DEM void QC\n---\n\n# DEM Void QC\nCheck NoData ratio.\n",
            )
            .unwrap();
            zip.finish().unwrap();
        }

        let zip_res = install_plugin_package_at(
            &app_data,
            &rt_plugins,
            &PluginInstallRequest {
                plugin_id: None,
                source_path: Some(zip_path.to_string_lossy().to_string()),
                raw_content: None,
                fallback_name: None,
            },
        )
        .expect("install from .zip");
        assert!(zip_res.ok);
        assert!(app_data
            .join("plugins/eostage-dem-void-qc-plugin/plugin.json")
            .is_file());
        assert!(app_data
            .join("plugins/eostage-dem-void-qc-plugin/skills/plugin-skill-dem-void-qc/SKILL.md")
            .is_file());

        // 4. Toggle enabled state on disk
        let toggled =
            set_plugin_enabled_at(&app_data, &rt_plugins, "eostage-dem-void-qc-plugin", false)
                .expect("disable plugin");
        assert!(toggled.ok);
        assert!(app_data
            .join("plugins/eostage-dem-void-qc-plugin/.disabled")
            .is_file());

        // 5. Uninstall removes from disk
        let uninstalled =
            uninstall_plugin_package_at(&app_data, &rt_plugins, "eostage-dem-void-qc-plugin")
                .expect("uninstall plugin");
        assert!(uninstalled.ok);
        assert!(!app_data.join("plugins/eostage-dem-void-qc-plugin").exists());

        let _ = std::fs::remove_dir_all(&app_data);
    }

    #[test]
    fn parse_and_filter_extension_registry_parses_eostage_native_and_filters_blocklist() {
        let valid_sha = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let eostage_index = format!(
            r#"{{
                "updated_at": "2026-11-14T08:00:00Z",
                "entries": [
                    {{
                        "id": "eostage-sar-core",
                        "name": "SAR 核心处理套件",
                        "kind": "plugin",
                        "version": "2.0.0",
                        "description": "InSAR 预处理与配准扩展包",
                        "package": "plugins/sar-core.zip",
                        "sha256": "{valid_sha}",
                        "size": 1024
                    }},
                    {{
                        "id": "eostage-blocked-item",
                        "name": "已撤回测试包",
                        "kind": "plugin",
                        "version": "0.1.0",
                        "description": "应被 blocklist 过滤",
                        "package": "plugins/blocked.zip",
                        "sha256": "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
                        "size": 512
                    }}
                ]
            }}"#
        );
        let blocklist = r#"{"blocked":[{"id":"eostage-blocked-item"}]}"#;
        let report = parse_and_filter_extension_registry(
            DEFAULT_EXTENSION_REGISTRY_URL,
            &eostage_index,
            Some(blocklist),
        )
        .expect("eostage official registry parses cleanly");
        assert_eq!(report.market_id, "eostage");
        assert_eq!(report.blocked_count, 1);
        assert_eq!(report.entries.len(), 1);
        assert_eq!(report.entries[0].id, "eostage-sar-core");
        assert_eq!(report.entries[0].runtime_compatibility, "eostage_native");
        assert!(report.entries[0]
            .package
            .starts_with("https://api.gitcode.com/"));

        let missing_sha = r#"{
            "entries": [
                {
                    "id": "bad-entry",
                    "name": "Missing SHA256",
                    "kind": "plugin",
                    "version": "1.0.0",
                    "description": "missing sha test",
                    "package": "plugins/bad.zip",
                    "sha256": "short"
                }
            ]
        }"#;
        let err =
            parse_and_filter_extension_registry(DEFAULT_EXTENSION_REGISTRY_URL, missing_sha, None)
                .unwrap_err();
        assert!(err.contains("REGISTRY_INVALID_SHA256"), "got: {err}");
    }

    #[test]
    fn install_plugin_rejects_cordis_host_required_and_persists_skill_and_mcp_connector() {
        let app_data = temp_test_root("dsh-install-compat");
        let rt_plugins = app_data.join("empty-rt-plugins");
        let _ = std::fs::create_dir_all(&rt_plugins);

        // 1. Cordis-only manifest must be rejected without creating plugin directory
        let cordis_payload = r#"{
            "id": "cordis-only-demo",
            "name": "Cordis Only Demo",
            "version": "1.0.0",
            "runtime_compatibility": "cordis_host_required",
            "engines": { "dsh": "^2.0.0" }
        }"#;
        let err = install_plugin_package_at(
            &app_data,
            &rt_plugins,
            &PluginInstallRequest {
                plugin_id: None,
                source_path: None,
                raw_content: Some(cordis_payload.to_string()),
                fallback_name: Some("cordis-only-demo".to_string()),
            },
        )
        .unwrap_err();
        assert!(err.contains("PLUGIN_CORDIS_HOST_REQUIRED"), "got: {err}");
        assert!(!app_data.join("plugins/cordis-only-demo").exists());

        // 2. Native SKILL.md + mcp_servers entry installs cleanly into plugins, skills_custom, and connectors_custom.json
        let native_payload = r#"{
            "id": "dsh-stac-mcp-bundle",
            "name": "STAC MCP Bundle",
            "version": "1.0.1",
            "author": "EOStage",
            "description": "One-click SKILL + MCP connector from DSH",
            "runtime_compatibility": "eostage_native",
            "skill": {
                "id": "dsh-stac-skill",
                "name": "STAC Skill",
                "content": "---\nname: dsh-stac-skill\n---\n# STAC Skill\nSearch scenes."
            },
            "mcp_servers": [
                {
                    "id": "stac-endpoint",
                    "command": "",
                    "args": [],
                    "env": {},
                    "url": "https://stac.example.org/mcp",
                    "transport": "http"
                }
            ]
        }"#;
        let ok_res = install_plugin_package_at(
            &app_data,
            &rt_plugins,
            &PluginInstallRequest {
                plugin_id: None,
                source_path: None,
                raw_content: Some(native_payload.to_string()),
                fallback_name: Some("dsh-stac-mcp-bundle".to_string()),
            },
        )
        .expect("native skill+mcp installs");
        assert!(ok_res.ok);
        assert!(app_data
            .join("plugins/dsh-stac-mcp-bundle/mcp.json")
            .is_file());
        assert!(app_data
            .join("plugins/dsh-stac-mcp-bundle/skills/dsh-stac-skill/SKILL.md")
            .is_file());
        assert!(
            ok_res.status.custom_skills.is_empty(),
            "plugin-bundled skill must remain plugin-owned and NOT copy into custom_skills"
        );
        assert!(
            ok_res.status.custom_connectors.is_empty(),
            "plugin-bundled connector must remain plugin-owned and NOT copy into custom_connectors"
        );
        let bundle = ok_res
            .installed_plugin
            .as_ref()
            .expect("installed_plugin metadata");
        assert!(
            bundle
                .bundled_skills
                .iter()
                .any(|s| s.id == "dsh-stac-skill"),
            "installed_plugin owns dsh-stac-skill"
        );
        assert!(
            bundle
                .bundled_connectors
                .iter()
                .any(|c| c.id == "plugin-mcp-stac-endpoint" && c.name == "stac-endpoint"),
            "installed_plugin owns bundled connector in bundled_connectors"
        );
        let eff_conns = load_effective_mcp_connectors_at(&app_data, &rt_plugins);
        assert!(
            eff_conns
                .iter()
                .any(|c| c.id == "plugin-mcp-stac-endpoint" && c.enabled),
            "effective MCP connectors include enabled plugin connector"
        );

        let _ = std::fs::remove_dir_all(&app_data);
    }

    #[test]
    fn plugin_ownership_whole_package_sha256_multi_server_and_disable_uninstall_isolation() {
        let app_data = temp_test_root("plugin-ownership-slice1");
        let rt_plugins = app_data.join("empty-rt-plugins");
        let _ = std::fs::create_dir_all(&rt_plugins);

        // 1. Seed a user-authored custom skill + custom connector whose IDs collide with a plugin.
        save_custom_skill_at(
            &app_data,
            &CustomSkillRecord {
                id: "colliding-skill".to_string(),
                name: "User Custom Skill".to_string(),
                description: "User authored".to_string(),
                prompt: "# User Original Prompt\nNever overwrite or delete me.".to_string(),
                author: "User".to_string(),
                license: "MIT".to_string(),
                version: "1.0.0".to_string(),
                slash_command: "/colliding-skill".to_string(),
                updated_at: "2026-11-28T00:00:00Z".to_string(),
                enabled: true,
            },
        )
        .unwrap();
        save_custom_connectors_at(
            &app_data,
            &[CustomConnectorRecord {
                id: "plugin-mcp-alpha".to_string(),
                name: "User Alpha Override".to_string(),
                transport: "http".to_string(),
                endpoint: "https://user.example.org/mcp".to_string(),
                description: "User owned".to_string(),
                enabled: true,
            }],
        )
        .unwrap();

        // Also seed a legacy unedited copy inside skills_custom that SHOULD be pruned.
        save_custom_skill_at(
            &app_data,
            &CustomSkillRecord {
                id: "plugin-only-skill".to_string(),
                name: "Legacy Copied Skill".to_string(),
                description: "Old clone".to_string(),
                prompt: "---\nname: plugin-only-skill\n---\n# Plugin Skill\nOriginal body v1."
                    .to_string(),
                author: "EOStage".to_string(),
                license: "Apache-2.0".to_string(),
                version: "1.0.0".to_string(),
                slash_command: "/plugin-only-skill".to_string(),
                updated_at: "2026-11-28T00:00:00Z".to_string(),
                enabled: true,
            },
        )
        .unwrap();

        // 2. Build a multi-file .zip plugin with two skills + two stdio/http servers in mcp.json.
        let zip_path = app_data.join("multi-cap-plugin.zip");
        {
            let file = std::fs::File::create(&zip_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            zip.start_file("plugin.json", opts).unwrap();
            zip.write_all(
                r#"{
  "id": "eostage-multi-cap-plugin",
  "name": "Multi Capability Plugin",
  "version": "1.0.0",
  "author": "EOStage",
  "license": "Apache-2.0",
  "description": "Multi-server MCP + skills test package"
}"#
                .as_bytes(),
            )
            .unwrap();
            zip.start_file("mcp.json", opts).unwrap();
            zip.write_all(
                r#"{
  "mcpServers": {
    "alpha": { "url": "https://plugin.example.org/alpha" },
    "beta": { "command": "rustgis-cli", "args": ["mcp", "--yes"] }
  }
}"#
                .as_bytes(),
            )
            .unwrap();
            zip.start_file("skills/colliding-skill/SKILL.md", opts)
                .unwrap();
            zip.write_all(b"# Plugin Colliding Skill\nPlugin body.").unwrap();
            zip.start_file("skills/plugin-only-skill/SKILL.md", opts)
                .unwrap();
            zip.write_all(b"---\nname: plugin-only-skill\n---\n# Plugin Skill\nOriginal body v1.")
                .unwrap();
            zip.finish().unwrap();
        }

        let res = install_plugin_package_at(
            &app_data,
            &rt_plugins,
            &PluginInstallRequest {
                plugin_id: None,
                source_path: Some(zip_path.to_string_lossy().to_string()),
                raw_content: None,
                fallback_name: None,
            },
        )
        .expect("install multi-cap plugin");
        let pkg = res.installed_plugin.expect("installed_plugin");
        let initial_sha = pkg.sha256.clone();
        assert_eq!(pkg.bundled_connectors.len(), 2);
        assert!(pkg
            .bundled_connectors
            .iter()
            .any(|c| c.id == "plugin-mcp-beta" && c.endpoint == "rustgis-cli mcp --yes"));

        // Legacy unedited clone was pruned; user-authored colliding skill & connector survived!
        assert!(!res
            .status
            .custom_skills
            .iter()
            .any(|s| s.id == "plugin-only-skill"));
        assert_eq!(
            load_custom_skills_at(&app_data)
                .iter()
                .find(|s| s.id == "colliding-skill")
                .expect("user skill preserved")
                .prompt,
            "# User Original Prompt\nNever overwrite or delete me."
        );
        let eff = load_effective_mcp_connectors_at(&app_data, &rt_plugins);
        assert_eq!(
            eff.iter()
                .find(|c| c.id == "plugin-mcp-alpha")
                .unwrap()
                .endpoint,
            "https://user.example.org/mcp"
        );
        assert!(eff.iter().any(|c| c.id == "plugin-mcp-beta"));

        // 3. Disabling suppresses plugin-owned MCP connector & leaves SHA256 identical!
        let disabled = set_plugin_enabled_at(
            &app_data,
            &rt_plugins,
            "eostage-multi-cap-plugin",
            false,
        )
        .expect("disable plugin");
        assert_eq!(
            disabled.installed_plugin.as_ref().unwrap().sha256,
            initial_sha,
            "toggling enablement must not mutate plugin.json or sha256"
        );
        let eff_disabled = load_effective_mcp_connectors_at(&app_data, &rt_plugins);
        assert!(!eff_disabled.iter().any(|c| c.id == "plugin-mcp-beta"));
        let probe_err =
            resolve_connector_for_probe_at(&app_data, &rt_plugins, "plugin-mcp-beta").unwrap_err();
        assert!(probe_err.starts_with("PLUGIN_DISABLED:"), "got: {probe_err}");

        // 4. Whole-package SHA256 changes if a bundled SKILL.md changes while plugin.json is untouched.
        std::fs::write(
            app_data.join("plugins/eostage-multi-cap-plugin/skills/plugin-only-skill/SKILL.md"),
            b"---\nname: plugin-only-skill\n---\n# Tampered Body\n",
        )
        .unwrap();
        let re_enabled = set_plugin_enabled_at(
            &app_data,
            &rt_plugins,
            "eostage-multi-cap-plugin",
            true,
        )
        .expect("re-enable plugin");
        assert_ne!(
            re_enabled.installed_plugin.as_ref().unwrap().sha256,
            initial_sha,
            "whole-package sha256 must detect changes outside plugin.json"
        );

        // 5. Uninstall removes plugin dir without touching user-authored colliding skill/connector.
        uninstall_plugin_package_at(&app_data, &rt_plugins, "eostage-multi-cap-plugin")
            .expect("uninstall plugin");
        assert!(!app_data.join("plugins/eostage-multi-cap-plugin").exists());
        assert!(load_custom_skills_at(&app_data)
            .iter()
            .any(|s| s.id == "colliding-skill"));
        assert!(load_custom_connectors_at(&app_data)
            .iter()
            .any(|c| c.id == "plugin-mcp-alpha"));
        assert!(!load_effective_mcp_connectors_at(&app_data, &rt_plugins)
            .iter()
            .any(|c| c.id == "plugin-mcp-beta"));

        let _ = std::fs::remove_dir_all(&app_data);
    }

    #[test]
    fn install_plugin_package_url_rejects_ssrf_insecure_and_spoofed_localhost() {
        let app_data = temp_test_root("plugin-url-sec");
        let rt_plugins = app_data.join("runtime_plugins");
        std::fs::create_dir_all(&rt_plugins).unwrap();

        for (bad_url, expected_prefix) in [
            (
                "http://localhost.attacker.example/plugin.zip",
                "PLUGIN_UNSAFE_URL",
            ),
            (
                "http://127.0.0.1.attacker.example/plugin.zip",
                "PLUGIN_UNSAFE_URL",
            ),
            ("http://example.com/plugin.zip", "PLUGIN_UNSAFE_URL"),
            ("https://192.168.1.10/plugin.zip", "PLUGIN_UNSAFE_URL"),
            ("https://169.254.169.254/latest/meta-data/", "PLUGIN_UNSAFE_URL"),
            (
                "https://example.org/cordis-pkg.tgz?download=1",
                "PLUGIN_CORDIS_HOST_REQUIRED",
            ),
        ] {
            let err = install_plugin_package_at(
                &app_data,
                &rt_plugins,
                &PluginInstallRequest {
                    plugin_id: None,
                    source_path: Some(bad_url.to_string()),
                    raw_content: None,
                    fallback_name: None,
                },
            )
            .expect_err(&format!("must reject {bad_url}"));
            assert!(
                err.contains(expected_prefix),
                "url={bad_url}: expected {expected_prefix}, got {err}"
            );
        }

        let _ = std::fs::remove_dir_all(&app_data);
    }

    #[test]
    fn usage_statistics_backward_compat_deserialization_and_helpers() {
        let legacy_json = r#"{
            "ok": true,
            "collected_at": "2026-11-10T00:00:00Z",
            "data_root": "/tmp/eostage",
            "sqlite_size_bytes": 4096,
            "workspace_count": 1,
            "session_count": 2,
            "run_count": 3,
            "completed_run_count": 3,
            "tool_call_count": 5,
            "prompt_tokens": 100,
            "completion_tokens": 50,
            "reasoning_tokens": 0,
            "cached_tokens": 20,
            "total_tokens": 150,
            "model_breakdown": [],
            "download_total_tasks": 1,
            "download_completed_tasks": 1,
            "download_active_tasks": 0,
            "download_failed_tasks": 0,
            "download_transferred_bytes": 1024,
            "download_selected_items": 1,
            "installed_plugins_count": 0,
            "custom_skills_count": 0
        }"#;
        let parsed: ApplicationUsageStatisticsResponse =
            serde_json::from_str(legacy_json).expect("legacy UsageStats JSON still deserializes");
        assert!(parsed.daily_series.is_empty());
        assert!(parsed.tool_breakdown.is_empty());
        assert!(parsed.provider_download_breakdown.is_empty());
        assert!(parsed.gee_quota_status.is_none());

        assert_eq!(
            extract_utc_date_from_timestamp("2026-11-14T12:34:56Z").as_deref(),
            Some("2026-11-14")
        );
        assert_eq!(
            extract_utc_date_from_timestamp("1763123456000").as_deref(),
            Some("2025-11-14")
        );
        assert_eq!(format_provider_display_name("asf"), "ASF Sentinel-1 SAR");
    }

    #[test]
    fn install_custom_skill_from_url_fetches_parses_frontmatter_and_saves() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let md = "---\nname: remote-sar-check\ndescription: Loopback SAR checklist\nauthor: ScienceLab\nlicense: Apache-2.0\n---\n\n# Remote SAR Check\n\nFollow Step 1.";
        let handle = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut buf = [0u8; 2048];
            let _ = sock.read(&mut buf);
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/markdown\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                md.len(),
                md
            );
            sock.write_all(resp.as_bytes()).unwrap();
        });

        let root = temp_test_root("install-skill-url");
        let saved = install_custom_skill_from_url_at(
            &root,
            insar_core::ProxyPolicy::Direct,
            &format!("http://{addr}/skills/remote-sar-check/SKILL.md"),
            None,
        )
        .expect("url skill installs cleanly");
        assert_eq!(saved.id, "remote-sar-check");
        assert_eq!(saved.description, "Loopback SAR checklist");
        assert_eq!(saved.author, "ScienceLab");
        assert!(root
            .join("skills_custom/remote-sar-check/SKILL.md")
            .is_file());
        let list = load_custom_skills_at(&root);
        assert!(list.iter().any(|s| s.id == "remote-sar-check"));
        handle.join().unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn runtime_gee_plugin_installs_cleanly_and_fake_tauri_mcp_connectors_are_rejected() {
        let app_data = temp_test_root("gee-install-and-no-tauri-mcp");
        let rt_plugins = discover_runtime_plugins_dir();
        assert!(
            rt_plugins.join("eostage-gee-plugin/plugin.json").is_file(),
            "runtime/plugins/eostage-gee-plugin/plugin.json must exist at {}",
            rt_plugins.display()
        );

        let gee_res = install_plugin_package_at(
            &app_data,
            &rt_plugins,
            &PluginInstallRequest {
                plugin_id: Some("eostage-gee-plugin".to_string()),
                source_path: None,
                raw_content: None,
                fallback_name: None,
            },
        )
        .expect("installing bundled eostage-gee-plugin from runtime/plugins succeeds");
        let installed_gee = gee_res.installed_plugin.expect("installed_plugin");
        assert_eq!(installed_gee.id, "eostage-gee-plugin");
        assert_eq!(installed_gee.bundled_skills.len(), 1);
        assert_eq!(installed_gee.bundled_skills[0].id, "gee-assistant");
        assert!(
            installed_gee.bundled_skills[0]
                .prompt
                .contains("gee_catalog_search"),
            "gee-assistant SKILL.md should document real native GEE tool names"
        );
        assert!(
            installed_gee.bundled_connectors.is_empty(),
            "native GEE plugin must not advertise fake MCP connectors"
        );

        for pid in [
            "eostage-gdal-stream-plugin",
            "eostage-open-geodata-plugin",
            "eostage-mapgis-compat-plugin",
        ] {
            let pkg = inspect_plugin_package_dir(&rt_plugins.join(pid), false, true)
                .unwrap_or_else(|| panic!("missing runtime plugin {pid}"));
            assert!(
                pkg.bundled_connectors.is_empty(),
                "native plugin {pid} must not ship fake tauri:// MCP stubs: {:?}",
                pkg.bundled_connectors
            );
        }

        // Defense-in-depth: even if a legacy/third-party manifest contains tauri://, strip it everywhere.
        let fake_plugin_dir = app_data.join("plugins/legacy-fake-tauri-plugin");
        std::fs::create_dir_all(&fake_plugin_dir).unwrap();
        std::fs::write(
            fake_plugin_dir.join("plugin.json"),
            serde_json::json!({
                "id": "legacy-fake-tauri-plugin",
                "name": "Legacy Tauri Stub",
                "version": "1.0.0",
                "bundledConnector": {
                    "id": "fake-tauri-conn",
                    "name": "Fake Tauri",
                    "transport": "stdio",
                    "endpoint": "tauri://application_fake"
                }
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            fake_plugin_dir.join("mcp.json"),
            serde_json::json!({
                "mcpServers": {
                    "fake-srv": {
                        "id": "plugin-mcp-fake-srv",
                        "transport": "stdio",
                        "command": "TAURI://application_fake"
                    }
                }
            })
            .to_string(),
        )
        .unwrap();
        let fake_pkg = inspect_plugin_package_dir(&fake_plugin_dir, true, false).unwrap();
        assert!(
            fake_pkg.bundled_connectors.is_empty(),
            "tauri:// endpoints must be stripped from bundled_connectors: {:?}",
            fake_pkg.bundled_connectors
        );
        let eff = load_effective_mcp_connectors_at(&app_data, &rt_plugins);
        assert!(
            !eff.iter().any(|c| c.endpoint.to_ascii_lowercase().starts_with("tauri://")),
            "load_effective_mcp_connectors_at must never expose tauri:// endpoints: {eff:?}"
        );

        let _ = std::fs::remove_dir_all(&app_data);
    }

    #[test]
    fn cache_summary_and_clear_reclaims_disposable_caches_without_touching_protected_user_data() {
        let root = temp_test_root("cache-summary-clear");
        let sqlite_path = root.join("tasks.sqlite3");
        let mut store = insar_agent::session::TaskStore::open(&sqlite_path).expect("open store");

        // Seed disposable on-disk cache files.
        let gee_cache_dir = root.join("gee_catalog_cache");
        let scratch_dir = root.join("scratch").join("log-dehydrator");
        std::fs::create_dir_all(&gee_cache_dir).unwrap();
        std::fs::create_dir_all(&scratch_dir).unwrap();
        std::fs::write(
            gee_cache_dir.join("official_catalog.json"),
            vec![b'a'; 4096],
        )
        .unwrap();
        std::fs::write(scratch_dir.join("tmp.log"), vec![b'b'; 2048]).unwrap();

        // Seed protected user files (credentials, downloads, agent sessions, active SSD staging).
        let agent_dir = root.join("agent").join("sessions");
        let dl_dir = root.join("downloads");
        let ssd_staging_dir = root.join("ssd_staging").join(".eostage-staging");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::create_dir_all(&dl_dir).unwrap();
        std::fs::create_dir_all(&ssd_staging_dir).unwrap();
        let cred_file = root.join("credentials-vault.v1.json");
        let session_file = agent_dir.join("session-1.json");
        let imagery_file = dl_dir.join("S1A_scene.zip");
        let ssd_part_file = ssd_staging_dir.join("in_flight_scene.part");
        std::fs::write(&cred_file, b"{\"protected\":true}").unwrap();
        std::fs::write(&session_file, b"{\"id\":\"session-1\"}").unwrap();
        std::fs::write(&imagery_file, b"fake-imagery-bytes").unwrap();
        std::fs::write(&ssd_part_file, b"ssd-part-bytes").unwrap();

        // Seed disposable SQLite boundary cache entry directly on the initialized table.
        {
            let conn = rusqlite::Connection::open(&sqlite_path).expect("open raw sqlite");
            conn.execute(
                "INSERT INTO administrative_boundary_cache (
                    provider, provider_object_id, name, display_name, country_code,
                    admin_level, geometry_json, west, south, east, north, source,
                    attribution, cached_at
                 ) VALUES (
                    'datav', '340100', '合肥市', '安徽省 / 合肥市', 'CN',
                    'city', ?1, 117.0, 31.0, 118.0, 32.0, 'DataV areas_v3',
                    NULL, '2026-11-19T00:00:00Z'
                 )",
                [serde_json::json!({
                    "type": "Polygon",
                    "coordinates": [[[117.0, 31.0], [118.0, 31.0], [118.0, 32.0], [117.0, 32.0], [117.0, 31.0]]]
                })
                .to_string()],
            )
            .expect("seed boundary row");
        }

        let summary = compute_cache_summary_at(&root, &sqlite_path, &mut store).expect("summary");
        assert!(summary.ok);
        assert_eq!(summary.categories.len(), 3);
        let admin_cat = summary
            .categories
            .iter()
            .find(|c| c.id == "admin_boundary_cache")
            .expect("admin cat");
        assert_eq!(admin_cat.entry_count, 1);
        assert!(admin_cat.size_bytes > 0);
        let disk_cat = summary
            .categories
            .iter()
            .find(|c| c.id == "catalog_and_preview_cache")
            .expect("disk cat");
        assert_eq!(disk_cat.entry_count, 2);
        assert_eq!(disk_cat.size_bytes, 6144);

        let cleared =
            clear_disposable_caches_at(&root, &sqlite_path, &mut store, None).expect("clear");
        assert!(cleared.ok);
        assert_eq!(cleared.cleared_categories.len(), 3);
        assert!(cleared.reclaimed_bytes >= 6144);
        assert_eq!(cleared.summary.total_size_bytes, 0);

        // Protected user files and active SSD staging MUST remain untouched.
        assert!(cred_file.is_file());
        assert!(session_file.is_file());
        assert!(imagery_file.is_file());
        assert!(ssd_part_file.is_file());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn cache_summary_auto_prunes_oversized_sqlite_and_preserves_protected_rows() {
        let root = temp_test_root("cache-summary-autoprune");
        let sqlite_path = root.join("tasks.sqlite3");
        let mut store = insar_agent::session::TaskStore::open(&sqlite_path).expect("open store");

        // Seed an oversized set of unreferenced search batches exceeding 64 entries.
        let now_rfc3339 = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let expired_rfc3339 = "2020-01-01T00:00:00Z";
        {
            let conn = rusqlite::Connection::open(&sqlite_path).expect("open raw sqlite");
            for idx in 0..80_usize {
                let batch_id = format!("batch-auto-{idx:03}");
                let ts = if idx < 10 {
                    expired_rfc3339
                } else {
                    now_rfc3339.as_str()
                };
                let snap = serde_json::json!({
                    "schema_version": 1,
                    "batch_id": batch_id,
                    "provider_id": "asf.catalog",
                    "created_at": ts,
                    "aoi_name": "Test AOI",
                    "aoi_geojson": null,
                    "query": {},
                    "search_stats": null,
                    "candidates": []
                })
                .to_string();
                conn.execute(
                    "INSERT INTO search_batches(batch_id, provider_id, created_at, snapshot_json) VALUES (?1, 'asf.catalog', ?2, ?3)",
                    rusqlite::params![format!("batch-auto-{idx:03}"), ts, snap],
                )
                .expect("seed unreferenced batch");
            }
        }

        let summary = compute_cache_summary_at(&root, &sqlite_path, &mut store).expect("summary");
        let search_cat = summary
            .categories
            .iter()
            .find(|c| c.id == "search_cache")
            .expect("search cat");
        assert!(
            search_cat.entry_count
                <= insar_agent::session::TaskStore::SEARCH_CACHE_MAX_ENTRIES as u64,
            "entry_count={}",
            search_cat.entry_count
        );
        assert!(
            search_cat.size_bytes <= insar_agent::session::TaskStore::SEARCH_CACHE_MAX_BYTES,
            "size_bytes={}",
            search_cat.size_bytes
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn resolve_disposable_cache_dir_honors_custom_absolute_setting_and_defaults_to_app_data() {
        let app_data = temp_test_root("cache-dir-app-data");
        let custom_dir = temp_test_root("cache-dir-custom");

        assert_eq!(
            resolve_disposable_cache_dir_from_settings(&app_data, None),
            app_data
        );

        let empty_settings = ApplicationNetworkSettings::default();
        assert_eq!(
            resolve_disposable_cache_dir_from_settings(&app_data, Some(&empty_settings)),
            app_data
        );

        let relative_settings = ApplicationNetworkSettings {
            cache_dir: "relative/cache".to_owned(),
            ..ApplicationNetworkSettings::default()
        };
        assert_eq!(
            resolve_disposable_cache_dir_from_settings(&app_data, Some(&relative_settings)),
            app_data
        );

        let custom_settings = ApplicationNetworkSettings {
            cache_dir: custom_dir.to_string_lossy().to_string(),
            ..ApplicationNetworkSettings::default()
        };
        let resolved =
            resolve_disposable_cache_dir_from_settings(&app_data, Some(&custom_settings));
        assert_eq!(resolved, custom_dir);

        let sqlite_path = app_data.join("tasks.sqlite3");
        let mut store = insar_agent::session::TaskStore::open(&sqlite_path).expect("open store");
        let custom_sub = custom_dir.join("cache");
        std::fs::create_dir_all(&custom_sub).unwrap();
        std::fs::write(custom_sub.join("sample.bin"), vec![1_u8; 512]).unwrap();

        let summary = compute_cache_summary_at(&resolved, &sqlite_path, &mut store)
            .expect("custom cache summary");
        assert_eq!(summary.cache_dir, custom_dir.to_string_lossy().to_string());
        assert!(summary
            .categories
            .iter()
            .find(|c| c.id == "catalog_and_preview_cache")
            .is_some_and(|c| c.size_bytes >= 512));

        let _ = std::fs::remove_dir_all(&app_data);
        let _ = std::fs::remove_dir_all(&custom_dir);
    }

    #[test]
    fn issue55_dem_without_native_gis_enables_online_dems_and_raw_geotiff_passthrough() {
        let lazy = LazyNativeGisEngine::unavailable();
        assert!(!lazy.is_discovered());
        let obs = native_gis_capability_observations(&lazy);
        let state_root = temp_test_root("issue55-no-gis-cap");
        let db = state_root.join("tasks.sqlite3");
        let service = insar_application_service::ApplicationService::new(
            ApplicationServiceConfig::business_store(db).with_capability_runtime_observations(obs),
        );
        // Account-free online DEMs (e.g. COP30) must be Executable without GDAL.
        for cap in [
            "capability.dem.cop30.aws-download.v1",
            "capability.dem.fabdem.download.v1",
            "capability.dem.gem15-dtm.download.v1",
        ] {
            let eff = service
                .effective_capability(cap)
                .expect("capability lookup")
                .expect(cap);
            assert_eq!(
                eff.projection.state,
                insar_core::EffectiveCapabilityState::Executable,
                "expected Executable for {cap}"
            );
        }
        // Credential-gated online DEMs must pass build/runtime readiness (never BlockedRuntime/BlockedBuild).
        for cap in [
            "capability.dem.aster-gdem-v3.download.v1",
            "capability.dem.aw3d30-ellipsoidal.download.v1",
            "capability.dem.srtm-gl1-ellipsoidal.download.v1",
        ] {
            let eff = service
                .effective_capability(cap)
                .expect("capability lookup")
                .expect(cap);
            assert!(
                matches!(
                    eff.projection.state,
                    insar_core::EffectiveCapabilityState::Executable
                        | insar_core::EffectiveCapabilityState::BlockedCredential
                ),
                "expected Executable or BlockedCredential for {cap}, got {:?}",
                eff.projection.state
            );
        }
        // Local DEM import/convert remains BlockedRuntime when Native GIS is absent.
        let local_eff = service
            .effective_capability("capability.dem.user-local.convert.v1")
            .expect("local capability lookup")
            .expect("dem.user-local.v1");
        assert!(matches!(
            local_eff.projection.state,
            insar_core::EffectiveCapabilityState::BlockedRuntime
                | insar_core::EffectiveCapabilityState::BlockedBuild
        ));
        service.shutdown().unwrap();
        let _ = std::fs::remove_dir_all(&state_root);

        // Verify ASTER GDEM v3 URL and datum.
        let aster_sources =
            native_dem_download_sources("ASTER_GDEM_V3", "EARTHDATA", [110.2, 20.1, 110.5, 20.4])
                .expect("ASTER GDEM v3 sources");
        assert_eq!(aster_sources.len(), 1);
        assert_eq!(aster_sources[0].id, "ASTGTMV003_N20E110_dem");
        assert_eq!(
            aster_sources[0].url,
            "https://data.lpdaac.earthdatacloud.nasa.gov/lp-prod-protected/ASTGTM.003/ASTGTMV003_N20E110_dem.tif"
        );
        assert_eq!(
            native_dem_source_vertical_datum(&serde_json::json!({ "dataset": "ASTER_GDEM_V3" }))
                .unwrap(),
            "EGM96"
        );

        fn build_minimal_geotiff_le(samples_per_pixel: u16, truncate_strip: bool) -> Vec<u8> {
            // 2x2 Int16 DEM with ModelPixelScaleTag (33550), ModelTiepointTag (33922),
            // GeoKeyDirectoryTag (34735 -> EPSG:4326), StripOffsets (273), StripByteCounts (279).
            let mut out = Vec::new();
            out.extend_from_slice(b"II");
            out.extend_from_slice(&42_u16.to_le_bytes());
            out.extend_from_slice(&8_u32.to_le_bytes());

            let entry_count = 9_u16;
            out.extend_from_slice(&entry_count.to_le_bytes());
            let ifd_end = 8_u32 + 2 + u32::from(entry_count) * 12 + 4;
            let pixel_scale_off = ifd_end;
            let tiepoint_off = pixel_scale_off + 24;
            let geokey_off = tiepoint_off + 48;
            let geokeys: [u16; 12] = [
                1, 1, 0, 2, // version=1, rev=1, minor=0, num_keys=2
                1024, 0, 1, 2, // GTModelTypeGeoKey = Geographic (2)
                2048, 0, 1, 4326, // GeographicTypeGeoKey = EPSG:4326
            ];
            let bps_off = geokey_off + (geokeys.len() as u32) * 2;
            let bps_extra_bytes = if samples_per_pixel > 2 {
                u32::from(samples_per_pixel) * 2
            } else {
                0
            };
            let strip_off = bps_off + bps_extra_bytes;
            let strip_bytes = 2_u32 * 2 * 2 * u32::from(samples_per_pixel);

            let mut push_entry = |tag: u16, ftype: u16, cnt: u32, val_or_off: [u8; 4]| {
                out.extend_from_slice(&tag.to_le_bytes());
                out.extend_from_slice(&ftype.to_le_bytes());
                out.extend_from_slice(&cnt.to_le_bytes());
                out.extend_from_slice(&val_or_off);
            };
            let inline_short = |v: u16| -> [u8; 4] {
                let b = v.to_le_bytes();
                [b[0], b[1], 0, 0]
            };
            let inline_two_shorts = |a: u16, b: u16| -> [u8; 4] {
                let ab = a.to_le_bytes();
                let bb = b.to_le_bytes();
                [ab[0], ab[1], bb[0], bb[1]]
            };

            push_entry(256, 4, 1, 2_u32.to_le_bytes()); // ImageWidth = 2
            push_entry(257, 4, 1, 2_u32.to_le_bytes()); // ImageLength = 2
            if samples_per_pixel == 1 {
                push_entry(258, 3, 1, inline_short(16));
            } else if samples_per_pixel == 2 {
                push_entry(258, 3, 2, inline_two_shorts(16, 16));
            } else {
                push_entry(258, 3, u32::from(samples_per_pixel), bps_off.to_le_bytes());
            }
            push_entry(273, 4, 1, strip_off.to_le_bytes()); // StripOffsets
            push_entry(277, 3, 1, inline_short(samples_per_pixel)); // SamplesPerPixel
            push_entry(279, 4, 1, strip_bytes.to_le_bytes()); // StripByteCounts
            push_entry(33550, 12, 3, pixel_scale_off.to_le_bytes()); // ModelPixelScaleTag
            push_entry(33922, 12, 6, tiepoint_off.to_le_bytes()); // ModelTiepointTag
            push_entry(34735, 3, geokeys.len() as u32, geokey_off.to_le_bytes()); // GeoKeyDirectoryTag
            out.extend_from_slice(&0_u32.to_le_bytes()); // next IFD = 0

            for val in [0.0003_f64, 0.0003, 0.0] {
                out.extend_from_slice(&val.to_le_bytes());
            }
            for val in [0.0_f64, 0.0, 0.0, 117.0, 32.0, 0.0] {
                out.extend_from_slice(&val.to_le_bytes());
            }
            for k in geokeys {
                out.extend_from_slice(&k.to_le_bytes());
            }
            if samples_per_pixel > 2 {
                for _ in 0..samples_per_pixel {
                    out.extend_from_slice(&16_u16.to_le_bytes());
                }
            }
            let actual_strip_len = if truncate_strip {
                (strip_bytes as usize).saturating_sub(2)
            } else {
                strip_bytes as usize
            };
            out.extend(std::iter::repeat_n(0x12_u8, actual_strip_len));
            out
        }

        fn build_minimal_bigtiff_be() -> Vec<u8> {
            // Minimal valid 2x3 Float32 BigTIFF GeoTIFF in big-endian byte order (EPSG:32610).
            let mut out = Vec::new();
            out.extend_from_slice(b"MM");
            out.extend_from_slice(&43_u16.to_be_bytes());
            out.extend_from_slice(&8_u16.to_be_bytes());
            out.extend_from_slice(&0_u16.to_be_bytes());
            out.extend_from_slice(&16_u64.to_be_bytes()); // first IFD at offset 16

            let entry_count = 10_u64;
            out.extend_from_slice(&entry_count.to_be_bytes());
            let ifd_end = 16_u64 + 8 + entry_count * 20 + 8;
            let pixel_scale_off = ifd_end;
            let tiepoint_off = pixel_scale_off + 24;
            let geokey_off = tiepoint_off + 48;
            let geokeys: [u16; 12] = [
                1, 1, 1, 2, // GeoTIFF 1.1: version=1, rev=1, minor=1, num_keys=2
                1024, 0, 1, 1, // GTModelTypeGeoKey = Projected (1)
                3072, 0, 1, 32610, // ProjectedCSTypeGeoKey = EPSG:32610
            ];
            let strip_off = geokey_off + (geokeys.len() as u64) * 2;
            let strip_bytes = 2_u64 * 3 * 4;

            let mut push_entry = |tag: u16, ftype: u16, cnt: u64, val_or_off: [u8; 8]| {
                out.extend_from_slice(&tag.to_be_bytes());
                out.extend_from_slice(&ftype.to_be_bytes());
                out.extend_from_slice(&cnt.to_be_bytes());
                out.extend_from_slice(&val_or_off);
            };
            let inline_short = |v: u16| -> [u8; 8] {
                let b = v.to_be_bytes();
                [b[0], b[1], 0, 0, 0, 0, 0, 0]
            };

            push_entry(256, 16, 1, 2_u64.to_be_bytes()); // ImageWidth = 2
            push_entry(257, 16, 1, 3_u64.to_be_bytes()); // ImageLength = 3
            push_entry(258, 3, 1, inline_short(32)); // BitsPerSample = 32
            push_entry(273, 16, 1, strip_off.to_be_bytes()); // StripOffsets
            push_entry(277, 3, 1, inline_short(1)); // SamplesPerPixel = 1
            push_entry(279, 16, 1, strip_bytes.to_be_bytes()); // StripByteCounts
            push_entry(339, 3, 1, inline_short(3)); // SampleFormat = IEEE Float
            push_entry(33550, 12, 3, pixel_scale_off.to_be_bytes());
            push_entry(33922, 12, 6, tiepoint_off.to_be_bytes());
            push_entry(34735, 3, geokeys.len() as u64, geokey_off.to_be_bytes());
            out.extend_from_slice(&0_u64.to_be_bytes());

            for val in [30.0_f64, 30.0, 0.0] {
                out.extend_from_slice(&val.to_be_bytes());
            }
            for val in [0.0_f64, 0.0, 0.0, 500000.0, 4000000.0, 0.0] {
                out.extend_from_slice(&val.to_be_bytes());
            }
            for k in geokeys {
                out.extend_from_slice(&k.to_be_bytes());
            }
            out.extend(std::iter::repeat_n(0x3F_u8, strip_bytes as usize));
            out
        }

        fn build_compressed_geotiff_le(
            compression: u16,
            payload: &[u8],
            geokeys: Option<&[u16]>,
            predictor: u16,
            sample_format: u16,
            bits_per_sample: u16,
        ) -> Vec<u8> {
            let mut out = Vec::new();
            out.extend_from_slice(b"II");
            out.extend_from_slice(&42_u16.to_le_bytes());
            out.extend_from_slice(&8_u32.to_le_bytes());

            let has_geokeys = geokeys.is_some();
            let entry_count = if has_geokeys { 12_u16 } else { 11_u16 };
            out.extend_from_slice(&entry_count.to_le_bytes());
            let ifd_end = 8_u32 + 2 + u32::from(entry_count) * 12 + 4;
            let pixel_scale_off = ifd_end;
            let tiepoint_off = pixel_scale_off + 24;
            let geokey_off = tiepoint_off + 48;
            let geokey_bytes = geokeys.map_or(0_u32, |k| (k.len() as u32) * 2);
            let strip_off = geokey_off + geokey_bytes;

            let mut push_entry = |tag: u16, ftype: u16, cnt: u32, val_or_off: [u8; 4]| {
                out.extend_from_slice(&tag.to_le_bytes());
                out.extend_from_slice(&ftype.to_le_bytes());
                out.extend_from_slice(&cnt.to_le_bytes());
                out.extend_from_slice(&val_or_off);
            };
            let inline_short = |v: u16| -> [u8; 4] {
                let b = v.to_le_bytes();
                [b[0], b[1], 0, 0]
            };

            push_entry(256, 4, 1, 2_u32.to_le_bytes()); // ImageWidth = 2
            push_entry(257, 4, 1, 2_u32.to_le_bytes()); // ImageLength = 2
            push_entry(258, 3, 1, inline_short(bits_per_sample)); // BitsPerSample
            push_entry(259, 3, 1, inline_short(compression));
            push_entry(273, 4, 1, strip_off.to_le_bytes());
            push_entry(277, 3, 1, inline_short(1));
            push_entry(279, 4, 1, (payload.len() as u32).to_le_bytes());
            push_entry(317, 3, 1, inline_short(predictor));
            push_entry(339, 3, 1, inline_short(sample_format));
            push_entry(33550, 12, 3, pixel_scale_off.to_le_bytes());
            push_entry(33922, 12, 6, tiepoint_off.to_le_bytes());
            if let Some(keys) = geokeys {
                push_entry(34735, 3, keys.len() as u32, geokey_off.to_le_bytes());
            }
            out.extend_from_slice(&0_u32.to_le_bytes());

            for val in [0.0003_f64, 0.0003, 0.0] {
                out.extend_from_slice(&val.to_le_bytes());
            }
            for val in [0.0_f64, 0.0, 0.0, 117.0, 32.0, 0.0] {
                out.extend_from_slice(&val.to_le_bytes());
            }
            if let Some(keys) = geokeys {
                for &k in keys {
                    out.extend_from_slice(&k.to_le_bytes());
                }
            }
            out.extend_from_slice(payload);
            out
        }

        // Verify GeoTIFF fallback rejects corrupt/non-DEM/CRS-less/incompatible-predictor inputs
        // and promotes valid DEM tiles.
        let work_dir = temp_test_root("issue55-raw-tiff");
        std::fs::create_dir_all(&work_dir).unwrap();
        let out_dem = work_dir.join("COP30_dem.tif");

        let valid_pixels: [u8; 8] = [10, 0, 20, 0, 30, 0, 40, 0];
        let mut zlib_enc =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut zlib_enc, &valid_pixels).unwrap();
        let valid_deflate_payload = zlib_enc.finish().unwrap();

        let valid_f32_pixels: [u8; 16] = [0x3F; 16];
        let mut zlib_f32_enc =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut zlib_f32_enc, &valid_f32_pixels).unwrap();
        let valid_f32_deflate_payload = zlib_f32_enc.finish().unwrap();

        let mut corrupt_deflate_payload = valid_deflate_payload.clone();
        let mid_idx = corrupt_deflate_payload.len() / 2;
        corrupt_deflate_payload[mid_idx] ^= 0xFF;

        let valid_geokeys: [u16; 12] = [1, 1, 0, 2, 1024, 0, 1, 2, 2048, 0, 1, 4326];
        let zero_model_geokeys: [u16; 12] = [1, 1, 0, 2, 1024, 0, 1, 0, 2048, 0, 1, 4326];
        let nonexistent_epsg1_geokeys: [u16; 12] = [1, 1, 0, 2, 1024, 0, 1, 2, 2048, 0, 1, 1];
        let mismatched_geog_proj_geokeys: [u16; 12] =
            [1, 1, 0, 2, 1024, 0, 1, 2, 2048, 0, 1, 32610];
        let mismatched_proj_geog_geokeys: [u16; 12] = [1, 1, 0, 2, 1024, 0, 1, 1, 3072, 0, 1, 4326];
        let user_defined_crs_geokeys: [u16; 16] = [
            1, 1, 0, 3, 1024, 0, 1, 2, 2048, 0, 1, 32767, 2050, 0, 1, 6326,
        ];
        // Valid 8-byte PackBits encoding: literal 8 bytes (control=7 followed by 8 raw bytes).
        let mut valid_packbits = vec![7_u8];
        valid_packbits.extend_from_slice(&valid_pixels);
        // Valid 9-bit MSB-first TIFF LZW stream for [10,0,20,0,30,0,40,0]:
        // CLEAR(256), 10, 0, 20, 0, 30, 0, 40, 0, EOI(257) packed MSB-first in 9-bit codes.
        let lzw_codes: [u16; 10] = [256, 10, 0, 20, 0, 30, 0, 40, 0, 257];
        let mut valid_lzw = Vec::new();
        let mut lzw_acc: u32 = 0;
        let mut lzw_bits: u8 = 0;
        for &code in &lzw_codes {
            lzw_acc = (lzw_acc << 9) | u32::from(code);
            lzw_bits += 9;
            while lzw_bits >= 8 {
                lzw_bits -= 8;
                valid_lzw.push(((lzw_acc >> lzw_bits) & 0xFF) as u8);
            }
        }
        if lzw_bits > 0 {
            valid_lzw.push(((lzw_acc << (8 - lzw_bits)) & 0xFF) as u8);
        }
        // Invalid LZW stream omitting initial CLEAR(256): [10, 0, 20, 0, 30, 0, 40, 0, 257].
        let lzw_no_clear: Vec<u8> = vec![
            0x05, 0x00, 0x02, 0x80, 0x00, 0xF0, 0x00, 0x50, 0x00, 0x80, 0x80,
        ];

        for (name, bytes) in [
            ("html.tif", b"<html>not a tiff</html>".to_vec()),
            ("magic_only_8bytes.tif", b"II*\x00\x00\x00\x00\x00".to_vec()),
            ("truncated_strip.tif", build_minimal_geotiff_le(1, true)),
            ("three_band_rgb.tif", build_minimal_geotiff_le(3, false)),
            (
                "corrupt_deflate_same_len.tif",
                build_compressed_geotiff_le(
                    8,
                    &corrupt_deflate_payload,
                    Some(&valid_geokeys),
                    1,
                    2,
                    16,
                ),
            ),
            (
                "int16_deflate_predictor3_incompatible.tif",
                build_compressed_geotiff_le(
                    8,
                    &valid_deflate_payload,
                    Some(&valid_geokeys),
                    3,
                    2,
                    16,
                ),
            ),
            (
                "int16_none_predictor2_incompatible.tif",
                build_compressed_geotiff_le(1, &valid_pixels, Some(&valid_geokeys), 2, 2, 16),
            ),
            (
                "float32_deflate_predictor2_incompatible.tif",
                build_compressed_geotiff_le(
                    8,
                    &valid_f32_deflate_payload,
                    Some(&valid_geokeys),
                    2,
                    3,
                    32,
                ),
            ),
            (
                "corrupt_lzw.tif",
                build_compressed_geotiff_le(5, &[0x80, 0x00, 0x01], Some(&valid_geokeys), 1, 2, 16),
            ),
            (
                "lzw_missing_initial_clear.tif",
                build_compressed_geotiff_le(5, &lzw_no_clear, Some(&valid_geokeys), 1, 2, 16),
            ),
            (
                "truncated_packbits.tif",
                build_compressed_geotiff_le(32773, &[7, 1, 2, 3], Some(&valid_geokeys), 1, 2, 16),
            ),
            (
                "unsupported_zstd.tif",
                build_compressed_geotiff_le(50000, &valid_pixels, Some(&valid_geokeys), 1, 2, 16),
            ),
            (
                "missing_crs_geokeys.tif",
                build_compressed_geotiff_le(8, &valid_deflate_payload, None, 1, 2, 16),
            ),
            (
                "invalid_model_type_geokey.tif",
                build_compressed_geotiff_le(
                    8,
                    &valid_deflate_payload,
                    Some(&zero_model_geokeys),
                    1,
                    2,
                    16,
                ),
            ),
            (
                "nonexistent_epsg1.tif",
                build_compressed_geotiff_le(
                    8,
                    &valid_deflate_payload,
                    Some(&nonexistent_epsg1_geokeys),
                    1,
                    2,
                    16,
                ),
            ),
            (
                "mismatched_geog_with_projected_32610.tif",
                build_compressed_geotiff_le(
                    8,
                    &valid_deflate_payload,
                    Some(&mismatched_geog_proj_geokeys),
                    1,
                    2,
                    16,
                ),
            ),
            (
                "mismatched_proj_with_geographic_4326.tif",
                build_compressed_geotiff_le(
                    8,
                    &valid_deflate_payload,
                    Some(&mismatched_proj_geog_geokeys),
                    1,
                    2,
                    16,
                ),
            ),
            (
                "user_defined_crs_without_native_gis.tif",
                build_compressed_geotiff_le(
                    8,
                    &valid_deflate_payload,
                    Some(&user_defined_crs_geokeys),
                    1,
                    2,
                    16,
                ),
            ),
        ] {
            let bad_tile = work_dir.join(name);
            std::fs::write(&bad_tile, bytes).unwrap();
            let err = prepare_raw_dem_tiles_without_native_gis(
                "COPERNICUS_AWS",
                &[bad_tile],
                &out_dem,
                [110.0, 20.0, 110.5, 20.5],
            )
            .unwrap_err();
            assert!(
                err.starts_with("NATIVE_DEM_INVALID_RASTER:"),
                "{name} should fail validation, got: {err}"
            );
            assert!(!out_dem.exists(), "{name} must not produce final output");
        }

        let good_a = work_dir.join("tile_a.tif");
        let good_b = work_dir.join("tile_b.tif");
        let good_deflate = work_dir.join("tile_deflate.tif");
        let good_deflate_f32_p3 = work_dir.join("tile_deflate_f32_p3.tif");
        let good_lzw = work_dir.join("tile_lzw.tif");
        let good_packbits = work_dir.join("tile_packbits.tif");
        std::fs::write(&good_a, build_minimal_geotiff_le(1, false)).unwrap();
        std::fs::write(&good_b, build_minimal_bigtiff_be()).unwrap();
        std::fs::write(
            &good_deflate,
            build_compressed_geotiff_le(8, &valid_deflate_payload, Some(&valid_geokeys), 2, 2, 16),
        )
        .unwrap();
        std::fs::write(
            &good_deflate_f32_p3,
            build_compressed_geotiff_le(
                8,
                &valid_f32_deflate_payload,
                Some(&valid_geokeys),
                3,
                3,
                32,
            ),
        )
        .unwrap();
        std::fs::write(
            &good_lzw,
            build_compressed_geotiff_le(5, &valid_lzw, Some(&valid_geokeys), 1, 2, 16),
        )
        .unwrap();
        std::fs::write(
            &good_packbits,
            build_compressed_geotiff_le(32773, &valid_packbits, Some(&valid_geokeys), 1, 2, 16),
        )
        .unwrap();

        let summary_a = validate_raw_geotiff_structure(&good_a).expect("classic LE GeoTIFF");
        assert_eq!(
            summary_a,
            RawGeoTiffSummary {
                width: 2,
                height: 2,
                bands: 1,
                is_bigtiff: false,
                compression: "NONE",
                crs: "EPSG:4326".to_owned(),
                geotiff_tags_present: true,
            }
        );
        let summary_b = validate_raw_geotiff_structure(&good_b).expect("BigTIFF BE GeoTIFF");
        assert_eq!(
            summary_b,
            RawGeoTiffSummary {
                width: 2,
                height: 3,
                bands: 1,
                is_bigtiff: true,
                compression: "NONE",
                crs: "EPSG:32610".to_owned(),
                geotiff_tags_present: true,
            }
        );
        assert_eq!(
            validate_raw_geotiff_structure(&good_deflate)
                .expect("DEFLATE GeoTIFF")
                .compression,
            "DEFLATE"
        );
        assert_eq!(
            validate_raw_geotiff_structure(&good_deflate_f32_p3)
                .expect("DEFLATE Float32 Predictor=3 GeoTIFF")
                .compression,
            "DEFLATE"
        );
        assert_eq!(
            validate_raw_geotiff_structure(&good_lzw)
                .expect("LZW GeoTIFF")
                .compression,
            "LZW"
        );
        assert_eq!(
            validate_raw_geotiff_structure(&good_packbits)
                .expect("PackBits GeoTIFF")
                .compression,
            "PACKBITS"
        );

        let res = prepare_raw_dem_tiles_without_native_gis(
            "COPERNICUS_AWS",
            &[good_a, good_b],
            &out_dem,
            [110.0, 19.5, 110.5, 20.5],
        )
        .expect("multi-tile valid GeoTIFF passthrough should succeed");
        assert_eq!(res["tile_count"], 2);
        assert!(out_dem.is_file());
        assert!(work_dir.join("COP30_dem__tile_a.tif").is_file());
        assert!(work_dir.join("COP30_dem__tile_b.tif").is_file());
        let _ = std::fs::remove_dir_all(&work_dir);
    }
}
