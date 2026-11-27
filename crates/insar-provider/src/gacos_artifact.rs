//! Pure admission checks for locally materialized GACOS result products.
//!
//! This module deliberately has no filesystem, network, Store, Scheduler, or
//! credential boundary. The caller supplies exact file sizes, bounded RSC/TIFF
//! probes, and, for GeoTIFF, facts obtained from the product-owned native GIS
//! runtime. The result is a compact scientific admission record; it never copies,
//! renames, or marks a task complete.
//!
//! The current `insar-gis` public raster entry point is named
//! `NativeGisEngine::inspect_dem` and returns `driver`, `width`, `height`,
//! `count`, `dtype`, `crs`, and `bounds` metadata.  There is no public
//! `inspect_raster` function on the current checkout.  `GacosRasterInspectionFacts`
//! mirrors those fields and additionally requires the native caller to supply
//! the signed longitude/latitude pixel steps.  This keeps the provider crate
//! independent of the GIS runtime while making a magic-only TIFF check
//! insufficient for admission.

use std::collections::{BTreeMap, BTreeSet};
use std::str;

use chrono::{Datelike, NaiveDate};
use thiserror::Error;

use crate::gacos_delivery::{GacosBoundingBox, GacosUtcTime};

/// The bounded `.ztd.rsc` header size accepted by the native import boundary.
pub const GACOS_MAX_RSC_HEADER_BYTES: usize = 2 * 1024 * 1024;
/// The existing native result-file product bound.
pub const GACOS_MAX_ARTIFACT_BYTES: u64 = 8 * 1024 * 1024 * 1024;
/// Binary GACOS values are little-endian 32-bit floating point samples.
pub const GACOS_BINARY_BYTES_PER_SAMPLE: u64 = 4;
/// The artifact batch cannot exceed the provider's 20-date request bound.
pub const GACOS_MAX_DATES_PER_ADMISSION: usize = 20;
/// At most two materialized binary files per date, or one GeoTIFF per date.
pub const GACOS_MAX_FILES_PER_ADMISSION: usize = 40;
/// Only the TIFF signature prefix is needed before native GIS inspection.
pub const GACOS_TIFF_PREFIX_BYTES: usize = 8;

const GRID_COORDINATE_EPSILON: f64 = 1e-8;

#[derive(Clone, Debug, PartialEq, Eq)]
struct WktRootCrs {
    geographic: bool,
    projected: bool,
    epsg_codes: Vec<u32>,
}

/// Typed, secret-free failures from GACOS artifact admission.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum GacosArtifactError {
    #[error("GACOS expected date set is empty")]
    MissingExpectedDates,
    #[error("GACOS expected date set contains a duplicate")]
    DuplicateExpectedDate,
    #[error("GACOS expected date set exceeds the 20-date bound")]
    TooManyExpectedDates,
    #[error("GACOS artifact file list exceeds the 40-file bound")]
    TooManyFiles,
    #[error("GACOS expected date cannot be represented as YYYYMMDD")]
    InvalidExpectedDate,
    #[error("GACOS artifact filename is invalid")]
    InvalidFilename,
    #[error("GACOS artifact filename contains a path traversal component")]
    PathTraversal,
    #[error("GACOS artifact file type is unsupported")]
    UnsupportedFile,
    #[error("GACOS artifact contains two product representations for one date")]
    DuplicateProductDate,
    #[error("GACOS artifact is missing an expected date")]
    MissingDate,
    #[error("GACOS artifact contains a date outside the frozen request")]
    ExtraDate,
    #[error("GACOS binary product is missing .ztd or .ztd.rsc")]
    MissingBinaryPart,
    #[error("GACOS binary product contains a duplicate part")]
    DuplicateBinaryPart,
    #[error("GACOS binary file cannot carry GeoTIFF inspection facts")]
    UnexpectedRasterInspection,
    #[error("GACOS GeoTIFF requires native raster inspection facts")]
    MissingRasterInspection,
    #[error("GACOS artifact is empty")]
    EmptyArtifact,
    #[error("GACOS artifact exceeds the product size bound")]
    ArtifactTooLarge,
    #[error("GACOS .ztd.rsc header exceeds the bounded header limit")]
    HeaderTooLarge,
    #[error("GACOS .ztd.rsc header is not valid UTF-8")]
    HeaderNotUtf8,
    #[error("GACOS .ztd.rsc header is malformed")]
    HeaderMalformed,
    #[error("GACOS .ztd.rsc header contains a duplicate field")]
    DuplicateHeaderField,
    #[error("GACOS .ztd.rsc FILE_LENGTH and LENGTH aliases conflict")]
    ConflictingLengthAliases,
    #[error("GACOS binary header evidence is missing")]
    MissingHeaderEvidence,
    #[error("GACOS binary header evidence is shorter than the declared file")]
    HeaderEvidenceIncomplete,
    #[error("GACOS GeoTIFF prefix evidence is missing")]
    MissingRasterPrefix,
    #[error("GACOS GeoTIFF prefix evidence is shorter than required")]
    RasterPrefixIncomplete,
    #[error("GACOS GeoTIFF prefix evidence exceeds the 8-byte bound")]
    RasterPrefixTooLarge,
    #[error("GACOS binary data contains unexpected header evidence")]
    UnexpectedHeaderEvidence,
    #[error("GACOS artifact contains unexpected TIFF prefix evidence")]
    UnexpectedRasterPrefix,
    #[error("GACOS .ztd.rsc header is missing {0}")]
    MissingHeaderField(&'static str),
    #[error("GACOS .ztd.rsc header field {0} is invalid")]
    InvalidHeaderField(&'static str),
    #[error("GACOS binary size arithmetic overflowed")]
    SizeOverflow,
    #[error("GACOS requested UTC is invalid")]
    InvalidRequestedUtc,
    #[error("GACOS .ztd size does not match the .ztd.rsc grid")]
    SizeMismatch,
    #[error("GACOS binary grid georeference is invalid")]
    InvalidBinaryGeoreference,
    #[error("GACOS binary grid CRS is not WGS84 geographic")]
    InvalidCrs,
    #[error("GACOS grid does not cover the frozen request bounding box")]
    BoundingBoxMismatch,
    #[error("GACOS GeoTIFF bytes are not a TIFF payload")]
    InvalidRasterBytes,
    #[error("GACOS GeoTIFF driver evidence is not GTiff")]
    InvalidRasterDriver,
    #[error("GACOS GeoTIFF dimensions or band evidence is invalid")]
    InvalidRasterDimensions,
    #[error("GACOS GeoTIFF data type evidence is invalid")]
    InvalidRasterDataType,
    #[error("GACOS GeoTIFF georeference evidence is invalid")]
    InvalidRasterGeoreference,
    #[error("GACOS GeoTIFF CRS evidence is invalid")]
    InvalidRasterCrs,
}

/// A complete byte-slice input supplied by the caller without any filesystem
/// access. This is the convenience form; large-file callers should use
/// [`GacosArtifactEvidence`] instead.
///
/// `raster_inspection` is meaningful only for `.tif` files. The caller must
/// obtain it from the reviewed native GIS runtime; this module does not trust a
/// TIFF magic prefix as a substitute for the injected facts.
#[derive(Clone, Copy, Debug)]
pub struct GacosArtifactFile<'a> {
    pub name: &'a str,
    pub bytes: &'a [u8],
    pub raster_inspection: Option<&'a GacosRasterInspectionFacts>,
}

impl<'a> GacosArtifactFile<'a> {
    pub const fn binary_data(name: &'a str, bytes: &'a [u8]) -> Self {
        Self {
            name,
            bytes,
            raster_inspection: None,
        }
    }

    pub const fn binary_header(name: &'a str, bytes: &'a [u8]) -> Self {
        Self {
            name,
            bytes,
            raster_inspection: None,
        }
    }

    pub const fn geotiff(
        name: &'a str,
        bytes: &'a [u8],
        raster_inspection: &'a GacosRasterInspectionFacts,
    ) -> Self {
        Self {
            name,
            bytes,
            raster_inspection: Some(raster_inspection),
        }
    }
}

