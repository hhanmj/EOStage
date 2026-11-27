//! Persistent transfer engine preferences and smart concurrency tuning.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{OnceLock, RwLock};

use serde::{Deserialize, Serialize};

use crate::disk_guard::VolumeSpace;
use crate::segment_planner::SegmentLevel;

/// File name under `app_data_dir` storing persisted transfer engine settings.
pub const TRANSFER_ENGINE_PREFS_FILENAME: &str = "transfer_engine_prefs.json";
/// Maximum configurable global download speed cap (`1 GiB/s` = `1_048_576 KiB/s`).
pub const MAX_DOWNLOAD_SPEED_KIB_S_LIMIT: u64 = 1_048_576;
/// Subdirectory under `app_data_dir` used for SSD staging when `ssd_staging_dir` is blank.
/// Intentionally separate from `cache/` so disposable cache cleanups never remove in-flight `.part` files.
pub const DEFAULT_SSD_STAGING_SUBDIR: &str = "ssd_staging";

/// Minimum free space (`2 GiB`) below which smart concurrency throttles to 1 task.
const LOW_DISK_THROTTLE_ONE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Free space threshold (`10 GiB`) below which smart concurrency caps at 2 tasks.
const MODERATE_DISK_CAP_TWO_BYTES: u64 = 10 * 1024 * 1024 * 1024;

static ACTIVE_DEFAULT_SSD_STAGING_DIR: OnceLock<RwLock<Option<PathBuf>>> = OnceLock::new();

fn default_ssd_staging_dir_slot() -> &'static RwLock<Option<PathBuf>> {
    ACTIVE_DEFAULT_SSD_STAGING_DIR.get_or_init(|| RwLock::new(None))
}

/// Returns the isolated SSD staging directory under `app_data_dir`.
pub fn default_ssd_staging_dir_for_app_data(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(DEFAULT_SSD_STAGING_SUBDIR)
}

/// Returns the active default SSD staging directory when initialized for an `app_data_dir`.
pub fn active_default_ssd_staging_dir() -> Option<PathBuf> {
    default_ssd_staging_dir_slot()
        .read()
        .ok()
        .and_then(|guard| guard.clone())
}

/// Updates the active default SSD staging directory (`None` clears it).
pub fn set_active_default_ssd_staging_dir(dir: Option<PathBuf>) {
    if let Ok(mut guard) = default_ssd_staging_dir_slot().write() {
        *guard = dir;
    }
}

/// User-configurable preferences for the upgraded smart download engine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TransferEnginePrefs {
    /// Tier 1: Global maximum concurrent active download tasks (`1..=8`, unified across SAR/Optical/DEM/Orbit).
    pub max_concurrent_tasks: u8,
    /// Dynamically tune active download task concurrency based on disk & network headroom.
    pub smart_concurrency: bool,
    /// Tier 2: HTTP Range connections per file (`1`, `2`, `4`, or `8`) for files >= 16 MiB.
    pub per_file_segments: u8,
    /// Tier 3: Concurrent request workers for global XYZ satellite tiles and GEE chunked raster fetches (`8`, `16`, `32`, or `64`).
    pub tile_chunk_concurrency: u8,
    /// Abort and reconnect streams stuck below 32 KiB/s for 15s (up to 12 times).
    pub stall_reconnect_enabled: bool,
    /// Stage `.part` downloads in an SSD directory before promoting to the target volume.
    pub ssd_staging_enabled: bool,
    /// Automatically engage SSD staging when the destination resides on a slow disk/share.
    pub ssd_staging_auto_slow_disk: bool,
    /// Optional explicit directory used as the high-speed SSD staging cache.
    pub ssd_staging_dir: Option<String>,
    /// Global download speed limit across main file transfers in KiB/s (`0` = unlimited).
    pub max_download_speed_kib_s: u64,
}

