use super::runtime::NativeGisRuntime;
use super::{NativeGisError, NativeGisErrorCode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::ffi::{CStr, CString};
use std::fs;
use std::os::raw::{c_char, c_int, c_uint, c_void};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

type Dataset = *mut c_void;
type RasterBand = *mut c_void;
type Driver = *mut c_void;

static STAGING_NONCE: AtomicU64 = AtomicU64::new(0);

const GDAL_OF_RASTER: c_uint = 0x02;
const GDAL_ACCESS_READ_ONLY: c_uint = 0x00;
const GDAL_READ: c_int = 0;
const GDAL_WRITE: c_int = 1;
const GDAL_BYTE: c_int = 1;
const GDAL_FLOAT32: c_int = 6;
const GDAL_FLOAT64: c_int = 7;
type GdalGetRasterScale = unsafe extern "C" fn(RasterBand, *mut c_int) -> f64;
type GdalGetRasterOffset = unsafe extern "C" fn(RasterBand, *mut c_int) -> f64;

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
type GdalGetDatasetDriver = unsafe extern "C" fn(Dataset) -> Driver;
type GdalGetDescription = unsafe extern "C" fn(*mut c_void) -> *const c_char;
type GdalGetMetadataItem =
    unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> *const c_char;
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
type GdalFlushCache = unsafe extern "C" fn(Dataset);
type GdalCreateCopy = unsafe extern "C" fn(
    Driver,
    *const c_char,
    Dataset,
    c_int,
    *const *const c_char,
    *const c_void,
    *mut c_void,
) -> Dataset;
type GdalBuildOverviews = unsafe extern "C" fn(
    Dataset,
    *const c_char,
    c_int,
    *const c_int,
    c_int,
    *const c_int,
    *const c_void,
    *mut c_void,
) -> c_int;
type GdalWarpAppOptionsNew = unsafe extern "C" fn(*mut *mut c_char, *mut c_void) -> *mut c_void;
type GdalWarpAppOptionsFree = unsafe extern "C" fn(*mut c_void);
type GdalWarp = unsafe extern "C" fn(
    *const c_char,
    Dataset,
    c_int,
    *mut Dataset,
    *const c_void,
    *mut c_int,
) -> Dataset;

struct GdalWarpOptionsGuard {
    options: *mut c_void,
    free: GdalWarpAppOptionsFree,
}

impl GdalWarpOptionsGuard {
    fn new(options: *mut c_void, free: GdalWarpAppOptionsFree) -> Self {
        Self { options, free }
    }

    fn raw(&self) -> *mut c_void {
        self.options
    }
}

impl Drop for GdalWarpOptionsGuard {
    fn drop(&mut self) {
        if !self.options.is_null() {
            unsafe { (self.free)(self.options) };
        }
    }
}

struct GdalDatasetGuard {
    dataset: Dataset,
    close: GdalClose,
}

impl GdalDatasetGuard {
    fn new(dataset: Dataset, close: GdalClose) -> Self {
        Self { dataset, close }
    }

    fn raw(&self) -> Dataset {
        self.dataset
    }
}

impl Drop for GdalDatasetGuard {
    fn drop(&mut self) {
        if !self.dataset.is_null() {
            unsafe { (self.close)(self.dataset) };
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RasterPixelProbeRequest {
    pub source: String,
    pub lon: f64,
    pub lat: f64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RasterProcessRequest {
    pub sources: Vec<String>,
    pub output_path: PathBuf,
    pub operation: String,
    #[serde(default)]
    pub crop_bbox: Option<[f64; 4]>,
    #[serde(default)]
    pub band_a: Option<usize>,
    #[serde(default)]
    pub band_b: Option<usize>,
    #[serde(default)]
    pub azimuth_deg: Option<f64>,
    #[serde(default)]
    pub altitude_deg: Option<f64>,
    #[serde(default)]
    pub z_factor: Option<f64>,
    #[serde(default)]
    pub target_crs: Option<String>,
    #[serde(default)]
    pub resampling: Option<String>,
    #[serde(default)]
    pub output_format: Option<String>,
    #[serde(default)]
    pub preserve_dtype: Option<bool>,
}

fn error(code: NativeGisErrorCode, reason: impl Into<String>) -> NativeGisError {
    NativeGisError::contract(code, reason)
}

fn c_path(
    path: &Path,
    code: NativeGisErrorCode,
    reason: &'static str,
) -> Result<CString, NativeGisError> {
    let text = path.to_str().ok_or_else(|| error(code, reason))?;
    CString::new(text).map_err(|_| error(code, reason))
}

fn gdal_data_type(code: c_int) -> &'static str {
    match code {
        1 => "uint8",
        2 => "uint16",
        3 => "int16",
        4 => "uint32",
        5 => "int32",
        6 => "float32",
        7 => "float64",
        8 => "cint16",
        9 => "cint32",
        10 => "cfloat32",
        11 => "cfloat64",
        _ => "unknown",
    }
}

fn gdal_dtype_byte_size(code: c_int) -> Option<usize> {
    match code {
        1 => Some(1),
        2 | 3 => Some(2),
        4..=6 => Some(4),
        7 => Some(8),
        _ => None,
    }
}

fn read_typed_sample_as_f64(code: c_int, bytes: &[u8]) -> Option<f64> {
    match code {
        1 => bytes.first().map(|&v| f64::from(v)),
        2 => bytes
            .get(..2)
            .and_then(|s| <[u8; 2]>::try_from(s).ok())
            .map(|b| f64::from(u16::from_ne_bytes(b))),
        3 => bytes
            .get(..2)
            .and_then(|s| <[u8; 2]>::try_from(s).ok())
            .map(|b| f64::from(i16::from_ne_bytes(b))),
        4 => bytes
            .get(..4)
            .and_then(|s| <[u8; 4]>::try_from(s).ok())
            .map(|b| f64::from(u32::from_ne_bytes(b))),
        5 => bytes
            .get(..4)
            .and_then(|s| <[u8; 4]>::try_from(s).ok())
            .map(|b| f64::from(i32::from_ne_bytes(b))),
        6 => bytes
            .get(..4)
            .and_then(|s| <[u8; 4]>::try_from(s).ok())
            .map(|b| f64::from(f32::from_ne_bytes(b))),
        7 => bytes
            .get(..8)
            .and_then(|s| <[u8; 8]>::try_from(s).ok())
            .map(f64::from_ne_bytes),
        _ => None,
    }
}

fn normalize_raster_output_format(
    requested: Option<&str>,
    output: &Path,
) -> Result<&'static str, NativeGisError> {
    if let Some(raw) = requested.map(str::trim).filter(|s| !s.is_empty()) {
        return match raw.to_ascii_lowercase().as_str() {
            "gtiff" | "geotiff" | "tif" | "tiff" => Ok("GTiff"),
            "envi" => Ok("ENVI"),
            other => Err(error(
                NativeGisErrorCode::DemReprojectionFailed,
                format!("不支持的输出栅格格式: {other} (仅支持 GTiff / ENVI)"),
            )),
        };
    }
    let ext = output
        .extension()
        .and_then(|v| v.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "img" | "dat" | "bsq" | "bil" | "bip" | "envi" => Ok("ENVI"),
        _ => Ok("GTiff"),
    }
}

fn normalize_warp_resampling(requested: Option<&str>) -> Result<&'static str, NativeGisError> {
    match requested
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        None | Some("bilinear") => Ok("bilinear"),
        Some("near" | "nearest") => Ok("near"),
        Some("cubic") => Ok("cubic"),
        Some("cubicspline") => Ok("cubicspline"),
        Some("lanczos") => Ok("lanczos"),
        Some("average") => Ok("average"),
        Some("mode") => Ok("mode"),
        Some("max") => Ok("max"),
        Some("min") => Ok("min"),
        Some("med") => Ok("med"),
        Some("q1") => Ok("q1"),
        Some("q3") => Ok("q3"),
        Some(other) => Err(error(
            NativeGisErrorCode::DemReprojectionFailed,
            format!("不支持的重采样方式: {other}"),
        )),
    }
}

struct StagingDirGuard {
    dir: PathBuf,
}

impl StagingDirGuard {
    fn create(output: &Path) -> Result<(Self, PathBuf), NativeGisError> {
        let file_name = output
            .file_name()
            .and_then(|v| v.to_str())
            .ok_or_else(|| error(NativeGisErrorCode::DemReprojectionFailed, "输出文件名无效"))?;
        let parent = output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent).map_err(|_| {
            error(
                NativeGisErrorCode::DemReprojectionFailed,
                "无法创建栅格处理输出目录",
            )
        })?;
        let nonce = STAGING_NONCE.fetch_add(1, Ordering::Relaxed);
        let dir = parent.join(format!(".eostage_tmp_{}_{}", std::process::id(), nonce));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).map_err(|_| {
            error(
                NativeGisErrorCode::DemReprojectionFailed,
                "无法创建栅格处理临时暂存目录",
            )
        })?;
        let staging_file = dir.join(file_name);
        Ok((Self { dir }, staging_file))
    }

    fn promote_outputs(
        self,
        staging_file: &Path,
        output: &Path,
        driver: &str,
    ) -> Result<(), NativeGisError> {
        let parent = output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        if output.exists() {
            let _ = fs::remove_file(output);
        }
        promote_single_file(staging_file, output)?;
        if driver.eq_ignore_ascii_case("ENVI") {
            if let Ok(entries) = fs::read_dir(&self.dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file()
                        && path
                            .extension()
                            .and_then(|e| e.to_str())
                            .is_some_and(|e| e.eq_ignore_ascii_case("hdr"))
                    {
                        if let Some(hdr_name) = path.file_name() {
                            let dest_hdr = parent.join(hdr_name);
                            if dest_hdr.exists() {
                                let _ = fs::remove_file(&dest_hdr);
                            }
                            promote_single_file(&path, &dest_hdr)?;
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

impl Drop for StagingDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn promote_single_file(src: &Path, dst: &Path) -> Result<(), NativeGisError> {
    if fs::rename(src, dst).is_ok() {
        return Ok(());
    }
    fs::copy(src, dst).map_err(|_| {
        error(
            NativeGisErrorCode::DemReprojectionFailed,
            "无法原子提交栅格处理输出文件",
        )
    })?;
    let _ = fs::remove_file(src);
    Ok(())
}

fn is_valid_nodata(value: f64, nodata: Option<f64>) -> bool {
    value.is_finite()
        && !nodata.is_some_and(|nd| {
            if nd.is_nan() {
                false
            } else {
                (value - nd).abs() <= 1e-5
            }
        })
}

fn build_statistics_json(
    total_pixels: usize,
    valid_count: usize,
    min_val: f64,
    max_val: f64,
    sum_val: f64,
) -> Value {
    let denom = total_pixels.max(1);
    json!({
        "valid_count": valid_count,
        "total_pixels": total_pixels,
        "valid_fraction": (valid_count as f64) / (denom as f64),
        "min": (valid_count > 0).then_some(min_val),
        "max": (valid_count > 0).then_some(max_val),
        "mean": (valid_count > 0).then_some(sum_val / (valid_count as f64)),
    })
}

fn is_wgs84_geographic_crs(projection: &str) -> bool {
    let upper = projection.trim().to_ascii_uppercase();
    if upper.contains("PROJCS[") || upper.contains("PROJCRS[") {
        return false;
    }
    let is_geographic = upper.contains("GEOGCS[")
        || upper.contains("GEOGCRS[")
        || upper == "EPSG:4326"
        || upper == "CRS:84";
    let is_wgs84 = upper.contains("WGS 84")
        || upper.contains("WGS_1984")
        || upper.contains("WGS84")
        || upper.contains("EPSG:4326")
        || upper == "CRS:84"
        || upper.contains("EPSG\",\"4326")
        || upper.contains("EPSG\",4326");
    is_geographic && is_wgs84
}

pub fn normalize_gdal_source_string(raw: &str) -> Result<(String, bool), NativeGisError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(error(
            NativeGisErrorCode::DemInvalidRaster,
            "栅格数据源路径或 URL 不能为空",
        ));
    }
    if trimmed.starts_with("/vsicurl/http://") || trimmed.starts_with("/vsicurl/https://") {
        return Ok((trimmed.to_owned(), true));
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return Ok((format!("/vsicurl/{trimmed}"), true));
    }
    let path = Path::new(trimmed);
    let metadata = fs::metadata(path)
        .map_err(|_| error(NativeGisErrorCode::DemInvalidRaster, "本地栅格文件不可读"))?;
    if !metadata.is_file() {
        return Err(error(
            NativeGisErrorCode::DemInvalidRaster,
            "本地栅格路径不是常规文件",
        ));
    }
    Ok((trimmed.to_owned(), false))
}

pub(crate) unsafe fn configure_gdal_runtime_for_source(
    runtime: &NativeGisRuntime,
    allow_remote_stream: bool,
) -> Result<(), NativeGisError> {
    let retry = if allow_remote_stream { "3" } else { "0" };
    let readdir = if allow_remote_stream {
        "EMPTY_DIR"
    } else {
        "NO"
    };
    let proj_data = c_path(
        &runtime.root().join(&runtime.manifest().proj_data_dir),
        NativeGisErrorCode::GisEngineUnavailable,
        "PROJ 数据目录编码无效",
    )?;
    let gdal_data = c_path(
        &runtime.root().join(&runtime.manifest().gdal_data_dir),
        NativeGisErrorCode::GisEngineUnavailable,
        "GDAL 数据目录编码无效",
    )?;
    runtime.configure_and_register_gdal(
        &[
            ("PROJ_NETWORK", "OFF"),
            ("CPL_CURL_VERBOSE", "NO"),
            ("GDAL_HTTP_MAX_RETRY", retry),
            ("GDAL_HTTP_RETRY_DELAY", "1"),
            ("GDAL_HTTP_TIMEOUT", "30"),
            ("GDAL_HTTP_MERGE_CONSECUTIVE_RANGES", "YES"),
            ("GDAL_HTTP_MULTIPLEX", "YES"),
            ("GDAL_DISABLE_READDIR_ON_OPEN", readdir),
            ("GDAL_DRIVER_PATH", ""),
        ],
        &proj_data,
        &gdal_data,
    )
}

/// Inspect either a local raster file or a remote COG/STAC asset via `/vsicurl/`
/// using HTTP Range requests without downloading the full file.
pub(crate) fn inspect_raster_source(
    runtime: &NativeGisRuntime,
    source: &str,
) -> Result<Value, NativeGisError> {
    let (gdal_source, is_remote) = normalize_gdal_source_string(source)?;
    let source_c = CString::new(gdal_source.as_str())
        .map_err(|_| error(NativeGisErrorCode::DemInvalidRaster, "栅格源编码无效"))?;
    let _proj_context = runtime.create_proj_context()?;
    let allowed_driver_names = [
        CString::new("GTiff").expect("static GDAL driver"),
        CString::new("ENVI").expect("static GDAL driver"),
        CString::new("VRT").expect("static GDAL driver"),
        CString::new("PNG").expect("static GDAL driver"),
        CString::new("JPEG").expect("static GDAL driver"),
    ];
    let allowed_driver_ptrs = [
        allowed_driver_names[0].as_ptr(),
        allowed_driver_names[1].as_ptr(),
        allowed_driver_names[2].as_ptr(),
        allowed_driver_names[3].as_ptr(),
        allowed_driver_names[4].as_ptr(),
        std::ptr::null(),
    ];
    unsafe {
        configure_gdal_runtime_for_source(runtime, is_remote)?;
        let open = *runtime.gdal_symbol::<GdalOpenEx>(b"GDALOpenEx\0")?;
        let close = *runtime.gdal_symbol::<GdalClose>(b"GDALClose\0")?;
        let dataset = open(
            source_c.as_ptr(),
            GDAL_OF_RASTER | GDAL_ACCESS_READ_ONLY,
            allowed_driver_ptrs.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
        );
        if is_remote {
            let _ = configure_gdal_runtime_for_source(runtime, false);
        }
        if dataset.is_null() {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                "目标源不是可读取的栅格或远程 COG",
            ));
        }
        let guard = GdalDatasetGuard::new(dataset, close);
        let width =
            runtime.gdal_symbol::<GdalGetRasterXSize>(b"GDALGetRasterXSize\0")?(guard.raw());
        let height =
            runtime.gdal_symbol::<GdalGetRasterYSize>(b"GDALGetRasterYSize\0")?(guard.raw());
        let count =
            runtime.gdal_symbol::<GdalGetRasterCount>(b"GDALGetRasterCount\0")?(guard.raw());
        if width <= 0 || height <= 0 || count <= 0 {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                "栅格尺寸或波段数无效",
            ));
        }
        let get_projection =
            runtime.gdal_symbol::<GdalGetProjectionRef>(b"GDALGetProjectionRef\0")?;
        let projection = get_projection(guard.raw());
        let crs = if projection.is_null() || CStr::from_ptr(projection).to_bytes().is_empty() {
            None
        } else {
            Some(CStr::from_ptr(projection).to_string_lossy().to_string())
        };
        let mut transform = [0.0f64; 6];
        let transform_status = runtime
            .gdal_symbol::<GdalGetGeoTransform>(b"GDALGetGeoTransform\0")?(
            guard.raw(),
            transform.as_mut_ptr(),
        );
        let driver =
            runtime.gdal_symbol::<GdalGetDatasetDriver>(b"GDALGetDatasetDriver\0")?(guard.raw());
        let get_description = runtime.gdal_symbol::<GdalGetDescription>(b"GDALGetDescription\0")?;
        let driver_name = if driver.is_null() {
            String::new()
        } else {
            let desc = get_description(driver);
            if desc.is_null() {
                String::new()
            } else {
                CStr::from_ptr(desc).to_string_lossy().to_string()
            }
        };
        let get_band = runtime.gdal_symbol::<GdalGetRasterBand>(b"GDALGetRasterBand\0")?;
        let get_type = runtime.gdal_symbol::<GdalGetRasterDataType>(b"GDALGetRasterDataType\0")?;
        let get_nodata =
            runtime.gdal_symbol::<GdalGetRasterNoDataValue>(b"GDALGetRasterNoDataValue\0")?;
        let mut dtypes = Vec::new();
        let mut nodata = None;
        for band_index in 1..=count {
            let band = get_band(guard.raw(), band_index);
            if band.is_null() {
                return Err(error(
                    NativeGisErrorCode::DemInvalidRaster,
                    "栅格波段不可读取",
                ));
            }
            dtypes.push(gdal_data_type(get_type(band)));
            if band_index == 1 {
                let mut has_nodata = 0;
                let value = get_nodata(band, &mut has_nodata);
                if has_nodata != 0 {
                    nodata = Some(value);
                }
            }
        }
        let get_metadata_item =
            runtime.gdal_symbol::<GdalGetMetadataItem>(b"GDALGetMetadataItem\0")?;
        let area_or_point_key = CString::new("AREA_OR_POINT").expect("static GDAL metadata key");
        let area_or_point =
            get_metadata_item(guard.raw(), area_or_point_key.as_ptr(), std::ptr::null());
        let area_or_point = (!area_or_point.is_null())
            .then(|| CStr::from_ptr(area_or_point).to_string_lossy().to_string());
        let img_struct_dom = CString::new("IMAGE_STRUCTURE").expect("static GDAL metadata domain");
        let layout_key = CString::new("LAYOUT").expect("static GDAL metadata key");
        let comp_key = CString::new("COMPRESSION").expect("static GDAL metadata key");
        let layout_ptr =
            get_metadata_item(guard.raw(), layout_key.as_ptr(), img_struct_dom.as_ptr());
        let layout = (!layout_ptr.is_null())
            .then(|| CStr::from_ptr(layout_ptr).to_string_lossy().to_string());
        let comp_ptr = get_metadata_item(guard.raw(), comp_key.as_ptr(), img_struct_dom.as_ptr());
        let compression =
            (!comp_ptr.is_null()).then(|| CStr::from_ptr(comp_ptr).to_string_lossy().to_string());
        let bounds = if transform_status == 0 {
            let west = transform[0];
            let north = transform[3];
            let east = west + transform[1] * f64::from(width) + transform[2] * f64::from(height);
            let south = north + transform[4] * f64::from(width) + transform[5] * f64::from(height);
            Some([
                west.min(east),
                south.min(north),
                west.max(east),
                south.max(north),
            ])
        } else {
            None
        };
        let size_bytes = if is_remote {
            None
        } else {
            fs::metadata(source.trim()).ok().map(|m| m.len())
        };
        Ok(json!({
            "ok": true,
            "source": source.trim(),
            "resolved_gdal_path": gdal_source,
            "is_remote_stream": is_remote,
            "size_bytes": size_bytes,
            "readable": true,
            "metadata": {
                "driver": driver_name,
                "width": width,
                "height": height,
                "count": count,
                "dtype": dtypes,
                "crs": crs,
                "bounds": bounds,
                "geotransform": (transform_status == 0).then_some(transform),
                "pixel_size": (transform_status == 0).then_some([transform[1].abs(), transform[5].abs()]),
                "nodata": nodata,
                "area_or_point": area_or_point,
                "layout": layout,
                "compression": compression,
            }
        }))
    }
}

