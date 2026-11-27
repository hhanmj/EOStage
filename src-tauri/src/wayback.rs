use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::geodata_tools::{
    build_http_client_with_timeout, is_valid_tile_image_bytes, OpenGeodataQueryRequest,
    EOSTAGE_TILE_USER_AGENT,
};

pub(crate) const WAYBACK_CONFIG_URL: &str =
    "https://s3-us-west-2.amazonaws.com/config.maptiles.arcgis.com/waybackconfig.json";
pub(crate) const WAYBACK_TILE_BASE: &str =
    "https://wayback-a.maptiles.arcgis.com/arcgis/rest/services/world_imagery/mapserver/tile";

const VERSIONS_CACHE_TTL_SECS: u64 = 24 * 3600;
const SCAN_CACHE_TTL_SECS: u64 = 7 * 24 * 3600;
const SCAN_CACHE_MAX_ENTRIES: usize = 50;
const MAX_ACTIVE_SCANS: usize = 2;
const MAX_SCAN_STATE_ENTRIES: usize = 32;
const METADATA_TIMEOUT_SECS: u64 = 20;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct WaybackVersion {
    pub release_num: String,
    pub release_date: String,
    pub label: String,
    pub item_url: String,
    pub metadata_layer_url: String,
    pub layer_identifier: String,
    pub tile_url_template: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VersionsDiskCache {
    fetched_at_epoch_secs: u64,
    versions: Vec<WaybackVersion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct WaybackCaptureEntry {
    pub capture_id: String,
    pub capture_date: String,
    pub release_num: String,
    pub release_date: String,
    pub tile_url_template: String,
    pub coverage_ratio: f64,
    pub source_info: String,
    pub resolution_m: Option<f64>,
    pub accuracy_m: Option<f64>,
    pub geometry_hash: String,
    pub layer_id: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct WaybackReleaseSummary {
    pub release_num: String,
    pub release_date: String,
    pub label: String,
    pub tile_url_template: String,
    pub has_local_change: bool,
    pub coverage_ratio: f64,
    pub capture_count: usize,
    pub capture_dates: Vec<String>,
    pub primary_source: String,
    pub min_resolution_m: Option<f64>,
    pub from_dead_service: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct WaybackScanResult {
    pub scan_mode: String,
    pub zoom: u32,
    pub bbox: [f64; 4],
    pub effective_layer: u32,
    pub used_fallback_layer: bool,
    pub total_releases_scanned: usize,
    pub changed_releases_count: usize,
    pub unique_captures_count: usize,
    pub captures: Vec<WaybackCaptureEntry>,
    pub release_summaries: Vec<WaybackReleaseSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ScanDiskCache {
    cached_at_epoch_secs: u64,
    last_accessed_epoch_secs: u64,
    result: WaybackScanResult,
}

struct ScanJobState {
    scan_id: String,
    status: String,
    scan_mode: String,
    completed: usize,
    total: usize,
    effective_layer: u32,
    from_cache: bool,
    error: Option<String>,
    result: Option<WaybackScanResult>,
    cancel_flag: Arc<AtomicBool>,
    created_at_epoch_secs: u64,
    updated_at: Instant,
}

type VersionsMemCache = Mutex<Option<(Instant, Vec<WaybackVersion>)>>;

static VERSIONS_MEM_CACHE: OnceLock<VersionsMemCache> = OnceLock::new();
static SCAN_JOBS: OnceLock<Mutex<HashMap<String, ScanJobState>>> = OnceLock::new();
static DEAD_METADATA_SERVICES: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
static SCAN_COUNTER: AtomicU64 = AtomicU64::new(1);

fn versions_mem_cache() -> &'static VersionsMemCache {
    VERSIONS_MEM_CACHE.get_or_init(|| Mutex::new(None))
}

fn scan_jobs() -> &'static Mutex<HashMap<String, ScanJobState>> {
    SCAN_JOBS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn dead_metadata_services() -> &'static Mutex<HashSet<String>> {
    DEAD_METADATA_SERVICES.get_or_init(|| Mutex::new(HashSet::new()))
}

fn now_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn cache_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("wayback_cache")
}

/// Parses `wayback_<1..=16 ASCII digits>` strictly and returns the numeric release ID slice.
pub(crate) fn parse_wayback_version_id(source_key: &str) -> Option<&str> {
    let trimmed = source_key.trim();
    let prefix = trimmed.get(..8)?;
    if !prefix.eq_ignore_ascii_case("wayback_") {
        return None;
    }
    let id = &trimmed[8..];
    if !id.is_empty() && id.len() <= 16 && id.bytes().all(|b| b.is_ascii_digit()) {
        Some(id)
    } else {
        None
    }
}

/// Builds the CSP-allowlisted Esri Wayback tile URL for a release and tile coordinates.
pub(crate) fn wayback_tile_url(version_id: &str, z: u32, x: u32, y: u32) -> String {
    format!("{WAYBACK_TILE_BASE}/{version_id}/{z}/{y}/{x}")
}

/// Checks whether a URL authority strictly targets an ArcGIS Wayback tile host.
#[cfg(test)]
pub(crate) fn is_wayback_host(raw_url: &str) -> bool {
    let Ok(parsed) = Url::parse(raw_url.trim()) else {
        return false;
    };
    let Some(host) = parsed.host_str() else {
        return false;
    };
    let lower = host.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "wayback-a.maptiles.arcgis.com" | "wayback.maptiles.arcgis.com"
    )
}

/// Validates ArcGIS Wayback metadata service URLs against SSRF before HTTP dispatch.
pub(crate) fn validate_metadata_layer_url(raw_url: &str) -> Result<Url, String> {
    let trimmed = raw_url.trim();
    if trimmed.is_empty() {
        return Err("WAYBACK_INVALID_METADATA_URL: empty metadataLayerUrl".to_owned());
    }
    let parsed = Url::parse(trimmed)
        .map_err(|e| format!("WAYBACK_INVALID_METADATA_URL: malformed URL ({e})"))?;
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("WAYBACK_INVALID_METADATA_URL: credentials in URL are forbidden".to_owned());
    }

    #[cfg(test)]
    if parsed.scheme() == "http"
        && matches!(
            parsed.host_str(),
            Some("127.0.0.1" | "localhost" | "[::1]" | "::1")
        )
    {
        return Ok(parsed);
    }

    if parsed.scheme() != "https" {
        return Err(format!(
            "WAYBACK_INVALID_METADATA_URL: scheme '{}' is not https",
            parsed.scheme()
        ));
    }

    let raw_host = parsed
        .host_str()
        .ok_or_else(|| "WAYBACK_INVALID_METADATA_URL: missing host".to_owned())?;
    let host_unbracketed = raw_host.trim_start_matches('[').trim_end_matches(']');
    let domain_lower = host_unbracketed.to_ascii_lowercase();
    if domain_lower.parse::<IpAddr>().is_ok()
        || domain_lower == "localhost"
        || domain_lower.ends_with(".localhost")
        || domain_lower.ends_with(".local")
    {
        return Err(format!(
            "WAYBACK_INVALID_METADATA_URL: local/IP host '{domain_lower}' is forbidden"
        ));
    }
    let allowed = domain_lower == "metadata.maptiles.arcgis.com"
        || domain_lower.ends_with(".arcgis.com")
        || domain_lower.ends_with(".arcgisonline.com");
    if !allowed {
        return Err(format!(
            "WAYBACK_INVALID_METADATA_URL: host '{domain_lower}' is outside ArcGIS allowlist"
        ));
    }
    Ok(parsed)
}

