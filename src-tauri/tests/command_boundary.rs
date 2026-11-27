// Compatibility entry point for the historical regression contract. The
// single-host assertions live in `single_host_boundary.rs`; keeping this thin
// include preserves existing governance references without restoring the old
// dual-runtime command surface.
include!("single_host_boundary.rs");

fn nearest_tauri_command_attribute<'a>(source: &'a str, command: &str) -> &'a str {
    let function = format!("pub fn {command}");
    let async_function = format!("pub async fn {command}");
    let function_start = source
        .find(&function)
        .or_else(|| source.find(&async_function))
        .expect("typed command exists");
    let prefix = &source[..function_start];
    let attribute_start = prefix
        .rfind("#[tauri::command")
        .expect("Tauri command attribute exists");
    let attribute_end = source[attribute_start..]
        .find(']')
        .map(|offset| attribute_start + offset + 1)
        .expect("Tauri command attribute closes");
    &source[attribute_start..attribute_end]
}

#[test]
fn blocking_search_commands_are_dispatched_off_the_tauri_main_thread() {
    let commands = include_str!("../src/commands.rs");
    let command = "application_catalog_search_and_freeze";
    assert_eq!(
        nearest_tauri_command_attribute(commands, command),
        "#[tauri::command]",
        "{command} must expose an explicit async wrapper"
    );
    let function_start = commands
        .find(&format!("pub async fn {command}"))
        .expect("catalogue search command is async");
    let body = &commands[function_start..];
    assert!(
        body.find("tauri::async_runtime::spawn_blocking")
            .is_some_and(|offset| offset < 1_000),
        "{command} must move reqwest::blocking work outside the Tokio worker"
    );

    for command in [
        "application_catalog_intent_search_and_freeze",
        "application_sar_search_and_freeze",
        "application_sar_background_count",
        "application_admin_boundary_search",
        "application_admin_boundary_resolve",
    ] {
        assert_eq!(
            nearest_tauri_command_attribute(commands, command),
            "#[tauri::command]",
            "{command} must expose the async command wrapper"
        );
        let function_start = commands
            .find(&format!("pub async fn {command}"))
            .expect("async typed command exists");
        let body = &commands[function_start..];
        assert!(
            body.find("tauri::async_runtime::spawn_blocking")
                .is_some_and(|offset| offset < 1_000),
            "{command} must move blocking provider work off the Tauri main thread"
        );
    }

    for control in [
        "application_core_restore",
        "application_workbench_page",
        "application_cancel_request",
    ] {
        assert_eq!(
            nearest_tauri_command_attribute(commands, control),
            "#[tauri::command]",
            "{control} must retain its existing bounded synchronous contract"
        );
    }
}

#[test]
fn task_historical_context_commands_are_typed_and_registered() {
    let commands = include_str!("../src/commands.rs");
    let shell = include_str!("../src/lib.rs");
    for command in [
        "application_load_task_source_batch_page",
        "application_append_task_from_source_batch",
        "application_create_task_from_historical_source_batch",
        "application_create_burst_asset_task",
        "application_append_burst_asset_task",
        "application_create_historical_burst_asset_task",
    ] {
        assert!(
            commands.contains(&format!("pub fn {command}"))
                || commands.contains(&format!("pub async fn {command}")),
            "{command} must expose a typed Tauri boundary"
        );
        assert!(
            shell.contains(&format!("commands::{command}")),
            "{command} must be registered in the one Tauri host"
        );
    }
    let read_start = commands
        .find("application_load_task_source_batch_page")
        .expect("task source read command exists");
    let read_signature = &commands[read_start..read_start + 420];
    assert!(read_signature.contains("StoreTaskSourceBatchPageRequest"));
    assert!(!read_signature.contains("expected_revision"));
}

#[test]
fn sar_download_policy_commands_are_typed_and_registered() {
    let commands = include_str!("../src/commands.rs");
    let shell = include_str!("../src/lib.rs");
    for command in [
        "application_get_sar_download_policy_settings",
        "application_save_sar_download_policy_settings",
    ] {
        assert!(commands.contains(&format!("pub fn {command}")));
        assert!(shell.contains(&format!("commands::{command}")));
    }
    assert!(commands.contains("settings: SarDownloadPolicySettings"));
}

#[test]
fn external_urls_use_the_registered_native_os_opener() {
    let commands = include_str!("../src/commands.rs");
    let external_url = include_str!("../src/external_url.rs");
    let shell = include_str!("../src/lib.rs");
    assert!(commands.contains("pub fn application_open_external_url"));
    assert!(commands.contains("crate::external_url::open_in_default_browser(normalized)"));
    assert!(commands.contains("normalized.starts_with(\"https://\")"));
    assert!(commands.contains("normalized.starts_with(\"http://\")"));
    assert!(external_url.contains("url.dll,FileProtocolHandler"));
    assert!(external_url.contains("std::process::Command::new"));
    assert!(shell.contains("mod external_url;"));
    assert!(shell.contains("commands::application_open_external_url"));
}