/// Bounded evidence for one already-materialized GACOS file.
///
/// `size_bytes` is the exact source file size and is independent of the
/// bounded evidence slices. Binary data needs no payload bytes. A binary RSC
/// file supplies its complete bounded header, while a GeoTIFF supplies only its
/// first eight bytes plus facts obtained from the native GIS runtime.
#[derive(Clone, Copy, Debug)]
pub struct GacosArtifactEvidence<'a> {
    pub name: &'a str,
    pub size_bytes: u64,
    pub header_bytes: Option<&'a [u8]>,
    pub tiff_prefix: Option<&'a [u8]>,
    pub raster_inspection: Option<&'a GacosRasterInspectionFacts>,
}

impl<'a> GacosArtifactEvidence<'a> {
    pub const fn binary_data(name: &'a str, size_bytes: u64) -> Self {
        Self {
            name,
            size_bytes,
            header_bytes: None,
            tiff_prefix: None,
            raster_inspection: None,
        }
    }

    pub const fn binary_header(name: &'a str, size_bytes: u64, header_bytes: &'a [u8]) -> Self {
        Self {
            name,
            size_bytes,
            header_bytes: Some(header_bytes),
            tiff_prefix: None,
            raster_inspection: None,
        }
    }

    pub const fn geotiff(
        name: &'a str,
        size_bytes: u64,
        tiff_prefix: &'a [u8],
        raster_inspection: &'a GacosRasterInspectionFacts,
    ) -> Self {
        Self {
            name,
            size_bytes,
            header_bytes: None,
            tiff_prefix: Some(tiff_prefix),
            raster_inspection: Some(raster_inspection),
        }
    }
}

/// Frozen scientific context used while admitting a result batch.
#[derive(Clone, Copy, Debug)]
pub struct GacosArtifactAdmissionRequest<'a> {
    pub expected_dates: &'a [NaiveDate],
    pub frozen_bbox: GacosBoundingBox,
    /// UTC supplied with the request. It is not artifact-observed time.
    pub requested_utc: GacosUtcTime,
    pub files: &'a [GacosArtifactFile<'a>],
}

impl<'a> GacosArtifactAdmissionRequest<'a> {
    pub const fn new(
        expected_dates: &'a [NaiveDate],
        frozen_bbox: GacosBoundingBox,
        requested_utc: GacosUtcTime,
        files: &'a [GacosArtifactFile<'a>],
    ) -> Self {
        Self {
            expected_dates,
            frozen_bbox,
            requested_utc,
            files,
        }
    }
}

/// Frozen scientific context backed by exact file sizes and bounded probes.
#[derive(Clone, Copy, Debug)]
pub struct GacosArtifactEvidenceRequest<'a> {
    pub expected_dates: &'a [NaiveDate],
    pub frozen_bbox: GacosBoundingBox,
    pub requested_utc: GacosUtcTime,
    pub files: &'a [GacosArtifactEvidence<'a>],
}

impl<'a> GacosArtifactEvidenceRequest<'a> {
    pub const fn new(
        expected_dates: &'a [NaiveDate],
        frozen_bbox: GacosBoundingBox,
        requested_utc: GacosUtcTime,
        files: &'a [GacosArtifactEvidence<'a>],
    ) -> Self {
        Self {
            expected_dates,
            frozen_bbox,
            requested_utc,
            files,
        }
    }
}

/// Native raster inspection facts required for a GeoTIFF admission.
///
/// The first seven fields map to the current `insar-gis` inspection JSON.  The
/// public GIS inspection currently exposes bounds but not the geotransform;
/// `x_step` and `y_step` therefore remain explicit required evidence here.
#[derive(Clone, Debug, PartialEq)]
pub struct GacosRasterInspectionFacts {
    pub driver: String,
    pub width: u64,
    pub height: u64,
    pub band_count: u64,
    pub dtypes: Vec<String>,
    pub crs: String,
    /// Outer pixel-edge bounds `[west, south, east, north]` in WGS84
    /// longitude/latitude degrees, as reported by native raster inspection.
    pub bounds: [f64; 4],
    pub x_step: f64,
    pub y_step: f64,
}

impl GacosRasterInspectionFacts {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        driver: impl Into<String>,
        width: u64,
        height: u64,
        band_count: u64,
        dtypes: Vec<String>,
        crs: impl Into<String>,
        bounds: [f64; 4],
        x_step: f64,
        y_step: f64,
    ) -> Self {
        Self {
            driver: driver.into(),
            width,
            height,
            band_count,
            dtypes,
            crs: crs.into(),
            bounds,
            x_step,
            y_step,
        }
    }
}

/// Parsed and validated binary `.ztd.rsc` scientific header.
#[derive(Clone, Debug, PartialEq)]
pub struct GacosBinaryHeader {
    pub width: u64,
    pub file_length: u64,
    pub x_first: f64,
    pub y_first: f64,
    pub x_step: f64,
    pub y_step: f64,
    /// CRS text explicitly declared by the provider header, if present.
    ///
    /// The absence of this value is intentional: TRAIN's GACOS reader uses
    /// longitude/latitude grid semantics without requiring a CRS field. The
    /// native admission record keeps that provider contract evidence separate
    /// from an explicit header declaration rather than fabricating EPSG:4326.
    pub header_declared: Option<String>,
    /// Geographic longitude/latitude semantics established by the GACOS
    /// provider contract and TRAIN compatibility behavior.
    pub provider_contract_geographic: bool,
}

impl GacosBinaryHeader {
    pub fn grid_bounds(&self) -> Result<[f64; 4], GacosArtifactError> {
        grid_bounds(
            self.width,
            self.file_length,
            self.x_first,
            self.y_first,
            self.x_step,
            self.y_step,
        )
    }
}

/// The representation admitted for one expected date.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GacosArtifactKind {
    Binary,
    GeoTiff,
}

/// A compact admission record.  It contains no payload bytes or filesystem
/// paths and is safe for a caller to hand to a later Store/Worker boundary.
#[derive(Clone, Debug, PartialEq)]
pub struct GacosAdmittedProduct {
    pub date: NaiveDate,
    pub kind: GacosArtifactKind,
    pub name: String,
    pub size_bytes: u64,
    pub binary_header: Option<GacosBinaryHeader>,
    pub raster_inspection: Option<GacosRasterInspectionFacts>,
}

/// Successful, network-free artifact admission.
#[derive(Clone, Debug, PartialEq)]
pub struct GacosArtifactAdmission {
    pub expected_dates: Vec<NaiveDate>,
    pub frozen_bbox: GacosBoundingBox,
    /// UTC frozen into the originating request.
    pub requested_utc: GacosUtcTime,
    /// Artifact-observed UTC, when an explicit provider field is eventually
    /// verified. `.ztd`/`.ztd.rsc` currently provide no such field.
    pub observed_utc: Option<GacosUtcTime>,
    pub products: Vec<GacosAdmittedProduct>,
}

/// Parse a bounded UTF-8 binary product header.
pub fn parse_gacos_binary_header(bytes: &[u8]) -> Result<GacosBinaryHeader, GacosArtifactError> {
    let fields = parse_header_fields(bytes)?;
    let width = positive_integer(&fields, "WIDTH")?;
    let file_length = positive_integer_alias(&fields)?;
    let x_first = finite_number(&fields, "X_FIRST")?;
    let y_first = finite_number(&fields, "Y_FIRST")?;
    let x_step = finite_number(&fields, "X_STEP")?;
    let y_step = finite_number(&fields, "Y_STEP")?;
    let header_declared = header_crs(&fields)?;

    // This validates dimensions, coordinate domains, monotonic direction, and
    // checked floating extent construction before the header leaves this API.
    let _ = grid_bounds(width, file_length, x_first, y_first, x_step, y_step)?;

    Ok(GacosBinaryHeader {
        width,
        file_length,
        x_first,
        y_first,
        x_step,
        y_step,
        header_declared,
        provider_contract_geographic: true,
    })
}

/// Validate binary byte size and georeference against frozen request bounds.
pub fn validate_gacos_binary_grid(
    header: &GacosBinaryHeader,
    ztd_size_bytes: usize,
    frozen_bbox: &GacosBoundingBox,
) -> Result<(), GacosArtifactError> {
    validate_gacos_binary_grid_size(header, ztd_size_bytes as u64, frozen_bbox)
}