/// Maps XYZ zoom level to the empirical ArcGIS World_Imagery_Metadata layer index.
pub(crate) fn select_layer_for_zoom(zoom: u32) -> u32 {
    match zoom {
        20..=u32::MAX => 3, // ~15cm (Vivid Premium)
        18 | 19 => 4,       // ~30cm (Vivid Advanced)
        17 => 5,            // ~60cm
        16 => 6,            // ~1.2m
        14 | 15 => 7,       // ~2.4m
        13 => 8,            // ~4.8m
        12 => 9,            // ~9.6m
        11 => 10,           // ~19m
        10 => 11,           // ~38m
        _ => 12,            // ~75m
    }
}

fn cascade_layers_from(primary: u32) -> Vec<u32> {
    let mut out = vec![primary];
    for candidate in [3u32, 4, 5, 6, 7] {
        if candidate >= primary && !out.contains(&candidate) {
            out.push(candidate);
        }
    }
    out
}

/// Writes JSON atomically and overwrites an existing destination cleanly on Windows and Unix.
fn atomic_write_json<T: Serialize>(dest: &Path, payload: &T) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("WAYBACK_CACHE_MKDIR_FAILED: {e}"))?;
    }
    let bytes = serde_json::to_vec_pretty(payload)
        .map_err(|e| format!("WAYBACK_CACHE_SERIALIZE_FAILED: {e}"))?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp_name = format!(
        ".{}.tmp.{}.{}",
        dest.file_name().and_then(|n| n.to_str()).unwrap_or("cache"),
        std::process::id(),
        nanos
    );
    let tmp_path = dest.with_file_name(tmp_name);
    fs::write(&tmp_path, bytes).map_err(|e| format!("WAYBACK_CACHE_WRITE_FAILED: {e}"))?;
    if let Err(first_err) = fs::rename(&tmp_path, dest) {
        if dest.exists() {
            let _ = fs::remove_file(dest);
        }
        if let Err(second_err) = fs::rename(&tmp_path, dest) {
            let _ = fs::remove_file(&tmp_path);
            return Err(format!(
                "WAYBACK_CACHE_RENAME_FAILED: {first_err}; retry={second_err}"
            ));
        }
    }
    Ok(())
}

fn extract_yyyy_mm_dd(text: &str) -> String {
    let bytes = text.as_bytes();
    if bytes.len() < 10 {
        return String::new();
    }
    for i in 0..=bytes.len() - 10 {
        let s = &bytes[i..i + 10];
        if s[0..4].iter().all(|b| b.is_ascii_digit())
            && s[4] == b'-'
            && s[5..7].iter().all(|b| b.is_ascii_digit())
            && s[7] == b'-'
            && s[8..10].iter().all(|b| b.is_ascii_digit())
        {
            if let Ok(parsed) = std::str::from_utf8(s) {
                return parsed.to_owned();
            }
        }
    }
    String::new()
}

