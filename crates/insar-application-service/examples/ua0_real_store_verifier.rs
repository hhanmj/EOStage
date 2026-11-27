//! Verification-only CoreRestore/page probe for an existing Store.
//!
//! The forensic-clone mode inventories the source files without opening SQLite,
//! requires a short stable interval, copies only the persistent main database
//! and a WAL containing durable bytes, and opens only the isolated clone. The
//! source SHM is never treated as persistent user-data authority or copied.
//! Normal Tauri startup is not used and no source mutation is attempted. The
//! clone workspace is explicit so large verification artifacts never default to
//! the user's system temporary directory.

use std::env;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use insar_application_service::{
    canonical_core_snapshot_value, canonical_workbench_page_value, ApplicationService,
    ApplicationServiceConfig,
};
use insar_core::TaskLifecycleStatus;
use insar_store::StoreWorkbenchPageRequest;
use serde::Serialize;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

const SOURCE_STABILITY_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct FileState {
    /// A redacted source role (`main`, `wal`, or `shm`), never a user path.
    path: String,
    exists: bool,
    size: Option<u64>,
    mtime_ns: Option<u128>,
    sha256: Option<String>,
}

#[derive(Debug)]
struct VerificationFailure {
    reason_code: String,
    message: Option<String>,
}

impl VerificationFailure {
    fn new(reason_code: impl Into<String>) -> Self {
        Self {
            reason_code: reason_code.into(),
            message: None,
        }
    }

    fn with_message(reason_code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            reason_code: reason_code.into(),
            message: Some(message.into()),
        }
    }
}

struct CloneGuard {
    root: PathBuf,
    database: PathBuf,
    wal_copied: bool,
}

impl CloneGuard {
    fn create(
        source: &Path,
        source_files: &[FileState; 3],
        workspace_root: &Path,
    ) -> Result<Self, VerificationFailure> {
        let file_name = source.file_name().ok_or_else(|| {
            VerificationFailure::with_message(
                "V3_STORE_CLONE_FAILED",
                "source Store path has no database filename",
            )
        })?;
        if !workspace_root.is_absolute() {
            return Err(VerificationFailure::with_message(
                "V3_STORE_CLONE_ROOT_INVALID",
                "forensic clone workspace must be an absolute path",
            ));
        }
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        fs::create_dir_all(workspace_root).map_err(|error| {
            VerificationFailure::with_message(
                "V3_STORE_CLONE_FAILED",
                format!("cannot create forensic clone workspace: {error}"),
            )
        })?;
        let root = workspace_root.join(format!(
            "insar-studio-ua0-forensic-clone-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&root).map_err(|error| {
            VerificationFailure::with_message(
                "V3_STORE_CLONE_FAILED",
                format!("cannot create isolated verification directory: {error}"),
            )
        })?;
        let mut guard = Self {
            database: root.join(file_name),
            root,
            wal_copied: false,
        };
        if let Err(error) = fs::copy(source, &guard.database) {
            return Err(VerificationFailure::with_message(
                "V3_STORE_CLONE_FAILED",
                format!("cannot copy the source main database: {error}"),
            ));
        }

        let wal_copied = source_files[1].exists && source_files[1].size.unwrap_or(0) > 0;
        if wal_copied {
            let source_wal = sidecar_path(source, "-wal");
            let clone_wal = sidecar_path(&guard.database, "-wal");
            if let Err(error) = fs::copy(source_wal, clone_wal) {
                return Err(VerificationFailure::with_message(
                    "V3_STORE_CLONE_FAILED",
                    format!("cannot copy the durable WAL into the isolated clone: {error}"),
                ));
            }
        }

        guard.wal_copied = wal_copied;
        Ok(guard)
    }

    fn binding(&self) -> Value {
        json!({
            "mode": "isolated_temporary_directory",
            "source_main_copied": true,
            "source_wal_copied": self.wal_copied,
            "source_shm_copied": false,
            "sqlite_may_create_clone_sidecars": true,
            "clone_discarded_after_verification": true
        })
    }
}

impl Drop for CloneGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(suffix);
    PathBuf::from(value)
}

fn file_state(label: &str, path: &Path) -> FileState {
    let Ok(metadata) = fs::metadata(path) else {
        return FileState {
            path: label.to_owned(),
            exists: false,
            size: None,
            mtime_ns: None,
            sha256: None,
        };
    };
    let is_file = metadata.is_file();
    let mtime_ns = metadata.modified().ok().and_then(|modified| {
        modified
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|duration| duration.as_nanos())
    });
    let sha256 = is_file.then(|| sha256_file(path).ok()).flatten();
    FileState {
        path: label.to_owned(),
        exists: is_file,
        size: is_file.then_some(metadata.len()),
        mtime_ns,
        sha256,
    }
}

