fn command_list(source: &str, function_name: &str) -> Vec<String> {
    let function_start = source
        .find(&format!("fn {function_name}"))
        .expect("single desktop runtime function exists");
    let handler_start = source[function_start..]
        .find("tauri::generate_handler![")
        .map(|offset| function_start + offset)
        .expect("single desktop runtime registers commands");
    let handler_end = source[handler_start..]
        .find("])")
        .map(|offset| handler_start + offset)
        .expect("single desktop command handler closes");
    let mut pending_feature = None;
    let mut registered = Vec::new();
    for line in source[handler_start..handler_end].lines() {
        let line = line.trim();
        if let Some(feature) = line
            .strip_prefix("#[cfg(feature = \"")
            .and_then(|rest| rest.strip_suffix("\")]"))
        {
            pending_feature = Some(feature);
            continue;
        }
        assert!(
            !line.starts_with("#[cfg("),
            "unhandled conditional command guard: {line}"
        );
        if let Some(command) = line.strip_prefix("commands::") {
            let enabled = match pending_feature.take() {
                Some("bhoonidhi-auth-revoke-acceptance") => {
                    cfg!(feature = "bhoonidhi-auth-revoke-acceptance")
                }
                Some(other) => panic!("unhandled conditional command feature: {other}"),
                None => true,
            };
            if enabled {
                registered.push(command.trim_end_matches(',').to_owned());
            }
        } else if !line.is_empty() {
            pending_feature = None;
        }
    }
    registered
}

fn command_block<'a>(source: &'a str, command: &str) -> &'a str {
    let sync_marker = format!("pub fn {command}");
    let async_marker = format!("pub async fn {command}");
    let start = source
        .find(&async_marker)
        .or_else(|| source.find(&sync_marker))
        .expect("command adapter exists");
    let body_end = source[start..]
        .find("\n}")
        .map(|offset| start + offset + 2)
        .expect("command adapter closes");
    &source[start..body_end]
}

fn contains_typed_command(source: &str, command: &str) -> bool {
    source.contains(&format!("pub fn {command}"))
        || source.contains(&format!("pub async fn {command}"))
}

#[test]
fn catalogue_selection_and_task_store_commands_do_not_wait_on_the_window_thread() {
    let commands = include_str!("../src/commands.rs");
    for command in [
        "application_get_dem_task_settings",
        "application_save_dem_task_settings",
        "application_standalone_dem",
        "application_core_restore",
        "application_workbench_page",
        "application_download_task_page",
        "application_load_task_source_batch_page",
        "application_download_history_page",
        "application_download_task_event_page",
        "application_list_task_conflict_files",
        "application_delete_task_conflict_files",
        "application_update_workbench_selection",
        "application_create_landsat_bundle_task",
        "application_create_landsat_asset_task",
        "application_create_landsat_asset_batch_task",
        "application_create_landsat_bundle_batch_task",
        "application_task_delivery_admission",
        "application_optical_preparation_progress",
        "application_create_reviewed_mpc_asset_task",
        "application_create_sentinel2_archive_batch_task",
        "application_create_sentinel2_archive_task",
        "application_create_sentinel2_band_task",
        "application_create_sentinel2_band_batch_task",
        "application_create_sentinel2_l1c_band_batch_task",
        "application_create_hls_band_batch_task",
        "application_precheck_optical_aoi_coverage",
        "application_create_optical_derived_task",
        "application_create_download_task",
        "application_create_task_from_workbench",
    ] {
        let body = command_block(commands, command);
        assert!(
            body.starts_with(&format!("pub async fn {command}")),
            "{command} blocks the window thread"
        );
        assert!(
            body.contains("tauri::async_runtime::spawn_blocking(move ||"),
            "{command} must offload actual Store/network work, not only become async"
        );
        assert!(body.contains("validate_application_window(&window)?"));
        assert!(body.contains("map_err(application_error)"));
    }
}

#[test]
fn account_edit_and_status_commands_keep_os_and_network_work_off_the_window_thread() {
    let commands = include_str!("../src/commands.rs");
    for name in [
        "credential_status",
        "credential_read_for_edit",
        "credential_agent_channel_key_read",
        "credential_update_field",
        "credential_earthdata_auth",
    ] {
        let body = command_block(commands, name);
        assert!(body.starts_with(&format!("pub async fn {name}")));
        assert!(body.contains("validate_application_window(&window)?"));
        assert!(body.contains("tauri::async_runtime::spawn_blocking"));
    }
    let edit = command_block(commands, "credential_update_field");
    assert!(edit.contains("field: insar_download::DesktopCredentialField"));
    assert!(edit.contains("DesktopCredentialEditAcknowledgement { ok: true }"));
    assert!(!edit.contains("credential_status_payload"));
    assert!(!edit.contains("pub service:"));
}