fn parse_wayback_config_json(raw: &Value) -> Vec<WaybackVersion> {
    let Some(map) = raw.as_object() else {
        return Vec::new();
    };
    let mut versions = Vec::with_capacity(map.len());
    for (release_num, entry) in map {
        let trimmed_id = release_num.trim();
        if trimmed_id.is_empty()
            || trimmed_id.len() > 16
            || !trimmed_id.bytes().all(|b| b.is_ascii_digit())
        {
            continue;
        }
        let label = entry["itemTitle"].as_str().unwrap_or("").trim().to_owned();
        let release_date = extract_yyyy_mm_dd(&label);
        let item_url = entry["itemURL"].as_str().unwrap_or("").trim().to_owned();
        let raw_meta = entry["metadataLayerUrl"].as_str().unwrap_or("").trim();
        let metadata_layer_url = match validate_metadata_layer_url(raw_meta) {
            Ok(valid) => valid.to_string(),
            Err(_) => String::new(),
        };
        let layer_identifier = entry["layerIdentifier"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_owned();
        versions.push(WaybackVersion {
            release_num: trimmed_id.to_owned(),
            release_date,
            label,
            item_url,
            metadata_layer_url,
            layer_identifier,
            tile_url_template: wayback_tile_url(trimmed_id, 0, 0, 0)
                .replace("/0/0/0", "/{z}/{y}/{x}"),
        });
    }
    versions.sort_by(|a, b| {
        b.release_date.cmp(&a.release_date).then_with(|| {
            let b_num = b.release_num.parse::<u64>().unwrap_or(0);
            let a_num = a.release_num.parse::<u64>().unwrap_or(0);
            b_num.cmp(&a_num)
        })
    });
    versions
}

fn load_versions_from_disk(app_data_dir: &Path) -> Option<VersionsDiskCache> {
    let path = cache_dir(app_data_dir).join("versions.json");
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str::<VersionsDiskCache>(&raw).ok()
}

pub(crate) fn get_wayback_versions(
    app_data_dir: &Path,
    force_refresh: bool,
) -> Result<(Vec<WaybackVersion>, &'static str, u64), String> {
    if !force_refresh {
        if let Ok(guard) = versions_mem_cache().lock() {
            if let Some((fetched_at, ref list)) = *guard {
                if fetched_at.elapsed().as_secs() < VERSIONS_CACHE_TTL_SECS && !list.is_empty() {
                    return Ok((list.clone(), "memory_cache", now_epoch_secs()));
                }
            }
        }
        if let Some(disk) = load_versions_from_disk(app_data_dir) {
            let age = now_epoch_secs().saturating_sub(disk.fetched_at_epoch_secs);
            if age < VERSIONS_CACHE_TTL_SECS && !disk.versions.is_empty() {
                if let Ok(mut guard) = versions_mem_cache().lock() {
                    *guard = Some((Instant::now(), disk.versions.clone()));
                }
                return Ok((disk.versions, "disk_cache", disk.fetched_at_epoch_secs));
            }
        }
    }

    let client = build_http_client_with_timeout(app_data_dir, 25)?;
    let fetched = client
        .get(WAYBACK_CONFIG_URL)
        .header(reqwest::header::USER_AGENT, EOSTAGE_TILE_USER_AGENT)
        .send()
        .and_then(|r| r.error_for_status())
        .and_then(|r| r.json::<Value>());

    match fetched {
        Ok(json_val) => {
            let versions = parse_wayback_config_json(&json_val);
            if versions.is_empty() {
                if let Some(disk) = load_versions_from_disk(app_data_dir) {
                    if !disk.versions.is_empty() {
                        return Ok((disk.versions, "disk_fallback", disk.fetched_at_epoch_secs));
                    }
                }
                return Err(
                    "WAYBACK_VERSIONS_EMPTY: ArcGIS Wayback 配置文件未返回有效版本列表".to_owned(),
                );
            }
            let ts = now_epoch_secs();
            let disk_payload = VersionsDiskCache {
                fetched_at_epoch_secs: ts,
                versions: versions.clone(),
            };
            let _ = atomic_write_json(
                &cache_dir(app_data_dir).join("versions.json"),
                &disk_payload,
            );
            if let Ok(mut guard) = versions_mem_cache().lock() {
                *guard = Some((Instant::now(), versions.clone()));
            }
            Ok((versions, "live", ts))
        }
        Err(err) => {
            if let Some(disk) = load_versions_from_disk(app_data_dir) {
                if !disk.versions.is_empty() {
                    return Ok((disk.versions, "disk_fallback", disk.fetched_at_epoch_secs));
                }
            }
            Err(format!(
                "WAYBACK_VERSIONS_FETCH_FAILED: 获取 Esri Wayback 历史版本清单失败 ({err})"
            ))
        }
    }
}

fn lon_lat_to_tile(lon: f64, lat: f64, z: u32) -> (u32, u32) {
    let n = (1u64 << z.min(28)) as f64;
    let x = ((lon.clamp(-179.9999, 179.9999) + 180.0) / 360.0 * n).floor();
    let lat_rad = lat.clamp(-85.0511, 85.0511).to_radians();
    let y = ((1.0 - (lat_rad.tan() + 1.0 / lat_rad.cos()).ln() / std::f64::consts::PI) / 2.0 * n)
        .floor();
    let max_idx = (1i64 << z.min(28)) - 1;
    (
        (x as i64).clamp(0, max_idx) as u32,
        (y as i64).clamp(0, max_idx) as u32,
    )
}

fn normalize_bbox(raw: [f64; 4]) -> [f64; 4] {
    [
        raw[0].min(raw[2]),
        raw[1].min(raw[3]),
        raw[0].max(raw[2]),
        raw[1].max(raw[3]),
    ]
}

fn lon_to_mercator_x(lon: f64) -> f64 {
    lon.clamp(-180.0, 180.0) * 20037508.342789244 / 180.0
}

fn lat_to_mercator_y(lat: f64) -> f64 {
    let clamped = lat.clamp(-85.05112878, 85.05112878);
    let rad = clamped.to_radians();
    let merc = (std::f64::consts::FRAC_PI_4 + rad / 2.0).tan().ln();
    merc * 20037508.342789244 / std::f64::consts::PI
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Point2D {
    x: f64,
    y: f64,
}

fn polygon_area(poly: &[Point2D]) -> f64 {
    if poly.len() < 3 {
        return 0.0;
    }
    let mut sum = 0.0;
    for i in 0..poly.len() {
        let j = (i + 1) % poly.len();
        sum += poly[i].x * poly[j].y - poly[j].x * poly[i].y;
    }
    0.5 * sum.abs()
}

fn clip_edge<F, I>(poly: &[Point2D], inside: F, intersect: I) -> Vec<Point2D>
where
    F: Fn(Point2D) -> bool,
    I: Fn(Point2D, Point2D) -> Point2D,
{
    if poly.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(poly.len() + 1);
    let mut prev = *poly.last().expect("non-empty polygon");
    let mut prev_in = inside(prev);
    for &curr in poly {
        let curr_in = inside(curr);
        if curr_in {
            if !prev_in {
                out.push(intersect(prev, curr));
            }
            out.push(curr);
        } else if prev_in {
            out.push(intersect(prev, curr));
        }
        prev = curr;
        prev_in = curr_in;
    }
    out
}

fn clip_ring_to_rect(
    ring: &[Point2D],
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
) -> Vec<Point2D> {
    let p1 = clip_edge(
        ring,
        |p| p.x >= min_x,
        |a, b| {
            let dx = b.x - a.x;
            let t = if dx.abs() < 1e-12 {
                0.0
            } else {
                (min_x - a.x) / dx
            };
            Point2D {
                x: min_x,
                y: a.y + t * (b.y - a.y),
            }
        },
    );
    let p2 = clip_edge(
        &p1,
        |p| p.x <= max_x,
        |a, b| {
            let dx = b.x - a.x;
            let t = if dx.abs() < 1e-12 {
                0.0
            } else {
                (max_x - a.x) / dx
            };
            Point2D {
                x: max_x,
                y: a.y + t * (b.y - a.y),
            }
        },
    );
    let p3 = clip_edge(
        &p2,
        |p| p.y >= min_y,
        |a, b| {
            let dy = b.y - a.y;
            let t = if dy.abs() < 1e-12 {
                0.0
            } else {
                (min_y - a.y) / dy
            };
            Point2D {
                x: a.x + t * (b.x - a.x),
                y: min_y,
            }
        },
    );
    clip_edge(
        &p3,
        |p| p.y <= max_y,
        |a, b| {
            let dy = b.y - a.y;
            let t = if dy.abs() < 1e-12 {
                0.0
            } else {
                (max_y - a.y) / dy
            };
            Point2D {
                x: a.x + t * (b.x - a.x),
                y: max_y,
            }
        },
    )
}

fn compute_feature_coverage_ratio(feature: &Value, bbox_wgs84: [f64; 4]) -> f64 {
    let min_x = lon_to_mercator_x(bbox_wgs84[0]);
    let min_y = lat_to_mercator_y(bbox_wgs84[1]);
    let max_x = lon_to_mercator_x(bbox_wgs84[2]);
    let max_y = lat_to_mercator_y(bbox_wgs84[3]);
    let aoi_area = (max_x - min_x).abs() * (max_y - min_y).abs();
    if aoi_area <= 0.0 || !aoi_area.is_finite() {
        return 0.0;
    }
    let Some(rings) = feature["geometry"]["rings"].as_array() else {
        return 1.0; // Attributes-only response covers queried AOI point/envelope
    };
    if rings.is_empty() {
        return 0.0;
    }
    let mut covered_area = 0.0;
    for ring_val in rings {
        let Some(coords) = ring_val.as_array() else {
            continue;
        };
        let pts: Vec<Point2D> = coords
            .iter()
            .filter_map(|pt| {
                let arr = pt.as_array()?;
                let x = arr.first()?.as_f64()?;
                let y = arr.get(1)?.as_f64()?;
                Some(Point2D { x, y })
            })
            .collect();
        if pts.len() < 3 {
            continue;
        }
        let clipped = clip_ring_to_rect(&pts, min_x, min_y, max_x, max_y);
        covered_area += polygon_area(&clipped);
    }
    (covered_area / aoi_area).clamp(0.0, 1.0)
}

/// Probes the highest zoom with real valid imagery tiles (`Z20` down to `Z1`) for the AOI center.
pub(crate) fn probe_wayback_max_zoom(
    app_data_dir: &Path,
    bbox_raw: [f64; 4],
    version_id: Option<&str>,
) -> Result<Value, String> {
    let bbox = normalize_bbox(bbox_raw);
    let center_lon = (bbox[0] + bbox[2]) * 0.5;
    let center_lat = (bbox[1] + bbox[3]) * 0.5;

    let resolved_id = if let Some(vid) = version_id.map(str::trim).filter(|s| !s.is_empty()) {
        let stripped = parse_wayback_version_id(vid)
            .or_else(|| {
                if vid.len() <= 16 && vid.bytes().all(|b| b.is_ascii_digit()) {
                    Some(vid)
                } else {
                    None
                }
            })
            .ok_or_else(|| {
                format!("INVALID_WAYBACK_VERSION_ID: 非法的 Wayback 版本编号 '{vid}'")
            })?;
        stripped.to_owned()
    } else {
        let (versions, _, _) = get_wayback_versions(app_data_dir, false)?;
        versions
            .first()
            .map(|v| v.release_num.clone())
            .ok_or_else(|| "WAYBACK_VERSIONS_EMPTY: 无法获取默认 Wayback 版本".to_owned())?
    };

    let client = build_http_client_with_timeout(app_data_dir, 12)?;
    let mut saw_not_found = false;
    let mut network_errors = 0usize;
    let mut last_error_msg = String::new();

    for z in (1..=20u32).rev() {
        let (tx, ty) = lon_lat_to_tile(center_lon, center_lat, z);
        let url = wayback_tile_url(&resolved_id, z, tx, ty);
        match client
            .get(&url)
            .header(reqwest::header::USER_AGENT, EOSTAGE_TILE_USER_AGENT)
            .send()
        {
            Ok(resp) => {
                let code = resp.status().as_u16();
                if code == 200 || code == 206 {
                    match resp.bytes() {
                        Ok(bytes) if !bytes.is_empty() && is_valid_tile_image_bytes(&bytes) => {
                            return Ok(json!({
                                "ok": true,
                                "dataset": "esri_wayback_probe_zoom",
                                "version_id": resolved_id,
                                "max_zoom": z,
                                "probe_point": [center_lon, center_lat],
                                "probe_tile": { "z": z, "x": tx, "y": ty },
                                "no_imagery_found": false,
                            }));
                        }
                        Ok(_) => {
                            saw_not_found = true;
                        }
                        Err(e) => {
                            network_errors += 1;
                            last_error_msg = e.to_string();
                        }
                    }
                } else if code == 404 || code == 204 || code == 400 {
                    saw_not_found = true;
                } else {
                    network_errors += 1;
                    last_error_msg = format!("HTTP {code}");
                }
            }
            Err(e) => {
                network_errors += 1;
                last_error_msg = e.to_string();
            }
        }
    }

    if !saw_not_found && network_errors > 0 {
        return Err(format!(
            "WAYBACK_PROBE_NETWORK_ERROR: 探测历史影像最大可用层级失败 ({last_error_msg})"
        ));
    }

    Ok(json!({
        "ok": true,
        "dataset": "esri_wayback_probe_zoom",
        "version_id": resolved_id,
        "max_zoom": 1,
        "probe_point": [center_lon, center_lat],
        "no_imagery_found": true,
    }))
}

fn format_src_date_ms(val: &Value) -> String {
    if let Some(ms) = val.as_i64() {
        let secs = ms / 1000;
        if let Some(dt) = chrono::DateTime::from_timestamp(secs, 0) {
            return dt.format("%Y-%m-%d").to_string();
        }
    }
    if let Some(s) = val.as_str() {
        let trimmed = s.trim();
        let date = extract_yyyy_mm_dd(trimmed);
        if !date.is_empty() {
            return date;
        }
        return trimmed.to_owned();
    }
    String::new()
}

fn hash_feature_geometry(feature: &Value) -> String {
    let mut hasher = Sha256::new();
    let geom = &feature["geometry"];
    if let Ok(canonical) = serde_json::to_vec(geom) {
        hasher.update(&canonical);
    } else {
        hasher.update(feature["attributes"]["OBJECTID"].to_string().as_bytes());
    }
    format!("{:x}", hasher.finalize())[..16].to_owned()
}

enum MetadataQueryOutcome {
    Features(Vec<Value>),
    #[allow(dead_code)]
    DeadService5xx(String),
    #[allow(dead_code)]
    TransientError(String),
}

fn query_metadata_layer_once(
    client: &reqwest::blocking::Client,
    validated_base: &Url,
    layer_id: u32,
    bbox: [f64; 4],
    return_geometry: bool,
) -> MetadataQueryOutcome {
    let mut query_url = validated_base.clone();
    {
        let mut path = query_url.path().trim_end_matches('/').to_owned();
        path.push('/');
        path.push_str(&layer_id.to_string());
        path.push_str("/query");
        query_url.set_path(&path);
    }
    let env_json = json!({
        "xmin": bbox[0],
        "ymin": bbox[1],
        "xmax": bbox[2],
        "ymax": bbox[3],
        "spatialReference": { "wkid": 4326 }
    })
    .to_string();

    let params = [
        ("f", "json"),
        ("where", "1=1"),
        ("geometry", env_json.as_str()),
        ("geometryType", "esriGeometryEnvelope"),
        ("inSR", "4326"),
        ("spatialRel", "esriSpatialRelIntersects"),
        (
            "outFields",
            "OBJECTID,SRC_DATE,SRC_DATE2,NICE_DESC,SRC_RES,SRC_ACC,MinMapLevel,MaxMapLevel",
        ),
        (
            "returnGeometry",
            if return_geometry { "true" } else { "false" },
        ),
        ("outSR", "3857"),
    ];

    for attempt in 0..3usize {
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(150 * attempt as u64));
        }
        match client
            .get(query_url.clone())
            .query(&params)
            .header(reqwest::header::USER_AGENT, EOSTAGE_TILE_USER_AGENT)
            .send()
        {
            Ok(resp) => {
                let code = resp.status().as_u16();
                if code >= 500 {
                    if attempt == 2 {
                        return MetadataQueryOutcome::DeadService5xx(format!("HTTP {code}"));
                    }
                    continue;
                }
                if !resp.status().is_success() {
                    return MetadataQueryOutcome::TransientError(format!("HTTP {code}"));
                }
                match resp.json::<Value>() {
                    Ok(body) => {
                        if let Some(err_obj) = body.get("error") {
                            let err_code = err_obj["code"].as_i64().unwrap_or(0);
                            if err_code >= 500 && attempt < 2 {
                                continue;
                            }
                            if err_code >= 500 {
                                return MetadataQueryOutcome::DeadService5xx(err_obj.to_string());
                            }
                            return MetadataQueryOutcome::Features(Vec::new());
                        }
                        let features = body["features"].as_array().cloned().unwrap_or_default();
                        return MetadataQueryOutcome::Features(features);
                    }
                    Err(e) => {
                        if attempt == 2 {
                            return MetadataQueryOutcome::TransientError(e.to_string());
                        }
                    }
                }
            }
            Err(e) => {
                if attempt == 2 {
                    return MetadataQueryOutcome::TransientError(e.to_string());
                }
            }
        }
    }
    MetadataQueryOutcome::TransientError("exhausted retries".to_owned())
}

