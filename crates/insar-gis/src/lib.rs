//! Bounded, product-owned GIS execution boundary.
//!
//! This crate deliberately does not own Store, Scheduler, credentials,
//! providers, or task policy.  It owns only the format/runtime contracts used
//! by `ApplicationService`.  The initial implementation is introduced behind
//! failing semantic tests so the native cutover cannot silently fall back to
//! the Python compatibility Worker.

use serde_json::Value;
use std::fmt;
use std::path::Path;
use std::sync::{Arc, OnceLock};
use thiserror::Error;

pub mod lbs;
pub mod mapgis;
pub mod optical_clip;
mod raster;
mod runtime;
pub mod stream_ops;
mod vector;
pub mod vector_ops;
mod vertical;

pub use lbs::{transform_coord_pair, transform_lbs_coordinates, LbsCoordSys};
pub use mapgis::{
    build_synthetic_mapgis_binary_wl, convert_mapgis_file, inspect_mapgis_file, MapGisShapeKind,
};
pub use optical_clip::{
    precheck_aoi_scene_coverage, redact_source_locator, validate_raw_optical_cog_source,
    OpticalBandAssetInput, OpticalCloudMaskPolicy, OpticalDerivedMode, OpticalDerivedProgress,
    OpticalDerivedRequest, OpticalDerivedResult, OpticalMosaicStrategy, OpticalPrecheckReport,
    OpticalPrecheckRequest, OpticalResampling, OpticalSceneCoverageReport,
    OpticalSceneFailureRecord, OpticalSceneInput, OpticalSourceKind, TestGeoTiffReadU16,
    OPTICAL_MOSAIC_MAX_SCENES,
};
pub use stream_ops::{normalize_gdal_source_string, RasterPixelProbeRequest, RasterProcessRequest};
pub use vector_ops::VectorProcessRequest;

pub use runtime::{
    measure_directory_usage, NativeGisRuntimeIdentity, NativeGisRuntimeManifest,
    RuntimeFileManifest, SubprocessScratchBudget, SubprocessScratchOutcome, SUPPORTED_GIS_DRIVERS,
};
pub use vector::import_vector;
pub use vertical::{VerticalGridManifest, VerticalGridReadiness, VerticalGridRegistry};

pub const NATIVE_GIS_CONTRACT_VERSION: &str = "native_gis_v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeGisErrorCode {
    GisEngineUnavailable,
    GisDriverUnavailable,
    AoiFormatUnsupported,
    AoiGeometryInvalid,
    DemInvalidRaster,
    DemCrsMissing,
    DemReprojectionFailed,
    VerticalGridMissing,
    VerticalGridIdentityMismatch,
    VerticalGridNotProductionAdmitted,
    VerticalTransformFailed,
}

impl NativeGisErrorCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::GisEngineUnavailable => "GIS_ENGINE_UNAVAILABLE",
            Self::GisDriverUnavailable => "GIS_DRIVER_UNAVAILABLE",
            Self::AoiFormatUnsupported => "AOI_FORMAT_UNSUPPORTED",
            Self::AoiGeometryInvalid => "AOI_GEOMETRY_INVALID",
            Self::DemInvalidRaster => "DEM_INVALID_RASTER",
            Self::DemCrsMissing => "DEM_CRS_MISSING",
            Self::DemReprojectionFailed => "DEM_REPROJECTION_FAILED",
            Self::VerticalGridMissing => "VERTICAL_GRID_MISSING",
            Self::VerticalGridIdentityMismatch => "VERTICAL_GRID_IDENTITY_MISMATCH",
            Self::VerticalGridNotProductionAdmitted => "VERTICAL_GRID_NOT_PRODUCTION_ADMITTED",
            Self::VerticalTransformFailed => "VERTICAL_TRANSFORM_FAILED",
        }
    }
}

impl fmt::Display for NativeGisErrorCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, Error)]
pub enum NativeGisError {
    #[error("{code}: {reason}")]
    Contract {
        code: NativeGisErrorCode,
        reason: String,
    },
}

impl NativeGisError {
    pub fn code(&self) -> NativeGisErrorCode {
        match self {
            Self::Contract { code, .. } => *code,
        }
    }

    pub(crate) fn unavailable(reason: impl Into<String>) -> Self {
        Self::Contract {
            code: NativeGisErrorCode::GisEngineUnavailable,
            reason: reason.into(),
        }
    }

    pub(crate) fn contract(code: NativeGisErrorCode, reason: impl Into<String>) -> Self {
        Self::Contract {
            code,
            reason: reason.into(),
        }
    }
}

/// Runtime-neutral native GIS facade.  The production constructor will bind
/// only to the product-owned `runtime/gis` directory; it never searches PATH
/// and never starts Python.
#[derive(Clone, Default)]
pub struct NativeGisEngine {
    runtime_root: Option<std::path::PathBuf>,
    runtime: Option<std::sync::Arc<runtime::NativeGisRuntime>>,
    vertical_grids: VerticalGridRegistry,
}

impl NativeGisEngine {
    pub fn unavailable() -> Self {
        Self::default()
    }

    pub fn from_product_root(root: impl AsRef<Path>) -> Result<Self, NativeGisError> {
        let root = root.as_ref().to_path_buf();
        let vertical_grid_root = vertical_grid_root_for_runtime(&root);
        Self::from_product_roots(root, vertical_grid_root)
    }

    pub fn from_product_roots(
        runtime_root: impl AsRef<Path>,
        vertical_grid_root: impl AsRef<Path>,
    ) -> Result<Self, NativeGisError> {
        let identity = NativeGisRuntimeIdentity::from_product_root(runtime_root)?;
        Self::from_product_identity_and_grid_root(&identity, vertical_grid_root)
    }

    pub fn from_product_identity(
        identity: &NativeGisRuntimeIdentity,
    ) -> Result<Self, NativeGisError> {
        Self::from_product_identity_and_grid_root(identity, identity.root().join("vertical-grids"))
    }

    pub fn from_product_identity_and_grid_root(
        identity: &NativeGisRuntimeIdentity,
        vertical_grid_root: impl AsRef<Path>,
    ) -> Result<Self, NativeGisError> {
        let root = identity.root().to_path_buf();
        let native_runtime = runtime::NativeGisRuntime::load_identity(identity)?;
        raster::verify_runtime_capabilities(&native_runtime)?;
        let vertical_grids = vertical::VerticalGridRegistry::load(vertical_grid_root)?;
        Ok(Self {
            runtime_root: Some(root),
            runtime: Some(std::sync::Arc::new(native_runtime)),
            vertical_grids,
        })
    }

    /// Discover only the package-owned runtime next to the executable.  No
    /// PATH/system Python/GDAL probing is permitted here; absence is represented
    /// by an unavailable engine so callers can return a stable typed error.
    pub fn discover_product() -> Self {
        let Some((runtime_root, vertical_grid_root)) = discovered_product_roots() else {
            return Self::unavailable();
        };
        Self::from_product_roots(runtime_root, vertical_grid_root)
            .unwrap_or_else(|_| Self::unavailable())
    }

