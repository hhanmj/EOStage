use super::runtime::NativeGisRuntime;
use super::{NativeGisError, NativeGisErrorCode};
use serde_json::json;
use std::ffi::{CStr, CString};
use std::fs;
use std::io::{BufWriter, Write};
use std::os::raw::{c_char, c_int, c_uint, c_void};
use std::path::{Path, PathBuf};

type Dataset = *mut c_void;
type RasterBand = *mut c_void;
type Driver = *mut c_void;

const GDAL_OF_RASTER: c_uint = 0x02;
const GDAL_ACCESS_READ_ONLY: c_uint = 0x00;
const GDAL_READ: c_int = 0;
const GDAL_WRITE: c_int = 1;
const GDAL_FLOAT32: c_int = 6;
const MAX_NATIVE_MOSAIC_WIDTH: u64 = 32 * 1024;
const MAX_NATIVE_MOSAIC_HEIGHT: u64 = 32 * 1024;
const MAX_NATIVE_MOSAIC_PIXELS: u64 = 512 * 1024 * 1024;
const MAX_ANTIMERIDIAN_MERGE_WIDTH: u64 = 64 * 1024;
const MAX_ANTIMERIDIAN_MERGE_HEIGHT: u64 = 32 * 1024;
const MAX_ANTIMERIDIAN_MERGE_PIXELS: u64 = 512 * 1024 * 1024;
const PJ_FWD: c_int = 1;
#[cfg(test)]
const PJ_INV: c_int = -1;

type GdalVersionInfo = unsafe extern "C" fn(*const c_char) -> *const c_char;
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
type GdalSetMetadataItem =
    unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char, *const c_char) -> c_int;
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
type GdalBuildVrtOptionsNew = unsafe extern "C" fn(*mut *mut c_char, *mut c_void) -> *mut c_void;
type GdalBuildVrtOptionsFree = unsafe extern "C" fn(*mut c_void);
type GdalBuildVrt = unsafe extern "C" fn(
    *const c_char,
    c_int,
    *mut Dataset,
    *const *const c_char,
    *const c_void,
    *mut c_int,
) -> Dataset;
type GdalTranslateOptionsNew = unsafe extern "C" fn(*mut *mut c_char, *mut c_void) -> *mut c_void;
type GdalTranslateOptionsFree = unsafe extern "C" fn(*mut c_void);
type GdalTranslate =
    unsafe extern "C" fn(*const c_char, Dataset, *const c_void, *mut c_int) -> Dataset;
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

type ProjObject = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct ProjCoord {
    x: f64,
    y: f64,
    z: f64,
    t: f64,
}

type ProjCreate = unsafe extern "C" fn(*mut c_void, *const c_char) -> ProjObject;
type ProjCreateCrsToCrs =
    unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char, *const c_void) -> ProjObject;
type ProjNormalizeForVisualization = unsafe extern "C" fn(*mut c_void, ProjObject) -> ProjObject;
type ProjAsWkt =
    unsafe extern "C" fn(*mut c_void, ProjObject, c_int, *const *const c_char) -> *const c_char;
type ProjDestroy = unsafe extern "C" fn(ProjObject) -> ProjObject;
type ProjTrans = unsafe extern "C" fn(ProjObject, c_int, ProjCoord) -> ProjCoord;

const PJ_WKT2_2019: c_int = 2;

#[repr(C)]
struct ProjInfo {
    major: c_int,
    minor: c_int,
    patch: c_int,
    release: *const c_char,
    version: *const c_char,
    searchpath: *const c_char,
    paths: *const *const c_char,
    path_count: usize,
}

type ProjInfoFn = unsafe extern "C" fn() -> ProjInfo;

/// Verify the capabilities that the product runtime actually exports.  The
/// manifest is still validated before this point, but a manifest declaration
/// alone is not evidence that the loaded GDAL binary contains usable drivers.
pub(crate) fn verify_runtime_capabilities(
    runtime: &NativeGisRuntime,
) -> Result<(), NativeGisError> {
    let proj_data = CString::new(
        runtime
            .root()
            .join(&runtime.manifest().proj_data_dir)
            .to_string_lossy()
            .as_bytes(),
    )
    .map_err(|_| {
        error(
            NativeGisErrorCode::GisEngineUnavailable,
            "PROJ 数据目录编码无效",
        )
    })?;
    let gdal_data = CString::new(
        runtime
            .root()
            .join(&runtime.manifest().gdal_data_dir)
            .to_string_lossy()
            .as_bytes(),
    )
    .map_err(|_| {
        error(
            NativeGisErrorCode::GisEngineUnavailable,
            "GDAL 数据目录编码无效",
        )
    })?;
    let required_drivers = [
        CString::new("GTiff").expect("static GDAL driver"),
        CString::new("ENVI").expect("static GDAL driver"),
    ];
    let create_capability = CString::new("DCAP_CREATE").expect("static GDAL capability");
    let _proj_context = runtime.create_proj_context()?;

    unsafe {
        runtime.configure_gdal_options(
            &[
                ("PROJ_NETWORK", "OFF"),
                ("CPL_CURL_VERBOSE", "NO"),
                ("GDAL_HTTP_MAX_RETRY", "0"),
                ("GDAL_DISABLE_READDIR_ON_OPEN", "EMPTY_DIR"),
                ("GDAL_DRIVER_PATH", ""),
            ],
            &proj_data,
            &gdal_data,
        )?;

        let gdal_version_info = runtime.gdal_symbol::<GdalVersionInfo>(b"GDALVersionInfo\0")?;
        let gdal_release_name = CString::new("RELEASE_NAME").expect("static GDAL version key");
        let gdal_release = gdal_version_info(gdal_release_name.as_ptr());
        let gdal_release = if gdal_release.is_null() {
            String::new()
        } else {
            CStr::from_ptr(gdal_release).to_string_lossy().into_owned()
        };
        if gdal_release != runtime.manifest().gdal_version {
            return Err(error(
                NativeGisErrorCode::GisEngineUnavailable,
                format!(
                    "实际 GDAL runtime 版本 {} 与 manifest 版本 {} 不一致",
                    if gdal_release.is_empty() {
                        "<missing>"
                    } else {
                        &gdal_release
                    },
                    runtime.manifest().gdal_version
                ),
            ));
        }
        let proj_info = runtime.proj_symbol::<ProjInfoFn>(b"proj_info\0")?();
        let proj_version = if proj_info.version.is_null() {
            String::new()
        } else {
            CStr::from_ptr(proj_info.version)
                .to_string_lossy()
                .into_owned()
        };
        if proj_version != runtime.manifest().proj_version {
            return Err(error(
                NativeGisErrorCode::GisEngineUnavailable,
                format!(
                    "实际 PROJ runtime 版本 {} 与 manifest 版本 {} 不一致",
                    if proj_version.is_empty() {
                        "<missing>"
                    } else {
                        &proj_version
                    },
                    runtime.manifest().proj_version
                ),
            ));
        }

        runtime.ensure_gdal_registered()?;
        let get_driver = runtime.gdal_symbol::<GdalGetDriverByName>(b"GDALGetDriverByName\0")?;
        let get_metadata_item =
            runtime.gdal_symbol::<GdalGetMetadataItem>(b"GDALGetMetadataItem\0")?;
        for driver_name in required_drivers {
            let driver = get_driver(driver_name.as_ptr());
            if driver.is_null() {
                return Err(error(
                    NativeGisErrorCode::GisDriverUnavailable,
                    format!(
                        "产品 GDAL runtime 未实际注册所需 driver {}",
                        driver_name.to_string_lossy()
                    ),
                ));
            }
            let capability =
                get_metadata_item(driver, create_capability.as_ptr(), std::ptr::null());
            let supports_create = !capability.is_null()
                && CStr::from_ptr(capability)
                    .to_bytes()
                    .eq_ignore_ascii_case(b"YES");
            if !supports_create {
                return Err(error(
                    NativeGisErrorCode::GisDriverUnavailable,
                    format!(
                        "产品 GDAL runtime 的 driver {} 不支持实际栅格创建",
                        driver_name.to_string_lossy()
                    ),
                ));
            }
        }
    }
    Ok(())
}

pub(crate) fn inspect_dem(
    runtime: &NativeGisRuntime,
    path: &Path,
) -> Result<serde_json::Value, NativeGisError> {
    let metadata = fs::metadata(path)
        .map_err(|_| error(NativeGisErrorCode::DemInvalidRaster, "DEM 文件不可读"))?;
    if !metadata.is_file() {
        return Err(error(
            NativeGisErrorCode::DemInvalidRaster,
            "DEM 文件不可读",
        ));
    }
    let path_text = path
        .to_str()
        .ok_or_else(|| error(NativeGisErrorCode::DemInvalidRaster, "DEM 路径编码无效"))?;
    let path_c = CString::new(path_text)
        .map_err(|_| error(NativeGisErrorCode::DemInvalidRaster, "DEM 路径编码无效"))?;
    let config = [
        ("PROJ_NETWORK", "OFF"),
        ("CPL_CURL_VERBOSE", "NO"),
        ("GDAL_HTTP_MAX_RETRY", "0"),
        ("GDAL_DISABLE_READDIR_ON_OPEN", "EMPTY_DIR"),
        // Do not inherit a host/Conda/QGIS plugin directory.  GTiff and ENVI
        // are required to be supplied by the reviewed core GDAL build.
        ("GDAL_DRIVER_PATH", ""),
    ];
    let proj_data = CString::new(
        runtime
            .root()
            .join(&runtime.manifest().proj_data_dir)
            .to_string_lossy()
            .as_bytes(),
    )
    .map_err(|_| {
        error(
            NativeGisErrorCode::GisEngineUnavailable,
            "PROJ 数据目录编码无效",
        )
    })?;
    let gdal_data = CString::new(
        runtime
            .root()
            .join(&runtime.manifest().gdal_data_dir)
            .to_string_lossy()
            .as_bytes(),
    )
    .map_err(|_| {
        error(
            NativeGisErrorCode::GisEngineUnavailable,
            "GDAL 数据目录编码无效",
        )
    })?;
    let _proj_context = runtime.create_proj_context()?;
    let allowed_driver_names = [
        CString::new("GTiff").expect("static GDAL driver"),
        CString::new("ENVI").expect("static GDAL driver"),
    ];
    let allowed_driver_ptrs = [
        allowed_driver_names[0].as_ptr(),
        allowed_driver_names[1].as_ptr(),
        std::ptr::null(),
    ];
    unsafe {
        runtime.configure_and_register_gdal(&config, &proj_data, &gdal_data)?;
        let open = runtime.gdal_symbol::<GdalOpenEx>(b"GDALOpenEx\0")?;
        let siblings = raster_siblings(path)?;
        let sibling_ptrs: Vec<_> = siblings
            .iter()
            .map(|name| name.as_ptr())
            .chain(std::iter::once(std::ptr::null()))
            .collect();
        let dataset = open(
            path_c.as_ptr(),
            GDAL_OF_RASTER | GDAL_ACCESS_READ_ONLY,
            allowed_driver_ptrs.as_ptr(),
            std::ptr::null(),
            sibling_ptrs.as_ptr(),
        );
        if dataset.is_null() {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                "DEM 不是可读取的栅格",
            ));
        }
        let close = runtime.gdal_symbol::<GdalClose>(b"GDALClose\0")?;
        let width = runtime.gdal_symbol::<GdalGetRasterXSize>(b"GDALGetRasterXSize\0")?(dataset);
        let height = runtime.gdal_symbol::<GdalGetRasterYSize>(b"GDALGetRasterYSize\0")?(dataset);
        let count = runtime.gdal_symbol::<GdalGetRasterCount>(b"GDALGetRasterCount\0")?(dataset);
        if width <= 0 || height <= 0 || count <= 0 {
            close(dataset);
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                "DEM 栅格尺寸无效",
            ));
        }
        let get_projection =
            runtime.gdal_symbol::<GdalGetProjectionRef>(b"GDALGetProjectionRef\0")?;
        let projection = get_projection(dataset);
        let crs = if projection.is_null() || CStr::from_ptr(projection).to_bytes().is_empty() {
            None
        } else {
            Some(CStr::from_ptr(projection).to_string_lossy().to_string())
        };
        let mut transform = [0.0f64; 6];
        let transform_status = runtime
            .gdal_symbol::<GdalGetGeoTransform>(b"GDALGetGeoTransform\0")?(
            dataset,
            transform.as_mut_ptr(),
        );
        let driver =
            runtime.gdal_symbol::<GdalGetDatasetDriver>(b"GDALGetDatasetDriver\0")?(dataset);
        let get_description = runtime.gdal_symbol::<GdalGetDescription>(b"GDALGetDescription\0")?;
        let driver_name = if driver.is_null() {
            String::new()
        } else {
            let description = get_description(driver);
            if description.is_null() {
                String::new()
            } else {
                CStr::from_ptr(description).to_string_lossy().to_string()
            }
        };
        if !runtime.supports_driver(&driver_name) {
            close(dataset);
            return Err(error(
                NativeGisErrorCode::GisDriverUnavailable,
                "DEM 使用了不在产品边界内的 GDAL driver",
            ));
        }
        let get_band = runtime.gdal_symbol::<GdalGetRasterBand>(b"GDALGetRasterBand\0")?;
        let get_type = runtime.gdal_symbol::<GdalGetRasterDataType>(b"GDALGetRasterDataType\0")?;
        let get_nodata =
            runtime.gdal_symbol::<GdalGetRasterNoDataValue>(b"GDALGetRasterNoDataValue\0")?;
        let mut dtypes = Vec::new();
        let mut nodata = None;
        for band_index in 1..=count {
            let band = get_band(dataset, band_index);
            if band.is_null() {
                close(dataset);
                return Err(error(
                    NativeGisErrorCode::DemInvalidRaster,
                    "DEM 波段不可读取",
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
            get_metadata_item(dataset, area_or_point_key.as_ptr(), std::ptr::null());
        let area_or_point = (!area_or_point.is_null())
            .then(|| CStr::from_ptr(area_or_point).to_string_lossy().to_string());
        close(dataset);
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
        Ok(json!({
            "ok": true,
            "path": path.to_string_lossy(),
            "suffix": path.extension().and_then(|value| value.to_str()).unwrap_or_default().to_ascii_lowercase(),
            "size_bytes": metadata.len(),
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
                "nodata": nodata,
                "area_or_point": area_or_point,
            }
        }))
    }
}

/// Validate full-resolution source readability with at most 1 MiB of caller
/// buffer. The caller has initialized the existing read-only inspection context.
pub(crate) fn verify_dem_pixels(
    runtime: &NativeGisRuntime,
    path: &Path,
) -> Result<(), NativeGisError> {
    let path = c_path(
        path,
        NativeGisErrorCode::DemInvalidRaster,
        "DEM path encoding is invalid",
    )?;
    let driver = CString::new("GTiff").expect("static driver");
    let drivers = [driver.as_ptr(), std::ptr::null()];
    unsafe {
        let open = *runtime.gdal_symbol::<GdalOpenEx>(b"GDALOpenEx\0")?;
        let dataset = open(
            path.as_ptr(),
            GDAL_OF_RASTER | GDAL_ACCESS_READ_ONLY,
            drivers.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
        );
        if dataset.is_null() {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                "DEM response cannot be decoded",
            ));
        }
        let close = *runtime.gdal_symbol::<GdalClose>(b"GDALClose\0")?;
        let dataset = GdalDatasetGuard::new(dataset, close);
        let width =
            runtime.gdal_symbol::<GdalGetRasterXSize>(b"GDALGetRasterXSize\0")?(dataset.raw());
        let height =
            runtime.gdal_symbol::<GdalGetRasterYSize>(b"GDALGetRasterYSize\0")?(dataset.raw());
        let count =
            runtime.gdal_symbol::<GdalGetRasterCount>(b"GDALGetRasterCount\0")?(dataset.raw());
        if width <= 0
            || height <= 0
            || count != 1
            || width as u64 * height as u64 > MAX_NATIVE_MOSAIC_PIXELS
        {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                "DEM response exceeds the bounded single-band validation limit",
            ));
        }
        let band =
            runtime.gdal_symbol::<GdalGetRasterBand>(b"GDALGetRasterBand\0")?(dataset.raw(), 1);
        if band.is_null() {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                "DEM response band is missing",
            ));
        }
        let dtype = runtime.gdal_symbol::<GdalGetRasterDataType>(b"GDALGetRasterDataType\0")?(band);
        type DataTypeSize = unsafe extern "C" fn(c_int) -> c_int;
        let bytes = runtime.gdal_symbol::<DataTypeSize>(b"GDALGetDataTypeSizeBytes\0")?(dtype);
        if !(1..=16).contains(&bytes) {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                "DEM response pixel type is invalid",
            ));
        }
        // u64 storage provides alignment even for native Float64 samples.
        let mut buffer = vec![0u64; (1024 * 64 * bytes as usize).div_ceil(8)];
        let read = *runtime.gdal_symbol::<GdalRasterIo>(b"GDALRasterIO\0")?;
        for y in (0..height).step_by(64) {
            for x in (0..width).step_by(1024) {
                let cols = (width - x).min(1024);
                let rows = (height - y).min(64);
                // Equal source/buffer dimensions and native dtype: no overview,
                // scaling, conversion or output encoding is requested.
                if read(
                    band,
                    GDAL_READ,
                    x,
                    y,
                    cols,
                    rows,
                    buffer.as_mut_ptr().cast(),
                    cols,
                    rows,
                    dtype,
                    0,
                    0,
                ) != 0
                {
                    return Err(error(
                        NativeGisErrorCode::DemInvalidRaster,
                        "DEM response pixel data is incomplete or unreadable",
                    ));
                }
            }
        }
    }
    Ok(())
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

struct GdalOptionsGuard {
    options: *mut c_void,
    free: unsafe extern "C" fn(*mut c_void),
}

impl GdalOptionsGuard {
    fn new(options: *mut c_void, free: unsafe extern "C" fn(*mut c_void)) -> Self {
        Self { options, free }
    }

    fn raw(&self) -> *const c_void {
        self.options.cast_const()
    }
}

impl Drop for GdalOptionsGuard {
    fn drop(&mut self) {
        if !self.options.is_null() {
            unsafe { (self.free)(self.options) };
        }
    }
}

struct ProjOperationGuard {
    operation: ProjObject,
    destroy: ProjDestroy,
}

impl ProjOperationGuard {
    fn new(operation: ProjObject, destroy: ProjDestroy) -> Self {
        Self { operation, destroy }
    }

    fn raw(&self) -> ProjObject {
        self.operation
    }
}

impl Drop for ProjOperationGuard {
    fn drop(&mut self) {
        if !self.operation.is_null() {
            unsafe { (self.destroy)(self.operation) };
        }
    }
}