fn source_inventory(path: &Path) -> [FileState; 3] {
    [
        file_state("main", path),
        file_state("wal", &sidecar_path(path, "-wal")),
        file_state("shm", &sidecar_path(path, "-shm")),
    ]
}

fn contains_key(value: &Value, key: &str) -> bool {
    match value {
        Value::Object(object) => {
            object.contains_key(key) || object.values().any(|child| contains_key(child, key))
        }
        Value::Array(values) => values.iter().any(|child| contains_key(child, key)),
        _ => false,
    }
}

fn task_lifecycle_counts(tasks: &[insar_store::StoreTaskSummary]) -> Value {
    let mut counts = [
        ("queued", 0_usize),
        ("running", 0),
        ("paused", 0),
        ("interrupted", 0),
        ("failed", 0),
        ("completed", 0),
        ("cancelled", 0),
    ];
    for task in tasks {
        let name = match &task.status {
            TaskLifecycleStatus::Queued => "queued",
            TaskLifecycleStatus::Running => "running",
            TaskLifecycleStatus::Paused => "paused",
            TaskLifecycleStatus::Interrupted => "interrupted",
            TaskLifecycleStatus::Failed => "failed",
            TaskLifecycleStatus::Completed => "completed",
            TaskLifecycleStatus::Cancelled => "cancelled",
        };
        if let Some((_, count)) = counts.iter_mut().find(|(candidate, _)| *candidate == name) {
            *count += 1;
        }
    }
    json!({
        "queued": counts[0].1,
        "running": counts[1].1,
        "paused": counts[2].1,
        "interrupted": counts[3].1,
        "failed": counts[4].1,
        "completed": counts[5].1,
        "cancelled": counts[6].1,
    })
}

fn emit(value: Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(&value).expect("verifier output serializes")
    );
}

fn failure_report(failure: &VerificationFailure, fields: Value) -> Value {
    let mut report = Map::new();
    report.insert("status".to_owned(), json!("blocked"));
    report.insert("reason_code".to_owned(), json!(failure.reason_code));
    if let Some(message) = &failure.message {
        report.insert("message".to_owned(), json!(message));
    }
    if let Value::Object(fields) = fields {
        report.extend(fields);
    }
    Value::Object(report)
}

fn run_restore_probe(path: &Path) -> Result<Value, VerificationFailure> {
    let service = ApplicationService::new(ApplicationServiceConfig::verification_only(path));
    let core_started = std::time::Instant::now();
    let core = service
        .core_restore(true)
        .map_err(|error| VerificationFailure::new(error.code()))?;
    let core_value = canonical_core_snapshot_value(&core)
        .map_err(|_| VerificationFailure::new("V3_APPLICATION_INVALID_SNAPSHOT"))?;
    if contains_key(&core_value, "candidates") {
        return Err(VerificationFailure::with_message(
            "V3_APPLICATION_INVALID_SNAPSHOT",
            "CoreRestore contains a candidate array",
        ));
    }

    let mut page_report = Value::Null;
    if let Some(workbench) = core.core.workbench.as_ref() {
        let page_started = std::time::Instant::now();
        let page = service
            .workbench_page(&StoreWorkbenchPageRequest {
                batch_id: workbench.batch_id.clone(),
                expected_revision: workbench.revision,
                page_index: workbench.page_index,
                page_size: workbench.page_size.clamp(1, 200),
            })
            .map_err(|error| VerificationFailure::new(error.code()))?;
        let page_value = canonical_workbench_page_value(&page)
            .map_err(|_| VerificationFailure::new("V3_APPLICATION_INVALID_SNAPSHOT"))?;
        if contains_key(&page_value, "candidates") {
            return Err(VerificationFailure::with_message(
                "V3_APPLICATION_INVALID_SNAPSHOT",
                "WorkbenchPage contains a candidate array",
            ));
        }
        page_report = json!({
            "elapsed_ms": page_started.elapsed().as_secs_f64() * 1_000.0,
            "payload_bytes": serde_json::to_vec(&page_value).map(|bytes| bytes.len()).unwrap_or(0),
            "batch_id": page.page.batch_id,
            "revision": page.page.revision,
            "page_index": page.page.page_index,
            "page_size": page.page.page_size,
            "native_page_max": 200,
            "ui_page_max": 100,
            "total_count": page.page.total_count,
            "transmitted_items": page.page.items.len(),
        });
    }

    Ok(json!({
        "schema_version": core.core.readiness.schema_version,
        "core": {
            "elapsed_ms": core_started.elapsed().as_secs_f64() * 1_000.0,
            "payload_bytes": serde_json::to_vec(&core_value).map(|bytes| bytes.len()).unwrap_or(0),
            "candidate_count": core.core.workbench.as_ref().map(|workbench| workbench.candidate_count),
            "transmitted_candidates": 0,
        },
        "page": page_report,
        "compatible_task_count": core.core.tasks.len(),
        "legacy_warning_count": core.core.legacy_task_warnings.len(),
        "task_lifecycle_counts": task_lifecycle_counts(&core.core.tasks),
    }))
}

