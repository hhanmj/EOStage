use std::fmt;
use std::path::Path;
use std::sync::Arc;
#[cfg(test)]
use std::time::Duration;

use insar_core::{DownloadFailure, DownloadOutcome};
use insar_provider::catalog::{NISAR_P05023_CRID, NISAR_P05023_PRODUCT_SPECIFICATION_VERSION};

use crate::nisar_ssar_hdf5_validation::{NisarSsarHdf5Validator, NisarSsarRslcValidationContext};
use crate::{nisar_hdf5_validation::NisarHdf5Validator, part_path, ResolvedAsset, TransferResult};

pub const NISAR_SSAR_RSLC_STRUCTURE_VALIDATOR_ID: ArtifactValidatorId =
    ArtifactValidatorId("nisar.ssar.rslc.structure");
pub const NISAR_SSAR_RSLC_STRUCTURE_VALIDATOR_VERSION: u32 = 1;
pub const NISAR_SSAR_RSLC_STRUCTURE_VALIDATOR_BINDING: ArtifactValidatorBinding =
    ArtifactValidatorBinding {
        id: NISAR_SSAR_RSLC_STRUCTURE_VALIDATOR_ID,
        version: NISAR_SSAR_RSLC_STRUCTURE_VALIDATOR_VERSION,
    };

pub const NISAR_RSLC_STRUCTURE_VALIDATOR_NAME: &str = "nisar.rslc.structure";
pub const NISAR_RSLC_STRUCTURE_VALIDATOR_VERSION: u32 = 1;
pub const NISAR_RSLC_STRUCTURE_VALIDATOR_V2_VERSION: u32 = 2;
const NISAR_RSLC_STRUCTURE_V1_PRODUCT_SPECIFICATION_VERSION: &str = "1.2.1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ArtifactValidatorId(&'static str);

impl ArtifactValidatorId {
    pub const fn as_str(self) -> &'static str {
        self.0
    }

    #[doc(hidden)]
    #[cfg(test)]
    pub(crate) const fn new_for_test(value: &'static str) -> Self {
        Self(value)
    }
}

impl fmt::Display for ArtifactValidatorId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

pub const NISAR_RSLC_STRUCTURE_VALIDATOR_ID: ArtifactValidatorId =
    ArtifactValidatorId(NISAR_RSLC_STRUCTURE_VALIDATOR_NAME);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ArtifactValidatorBinding {
    id: ArtifactValidatorId,
    version: u32,
}

impl ArtifactValidatorBinding {
    pub const fn id(self) -> ArtifactValidatorId {
        self.id
    }

    pub const fn version(self) -> u32 {
        self.version
    }

    #[doc(hidden)]
    #[cfg(test)]
    pub(crate) const fn new_for_test(id: ArtifactValidatorId, version: u32) -> Self {
        Self { id, version }
    }
}

pub const NISAR_RSLC_STRUCTURE_VALIDATOR_BINDING: ArtifactValidatorBinding =
    ArtifactValidatorBinding {
        id: NISAR_RSLC_STRUCTURE_VALIDATOR_ID,
        version: NISAR_RSLC_STRUCTURE_VALIDATOR_VERSION,
    };
pub const NISAR_RSLC_STRUCTURE_VALIDATOR_V2_BINDING: ArtifactValidatorBinding =
    ArtifactValidatorBinding {
        id: NISAR_RSLC_STRUCTURE_VALIDATOR_ID,
        version: NISAR_RSLC_STRUCTURE_VALIDATOR_V2_VERSION,
    };

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum NisarFrequency {
    A,
    B,
}

impl NisarFrequency {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum NisarPolarization {
    Hh,
    Hv,
    Vv,
}

impl NisarPolarization {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Hh => "HH",
            Self::Hv => "HV",
            Self::Vv => "VV",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NisarRslcValidationContext {
    granule_id: String,
    frequency_a: Option<Vec<NisarPolarization>>,
    frequency_b: Option<Vec<NisarPolarization>>,
    crid: Option<String>,
    product_specification_version: String,
    product_specification_profile_registry_version: Option<u32>,
    validator_version: u32,
}

impl NisarRslcValidationContext {
    /// Constructs the private science expectation from fields that have already
    /// passed frozen NISAR policy validation. This does not select a validator.
    #[doc(hidden)]
    pub(crate) fn from_frozen_metadata(
        granule_id: impl Into<String>,
        frequency_a_polarization: Option<&str>,
        frequency_b_polarization: Option<&str>,
    ) -> Result<Self, ArtifactValidationError> {
        Self::from_frozen_parts(
            granule_id,
            frequency_a_polarization,
            frequency_b_polarization,
            None,
            NISAR_RSLC_STRUCTURE_V1_PRODUCT_SPECIFICATION_VERSION,
            None,
            NISAR_RSLC_STRUCTURE_VALIDATOR_VERSION,
        )
    }