    pub fn runtime_root(&self) -> Option<&Path> {
        self.runtime_root.as_deref()
    }

    pub fn is_available(&self) -> bool {
        self.runtime.is_some()
    }

    pub fn runtime_manifest(&self) -> Option<&NativeGisRuntimeManifest> {
        self.runtime
            .as_deref()
            .map(runtime::NativeGisRuntime::manifest)
    }

    pub fn vertical_grids(&self) -> &VerticalGridRegistry {
        &self.vertical_grids
    }

    pub fn inspect_dem(&self, path: impl AsRef<Path>) -> Result<Value, NativeGisError> {
        let runtime = self.runtime.as_deref().ok_or_else(|| {
            NativeGisError::unavailable("product-owned native GIS runtime is unavailable")
        })?;
        raster::inspect_dem(runtime, path.as_ref())
    }

    /// Read every source pixel in bounded native-type windows without writing
    /// or resampling. Metadata readability alone cannot detect a truncated body.
    pub fn verify_dem_pixels(&self, path: impl AsRef<Path>) -> Result<(), NativeGisError> {
        self.inspect_dem(path.as_ref())?;
        let runtime = self.runtime.as_deref().ok_or_else(|| {
            NativeGisError::unavailable("product-owned native GIS runtime is unavailable")
        })?;
        raster::verify_dem_pixels(runtime, path.as_ref())
    }

    pub fn convert_dem(
        &self,
        input: impl AsRef<Path>,
        ellipsoid_output: impl AsRef<Path>,
        sarscape_output: Option<&Path>,
        source_model: &str,
    ) -> Result<Value, NativeGisError> {
        let runtime = self.runtime.as_deref().ok_or_else(|| {
            NativeGisError::unavailable("product-owned native GIS runtime is unavailable")
        })?;
        let source_model = source_model.trim().to_ascii_uppercase();
        let grid_path = if source_model == "WGS84_ELLIPSOID" {
            None
        } else {
            Some(self.vertical_grids.required_path(&source_model)?)
        };
        raster::convert_dem(
            runtime,
            grid_path.as_deref(),
            input.as_ref(),
            ellipsoid_output.as_ref(),
            sarscape_output,
            &source_model,
        )
    }

    /// Crop a bounded target window and resample it through the product-owned
    /// GDAL/PROJ runtime.  This is deliberately an explicit native operation:
    /// the caller supplies the target CRS, target bounds, output dimensions,
    /// and resampling policy; no CRS or pixel grid is guessed from ambient
    /// process state.
    #[allow(clippy::too_many_arguments)]
    pub fn crop_reproject_resample_dem(
        &self,
        input: impl AsRef<Path>,
        output: impl AsRef<Path>,
        target_crs: &str,
        crop_bbox: [f64; 4],
        width: u32,
        height: u32,
        resampling: &str,
    ) -> Result<Value, NativeGisError> {
        let runtime = self.runtime.as_deref().ok_or_else(|| {
            NativeGisError::unavailable("product-owned native GIS runtime is unavailable")
        })?;
        raster::crop_reproject_resample_dem(
            runtime,
            input.as_ref(),
            output.as_ref(),
            target_crs,
            crop_bbox,
            width,
            height,
            resampling,
        )
    }

    pub fn mosaic_crop_dem_tiles(
        &self,
        inputs: &[std::path::PathBuf],
        output: impl AsRef<Path>,
        crop_bbox: [f64; 4],
    ) -> Result<Value, NativeGisError> {
        let runtime = self.runtime.as_deref().ok_or_else(|| {
            NativeGisError::unavailable("product-owned native GIS runtime is unavailable")
        })?;
        raster::mosaic_crop_dem_tiles(runtime, inputs, output.as_ref(), crop_bbox)
    }

    pub fn warp_mosaic_crop_utm_dem_tiles(
        &self,
        inputs: &[std::path::PathBuf],
        output: impl AsRef<Path>,
        crop_bbox: [f64; 4],
    ) -> Result<Value, NativeGisError> {
        let runtime = self.runtime.as_deref().ok_or_else(|| {
            NativeGisError::unavailable("product-owned native GIS runtime is unavailable")
        })?;
        raster::warp_mosaic_crop_utm_dem_tiles(runtime, inputs, output.as_ref(), crop_bbox)
    }

    pub fn merge_antimeridian_dem(
        &self,
        west_input: impl AsRef<Path>,
        east_input: impl AsRef<Path>,
        output: impl AsRef<Path>,
    ) -> Result<Value, NativeGisError> {
        let runtime = self.runtime.as_deref().ok_or_else(|| {
            NativeGisError::unavailable("product-owned native GIS runtime is unavailable")
        })?;
        raster::merge_antimeridian_dem(
            runtime,
            west_input.as_ref(),
            east_input.as_ref(),
            output.as_ref(),
        )
    }

    pub fn import_vector(
        &self,
        file_name: &str,
        bytes: &[u8],
        bundle: &[(String, Vec<u8>)],
    ) -> Result<Value, NativeGisError> {
        vector::import_vector_with_runtime(self.runtime.as_deref(), file_name, bytes, bundle)
    }

    pub fn inspect_raster_source(&self, source: &str) -> Result<Value, NativeGisError> {
        let runtime = self.runtime.as_deref().ok_or_else(|| {
            NativeGisError::unavailable("product-owned native GIS runtime is unavailable")
        })?;
        stream_ops::inspect_raster_source(runtime, source)
    }

    pub fn probe_raster_pixel(
        &self,
        request: &RasterPixelProbeRequest,
    ) -> Result<Value, NativeGisError> {
        let runtime = self.runtime.as_deref().ok_or_else(|| {
            NativeGisError::unavailable("product-owned native GIS runtime is unavailable")
        })?;
        stream_ops::probe_raster_pixel(runtime, request)
    }

    pub fn process_raster_window(
        &self,
        request: &RasterProcessRequest,
    ) -> Result<Value, NativeGisError> {
        let runtime = self.runtime.as_deref().ok_or_else(|| {
            NativeGisError::unavailable("product-owned native GIS runtime is unavailable")
        })?;
        stream_ops::process_raster_window(runtime, request)
    }

