#[allow(dead_code)]
#[path = "../examples/admission_benchmark.rs"]
mod admission_benchmark;

use serde_json::Value;

#[test]
fn loopback_smoke_emits_valid_opaque_schema_and_preserves_admission_invariants() {
    let rendered = admission_benchmark::run_smoke_suite_for_test()
        .expect("loopback benchmark smoke suite must complete");
    let report: Value = serde_json::from_str(&rendered).expect("smoke output must be JSON");

    assert_eq!(report["schema_version"], "d2b-admission-benchmark-v1");
    let failed_scenarios = report["samples"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|sample| sample["hard_gates_passed"].as_bool() != Some(true))
        .map(|sample| {
            format!(
                "{} integrity={} server_failures={} failure_codes={} leak={} bounds={}",
                sample["scenario"].as_str().unwrap_or("unknown"),
                sample["file_integrity_passed"],
                sample["fixture_server_failures"],
                sample["fixture_failure_codes"],
                sample["no_leak"],
                sample["policy_bounds_observed"],
            )
        })
        .collect::<Vec<_>>();
    assert!(
        report["hard_gates_passed"].as_bool().unwrap_or(false),
        "failed smoke scenarios: {failed_scenarios:?}"
    );
    assert!(report["samples"].as_array().is_some_and(|samples| {
        samples.iter().any(|sample| {
            sample["candidate"] == "compatibility" && sample["scenario"] == "single_unconstrained"
        }) && samples.iter().any(|sample| {
            sample["candidate"] == "g1-h1" && sample["scenario"] == "queued_cancellation"
        }) && samples.iter().any(|sample| {
            sample["scenario"] == "redirect_same_origin"
                && sample["initial_origin_grouping"].as_bool() == Some(true)
        }) && samples.iter().any(|sample| {
            sample["scenario"] == "redirect_other_origin"
                && sample["initial_origin_grouping"].as_bool() == Some(true)
        }) && samples.iter().any(|sample| {
            sample["scenario"] == "restart_oversubscription" && sample["candidate"] == "g1-h1"
        }) && samples.iter().any(|sample| {
            sample["scenario"] == "restart_oversubscription"
                && sample["candidate"] == "compatibility"
        })
    }));

    assert!(!rendered.contains("127.0.0.1"));
    assert!(!rendered.contains("benchmark-task"));
    assert!(admission_benchmark::rejects_non_loopback_for_test());
}

#[test]
fn scaled_batch_emits_streaming_default_profile_and_monotonic_timing() {
    let rendered = admission_benchmark::run_scaled_batch_for_test()
        .expect("scaled benchmark smoke batch must complete");
    let report: Value = serde_json::from_str(&rendered).expect("scaled output must be JSON");

    assert_eq!(report["schema_version"], "d2b-admission-scaled-v1");
    assert!(report["round"].is_object());
    assert_eq!(
        report["profile"]["transfer_options"]["max_attempts"].as_u64(),
        Some(3)
    );
    assert_eq!(
        report["profile"]["transfer_options"]["backoff_ms"].as_u64(),
        Some(2_000)
    );
    assert_eq!(
        report["profile"]["transfer_options"]["timeout_ms"].as_u64(),
        Some(60_000)
    );
    assert_eq!(
        report["profile"]["transfer_options"]["chunk_size"].as_u64(),
        Some(1024 * 1024)
    );
    assert!(report["hard_gates_passed"].as_bool().unwrap_or(false));

    let samples = report["samples"]
        .as_array()
        .expect("scaled batch must retain raw samples");
    assert_eq!(samples.len(), 1);
    let sample = &samples[0];
    assert_eq!(sample["scenario"], "single_origin_sustained");
    assert_eq!(sample["candidate"], "compatibility");
    assert_eq!(sample["phase"], "measured");
    assert_eq!(sample["round"].as_u64(), Some(1));
    assert_eq!(sample["order_index"].as_u64(), Some(0));
    assert!(sample["fixture"]["streaming_body"]
        .as_bool()
        .unwrap_or(false));
    assert_eq!(sample["fixture"]["file_count"].as_u64(), Some(4));
    assert_eq!(
        sample["fixture"]["total_bytes"].as_u64(),
        Some(24 * 1024 * 1024)
    );
    assert!(
        sample["metrics"]["scenario_elapsed_ns"]
            .as_u64()
            .unwrap_or(0)
            >= 750_000_000,
        "scaled Compatibility smoke must satisfy the duration validity floor"
    );
    assert!(sample["file_integrity_passed"].as_bool().unwrap_or(false));
    assert!(sample["no_leak"].as_bool().unwrap_or(false));
    assert!(!rendered.contains("127.0.0.1"));
    assert!(!rendered.contains("opaque-scaled-task"));
}

#[test]
fn scaled_runner_spools_large_batch_output_without_pipe_backpressure() {
    let runner = include_str!("../../../scripts/run_download_admission_benchmark.py");

    assert!(
        runner.contains("tempfile.TemporaryFile() as stdout_file"),
        "the runner must spool complete batch stdout outside an OS pipe"
    );
    assert!(
        runner.contains("stdout=stdout_file") && runner.contains("stderr=stderr_file"),
        "the benchmark process must not block while emitting a large batch JSON document"
    );
    assert!(
        !runner.contains("stdout=subprocess.PIPE") && !runner.contains("stderr=subprocess.PIPE"),
        "the runner must not wait for process exit while its output pipe can fill"
    );
}

#[test]
fn scaled_runner_matches_rust_wrapping_seed_for_second_round() {
    let runner =
        include_str!("../../../scripts/run_download_admission_benchmark.py").replace("\r\n", "\n");

    assert!(
        runner.contains(
            "def effective_seed(invocation: BatchInvocation) -> int:\n    return invocation.seed ^ (\n        (invocation.round_number * SEED_MIX_MULTIPLIER) & U64_MASK\n    )"
        ),
        "the Python runner must reproduce Rust u64 wrapping multiplication for round two"
    );
}