#[cfg(test)]
pub(crate) fn vertical_reference_round_trip(
    runtime: &NativeGisRuntime,
    grid_path: &Path,
    longitude: f64,
    latitude: f64,
    source_height: f64,
) -> Result<(f64, f64), NativeGisError> {
    let grid_parent = grid_path.parent().ok_or_else(|| {
        error(
            NativeGisErrorCode::VerticalTransformFailed,
            "垂直参考网格目录不可解析",
        )
    })?;
    let grid_name = grid_path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            error(
                NativeGisErrorCode::VerticalTransformFailed,
                "垂直参考网格文件名编码无效",
            )
        })?;
    let pipeline = CString::new(format!(
        "+proj=pipeline +step +proj=unitconvert +xy_in=deg +xy_out=rad +step +proj=vgridshift +grids={grid_name} +multiplier=1"
    ))
    .map_err(|_| {
        error(
            NativeGisErrorCode::VerticalTransformFailed,
            "垂直参考 pipeline 编码无效",
        )
    })?;
    let context = runtime.create_proj_context_with_search_paths(&[grid_parent])?;
    unsafe {
        let create = *runtime.proj_symbol::<ProjCreate>(b"proj_create\0")?;
        let destroy = *runtime.proj_symbol::<ProjDestroy>(b"proj_destroy\0")?;
        let transform = *runtime.proj_symbol::<ProjTrans>(b"proj_trans\0")?;
        let operation = create(context.raw(), pipeline.as_ptr());
        if operation.is_null() {
            return Err(error(
                NativeGisErrorCode::VerticalTransformFailed,
                "PROJ 无法建立垂直参考 pipeline",
            ));
        }
        let operation = ProjOperationGuard::new(operation, destroy);
        let forward = transform(
            operation.raw(),
            PJ_FWD,
            ProjCoord {
                x: longitude,
                y: latitude,
                z: source_height,
                t: 0.0,
            },
        );
        if !forward.z.is_finite() {
            return Err(error(
                NativeGisErrorCode::VerticalTransformFailed,
                "PROJ 垂直正向参考值无效",
            ));
        }
        let reverse = transform(operation.raw(), PJ_INV, forward);
        if !reverse.z.is_finite() {
            return Err(error(
                NativeGisErrorCode::VerticalTransformFailed,
                "PROJ 垂直反向参考值无效",
            ));
        }
        Ok((forward.z, reverse.z))
    }
}

// Keep EMPTY_DIR: admit only the selected raster and its exact ENVI headers,
// never enumerate the directory or enable arbitrary auxiliary metadata.
fn raster_siblings(path: &Path) -> Result<Vec<CString>, NativeGisError> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            error(
                NativeGisErrorCode::DemInvalidRaster,
                "DEM filename is invalid",
            )
        })?;
    let mut names = vec![CString::new(name).map_err(|_| {
        error(
            NativeGisErrorCode::DemInvalidRaster,
            "DEM filename is invalid",
        )
    })?];
    for header in [
        path.with_file_name(format!("{name}.hdr")),
        path.with_extension("hdr"),
    ] {
        if let Ok(metadata) = fs::symlink_metadata(&header) {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(error(
                    NativeGisErrorCode::DemInvalidRaster,
                    "DEM header is not a regular local file",
                ));
            }
            let header_name = header
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| {
                    error(
                        NativeGisErrorCode::DemInvalidRaster,
                        "DEM header filename is invalid",
                    )
                })?;
            let item = CString::new(header_name).map_err(|_| {
                error(
                    NativeGisErrorCode::DemInvalidRaster,
                    "DEM header filename is invalid",
                )
            })?;
            if !names.contains(&item) {
                names.push(item);
            }
        }
    }
    Ok(names)
}

pub(crate) fn convert_dem(
    runtime: &NativeGisRuntime,
    grid_path: Option<&Path>,
    input: &Path,
    ellipsoid_output: &Path,
    sarscape_output: Option<&Path>,
    source_model: &str,
) -> Result<serde_json::Value, NativeGisError> {
    let metadata = fs::metadata(input)
        .map_err(|_| error(NativeGisErrorCode::DemInvalidRaster, "DEM 文件不可读"))?;
    if !metadata.is_file() {
        return Err(error(
            NativeGisErrorCode::DemInvalidRaster,
            "DEM 文件不可读",
        ));
    }
    if ellipsoid_output == input || sarscape_output.is_some_and(|path| path == input) {
        return Err(error(
            NativeGisErrorCode::DemInvalidRaster,
            "DEM 输出路径不能覆盖输入文件",
        ));
    }
    if let Some(output) = sarscape_output {
        if output == ellipsoid_output {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                "DEM 输出路径不能相同",
            ));
        }
    }
    let input_c = c_path(
        input,
        NativeGisErrorCode::DemInvalidRaster,
        "DEM 路径编码无效",
    )?;
    let proj_context = if let Some(grid_path) = grid_path {
        let parent = grid_path.parent().ok_or_else(|| {
            error(
                NativeGisErrorCode::VerticalTransformFailed,
                "垂直网格目录不可解析",
            )
        })?;
        Some(runtime.create_proj_context_with_search_paths(&[parent])?)
    } else {
        None
    };
    let allowed_driver_names = [
        CString::new("GTiff").expect("static GDAL driver"),
        CString::new("ENVI").expect("static GDAL driver"),
    ];
    let allowed_driver_ptrs = [
        allowed_driver_names[0].as_ptr(),
        allowed_driver_names[1].as_ptr(),
        std::ptr::null(),
    ];
    let ellipsoid_part = part_path(ellipsoid_output)?;
    let sarscape_part = sarscape_output.map(sarscape_part_path).transpose()?;
    remove_stale_output(&ellipsoid_part)?;
    if let Some(path) = sarscape_part.as_ref() {
        remove_stale_output(path)?;
        remove_stale_output(&sarscape_part_header_path(path)?)?;
        remove_stale_output(&sidecar_path(path, ".sml")?)?;
    }
    if let Some(parent) = ellipsoid_part.parent() {
        fs::create_dir_all(parent).map_err(|_| {
            error(
                NativeGisErrorCode::DemReprojectionFailed,
                "椭球高 DEM 输出目录无法创建",
            )
        })?;
    }
    if let Some(path) = sarscape_part.as_ref().and_then(|path| path.parent()) {
        fs::create_dir_all(path).map_err(|_| {
            error(
                NativeGisErrorCode::DemReprojectionFailed,
                "SARscape DEM 输出目录无法创建",
            )
        })?;
    }

    unsafe {
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
        // Keep the native product boundary independent of host GDAL
        // plugin paths; only the manifest's GTiff/ENVI core drivers count.
        runtime.configure_and_register_gdal(
            &[
                ("PROJ_NETWORK", "OFF"),
                ("CPL_CURL_VERBOSE", "NO"),
                ("GDAL_HTTP_MAX_RETRY", "0"),
                ("GDAL_DISABLE_READDIR_ON_OPEN", "EMPTY_DIR"),
                ("GDAL_DRIVER_PATH", ""),
            ],
            &proj_data,
            &gdal_data,
        )?;
        let close = *runtime.gdal_symbol::<GdalClose>(b"GDALClose\0")?;
        let open = *runtime.gdal_symbol::<GdalOpenEx>(b"GDALOpenEx\0")?;
        let siblings = raster_siblings(input)?;
        let sibling_ptrs: Vec<_> = siblings
            .iter()
            .map(|name| name.as_ptr())
            .chain(std::iter::once(std::ptr::null()))
            .collect();
        let input_dataset = open(
            input_c.as_ptr(),
            GDAL_OF_RASTER | GDAL_ACCESS_READ_ONLY,
            allowed_driver_ptrs.as_ptr(),
            std::ptr::null(),
            sibling_ptrs.as_ptr(),
        );
        if input_dataset.is_null() {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                "DEM 不是产品允许的 GTiff/ENVI 栅格",
            ));
        }
        let input_dataset = GdalDatasetGuard::new(input_dataset, close);
        let get_width = *runtime.gdal_symbol::<GdalGetRasterXSize>(b"GDALGetRasterXSize\0")?;
        let get_height = *runtime.gdal_symbol::<GdalGetRasterYSize>(b"GDALGetRasterYSize\0")?;
        let get_count = *runtime.gdal_symbol::<GdalGetRasterCount>(b"GDALGetRasterCount\0")?;
        let width = get_width(input_dataset.raw());
        let height = get_height(input_dataset.raw());
        let count = get_count(input_dataset.raw());
        if width <= 0 || height <= 0 || count <= 0 {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                "DEM 栅格尺寸无效",
            ));
        }
        let get_projection =
            *runtime.gdal_symbol::<GdalGetProjectionRef>(b"GDALGetProjectionRef\0")?;
        let projection_ptr = get_projection(input_dataset.raw());
        if projection_ptr.is_null() || CStr::from_ptr(projection_ptr).to_bytes().is_empty() {
            return Err(error(
                NativeGisErrorCode::DemCrsMissing,
                "DEM 缺少水平 CRS，拒绝执行垂直转换",
            ));
        }
        let projection = CStr::from_ptr(projection_ptr).to_string_lossy().to_string();
        if !is_wgs84_geographic_crs(&projection) {
            return Err(error(
                NativeGisErrorCode::DemReprojectionFailed,
                "DEM CRS must be geographic WGS84 lon/lat; native path does not reproject",
            ));
        }
        let projection_c = CString::new(projection.as_bytes())
            .map_err(|_| error(NativeGisErrorCode::DemCrsMissing, "DEM CRS 编码无效"))?;
        let get_transform =
            *runtime.gdal_symbol::<GdalGetGeoTransform>(b"GDALGetGeoTransform\0")?;
        let mut transform = [0.0f64; 6];
        if get_transform(input_dataset.raw(), transform.as_mut_ptr()) != 0 {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                "DEM 地理变换参数不可读取",
            ));
        }
        if sarscape_output.is_some()
            && (transform[1] <= 0.0
                || transform[5] >= 0.0
                || transform[2] != 0.0
                || transform[4] != 0.0)
        {
            return Err(error(
                NativeGisErrorCode::DemReprojectionFailed,
                "SARscape ENVI 输出只接受未旋转的北向上 DEM",
            ));
        }
        let get_driver = *runtime.gdal_symbol::<GdalGetDatasetDriver>(b"GDALGetDatasetDriver\0")?;
        let get_description =
            *runtime.gdal_symbol::<GdalGetDescription>(b"GDALGetDescription\0")?;
        let driver = get_driver(input_dataset.raw());
        let driver_name = if driver.is_null() {
            String::new()
        } else {
            let description = get_description(driver);
            if description.is_null() {
                String::new()
            } else {
                CStr::from_ptr(description).to_string_lossy().to_string()
            }
        };
        if !runtime.supports_driver(&driver_name) {
            return Err(error(
                NativeGisErrorCode::GisDriverUnavailable,
                "DEM 使用了不在产品边界内的 GDAL driver",
            ));
        }
        let get_band = *runtime.gdal_symbol::<GdalGetRasterBand>(b"GDALGetRasterBand\0")?;
        let input_band = get_band(input_dataset.raw(), 1);
        if input_band.is_null() {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                "DEM 第一波段不可读取",
            ));
        }
        let get_nodata =
            *runtime.gdal_symbol::<GdalGetRasterNoDataValue>(b"GDALGetRasterNoDataValue\0")?;
        let mut has_nodata = 0;
        let nodata = get_nodata(input_band, &mut has_nodata);
        let nodata = (has_nodata != 0).then_some(nodata);
        let get_metadata_item =
            *runtime.gdal_symbol::<GdalGetMetadataItem>(b"GDALGetMetadataItem\0")?;
        let area_or_point_key = CString::new("AREA_OR_POINT").expect("static GDAL metadata key");
        let source_area_or_point = get_metadata_item(
            input_dataset.raw(),
            area_or_point_key.as_ptr(),
            std::ptr::null(),
        );
        let source_area_or_point = (!source_area_or_point.is_null()).then(|| {
            CStr::from_ptr(source_area_or_point)
                .to_string_lossy()
                .to_string()
        });

        let operation = if let Some(grid_path) = grid_path {
            let grid_name = grid_path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| {
                    error(
                        NativeGisErrorCode::VerticalTransformFailed,
                        "垂直网格文件名编码无效",
                    )
                })?;
            let pipeline = CString::new(format!(
                "+proj=pipeline +step +proj=unitconvert +xy_in=deg +xy_out=rad +step +proj=vgridshift +grids={grid_name} +multiplier=1"
            ))
            .map_err(|_| {
                error(
                    NativeGisErrorCode::VerticalTransformFailed,
                    "垂直转换 pipeline 编码无效",
                )
            })?;
            let create = *runtime.proj_symbol::<ProjCreate>(b"proj_create\0")?;
            let destroy = *runtime.proj_symbol::<ProjDestroy>(b"proj_destroy\0")?;
            let transform = *runtime.proj_symbol::<ProjTrans>(b"proj_trans\0")?;
            let context = proj_context.as_ref().ok_or_else(|| {
                error(
                    NativeGisErrorCode::VerticalTransformFailed,
                    "垂直转换缺少 PROJ context",
                )
            })?;
            let operation = create(context.raw(), pipeline.as_ptr());
            if operation.is_null() {
                return Err(error(
                    NativeGisErrorCode::VerticalTransformFailed,
                    "PROJ 无法建立垂直网格转换 pipeline",
                ));
            }
            Some((ProjOperationGuard::new(operation, destroy), transform))
        } else {
            if source_model != "WGS84_ELLIPSOID" {
                return Err(error(
                    NativeGisErrorCode::VerticalGridMissing,
                    "DEM 垂直基准没有对应的已审查网格",
                ));
            }
            None
        };
        // ENVI Create must not run while the source dataset is still open.
        // Reopen the bounded source only after each output dataset exists;
        // this also keeps input and output handles independently scoped.
        drop(input_dataset);

        if let (Some(_), Some(part)) = (sarscape_output, sarscape_part.as_ref()) {
            write_raster_output(RasterOutputRequest {
                runtime,
                input,
                width,
                height,
                projection: &projection_c,
                transform: &mut transform,
                nodata,
                area_or_point: source_area_or_point.as_deref(),
                operation: operation.as_ref(),
                output: part,
                driver_name: "ENVI",
            })?;
            let sml_projection = sarscape_wkt(runtime, &projection)?;
            // Stage metadata with the raster; never expose a final SML before its data is ready.
            write_sarscape_sml(part, width, height, &transform, nodata, &sml_projection)?;
        }
        write_raster_output(RasterOutputRequest {
            runtime,
            input,
            width,
            height,
            projection: &projection_c,
            transform: &mut transform,
            nodata,
            area_or_point: source_area_or_point.as_deref(),
            operation: operation.as_ref(),
            output: &ellipsoid_part,
            driver_name: "GTiff",
        })?;
    }

    // Validate the complete staged set before replacing any final output.
    if let Some(part) = sarscape_part.as_ref() {
        validate_sarscape_staged(part)?;
    }
    finalize_output(&ellipsoid_part, ellipsoid_output)?;
    if let (Some(part), Some(output)) = (sarscape_part.as_ref(), sarscape_output) {
        finalize_output(part, output)?;
        finalize_output(
            &sarscape_part_header_path(part)?,
            &sidecar_path(output, ".hdr")?,
        )?;
        // These are separate file promotions, not an atomic multi-file transaction.
        // Commit SML last so a newly written sidecar never advertises missing data/header.
        finalize_output(&sidecar_path(part, ".sml")?, &sidecar_path(output, ".sml")?)?;
    }
    let bytes_written = fs::metadata(ellipsoid_output)
        .map(|metadata| metadata.len())
        .unwrap_or(0)
        + sarscape_output
            .and_then(|path| fs::metadata(path).ok())
            .map(|metadata| metadata.len())
            .unwrap_or(0);
    Ok(json!({
        "ok": true,
        "executed": true,
        "outcome": "success",
        "source_vertical_datum": source_model,
        "target_vertical_datum": "WGS84_ELLIPSOID",
        "ellipsoid_path": ellipsoid_output,
        "sarscape_path": sarscape_output,
        "bytes_written": bytes_written,
    }))
}

const MAX_NATIVE_PREPARE_WIDTH: u32 = 4096;
const MAX_NATIVE_PREPARE_HEIGHT: u32 = 4096;
const MAX_NATIVE_PREPARE_PIXELS: u64 = 16 * 1024 * 1024;

