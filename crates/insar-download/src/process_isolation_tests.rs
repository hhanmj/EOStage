//! Child-process tests for HDF5 pre-initialization isolation.
//!
//! HDF5 initialisation and environment variables are process state, so the
//! isolation policy is proven by spawning the unit-test binary itself as a
//! fresh child process in three scenarios:
//!
//! A. the parent provides no HDF5_PLUGIN_PRELOAD (and no hostile values):
//!    the app establishes isolation itself and validates HDF5;
//! B. the parent provides hostile HDF5_PLUGIN_PATH / VOL connector / driver
//!    values: the app removes them BEFORE H5open and validates HDF5;
//! C. isolation is deliberately skipped: the validator must fail closed with
//!    NISAR_HDF5_VALIDATION_UNAVAILABLE.
//!
//! `child_probe_entry` is the child-mode entry: when the environment variable
//! `INSAR_HDF5_CHILD_MODE` is set it runs one probe and exits. In normal test
//! runs it is a no-op.

use std::process::{Command, Stdio};

use crate::test_suites::nisar_hdf5_support::{create_valid_fixture, FrequencyScope};
use crate::test_suites::nisar_hdf5_validation::validate_probe_without_isolation;
use serde_json::{json, Value};

const CHILD_MODE_ENV: &str = "INSAR_HDF5_CHILD_MODE";
const PROBE_LINE_PREFIX: &str = "PROBE_RESULT ";

fn current_exe() -> std::path::PathBuf {
    std::env::current_exe().expect("test binary path resolves")
}

fn env_after_json() -> Value {
    // Read through the CRT environment so the report matches exactly what
    // HDF5's getenv observes, not just the Windows OS environment block.
    json!({
        "HDF5_PLUGIN_PRELOAD": crate::hdf5_plugin_isolation::crt_env::get("HDF5_PLUGIN_PRELOAD"),
        "HDF5_PLUGIN_PATH": crate::hdf5_plugin_isolation::crt_env::get("HDF5_PLUGIN_PATH"),
        "HDF5_VOL_CONNECTOR": crate::hdf5_plugin_isolation::crt_env::get("HDF5_VOL_CONNECTOR"),
        "HDF5_DRIVER": crate::hdf5_plugin_isolation::crt_env::get("HDF5_DRIVER"),
        "HDF5_DRIVER_CONFIG": crate::hdf5_plugin_isolation::crt_env::get("HDF5_DRIVER_CONFIG"),
    })
}

/// Child-mode entry. Runs only when the parent spawned this binary as a probe
/// child; otherwise it returns immediately and is a harmless no-op test.
#[test]
fn child_probe_entry() {
    let Some(mode) = std::env::var(CHILD_MODE_ENV).ok() else {
        return;
    };
    let probe = run_child_probe(&mode);
    let line = format!(
        "{PROBE_LINE_PREFIX}{}",
        serde_json::to_string(&probe).unwrap()
    );
    use std::io::Write;
    let mut stdout = std::io::stdout();
    writeln!(stdout, "{line}").unwrap();
    stdout.flush().unwrap();
    std::process::exit(if probe["probe_ok"] == json!(true) {
        0
    } else {
        1
    });
}

fn run_child_probe(mode: &str) -> Value {
    match mode {
        "establish" | "hostile" => {
            let status = crate::hdf5_plugin_isolation::establish_hdf5_process_isolation()
                .map_err(|error| error.to_string());
            let status = match status {
                Ok(status) => status,
                Err(error) => {
                    return json!({
                        "mode": mode,
                        "probe_ok": false,
                        "error": error,
                        "env_after": env_after_json(),
                    });
                }
            };
            let fixture_result = validate_fixture_after_establish();
            let (validation_ok, validation_code) = match fixture_result {
                Ok(()) => (true, "ok".to_owned()),
                Err(error) => (false, error.code().to_owned()),
            };
            json!({
                "mode": mode,
                "probe_ok": true,
                "status": serde_json::to_value(&status).unwrap(),
                "validation_ok": validation_ok,
                "validation_code": validation_code,
                "env_after": env_after_json(),
            })
        }
        "skip" => {
            // Deliberately do NOT establish isolation. HDF5 initialises with
            // dynamic plugin loading enabled; the validator must fail closed.
            let fixture_result = validate_fixture_without_isolation();
            match fixture_result {
                Err(error) if error.code() == "NISAR_HDF5_VALIDATION_UNAVAILABLE" => json!({
                    "mode": mode,
                    "probe_ok": true,
                    "validation_ok": false,
                    "validation_code": error.code(),
                    "env_after": env_after_json(),
                }),
                other => json!({
                    "mode": mode,
                    "probe_ok": false,
                    "unexpected": match other {
                        Ok(()) => "validation unexpectedly succeeded".to_owned(),
                        Err(error) => format!("unexpected code: {}", error.code()),
                    },
                    "env_after": env_after_json(),
                }),
            }
        }
        other => json!({ "mode": other, "probe_ok": false, "error": "unknown child mode" }),
    }
}

fn validate_fixture_after_establish() -> Result<(), crate::ArtifactValidationError> {
    let fixtures = crate::test_suites::nisar_hdf5_support::FixtureDirectory::new("child-establish");
    let path = fixtures.file("valid");
    create_valid_fixture(&path, FrequencyScope::Dual);
    validate_probe_without_isolation(&path, FrequencyScope::Dual)
}

fn validate_fixture_without_isolation() -> Result<(), crate::ArtifactValidationError> {
    let fixtures = crate::test_suites::nisar_hdf5_support::FixtureDirectory::new("child-skip");
    let path = fixtures.file("valid");
    create_valid_fixture(&path, FrequencyScope::Dual);
    validate_probe_without_isolation(&path, FrequencyScope::Dual)
}