#[test]
fn optical_preview_and_asset_reads_do_not_block_the_window_thread() {
    let commands = include_str!("../src/commands.rs");
    for (command, service_call) in [
        ("application_preview_asset", ".preview_asset(request)"),
        ("application_cdse_quicklook", ".cdse_quicklook(request)"),
        (
            "application_sentinel2_map_preview",
            ".sentinel2_map_preview(request)",
        ),
        ("application_landsat_browse", ".landsat_browse(request)"),
        (
            "application_landsat_asset_details",
            ".landsat_asset_details(request)",
        ),
        (
            "application_bhoonidhi_asset_size",
            ".bhoonidhi_asset_size(request)",
        ),
    ] {
        let body = command_block(commands, command);
        assert!(
            body.starts_with(&format!("pub async fn {command}")),
            "{command} blocks the window thread"
        );
        let offload = body
            .find("tauri::async_runtime::spawn_blocking(move ||")
            .expect("blocking optical I/O must use the existing background boundary");
        let service = body
            .find(service_call)
            .expect("the actual provider call is retained");
        let await_result = body.find(".await").expect("background result is awaited");
        assert!(
            offload < service && service < await_result,
            "{command} must offload the real provider call"
        );
        assert!(body[..offload].contains("validate_application_window(&window)?"));
        assert!(body.contains("map_err(application_error)"));
    }
}

#[test]
fn administrative_provider_commands_run_blocking_clients_off_the_async_runtime() {
    let commands = include_str!("../src/commands.rs");
    for command in [
        "application_admin_boundary_search",
        "application_admin_boundary_resolve",
        "application_admin_options",
        "application_geo_boundary_countries",
        "application_geo_boundary_levels",
        "application_geo_boundary_units",
        "application_geo_boundary_resolve",
    ] {
        let body = command_block(commands, command);
        assert!(
            body.starts_with(&format!("pub async fn {command}")),
            "{command} must be an async IPC adapter"
        );
        assert!(
            body.contains("tauri::async_runtime::spawn_blocking"),
            "{command} must keep reqwest::blocking off Tauri's async runtime"
        );
    }
    assert!(!commands.contains("#[tauri::command(async)]\npub fn application_admin_boundary_"));
}

#[test]
fn saved_cdse_and_usgs_credentials_are_resolved_only_at_the_native_login_boundary() {
    let commands = include_str!("../src/commands.rs");
    let credentials = include_str!("../../crates/insar-download/src/credentials.rs");
    for (command, resolver) in [
        ("application_cdse_login", "resolve_desktop_cdse_login"),
        ("application_usgs_login", "resolve_desktop_usgs_login"),
    ] {
        let body = command_block(commands, command);
        assert!(body.contains("validate_application_window(&window)?"));
        assert!(
            body.contains(resolver),
            "{command} must resolve saved material natively"
        );
        assert!(
            body.contains("username.trim().is_empty()"),
            "{command} may use saved material only for an explicitly empty login request"
        );
        assert!(
            body.contains("map_err(credential_error)?"),
            "{command} must fail closed when the credential store cannot resolve a complete login"
        );
    }
    assert!(
        credentials.contains("let _mutation = desktop_login_pair_write_guard()?;"),
        "CDSE/USGS pair save and clear must serialize native credential mutations"
    );
    assert!(
        credentials.contains("let _guard = desktop_login_pair_read_guard()?;"),
        "CDSE/USGS native login resolution must not observe a half-written pair"
    );
    assert!(
        credentials.contains("Some(desktop_login_pair_read_guard()?)"),
        "explicit saved-value reveal must share the CDSE/USGS pair read boundary"
    );
}

#[test]
fn gacos_async_commands_are_typed_blocking_adapters() {
    let commands = include_str!("../src/commands.rs");
    for command in [
        "application_gacos_submit_request",
        "application_gacos_request_status",
        "application_gacos_create_import_task",
        "application_gacos_create_result_task",
        "application_gacos_supply_result_link",
    ] {
        let body = command_block(commands, command);
        assert!(
            body.starts_with(&format!("pub async fn {command}")),
            "{command} must be an async IPC adapter"
        );
        assert!(
            body.contains("tauri::async_runtime::spawn_blocking(move ||"),
            "{command} must keep blocking application work off Tauri's async runtime"
        );
        assert!(body.contains("validate_application_window(&window)?"));
        assert!(body.contains("map_err(application_error)"));
    }
}