/// Execute the bounded horizontal DEM preparation contract.  The source may
/// be projected or geographic, but its CRS must be present and the caller must
/// provide an explicit target CRS, target crop bounds, output dimensions, and
/// resampling method.  Sampling is performed through a product-owned PROJ
/// operation with network disabled and the result is written by the reviewed
/// GTiff driver.
#[allow(clippy::too_many_arguments)]
pub(crate) fn crop_reproject_resample_dem(
    runtime: &NativeGisRuntime,
    input: &Path,
    output: &Path,
    target_crs: &str,
    crop_bbox: [f64; 4],
    width: u32,
    height: u32,
    resampling: &str,
) -> Result<serde_json::Value, NativeGisError> {
    let metadata = fs::metadata(input)
        .map_err(|_| error(NativeGisErrorCode::DemInvalidRaster, "DEM 文件不可读"))?;
    if !metadata.is_file() {
        return Err(error(
            NativeGisErrorCode::DemInvalidRaster,
            "DEM 文件不可读",
        ));
    }
    if input == output {
        return Err(error(
            NativeGisErrorCode::DemInvalidRaster,
            "DEM 输出路径不能覆盖输入文件",
        ));
    }
    let target_crs = target_crs.trim();
    if target_crs.is_empty() || target_crs.len() > 16 * 1024 {
        return Err(error(
            NativeGisErrorCode::DemReprojectionFailed,
            "目标 CRS 不能为空且必须是有限长度",
        ));
    }
    if !crop_bbox.iter().all(|value| value.is_finite())
        || crop_bbox[0] >= crop_bbox[2]
        || crop_bbox[1] >= crop_bbox[3]
    {
        return Err(error(
            NativeGisErrorCode::DemReprojectionFailed,
            "DEM crop bbox 必须是有限的 west,south,east,north 且范围有效",
        ));
    }
    if width == 0
        || height == 0
        || width > MAX_NATIVE_PREPARE_WIDTH
        || height > MAX_NATIVE_PREPARE_HEIGHT
        || u64::from(width) * u64::from(height) > MAX_NATIVE_PREPARE_PIXELS
    {
        return Err(error(
            NativeGisErrorCode::DemReprojectionFailed,
            "DEM native preparation output exceeds the bounded raster limit",
        ));
    }
    let resampling = resampling.trim().to_ascii_lowercase();
    if !matches!(resampling.as_str(), "nearest" | "bilinear") {
        return Err(error(
            NativeGisErrorCode::DemReprojectionFailed,
            "native DEM resampling must be nearest or bilinear",
        ));
    }

    let input_c = c_path(
        input,
        NativeGisErrorCode::DemInvalidRaster,
        "DEM 路径编码无效",
    )?;
    let target_crs_c = CString::new(target_crs.as_bytes()).map_err(|_| {
        error(
            NativeGisErrorCode::DemReprojectionFailed,
            "目标 CRS 编码无效",
        )
    })?;
    let output_part = part_path(output)?;
    remove_stale_output(&output_part)?;
    if let Some(parent) = output_part.parent() {
        fs::create_dir_all(parent).map_err(|_| {
            error(
                NativeGisErrorCode::DemReprojectionFailed,
                "DEM crop/reproject 输出目录无法创建",
            )
        })?;
    }
    let proj_context = runtime.create_proj_context()?;
    let result = unsafe {
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
                ("GDAL_HTTP_MAX_RETRY", "0"),
                ("GDAL_DISABLE_READDIR_ON_OPEN", "EMPTY_DIR"),
                ("GDAL_DRIVER_PATH", ""),
            ],
            &proj_data,
            &gdal_data,
        )?;
        let allowed_driver_names = [
            CString::new("GTiff").expect("static GDAL driver"),
            CString::new("ENVI").expect("static GDAL driver"),
        ];
        let allowed_driver_ptrs = [
            allowed_driver_names[0].as_ptr(),
            allowed_driver_names[1].as_ptr(),
            std::ptr::null(),
        ];
        let open = *runtime.gdal_symbol::<GdalOpenEx>(b"GDALOpenEx\0")?;
        let source_dataset = open(
            input_c.as_ptr(),
            GDAL_OF_RASTER | GDAL_ACCESS_READ_ONLY,
            allowed_driver_ptrs.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
        );
        if source_dataset.is_null() {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                "DEM 不是产品允许的 GTiff/ENVI 栅格",
            ));
        }
        let close = *runtime.gdal_symbol::<GdalClose>(b"GDALClose\0")?;
        let source_dataset = GdalDatasetGuard::new(source_dataset, close);
        let get_width = *runtime.gdal_symbol::<GdalGetRasterXSize>(b"GDALGetRasterXSize\0")?;
        let get_height = *runtime.gdal_symbol::<GdalGetRasterYSize>(b"GDALGetRasterYSize\0")?;
        let get_count = *runtime.gdal_symbol::<GdalGetRasterCount>(b"GDALGetRasterCount\0")?;
        let source_width = get_width(source_dataset.raw());
        let source_height = get_height(source_dataset.raw());
        let source_count = get_count(source_dataset.raw());
        if source_width <= 0
            || source_height <= 0
            || source_count <= 0
            || u64::try_from(source_width).unwrap_or(0) * u64::try_from(source_height).unwrap_or(0)
                > MAX_NATIVE_PREPARE_PIXELS
        {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                "DEM source raster exceeds the bounded native preparation limit",
            ));
        }
        let get_projection =
            *runtime.gdal_symbol::<GdalGetProjectionRef>(b"GDALGetProjectionRef\0")?;
        let projection_ptr = get_projection(source_dataset.raw());
        if projection_ptr.is_null() || CStr::from_ptr(projection_ptr).to_bytes().is_empty() {
            return Err(error(
                NativeGisErrorCode::DemCrsMissing,
                "DEM 缺少水平 CRS，拒绝执行 crop/reproject/resample",
            ));
        }
        let source_projection = CStr::from_ptr(projection_ptr).to_string_lossy().to_string();
        let source_projection_c = CString::new(source_projection.as_bytes())
            .map_err(|_| error(NativeGisErrorCode::DemCrsMissing, "DEM CRS 编码无效"))?;
        let get_transform =
            *runtime.gdal_symbol::<GdalGetGeoTransform>(b"GDALGetGeoTransform\0")?;
        let mut source_transform = [0.0f64; 6];
        if get_transform(source_dataset.raw(), source_transform.as_mut_ptr()) != 0 {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                "DEM 地理变换参数不可读取",
            ));
        }
        let get_band = *runtime.gdal_symbol::<GdalGetRasterBand>(b"GDALGetRasterBand\0")?;
        let source_band = get_band(source_dataset.raw(), 1);
        if source_band.is_null() {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                "DEM 第一波段不可读取",
            ));
        }
        let get_nodata =
            *runtime.gdal_symbol::<GdalGetRasterNoDataValue>(b"GDALGetRasterNoDataValue\0")?;
        let mut has_nodata = 0;
        let nodata = get_nodata(source_band, &mut has_nodata);
        let nodata = (has_nodata != 0).then_some(nodata);
        let source_pixels = usize::try_from(source_width)
            .ok()
            .and_then(|width| {
                usize::try_from(source_height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .ok_or_else(|| {
                error(
                    NativeGisErrorCode::DemInvalidRaster,
                    "DEM source raster size cannot be represented",
                )
            })?;
        let mut source_values = vec![0.0f32; source_pixels];
        let raster_io = *runtime.gdal_symbol::<GdalRasterIo>(b"GDALRasterIO\0")?;
        if raster_io(
            source_band,
            GDAL_READ,
            0,
            0,
            source_width,
            source_height,
            source_values.as_mut_ptr().cast(),
            source_width,
            source_height,
            GDAL_FLOAT32,
            0,
            0,
        ) != 0
        {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                "DEM source pixels cannot be read",
            ));
        }

        let create_crs_to_crs =
            *runtime.proj_symbol::<ProjCreateCrsToCrs>(b"proj_create_crs_to_crs\0")?;
        let normalize = runtime
            .proj_symbol::<ProjNormalizeForVisualization>(b"proj_normalize_for_visualization\0")?;
        let destroy = *runtime.proj_symbol::<ProjDestroy>(b"proj_destroy\0")?;
        let transform_point = *runtime.proj_symbol::<ProjTrans>(b"proj_trans\0")?;
        // Transform output target coordinates back into source pixel space.
        let raw_operation = create_crs_to_crs(
            proj_context.raw(),
            target_crs_c.as_ptr(),
            source_projection_c.as_ptr(),
            std::ptr::null(),
        );
        if raw_operation.is_null() {
            return Err(error(
                NativeGisErrorCode::DemReprojectionFailed,
                "PROJ 无法建立目标 CRS 到 DEM source CRS 的转换",
            ));
        }
        let normalized_operation = normalize(proj_context.raw(), raw_operation);
        let operation = if normalized_operation.is_null() {
            raw_operation
        } else {
            destroy(raw_operation);
            normalized_operation
        };
        let operation = ProjOperationGuard::new(operation, destroy);
        let target_projection_c =
            target_crs_to_wkt(runtime, proj_context.raw(), target_crs_c.as_ptr())?;
        let output_width = i32::try_from(width).map_err(|_| {
            error(
                NativeGisErrorCode::DemReprojectionFailed,
                "DEM output width exceeds native integer range",
            )
        })?;
        let output_height = i32::try_from(height).map_err(|_| {
            error(
                NativeGisErrorCode::DemReprojectionFailed,
                "DEM output height exceeds native integer range",
            )
        })?;
        let mut output_transform = [
            crop_bbox[0],
            (crop_bbox[2] - crop_bbox[0]) / f64::from(width),
            0.0,
            crop_bbox[3],
            0.0,
            -(crop_bbox[3] - crop_bbox[1]) / f64::from(height),
        ];
        let driver_name = CString::new("GTiff").expect("static GDAL driver");
        let get_driver = *runtime.gdal_symbol::<GdalGetDriverByName>(b"GDALGetDriverByName\0")?;
        let driver = get_driver(driver_name.as_ptr());
        if driver.is_null() {
            return Err(error(
                NativeGisErrorCode::GisDriverUnavailable,
                "产品 GDAL runtime 缺少 GTiff 输出 driver",
            ));
        }
        let create = *runtime.gdal_symbol::<GdalCreate>(b"GDALCreate\0")?;
        let output_c = c_path(
            &output_part,
            NativeGisErrorCode::DemReprojectionFailed,
            "DEM 输出路径编码无效",
        )?;
        let output_dataset = create(
            driver,
            output_c.as_ptr(),
            output_width,
            output_height,
            1,
            GDAL_FLOAT32,
            std::ptr::null(),
        );
        if output_dataset.is_null() {
            return Err(error(
                NativeGisErrorCode::DemReprojectionFailed,
                "GDAL 无法创建 crop/reproject GTiff 输出",
            ));
        }
        let output_dataset = GdalDatasetGuard::new(output_dataset, close);
        let set_projection = *runtime.gdal_symbol::<GdalSetProjection>(b"GDALSetProjection\0")?;
        if set_projection(output_dataset.raw(), target_projection_c.as_ptr()) != 0 {
            return Err(error(
                NativeGisErrorCode::DemReprojectionFailed,
                "DEM target CRS 无法写入",
            ));
        }
        let set_transform =
            *runtime.gdal_symbol::<GdalSetGeoTransform>(b"GDALSetGeoTransform\0")?;
        if set_transform(output_dataset.raw(), output_transform.as_mut_ptr()) != 0 {
            return Err(error(
                NativeGisErrorCode::DemReprojectionFailed,
                "DEM target geotransform 无法写入",
            ));
        }
        let output_band = get_band(output_dataset.raw(), 1);
        if output_band.is_null() {
            return Err(error(
                NativeGisErrorCode::DemReprojectionFailed,
                "DEM crop/reproject 输出波段无法创建",
            ));
        }
        if let Some(nodata) = nodata {
            let set_nodata =
                *runtime.gdal_symbol::<GdalSetRasterNoDataValue>(b"GDALSetRasterNoDataValue\0")?;
            if set_nodata(output_band, nodata) != 0 {
                return Err(error(
                    NativeGisErrorCode::DemReprojectionFailed,
                    "DEM crop/reproject nodata 无法写入",
                ));
            }
        }
        let mut output_row = vec![missing_sample(nodata); width as usize];
        for row in 0..height as usize {
            for (column, output_value) in output_row.iter_mut().enumerate() {
                let target_x = crop_bbox[0]
                    + (column as f64 + 0.5) * output_transform[1]
                    + (row as f64 + 0.5) * output_transform[2];
                let target_y = crop_bbox[3]
                    + (column as f64 + 0.5) * output_transform[4]
                    + (row as f64 + 0.5) * output_transform[5];
                let transformed = transform_point(
                    operation.raw(),
                    PJ_FWD,
                    ProjCoord {
                        x: target_x,
                        y: target_y,
                        z: 0.0,
                        t: 0.0,
                    },
                );
                let Some((source_x, source_y)) =
                    world_to_pixel(&source_transform, transformed.x, transformed.y)
                else {
                    *output_value = missing_sample(nodata);
                    continue;
                };
                *output_value = sample_raster(
                    &source_values,
                    source_width as usize,
                    source_height as usize,
                    source_x,
                    source_y,
                    nodata,
                    &resampling,
                );
            }
            if raster_io(
                output_band,
                GDAL_WRITE,
                0,
                row as c_int,
                output_width,
                1,
                output_row.as_mut_ptr().cast(),
                output_width,
                1,
                GDAL_FLOAT32,
                0,
                0,
            ) != 0
            {
                return Err(error(
                    NativeGisErrorCode::DemReprojectionFailed,
                    "DEM crop/reproject 像元写入失败",
                ));
            }
        }
        let flush = *runtime.gdal_symbol::<GdalFlushCache>(b"GDALFlushCache\0")?;
        flush(output_dataset.raw());
        Ok(())
    };
    if result.is_err() {
        let _ = fs::remove_file(&output_part);
    }
    result?;
    finalize_output(&output_part, output)?;
    let output_bytes = fs::metadata(output)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    Ok(json!({
        "ok": true,
        "executed": true,
        "operation": "crop_reproject_resample",
        "source_path": input,
        "output_path": output,
        "target_crs": target_crs,
        "crop_bbox": crop_bbox,
        "width": width,
        "height": height,
        "resampling": resampling,
        "driver": "GTiff",
        "bytes_written": output_bytes,
    }))
}

