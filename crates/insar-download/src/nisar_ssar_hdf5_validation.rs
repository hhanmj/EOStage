//! SSAR RSLC HDF5 artifact validation.
//!
//! This module deliberately stays separate from the LSAR validator. The
//! catalog's `CP` polarization and `W` mode are provider fields; they are
//! checked as frozen inputs but are not converted into the LSAR polarization
//! enum. The science declaration comes from the S-band filename and the
//! HDF5 `listOfPolarizations` dataset. The official Chapter 5 document places
//! the processing `softwareVersion` under
//! `/science/SSAR/RSLC/processingInformation/algorithms`, while the
//! identification `productVersion` is a separate field. This preparation
//! therefore keeps the provider catalog spelling opaque and does not compare
//! it to `productVersion`.

use std::path::Path;
use std::time::{Duration, Instant};

use hdf5::{File, Group};

use crate::artifact_validation::{ArtifactValidationError, ArtifactValidationIssue};
use crate::nisar_hdf5_validation::{
    bounded_extents, checked_link_exists, read_fixed_list, read_fixed_scalar, required_dataset,
    validate_derived_hdf5_security, validate_image_dataset, Deadline, StringBudget,
};

const IDENTIFICATION_PATH: &str = "/science/SSAR/identification";
const SSAR_ROOT_PATH: &str = "/science/SSAR";
const RSLC_PATH: &str = "/science/SSAR/RSLC";
const SWATHS_PATH: &str = "/science/SSAR/RSLC/swaths";
const FREQUENCY_A_PATH: &str = "/science/SSAR/RSLC/swaths/frequencyA";
const GEOLOCATION_PATH: &str = "/science/SSAR/RSLC/metadata/geolocationGrid";
// The reviewed SSAR metadata body contains productVersion=1.2.1. Keep this
// HDF5 identity value independent from the provider's SoftwareVersion field.
const HDF5_PRODUCT_VERSION: &str = "1.2.1";
const VALIDATION_DEADLINE: Duration = Duration::from_secs(2);

/// The S-band channel names permitted by the frozen filename contract.
///
/// This is intentionally a distinct type from [`crate::NisarPolarization`].
/// In particular, the provider's catalog value `CP` remains opaque and does
/// not imply `HH`/`HV`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum NisarSsarPolarization {
    Hh,
    Hv,
    Vv,
    Vh,
    Lh,
    Lv,
    Rh,
    Rv,
}

impl NisarSsarPolarization {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Hh => "HH",
            Self::Hv => "HV",
            Self::Vv => "VV",
            Self::Vh => "VH",
            Self::Lh => "LH",
            Self::Lv => "LV",
            Self::Rh => "RH",
            Self::Rv => "RV",
        }
    }

    const fn all() -> [Self; 8] {
        [
            Self::Hh,
            Self::Hv,
            Self::Vv,
            Self::Vh,
            Self::Lh,
            Self::Lv,
            Self::Rh,
            Self::Rv,
        ]
    }
}

/// Frozen provider identity plus the S-band science channels declared by its
/// filename.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NisarSsarRslcValidationContext {
    item_id: String,
    filename: String,
    expected_size: Option<u64>,
    /// The provider's catalog spelling is retained as evidence. It is not
    /// interpreted as the HDF5 `productVersion` field.
    catalog_software_version: String,
    product_version: String,
    polarizations: Vec<NisarSsarPolarization>,
}

impl NisarSsarRslcValidationContext {
    /// Build context from the reviewed Bhoonidhi S-RSLC catalog fields.
    ///
    /// `catalog_software_version` must be the exact reviewed catalog spelling
    /// `1.4.00`. It is retained as provider evidence and is not used to derive
    /// the HDF5 `productVersion` expectation. `mode` is compared with the
    /// reviewed catalogue value. Provider `polarization` is identity evidence:
    /// it must agree with the filename POLE token (for example DHNA -> DH),
    /// while the actual HDF5 science-channel set is still derived only from
    /// that reviewed filename token. The coverage flag is likewise compared
    /// with the filename's official `F`/`P` coverage token.
    #[doc(hidden)]
    pub fn from_frozen_bhoonidhi(
        item_id: impl Into<String>,
        filename: impl Into<String>,
        expected_size: u64,
        catalog_software_version: &str,
        mode: &str,
        polarization: &str,
        provider_partial: bool,
    ) -> Result<Self, ArtifactValidationError> {
        if expected_size == 0 {
            return Err(invalid_context());
        }
        Self::build(
            item_id.into(),
            filename.into(),
            Some(expected_size),
            catalog_software_version,
            mode,
            polarization,
            provider_partial,
        )
    }

