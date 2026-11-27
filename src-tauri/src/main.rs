// L5 recheck 2026-10-19: a release build is a Windows GUI program (PE
// Subsystem 2): started from Explorer it opens no console window whose closing
// would end the application. Debug builds keep their console.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// The command-line modes that print a payload instead of opening the window.
const CLI_MODES: [&str; 5] = [
    "--selftest",
    "--s1-live-acceptance",
    "--native-gis-smoke",
    "--smoke-test",
    "--ui-smoke-test",
];

fn main() {
    // A GUI-subsystem host started from a terminal has no console; its
    // command-line modes attach to the parent's (see cli_console).
    if std::env::args().any(|argument| CLI_MODES.contains(&argument.as_str())) {
        eostage_desktop::attach_parent_console_for_cli();
    }

    // The packaged host guarantees HDF5 dynamic plugin isolation itself at
    // the earliest possible point, BEFORE any other HDF5 function call and
    // before any thread creation: it sets the H5PL sentinel, removes any
    // inherited HDF5_PLUGIN_PATH / VOL / driver configuration, then
    // initialises HDF5 and disables dynamic plugin loading through the formal
    // H5PL API. NISAR HDF5 validation stays fail-closed when this cannot be
    // established, so a failure here only disables NISAR validation (never
    // weakening it) and leaves non-HDF5 features intact.
    if let Err(error) = insar_download::establish_hdf5_process_isolation() {
        eprintln!("HDF5 process isolation unavailable: {error}");
    }

    if std::env::args().any(|argument| argument == "--selftest") {
        let payload = eostage_desktop::selftest_payload();
        println!(
            "{}",
            serde_json::to_string(&payload).expect("selftest payload serializes")
        );
        return;
    }

    #[cfg(feature = "s1-live-acceptance")]
    if std::env::args().any(|argument| argument == "--s1-live-acceptance") {
        let result = eostage_desktop::s1_live_acceptance_payload();
        match result {
            Ok(payload) => {
                let encoded = serde_json::to_string_pretty(&payload)
                    .expect("S1 live acceptance payload serializes");
                if let Some(path) = std::env::var_os("INSAR_S1_LIVE_OUTPUT_PATH") {
                    if std::fs::write(path, format!("{encoded}\n")).is_err() {
                        eprintln!("S1 live acceptance output could not be written");
                        std::process::exit(1);
                    }
                }
                println!("{encoded}");
            }
            Err(error) => {
                eprintln!("S1 live acceptance failed: {error}");
                std::process::exit(1);
            }
        }
        return;
    }

    if std::env::args().any(|argument| argument == "--native-gis-smoke") {
        match eostage_desktop::native_gis_smoke_payload() {
            Ok(payload) => println!(
                "{}",
                serde_json::to_string(&payload).expect("native GIS smoke payload serializes")
            ),
            Err(error) => {
                eprintln!("Native GIS packaged smoke failed: {error}");
                std::process::exit(1);
            }
        }
        return;
    }

    if std::env::args().any(|argument| argument == "--smoke-test") {
        match eostage_desktop::smoke_test_payload() {
            Ok(payload) => println!(
                "{}",
                serde_json::to_string(&payload).expect("smoke-test payload serializes")
            ),
            Err(error) => {
                eprintln!("Tauri single-host smoke test failed: {error}");
                std::process::exit(1);
            }
        }
        return;
    }

    if std::env::args().any(|argument| argument == "--ui-smoke-test") {
        let phase = match std::env::var("INSAR_UI_SMOKE_PHASE").as_deref() {
            Ok("phase_a") => eostage_desktop::ui_smoke::UiSmokePhase::PhaseA,
            Ok("phase_b") => eostage_desktop::ui_smoke::UiSmokePhase::PhaseB,
            _ => {
                eprintln!("Tauri UI smoke test requires INSAR_UI_SMOKE_PHASE=phase_a|phase_b");
                std::process::exit(2);
            }
        };
        let Some(store) = std::env::var_os("INSAR_UI_SMOKE_STORE").map(std::path::PathBuf::from)
        else {
            eprintln!("Tauri UI smoke test requires INSAR_UI_SMOKE_STORE");
            std::process::exit(2);
        };
        match eostage_desktop::run_ui_smoke_test(phase, store) {
            Ok(()) => {}
            Err(error) => {
                eprintln!("Tauri UI smoke test failed: {error}");
                std::process::exit(1);
            }
        }
        return;
    }

    let result = eostage_desktop::run();

    if result.is_err() {
        eprintln!("GUI003: shell runtime failed");
        std::process::exit(1);
    }
}