    pub fn process_vector(&self, request: &VectorProcessRequest) -> Result<Value, NativeGisError> {
        let runtime = self.runtime.as_deref().ok_or_else(|| {
            NativeGisError::unavailable("product-owned native GIS runtime is unavailable")
        })?;
        vector_ops::process_vector(runtime, request)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn stitch_xyz_tiles_to_geotiff(
        &self,
        tiles: &[(u32, u32, std::path::PathBuf)],
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
        let runtime = self.runtime.as_deref().ok_or_else(|| {
            NativeGisError::unavailable("product-owned native GIS runtime is unavailable")
        })?;
        stream_ops::stitch_xyz_tiles_to_geotiff(
            runtime,
            tiles,
            x_min,
            x_max,
            y_min,
            y_max,
            geotransform,
            projection_wkt,
            output_path,
            compression,
            build_pyramids,
            export_format,
        )
    }

    pub fn crop_and_upsample_subtile(
        &self,
        tile_path: &Path,
        sub_ix: u32,
        sub_iy: u32,
        shift: u32,
    ) -> Result<bool, NativeGisError> {
        let runtime = self.runtime.as_deref().ok_or_else(|| {
            NativeGisError::unavailable("product-owned native GIS runtime is unavailable")
        })?;
        stream_ops::crop_and_upsample_subtile(runtime, tile_path, sub_ix, sub_iy, shift)
    }

    #[doc(hidden)]
    pub fn write_test_geotiff_u16(
        &self,
        path: &Path,
        epsg: u32,
        geotransform: [f64; 6],
        size: [i32; 2],
        nodata: Option<f64>,
        data: &[u16],
    ) -> Result<(), NativeGisError> {
        let runtime = self.runtime.as_deref().ok_or_else(|| {
            NativeGisError::unavailable("product-owned native GIS runtime is unavailable")
        })?;
        let raw_bytes: Vec<u8> = data.iter().flat_map(|v| v.to_ne_bytes()).collect();
        optical_clip::write_test_geotiff_bytes_with_runtime(
            runtime,
            path,
            epsg,
            geotransform,
            size,
            (2, nodata),
            &raw_bytes,
        )
    }

    #[doc(hidden)]
    pub fn write_test_geotiff_u8(
        &self,
        path: &Path,
        epsg: u32,
        geotransform: [f64; 6],
        size: [i32; 2],
        nodata: Option<f64>,
        data: &[u8],
    ) -> Result<(), NativeGisError> {
        let runtime = self.runtime.as_deref().ok_or_else(|| {
            NativeGisError::unavailable("product-owned native GIS runtime is unavailable")
        })?;
        optical_clip::write_test_geotiff_u8_with_runtime(
            runtime,
            path,
            epsg,
            geotransform,
            size,
            nodata,
            data,
        )
    }

    #[doc(hidden)]
    pub fn read_test_geotiff_u16(
        &self,
        path: &Path,
        band_index: i32,
    ) -> Result<TestGeoTiffReadU16, NativeGisError> {
        let runtime = self.runtime.as_deref().ok_or_else(|| {
            NativeGisError::unavailable("product-owned native GIS runtime is unavailable")
        })?;
        optical_clip::read_test_geotiff_u16_with_runtime(runtime, path, band_index)
    }

    pub fn render_raster_preview_png(
        &self,
        raster_path: &Path,
        max_dim: u32,
    ) -> Result<(Vec<u8>, [f64; 4]), NativeGisError> {
        let runtime = self.runtime.as_deref().ok_or_else(|| {
            NativeGisError::unavailable("product-owned native GIS runtime is unavailable")
        })?;
        stream_ops::render_raster_preview_png(runtime, raster_path, max_dim)
    }

    pub fn precheck_optical_aoi_coverage(
        &self,
        request: &OpticalPrecheckRequest,
    ) -> Result<OpticalPrecheckReport, NativeGisError> {
        optical_clip::precheck_aoi_scene_coverage(request)
    }

    pub fn execute_optical_derived(
        &self,
        request: &OpticalDerivedRequest,
        interrupted: &(dyn Fn() -> bool + Sync),
        progress: &mut dyn FnMut(OpticalDerivedProgress),
    ) -> Result<OpticalDerivedResult, NativeGisError> {
        let runtime = self.runtime.as_deref().ok_or_else(|| {
            NativeGisError::unavailable("product-owned native GIS runtime is unavailable")
        })?;
        optical_clip::execute_optical_derived(runtime, request, interrupted, progress)
    }

    #[allow(dead_code)]
    fn _import_vector_marker(&self) {}
}

/// Thread-safe lazy owner for the product GIS engine. Discovery reads only the
/// small immutable manifest. The exact native closure and requested vertical
/// grid are verified only when a GIS/DEM operation first needs them.
#[derive(Clone)]
pub struct LazyNativeGisEngine {
    identity: Option<NativeGisRuntimeIdentity>,
    vertical_grid_root: Option<std::path::PathBuf>,
    initialized: Arc<OnceLock<Result<NativeGisEngine, NativeGisError>>>,
}

impl LazyNativeGisEngine {
    pub fn unavailable() -> Self {
        Self {
            identity: None,
            vertical_grid_root: None,
            initialized: Arc::new(OnceLock::new()),
        }
    }

    pub fn from_product_root(root: impl AsRef<Path>) -> Result<Self, NativeGisError> {
        let root = root.as_ref().to_path_buf();
        let vertical_grid_root = vertical_grid_root_for_runtime(&root);
        Self::from_product_roots(root, vertical_grid_root)
    }

    pub fn from_product_roots(
        runtime_root: impl AsRef<Path>,
        vertical_grid_root: impl AsRef<Path>,
    ) -> Result<Self, NativeGisError> {
        Ok(Self {
            identity: Some(NativeGisRuntimeIdentity::from_product_root(runtime_root)?),
            vertical_grid_root: Some(vertical_grid_root.as_ref().to_path_buf()),
            initialized: Arc::new(OnceLock::new()),
        })
    }

    pub fn discover_product() -> Self {
        let Some((runtime_root, vertical_grid_root)) = discovered_product_roots() else {
            return Self::unavailable();
        };
        Self::from_product_roots(runtime_root, vertical_grid_root)
            .unwrap_or_else(|_| Self::unavailable())
    }

    pub fn is_discovered(&self) -> bool {
        self.identity.is_some()
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized.get().is_some()
    }

    pub fn runtime_identity(&self) -> Option<&NativeGisRuntimeIdentity> {
        self.identity.as_ref()
    }

    fn engine(&self) -> Result<&NativeGisEngine, NativeGisError> {
        let initialized = self.initialized.get_or_init(|| {
            let identity = self.identity.as_ref().ok_or_else(|| {
                NativeGisError::unavailable("product-owned native GIS runtime is unavailable")
            })?;
            let vertical_grid_root = self.vertical_grid_root.as_ref().ok_or_else(|| {
                NativeGisError::unavailable("product-owned vertical-grid root is unavailable")
            })?;
            NativeGisEngine::from_product_identity_and_grid_root(identity, vertical_grid_root)
        });
        initialized.as_ref().map_err(Clone::clone)
    }

    pub fn inspect_dem(&self, path: impl AsRef<Path>) -> Result<Value, NativeGisError> {
        self.engine()?.inspect_dem(path)
    }

    pub fn verify_dem_pixels(&self, path: impl AsRef<Path>) -> Result<(), NativeGisError> {
        self.engine()?.verify_dem_pixels(path)
    }

