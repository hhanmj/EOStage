//! Stable scientific/physical authority for the NISAR live subset plan.
//!
//! `Hdf5RangePlanV1::plan_sha256()` remains the exact execution and recovery
//! identity.  This module deliberately derives a second identity from only
//! source-content, selection, mapping, metadata-inventory, and physical Range
//! facts so catalogue and transport provenance can vary without changing the
//! scientific admission authority.

use insar_core::NisarSubsetRequestManifestV1;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::hdf5_range_transfer::{Hdf5ChunkPhysicalRangeV1, Hdf5RangePlanV1};
use crate::NisarSubsetMetadataSnapshotV1;

pub const NISAR_LIVE_PLAN_SEMANTIC_AUTHORITY_CONTRACT_V1: &str = "NisarLivePlanSemanticAuthorityV1";

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum NisarSemanticPlanAuthorityError {
    #[error("NISAR semantic plan request is invalid")]
    RequestInvalid,
    #[error("NISAR semantic plan source checksum is missing")]
    SourceChecksumMissing,
    #[error("NISAR semantic plan execution request identity is inconsistent")]
    PlanRequestMismatch,
    #[error("NISAR semantic plan source length is inconsistent")]
    SourceLengthMismatch,
    #[error("NISAR semantic plan metadata inventory is invalid")]
    MetadataInventoryInvalid,
    #[error("NISAR semantic plan identity could not be serialized")]
    Serialization,
}

impl NisarSemanticPlanAuthorityError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::RequestInvalid
            | Self::SourceChecksumMissing
            | Self::PlanRequestMismatch
            | Self::SourceLengthMismatch
            | Self::MetadataInventoryInvalid
            | Self::Serialization => "NISAR_SUBSET_SEMANTIC_PLAN_INVALID",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NisarLivePlanSemanticAuthorityV1 {
    semantic_plan_sha256: String,
}

/// Test/forensic view of the exact semantic canonical preimage.
///
/// The section values and digests are produced from the same
/// `SemanticPlanIdentity` that admission hashes. This is intentionally a
/// diagnostic view, not a second canonicalization contract.
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct NisarSemanticPlanCanonicalSnapshotV1 {
    pub canonical_json: String,
    pub source: Value,
    pub request: Value,
    pub mapping: Value,
    pub metadata: Value,
    pub physical: Value,
    pub semantic_source_sha256: String,
    pub semantic_request_sha256: String,
    pub semantic_mapping_sha256: String,
    pub semantic_metadata_sha256: String,
    pub semantic_physical_sha256: String,
    pub semantic_plan_sha256: String,
}

impl NisarLivePlanSemanticAuthorityV1 {
    #[doc(hidden)]
    pub fn canonical_snapshot(
        request: &NisarSubsetRequestManifestV1,
        metadata: &NisarSubsetMetadataSnapshotV1,
        plan: &Hdf5RangePlanV1,
    ) -> Result<NisarSemanticPlanCanonicalSnapshotV1, NisarSemanticPlanAuthorityError> {
        let canonical = SemanticPlanIdentity::from_verified_inputs(request, metadata, plan)?;
        let canonical_json = serde_json::to_string(&canonical)
            .map_err(|_| NisarSemanticPlanAuthorityError::Serialization)?;
        let source_json = serde_json::to_vec(&canonical.source)
            .map_err(|_| NisarSemanticPlanAuthorityError::Serialization)?;
        let request_json = serde_json::to_vec(canonical.request)
            .map_err(|_| NisarSemanticPlanAuthorityError::Serialization)?;
        let mapping_json = serde_json::to_vec(canonical.mapping)
            .map_err(|_| NisarSemanticPlanAuthorityError::Serialization)?;
        let metadata_json = serde_json::to_vec(&canonical.metadata)
            .map_err(|_| NisarSemanticPlanAuthorityError::Serialization)?;
        let physical_json = serde_json::to_vec(&canonical.physical)
            .map_err(|_| NisarSemanticPlanAuthorityError::Serialization)?;
        let source = serde_json::to_value(&canonical.source)
            .map_err(|_| NisarSemanticPlanAuthorityError::Serialization)?;
        let request = serde_json::to_value(canonical.request)
            .map_err(|_| NisarSemanticPlanAuthorityError::Serialization)?;
        let mapping = serde_json::to_value(canonical.mapping)
            .map_err(|_| NisarSemanticPlanAuthorityError::Serialization)?;
        let metadata = serde_json::to_value(&canonical.metadata)
            .map_err(|_| NisarSemanticPlanAuthorityError::Serialization)?;
        let physical = serde_json::to_value(&canonical.physical)
            .map_err(|_| NisarSemanticPlanAuthorityError::Serialization)?;

        Ok(NisarSemanticPlanCanonicalSnapshotV1 {
            canonical_json: canonical_json.clone(),
            source,
            request,
            mapping,
            metadata,
            physical,
            semantic_source_sha256: sha256(&source_json),
            semantic_request_sha256: sha256(&request_json),
            semantic_mapping_sha256: sha256(&mapping_json),
            semantic_metadata_sha256: sha256(&metadata_json),
            semantic_physical_sha256: sha256(&physical_json),
            semantic_plan_sha256: sha256(canonical_json.as_bytes()),
        })
    }

    pub fn derive(
        request: &NisarSubsetRequestManifestV1,
        metadata: &NisarSubsetMetadataSnapshotV1,
        plan: &Hdf5RangePlanV1,
    ) -> Result<Self, NisarSemanticPlanAuthorityError> {
        let snapshot = Self::canonical_snapshot(request, metadata, plan)?;
        Ok(Self {
            semantic_plan_sha256: snapshot.semantic_plan_sha256,
        })
    }

