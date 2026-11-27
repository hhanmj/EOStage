use std::collections::BTreeSet;
use std::ffi::{c_char, c_int, c_uint, c_void, CStr, CString};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use super::runtime::{NativeGisRuntime, ProjContextGuard};
use super::stream_ops::{configure_gdal_runtime_for_source, normalize_gdal_source_string};
use super::{NativeGisError, NativeGisErrorCode};

const GDAL_OF_RASTER: c_uint = 0x02;
const GDAL_ACCESS_READ_ONLY: c_uint = 0x00;
const GDAL_READ: c_int = 0;
const GDAL_WRITE: c_int = 1;

const GDAL_BYTE: c_int = 1;
const GDAL_UINT16: c_int = 2;
const GDAL_INT16: c_int = 3;
const GDAL_UINT32: c_int = 4;
const GDAL_INT32: c_int = 5;
const GDAL_FLOAT32: c_int = 6;
const GDAL_FLOAT64: c_int = 7;

const PJ_FWD: c_int = 1;
const PJ_WKT2_2019: c_int = 2;

pub const OPTICAL_PRECHECK_GRID_SIZE: usize = 128;
pub const OPTICAL_OUTPUT_PIXEL_LIMIT: u64 = 400_000_000;
pub const OPTICAL_MOSAIC_MAX_SCENES: usize = 64;
pub const OPTICAL_MOSAIC_MAX_TEMPORAL_SPAN_DAYS: f64 = 16.0;
pub const OPTICAL_BIGTIFF_THRESHOLD_BYTES: u64 = 3_800_000_000;
pub const OPTICAL_MIN_FREE_DISK_BYTES: u64 = 128 * 1024 * 1024;
const STRIP_ROWS: usize = 512;

type Dataset = *mut c_void;
type Driver = *mut c_void;
type RasterBand = *mut c_void;
type ProjObject = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct ProjCoord {
    x: f64,
    y: f64,
    z: f64,
    t: f64,
}

type GdalOpenEx = unsafe extern "C" fn(
    *const c_char,
    c_uint,
    *const *const c_char,
    *const *const c_char,
    *const *const c_char,
) -> Dataset;
type GdalClose = unsafe extern "C" fn(Dataset);
type GdalGetRasterXSize = unsafe extern "C" fn(Dataset) -> c_int;
type GdalGetRasterYSize = unsafe extern "C" fn(Dataset) -> c_int;
type GdalGetRasterCount = unsafe extern "C" fn(Dataset) -> c_int;
type GdalGetProjectionRef = unsafe extern "C" fn(Dataset) -> *const c_char;
type GdalGetGeoTransform = unsafe extern "C" fn(Dataset, *mut f64) -> c_int;
type GdalGetRasterBand = unsafe extern "C" fn(Dataset, c_int) -> RasterBand;
type GdalGetRasterDataType = unsafe extern "C" fn(RasterBand) -> c_int;
type GdalGetRasterNoDataValue = unsafe extern "C" fn(RasterBand, *mut c_int) -> f64;
type GdalRasterIo = unsafe extern "C" fn(
    RasterBand,
    c_int,
    c_int,
    c_int,
    c_int,
    c_int,
    *mut c_void,
    c_int,
    c_int,
    c_int,
    c_int,
    c_int,
) -> c_int;
type GdalGetDriverByName = unsafe extern "C" fn(*const c_char) -> Driver;
type GdalCreate = unsafe extern "C" fn(
    Driver,
    *const c_char,
    c_int,
    c_int,
    c_int,
    c_int,
    *const *const c_char,
) -> Dataset;
type GdalSetProjection = unsafe extern "C" fn(Dataset, *const c_char) -> c_int;
type GdalSetGeoTransform = unsafe extern "C" fn(Dataset, *mut f64) -> c_int;
type GdalSetRasterNoDataValue = unsafe extern "C" fn(RasterBand, f64) -> c_int;
type GdalSetMetadataItem =
    unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char, *const c_char) -> c_int;
type GdalSetDescription = unsafe extern "C" fn(*mut c_void, *const c_char);
type GdalFlushCache = unsafe extern "C" fn(Dataset);
type GdalBuildOverviews = unsafe extern "C" fn(
    Dataset,
    *const c_char,
    c_int,
    *const c_int,
    c_int,
    *const c_int,
    *mut c_void,
    *mut c_void,
) -> c_int;

type ProjCreate = unsafe extern "C" fn(*mut c_void, *const c_char) -> ProjObject;
type ProjCreateCrsToCrs =
    unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char, *mut c_void) -> ProjObject;
type ProjNormalizeForVisualization = unsafe extern "C" fn(*mut c_void, ProjObject) -> ProjObject;
type ProjAsWkt =
    unsafe extern "C" fn(*mut c_void, ProjObject, c_int, *const *const c_char) -> *const c_char;
type ProjDestroy = unsafe extern "C" fn(ProjObject) -> ProjObject;
type ProjTrans = unsafe extern "C" fn(ProjObject, c_int, ProjCoord) -> ProjCoord;

struct DatasetGuard {
    raw: Dataset,
    close: GdalClose,
}

impl DatasetGuard {
    fn new(raw: Dataset, close: GdalClose) -> Self {
        Self { raw, close }
    }

    fn raw(&self) -> Dataset {
        self.raw
    }
}

impl Drop for DatasetGuard {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            unsafe { (self.close)(self.raw) };
        }
    }
}

struct ProjTransformGuard {
    raw: ProjObject,
    trans: ProjTrans,
    destroy: ProjDestroy,
}

impl ProjTransformGuard {
    fn from_crs(
        runtime: &NativeGisRuntime,
        ctx: &ProjContextGuard,
        src_crs: &str,
        dst_crs: &str,
    ) -> Result<Self, NativeGisError> {
        let src_c = CString::new(src_crs).map_err(|_| {
            error(
                NativeGisErrorCode::DemCrsMissing,
                "源坐标系字符串包含无效 NUL 字节",
            )
        })?;
        let dst_c = CString::new(dst_crs).map_err(|_| {
            error(
                NativeGisErrorCode::DemCrsMissing,
                "目标坐标系字符串包含无效 NUL 字节",
            )
        })?;
        unsafe {
            let create = *runtime.proj_symbol::<ProjCreateCrsToCrs>(b"proj_create_crs_to_crs\0")?;
            let normalize = *runtime.proj_symbol::<ProjNormalizeForVisualization>(
                b"proj_normalize_for_visualization\0",
            )?;
            let trans = *runtime.proj_symbol::<ProjTrans>(b"proj_trans\0")?;
            let destroy = *runtime.proj_symbol::<ProjDestroy>(b"proj_destroy\0")?;
            let pj = create(
                ctx.raw(),
                src_c.as_ptr(),
                dst_c.as_ptr(),
                std::ptr::null_mut(),
            );
            if pj.is_null() {
                return Err(error(
                    NativeGisErrorCode::DemReprojectionFailed,
                    "无法创建 PROJ 坐标转换对象",
                ));
            }
            let normalized = normalize(ctx.raw(), pj);
            destroy(pj);
            if normalized.is_null() {
                return Err(error(
                    NativeGisErrorCode::DemReprojectionFailed,
                    "无法标准化 PROJ 坐标轴顺序",
                ));
            }
            Ok(Self {
                raw: normalized,
                trans,
                destroy,
            })
        }
    }

    fn transform(&self, _runtime: &NativeGisRuntime, x: f64, y: f64) -> Option<(f64, f64)> {
        unsafe {
            let res = (self.trans)(
                self.raw,
                PJ_FWD,
                ProjCoord {
                    x,
                    y,
                    z: 0.0,
                    t: 0.0,
                },
            );
            (res.x.is_finite() && res.y.is_finite()).then_some((res.x, res.y))
        }
    }
}