impl Default for TransferEnginePrefs {
    fn default() -> Self {
        Self {
            max_concurrent_tasks: 2,
            smart_concurrency: true,
            per_file_segments: 4,
            tile_chunk_concurrency: 16,
            stall_reconnect_enabled: true,
            ssd_staging_enabled: false,
            ssd_staging_auto_slow_disk: true,
            ssd_staging_dir: None,
            max_download_speed_kib_s: 0,
        }
    }
}

impl TransferEnginePrefs {
    /// Normalizes enum-like integer bounds and trims blank path strings.
    pub fn normalized(mut self) -> Self {
        self.max_concurrent_tasks = normalize_max_concurrent_tasks(self.max_concurrent_tasks);
        self.per_file_segments = normalize_per_file_segments(self.per_file_segments);
        self.tile_chunk_concurrency = normalize_tile_chunk_concurrency(self.tile_chunk_concurrency);
        self.ssd_staging_dir = self
            .ssd_staging_dir
            .take()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        self.max_download_speed_kib_s =
            normalize_max_download_speed_kib_s(self.max_download_speed_kib_s);
        self
    }

    /// Global speed limit converted to bytes/sec (`0` = unlimited).
    pub fn max_download_speed_bps(&self) -> u64 {
        normalize_max_download_speed_kib_s(self.max_download_speed_kib_s).saturating_mul(1024)
    }

    /// Maps `per_file_segments` to the planner's `SegmentLevel`, applying safety throttles for rate-limited servers.
    pub fn effective_segments_for_provider(&self, provider_id: &str) -> u8 {
        let base = normalize_per_file_segments(self.per_file_segments);
        let lower = provider_id.trim().to_ascii_lowercase();
        if lower.contains("bhoonidhi") || lower.contains("isro") {
            // ISRO Bhoonidhi APISIX enforces 2 requests/minute per IP on `/download`;
            // keep full-file downloads single-stream so segments do not burn the quota.
            base.min(1)
        } else {
            base
        }
    }

    /// Maps `per_file_segments` to the planner's `SegmentLevel`.
    pub fn effective_segment_level(&self) -> SegmentLevel {
        SegmentLevel::from_count(self.per_file_segments)
    }

    /// Returns a validated `PathBuf` when `ssd_staging_dir` is populated.
    pub fn staging_dir_path(&self) -> Option<PathBuf> {
        self.ssd_staging_dir
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    }
}

/// Normalizes global task concurrency onto `[1, 8]`.
pub fn normalize_max_concurrent_tasks(raw: u8) -> u8 {
    if raw == 0 {
        3
    } else {
        raw.clamp(1, 8)
    }
}

/// Normalizes XYZ satellite tile / GEE chunked raster concurrency onto `{8, 16, 32, 64}`.
pub fn normalize_tile_chunk_concurrency(raw: u8) -> u8 {
    match raw {
        0 => 16,
        1..=11 => 8,
        12..=23 => 16,
        24..=47 => 32,
        _ => 64,
    }
}

/// Normalizes arbitrary segment counts onto the supported `{1, 2, 4, 8}` ladder.
pub fn normalize_per_file_segments(raw: u8) -> u8 {
    match raw {
        0 | 1 => 1,
        2 | 3 => 2,
        4..=7 => 4,
        _ => 8,
    }
}

/// Normalizes global download speed ceiling in KiB/s (`0` = unlimited, capped at `MAX_DOWNLOAD_SPEED_KIB_S_LIMIT`).
pub fn normalize_max_download_speed_kib_s(raw: u64) -> u64 {
    raw.min(MAX_DOWNLOAD_SPEED_KIB_S_LIMIT)
}

static ACTIVE_PREFS: OnceLock<RwLock<TransferEnginePrefs>> = OnceLock::new();
static ACTIVE_PREFS_EXPLICITLY_SET: AtomicBool = AtomicBool::new(false);

fn active_prefs_lock() -> &'static RwLock<TransferEnginePrefs> {
    ACTIVE_PREFS.get_or_init(|| RwLock::new(TransferEnginePrefs::default()))
}