fn spawn_child(mode: &str, extra_env: &[(&str, &std::ffi::OsStr)]) -> Value {
    let mut command = Command::new(current_exe());
    command
        .arg("--exact")
        .arg("process_isolation_tests::child_probe_entry")
        .arg("--test-threads=1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env(CHILD_MODE_ENV, mode)
        // The packaging condition: the parent does not supply the sentinel.
        .env_remove("HDF5_PLUGIN_PRELOAD")
        .env_remove("HDF5_PLUGIN_PATH")
        .env_remove("HDF5_VOL_CONNECTOR")
        .env_remove("HDF5_DRIVER")
        .env_remove("HDF5_DRIVER_CONFIG");
    for (key, value) in extra_env {
        command.env(key, value);
    }
    let output = command.output().expect("child probe process launches");
    assert!(
        output.status.success(),
        "child probe failed (mode {mode}): {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let index = stdout.find(PROBE_LINE_PREFIX).unwrap_or_else(|| {
        panic!(
            "child probe produced no result line (mode {mode}); stdout={stdout:?}; stderr={}",
            String::from_utf8_lossy(&output.stderr)
        )
    });
    let probe_json = stdout[index + PROBE_LINE_PREFIX.len()..].trim();
    serde_json::from_str(probe_json)
        .unwrap_or_else(|error| panic!("child probe JSON parse failed: {error}: {probe_json}"))
}

/// A. Parent provides no HDF5_PLUGIN_PRELOAD (and no hostile values): the
/// application establishes isolation itself and successfully validates HDF5.
#[test]
fn child_process_without_parent_sentinel_establishes_and_validates() {
    let probe = spawn_child("establish", &[]);
    assert_eq!(probe["probe_ok"], json!(true), "{probe}");
    assert_eq!(probe["validation_ok"], json!(true), "{probe}");
    assert_eq!(
        probe["status"]["isolation_established"],
        json!(true),
        "{probe}"
    );
    assert_eq!(
        probe["status"]["startup_environment_sanitized"],
        json!(true),
        "{probe}"
    );
    assert_eq!(
        probe["status"]["preload_sentinel_set_by_application"],
        json!(true),
        "{probe}"
    );
    assert_eq!(
        probe["status"]["parent_env_preload_absent"],
        json!(true),
        "{probe}"
    );
    assert_eq!(probe["status"]["loading_mask_zero"], json!(true), "{probe}");
}

/// B. Parent provides hostile HDF5_PLUGIN_PATH / VOL connector / driver
/// values: the application removes them BEFORE H5open, starts successfully and
/// validates HDF5 with the safe local configuration.
#[test]
fn child_process_sanitizes_hostile_hdf5_environment_before_h5open() {
    let hostile =
        std::env::temp_dir().join(format!("insar-hdf5-hostile-child-{}", std::process::id()));
    std::fs::create_dir_all(&hostile).unwrap();
    std::fs::write(hostile.join("hdf5_plugin.dll"), b"not a plugin").unwrap();

    let probe = spawn_child(
        "hostile",
        &[
            ("HDF5_PLUGIN_PATH", hostile.as_os_str()),
            (
                "HDF5_VOL_CONNECTOR",
                std::ffi::OsStr::new("nonexistent_external_connector"),
            ),
            (
                "HDF5_DRIVER",
                std::ffi::OsStr::new("nonexistent_external_driver"),
            ),
            ("HDF5_DRIVER_CONFIG", std::ffi::OsStr::new("hostile")),
        ],
    );
    let _ = std::fs::remove_dir_all(&hostile);

    assert_eq!(probe["probe_ok"], json!(true), "{probe}");
    assert_eq!(probe["validation_ok"], json!(true), "{probe}");
    assert_eq!(
        probe["status"]["isolation_established"],
        json!(true),
        "{probe}"
    );
    assert_eq!(
        probe["status"]["startup_environment_sanitized"],
        json!(true),
        "{probe}"
    );
    assert_eq!(
        probe["status"]["plugin_path_removed"],
        json!(true),
        "{probe}"
    );
    assert_eq!(
        probe["status"]["vol_connector_removed"],
        json!(true),
        "{probe}"
    );
    assert_eq!(probe["status"]["driver_removed"], json!(true), "{probe}");
    assert_eq!(
        probe["status"]["driver_config_removed"],
        json!(true),
        "{probe}"
    );
    assert_eq!(probe["status"]["loading_mask_zero"], json!(true), "{probe}");
    assert_eq!(
        probe["env_after"]["HDF5_PLUGIN_PATH"],
        Value::Null,
        "{probe}"
    );
    assert_eq!(
        probe["env_after"]["HDF5_VOL_CONNECTOR"],
        Value::Null,
        "{probe}"
    );
    assert_eq!(probe["env_after"]["HDF5_DRIVER"], Value::Null, "{probe}");
    assert_eq!(
        probe["env_after"]["HDF5_DRIVER_CONFIG"],
        Value::Null,
        "{probe}"
    );
    assert_eq!(
        probe["env_after"]["HDF5_PLUGIN_PRELOAD"],
        json!("::"),
        "{probe}"
    );
}

/// C. Isolation is deliberately skipped: the validator must return
/// NISAR_HDF5_VALIDATION_UNAVAILABLE.
#[test]
fn child_process_skipping_isolation_keeps_validator_unavailable() {
    let probe = spawn_child("skip", &[]);
    assert_eq!(probe["probe_ok"], json!(true), "{probe}");
    assert_eq!(probe["validation_ok"], json!(false), "{probe}");
    assert_eq!(
        probe["validation_code"],
        json!("NISAR_HDF5_VALIDATION_UNAVAILABLE"),
        "{probe}"
    );
}