#[test]
fn normal_run_registers_the_complete_named_application_surface_once() {
    let library = include_str!("../src/lib.rs");
    let commands = include_str!("../src/commands.rs");
    let registered = command_list(library, "run_desktop_with_application_service");
    let mut expected = vec![
        "shell_app_info",
        "shell_window_toggle_maximize",
        "application_runtime_components_inspect",
        "application_extension_registry_fetch",
        "application_runtime_component_install",
        "application_plugins_status",
        "plugin_list",
        "application_plugin_install",
        "application_plugin_set_enabled",
        "application_plugin_uninstall",
        "application_custom_connector_save",
        "application_custom_connector_delete",
        "application_custom_connector_probe_tools",
        "application_custom_skill_save",
        "application_custom_skill_delete",
        "application_install_custom_skill_from_url",
        "application_install_custom_skill_from_folder",
        "application_usage_statistics",
        "get_cache_summary",
        "clear_disposable_caches",
        "application_gis_raster_inspect",
        "application_gis_raster_pixel_probe",
        "application_gis_raster_process",
        "application_gis_vector_process",
        "application_external_gis_inspect",
        "application_external_gis_save_prefs",
        "application_external_gis_execute",
        "application_conda_install_geo_package",
        "application_lbs_coord_transform",
        "application_mapgis_inspect",
        "application_mapgis_convert",
        "application_open_geodata_query",
        "geodata_fetch_preview_data_url",
        "application_experiment_project_get",
        "application_experiment_project_add_task",
        "application_experiment_project_remove_item",
        "application_experiment_project_export",
        "application_service_readiness",
        "application_core_restore",
        "application_task_overview",
        "application_workbench_page",
        "application_batch_page",
        "application_download_task_page",
        "application_load_task_source_batch_page",
        "application_download_history_page",
        "application_download_task_event_page",
        "application_open_download_task_output",
        "application_list_task_conflict_files",
        "application_delete_task_conflict_files",
        "application_open_external_url",
        "application_aoi_import",
        "application_aoi_normalize",
        "application_admin_boundary_search",
        "application_admin_boundary_resolve",
        "application_admin_options",
        "application_geo_boundary_countries",
        "application_geo_boundary_levels",
        "application_geo_boundary_units",
        "application_geo_boundary_resolve",
        "application_gacos_plan",
        "application_gacos_dry_run",
        "application_gacos_submit_request",
        "application_gacos_request_status",
        "application_gacos_ledger",
        "application_gacos_import_result",
        "application_gacos_create_import_task",
        "application_gacos_create_result_task",
        "application_gacos_supply_result_link",
        "application_report_generate",
        "application_update_workbench_selection",
        "application_workspace_open",
        "application_workspace_create",
        "application_workspace_prepare_location",
        "application_project_create",
        "application_project_select",
        "application_region_create",
        "application_region_select",
        "credential_status",
        "credential_bhoonidhi_auth",
        "credential_bhoonidhi_session_status",
        "credential_reveal",
        "credential_read_for_edit",
        "credential_agent_channel_key_read",
        "credential_update_field",
        "credential_earthdata_auth",
        "credential_save_cdse_login",
        "credential_clear_cdse",
        "credential_save_usgs_login",
        "credential_clear_usgs",
        "credential_save_earthdata_token",
        "credential_save_earthdata_login",
        "credential_clear_earthdata",
        "credential_save_bhoonidhi_login",
        "credential_clear_bhoonidhi",
        "credential_save_opentopography_key",
        "credential_clear_opentopography_key",
        "credential_save_gacos_email",
        "credential_clear_gacos_email",
        "application_bind_region_aoi",
        "application_clear_region_aoi",
        "application_set_working_aoi",
        "application_clear_working_aoi",
        "application_working_aoi",
        "application_plan_dem_task",
        "application_create_dem_task",
        "application_set_dem_source_identity",
        "application_resolve_dem_conversion_source",
        "application_handoff_dem_conversion",
        "application_get_dem_task_settings",
        "application_save_dem_task_settings",
        "application_get_sar_download_policy_settings",
        "application_save_sar_download_policy_settings",
        "application_get_network_settings",
        "application_save_network_settings",
        "application_test_network_connection",
        "application_probe_provider_connectivity",
        "application_standalone_dem",
        "application_create_standalone_dem_task",
        "application_import_scenes",
        "application_match_orbits_directory",
        "application_create_orbit_task",
        "application_create_standalone_orbit_task",
        "application_create_companion_orbit_task",
        "application_list_sources",
        "application_source_views",
        "application_provider_capabilities",
        "application_effective_capability",
        "application_search_start",
        "application_search_progress",
        "application_search_cancel",
        "application_sar_search_and_freeze",
        "application_sar_search_progress",
        "application_sar_background_count",
        "application_catalog_search_and_freeze",
        "application_catalog_intent_search_and_freeze",
        "application_cancel_request",
        "application_cancel_catalog_search",
        "application_account_session_statuses",
        "application_cdse_session_status",
        "application_cdse_login",
        "application_cdse_logout",
        "application_cdse_network_policy",
        "application_set_cdse_network_policy",
        "application_cdse_network_diagnostic",
        "application_preview_asset",
        "application_save_preview_asset",
        "application_cdse_quicklook",
        "application_sentinel2_map_preview",
        "application_usgs_session_status",
        "application_usgs_login",
        "application_usgs_logout",
        "application_landsat_browse",
        "application_landsat_asset_details",
        "application_bhoonidhi_asset_size",
        "application_create_landsat_bundle_task",
        "application_create_landsat_asset_task",
        "application_create_landsat_asset_batch_task",
        "application_create_landsat_bundle_batch_task",
        "application_task_delivery_admission",
        "application_optical_preparation_progress",
        "application_create_reviewed_mpc_asset_task",
        "application_create_sentinel2_archive_batch_task",
        "application_create_sentinel2_archive_task",
        "application_create_sentinel2_band_task",
        "application_create_sentinel2_band_batch_task",
        "application_create_sentinel2_l1c_band_batch_task",
        "application_create_hls_band_batch_task",
        "application_precheck_optical_aoi_coverage",
        "application_create_optical_derived_task",
        "application_nisar_remote_subset_capability",
        "application_create_nisar_remote_subset_task",
        "application_prepare_nisar_subset_intent_v2",
        "application_confirm_nisar_subset_task_v2",
        "application_preflight_nisar_subset_batch_v2",
        "application_prepare_nisar_subset_batch_v2",
        "application_confirm_nisar_subset_batch_task_v2",
        "application_append_nisar_subset_batch_task_v2",
        "application_read_nisar_subset_append_context_v2",
        "application_preflight_nisar_subset_append_v2",
        "application_create_nisar_subset_successor_task_v2",
        "application_read_nisar_archived_recovery_context_v2",
        "application_create_nisar_archived_recovery_v2",
        "application_resume_nisar_subset_task",
        "application_start_newly_confirmed_task",
        "application_update_search_workbench",
        "application_create_task",
        "application_preview_all_match_task",
        "application_create_all_match_task",
        "application_create_download_task",
        "application_append_download_task",
        "application_create_task_from_workbench",
        "application_append_task_from_workbench",
        "application_query_source_append_status",
        "application_query_historical_successor_status",
        "application_query_burst_append_status",
        "application_query_burst_successor_status",
        "application_query_nisar_append_status",
        "application_query_nisar_successor_status",
        "application_query_nisar_archived_recovery_status",
        "application_query_reacquire_status",
        "application_append_task_from_source_batch",
        "application_create_task_from_historical_source_batch",
        "application_create_burst_asset_task",
        "application_append_burst_asset_task",
        "application_create_historical_burst_asset_task",
        "application_task_action",
        "application_task_retry_fallback",
        "application_update_download_task",
        "application_update_download_task_view",
        "application_archive_download_task",
        "application_delete_download_history",
        "application_task_item_action",
        "application_task_delete_history",
        "application_task_set_pinned",
        "application_list_pinned_tasks",
        "application_task_retry_fallback_eligibility",
        "check_existing_downloaded_scenes",
        "agent_sessions_list",
        "agent_sessions_create",
        "agent_sessions_delete",
        "agent_sessions_rename",
        "agent_sessions_set_pinned",
        "agent_sessions_set_archived",
        "agent_sessions_move",
        "agent_sessions_export",
        "agent_sessions_export_to",
        "agent_workspaces_list",
        "agent_workspaces_create",
        "agent_workspaces_update",
        "agent_workspaces_delete",
        "agent_send",
        "agent_stream_poll",
        "agent_plan_approve",
        "agent_plan_reject",
        "agent_run_cancel",
        "agent_runs_list",
        "agent_session_work",
        "agent_session_truncate",
        "agent_session_workspace_state",
        "agent_session_set_working_aoi",
        "agent_runs_open",
        "agent_channels_get",
        "agent_disabled_items_get",
        "agent_disabled_items_set",
        "agent_channels_save",
        "agent_channels_test",
        "agent_channels_models",
        "agent_channels_probe_draft",
        "agent_channels_test_draft",
        "agent_channels_set_default",
        "agent_channels_delete",
        "agent_channel_rename",
        "agent_sync_public_model_registry",
        "agent_oauth_open_browser",
        "agent_oauth_device_start",
        "agent_oauth_device_poll",
        "agent_oauth_pkce_start",
        "agent_oauth_pkce_complete",
        "agent_oauth_channels_meta",
        "agent_oauth_channel_quota",
        "agent_permission_mode_get",
        "agent_permission_mode_set_global",
        "agent_permission_mode_set_session",
        "agent_stop_all",
        "agent_activity_status",
        "eostage_account_status",
        "eostage_account_login_start",
        "eostage_account_login_poll",
        "eostage_account_login_complete_code",
        "eostage_account_logout",
        "eostage_account_sync_models",
        "eostage_billing_config",
        "eostage_billing_redeem",
        "eostage_billing_orders_list",
        "eostage_billing_order_create",
        "eostage_billing_order_check",
        "eostage_billing_order_cancel",
        "eostage_usage_list",
        "desktop_onboarding_state_get",
        "desktop_onboarding_state_set",
        "desktop_active_resource_get",
        "desktop_active_resource_set",
        "shell_diagnostics_get_state",
        "shell_diagnostics_set_enabled",
        "shell_diagnostics_record",
        "shell_diagnostics_list",
        "shell_diagnostics_clear",
        "shell_diagnostics_upload",
        "shell_check_app_update",
        "ui_smoke_status",
        "ui_smoke_report",
        "ui_smoke_phase",
        "ui_smoke_checkpoint",
        "ui_smoke_phase_a_complete",
        "ui_smoke_captured_search_query",
    ];
    if cfg!(feature = "bhoonidhi-auth-revoke-acceptance") {
        let after_session_status = expected
            .iter()
            .position(|command| *command == "credential_bhoonidhi_session_status")
            .expect("Bhoonidhi session status command is registered")
            + 1;
        expected.splice(
            after_session_status..after_session_status,
            [
                "acceptance_bhoonidhi_auth_generation",
                "acceptance_bhoonidhi_auth_revoke_once",
            ],
        );
    }
    assert_eq!(registered, expected);
    assert_eq!(registered.len(), expected.len());

    for command in expected {
        assert!(
            contains_typed_command(commands, command),
            "missing typed Tauri command: {command}"
        );
    }
}