fn args() -> Result<(PathBuf, bool, Option<PathBuf>), Value> {
    let arguments = env::args_os().skip(1).collect::<Vec<_>>();
    let Some(path) = arguments.first().map(PathBuf::from) else {
        return Err(json!({
            "status": "blocked",
            "reason_code": "V3_STORE_PATH_REQUIRED",
            "message": "usage: ua0_real_store_verifier <tasks.sqlite3> [--forensic-clone --clone-root <absolute-directory>]"
        }));
    };
    let mut forensic_clone = false;
    let mut clone_root = None;
    let mut index = 1;
    while index < arguments.len() {
        match arguments[index].to_string_lossy().as_ref() {
            "--forensic-clone" => {
                forensic_clone = true;
                index += 1;
            }
            "--clone-root" => {
                let Some(value) = arguments.get(index + 1) else {
                    return Err(json!({
                        "status": "blocked",
                        "reason_code": "V3_STORE_CLONE_ROOT_REQUIRED",
                        "message": "--clone-root requires an absolute owner verification workspace"
                    }));
                };
                clone_root = Some(PathBuf::from(value));
                index += 2;
            }
            _ => {
                return Err(json!({
                    "status": "blocked",
                    "reason_code": "V3_STORE_ARGUMENT_INVALID",
                    "message": "unknown verifier argument"
                }));
            }
        }
    }
    if forensic_clone && clone_root.is_none() {
        return Err(json!({
            "status": "blocked",
            "reason_code": "V3_STORE_CLONE_ROOT_REQUIRED",
            "message": "forensic clone requires --clone-root on an owner-supplied non-system temporary workspace"
        }));
    }
    Ok((path, forensic_clone, clone_root))
}