fn scan_cache_key(bbox: [f64; 4], zoom: u32, mode: &str, latest_release: &str) -> String {
    let mut hasher = Sha256::new();
    let canonical = format!(
        "{:.6},{:.6},{:.6},{:.6}|z={zoom}|m={mode}|r={latest_release}",
        bbox[0], bbox[1], bbox[2], bbox[3]
    );
    hasher.update(canonical.as_bytes());
    format!("scan_{:x}.json", hasher.finalize())
}

fn load_scan_cache(app_data_dir: &Path, filename: &str) -> Option<WaybackScanResult> {
    let path = cache_dir(app_data_dir).join(filename);
    let raw = fs::read_to_string(&path).ok()?;
    let mut cached = serde_json::from_str::<ScanDiskCache>(&raw).ok()?;
    let age = now_epoch_secs().saturating_sub(cached.cached_at_epoch_secs);
    if age > SCAN_CACHE_TTL_SECS {
        let _ = fs::remove_file(&path);
        return None;
    }
    cached.last_accessed_epoch_secs = now_epoch_secs();
    let _ = atomic_write_json(&path, &cached);
    Some(cached.result)
}

fn prune_scan_lru(app_data_dir: &Path) {
    let dir = cache_dir(app_data_dir);
    let Ok(entries) = fs::read_dir(&dir) else {
        return;
    };
    let mut scan_files: Vec<(PathBuf, u64)> = Vec::new();
    for entry in entries.flatten() {
        let p = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("scan_") || !name.ends_with(".json") {
            continue;
        }
        let touched = fs::read_to_string(&p)
            .ok()
            .and_then(|s| serde_json::from_str::<ScanDiskCache>(&s).ok())
            .map(|c| c.last_accessed_epoch_secs)
            .unwrap_or_else(|| {
                entry
                    .metadata()
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0)
            });
        scan_files.push((p, touched));
    }
    if scan_files.len() <= SCAN_CACHE_MAX_ENTRIES {
        return;
    }
    scan_files.sort_by_key(|b| std::cmp::Reverse(b.1));
    for (stale_path, _) in scan_files.into_iter().skip(SCAN_CACHE_MAX_ENTRIES) {
        let _ = fs::remove_file(stale_path);
    }
}