    #[doc(hidden)]
    pub(crate) fn from_frozen_product_profile(
        granule_id: impl Into<String>,
        frequency_a_polarization: Option<&str>,
        frequency_b_polarization: Option<&str>,
        crid: &str,
        product_specification_version: &str,
        product_specification_profile_registry_version: u32,
    ) -> Result<Self, ArtifactValidationError> {
        if crid != NISAR_P05023_CRID
            || product_specification_version != NISAR_P05023_PRODUCT_SPECIFICATION_VERSION
            || product_specification_profile_registry_version
                != insar_provider::catalog::NISAR_PRODUCT_SPECIFICATION_PROFILE_REGISTRY_VERSION
        {
            return Err(ArtifactValidationError::unavailable(
                ArtifactValidationIssue::InvalidContext,
            ));
        }
        Self::from_frozen_parts(
            granule_id,
            frequency_a_polarization,
            frequency_b_polarization,
            Some(crid),
            product_specification_version,
            Some(product_specification_profile_registry_version),
            NISAR_RSLC_STRUCTURE_VALIDATOR_V2_VERSION,
        )
    }

    fn from_frozen_parts(
        granule_id: impl Into<String>,
        frequency_a_polarization: Option<&str>,
        frequency_b_polarization: Option<&str>,
        crid: Option<&str>,
        product_specification_version: &str,
        product_specification_profile_registry_version: Option<u32>,
        validator_version: u32,
    ) -> Result<Self, ArtifactValidationError> {
        let granule_id = granule_id.into();
        if granule_id.is_empty()
            || granule_id != granule_id.trim()
            || granule_id.len() > 512
            || granule_id.as_bytes().contains(&0)
        {
            return Err(ArtifactValidationError::unavailable(
                ArtifactValidationIssue::InvalidContext,
            ));
        }
        let frequency_a = parse_polarizations(NisarFrequency::A, frequency_a_polarization)?;
        let frequency_b = parse_polarizations(NisarFrequency::B, frequency_b_polarization)?;
        if frequency_a.is_none() && frequency_b.is_none() {
            return Err(ArtifactValidationError::unavailable(
                ArtifactValidationIssue::InvalidContext,
            ));
        }
        Ok(Self {
            granule_id,
            frequency_a,
            frequency_b,
            crid: crid.map(str::to_owned),
            product_specification_version: product_specification_version.to_owned(),
            product_specification_profile_registry_version,
            validator_version,
        })
    }

    pub fn granule_id(&self) -> &str {
        &self.granule_id
    }

    pub fn polarizations(&self, frequency: NisarFrequency) -> Option<&[NisarPolarization]> {
        match frequency {
            NisarFrequency::A => self.frequency_a.as_deref(),
            NisarFrequency::B => self.frequency_b.as_deref(),
        }
    }

    pub fn has_frequency(&self, frequency: NisarFrequency) -> bool {
        self.polarizations(frequency).is_some()
    }

    pub fn crid(&self) -> Option<&str> {
        self.crid.as_deref()
    }

    pub fn product_specification_version(&self) -> &str {
        &self.product_specification_version
    }

    pub fn product_specification_profile_registry_version(&self) -> Option<u32> {
        self.product_specification_profile_registry_version
    }

