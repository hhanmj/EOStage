//! Strict, offline-only contracts for a client-derived NISAR RSLC subset.
//!
//! These contracts deliberately live outside the generated V3 provider schema.
//! A request manifest may be embedded in a task-specific `SnapshotItem` metadata
//! envelope, while transfer and artifact manifests remain local sidecars.

use std::cmp::Ordering;

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{validate_debug_context, AssetRole, SnapshotItem};

pub const NISAR_REMOTE_SUBSET_METADATA_KEY: &str = "nisar_remote_subset";
pub const NISAR_SUBSET_REQUEST_MANIFEST_CONTRACT: &str = "NisarSubsetRequestManifestV1";
pub const NISAR_SUBSET_TRANSFER_MANIFEST_CONTRACT: &str = "NisarSubsetTransferManifestV1";
pub const NISAR_SUBSET_ARTIFACT_MANIFEST_CONTRACT: &str = "NisarSubsetArtifactManifestV1";
pub const NISAR_SUBSET_SCHEMA_VERSION: u32 = 1;
pub const NISAR_SUBSET_MECHANISM: &str = "RemoteHdf5SpatialWindow";
pub const NISAR_SUBSET_IMPLEMENTATION_SCOPE: &str = "offline_only";
pub const NISAR_SUBSET_OUTPUT_AUTHORITY: &str = "client_derived";
pub const NISAR_SUBSET_METHOD: &str = "range_hyperslab";
pub const NISAR_SUBSET_MAPPING_ALGORITHM: &str = "sparse_grid_polygon_cells_v1";
pub const NISAR_SUBSET_MAPPING_ALGORITHM_VERSION: u32 = 1;
pub const NISAR_SUBSET_VALIDITY_MASK_SUFFIX: &str = "inputDataExceptionMask";
pub const NISAR_SUBSET_MAX_CANONICAL_JSON_BYTES: usize = 32 * 1024;
/// A 2 MiB sidecar comfortably contains the bounded 4,096-range ledger while
/// remaining small enough to reject accidental unbounded transfer history.
pub const NISAR_SUBSET_MAX_TRANSFER_CANONICAL_JSON_BYTES: usize = 2 * 1024 * 1024;
/// A 1 MiB sidecar covers 64 windows, 64 masks and bounded validation evidence.
pub const NISAR_SUBSET_MAX_ARTIFACT_CANONICAL_JSON_BYTES: usize = 1024 * 1024;
pub const NISAR_SUBSET_MAX_WINDOWS: usize = 64;
pub const NISAR_SUBSET_MAX_COMPONENTS: u16 = 64;
pub const NISAR_SUBSET_MAX_RINGS: u16 = 256;
pub const NISAR_SUBSET_MAX_VERTICES: u32 = 4_096;
pub const NISAR_SUBSET_MAX_RANGES: usize = 4_096;
pub const NISAR_SUBSET_MAX_STRING_BYTES: usize = 512;
pub const NISAR_SUBSET_MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum NisarSubsetManifestError {
    #[error("NISAR_SUBSET_MANIFEST_MISSING")]
    ManifestMissing,
    #[error("NISAR_SUBSET_MANIFEST_INVALID: {0}")]
    ManifestInvalid(String),
    #[error("NISAR_SUBSET_MANIFEST_TOO_LARGE")]
    ManifestTooLarge,
    #[error("NISAR_SUBSET_IDENTITY_MISMATCH: {0}")]
    IdentityMismatch(String),
    #[error("NISAR_SUBSET_UNSUPPORTED_SCOPE: {0}")]
    UnsupportedScope(String),
    #[error("NISAR_SUBSET_SENSITIVE_FIELD: {0}")]
    SensitiveField(String),
    #[error("NISAR_SUBSET_RANGE_INVALID: {0}")]
    RangeInvalid(String),
    #[error("NISAR_SUBSET_WINDOW_INVALID: {0}")]
    WindowInvalid(String),
    #[error("NISAR_SUBSET_ARTIFACT_NOT_VALIDATED: {0}")]
    ArtifactNotValidated(String),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NisarSubsetChecksumV1 {
    pub algorithm: NisarSubsetChecksumAlgorithm,
    pub value: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum NisarSubsetChecksumAlgorithm {
    #[serde(rename = "md5")]
    Md5,
    #[serde(rename = "sha1")]
    Sha1,
    #[serde(rename = "sha256")]
    Sha256,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NisarSubsetSourceV1 {
    pub provider_id: String,
    pub collection_id: String,
    pub collection_profile_id: String,
    pub provider_product_id: String,
    pub provider_product_key: String,
    pub stable_asset_key: String,
    pub file_name: String,
    pub expected_size_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum: Option<NisarSubsetChecksumV1>,
    pub product_specification_version: String,
    pub crid: String,
}

pub type NisarSubsetSourceIdentityV1 = NisarSubsetSourceV1;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NisarSubsetRequestV1 {
    pub aoi_sha256: String,
    pub aoi_crs: String,
    pub aoi_component_count: u16,
    pub aoi_ring_count: u16,
    pub aoi_vertex_count: u32,
    pub frequency: NisarSubsetFrequency,
    pub polarization: String,
    pub swath: String,
    pub subset_method: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum NisarSubsetFrequency {
    #[serde(rename = "A")]
    A,
    #[serde(rename = "B")]
    B,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NisarSubsetMappingV1 {
    pub algorithm: String,
    pub algorithm_version: u32,
    pub evidence_digest: String,
    pub windows: Vec<NisarSubsetLogicalWindowV1>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NisarSubsetLogicalWindowV1 {
    pub component_id: u16,
    pub dataset_path: String,
    pub frequency: NisarSubsetFrequency,
    pub polarization: String,
    pub swath: String,
    pub line_start: u64,
    pub line_end_exclusive: u64,
    pub sample_start: u64,
    pub sample_end_exclusive: u64,
    pub source_shape: [u64; 2],
    pub expected_shape: [u64; 2],
    pub dtype: String,
    pub chunk_shape: [u64; 2],
    pub margin_lines: u64,
    pub margin_samples: u64,
    pub mapping_confidence: NisarSubsetMappingConfidence,
    pub validity_mask_required: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum NisarSubsetMappingConfidence {
    #[serde(rename = "conservative")]
    Conservative,
    #[serde(rename = "reference_bounded")]
    ReferenceBounded,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NisarSubsetExpectedOutputV1 {
    pub derived_artifact_id: String,
    pub file_name: String,
    pub estimated_output_bytes: u64,
    pub estimated_remote_bytes: u64,
    pub full_product_bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NisarSubsetRequestManifestV1 {
    pub manifest_contract: String,
    pub schema_version: u32,
    pub mechanism: String,
    pub implementation_scope: String,
    pub output_authority: String,
    pub default_enabled: bool,
    pub source: NisarSubsetSourceV1,
    pub request: NisarSubsetRequestV1,
    pub mapping: NisarSubsetMappingV1,
    pub expected_output: NisarSubsetExpectedOutputV1,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum NisarSubsetValidatorKind {
    #[serde(rename = "strong_etag")]
    StrongEtag,
    #[serde(rename = "last_modified")]
    LastModified,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NisarSubsetSourceValidatorV1 {
    pub kind: NisarSubsetValidatorKind,
    pub value: String,
    pub content_length: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NisarSubsetByteRangeV1 {
    pub start: u64,
    pub end_exclusive: u64,
    pub expected_bytes: u64,
    pub received_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    pub state: NisarSubsetRangeState,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum NisarSubsetRangeState {
    #[serde(rename = "PLANNED")]
    Planned,
    #[serde(rename = "IN_PROGRESS")]
    InProgress,
    #[serde(rename = "COMPLETE")]
    Complete,
    #[serde(rename = "FAILED")]
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum NisarSubsetTransferState {
    #[serde(rename = "PLANNED")]
    Planned,
    #[serde(rename = "IN_PROGRESS")]
    InProgress,
    #[serde(rename = "INTERRUPTED")]
    Interrupted,
    #[serde(rename = "SOURCE_CHANGED")]
    SourceChanged,
    #[serde(rename = "RANGES_COMPLETE")]
    RangesComplete,
    #[serde(rename = "FAILED")]
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NisarSubsetTransferManifestV1 {
    pub manifest_contract: String,
    pub schema_version: u32,
    pub request_manifest_sha256: String,
    pub source_identity: NisarSubsetSourceIdentityV1,
    pub source_validator: NisarSubsetSourceValidatorV1,
    pub ranges: Vec<NisarSubsetByteRangeV1>,
    pub expected_remote_bytes: u64,
    pub received_bytes: u64,
    pub state: NisarSubsetTransferState,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum NisarSubsetCheckStatus {
    #[serde(rename = "passed")]
    Passed,
    #[serde(rename = "failed")]
    Failed,
    #[serde(rename = "not_run")]
    NotRun,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NisarSubsetValidationCheckV1 {
    pub status: NisarSubsetCheckStatus,
    pub evidence: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NisarSubsetActualExtentV1 {
    pub line_start: u64,
    pub line_end_exclusive: u64,
    pub sample_start: u64,
    pub sample_end_exclusive: u64,
    pub component_count: u16,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NisarSubsetValidityMaskV1 {
    pub component_id: u16,
    pub dataset_path: String,
    pub frequency: NisarSubsetFrequency,
    pub swath: String,
    pub line_start: u64,
    pub line_end_exclusive: u64,
    pub sample_start: u64,
    pub sample_end_exclusive: u64,
    pub shape: [u64; 2],
    pub sha256: String,
    pub semantics: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NisarSubsetWriterV1 {
    pub writer_id: String,
    pub version: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NisarSubsetValidatorPolicyV1 {
    pub policy_id: String,
    pub version: u32,
    pub product_specification_version: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NisarSubsetScienceValidationV1 {
    pub structure: NisarSubsetValidationCheckV1,
    pub complex_values: NisarSubsetValidationCheckV1,
    pub phase: NisarSubsetValidationCheckV1,
    pub amplitude: NisarSubsetValidationCheckV1,
    pub coordinates: NisarSubsetValidationCheckV1,
    pub metadata: NisarSubsetValidationCheckV1,
    pub provenance: NisarSubsetValidationCheckV1,
    pub recovery: NisarSubsetValidationCheckV1,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NisarSubsetArtifactManifestV1 {
    pub manifest_contract: String,
    pub schema_version: u32,
    pub request_manifest_sha256: String,
    pub transfer_manifest_sha256: String,
    pub output_authority: String,
    pub artifact_id: String,
    pub artifact_file_name: String,
    pub artifact_size_bytes: u64,
    pub artifact_sha256: String,
    pub source_product_identity: NisarSubsetSourceIdentityV1,
    pub actual_extent: NisarSubsetActualExtentV1,
    pub dataset_windows: Vec<NisarSubsetLogicalWindowV1>,
    pub validity_masks: Vec<NisarSubsetValidityMaskV1>,
    pub writer: NisarSubsetWriterV1,
    pub validator: NisarSubsetValidatorPolicyV1,
    pub science_validation: NisarSubsetScienceValidationV1,
    pub remote_bytes: u64,
    pub full_product_bytes: u64,
    pub complete: bool,
}

impl NisarSubsetRequestManifestV1 {
    pub fn validate(&self) -> Result<(), NisarSubsetManifestError> {
        validate_header(
            &self.manifest_contract,
            NISAR_SUBSET_REQUEST_MANIFEST_CONTRACT,
            self.schema_version,
            &self.mechanism,
            &self.implementation_scope,
            &self.output_authority,
            self.default_enabled,
        )?;
        validate_source(&self.source)?;
        validate_request(&self.request)?;
        validate_mapping(&self.mapping, &self.request)?;
        validate_expected_output(&self.expected_output, self.source.expected_size_bytes)?;
        canonical_json_with_limit(self, NISAR_SUBSET_MAX_CANONICAL_JSON_BYTES)?;
        Ok(())
    }

    pub fn canonical_json(&self) -> Result<String, NisarSubsetManifestError> {
        self.validate()?;
        let mut normalized = self.clone();
        normalized.mapping.windows.sort_by(compare_logical_windows);
        canonical_json_with_limit(&normalized, NISAR_SUBSET_MAX_CANONICAL_JSON_BYTES)
    }

    pub fn sha256(&self) -> Result<String, NisarSubsetManifestError> {
        sha256_text(&self.canonical_json()?)
    }
}

impl NisarSubsetTransferManifestV1 {
    pub fn validate(&self) -> Result<(), NisarSubsetManifestError> {
        validate_contract_header(
            &self.manifest_contract,
            NISAR_SUBSET_TRANSFER_MANIFEST_CONTRACT,
            self.schema_version,
        )?;
        validate_sha256(&self.request_manifest_sha256, "request_manifest_sha256")?;
        validate_source(&self.source_identity)?;
        validate_source_validator(
            &self.source_validator,
            self.source_identity.expected_size_bytes,
        )?;
        let created_at = parse_rfc3339_instant("created_at", &self.created_at)?;
        let updated_at = parse_rfc3339_instant("updated_at", &self.updated_at)?;
        if updated_at < created_at {
            return Err(NisarSubsetManifestError::ManifestInvalid(
                "updated_at precedes created_at".to_owned(),
            ));
        }
        validate_safe_integer(self.expected_remote_bytes, "expected_remote_bytes")?;
        validate_safe_integer(self.received_bytes, "received_bytes")?;
        if self.expected_remote_bytes == 0 || self.ranges.is_empty() {
            return Err(NisarSubsetManifestError::RangeInvalid(
                "ranges must be non-empty".to_owned(),
            ));
        }
        if self.ranges.len() > NISAR_SUBSET_MAX_RANGES {
            return Err(NisarSubsetManifestError::RangeInvalid(
                "range count exceeds bound".to_owned(),
            ));
        }
        let mut previous_end = 0_u64;
        let mut expected_total = 0_u64;
        let mut received_total = 0_u64;
        for (index, range) in self.ranges.iter().enumerate() {
            if range.start >= range.end_exclusive {
                return Err(NisarSubsetManifestError::RangeInvalid(format!(
                    "range {index} is not half-open"
                )));
            }
            if index > 0 && range.start < previous_end {
                return Err(NisarSubsetManifestError::RangeInvalid(
                    "ranges overlap or are not sorted".to_owned(),
                ));
            }
            let width = range
                .end_exclusive
                .checked_sub(range.start)
                .ok_or_else(|| {
                    NisarSubsetManifestError::RangeInvalid("range overflow".to_owned())
                })?;
            validate_safe_integer(range.start, &format!("ranges[{index}].start"))?;
            validate_safe_integer(
                range.end_exclusive,
                &format!("ranges[{index}].end_exclusive"),
            )?;
            validate_safe_integer(
                range.expected_bytes,
                &format!("ranges[{index}].expected_bytes"),
            )?;
            validate_safe_integer(
                range.received_bytes,
                &format!("ranges[{index}].received_bytes"),
            )?;
            if range.expected_bytes != width || range.received_bytes > width {
                return Err(NisarSubsetManifestError::RangeInvalid(format!(
                    "range {index} byte accounting is invalid"
                )));
            }
            if let Some(digest) = &range.sha256 {
                validate_sha256(digest, &format!("ranges[{index}].sha256"))?;
            }
            if range.state == NisarSubsetRangeState::Complete
                && (range.received_bytes != width || range.sha256.is_none())
            {
                return Err(NisarSubsetManifestError::RangeInvalid(format!(
                    "completed range {index} lacks complete evidence"
                )));
            }
            validate_range_state(range, index, width)?;
            expected_total = expected_total.checked_add(width).ok_or_else(|| {
                NisarSubsetManifestError::RangeInvalid("expected byte overflow".to_owned())
            })?;
            received_total = received_total
                .checked_add(range.received_bytes)
                .ok_or_else(|| {
                    NisarSubsetManifestError::RangeInvalid("received byte overflow".to_owned())
                })?;
            previous_end = range.end_exclusive;
        }
        if expected_total != self.expected_remote_bytes || received_total != self.received_bytes {
            return Err(NisarSubsetManifestError::RangeInvalid(
                "manifest and range byte totals differ".to_owned(),
            ));
        }
        if self.received_bytes > self.expected_remote_bytes {
            return Err(NisarSubsetManifestError::RangeInvalid(
                "received bytes exceed expected bytes".to_owned(),
            ));
        }
        validate_transfer_state(
            &self.state,
            &self.ranges,
            self.expected_remote_bytes,
            self.received_bytes,
        )?;
        canonical_json_with_limit(self, NISAR_SUBSET_MAX_TRANSFER_CANONICAL_JSON_BYTES)?;
        Ok(())
    }

    pub fn canonical_json(&self) -> Result<String, NisarSubsetManifestError> {
        self.validate()?;
        let mut normalized = self.clone();
        normalized.ranges.sort_by(compare_byte_ranges);
        canonical_json_with_limit(&normalized, NISAR_SUBSET_MAX_TRANSFER_CANONICAL_JSON_BYTES)
    }

    pub fn sha256(&self) -> Result<String, NisarSubsetManifestError> {
        sha256_text(&self.canonical_json()?)
    }

    pub fn validate_against_request(
        &self,
        request: &NisarSubsetRequestManifestV1,
    ) -> Result<(), NisarSubsetManifestError> {
        request.validate()?;
        self.validate()?;
        if self.request_manifest_sha256 != request.sha256()? {
            return Err(NisarSubsetManifestError::IdentityMismatch(
                "transfer request digest differs".to_owned(),
            ));
        }
        if self.source_identity != request.source {
            return Err(NisarSubsetManifestError::IdentityMismatch(
                "transfer source identity differs".to_owned(),
            ));
        }
        if self.expected_remote_bytes != request.expected_output.estimated_remote_bytes {
            return Err(NisarSubsetManifestError::RangeInvalid(
                "transfer bytes differ from request".to_owned(),
            ));
        }
        Ok(())
    }
}

impl NisarSubsetArtifactManifestV1 {
    pub fn validate(&self) -> Result<(), NisarSubsetManifestError> {
        validate_contract_header(
            &self.manifest_contract,
            NISAR_SUBSET_ARTIFACT_MANIFEST_CONTRACT,
            self.schema_version,
        )?;
        if self.output_authority != NISAR_SUBSET_OUTPUT_AUTHORITY {
            return Err(NisarSubsetManifestError::UnsupportedScope(
                "artifact output authority must be client_derived".to_owned(),
            ));
        }
        validate_sha256(&self.request_manifest_sha256, "request_manifest_sha256")?;
        validate_sha256(&self.transfer_manifest_sha256, "transfer_manifest_sha256")?;
        validate_safe_text(
            "artifact_id",
            &self.artifact_id,
            NISAR_SUBSET_MAX_STRING_BYTES,
        )?;
        for (label, value) in [
            ("artifact_size_bytes", self.artifact_size_bytes),
            ("remote_bytes", self.remote_bytes),
            ("full_product_bytes", self.full_product_bytes),
            ("writer.version", self.writer.version as u64),
            ("validator.version", self.validator.version as u64),
        ] {
            validate_safe_integer(value, label)?;
        }
        validate_derived_file_name(&self.artifact_file_name, "artifact_file_name")?;
        validate_sha256(&self.artifact_sha256, "artifact_sha256")?;
        validate_source(&self.source_product_identity)?;
        validate_extent(&self.actual_extent)?;
        if self.dataset_windows.is_empty() || self.dataset_windows.len() > NISAR_SUBSET_MAX_WINDOWS
        {
            return Err(NisarSubsetManifestError::WindowInvalid(
                "artifact dataset window count is outside the bound".to_owned(),
            ));
        }
        for window in &self.dataset_windows {
            validate_window(window, None)?;
        }
        if self.validity_masks.len() > NISAR_SUBSET_MAX_WINDOWS {
            return Err(NisarSubsetManifestError::ArtifactNotValidated(
                "artifact validity masks exceed the bound".to_owned(),
            ));
        }
        for mask in &self.validity_masks {
            validate_mask(mask)?;
        }
        validate_safe_text(
            "writer.writer_id",
            &self.writer.writer_id,
            NISAR_SUBSET_MAX_STRING_BYTES,
        )?;
        if self.writer.version == 0 {
            return Err(NisarSubsetManifestError::ArtifactNotValidated(
                "writer version must be positive".to_owned(),
            ));
        }
        validate_safe_text(
            "validator.policy_id",
            &self.validator.policy_id,
            NISAR_SUBSET_MAX_STRING_BYTES,
        )?;
        if self.validator.version == 0 {
            return Err(NisarSubsetManifestError::ArtifactNotValidated(
                "validator version must be positive".to_owned(),
            ));
        }
        validate_safe_text(
            "validator.product_specification_version",
            &self.validator.product_specification_version,
            64,
        )?;
        validate_science_validation(&self.science_validation)?;
        validate_byte_totals(self.remote_bytes, self.full_product_bytes, "artifact")?;
        if self.validator.product_specification_version
            != self.source_product_identity.product_specification_version
        {
            return Err(NisarSubsetManifestError::IdentityMismatch(
                "validator specification version differs from source".to_owned(),
            ));
        }
        if self.artifact_size_bytes == 0 || !self.complete {
            return Err(NisarSubsetManifestError::ArtifactNotValidated(
                "artifact must be complete and non-empty".to_owned(),
            ));
        }
        canonical_json_with_limit(self, NISAR_SUBSET_MAX_ARTIFACT_CANONICAL_JSON_BYTES)?;
        Ok(())
    }

    pub fn canonical_json(&self) -> Result<String, NisarSubsetManifestError> {
        self.validate()?;
        let mut normalized = self.clone();
        normalized.dataset_windows.sort_by(compare_logical_windows);
        normalized.validity_masks.sort_by(compare_validity_masks);
        canonical_json_with_limit(&normalized, NISAR_SUBSET_MAX_ARTIFACT_CANONICAL_JSON_BYTES)
    }

    pub fn sha256(&self) -> Result<String, NisarSubsetManifestError> {
        sha256_text(&self.canonical_json()?)
    }

    pub fn validate_against_request_and_transfer(
        &self,
        request: &NisarSubsetRequestManifestV1,
        transfer: &NisarSubsetTransferManifestV1,
    ) -> Result<(), NisarSubsetManifestError> {
        request.validate()?;
        transfer.validate_against_request(request)?;
        self.validate()?;
        if self.request_manifest_sha256 != request.sha256()? {
            return Err(NisarSubsetManifestError::IdentityMismatch(
                "artifact request digest differs".to_owned(),
            ));
        }
        if self.transfer_manifest_sha256 != transfer.sha256()? {
            return Err(NisarSubsetManifestError::IdentityMismatch(
                "artifact transfer digest differs".to_owned(),
            ));
        }
        if self.source_product_identity != request.source
            || self.source_product_identity != transfer.source_identity
        {
            return Err(NisarSubsetManifestError::IdentityMismatch(
                "artifact source identity differs".to_owned(),
            ));
        }
        if transfer.state != NisarSubsetTransferState::RangesComplete
            || transfer.received_bytes != transfer.expected_remote_bytes
            || transfer.ranges.iter().any(|range| {
                range.state != NisarSubsetRangeState::Complete || range.sha256.is_none()
            })
        {
            return Err(NisarSubsetManifestError::ArtifactNotValidated(
                "artifact requires a fully completed transfer".to_owned(),
            ));
        }
        if self.remote_bytes != transfer.received_bytes
            || self.remote_bytes != request.expected_output.estimated_remote_bytes
        {
            return Err(NisarSubsetManifestError::RangeInvalid(
                "artifact remote bytes differ".to_owned(),
            ));
        }
        if self.artifact_id != request.expected_output.derived_artifact_id
            || self.artifact_file_name != request.expected_output.file_name
        {
            return Err(NisarSubsetManifestError::IdentityMismatch(
                "artifact output identity differs from request".to_owned(),
            ));
        }
        if self.full_product_bytes != request.source.expected_size_bytes
            || self.full_product_bytes != request.expected_output.full_product_bytes
        {
            return Err(NisarSubsetManifestError::IdentityMismatch(
                "artifact full product bytes differ".to_owned(),
            ));
        }
        validate_artifact_windows_and_masks(self, request)?;
        Ok(())
    }
}

pub fn embed_request_manifest(
    metadata: &mut Map<String, Value>,
    manifest: &NisarSubsetRequestManifestV1,
) -> Result<(), NisarSubsetManifestError> {
    manifest.canonical_json()?;
    let value = serde_json::to_value(manifest)
        .map_err(|error| NisarSubsetManifestError::ManifestInvalid(error.to_string()))?;
    metadata.insert(NISAR_REMOTE_SUBSET_METADATA_KEY.to_owned(), value);
    validate_debug_context(metadata)
        .map_err(|error| NisarSubsetManifestError::SensitiveField(error.to_string()))?;
    Ok(())
}

pub fn parse_request_manifest(
    metadata: &Map<String, Value>,
) -> Result<NisarSubsetRequestManifestV1, NisarSubsetManifestError> {
    let value = metadata
        .get(NISAR_REMOTE_SUBSET_METADATA_KEY)
        .ok_or(NisarSubsetManifestError::ManifestMissing)?;
    let manifest: NisarSubsetRequestManifestV1 = serde_json::from_value(value.clone())
        .map_err(|error| NisarSubsetManifestError::ManifestInvalid(error.to_string()))?;
    manifest.canonical_json()?;
    Ok(manifest)
}

/// Bind an offline Request Manifest to the frozen primary HDF5 asset without
/// copying any URL, credential profile, header or temporary access material.
pub fn validate_request_against_snapshot_item(
    manifest: &NisarSubsetRequestManifestV1,
    item: &SnapshotItem,
) -> Result<(), NisarSubsetManifestError> {
    manifest.validate()?;
    item.validate()
        .map_err(NisarSubsetManifestError::ManifestInvalid)?;
    let source = &manifest.source;
    let asset = item.asset_access.as_ref().ok_or_else(|| {
        NisarSubsetManifestError::IdentityMismatch("snapshot primary asset is missing".to_owned())
    })?;
    let reference = &asset.reference;
    if item.provider_id != source.provider_id
        || reference.provider_id != source.provider_id
        || reference.collection_id != source.collection_id
        || reference.provider_product_id != source.provider_product_id
        || reference.provider_product_key != source.provider_product_key
        || reference.stable_asset_key != source.stable_asset_key
    {
        return Err(NisarSubsetManifestError::IdentityMismatch(
            "snapshot product or asset identity differs from request".to_owned(),
        ));
    }
    if asset.role != AssetRole::Primary
        || asset.file_name.as_deref() != Some(source.file_name.as_str())
        || !source.file_name.to_ascii_lowercase().ends_with(".h5")
        || asset.expected_size_bytes != Some(source.expected_size_bytes)
    {
        return Err(NisarSubsetManifestError::IdentityMismatch(
            "snapshot asset is not the requested primary HDF5".to_owned(),
        ));
    }
    let snapshot_profile = item
        .execution_policy
        .as_ref()
        .and_then(|policy| policy.collection_profile_id.as_deref())
        .or_else(|| {
            item.metadata
                .get("collection_profile_id")
                .and_then(Value::as_str)
        });
    if snapshot_profile != Some(source.collection_profile_id.as_str()) {
        return Err(NisarSubsetManifestError::IdentityMismatch(
            "snapshot collection profile differs from request".to_owned(),
        ));
    }
    match (
        &source.checksum,
        &asset.checksum_algorithm,
        &asset.checksum_value,
    ) {
        (None, None, None) => {}
        (Some(checksum), Some(algorithm), Some(value))
            if snapshot_checksum_algorithm(algorithm) == Some(&checksum.algorithm)
                && value == &checksum.value => {}
        _ => {
            return Err(NisarSubsetManifestError::IdentityMismatch(
                "snapshot checksum differs from request".to_owned(),
            ))
        }
    }
    Ok(())
}

fn snapshot_checksum_algorithm(value: &str) -> Option<&'static NisarSubsetChecksumAlgorithm> {
    static MD5: NisarSubsetChecksumAlgorithm = NisarSubsetChecksumAlgorithm::Md5;
    static SHA1: NisarSubsetChecksumAlgorithm = NisarSubsetChecksumAlgorithm::Sha1;
    static SHA256: NisarSubsetChecksumAlgorithm = NisarSubsetChecksumAlgorithm::Sha256;
    match value.to_ascii_lowercase().as_str() {
        "md5" => Some(&MD5),
        "sha1" => Some(&SHA1),
        "sha256" => Some(&SHA256),
        _ => None,
    }
}

pub fn canonical_json<T: Serialize>(value: &T) -> Result<String, NisarSubsetManifestError> {
    canonical_json_with_limit(value, NISAR_SUBSET_MAX_CANONICAL_JSON_BYTES)
}

fn canonical_json_with_limit<T: Serialize>(
    value: &T,
    maximum_bytes: usize,
) -> Result<String, NisarSubsetManifestError> {
    let json = serde_json::to_value(value)
        .map_err(|error| NisarSubsetManifestError::ManifestInvalid(error.to_string()))?;
    let json = canonicalize_json(json);
    let rendered = serde_json::to_string(&json)
        .map_err(|error| NisarSubsetManifestError::ManifestInvalid(error.to_string()))?;
    if rendered.len() > maximum_bytes {
        return Err(NisarSubsetManifestError::ManifestTooLarge);
    }
    Ok(rendered)
}

pub fn parse_canonical_json<T: DeserializeOwned>(
    value: &str,
) -> Result<T, NisarSubsetManifestError> {
    if value.len() > NISAR_SUBSET_MAX_CANONICAL_JSON_BYTES {
        return Err(NisarSubsetManifestError::ManifestTooLarge);
    }
    serde_json::from_str(value)
        .map_err(|error| NisarSubsetManifestError::ManifestInvalid(error.to_string()))
}

fn validate_header(
    contract: &str,
    expected_contract: &str,
    schema_version: u32,
    mechanism: &str,
    implementation_scope: &str,
    output_authority: &str,
    default_enabled: bool,
) -> Result<(), NisarSubsetManifestError> {
    validate_contract_header(contract, expected_contract, schema_version)?;
    if mechanism != NISAR_SUBSET_MECHANISM
        || implementation_scope != NISAR_SUBSET_IMPLEMENTATION_SCOPE
        || output_authority != NISAR_SUBSET_OUTPUT_AUTHORITY
        || default_enabled
    {
        return Err(NisarSubsetManifestError::UnsupportedScope(
            "manifest is not offline-only, client-derived and default-off".to_owned(),
        ));
    }
    Ok(())
}

fn validate_contract_header(
    contract: &str,
    expected_contract: &str,
    schema_version: u32,
) -> Result<(), NisarSubsetManifestError> {
    validate_safe_integer(schema_version as u64, "schema_version")?;
    if contract != expected_contract || schema_version != NISAR_SUBSET_SCHEMA_VERSION {
        return Err(NisarSubsetManifestError::ManifestInvalid(format!(
            "expected {expected_contract} schema version {NISAR_SUBSET_SCHEMA_VERSION}"
        )));
    }
    Ok(())
}

fn validate_source(source: &NisarSubsetSourceV1) -> Result<(), NisarSubsetManifestError> {
    for (label, value) in [
        ("provider_id", &source.provider_id),
        ("collection_id", &source.collection_id),
        ("collection_profile_id", &source.collection_profile_id),
        ("provider_product_id", &source.provider_product_id),
        ("provider_product_key", &source.provider_product_key),
        ("stable_asset_key", &source.stable_asset_key),
        ("file_name", &source.file_name),
        (
            "product_specification_version",
            &source.product_specification_version,
        ),
        ("crid", &source.crid),
    ] {
        validate_safe_text(label, value, NISAR_SUBSET_MAX_STRING_BYTES)?;
    }
    validate_safe_integer(source.expected_size_bytes, "source.expected_size_bytes")?;
    if source.expected_size_bytes == 0 {
        return Err(NisarSubsetManifestError::IdentityMismatch(
            "expected_size_bytes must be positive".to_owned(),
        ));
    }
    if source.file_name.contains('/')
        || source.file_name.contains('\\')
        || source.file_name.ends_with(".part")
    {
        return Err(NisarSubsetManifestError::IdentityMismatch(
            "source file_name must be a plain complete-product name".to_owned(),
        ));
    }
    if let Some(checksum) = &source.checksum {
        validate_checksum(checksum)?;
    }
    Ok(())
}

fn validate_checksum(checksum: &NisarSubsetChecksumV1) -> Result<(), NisarSubsetManifestError> {
    let expected = match checksum.algorithm {
        NisarSubsetChecksumAlgorithm::Md5 => 32,
        NisarSubsetChecksumAlgorithm::Sha1 => 40,
        NisarSubsetChecksumAlgorithm::Sha256 => 64,
    };
    if checksum.value.len() != expected
        || !checksum.value.bytes().all(|byte| byte.is_ascii_hexdigit())
        || checksum
            .value
            .chars()
            .any(|character| character.is_ascii_uppercase())
    {
        return Err(NisarSubsetManifestError::IdentityMismatch(
            "checksum value is not a lower-case digest".to_owned(),
        ));
    }
    Ok(())
}

fn validate_request(request: &NisarSubsetRequestV1) -> Result<(), NisarSubsetManifestError> {
    validate_sha256(&request.aoi_sha256, "aoi_sha256")?;
    if request.aoi_crs != "EPSG:4326" {
        return Err(NisarSubsetManifestError::UnsupportedScope(
            "only EPSG:4326 AOIs are supported in P1.0".to_owned(),
        ));
    }
    validate_safe_integer(
        request.aoi_component_count as u64,
        "request.aoi_component_count",
    )?;
    validate_safe_integer(request.aoi_ring_count as u64, "request.aoi_ring_count")?;
    validate_safe_integer(request.aoi_vertex_count as u64, "request.aoi_vertex_count")?;
    if request.aoi_component_count == 0
        || request.aoi_component_count > NISAR_SUBSET_MAX_COMPONENTS
        || request.aoi_ring_count == 0
        || request.aoi_ring_count > NISAR_SUBSET_MAX_RINGS
        || request.aoi_vertex_count == 0
        || request.aoi_vertex_count > NISAR_SUBSET_MAX_VERTICES
    {
        return Err(NisarSubsetManifestError::WindowInvalid(
            "AOI summary exceeds P1.0 bounds".to_owned(),
        ));
    }
    if request.subset_method != NISAR_SUBSET_METHOD {
        return Err(NisarSubsetManifestError::UnsupportedScope(
            "subset_method must be range_hyperslab".to_owned(),
        ));
    }
    validate_polarization(&request.frequency, &request.polarization)?;
    validate_safe_text("swath", &request.swath, 128)?;
    let upper_swath = request.swath.to_ascii_uppercase();
    if upper_swath.contains("SSAR")
        || upper_swath.contains("S-BAND")
        || upper_swath.contains("BHOONIDHI")
    {
        return Err(NisarSubsetManifestError::UnsupportedScope(
            "S-band and Bhoonidhi are not supported".to_owned(),
        ));
    }
    Ok(())
}

fn validate_mapping(
    mapping: &NisarSubsetMappingV1,
    request: &NisarSubsetRequestV1,
) -> Result<(), NisarSubsetManifestError> {
    if mapping.algorithm != NISAR_SUBSET_MAPPING_ALGORITHM
        || mapping.algorithm_version != NISAR_SUBSET_MAPPING_ALGORITHM_VERSION
    {
        return Err(NisarSubsetManifestError::UnsupportedScope(
            "mapping algorithm is not the approved P1.0 candidate".to_owned(),
        ));
    }
    validate_safe_integer(
        mapping.algorithm_version as u64,
        "mapping.algorithm_version",
    )?;
    validate_sha256(&mapping.evidence_digest, "mapping.evidence_digest")?;
    if mapping.windows.is_empty() || mapping.windows.len() > NISAR_SUBSET_MAX_WINDOWS {
        return Err(NisarSubsetManifestError::WindowInvalid(
            "logical window count is outside the bound".to_owned(),
        ));
    }
    for window in &mapping.windows {
        validate_window(window, Some(request))?;
    }
    let component_count = usize::from(request.aoi_component_count);
    let mut covered = vec![false; component_count];
    let mut logical_identities = std::collections::BTreeSet::new();
    let mut mask_identities = std::collections::BTreeSet::new();
    for window in &mapping.windows {
        let component = usize::from(window.component_id);
        if component >= component_count {
            return Err(NisarSubsetManifestError::WindowInvalid(
                "window component is outside the request AOI".to_owned(),
            ));
        }
        covered[component] = true;
        let logical_identity = (
            window.component_id,
            window.dataset_path.as_str(),
            &window.frequency,
            window.polarization.as_str(),
            window.swath.as_str(),
            window.line_start,
            window.line_end_exclusive,
            window.sample_start,
            window.sample_end_exclusive,
        );
        if !logical_identities.insert(logical_identity) {
            return Err(NisarSubsetManifestError::WindowInvalid(
                "duplicate logical window".to_owned(),
            ));
        }
        if window.validity_mask_required {
            let mask_identity = (
                window.component_id,
                &window.frequency,
                window.swath.as_str(),
                window.line_start,
                window.line_end_exclusive,
                window.sample_start,
                window.sample_end_exclusive,
            );
            if !mask_identities.insert(mask_identity) {
                return Err(NisarSubsetManifestError::WindowInvalid(
                    "duplicate frequency-level mask identity".to_owned(),
                ));
            }
        }
    }
    if covered.iter().any(|present| !present) {
        return Err(NisarSubsetManifestError::WindowInvalid(
            "mapping does not cover every AOI component".to_owned(),
        ));
    }
    Ok(())
}

fn validate_window(
    window: &NisarSubsetLogicalWindowV1,
    request: Option<&NisarSubsetRequestV1>,
) -> Result<(), NisarSubsetManifestError> {
    validate_safe_integer(window.component_id as u64, "window.component_id")?;
    for (label, value) in [
        ("window.line_start", window.line_start),
        ("window.line_end_exclusive", window.line_end_exclusive),
        ("window.sample_start", window.sample_start),
        ("window.sample_end_exclusive", window.sample_end_exclusive),
        ("window.source_shape[0]", window.source_shape[0]),
        ("window.source_shape[1]", window.source_shape[1]),
        ("window.expected_shape[0]", window.expected_shape[0]),
        ("window.expected_shape[1]", window.expected_shape[1]),
        ("window.chunk_shape[0]", window.chunk_shape[0]),
        ("window.chunk_shape[1]", window.chunk_shape[1]),
        ("window.margin_lines", window.margin_lines),
        ("window.margin_samples", window.margin_samples),
    ] {
        validate_safe_integer(value, label)?;
    }
    if window.component_id >= NISAR_SUBSET_MAX_COMPONENTS {
        return Err(NisarSubsetManifestError::WindowInvalid(
            "component_id exceeds bound".to_owned(),
        ));
    }
    validate_safe_text("dataset_path", &window.dataset_path, 512)?;
    if !window.dataset_path.starts_with('/')
        || window.dataset_path.contains("..")
        || window.dataset_path.contains('\\')
    {
        return Err(NisarSubsetManifestError::WindowInvalid(
            "dataset_path must be an absolute HDF5 path".to_owned(),
        ));
    }
    validate_safe_text("window.polarization", &window.polarization, 16)?;
    validate_safe_text("window.swath", &window.swath, 128)?;
    if let Some(request) = request {
        if window.frequency != request.frequency
            || window.polarization != request.polarization
            || window.swath != request.swath
        {
            return Err(NisarSubsetManifestError::IdentityMismatch(
                "window science identity differs from request".to_owned(),
            ));
        }
    }
    validate_polarization(&window.frequency, &window.polarization)?;
    if window.dataset_path != science_dataset_path(&window.frequency, &window.polarization) {
        return Err(NisarSubsetManifestError::WindowInvalid(
            "dataset_path does not match the frequency/polarization dataset".to_owned(),
        ));
    }
    if window.line_start >= window.line_end_exclusive
        || window.sample_start >= window.sample_end_exclusive
        || window.source_shape.contains(&0)
        || window.chunk_shape.contains(&0)
        || window.line_end_exclusive > window.source_shape[0]
        || window.sample_end_exclusive > window.source_shape[1]
    {
        return Err(NisarSubsetManifestError::WindowInvalid(
            "window is outside its source shape or is not half-open".to_owned(),
        ));
    }
    let expected_shape = [
        window.line_end_exclusive - window.line_start,
        window.sample_end_exclusive - window.sample_start,
    ];
    if window.expected_shape != expected_shape {
        return Err(NisarSubsetManifestError::WindowInvalid(
            "expected_shape does not match the half-open window".to_owned(),
        ));
    }
    validate_safe_text("window.dtype", &window.dtype, 64)?;
    Ok(())
}

fn validate_expected_output(
    output: &NisarSubsetExpectedOutputV1,
    source_size: u64,
) -> Result<(), NisarSubsetManifestError> {
    validate_safe_text(
        "derived_artifact_id",
        &output.derived_artifact_id,
        NISAR_SUBSET_MAX_STRING_BYTES,
    )?;
    validate_derived_file_name(&output.file_name, "expected_output.file_name")?;
    for (label, value) in [
        (
            "expected_output.estimated_output_bytes",
            output.estimated_output_bytes,
        ),
        (
            "expected_output.estimated_remote_bytes",
            output.estimated_remote_bytes,
        ),
        (
            "expected_output.full_product_bytes",
            output.full_product_bytes,
        ),
    ] {
        validate_safe_integer(value, label)?;
    }
    if output.estimated_output_bytes == 0
        || output.estimated_remote_bytes == 0
        || output.full_product_bytes == 0
        || output.full_product_bytes != source_size
    {
        return Err(NisarSubsetManifestError::IdentityMismatch(
            "expected output byte totals are invalid".to_owned(),
        ));
    }
    validate_byte_totals(
        output.estimated_remote_bytes,
        output.full_product_bytes,
        "expected output",
    )
}

fn validate_source_validator(
    validator: &NisarSubsetSourceValidatorV1,
    expected_size: u64,
) -> Result<(), NisarSubsetManifestError> {
    validate_safe_text("source_validator.value", &validator.value, 256)?;
    validate_safe_integer(validator.content_length, "source_validator.content_length")?;
    if validator.content_length != expected_size {
        return Err(NisarSubsetManifestError::IdentityMismatch(
            "validator content length differs from source size".to_owned(),
        ));
    }
    match validator.kind {
        NisarSubsetValidatorKind::StrongEtag => {
            if !validator.value.starts_with('"')
                || !validator.value.ends_with('"')
                || validator.value.starts_with("W/")
            {
                return Err(NisarSubsetManifestError::RangeInvalid(
                    "strong_etag must be quoted and non-weak".to_owned(),
                ));
            }
        }
        NisarSubsetValidatorKind::LastModified => {}
    }
    Ok(())
}

fn validate_range_state(
    range: &NisarSubsetByteRangeV1,
    index: usize,
    width: u64,
) -> Result<(), NisarSubsetManifestError> {
    let valid = match range.state {
        NisarSubsetRangeState::Planned => range.received_bytes == 0 && range.sha256.is_none(),
        NisarSubsetRangeState::InProgress => {
            range.received_bytes > 0 && range.received_bytes < width && range.sha256.is_none()
        }
        NisarSubsetRangeState::Complete => range.received_bytes == width && range.sha256.is_some(),
        NisarSubsetRangeState::Failed => range.received_bytes < width && range.sha256.is_none(),
    };
    if !valid {
        return Err(NisarSubsetManifestError::RangeInvalid(format!(
            "range {index} state does not match its byte evidence"
        )));
    }
    Ok(())
}

fn validate_transfer_state(
    state: &NisarSubsetTransferState,
    ranges: &[NisarSubsetByteRangeV1],
    expected_remote_bytes: u64,
    received_bytes: u64,
) -> Result<(), NisarSubsetManifestError> {
    let all_complete = ranges
        .iter()
        .all(|range| range.state == NisarSubsetRangeState::Complete);
    let any_failed = ranges
        .iter()
        .any(|range| range.state == NisarSubsetRangeState::Failed);
    let any_started = ranges.iter().any(|range| {
        range.state == NisarSubsetRangeState::InProgress
            || range.state == NisarSubsetRangeState::Complete
            || range.state == NisarSubsetRangeState::Failed
    });
    let valid = match state {
        NisarSubsetTransferState::Planned => {
            received_bytes == 0
                && ranges
                    .iter()
                    .all(|range| range.state == NisarSubsetRangeState::Planned)
        }
        NisarSubsetTransferState::InProgress => {
            received_bytes < expected_remote_bytes && !all_complete && any_started
        }
        NisarSubsetTransferState::Interrupted => {
            received_bytes < expected_remote_bytes && !all_complete
        }
        NisarSubsetTransferState::SourceChanged => {
            received_bytes < expected_remote_bytes && !all_complete
        }
        NisarSubsetTransferState::RangesComplete => {
            received_bytes == expected_remote_bytes && all_complete
        }
        NisarSubsetTransferState::Failed => {
            received_bytes < expected_remote_bytes && !all_complete && any_failed
        }
    };
    if !valid {
        return Err(NisarSubsetManifestError::RangeInvalid(
            "transfer state does not match range evidence".to_owned(),
        ));
    }
    Ok(())
}

fn parse_rfc3339_instant(label: &str, value: &str) -> Result<(i64, u32), NisarSubsetManifestError> {
    validate_safe_text(label, value, 64)?;
    let bytes = value.as_bytes();
    if bytes.len() < 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return Err(NisarSubsetManifestError::ManifestInvalid(format!(
            "{label} is not RFC3339"
        )));
    }
    let year = parse_digits(bytes, 0, 4)?;
    let month = parse_digits(bytes, 5, 2)?;
    let day = parse_digits(bytes, 8, 2)?;
    let hour = parse_digits(bytes, 11, 2)?;
    let minute = parse_digits(bytes, 14, 2)?;
    let second = parse_digits(bytes, 17, 2)?;
    if year == 0
        || !(1..=12).contains(&month)
        || day == 0
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return Err(NisarSubsetManifestError::ManifestInvalid(format!(
            "{label} has an invalid date or time"
        )));
    }
    let mut index = 19;
    let mut nanoseconds = 0_u32;
    if bytes.get(index) == Some(&b'.') {
        index += 1;
        let start = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        if start == index {
            return Err(NisarSubsetManifestError::ManifestInvalid(format!(
                "{label} has an invalid fractional second"
            )));
        }
        let digits = index - start;
        if digits > 9 {
            return Err(NisarSubsetManifestError::ManifestInvalid(format!(
                "{label} fractional precision exceeds nanoseconds"
            )));
        }
        nanoseconds = parse_digits(bytes, start, digits)? * 10_u32.pow((9 - digits) as u32);
    }
    let mut offset_seconds = 0_i64;
    match bytes.get(index) {
        Some(b'Z') => index += 1,
        Some(sign @ (b'+' | b'-')) => {
            if bytes.len() < index + 6 || bytes[index + 3] != b':' {
                return Err(NisarSubsetManifestError::ManifestInvalid(format!(
                    "{label} has an invalid offset"
                )));
            }
            let offset_hour = parse_digits(bytes, index + 1, 2)?;
            let offset_minute = parse_digits(bytes, index + 4, 2)?;
            if offset_hour > 23 || offset_minute > 59 {
                return Err(NisarSubsetManifestError::ManifestInvalid(format!(
                    "{label} has an invalid offset"
                )));
            }
            let magnitude = i64::from(offset_hour * 3600 + offset_minute * 60);
            offset_seconds = if *sign == b'+' { magnitude } else { -magnitude };
            index += 6;
        }
        _ => {
            return Err(NisarSubsetManifestError::ManifestInvalid(format!(
                "{label} has no RFC3339 timezone"
            )))
        }
    }
    if index != bytes.len() {
        return Err(NisarSubsetManifestError::ManifestInvalid(format!(
            "{label} has trailing data"
        )));
    }
    let days = days_before_year(year) + i64::from(days_before_month(year, month) + day - 1);
    let local_seconds = days * 86_400 + i64::from(hour * 3_600 + minute * 60 + second);
    Ok((local_seconds - offset_seconds, nanoseconds))
}

fn days_before_year(year: u32) -> i64 {
    let previous = i64::from(year - 1);
    previous * 365 + previous / 4 - previous / 100 + previous / 400
}

fn days_before_month(year: u32, month: u32) -> u32 {
    const DAYS: [u32; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    let leap = month > 2
        && (year.is_multiple_of(400) || (year.is_multiple_of(4) && !year.is_multiple_of(100)));
    DAYS[(month - 1) as usize] + u32::from(leap)
}

fn parse_digits(bytes: &[u8], start: usize, width: usize) -> Result<u32, NisarSubsetManifestError> {
    if start + width > bytes.len() || !bytes[start..start + width].iter().all(u8::is_ascii_digit) {
        return Err(NisarSubsetManifestError::ManifestInvalid(
            "timestamp contains non-digits".to_owned(),
        ));
    }
    Ok(bytes[start..start + width]
        .iter()
        .fold(0_u32, |value, digit| value * 10 + u32::from(digit - b'0')))
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        2 if year.is_multiple_of(400) || (year.is_multiple_of(4) && !year.is_multiple_of(100)) => {
            29
        }
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

fn validate_extent(extent: &NisarSubsetActualExtentV1) -> Result<(), NisarSubsetManifestError> {
    for (label, value) in [
        ("actual_extent.line_start", extent.line_start),
        (
            "actual_extent.line_end_exclusive",
            extent.line_end_exclusive,
        ),
        ("actual_extent.sample_start", extent.sample_start),
        (
            "actual_extent.sample_end_exclusive",
            extent.sample_end_exclusive,
        ),
        (
            "actual_extent.component_count",
            extent.component_count as u64,
        ),
    ] {
        validate_safe_integer(value, label)?;
    }
    if extent.line_start >= extent.line_end_exclusive
        || extent.sample_start >= extent.sample_end_exclusive
        || extent.component_count == 0
        || extent.component_count > NISAR_SUBSET_MAX_COMPONENTS
    {
        return Err(NisarSubsetManifestError::WindowInvalid(
            "actual extent is invalid".to_owned(),
        ));
    }
    Ok(())
}

fn validate_mask(mask: &NisarSubsetValidityMaskV1) -> Result<(), NisarSubsetManifestError> {
    for (label, value) in [
        ("mask.component_id", u64::from(mask.component_id)),
        ("mask.line_start", mask.line_start),
        ("mask.line_end_exclusive", mask.line_end_exclusive),
        ("mask.sample_start", mask.sample_start),
        ("mask.sample_end_exclusive", mask.sample_end_exclusive),
        ("mask.shape[0]", mask.shape[0]),
        ("mask.shape[1]", mask.shape[1]),
    ] {
        validate_safe_integer(value, label)?;
    }
    validate_safe_text("mask.dataset_path", &mask.dataset_path, 512)?;
    if !mask.dataset_path.starts_with('/')
        || mask.dataset_path.contains("..")
        || mask.dataset_path.contains('\\')
        || mask.dataset_path != validity_mask_dataset_path(&mask.frequency)
    {
        return Err(NisarSubsetManifestError::WindowInvalid(
            "mask dataset path is invalid".to_owned(),
        ));
    }
    validate_safe_text("mask.swath", &mask.swath, 128)?;
    if mask.component_id >= NISAR_SUBSET_MAX_COMPONENTS
        || mask.line_start >= mask.line_end_exclusive
        || mask.sample_start >= mask.sample_end_exclusive
        || mask.shape
            != [
                mask.line_end_exclusive - mask.line_start,
                mask.sample_end_exclusive - mask.sample_start,
            ]
        || mask.shape.contains(&0)
    {
        return Err(NisarSubsetManifestError::WindowInvalid(
            "mask shape must be non-empty".to_owned(),
        ));
    }
    validate_sha256(&mask.sha256, "validity_masks.sha256")?;
    validate_safe_text("mask.semantics", &mask.semantics, 128)
}

fn validate_artifact_windows_and_masks(
    artifact: &NisarSubsetArtifactManifestV1,
    request: &NisarSubsetRequestManifestV1,
) -> Result<(), NisarSubsetManifestError> {
    let mut request_windows = request.mapping.windows.clone();
    let mut artifact_windows = artifact.dataset_windows.clone();
    request_windows.sort_by(compare_logical_windows);
    artifact_windows.sort_by(compare_logical_windows);
    if request_windows != artifact_windows {
        return Err(NisarSubsetManifestError::IdentityMismatch(
            "artifact windows differ from request".to_owned(),
        ));
    }

    let component_count = request.request.aoi_component_count as usize;
    if artifact.actual_extent.component_count as usize != component_count {
        return Err(NisarSubsetManifestError::WindowInvalid(
            "artifact extent component count differs from request".to_owned(),
        ));
    }
    let mut components = vec![false; component_count];
    let mut line_start = u64::MAX;
    let mut line_end = 0_u64;
    let mut sample_start = u64::MAX;
    let mut sample_end = 0_u64;
    for window in &request_windows {
        let component = window.component_id as usize;
        if component >= component_count {
            return Err(NisarSubsetManifestError::WindowInvalid(
                "window component is outside the request extent".to_owned(),
            ));
        }
        components[component] = true;
        line_start = line_start.min(window.line_start);
        line_end = line_end.max(window.line_end_exclusive);
        sample_start = sample_start.min(window.sample_start);
        sample_end = sample_end.max(window.sample_end_exclusive);
    }
    if components.iter().any(|present| !present)
        || artifact.actual_extent.line_start != line_start
        || artifact.actual_extent.line_end_exclusive != line_end
        || artifact.actual_extent.sample_start != sample_start
        || artifact.actual_extent.sample_end_exclusive != sample_end
    {
        return Err(NisarSubsetManifestError::WindowInvalid(
            "artifact extent does not cover the request components and windows".to_owned(),
        ));
    }

    let required_keys = request_windows
        .iter()
        .filter(|window| window.validity_mask_required)
        .map(|window| {
            (
                window.component_id,
                validity_mask_dataset_path(&window.frequency),
                window.frequency.clone(),
                window.swath.clone(),
                window.line_start,
                window.line_end_exclusive,
                window.sample_start,
                window.sample_end_exclusive,
                window.expected_shape,
            )
        })
        .collect::<Vec<_>>();
    let mut unique_required = required_keys.clone();
    unique_required.sort();
    unique_required.dedup();
    if unique_required.len() != required_keys.len() {
        return Err(NisarSubsetManifestError::ArtifactNotValidated(
            "required windows have duplicate mask identities".to_owned(),
        ));
    }
    let mut artifact_keys = artifact
        .validity_masks
        .iter()
        .map(|mask| {
            (
                mask.component_id,
                mask.dataset_path.clone(),
                mask.frequency.clone(),
                mask.swath.clone(),
                mask.line_start,
                mask.line_end_exclusive,
                mask.sample_start,
                mask.sample_end_exclusive,
                mask.shape,
            )
        })
        .collect::<Vec<_>>();
    artifact_keys.sort();
    if artifact_keys.windows(2).any(|pair| pair[0] == pair[1]) || artifact_keys != unique_required {
        return Err(NisarSubsetManifestError::ArtifactNotValidated(
            "artifact masks are missing, duplicated or unrelated".to_owned(),
        ));
    }
    Ok(())
}

fn validate_science_validation(
    validation: &NisarSubsetScienceValidationV1,
) -> Result<(), NisarSubsetManifestError> {
    for (label, check) in [
        ("structure", &validation.structure),
        ("complex_values", &validation.complex_values),
        ("phase", &validation.phase),
        ("amplitude", &validation.amplitude),
        ("coordinates", &validation.coordinates),
        ("metadata", &validation.metadata),
        ("provenance", &validation.provenance),
        ("recovery", &validation.recovery),
    ] {
        if check.status != NisarSubsetCheckStatus::Passed {
            return Err(NisarSubsetManifestError::ArtifactNotValidated(format!(
                "science check {label} is not passed"
            )));
        }
        validate_safe_text(
            &format!("science_validation.{label}.evidence"),
            &check.evidence,
            512,
        )?;
    }
    Ok(())
}

fn validate_byte_totals(
    remote_bytes: u64,
    full_bytes: u64,
    label: &str,
) -> Result<(), NisarSubsetManifestError> {
    validate_safe_integer(remote_bytes, &format!("{label}.remote_bytes"))?;
    validate_safe_integer(full_bytes, &format!("{label}.full_product_bytes"))?;
    if full_bytes == 0 || remote_bytes == 0 || remote_bytes > full_bytes {
        return Err(NisarSubsetManifestError::RangeInvalid(format!(
            "{label} byte totals are invalid"
        )));
    }
    Ok(())
}

fn validate_polarization(
    frequency: &NisarSubsetFrequency,
    polarization: &str,
) -> Result<(), NisarSubsetManifestError> {
    let allowed = match frequency {
        NisarSubsetFrequency::A => ["HH", "HV"].as_slice(),
        NisarSubsetFrequency::B => ["HH", "HV", "VV"].as_slice(),
    };
    if !allowed.contains(&polarization) {
        return Err(NisarSubsetManifestError::UnsupportedScope(
            "frequency/polarization is outside the closed L-band set".to_owned(),
        ));
    }
    Ok(())
}

fn science_dataset_path(frequency: &NisarSubsetFrequency, polarization: &str) -> String {
    let frequency = match frequency {
        NisarSubsetFrequency::A => "A",
        NisarSubsetFrequency::B => "B",
    };
    format!("/science/LSAR/RSLC/swaths/frequency{frequency}/{polarization}")
}

fn validity_mask_dataset_path(frequency: &NisarSubsetFrequency) -> String {
    let frequency = match frequency {
        NisarSubsetFrequency::A => "A",
        NisarSubsetFrequency::B => "B",
    };
    format!("/science/LSAR/RSLC/swaths/frequency{frequency}/{NISAR_SUBSET_VALIDITY_MASK_SUFFIX}")
}

fn validate_derived_file_name(value: &str, label: &str) -> Result<(), NisarSubsetManifestError> {
    validate_safe_text(label, value, NISAR_SUBSET_MAX_STRING_BYTES)?;
    let lower = value.to_ascii_lowercase();
    if value.contains('/')
        || value.contains('\\')
        || value.ends_with(".part")
        || !(lower.contains("subset") || lower.contains("derived"))
    {
        return Err(NisarSubsetManifestError::IdentityMismatch(format!(
            "{label} must name a derived subset artifact"
        )));
    }
    Ok(())
}

fn validate_sha256(value: &str, label: &str) -> Result<(), NisarSubsetManifestError> {
    if value.len() != 64
        || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
        || value
            .chars()
            .any(|character| character.is_ascii_uppercase())
    {
        return Err(NisarSubsetManifestError::ManifestInvalid(format!(
            "{label} must be a lower-case SHA-256 digest"
        )));
    }
    Ok(())
}

fn validate_safe_integer(value: u64, label: &str) -> Result<(), NisarSubsetManifestError> {
    if value > NISAR_SUBSET_MAX_SAFE_INTEGER {
        return Err(NisarSubsetManifestError::ManifestInvalid(format!(
            "{label} exceeds JavaScript safe integer range"
        )));
    }
    Ok(())
}

fn validate_safe_text(
    label: &str,
    value: &str,
    max_bytes: usize,
) -> Result<(), NisarSubsetManifestError> {
    if value.is_empty() || value.len() > max_bytes || value.chars().any(char::is_control) {
        return Err(NisarSubsetManifestError::ManifestInvalid(format!(
            "{label} is empty, oversized or contains control characters"
        )));
    }
    let lower = value.to_ascii_lowercase();
    if [
        "http://",
        "https://",
        "authorization",
        "cookie",
        "password",
        "secret",
        "token",
        ".netrc",
        "signed_url",
        "signedurl",
    ]
    .iter()
    .any(|forbidden| lower.contains(forbidden))
    {
        return Err(NisarSubsetManifestError::SensitiveField(label.to_owned()));
    }
    Ok(())
}

fn compare_logical_windows(
    left: &NisarSubsetLogicalWindowV1,
    right: &NisarSubsetLogicalWindowV1,
) -> Ordering {
    macro_rules! compare_fields {
        ($first_left:expr, $first_right:expr $(, $left_value:expr, $right_value:expr)* $(,)?) => {{
            let mut ordering = $first_left.cmp(&$first_right);
            $(
                if ordering == Ordering::Equal {
                    ordering = $left_value.cmp(&$right_value);
                }
            )*
            ordering
        }};
    }
    compare_fields!(
        left.component_id,
        right.component_id,
        left.dataset_path,
        right.dataset_path,
        left.frequency,
        right.frequency,
        left.polarization,
        right.polarization,
        left.swath,
        right.swath,
        left.dtype,
        right.dtype,
        left.line_start,
        right.line_start,
        left.line_end_exclusive,
        right.line_end_exclusive,
        left.sample_start,
        right.sample_start,
        left.sample_end_exclusive,
        right.sample_end_exclusive,
        left.source_shape,
        right.source_shape,
        left.expected_shape,
        right.expected_shape,
        left.chunk_shape,
        right.chunk_shape,
        left.margin_lines,
        right.margin_lines,
        left.margin_samples,
        right.margin_samples,
        left.mapping_confidence,
        right.mapping_confidence,
        left.validity_mask_required,
        right.validity_mask_required,
    )
}

fn compare_byte_ranges(left: &NisarSubsetByteRangeV1, right: &NisarSubsetByteRangeV1) -> Ordering {
    match left.start.cmp(&right.start) {
        Ordering::Equal => {}
        other => return other,
    }
    match left.end_exclusive.cmp(&right.end_exclusive) {
        Ordering::Equal => {}
        other => return other,
    }
    match left.expected_bytes.cmp(&right.expected_bytes) {
        Ordering::Equal => {}
        other => return other,
    }
    match left.received_bytes.cmp(&right.received_bytes) {
        Ordering::Equal => {}
        other => return other,
    }
    match left.sha256.cmp(&right.sha256) {
        Ordering::Equal => {}
        other => return other,
    }
    left.state.cmp(&right.state)
}

fn compare_validity_masks(
    left: &NisarSubsetValidityMaskV1,
    right: &NisarSubsetValidityMaskV1,
) -> Ordering {
    match left.dataset_path.cmp(&right.dataset_path) {
        Ordering::Equal => {}
        other => return other,
    }
    match left.component_id.cmp(&right.component_id) {
        Ordering::Equal => {}
        other => return other,
    }
    match left.frequency.cmp(&right.frequency) {
        Ordering::Equal => {}
        other => return other,
    }
    match left.swath.cmp(&right.swath) {
        Ordering::Equal => {}
        other => return other,
    }
    match left.line_start.cmp(&right.line_start) {
        Ordering::Equal => {}
        other => return other,
    }
    match left.line_end_exclusive.cmp(&right.line_end_exclusive) {
        Ordering::Equal => {}
        other => return other,
    }
    match left.sample_start.cmp(&right.sample_start) {
        Ordering::Equal => {}
        other => return other,
    }
    match left.sample_end_exclusive.cmp(&right.sample_end_exclusive) {
        Ordering::Equal => {}
        other => return other,
    }
    match left.shape.cmp(&right.shape) {
        Ordering::Equal => {}
        other => return other,
    }
    match left.sha256.cmp(&right.sha256) {
        Ordering::Equal => {}
        other => return other,
    }
    left.semantics.cmp(&right.semantics)
}

fn canonicalize_json(value: Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut entries = object
                .into_iter()
                .map(|(key, value)| (key, canonicalize_json(value)))
                .collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            Value::Object(Map::from_iter(entries))
        }
        Value::Array(items) => Value::Array(items.into_iter().map(canonicalize_json).collect()),
        other => other,
    }
}

fn sha256_text(value: &str) -> Result<String, NisarSubsetManifestError> {
    Ok(format!("{:x}", Sha256::digest(value.as_bytes())))
}
