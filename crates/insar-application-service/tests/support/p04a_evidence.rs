use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

const REDACTION_MARKERS: &[&str] = &[
    "authorization:",
    "bearer ",
    "access_token",
    "refresh_token",
    "password",
    "set-cookie",
];

pub fn create_external_attempt_dir(value: String) -> Result<PathBuf, &'static str> {
    let path = PathBuf::from(value);
    if path.as_os_str().is_empty() {
        return Err("LIVE_EVIDENCE_INPUT_REQUIRED");
    }
    let repository = repo_root()?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or("LIVE_EVIDENCE_DESTINATION_UNSAFE")?
        .canonicalize()
        .map_err(|_| "LIVE_EVIDENCE_DESTINATION_UNSAFE")?;
    if parent.starts_with(&repository) {
        return Err("LIVE_EVIDENCE_DESTINATION_UNSAFE");
    }
    if path.exists() {
        let canonical = path
            .canonicalize()
            .map_err(|_| "LIVE_EVIDENCE_DESTINATION_UNSAFE")?;
        if canonical.starts_with(&repository) {
            return Err("LIVE_EVIDENCE_DESTINATION_UNSAFE");
        }
        if !canonical.is_dir()
            || fs::read_dir(&canonical)
                .map_err(|_| "LIVE_EVIDENCE_DESTINATION_UNSAFE")?
                .next()
                .is_some()
        {
            return Err("LIVE_EVIDENCE_DESTINATION_OCCUPIED");
        }
        return Ok(canonical);
    }
    fs::create_dir(&path).map_err(|_| "LIVE_EVIDENCE_DESTINATION_UNSAFE")?;
    let canonical = path
        .canonicalize()
        .map_err(|_| "LIVE_EVIDENCE_DESTINATION_UNSAFE")?;
    if canonical.starts_with(&repository) {
        return Err("LIVE_EVIDENCE_DESTINATION_UNSAFE");
    }
    Ok(canonical)
}

pub fn write_immutable_json(
    directory: &Path,
    file_name: &str,
    value: &Value,
) -> Result<(), String> {
    if !safe_file_name(file_name) {
        return Err("unsafe evidence file name".to_owned());
    }
    let bytes = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    let redacted = String::from_utf8(bytes.clone()).map_err(|error| error.to_string())?;
    let lowercase = redacted.to_ascii_lowercase();
    if REDACTION_MARKERS
        .iter()
        .any(|marker| lowercase.contains(marker))
    {
        return Err("credential material marker found in evidence".to_owned());
    }
    let path = directory.join(file_name);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    file.write_all(&bytes).map_err(|error| error.to_string())?;
    file.write_all(b"\n").map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    Ok(())
}

pub fn sha256_hex(value: impl AsRef<[u8]>) -> String {
    format!("{:x}", Sha256::digest(value.as_ref()))
}

pub fn geojson_bbox(geometry: &Map<String, Value>) -> Result<Vec<f64>, &'static str> {
    let mut points = Vec::new();
    collect_points(&Value::Object(geometry.clone()), &mut points);
    if points.is_empty() {
        return Err("LIVE_AOI_INVALID");
    }
    let mut west = f64::INFINITY;
    let mut east = f64::NEG_INFINITY;
    let mut south = f64::INFINITY;
    let mut north = f64::NEG_INFINITY;
    for (x, y) in points {
        if !x.is_finite() || !y.is_finite() {
            return Err("LIVE_AOI_INVALID");
        }
        west = west.min(x);
        east = east.max(x);
        south = south.min(y);
        north = north.max(y);
    }
    if !(west < east && south < north) {
        return Err("LIVE_AOI_INVALID");
    }
    Ok(vec![west, south, east, north])
}

fn repo_root() -> Result<PathBuf, &'static str> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .ok_or("LIVE_EVIDENCE_DESTINATION_UNSAFE")?
        .canonicalize()
        .map_err(|_| "LIVE_EVIDENCE_DESTINATION_UNSAFE")
}

fn safe_file_name(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && !value.contains(['/', '\\', '\0'])
        && !value.chars().any(char::is_control)
}

fn collect_points(value: &Value, points: &mut Vec<(f64, f64)>) {
    match value {
        Value::Array(values) => {
            let point = if values.len() >= 2 && values[0].is_number() && values[1].is_number() {
                values[0].as_f64().zip(values[1].as_f64())
            } else {
                None
            };
            if let Some(point) = point {
                points.push(point);
                return;
            }
            for value in values {
                collect_points(value, points);
            }
        }
        Value::Object(values) => {
            for (key, value) in values {
                if key == "coordinates" || key == "geometry" {
                    collect_points(value, points);
                }
            }
        }
        _ => {}
    }
}

pub fn evidence_base(
    resource_id: &str,
    capability_id: &str,
    provider: &str,
    product_id: &str,
    attempt_id: &str,
) -> Value {
    serde_json::json!({
        "schema": "capability-admission-evidence.p04a.v1",
        "gate": "P0.4A",
        "attempt_id": attempt_id,
        "resource_id": resource_id,
        "capability_id": capability_id,
        "provider": provider,
        "provider_product_id": product_id,
        "application": "InSAR Studio",
        "application_service_package": "insar-application-service",
        "application_service_version": env!("CARGO_PKG_VERSION"),
        "release_branch": "release/2.2.0-rc1",
        "credential_material_written": false,
        "credential_values_logged": false,
        "credential_redaction_contract": "P0.4A-credential-redaction-v1"
    })
}

pub fn add_object_field(base: &mut Value, key: &str, value: Value) {
    base.as_object_mut()
        .expect("evidence base is an object")
        .insert(key.to_owned(), value);
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    struct TempDirectory(PathBuf);

    impl TempDirectory {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock after Unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "insar-p04a-evidence-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("create evidence test directory");
            Self(path)
        }
    }

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn evidence_directory_rejects_the_repository_itself() {
        let repository = repo_root().expect("integration test repository root");
        assert_eq!(
            create_external_attempt_dir(repository.to_string_lossy().into_owned()),
            Err("LIVE_EVIDENCE_DESTINATION_UNSAFE")
        );
    }

    #[test]
    fn evidence_redaction_rejects_sensitive_markers_before_file_creation() {
        let temp = TempDirectory::new();
        let path = temp.0.join("evidence.json");
        assert!(write_immutable_json(
            &temp.0,
            "evidence.json",
            &serde_json::json!({"access_token": "fixture-secret"}),
        )
        .is_err());
        assert!(!path.exists());
    }

    #[test]
    fn evidence_files_are_immutable_after_the_first_write() {
        let temp = TempDirectory::new();
        let first = serde_json::json!({"attempt_status": "frozen"});
        let replacement = serde_json::json!({"attempt_status": "accepted"});
        write_immutable_json(&temp.0, "evidence.json", &first).expect("first evidence write");
        assert!(write_immutable_json(&temp.0, "evidence.json", &replacement).is_err());
        let stored: Value = serde_json::from_slice(
            &fs::read(temp.0.join("evidence.json")).expect("read immutable evidence"),
        )
        .expect("parse immutable evidence");
        assert_eq!(stored, first);
    }
}
