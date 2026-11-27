use super::{NativeGisError, NativeGisErrorCode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VerticalGridReadiness {
    Ready,
    Missing,
    IdentityMismatch,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VerticalGridManifest {
    pub grid_id: String,
    pub model: String,
    pub vertical_crs: String,
    pub target_vertical_crs: String,
    pub source_horizontal_crs: String,
    pub source_vertical_crs: String,
    pub target_horizontal_crs: String,
    pub direction: String,
    pub filename: String,
    pub version: String,
    pub source: String,
    pub sha256: String,
    pub expected_bytes: u64,
    pub coverage: String,
    pub resolution: String,
    pub license: String,
    pub bundled: bool,
    pub production_admitted: bool,
    pub applicable_transformations: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct VerticalGridRegistry {
    root: Option<PathBuf>,
    entries: BTreeMap<String, VerticalGridManifest>,
    verified: Arc<Mutex<BTreeMap<String, VerifiedGridIdentity>>>,
}

#[derive(Clone, Debug)]
struct VerifiedGridIdentity {
    manifest_sha256: String,
    bytes: u64,
    modified: SystemTime,
}

impl VerticalGridRegistry {
    pub fn load(root: impl AsRef<Path>) -> Result<Self, NativeGisError> {
        let root = root.as_ref().to_path_buf();
        let manifest_path = root.join("manifest.json");
        if !manifest_path.is_file() {
            return Ok(Self::default());
        }
        let bytes = fs::read(&manifest_path)
            .map_err(|_| mismatch("vertical grid manifest cannot be read"))?;
        let entries: Vec<VerticalGridManifest> = serde_json::from_slice(&bytes)
            .map_err(|_| mismatch("vertical grid manifest is invalid"))?;
        let mut registry = Self {
            root: Some(root),
            entries: BTreeMap::new(),
            verified: Arc::new(Mutex::new(BTreeMap::new())),
        };
        for entry in entries {
            validate_entry(&entry)?;
            if registry
                .entries
                .insert(entry.model.to_ascii_uppercase(), entry)
                .is_some()
            {
                return Err(mismatch("vertical grid models must be unique"));
            }
        }
        Ok(registry)
    }

    pub fn entries(&self) -> impl Iterator<Item = &VerticalGridManifest> {
        self.entries.values()
    }

    pub fn readiness(&self, model: &str) -> VerticalGridReadiness {
        let Some(entry) = self.entries.get(&model.to_ascii_uppercase()) else {
            return VerticalGridReadiness::Missing;
        };
        if !entry.bundled {
            return VerticalGridReadiness::Missing;
        }
        let Some(root) = self.root.as_ref() else {
            return VerticalGridReadiness::Missing;
        };
        let path = root.join(&entry.filename);
        let Ok(metadata) = fs::metadata(&path) else {
            return VerticalGridReadiness::Missing;
        };
        if !metadata.is_file() || metadata.len() != entry.expected_bytes {
            return VerticalGridReadiness::IdentityMismatch;
        }
        let modified = metadata.modified().ok();
        if let Some(modified) = modified {
            if self
                .verified
                .lock()
                .ok()
                .and_then(|verified| {
                    verified
                        .get(&entry.model.to_ascii_uppercase())
                        .filter(|identity| {
                            identity.manifest_sha256.eq_ignore_ascii_case(&entry.sha256)
                                && identity.bytes == entry.expected_bytes
                                && identity.modified == modified
                        })
                        .cloned()
                })
                .is_some()
            {
                return VerticalGridReadiness::Ready;
            }
        }
        let Ok(actual_sha256) = sha256_file(&path) else {
            return VerticalGridReadiness::IdentityMismatch;
        };
        if actual_sha256.eq_ignore_ascii_case(&entry.sha256) {
            if let Some(modified) = modified {
                if let Ok(mut verified) = self.verified.lock() {
                    verified.insert(
                        entry.model.to_ascii_uppercase(),
                        VerifiedGridIdentity {
                            manifest_sha256: entry.sha256.clone(),
                            bytes: entry.expected_bytes,
                            modified,
                        },
                    );
                }
            }
            VerticalGridReadiness::Ready
        } else {
            if let Ok(mut verified) = self.verified.lock() {
                verified.remove(&entry.model.to_ascii_uppercase());
            }
            VerticalGridReadiness::IdentityMismatch
        }
    }

    pub fn require(&self, model: &str) -> Result<&VerticalGridManifest, NativeGisError> {
        let Some(entry) = self.entries.get(&model.to_ascii_uppercase()) else {
            return Err(missing(format!("vertical grid {model} is not registered")));
        };
        match self.readiness(model) {
            VerticalGridReadiness::Ready => {
                if entry.production_admitted {
                    Ok(entry)
                } else {
                    Err(not_admitted(format!(
                        "vertical grid {model} is valid but not production-admitted"
                    )))
                }
            }
            VerticalGridReadiness::Missing => {
                Err(missing(format!("vertical grid {model} is not bundled")))
            }
            VerticalGridReadiness::IdentityMismatch => Err(mismatch(format!(
                "vertical grid {model} identity does not match its manifest"
            ))),
        }
    }

    pub fn required_path(&self, model: &str) -> Result<PathBuf, NativeGisError> {
        let entry = self.require(model)?;
        let root = self
            .root
            .as_ref()
            .ok_or_else(|| missing(format!("vertical grid {model} is not bundled")))?;
        Ok(root.join(&entry.filename))
    }

    #[cfg(test)]
    fn verified_model_count(&self) -> usize {
        self.verified
            .lock()
            .map(|verified| verified.len())
            .unwrap_or(0)
    }
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut digest = Sha256::new();
    // Keep the one-megabyte streaming buffer on the heap. The packaged
    // Windows executable's main thread has a comparatively small stack, and
    // placing this buffer there made the first requested grid verification
    // terminate with STATUS_STACK_OVERFLOW before any DEM work could start.
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn validate_entry(entry: &VerticalGridManifest) -> Result<(), NativeGisError> {
    if entry.grid_id.trim().is_empty()
        || entry.model.trim().is_empty()
        || entry.vertical_crs.trim().is_empty()
        || entry.target_vertical_crs.trim().is_empty()
        || entry.source_horizontal_crs.trim().is_empty()
        || entry.source_vertical_crs.trim().is_empty()
        || entry.target_horizontal_crs.trim().is_empty()
        || entry.direction.trim().is_empty()
        || entry.version.trim().is_empty()
        || entry.source.trim().is_empty()
        || entry.coverage.trim().is_empty()
        || entry.resolution.trim().is_empty()
        || entry.license.trim().is_empty()
        || entry.expected_bytes == 0
        || entry.sha256.len() != 64
        || !safe_relative_path(&entry.filename)
        || entry.applicable_transformations.is_empty()
        || (entry.production_admitted && !entry.bundled)
    {
        return Err(mismatch(
            "vertical grid identity or provenance is incomplete",
        ));
    }
    Ok(())
}

fn safe_relative_path(value: &str) -> bool {
    let path = Path::new(value);
    !path.as_os_str().is_empty()
        && path.components().all(|component| {
            !matches!(
                component,
                Component::RootDir | Component::Prefix(_) | Component::ParentDir
            )
        })
}

fn missing(reason: impl Into<String>) -> NativeGisError {
    NativeGisError::contract(NativeGisErrorCode::VerticalGridMissing, reason)
}

fn mismatch(reason: impl Into<String>) -> NativeGisError {
    NativeGisError::contract(NativeGisErrorCode::VerticalGridIdentityMismatch, reason)
}

fn not_admitted(reason: impl Into<String>) -> NativeGisError {
    NativeGisError::contract(
        NativeGisErrorCode::VerticalGridNotProductionAdmitted,
        reason,
    )
}

#[cfg(test)]
mod tests {
    use super::{VerticalGridReadiness, VerticalGridRegistry};
    use serde_json::json;
    use sha2::{Digest, Sha256};
    use std::fs;

    #[test]
    fn egm2008_missing_is_typed_and_does_not_fallback_to_egm96() {
        let root = std::env::temp_dir().join(format!(
            "insar-studio-vertical-grid-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("test directory");
        fs::write(
            root.join("manifest.json"),
            serde_json::to_vec(&json!([{
                "grid_id": "egm2008-global",
                "model": "EGM2008",
                "vertical_crs": "EGM2008",
                "target_vertical_crs": "EPSG:3855",
                "source_horizontal_crs": "EPSG:4326",
                "source_vertical_crs": "EPSG:4979",
                "target_horizontal_crs": "EPSG:4326",
                "direction": "bidirectional",
                "filename": "egm2008.npz",
                "version": "fixture",
                "source": "fixture-source",
                "sha256": "0000000000000000000000000000000000000000000000000000000000000000",
                "expected_bytes": 1,
                "coverage": "world",
                "resolution": "fixture",
                "license": "pending-reviewed-source",
                "bundled": true,
                "production_admitted": false,
                "applicable_transformations": ["EGM2008_TO_WGS84_ELLIPSOID"]
            }]))
            .expect("manifest JSON"),
        )
        .expect("manifest file");
        let registry = VerticalGridRegistry::load(&root).expect("registry");
        assert_eq!(
            registry.readiness("EGM2008"),
            VerticalGridReadiness::Missing
        );
        assert_eq!(
            registry.require("EGM2008").unwrap_err().code().as_str(),
            "VERTICAL_GRID_MISSING"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn reviewed_manifest_retains_explicit_vertical_contract_and_admission() {
        let root = std::env::temp_dir().join(format!(
            "insar-studio-vertical-grid-contract-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("test directory");
        fs::write(
            root.join("manifest.json"),
            serde_json::to_vec(&json!([{
                "grid_id": "egm2008-global",
                "model": "EGM2008",
                "vertical_crs": "EPSG:3855",
                "target_vertical_crs": "EPSG:3855",
                "source_horizontal_crs": "EPSG:4326",
                "source_vertical_crs": "EPSG:4979",
                "target_horizontal_crs": "EPSG:4326",
                "direction": "bidirectional",
                "filename": "egm2008.tif",
                "version": "nga-reviewed-2019",
                "source": "NGA-derived PROJ GeoTIFF",
                "sha256": "0000000000000000000000000000000000000000000000000000000000000000",
                "expected_bytes": 1,
                "coverage": "world",
                "resolution": "2.5 arc-minute",
                "license": "public-domain",
                "bundled": true,
                "production_admitted": true,
                "applicable_transformations": [
                    "WGS84_ELLIPSOIDAL_TO_EGM2008",
                    "EGM2008_TO_WGS84_ELLIPSOIDAL"
                ]
            }]))
            .expect("manifest JSON"),
        )
        .expect("manifest file");
        let registry = VerticalGridRegistry::load(&root).expect("reviewed registry");
        let entry = registry.entries().next().expect("one reviewed entry");
        assert_eq!(entry.target_vertical_crs, "EPSG:3855");
        assert_eq!(entry.source_horizontal_crs, "EPSG:4326");
        assert_eq!(entry.source_vertical_crs, "EPSG:4979");
        assert_eq!(entry.target_horizontal_crs, "EPSG:4326");
        assert_eq!(entry.direction, "bidirectional");
        assert_eq!(entry.coverage, "world");
        assert_eq!(entry.resolution, "2.5 arc-minute");
        assert!(entry.production_admitted);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn production_admission_requires_a_bundled_grid() {
        let root = std::env::temp_dir().join(format!(
            "insar-studio-vertical-grid-admission-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("test directory");
        fs::write(
            root.join("manifest.json"),
            serde_json::to_vec(&json!([{
                "grid_id": "egm96-global",
                "model": "EGM96",
                "vertical_crs": "EPSG:5773",
                "target_vertical_crs": "EPSG:5773",
                "source_horizontal_crs": "EPSG:4326",
                "source_vertical_crs": "EPSG:4979",
                "target_horizontal_crs": "EPSG:4326",
                "direction": "bidirectional",
                "filename": "egm96.tif",
                "version": "fixture",
                "source": "fixture-source",
                "sha256": "0000000000000000000000000000000000000000000000000000000000000000",
                "expected_bytes": 1,
                "coverage": "world",
                "resolution": "15 arc-minute",
                "license": "fixture-license",
                "bundled": false,
                "production_admitted": true,
                "applicable_transformations": ["EGM96_TO_WGS84_ELLIPSOID"]
            }]))
            .expect("manifest JSON"),
        )
        .expect("manifest file");
        let error = VerticalGridRegistry::load(&root)
            .expect_err("an unbundled grid cannot be production-admitted");
        assert_eq!(error.code().as_str(), "VERTICAL_GRID_IDENTITY_MISMATCH");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn require_rejects_a_valid_but_non_admitted_grid() {
        let root = std::env::temp_dir().join(format!(
            "insar-studio-vertical-grid-non-admitted-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("test directory");
        let grid = b"valid-fixture-grid";
        fs::write(root.join("egm96.tif"), grid).expect("grid file");
        let mut digest = Sha256::new();
        digest.update(grid);
        fs::write(
            root.join("manifest.json"),
            serde_json::to_vec(&json!([{
                "grid_id": "egm96-global",
                "model": "EGM96",
                "vertical_crs": "EPSG:5773",
                "target_vertical_crs": "EPSG:5773",
                "source_horizontal_crs": "EPSG:4326",
                "source_vertical_crs": "EPSG:4979",
                "target_horizontal_crs": "EPSG:4326",
                "direction": "bidirectional",
                "filename": "egm96.tif",
                "version": "fixture",
                "source": "fixture-source",
                "sha256": format!("{:x}", digest.finalize()),
                "expected_bytes": grid.len(),
                "coverage": "world",
                "resolution": "15 arc-minute",
                "license": "fixture-license",
                "bundled": true,
                "production_admitted": false,
                "applicable_transformations": ["EGM96_TO_WGS84_ELLIPSOID"]
            }]))
            .expect("manifest JSON"),
        )
        .expect("manifest file");
        let registry = VerticalGridRegistry::load(&root).expect("registry");
        assert_eq!(registry.readiness("EGM96"), VerticalGridReadiness::Ready);
        let error = registry
            .require("EGM96")
            .expect_err("a valid but non-admitted grid must not be production-required");
        assert_eq!(
            error.code().as_str(),
            "VERTICAL_GRID_NOT_PRODUCTION_ADMITTED"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn requested_grid_verification_is_capability_specific_and_cached() {
        let root = std::env::temp_dir().join(format!(
            "insar-studio-vertical-grid-lazy-cache-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("test directory");
        let egm96 = b"verified-egm96-grid";
        let egm2008 = b"verified-egm2008-grid";
        fs::write(root.join("egm96.tif"), egm96).expect("EGM96 grid");
        fs::write(root.join("egm2008.tif"), egm2008).expect("EGM2008 grid");
        let digest = |bytes: &[u8]| {
            let mut digest = Sha256::new();
            digest.update(bytes);
            format!("{:x}", digest.finalize())
        };
        let entry = |model: &str, filename: &str, bytes: &[u8]| {
            json!({
                "grid_id": format!("{}-fixture", model.to_ascii_lowercase()),
                "model": model,
                "vertical_crs": if model == "EGM96" { "EPSG:5773" } else { "EPSG:3855" },
                "target_vertical_crs": if model == "EGM96" { "EPSG:5773" } else { "EPSG:3855" },
                "source_horizontal_crs": "EPSG:4326",
                "source_vertical_crs": "EPSG:4979",
                "target_horizontal_crs": "EPSG:4326",
                "direction": "bidirectional",
                "filename": filename,
                "version": "fixture",
                "source": "fixture-source",
                "sha256": digest(bytes),
                "expected_bytes": bytes.len(),
                "coverage": "world",
                "resolution": "fixture",
                "license": "fixture-license",
                "bundled": true,
                "production_admitted": true,
                "applicable_transformations": [format!("{model}_TO_WGS84_ELLIPSOID")]
            })
        };
        fs::write(
            root.join("manifest.json"),
            serde_json::to_vec(&json!([
                entry("EGM96", "egm96.tif", egm96),
                entry("EGM2008", "egm2008.tif", egm2008)
            ]))
            .expect("manifest JSON"),
        )
        .expect("manifest file");
        let registry = VerticalGridRegistry::load(&root).expect("registry");

        registry.require("EGM96").expect("requested EGM96 verifies");
        assert_eq!(registry.verified_model_count(), 1);
        registry.require("EGM96").expect("EGM96 cache hit");
        assert_eq!(registry.verified_model_count(), 1);
        registry
            .require("EGM2008")
            .expect("requested EGM2008 verifies");
        assert_eq!(registry.verified_model_count(), 2);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn requested_grid_hashing_fits_the_packaged_windows_main_thread_stack_budget() {
        let root = std::env::temp_dir().join(format!(
            "insar-studio-vertical-grid-stack-budget-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("test directory");
        let grid = b"stack-budget-grid";
        fs::write(root.join("egm96.tif"), grid).expect("grid file");
        let mut digest = Sha256::new();
        digest.update(grid);
        fs::write(
            root.join("manifest.json"),
            serde_json::to_vec(&json!([{
                "grid_id": "egm96-stack-budget",
                "model": "EGM96",
                "vertical_crs": "EPSG:5773",
                "target_vertical_crs": "EPSG:5773",
                "source_horizontal_crs": "EPSG:4326",
                "source_vertical_crs": "EPSG:4979",
                "target_horizontal_crs": "EPSG:4326",
                "direction": "bidirectional",
                "filename": "egm96.tif",
                "version": "fixture",
                "source": "fixture-source",
                "sha256": format!("{:x}", digest.finalize()),
                "expected_bytes": grid.len(),
                "coverage": "world",
                "resolution": "fixture",
                "license": "fixture-license",
                "bundled": true,
                "production_admitted": true,
                "applicable_transformations": ["EGM96_TO_WGS84_ELLIPSOID"]
            }]))
            .expect("manifest JSON"),
        )
        .expect("manifest file");
        let registry = VerticalGridRegistry::load(&root).expect("registry");
        let readiness = std::thread::Builder::new()
            .stack_size(512 * 1024)
            .spawn(move || registry.readiness("EGM96"))
            .expect("bounded-stack verification thread")
            .join()
            .expect("grid verification must not overflow the bounded stack");
        assert_eq!(readiness, VerticalGridReadiness::Ready);
        let _ = fs::remove_dir_all(root);
    }
}