/// Build a bounded local mosaic from already-downloaded Copernicus GeoTIFF
/// tiles, then crop it on the source grid into one logical request window.
/// Every tile must already share one aligned native pixel grid. We reject mixed
/// grids before BuildVRT so GDAL cannot select an averaged resolution and
/// silently resample source elevations. Translate receives no `outsize` or
/// resampling option.
pub(crate) fn mosaic_crop_dem_tiles(
    runtime: &NativeGisRuntime,
    inputs: &[PathBuf],
    output: &Path,
    crop_bbox: [f64; 4],
) -> Result<serde_json::Value, NativeGisError> {
    if inputs.is_empty() || inputs.len() > 512 {
        return Err(error(
            NativeGisErrorCode::DemInvalidRaster,
            "DEM tile mosaic must contain between 1 and 512 inputs",
        ));
    }
    if crop_bbox.iter().any(|value| !value.is_finite())
        || crop_bbox[0] < -180.0
        || crop_bbox[2] > 180.0
        || crop_bbox[1] < -90.0
        || crop_bbox[3] > 90.0
        || crop_bbox[0] >= crop_bbox[2]
        || crop_bbox[1] >= crop_bbox[3]
    {
        return Err(error(
            NativeGisErrorCode::DemReprojectionFailed,
            "DEM tile mosaic crop bbox is invalid",
        ));
    }
    for input in inputs {
        let metadata = fs::metadata(input).map_err(|_| {
            error(
                NativeGisErrorCode::DemInvalidRaster,
                "DEM mosaic tile is not readable",
            )
        })?;
        if !metadata.is_file() || input == output {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                "DEM mosaic tile is invalid or conflicts with the output path",
            ));
        }
    }

    let output_part = part_path(output)?;
    remove_stale_output(&output_part)?;
    if let Some(parent) = output_part.parent() {
        fs::create_dir_all(parent).map_err(|_| {
            error(
                NativeGisErrorCode::DemReprojectionFailed,
                "DEM tile mosaic output directory cannot be prepared",
            )
        })?;
    }
    let vrt_path = output_part.with_file_name(format!(
        ".{}.mosaic.vrt",
        output_part
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("dem")
    ));
    let _ = fs::remove_file(&vrt_path);

    let result = unsafe {
        let proj_data = c_path(
            &runtime.root().join(&runtime.manifest().proj_data_dir),
            NativeGisErrorCode::GisEngineUnavailable,
            "PROJ data directory encoding is invalid",
        )?;
        let gdal_data = c_path(
            &runtime.root().join(&runtime.manifest().gdal_data_dir),
            NativeGisErrorCode::GisEngineUnavailable,
            "GDAL data directory encoding is invalid",
        )?;
        runtime.configure_and_register_gdal(
            &[
                ("PROJ_NETWORK", "OFF"),
                ("CPL_CURL_VERBOSE", "NO"),
                ("GDAL_HTTP_MAX_RETRY", "0"),
                ("GDAL_DISABLE_READDIR_ON_OPEN", "EMPTY_DIR"),
                ("GDAL_DRIVER_PATH", ""),
            ],
            &proj_data,
            &gdal_data,
        )?;
        let allowed_driver_names = [CString::new("GTiff").expect("static GDAL driver")];
        let allowed_driver_ptrs = [allowed_driver_names[0].as_ptr(), std::ptr::null()];
        let open = *runtime.gdal_symbol::<GdalOpenEx>(b"GDALOpenEx\0")?;
        let close = *runtime.gdal_symbol::<GdalClose>(b"GDALClose\0")?;
        let get_count = *runtime.gdal_symbol::<GdalGetRasterCount>(b"GDALGetRasterCount\0")?;
        let get_projection =
            *runtime.gdal_symbol::<GdalGetProjectionRef>(b"GDALGetProjectionRef\0")?;
        let get_transform =
            *runtime.gdal_symbol::<GdalGetGeoTransform>(b"GDALGetGeoTransform\0")?;
        let get_band = *runtime.gdal_symbol::<GdalGetRasterBand>(b"GDALGetRasterBand\0")?;
        let get_type = *runtime.gdal_symbol::<GdalGetRasterDataType>(b"GDALGetRasterDataType\0")?;
        let get_nodata =
            *runtime.gdal_symbol::<GdalGetRasterNoDataValue>(b"GDALGetRasterNoDataValue\0")?;
        let get_metadata_item =
            *runtime.gdal_symbol::<GdalGetMetadataItem>(b"GDALGetMetadataItem\0")?;
        let area_or_point_key = CString::new("AREA_OR_POINT").expect("static GDAL metadata key");
        let close_enough = |left: f64, right: f64, scale: f64| {
            (left - right).abs() <= scale.abs().max(1.0) * 1.0e-9
        };
        let grid_aligned = |origin: f64, reference: f64, spacing: f64| {
            let offset = (origin - reference) / spacing;
            offset.is_finite() && close_enough(offset, offset.round(), 1.0)
        };
        let mut reference_transform: Option<[f64; 6]> = None;
        let mut reference_projection: Option<Vec<u8>> = None;
        let mut reference_data_type: Option<c_int> = None;
        let mut reference_nodata: Option<Option<f64>> = None;
        let mut reference_area_or_point: Option<Option<Vec<u8>>> = None;
        let mut source_datasets = Vec::with_capacity(inputs.len());
        for input in inputs {
            let input_c = c_path(
                input,
                NativeGisErrorCode::DemInvalidRaster,
                "DEM mosaic tile path encoding is invalid",
            )?;
            let dataset = open(
                input_c.as_ptr(),
                GDAL_OF_RASTER | GDAL_ACCESS_READ_ONLY,
                allowed_driver_ptrs.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
            );
            if dataset.is_null() {
                return Err(error(
                    NativeGisErrorCode::DemInvalidRaster,
                    "DEM mosaic input is not a product-admitted GeoTIFF",
                ));
            }
            let guard = GdalDatasetGuard::new(dataset, close);
            let projection = get_projection(guard.raw());
            let mut transform = [0.0f64; 6];
            let band = get_band(guard.raw(), 1);
            if get_count(guard.raw()) != 1
                || projection.is_null()
                || CStr::from_ptr(projection).to_bytes().is_empty()
                || !is_wgs84_geographic_crs(&CStr::from_ptr(projection).to_string_lossy())
                || get_transform(guard.raw(), transform.as_mut_ptr()) != 0
                || transform[1] <= 0.0
                || transform[5] >= 0.0
                || transform[2].abs() > 1.0e-12
                || transform[4].abs() > 1.0e-12
                || band.is_null()
            {
                return Err(error(
                    NativeGisErrorCode::DemInvalidRaster,
                    "DEM mosaic tiles must be single-band north-up WGS84 geographic rasters",
                ));
            }
            let projection_bytes = CStr::from_ptr(projection).to_bytes().to_vec();
            let data_type = get_type(band);
            let mut has_nodata = 0;
            let nodata_value = get_nodata(band, &mut has_nodata);
            let nodata = (has_nodata != 0).then_some(nodata_value);
            let area_or_point =
                get_metadata_item(guard.raw(), area_or_point_key.as_ptr(), std::ptr::null());
            let area_or_point = (!area_or_point.is_null())
                .then(|| CStr::from_ptr(area_or_point).to_bytes().to_vec());
            if let Some(reference) = reference_transform {
                let reference_nodata_value =
                    reference_nodata.expect("reference grid has nodata state");
                let nodata_matches = match (reference_nodata_value, nodata) {
                    (None, None) => true,
                    (Some(left), Some(right)) => left == right || (left.is_nan() && right.is_nan()),
                    _ => false,
                };
                if !close_enough(transform[1], reference[1], reference[1])
                    || !close_enough(transform[5], reference[5], reference[5])
                    || !grid_aligned(transform[0], reference[0], reference[1])
                    || !grid_aligned(transform[3], reference[3], reference[5])
                    || reference_projection.as_deref() != Some(projection_bytes.as_slice())
                    || reference_data_type != Some(data_type)
                    || !nodata_matches
                    || reference_area_or_point.as_ref() != Some(&area_or_point)
                {
                    return Err(error(
                        NativeGisErrorCode::DemInvalidRaster,
                        "DEM mosaic tiles use incompatible native grids or pixel semantics; explicit resampling is required",
                    ));
                }
            } else {
                reference_transform = Some(transform);
                reference_projection = Some(projection_bytes);
                reference_data_type = Some(data_type);
                reference_nodata = Some(nodata);
                reference_area_or_point = Some(area_or_point);
            }
            source_datasets.push(guard);
        }

        let build_options_new =
            *runtime.gdal_symbol::<GdalBuildVrtOptionsNew>(b"GDALBuildVRTOptionsNew\0")?;
        let build_options_free =
            *runtime.gdal_symbol::<GdalBuildVrtOptionsFree>(b"GDALBuildVRTOptionsFree\0")?;
        let build_vrt = *runtime.gdal_symbol::<GdalBuildVrt>(b"GDALBuildVRT\0")?;
        let mut build_args = [std::ptr::null_mut::<c_char>()];
        let build_options = build_options_new(build_args.as_mut_ptr(), std::ptr::null_mut());
        if build_options.is_null() {
            return Err(error(
                NativeGisErrorCode::DemReprojectionFailed,
                "GDAL could not prepare the bounded VRT mosaic options",
            ));
        }
        let build_options = GdalOptionsGuard::new(build_options, build_options_free);
        let vrt_c = c_path(
            &vrt_path,
            NativeGisErrorCode::DemReprojectionFailed,
            "DEM VRT path encoding is invalid",
        )?;
        let mut source_handles = source_datasets
            .iter()
            .map(GdalDatasetGuard::raw)
            .collect::<Vec<_>>();
        let mut usage_error = 0;
        let vrt_dataset = build_vrt(
            vrt_c.as_ptr(),
            c_int::try_from(source_handles.len()).map_err(|_| {
                error(
                    NativeGisErrorCode::DemReprojectionFailed,
                    "DEM tile count exceeds the native integer range",
                )
            })?,
            source_handles.as_mut_ptr(),
            std::ptr::null(),
            build_options.raw(),
            &mut usage_error,
        );
        if usage_error != 0 || vrt_dataset.is_null() {
            return Err(error(
                NativeGisErrorCode::DemReprojectionFailed,
                "GDAL could not build the bounded DEM tile VRT",
            ));
        }
        let vrt_dataset = GdalDatasetGuard::new(vrt_dataset, close);
        let mut vrt_transform = [0.0f64; 6];
        if get_transform(vrt_dataset.raw(), vrt_transform.as_mut_ptr()) != 0
            || vrt_transform[1] <= 0.0
            || vrt_transform[5] >= 0.0
            || vrt_transform[2].abs() > 1.0e-12
            || vrt_transform[4].abs() > 1.0e-12
        {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                "DEM tile mosaic does not expose a north-up source pixel grid",
            ));
        }
        let source_transform = reference_transform.expect("at least one source grid was validated");
        if !close_enough(vrt_transform[1], source_transform[1], source_transform[1])
            || !close_enough(vrt_transform[5], source_transform[5], source_transform[5])
            || !grid_aligned(vrt_transform[0], source_transform[0], source_transform[1])
            || !grid_aligned(vrt_transform[3], source_transform[3], source_transform[5])
        {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                "GDAL VRT changed the validated source DEM pixel grid",
            ));
        }
        let expected_width = ((crop_bbox[2] - crop_bbox[0]) / vrt_transform[1]).ceil() as u64;
        let expected_height = ((crop_bbox[3] - crop_bbox[1]) / -vrt_transform[5]).ceil() as u64;
        if expected_width == 0
            || expected_height == 0
            || expected_width > MAX_NATIVE_MOSAIC_WIDTH
            || expected_height > MAX_NATIVE_MOSAIC_HEIGHT
            || expected_width.saturating_mul(expected_height) > MAX_NATIVE_MOSAIC_PIXELS
        {
            return Err(error(
                NativeGisErrorCode::DemReprojectionFailed,
                "source-aligned DEM tile mosaic exceeds the bounded raster limit",
            ));
        }

        let translate_options_new =
            *runtime.gdal_symbol::<GdalTranslateOptionsNew>(b"GDALTranslateOptionsNew\0")?;
        let translate_options_free =
            *runtime.gdal_symbol::<GdalTranslateOptionsFree>(b"GDALTranslateOptionsFree\0")?;
        let translate = *runtime.gdal_symbol::<GdalTranslate>(b"GDALTranslate\0")?;
        let translate_args = [
            "-of".to_owned(),
            "GTiff".to_owned(),
            "-projwin".to_owned(),
            crop_bbox[0].to_string(),
            crop_bbox[3].to_string(),
            crop_bbox[2].to_string(),
            crop_bbox[1].to_string(),
        ]
        .into_iter()
        .map(|value| {
            CString::new(value).map_err(|_| {
                error(
                    NativeGisErrorCode::DemReprojectionFailed,
                    "DEM mosaic translate option encoding is invalid",
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
        let mut translate_arg_ptrs = translate_args
            .iter()
            .map(|value| value.as_ptr().cast_mut())
            .collect::<Vec<_>>();
        translate_arg_ptrs.push(std::ptr::null_mut());
        let translate_options =
            translate_options_new(translate_arg_ptrs.as_mut_ptr(), std::ptr::null_mut());
        if translate_options.is_null() {
            return Err(error(
                NativeGisErrorCode::DemReprojectionFailed,
                "GDAL could not prepare DEM mosaic translation",
            ));
        }
        let translate_options = GdalOptionsGuard::new(translate_options, translate_options_free);
        let output_c = c_path(
            &output_part,
            NativeGisErrorCode::DemReprojectionFailed,
            "DEM mosaic output path encoding is invalid",
        )?;
        usage_error = 0;
        let output_dataset = translate(
            output_c.as_ptr(),
            vrt_dataset.raw(),
            translate_options.raw(),
            &mut usage_error,
        );
        if usage_error != 0 || output_dataset.is_null() {
            return Err(error(
                NativeGisErrorCode::DemReprojectionFailed,
                "GDAL could not crop the DEM tile mosaic",
            ));
        }
        let output_dataset = GdalDatasetGuard::new(output_dataset, close);
        let flush = *runtime.gdal_symbol::<GdalFlushCache>(b"GDALFlushCache\0")?;
        flush(output_dataset.raw());
        Ok::<_, NativeGisError>(())
    };
    let _ = fs::remove_file(&vrt_path);
    if result.is_err() {
        let _ = fs::remove_file(&output_part);
    }
    result?;
    finalize_output(&output_part, output)?;
    let inspection = inspect_dem(runtime, output).and_then(|inspection| {
        let metadata = inspection
            .get("metadata")
            .and_then(serde_json::Value::as_object);
        let actual_width = metadata
            .and_then(|value| value.get("width"))
            .and_then(serde_json::Value::as_u64);
        let actual_height = metadata
            .and_then(|value| value.get("height"))
            .and_then(serde_json::Value::as_u64);
        let count = metadata
            .and_then(|value| value.get("count"))
            .and_then(serde_json::Value::as_u64);
        if actual_width.is_none_or(|value| value == 0 || value > MAX_NATIVE_MOSAIC_WIDTH)
            || actual_height.is_none_or(|value| value == 0 || value > MAX_NATIVE_MOSAIC_HEIGHT)
            || actual_width
                .zip(actual_height)
                .is_none_or(|(width, height)| {
                    width.saturating_mul(height) > MAX_NATIVE_MOSAIC_PIXELS
                })
            || count != Some(1)
        {
            return Err(error(
                NativeGisErrorCode::DemReprojectionFailed,
                "promoted DEM mosaic failed the final raster identity check",
            ));
        }
        Ok(inspection)
    });
    let inspection = match inspection {
        Ok(value) => value,
        Err(error) => {
            let _ = fs::remove_file(output);
            return Err(error);
        }
    };
    Ok(json!({
        "ok": true,
        "executed": true,
        "operation": "mosaic_crop_dem_tiles",
        "tile_count": inputs.len(),
        "output_path": output,
        "crop_bbox": crop_bbox,
        "width": inspection["metadata"]["width"],
        "height": inspection["metadata"]["height"],
        "resampling": serde_json::Value::Null,
        "bytes_written": fs::metadata(output).map(|metadata| metadata.len()).unwrap_or(0),
        "inspection": inspection,
    }))
}

/// Reproject, mosaic, and crop one or more projected (e.g. UTM) Float32 GeoTIFF
/// tiles (such as GEM-15 UTM tiles, which may carry 1 band for DTM or 5 bands
/// where band 1 is DSM) into a bounded single-band north-up `EPSG:4326` GeoTIFF
/// with explicit `-dstnodata -9999` and `bilinear` resampling at 0.5 arc-sec.
/// Leaves `mosaic_crop_dem_tiles` untouched for `EPSG:4326` source-aligned routes.
pub(crate) fn warp_mosaic_crop_utm_dem_tiles(
    runtime: &NativeGisRuntime,
    inputs: &[PathBuf],
    output: &Path,
    crop_bbox: [f64; 4],
) -> Result<serde_json::Value, NativeGisError> {
    const GEM15_SAMPLES_PER_DEGREE: f64 = 7_200.0;
    if inputs.is_empty() || inputs.len() > 512 {
        return Err(error(
            NativeGisErrorCode::DemInvalidRaster,
            "UTM DEM warp mosaic must contain between 1 and 512 inputs",
        ));
    }
    if crop_bbox.iter().any(|value| !value.is_finite())
        || crop_bbox[0] < -180.0
        || crop_bbox[2] > 180.0
        || crop_bbox[1] < -90.0
        || crop_bbox[3] > 90.0
        || crop_bbox[0] >= crop_bbox[2]
        || crop_bbox[1] >= crop_bbox[3]
    {
        return Err(error(
            NativeGisErrorCode::DemReprojectionFailed,
            "UTM DEM warp mosaic crop bbox is invalid",
        ));
    }
    for input in inputs {
        let metadata = fs::metadata(input).map_err(|_| {
            error(
                NativeGisErrorCode::DemInvalidRaster,
                "UTM DEM tile is not readable",
            )
        })?;
        if !metadata.is_file() || input == output {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                "UTM DEM tile is invalid or conflicts with the output path",
            ));
        }
    }

    let snap_axis = |min_deg: f64, max_deg: f64, limit_min: f64, limit_max: f64| -> (f64, f64) {
        let w_eps = ((min_deg * GEM15_SAMPLES_PER_DEGREE + 1.0e-9).floor()
            / GEM15_SAMPLES_PER_DEGREE)
            .max(limit_min);
        let e_eps = ((max_deg * GEM15_SAMPLES_PER_DEGREE - 1.0e-9).ceil()
            / GEM15_SAMPLES_PER_DEGREE)
            .min(limit_max);
        if e_eps > w_eps {
            (w_eps, e_eps)
        } else {
            let w_raw = ((min_deg * GEM15_SAMPLES_PER_DEGREE).floor() / GEM15_SAMPLES_PER_DEGREE)
                .max(limit_min);
            let e_raw = ((max_deg * GEM15_SAMPLES_PER_DEGREE).ceil() / GEM15_SAMPLES_PER_DEGREE)
                .min(limit_max);
            (w_raw, e_raw)
        }
    };
    let (snapped_west, snapped_east) = snap_axis(crop_bbox[0], crop_bbox[2], -180.0, 180.0);
    let (snapped_south, snapped_north) = snap_axis(crop_bbox[1], crop_bbox[3], -90.0, 90.0);
    let target_width = ((snapped_east - snapped_west) * GEM15_SAMPLES_PER_DEGREE)
        .round()
        .max(0.0) as u64;
    let target_height = ((snapped_north - snapped_south) * GEM15_SAMPLES_PER_DEGREE)
        .round()
        .max(0.0) as u64;
    if snapped_west >= snapped_east
        || snapped_south >= snapped_north
        || target_width == 0
        || target_height == 0
        || target_width > MAX_NATIVE_MOSAIC_WIDTH
        || target_height > MAX_NATIVE_MOSAIC_HEIGHT
        || target_width.saturating_mul(target_height) > MAX_NATIVE_MOSAIC_PIXELS
    {
        return Err(error(
            NativeGisErrorCode::DemReprojectionFailed,
            "UTM DEM warp mosaic exceeds the bounded raster limit",
        ));
    }

    let output_part = part_path(output)?;
    remove_stale_output(&output_part)?;
    if let Some(parent) = output_part.parent() {
        fs::create_dir_all(parent).map_err(|_| {
            error(
                NativeGisErrorCode::DemReprojectionFailed,
                "UTM DEM warp mosaic output directory cannot be prepared",
            )
        })?;
    }
    let part_prefix = output_part
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("dem");
    let mut temp_vrt_paths: Vec<PathBuf> = Vec::new();

    let result = unsafe {
        let proj_data = c_path(
            &runtime.root().join(&runtime.manifest().proj_data_dir),
            NativeGisErrorCode::GisEngineUnavailable,
            "PROJ data directory encoding is invalid",
        )?;
        let gdal_data = c_path(
            &runtime.root().join(&runtime.manifest().gdal_data_dir),
            NativeGisErrorCode::GisEngineUnavailable,
            "GDAL data directory encoding is invalid",
        )?;
        runtime.configure_and_register_gdal(
            &[
                ("PROJ_NETWORK", "OFF"),
                ("CPL_CURL_VERBOSE", "NO"),
                ("GDAL_HTTP_MAX_RETRY", "0"),
                ("GDAL_DISABLE_READDIR_ON_OPEN", "EMPTY_DIR"),
                ("GDAL_DRIVER_PATH", ""),
            ],
            &proj_data,
            &gdal_data,
        )?;
        let allowed_driver_names = [CString::new("GTiff").expect("static GDAL driver")];
        let allowed_driver_ptrs = [allowed_driver_names[0].as_ptr(), std::ptr::null()];
        let open = *runtime.gdal_symbol::<GdalOpenEx>(b"GDALOpenEx\0")?;
        let close = *runtime.gdal_symbol::<GdalClose>(b"GDALClose\0")?;
        let get_count = *runtime.gdal_symbol::<GdalGetRasterCount>(b"GDALGetRasterCount\0")?;
        let get_projection =
            *runtime.gdal_symbol::<GdalGetProjectionRef>(b"GDALGetProjectionRef\0")?;
        let get_transform =
            *runtime.gdal_symbol::<GdalGetGeoTransform>(b"GDALGetGeoTransform\0")?;
        let get_band = *runtime.gdal_symbol::<GdalGetRasterBand>(b"GDALGetRasterBand\0")?;
        let get_type = *runtime.gdal_symbol::<GdalGetRasterDataType>(b"GDALGetRasterDataType\0")?;
        let get_nodata =
            *runtime.gdal_symbol::<GdalGetRasterNoDataValue>(b"GDALGetRasterNoDataValue\0")?;

        let mut source_datasets: Vec<GdalDatasetGuard> = Vec::with_capacity(inputs.len());
        let mut vrt_datasets: Vec<GdalDatasetGuard> = Vec::new();
        let mut warp_source_handles: Vec<Dataset> = Vec::with_capacity(inputs.len());
        let mut uniform_finite_nodata: Option<f64> = None;
        let mut use_nan_srcnodata = false;

        let build_options_new =
            *runtime.gdal_symbol::<GdalBuildVrtOptionsNew>(b"GDALBuildVRTOptionsNew\0")?;
        let build_options_free =
            *runtime.gdal_symbol::<GdalBuildVrtOptionsFree>(b"GDALBuildVRTOptionsFree\0")?;
        let build_vrt = *runtime.gdal_symbol::<GdalBuildVrt>(b"GDALBuildVRT\0")?;

        for (index, input) in inputs.iter().enumerate() {
            let input_c = c_path(
                input,
                NativeGisErrorCode::DemInvalidRaster,
                "UTM DEM tile path encoding is invalid",
            )?;
            let dataset = open(
                input_c.as_ptr(),
                GDAL_OF_RASTER | GDAL_ACCESS_READ_ONLY,
                allowed_driver_ptrs.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
            );
            if dataset.is_null() {
                return Err(error(
                    NativeGisErrorCode::DemInvalidRaster,
                    "UTM DEM input is not a product-admitted GeoTIFF",
                ));
            }
            let guard = GdalDatasetGuard::new(dataset, close);
            let projection = get_projection(guard.raw());
            let mut transform = [0.0f64; 6];
            let count = get_count(guard.raw());
            let band = get_band(guard.raw(), 1);
            let det = if get_transform(guard.raw(), transform.as_mut_ptr()) == 0 {
                transform[1] * transform[5] - transform[2] * transform[4]
            } else {
                0.0
            };
            if count < 1
                || projection.is_null()
                || CStr::from_ptr(projection).to_bytes().is_empty()
                || det.abs() <= 1.0e-12
                || band.is_null()
                || get_type(band) != GDAL_FLOAT32
            {
                return Err(error(
                    NativeGisErrorCode::DemInvalidRaster,
                    "UTM DEM tiles must be valid Float32 GeoTIFF rasters with CRS metadata",
                ));
            }
            let mut has_nodata = 0;
            let nodata_val = get_nodata(band, &mut has_nodata);
            if has_nodata == 0 || !nodata_val.is_finite() {
                use_nan_srcnodata = true;
            } else if let Some(prev) = uniform_finite_nodata {
                if (prev - nodata_val).abs() > 1.0e-9 {
                    use_nan_srcnodata = true;
                }
            } else {
                uniform_finite_nodata = Some(nodata_val);
            }

            if count > 1 {
                let tile_vrt_path =
                    output_part.with_file_name(format!(".{part_prefix}.band1-{index}.vrt"));
                let _ = fs::remove_file(&tile_vrt_path);
                temp_vrt_paths.push(tile_vrt_path.clone());
                let tile_vrt_c = c_path(
                    &tile_vrt_path,
                    NativeGisErrorCode::DemReprojectionFailed,
                    "UTM DEM band-select VRT path encoding is invalid",
                )?;
                let b_flag = CString::new("-b").expect("static GDAL flag");
                let b_one = CString::new("1").expect("static band index");
                let mut build_args = [
                    b_flag.as_ptr().cast_mut(),
                    b_one.as_ptr().cast_mut(),
                    std::ptr::null_mut::<c_char>(),
                ];
                let build_options =
                    build_options_new(build_args.as_mut_ptr(), std::ptr::null_mut());
                if build_options.is_null() {
                    return Err(error(
                        NativeGisErrorCode::DemReprojectionFailed,
                        "GDAL could not prepare UTM DEM band-select VRT options",
                    ));
                }
                let build_options = GdalOptionsGuard::new(build_options, build_options_free);
                let mut raw_handle = guard.raw();
                let mut usage_error = 0;
                let vrt_dataset = build_vrt(
                    tile_vrt_c.as_ptr(),
                    1,
                    &mut raw_handle,
                    std::ptr::null(),
                    build_options.raw(),
                    &mut usage_error,
                );
                if usage_error != 0 || vrt_dataset.is_null() {
                    return Err(error(
                        NativeGisErrorCode::DemReprojectionFailed,
                        "GDAL could not build single-band VRT for multi-band UTM DEM tile",
                    ));
                }
                let vrt_guard = GdalDatasetGuard::new(vrt_dataset, close);
                warp_source_handles.push(vrt_guard.raw());
                vrt_datasets.push(vrt_guard);
            } else {
                warp_source_handles.push(guard.raw());
            }
            source_datasets.push(guard);
        }

        let src_nodata_arg = if use_nan_srcnodata {
            "nan".to_owned()
        } else {
            uniform_finite_nodata
                .map(|val| val.to_string())
                .unwrap_or_else(|| "nan".to_owned())
        };

        let warp_options_new =
            *runtime.gdal_symbol::<GdalWarpAppOptionsNew>(b"GDALWarpAppOptionsNew\0")?;
        let warp_options_free =
            *runtime.gdal_symbol::<GdalWarpAppOptionsFree>(b"GDALWarpAppOptionsFree\0")?;
        let warp = *runtime.gdal_symbol::<GdalWarp>(b"GDALWarp\0")?;
        let warp_args = [
            "-of".to_owned(),
            "GTiff".to_owned(),
            "-co".to_owned(),
            "COMPRESS=LZW".to_owned(),
            "-co".to_owned(),
            "TILED=YES".to_owned(),
            "-t_srs".to_owned(),
            "EPSG:4326".to_owned(),
            "-te".to_owned(),
            snapped_west.to_string(),
            snapped_south.to_string(),
            snapped_east.to_string(),
            snapped_north.to_string(),
            "-ts".to_owned(),
            target_width.to_string(),
            target_height.to_string(),
            "-ot".to_owned(),
            "Float32".to_owned(),
            "-r".to_owned(),
            "bilinear".to_owned(),
            "-srcnodata".to_owned(),
            src_nodata_arg,
            "-dstnodata".to_owned(),
            "-9999".to_owned(),
            "-wo".to_owned(),
            "INIT_DEST=NO_DATA".to_owned(),
            "-overwrite".to_owned(),
        ]
        .into_iter()
        .map(|value| {
            CString::new(value).map_err(|_| {
                error(
                    NativeGisErrorCode::DemReprojectionFailed,
                    "UTM DEM warp option encoding is invalid",
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
        let mut warp_arg_ptrs = warp_args
            .iter()
            .map(|value| value.as_ptr().cast_mut())
            .collect::<Vec<_>>();
        warp_arg_ptrs.push(std::ptr::null_mut());
        let warp_options = warp_options_new(warp_arg_ptrs.as_mut_ptr(), std::ptr::null_mut());
        if warp_options.is_null() {
            return Err(error(
                NativeGisErrorCode::DemReprojectionFailed,
                "GDAL could not prepare UTM DEM warp options",
            ));
        }
        let warp_options = GdalOptionsGuard::new(warp_options, warp_options_free);
        let output_c = c_path(
            &output_part,
            NativeGisErrorCode::DemReprojectionFailed,
            "UTM DEM warp output path encoding is invalid",
        )?;
        let n_sources = c_int::try_from(warp_source_handles.len()).map_err(|_| {
            error(
                NativeGisErrorCode::DemReprojectionFailed,
                "UTM DEM tile count exceeds native integer range",
            )
        })?;
        let mut usage_error = 0;
        let output_dataset = warp(
            output_c.as_ptr(),
            std::ptr::null_mut(),
            n_sources,
            warp_source_handles.as_mut_ptr(),
            warp_options.raw(),
            &mut usage_error,
        );
        if usage_error != 0 || output_dataset.is_null() {
            return Err(error(
                NativeGisErrorCode::DemReprojectionFailed,
                "GDAL could not warp UTM DEM tiles to EPSG:4326",
            ));
        }
        let output_dataset = GdalDatasetGuard::new(output_dataset, close);
        let flush = *runtime.gdal_symbol::<GdalFlushCache>(b"GDALFlushCache\0")?;
        flush(output_dataset.raw());
        drop(output_dataset);
        drop(vrt_datasets);
        drop(source_datasets);
        Ok::<_, NativeGisError>(())
    };

    for vrt_path in &temp_vrt_paths {
        let _ = fs::remove_file(vrt_path);
    }
    if result.is_err() {
        let _ = fs::remove_file(&output_part);
    }
    result?;
    finalize_output(&output_part, output)?;
    let inspection = inspect_dem(runtime, output).and_then(|inspection| {
        let metadata = inspection
            .get("metadata")
            .and_then(serde_json::Value::as_object);
        let actual_width = metadata
            .and_then(|value| value.get("width"))
            .and_then(serde_json::Value::as_u64);
        let actual_height = metadata
            .and_then(|value| value.get("height"))
            .and_then(serde_json::Value::as_u64);
        let count = metadata
            .and_then(|value| value.get("count"))
            .and_then(serde_json::Value::as_u64);
        let crs = metadata
            .and_then(|value| value.get("crs"))
            .and_then(serde_json::Value::as_str);
        if actual_width != Some(target_width)
            || actual_height != Some(target_height)
            || count != Some(1)
            || crs.is_none_or(|value| !is_wgs84_geographic_crs(value))
        {
            return Err(error(
                NativeGisErrorCode::DemReprojectionFailed,
                "warped UTM DEM failed the final EPSG:4326 raster identity check",
            ));
        }
        Ok(inspection)
    });
    let inspection = match inspection {
        Ok(value) => value,
        Err(error) => {
            let _ = fs::remove_file(output);
            return Err(error);
        }
    };
    Ok(json!({
        "ok": true,
        "executed": true,
        "operation": "warp_mosaic_crop_utm_dem_tiles",
        "tile_count": inputs.len(),
        "output_path": output,
        "crop_bbox": [snapped_west, snapped_south, snapped_east, snapped_north],
        "requested_crop_bbox": crop_bbox,
        "width": inspection["metadata"]["width"],
        "height": inspection["metadata"]["height"],
        "resampling": "bilinear",
        "bytes_written": fs::metadata(output).map(|metadata| metadata.len()).unwrap_or(0),
        "inspection": inspection,
    }))
}

/// Join the two already-cropped EPSG:4326 DEM windows that meet at the
/// antimeridian into one local, unwrapped raster. The east window pixels are
/// copied immediately after the west window; its -180-based geotransform is
/// deliberately represented as +180 in the output grid. Inputs must share the
/// same north-up native pixel spacing, row grid and vertical extent. Mixed
/// native grids fail closed rather than resampling either side at the seam.
pub(crate) fn merge_antimeridian_dem(
    runtime: &NativeGisRuntime,
    west_input: &Path,
    east_input: &Path,
    output: &Path,
) -> Result<serde_json::Value, NativeGisError> {
    for input in [west_input, east_input] {
        let metadata = fs::metadata(input)
            .map_err(|_| error(NativeGisErrorCode::DemInvalidRaster, "DEM 窗口文件不可读"))?;
        if !metadata.is_file() || input == output {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                "DEM 窗口文件不可读或与输出路径冲突",
            ));
        }
    }
    if west_input == east_input {
        return Err(error(
            NativeGisErrorCode::DemInvalidRaster,
            "DEM 日期变更线窗口必须是两个不同文件",
        ));
    }

    let output_part = part_path(output)?;
    remove_stale_output(&output_part)?;
    if let Some(parent) = output_part.parent() {
        fs::create_dir_all(parent).map_err(|_| {
            error(
                NativeGisErrorCode::DemReprojectionFailed,
                "DEM 日期变更线合并输出目录无法创建",
            )
        })?;
    }
    let result = unsafe {
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
                ("GDAL_HTTP_MAX_RETRY", "0"),
                ("GDAL_DISABLE_READDIR_ON_OPEN", "EMPTY_DIR"),
                ("GDAL_DRIVER_PATH", ""),
            ],
            &proj_data,
            &gdal_data,
        )?;
        let allowed_driver_names = [CString::new("GTiff").expect("static GDAL driver")];
        let allowed_driver_ptrs = [allowed_driver_names[0].as_ptr(), std::ptr::null()];
        let open = *runtime.gdal_symbol::<GdalOpenEx>(b"GDALOpenEx\0")?;
        let close = *runtime.gdal_symbol::<GdalClose>(b"GDALClose\0")?;
        let west_c = c_path(
            west_input,
            NativeGisErrorCode::DemInvalidRaster,
            "西侧 DEM 窗口路径编码无效",
        )?;
        let east_c = c_path(
            east_input,
            NativeGisErrorCode::DemInvalidRaster,
            "东侧 DEM 窗口路径编码无效",
        )?;
        let west_dataset = open(
            west_c.as_ptr(),
            GDAL_OF_RASTER | GDAL_ACCESS_READ_ONLY,
            allowed_driver_ptrs.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
        );
        let east_dataset = open(
            east_c.as_ptr(),
            GDAL_OF_RASTER | GDAL_ACCESS_READ_ONLY,
            allowed_driver_ptrs.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
        );
        if west_dataset.is_null() || east_dataset.is_null() {
            if !west_dataset.is_null() {
                close(west_dataset);
            }
            if !east_dataset.is_null() {
                close(east_dataset);
            }
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                "日期变更线 DEM 窗口不是产品允许的 GeoTIFF",
            ));
        }
        let west_dataset = GdalDatasetGuard::new(west_dataset, close);
        let east_dataset = GdalDatasetGuard::new(east_dataset, close);
        let get_width = *runtime.gdal_symbol::<GdalGetRasterXSize>(b"GDALGetRasterXSize\0")?;
        let get_height = *runtime.gdal_symbol::<GdalGetRasterYSize>(b"GDALGetRasterYSize\0")?;
        let west_width = get_width(west_dataset.raw());
        let east_width = get_width(east_dataset.raw());
        let west_height = get_height(west_dataset.raw());
        let east_height = get_height(east_dataset.raw());
        if west_width <= 0
            || east_width <= 0
            || west_height <= 0
            || west_height != east_height
            || u64::try_from(west_height).unwrap_or(u64::MAX) > MAX_ANTIMERIDIAN_MERGE_HEIGHT
        {
            return Err(error(
                NativeGisErrorCode::DemReprojectionFailed,
                "日期变更线 DEM 窗口尺寸不一致或超过产品限制",
            ));
        }
        let get_projection =
            *runtime.gdal_symbol::<GdalGetProjectionRef>(b"GDALGetProjectionRef\0")?;
        let west_projection = get_projection(west_dataset.raw());
        let east_projection = get_projection(east_dataset.raw());
        if west_projection.is_null()
            || east_projection.is_null()
            || CStr::from_ptr(west_projection).to_bytes().is_empty()
            || CStr::from_ptr(west_projection).to_bytes()
                != CStr::from_ptr(east_projection).to_bytes()
        {
            return Err(error(
                NativeGisErrorCode::DemCrsMissing,
                "日期变更线 DEM 窗口 CRS 缺失或不一致",
            ));
        }
        let projection =
            CString::new(CStr::from_ptr(west_projection).to_bytes()).map_err(|_| {
                error(
                    NativeGisErrorCode::DemCrsMissing,
                    "日期变更线 DEM CRS 编码无效",
                )
            })?;
        let get_transform =
            *runtime.gdal_symbol::<GdalGetGeoTransform>(b"GDALGetGeoTransform\0")?;
        let mut west_transform = [0.0f64; 6];
        let mut east_transform = [0.0f64; 6];
        if get_transform(west_dataset.raw(), west_transform.as_mut_ptr()) != 0
            || get_transform(east_dataset.raw(), east_transform.as_mut_ptr()) != 0
        {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                "日期变更线 DEM 窗口地理变换不可读",
            ));
        }
        let close_enough = |left: f64, right: f64, scale: f64| {
            (left - right).abs() <= scale.abs().max(1.0) * 1.0e-7
        };
        let west_east = west_transform[0] + west_transform[1] * f64::from(west_width);
        let east_south = east_transform[3] + east_transform[5] * f64::from(east_height);
        let west_south = west_transform[3] + west_transform[5] * f64::from(west_height);
        if west_transform[1] <= 0.0
            || east_transform[1] <= 0.0
            || west_transform[5] >= 0.0
            || east_transform[5] >= 0.0
            || !close_enough(west_transform[2], 0.0, 1.0)
            || !close_enough(west_transform[4], 0.0, 1.0)
            || !close_enough(east_transform[2], 0.0, 1.0)
            || !close_enough(east_transform[4], 0.0, 1.0)
            || !close_enough(west_transform[1], east_transform[1], west_transform[1])
            || !close_enough(west_transform[5], east_transform[5], west_transform[5])
            || !close_enough(west_transform[3], east_transform[3], west_transform[5])
            || !close_enough(west_south, east_south, west_transform[5])
            || !close_enough(west_east, 180.0, west_transform[1])
            || !close_enough(east_transform[0], -180.0, east_transform[1])
        {
            return Err(error(
                NativeGisErrorCode::DemReprojectionFailed,
                "日期变更线 DEM 窗口未在同一北向像元网格的 ±180° 接缝处对齐",
            ));
        }
        let total_width = i64::from(west_width) + i64::from(east_width);
        if total_width <= 0
            || u64::try_from(total_width).unwrap_or(u64::MAX) > MAX_ANTIMERIDIAN_MERGE_WIDTH
            || u64::try_from(total_width)
                .unwrap_or(u64::MAX)
                .saturating_mul(u64::try_from(west_height).unwrap_or(u64::MAX))
                > MAX_ANTIMERIDIAN_MERGE_PIXELS
        {
            return Err(error(
                NativeGisErrorCode::DemReprojectionFailed,
                "日期变更线 DEM 合并网格超过产品限制",
            ));
        }
        let total_width = c_int::try_from(total_width).map_err(|_| {
            error(
                NativeGisErrorCode::DemReprojectionFailed,
                "日期变更线 DEM 输出宽度无法表示",
            )
        })?;

        let get_count = *runtime.gdal_symbol::<GdalGetRasterCount>(b"GDALGetRasterCount\0")?;
        let get_band = *runtime.gdal_symbol::<GdalGetRasterBand>(b"GDALGetRasterBand\0")?;
        let west_band = get_band(west_dataset.raw(), 1);
        let east_band = get_band(east_dataset.raw(), 1);
        if get_count(west_dataset.raw()) != 1
            || get_count(east_dataset.raw()) != 1
            || west_band.is_null()
            || east_band.is_null()
        {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                "日期变更线 DEM 窗口必须是可读的单波段栅格",
            ));
        }
        let get_type = *runtime.gdal_symbol::<GdalGetRasterDataType>(b"GDALGetRasterDataType\0")?;
        let source_data_type = get_type(west_band);
        if source_data_type != get_type(east_band) || source_data_type != GDAL_FLOAT32 {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                "日期变更线 DEM 窗口必须保持一致的 Float32 原生像元类型",
            ));
        }
        let get_nodata =
            *runtime.gdal_symbol::<GdalGetRasterNoDataValue>(b"GDALGetRasterNoDataValue\0")?;
        let mut west_has_nodata = 0;
        let mut east_has_nodata = 0;
        let west_nodata = get_nodata(west_band, &mut west_has_nodata);
        let east_nodata = get_nodata(east_band, &mut east_has_nodata);
        if west_has_nodata != east_has_nodata
            || (west_has_nodata != 0
                && !(west_nodata == east_nodata || (west_nodata.is_nan() && east_nodata.is_nan())))
        {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                "日期变更线 DEM 窗口 nodata 定义不一致",
            ));
        }

        let driver_name = CString::new("GTiff").expect("static GDAL driver");
        let get_driver = *runtime.gdal_symbol::<GdalGetDriverByName>(b"GDALGetDriverByName\0")?;
        let driver = get_driver(driver_name.as_ptr());
        if driver.is_null() {
            return Err(error(
                NativeGisErrorCode::GisDriverUnavailable,
                "产品 GDAL runtime 缺少 GTiff 输出 driver",
            ));
        }
        let output_c = c_path(
            &output_part,
            NativeGisErrorCode::DemReprojectionFailed,
            "日期变更线 DEM 输出路径编码无效",
        )?;
        let create = *runtime.gdal_symbol::<GdalCreate>(b"GDALCreate\0")?;
        let output_dataset = create(
            driver,
            output_c.as_ptr(),
            total_width,
            west_height,
            1,
            source_data_type,
            std::ptr::null(),
        );
        if output_dataset.is_null() {
            return Err(error(
                NativeGisErrorCode::DemReprojectionFailed,
                "GDAL 无法创建日期变更线 DEM 合并输出",
            ));
        }
        let output_dataset = GdalDatasetGuard::new(output_dataset, close);
        let set_projection = *runtime.gdal_symbol::<GdalSetProjection>(b"GDALSetProjection\0")?;
        if set_projection(output_dataset.raw(), projection.as_ptr()) != 0 {
            return Err(error(
                NativeGisErrorCode::DemReprojectionFailed,
                "日期变更线 DEM 输出 CRS 无法写入",
            ));
        }
        let mut output_transform = west_transform;
        let set_transform =
            *runtime.gdal_symbol::<GdalSetGeoTransform>(b"GDALSetGeoTransform\0")?;
        if set_transform(output_dataset.raw(), output_transform.as_mut_ptr()) != 0 {
            return Err(error(
                NativeGisErrorCode::DemReprojectionFailed,
                "日期变更线 DEM 输出地理变换无法写入",
            ));
        }
        let output_band = get_band(output_dataset.raw(), 1);
        if output_band.is_null() {
            return Err(error(
                NativeGisErrorCode::DemReprojectionFailed,
                "日期变更线 DEM 输出波段无法创建",
            ));
        }
        if west_has_nodata != 0 {
            let set_nodata =
                *runtime.gdal_symbol::<GdalSetRasterNoDataValue>(b"GDALSetRasterNoDataValue\0")?;
            if set_nodata(output_band, west_nodata) != 0 {
                return Err(error(
                    NativeGisErrorCode::DemReprojectionFailed,
                    "日期变更线 DEM 输出 nodata 无法写入",
                ));
            }
        }
        let raster_io = *runtime.gdal_symbol::<GdalRasterIo>(b"GDALRasterIO\0")?;
        let mut west_row = vec![0.0f32; west_width as usize];
        let mut east_row = vec![0.0f32; east_width as usize];
        for row in 0..west_height {
            for (source_band, source_width, output_width, x_offset, buffer) in [
                (west_band, west_width, west_width, 0, &mut west_row),
                (east_band, east_width, east_width, west_width, &mut east_row),
            ] {
                if raster_io(
                    source_band,
                    GDAL_READ,
                    0,
                    row,
                    source_width,
                    1,
                    buffer.as_mut_ptr().cast(),
                    output_width,
                    1,
                    GDAL_FLOAT32,
                    0,
                    0,
                ) != 0
                    || raster_io(
                        output_band,
                        GDAL_WRITE,
                        x_offset,
                        row,
                        output_width,
                        1,
                        buffer.as_mut_ptr().cast(),
                        output_width,
                        1,
                        GDAL_FLOAT32,
                        0,
                        0,
                    ) != 0
                {
                    return Err(error(
                        NativeGisErrorCode::DemReprojectionFailed,
                        "日期变更线 DEM 像元读取或写入失败",
                    ));
                }
            }
        }
        let flush = *runtime.gdal_symbol::<GdalFlushCache>(b"GDALFlushCache\0")?;
        flush(output_dataset.raw());
        Ok::<_, NativeGisError>((west_width, east_width, west_height, output_transform))
    };
    if result.is_err() {
        let _ = fs::remove_file(&output_part);
    }
    let (west_width, east_width, height, transform) = result?;
    finalize_output(&output_part, output)?;
    let inspection = match inspect_dem(runtime, output) {
        Ok(value) => value,
        Err(error) => {
            let _ = fs::remove_file(output);
            return Err(error);
        }
    };
    let metadata = inspection
        .get("metadata")
        .and_then(serde_json::Value::as_object);
    let expected_width = u64::try_from(west_width)
        .unwrap_or(0)
        .saturating_add(u64::try_from(east_width).unwrap_or(0));
    if metadata
        .and_then(|value| value.get("width"))
        .and_then(serde_json::Value::as_u64)
        != Some(expected_width)
        || metadata
            .and_then(|value| value.get("height"))
            .and_then(serde_json::Value::as_u64)
            != u64::try_from(height).ok()
        || metadata
            .and_then(|value| value.get("count"))
            .and_then(serde_json::Value::as_u64)
            != Some(1)
    {
        let _ = fs::remove_file(output);
        return Err(error(
            NativeGisErrorCode::DemReprojectionFailed,
            "日期变更线 DEM promotion 后栅格身份校验失败",
        ));
    }
    Ok(json!({
        "ok": true,
        "executed": true,
        "operation": "merge_antimeridian_dem",
        "west_input": west_input,
        "east_input": east_input,
        "output_path": output,
        "west_width": west_width,
        "east_width": east_width,
        "height": height,
        "output_transform": transform,
        "resampling": serde_json::Value::Null,
        "bytes_written": fs::metadata(output).map(|metadata| metadata.len()).unwrap_or(0),
        "inspection": inspection,
    }))
}