    pub fn convert_dem(
        &self,
        input: impl AsRef<Path>,
        ellipsoid_output: impl AsRef<Path>,
        sarscape_output: Option<&Path>,
        source_model: &str,
    ) -> Result<Value, NativeGisError> {
        self.engine()?
            .convert_dem(input, ellipsoid_output, sarscape_output, source_model)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn crop_reproject_resample_dem(
        &self,
        input: impl AsRef<Path>,
        output: impl AsRef<Path>,
        target_crs: &str,
        crop_bbox: [f64; 4],
        width: u32,
        height: u32,
        resampling: &str,
    ) -> Result<Value, NativeGisError> {
        self.engine()?.crop_reproject_resample_dem(
            input, output, target_crs, crop_bbox, width, height, resampling,
        )
    }

    pub fn mosaic_crop_dem_tiles(
        &self,
        inputs: &[std::path::PathBuf],
        output: impl AsRef<Path>,
        crop_bbox: [f64; 4],
    ) -> Result<Value, NativeGisError> {
        self.engine()?
            .mosaic_crop_dem_tiles(inputs, output, crop_bbox)
    }

    pub fn warp_mosaic_crop_utm_dem_tiles(
        &self,
        inputs: &[std::path::PathBuf],
        output: impl AsRef<Path>,
        crop_bbox: [f64; 4],
    ) -> Result<Value, NativeGisError> {
        self.engine()?
            .warp_mosaic_crop_utm_dem_tiles(inputs, output, crop_bbox)
    }

    pub fn merge_antimeridian_dem(
        &self,
        west_input: impl AsRef<Path>,
        east_input: impl AsRef<Path>,
        output: impl AsRef<Path>,
    ) -> Result<Value, NativeGisError> {
        self.engine()?
            .merge_antimeridian_dem(west_input, east_input, output)
    }

    pub fn inspect_raster_source(&self, source: &str) -> Result<Value, NativeGisError> {
        self.engine()?.inspect_raster_source(source)
    }

    pub fn probe_raster_pixel(
        &self,
        request: &RasterPixelProbeRequest,
    ) -> Result<Value, NativeGisError> {
        self.engine()?.probe_raster_pixel(request)
    }

    pub fn process_raster_window(
        &self,
        request: &RasterProcessRequest,
    ) -> Result<Value, NativeGisError> {
        self.engine()?.process_raster_window(request)
    }

    pub fn process_vector(&self, request: &VectorProcessRequest) -> Result<Value, NativeGisError> {
        self.engine()?.process_vector(request)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn stitch_xyz_tiles_to_geotiff(
        &self,
        tiles: &[(u32, u32, std::path::PathBuf)],
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
        self.engine()?.stitch_xyz_tiles_to_geotiff(
            tiles,
            x_min,
            x_max,
            y_min,
            y_max,
            geotransform,
            projection_wkt,
            output_path,
            compression,
            build_pyramids,
            export_format,
        )
    }

    pub fn crop_and_upsample_subtile(
        &self,
        tile_path: &Path,
        sub_ix: u32,
        sub_iy: u32,
        shift: u32,
    ) -> Result<bool, NativeGisError> {
        self.engine()?
            .crop_and_upsample_subtile(tile_path, sub_ix, sub_iy, shift)
    }

    pub fn render_raster_preview_png(
        &self,
        raster_path: &Path,
        max_dim: u32,
    ) -> Result<(Vec<u8>, [f64; 4]), NativeGisError> {
        self.engine()?
            .render_raster_preview_png(raster_path, max_dim)
    }

    pub fn precheck_optical_aoi_coverage(
        &self,
        request: &OpticalPrecheckRequest,
    ) -> Result<OpticalPrecheckReport, NativeGisError> {
        optical_clip::precheck_aoi_scene_coverage(request)
    }

    pub fn execute_optical_derived(
        &self,
        request: &OpticalDerivedRequest,
        interrupted: &(dyn Fn() -> bool + Sync),
        progress: &mut dyn FnMut(OpticalDerivedProgress),
    ) -> Result<OpticalDerivedResult, NativeGisError> {
        self.engine()?
            .execute_optical_derived(request, interrupted, progress)
    }

    pub fn import_vector(
        &self,
        file_name: &str,
        bytes: &[u8],
        bundle: &[(String, Vec<u8>)],
    ) -> Result<Value, NativeGisError> {
        self.engine()?.import_vector(file_name, bytes, bundle)
    }

    #[doc(hidden)]
    pub fn write_test_geotiff_u16(
        &self,
        path: &Path,
        epsg: u32,
        geotransform: [f64; 6],
        size: [i32; 2],
        nodata: Option<f64>,
        data: &[u16],
    ) -> Result<(), NativeGisError> {
        self.engine()?
            .write_test_geotiff_u16(path, epsg, geotransform, size, nodata, data)
    }

    #[doc(hidden)]
    pub fn write_test_geotiff_u8(
        &self,
        path: &Path,
        epsg: u32,
        geotransform: [f64; 6],
        size: [i32; 2],
        nodata: Option<f64>,
        data: &[u8],
    ) -> Result<(), NativeGisError> {
        self.engine()?
            .write_test_geotiff_u8(path, epsg, geotransform, size, nodata, data)
    }

    #[doc(hidden)]
    pub fn read_test_geotiff_u16(
        &self,
        path: &Path,
        band_index: i32,
    ) -> Result<TestGeoTiffReadU16, NativeGisError> {
        self.engine()?.read_test_geotiff_u16(path, band_index)
    }

    #[allow(dead_code)]
    fn _merge_marker(&self) {}
}

impl Default for LazyNativeGisEngine {
    fn default() -> Self {
        Self::unavailable()
    }
}

fn discovered_product_roots() -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    let executable_path = std::env::current_exe().ok()?;
    discovered_product_roots_for_executable(&executable_path)
}

fn discovered_product_roots_for_executable(
    executable_path: &Path,
) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    let product_root = executable_path.parent()?;
    let direct = product_root.join("runtime/gis");
    let runtime_root = if direct.join("manifest.json").is_file() {
        direct
    } else if product_root.file_name() == Some(std::ffi::OsStr::new("MacOS")) {
        if let Some(contents) = product_root.parent() {
            let bundled = contents.join("Resources/runtime/gis");
            if bundled.join("manifest.json").is_file() {
                bundled
            } else {
                direct
            }
        } else {
            direct
        }
    } else {
        direct
    };
    let vertical_grid_root = vertical_grid_root_for_runtime(&runtime_root);
    Some((runtime_root, vertical_grid_root))
}

fn vertical_grid_root_for_runtime(runtime_root: &Path) -> std::path::PathBuf {
    let separated_grid_root = runtime_root
        .parent()
        .map(|runtime_parent| runtime_parent.join("vertical-grids"));
    match separated_grid_root {
        Some(root) if root.join("manifest.json").is_file() => root,
        _ => runtime_root.join("vertical-grids"),
    }
}

#[cfg(test)]
mod tests {
    use super::raster;
    use super::{import_vector, LazyNativeGisEngine, NativeGisEngine, NativeGisErrorCode};
    use serde_json::json;
    use std::fs;

