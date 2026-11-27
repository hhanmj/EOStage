use chrono::{SecondsFormat, Utc};
use insar_application_service::{ChinaAdminAuthority, ChinaAdminAuthorityGateStatus};
use serde::Serialize;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Serialize)]
struct VerificationEnvelope<'a> {
    verification_schema_version: &'static str,
    verified_at: String,
    authority_path: String,
    report: &'a insar_application_service::ChinaAdminAuthorityVerificationReport,
}

struct Arguments {
    authority: PathBuf,
    report: PathBuf,
    evidence: PathBuf,
}

fn main() -> ExitCode {
    match run() {
        Ok(status) => status,
        Err(error) => {
            eprintln!("China administrative authority verifier failed: {error}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<ExitCode, String> {
    let arguments = parse_arguments()?;
    let authority =
        ChinaAdminAuthority::from_path(&arguments.authority).map_err(|error| error.to_string())?;
    let report = authority.verify_exhaustive();
    let envelope = VerificationEnvelope {
        verification_schema_version: "1.2.0",
        verified_at: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
        authority_path: arguments.authority.to_string_lossy().replace('\\', "/"),
        report: &report,
    };

    write_text(
        &arguments.report,
        &format!(
            "{}\n",
            serde_json::to_string_pretty(&envelope).map_err(|error| error.to_string())?
        ),
    )?;
    write_text(
        &arguments.evidence,
        &report.evidence_jsonl().map_err(|error| error.to_string())?,
    )?;

    for line in report.summary_lines() {
        println!("{line}");
    }
    println!("Aggregate report: {}", arguments.report.display());
    println!("Per-unit evidence: {}", arguments.evidence.display());

    Ok(match report.structural_gate_status() {
        ChinaAdminAuthorityGateStatus::Pass => ExitCode::SUCCESS,
        ChinaAdminAuthorityGateStatus::Fail => ExitCode::from(2),
    })
}

fn parse_arguments() -> Result<Arguments, String> {
    let mut values = env::args().skip(1);
    let mut authority = None;
    let mut report = None;
    let mut evidence = None;
    while let Some(argument) = values.next() {
        let value = values
            .next()
            .ok_or_else(|| format!("missing value for {argument}"))?;
        match argument.as_str() {
            "--authority" => authority = Some(PathBuf::from(value)),
            "--report" => report = Some(PathBuf::from(value)),
            "--evidence" => evidence = Some(PathBuf::from(value)),
            _ => return Err(format!("unknown argument {argument}")),
        }
    }
    Ok(Arguments {
        authority: authority.ok_or_else(|| "--authority is required".to_owned())?,
        report: report.ok_or_else(|| "--report is required".to_owned())?,
        evidence: evidence.ok_or_else(|| "--evidence is required".to_owned())?,
    })
}

fn write_text(path: &Path, value: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    fs::write(path, value).map_err(|error| format!("could not write {}: {error}", path.display()))
}