impl Drop for ProjTransformGuard {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            unsafe { (self.destroy)(self.raw) };
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OpticalSourceKind {
    Sentinel2L2a,
    Sentinel2L1c,
    LandsatC2L2,
    HlsL30,
    HlsS30,
    OpticalRawCog,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OpticalDerivedMode {
    SingleSceneClip,
    BatchSingleSceneClip,
    NearCoincidentSpatialMosaic,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum OpticalResampling {
    Nearest,
    #[default]
    Bilinear,
    Cubic,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum OpticalCloudMaskPolicy {
    #[default]
    None,
    CloudToNodata,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum OpticalMosaicStrategy {
    #[default]
    QualityFirst,
    LatestFirst,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct OpticalBandAssetInput {
    pub band_id: String,
    pub stable_href: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_href: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub band_resolution_m: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct OpticalSceneInput {
    pub scene_id: String,
    pub source_kind: OpticalSourceKind,
    pub collection: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acquired_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloud_cover_pct: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub footprint_bbox_wgs84: Option<[f64; 4]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub footprint_rings_wgs84: Option<Vec<Vec<[f64; 2]>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_epsg: Option<u32>,
    #[serde(default)]
    pub band_assets: Vec<OpticalBandAssetInput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qa_asset: Option<OpticalBandAssetInput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secondary_qa_asset: Option<OpticalBandAssetInput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stac_metadata: Option<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct OpticalPrecheckRequest {
    pub aoi_bbox_wgs84: [f64; 4],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aoi_rings_wgs84: Option<Vec<Vec<[f64; 2]>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_resolution_m: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<OpticalDerivedMode>,
    pub scenes: Vec<OpticalSceneInput>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct OpticalSceneCoverageReport {
    pub scene_id: String,
    pub intersects_aoi: bool,
    pub aoi_coverage_fraction: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene_epsg: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acquired_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloud_cover_pct: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct OpticalPrecheckReport {
    pub ok: bool,
    pub recommended_mode: OpticalDerivedMode,
    pub aoi_bbox_wgs84: [f64; 4],
    pub aoi_center_wgs84: [f64; 2],
    pub recommended_utm_epsg: u32,
    pub crosses_utm_zones: bool,
    pub distinct_utm_zones: Vec<u32>,
    pub scene_count: usize,
    pub intersecting_scene_count: usize,
    pub combined_aoi_coverage_fraction: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temporal_span_days: Option<f64>,
    pub within_near_coincident_limit: bool,
    pub estimated_output_pixels_per_band: u64,
    pub warnings: Vec<String>,
    pub blocking_reasons: Vec<String>,
    pub scene_coverages: Vec<OpticalSceneCoverageReport>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct OpticalDerivedRequest {
    pub mode: OpticalDerivedMode,
    pub aoi_bbox_wgs84: [f64; 4],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aoi_rings_wgs84: Option<Vec<Vec<[f64; 2]>>>,
    pub band_order: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_crs: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_resolution_m: Option<f64>,
    #[serde(default)]
    pub resampling: OpticalResampling,
    #[serde(default)]
    pub cloud_mask: OpticalCloudMaskPolicy,
    #[serde(default)]
    pub mosaic_strategy: OpticalMosaicStrategy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_temporal_span_days: Option<f64>,
    #[serde(default)]
    pub overwrite: bool,
    pub scenes: Vec<OpticalSceneInput>,
    pub output_tif_path: PathBuf,
    #[serde(default = "default_true")]
    pub write_lineage_sidecar: bool,
    #[serde(default = "default_true")]
    pub write_provenance_sidecars: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct OpticalDerivedProgress {
    pub stage: String,
    pub scene_index: usize,
    pub scene_total: usize,
    pub strip_index: usize,
    pub strip_total: usize,
    pub bytes_written_estimate: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct OpticalSceneFailureRecord {
    pub scene_id: String,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct OpticalDerivedResult {
    pub ok: bool,
    pub mode: OpticalDerivedMode,
    pub output_tif_path: PathBuf,
    pub provenance_path: PathBuf,
    pub source_stac_path: PathBuf,
    pub aoi_geojson_path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineage_source_index_path: Option<PathBuf>,
    pub output_sha256: String,
    pub bytes_written: u64,
    pub width: u32,
    pub height: u32,
    pub band_count: usize,
    pub dtype: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nodata: Option<f64>,
    pub crs: String,
    pub geotransform: [f64; 6],
    pub is_bigtiff: bool,
    pub aoi_coverage_fraction: f64,
    pub valid_pixel_fraction: f64,
    pub scenes_succeeded: Vec<String>,
    pub scenes_failed: Vec<OpticalSceneFailureRecord>,
    pub credential_fields_redacted: usize,
    pub is_native_grid_exact: bool,
    #[serde(default)]
    pub fallback_pixels: u64,
    #[serde(default)]
    pub readback_verified: bool,
}

fn error(code: NativeGisErrorCode, reason: impl Into<String>) -> NativeGisError {
    NativeGisError::contract(code, reason)
}

fn c_path(path: &Path, code: NativeGisErrorCode, label: &str) -> Result<CString, NativeGisError> {
    CString::new(path.to_string_lossy().as_bytes()).map_err(|_| error(code, label))
}

fn is_sensitive_query_key(key: &str) -> bool {
    let lower = key.trim().to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "sig"
            | "se"
            | "sv"
            | "sr"
            | "sp"
            | "st"
            | "si"
            | "spr"
            | "skoid"
            | "sktid"
            | "skt"
            | "ske"
            | "sks"
            | "skv"
            | "token"
            | "access_token"
            | "id_token"
            | "refresh_token"
            | "x-amz-signature"
            | "x-amz-security-token"
            | "x-amz-credential"
            | "x-amz-date"
            | "x-amz-expires"
            | "x-amz-signedheaders"
            | "x-goog-signature"
            | "x-goog-credential"
            | "signature"
            | "apikey"
            | "api_key"
            | "auth"
            | "sas"
            | "bearer"
            | "password"
            | "cookie"
            | "secret"
            | "jwt"
    ) || lower.ends_with("_token")
        || lower.ends_with("_secret")
        || lower.ends_with("_password")
        || lower.ends_with("_signature")
}

/// Validates that an optical asset URL/path represents a raw single-scene COG/GeoTIFF
/// rather than rendered web map tiles (XYZ/WMTS/TMS/GEE/rendered PNG/JPEG/WEBP).
pub fn validate_raw_optical_cog_source(raw: &str) -> Result<(), NativeGisError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(error(
            NativeGisErrorCode::DemInvalidRaster,
            "[OPTICAL_SOURCE_REJECTED] 光学影像源地址不能为空",
        ));
    }
    let lower = trimmed.to_ascii_lowercase();
    let forbidden_substrings = [
        "{x}",
        "{y}",
        "{z}",
        "{q}",
        "{-y}",
        "tilematrixset",
        "request=gettile",
        "/wmts",
        "/tms/",
        "earthengine.googleapis.com",
        "/mapid",
        "/tiles/v1/",
        "rendered_preview",
        "tilejson",
        "image/png",
        "image/jpeg",
        "image/webp",
    ];
    for needle in forbidden_substrings {
        if lower.contains(needle) {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                format!(
                    "[OPTICAL_SOURCE_REJECTED] 拒绝非原始科学栅格或渲染瓦片源 ({needle}): {}",
                    redact_source_locator(trimmed).0
                ),
            ));
        }
    }

    let without_vsicurl = lower.strip_prefix("/vsicurl/").unwrap_or(&lower);
    let path_only = without_vsicurl.split('?').next().unwrap_or(without_vsicurl);
    let rendered_exts = [".png", ".jpg", ".jpeg", ".webp", ".gif", ".bmp", ".svg"];
    for ext in rendered_exts {
        if path_only.ends_with(ext) {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                format!(
                    "[OPTICAL_SOURCE_REJECTED] 拒绝渲染预览图像格式 ({ext})，仅允许原始科学 GeoTIFF/COG"
                ),
            ));
        }
    }

    if lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("/vsicurl/")
    {
        return Ok(());
    }
    if Path::new(trimmed).exists() {
        return Ok(());
    }
    Err(error(
        NativeGisErrorCode::DemInvalidRaster,
        format!(
            "[OPTICAL_SOURCE_REJECTED] 本地或受支持的远程 COG 源不存在: {}",
            redact_source_locator(trimmed).0
        ),
    ))
}

/// Strips signed SAS/presigned/token query parameters and URL userinfo from a source locator,
/// returning `(redacted_locator, credential_fields_redacted)`.
pub fn redact_source_locator(raw: &str) -> (String, usize) {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return (String::new(), 0);
    }
    let mut redacted_count = 0usize;

    let (prefix, rest) = if let Some(after) = trimmed.strip_prefix("/vsicurl/") {
        ("/vsicurl/", after)
    } else {
        ("", trimmed)
    };

    let (base_no_frag, fragment) = match rest.split_once('#') {
        Some((before, frag)) => (before, Some(frag)),
        None => (rest, None),
    };

    let (before_query, query_opt) = match base_no_frag.split_once('?') {
        Some((b, q)) => (b, Some(q)),
        None => (base_no_frag, None),
    };

    let cleaned_base = if let Some(scheme_idx) = before_query.find("://") {
        let after_scheme = &before_query[scheme_idx + 3..];
        let authority_end = after_scheme.find('/').unwrap_or(after_scheme.len());
        let authority = &after_scheme[..authority_end];
        if let Some((_, host)) = authority.split_once('@') {
            redacted_count += 1;
            format!(
                "{}{}{}",
                &before_query[..scheme_idx + 3],
                host,
                &after_scheme[authority_end..]
            )
        } else {
            before_query.to_owned()
        }
    } else {
        before_query.to_owned()
    };

    let mut kept_params = Vec::new();
    if let Some(query) = query_opt {
        for pair in query.split('&') {
            if pair.is_empty() {
                continue;
            }
            let key = pair.split('=').next().unwrap_or(pair);
            if is_sensitive_query_key(key) {
                redacted_count += 1;
            } else {
                kept_params.push(pair);
            }
        }
    }

    let mut out = format!("{prefix}{cleaned_base}");
    if !kept_params.is_empty() {
        out.push('?');
        out.push_str(&kept_params.join("&"));
    }
    if let Some(frag) = fragment {
        if !frag.is_empty() {
            out.push('#');
            out.push_str(frag);
        }
    }
    (out, redacted_count)
}

fn redact_json_value(value: Value, redacted_total: &mut usize) -> Value {
    match value {
        Value::String(s) => {
            if s.contains("://") || s.starts_with("/vsicurl/") || s.contains('?') {
                let (clean, count) = redact_source_locator(&s);
                *redacted_total += count;
                Value::String(clean)
            } else {
                Value::String(s)
            }
        }
        Value::Array(arr) => Value::Array(
            arr.into_iter()
                .map(|v| redact_json_value(v, redacted_total))
                .collect(),
        ),
        Value::Object(map) => {
            let mut next = Map::with_capacity(map.len());
            for (k, v) in map {
                if is_sensitive_query_key(&k) {
                    *redacted_total += 1;
                    continue;
                }
                next.insert(k, redact_json_value(v, redacted_total));
            }
            Value::Object(next)
        }
        other => other,
    }
}

pub fn validate_aoi_bbox(bbox: [f64; 4]) -> Result<[f64; 4], NativeGisError> {
    let [west, south, east, north] = bbox;
    if !west.is_finite()
        || !south.is_finite()
        || !east.is_finite()
        || !north.is_finite()
        || west >= east
        || south >= north
        || west < -180.0
        || east > 180.0
        || south < -90.0
        || north > 90.0
    {
        return Err(error(
            NativeGisErrorCode::AoiGeometryInvalid,
            "光学裁切 AOI 经纬度边界无效",
        ));
    }
    Ok(bbox)
}

pub fn utm_epsg_for_wgs84(lon: f64, lat: f64) -> u32 {
    let wrapped_lon = if lon == 180.0 {
        179.999999
    } else {
        lon.clamp(-180.0, 179.999999)
    };
    let mut zone = (((wrapped_lon + 180.0) / 6.0).floor() as i32 + 1).clamp(1, 60);
    if (56.0..64.0).contains(&lat) && (3.0..12.0).contains(&wrapped_lon) {
        zone = 32;
    } else if (72.0..84.0).contains(&lat) {
        if (0.0..9.0).contains(&wrapped_lon) {
            zone = 31;
        } else if (9.0..21.0).contains(&wrapped_lon) {
            zone = 33;
        } else if (21.0..33.0).contains(&wrapped_lon) {
            zone = 35;
        } else if (33.0..42.0).contains(&wrapped_lon) {
            zone = 37;
        }
    }
    if lat >= 0.0 {
        32600 + zone as u32
    } else {
        32700 + zone as u32
    }
}

fn standard_utm_zone(lon: f64) -> u32 {
    let wrapped = if lon == 180.0 {
        179.999999
    } else {
        lon.clamp(-180.0, 179.999999)
    };
    (((wrapped + 180.0) / 6.0).floor() as i32 + 1).clamp(1, 60) as u32
}

fn point_in_rings(x: f64, y: f64, rings: &[Vec<[f64; 2]>]) -> bool {
    let mut parity = false;
    for ring in rings {
        if ring.len() < 3 {
            continue;
        }
        let n = ring.len();
        let mut j = n - 1;
        for i in 0..n {
            let [xi, yi] = ring[i];
            let [xj, yj] = ring[j];
            let intersects =
                ((yi > y) != (yj > y)) && (x < (xj - xi) * (y - yi) / ((yj - yi) + 1e-30) + xi);
            if intersects {
                parity = !parity;
            }
            j = i;
        }
    }
    parity
}

#[derive(Clone, Copy)]
struct ProjectedPolygonEdge {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    y_min: f64,
    y_max: f64,
}

/// Pre-projects WGS84 AOI polygon edges into the output CRS once so strip
/// masking runs as an O(active_edges + width) scanline fill per row instead of
/// repeating per-pixel PROJ calls against all boundary vertices.
fn build_projected_polygon_edges(
    runtime: &NativeGisRuntime,
    ctx: &ProjContextGuard,
    target_wkt: &str,
    rings_wgs84: &[Vec<[f64; 2]>],
) -> Option<Vec<ProjectedPolygonEdge>> {
    let fwd = ProjTransformGuard::from_crs(runtime, ctx, "EPSG:4326", target_wkt).ok()?;
    let mut edges = Vec::new();
    for ring in rings_wgs84 {
        let n = ring.len();
        if n < 3 {
            continue;
        }
        let first_pt = fwd.transform(runtime, ring[0][0], ring[0][1])?;
        let mut cur_xy = first_pt;
        for idx in 0..n {
            let p0 = ring[idx];
            let p1 = ring[(idx + 1) % n];
            let span = (p1[0] - p0[0]).abs().max((p1[1] - p0[1]).abs());
            let steps = ((span / 0.02).ceil() as usize).clamp(1, 256);
            for step in 1..=steps {
                let next_xy = if step == steps && idx + 1 == n {
                    first_pt
                } else {
                    let t = (step as f64) / (steps as f64);
                    let lon = p0[0] + (p1[0] - p0[0]) * t;
                    let lat = p0[1] + (p1[1] - p0[1]) * t;
                    fwd.transform(runtime, lon, lat)?
                };
                if cur_xy.1 != next_xy.1 {
                    edges.push(ProjectedPolygonEdge {
                        x0: cur_xy.0,
                        y0: cur_xy.1,
                        x1: next_xy.0,
                        y1: next_xy.1,
                        y_min: cur_xy.1.min(next_xy.1),
                        y_max: cur_xy.1.max(next_xy.1),
                    });
                }
                cur_xy = next_xy;
            }
        }
    }
    Some(edges)
}

/// Fills an even-odd polygon mask across a horizontal north-up raster strip.
fn rasterize_projected_polygon_strip(
    edges: &[ProjectedPolygonEdge],
    out_gt: &[f64; 6],
    y0: usize,
    out_w: usize,
    strip_h: usize,
    inside_aoi: &mut [bool],
) {
    inside_aoi.fill(false);
    if edges.is_empty()
        || out_w == 0
        || strip_h == 0
        || out_gt[1] <= 0.0
        || out_gt[2].abs() >= 1e-12
        || out_gt[4].abs() >= 1e-12
    {
        return;
    }

    let row_y_top = out_gt[3] + (y0 as f64 + 0.5) * out_gt[5];
    let row_y_bot = out_gt[3] + ((y0 + strip_h - 1) as f64 + 0.5) * out_gt[5];
    let (strip_y_min, strip_y_max) = (row_y_top.min(row_y_bot), row_y_top.max(row_y_bot));
    let strip_edges: Vec<ProjectedPolygonEdge> = edges
        .iter()
        .copied()
        .filter(|e| e.y_max >= strip_y_min && e.y_min <= strip_y_max)
        .collect();
    if strip_edges.is_empty() {
        return;
    }

    let px_w = out_gt[1];
    let origin_x = out_gt[0];
    let mut crossings: Vec<f64> = Vec::new();
    for local_r in 0..strip_h {
        let out_y = out_gt[3] + ((y0 + local_r) as f64 + 0.5) * out_gt[5];
        crossings.clear();
        for e in &strip_edges {
            if out_y < e.y_min || out_y > e.y_max {
                continue;
            }
            if (e.y0 > out_y) != (e.y1 > out_y) {
                let x_cross = e.x0 + (e.x1 - e.x0) * (out_y - e.y0) / (e.y1 - e.y0);
                if x_cross.is_finite() {
                    crossings.push(x_cross);
                }
            }
        }
        if crossings.len() < 2 {
            continue;
        }
        crossings.sort_unstable_by(|a, b| a.total_cmp(b));
        let row_slice = &mut inside_aoi[local_r * out_w..(local_r + 1) * out_w];
        for pair in crossings.chunks_exact(2) {
            let x_even = pair[0];
            let x_odd = pair[1];
            if x_odd <= x_even {
                continue;
            }
            let mut start_col =
                (((x_even - origin_x) / px_w - 0.5).ceil().max(0.0) as usize).min(out_w);
            while start_col > 0 && (origin_x + (start_col as f64 - 0.5) * px_w) >= x_even {
                start_col -= 1;
            }
            while start_col < out_w && (origin_x + (start_col as f64 + 0.5) * px_w) < x_even {
                start_col += 1;
            }
            let mut end_col =
                (((x_odd - origin_x) / px_w - 0.5).ceil().max(0.0) as usize).min(out_w);
            while end_col > start_col && (origin_x + (end_col as f64 - 0.5) * px_w) >= x_odd {
                end_col -= 1;
            }
            while end_col < out_w && (origin_x + (end_col as f64 + 0.5) * px_w) < x_odd {
                end_col += 1;
            }
            if start_col < end_col {
                row_slice[start_col..end_col].fill(true);
            }
        }
    }
}

fn point_in_bbox(lon: f64, lat: f64, bbox: [f64; 4]) -> bool {
    lon >= bbox[0] && lon <= bbox[2] && lat >= bbox[1] && lat <= bbox[3]
}

fn is_rectangular_rings(bbox: [f64; 4], rings_opt: &Option<Vec<Vec<[f64; 2]>>>) -> bool {
    let Some(rings) = rings_opt else {
        return true;
    };
    if rings.is_empty() {
        return true;
    }
    if rings.len() != 1 {
        return false;
    }
    let ring = &rings[0];
    let eps = 1e-9;
    ring.iter().all(|&[x, y]| {
        ((x - bbox[0]).abs() < eps || (x - bbox[2]).abs() < eps)
            && ((y - bbox[1]).abs() < eps || (y - bbox[3]).abs() < eps)
    })
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = y - if m <= 2 { 1 } else { 0 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

pub fn parse_iso8601_epoch_seconds(raw: &str) -> Option<i64> {
    let trimmed = raw.trim();
    if trimmed.len() < 10 {
        return None;
    }
    let date_part = &trimmed[..10];
    let mut date_parts = date_part.split('-');
    let year: i64 = date_parts.next()?.parse().ok()?;
    let month: i64 = date_parts.next()?.parse().ok()?;
    let day: i64 = date_parts.next()?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let days = days_from_civil(year, month, day);
    let mut secs = days * 86_400;
    if let Some((_, time_raw)) = trimmed.split_once('T') {
        let hms = time_raw
            .trim_end_matches('Z')
            .split(['+', '-'])
            .next()
            .unwrap_or("");
        let mut t = hms.split(':');
        let h: i64 = t.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        let m: i64 = t.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        let s: i64 = t
            .next()
            .and_then(|s| s.split('.').next())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        secs += h * 3600 + m * 60 + s;
    }
    Some(secs)
}

pub fn precheck_aoi_scene_coverage(
    request: &OpticalPrecheckRequest,
) -> Result<OpticalPrecheckReport, NativeGisError> {
    let aoi_bbox = validate_aoi_bbox(request.aoi_bbox_wgs84)?;
    let [west, south, east, north] = aoi_bbox;
    let center_lon = (west + east) * 0.5;
    let center_lat = (south + north) * 0.5;
    let recommended_utm_epsg = utm_epsg_for_wgs84(center_lon, center_lat);

    let mut zone_set = BTreeSet::new();
    let z_w = standard_utm_zone(west);
    let z_e = standard_utm_zone(east);
    for z in z_w.min(z_e)..=z_w.max(z_e) {
        zone_set.insert(z);
    }
    for scene in &request.scenes {
        if let Some(epsg) = scene.native_epsg {
            if (32601..=32660).contains(&epsg) {
                zone_set.insert(epsg - 32600);
            } else if (32701..=32760).contains(&epsg) {
                zone_set.insert(epsg - 32700);
            }
        }
    }
    let distinct_utm_zones: Vec<u32> = zone_set.into_iter().collect();
    let crosses_utm_zones = distinct_utm_zones.len() > 1;

    let n = OPTICAL_PRECHECK_GRID_SIZE;
    let dx = (east - west) / (n as f64);
    let dy = (north - south) / (n as f64);
    let has_polygon = request
        .aoi_rings_wgs84
        .as_ref()
        .is_some_and(|rings| !rings.is_empty());

    let mut total_aoi_points = 0usize;
    let mut aoi_inside_points = Vec::with_capacity(n * n);
    for row in 0..n {
        let lat = south + (row as f64 + 0.5) * dy;
        for col in 0..n {
            let lon = west + (col as f64 + 0.5) * dx;
            let inside = if has_polygon {
                point_in_rings(lon, lat, request.aoi_rings_wgs84.as_deref().unwrap_or(&[]))
            } else {
                true
            };
            aoi_inside_points.push((lon, lat, inside));
            if inside {
                total_aoi_points += 1;
            }
        }
    }
    if total_aoi_points == 0 {
        return Err(error(
            NativeGisErrorCode::AoiGeometryInvalid,
            "光学裁切 AOI 多边形在边界内没有有效面积",
        ));
    }

    let mut union_covered = vec![false; n * n];
    let mut scene_coverages = Vec::with_capacity(request.scenes.len());
    let mut intersecting_scene_count = 0usize;

    for scene in &request.scenes {
        let mut scene_covered = 0usize;
        let has_scene_rings = scene
            .footprint_rings_wgs84
            .as_ref()
            .is_some_and(|r| !r.is_empty());
        for (idx, &(lon, lat, in_aoi)) in aoi_inside_points.iter().enumerate() {
            if !in_aoi {
                continue;
            }
            let in_scene = if has_scene_rings {
                point_in_rings(
                    lon,
                    lat,
                    scene.footprint_rings_wgs84.as_deref().unwrap_or(&[]),
                )
            } else if let Some(sb) = scene.footprint_bbox_wgs84 {
                point_in_bbox(lon, lat, sb)
            } else {
                true
            };
            if in_scene {
                scene_covered += 1;
                union_covered[idx] = true;
            }
        }
        let frac = (scene_covered as f64) / (total_aoi_points as f64);
        let intersects = scene_covered > 0;
        if intersects {
            intersecting_scene_count += 1;
        }
        scene_coverages.push(OpticalSceneCoverageReport {
            scene_id: scene.scene_id.clone(),
            intersects_aoi: intersects,
            aoi_coverage_fraction: frac,
            scene_epsg: scene.native_epsg,
            acquired_at: scene.acquired_at.clone(),
            cloud_cover_pct: scene.cloud_cover_pct,
        });
    }

    let union_count = union_covered.iter().filter(|&&b| b).count();
    let combined_aoi_coverage_fraction = (union_count as f64) / (total_aoi_points as f64);

    let epochs: Vec<i64> = request
        .scenes
        .iter()
        .filter_map(|s| {
            s.acquired_at
                .as_deref()
                .and_then(parse_iso8601_epoch_seconds)
        })
        .collect();
    let temporal_span_days = if epochs.len() >= 2 {
        let min_t = *epochs.iter().min().unwrap();
        let max_t = *epochs.iter().max().unwrap();
        Some(((max_t - min_t) as f64) / 86_400.0)
    } else if epochs.len() == 1 {
        Some(0.0)
    } else {
        None
    };

    let within_near_coincident_limit = request.scenes.len() <= OPTICAL_MOSAIC_MAX_SCENES
        && temporal_span_days.is_none_or(|d| d <= OPTICAL_MOSAIC_MAX_TEMPORAL_SPAN_DAYS);

    let recommended_mode = request.mode.unwrap_or(if request.scenes.len() <= 1 {
        OpticalDerivedMode::SingleSceneClip
    } else {
        OpticalDerivedMode::NearCoincidentSpatialMosaic
    });

    // Estimate output pixels using explicit target resolution when supplied, otherwise finest requested band resolution (default 10m if S2/COG else 30m)
    let default_res_m = request
        .target_resolution_m
        .filter(|r| r.is_finite() && *r > 0.0)
        .unwrap_or_else(|| {
            request
                .scenes
                .first()
                .and_then(|s| {
                    s.band_assets
                        .iter()
                        .filter_map(|b| b.band_resolution_m)
                        .reduce(f64::min)
                        .or(match s.source_kind {
                            OpticalSourceKind::Sentinel2L2a | OpticalSourceKind::Sentinel2L1c => {
                                Some(10.0)
                            }
                            OpticalSourceKind::LandsatC2L2
                            | OpticalSourceKind::HlsL30
                            | OpticalSourceKind::HlsS30 => Some(30.0),
                            OpticalSourceKind::OpticalRawCog => Some(10.0),
                        })
                })
                .unwrap_or(10.0)
                .max(0.5)
        });
    let meters_per_deg_lat = 111_320.0;
    let meters_per_deg_lon = (111_320.0 * center_lat.to_radians().cos().abs()).max(1_000.0);
    let est_w = ((east - west) * meters_per_deg_lon / default_res_m)
        .ceil()
        .max(1.0) as u64;
    let est_h = ((north - south) * meters_per_deg_lat / default_res_m)
        .ceil()
        .max(1.0) as u64;
    let estimated_output_pixels_per_band = est_w.saturating_mul(est_h);

    let mut warnings = Vec::new();
    let mut blocking_reasons = Vec::new();

    if request.scenes.is_empty() {
        blocking_reasons.push("未选择任何光学景记录".to_owned());
    } else if intersecting_scene_count == 0 {
        blocking_reasons.push("AOI_SCENE_DISJOINT: 所选光学景均与当前 AOI 不相交".to_owned());
    }
    if matches!(
        recommended_mode,
        OpticalDerivedMode::BatchSingleSceneClip | OpticalDerivedMode::NearCoincidentSpatialMosaic
    ) && request.scenes.len() > OPTICAL_MOSAIC_MAX_SCENES
    {
        blocking_reasons.push(format!(
            "单次最多支持 {} 景（当前选择 {} 景）",
            OPTICAL_MOSAIC_MAX_SCENES,
            request.scenes.len()
        ));
    }
    if recommended_mode == OpticalDerivedMode::NearCoincidentSpatialMosaic {
        if let Some(span) = temporal_span_days {
            if span > OPTICAL_MOSAIC_MAX_TEMPORAL_SPAN_DAYS {
                if request.mode.is_some() {
                    warnings.push(format!(
                        "所选多景影像时间跨度为 {:.1} 天（超过 {:.0} 天近同期推荐窗口），空间镶嵌可能包含跨时相地表变化",
                        span, OPTICAL_MOSAIC_MAX_TEMPORAL_SPAN_DAYS
                    ));
                } else {
                    blocking_reasons.push(format!(
                        "近同步空间镶嵌要求时相跨度 <= {:.0} 天（当前跨度 {:.1} 天）",
                        OPTICAL_MOSAIC_MAX_TEMPORAL_SPAN_DAYS, span
                    ));
                }
            }
        }
    }
    if estimated_output_pixels_per_band > OPTICAL_OUTPUT_PIXEL_LIMIT {
        blocking_reasons.push(format!(
            "OPTICAL_OUTPUT_PIXEL_LIMIT: 预计单波段输出像元数为 {estimated_output_pixels_per_band}，超过安全上限 {OPTICAL_OUTPUT_PIXEL_LIMIT}"
        ));
    }
    if crosses_utm_zones {
        warnings.push(format!(
            "所选区域或景跨越多个 UTM 带 ({:?})，将统一使用 EPSG:{} 坐标网格执行重投影对齐",
            distinct_utm_zones, recommended_utm_epsg
        ));
    }
    if combined_aoi_coverage_fraction < 0.99 && intersecting_scene_count > 0 {
        warnings.push(format!(
            "所选光学景对当前 AOI 的有效覆盖率为 {:.1}%，未覆盖区域将保留为 NoData",
            combined_aoi_coverage_fraction * 100.0
        ));
    }

    Ok(OpticalPrecheckReport {
        ok: blocking_reasons.is_empty(),
        recommended_mode,
        aoi_bbox_wgs84: aoi_bbox,
        aoi_center_wgs84: [center_lon, center_lat],
        recommended_utm_epsg,
        crosses_utm_zones,
        distinct_utm_zones,
        scene_count: request.scenes.len(),
        intersecting_scene_count,
        combined_aoi_coverage_fraction,
        temporal_span_days,
        within_near_coincident_limit,
        estimated_output_pixels_per_band,
        warnings,
        blocking_reasons,
        scene_coverages,
    })
}

fn free_bytes_for_path(path: &Path) -> Result<u64, NativeGisError> {
    let mut probe = path;
    while !probe.exists() {
        let Some(parent) = probe.parent() else {
            break;
        };
        probe = parent;
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let cstr = CString::new(probe.as_os_str().as_bytes()).map_err(|_| {
            error(
                NativeGisErrorCode::DemInvalidRaster,
                "输出路径包含无效字符，无法检测磁盘空间",
            )
        })?;
        let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
        let rc = unsafe { libc::statvfs(cstr.as_ptr(), &mut stat) };
        if rc != 0 {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                "无法读取目标路径磁盘可用空间",
            ));
        }
        Ok((stat.f_bavail as u64).saturating_mul(stat.f_frsize as u64))
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
        let wide: Vec<u16> = probe
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let mut free_available: u64 = 0;
        let mut total_bytes: u64 = 0;
        let mut total_free: u64 = 0;
        let ok = unsafe {
            GetDiskFreeSpaceExW(
                wide.as_ptr(),
                &mut free_available,
                &mut total_bytes,
                &mut total_free,
            )
        };
        if ok == 0 {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                "无法读取目标路径磁盘可用空间",
            ));
        }
        Ok(free_available)
    }
}

pub fn check_disk_space_budget(
    output_path: &Path,
    estimated_bytes: u64,
    free_bytes_override: Option<u64>,
) -> Result<u64, NativeGisError> {
    let required = estimated_bytes
        .saturating_mul(2)
        .max(OPTICAL_MIN_FREE_DISK_BYTES);
    let available = match free_bytes_override {
        Some(v) => v,
        None => free_bytes_for_path(output_path)?,
    };
    if available < required {
        return Err(error(
            NativeGisErrorCode::DemInvalidRaster,
            format!(
                "[OPTICAL_DISK_SPACE_INSUFFICIENT] 磁盘可用空间不足: 需要至少 {} MiB，当前可用 {} MiB",
                required.div_ceil(1024 * 1024),
                available / (1024 * 1024)
            ),
        ));
    }
    Ok(available)
}

fn sidecar_paths(
    output_tif: &Path,
) -> Result<(PathBuf, PathBuf, PathBuf, PathBuf), NativeGisError> {
    let parent = output_tif.parent().unwrap_or_else(|| Path::new("."));
    let file_name = output_tif
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| {
            error(
                NativeGisErrorCode::DemInvalidRaster,
                "光学裁切输出文件名无效",
            )
        })?;
    let stem = output_tif
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(file_name);
    let provenance_path = parent.join(format!("{file_name}.provenance.json"));
    let source_stac_path = parent.join(format!("{stem}.source.stac.json"));
    let aoi_geojson_path = parent.join(format!("{stem}.aoi.geojson"));
    let lineage_source_index_path = parent.join(format!("{stem}.lineage_source_index.tif"));
    Ok((
        provenance_path,
        source_stac_path,
        aoi_geojson_path,
        lineage_source_index_path,
    ))
}

fn part_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("output");
    path.with_file_name(format!("{name}.part"))
}

fn target_crs_to_wkt(
    runtime: &NativeGisRuntime,
    ctx: &ProjContextGuard,
    target_crs: &str,
) -> Result<String, NativeGisError> {
    let target_c = CString::new(target_crs).map_err(|_| {
        error(
            NativeGisErrorCode::DemCrsMissing,
            "目标坐标系标识包含无效字符",
        )
    })?;
    unsafe {
        let create = *runtime.proj_symbol::<ProjCreate>(b"proj_create\0")?;
        let as_wkt = *runtime.proj_symbol::<ProjAsWkt>(b"proj_as_wkt\0")?;
        let destroy = *runtime.proj_symbol::<ProjDestroy>(b"proj_destroy\0")?;
        let obj = create(ctx.raw(), target_c.as_ptr());
        if obj.is_null() {
            return Err(error(
                NativeGisErrorCode::DemCrsMissing,
                format!("无法识别目标坐标系: {target_crs}"),
            ));
        }
        let wkt_ptr = as_wkt(ctx.raw(), obj, PJ_WKT2_2019, std::ptr::null());
        let out = if wkt_ptr.is_null() {
            None
        } else {
            Some(CStr::from_ptr(wkt_ptr).to_string_lossy().into_owned())
        };
        destroy(obj);
        out.ok_or_else(|| {
            error(
                NativeGisErrorCode::DemCrsMissing,
                format!("无法导出目标坐标系 WKT: {target_crs}"),
            )
        })
    }
}

#[derive(Clone, Debug)]
struct RasterHeader {
    width: i32,
    height: i32,
    wkt: String,
    geotransform: [f64; 6],
    dtype: c_int,
    nodata: Option<f64>,
}

fn gdal_dtype_name(dtype: c_int) -> &'static str {
    match dtype {
        GDAL_BYTE => "Byte",
        GDAL_UINT16 => "UInt16",
        GDAL_INT16 => "Int16",
        GDAL_UINT32 => "UInt32",
        GDAL_INT32 => "Int32",
        GDAL_FLOAT32 => "Float32",
        GDAL_FLOAT64 => "Float64",
        _ => "Unknown",
    }
}

fn bytes_per_sample(dtype: c_int) -> u64 {
    match dtype {
        GDAL_BYTE => 1,
        GDAL_UINT16 | GDAL_INT16 => 2,
        GDAL_UINT32 | GDAL_INT32 | GDAL_FLOAT32 => 4,
        GDAL_FLOAT64 => 8,
        _ => 4,
    }
}

fn default_nodata_for_dtype(dtype: c_int) -> f64 {
    match dtype {
        GDAL_BYTE | GDAL_UINT16 | GDAL_UINT32 => 0.0,
        GDAL_INT16 | GDAL_INT32 => -9999.0,
        GDAL_FLOAT32 | GDAL_FLOAT64 => -9999.0,
        _ => 0.0,
    }
}

fn promote_dtype(a: c_int, b: c_int) -> c_int {
    if a == b {
        return a;
    }
    match (a, b) {
        (GDAL_FLOAT64, _) | (_, GDAL_FLOAT64) => GDAL_FLOAT64,
        (GDAL_FLOAT32, _) | (_, GDAL_FLOAT32) => GDAL_FLOAT32,
        (GDAL_INT32, _) | (_, GDAL_INT32) => GDAL_INT32,
        (GDAL_UINT32, GDAL_INT16) | (GDAL_INT16, GDAL_UINT32) => GDAL_INT32,
        (GDAL_UINT32, _) | (_, GDAL_UINT32) => GDAL_UINT32,
        (GDAL_INT16, GDAL_UINT16) | (GDAL_UINT16, GDAL_INT16) => GDAL_INT32,
        (GDAL_INT16, _) | (_, GDAL_INT16) => GDAL_INT16,
        (GDAL_UINT16, _) | (_, GDAL_UINT16) => GDAL_UINT16,
        _ => GDAL_FLOAT32,
    }
}

fn open_gdal_dataset(
    runtime: &NativeGisRuntime,
    raw_locator: &str,
) -> Result<(DatasetGuard, bool), NativeGisError> {
    validate_raw_optical_cog_source(raw_locator)?;
    let (normalized, is_remote) = normalize_gdal_source_string(raw_locator)?;
    unsafe {
        configure_gdal_runtime_for_source(runtime, is_remote)?;
        let open = *runtime.gdal_symbol::<GdalOpenEx>(b"GDALOpenEx\0")?;
        let close = *runtime.gdal_symbol::<GdalClose>(b"GDALClose\0")?;
        let path_c = CString::new(normalized.as_str()).map_err(|_| {
            error(
                NativeGisErrorCode::DemInvalidRaster,
                "光学栅格源路径编码无效",
            )
        })?;
        let ds = open(
            path_c.as_ptr(),
            GDAL_OF_RASTER | GDAL_ACCESS_READ_ONLY,
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
        );
        if ds.is_null() {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                format!(
                    "GDAL 无法打开光学影像源: {}",
                    redact_source_locator(&normalized).0
                ),
            ));
        }
        Ok((DatasetGuard::new(ds, close), is_remote))
    }
}

fn inspect_open_dataset(
    runtime: &NativeGisRuntime,
    ds: Dataset,
) -> Result<RasterHeader, NativeGisError> {
    unsafe {
        let get_w = *runtime.gdal_symbol::<GdalGetRasterXSize>(b"GDALGetRasterXSize\0")?;
        let get_h = *runtime.gdal_symbol::<GdalGetRasterYSize>(b"GDALGetRasterYSize\0")?;
        let get_cnt = *runtime.gdal_symbol::<GdalGetRasterCount>(b"GDALGetRasterCount\0")?;
        let get_proj = *runtime.gdal_symbol::<GdalGetProjectionRef>(b"GDALGetProjectionRef\0")?;
        let get_tf = *runtime.gdal_symbol::<GdalGetGeoTransform>(b"GDALGetGeoTransform\0")?;
        let get_band = *runtime.gdal_symbol::<GdalGetRasterBand>(b"GDALGetRasterBand\0")?;
        let get_dtype =
            *runtime.gdal_symbol::<GdalGetRasterDataType>(b"GDALGetRasterDataType\0")?;
        let get_nd =
            *runtime.gdal_symbol::<GdalGetRasterNoDataValue>(b"GDALGetRasterNoDataValue\0")?;

        let width = get_w(ds);
        let height = get_h(ds);
        let count = get_cnt(ds);
        if width <= 0 || height <= 0 || count < 1 {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                "光学 GeoTIFF 栅格尺寸或波段数为 0",
            ));
        }
        let proj_ptr = get_proj(ds);
        let wkt = if proj_ptr.is_null() {
            String::new()
        } else {
            CStr::from_ptr(proj_ptr).to_string_lossy().into_owned()
        };
        if wkt.trim().is_empty() {
            return Err(error(
                NativeGisErrorCode::DemCrsMissing,
                "光学 GeoTIFF 缺少坐标系投影定义",
            ));
        }
        let mut gt = [0.0f64; 6];
        if get_tf(ds, gt.as_mut_ptr()) != 0 {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                "光学 GeoTIFF 缺少仿射地理变换",
            ));
        }
        let band1 = get_band(ds, 1);
        if band1.is_null() {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                "光学 GeoTIFF 缺少 Band 1",
            ));
        }
        let dtype = get_dtype(band1);
        let mut has_nd: c_int = 0;
        let nd_val = get_nd(band1, &mut has_nd);
        let nodata = (has_nd != 0 && nd_val.is_finite()).then_some(nd_val);
        Ok(RasterHeader {
            width,
            height,
            wkt,
            geotransform: gt,
            dtype,
            nodata,
        })
    }
}