    fn sample_shp() -> Vec<u8> {
        let points: [(f64, f64); 4] = [(100.0, 30.0), (101.0, 30.0), (101.0, 31.0), (100.0, 30.0)];
        let mut content = Vec::new();
        content.extend_from_slice(&5i32.to_le_bytes());
        for value in [100.0f64, 30.0, 101.0, 31.0] {
            content.extend_from_slice(&value.to_le_bytes());
        }
        content.extend_from_slice(&1i32.to_le_bytes());
        content.extend_from_slice(&(points.len() as i32).to_le_bytes());
        content.extend_from_slice(&0i32.to_le_bytes());
        for (x, y) in points {
            content.extend_from_slice(&x.to_le_bytes());
            content.extend_from_slice(&y.to_le_bytes());
        }
        let total_bytes = 100 + 8 + content.len();
        let mut shp = vec![0u8; 100];
        shp[0..4].copy_from_slice(&9994i32.to_be_bytes());
        shp[24..28].copy_from_slice(&((total_bytes / 2) as i32).to_be_bytes());
        shp[32..36].copy_from_slice(&5i32.to_le_bytes());
        shp[36..44].copy_from_slice(&100.0f64.to_le_bytes());
        shp[44..52].copy_from_slice(&30.0f64.to_le_bytes());
        shp[52..60].copy_from_slice(&101.0f64.to_le_bytes());
        shp[60..68].copy_from_slice(&31.0f64.to_le_bytes());
        shp.extend_from_slice(&1i32.to_be_bytes());
        shp.extend_from_slice(&((content.len() / 2) as i32).to_be_bytes());
        shp.extend_from_slice(&content);
        shp
    }

    fn sample_dbf() -> Vec<u8> {
        let mut dbf = vec![0u8; 65 + 11 + 1];
        dbf[0] = 3;
        dbf[4..8].copy_from_slice(&1u32.to_le_bytes());
        dbf[8..10].copy_from_slice(&65u16.to_le_bytes());
        dbf[10..12].copy_from_slice(&11u16.to_le_bytes());
        dbf[32..36].copy_from_slice(b"name");
        dbf[43] = b'C';
        dbf[48] = 10;
        dbf[64] = 0x0d;
        dbf[65] = b' ';
        dbf[66..70].copy_from_slice(b"Area");
        dbf[76] = 0x1a;
        dbf
    }

    fn stored_zip(name: &[u8], payload: &[u8]) -> Vec<u8> {
        stored_zip_entries(&[(name, payload)])
    }

    fn stored_zip_entries(entries: &[(&[u8], &[u8])]) -> Vec<u8> {
        let mut zip = Vec::new();
        let mut central_records: Vec<(u32, &[u8], &[u8])> = Vec::with_capacity(entries.len());
        for (name, payload) in entries {
            let local_offset = zip.len() as u32;
            zip.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
            zip.extend_from_slice(&20u16.to_le_bytes());
            zip.extend_from_slice(&0u16.to_le_bytes());
            zip.extend_from_slice(&0u16.to_le_bytes());
            zip.extend_from_slice(&0u16.to_le_bytes());
            zip.extend_from_slice(&0u16.to_le_bytes());
            zip.extend_from_slice(&0u32.to_le_bytes());
            zip.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            zip.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            zip.extend_from_slice(&(name.len() as u16).to_le_bytes());
            zip.extend_from_slice(&0u16.to_le_bytes());
            zip.extend_from_slice(name);
            zip.extend_from_slice(payload);
            central_records.push((local_offset, name, payload));
        }
        let central_offset = zip.len() as u32;
        for (local_offset, name, payload) in central_records {
            zip.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            zip.extend_from_slice(&20u16.to_le_bytes());
            zip.extend_from_slice(&20u16.to_le_bytes());
            zip.extend_from_slice(&0u16.to_le_bytes());
            zip.extend_from_slice(&0u16.to_le_bytes());
            zip.extend_from_slice(&0u16.to_le_bytes());
            zip.extend_from_slice(&0u16.to_le_bytes());
            zip.extend_from_slice(&0u32.to_le_bytes());
            zip.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            zip.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            zip.extend_from_slice(&(name.len() as u16).to_le_bytes());
            zip.extend_from_slice(&0u16.to_le_bytes());
            zip.extend_from_slice(&0u16.to_le_bytes());
            zip.extend_from_slice(&0u16.to_le_bytes());
            zip.extend_from_slice(&0u16.to_le_bytes());
            zip.extend_from_slice(&0u32.to_le_bytes());
            zip.extend_from_slice(&local_offset.to_le_bytes());
            zip.extend_from_slice(name);
        }
        let count = entries.len() as u16;
        zip.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&count.to_le_bytes());
        zip.extend_from_slice(&count.to_le_bytes());
        zip.extend_from_slice(&((zip.len() as u32) - central_offset).to_le_bytes());
        zip.extend_from_slice(&central_offset.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip
    }

    #[test]
    fn native_vector_codec_handles_kml_without_python_or_worker() {
        let kml = br#"<?xml version="1.0"?><kml xmlns="http://www.opengis.net/kml/2.2"><Placemark><name>Test AOI</name><Polygon><outerBoundaryIs><LinearRing><coordinates>100,30 101,30 101,31 100,30</coordinates></LinearRing></outerBoundaryIs></Polygon></Placemark></kml>"#;
        let preview = NativeGisEngine::unavailable()
            .import_vector("boundary.kml", kml, &[])
            .expect("KML must be native and must not require the compatibility Worker");
        assert_eq!(preview["ok"], true);
        assert_eq!(preview["source_kind"], "content");
        assert_eq!(preview["total_features"], 1);
        assert_eq!(preview["display_field"], "name");
        assert_eq!(preview["features"][0]["name"], "Test AOI");
    }

