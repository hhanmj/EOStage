use crate::agent::{agent_error, AgentHostState};
use crate::application::ApplicationServiceState;
use crate::error::{
    SarSearchDateRangeDiagnostic, SarSearchFailureDiagnostic, SarSearchFilterDiagnostic,
    SarSearchQueryShapeDiagnostic, SarSearchRangeDiagnostic,
};
use crate::ui_smoke::{UiSmokeReport, UiSmokeState};
use crate::{
    desktop_app_info, is_allowed_desktop_origin, is_allowed_desktop_window_label, AppInfo,
    ShellError,
};
use insar_application_service::{
    AccountSessionStatus, AdminBoundaryOptionsRequest, AdminBoundaryOptionsResult,
    AdminBoundaryResolveRequest, AdminBoundaryResult, AdminBoundarySearchRequest,
    AdminBoundarySearchResult, ApplicationAoiImportRequest, ApplicationAoiNormalizeRequest,
    ApplicationBindRegionAoiRequest, ApplicationCatalogIntentSearchAndFreezeRequest,
    ApplicationCatalogSearchAndFreezeRequest, ApplicationClearRegionAoiRequest,
    ApplicationNetworkSettings, ApplicationSarBackgroundCountRequest,
    ApplicationSarBackgroundCountResult, ApplicationSarSearchAndFreezeRequest,
    ApplicationSetWorkingAoiRequest, ApplicationWorkbenchPageRequest, ApplicationWorkingAoi,
    ArchiveDownloadTaskAck, BhoonidhiAssetSizeRequest, BhoonidhiAssetSizeResponse,
    BhoonidhiDesktopSessionStatus, BurstAppendAssetTaskRequest, BurstCreateAssetTaskRequest,
    BurstCreateHistoricalAssetTaskRequest, CdseLoginRequest, CdseNetworkDiagnosticResult,
    CdseNetworkMode, CdseQuicklookRequest, CdseQuicklookResponse, CdseSessionStatus,
    CoreServiceSnapshot, DemConversionRequest, DemTaskPlan, DemTaskPlanRequest, DemTaskRequest,
    DemTaskSettings, EffectiveCapabilityStatus, GacosConfirmRequest, GacosCreateImportTaskRequest,
    GacosCreateResultTaskRequest, GacosResultImportRequest, GacosSupplyResultLinkRequest,
    GeoBoundaryCountriesResult, GeoBoundaryLevelsRequest, GeoBoundaryLevelsResult,
    GeoBoundaryResolveRequest, GeoBoundaryUnitsRequest, GeoBoundaryUnitsResult,
    LandsatAssetDetailsRequest, LandsatAssetDetailsResponse, LandsatBrowseRequest,
    LandsatBrowseResponse, LandsatCreateAssetTaskRequest, LandsatCreateBundleTaskRequest,
    NisarAppendSubsetBatchTaskV2Request, NisarAppendSubsetContextV2,
    NisarAppendSubsetContextV2Request, NisarConfirmSubsetBatchTaskV2Request,
    NisarConfirmSubsetTaskV2Request, NisarCreateRemoteSubsetTaskRequest,
    NisarCreateSubsetSuccessorTaskV2Request, NisarPreflightSubsetAppendV2,
    NisarPreflightSubsetAppendV2Request, NisarPreflightSubsetBatchV2,
    NisarPreflightSubsetBatchV2Request, NisarPrepareSubsetBatchV2Request,
    NisarPrepareSubsetIntentV2Request, NisarPreparedSubsetBatchV2, NisarPreparedSubsetIntentV2,
    NisarRemoteSubsetCapability, OrbitDirectoryMatchRequest, OrbitTaskRequest, PreviewAssetRequest,
    PreviewSaveRequest, PreviewSaveResponse, ProviderConnectivityResult, ResolvedPreviewAsset,
    ReviewedMpcCreateAssetTaskRequest, SarDownloadPolicySettings, SarSearchProgressUpdate,
    SceneImportRequest, Sentinel2CreateArchiveBatchTaskRequest,
    Sentinel2CreateArchiveBatchTaskResult, Sentinel2CreateArchiveTaskRequest,
    Sentinel2CreateBandTaskRequest, Sentinel2MapPreviewRequest, Sentinel2MapPreviewResponse,
    StandaloneDemAoiTaskRequest, StandaloneDemRequest, StandaloneOrbitTaskRequest,
    StoreDemConversionSource, StoreDemConversionSourceRequest, StoreDemSourceIdentity,
    StoreTaskConflictFilesRequest, StoreTaskEventPage, StoreTaskEventPageRequest,
    StoreTaskHistoryPage, StoreTaskHistoryPageRequest, StoreTaskOutputDirectoryRequest,
    StoreTaskSourceBatchPage, StoreTaskSourceBatchPageRequest, TaskConflictFileDeleteResult,
    TaskConflictFileExpectation, TaskConflictFileList, UsgsM2mDesktopSessionStatus,
    UsgsM2mLoginRequest, WorkspaceCreateRequest, WorkspaceNamedNodeRequest, WorkspaceOpenRequest,
    WorkspacePrepareLocationRequest, WorkspacePreparedLocation, WorkspaceSelectNodeRequest,
};
use insar_core::{
    AppendTaskFromSourceBatchRequest, AppendTaskFromWorkbenchRequest,
    CreateTaskFromHistoricalSourceBatchRequest, CreateTaskFromWorkbenchRequest,
    ProviderCapabilitiesResponse, StoreAppendTaskRequest, StoreArchiveTaskRequest,
    StoreCreateTaskRequest, StoreDeleteHistoryRequest, StoreTaskPage, StoreTaskPageRequest,
    StoreUpdateTaskRequest, StoreUpdateTaskViewRequest, StoreUpdateWorkbenchRequest,
    WorkbenchSelectionCommand, WorkbenchSelectionSummary,
};
use insar_download::{
    clear_desktop_bhoonidhi, clear_desktop_cdse, clear_desktop_earthdata,
    clear_desktop_gacos_email, clear_desktop_opentopography_key, clear_desktop_usgs,
    desktop_bhoonidhi_status, desktop_cdse_status, desktop_earthdata_status, desktop_gacos_status,
    desktop_opentopography_status, desktop_usgs_status, resolve_desktop_cdse_login,
    resolve_desktop_usgs_login, resolve_earthdata_credential, store_desktop_bhoonidhi_login,
    store_desktop_cdse_login, store_desktop_earthdata_login, store_desktop_earthdata_token,
    store_desktop_gacos_email, store_desktop_opentopography_key, store_desktop_usgs_login,
    DesktopCredentialError, EarthdataAuthProbeOutcome, EarthdataCredentialError,
    EarthdataCredentialSource, HttpTransfer, TransferOptions,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
use tauri::{Emitter, Manager};

#[tauri::command]
pub fn ui_smoke_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, UiSmokeState>,
) -> Result<bool, ShellError> {
    if let Err(error) = validate_application_window(&window) {
        eprintln!("UI_SMOKE_STATUS_VALIDATE_ERROR:{}", error.code);
        return Err(error);
    }
    if !state.is_enabled() {
        eprintln!("UI_SMOKE_STATUS_DISABLED");
        return Err(ShellError::new(
            "UI_SMOKE_NOT_ENABLED",
            "ui_smoke.not_enabled",
        ));
    }
    eprintln!("UI_SMOKE_STATUS_ENABLED");
    Ok(true)
}

#[tauri::command]
pub fn ui_smoke_report(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, UiSmokeState>,
    report: UiSmokeReport,
) -> Result<(), ShellError> {
    validate_application_window(&window)?;
    let output_report = report.clone();
    state
        .accept(report)
        .map_err(|code| ShellError::with_code(code, "ui_smoke.report_rejected"))?;
    println!(
        "{}",
        serde_json::to_string(&serde_json::json!({ "ok": true, "ui_smoke": output_report }))
            .expect("UI smoke report serializes")
    );
    use std::io::Write;
    let _ = std::io::stdout().flush();
    window.app_handle().exit(0);
    Ok(())
}

#[tauri::command]
pub async fn ui_smoke_phase(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, UiSmokeState>,
) -> Result<String, ShellError> {
    #[cfg(target_os = "macos")]
    if crate::macos_cdp_bridge::is_active() && !state.is_enabled() {
        return tauri::async_runtime::spawn_blocking(crate::macos_cdp_bridge::pop_next_request)
            .await
            .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"));
    }
    if let Err(error) = validate_application_window(&window) {
        eprintln!("UI_SMOKE_PHASE_VALIDATE_ERROR:{}", error.code);
        return Err(error);
    }
    if !state.is_enabled() {
        eprintln!("UI_SMOKE_PHASE_DISABLED");
        return Err(ShellError::new(
            "UI_SMOKE_NOT_ENABLED",
            "ui_smoke.not_enabled",
        ));
    }
    let phase = state.phase().as_str().to_owned();
    eprintln!("UI_SMOKE_PHASE:{}", phase);
    Ok(phase)
}

#[tauri::command]
pub fn ui_smoke_checkpoint(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, UiSmokeState>,
    step: String,
) -> Result<(), ShellError> {
    #[cfg(target_os = "macos")]
    if let Some(payload) = step.strip_prefix("__eostage_cdp_reply__:") {
        crate::macos_cdp_bridge::deliver_reply(payload);
        return Ok(());
    }
    validate_application_window(&window)?;
    if !state.is_enabled() {
        return Err(ShellError::new(
            "UI_SMOKE_NOT_ENABLED",
            "ui_smoke.not_enabled",
        ));
    }
    state.checkpoint(step.clone());
    eprintln!("UI_SMOKE_CHECKPOINT:{step}");
    Ok(())
}

#[tauri::command]
pub fn ui_smoke_captured_search_query(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    ui_smoke: tauri::State<'_, UiSmokeState>,
) -> Result<Option<insar_core::CatalogSearchQuery>, ShellError> {
    validate_application_window(&window)?;
    if !ui_smoke.is_enabled() {
        return Err(ShellError::new(
            "UI_SMOKE_NOT_ENABLED",
            "ui_smoke.not_enabled",
        ));
    }
    state
        .ui_smoke_captured_query()
        .ok_or_else(|| ShellError::new("UI_SMOKE_NO_QUERY", "ui_smoke.no_query"))
        .map(Some)
}

#[tauri::command]
pub fn ui_smoke_phase_a_complete(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, UiSmokeState>,
) -> Result<(), ShellError> {
    validate_application_window(&window)?;
    if !state.is_enabled() || state.phase() != crate::ui_smoke::UiSmokePhase::PhaseA {
        return Err(ShellError::new(
            "UI_SMOKE_PHASE_MISMATCH",
            "ui_smoke.phase_mismatch",
        ));
    }
    state.complete_phase_a();
    eprintln!("UI_SMOKE_PHASE_A_COMPLETE_REQUESTED");
    println!("UI_SMOKE_PHASE_A_COMPLETE");
    use std::io::Write;
    let _ = std::io::stdout().flush();
    window.app_handle().exit(0);
    Ok(())
}

#[derive(Default)]
pub struct SavedWindowBounds(
    pub std::sync::Mutex<Option<(tauri::PhysicalPosition<i32>, tauri::PhysicalSize<u32>)>>,
);

#[tauri::command]
pub fn shell_app_info(window: tauri::WebviewWindow) -> Result<AppInfo, ShellError> {
    validate_application_window(&window)?;
    let package = window.app_handle().package_info();
    Ok(desktop_app_info(
        package.name.clone(),
        package.version.to_string(),
    ))
}

#[tauri::command]
pub fn shell_window_toggle_maximize(
    window: tauri::WebviewWindow,
    saved: tauri::State<'_, SavedWindowBounds>,
) -> Result<bool, ShellError> {
    validate_application_window(&window)?;
    #[cfg(windows)]
    {
        let _ = &saved;
        let currently_max = window.is_maximized().unwrap_or(false);
        if currently_max {
            window
                .unmaximize()
                .map_err(|_| ShellError::new("WINDOW_UNMAXIMIZE_FAILED", "window.unmaximize"))?;
            Ok(false)
        } else {
            window
                .maximize()
                .map_err(|_| ShellError::new("WINDOW_MAXIMIZE_FAILED", "window.maximize"))?;
            Ok(true)
        }
    }
    #[cfg(not(windows))]
    {
        let monitor = window
            .current_monitor()
            .ok()
            .flatten()
            .or_else(|| window.primary_monitor().ok().flatten());
        let scale = monitor
            .as_ref()
            .map(|m| m.scale_factor())
            .unwrap_or_else(|| window.scale_factor().unwrap_or(1.0))
            .max(1.0);
        let cur_size = window.outer_size().unwrap_or_else(|_| {
            tauri::PhysicalSize::new((1320.0 * scale) as u32, (880.0 * scale) as u32)
        });
        let cur_pos = window
            .outer_position()
            .unwrap_or_else(|_| tauri::PhysicalPosition::new(0, 0));
        let work_area = monitor.as_ref().map(|m| *m.work_area());

        let is_currently_zoomed = window.is_maximized().unwrap_or(false)
            || work_area.is_some_and(|wa| {
                cur_size.width as f64 >= wa.size.width as f64 * 0.93
                    && cur_size.height as f64 >= wa.size.height as f64 * 0.90
            });

        if is_currently_zoomed {
            let _ = window.unmaximize();
            let cached = saved.0.lock().ok().and_then(|mut guard| guard.take());
            let (restore_pos, restore_size) = if let Some((p, s)) = cached.filter(|(_, s)| {
                work_area.is_none_or(|wa| {
                    (s.width as f64) < wa.size.width as f64 * 0.92
                        || (s.height as f64) < wa.size.height as f64 * 0.89
                })
            }) {
                (p, s)
            } else if let Some(wa) = work_area {
                let min_w = (920.0 * scale).round() as u32;
                let min_h = (620.0 * scale).round() as u32;
                let target_w = ((1320.0 * scale).round() as u32)
                    .min(((wa.size.width as f64) * 0.82).round() as u32)
                    .max(min_w.min(wa.size.width));
                let target_h = ((880.0 * scale).round() as u32)
                    .min(((wa.size.height as f64) * 0.82).round() as u32)
                    .max(min_h.min(wa.size.height));
                let x = wa.position.x + ((wa.size.width.saturating_sub(target_w)) / 2) as i32;
                let y = wa.position.y + ((wa.size.height.saturating_sub(target_h)) / 2) as i32;
                (
                    tauri::PhysicalPosition::new(x, y),
                    tauri::PhysicalSize::new(target_w, target_h),
                )
            } else {
                (
                    tauri::PhysicalPosition::new(80, 60),
                    tauri::PhysicalSize::new((1200.0 * scale) as u32, (800.0 * scale) as u32),
                )
            };
            let _ = window.set_size(tauri::Size::Physical(restore_size));
            let _ = window.set_position(tauri::Position::Physical(restore_pos));
            Ok(false)
        } else {
            if let Ok(mut guard) = saved.0.lock() {
                *guard = Some((cur_pos, cur_size));
            }
            if let Some(wa) = work_area {
                let _ = window.set_position(tauri::Position::Physical(wa.position));
                let _ = window.set_size(tauri::Size::Physical(wa.size));
            } else {
                let _ = window.maximize();
            }
            Ok(true)
        }
    }
}

#[tauri::command]
pub fn application_service_readiness(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
) -> Result<insar_application_service::ServiceReadiness, ShellError> {
    validate_application_window(&window)?;
    state.readiness().map_err(application_error)
}

#[tauri::command]
pub async fn application_core_restore(
    window: tauri::WebviewWindow,
    include_terminal: Option<bool>,
) -> Result<CoreServiceSnapshot, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    // Store reads can wait behind a task/selection writer. Keep the wait and
    // projection off the window thread, not just the frontend Promise.
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .core_restore(include_terminal.unwrap_or(false))
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_workbench_page(
    window: tauri::WebviewWindow,
    request: ApplicationWorkbenchPageRequest,
) -> Result<insar_application_service::ApplicationWorkbenchPage, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .workbench_page(&request)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_batch_page(
    window: tauri::WebviewWindow,
    batch_id: String,
    revision: u64,
    page: u64,
    page_size: u64,
) -> Result<insar_application_service::StoreBatchPage, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .batch_page(&insar_application_service::StoreBatchPageRequest {
                batch_id,
                revision,
                page,
                page_size,
            })
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_download_task_page(
    window: tauri::WebviewWindow,
    request: StoreTaskPageRequest,
) -> Result<StoreTaskPage, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .task_page(&request)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_load_task_source_batch_page(
    window: tauri::WebviewWindow,
    request: StoreTaskSourceBatchPageRequest,
) -> Result<StoreTaskSourceBatchPage, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .task_source_batch_page(&request)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_task_overview(
    window: tauri::WebviewWindow,
) -> Result<insar_application_service::TaskOverview, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .task_overview()
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_download_history_page(
    window: tauri::WebviewWindow,
    request: StoreTaskHistoryPageRequest,
) -> Result<StoreTaskHistoryPage, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .task_history_page(&request)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_download_task_event_page(
    window: tauri::WebviewWindow,
    request: StoreTaskEventPageRequest,
) -> Result<StoreTaskEventPage, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .task_event_page(&request)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct OpenTaskOutputDirectoryAck {
    pub task_id: String,
    pub revision: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct OpenExternalUrlAck {
    pub url: String,
}

/// B15: the confirmed conflict files of one task, each by its exact
/// (relative path, size, mtime) from the listing the user saw.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct DeleteTaskConflictFilesRequest {
    pub task_id: String,
    pub expected: Vec<TaskConflictFileExpectation>,
}

fn task_conflict_expectations(
    expected: &[TaskConflictFileExpectation],
) -> Result<Vec<(String, u64, u64)>, ShellError> {
    if expected.len() > 10_000 {
        return Err(ShellError::application_service(
            "V3_APPLICATION_CONFLICT_CLEANUP_LIMIT",
        ));
    }
    let mut pairs = Vec::with_capacity(expected.len());
    for item in expected {
        item.validate().map_err(|_| {
            ShellError::application_service("V3_APPLICATION_CONFLICT_CLEANUP_INVALID")
        })?;
        pairs.push((
            item.path.clone(),
            item.size_bytes,
            item.modified_at_epoch_ms,
        ));
    }
    Ok(pairs)
}

#[tauri::command]
pub async fn application_list_task_conflict_files(
    window: tauri::WebviewWindow,
    request: StoreTaskConflictFilesRequest,
) -> Result<TaskConflictFileList, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .list_task_conflict_files(request)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_delete_task_conflict_files(
    window: tauri::WebviewWindow,
    request: DeleteTaskConflictFilesRequest,
) -> Result<TaskConflictFileDeleteResult, ShellError> {
    validate_application_window(&window)?;
    let expected = task_conflict_expectations(&request.expected)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .delete_task_conflict_files(&request.task_id, &expected)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub fn application_open_external_url(
    window: tauri::WebviewWindow,
    url: String,
) -> Result<OpenExternalUrlAck, ShellError> {
    validate_application_window(&window)?;
    let normalized = url.trim();
    if normalized.len() > 4_096
        || normalized.chars().any(char::is_control)
        || !(normalized.starts_with("https://") || normalized.starts_with("http://"))
    {
        return Err(ShellError::new("GUI003", "shell.external_url_not_allowed"));
    }

    if !crate::oauth_login::has_recent_or_active_native_pkce_session()
        && crate::external_url::is_local_oauth_authorize_url(normalized)
    {
        crate::external_url::spawn_oauth_loopback_listener(window.clone());
    }

    crate::external_url::open_in_default_browser(normalized)
        .map_err(|_| ShellError::new("GUI003", "shell.external_url_open_failed"))?;
    Ok(OpenExternalUrlAck {
        url: normalized.to_owned(),
    })
}

#[tauri::command]
pub fn application_open_download_task_output(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    request: StoreTaskOutputDirectoryRequest,
) -> Result<OpenTaskOutputDirectoryAck, ShellError> {
    validate_application_window(&window)?;
    let resolved = state
        .open_task_output_directory(&request)
        .map_err(application_error)?;
    Ok(OpenTaskOutputDirectoryAck {
        task_id: resolved.task_id,
        revision: resolved.revision,
    })
}

#[tauri::command]
pub fn application_aoi_import(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    request: ApplicationAoiImportRequest,
) -> Result<Value, ShellError> {
    validate_application_window(&window)?;
    state.aoi_import(&request).map_err(application_error)
}

#[tauri::command]
pub fn application_aoi_normalize(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    request: ApplicationAoiNormalizeRequest,
) -> Result<Value, ShellError> {
    validate_application_window(&window)?;
    state.aoi_normalize(&request).map_err(application_error)
}

#[tauri::command]
pub async fn application_admin_boundary_search(
    window: tauri::WebviewWindow,
    request: AdminBoundarySearchRequest,
) -> Result<AdminBoundarySearchResult, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .admin_boundary_search(&request)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_admin_boundary_resolve(
    window: tauri::WebviewWindow,
    request: AdminBoundaryResolveRequest,
) -> Result<AdminBoundaryResult, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .admin_boundary_resolve(&request)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_admin_options(
    window: tauri::WebviewWindow,
    request: AdminBoundaryOptionsRequest,
) -> Result<AdminBoundaryOptionsResult, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .admin_options(&request)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_geo_boundary_countries(
    window: tauri::WebviewWindow,
) -> Result<GeoBoundaryCountriesResult, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .geo_boundary_countries()
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_geo_boundary_levels(
    window: tauri::WebviewWindow,
    request: GeoBoundaryLevelsRequest,
) -> Result<GeoBoundaryLevelsResult, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .geo_boundary_levels(&request)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_geo_boundary_units(
    window: tauri::WebviewWindow,
    request: GeoBoundaryUnitsRequest,
) -> Result<GeoBoundaryUnitsResult, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .geo_boundary_units(&request)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_geo_boundary_resolve(
    window: tauri::WebviewWindow,
    request: GeoBoundaryResolveRequest,
) -> Result<AdminBoundaryResult, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .geo_boundary_resolve(&request)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub fn application_gacos_plan(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    payload: Value,
) -> Result<Value, ShellError> {
    validate_application_window(&window)?;
    state.gacos_plan(&payload).map_err(application_error)
}

#[tauri::command]
pub fn application_gacos_dry_run(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    payload: Value,
) -> Result<Value, ShellError> {
    validate_application_window(&window)?;
    state.gacos_dry_run(&payload).map_err(application_error)
}

#[tauri::command]
pub async fn application_gacos_submit_request(
    window: tauri::WebviewWindow,
    request: GacosConfirmRequest,
) -> Result<Value, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .gacos_submit_request(&request)
            .map_err(application_error)
    })
    .await
    .map_err(|_| {
        ShellError::with_code("GACOS_TASK_FAILED", "application_service.gacos_task_failed")
    })?
}

#[tauri::command]
pub async fn application_gacos_request_status(
    window: tauri::WebviewWindow,
    task_id: String,
) -> Result<Value, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .gacos_request_status(&task_id)
            .map_err(application_error)
    })
    .await
    .map_err(|_| {
        ShellError::with_code("GACOS_TASK_FAILED", "application_service.gacos_task_failed")
    })?
}

#[tauri::command]
pub fn application_gacos_ledger(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
) -> Result<Value, ShellError> {
    validate_application_window(&window)?;
    state.gacos_ledger().map_err(application_error)
}

#[tauri::command]
pub fn application_gacos_import_result(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    request: GacosResultImportRequest,
) -> Result<Value, ShellError> {
    validate_application_window(&window)?;
    state
        .gacos_import_result(&request)
        .map_err(application_error)
}

#[tauri::command]
pub async fn application_gacos_create_import_task(
    window: tauri::WebviewWindow,
    request: GacosCreateImportTaskRequest,
) -> Result<Value, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .gacos_create_import_task(&request)
            .map_err(application_error)
    })
    .await
    .map_err(|_| {
        ShellError::with_code("GACOS_TASK_FAILED", "application_service.gacos_task_failed")
    })?
}

#[tauri::command]
pub async fn application_gacos_create_result_task(
    window: tauri::WebviewWindow,
    request: GacosCreateResultTaskRequest,
) -> Result<Value, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .gacos_create_result_task(&request)
            .map_err(application_error)
    })
    .await
    .map_err(|_| {
        ShellError::with_code("GACOS_TASK_FAILED", "application_service.gacos_task_failed")
    })?
}

#[tauri::command]
pub async fn application_gacos_supply_result_link(
    window: tauri::WebviewWindow,
    request: GacosSupplyResultLinkRequest,
) -> Result<Value, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .gacos_supply_result_link(&request)
            .map_err(application_error)
    })
    .await
    .map_err(|_| {
        ShellError::with_code("GACOS_TASK_FAILED", "application_service.gacos_task_failed")
    })?
}

#[tauri::command]
pub fn application_report_generate(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    payload: Value,
) -> Result<Value, ShellError> {
    validate_application_window(&window)?;
    state.report_generate(&payload).map_err(application_error)
}

#[tauri::command]
pub async fn application_update_workbench_selection(
    window: tauri::WebviewWindow,
    command: WorkbenchSelectionCommand,
) -> Result<WorkbenchSelectionSummary, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .update_workbench_selection(&command)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub fn application_workspace_open(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    request: WorkspaceOpenRequest,
) -> Result<insar_core::WorkspaceContextState, ShellError> {
    validate_application_window(&window)?;
    state.workspace_open(&request).map_err(application_error)
}

#[tauri::command]
pub fn application_workspace_create(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    request: WorkspaceCreateRequest,
) -> Result<insar_core::WorkspaceContextState, ShellError> {
    validate_application_window(&window)?;
    state.workspace_create(&request).map_err(application_error)
}