    fn matches_binding(&self, binding: ArtifactValidatorBinding) -> bool {
        binding.id == NISAR_RSLC_STRUCTURE_VALIDATOR_ID && binding.version == self.validator_version
    }
}

fn parse_polarizations(
    frequency: NisarFrequency,
    value: Option<&str>,
) -> Result<Option<Vec<NisarPolarization>>, ArtifactValidationError> {
    let parsed = match (frequency, value) {
        (_, None) => return Ok(None),
        (NisarFrequency::A | NisarFrequency::B, Some("HH")) => vec![NisarPolarization::Hh],
        (NisarFrequency::A | NisarFrequency::B, Some("HH+HV")) => {
            vec![NisarPolarization::Hh, NisarPolarization::Hv]
        }
        (NisarFrequency::B, Some("VV")) => vec![NisarPolarization::Vv],
        _ => {
            return Err(ArtifactValidationError::unavailable(
                ArtifactValidationIssue::InvalidContext,
            ))
        }
    };
    Ok(Some(parsed))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArtifactValidationContext {
    NisarRslc(NisarRslcValidationContext),
    NisarSsarRslc(NisarSsarRslcValidationContext),
}

impl From<NisarRslcValidationContext> for ArtifactValidationContext {
    fn from(value: NisarRslcValidationContext) -> Self {
        Self::NisarRslc(value)
    }
}

impl From<NisarSsarRslcValidationContext> for ArtifactValidationContext {
    fn from(value: NisarSsarRslcValidationContext) -> Self {
        Self::NisarSsarRslc(value)
    }
}

impl ArtifactValidationContext {
    pub fn as_nisar_rslc(&self) -> Option<&NisarRslcValidationContext> {
        match self {
            Self::NisarRslc(context) => Some(context),
            Self::NisarSsarRslc(_) => None,
        }
    }

    pub fn as_nisar_ssar_rslc(&self) -> Option<&NisarSsarRslcValidationContext> {
        match self {
            Self::NisarSsarRslc(context) => Some(context),
            Self::NisarRslc(_) => None,
        }
    }

    fn matches_binding(&self, binding: ArtifactValidatorBinding) -> bool {
        match self {
            Self::NisarRslc(context) => context.matches_binding(binding),
            Self::NisarSsarRslc(_) => binding == NISAR_SSAR_RSLC_STRUCTURE_VALIDATOR_BINDING,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArtifactValidationErrorKind {
    Unavailable,
    StructureInvalid,
    /// The operating system refused to open or read the downloaded candidate
    /// (a path the reader cannot use, another program holding the file, a
    /// missing permission): its bytes were not judged and a retry may succeed
    /// (L3 gate 2026-10-19; reported as STRUCTURE_INVALID before).
    LocalReadFailed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArtifactValidationIssue {
    MissingBinding,
    UnknownValidator,
    ValidatorVersionMismatch,
    ValidatorSelfMismatch,
    InvalidContext,
    PluginIsolationUnavailable,
    ValidatedTransferUnsupported,
    CandidateIo,
    InvalidSignature,
    Hdf5Open,
    DeadlineExceeded,
    TraversalLimit,
    LinkNameLimit,
    LinkNotHard,
    ObjectAliasOrCycle,
    AttributeLimit,
    UnsupportedObject,
    DatasetShape,
    DatasetReadBudget,
    DatasetExternalStorage,
    DatasetVirtualLayout,
    DatasetFilter,
    RequiredObjectMissing,
    IdentificationMismatch,
    FrequencyMismatch,
    PolarizationMismatch,
    ImageDatasetInvalid,
    BoundingPolygonInvalid,
    GeolocationGridInvalid,
    FinalSizeMismatch,
    ValidationCancelled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtifactValidationError {
    kind: ArtifactValidationErrorKind,
    issue: ArtifactValidationIssue,
}

impl ArtifactValidationError {
    pub const fn kind(&self) -> ArtifactValidationErrorKind {
        self.kind
    }

    pub const fn issue(&self) -> ArtifactValidationIssue {
        self.issue
    }

    pub const fn code(&self) -> &'static str {
        match self.kind {
            ArtifactValidationErrorKind::Unavailable => "NISAR_HDF5_VALIDATION_UNAVAILABLE",
            ArtifactValidationErrorKind::StructureInvalid => "NISAR_HDF5_STRUCTURE_INVALID",
            ArtifactValidationErrorKind::LocalReadFailed => "NISAR_HDF5_LOCAL_READ_FAILED",
        }
    }

    pub const fn reason_zh(&self) -> &'static str {
        match self.kind {
            ArtifactValidationErrorKind::Unavailable => {
                "NISAR HDF5 结构校验当前不可用，已安全拒绝传输结果。"
            }
            ArtifactValidationErrorKind::StructureInvalid => {
                "NISAR HDF5 文件结构不符合受支持的 RSLC 产品规范。"
            }
            ArtifactValidationErrorKind::LocalReadFailed => {
                "本机无法打开或读取已下载的 NISAR HDF5 文件（例如路径过长、文件被其他程序占用或没有读取权限），文件已保留，可稍后重试。"
            }
        }
    }

    /// Only a local read failure may succeed on a later attempt; a structure
    /// verdict or an unavailable validator does not change by retrying.
    pub const fn retryable(&self) -> bool {
        matches!(self.kind, ArtifactValidationErrorKind::LocalReadFailed)
    }

    pub(crate) const fn unavailable(issue: ArtifactValidationIssue) -> Self {
        Self {
            kind: ArtifactValidationErrorKind::Unavailable,
            issue,
        }
    }

    pub(crate) const fn structure_invalid(issue: ArtifactValidationIssue) -> Self {
        Self {
            kind: ArtifactValidationErrorKind::StructureInvalid,
            issue,
        }
    }

    pub(crate) const fn local_read_failed(issue: ArtifactValidationIssue) -> Self {
        Self {
            kind: ArtifactValidationErrorKind::LocalReadFailed,
            issue,
        }
    }
}

impl fmt::Display for ArtifactValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} ({:?})", self.code(), self.issue)
    }
}

impl std::error::Error for ArtifactValidationError {}

pub(crate) trait CompletedArtifactValidator: Send + Sync {
    fn id(&self) -> ArtifactValidatorId;

    fn version(&self) -> u32;

    fn validate(
        &self,
        candidate_path: &Path,
        asset: &ResolvedAsset,
        context: &ArtifactValidationContext,
    ) -> Result<(), ArtifactValidationError>;
}

#[derive(Clone)]
struct RegistryEntry {
    binding: ArtifactValidatorBinding,
    validator: Arc<dyn CompletedArtifactValidator>,
}

#[derive(Clone)]
pub(crate) struct ArtifactValidatorRegistry {
    entries: Vec<RegistryEntry>,
}

impl Default for ArtifactValidatorRegistry {
    fn default() -> Self {
        Self::standard()
    }
}

impl ArtifactValidatorRegistry {
    pub(crate) fn standard() -> Self {
        Self {
            entries: vec![
                RegistryEntry {
                    binding: NISAR_SSAR_RSLC_STRUCTURE_VALIDATOR_BINDING,
                    validator: Arc::new(NisarSsarHdf5Validator::new()),
                },
                RegistryEntry {
                    binding: NISAR_RSLC_STRUCTURE_VALIDATOR_BINDING,
                    validator: Arc::new(NisarHdf5Validator::new()),
                },
                RegistryEntry {
                    binding: NISAR_RSLC_STRUCTURE_VALIDATOR_V2_BINDING,
                    validator: Arc::new(NisarHdf5Validator::new_v2()),
                },
            ],
        }
    }

    #[doc(hidden)]
    #[cfg(test)]
    pub(crate) fn empty_for_test() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    #[doc(hidden)]
    #[cfg(test)]
    pub(crate) fn with_validator_for_test(
        binding: ArtifactValidatorBinding,
        validator: Arc<dyn CompletedArtifactValidator>,
    ) -> Self {
        Self {
            entries: vec![RegistryEntry { binding, validator }],
        }
    }

    #[doc(hidden)]
    #[cfg(test)]
    pub(crate) fn nisar_with_deadline_for_test(deadline: Duration) -> Self {
        Self {
            entries: vec![RegistryEntry {
                binding: NISAR_RSLC_STRUCTURE_VALIDATOR_BINDING,
                validator: Arc::new(NisarHdf5Validator::with_deadline_for_test(deadline)),
            }],
        }
    }

    #[doc(hidden)]
    #[cfg(test)]
    pub(crate) fn validate_for_test(
        &self,
        binding: ArtifactValidatorBinding,
        candidate_path: &Path,
        asset: &ResolvedAsset,
        context: ArtifactValidationContext,
    ) -> Result<(), ArtifactValidationError> {
        self.resolve(binding, context)?
            .validate(candidate_path, asset)
    }

    pub(crate) fn resolve(
        &self,
        binding: ArtifactValidatorBinding,
        context: ArtifactValidationContext,
    ) -> Result<ArtifactValidationPlan, ArtifactValidationError> {
        let Some(entry) = self.entries.iter().find(|entry| entry.binding == binding) else {
            let issue = if self
                .entries
                .iter()
                .any(|entry| entry.binding.id == binding.id)
            {
                ArtifactValidationIssue::ValidatorVersionMismatch
            } else {
                ArtifactValidationIssue::UnknownValidator
            };
            return Err(ArtifactValidationError::unavailable(issue));
        };
        if entry.validator.id() != binding.id || entry.validator.version() != binding.version {
            return Err(ArtifactValidationError::unavailable(
                ArtifactValidationIssue::ValidatorSelfMismatch,
            ));
        }
        if !context.matches_binding(binding) {
            return Err(ArtifactValidationError::unavailable(
                ArtifactValidationIssue::InvalidContext,
            ));
        }
        Ok(ArtifactValidationPlan {
            binding,
            validator: Arc::clone(&entry.validator),
            context,
            attestation_identity: Arc::new(()),
        })
    }
}

#[derive(Clone)]
pub struct ArtifactValidationPlan {
    binding: ArtifactValidatorBinding,
    validator: Arc<dyn CompletedArtifactValidator>,
    context: ArtifactValidationContext,
    attestation_identity: Arc<()>,
}

impl fmt::Debug for ArtifactValidationPlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ArtifactValidationPlan")
            .field("binding", &self.binding)
            .field("context", &self.context)
            .finish_non_exhaustive()
    }
}

impl ArtifactValidationPlan {
    pub const fn binding(&self) -> ArtifactValidatorBinding {
        self.binding
    }

