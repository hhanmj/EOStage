use chrono::{SecondsFormat, Utc};
use insar_application_service::{
    apply_datav_geometry_batch, bounded_exact_fallback_units,
    build_datav_geometry_verification_report_with_stats, enumerate_datav_geometry_batch_adcodes,
    enumerate_datav_geometry_expected_units, mark_reusable_pass_evidence,
    verify_datav_geometry_unit, ChinaAdminAuthority, ChinaAdminAuthorityStatus, ContractError,
    DatavBatchResumeState, DatavGeometryBreakdown, DatavGeometryGateStatus, DatavGeometryOutcome,
    DatavGeometryTransportKind, DatavGeometryUnitEvidence, DatavTransportRequestEvidence,
    DatavTransportRequestOutcome,
};
use insar_provider::boundary::{DataVBoundaryProvider, DATAV_BOUNDARY_ROOT};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::thread;
use std::time::{Duration, Instant};

struct Arguments {
    authority: PathBuf,
    report: PathBuf,
    evidence: PathBuf,
    state: PathBuf,
    reuse_evidence: Option<PathBuf>,
    request_spacing: Duration,
    max_exact_fallback_requests: usize,
}

fn main() -> ExitCode {
    match run() {
        Ok(status) => status,
        Err(error) => {
            eprintln!("DataV batch completeness verifier failed: {error}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<ExitCode, String> {
    let arguments = parse_arguments()?;
    let authority =
        ChinaAdminAuthority::from_path(&arguments.authority).map_err(|error| error.to_string())?;
    if authority.authority_status() != ChinaAdminAuthorityStatus::ProductionCandidate {
        return Err(format!(
            "A2-B.1 requires a production_candidate authority, received {:?}",
            authority.authority_status()
        ));
    }

    let expected =
        enumerate_datav_geometry_expected_units(&authority).map_err(|error| error.to_string())?;
    let batch_plan =
        enumerate_datav_geometry_batch_adcodes(&authority).map_err(|error| error.to_string())?;
    if expected.iter().any(|unit| unit.adcode == "710000") {
        return Err("Taiwan 710000 must remain outside the A2-B deterministic route".to_owned());
    }

    let (mut state, mut evidence, started_at) =
        load_or_initialize(&arguments, &authority, &expected, &batch_plan)?;
    state.begin_resume();
    let reused_pass_results = evidence
        .iter()
        .filter(|row| row.reused_prior_pass && row.outcome == DatavGeometryOutcome::Pass)
        .count();
    checkpoint(
        &arguments,
        &authority,
        &expected,
        &state,
        &evidence,
        &started_at,
        reused_pass_results,
    )?;

    let provider = DataVBoundaryProvider::production().map_err(|error| error.to_string())?;
    let mut last_request_started = None;
    let mut stop_requests = false;
    for transport_adcode in state.remaining_batch_adcodes() {
        pace_requests(&mut last_request_started, arguments.request_spacing);
        let request_started = Instant::now();
        let result = provider.resolve_adcode_geometry_batch(&transport_adcode);
        let elapsed_ms = elapsed_ms(request_started);
        let sequence = state.requests.len() + 1;

        match result {
            Ok(batch) => {
                let applied = apply_datav_geometry_batch(
                    authority.authority_version(),
                    &expected,
                    &mut evidence,
                    batch,
                    elapsed_ms,
                )
                .map_err(|error| error.to_string())?;
                state
                    .record_request(DatavTransportRequestEvidence {
                        sequence,
                        transport_kind: DatavGeometryTransportKind::FullBatch,
                        transport_adcode: transport_adcode.clone(),
                        requested_endpoint: batch_endpoint(&transport_adcode),
                        outcome: DatavTransportRequestOutcome::Pass,
                        elapsed_ms,
                        matched_pass: applied.matched_pass,
                        matched_invalid_geometry: applied.matched_invalid_geometry,
                        ignored_non_mca_features: applied.ignored_non_mca_features,
                        error_code: None,
                        error_reason: None,
                    })
                    .map_err(|error| error.to_string())?;
            }
            Err(error) => {
                let outcome = classify_transport_error(&error);
                state
                    .record_request(DatavTransportRequestEvidence {
                        sequence,
                        transport_kind: DatavGeometryTransportKind::FullBatch,
                        transport_adcode: transport_adcode.clone(),
                        requested_endpoint: batch_endpoint(&transport_adcode),
                        outcome,
                        elapsed_ms,
                        matched_pass: 0,
                        matched_invalid_geometry: 0,
                        ignored_non_mca_features: 0,
                        error_code: Some(error.code.clone()),
                        error_reason: Some(contract_error_reason(&error)),
                    })
                    .map_err(|error| error.to_string())?;
            }
        }

        checkpoint(
            &arguments,
            &authority,
            &expected,
            &state,
            &evidence,
            &started_at,
            reused_pass_results,
        )?;
        let latest_outcome = state
            .requests
            .last()
            .expect("request was durably recorded")
            .outcome;
        eprintln!(
            "Batch {}/{}: {} {:?}",
            state.completed_batch_adcodes.len(),
            state.planned_batch_adcodes.len(),
            transport_adcode,
            latest_outcome
        );
        if matches!(
            latest_outcome,
            DatavTransportRequestOutcome::RateLimited
                | DatavTransportRequestOutcome::VerificationBlocked
        ) {
            stop_requests = true;
            break;
        }
    }

    if !stop_requests && state.remaining_batch_adcodes().is_empty() {
        mark_batch_misses_for_fallback(&mut evidence);
        checkpoint(
            &arguments,
            &authority,
            &expected,
            &state,
            &evidence,
            &started_at,
            reused_pass_results,
        )?;
        let fallback = bounded_exact_fallback_units(
            &expected,
            &evidence,
            arguments.max_exact_fallback_requests,
        )
        .map_err(|error| error.to_string())?;
        for unit in fallback {
            pace_requests(&mut last_request_started, arguments.request_spacing);
            let request_started = Instant::now();
            let row = verify_datav_geometry_unit(&provider, authority.authority_version(), &unit);
            let request_outcome = request_outcome_from_unit(&row);
            let sequence = state.requests.len() + 1;
            let row_index = evidence
                .binary_search_by(|candidate| candidate.adcode.cmp(&unit.adcode))
                .map_err(|_| format!("missing evidence row for exact fallback {}", unit.adcode))?;
            evidence[row_index] = row.clone();
            state
                .record_request(DatavTransportRequestEvidence {
                    sequence,
                    transport_kind: DatavGeometryTransportKind::Exact,
                    transport_adcode: unit.adcode.clone(),
                    requested_endpoint: exact_endpoint(&unit.adcode),
                    outcome: request_outcome,
                    elapsed_ms: elapsed_ms(request_started),
                    matched_pass: usize::from(row.outcome == DatavGeometryOutcome::Pass),
                    matched_invalid_geometry: usize::from(
                        row.outcome == DatavGeometryOutcome::InvalidGeometry,
                    ),
                    ignored_non_mca_features: 0,
                    error_code: row.error_code.clone(),
                    error_reason: row.error_reason.clone(),
                })
                .map_err(|error| error.to_string())?;
            checkpoint(
                &arguments,
                &authority,
                &expected,
                &state,
                &evidence,
                &started_at,
                reused_pass_results,
            )?;
            eprintln!("Exact fallback {}: {:?}", unit.adcode, request_outcome);
            if matches!(
                request_outcome,
                DatavTransportRequestOutcome::RateLimited
                    | DatavTransportRequestOutcome::VerificationBlocked
            ) {
                break;
            }
        }
    }

    let report = checkpoint(
        &arguments,
        &authority,
        &expected,
        &state,
        &evidence,
        &started_at,
        reused_pass_results,
    )?;
    print_report(&report, &arguments);
    Ok(match report.a2b_datav_geometry_gate {
        DatavGeometryGateStatus::Pass => ExitCode::SUCCESS,
        DatavGeometryGateStatus::Fail => ExitCode::from(2),
    })
}

fn load_or_initialize(
    arguments: &Arguments,
    authority: &ChinaAdminAuthority,
    expected: &[insar_application_service::DatavGeometryExpectedUnit],
    batch_plan: &[String],
) -> Result<
    (
        DatavBatchResumeState,
        Vec<DatavGeometryUnitEvidence>,
        String,
    ),
    String,
> {
    if arguments.state.exists() {
        let state: DatavBatchResumeState = read_json(&arguments.state)?;
        state
            .validate_against(authority, batch_plan)
            .map_err(|error| error.to_string())?;
        let evidence = read_evidence_jsonl(&arguments.evidence)?;
        let started_at = existing_started_at(&arguments.report)?;
        build_datav_geometry_verification_report_with_stats(
            authority,
            &started_at,
            &Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
            1,
            expected,
            &evidence,
            state.request_stats(evidence.iter().filter(|row| row.reused_prior_pass).count()),
        )
        .map_err(|error| error.to_string())?;
        return Ok((state, evidence, started_at));
    }

    let reuse_path = arguments
        .reuse_evidence
        .as_ref()
        .ok_or_else(|| "--reuse-evidence is required when no resumable state exists".to_owned())?;
    let mut evidence = read_evidence_jsonl(reuse_path)?;
    evidence.sort_by(|left, right| left.adcode.cmp(&right.adcode));
    let reused = mark_reusable_pass_evidence(&mut evidence);
    if reused == 0 {
        return Err("reuse evidence contains no admissible PASS rows".to_owned());
    }
    let state = DatavBatchResumeState::new(authority, batch_plan.to_vec());
    let started_at = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
    build_datav_geometry_verification_report_with_stats(
        authority,
        &started_at,
        &started_at,
        1,
        expected,
        &evidence,
        state.request_stats(reused),
    )
    .map_err(|error| error.to_string())?;
    Ok((state, evidence, started_at))
}

fn checkpoint(
    arguments: &Arguments,
    authority: &ChinaAdminAuthority,
    expected: &[insar_application_service::DatavGeometryExpectedUnit],
    state: &DatavBatchResumeState,
    evidence: &[DatavGeometryUnitEvidence],
    started_at: &str,
    reused_pass_results: usize,
) -> Result<insar_application_service::DatavGeometryVerificationReport, String> {
    let report = build_datav_geometry_verification_report_with_stats(
        authority,
        started_at,
        &Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
        1,
        expected,
        evidence,
        state.request_stats(reused_pass_results),
    )
    .map_err(|error| error.to_string())?;
    write_evidence_jsonl(&arguments.evidence, evidence)?;
    write_json(&arguments.state, state)?;
    write_json(&arguments.report, &report)?;
    Ok(report)
}

fn mark_batch_misses_for_fallback(evidence: &mut [DatavGeometryUnitEvidence]) {
    for row in evidence {
        if row.outcome == DatavGeometryOutcome::VerificationBlocked {
            *row = DatavGeometryUnitEvidence::failed(
                row,
                DatavGeometryOutcome::VerificationBlocked,
                "NOT_FOUND_IN_DATAV_BATCH_SET",
                "bounded_exact_fallback_required",
                0,
            );
        }
    }
}

fn classify_transport_error(error: &ContractError) -> DatavTransportRequestOutcome {
    let http_status = error
        .debug_context
        .get("http_status")
        .and_then(serde_json::Value::as_u64);
    match (error.code.as_str(), http_status) {
        ("BOUNDARY_PROVIDER_RATE_LIMITED", _) => DatavTransportRequestOutcome::RateLimited,
        ("BOUNDARY_PROVIDER_HTTP_ERROR", Some(404 | 410)) => {
            DatavTransportRequestOutcome::Unavailable
        }
        ("BOUNDARY_PROVIDER_UNAVAILABLE", _) | ("BOUNDARY_PROVIDER_HTTP_ERROR", _) => {
            DatavTransportRequestOutcome::VerificationBlocked
        }
        _ => DatavTransportRequestOutcome::InvalidResponse,
    }
}

fn request_outcome_from_unit(row: &DatavGeometryUnitEvidence) -> DatavTransportRequestOutcome {
    match row.outcome {
        DatavGeometryOutcome::Pass => DatavTransportRequestOutcome::Pass,
        DatavGeometryOutcome::Unavailable => DatavTransportRequestOutcome::Unavailable,
        DatavGeometryOutcome::InvalidGeometry => DatavTransportRequestOutcome::InvalidResponse,
        DatavGeometryOutcome::VerificationBlocked
            if row.error_code.as_deref() == Some("BOUNDARY_PROVIDER_RATE_LIMITED") =>
        {
            DatavTransportRequestOutcome::RateLimited
        }
        DatavGeometryOutcome::VerificationBlocked => {
            DatavTransportRequestOutcome::VerificationBlocked
        }
    }
}

fn pace_requests(last_started: &mut Option<Instant>, spacing: Duration) {
    if let Some(started) = last_started {
        let elapsed = started.elapsed();
        if elapsed < spacing {
            thread::sleep(spacing - elapsed);
        }
    }
    *last_started = Some(Instant::now());
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u64::MAX as u128) as u64
}

fn contract_error_reason(error: &ContractError) -> String {
    error
        .debug_context
        .get("reason")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("provider_error")
        .to_owned()
}

fn parse_arguments() -> Result<Arguments, String> {
    let mut values = env::args().skip(1);
    let mut authority = None;
    let mut report = None;
    let mut evidence = None;
    let mut state = None;
    let mut reuse_evidence = None;
    let mut request_spacing_ms = 1_000_u64;
    let mut max_exact_fallback_requests = 128_usize;
    while let Some(argument) = values.next() {
        let value = values
            .next()
            .ok_or_else(|| format!("missing value for {argument}"))?;
        match argument.as_str() {
            "--authority" => authority = Some(PathBuf::from(value)),
            "--report" => report = Some(PathBuf::from(value)),
            "--evidence" => evidence = Some(PathBuf::from(value)),
            "--state" => state = Some(PathBuf::from(value)),
            "--reuse-evidence" => reuse_evidence = Some(PathBuf::from(value)),
            "--request-spacing-ms" => {
                request_spacing_ms = value
                    .parse::<u64>()
                    .map_err(|_| "--request-spacing-ms must be an integer".to_owned())?;
                if !(250..=60_000).contains(&request_spacing_ms) {
                    return Err("--request-spacing-ms must be between 250 and 60000".to_owned());
                }
            }
            "--max-exact-fallback-requests" => {
                max_exact_fallback_requests = value
                    .parse::<usize>()
                    .map_err(|_| "--max-exact-fallback-requests must be an integer".to_owned())?;
                if !(1..=256).contains(&max_exact_fallback_requests) {
                    return Err(
                        "--max-exact-fallback-requests must be between 1 and 256".to_owned()
                    );
                }
            }
            _ => return Err(format!("unknown argument {argument}")),
        }
    }
    Ok(Arguments {
        authority: authority.ok_or_else(|| "--authority is required".to_owned())?,
        report: report.ok_or_else(|| "--report is required".to_owned())?,
        evidence: evidence.ok_or_else(|| "--evidence is required".to_owned())?,
        state: state.ok_or_else(|| "--state is required".to_owned())?,
        reuse_evidence,
        request_spacing: Duration::from_millis(request_spacing_ms),
        max_exact_fallback_requests,
    })
}

fn print_report(
    report: &insar_application_service::DatavGeometryVerificationReport,
    arguments: &Arguments,
) {
    println!("Total expected: {}", report.total_expected);
    println!("Geometry PASS: {}", report.geometry_pass);
    println!("Unavailable: {}", report.unavailable_adcodes.len());
    println!(
        "Invalid geometry: {}",
        report.invalid_geometry_adcodes.len()
    );
    println!("Verification blocked: {}", report.verification_blocked);
    println!("Batch requests: {}", report.batch_requests);
    println!(
        "Exact fallback requests: {}",
        report.exact_fallback_requests
    );
    println!("Rate-limit events: {}", report.rate_limit_events);
    print_breakdown("Country", &report.breakdown["country"]);
    print_breakdown("Province", &report.breakdown["province"]);
    print_breakdown("Prefecture-level", &report.breakdown["prefecture"]);
    print_breakdown("County-level", &report.breakdown["county"]);
    print_breakdown("Special hierarchy", &report.breakdown["special_hierarchy"]);
    println!(
        "A2-B DataV geometry gate: {}",
        match report.a2b_datav_geometry_gate {
            DatavGeometryGateStatus::Pass => "PASS",
            DatavGeometryGateStatus::Fail => "FAIL",
        }
    );
    println!("Aggregate report: {}", arguments.report.display());
    println!("Per-unit evidence: {}", arguments.evidence.display());
    println!("Resume state: {}", arguments.state.display());
}

fn print_breakdown(label: &str, value: &DatavGeometryBreakdown) {
    println!(
        "{label}: expected {}, pass {}, fail {}, unavailable {}, invalid geometry {}, verification blocked {}",
        value.total_expected,
        value.geometry_pass,
        value.geometry_fail,
        value.unavailable,
        value.invalid_geometry,
        value.verification_blocked
    );
}

fn exact_endpoint(adcode: &str) -> String {
    format!("{DATAV_BOUNDARY_ROOT}/{adcode}.json")
}

fn batch_endpoint(adcode: &str) -> String {
    format!("{DATAV_BOUNDARY_ROOT}/{adcode}_full.json")
}

fn read_evidence_jsonl(path: &Path) -> Result<Vec<DatavGeometryUnitEvidence>, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).map_err(|error| error.to_string()))
        .collect()
}

fn write_evidence_jsonl(path: &Path, evidence: &[DatavGeometryUnitEvidence]) -> Result<(), String> {
    let mut output = String::new();
    for row in evidence {
        output.push_str(&serde_json::to_string(row).map_err(|error| error.to_string())?);
        output.push('\n');
    }
    write_text(path, &output)
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    serde_json::from_str(&text)
        .map_err(|error| format!("could not parse {}: {error}", path.display()))
}

fn write_json(path: &Path, value: &impl serde::Serialize) -> Result<(), String> {
    let text = format!(
        "{}\n",
        serde_json::to_string_pretty(value).map_err(|error| error.to_string())?
    );
    write_text(path, &text)
}

fn existing_started_at(report_path: &Path) -> Result<String, String> {
    let value: serde_json::Value = read_json(report_path)?;
    value
        .get("started_at")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| "existing report is missing started_at".to_owned())
}

fn write_text(path: &Path, value: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    fs::write(path, value).map_err(|error| format!("could not write {}: {error}", path.display()))
}
