use std::path::{Path, PathBuf};

use insar_application_service::DemTaskSettings;
use insar_download::{
    load_and_activate_transfer_engine_prefs_in_dir, save_and_activate_transfer_engine_prefs_in_dir,
    set_active_transfer_engine_prefs, transfer_engine_prefs::normalize_max_concurrent_tasks,
    transfer_engine_prefs_path, TransferEnginePrefs,
};
use tauri::Manager;

use crate::application::{resolve_app_data_dir, ApplicationServiceState};

fn resolve_command_app_data_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    if let Some(state) = app.try_state::<ApplicationServiceState>() {
        return Ok(resolve_app_data_dir(&state));
    }
    app.path()
        .app_data_dir()
        .map_err(|err| format!("failed to resolve app data directory: {err}"))
}

/// Synchronizes Tier-1 task concurrency across DEM and SAR download policy sidecars.
pub fn sync_concurrency_to_application_service(
    service: &ApplicationServiceState,
    limit: u8,
) -> Result<(), String> {
    let clamped = normalize_max_concurrent_tasks(limit);
    let limit_u64 = u64::from(clamped);

    let current_dem = service
        .get_dem_task_settings()
        .map_err(|err| format!("failed to read DEM task settings: {err:?}"))?;
    if current_dem.max_concurrent_tasks != limit_u64 || current_dem.max_concurrent_conversions != 1
    {
        service
            .save_dem_task_settings(&DemTaskSettings {
                max_concurrent_tasks: limit_u64,
                max_concurrent_conversions: 1,
            })
            .map_err(|err| format!("failed to sync DEM task settings: {err:?}"))?;
    }

    let mut current_sar = service
        .get_sar_download_policy_settings()
        .map_err(|err| format!("failed to read SAR download policy settings: {err:?}"))?;
    if current_sar.max_concurrent_items != limit_u64 {
        current_sar.max_concurrent_items = limit_u64;
        service
            .save_sar_download_policy_settings(&current_sar)
            .map_err(|err| format!("failed to sync SAR download policy settings: {err:?}"))?;
    }

    Ok(())
}

/// Loads [`TransferEnginePrefs`] from `dir`, reconciles legacy SAR/DEM sidecar limits on first
/// run when `transfer_engine_prefs.json` does not yet exist, and synchronizes all sidecars.
pub fn reconcile_startup_transfer_prefs_with_service(
    dir: &Path,
    service: Option<&ApplicationServiceState>,
) -> Result<TransferEnginePrefs, String> {
    let prefs_file_exists = transfer_engine_prefs_path(dir).exists();
    let mut prefs = load_and_activate_transfer_engine_prefs_in_dir(dir)?;
    let Some(svc) = service else {
        return Ok(prefs);
    };

    if !prefs_file_exists {
        let sar_limit = svc.get_sar_download_policy_settings().ok().map(|p| {
            normalize_max_concurrent_tasks(u8::try_from(p.max_concurrent_items).unwrap_or(8))
        });
        let dem_limit = svc.get_dem_task_settings().ok().map(|d| {
            normalize_max_concurrent_tasks(u8::try_from(d.max_concurrent_tasks).unwrap_or(8))
        });
        let adopted = match (sar_limit, dem_limit) {
            (Some(sar), _) if sar != prefs.max_concurrent_tasks => sar,
            (_, Some(dem)) if dem != prefs.max_concurrent_tasks => dem,
            _ => prefs.max_concurrent_tasks,
        };
        if adopted != prefs.max_concurrent_tasks {
            prefs.max_concurrent_tasks = adopted;
            prefs = set_active_transfer_engine_prefs(prefs);
        }
    }

    sync_concurrency_to_application_service(svc, prefs.max_concurrent_tasks)?;
    Ok(prefs)
}

/// Loads [`TransferEnginePrefs`] from `dir` and synchronizes the active process snapshot.
#[allow(dead_code)]
pub fn load_transfer_engine_prefs_in_dir(dir: &Path) -> Result<TransferEnginePrefs, String> {
    load_and_activate_transfer_engine_prefs_in_dir(dir)
}