fn transform_points_bbox(
    runtime: &NativeGisRuntime,
    ctx: &ProjContextGuard,
    src_crs: &str,
    dst_crs: &str,
    bbox: [f64; 4],
    extra_rings: Option<&[Vec<[f64; 2]>]>,
) -> Option<[f64; 4]> {
    let tf = ProjTransformGuard::from_crs(runtime, ctx, src_crs, dst_crs).ok()?;
    let [west, south, east, north] = bbox;
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;

    let steps = 8usize;
    for i in 0..=steps {
        let f = (i as f64) / (steps as f64);
        let x_edge = west + (east - west) * f;
        let y_edge = south + (north - south) * f;
        for (px, py) in [
            (x_edge, south),
            (x_edge, north),
            (west, y_edge),
            (east, y_edge),
        ] {
            if let Some((tx, ty)) = tf.transform(runtime, px, py) {
                min_x = min_x.min(tx);
                max_x = max_x.max(tx);
                min_y = min_y.min(ty);
                max_y = max_y.max(ty);
            }
        }
    }
    if let Some(rings) = extra_rings {
        for ring in rings {
            for &[px, py] in ring {
                if let Some((tx, ty)) = tf.transform(runtime, px, py) {
                    min_x = min_x.min(tx);
                    max_x = max_x.max(tx);
                    min_y = min_y.min(ty);
                    max_y = max_y.max(ty);
                }
            }
        }
    }
    if min_x.is_finite() && min_y.is_finite() && max_x.is_finite() && max_y.is_finite() {
        Some([min_x, min_y, max_x, max_y])
    } else {
        None
    }
}

fn crs_grid_equivalent(
    runtime: &NativeGisRuntime,
    ctx: &ProjContextGuard,
    src_wkt: &str,
    dst_crs: &str,
    test_bbox_src: [f64; 4],
) -> bool {
    if src_wkt == dst_crs {
        return true;
    }
    let Ok(tf) = ProjTransformGuard::from_crs(runtime, ctx, src_wkt, dst_crs) else {
        return false;
    };
    let [x0, y0, x1, y1] = test_bbox_src;
    let tol = ((x1 - x0).abs() + (y1 - y0).abs()).max(1.0) * 1e-7;
    for (x, y) in [
        (x0, y0),
        (x1, y0),
        (x0, y1),
        (x1, y1),
        ((x0 + x1) * 0.5, (y0 + y1) * 0.5),
    ] {
        let Some((tx, ty)) = tf.transform(runtime, x, y) else {
            return false;
        };
        if (tx - x).abs() > tol || (ty - y).abs() > tol {
            return false;
        }
    }
    true
}

fn world_to_col_row(gt: &[f64; 6], x: f64, y: f64) -> Option<(f64, f64)> {
    let det = gt[1] * gt[5] - gt[2] * gt[4];
    if det.abs() < 1e-18 {
        return None;
    }
    let dx = x - gt[0];
    let dy = y - gt[3];
    let col = (gt[5] * dx - gt[2] * dy) / det;
    let row = (-gt[4] * dx + gt[1] * dy) / det;
    Some((col, row))
}

fn is_nodata_f64(v: f64, nodata: Option<f64>) -> bool {
    if !v.is_finite() {
        return true;
    }
    if let Some(nd) = nodata {
        if nd.is_nan() {
            return v.is_nan();
        }
        return (v - nd).abs() <= 1e-6 * nd.abs().max(1.0);
    }
    false
}

fn qa_pixel_is_cloudy(source_kind: OpticalSourceKind, band_id: &str, raw_val: f64) -> bool {
    if !raw_val.is_finite() {
        return true;
    }
    let lower_id = band_id.to_ascii_lowercase();
    let val = raw_val.round() as i64;
    if val < 0 {
        return true;
    }
    let uval = val as u64;
    if lower_id.contains("radsat") {
        // Landsat C2 QA_RADSAT: bits 0..=9 indicate radiometrically saturated bands (B1..B7/B9) or dropped detector pixel;
        // bit 11 (terrain occlusion) is intentionally preserved so steep mountain terrain in InSAR AOIs is not dropped.
        return uval == 0xFFFF || (uval & 0x03FF) != 0;
    }
    match source_kind {
        OpticalSourceKind::Sentinel2L2a => {
            // SCL classes: 0=NoData, 1=Saturated/defective, 3=Cloud shadow,
            // 8=Cloud medium prob, 9=Cloud high prob, 10=Thin cirrus
            matches!(uval, 0 | 1 | 3 | 8 | 9 | 10)
        }
        OpticalSourceKind::Sentinel2L1c => false,
        OpticalSourceKind::LandsatC2L2 => {
            // Collection 2 QA_PIXEL bits 0..=4: Fill, DilatedCloud, Cirrus, Cloud, CloudShadow
            (uval & 0b0001_1111) != 0
        }
        OpticalSourceKind::HlsL30 | OpticalSourceKind::HlsS30 => {
            // HLS v2.0 Fmask: 255=Fill, bits 0..=3 = Cirrus, Cloud, AdjCloud, CloudShadow
            uval == 255 || (uval & 0b0000_1111) != 0
        }
        OpticalSourceKind::OpticalRawCog => {
            if lower_id.contains("scl") {
                matches!(uval, 0 | 1 | 3 | 8 | 9 | 10)
            } else if lower_id.contains("fmask") {
                uval == 255 || (uval & 0b0000_1111) != 0
            } else if lower_id.contains("qa") {
                (uval & 0b0001_1111) != 0
            } else {
                uval != 0
            }
        }
    }
}

fn read_band_window_f64(
    runtime: &NativeGisRuntime,
    band: RasterBand,
    x_off: i32,
    y_off: i32,
    w: i32,
    h: i32,
) -> Result<Vec<f64>, NativeGisError> {
    let len = (w as usize)
        .checked_mul(h as usize)
        .ok_or_else(|| error(NativeGisErrorCode::DemInvalidRaster, "波段读取窗口大小溢出"))?;
    let mut buf = vec![0.0f64; len];
    unsafe {
        let raster_io = *runtime.gdal_symbol::<GdalRasterIo>(b"GDALRasterIO\0")?;
        if raster_io(
            band,
            GDAL_READ,
            x_off,
            y_off,
            w,
            h,
            buf.as_mut_ptr().cast(),
            w,
            h,
            GDAL_FLOAT64,
            0,
            0,
        ) != 0
        {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                "GDALRasterIO 读取光学波段数据失败",
            ));
        }
    }
    Ok(buf)
}

fn read_band_window_native_bytes(
    runtime: &NativeGisRuntime,
    band: RasterBand,
    dtype: c_int,
    x_off: i32,
    y_off: i32,
    w: i32,
    h: i32,
) -> Result<Vec<u8>, NativeGisError> {
    let bps = bytes_per_sample(dtype) as usize;
    let byte_len = (w as usize)
        .checked_mul(h as usize)
        .and_then(|p| p.checked_mul(bps))
        .ok_or_else(|| error(NativeGisErrorCode::DemInvalidRaster, "波段原生窗口字节溢出"))?;
    let mut buf = vec![0u8; byte_len];
    unsafe {
        let raster_io = *runtime.gdal_symbol::<GdalRasterIo>(b"GDALRasterIO\0")?;
        if raster_io(
            band,
            GDAL_READ,
            x_off,
            y_off,
            w,
            h,
            buf.as_mut_ptr().cast(),
            w,
            h,
            dtype,
            0,
            0,
        ) != 0
        {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                "GDALRasterIO 读取原生光学波段像元失败",
            ));
        }
    }
    Ok(buf)
}

#[derive(Clone, Copy)]
struct PixelWindow {
    x_off: i32,
    y_off: i32,
    w: i32,
    h: i32,
}

struct StripWrite<'a> {
    dtype: c_int,
    nodata_val: f64,
    win: PixelWindow,
    values: &'a [f64],
    valid: &'a [bool],
}

fn write_band_window_native_bytes(
    runtime: &NativeGisRuntime,
    band: RasterBand,
    dtype: c_int,
    win: PixelWindow,
    buf: &mut [u8],
) -> Result<(), NativeGisError> {
    let PixelWindow { x_off, y_off, w, h } = win;
    unsafe {
        let raster_io = *runtime.gdal_symbol::<GdalRasterIo>(b"GDALRasterIO\0")?;
        if raster_io(
            band,
            GDAL_WRITE,
            x_off,
            y_off,
            w,
            h,
            buf.as_mut_ptr().cast(),
            w,
            h,
            dtype,
            0,
            0,
        ) != 0
        {
            return Err(error(
                NativeGisErrorCode::DemReprojectionFailed,
                "GDALRasterIO 写入输出光学波段像元失败",
            ));
        }
    }
    Ok(())
}

fn write_f64_strip_to_band(
    runtime: &NativeGisRuntime,
    band: RasterBand,
    strip: StripWrite<'_>,
) -> Result<(), NativeGisError> {
    let StripWrite {
        dtype,
        nodata_val,
        win,
        values,
        valid,
    } = strip;
    let bps = bytes_per_sample(dtype) as usize;
    let mut raw = vec![0u8; values.len() * bps];
    for (idx, (&val, &ok)) in values.iter().zip(valid.iter()).enumerate() {
        let chosen = if ok && val.is_finite() {
            val
        } else {
            nodata_val
        };
        let offset = idx * bps;
        match dtype {
            GDAL_BYTE => {
                raw[offset] = chosen.round().clamp(0.0, u8::MAX as f64) as u8;
            }
            GDAL_UINT16 => {
                let v = chosen.round().clamp(0.0, u16::MAX as f64) as u16;
                raw[offset..offset + 2].copy_from_slice(&v.to_ne_bytes());
            }
            GDAL_INT16 => {
                let v = chosen.round().clamp(i16::MIN as f64, i16::MAX as f64) as i16;
                raw[offset..offset + 2].copy_from_slice(&v.to_ne_bytes());
            }
            GDAL_UINT32 => {
                let v = chosen.round().clamp(0.0, u32::MAX as f64) as u32;
                raw[offset..offset + 4].copy_from_slice(&v.to_ne_bytes());
            }
            GDAL_INT32 => {
                let v = chosen.round().clamp(i32::MIN as f64, i32::MAX as f64) as i32;
                raw[offset..offset + 4].copy_from_slice(&v.to_ne_bytes());
            }
            GDAL_FLOAT32 => {
                let v = chosen as f32;
                raw[offset..offset + 4].copy_from_slice(&v.to_ne_bytes());
            }
            GDAL_FLOAT64 => {
                raw[offset..offset + 8].copy_from_slice(&chosen.to_ne_bytes());
            }
            _ => {
                let v = chosen as f32;
                raw[offset..offset + 4].copy_from_slice(&v.to_ne_bytes());
            }
        }
    }
    write_band_window_native_bytes(runtime, band, dtype, win, &mut raw)
}

fn mask_native_bytes_outside_valid(raw: &mut [u8], dtype: c_int, nodata_val: f64, valid: &[bool]) {
    let bps = bytes_per_sample(dtype) as usize;
    for (idx, &ok) in valid.iter().enumerate() {
        if ok {
            continue;
        }
        let offset = idx * bps;
        match dtype {
            GDAL_BYTE => {
                raw[offset] = nodata_val.round().clamp(0.0, u8::MAX as f64) as u8;
            }
            GDAL_UINT16 => {
                let v = nodata_val.round().clamp(0.0, u16::MAX as f64) as u16;
                raw[offset..offset + 2].copy_from_slice(&v.to_ne_bytes());
            }
            GDAL_INT16 => {
                let v = nodata_val.round().clamp(i16::MIN as f64, i16::MAX as f64) as i16;
                raw[offset..offset + 2].copy_from_slice(&v.to_ne_bytes());
            }
            GDAL_UINT32 => {
                let v = nodata_val.round().clamp(0.0, u32::MAX as f64) as u32;
                raw[offset..offset + 4].copy_from_slice(&v.to_ne_bytes());
            }
            GDAL_INT32 => {
                let v = nodata_val.round().clamp(i32::MIN as f64, i32::MAX as f64) as i32;
                raw[offset..offset + 4].copy_from_slice(&v.to_ne_bytes());
            }
            GDAL_FLOAT32 => {
                let v = nodata_val as f32;
                raw[offset..offset + 4].copy_from_slice(&v.to_ne_bytes());
            }
            GDAL_FLOAT64 => {
                raw[offset..offset + 8].copy_from_slice(&nodata_val.to_ne_bytes());
            }
            _ => {}
        }
    }
}

fn count_valid_in_native_bytes(raw: &[u8], dtype: c_int, nodata: Option<f64>) -> usize {
    let bps = bytes_per_sample(dtype) as usize;
    let count = raw.len() / bps;
    let mut valid = 0usize;
    for idx in 0..count {
        let offset = idx * bps;
        let val = match dtype {
            GDAL_BYTE => f64::from(raw[offset]),
            GDAL_UINT16 => {
                let b: [u8; 2] = raw[offset..offset + 2].try_into().unwrap();
                f64::from(u16::from_ne_bytes(b))
            }
            GDAL_INT16 => {
                let b: [u8; 2] = raw[offset..offset + 2].try_into().unwrap();
                f64::from(i16::from_ne_bytes(b))
            }
            GDAL_UINT32 => {
                let b: [u8; 4] = raw[offset..offset + 4].try_into().unwrap();
                f64::from(u32::from_ne_bytes(b))
            }
            GDAL_INT32 => {
                let b: [u8; 4] = raw[offset..offset + 4].try_into().unwrap();
                f64::from(i32::from_ne_bytes(b))
            }
            GDAL_FLOAT32 => {
                let b: [u8; 4] = raw[offset..offset + 4].try_into().unwrap();
                f64::from(f32::from_ne_bytes(b))
            }
            GDAL_FLOAT64 => {
                let b: [u8; 8] = raw[offset..offset + 8].try_into().unwrap();
                f64::from_ne_bytes(b)
            }
            _ => continue,
        };
        if !is_nodata_f64(val, nodata) {
            valid += 1;
        }
    }
    valid
}

#[derive(Clone, Copy)]
struct F64Window<'a> {
    buf: &'a [f64],
    x0: i32,
    y0: i32,
    w: i32,
    h: i32,
}

fn sample_window_f64(
    win: F64Window<'_>,
    col_f: f64,
    row_f: f64,
    resampling: OpticalResampling,
    src_nodata: Option<f64>,
) -> Option<f64> {
    let F64Window {
        buf,
        x0: win_x0,
        y0: win_y0,
        w: win_w,
        h: win_h,
    } = win;
    let local_c = col_f - f64::from(win_x0);
    let local_r = row_f - f64::from(win_y0);
    if local_c < 0.0 || local_r < 0.0 || local_c >= f64::from(win_w) || local_r >= f64::from(win_h)
    {
        return None;
    }
    let get = |c: i32, r: i32| -> Option<f64> {
        if c < 0 || r < 0 || c >= win_w || r >= win_h {
            return None;
        }
        let v = buf[(r as usize) * (win_w as usize) + (c as usize)];
        (!is_nodata_f64(v, src_nodata)).then_some(v)
    };
    match resampling {
        OpticalResampling::Nearest => {
            let c = local_c.floor().clamp(0.0, f64::from(win_w - 1)) as i32;
            let r = local_r.floor().clamp(0.0, f64::from(win_h - 1)) as i32;
            get(c, r)
        }
        OpticalResampling::Bilinear | OpticalResampling::Cubic => {
            // Sample at pixel center coordinates (col_f - 0.5, row_f - 0.5)
            let cx = local_c - 0.5;
            let cy = local_r - 0.5;
            let c0 = cx.floor() as i32;
            let r0 = cy.floor() as i32;
            let fx = cx - f64::from(c0);
            let fy = cy - f64::from(r0);

            if resampling == OpticalResampling::Cubic {
                let mut sum_w = 0.0;
                let mut sum_v = 0.0;
                for dr in -1..=2 {
                    let wy = catmull_rom(f64::from(dr) - fy);
                    if wy == 0.0 {
                        continue;
                    }
                    for dc in -1..=2 {
                        let wx = catmull_rom(f64::from(dc) - fx);
                        if wx == 0.0 {
                            continue;
                        }
                        if let Some(v) = get(c0 + dc, r0 + dr) {
                            let w = wx * wy;
                            sum_v += v * w;
                            sum_w += w;
                        }
                    }
                }
                if sum_w > 1e-6 {
                    let mut interp = sum_v / sum_w;
                    // Fallback clamp only if all contributing weights had opposite signs
                    if !interp.is_finite() {
                        return get(local_c.floor() as i32, local_r.floor() as i32);
                    }
                    // Prevent negative overshoot on reflectance unsigned bands
                    if src_nodata.is_some_and(|nd| nd >= 0.0) && interp < 0.0 {
                        interp = 0.0;
                    }
                    return Some(interp);
                }
                return get(local_c.floor() as i32, local_r.floor() as i32);
            }

            let candidates = [
                (get(c0, r0), (1.0 - fx) * (1.0 - fy)),
                (get(c0 + 1, r0), fx * (1.0 - fy)),
                (get(c0, r0 + 1), (1.0 - fx) * fy),
                (get(c0 + 1, r0 + 1), fx * fy),
            ];
            let mut sum_v = 0.0;
            let mut sum_w = 0.0;
            for (val_opt, w) in candidates {
                if let Some(v) = val_opt {
                    sum_v += v * w;
                    sum_w += w;
                }
            }
            if sum_w > 1e-6 {
                Some(sum_v / sum_w)
            } else {
                get(local_c.floor() as i32, local_r.floor() as i32)
            }
        }
    }
}

fn catmull_rom(t: f64) -> f64 {
    let x = t.abs();
    if x < 1.0 {
        1.5 * x * x * x - 2.5 * x * x + 1.0
    } else if x < 2.0 {
        -0.5 * x * x * x + 2.5 * x * x - 4.0 * x + 2.0
    } else {
        0.0
    }
}

struct OutputGeoTiffSpec<'a> {
    path: &'a Path,
    width: i32,
    height: i32,
    band_count: i32,
    dtype: c_int,
    crs_wkt: &'a str,
    geotransform: &'a [f64; 6],
    nodata: Option<f64>,
    band_names: &'a [String],
    use_bigtiff: bool,
}

fn create_output_geotiff(
    runtime: &NativeGisRuntime,
    spec: OutputGeoTiffSpec<'_>,
) -> Result<DatasetGuard, NativeGisError> {
    let OutputGeoTiffSpec {
        path,
        width,
        height,
        band_count,
        dtype,
        crs_wkt,
        geotransform,
        nodata,
        band_names,
        use_bigtiff,
    } = spec;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            error(
                NativeGisErrorCode::DemReprojectionFailed,
                format!("无法创建光学输出目录: {e}"),
            )
        })?;
    }
    unsafe {
        configure_gdal_runtime_for_source(runtime, false)?;
        let driver_name = CString::new("GTiff").expect("GTiff");
        let get_driver = *runtime.gdal_symbol::<GdalGetDriverByName>(b"GDALGetDriverByName\0")?;
        let driver = get_driver(driver_name.as_ptr());
        if driver.is_null() {
            return Err(error(
                NativeGisErrorCode::GisDriverUnavailable,
                "GTiff 驱动不可用",
            ));
        }
        let path_c = c_path(
            path,
            NativeGisErrorCode::DemReprojectionFailed,
            "输出路径编码无效",
        )?;
        let compress = CString::new("COMPRESS=DEFLATE").expect("static option");
        let tiled = CString::new("TILED=YES").expect("static option");
        let bx = CString::new("BLOCKXSIZE=256").expect("static option");
        let by = CString::new("BLOCKYSIZE=256").expect("static option");
        let bigtiff = CString::new(if use_bigtiff {
            "BIGTIFF=YES"
        } else {
            "BIGTIFF=IF_SAFER"
        })
        .expect("static option");
        let opts = [
            compress.as_ptr(),
            tiled.as_ptr(),
            bx.as_ptr(),
            by.as_ptr(),
            bigtiff.as_ptr(),
            std::ptr::null(),
        ];

        let create = *runtime.gdal_symbol::<GdalCreate>(b"GDALCreate\0")?;
        let close = *runtime.gdal_symbol::<GdalClose>(b"GDALClose\0")?;
        let ds = create(
            driver,
            path_c.as_ptr(),
            width,
            height,
            band_count,
            dtype,
            opts.as_ptr(),
        );
        if ds.is_null() {
            return Err(error(
                NativeGisErrorCode::DemReprojectionFailed,
                "无法创建输出 GeoTIFF 文件",
            ));
        }
        let guard = DatasetGuard::new(ds, close);

        let set_proj = *runtime.gdal_symbol::<GdalSetProjection>(b"GDALSetProjection\0")?;
        let wkt_c = CString::new(crs_wkt).map_err(|_| {
            error(
                NativeGisErrorCode::DemCrsMissing,
                "输出坐标系 WKT 包含无效 NUL 字节",
            )
        })?;
        if set_proj(guard.raw(), wkt_c.as_ptr()) != 0 {
            return Err(error(
                NativeGisErrorCode::DemReprojectionFailed,
                "设置输出 GeoTIFF 坐标系失败",
            ));
        }
        let set_tf = *runtime.gdal_symbol::<GdalSetGeoTransform>(b"GDALSetGeoTransform\0")?;
        let mut gt_mut = *geotransform;
        if set_tf(guard.raw(), gt_mut.as_mut_ptr()) != 0 {
            return Err(error(
                NativeGisErrorCode::DemReprojectionFailed,
                "设置输出 GeoTIFF 仿射地理变换失败",
            ));
        }

        let get_band = *runtime.gdal_symbol::<GdalGetRasterBand>(b"GDALGetRasterBand\0")?;
        let set_nd =
            *runtime.gdal_symbol::<GdalSetRasterNoDataValue>(b"GDALSetRasterNoDataValue\0")?;
        let set_meta = *runtime.gdal_symbol::<GdalSetMetadataItem>(b"GDALSetMetadataItem\0")?;
        let set_desc = runtime
            .gdal_symbol::<GdalSetDescription>(b"GDALSetDescription\0")
            .ok();

        let meta_key = CString::new("BAND_ID").expect("BAND_ID");
        for idx in 0..band_count {
            let band = get_band(guard.raw(), idx + 1);
            if !band.is_null() {
                if let Some(nd) = nodata {
                    let _ = set_nd(band, nd);
                }
                if let Some(name) = band_names.get(idx as usize) {
                    if let Ok(name_c) = CString::new(name.as_str()) {
                        let _ =
                            set_meta(band, meta_key.as_ptr(), name_c.as_ptr(), std::ptr::null());
                        if let Some(sd) = set_desc.as_ref() {
                            sd(band, name_c.as_ptr());
                        }
                    }
                }
            }
        }
        Ok(guard)
    }
}