#[test]
fn normal_run_registers_only_bounded_restore_commands() {
    let library = include_str!("../src/lib.rs");
    let registered = command_list(library, "run_desktop_with_application_service");
    assert!(registered
        .iter()
        .any(|name| name == "application_core_restore"));
    assert!(registered
        .iter()
        .any(|name| name == "application_workbench_page"));
    assert!(!registered
        .iter()
        .any(|name| name == "application_service_restore"));
    assert!(!registered
        .iter()
        .any(|name| name == "application_download_snapshot"));
    assert!(!library.contains("application_service_restore"));
    assert!(!library.contains("application_download_snapshot"));
}

#[test]
fn bounded_restore_and_page_commands_are_thin_typed_adapters() {
    let commands = include_str!("../src/commands.rs");
    let application = include_str!("../src/application.rs");

    assert!(contains_typed_command(commands, "application_core_restore"));
    assert!(contains_typed_command(
        commands,
        "application_workbench_page"
    ));
    assert!(commands.contains("Result<CoreServiceSnapshot, ShellError>"));
    assert!(commands
        .contains("Result<insar_application_service::ApplicationWorkbenchPage, ShellError>"));
    assert!(application.contains("pub fn core_restore"));
    assert!(application.contains("pub fn workbench_page"));
    assert!(contains_typed_command(
        commands,
        "application_update_workbench_selection"
    ));
    assert!(application.contains("pub fn update_workbench_selection"));
    assert!(contains_typed_command(
        commands,
        "application_create_task_from_workbench"
    ));
    assert!(commands.contains("pub fn application_append_task_from_workbench"));
    assert!(!commands.contains("pub fn application_download_snapshot"));
    assert!(!commands.contains("pub fn application_service_restore"));

    for command in [
        "application_aoi_import",
        "application_aoi_normalize",
        "application_admin_boundary_search",
        "application_admin_boundary_resolve",
        "application_admin_options",
        "application_gacos_plan",
        "application_gacos_dry_run",
        "application_gacos_submit_request",
        "application_gacos_request_status",
        "application_gacos_create_import_task",
        "application_gacos_create_result_task",
        "application_gacos_supply_result_link",
        "application_report_generate",
    ] {
        assert!(contains_typed_command(commands, command));
        assert!(commands.contains("validate_application_window(&window)?"));
        assert!(commands.contains("map_err(application_error)"));
    }

    for command in ["application_core_restore", "application_workbench_page"] {
        let body = command_block(commands, command);
        assert!(body.contains("validate_application_window(&window)?"));
        assert!(body.contains("map_err(application_error)"));
        assert!(!body.contains("compatibility_worker_call"));
        assert!(!body.contains("snapshot("));
    }

    {
        let command = "application_create_sentinel2_band_task";
        let body = command_block(commands, command);
        assert!(body.contains("validate_application_window(&window)?"));
        assert!(body.contains("map_err(application_error)"));
        assert!(!body.contains("url"));
        assert!(!body.contains("credential"));
    }
    assert!(application.contains("pub fn create_sentinel2_band_task"));

    for command in [
        "application_preflight_nisar_subset_batch_v2",
        "application_prepare_nisar_subset_batch_v2",
        "application_confirm_nisar_subset_batch_task_v2",
        "application_append_nisar_subset_batch_task_v2",
        "application_read_nisar_subset_append_context_v2",
        "application_preflight_nisar_subset_append_v2",
        "application_create_nisar_subset_successor_task_v2",
        "application_resume_nisar_subset_task",
        "application_start_newly_confirmed_task",
    ] {
        let start = commands
            .find(&format!("pub fn {command}"))
            .expect("batch NISAR command adapter exists");
        let body = &commands[start..commands[start..].find("\n}").unwrap() + start + 2];
        assert!(body.contains("validate_application_window(&window)?"));
        assert!(body.contains("map_err(application_error)"));
        assert!(!body.contains("url"));
        assert!(!body.contains("credential"));
    }
    assert!(application.contains("pub fn preflight_nisar_subset_batch_v2"));
    assert!(application.contains("pub fn prepare_nisar_subset_batch_v2"));
    assert!(application.contains("pub fn confirm_nisar_subset_batch_task_v2"));
    assert!(application.contains("pub fn append_nisar_subset_batch_task_v2"));
    assert!(application.contains("pub fn read_nisar_subset_append_context_v2"));
    assert!(application.contains("pub fn preflight_nisar_subset_append_v2"));
    assert!(application.contains("pub fn create_nisar_subset_successor_task_v2"));
    assert!(application.contains("pub fn resume_nisar_subset_task"));
    assert!(application.contains("pub fn start_newly_confirmed_task"));
}