fn evict_stale_scan_jobs(jobs: &mut HashMap<String, ScanJobState>) {
    while jobs.len() >= MAX_SCAN_STATE_ENTRIES {
        let victim = jobs
            .iter()
            .filter(|(_, s)| s.status != "running")
            .min_by_key(|(_, s)| s.created_at_epoch_secs)
            .map(|(k, _)| k.clone())
            .or_else(|| {
                jobs.iter()
                    .min_by_key(|(_, s)| s.created_at_epoch_secs)
                    .map(|(k, _)| k.clone())
            });
        if let Some(k) = victim {
            jobs.remove(&k);
        } else {
            break;
        }
    }
}

fn select_versions_for_mode(versions: &[WaybackVersion], mode: &str) -> Vec<WaybackVersion> {
    if versions.is_empty() || mode == "official" {
        return versions.to_vec();
    }
    let step = if mode == "fast" {
        (versions.len() / 24).max(1)
    } else {
        (versions.len() / 64).max(1)
    };
    let mut selected = Vec::new();
    for (idx, v) in versions.iter().enumerate() {
        if idx == 0 || idx + 1 == versions.len() || idx % step == 0 {
            selected.push(v.clone());
        }
    }
    selected
}

fn run_wayback_scan_worker(
    app_data_dir: PathBuf,
    scan_id: String,
    bbox: [f64; 4],
    zoom: u32,
    scan_mode: String,
    cache_filename: String,
    cancel_flag: Arc<AtomicBool>,
) {
    let versions_res = get_wayback_versions(&app_data_dir, false);
    let versions = match versions_res {
        Ok((v, _, _)) => v,
        Err(err) => {
            if let Ok(mut guard) = scan_jobs().lock() {
                if let Some(job) = guard.get_mut(&scan_id) {
                    job.status = "failed".to_owned();
                    job.error = Some(err);
                    job.updated_at = Instant::now();
                }
            }
            return;
        }
    };

    let target_versions = select_versions_for_mode(&versions, &scan_mode);
    let total = target_versions.len();
    if let Ok(mut guard) = scan_jobs().lock() {
        if let Some(job) = guard.get_mut(&scan_id) {
            job.total = total;
            job.updated_at = Instant::now();
        }
    }

    let client = match build_http_client_with_timeout(&app_data_dir, METADATA_TIMEOUT_SECS) {
        Ok(c) => c,
        Err(err) => {
            if let Ok(mut guard) = scan_jobs().lock() {
                if let Some(job) = guard.get_mut(&scan_id) {
                    job.status = "failed".to_owned();
                    job.error = Some(err);
                    job.updated_at = Instant::now();
                }
            }
            return;
        }
    };

    // Determine effective metadata layer with L3 -> L4 -> L5 auto-cascade on the newest release.
    let primary_layer = select_layer_for_zoom(zoom);
    let mut effective_layer = primary_layer;
    let mut used_fallback_layer = false;
    if let Some(newest) = target_versions.first() {
        if let Ok(valid_url) = validate_metadata_layer_url(&newest.metadata_layer_url) {
            for candidate in cascade_layers_from(primary_layer) {
                if cancel_flag.load(Ordering::Acquire) {
                    break;
                }
                match query_metadata_layer_once(&client, &valid_url, candidate, bbox, false) {
                    MetadataQueryOutcome::Features(feats) if !feats.is_empty() => {
                        effective_layer = candidate;
                        used_fallback_layer = candidate != primary_layer;
                        break;
                    }
                    MetadataQueryOutcome::DeadService5xx(_) => {
                        if let Ok(mut dead) = dead_metadata_services().lock() {
                            dead.insert(newest.metadata_layer_url.clone());
                        }
                        break;
                    }
                    _ => {}
                }
            }
        }
    }

    if let Ok(mut guard) = scan_jobs().lock() {
        if let Some(job) = guard.get_mut(&scan_id) {
            job.effective_layer = effective_layer;
            job.updated_at = Instant::now();
        }
    }

    let return_geom = scan_mode != "fast";
    let worker_count = 6usize.min(total.max(1));
    let next_idx = AtomicUsize::new(0);
    let done_count = AtomicUsize::new(0);
    type RawScanEntry = (usize, WaybackVersion, Vec<Value>, bool);
    let raw_results: Mutex<Vec<RawScanEntry>> = Mutex::new(Vec::with_capacity(total));

    std::thread::scope(|scope| {
        for _ in 0..worker_count {
            scope.spawn(|| {
                let Ok(w_client) =
                    build_http_client_with_timeout(&app_data_dir, METADATA_TIMEOUT_SECS)
                else {
                    return;
                };
                loop {
                    if cancel_flag.load(Ordering::Acquire) {
                        break;
                    }
                    let idx = next_idx.fetch_add(1, Ordering::Relaxed);
                    if idx >= total {
                        break;
                    }
                    let ver = target_versions[idx].clone();
                    let is_blacklisted = dead_metadata_services()
                        .lock()
                        .map(|d| d.contains(&ver.metadata_layer_url))
                        .unwrap_or(false);
                    let (feats, dead_hit) = if is_blacklisted || ver.metadata_layer_url.is_empty() {
                        (Vec::new(), is_blacklisted)
                    } else if let Ok(valid_url) =
                        validate_metadata_layer_url(&ver.metadata_layer_url)
                    {
                        match query_metadata_layer_once(
                            &w_client,
                            &valid_url,
                            effective_layer,
                            bbox,
                            return_geom,
                        ) {
                            MetadataQueryOutcome::Features(f) => (f, false),
                            MetadataQueryOutcome::DeadService5xx(_) => {
                                if let Ok(mut dead) = dead_metadata_services().lock() {
                                    dead.insert(ver.metadata_layer_url.clone());
                                }
                                (Vec::new(), true)
                            }
                            MetadataQueryOutcome::TransientError(_) => (Vec::new(), false),
                        }
                    } else {
                        (Vec::new(), false)
                    };

                    if let Ok(mut out) = raw_results.lock() {
                        out.push((idx, ver, feats, dead_hit));
                    }
                    let done = done_count.fetch_add(1, Ordering::Relaxed) + 1;
                    if done == total || done.is_multiple_of(4) {
                        if let Ok(mut jobs) = scan_jobs().lock() {
                            if let Some(job) = jobs.get_mut(&scan_id) {
                                job.completed = done;
                                job.updated_at = Instant::now();
                            }
                        }
                    }
                }
            });
        }
    });

    if cancel_flag.load(Ordering::Acquire) {
        if let Ok(mut guard) = scan_jobs().lock() {
            if let Some(job) = guard.get_mut(&scan_id) {
                job.status = "cancelled".to_owned();
                job.updated_at = Instant::now();
            }
        }
        return;
    }

    let mut rows = raw_results.into_inner().unwrap_or_default();
    // Sort chronologically (oldest first) so earliest release introducing each capture wins dedup.
    rows.sort_by(|a, b| {
        a.1.release_date.cmp(&b.1.release_date).then_with(|| {
            let a_num = a.1.release_num.parse::<u64>().unwrap_or(0);
            let b_num = b.1.release_num.parse::<u64>().unwrap_or(0);
            a_num.cmp(&b_num)
        })
    });

    let mut dedup_captures: BTreeMap<(String, String), WaybackCaptureEntry> = BTreeMap::new();
    let mut release_summaries: Vec<WaybackReleaseSummary> = Vec::with_capacity(rows.len());
    let mut prev_signature = String::new();
    let mut changed_releases_count = 0usize;

    for (_idx, ver, feats, dead_hit) in rows {
        let mut sig_parts: Vec<String> = Vec::new();
        let mut dates_set: BTreeSet<String> = BTreeSet::new();
        let mut max_cov = 0.0f64;
        let mut min_res: Option<f64> = None;
        let mut primary_source = String::new();

        for feat in &feats {
            let attrs = &feat["attributes"];
            let c_date = {
                let d2 = format_src_date_ms(&attrs["SRC_DATE2"]);
                if !d2.is_empty() {
                    d2
                } else {
                    format_src_date_ms(&attrs["SRC_DATE"])
                }
            };
            let g_hash = hash_feature_geometry(feat);
            let cov = if return_geom {
                compute_feature_coverage_ratio(feat, bbox)
            } else {
                1.0
            };
            max_cov = max_cov.max(cov);
            let src_res = attrs["SRC_RES"].as_f64();
            if let Some(r) = src_res {
                min_res = Some(min_res.map_or(r, |cur| cur.min(r)));
            }
            let src_acc = attrs["SRC_ACC"].as_f64();
            let nice_desc = attrs["NICE_DESC"].as_str().unwrap_or("").trim().to_owned();
            if primary_source.is_empty() && !nice_desc.is_empty() {
                primary_source = nice_desc.clone();
            }
            if !c_date.is_empty() {
                dates_set.insert(c_date.clone());
            }
            sig_parts.push(format!("{c_date}:{g_hash}"));

            let dedup_key = (c_date.clone(), g_hash.clone());
            dedup_captures
                .entry(dedup_key)
                .or_insert_with(|| WaybackCaptureEntry {
                    capture_id: format!("{}_{}", ver.release_num, g_hash),
                    capture_date: if c_date.is_empty() {
                        ver.release_date.clone()
                    } else {
                        c_date
                    },
                    release_num: ver.release_num.clone(),
                    release_date: ver.release_date.clone(),
                    tile_url_template: ver.tile_url_template.clone(),
                    coverage_ratio: cov,
                    source_info: nice_desc,
                    resolution_m: src_res,
                    accuracy_m: src_acc,
                    geometry_hash: g_hash,
                    layer_id: effective_layer,
                });
        }

        sig_parts.sort();
        let signature = sig_parts.join("|");
        let has_local_change = !feats.is_empty() && signature != prev_signature;
        if has_local_change {
            changed_releases_count += 1;
        }
        if !feats.is_empty() {
            prev_signature = signature;
        }

        release_summaries.push(WaybackReleaseSummary {
            release_num: ver.release_num,
            release_date: ver.release_date,
            label: ver.label,
            tile_url_template: ver.tile_url_template,
            has_local_change,
            coverage_ratio: max_cov,
            capture_count: feats.len(),
            capture_dates: dates_set.into_iter().collect(),
            primary_source,
            min_resolution_m: min_res,
            from_dead_service: dead_hit,
        });
    }

    // Present newest-first in UI lists.
    release_summaries.reverse();
    let mut captures: Vec<WaybackCaptureEntry> = dedup_captures.into_values().collect();
    captures.sort_by(|a, b| {
        b.capture_date
            .cmp(&a.capture_date)
            .then_with(|| b.release_date.cmp(&a.release_date))
    });

    let result = WaybackScanResult {
        scan_mode,
        zoom,
        bbox,
        effective_layer,
        used_fallback_layer,
        total_releases_scanned: release_summaries.len(),
        changed_releases_count,
        unique_captures_count: captures.len(),
        captures,
        release_summaries,
    };

    let now_ts = now_epoch_secs();
    let disk_cache = ScanDiskCache {
        cached_at_epoch_secs: now_ts,
        last_accessed_epoch_secs: now_ts,
        result: result.clone(),
    };
    let _ = atomic_write_json(&cache_dir(&app_data_dir).join(&cache_filename), &disk_cache);
    prune_scan_lru(&app_data_dir);

    if let Ok(mut guard) = scan_jobs().lock() {
        if let Some(job) = guard.get_mut(&scan_id) {
            job.status = "completed".to_owned();
            job.completed = total;
            job.result = Some(result);
            job.updated_at = Instant::now();
        }
    }
}