unsafe fn target_crs_to_wkt(
    runtime: &NativeGisRuntime,
    context: *mut c_void,
    target_crs: *const c_char,
) -> Result<CString, NativeGisError> {
    let target_text = CStr::from_ptr(target_crs).to_string_lossy();
    if target_text.contains('[') {
        return CString::new(target_text.as_bytes()).map_err(|_| {
            error(
                NativeGisErrorCode::DemReprojectionFailed,
                "目标 CRS WKT 编码无效",
            )
        });
    }
    let create = *runtime.proj_symbol::<ProjCreate>(b"proj_create\0")?;
    let as_wkt = *runtime.proj_symbol::<ProjAsWkt>(b"proj_as_wkt\0")?;
    let destroy = *runtime.proj_symbol::<ProjDestroy>(b"proj_destroy\0")?;
    let crs = create(context, target_crs);
    if crs.is_null() {
        return Err(error(
            NativeGisErrorCode::DemReprojectionFailed,
            "PROJ 无法创建目标 CRS",
        ));
    }
    let wkt_ptr = as_wkt(context, crs, PJ_WKT2_2019, std::ptr::null());
    if wkt_ptr.is_null() {
        destroy(crs);
        return Err(error(
            NativeGisErrorCode::DemReprojectionFailed,
            "PROJ 无法导出目标 CRS WKT",
        ));
    }
    let wkt = CStr::from_ptr(wkt_ptr).to_bytes().to_vec();
    destroy(crs);
    CString::new(wkt).map_err(|_| {
        error(
            NativeGisErrorCode::DemReprojectionFailed,
            "目标 CRS WKT 编码无效",
        )
    })
}