    pub fn context(&self) -> &ArtifactValidationContext {
        &self.context
    }

    #[doc(hidden)]
    #[cfg(test)]
    pub(crate) fn unavailable_for_test() -> Self {
        Self {
            binding: NISAR_RSLC_STRUCTURE_VALIDATOR_BINDING,
            validator: Arc::new(NisarHdf5Validator::new()),
            context: ArtifactValidationContext::NisarRslc(NisarRslcValidationContext {
                granule_id: "NISAR_TEST_UNAVAILABLE".to_owned(),
                frequency_a: Some(vec![NisarPolarization::Hh]),
                frequency_b: None,
                crid: None,
                product_specification_version:
                    NISAR_RSLC_STRUCTURE_V1_PRODUCT_SPECIFICATION_VERSION.to_owned(),
                product_specification_profile_registry_version: None,
                validator_version: NISAR_RSLC_STRUCTURE_VALIDATOR_VERSION,
            }),
            attestation_identity: Arc::new(()),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_binding_for_test(&self, binding: ArtifactValidatorBinding) -> Self {
        Self {
            binding,
            validator: Arc::clone(&self.validator),
            context: self.context.clone(),
            attestation_identity: Arc::clone(&self.attestation_identity),
        }
    }

    pub(crate) fn attestation_identity(&self) -> Arc<()> {
        Arc::clone(&self.attestation_identity)
    }

    pub(crate) fn matches_attestation_identity(&self, identity: &Arc<()>) -> bool {
        Arc::ptr_eq(&self.attestation_identity, identity)
    }

    pub(crate) fn unavailable_transfer_result(
        &self,
        destination: &Path,
        attempts: u32,
    ) -> TransferResult {
        let error = ArtifactValidationError::unavailable(
            ArtifactValidationIssue::ValidatedTransferUnsupported,
        );
        let bytes_written = std::fs::metadata(part_path(destination))
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        TransferResult {
            outcome: DownloadOutcome::Failed,
            bytes_written,
            attempt_count: attempts,
            failure: Some(DownloadFailure {
                code: error.code().to_owned(),
                reason_zh: error.reason_zh().to_owned(),
                retryable: false,
            }),
        }
    }

    pub(crate) fn validate(
        &self,
        candidate_path: &Path,
        asset: &ResolvedAsset,
    ) -> Result<(), ArtifactValidationError> {
        self.validator
            .validate(candidate_path, asset, &self.context)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ArtifactValidationErrorKind, ArtifactValidationIssue, NisarFrequency, NisarPolarization,
        NisarRslcValidationContext,
    };

    #[test]
    fn frozen_context_accepts_only_the_reviewed_frequency_scopes() {
        let context = NisarRslcValidationContext::from_frozen_metadata(
            "NISAR_L1_PR_RSLC_FIXTURE",
            Some("HH+HV"),
            Some("VV"),
        )
        .unwrap();
        assert_eq!(
            context.polarizations(NisarFrequency::A),
            Some(&[NisarPolarization::Hh, NisarPolarization::Hv][..])
        );
        assert_eq!(
            context.polarizations(NisarFrequency::B),
            Some(&[NisarPolarization::Vv][..])
        );

        for (a, b) in [(None, None), (Some("HV"), None), (None, Some("HH+VV"))] {
            let error =
                NisarRslcValidationContext::from_frozen_metadata("fixture", a, b).unwrap_err();
            assert_eq!(error.kind(), ArtifactValidationErrorKind::Unavailable);
            assert_eq!(error.issue(), ArtifactValidationIssue::InvalidContext);
        }
    }
}