    /// Build context before the download layer has obtained the content
    /// length. Validation fails closed until a positive size is supplied by
    /// the caller.
    #[doc(hidden)]
    pub fn from_frozen_bhoonidhi_deferred_size(
        item_id: impl Into<String>,
        filename: impl Into<String>,
        catalog_software_version: &str,
        mode: &str,
        polarization: &str,
        provider_partial: bool,
    ) -> Result<Self, ArtifactValidationError> {
        Self::build(
            item_id.into(),
            filename.into(),
            None,
            catalog_software_version,
            mode,
            polarization,
            provider_partial,
        )
    }

    fn build(
        item_id: String,
        filename: String,
        expected_size: Option<u64>,
        catalog_software_version: &str,
        mode: &str,
        polarization: &str,
        provider_partial: bool,
    ) -> Result<Self, ArtifactValidationError> {
        let identity = SsarFilenameIdentity::parse(&item_id).ok_or_else(invalid_context)?;
        let catalog_software_version = validate_catalog_software_version(catalog_software_version)
            .ok_or_else(invalid_context)?
            .to_owned();
        if filename != format!("{item_id}.h5")
            || mode != "W"
            || polarization != identity.provider_polarization
            || identity.coverage_partial != provider_partial
        {
            return Err(invalid_context());
        }
        Ok(Self {
            item_id,
            filename,
            expected_size,
            catalog_software_version,
            // The reviewed live catalog records `SoftwareVersion=1.4.00`,
            // while the real SSAR metadata body contains
            // `productVersion=1.2.1`. Keep those fields independent.
            product_version: HDF5_PRODUCT_VERSION.to_owned(),
            polarizations: identity.polarizations,
        })
    }

    pub fn item_id(&self) -> &str {
        &self.item_id
    }

    pub fn filename(&self) -> &str {
        &self.filename
    }

    pub const fn expected_size(&self) -> Option<u64> {
        self.expected_size
    }

    pub fn product_version(&self) -> &str {
        &self.product_version
    }

    /// Return the exact provider serialization supplied to the constructor.
    /// This is deliberately not normalized to [`Self::product_version`].
    pub fn catalog_software_version(&self) -> &str {
        &self.catalog_software_version
    }

    pub fn subset_polarization_labels(&self) -> Vec<String> {
        self.polarizations
            .iter()
            .map(|polarization| polarization.as_str().to_owned())
            .collect()
    }

    #[allow(dead_code)]
    pub(crate) fn polarizations(&self) -> &[NisarSsarPolarization] {
        &self.polarizations
    }
}

struct SsarFilenameIdentity {
    polarizations: Vec<NisarSsarPolarization>,
    provider_polarization: &'static str,
    coverage_partial: bool,
}

impl SsarFilenameIdentity {
    fn parse(value: &str) -> Option<Self> {
        if value.is_empty()
            || value.len() > 512
            || !value.is_ascii()
            || value.bytes().any(|byte| byte.is_ascii_control())
        {
            return None;
        }
        let parts = value.split('_').collect::<Vec<_>>();
        if parts.len() != 18 || parts[0] != "NISAR" || parts[1] != "S1" || parts[3] != "RSLC" {
            return None;
        }
        if parts.iter().any(|part| part.is_empty())
            || parts[13] != "P00500"
            || !matches!(parts[15], "F" | "P")
        {
            return None;
        }
        let (polarizations, provider_polarization) = parse_polarization_code(parts[9])?;
        Some(Self {
            polarizations,
            provider_polarization,
            coverage_partial: parts[15] == "P",
        })
    }
}