#[test]
fn dem_command_carries_exact_aoi_identity_and_derives_windows_in_application_service() {
    let commands = include_str!("../src/commands.rs");
    let application = include_str!("../src/application.rs");

    let command_start = commands
        .find("pub fn application_create_standalone_dem_task")
        .expect("standalone DEM command exists");
    let command = &commands
        [command_start..commands[command_start..].find("\n}").unwrap() + command_start + 2];
    assert!(command.contains("StandaloneDemAoiTaskRequest"));
    assert!(!command.contains("StandaloneDemTaskRequest"));
    assert!(application.contains("self.service.create_standalone_dem_aoi_task(request)"));
    assert!(application.contains(
        "\"dem_finalize_windows\" => native_dem_finalize_windows(&self.engine, &payload)"
    ));
    assert!(application.contains(".merge_antimeridian_dem(&inputs[0], &inputs[1], &merged_input)"));
}

#[test]
fn bounded_core_restore_contract_does_not_expose_candidate_array() {
    let projection = include_str!("../../crates/insar-application-service/src/projection.rs");
    let core_restore = include_str!("../../crates/insar-store/src/core_restore.rs");

    assert!(projection.contains("CoreServiceSnapshot"));
    assert!(projection.contains("ApplicationWorkbenchPage"));
    assert!(core_restore.contains("candidate_count"));
    assert!(core_restore.contains("items: Vec<StoreWorkbenchPageItem>"));
    assert!(!core_restore.contains("pub candidates:"));
}