#[tauri::command]
pub fn application_workspace_prepare_location(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    request: WorkspacePrepareLocationRequest,
) -> Result<WorkspacePreparedLocation, ShellError> {
    validate_application_window(&window)?;
    state
        .workspace_prepare_location(&request)
        .map_err(application_error)
}

#[tauri::command]
pub fn application_project_create(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    request: WorkspaceNamedNodeRequest,
) -> Result<insar_core::WorkspaceContextState, ShellError> {
    validate_application_window(&window)?;
    state.project_create(&request).map_err(application_error)
}

#[tauri::command]
pub fn application_project_select(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    request: WorkspaceSelectNodeRequest,
) -> Result<insar_core::WorkspaceContextState, ShellError> {
    validate_application_window(&window)?;
    state.project_select(&request).map_err(application_error)
}

#[tauri::command]
pub fn application_region_create(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    request: WorkspaceNamedNodeRequest,
) -> Result<insar_core::WorkspaceContextState, ShellError> {
    validate_application_window(&window)?;
    state.region_create(&request).map_err(application_error)
}

#[tauri::command]
pub fn application_region_select(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    request: WorkspaceSelectNodeRequest,
) -> Result<insar_core::WorkspaceContextState, ShellError> {
    validate_application_window(&window)?;
    state.region_select(&request).map_err(application_error)
}

#[tauri::command]
pub fn application_bind_region_aoi(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    request: ApplicationBindRegionAoiRequest,
) -> Result<CoreServiceSnapshot, ShellError> {
    validate_application_window(&window)?;
    state.bind_region_aoi(&request).map_err(application_error)
}

#[tauri::command]
pub fn application_clear_region_aoi(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    request: ApplicationClearRegionAoiRequest,
) -> Result<CoreServiceSnapshot, ShellError> {
    validate_application_window(&window)?;
    state.clear_region_aoi(&request).map_err(application_error)
}

#[tauri::command]
pub fn application_set_working_aoi(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    request: ApplicationSetWorkingAoiRequest,
) -> Result<ApplicationWorkingAoi, ShellError> {
    validate_application_window(&window)?;
    let aoi = state.set_working_aoi(&request).map_err(application_error)?;
    crate::emit_host_event(
        window.app_handle(),
        "workbench:aoi-changed",
        serde_json::json!({
            "state": "present",
            "display_name": aoi.display_name,
            "bbox": aoi.bbox,
            "updated_by": "manual",
        }),
    );
    Ok(aoi)
}

#[tauri::command]
pub fn application_clear_working_aoi(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
) -> Result<(), ShellError> {
    validate_application_window(&window)?;
    state.clear_working_aoi().map_err(application_error)?;
    crate::emit_host_event(
        window.app_handle(),
        "workbench:aoi-changed",
        serde_json::json!({
            "state": "empty",
            "display_name": serde_json::Value::Null,
            "bbox": serde_json::Value::Null,
            "updated_by": "manual",
        }),
    );
    Ok(())
}

#[tauri::command]
pub fn application_working_aoi(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
) -> Result<Option<ApplicationWorkingAoi>, ShellError> {
    validate_application_window(&window)?;
    state.working_aoi().map_err(application_error)
}

#[tauri::command]
pub fn application_plan_dem_task(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    request: DemTaskPlanRequest,
) -> Result<DemTaskPlan, ShellError> {
    validate_application_window(&window)?;
    state.plan_dem_task(&request).map_err(application_error)
}

#[tauri::command]
pub fn application_create_dem_task(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    request: DemTaskRequest,
) -> Result<CoreServiceSnapshot, ShellError> {
    validate_application_window(&window)?;
    state.create_dem_task(&request).map_err(application_error)
}

#[tauri::command]
pub fn application_handoff_dem_conversion(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    request: DemConversionRequest,
) -> Result<CoreServiceSnapshot, ShellError> {
    validate_application_window(&window)?;
    state
        .handoff_dem_conversion(&request)
        .map_err(application_error)
}

#[tauri::command]
pub fn application_resolve_dem_conversion_source(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    request: StoreDemConversionSourceRequest,
) -> Result<Option<StoreDemConversionSource>, ShellError> {
    validate_application_window(&window)?;
    state
        .resolve_dem_conversion_source(&request)
        .map_err(application_error)
}

#[tauri::command]
pub fn application_set_dem_source_identity(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    identity: StoreDemSourceIdentity,
) -> Result<StoreDemSourceIdentity, ShellError> {
    validate_application_window(&window)?;
    state
        .set_dem_source_identity(&identity)
        .map_err(application_error)
}

#[tauri::command]
pub async fn application_get_dem_task_settings(
    window: tauri::WebviewWindow,
) -> Result<DemTaskSettings, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .get_dem_task_settings()
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_save_dem_task_settings(
    window: tauri::WebviewWindow,
    settings: DemTaskSettings,
) -> Result<DemTaskSettings, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .save_dem_task_settings(&settings)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub fn application_get_sar_download_policy_settings(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
) -> Result<SarDownloadPolicySettings, ShellError> {
    validate_application_window(&window)?;
    state
        .get_sar_download_policy_settings()
        .map_err(application_error)
}

#[tauri::command]
pub fn application_save_sar_download_policy_settings(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    settings: SarDownloadPolicySettings,
) -> Result<SarDownloadPolicySettings, ShellError> {
    validate_application_window(&window)?;
    state
        .save_sar_download_policy_settings(&settings)
        .map_err(application_error)
}

#[tauri::command]
pub fn application_get_network_settings(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
) -> Result<ApplicationNetworkSettings, ShellError> {
    validate_application_window(&window)?;
    state.get_network_settings().map_err(application_error)
}

#[tauri::command]
pub fn application_save_network_settings(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    settings: ApplicationNetworkSettings,
) -> Result<ApplicationNetworkSettings, ShellError> {
    validate_application_window(&window)?;
    state
        .save_network_settings(&settings)
        .map_err(application_error)
}

#[tauri::command]
pub async fn application_test_network_connection(
    window: tauri::WebviewWindow,
) -> Result<String, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .test_network_connection()
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_probe_provider_connectivity(
    window: tauri::WebviewWindow,
) -> Result<Vec<ProviderConnectivityResult>, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .probe_provider_connectivity()
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_standalone_dem(
    window: tauri::WebviewWindow,
    request: StandaloneDemRequest,
) -> Result<Value, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ApplicationServiceState>();
        let Some(_run) = state.begin_standalone_dem() else {
            return Err(ShellError::application_service("STANDALONE_DEM_BUSY"));
        };
        state.standalone_dem(&request).map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub fn application_create_standalone_dem_task(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    request: StandaloneDemAoiTaskRequest,
) -> Result<CoreServiceSnapshot, ShellError> {
    validate_application_window(&window)?;
    state
        .create_standalone_dem_task(&request)
        .map_err(application_error)
}

#[tauri::command]
pub fn application_import_scenes(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    request: SceneImportRequest,
) -> Result<Value, ShellError> {
    validate_application_window(&window)?;
    state.import_scenes(&request).map_err(application_error)
}

#[tauri::command]
pub fn application_match_orbits_directory(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    request: OrbitDirectoryMatchRequest,
) -> Result<Value, ShellError> {
    validate_application_window(&window)?;
    state
        .match_orbits_directory(&request)
        .map_err(application_error)
}

#[tauri::command]
pub fn application_create_companion_orbit_task(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    source_task_id: String,
) -> Result<CoreServiceSnapshot, ShellError> {
    validate_application_window(&window)?;
    state
        .create_companion_orbit_task(&source_task_id)
        .map_err(application_error)
}

#[tauri::command]
pub fn application_create_orbit_task(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    request: OrbitTaskRequest,
) -> Result<CoreServiceSnapshot, ShellError> {
    validate_application_window(&window)?;
    state.create_orbit_task(&request).map_err(application_error)
}

#[tauri::command]
pub fn application_create_standalone_orbit_task(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    request: StandaloneOrbitTaskRequest,
) -> Result<CoreServiceSnapshot, ShellError> {
    validate_application_window(&window)?;
    state
        .create_standalone_orbit_task(&request)
        .map_err(application_error)
}

#[tauri::command]
pub async fn application_list_sources(
    window: tauri::WebviewWindow,
) -> Result<Vec<insar_core::SourceDescriptor>, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .list_sources()
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_source_views(
    window: tauri::WebviewWindow,
) -> Result<Vec<insar_core::SourceView>, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .source_views()
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub fn application_provider_capabilities(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
) -> Result<ProviderCapabilitiesResponse, ShellError> {
    validate_application_window(&window)?;
    state.provider_capabilities().map_err(application_error)
}

/// Read-only typed capability projection. It does not search, authenticate,
/// download, mutate Store state, or alter product policy.
#[tauri::command]
pub fn application_effective_capability(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    capability_id: String,
) -> Result<Option<EffectiveCapabilityStatus>, ShellError> {
    validate_application_window(&window)?;
    state
        .effective_capability(&capability_id)
        .map_err(application_error)
}

#[tauri::command]
pub async fn application_search_start(
    window: tauri::WebviewWindow,
    request: insar_application_service::UnifiedSearchRequest,
    bypass_cache: Option<bool>,
) -> Result<insar_core::SearchProgress, ShellError> {
    // 检索启动需要获取 Store/运行时锁；批量建任务或调度器持锁时若在主线程
    // 同步等待会冻结整个 WebView（地图台卡死）。改为后台阻塞线程执行。
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    let bypass_cache = bypass_cache.unwrap_or(false);
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .search_start_with_options(request, bypass_cache)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_search_progress(
    window: tauri::WebviewWindow,
    request_id: String,
) -> Result<Option<insar_core::SearchProgress>, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .search_progress(&request_id)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))
}

#[tauri::command]
pub async fn application_search_cancel(
    window: tauri::WebviewWindow,
    request_id: String,
) -> Result<bool, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .search_cancel(&request_id)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_sar_search_and_freeze(
    window: tauri::WebviewWindow,
    request: ApplicationSarSearchAndFreezeRequest,
) -> Result<CoreServiceSnapshot, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .sar_search_and_freeze(&request)
            .map_err(|error| application_sar_error(&request, error))
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub fn application_sar_search_progress(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    request_id: String,
) -> Result<Option<SarSearchProgressUpdate>, ShellError> {
    validate_application_window(&window)?;
    Ok(state.sar_search_progress(&request_id))
}

#[tauri::command]
pub async fn application_sar_background_count(
    window: tauri::WebviewWindow,
    request: ApplicationSarBackgroundCountRequest,
) -> Result<ApplicationSarBackgroundCountResult, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .sar_background_count_and_update(&request)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_catalog_search_and_freeze(
    window: tauri::WebviewWindow,
    request: ApplicationCatalogSearchAndFreezeRequest,
) -> Result<CoreServiceSnapshot, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .catalog_search_and_freeze(&request)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

/// Issue #3 R2 B4: route-neutral catalogue intent. Planning and the
/// existing catalogue path run off the Tauri main thread like the seed form.
#[tauri::command]
pub async fn application_catalog_intent_search_and_freeze(
    window: tauri::WebviewWindow,
    request: ApplicationCatalogIntentSearchAndFreezeRequest,
) -> Result<CoreServiceSnapshot, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .catalog_intent_search_and_freeze(&request)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

/// #22 B slice 3(c): cancel a catalogue search. A search that already
/// showed its first page keeps the arrived rows (state cancelled, possibly
/// incomplete); sealing them runs off the Tauri main thread.
#[tauri::command]
pub async fn application_cancel_catalog_search(
    window: tauri::WebviewWindow,
    request_id: String,
) -> Result<insar_core::SearchCancelOutcome, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .cancel_catalog_search(&request_id)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub fn application_cancel_request(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    request_id: String,
) -> Result<bool, ShellError> {
    validate_application_window(&window)?;
    state.cancel_request(&request_id).map_err(application_error)
}

#[tauri::command]
pub async fn application_account_session_statuses(
    window: tauri::WebviewWindow,
) -> Result<Vec<AccountSessionStatus>, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .account_session_statuses()
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub fn application_cdse_session_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
) -> Result<CdseSessionStatus, ShellError> {
    validate_application_window(&window)?;
    state.cdse_session_status().map_err(application_error)
}

#[tauri::command]
pub fn application_cdse_login(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    request: CdseLoginRequest,
) -> Result<CdseSessionStatus, ShellError> {
    validate_application_window(&window)?;
    let CdseLoginRequest {
        username,
        password,
        totp,
    } = request;
    let request = if username.trim().is_empty() && password.is_empty() {
        let (username, password) = resolve_desktop_cdse_login().map_err(credential_error)?;
        CdseLoginRequest {
            username: username.to_string(),
            password: password.to_string(),
            totp,
        }
    } else {
        CdseLoginRequest {
            username,
            password,
            totp,
        }
    };
    state.cdse_login(request).map_err(application_error)
}

#[tauri::command]
pub fn application_cdse_logout(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
) -> Result<CdseSessionStatus, ShellError> {
    validate_application_window(&window)?;
    state.cdse_logout().map_err(application_error)
}

#[tauri::command]
pub fn application_cdse_network_policy(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
) -> Result<CdseNetworkMode, ShellError> {
    validate_application_window(&window)?;
    state.cdse_network_policy().map_err(application_error)
}

#[tauri::command]
pub fn application_set_cdse_network_policy(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    mode: CdseNetworkMode,
) -> Result<CdseSessionStatus, ShellError> {
    validate_application_window(&window)?;
    state
        .set_cdse_network_policy(mode)
        .map_err(application_error)
}

#[tauri::command]
pub fn application_cdse_network_diagnostic(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    mode: CdseNetworkMode,
) -> Result<CdseNetworkDiagnosticResult, ShellError> {
    validate_application_window(&window)?;
    state
        .cdse_network_diagnostic(mode)
        .map_err(application_error)
}

#[tauri::command]
pub async fn application_preview_asset(
    window: tauri::WebviewWindow,
    request: PreviewAssetRequest,
) -> Result<ResolvedPreviewAsset, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .preview_asset(request)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub fn application_save_preview_asset(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    request: PreviewSaveRequest,
) -> Result<PreviewSaveResponse, ShellError> {
    validate_application_window(&window)?;
    state.save_preview_asset(request).map_err(application_error)
}

#[tauri::command]
pub async fn application_cdse_quicklook(
    window: tauri::WebviewWindow,
    request: CdseQuicklookRequest,
) -> Result<CdseQuicklookResponse, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .cdse_quicklook(request)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_sentinel2_map_preview(
    window: tauri::WebviewWindow,
    request: Sentinel2MapPreviewRequest,
) -> Result<Sentinel2MapPreviewResponse, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .sentinel2_map_preview(request)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub fn application_usgs_session_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
) -> Result<UsgsM2mDesktopSessionStatus, ShellError> {
    validate_application_window(&window)?;
    state.usgs_m2m_session_status().map_err(application_error)
}

#[tauri::command]
pub fn application_usgs_login(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    request: UsgsM2mLoginRequest,
) -> Result<UsgsM2mDesktopSessionStatus, ShellError> {
    validate_application_window(&window)?;
    let UsgsM2mLoginRequest {
        username,
        application_token,
    } = request;
    let request = if username.trim().is_empty() && application_token.is_empty() {
        let (username, application_token) =
            resolve_desktop_usgs_login().map_err(credential_error)?;
        UsgsM2mLoginRequest {
            username: username.to_string(),
            application_token: application_token.to_string(),
        }
    } else {
        UsgsM2mLoginRequest {
            username,
            application_token,
        }
    };
    state.usgs_m2m_login(request).map_err(application_error)
}

#[tauri::command]
pub fn application_usgs_logout(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
) -> Result<UsgsM2mDesktopSessionStatus, ShellError> {
    validate_application_window(&window)?;
    state.usgs_m2m_logout().map_err(application_error)
}

#[tauri::command]
pub async fn application_landsat_browse(
    window: tauri::WebviewWindow,
    request: LandsatBrowseRequest,
) -> Result<LandsatBrowseResponse, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .landsat_browse(request)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_landsat_asset_details(
    window: tauri::WebviewWindow,
    request: LandsatAssetDetailsRequest,
) -> Result<LandsatAssetDetailsResponse, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .landsat_asset_details(request)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_bhoonidhi_asset_size(
    window: tauri::WebviewWindow,
    request: BhoonidhiAssetSizeRequest,
) -> Result<BhoonidhiAssetSizeResponse, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .bhoonidhi_asset_size(request)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_create_landsat_bundle_task(
    window: tauri::WebviewWindow,
    request: LandsatCreateBundleTaskRequest,
) -> Result<CoreServiceSnapshot, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .create_landsat_bundle_task(&request)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[derive(Clone, Debug, Serialize)]
pub struct DesktopEarthdataInput {
    pub mode: &'static str,
    pub token: &'static str,
    pub username: &'static str,
    pub password: &'static str,
}

#[derive(Clone, Debug, Serialize)]
pub struct DesktopCredentialStatus {
    pub ok: bool,
    pub cdse: String,
    pub usgs: String,
    pub earthdata: String,
    pub earthdata_input: DesktopEarthdataInput,
    pub bhoonidhi: String,
    pub opentopography: String,
    pub gacos: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct DesktopCredentialMutation {
    pub ok: bool,
    pub status: DesktopCredentialStatus,
    pub removed: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct DesktopEarthdataAuth {
    pub ok: bool,
    pub configured: bool,
    pub status: &'static str,
    pub message: &'static str,
}

fn credential_status_payload() -> DesktopCredentialStatus {
    let earthdata = desktop_earthdata_status();
    let mode = if earthdata == "token" || earthdata == "env-token" {
        "token"
    } else if earthdata.starts_with("login:") {
        "login"
    } else if earthdata == "netrc" {
        "netrc"
    } else {
        "none"
    };
    DesktopCredentialStatus {
        ok: true,
        cdse: desktop_cdse_status(),
        usgs: desktop_usgs_status(),
        earthdata,
        earthdata_input: DesktopEarthdataInput {
            mode,
            token: "",
            username: "",
            password: "",
        },
        bhoonidhi: desktop_bhoonidhi_status(),
        opentopography: desktop_opentopography_status(),
        gacos: desktop_gacos_status(),
    }
}

fn credential_error(error: DesktopCredentialError) -> ShellError {
    ShellError::new(error.code(), error.reason_zh())
}

fn credential_mutation(removed: bool) -> DesktopCredentialMutation {
    DesktopCredentialMutation {
        ok: true,
        status: credential_status_payload(),
        removed,
    }
}

#[tauri::command]
pub async fn credential_status(
    window: tauri::WebviewWindow,
) -> Result<DesktopCredentialStatus, ShellError> {
    validate_application_window(&window)?;
    tauri::async_runtime::spawn_blocking(credential_status_payload)
        .await
        .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))
}

#[tauri::command]
pub async fn credential_bhoonidhi_auth(window: tauri::WebviewWindow) -> Result<Value, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .bhoonidhi_authentication_check()
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub fn credential_bhoonidhi_session_status(
    window: tauri::WebviewWindow,
) -> Result<BhoonidhiDesktopSessionStatus, ShellError> {
    validate_application_window(&window)?;
    window
        .app_handle()
        .state::<ApplicationServiceState>()
        .bhoonidhi_session_status()
        .map_err(application_error)
}

#[cfg(feature = "bhoonidhi-auth-revoke-acceptance")]
#[tauri::command]
pub fn acceptance_bhoonidhi_auth_generation(
    window: tauri::WebviewWindow,
    task_id: String,
) -> Result<u64, ShellError> {
    validate_application_window(&window)?;
    window
        .app_handle()
        .state::<ApplicationServiceState>()
        .bhoonidhi_auth_revoke_acceptance_generation(&task_id)
        .map_err(application_error)
}

#[cfg(feature = "bhoonidhi-auth-revoke-acceptance")]
#[tauri::command]
pub async fn acceptance_bhoonidhi_auth_revoke_once(
    window: tauri::WebviewWindow,
    task_id: String,
    generation: u64,
) -> Result<(), ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .bhoonidhi_auth_revoke_acceptance_once(&task_id, generation)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

// Intentionally not Debug: this one-shot IPC response contains a selected secret.
#[derive(Serialize)]
pub struct DesktopCredentialReveal {
    pub ok: bool,
    pub value: Option<String>,
}

#[tauri::command]
pub fn credential_reveal(
    window: tauri::WebviewWindow,
    field: insar_download::DesktopCredentialField,
) -> Result<DesktopCredentialReveal, ShellError> {
    validate_application_window(&window)?;
    let value = insar_download::reveal_desktop_credential_field(field).map_err(credential_error)?;
    Ok(DesktopCredentialReveal {
        ok: true,
        value: value.map(|value| value.to_string()),
    })
}

