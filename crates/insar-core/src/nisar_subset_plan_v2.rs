//! Authoritative Phase-1 geometry and Phase-2B prepared-intent contracts for a
//! client-derived NISAR RSLC AOI subset.
//!
//! V1 request/transfer/artifact manifests remain historical execution contracts.
//! This module owns the V2 geometry, frozen prepared-intent authority, and final
//! semantic identity that later consumers must accept without reinterpreting source,
//! requested/effective geometry, frequency, or polarization selection.

use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet, VecDeque},
};

use geo::{Area, BooleanOps, Coord, LineString, MultiPolygon, Polygon, Relate};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{
    polygonal_geojson_map, validated_authoritative_polygonal_geojson, validated_polygonal_geojson,
    NisarSubsetChecksumAlgorithm, NisarSubsetChecksumV1, NisarSubsetFrequency,
};

pub const NISAR_SUBSET_PLAN_V2_CONTRACT: &str = "NisarSubsetPlanV2";
pub const NISAR_SUBSET_PLAN_V2_SCHEMA_VERSION: u32 = 2;
pub const NISAR_SUBSET_PREPARED_INTENT_V2_CONTRACT: &str = "NisarSubsetPreparedIntentV2";
pub const NISAR_SUBSET_PREPARED_INTENT_V2_SCHEMA_VERSION: u32 = 2;
pub const NISAR_SUBSET_PLAN_V2_CRS: &str = "EPSG:4326";
pub const NISAR_SUBSET_PLAN_V2_DEPENDENCY_ALGORITHM: &str = "nisar_rslc_hdf5_dependency_closure_v1";
pub const NISAR_SUBSET_PLAN_V2_DEPENDENCY_ALGORITHM_VERSION: u32 = 1;
pub const NISAR_SUBSET_SOURCE_DEPENDENCY_GRAPH_V2_CONTRACT: &str =
    "NisarSubsetSourceDependencyGraphV2";