fn decode_band_qa_semantics(band_name: &str, is_nodata: bool, raw: f64) -> Option<String> {
    if is_nodata || !raw.is_finite() {
        return None;
    }
    let upper = band_name.trim().to_ascii_uppercase();
    if upper == "SCL" || upper.contains("SCENE_CLASSIFICATION") {
        let code = raw.round() as i64;
        let label = match code {
            0 => "No Data (无数据)",
            1 => "Saturated / Defective (饱和/缺陷)",
            2 => "Dark Area Pixels (阴影/暗像元)",
            3 => "Cloud Shadows (云阴影)",
            4 => "Vegetation (植被)",
            5 => "Not Vegetated (裸土/建筑)",
            6 => "Water (水体)",
            7 => "Unclassified (未分类)",
            8 => "Cloud Medium Probability (中概率云)",
            9 => "Cloud High Probability (高概率云)",
            10 => "Thin Cirrus (薄卷云)",
            11 => "Snow / Ice (冰雪)",
            _ => "Unknown SCL Class",
        };
        return Some(format!("SCL={code}: {label}"));
    }
    if upper.contains("QA_PIXEL") {
        let bits = raw.round() as u64;
        let flags = [
            (0, "Fill"),
            (1, "Dilated Cloud"),
            (2, "Cirrus"),
            (3, "Cloud"),
            (4, "Cloud Shadow"),
            (5, "Snow"),
            (6, "Clear"),
            (7, "Water"),
        ];
        let active: Vec<&str> = flags
            .iter()
            .filter_map(|(bit, name)| ((bits & (1 << bit)) != 0).then_some(*name))
            .collect();
        return Some(if active.is_empty() {
            format!("QA_PIXEL=0x{bits:04x}")
        } else {
            format!("QA_PIXEL=0x{bits:04x} ({})", active.join(", "))
        });
    }
    None
}

/// Probe a single physical pixel at WGS84 `(lon, lat)` across all bands using
/// PROJ coordinate normalization + full 6-term affine GeoTransform inversion
/// and a `1x1` `GDALRasterIO` read (Pixel Identify / Cursor Probe).
pub(crate) fn probe_raster_pixel(
    runtime: &NativeGisRuntime,
    req: &RasterPixelProbeRequest,
) -> Result<Value, NativeGisError> {
    if !req.lon.is_finite() || !req.lat.is_finite() || req.lon.abs() > 180.0 || req.lat.abs() > 90.0
    {
        return Err(error(
            NativeGisErrorCode::AoiGeometryInvalid,
            "查询坐标 (lon, lat) 超出有效经纬度范围",
        ));
    }
    let (gdal_source, is_remote) = normalize_gdal_source_string(&req.source)?;
    let source_c = CString::new(gdal_source.as_str())
        .map_err(|_| error(NativeGisErrorCode::DemInvalidRaster, "栅格源编码无效"))?;
    let proj_context = runtime.create_proj_context()?;
    let allowed_driver_names = [
        CString::new("GTiff").expect("static GDAL driver"),
        CString::new("ENVI").expect("static GDAL driver"),
        CString::new("VRT").expect("static GDAL driver"),
        CString::new("PNG").expect("static GDAL driver"),
        CString::new("JPEG").expect("static GDAL driver"),
    ];
    let allowed_driver_ptrs = [
        allowed_driver_names[0].as_ptr(),
        allowed_driver_names[1].as_ptr(),
        allowed_driver_names[2].as_ptr(),
        allowed_driver_names[3].as_ptr(),
        allowed_driver_names[4].as_ptr(),
        std::ptr::null(),
    ];

    unsafe {
        configure_gdal_runtime_for_source(runtime, is_remote)?;
        let open = *runtime.gdal_symbol::<GdalOpenEx>(b"GDALOpenEx\0")?;
        let close = *runtime.gdal_symbol::<GdalClose>(b"GDALClose\0")?;
        let dataset = open(
            source_c.as_ptr(),
            GDAL_OF_RASTER | GDAL_ACCESS_READ_ONLY,
            allowed_driver_ptrs.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
        );
        if is_remote {
            let _ = configure_gdal_runtime_for_source(runtime, false);
        }
        if dataset.is_null() {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                "目标栅格不可读取或路径不存在",
            ));
        }
        let guard = GdalDatasetGuard::new(dataset, close);
        let width =
            runtime.gdal_symbol::<GdalGetRasterXSize>(b"GDALGetRasterXSize\0")?(guard.raw());
        let height =
            runtime.gdal_symbol::<GdalGetRasterYSize>(b"GDALGetRasterYSize\0")?(guard.raw());
        let count =
            runtime.gdal_symbol::<GdalGetRasterCount>(b"GDALGetRasterCount\0")?(guard.raw());
        if width <= 0 || height <= 0 || count <= 0 {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                "栅格尺寸或波段数无效",
            ));
        }

        let proj_ptr =
            runtime.gdal_symbol::<GdalGetProjectionRef>(b"GDALGetProjectionRef\0")?(guard.raw());
        let projection = if proj_ptr.is_null() {
            String::new()
        } else {
            CStr::from_ptr(proj_ptr).to_string_lossy().to_string()
        };
        let mut gt = [0.0_f64; 6];
        let gt_status = runtime.gdal_symbol::<GdalGetGeoTransform>(b"GDALGetGeoTransform\0")?(
            guard.raw(),
            gt.as_mut_ptr(),
        );
        if gt_status != 0 {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                "目标栅格缺少有效的仿射地理变换矩阵 (GeoTransform)",
            ));
        }

        let (proj_x, proj_y) = if !projection.trim().is_empty()
            && !is_wgs84_geographic_crs(&projection)
        {
            #[repr(C)]
            #[derive(Clone, Copy)]
            struct ProjCoord {
                x: f64,
                y: f64,
                z: f64,
                t: f64,
            }
            type ProjCreateCrs = unsafe extern "C" fn(
                *mut c_void,
                *const c_char,
                *const c_char,
                *const c_void,
            ) -> *mut c_void;
            type ProjNormalize = unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void;
            type ProjDestroy = unsafe extern "C" fn(*mut c_void) -> *mut c_void;
            type ProjTransFn = unsafe extern "C" fn(*mut c_void, c_int, ProjCoord) -> ProjCoord;

            let src_c = CString::new("EPSG:4326").expect("static EPSG:4326");
            let dst_c = CString::new(projection.trim()).map_err(|_| {
                error(
                    NativeGisErrorCode::DemCrsMissing,
                    "目标坐标系 WKT 包含 NUL 字节",
                )
            })?;
            let crs_create = *runtime.proj_symbol::<ProjCreateCrs>(b"proj_create_crs_to_crs\0")?;
            let normalize =
                *runtime.proj_symbol::<ProjNormalize>(b"proj_normalize_for_visualization\0")?;
            let destroy = *runtime.proj_symbol::<ProjDestroy>(b"proj_destroy\0")?;
            let trans = *runtime.proj_symbol::<ProjTransFn>(b"proj_trans\0")?;

            let op = crs_create(
                proj_context.raw(),
                src_c.as_ptr(),
                dst_c.as_ptr(),
                std::ptr::null(),
            );
            if op.is_null() {
                return Err(error(
                    NativeGisErrorCode::DemReprojectionFailed,
                    "无法创建 EPSG:4326 到目标栅格坐标系的 PROJ 变换",
                ));
            }
            let norm = normalize(proj_context.raw(), op);
            let pj = if norm.is_null() {
                op
            } else {
                destroy(op);
                norm
            };
            let out = trans(
                pj,
                1,
                ProjCoord {
                    x: req.lon,
                    y: req.lat,
                    z: 0.0,
                    t: 0.0,
                },
            );
            destroy(pj);
            if !out.x.is_finite() || !out.y.is_finite() {
                return Err(error(
                    NativeGisErrorCode::DemReprojectionFailed,
                    "指定经纬度无法投影到目标栅格坐标系",
                ));
            }
            (out.x, out.y)
        } else {
            (req.lon, req.lat)
        };

        let det = gt[1] * gt[5] - gt[2] * gt[4];
        if !det.is_finite() || det.abs() < 1e-18 {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                "仿射地理变换矩阵不可逆",
            ));
        }
        let dx = proj_x - gt[0];
        let dy = proj_y - gt[3];
        let px = (dx * gt[5] - dy * gt[2]) / det;
        let py = (gt[1] * dy - gt[4] * dx) / det;
        if !px.is_finite() || !py.is_finite() {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                "反算像元行列号得到非有限值",
            ));
        }
        let col = px.floor() as i64;
        let row = py.floor() as i64;
        let crs_value = (!projection.trim().is_empty()).then_some(projection);

        if col < 0 || row < 0 || col >= i64::from(width) || row >= i64::from(height) {
            return Ok(json!({
                "ok": true,
                "source": req.source.trim(),
                "lon": req.lon,
                "lat": req.lat,
                "projected_coord": [proj_x, proj_y],
                "crs": crs_value,
                "width": width,
                "height": height,
                "in_bounds": false,
                "col": col,
                "row": row,
                "bands": [],
            }));
        }

        let get_band = *runtime.gdal_symbol::<GdalGetRasterBand>(b"GDALGetRasterBand\0")?;
        let get_type = *runtime.gdal_symbol::<GdalGetRasterDataType>(b"GDALGetRasterDataType\0")?;
        let get_nodata =
            *runtime.gdal_symbol::<GdalGetRasterNoDataValue>(b"GDALGetRasterNoDataValue\0")?;
        let get_desc = *runtime.gdal_symbol::<GdalGetDescription>(b"GDALGetDescription\0")?;
        let raster_io = *runtime.gdal_symbol::<GdalRasterIo>(b"GDALRasterIO\0")?;
        let get_scale = runtime
            .gdal_symbol::<GdalGetRasterScale>(b"GDALGetRasterScale\0")
            .ok();
        let get_offset = runtime
            .gdal_symbol::<GdalGetRasterOffset>(b"GDALGetRasterOffset\0")
            .ok();

        let mut bands = Vec::with_capacity(usize::try_from(count).unwrap_or(1));
        for band_index in 1..=count {
            let band = get_band(guard.raw(), band_index);
            if band.is_null() {
                return Err(error(
                    NativeGisErrorCode::DemInvalidRaster,
                    "栅格波段句柄为空",
                ));
            }
            let desc_ptr = get_desc(band);
            let band_name = if desc_ptr.is_null() {
                format!("Band {band_index}")
            } else {
                let raw_name = CStr::from_ptr(desc_ptr).to_string_lossy().trim().to_owned();
                if raw_name.is_empty() {
                    format!("Band {band_index}")
                } else {
                    raw_name
                }
            };
            let dtype = gdal_data_type(get_type(band));
            let mut has_nodata = 0;
            let raw_nodata = get_nodata(band, &mut has_nodata);
            let nodata = (has_nodata != 0).then_some(raw_nodata);

            let mut has_scale = 0;
            let scale = match get_scale.as_ref() {
                Some(sym) => {
                    let s = sym(band, &mut has_scale);
                    if has_scale != 0 && s.is_finite() {
                        s
                    } else {
                        1.0
                    }
                }
                None => 1.0,
            };
            let mut has_offset = 0;
            let offset = match get_offset.as_ref() {
                Some(sym) => {
                    let o = sym(band, &mut has_offset);
                    if has_offset != 0 && o.is_finite() {
                        o
                    } else {
                        0.0
                    }
                }
                None => 0.0,
            };

            let mut pixel: f64 = f64::NAN;
            let io_err = raster_io(
                band,
                GDAL_READ,
                col as c_int,
                row as c_int,
                1,
                1,
                (&mut pixel as *mut f64).cast::<c_void>(),
                1,
                1,
                GDAL_FLOAT64,
                0,
                0,
            );
            if io_err != 0 {
                return Err(error(
                    NativeGisErrorCode::DemInvalidRaster,
                    format!("读取第 {band_index} 波段像元失败 (GDAL code {io_err})"),
                ));
            }

            let is_nodata = !pixel.is_finite()
                || nodata.is_some_and(|nd| {
                    if nd.is_nan() {
                        pixel.is_nan()
                    } else {
                        (pixel - nd).abs() <= 1e-9_f64.max(nd.abs() * 1e-9)
                    }
                });
            let physical_value = (!is_nodata).then_some(pixel * scale + offset);
            let qa_semantics = decode_band_qa_semantics(&band_name, is_nodata, pixel);

            bands.push(json!({
                "band": band_index,
                "name": band_name,
                "dtype": dtype,
                "raw_value": pixel.is_finite().then_some(pixel),
                "scale": scale,
                "offset": offset,
                "physical_value": physical_value,
                "nodata": nodata.filter(|nd| nd.is_finite()),
                "is_nodata": is_nodata,
                "qa_semantics": qa_semantics,
            }));
        }

        Ok(json!({
            "ok": true,
            "source": req.source.trim(),
            "lon": req.lon,
            "lat": req.lat,
            "projected_coord": [proj_x, proj_y],
            "crs": crs_value,
            "width": width,
            "height": height,
            "in_bounds": true,
            "col": col,
            "row": row,
            "bands": bands,
        }))
    }
}

unsafe fn open_raster_dataset_ro(
    runtime: &NativeGisRuntime,
    source: &str,
) -> Result<(GdalDatasetGuard, bool), NativeGisError> {
    let (gdal_source, is_remote) = normalize_gdal_source_string(source)?;
    let source_c = CString::new(gdal_source.as_str())
        .map_err(|_| error(NativeGisErrorCode::DemInvalidRaster, "栅格源编码无效"))?;
    let allowed_driver_names = [
        CString::new("GTiff").expect("static GDAL driver"),
        CString::new("ENVI").expect("static GDAL driver"),
        CString::new("VRT").expect("static GDAL driver"),
        CString::new("PNG").expect("static GDAL driver"),
        CString::new("JPEG").expect("static GDAL driver"),
    ];
    let allowed_driver_ptrs = [
        allowed_driver_names[0].as_ptr(),
        allowed_driver_names[1].as_ptr(),
        allowed_driver_names[2].as_ptr(),
        allowed_driver_names[3].as_ptr(),
        allowed_driver_names[4].as_ptr(),
        std::ptr::null(),
    ];
    configure_gdal_runtime_for_source(runtime, is_remote)?;
    let open = *runtime.gdal_symbol::<GdalOpenEx>(b"GDALOpenEx\0")?;
    let close = *runtime.gdal_symbol::<GdalClose>(b"GDALClose\0")?;
    let dataset = open(
        source_c.as_ptr(),
        GDAL_OF_RASTER | GDAL_ACCESS_READ_ONLY,
        allowed_driver_ptrs.as_ptr(),
        std::ptr::null(),
        std::ptr::null(),
    );
    if dataset.is_null() {
        let _ = configure_gdal_runtime_for_source(runtime, false);
        return Err(error(
            NativeGisErrorCode::DemInvalidRaster,
            "无法打开输入栅格或远程 /vsicurl/ 数据源",
        ));
    }
    Ok((GdalDatasetGuard::new(dataset, close), is_remote))
}

unsafe fn read_dataset_georef(
    runtime: &NativeGisRuntime,
    dataset: Dataset,
) -> Result<(c_int, c_int, c_int, String, [f64; 6]), NativeGisError> {
    let get_width = *runtime.gdal_symbol::<GdalGetRasterXSize>(b"GDALGetRasterXSize\0")?;
    let get_height = *runtime.gdal_symbol::<GdalGetRasterYSize>(b"GDALGetRasterYSize\0")?;
    let get_count = *runtime.gdal_symbol::<GdalGetRasterCount>(b"GDALGetRasterCount\0")?;
    let get_projection = *runtime.gdal_symbol::<GdalGetProjectionRef>(b"GDALGetProjectionRef\0")?;
    let get_transform = *runtime.gdal_symbol::<GdalGetGeoTransform>(b"GDALGetGeoTransform\0")?;
    let full_width = get_width(dataset);
    let full_height = get_height(dataset);
    let band_count = get_count(dataset);
    if full_width <= 0 || full_height <= 0 || band_count <= 0 {
        return Err(error(
            NativeGisErrorCode::DemInvalidRaster,
            "栅格尺寸或波段数超出有效范围",
        ));
    }
    let proj_ptr = get_projection(dataset);
    let projection = if proj_ptr.is_null() {
        String::new()
    } else {
        CStr::from_ptr(proj_ptr).to_string_lossy().to_string()
    };
    let mut transform = [0.0f64; 6];
    if get_transform(dataset, transform.as_mut_ptr()) != 0
        || transform[1] <= 0.0
        || transform[5] >= 0.0
    {
        return Err(error(
            NativeGisErrorCode::DemInvalidRaster,
            "栅格缺少有效的北向仿射变换参数",
        ));
    }
    Ok((full_width, full_height, band_count, projection, transform))
}