#[tauri::command]
pub async fn credential_read_for_edit(
    window: tauri::WebviewWindow,
    field: insar_download::DesktopCredentialField,
) -> Result<DesktopCredentialReveal, ShellError> {
    validate_application_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let value =
            insar_download::read_desktop_credential_for_edit(field).map_err(credential_error)?;
        Ok(DesktopCredentialReveal {
            ok: true,
            value: value.map(|value| value.to_string()),
        })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

/// Explicit desktop-window credential read for an existing saved model channel key
/// (Owner requirement 2026-11-12 #3: saved channel keys stay visible/masked in Settings
/// instead of disappearing into an empty box after save; status polling via `agent_channels_get`
/// continues to project only `key_ready: bool`).
#[tauri::command]
pub async fn credential_agent_channel_key_read(
    window: tauri::WebviewWindow,
    channel_id: String,
) -> Result<DesktopCredentialReveal, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let trimmed = channel_id.trim();
        if trimmed.is_empty() {
            return Ok(DesktopCredentialReveal {
                ok: true,
                value: None,
            });
        }
        if insar_agent_model::channels::is_reserved_channel_id(trimmed)
            || crate::eostage_account::is_official_channel_id(trimmed)
        {
            return Err(agent_error(insar_agent::session::HostError::Invalid));
        }
        let is_saved_channel = app
            .state::<AgentHostState>()
            .has_saved_channel(trimmed)
            .map_err(agent_error)?;
        if !is_saved_channel {
            return Err(agent_error(insar_agent::session::HostError::NotFound));
        }
        let value = insar_download::with_desktop_agent_model_key(trimmed, |key| key.to_owned())
            .map_err(credential_error)?;
        if crate::oauth_login::classify_channel_secret_auth_meta(value.as_deref()).auth_kind
            == "subscription"
        {
            return Ok(DesktopCredentialReveal {
                ok: true,
                value: None,
            });
        }
        Ok(DesktopCredentialReveal { ok: true, value })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[derive(Serialize)]
pub struct DesktopCredentialEditAcknowledgement {
    pub ok: bool,
}

#[tauri::command]
pub async fn credential_update_field(
    window: tauri::WebviewWindow,
    field: insar_download::DesktopCredentialField,
    value: String,
) -> Result<DesktopCredentialEditAcknowledgement, ShellError> {
    validate_application_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        insar_download::update_desktop_credential_field(field, value).map_err(credential_error)?;
        Ok(DesktopCredentialEditAcknowledgement { ok: true })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn credential_earthdata_auth(
    window: tauri::WebviewWindow,
    force: Option<bool>,
) -> Result<DesktopEarthdataAuth, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    let proxy_policy = app
        .state::<ApplicationServiceState>()
        .ordinary_proxy_policy();
    tauri::async_runtime::spawn_blocking(move || {
        credential_earthdata_auth_blocking(proxy_policy, force)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

fn credential_earthdata_auth_blocking(
    proxy_policy: insar_core::ProxyPolicy,
    force: Option<bool>,
) -> Result<DesktopEarthdataAuth, ShellError> {
    let configured_status = desktop_earthdata_status();
    let configured = !matches!(configured_status.as_str(), "none" | "unavailable");
    if !configured {
        return Ok(DesktopEarthdataAuth {
            ok: true,
            configured: false,
            status: "missing",
            message: "未读取到 Earthdata/ASF 凭证，请在设置中保存 Token 或账号密码。",
        });
    }
    if force != Some(true) {
        return Ok(DesktopEarthdataAuth {
            ok: true,
            configured: true,
            status: "unknown",
            message: "已读取系统凭证存储；仅在手动点击检测登录时联网验证。",
        });
    }

    let auth = match resolve_earthdata_credential(EarthdataCredentialSource::Auto, None) {
        Ok(auth) => auth,
        Err(EarthdataCredentialError::Missing) => {
            return Ok(DesktopEarthdataAuth {
                ok: true,
                configured: false,
                status: "missing",
                message: "未读取到可用的 Earthdata/ASF 凭证。",
            });
        }
        Err(EarthdataCredentialError::Invalid | EarthdataCredentialError::NetrcParse) => {
            return Ok(DesktopEarthdataAuth {
                ok: true,
                configured: true,
                status: "invalid",
                message: "Earthdata/ASF 凭据格式无效，请检查后重新保存。",
            });
        }
        Err(_) => {
            return Ok(DesktopEarthdataAuth {
                ok: true,
                configured: true,
                status: "unavailable",
                message: "无法安全读取 Earthdata/ASF 凭据，尚未执行远端检测。",
            });
        }
    };
    let transfer = HttpTransfer::new_with_proxy_policy(
        TransferOptions {
            max_attempts: 1,
            backoff: Duration::ZERO,
            timeout: Duration::from_secs(8),
            ..TransferOptions::default()
        },
        proxy_policy,
    )
    .map_err(|_| {
        ShellError::new(
            "EARTHDATA_AUTH_PROBE_UNAVAILABLE",
            "无法创建 Earthdata 登录检测客户端",
        )
    })?;
    let (status, message) = match transfer.probe_earthdata_auth(&auth) {
        EarthdataAuthProbeOutcome::Valid => ("valid", "Earthdata Login 已确认当前凭据有效。"),
        EarthdataAuthProbeOutcome::Invalid => (
            "invalid",
            "Earthdata Login 拒绝了当前账号密码或 Token，请检查凭据。",
        ),
        EarthdataAuthProbeOutcome::ReachableUnverified => (
            "unknown",
            "Earthdata Login 服务可达；当前 Token 暂无法免变更直接核验，下载时仍按原认证链使用。",
        ),
        EarthdataAuthProbeOutcome::Unavailable => (
            "unavailable",
            "无法连接 Earthdata Login 或服务暂时不可用；已保存凭据未被修改。",
        ),
    };
    Ok(DesktopEarthdataAuth {
        ok: true,
        configured: true,
        status,
        message,
    })
}

#[tauri::command]
pub fn credential_save_cdse_login(
    window: tauri::WebviewWindow,
    username: String,
    password: String,
) -> Result<DesktopCredentialMutation, ShellError> {
    validate_application_window(&window)?;
    store_desktop_cdse_login(&username, &password).map_err(credential_error)?;
    Ok(credential_mutation(false))
}

#[tauri::command]
pub fn credential_clear_cdse(
    window: tauri::WebviewWindow,
) -> Result<DesktopCredentialMutation, ShellError> {
    validate_application_window(&window)?;
    let removed = clear_desktop_cdse().map_err(credential_error)?;
    Ok(credential_mutation(removed))
}

#[tauri::command]
pub fn credential_save_usgs_login(
    window: tauri::WebviewWindow,
    username: String,
    application_token: String,
) -> Result<DesktopCredentialMutation, ShellError> {
    validate_application_window(&window)?;
    store_desktop_usgs_login(&username, &application_token).map_err(credential_error)?;
    Ok(credential_mutation(false))
}

#[tauri::command]
pub fn credential_clear_usgs(
    window: tauri::WebviewWindow,
) -> Result<DesktopCredentialMutation, ShellError> {
    validate_application_window(&window)?;
    let removed = clear_desktop_usgs().map_err(credential_error)?;
    Ok(credential_mutation(removed))
}

#[tauri::command]
pub fn credential_save_earthdata_token(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    token: String,
) -> Result<DesktopCredentialMutation, ShellError> {
    validate_application_window(&window)?;
    store_desktop_earthdata_token(&token).map_err(credential_error)?;
    state.earthdata_credentials_changed();
    Ok(credential_mutation(false))
}

#[tauri::command]
pub fn credential_save_earthdata_login(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    username: String,
    password: String,
) -> Result<DesktopCredentialMutation, ShellError> {
    validate_application_window(&window)?;
    store_desktop_earthdata_login(&username, &password).map_err(credential_error)?;
    state.earthdata_credentials_changed();
    Ok(credential_mutation(false))
}

#[tauri::command]
pub fn credential_clear_earthdata(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
) -> Result<DesktopCredentialMutation, ShellError> {
    validate_application_window(&window)?;
    let removed = clear_desktop_earthdata().map_err(credential_error)?;
    state.earthdata_credentials_changed();
    Ok(credential_mutation(removed))
}

#[tauri::command]
pub fn credential_save_bhoonidhi_login(
    window: tauri::WebviewWindow,
    username: String,
    password: String,
) -> Result<DesktopCredentialMutation, ShellError> {
    validate_application_window(&window)?;
    store_desktop_bhoonidhi_login(&username, &password).map_err(credential_error)?;
    Ok(credential_mutation(false))
}

#[tauri::command]
pub fn credential_clear_bhoonidhi(
    window: tauri::WebviewWindow,
) -> Result<DesktopCredentialMutation, ShellError> {
    validate_application_window(&window)?;
    let removed = clear_desktop_bhoonidhi().map_err(credential_error)?;
    Ok(credential_mutation(removed))
}

#[tauri::command]
pub fn credential_save_opentopography_key(
    window: tauri::WebviewWindow,
    api_key: String,
) -> Result<DesktopCredentialMutation, ShellError> {
    validate_application_window(&window)?;
    store_desktop_opentopography_key(&api_key).map_err(credential_error)?;
    Ok(credential_mutation(false))
}

#[tauri::command]
pub fn credential_clear_opentopography_key(
    window: tauri::WebviewWindow,
) -> Result<DesktopCredentialMutation, ShellError> {
    validate_application_window(&window)?;
    let removed = clear_desktop_opentopography_key().map_err(credential_error)?;
    Ok(credential_mutation(removed))
}

#[tauri::command]
pub fn credential_save_gacos_email(
    window: tauri::WebviewWindow,
    email: String,
) -> Result<DesktopCredentialMutation, ShellError> {
    validate_application_window(&window)?;
    store_desktop_gacos_email(&email).map_err(credential_error)?;
    Ok(credential_mutation(false))
}

#[tauri::command]
pub fn credential_clear_gacos_email(
    window: tauri::WebviewWindow,
) -> Result<DesktopCredentialMutation, ShellError> {
    validate_application_window(&window)?;
    let removed = clear_desktop_gacos_email().map_err(credential_error)?;
    Ok(credential_mutation(removed))
}

#[tauri::command]
pub async fn application_create_landsat_asset_task(
    window: tauri::WebviewWindow,
    request: LandsatCreateAssetTaskRequest,
) -> Result<CoreServiceSnapshot, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .create_landsat_asset_task(&request)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_create_reviewed_mpc_asset_task(
    window: tauri::WebviewWindow,
    request: ReviewedMpcCreateAssetTaskRequest,
) -> Result<CoreServiceSnapshot, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .create_reviewed_mpc_asset_task(&request)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_create_landsat_bundle_batch_task(
    window: tauri::WebviewWindow,
    request: insar_application_service::LandsatCreateBundleBatchTaskRequest,
) -> Result<insar_application_service::LandsatCreateBundleBatchTaskResult, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .create_landsat_bundle_batch_task(&request)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_create_landsat_asset_batch_task(
    window: tauri::WebviewWindow,
    request: insar_application_service::LandsatCreateAssetBatchTaskRequest,
) -> Result<insar_application_service::LandsatCreateAssetBatchTaskResult, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .create_landsat_asset_batch_task(&request)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_task_delivery_admission(
    window: tauri::WebviewWindow,
    task_id: String,
) -> Result<Option<insar_application_service::OpticalDeliveryAdmissionSummary>, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .task_delivery_admission(&task_id)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_optical_preparation_progress(
    window: tauri::WebviewWindow,
    request_id: String,
) -> Result<Option<insar_application_service::OpticalPreparationProgress>, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .optical_preparation_progress(&request_id)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_create_sentinel2_archive_batch_task(
    window: tauri::WebviewWindow,
    request: Sentinel2CreateArchiveBatchTaskRequest,
) -> Result<Sentinel2CreateArchiveBatchTaskResult, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .create_sentinel2_archive_batch_task(&request)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_create_sentinel2_archive_task(
    window: tauri::WebviewWindow,
    request: Sentinel2CreateArchiveTaskRequest,
) -> Result<CoreServiceSnapshot, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .create_sentinel2_archive_task(&request)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_create_sentinel2_band_task(
    window: tauri::WebviewWindow,
    request: Sentinel2CreateBandTaskRequest,
) -> Result<CoreServiceSnapshot, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .create_sentinel2_band_task(&request)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_create_sentinel2_band_batch_task(
    window: tauri::WebviewWindow,
    request: insar_application_service::Sentinel2CreateBandBatchTaskRequest,
) -> Result<insar_application_service::Sentinel2CreateBandBatchTaskResult, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .create_sentinel2_band_batch_task(&request)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_create_sentinel2_l1c_band_batch_task(
    window: tauri::WebviewWindow,
    request: insar_application_service::Sentinel2L1cCreateBandBatchTaskRequest,
) -> Result<insar_application_service::Sentinel2L1cCreateBandBatchTaskResult, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .create_sentinel2_l1c_band_batch_task(&request)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

fn optical_derived_error_code(reason: &str) -> Option<&'static str> {
    [
        "OPTICAL_DERIVED_SOURCE_UNSUPPORTED",
        "OPTICAL_MOSAIC_TEMPORAL_SPAN_EXCEEDED",
        "OPTICAL_CLOUD_MASK_BAND_MISSING",
        "OPTICAL_CLOUD_MASK_UNSUPPORTED_L1C",
        "OPTICAL_AOI_MISSING",
        "OPTICAL_AOI_INVALID",
        "OPTICAL_DERIVED_SECRET_REJECTED",
        "OPTICAL_DERIVED_BAND_MISSING",
        "OPTICAL_DERIVED_BAND_UNSUPPORTED",
        "AOI_SCENE_DISJOINT",
        "OPTICAL_PRECHECK_BLOCKED",
        "OPTICAL_SOURCE_BATCH_NOT_FOUND",
        "OPTICAL_SOURCE_REVISION_CONFLICT",
        "OPTICAL_DERIVED_SCENE_COUNT_INVALID",
        "OPTICAL_READBACK_VERIFY_FAILED",
    ]
    .into_iter()
    .find(|code| reason.contains(code))
}

#[tauri::command]
pub async fn application_precheck_optical_aoi_coverage(
    window: tauri::WebviewWindow,
    request: insar_application_service::OpticalDerivedPrecheckBatchRequest,
) -> Result<insar_gis::OpticalPrecheckReport, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .precheck_optical_aoi_coverage(request)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_create_optical_derived_task(
    window: tauri::WebviewWindow,
    request: insar_application_service::OpticalDerivedCreateTaskRequest,
) -> Result<insar_application_service::OpticalDerivedTaskCreatedResult, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .create_optical_derived_task(request)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_create_hls_band_batch_task(
    window: tauri::WebviewWindow,
    request: insar_application_service::HlsCreateBandBatchTaskRequest,
) -> Result<insar_application_service::HlsCreateBandBatchTaskResult, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .create_hls_band_batch_task(&request)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub fn application_nisar_remote_subset_capability(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
) -> Result<NisarRemoteSubsetCapability, ShellError> {
    validate_application_window(&window)?;
    state
        .nisar_remote_subset_capability()
        .map_err(application_error)
}

#[tauri::command]
pub fn application_create_nisar_remote_subset_task(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    request: NisarCreateRemoteSubsetTaskRequest,
) -> Result<CoreServiceSnapshot, ShellError> {
    validate_application_window(&window)?;
    state
        .create_nisar_remote_subset_task(&request)
        .map_err(application_error)
}

#[tauri::command]
pub fn application_prepare_nisar_subset_intent_v2(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    request: NisarPrepareSubsetIntentV2Request,
) -> Result<NisarPreparedSubsetIntentV2, ShellError> {
    validate_application_window(&window)?;
    state
        .prepare_nisar_subset_intent_v2(&request)
        .map_err(application_error)
}

#[tauri::command]
pub fn application_confirm_nisar_subset_task_v2(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    request: NisarConfirmSubsetTaskV2Request,
) -> Result<CoreServiceSnapshot, ShellError> {
    validate_application_window(&window)?;
    state
        .confirm_nisar_subset_task_v2(&request)
        .map_err(application_error)
}

#[tauri::command]
pub fn application_preflight_nisar_subset_batch_v2(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    request: NisarPreflightSubsetBatchV2Request,
) -> Result<NisarPreflightSubsetBatchV2, ShellError> {
    validate_application_window(&window)?;
    state
        .preflight_nisar_subset_batch_v2(&request)
        .map_err(application_error)
}

#[tauri::command]
pub fn application_prepare_nisar_subset_batch_v2(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    request: NisarPrepareSubsetBatchV2Request,
) -> Result<NisarPreparedSubsetBatchV2, ShellError> {
    validate_application_window(&window)?;
    state
        .prepare_nisar_subset_batch_v2(&request)
        .map_err(application_error)
}

#[tauri::command]
pub fn application_confirm_nisar_subset_batch_task_v2(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    request: NisarConfirmSubsetBatchTaskV2Request,
) -> Result<CoreServiceSnapshot, ShellError> {
    validate_application_window(&window)?;
    state
        .confirm_nisar_subset_batch_task_v2(&request)
        .map_err(application_error)
}

#[tauri::command]
pub fn application_append_nisar_subset_batch_task_v2(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    request: NisarAppendSubsetBatchTaskV2Request,
) -> Result<CoreServiceSnapshot, ShellError> {
    validate_application_window(&window)?;
    state
        .append_nisar_subset_batch_task_v2(&request)
        .map_err(application_error)
}

#[tauri::command]
pub fn application_read_nisar_subset_append_context_v2(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    request: NisarAppendSubsetContextV2Request,
) -> Result<NisarAppendSubsetContextV2, ShellError> {
    validate_application_window(&window)?;
    state
        .read_nisar_subset_append_context_v2(&request)
        .map_err(application_error)
}

#[tauri::command]
pub fn application_preflight_nisar_subset_append_v2(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    request: NisarPreflightSubsetAppendV2Request,
) -> Result<NisarPreflightSubsetAppendV2, ShellError> {
    validate_application_window(&window)?;
    state
        .preflight_nisar_subset_append_v2(&request)
        .map_err(application_error)
}

#[tauri::command]
pub fn application_create_nisar_subset_successor_task_v2(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    request: NisarCreateSubsetSuccessorTaskV2Request,
) -> Result<CoreServiceSnapshot, ShellError> {
    validate_application_window(&window)?;
    state
        .create_nisar_subset_successor_task_v2(&request)
        .map_err(application_error)
}

#[tauri::command]
pub fn application_read_nisar_archived_recovery_context_v2(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    source_task_id: String,
    new_task_id: String,
) -> Result<insar_application_service::NisarArchivedRecoveryContextV2, ShellError> {
    validate_application_window(&window)?;
    state
        .read_nisar_archived_recovery_context_v2(&source_task_id, &new_task_id)
        .map_err(application_error)
}

#[tauri::command]
pub fn application_create_nisar_archived_recovery_v2(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    request: insar_application_service::NisarCreateArchivedRecoveryV2Request,
) -> Result<CoreServiceSnapshot, ShellError> {
    validate_application_window(&window)?;
    state
        .create_nisar_archived_recovery_v2(&request)
        .map_err(application_error)
}

#[tauri::command]
pub fn application_resume_nisar_subset_task(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    task_id: String,
) -> Result<CoreServiceSnapshot, ShellError> {
    validate_application_window(&window)?;
    state
        .resume_nisar_subset_task(&task_id)
        .map_err(application_error)
}

#[tauri::command]
pub fn application_start_newly_confirmed_task(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    task_id: String,
) -> Result<CoreServiceSnapshot, ShellError> {
    validate_application_window(&window)?;
    state
        .start_newly_confirmed_task(&task_id)
        .map_err(application_error)
}

#[tauri::command]
pub async fn application_update_search_workbench(
    window: tauri::WebviewWindow,
    request: StoreUpdateWorkbenchRequest,
) -> Result<CoreServiceSnapshot, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .update_search_workbench(&request)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_create_task(
    window: tauri::WebviewWindow,
    batch_id: String,
    revision: u64,
    item_ids: Vec<String>,
    options: insar_application_service::UnifiedCreateTaskOptions,
) -> Result<insar_core::TaskRef, insar_core::TypedError> {
    validate_application_window(&window).map_err(|error| insar_core::TypedError {
        code: error.code.to_owned(),
        message_zh: "窗口未授权".to_owned(),
        retryable: false,
        needs_account: None,
        needs_user_confirmation: false,
    })?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .create_task(batch_id, revision, item_ids, options)
    })
    .await
    .map_err(|_| insar_core::TypedError {
        code: "V3_APPLICATION_UNAVAILABLE".to_owned(),
        message_zh: "任务服务暂不可用".to_owned(),
        retryable: true,
        needs_account: None,
        needs_user_confirmation: false,
    })?
}

#[tauri::command]
pub async fn application_preview_all_match_task(
    window: tauri::WebviewWindow,
    request: insar_application_service::BatchAllMatchPreviewRequest,
) -> Result<insar_application_service::BatchAllMatchPreview, insar_core::TypedError> {
    validate_application_window(&window).map_err(|error| insar_core::TypedError {
        code: error.code.to_owned(),
        message_zh: "窗口未授权".to_owned(),
        retryable: false,
        needs_account: None,
        needs_user_confirmation: false,
    })?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .preview_all_match_task(request)
    })
    .await
    .map_err(|_| insar_core::TypedError {
        code: "V3_APPLICATION_UNAVAILABLE".to_owned(),
        message_zh: "任务服务暂不可用".to_owned(),
        retryable: true,
        needs_account: None,
        needs_user_confirmation: false,
    })?
}

#[tauri::command]
pub async fn application_create_all_match_task(
    window: tauri::WebviewWindow,
    request: insar_application_service::BatchAllMatchCreateTaskRequest,
) -> Result<insar_application_service::BatchAllMatchTaskOutcome, insar_core::TypedError> {
    validate_application_window(&window).map_err(|error| insar_core::TypedError {
        code: error.code.to_owned(),
        message_zh: "窗口未授权".to_owned(),
        retryable: false,
        needs_account: None,
        needs_user_confirmation: false,
    })?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .create_all_match_task(request)
    })
    .await
    .map_err(|_| insar_core::TypedError {
        code: "V3_APPLICATION_UNAVAILABLE".to_owned(),
        message_zh: "任务服务暂不可用".to_owned(),
        retryable: true,
        needs_account: None,
        needs_user_confirmation: false,
    })?
}

#[tauri::command]
pub async fn application_create_download_task(
    window: tauri::WebviewWindow,
    request: StoreCreateTaskRequest,
) -> Result<CoreServiceSnapshot, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .create_download_task(&request)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub fn application_append_download_task(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    request: StoreAppendTaskRequest,
) -> Result<CoreServiceSnapshot, ShellError> {
    validate_application_window(&window)?;
    state
        .append_download_task(&request)
        .map_err(application_error)
}

#[tauri::command]
pub async fn application_create_task_from_workbench(
    window: tauri::WebviewWindow,
    request: CreateTaskFromWorkbenchRequest,
) -> Result<CoreServiceSnapshot, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .create_task_from_workbench(&request)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub fn application_append_task_from_workbench(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    request: AppendTaskFromWorkbenchRequest,
) -> Result<CoreServiceSnapshot, ShellError> {
    validate_application_window(&window)?;
    state
        .append_task_from_workbench(&request)
        .map_err(application_error)
}

#[tauri::command]
pub async fn application_query_source_append_status(
    window: tauri::WebviewWindow,
    request: insar_application_service::AppendTaskFromSourceBatchRequest,
) -> Result<insar_application_service::SourceMutationStatus, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .query_source_mutation_status(
                insar_application_service::SourceMutationStatusQuery::SourceAppend(&request),
            )
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_query_historical_successor_status(
    window: tauri::WebviewWindow,
    request: insar_application_service::CreateTaskFromHistoricalSourceBatchRequest,
) -> Result<insar_application_service::SourceMutationStatus, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .query_source_mutation_status(
                insar_application_service::SourceMutationStatusQuery::HistoricalSuccessor(&request),
            )
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_query_burst_append_status(
    window: tauri::WebviewWindow,
    request: insar_application_service::BurstAppendAssetTaskRequest,
) -> Result<insar_application_service::SourceMutationStatus, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .query_source_mutation_status(
                insar_application_service::SourceMutationStatusQuery::BurstAppend(&request),
            )
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_query_burst_successor_status(
    window: tauri::WebviewWindow,
    request: insar_application_service::BurstCreateHistoricalAssetTaskRequest,
) -> Result<insar_application_service::SourceMutationStatus, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .query_source_mutation_status(
                insar_application_service::SourceMutationStatusQuery::BurstSuccessor(&request),
            )
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_query_nisar_append_status(
    window: tauri::WebviewWindow,
    request: insar_application_service::NisarAppendSubsetBatchTaskV2Request,
) -> Result<insar_application_service::SourceMutationStatus, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .query_source_mutation_status(
                insar_application_service::SourceMutationStatusQuery::NisarAppend(&request),
            )
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_query_nisar_successor_status(
    window: tauri::WebviewWindow,
    request: insar_application_service::NisarCreateSubsetSuccessorTaskV2Request,
) -> Result<insar_application_service::SourceMutationStatus, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .query_source_mutation_status(
                insar_application_service::SourceMutationStatusQuery::NisarSuccessor(&request),
            )
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_query_nisar_archived_recovery_status(
    window: tauri::WebviewWindow,
    request: insar_application_service::NisarCreateArchivedRecoveryV2Request,
) -> Result<insar_application_service::SourceMutationStatus, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .query_source_mutation_status(
                insar_application_service::SourceMutationStatusQuery::NisarArchivedRecovery(
                    &request,
                ),
            )
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_query_reacquire_status(
    window: tauri::WebviewWindow,
    request: insar_core::StoreUpdateTaskRequest,
) -> Result<insar_application_service::SourceMutationStatus, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .query_source_mutation_status(
                insar_application_service::SourceMutationStatusQuery::Reacquire(&request),
            )
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub fn application_append_task_from_source_batch(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    request: AppendTaskFromSourceBatchRequest,
) -> Result<CoreServiceSnapshot, ShellError> {
    validate_application_window(&window)?;
    state
        .append_task_from_source_batch(&request)
        .map_err(application_error)
}

#[tauri::command]
pub fn application_create_task_from_historical_source_batch(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    request: CreateTaskFromHistoricalSourceBatchRequest,
) -> Result<CoreServiceSnapshot, ShellError> {
    validate_application_window(&window)?;
    state
        .create_task_from_historical_source_batch(&request)
        .map_err(application_error)
}

#[tauri::command]
pub fn application_create_burst_asset_task(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    request: BurstCreateAssetTaskRequest,
) -> Result<CoreServiceSnapshot, ShellError> {
    validate_application_window(&window)?;
    state
        .create_burst_asset_task(&request)
        .map_err(application_error)
}

#[tauri::command]
pub fn application_append_burst_asset_task(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    request: BurstAppendAssetTaskRequest,
) -> Result<CoreServiceSnapshot, ShellError> {
    validate_application_window(&window)?;
    state
        .append_burst_asset_task(&request)
        .map_err(application_error)
}

#[tauri::command]
pub fn application_create_historical_burst_asset_task(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    request: BurstCreateHistoricalAssetTaskRequest,
) -> Result<CoreServiceSnapshot, ShellError> {
    validate_application_window(&window)?;
    state
        .create_historical_burst_asset_task(&request)
        .map_err(application_error)
}

#[tauri::command]
pub async fn application_task_action(
    window: tauri::WebviewWindow,
    task_id: String,
    revision: u64,
    action: insar_application_service::UnifiedTaskAction,
) -> Result<insar_core::TaskRef, insar_core::TypedError> {
    validate_application_window(&window).map_err(|error| insar_core::TypedError {
        code: error.code.to_owned(),
        message_zh: "窗口未授权".to_owned(),
        retryable: false,
        needs_account: None,
        needs_user_confirmation: false,
    })?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .task_action(task_id, revision, action)
    })
    .await
    .map_err(|_| insar_core::TypedError {
        code: "V3_APPLICATION_UNAVAILABLE".to_owned(),
        message_zh: "任务服务暂不可用".to_owned(),
        retryable: true,
        needs_account: None,
        needs_user_confirmation: false,
    })?
}

/// C1 retry fallback: re-runs the failed task's frozen search off the window
/// thread and rebuilds its items in place, or reports the changed product set.
#[tauri::command]
pub async fn application_task_retry_fallback(
    window: tauri::WebviewWindow,
    task_id: String,
    confirm_refreshed_batch_id: Option<String>,
) -> Result<insar_application_service::TaskRetryFallbackOutcome, insar_core::TypedError> {
    validate_application_window(&window).map_err(|error| insar_core::TypedError {
        code: error.code.to_owned(),
        message_zh: "窗口未授权".to_owned(),
        retryable: false,
        needs_account: None,
        needs_user_confirmation: false,
    })?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .task_retry_fallback(task_id, confirm_refreshed_batch_id)
    })
    .await
    .map_err(|_| insar_core::TypedError {
        code: "V3_APPLICATION_UNAVAILABLE".to_owned(),
        message_zh: "任务服务暂不可用".to_owned(),
        retryable: true,
        needs_account: None,
        needs_user_confirmation: false,
    })?
}

#[tauri::command]
pub async fn application_update_download_task(
    window: tauri::WebviewWindow,
    request: StoreUpdateTaskRequest,
) -> Result<CoreServiceSnapshot, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .update_download_task(&request)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub fn application_update_download_task_view(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    request: StoreUpdateTaskViewRequest,
) -> Result<CoreServiceSnapshot, ShellError> {
    validate_application_window(&window)?;
    state
        .update_download_task_view(&request)
        .map_err(application_error)
}

#[tauri::command]
pub fn application_archive_download_task(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ApplicationServiceState>,
    request: StoreArchiveTaskRequest,
) -> Result<ArchiveDownloadTaskAck, ShellError> {
    validate_application_window(&window)?;
    state
        .archive_download_task(&request)
        .map_err(application_error)
}

#[tauri::command]
pub async fn application_delete_download_history(
    window: tauri::WebviewWindow,
    request: StoreDeleteHistoryRequest,
) -> Result<CoreServiceSnapshot, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .delete_download_history(&request)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_task_item_action(
    window: tauri::WebviewWindow,
    task_id: String,
    revision: Option<u64>,
    item_id: String,
    action: insar_application_service::UnifiedTaskItemAction,
) -> Result<insar_core::TaskRef, insar_core::TypedError> {
    validate_application_window(&window).map_err(|error| insar_core::TypedError {
        code: error.code.to_owned(),
        message_zh: "窗口未授权".to_owned(),
        retryable: false,
        needs_account: None,
        needs_user_confirmation: false,
    })?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .task_item_action(task_id, revision, item_id, action)
    })
    .await
    .map_err(|_| insar_core::TypedError {
        code: "V3_APPLICATION_UNAVAILABLE".to_owned(),
        message_zh: "任务服务暂不可用".to_owned(),
        retryable: true,
        needs_account: None,
        needs_user_confirmation: false,
    })?
}