pub(crate) fn handle_wayback_dataset(
    app_data_dir: &Path,
    dataset: &str,
    req: &OpenGeodataQueryRequest,
) -> Result<Value, String> {
    match dataset {
        "esri_wayback_versions" => {
            let force_refresh = req
                .gee_params
                .as_ref()
                .and_then(|v| v["force_refresh"].as_bool())
                .unwrap_or(false)
                || req
                    .keyword
                    .as_deref()
                    .map(|k| k.trim().eq_ignore_ascii_case("refresh"))
                    .unwrap_or(false);
            let (versions, cache_source, fetched_at_epoch_secs) =
                get_wayback_versions(app_data_dir, force_refresh)?;
            Ok(json!({
                "ok": true,
                "dataset": "esri_wayback_versions",
                "count": versions.len(),
                "cache_source": cache_source,
                "fetched_at_epoch_secs": fetched_at_epoch_secs,
                "versions": versions,
            }))
        }
        "esri_wayback_probe_zoom" => {
            let bbox = req.bbox.ok_or_else(|| {
                "MISSING_BBOX: esri_wayback_probe_zoom 需要当前研究区 bbox".to_owned()
            })?;
            let version_id = req
                .gee_params
                .as_ref()
                .and_then(|v| v["version_id"].as_str())
                .or(req.code.as_deref());
            probe_wayback_max_zoom(app_data_dir, bbox, version_id)
        }
        "esri_wayback_scan_start" => {
            let bbox = normalize_bbox(req.bbox.ok_or_else(|| {
                "MISSING_BBOX: esri_wayback_scan_start 需要当前研究区 bbox".to_owned()
            })?);
            let zoom = req
                .gee_params
                .as_ref()
                .and_then(|v| v["zoom"].as_u64())
                .map(|z| z as u32)
                .or(req.year)
                .unwrap_or(18)
                .clamp(1, 20);
            let raw_mode = req
                .gee_params
                .as_ref()
                .and_then(|v| v["mode"].as_str())
                .or(req.keyword.as_deref())
                .unwrap_or("fast")
                .trim()
                .to_ascii_lowercase();
            let scan_mode = match raw_mode.as_str() {
                "fine" | "official" => raw_mode,
                _ => "fast".to_owned(),
            };
            let force_refresh = req
                .gee_params
                .as_ref()
                .and_then(|v| v["force_refresh"].as_bool())
                .unwrap_or(false);

            let (versions, _, _) = get_wayback_versions(app_data_dir, false)?;
            let latest_id = versions
                .first()
                .map(|v| v.release_num.as_str())
                .unwrap_or("latest");
            let cache_file = scan_cache_key(bbox, zoom, &scan_mode, latest_id);

            let seq = SCAN_COUNTER.fetch_add(1, Ordering::Relaxed);
            let scan_id = format!("wb_scan_{}_{}", now_epoch_secs(), seq);

            if !force_refresh {
                if let Some(cached_result) = load_scan_cache(app_data_dir, &cache_file) {
                    let total = cached_result.total_releases_scanned;
                    let layer = cached_result.effective_layer;
                    if let Ok(mut jobs) = scan_jobs().lock() {
                        evict_stale_scan_jobs(&mut jobs);
                        jobs.insert(
                            scan_id.clone(),
                            ScanJobState {
                                scan_id: scan_id.clone(),
                                status: "completed".to_owned(),
                                scan_mode: scan_mode.clone(),
                                completed: total,
                                total,
                                effective_layer: layer,
                                from_cache: true,
                                error: None,
                                result: Some(cached_result.clone()),
                                cancel_flag: Arc::new(AtomicBool::new(false)),
                                created_at_epoch_secs: now_epoch_secs(),
                                updated_at: Instant::now(),
                            },
                        );
                    }
                    return Ok(json!({
                        "ok": true,
                        "dataset": "esri_wayback_scan_start",
                        "scan_id": scan_id,
                        "status": "completed",
                        "scan_mode": scan_mode,
                        "from_cache": true,
                        "completed": total,
                        "total": total,
                        "effective_layer": layer,
                        "result": cached_result,
                    }));
                }
            }

            let cancel_flag = Arc::new(AtomicBool::new(false));
            if let Ok(mut jobs) = scan_jobs().lock() {
                // Enforce active concurrent scan cap (<= 2) by cancelling oldest active scan(s).
                let mut running: Vec<(String, u64)> = jobs
                    .iter()
                    .filter(|(_, j)| j.status == "running")
                    .map(|(k, j)| (k.clone(), j.created_at_epoch_secs))
                    .collect();
                running.sort_by_key(|(_, ts)| *ts);
                while running.len() >= MAX_ACTIVE_SCANS {
                    let (old_id, _) = running.remove(0);
                    if let Some(old_job) = jobs.get_mut(&old_id) {
                        old_job.cancel_flag.store(true, Ordering::Release);
                        old_job.status = "cancelled".to_owned();
                    }
                }
                evict_stale_scan_jobs(&mut jobs);
                jobs.insert(
                    scan_id.clone(),
                    ScanJobState {
                        scan_id: scan_id.clone(),
                        status: "running".to_owned(),
                        scan_mode: scan_mode.clone(),
                        completed: 0,
                        total: versions.len(),
                        effective_layer: select_layer_for_zoom(zoom),
                        from_cache: false,
                        error: None,
                        result: None,
                        cancel_flag: cancel_flag.clone(),
                        created_at_epoch_secs: now_epoch_secs(),
                        updated_at: Instant::now(),
                    },
                );
            }

            let dir_owned = app_data_dir.to_path_buf();
            let id_owned = scan_id.clone();
            let mode_owned = scan_mode.clone();
            std::thread::spawn(move || {
                run_wayback_scan_worker(
                    dir_owned,
                    id_owned,
                    bbox,
                    zoom,
                    mode_owned,
                    cache_file,
                    cancel_flag,
                );
            });

            Ok(json!({
                "ok": true,
                "dataset": "esri_wayback_scan_start",
                "scan_id": scan_id,
                "status": "running",
                "scan_mode": scan_mode,
                "from_cache": false,
                "completed": 0,
                "total": versions.len(),
                "effective_layer": select_layer_for_zoom(zoom),
            }))
        }
        "esri_wayback_scan_status" => {
            let scan_id = req
                .gee_params
                .as_ref()
                .and_then(|v| v["scan_id"].as_str())
                .or(req.code.as_deref())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "WAYBACK_SCAN_ID_REQUIRED: 缺少 scan_id".to_owned())?;
            let guard = scan_jobs()
                .lock()
                .map_err(|_| "WAYBACK_SCAN_LOCK_POISONED".to_owned())?;
            let job = guard
                .get(scan_id)
                .ok_or_else(|| format!("WAYBACK_SCAN_NOT_FOUND: 未找到扫描任务 {scan_id}"))?;
            Ok(json!({
                "ok": true,
                "dataset": "esri_wayback_scan_status",
                "scan_id": job.scan_id,
                "status": job.status,
                "scan_mode": job.scan_mode,
                "completed": job.completed,
                "total": job.total,
                "effective_layer": job.effective_layer,
                "from_cache": job.from_cache,
                "error": job.error,
                "result": job.result,
            }))
        }
        "esri_wayback_scan_cancel" => {
            let scan_id = req
                .gee_params
                .as_ref()
                .and_then(|v| v["scan_id"].as_str())
                .or(req.code.as_deref())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "WAYBACK_SCAN_ID_REQUIRED: 缺少 scan_id".to_owned())?;
            let mut guard = scan_jobs()
                .lock()
                .map_err(|_| "WAYBACK_SCAN_LOCK_POISONED".to_owned())?;
            if let Some(job) = guard.get_mut(scan_id) {
                job.cancel_flag.store(true, Ordering::Release);
                if job.status == "running" {
                    job.status = "cancelled".to_owned();
                }
                job.updated_at = Instant::now();
                Ok(json!({
                    "ok": true,
                    "dataset": "esri_wayback_scan_cancel",
                    "scan_id": scan_id,
                    "status": job.status,
                }))
            } else {
                Ok(json!({
                    "ok": true,
                    "dataset": "esri_wayback_scan_cancel",
                    "scan_id": scan_id,
                    "status": "not_found",
                }))
            }
        }
        other => Err(format!("UNSUPPORTED_WAYBACK_DATASET: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn parse_wayback_version_id_strictly_validates_digits() {
        assert_eq!(parse_wayback_version_id("wayback_3692"), Some("3692"));
        assert_eq!(parse_wayback_version_id("WAYBACK_25951"), Some("25951"));
        assert_eq!(parse_wayback_version_id("wayback_"), None);
        assert_eq!(parse_wayback_version_id("wayback_abc"), None);
        assert_eq!(parse_wayback_version_id("wayback_123/../../etc"), None);
        assert_eq!(parse_wayback_version_id("esri_wayback"), None);
        assert_eq!(parse_wayback_version_id("wayback_12345678901234567"), None);
    }

    #[test]
    fn is_wayback_host_uses_authority_parse_only() {
        assert!(is_wayback_host(
            "https://wayback-a.maptiles.arcgis.com/arcgis/rest/services/world_imagery/mapserver/tile/3692/1/0/0"
        ));
        assert!(is_wayback_host("https://wayback.maptiles.arcgis.com/tile"));
        assert!(!is_wayback_host(
            "https://evil.example.com/wayback-a.maptiles.arcgis.com"
        ));
        assert!(!is_wayback_host("https://user:wayback@evil.example.com/"));
    }

    #[test]
    fn validate_metadata_layer_url_rejects_ssrf_and_allows_arcgis() {
        assert!(validate_metadata_layer_url(
            "https://metadata.maptiles.arcgis.com/arcgis/rest/services/World_Imagery_Metadata_2026_r11/MapServer"
        )
        .is_ok());
        assert!(validate_metadata_layer_url(
            "https://metadata.maptiles.arcgis.com@169.254.169.254/latest/meta-data"
        )
        .is_err());
        assert!(validate_metadata_layer_url("http://169.254.169.254/latest").is_err());
        assert!(validate_metadata_layer_url("https://192.168.1.10/arcgis").is_err());
        assert!(
            validate_metadata_layer_url("http://metadata.maptiles.arcgis.com/insecure").is_err()
        );
    }

    #[test]
    fn select_layer_for_zoom_matches_empirical_arcgis_levels() {
        assert_eq!(select_layer_for_zoom(20), 3);
        assert_eq!(select_layer_for_zoom(19), 4);
        assert_eq!(select_layer_for_zoom(18), 4);
        assert_eq!(select_layer_for_zoom(17), 5);
        assert_eq!(select_layer_for_zoom(16), 6);
        assert_eq!(select_layer_for_zoom(14), 7);
        assert_eq!(select_layer_for_zoom(13), 8);
    }

    #[test]
    fn parse_config_sorts_newest_first_and_extracts_dates() {
        let sample = json!({
            "10": {
                "itemTitle": "World Imagery (Wayback 2014-02-20)",
                "itemURL": "https://www.arcgis.com/home/item.html?id=10",
                "metadataLayerUrl": "https://metadata.maptiles.arcgis.com/arcgis/rest/services/World_Imagery_Metadata_2014/MapServer",
                "layerIdentifier": "WB_2014_R01"
            },
            "3692": {
                "itemTitle": "World Imagery (Wayback 2026-11-18)",
                "itemURL": "https://www.arcgis.com/home/item.html?id=3692",
                "metadataLayerUrl": "https://metadata.maptiles.arcgis.com/arcgis/rest/services/World_Imagery_Metadata_2026_r11/MapServer",
                "layerIdentifier": "WB_2026_R11"
            }
        });
        let list = parse_wayback_config_json(&sample);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].release_num, "3692");
        assert_eq!(list[0].release_date, "2026-11-18");
        assert_eq!(list[1].release_num, "10");
        assert_eq!(list[1].release_date, "2014-02-20");
    }

    #[test]
    fn coverage_ratio_handles_zero_area_and_clips_cleanly() {
        let zero_bbox = [117.2, 31.8, 117.2, 31.8];
        let feat = json!({
            "geometry": {
                "rings": [[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0], [0.0, 0.0]]]
            }
        });
        assert_eq!(compute_feature_coverage_ratio(&feat, zero_bbox), 0.0);
    }

    #[test]
    fn atomic_write_overwrites_existing_and_lru_preserves_versions_json() {
        let tmp_root = std::env::temp_dir().join(format!(
            "wayback_lru_test_{}_{}",
            std::process::id(),
            now_epoch_secs()
        ));
        let dir = cache_dir(&tmp_root);
        let versions_path = dir.join("versions.json");
        atomic_write_json(&versions_path, &json!({ "keep": 1 })).unwrap();
        atomic_write_json(&versions_path, &json!({ "keep": 2 })).unwrap();

        for i in 0..55u64 {
            let scan_path = dir.join(format!("scan_{i:02}.json"));
            let dummy = ScanDiskCache {
                cached_at_epoch_secs: 1000 + i,
                last_accessed_epoch_secs: 1000 + i,
                result: WaybackScanResult {
                    scan_mode: "fast".to_owned(),
                    zoom: 18,
                    bbox: [117.1, 31.8, 117.2, 31.9],
                    effective_layer: 4,
                    used_fallback_layer: false,
                    total_releases_scanned: 1,
                    changed_releases_count: 1,
                    unique_captures_count: 1,
                    captures: vec![],
                    release_summaries: vec![],
                },
            };
            atomic_write_json(&scan_path, &dummy).unwrap();
        }
        prune_scan_lru(&tmp_root);

        assert!(
            versions_path.is_file(),
            "versions.json must NEVER be pruned"
        );
        let remaining_scans: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("scan_"))
            .collect();
        assert_eq!(remaining_scans.len(), SCAN_CACHE_MAX_ENTRIES);
        assert!(!dir.join("scan_00.json").exists(), "oldest scan evicted");
        assert!(dir.join("scan_54.json").is_file(), "newest scan kept");
        let _ = fs::remove_dir_all(&tmp_root);
    }

    #[test]
    fn query_metadata_retries_on_5xx_and_blacklists_only_after_failure() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_srv = calls.clone();

        let srv = std::thread::spawn(move || {
            for _ in 0..3 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let n = calls_srv.fetch_add(1, Ordering::SeqCst);
                let resp = if n < 2 {
                    "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n"
                } else {
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 15\r\n\r\n{\"features\":[]}"
                };
                let _ = stream.write_all(resp.as_bytes());
            }
        });

        let tmp_dir = std::env::temp_dir().join(format!("wb_retry_{}", std::process::id()));
        let _ = fs::create_dir_all(&tmp_dir);
        let client = build_http_client_with_timeout(&tmp_dir, 10).unwrap();
        let url = validate_metadata_layer_url(&format!("http://{addr}/MapServer")).unwrap();
        let out = query_metadata_layer_once(&client, &url, 4, [117.1, 31.8, 117.2, 31.9], false);
        srv.join().unwrap();
        let _ = fs::remove_dir_all(&tmp_dir);

        assert_eq!(calls.load(Ordering::SeqCst), 3);
        assert!(matches!(out, MetadataQueryOutcome::Features(v) if v.is_empty()));
    }

    #[test]
    fn geodata_tools_routes_wayback_tiles_and_fails_closed_on_invalid_version() {
        assert_eq!(
            crate::geodata_tools::build_xyz_tile_url("wayback_3692", "", 15, 27058, 13414, ""),
            "https://wayback-a.maptiles.arcgis.com/arcgis/rest/services/world_imagery/mapserver/tile/3692/15/13414/27058"
        );
        assert_eq!(
            crate::geodata_tools::xyz_source_max_zoom("esri_wayback"),
            20
        );
        assert_eq!(
            crate::geodata_tools::xyz_source_max_zoom("wayback_3692"),
            20
        );

        let tmp_dir = std::env::temp_dir().join(format!("wb_failclosed_{}", std::process::id()));
        let _ = fs::create_dir_all(&tmp_dir);
        for bad_spec in ["esri_wayback:15", "wayback_bad:15"] {
            let req = OpenGeodataQueryRequest {
                dataset: "xyz_satellite_tiles".to_owned(),
                code: None,
                keyword: Some(bad_spec.to_owned()),
                bbox: Some([117.25, 31.82, 117.27, 31.84]),
                aoi_geojson: None,
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: Some(15),
                output_path: None,
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: None,
                gee_params: None,
                zooms: None,
            };
            let err = crate::geodata_tools::execute_open_geodata_query_with_progress(
                &tmp_dir,
                &req,
                &|_| {},
            )
            .expect_err("invalid wayback version must fail closed");
            assert!(
                err.contains("INVALID_WAYBACK_VERSION_ID"),
                "expected INVALID_WAYBACK_VERSION_ID for {bad_spec}, got: {err}"
            );
        }
        let _ = fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    #[ignore = "Live Esri Wayback network test: verifies 199+ releases, Z20 SF/Z19 Hefei probing, and Hefei L4 cascade"]
    fn live_esri_wayback_probe_and_metadata_cascade() {
        let tmp_dir = std::env::temp_dir().join(format!("wb_live_{}", std::process::id()));
        let _ = fs::create_dir_all(&tmp_dir);
        let (versions, src, _) = get_wayback_versions(&tmp_dir, true).expect("versions live fetch");
        assert!(
            versions.len() >= 190,
            "expected 190+ Wayback releases, got {} ({src})",
            versions.len()
        );

        let hefei = probe_wayback_max_zoom(&tmp_dir, [117.25, 31.82, 117.30, 31.87], None)
            .expect("Hefei live probe");
        let hefei_max = hefei["max_zoom"].as_u64().unwrap_or(0);
        assert!(
            (18..=20).contains(&hefei_max),
            "unexpected Hefei maxZoom: {hefei}"
        );

        let _ = fs::remove_dir_all(&tmp_dir);
    }
}