#[test]
fn there_is_no_foundation_or_provider_ready_command_registration_split() {
    let library = include_str!("../src/lib.rs");
    let main = include_str!("../src/main.rs");
    assert!(library.contains("ApplicationServiceState::with_store_path_and_runtime_observations"));
    assert!(library.contains("fn run_desktop_with_application_service"));
    for forbidden in [
        "run_foundation_with_application_state",
        "run_provider_ready_with_application_state",
        "run_provider_ready_m1",
        "--m1-provider-ready",
        "ApplicationServiceConfig::existing_store",
    ] {
        assert!(
            !library.contains(forbidden),
            "stale dual-profile marker: {forbidden}"
        );
        assert!(
            !main.contains(forbidden),
            "stale dual-profile marker: {forbidden}"
        );
    }
}

#[test]
fn application_commands_are_typed_adapters_and_do_not_own_runtime_state() {
    let commands = include_str!("../src/commands.rs");
    for forbidden in [
        "SidecarRequest",
        "TaskStore",
        "DownloadScheduler",
        "Dispatcher",
        "std::fs",
        "reqwest",
        "Command::",
    ] {
        assert!(
            !commands.contains(forbidden),
            "Tauri command adapter must not own {forbidden}"
        );
    }
    for command in [
        "application_effective_capability",
        "application_create_download_task",
        "application_create_nisar_remote_subset_task",
        "application_preflight_nisar_subset_batch_v2",
        "application_prepare_nisar_subset_batch_v2",
        "application_confirm_nisar_subset_batch_task_v2",
        "application_append_nisar_subset_batch_task_v2",
        "application_read_nisar_subset_append_context_v2",
        "application_preflight_nisar_subset_append_v2",
        "application_create_nisar_subset_successor_task_v2",
        "application_resume_nisar_subset_task",
        "application_start_newly_confirmed_task",
    ] {
        assert!(contains_typed_command(commands, command));
        assert!(commands.contains("validate_application_window(&window)?"));
        assert!(commands.contains("map_err(application_error)"));
    }
    assert!(commands.contains("pub async fn application_catalog_search_and_freeze"));
    assert!(commands.contains("pub async fn application_sar_search_and_freeze"));
    assert!(commands.contains("tauri::async_runtime::spawn_blocking(move ||"));
}