#[allow(clippy::type_complexity)]
fn resolve_crop_window(
    runtime: &NativeGisRuntime,
    full_width: c_int,
    full_height: c_int,
    transform: &[f64; 6],
    projection: &str,
    crop_bbox: Option<[f64; 4]>,
) -> Result<(c_int, c_int, c_int, c_int, c_int, c_int, [f64; 6]), NativeGisError> {
    let (xoff, yoff, win_w, win_h, win_transform) = if let Some(bbox) = crop_bbox {
        if bbox[0] >= bbox[2] || bbox[1] >= bbox[3] {
            return Err(error(
                NativeGisErrorCode::DemReprojectionFailed,
                "裁剪窗口 bbox 无效 (需满足 west < east 且 south < north)",
            ));
        }
        let is_wgs84_bbox = bbox[0].abs() <= 180.0
            && bbox[2].abs() <= 180.0
            && bbox[1].abs() <= 90.0
            && bbox[3].abs() <= 90.0;
        let is_projected_raster = !projection.trim().is_empty()
            && (!is_wgs84_geographic_crs(projection)
                || transform[0].abs() > 180.0
                || transform[3].abs() > 90.0);
        let eff_bbox = if is_wgs84_bbox && is_projected_raster {
            if let Some(proj_bbox) =
                transform_bbox_with_proj(runtime, "EPSG:4326", projection, bbox)
            {
                proj_bbox
            } else if (projection.contains("Sinusoidal") || projection.contains("6371007"))
                && transform[0].abs() > 1000.0
            {
                const R: f64 = 6_371_007.181;
                let corners = [
                    (bbox[0], bbox[1]),
                    (bbox[2], bbox[1]),
                    (bbox[2], bbox[3]),
                    (bbox[0], bbox[3]),
                ];
                let mut min_x = f64::INFINITY;
                let mut max_x = f64::NEG_INFINITY;
                let mut min_y = f64::INFINITY;
                let mut max_y = f64::NEG_INFINITY;
                for (lon, lat) in corners {
                    let lat_rad = lat.to_radians();
                    let x = R * lon.to_radians() * lat_rad.cos();
                    let y = R * lat_rad;
                    min_x = min_x.min(x);
                    max_x = max_x.max(x);
                    min_y = min_y.min(y);
                    max_y = max_y.max(y);
                }
                [min_x, min_y, max_x, max_y]
            } else {
                bbox
            }
        } else {
            bbox
        };
        let col0 = ((eff_bbox[0] - transform[0]) / transform[1]).floor() as i64;
        let col1 = ((eff_bbox[2] - transform[0]) / transform[1]).ceil() as i64;
        let row0 = ((eff_bbox[3] - transform[3]) / transform[5]).floor() as i64;
        let row1 = ((eff_bbox[1] - transform[3]) / transform[5]).ceil() as i64;
        let x0 = col0.clamp(0, i64::from(full_width));
        let x1 = col1.clamp(0, i64::from(full_width));
        let y0 = row0.clamp(0, i64::from(full_height));
        let y1 = row1.clamp(0, i64::from(full_height));
        if x1 <= x0 || y1 <= y0 {
            return Err(error(
                NativeGisErrorCode::DemReprojectionFailed,
                "裁剪窗口与源栅格空间范围无交集",
            ));
        }
        let sub_transform = [
            transform[0] + (x0 as f64) * transform[1],
            transform[1],
            0.0,
            transform[3] + (y0 as f64) * transform[5],
            0.0,
            transform[5],
        ];
        (
            x0 as c_int,
            y0 as c_int,
            (x1 - x0) as c_int,
            (y1 - y0) as c_int,
            sub_transform,
        )
    } else {
        (0, 0, full_width, full_height, *transform)
    };

    let max_out_dim: i64 = 4096;
    let (out_w, out_h) = if i64::from(win_w) > max_out_dim || i64::from(win_h) > max_out_dim {
        let scale = (max_out_dim as f64) / (win_w.max(win_h) as f64);
        let ow = ((win_w as f64) * scale).round().max(1.0) as c_int;
        let oh = ((win_h as f64) * scale).round().max(1.0) as c_int;
        (ow.min(4096), oh.min(4096))
    } else {
        (win_w, win_h)
    };
    let mut out_transform = win_transform;
    if out_w != win_w {
        out_transform[1] = win_transform[1] * (f64::from(win_w) / f64::from(out_w));
    }
    if out_h != win_h {
        out_transform[5] = win_transform[5] * (f64::from(win_h) / f64::from(out_h));
    }
    Ok((xoff, yoff, win_w, win_h, out_w, out_h, out_transform))
}

#[allow(clippy::type_complexity)]
unsafe fn read_source_window_f32(
    runtime: &NativeGisRuntime,
    source: &str,
    band_index: c_int,
    crop_bbox: Option<[f64; 4]>,
) -> Result<(Vec<f32>, c_int, c_int, [f64; 6], String, Option<f64>, bool), NativeGisError> {
    let (guard, is_remote) = open_raster_dataset_ro(runtime, source)?;
    let (full_width, full_height, band_count, projection, transform) =
        match read_dataset_georef(runtime, guard.raw()) {
            Ok(v) => v,
            Err(err) => {
                let _ = configure_gdal_runtime_for_source(runtime, false);
                return Err(err);
            }
        };
    if band_index < 1 || band_index > band_count {
        let _ = configure_gdal_runtime_for_source(runtime, false);
        return Err(error(
            NativeGisErrorCode::DemInvalidRaster,
            "请求波段索引或栅格尺寸超出有效范围",
        ));
    }
    let (xoff, yoff, win_w, win_h, out_w, out_h, out_transform) = match resolve_crop_window(
        runtime,
        full_width,
        full_height,
        &transform,
        &projection,
        crop_bbox,
    ) {
        Ok(v) => v,
        Err(err) => {
            let _ = configure_gdal_runtime_for_source(runtime, false);
            return Err(err);
        }
    };

    let get_band = *runtime.gdal_symbol::<GdalGetRasterBand>(b"GDALGetRasterBand\0")?;
    let get_nodata =
        *runtime.gdal_symbol::<GdalGetRasterNoDataValue>(b"GDALGetRasterNoDataValue\0")?;
    let raster_io = *runtime.gdal_symbol::<GdalRasterIo>(b"GDALRasterIO\0")?;
    let band = get_band(guard.raw(), band_index);
    if band.is_null() {
        let _ = configure_gdal_runtime_for_source(runtime, false);
        return Err(error(
            NativeGisErrorCode::DemInvalidRaster,
            "无法获取栅格波段句柄",
        ));
    }
    let mut has_nodata = 0;
    let nodata_val = get_nodata(band, &mut has_nodata);
    let nodata = (has_nodata != 0).then_some(nodata_val);

    let mut buffer = vec![0.0f32; (out_w as usize) * (out_h as usize)];
    let status = raster_io(
        band,
        GDAL_READ,
        xoff,
        yoff,
        win_w,
        win_h,
        buffer.as_mut_ptr().cast(),
        out_w,
        out_h,
        GDAL_FLOAT32,
        0,
        0,
    );
    let _ = configure_gdal_runtime_for_source(runtime, false);
    if status != 0 {
        return Err(error(
            NativeGisErrorCode::DemInvalidRaster,
            "读取栅格窗口像元失败 (GDALRasterIO 错误)",
        ));
    }
    Ok((
        buffer,
        out_w,
        out_h,
        out_transform,
        projection,
        nodata,
        is_remote,
    ))
}

#[allow(clippy::too_many_arguments)]
fn compute_terrain_derivative(
    values: &[f32],
    width: usize,
    height: usize,
    transform: &[f64; 6],
    projection: &str,
    nodata: Option<f64>,
    operation: &str,
    azimuth_deg: f64,
    altitude_deg: f64,
    z_factor: f64,
) -> (Vec<f32>, f32) {
    let out_nodata: f32 = -9999.0;
    let mut out = vec![out_nodata; width * height];
    let is_valid = |v: f32| -> bool {
        v.is_finite()
            && !nodata.is_some_and(|nd| {
                if nd.is_nan() {
                    v.is_nan()
                } else {
                    (f64::from(v) - nd).abs() <= 1e-5
                }
            })
    };
    let is_geographic = is_wgs84_geographic_crs(projection)
        || projection.to_ascii_uppercase().contains("UNIT[\"DEGREE\"")
        || (transform[1].abs() < 0.05 && transform[0].abs() <= 180.0 && transform[3].abs() <= 90.0);

    let az_rad = azimuth_deg.to_radians();
    let alt_rad = altitude_deg.to_radians();
    let zenith_rad = std::f64::consts::FRAC_PI_2 - alt_rad;

    for r in 0..height {
        let lat = transform[3] + (r as f64 + 0.5) * transform[5];
        let (dx, dy) = if is_geographic {
            let cos_lat = lat.to_radians().cos().abs().max(0.05);
            (
                transform[1].abs() * 111_320.0 * cos_lat,
                transform[5].abs() * 111_320.0,
            )
        } else {
            (transform[1].abs().max(1e-6), transform[5].abs().max(1e-6))
        };

        for c in 0..width {
            let center = values[r * width + c];
            if !is_valid(center) {
                continue;
            }
            let r0 = r.saturating_sub(1);
            let r2 = (r + 1).min(height - 1);
            let c0 = c.saturating_sub(1);
            let c2 = (c + 1).min(width - 1);
            let sample = |rr: usize, cc: usize| -> f64 {
                let v = values[rr * width + cc];
                if is_valid(v) {
                    f64::from(v) * z_factor
                } else {
                    f64::from(center) * z_factor
                }
            };
            let z1 = sample(r0, c0);
            let z2 = sample(r0, c);
            let z3 = sample(r0, c2);
            let z4 = sample(r, c0);
            let z5 = sample(r, c);
            let z6 = sample(r, c2);
            let z7 = sample(r2, c0);
            let z8 = sample(r2, c);
            let z9 = sample(r2, c2);

            let dz_dx = ((z3 + 2.0 * z6 + z9) - (z1 + 2.0 * z4 + z7)) / (8.0 * dx);
            let dz_dy = ((z7 + 2.0 * z8 + z9) - (z1 + 2.0 * z2 + z3)) / (8.0 * dy);
            let slope_rad = (dz_dx * dz_dx + dz_dy * dz_dy).sqrt().atan();

            let result_val = match operation {
                "slope" => slope_rad.to_degrees() as f32,
                "aspect" => {
                    if dz_dx.abs() < 1e-12 && dz_dy.abs() < 1e-12 {
                        -1.0
                    } else {
                        let mut deg = dz_dy.atan2(-dz_dx).to_degrees();
                        if deg < 0.0 {
                            deg += 360.0;
                        }
                        let mut compass = 90.0 - deg;
                        if compass < 0.0 {
                            compass += 360.0;
                        }
                        compass as f32
                    }
                }
                "hillshade" => {
                    let aspect_rad = if dz_dx.abs() < 1e-12 && dz_dy.abs() < 1e-12 {
                        0.0
                    } else {
                        let mut a = std::f64::consts::PI * 0.5 - dz_dy.atan2(-dz_dx);
                        if a < 0.0 {
                            a += 2.0 * std::f64::consts::PI;
                        }
                        a
                    };
                    let hs = 255.0
                        * ((zenith_rad.cos() * slope_rad.cos())
                            + (zenith_rad.sin() * slope_rad.sin() * (az_rad - aspect_rad).cos()));
                    hs.clamp(0.0, 255.0) as f32
                }
                "tpi" => {
                    let mean_nbr = (z1 + z2 + z3 + z4 + z6 + z7 + z8 + z9) / 8.0;
                    (z5 - mean_nbr) as f32
                }
                "tri" => {
                    let sum_abs = (z1 - z5).abs()
                        + (z2 - z5).abs()
                        + (z3 - z5).abs()
                        + (z4 - z5).abs()
                        + (z6 - z5).abs()
                        + (z7 - z5).abs()
                        + (z8 - z5).abs()
                        + (z9 - z5).abs();
                    (sum_abs / 8.0) as f32
                }
                "roughness" => {
                    let nbrs = [z1, z2, z3, z4, z5, z6, z7, z8, z9];
                    let min_z = nbrs.iter().copied().fold(f64::INFINITY, f64::min);
                    let max_z = nbrs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                    (max_z - min_z) as f32
                }
                _ => center,
            };
            out[r * width + c] = result_val;
        }
    }
    (out, out_nodata)
}

#[allow(clippy::type_complexity)]
unsafe fn collect_raster_band_stats_f64(
    runtime: &NativeGisRuntime,
    band: RasterBand,
    width: c_int,
    height: c_int,
    nodata: Option<f64>,
) -> Result<(usize, usize, f64, f64, f64), NativeGisError> {
    let raster_io = *runtime.gdal_symbol::<GdalRasterIo>(b"GDALRasterIO\0")?;
    let total_pixels = (width as usize).saturating_mul(height as usize);
    let mut min_val = f64::INFINITY;
    let mut max_val = f64::NEG_INFINITY;
    let mut sum_val = 0.0f64;
    let mut valid_count = 0usize;
    let chunk_rows = 512.min(height).max(1);
    let mut row_buf = vec![0.0f64; (width as usize).saturating_mul(chunk_rows as usize)];
    let mut y = 0;
    while y < height {
        let rows = (height - y).min(chunk_rows);
        let len = (width as usize).saturating_mul(rows as usize);
        if raster_io(
            band,
            GDAL_READ,
            0,
            y,
            width,
            rows,
            row_buf[..len].as_mut_ptr().cast(),
            width,
            rows,
            GDAL_FLOAT64,
            0,
            0,
        ) != 0
        {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                "读取输出栅格波段统计像元失败",
            ));
        }
        for &vf in &row_buf[..len] {
            if is_valid_nodata(vf, nodata) {
                min_val = min_val.min(vf);
                max_val = max_val.max(vf);
                sum_val += vf;
                valid_count += 1;
            }
        }
        y += rows;
    }
    Ok((total_pixels, valid_count, min_val, max_val, sum_val))
}

fn execute_exact_dtype_clip_or_translate(
    runtime: &NativeGisRuntime,
    request: &RasterProcessRequest,
    op: &str,
    out_fmt: &str,
) -> Result<Value, NativeGisError> {
    let output = &request.output_path;
    let (staging, staging_file) = StagingDirGuard::create(output)?;
    let _proj_ctx = runtime.create_proj_context()?;

    let (width, height, band_count, dtypes, projection, out_transform, nodata, is_remote, stats) = unsafe {
        let (src_guard, is_remote) = open_raster_dataset_ro(runtime, &request.sources[0])?;
        let (full_width, full_height, src_band_count, projection, transform) =
            match read_dataset_georef(runtime, src_guard.raw()) {
                Ok(v) => v,
                Err(err) => {
                    let _ = configure_gdal_runtime_for_source(runtime, false);
                    return Err(err);
                }
            };
        let (xoff, yoff, win_w, win_h, out_w, out_h, mut out_transform) = match resolve_crop_window(
            runtime,
            full_width,
            full_height,
            &transform,
            &projection,
            request.crop_bbox,
        ) {
            Ok(v) => v,
            Err(err) => {
                let _ = configure_gdal_runtime_for_source(runtime, false);
                return Err(err);
            }
        };

        let selected_bands: Vec<c_int> = if let Some(b) = request.band_a {
            if b == 0 || b > src_band_count as usize {
                let _ = configure_gdal_runtime_for_source(runtime, false);
                return Err(error(
                    NativeGisErrorCode::DemInvalidRaster,
                    "请求波段索引超出源栅格波段范围",
                ));
            }
            vec![b as c_int]
        } else {
            (1..=src_band_count).collect()
        };

        let get_band = *runtime.gdal_symbol::<GdalGetRasterBand>(b"GDALGetRasterBand\0")?;
        let get_type = *runtime.gdal_symbol::<GdalGetRasterDataType>(b"GDALGetRasterDataType\0")?;
        let get_nodata =
            *runtime.gdal_symbol::<GdalGetRasterNoDataValue>(b"GDALGetRasterNoDataValue\0")?;
        let raster_io = *runtime.gdal_symbol::<GdalRasterIo>(b"GDALRasterIO\0")?;

        let mut band_infos = Vec::with_capacity(selected_bands.len());
        let mut common_dtype: Option<c_int> = None;
        for &band_idx in &selected_bands {
            let band = get_band(src_guard.raw(), band_idx);
            if band.is_null() {
                let _ = configure_gdal_runtime_for_source(runtime, false);
                return Err(error(
                    NativeGisErrorCode::DemInvalidRaster,
                    "无法获取源栅格波段句柄",
                ));
            }
            let dt = get_type(band);
            if gdal_dtype_byte_size(dt).is_none() {
                let _ = configure_gdal_runtime_for_source(runtime, false);
                return Err(error(
                    NativeGisErrorCode::DemInvalidRaster,
                    format!("暂不支持无损复制的像元类型: {}", gdal_data_type(dt)),
                ));
            }
            if let Some(prev) = common_dtype {
                if prev != dt {
                    let _ = configure_gdal_runtime_for_source(runtime, false);
                    return Err(error(
                        NativeGisErrorCode::DemInvalidRaster,
                        "源栅格各波段像元数据类型不一致，请通过 band_a 指定单波段或关闭 preserve_dtype",
                    ));
                }
            } else {
                common_dtype = Some(dt);
            }
            let mut has_nd = 0;
            let nd_val = get_nodata(band, &mut has_nd);
            band_infos.push((band_idx, band, (has_nd != 0).then_some(nd_val)));
        }
        let dtype_code = common_dtype.ok_or_else(|| {
            let _ = configure_gdal_runtime_for_source(runtime, false);
            error(NativeGisErrorCode::DemInvalidRaster, "未找到可用栅格波段")
        })?;
        let byte_size = gdal_dtype_byte_size(dtype_code).expect("validated dtype");
        let pixel_count = (out_w as usize).saturating_mul(out_h as usize);
        let mut raw_bands: Vec<(Option<f64>, Vec<u8>)> = Vec::with_capacity(band_infos.len());
        for (_, band, nd) in &band_infos {
            let mut raw = vec![0u8; pixel_count.saturating_mul(byte_size)];
            let status = raster_io(
                *band,
                GDAL_READ,
                xoff,
                yoff,
                win_w,
                win_h,
                raw.as_mut_ptr().cast(),
                out_w,
                out_h,
                dtype_code,
                0,
                0,
            );
            if status != 0 {
                let _ = configure_gdal_runtime_for_source(runtime, false);
                return Err(error(
                    NativeGisErrorCode::DemInvalidRaster,
                    "读取栅格窗口原始像元失败 (GDALRasterIO 错误)",
                ));
            }
            raw_bands.push((*nd, raw));
        }
        drop(src_guard);
        let _ = configure_gdal_runtime_for_source(runtime, false);

        let drv_c = CString::new(out_fmt).expect("static GDAL driver");
        let get_driver = *runtime.gdal_symbol::<GdalGetDriverByName>(b"GDALGetDriverByName\0")?;
        let driver = get_driver(drv_c.as_ptr());
        if driver.is_null() {
            return Err(error(
                NativeGisErrorCode::GisDriverUnavailable,
                format!("产品 GDAL runtime 缺少 {out_fmt} 驱动"),
            ));
        }
        let create = *runtime.gdal_symbol::<GdalCreate>(b"GDALCreate\0")?;
        let close = *runtime.gdal_symbol::<GdalClose>(b"GDALClose\0")?;
        let staging_c = c_path(
            &staging_file,
            NativeGisErrorCode::DemReprojectionFailed,
            "栅格处理输出路径编码无效",
        )?;
        let out_ds = create(
            driver,
            staging_c.as_ptr(),
            out_w,
            out_h,
            raw_bands.len() as c_int,
            dtype_code,
            std::ptr::null(),
        );
        if out_ds.is_null() {
            type CplLastErrFn = unsafe extern "C" fn() -> *const c_char;
            let detail = runtime
                .gdal_symbol::<CplLastErrFn>(b"CPLGetLastErrorMsg\0")
                .map(|f| CStr::from_ptr((*f)()).to_string_lossy().into_owned())
                .unwrap_or_default();
            return Err(error(
                NativeGisErrorCode::DemReprojectionFailed,
                format!("GDAL 无法创建输出 {out_fmt} 栅格 ({detail})"),
            ));
        }
        let out_guard = GdalDatasetGuard::new(out_ds, close);
        if !projection.is_empty() {
            if let Ok(proj_c) = CString::new(projection.as_str()) {
                let set_proj = *runtime.gdal_symbol::<GdalSetProjection>(b"GDALSetProjection\0")?;
                let _ = set_proj(out_guard.raw(), proj_c.as_ptr());
            }
        }
        let set_tf = *runtime.gdal_symbol::<GdalSetGeoTransform>(b"GDALSetGeoTransform\0")?;
        let _ = set_tf(out_guard.raw(), out_transform.as_mut_ptr());
        let set_nd =
            *runtime.gdal_symbol::<GdalSetRasterNoDataValue>(b"GDALSetRasterNoDataValue\0")?;

        let mut band1_stats = (0usize, 0usize, f64::INFINITY, f64::NEG_INFINITY, 0.0f64);
        for (idx, (nd, raw)) in raw_bands.iter().enumerate() {
            let dst_band = get_band(out_guard.raw(), (idx + 1) as c_int);
            if dst_band.is_null() {
                return Err(error(
                    NativeGisErrorCode::DemReprojectionFailed,
                    "无法获取输出栅格波段",
                ));
            }
            if let Some(nd_val) = nd {
                let _ = set_nd(dst_band, *nd_val);
            }
            let mut writable = raw.clone();
            if raster_io(
                dst_band,
                GDAL_WRITE,
                0,
                0,
                out_w,
                out_h,
                writable.as_mut_ptr().cast(),
                out_w,
                out_h,
                dtype_code,
                0,
                0,
            ) != 0
            {
                return Err(error(
                    NativeGisErrorCode::DemReprojectionFailed,
                    "写入输出栅格原始像元失败",
                ));
            }
            if idx == 0 {
                let mut min_v = f64::INFINITY;
                let mut max_v = f64::NEG_INFINITY;
                let mut sum_v = 0.0f64;
                let mut valid_c = 0usize;
                for chunk in raw.chunks_exact(byte_size) {
                    if let Some(vf) = read_typed_sample_as_f64(dtype_code, chunk) {
                        if is_valid_nodata(vf, *nd) {
                            min_v = min_v.min(vf);
                            max_v = max_v.max(vf);
                            sum_v += vf;
                            valid_c += 1;
                        }
                    }
                }
                band1_stats = (pixel_count, valid_c, min_v, max_v, sum_v);
            }
        }
        let flush = *runtime.gdal_symbol::<GdalFlushCache>(b"GDALFlushCache\0")?;
        flush(out_guard.raw());
        drop(out_guard);

        let dtype_label = gdal_data_type(dtype_code);
        let dtypes = vec![dtype_label; raw_bands.len()];
        let band1_nodata = raw_bands.first().and_then(|(nd, _)| *nd);
        (
            out_w,
            out_h,
            raw_bands.len(),
            dtypes,
            projection,
            out_transform,
            band1_nodata,
            is_remote,
            band1_stats,
        )
    };

    staging.promote_outputs(&staging_file, output, out_fmt)?;
    let bytes_written = fs::metadata(output).map(|m| m.len()).unwrap_or(0);
    let west = out_transform[0];
    let north = out_transform[3];
    let east = west + out_transform[1] * f64::from(width);
    let south = north + out_transform[5] * f64::from(height);
    let (total_px, valid_cnt, min_v, max_v, sum_v) = stats;

    Ok(json!({
        "ok": true,
        "executed": true,
        "operation": op,
        "sources": request.sources,
        "is_remote_stream": is_remote,
        "output_path": output,
        "driver": out_fmt,
        "width": width,
        "height": height,
        "band_count": band_count,
        "dtype": dtypes,
        "crs": (!projection.is_empty()).then_some(projection),
        "bounds": [west.min(east), south.min(north), west.max(east), south.max(north)],
        "geotransform": out_transform,
        "nodata": nodata,
        "bytes_written": bytes_written,
        "statistics": build_statistics_json(total_px, valid_cnt, min_v, max_v, sum_v),
    }))
}

