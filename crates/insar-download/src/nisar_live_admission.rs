//! Explicit, scene-generic admission checks for the live NISAR Range route.
//!
//! This module is production-feature-gated and does not perform network
//! I/O.  It consumes already verified source, transport, request, metadata,
//! physical-plan, and budget facts.  Semantic/execution hashes remain useful
//! provenance, but no single hash is an admission authority here.

#![cfg(feature = "nisar-subset-production")]

use insar_core::{
    validate_request_against_snapshot_item, NisarSubsetFrequency, NisarSubsetSourceValidatorV1,
    NisarSubsetValidatorKind, SnapshotItem, NISAR_SUBSET_MAPPING_ALGORITHM,
    NISAR_SUBSET_MAPPING_ALGORITHM_VERSION, NISAR_SUBSET_METHOD,
};
use insar_provider::asf::nisar_subset::supported_dataset_polarizations;
use insar_provider::catalog::{
    NISAR_BAND, NISAR_NATIVE_COLLECTION_ID, NISAR_P05023_CRID,
    NISAR_P05023_PRODUCT_SPECIFICATION_VERSION, NISAR_PRODUCTION_CONFIGURATION, NISAR_PROFILE_ID,
    NISAR_PROVIDER_ID, NISAR_REQUIRED_PRODUCT_TYPE, NISAR_SCIENCE_LEVEL,
};
use serde_json::Value;
use thiserror::Error;

use crate::hdf5_range_transfer::{
    Hdf5RangePlanV1, NisarLiveNetworkBudgetV1, NisarLiveTransportAdmissionV1,
};
use crate::nisar_subset_writer::{
    NisarSubsetMetadataSnapshotV1, MAX_SUPPORTED_SOURCE_DATASET_COUNT,
};

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub(crate) enum NisarLiveAdmissionError {
    #[error("NISAR live source identity admission failed")]
    SourceIdentity,
    #[error("NISAR live transport admission failed")]
    Transport,
    #[error("NISAR live request admission failed")]
    Request,
    #[error("NISAR live mapping admission failed")]
    Mapping,
    #[error("NISAR live metadata admission failed")]
    Metadata,
    #[error("NISAR live physical-plan admission failed")]
    PhysicalPlan,
    #[error("NISAR live network budget admission failed")]
    Budget,
}

impl NisarLiveAdmissionError {
    pub(crate) const fn code(self) -> &'static str {
        match self {
            Self::SourceIdentity => "NISAR_SUBSET_ADMISSION_SOURCE_INVALID",
            Self::Transport => "NISAR_SUBSET_ADMISSION_TRANSPORT_INVALID",
            Self::Request => "NISAR_SUBSET_ADMISSION_REQUEST_INVALID",
            Self::Mapping => "NISAR_SUBSET_ADMISSION_MAPPING_INVALID",
            Self::Metadata => "NISAR_SUBSET_ADMISSION_METADATA_INVALID",
            Self::PhysicalPlan => "NISAR_SUBSET_ADMISSION_PHYSICAL_PLAN_INVALID",
            Self::Budget => "NISAR_SUBSET_ADMISSION_BUDGET_INVALID",
        }
    }
}

/// Validate every explicit live admission section before any science Range is
/// issued.  The function intentionally accepts the frozen catalogue item and
/// the source validator separately so a catalogue/transport drift cannot be
/// hidden inside a semantic digest.
pub(crate) fn validate_nisar_live_admission(
    item: &SnapshotItem,
    request: &insar_core::NisarSubsetRequestManifestV1,
    metadata: &NisarSubsetMetadataSnapshotV1,
    plan: &Hdf5RangePlanV1,
    source_validator: &NisarSubsetSourceValidatorV1,
    transport: &NisarLiveTransportAdmissionV1,
    budget: &NisarLiveNetworkBudgetV1,
) -> Result<(), NisarLiveAdmissionError> {
    validate_source_identity(item, request).map_err(|_| NisarLiveAdmissionError::SourceIdentity)?;
    validate_transport(transport).map_err(|_| NisarLiveAdmissionError::Transport)?;
    validate_request(item, request).map_err(|_| NisarLiveAdmissionError::Request)?;
    validate_mapping(request).map_err(|_| NisarLiveAdmissionError::Mapping)?;
    validate_metadata(request, metadata, source_validator)
        .map_err(|_| NisarLiveAdmissionError::Metadata)?;
    plan.validate_nisar_live_physical_admission(request)
        .map_err(|_| NisarLiveAdmissionError::PhysicalPlan)?;
    budget
        .validate_planned_science(plan)
        .map_err(|_| NisarLiveAdmissionError::Budget)?;
    Ok(())
}