#[test]
fn window_shutdown_is_registered_once_on_the_single_managed_state() {
    let library = include_str!("../src/lib.rs");
    assert_eq!(library.matches(".on_window_event").count(), 1);
    let close_hook = library
        .split(".on_window_event")
        .nth(1)
        .and_then(|section| section.split(".plugin").next())
        .expect("single close hook section");
    assert_eq!(close_hook.matches("state.shutdown()").count(), 1);
    assert_eq!(close_hook.matches("worker.shutdown()").count(), 1);
    assert_eq!(library.matches(".manage(state)").count(), 1);
    assert_eq!(library.matches(".manage(compat_worker)").count(), 1);
    assert_eq!(library.matches(".manage(ui_smoke_state)").count(), 1);
}

#[test]
fn compatibility_worker_is_external_and_persistent_by_contract() {
    let worker = include_str!("../src/compat_worker.rs");
    let build = include_str!("../build.rs");
    let contract = include_str!("../../contracts/compat-worker-component.v1.json");
    for marker in [
        "child: Option<Child>",
        "stdin:",
        "stdout_reader:",
        "pending:",
        "process_generation:",
        "restart_count:",
        "verified_component_identity:",
        "COMPAT_WORKER_INTEGRITY_FAILED",
        "COMPAT_WORKER_PROTOCOL_MISMATCH",
        "COMPAT_WORKER_CAPABILITY_MISMATCH",
        "COMPAT_WORKER_TEST_OVERRIDE",
    ] {
        assert!(
            worker.contains(marker),
            "missing Worker lifecycle marker: {marker}"
        );
    }
    for forbidden in [
        "include_bytes!",
        "WORKER_BYTES",
        "extract_embedded_worker",
        "EOSTAGE_COMPAT_WORKER_EXE",
        "temp_dir()",
        "Command::new(path)",
        "EOSTAGE_PYTHON",
    ] {
        assert!(
            !worker.contains(forbidden),
            "forbidden Worker boundary: {forbidden}"
        );
        assert!(
            !build.contains(forbidden),
            "forbidden build boundary: {forbidden}"
        );
    }
    for field in [
        "component_id",
        "protocol",
        "relative_path",
        "executable_name",
        "version",
        "sha256",
        "supported_operations",
        "offline_only",
        "store_access",
        "credential_access",
        "download_queue_access",
        "lifecycle_owner",
        "task_id_owner",
    ] {
        assert!(contract.contains(&format!("\"{field}\"")));
    }
}

