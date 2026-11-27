#[cfg(windows)]
use std::io::Read;
use std::process::Command;
#[cfg(windows)]
use std::process::Stdio;
#[cfg(windows)]
use std::time::{Duration, Instant};

fn desktop_executable() -> String {
    std::env::var("CARGO_BIN_EXE_eostage_desktop")
        .or_else(|_| std::env::var("CARGO_BIN_EXE_eostage-desktop"))
        .expect("Cargo must expose the desktop host binary to integration tests")
}

#[test]
fn selftest_identifies_the_single_business_desktop_host() {
    let output = Command::new(desktop_executable())
        .arg("--selftest")
        .output()
        .expect("desktop selftest launches");
    assert!(output.status.success());
    assert!(output.stderr.is_empty());

    let payload: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("selftest emits one JSON payload");
    assert_eq!(payload["shell_kind"], "tauri_single_desktop_host");
    assert_eq!(payload["business_runtime"], "application_service");
    let serialized = payload.to_string().to_lowercase();
    for forbidden in ["token", "password", "sqlite", "sidecar", "python"] {
        assert!(
            !serialized.contains(forbidden),
            "unexpected selftest field: {forbidden}"
        );
    }
}

#[test]
fn runtime_smoke_reports_the_disabled_nisar_subset_boundary() {
    if std::env::var_os("EOSTAGE_COMPAT_WORKER_TEST_OVERRIDE").is_none() {
        eprintln!("runtime smoke boundary test requires EOSTAGE_COMPAT_WORKER_TEST_OVERRIDE");
        return;
    }
    let output = Command::new(desktop_executable())
        .arg("--smoke-test")
        .output()
        .expect("desktop runtime smoke launches");
    assert!(output.status.success(), "stderr: {:?}", output.stderr);
    assert!(output.stderr.is_empty());

    let payload: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("runtime smoke emits one JSON payload");
    assert_eq!(payload["runtime_smoke"], true);
    assert_eq!(payload["application_service_ready"], true);
    assert_eq!(payload["worker_protocol_ready"], true);
    assert_eq!(
        payload["application_service"]["nisar_subset_capability"]["visible"],
        true
    );
    assert_eq!(
        payload["application_service"]["nisar_subset_capability"]["enabled"],
        false
    );
    assert_eq!(
        payload["application_service"]["nisar_subset_capability"]["live_verified"],
        false
    );
    assert_eq!(
        payload["application_service"]["nisar_subset_capability"]["production_enabled"],
        false
    );
    assert_eq!(
        payload["application_service"]["nisar_subset_capability"]["default_enabled"],
        false
    );
    assert_eq!(
        payload["application_service"]["nisar_subset_capability"]["reason_code"],
        "NISAR_SUBSET_NOT_ENABLED"
    );
    assert_eq!(
        payload["application_service"]["nisar_subset_task_rejected"]["reason_code"],
        "NISAR_SUBSET_NOT_ENABLED"
    );
    assert_eq!(
        payload["compatibility_worker"]["probe"]["public_requests"],
        0
    );
}

#[test]
fn normal_startup_has_no_second_host_or_child_application_path() {
    let library = include_str!("../src/lib.rs").to_lowercase();
    let application = include_str!("../src/application.rs").to_lowercase();
    let main = include_str!("../src/main.rs").to_lowercase();
    let cargo = include_str!("../Cargo.toml").to_lowercase();
    assert!(library.contains("applicationservicestate::with_store_path_and_runtime_observations"));
    assert!(application.contains("pub fn with_store_path"));
    assert!(application.contains("applicationserviceconfig::business_store"));
    let formal_dependency = cargo
        .lines()
        .find(|line| line.contains("insar-application-service"))
        .expect("formal Tauri dependency is declared");
    assert!(!formal_dependency.contains("features"));
    assert!(main.contains("--ui-smoke-test"));
    for forbidden in [
        "run_foundation_with_application_state",
        "run_provider_ready_with_application_state",
        "--m1-provider-ready",
        "pywebview",
        "command::new",
        "insar-service",
    ] {
        assert!(
            !library.contains(forbidden),
            "stale desktop startup path: {forbidden}"
        );
        assert!(
            !main.contains(forbidden),
            "stale desktop startup path: {forbidden}"
        );
    }
}

/// L5 recheck 2026-10-19 (experiment C): when WebView2 could not be
/// initialised, tauri only logged the window creation failure and the host kept
/// running without any window, holding the Store lock. Here WebView2's data
/// folder cannot be created (its parent is a regular file): the host must report
/// the failure on stderr and exit with code 4 instead of lingering.
///
/// Manual only: this launches the real host on the interactive desktop. Its main
/// window appears for a few seconds, and the WebView2 runtime itself shows a
/// modal "cannot create data folder" dialog (Microsoft Edge cannot read and
/// write its data directory) that stays until someone dismisses it. The
/// application's own dialog is suppressed by EOSTAGE_NO_STARTUP_ERROR_DIALOG.
/// The automated coverage of the same outcome is in the desktop library's unit
/// tests (main_window_failure, report_main_window_failure). Run it by hand with
/// `cargo test -p eostage-desktop --test desktop_lifecycle
/// an_unusable_webview2_data_folder -- --ignored` and click OK in the WebView2
/// dialog; the host exits with code 4 within seconds either way.
#[cfg(windows)]
#[test]
#[ignore = "manual: launches the real host; the WebView2 runtime shows a modal dialog that must be dismissed by hand"]
fn an_unusable_webview2_data_folder_ends_the_host_with_a_diagnostic() {
    let root = std::env::temp_dir().join(format!(
        "insar-desktop-webview-failure-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("localappdata")).expect("isolated LOCALAPPDATA");
    std::fs::create_dir_all(root.join("appdata")).expect("isolated APPDATA");
    let blocker = root.join("not-a-directory");
    std::fs::write(&blocker, b"a file where WebView2 expects a folder").expect("blocker file");
    let mut child = Command::new(desktop_executable())
        .env("LOCALAPPDATA", root.join("localappdata"))
        .env("APPDATA", root.join("appdata"))
        .env("WEBVIEW2_USER_DATA_FOLDER", blocker.join("webview2"))
        .env("EOSTAGE_NO_STARTUP_ERROR_DIALOG", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("desktop host launches");
    let deadline = Instant::now() + Duration::from_secs(120);
    let status = loop {
        if let Some(status) = child.try_wait().expect("poll the desktop host") {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("the desktop host kept running without a usable main window");
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .expect("captured stderr")
        .read_to_string(&mut stderr)
        .expect("read stderr");
    assert_eq!(status.code(), Some(4), "stderr: {stderr}");
    assert!(
        stderr.contains("DESKTOP_MAIN_WINDOW_UNAVAILABLE"),
        "stderr: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// L5 recheck 2026-10-19: a release build is a Windows GUI program and its
/// command-line modes attach to the parent's console (the packaged artifact's
/// PE Subsystem is checked by the build seam).
#[test]
fn release_builds_are_gui_programs_whose_cli_modes_attach_a_console() {
    let main = include_str!("../src/main.rs");
    assert!(main.contains(r#"#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]"#));
    let attach = main
        .find("attach_parent_console_for_cli()")
        .expect("the CLI modes attach the parent console");
    for mode in [
        "\"--selftest\"",
        "\"--smoke-test\"",
        "\"--native-gis-smoke\"",
        "\"--ui-smoke-test\"",
    ] {
        let printed = main
            .match_indices(mode)
            .map(|(index, _)| index)
            .max()
            .expect("the mode is handled");
        assert!(
            attach < printed,
            "{mode} is handled after the console is attached"
        );
    }
}