fn world_to_pixel(transform: &[f64; 6], x: f64, y: f64) -> Option<(f64, f64)> {
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    let determinant = transform[1] * transform[5] - transform[2] * transform[4];
    if !determinant.is_finite() || determinant.abs() < f64::EPSILON {
        return None;
    }
    let translated_x = x - transform[0];
    let translated_y = y - transform[3];
    let pixel = (transform[5] * translated_x - transform[2] * translated_y) / determinant - 0.5;
    let line = (-transform[4] * translated_x + transform[1] * translated_y) / determinant - 0.5;
    Some((pixel, line))
}

fn missing_sample(nodata: Option<f64>) -> f32 {
    nodata.map(|value| value as f32).unwrap_or(f32::NAN)
}

fn sample_raster(
    values: &[f32],
    width: usize,
    height: usize,
    x: f64,
    y: f64,
    nodata: Option<f64>,
    resampling: &str,
) -> f32 {
    if !x.is_finite()
        || !y.is_finite()
        || x < 0.0
        || y < 0.0
        || x > width.saturating_sub(1) as f64
        || y > height.saturating_sub(1) as f64
    {
        return missing_sample(nodata);
    }
    let is_valid = |value: f32| {
        value.is_finite()
            && !nodata.is_some_and(|candidate| {
                if candidate.is_nan() {
                    value.is_nan()
                } else {
                    f64::from(value) == candidate
                }
            })
    };
    let index = |column: usize, row: usize| row * width + column;
    if resampling == "nearest" {
        let column = x.round() as usize;
        let row = y.round() as usize;
        let value = values[index(column.min(width - 1), row.min(height - 1))];
        return if is_valid(value) {
            value
        } else {
            missing_sample(nodata)
        };
    }
    let x0 = x.floor() as usize;
    let y0 = y.floor() as usize;
    let x1 = (x0 + 1).min(width - 1);
    let y1 = (y0 + 1).min(height - 1);
    let wx = if x1 == x0 { 0.0 } else { x - x0 as f64 };
    let wy = if y1 == y0 { 0.0 } else { y - y0 as f64 };
    let samples = [
        values[index(x0, y0)],
        values[index(x1, y0)],
        values[index(x0, y1)],
        values[index(x1, y1)],
    ];
    if samples.iter().copied().any(|value| !is_valid(value)) {
        return missing_sample(nodata);
    }
    let top = f64::from(samples[0]) * (1.0 - wx) + f64::from(samples[1]) * wx;
    let bottom = f64::from(samples[2]) * (1.0 - wx) + f64::from(samples[3]) * wx;
    (top * (1.0 - wy) + bottom * wy) as f32
}

struct RasterOutputRequest<'a> {
    runtime: &'a NativeGisRuntime,
    input: &'a Path,
    width: c_int,
    height: c_int,
    projection: &'a CString,
    transform: &'a mut [f64; 6],
    nodata: Option<f64>,
    area_or_point: Option<&'a str>,
    operation: Option<&'a (ProjOperationGuard, ProjTrans)>,
    output: &'a Path,
    driver_name: &'a str,
}

unsafe fn write_raster_output(request: RasterOutputRequest<'_>) -> Result<(), NativeGisError> {
    if request.driver_name == "ENVI" {
        return write_envi_output(request);
    }
    let RasterOutputRequest {
        runtime,
        input,
        width,
        height,
        projection,
        transform,
        nodata,
        area_or_point,
        operation,
        output,
        driver_name,
    } = request;
    let output_c = c_path(
        output,
        NativeGisErrorCode::DemReprojectionFailed,
        "DEM 输出路径编码无效",
    )?;
    let driver_c = CString::new(driver_name).expect("static GDAL driver");
    let get_driver = *runtime.gdal_symbol::<GdalGetDriverByName>(b"GDALGetDriverByName\0")?;
    let driver = get_driver(driver_c.as_ptr());
    if driver.is_null() {
        return Err(error(
            NativeGisErrorCode::GisDriverUnavailable,
            "产品 GDAL runtime 缺少所需输出 driver",
        ));
    }
    let create = *runtime.gdal_symbol::<GdalCreate>(b"GDALCreate\0")?;
    let close = *runtime.gdal_symbol::<GdalClose>(b"GDALClose\0")?;
    let dataset = create(
        driver,
        output_c.as_ptr(),
        width,
        height,
        1,
        GDAL_FLOAT32,
        std::ptr::null(),
    );
    if dataset.is_null() {
        return Err(error(
            NativeGisErrorCode::DemReprojectionFailed,
            "GDAL 无法创建 GTiff DEM 输出栅格",
        ));
    }
    let output_dataset = GdalDatasetGuard::new(dataset, close);
    let set_projection = *runtime.gdal_symbol::<GdalSetProjection>(b"GDALSetProjection\0")?;
    if set_projection(output_dataset.raw(), projection.as_ptr()) != 0 {
        return Err(error(
            NativeGisErrorCode::DemReprojectionFailed,
            "DEM 输出 CRS 无法写入",
        ));
    }
    let set_transform = *runtime.gdal_symbol::<GdalSetGeoTransform>(b"GDALSetGeoTransform\0")?;
    if set_transform(output_dataset.raw(), transform.as_mut_ptr()) != 0 {
        return Err(error(
            NativeGisErrorCode::DemReprojectionFailed,
            "DEM 输出地理变换无法写入",
        ));
    }
    if let Some(area_or_point) = area_or_point {
        let set_metadata_item =
            *runtime.gdal_symbol::<GdalSetMetadataItem>(b"GDALSetMetadataItem\0")?;
        let key = CString::new("AREA_OR_POINT").expect("static GDAL metadata key");
        let value = CString::new(area_or_point).map_err(|_| {
            error(
                NativeGisErrorCode::DemReprojectionFailed,
                "DEM AREA_OR_POINT metadata 编码无效",
            )
        })?;
        if set_metadata_item(
            output_dataset.raw(),
            key.as_ptr(),
            value.as_ptr(),
            std::ptr::null(),
        ) != 0
        {
            return Err(error(
                NativeGisErrorCode::DemReprojectionFailed,
                "DEM AREA_OR_POINT metadata 无法写入",
            ));
        }
    }
    let get_band = *runtime.gdal_symbol::<GdalGetRasterBand>(b"GDALGetRasterBand\0")?;
    let output_band = get_band(output_dataset.raw(), 1);
    if output_band.is_null() {
        return Err(error(
            NativeGisErrorCode::DemReprojectionFailed,
            "DEM 输出波段无法创建",
        ));
    }
    if let Some(nodata) = nodata {
        let set_nodata =
            *runtime.gdal_symbol::<GdalSetRasterNoDataValue>(b"GDALSetRasterNoDataValue\0")?;
        if set_nodata(output_band, nodata) != 0 {
            return Err(error(
                NativeGisErrorCode::DemReprojectionFailed,
                "DEM 输出 nodata 无法写入",
            ));
        }
    }
    let input_c = c_path(
        input,
        NativeGisErrorCode::DemInvalidRaster,
        "DEM 路径编码无效",
    )?;
    let allowed_driver_names = [
        CString::new("GTiff").expect("static GDAL driver"),
        CString::new("ENVI").expect("static GDAL driver"),
    ];
    let allowed_driver_ptrs = [
        allowed_driver_names[0].as_ptr(),
        allowed_driver_names[1].as_ptr(),
        std::ptr::null(),
    ];
    let open = *runtime.gdal_symbol::<GdalOpenEx>(b"GDALOpenEx\0")?;
    let siblings = raster_siblings(input)?;
    let sibling_ptrs: Vec<_> = siblings
        .iter()
        .map(|name| name.as_ptr())
        .chain(std::iter::once(std::ptr::null()))
        .collect();
    let input_dataset = open(
        input_c.as_ptr(),
        GDAL_OF_RASTER | GDAL_ACCESS_READ_ONLY,
        allowed_driver_ptrs.as_ptr(),
        std::ptr::null(),
        sibling_ptrs.as_ptr(),
    );
    if input_dataset.is_null() {
        return Err(error(
            NativeGisErrorCode::DemInvalidRaster,
            "DEM 不是产品允许的 GTiff/ENVI 栅格",
        ));
    }
    let input_dataset = GdalDatasetGuard::new(input_dataset, close);
    let input_band = get_band(input_dataset.raw(), 1);
    if input_band.is_null() {
        return Err(error(
            NativeGisErrorCode::DemInvalidRaster,
            "DEM 第一波段不可读取",
        ));
    }
    let raster_io = *runtime.gdal_symbol::<GdalRasterIo>(b"GDALRasterIO\0")?;
    let flush = *runtime.gdal_symbol::<GdalFlushCache>(b"GDALFlushCache\0")?;
    let mut source_row = vec![0.0f32; width as usize];
    let mut output_row = vec![0.0f32; width as usize];
    for row in 0..height {
        if raster_io(
            input_band,
            GDAL_READ,
            0,
            row,
            width,
            1,
            source_row.as_mut_ptr().cast(),
            width,
            1,
            GDAL_FLOAT32,
            0,
            0,
        ) != 0
        {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                "DEM 像元读取失败",
            ));
        }
        for column in 0..width as usize {
            let value = source_row[column];
            if nodata.is_some_and(|candidate| f64::from(value) == candidate) {
                output_row[column] = value;
                continue;
            }
            let Some((operation, transform_point)) = operation else {
                output_row[column] = value;
                continue;
            };
            let coord = ProjCoord {
                x: transform[0]
                    + transform[1] * (column as f64 + 0.5)
                    + transform[2] * (row as f64 + 0.5),
                y: transform[3]
                    + transform[4] * (column as f64 + 0.5)
                    + transform[5] * (row as f64 + 0.5),
                z: f64::from(value),
                t: 0.0,
            };
            let transformed = transform_point(operation.raw(), PJ_FWD, coord);
            if !transformed.z.is_finite() {
                return Err(error(
                    NativeGisErrorCode::VerticalTransformFailed,
                    "PROJ 垂直转换返回无效高程",
                ));
            }
            output_row[column] = transformed.z as f32;
        }
        if raster_io(
            output_band,
            GDAL_WRITE,
            0,
            row,
            width,
            1,
            output_row.as_mut_ptr().cast(),
            width,
            1,
            GDAL_FLOAT32,
            0,
            0,
        ) != 0
        {
            return Err(error(
                NativeGisErrorCode::DemReprojectionFailed,
                "DEM 像元写入失败",
            ));
        }
    }
    flush(output_dataset.raw());
    Ok(())
}

unsafe fn write_envi_output(request: RasterOutputRequest<'_>) -> Result<(), NativeGisError> {
    let RasterOutputRequest {
        runtime,
        input,
        width,
        height,
        projection,
        transform,
        nodata,
        area_or_point: _,
        operation,
        output,
        driver_name: _,
    } = request;
    let header = sarscape_part_header_path(output)?;
    let mut data = BufWriter::new(fs::File::create(output).map_err(|_| {
        error(
            NativeGisErrorCode::DemReprojectionFailed,
            "ENVI DEM 临时数据文件无法创建",
        )
    })?);
    let input_c = c_path(
        input,
        NativeGisErrorCode::DemInvalidRaster,
        "DEM 路径编码无效",
    )?;
    let allowed_driver_names = [
        CString::new("GTiff").expect("static GDAL driver"),
        CString::new("ENVI").expect("static GDAL driver"),
    ];
    let allowed_driver_ptrs = [
        allowed_driver_names[0].as_ptr(),
        allowed_driver_names[1].as_ptr(),
        std::ptr::null(),
    ];
    let open = *runtime.gdal_symbol::<GdalOpenEx>(b"GDALOpenEx\0")?;
    let siblings = raster_siblings(input)?;
    let sibling_ptrs: Vec<_> = siblings
        .iter()
        .map(|name| name.as_ptr())
        .chain(std::iter::once(std::ptr::null()))
        .collect();
    let input_dataset = open(
        input_c.as_ptr(),
        GDAL_OF_RASTER | GDAL_ACCESS_READ_ONLY,
        allowed_driver_ptrs.as_ptr(),
        std::ptr::null(),
        sibling_ptrs.as_ptr(),
    );
    if input_dataset.is_null() {
        return Err(error(
            NativeGisErrorCode::DemInvalidRaster,
            "DEM 不是产品允许的 GTiff/ENVI 栅格",
        ));
    }
    let close = *runtime.gdal_symbol::<GdalClose>(b"GDALClose\0")?;
    let input_dataset = GdalDatasetGuard::new(input_dataset, close);
    let get_band = *runtime.gdal_symbol::<GdalGetRasterBand>(b"GDALGetRasterBand\0")?;
    let input_band = get_band(input_dataset.raw(), 1);
    if input_band.is_null() {
        return Err(error(
            NativeGisErrorCode::DemInvalidRaster,
            "DEM 第一波段不可读取",
        ));
    }
    let raster_io = *runtime.gdal_symbol::<GdalRasterIo>(b"GDALRasterIO\0")?;
    let mut source_row = vec![0.0f32; width as usize];
    let mut output_row = vec![0.0f32; width as usize];
    let mut row_bytes = vec![0u8; width as usize * std::mem::size_of::<f32>()];
    for row in 0..height {
        if raster_io(
            input_band,
            GDAL_READ,
            0,
            row,
            width,
            1,
            source_row.as_mut_ptr().cast(),
            width,
            1,
            GDAL_FLOAT32,
            0,
            0,
        ) != 0
        {
            return Err(error(
                NativeGisErrorCode::DemInvalidRaster,
                "DEM 像元读取失败",
            ));
        }
        for column in 0..width as usize {
            let value = source_row[column];
            if nodata.is_some_and(|candidate| f64::from(value) == candidate) {
                output_row[column] = value;
                continue;
            }
            let Some((operation, transform_point)) = operation else {
                output_row[column] = value;
                continue;
            };
            let coord = ProjCoord {
                x: transform[0]
                    + transform[1] * (column as f64 + 0.5)
                    + transform[2] * (row as f64 + 0.5),
                y: transform[3]
                    + transform[4] * (column as f64 + 0.5)
                    + transform[5] * (row as f64 + 0.5),
                z: f64::from(value),
                t: 0.0,
            };
            let transformed = transform_point(operation.raw(), PJ_FWD, coord);
            if !transformed.z.is_finite() {
                return Err(error(
                    NativeGisErrorCode::VerticalTransformFailed,
                    "PROJ 垂直转换返回无效高程",
                ));
            }
            output_row[column] = transformed.z as f32;
        }
        for (index, value) in output_row.iter().enumerate() {
            row_bytes[index * 4..(index + 1) * 4].copy_from_slice(&value.to_le_bytes());
        }
        data.write_all(&row_bytes).map_err(|_| {
            error(
                NativeGisErrorCode::DemReprojectionFailed,
                "ENVI DEM 像元写入失败",
            )
        })?;
    }
    data.flush().map_err(|_| {
        error(
            NativeGisErrorCode::DemReprojectionFailed,
            "ENVI DEM 临时数据文件无法刷新",
        )
    })?;
    let projection_text = CStr::from_ptr(projection.as_ptr()).to_string_lossy();
    write_envi_header(&header, width, height, transform, nodata, &projection_text)?;
    Ok(())
}