fn execute_gdal_warp_operation(
    runtime: &NativeGisRuntime,
    request: &RasterProcessRequest,
    op: &str,
    out_fmt: &str,
) -> Result<Value, NativeGisError> {
    let target_crs = request
        .target_crs
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);
    if op == "reproject" && target_crs.is_none() {
        return Err(error(
            NativeGisErrorCode::DemReprojectionFailed,
            "reproject 操作必须提供非空的 target_crs (例如 EPSG:32650)",
        ));
    }
    let resampling = normalize_warp_resampling(request.resampling.as_deref())?;
    if let Some(bbox) = request.crop_bbox {
        if bbox[0] >= bbox[2] || bbox[1] >= bbox[3] {
            return Err(error(
                NativeGisErrorCode::DemReprojectionFailed,
                "裁剪窗口 bbox 无效 (需满足 west < east 且 south < north)",
            ));
        }
    }

    let output = &request.output_path;
    let (staging, staging_file) = StagingDirGuard::create(output)?;
    let _proj_ctx = runtime.create_proj_context()?;

    let is_remote = unsafe {
        let mut src_guards = Vec::with_capacity(request.sources.len());
        let mut any_remote = false;
        let get_band = *runtime.gdal_symbol::<GdalGetRasterBand>(b"GDALGetRasterBand\0")?;
        let get_type = *runtime.gdal_symbol::<GdalGetRasterDataType>(b"GDALGetRasterDataType\0")?;
        let mut ref_band_count: Option<c_int> = None;
        let mut ref_dtypes: Option<Vec<c_int>> = None;

        for source in &request.sources {
            let (guard, rem) = open_raster_dataset_ro(runtime, source)?;
            any_remote |= rem;
            let (_, _, band_count, crs, _) = match read_dataset_georef(runtime, guard.raw()) {
                Ok(v) => v,
                Err(err) => {
                    let _ = configure_gdal_runtime_for_source(runtime, false);
                    return Err(err);
                }
            };
            if op == "reproject" && crs.trim().is_empty() {
                let _ = configure_gdal_runtime_for_source(runtime, false);
                return Err(error(
                    NativeGisErrorCode::DemReprojectionFailed,
                    "reproject 源栅格缺少坐标系定义 (CRS)",
                ));
            }
            let mut dtypes = Vec::with_capacity(band_count as usize);
            for b in 1..=band_count {
                let band = get_band(guard.raw(), b);
                if band.is_null() {
                    let _ = configure_gdal_runtime_for_source(runtime, false);
                    return Err(error(
                        NativeGisErrorCode::DemInvalidRaster,
                        "无法读取输入栅格波段句柄",
                    ));
                }
                dtypes.push(get_type(band));
            }
            if let (Some(expected_cnt), Some(expected_dts)) = (ref_band_count, ref_dtypes.as_ref())
            {
                if band_count != expected_cnt {
                    let _ = configure_gdal_runtime_for_source(runtime, false);
                    return Err(error(
                        NativeGisErrorCode::DemInvalidRaster,
                        format!(
                            "镶嵌输入栅格波段数不一致: 期望 {expected_cnt}，检测到 {band_count}"
                        ),
                    ));
                }
                if &dtypes != expected_dts {
                    let _ = configure_gdal_runtime_for_source(runtime, false);
                    return Err(error(
                        NativeGisErrorCode::DemInvalidRaster,
                        "镶嵌输入栅格像元数据类型 (dtype) 不一致，无法直接镶嵌",
                    ));
                }
            } else {
                ref_band_count = Some(band_count);
                ref_dtypes = Some(dtypes);
            }
            src_guards.push(guard);
        }

        configure_gdal_runtime_for_source(runtime, any_remote)?;
        let options_new =
            *runtime.gdal_symbol::<GdalWarpAppOptionsNew>(b"GDALWarpAppOptionsNew\0")?;
        let options_free =
            *runtime.gdal_symbol::<GdalWarpAppOptionsFree>(b"GDALWarpAppOptionsFree\0")?;
        let warp = *runtime.gdal_symbol::<GdalWarp>(b"GDALWarp\0")?;
        let close = *runtime.gdal_symbol::<GdalClose>(b"GDALClose\0")?;
        let flush = *runtime.gdal_symbol::<GdalFlushCache>(b"GDALFlushCache\0")?;

        let mut args = vec![
            "-of".to_owned(),
            out_fmt.to_owned(),
            "-r".to_owned(),
            resampling.to_owned(),
            "-overwrite".to_owned(),
        ];
        if out_fmt == "GTiff" {
            args.push("-co".to_owned());
            args.push("COMPRESS=LZW".to_owned());
        }
        if let Some(ref tcrs) = target_crs {
            args.push("-t_srs".to_owned());
            args.push(tcrs.clone());
        }
        if let Some(bbox) = request.crop_bbox {
            let is_wgs84_bbox = bbox[0].abs() <= 180.0
                && bbox[2].abs() <= 180.0
                && bbox[1].abs() <= 90.0
                && bbox[3].abs() <= 90.0;
            if is_wgs84_bbox {
                args.push("-te_srs".to_owned());
                args.push("EPSG:4326".to_owned());
            }
            args.push("-te".to_owned());
            args.push(bbox[0].to_string());
            args.push(bbox[1].to_string());
            args.push(bbox[2].to_string());
            args.push(bbox[3].to_string());
        }

        let arg_cstrings = args
            .into_iter()
            .map(|v| {
                CString::new(v).map_err(|_| {
                    error(
                        NativeGisErrorCode::DemReprojectionFailed,
                        "GDALWarp 参数包含无效空字符",
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut arg_ptrs = arg_cstrings
            .iter()
            .map(|c| c.as_ptr().cast_mut())
            .collect::<Vec<_>>();
        arg_ptrs.push(std::ptr::null_mut());

        let warp_options = options_new(arg_ptrs.as_mut_ptr(), std::ptr::null_mut());
        if warp_options.is_null() {
            let _ = configure_gdal_runtime_for_source(runtime, false);
            return Err(error(
                NativeGisErrorCode::DemReprojectionFailed,
                "GDALWarp 无法初始化转换选项",
            ));
        }
        let warp_opts_guard = GdalWarpOptionsGuard::new(warp_options, options_free);
        let staging_c = c_path(
            &staging_file,
            NativeGisErrorCode::DemReprojectionFailed,
            "GDALWarp 输出路径编码无效",
        )?;
        let mut src_ptrs = src_guards
            .iter()
            .map(GdalDatasetGuard::raw)
            .collect::<Vec<_>>();
        let mut usage_error: c_int = 0;
        let out_ds = warp(
            staging_c.as_ptr(),
            std::ptr::null_mut(),
            src_ptrs.len() as c_int,
            src_ptrs.as_mut_ptr(),
            warp_opts_guard.raw(),
            &mut usage_error,
        );
        let _ = configure_gdal_runtime_for_source(runtime, false);
        if usage_error != 0 || out_ds.is_null() {
            return Err(error(
                NativeGisErrorCode::DemReprojectionFailed,
                format!("GDALWarp 执行 {op} 失败 (usage_error={usage_error})"),
            ));
        }
        let out_guard = GdalDatasetGuard::new(out_ds, close);
        flush(out_guard.raw());
        drop(out_guard);
        drop(src_guards);
        any_remote
    };

    staging.promote_outputs(&staging_file, output, out_fmt)?;

    let (width, height, band_count, dtypes, projection, transform, nodata, stats) = unsafe {
        let output_str = output.to_str().ok_or_else(|| {
            error(
                NativeGisErrorCode::DemReprojectionFailed,
                "输出路径包含非 UTF-8 字符",
            )
        })?;
        let (out_guard, _) = open_raster_dataset_ro(runtime, output_str)?;
        let (width, height, band_count, projection, transform) =
            read_dataset_georef(runtime, out_guard.raw())?;
        let get_band = *runtime.gdal_symbol::<GdalGetRasterBand>(b"GDALGetRasterBand\0")?;
        let get_type = *runtime.gdal_symbol::<GdalGetRasterDataType>(b"GDALGetRasterDataType\0")?;
        let get_nodata =
            *runtime.gdal_symbol::<GdalGetRasterNoDataValue>(b"GDALGetRasterNoDataValue\0")?;
        let mut dtypes = Vec::with_capacity(band_count as usize);
        let mut band1 = std::ptr::null_mut();
        let mut nodata = None;
        for b in 1..=band_count {
            let band = get_band(out_guard.raw(), b);
            if band.is_null() {
                return Err(error(
                    NativeGisErrorCode::DemInvalidRaster,
                    "输出栅格波段不可读取",
                ));
            }
            dtypes.push(gdal_data_type(get_type(band)));
            if b == 1 {
                band1 = band;
                let mut has_nd = 0;
                let nd = get_nodata(band, &mut has_nd);
                if has_nd != 0 {
                    nodata = Some(nd);
                }
            }
        }
        let stats = collect_raster_band_stats_f64(runtime, band1, width, height, nodata)?;
        let _ = configure_gdal_runtime_for_source(runtime, false);
        (
            width,
            height,
            band_count as usize,
            dtypes,
            projection,
            transform,
            nodata,
            stats,
        )
    };

    let bytes_written = fs::metadata(output).map(|m| m.len()).unwrap_or(0);
    let west = transform[0];
    let north = transform[3];
    let east = west + transform[1] * f64::from(width);
    let south = north + transform[5] * f64::from(height);
    let (total_px, valid_cnt, min_v, max_v, sum_v) = stats;

    Ok(json!({
        "ok": true,
        "executed": true,
        "operation": op,
        "sources": request.sources,
        "is_remote_stream": is_remote,
        "output_path": output,
        "driver": out_fmt,
        "width": width,
        "height": height,
        "band_count": band_count,
        "dtype": dtypes,
        "crs": (!projection.is_empty()).then_some(projection),
        "bounds": [west.min(east), south.min(north), west.max(east), south.max(north)],
        "geotransform": transform,
        "nodata": nodata,
        "bytes_written": bytes_written,
        "statistics": build_statistics_json(total_px, valid_cnt, min_v, max_v, sum_v),
    }))
}

pub(crate) fn process_raster_window(
    runtime: &NativeGisRuntime,
    request: &RasterProcessRequest,
) -> Result<Value, NativeGisError> {
    if request.sources.is_empty() {
        return Err(error(
            NativeGisErrorCode::DemInvalidRaster,
            "至少需要提供一个栅格数据源路径或远程 COG URL",
        ));
    }
    let op = request.operation.trim().to_ascii_lowercase();
    let supported_ops = [
        "clip",
        "translate",
        "reproject",
        "mosaic",
        "slope",
        "aspect",
        "hillshade",
        "tpi",
        "tri",
        "roughness",
        "ndvi",
        "ndwi",
        "nbr",
        "band_math",
    ];
    if !supported_ops.contains(&op.as_str()) {
        return Err(error(
            NativeGisErrorCode::DemReprojectionFailed,
            format!("不支持的栅格处理算子: {op}"),
        ));
    }

    let output = &request.output_path;
    let out_fmt = normalize_raster_output_format(request.output_format.as_deref(), output)?;

    if matches!(op.as_str(), "reproject" | "mosaic") {
        return execute_gdal_warp_operation(runtime, request, &op, out_fmt);
    }
    if matches!(op.as_str(), "clip" | "translate") && request.preserve_dtype.unwrap_or(true) {
        return execute_exact_dtype_clip_or_translate(runtime, request, &op, out_fmt);
    }

    let (staging, staging_file) = StagingDirGuard::create(output)?;

    let (out_values, width, height, mut out_transform, projection, out_nodata, is_remote) = unsafe {
        if matches!(op.as_str(), "ndvi" | "ndwi" | "nbr" | "band_math") {
            let (band_a_vals, w_a, h_a, tf_a, proj_a, nd_a, rem_a) = read_source_window_f32(
                runtime,
                &request.sources[0],
                request.band_a.unwrap_or(1) as c_int,
                request.crop_bbox,
            )?;
            let (band_b_vals, w_b, h_b, _, _, nd_b, rem_b) = if request.sources.len() >= 2 {
                read_source_window_f32(
                    runtime,
                    &request.sources[1],
                    request.band_b.unwrap_or(1) as c_int,
                    request.crop_bbox,
                )?
            } else {
                read_source_window_f32(
                    runtime,
                    &request.sources[0],
                    request.band_b.unwrap_or(2) as c_int,
                    request.crop_bbox,
                )?
            };
            if w_a != w_b || h_a != h_b {
                return Err(error(
                    NativeGisErrorCode::DemInvalidRaster,
                    "双波段栅格计算的两个输入窗口尺寸不一致",
                ));
            }
            let out_nd: f32 = -9999.0;
            let mut result_buf = vec![out_nd; (w_a as usize) * (h_a as usize)];
            for i in 0..result_buf.len() {
                let a = band_a_vals[i];
                let b = band_b_vals[i];
                let valid_a =
                    a.is_finite() && !nd_a.is_some_and(|nd| (f64::from(a) - nd).abs() <= 1e-5);
                let valid_b =
                    b.is_finite() && !nd_b.is_some_and(|nd| (f64::from(b) - nd).abs() <= 1e-5);
                if !valid_a || !valid_b {
                    continue;
                }
                let denom = a + b;
                if denom.abs() > 1e-6 {
                    result_buf[i] = ((a - b) / denom).clamp(-1.0, 1.0);
                }
            }
            (
                result_buf,
                w_a,
                h_a,
                tf_a,
                proj_a,
                Some(f64::from(out_nd)),
                rem_a || rem_b,
            )
        } else {
            let (vals, w, h, tf, proj, nd, rem) = read_source_window_f32(
                runtime,
                &request.sources[0],
                request.band_a.unwrap_or(1) as c_int,
                request.crop_bbox,
            )?;
            if matches!(op.as_str(), "clip" | "translate") {
                (vals, w, h, tf, proj, nd, rem)
            } else {
                let (derived, derived_nd) = compute_terrain_derivative(
                    &vals,
                    w as usize,
                    h as usize,
                    &tf,
                    &proj,
                    nd,
                    &op,
                    request.azimuth_deg.unwrap_or(315.0),
                    request.altitude_deg.unwrap_or(45.0),
                    request.z_factor.unwrap_or(1.0),
                );
                (derived, w, h, tf, proj, Some(f64::from(derived_nd)), rem)
            }
        }
    };

    let write_res = unsafe {
        configure_gdal_runtime_for_source(runtime, false)?;
        let driver_name = CString::new(out_fmt).expect("static GDAL driver");
        let get_driver = *runtime.gdal_symbol::<GdalGetDriverByName>(b"GDALGetDriverByName\0")?;
        let driver = get_driver(driver_name.as_ptr());
        if driver.is_null() {
            return Err(error(
                NativeGisErrorCode::GisDriverUnavailable,
                format!("产品 GDAL runtime 缺少 {out_fmt} 驱动"),
            ));
        }
        let create = *runtime.gdal_symbol::<GdalCreate>(b"GDALCreate\0")?;
        let close = *runtime.gdal_symbol::<GdalClose>(b"GDALClose\0")?;
        let output_c = c_path(
            &staging_file,
            NativeGisErrorCode::DemReprojectionFailed,
            "栅格处理输出路径编码无效",
        )?;
        let ds = create(
            driver,
            output_c.as_ptr(),
            width,
            height,
            1,
            GDAL_FLOAT32,
            std::ptr::null(),
        );
        if ds.is_null() {
            return Err(error(
                NativeGisErrorCode::DemReprojectionFailed,
                "GDAL 无法创建输出 GeoTIFF",
            ));
        }
        let ds_guard = GdalDatasetGuard::new(ds, close);
        if !projection.is_empty() {
            if let Ok(proj_c) = CString::new(projection.as_str()) {
                let set_proj = *runtime.gdal_symbol::<GdalSetProjection>(b"GDALSetProjection\0")?;
                let _ = set_proj(ds_guard.raw(), proj_c.as_ptr());
            }
        }
        let set_tf = *runtime.gdal_symbol::<GdalSetGeoTransform>(b"GDALSetGeoTransform\0")?;
        let _ = set_tf(ds_guard.raw(), out_transform.as_mut_ptr());
        let get_band = *runtime.gdal_symbol::<GdalGetRasterBand>(b"GDALGetRasterBand\0")?;
        let band = get_band(ds_guard.raw(), 1);
        if band.is_null() {
            return Err(error(
                NativeGisErrorCode::DemReprojectionFailed,
                "无法获取输出 GeoTIFF 波段",
            ));
        }
        if let Some(nd) = out_nodata {
            let set_nd =
                *runtime.gdal_symbol::<GdalSetRasterNoDataValue>(b"GDALSetRasterNoDataValue\0")?;
            let _ = set_nd(band, nd);
        }
        let raster_io = *runtime.gdal_symbol::<GdalRasterIo>(b"GDALRasterIO\0")?;
        let mut writable = out_values.clone();
        if raster_io(
            band,
            GDAL_WRITE,
            0,
            0,
            width,
            height,
            writable.as_mut_ptr().cast(),
            width,
            height,
            GDAL_FLOAT32,
            0,
            0,
        ) != 0
        {
            return Err(error(
                NativeGisErrorCode::DemReprojectionFailed,
                "写入输出 GeoTIFF 像元失败",
            ));
        }
        let flush = *runtime.gdal_symbol::<GdalFlushCache>(b"GDALFlushCache\0")?;
        flush(ds_guard.raw());
        Ok::<_, NativeGisError>(())
    };
    write_res?;
    staging.promote_outputs(&staging_file, output, out_fmt)?;

    let mut min_val = f64::INFINITY;
    let mut max_val = f64::NEG_INFINITY;
    let mut sum_val = 0.0f64;
    let mut valid_count = 0usize;
    for &v in &out_values {
        let vf = f64::from(v);
        if is_valid_nodata(vf, out_nodata) {
            min_val = min_val.min(vf);
            max_val = max_val.max(vf);
            sum_val += vf;
            valid_count += 1;
        }
    }
    let bytes_written = fs::metadata(output).map(|m| m.len()).unwrap_or(0);
    let west = out_transform[0];
    let north = out_transform[3];
    let east = west + out_transform[1] * f64::from(width);
    let south = north + out_transform[5] * f64::from(height);

    Ok(json!({
        "ok": true,
        "executed": true,
        "operation": op,
        "sources": request.sources,
        "is_remote_stream": is_remote,
        "output_path": output,
        "driver": out_fmt,
        "width": width,
        "height": height,
        "band_count": 1,
        "dtype": ["float32"],
        "crs": (!projection.is_empty()).then_some(projection),
        "bounds": [west.min(east), south.min(north), west.max(east), south.max(north)],
        "geotransform": out_transform,
        "nodata": out_nodata,
        "bytes_written": bytes_written,
        "statistics": build_statistics_json(out_values.len(), valid_count, min_val, max_val, sum_val),
    }))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn stitch_xyz_tiles_to_geotiff(
    runtime: &NativeGisRuntime,
    tiles: &[(u32, u32, PathBuf)],
    x_min: u32,
    x_max: u32,
    y_min: u32,
    y_max: u32,
    geotransform: [f64; 6],
    projection_wkt: &str,
    output_path: &Path,
    compression: &str,
    build_pyramids: bool,
    export_format: &str,
) -> Result<Value, NativeGisError> {
    if tiles.is_empty() || x_max < x_min || y_max < y_min {
        return Err(error(
            NativeGisErrorCode::DemInvalidRaster,
            "没有可用于拼接的瓦片",
        ));
    }
    let cols = (x_max - x_min + 1) as c_int;
    let rows = (y_max - y_min + 1) as c_int;
    let out_w = cols * 256;
    let out_h = rows * 256;
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent).map_err(|_| {
            error(
                NativeGisErrorCode::DemReprojectionFailed,
                "无法创建拼接 GeoTIFF 输出目录",
            )
        })?;
    }
    let _proj_context = runtime.create_proj_context()?;
    let out_c = c_path(
        output_path,
        NativeGisErrorCode::DemReprojectionFailed,
        "拼接 GeoTIFF 输出路径编码无效",
    )?;
    let wkt_c = CString::new(projection_wkt)
        .map_err(|_| error(NativeGisErrorCode::DemCrsMissing, "投影 WKT 字符串编码无效"))?;
    let mut tf = geotransform;
    let mut stitched_count = 0usize;
    let mut pyramids_built = false;
    let mut exported_extra_path: Option<String> = None;

    let norm_comp = compression.trim().to_ascii_uppercase();
    let (effective_comp, comp_label) = match norm_comp.as_str() {
        "LZW" => (Some("COMPRESS=LZW"), "LZW"),
        "DEFLATE" => (Some("COMPRESS=DEFLATE"), "DEFLATE"),
        "JPEG" => (Some("COMPRESS=JPEG"), "JPEG"),
        "WEBP" => (Some("COMPRESS=WEBP"), "WEBP"),
        "NONE" | "" => (None, "NONE"),
        _ => (Some("COMPRESS=DEFLATE"), "DEFLATE"),
    };
    let fmt_lower = export_format.trim().to_ascii_lowercase();
    let is_native_cog = fmt_lower == "cog";

    unsafe {
        configure_gdal_runtime_for_source(runtime, false)?;
        let open = *runtime.gdal_symbol::<GdalOpenEx>(b"GDALOpenEx\0")?;
        let close = *runtime.gdal_symbol::<GdalClose>(b"GDALClose\0")?;
        let get_driver = *runtime.gdal_symbol::<GdalGetDriverByName>(b"GDALGetDriverByName\0")?;
        let get_meta = *runtime.gdal_symbol::<GdalGetMetadataItem>(b"GDALGetMetadataItem\0")?;
        let create = *runtime.gdal_symbol::<GdalCreate>(b"GDALCreate\0")?;
        let set_proj = *runtime.gdal_symbol::<GdalSetProjection>(b"GDALSetProjection\0")?;
        let set_tf = *runtime.gdal_symbol::<GdalSetGeoTransform>(b"GDALSetGeoTransform\0")?;
        let get_band = *runtime.gdal_symbol::<GdalGetRasterBand>(b"GDALGetRasterBand\0")?;
        let get_count = *runtime.gdal_symbol::<GdalGetRasterCount>(b"GDALGetRasterCount\0")?;
        let get_x = *runtime.gdal_symbol::<GdalGetRasterXSize>(b"GDALGetRasterXSize\0")?;
        let get_y = *runtime.gdal_symbol::<GdalGetRasterYSize>(b"GDALGetRasterYSize\0")?;
        let raster_io = *runtime.gdal_symbol::<GdalRasterIo>(b"GDALRasterIO\0")?;
        let flush = *runtime.gdal_symbol::<GdalFlushCache>(b"GDALFlushCache\0")?;

        let driver_supports_webp = |drv: Driver| -> bool {
            let opt_key = CString::new("DMD_CREATIONOPTIONLIST").unwrap();
            let ptr = get_meta(drv, opt_key.as_ptr(), std::ptr::null());
            if ptr.is_null() {
                false
            } else {
                CStr::from_ptr(ptr)
                    .to_string_lossy()
                    .to_ascii_uppercase()
                    .contains("WEBP")
            }
        };

        let gtiff_name = CString::new("GTiff").expect("static GDAL driver");
        let driver = get_driver(gtiff_name.as_ptr());
        if driver.is_null() {
            return Err(error(
                NativeGisErrorCode::GisDriverUnavailable,
                "GTiff 驱动不可用",
            ));
        }

        let cog_driver = if is_native_cog {
            let cog_name = CString::new("COG").expect("static GDAL driver");
            let drv = get_driver(cog_name.as_ptr());
            if drv.is_null() {
                return Err(error(
                    NativeGisErrorCode::GisDriverUnavailable,
                    "产品 GDAL runtime 缺少原生 COG 驱动",
                ));
            }
            if comp_label == "WEBP" && !driver_supports_webp(drv) {
                return Err(error(
                    NativeGisErrorCode::GisDriverUnavailable,
                    "当前 GDAL COG 驱动不支持 WEBP 压缩",
                ));
            }
            Some(drv)
        } else {
            if comp_label == "WEBP" && !driver_supports_webp(driver) {
                return Err(error(
                    NativeGisErrorCode::GisDriverUnavailable,
                    "当前 GDAL GTiff 驱动不支持 WEBP 压缩",
                ));
            }
            None
        };

        // When exporting native COG, stitch into an uncompressed staging GTiff first so we
        // can invoke GDAL's native COG CreateCopy driver for true front-IFD layout without
        // double-compressing lossy JPEG/WEBP payloads.
        let staging_path = if is_native_cog {
            output_path.with_extension("cog_staging.tif")
        } else {
            output_path.to_path_buf()
        };
        let _ = fs::remove_file(&staging_path);
        let staging_c = c_path(
            &staging_path,
            NativeGisErrorCode::DemReprojectionFailed,
            "拼接临时路径编码无效",
        )?;

        let mut opt_cstrings: Vec<CString> = vec![CString::new("BIGTIFF=IF_SAFER").unwrap()];
        if !is_native_cog {
            if let Some(comp_opt) = effective_comp {
                if let Ok(cs) = CString::new(comp_opt) {
                    opt_cstrings.push(cs);
                }
            }
            if build_pyramids {
                opt_cstrings.push(CString::new("TILED=YES").unwrap());
                opt_cstrings.push(CString::new("BLOCKXSIZE=256").unwrap());
                opt_cstrings.push(CString::new("BLOCKYSIZE=256").unwrap());
            }
        }
        let mut opt_ptrs: Vec<*const c_char> = opt_cstrings.iter().map(|c| c.as_ptr()).collect();
        opt_ptrs.push(std::ptr::null());
        let create_opts = if opt_cstrings.is_empty() {
            std::ptr::null()
        } else {
            opt_ptrs.as_ptr()
        };

        let out_ds = create(
            driver,
            if is_native_cog {
                staging_c.as_ptr()
            } else {
                out_c.as_ptr()
            },
            out_w,
            out_h,
            3,
            GDAL_BYTE,
            create_opts,
        );
        if out_ds.is_null() {
            let _ = if is_native_cog {
                fs::remove_file(&staging_path)
            } else {
                Ok(())
            };
            return Err(error(
                NativeGisErrorCode::DemReprojectionFailed,
                "创建拼接 GeoTIFF 数据集失败",
            ));
        }
        let out_guard = GdalDatasetGuard::new(out_ds, close);
        set_proj(out_guard.raw(), wkt_c.as_ptr());
        set_tf(out_guard.raw(), tf.as_mut_ptr());

        let mut tile_buf = vec![0u8; 256 * 256];
        for (tx, ty, tile_path) in tiles {
            if *tx < x_min || *tx > x_max || *ty < y_min || *ty > y_max {
                continue;
            }
            let Ok(tile_c) = c_path(
                tile_path,
                NativeGisErrorCode::DemInvalidRaster,
                "瓦片文件路径编码无效",
            ) else {
                continue;
            };
            let src_ds = open(
                tile_c.as_ptr(),
                GDAL_OF_RASTER | GDAL_ACCESS_READ_ONLY,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
            );
            if src_ds.is_null() {
                continue;
            }
            let src_guard = GdalDatasetGuard::new(src_ds, close);
            let src_bands = get_count(src_guard.raw());
            let src_w = get_x(src_guard.raw()).max(1);
            let src_h = get_y(src_guard.raw()).max(1);
            if src_bands < 1 {
                continue;
            }
            let dst_x = ((*tx - x_min) * 256) as c_int;
            let dst_y = ((*ty - y_min) * 256) as c_int;

            for b in 1..=3 {
                let src_b = if src_bands >= b { b } else { 1 };
                let sb = get_band(src_guard.raw(), src_b);
                let db = get_band(out_guard.raw(), b);
                if sb.is_null() || db.is_null() {
                    continue;
                }
                tile_buf.fill(0);
                if raster_io(
                    sb,
                    GDAL_READ,
                    0,
                    0,
                    src_w,
                    src_h,
                    tile_buf.as_mut_ptr().cast(),
                    256,
                    256,
                    GDAL_BYTE,
                    0,
                    0,
                ) == 0
                {
                    let _ = raster_io(
                        db,
                        GDAL_WRITE,
                        dst_x,
                        dst_y,
                        256,
                        256,
                        tile_buf.as_mut_ptr().cast(),
                        256,
                        256,
                        GDAL_BYTE,
                        0,
                        0,
                    );
                }
            }
            stitched_count += 1;
        }
        flush(out_guard.raw());

        if let Some(cog_drv) = cog_driver {
            let create_copy = *runtime.gdal_symbol::<GdalCreateCopy>(b"GDALCreateCopy\0")?;
            let _ = fs::remove_file(output_path);
            let cog_comp = format!("COMPRESS={comp_label}");
            let cog_cstrings = [
                CString::new(cog_comp).unwrap(),
                CString::new("BLOCKSIZE=256").unwrap(),
                CString::new("OVERVIEW_RESAMPLING=AVERAGE").unwrap(),
                CString::new("BIGTIFF=IF_SAFER").unwrap(),
            ];
            let cog_ptrs = [
                cog_cstrings[0].as_ptr(),
                cog_cstrings[1].as_ptr(),
                cog_cstrings[2].as_ptr(),
                cog_cstrings[3].as_ptr(),
                std::ptr::null(),
            ];
            let cog_ds = create_copy(
                cog_drv,
                out_c.as_ptr(),
                out_guard.raw(),
                0,
                cog_ptrs.as_ptr(),
                std::ptr::null(),
                std::ptr::null_mut(),
            );
            drop(out_guard);
            let _ = fs::remove_file(&staging_path);
            let _ = fs::remove_file(staging_path.with_extension("tif.aux.xml"));
            if cog_ds.is_null() {
                return Err(error(
                    NativeGisErrorCode::DemReprojectionFailed,
                    format!(
                        "GDAL COG 驱动生成 Cloud-Optimized GeoTIFF 失败 (COMPRESS={comp_label})"
                    ),
                ));
            }
            let cog_guard = GdalDatasetGuard::new(cog_ds, close);
            flush(cog_guard.raw());
            pyramids_built = true;
        } else {
            // Build internal multi-scale pyramid overviews (2x, 4x, 8x, 16x) if requested
            if build_pyramids && stitched_count > 0 {
                if let Ok(build_ovr) =
                    runtime.gdal_symbol::<GdalBuildOverviews>(b"GDALBuildOverviews\0")
                {
                    let resampling_c = CString::new("AVERAGE").unwrap();
                    let factors: [c_int; 4] = [2, 4, 8, 16];
                    if (*build_ovr)(
                        out_guard.raw(),
                        resampling_c.as_ptr(),
                        factors.len() as c_int,
                        factors.as_ptr(),
                        0,
                        std::ptr::null(),
                        std::ptr::null(),
                        std::ptr::null_mut(),
                    ) == 0
                    {
                        pyramids_built = true;
                        flush(out_guard.raw());
                    }
                }
            }

            // Export additional format (PNG / JPEG / ENVI) via GDALCreateCopy if requested
            let extra_driver_and_ext = match fmt_lower.as_str() {
                "png" => Some(("PNG", "png", Some("pgw"))),
                "jpg" | "jpeg" => Some(("JPEG", "jpg", Some("jgw"))),
                "envi" => Some(("ENVI", "bin", None)),
                _ => None,
            };
            if let Some((drv_str, ext_str, world_ext_opt)) = extra_driver_and_ext {
                let create_copy = *runtime.gdal_symbol::<GdalCreateCopy>(b"GDALCreateCopy\0")?;
                let drv_c = CString::new(drv_str).map_err(|_| {
                    error(
                        NativeGisErrorCode::DemReprojectionFailed,
                        format!("GDAL 驱动名称无效: {drv_str}"),
                    )
                })?;
                let copy_drv = get_driver(drv_c.as_ptr());
                if copy_drv.is_null() {
                    return Err(error(
                        NativeGisErrorCode::DemReprojectionFailed,
                        format!("GDAL 未注册 {drv_str} 驱动，无法导出 {fmt_lower} 格式"),
                    ));
                }
                let extra_path = output_path.with_extension(ext_str);
                let extra_c = c_path(
                    &extra_path,
                    NativeGisErrorCode::DemReprojectionFailed,
                    "附加导出格式路径无效",
                )?;
                let copy_ds = create_copy(
                    copy_drv,
                    extra_c.as_ptr(),
                    out_guard.raw(),
                    0,
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null_mut(),
                );
                if copy_ds.is_null() {
                    return Err(error(
                        NativeGisErrorCode::DemReprojectionFailed,
                        format!(
                            "GDAL {drv_str} 驱动生成 {fmt_lower} 导出文件失败 (尺寸 {out_w}x{out_h}；若为 JPEG 请确认不超过 65,535 像素上限)"
                        ),
                    ));
                }
                let copy_guard = GdalDatasetGuard::new(copy_ds, close);
                flush(copy_guard.raw());
                if let Some(w_ext) = world_ext_opt {
                    let ul_x = tf[0] + tf[1] * 0.5;
                    let ul_y = tf[3] + tf[5] * 0.5;
                    let w_content = format!(
                        "{:.8}\n0.00000000\n0.00000000\n{:.8}\n{:.6}\n{:.6}\n",
                        tf[1], tf[5], ul_x, ul_y
                    );
                    let _ = fs::write(extra_path.with_extension(w_ext), w_content);
                    let _ = fs::write(extra_path.with_extension("prj"), projection_wkt);
                }
                exported_extra_path = Some(extra_path.to_string_lossy().to_string());
            }
        }
    }

    let bytes = fs::metadata(output_path).map(|m| m.len()).unwrap_or(0);
    let exported_path_final = exported_extra_path
        .clone()
        .unwrap_or_else(|| output_path.to_string_lossy().to_string());
    Ok(json!({
        "ok": stitched_count > 0 && bytes > 0,
        "output_path": output_path.to_string_lossy(),
        "exported_extra_path": exported_extra_path,
        "exported_path": exported_path_final,
        "compression": comp_label,
        "pyramids_built": pyramids_built,
        "export_format": fmt_lower,
        "width": out_w,
        "height": out_h,
        "bands": 3,
        "stitched_tiles": stitched_count,
        "bytes": bytes,
    }))
}

/// Crop a parent tile at `tile_path` to the child quadrant `(sub_ix, sub_iy)` at zoom
/// difference `shift` (`sub_div = 1 << shift`) and bilinearly upsample it to `256 x 256`
/// in-place so Z19–Z21 super-zoom fallback tiles represent the exact child sub-tile.
pub(crate) fn crop_and_upsample_subtile(
    runtime: &NativeGisRuntime,
    tile_path: &Path,
    sub_ix: u32,
    sub_iy: u32,
    shift: u32,
) -> Result<bool, NativeGisError> {
    if shift == 0 || !tile_path.is_file() {
        return Ok(false);
    }
    let _proj_context = runtime.create_proj_context()?;
    let tile_c = c_path(
        tile_path,
        NativeGisErrorCode::DemInvalidRaster,
        "瓦片文件路径编码无效",
    )?;
    let ext_lower = tile_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("jpg")
        .to_ascii_lowercase();

    unsafe {
        configure_gdal_runtime_for_source(runtime, false)?;
        let open = *runtime.gdal_symbol::<GdalOpenEx>(b"GDALOpenEx\0")?;
        let close = *runtime.gdal_symbol::<GdalClose>(b"GDALClose\0")?;
        let get_driver = *runtime.gdal_symbol::<GdalGetDriverByName>(b"GDALGetDriverByName\0")?;
        let create = *runtime.gdal_symbol::<GdalCreate>(b"GDALCreate\0")?;
        let get_band = *runtime.gdal_symbol::<GdalGetRasterBand>(b"GDALGetRasterBand\0")?;
        let get_count = *runtime.gdal_symbol::<GdalGetRasterCount>(b"GDALGetRasterCount\0")?;
        let get_x = *runtime.gdal_symbol::<GdalGetRasterXSize>(b"GDALGetRasterXSize\0")?;
        let get_y = *runtime.gdal_symbol::<GdalGetRasterYSize>(b"GDALGetRasterYSize\0")?;
        let raster_io = *runtime.gdal_symbol::<GdalRasterIo>(b"GDALRasterIO\0")?;
        let flush = *runtime.gdal_symbol::<GdalFlushCache>(b"GDALFlushCache\0")?;

        let (bands, crop_w, crop_h, band_buffers) = {
            let src_ds = open(
                tile_c.as_ptr(),
                GDAL_OF_RASTER | GDAL_ACCESS_READ_ONLY,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
            );
            if src_ds.is_null() {
                return Ok(false);
            }
            let src_guard = GdalDatasetGuard::new(src_ds, close);
            let src_w = get_x(src_guard.raw()).max(1) as u32;
            let src_h = get_y(src_guard.raw()).max(1) as u32;
            let b_count = get_count(src_guard.raw()).clamp(1, 4);
            let sub_div = (1u32 << shift.min(6)).max(2);
            let cw = (src_w / sub_div).max(1);
            let ch = (src_h / sub_div).max(1);
            let cx = (sub_ix.min(sub_div - 1) * cw).min(src_w.saturating_sub(cw));
            let cy = (sub_iy.min(sub_div - 1) * ch).min(src_h.saturating_sub(ch));

            let mut bufs = Vec::with_capacity(b_count as usize);
            for b in 1..=b_count {
                let sb = get_band(src_guard.raw(), b);
                if sb.is_null() {
                    return Ok(false);
                }
                let mut src_buf = vec![0u8; (cw as usize) * (ch as usize)];
                if raster_io(
                    sb,
                    GDAL_READ,
                    cx as c_int,
                    cy as c_int,
                    cw as c_int,
                    ch as c_int,
                    src_buf.as_mut_ptr().cast(),
                    cw as c_int,
                    ch as c_int,
                    GDAL_BYTE,
                    0,
                    0,
                ) != 0
                {
                    return Ok(false);
                }
                bufs.push(src_buf);
            }
            (b_count, cw as usize, ch as usize, bufs)
        };

        // Bilinearly upsample each band from (crop_w x crop_h) to (256 x 256)
        let mut upsampled_bands: Vec<Vec<u8>> = Vec::with_capacity(bands as usize);
        for src_buf in &band_buffers {
            let mut dst_buf = vec![0u8; 256 * 256];
            for dy in 0..256usize {
                let sy = ((dy as f64 + 0.5) * (crop_h as f64) / 256.0 - 0.5)
                    .clamp(0.0, (crop_h.saturating_sub(1)) as f64);
                let y0 = sy.floor() as usize;
                let y1 = (y0 + 1).min(crop_h.saturating_sub(1));
                let wy = sy - (y0 as f64);
                for dx in 0..256usize {
                    let sx = ((dx as f64 + 0.5) * (crop_w as f64) / 256.0 - 0.5)
                        .clamp(0.0, (crop_w.saturating_sub(1)) as f64);
                    let x0 = sx.floor() as usize;
                    let x1 = (x0 + 1).min(crop_w.saturating_sub(1));
                    let wx = sx - (x0 as f64);
                    let v00 = f64::from(src_buf[y0 * crop_w + x0]);
                    let v10 = f64::from(src_buf[y0 * crop_w + x1]);
                    let v01 = f64::from(src_buf[y1 * crop_w + x0]);
                    let v11 = f64::from(src_buf[y1 * crop_w + x1]);
                    let top = v00 * (1.0 - wx) + v10 * wx;
                    let bot = v01 * (1.0 - wx) + v11 * wx;
                    let val = (top * (1.0 - wy) + bot * wy).round().clamp(0.0, 255.0) as u8;
                    dst_buf[dy * 256 + dx] = val;
                }
            }
            upsampled_bands.push(dst_buf);
        }

        let tmp_tif = tile_path.with_extension("subtile_tmp.tif");
        let tmp_tif_c = c_path(
            &tmp_tif,
            NativeGisErrorCode::DemReprojectionFailed,
            "临时瓦片路径编码无效",
        )?;
        let gtiff_name = CString::new("GTiff").expect("static GDAL driver");
        let gtiff_drv = get_driver(gtiff_name.as_ptr());
        if gtiff_drv.is_null() {
            return Ok(false);
        }
        let tmp_ds = create(
            gtiff_drv,
            tmp_tif_c.as_ptr(),
            256,
            256,
            bands,
            GDAL_BYTE,
            std::ptr::null(),
        );
        if tmp_ds.is_null() {
            return Ok(false);
        }
        {
            let tmp_guard = GdalDatasetGuard::new(tmp_ds, close);
            for (idx, mut buf) in upsampled_bands.into_iter().enumerate() {
                let db = get_band(tmp_guard.raw(), (idx + 1) as c_int);
                if !db.is_null() {
                    let _ = raster_io(
                        db,
                        GDAL_WRITE,
                        0,
                        0,
                        256,
                        256,
                        buf.as_mut_ptr().cast(),
                        256,
                        256,
                        GDAL_BYTE,
                        0,
                        0,
                    );
                }
            }
            flush(tmp_guard.raw());

            let target_drv_name = if ext_lower == "png" {
                Some("PNG")
            } else if ext_lower == "jpg" || ext_lower == "jpeg" {
                Some("JPEG")
            } else {
                None
            };
            let mut wrote_out = false;
            if let Some(drv_str) = target_drv_name {
                if let Ok(create_copy) = runtime.gdal_symbol::<GdalCreateCopy>(b"GDALCreateCopy\0")
                {
                    if let Ok(drv_c) = CString::new(drv_str) {
                        let copy_drv = get_driver(drv_c.as_ptr());
                        if !copy_drv.is_null() {
                            let tmp_out =
                                tile_path.with_extension(format!("subtile_out.{ext_lower}"));
                            if let Ok(tmp_out_c) = c_path(
                                &tmp_out,
                                NativeGisErrorCode::DemReprojectionFailed,
                                "临时输出瓦片路径编码无效",
                            ) {
                                let copy_ds = (*create_copy)(
                                    copy_drv,
                                    tmp_out_c.as_ptr(),
                                    tmp_guard.raw(),
                                    0,
                                    std::ptr::null(),
                                    std::ptr::null(),
                                    std::ptr::null_mut(),
                                );
                                if !copy_ds.is_null() {
                                    let copy_guard = GdalDatasetGuard::new(copy_ds, close);
                                    flush(copy_guard.raw());
                                    drop(copy_guard);
                                    if tmp_out.is_file() {
                                        let _ = fs::remove_file(tile_path);
                                        if fs::rename(&tmp_out, tile_path).is_ok() {
                                            wrote_out = true;
                                        } else if fs::copy(&tmp_out, tile_path).is_ok() {
                                            let _ = fs::remove_file(&tmp_out);
                                            wrote_out = true;
                                        }
                                    }
                                    let _ = fs::remove_file(
                                        tmp_out.with_extension(format!("{ext_lower}.aux.xml")),
                                    );
                                    let _ = fs::remove_file(
                                        tile_path.with_extension(format!("{ext_lower}.aux.xml")),
                                    );
                                }
                            }
                        }
                    }
                }
            }
            if !wrote_out {
                drop(tmp_guard);
                if tmp_tif.is_file() {
                    let _ = fs::remove_file(tile_path);
                    if fs::rename(&tmp_tif, tile_path).is_ok() {
                        wrote_out = true;
                    } else if fs::copy(&tmp_tif, tile_path).is_ok() {
                        let _ = fs::remove_file(&tmp_tif);
                        wrote_out = true;
                    }
                }
                return Ok(wrote_out);
            }
        }
        let _ = fs::remove_file(&tmp_tif);
        Ok(true)
    }
}

fn encode_rgba_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, NativeGisError> {
    use flate2::write::ZlibEncoder;
    use flate2::{Compression, Crc};
    use std::io::Write;

    let expected_len = (width as usize)
        .saturating_mul(height as usize)
        .saturating_mul(4);
    if width == 0 || height == 0 || rgba.len() != expected_len {
        return Err(error(
            NativeGisErrorCode::DemInvalidRaster,
            "PNG 预览图像尺寸与 RGBA 缓冲区长度不匹配",
        ));
    }

    let mut out = Vec::with_capacity(64 + expected_len / 2);
    out.extend_from_slice(b"\x89PNG\r\n\x1a\n");

    let write_chunk = |dest: &mut Vec<u8>, chunk_type: &[u8; 4], data: &[u8]| {
        dest.extend_from_slice(&(data.len() as u32).to_be_bytes());
        dest.extend_from_slice(chunk_type);
        dest.extend_from_slice(data);
        let mut crc = Crc::new();
        crc.update(chunk_type);
        crc.update(data);
        dest.extend_from_slice(&crc.sum().to_be_bytes());
    };

    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&width.to_be_bytes());
    ihdr[4..8].copy_from_slice(&height.to_be_bytes());
    ihdr[8] = 8; // 8-bit
    ihdr[9] = 6; // RGBA
    ihdr[10] = 0; // Deflate
    ihdr[11] = 0; // Adaptive filter
    ihdr[12] = 0; // No interlace
    write_chunk(&mut out, b"IHDR", &ihdr);

    let row_bytes = (width as usize) * 4;
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
    for y in 0..(height as usize) {
        encoder.write_all(&[0u8]).map_err(|_| {
            error(
                NativeGisErrorCode::DemReprojectionFailed,
                "PNG 扫描线压缩失败",
            )
        })?;
        let row_start = y * row_bytes;
        encoder
            .write_all(&rgba[row_start..row_start + row_bytes])
            .map_err(|_| {
                error(
                    NativeGisErrorCode::DemReprojectionFailed,
                    "PNG 像元流写入失败",
                )
            })?;
    }
    let idat = encoder.finish().map_err(|_| {
        error(
            NativeGisErrorCode::DemReprojectionFailed,
            "PNG Zlib 流结束失败",
        )
    })?;
    write_chunk(&mut out, b"IDAT", &idat);
    write_chunk(&mut out, b"IEND", &[]);
    Ok(out)
}

pub(crate) fn transform_bbox_with_proj(
    runtime: &NativeGisRuntime,
    src_crs: &str,
    dst_crs: &str,
    bbox: [f64; 4],
) -> Option<[f64; 4]> {
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct ProjCoord {
        x: f64,
        y: f64,
        z: f64,
        t: f64,
    }
    type ProjCreateCrs = unsafe extern "C" fn(
        *mut c_void,
        *const c_char,
        *const c_char,
        *const c_void,
    ) -> *mut c_void;
    type ProjNormalize = unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void;
    type ProjDestroy = unsafe extern "C" fn(*mut c_void) -> *mut c_void;
    type ProjTransFn = unsafe extern "C" fn(*mut c_void, c_int, ProjCoord) -> ProjCoord;

    let src_c = CString::new(src_crs.trim()).ok()?;
    let dst_c = CString::new(dst_crs.trim()).ok()?;
    let ctx_guard = runtime.create_proj_context().ok()?;
    let ctx = ctx_guard.raw();
    unsafe {
        let crs_create = *runtime
            .proj_symbol::<ProjCreateCrs>(b"proj_create_crs_to_crs\0")
            .ok()?;
        let normalize = *runtime
            .proj_symbol::<ProjNormalize>(b"proj_normalize_for_visualization\0")
            .ok()?;
        let destroy = *runtime.proj_symbol::<ProjDestroy>(b"proj_destroy\0").ok()?;
        let trans = *runtime.proj_symbol::<ProjTransFn>(b"proj_trans\0").ok()?;

        let op = crs_create(ctx, src_c.as_ptr(), dst_c.as_ptr(), std::ptr::null());
        if op.is_null() {
            return None;
        }
        let norm = normalize(ctx, op);
        let pj = if norm.is_null() {
            op
        } else {
            destroy(op);
            norm
        };

        let mx = (bbox[0] + bbox[2]) * 0.5;
        let my = (bbox[1] + bbox[3]) * 0.5;
        let samples = [
            (bbox[0], bbox[1]),
            (bbox[2], bbox[1]),
            (bbox[2], bbox[3]),
            (bbox[0], bbox[3]),
            (mx, bbox[1]),
            (mx, bbox[3]),
            (bbox[0], my),
            (bbox[2], my),
        ];
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        let mut valid_pts = 0usize;
        for (sx, sy) in samples {
            let out = trans(
                pj,
                1,
                ProjCoord {
                    x: sx,
                    y: sy,
                    z: 0.0,
                    t: 0.0,
                },
            );
            if out.x.is_finite() && out.y.is_finite() {
                min_x = min_x.min(out.x);
                max_x = max_x.max(out.x);
                min_y = min_y.min(out.y);
                max_y = max_y.max(out.y);
                valid_pts += 1;
            }
        }
        destroy(pj);
        if valid_pts >= 4 && min_x < max_x && min_y < max_y {
            Some([min_x, min_y, max_x, max_y])
        } else {
            None
        }
    }
}

fn dataset_bounds_to_wgs84(
    runtime: &NativeGisRuntime,
    projection: &str,
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
) -> [f64; 4] {
    if !projection.trim().is_empty() && !is_wgs84_geographic_crs(projection) {
        if let Some(b) = transform_bbox_with_proj(
            runtime,
            projection,
            "EPSG:4326",
            [
                min_x.min(max_x),
                min_y.min(max_y),
                min_x.max(max_x),
                min_y.max(max_y),
            ],
        ) {
            return [
                b[0].clamp(-180.0, 180.0),
                b[1].clamp(-90.0, 90.0),
                b[2].clamp(-180.0, 180.0),
                b[3].clamp(-90.0, 90.0),
            ];
        }
    }
    let proj_upper = projection.to_ascii_uppercase();
    let is_web_mercator = proj_upper.contains("PSEUDO-MERCATOR")
        || proj_upper.contains("MERCATOR_1SP")
        || proj_upper.contains("3857");
    if is_web_mercator {
        let origin_shift = 20_037_508.342_789_244_f64;
        let merc_to_lon = |x: f64| (x / origin_shift) * 180.0;
        let merc_to_lat = |y: f64| {
            let lat_rad = 2.0 * ((y / origin_shift * std::f64::consts::PI).exp().atan())
                - std::f64::consts::FRAC_PI_2;
            lat_rad.to_degrees()
        };
        let w = merc_to_lon(min_x).clamp(-180.0, 180.0);
        let e = merc_to_lon(max_x).clamp(-180.0, 180.0);
        let s = merc_to_lat(min_y).clamp(-85.051_129, 85.051_129);
        let n = merc_to_lat(max_y).clamp(-85.051_129, 85.051_129);
        return [w.min(e), s.min(n), w.max(e), s.max(n)];
    }
    if proj_upper.contains("SINUSOIDAL") || proj_upper.contains("6371007") {
        const R: f64 = 6_371_007.181;
        let corners = [
            (min_x, min_y),
            (max_x, min_y),
            (max_x, max_y),
            (min_x, max_y),
        ];
        let mut w = f64::INFINITY;
        let mut s = f64::INFINITY;
        let mut e = f64::NEG_INFINITY;
        let mut n = f64::NEG_INFINITY;
        for (x, y) in corners {
            let lat_rad = (y / R).clamp(-1.55, 1.55);
            let lon_rad = x / (R * lat_rad.cos().max(1e-4));
            let lon = lon_rad.to_degrees().clamp(-180.0, 180.0);
            let lat = lat_rad.to_degrees().clamp(-90.0, 90.0);
            w = w.min(lon);
            e = e.max(lon);
            s = s.min(lat);
            n = n.max(lat);
        }
        return [w, s, e, n];
    }
    [
        min_x.min(max_x),
        min_y.min(max_y),
        min_x.max(max_x),
        min_y.max(max_y),
    ]
}

/// Render a lightweight RGBA PNG preview (`<= max_dim` on longest axis) and return
/// `(png_bytes, wgs84_bbox_[west, south, east, north])` for Leaflet `<ImageOverlay>`.
pub(crate) fn render_raster_preview_png(
    runtime: &NativeGisRuntime,
    raster_path: &Path,
    max_dim: u32,
) -> Result<(Vec<u8>, [f64; 4]), NativeGisError> {
    if !raster_path.is_file() {
        return Err(error(
            NativeGisErrorCode::DemInvalidRaster,
            "用于生成地图预览图的栅格文件不存在",
        ));
    }
    let _proj_context = runtime.create_proj_context()?;
    let raster_c = c_path(
        raster_path,
        NativeGisErrorCode::DemInvalidRaster,
        "预览栅格路径编码无效",
    )?;
    let cap_dim = max_dim.clamp(64, 1024);

    unsafe {
        configure_gdal_runtime_for_source(runtime, false)?;
        let open = *runtime.gdal_symbol::<GdalOpenEx>(b"GDALOpenEx\0")?;
        let close = *runtime.gdal_symbol::<GdalClose>(b"GDALClose\0")?;
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
        let raster_io = *runtime.gdal_symbol::<GdalRasterIo>(b"GDALRasterIO\0")?;

        let ds = open(
            raster_c.as_ptr(),
            GDAL_OF_RASTER | GDAL_ACCESS_READ_ONLY,
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
        );
        if ds.is_null() {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                "GDAL 无法打开目标栅格生成预览图",
            ));
        }
        let guard = GdalDatasetGuard::new(ds, close);
        let src_w = get_w(guard.raw());
        let src_h = get_h(guard.raw());
        let band_cnt = get_cnt(guard.raw());
        if src_w <= 0 || src_h <= 0 || band_cnt <= 0 {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                "栅格尺寸或波段数无效",
            ));
        }
        let proj_ptr = get_proj(guard.raw());
        let projection = if proj_ptr.is_null() {
            String::new()
        } else {
            CStr::from_ptr(proj_ptr).to_string_lossy().to_string()
        };
        let mut tf = [0.0f64; 6];
        if get_tf(guard.raw(), tf.as_mut_ptr()) != 0 {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                "预览栅格缺少有效地理参考变换",
            ));
        }
        let x0 = tf[0];
        let y0 = tf[3];
        let x1 = x0 + tf[1] * f64::from(src_w) + tf[2] * f64::from(src_h);
        let y1 = y0 + tf[4] * f64::from(src_w) + tf[5] * f64::from(src_h);
        let wgs84_bbox = dataset_bounds_to_wgs84(
            runtime,
            &projection,
            x0.min(x1),
            y0.min(y1),
            x0.max(x1),
            y0.max(y1),
        );

        let scale = ((cap_dim as f64) / (src_w.max(src_h) as f64)).min(1.0);
        let out_w = ((src_w as f64 * scale).round() as c_int).max(1);
        let out_h = ((src_h as f64 * scale).round() as c_int).max(1);
        let npx = (out_w as usize) * (out_h as usize);
        let mut rgba = vec![0u8; npx * 4];

        let b1 = get_band(guard.raw(), 1);
        if b1.is_null() {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                "无法获取栅格第一波段",
            ));
        }
        let dtype1 = get_dtype(b1);

        if band_cnt >= 3 && dtype1 == GDAL_BYTE {
            let mut r_buf = vec![0u8; npx];
            let mut g_buf = vec![0u8; npx];
            let mut b_buf = vec![0u8; npx];
            let b2 = get_band(guard.raw(), 2);
            let b3 = get_band(guard.raw(), 3);
            if b2.is_null() || b3.is_null() {
                return Err(error(
                    NativeGisErrorCode::DemInvalidRaster,
                    "无法读取 RGB 波段",
                ));
            }
            if raster_io(
                b1,
                GDAL_READ,
                0,
                0,
                src_w,
                src_h,
                r_buf.as_mut_ptr().cast(),
                out_w,
                out_h,
                GDAL_BYTE,
                0,
                0,
            ) != 0
                || raster_io(
                    b2,
                    GDAL_READ,
                    0,
                    0,
                    src_w,
                    src_h,
                    g_buf.as_mut_ptr().cast(),
                    out_w,
                    out_h,
                    GDAL_BYTE,
                    0,
                    0,
                ) != 0
                || raster_io(
                    b3,
                    GDAL_READ,
                    0,
                    0,
                    src_w,
                    src_h,
                    b_buf.as_mut_ptr().cast(),
                    out_w,
                    out_h,
                    GDAL_BYTE,
                    0,
                    0,
                ) != 0
            {
                return Err(error(
                    NativeGisErrorCode::DemInvalidRaster,
                    "读取 RGB 预览像元失败",
                ));
            }
            let mut a_buf = vec![255u8; npx];
            if band_cnt >= 4 {
                let b4 = get_band(guard.raw(), 4);
                if !b4.is_null() {
                    let _ = raster_io(
                        b4,
                        GDAL_READ,
                        0,
                        0,
                        src_w,
                        src_h,
                        a_buf.as_mut_ptr().cast(),
                        out_w,
                        out_h,
                        GDAL_BYTE,
                        0,
                        0,
                    );
                }
            }
            for i in 0..npx {
                rgba[i * 4] = r_buf[i];
                rgba[i * 4 + 1] = g_buf[i];
                rgba[i * 4 + 2] = b_buf[i];
                rgba[i * 4 + 3] = a_buf[i];
            }
        } else if band_cnt >= 3 {
            let mut chans: Vec<Vec<f32>> = Vec::with_capacity(3);
            let mut nds: Vec<Option<f64>> = Vec::with_capacity(3);
            for idx in 1..=3 {
                let b = get_band(guard.raw(), idx);
                if b.is_null() {
                    return Err(error(
                        NativeGisErrorCode::DemInvalidRaster,
                        "无法读取多波段预览数据",
                    ));
                }
                let mut has_nd = 0;
                let nd_val = get_nd(b, &mut has_nd);
                nds.push((has_nd != 0).then_some(nd_val));
                let mut buf = vec![0.0f32; npx];
                if raster_io(
                    b,
                    GDAL_READ,
                    0,
                    0,
                    src_w,
                    src_h,
                    buf.as_mut_ptr().cast(),
                    out_w,
                    out_h,
                    GDAL_FLOAT32,
                    0,
                    0,
                ) != 0
                {
                    return Err(error(
                        NativeGisErrorCode::DemInvalidRaster,
                        "读取多波段浮点像元失败",
                    ));
                }
                chans.push(buf);
            }
            let mut mins = [f64::INFINITY; 3];
            let mut maxs = [f64::NEG_INFINITY; 3];
            for c in 0..3 {
                for &v in &chans[c] {
                    let vf = f64::from(v);
                    if v.is_finite() && !nds[c].is_some_and(|nd| (vf - nd).abs() <= 1e-5) {
                        mins[c] = mins[c].min(vf);
                        maxs[c] = maxs[c].max(vf);
                    }
                }
                if !mins[c].is_finite() || (maxs[c] - mins[c]).abs() < 1e-9 {
                    mins[c] = 0.0;
                    maxs[c] = 1.0;
                }
            }
            for i in 0..npx {
                let mut any_valid = false;
                for c in 0..3 {
                    let v = chans[c][i];
                    let vf = f64::from(v);
                    if v.is_finite() && !nds[c].is_some_and(|nd| (vf - nd).abs() <= 1e-5) {
                        any_valid = true;
                        let norm = ((vf - mins[c]) / (maxs[c] - mins[c])).clamp(0.0, 1.0);
                        rgba[i * 4 + c] = (norm * 255.0).round() as u8;
                    }
                }
                rgba[i * 4 + 3] = if any_valid { 255 } else { 0 };
            }
        } else {
            let mut has_nd = 0;
            let nd_val = get_nd(b1, &mut has_nd);
            let nodata = (has_nd != 0).then_some(nd_val);
            let mut vals = vec![0.0f32; npx];
            if raster_io(
                b1,
                GDAL_READ,
                0,
                0,
                src_w,
                src_h,
                vals.as_mut_ptr().cast(),
                out_w,
                out_h,
                GDAL_FLOAT32,
                0,
                0,
            ) != 0
            {
                return Err(error(
                    NativeGisErrorCode::DemInvalidRaster,
                    "读取单波段预览像元失败",
                ));
            }
            let is_valid = |v: f32| -> bool {
                v.is_finite()
                    && !nodata.is_some_and(|nd| {
                        if nd.is_nan() {
                            v.is_nan()
                        } else {
                            (f64::from(v) - nd).abs() <= 1e-5
                        }
                    })
            };
            let mut min_v = f64::INFINITY;
            let mut max_v = f64::NEG_INFINITY;
            for &v in &vals {
                if is_valid(v) {
                    let vf = f64::from(v);
                    min_v = min_v.min(vf);
                    max_v = max_v.max(vf);
                }
            }
            let span = if min_v.is_finite() && (max_v - min_v).abs() > 1e-9 {
                max_v - min_v
            } else {
                min_v = 0.0;
                1.0
            };
            // Scientific Viridis-inspired 5-stop perceptual color ramp
            const STOPS: [[f64; 3]; 5] = [
                [68.0, 1.0, 84.0],
                [59.0, 82.0, 139.0],
                [33.0, 145.0, 140.0],
                [94.0, 201.0, 98.0],
                [253.0, 231.0, 37.0],
            ];
            for (i, &v) in vals.iter().enumerate() {
                if !is_valid(v) {
                    rgba[i * 4 + 3] = 0;
                    continue;
                }
                let t = ((f64::from(v) - min_v) / span).clamp(0.0, 1.0) * 4.0;
                let idx = (t.floor() as usize).min(3);
                let frac = t - (idx as f64);
                let r = STOPS[idx][0] + (STOPS[idx + 1][0] - STOPS[idx][0]) * frac;
                let g = STOPS[idx][1] + (STOPS[idx + 1][1] - STOPS[idx][1]) * frac;
                let b = STOPS[idx][2] + (STOPS[idx + 1][2] - STOPS[idx][2]) * frac;
                rgba[i * 4] = r.round().clamp(0.0, 255.0) as u8;
                rgba[i * 4 + 1] = g.round().clamp(0.0, 255.0) as u8;
                rgba[i * 4 + 2] = b.round().clamp(0.0, 255.0) as u8;
                rgba[i * 4 + 3] = 235;
            }
        }

        let png_bytes = encode_rgba_png(out_w as u32, out_h as u32, &rgba)?;
        Ok((png_bytes, wgs84_bbox))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::NativeGisRuntimeIdentity;

    fn require_runtime() -> NativeGisRuntime {
        let mut candidates = Vec::new();
        if let Some(env_root) = std::env::var_os("INSAR_GIS_RUNTIME_ROOT") {
            candidates.push(PathBuf::from(env_root));
        }
        candidates.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime/gis"));
        candidates.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../runtime/gis"));
        for candidate in candidates {
            if candidate.join("manifest.json").is_file() {
                let identity = NativeGisRuntimeIdentity::from_product_root(&candidate)
                    .unwrap_or_else(|e| {
                        panic!(
                            "invalid GIS runtime manifest at {}: {e}",
                            candidate.display()
                        )
                    });
                return NativeGisRuntime::load_identity(&identity).unwrap_or_else(|e| {
                    panic!("failed to load GIS runtime at {}: {e}", candidate.display())
                });
            }
        }
        panic!("INSAR_GIS_RUNTIME_ROOT product GIS runtime must be available for stream_ops tests");
    }

    #[test]
    fn normalize_source_recognizes_http_as_vsicurl() {
        let (resolved, is_remote) =
            normalize_gdal_source_string("https://example.com/cog/dem.tif").unwrap();
        assert!(is_remote);
        assert_eq!(resolved, "/vsicurl/https://example.com/cog/dem.tif");
    }

    #[test]
    #[ignore = "requires optional native GIS runtime (run scripts/run_native_gis_tests.sh with INSAR_GIS_RUNTIME_ROOT)"]
    fn native_stream_ops_slope_hillshade_ndvi_and_clip_work_with_real_gdal() {
        let runtime = require_runtime();
        let dir =
            std::env::temp_dir().join(format!("insar-gis-stream-ops-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let dem_path = dir.join("dem.tif");
        let nir_path = dir.join("nir.tif");
        let red_path = dir.join("red.tif");
        let w = 4;
        let h = 4;
        let mut tf = [117.0, 0.01, 0.0, 32.0, 0.0, -0.01];
        let wkt = CString::new(
            r#"GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433],AUTHORITY["EPSG","4326"]]"#,
        )
        .unwrap();

        unsafe {
            configure_gdal_runtime_for_source(&runtime, false).unwrap();
            let get_driver = *runtime
                .gdal_symbol::<GdalGetDriverByName>(b"GDALGetDriverByName\0")
                .unwrap();
            let create = *runtime.gdal_symbol::<GdalCreate>(b"GDALCreate\0").unwrap();
            let close = *runtime.gdal_symbol::<GdalClose>(b"GDALClose\0").unwrap();
            let set_proj = *runtime
                .gdal_symbol::<GdalSetProjection>(b"GDALSetProjection\0")
                .unwrap();
            let set_tf = *runtime
                .gdal_symbol::<GdalSetGeoTransform>(b"GDALSetGeoTransform\0")
                .unwrap();
            let get_band = *runtime
                .gdal_symbol::<GdalGetRasterBand>(b"GDALGetRasterBand\0")
                .unwrap();
            let raster_io = *runtime
                .gdal_symbol::<GdalRasterIo>(b"GDALRasterIO\0")
                .unwrap();
            let flush = *runtime
                .gdal_symbol::<GdalFlushCache>(b"GDALFlushCache\0")
                .unwrap();
            let drv_name = CString::new("GTiff").unwrap();
            let drv = get_driver(drv_name.as_ptr());

            let mut write_tif = |p: &Path, data: &[f32]| {
                let pc = c_path(p, NativeGisErrorCode::DemInvalidRaster, "test path").unwrap();
                let ds = create(drv, pc.as_ptr(), w, h, 1, GDAL_FLOAT32, std::ptr::null());
                let g = GdalDatasetGuard::new(ds, close);
                set_proj(g.raw(), wkt.as_ptr());
                set_tf(g.raw(), tf.as_mut_ptr());
                let b = get_band(g.raw(), 1);
                let mut buf = data.to_vec();
                assert_eq!(
                    raster_io(
                        b,
                        GDAL_WRITE,
                        0,
                        0,
                        w,
                        h,
                        buf.as_mut_ptr().cast(),
                        w,
                        h,
                        GDAL_FLOAT32,
                        0,
                        0
                    ),
                    0
                );
                flush(g.raw());
            };

            let dem_vals: Vec<f32> = (0..16).map(|i| 100.0 + (i as f32) * 25.0).collect();
            let nir_vals: Vec<f32> = vec![0.6f32; 16];
            let red_vals: Vec<f32> = vec![0.2f32; 16];
            write_tif(&dem_path, &dem_vals);
            write_tif(&nir_path, &nir_vals);
            write_tif(&red_path, &red_vals);
        }

        let inspected =
            inspect_raster_source(&runtime, &dem_path.to_string_lossy()).expect("inspect dem");
        assert_eq!(inspected["metadata"]["width"], 4);
        assert_eq!(inspected["metadata"]["height"], 4);

        let slope_out = dir.join("slope.tif");
        let slope_res = process_raster_window(
            &runtime,
            &RasterProcessRequest {
                sources: vec![dem_path.to_string_lossy().to_string()],
                output_path: slope_out.clone(),
                operation: "slope".to_owned(),
                crop_bbox: None,
                band_a: None,
                band_b: None,
                azimuth_deg: None,
                altitude_deg: None,
                z_factor: None,
                ..Default::default()
            },
        )
        .expect("compute slope");
        assert!(slope_out.is_file());
        assert!(slope_res["statistics"]["max"].as_f64().unwrap() > 0.5);

        let ndvi_out = dir.join("ndvi.tif");
        let ndvi_res = process_raster_window(
            &runtime,
            &RasterProcessRequest {
                sources: vec![
                    nir_path.to_string_lossy().to_string(),
                    red_path.to_string_lossy().to_string(),
                ],
                output_path: ndvi_out.clone(),
                operation: "ndvi".to_owned(),
                crop_bbox: Some([117.0, 31.97, 117.03, 32.0]),
                band_a: Some(1),
                band_b: Some(1),
                azimuth_deg: None,
                altitude_deg: None,
                z_factor: None,
                ..Default::default()
            },
        )
        .expect("compute ndvi on cropped window");
        assert!(ndvi_out.is_file());
        let mean_ndvi = ndvi_res["statistics"]["mean"].as_f64().unwrap();
        assert!(
            (mean_ndvi - 0.5).abs() < 1e-4,
            "expected (0.6-0.2)/(0.6+0.2) = 0.5"
        );

        let (prev_png, prev_bbox) =
            render_raster_preview_png(&runtime, &dem_path, 256).expect("render dem preview png");
        assert!(prev_png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!((prev_bbox[0] - 117.0).abs() < 1e-5);
        assert!((prev_bbox[3] - 32.0).abs() < 1e-5);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore = "requires optional native GIS runtime (run scripts/run_native_gis_tests.sh with INSAR_GIS_RUNTIME_ROOT)"]
    fn native_cog_driver_webp_compression_and_overview_decimation_work() {
        let runtime = require_runtime();
        let dir = std::env::temp_dir().join(format!(
            "insar-gis-cog-webp-decimate-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        // 1. Create a 256x256 PNG tile and stitch into both native COG+DEFLATE and COG+WEBP
        let tile_path = dir.join("tile_0_0.png");
        let mut rgba = vec![0u8; 256 * 256 * 4];
        for y in 0..256usize {
            for x in 0..256usize {
                let idx = (y * 256 + x) * 4;
                rgba[idx] = x as u8;
                rgba[idx + 1] = y as u8;
                rgba[idx + 2] = 128;
                rgba[idx + 3] = 255;
            }
        }
        let tile_png = encode_rgba_png(256, 256, &rgba).expect("encode tile png");
        fs::write(&tile_path, &tile_png).unwrap();

        let epsg3857_wkt = r#"PROJCS["WGS 84 / Pseudo-Mercator",GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433]],PROJECTION["Mercator_1SP"],PARAMETER["central_meridian",0],PARAMETER["scale_factor",1],PARAMETER["false_easting",0],PARAMETER["false_northing",0],UNIT["metre",1]]"#;
        let gt = [13_050_000.0, 10.0, 0.0, 3_750_000.0, 0.0, -10.0];

        let cog_webp_path = dir.join("mosaic_cog_webp.tif");
        let stitch_res = stitch_xyz_tiles_to_geotiff(
            &runtime,
            &[(10, 20, tile_path.clone())],
            10,
            10,
            20,
            20,
            gt,
            epsg3857_wkt,
            &cog_webp_path,
            "WEBP",
            true,
            "cog",
        )
        .expect("stitch native COG with WEBP compression");
        assert_eq!(stitch_res["ok"], true);
        assert_eq!(stitch_res["export_format"], "cog");
        assert_eq!(stitch_res["compression"], "WEBP");
        assert_eq!(stitch_res["pyramids_built"], true);
        assert!(cog_webp_path.is_file());

        let cog_info = inspect_raster_source(&runtime, &cog_webp_path.to_string_lossy())
            .expect("inspect COG WEBP output");
        assert_eq!(cog_info["metadata"]["layout"], "COG");
        assert_eq!(cog_info["metadata"]["compression"], "WEBP");

        // Verify EPSG:3857 -> WGS84 preview bbox conversion & PNG header
        let (mosaic_png, wgs84_bbox) =
            render_raster_preview_png(&runtime, &cog_webp_path, 128).expect("mosaic preview png");
        assert!(mosaic_png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(
            wgs84_bbox[0] > 117.0
                && wgs84_bbox[2] < 118.0
                && wgs84_bbox[1] > 31.0
                && wgs84_bbox[3] < 33.0,
            "EPSG:3857 Mercator bounds must convert to Hefei WGS84 degrees, got: {:?}",
            wgs84_bbox
        );

        // 2. Test >4096 pixel window decimation preserving full spatial extent without OOM
        let large_tif = dir.join("large_5000x2500.tif");
        let wkt_4326 = CString::new(
            r#"GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433],AUTHORITY["EPSG","4326"]]"#,
        )
        .unwrap();
        let mut large_tf = [116.0, 0.0004, 0.0, 33.0, 0.0, -0.0004]; // covers [116.0, 32.0, 118.0, 33.0] across 5000x2500 px
        unsafe {
            configure_gdal_runtime_for_source(&runtime, false).unwrap();
            let get_driver = *runtime
                .gdal_symbol::<GdalGetDriverByName>(b"GDALGetDriverByName\0")
                .unwrap();
            let create = *runtime.gdal_symbol::<GdalCreate>(b"GDALCreate\0").unwrap();
            let close = *runtime.gdal_symbol::<GdalClose>(b"GDALClose\0").unwrap();
            let set_proj = *runtime
                .gdal_symbol::<GdalSetProjection>(b"GDALSetProjection\0")
                .unwrap();
            let set_tf = *runtime
                .gdal_symbol::<GdalSetGeoTransform>(b"GDALSetGeoTransform\0")
                .unwrap();
            let flush = *runtime
                .gdal_symbol::<GdalFlushCache>(b"GDALFlushCache\0")
                .unwrap();
            let drv_name = CString::new("GTiff").unwrap();
            let drv = get_driver(drv_name.as_ptr());
            let opt = CString::new("COMPRESS=DEFLATE").unwrap();
            let opts = [opt.as_ptr(), std::ptr::null()];
            let pc = c_path(
                &large_tif,
                NativeGisErrorCode::DemInvalidRaster,
                "large tif",
            )
            .unwrap();
            let ds = create(drv, pc.as_ptr(), 5000, 2500, 1, GDAL_BYTE, opts.as_ptr());
            let g = GdalDatasetGuard::new(ds, close);
            set_proj(g.raw(), wkt_4326.as_ptr());
            set_tf(g.raw(), large_tf.as_mut_ptr());
            flush(g.raw());
        }

        let clipped_large = dir.join("clipped_full_extent.tif");
        let clip_res = process_raster_window(
            &runtime,
            &RasterProcessRequest {
                sources: vec![large_tif.to_string_lossy().to_string()],
                output_path: clipped_large.clone(),
                operation: "clip".to_owned(),
                crop_bbox: Some([116.0, 32.0, 118.0, 33.0]),
                band_a: Some(1),
                band_b: None,
                azimuth_deg: None,
                altitude_deg: None,
                z_factor: None,
                ..Default::default()
            },
        )
        .expect("clip 5000x2500 window via 4096 overview decimation");
        assert_eq!(clip_res["width"], 4096);
        assert_eq!(clip_res["height"], 2048);
        let bounds = clip_res["bounds"].as_array().unwrap();
        assert!((bounds[0].as_f64().unwrap() - 116.0).abs() < 1e-6);
        assert!((bounds[1].as_f64().unwrap() - 32.0).abs() < 1e-6);
        assert!((bounds[2].as_f64().unwrap() - 118.0).abs() < 1e-6);
        assert!((bounds[3].as_f64().unwrap() - 33.0).abs() < 1e-6);

        // 1. Multi-band UInt16 scientific clip preserves dtype, all bands, and exact pixel values
        let mb_u16_path = dir.join("multiband_u16.tif");
        unsafe {
            let drv_c = CString::new("GTiff").unwrap();
            let drv = (*runtime
                .gdal_symbol::<GdalGetDriverByName>(b"GDALGetDriverByName\0")
                .unwrap())(drv_c.as_ptr());
            let create = *runtime.gdal_symbol::<GdalCreate>(b"GDALCreate\0").unwrap();
            let close = *runtime.gdal_symbol::<GdalClose>(b"GDALClose\0").unwrap();
            let flush = *runtime
                .gdal_symbol::<GdalFlushCache>(b"GDALFlushCache\0")
                .unwrap();
            let get_band = *runtime
                .gdal_symbol::<GdalGetRasterBand>(b"GDALGetRasterBand\0")
                .unwrap();
            let raster_io = *runtime
                .gdal_symbol::<GdalRasterIo>(b"GDALRasterIO\0")
                .unwrap();
            let set_proj = *runtime
                .gdal_symbol::<GdalSetProjection>(b"GDALSetProjection\0")
                .unwrap();
            let set_tf = *runtime
                .gdal_symbol::<GdalSetGeoTransform>(b"GDALSetGeoTransform\0")
                .unwrap();
            let set_nd = *runtime
                .gdal_symbol::<GdalSetRasterNoDataValue>(b"GDALSetRasterNoDataValue\0")
                .unwrap();
            let pc = c_path(
                &mb_u16_path,
                NativeGisErrorCode::DemInvalidRaster,
                "u16 test",
            )
            .unwrap();
            let ds = create(drv, pc.as_ptr(), 8, 8, 3, 2, std::ptr::null());
            assert!(!ds.is_null());
            let g = GdalDatasetGuard::new(ds, close);
            let mut tf = [117.0, 0.01, 0.0, 32.0, 0.0, -0.01];
            let crs_wkt = CString::new("EPSG:4326").unwrap();
            let _ = set_proj(g.raw(), crs_wkt.as_ptr());
            let _ = set_tf(g.raw(), tf.as_mut_ptr());
            for band_idx in 1..=3 {
                let b = get_band(g.raw(), band_idx);
                let _ = set_nd(b, 0.0);
                let mut data: Vec<u16> = (0..64)
                    .map(|idx| ((band_idx as u16) * 1000) + (idx as u16) + 1)
                    .collect();
                assert_eq!(
                    raster_io(
                        b,
                        GDAL_WRITE,
                        0,
                        0,
                        8,
                        8,
                        data.as_mut_ptr().cast(),
                        8,
                        8,
                        2,
                        0,
                        0,
                    ),
                    0
                );
            }
            flush(g.raw());
        }
        let u16_clip_out = dir.join("multiband_u16_clip.tif");
        let u16_clip_res = process_raster_window(
            &runtime,
            &RasterProcessRequest {
                sources: vec![mb_u16_path.to_string_lossy().to_string()],
                output_path: u16_clip_out.clone(),
                operation: "clip".to_owned(),
                crop_bbox: Some([117.011, 31.941, 117.049, 31.979]),
                ..Default::default()
            },
        )
        .expect("exact dtype multi-band clip");
        assert_eq!(u16_clip_res["band_count"], 3);
        assert_eq!(u16_clip_res["dtype"], json!(["uint16", "uint16", "uint16"]));
        let inspected_u16 =
            inspect_raster_source(&runtime, &u16_clip_out.to_string_lossy()).expect("inspect u16");
        assert_eq!(inspected_u16["metadata"]["count"], 3);
        assert_eq!(
            inspected_u16["metadata"]["dtype"],
            json!(["uint16", "uint16", "uint16"])
        );
        assert_eq!(inspected_u16["metadata"]["width"], 4);
        assert_eq!(inspected_u16["metadata"]["height"], 4);
        unsafe {
            let (g, _) = open_raster_dataset_ro(&runtime, &u16_clip_out.to_string_lossy()).unwrap();
            let get_band = *runtime
                .gdal_symbol::<GdalGetRasterBand>(b"GDALGetRasterBand\0")
                .unwrap();
            let raster_io = *runtime
                .gdal_symbol::<GdalRasterIo>(b"GDALRasterIO\0")
                .unwrap();
            let mut b3 = vec![0u16; 16];
            assert_eq!(
                raster_io(
                    get_band(g.raw(), 3),
                    GDAL_READ,
                    0,
                    0,
                    4,
                    4,
                    b3.as_mut_ptr().cast(),
                    4,
                    4,
                    2,
                    0,
                    0,
                ),
                0
            );
            // Row 2..6, Col 1..5 of original 8x8 -> index 2*8+1 = 17 => 3000 + 17 + 1 = 3018
            assert_eq!(b3[0], 3018);
        }

        // 2. ENVI format translate generates both data binary and companion .hdr
        let envi_out = dir.join("translated_envi.bin");
        let envi_res = process_raster_window(
            &runtime,
            &RasterProcessRequest {
                sources: vec![mb_u16_path.to_string_lossy().to_string()],
                output_path: envi_out.clone(),
                operation: "translate".to_owned(),
                output_format: Some("ENVI".to_owned()),
                ..Default::default()
            },
        )
        .expect("translate to ENVI");
        assert_eq!(envi_res["driver"], "ENVI");
        assert!(envi_out.is_file());
        assert!(dir.join("translated_envi.hdr").is_file());
        let inspected_envi =
            inspect_raster_source(&runtime, &envi_out.to_string_lossy()).expect("inspect envi");
        assert_eq!(inspected_envi["metadata"]["driver"], "ENVI");
        assert_eq!(
            inspected_envi["metadata"]["dtype"],
            json!(["uint16", "uint16", "uint16"])
        );

        // 3. Reproject Int16 DEM from EPSG:4326 to EPSG:32650 via GDALWarp
        let i16_dem_path = dir.join("dem_i16.tif");
        let i16_tile2_path = dir.join("dem_i16_east.tif");
        let i16_tile_f32_path = dir.join("dem_f32_mismatch.tif");
        unsafe {
            let drv_c = CString::new("GTiff").unwrap();
            let drv = (*runtime
                .gdal_symbol::<GdalGetDriverByName>(b"GDALGetDriverByName\0")
                .unwrap())(drv_c.as_ptr());
            let create = *runtime.gdal_symbol::<GdalCreate>(b"GDALCreate\0").unwrap();
            let close = *runtime.gdal_symbol::<GdalClose>(b"GDALClose\0").unwrap();
            let flush = *runtime
                .gdal_symbol::<GdalFlushCache>(b"GDALFlushCache\0")
                .unwrap();
            let get_band = *runtime
                .gdal_symbol::<GdalGetRasterBand>(b"GDALGetRasterBand\0")
                .unwrap();
            let raster_io = *runtime
                .gdal_symbol::<GdalRasterIo>(b"GDALRasterIO\0")
                .unwrap();
            let set_proj = *runtime
                .gdal_symbol::<GdalSetProjection>(b"GDALSetProjection\0")
                .unwrap();
            let set_tf = *runtime
                .gdal_symbol::<GdalSetGeoTransform>(b"GDALSetGeoTransform\0")
                .unwrap();
            let set_nd = *runtime
                .gdal_symbol::<GdalSetRasterNoDataValue>(b"GDALSetRasterNoDataValue\0")
                .unwrap();

            for (path, origin_lon) in [(&i16_dem_path, 117.0), (&i16_tile2_path, 117.1)] {
                let pc = c_path(path, NativeGisErrorCode::DemInvalidRaster, "i16 tile").unwrap();
                let ds = create(drv, pc.as_ptr(), 10, 10, 1, 3, std::ptr::null());
                let g = GdalDatasetGuard::new(ds, close);
                let mut tf = [origin_lon, 0.01, 0.0, 32.0, 0.0, -0.01];
                let crs_wkt = CString::new("EPSG:4326").unwrap();
                let _ = set_proj(g.raw(), crs_wkt.as_ptr());
                let _ = set_tf(g.raw(), tf.as_mut_ptr());
                let b = get_band(g.raw(), 1);
                let _ = set_nd(b, -32768.0);
                let mut data: Vec<i16> = (0..100).map(|idx| 120 + (idx as i16)).collect();
                assert_eq!(
                    raster_io(
                        b,
                        GDAL_WRITE,
                        0,
                        0,
                        10,
                        10,
                        data.as_mut_ptr().cast(),
                        10,
                        10,
                        3,
                        0,
                        0,
                    ),
                    0
                );
                flush(g.raw());
            }

            let pc = c_path(
                &i16_tile_f32_path,
                NativeGisErrorCode::DemInvalidRaster,
                "f32 tile",
            )
            .unwrap();
            let ds = create(drv, pc.as_ptr(), 10, 10, 1, GDAL_FLOAT32, std::ptr::null());
            let g = GdalDatasetGuard::new(ds, close);
            let mut tf = [117.2, 0.01, 0.0, 32.0, 0.0, -0.01];
            let crs_wkt = CString::new("EPSG:4326").unwrap();
            let _ = set_proj(g.raw(), crs_wkt.as_ptr());
            let _ = set_tf(g.raw(), tf.as_mut_ptr());
            let mut data = vec![50.0f32; 100];
            assert_eq!(
                raster_io(
                    get_band(g.raw(), 1),
                    GDAL_WRITE,
                    0,
                    0,
                    10,
                    10,
                    data.as_mut_ptr().cast(),
                    10,
                    10,
                    GDAL_FLOAT32,
                    0,
                    0,
                ),
                0
            );
            flush(g.raw());
        }

        let reproj_out = dir.join("dem_utm50n.tif");
        let reproj_res = process_raster_window(
            &runtime,
            &RasterProcessRequest {
                sources: vec![i16_dem_path.to_string_lossy().to_string()],
                output_path: reproj_out.clone(),
                operation: "reproject".to_owned(),
                target_crs: Some("EPSG:32650".to_owned()),
                resampling: Some("near".to_owned()),
                ..Default::default()
            },
        )
        .expect("reproject int16 raster to EPSG:32650");
        assert!(reproj_out.is_file());
        assert_eq!(reproj_res["dtype"], json!(["int16"]));
        let reproj_crs = reproj_res["crs"].as_str().unwrap_or_default();
        assert!(
            reproj_crs.contains("32650") || reproj_crs.contains("UTM zone 50N"),
            "unexpected projected crs: {reproj_crs}"
        );
        let reproj_gt = reproj_res["geotransform"].as_array().unwrap();
        assert!(reproj_gt[0].as_f64().unwrap() > 100_000.0);

        // 4. Multi-tile mosaic merges extent and fails closed on dtype mismatch
        let mosaic_out = dir.join("dem_mosaic.tif");
        let mosaic_res = process_raster_window(
            &runtime,
            &RasterProcessRequest {
                sources: vec![
                    i16_dem_path.to_string_lossy().to_string(),
                    i16_tile2_path.to_string_lossy().to_string(),
                ],
                output_path: mosaic_out.clone(),
                operation: "mosaic".to_owned(),
                resampling: Some("near".to_owned()),
                ..Default::default()
            },
        )
        .expect("mosaic two i16 tiles");
        assert!(mosaic_out.is_file());
        assert_eq!(mosaic_res["width"], 20);
        assert_eq!(mosaic_res["height"], 10);
        assert_eq!(mosaic_res["dtype"], json!(["int16"]));

        let mismatch_err = process_raster_window(
            &runtime,
            &RasterProcessRequest {
                sources: vec![
                    i16_dem_path.to_string_lossy().to_string(),
                    i16_tile_f32_path.to_string_lossy().to_string(),
                ],
                output_path: dir.join("mismatch_mosaic.tif"),
                operation: "mosaic".to_owned(),
                ..Default::default()
            },
        )
        .expect_err("mosaic must reject mismatched dtypes");
        assert!(mismatch_err.to_string().contains("dtype"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore = "requires optional native GIS runtime (run scripts/run_native_gis_tests.sh with INSAR_GIS_RUNTIME_ROOT)"]
    fn probe_raster_pixel_reads_scaled_multiband_and_scl_semantics_across_crs() {
        let runtime = require_runtime();
        let dir =
            std::env::temp_dir().join(format!("insar-gis-pixel-probe-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        type GdalSetDescriptionFn = unsafe extern "C" fn(*mut c_void, *const c_char);
        type GdalSetRasterScaleFn = unsafe extern "C" fn(*mut c_void, f64) -> c_int;

        let tif_path = dir.join("probe_32650_multiband.tif");
        // UTM Zone 50N bounds covering Hefei center (~117.211E, 31.825N -> 520000E, 3521000N)
        let wkt_32650 = CString::new(
            r#"PROJCS["WGS 84 / UTM zone 50N",GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433]],PROJECTION["Transverse_Mercator"],PARAMETER["latitude_of_origin",0],PARAMETER["central_meridian",117],PARAMETER["scale_factor",0.9996],PARAMETER["false_easting",500000],PARAMETER["false_northing",0],UNIT["metre",1],AUTHORITY["EPSG","32650"]]"#,
        )
        .unwrap();
        let origin_x = 520_000.0_f64;
        let origin_y = 3_521_000.0_f64;
        let pixel_size = 100.0_f64;
        let mut tf = [origin_x, pixel_size, 0.0, origin_y, 0.0, -pixel_size];

        unsafe {
            configure_gdal_runtime_for_source(&runtime, false).unwrap();
            let get_driver = *runtime
                .gdal_symbol::<GdalGetDriverByName>(b"GDALGetDriverByName\0")
                .unwrap();
            let create = *runtime.gdal_symbol::<GdalCreate>(b"GDALCreate\0").unwrap();
            let close = *runtime.gdal_symbol::<GdalClose>(b"GDALClose\0").unwrap();
            let set_proj = *runtime
                .gdal_symbol::<GdalSetProjection>(b"GDALSetProjection\0")
                .unwrap();
            let set_tf = *runtime
                .gdal_symbol::<GdalSetGeoTransform>(b"GDALSetGeoTransform\0")
                .unwrap();
            let get_band = *runtime
                .gdal_symbol::<GdalGetRasterBand>(b"GDALGetRasterBand\0")
                .unwrap();
            let set_desc = *runtime
                .gdal_symbol::<GdalSetDescriptionFn>(b"GDALSetDescription\0")
                .unwrap();
            let set_scale = *runtime
                .gdal_symbol::<GdalSetRasterScaleFn>(b"GDALSetRasterScale\0")
                .unwrap();
            let set_nodata = *runtime
                .gdal_symbol::<GdalSetRasterNoDataValue>(b"GDALSetRasterNoDataValue\0")
                .unwrap();
            let raster_io = *runtime
                .gdal_symbol::<GdalRasterIo>(b"GDALRasterIO\0")
                .unwrap();
            let flush = *runtime
                .gdal_symbol::<GdalFlushCache>(b"GDALFlushCache\0")
                .unwrap();

            let drv_name = CString::new("GTiff").unwrap();
            let drv = get_driver(drv_name.as_ptr());
            let opts = [std::ptr::null()];
            let pc = c_path(&tif_path, NativeGisErrorCode::DemInvalidRaster, "probe tif").unwrap();
            // 4x4, 2 bands Float32
            let ds = create(drv, pc.as_ptr(), 4, 4, 2, GDAL_FLOAT32, opts.as_ptr());
            assert!(!ds.is_null());
            let g = GdalDatasetGuard::new(ds, close);
            set_proj(g.raw(), wkt_32650.as_ptr());
            set_tf(g.raw(), tf.as_mut_ptr());

            let b1 = get_band(g.raw(), 1);
            let b1_name = CString::new("B04").unwrap();
            set_desc(b1, b1_name.as_ptr());
            set_scale(b1, 0.0001);
            set_nodata(b1, -9999.0);
            let mut b1_data = vec![1000.0_f32; 16];
            b1_data[4 + 2] = 1500.0; // row 1, col 2
            b1_data[0] = -9999.0; // row 0, col 0 = nodata
            raster_io(
                b1,
                GDAL_WRITE,
                0,
                0,
                4,
                4,
                b1_data.as_mut_ptr().cast::<c_void>(),
                4,
                4,
                GDAL_FLOAT32,
                0,
                0,
            );

            let b2 = get_band(g.raw(), 2);
            let b2_name = CString::new("SCL").unwrap();
            set_desc(b2, b2_name.as_ptr());
            let mut b2_data = vec![5.0_f32; 16];
            b2_data[4 + 2] = 4.0; // row 1, col 2 = Vegetation
            raster_io(
                b2,
                GDAL_WRITE,
                0,
                0,
                4,
                4,
                b2_data.as_mut_ptr().cast::<c_void>(),
                4,
                4,
                GDAL_FLOAT32,
                0,
                0,
            );
            flush(g.raw());
        }

        // Convert small symmetric box around cell (col=2, row=1) -> center in WGS84
        let px_center_x = origin_x + 2.5 * pixel_size;
        let px_center_y = origin_y - 1.5 * pixel_size;
        let wgs84_box = transform_bbox_with_proj(
            &runtime,
            "EPSG:32650",
            "EPSG:4326",
            [
                px_center_x - 5.0,
                px_center_y - 5.0,
                px_center_x + 5.0,
                px_center_y + 5.0,
            ],
        )
        .expect("project UTM 50N pixel center to WGS84");
        let wgs84_pt = [
            (wgs84_box[0] + wgs84_box[2]) * 0.5,
            (wgs84_box[1] + wgs84_box[3]) * 0.5,
        ];

        let hit = probe_raster_pixel(
            &runtime,
            &RasterPixelProbeRequest {
                source: tif_path.to_string_lossy().to_string(),
                lon: wgs84_pt[0],
                lat: wgs84_pt[1],
            },
        )
        .expect("probe pixel inside bounds");
        assert_eq!(hit["in_bounds"], true);
        assert_eq!(hit["col"], 2);
        assert_eq!(hit["row"], 1);
        let bands = hit["bands"].as_array().unwrap();
        assert_eq!(bands.len(), 2);
        assert_eq!(bands[0]["name"], "B04");
        assert_eq!(bands[0]["is_nodata"], false);
        assert!((bands[0]["physical_value"].as_f64().unwrap() - 0.15).abs() < 1e-6);
        assert_eq!(bands[1]["name"], "SCL");
        assert_eq!(bands[1]["qa_semantics"], "SCL=4: Vegetation (植被)");

        let nd_x = origin_x + 0.5 * pixel_size;
        let nd_y = origin_y - 0.5 * pixel_size;
        let nodata_box = transform_bbox_with_proj(
            &runtime,
            "EPSG:32650",
            "EPSG:4326",
            [nd_x - 5.0, nd_y - 5.0, nd_x + 5.0, nd_y + 5.0],
        )
        .unwrap();
        let nodata_center = [
            (nodata_box[0] + nodata_box[2]) * 0.5,
            (nodata_box[1] + nodata_box[3]) * 0.5,
        ];
        let nd_hit = probe_raster_pixel(
            &runtime,
            &RasterPixelProbeRequest {
                source: tif_path.to_string_lossy().to_string(),
                lon: nodata_center[0],
                lat: nodata_center[1],
            },
        )
        .unwrap();
        assert_eq!(nd_hit["bands"][0]["is_nodata"], true);
        assert!(nd_hit["bands"][0]["physical_value"].is_null());

        let outside = probe_raster_pixel(
            &runtime,
            &RasterPixelProbeRequest {
                source: tif_path.to_string_lossy().to_string(),
                lon: wgs84_pt[0] + 5.0,
                lat: wgs84_pt[1] + 5.0,
            },
        )
        .unwrap();
        assert_eq!(outside["in_bounds"], false);
        assert_eq!(outside["bands"].as_array().unwrap().len(), 0);

        let _ = fs::remove_dir_all(&dir);
    }
}