#[tauri::command]
pub async fn application_task_delete_history(
    window: tauri::WebviewWindow,
    task_id: String,
) -> Result<(), insar_core::TypedError> {
    validate_application_window(&window).map_err(|error| insar_core::TypedError {
        code: error.code.to_owned(),
        message_zh: "窗口未授权".to_owned(),
        retryable: false,
        needs_account: None,
        needs_user_confirmation: false,
    })?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .task_delete_history(task_id)
    })
    .await
    .map_err(|_| insar_core::TypedError {
        code: "V3_APPLICATION_UNAVAILABLE".to_owned(),
        message_zh: "任务服务暂不可用".to_owned(),
        retryable: true,
        needs_account: None,
        needs_user_confirmation: false,
    })?
}

#[tauri::command]
pub async fn application_task_set_pinned(
    window: tauri::WebviewWindow,
    task_id: String,
    pinned: bool,
) -> Result<Vec<String>, insar_core::TypedError> {
    validate_application_window(&window).map_err(|error| insar_core::TypedError {
        code: error.code.to_owned(),
        message_zh: "窗口未授权".to_owned(),
        retryable: false,
        needs_account: None,
        needs_user_confirmation: false,
    })?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .set_task_pinned(task_id, pinned)
    })
    .await
    .map_err(|_| insar_core::TypedError {
        code: "V3_APPLICATION_UNAVAILABLE".to_owned(),
        message_zh: "任务服务暂不可用".to_owned(),
        retryable: true,
        needs_account: None,
        needs_user_confirmation: false,
    })?
}

#[tauri::command]
pub async fn application_list_pinned_tasks(
    window: tauri::WebviewWindow,
) -> Result<Vec<String>, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .list_pinned_tasks()
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_task_retry_fallback_eligibility(
    window: tauri::WebviewWindow,
    task_ids: Vec<String>,
) -> Result<std::collections::BTreeMap<String, bool>, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .task_retry_fallback_eligibility(&task_ids)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn check_existing_downloaded_scenes(
    window: tauri::WebviewWindow,
    items: Vec<insar_application_service::ExistingDownloadCheckItem>,
    target_dir: Option<String>,
) -> Result<Vec<insar_application_service::ExistingDownloadMatch>, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .check_existing_downloaded_scenes(items, target_dir)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

fn validate_application_window(window: &tauri::WebviewWindow) -> Result<(), ShellError> {
    if !is_allowed_desktop_window_label(window.label()) {
        return Err(ShellError::window_not_allowed());
    }
    #[cfg(target_os = "macos")]
    if crate::macos_cdp_bridge::is_active() {
        return Ok(());
    }
    let url = window
        .url()
        .map_err(|_| ShellError::window_url_unavailable())?;
    if !is_allowed_desktop_origin(&url, cfg!(debug_assertions)) {
        return Err(ShellError::invalid_origin());
    }
    Ok(())
}

fn application_error(error: insar_application_service::ApplicationServiceError) -> ShellError {
    let code = error.code();
    if let insar_application_service::ApplicationServiceError::DemRangeLimitExceeded {
        width,
        height,
        pixels,
        coarser_dataset_available,
    } = &error
    {
        // Numbers only; the UI words the message.
        return ShellError::application_service_with_context(
            code,
            format!(
                "width={width};height={height};pixels={pixels};coarser={}",
                u8::from(*coarser_dataset_available)
            ),
        );
    }
    if let insar_application_service::ApplicationServiceError::InvalidRequest { reason } = &error {
        if let Some(optical_code) = optical_derived_error_code(reason) {
            return ShellError::application_service(optical_code);
        }
        let context = sanitize_invalid_request_reason(reason);
        if context != "invalid request reason redacted" {
            return ShellError::application_service_with_context(code, context);
        }
    }
    if let insar_application_service::ApplicationServiceError::Provider(source) = &error {
        // The provider owns this numeric status. Never forward its raw response,
        // request URL, query, authorization headers or arbitrary debug context.
        if source.code == "BHOONIDHI_HTTP_ERROR" {
            if let Some(status) = source
                .debug_context
                .get("status")
                .and_then(serde_json::Value::as_u64)
                .filter(|status| (300..=599).contains(status))
            {
                return ShellError::application_service_with_unavailable_context(
                    code,
                    format!("http_status:{status}"),
                );
            }
        }
        // Boundary refusals name the selector the UI switches to. Only the
        // fixed token crosses, next to the bounded message.
        let fallback = source
            .debug_context
            .get("fallback")
            .and_then(serde_json::Value::as_str)
            .filter(|fallback| *fallback == "china_selector");
        let message = sanitize_provider_user_message(&source.user_message);
        if let Some(fallback) = fallback {
            let context = match message {
                Some(message) => format!("fallback:{fallback};{message}"),
                None => format!("fallback:{fallback}"),
            };
            return ShellError::application_service_with_unavailable_context(code, context);
        }
        if let Some(context) = message {
            return ShellError::application_service_with_unavailable_context(code, context);
        }
    }
    if let insar_application_service::ApplicationServiceError::CapabilityUnavailable {
        capability,
    } = &error
    {
        if *capability == "formal_dem_download" {
            return ShellError {
                code: code.to_owned(),
                message_key: "application_service.dem_native_gis_unavailable".to_owned(),
                context: Some((*capability).to_owned()),
                diagnostic: None,
            };
        }
        if *capability == "formal_dem_credential" {
            return ShellError {
                code: code.to_owned(),
                message_key: "application_service.dem_credential_unavailable".to_owned(),
                context: Some((*capability).to_owned()),
                diagnostic: None,
            };
        }
        return ShellError::application_service_with_unavailable_context(
            code,
            (*capability).to_owned(),
        );
    }
    ShellError::application_service(code)
}

fn application_sar_error(
    request: &ApplicationSarSearchAndFreezeRequest,
    error: insar_application_service::ApplicationServiceError,
) -> ShellError {
    use insar_application_service::ApplicationServiceError;

    let code = error.code();
    let context = match &error {
        ApplicationServiceError::InvalidRequest { reason } => {
            Some(sanitize_invalid_request_reason(reason))
        }
        _ => None,
    };
    let sanitized_reason = context.clone().unwrap_or_else(|| code.to_owned());
    let query = &request.search.query;
    let aoi_state = if query.aoi_geojson.is_some() || request.aoi_geojson.is_some() {
        "geojson"
    } else if query.bbox.is_some() {
        "bbox"
    } else if query.aoi_wkt.is_some() {
        "wkt"
    } else {
        "none"
    };
    let diagnostic = SarSearchFailureDiagnostic {
        operation: "sar_search_and_freeze".to_owned(),
        route: "legacy_asf_search".to_owned(),
        sanitized_reason,
        dataset: sanitize_diagnostic_scalar(&query.dataset, 80),
        query_shape: SarSearchQueryShapeDiagnostic {
            platform_count: query.platforms.len().min(16),
            processing_level_count: query.processing_levels.len().min(16),
            product_family_count: query.product_families.len().min(16),
            beam_mode_count: query.beam_modes.len().min(16),
            polarization_count: query.polarizations.len().min(16),
            has_instrument: query.instrument.is_some(),
            has_relative_orbit: query.relative_orbit.is_some(),
            has_frame: query.frame.is_some(),
            has_asf_frame: query.asf_frame.is_some(),
            has_path: query.path.is_some(),
            has_track_range: query.track_range.is_some(),
            has_frame_range: query.frame_range.is_some(),
            has_aoi_geojson: query.aoi_geojson.is_some(),
            has_aoi_wkt: query.aoi_wkt.is_some(),
            has_bbox: query.bbox.is_some(),
            has_burst: query.burst.is_some(),
        },
        aoi_state: aoi_state.to_owned(),
        date_range: SarSearchDateRangeDiagnostic {
            start: query
                .start
                .as_deref()
                .map(|value| sanitize_diagnostic_scalar(value, 48)),
            end: query
                .end
                .as_deref()
                .map(|value| sanitize_diagnostic_scalar(value, 48)),
        },
        filter_state: SarSearchFilterDiagnostic {
            platforms: sanitize_diagnostic_values(&query.platforms),
            instrument: sanitize_optional_diagnostic_scalar(query.instrument.as_deref()),
            processing_levels: sanitize_diagnostic_values(&query.processing_levels),
            product_families: sanitize_diagnostic_values(&query.product_families),
            beam_modes: sanitize_diagnostic_values(&query.beam_modes),
            polarizations: sanitize_diagnostic_values(&query.polarizations),
            flight_direction: sanitize_optional_diagnostic_scalar(
                query.flight_direction.as_deref(),
            ),
            look_direction: sanitize_optional_diagnostic_scalar(query.look_direction.as_deref()),
            relative_orbit: query.relative_orbit,
            frame: query.frame,
            asf_frame: sanitize_optional_diagnostic_scalar(query.asf_frame.as_deref()),
            path: query.path,
            track_range: query
                .track_range
                .as_ref()
                .map(|range| SarSearchRangeDiagnostic {
                    start: range
                        .start
                        .as_deref()
                        .map(|value| sanitize_diagnostic_scalar(value, 48)),
                    end: range
                        .end
                        .as_deref()
                        .map(|value| sanitize_diagnostic_scalar(value, 48)),
                }),
            frame_range: query
                .frame_range
                .as_ref()
                .map(|range| SarSearchRangeDiagnostic {
                    start: range
                        .start
                        .as_deref()
                        .map(|value| sanitize_diagnostic_scalar(value, 48)),
                    end: range
                        .end
                        .as_deref()
                        .map(|value| sanitize_diagnostic_scalar(value, 48)),
                }),
        },
        max_results: query.max_results,
    };
    ShellError::application_sar_search(code, context, diagnostic)
}

fn sanitize_optional_diagnostic_scalar(value: Option<&str>) -> Option<String> {
    value.map(|value| sanitize_diagnostic_scalar(value, 80))
}

fn sanitize_diagnostic_values(values: &[String]) -> Vec<String> {
    values
        .iter()
        .take(16)
        .map(|value| sanitize_diagnostic_scalar(value, 80))
        .collect()
}

fn sanitize_invalid_request_reason(reason: &str) -> String {
    let trimmed = reason.trim();
    if is_safe_diagnostic_scalar(trimmed, 240) {
        trimmed.to_owned()
    } else {
        "invalid request reason redacted".to_owned()
    }
}