fn flush_and_finalize_dataset(
    runtime: &NativeGisRuntime,
    guard: DatasetGuard,
    width: i32,
    height: i32,
) -> Result<(), NativeGisError> {
    unsafe {
        if width >= 64 && height >= 64 {
            if let Ok(build_ovr) =
                runtime.gdal_symbol::<GdalBuildOverviews>(b"GDALBuildOverviews\0")
            {
                let method = CString::new("NEAREST").expect("NEAREST");
                let levels = [2, 4, 8, 16];
                let _ = build_ovr(
                    guard.raw(),
                    method.as_ptr(),
                    levels.len() as c_int,
                    levels.as_ptr(),
                    0,
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                );
            }
        }
        let flush = *runtime.gdal_symbol::<GdalFlushCache>(b"GDALFlushCache\0")?;
        flush(guard.raw());
    }
    drop(guard);
    Ok(())
}

struct ReadbackVerifySpec<'a> {
    path: &'a Path,
    expected_width: i32,
    expected_height: i32,
    expected_band_count: i32,
    expected_dtype: c_int,
    expected_geotransform: &'a [f64; 6],
    expected_nodata: Option<f64>,
}

fn verify_written_geotiff_readback(
    runtime: &NativeGisRuntime,
    spec: ReadbackVerifySpec<'_>,
) -> Result<(), NativeGisError> {
    let wrap_err = |msg: String| {
        error(
            NativeGisErrorCode::DemReprojectionFailed,
            format!("[OPTICAL_READBACK_VERIFY_FAILED] {msg}"),
        )
    };
    let (ds, _) = open_gdal_dataset(runtime, &spec.path.to_string_lossy())
        .map_err(|e| wrap_err(format!("重新打开输出 GeoTIFF 失败: {e}")))?;
    let header = inspect_open_dataset(runtime, ds.raw())
        .map_err(|e| wrap_err(format!("解析输出 GeoTIFF 头信息失败: {e}")))?;
    if header.width != spec.expected_width || header.height != spec.expected_height {
        return Err(wrap_err(format!(
            "输出尺寸不匹配: 期望 {}x{}, 实际 {}x{}",
            spec.expected_width, spec.expected_height, header.width, header.height
        )));
    }
    let band_count = unsafe {
        let get_cnt = *runtime.gdal_symbol::<GdalGetRasterCount>(b"GDALGetRasterCount\0")?;
        get_cnt(ds.raw())
    };
    if band_count != spec.expected_band_count {
        return Err(wrap_err(format!(
            "输出波段数不匹配: 期望 {}, 实际 {}",
            spec.expected_band_count, band_count
        )));
    }
    if header.dtype != spec.expected_dtype {
        return Err(wrap_err(format!(
            "输出像元数据类型不匹配: 期望 {}, 实际 {}",
            spec.expected_dtype, header.dtype
        )));
    }
    for (idx, (got, exp)) in header
        .geotransform
        .iter()
        .zip(spec.expected_geotransform.iter())
        .enumerate()
    {
        let tol = 1e-6 * exp.abs().max(1.0);
        if (got - exp).abs() > tol {
            return Err(wrap_err(format!(
                "输出 GeoTransform[{idx}] 不匹配: 期望 {exp}, 实际 {got}"
            )));
        }
    }
    if let Some(exp_nd) = spec.expected_nodata {
        let matches = match header.nodata {
            Some(got_nd) if exp_nd.is_nan() && got_nd.is_nan() => true,
            Some(got_nd) => (got_nd - exp_nd).abs() <= 1e-6 * exp_nd.abs().max(1.0),
            None => false,
        };
        if !matches {
            return Err(wrap_err(format!(
                "输出 NoData 元数据不匹配: 期望 {exp_nd}, 实际 {:?}",
                header.nodata
            )));
        }
    }

    let mut sample_rows = BTreeSet::new();
    sample_rows.insert(0);
    sample_rows.insert((header.height / 2).clamp(0, header.height - 1));
    sample_rows.insert((header.height - 1).max(0));

    let get_band = unsafe { *runtime.gdal_symbol::<GdalGetRasterBand>(b"GDALGetRasterBand\0")? };
    let expected_row_bytes =
        (header.width as usize).saturating_mul(bytes_per_sample(spec.expected_dtype) as usize);
    for b_idx in 1..=band_count {
        let band = unsafe { get_band(ds.raw(), b_idx) };
        if band.is_null() {
            return Err(wrap_err(format!("输出缺少波段 {b_idx}")));
        }
        for &row_y in &sample_rows {
            let bytes = read_band_window_native_bytes(
                runtime,
                band,
                spec.expected_dtype,
                0,
                row_y,
                header.width,
                1,
            )
            .map_err(|e| wrap_err(format!("波段 {b_idx} 第 {row_y} 行解码回读失败: {e}")))?;
            if bytes.len() != expected_row_bytes {
                return Err(wrap_err(format!(
                    "波段 {b_idx} 第 {row_y} 行回读字节数不完整: {} != {}",
                    bytes.len(),
                    expected_row_bytes
                )));
            }
        }
    }
    Ok(())
}

fn build_redacted_qa_record(
    asset: Option<&OpticalBandAssetInput>,
    credential_fields_redacted: &mut usize,
) -> Option<Value> {
    asset.map(|qa| {
        let (clean_qa, c1) = redact_source_locator(&qa.stable_href);
        *credential_fields_redacted += c1;
        if let Some(resolved) = qa.resolved_href.as_deref() {
            let (_, c2) = redact_source_locator(resolved);
            *credential_fields_redacted += c2;
        }
        json!({
            "band_id": qa.band_id,
            "stable_href": clean_qa,
        })
    })
}

fn sha256_file_hex(path: &Path) -> Result<String, NativeGisError> {
    let mut file = File::open(path).map_err(|e| {
        error(
            NativeGisErrorCode::DemInvalidRaster,
            format!("无法读取输出文件以计算校验和: {e}"),
        )
    })?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).map_err(|e| {
            error(
                NativeGisErrorCode::DemInvalidRaster,
                format!("读取输出文件计算 SHA256 失败: {e}"),
            )
        })?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(&mut out, "{b:02x}");
    }
    Ok(out)
}

fn write_atomic_json(path: &Path, value: &Value) -> Result<(), NativeGisError> {
    let part = part_path(path);
    let encoded = serde_json::to_vec_pretty(value).map_err(|e| {
        error(
            NativeGisErrorCode::DemReprojectionFailed,
            format!("序列化溯源 JSON 失败: {e}"),
        )
    })?;
    let mut file = File::create(&part).map_err(|e| {
        error(
            NativeGisErrorCode::DemReprojectionFailed,
            format!("创建溯源临时文件失败: {e}"),
        )
    })?;
    if let Err(e) = file.write_all(&encoded).and_then(|_| file.sync_all()) {
        let _ = fs::remove_file(&part);
        return Err(error(
            NativeGisErrorCode::DemReprojectionFailed,
            format!("写入溯源 JSON 失败: {e}"),
        ));
    }
    drop(file);
    if path.exists() {
        let _ = fs::remove_file(path);
    }
    fs::rename(&part, path).map_err(|e| {
        let _ = fs::remove_file(&part);
        error(
            NativeGisErrorCode::DemReprojectionFailed,
            format!("原子提交溯源 JSON 失败: {e}"),
        )
    })
}

fn find_scene_band<'a>(
    scene: &'a OpticalSceneInput,
    band_id: &str,
) -> Option<&'a OpticalBandAssetInput> {
    scene
        .band_assets
        .iter()
        .find(|b| b.band_id.eq_ignore_ascii_case(band_id))
}

fn locator_for_execution(asset: &OpticalBandAssetInput) -> &str {
    asset
        .resolved_href
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| asset.stable_href.trim())
}

fn sort_mosaic_scenes(scenes: &[OpticalSceneInput], strategy: OpticalMosaicStrategy) -> Vec<usize> {
    let mut indices: Vec<usize> = (0..scenes.len()).collect();
    indices.sort_by(|&a, &b| {
        let sa = &scenes[a];
        let sb = &scenes[b];
        let ca = sa.cloud_cover_pct.unwrap_or(1000.0);
        let cb = sb.cloud_cover_pct.unwrap_or(1000.0);
        let ta = sa
            .acquired_at
            .as_deref()
            .and_then(parse_iso8601_epoch_seconds)
            .unwrap_or(i64::MIN);
        let tb = sb
            .acquired_at
            .as_deref()
            .and_then(parse_iso8601_epoch_seconds)
            .unwrap_or(i64::MIN);
        match strategy {
            OpticalMosaicStrategy::QualityFirst => ca
                .total_cmp(&cb)
                .then_with(|| tb.cmp(&ta))
                .then_with(|| a.cmp(&b)),
            OpticalMosaicStrategy::LatestFirst => tb
                .cmp(&ta)
                .then_with(|| ca.total_cmp(&cb))
                .then_with(|| a.cmp(&b)),
        }
    });
    indices
}

struct OpenedBand {
    _ds: DatasetGuard,
    band: RasterBand,
    header: RasterHeader,
}

fn open_band_asset(
    runtime: &NativeGisRuntime,
    asset: &OpticalBandAssetInput,
) -> Result<OpenedBand, NativeGisError> {
    let exec_loc = locator_for_execution(asset);
    let (ds, _is_remote) = open_gdal_dataset(runtime, exec_loc)?;
    let header = inspect_open_dataset(runtime, ds.raw())?;
    let band = unsafe {
        let get_band = *runtime.gdal_symbol::<GdalGetRasterBand>(b"GDALGetRasterBand\0")?;
        let b = get_band(ds.raw(), 1);
        if b.is_null() {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                "光学波段缺少 Band 1",
            ));
        }
        b
    };
    Ok(OpenedBand {
        _ds: ds,
        band,
        header,
    })
}

/// Core optical AOI clip (P0) and near-coincident spatial mosaic (P1) execution engine.
pub(crate) fn execute_optical_derived(
    runtime: &NativeGisRuntime,
    request: &OpticalDerivedRequest,
    interrupted: &(dyn Fn() -> bool + Sync),
    progress: &mut dyn FnMut(OpticalDerivedProgress),
) -> Result<OpticalDerivedResult, NativeGisError> {
    execute_optical_derived_with_disk_override(runtime, request, interrupted, progress, None)
}