fn write_envi_header(
    path: &Path,
    width: c_int,
    height: c_int,
    transform: &[f64; 6],
    nodata: Option<f64>,
    projection: &str,
) -> Result<(), NativeGisError> {
    let mut text = format!(
        "ENVI\ndescription = {{EOStage native ellipsoidal DEM}}\nsamples = {width}\nlines = {height}\nbands = 1\nheader offset = 0\nfile type = ENVI Standard\ndata type = 4\ninterleave = bsq\nbyte order = 0\nmap info = {{Geographic Lat/Lon, 1.000000, 1.000000, {}, {}, {}, {}, WGS-84}}\ncoordinate system string = {{{}}}\n",
        transform[0],
        transform[3],
        transform[1],
        transform[5].abs(),
        projection
    );
    if let Some(nodata) = nodata {
        text.push_str(&format!("data ignore value = {nodata}\n"));
    }
    fs::write(path, text).map_err(|_| {
        error(
            NativeGisErrorCode::DemReprojectionFailed,
            "ENVI DEM header 无法写入",
        )
    })
}

fn c_path(path: &Path, code: NativeGisErrorCode, reason: &str) -> Result<CString, NativeGisError> {
    let text = path
        .to_str()
        .ok_or_else(|| error(code, reason.to_owned()))?;
    CString::new(text).map_err(|_| error(code, reason.to_owned()))
}

fn part_path(path: &Path) -> Result<std::path::PathBuf, NativeGisError> {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            error(
                NativeGisErrorCode::DemReprojectionFailed,
                "DEM 输出文件名无效",
            )
        })?;
    Ok(path.with_file_name(format!("{name}.part")))
}

fn sarscape_part_path(path: &Path) -> Result<std::path::PathBuf, NativeGisError> {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            error(
                NativeGisErrorCode::DemReprojectionFailed,
                "DEM 输出文件名无效",
            )
        })?;
    // The ENVI driver replaces the final extension with `.hdr`.  Keep a
    // staging `.bin` extension so its sidecar remains task-scoped as
    // `<output>.part.hdr`, then promote both data and header together.
    Ok(path.with_file_name(format!("{name}.part.bin")))
}

fn sarscape_part_header_path(path: &Path) -> Result<std::path::PathBuf, NativeGisError> {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            error(
                NativeGisErrorCode::DemReprojectionFailed,
                "DEM 输出文件名无效",
            )
        })?;
    let stem = name.strip_suffix(".bin").ok_or_else(|| {
        error(
            NativeGisErrorCode::DemReprojectionFailed,
            "ENVI 临时输出文件名无效",
        )
    })?;
    Ok(path.with_file_name(format!("{stem}.hdr")))
}

fn sidecar_path(path: &Path, suffix: &str) -> Result<std::path::PathBuf, NativeGisError> {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            error(
                NativeGisErrorCode::DemReprojectionFailed,
                "DEM 输出文件名无效",
            )
        })?;
    Ok(path.with_file_name(format!("{name}{suffix}")))
}

fn remove_stale_output(path: &Path) -> Result<(), NativeGisError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(error(
            NativeGisErrorCode::DemReprojectionFailed,
            "无法清理 DEM 临时输出",
        )),
    }
}