/// Returns the active process-wide transfer engine preferences snapshot.
pub fn active_transfer_engine_prefs() -> TransferEnginePrefs {
    match active_prefs_lock().read() {
        Ok(guard) => guard.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

/// Returns the active process-wide preferences snapshot only when explicitly activated by host
/// startup/configuration or tests; uninitialized standalone unit tests receive `None`.
pub fn active_transfer_engine_prefs_override() -> Option<TransferEnginePrefs> {
    ACTIVE_PREFS_EXPLICITLY_SET
        .load(Ordering::Acquire)
        .then(active_transfer_engine_prefs)
}

/// Clears any explicitly activated preferences override and restores defaults (for test teardown).
pub fn clear_active_transfer_engine_prefs_override() {
    ACTIVE_PREFS_EXPLICITLY_SET.store(false, Ordering::Release);
    let defaults = TransferEnginePrefs::default();
    match active_prefs_lock().write() {
        Ok(mut guard) => {
            *guard = defaults;
        }
        Err(poisoned) => {
            *poisoned.into_inner() = defaults;
        }
    }
    crate::transfer_tuning::global_bandwidth_limiter().set_limit_bps(0);
}

/// Updates the process-wide transfer engine preferences snapshot and returns the normalized value.
pub fn set_active_transfer_engine_prefs(prefs: TransferEnginePrefs) -> TransferEnginePrefs {
    let normalized = prefs.normalized();
    match active_prefs_lock().write() {
        Ok(mut guard) => {
            *guard = normalized.clone();
        }
        Err(poisoned) => {
            *poisoned.into_inner() = normalized.clone();
        }
    }
    ACTIVE_PREFS_EXPLICITLY_SET.store(true, Ordering::Release);
    crate::transfer_tuning::global_bandwidth_limiter()
        .set_limit_bps(normalized.max_download_speed_bps());
    normalized
}

/// Resolves the JSON file path used to persist transfer engine preferences inside `app_data_dir`.
pub fn transfer_engine_prefs_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(TRANSFER_ENGINE_PREFS_FILENAME)
}

/// Loads transfer engine preferences from `app_data_dir`, returning defaults if not yet saved.
pub fn load_transfer_engine_prefs_in_dir(
    app_data_dir: &Path,
) -> Result<TransferEnginePrefs, String> {
    let path = transfer_engine_prefs_path(app_data_dir);
    if !path.exists() {
        return Ok(TransferEnginePrefs::default());
    }
    let raw = fs::read_to_string(&path)
        .map_err(|err| format!("failed to read transfer engine preferences: {err}"))?;
    let parsed: TransferEnginePrefs = serde_json::from_str(&raw)
        .map_err(|err| format!("failed to parse transfer engine preferences: {err}"))?;
    Ok(parsed.normalized())
}

/// Atomically persists normalized transfer engine preferences inside `app_data_dir`.
pub fn save_transfer_engine_prefs_in_dir(
    app_data_dir: &Path,
    prefs: TransferEnginePrefs,
) -> Result<TransferEnginePrefs, String> {
    fs::create_dir_all(app_data_dir)
        .map_err(|err| format!("failed to create app data directory: {err}"))?;
    let normalized = prefs.normalized();
    let path = transfer_engine_prefs_path(app_data_dir);
    let temp_path = app_data_dir.join(format!("{TRANSFER_ENGINE_PREFS_FILENAME}.tmp"));
    let payload = serde_json::to_string_pretty(&normalized)
        .map_err(|err| format!("failed to serialize transfer engine preferences: {err}"))?;
    let mut file = fs::File::create(&temp_path)
        .map_err(|err| format!("failed to open temp preferences file: {err}"))?;
    file.write_all(payload.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|err| format!("failed to flush temp preferences file: {err}"))?;
    drop(file);
    fs::rename(&temp_path, &path)
        .map_err(|err| format!("failed to commit transfer engine preferences: {err}"))?;
    Ok(normalized)
}

/// Loads preferences from `app_data_dir` and synchronizes the process-wide runtime snapshot.
pub fn load_and_activate_transfer_engine_prefs_in_dir(
    app_data_dir: &Path,
) -> Result<TransferEnginePrefs, String> {
    let prefs = load_transfer_engine_prefs_in_dir(app_data_dir)?;
    set_active_default_ssd_staging_dir(Some(default_ssd_staging_dir_for_app_data(app_data_dir)));
    Ok(set_active_transfer_engine_prefs(prefs))
}

/// Saves preferences to `app_data_dir` and updates the process-wide runtime snapshot.
pub fn save_and_activate_transfer_engine_prefs_in_dir(
    app_data_dir: &Path,
    prefs: TransferEnginePrefs,
) -> Result<TransferEnginePrefs, String> {
    let saved = save_transfer_engine_prefs_in_dir(app_data_dir, prefs)?;
    set_active_default_ssd_staging_dir(Some(default_ssd_staging_dir_for_app_data(app_data_dir)));
    Ok(set_active_transfer_engine_prefs(saved))
}

/// Inputs consumed by [`SmartConcurrencyTuner::recommend_concurrency`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SmartConcurrencyInputs {
    pub configured_concurrency: u32,
    pub smart_concurrency_enabled: bool,
    pub free_space: Option<VolumeSpace>,
    pub observed_throughput_bps: u64,
    pub recent_stall_count: u32,
}