fn validate_source_identity(
    item: &SnapshotItem,
    request: &insar_core::NisarSubsetRequestManifestV1,
) -> Result<(), ()> {
    validate_request_against_snapshot_item(request, item).map_err(|_| ())?;
    let source = &request.source;
    if source.provider_id != NISAR_PROVIDER_ID
        || source.collection_id != NISAR_NATIVE_COLLECTION_ID
        || source.collection_profile_id != NISAR_PROFILE_ID
        || source.product_specification_version != NISAR_P05023_PRODUCT_SPECIFICATION_VERSION
        || source.crid != NISAR_P05023_CRID
        || source.file_name.is_empty()
        || !source.file_name.to_ascii_lowercase().ends_with(".h5")
        || source.expected_size_bytes == 0
        || source.checksum.is_none()
    {
        return Err(());
    }
    for (key, expected) in [
        ("band", NISAR_BAND),
        ("processing_level", NISAR_REQUIRED_PRODUCT_TYPE),
        ("science_level", NISAR_SCIENCE_LEVEL),
        ("production_configuration", NISAR_PRODUCTION_CONFIGURATION),
        ("crid", NISAR_P05023_CRID),
        (
            "product_specification_version",
            NISAR_P05023_PRODUCT_SPECIFICATION_VERSION,
        ),
        ("collection_profile_id", NISAR_PROFILE_ID),
    ] {
        if item.metadata.get(key).and_then(Value::as_str) != Some(expected) {
            return Err(());
        }
    }
    let asset = item.asset_access.as_ref().ok_or(())?;
    if asset.expected_size_bytes != Some(source.expected_size_bytes)
        || asset.file_name.as_deref() != Some(source.file_name.as_str())
    {
        return Err(());
    }
    Ok(())
}

fn validate_transport(transport: &NisarLiveTransportAdmissionV1) -> Result<(), ()> {
    if transport.final_host_class != "earthdata_asf_v1"
        || transport.final_path_class != "approved_nisar_hdf5_object"
        || !transport.range_status_206
        || !transport.content_range_exact
        || !transport.response_length_exact
        || !transport.full_file_fallback_rejected
        || !transport.unauthorized_redirect_rejected
    {
        return Err(());
    }
    Ok(())
}

fn validate_request(
    item: &SnapshotItem,
    request: &insar_core::NisarSubsetRequestManifestV1,
) -> Result<(), ()> {
    request.validate().map_err(|_| ())?;
    if request.request.subset_method != NISAR_SUBSET_METHOD
        || request.request.swath != "LSAR_RSLC"
        || request.source.expected_size_bytes != request.expected_output.full_product_bytes
    {
        return Err(());
    }
    let key = match request.request.frequency {
        NisarSubsetFrequency::A => "frequency_a_polarization",
        NisarSubsetFrequency::B => "frequency_b_polarization",
    };
    let available = item.metadata.get(key).and_then(Value::as_str).ok_or(())?;
    if !supported_dataset_polarizations(&request.request.frequency)
        .contains(&request.request.polarization.as_str())
        || !available_polarization(available, &request.request.polarization)
    {
        return Err(());
    }
    Ok(())
}

fn available_polarization(available: &str, requested: &str) -> bool {
    available.split('+').any(|value| value == requested)
}

fn validate_mapping(request: &insar_core::NisarSubsetRequestManifestV1) -> Result<(), ()> {
    if request.mapping.algorithm != NISAR_SUBSET_MAPPING_ALGORITHM
        || request.mapping.algorithm_version != NISAR_SUBSET_MAPPING_ALGORITHM_VERSION
        || !is_sha256(&request.mapping.evidence_digest)
        || request.mapping.windows.is_empty()
    {
        return Err(());
    }
    for window in &request.mapping.windows {
        if window.mapping_confidence != insar_core::NisarSubsetMappingConfidence::ReferenceBounded
            || window.line_start >= window.line_end_exclusive
            || window.sample_start >= window.sample_end_exclusive
            || window.source_shape.contains(&0)
            || window.chunk_shape.contains(&0)
            || window.line_end_exclusive > window.source_shape[0]
            || window.sample_end_exclusive > window.source_shape[1]
        {
            return Err(());
        }
    }
    Ok(())
}

fn validate_metadata(
    request: &insar_core::NisarSubsetRequestManifestV1,
    metadata: &NisarSubsetMetadataSnapshotV1,
    source_validator: &NisarSubsetSourceValidatorV1,
) -> Result<(), ()> {
    let request_sha256 = request.sha256().map_err(|_| ())?;
    if metadata.request_manifest_sha256() != request_sha256
        || metadata.source_identity() != &request.source
        || metadata.source_validator() != source_validator
        || metadata.source_dataset_count() == 0
        || metadata.source_dataset_count() > MAX_SUPPORTED_SOURCE_DATASET_COUNT as u64
        || metadata.metadata_binding_count() == 0
        || !is_sha256(metadata.source_inventory_sha256())
        || !is_sha256(metadata.evidence_sha256())
    {
        return Err(());
    }
    if source_validator.kind != NisarSubsetValidatorKind::StrongEtag
        || source_validator.value.starts_with("W/")
        || !source_validator.value.starts_with('"')
        || !source_validator.value.ends_with('"')
        || source_validator.value.len() < 3
        || source_validator.content_length != request.source.expected_size_bytes
    {
        return Err(());
    }
    Ok(())
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}