pub(crate) fn execute_optical_derived_with_disk_override(
    runtime: &NativeGisRuntime,
    request: &OpticalDerivedRequest,
    interrupted: &(dyn Fn() -> bool + Sync),
    progress: &mut dyn FnMut(OpticalDerivedProgress),
    free_bytes_override: Option<u64>,
) -> Result<OpticalDerivedResult, NativeGisError> {
    let aoi_bbox = validate_aoi_bbox(request.aoi_bbox_wgs84)?;
    if request.band_order.is_empty() {
        return Err(error(
            NativeGisErrorCode::AoiGeometryInvalid,
            "光学裁切必须至少指定 1 个输出波段",
        ));
    }
    if request.scenes.is_empty() {
        return Err(error(
            NativeGisErrorCode::AoiGeometryInvalid,
            "光学裁切未提供输入影像景",
        ));
    }
    let is_single_scene_mode = matches!(
        request.mode,
        OpticalDerivedMode::SingleSceneClip | OpticalDerivedMode::BatchSingleSceneClip
    );
    match request.mode {
        OpticalDerivedMode::SingleSceneClip | OpticalDerivedMode::BatchSingleSceneClip
            if request.scenes.len() != 1 =>
        {
            return Err(error(
                NativeGisErrorCode::AoiGeometryInvalid,
                format!(
                    "单景 AOI 裁切只允许 1 景输入，当前提供 {} 景",
                    request.scenes.len()
                ),
            ));
        }
        OpticalDerivedMode::NearCoincidentSpatialMosaic => {
            if !(2..=OPTICAL_MOSAIC_MAX_SCENES).contains(&request.scenes.len()) {
                return Err(error(
                    NativeGisErrorCode::AoiGeometryInvalid,
                    format!(
                        "空间镶嵌要求 2..={} 景输入（当前 {} 景）",
                        OPTICAL_MOSAIC_MAX_SCENES,
                        request.scenes.len()
                    ),
                ));
            }
            if let Some(max_days) = request
                .max_temporal_span_days
                .filter(|d| d.is_finite() && *d >= 0.0)
            {
                let epochs: Vec<i64> = request
                    .scenes
                    .iter()
                    .filter_map(|s| {
                        s.acquired_at
                            .as_deref()
                            .and_then(parse_iso8601_epoch_seconds)
                    })
                    .collect();
                if epochs.len() >= 2 {
                    let span_days = ((*epochs.iter().max().unwrap() - *epochs.iter().min().unwrap())
                        as f64)
                        / 86_400.0;
                    if span_days > max_days {
                        return Err(error(
                            NativeGisErrorCode::AoiGeometryInvalid,
                            format!(
                                "[OPTICAL_TEMPORAL_SPAN_EXCEEDED] 近同步空间镶嵌不允许跨时相长周期合成：当前跨度 {:.1} 天 > 上限 {:.1} 天",
                                span_days, max_days
                            ),
                        ));
                    }
                }
            }
        }
        _ => {}
    }

    // Validate all input band/QA locators upfront before opening GDAL handles
    for scene in &request.scenes {
        for band_id in &request.band_order {
            let asset = find_scene_band(scene, band_id).ok_or_else(|| {
                error(
                    NativeGisErrorCode::DemInvalidRaster,
                    format!("景 {} 缺少所选波段 {}", scene.scene_id, band_id),
                )
            })?;
            validate_raw_optical_cog_source(&asset.stable_href)?;
            if let Some(resolved) = asset.resolved_href.as_deref() {
                validate_raw_optical_cog_source(resolved)?;
            }
        }
        if request.cloud_mask == OpticalCloudMaskPolicy::CloudToNodata {
            for qa in [scene.qa_asset.as_ref(), scene.secondary_qa_asset.as_ref()]
                .into_iter()
                .flatten()
            {
                validate_raw_optical_cog_source(&qa.stable_href)?;
                if let Some(resolved) = qa.resolved_href.as_deref() {
                    validate_raw_optical_cog_source(resolved)?;
                }
            }
        }
    }

    let (provenance_path, source_stac_path, aoi_geojson_path, lineage_source_index_path) =
        sidecar_paths(&request.output_tif_path)?;
    if !request.overwrite
        && (request.output_tif_path.exists()
            || (request.write_provenance_sidecars
                && (provenance_path.exists()
                    || source_stac_path.exists()
                    || aoi_geojson_path.exists())))
    {
        return Err(error(
            NativeGisErrorCode::DemInvalidRaster,
            format!(
                "[OPTICAL_OUTPUT_EXISTS] 目标输出或伴随溯源文件已存在，为防覆盖已拒绝写入: {}",
                request.output_tif_path.display()
            ),
        ));
    }

    let precheck = precheck_aoi_scene_coverage(&OpticalPrecheckRequest {
        aoi_bbox_wgs84: aoi_bbox,
        aoi_rings_wgs84: request.aoi_rings_wgs84.clone(),
        target_resolution_m: request.target_resolution_m,
        mode: Some(request.mode),
        scenes: request.scenes.clone(),
    })?;
    if !precheck.ok {
        return Err(error(
            NativeGisErrorCode::AoiGeometryInvalid,
            precheck.blocking_reasons.join("; "),
        ));
    }

    if interrupted() {
        return Err(error(
            NativeGisErrorCode::DemReprojectionFailed,
            "[OPTICAL_OPERATION_CANCELLED] 用户在预检阶段取消任务",
        ));
    }

    let proj_ctx = runtime.create_proj_context()?;
    let ordered_scene_indices = if is_single_scene_mode {
        vec![0]
    } else {
        sort_mosaic_scenes(&request.scenes, request.mosaic_strategy)
            .into_iter()
            .filter(|&idx| {
                precheck
                    .scene_coverages
                    .get(idx)
                    .is_none_or(|c| c.intersects_aoi)
            })
            .collect()
    };

    // Open reference scene/band(s) to establish native dtype, nodata, and CRS
    let mut ref_opened: Option<(usize, Vec<OpenedBand>)> = None;
    let mut early_failures: Vec<OpticalSceneFailureRecord> = Vec::new();

    for &scene_idx in &ordered_scene_indices {
        let scene = &request.scenes[scene_idx];
        let mut bands_for_scene = Vec::with_capacity(request.band_order.len());
        let mut open_err: Option<NativeGisError> = None;
        for band_id in &request.band_order {
            let asset = find_scene_band(scene, band_id).expect("validated");
            match open_band_asset(runtime, asset) {
                Ok(opened) => bands_for_scene.push(opened),
                Err(e) => {
                    open_err = Some(e);
                    break;
                }
            }
        }
        if let Some(e) = open_err {
            if is_single_scene_mode {
                return Err(e);
            }
            early_failures.push(OpticalSceneFailureRecord {
                scene_id: scene.scene_id.clone(),
                reason: e.to_string(),
            });
            continue;
        }
        ref_opened = Some((scene_idx, bands_for_scene));
        break;
    }

    let (ref_scene_idx, ref_bands) = ref_opened.ok_or_else(|| {
        error(
            NativeGisErrorCode::DemInvalidRaster,
            "所有所选光学景均无法打开，无法构建输出网格",
        )
    })?;

    // Choose finest resolution band among requested bands as geometric anchor
    let anchor_band_pos = ref_bands
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| {
            a.header.geotransform[1]
                .abs()
                .total_cmp(&b.header.geotransform[1].abs())
        })
        .map(|(i, _)| i)
        .unwrap_or(0);
    let anchor_header = ref_bands[anchor_band_pos].header.clone();

    let mut out_dtype = ref_bands[0].header.dtype;
    let out_nodata_source = ref_bands[0].header.nodata;
    for b in &ref_bands[1..] {
        out_dtype = promote_dtype(out_dtype, b.header.dtype);
    }
    let out_nodata = out_nodata_source.unwrap_or_else(|| default_nodata_for_dtype(out_dtype));

    let requested_target_crs = request
        .target_crs
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let want_native_crs =
        matches!(requested_target_crs, None | Some("native")) && is_single_scene_mode;

    let recommended_crs_str = format!("EPSG:{}", precheck.recommended_utm_epsg);
    let target_crs_label = if want_native_crs {
        request.scenes[ref_scene_idx]
            .native_epsg
            .map(|e| format!("EPSG:{e}"))
            .unwrap_or_else(|| "native".to_owned())
    } else if let Some(spec) = requested_target_crs.filter(|&s| s != "native") {
        spec.to_owned()
    } else {
        recommended_crs_str.clone()
    };

    let target_wkt = if want_native_crs {
        anchor_header.wkt.clone()
    } else {
        target_crs_to_wkt(runtime, &proj_ctx, &target_crs_label)?
    };

    let anchor_is_target_crs = want_native_crs
        || crs_grid_equivalent(
            runtime,
            &proj_ctx,
            &anchor_header.wkt,
            &target_wkt,
            [
                anchor_header.geotransform[0],
                anchor_header.geotransform[3]
                    + anchor_header.geotransform[5] * f64::from(anchor_header.height),
                anchor_header.geotransform[0]
                    + anchor_header.geotransform[1] * f64::from(anchor_header.width),
                anchor_header.geotransform[3],
            ],
        );

    let is_rect_only = is_rectangular_bbox_only_check(aoi_bbox, &request.aoi_rings_wgs84);

    // Compute exact target grid & geotransform
    let mut is_native_grid_exact = false;
    let (out_width, out_height, out_gt): (u32, u32, [f64; 6]) = if anchor_is_target_crs
        && is_single_scene_mode
        && request.target_resolution_m.is_none()
        && anchor_header.geotransform[2].abs() < 1e-12
        && anchor_header.geotransform[4].abs() < 1e-12
        && anchor_header.geotransform[1] > 0.0
        && anchor_header.geotransform[5] < 0.0
    {
        // Native grid alignment for bit-for-bit exact window extraction
        let aoi_in_native = transform_points_bbox(
            runtime,
            &proj_ctx,
            "EPSG:4326",
            &anchor_header.wkt,
            aoi_bbox,
            request.aoi_rings_wgs84.as_deref(),
        )
        .ok_or_else(|| {
            error(
                NativeGisErrorCode::DemReprojectionFailed,
                "无法将 AOI 投影到影像原生坐标系",
            )
        })?;
        let gt = anchor_header.geotransform;
        let col_start_f = (aoi_in_native[0] - gt[0]) / gt[1];
        let col_end_f = (aoi_in_native[2] - gt[0]) / gt[1];
        let row_start_f = (aoi_in_native[3] - gt[3]) / gt[5];
        let row_end_f = (aoi_in_native[1] - gt[3]) / gt[5];

        let col_start = col_start_f.floor().max(0.0) as i32;
        let col_end = col_end_f.ceil().min(f64::from(anchor_header.width)) as i32;
        let row_start = row_start_f.floor().max(0.0) as i32;
        let row_end = row_end_f.ceil().min(f64::from(anchor_header.height)) as i32;

        if col_end <= col_start || row_end <= row_start {
            return Err(error(
                NativeGisErrorCode::AoiGeometryInvalid,
                "[AOI_SCENE_DISJOINT] AOI 与光学影像有效栅格区域不相交",
            ));
        }
        is_native_grid_exact = true;
        let clipped_gt = [
            gt[0] + f64::from(col_start) * gt[1],
            gt[1],
            0.0,
            gt[3] + f64::from(row_start) * gt[5],
            0.0,
            gt[5],
        ];
        (
            (col_end - col_start) as u32,
            (row_end - row_start) as u32,
            clipped_gt,
        )
    } else {
        // Reprojected / resampled grid (or near-coincident spatial mosaic)
        let target_bounds = transform_points_bbox(
            runtime,
            &proj_ctx,
            "EPSG:4326",
            &target_wkt,
            aoi_bbox,
            request.aoi_rings_wgs84.as_deref(),
        )
        .ok_or_else(|| {
            error(
                NativeGisErrorCode::DemReprojectionFailed,
                "无法将 AOI 边界转换至目标输出坐标系",
            )
        })?;
        let [tx_min, ty_min, tx_max, ty_max] = target_bounds;
        if tx_max <= tx_min || ty_max <= ty_min {
            return Err(error(
                NativeGisErrorCode::AoiGeometryInvalid,
                "目标坐标系下 AOI 边界退化为空",
            ));
        }
        let target_is_geographic = target_crs_label.eq_ignore_ascii_case("EPSG:4326");
        let (px_w, px_h) = if let Some(res_m) = request.target_resolution_m.filter(|&r| r > 0.0) {
            if target_is_geographic {
                let lat_m = 111_320.0;
                let lon_m = (111_320.0 * precheck.aoi_center_wgs84[1].to_radians().cos().abs())
                    .max(1_000.0);
                (res_m / lon_m, res_m / lat_m)
            } else {
                (res_m, res_m)
            }
        } else if anchor_is_target_crs {
            (
                anchor_header.geotransform[1].abs(),
                anchor_header.geotransform[5].abs(),
            )
        } else {
            // Compute target resolution from anchor pixel spacing
            let src_px_w = anchor_header.geotransform[1].abs();
            let src_px_h = anchor_header.geotransform[5].abs();
            let src_is_geo = src_px_w < 1.0 && src_px_h < 1.0;
            if target_is_geographic && !src_is_geo {
                let lat_m = 111_320.0;
                let lon_m = (111_320.0 * precheck.aoi_center_wgs84[1].to_radians().cos().abs())
                    .max(1_000.0);
                (src_px_w / lon_m, src_px_h / lat_m)
            } else if !target_is_geographic && src_is_geo {
                let lat_m = 111_320.0;
                let lon_m = (111_320.0 * precheck.aoi_center_wgs84[1].to_radians().cos().abs())
                    .max(1_000.0);
                ((src_px_w * lon_m).max(0.5), (src_px_h * lat_m).max(0.5))
            } else {
                (src_px_w, src_px_h)
            }
        };
        let w = ((tx_max - tx_min) / px_w).ceil().max(1.0) as u32;
        let h = ((ty_max - ty_min) / px_h).ceil().max(1.0) as u32;
        let gt = [tx_min, px_w, 0.0, ty_max, 0.0, -px_h];
        (w, h, gt)
    };

    let pixels_per_band = u64::from(out_width).saturating_mul(u64::from(out_height));
    if pixels_per_band > OPTICAL_OUTPUT_PIXEL_LIMIT {
        return Err(error(
            NativeGisErrorCode::AoiGeometryInvalid,
            format!(
                "[OPTICAL_OUTPUT_PIXEL_LIMIT] 输出栅格单波段像元数 ({out_width}x{out_height} = {pixels_per_band}) 超过安全上限 {OPTICAL_OUTPUT_PIXEL_LIMIT}"
            ),
        ));
    }

    let band_count = request.band_order.len();
    let est_main_bytes = pixels_per_band
        .saturating_mul(band_count as u64)
        .saturating_mul(bytes_per_sample(out_dtype));
    let est_lineage_bytes = if request.mode == OpticalDerivedMode::NearCoincidentSpatialMosaic
        && request.write_lineage_sidecar
    {
        pixels_per_band.saturating_mul(2)
    } else {
        0
    };
    let est_total_bytes = est_main_bytes.saturating_add(est_lineage_bytes);
    let _ = check_disk_space_budget(
        &request.output_tif_path,
        est_total_bytes,
        free_bytes_override,
    )?;

    let use_bigtiff = est_main_bytes > OPTICAL_BIGTIFF_THRESHOLD_BYTES;
    let out_tif_part = part_path(&request.output_tif_path);
    let write_lineage = request.mode == OpticalDerivedMode::NearCoincidentSpatialMosaic
        && request.write_lineage_sidecar;
    let lineage_part = write_lineage.then(|| part_path(&lineage_source_index_path));

    // Close ref_bands first only after collecting scenes cleanly, or keep ref scene's bands reused
    // Actually we open/prepare all usable scenes for strip processing
    struct PreparedScene {
        scene_index: usize,
        bands: Vec<OpenedBand>,
        qa: Option<OpenedBand>,
        qa_band_id: String,
        secondary_qa: Option<OpenedBand>,
        secondary_qa_band_id: String,
        band_native_exact: Vec<bool>,
    }

    let mut prepared_scenes: Vec<PreparedScene> = Vec::with_capacity(ordered_scene_indices.len());
    let mut ref_bands_opt = Some(ref_bands);
    let mut scenes_failed = early_failures;

    for &scene_idx in &ordered_scene_indices {
        if scenes_failed
            .iter()
            .any(|f| f.scene_id == request.scenes[scene_idx].scene_id)
        {
            continue;
        }
        let scene = &request.scenes[scene_idx];
        let bands_for_scene = if scene_idx == ref_scene_idx {
            ref_bands_opt.take().expect("taken once")
        } else {
            let mut opened = Vec::with_capacity(band_count);
            let mut open_err = None;
            for band_id in &request.band_order {
                let asset = find_scene_band(scene, band_id).expect("validated");
                match open_band_asset(runtime, asset) {
                    Ok(ob) => opened.push(ob),
                    Err(e) => {
                        open_err = Some(e);
                        break;
                    }
                }
            }
            if let Some(e) = open_err {
                if is_single_scene_mode {
                    return Err(e);
                }
                scenes_failed.push(OpticalSceneFailureRecord {
                    scene_id: scene.scene_id.clone(),
                    reason: e.to_string(),
                });
                continue;
            }
            opened
        };

        let open_optional_qa = |asset_opt: Option<&OpticalBandAssetInput>,
                                default_id: &str|
         -> Result<Option<(OpenedBand, String)>, NativeGisError> {
            if request.cloud_mask != OpticalCloudMaskPolicy::CloudToNodata {
                return Ok(None);
            }
            let Some(qa_asset) = asset_opt else {
                return Ok(None);
            };
            let band_id = if qa_asset.band_id.trim().is_empty() {
                default_id.to_owned()
            } else {
                qa_asset.band_id.trim().to_owned()
            };
            open_band_asset(runtime, qa_asset).map(|ob| Some((ob, band_id)))
        };

        let (qa, qa_band_id) = match open_optional_qa(
            scene.qa_asset.as_ref(),
            default_qa_band_id(scene.source_kind),
        ) {
            Ok(Some((ob, id))) => (Some(ob), id),
            Ok(None) => (None, String::new()),
            Err(e) => {
                if is_single_scene_mode {
                    return Err(e);
                }
                scenes_failed.push(OpticalSceneFailureRecord {
                    scene_id: scene.scene_id.clone(),
                    reason: format!("云掩膜波段不可用: {e}"),
                });
                continue;
            }
        };

        let (secondary_qa, secondary_qa_band_id) =
            match open_optional_qa(scene.secondary_qa_asset.as_ref(), "QA_RADSAT") {
                Ok(Some((ob, id))) => (Some(ob), id),
                Ok(None) => (None, String::new()),
                Err(e) => {
                    if is_single_scene_mode {
                        return Err(e);
                    }
                    scenes_failed.push(OpticalSceneFailureRecord {
                        scene_id: scene.scene_id.clone(),
                        reason: format!("辅助质量/饱和掩膜波段不可用: {e}"),
                    });
                    continue;
                }
            };

        let band_native_exact: Vec<bool> = bands_for_scene
            .iter()
            .map(|b| {
                is_native_grid_exact
                    && b.header.dtype == out_dtype
                    && b.header.geotransform[2].abs() < 1e-12
                    && b.header.geotransform[4].abs() < 1e-12
                    && (b.header.geotransform[1] - out_gt[1]).abs() < 1e-9
                    && (b.header.geotransform[5] - out_gt[5]).abs() < 1e-9
                    && crs_grid_equivalent(
                        runtime,
                        &proj_ctx,
                        &b.header.wkt,
                        &target_wkt,
                        [
                            out_gt[0],
                            out_gt[3] + out_gt[5] * f64::from(out_height as i32),
                            out_gt[0] + out_gt[1] * f64::from(out_width as i32),
                            out_gt[3],
                        ],
                    )
                    && ((out_gt[0] - b.header.geotransform[0]) / b.header.geotransform[1])
                        .fract()
                        .abs()
                        .min(
                            1.0 - ((out_gt[0] - b.header.geotransform[0])
                                / b.header.geotransform[1])
                                .fract()
                                .abs(),
                        )
                        < 1e-4
                    && ((out_gt[3] - b.header.geotransform[3]) / b.header.geotransform[5])
                        .fract()
                        .abs()
                        .min(
                            1.0 - ((out_gt[3] - b.header.geotransform[3])
                                / b.header.geotransform[5])
                                .fract()
                                .abs(),
                        )
                        < 1e-4
            })
            .collect();

        prepared_scenes.push(PreparedScene {
            scene_index: scene_idx,
            bands: bands_for_scene,
            qa,
            qa_band_id,
            secondary_qa,
            secondary_qa_band_id,
            band_native_exact,
        });
    }

    if prepared_scenes.is_empty() {
        return Err(error(
            NativeGisErrorCode::DemInvalidRaster,
            "没有可用光学景可执行裁切/镶嵌",
        ));
    }

    let cleanup_parts = || {
        let _ = fs::remove_file(&out_tif_part);
        if let Some(lp) = &lineage_part {
            let _ = fs::remove_file(lp);
        }
    };

    let out_ds = match create_output_geotiff(
        runtime,
        OutputGeoTiffSpec {
            path: &out_tif_part,
            width: out_width as i32,
            height: out_height as i32,
            band_count: band_count as i32,
            dtype: out_dtype,
            crs_wkt: &target_wkt,
            geotransform: &out_gt,
            nodata: Some(out_nodata),
            band_names: &request.band_order,
            use_bigtiff,
        },
    ) {
        Ok(ds) => ds,
        Err(e) => {
            cleanup_parts();
            return Err(e);
        }
    };

    let lineage_ds = if let Some(lp) = &lineage_part {
        match create_output_geotiff(
            runtime,
            OutputGeoTiffSpec {
                path: lp,
                width: out_width as i32,
                height: out_height as i32,
                band_count: 1,
                dtype: GDAL_UINT16,
                crs_wkt: &target_wkt,
                geotransform: &out_gt,
                nodata: Some(0.0),
                band_names: &["LINEAGE_SOURCE_INDEX".to_owned()],
                use_bigtiff: false,
            },
        ) {
            Ok(ds) => Some(ds),
            Err(e) => {
                drop(out_ds);
                cleanup_parts();
                return Err(e);
            }
        }
    } else {
        None
    };

    let strip_total = (out_height as usize).div_ceil(STRIP_ROWS).max(1);
    let to_wgs84 = if is_rect_only {
        None
    } else {
        ProjTransformGuard::from_crs(runtime, &proj_ctx, &target_wkt, "EPSG:4326").ok()
    };
    let projected_aoi_edges =
        if is_rect_only || out_gt[1] <= 0.0 || out_gt[2].abs() >= 1e-12 || out_gt[4].abs() >= 1e-12
        {
            None
        } else {
            request.aoi_rings_wgs84.as_deref().and_then(|rings| {
                build_projected_polygon_edges(runtime, &proj_ctx, &target_wkt, rings)
            })
        };

    // 1-based source index table aligned with provenance sources_used
    let mut scene_idx_to_lineage_id: Vec<u16> = vec![0; request.scenes.len()];
    for (pos, ps) in prepared_scenes.iter().enumerate() {
        scene_idx_to_lineage_id[ps.scene_index] = (pos + 1) as u16;
    }
    let mut scene_contributed_pixels = vec![0u64; request.scenes.len()];
    let mut total_valid_output_pixels = 0u64;
    let total_output_pixels = pixels_per_band;

    let run_strips = (|| -> Result<(), NativeGisError> {
        let get_band =
            unsafe { *runtime.gdal_symbol::<GdalGetRasterBand>(b"GDALGetRasterBand\0")? };

        // Pure fast path for SingleSceneClip with ALL bands native exact + no QA + rect-only
        let single_native_fast = is_single_scene_mode
            && prepared_scenes[0].band_native_exact.iter().all(|&x| x)
            && prepared_scenes[0].qa.is_none()
            && prepared_scenes[0].secondary_qa.is_none()
            && is_rect_only;

        for strip_idx in 0..strip_total {
            if interrupted() {
                return Err(error(
                    NativeGisErrorCode::DemReprojectionFailed,
                    "[OPTICAL_OPERATION_CANCELLED] 用户中止光学影像处理",
                ));
            }
            let y0 = strip_idx * STRIP_ROWS;
            let strip_h = (out_height as usize - y0).min(STRIP_ROWS);
            let strip_pixels = strip_h * (out_width as usize);

            progress(OpticalDerivedProgress {
                stage: if is_single_scene_mode {
                    "clip".to_owned()
                } else {
                    "mosaic".to_owned()
                },
                scene_index: 0,
                scene_total: prepared_scenes.len(),
                strip_index: strip_idx + 1,
                strip_total,
                bytes_written_estimate: ((y0 + strip_h) as u64)
                    .saturating_mul(u64::from(out_width))
                    .saturating_mul(band_count as u64)
                    .saturating_mul(bytes_per_sample(out_dtype)),
            });

            if single_native_fast {
                let ps = &prepared_scenes[0];
                let mut valid_in_band0 = 0usize;
                for b_pos in 0..band_count {
                    let ob = &ps.bands[b_pos];
                    let col_off = ((out_gt[0] - ob.header.geotransform[0])
                        / ob.header.geotransform[1])
                        .round() as i32;
                    let row_off = ((out_gt[3] - ob.header.geotransform[3])
                        / ob.header.geotransform[5])
                        .round() as i32;
                    let mut raw = read_band_window_native_bytes(
                        runtime,
                        ob.band,
                        out_dtype,
                        col_off,
                        row_off + y0 as i32,
                        out_width as i32,
                        strip_h as i32,
                    )?;
                    if b_pos == 0 {
                        valid_in_band0 =
                            count_valid_in_native_bytes(&raw, out_dtype, ob.header.nodata);
                    }
                    let dst_band = unsafe { get_band(out_ds.raw(), (b_pos + 1) as c_int) };
                    write_band_window_native_bytes(
                        runtime,
                        dst_band,
                        out_dtype,
                        PixelWindow {
                            x_off: 0,
                            y_off: y0 as i32,
                            w: out_width as i32,
                            h: strip_h as i32,
                        },
                        &mut raw,
                    )?;
                }
                scene_contributed_pixels[ps.scene_index] += valid_in_band0 as u64;
                total_valid_output_pixels += valid_in_band0 as u64;
                continue;
            }

            // Compute AOI polygon mask for this strip
            let mut inside_aoi = vec![true; strip_pixels];
            if !is_rect_only {
                if let Some(edges) = projected_aoi_edges.as_deref() {
                    rasterize_projected_polygon_strip(
                        edges,
                        &out_gt,
                        y0,
                        out_width as usize,
                        strip_h,
                        &mut inside_aoi,
                    );
                } else {
                    let rings = request.aoi_rings_wgs84.as_deref().unwrap_or(&[]);
                    if let Some(tf_geo) = to_wgs84.as_ref() {
                        for local_r in 0..strip_h {
                            let out_y = out_gt[3] + ((y0 + local_r) as f64 + 0.5) * out_gt[5];
                            for col in 0..(out_width as usize) {
                                let out_x = out_gt[0] + (col as f64 + 0.5) * out_gt[1];
                                let ok = tf_geo
                                    .transform(runtime, out_x, out_y)
                                    .is_some_and(|(lon, lat)| point_in_rings(lon, lat, rings));
                                inside_aoi[local_r * (out_width as usize) + col] = ok;
                            }
                        }
                    }
                }
            }

            // Fast native bit-exact path for single-scene polygon clip on native grid without QA resampling
            if is_single_scene_mode && prepared_scenes[0].band_native_exact.iter().all(|&x| x) {
                let ps = &prepared_scenes[0];
                let source_kind = request.scenes[ps.scene_index].source_kind;
                let mut clear_mask = inside_aoi.clone();
                for (qa_opt, qa_band_id) in [
                    (ps.qa.as_ref(), ps.qa_band_id.as_str()),
                    (ps.secondary_qa.as_ref(), ps.secondary_qa_band_id.as_str()),
                ] {
                    if let Some(qa_ob) = qa_opt {
                        apply_qa_mask_for_strip(
                            runtime,
                            &proj_ctx,
                            QaMaskStripSpec {
                                qa_ob,
                                source_kind,
                                qa_band_id,
                                target_wkt: &target_wkt,
                                out_gt: &out_gt,
                                y0,
                                out_w: out_width as usize,
                                strip_h,
                            },
                            &mut clear_mask,
                        )?;
                    }
                }
                let mut valid_in_band0 = 0usize;
                for b_pos in 0..band_count {
                    let ob = &ps.bands[b_pos];
                    let col_off = ((out_gt[0] - ob.header.geotransform[0])
                        / ob.header.geotransform[1])
                        .round() as i32;
                    let row_off = ((out_gt[3] - ob.header.geotransform[3])
                        / ob.header.geotransform[5])
                        .round() as i32;
                    let mut raw = read_band_window_native_bytes(
                        runtime,
                        ob.band,
                        out_dtype,
                        col_off,
                        row_off + y0 as i32,
                        out_width as i32,
                        strip_h as i32,
                    )?;
                    mask_native_bytes_outside_valid(&mut raw, out_dtype, out_nodata, &clear_mask);
                    if b_pos == 0 {
                        valid_in_band0 =
                            count_valid_in_native_bytes(&raw, out_dtype, Some(out_nodata));
                    }
                    let dst_band = unsafe { get_band(out_ds.raw(), (b_pos + 1) as c_int) };
                    write_band_window_native_bytes(
                        runtime,
                        dst_band,
                        out_dtype,
                        PixelWindow {
                            x_off: 0,
                            y_off: y0 as i32,
                            w: out_width as i32,
                            h: strip_h as i32,
                        },
                        &mut raw,
                    )?;
                }
                scene_contributed_pixels[ps.scene_index] += valid_in_band0 as u64;
                total_valid_output_pixels += valid_in_band0 as u64;
                continue;
            }

            // General reprojection / multi-resolution / mosaic path in 512-row strips
            let mut accum_bands = vec![vec![out_nodata; strip_pixels]; band_count];
            let mut accum_valid = vec![false; strip_pixels];
            let mut accum_lineage = vec![0u16; strip_pixels];
            let mut remaining = strip_pixels;

            for ps in &prepared_scenes {
                if remaining == 0 {
                    break;
                }
                let scene = &request.scenes[ps.scene_index];
                let mut qa_mask = inside_aoi.clone();
                let mut qa_failed = None;
                for (qa_opt, qa_band_id) in [
                    (ps.qa.as_ref(), ps.qa_band_id.as_str()),
                    (ps.secondary_qa.as_ref(), ps.secondary_qa_band_id.as_str()),
                ] {
                    if let Some(qa_ob) = qa_opt {
                        if let Err(e) = apply_qa_mask_for_strip(
                            runtime,
                            &proj_ctx,
                            QaMaskStripSpec {
                                qa_ob,
                                source_kind: scene.source_kind,
                                qa_band_id,
                                target_wkt: &target_wkt,
                                out_gt: &out_gt,
                                y0,
                                out_w: out_width as usize,
                                strip_h,
                            },
                            &mut qa_mask,
                        ) {
                            qa_failed = Some(e);
                            break;
                        }
                    }
                }
                if let Some(e) = qa_failed {
                    if is_single_scene_mode {
                        return Err(e);
                    }
                    if !scenes_failed.iter().any(|f| f.scene_id == scene.scene_id) {
                        scenes_failed.push(OpticalSceneFailureRecord {
                            scene_id: scene.scene_id.clone(),
                            reason: format!("云/饱和掩膜波段读取失败: {e}"),
                        });
                    }
                    continue;
                }

                // Sample all requested bands for pixels not yet filled and passing QA + AOI
                let mut scene_band_vals: Vec<Vec<(f64, bool)>> = Vec::with_capacity(band_count);
                let mut band_failed = None;
                for (b_pos, ob) in ps.bands.iter().enumerate() {
                    let res = sample_band_strip(
                        runtime,
                        &proj_ctx,
                        ob,
                        &target_wkt,
                        &out_gt,
                        y0,
                        out_width as usize,
                        strip_h,
                        if ps.band_native_exact[b_pos] {
                            OpticalResampling::Nearest
                        } else {
                            request.resampling
                        },
                        &accum_valid,
                        &qa_mask,
                    );
                    match res {
                        Ok(vals) => scene_band_vals.push(vals),
                        Err(e) => {
                            band_failed = Some(e);
                            break;
                        }
                    }
                }
                if let Some(e) = band_failed {
                    if is_single_scene_mode {
                        return Err(e);
                    }
                    if !scenes_failed.iter().any(|f| f.scene_id == scene.scene_id) {
                        scenes_failed.push(OpticalSceneFailureRecord {
                            scene_id: scene.scene_id.clone(),
                            reason: e.to_string(),
                        });
                    }
                    continue;
                }

                let lineage_id = scene_idx_to_lineage_id[ps.scene_index];
                for px in 0..strip_pixels {
                    if accum_valid[px] || !qa_mask[px] {
                        continue;
                    }
                    let all_bands_valid = (0..band_count).all(|b| scene_band_vals[b][px].1);
                    if !all_bands_valid {
                        continue;
                    }
                    for b in 0..band_count {
                        accum_bands[b][px] = scene_band_vals[b][px].0;
                    }
                    accum_valid[px] = true;
                    accum_lineage[px] = lineage_id;
                    scene_contributed_pixels[ps.scene_index] += 1;
                    total_valid_output_pixels += 1;
                    remaining = remaining.saturating_sub(1);
                }
            }

            let strip_win = PixelWindow {
                x_off: 0,
                y_off: y0 as i32,
                w: out_width as i32,
                h: strip_h as i32,
            };
            for (b_pos, band_accum) in accum_bands.iter().enumerate().take(band_count) {
                let dst_band = unsafe { get_band(out_ds.raw(), (b_pos + 1) as c_int) };
                write_f64_strip_to_band(
                    runtime,
                    dst_band,
                    StripWrite {
                        dtype: out_dtype,
                        nodata_val: out_nodata,
                        win: strip_win,
                        values: band_accum,
                        valid: &accum_valid,
                    },
                )?;
            }
            if let Some(lds) = lineage_ds.as_ref() {
                let lband = unsafe { get_band(lds.raw(), 1) };
                let lvals: Vec<f64> = accum_lineage.iter().map(|&v| f64::from(v)).collect();
                write_f64_strip_to_band(
                    runtime,
                    lband,
                    StripWrite {
                        dtype: GDAL_UINT16,
                        nodata_val: 0.0,
                        win: strip_win,
                        values: &lvals,
                        valid: &accum_valid,
                    },
                )?;
            }
        }
        Ok(())
    })();

    if let Err(e) = run_strips {
        drop(lineage_ds);
        drop(out_ds);
        cleanup_parts();
        return Err(e);
    }

    let scenes_succeeded: Vec<String> = prepared_scenes
        .iter()
        .filter(|ps| scene_contributed_pixels[ps.scene_index] > 0)
        .map(|ps| request.scenes[ps.scene_index].scene_id.clone())
        .collect();

    if total_valid_output_pixels == 0 || scenes_succeeded.is_empty() {
        drop(lineage_ds);
        drop(out_ds);
        cleanup_parts();
        return Err(error(
            NativeGisErrorCode::AoiGeometryInvalid,
            "[AOI_SCENE_DISJOINT] 所有像元均超出光学景有效范围或被云掩膜遮蔽，未生成任何有效观测像元",
        ));
    }

    let finalize_and_verify = (|| -> Result<(), NativeGisError> {
        flush_and_finalize_dataset(runtime, out_ds, out_width as i32, out_height as i32)?;
        if let Some(lds) = lineage_ds {
            flush_and_finalize_dataset(runtime, lds, out_width as i32, out_height as i32)?;
        }
        verify_written_geotiff_readback(
            runtime,
            ReadbackVerifySpec {
                path: &out_tif_part,
                expected_width: out_width as i32,
                expected_height: out_height as i32,
                expected_band_count: band_count as i32,
                expected_dtype: out_dtype,
                expected_geotransform: &out_gt,
                expected_nodata: Some(out_nodata),
            },
        )?;
        if let Some(lp) = &lineage_part {
            verify_written_geotiff_readback(
                runtime,
                ReadbackVerifySpec {
                    path: lp,
                    expected_width: out_width as i32,
                    expected_height: out_height as i32,
                    expected_band_count: 1,
                    expected_dtype: GDAL_UINT16,
                    expected_geotransform: &out_gt,
                    expected_nodata: Some(0.0),
                },
            )?;
        }
        Ok(())
    })();
    if let Err(e) = finalize_and_verify {
        cleanup_parts();
        return Err(e);
    }

    let fallback_pixels = if request.mode == OpticalDerivedMode::NearCoincidentSpatialMosaic {
        let primary_pixels = prepared_scenes
            .first()
            .map_or(0, |ps| scene_contributed_pixels[ps.scene_index]);
        total_valid_output_pixels.saturating_sub(primary_pixels)
    } else {
        0
    };

    let output_sha256 = sha256_file_hex(&out_tif_part)?;
    let bytes_written = fs::metadata(&out_tif_part).map(|m| m.len()).unwrap_or(0);

    // Build redacted provenance + source STAC + AOI GeoJSON
    let mut credential_fields_redacted = 0usize;
    let mut sources_used = Vec::new();
    let mut stac_features = Vec::new();

    for ps in &prepared_scenes {
        let scene = &request.scenes[ps.scene_index];
        let contributed = scene_contributed_pixels[ps.scene_index];
        if contributed == 0 && !scenes_failed.iter().any(|f| f.scene_id == scene.scene_id) {
            continue;
        }
        let mut band_records = Vec::new();
        for band_id in &request.band_order {
            let asset = find_scene_band(scene, band_id).expect("validated");
            let (clean_stable, c1) = redact_source_locator(&asset.stable_href);
            credential_fields_redacted += c1;
            if let Some(resolved) = asset.resolved_href.as_deref() {
                let (_, c2) = redact_source_locator(resolved);
                credential_fields_redacted += c2;
            }
            band_records.push(json!({
                "band_id": asset.band_id,
                "stable_href": clean_stable,
                "band_resolution_m": asset.band_resolution_m,
            }));
        }
        let qa_record =
            build_redacted_qa_record(scene.qa_asset.as_ref(), &mut credential_fields_redacted);
        let secondary_qa_record = build_redacted_qa_record(
            scene.secondary_qa_asset.as_ref(),
            &mut credential_fields_redacted,
        );
        sources_used.push(json!({
            "lineage_source_index": scene_idx_to_lineage_id[ps.scene_index],
            "scene_id": scene.scene_id,
            "source_kind": scene.source_kind,
            "collection": scene.collection,
            "acquired_at": scene.acquired_at,
            "cloud_cover_pct": scene.cloud_cover_pct,
            "native_epsg": scene.native_epsg,
            "contributed_pixels": contributed,
            "bands": band_records,
            "qa_asset": qa_record,
            "secondary_qa_asset": secondary_qa_record,
        }));

        let redacted_stac = scene
            .stac_metadata
            .clone()
            .map(|v| redact_json_value(v, &mut credential_fields_redacted))
            .unwrap_or_else(|| {
                json!({
                    "id": scene.scene_id,
                    "collection": scene.collection,
                    "properties": {
                        "datetime": scene.acquired_at,
                        "eo:cloud_cover": scene.cloud_cover_pct,
                    },
                    "bbox": scene.footprint_bbox_wgs84,
                })
            });
        stac_features.push(redacted_stac);
    }

    let provenance_json = json!({
        "schema_version": 1,
        "product": "EOStage",
        "operation": match request.mode {
            OpticalDerivedMode::SingleSceneClip | OpticalDerivedMode::BatchSingleSceneClip => {
                "optical_single_scene_aoi_clip_v1"
            }
            OpticalDerivedMode::NearCoincidentSpatialMosaic => "optical_near_coincident_spatial_mosaic_v1",
        },
        "scientific_policy": {
            "raw_single_scene_cog_only": true,
            "synthetic_compositing_or_hole_filling": false,
            "cloud_mask_policy": request.cloud_mask,
            "mosaic_strategy": (request.mode == OpticalDerivedMode::NearCoincidentSpatialMosaic).then_some(request.mosaic_strategy),
            "max_temporal_span_days": (request.mode == OpticalDerivedMode::NearCoincidentSpatialMosaic).then_some(request.max_temporal_span_days).flatten(),
            "is_native_grid_exact": is_native_grid_exact,
        },
        "output": {
            "file_name": request.output_tif_path.file_name().and_then(|s| s.to_str()),
            "sha256": output_sha256,
            "bytes_written": bytes_written,
            "width": out_width,
            "height": out_height,
            "band_count": band_count,
            "band_order": request.band_order,
            "dtype": gdal_dtype_name(out_dtype),
            "nodata": out_nodata,
            "crs": target_crs_label,
            "geotransform": out_gt,
            "is_bigtiff": use_bigtiff,
            "valid_pixel_fraction": (total_valid_output_pixels as f64) / (total_output_pixels.max(1) as f64),
            "fallback_pixels": fallback_pixels,
            "readback_verified": true,
            "lineage_source_index_file": lineage_part.as_ref().and_then(|_| lineage_source_index_path.file_name().and_then(|s| s.to_str())),
        },
        "aoi": {
            "bbox_wgs84": aoi_bbox,
            "coverage_fraction": precheck.combined_aoi_coverage_fraction,
            "aoi_geojson_file": aoi_geojson_path.file_name().and_then(|s| s.to_str()),
        },
        "sources_used": sources_used,
        "scenes_failed": scenes_failed,
        "credential_fields_redacted": credential_fields_redacted,
    });

    let source_stac_json = json!({
        "type": "FeatureCollection",
        "stac_version": "1.0.0",
        "features": stac_features,
    });

    let aoi_geometry_json =
        if let Some(rings) = request.aoi_rings_wgs84.as_ref().filter(|r| !r.is_empty()) {
            let closed_rings: Vec<Vec<[f64; 2]>> = rings
                .iter()
                .filter(|r| r.len() >= 3)
                .map(|r| {
                    let mut closed = r.clone();
                    if closed.first() != closed.last() {
                        closed.push(closed[0]);
                    }
                    closed
                })
                .collect();
            json!({
                "type": "Polygon",
                "coordinates": closed_rings,
            })
        } else {
            let [w, s, e, n] = aoi_bbox;
            json!({
                "type": "Polygon",
                "coordinates": [[[w, s], [e, s], [e, n], [w, n], [w, s]]],
            })
        };
    let aoi_geojson = json!({
        "type": "FeatureCollection",
        "features": [{
            "type": "Feature",
            "properties": {
                "bbox_wgs84": aoi_bbox,
            },
            "geometry": aoi_geometry_json,
        }]
    });

    progress(OpticalDerivedProgress {
        stage: "provenance".to_owned(),
        scene_index: prepared_scenes.len(),
        scene_total: prepared_scenes.len(),
        strip_index: strip_total,
        strip_total,
        bytes_written_estimate: bytes_written,
    });

    if request.write_provenance_sidecars {
        if let Err(e) = write_atomic_json(&provenance_path, &provenance_json)
            .and_then(|_| write_atomic_json(&source_stac_path, &source_stac_json))
            .and_then(|_| write_atomic_json(&aoi_geojson_path, &aoi_geojson))
        {
            cleanup_parts();
            return Err(e);
        }
    }

    if let Some(lp) = &lineage_part {
        if lineage_source_index_path.exists() {
            let _ = fs::remove_file(&lineage_source_index_path);
        }
        if let Err(e) = fs::rename(lp, &lineage_source_index_path) {
            cleanup_parts();
            return Err(error(
                NativeGisErrorCode::DemReprojectionFailed,
                format!("原子提交镶嵌来源索引文件失败: {e}"),
            ));
        }
    }

    if request.output_tif_path.exists() {
        let _ = fs::remove_file(&request.output_tif_path);
    }
    if let Err(e) = fs::rename(&out_tif_part, &request.output_tif_path) {
        cleanup_parts();
        return Err(error(
            NativeGisErrorCode::DemReprojectionFailed,
            format!("原子提交输出 GeoTIFF 失败: {e}"),
        ));
    }

    Ok(OpticalDerivedResult {
        ok: true,
        mode: request.mode,
        output_tif_path: request.output_tif_path.clone(),
        provenance_path,
        source_stac_path,
        aoi_geojson_path,
        lineage_source_index_path: write_lineage.then_some(lineage_source_index_path),
        output_sha256,
        bytes_written,
        width: out_width,
        height: out_height,
        band_count,
        dtype: gdal_dtype_name(out_dtype).to_owned(),
        nodata: Some(out_nodata),
        crs: target_crs_label,
        geotransform: out_gt,
        is_bigtiff: use_bigtiff,
        aoi_coverage_fraction: precheck.combined_aoi_coverage_fraction,
        valid_pixel_fraction: (total_valid_output_pixels as f64)
            / (total_output_pixels.max(1) as f64),
        scenes_succeeded,
        scenes_failed,
        credential_fields_redacted,
        is_native_grid_exact,
        fallback_pixels,
        readback_verified: true,
    })
}