/// Normalizes, atomically persists, and activates [`TransferEnginePrefs`] inside `dir`.
pub fn save_transfer_engine_prefs_in_dir(
    dir: &Path,
    prefs: TransferEnginePrefs,
) -> Result<TransferEnginePrefs, String> {
    save_and_activate_transfer_engine_prefs_in_dir(dir, prefs)
}

/// Reads persisted download engine preferences and activates them for the current session.
#[tauri::command]
pub fn transfer_engine_get_prefs(app: tauri::AppHandle) -> Result<TransferEnginePrefs, String> {
    let app_data_dir = resolve_command_app_data_dir(&app)?;
    let service = app.try_state::<ApplicationServiceState>();
    reconcile_startup_transfer_prefs_with_service(&app_data_dir, service.as_deref())
}

/// Persists updated download engine preferences and applies them to active transfers and sidecars.
#[tauri::command]
pub fn transfer_engine_save_prefs(
    app: tauri::AppHandle,
    prefs: TransferEnginePrefs,
) -> Result<TransferEnginePrefs, String> {
    let app_data_dir = resolve_command_app_data_dir(&app)?;
    let saved = save_transfer_engine_prefs_in_dir(&app_data_dir, prefs)?;
    if let Some(svc) = app.try_state::<ApplicationServiceState>() {
        sync_concurrency_to_application_service(&svc, saved.max_concurrent_tasks)?;
    }
    Ok(saved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use insar_core::{SarDownloadPolicyMode, SarDownloadPolicySettings};
    use std::sync::Mutex;

    static TEST_PREFS_LOCK: Mutex<()> = Mutex::new(());

    struct TestPrefsGuard(PathBuf);

    impl Drop for TestPrefsGuard {
        fn drop(&mut self) {
            insar_download::clear_active_transfer_engine_prefs_override();
            insar_download::set_active_default_ssd_staging_dir(None);
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn make_temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "eostage-transfer-cmd-{label}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn transfer_engine_commands_roundtrip_and_default_ssd_staging() {
        let _lock = TEST_PREFS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = make_temp_dir("roundtrip");
        let _guard = TestPrefsGuard(dir.clone());

        let custom = TransferEnginePrefs {
            max_concurrent_tasks: 6,
            smart_concurrency: true,
            per_file_segments: 8,
            tile_chunk_concurrency: 32,
            stall_reconnect_enabled: false,
            ssd_staging_enabled: false,
            ssd_staging_auto_slow_disk: true,
            ssd_staging_dir: None,
            max_download_speed_kib_s: 5120,
        };
        let saved = save_transfer_engine_prefs_in_dir(&dir, custom).unwrap();
        assert_eq!(saved.max_concurrent_tasks, 6);
        assert_eq!(saved.tile_chunk_concurrency, 32);
        assert!(!saved.stall_reconnect_enabled);
        assert_eq!(saved.max_download_speed_kib_s, 5120);
        assert_eq!(
            insar_download::active_default_ssd_staging_dir(),
            Some(insar_download::default_ssd_staging_dir_for_app_data(&dir))
        );

        let loaded = load_transfer_engine_prefs_in_dir(&dir).unwrap();
        assert_eq!(loaded, saved);
        assert_eq!(insar_download::active_transfer_engine_prefs(), saved);

        // Simulate app restart reconciliation: global bandwidth limiter must be restored to 5120 KiB/s.
        insar_download::clear_active_transfer_engine_prefs_override();
        assert_eq!(insar_download::global_bandwidth_limiter().limit_bps(), 0);
        let state = ApplicationServiceState::with_store_path(dir.join("insar_tasks_v6.sqlite3"));
        let reloaded = reconcile_startup_transfer_prefs_with_service(&dir, Some(&state)).unwrap();
        assert_eq!(reloaded.max_download_speed_kib_s, 5120);
        assert_eq!(
            insar_download::global_bandwidth_limiter().limit_bps(),
            5120 * 1024
        );

        // Saving 0 clears the global bandwidth limiter back to unlimited.
        let cleared = save_transfer_engine_prefs_in_dir(
            &dir,
            TransferEnginePrefs {
                max_download_speed_kib_s: 0,
                ..reloaded
            },
        )
        .unwrap();
        assert_eq!(cleared.max_download_speed_kib_s, 0);
        let after_reset =
            reconcile_startup_transfer_prefs_with_service(&dir, Some(&state)).unwrap();
        assert_eq!(after_reset.max_download_speed_kib_s, 0);
        assert_eq!(insar_download::global_bandwidth_limiter().limit_bps(), 0);
    }

    #[test]
    fn fresh_install_reconciles_all_three_sidecars_to_two() {
        let _lock = TEST_PREFS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = make_temp_dir("fresh");
        let _guard = TestPrefsGuard(dir.clone());
        let state = ApplicationServiceState::with_store_path(dir.join("insar_tasks_v6.sqlite3"));

        let prefs = reconcile_startup_transfer_prefs_with_service(&dir, Some(&state)).unwrap();
        assert_eq!(prefs.max_concurrent_tasks, 2);
        assert_eq!(
            state.get_dem_task_settings().unwrap().max_concurrent_tasks,
            2
        );
        assert_eq!(
            state
                .get_sar_download_policy_settings()
                .unwrap()
                .max_concurrent_items,
            2
        );
    }

    #[test]
    fn pre_existing_sar_policy_five_is_adopted_when_transfer_prefs_missing() {
        let _lock = TEST_PREFS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = make_temp_dir("upgrade");
        let _guard = TestPrefsGuard(dir.clone());
        let state = ApplicationServiceState::with_store_path(dir.join("insar_tasks_v6.sqlite3"));
        let expected_root = dir.join("custom_downloads").to_string_lossy().to_string();
        state
            .save_sar_download_policy_settings(&SarDownloadPolicySettings {
                mode: SarDownloadPolicyMode::Manual,
                max_concurrent_items: 5,
                organize_by_product: true,
                default_output_root: expected_root.clone(),
            })
            .unwrap();

        let prefs = reconcile_startup_transfer_prefs_with_service(&dir, Some(&state)).unwrap();
        assert_eq!(prefs.max_concurrent_tasks, 5);
        assert_eq!(
            insar_download::active_transfer_engine_prefs().max_concurrent_tasks,
            5
        );
        assert_eq!(
            state.get_dem_task_settings().unwrap().max_concurrent_tasks,
            5
        );
        let sar = state.get_sar_download_policy_settings().unwrap();
        assert_eq!(sar.max_concurrent_items, 5);
        assert!(sar.organize_by_product);
        assert_eq!(sar.default_output_root, expected_root);
    }

    #[test]
    fn sync_concurrency_updates_both_dem_and_sar_preserving_output_root() {
        let _lock = TEST_PREFS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = make_temp_dir("sync");
        let _guard = TestPrefsGuard(dir.clone());
        let state = ApplicationServiceState::with_store_path(dir.join("insar_tasks_v6.sqlite3"));
        let preserved_root = dir.join("preserved_out").to_string_lossy().to_string();
        state
            .save_sar_download_policy_settings(&SarDownloadPolicySettings {
                mode: SarDownloadPolicyMode::Manual,
                max_concurrent_items: 2,
                organize_by_product: true,
                default_output_root: preserved_root.clone(),
            })
            .unwrap();

        let saved = save_transfer_engine_prefs_in_dir(
            &dir,
            TransferEnginePrefs {
                max_concurrent_tasks: 6,
                ..TransferEnginePrefs::default()
            },
        )
        .unwrap();
        sync_concurrency_to_application_service(&state, saved.max_concurrent_tasks).unwrap();

        assert_eq!(
            state.get_dem_task_settings().unwrap().max_concurrent_tasks,
            6
        );
        let sar = state.get_sar_download_policy_settings().unwrap();
        assert_eq!(sar.max_concurrent_items, 6);
        assert!(sar.organize_by_product);
        assert_eq!(sar.default_output_root, preserved_root);
    }
}