fn parse_polarization_code(code: &str) -> Option<(Vec<NisarSsarPolarization>, &'static str)> {
    match code {
        "SHNA" => Some((vec![NisarSsarPolarization::Hh], "SH")),
        "SVNA" => Some((vec![NisarSsarPolarization::Vv], "SV")),
        "DHNA" => Some((
            vec![NisarSsarPolarization::Hh, NisarSsarPolarization::Hv],
            "DH",
        )),
        "DVNA" => Some((
            vec![NisarSsarPolarization::Vv, NisarSsarPolarization::Vh],
            "DV",
        )),
        "CLNA" => Some((
            vec![NisarSsarPolarization::Lh, NisarSsarPolarization::Lv],
            "CP",
        )),
        "CRNA" => Some((
            vec![NisarSsarPolarization::Rh, NisarSsarPolarization::Rv],
            "CP",
        )),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug)]
pub struct NisarSsarHdf5Validator {
    deadline: Duration,
}

impl NisarSsarHdf5Validator {
    pub const fn new() -> Self {
        Self {
            deadline: VALIDATION_DEADLINE,
        }
    }

    /// Validate a completed or `.part` HDF5 artifact with cooperative
    /// cancellation. The final byte count is checked before any HDF5 open.
    #[doc(hidden)]
    pub fn validate_path_with_cancel(
        &self,
        candidate_path: &Path,
        context: &NisarSsarRslcValidationContext,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<(), ArtifactValidationError> {
        let expires_at = Instant::now()
            .checked_add(self.deadline)
            .ok_or_else(|| invalid(ArtifactValidationIssue::DeadlineExceeded))?;
        check_cancelled(cancelled)?;

        let metadata = std::fs::metadata(candidate_path).map_err(|_| {
            crate::ArtifactValidationError::local_read_failed(ArtifactValidationIssue::CandidateIo)
        })?;
        let expected_size = context
            .expected_size
            .ok_or_else(|| invalid(ArtifactValidationIssue::FinalSizeMismatch))?;
        if !metadata.is_file() || metadata.len() != expected_size {
            return Err(invalid(ArtifactValidationIssue::FinalSizeMismatch));
        }
        let candidate_name = candidate_path.file_name().and_then(|value| value.to_str());
        if candidate_name != Some(context.filename.as_str())
            && candidate_name.and_then(|value| value.strip_suffix(".part"))
                != Some(context.filename.as_str())
        {
            return Err(invalid(ArtifactValidationIssue::IdentificationMismatch));
        }

        // This is the shared full-graph guard: plugin loading, hard-link-only
        // traversal, aliases/cycles, bounded shapes, filters and external
        // storage are all checked before any semantic dataset reads.
        validate_derived_hdf5_security(candidate_path)?;
        check_stage(expires_at, cancelled)?;

        let file = crate::hdf5_file::open(candidate_path)
            .map_err(|error| crate::nisar_hdf5_validation::hdf5_open_error(&error))?;
        if !file.is_read_only() {
            return Err(invalid(ArtifactValidationIssue::Hdf5Open));
        }
        let deadline = Deadline::until(expires_at);
        validate_ssar_file(&file, context, &deadline, expires_at, cancelled)
    }
}

impl crate::artifact_validation::CompletedArtifactValidator for NisarSsarHdf5Validator {
    fn id(&self) -> crate::ArtifactValidatorId {
        crate::NISAR_SSAR_RSLC_STRUCTURE_VALIDATOR_ID
    }

    fn version(&self) -> u32 {
        crate::NISAR_SSAR_RSLC_STRUCTURE_VALIDATOR_VERSION
    }