/// Validate a binary grid using an exact source size without requiring payload
/// allocation.
pub fn validate_gacos_binary_grid_size(
    header: &GacosBinaryHeader,
    ztd_size_bytes: u64,
    frozen_bbox: &GacosBoundingBox,
) -> Result<(), GacosArtifactError> {
    validate_frozen_bbox(frozen_bbox)?;
    validate_artifact_size_u64(ztd_size_bytes)?;
    let expected = header
        .width
        .checked_mul(header.file_length)
        .and_then(|pixels| pixels.checked_mul(GACOS_BINARY_BYTES_PER_SAMPLE))
        .ok_or(GacosArtifactError::SizeOverflow)?;
    if expected != ztd_size_bytes {
        return Err(GacosArtifactError::SizeMismatch);
    }
    if !header.provider_contract_geographic {
        return Err(GacosArtifactError::InvalidCrs);
    }
    if let Some(header_declared) = &header.header_declared {
        validate_header_declared_crs(header_declared)?;
    }
    let bounds = header
        .grid_bounds()
        .map_err(|_| GacosArtifactError::InvalidBinaryGeoreference)?;
    if !bounds_cover_request(bounds, frozen_bbox) {
        return Err(GacosArtifactError::BoundingBoxMismatch);
    }
    Ok(())
}

/// Validate native GeoTIFF facts and a complete TIFF payload.
pub fn validate_gacos_geotiff_facts(
    facts: &GacosRasterInspectionFacts,
    tiff_bytes: &[u8],
    frozen_bbox: &GacosBoundingBox,
) -> Result<(), GacosArtifactError> {
    let prefix_len = tiff_bytes.len().min(GACOS_TIFF_PREFIX_BYTES);
    validate_gacos_geotiff_evidence(
        facts,
        tiff_bytes.len() as u64,
        &tiff_bytes[..prefix_len],
        frozen_bbox,
    )
}

/// Validate GeoTIFF evidence using an exact source size and at most the first
/// eight bytes of the payload. Native GIS facts remain mandatory.
pub fn validate_gacos_geotiff_evidence(
    facts: &GacosRasterInspectionFacts,
    tiff_size_bytes: u64,
    tiff_prefix: &[u8],
    frozen_bbox: &GacosBoundingBox,
) -> Result<(), GacosArtifactError> {
    validate_frozen_bbox(frozen_bbox)?;
    validate_artifact_size_u64(tiff_size_bytes)?;
    validate_tiff_prefix(tiff_size_bytes, tiff_prefix)?;
    if !is_tiff_signature(tiff_prefix) {
        return Err(GacosArtifactError::InvalidRasterBytes);
    }
    if !matches!(
        facts.driver.trim().to_ascii_lowercase().as_str(),
        "gtiff" | "geotiff"
    ) {
        return Err(GacosArtifactError::InvalidRasterDriver);
    }
    if facts.width == 0 || facts.height == 0 || facts.band_count != 1 || facts.dtypes.len() != 1 {
        return Err(GacosArtifactError::InvalidRasterDimensions);
    }
    let dtype = facts.dtypes[0].trim().to_ascii_lowercase();
    if dtype != "float32" && dtype != "float64" {
        return Err(GacosArtifactError::InvalidRasterDataType);
    }
    if validate_crs(&facts.crs).is_err() {
        return Err(GacosArtifactError::InvalidRasterCrs);
    }
    validate_raster_georeference(facts, frozen_bbox)
}

/// Admit a complete result batch from complete in-memory file bytes.
///
/// This is the compatibility wrapper. Future workers should prefer
/// [`admit_gacos_artifact_evidence`] so multi-gigabyte files are represented by
/// exact sizes and bounded probes rather than allocated payloads.
pub fn admit_gacos_artifacts(
    request: &GacosArtifactAdmissionRequest<'_>,
) -> Result<GacosArtifactAdmission, GacosArtifactError> {
    let evidence_files = request
        .files
        .iter()
        .map(byte_file_to_evidence)
        .collect::<Vec<_>>();
    let evidence_request = GacosArtifactEvidenceRequest::new(
        request.expected_dates,
        request.frozen_bbox,
        request.requested_utc,
        &evidence_files,
    );
    admit_gacos_artifact_evidence(&evidence_request)
}

fn byte_file_to_evidence<'a>(file: &GacosArtifactFile<'a>) -> GacosArtifactEvidence<'a> {
    let header_bytes = file.name.ends_with(".ztd.rsc").then_some(file.bytes);
    let tiff_prefix = file
        .name
        .ends_with(".tif")
        .then_some(&file.bytes[..file.bytes.len().min(GACOS_TIFF_PREFIX_BYTES)]);
    GacosArtifactEvidence {
        name: file.name,
        size_bytes: file.bytes.len() as u64,
        header_bytes,
        tiff_prefix,
        raster_inspection: file.raster_inspection,
    }
}

/// Admit a complete result batch from exact sizes, bounded header/prefix bytes,
/// and optional native raster facts.
pub fn admit_gacos_artifact_evidence(
    request: &GacosArtifactEvidenceRequest<'_>,
) -> Result<GacosArtifactAdmission, GacosArtifactError> {
    validate_frozen_bbox(&request.frozen_bbox)?;
    // `GacosUtcTime` is intentionally opaque and normally constructor-validated,
    // but retain this check for values assembled by future deserialization paths.
    GacosUtcTime::new(request.requested_utc.hour(), request.requested_utc.minute())
        .map_err(|_| GacosArtifactError::InvalidRequestedUtc)?;

    let expected_dates = normalized_expected_dates(request.expected_dates)?;
    if request.files.len() > GACOS_MAX_FILES_PER_ADMISSION {
        return Err(GacosArtifactError::TooManyFiles);
    }
    let mut pending = BTreeMap::<NaiveDate, PendingProduct>::new();
    for (index, file) in request.files.iter().enumerate() {
        let (date, part) = parse_artifact_name(file.name)?;
        let product = pending.entry(date).or_default();
        match part {
            ArtifactPart::BinaryData => {
                if file.header_bytes.is_some() {
                    return Err(GacosArtifactError::UnexpectedHeaderEvidence);
                }
                if file.tiff_prefix.is_some() {
                    return Err(GacosArtifactError::UnexpectedRasterPrefix);
                }
                if file.raster_inspection.is_some() {
                    return Err(GacosArtifactError::UnexpectedRasterInspection);
                }
                if product.binary_data.replace(index).is_some() {
                    return Err(GacosArtifactError::DuplicateBinaryPart);
                }
            }
            ArtifactPart::BinaryHeader => {
                if file.header_bytes.is_none() {
                    return Err(GacosArtifactError::MissingHeaderEvidence);
                }
                if file.tiff_prefix.is_some() {
                    return Err(GacosArtifactError::UnexpectedRasterPrefix);
                }
                if file.raster_inspection.is_some() {
                    return Err(GacosArtifactError::UnexpectedRasterInspection);
                }
                if product.binary_header.replace(index).is_some() {
                    return Err(GacosArtifactError::DuplicateBinaryPart);
                }
            }
            ArtifactPart::GeoTiff => {
                if file.header_bytes.is_some() {
                    return Err(GacosArtifactError::UnexpectedHeaderEvidence);
                }
                if product.geotiff.replace(index).is_some()
                    || product.binary_data.is_some()
                    || product.binary_header.is_some()
                {
                    return Err(GacosArtifactError::DuplicateProductDate);
                }
                if file.raster_inspection.is_none() {
                    return Err(GacosArtifactError::MissingRasterInspection);
                }
                if file.tiff_prefix.is_none() {
                    return Err(GacosArtifactError::MissingRasterPrefix);
                }
            }
        }
        if part != ArtifactPart::GeoTiff && product.geotiff.is_some() {
            return Err(GacosArtifactError::DuplicateProductDate);
        }
    }

    let actual_dates = pending.keys().copied().collect::<BTreeSet<_>>();
    if expected_dates
        .iter()
        .any(|date| !actual_dates.contains(date))
    {
        return Err(GacosArtifactError::MissingDate);
    }
    if actual_dates
        .iter()
        .any(|date| !expected_dates.contains(date))
    {
        return Err(GacosArtifactError::ExtraDate);
    }

    let mut products = Vec::with_capacity(pending.len());
    for (date, product) in pending {
        if let Some(index) = product.geotiff {
            let file = &request.files[index];
            let facts = file
                .raster_inspection
                .ok_or(GacosArtifactError::MissingRasterInspection)?;
            let tiff_prefix = file
                .tiff_prefix
                .ok_or(GacosArtifactError::MissingRasterPrefix)?;
            validate_gacos_geotiff_evidence(
                facts,
                file.size_bytes,
                tiff_prefix,
                &request.frozen_bbox,
            )?;
            products.push(GacosAdmittedProduct {
                date,
                kind: GacosArtifactKind::GeoTiff,
                name: file.name.to_owned(),
                size_bytes: file.size_bytes,
                binary_header: None,
                raster_inspection: Some(facts.clone()),
            });
            continue;
        }

        let data_index = product
            .binary_data
            .ok_or(GacosArtifactError::MissingBinaryPart)?;
        let header_index = product
            .binary_header
            .ok_or(GacosArtifactError::MissingBinaryPart)?;
        let data = &request.files[data_index];
        let header_file = &request.files[header_index];
        let header_bytes = header_file
            .header_bytes
            .ok_or(GacosArtifactError::MissingHeaderEvidence)?;
        validate_header_evidence(header_file.size_bytes, header_bytes)?;
        let header = parse_gacos_binary_header(header_bytes)?;
        validate_gacos_binary_grid_size(&header, data.size_bytes, &request.frozen_bbox)?;
        products.push(GacosAdmittedProduct {
            date,
            kind: GacosArtifactKind::Binary,
            name: data.name.to_owned(),
            size_bytes: data.size_bytes,
            binary_header: Some(header),
            raster_inspection: None,
        });
    }

    Ok(GacosArtifactAdmission {
        expected_dates,
        frozen_bbox: request.frozen_bbox,
        requested_utc: request.requested_utc,
        observed_utc: None,
        products,
    })
}