    pub fn semantic_plan_sha256(&self) -> &str {
        &self.semantic_plan_sha256
    }
}

pub fn derive_nisar_live_semantic_plan_sha256(
    request: &NisarSubsetRequestManifestV1,
    metadata: &NisarSubsetMetadataSnapshotV1,
    plan: &Hdf5RangePlanV1,
) -> Result<String, NisarSemanticPlanAuthorityError> {
    Ok(NisarLivePlanSemanticAuthorityV1::derive(request, metadata, plan)?.semantic_plan_sha256)
}

#[doc(hidden)]
pub fn semantic_plan_canonical_snapshot(
    request: &NisarSubsetRequestManifestV1,
    metadata: &NisarSubsetMetadataSnapshotV1,
    plan: &Hdf5RangePlanV1,
) -> Result<NisarSemanticPlanCanonicalSnapshotV1, NisarSemanticPlanAuthorityError> {
    NisarLivePlanSemanticAuthorityV1::canonical_snapshot(request, metadata, plan)
}

#[derive(Serialize)]
struct SemanticPlanIdentity<'a> {
    authority_contract: &'static str,
    source: SemanticSourceIdentity<'a>,
    request: &'a insar_core::NisarSubsetRequestV1,
    mapping: &'a insar_core::NisarSubsetMappingV1,
    metadata: SemanticMetadataAuthority<'a>,
    physical: SemanticPhysicalAuthority<'a>,
}

#[derive(Serialize)]
struct SemanticSourceIdentity<'a> {
    provider_id: &'a str,
    collection_profile_id: &'a str,
    file_name: &'a str,
    expected_size_bytes: u64,
    checksum_algorithm: &'a insar_core::NisarSubsetChecksumAlgorithm,
    checksum_value: &'a str,
    product_specification_version: &'a str,
    crid: &'a str,
}

#[derive(Serialize)]
struct SemanticMetadataAuthority<'a> {
    source_dataset_count: u64,
    metadata_group_binding_count: u64,
    metadata_dataset_binding_count: u64,
    metadata_binding_count: u64,
    inventory_sha256: &'a str,
}

#[derive(Serialize)]
struct SemanticPhysicalAuthority<'a> {
    source_length: u64,
    metadata_prefix_bytes: u64,
    chunks: &'a [Hdf5ChunkPhysicalRangeV1],
    ranges: &'a [crate::hdf5_range_transfer::Hdf5PhysicalRangeV1],
    expected_remote_bytes: u64,
}

impl<'a> SemanticPlanIdentity<'a> {
    fn from_verified_inputs(
        request: &'a NisarSubsetRequestManifestV1,
        metadata: &'a NisarSubsetMetadataSnapshotV1,
        plan: &'a Hdf5RangePlanV1,
    ) -> Result<Self, NisarSemanticPlanAuthorityError> {
        request
            .validate()
            .map_err(|_| NisarSemanticPlanAuthorityError::RequestInvalid)?;
        let request_manifest_sha256 = request
            .sha256()
            .map_err(|_| NisarSemanticPlanAuthorityError::RequestInvalid)?;
        if request_manifest_sha256 != plan.request_manifest_sha256()
            || request.source.expected_size_bytes != request.expected_output.full_product_bytes
            || plan.expected_remote_bytes() != request.expected_output.estimated_remote_bytes
        {
            return Err(NisarSemanticPlanAuthorityError::PlanRequestMismatch);
        }
        if request.source.expected_size_bytes != plan.source_length() {
            return Err(NisarSemanticPlanAuthorityError::SourceLengthMismatch);
        }
        let checksum = request
            .source
            .checksum
            .as_ref()
            .ok_or(NisarSemanticPlanAuthorityError::SourceChecksumMissing)?;
        let metadata_binding_count = metadata
            .metadata_group_binding_count()
            .checked_add(metadata.metadata_dataset_binding_count())
            .ok_or(NisarSemanticPlanAuthorityError::MetadataInventoryInvalid)?;
        if metadata.source_dataset_count() == 0
            || metadata.metadata_binding_count() != metadata_binding_count
            || metadata.source_inventory_sha256().len() != 64
            || !metadata
                .source_inventory_sha256()
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(NisarSemanticPlanAuthorityError::MetadataInventoryInvalid);
        }
        if checksum.value.is_empty() || request.source.file_name.is_empty() {
            return Err(NisarSemanticPlanAuthorityError::RequestInvalid);
        }

        Ok(Self {
            authority_contract: NISAR_LIVE_PLAN_SEMANTIC_AUTHORITY_CONTRACT_V1,
            source: SemanticSourceIdentity {
                provider_id: &request.source.provider_id,
                collection_profile_id: &request.source.collection_profile_id,
                file_name: &request.source.file_name,
                expected_size_bytes: request.source.expected_size_bytes,
                checksum_algorithm: &checksum.algorithm,
                checksum_value: &checksum.value,
                product_specification_version: &request.source.product_specification_version,
                crid: &request.source.crid,
            },
            request: &request.request,
            mapping: &request.mapping,
            metadata: SemanticMetadataAuthority {
                source_dataset_count: metadata.source_dataset_count(),
                metadata_group_binding_count: metadata.metadata_group_binding_count(),
                metadata_dataset_binding_count: metadata.metadata_dataset_binding_count(),
                metadata_binding_count: metadata.metadata_binding_count(),
                inventory_sha256: metadata.source_inventory_sha256(),
            },
            physical: SemanticPhysicalAuthority {
                source_length: plan.source_length(),
                metadata_prefix_bytes: plan.metadata_prefix_bytes(),
                chunks: plan.chunks(),
                ranges: plan.ranges(),
                expected_remote_bytes: plan.expected_remote_bytes(),
            },
        })
    }
}

fn sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