    fn validate(
        &self,
        candidate_path: &Path,
        asset: &crate::ResolvedAsset,
        context: &crate::ArtifactValidationContext,
    ) -> Result<(), ArtifactValidationError> {
        let context = context.as_nisar_ssar_rslc().ok_or_else(invalid_context)?;
        let size = asset
            .expected_size()
            .filter(|size| *size > 0)
            .ok_or_else(invalid_context)?;
        if context.expected_size.is_some_and(|frozen| frozen != size) {
            return Err(invalid_context());
        }
        // Use the runtime source length obtained before transfer. Local file
        // length is only a postcondition and never supplies missing authority.
        let mut resolved_context = context.clone();
        resolved_context.expected_size = Some(size);
        self.validate_path_with_cancel(candidate_path, &resolved_context, &|| false)
    }
}

impl Default for NisarSsarHdf5Validator {
    fn default() -> Self {
        Self::new()
    }
}

fn validate_ssar_file(
    file: &File,
    context: &NisarSsarRslcValidationContext,
    deadline: &Deadline,
    expires_at: Instant,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<(), ArtifactValidationError> {
    for path in [
        "/science",
        SSAR_ROOT_PATH,
        IDENTIFICATION_PATH,
        RSLC_PATH,
        SWATHS_PATH,
        "/science/SSAR/RSLC/metadata",
        GEOLOCATION_PATH,
    ] {
        file.group(path)
            .map_err(|_| invalid(ArtifactValidationIssue::RequiredObjectMissing))?;
        check_stage(expires_at, cancelled)?;
    }
    if checked_link_exists(
        file.id(),
        "/science/LSAR",
        ArtifactValidationIssue::IdentificationMismatch,
    )? {
        return Err(invalid(ArtifactValidationIssue::IdentificationMismatch));
    }

    let identification = file
        .group(IDENTIFICATION_PATH)
        .map_err(|_| invalid(ArtifactValidationIssue::RequiredObjectMissing))?;
    let mut strings = StringBudget::default();
    for (name, expected) in [
        ("missionId", "NISAR"),
        ("radarBand", "S"),
        ("productType", "RSLC"),
        ("productLevel", "L1"),
        ("granuleId", context.item_id.as_str()),
        ("productSpecificationVersion", "1.2.1"),
        ("compositeReleaseId", "P00500"),
        ("productVersion", context.product_version.as_str()),
    ] {
        let value = read_fixed_scalar(
            &required_dataset(
                &identification,
                name,
                ArtifactValidationIssue::IdentificationMismatch,
            )?,
            &mut strings,
            ArtifactValidationIssue::IdentificationMismatch,
        )?;
        if value != expected {
            return Err(invalid(ArtifactValidationIssue::IdentificationMismatch));
        }
        check_stage(expires_at, cancelled)?;
    }

    let frequencies = read_fixed_list(
        &required_dataset(
            &identification,
            "listOfFrequencies",
            ArtifactValidationIssue::FrequencyMismatch,
        )?,
        &mut strings,
        ArtifactValidationIssue::FrequencyMismatch,
    )?;
    if frequencies.as_slice() != ["A"] {
        return Err(invalid(ArtifactValidationIssue::FrequencyMismatch));
    }
    check_stage(expires_at, cancelled)?;

    let swaths = file
        .group(SWATHS_PATH)
        .map_err(|_| invalid(ArtifactValidationIssue::RequiredObjectMissing))?;
    if checked_link_exists(
        swaths.id(),
        "frequencyB",
        ArtifactValidationIssue::FrequencyMismatch,
    )? {
        return Err(invalid(ArtifactValidationIssue::FrequencyMismatch));
    }

    let frequency = file
        .group(FREQUENCY_A_PATH)
        .map_err(|_| invalid(ArtifactValidationIssue::FrequencyMismatch))?;
    let actual = read_fixed_list(
        &required_dataset(
            &frequency,
            "listOfPolarizations",
            ArtifactValidationIssue::PolarizationMismatch,
        )?,
        &mut strings,
        ArtifactValidationIssue::PolarizationMismatch,
    )?;
    let expected = context
        .polarizations
        .iter()
        .map(|polarization| polarization.as_str().to_owned())
        .collect::<Vec<_>>();
    if actual != expected {
        return Err(invalid(ArtifactValidationIssue::PolarizationMismatch));
    }

    for candidate in NisarSsarPolarization::all() {
        let declared = context.polarizations.contains(&candidate);
        if checked_link_exists(
            frequency.id(),
            candidate.as_str(),
            ArtifactValidationIssue::PolarizationMismatch,
        )? != declared
        {
            return Err(invalid(ArtifactValidationIssue::PolarizationMismatch));
        }
        if declared {
            // The shared helper matches the official H5T_CPX_F32LE
            // compound (`r`/`i`, each 32-bit little-endian float).
            validate_image_dataset(
                &required_dataset(
                    &frequency,
                    candidate.as_str(),
                    ArtifactValidationIssue::ImageDatasetInvalid,
                )?,
                deadline,
            )?;
            check_stage(expires_at, cancelled)?;
        }
    }

    validate_geolocation(file, deadline, expires_at, cancelled)
}

fn validate_geolocation(
    file: &File,
    deadline: &Deadline,
    expires_at: Instant,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<(), ArtifactValidationError> {
    let group = file
        .group(GEOLOCATION_PATH)
        .map_err(|_| invalid(ArtifactValidationIssue::GeolocationGridInvalid))?;
    let epsg = required_dataset(
        &group,
        "epsg",
        ArtifactValidationIssue::GeolocationGridInvalid,
    )?;
    let epsg_extents = bounded_extents(&epsg, ArtifactValidationIssue::GeolocationGridInvalid)?;
    if !epsg_extents.scalar
        || !epsg
            .dtype()
            .map_err(|_| invalid(ArtifactValidationIssue::GeolocationGridInvalid))?
            .is::<i32>()
    {
        return Err(invalid(ArtifactValidationIssue::GeolocationGridInvalid));
    }
    check_stage(expires_at, cancelled)?;

    let coordinate_x = required_float64_shape(&group, "coordinateX", 3)?;
    let coordinate_y = required_float64_shape(&group, "coordinateY", 3)?;
    if coordinate_x != coordinate_y {
        return Err(invalid(ArtifactValidationIssue::GeolocationGridInvalid));
    }
    check_stage(expires_at, cancelled)?;

    // Chapter 5 names these dimensions explicitly: the cube is ordered as
    // (geolocationCubeHeight, geolocationCubeLength, geolocationCubeWidth),
    // while the three one-dimensional datasets carry width, length and
    // height respectively. Only those relationships are enforced; numeric
    // dimension values remain product-specific.
    let slant_range = required_float64_shape(&group, "slantRange", 1)?;
    if slant_range.as_slice() != [coordinate_x[2]] {
        return Err(invalid(ArtifactValidationIssue::GeolocationGridInvalid));
    }
    check_stage(expires_at, cancelled)?;
    let zero_doppler = required_float64_shape(&group, "zeroDopplerTime", 1)?;
    if zero_doppler.as_slice() != [coordinate_x[1]] {
        return Err(invalid(ArtifactValidationIssue::GeolocationGridInvalid));
    }
    check_stage(expires_at, cancelled)?;
    let height = required_float64_shape(&group, "heightAboveEllipsoid", 1)?;
    if height.as_slice() != [coordinate_x[0]] {
        return Err(invalid(ArtifactValidationIssue::GeolocationGridInvalid));
    }
    deadline.check()
}

fn required_float64_shape(
    group: &Group,
    name: &str,
    rank: usize,
) -> Result<Vec<usize>, ArtifactValidationError> {
    let dataset = required_dataset(group, name, ArtifactValidationIssue::GeolocationGridInvalid)?;
    let extents = bounded_extents(&dataset, ArtifactValidationIssue::GeolocationGridInvalid)?;
    if extents.scalar
        || extents.dims.len() != rank
        || extents.dims.contains(&0)
        || !dataset
            .dtype()
            .map_err(|_| invalid(ArtifactValidationIssue::GeolocationGridInvalid))?
            .is::<f64>()
    {
        return Err(invalid(ArtifactValidationIssue::GeolocationGridInvalid));
    }
    Ok(extents.dims)
}

fn validate_catalog_software_version(value: &str) -> Option<&str> {
    // The reviewed live catalog records exactly `SoftwareVersion=1.4.00`.
    // A future provider spelling must be reviewed explicitly; it is not
    // interchangeable with the HDF5 `productVersion` field.
    match value {
        "1.4.00" => Some(value),
        _ => None,
    }
}

fn invalid(issue: ArtifactValidationIssue) -> ArtifactValidationError {
    ArtifactValidationError::structure_invalid(issue)
}

fn invalid_context() -> ArtifactValidationError {
    ArtifactValidationError::unavailable(ArtifactValidationIssue::InvalidContext)
}

fn check_cancelled(cancelled: &(dyn Fn() -> bool + Sync)) -> Result<(), ArtifactValidationError> {
    if cancelled() {
        Err(ArtifactValidationError::unavailable(
            ArtifactValidationIssue::ValidationCancelled,
        ))
    } else {
        Ok(())
    }
}

fn check_stage(
    expires_at: Instant,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<(), ArtifactValidationError> {
    check_cancelled(cancelled)?;
    if Instant::now() >= expires_at {
        Err(invalid(ArtifactValidationIssue::DeadlineExceeded))
    } else {
        Ok(())
    }
}