#[derive(Clone, Copy, Debug, Default)]
struct PendingProduct {
    binary_data: Option<usize>,
    binary_header: Option<usize>,
    geotiff: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ArtifactPart {
    BinaryData,
    BinaryHeader,
    GeoTiff,
}

fn normalized_expected_dates(
    expected_dates: &[NaiveDate],
) -> Result<Vec<NaiveDate>, GacosArtifactError> {
    if expected_dates.is_empty() {
        return Err(GacosArtifactError::MissingExpectedDates);
    }
    if expected_dates.len() > GACOS_MAX_DATES_PER_ADMISSION {
        return Err(GacosArtifactError::TooManyExpectedDates);
    }
    let mut dates = BTreeSet::new();
    for date in expected_dates {
        if !representable_date(*date) {
            return Err(GacosArtifactError::InvalidExpectedDate);
        }
        if !dates.insert(*date) {
            return Err(GacosArtifactError::DuplicateExpectedDate);
        }
    }
    Ok(dates.into_iter().collect())
}

fn representable_date(date: NaiveDate) -> bool {
    (1..=9999).contains(&date.year())
}

fn parse_artifact_name(name: &str) -> Result<(NaiveDate, ArtifactPart), GacosArtifactError> {
    validate_artifact_name(name)?;
    if let Some(token) = name.strip_suffix(".ztd.rsc") {
        return parse_date_token(token).map(|date| (date, ArtifactPart::BinaryHeader));
    }
    if let Some(token) = name.strip_suffix(".ztd") {
        return parse_date_token(token).map(|date| (date, ArtifactPart::BinaryData));
    }
    if let Some(token) = name.strip_suffix(".tif") {
        // Preserve the existing importer’s optional YYYYMMDD.ztd.tif spelling,
        // while keeping the date token itself exact and unambiguous.
        let token = token.strip_suffix(".ztd").unwrap_or(token);
        return parse_date_token(token).map(|date| (date, ArtifactPart::GeoTiff));
    }
    Err(GacosArtifactError::UnsupportedFile)
}

fn validate_artifact_name(name: &str) -> Result<(), GacosArtifactError> {
    if name.is_empty() || name == "." || name == ".." || name.contains(':') {
        return Err(GacosArtifactError::InvalidFilename);
    }
    if name.bytes().any(|byte| byte == b'/' || byte == b'\\') || name.contains("..") {
        return Err(GacosArtifactError::PathTraversal);
    }
    if name.chars().any(char::is_control) {
        return Err(GacosArtifactError::InvalidFilename);
    }
    Ok(())
}

fn parse_date_token(token: &str) -> Result<NaiveDate, GacosArtifactError> {
    if token.len() != 8 || !token.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(GacosArtifactError::InvalidFilename);
    }
    let date = NaiveDate::parse_from_str(token, "%Y%m%d")
        .map_err(|_| GacosArtifactError::InvalidFilename)?;
    representable_date(date)
        .then_some(date)
        .ok_or(GacosArtifactError::InvalidExpectedDate)
}

fn parse_header_fields(bytes: &[u8]) -> Result<BTreeMap<String, String>, GacosArtifactError> {
    if bytes.len() > GACOS_MAX_RSC_HEADER_BYTES {
        return Err(GacosArtifactError::HeaderTooLarge);
    }
    let text = str::from_utf8(bytes).map_err(|_| GacosArtifactError::HeaderNotUtf8)?;
    let mut fields = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line
            .chars()
            .any(|character| character.is_control() && character != '\t')
        {
            return Err(GacosArtifactError::HeaderMalformed);
        }
        let (key, value) = if let Some((key, value)) = line.split_once('=') {
            (key.trim().to_owned(), value.trim().to_owned())
        } else {
            let mut parts = line.split_whitespace();
            let key = parts.next().unwrap_or_default().to_owned();
            let value = parts.collect::<Vec<_>>().join(" ");
            (key, value)
        };
        if key.is_empty() || value.is_empty() {
            return Err(GacosArtifactError::HeaderMalformed);
        }
        let key = key.to_ascii_uppercase();
        if !key
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err(GacosArtifactError::HeaderMalformed);
        }
        if fields.insert(key, value).is_some() {
            return Err(GacosArtifactError::DuplicateHeaderField);
        }
    }
    Ok(fields)
}

fn positive_integer(
    fields: &BTreeMap<String, String>,
    key: &'static str,
) -> Result<u64, GacosArtifactError> {
    let value = fields
        .get(key)
        .ok_or(GacosArtifactError::MissingHeaderField(key))?;
    let parsed = value
        .parse::<u64>()
        .map_err(|_| GacosArtifactError::InvalidHeaderField(key))?;
    if parsed == 0 {
        return Err(GacosArtifactError::InvalidHeaderField(key));
    }
    Ok(parsed)
}

fn positive_integer_alias(fields: &BTreeMap<String, String>) -> Result<u64, GacosArtifactError> {
    match (fields.get("FILE_LENGTH"), fields.get("LENGTH")) {
        (None, None) => Err(GacosArtifactError::MissingHeaderField("FILE_LENGTH")),
        (Some(file_length), Some(length)) => {
            let file_length = file_length
                .parse::<u64>()
                .map_err(|_| GacosArtifactError::InvalidHeaderField("FILE_LENGTH"))?;
            let length = length
                .parse::<u64>()
                .map_err(|_| GacosArtifactError::InvalidHeaderField("LENGTH"))?;
            if file_length == 0 {
                return Err(GacosArtifactError::InvalidHeaderField("FILE_LENGTH"));
            }
            if length == 0 {
                return Err(GacosArtifactError::InvalidHeaderField("LENGTH"));
            }
            if file_length != length {
                return Err(GacosArtifactError::ConflictingLengthAliases);
            }
            Ok(file_length)
        }
        (Some(file_length), None) => {
            let parsed = file_length
                .parse::<u64>()
                .map_err(|_| GacosArtifactError::InvalidHeaderField("FILE_LENGTH"))?;
            (parsed > 0)
                .then_some(parsed)
                .ok_or(GacosArtifactError::InvalidHeaderField("FILE_LENGTH"))
        }
        (None, Some(length)) => {
            let parsed = length
                .parse::<u64>()
                .map_err(|_| GacosArtifactError::InvalidHeaderField("LENGTH"))?;
            (parsed > 0)
                .then_some(parsed)
                .ok_or(GacosArtifactError::InvalidHeaderField("LENGTH"))
        }
    }
}

fn finite_number(
    fields: &BTreeMap<String, String>,
    key: &'static str,
) -> Result<f64, GacosArtifactError> {
    let value = fields
        .get(key)
        .ok_or(GacosArtifactError::MissingHeaderField(key))?;
    let parsed = value
        .parse::<f64>()
        .map_err(|_| GacosArtifactError::InvalidHeaderField(key))?;
    parsed
        .is_finite()
        .then_some(parsed)
        .ok_or(GacosArtifactError::InvalidHeaderField(key))
}