/// Computes an adaptive task concurrency ceiling constrained to `[1, 8]` while respecting disk
/// headroom and recent stall pressure.
#[derive(Debug, Clone, Copy, Default)]
pub struct SmartConcurrencyTuner;

impl SmartConcurrencyTuner {
    pub fn recommend_concurrency(inputs: &SmartConcurrencyInputs) -> u32 {
        let base = inputs.configured_concurrency.clamp(1, 8);
        if !inputs.smart_concurrency_enabled {
            return base;
        }

        let mut target = base;

        if let Some(space) = inputs.free_space {
            if space.free_bytes < LOW_DISK_THROTTLE_ONE_BYTES {
                return 1;
            }
            if space.free_bytes < MODERATE_DISK_CAP_TWO_BYTES {
                target = target.min(2);
            }
        }

        // Healthy disk with zero recent stalls preserves the user's configured
        // Tier-1 ceiling (e.g., setting 8 can reach 8 rather than being capped to 4..6).
        let _ = inputs.observed_throughput_bps;

        if inputs.recent_stall_count >= 2 {
            target = (target / 2).max(1);
        } else if inputs.recent_stall_count == 1 && target > 2 {
            target -= 1;
        }

        target.clamp(1, base)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_legacy_json_without_tier1_or_tier3_fields() {
        let legacy = r#"{"smart_concurrency":false,"per_file_segments":8}"#;
        let parsed: TransferEnginePrefs = serde_json::from_str(legacy).unwrap();
        let norm = parsed.normalized();
        assert_eq!(norm.max_concurrent_tasks, 2);
        assert_eq!(norm.per_file_segments, 8);
        assert_eq!(norm.tile_chunk_concurrency, 16);
        assert_eq!(norm.max_download_speed_kib_s, 0);
    }

    #[test]
    fn normalizes_3tier_bounds_and_bhoonidhi_safety_throttle() {
        let prefs = TransferEnginePrefs {
            max_concurrent_tasks: 20,
            smart_concurrency: true,
            per_file_segments: 8,
            tile_chunk_concurrency: 50,
            max_download_speed_kib_s: 9_999_999,
            ..TransferEnginePrefs::default()
        }
        .normalized();
        assert_eq!(prefs.max_concurrent_tasks, 8);
        assert_eq!(prefs.tile_chunk_concurrency, 64);
        assert_eq!(
            prefs.max_download_speed_kib_s,
            MAX_DOWNLOAD_SPEED_KIB_S_LIMIT
        );
        assert_eq!(prefs.effective_segments_for_provider("asf"), 8);
        assert_eq!(prefs.effective_segments_for_provider("isro_bhoonidhi"), 1);
    }
}