#[test]
fn companion_orbit_command_uses_only_the_validated_window_and_source_task_identity() {
    let commands = include_str!("../src/commands.rs");
    let body = command_block(commands, "application_create_companion_orbit_task");
    assert!(body.contains("validate_application_window(&window)?"));
    assert!(body.contains("source_task_id: String"));
    assert!(body.contains(".create_companion_orbit_task(&source_task_id)"));
    assert!(body.contains("map_err(application_error)"));
    assert!(!body.contains("url"));
    assert!(!body.contains("credential"));
}

#[test]
fn agent_commands_stay_off_the_window_thread_and_never_return_model_keys() {
    let commands = include_str!("../src/commands.rs");
    for command in [
        "agent_sessions_list",
        "agent_sessions_create",
        "agent_sessions_delete",
        "agent_sessions_rename",
        "agent_sessions_set_pinned",
        "agent_sessions_set_archived",
        "agent_sessions_move",
        "agent_sessions_export",
        "agent_sessions_export_to",
        "agent_workspaces_list",
        "agent_workspaces_create",
        "agent_workspaces_update",
        "agent_workspaces_delete",
        "agent_send",
        "agent_stream_poll",
        "agent_plan_approve",
        "agent_plan_reject",
        "agent_run_cancel",
        "agent_runs_list",
        "agent_session_work",
        "agent_session_truncate",
        "agent_session_workspace_state",
        "agent_session_set_working_aoi",
        "agent_runs_open",
        "agent_channels_get",
        "agent_disabled_items_get",
        "agent_disabled_items_set",
        "agent_channels_save",
        "agent_channels_test",
        "agent_channels_models",
        "agent_channels_probe_draft",
        "agent_channels_test_draft",
        "agent_channels_set_default",
        "agent_channels_delete",
        "agent_channel_rename",
        "agent_permission_mode_get",
        "agent_permission_mode_set_global",
        "agent_permission_mode_set_session",
        "agent_stop_all",
        "agent_activity_status",
    ] {
        let body = command_block(commands, command);
        assert!(
            body.starts_with(&format!("pub async fn {command}")),
            "{command} blocks the window thread"
        );
        assert!(
            body.contains("validate_application_window(&window)?"),
            "{command}"
        );
        assert!(
            body.contains("tauri::async_runtime::spawn_blocking(move ||"),
            "{command} must run Agent Store/file/network work in the background"
        );
        assert!(body.contains("map_err(agent_error)"), "{command}");
    }
    for command in [
        "agent_sync_public_model_registry",
        "agent_oauth_device_start",
        "agent_oauth_device_poll",
        "agent_oauth_pkce_start",
        "agent_oauth_pkce_complete",
        "agent_oauth_channels_meta",
        "agent_oauth_channel_quota",
        "eostage_account_status",
        "eostage_account_login_start",
        "eostage_account_login_poll",
        "eostage_account_login_complete_code",
        "eostage_account_logout",
        "eostage_account_sync_models",
        "eostage_billing_config",
        "eostage_billing_redeem",
        "eostage_billing_orders_list",
        "eostage_billing_order_create",
        "eostage_billing_order_check",
        "eostage_billing_order_cancel",
        "eostage_usage_list",
        "desktop_onboarding_state_get",
        "desktop_onboarding_state_set",
        "desktop_active_resource_get",
        "desktop_active_resource_set",
    ] {
        let body = command_block(commands, command);
        assert!(
            body.starts_with(&format!("pub async fn {command}")),
            "{command} blocks the window thread"
        );
        assert!(
            body.contains("validate_application_window(&window)?"),
            "{command}"
        );
        assert!(
            body.contains("tauri::async_runtime::spawn_blocking("),
            "{command} must run OAuth/registry work in the background"
        );
        assert!(
            body.contains("ShellError::application_service"),
            "{command}"
        );
    }
    let get = command_block(commands, "agent_channels_get");
    assert!(get.contains("Vec<crate::agent::AgentChannelView>"));
    let agent = include_str!("../src/agent.rs");
    let view_start = agent
        .find("pub struct AgentChannelView")
        .expect("channel view projection exists");
    let view = &agent[view_start..view_start + agent[view_start..].find('}').unwrap()];
    assert!(view.contains("pub key_ready: bool"));
    assert!(view.contains("pub is_default: bool"));
    assert!(!view.contains("pub key:") && !view.contains("api_key"));
    assert!(
        !agent.contains("reveal"),
        "a model key is never read back to the UI"
    );
}