fn header_crs(fields: &BTreeMap<String, String>) -> Result<Option<String>, GacosArtifactError> {
    let mut values = Vec::new();
    for key in [
        "CRS",
        "SRS",
        "EPSG",
        "PROJECTION",
        "DATUM",
        "COORDINATE_SYSTEM",
    ] {
        if let Some(value) = fields.get(key) {
            let value = if key == "EPSG" {
                format!("EPSG {value}")
            } else {
                value.clone()
            };
            // Reject a known projected/conflicting identity declaration even
            // when a second field separately mentions EPSG:4326. Otherwise
            // joining all declarations could accidentally hide the
            // contradiction. PROJECTION/DATUM may be separate components and
            // are validated together below.
            if matches!(key, "CRS" | "SRS" | "EPSG" | "COORDINATE_SYSTEM") {
                validate_header_declared_crs(&value)?;
            } else if key == "PROJECTION" || key == "DATUM" {
                let compact = value
                    .chars()
                    .filter(|character| character.is_ascii_alphanumeric())
                    .collect::<String>()
                    .to_ascii_uppercase();
                if (key == "DATUM" && !compact.contains("WGS84") && !compact.contains("WGS1984"))
                    || (key == "PROJECTION"
                        && (compact.contains("PROJECTED")
                            || compact.contains("PROJCRS")
                            || compact.contains("UTM")
                            || compact.contains("MERCATOR")))
                {
                    return Err(GacosArtifactError::InvalidCrs);
                }
            }
            values.push(value);
        }
    }
    if values.is_empty() {
        return Ok(None);
    }
    let joined = values.join(" ");
    validate_header_declared_crs(&joined)?;
    Ok(Some(joined))
}

fn validate_header_declared_crs(value: &str) -> Result<(), GacosArtifactError> {
    if let Some(root) = parse_wkt_root(value)? {
        validate_wkt_root(&root)?;
        let compact = compact_crs(value);
        if !compact.contains("WGS84")
            && !compact.contains("WGS1984")
            && !root.epsg_codes.contains(&4326)
        {
            return Err(GacosArtifactError::InvalidCrs);
        }
        return Ok(());
    }
    let epsg_codes = explicit_epsg_codes(value)?;
    if epsg_codes.iter().any(|code| *code != 4326) {
        return Err(GacosArtifactError::InvalidCrs);
    }
    let compact = compact_crs(value);
    let wgs84 =
        compact.contains("WGS84") || compact.contains("WGS1984") || epsg_codes.contains(&4326);
    let known_projected = compact.contains("WEBMERCATOR")
        || compact.contains("PROJECTED")
        || compact.contains("PROJCS")
        || compact.contains("PROJCRS")
        || compact.contains("UTM");
    if !wgs84 || known_projected {
        return Err(GacosArtifactError::InvalidCrs);
    }
    Ok(())
}

fn validate_crs(value: &str) -> Result<(), GacosArtifactError> {
    if let Some(root) = parse_wkt_root(value)? {
        validate_wkt_root(&root)?;
        let compact = compact_crs(value);
        if (compact.contains("WGS84")
            || compact.contains("WGS1984")
            || root.epsg_codes.contains(&4326))
            && root.geographic
        {
            return Ok(());
        }
        return Err(GacosArtifactError::InvalidCrs);
    }
    let epsg_codes = explicit_epsg_codes(value)?;
    if epsg_codes.iter().any(|code| *code != 4326) {
        return Err(GacosArtifactError::InvalidCrs);
    }
    let compact = compact_crs(value);
    let wgs84 =
        compact.contains("WGS84") || compact.contains("WGS1984") || epsg_codes.contains(&4326);
    let geographic = epsg_codes.contains(&4326)
        || compact.contains("LATLON")
        || compact.contains("GEOGRAPHIC")
        || compact.contains("GEOGCS")
        || compact.contains("GEOGCRS");
    let known_projected = compact.contains("WEBMERCATOR")
        || compact.contains("PROJECTED")
        || compact.contains("PROJCS")
        || compact.contains("PROJCRS")
        || compact.contains("UTM");
    if wgs84 && geographic && !known_projected {
        Ok(())
    } else {
        Err(GacosArtifactError::InvalidCrs)
    }
}

fn compact_crs(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_uppercase()
}

fn validate_wkt_root(root: &WktRootCrs) -> Result<(), GacosArtifactError> {
    if root.projected || !root.geographic || root.epsg_codes.iter().any(|code| *code != 4326) {
        return Err(GacosArtifactError::InvalidCrs);
    }
    Ok(())
}

/// Parse only the outer WKT CRS identity. Authority records nested in datum,
/// ellipsoid, unit, axis, or base-CRS nodes are metadata, not the CRS being
/// inspected, so they must not be treated as competing root EPSG identities.
fn parse_wkt_root(value: &str) -> Result<Option<WktRootCrs>, GacosArtifactError> {
    let trimmed = value.trim_start();
    let upper = trimmed.to_ascii_uppercase();
    let bytes = upper.as_bytes();
    let candidates = [
        (b"GEOGCS".as_slice(), true, false),
        (b"GEOGCRS".as_slice(), true, false),
        (b"PROJCS".as_slice(), false, true),
        (b"PROJCRS".as_slice(), false, true),
    ];
    let Some((keyword, geographic, projected)) = candidates.into_iter().find(|(keyword, _, _)| {
        bytes.starts_with(keyword)
            && bytes
                .get(keyword.len())
                .is_none_or(|byte| !is_wkt_identifier_byte(*byte))
    }) else {
        return Ok(None);
    };

    let mut open = keyword.len();
    while open < bytes.len() && bytes[open].is_ascii_whitespace() {
        open += 1;
    }
    if bytes.get(open) != Some(&b'[') {
        return Err(GacosArtifactError::InvalidCrs);
    }
    let close = matching_wkt_bracket(bytes, open).ok_or(GacosArtifactError::InvalidCrs)?;
    if bytes[close + 1..]
        .iter()
        .any(|byte| !byte.is_ascii_whitespace())
    {
        return Err(GacosArtifactError::InvalidCrs);
    }

    let mut epsg_codes = Vec::new();
    let mut cursor = open + 1;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    while cursor < close {
        let byte = bytes[cursor];
        if in_string {
            if byte == b'\\' && !escaped {
                escaped = true;
            } else {
                if byte == b'"' && !escaped {
                    in_string = false;
                }
                escaped = false;
            }
            cursor += 1;
            continue;
        }
        if byte == b'"' {
            in_string = true;
            cursor += 1;
            continue;
        }
        if depth == 0 {
            let entry = wkt_entry_open(bytes, cursor, close, b"AUTHORITY")
                .or_else(|| wkt_entry_open(bytes, cursor, close, b"ID"));
            if let Some(entry_open) = entry {
                let entry_close = matching_wkt_bracket(bytes, entry_open)
                    .ok_or(GacosArtifactError::InvalidCrs)?;
                if entry_close > close {
                    return Err(GacosArtifactError::InvalidCrs);
                }
                let entry_text = &trimmed[cursor..=entry_close];
                epsg_codes.extend(explicit_epsg_codes(entry_text)?);
                cursor = entry_close + 1;
                continue;
            }
        }
        match byte {
            b'[' => depth = depth.checked_add(1).ok_or(GacosArtifactError::InvalidCrs)?,
            b']' => {
                if depth == 0 {
                    return Err(GacosArtifactError::InvalidCrs);
                }
                depth -= 1;
            }
            _ => {}
        }
        cursor += 1;
    }
    if depth != 0 || in_string {
        return Err(GacosArtifactError::InvalidCrs);
    }
    Ok(Some(WktRootCrs {
        geographic,
        projected,
        epsg_codes,
    }))
}

fn wkt_entry_open(bytes: &[u8], cursor: usize, limit: usize, keyword: &[u8]) -> Option<usize> {
    if cursor + keyword.len() > limit
        || !bytes[cursor..].starts_with(keyword)
        || (cursor > 0 && is_wkt_identifier_byte(bytes[cursor - 1]))
    {
        return None;
    }
    let mut open = cursor + keyword.len();
    while open < limit && bytes[open].is_ascii_whitespace() {
        open += 1;
    }
    (open < limit && bytes[open] == b'[').then_some(open)
}