    #[test]
    fn lazy_product_discovery_reads_only_the_small_manifest_until_gis_first_use() {
        let root = std::env::temp_dir().join(format!(
            "insar-studio-lazy-gis-discovery-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("runtime root");
        fs::write(
            root.join("manifest.json"),
            serde_json::to_vec(&json!({
                "schema_version": 1,
                "runtime_id": "lazy-test-runtime",
                "gdal_version": "3.10.0",
                "proj_version": "9.5.1",
                "gdal_library": "bin/gdal.dll",
                "proj_library": "bin/proj.dll",
                "proj_data_dir": "share/proj",
                "gdal_data_dir": "share/gdal",
                "proj_network": "disabled",
                "drivers": ["GTiff", "ENVI"],
                "files": [{
                    "relative_path": "vertical-grids/egm2008/missing-80mb-grid.tif",
                    "bytes": 80585622,
                    "sha256": "0000000000000000000000000000000000000000000000000000000000000000"
                }]
            }))
            .expect("manifest JSON"),
        )
        .expect("manifest file");

        let lazy = LazyNativeGisEngine::from_product_root(&root)
            .expect("cheap discovery must not require DLLs, data trees, or grids");
        assert!(lazy.is_discovered());
        assert!(!lazy.is_initialized());
        assert_eq!(
            lazy.runtime_identity()
                .map(|identity| identity.runtime_id()),
            Some("lazy-test-runtime")
        );

        let error = lazy
            .inspect_dem(root.join("missing-dem.tif"))
            .expect_err("first GIS use performs fail-closed runtime verification");
        assert_eq!(error.code(), NativeGisErrorCode::GisEngineUnavailable);
        assert!(lazy.is_initialized());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn lazy_product_runtime_accepts_a_separate_vertical_grid_root() {
        let root = std::env::temp_dir().join(format!(
            "insar-studio-split-gis-discovery-{}",
            std::process::id()
        ));
        let runtime_root = root.join("runtime/gis");
        let grid_root = root.join("runtime/vertical-grids");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&runtime_root).expect("runtime root");
        fs::create_dir_all(&grid_root).expect("grid root");
        fs::write(
            runtime_root.join("manifest.json"),
            serde_json::to_vec(&json!({
                "schema_version": 1,
                "runtime_id": "split-test-runtime",
                "gdal_version": "3.10.0",
                "proj_version": "9.5.1",
                "gdal_library": "bin/gdal.dll",
                "proj_library": "bin/proj.dll",
                "proj_data_dir": "share/proj",
                "gdal_data_dir": "share/gdal",
                "proj_network": "disabled",
                "drivers": ["GTiff", "ENVI"],
                "files": []
            }))
            .expect("manifest JSON"),
        )
        .expect("manifest file");
        fs::write(grid_root.join("manifest.json"), "[]").expect("grid manifest");

        let lazy = LazyNativeGisEngine::from_product_root(&runtime_root)
            .expect("product runtime must prefer the separated sibling vertical-grid root");
        assert!(lazy.is_discovered());
        assert_eq!(
            lazy.vertical_grid_root.as_deref(),
            Some(grid_root.as_path())
        );
        assert_eq!(
            lazy.runtime_identity()
                .map(|identity| identity.runtime_id()),
            Some("split-test-runtime")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn native_vector_codec_handles_shapefile_bundle_and_dbf_display_name() {
        let preview = import_vector(
            "boundary.shp",
            &sample_shp(),
            &[("boundary.dbf".to_owned(), sample_dbf())],
        )
        .expect("Shapefile must be native");
        assert_eq!(preview["total_features"], 1);
        assert_eq!(preview["display_field"], "name");
        assert_eq!(preview["features"][0]["name"], "Area");
        assert_eq!(
            preview["geojson"]["features"][0]["geometry"]["type"],
            "Polygon"
        );
    }

    #[test]
    fn projected_shapefile_fails_closed_without_a_product_proj_runtime() {
        let error = import_vector(
            "boundary.shp",
            &sample_shp(),
            &[(
                "boundary.prj".to_owned(),
                b"PROJCS[\"reviewed projected fixture\"]".to_vec(),
            )],
        )
        .expect_err("projected Shapefile must not be interpreted as EPSG:4326");
        assert_eq!(error.code().as_str(), "GIS_ENGINE_UNAVAILABLE");
    }

    #[test]
    fn native_vector_codec_handles_kmz_without_a_python_zip_reader() {
        let kml = br#"<kml><Placemark><Polygon><outerBoundaryIs><LinearRing><coordinates>100,30 101,30 101,31 100,30</coordinates></LinearRing></outerBoundaryIs></Polygon></Placemark></kml>"#;
        let zip = stored_zip(b"doc.kml", kml);
        let preview = import_vector("boundary.kmz", &zip, &[]).expect("KMZ must be native");
        assert_eq!(preview["total_features"], 1);
        assert_eq!(
            preview["geojson"]["features"][0]["geometry"]["type"],
            "Polygon"
        );
    }

    #[test]
    fn native_vector_codec_handles_zip_shapefile_bundle() {
        let shp = sample_shp();
        let dbf = sample_dbf();
        let zip = stored_zip_entries(&[
            (b"nested/boundary.shp", shp.as_slice()),
            (b"nested/boundary.dbf", dbf.as_slice()),
        ]);
        let preview =
            import_vector("boundary.zip", &zip, &[]).expect("ZIP Shapefile bundle must be native");
        assert_eq!(preview["total_features"], 1);
        assert_eq!(preview["display_field"], "name");
        assert_eq!(preview["features"][0]["name"], "Area");
        assert_eq!(
            preview["geojson"]["features"][0]["geometry"]["type"],
            "Polygon"
        );
    }

    #[test]
    fn native_vector_codec_rejects_non_polygon_geometry_with_stable_code() {
        let error = import_vector(
            "boundary.geojson",
            json!({"type":"Point","coordinates":[100,30]})
                .to_string()
                .as_bytes(),
            &[],
        )
        .expect_err("Point is outside the AOI contract");
        assert_eq!(error.code().as_str(), "AOI_GEOMETRY_INVALID");
    }

    #[test]
    #[ignore = "requires optional native GIS runtime and INSAR_NATIVE_GIS_* fixture inputs"]
    fn configured_product_runtime_inspects_a_bounded_dem_without_system_search() {
        let root = std::env::var("INSAR_NATIVE_GIS_RUNTIME_ROOT")
            .expect("INSAR_NATIVE_GIS_RUNTIME_ROOT must be set");
        let dem = std::env::var("INSAR_NATIVE_GIS_DEM").expect("INSAR_NATIVE_GIS_DEM must be set");
        let engine = NativeGisEngine::from_product_root(root).expect("configured GIS runtime");
        assert!(engine.is_available());
        let preview = engine.inspect_dem(dem).expect("bounded DEM inspection");
        assert_eq!(preview["ok"], true);
        assert_eq!(preview["metadata"]["driver"], "GTiff");
        let transform = preview["metadata"]["geotransform"]
            .as_array()
            .expect("native affine georeference");
        assert_eq!(transform.len(), 6);
        assert!(transform
            .iter()
            .all(|value| value.as_f64().is_some_and(f64::is_finite)));
        assert_ne!(transform[1].as_f64().unwrap(), 0.0);
        assert_ne!(transform[5].as_f64().unwrap(), 0.0);
        println!(
            "native_geotransform={}",
            preview["metadata"]["geotransform"]
        );
    }

    #[test]
    #[ignore = "requires optional native GIS runtime and INSAR_NATIVE_GIS_* fixture inputs"]
    fn configured_product_runtime_executes_a_bounded_native_dem_without_worker_fallback() {
        let root = std::env::var("INSAR_NATIVE_GIS_RUNTIME_ROOT")
            .expect("INSAR_NATIVE_GIS_RUNTIME_ROOT must be set");
        let dem = std::env::var("INSAR_NATIVE_GIS_DEM").expect("INSAR_NATIVE_GIS_DEM must be set");
        let output_root = std::env::var("INSAR_NATIVE_GIS_OUTPUT_ROOT")
            .expect("INSAR_NATIVE_GIS_OUTPUT_ROOT must be set");
        let engine = NativeGisEngine::from_product_root(root).expect("configured GIS runtime");
        let output_root = std::path::PathBuf::from(output_root);
        std::fs::create_dir_all(&output_root).expect("native DEM output directory");
        let ellipsoid = output_root.join("native_ellipsoid.tif");
        let sarscape = output_root.join("native_dem");
        let result = engine
            .convert_dem(&dem, &ellipsoid, Some(&sarscape), "EGM96")
            .expect("bounded native DEM conversion");
        assert_eq!(result["ok"], true);
        assert_eq!(result["executed"], true);
        assert!(ellipsoid.is_file());
        assert!(sarscape.is_file());
        assert!(sarscape.with_file_name("native_dem.hdr").is_file());
        assert!(sarscape.with_file_name("native_dem.sml").is_file());
        let header = std::fs::read_to_string(sarscape.with_file_name("native_dem.hdr"))
            .expect("native ENVI header");
        assert!(header.contains("data type = 4"));
        assert!(header.contains("interleave = bsq"));
    }

    #[test]
    #[ignore = "requires optional native GIS runtime and INSAR_NATIVE_GIS_* fixture inputs"]
    fn configured_product_runtime_executes_egm2008_without_egm96_fallback() {
        let root = std::env::var("INSAR_NATIVE_GIS_RUNTIME_ROOT")
            .expect("INSAR_NATIVE_GIS_RUNTIME_ROOT must be set");
        let dem = std::env::var("INSAR_NATIVE_GIS_DEM").expect("INSAR_NATIVE_GIS_DEM must be set");
        let output_root = std::env::var("INSAR_NATIVE_GIS_EGM2008_OUTPUT_ROOT")
            .expect("INSAR_NATIVE_GIS_EGM2008_OUTPUT_ROOT must be set");
        let engine = NativeGisEngine::from_product_root(root).expect("configured GIS runtime");
        let input_preview = engine
            .inspect_dem(&dem)
            .expect("native EGM2008 input inspection");
        let output_root = std::path::PathBuf::from(output_root);
        std::fs::create_dir_all(&output_root).expect("native EGM2008 output directory");
        let ellipsoid = output_root.join("native_egm2008_ellipsoid.tif");
        let result = engine
            .convert_dem(&dem, &ellipsoid, None, "EGM2008")
            .expect("bounded native EGM2008 conversion");
        assert_eq!(result["ok"], true);
        assert_eq!(result["source_vertical_datum"], "EGM2008");
        assert!(ellipsoid.is_file());
        let output_preview = engine
            .inspect_dem(&ellipsoid)
            .expect("native EGM2008 output inspection");
        assert_eq!(output_preview["metadata"]["driver"], "GTiff");
        assert_eq!(output_preview["metadata"]["width"], 4);
        assert_eq!(output_preview["metadata"]["height"], 4);
        assert_eq!(
            output_preview["metadata"]["area_or_point"], input_preview["metadata"]["area_or_point"],
            "vertical conversion must preserve the source pixel registration convention"
        );
    }

    #[test]
    #[ignore = "requires optional native GIS runtime and INSAR_NATIVE_GIS_* fixture inputs"]
    fn configured_product_runtime_preserves_point_registration_through_egm2008() {
        let root = std::env::var("INSAR_NATIVE_GIS_RUNTIME_ROOT")
            .expect("INSAR_NATIVE_GIS_RUNTIME_ROOT must be set");
        let dem = std::env::var("INSAR_NATIVE_GIS_POINT_DEM")
            .expect("INSAR_NATIVE_GIS_POINT_DEM must be set");
        let engine = NativeGisEngine::from_product_root(root).expect("configured GIS runtime");
        let input_preview = engine
            .inspect_dem(&dem)
            .expect("Point DEM input inspection");
        assert_eq!(input_preview["metadata"]["area_or_point"], "Point");
        let output_root = std::env::temp_dir().join(format!(
            "sceneport-dem-point-registration-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&output_root);
        std::fs::create_dir_all(&output_root).expect("Point DEM output directory");
        let ellipsoid = output_root.join("point_ellipsoid.tif");
        let sarscape = output_root.join("point_dem");
        engine
            .convert_dem(&dem, &ellipsoid, Some(&sarscape), "EGM2008")
            .expect("Point DEM EGM2008 conversion");
        let output_preview = engine
            .inspect_dem(&ellipsoid)
            .expect("Point DEM output inspection");
        assert_eq!(output_preview["metadata"]["area_or_point"], "Point");
        assert_eq!(
            output_preview["metadata"]["geotransform"],
            input_preview["metadata"]["geotransform"]
        );
        let sarscape_preview = engine
            .inspect_dem(&sarscape)
            .expect("Point SARscape DEM output inspection");
        assert_eq!(sarscape_preview["metadata"]["driver"], "ENVI");
        assert_eq!(
            sarscape_preview["metadata"]["geotransform"], input_preview["metadata"]["geotransform"],
            "SARscape map info must reopen on the same Point grid"
        );
        std::fs::remove_dir_all(output_root).expect("remove Point DEM test output");
    }

    #[test]
    #[ignore = "requires optional native GIS runtime and INSAR_NATIVE_GIS_* fixture inputs"]
    fn configured_product_runtime_preserves_nodata_through_vertical_conversion() {
        let root = std::env::var("INSAR_NATIVE_GIS_RUNTIME_ROOT")
            .expect("INSAR_NATIVE_GIS_RUNTIME_ROOT must be set");
        let nodata_dem = std::env::var("INSAR_NATIVE_GIS_NODATA_DEM")
            .expect("INSAR_NATIVE_GIS_NODATA_DEM must be set");
        let output_root = std::env::var("INSAR_NATIVE_GIS_NODATA_OUTPUT_ROOT")
            .expect("INSAR_NATIVE_GIS_NODATA_OUTPUT_ROOT must be set");
        let engine = NativeGisEngine::from_product_root(root).expect("configured GIS runtime");
        let input_preview = engine
            .inspect_dem(&nodata_dem)
            .expect("native nodata input inspection");
        assert_eq!(input_preview["metadata"]["nodata"], -9999.0);
        let output_root = std::path::PathBuf::from(output_root);
        std::fs::create_dir_all(&output_root).expect("native nodata output directory");
        let ellipsoid = output_root.join("native_nodata_ellipsoid.tif");
        engine
            .convert_dem(&nodata_dem, &ellipsoid, None, "EGM96")
            .expect("native nodata conversion");
        let output_preview = engine
            .inspect_dem(&ellipsoid)
            .expect("native nodata output inspection");
        assert_eq!(output_preview["metadata"]["nodata"], -9999.0);
    }

    #[test]
    #[ignore = "requires optional native GIS runtime and INSAR_NATIVE_GIS_* fixture inputs"]
    fn configured_product_runtime_checks_forward_and_reverse_vertical_references() {
        let root = std::env::var("INSAR_NATIVE_GIS_RUNTIME_ROOT")
            .expect("INSAR_NATIVE_GIS_RUNTIME_ROOT must be set");
        let engine = NativeGisEngine::from_product_root(root).expect("configured GIS runtime");
        let runtime = engine.runtime.as_deref().expect("native runtime");
        for (model, expected_forward) in [
            ("EGM96", 68.077_068_328_857_42),
            ("EGM2008", 65.756_599_426_269_53),
        ] {
            let grid = engine
                .vertical_grids()
                .required_path(model)
                .expect("admitted vertical grid");
            let (forward, reverse) =
                raster::vertical_reference_round_trip(runtime, &grid, 100.125, 30.875, 100.0)
                    .expect("native forward/reverse vertical reference");
            println!("native_vertical_reference model={model} forward={forward} reverse={reverse}");
            assert!((forward - expected_forward).abs() < 0.0001);
            assert!((reverse - 100.0).abs() < 0.0001);
        }
    }

    #[test]
    #[ignore = "requires optional native GIS runtime and INSAR_NATIVE_GIS_* fixture inputs"]
    fn configured_product_runtime_crops_reprojects_and_resamples_without_worker() {
        let root = std::env::var("INSAR_NATIVE_GIS_RUNTIME_ROOT")
            .expect("INSAR_NATIVE_GIS_RUNTIME_ROOT must be set");
        let dem = std::env::var("INSAR_NATIVE_GIS_CROP_REPROJECT_INPUT")
            .expect("INSAR_NATIVE_GIS_CROP_REPROJECT_INPUT must be set");
        let output_root = std::env::var("INSAR_NATIVE_GIS_CROP_REPROJECT_OUTPUT_ROOT")
            .expect("INSAR_NATIVE_GIS_CROP_REPROJECT_OUTPUT_ROOT must be set");
        let engine = NativeGisEngine::from_product_root(root).expect("configured GIS runtime");
        let output_root = std::path::PathBuf::from(output_root);
        std::fs::create_dir_all(&output_root).expect("native crop/reproject output directory");
        let mercator = |degrees: f64| degrees.to_radians() * 6_378_137.0;
        let mercator_y = |degrees: f64| {
            let radians = degrees.to_radians();
            6_378_137.0 * (std::f64::consts::FRAC_PI_4 + radians / 2.0).tan().ln()
        };
        let output = output_root.join("native_crop_reprojected.tif");
        let result = engine
            .crop_reproject_resample_dem(
                &dem,
                &output,
                "EPSG:3857",
                [
                    mercator(100.0),
                    mercator_y(30.5),
                    mercator(100.5),
                    mercator_y(31.0),
                ],
                2,
                2,
                "bilinear",
            )
            .expect("native crop/reproject/resample");
        assert_eq!(result["operation"], "crop_reproject_resample");
        assert_eq!(result["resampling"], "bilinear");
        let preview = engine
            .inspect_dem(&output)
            .expect("native crop/reproject output inspection");
        assert_eq!(preview["metadata"]["driver"], "GTiff");
        assert_eq!(preview["metadata"]["width"], 2);
        assert_eq!(preview["metadata"]["height"], 2);
        assert!(preview["metadata"]["crs"]
            .as_str()
            .is_some_and(|value| value.contains("3857") || value.contains("Mercator")));
    }

    #[test]
    fn unavailable_native_engine_fails_closed_without_a_worker_fallback() {
        let error = NativeGisEngine::unavailable()
            .convert_dem("missing.tif", "out.tif", None, "EGM2008")
            .expect_err("native DEM execution must not fall back to a Worker");
        assert_eq!(error.code(), NativeGisErrorCode::GisEngineUnavailable);
    }

    #[test]
    #[ignore = "requires optional native GIS runtime (run scripts/run_native_gis_tests.sh with INSAR_GIS_RUNTIME_ROOT)"]
    fn concurrent_from_product_root_initialization_is_race_free() {
        let root = std::env::var("INSAR_GIS_RUNTIME_ROOT")
            .expect("INSAR_GIS_RUNTIME_ROOT must point to runtime/gis");
        assert!(
            std::path::Path::new(&root).join("manifest.json").is_file(),
            "INSAR_GIS_RUNTIME_ROOT must contain manifest.json: {root}"
        );
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(16));
        let mut handles = Vec::with_capacity(16);
        for _ in 0..16 {
            let root = root.clone();
            let barrier = std::sync::Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                for _ in 0..4 {
                    let engine =
                        NativeGisEngine::from_product_root(&root).expect("concurrent GIS runtime");
                    assert!(engine.is_available());
                }
            }));
        }
        for handle in handles {
            handle.join().expect("concurrent GIS thread must not panic");
        }
    }

    #[test]
    fn macos_app_bundle_discovery_resolves_contents_resources_runtime_gis() {
        let root = std::env::temp_dir().join(format!(
            "insar-studio-macos-bundle-discovery-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);

        let app_exe = root.join("Fake.app/Contents/MacOS/EOStage");
        let bundled_gis = root.join("Fake.app/Contents/Resources/runtime/gis");
        let bundled_grids = root.join("Fake.app/Contents/Resources/runtime/vertical-grids");
        fs::create_dir_all(app_exe.parent().expect("MacOS dir")).expect("create MacOS dir");
        fs::create_dir_all(&bundled_gis).expect("create Resources/runtime/gis");
        fs::create_dir_all(&bundled_grids).expect("create Resources/runtime/vertical-grids");
        fs::write(&app_exe, b"").expect("exe placeholder");
        fs::write(bundled_gis.join("manifest.json"), b"{}").expect("gis manifest");
        fs::write(bundled_grids.join("manifest.json"), b"[]").expect("grid manifest");

        let (resolved_gis, resolved_grids) =
            super::discovered_product_roots_for_executable(&app_exe)
                .expect("macOS .app layout should resolve bundle roots");
        assert_eq!(resolved_gis, bundled_gis);
        assert_eq!(resolved_grids, bundled_grids);

        let unbundled_exe = root.join("target/debug/deps/test_bin");
        fs::create_dir_all(unbundled_exe.parent().expect("deps dir")).expect("create deps dir");
        fs::write(&unbundled_exe, b"").expect("unbundled exe placeholder");
        let (unbundled_gis, _) = super::discovered_product_roots_for_executable(&unbundled_exe)
            .expect("unbundled exe should compute direct fallback without walking parents");
        assert_eq!(unbundled_gis, root.join("target/debug/deps/runtime/gis"));

        let _ = fs::remove_dir_all(&root);
    }
}