fn validate_sarscape_staged(part: &Path) -> Result<(), NativeGisError> {
    let invalid = || {
        error(
            NativeGisErrorCode::DemReprojectionFailed,
            "SARscape 三文件输出缺失、为空或尺寸不一致",
        )
    };
    let header_path = sarscape_part_header_path(part)?;
    let sml_path = sidecar_path(part, ".sml")?;
    for path in [part, header_path.as_path(), sml_path.as_path()] {
        let metadata = fs::metadata(path).map_err(|_| invalid())?;
        if !metadata.is_file() || metadata.len() == 0 {
            return Err(invalid());
        }
    }
    if fs::metadata(&header_path).map_err(|_| invalid())?.len() > 128 * 1024 {
        return Err(invalid());
    }
    let header = fs::read_to_string(header_path).map_err(|_| invalid())?;
    let scalar = |key: &str| -> Option<String> {
        let mut values = header.lines().filter_map(|line| {
            let (name, value) = line.split_once('=')?;
            name.trim()
                .eq_ignore_ascii_case(key)
                .then(|| value.trim().to_ascii_lowercase())
        });
        let value = values.next()?;
        values.next().is_none().then_some(value)
    };
    let dimension = |key| {
        scalar(key)
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value > 0)
    };
    let expected_bytes = dimension("samples")
        .zip(dimension("lines"))
        .and_then(|(width, height)| width.checked_mul(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(invalid)?;
    if scalar("bands").as_deref() != Some("1")
        || scalar("header offset").as_deref() != Some("0")
        || scalar("data type").as_deref() != Some("4")
        || scalar("interleave").as_deref() != Some("bsq")
        || scalar("byte order").as_deref() != Some("0")
        || fs::metadata(part).map_err(|_| invalid())?.len() != expected_bytes
    {
        return Err(invalid());
    }
    Ok(())
}

fn finalize_output(part: &Path, output: &Path) -> Result<(), NativeGisError> {
    if output.is_file() {
        fs::remove_file(output).map_err(|_| {
            error(
                NativeGisErrorCode::DemReprojectionFailed,
                "无法替换 DEM 输出文件",
            )
        })?;
    }
    fs::rename(part, output).map_err(|_| {
        error(
            NativeGisErrorCode::DemReprojectionFailed,
            "无法提交 DEM 输出文件",
        )
    })
}

/// Match v2.1.9's WKT1_ESRI serialization without changing raster coordinates.
fn sarscape_wkt(runtime: &NativeGisRuntime, projection: &str) -> Result<String, NativeGisError> {
    type NewSpatialReference = unsafe extern "C" fn(*const c_char) -> *mut c_void;
    type DestroySpatialReference = unsafe extern "C" fn(*mut c_void);
    type ExportWkt =
        unsafe extern "C" fn(*mut c_void, *mut *mut c_char, *const *const c_char) -> c_int;
    type Free = unsafe extern "C" fn(*mut c_void);
    let projection = CString::new(projection)
        .map_err(|_| error(NativeGisErrorCode::DemCrsMissing, "SARscape CRS 编码无效"))?;
    let format = CString::new("FORMAT=WKT1_ESRI").expect("static export option");
    let multiline = CString::new("MULTILINE=NO").expect("static export option");
    let options = [format.as_ptr(), multiline.as_ptr(), std::ptr::null()];
    unsafe {
        // Resolve every function before acquiring foreign-owned allocations.
        let create = *runtime.gdal_symbol::<NewSpatialReference>(b"OSRNewSpatialReference\0")?;
        let destroy =
            *runtime.gdal_symbol::<DestroySpatialReference>(b"OSRDestroySpatialReference\0")?;
        let export = *runtime.gdal_symbol::<ExportWkt>(b"OSRExportToWktEx\0")?;
        let free = *runtime.gdal_symbol::<Free>(b"VSIFree\0")?;
        let spatial = create(projection.as_ptr());
        if spatial.is_null() {
            return Err(error(
                NativeGisErrorCode::DemCrsMissing,
                "无法读取 SARscape 坐标系",
            ));
        }
        let mut output: *mut c_char = std::ptr::null_mut();
        let status = export(spatial, &mut output, options.as_ptr());
        destroy(spatial);
        let text = if status == 0 && !output.is_null() {
            CStr::from_ptr(output)
                .to_str()
                .ok()
                .filter(|text| !text.is_empty())
                .map(str::to_owned)
        } else {
            None
        };
        if !output.is_null() {
            free(output.cast());
        }
        text.ok_or_else(|| {
            error(
                NativeGisErrorCode::DemCrsMissing,
                "无法导出 SARscape WKT1_ESRI 坐标系",
            )
        })
    }
}

fn write_sarscape_sml(
    output: &Path,
    width: c_int,
    height: c_int,
    transform: &[f64; 6],
    nodata: Option<f64>,
    projection: &str,
) -> Result<(), NativeGisError> {
    let west = transform[0];
    let north = transform[3];
    let east = west + transform[1] * f64::from(width) + transform[2] * f64::from(height);
    let south = north + transform[4] * f64::from(width) + transform[5] * f64::from(height);
    let number = |value: f64| {
        if value.is_nan() {
            "NaN".to_owned()
        } else {
            // Fixed decimal places lose significant digits for arc-second grids.
            // Display preserves the exact f64 on parse, also for small spacings.
            value.to_string()
        }
    };
    let text = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<HEADER_INFO xmlns=\"http://www.sarmap.ch/xml/SARscapeHeaderSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:schemaLocation=\"http://www.sarmap.ch/xml/SARscapeHeaderSchema http://www.sarmap.ch/xml/SARscapeHeaderSchema/SARscapeHeaderSchema_version_2.0.2.xsd\">\n  <RasterInfo>\n    <HeaderOffset>0</HeaderOffset>\n    <RowPrefix>0</RowPrefix>\n    <RowSuffix>0</RowSuffix>\n    <CellType>FLOAT</CellType>\n    <DataUnits>ELLIPSOIDAL DEM</DataUnits>\n    <NullCellValue>{}</NullCellValue>\n    <NrOfChannels>1</NrOfChannels>\n    <NrOfPixelsPerLine>{width}</NrOfPixelsPerLine>\n    <NrOfLinesPerImage>{height}</NrOfLinesPerImage>\n    <GeocodedImage>OK</GeocodedImage>\n    <Interleave>LAYOUT_BSQ</Interleave>\n    <SmlVersion>2.0.2</SmlVersion>\n    <BytesOrder>LSBF</BytesOrder>\n    <OtherInfo>\n      <MatrixString NumberOfRows=\"2\" NumberOfColumns=\"2\">\n        <MatrixRowString ID=\"0\"><ValueString ID=\"0\">SOFTWARE</ValueString><ValueString ID=\"1\">EOStage</ValueString></MatrixRowString>\n        <MatrixRowString ID=\"1\"><ValueString ID=\"0\">SML VERSION</ValueString><ValueString ID=\"1\">2.0.2</ValueString></MatrixRowString>\n      </MatrixString>\n    </OtherInfo>\n  </RasterInfo>\n  <CartographicSystem><cartographic_system_specification>{}</cartographic_system_specification></CartographicSystem>\n  <RegistrationCoordinates>\n    <LatNorthing>{}</LatNorthing><LonEasting>{}</LonEasting>\n    <PixelSpacingLatNorth>{}</PixelSpacingLatNorth><PixelSpacingLonEast>{}</PixelSpacingLonEast>\n  </RegistrationCoordinates>\n  <DEMCoordinates>\n    <EastingCoordinateBegin>{}</EastingCoordinateBegin><EastingCoordinateEnd>{}</EastingCoordinateEnd>\n    <EastingGridSize>{}</EastingGridSize><NorthingCoordinateBegin>{}</NorthingCoordinateBegin>\n    <NorthingCoordinateEnd>{}</NorthingCoordinateEnd><NorthingGridSize>{}</NorthingGridSize>\n  </DEMCoordinates>\n</HEADER_INFO>\n",
        nodata.map(number).unwrap_or_else(|| "NaN".to_owned()),
        xml_escape(projection),
        number(north),
        number(west),
        number(transform[5]),
        number(transform[1]),
        number(west),
        number(east),
        number(transform[1].abs()),
        number(south),
        number(north),
        number(transform[5].abs()),
    );
    fs::write(sidecar_path(output, ".sml")?, text).map_err(|_| {
        error(
            NativeGisErrorCode::DemReprojectionFailed,
            "SARscape SML sidecar 无法写入",
        )
    })
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn gdal_data_type(value: c_int) -> &'static str {
    match value {
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

fn is_wgs84_geographic_crs(projection: &str) -> bool {
    let upper = projection.to_ascii_uppercase();
    let is_geographic = upper.contains("GEOGCS[") || upper.contains("GEOGCRS[");
    let is_wgs84 = upper.contains("WGS 84")
        || upper.contains("WGS_1984")
        || upper.contains("WGS84")
        || upper.contains("EPSG:4326")
        || upper.contains("EPSG\",\"4326")
        || upper.contains("EPSG\",4326");
    is_geographic && is_wgs84
}

fn error(code: NativeGisErrorCode, reason: impl Into<String>) -> NativeGisError {
    NativeGisError::contract(code, reason)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{NativeGisRuntime, NativeGisRuntimeIdentity};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_NATIVE_TEST_ID: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn native_dem_crs_gate_rejects_projected_or_non_wgs84_inputs() {
        assert!(is_wgs84_geographic_crs(
            r#"GEOGCS["WGS 84",DATUM["WGS_1984"],AUTHORITY["EPSG","4326"]]"#
        ));
        assert!(!is_wgs84_geographic_crs(
            r#"PROJCS["WGS 84 / UTM zone 32N",AUTHORITY["EPSG","32632"]]"#
        ));
        assert!(!is_wgs84_geographic_crs(
            r#"GEOGCRS["NAD83",DATUM["North American Datum 1983"]]"#
        ));
    }

    unsafe fn write_test_geotiff_values(
        runtime: &NativeGisRuntime,
        path: &Path,
        width: c_int,
        height: c_int,
        mut transform: [f64; 6],
        values: &[f32],
    ) {
        assert_eq!(values.len(), (width * height) as usize);
        runtime.ensure_gdal_registered().expect("GDALAllRegister");
        let get_driver = *runtime
            .gdal_symbol::<GdalGetDriverByName>(b"GDALGetDriverByName\0")
            .expect("GDALGetDriverByName");
        let driver_name = CString::new("GTiff").unwrap();
        let driver = get_driver(driver_name.as_ptr());
        assert!(!driver.is_null(), "product runtime must expose GTiff");
        let create = *runtime
            .gdal_symbol::<GdalCreate>(b"GDALCreate\0")
            .expect("GDALCreate");
        let close = *runtime
            .gdal_symbol::<GdalClose>(b"GDALClose\0")
            .expect("GDALClose");
        let output = c_path(
            path,
            NativeGisErrorCode::DemReprojectionFailed,
            "test GeoTIFF path",
        )
        .expect("test path");
        let dataset = create(
            driver,
            output.as_ptr(),
            width,
            height,
            1,
            GDAL_FLOAT32,
            std::ptr::null(),
        );
        assert!(!dataset.is_null(), "create synthetic GeoTIFF");
        let dataset = GdalDatasetGuard::new(dataset, close);
        let projection = CString::new(
            r#"GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433],AUTHORITY["EPSG","4326"]]"#,
        )
        .unwrap();
        let set_projection = *runtime
            .gdal_symbol::<GdalSetProjection>(b"GDALSetProjection\0")
            .expect("GDALSetProjection");
        assert_eq!(set_projection(dataset.raw(), projection.as_ptr()), 0);
        let set_transform = *runtime
            .gdal_symbol::<GdalSetGeoTransform>(b"GDALSetGeoTransform\0")
            .expect("GDALSetGeoTransform");
        assert_eq!(set_transform(dataset.raw(), transform.as_mut_ptr()), 0);
        let get_band = *runtime
            .gdal_symbol::<GdalGetRasterBand>(b"GDALGetRasterBand\0")
            .expect("GDALGetRasterBand");
        let band = get_band(dataset.raw(), 1);
        assert!(!band.is_null(), "synthetic GeoTIFF band");
        let raster_io = *runtime
            .gdal_symbol::<GdalRasterIo>(b"GDALRasterIO\0")
            .expect("GDALRasterIO");
        let mut values = values.to_vec();
        assert_eq!(
            raster_io(
                band,
                GDAL_WRITE,
                0,
                0,
                width,
                height,
                values.as_mut_ptr().cast(),
                width,
                height,
                GDAL_FLOAT32,
                0,
                0,
            ),
            0
        );
        let flush = *runtime
            .gdal_symbol::<GdalFlushCache>(b"GDALFlushCache\0")
            .expect("GDALFlushCache");
        flush(dataset.raw());
    }

    unsafe fn read_test_geotiff(runtime: &NativeGisRuntime, path: &Path) -> Vec<f32> {
        let open = *runtime
            .gdal_symbol::<GdalOpenEx>(b"GDALOpenEx\0")
            .expect("GDALOpenEx");
        let close = *runtime
            .gdal_symbol::<GdalClose>(b"GDALClose\0")
            .expect("GDALClose");
        let input = c_path(
            path,
            NativeGisErrorCode::DemInvalidRaster,
            "test GeoTIFF path",
        )
        .expect("test path");
        let driver_name = CString::new("GTiff").unwrap();
        let drivers = [driver_name.as_ptr(), std::ptr::null()];
        let dataset = open(
            input.as_ptr(),
            GDAL_OF_RASTER | GDAL_ACCESS_READ_ONLY,
            drivers.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
        );
        assert!(!dataset.is_null(), "open synthetic GeoTIFF");
        let dataset = GdalDatasetGuard::new(dataset, close);
        let get_width = *runtime
            .gdal_symbol::<GdalGetRasterXSize>(b"GDALGetRasterXSize\0")
            .expect("GDALGetRasterXSize");
        let get_height = *runtime
            .gdal_symbol::<GdalGetRasterYSize>(b"GDALGetRasterYSize\0")
            .expect("GDALGetRasterYSize");
        let width = get_width(dataset.raw());
        let height = get_height(dataset.raw());
        let get_band = *runtime
            .gdal_symbol::<GdalGetRasterBand>(b"GDALGetRasterBand\0")
            .expect("GDALGetRasterBand");
        let band = get_band(dataset.raw(), 1);
        let raster_io = *runtime
            .gdal_symbol::<GdalRasterIo>(b"GDALRasterIO\0")
            .expect("GDALRasterIO");
        let mut values = vec![0.0f32; (width * height) as usize];
        assert_eq!(
            raster_io(
                band,
                GDAL_READ,
                0,
                0,
                width,
                height,
                values.as_mut_ptr().cast(),
                width,
                height,
                GDAL_FLOAT32,
                0,
                0,
            ),
            0
        );
        values
    }

    #[test]
    #[ignore = "requires INSAR_GIS_RUNTIME_ROOT pointing to a packaged product runtime"]
    fn native_dem_grid_integrity_fails_closed_without_resampling() {
        let runtime_root = std::env::var_os("INSAR_GIS_RUNTIME_ROOT")
            .map(PathBuf::from)
            .expect("INSAR_GIS_RUNTIME_ROOT must point to runtime/gis");
        let identity = NativeGisRuntimeIdentity::from_product_root(&runtime_root)
            .expect("discover product runtime identity");
        let runtime = NativeGisRuntime::load_identity(&identity).expect("load product runtime");
        verify_runtime_capabilities(&runtime).expect("verify product runtime exports");

        let id = NEXT_NATIVE_TEST_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "insar-gis-native-mosaic-{id}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();

        let tile_a = root.join("tile-a.tif");
        let tile_b = root.join("tile-b.tif");
        unsafe {
            write_test_geotiff_values(
                &runtime,
                &tile_a,
                4,
                2,
                [0.0, 0.25, 0.0, 1.0, 0.0, -0.5],
                &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0],
            );
            write_test_geotiff_values(
                &runtime,
                &tile_b,
                4,
                2,
                [1.0, 0.25, 0.0, 1.0, 0.0, -0.5],
                &[101.0, 102.0, 103.0, 104.0, 105.0, 106.0, 107.0, 108.0],
            );
        }
        let mosaic = root.join("mosaic.tif");
        let result =
            mosaic_crop_dem_tiles(&runtime, &[tile_a, tile_b], &mosaic, [0.0, 0.0, 2.0, 1.0])
                .expect("native GDAL VRT/Translate mosaic");
        assert_eq!(result["width"], 8);
        assert_eq!(result["height"], 2);
        assert!(result["resampling"].is_null());
        let values = unsafe { read_test_geotiff(&runtime, &mosaic) };
        assert_eq!(
            values,
            vec![
                1.0, 2.0, 3.0, 4.0, 101.0, 102.0, 103.0, 104.0, 5.0, 6.0, 7.0, 8.0, 105.0, 106.0,
                107.0, 108.0,
            ]
        );

        let mixed = root.join("tile-mixed.tif");
        unsafe {
            write_test_geotiff_values(
                &runtime,
                &mixed,
                2,
                2,
                [1.0, 0.5, 0.0, 1.0, 0.0, -0.5],
                &[201.0, 202.0, 203.0, 204.0],
            );
        }
        let mixed_output = root.join("mixed-mosaic.tif");
        let error = mosaic_crop_dem_tiles(
            &runtime,
            &[root.join("tile-a.tif"), mixed],
            &mixed_output,
            [0.0, 0.0, 2.0, 1.0],
        )
        .expect_err(
            "mixed native pixel spacing must fail instead of creating an averaged VRT grid",
        );
        assert_eq!(error.code(), NativeGisErrorCode::DemInvalidRaster);
        assert!(!mixed_output.exists());

        let west = root.join("west.tif");
        let east = root.join("east.tif");
        unsafe {
            write_test_geotiff_values(
                &runtime,
                &west,
                4,
                2,
                [179.0, 0.25, 0.0, 1.0, 0.0, -0.5],
                &[11.0, 12.0, 13.0, 14.0, 15.0, 16.0, 17.0, 18.0],
            );
            write_test_geotiff_values(
                &runtime,
                &east,
                4,
                2,
                [-180.0, 0.25, 0.0, 1.0, 0.0, -0.5],
                &[21.0, 22.0, 23.0, 24.0, 25.0, 26.0, 27.0, 28.0],
            );
        }
        let merged = root.join("merged.tif");
        let result = merge_antimeridian_dem(&runtime, &west, &east, &merged)
            .expect("native GDAL antimeridian merge");
        assert_eq!(result["west_width"], 4);
        assert_eq!(result["east_width"], 4);
        assert_eq!(
            inspect_dem(&runtime, &merged).unwrap()["metadata"]["width"],
            8
        );
        assert_eq!(
            unsafe { read_test_geotiff(&runtime, &merged) },
            vec![
                11.0, 12.0, 13.0, 14.0, 21.0, 22.0, 23.0, 24.0, 15.0, 16.0, 17.0, 18.0, 25.0, 26.0,
                27.0, 28.0,
            ]
        );

        let east_mixed = root.join("east-mixed.tif");
        unsafe {
            write_test_geotiff_values(
                &runtime,
                &east_mixed,
                2,
                2,
                [-180.0, 0.5, 0.0, 1.0, 0.0, -0.5],
                &[31.0, 32.0, 33.0, 34.0],
            );
        }
        let mixed_merged = root.join("mixed-merged.tif");
        let error = merge_antimeridian_dem(&runtime, &west, &east_mixed, &mixed_merged)
            .expect_err("antimeridian windows with different x spacing must not be resampled");
        assert_eq!(error.code(), NativeGisErrorCode::DemReprojectionFailed);
        assert!(!mixed_merged.exists());
        let _ = fs::remove_dir_all(root);
    }

    fn utm_north_wkt(zone: u32) -> String {
        let central_meridian = -183 + 6 * (zone as i32);
        let epsg = 32600 + zone;
        format!(
            r#"PROJCS["WGS 84 / UTM zone {zone}N",GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433]],PROJECTION["Transverse_Mercator"],PARAMETER["latitude_of_origin",0],PARAMETER["central_meridian",{central_meridian}],PARAMETER["scale_factor",0.9996],PARAMETER["false_easting",500000],PARAMETER["false_northing",0],UNIT["metre",1],AUTHORITY["EPSG","{epsg}"]] "#
        )
        .trim_end()
        .to_owned()
    }

    unsafe fn project_lonlat_to_utm(
        runtime: &NativeGisRuntime,
        zone: u32,
        lon: f64,
        lat: f64,
    ) -> (f64, f64) {
        let proj_ctx = runtime.create_proj_context().expect("PROJ context");
        let create_crs_to_crs = *runtime
            .proj_symbol::<ProjCreateCrsToCrs>(b"proj_create_crs_to_crs\0")
            .expect("proj_create_crs_to_crs");
        let normalize = *runtime
            .proj_symbol::<ProjNormalizeForVisualization>(b"proj_normalize_for_visualization\0")
            .expect("proj_normalize_for_visualization");
        let proj_trans = *runtime
            .proj_symbol::<ProjTrans>(b"proj_trans\0")
            .expect("proj_trans");
        let proj_destroy = *runtime
            .proj_symbol::<ProjDestroy>(b"proj_destroy\0")
            .expect("proj_destroy");
        let src = CString::new("EPSG:4326").unwrap();
        let dst = CString::new(format!("EPSG:{}", 32600 + zone)).unwrap();
        let raw = create_crs_to_crs(proj_ctx.raw(), src.as_ptr(), dst.as_ptr(), std::ptr::null());
        assert!(!raw.is_null(), "proj_create_crs_to_crs");
        let norm = normalize(proj_ctx.raw(), raw);
        proj_destroy(raw);
        assert!(!norm.is_null(), "proj_normalize_for_visualization");
        let out = proj_trans(
            norm,
            PJ_FWD,
            ProjCoord {
                x: lon,
                y: lat,
                z: 0.0,
                t: 0.0,
            },
        );
        proj_destroy(norm);
        assert!(out.x.is_finite() && out.y.is_finite());
        (out.x, out.y)
    }

    unsafe fn write_test_utm_geotiff_bands(
        runtime: &NativeGisRuntime,
        path: &Path,
        zone: u32,
        (width, height): (c_int, c_int),
        mut transform: [f64; 6],
        bands: &[Vec<f32>],
        nodata: Option<f64>,
    ) {
        runtime.ensure_gdal_registered().expect("GDALAllRegister");
        let get_driver = *runtime
            .gdal_symbol::<GdalGetDriverByName>(b"GDALGetDriverByName\0")
            .expect("GDALGetDriverByName");
        let driver_name = CString::new("GTiff").unwrap();
        let driver = get_driver(driver_name.as_ptr());
        let create = *runtime
            .gdal_symbol::<GdalCreate>(b"GDALCreate\0")
            .expect("GDALCreate");
        let close = *runtime
            .gdal_symbol::<GdalClose>(b"GDALClose\0")
            .expect("GDALClose");
        let output = c_path(
            path,
            NativeGisErrorCode::DemReprojectionFailed,
            "test UTM GeoTIFF path",
        )
        .expect("test path");
        let dataset = create(
            driver,
            output.as_ptr(),
            width,
            height,
            c_int::try_from(bands.len()).unwrap(),
            GDAL_FLOAT32,
            std::ptr::null(),
        );
        assert!(!dataset.is_null(), "create synthetic UTM GeoTIFF");
        let dataset = GdalDatasetGuard::new(dataset, close);
        let projection = CString::new(utm_north_wkt(zone)).unwrap();
        let set_projection = *runtime
            .gdal_symbol::<GdalSetProjection>(b"GDALSetProjection\0")
            .expect("GDALSetProjection");
        assert_eq!(set_projection(dataset.raw(), projection.as_ptr()), 0);
        let set_transform = *runtime
            .gdal_symbol::<GdalSetGeoTransform>(b"GDALSetGeoTransform\0")
            .expect("GDALSetGeoTransform");
        assert_eq!(set_transform(dataset.raw(), transform.as_mut_ptr()), 0);
        let get_band = *runtime
            .gdal_symbol::<GdalGetRasterBand>(b"GDALGetRasterBand\0")
            .expect("GDALGetRasterBand");
        let set_nodata = *runtime
            .gdal_symbol::<GdalSetRasterNoDataValue>(b"GDALSetRasterNoDataValue\0")
            .expect("GDALSetRasterNoDataValue");
        let raster_io = *runtime
            .gdal_symbol::<GdalRasterIo>(b"GDALRasterIO\0")
            .expect("GDALRasterIO");
        for (idx, band_values) in bands.iter().enumerate() {
            assert_eq!(band_values.len(), (width * height) as usize);
            let band = get_band(dataset.raw(), (idx + 1) as c_int);
            assert!(!band.is_null(), "synthetic UTM GeoTIFF band");
            if let Some(nodata_val) = nodata {
                assert_eq!(set_nodata(band, nodata_val), 0);
            }
            let mut copy = band_values.clone();
            assert_eq!(
                raster_io(
                    band,
                    GDAL_WRITE,
                    0,
                    0,
                    width,
                    height,
                    copy.as_mut_ptr().cast(),
                    width,
                    height,
                    GDAL_FLOAT32,
                    0,
                    0,
                ),
                0
            );
        }
        let flush = *runtime
            .gdal_symbol::<GdalFlushCache>(b"GDALFlushCache\0")
            .expect("GDALFlushCache");
        flush(dataset.raw());
    }

    #[test]
    #[ignore = "requires INSAR_GIS_RUNTIME_ROOT pointing to a packaged product runtime"]
    fn native_utm_multiband_warp_convert_and_antimeridian_merge() {
        let runtime_root = std::env::var_os("INSAR_GIS_RUNTIME_ROOT")
            .map(PathBuf::from)
            .expect("INSAR_GIS_RUNTIME_ROOT must point to runtime/gis");
        let identity = NativeGisRuntimeIdentity::from_product_root(&runtime_root)
            .expect("discover product runtime identity");
        let runtime = NativeGisRuntime::load_identity(&identity).expect("load product runtime");
        verify_runtime_capabilities(&runtime).expect("verify product runtime exports");

        let id = NEXT_NATIVE_TEST_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "insar-gis-native-utm-warp-{id}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();

        // Build two 5-band Float32 UTM tiles across the UTM zone 10N / 11N seam (-120 deg)
        // mirroring GEM-15 DSM layout (band 1 = DSM elevation with NaN nodata, bands 2..=5 ignored).
        let (z10_min_e, z10_min_n) = unsafe { project_lonlat_to_utm(&runtime, 10, -120.02, 37.98) };
        let (z10_max_e, z10_max_n) =
            unsafe { project_lonlat_to_utm(&runtime, 10, -119.998, 38.02) };
        let (z11_min_e, z11_min_n) =
            unsafe { project_lonlat_to_utm(&runtime, 11, -120.002, 37.98) };
        let (z11_max_e, z11_max_n) = unsafe { project_lonlat_to_utm(&runtime, 11, -119.98, 38.02) };

        let grid = 120i32;
        let tile_z10 = root.join("gem15-z10.tif");
        let tile_z11 = root.join("gem15-z11.tif");
        let z10_dx = (z10_max_e - z10_min_e) / f64::from(grid);
        let z10_dy = (z10_max_n - z10_min_n) / f64::from(grid);
        let mut z10_band1 = vec![120.0f32; (grid * grid) as usize];
        // Leave the northernmost 20% rows of zone 10 tile as NaN nodata.
        for r in 0..30 {
            for c in 0..grid {
                z10_band1[(r * grid + c) as usize] = f32::NAN;
            }
        }
        let z11_dx = (z11_max_e - z11_min_e) / f64::from(grid);
        let z11_dy = (z11_max_n - z11_min_n) / f64::from(grid);
        let mut z11_band1 = vec![120.0f32; (grid * grid) as usize];
        for r in 0..30 {
            for c in 0..grid {
                z11_band1[(r * grid + c) as usize] = f32::NAN;
            }
        }
        let poison_band = vec![99_999.0f32; (grid * grid) as usize];
        unsafe {
            write_test_utm_geotiff_bands(
                &runtime,
                &tile_z10,
                10,
                (grid, grid),
                [
                    z10_min_e - 200.0,
                    z10_dx,
                    0.0,
                    z10_max_n + 200.0,
                    0.0,
                    -z10_dy,
                ],
                &[
                    z10_band1,
                    poison_band.clone(),
                    poison_band.clone(),
                    poison_band.clone(),
                    poison_band.clone(),
                ],
                Some(f64::NAN),
            );
            write_test_utm_geotiff_bands(
                &runtime,
                &tile_z11,
                11,
                (grid, grid),
                [
                    z11_min_e - 200.0,
                    z11_dx,
                    0.0,
                    z11_max_n + 200.0,
                    0.0,
                    -z11_dy,
                ],
                &[
                    z11_band1,
                    poison_band.clone(),
                    poison_band.clone(),
                    poison_band.clone(),
                    poison_band,
                ],
                Some(f64::NAN),
            );
        }

        let warped = root.join("warped-gem15.tif");
        let crop_bbox = [-120.005, 37.995, -119.995, 38.015];
        let warp_meta =
            warp_mosaic_crop_utm_dem_tiles(&runtime, &[tile_z10, tile_z11], &warped, crop_bbox)
                .expect("warp mixed-UTM 5-band GEM-15 tiles");
        assert_eq!(warp_meta["width"], 72);
        assert_eq!(warp_meta["height"], 144);
        assert_eq!(warp_meta["inspection"]["metadata"]["count"], 1);
        assert_eq!(warp_meta["inspection"]["metadata"]["nodata"], -9999.0);
        let warped_values = unsafe { read_test_geotiff(&runtime, &warped) };
        assert!(warped_values.iter().all(|v| v.is_finite()));
        assert!(warped_values.iter().any(|v| (*v - -9999.0).abs() < 1.0e-3));
        assert!(warped_values.iter().any(|v| (*v - 120.0).abs() < 0.5));
        assert!(warped_values.iter().all(|v| *v < 200.0));

        let grid_path = runtime
            .root()
            .join("vertical-grids")
            .join("egm96")
            .join("us_nga_egm96_15.tif");
        assert!(
            grid_path.is_file(),
            "EGM96 grid must exist in runtime/gis/vertical-grids/egm96"
        );
        let ellipsoid = root.join("ellipsoid-gem15.tif");
        let sarscape = root.join("sarscape-gem15");
        let convert_meta = convert_dem(
            &runtime,
            Some(&grid_path),
            &warped,
            &ellipsoid,
            Some(&sarscape),
            "EGM96",
        )
        .expect("convert warped GEM-15 EGM96 output to WGS84 ellipsoid and SARscape");
        assert_eq!(convert_meta["executed"], true);
        let ellipsoid_values = unsafe { read_test_geotiff(&runtime, &ellipsoid) };
        assert!(ellipsoid_values
            .iter()
            .any(|v| (*v - -9999.0).abs() < 1.0e-3));
        assert!(ellipsoid_values
            .iter()
            .any(|v| v.is_finite() && (*v - -9999.0).abs() > 1.0 && (*v - 120.0).abs() > 5.0));
        assert!(sarscape.is_file());
        assert!(
            sidecar_path(&sarscape, ".hdr").unwrap().is_file()
                || sarscape.with_extension("hdr").is_file()
        );
        assert!(sidecar_path(&sarscape, ".sml").unwrap().is_file());

        // Antimeridian split windows using UTM zones 60N and 1N must align cleanly for merge_antimeridian_dem.
        let (w_min_e, w_min_n) = unsafe { project_lonlat_to_utm(&runtime, 60, 179.97, 9.99) };
        let (w_max_e, w_max_n) = unsafe { project_lonlat_to_utm(&runtime, 60, 180.00, 10.02) };
        let (e_min_e, e_min_n) = unsafe { project_lonlat_to_utm(&runtime, 1, -180.00, 9.99) };
        let (e_max_e, e_max_n) = unsafe { project_lonlat_to_utm(&runtime, 1, -179.97, 10.02) };
        let am_w_tile = root.join("am-z60.tif");
        let am_e_tile = root.join("am-z01.tif");
        unsafe {
            write_test_utm_geotiff_bands(
                &runtime,
                &am_w_tile,
                60,
                (60, 60),
                [
                    w_min_e - 200.0,
                    (w_max_e - w_min_e + 400.0) / 60.0,
                    0.0,
                    w_max_n + 200.0,
                    0.0,
                    -(w_max_n - w_min_n + 400.0) / 60.0,
                ],
                &[vec![42.0f32; 3600]],
                Some(f64::NAN),
            );
            write_test_utm_geotiff_bands(
                &runtime,
                &am_e_tile,
                1,
                (60, 60),
                [
                    e_min_e - 200.0,
                    (e_max_e - e_min_e + 400.0) / 60.0,
                    0.0,
                    e_max_n + 200.0,
                    0.0,
                    -(e_max_n - e_min_n + 400.0) / 60.0,
                ],
                &[vec![43.0f32; 3600]],
                Some(f64::NAN),
            );
        }
        let am_w_warped = root.join("am-west.tif");
        let am_e_warped = root.join("am-east.tif");
        warp_mosaic_crop_utm_dem_tiles(
            &runtime,
            &[am_w_tile],
            &am_w_warped,
            [179.99, 10.00, 180.00, 10.01],
        )
        .expect("warp antimeridian west UTM window");
        warp_mosaic_crop_utm_dem_tiles(
            &runtime,
            &[am_e_tile],
            &am_e_warped,
            [-180.00, 10.00, -179.99, 10.01],
        )
        .expect("warp antimeridian east UTM window");
        let am_merged = root.join("am-merged.tif");
        let am_meta = merge_antimeridian_dem(&runtime, &am_w_warped, &am_e_warped, &am_merged)
            .expect("antimeridian merge across warped GEM-15 UTM outputs");
        assert_eq!(am_meta["west_width"], 72);
        assert_eq!(am_meta["east_width"], 72);
        assert_eq!(
            inspect_dem(&runtime, &am_merged).unwrap()["metadata"]["width"],
            144
        );

        let _ = fs::remove_dir_all(root);
    }
}

#[cfg(test)]
#[path = "sarscape_delivery_tests.rs"]
mod sarscape_delivery_tests;