fn sanitize_provider_user_message(message: &str) -> Option<String> {
    let trimmed = message.trim();
    if trimmed.is_empty() || trimmed.chars().count() > 240 {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    if [
        "password",
        "token",
        "secret",
        "authorization",
        "bearer",
        "http://",
        "https://",
        "applicationservice",
        "desktop api",
        "worker",
        "tauri",
        "rust",
        "sidecar",
        "v3_",
        "gui",
        "does not expose",
        "not implemented",
        "task summary",
        "task-view",
        "snapshot",
        "internal",
        "ownership",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
    {
        return None;
    }
    if trimmed
        .chars()
        .any(|character| character == '/' || character == '\\')
    {
        return None;
    }
    if !trimmed.chars().all(|character| {
        character.is_alphanumeric()
            || character.is_whitespace()
            || matches!(
                character,
                '_' | '-'
                    | '.'
                    | ','
                    | ':'
                    | ';'
                    | '('
                    | ')'
                    | '+'
                    | '='
                    | '>'
                    | '<'
                    | '，'
                    | '。'
                    | '；'
                    | '：'
                    | '！'
                    | '？'
                    | '、'
                    | '（'
                    | '）'
            )
    }) {
        return None;
    }
    Some(trimmed.to_owned())
}

fn sanitize_diagnostic_scalar(value: &str, max_chars: usize) -> String {
    let trimmed = value.trim();
    if is_safe_diagnostic_scalar(trimmed, max_chars) {
        trimmed.to_owned()
    } else {
        "<redacted>".to_owned()
    }
}

fn is_safe_diagnostic_scalar(value: &str, max_chars: usize) -> bool {
    if value.is_empty() || value.chars().count() > max_chars {
        return false;
    }
    let lower = value.to_ascii_lowercase();
    if [
        "password",
        "token",
        "secret",
        "authorization",
        "bearer",
        "http://",
        "https://",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
    {
        return false;
    }
    value.chars().all(|character| {
        character.is_ascii_alphanumeric()
            || character.is_ascii_whitespace()
            || matches!(
                character,
                '_' | '-' | '.' | ',' | ':' | ';' | '(' | ')' | '+' | '=' | '>' | '<'
            )
    })
}

#[cfg(test)]
mod tests {
    use super::{application_error, application_sar_error};
    use insar_application_service::{
        ApplicationSarSearchAndFreezeRequest, ApplicationServiceError,
    };
    use insar_core::ContractError;
    use serde_json::{json, Map, Value};

    #[test]
    fn application_error_projection_discards_write_only_cdse_input() {
        let error = application_error(ApplicationServiceError::InvalidRequest {
            reason: "fixture-write-only-password".to_owned(),
        });
        let serialized = serde_json::to_string(&error).expect("ShellError serializes");

        assert_eq!(error.code, "V3_APPLICATION_INVALID_REQUEST");
        assert!(!serialized.contains("fixture-write-only-password"));
        assert!(!serialized.contains("reason"));
    }

    #[test]
    fn provider_error_projection_keeps_bounded_user_message_for_unknown_failures() {
        let error = application_error(ApplicationServiceError::Provider(
            ContractError::new(
                "PROVIDER_FIXTURE_UNKNOWN",
                "Bhoonidhi 网络请求失败",
                Map::new(),
            )
            .expect("safe provider error"),
        ));

        assert_eq!(error.code, "V3_PROVIDER_OPERATION_FAILED");
        assert_eq!(error.message_key, "application_service.unavailable");
        assert_eq!(error.context.as_deref(), Some("Bhoonidhi 网络请求失败"));
    }

    #[test]
    fn boundary_refusals_keep_their_code_and_china_selector_fallback() {
        for (code, message) in [
            (
                "BOUNDARY_ROUTE_REFUSED_CHINA_SCOPE",
                "中国（含台湾、香港、澳门）行政区请使用中国行政区选择器，不使用 geoBoundaries",
            ),
            (
                "BOUNDARY_ROUTE_REFUSED_DISPUTED",
                "该行政单元不提供 geoBoundaries 边界；该区域请使用中国行政区选择器",
            ),
            (
                "BOUNDARY_PROVIDER_IDENTITY_INVALID",
                "DataV 行政区请在中国行政区选择器中选择后加载，不经此处解析",
            ),
        ] {
            let error = application_error(ApplicationServiceError::Provider(
                ContractError::new(
                    code,
                    message,
                    Map::from_iter([
                        ("reason".to_owned(), Value::String("fixture".to_owned())),
                        (
                            "fallback".to_owned(),
                            Value::String("china_selector".to_owned()),
                        ),
                    ]),
                )
                .expect("safe provider error"),
            ));
            assert_eq!(error.code, code);
            assert_eq!(
                error.context.as_deref(),
                Some(format!("fallback:china_selector;{message}").as_str()),
                "{code}"
            );
        }
        // An unknown fallback value is never forwarded.
        let error = application_error(ApplicationServiceError::Provider(
            ContractError::new(
                "BOUNDARY_ROUTE_REFUSED_DISPUTED",
                "该区域请使用中国行政区选择器",
                Map::from_iter([(
                    "fallback".to_owned(),
                    Value::String("somewhere_else".to_owned()),
                )]),
            )
            .expect("safe provider error"),
        ));
        assert_eq!(
            error.context.as_deref(),
            Some("该区域请使用中国行政区选择器")
        );
    }

    #[test]
    fn provider_error_projection_redacts_internal_or_sensitive_user_message() {
        let error = application_error(ApplicationServiceError::Provider(
            ContractError::new(
                "PROVIDER_FIXTURE_UNKNOWN",
                "workspace snapshot route requires internal owner token=fixture-secret",
                Map::new(),
            )
            .expect("safe provider error"),
        ));

        let serialized = serde_json::to_string(&error).expect("ShellError serializes");
        assert!(error.context.is_none());
        assert!(!serialized.contains("fixture-secret"));
        assert!(!serialized.contains("internal owner"));
    }

    #[test]
    fn bhoonidhi_catalogue_http_error_preserves_only_numeric_status() {
        for status in [400, 413, 422] {
            let error = application_error(ApplicationServiceError::Provider(
                ContractError::new(
                    "BHOONIDHI_HTTP_ERROR",
                    "Bhoonidhi 请求失败",
                    Map::from_iter([
                        ("reason".to_owned(), json!("http_status")),
                        ("status".to_owned(), json!(status)),
                        (
                            "request_url".to_owned(),
                            json!("https://private/?token=fixture-secret"),
                        ),
                    ]),
                )
                .expect("safe provider error"),
            ));
            assert_eq!(error.code, "BHOONIDHI_HTTP_ERROR");
            assert_eq!(error.context, Some(format!("http_status:{status}")));
            let serialized = serde_json::to_string(&error).unwrap();
            assert!(!serialized.contains("private"));
            assert!(!serialized.contains("fixture-secret"));
        }
        for status in [0, 200, 999] {
            let error = application_error(ApplicationServiceError::Provider(
                ContractError::new(
                    "BHOONIDHI_HTTP_ERROR",
                    "Bhoonidhi 请求失败",
                    Map::from_iter([("status".to_owned(), json!(status))]),
                )
                .expect("safe provider error"),
            ));
            assert_ne!(error.context, Some(format!("http_status:{status}")));
        }
    }

    #[test]
    fn sar_invalid_request_projection_preserves_bounded_search_diagnostics() {
        let aoi = json!({
            "type": "Polygon",
            "coordinates": [[[100.0, 30.0], [101.0, 30.0], [101.0, 31.0], [100.0, 30.0]]]
        });
        let request: ApplicationSarSearchAndFreezeRequest = serde_json::from_value(json!({
            "request_id": "m1-search-diagnostic",
            "search": {"query": {
                "provider": "ASF",
                "dataset": "SENTINEL-1",
                "processing_levels": ["SLC"],
                "beam_modes": ["IW"],
                "polarizations": ["VV+VH"],
                "flight_direction": "ASCENDING",
                "look_direction": "RIGHT",
                "track_range": {"start": "118", "end": "118"},
                "frame_range": {"start": "42", "end": "42"},
                "start": "2026-08-01T00:00:00Z",
                "end": "2026-08-02T23:59:59Z",
                "aoi_geojson": aoi,
                "max_results": 20
            }},
            "batch_id": "search-diagnostic",
            "created_at": "2026-08-16T00:00:00Z",
            "aoi_name": "diagnostic fixture",
            "aoi_geojson": aoi,
            "page_size": 24
        }))
        .expect("valid diagnostic request");
        let error = application_sar_error(
            &request,
            ApplicationServiceError::InvalidRequest {
                reason: "M1 accepts only mapped ASF catalogue products".to_owned(),
            },
        );
        let serialized: Value = serde_json::to_value(&error).expect("ShellError serializes");

        assert_eq!(error.code, "V3_APPLICATION_INVALID_REQUEST");
        assert_eq!(
            error.context.as_deref(),
            Some("M1 accepts only mapped ASF catalogue products")
        );
        assert_eq!(
            serialized["diagnostic"]["operation"],
            "sar_search_and_freeze"
        );
        assert_eq!(serialized["diagnostic"]["route"], "legacy_asf_search");
        assert_eq!(serialized["diagnostic"]["dataset"], "SENTINEL-1");
        assert_eq!(serialized["diagnostic"]["aoi_state"], "geojson");
        assert_eq!(
            serialized["diagnostic"]["date_range"]["start"],
            "2026-08-01T00:00:00Z"
        );
        assert_eq!(
            serialized["diagnostic"]["filter_state"]["track_range"]["start"],
            "118"
        );
        assert_eq!(
            serialized["diagnostic"]["filter_state"]["frame_range"]["end"],
            "42"
        );
        assert_eq!(serialized["diagnostic"]["max_results"], 20);
        assert_eq!(
            serialized["diagnostic"]["query_shape"]["has_aoi_geojson"],
            true
        );
        assert_eq!(
            serialized["diagnostic"]["query_shape"]["has_track_range"],
            true
        );
        assert_eq!(
            serialized["diagnostic"]["query_shape"]["processing_level_count"],
            1
        );
        assert!(serialized["diagnostic"].get("aoi_geojson").is_none());
    }

    #[test]
    fn sar_invalid_request_projection_redacts_secrets_urls_paths_and_unbounded_inputs() {
        let request: ApplicationSarSearchAndFreezeRequest = serde_json::from_value(json!({
            "request_id": "m1-search-redaction",
            "search": {"query": {
                "provider": "ASF",
                "dataset": "token=fixture-secret",
                "platforms": ["https://example.invalid/?token=fixture-secret"],
                "start": "C:\\Users\\fixture-secret\\input",
                "max_results": 20
            }},
            "batch_id": "search-redaction",
            "created_at": "2026-08-16T00:00:00Z",
            "aoi_name": "redaction fixture",
            "aoi_geojson": null,
            "page_size": 24
        }))
        .expect("bounded request object");
        let error = application_sar_error(
            &request,
            ApplicationServiceError::InvalidRequest {
                reason: "password=fixture-secret https://example.invalid/?token=fixture-secret C:\\Users\\fixture-secret".to_owned(),
            },
        );
        let serialized = serde_json::to_string(&error).expect("ShellError serializes");

        assert_eq!(
            error.context.as_deref(),
            Some("invalid request reason redacted")
        );
        assert!(!serialized.contains("fixture-secret"));
        assert!(!serialized.contains("example.invalid"));
        assert!(!serialized.contains("C:\\\\Users"));
    }
}

/// Not archived sessions, pinned first; `include_archived` adds the rest.
#[tauri::command]
pub async fn agent_sessions_list(
    window: tauri::WebviewWindow,
    include_archived: Option<bool>,
) -> Result<Vec<insar_agent::session::SessionSummary>, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AgentHostState>()
            .host()?
            .sessions_list_with(include_archived.unwrap_or(false))
            .map_err(agent_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn agent_sessions_create(
    window: tauri::WebviewWindow,
    title: Option<String>,
    workspace_id: Option<String>,
) -> Result<insar_agent::session::SessionSummary, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AgentHostState>()
            .host()?
            .session_create_in(title, workspace_id)
            .map_err(agent_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn agent_sessions_delete(
    window: tauri::WebviewWindow,
    session_id: String,
) -> Result<(), ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AgentHostState>()
            .host()?
            .session_delete(&session_id)
            .map_err(agent_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn agent_sessions_rename(
    window: tauri::WebviewWindow,
    session_id: String,
    title: String,
) -> Result<insar_agent::session::SessionSummary, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AgentHostState>()
            .host()?
            .session_rename(&session_id, &title)
            .map_err(agent_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn agent_sessions_set_pinned(
    window: tauri::WebviewWindow,
    session_id: String,
    pinned: bool,
) -> Result<insar_agent::session::SessionSummary, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AgentHostState>()
            .host()?
            .session_set_pinned(&session_id, pinned)
            .map_err(agent_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn agent_sessions_set_archived(
    window: tauri::WebviewWindow,
    session_id: String,
    archived: bool,
) -> Result<insar_agent::session::SessionSummary, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AgentHostState>()
            .host()?
            .session_set_archived(&session_id, archived)
            .map_err(agent_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

/// Files the session under a workspace; null moves it back to "unfiled".
#[tauri::command]
pub async fn agent_sessions_move(
    window: tauri::WebviewWindow,
    session_id: String,
    workspace_id: Option<String>,
) -> Result<insar_agent::session::SessionSummary, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AgentHostState>()
            .host()?
            .session_move(&session_id, workspace_id.as_deref())
            .map_err(agent_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

/// Returns the file content; the desktop asks the user where to save it.
#[tauri::command]
pub async fn agent_sessions_export(
    window: tauri::WebviewWindow,
    session_id: String,
    format: insar_agent::session::SessionExportFormat,
) -> Result<insar_agent::session::SessionExport, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AgentHostState>()
            .host()?
            .session_export(&session_id, format)
            .map_err(agent_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

/// Writes the export into a folder the user chose on a dialog just before;
/// returns the new file's path.
#[tauri::command]
pub async fn agent_sessions_export_to(
    window: tauri::WebviewWindow,
    session_id: String,
    format: insar_agent::session::SessionExportFormat,
    directory: String,
) -> Result<String, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AgentHostState>()
            .host()?
            .session_export_to(&session_id, format, &directory)
            .map_err(agent_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn agent_workspaces_list(
    window: tauri::WebviewWindow,
) -> Result<Vec<insar_agent::session::Workspace>, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AgentHostState>()
            .host()?
            .workspaces_list()
            .map_err(agent_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn agent_workspaces_create(
    window: tauri::WebviewWindow,
    name: String,
    folder: Option<String>,
) -> Result<insar_agent::session::Workspace, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AgentHostState>()
            .host()?
            .workspace_create(&name, folder)
            .map_err(agent_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

/// Renames a workspace and/or rebinds its folder. `clear_folder` unbinds it.
#[tauri::command]
pub async fn agent_workspaces_update(
    window: tauri::WebviewWindow,
    workspace_id: String,
    name: Option<String>,
    folder: Option<String>,
    clear_folder: Option<bool>,
) -> Result<insar_agent::session::Workspace, ShellError> {
    validate_application_window(&window)?;
    let folder = if clear_folder.unwrap_or(false) {
        Some(None)
    } else {
        folder.map(Some)
    };
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AgentHostState>()
            .host()?
            .workspace_update(&workspace_id, name.as_deref(), folder)
            .map_err(agent_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

/// Deletes the workspace; its sessions return to "unfiled", files stay.
#[tauri::command]
pub async fn agent_workspaces_delete(
    window: tauri::WebviewWindow,
    workspace_id: String,
) -> Result<(), ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AgentHostState>()
            .host()?
            .workspace_delete(&workspace_id)
            .map_err(agent_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

/// Starts one run and returns its id at once; model and tool work continue on
/// a worker thread and surface through `agent_stream_poll`.
#[tauri::command]
pub async fn agent_send(
    window: tauri::WebviewWindow,
    session_id: String,
    text: String,
    context: Option<insar_agent::session::SendContext>,
) -> Result<String, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AgentHostState>()
            .host()?
            .send(&session_id, &text, context.unwrap_or_default())
            .map_err(agent_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn agent_stream_poll(
    window: tauri::WebviewWindow,
    session_id: String,
    cursor: u64,
) -> Result<insar_agent::session::StreamPage, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AgentHostState>()
            .host()?
            .stream_poll(&session_id, cursor)
            .map_err(agent_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

/// Desktop confirm card only: `snapshot` must equal the card the host showed.
/// `output_dir` is a directory the user chose on a create_task card; the
/// plan is rewritten to write there before it is approved.
#[tauri::command]
pub async fn agent_plan_approve(
    window: tauri::WebviewWindow,
    plan_id: String,
    snapshot: serde_json::Value,
    output_dir: Option<String>,
) -> Result<(), ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AgentHostState>()
            .host()?
            .plan_approve(&plan_id, snapshot, output_dir)
            .map_err(agent_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn agent_plan_reject(
    window: tauri::WebviewWindow,
    plan_id: String,
) -> Result<(), ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AgentHostState>()
            .host()?
            .plan_reject(&plan_id)
            .map_err(agent_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn agent_run_cancel(
    window: tauri::WebviewWindow,
    run_id: String,
) -> Result<bool, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AgentHostState>()
            .host()?
            .run_cancel(&run_id)
            .map_err(agent_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn agent_runs_list(
    window: tauri::WebviewWindow,
    session_id: String,
) -> Result<Vec<insar_agent::session::RunSummary>, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AgentHostState>()
            .host()?
            .runs_list(&session_id)
            .map_err(agent_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

/// The session's runs with their request, plans and created download tasks.
#[tauri::command]
pub async fn agent_session_work(
    window: tauri::WebviewWindow,
    session_id: String,
) -> Result<Vec<insar_agent::session::WorkItem>, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AgentHostState>()
            .host()?
            .session_work(&session_id)
            .map_err(agent_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

/// Truncates a session stream before `before_seq` (for turn edit/revert) and optionally restores pre-turn AOI.
#[tauri::command]
pub async fn agent_session_truncate(
    window: tauri::WebviewWindow,
    session_id: String,
    before_seq: u64,
    restore_aoi: Option<bool>,
) -> Result<crate::agent::SessionTruncateResult, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let out = app
            .state::<AgentHostState>()
            .host()?
            .session_truncate(&session_id, before_seq, restore_aoi.unwrap_or(true))
            .map_err(agent_error)?;
        if out.aoi_restored {
            crate::emit_host_event(
                &app,
                "workbench:aoi-changed",
                serde_json::json!({
                    "session_id": session_id,
                    "updated_by": "agent_revert",
                }),
            );
        }
        Ok(out)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

/// Reads per-session isolated workspace state (`<session_dir>/workspace_state.json`).
#[tauri::command]
pub async fn agent_session_workspace_state(
    window: tauri::WebviewWindow,
    session_id: String,
) -> Result<crate::agent::SessionWorkspaceState, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AgentHostState>()
            .host()?
            .session_workspace_state(&session_id)
            .map_err(agent_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

/// Updates per-session working AOI / active search batch without mutating global Map Workbench AOI.
#[tauri::command]
pub async fn agent_session_set_working_aoi(
    window: tauri::WebviewWindow,
    session_id: String,
    request: Option<ApplicationSetWorkingAoiRequest>,
    clear: Option<bool>,
    active_batch_id: Option<String>,
    active_resource_id: Option<String>,
    active_batch_revision: Option<u64>,
) -> Result<crate::agent::SessionWorkspaceState, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    let req_value = request.map(serde_json::to_value).transpose().map_err(|e| {
        ShellError::with_code(format!("INVALID_REQUEST: {e}"), "agent.invalid_request")
    })?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AgentHostState>()
            .host()?
            .session_set_working_aoi(
                &session_id,
                req_value,
                clear,
                active_batch_id,
                active_resource_id,
                active_batch_revision,
            )
            .map_err(agent_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn agent_runs_open(
    window: tauri::WebviewWindow,
    run_id: String,
) -> Result<insar_agent::session::RunDetail, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AgentHostState>()
            .host()?
            .run_open(&run_id)
            .map_err(agent_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

/// Channel metadata and `key_ready` only; a saved key is never returned.
#[tauri::command]
pub async fn agent_channels_get(
    window: tauri::WebviewWindow,
) -> Result<Vec<crate::agent::AgentChannelView>, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AgentHostState>()
            .channels_get()
            .map_err(agent_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn agent_disabled_items_get(
    window: tauri::WebviewWindow,
) -> Result<crate::agent::AgentDisabledItems, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AgentHostState>()
            .disabled_items_get()
            .map_err(agent_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn agent_disabled_items_set(
    window: tauri::WebviewWindow,
    items: crate::agent::AgentDisabledItems,
) -> Result<crate::agent::AgentDisabledItems, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AgentHostState>()
            .disabled_items_set(items)
            .map_err(agent_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn agent_channels_save(
    window: tauri::WebviewWindow,
    channel: crate::agent::AgentChannelSave,
) -> Result<Vec<crate::agent::AgentChannelView>, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AgentHostState>()
            .channels_save(channel)
            .map_err(agent_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn agent_channels_test(
    window: tauri::WebviewWindow,
    channel_id: String,
) -> Result<crate::agent::AgentChannelTest, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AgentHostState>()
            .channels_test(&channel_id)
            .map_err(agent_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn agent_channels_models(
    window: tauri::WebviewWindow,
    channel_id: String,
) -> Result<Vec<String>, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AgentHostState>()
            .channels_models(&channel_id)
            .map_err(agent_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn agent_channels_probe_draft(
    window: tauri::WebviewWindow,
    draft: crate::agent::AgentChannelDraft,
) -> Result<crate::agent::AgentChannelProbe, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AgentHostState>()
            .channels_probe_draft(draft)
            .map_err(agent_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn agent_channels_test_draft(
    window: tauri::WebviewWindow,
    draft: crate::agent::AgentChannelDraft,
) -> Result<crate::agent::AgentChannelTest, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AgentHostState>()
            .channels_test_draft(draft)
            .map_err(agent_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

/// Sets the current channel used by new runs; returns the refreshed list.
#[tauri::command]
pub async fn agent_channels_set_default(
    window: tauri::WebviewWindow,
    channel_id: String,
) -> Result<Vec<crate::agent::AgentChannelView>, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AgentHostState>()
            .channels_set_default(&channel_id)
            .map_err(agent_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

/// Deletes a channel: its saved key (matched in the system credential store
/// by target and user name; the value is not decoded or returned), its
/// metadata entry, and a default pointer to it; returns the refreshed list.
#[tauri::command]
pub async fn agent_channels_delete(
    window: tauri::WebviewWindow,
    channel_id: String,
) -> Result<Vec<crate::agent::AgentChannelView>, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AgentHostState>()
            .channels_delete(&channel_id)
            .map_err(agent_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

/// Atomically renames a channel in-process, migrating any saved credential to
/// `new_id` before updating `agent-channels.json` and deleting `old_id`'s key.
#[tauri::command]
pub async fn agent_channel_rename(
    window: tauri::WebviewWindow,
    old_id: String,
    new_id: String,
    api_key: Option<String>,
) -> Result<Vec<crate::agent::AgentChannelView>, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AgentHostState>()
            .channels_rename(&old_id, &new_id, api_key)
            .map_err(agent_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn agent_sync_public_model_registry(
    window: tauri::WebviewWindow,
) -> Result<crate::oauth_login::PublicModelRegistrySyncResult, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let application = app.state::<AgentHostState>().application().clone();
        crate::oauth_login::sync_public_model_registry(&application).map_err(|msg| {
            ShellError::application_service_with_unavailable_context(
                "AGENT_MODEL_REGISTRY_SYNC_FAILED",
                msg,
            )
        })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn agent_oauth_open_browser(
    window: tauri::WebviewWindow,
    url: String,
) -> Result<OpenExternalUrlAck, ShellError> {
    application_open_external_url(window, url)
}

#[tauri::command]
pub async fn agent_oauth_device_start(
    window: tauri::WebviewWindow,
    provider_id: String,
) -> Result<crate::oauth_login::OAuthDeviceStartResult, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let application = app.state::<AgentHostState>().application().clone();
        crate::oauth_login::oauth_device_start(&application, &provider_id).map_err(|msg| {
            ShellError::application_service_with_context("AGENT_OAUTH_DEVICE_START_FAILED", msg)
        })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn agent_oauth_device_poll(
    window: tauri::WebviewWindow,
    session_id: String,
) -> Result<crate::oauth_login::OAuthDevicePollResult, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let application = app.state::<AgentHostState>().application().clone();
        crate::oauth_login::oauth_device_poll(&application, &session_id).map_err(|msg| {
            ShellError::application_service_with_context("AGENT_OAUTH_DEVICE_POLL_FAILED", msg)
        })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn agent_oauth_pkce_start(
    window: tauri::WebviewWindow,
    provider_id: String,
) -> Result<crate::oauth_login::OAuthPkceStartResult, ShellError> {
    validate_application_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        crate::oauth_login::oauth_pkce_start(&provider_id).map_err(|msg| {
            ShellError::application_service_with_context("AGENT_OAUTH_PKCE_START_FAILED", msg)
        })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn agent_oauth_pkce_complete(
    window: tauri::WebviewWindow,
    session_id: String,
    callback_input: Option<String>,
) -> Result<crate::oauth_login::OAuthPkceCompleteResult, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let application = app.state::<AgentHostState>().application().clone();
        crate::oauth_login::oauth_pkce_poll_or_exchange(
            &application,
            &session_id,
            callback_input.as_deref(),
        )
        .map_err(|msg| {
            ShellError::application_service_with_context("AGENT_OAUTH_PKCE_COMPLETE_FAILED", msg)
        })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn agent_oauth_channels_meta(
    window: tauri::WebviewWindow,
) -> Result<Vec<crate::oauth_login::ChannelAuthMeta>, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let application = app.state::<AgentHostState>().application().clone();
        crate::oauth_login::inspect_channels_auth_meta(&application).map_err(|msg| {
            ShellError::application_service_with_context("AGENT_OAUTH_CHANNELS_META_FAILED", msg)
        })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn agent_oauth_channel_quota(
    window: tauri::WebviewWindow,
    channel_id: String,
) -> Result<crate::oauth_login::SubscriptionChannelQuota, ShellError> {
    validate_application_window(&window)?;
    let trimmed_id = channel_id.trim().to_owned();
    let has_channel = window
        .state::<AgentHostState>()
        .has_saved_channel(&trimmed_id)
        .map_err(agent_error)?;
    if !has_channel {
        return Err(ShellError::application_service_with_context(
            "AGENT_OAUTH_QUOTA_INVALID_CHANNEL",
            "目标模型渠道不存在".to_owned(),
        ));
    }
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let application = app.state::<AgentHostState>().application().clone();
        crate::oauth_login::fetch_subscription_channel_quota(&application, &trimmed_id).map_err(
            |msg| ShellError::application_service_with_context("AGENT_OAUTH_QUOTA_FAILED", msg),
        )
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

/// Permission mode the next decision in `session_id` (or, without one, a new
/// session) uses, the session's own setting and the global default.
#[tauri::command]
pub async fn agent_permission_mode_get(
    window: tauri::WebviewWindow,
    session_id: Option<String>,
) -> Result<insar_agent::permission::PermissionModeView, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AgentHostState>()
            .host()?
            .permission_mode_get(session_id.as_deref())
            .map_err(agent_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

/// Sets the global default mode; only the desktop window can call this.
#[tauri::command]
pub async fn agent_permission_mode_set_global(
    window: tauri::WebviewWindow,
    mode: insar_agent::permission::PermissionMode,
) -> Result<insar_agent::permission::PermissionModeView, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AgentHostState>()
            .host()?
            .permission_mode_set_global(mode)
            .map_err(agent_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

/// Sets (or, with `mode: null`, clears) one session's own mode.
#[tauri::command]
pub async fn agent_permission_mode_set_session(
    window: tauri::WebviewWindow,
    session_id: String,
    mode: Option<insar_agent::permission::PermissionMode>,
) -> Result<insar_agent::permission::PermissionModeView, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AgentHostState>()
            .host()?
            .permission_mode_set_session(&session_id, mode)
            .map_err(agent_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

/// Cancels every in-progress Agent run and every active task an Agent run
/// created, through the existing run and task cancel paths.
#[tauri::command]
pub async fn agent_stop_all(
    window: tauri::WebviewWindow,
) -> Result<insar_agent::permission::StopAllResult, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AgentHostState>()
            .host()?
            .stop_all()
            .map_err(agent_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

/// Live Agent runs and active Agent-created tasks across sessions (the state
/// of the stop-everything button); changes are also announced with the
/// `activity_changed` stream event.
#[tauri::command]
pub async fn agent_activity_status(
    window: tauri::WebviewWindow,
) -> Result<insar_agent::session::ActivityStatus, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AgentHostState>()
            .host()?
            .activity_status()
            .map_err(agent_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn eostage_account_status(
    window: tauri::WebviewWindow,
    refresh: Option<bool>,
) -> Result<crate::eostage_account::EOStageAccountStatus, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let application = app.state::<AgentHostState>().application().clone();
        crate::eostage_account::account_status(&application, refresh.unwrap_or(false)).map_err(
            |msg| ShellError::application_service_with_context("EOSTAGE_ACCOUNT_FAILED", msg),
        )
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn eostage_account_login_start(
    window: tauri::WebviewWindow,
) -> Result<crate::eostage_account::EOStageLoginStartResult, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let application = app.state::<AgentHostState>().application().clone();
        crate::eostage_account::login_start(&application).map_err(|msg| {
            ShellError::application_service_with_context("EOSTAGE_LOGIN_START_FAILED", msg)
        })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn eostage_account_login_poll(
    window: tauri::WebviewWindow,
    session_id: String,
) -> Result<crate::eostage_account::EOStageLoginPollResult, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let application = app.state::<AgentHostState>().application().clone();
        crate::eostage_account::login_poll(&application, &session_id).map_err(|msg| {
            ShellError::application_service_with_context("EOSTAGE_LOGIN_POLL_FAILED", msg)
        })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn eostage_account_login_complete_code(
    window: tauri::WebviewWindow,
    code: String,
) -> Result<crate::eostage_account::EOStageAccountStatus, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let application = app.state::<AgentHostState>().application().clone();
        crate::eostage_account::login_complete_code(&application, &code).map_err(|msg| {
            ShellError::application_service_with_context("EOSTAGE_LOGIN_COMPLETE_FAILED", msg)
        })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn eostage_account_logout(
    window: tauri::WebviewWindow,
) -> Result<crate::eostage_account::EOStageAccountStatus, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let application = app.state::<AgentHostState>().application().clone();
        crate::eostage_account::account_logout(&application).map_err(|msg| {
            ShellError::application_service_with_context("EOSTAGE_LOGOUT_FAILED", msg)
        })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn eostage_account_sync_models(
    window: tauri::WebviewWindow,
) -> Result<Vec<crate::agent::AgentChannelView>, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let host_state = app.state::<AgentHostState>();
        let application = host_state.application().clone();
        crate::eostage_account::sync_official_models(&application, &host_state).map_err(|msg| {
            ShellError::application_service_with_context("EOSTAGE_MODELS_SYNC_FAILED", msg)
        })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn eostage_billing_config(
    window: tauri::WebviewWindow,
) -> Result<crate::eostage_account::EOStageBillingConfig, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let application = app.state::<AgentHostState>().application().clone();
        crate::eostage_account::billing_config(&application).map_err(|msg| {
            ShellError::application_service_with_context("EOSTAGE_BILLING_CONFIG_FAILED", msg)
        })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn eostage_billing_redeem(
    window: tauri::WebviewWindow,
    code: String,
) -> Result<crate::eostage_account::EOStageRedeemResult, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let application = app.state::<AgentHostState>().application().clone();
        crate::eostage_account::billing_redeem(&application, &code).map_err(|msg| {
            ShellError::application_service_with_context("EOSTAGE_REDEEM_FAILED", msg)
        })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn eostage_billing_orders_list(
    window: tauri::WebviewWindow,
) -> Result<Vec<crate::eostage_account::EOStageOrder>, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let application = app.state::<AgentHostState>().application().clone();
        crate::eostage_account::billing_orders_list(&application).map_err(|msg| {
            ShellError::application_service_with_context("EOSTAGE_ORDERS_LIST_FAILED", msg)
        })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn eostage_billing_order_create(
    window: tauri::WebviewWindow,
    plan_id: String,
) -> Result<crate::eostage_account::EOStageOrder, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let application = app.state::<AgentHostState>().application().clone();
        crate::eostage_account::billing_order_create(&application, &plan_id).map_err(|msg| {
            ShellError::application_service_with_context("EOSTAGE_ORDER_CREATE_FAILED", msg)
        })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn eostage_billing_order_check(
    window: tauri::WebviewWindow,
    order_no: String,
) -> Result<crate::eostage_account::EOStageOrder, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let application = app.state::<AgentHostState>().application().clone();
        crate::eostage_account::billing_order_action(&application, &order_no, false).map_err(
            |msg| ShellError::application_service_with_context("EOSTAGE_ORDER_CHECK_FAILED", msg),
        )
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn eostage_billing_order_cancel(
    window: tauri::WebviewWindow,
    order_no: String,
) -> Result<crate::eostage_account::EOStageOrder, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let application = app.state::<AgentHostState>().application().clone();
        crate::eostage_account::billing_order_action(&application, &order_no, true).map_err(|msg| {
            ShellError::application_service_with_context("EOSTAGE_ORDER_CANCEL_FAILED", msg)
        })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn eostage_usage_list(
    window: tauri::WebviewWindow,
    limit: Option<u32>,
) -> Result<crate::eostage_account::EOStageUsageListResult, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let application = app.state::<AgentHostState>().application().clone();
        crate::eostage_account::usage_list(&application, limit).map_err(|msg| {
            ShellError::application_service_with_context("EOSTAGE_USAGE_LIST_FAILED", msg)
        })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn desktop_onboarding_state_get(
    window: tauri::WebviewWindow,
) -> Result<crate::desktop_ui_prefs::DesktopOnboardingState, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ApplicationServiceState>();
        let app_data = crate::application::resolve_app_data_dir(&state);
        let has_existing_tasks = state
            .task_overview()
            .map(|ov| !ov.queue.is_empty() || !ov.history_page.items.is_empty())
            .unwrap_or(false);
        Ok(crate::desktop_ui_prefs::get_desktop_onboarding_state(
            &app_data,
            has_existing_tasks,
        ))
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn desktop_onboarding_state_set(
    window: tauri::WebviewWindow,
    completed: bool,
) -> Result<crate::desktop_ui_prefs::DesktopOnboardingState, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ApplicationServiceState>();
        let app_data = crate::application::resolve_app_data_dir(&state);
        crate::desktop_ui_prefs::set_desktop_onboarding_completed(&app_data, completed).map_err(
            |msg| ShellError::application_service_with_context("DESKTOP_UI_PREFS_SAVE_FAILED", msg),
        )
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn desktop_active_resource_get(
    window: tauri::WebviewWindow,
) -> Result<Option<String>, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ApplicationServiceState>();
        let app_data = crate::application::resolve_app_data_dir(&state);
        Ok(crate::desktop_ui_prefs::get_active_resource_id(&app_data))
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn desktop_active_resource_set(
    window: tauri::WebviewWindow,
    resource_id: String,
) -> Result<Option<String>, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ApplicationServiceState>();
        let app_data = crate::application::resolve_app_data_dir(&state);
        let has_existing_tasks = state
            .task_overview()
            .map(|ov| !ov.queue.is_empty() || !ov.history_page.items.is_empty())
            .unwrap_or(false);
        crate::desktop_ui_prefs::set_active_resource_id(&app_data, &resource_id, has_existing_tasks)
            .map_err(|msg| {
                ShellError::application_service_with_context("DESKTOP_UI_PREFS_SAVE_FAILED", msg)
            })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn shell_diagnostics_get_state(
    window: tauri::WebviewWindow,
) -> Result<crate::diagnostics::DiagnosticsStateView, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ApplicationServiceState>();
        let app_data = crate::application::resolve_app_data_dir(&state);
        Ok(crate::diagnostics::diagnostics_state(&app_data))
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn shell_diagnostics_set_enabled(
    window: tauri::WebviewWindow,
    enabled: bool,
) -> Result<crate::diagnostics::DiagnosticsStateView, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ApplicationServiceState>();
        let app_data = crate::application::resolve_app_data_dir(&state);
        crate::diagnostics::set_diagnostics_enabled_state(&app_data, enabled).map_err(|msg| {
            ShellError::application_service_with_context("DIAGNOSTICS_PREFS_SAVE_FAILED", msg)
        })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn shell_diagnostics_record(
    window: tauri::WebviewWindow,
    input: crate::diagnostics::DiagnosticsRecordInput,
) -> Result<crate::diagnostics::SanitizedErrorReport, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    let app_version = app.package_info().version.to_string();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ApplicationServiceState>();
        let app_data = crate::application::resolve_app_data_dir(&state);
        let application = app.state::<AgentHostState>().application().clone();
        crate::diagnostics::record_diagnostic_and_maybe_upload(
            &app_data,
            &app_version,
            input,
            Some(application),
        )
        .map_err(|msg| {
            ShellError::application_service_with_context("DIAGNOSTICS_RECORD_FAILED", msg)
        })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn shell_diagnostics_list(
    window: tauri::WebviewWindow,
) -> Result<Vec<crate::diagnostics::SanitizedErrorReport>, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ApplicationServiceState>();
        let app_data = crate::application::resolve_app_data_dir(&state);
        Ok(crate::diagnostics::list_diagnostics(&app_data))
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn shell_diagnostics_clear(
    window: tauri::WebviewWindow,
) -> Result<crate::diagnostics::DiagnosticsStateView, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ApplicationServiceState>();
        let app_data = crate::application::resolve_app_data_dir(&state);
        crate::diagnostics::clear_diagnostics(&app_data).map_err(|msg| {
            ShellError::application_service_with_context("DIAGNOSTICS_CLEAR_FAILED", msg)
        })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn shell_diagnostics_upload(
    window: tauri::WebviewWindow,
) -> Result<crate::diagnostics::DiagnosticsUploadOutcome, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ApplicationServiceState>();
        let app_data = crate::application::resolve_app_data_dir(&state);
        let application = app.state::<AgentHostState>().application().clone();
        crate::diagnostics::upload_pending_diagnostics(&app_data, &application).map_err(|msg| {
            ShellError::application_service_with_context("DIAGNOSTICS_UPLOAD_FAILED", msg)
        })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn shell_check_app_update(
    window: tauri::WebviewWindow,
) -> Result<crate::app_update::AppUpdateCheckResult, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    let current_version = app.package_info().version.to_string();
    tauri::async_runtime::spawn_blocking(move || {
        let application = app.state::<AgentHostState>().application().clone();
        Ok(crate::app_update::check_latest_release(
            &application,
            &current_version,
        ))
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_runtime_components_inspect(
    window: tauri::WebviewWindow,
) -> Result<crate::application::RuntimeComponentsReport, ShellError> {
    validate_application_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        crate::application::inspect_runtime_components()
            .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_extension_registry_fetch(
    window: tauri::WebviewWindow,
    registry_url: Option<String>,
) -> Result<crate::application::ExtensionRegistryReport, ShellError> {
    validate_application_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        crate::application::fetch_extension_registry(registry_url).map_err(|err| {
            ShellError::application_service_with_context("V3_EXTENSION_REGISTRY_FAILED", err)
        })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_runtime_component_install(
    window: tauri::WebviewWindow,
    request: crate::application::RuntimeComponentInstallRequest,
) -> Result<crate::application::RuntimeComponentInstallResult, ShellError> {
    validate_application_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        crate::application::install_runtime_component(&request).map_err(|err| {
            ShellError::application_service_with_context("V3_RUNTIME_COMPONENT_INSTALL_FAILED", err)
        })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_plugins_status(
    window: tauri::WebviewWindow,
) -> Result<crate::application::ApplicationPluginsStatusResponse, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ApplicationServiceState>();
        let app_data = crate::application::resolve_app_data_dir(&state);
        let rt_plugins = crate::application::discover_runtime_plugins_dir();
        crate::application::inspect_plugins_status_at(&app_data, &rt_plugins).map_err(|err| {
            ShellError::application_service_with_context("V3_PLUGINS_STATUS_FAILED", err)
        })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn plugin_list(
    window: tauri::WebviewWindow,
) -> Result<crate::application::ApplicationPluginsStatusResponse, ShellError> {
    application_plugins_status(window).await
}

#[tauri::command]
pub async fn application_plugin_install(
    window: tauri::WebviewWindow,
    request: crate::application::PluginInstallRequest,
) -> Result<crate::application::PluginMutationResponse, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ApplicationServiceState>();
        let app_data = crate::application::resolve_app_data_dir(&state);
        let rt_plugins = crate::application::discover_runtime_plugins_dir();
        crate::application::install_plugin_package_at(&app_data, &rt_plugins, &request).map_err(
            |err| ShellError::application_service_with_context("V3_PLUGIN_INSTALL_FAILED", err),
        )
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_plugin_set_enabled(
    window: tauri::WebviewWindow,
    plugin_id: String,
    enabled: bool,
) -> Result<crate::application::PluginMutationResponse, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ApplicationServiceState>();
        let app_data = crate::application::resolve_app_data_dir(&state);
        let rt_plugins = crate::application::discover_runtime_plugins_dir();
        crate::application::set_plugin_enabled_at(&app_data, &rt_plugins, &plugin_id, enabled)
            .map_err(|err| {
                ShellError::application_service_with_context("V3_PLUGIN_TOGGLE_FAILED", err)
            })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_plugin_uninstall(
    window: tauri::WebviewWindow,
    plugin_id: String,
) -> Result<crate::application::PluginMutationResponse, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ApplicationServiceState>();
        let app_data = crate::application::resolve_app_data_dir(&state);
        let rt_plugins = crate::application::discover_runtime_plugins_dir();
        crate::application::uninstall_plugin_package_at(&app_data, &rt_plugins, &plugin_id).map_err(
            |err| ShellError::application_service_with_context("V3_PLUGIN_UNINSTALL_FAILED", err),
        )
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_custom_connector_save(
    window: tauri::WebviewWindow,
    connector: crate::application::CustomConnectorRecord,
) -> Result<crate::application::ApplicationPluginsStatusResponse, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ApplicationServiceState>();
        let app_data = crate::application::resolve_app_data_dir(&state);
        let rt_plugins = crate::application::discover_runtime_plugins_dir();
        let mut items = crate::application::load_custom_connectors_at(&app_data);
        if let Some(existing) = items.iter_mut().find(|c| c.id == connector.id) {
            *existing = connector;
        } else {
            items.insert(0, connector);
        }
        crate::application::save_custom_connectors_at(&app_data, &items).map_err(|err| {
            ShellError::application_service_with_context("V3_CONNECTOR_SAVE_FAILED", err)
        })?;
        crate::mcp_client::invalidate_connector_cache(None);
        crate::application::inspect_plugins_status_at(&app_data, &rt_plugins).map_err(|err| {
            ShellError::application_service_with_context("V3_PLUGINS_STATUS_FAILED", err)
        })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_custom_connector_delete(
    window: tauri::WebviewWindow,
    connector_id: String,
) -> Result<crate::application::ApplicationPluginsStatusResponse, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ApplicationServiceState>();
        let app_data = crate::application::resolve_app_data_dir(&state);
        let rt_plugins = crate::application::discover_runtime_plugins_dir();
        let items: Vec<_> = crate::application::load_custom_connectors_at(&app_data)
            .into_iter()
            .filter(|c| c.id != connector_id)
            .collect();
        crate::application::save_custom_connectors_at(&app_data, &items).map_err(|err| {
            ShellError::application_service_with_context("V3_CONNECTOR_DELETE_FAILED", err)
        })?;
        crate::mcp_client::invalidate_connector_cache(Some(&connector_id));
        crate::application::inspect_plugins_status_at(&app_data, &rt_plugins).map_err(|err| {
            ShellError::application_service_with_context("V3_PLUGINS_STATUS_FAILED", err)
        })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_custom_connector_probe_tools(
    window: tauri::WebviewWindow,
    connector_id: String,
) -> Result<crate::mcp_client::McpProbeResult, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ApplicationServiceState>();
        let app_data = crate::application::resolve_app_data_dir(&state);
        let items = crate::application::load_custom_connectors_at(&app_data);
        let Some(connector) = items.into_iter().find(|c| c.id == connector_id) else {
            return Err(ShellError::application_service_with_context(
                "V3_CONNECTOR_NOT_FOUND",
                format!("未找到连接器 '{connector_id}'"),
            ));
        };
        crate::mcp_client::invalidate_connector_cache(Some(&connector.id));
        Ok(crate::mcp_client::probe_connector_tools(&connector))
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_custom_skill_save(
    window: tauri::WebviewWindow,
    skill: crate::application::CustomSkillRecord,
) -> Result<crate::application::ApplicationPluginsStatusResponse, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ApplicationServiceState>();
        let app_data = crate::application::resolve_app_data_dir(&state);
        let rt_plugins = crate::application::discover_runtime_plugins_dir();
        crate::application::save_custom_skill_at(&app_data, &skill).map_err(|err| {
            ShellError::application_service_with_context("V3_SKILL_SAVE_FAILED", err)
        })?;
        crate::application::inspect_plugins_status_at(&app_data, &rt_plugins).map_err(|err| {
            ShellError::application_service_with_context("V3_PLUGINS_STATUS_FAILED", err)
        })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_custom_skill_delete(
    window: tauri::WebviewWindow,
    skill_id: String,
) -> Result<crate::application::ApplicationPluginsStatusResponse, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ApplicationServiceState>();
        let app_data = crate::application::resolve_app_data_dir(&state);
        let rt_plugins = crate::application::discover_runtime_plugins_dir();
        crate::application::delete_custom_skill_at(&app_data, &skill_id).map_err(|err| {
            ShellError::application_service_with_context("V3_SKILL_DELETE_FAILED", err)
        })?;
        crate::application::inspect_plugins_status_at(&app_data, &rt_plugins).map_err(|err| {
            ShellError::application_service_with_context("V3_PLUGINS_STATUS_FAILED", err)
        })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_install_custom_skill_from_url(
    window: tauri::WebviewWindow,
    url: String,
    fallback_name: Option<String>,
) -> Result<crate::application::ApplicationPluginsStatusResponse, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ApplicationServiceState>();
        let app_data = crate::application::resolve_app_data_dir(&state);
        let rt_plugins = crate::application::discover_runtime_plugins_dir();
        let proxy_policy = state.ordinary_proxy_policy();
        crate::application::install_custom_skill_from_url_at(
            &app_data,
            proxy_policy,
            &url,
            fallback_name.as_deref(),
        )
        .map_err(|err| {
            ShellError::application_service_with_context("V3_SKILL_URL_INSTALL_FAILED", err)
        })?;
        crate::application::inspect_plugins_status_at(&app_data, &rt_plugins).map_err(|err| {
            ShellError::application_service_with_context("V3_PLUGINS_STATUS_FAILED", err)
        })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_install_custom_skill_from_folder(
    window: tauri::WebviewWindow,
    folder_path: String,
) -> Result<crate::application::ApplicationPluginsStatusResponse, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ApplicationServiceState>();
        let app_data = crate::application::resolve_app_data_dir(&state);
        let rt_plugins = crate::application::discover_runtime_plugins_dir();
        crate::application::import_custom_skill_folder_at(&app_data, &folder_path).map_err(
            |err| {
                ShellError::application_service_with_context("V3_SKILL_FOLDER_IMPORT_FAILED", err)
            },
        )?;
        crate::application::inspect_plugins_status_at(&app_data, &rt_plugins).map_err(|err| {
            ShellError::application_service_with_context("V3_PLUGINS_STATUS_FAILED", err)
        })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_usage_statistics(
    window: tauri::WebviewWindow,
) -> Result<crate::application::ApplicationUsageStatisticsResponse, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ApplicationServiceState>();
        crate::application::compute_application_usage_statistics(&state).map_err(|err| {
            ShellError::application_service_with_context("V3_USAGE_STATISTICS_FAILED", err)
        })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn get_cache_summary(
    window: tauri::WebviewWindow,
) -> Result<crate::application::CacheSummaryResponse, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ApplicationServiceState>();
        crate::application::compute_cache_summary(&state).map_err(|err| {
            ShellError::application_service_with_context("V3_CACHE_SUMMARY_FAILED", err)
        })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn clear_disposable_caches(
    window: tauri::WebviewWindow,
    categories: Option<Vec<String>>,
) -> Result<crate::application::ClearCachesResponse, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ApplicationServiceState>();
        crate::application::clear_disposable_caches(&state, categories).map_err(|err| {
            ShellError::application_service_with_context("V3_CACHE_CLEAR_FAILED", err)
        })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_gis_raster_inspect(
    window: tauri::WebviewWindow,
    request: crate::geodata_tools::GisRasterInspectRequest,
) -> Result<serde_json::Value, ShellError> {
    validate_application_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        crate::geodata_tools::execute_gis_raster_inspect(&request).map_err(|err| {
            ShellError::application_service_with_context("V3_GIS_RASTER_INSPECT_FAILED", err)
        })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_gis_raster_pixel_probe(
    window: tauri::WebviewWindow,
    request: insar_gis::RasterPixelProbeRequest,
) -> Result<serde_json::Value, ShellError> {
    validate_application_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        crate::geodata_tools::execute_gis_raster_pixel_probe(&request).map_err(|err| {
            ShellError::application_service_with_context("V3_GIS_RASTER_PIXEL_PROBE_FAILED", err)
        })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_gis_raster_process(
    window: tauri::WebviewWindow,
    request: insar_gis::RasterProcessRequest,
) -> Result<serde_json::Value, ShellError> {
    validate_application_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        crate::geodata_tools::execute_gis_raster_process(&request).map_err(|err| {
            ShellError::application_service_with_context("V3_GIS_RASTER_PROCESS_FAILED", err)
        })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_gis_vector_process(
    window: tauri::WebviewWindow,
    request: insar_gis::VectorProcessRequest,
) -> Result<serde_json::Value, ShellError> {
    validate_application_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        crate::geodata_tools::execute_gis_vector_process(&request).map_err(|err| {
            ShellError::application_service_with_context("V3_GIS_VECTOR_PROCESS_FAILED", err)
        })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_external_gis_inspect(
    window: tauri::WebviewWindow,
    request: Option<crate::external_gis_bridge::ExternalGisInspectRequest>,
) -> Result<crate::external_gis_bridge::ExternalGisInspectReport, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    let probe_versions = request.map(|r| r.probe_versions).unwrap_or(false);
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ApplicationServiceState>();
        let app_data = crate::application::resolve_app_data_dir(&state);
        let rt_root = crate::application::discover_runtime_root();
        crate::external_gis_bridge::inspect_external_gis_toolchains(
            &rt_root,
            &app_data,
            probe_versions,
        )
        .map_err(|err| {
            ShellError::application_service_with_context("V3_EXTERNAL_GIS_INSPECT_FAILED", err)
        })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_external_gis_save_prefs(
    window: tauri::WebviewWindow,
    prefs: crate::external_gis_bridge::ExternalGisPrefs,
) -> Result<crate::external_gis_bridge::ExternalGisInspectReport, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ApplicationServiceState>();
        let app_data = crate::application::resolve_app_data_dir(&state);
        let rt_root = crate::application::discover_runtime_root();
        crate::external_gis_bridge::save_external_gis_prefs(&app_data, &prefs).map_err(|err| {
            ShellError::application_service_with_context("V3_EXTERNAL_GIS_SAVE_PREFS_FAILED", err)
        })?;
        crate::external_gis_bridge::inspect_external_gis_toolchains(&rt_root, &app_data, true)
            .map_err(|err| {
                ShellError::application_service_with_context("V3_EXTERNAL_GIS_INSPECT_FAILED", err)
            })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_external_gis_execute(
    window: tauri::WebviewWindow,
    request: crate::external_gis_bridge::ExternalGisExecuteRequest,
) -> Result<crate::external_gis_bridge::ExternalGisExecuteResult, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ApplicationServiceState>();
        let app_data = crate::application::resolve_app_data_dir(&state);
        let rt_root = crate::application::discover_runtime_root();
        crate::external_gis_bridge::execute_external_gis_action(&rt_root, &app_data, &request)
            .map_err(|err| {
                ShellError::application_service_with_context("V3_EXTERNAL_GIS_EXECUTE_FAILED", err)
            })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_conda_install_geo_package(
    window: tauri::WebviewWindow,
    request: crate::external_gis_bridge::CondaGeoInstallRequest,
) -> Result<crate::external_gis_bridge::CondaGeoInstallResult, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ApplicationServiceState>();
        let app_data = crate::application::resolve_app_data_dir(&state);
        let rt_root = crate::application::discover_runtime_root();
        crate::external_gis_bridge::install_conda_geo_package(&rt_root, &app_data, &request)
            .map_err(|err| {
                ShellError::application_service_with_context("V3_CONDA_INSTALL_FAILED", err)
            })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_lbs_coord_transform(
    window: tauri::WebviewWindow,
    request: crate::geodata_tools::LbsCoordTransformRequest,
) -> Result<serde_json::Value, ShellError> {
    validate_application_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        crate::geodata_tools::execute_lbs_coord_transform(&request).map_err(|err| {
            ShellError::application_service_with_context("V3_LBS_COORD_TRANSFORM_FAILED", err)
        })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_mapgis_inspect(
    window: tauri::WebviewWindow,
    request: crate::geodata_tools::MapGisInspectRequest,
) -> Result<serde_json::Value, ShellError> {
    validate_application_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        crate::geodata_tools::execute_mapgis_inspect(&request).map_err(|err| {
            ShellError::application_service_with_context("V3_MAPGIS_INSPECT_FAILED", err)
        })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_mapgis_convert(
    window: tauri::WebviewWindow,
    request: crate::geodata_tools::MapGisConvertRequest,
) -> Result<serde_json::Value, ShellError> {
    validate_application_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        crate::geodata_tools::execute_mapgis_convert(&request).map_err(|err| {
            ShellError::application_service_with_context("V3_MAPGIS_CONVERT_FAILED", err)
        })
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_open_geodata_query(
    window: tauri::WebviewWindow,
    request: crate::geodata_tools::OpenGeodataQueryRequest,
) -> Result<serde_json::Value, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let started = std::time::Instant::now();
        let state = app.state::<ApplicationServiceState>();
        let app_data = crate::application::resolve_app_data_dir(&state);
        let app_for_emit = app.clone();
        let mut result = crate::geodata_tools::execute_open_geodata_query_with_progress(
            &app_data,
            &request,
            &move |progress_ev| {
                let _ = app_for_emit.emit("geodata:xyz-tiles-progress", progress_ev);
            },
        )
        .map_err(|err| {
            ShellError::application_service_with_context("V3_OPEN_GEODATA_QUERY_FAILED", err)
        })?;
        crate::geodata_tools::maybe_record_geodata_download_task(
            &state,
            &request,
            &mut result,
            started.elapsed(),
        );
        Ok(result)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn geodata_fetch_preview_data_url(
    window: tauri::WebviewWindow,
    url: String,
) -> Result<serde_json::Value, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ApplicationServiceState>();
        let app_data = crate::application::resolve_app_data_dir(&state);
        match crate::geodata_tools::fetch_allowlisted_preview_data_url(&app_data, &url) {
            Ok(data_url) => Ok(serde_json::json!({ "ok": true, "data_url": data_url })),
            Err(err) => Ok(serde_json::json!({ "ok": false, "error": err })),
        }
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_experiment_project_get(
    window: tauri::WebviewWindow,
) -> Result<insar_application_service::ExperimentProjectSnapshot, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .experiment_project_get()
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_experiment_project_add_task(
    window: tauri::WebviewWindow,
    task_id: String,
) -> Result<insar_application_service::ExperimentProjectSnapshot, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .experiment_project_add_task(&task_id)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_experiment_project_remove_item(
    window: tauri::WebviewWindow,
    item_id: String,
) -> Result<insar_application_service::ExperimentProjectSnapshot, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ApplicationServiceState>()
            .experiment_project_remove_item(&item_id)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

#[tauri::command]
pub async fn application_experiment_project_export(
    window: tauri::WebviewWindow,
    export_dir: Option<String>,
) -> Result<insar_application_service::ExperimentProjectExportResult, ShellError> {
    validate_application_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let dir_opt = export_dir
            .map(|d| d.trim().to_owned())
            .filter(|d| !d.is_empty())
            .map(std::path::PathBuf::from);
        app.state::<ApplicationServiceState>()
            .experiment_project_export(dir_opt)
            .map_err(application_error)
    })
    .await
    .map_err(|_| ShellError::application_service("V3_APPLICATION_UNAVAILABLE"))?
}

// --- BEGIN AUTO-GENERATED MACOS CDP DISPATCH ---
#[cfg(target_os = "macos")]
fn extract_cdp_arg<T: serde::de::DeserializeOwned>(
    args: &serde_json::Value,
    snake: &str,
    camel: &str,
) -> Result<T, serde_json::Value> {
    let raw = args
        .get(camel)
        .or_else(|| args.get(snake))
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    serde_json::from_value(raw).map_err(|e| {
        serde_json::json!({
            "code": "V3_INVALID_ARGS",
            "message": format!("invalid arg {camel}/{snake}: {e}"),
        })
    })
}

#[cfg(target_os = "macos")]
pub fn dispatch_macos_cdp_command(
    app: &tauri::AppHandle,
    cmd: &str,
    args: &serde_json::Value,
) -> Result<serde_json::Value, serde_json::Value> {
    let Some(window) = app.get_webview_window("main") else {
        return Err(
            serde_json::json!({"code": "WINDOW_MISSING", "message": "main window not found"}),
        );
    };
    match cmd {
        "ui_smoke_status" => match ui_smoke_status(window.clone(), app.state::<UiSmokeState>()) {
            Ok(val) => serde_json::to_value(val)
                .map_err(|e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()})),
            Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
            )),
        },
        "shell_app_info" => match shell_app_info(window.clone()) {
            Ok(val) => serde_json::to_value(val)
                .map_err(|e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()})),
            Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
            )),
        },
        "shell_window_toggle_maximize" => {
            match shell_window_toggle_maximize(window.clone(), app.state::<SavedWindowBounds>()) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_service_readiness" => {
            match application_service_readiness(
                window.clone(),
                app.state::<ApplicationServiceState>(),
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_core_restore" => {
            let arg_include_terminal: Option<bool> =
                extract_cdp_arg(args, "include_terminal", "includeTerminal")?;
            match tauri::async_runtime::block_on(application_core_restore(
                window.clone(),
                arg_include_terminal,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_workbench_page" => {
            let arg_request: ApplicationWorkbenchPageRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_workbench_page(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_batch_page" => {
            let arg_batch_id: String = extract_cdp_arg(args, "batch_id", "batchId")?;
            let arg_revision: u64 = extract_cdp_arg(args, "revision", "revision")?;
            let arg_page: u64 = extract_cdp_arg(args, "page", "page")?;
            let arg_page_size: u64 = extract_cdp_arg(args, "page_size", "pageSize")?;
            match tauri::async_runtime::block_on(application_batch_page(
                window.clone(),
                arg_batch_id,
                arg_revision,
                arg_page,
                arg_page_size,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_download_task_page" => {
            let arg_request: StoreTaskPageRequest = extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_download_task_page(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_load_task_source_batch_page" => {
            let arg_request: StoreTaskSourceBatchPageRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_load_task_source_batch_page(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_task_overview" => {
            match tauri::async_runtime::block_on(application_task_overview(window.clone())) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_download_history_page" => {
            let arg_request: StoreTaskHistoryPageRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_download_history_page(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_download_task_event_page" => {
            let arg_request: StoreTaskEventPageRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_download_task_event_page(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_list_task_conflict_files" => {
            let arg_request: StoreTaskConflictFilesRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_list_task_conflict_files(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_delete_task_conflict_files" => {
            let arg_request: DeleteTaskConflictFilesRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_delete_task_conflict_files(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_open_external_url" => {
            let arg_url: String = extract_cdp_arg(args, "url", "url")?;
            match application_open_external_url(window.clone(), arg_url) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_open_download_task_output" => {
            let arg_request: StoreTaskOutputDirectoryRequest =
                extract_cdp_arg(args, "request", "request")?;
            match application_open_download_task_output(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_request,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_aoi_import" => {
            let arg_request: ApplicationAoiImportRequest =
                extract_cdp_arg(args, "request", "request")?;
            match application_aoi_import(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_request,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_aoi_normalize" => {
            let arg_request: ApplicationAoiNormalizeRequest =
                extract_cdp_arg(args, "request", "request")?;
            match application_aoi_normalize(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_request,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_admin_boundary_search" => {
            let arg_request: AdminBoundarySearchRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_admin_boundary_search(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_admin_boundary_resolve" => {
            let arg_request: AdminBoundaryResolveRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_admin_boundary_resolve(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_admin_options" => {
            let arg_request: AdminBoundaryOptionsRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_admin_options(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_geo_boundary_countries" => {
            match tauri::async_runtime::block_on(application_geo_boundary_countries(window.clone()))
            {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_geo_boundary_levels" => {
            let arg_request: GeoBoundaryLevelsRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_geo_boundary_levels(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_geo_boundary_units" => {
            let arg_request: GeoBoundaryUnitsRequest = extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_geo_boundary_units(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_geo_boundary_resolve" => {
            let arg_request: GeoBoundaryResolveRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_geo_boundary_resolve(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_gacos_plan" => {
            let arg_payload: Value = extract_cdp_arg(args, "payload", "payload")?;
            match application_gacos_plan(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_payload,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_gacos_dry_run" => {
            let arg_payload: Value = extract_cdp_arg(args, "payload", "payload")?;
            match application_gacos_dry_run(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_payload,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_gacos_submit_request" => {
            let arg_request: GacosConfirmRequest = extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_gacos_submit_request(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_gacos_request_status" => {
            let arg_task_id: String = extract_cdp_arg(args, "task_id", "taskId")?;
            match tauri::async_runtime::block_on(application_gacos_request_status(
                window.clone(),
                arg_task_id,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_gacos_ledger" => {
            match application_gacos_ledger(window.clone(), app.state::<ApplicationServiceState>()) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_gacos_import_result" => {
            let arg_request: GacosResultImportRequest =
                extract_cdp_arg(args, "request", "request")?;
            match application_gacos_import_result(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_request,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_gacos_create_import_task" => {
            let arg_request: GacosCreateImportTaskRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_gacos_create_import_task(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_gacos_create_result_task" => {
            let arg_request: GacosCreateResultTaskRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_gacos_create_result_task(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_gacos_supply_result_link" => {
            let arg_request: GacosSupplyResultLinkRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_gacos_supply_result_link(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_report_generate" => {
            let arg_payload: Value = extract_cdp_arg(args, "payload", "payload")?;
            match application_report_generate(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_payload,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_update_workbench_selection" => {
            let arg_command: WorkbenchSelectionCommand =
                extract_cdp_arg(args, "command", "command")?;
            match tauri::async_runtime::block_on(application_update_workbench_selection(
                window.clone(),
                arg_command,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_workspace_open" => {
            let arg_request: WorkspaceOpenRequest = extract_cdp_arg(args, "request", "request")?;
            match application_workspace_open(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_request,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_workspace_create" => {
            let arg_request: WorkspaceCreateRequest = extract_cdp_arg(args, "request", "request")?;
            match application_workspace_create(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_request,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_workspace_prepare_location" => {
            let arg_request: WorkspacePrepareLocationRequest =
                extract_cdp_arg(args, "request", "request")?;
            match application_workspace_prepare_location(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_request,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_project_create" => {
            let arg_request: WorkspaceNamedNodeRequest =
                extract_cdp_arg(args, "request", "request")?;
            match application_project_create(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_request,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_project_select" => {
            let arg_request: WorkspaceSelectNodeRequest =
                extract_cdp_arg(args, "request", "request")?;
            match application_project_select(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_request,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_region_create" => {
            let arg_request: WorkspaceNamedNodeRequest =
                extract_cdp_arg(args, "request", "request")?;
            match application_region_create(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_request,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_region_select" => {
            let arg_request: WorkspaceSelectNodeRequest =
                extract_cdp_arg(args, "request", "request")?;
            match application_region_select(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_request,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_bind_region_aoi" => {
            let arg_request: ApplicationBindRegionAoiRequest =
                extract_cdp_arg(args, "request", "request")?;
            match application_bind_region_aoi(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_request,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_clear_region_aoi" => {
            let arg_request: ApplicationClearRegionAoiRequest =
                extract_cdp_arg(args, "request", "request")?;
            match application_clear_region_aoi(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_request,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_set_working_aoi" => {
            let arg_request: ApplicationSetWorkingAoiRequest =
                extract_cdp_arg(args, "request", "request")?;
            match application_set_working_aoi(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_request,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_clear_working_aoi" => {
            match application_clear_working_aoi(
                window.clone(),
                app.state::<ApplicationServiceState>(),
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_working_aoi" => {
            match application_working_aoi(window.clone(), app.state::<ApplicationServiceState>()) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_plan_dem_task" => {
            let arg_request: DemTaskPlanRequest = extract_cdp_arg(args, "request", "request")?;
            match application_plan_dem_task(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_request,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_create_dem_task" => {
            let arg_request: DemTaskRequest = extract_cdp_arg(args, "request", "request")?;
            match application_create_dem_task(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_request,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_handoff_dem_conversion" => {
            let arg_request: DemConversionRequest = extract_cdp_arg(args, "request", "request")?;
            match application_handoff_dem_conversion(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_request,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_resolve_dem_conversion_source" => {
            let arg_request: StoreDemConversionSourceRequest =
                extract_cdp_arg(args, "request", "request")?;
            match application_resolve_dem_conversion_source(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_request,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_set_dem_source_identity" => {
            let arg_identity: StoreDemSourceIdentity =
                extract_cdp_arg(args, "identity", "identity")?;
            match application_set_dem_source_identity(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_identity,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_get_dem_task_settings" => {
            match tauri::async_runtime::block_on(application_get_dem_task_settings(window.clone()))
            {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_save_dem_task_settings" => {
            let arg_settings: DemTaskSettings = extract_cdp_arg(args, "settings", "settings")?;
            match tauri::async_runtime::block_on(application_save_dem_task_settings(
                window.clone(),
                arg_settings,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_get_sar_download_policy_settings" => {
            match application_get_sar_download_policy_settings(
                window.clone(),
                app.state::<ApplicationServiceState>(),
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_save_sar_download_policy_settings" => {
            let arg_settings: SarDownloadPolicySettings =
                extract_cdp_arg(args, "settings", "settings")?;
            match application_save_sar_download_policy_settings(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_settings,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_get_network_settings" => {
            match application_get_network_settings(
                window.clone(),
                app.state::<ApplicationServiceState>(),
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_save_network_settings" => {
            let arg_settings: ApplicationNetworkSettings =
                extract_cdp_arg(args, "settings", "settings")?;
            match application_save_network_settings(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_settings,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_test_network_connection" => {
            match tauri::async_runtime::block_on(application_test_network_connection(
                window.clone(),
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_probe_provider_connectivity" => {
            match tauri::async_runtime::block_on(application_probe_provider_connectivity(
                window.clone(),
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_standalone_dem" => {
            let arg_request: StandaloneDemRequest = extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_standalone_dem(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_create_standalone_dem_task" => {
            let arg_request: StandaloneDemAoiTaskRequest =
                extract_cdp_arg(args, "request", "request")?;
            match application_create_standalone_dem_task(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_request,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_import_scenes" => {
            let arg_request: SceneImportRequest = extract_cdp_arg(args, "request", "request")?;
            match application_import_scenes(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_request,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_match_orbits_directory" => {
            let arg_request: OrbitDirectoryMatchRequest =
                extract_cdp_arg(args, "request", "request")?;
            match application_match_orbits_directory(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_request,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_create_companion_orbit_task" => {
            let arg_source_task_id: String =
                extract_cdp_arg(args, "source_task_id", "sourceTaskId")?;
            match application_create_companion_orbit_task(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_source_task_id,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_create_orbit_task" => {
            let arg_request: OrbitTaskRequest = extract_cdp_arg(args, "request", "request")?;
            match application_create_orbit_task(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_request,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_create_standalone_orbit_task" => {
            let arg_request: StandaloneOrbitTaskRequest =
                extract_cdp_arg(args, "request", "request")?;
            match application_create_standalone_orbit_task(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_request,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_list_sources" => {
            match tauri::async_runtime::block_on(application_list_sources(window.clone())) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_source_views" => {
            match tauri::async_runtime::block_on(application_source_views(window.clone())) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_provider_capabilities" => {
            match application_provider_capabilities(
                window.clone(),
                app.state::<ApplicationServiceState>(),
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_effective_capability" => {
            let arg_capability_id: String = extract_cdp_arg(args, "capability_id", "capabilityId")?;
            match application_effective_capability(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_capability_id,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_search_start" => {
            let arg_request: insar_application_service::UnifiedSearchRequest =
                extract_cdp_arg(args, "request", "request")?;
            let arg_bypass_cache: Option<bool> =
                extract_cdp_arg(args, "bypass_cache", "bypassCache")?;
            match tauri::async_runtime::block_on(application_search_start(
                window.clone(),
                arg_request,
                arg_bypass_cache,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_search_progress" => {
            let arg_request_id: String = extract_cdp_arg(args, "request_id", "requestId")?;
            match tauri::async_runtime::block_on(application_search_progress(
                window.clone(),
                arg_request_id,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_search_cancel" => {
            let arg_request_id: String = extract_cdp_arg(args, "request_id", "requestId")?;
            match tauri::async_runtime::block_on(application_search_cancel(
                window.clone(),
                arg_request_id,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_sar_search_and_freeze" => {
            let arg_request: ApplicationSarSearchAndFreezeRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_sar_search_and_freeze(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_sar_search_progress" => {
            let arg_request_id: String = extract_cdp_arg(args, "request_id", "requestId")?;
            match application_sar_search_progress(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_request_id,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_sar_background_count" => {
            let arg_request: ApplicationSarBackgroundCountRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_sar_background_count(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_catalog_search_and_freeze" => {
            let arg_request: ApplicationCatalogSearchAndFreezeRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_catalog_search_and_freeze(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_catalog_intent_search_and_freeze" => {
            let arg_request: ApplicationCatalogIntentSearchAndFreezeRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_catalog_intent_search_and_freeze(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_cancel_catalog_search" => {
            let arg_request_id: String = extract_cdp_arg(args, "request_id", "requestId")?;
            match tauri::async_runtime::block_on(application_cancel_catalog_search(
                window.clone(),
                arg_request_id,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_cancel_request" => {
            let arg_request_id: String = extract_cdp_arg(args, "request_id", "requestId")?;
            match application_cancel_request(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_request_id,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_account_session_statuses" => {
            match tauri::async_runtime::block_on(application_account_session_statuses(
                window.clone(),
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_cdse_session_status" => {
            match application_cdse_session_status(
                window.clone(),
                app.state::<ApplicationServiceState>(),
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_cdse_login" => {
            let arg_request: CdseLoginRequest = extract_cdp_arg(args, "request", "request")?;
            match application_cdse_login(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_request,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_cdse_logout" => {
            match application_cdse_logout(window.clone(), app.state::<ApplicationServiceState>()) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_cdse_network_policy" => {
            match application_cdse_network_policy(
                window.clone(),
                app.state::<ApplicationServiceState>(),
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_set_cdse_network_policy" => {
            let arg_mode: CdseNetworkMode = extract_cdp_arg(args, "mode", "mode")?;
            match application_set_cdse_network_policy(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_mode,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_cdse_network_diagnostic" => {
            let arg_mode: CdseNetworkMode = extract_cdp_arg(args, "mode", "mode")?;
            match application_cdse_network_diagnostic(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_mode,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_preview_asset" => {
            let arg_request: PreviewAssetRequest = extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_preview_asset(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_save_preview_asset" => {
            let arg_request: PreviewSaveRequest = extract_cdp_arg(args, "request", "request")?;
            match application_save_preview_asset(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_request,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_cdse_quicklook" => {
            let arg_request: CdseQuicklookRequest = extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_cdse_quicklook(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_sentinel2_map_preview" => {
            let arg_request: Sentinel2MapPreviewRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_sentinel2_map_preview(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_usgs_session_status" => {
            match application_usgs_session_status(
                window.clone(),
                app.state::<ApplicationServiceState>(),
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_usgs_login" => {
            let arg_request: UsgsM2mLoginRequest = extract_cdp_arg(args, "request", "request")?;
            match application_usgs_login(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_request,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_usgs_logout" => {
            match application_usgs_logout(window.clone(), app.state::<ApplicationServiceState>()) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_landsat_browse" => {
            let arg_request: LandsatBrowseRequest = extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_landsat_browse(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_landsat_asset_details" => {
            let arg_request: LandsatAssetDetailsRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_landsat_asset_details(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_bhoonidhi_asset_size" => {
            let arg_request: BhoonidhiAssetSizeRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_bhoonidhi_asset_size(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_create_landsat_bundle_task" => {
            let arg_request: LandsatCreateBundleTaskRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_create_landsat_bundle_task(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "credential_status" => {
            match tauri::async_runtime::block_on(credential_status(window.clone())) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "credential_bhoonidhi_auth" => {
            match tauri::async_runtime::block_on(credential_bhoonidhi_auth(window.clone())) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "credential_bhoonidhi_session_status" => {
            match credential_bhoonidhi_session_status(window.clone()) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "credential_reveal" => {
            let arg_field: insar_download::DesktopCredentialField =
                extract_cdp_arg(args, "field", "field")?;
            match credential_reveal(window.clone(), arg_field) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "credential_read_for_edit" => {
            let arg_field: insar_download::DesktopCredentialField =
                extract_cdp_arg(args, "field", "field")?;
            match tauri::async_runtime::block_on(credential_read_for_edit(
                window.clone(),
                arg_field,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "credential_agent_channel_key_read" => {
            let arg_channel_id: String = extract_cdp_arg(args, "channelId", "channel_id")?;
            match tauri::async_runtime::block_on(credential_agent_channel_key_read(
                window.clone(),
                arg_channel_id,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "credential_update_field" => {
            let arg_field: insar_download::DesktopCredentialField =
                extract_cdp_arg(args, "field", "field")?;
            let arg_value: String = extract_cdp_arg(args, "value", "value")?;
            match tauri::async_runtime::block_on(credential_update_field(
                window.clone(),
                arg_field,
                arg_value,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "credential_earthdata_auth" => {
            let arg_force: Option<bool> = extract_cdp_arg(args, "force", "force")?;
            match tauri::async_runtime::block_on(credential_earthdata_auth(
                window.clone(),
                arg_force,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "credential_save_cdse_login" => {
            let arg_username: String = extract_cdp_arg(args, "username", "username")?;
            let arg_password: String = extract_cdp_arg(args, "password", "password")?;
            match credential_save_cdse_login(window.clone(), arg_username, arg_password) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "credential_clear_cdse" => match credential_clear_cdse(window.clone()) {
            Ok(val) => serde_json::to_value(val)
                .map_err(|e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()})),
            Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
            )),
        },
        "credential_save_usgs_login" => {
            let arg_username: String = extract_cdp_arg(args, "username", "username")?;
            let arg_application_token: String =
                extract_cdp_arg(args, "application_token", "applicationToken")?;
            match credential_save_usgs_login(window.clone(), arg_username, arg_application_token) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "credential_clear_usgs" => match credential_clear_usgs(window.clone()) {
            Ok(val) => serde_json::to_value(val)
                .map_err(|e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()})),
            Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
            )),
        },
        "credential_save_earthdata_token" => {
            let arg_token: String = extract_cdp_arg(args, "token", "token")?;
            match credential_save_earthdata_token(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_token,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "credential_save_earthdata_login" => {
            let arg_username: String = extract_cdp_arg(args, "username", "username")?;
            let arg_password: String = extract_cdp_arg(args, "password", "password")?;
            match credential_save_earthdata_login(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_username,
                arg_password,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "credential_clear_earthdata" => {
            match credential_clear_earthdata(window.clone(), app.state::<ApplicationServiceState>())
            {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "credential_save_bhoonidhi_login" => {
            let arg_username: String = extract_cdp_arg(args, "username", "username")?;
            let arg_password: String = extract_cdp_arg(args, "password", "password")?;
            match credential_save_bhoonidhi_login(window.clone(), arg_username, arg_password) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "credential_clear_bhoonidhi" => match credential_clear_bhoonidhi(window.clone()) {
            Ok(val) => serde_json::to_value(val)
                .map_err(|e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()})),
            Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
            )),
        },
        "credential_save_opentopography_key" => {
            let arg_api_key: String = extract_cdp_arg(args, "api_key", "apiKey")?;
            match credential_save_opentopography_key(window.clone(), arg_api_key) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "credential_clear_opentopography_key" => {
            match credential_clear_opentopography_key(window.clone()) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "credential_save_gacos_email" => {
            let arg_email: String = extract_cdp_arg(args, "email", "email")?;
            match credential_save_gacos_email(window.clone(), arg_email) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "credential_clear_gacos_email" => match credential_clear_gacos_email(window.clone()) {
            Ok(val) => serde_json::to_value(val)
                .map_err(|e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()})),
            Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
            )),
        },
        "application_create_landsat_asset_task" => {
            let arg_request: LandsatCreateAssetTaskRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_create_landsat_asset_task(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_create_reviewed_mpc_asset_task" => {
            let arg_request: ReviewedMpcCreateAssetTaskRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_create_reviewed_mpc_asset_task(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_create_landsat_bundle_batch_task" => {
            let arg_request: insar_application_service::LandsatCreateBundleBatchTaskRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_create_landsat_bundle_batch_task(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_create_landsat_asset_batch_task" => {
            let arg_request: insar_application_service::LandsatCreateAssetBatchTaskRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_create_landsat_asset_batch_task(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_task_delivery_admission" => {
            let arg_task_id: String = extract_cdp_arg(args, "task_id", "taskId")?;
            match tauri::async_runtime::block_on(application_task_delivery_admission(
                window.clone(),
                arg_task_id,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_optical_preparation_progress" => {
            let arg_request_id: String = extract_cdp_arg(args, "request_id", "requestId")?;
            match tauri::async_runtime::block_on(application_optical_preparation_progress(
                window.clone(),
                arg_request_id,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_create_sentinel2_archive_batch_task" => {
            let arg_request: Sentinel2CreateArchiveBatchTaskRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_create_sentinel2_archive_batch_task(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_create_sentinel2_archive_task" => {
            let arg_request: Sentinel2CreateArchiveTaskRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_create_sentinel2_archive_task(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_create_sentinel2_band_task" => {
            let arg_request: Sentinel2CreateBandTaskRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_create_sentinel2_band_task(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_create_sentinel2_band_batch_task" => {
            let arg_request: insar_application_service::Sentinel2CreateBandBatchTaskRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_create_sentinel2_band_batch_task(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_create_sentinel2_l1c_band_batch_task" => {
            let arg_request: insar_application_service::Sentinel2L1cCreateBandBatchTaskRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_create_sentinel2_l1c_band_batch_task(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_create_hls_band_batch_task" => {
            let arg_request: insar_application_service::HlsCreateBandBatchTaskRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_create_hls_band_batch_task(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_precheck_optical_aoi_coverage" => {
            let arg_request: insar_application_service::OpticalDerivedPrecheckBatchRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_precheck_optical_aoi_coverage(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_create_optical_derived_task" => {
            let arg_request: insar_application_service::OpticalDerivedCreateTaskRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_create_optical_derived_task(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_nisar_remote_subset_capability" => {
            match application_nisar_remote_subset_capability(
                window.clone(),
                app.state::<ApplicationServiceState>(),
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_create_nisar_remote_subset_task" => {
            let arg_request: NisarCreateRemoteSubsetTaskRequest =
                extract_cdp_arg(args, "request", "request")?;
            match application_create_nisar_remote_subset_task(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_request,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_prepare_nisar_subset_intent_v2" => {
            let arg_request: NisarPrepareSubsetIntentV2Request =
                extract_cdp_arg(args, "request", "request")?;
            match application_prepare_nisar_subset_intent_v2(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_request,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_confirm_nisar_subset_task_v2" => {
            let arg_request: NisarConfirmSubsetTaskV2Request =
                extract_cdp_arg(args, "request", "request")?;
            match application_confirm_nisar_subset_task_v2(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_request,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_preflight_nisar_subset_batch_v2" => {
            let arg_request: NisarPreflightSubsetBatchV2Request =
                extract_cdp_arg(args, "request", "request")?;
            match application_preflight_nisar_subset_batch_v2(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_request,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_prepare_nisar_subset_batch_v2" => {
            let arg_request: NisarPrepareSubsetBatchV2Request =
                extract_cdp_arg(args, "request", "request")?;
            match application_prepare_nisar_subset_batch_v2(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_request,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_confirm_nisar_subset_batch_task_v2" => {
            let arg_request: NisarConfirmSubsetBatchTaskV2Request =
                extract_cdp_arg(args, "request", "request")?;
            match application_confirm_nisar_subset_batch_task_v2(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_request,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_append_nisar_subset_batch_task_v2" => {
            let arg_request: NisarAppendSubsetBatchTaskV2Request =
                extract_cdp_arg(args, "request", "request")?;
            match application_append_nisar_subset_batch_task_v2(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_request,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_read_nisar_subset_append_context_v2" => {
            let arg_request: NisarAppendSubsetContextV2Request =
                extract_cdp_arg(args, "request", "request")?;
            match application_read_nisar_subset_append_context_v2(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_request,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_preflight_nisar_subset_append_v2" => {
            let arg_request: NisarPreflightSubsetAppendV2Request =
                extract_cdp_arg(args, "request", "request")?;
            match application_preflight_nisar_subset_append_v2(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_request,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_create_nisar_subset_successor_task_v2" => {
            let arg_request: NisarCreateSubsetSuccessorTaskV2Request =
                extract_cdp_arg(args, "request", "request")?;
            match application_create_nisar_subset_successor_task_v2(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_request,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_read_nisar_archived_recovery_context_v2" => {
            let arg_source_task_id: String =
                extract_cdp_arg(args, "source_task_id", "sourceTaskId")?;
            let arg_new_task_id: String = extract_cdp_arg(args, "new_task_id", "newTaskId")?;
            match application_read_nisar_archived_recovery_context_v2(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_source_task_id,
                arg_new_task_id,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_create_nisar_archived_recovery_v2" => {
            let arg_request: insar_application_service::NisarCreateArchivedRecoveryV2Request =
                extract_cdp_arg(args, "request", "request")?;
            match application_create_nisar_archived_recovery_v2(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_request,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_resume_nisar_subset_task" => {
            let arg_task_id: String = extract_cdp_arg(args, "task_id", "taskId")?;
            match application_resume_nisar_subset_task(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_task_id,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_start_newly_confirmed_task" => {
            let arg_task_id: String = extract_cdp_arg(args, "task_id", "taskId")?;
            match application_start_newly_confirmed_task(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_task_id,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_update_search_workbench" => {
            let arg_request: StoreUpdateWorkbenchRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_update_search_workbench(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_create_task" => {
            let arg_batch_id: String = extract_cdp_arg(args, "batch_id", "batchId")?;
            let arg_revision: u64 = extract_cdp_arg(args, "revision", "revision")?;
            let arg_item_ids: Vec<String> = extract_cdp_arg(args, "item_ids", "itemIds")?;
            let arg_options: insar_application_service::UnifiedCreateTaskOptions =
                extract_cdp_arg(args, "options", "options")?;
            match tauri::async_runtime::block_on(application_create_task(
                window.clone(),
                arg_batch_id,
                arg_revision,
                arg_item_ids,
                arg_options,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_preview_all_match_task" => {
            let arg_request: insar_application_service::BatchAllMatchPreviewRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_preview_all_match_task(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_create_all_match_task" => {
            let arg_request: insar_application_service::BatchAllMatchCreateTaskRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_create_all_match_task(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_create_download_task" => {
            let arg_request: StoreCreateTaskRequest = extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_create_download_task(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_append_download_task" => {
            let arg_request: StoreAppendTaskRequest = extract_cdp_arg(args, "request", "request")?;
            match application_append_download_task(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_request,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_create_task_from_workbench" => {
            let arg_request: CreateTaskFromWorkbenchRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_create_task_from_workbench(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_append_task_from_workbench" => {
            let arg_request: AppendTaskFromWorkbenchRequest =
                extract_cdp_arg(args, "request", "request")?;
            match application_append_task_from_workbench(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_request,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_query_source_append_status" => {
            let arg_request: insar_application_service::AppendTaskFromSourceBatchRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_query_source_append_status(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_query_historical_successor_status" => {
            let arg_request: insar_application_service::CreateTaskFromHistoricalSourceBatchRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_query_historical_successor_status(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_query_burst_append_status" => {
            let arg_request: insar_application_service::BurstAppendAssetTaskRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_query_burst_append_status(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_query_burst_successor_status" => {
            let arg_request: insar_application_service::BurstCreateHistoricalAssetTaskRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_query_burst_successor_status(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_query_nisar_append_status" => {
            let arg_request: insar_application_service::NisarAppendSubsetBatchTaskV2Request =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_query_nisar_append_status(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_query_nisar_successor_status" => {
            let arg_request: insar_application_service::NisarCreateSubsetSuccessorTaskV2Request =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_query_nisar_successor_status(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_query_nisar_archived_recovery_status" => {
            let arg_request: insar_application_service::NisarCreateArchivedRecoveryV2Request =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_query_nisar_archived_recovery_status(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_query_reacquire_status" => {
            let arg_request: insar_core::StoreUpdateTaskRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_query_reacquire_status(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_append_task_from_source_batch" => {
            let arg_request: AppendTaskFromSourceBatchRequest =
                extract_cdp_arg(args, "request", "request")?;
            match application_append_task_from_source_batch(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_request,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_create_task_from_historical_source_batch" => {
            let arg_request: CreateTaskFromHistoricalSourceBatchRequest =
                extract_cdp_arg(args, "request", "request")?;
            match application_create_task_from_historical_source_batch(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_request,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_create_burst_asset_task" => {
            let arg_request: BurstCreateAssetTaskRequest =
                extract_cdp_arg(args, "request", "request")?;
            match application_create_burst_asset_task(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_request,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_append_burst_asset_task" => {
            let arg_request: BurstAppendAssetTaskRequest =
                extract_cdp_arg(args, "request", "request")?;
            match application_append_burst_asset_task(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_request,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_create_historical_burst_asset_task" => {
            let arg_request: BurstCreateHistoricalAssetTaskRequest =
                extract_cdp_arg(args, "request", "request")?;
            match application_create_historical_burst_asset_task(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_request,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_task_action" => {
            let arg_task_id: String = extract_cdp_arg(args, "task_id", "taskId")?;
            let arg_revision: u64 = extract_cdp_arg(args, "revision", "revision")?;
            let arg_action: insar_application_service::UnifiedTaskAction =
                extract_cdp_arg(args, "action", "action")?;
            match tauri::async_runtime::block_on(application_task_action(
                window.clone(),
                arg_task_id,
                arg_revision,
                arg_action,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_task_retry_fallback" => {
            let arg_task_id: String = extract_cdp_arg(args, "task_id", "taskId")?;
            let arg_confirm_refreshed_batch_id: Option<String> = extract_cdp_arg(
                args,
                "confirm_refreshed_batch_id",
                "confirmRefreshedBatchId",
            )?;
            match tauri::async_runtime::block_on(application_task_retry_fallback(
                window.clone(),
                arg_task_id,
                arg_confirm_refreshed_batch_id,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_update_download_task" => {
            let arg_request: StoreUpdateTaskRequest = extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_update_download_task(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_update_download_task_view" => {
            let arg_request: StoreUpdateTaskViewRequest =
                extract_cdp_arg(args, "request", "request")?;
            match application_update_download_task_view(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_request,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_archive_download_task" => {
            let arg_request: StoreArchiveTaskRequest = extract_cdp_arg(args, "request", "request")?;
            match application_archive_download_task(
                window.clone(),
                app.state::<ApplicationServiceState>(),
                arg_request,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_delete_download_history" => {
            let arg_request: StoreDeleteHistoryRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_delete_download_history(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_task_item_action" => {
            let arg_task_id: String = extract_cdp_arg(args, "task_id", "taskId")?;
            let arg_revision: Option<u64> = extract_cdp_arg(args, "revision", "revision")?;
            let arg_item_id: String = extract_cdp_arg(args, "item_id", "itemId")?;
            let arg_action: insar_application_service::UnifiedTaskItemAction =
                extract_cdp_arg(args, "action", "action")?;
            match tauri::async_runtime::block_on(application_task_item_action(
                window.clone(),
                arg_task_id,
                arg_revision,
                arg_item_id,
                arg_action,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_task_delete_history" => {
            let arg_task_id: String = extract_cdp_arg(args, "task_id", "taskId")?;
            match tauri::async_runtime::block_on(application_task_delete_history(
                window.clone(),
                arg_task_id,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_task_set_pinned" => {
            let arg_task_id: String = extract_cdp_arg(args, "task_id", "taskId")?;
            let arg_pinned: bool = extract_cdp_arg(args, "pinned", "pinned")?;
            match tauri::async_runtime::block_on(application_task_set_pinned(
                window.clone(),
                arg_task_id,
                arg_pinned,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_list_pinned_tasks" => {
            match tauri::async_runtime::block_on(application_list_pinned_tasks(window.clone())) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_task_retry_fallback_eligibility" => {
            let arg_task_ids: Vec<String> = extract_cdp_arg(args, "task_ids", "taskIds")?;
            match tauri::async_runtime::block_on(application_task_retry_fallback_eligibility(
                window.clone(),
                arg_task_ids,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "check_existing_downloaded_scenes" => {
            let arg_items: Vec<insar_application_service::ExistingDownloadCheckItem> =
                extract_cdp_arg(args, "items", "items")?;
            let arg_target_dir: Option<String> = extract_cdp_arg(args, "target_dir", "targetDir")?;
            match tauri::async_runtime::block_on(check_existing_downloaded_scenes(
                window.clone(),
                arg_items,
                arg_target_dir,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "agent_sessions_list" => {
            let arg_include_archived: Option<bool> =
                extract_cdp_arg(args, "include_archived", "includeArchived")?;
            match tauri::async_runtime::block_on(agent_sessions_list(
                window.clone(),
                arg_include_archived,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "agent_sessions_create" => {
            let arg_title: Option<String> = extract_cdp_arg(args, "title", "title")?;
            let arg_workspace_id: Option<String> =
                extract_cdp_arg(args, "workspace_id", "workspaceId")?;
            match tauri::async_runtime::block_on(agent_sessions_create(
                window.clone(),
                arg_title,
                arg_workspace_id,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "agent_sessions_delete" => {
            let arg_session_id: String = extract_cdp_arg(args, "session_id", "sessionId")?;
            match tauri::async_runtime::block_on(agent_sessions_delete(
                window.clone(),
                arg_session_id,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "agent_sessions_rename" => {
            let arg_session_id: String = extract_cdp_arg(args, "session_id", "sessionId")?;
            let arg_title: String = extract_cdp_arg(args, "title", "title")?;
            match tauri::async_runtime::block_on(agent_sessions_rename(
                window.clone(),
                arg_session_id,
                arg_title,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "agent_sessions_set_pinned" => {
            let arg_session_id: String = extract_cdp_arg(args, "session_id", "sessionId")?;
            let arg_pinned: bool = extract_cdp_arg(args, "pinned", "pinned")?;
            match tauri::async_runtime::block_on(agent_sessions_set_pinned(
                window.clone(),
                arg_session_id,
                arg_pinned,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "agent_sessions_set_archived" => {
            let arg_session_id: String = extract_cdp_arg(args, "session_id", "sessionId")?;
            let arg_archived: bool = extract_cdp_arg(args, "archived", "archived")?;
            match tauri::async_runtime::block_on(agent_sessions_set_archived(
                window.clone(),
                arg_session_id,
                arg_archived,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "agent_sessions_move" => {
            let arg_session_id: String = extract_cdp_arg(args, "session_id", "sessionId")?;
            let arg_workspace_id: Option<String> =
                extract_cdp_arg(args, "workspace_id", "workspaceId")?;
            match tauri::async_runtime::block_on(agent_sessions_move(
                window.clone(),
                arg_session_id,
                arg_workspace_id,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "agent_sessions_export" => {
            let arg_session_id: String = extract_cdp_arg(args, "session_id", "sessionId")?;
            let arg_format: insar_agent::session::SessionExportFormat =
                extract_cdp_arg(args, "format", "format")?;
            match tauri::async_runtime::block_on(agent_sessions_export(
                window.clone(),
                arg_session_id,
                arg_format,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "agent_sessions_export_to" => {
            let arg_session_id: String = extract_cdp_arg(args, "session_id", "sessionId")?;
            let arg_format: insar_agent::session::SessionExportFormat =
                extract_cdp_arg(args, "format", "format")?;
            let arg_directory: String = extract_cdp_arg(args, "directory", "directory")?;
            match tauri::async_runtime::block_on(agent_sessions_export_to(
                window.clone(),
                arg_session_id,
                arg_format,
                arg_directory,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "agent_workspaces_list" => {
            match tauri::async_runtime::block_on(agent_workspaces_list(window.clone())) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "agent_workspaces_create" => {
            let arg_name: String = extract_cdp_arg(args, "name", "name")?;
            let arg_folder: Option<String> = extract_cdp_arg(args, "folder", "folder")?;
            match tauri::async_runtime::block_on(agent_workspaces_create(
                window.clone(),
                arg_name,
                arg_folder,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "agent_workspaces_update" => {
            let arg_workspace_id: String = extract_cdp_arg(args, "workspace_id", "workspaceId")?;
            let arg_name: Option<String> = extract_cdp_arg(args, "name", "name")?;
            let arg_folder: Option<String> = extract_cdp_arg(args, "folder", "folder")?;
            let arg_clear_folder: Option<bool> =
                extract_cdp_arg(args, "clear_folder", "clearFolder")?;
            match tauri::async_runtime::block_on(agent_workspaces_update(
                window.clone(),
                arg_workspace_id,
                arg_name,
                arg_folder,
                arg_clear_folder,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "agent_workspaces_delete" => {
            let arg_workspace_id: String = extract_cdp_arg(args, "workspace_id", "workspaceId")?;
            match tauri::async_runtime::block_on(agent_workspaces_delete(
                window.clone(),
                arg_workspace_id,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "agent_send" => {
            let arg_session_id: String = extract_cdp_arg(args, "session_id", "sessionId")?;
            let arg_text: String = extract_cdp_arg(args, "text", "text")?;
            let arg_context: Option<insar_agent::session::SendContext> =
                extract_cdp_arg(args, "context", "context")?;
            match tauri::async_runtime::block_on(agent_send(
                window.clone(),
                arg_session_id,
                arg_text,
                arg_context,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "agent_stream_poll" => {
            let arg_session_id: String = extract_cdp_arg(args, "session_id", "sessionId")?;
            let arg_cursor: u64 = extract_cdp_arg(args, "cursor", "cursor")?;
            match tauri::async_runtime::block_on(agent_stream_poll(
                window.clone(),
                arg_session_id,
                arg_cursor,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "agent_plan_approve" => {
            let arg_plan_id: String = extract_cdp_arg(args, "plan_id", "planId")?;
            let arg_snapshot: serde_json::Value = extract_cdp_arg(args, "snapshot", "snapshot")?;
            let arg_output_dir: Option<String> = extract_cdp_arg(args, "output_dir", "outputDir")?;
            match tauri::async_runtime::block_on(agent_plan_approve(
                window.clone(),
                arg_plan_id,
                arg_snapshot,
                arg_output_dir,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "agent_plan_reject" => {
            let arg_plan_id: String = extract_cdp_arg(args, "plan_id", "planId")?;
            match tauri::async_runtime::block_on(agent_plan_reject(window.clone(), arg_plan_id)) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "agent_run_cancel" => {
            let arg_run_id: String = extract_cdp_arg(args, "run_id", "runId")?;
            match tauri::async_runtime::block_on(agent_run_cancel(window.clone(), arg_run_id)) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "agent_runs_list" => {
            let arg_session_id: String = extract_cdp_arg(args, "session_id", "sessionId")?;
            match tauri::async_runtime::block_on(agent_runs_list(window.clone(), arg_session_id)) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "agent_session_work" => {
            let arg_session_id: String = extract_cdp_arg(args, "session_id", "sessionId")?;
            match tauri::async_runtime::block_on(agent_session_work(window.clone(), arg_session_id))
            {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "agent_session_truncate" => {
            let arg_session_id: String = extract_cdp_arg(args, "session_id", "sessionId")?;
            let arg_before_seq: u64 = extract_cdp_arg(args, "before_seq", "beforeSeq")?;
            let arg_restore_aoi: Option<bool> = extract_cdp_arg(args, "restore_aoi", "restoreAoi")?;
            match tauri::async_runtime::block_on(agent_session_truncate(
                window.clone(),
                arg_session_id,
                arg_before_seq,
                arg_restore_aoi,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "agent_session_workspace_state" => {
            let arg_session_id: String = extract_cdp_arg(args, "session_id", "sessionId")?;
            match tauri::async_runtime::block_on(agent_session_workspace_state(
                window.clone(),
                arg_session_id,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "agent_session_set_working_aoi" => {
            let arg_session_id: String = extract_cdp_arg(args, "session_id", "sessionId")?;
            let arg_request: Option<ApplicationSetWorkingAoiRequest> =
                extract_cdp_arg(args, "request", "request")?;
            let arg_clear: Option<bool> = extract_cdp_arg(args, "clear", "clear")?;
            let arg_active_batch_id: Option<String> =
                extract_cdp_arg(args, "active_batch_id", "activeBatchId")?;
            let arg_active_resource_id: Option<String> =
                extract_cdp_arg(args, "active_resource_id", "activeResourceId")?;
            let arg_active_batch_revision: Option<u64> =
                extract_cdp_arg(args, "active_batch_revision", "activeBatchRevision")?;
            match tauri::async_runtime::block_on(agent_session_set_working_aoi(
                window.clone(),
                arg_session_id,
                arg_request,
                arg_clear,
                arg_active_batch_id,
                arg_active_resource_id,
                arg_active_batch_revision,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "agent_runs_open" => {
            let arg_run_id: String = extract_cdp_arg(args, "run_id", "runId")?;
            match tauri::async_runtime::block_on(agent_runs_open(window.clone(), arg_run_id)) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "agent_channels_get" => {
            match tauri::async_runtime::block_on(agent_channels_get(window.clone())) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "agent_disabled_items_get" => {
            match tauri::async_runtime::block_on(agent_disabled_items_get(window.clone())) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "agent_disabled_items_set" => {
            let arg_items: crate::agent::AgentDisabledItems =
                extract_cdp_arg(args, "items", "items")?;
            match tauri::async_runtime::block_on(agent_disabled_items_set(
                window.clone(),
                arg_items,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "agent_channels_save" => {
            let arg_channel: crate::agent::AgentChannelSave =
                extract_cdp_arg(args, "channel", "channel")?;
            match tauri::async_runtime::block_on(agent_channels_save(window.clone(), arg_channel)) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "agent_channels_test" => {
            let arg_channel_id: String = extract_cdp_arg(args, "channel_id", "channelId")?;
            match tauri::async_runtime::block_on(agent_channels_test(
                window.clone(),
                arg_channel_id,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "agent_channels_models" => {
            let arg_channel_id: String = extract_cdp_arg(args, "channel_id", "channelId")?;
            match tauri::async_runtime::block_on(agent_channels_models(
                window.clone(),
                arg_channel_id,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "agent_channels_probe_draft" => {
            let arg_draft: crate::agent::AgentChannelDraft =
                extract_cdp_arg(args, "draft", "draft")?;
            match tauri::async_runtime::block_on(agent_channels_probe_draft(
                window.clone(),
                arg_draft,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "agent_channels_test_draft" => {
            let arg_draft: crate::agent::AgentChannelDraft =
                extract_cdp_arg(args, "draft", "draft")?;
            match tauri::async_runtime::block_on(agent_channels_test_draft(
                window.clone(),
                arg_draft,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "agent_channels_set_default" => {
            let arg_channel_id: String = extract_cdp_arg(args, "channel_id", "channelId")?;
            match tauri::async_runtime::block_on(agent_channels_set_default(
                window.clone(),
                arg_channel_id,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "agent_channels_delete" => {
            let arg_channel_id: String = extract_cdp_arg(args, "channel_id", "channelId")?;
            match tauri::async_runtime::block_on(agent_channels_delete(
                window.clone(),
                arg_channel_id,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "agent_channel_rename" => {
            let arg_old_id: String = extract_cdp_arg(args, "old_id", "oldId")?;
            let arg_new_id: String = extract_cdp_arg(args, "new_id", "newId")?;
            let arg_api_key: Option<String> = extract_cdp_arg(args, "api_key", "apiKey")?;
            match tauri::async_runtime::block_on(agent_channel_rename(
                window.clone(),
                arg_old_id,
                arg_new_id,
                arg_api_key,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "agent_sync_public_model_registry" => {
            match tauri::async_runtime::block_on(agent_sync_public_model_registry(window.clone())) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "agent_oauth_open_browser" => {
            let arg_url: String = extract_cdp_arg(args, "url", "url")?;
            match tauri::async_runtime::block_on(agent_oauth_open_browser(window.clone(), arg_url))
            {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "agent_oauth_device_start" => {
            let arg_provider_id: String = extract_cdp_arg(args, "provider_id", "providerId")?;
            match tauri::async_runtime::block_on(agent_oauth_device_start(
                window.clone(),
                arg_provider_id,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "agent_oauth_device_poll" => {
            let arg_session_id: String = extract_cdp_arg(args, "session_id", "sessionId")?;
            match tauri::async_runtime::block_on(agent_oauth_device_poll(
                window.clone(),
                arg_session_id,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "agent_oauth_pkce_start" => {
            let arg_provider_id: String = extract_cdp_arg(args, "provider_id", "providerId")?;
            match tauri::async_runtime::block_on(agent_oauth_pkce_start(
                window.clone(),
                arg_provider_id,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "agent_oauth_pkce_complete" => {
            let arg_session_id: String = extract_cdp_arg(args, "session_id", "sessionId")?;
            let arg_callback: Option<String> =
                extract_cdp_arg(args, "callback_input", "callbackInput")?;
            match tauri::async_runtime::block_on(agent_oauth_pkce_complete(
                window.clone(),
                arg_session_id,
                arg_callback,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "agent_oauth_channels_meta" => {
            match tauri::async_runtime::block_on(agent_oauth_channels_meta(window.clone())) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "agent_oauth_channel_quota" => {
            let arg_channel_id: String = extract_cdp_arg(args, "channel_id", "channelId")?;
            match tauri::async_runtime::block_on(agent_oauth_channel_quota(
                window.clone(),
                arg_channel_id,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "agent_permission_mode_get" => {
            let arg_session_id: Option<String> = extract_cdp_arg(args, "session_id", "sessionId")?;
            match tauri::async_runtime::block_on(agent_permission_mode_get(
                window.clone(),
                arg_session_id,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "agent_permission_mode_set_global" => {
            let arg_mode: insar_agent::permission::PermissionMode =
                extract_cdp_arg(args, "mode", "mode")?;
            match tauri::async_runtime::block_on(agent_permission_mode_set_global(
                window.clone(),
                arg_mode,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "agent_permission_mode_set_session" => {
            let arg_session_id: String = extract_cdp_arg(args, "session_id", "sessionId")?;
            let arg_mode: Option<insar_agent::permission::PermissionMode> =
                extract_cdp_arg(args, "mode", "mode")?;
            match tauri::async_runtime::block_on(agent_permission_mode_set_session(
                window.clone(),
                arg_session_id,
                arg_mode,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "agent_stop_all" => match tauri::async_runtime::block_on(agent_stop_all(window.clone())) {
            Ok(val) => serde_json::to_value(val)
                .map_err(|e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()})),
            Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
            )),
        },
        "agent_activity_status" => {
            match tauri::async_runtime::block_on(agent_activity_status(window.clone())) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "eostage_account_status" => {
            let arg_refresh: Option<bool> =
                extract_cdp_arg(args, "refresh", "refresh").unwrap_or(None);
            match tauri::async_runtime::block_on(eostage_account_status(
                window.clone(),
                arg_refresh,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "eostage_account_login_start" => {
            match tauri::async_runtime::block_on(eostage_account_login_start(window.clone())) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "eostage_account_login_poll" => {
            let arg_session_id: String = extract_cdp_arg(args, "session_id", "sessionId")?;
            match tauri::async_runtime::block_on(eostage_account_login_poll(
                window.clone(),
                arg_session_id,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "eostage_account_login_complete_code" => {
            let arg_code: String = extract_cdp_arg(args, "code", "code")?;
            match tauri::async_runtime::block_on(eostage_account_login_complete_code(
                window.clone(),
                arg_code,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "eostage_account_logout" => {
            match tauri::async_runtime::block_on(eostage_account_logout(window.clone())) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "eostage_account_sync_models" => {
            match tauri::async_runtime::block_on(eostage_account_sync_models(window.clone())) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "eostage_billing_config" => {
            match tauri::async_runtime::block_on(eostage_billing_config(window.clone())) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "eostage_billing_redeem" => {
            let arg_code: String = extract_cdp_arg(args, "code", "code")?;
            match tauri::async_runtime::block_on(eostage_billing_redeem(window.clone(), arg_code)) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "eostage_billing_orders_list" => {
            match tauri::async_runtime::block_on(eostage_billing_orders_list(window.clone())) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "eostage_billing_order_create" => {
            let arg_plan_id: String = extract_cdp_arg(args, "plan_id", "planId")?;
            match tauri::async_runtime::block_on(eostage_billing_order_create(
                window.clone(),
                arg_plan_id,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "eostage_billing_order_check" => {
            let arg_order_no: String = extract_cdp_arg(args, "order_no", "orderNo")?;
            match tauri::async_runtime::block_on(eostage_billing_order_check(
                window.clone(),
                arg_order_no,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "eostage_billing_order_cancel" => {
            let arg_order_no: String = extract_cdp_arg(args, "order_no", "orderNo")?;
            match tauri::async_runtime::block_on(eostage_billing_order_cancel(
                window.clone(),
                arg_order_no,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "eostage_usage_list" => {
            let arg_limit: Option<u32> = extract_cdp_arg(args, "limit", "limit").unwrap_or(None);
            match tauri::async_runtime::block_on(eostage_usage_list(window.clone(), arg_limit)) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "desktop_onboarding_state_get" => {
            match tauri::async_runtime::block_on(desktop_onboarding_state_get(window.clone())) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "desktop_onboarding_state_set" => {
            let arg_completed: bool = extract_cdp_arg(args, "completed", "completed")?;
            match tauri::async_runtime::block_on(desktop_onboarding_state_set(
                window.clone(),
                arg_completed,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "desktop_active_resource_get" => {
            match tauri::async_runtime::block_on(desktop_active_resource_get(window.clone())) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "desktop_active_resource_set" => {
            let arg_resource_id: String = extract_cdp_arg(args, "resource_id", "resourceId")?;
            match tauri::async_runtime::block_on(desktop_active_resource_set(
                window.clone(),
                arg_resource_id,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "transfer_engine_get_prefs" => {
            match crate::transfer_engine_commands::transfer_engine_get_prefs(app.clone()) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::json!({"code": "TRANSFER_ENGINE_ERR", "message": err})),
            }
        }
        "transfer_engine_save_prefs" => {
            let arg_prefs: insar_download::TransferEnginePrefs =
                extract_cdp_arg(args, "prefs", "prefs")?;
            match crate::transfer_engine_commands::transfer_engine_save_prefs(
                app.clone(),
                arg_prefs,
            ) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::json!({"code": "TRANSFER_ENGINE_ERR", "message": err})),
            }
        }
        "application_runtime_components_inspect" => {
            match tauri::async_runtime::block_on(application_runtime_components_inspect(
                window.clone(),
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_extension_registry_fetch" => {
            let arg_registry_url: Option<String> =
                extract_cdp_arg(args, "registry_url", "registryUrl")?;
            match tauri::async_runtime::block_on(application_extension_registry_fetch(
                window.clone(),
                arg_registry_url,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_runtime_component_install" => {
            let arg_request: crate::application::RuntimeComponentInstallRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_runtime_component_install(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_plugins_status" => {
            match tauri::async_runtime::block_on(application_plugins_status(window.clone())) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "plugin_list" => match tauri::async_runtime::block_on(plugin_list(window.clone())) {
            Ok(val) => serde_json::to_value(val)
                .map_err(|e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()})),
            Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
            )),
        },
        "application_plugin_install" => {
            let arg_request: crate::application::PluginInstallRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_plugin_install(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_plugin_set_enabled" => {
            let arg_plugin_id: String = extract_cdp_arg(args, "plugin_id", "pluginId")?;
            let arg_enabled: bool = extract_cdp_arg(args, "enabled", "enabled")?;
            match tauri::async_runtime::block_on(application_plugin_set_enabled(
                window.clone(),
                arg_plugin_id,
                arg_enabled,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_plugin_uninstall" => {
            let arg_plugin_id: String = extract_cdp_arg(args, "plugin_id", "pluginId")?;
            match tauri::async_runtime::block_on(application_plugin_uninstall(
                window.clone(),
                arg_plugin_id,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_custom_connector_save" => {
            let arg_connector: crate::application::CustomConnectorRecord =
                extract_cdp_arg(args, "connector", "connector")?;
            match tauri::async_runtime::block_on(application_custom_connector_save(
                window.clone(),
                arg_connector,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_custom_connector_delete" => {
            let arg_connector_id: String = extract_cdp_arg(args, "connector_id", "connectorId")?;
            match tauri::async_runtime::block_on(application_custom_connector_delete(
                window.clone(),
                arg_connector_id,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_custom_connector_probe_tools" => {
            let arg_connector_id: String = extract_cdp_arg(args, "connector_id", "connectorId")?;
            match tauri::async_runtime::block_on(application_custom_connector_probe_tools(
                window.clone(),
                arg_connector_id,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_custom_skill_save" => {
            let arg_skill: crate::application::CustomSkillRecord =
                extract_cdp_arg(args, "skill", "skill")?;
            match tauri::async_runtime::block_on(application_custom_skill_save(
                window.clone(),
                arg_skill,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_custom_skill_delete" => {
            let arg_skill_id: String = extract_cdp_arg(args, "skill_id", "skillId")?;
            match tauri::async_runtime::block_on(application_custom_skill_delete(
                window.clone(),
                arg_skill_id,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_install_custom_skill_from_url" => {
            let arg_url: String = extract_cdp_arg(args, "url", "url")?;
            let arg_fallback_name: Option<String> = args
                .get("fallback_name")
                .or_else(|| args.get("fallbackName"))
                .and_then(|v| v.as_str())
                .map(str::to_owned);
            match tauri::async_runtime::block_on(application_install_custom_skill_from_url(
                window.clone(),
                arg_url,
                arg_fallback_name,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_install_custom_skill_from_folder" => {
            let arg_folder_path: String = extract_cdp_arg(args, "folder_path", "folderPath")?;
            match tauri::async_runtime::block_on(application_install_custom_skill_from_folder(
                window.clone(),
                arg_folder_path,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_usage_statistics" => {
            match tauri::async_runtime::block_on(application_usage_statistics(window.clone())) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "get_cache_summary" => {
            match tauri::async_runtime::block_on(get_cache_summary(window.clone())) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "clear_disposable_caches" => {
            let arg_categories: Option<Vec<String>> =
                extract_cdp_arg(args, "categories", "categories").unwrap_or(None);
            match tauri::async_runtime::block_on(clear_disposable_caches(
                window.clone(),
                arg_categories,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_gis_raster_inspect" => {
            let arg_request: crate::geodata_tools::GisRasterInspectRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_gis_raster_inspect(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_gis_raster_pixel_probe" => {
            let arg_request: insar_gis::RasterPixelProbeRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_gis_raster_pixel_probe(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_gis_raster_process" => {
            let arg_request: insar_gis::RasterProcessRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_gis_raster_process(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_gis_vector_process" => {
            let arg_request: insar_gis::VectorProcessRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_gis_vector_process(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_external_gis_inspect" => {
            let arg_request: Option<crate::external_gis_bridge::ExternalGisInspectRequest> =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_external_gis_inspect(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_external_gis_save_prefs" => {
            let arg_prefs: crate::external_gis_bridge::ExternalGisPrefs =
                extract_cdp_arg(args, "prefs", "prefs")?;
            match tauri::async_runtime::block_on(application_external_gis_save_prefs(
                window.clone(),
                arg_prefs,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_external_gis_execute" => {
            let arg_request: crate::external_gis_bridge::ExternalGisExecuteRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_external_gis_execute(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_conda_install_geo_package" => {
            let arg_request: crate::external_gis_bridge::CondaGeoInstallRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_conda_install_geo_package(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_lbs_coord_transform" => {
            let arg_request: crate::geodata_tools::LbsCoordTransformRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_lbs_coord_transform(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_mapgis_inspect" => {
            let arg_request: crate::geodata_tools::MapGisInspectRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_mapgis_inspect(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_mapgis_convert" => {
            let arg_request: crate::geodata_tools::MapGisConvertRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_mapgis_convert(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_open_geodata_query" => {
            let arg_request: crate::geodata_tools::OpenGeodataQueryRequest =
                extract_cdp_arg(args, "request", "request")?;
            match tauri::async_runtime::block_on(application_open_geodata_query(
                window.clone(),
                arg_request,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "geodata_fetch_preview_data_url" => {
            let arg_url: String = extract_cdp_arg(args, "url", "url")?;
            match tauri::async_runtime::block_on(geodata_fetch_preview_data_url(
                window.clone(),
                arg_url,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_experiment_project_get" => {
            match tauri::async_runtime::block_on(application_experiment_project_get(window.clone()))
            {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_experiment_project_add_task" => {
            let arg_task_id: String = extract_cdp_arg(args, "task_id", "taskId")?;
            match tauri::async_runtime::block_on(application_experiment_project_add_task(
                window.clone(),
                arg_task_id,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_experiment_project_remove_item" => {
            let arg_item_id: String = extract_cdp_arg(args, "item_id", "itemId")?;
            match tauri::async_runtime::block_on(application_experiment_project_remove_item(
                window.clone(),
                arg_item_id,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "application_experiment_project_export" => {
            let arg_export_dir: Option<String> = extract_cdp_arg(args, "export_dir", "exportDir")?;
            match tauri::async_runtime::block_on(application_experiment_project_export(
                window.clone(),
                arg_export_dir,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "shell_diagnostics_get_state" => {
            match tauri::async_runtime::block_on(shell_diagnostics_get_state(window.clone())) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "shell_diagnostics_set_enabled" => {
            let arg_enabled: bool = extract_cdp_arg(args, "enabled", "enabled")?;
            match tauri::async_runtime::block_on(shell_diagnostics_set_enabled(
                window.clone(),
                arg_enabled,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "shell_diagnostics_record" => {
            let arg_input: crate::diagnostics::DiagnosticsRecordInput =
                extract_cdp_arg(args, "input", "input")?;
            match tauri::async_runtime::block_on(shell_diagnostics_record(
                window.clone(),
                arg_input,
            )) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "shell_diagnostics_list" => {
            match tauri::async_runtime::block_on(shell_diagnostics_list(window.clone())) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "shell_diagnostics_clear" => {
            match tauri::async_runtime::block_on(shell_diagnostics_clear(window.clone())) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "shell_diagnostics_upload" => {
            match tauri::async_runtime::block_on(shell_diagnostics_upload(window.clone())) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        "shell_check_app_update" => {
            match tauri::async_runtime::block_on(shell_check_app_update(window.clone())) {
                Ok(val) => serde_json::to_value(val).map_err(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                ),
                Err(err) => Err(serde_json::to_value(err).unwrap_or_else(
                    |e| serde_json::json!({"code": "SERDE_ERR", "message": e.to_string()}),
                )),
            }
        }
        other => Err(
            serde_json::json!({"code": "UNKNOWN_CMD", "message": format!("unknown command: {other}")}),
        ),
    }
}
// --- END AUTO-GENERATED MACOS CDP DISPATCH ---

#[cfg(test)]
mod commands_error_mapping_tests {
    use super::application_error;
    use insar_application_service::ApplicationServiceError;

    #[test]
    fn application_error_maps_dem_native_gis_capability_unavailable_truthfully() {
        let shell = application_error(ApplicationServiceError::CapabilityUnavailable {
            capability: "formal_dem_download",
        });
        assert_eq!(shell.code, "V3_STORE_READINESS_UNSUPPORTED");
        assert_eq!(
            shell.message_key,
            "application_service.dem_native_gis_unavailable"
        );
        assert_eq!(shell.context.as_deref(), Some("formal_dem_download"));
    }

    #[test]
    fn application_error_maps_dem_credential_unavailable_without_blaming_gdal() {
        let shell = application_error(ApplicationServiceError::CapabilityUnavailable {
            capability: "formal_dem_credential",
        });
        assert_eq!(shell.code, "V3_STORE_READINESS_UNSUPPORTED");
        assert_eq!(
            shell.message_key,
            "application_service.dem_credential_unavailable"
        );
        assert_eq!(shell.context.as_deref(), Some("formal_dem_credential"));
    }
}