fn default_qa_band_id(source_kind: OpticalSourceKind) -> &'static str {
    match source_kind {
        OpticalSourceKind::Sentinel2L2a => "SCL",
        OpticalSourceKind::LandsatC2L2 => "QA_PIXEL",
        OpticalSourceKind::HlsL30 | OpticalSourceKind::HlsS30 => "Fmask",
        OpticalSourceKind::Sentinel2L1c | OpticalSourceKind::OpticalRawCog => "QA",
    }
}

fn is_rectangular_bbox_only_check(bbox: [f64; 4], rings: &Option<Vec<Vec<[f64; 2]>>>) -> bool {
    is_rectangular_rings(bbox, rings)
}

struct QaMaskStripSpec<'a> {
    qa_ob: &'a OpenedBand,
    source_kind: OpticalSourceKind,
    qa_band_id: &'a str,
    target_wkt: &'a str,
    out_gt: &'a [f64; 6],
    y0: usize,
    out_w: usize,
    strip_h: usize,
}

fn apply_qa_mask_for_strip(
    runtime: &NativeGisRuntime,
    ctx: &ProjContextGuard,
    spec: QaMaskStripSpec<'_>,
    valid_mask: &mut [bool],
) -> Result<(), NativeGisError> {
    let QaMaskStripSpec {
        qa_ob,
        source_kind,
        qa_band_id,
        target_wkt,
        out_gt,
        y0,
        out_w,
        strip_h,
    } = spec;
    let tf = ProjTransformGuard::from_crs(runtime, ctx, target_wkt, &qa_ob.header.wkt)?;
    let (win_x0, win_y0, win_w, win_h) = match compute_strip_source_window(
        runtime,
        &tf,
        &qa_ob.header,
        out_gt,
        y0,
        out_w,
        strip_h,
        1,
    ) {
        Some(w) => w,
        None => {
            valid_mask.fill(false);
            return Ok(());
        }
    };
    let qa_vals = read_band_window_f64(runtime, qa_ob.band, win_x0, win_y0, win_w, win_h)?;
    let band_id = if qa_band_id.trim().is_empty() {
        default_qa_band_id(source_kind)
    } else {
        qa_band_id.trim()
    };

    for local_r in 0..strip_h {
        let out_y = out_gt[3] + ((y0 + local_r) as f64 + 0.5) * out_gt[5];
        for col in 0..out_w {
            let idx = local_r * out_w + col;
            if !valid_mask[idx] {
                continue;
            }
            let out_x = out_gt[0] + (col as f64 + 0.5) * out_gt[1];
            let Some((sx, sy)) = tf.transform(runtime, out_x, out_y) else {
                valid_mask[idx] = false;
                continue;
            };
            let Some((sc, sr)) = world_to_col_row(&qa_ob.header.geotransform, sx, sy) else {
                valid_mask[idx] = false;
                continue;
            };
            if let Some(val) = sample_window_f64(
                F64Window {
                    buf: &qa_vals,
                    x0: win_x0,
                    y0: win_y0,
                    w: win_w,
                    h: win_h,
                },
                sc,
                sr,
                OpticalResampling::Nearest,
                None,
            ) {
                if qa_pixel_is_cloudy(source_kind, band_id, val) {
                    valid_mask[idx] = false;
                }
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn sample_band_strip(
    runtime: &NativeGisRuntime,
    ctx: &ProjContextGuard,
    ob: &OpenedBand,
    target_wkt: &str,
    out_gt: &[f64; 6],
    y0: usize,
    out_w: usize,
    strip_h: usize,
    resampling: OpticalResampling,
    skip_mask: &[bool],
    allow_mask: &[bool],
) -> Result<Vec<(f64, bool)>, NativeGisError> {
    let strip_pixels = out_w * strip_h;
    let mut out = vec![(0.0, false); strip_pixels];
    let tf = ProjTransformGuard::from_crs(runtime, ctx, target_wkt, &ob.header.wkt)?;
    let pad = match resampling {
        OpticalResampling::Nearest => 1,
        OpticalResampling::Bilinear => 2,
        OpticalResampling::Cubic => 3,
    };
    let Some((win_x0, win_y0, win_w, win_h)) =
        compute_strip_source_window(runtime, &tf, &ob.header, out_gt, y0, out_w, strip_h, pad)
    else {
        return Ok(out);
    };

    let buf = read_band_window_f64(runtime, ob.band, win_x0, win_y0, win_w, win_h)?;
    for local_r in 0..strip_h {
        let out_y = out_gt[3] + ((y0 + local_r) as f64 + 0.5) * out_gt[5];
        for col in 0..out_w {
            let idx = local_r * out_w + col;
            if skip_mask[idx] || !allow_mask[idx] {
                continue;
            }
            let out_x = out_gt[0] + (col as f64 + 0.5) * out_gt[1];
            let Some((sx, sy)) = tf.transform(runtime, out_x, out_y) else {
                continue;
            };
            let Some((sc, sr)) = world_to_col_row(&ob.header.geotransform, sx, sy) else {
                continue;
            };
            if let Some(val) = sample_window_f64(
                F64Window {
                    buf: &buf,
                    x0: win_x0,
                    y0: win_y0,
                    w: win_w,
                    h: win_h,
                },
                sc,
                sr,
                resampling,
                ob.header.nodata,
            ) {
                out[idx] = (val, true);
            }
        }
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn compute_strip_source_window(
    runtime: &NativeGisRuntime,
    tf: &ProjTransformGuard,
    header: &RasterHeader,
    out_gt: &[f64; 6],
    y0: usize,
    out_w: usize,
    strip_h: usize,
    pad_pixels: i32,
) -> Option<(i32, i32, i32, i32)> {
    let min_ox = out_gt[0];
    let max_ox = out_gt[0] + (out_w as f64) * out_gt[1];
    let max_oy = out_gt[3] + (y0 as f64) * out_gt[5];
    let min_oy = out_gt[3] + ((y0 + strip_h) as f64) * out_gt[5];

    let mut min_c = f64::INFINITY;
    let mut max_c = f64::NEG_INFINITY;
    let mut min_r = f64::INFINITY;
    let mut max_r = f64::NEG_INFINITY;

    let steps = 4usize;
    for i in 0..=steps {
        let fx = (i as f64) / (steps as f64);
        let ox = min_ox + (max_ox - min_ox) * fx;
        for j in 0..=steps {
            let fy = (j as f64) / (steps as f64);
            let oy = min_oy + (max_oy - min_oy) * fy;
            if let Some((sx, sy)) = tf.transform(runtime, ox, oy) {
                if let Some((c, r)) = world_to_col_row(&header.geotransform, sx, sy) {
                    min_c = min_c.min(c);
                    max_c = max_c.max(c);
                    min_r = min_r.min(r);
                    max_r = max_r.max(r);
                }
            }
        }
    }
    if !min_c.is_finite() || !min_r.is_finite() {
        return None;
    }

    let c0 = (min_c.floor() as i32 - pad_pixels).max(0);
    let r0 = (min_r.floor() as i32 - pad_pixels).max(0);
    let c1 = (max_c.ceil() as i32 + pad_pixels).min(header.width);
    let r1 = (max_r.ceil() as i32 + pad_pixels).min(header.height);
    if c1 <= c0 || r1 <= r0 {
        return None;
    }
    Some((c0, r0, c1 - c0, r1 - r0))
}

#[doc(hidden)]
pub type TestGeoTiffReadU16 = (u32, u32, usize, Option<f64>, Vec<u16>);

#[doc(hidden)]
pub(crate) fn write_test_geotiff_bytes_with_runtime(
    runtime: &NativeGisRuntime,
    path: &Path,
    epsg: u32,
    geotransform: [f64; 6],
    size: [i32; 2],
    dtype_and_nodata: (c_int, Option<f64>),
    raw_bytes: &[u8],
) -> Result<(), NativeGisError> {
    let [width, height] = size;
    let (dtype, nodata) = dtype_and_nodata;
    let proj_ctx = runtime.create_proj_context()?;
    let wkt = target_crs_to_wkt(runtime, &proj_ctx, &format!("EPSG:{epsg}"))?;
    let ds = create_output_geotiff(
        runtime,
        OutputGeoTiffSpec {
            path,
            width,
            height,
            band_count: 1,
            dtype,
            crs_wkt: &wkt,
            geotransform: &geotransform,
            nodata,
            band_names: &["TEST".to_owned()],
            use_bigtiff: false,
        },
    )?;
    let mut writable = raw_bytes.to_vec();
    unsafe {
        let get_band = *runtime.gdal_symbol::<GdalGetRasterBand>(b"GDALGetRasterBand\0")?;
        let band = get_band(ds.raw(), 1);
        write_band_window_native_bytes(
            runtime,
            band,
            dtype,
            PixelWindow {
                x_off: 0,
                y_off: 0,
                w: width,
                h: height,
            },
            &mut writable,
        )?;
    }
    flush_and_finalize_dataset(runtime, ds, width, height)
}

#[doc(hidden)]
pub(crate) fn write_test_geotiff_u8_with_runtime(
    runtime: &NativeGisRuntime,
    path: &Path,
    epsg: u32,
    geotransform: [f64; 6],
    size: [i32; 2],
    nodata: Option<f64>,
    data: &[u8],
) -> Result<(), NativeGisError> {
    write_test_geotiff_bytes_with_runtime(
        runtime,
        path,
        epsg,
        geotransform,
        size,
        (GDAL_BYTE, nodata),
        data,
    )
}

#[doc(hidden)]
pub(crate) fn read_test_geotiff_u16_with_runtime(
    runtime: &NativeGisRuntime,
    path: &Path,
    band_index: i32,
) -> Result<TestGeoTiffReadU16, NativeGisError> {
    let (ds, _) = open_gdal_dataset(runtime, &path.to_string_lossy())?;
    let header = inspect_open_dataset(runtime, ds.raw())?;
    let (band_count, vals) = unsafe {
        let get_cnt = *runtime.gdal_symbol::<GdalGetRasterCount>(b"GDALGetRasterCount\0")?;
        let band_count = get_cnt(ds.raw());
        let get_band = *runtime.gdal_symbol::<GdalGetRasterBand>(b"GDALGetRasterBand\0")?;
        let band = get_band(ds.raw(), band_index);
        let bytes = read_band_window_native_bytes(
            runtime,
            band,
            GDAL_UINT16,
            0,
            0,
            header.width,
            header.height,
        )?;
        let vals = bytes
            .chunks_exact(2)
            .map(|c| u16::from_ne_bytes([c[0], c[1]]))
            .collect::<Vec<_>>();
        (band_count, vals)
    };
    Ok((
        u32::try_from(header.width).unwrap_or(0),
        u32::try_from(header.height).unwrap_or(0),
        usize::try_from(band_count).unwrap_or(0),
        header.nodata,
        vals,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "insar-gis-optical-{name}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    fn write_test_geotiff_u16(
        runtime: &NativeGisRuntime,
        path: &Path,
        epsg: u32,
        geotransform: [f64; 6],
        size: [i32; 2],
        nodata: Option<f64>,
        data: &[u16],
    ) {
        let raw_bytes: Vec<u8> = data.iter().flat_map(|v| v.to_ne_bytes()).collect();
        write_test_geotiff_bytes_with_runtime(
            runtime,
            path,
            epsg,
            geotransform,
            size,
            (GDAL_UINT16, nodata),
            &raw_bytes,
        )
        .expect("write test tif");
    }

    fn read_test_geotiff_u16(
        runtime: &NativeGisRuntime,
        path: &Path,
        band_index: i32,
    ) -> (RasterHeader, Vec<u16>) {
        let (ds, _) = open_gdal_dataset(runtime, &path.to_string_lossy()).expect("open");
        let header = inspect_open_dataset(runtime, ds.raw()).expect("inspect");
        let (_, _, _, _, vals) =
            read_test_geotiff_u16_with_runtime(runtime, path, band_index).expect("read");
        (header, vals)
    }

    fn require_runtime_for_optical_test() -> NativeGisRuntime {
        use crate::runtime::NativeGisRuntimeIdentity;
        let mut candidates = Vec::new();
        for env_key in ["INSAR_GIS_RUNTIME_ROOT", "INSAR_NATIVE_GIS_RUNTIME_ROOT"] {
            if let Some(root) = std::env::var_os(env_key) {
                candidates.push((env_key, PathBuf::from(root)));
            }
        }
        for rel in [
            "../../runtime/gis",
            "../../../runtime/gis",
            "../../../../runtime/gis",
        ] {
            candidates.push(("relative", Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)));
        }
        for (source, candidate) in candidates {
            if candidate.join("manifest.json").is_file() {
                let identity = NativeGisRuntimeIdentity::from_product_root(&candidate)
                    .unwrap_or_else(|e| {
                        panic!(
                            "invalid GIS runtime manifest at {} ({source}): {e}",
                            candidate.display()
                        )
                    });
                return NativeGisRuntime::load_identity(&identity).unwrap_or_else(|e| {
                    panic!(
                        "failed to load GIS runtime at {} ({source}): {e}",
                        candidate.display()
                    )
                });
            }
        }
        panic!("INSAR_GIS_RUNTIME_ROOT product GIS runtime must be available for real-GDAL optical clip tests");
    }

    #[test]
    fn raw_optical_source_guard_rejects_xyz_wmts_gee_and_rendered_images() {
        let dir = test_temp_dir("source-guard");
        let cog_path = dir.join("raw_b04.tif");
        fs::write(&cog_path, b"dummy").expect("dummy");

        assert!(validate_raw_optical_cog_source(
            "https://planetarycomputer.microsoft.com/api/data/v1/item.tif"
        )
        .is_ok());
        assert!(validate_raw_optical_cog_source("/vsicurl/https://example.com/S2_B04.tif").is_ok());
        assert!(validate_raw_optical_cog_source(&cog_path.to_string_lossy()).is_ok());

        for bad in [
            "https://tiles.example.com/{z}/{x}/{y}.png",
            "https://wmts.example.com/service?REQUEST=GetTile&TileMatrixSet=GoogleMapsCompatible",
            "https://earthengine.googleapis.com/v1/projects/foo/maps/bar/tiles/0/0/0",
            "https://planetarycomputer.microsoft.com/api/data/v1/item/rendered_preview.png",
            "https://example.com/thumb.webp",
            "https://example.com/preview.jpg",
        ] {
            let err = validate_raw_optical_cog_source(bad).expect_err("must reject");
            assert!(err.to_string().contains("OPTICAL_SOURCE_REJECTED"), "{err}");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn redacts_sas_and_bearer_secrets_completely() {
        let sas = "/vsicurl/https://sentinel2l2a01.blob.core.windows.net/sentinel2-l2/tile.tif?st=2026-01-01T00%3A00%3A00Z&se=2026-01-02T00%3A00%3A00Z&sp=rl&sv=2021-06-08&sr=c&sig=SECRET_SIG_12345&foo=bar";
        let (clean, count) = redact_source_locator(sas);
        assert_eq!(count, 6);
        assert_eq!(
            clean,
            "/vsicurl/https://sentinel2l2a01.blob.core.windows.net/sentinel2-l2/tile.tif?foo=bar"
        );
        assert!(!clean.contains("sig="));
        assert!(!clean.contains("SECRET_SIG"));

        let userinfo = "https://user:pass123@example.com/scene.tif?access_token=tok999";
        let (clean2, count2) = redact_source_locator(userinfo);
        assert_eq!(count2, 2);
        assert_eq!(clean2, "https://example.com/scene.tif");
    }

    #[test]
    fn precheck_128x128_coverage_and_cross_utm_are_deterministic() {
        // AOI centered around (117.0, 31.8) in EPSG:32650 spanning UTM boundary 120.0 -> 50 & 51
        let req = OpticalPrecheckRequest {
            aoi_bbox_wgs84: [119.5, 31.5, 120.5, 32.0],
            aoi_rings_wgs84: None,
            target_resolution_m: None,
            mode: None,
            scenes: vec![
                OpticalSceneInput {
                    scene_id: "S2A_WEST".to_owned(),
                    source_kind: OpticalSourceKind::Sentinel2L2a,
                    collection: "sentinel-2-l2a".to_owned(),
                    acquired_at: Some("2026-06-10T02:55:00Z".to_owned()),
                    cloud_cover_pct: Some(12.0),
                    footprint_bbox_wgs84: Some([119.0, 31.0, 120.0, 32.5]),
                    footprint_rings_wgs84: None,
                    native_epsg: Some(32650),
                    band_assets: vec![],
                    qa_asset: None,
                    secondary_qa_asset: None,
                    stac_metadata: None,
                },
                OpticalSceneInput {
                    scene_id: "S2B_EAST".to_owned(),
                    source_kind: OpticalSourceKind::Sentinel2L2a,
                    collection: "sentinel-2-l2a".to_owned(),
                    acquired_at: Some("2026-06-12T02:55:00Z".to_owned()),
                    cloud_cover_pct: Some(5.0),
                    footprint_bbox_wgs84: Some([120.0, 31.0, 121.0, 32.5]),
                    footprint_rings_wgs84: None,
                    native_epsg: Some(32651),
                    band_assets: vec![],
                    qa_asset: None,
                    secondary_qa_asset: None,
                    stac_metadata: None,
                },
            ],
        };
        let r1 = precheck_aoi_scene_coverage(&req).expect("precheck");
        let r2 = precheck_aoi_scene_coverage(&req).expect("deterministic");
        assert_eq!(r1, r2);
        assert!(r1.ok);
        assert!(r1.crosses_utm_zones);
        assert_eq!(r1.distinct_utm_zones, vec![50, 51]);
        assert!((r1.combined_aoi_coverage_fraction - 1.0).abs() < 1e-6);
        assert!((r1.temporal_span_days.unwrap() - 2.0).abs() < 1e-6);
        assert!(r1.within_near_coincident_limit);

        // Reject mosaic > 16 days when mode is unspecified
        let mut stale = req.clone();
        stale.scenes[1].acquired_at = Some("2026-07-10T02:55:00Z".to_owned());
        let r_stale = precheck_aoi_scene_coverage(&stale).expect("precheck");
        assert!(!r_stale.ok);
        assert!(!r_stale.within_near_coincident_limit);

        // Explicit mode precheck treats >16d mosaic span as non-blocking warning and supports batch per-scene clip
        let mut stale_mosaic = stale.clone();
        stale_mosaic.mode = Some(OpticalDerivedMode::NearCoincidentSpatialMosaic);
        let r_mosaic_warn =
            precheck_aoi_scene_coverage(&stale_mosaic).expect("precheck mosaic warn");
        assert!(
            r_mosaic_warn.ok,
            "explicit mosaic mode >16d should warn rather than block"
        );
        assert!(!r_mosaic_warn.within_near_coincident_limit);
        assert!(r_mosaic_warn.warnings.iter().any(|w| w.contains("16 天")));

        let mut stale_batch = stale.clone();
        stale_batch.mode = Some(OpticalDerivedMode::BatchSingleSceneClip);
        let r_batch = precheck_aoi_scene_coverage(&stale_batch).expect("precheck batch");
        assert!(r_batch.ok);
        assert_eq!(
            r_batch.recommended_mode,
            OpticalDerivedMode::BatchSingleSceneClip
        );

        // Explicit target_resolution_m (20m vs default 10m) scales estimated output pixels by ~1/4
        let mut req_20m = req.clone();
        req_20m.target_resolution_m = Some(20.0);
        let r_20m = precheck_aoi_scene_coverage(&req_20m).expect("precheck 20m");
        assert!(
            r_20m.estimated_output_pixels_per_band * 3 <= r1.estimated_output_pixels_per_band,
            "20m resolution should yield roughly 1/4 pixels of 10m ({} vs {})",
            r_20m.estimated_output_pixels_per_band,
            r1.estimated_output_pixels_per_band
        );
    }

    #[test]
    #[ignore = "requires optional native GIS runtime (run scripts/run_native_gis_tests.sh with INSAR_GIS_RUNTIME_ROOT)"]
    fn single_scene_native_clip_preserves_uint16_bit_exact_and_writes_redacted_provenance() {
        let runtime = require_runtime_for_optical_test();
        let dir = test_temp_dir("native-clip");

        // Build 32x32 UTM 50N (EPSG:32650) COG at 10m resolution around Hefei ~ (520000, 3520000)
        let gt = [520_000.0, 10.0, 0.0, 3_520_320.0, 0.0, -10.0];
        let mut b04 = vec![0u16; 32 * 32];
        let mut b03 = vec![0u16; 32 * 32];
        for r in 0..32 {
            for c in 0..32 {
                b04[r * 32 + c] = 1000 + (r as u16) * 50 + (c as u16);
                b03[r * 32 + c] = 2000 + (r as u16) * 30 + (c as u16);
            }
        }
        let b04_path = dir.join("S2_B04.tif");
        let b03_path = dir.join("S2_B03.tif");
        write_test_geotiff_u16(&runtime, &b04_path, 32650, gt, [32, 32], Some(0.0), &b04);
        write_test_geotiff_u16(&runtime, &b03_path, 32650, gt, [32, 32], Some(0.0), &b03);

        // Compute WGS84 bbox covering center pixels cols [8..24], rows [8..24] -> world [520080..520240, 3520080..3520240]
        let ctx = runtime.create_proj_context().expect("ctx");
        let wgs = transform_points_bbox(
            &runtime,
            &ctx,
            "EPSG:32650",
            "EPSG:4326",
            [520_081.0, 3_520_081.0, 520_239.0, 3_520_239.0],
            None,
        )
        .expect("wgs84 bbox");

        let out_tif = dir.join("clip_out.tif");
        let result = execute_optical_derived(
            &runtime,
            &OpticalDerivedRequest {
                mode: OpticalDerivedMode::SingleSceneClip,
                aoi_bbox_wgs84: wgs,
                aoi_rings_wgs84: None,
                band_order: vec!["B04".to_owned(), "B03".to_owned()],
                target_crs: None,
                target_resolution_m: None,
                resampling: OpticalResampling::Bilinear,
                cloud_mask: OpticalCloudMaskPolicy::None,
                mosaic_strategy: OpticalMosaicStrategy::QualityFirst,
                max_temporal_span_days: None,
                overwrite: false,
                scenes: vec![OpticalSceneInput {
                    scene_id: "S2A_HEFEI_01".to_owned(),
                    source_kind: OpticalSourceKind::Sentinel2L2a,
                    collection: "sentinel-2-l2a".to_owned(),
                    acquired_at: Some("2026-06-15T02:55:11Z".to_owned()),
                    cloud_cover_pct: Some(2.5),
                    footprint_bbox_wgs84: Some([wgs[0] - 0.05, wgs[1] - 0.05, wgs[2] + 0.05, wgs[3] + 0.05]),
                    footprint_rings_wgs84: None,
                    native_epsg: Some(32650),
                    band_assets: vec![
                        OpticalBandAssetInput {
                            band_id: "B04".to_owned(),
                            stable_href: "https://example.com/S2_B04.tif?sig=LEAK_FORBIDDEN&se=2026-11-15".to_owned(),
                            resolved_href: Some(b04_path.to_string_lossy().into_owned()),
                            band_resolution_m: Some(10.0),
                            role: Some("data".to_owned()),
                        },
                        OpticalBandAssetInput {
                            band_id: "B03".to_owned(),
                            stable_href: b03_path.to_string_lossy().into_owned(),
                            resolved_href: None,
                            band_resolution_m: Some(10.0),
                            role: Some("data".to_owned()),
                        },
                    ],
                    qa_asset: None,
                    secondary_qa_asset: None,
                    stac_metadata: Some(json!({
                        "id": "S2A_HEFEI_01",
                        "assets": {
                            "B04": { "href": "https://example.com/S2_B04.tif?sv=2021&sig=SECRET_XYZ" }
                        }
                    })),
                }],
                output_tif_path: out_tif.clone(),
                write_lineage_sidecar: false,
                write_provenance_sidecars: true,
            },
            &|| false,
            &mut |_| {},
        )
        .expect("single scene clip");

        assert!(result.is_native_grid_exact);
        assert!(result.readback_verified);
        assert_eq!(result.fallback_pixels, 0);
        assert_eq!(result.dtype, "UInt16");
        assert_eq!(result.band_count, 2);
        assert!(result.provenance_path.exists());
        assert!(result.source_stac_path.exists());
        assert!(result.aoi_geojson_path.exists());
        assert!(result.credential_fields_redacted >= 4);

        // Verify bit-for-bit pixel equality against original raster window
        let (hdr_out, vals_b04) = read_test_geotiff_u16(&runtime, &out_tif, 1);
        let (_, vals_b03) = read_test_geotiff_u16(&runtime, &out_tif, 2);
        let col0 = ((hdr_out.geotransform[0] - gt[0]) / gt[1]).round() as usize;
        let row0 = ((hdr_out.geotransform[3] - gt[3]) / gt[5]).round() as usize;
        for r in 0..(hdr_out.height as usize) {
            for c in 0..(hdr_out.width as usize) {
                let expected_b04 = b04[(row0 + r) * 32 + (col0 + c)];
                let expected_b03 = b03[(row0 + r) * 32 + (col0 + c)];
                assert_eq!(vals_b04[r * (hdr_out.width as usize) + c], expected_b04);
                assert_eq!(vals_b03[r * (hdr_out.width as usize) + c], expected_b03);
            }
        }

        // Verify zero SAS/secret leak in provenance and source stac JSON
        let prov_raw = fs::read_to_string(&result.provenance_path).expect("prov");
        let stac_raw = fs::read_to_string(&result.source_stac_path).expect("stac");
        assert!(!prov_raw.contains("LEAK_FORBIDDEN"));
        assert!(!prov_raw.contains("sig="));
        assert!(!stac_raw.contains("SECRET_XYZ"));
        assert!(!stac_raw.contains("sig="));

        // Verify overwrite refusal by default
        let err_overwrite = execute_optical_derived(
            &runtime,
            &OpticalDerivedRequest {
                mode: OpticalDerivedMode::SingleSceneClip,
                aoi_bbox_wgs84: wgs,
                aoi_rings_wgs84: None,
                band_order: vec!["B04".to_owned()],
                target_crs: None,
                target_resolution_m: None,
                resampling: OpticalResampling::Nearest,
                cloud_mask: OpticalCloudMaskPolicy::None,
                mosaic_strategy: OpticalMosaicStrategy::QualityFirst,
                max_temporal_span_days: None,
                overwrite: false,
                scenes: vec![OpticalSceneInput {
                    scene_id: "S2A_HEFEI_01".to_owned(),
                    source_kind: OpticalSourceKind::Sentinel2L2a,
                    collection: "sentinel-2-l2a".to_owned(),
                    acquired_at: None,
                    cloud_cover_pct: None,
                    footprint_bbox_wgs84: Some(wgs),
                    footprint_rings_wgs84: None,
                    native_epsg: Some(32650),
                    band_assets: vec![OpticalBandAssetInput {
                        band_id: "B04".to_owned(),
                        stable_href: b04_path.to_string_lossy().into_owned(),
                        resolved_href: None,
                        band_resolution_m: Some(10.0),
                        role: None,
                    }],
                    qa_asset: None,
                    secondary_qa_asset: None,
                    stac_metadata: None,
                }],
                output_tif_path: out_tif,
                write_lineage_sidecar: false,
                write_provenance_sidecars: true,
            },
            &|| false,
            &mut |_| {},
        )
        .expect_err("must refuse overwrite");
        assert!(err_overwrite.to_string().contains("OPTICAL_OUTPUT_EXISTS"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore = "requires optional native GIS runtime (run scripts/run_native_gis_tests.sh with INSAR_GIS_RUNTIME_ROOT)"]
    fn cloud_mask_sets_only_cloudy_pixels_to_nodata_without_altering_clear_reflectance() {
        let runtime = require_runtime_for_optical_test();
        let dir = test_temp_dir("cloud-mask");

        let gt = [520_000.0, 10.0, 0.0, 3_520_160.0, 0.0, -10.0];
        let mut b04 = vec![0u16; 16 * 16];
        let mut scl = vec![4u16; 16 * 16]; // class 4 = vegetation (clear)
        for (i, px) in b04.iter_mut().enumerate().take(256) {
            *px = 1500 + (i as u16) * 3;
        }
        // Mark columns 0..4 as cloud high probability (SCL = 9) and row 15 as shadow (SCL = 3)
        for r in 0..16 {
            for c in 0..16 {
                if c < 4 || r == 15 {
                    scl[r * 16 + c] = if c < 4 { 9 } else { 3 };
                }
            }
        }
        let b04_path = dir.join("S2_B04.tif");
        let scl_path = dir.join("S2_SCL.tif");
        write_test_geotiff_u16(&runtime, &b04_path, 32650, gt, [16, 16], Some(0.0), &b04);
        write_test_geotiff_u16(&runtime, &scl_path, 32650, gt, [16, 16], None, &scl);

        let ctx = runtime.create_proj_context().expect("ctx");
        let wgs = transform_points_bbox(
            &runtime,
            &ctx,
            "EPSG:32650",
            "EPSG:4326",
            [520_001.0, 3_520_001.0, 520_159.0, 3_520_159.0],
            None,
        )
        .expect("wgs");

        let out_tif = dir.join("cloud_masked.tif");
        execute_optical_derived(
            &runtime,
            &OpticalDerivedRequest {
                mode: OpticalDerivedMode::SingleSceneClip,
                aoi_bbox_wgs84: wgs,
                aoi_rings_wgs84: None,
                band_order: vec!["B04".to_owned()],
                target_crs: None,
                target_resolution_m: None,
                resampling: OpticalResampling::Nearest,
                cloud_mask: OpticalCloudMaskPolicy::CloudToNodata,
                mosaic_strategy: OpticalMosaicStrategy::QualityFirst,
                max_temporal_span_days: None,
                overwrite: false,
                scenes: vec![OpticalSceneInput {
                    scene_id: "S2_CLOUDY".to_owned(),
                    source_kind: OpticalSourceKind::Sentinel2L2a,
                    collection: "sentinel-2-l2a".to_owned(),
                    acquired_at: Some("2026-06-15".to_owned()),
                    cloud_cover_pct: Some(25.0),
                    footprint_bbox_wgs84: Some(wgs),
                    footprint_rings_wgs84: None,
                    native_epsg: Some(32650),
                    band_assets: vec![OpticalBandAssetInput {
                        band_id: "B04".to_owned(),
                        stable_href: b04_path.to_string_lossy().into_owned(),
                        resolved_href: None,
                        band_resolution_m: Some(10.0),
                        role: None,
                    }],
                    qa_asset: Some(OpticalBandAssetInput {
                        band_id: "SCL".to_owned(),
                        stable_href: scl_path.to_string_lossy().into_owned(),
                        resolved_href: None,
                        band_resolution_m: Some(10.0),
                        role: Some("scl".to_owned()),
                    }),
                    secondary_qa_asset: None,
                    stac_metadata: None,
                }],
                output_tif_path: out_tif.clone(),
                write_lineage_sidecar: false,
                write_provenance_sidecars: true,
            },
            &|| false,
            &mut |_| {},
        )
        .expect("cloud masked clip");

        let (hdr, out_vals) = read_test_geotiff_u16(&runtime, &out_tif, 1);
        assert_eq!(hdr.width, 16);
        assert_eq!(hdr.height, 16);
        for r in 0..16 {
            for c in 0..16 {
                let got = out_vals[r * 16 + c];
                if c < 4 || r == 15 {
                    assert_eq!(got, 0, "cloudy pixel ({c},{r}) must become NoData=0");
                } else {
                    assert_eq!(
                        got,
                        b04[r * 16 + c],
                        "clear pixel ({c},{r}) must remain unaltered"
                    );
                }
            }
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore = "requires optional native GIS runtime (run scripts/run_native_gis_tests.sh with INSAR_GIS_RUNTIME_ROOT)"]
    fn spatial_mosaic_selects_clear_quality_pixel_writes_lineage_and_tolerates_single_scene_failure(
    ) {
        let runtime = require_runtime_for_optical_test();
        let dir = test_temp_dir("mosaic");

        // Create two overlapping 16x16 scenes in EPSG:32650 + one missing scene
        // Scene A (cloud 5%): cols 0..8 clear (val 1100), cols 8..16 cloudy (SCL=9)
        // Scene B (cloud 20%): cols 0..16 clear (val 2200)
        // Scene C: broken/missing path (tolerated because >= 1 scene succeeds!)
        let gt = [520_000.0, 10.0, 0.0, 3_520_160.0, 0.0, -10.0];
        let a_b04_data = vec![1100u16; 16 * 16];
        let mut a_scl_data = vec![4u16; 16 * 16];
        for r in 0..16 {
            for c in 8..16 {
                a_scl_data[r * 16 + c] = 9;
            }
        }
        let b_b04_data = vec![2200u16; 16 * 16];
        let b_scl_data = vec![4u16; 16 * 16];

        let a_b04 = dir.join("A_B04.tif");
        let a_scl = dir.join("A_SCL.tif");
        let b_b04 = dir.join("B_B04.tif");
        let b_scl = dir.join("B_SCL.tif");
        write_test_geotiff_u16(
            &runtime,
            &a_b04,
            32650,
            gt,
            [16, 16],
            Some(0.0),
            &a_b04_data,
        );
        write_test_geotiff_u16(&runtime, &a_scl, 32650, gt, [16, 16], None, &a_scl_data);
        write_test_geotiff_u16(
            &runtime,
            &b_b04,
            32650,
            gt,
            [16, 16],
            Some(0.0),
            &b_b04_data,
        );
        write_test_geotiff_u16(&runtime, &b_scl, 32650, gt, [16, 16], None, &b_scl_data);

        let ctx = runtime.create_proj_context().expect("ctx");
        let wgs = transform_points_bbox(
            &runtime,
            &ctx,
            "EPSG:32650",
            "EPSG:4326",
            [520_001.0, 3_520_001.0, 520_159.0, 3_520_159.0],
            None,
        )
        .expect("wgs");

        let out_tif = dir.join("mosaic_out.tif");
        let res = execute_optical_derived(
            &runtime,
            &OpticalDerivedRequest {
                mode: OpticalDerivedMode::NearCoincidentSpatialMosaic,
                aoi_bbox_wgs84: wgs,
                aoi_rings_wgs84: None,
                band_order: vec!["B04".to_owned()],
                target_crs: Some("EPSG:32650".to_owned()),
                target_resolution_m: Some(10.0),
                resampling: OpticalResampling::Nearest,
                cloud_mask: OpticalCloudMaskPolicy::CloudToNodata,
                mosaic_strategy: OpticalMosaicStrategy::QualityFirst,
                max_temporal_span_days: Some(16.0),
                overwrite: false,
                scenes: vec![
                    OpticalSceneInput {
                        scene_id: "SCENE_B".to_owned(),
                        source_kind: OpticalSourceKind::Sentinel2L2a,
                        collection: "sentinel-2-l2a".to_owned(),
                        acquired_at: Some("2026-06-14".to_owned()),
                        cloud_cover_pct: Some(20.0),
                        footprint_bbox_wgs84: Some(wgs),
                        footprint_rings_wgs84: None,
                        native_epsg: Some(32650),
                        band_assets: vec![OpticalBandAssetInput {
                            band_id: "B04".to_owned(),
                            stable_href: b_b04.to_string_lossy().into_owned(),
                            resolved_href: None,
                            band_resolution_m: Some(10.0),
                            role: None,
                        }],
                        qa_asset: Some(OpticalBandAssetInput {
                            band_id: "SCL".to_owned(),
                            stable_href: b_scl.to_string_lossy().into_owned(),
                            resolved_href: None,
                            band_resolution_m: Some(10.0),
                            role: Some("scl".to_owned()),
                        }),
                        secondary_qa_asset: None,
                        stac_metadata: None,
                    },
                    OpticalSceneInput {
                        scene_id: "SCENE_A_LOW_CLOUD".to_owned(),
                        source_kind: OpticalSourceKind::Sentinel2L2a,
                        collection: "sentinel-2-l2a".to_owned(),
                        acquired_at: Some("2026-06-10".to_owned()),
                        cloud_cover_pct: Some(5.0),
                        footprint_bbox_wgs84: Some(wgs),
                        footprint_rings_wgs84: None,
                        native_epsg: Some(32650),
                        band_assets: vec![OpticalBandAssetInput {
                            band_id: "B04".to_owned(),
                            stable_href: a_b04.to_string_lossy().into_owned(),
                            resolved_href: None,
                            band_resolution_m: Some(10.0),
                            role: None,
                        }],
                        qa_asset: Some(OpticalBandAssetInput {
                            band_id: "SCL".to_owned(),
                            stable_href: a_scl.to_string_lossy().into_owned(),
                            resolved_href: None,
                            band_resolution_m: Some(10.0),
                            role: Some("scl".to_owned()),
                        }),
                        secondary_qa_asset: None,
                        stac_metadata: None,
                    },
                    OpticalSceneInput {
                        scene_id: "SCENE_BROKEN".to_owned(),
                        source_kind: OpticalSourceKind::Sentinel2L2a,
                        collection: "sentinel-2-l2a".to_owned(),
                        acquired_at: Some("2026-06-12".to_owned()),
                        cloud_cover_pct: Some(90.0),
                        footprint_bbox_wgs84: Some(wgs),
                        footprint_rings_wgs84: None,
                        native_epsg: Some(32650),
                        band_assets: vec![OpticalBandAssetInput {
                            band_id: "B04".to_owned(),
                            stable_href: "https://invalid.example.invalid/not_present_404.tif"
                                .to_owned(),
                            resolved_href: None,
                            band_resolution_m: Some(10.0),
                            role: None,
                        }],
                        qa_asset: None,
                        secondary_qa_asset: None,
                        stac_metadata: None,
                    },
                ],
                output_tif_path: out_tif.clone(),
                write_lineage_sidecar: true,
                write_provenance_sidecars: true,
            },
            &|| false,
            &mut |_| {},
        )
        .expect("mosaic succeeds despite broken 3rd scene");

        assert!(res.readback_verified);
        assert_eq!(res.fallback_pixels, 128);
        assert_eq!(res.scenes_succeeded.len(), 2);
        assert_eq!(res.scenes_failed.len(), 1);
        assert_eq!(res.scenes_failed[0].scene_id, "SCENE_BROKEN");
        let lineage_path = res.lineage_source_index_path.expect("lineage path");
        assert!(lineage_path.exists());

        let (hdr, out_vals) = read_test_geotiff_u16(&runtime, &out_tif, 1);
        let (_, lineage_vals) = read_test_geotiff_u16(&runtime, &lineage_path, 1);
        let w = hdr.width as usize;
        for r in 0..(hdr.height as usize) {
            for c in 0..w {
                let x_world = hdr.geotransform[0] + (c as f64 + 0.5) * hdr.geotransform[1];
                let val = out_vals[r * w + c];
                let lid = lineage_vals[r * w + c];
                if x_world < 520_080.0 {
                    // Clear in Scene A (index 1 in sorted quality_first list)
                    assert_eq!(val, 1100);
                    assert_eq!(lid, 1);
                } else {
                    // Cloudy in Scene A -> falls back to Scene B clear pixel (index 2)
                    assert_eq!(val, 2200);
                    assert_eq!(lid, 2);
                }
            }
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore = "requires optional native GIS runtime (run scripts/run_native_gis_tests.sh with INSAR_GIS_RUNTIME_ROOT)"]
    fn guardrails_enforce_pixel_limit_disk_budget_and_cancellation_cleanup() {
        let runtime = require_runtime_for_optical_test();
        let dir = test_temp_dir("guardrails");
        let gt = [520_000.0, 10.0, 0.0, 3_520_160.0, 0.0, -10.0];
        let b04_path = dir.join("S2_B04.tif");
        write_test_geotiff_u16(
            &runtime,
            &b04_path,
            32650,
            gt,
            [16, 16],
            Some(0.0),
            &vec![1234u16; 256],
        );

        let ctx = runtime.create_proj_context().expect("ctx");
        let wgs = transform_points_bbox(
            &runtime,
            &ctx,
            "EPSG:32650",
            "EPSG:4326",
            [520_001.0, 3_520_001.0, 520_159.0, 3_520_159.0],
            None,
        )
        .expect("wgs");

        // 1. Disk space exhaustion check
        let err_disk = check_disk_space_budget(&dir.join("out.tif"), 100 * 1024 * 1024, Some(1024))
            .expect_err("must fail low disk");
        assert!(err_disk
            .to_string()
            .contains("OPTICAL_DISK_SPACE_INSUFFICIENT"));

        // 2. Output pixel limit > 400_000_000 (e.g. 0.005m resolution over 158m -> ~9.98e8 px)
        let out_tif = dir.join("cancelled.tif");
        let req_huge = OpticalDerivedRequest {
            mode: OpticalDerivedMode::SingleSceneClip,
            aoi_bbox_wgs84: wgs,
            aoi_rings_wgs84: None,
            band_order: vec!["B04".to_owned()],
            target_crs: Some("EPSG:32650".to_owned()),
            target_resolution_m: Some(0.005),
            resampling: OpticalResampling::Nearest,
            cloud_mask: OpticalCloudMaskPolicy::None,
            mosaic_strategy: OpticalMosaicStrategy::QualityFirst,
            max_temporal_span_days: None,
            overwrite: false,
            scenes: vec![OpticalSceneInput {
                scene_id: "S2_HUGE".to_owned(),
                source_kind: OpticalSourceKind::Sentinel2L2a,
                collection: "sentinel-2-l2a".to_owned(),
                acquired_at: None,
                cloud_cover_pct: None,
                footprint_bbox_wgs84: Some(wgs),
                footprint_rings_wgs84: None,
                native_epsg: Some(32650),
                band_assets: vec![OpticalBandAssetInput {
                    band_id: "B04".to_owned(),
                    stable_href: b04_path.to_string_lossy().into_owned(),
                    resolved_href: None,
                    band_resolution_m: Some(10.0),
                    role: None,
                }],
                qa_asset: None,
                secondary_qa_asset: None,
                stac_metadata: None,
            }],
            output_tif_path: out_tif.clone(),
            write_lineage_sidecar: false,
            write_provenance_sidecars: true,
        };
        let err_px = execute_optical_derived(&runtime, &req_huge, &|| false, &mut |_| {})
            .expect_err("must enforce pixel limit");
        assert!(err_px.to_string().contains("OPTICAL_OUTPUT_PIXEL_LIMIT"));

        // 3. Cancellation leaves zero leftover .tif or .part files
        let mut req_cancel = req_huge;
        req_cancel.target_resolution_m = None;
        let cancel_flag = AtomicBool::new(true);
        let err_cancel = execute_optical_derived(
            &runtime,
            &req_cancel,
            &|| cancel_flag.load(Ordering::SeqCst),
            &mut |_| {},
        )
        .expect_err("must cancel");
        assert!(err_cancel
            .to_string()
            .contains("OPTICAL_OPERATION_CANCELLED"));
        assert!(!out_tif.exists());
        assert!(!part_path(&out_tif).exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn bigtiff_threshold_constant_matches_spec() {
        assert_eq!(OPTICAL_BIGTIFF_THRESHOLD_BYTES, 3_800_000_000);
        assert_eq!(OPTICAL_OUTPUT_PIXEL_LIMIT, 400_000_000);
    }

    #[test]
    #[ignore = "requires optional native GIS runtime (run scripts/run_native_gis_tests.sh with INSAR_GIS_RUNTIME_ROOT)"]
    fn coupled_landsat_qa_pixel_and_qa_radsat_masks_saturated_pixels() {
        let runtime = require_runtime_for_optical_test();
        let dir = test_temp_dir("landsat-coupled-qa");
        let gt = [520_000.0, 30.0, 0.0, 3_520_480.0, 0.0, -30.0];

        let mut sr_b4 = vec![12_000u16; 16 * 16];
        // Make pixel values distinct so clear vs terrain-occlusion values are verified exact
        for (idx, val) in sr_b4.iter_mut().enumerate() {
            *val = 10_000 + (idx as u16) * 5;
        }
        // All QA_PIXEL clear (21824 = clear bit6, bits 0..4 all 0), EXCEPT col 0 cloudy (bit 3 = 8 -> 21832)
        let mut qa_pixel = vec![21824u16; 16 * 16];
        for r in 0..16 {
            qa_pixel[r * 16] = 21824 | (1 << 3);
        }
        // QA_RADSAT: cols 8..16 have band-3 radiometric saturation (bit 2 = 4),
        // while col 4 has ONLY bit 11 (terrain occlusion = 2048), which must remain valid!
        let mut qa_radsat = vec![0u16; 16 * 16];
        for r in 0..16 {
            qa_radsat[r * 16 + 4] = 1 << 11;
            for c in 8..16 {
                qa_radsat[r * 16 + c] = 1 << 2;
            }
        }

        let b4_path = dir.join("LC09_SR_B4.tif");
        let qa_pixel_path = dir.join("LC09_QA_PIXEL.tif");
        let qa_radsat_path = dir.join("LC09_QA_RADSAT.tif");
        write_test_geotiff_u16(&runtime, &b4_path, 32650, gt, [16, 16], Some(0.0), &sr_b4);
        write_test_geotiff_u16(
            &runtime,
            &qa_pixel_path,
            32650,
            gt,
            [16, 16],
            None,
            &qa_pixel,
        );
        write_test_geotiff_u16(
            &runtime,
            &qa_radsat_path,
            32650,
            gt,
            [16, 16],
            None,
            &qa_radsat,
        );

        let ctx = runtime.create_proj_context().expect("ctx");
        let wgs = transform_points_bbox(
            &runtime,
            &ctx,
            "EPSG:32650",
            "EPSG:4326",
            [520_001.0, 3_520_001.0, 520_479.0, 3_520_479.0],
            None,
        )
        .expect("wgs");

        let out_tif = dir.join("landsat_coupled_out.tif");
        let res = execute_optical_derived(
            &runtime,
            &OpticalDerivedRequest {
                mode: OpticalDerivedMode::SingleSceneClip,
                aoi_bbox_wgs84: wgs,
                aoi_rings_wgs84: None,
                band_order: vec!["SR_B4".to_owned()],
                target_crs: None,
                target_resolution_m: None,
                resampling: OpticalResampling::Nearest,
                cloud_mask: OpticalCloudMaskPolicy::CloudToNodata,
                mosaic_strategy: OpticalMosaicStrategy::QualityFirst,
                max_temporal_span_days: None,
                overwrite: false,
                scenes: vec![OpticalSceneInput {
                    scene_id: "LC09_L2SP_121038_20260615".to_owned(),
                    source_kind: OpticalSourceKind::LandsatC2L2,
                    collection: "landsat-c2-l2".to_owned(),
                    acquired_at: Some("2026-06-15T02:30:00Z".to_owned()),
                    cloud_cover_pct: Some(5.0),
                    footprint_bbox_wgs84: Some(wgs),
                    footprint_rings_wgs84: None,
                    native_epsg: Some(32650),
                    band_assets: vec![OpticalBandAssetInput {
                        band_id: "SR_B4".to_owned(),
                        stable_href: b4_path.to_string_lossy().into_owned(),
                        resolved_href: None,
                        band_resolution_m: Some(30.0),
                        role: Some("data".to_owned()),
                    }],
                    qa_asset: Some(OpticalBandAssetInput {
                        band_id: "QA_PIXEL".to_owned(),
                        stable_href: qa_pixel_path.to_string_lossy().into_owned(),
                        resolved_href: None,
                        band_resolution_m: Some(30.0),
                        role: Some("cloud".to_owned()),
                    }),
                    secondary_qa_asset: Some(OpticalBandAssetInput {
                        band_id: "QA_RADSAT".to_owned(),
                        stable_href: "https://landsatlook.usgs.gov/QA_RADSAT.tif?sig=RADSAT_SECRET"
                            .to_owned(),
                        resolved_href: Some(qa_radsat_path.to_string_lossy().into_owned()),
                        band_resolution_m: Some(30.0),
                        role: Some("saturation".to_owned()),
                    }),
                    stac_metadata: None,
                }],
                output_tif_path: out_tif.clone(),
                write_lineage_sidecar: false,
                write_provenance_sidecars: true,
            },
            &|| false,
            &mut |_| {},
        )
        .expect("coupled QA clip");

        assert!(res.readback_verified);
        assert!(res.credential_fields_redacted >= 1);
        let prov_raw = fs::read_to_string(&res.provenance_path).expect("prov");
        assert!(!prov_raw.contains("RADSAT_SECRET"));
        assert!(prov_raw.contains("\"secondary_qa_asset\""));

        let (_hdr, out_vals) = read_test_geotiff_u16(&runtime, &out_tif, 1);
        for r in 0..16 {
            for c in 0..16 {
                let got = out_vals[r * 16 + c];
                if c == 0 || c >= 8 {
                    assert_eq!(
                        got, 0,
                        "cloudy/saturated pixel ({c},{r}) must become NoData"
                    );
                } else {
                    assert_eq!(
                        got,
                        sr_b4[r * 16 + c],
                        "clear and terrain-occluded pixel ({c},{r}) must stay exact"
                    );
                }
            }
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore = "requires optional native GIS runtime (run scripts/run_native_gis_tests.sh with INSAR_GIS_RUNTIME_ROOT)"]
    fn post_write_readback_rejects_dimension_mismatch_and_corrupted_geotiff() {
        let runtime = require_runtime_for_optical_test();
        let dir = test_temp_dir("readback-verify");
        let gt = [520_000.0, 10.0, 0.0, 3_520_160.0, 0.0, -10.0];
        let tif_path = dir.join("valid.tif.part");
        write_test_geotiff_u16(
            &runtime,
            &tif_path,
            32650,
            gt,
            [16, 16],
            Some(0.0),
            &vec![777u16; 256],
        );

        verify_written_geotiff_readback(
            &runtime,
            ReadbackVerifySpec {
                path: &tif_path,
                expected_width: 16,
                expected_height: 16,
                expected_band_count: 1,
                expected_dtype: GDAL_UINT16,
                expected_geotransform: &gt,
                expected_nodata: Some(0.0),
            },
        )
        .expect("valid geotiff must pass readback verify");

        let dim_err = verify_written_geotiff_readback(
            &runtime,
            ReadbackVerifySpec {
                path: &tif_path,
                expected_width: 32,
                expected_height: 16,
                expected_band_count: 1,
                expected_dtype: GDAL_UINT16,
                expected_geotransform: &gt,
                expected_nodata: Some(0.0),
            },
        )
        .expect_err("dimension mismatch must fail readback verify");
        assert!(dim_err
            .to_string()
            .contains("OPTICAL_READBACK_VERIFY_FAILED"));

        let corrupt_path = dir.join("corrupt.tif.part");
        fs::write(&corrupt_path, b"NOT_A_VALID_TIFF").expect("write garbage");
        let corrupt_err = verify_written_geotiff_readback(
            &runtime,
            ReadbackVerifySpec {
                path: &corrupt_path,
                expected_width: 16,
                expected_height: 16,
                expected_band_count: 1,
                expected_dtype: GDAL_UINT16,
                expected_geotransform: &gt,
                expected_nodata: Some(0.0),
            },
        )
        .expect_err("corrupt file must fail readback verify");
        assert!(corrupt_err
            .to_string()
            .contains("OPTICAL_READBACK_VERIFY_FAILED"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore = "requires optional native GIS runtime (run scripts/run_native_gis_tests.sh with INSAR_GIS_RUNTIME_ROOT)"]
    fn dense_polygon_scanline_mask_matches_point_in_rings_and_clips_fast() {
        let runtime = require_runtime_for_optical_test();
        let ctx = runtime.create_proj_context().expect("proj ctx");
        let dir = test_temp_dir("scanline-polygon");

        let build_ring = |vertex_count: usize| -> Vec<[f64; 2]> {
            let center_lon = 117.16;
            let center_lat = 31.85;
            let mut ring = Vec::with_capacity(vertex_count + 1);
            for idx in 0..vertex_count {
                let theta = 2.0 * std::f64::consts::PI * (idx as f64) / (vertex_count as f64);
                let r = 0.018 * (1.0 + 0.22 * (5.0 * theta).sin() + 0.10 * (11.0 * theta).cos());
                ring.push([center_lon + r * theta.cos(), center_lat + r * theta.sin()]);
            }
            ring.push(ring[0]);
            ring
        };

        let center_tf = ProjTransformGuard::from_crs(&runtime, &ctx, "EPSG:4326", "EPSG:32650")
            .expect("fwd tf");
        let (cx, cy) = center_tf
            .transform(&runtime, 117.16, 31.85)
            .expect("center utm");
        let to_wgs84 =
            ProjTransformGuard::from_crs(&runtime, &ctx, "EPSG:32650", "EPSG:4326").expect("inv");

        // Part A: Full pixel parity against reference point_in_rings on a 256x256 grid
        let ring_500 = build_ring(500);
        let gt_256 = [cx - 1_280.0, 10.0, 0.0, cy + 1_280.0, 0.0, -10.0];
        let edges_500 = build_projected_polygon_edges(
            &runtime,
            &ctx,
            "EPSG:32650",
            std::slice::from_ref(&ring_500),
        )
        .expect("edges 500");
        let mut mask_500 = vec![false; 256 * 256];
        rasterize_projected_polygon_strip(&edges_500, &gt_256, 0, 256, 256, &mut mask_500);
        let mut mismatches = 0usize;
        for r in 0..256 {
            let out_y = gt_256[3] + (r as f64 + 0.5) * gt_256[5];
            for c in 0..256 {
                let out_x = gt_256[0] + (c as f64 + 0.5) * gt_256[1];
                let expected =
                    to_wgs84
                        .transform(&runtime, out_x, out_y)
                        .is_some_and(|(lon, lat)| {
                            point_in_rings(lon, lat, std::slice::from_ref(&ring_500))
                        });
                if mask_500[r * 256 + c] != expected {
                    mismatches += 1;
                }
            }
        }
        let parity_rate_256 = 1.0 - (mismatches as f64) / ((256 * 256) as f64);
        assert!(
            parity_rate_256 >= 0.9999,
            "scanline parity too low: {parity_rate_256} (mismatches={mismatches})"
        );

        // Part B: 20,000-vertex polygon rasterization speed on a 512x512 10m grid
        let dense_ring = build_ring(20_000);
        let gt_512 = [cx - 2_560.0, 10.0, 0.0, cy + 2_560.0, 0.0, -10.0];
        let started = std::time::Instant::now();
        let dense_edges = build_projected_polygon_edges(
            &runtime,
            &ctx,
            "EPSG:32650",
            std::slice::from_ref(&dense_ring),
        )
        .expect("dense edges");
        let mut dense_mask = vec![false; 512 * 512];
        rasterize_projected_polygon_strip(&dense_edges, &gt_512, 0, 512, 512, &mut dense_mask);
        let elapsed = started.elapsed();
        assert!(
            elapsed.as_millis() < 250,
            "20k-vertex 512x512 scanline rasterization took too long: {elapsed:?}"
        );
        let inside_count = dense_mask.iter().filter(|&&inside| inside).count();
        assert!(
            inside_count > 10_000 && inside_count < 512 * 512,
            "unexpected inside_count={inside_count}"
        );

        let mut strided_mismatches = 0usize;
        let mut strided_total = 0usize;
        for r in (0..512).step_by(8) {
            let out_y = gt_512[3] + (r as f64 + 0.5) * gt_512[5];
            for c in (0..512).step_by(8) {
                let out_x = gt_512[0] + (c as f64 + 0.5) * gt_512[1];
                let expected =
                    to_wgs84
                        .transform(&runtime, out_x, out_y)
                        .is_some_and(|(lon, lat)| {
                            point_in_rings(lon, lat, std::slice::from_ref(&dense_ring))
                        });
                strided_total += 1;
                if dense_mask[r * 512 + c] != expected {
                    strided_mismatches += 1;
                }
            }
        }
        let dense_parity = 1.0 - (strided_mismatches as f64) / (strided_total as f64);
        assert!(
            dense_parity >= 0.9999,
            "20k-vertex scanline strided parity too low: {dense_parity} ({strided_mismatches}/{strided_total})"
        );

        // Part C: End-to-end single-scene clip using 20k-vertex polygon
        let src_gt = [cx - 2_560.0, 10.0, 0.0, cy + 2_560.0, 0.0, -10.0];
        let src_b04 = vec![1234u16; 512 * 512];
        let src_path = dir.join("S2_B04_dense_poly.tif");
        write_test_geotiff_u16(
            &runtime,
            &src_path,
            32650,
            src_gt,
            [512, 512],
            Some(0.0),
            &src_b04,
        );

        let mut min_lon = f64::INFINITY;
        let mut max_lon = f64::NEG_INFINITY;
        let mut min_lat = f64::INFINITY;
        let mut max_lat = f64::NEG_INFINITY;
        for &[lon, lat] in &dense_ring {
            min_lon = min_lon.min(lon);
            max_lon = max_lon.max(lon);
            min_lat = min_lat.min(lat);
            max_lat = max_lat.max(lat);
        }
        let out_tif = dir.join("clip_dense_poly_out.tif");
        let res = execute_optical_derived(
            &runtime,
            &OpticalDerivedRequest {
                mode: OpticalDerivedMode::SingleSceneClip,
                aoi_bbox_wgs84: [min_lon, min_lat, max_lon, max_lat],
                aoi_rings_wgs84: Some(vec![dense_ring]),
                band_order: vec!["B04".to_owned()],
                target_crs: Some("EPSG:32650".to_owned()),
                target_resolution_m: None,
                resampling: OpticalResampling::Nearest,
                cloud_mask: OpticalCloudMaskPolicy::None,
                mosaic_strategy: OpticalMosaicStrategy::QualityFirst,
                max_temporal_span_days: None,
                overwrite: false,
                scenes: vec![OpticalSceneInput {
                    scene_id: "S2A_POLY_TEST".to_owned(),
                    source_kind: OpticalSourceKind::Sentinel2L2a,
                    collection: "sentinel-2-l2a".to_owned(),
                    acquired_at: Some("2026-06-15T02:55:11Z".to_owned()),
                    cloud_cover_pct: Some(1.0),
                    footprint_bbox_wgs84: Some([
                        min_lon - 0.1,
                        min_lat - 0.1,
                        max_lon + 0.1,
                        max_lat + 0.1,
                    ]),
                    footprint_rings_wgs84: None,
                    native_epsg: Some(32650),
                    band_assets: vec![OpticalBandAssetInput {
                        band_id: "B04".to_owned(),
                        stable_href: src_path.to_string_lossy().into_owned(),
                        resolved_href: None,
                        band_resolution_m: Some(10.0),
                        role: Some("data".to_owned()),
                    }],
                    qa_asset: None,
                    secondary_qa_asset: None,
                    stac_metadata: None,
                }],
                output_tif_path: out_tif.clone(),
                write_lineage_sidecar: false,
                write_provenance_sidecars: true,
            },
            &|| false,
            &mut |_| {},
        )
        .expect("dense polygon clip");

        assert!(res.readback_verified);
        let (_hdr_out, vals_out) = read_test_geotiff_u16(&runtime, &out_tif, 1);
        let valid_pixels = vals_out.iter().filter(|&&v| v == 1234).count();
        let nodata_pixels = vals_out.iter().filter(|&&v| v == 0).count();
        assert!(
            valid_pixels > 0 && nodata_pixels > 0,
            "expected both valid ({valid_pixels}) and masked NoData ({nodata_pixels}) pixels"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore = "requires optional native GIS runtime (run scripts/run_native_gis_tests.sh with INSAR_GIS_RUNTIME_ROOT)"]
    fn optional_provenance_sidecars_disabled_skips_json_writes() {
        let runtime = require_runtime_for_optical_test();
        let dir = test_temp_dir("no-sidecars");
        let gt = [520_000.0, 10.0, 0.0, 3_520_160.0, 0.0, -10.0];
        let b04 = vec![2500u16; 16 * 16];
        let b04_path = dir.join("S2_L1C_B04.tif");
        write_test_geotiff_u16(&runtime, &b04_path, 32650, gt, [16, 16], Some(0.0), &b04);

        let ctx = runtime.create_proj_context().expect("ctx");
        let wgs = transform_points_bbox(
            &runtime,
            &ctx,
            "EPSG:32650",
            "EPSG:4326",
            [520_010.0, 3_520_010.0, 520_150.0, 3_520_150.0],
            None,
        )
        .expect("wgs");

        let out_tif = dir.join("l1c_clean.tif");
        let res = execute_optical_derived(
            &runtime,
            &OpticalDerivedRequest {
                mode: OpticalDerivedMode::SingleSceneClip,
                aoi_bbox_wgs84: wgs,
                aoi_rings_wgs84: None,
                band_order: vec!["B04".to_owned()],
                target_crs: None,
                target_resolution_m: None,
                resampling: OpticalResampling::Nearest,
                cloud_mask: OpticalCloudMaskPolicy::None,
                mosaic_strategy: OpticalMosaicStrategy::QualityFirst,
                max_temporal_span_days: None,
                overwrite: false,
                scenes: vec![OpticalSceneInput {
                    scene_id: "S2A_MSIL1C_TEST".to_owned(),
                    source_kind: OpticalSourceKind::Sentinel2L1c,
                    collection: "sentinel-2-l1c".to_owned(),
                    acquired_at: Some("2026-06-15T02:55:00Z".to_owned()),
                    cloud_cover_pct: Some(1.0),
                    footprint_bbox_wgs84: Some(wgs),
                    footprint_rings_wgs84: None,
                    native_epsg: Some(32650),
                    band_assets: vec![OpticalBandAssetInput {
                        band_id: "B04".to_owned(),
                        stable_href: b04_path.to_string_lossy().into_owned(),
                        resolved_href: None,
                        band_resolution_m: Some(10.0),
                        role: Some("data".to_owned()),
                    }],
                    qa_asset: None,
                    secondary_qa_asset: None,
                    stac_metadata: None,
                }],
                output_tif_path: out_tif.clone(),
                write_lineage_sidecar: false,
                write_provenance_sidecars: false,
            },
            &|| false,
            &mut |_| {},
        )
        .expect("clip without provenance sidecars");

        assert!(out_tif.exists());
        assert!(res.readback_verified);
        assert!(!res.provenance_path.exists());
        assert!(!res.source_stac_path.exists());
        assert!(!res.aoi_geojson_path.exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