pub const NISAR_SUBSET_SOURCE_DEPENDENCY_GRAPH_V2_SCHEMA_VERSION: u32 = 2;
pub const NISAR_SUBSET_PLAN_V2_PRODUCT_SEMANTICS: &str = "NISAR RSLC AOI 子集（客户端派生）";
const NISAR_LSAR_SWATHS_ROOT: &str = "/science/LSAR/RSLC/swaths";
const NISAR_SSAR_SWATHS_ROOT: &str = "/science/SSAR/RSLC/swaths";
pub const NISAR_SUBSET_PLAN_V2_MAX_CANONICAL_JSON_BYTES: usize = 512 * 1024;
const MAX_DEPENDENCIES: usize = 4_096;
const MAX_WARNINGS: usize = 64;
const MAX_TEXT_BYTES: usize = 2_048;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NisarSubsetGeometryRoleV2 {
    SourceFootprint,
    Requested,
    Effective,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NisarSubsetGeometryInvalidReasonV2 {
    InvalidGeoJson,
    InvalidCoordinate,
    CoordinateOutOfRange,
    EmptyGeometry,
    InvalidTopology,
    CanonicalizationFailed,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum NisarSubsetPlanV2Error {
    #[error("NISAR_SUBSET_PLAN_V2_GEOMETRY_INVALID: {geometry:?}/{reason:?}")]
    GeometryInvalid {
        geometry: NisarSubsetGeometryRoleV2,
        reason: NisarSubsetGeometryInvalidReasonV2,
    },
    #[error("NISAR_SUBSET_PLAN_V2_ANTIMERIDIAN_UNSUPPORTED: {geometry:?}")]
    AntimeridianUnsupported { geometry: NisarSubsetGeometryRoleV2 },
    #[error("NISAR_SUBSET_PLAN_V2_EFFECTIVE_GEOMETRY_EMPTY")]
    EmptyEffectiveGeometry,
    #[error("NISAR_SUBSET_PLAN_V2_SOURCE_IDENTITY_INVALID: {0}")]
    SourceIdentityInvalid(String),
    #[error("NISAR_SUBSET_PLAN_V2_SOURCE_CAPABILITY_INVALID: {0}")]
    SourceCapabilityInvalid(String),
    #[error("NISAR_SUBSET_PLAN_V2_POLARIZATION_INVALID: {0}")]
    PolarizationInvalid(String),
    #[error("NISAR_SUBSET_PLAN_V2_DEPENDENCY_CLOSURE_INVALID: {0}")]
    DependencyClosureInvalid(String),
    #[error("NISAR_SUBSET_PLAN_V2_DEPENDENCY_GRAPH_INVALID: {0}")]
    DependencyGraphInvalid(String),
    #[error("NISAR_SUBSET_PLAN_V2_ESTIMATE_INVALID: {0}")]
    EstimateInvalid(String),
    #[error("NISAR_SUBSET_PLAN_V2_IDENTITY_MISMATCH: {0}")]
    IdentityMismatch(String),
    #[error("NISAR_SUBSET_PLAN_V2_CONTRACT_INVALID: {0}")]
    ContractInvalid(String),
    #[error("NISAR_SUBSET_PLAN_V2_TOO_LARGE")]
    PlanTooLarge,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NisarSubsetSourceProductIdentityV2 {
    pub provider_id: String,
    pub collection_id: String,
    pub collection_profile_id: String,
    pub provider_product_id: String,
    pub provider_product_key: String,
    pub product_specification_version: String,
    pub crid: String,
    pub quality_status: NisarSubsetQualityStatusV2,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NisarSubsetSourceAssetIdentityV2 {
    pub asset_key: String,
    pub stable_asset_key: String,
    pub file_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum: Option<NisarSubsetChecksumV1>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum NisarSubsetQualityStatusV2 {
    #[serde(rename = "PROVISIONAL")]
    Provisional,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NisarSubsetGeometryV2 {
    pub crs: String,
    pub canonical_geojson: Map<String, Value>,
    pub component_count: u16,
    pub ring_count: u16,
    pub vertex_count: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NisarSubsetCanonicalGeometryDigestV2 {
    pub requested_sha256: String,
    pub effective_sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NisarSubsetFrequencyCapabilityV2 {
    pub frequency: NisarSubsetFrequency,
    pub available_polarizations: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NisarSubsetSourceCapabilityV2 {
    pub evidence_sha256: String,
    pub frequencies: Vec<NisarSubsetFrequencyCapabilityV2>,
}

/// Identifies the reviewed evidence producer that enumerated the source HDF5 graph.
/// This is not a completeness flag: graph completeness is checked structurally by
/// the planner before it derives a closure.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NisarSubsetDependencyEvidenceAuthorityV2 {
    Hdf5MetadataInventoryV1,
    ReviewedSourceEvidenceV1,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum NisarSubsetSourceDependencyNodeKindV2 {
    ScienceDataset,
    CoordinateDataset,
    DimensionScale,
    ValidityMask,
    IdentificationMetadata,
    ProvenanceMetadata,
    Attribute,
    OtherScientificDependency,
    Unknown,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum NisarSubsetDependencyRelationV2 {
    Coordinate,
    DimensionScale,
    ValidityMask,
    RequiredMetadata,
    Provenance,
    RequiredAttribute,
    OtherScientificDependency,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NisarSubsetSourceDependencyNodeV2 {
    pub source_path: String,
    pub kind: NisarSubsetSourceDependencyNodeKindV2,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attribute_name: Option<String>,
}

/// `dependent_path` requires `dependency_path` through the typed HDF5 relation.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NisarSubsetSourceDependencyEdgeV2 {
    pub dependent_path: String,
    pub dependency_path: String,
    pub relation: NisarSubsetDependencyRelationV2,
}

/// Authoritative HDF5 node/edge evidence. It intentionally has no caller-set
/// `complete`, `unknown_dependency_count`, or `source_graph_sha256` fields.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NisarSubsetSourceDependencyGraphV2 {
    pub graph_contract: String,
    pub schema_version: u32,
    pub evidence_authority: NisarSubsetDependencyEvidenceAuthorityV2,
    pub evidence_sha256: String,
    pub nodes: Vec<NisarSubsetSourceDependencyNodeV2>,
    pub edges: Vec<NisarSubsetSourceDependencyEdgeV2>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum NisarSubsetDependencyKindV2 {
    SelectedScienceDataset,
    CoordinateDataset,
    DimensionScale,
    ValidityMask,
    IdentificationMetadata,
    ProvenanceMetadata,
    Attribute,
    OtherScientificDependency,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NisarSubsetDependencyV2 {
    pub kind: NisarSubsetDependencyKindV2,
    pub source_path: String,
    pub required_by: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NisarSubsetDependencyClosureV2 {
    pub closure_algorithm: String,
    pub closure_algorithm_version: u32,
    pub source_graph_sha256: String,
    pub complete: bool,
    pub unknown_dependency_count: u32,
    pub items: Vec<NisarSubsetDependencyV2>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum NisarSubsetTransferModeV2 {
    #[serde(rename = "remote_range")]
    RemoteRange,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NisarSubsetByteEstimateV2 {
    pub bytes: u64,
    pub evidence_sha256: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum NisarSubsetScientificAdmissionRequirementV2 {
    ExactSourceIdentity,
    CanonicalRequestedGeometry,
    NonEmptyEffectiveIntersection,
    CompleteDependencyClosure,
    RemoteRangeOnly,
    ClientDerivedOutput,
    PostWriteScientificValidation,
    ProvisionalQualityDisclosure,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum NisarSubsetDerivedIdentityAuthorityV2 {
    #[serde(rename = "client_derived")]
    ClientDerived,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NisarSubsetPlanProvenanceV2 {
    pub product_semantics: String,
    pub derived_identity_authority: NisarSubsetDerivedIdentityAuthorityV2,
    pub derived_artifact_id: String,
    pub source_product_identity_sha256: String,
    pub source_asset_identity_sha256: String,
    pub source_footprint_sha256: String,
    pub requested_geometry_sha256: String,
    pub effective_geometry_sha256: String,
    pub plan_identity_sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NisarSubsetPreparedIntentProvenanceV2 {
    pub product_semantics: String,
    pub derived_identity_authority: NisarSubsetDerivedIdentityAuthorityV2,
    pub source_product_identity_sha256: String,
    pub source_asset_identity_sha256: String,
    pub source_footprint_sha256: String,
    pub requested_geometry_sha256: String,
    pub effective_geometry_sha256: String,
    pub intent_identity_sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NisarSubsetPreparedIntentV2 {
    pub intent_contract: String,
    pub schema_version: u32,
    pub source_product_identity: NisarSubsetSourceProductIdentityV2,
    pub source_asset_identity: NisarSubsetSourceAssetIdentityV2,
    pub source_capability: NisarSubsetSourceCapabilityV2,
    pub source_footprint_geometry: NisarSubsetGeometryV2,
    pub source_footprint_sha256: String,
    pub requested_geometry: NisarSubsetGeometryV2,
    pub effective_geometry: NisarSubsetGeometryV2,
    pub selected_frequency: NisarSubsetFrequency,
    pub selected_polarizations: Vec<String>,
    pub transfer_mode: NisarSubsetTransferModeV2,
    pub quality_status: NisarSubsetQualityStatusV2,
    pub derived_identity_authority: NisarSubsetDerivedIdentityAuthorityV2,
    pub product_semantics: String,
    pub pre_metadata_scientific_requirements: Vec<NisarSubsetScientificAdmissionRequirementV2>,
    pub canonical_geometry_digest: NisarSubsetCanonicalGeometryDigestV2,
    pub canonical_intent_sha256: String,
    pub provenance: NisarSubsetPreparedIntentProvenanceV2,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NisarSubsetPreparedIntentBuildInputV2 {
    pub source_product_identity: NisarSubsetSourceProductIdentityV2,
    pub source_asset_identity: NisarSubsetSourceAssetIdentityV2,
    pub source_capability: NisarSubsetSourceCapabilityV2,
    pub source_footprint: Map<String, Value>,
    pub requested_geometry: Map<String, Value>,
    pub selected_frequency: NisarSubsetFrequency,
    pub selected_polarizations: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NisarSubsetPlanMetadataEvidenceV2 {
    pub source_estimate: Option<NisarSubsetByteEstimateV2>,
    pub transfer_estimate: Option<NisarSubsetByteEstimateV2>,
    pub derived_estimate: Option<NisarSubsetByteEstimateV2>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NisarSubsetPlanV2 {
    pub plan_contract: String,
    pub schema_version: u32,
    pub prepared_intent_sha256: String,
    pub source_product_identity: NisarSubsetSourceProductIdentityV2,
    pub source_asset_identity: NisarSubsetSourceAssetIdentityV2,
    pub source_capability: NisarSubsetSourceCapabilityV2,
    pub source_footprint_geometry: NisarSubsetGeometryV2,
    pub source_footprint_sha256: String,
    pub requested_geometry: NisarSubsetGeometryV2,
    pub effective_geometry: NisarSubsetGeometryV2,
    pub selected_frequency: NisarSubsetFrequency,
    pub selected_polarizations: Vec<String>,
    pub source_dependency_graph: NisarSubsetSourceDependencyGraphV2,
    pub required_dependencies: NisarSubsetDependencyClosureV2,
    pub transfer_mode: NisarSubsetTransferModeV2,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_estimate: Option<NisarSubsetByteEstimateV2>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transfer_estimate: Option<NisarSubsetByteEstimateV2>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub derived_estimate: Option<NisarSubsetByteEstimateV2>,
    pub warnings: Vec<String>,
    pub scientific_admission_requirements: Vec<NisarSubsetScientificAdmissionRequirementV2>,
    pub canonical_geometry_digest: NisarSubsetCanonicalGeometryDigestV2,
    pub canonical_semantic_plan_digest: String,
    pub provenance: NisarSubsetPlanProvenanceV2,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NisarSubsetPlanDraftV2 {
    pub source_product_identity: NisarSubsetSourceProductIdentityV2,
    pub source_asset_identity: NisarSubsetSourceAssetIdentityV2,
    pub source_capability: NisarSubsetSourceCapabilityV2,
    pub source_footprint: Map<String, Value>,
    pub requested_geometry: Map<String, Value>,
    pub selected_frequency: NisarSubsetFrequency,
    pub selected_polarizations: Vec<String>,
    pub source_dependency_graph: NisarSubsetSourceDependencyGraphV2,
    pub source_estimate: Option<NisarSubsetByteEstimateV2>,
    pub transfer_estimate: Option<NisarSubsetByteEstimateV2>,
    pub derived_estimate: Option<NisarSubsetByteEstimateV2>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NisarSubsetGeometryAuthorityV2 {
    pub source_footprint_geometry: NisarSubsetGeometryV2,
    pub source_footprint_sha256: String,
    pub requested_geometry: NisarSubsetGeometryV2,
    pub effective_geometry: NisarSubsetGeometryV2,
    pub canonical_geometry_digest: NisarSubsetCanonicalGeometryDigestV2,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct NisarSubsetDependencyAuthorityV2 {
    pub source_dependency_graph: NisarSubsetSourceDependencyGraphV2,
    pub required_dependencies: NisarSubsetDependencyClosureV2,
}

struct CanonicalizedGeometryV2 {
    contract: NisarSubsetGeometryV2,
    geometry: MultiPolygon<f64>,
}

impl NisarSubsetPreparedIntentV2 {
    pub fn build(
        mut input: NisarSubsetPreparedIntentBuildInputV2,
    ) -> Result<Self, NisarSubsetPlanV2Error> {
        validate_source_product_identity(&input.source_product_identity)?;
        validate_source_asset_identity(&input.source_asset_identity)?;
        input.source_capability = normalize_source_capability(input.source_capability)?;
        let selected_polarizations = normalize_polarizations(input.selected_polarizations)?;
        validate_selected_polarizations(
            &input.source_capability,
            &input.selected_frequency,
            &selected_polarizations,
        )?;
        let geometry = derive_nisar_subset_geometry_authority_v2(
            &input.source_footprint,
            &input.requested_geometry,
        )?;
        let mut intent = Self {
            intent_contract: NISAR_SUBSET_PREPARED_INTENT_V2_CONTRACT.to_owned(),
            schema_version: NISAR_SUBSET_PREPARED_INTENT_V2_SCHEMA_VERSION,
            source_product_identity: input.source_product_identity,
            source_asset_identity: input.source_asset_identity,
            source_capability: input.source_capability,
            source_footprint_geometry: geometry.source_footprint_geometry,
            source_footprint_sha256: geometry.source_footprint_sha256,
            requested_geometry: geometry.requested_geometry,
            effective_geometry: geometry.effective_geometry,
            selected_frequency: input.selected_frequency,
            selected_polarizations,
            transfer_mode: NisarSubsetTransferModeV2::RemoteRange,
            quality_status: NisarSubsetQualityStatusV2::Provisional,
            derived_identity_authority: NisarSubsetDerivedIdentityAuthorityV2::ClientDerived,
            product_semantics: NISAR_SUBSET_PLAN_V2_PRODUCT_SEMANTICS.to_owned(),
            pre_metadata_scientific_requirements: canonical_pre_metadata_requirements(),
            canonical_geometry_digest: geometry.canonical_geometry_digest,
            canonical_intent_sha256: String::new(),
            provenance: empty_prepared_intent_provenance(),
        };
        intent.canonical_intent_sha256 = intent.intent_digest()?;
        intent.provenance = intent.expected_provenance()?;
        intent.validate()?;
        Ok(intent)
    }

    pub fn validate(&self) -> Result<(), NisarSubsetPlanV2Error> {
        if self.intent_contract != NISAR_SUBSET_PREPARED_INTENT_V2_CONTRACT
            || self.schema_version != NISAR_SUBSET_PREPARED_INTENT_V2_SCHEMA_VERSION
        {
            return Err(NisarSubsetPlanV2Error::ContractInvalid(
                "prepared intent contract/schema version is not V2".to_owned(),
            ));
        }
        validate_source_product_identity(&self.source_product_identity)?;
        validate_source_asset_identity(&self.source_asset_identity)?;
        if normalize_source_capability(self.source_capability.clone())? != self.source_capability {
            return Err(NisarSubsetPlanV2Error::SourceCapabilityInvalid(
                "source capability is not canonical".to_owned(),
            ));
        }
        let selected = normalize_polarizations(self.selected_polarizations.clone())?;
        if selected != self.selected_polarizations {
            return Err(NisarSubsetPlanV2Error::PolarizationInvalid(
                "selected polarizations are not sorted and unique".to_owned(),
            ));
        }
        validate_selected_polarizations(
            &self.source_capability,
            &self.selected_frequency,
            &self.selected_polarizations,
        )?;
        if self.transfer_mode != NisarSubsetTransferModeV2::RemoteRange
            || self.quality_status != NisarSubsetQualityStatusV2::Provisional
            || self.source_product_identity.quality_status != self.quality_status
            || self.derived_identity_authority
                != NisarSubsetDerivedIdentityAuthorityV2::ClientDerived
            || self.product_semantics != NISAR_SUBSET_PLAN_V2_PRODUCT_SEMANTICS
            || self.pre_metadata_scientific_requirements != canonical_pre_metadata_requirements()
        {
            return Err(NisarSubsetPlanV2Error::ContractInvalid(
                "prepared intent scientific semantics are not canonical".to_owned(),
            ));
        }
        let expected = derive_nisar_subset_geometry_authority_v2(
            &self.source_footprint_geometry.canonical_geojson,
            &self.requested_geometry.canonical_geojson,
        )?;
        if expected.source_footprint_geometry != self.source_footprint_geometry
            || expected.source_footprint_sha256 != self.source_footprint_sha256
            || expected.requested_geometry != self.requested_geometry
            || expected.effective_geometry != self.effective_geometry
            || expected.canonical_geometry_digest != self.canonical_geometry_digest
        {
            return Err(NisarSubsetPlanV2Error::IdentityMismatch(
                "prepared intent geometry authority".to_owned(),
            ));
        }
        validate_sha256(&self.canonical_intent_sha256, "canonical_intent_sha256")?;
        if self.intent_digest()? != self.canonical_intent_sha256 {
            return Err(NisarSubsetPlanV2Error::IdentityMismatch(
                "canonical prepared intent digest".to_owned(),
            ));
        }
        if self.provenance != self.expected_provenance()? {
            return Err(NisarSubsetPlanV2Error::IdentityMismatch(
                "prepared intent provenance binding".to_owned(),
            ));
        }
        canonical_json_bytes(self, NISAR_SUBSET_PLAN_V2_MAX_CANONICAL_JSON_BYTES)?;
        Ok(())
    }

    pub fn canonical_json(&self) -> Result<String, NisarSubsetPlanV2Error> {
        self.validate()?;
        let bytes = canonical_json_bytes(self, NISAR_SUBSET_PLAN_V2_MAX_CANONICAL_JSON_BYTES)?;
        String::from_utf8(bytes).map_err(|_| {
            NisarSubsetPlanV2Error::ContractInvalid("canonical JSON was not UTF-8".to_owned())
        })
    }

    fn intent_digest(&self) -> Result<String, NisarSubsetPlanV2Error> {
        #[derive(Serialize)]
        struct Seed<'a> {
            intent_contract: &'a str,
            schema_version: u32,
            source_product_identity: &'a NisarSubsetSourceProductIdentityV2,
            source_asset_identity: &'a NisarSubsetSourceAssetIdentityV2,
            source_capability: &'a NisarSubsetSourceCapabilityV2,
            source_footprint_sha256: &'a str,
            requested_geometry_sha256: &'a str,
            effective_geometry_sha256: &'a str,
            selected_frequency: &'a NisarSubsetFrequency,
            selected_polarizations: &'a [String],
            transfer_mode: NisarSubsetTransferModeV2,
            quality_status: NisarSubsetQualityStatusV2,
            derived_identity_authority: NisarSubsetDerivedIdentityAuthorityV2,
            product_semantics: &'a str,
            pre_metadata_scientific_requirements:
                &'a [NisarSubsetScientificAdmissionRequirementV2],
        }
        digest_serializable(&Seed {
            intent_contract: &self.intent_contract,
            schema_version: self.schema_version,
            source_product_identity: &self.source_product_identity,
            source_asset_identity: &self.source_asset_identity,
            source_capability: &self.source_capability,
            source_footprint_sha256: &self.source_footprint_sha256,
            requested_geometry_sha256: &self.canonical_geometry_digest.requested_sha256,
            effective_geometry_sha256: &self.canonical_geometry_digest.effective_sha256,
            selected_frequency: &self.selected_frequency,
            selected_polarizations: &self.selected_polarizations,
            transfer_mode: self.transfer_mode,
            quality_status: self.quality_status,
            derived_identity_authority: self.derived_identity_authority,
            product_semantics: &self.product_semantics,
            pre_metadata_scientific_requirements: &self.pre_metadata_scientific_requirements,
        })
    }

    fn expected_provenance(
        &self,
    ) -> Result<NisarSubsetPreparedIntentProvenanceV2, NisarSubsetPlanV2Error> {
        Ok(NisarSubsetPreparedIntentProvenanceV2 {
            product_semantics: NISAR_SUBSET_PLAN_V2_PRODUCT_SEMANTICS.to_owned(),
            derived_identity_authority: NisarSubsetDerivedIdentityAuthorityV2::ClientDerived,
            source_product_identity_sha256: digest_serializable(&self.source_product_identity)?,
            source_asset_identity_sha256: digest_serializable(&self.source_asset_identity)?,
            source_footprint_sha256: self.source_footprint_sha256.clone(),
            requested_geometry_sha256: self.canonical_geometry_digest.requested_sha256.clone(),
            effective_geometry_sha256: self.canonical_geometry_digest.effective_sha256.clone(),
            intent_identity_sha256: self.canonical_intent_sha256.clone(),
        })
    }
}

impl NisarSubsetPlanV2 {
    pub fn build(draft: NisarSubsetPlanDraftV2) -> Result<Self, NisarSubsetPlanV2Error> {
        let NisarSubsetPlanDraftV2 {
            source_product_identity,
            source_asset_identity,
            source_capability,
            source_footprint,
            requested_geometry,
            selected_frequency,
            selected_polarizations,
            source_dependency_graph,
            source_estimate,
            transfer_estimate,
            derived_estimate,
            warnings,
        } = draft;
        let prepared_intent =
            NisarSubsetPreparedIntentV2::build(NisarSubsetPreparedIntentBuildInputV2 {
                source_product_identity,
                source_asset_identity,
                source_capability,
                source_footprint,
                requested_geometry,
                selected_frequency,
                selected_polarizations,
            })?;
        Self::build_from_prepared_intent(
            prepared_intent,
            source_dependency_graph,
            NisarSubsetPlanMetadataEvidenceV2 {
                source_estimate,
                transfer_estimate,
                derived_estimate,
                warnings,
            },
        )
    }

    pub fn build_from_prepared_intent(
        prepared_intent: NisarSubsetPreparedIntentV2,
        source_dependency_graph: NisarSubsetSourceDependencyGraphV2,
        metadata: NisarSubsetPlanMetadataEvidenceV2,
    ) -> Result<Self, NisarSubsetPlanV2Error> {
        prepared_intent.validate()?;
        let dependency = derive_nisar_subset_dependency_authority_v2(
            source_dependency_graph,
            &prepared_intent.source_product_identity,
            &prepared_intent.selected_frequency,
            &prepared_intent.selected_polarizations,
        )?;
        validate_estimate("source_estimate", metadata.source_estimate.as_ref())?;
        validate_estimate("transfer_estimate", metadata.transfer_estimate.as_ref())?;
        validate_estimate("derived_estimate", metadata.derived_estimate.as_ref())?;
        let warnings = normalize_warnings(metadata.warnings)?;

        let mut plan = Self {
            plan_contract: NISAR_SUBSET_PLAN_V2_CONTRACT.to_owned(),
            schema_version: NISAR_SUBSET_PLAN_V2_SCHEMA_VERSION,
            prepared_intent_sha256: prepared_intent.canonical_intent_sha256,
            source_product_identity: prepared_intent.source_product_identity,
            source_asset_identity: prepared_intent.source_asset_identity,
            source_capability: prepared_intent.source_capability,
            source_footprint_geometry: prepared_intent.source_footprint_geometry,
            source_footprint_sha256: prepared_intent.source_footprint_sha256,
            requested_geometry: prepared_intent.requested_geometry,
            effective_geometry: prepared_intent.effective_geometry,
            selected_frequency: prepared_intent.selected_frequency,
            selected_polarizations: prepared_intent.selected_polarizations,
            source_dependency_graph: dependency.source_dependency_graph,
            required_dependencies: dependency.required_dependencies,
            transfer_mode: prepared_intent.transfer_mode,
            source_estimate: metadata.source_estimate,
            transfer_estimate: metadata.transfer_estimate,
            derived_estimate: metadata.derived_estimate,
            warnings,
            scientific_admission_requirements: canonical_admission_requirements(),
            canonical_geometry_digest: prepared_intent.canonical_geometry_digest,
            canonical_semantic_plan_digest: String::new(),
            provenance: empty_provenance(),
        };
        plan.canonical_semantic_plan_digest = plan.semantic_digest()?;
        plan.provenance = plan.expected_provenance()?;
        plan.validate()?;
        Ok(plan)
    }

    pub fn validate(&self) -> Result<(), NisarSubsetPlanV2Error> {
        if self.plan_contract != NISAR_SUBSET_PLAN_V2_CONTRACT
            || self.schema_version != NISAR_SUBSET_PLAN_V2_SCHEMA_VERSION
        {
            return Err(NisarSubsetPlanV2Error::ContractInvalid(
                "plan contract/schema version is not V2".to_owned(),
            ));
        }
        validate_source_product_identity(&self.source_product_identity)?;
        validate_source_asset_identity(&self.source_asset_identity)?;
        if normalize_source_capability(self.source_capability.clone())? != self.source_capability {
            return Err(NisarSubsetPlanV2Error::SourceCapabilityInvalid(
                "source capability is not canonical".to_owned(),
            ));
        }
        let selected = normalize_polarizations(self.selected_polarizations.clone())?;
        if selected != self.selected_polarizations {
            return Err(NisarSubsetPlanV2Error::PolarizationInvalid(
                "selected polarizations are not sorted and unique".to_owned(),
            ));
        }
        validate_selected_polarizations(
            &self.source_capability,
            &self.selected_frequency,
            &self.selected_polarizations,
        )?;
        let dependency = derive_nisar_subset_dependency_authority_v2(
            self.source_dependency_graph.clone(),
            &self.source_product_identity,
            &self.selected_frequency,
            &self.selected_polarizations,
        )?;
        if dependency.source_dependency_graph != self.source_dependency_graph
            || dependency.required_dependencies != self.required_dependencies
        {
            return Err(NisarSubsetPlanV2Error::DependencyClosureInvalid(
                "stored dependency graph or derived closure is not canonical".to_owned(),
            ));
        }
        validate_estimate("source_estimate", self.source_estimate.as_ref())?;
        validate_estimate("transfer_estimate", self.transfer_estimate.as_ref())?;
        validate_estimate("derived_estimate", self.derived_estimate.as_ref())?;
        if normalize_warnings(self.warnings.clone())? != self.warnings {
            return Err(NisarSubsetPlanV2Error::ContractInvalid(
                "warnings are not sorted and unique".to_owned(),
            ));
        }
        if self.scientific_admission_requirements != canonical_admission_requirements() {
            return Err(NisarSubsetPlanV2Error::ContractInvalid(
                "scientific admission requirements are incomplete".to_owned(),
            ));
        }

        let source = canonicalize_nisar_subset_geojson_internal(
            &self.source_footprint_geometry.canonical_geojson,
            NisarSubsetGeometryRoleV2::SourceFootprint,
        )?;
        let requested = canonicalize_nisar_subset_geojson_internal(
            &self.requested_geometry.canonical_geojson,
            NisarSubsetGeometryRoleV2::Requested,
        )?;
        let effective = canonicalize_nisar_subset_geojson_internal(
            &self.effective_geometry.canonical_geojson,
            NisarSubsetGeometryRoleV2::Effective,
        )?;
        if source.contract != self.source_footprint_geometry
            || requested.contract != self.requested_geometry
            || effective.contract != self.effective_geometry
        {
            return Err(NisarSubsetPlanV2Error::IdentityMismatch(
                "stored geometry is not canonical".to_owned(),
            ));
        }
        if effective.geometry.unsigned_area() <= 0.0 {
            return Err(NisarSubsetPlanV2Error::EmptyEffectiveGeometry);
        }
        validate_sha256(&self.source_footprint_sha256, "source_footprint_sha256")?;
        if geometry_digest(&self.source_footprint_geometry)? != self.source_footprint_sha256 {
            return Err(NisarSubsetPlanV2Error::IdentityMismatch(
                "source footprint digest".to_owned(),
            ));
        }
        if !source.geometry.relate(&effective.geometry).is_covers() {
            return Err(NisarSubsetPlanV2Error::IdentityMismatch(
                "effective geometry extends outside the frozen source footprint".to_owned(),
            ));
        }
        if !requested.geometry.relate(&effective.geometry).is_covers() {
            return Err(NisarSubsetPlanV2Error::IdentityMismatch(
                "effective geometry extends outside the requested geometry".to_owned(),
            ));
        }
        let expected = derive_nisar_subset_geometry_authority_v2(
            &self.source_footprint_geometry.canonical_geojson,
            &self.requested_geometry.canonical_geojson,
        )?;
        let expected_effective = canonicalize_nisar_subset_geojson_internal(
            &expected.effective_geometry.canonical_geojson,
            NisarSubsetGeometryRoleV2::Effective,
        )?;
        // Boolean intersection coordinates can vary at native floating-point
        // boundaries. Require exact topological equality plus both containment
        // invariants instead of making persisted validity platform-byte-specific.
        if !expected_effective
            .geometry
            .relate(&effective.geometry)
            .is_covers()
            || !effective
                .geometry
                .relate(&expected_effective.geometry)
                .is_covers()
        {
            return Err(NisarSubsetPlanV2Error::IdentityMismatch(
                "effective geometry is not the source/request intersection".to_owned(),
            ));
        }
        let geometry_digest = NisarSubsetCanonicalGeometryDigestV2 {
            requested_sha256: geometry_digest(&self.requested_geometry)?,
            effective_sha256: geometry_digest(&self.effective_geometry)?,
        };
        if geometry_digest != self.canonical_geometry_digest {
            return Err(NisarSubsetPlanV2Error::IdentityMismatch(
                "canonical geometry digest".to_owned(),
            ));
        }
        let mut prepared_intent = NisarSubsetPreparedIntentV2 {
            intent_contract: NISAR_SUBSET_PREPARED_INTENT_V2_CONTRACT.to_owned(),
            schema_version: NISAR_SUBSET_PREPARED_INTENT_V2_SCHEMA_VERSION,
            source_product_identity: self.source_product_identity.clone(),
            source_asset_identity: self.source_asset_identity.clone(),
            source_capability: self.source_capability.clone(),
            source_footprint_geometry: self.source_footprint_geometry.clone(),
            source_footprint_sha256: self.source_footprint_sha256.clone(),
            requested_geometry: self.requested_geometry.clone(),
            effective_geometry: self.effective_geometry.clone(),
            selected_frequency: self.selected_frequency.clone(),
            selected_polarizations: self.selected_polarizations.clone(),
            transfer_mode: self.transfer_mode,
            quality_status: self.source_product_identity.quality_status,
            derived_identity_authority: NisarSubsetDerivedIdentityAuthorityV2::ClientDerived,
            product_semantics: NISAR_SUBSET_PLAN_V2_PRODUCT_SEMANTICS.to_owned(),
            pre_metadata_scientific_requirements: canonical_pre_metadata_requirements(),
            canonical_geometry_digest: self.canonical_geometry_digest.clone(),
            canonical_intent_sha256: self.prepared_intent_sha256.clone(),
            provenance: empty_prepared_intent_provenance(),
        };
        prepared_intent.provenance = prepared_intent.expected_provenance()?;
        prepared_intent.validate().map_err(|error| {
            NisarSubsetPlanV2Error::IdentityMismatch(format!("prepared intent binding: {error}"))
        })?;
        validate_sha256(
            &self.canonical_semantic_plan_digest,
            "canonical_semantic_plan_digest",
        )?;
        if self.semantic_digest()? != self.canonical_semantic_plan_digest {
            return Err(NisarSubsetPlanV2Error::IdentityMismatch(
                "canonical semantic plan digest".to_owned(),
            ));
        }
        if self.provenance != self.expected_provenance()? {
            return Err(NisarSubsetPlanV2Error::IdentityMismatch(
                "plan provenance binding".to_owned(),
            ));
        }
        canonical_json_bytes(self, NISAR_SUBSET_PLAN_V2_MAX_CANONICAL_JSON_BYTES)?;
        Ok(())
    }

    pub fn canonical_json(&self) -> Result<String, NisarSubsetPlanV2Error> {
        self.validate()?;
        let bytes = canonical_json_bytes(self, NISAR_SUBSET_PLAN_V2_MAX_CANONICAL_JSON_BYTES)?;
        String::from_utf8(bytes).map_err(|_| {
            NisarSubsetPlanV2Error::ContractInvalid("canonical JSON was not UTF-8".to_owned())
        })
    }

    fn semantic_digest(&self) -> Result<String, NisarSubsetPlanV2Error> {
        #[derive(Serialize)]
        struct Seed<'a> {
            plan_contract: &'a str,
            schema_version: u32,
            prepared_intent_sha256: &'a str,
            source_product_identity: &'a NisarSubsetSourceProductIdentityV2,
            source_asset_identity: &'a NisarSubsetSourceAssetIdentityV2,
            source_capability: &'a NisarSubsetSourceCapabilityV2,
            source_footprint_sha256: &'a str,
            requested_geometry_sha256: &'a str,
            effective_geometry_sha256: &'a str,
            selected_frequency: &'a NisarSubsetFrequency,
            selected_polarizations: &'a [String],
            required_dependencies: &'a NisarSubsetDependencyClosureV2,
            transfer_mode: NisarSubsetTransferModeV2,
            scientific_admission_requirements: &'a [NisarSubsetScientificAdmissionRequirementV2],
        }
        digest_serializable(&Seed {
            plan_contract: &self.plan_contract,
            schema_version: self.schema_version,
            prepared_intent_sha256: &self.prepared_intent_sha256,
            source_product_identity: &self.source_product_identity,
            source_asset_identity: &self.source_asset_identity,
            source_capability: &self.source_capability,
            source_footprint_sha256: &self.source_footprint_sha256,
            requested_geometry_sha256: &self.canonical_geometry_digest.requested_sha256,
            effective_geometry_sha256: &self.canonical_geometry_digest.effective_sha256,
            selected_frequency: &self.selected_frequency,
            selected_polarizations: &self.selected_polarizations,
            required_dependencies: &self.required_dependencies,
            transfer_mode: self.transfer_mode,
            scientific_admission_requirements: &self.scientific_admission_requirements,
        })
    }

    fn expected_provenance(&self) -> Result<NisarSubsetPlanProvenanceV2, NisarSubsetPlanV2Error> {
        Ok(NisarSubsetPlanProvenanceV2 {
            product_semantics: NISAR_SUBSET_PLAN_V2_PRODUCT_SEMANTICS.to_owned(),
            derived_identity_authority: NisarSubsetDerivedIdentityAuthorityV2::ClientDerived,
            derived_artifact_id: format!(
                "nisar-rslc-subset-{}",
                &self.canonical_semantic_plan_digest[..24]
            ),
            source_product_identity_sha256: digest_serializable(&self.source_product_identity)?,
            source_asset_identity_sha256: digest_serializable(&self.source_asset_identity)?,
            source_footprint_sha256: self.source_footprint_sha256.clone(),
            requested_geometry_sha256: self.canonical_geometry_digest.requested_sha256.clone(),
            effective_geometry_sha256: self.canonical_geometry_digest.effective_sha256.clone(),
            plan_identity_sha256: self.canonical_semantic_plan_digest.clone(),
        })
    }
}

pub fn derive_nisar_subset_geometry_authority_v2(
    source_footprint: &Map<String, Value>,
    requested_geometry: &Map<String, Value>,
) -> Result<NisarSubsetGeometryAuthorityV2, NisarSubsetPlanV2Error> {
    let source = canonicalize_nisar_subset_geojson_internal(
        source_footprint,
        NisarSubsetGeometryRoleV2::SourceFootprint,
    )?;
    let requested = canonicalize_nisar_subset_geojson_internal(
        requested_geometry,
        NisarSubsetGeometryRoleV2::Requested,
    )?;
    let effective_geometry = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if source.geometry.relate(&requested.geometry).is_covers() {
            requested.geometry.clone()
        } else if requested.geometry.relate(&source.geometry).is_covers() {
            source.geometry.clone()
        } else {
            source.geometry.intersection(&requested.geometry)
        }
    }))
    .map_err(|_| {
        geometry_error(
            NisarSubsetGeometryRoleV2::Effective,
            NisarSubsetGeometryInvalidReasonV2::CanonicalizationFailed,
        )
    })?;
    let area = effective_geometry.unsigned_area();
    if effective_geometry.0.is_empty() || !area.is_finite() || area <= 0.0 {
        return Err(NisarSubsetPlanV2Error::EmptyEffectiveGeometry);
    }
    let effective = canonicalize_nisar_subset_multipolygon_internal(
        &effective_geometry,
        NisarSubsetGeometryRoleV2::Effective,
    )?;
    let source_footprint_sha256 = geometry_digest(&source.contract)?;
    let canonical_geometry_digest = NisarSubsetCanonicalGeometryDigestV2 {
        requested_sha256: geometry_digest(&requested.contract)?,
        effective_sha256: geometry_digest(&effective.contract)?,
    };
    Ok(NisarSubsetGeometryAuthorityV2 {
        source_footprint_geometry: source.contract,
        source_footprint_sha256,
        requested_geometry: requested.contract,
        effective_geometry: effective.contract,
        canonical_geometry_digest,
    })
}

pub fn canonicalize_nisar_subset_geojson_v2(
    value: &Map<String, Value>,
    role: NisarSubsetGeometryRoleV2,
) -> Result<NisarSubsetGeometryV2, NisarSubsetPlanV2Error> {
    canonicalize_nisar_subset_geojson_internal(value, role).map(|value| value.contract)
}

pub fn canonicalize_nisar_subset_multipolygon_v2(
    value: &MultiPolygon<f64>,
    role: NisarSubsetGeometryRoleV2,
) -> Result<NisarSubsetGeometryV2, NisarSubsetPlanV2Error> {
    canonicalize_nisar_subset_multipolygon_internal(value, role).map(|value| value.contract)
}

fn validated_subset_polygonal_geojson(
    value: &Map<String, Value>,
    role: NisarSubsetGeometryRoleV2,
) -> Option<MultiPolygon<f64>> {
    if let Some(geometry) = validated_polygonal_geojson(value) {
        return Some(geometry);
    }
    match role {
        NisarSubsetGeometryRoleV2::SourceFootprint => None,
        NisarSubsetGeometryRoleV2::Requested | NisarSubsetGeometryRoleV2::Effective => {
            validated_authoritative_polygonal_geojson(value).map(|validated| validated.geometry)
        }
    }
}

fn canonicalize_nisar_subset_geojson_internal(
    value: &Map<String, Value>,
    role: NisarSubsetGeometryRoleV2,
) -> Result<CanonicalizedGeometryV2, NisarSubsetPlanV2Error> {
    if value.len() != 2 || !value.contains_key("type") || !value.contains_key("coordinates") {
        return Err(geometry_error(
            role,
            NisarSubsetGeometryInvalidReasonV2::InvalidGeoJson,
        ));
    }
    preflight_geojson_coordinates(value, role)?;
    let geometry = validated_subset_polygonal_geojson(value, role)
        .ok_or_else(|| geometry_error(role, NisarSubsetGeometryInvalidReasonV2::InvalidTopology))?;
    reject_antimeridian(&geometry, role)?;
    canonicalize_valid_geometry(geometry, role)
}

fn canonicalize_nisar_subset_multipolygon_internal(
    value: &MultiPolygon<f64>,
    role: NisarSubsetGeometryRoleV2,
) -> Result<CanonicalizedGeometryV2, NisarSubsetPlanV2Error> {
    validate_coordinate_domain(value, role)?;
    reject_antimeridian(value, role)?;
    let map = polygonal_geojson_map(value, value.0.len() != 1).ok_or_else(|| {
        geometry_error(
            role,
            NisarSubsetGeometryInvalidReasonV2::CanonicalizationFailed,
        )
    })?;
    let geometry = validated_subset_polygonal_geojson(&map, role)
        .ok_or_else(|| geometry_error(role, NisarSubsetGeometryInvalidReasonV2::InvalidTopology))?;
    canonicalize_valid_geometry(geometry, role)
}

fn canonicalize_valid_geometry(
    geometry: MultiPolygon<f64>,
    role: NisarSubsetGeometryRoleV2,
) -> Result<CanonicalizedGeometryV2, NisarSubsetPlanV2Error> {
    if geometry.0.is_empty() {
        return Err(geometry_error(
            role,
            NisarSubsetGeometryInvalidReasonV2::EmptyGeometry,
        ));
    }
    let mut polygons = geometry
        .0
        .into_iter()
        .map(canonical_polygon)
        .collect::<Vec<_>>();
    polygons.sort_by(compare_polygons);
    let geometry = MultiPolygon(polygons);
    let canonical_geojson =
        polygonal_geojson_map(&geometry, geometry.0.len() != 1).ok_or_else(|| {
            geometry_error(
                role,
                NisarSubsetGeometryInvalidReasonV2::CanonicalizationFailed,
            )
        })?;
    let geometry = validated_subset_polygonal_geojson(&canonical_geojson, role)
        .ok_or_else(|| geometry_error(role, NisarSubsetGeometryInvalidReasonV2::InvalidTopology))?;
    let ring_count = geometry
        .0
        .iter()
        .try_fold(0usize, |count, polygon| {
            count.checked_add(1 + polygon.interiors().len())
        })
        .and_then(|count| u16::try_from(count).ok())
        .ok_or_else(|| geometry_error(role, NisarSubsetGeometryInvalidReasonV2::InvalidTopology))?;
    let vertex_count = geometry
        .0
        .iter()
        .flat_map(|polygon| std::iter::once(polygon.exterior()).chain(polygon.interiors().iter()))
        .try_fold(0usize, |count, ring| count.checked_add(ring.0.len()))
        .and_then(|count| u32::try_from(count).ok())
        .ok_or_else(|| geometry_error(role, NisarSubsetGeometryInvalidReasonV2::InvalidTopology))?;
    let component_count = u16::try_from(geometry.0.len())
        .map_err(|_| geometry_error(role, NisarSubsetGeometryInvalidReasonV2::InvalidTopology))?;
    Ok(CanonicalizedGeometryV2 {
        contract: NisarSubsetGeometryV2 {
            crs: NISAR_SUBSET_PLAN_V2_CRS.to_owned(),
            canonical_geojson,
            component_count,
            ring_count,
            vertex_count,
        },
        geometry,
    })
}

fn preflight_geojson_coordinates(
    value: &Map<String, Value>,
    role: NisarSubsetGeometryRoleV2,
) -> Result<(), NisarSubsetPlanV2Error> {
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| geometry_error(role, NisarSubsetGeometryInvalidReasonV2::InvalidGeoJson))?;
    let coordinates = value
        .get("coordinates")
        .and_then(Value::as_array)
        .ok_or_else(|| geometry_error(role, NisarSubsetGeometryInvalidReasonV2::InvalidGeoJson))?;
    match kind {
        "Polygon" => preflight_polygon(coordinates, role),
        "MultiPolygon" => {
            if coordinates.is_empty() {
                return Err(geometry_error(
                    role,
                    NisarSubsetGeometryInvalidReasonV2::EmptyGeometry,
                ));
            }
            for polygon in coordinates {
                let polygon = polygon.as_array().ok_or_else(|| {
                    geometry_error(role, NisarSubsetGeometryInvalidReasonV2::InvalidGeoJson)
                })?;
                preflight_polygon(polygon, role)?;
            }
            Ok(())
        }
        _ => Err(geometry_error(
            role,
            NisarSubsetGeometryInvalidReasonV2::InvalidGeoJson,
        )),
    }
}

fn preflight_polygon(
    rings: &[Value],
    role: NisarSubsetGeometryRoleV2,
) -> Result<(), NisarSubsetPlanV2Error> {
    if rings.is_empty() {
        return Err(geometry_error(
            role,
            NisarSubsetGeometryInvalidReasonV2::EmptyGeometry,
        ));
    }
    for ring in rings {
        let positions = ring.as_array().ok_or_else(|| {
            geometry_error(role, NisarSubsetGeometryInvalidReasonV2::InvalidGeoJson)
        })?;
        if positions.len() < 4 {
            return Err(geometry_error(
                role,
                NisarSubsetGeometryInvalidReasonV2::InvalidTopology,
            ));
        }
        for position in positions {
            let position = position
                .as_array()
                .filter(|position| position.len() == 2)
                .ok_or_else(|| {
                    geometry_error(role, NisarSubsetGeometryInvalidReasonV2::InvalidCoordinate)
                })?;
            let longitude = position[0].as_f64().ok_or_else(|| {
                geometry_error(role, NisarSubsetGeometryInvalidReasonV2::InvalidCoordinate)
            })?;
            let latitude = position[1].as_f64().ok_or_else(|| {
                geometry_error(role, NisarSubsetGeometryInvalidReasonV2::InvalidCoordinate)
            })?;
            if !longitude.is_finite() || !latitude.is_finite() {
                return Err(geometry_error(
                    role,
                    NisarSubsetGeometryInvalidReasonV2::InvalidCoordinate,
                ));
            }
            if !(-180.0..=180.0).contains(&longitude) || !(-90.0..=90.0).contains(&latitude) {
                return Err(geometry_error(
                    role,
                    NisarSubsetGeometryInvalidReasonV2::CoordinateOutOfRange,
                ));
            }
        }
    }
    Ok(())
}

fn validate_coordinate_domain(
    geometry: &MultiPolygon<f64>,
    role: NisarSubsetGeometryRoleV2,
) -> Result<(), NisarSubsetPlanV2Error> {
    if geometry.0.is_empty() {
        return Err(geometry_error(
            role,
            NisarSubsetGeometryInvalidReasonV2::EmptyGeometry,
        ));
    }
    for coordinate in geometry_coordinates(geometry) {
        if !coordinate.x.is_finite() || !coordinate.y.is_finite() {
            return Err(geometry_error(
                role,
                NisarSubsetGeometryInvalidReasonV2::InvalidCoordinate,
            ));
        }
        if !(-180.0..=180.0).contains(&coordinate.x) || !(-90.0..=90.0).contains(&coordinate.y) {
            return Err(geometry_error(
                role,
                NisarSubsetGeometryInvalidReasonV2::CoordinateOutOfRange,
            ));
        }
    }
    Ok(())
}

fn reject_antimeridian(
    geometry: &MultiPolygon<f64>,
    role: NisarSubsetGeometryRoleV2,
) -> Result<(), NisarSubsetPlanV2Error> {
    let crosses = geometry.0.iter().any(|polygon| {
        std::iter::once(polygon.exterior())
            .chain(polygon.interiors().iter())
            .any(|ring| {
                ring.0
                    .windows(2)
                    .any(|edge| (edge[1].x - edge[0].x).abs() > 180.0)
            })
    });
    if crosses {
        Err(NisarSubsetPlanV2Error::AntimeridianUnsupported { geometry: role })
    } else {
        Ok(())
    }
}

fn geometry_coordinates(geometry: &MultiPolygon<f64>) -> impl Iterator<Item = &Coord<f64>> {
    geometry.0.iter().flat_map(|polygon| {
        std::iter::once(polygon.exterior())
            .chain(polygon.interiors().iter())
            .flat_map(|ring| ring.0.iter())
    })
}

fn canonical_polygon(polygon: Polygon<f64>) -> Polygon<f64> {
    let exterior = canonical_ring(polygon.exterior());
    let mut interiors = polygon
        .interiors()
        .iter()
        .map(canonical_ring)
        .collect::<Vec<_>>();
    interiors.sort_by(compare_rings);
    Polygon::new(exterior, interiors)
}

fn canonical_ring(ring: &LineString<f64>) -> LineString<f64> {
    let points = ring.0[..ring.0.len() - 1]
        .iter()
        .copied()
        .map(normalize_coordinate)
        .collect::<Vec<_>>();
    let forward = minimum_rotation(&points);
    let mut reversed_points = points;
    reversed_points.reverse();
    let reversed = minimum_rotation(&reversed_points);
    let mut canonical = if compare_coordinate_slices(&forward, &reversed) == Ordering::Greater {
        reversed
    } else {
        forward
    };
    canonical.push(canonical[0]);
    LineString::new(canonical)
}

fn normalize_coordinate(mut coordinate: Coord<f64>) -> Coord<f64> {
    if coordinate.x == 0.0 {
        coordinate.x = 0.0;
    }
    if coordinate.y == 0.0 {
        coordinate.y = 0.0;
    }
    coordinate
}

fn minimum_rotation(points: &[Coord<f64>]) -> Vec<Coord<f64>> {
    let best_start = (0..points.len())
        .min_by(|&left, &right| compare_rotated_coordinate_slices(points, left, right))
        .expect("validated ring has at least three coordinates");
    points[best_start..]
        .iter()
        .chain(&points[..best_start])
        .copied()
        .collect()
}

fn compare_rotated_coordinate_slices(
    points: &[Coord<f64>],
    left_start: usize,
    right_start: usize,
) -> Ordering {
    if left_start == right_start {
        return Ordering::Equal;
    }
    let left_iter = points[left_start..].iter().chain(&points[..left_start]);
    let right_iter = points[right_start..].iter().chain(&points[..right_start]);
    left_iter
        .zip(right_iter)
        .find_map(|(left, right)| {
            let ordering = left
                .x
                .total_cmp(&right.x)
                .then_with(|| left.y.total_cmp(&right.y));
            (ordering != Ordering::Equal).then_some(ordering)
        })
        .unwrap_or(Ordering::Equal)
}

fn compare_coordinate_slices(left: &[Coord<f64>], right: &[Coord<f64>]) -> Ordering {
    left.iter()
        .zip(right)
        .find_map(|(left, right)| {
            let ordering = left
                .x
                .total_cmp(&right.x)
                .then_with(|| left.y.total_cmp(&right.y));
            (ordering != Ordering::Equal).then_some(ordering)
        })
        .unwrap_or_else(|| left.len().cmp(&right.len()))
}

fn compare_rings(left: &LineString<f64>, right: &LineString<f64>) -> Ordering {
    compare_coordinate_slices(&left.0, &right.0)
}

fn compare_polygons(left: &Polygon<f64>, right: &Polygon<f64>) -> Ordering {
    let exterior = compare_rings(left.exterior(), right.exterior());
    if exterior != Ordering::Equal {
        return exterior;
    }
    left.interiors()
        .iter()
        .zip(right.interiors())
        .find_map(|(left, right)| {
            let ordering = compare_rings(left, right);
            (ordering != Ordering::Equal).then_some(ordering)
        })
        .unwrap_or_else(|| left.interiors().len().cmp(&right.interiors().len()))
}

fn normalize_source_capability(
    mut capability: NisarSubsetSourceCapabilityV2,
) -> Result<NisarSubsetSourceCapabilityV2, NisarSubsetPlanV2Error> {
    validate_sha256(
        &capability.evidence_sha256,
        "source_capability.evidence_sha256",
    )?;
    if capability.frequencies.is_empty() {
        return Err(NisarSubsetPlanV2Error::SourceCapabilityInvalid(
            "no source frequency is available".to_owned(),
        ));
    }
    for frequency in &mut capability.frequencies {
        frequency.available_polarizations =
            normalize_polarizations(std::mem::take(&mut frequency.available_polarizations))?;
    }
    capability
        .frequencies
        .sort_by(|left, right| left.frequency.cmp(&right.frequency));
    if capability
        .frequencies
        .windows(2)
        .any(|pair| pair[0].frequency == pair[1].frequency)
    {
        return Err(NisarSubsetPlanV2Error::SourceCapabilityInvalid(
            "duplicate source frequency capability".to_owned(),
        ));
    }
    Ok(capability)
}

fn normalize_polarizations(
    polarizations: Vec<String>,
) -> Result<Vec<String>, NisarSubsetPlanV2Error> {
    let mut normalized = BTreeSet::new();
    for polarization in polarizations {
        let value = polarization.trim().to_ascii_uppercase();
        if !matches!(
            value.as_str(),
            "HH" | "HV" | "VH" | "VV" | "LH" | "LV" | "RH" | "RV"
        ) {
            return Err(NisarSubsetPlanV2Error::PolarizationInvalid(value));
        }
        normalized.insert(value);
    }
    if normalized.is_empty() {
        return Err(NisarSubsetPlanV2Error::PolarizationInvalid(
            "at least one polarization is required".to_owned(),
        ));
    }
    Ok(normalized.into_iter().collect())
}

fn validate_selected_polarizations(
    capability: &NisarSubsetSourceCapabilityV2,
    frequency: &NisarSubsetFrequency,
    selected: &[String],
) -> Result<(), NisarSubsetPlanV2Error> {
    let available = capability
        .frequencies
        .iter()
        .find(|candidate| &candidate.frequency == frequency)
        .ok_or_else(|| {
            NisarSubsetPlanV2Error::SourceCapabilityInvalid(
                "selected frequency is absent from the source".to_owned(),
            )
        })?;
    if selected
        .iter()
        .any(|polarization| !available.available_polarizations.contains(polarization))
    {
        return Err(NisarSubsetPlanV2Error::PolarizationInvalid(
            "selected polarization is absent from the source".to_owned(),
        ));
    }
    Ok(())
}

/// Derives a complete, canonical closure from authoritative source graph evidence.
/// Callers cannot provide closure completeness or graph identity through this boundary.
pub fn derive_nisar_subset_dependency_closure_v2(
    source_graph: &NisarSubsetSourceDependencyGraphV2,
    frequency: &NisarSubsetFrequency,
    polarizations: &[String],
) -> Result<NisarSubsetDependencyClosureV2, NisarSubsetPlanV2Error> {
    let canonical_graph = normalize_source_dependency_graph(source_graph.clone())?;
    let swaths_root = dependency_graph_swaths_root(&canonical_graph)?;
    derive_dependency_closure_from_canonical_graph(
        &canonical_graph,
        swaths_root,
        frequency,
        polarizations,
    )
}

fn derive_nisar_subset_dependency_authority_v2(
    source_graph: NisarSubsetSourceDependencyGraphV2,
    source_identity: &NisarSubsetSourceProductIdentityV2,
    frequency: &NisarSubsetFrequency,
    polarizations: &[String],
) -> Result<NisarSubsetDependencyAuthorityV2, NisarSubsetPlanV2Error> {
    let source_dependency_graph = normalize_source_dependency_graph(source_graph)?;
    let swaths_root = source_swaths_root(source_identity)?;
    let required_dependencies = derive_dependency_closure_from_canonical_graph(
        &source_dependency_graph,
        swaths_root,
        frequency,
        polarizations,
    )?;
    Ok(NisarSubsetDependencyAuthorityV2 {
        source_dependency_graph,
        required_dependencies,
    })
}

fn normalize_source_dependency_graph(
    mut graph: NisarSubsetSourceDependencyGraphV2,
) -> Result<NisarSubsetSourceDependencyGraphV2, NisarSubsetPlanV2Error> {
    if graph.graph_contract != NISAR_SUBSET_SOURCE_DEPENDENCY_GRAPH_V2_CONTRACT
        || graph.schema_version != NISAR_SUBSET_SOURCE_DEPENDENCY_GRAPH_V2_SCHEMA_VERSION
    {
        return Err(NisarSubsetPlanV2Error::DependencyGraphInvalid(
            "source dependency graph contract/schema is not V2".to_owned(),
        ));
    }
    validate_sha256(
        &graph.evidence_sha256,
        "source_dependency_graph.evidence_sha256",
    )
    .map_err(|error| NisarSubsetPlanV2Error::DependencyGraphInvalid(error.to_string()))?;
    if graph.nodes.is_empty() || graph.nodes.len() > MAX_DEPENDENCIES {
        return Err(NisarSubsetPlanV2Error::DependencyGraphInvalid(
            "source dependency node count is outside the closed bound".to_owned(),
        ));
    }
    for node in &graph.nodes {
        validate_hdf5_path(&node.source_path)
            .map_err(|error| NisarSubsetPlanV2Error::DependencyGraphInvalid(error.to_string()))?;
        if node.kind == NisarSubsetSourceDependencyNodeKindV2::Unknown {
            return Err(NisarSubsetPlanV2Error::DependencyGraphInvalid(format!(
                "unknown or unclassified source node: {}",
                node.source_path
            )));
        }
        if node.source_path == "/" && node.kind != NisarSubsetSourceDependencyNodeKindV2::Attribute
        {
            return Err(NisarSubsetPlanV2Error::DependencyGraphInvalid(
                "the canonical HDF5 root is reserved for an attribute owner".to_owned(),
            ));
        }
        match (node.kind, node.attribute_name.as_deref()) {
            (NisarSubsetSourceDependencyNodeKindV2::Attribute, Some(name)) => {
                validate_hdf5_attribute_name(name).map_err(|error| {
                    NisarSubsetPlanV2Error::DependencyGraphInvalid(error.to_string())
                })?;
            }
            (NisarSubsetSourceDependencyNodeKindV2::Attribute, None) => {
                return Err(NisarSubsetPlanV2Error::DependencyGraphInvalid(
                    "attribute dependency node is missing its exact attribute name".to_owned(),
                ));
            }
            (_, Some(_)) => {
                return Err(NisarSubsetPlanV2Error::DependencyGraphInvalid(
                    "non-attribute dependency node carries an attribute name".to_owned(),
                ));
            }
            (_, None) => {}
        }
    }
    graph.nodes.sort_by(|left, right| {
        left.source_path
            .cmp(&right.source_path)
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| left.attribute_name.cmp(&right.attribute_name))
    });
    if graph
        .nodes
        .windows(2)
        .any(|pair| pair[0].source_path == pair[1].source_path)
    {
        return Err(NisarSubsetPlanV2Error::DependencyGraphInvalid(
            "duplicate source dependency node path".to_owned(),
        ));
    }
    if graph.edges.is_empty() || graph.edges.len() > MAX_DEPENDENCIES.saturating_mul(8) {
        return Err(NisarSubsetPlanV2Error::DependencyGraphInvalid(
            "source dependency edge count is outside the closed bound".to_owned(),
        ));
    }
    for edge in &graph.edges {
        validate_hdf5_path(&edge.dependent_path)
            .map_err(|error| NisarSubsetPlanV2Error::DependencyGraphInvalid(error.to_string()))?;
        validate_hdf5_path(&edge.dependency_path)
            .map_err(|error| NisarSubsetPlanV2Error::DependencyGraphInvalid(error.to_string()))?;
        if edge.dependent_path == edge.dependency_path {
            return Err(NisarSubsetPlanV2Error::DependencyGraphInvalid(
                "self-referential dependency edge".to_owned(),
            ));
        }
    }
    graph.edges.sort_by(|left, right| {
        left.dependent_path
            .cmp(&right.dependent_path)
            .then_with(|| left.dependency_path.cmp(&right.dependency_path))
            .then_with(|| left.relation.cmp(&right.relation))
    });
    if graph.edges.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(NisarSubsetPlanV2Error::DependencyGraphInvalid(
            "duplicate source dependency edge".to_owned(),
        ));
    }

    let node_kinds = graph
        .nodes
        .iter()
        .map(|node| (node.source_path.as_str(), node.kind))
        .collect::<BTreeMap<_, _>>();
    if !node_kinds
        .values()
        .any(|kind| *kind == NisarSubsetSourceDependencyNodeKindV2::ScienceDataset)
    {
        return Err(NisarSubsetPlanV2Error::DependencyGraphInvalid(
            "source graph contains no science dataset".to_owned(),
        ));
    }
    for edge in &graph.edges {
        if !node_kinds.contains_key(edge.dependent_path.as_str())
            || !node_kinds.contains_key(edge.dependency_path.as_str())
        {
            return Err(NisarSubsetPlanV2Error::DependencyGraphInvalid(
                "dependency edge references a missing node".to_owned(),
            ));
        }
        let target_kind = node_kinds[edge.dependency_path.as_str()];
        if !dependency_relation_matches_kind(edge.relation, target_kind) {
            return Err(NisarSubsetPlanV2Error::DependencyGraphInvalid(format!(
                "dependency relation does not match target node kind: {}",
                edge.dependency_path
            )));
        }
    }

    let mut reachable = BTreeSet::new();
    let mut queue = VecDeque::new();
    for node in &graph.nodes {
        if node.kind == NisarSubsetSourceDependencyNodeKindV2::ScienceDataset {
            reachable.insert(node.source_path.clone());
            queue.push_back(node.source_path.clone());
        }
    }
    while let Some(path) = queue.pop_front() {
        for edge in graph
            .edges
            .iter()
            .filter(|edge| edge.dependent_path == path)
        {
            if reachable.insert(edge.dependency_path.clone()) {
                queue.push_back(edge.dependency_path.clone());
            }
        }
    }
    if let Some(orphan) = graph
        .nodes
        .iter()
        .find(|node| !reachable.contains(&node.source_path))
    {
        return Err(NisarSubsetPlanV2Error::DependencyGraphInvalid(format!(
            "orphan required dependency node: {}",
            orphan.source_path
        )));
    }
    Ok(graph)
}

fn dependency_relation_matches_kind(
    relation: NisarSubsetDependencyRelationV2,
    kind: NisarSubsetSourceDependencyNodeKindV2,
) -> bool {
    matches!(
        (relation, kind),
        (
            NisarSubsetDependencyRelationV2::Coordinate,
            NisarSubsetSourceDependencyNodeKindV2::CoordinateDataset
        ) | (
            NisarSubsetDependencyRelationV2::DimensionScale,
            NisarSubsetSourceDependencyNodeKindV2::DimensionScale
        ) | (
            NisarSubsetDependencyRelationV2::ValidityMask,
            NisarSubsetSourceDependencyNodeKindV2::ValidityMask
        ) | (
            NisarSubsetDependencyRelationV2::RequiredMetadata,
            NisarSubsetSourceDependencyNodeKindV2::IdentificationMetadata
        ) | (
            NisarSubsetDependencyRelationV2::Provenance,
            NisarSubsetSourceDependencyNodeKindV2::ProvenanceMetadata
        ) | (
            NisarSubsetDependencyRelationV2::RequiredAttribute,
            NisarSubsetSourceDependencyNodeKindV2::Attribute
        ) | (
            NisarSubsetDependencyRelationV2::OtherScientificDependency,
            NisarSubsetSourceDependencyNodeKindV2::OtherScientificDependency
        )
    )
}

fn derive_dependency_closure_from_canonical_graph(
    graph: &NisarSubsetSourceDependencyGraphV2,
    swaths_root: &str,
    frequency: &NisarSubsetFrequency,
    polarizations: &[String],
) -> Result<NisarSubsetDependencyClosureV2, NisarSubsetPlanV2Error> {
    let polarizations = normalize_polarizations(polarizations.to_vec())?;
    let nodes = graph
        .nodes
        .iter()
        .map(|node| (node.source_path.as_str(), node.kind))
        .collect::<BTreeMap<_, _>>();
    let mut reached = BTreeSet::new();
    let mut required_by = BTreeMap::<String, BTreeSet<String>>::new();
    let mut queue = VecDeque::new();
    for polarization in &polarizations {
        let path = science_dataset_path(swaths_root, frequency, polarization);
        if nodes.get(path.as_str()) != Some(&NisarSubsetSourceDependencyNodeKindV2::ScienceDataset)
        {
            return Err(NisarSubsetPlanV2Error::DependencyClosureInvalid(format!(
                "selected science dataset is absent from authoritative evidence: {path}"
            )));
        }
        reached.insert(path.clone());
        required_by.entry(path.clone()).or_default().insert(format!(
            "selection:frequency{}:{polarization}",
            frequency_name(frequency)
        ));
        queue.push_back(path);
    }
    while let Some(path) = queue.pop_front() {
        for edge in graph
            .edges
            .iter()
            .filter(|edge| edge.dependent_path == path)
        {
            required_by
                .entry(edge.dependency_path.clone())
                .or_default()
                .insert(edge.dependent_path.clone());
            if reached.insert(edge.dependency_path.clone()) {
                queue.push_back(edge.dependency_path.clone());
            }
        }
    }

    let items = reached
        .into_iter()
        .map(|source_path| {
            let source_kind = nodes[source_path.as_str()];
            let kind = dependency_closure_kind(source_kind)?;
            let required_by = required_by
                .remove(&source_path)
                .unwrap_or_default()
                .into_iter()
                .collect();
            Ok(NisarSubsetDependencyV2 {
                kind,
                source_path,
                required_by,
            })
        })
        .collect::<Result<Vec<_>, NisarSubsetPlanV2Error>>()?;
    normalize_dependency_closure(
        NisarSubsetDependencyClosureV2 {
            closure_algorithm: NISAR_SUBSET_PLAN_V2_DEPENDENCY_ALGORITHM.to_owned(),
            closure_algorithm_version: NISAR_SUBSET_PLAN_V2_DEPENDENCY_ALGORITHM_VERSION,
            source_graph_sha256: digest_serializable(graph)?,
            complete: true,
            unknown_dependency_count: 0,
            items,
        },
        swaths_root,
        frequency,
        &polarizations,
    )
}

fn dependency_closure_kind(
    kind: NisarSubsetSourceDependencyNodeKindV2,
) -> Result<NisarSubsetDependencyKindV2, NisarSubsetPlanV2Error> {
    match kind {
        NisarSubsetSourceDependencyNodeKindV2::ScienceDataset => {
            Ok(NisarSubsetDependencyKindV2::SelectedScienceDataset)
        }
        NisarSubsetSourceDependencyNodeKindV2::CoordinateDataset => {
            Ok(NisarSubsetDependencyKindV2::CoordinateDataset)
        }
        NisarSubsetSourceDependencyNodeKindV2::DimensionScale => {
            Ok(NisarSubsetDependencyKindV2::DimensionScale)
        }
        NisarSubsetSourceDependencyNodeKindV2::ValidityMask => {
            Ok(NisarSubsetDependencyKindV2::ValidityMask)
        }
        NisarSubsetSourceDependencyNodeKindV2::IdentificationMetadata => {
            Ok(NisarSubsetDependencyKindV2::IdentificationMetadata)
        }
        NisarSubsetSourceDependencyNodeKindV2::ProvenanceMetadata => {
            Ok(NisarSubsetDependencyKindV2::ProvenanceMetadata)
        }
        NisarSubsetSourceDependencyNodeKindV2::Attribute => {
            Ok(NisarSubsetDependencyKindV2::Attribute)
        }
        NisarSubsetSourceDependencyNodeKindV2::OtherScientificDependency => {
            Ok(NisarSubsetDependencyKindV2::OtherScientificDependency)
        }
        NisarSubsetSourceDependencyNodeKindV2::Unknown => {
            Err(NisarSubsetPlanV2Error::DependencyGraphInvalid(
                "unknown dependency reached closure derivation".to_owned(),
            ))
        }
    }
}

fn normalize_dependency_closure(
    mut closure: NisarSubsetDependencyClosureV2,
    swaths_root: &str,
    frequency: &NisarSubsetFrequency,
    polarizations: &[String],
) -> Result<NisarSubsetDependencyClosureV2, NisarSubsetPlanV2Error> {
    if closure.closure_algorithm != NISAR_SUBSET_PLAN_V2_DEPENDENCY_ALGORITHM
        || closure.closure_algorithm_version != NISAR_SUBSET_PLAN_V2_DEPENDENCY_ALGORITHM_VERSION
        || !closure.complete
        || closure.unknown_dependency_count != 0
    {
        return Err(NisarSubsetPlanV2Error::DependencyClosureInvalid(
            "closure algorithm is unsupported, incomplete, or contains unknown dependencies"
                .to_owned(),
        ));
    }
    validate_sha256(
        &closure.source_graph_sha256,
        "required_dependencies.source_graph_sha256",
    )?;
    if closure.items.is_empty() || closure.items.len() > MAX_DEPENDENCIES {
        return Err(NisarSubsetPlanV2Error::DependencyClosureInvalid(
            "dependency item count is outside the closed bound".to_owned(),
        ));
    }
    for item in &mut closure.items {
        validate_hdf5_path(&item.source_path)?;
        item.required_by.sort();
        item.required_by.dedup();
        if item.required_by.is_empty() {
            return Err(NisarSubsetPlanV2Error::DependencyClosureInvalid(
                "dependency provenance is empty".to_owned(),
            ));
        }
        for required_by in &item.required_by {
            validate_text("dependency.required_by", required_by)?;
        }
    }
    closure.items.sort_by(|left, right| {
        left.kind
            .cmp(&right.kind)
            .then_with(|| left.source_path.cmp(&right.source_path))
            .then_with(|| left.required_by.cmp(&right.required_by))
    });
    if closure
        .items
        .windows(2)
        .any(|pair| pair[0].kind == pair[1].kind && pair[0].source_path == pair[1].source_path)
    {
        return Err(NisarSubsetPlanV2Error::DependencyClosureInvalid(
            "duplicate dependency identity".to_owned(),
        ));
    }

    let expected_science = polarizations
        .iter()
        .map(|polarization| science_dataset_path(swaths_root, frequency, polarization))
        .collect::<BTreeSet<_>>();
    let actual_science = closure
        .items
        .iter()
        .filter(|item| item.kind == NisarSubsetDependencyKindV2::SelectedScienceDataset)
        .map(|item| item.source_path.clone())
        .collect::<BTreeSet<_>>();
    if actual_science != expected_science {
        return Err(NisarSubsetPlanV2Error::DependencyClosureInvalid(
            "selected science dataset closure does not match the polarization set".to_owned(),
        ));
    }
    let expected_mask = validity_mask_dataset_path(swaths_root, frequency);
    let actual_masks = closure
        .items
        .iter()
        .filter(|item| item.kind == NisarSubsetDependencyKindV2::ValidityMask)
        .map(|item| item.source_path.as_str())
        .collect::<Vec<_>>();
    if actual_masks != [expected_mask.as_str()] {
        return Err(NisarSubsetPlanV2Error::DependencyClosureInvalid(
            "validity-mask closure does not match the selected frequency".to_owned(),
        ));
    }
    let mut required_kinds = vec![
        NisarSubsetDependencyKindV2::CoordinateDataset,
        NisarSubsetDependencyKindV2::IdentificationMetadata,
        NisarSubsetDependencyKindV2::ProvenanceMetadata,
        NisarSubsetDependencyKindV2::Attribute,
        NisarSubsetDependencyKindV2::OtherScientificDependency,
    ];
    if swaths_root == NISAR_LSAR_SWATHS_ROOT {
        required_kinds.push(NisarSubsetDependencyKindV2::DimensionScale);
    }
    for required_kind in required_kinds {
        if !closure.items.iter().any(|item| item.kind == required_kind) {
            return Err(NisarSubsetPlanV2Error::DependencyClosureInvalid(format!(
                "required dependency category {required_kind:?} is absent"
            )));
        }
    }
    Ok(closure)
}

fn science_dataset_path(
    swaths_root: &str,
    frequency: &NisarSubsetFrequency,
    polarization: &str,
) -> String {
    format!(
        "{swaths_root}/frequency{}/{polarization}",
        frequency_name(frequency)
    )
}

fn validity_mask_dataset_path(swaths_root: &str, frequency: &NisarSubsetFrequency) -> String {
    format!(
        "{swaths_root}/frequency{}/inputDataExceptionMask",
        frequency_name(frequency)
    )
}

fn dependency_graph_swaths_root(
    graph: &NisarSubsetSourceDependencyGraphV2,
) -> Result<&'static str, NisarSubsetPlanV2Error> {
    let mut root = None;
    for node in graph
        .nodes
        .iter()
        .filter(|node| node.kind == NisarSubsetSourceDependencyNodeKindV2::ScienceDataset)
    {
        let candidate = if node
            .source_path
            .strip_prefix(NISAR_LSAR_SWATHS_ROOT)
            .is_some_and(|suffix| suffix.starts_with('/'))
        {
            NISAR_LSAR_SWATHS_ROOT
        } else if node
            .source_path
            .strip_prefix(NISAR_SSAR_SWATHS_ROOT)
            .is_some_and(|suffix| suffix.starts_with('/'))
        {
            NISAR_SSAR_SWATHS_ROOT
        } else {
            return Err(NisarSubsetPlanV2Error::DependencyGraphInvalid(
                "science dataset is outside a reviewed NISAR swaths root".to_owned(),
            ));
        };
        match root {
            None => root = Some(candidate),
            Some(existing) if existing == candidate => {}
            Some(_) => {
                return Err(NisarSubsetPlanV2Error::DependencyGraphInvalid(
                    "science datasets span multiple NISAR swaths roots".to_owned(),
                ));
            }
        }
    }
    root.ok_or_else(|| {
        NisarSubsetPlanV2Error::DependencyGraphInvalid(
            "source dependency graph contains no science dataset".to_owned(),
        )
    })
}

fn source_swaths_root(
    identity: &NisarSubsetSourceProductIdentityV2,
) -> Result<&'static str, NisarSubsetPlanV2Error> {
    match (
        identity.provider_id.as_str(),
        identity.collection_id.as_str(),
        identity.collection_profile_id.as_str(),
    ) {
        ("asf.nasa.earthdata", "C2853145197-ASF", "asf.nisar.l.rslc.provisional.v1") => {
            Ok(NISAR_LSAR_SWATHS_ROOT)
        }
        ("isro.nrsc.bhoonidhi", "NISAR_SSAR_RSLC", "isro.nrsc.bhoonidhi.nisar.s.ssar.rslc.v1") => {
            Ok(NISAR_SSAR_SWATHS_ROOT)
        }
        _ => Err(NisarSubsetPlanV2Error::SourceIdentityInvalid(
            "source identity has no reviewed NISAR science root".to_owned(),
        )),
    }
}

fn frequency_name(frequency: &NisarSubsetFrequency) -> &'static str {
    match frequency {
        NisarSubsetFrequency::A => "A",
        NisarSubsetFrequency::B => "B",
    }
}

fn validate_source_product_identity(
    identity: &NisarSubsetSourceProductIdentityV2,
) -> Result<(), NisarSubsetPlanV2Error> {
    for (label, value) in [
        ("provider_id", identity.provider_id.as_str()),
        ("collection_id", identity.collection_id.as_str()),
        (
            "collection_profile_id",
            identity.collection_profile_id.as_str(),
        ),
        ("provider_product_id", identity.provider_product_id.as_str()),
        (
            "provider_product_key",
            identity.provider_product_key.as_str(),
        ),
        (
            "product_specification_version",
            identity.product_specification_version.as_str(),
        ),
        ("crid", identity.crid.as_str()),
    ] {
        validate_text(label, value)
            .map_err(|error| NisarSubsetPlanV2Error::SourceIdentityInvalid(error.to_string()))?;
    }
    if identity.quality_status != NisarSubsetQualityStatusV2::Provisional {
        return Err(NisarSubsetPlanV2Error::SourceIdentityInvalid(
            "Quality Status must be PROVISIONAL".to_owned(),
        ));
    }
    Ok(())
}

fn validate_source_asset_identity(
    identity: &NisarSubsetSourceAssetIdentityV2,
) -> Result<(), NisarSubsetPlanV2Error> {
    for (label, value) in [
        ("asset_key", identity.asset_key.as_str()),
        ("stable_asset_key", identity.stable_asset_key.as_str()),
        ("file_name", identity.file_name.as_str()),
    ] {
        validate_text(label, value)
            .map_err(|error| NisarSubsetPlanV2Error::SourceIdentityInvalid(error.to_string()))?;
    }
    if !identity.file_name.to_ascii_lowercase().ends_with(".h5")
        || identity.file_name.contains('/')
        || identity.file_name.contains('\\')
    {
        return Err(NisarSubsetPlanV2Error::SourceIdentityInvalid(
            "source asset must be one HDF5 file name".to_owned(),
        ));
    }
    if let Some(checksum) = &identity.checksum {
        let length = match checksum.algorithm {
            NisarSubsetChecksumAlgorithm::Md5 => 32,
            NisarSubsetChecksumAlgorithm::Sha1 => 40,
            NisarSubsetChecksumAlgorithm::Sha256 => 64,
        };
        if checksum.value.len() != length
            || !checksum
                .value
                .bytes()
                .all(|value| value.is_ascii_hexdigit() && !value.is_ascii_uppercase())
        {
            return Err(NisarSubsetPlanV2Error::SourceIdentityInvalid(
                "source checksum is not canonical lower-case hexadecimal".to_owned(),
            ));
        }
    }
    Ok(())
}

fn validate_estimate(
    label: &str,
    estimate: Option<&NisarSubsetByteEstimateV2>,
) -> Result<(), NisarSubsetPlanV2Error> {
    let Some(estimate) = estimate else {
        return Ok(());
    };
    if estimate.bytes == 0 || estimate.bytes > MAX_SAFE_INTEGER {
        return Err(NisarSubsetPlanV2Error::EstimateInvalid(format!(
            "{label} byte count is invalid"
        )));
    }
    validate_sha256(&estimate.evidence_sha256, label)
}

fn normalize_warnings(mut warnings: Vec<String>) -> Result<Vec<String>, NisarSubsetPlanV2Error> {
    if warnings.len() > MAX_WARNINGS {
        return Err(NisarSubsetPlanV2Error::ContractInvalid(
            "warning count exceeds bound".to_owned(),
        ));
    }
    for warning in &warnings {
        validate_text("warning", warning)?;
    }
    warnings.sort();
    warnings.dedup();
    Ok(warnings)
}

fn canonical_pre_metadata_requirements() -> Vec<NisarSubsetScientificAdmissionRequirementV2> {
    use NisarSubsetScientificAdmissionRequirementV2 as Requirement;
    vec![
        Requirement::ExactSourceIdentity,
        Requirement::CanonicalRequestedGeometry,
        Requirement::NonEmptyEffectiveIntersection,
        Requirement::RemoteRangeOnly,
        Requirement::ClientDerivedOutput,
        Requirement::ProvisionalQualityDisclosure,
    ]
}

fn canonical_admission_requirements() -> Vec<NisarSubsetScientificAdmissionRequirementV2> {
    use NisarSubsetScientificAdmissionRequirementV2 as Requirement;
    vec![
        Requirement::ExactSourceIdentity,
        Requirement::CanonicalRequestedGeometry,
        Requirement::NonEmptyEffectiveIntersection,
        Requirement::CompleteDependencyClosure,
        Requirement::RemoteRangeOnly,
        Requirement::ClientDerivedOutput,
        Requirement::PostWriteScientificValidation,
        Requirement::ProvisionalQualityDisclosure,
    ]
}

fn empty_prepared_intent_provenance() -> NisarSubsetPreparedIntentProvenanceV2 {
    NisarSubsetPreparedIntentProvenanceV2 {
        product_semantics: String::new(),
        derived_identity_authority: NisarSubsetDerivedIdentityAuthorityV2::ClientDerived,
        source_product_identity_sha256: String::new(),
        source_asset_identity_sha256: String::new(),
        source_footprint_sha256: String::new(),
        requested_geometry_sha256: String::new(),
        effective_geometry_sha256: String::new(),
        intent_identity_sha256: String::new(),
    }
}

fn empty_provenance() -> NisarSubsetPlanProvenanceV2 {
    NisarSubsetPlanProvenanceV2 {
        product_semantics: String::new(),
        derived_identity_authority: NisarSubsetDerivedIdentityAuthorityV2::ClientDerived,
        derived_artifact_id: String::new(),
        source_product_identity_sha256: String::new(),
        source_asset_identity_sha256: String::new(),
        source_footprint_sha256: String::new(),
        requested_geometry_sha256: String::new(),
        effective_geometry_sha256: String::new(),
        plan_identity_sha256: String::new(),
    }
}

fn geometry_digest(geometry: &NisarSubsetGeometryV2) -> Result<String, NisarSubsetPlanV2Error> {
    digest_serializable(&json!({
        "crs": geometry.crs,
        "canonical_geojson": geometry.canonical_geojson,
    }))
}

fn digest_serializable(value: &impl Serialize) -> Result<String, NisarSubsetPlanV2Error> {
    let bytes = canonical_json_bytes(value, NISAR_SUBSET_PLAN_V2_MAX_CANONICAL_JSON_BYTES)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn canonical_json_bytes(
    value: &impl Serialize,
    maximum_bytes: usize,
) -> Result<Vec<u8>, NisarSubsetPlanV2Error> {
    let value = serde_json::to_value(value)
        .map_err(|error| NisarSubsetPlanV2Error::ContractInvalid(error.to_string()))?;
    let value = canonicalize_json(value);
    let bytes = serde_json::to_vec(&value)
        .map_err(|error| NisarSubsetPlanV2Error::ContractInvalid(error.to_string()))?;
    if bytes.len() > maximum_bytes {
        return Err(NisarSubsetPlanV2Error::PlanTooLarge);
    }
    Ok(bytes)
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

fn validate_hdf5_attribute_name(value: &str) -> Result<(), NisarSubsetPlanV2Error> {
    validate_text("dependency.attribute_name", value)?;
    if value == "." || value == ".." || value.contains('/') || value.contains('\\') {
        return Err(NisarSubsetPlanV2Error::DependencyClosureInvalid(
            "dependency attribute_name is not one exact HDF5 attribute name".to_owned(),
        ));
    }
    Ok(())
}

fn validate_hdf5_path(value: &str) -> Result<(), NisarSubsetPlanV2Error> {
    validate_text("dependency.source_path", value)?;
    // The canonical HDF5 root is a real group and can own required attributes
    // such as the RSLC `Conventions` evidence.
    if value == "/" {
        return Ok(());
    }
    let canonical = value.starts_with('/')
        && !value.ends_with('/')
        && !value.contains('\\')
        && value
            .split('/')
            .skip(1)
            .all(|component| !component.is_empty() && component != "." && component != "..");
    if !canonical {
        return Err(NisarSubsetPlanV2Error::DependencyClosureInvalid(
            "dependency source_path is not one canonical absolute HDF5 path".to_owned(),
        ));
    }
    Ok(())
}

fn validate_text(label: &str, value: &str) -> Result<(), NisarSubsetPlanV2Error> {
    if value.is_empty() || value.len() > MAX_TEXT_BYTES || value.chars().any(char::is_control) {
        return Err(NisarSubsetPlanV2Error::ContractInvalid(format!(
            "{label} is empty, oversized, or contains control characters"
        )));
    }
    let lower = value.to_ascii_lowercase();
    if [
        "authorization",
        "cookie",
        "password",
        "secret",
        "token",
        ".netrc",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
    {
        return Err(NisarSubsetPlanV2Error::ContractInvalid(format!(
            "{label} contains a sensitive marker"
        )));
    }
    Ok(())
}

fn validate_sha256(value: &str, label: &str) -> Result<(), NisarSubsetPlanV2Error> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|value| value.is_ascii_hexdigit() && !value.is_ascii_uppercase())
    {
        return Err(NisarSubsetPlanV2Error::ContractInvalid(format!(
            "{label} must be a lower-case SHA-256 digest"
        )));
    }
    Ok(())
}

fn geometry_error(
    geometry: NisarSubsetGeometryRoleV2,
    reason: NisarSubsetGeometryInvalidReasonV2,
) -> NisarSubsetPlanV2Error {
    NisarSubsetPlanV2Error::GeometryInvalid { geometry, reason }
}