fn main() -> ExitCode {
    let (path, forensic_clone, clone_root) = match args() {
        Ok(arguments) => arguments,
        Err(report) => {
            emit(report);
            return ExitCode::from(2);
        }
    };

    let physical_before = source_inventory(&path);
    if !physical_before[0].exists || physical_before[0].size.unwrap_or(0) == 0 {
        emit(json!({
            "status": "blocked",
            "reason_code": "V3_STORE_NOT_FOUND",
            "physical_before": physical_before
        }));
        return ExitCode::from(2);
    }

    let sidecars = physical_before[1..]
        .iter()
        .filter(|state| state.exists)
        .map(|state| state.path.clone())
        .collect::<Vec<_>>();

    if !forensic_clone {
        if !sidecars.is_empty() {
            emit(json!({
                "status": "blocked",
                "reason_code": "V3_STORE_INVALID_SNAPSHOT",
                "message": "verification requires a consistent read without WAL/SHM sidecars; use --forensic-clone for an isolated clone",
                "physical_before": physical_before,
                "sidecars": sidecars
            }));
            return ExitCode::from(2);
        }

        let probe = match run_restore_probe(&path) {
            Ok(probe) => probe,
            Err(failure) => {
                emit(failure_report(
                    &failure,
                    json!({ "physical_before": physical_before }),
                ));
                return ExitCode::from(2);
            }
        };
        let physical_after = source_inventory(&path);
        if physical_before != physical_after {
            emit(failure_report(
                &VerificationFailure::new("V3_STORE_VERIFICATION_MUTATED"),
                json!({
                    "physical_before": physical_before,
                    "physical_after": physical_after,
                    "source_unchanged": false,
                    "verification": probe
                }),
            ));
            return ExitCode::from(3);
        }
        emit(json!({
            "status": "ok",
            "mode": "direct_read_only",
            "physical_before": physical_before,
            "physical_after": physical_after,
            "source_unchanged": true,
            "verification": probe
        }));
        return ExitCode::SUCCESS;
    }

    thread::sleep(SOURCE_STABILITY_INTERVAL);
    let physical_stable = source_inventory(&path);
    if physical_before != physical_stable {
        emit(failure_report(
            &VerificationFailure::new("V3_STORE_SOURCE_CHANGED"),
            json!({
                "message": "source file inventory changed during the stability interval",
                "stability_interval_ms": SOURCE_STABILITY_INTERVAL.as_millis(),
                "physical_before": physical_before,
                "physical_stable": physical_stable,
                "source_unchanged": false,
                "clone_discarded": true
            }),
        ));
        return ExitCode::from(3);
    }

    let clone_root = clone_root.expect("forensic clone requires an explicit clone root");
    let clone = match CloneGuard::create(&path, &physical_stable, &clone_root) {
        Ok(clone) => clone,
        Err(failure) => {
            emit(failure_report(
                &failure,
                json!({
                    "stability_interval_ms": SOURCE_STABILITY_INTERVAL.as_millis(),
                    "physical_before": physical_before,
                    "physical_stable": physical_stable,
                    "source_unchanged": true,
                    "clone_discarded": true
                }),
            ));
            return ExitCode::from(3);
        }
    };
    let clone_binding = clone.binding();
    let physical_after_copy = source_inventory(&path);
    if physical_stable != physical_after_copy {
        drop(clone);
        emit(failure_report(
            &VerificationFailure::new("V3_STORE_SOURCE_CHANGED"),
            json!({
                "message": "source file inventory changed while creating the isolated clone",
                "stability_interval_ms": SOURCE_STABILITY_INTERVAL.as_millis(),
                "physical_before": physical_before,
                "physical_stable": physical_stable,
                "physical_after_copy": physical_after_copy,
                "source_unchanged": false,
                "clone_source_binding": clone_binding,
                "clone_discarded": true
            }),
        ));
        return ExitCode::from(3);
    }

    let verification = run_restore_probe(&clone.database);
    let physical_after = source_inventory(&path);
    let source_unchanged = physical_after_copy == physical_after;
    if !source_unchanged {
        drop(clone);
        emit(failure_report(
            &VerificationFailure::new("V3_STORE_SOURCE_CHANGED"),
            json!({
                "message": "source file inventory changed during clone verification",
                "stability_interval_ms": SOURCE_STABILITY_INTERVAL.as_millis(),
                "physical_before": physical_before,
                "physical_after_copy": physical_after_copy,
                "physical_after": physical_after,
                "source_unchanged": false,
                "clone_source_binding": clone_binding,
                "clone_discarded": true
            }),
        ));
        return ExitCode::from(3);
    }

    drop(clone);
    match verification {
        Ok(verification) => {
            emit(json!({
                "status": "ok",
                "mode": "forensic_clone",
                "source_stability_interval_ms": SOURCE_STABILITY_INTERVAL.as_millis(),
                "source_quiescence_evidence": "stable_main_wal_shm_inventory",
                "physical_before": physical_before,
                "physical_after": physical_after,
                "source_unchanged": true,
                "clone_source_binding": clone_binding,
                "clone_discarded": true,
                "verification": verification
            }));
            ExitCode::SUCCESS
        }
        Err(failure) => {
            emit(failure_report(
                &failure,
                json!({
                    "mode": "forensic_clone",
                    "source_stability_interval_ms": SOURCE_STABILITY_INTERVAL.as_millis(),
                    "source_quiescence_evidence": "stable_main_wal_shm_inventory",
                    "physical_before": physical_before,
                    "physical_after": physical_after,
                    "source_unchanged": true,
                    "clone_source_binding": clone_binding,
                    "clone_discarded": true
                }),
            ));
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forensic_clone_copies_main_and_durable_wal_but_never_source_shm() {
        let root = env::temp_dir().join(format!(
            "insar-studio-ua0-clone-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("test clock is after epoch")
                .as_nanos()
        ));
        fs::create_dir(&root).expect("create clone test source directory");
        let source = root.join("tasks.sqlite3");
        fs::write(&source, b"main").expect("write test main");
        fs::write(sidecar_path(&source, "-wal"), b"durable wal").expect("write test wal");
        fs::write(sidecar_path(&source, "-shm"), b"shared memory").expect("write test shm");
        let source_files = source_inventory(&source);

        let clone_workspace = root.join("clone-workspace");
        let clone = CloneGuard::create(&source, &source_files, &clone_workspace)
            .expect("create forensic clone");
        let clone_database = clone.database.clone();
        let clone_root = clone.root.clone();
        assert!(clone_database.starts_with(&clone_workspace));
        assert!(clone_database.is_file());
        assert!(sidecar_path(&clone_database, "-wal").is_file());
        assert!(!sidecar_path(&clone_database, "-shm").exists());
        assert_eq!(clone.binding()["source_shm_copied"], false);
        drop(clone);
        assert!(!clone_root.exists());
        fs::remove_dir_all(root).expect("remove clone test source directory");
    }
}