fn matching_wkt_bracket(bytes: &[u8], open: usize) -> Option<usize> {
    if bytes.get(open) != Some(&b'[') {
        return None;
    }
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (index, byte) in bytes.iter().enumerate().skip(open) {
        if in_string {
            if *byte == b'\\' && !escaped {
                escaped = true;
            } else {
                if *byte == b'"' && !escaped {
                    in_string = false;
                }
                escaped = false;
            }
            continue;
        }
        match *byte {
            b'"' => in_string = true,
            b'[' => depth = depth.checked_add(1)?,
            b']' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

fn is_wkt_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn explicit_epsg_codes(value: &str) -> Result<Vec<u32>, GacosArtifactError> {
    let upper = value.to_ascii_uppercase();
    let bytes = upper.as_bytes();
    let mut codes = Vec::new();
    let mut index = 0;
    while index + 4 <= bytes.len() {
        if &bytes[index..index + 4] != b"EPSG"
            || (index > 0 && is_wkt_identifier_byte(bytes[index - 1]))
        {
            index += 1;
            continue;
        }
        let mut cursor = index + 4;
        while cursor < bytes.len()
            && matches!(
                bytes[cursor],
                b':' | b'='
                    | b' '
                    | b'\t'
                    | b'\r'
                    | b'\n'
                    | b'"'
                    | b'\''
                    | b'['
                    | b']'
                    | b'('
                    | b')'
                    | b'{'
                    | b'}'
                    | b','
            )
        {
            cursor += 1;
        }
        let start = cursor;
        while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
            cursor += 1;
        }
        if start == cursor {
            return Err(GacosArtifactError::InvalidCrs);
        }
        let code = str::from_utf8(&bytes[start..cursor])
            .map_err(|_| GacosArtifactError::InvalidCrs)?
            .parse::<u32>()
            .map_err(|_| GacosArtifactError::InvalidCrs)?;
        if cursor < bytes.len() && is_wkt_identifier_byte(bytes[cursor]) {
            return Err(GacosArtifactError::InvalidCrs);
        }
        codes.push(code);
        index = cursor;
    }
    Ok(codes)
}

fn grid_bounds(
    width: u64,
    height: u64,
    x_first: f64,
    y_first: f64,
    x_step: f64,
    y_step: f64,
) -> Result<[f64; 4], GacosArtifactError> {
    if width == 0
        || height == 0
        || !x_first.is_finite()
        || !y_first.is_finite()
        || !x_step.is_finite()
        || !y_step.is_finite()
        || x_step <= 0.0
        || y_step >= 0.0
    {
        return Err(GacosArtifactError::InvalidBinaryGeoreference);
    }
    // TRAIN reads GACOS as a gridline-registered raster: X_FIRST/Y_FIRST
    // identify the first sample and the outer coordinate of the last sample is
    // `(dimension - 1) * step` away, not `dimension * step` away.
    let east = x_first + x_step * (width - 1) as f64;
    let south = y_first + y_step * (height - 1) as f64;
    if !east.is_finite()
        || !south.is_finite()
        || x_first < -180.0
        || east > 180.0
        || south < -90.0
        || y_first > 90.0
        || east < x_first
        || y_first < south
    {
        return Err(GacosArtifactError::InvalidBinaryGeoreference);
    }
    Ok([x_first, south, east, y_first])
}

/// Validate GeoTIFF outer pixel-edge bounds. Unlike the binary GACOS
/// gridline coordinates above, a native raster inspection reports the outer
/// edge, so the span is `dimension * pixel_step`.
fn validate_raster_georeference(
    facts: &GacosRasterInspectionFacts,
    frozen_bbox: &GacosBoundingBox,
) -> Result<(), GacosArtifactError> {
    let [west, south, east, north] = facts.bounds;
    if !facts.x_step.is_finite()
        || !facts.y_step.is_finite()
        || facts.x_step <= 0.0
        || facts.y_step >= 0.0
        || ![west, south, east, north]
            .iter()
            .all(|value| value.is_finite())
        || west < -180.0
        || east > 180.0
        || south < -90.0
        || north > 90.0
        || west >= east
        || south >= north
    {
        return Err(GacosArtifactError::InvalidRasterGeoreference);
    }
    let expected_width = facts.x_step * facts.width as f64;
    let expected_height = facts.y_step.abs() * facts.height as f64;
    if !approximately_equal(east - west, expected_width)
        || !approximately_equal(north - south, expected_height)
    {
        return Err(GacosArtifactError::InvalidRasterGeoreference);
    }
    if !bounds_cover_request(facts.bounds, frozen_bbox) {
        return Err(GacosArtifactError::BoundingBoxMismatch);
    }
    Ok(())
}

fn approximately_equal(left: f64, right: f64) -> bool {
    let tolerance = GRID_COORDINATE_EPSILON * left.abs().max(right.abs()).max(1.0);
    (left - right).abs() <= tolerance
}

fn bounds_cover_request(bounds: [f64; 4], request: &GacosBoundingBox) -> bool {
    bounds[0] <= request.west + GRID_COORDINATE_EPSILON
        && bounds[1] <= request.south + GRID_COORDINATE_EPSILON
        && bounds[2] + GRID_COORDINATE_EPSILON >= request.east
        && bounds[3] + GRID_COORDINATE_EPSILON >= request.north
}

fn validate_frozen_bbox(bbox: &GacosBoundingBox) -> Result<(), GacosArtifactError> {
    bbox.validate()
        .map_err(|_| GacosArtifactError::InvalidBinaryGeoreference)
}

fn validate_artifact_size_u64(size: u64) -> Result<(), GacosArtifactError> {
    if size == 0 {
        return Err(GacosArtifactError::EmptyArtifact);
    }
    if size > GACOS_MAX_ARTIFACT_BYTES {
        return Err(GacosArtifactError::ArtifactTooLarge);
    }
    Ok(())
}

fn validate_header_evidence(
    header_size_bytes: u64,
    header_bytes: &[u8],
) -> Result<(), GacosArtifactError> {
    if header_size_bytes > GACOS_MAX_RSC_HEADER_BYTES as u64
        || header_bytes.len() > GACOS_MAX_RSC_HEADER_BYTES
    {
        return Err(GacosArtifactError::HeaderTooLarge);
    }
    validate_artifact_size_u64(header_size_bytes)?;
    if header_size_bytes != header_bytes.len() as u64 {
        return Err(GacosArtifactError::HeaderEvidenceIncomplete);
    }
    Ok(())
}

fn validate_tiff_prefix(
    tiff_size_bytes: u64,
    tiff_prefix: &[u8],
) -> Result<(), GacosArtifactError> {
    if tiff_prefix.len() > GACOS_TIFF_PREFIX_BYTES || tiff_prefix.len() as u64 > tiff_size_bytes {
        return Err(GacosArtifactError::RasterPrefixTooLarge);
    }
    let required = tiff_size_bytes.min(GACOS_TIFF_PREFIX_BYTES as u64) as usize;
    if tiff_prefix.len() < required {
        return Err(GacosArtifactError::RasterPrefixIncomplete);
    }
    Ok(())
}

fn is_tiff_signature(bytes: &[u8]) -> bool {
    bytes.starts_with(b"II*\0")
        || bytes.starts_with(b"MM\0*")
        || bytes.starts_with(b"II+\0")
        || bytes.starts_with(b"MM\0+")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2024, 3, 12).expect("fixture date")
    }

    fn bbox() -> GacosBoundingBox {
        GacosBoundingBox::new(110.0, 30.0, 110.002, 30.002)
    }

    fn binary_bbox() -> GacosBoundingBox {
        // A 2x2 gridline-registered fixture spans one step in each direction.
        GacosBoundingBox::new(110.0, 30.001, 110.001, 30.002)
    }

    fn utc() -> GacosUtcTime {
        GacosUtcTime::new(10, 26).expect("fixture UTC")
    }

    fn binary_header() -> Vec<u8> {
        // This mirrors the observed TRAIN-compatible RSC fields: no CRS field
        // is fabricated, and tabs are accepted as ordinary header spacing.
        b"WIDTH\t2\nFILE_LENGTH\t2\nX_FIRST\t110.0\nY_FIRST\t30.002\nX_STEP\t0.001\nY_STEP\t-0.001\n".to_vec()
    }

    fn binary_request<'a>(files: &'a [GacosArtifactFile<'a>]) -> GacosArtifactAdmissionRequest<'a> {
        let expected = Box::leak(Box::new([date()]));
        GacosArtifactAdmissionRequest::new(expected, binary_bbox(), utc(), files)
    }

    #[test]
    fn admits_complete_binary_pair_with_scientific_header() {
        let data = [0_u8; 16];
        let header = binary_header();
        let files = [
            GacosArtifactFile::binary_data("20240312.ztd", &data),
            GacosArtifactFile::binary_header("20240312.ztd.rsc", &header),
        ];
        let admission = admit_gacos_artifacts(&binary_request(&files)).expect("binary admission");
        assert_eq!(admission.products.len(), 1);
        assert_eq!(admission.products[0].kind, GacosArtifactKind::Binary);
        assert_eq!(admission.products[0].size_bytes, 16);
        let admitted_header = admission.products[0].binary_header.as_ref().unwrap();
        assert_eq!(admitted_header.width, 2);
        assert_eq!(admitted_header.header_declared, None);
        assert!(admitted_header.provider_contract_geographic);
        assert_eq!(admission.requested_utc, utc());
        assert_eq!(admission.observed_utc, None);
        let bounds = admitted_header.grid_bounds().expect("gridline bounds");
        for (actual, expected) in bounds.into_iter().zip([110.0, 30.001, 110.001, 30.002]) {
            assert!((actual - expected).abs() < 1e-12);
        }
    }

    #[test]
    fn evidence_admits_eight_gib_binary_without_payload_allocation() {
        let header = b"WIDTH 65536\nFILE_LENGTH 32768\nX_FIRST 110.0\nY_FIRST 30.002\nX_STEP .000001\nY_STEP -.000001\n";
        let files = [
            GacosArtifactEvidence::binary_data("20240312.ztd", GACOS_MAX_ARTIFACT_BYTES),
            GacosArtifactEvidence::binary_header("20240312.ztd.rsc", header.len() as u64, header),
        ];
        let dates = [date()];
        let request = GacosArtifactEvidenceRequest::new(&dates, binary_bbox(), utc(), &files);
        let admission = admit_gacos_artifact_evidence(&request).expect("large evidence admission");
        assert_eq!(admission.products[0].size_bytes, GACOS_MAX_ARTIFACT_BYTES);
    }

    #[test]
    fn evidence_requires_complete_bounded_header_without_allocating_source_file() {
        let header = binary_header();
        let files = [
            GacosArtifactEvidence::binary_data("20240312.ztd", 16),
            GacosArtifactEvidence::binary_header(
                "20240312.ztd.rsc",
                GACOS_MAX_RSC_HEADER_BYTES as u64,
                &header,
            ),
        ];
        let dates = [date()];
        let request = GacosArtifactEvidenceRequest::new(&dates, binary_bbox(), utc(), &files);
        assert_eq!(
            admit_gacos_artifact_evidence(&request),
            Err(GacosArtifactError::HeaderEvidenceIncomplete)
        );
    }

    #[test]
    fn evidence_admits_large_geotiff_from_eight_byte_prefix() {
        let facts = GacosRasterInspectionFacts::new(
            "GTiff",
            2,
            2,
            1,
            vec!["float32".to_owned()],
            "EPSG:4326",
            [110.0, 30.0, 110.002, 30.002],
            0.001,
            -0.001,
        );
        let prefix = b"II*\0\x08\0\0\0";
        let files = [GacosArtifactEvidence::geotiff(
            "20240312.tif",
            GACOS_MAX_ARTIFACT_BYTES,
            prefix,
            &facts,
        )];
        let dates = [date()];
        let request = GacosArtifactEvidenceRequest::new(&dates, bbox(), utc(), &files);
        let admission = admit_gacos_artifact_evidence(&request).expect("large TIFF evidence");
        assert_eq!(admission.products[0].size_bytes, GACOS_MAX_ARTIFACT_BYTES);
    }

    #[test]
    fn evidence_file_and_date_bounds_are_enforced_without_payloads() {
        let too_many_files = [GacosArtifactEvidence::binary_data("invalid", 1); 41];
        let dates = [date()];
        let file_request =
            GacosArtifactEvidenceRequest::new(&dates, binary_bbox(), utc(), &too_many_files);
        assert_eq!(
            admit_gacos_artifact_evidence(&file_request),
            Err(GacosArtifactError::TooManyFiles)
        );

        let dates = (1..=21)
            .map(|day| NaiveDate::from_ymd_opt(2024, 1, day).expect("fixture date"))
            .collect::<Vec<_>>();
        let no_files: [GacosArtifactEvidence<'static>; 0] = [];
        let date_request =
            GacosArtifactEvidenceRequest::new(&dates, binary_bbox(), utc(), &no_files);
        assert_eq!(
            admit_gacos_artifact_evidence(&date_request),
            Err(GacosArtifactError::TooManyExpectedDates)
        );
    }

    #[test]
    fn missing_dimensions_cannot_bypass_size_validation() {
        let data = [0_u8; 16];
        let header = b"X_FIRST 110.0\nY_FIRST 30.002\nX_STEP 0.001\nY_STEP -0.001\nCRS EPSG:4326\n";
        let files = [
            GacosArtifactFile::binary_data("20240312.ztd", &data),
            GacosArtifactFile::binary_header("20240312.ztd.rsc", header),
        ];
        assert_eq!(
            admit_gacos_artifacts(&binary_request(&files)),
            Err(GacosArtifactError::MissingHeaderField("WIDTH"))
        );
    }

    #[test]
    fn rejects_invalid_utf8_and_oversized_headers() {
        let invalid = [0xff_u8];
        assert_eq!(
            parse_gacos_binary_header(&invalid),
            Err(GacosArtifactError::HeaderNotUtf8)
        );
        let oversized = vec![b'x'; GACOS_MAX_RSC_HEADER_BYTES + 1];
        assert_eq!(
            parse_gacos_binary_header(&oversized),
            Err(GacosArtifactError::HeaderTooLarge)
        );
    }

    #[test]
    fn rejects_duplicate_header_fields() {
        let header = b"WIDTH 2\nWIDTH 2\nFILE_LENGTH 2\nX_FIRST 110\nY_FIRST 30.002\nX_STEP .001\nY_STEP -.001\nCRS EPSG:4326\n";
        assert_eq!(
            parse_gacos_binary_header(header),
            Err(GacosArtifactError::DuplicateHeaderField)
        );
    }

    #[test]
    fn accepts_length_alias_and_equal_duplicate_but_rejects_conflict() {
        let length_only =
            b"WIDTH\t2\nLENGTH\t2\nX_FIRST\t110\nY_FIRST\t30.002\nX_STEP\t.001\nY_STEP\t-.001\n";
        let parsed = parse_gacos_binary_header(length_only).expect("LENGTH alias");
        assert_eq!(parsed.file_length, 2);
        assert_eq!(parsed.header_declared, None);

        let equal_aliases = b"WIDTH 2\nFILE_LENGTH 2\nLENGTH 2\nX_FIRST 110\nY_FIRST 30.002\nX_STEP .001\nY_STEP -.001\n";
        assert_eq!(
            parse_gacos_binary_header(equal_aliases)
                .expect("equal aliases")
                .file_length,
            2
        );

        let conflicting = b"WIDTH 2\nFILE_LENGTH 2\nLENGTH 3\nX_FIRST 110\nY_FIRST 30.002\nX_STEP .001\nY_STEP -.001\n";
        assert_eq!(
            parse_gacos_binary_header(conflicting),
            Err(GacosArtifactError::ConflictingLengthAliases)
        );
    }

    #[test]
    fn rejects_explicit_conflicting_crs_even_with_wgs84_declaration() {
        let header = b"WIDTH 2\nFILE_LENGTH 2\nX_FIRST 110\nY_FIRST 30.002\nX_STEP .001\nY_STEP -.001\nCRS EPSG:4326\nEPSG 3857\n";
        assert_eq!(
            parse_gacos_binary_header(header),
            Err(GacosArtifactError::InvalidCrs)
        );
    }

    #[test]
    fn rejects_size_mismatch_and_overflow() {
        let data = [0_u8; 12];
        let header = binary_header();
        let files = [
            GacosArtifactFile::binary_data("20240312.ztd", &data),
            GacosArtifactFile::binary_header("20240312.ztd.rsc", &header),
        ];
        assert_eq!(
            admit_gacos_artifacts(&binary_request(&files)),
            Err(GacosArtifactError::SizeMismatch)
        );
        let overflow = b"WIDTH 18446744073709551615\nFILE_LENGTH 2\nX_FIRST 110\nY_FIRST 30.002\nX_STEP .001\nY_STEP -.001\nCRS EPSG:4326\n";
        assert_eq!(
            parse_gacos_binary_header(overflow),
            Err(GacosArtifactError::InvalidBinaryGeoreference)
        );
    }

    #[test]
    fn rejects_missing_duplicate_and_extra_dates() {
        let data = [0_u8; 16];
        let header = binary_header();
        let duplicate_files = [
            GacosArtifactFile::binary_data("20240312.ztd", &data),
            GacosArtifactFile::binary_data("20240312.ztd", &data),
            GacosArtifactFile::binary_header("20240312.ztd.rsc", &header),
        ];
        assert_eq!(
            admit_gacos_artifacts(&binary_request(&duplicate_files)),
            Err(GacosArtifactError::DuplicateBinaryPart)
        );

        let missing_files = [GacosArtifactFile::binary_data("20240312.ztd", &data)];
        assert_eq!(
            admit_gacos_artifacts(&binary_request(&missing_files)),
            Err(GacosArtifactError::MissingBinaryPart)
        );

        let missing_date_files = [
            GacosArtifactFile::binary_data("20240313.ztd", &data),
            GacosArtifactFile::binary_header("20240313.ztd.rsc", &header),
        ];
        assert_eq!(
            admit_gacos_artifacts(&binary_request(&missing_date_files)),
            Err(GacosArtifactError::MissingDate)
        );

        let extra_files = [
            GacosArtifactFile::binary_data("20240312.ztd", &data),
            GacosArtifactFile::binary_header("20240312.ztd.rsc", &header),
            GacosArtifactFile::binary_data("20240313.ztd", &data),
            GacosArtifactFile::binary_header("20240313.ztd.rsc", &header),
        ];
        assert_eq!(
            admit_gacos_artifacts(&binary_request(&extra_files)),
            Err(GacosArtifactError::ExtraDate)
        );
    }

    #[test]
    fn rejects_path_traversal_and_extra_executable_files() {
        let data = [0_u8; 16];
        let header = binary_header();
        let traversal = [GacosArtifactFile::binary_data("../20240312.ztd", &data)];
        assert_eq!(
            admit_gacos_artifacts(&binary_request(&traversal)),
            Err(GacosArtifactError::PathTraversal)
        );
        let executable = [GacosArtifactFile::binary_data("20240312.exe", &data)];
        assert_eq!(
            admit_gacos_artifacts(&binary_request(&executable)),
            Err(GacosArtifactError::UnsupportedFile)
        );
        let valid = [
            GacosArtifactFile::binary_data("20240312.ztd", &data),
            GacosArtifactFile::binary_header("20240312.ztd.rsc", &header),
            GacosArtifactFile::binary_data("notes.exe", &[]),
        ];
        assert_eq!(
            admit_gacos_artifacts(&binary_request(&valid)),
            Err(GacosArtifactError::UnsupportedFile)
        );
    }

    #[test]
    fn admits_geotiff_only_with_native_facts() {
        let facts = GacosRasterInspectionFacts::new(
            "GTiff",
            2,
            2,
            1,
            vec!["float32".to_owned()],
            "EPSG:4326",
            [110.0, 30.0, 110.002, 30.002],
            0.001,
            -0.001,
        );
        let bytes = b"II*\0native-inspection-required";
        let files = [GacosArtifactFile::geotiff("20240312.tif", bytes, &facts)];
        let admission = admit_gacos_artifacts(&GacosArtifactAdmissionRequest::new(
            &[date()],
            bbox(),
            utc(),
            &files,
        ))
        .expect("GeoTIFF admission");
        assert_eq!(admission.products[0].kind, GacosArtifactKind::GeoTiff);
    }

    #[test]
    fn geotiff_magic_without_native_facts_is_rejected() {
        let files = [GacosArtifactFile {
            name: "20240312.tif",
            bytes: b"II*\0",
            raster_inspection: None,
        }];
        assert_eq!(
            admit_gacos_artifacts(&GacosArtifactAdmissionRequest::new(
                &[date()],
                bbox(),
                utc(),
                &files,
            )),
            Err(GacosArtifactError::MissingRasterInspection)
        );
    }

    #[test]
    fn geotiff_facts_require_crs_dimensions_dtype_and_bbox() {
        let bytes = b"II*\0payload";
        let mut facts = GacosRasterInspectionFacts::new(
            "GTiff",
            2,
            2,
            1,
            vec!["uint16".to_owned()],
            "EPSG:3857",
            [110.0, 30.0, 110.002, 30.002],
            0.001,
            -0.001,
        );
        assert_eq!(
            validate_gacos_geotiff_facts(&facts, bytes, &bbox()),
            Err(GacosArtifactError::InvalidRasterDataType)
        );
        facts.dtypes = vec!["float32".to_owned()];
        assert_eq!(
            validate_gacos_geotiff_facts(&facts, bytes, &bbox()),
            Err(GacosArtifactError::InvalidRasterCrs)
        );
        facts.crs = "EPSG:4326 EPSG:3857".to_owned();
        assert_eq!(
            validate_gacos_geotiff_facts(&facts, bytes, &bbox()),
            Err(GacosArtifactError::InvalidRasterCrs)
        );
        facts.crs = "EPSG:4326".to_owned();
        facts.bounds = [111.0, 30.0, 111.002, 30.002];
        assert_eq!(
            validate_gacos_geotiff_facts(&facts, bytes, &bbox()),
            Err(GacosArtifactError::BoundingBoxMismatch)
        );
    }

    #[test]
    fn wkt_crs_uses_outer_geographic_identity_and_rejects_projected_root() {
        let bytes = b"II*\0payload";
        let geographic_wkt = r#"GEOGCRS["WGS 84",DATUM["World Geodetic System 1984",ELLIPSOID["WGS 84",6378137,298.257223563,AUTHORITY["EPSG","7030"]],AUTHORITY["EPSG","6326"]],CS[ellipsoidal,2],AXIS["geodetic latitude (Lat)",north,ORDER[1],ANGLEUNIT["degree",0.0174532925199433,AUTHORITY["EPSG","9122"]]],AXIS["geodetic longitude (Lon)",east,ORDER[2],ANGLEUNIT["degree",0.0174532925199433,AUTHORITY["EPSG","9122"]]],ID["EPSG",4326]]"#;
        let projected_wkt = r#"PROJCRS["WGS 84 / Pseudo-Mercator",BASEGEOGCRS["WGS 84",DATUM["World Geodetic System 1984",ID["EPSG",6326]],ID["EPSG",4326]],CONVERSION["Popular Visualisation Pseudo-Mercator"],CS[Cartesian,2],AXIS["easting",east,ORDER[1]],AXIS["northing",north,ORDER[2]],ID["EPSG",3857]]"#;
        let mut facts = GacosRasterInspectionFacts::new(
            "GTiff",
            2,
            2,
            1,
            vec!["float32".to_owned()],
            geographic_wkt,
            [110.0, 30.0, 110.002, 30.002],
            0.001,
            -0.001,
        );
        assert!(validate_gacos_geotiff_facts(&facts, bytes, &bbox()).is_ok());
        facts.crs = projected_wkt.to_owned();
        assert_eq!(
            validate_gacos_geotiff_facts(&facts, bytes, &bbox()),
            Err(GacosArtifactError::InvalidRasterCrs)
        );
    }

    #[test]
    fn binary_grid_requires_provider_geographic_contract_evidence() {
        let header = GacosBinaryHeader {
            width: 2,
            file_length: 2,
            x_first: 110.0,
            y_first: 30.002,
            x_step: 0.001,
            y_step: -0.001,
            header_declared: None,
            provider_contract_geographic: false,
        };
        assert_eq!(
            validate_gacos_binary_grid(&header, 16, &binary_bbox()),
            Err(GacosArtifactError::InvalidCrs)
        );
    }

    #[test]
    fn binary_grid_requires_wgs84_and_frozen_bbox_coverage() {
        let header = GacosBinaryHeader {
            width: 2,
            file_length: 2,
            x_first: 111.0,
            y_first: 30.002,
            x_step: 0.001,
            y_step: -0.001,
            header_declared: Some("EPSG:3857".to_owned()),
            provider_contract_geographic: true,
        };
        assert_eq!(
            validate_gacos_binary_grid(&header, 16, &bbox()),
            Err(GacosArtifactError::InvalidCrs)
        );
        let header = GacosBinaryHeader {
            header_declared: None,
            ..header
        };
        assert_eq!(
            validate_gacos_binary_grid(&header, 16, &bbox()),
            Err(GacosArtifactError::BoundingBoxMismatch)
        );
    }
}
