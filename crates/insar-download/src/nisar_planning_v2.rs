//! NISAR RSLC V2 metadata bootstrap and final-plan freeze authority.
//!
//! The bootstrap ledger is sealed inside this crate. Statistics are projected
//! only from successful, bounded range operations against a real HDF5 metadata
//! workspace; callers cannot manufacture no-science evidence.
#![cfg_attr(not(test), allow(dead_code))]

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::ffi::{CStr, CString};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use insar_core::{
    NisarSubsetByteEstimateV2, NisarSubsetDependencyKindV2, NisarSubsetFrequency,
    NisarSubsetPlanMetadataEvidenceV2, NisarSubsetPlanV2, NisarSubsetPreparedIntentV2,
    NisarSubsetSourceAssetIdentityV2, NisarSubsetSourceDependencyNodeKindV2,
    NisarSubsetSourceProductIdentityV2, NisarSubsetSourceValidatorV1, NisarSubsetValidatorKind,
};
use insar_store::{
    parse_nisar_subset_plan_v2_envelope, parse_nisar_subset_prepared_intent_v2_envelope,
    NisarSubsetPreparedIntentEnvelopeV2,
};
use reqwest::{header::ETAG, StatusCode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::hdf5_range_transfer::{
    inspect_nisar_planning_physical_ranges_v2, nisar_live_budget_response_limit,
    response_strong_etag, validate_content_range, NisarPlanningPhysicalRangeCatalogV2,
};
use crate::nisar_science_root::NisarScienceRoot;
use crate::{
    build_nisar_subset_source_dependency_graph_v2, DownloadWork, Hdf5RangeWorkspaceV1,
    HttpTransfer, NisarExecutionPlanFreezeAuthority, NisarExecutionPlanReadyV2,
    NisarSubsetVerifiedMetadataInventoryV2, NisarSubsetWriteError, RemoteValidator, ResolvedAsset,
    ResolvedMethod, WorkerControl, NISAR_LIVE_MAX_REQUESTS, NISAR_LIVE_MAX_RESPONSE_BODY_BYTES,
    NISAR_SUBSET_V2_DELIVERY_KIND,
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NisarSubsetBootstrapRangeV2 {
    start: u64,
    end_exclusive: u64,
}

impl NisarSubsetBootstrapRangeV2 {
    fn new(start: u64, end_exclusive: u64) -> Result<Self, NisarSubsetPlanningErrorV2> {
        if start >= end_exclusive {
            return Err(NisarSubsetPlanningErrorV2::BootstrapStatisticsInvalid);
        }
        Ok(Self {
            start,
            end_exclusive,
        })
    }

    pub fn start(&self) -> u64 {
        self.start
    }

    pub fn end_exclusive(&self) -> u64 {
        self.end_exclusive
    }

    fn len(&self) -> u64 {
        self.end_exclusive - self.start
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BootstrapCategory {
    Preflight,
    MetadataPrefix,
    MetadataDiscovery,
    AdditionalGeolocation,
    Revalidation,
}

impl BootstrapCategory {
    fn materializes(self) -> bool {
        matches!(
            self,
            Self::MetadataPrefix | Self::MetadataDiscovery | Self::AdditionalGeolocation
        )
    }

    fn evidence_stage(self) -> &'static str {
        match self {
            Self::Preflight => "preflight",
            Self::MetadataPrefix => "metadata_prefix",
            Self::MetadataDiscovery => "metadata_discovery",
            Self::AdditionalGeolocation => "additional_geolocation",
            Self::Revalidation => "planning_revalidation",
        }
    }
}

fn metadata_prefix_overlap_allowed(
    category: BootstrapCategory,
    range: &NisarSubsetBootstrapRangeV2,
) -> bool {
    if !matches!(category, BootstrapCategory::MetadataPrefix) {
        return false;
    }
    #[cfg(test)]
    {
        let _ = range;
        true
    }
    #[cfg(not(test))]
    {
        range.end_exclusive <= crate::NISAR_LIVE_METADATA_PREFIX_BYTES
    }
}

#[derive(Clone, Debug)]
struct BootstrapEvent {
    category: BootstrapCategory,
    range: NisarSubsetBootstrapRangeV2,
    request_count: u64,
    response_body_bytes: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct NisarSubsetBootstrapCategoryStatisticsV2 {
    request_count: u64,
    response_body_bytes: u64,
    requested_ranges: Vec<NisarSubsetBootstrapRangeV2>,
    #[serde(default)]
    request_counts: Vec<u64>,
}

impl NisarSubsetBootstrapCategoryStatisticsV2 {
    pub fn request_count(&self) -> u64 {
        self.request_count
    }

    pub fn response_body_bytes(&self) -> u64 {
        self.response_body_bytes
    }

    pub fn requested_ranges(&self) -> &[NisarSubsetBootstrapRangeV2] {
        &self.requested_ranges
    }

    pub fn request_counts(&self) -> &[u64] {
        &self.request_counts
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NisarSubsetBootstrapBudgetV2 {
    max_request_count: u64,
    max_response_body_bytes: u64,
}

impl NisarSubsetBootstrapBudgetV2 {
    pub fn max_request_count(&self) -> u64 {
        self.max_request_count
    }

    pub fn max_response_body_bytes(&self) -> u64 {
        self.max_response_body_bytes
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NisarSubsetBootstrapStatisticsV2 {
    frozen_plan_fast_path: bool,
    preflight: NisarSubsetBootstrapCategoryStatisticsV2,
    #[serde(default)]
    revalidation: NisarSubsetBootstrapCategoryStatisticsV2,
    metadata_prefix: NisarSubsetBootstrapCategoryStatisticsV2,
    metadata_discovery: NisarSubsetBootstrapCategoryStatisticsV2,
    additional_geolocation: NisarSubsetBootstrapCategoryStatisticsV2,
    total_planning_request_count: u64,
    total_planning_response_body_bytes: u64,
    #[serde(default)]
    charged_failure_requests: u64,
    #[serde(default)]
    charged_failure_response_bytes: u64,
    source_size: u64,
    source_validator_sha256: Option<String>,
    bootstrap_budget: Option<NisarSubsetBootstrapBudgetV2>,
    science_request_count: u64,
    science_response_body_bytes: u64,
}

impl NisarSubsetBootstrapStatisticsV2 {
    pub fn frozen_plan_fast_path_used(&self) -> bool {
        self.frozen_plan_fast_path
    }

    pub fn preflight(&self) -> &NisarSubsetBootstrapCategoryStatisticsV2 {
        &self.preflight
    }

    pub fn revalidation(&self) -> &NisarSubsetBootstrapCategoryStatisticsV2 {
        &self.revalidation
    }

    pub fn metadata_prefix(&self) -> &NisarSubsetBootstrapCategoryStatisticsV2 {
        &self.metadata_prefix
    }

    pub fn metadata_discovery(&self) -> &NisarSubsetBootstrapCategoryStatisticsV2 {
        &self.metadata_discovery
    }

    pub fn additional_geolocation(&self) -> &NisarSubsetBootstrapCategoryStatisticsV2 {
        &self.additional_geolocation
    }

    pub fn total_planning_request_count(&self) -> u64 {
        self.total_planning_request_count
    }

    pub fn total_planning_response_body_bytes(&self) -> u64 {
        self.total_planning_response_body_bytes
    }

    pub fn charged_failure_requests(&self) -> u64 {
        self.charged_failure_requests
    }

    pub fn charged_failure_response_bytes(&self) -> u64 {
        self.charged_failure_response_bytes
    }

    pub fn source_size(&self) -> u64 {
        self.source_size
    }

    pub fn source_validator_sha256(&self) -> Option<&str> {
        self.source_validator_sha256.as_deref()
    }

    pub fn bootstrap_budget(&self) -> Option<&NisarSubsetBootstrapBudgetV2> {
        self.bootstrap_budget.as_ref()
    }

    pub fn science_request_count(&self) -> u64 {
        self.science_request_count
    }

    pub fn science_response_body_bytes(&self) -> u64 {
        self.science_response_body_bytes
    }

    fn mark_frozen_plan_fast_path(mut self) -> Self {
        self.frozen_plan_fast_path = true;
        self
    }

    fn validate_checkpoint(&self) -> Result<(), NisarSubsetPlanningErrorV2> {
        let categories = [
            &self.preflight,
            &self.revalidation,
            &self.metadata_prefix,
            &self.metadata_discovery,
            &self.additional_geolocation,
        ];
        let request_count = categories.iter().try_fold(0_u64, |total, category| {
            let category_response_body_bytes =
                category
                    .requested_ranges
                    .iter()
                    .try_fold(0_u64, |bytes, range| {
                        if range.start >= range.end_exclusive
                            || range.end_exclusive > self.source_size
                        {
                            return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
                        }
                        bytes
                            .checked_add(range.len())
                            .ok_or(NisarSubsetPlanningErrorV2::RecoveryInvalid)
                    })?;
            let request_counts = if category.request_counts.is_empty() {
                vec![1_u64; category.requested_ranges.len()]
            } else {
                category.request_counts.clone()
            };
            let request_count_sum = request_counts.iter().try_fold(0_u64, |sum, count| {
                sum.checked_add(*count)
                    .filter(|count| *count > 0)
                    .ok_or(NisarSubsetPlanningErrorV2::RecoveryInvalid)
            })?;
            if category.request_count != request_count_sum
                || request_counts.len() != category.requested_ranges.len()
                || category.response_body_bytes != category_response_body_bytes
            {
                return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
            }
            total
                .checked_add(category.request_count)
                .ok_or(NisarSubsetPlanningErrorV2::RecoveryInvalid)
        })?;
        let response_bytes = categories.iter().try_fold(0_u64, |total, category| {
            total
                .checked_add(category.response_body_bytes)
                .ok_or(NisarSubsetPlanningErrorV2::RecoveryInvalid)
        })?;
        let max_response_body_bytes = nisar_live_budget_response_limit(self.source_size)
            .map_err(|_| NisarSubsetPlanningErrorV2::RecoveryInvalid)?;
        let combined_request_count = request_count
            .checked_add(self.charged_failure_requests)
            .ok_or(NisarSubsetPlanningErrorV2::RecoveryInvalid)?;
        let combined_response_bytes = response_bytes
            .checked_add(self.charged_failure_response_bytes)
            .ok_or(NisarSubsetPlanningErrorV2::RecoveryInvalid)?;
        let valid_probe = |category: &NisarSubsetBootstrapCategoryStatisticsV2| {
            category.request_count >= 1
                && category.response_body_bytes == 1
                && category.requested_ranges.len() == 1
                && (category.request_counts.is_empty()
                    || category.request_counts == [category.request_count])
                && category.requested_ranges[0].start == 0
                && category.requested_ranges[0].end_exclusive == 1
        };
        let bootstrap_budget = self
            .bootstrap_budget
            .as_ref()
            .ok_or(NisarSubsetPlanningErrorV2::RecoveryInvalid)?;
        if self.source_size == 0
            || !valid_probe(&self.preflight)
            || !valid_probe(&self.revalidation)
            || self.science_request_count != 0
            || self.science_response_body_bytes != 0
            || request_count != self.total_planning_request_count
            || response_bytes != self.total_planning_response_body_bytes
            || (self.charged_failure_requests == 0 && self.charged_failure_response_bytes != 0)
            || !valid_planning_request_limit_v2(bootstrap_budget.max_request_count)
            || combined_request_count > bootstrap_budget.max_request_count
            || combined_response_bytes > max_response_body_bytes
            || self
                .source_validator_sha256
                .as_deref()
                .is_none_or(|digest| !valid_sha256(digest))
            || bootstrap_budget.max_response_body_bytes != max_response_body_bytes
        {
            return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
        }
        Ok(())
    }

    fn validate_for_science_root(
        &self,
        science_root: NisarScienceRoot,
    ) -> Result<(), NisarSubsetPlanningErrorV2> {
        self.validate_checkpoint()?;
        if self.bootstrap_budget.as_ref().is_none_or(|budget| {
            budget.max_request_count != planning_request_limit_v2(science_root)
        }) {
            return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
        }
        Ok(())
    }
}

const NISAR_SUBSET_PHYSICAL_EXECUTION_PLAN_V2_CONTRACT: &str = "NisarSubsetPhysicalExecutionPlanV2";
const NISAR_SUBSET_PHYSICAL_EXECUTION_PLAN_V2_SCHEMA_VERSION: u32 = 2;
const NISAR_SUBSET_PLANNING_WORKSPACE_CHECKPOINT_V2_CONTRACT: &str =
    "NisarSubsetPlanningWorkspaceCheckpointV2";
const NISAR_SUBSET_PLANNING_WORKSPACE_CHECKPOINT_V2_SCHEMA_VERSION: u32 = 3;
const NISAR_SUBSET_PHYSICAL_MAX_DATASETS: usize = 32;
const NISAR_SUBSET_PHYSICAL_MAX_WINDOWS: usize = 256;
const NISAR_SUBSET_PHYSICAL_MAX_DEPENDENCY_RANGES: usize = 8_192;
const NISAR_SUBSET_PHYSICAL_MAX_EXECUTION_RANGES_V2: usize = 8_192;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub(crate) struct NisarSubsetPhysicalLogicalOwnerV2 {
    dataset_path: String,
    polarization: Option<String>,
    component_id: Option<u16>,
    window_sha256: Option<String>,
    logical_chunk: Option<[u64; 2]>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct NisarSubsetPhysicalRangeV2 {
    start: u64,
    end_exclusive: u64,
    logical_owners: Vec<NisarSubsetPhysicalLogicalOwnerV2>,
}

impl NisarSubsetPhysicalRangeV2 {
    pub(crate) fn start(&self) -> u64 {
        self.start
    }

    pub(crate) fn end_exclusive(&self) -> u64 {
        self.end_exclusive
    }

    pub(crate) fn logical_owners(&self) -> &[NisarSubsetPhysicalLogicalOwnerV2] {
        &self.logical_owners
    }

    #[cfg(test)]
    pub(crate) fn remove_logical_owner_for_test(&mut self) {
        self.logical_owners.pop();
    }

    pub(crate) fn len(&self) -> Result<u64, NisarSubsetPlanningErrorV2> {
        self.end_exclusive
            .checked_sub(self.start)
            .filter(|length| *length > 0)
            .ok_or(NisarSubsetPlanningErrorV2::PhysicalPlanInvalid)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct NisarSubsetPhysicalWindowV2 {
    component_id: u16,
    component_sha256: String,
    window_sha256: String,
    line_start: u64,
    line_end_exclusive: u64,
    sample_start: u64,
    sample_end_exclusive: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct NisarSubsetPhysicalDatasetMappingV2 {
    polarization: String,
    source_dataset_path: String,
    dataset_shape: [u64; 2],
    chunk_shape: [u64; 2],
    datatype_sha256: String,
    windows: Vec<NisarSubsetPhysicalWindowV2>,
}

impl NisarSubsetPhysicalDatasetMappingV2 {
    pub(crate) fn polarization(&self) -> &str {
        &self.polarization
    }

    pub(crate) fn source_dataset_path(&self) -> &str {
        &self.source_dataset_path
    }

    pub(crate) const fn dataset_shape(&self) -> [u64; 2] {
        self.dataset_shape
    }

    pub(crate) const fn chunk_shape(&self) -> [u64; 2] {
        self.chunk_shape
    }

    pub(crate) fn windows(&self) -> &[NisarSubsetPhysicalWindowV2] {
        &self.windows
    }
}

impl NisarSubsetPhysicalWindowV2 {
    pub(crate) const fn component_id(&self) -> u16 {
        self.component_id
    }

    pub(crate) fn component_sha256(&self) -> &str {
        &self.component_sha256
    }

    pub(crate) fn window_sha256(&self) -> &str {
        &self.window_sha256
    }

    pub(crate) const fn line_start(&self) -> u64 {
        self.line_start
    }

    pub(crate) const fn line_end_exclusive(&self) -> u64 {
        self.line_end_exclusive
    }

    pub(crate) const fn sample_start(&self) -> u64 {
        self.sample_start
    }

    pub(crate) const fn sample_end_exclusive(&self) -> u64 {
        self.sample_end_exclusive
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct NisarSubsetPhysicalNonDatasetDependencyV2 {
    source_path: String,
    attribute_names: Vec<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NisarSubsetCombinedBudgetPolicyV2 {
    PhysicalExecutionUnionV1,
    PhysicalExecutionUnionV2,
}

// Persisted V2 semantics: two aggregate fetch attempts, each with at most
// eleven initial/redirect requests and one authentication retry. Changing the
// transport ceiling needs an explicit policy review, not a digest change.
const PHYSICAL_EXECUTION_UNION_V2_REQUESTS_PER_RANGE: u64 = 44;
const _: () = assert!(crate::http_transfer::MAX_REQUESTS_PER_TRUSTED_REDIRECT_SEND <= 11);

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct NisarSubsetCombinedBudgetProjectionV2 {
    max_request_count: u64,
    max_response_body_bytes: u64,
    committed_planning_request_count: u64,
    committed_planning_response_body_bytes: u64,
    projected_science_request_count: u64,
    projected_science_response_body_bytes: u64,
    remaining_science_request_count: u64,
    remaining_science_response_body_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    budget_policy: Option<NisarSubsetCombinedBudgetPolicyV2>,
}

impl NisarSubsetCombinedBudgetProjectionV2 {
    pub(crate) fn uses_physical_execution_union_v1(&self) -> bool {
        self.budget_policy == Some(NisarSubsetCombinedBudgetPolicyV2::PhysicalExecutionUnionV1)
    }

    pub(crate) fn max_request_count(&self) -> u64 {
        self.max_request_count
    }

    pub(crate) fn max_response_body_bytes(&self) -> u64 {
        self.max_response_body_bytes
    }

    pub(crate) fn committed_planning_request_count(&self) -> u64 {
        self.committed_planning_request_count
    }

    pub(crate) fn committed_planning_response_body_bytes(&self) -> u64 {
        self.committed_planning_response_body_bytes
    }

    pub(crate) fn projected_science_request_count(&self) -> u64 {
        self.projected_science_request_count
    }

    pub(crate) fn projected_science_response_body_bytes(&self) -> u64 {
        self.projected_science_response_body_bytes
    }

    pub(crate) fn remaining_science_request_count(&self) -> u64 {
        self.remaining_science_request_count
    }

    pub(crate) fn remaining_science_response_body_bytes(&self) -> u64 {
        self.remaining_science_response_body_bytes
    }

    pub(crate) fn max_science_request_count(&self) -> u64 {
        match self.budget_policy {
            Some(
                NisarSubsetCombinedBudgetPolicyV2::PhysicalExecutionUnionV1
                | NisarSubsetCombinedBudgetPolicyV2::PhysicalExecutionUnionV2,
            ) => self.remaining_science_request_count,
            None => self
                .remaining_science_request_count
                .min(NISAR_LIVE_MAX_REQUESTS),
        }
    }

    pub(crate) fn total_request_count(&self) -> u64 {
        self.committed_planning_request_count
            .saturating_add(self.projected_science_request_count)
    }

    pub(crate) fn total_response_body_bytes(&self) -> u64 {
        self.committed_planning_response_body_bytes
            .saturating_add(self.projected_science_response_body_bytes)
    }

    fn validate(
        &self,
        source_size: u64,
        max_request_count: u64,
    ) -> Result<(), NisarSubsetPlanningErrorV2> {
        let (expected_max_request_count, max_response_body_bytes) = combined_budget_limits_v2(
            self.budget_policy.as_ref(),
            self.committed_planning_request_count,
            self.committed_planning_response_body_bytes,
            self.projected_science_request_count,
            self.projected_science_response_body_bytes,
            source_size,
            max_request_count,
        )
        .map_err(|_| NisarSubsetPlanningErrorV2::PhysicalPlanInvalid)?;
        let remaining_science_request_count = expected_max_request_count
            .checked_sub(self.committed_planning_request_count)
            .ok_or(NisarSubsetPlanningErrorV2::PhysicalPlanInvalid)?;
        let remaining_science_response_body_bytes = max_response_body_bytes
            .checked_sub(self.committed_planning_response_body_bytes)
            .ok_or(NisarSubsetPlanningErrorV2::PhysicalPlanInvalid)?;
        if self.max_request_count != expected_max_request_count
            || self.max_response_body_bytes != max_response_body_bytes
            || self.remaining_science_request_count != remaining_science_request_count
            || self.remaining_science_response_body_bytes != remaining_science_response_body_bytes
            || self.projected_science_request_count > self.max_science_request_count()
            || self.projected_science_request_count > remaining_science_request_count
            || self.projected_science_response_body_bytes > remaining_science_response_body_bytes
        {
            return Err(NisarSubsetPlanningErrorV2::PhysicalPlanInvalid);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NisarSubsetPhysicalExecutionPlanV2 {
    physical_plan_contract: String,
    schema_version: u32,
    prepared_intent_sha256: String,
    canonical_semantic_plan_digest: String,
    source_product_identity: NisarSubsetSourceProductIdentityV2,
    source_asset_identity: NisarSubsetSourceAssetIdentityV2,
    source_validator: NisarSubsetSourceValidatorV1,
    source_validator_sha256: String,
    source_size: u64,
    selected_frequency: NisarSubsetFrequency,
    selected_polarizations: Vec<String>,
    effective_geometry_sha256: String,
    inventory_sha256: String,
    inventory_evidence_sha256: String,
    source_graph_sha256: String,
    exact_selected_science_dataset_paths: Vec<String>,
    dataset_mappings: Vec<NisarSubsetPhysicalDatasetMappingV2>,
    dependency_ranges: Vec<NisarSubsetPhysicalRangeV2>,
    non_dataset_dependencies: Vec<NisarSubsetPhysicalNonDatasetDependencyV2>,
    science_ranges: Vec<NisarSubsetPhysicalRangeV2>,
    combined_budget: NisarSubsetCombinedBudgetProjectionV2,
    estimated_science_response_bytes: u64,
    estimated_derived_artifact_bytes: u64,
    physical_plan_sha256: String,
}

impl NisarSubsetPhysicalExecutionPlanV2 {
    pub(crate) fn prepared_intent_sha256(&self) -> &str {
        &self.prepared_intent_sha256
    }

    pub(crate) fn final_plan_sha256(&self) -> &str {
        &self.canonical_semantic_plan_digest
    }

    pub(crate) fn source_product_identity(&self) -> &NisarSubsetSourceProductIdentityV2 {
        &self.source_product_identity
    }

    pub(crate) fn source_asset_identity(&self) -> &NisarSubsetSourceAssetIdentityV2 {
        &self.source_asset_identity
    }

    pub(crate) fn source_validator(&self) -> &NisarSubsetSourceValidatorV1 {
        &self.source_validator
    }

    pub(crate) fn source_validator_sha256(&self) -> &str {
        &self.source_validator_sha256
    }

    pub(crate) fn source_size(&self) -> u64 {
        self.source_size
    }

    pub(crate) fn inventory_sha256(&self) -> &str {
        &self.inventory_sha256
    }

    pub(crate) fn inventory_evidence_sha256(&self) -> &str {
        &self.inventory_evidence_sha256
    }

    pub(crate) fn source_graph_sha256(&self) -> &str {
        &self.source_graph_sha256
    }

    pub(crate) fn budget_projection(&self) -> &NisarSubsetCombinedBudgetProjectionV2 {
        &self.combined_budget
    }

    pub(crate) fn physical_plan_sha256(&self) -> &str {
        &self.physical_plan_sha256
    }

    pub(crate) fn selected_frequency(&self) -> &NisarSubsetFrequency {
        &self.selected_frequency
    }

    pub(crate) fn selected_polarizations(&self) -> &[String] {
        &self.selected_polarizations
    }

    pub(crate) fn dataset_mappings(&self) -> &[NisarSubsetPhysicalDatasetMappingV2] {
        &self.dataset_mappings
    }

    pub(crate) fn dependency_ranges(&self) -> &[NisarSubsetPhysicalRangeV2] {
        &self.dependency_ranges
    }

    pub(crate) fn science_ranges(&self) -> &[NisarSubsetPhysicalRangeV2] {
        &self.science_ranges
    }

    pub(crate) fn execution_transfer_ranges(
        &self,
    ) -> Result<Vec<NisarSubsetPhysicalRangeV2>, NisarSubsetPlanningErrorV2> {
        canonical_execution_transfer_ranges_with_policy_v2(
            &self.dependency_ranges,
            &self.science_ranges,
            self.source_size,
            self.combined_budget.budget_policy.as_ref(),
        )
    }

    pub(crate) fn estimated_execution_transfer_response_bytes(
        &self,
    ) -> Result<u64, NisarSubsetPlanningErrorV2> {
        self.execution_transfer_ranges()?
            .iter()
            .try_fold(0_u64, |total, range| {
                total
                    .checked_add(range.len()?)
                    .ok_or(NisarSubsetPlanningErrorV2::CombinedBudgetExceeded)
            })
    }

    pub(crate) fn estimated_science_response_bytes(&self) -> u64 {
        self.estimated_science_response_bytes
    }

    pub(crate) fn estimated_derived_artifact_bytes(&self) -> u64 {
        self.estimated_derived_artifact_bytes
    }

    fn validate_binding(
        &self,
        frozen_plan: &NisarSubsetPlanV2,
    ) -> Result<(), NisarSubsetPlanningErrorV2> {
        frozen_plan
            .validate()
            .map_err(|_| NisarSubsetPlanningErrorV2::FrozenPlanInvalid)?;
        if self.prepared_intent_sha256 != frozen_plan.prepared_intent_sha256
            || self.canonical_semantic_plan_digest != frozen_plan.canonical_semantic_plan_digest
            || self.source_product_identity != frozen_plan.source_product_identity
            || self.source_asset_identity != frozen_plan.source_asset_identity
            || self.selected_frequency != frozen_plan.selected_frequency
            || self.selected_polarizations != frozen_plan.selected_polarizations
            || self.effective_geometry_sha256
                != frozen_plan.canonical_geometry_digest.effective_sha256
            || self.inventory_evidence_sha256 != frozen_plan.source_dependency_graph.evidence_sha256
            || self.source_graph_sha256 != frozen_plan.required_dependencies.source_graph_sha256
        {
            return Err(NisarSubsetPlanningErrorV2::PhysicalPlanInvalid);
        }
        let expected_component_ids =
            (0..frozen_plan.effective_geometry.component_count).collect::<Vec<_>>();
        for mapping in &self.dataset_mappings {
            let component_ids = mapping
                .windows
                .iter()
                .map(|window| window.component_id)
                .collect::<Vec<_>>();
            if component_ids != expected_component_ids {
                return Err(NisarSubsetPlanningErrorV2::PhysicalPlanInvalid);
            }
            for window in &mapping.windows {
                if window.component_sha256
                    != digest_serializable(&(
                        &frozen_plan.canonical_geometry_digest.effective_sha256,
                        window.component_id,
                    ))?
                {
                    return Err(NisarSubsetPlanningErrorV2::PhysicalPlanInvalid);
                }
            }
        }
        let expected_dependency_paths = frozen_plan
            .required_dependencies
            .items
            .iter()
            .filter(|dependency| {
                dependency.kind != NisarSubsetDependencyKindV2::SelectedScienceDataset
                    && dependency.kind != NisarSubsetDependencyKindV2::Attribute
            })
            .map(|dependency| dependency.source_path.clone())
            .collect::<BTreeSet<_>>();
        let actual_dependency_paths = self
            .dependency_ranges
            .iter()
            .flat_map(|range| &range.logical_owners)
            .map(|owner| owner.dataset_path.clone())
            .collect::<BTreeSet<_>>();
        if actual_dependency_paths != expected_dependency_paths
            || self.dependency_ranges.iter().any(|range| {
                range.logical_owners.iter().any(|owner| {
                    owner.polarization.is_some()
                        || owner.component_id.is_some()
                        || owner.window_sha256.is_some()
                        || owner.logical_chunk.is_some()
                })
            })
        {
            return Err(NisarSubsetPlanningErrorV2::PhysicalPlanInvalid);
        }
        let expected_attributes = frozen_plan
            .source_dependency_graph
            .nodes
            .iter()
            .filter(|node| {
                node.kind == NisarSubsetSourceDependencyNodeKindV2::Attribute
                    && node.source_path == "/"
            })
            .filter_map(|node| node.attribute_name.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        if self.non_dataset_dependencies
            != vec![NisarSubsetPhysicalNonDatasetDependencyV2 {
                source_path: "/".to_owned(),
                attribute_names: expected_attributes,
            }]
        {
            return Err(NisarSubsetPlanningErrorV2::PhysicalPlanInvalid);
        }
        self.validate()
    }

    pub(crate) fn validate(&self) -> Result<(), NisarSubsetPlanningErrorV2> {
        if self.physical_plan_contract != NISAR_SUBSET_PHYSICAL_EXECUTION_PLAN_V2_CONTRACT
            || self.schema_version != NISAR_SUBSET_PHYSICAL_EXECUTION_PLAN_V2_SCHEMA_VERSION
            || !valid_sha256(&self.prepared_intent_sha256)
            || !valid_sha256(&self.canonical_semantic_plan_digest)
            || !valid_sha256(&self.source_validator_sha256)
            || !valid_sha256(&self.effective_geometry_sha256)
            || !valid_sha256(&self.inventory_sha256)
            || !valid_sha256(&self.inventory_evidence_sha256)
            || !valid_sha256(&self.source_graph_sha256)
            || !valid_sha256(&self.physical_plan_sha256)
            || self.source_size == 0
            || self.source_validator.kind != NisarSubsetValidatorKind::StrongEtag
            || self.source_validator.value.starts_with("W/")
            || !self.source_validator.value.starts_with('"')
            || !self.source_validator.value.ends_with('"')
            || self.source_validator.value.len() < 3
            || self.source_validator.content_length != self.source_size
            || digest_serializable(&self.source_validator)? != self.source_validator_sha256
            || self.selected_polarizations.is_empty()
            || self.dataset_mappings.is_empty()
            || self.science_ranges.is_empty()
            || self.estimated_science_response_bytes == 0
            || self.dataset_mappings.len() > NISAR_SUBSET_PHYSICAL_MAX_DATASETS
            || self.non_dataset_dependencies.len() != 1
            || self.non_dataset_dependencies[0].source_path != "/"
            || self.non_dataset_dependencies[0].attribute_names.is_empty()
            || self.non_dataset_dependencies[0]
                .attribute_names
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            || self.non_dataset_dependencies[0]
                .attribute_names
                .iter()
                .any(|name| name.is_empty() || name.len() > 255)
            || self.exact_selected_science_dataset_paths.len() != self.dataset_mappings.len()
            || self.estimated_derived_artifact_bytes == 0
        {
            return Err(NisarSubsetPlanningErrorV2::PhysicalPlanInvalid);
        }
        let expected_polarizations = self
            .dataset_mappings
            .iter()
            .map(|mapping| mapping.polarization.clone())
            .collect::<Vec<_>>();
        let expected_paths = self
            .dataset_mappings
            .iter()
            .map(|mapping| mapping.source_dataset_path.clone())
            .collect::<Vec<_>>();
        if expected_polarizations != self.selected_polarizations
            || expected_paths != self.exact_selected_science_dataset_paths
            || self.dataset_mappings.iter().any(|mapping| {
                mapping.dataset_shape.contains(&0)
                    || mapping.chunk_shape.contains(&0)
                    || !valid_sha256(&mapping.datatype_sha256)
                    || mapping.windows.is_empty()
                    || mapping.windows.len() > NISAR_SUBSET_PHYSICAL_MAX_WINDOWS
                    || mapping.windows.iter().any(|window| {
                        !valid_sha256(&window.component_sha256)
                            || !valid_sha256(&window.window_sha256)
                            || window.line_start >= window.line_end_exclusive
                            || window.sample_start >= window.sample_end_exclusive
                            || window.line_end_exclusive > mapping.dataset_shape[0]
                            || window.sample_end_exclusive > mapping.dataset_shape[1]
                    })
            })
        {
            return Err(NisarSubsetPlanningErrorV2::PhysicalPlanInvalid);
        }
        validate_physical_ranges(
            &self.dependency_ranges,
            self.source_size,
            NISAR_SUBSET_PHYSICAL_MAX_DEPENDENCY_RANGES,
        )?;
        validate_physical_ranges(
            &self.science_ranges,
            self.source_size,
            execution_range_limit_v2(self.combined_budget.budget_policy.as_ref()),
        )?;
        validate_science_owner_coverage_v2(&self.dataset_mappings, &self.science_ranges)?;
        let estimated_science_response_bytes =
            self.science_ranges.iter().try_fold(0_u64, |total, range| {
                total
                    .checked_add(range.len()?)
                    .ok_or(NisarSubsetPlanningErrorV2::PhysicalPlanInvalid)
            })?;
        let execution_transfer_ranges = self
            .execution_transfer_ranges()
            .map_err(|_| NisarSubsetPlanningErrorV2::PhysicalPlanInvalid)?;
        let estimated_execution_transfer_response_bytes = execution_transfer_ranges
            .iter()
            .try_fold(0_u64, |total, range| {
                total
                    .checked_add(range.len()?)
                    .ok_or(NisarSubsetPlanningErrorV2::PhysicalPlanInvalid)
            })?;
        let science_root = science_root_for_identity(&self.source_product_identity)?;
        let max_request_count = total_request_limit_v2(science_root)?;
        self.combined_budget
            .validate(self.source_size, max_request_count)?;
        if estimated_science_response_bytes != self.estimated_science_response_bytes
            || self.combined_budget.projected_science_request_count
                != execution_transfer_ranges.len() as u64
            || self.combined_budget.projected_science_response_body_bytes
                != estimated_execution_transfer_response_bytes
            || self.combined_budget.total_request_count() > self.combined_budget.max_request_count
            || self.combined_budget.total_response_body_bytes()
                > self.combined_budget.max_response_body_bytes
            || self.recompute_physical_plan_sha256()? != self.physical_plan_sha256
        {
            return Err(NisarSubsetPlanningErrorV2::PhysicalPlanInvalid);
        }
        Ok(())
    }

    fn recompute_physical_plan_sha256(&self) -> Result<String, NisarSubsetPlanningErrorV2> {
        #[derive(Serialize)]
        struct Identity<'a> {
            physical_plan_contract: &'a str,
            schema_version: u32,
            prepared_intent_sha256: &'a str,
            canonical_semantic_plan_digest: &'a str,
            source_product_identity: &'a NisarSubsetSourceProductIdentityV2,
            source_asset_identity: &'a NisarSubsetSourceAssetIdentityV2,
            source_validator: &'a NisarSubsetSourceValidatorV1,
            source_validator_sha256: &'a str,
            source_size: u64,
            selected_frequency: &'a NisarSubsetFrequency,
            selected_polarizations: &'a [String],
            effective_geometry_sha256: &'a str,
            inventory_sha256: &'a str,
            inventory_evidence_sha256: &'a str,
            source_graph_sha256: &'a str,
            exact_selected_science_dataset_paths: &'a [String],
            dataset_mappings: &'a [NisarSubsetPhysicalDatasetMappingV2],
            dependency_ranges: &'a [NisarSubsetPhysicalRangeV2],
            non_dataset_dependencies: &'a [NisarSubsetPhysicalNonDatasetDependencyV2],
            science_ranges: &'a [NisarSubsetPhysicalRangeV2],
            combined_budget: &'a NisarSubsetCombinedBudgetProjectionV2,
            estimated_science_response_bytes: u64,
            estimated_derived_artifact_bytes: u64,
        }
        digest_serializable(&Identity {
            physical_plan_contract: &self.physical_plan_contract,
            schema_version: self.schema_version,
            prepared_intent_sha256: &self.prepared_intent_sha256,
            canonical_semantic_plan_digest: &self.canonical_semantic_plan_digest,
            source_product_identity: &self.source_product_identity,
            source_asset_identity: &self.source_asset_identity,
            source_validator: &self.source_validator,
            source_validator_sha256: &self.source_validator_sha256,
            source_size: self.source_size,
            selected_frequency: &self.selected_frequency,
            selected_polarizations: &self.selected_polarizations,
            effective_geometry_sha256: &self.effective_geometry_sha256,
            inventory_sha256: &self.inventory_sha256,
            inventory_evidence_sha256: &self.inventory_evidence_sha256,
            source_graph_sha256: &self.source_graph_sha256,
            exact_selected_science_dataset_paths: &self.exact_selected_science_dataset_paths,
            dataset_mappings: &self.dataset_mappings,
            dependency_ranges: &self.dependency_ranges,
            non_dataset_dependencies: &self.non_dataset_dependencies,
            science_ranges: &self.science_ranges,
            combined_budget: &self.combined_budget,
            estimated_science_response_bytes: self.estimated_science_response_bytes,
            estimated_derived_artifact_bytes: self.estimated_derived_artifact_bytes,
        })
    }

    #[cfg(test)]
    fn recompute_physical_plan_sha256_for_test(
        &self,
    ) -> Result<String, NisarSubsetPlanningErrorV2> {
        self.recompute_physical_plan_sha256()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct NisarSubsetPlanningWorkspaceCheckpointV2 {
    checkpoint_contract: String,
    schema_version: u32,
    workspace_canonical_path: String,
    workspace_identity: String,
    task_id: String,
    item_id: String,
    prepared_intent_sha256: String,
    final_plan_sha256: String,
    inventory_sha256: String,
    inventory_evidence_sha256: String,
    source_validator_sha256: String,
    source_size: u64,
    sparse_part_identity_sha256: String,
    sparse_part_material_ranges_sha256: String,
    sparse_part_material_sha256: String,
    bootstrap_statistics: NisarSubsetBootstrapStatisticsV2,
    physical_plan: NisarSubsetPhysicalExecutionPlanV2,
    checkpoint_sha256: String,
}

impl NisarSubsetPlanningWorkspaceCheckpointV2 {
    fn recompute_sha256(&self) -> Result<String, NisarSubsetPlanningErrorV2> {
        #[derive(Serialize)]
        struct Identity<'a> {
            checkpoint_contract: &'a str,
            schema_version: u32,
            workspace_canonical_path: &'a str,
            workspace_identity: &'a str,
            task_id: &'a str,
            item_id: &'a str,
            prepared_intent_sha256: &'a str,
            final_plan_sha256: &'a str,
            inventory_sha256: &'a str,
            inventory_evidence_sha256: &'a str,
            source_validator_sha256: &'a str,
            source_size: u64,
            sparse_part_identity_sha256: &'a str,
            sparse_part_material_ranges_sha256: &'a str,
            sparse_part_material_sha256: &'a str,
            bootstrap_statistics: &'a NisarSubsetBootstrapStatisticsV2,
            physical_plan: &'a NisarSubsetPhysicalExecutionPlanV2,
        }
        digest_serializable(&Identity {
            checkpoint_contract: &self.checkpoint_contract,
            schema_version: self.schema_version,
            workspace_canonical_path: &self.workspace_canonical_path,
            workspace_identity: &self.workspace_identity,
            task_id: &self.task_id,
            item_id: &self.item_id,
            prepared_intent_sha256: &self.prepared_intent_sha256,
            final_plan_sha256: &self.final_plan_sha256,
            inventory_sha256: &self.inventory_sha256,
            inventory_evidence_sha256: &self.inventory_evidence_sha256,
            source_validator_sha256: &self.source_validator_sha256,
            source_size: self.source_size,
            sparse_part_identity_sha256: &self.sparse_part_identity_sha256,
            sparse_part_material_ranges_sha256: &self.sparse_part_material_ranges_sha256,
            sparse_part_material_sha256: &self.sparse_part_material_sha256,
            bootstrap_statistics: &self.bootstrap_statistics,
            physical_plan: &self.physical_plan,
        })
    }
}

/// Read-only, locally verified planning authority. It grants no network authority
/// and cannot be constructed from caller-supplied checkpoint fields.
pub(crate) struct NisarArchivedPlanningExportV2 {
    checkpoint: NisarSubsetPlanningWorkspaceCheckpointV2,
    frozen_plan: NisarSubsetPlanV2,
    inventory: NisarSubsetVerifiedMetadataInventoryV2,
    material_ranges: Vec<(u64, u64)>,
    prefix_canonical: Option<String>,
}

impl NisarArchivedPlanningExportV2 {
    pub(crate) fn source_size(&self) -> u64 {
        self.checkpoint.source_size
    }

    pub(crate) fn physical_plan(&self) -> &NisarSubsetPhysicalExecutionPlanV2 {
        &self.checkpoint.physical_plan
    }

    pub(crate) fn material_ranges(&self) -> Vec<(u64, u64)> {
        self.material_ranges.clone()
    }

    pub(crate) fn checkpoint_sha256(&self) -> &str {
        &self.checkpoint.checkpoint_sha256
    }

    pub(crate) fn prefix_canonical(&self) -> Option<&str> {
        self.prefix_canonical.as_deref()
    }
}

/// The caller pins the source part against writes for the export/copy lifetime.
/// This path deliberately never opens a writable prefix materializer, binds an
/// HTTP source, creates a workspace, or reconciles an old checkpoint.
pub(crate) fn export_archived_planning_v2(
    work: &DownloadWork,
    workspace: &Hdf5RangeWorkspaceV1,
    control: &WorkerControl,
) -> Result<NisarArchivedPlanningExportV2, NisarSubsetPlanningErrorV2> {
    ensure_active(control)?;
    let prepared = read_prepared_intent(work)?;
    let frozen_plan = parse_nisar_subset_plan_v2_envelope(&work.item.metadata)
        .map_err(|_| NisarSubsetPlanningErrorV2::FrozenPlanInvalid)?
        .ok_or(NisarSubsetPlanningErrorV2::FrozenPlanMissing)?
        .plan()
        .map_err(|_| NisarSubsetPlanningErrorV2::FrozenPlanInvalid)?;
    validate_plan_binding(&frozen_plan, &prepared)?;
    let intent = prepared
        .intent()
        .map_err(|_| NisarSubsetPlanningErrorV2::PreparedIntentInvalid)?;
    let canonical = workspace
        .load_planning_checkpoint_v2()
        .map_err(map_workspace_load_error)?
        .ok_or(NisarSubsetPlanningErrorV2::PlanningWorkspaceMissing)?;
    let checkpoint = parse_planning_workspace_checkpoint_v2(&canonical)?;
    let (path, identity) = workspace
        .bound_workspace_identity_v2()
        .map_err(map_workspace_load_error)?;
    if checkpoint.workspace_canonical_path != path
        || checkpoint.workspace_identity != identity
        || checkpoint.task_id != work.task.task_id
        || checkpoint.item_id != work.item.item_id
        || checkpoint.prepared_intent_sha256 != frozen_plan.prepared_intent_sha256
        || checkpoint.final_plan_sha256 != frozen_plan.canonical_semantic_plan_digest
    {
        return Err(NisarSubsetPlanningErrorV2::PlanningWorkspaceIdentityMismatch);
    }
    let validator = checkpoint.physical_plan.source_validator.clone();
    if digest_serializable(&validator)? != checkpoint.source_validator_sha256
        || validator.content_length != checkpoint.source_size
    {
        return Err(NisarSubsetPlanningErrorV2::ValidatorChanged);
    }
    checkpoint
        .bootstrap_statistics
        .validate_for_science_root(science_root_for_identity(
            &frozen_plan.source_product_identity,
        )?)?;
    let material_ranges =
        planning_material_ranges_from_statistics_v2(&checkpoint.bootstrap_statistics)?;
    if planning_material_ranges_sha256(&material_ranges)?
        != checkpoint.sparse_part_material_ranges_sha256
    {
        return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
    }
    let evidence = workspace
        .inspect_planning_part_evidence_v2(checkpoint.source_size, &material_ranges, &|| {
            control.is_cancelled() || control.is_paused()
        })
        .map_err(map_workspace_load_error)?
        .ok_or(NisarSubsetPlanningErrorV2::PlanningWorkspaceMissing)?;
    if evidence.identity_sha256 != checkpoint.sparse_part_identity_sha256 {
        return Err(NisarSubsetPlanningErrorV2::PlanningWorkspaceIdentityMismatch);
    }
    if evidence.material_sha256 != checkpoint.sparse_part_material_sha256 {
        return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
    }
    let inventory = NisarSubsetVerifiedMetadataInventoryV2::from_verified_live_metadata_hdf5(
        &intent,
        validator,
        &workspace.part_path(),
        &|| control.is_cancelled() || control.is_paused(),
    )
    .map_err(|error| {
        map_live_inventory_error(error, NisarSubsetPlanningErrorV2::RecoveryInvalid)
    })?;
    validate_inventory_against_frozen_plan_v2(&inventory, &frozen_plan)?;
    if inventory.evidence().inventory_sha256() != checkpoint.inventory_sha256
        || inventory.evidence().evidence_sha256() != checkpoint.inventory_evidence_sha256
    {
        return Err(NisarSubsetPlanningErrorV2::InventoryMismatch);
    }
    let physical = build_physical_execution_plan_v2(
        &frozen_plan,
        &inventory,
        &checkpoint.bootstrap_statistics,
        &workspace.part_path(),
        checkpoint.physical_plan.combined_budget.budget_policy,
        &|| control.is_cancelled() || control.is_paused(),
    )?;
    physical.validate_binding(&frozen_plan)?;
    if physical != checkpoint.physical_plan
        || physical.execution_transfer_ranges()?
            != checkpoint.physical_plan.execution_transfer_ranges()?
    {
        return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
    }
    let prefix_canonical = workspace
        .load_planning_prefix_checkpoint_v2()
        .map_err(map_workspace_load_error)?;
    if let Some(canonical) = prefix_canonical.as_deref() {
        let prefix = parse_prefix_materialization_checkpoint(canonical, false)?;
        if prefix.source_size != checkpoint.source_size
            || prefix.source_validator_sha256 != checkpoint.source_validator_sha256
            || prefix.source_identity_sha256 != prefix_source_identity_sha256(&intent)?
            || prefix.completed_ranges.iter().any(|completed| {
                let mut cursor = completed.start;
                for &(start, end) in &material_ranges {
                    if start > cursor {
                        break;
                    }
                    if end > cursor {
                        cursor = end;
                    }
                    if cursor >= completed.end_exclusive {
                        return false;
                    }
                }
                true
            })
        {
            return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
        }
        let mut part = File::open(workspace.part_path())
            .map_err(|_| NisarSubsetPlanningErrorV2::RecoveryInvalid)?;
        verify_prefix_completed_ranges(&prefix, &mut part)?;
    }
    ensure_active(control)?;
    Ok(NisarArchivedPlanningExportV2 {
        checkpoint,
        frozen_plan,
        inventory,
        material_ranges,
        prefix_canonical,
    })
}

/// Rebuild target authority from the copied HDF5, preserving every planning
/// charge. The held destination lock covers both comparisons and publication.
pub(crate) fn rebind_archived_planning_v2(
    export: &NisarArchivedPlanningExportV2,
    new_work: &DownloadWork,
    dest: &Hdf5RangeWorkspaceV1,
    control: &WorkerControl,
    held_lock: &crate::hdf5_range_transfer::Hdf5WorkspaceLockV1,
) -> Result<(), NisarSubsetPlanningErrorV2> {
    ensure_active(control)?;
    let prepared = read_prepared_intent(new_work)?;
    let frozen_plan = parse_nisar_subset_plan_v2_envelope(&new_work.item.metadata)
        .map_err(|_| NisarSubsetPlanningErrorV2::FrozenPlanInvalid)?
        .ok_or(NisarSubsetPlanningErrorV2::FrozenPlanMissing)?
        .plan()
        .map_err(|_| NisarSubsetPlanningErrorV2::FrozenPlanInvalid)?;
    validate_plan_binding(&frozen_plan, &prepared)?;
    if frozen_plan != export.frozen_plan {
        return Err(NisarSubsetPlanningErrorV2::FrozenPlanInvalid);
    }
    let (path, identity) = dest
        .bound_workspace_identity_v2()
        .map_err(map_workspace_load_error)?;
    // Design 2026-10-17 §3: an archived recovery never targets its own source,
    // by recorded path or by directory identity.
    if new_work.task.task_id == export.checkpoint.task_id
        || path == export.checkpoint.workspace_canonical_path
        || identity == export.checkpoint.workspace_identity
    {
        return Err(NisarSubsetPlanningErrorV2::PlanningWorkspaceIdentityMismatch);
    }
    let evidence = dest
        .inspect_planning_part_evidence_v2(export.source_size(), &export.material_ranges, &|| {
            control.is_cancelled() || control.is_paused()
        })
        .map_err(map_workspace_load_error)?
        .ok_or(NisarSubsetPlanningErrorV2::PlanningWorkspaceMissing)?;
    if evidence.material_sha256 != export.checkpoint.sparse_part_material_sha256 {
        return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
    }
    let intent = prepared
        .intent()
        .map_err(|_| NisarSubsetPlanningErrorV2::PreparedIntentInvalid)?;
    let inventory = NisarSubsetVerifiedMetadataInventoryV2::from_verified_live_metadata_hdf5(
        &intent,
        export.checkpoint.physical_plan.source_validator.clone(),
        &dest.part_path(),
        &|| control.is_cancelled() || control.is_paused(),
    )
    .map_err(|error| {
        map_live_inventory_error(error, NisarSubsetPlanningErrorV2::RecoveryInvalid)
    })?;
    validate_inventory_against_frozen_plan_v2(&inventory, &frozen_plan)?;
    if inventory.evidence().inventory_sha256() != export.inventory.evidence().inventory_sha256()
        || inventory.evidence().evidence_sha256() != export.inventory.evidence().evidence_sha256()
    {
        return Err(NisarSubsetPlanningErrorV2::InventoryMismatch);
    }
    let physical = build_physical_execution_plan_v2(
        &frozen_plan,
        &inventory,
        &export.checkpoint.bootstrap_statistics,
        &dest.part_path(),
        export
            .checkpoint
            .physical_plan
            .combined_budget
            .budget_policy,
        &|| control.is_cancelled() || control.is_paused(),
    )?;
    physical.validate_binding(&frozen_plan)?;
    if physical != export.checkpoint.physical_plan
        || physical.execution_transfer_ranges()?
            != export
                .checkpoint
                .physical_plan
                .execution_transfer_ranges()?
    {
        return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
    }
    let mut checkpoint = export.checkpoint.clone();
    checkpoint.workspace_canonical_path = path;
    checkpoint.workspace_identity = identity;
    checkpoint.task_id = new_work.task.task_id.clone();
    checkpoint.item_id = new_work.item.item_id.clone();
    checkpoint.sparse_part_identity_sha256 = evidence.identity_sha256;
    checkpoint.physical_plan = physical;
    checkpoint.checkpoint_sha256 = checkpoint.recompute_sha256()?;
    let canonical = serde_json::to_string(&checkpoint)
        .map_err(|_| NisarSubsetPlanningErrorV2::RecoveryInvalid)?;
    let existing = dest
        .load_planning_checkpoint_v2()
        .map_err(map_workspace_load_error)?;
    let existing_prefix = dest
        .load_planning_prefix_checkpoint_v2()
        .map_err(map_workspace_load_error)?;
    if existing.as_ref().is_some_and(|value| value != &canonical)
        || existing_prefix
            .as_ref()
            .is_some_and(|value| Some(value) != export.prefix_canonical.as_ref())
    {
        return Err(NisarSubsetPlanningErrorV2::PlanningWorkspaceIdentityMismatch);
    }
    ensure_active(control)?;
    if existing_prefix.is_none() {
        if let Some(prefix) = export.prefix_canonical() {
            dest.persist_planning_prefix_checkpoint_locked_v2(prefix, held_lock)
                .map_err(map_workspace_load_error)?;
        }
    }
    if existing.is_none() {
        dest.persist_planning_checkpoint_locked_v2(&canonical, held_lock)
            .map_err(map_workspace_load_error)?;
    }
    ensure_active(control)
}

#[derive(Debug)]
struct BootstrapReservation {
    id: u64,
    event: BootstrapEvent,
}

#[derive(Debug)]
struct BootstrapReservationToken {
    id: u64,
}

struct BootstrapLedger {
    source_size: u64,
    source_validator_sha256: Option<String>,
    selected_science_ranges: Vec<NisarSubsetBootstrapRangeV2>,
    events: Vec<BootstrapEvent>,
    pending: Option<BootstrapReservation>,
    next_reservation_id: u64,
    max_request_count: u64,
    max_response_body_bytes: u64,
    charged_failure_requests: u64,
    charged_failure_response_bytes: u64,
}

impl BootstrapLedger {
    fn new(
        source_size: u64,
        source_validator_sha256: String,
        selected_science_ranges: Vec<(u64, u64)>,
    ) -> Result<Self, NisarSubsetPlanningErrorV2> {
        Self::new_with_request_limit(
            source_size,
            source_validator_sha256,
            selected_science_ranges,
            NISAR_LIVE_MAX_REQUESTS,
        )
    }

    fn new_with_request_limit(
        source_size: u64,
        source_validator_sha256: String,
        selected_science_ranges: Vec<(u64, u64)>,
        max_request_count: u64,
    ) -> Result<Self, NisarSubsetPlanningErrorV2> {
        if source_size == 0
            || !valid_sha256(&source_validator_sha256)
            || !valid_planning_request_limit_v2(max_request_count)
            || selected_science_ranges.is_empty()
            || selected_science_ranges
                .iter()
                .any(|(start, end)| start >= end || *end > source_size)
            || selected_science_ranges
                .windows(2)
                .any(|pair| pair[0].1 >= pair[1].0)
        {
            return Err(NisarSubsetPlanningErrorV2::BootstrapStatisticsInvalid);
        }
        let selected_science_ranges = selected_science_ranges
            .into_iter()
            .map(|(start, end)| NisarSubsetBootstrapRangeV2::new(start, end))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            source_size,
            source_validator_sha256: Some(source_validator_sha256),
            selected_science_ranges,
            events: Vec::new(),
            pending: None,
            next_reservation_id: 0,
            max_request_count,
            max_response_body_bytes: nisar_live_budget_response_limit(source_size)
                .map_err(|_| NisarSubsetPlanningErrorV2::BootstrapBudgetExceeded)?,
            charged_failure_requests: 0,
            charged_failure_response_bytes: 0,
        })
    }

    fn new_preflight_with_request_limit(
        source_size: u64,
        max_request_count: u64,
    ) -> Result<Self, NisarSubsetPlanningErrorV2> {
        if source_size == 0 || !valid_planning_request_limit_v2(max_request_count) {
            return Err(NisarSubsetPlanningErrorV2::BootstrapStatisticsInvalid);
        }
        Ok(Self {
            source_size,
            source_validator_sha256: None,
            selected_science_ranges: Vec::new(),
            events: Vec::new(),
            pending: None,
            next_reservation_id: 0,
            max_request_count,
            max_response_body_bytes: nisar_live_budget_response_limit(source_size)
                .map_err(|_| NisarSubsetPlanningErrorV2::BootstrapBudgetExceeded)?,
            charged_failure_requests: 0,
            charged_failure_response_bytes: 0,
        })
    }

    fn bind_source_validator(
        &mut self,
        source_validator_sha256: String,
    ) -> Result<(), NisarSubsetPlanningErrorV2> {
        if self.source_validator_sha256.is_some()
            || !valid_sha256(&source_validator_sha256)
            || !self.events.is_empty()
            || !matches!(
                self.pending.as_ref().map(|pending| pending.event.category),
                Some(BootstrapCategory::Preflight)
            )
        {
            return Err(NisarSubsetPlanningErrorV2::BootstrapStatisticsInvalid);
        }
        self.source_validator_sha256 = Some(source_validator_sha256);
        Ok(())
    }

    fn bind_selected_science_ranges(
        &mut self,
        selected_science_ranges: Vec<(u64, u64)>,
    ) -> Result<(), NisarSubsetPlanningErrorV2> {
        if self.pending.is_some()
            || !self.selected_science_ranges.is_empty()
            || selected_science_ranges.is_empty()
            || selected_science_ranges
                .iter()
                .any(|(start, end)| start >= end || *end > self.source_size)
            || selected_science_ranges
                .windows(2)
                .any(|pair| pair[0].1 >= pair[1].0)
        {
            return Err(NisarSubsetPlanningErrorV2::BootstrapStatisticsInvalid);
        }
        let bound = selected_science_ranges
            .into_iter()
            .map(|(start, end)| NisarSubsetBootstrapRangeV2::new(start, end))
            .collect::<Result<Vec<_>, _>>()?;
        if self.events.iter().any(|event| {
            event.category.materializes()
                && !metadata_prefix_overlap_allowed(event.category, &event.range)
                && bound
                    .iter()
                    .any(|science| ranges_overlap(&event.range, science))
        }) {
            return Err(NisarSubsetPlanningErrorV2::ScienceTransferObserved);
        }
        self.selected_science_ranges = bound;
        Ok(())
    }

    fn accepted_totals(&self) -> Result<(u64, u64), NisarSubsetPlanningErrorV2> {
        self.events
            .iter()
            .try_fold((0_u64, 0_u64), |(request_total, response_total), event| {
                Ok((
                    request_total
                        .checked_add(event.request_count)
                        .ok_or(NisarSubsetPlanningErrorV2::BootstrapBudgetExceeded)?,
                    response_total
                        .checked_add(event.response_body_bytes)
                        .ok_or(NisarSubsetPlanningErrorV2::BootstrapBudgetExceeded)?,
                ))
            })
    }

    fn check_combined_budget(
        &self,
        additional_requests: u64,
        additional_response_bytes: u64,
    ) -> Result<(), NisarSubsetPlanningErrorV2> {
        if additional_requests == 0 && additional_response_bytes != 0 {
            return Err(NisarSubsetPlanningErrorV2::BootstrapStatisticsInvalid);
        }
        let (accepted_requests, accepted_response_bytes) = self.accepted_totals()?;
        let combined_requests = accepted_requests
            .checked_add(self.charged_failure_requests)
            .and_then(|total| total.checked_add(additional_requests))
            .ok_or(NisarSubsetPlanningErrorV2::BootstrapBudgetExceeded)?;
        let combined_response_bytes = accepted_response_bytes
            .checked_add(self.charged_failure_response_bytes)
            .and_then(|total| total.checked_add(additional_response_bytes))
            .ok_or(NisarSubsetPlanningErrorV2::BootstrapBudgetExceeded)?;
        if combined_requests > self.max_request_count
            || combined_response_bytes > self.max_response_body_bytes
        {
            return Err(NisarSubsetPlanningErrorV2::BootstrapBudgetExceeded);
        }
        Ok(())
    }

    fn check_attempt_budget(
        &self,
        token: &BootstrapReservationToken,
        conservative_request_count: u64,
        conservative_response_body_bytes: u64,
    ) -> Result<(), NisarSubsetPlanningErrorV2> {
        if conservative_request_count == 0
            || self
                .pending
                .as_ref()
                .is_none_or(|reservation| reservation.id != token.id)
        {
            return Err(NisarSubsetPlanningErrorV2::BootstrapStatisticsInvalid);
        }
        self.check_combined_budget(conservative_request_count, conservative_response_body_bytes)
    }

    fn reserve(
        &mut self,
        category: BootstrapCategory,
        start: u64,
        end_exclusive: u64,
        expected_response_body_bytes: u64,
    ) -> Result<BootstrapReservationToken, NisarSubsetPlanningErrorV2> {
        if self.pending.is_some() {
            return Err(NisarSubsetPlanningErrorV2::BootstrapStatisticsInvalid);
        }
        let range = NisarSubsetBootstrapRangeV2::new(start, end_exclusive)?;
        if range.end_exclusive > self.source_size
            || expected_response_body_bytes == 0
            || expected_response_body_bytes != range.len()
        {
            return Err(NisarSubsetPlanningErrorV2::BootstrapStatisticsInvalid);
        }
        if category.materializes()
            && !metadata_prefix_overlap_allowed(category, &range)
            && self
                .selected_science_ranges
                .iter()
                .any(|science| ranges_overlap(&range, science))
        {
            return Err(NisarSubsetPlanningErrorV2::ScienceTransferObserved);
        }
        if category.materializes()
            && self
                .events
                .iter()
                .any(|event| event.category.materializes() && ranges_overlap(&range, &event.range))
        {
            return Err(NisarSubsetPlanningErrorV2::BootstrapStatisticsInvalid);
        }
        if matches!(
            category,
            BootstrapCategory::Preflight | BootstrapCategory::Revalidation
        ) && (start, end_exclusive) != (0, 1)
        {
            return Err(NisarSubsetPlanningErrorV2::BootstrapStatisticsInvalid);
        }
        self.check_combined_budget(1, expected_response_body_bytes)?;
        let id = self.next_reservation_id;
        self.next_reservation_id = self
            .next_reservation_id
            .checked_add(1)
            .ok_or(NisarSubsetPlanningErrorV2::BootstrapBudgetExceeded)?;
        self.pending = Some(BootstrapReservation {
            id,
            event: BootstrapEvent {
                category,
                range,
                request_count: 1,
                response_body_bytes: expected_response_body_bytes,
            },
        });
        Ok(BootstrapReservationToken { id })
    }

    fn commit(
        &mut self,
        token: BootstrapReservationToken,
        actual_response_body_bytes: u64,
    ) -> Result<(), NisarSubsetPlanningErrorV2> {
        self.commit_with_request_count(token, actual_response_body_bytes, 1)
    }

    fn commit_with_request_count(
        &mut self,
        token: BootstrapReservationToken,
        actual_response_body_bytes: u64,
        request_count: u64,
    ) -> Result<(), NisarSubsetPlanningErrorV2> {
        let reservation = self
            .pending
            .as_ref()
            .ok_or(NisarSubsetPlanningErrorV2::BootstrapStatisticsInvalid)?;
        if reservation.id != token.id
            || request_count == 0
            || actual_response_body_bytes != reservation.event.response_body_bytes
        {
            return Err(NisarSubsetPlanningErrorV2::BootstrapStatisticsInvalid);
        }
        self.check_combined_budget(request_count, actual_response_body_bytes)?;
        let mut event = self
            .pending
            .take()
            .ok_or(NisarSubsetPlanningErrorV2::BootstrapStatisticsInvalid)?
            .event;
        event.request_count = request_count;
        self.events.push(event);
        Ok(())
    }

    fn cancel(
        &mut self,
        token: BootstrapReservationToken,
    ) -> Result<(), NisarSubsetPlanningErrorV2> {
        let reservation = self
            .pending
            .take()
            .ok_or(NisarSubsetPlanningErrorV2::BootstrapStatisticsInvalid)?;
        if reservation.id != token.id {
            return Err(NisarSubsetPlanningErrorV2::BootstrapStatisticsInvalid);
        }
        Ok(())
    }

    fn fail(
        &mut self,
        token: BootstrapReservationToken,
        observation: &NisarSubsetRangeFailureObservationV2,
    ) -> Result<(), NisarSubsetPlanningErrorV2> {
        let requested_response_body_bytes = self
            .pending
            .as_ref()
            .filter(|pending| pending.id == token.id)
            .map(|pending| pending.event.response_body_bytes)
            .ok_or(NisarSubsetPlanningErrorV2::BootstrapStatisticsInvalid)?;
        let charged_requests = if observation.disposition == "before_send_rejected" {
            0
        } else {
            observation.request_count
        };
        let charged_response_body_bytes = if observation.disposition == "before_send_rejected" {
            0
        } else if let Some(observed_bytes) = observation.response_body_bytes {
            observed_bytes
        } else {
            requested_response_body_bytes
                .checked_mul(observation.request_count)
                .ok_or(NisarSubsetPlanningErrorV2::BootstrapBudgetExceeded)?
        };
        self.fail_with_charge(token, charged_requests, charged_response_body_bytes)
    }

    fn next_failure_totals(
        &self,
        token: &BootstrapReservationToken,
        charged_requests: u64,
        charged_response_body_bytes: u64,
    ) -> Result<(u64, u64), NisarSubsetPlanningErrorV2> {
        if self
            .pending
            .as_ref()
            .is_none_or(|reservation| reservation.id != token.id)
            || (charged_requests == 0 && charged_response_body_bytes != 0)
        {
            return Err(NisarSubsetPlanningErrorV2::BootstrapStatisticsInvalid);
        }
        self.check_combined_budget(charged_requests, charged_response_body_bytes)?;
        Ok((
            self.charged_failure_requests
                .checked_add(charged_requests)
                .ok_or(NisarSubsetPlanningErrorV2::BootstrapBudgetExceeded)?,
            self.charged_failure_response_bytes
                .checked_add(charged_response_body_bytes)
                .ok_or(NisarSubsetPlanningErrorV2::BootstrapBudgetExceeded)?,
        ))
    }

    fn charge_retryable_attempt(
        &mut self,
        token: &BootstrapReservationToken,
        charged_requests: u64,
        charged_response_body_bytes: u64,
    ) -> Result<(), NisarSubsetPlanningErrorV2> {
        if charged_requests == 0 {
            return Err(NisarSubsetPlanningErrorV2::BootstrapStatisticsInvalid);
        }
        let (next_requests, next_response_bytes) =
            self.next_failure_totals(token, charged_requests, charged_response_body_bytes)?;
        self.charged_failure_requests = next_requests;
        self.charged_failure_response_bytes = next_response_bytes;
        Ok(())
    }

    fn charge_persisted_failure(
        &mut self,
        charged_requests: u64,
        charged_response_body_bytes: u64,
    ) -> Result<(), NisarSubsetPlanningErrorV2> {
        if self.pending.is_some() || (charged_requests == 0 && charged_response_body_bytes != 0) {
            return Err(NisarSubsetPlanningErrorV2::BootstrapStatisticsInvalid);
        }
        self.check_combined_budget(charged_requests, charged_response_body_bytes)?;
        let next_requests = self
            .charged_failure_requests
            .checked_add(charged_requests)
            .ok_or(NisarSubsetPlanningErrorV2::BootstrapBudgetExceeded)?;
        let next_response_bytes = self
            .charged_failure_response_bytes
            .checked_add(charged_response_body_bytes)
            .ok_or(NisarSubsetPlanningErrorV2::BootstrapBudgetExceeded)?;
        self.charged_failure_requests = next_requests;
        self.charged_failure_response_bytes = next_response_bytes;
        Ok(())
    }

    fn fail_with_charge(
        &mut self,
        token: BootstrapReservationToken,
        charged_requests: u64,
        charged_response_body_bytes: u64,
    ) -> Result<(), NisarSubsetPlanningErrorV2> {
        let (next_requests, next_response_bytes) =
            self.next_failure_totals(&token, charged_requests, charged_response_body_bytes)?;
        self.pending
            .take()
            .ok_or(NisarSubsetPlanningErrorV2::BootstrapStatisticsInvalid)?;
        self.charged_failure_requests = next_requests;
        self.charged_failure_response_bytes = next_response_bytes;
        Ok(())
    }

    fn statistics(self) -> Result<NisarSubsetBootstrapStatisticsV2, NisarSubsetPlanningErrorV2> {
        if self.pending.is_some() || self.selected_science_ranges.is_empty() {
            return Err(NisarSubsetPlanningErrorV2::BootstrapStatisticsInvalid);
        }
        self.check_combined_budget(0, 0)?;
        let category = |wanted| project_category(&self.events, wanted);
        let total_planning_request_count = self
            .events
            .iter()
            .try_fold(0_u64, |total, event| total.checked_add(event.request_count))
            .ok_or(NisarSubsetPlanningErrorV2::BootstrapBudgetExceeded)?;
        let total_planning_response_body_bytes =
            self.events.iter().try_fold(0_u64, |total, event| {
                total.checked_add(event.response_body_bytes)
            });
        let science_events = self.events.iter().filter(|event| {
            event.category.materializes()
                && !metadata_prefix_overlap_allowed(event.category, &event.range)
                && self
                    .selected_science_ranges
                    .iter()
                    .any(|science| ranges_overlap(&event.range, science))
        });
        let science_request_count = science_events.clone().count() as u64;
        let science_response_body_bytes = science_events.fold(0_u64, |total, event| {
            total.saturating_add(event.response_body_bytes)
        });
        if science_request_count != 0 || science_response_body_bytes != 0 {
            return Err(NisarSubsetPlanningErrorV2::ScienceTransferObserved);
        }
        Ok(NisarSubsetBootstrapStatisticsV2 {
            frozen_plan_fast_path: false,
            preflight: project_category(&self.events, BootstrapCategory::Preflight),
            revalidation: project_category(&self.events, BootstrapCategory::Revalidation),
            metadata_prefix: category(BootstrapCategory::MetadataPrefix),
            metadata_discovery: category(BootstrapCategory::MetadataDiscovery),
            additional_geolocation: category(BootstrapCategory::AdditionalGeolocation),
            total_planning_request_count,
            total_planning_response_body_bytes: total_planning_response_body_bytes
                .ok_or(NisarSubsetPlanningErrorV2::BootstrapBudgetExceeded)?,
            charged_failure_requests: self.charged_failure_requests,
            charged_failure_response_bytes: self.charged_failure_response_bytes,
            source_size: self.source_size,
            source_validator_sha256: Some(
                self.source_validator_sha256
                    .ok_or(NisarSubsetPlanningErrorV2::BootstrapStatisticsInvalid)?,
            ),
            bootstrap_budget: Some(NisarSubsetBootstrapBudgetV2 {
                max_request_count: self.max_request_count,
                max_response_body_bytes: self.max_response_body_bytes,
            }),
            science_request_count,
            science_response_body_bytes,
        })
    }
}

fn project_category(
    events: &[BootstrapEvent],
    category: BootstrapCategory,
) -> NisarSubsetBootstrapCategoryStatisticsV2 {
    let matching = events.iter().filter(|event| event.category == category);
    let requested_ranges = matching
        .clone()
        .map(|event| event.range.clone())
        .collect::<Vec<_>>();
    let request_counts = matching
        .clone()
        .map(|event| event.request_count)
        .collect::<Vec<_>>();
    NisarSubsetBootstrapCategoryStatisticsV2 {
        request_count: request_counts.iter().copied().sum(),
        response_body_bytes: matching.fold(0_u64, |total, event| {
            total.saturating_add(event.response_body_bytes)
        }),
        requested_ranges,
        request_counts,
    }
}

pub struct NisarSubsetMetadataBootstrapOutputV2 {
    inventory: NisarSubsetVerifiedMetadataInventoryV2,
    ledger: BootstrapLedger,
}

pub struct NisarSubsetBootstrapRevalidationV2 {
    prepared_intent_sha256: String,
    source_validator_sha256: String,
    actual_response_body_bytes: u64,
    request_count: u64,
}

mod sealed {
    pub trait Sealed {}
}

pub trait NisarSubsetMetadataBootstrapV2: sealed::Sealed + Send + Sync {
    /// A prepared checkpoint is recovery evidence, never Store execution authority.
    fn recover_prepared_plan(
        &self,
        _work: &DownloadWork,
        _prepared_intent: &NisarSubsetPreparedIntentV2,
        _control: &WorkerControl,
    ) -> Result<Option<NisarSubsetPlanV2>, NisarSubsetPlanningErrorV2> {
        Ok(None)
    }

    fn repair_missing_planning_workspace(
        &self,
        _work: &DownloadWork,
        _plan: &NisarSubsetPlanV2,
        _control: &WorkerControl,
        _transfer_progress: &mut dyn FnMut(u64),
    ) -> Result<(), NisarSubsetPlanningErrorV2> {
        Err(NisarSubsetPlanningErrorV2::PlanningWorkspaceMissing)
    }

    fn bootstrap(
        &self,
        work: &DownloadWork,
        prepared_intent: &NisarSubsetPreparedIntentV2,
        control: &WorkerControl,
    ) -> Result<NisarSubsetMetadataBootstrapOutputV2, NisarSubsetPlanningErrorV2>;

    fn bootstrap_with_progress(
        &self,
        work: &DownloadWork,
        prepared_intent: &NisarSubsetPreparedIntentV2,
        control: &WorkerControl,
        _transfer_progress: &mut dyn FnMut(u64),
    ) -> Result<NisarSubsetMetadataBootstrapOutputV2, NisarSubsetPlanningErrorV2> {
        self.bootstrap(work, prepared_intent, control)
    }

    fn revalidate_before_freeze(
        &self,
        work: &DownloadWork,
        startup_prepared_intent_sha256: &str,
        startup_source_validator_sha256: &str,
        control: &WorkerControl,
    ) -> Result<NisarSubsetBootstrapRevalidationV2, NisarSubsetPlanningErrorV2>;

    fn build_and_checkpoint_physical_plan(
        &self,
        work: &DownloadWork,
        frozen_plan: &NisarSubsetPlanV2,
        inventory: &NisarSubsetVerifiedMetadataInventoryV2,
        statistics: &NisarSubsetBootstrapStatisticsV2,
        control: &WorkerControl,
    ) -> Result<NisarSubsetPhysicalExecutionPlanV2, NisarSubsetPlanningErrorV2>;

    fn reopen_planning_workspace(
        &self,
        work: &DownloadWork,
        frozen_plan: &NisarSubsetPlanV2,
        control: &WorkerControl,
    ) -> Result<
        (
            NisarSubsetPhysicalExecutionPlanV2,
            NisarSubsetBootstrapStatisticsV2,
            NisarSubsetVerifiedMetadataInventoryV2,
        ),
        NisarSubsetPlanningErrorV2,
    >;
}

/// Concrete bounded local-range transport used by the real-HDF5 planning and
/// Scheduler/Store integration harness. The adapter state machine and ledger
/// are the same authority boundary used by a future HTTP transport binding.
pub(crate) struct NisarSubsetLocalHdf5MetadataBootstrapV2 {
    source_path: PathBuf,
    hdf5_extent: u64,
    workspace: Hdf5RangeWorkspaceV1,
    #[cfg(test)]
    revalidation_hook: Option<std::sync::Arc<dyn Fn() + Send + Sync>>,
    #[cfg(test)]
    copy_counter: Option<std::sync::Arc<std::sync::atomic::AtomicU64>>,
}

impl sealed::Sealed for NisarSubsetLocalHdf5MetadataBootstrapV2 {}

impl NisarSubsetLocalHdf5MetadataBootstrapV2 {
    #[cfg(test)]
    pub(crate) fn new_for_test(
        source_path: PathBuf,
        hdf5_extent: u64,
        workspace: Hdf5RangeWorkspaceV1,
    ) -> Result<Self, NisarSubsetPlanningErrorV2> {
        let size = std::fs::metadata(&source_path)
            .map_err(|_| NisarSubsetPlanningErrorV2::BootstrapFailed)?
            .len();
        if hdf5_extent == 0 || hdf5_extent != size {
            return Err(NisarSubsetPlanningErrorV2::BootstrapFailed);
        }
        Ok(Self {
            source_path,
            hdf5_extent,
            workspace,
            revalidation_hook: None,
            copy_counter: None,
        })
    }

    #[cfg(test)]
    pub(crate) fn planning_checkpoint_path_for_test(&self) -> PathBuf {
        self.workspace.planning_checkpoint_path_v2()
    }

    pub(crate) fn science_workspace_v2(&self) -> &Hdf5RangeWorkspaceV1 {
        &self.workspace
    }

    #[cfg(test)]
    pub(crate) fn with_revalidation_hook_for_test(
        mut self,
        hook: std::sync::Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        self.revalidation_hook = Some(hook);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_copy_counter_for_test(
        mut self,
        counter: std::sync::Arc<std::sync::atomic::AtomicU64>,
    ) -> Self {
        self.copy_counter = Some(counter);
        self
    }

    fn source_validator(&self) -> Result<NisarSubsetSourceValidatorV1, NisarSubsetPlanningErrorV2> {
        let (content_sha256, content_length) =
            crate::hdf5_range_transfer::guarded_regular_file_sha256_v2(&self.source_path)
                .map_err(|_| NisarSubsetPlanningErrorV2::BootstrapFailed)?;
        Ok(NisarSubsetSourceValidatorV1 {
            kind: NisarSubsetValidatorKind::StrongEtag,
            value: format!("\"sha256-{content_sha256}\""),
            content_length,
        })
    }

    fn copy_range(
        &self,
        target: &mut File,
        start: u64,
        end_exclusive: u64,
        control: &WorkerControl,
    ) -> Result<u64, NisarSubsetPlanningErrorV2> {
        if control.is_cancelled() || control.is_paused() || start >= end_exclusive {
            return Err(NisarSubsetPlanningErrorV2::ExecutionStopped);
        }
        let mut source = File::open(&self.source_path)
            .map_err(|_| NisarSubsetPlanningErrorV2::BootstrapFailed)?;
        source
            .seek(SeekFrom::Start(start))
            .and_then(|_| target.seek(SeekFrom::Start(start)))
            .map_err(|_| NisarSubsetPlanningErrorV2::BootstrapFailed)?;
        let mut remaining = end_exclusive - start;
        let mut copied = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        while remaining > 0 {
            if control.is_cancelled() || control.is_paused() {
                return Err(NisarSubsetPlanningErrorV2::ExecutionStopped);
            }
            let limit = usize::try_from(remaining.min(buffer.len() as u64))
                .map_err(|_| NisarSubsetPlanningErrorV2::BootstrapFailed)?;
            source
                .read_exact(&mut buffer[..limit])
                .and_then(|_| target.write_all(&buffer[..limit]))
                .map_err(|_| NisarSubsetPlanningErrorV2::BootstrapFailed)?;
            #[cfg(test)]
            if let Some(counter) = &self.copy_counter {
                counter.fetch_add(limit as u64, std::sync::atomic::Ordering::SeqCst);
            }
            copied += limit as u64;
            remaining -= limit as u64;
        }
        Ok(copied)
    }

    fn copy_reserved_range(
        &self,
        ledger: &mut BootstrapLedger,
        target: &mut File,
        category: BootstrapCategory,
        start: u64,
        end_exclusive: u64,
        control: &WorkerControl,
    ) -> Result<u64, NisarSubsetPlanningErrorV2> {
        let expected_response_body_bytes = end_exclusive
            .checked_sub(start)
            .ok_or(NisarSubsetPlanningErrorV2::BootstrapStatisticsInvalid)?;
        let reservation =
            ledger.reserve(category, start, end_exclusive, expected_response_body_bytes)?;
        match self.copy_range(target, start, end_exclusive, control) {
            Ok(actual_response_body_bytes) => {
                ledger.commit(reservation, actual_response_body_bytes)?;
                Ok(actual_response_body_bytes)
            }
            Err(error) => {
                ledger.cancel(reservation)?;
                Err(error)
            }
        }
    }
}

impl NisarSubsetMetadataBootstrapV2 for NisarSubsetLocalHdf5MetadataBootstrapV2 {
    fn recover_prepared_plan(
        &self,
        work: &DownloadWork,
        prepared_intent: &NisarSubsetPreparedIntentV2,
        control: &WorkerControl,
    ) -> Result<Option<NisarSubsetPlanV2>, NisarSubsetPlanningErrorV2> {
        recover_prepared_checkpoint_plan_v2(self, &self.workspace, work, prepared_intent, control)
    }

    fn bootstrap(
        &self,
        work: &DownloadWork,
        prepared_intent: &NisarSubsetPreparedIntentV2,
        control: &WorkerControl,
    ) -> Result<NisarSubsetMetadataBootstrapOutputV2, NisarSubsetPlanningErrorV2> {
        ensure_active(control)?;
        let (science_root, selected_paths) = selected_science_paths(prepared_intent)?;
        let source_hdf5 = crate::hdf5_file::open(&self.source_path)
            .map_err(|_| NisarSubsetPlanningErrorV2::BootstrapFailed)?;
        if selected_paths
            .iter()
            .any(|path| source_hdf5.dataset(path).is_err())
        {
            return Err(NisarSubsetPlanningErrorV2::SelectedScienceMissing);
        }
        drop(source_hdf5);
        let catalog = inspect_nisar_planning_physical_ranges_v2(
            &self.source_path,
            &selected_paths,
            science_root,
        )
        .map_err(|_| NisarSubsetPlanningErrorV2::BootstrapFailed)?;
        let source_validator = self.source_validator()?;
        if work
            .item
            .asset_access
            .as_ref()
            .and_then(|asset| asset.expected_size_bytes)
            .is_some_and(|expected| expected != source_validator.content_length)
        {
            return Err(NisarSubsetPlanningErrorV2::BootstrapFailed);
        }
        let validator_sha256 = digest_serializable(&source_validator)?;
        let mut ledger = BootstrapLedger::new_with_request_limit(
            source_validator.content_length,
            validator_sha256,
            catalog.selected_science_ranges.clone(),
            planning_request_limit_v2(science_root),
        )?;
        let part_path = self.workspace.part_path();
        let mut target = self
            .workspace
            .reset_planning_part_v2(source_validator.content_length)
            .map_err(|_| NisarSubsetPlanningErrorV2::BootstrapFailed)?;

        self.copy_reserved_range(
            &mut ledger,
            &mut target,
            BootstrapCategory::Preflight,
            0,
            1,
            control,
        )?;
        let materialization = complement_ranges(&catalog.all_science_ranges, self.hdf5_extent)?;
        if catalog
            .metadata_ranges
            .iter()
            .chain(&catalog.geolocation_ranges)
            .any(|required| {
                !materialization
                    .iter()
                    .any(|available| available.0 <= required.0 && available.1 >= required.1)
            })
        {
            return Err(NisarSubsetPlanningErrorV2::BootstrapFailed);
        }
        for (index, (start, end)) in materialization.into_iter().enumerate() {
            let category = if index == 0 && start == 0 {
                BootstrapCategory::MetadataPrefix
            } else if catalog
                .geolocation_ranges
                .iter()
                .any(|range| tuple_ranges_overlap((start, end), *range))
            {
                BootstrapCategory::AdditionalGeolocation
            } else {
                BootstrapCategory::MetadataDiscovery
            };
            self.copy_reserved_range(&mut ledger, &mut target, category, start, end, control)?;
        }
        target
            .sync_all()
            .map_err(|_| NisarSubsetPlanningErrorV2::BootstrapFailed)?;
        drop(target);
        let inventory = NisarSubsetVerifiedMetadataInventoryV2::from_verified_live_metadata_hdf5(
            prepared_intent,
            source_validator,
            &part_path,
            &|| control.is_cancelled() || control.is_paused(),
        )
        .map_err(|error| {
            map_live_inventory_error(error, NisarSubsetPlanningErrorV2::BootstrapFailed)
        })?;
        Ok(NisarSubsetMetadataBootstrapOutputV2 { inventory, ledger })
    }

    fn revalidate_before_freeze(
        &self,
        _work: &DownloadWork,
        startup_prepared_intent_sha256: &str,
        _startup_source_validator_sha256: &str,
        control: &WorkerControl,
    ) -> Result<NisarSubsetBootstrapRevalidationV2, NisarSubsetPlanningErrorV2> {
        ensure_active(control)?;
        #[cfg(test)]
        if let Some(hook) = &self.revalidation_hook {
            hook();
        }
        let mut source = File::open(&self.source_path)
            .map_err(|_| NisarSubsetPlanningErrorV2::BootstrapFailed)?;
        let mut probe = [0_u8; 1];
        source
            .read_exact(&mut probe)
            .map_err(|_| NisarSubsetPlanningErrorV2::BootstrapFailed)?;
        let bytes = probe.len() as u64;
        let validator = self.source_validator()?;
        Ok(NisarSubsetBootstrapRevalidationV2 {
            prepared_intent_sha256: startup_prepared_intent_sha256.to_owned(),
            source_validator_sha256: digest_serializable(&validator)?,
            actual_response_body_bytes: bytes,
            request_count: 1,
        })
    }

    fn build_and_checkpoint_physical_plan(
        &self,
        work: &DownloadWork,
        frozen_plan: &NisarSubsetPlanV2,
        inventory: &NisarSubsetVerifiedMetadataInventoryV2,
        statistics: &NisarSubsetBootstrapStatisticsV2,
        control: &WorkerControl,
    ) -> Result<NisarSubsetPhysicalExecutionPlanV2, NisarSubsetPlanningErrorV2> {
        ensure_active(control)?;
        let science_root = science_root_for_identity(&frozen_plan.source_product_identity)?;
        statistics.validate_for_science_root(science_root)?;
        let basic_part_evidence = self
            .workspace
            .inspect_planning_part_evidence_v2(inventory.evidence().source_size(), &[], &|| {
                control.is_cancelled() || control.is_paused()
            })
            .map_err(map_workspace_load_error)?
            .ok_or(NisarSubsetPlanningErrorV2::PlanningWorkspaceMissing)?;
        let selected_paths = selected_science_paths_from_selection(
            science_root,
            &frozen_plan.selected_frequency,
            &frozen_plan.selected_polarizations,
        );
        let catalog = inspect_nisar_planning_physical_ranges_v2(
            &self.source_path,
            &selected_paths,
            science_root,
        )
        .map_err(|_| NisarSubsetPlanningErrorV2::RecoveryInvalid)?;
        let (expected_statistics, material_ranges) = expected_bootstrap_checkpoint_v2(
            &catalog,
            inventory.evidence().source_size(),
            inventory.evidence().source_validator_sha256(),
            planning_request_limit_v2(science_root),
        )?;
        if *statistics != expected_statistics {
            return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
        }
        let physical_plan = build_physical_execution_plan_v2(
            frozen_plan,
            inventory,
            statistics,
            &self.workspace.part_path(),
            Some(NisarSubsetCombinedBudgetPolicyV2::PhysicalExecutionUnionV1),
            &|| control.is_cancelled() || control.is_paused(),
        )?;
        physical_plan.validate_binding(frozen_plan)?;
        let part_evidence = self
            .workspace
            .inspect_planning_part_evidence_v2(
                inventory.evidence().source_size(),
                &material_ranges,
                &|| control.is_cancelled() || control.is_paused(),
            )
            .map_err(map_workspace_load_error)?
            .ok_or(NisarSubsetPlanningErrorV2::PlanningWorkspaceMissing)?;
        if part_evidence.identity_sha256 != basic_part_evidence.identity_sha256 {
            return Err(NisarSubsetPlanningErrorV2::PlanningWorkspaceIdentityMismatch);
        }
        if part_evidence.material_sha256
            != planning_source_material_sha256(
                &self.source_path,
                inventory.evidence().source_size(),
                &material_ranges,
            )?
        {
            return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
        }
        let sparse_part_material_ranges_sha256 = planning_material_ranges_sha256(&material_ranges)?;
        let (workspace_canonical_path, workspace_identity) = self
            .workspace
            .bound_workspace_identity_v2()
            .map_err(|_| NisarSubsetPlanningErrorV2::PlanningWorkspaceIdentityMismatch)?;
        let mut checkpoint = NisarSubsetPlanningWorkspaceCheckpointV2 {
            checkpoint_contract: NISAR_SUBSET_PLANNING_WORKSPACE_CHECKPOINT_V2_CONTRACT.to_owned(),
            schema_version: NISAR_SUBSET_PLANNING_WORKSPACE_CHECKPOINT_V2_SCHEMA_VERSION,
            workspace_canonical_path,
            workspace_identity,
            task_id: work.task.task_id.clone(),
            item_id: work.item.item_id.clone(),
            prepared_intent_sha256: frozen_plan.prepared_intent_sha256.clone(),
            final_plan_sha256: frozen_plan.canonical_semantic_plan_digest.clone(),
            inventory_sha256: inventory.evidence().inventory_sha256().to_owned(),
            inventory_evidence_sha256: inventory.evidence().evidence_sha256().to_owned(),
            source_validator_sha256: inventory.evidence().source_validator_sha256().to_owned(),
            source_size: inventory.evidence().source_size(),
            sparse_part_identity_sha256: part_evidence.identity_sha256,
            sparse_part_material_ranges_sha256,
            sparse_part_material_sha256: part_evidence.material_sha256,
            bootstrap_statistics: statistics.clone(),
            physical_plan,
            checkpoint_sha256: String::new(),
        };
        checkpoint.checkpoint_sha256 = checkpoint.recompute_sha256()?;
        let canonical = serde_json::to_string(&checkpoint)
            .map_err(|_| NisarSubsetPlanningErrorV2::RecoveryInvalid)?;
        if let Some(existing) = self
            .workspace
            .load_planning_checkpoint_v2()
            .map_err(map_workspace_load_error)?
        {
            let existing = parse_planning_workspace_checkpoint_v2(&existing)?;
            if existing != checkpoint {
                return Err(NisarSubsetPlanningErrorV2::PlanningWorkspaceIdentityMismatch);
            }
        } else {
            self.workspace
                .persist_planning_checkpoint_v2(&canonical)
                .map_err(|_| NisarSubsetPlanningErrorV2::RecoveryInvalid)?;
        }
        ensure_active(control)?;
        Ok(checkpoint.physical_plan)
    }

    fn reopen_planning_workspace(
        &self,
        work: &DownloadWork,
        frozen_plan: &NisarSubsetPlanV2,
        control: &WorkerControl,
    ) -> Result<
        (
            NisarSubsetPhysicalExecutionPlanV2,
            NisarSubsetBootstrapStatisticsV2,
            NisarSubsetVerifiedMetadataInventoryV2,
        ),
        NisarSubsetPlanningErrorV2,
    > {
        ensure_active(control)?;
        let text = self
            .workspace
            .load_planning_checkpoint_v2()
            .map_err(map_workspace_load_error)?
            .ok_or(NisarSubsetPlanningErrorV2::PlanningWorkspaceMissing)?;
        let checkpoint = parse_planning_workspace_checkpoint_v2(&text)?;
        let (workspace_canonical_path, workspace_identity) = self
            .workspace
            .bound_workspace_identity_v2()
            .map_err(|_| NisarSubsetPlanningErrorV2::PlanningWorkspaceIdentityMismatch)?;
        if checkpoint.workspace_canonical_path != workspace_canonical_path
            || checkpoint.workspace_identity != workspace_identity
            || checkpoint.task_id != work.task.task_id
            || checkpoint.item_id != work.item.item_id
            || checkpoint.prepared_intent_sha256 != frozen_plan.prepared_intent_sha256
            || checkpoint.final_plan_sha256 != frozen_plan.canonical_semantic_plan_digest
            || checkpoint.source_size
                != frozen_plan
                    .source_estimate
                    .as_ref()
                    .map(|estimate| estimate.bytes)
                    .unwrap_or_default()
        {
            return Err(NisarSubsetPlanningErrorV2::PlanningWorkspaceIdentityMismatch);
        }
        let basic_part_evidence = self
            .workspace
            .inspect_planning_part_evidence_v2(checkpoint.source_size, &[], &|| {
                control.is_cancelled() || control.is_paused()
            })
            .map_err(map_workspace_load_error)?
            .ok_or(NisarSubsetPlanningErrorV2::PlanningWorkspaceMissing)?;
        if basic_part_evidence.identity_sha256 != checkpoint.sparse_part_identity_sha256 {
            return Err(NisarSubsetPlanningErrorV2::PlanningWorkspaceIdentityMismatch);
        }
        if checkpoint.inventory_evidence_sha256
            != frozen_plan.source_dependency_graph.evidence_sha256
            || checkpoint.physical_plan.inventory_sha256 != checkpoint.inventory_sha256
            || checkpoint.physical_plan.inventory_evidence_sha256
                != checkpoint.inventory_evidence_sha256
        {
            return Err(NisarSubsetPlanningErrorV2::InventoryMismatch);
        }
        let validator = self.source_validator()?;
        let current_validator_sha256 = digest_serializable(&validator)?;
        if checkpoint.source_validator_sha256 != current_validator_sha256
            || checkpoint.physical_plan.source_validator_sha256 != current_validator_sha256
            || checkpoint.source_size != validator.content_length
        {
            return Err(NisarSubsetPlanningErrorV2::ValidatorChanged);
        }
        checkpoint.bootstrap_statistics.validate_checkpoint()?;
        let prepared_intent = read_prepared_intent(work)?
            .intent()
            .map_err(|_| NisarSubsetPlanningErrorV2::PreparedIntentInvalid)?;
        let recovered_inventory =
            NisarSubsetVerifiedMetadataInventoryV2::from_verified_live_metadata_hdf5(
                &prepared_intent,
                validator,
                &self.workspace.part_path(),
                &|| control.is_cancelled() || control.is_paused(),
            )
            .map_err(|error| {
                map_live_inventory_error(error, NisarSubsetPlanningErrorV2::RecoveryInvalid)
            })?;
        validate_inventory_against_frozen_plan_v2(&recovered_inventory, frozen_plan)?;
        if recovered_inventory.evidence().inventory_sha256() != checkpoint.inventory_sha256
            || recovered_inventory.evidence().evidence_sha256()
                != checkpoint.inventory_evidence_sha256
        {
            return Err(NisarSubsetPlanningErrorV2::InventoryMismatch);
        }
        let science_root = science_root_for_identity(&frozen_plan.source_product_identity)?;
        checkpoint
            .bootstrap_statistics
            .validate_for_science_root(science_root)?;
        let selected_paths = selected_science_paths_from_selection(
            science_root,
            &frozen_plan.selected_frequency,
            &frozen_plan.selected_polarizations,
        );
        let catalog = inspect_nisar_planning_physical_ranges_v2(
            &self.source_path,
            &selected_paths,
            science_root,
        )
        .map_err(|_| NisarSubsetPlanningErrorV2::RecoveryInvalid)?;
        let (expected_statistics, material_ranges) = expected_bootstrap_checkpoint_v2(
            &catalog,
            checkpoint.source_size,
            &checkpoint.source_validator_sha256,
            planning_request_limit_v2(science_root),
        )?;
        if checkpoint.bootstrap_statistics != expected_statistics
            || checkpoint.sparse_part_material_ranges_sha256
                != planning_material_ranges_sha256(&material_ranges)?
        {
            return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
        }
        let part_evidence = self
            .workspace
            .inspect_planning_part_evidence_v2(checkpoint.source_size, &material_ranges, &|| {
                control.is_cancelled() || control.is_paused()
            })
            .map_err(map_workspace_load_error)?
            .ok_or(NisarSubsetPlanningErrorV2::PlanningWorkspaceMissing)?;
        if part_evidence.identity_sha256 != checkpoint.sparse_part_identity_sha256 {
            return Err(NisarSubsetPlanningErrorV2::PlanningWorkspaceIdentityMismatch);
        }
        if part_evidence.material_sha256 != checkpoint.sparse_part_material_sha256
            || part_evidence.material_sha256
                != planning_source_material_sha256(
                    &self.source_path,
                    checkpoint.source_size,
                    &material_ranges,
                )?
        {
            return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
        }
        let expected_physical_plan = build_physical_execution_plan_v2(
            frozen_plan,
            &recovered_inventory,
            &checkpoint.bootstrap_statistics,
            &self.workspace.part_path(),
            checkpoint.physical_plan.combined_budget.budget_policy,
            &|| control.is_cancelled() || control.is_paused(),
        )?;
        expected_physical_plan.validate_binding(frozen_plan)?;
        if checkpoint.physical_plan != expected_physical_plan
            || checkpoint.physical_plan.physical_plan_sha256
                != expected_physical_plan.physical_plan_sha256
        {
            return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
        }
        checkpoint.physical_plan.validate_binding(frozen_plan)?;
        ensure_active(control)?;
        Ok((
            checkpoint.physical_plan,
            checkpoint.bootstrap_statistics.mark_frozen_plan_fast_path(),
            recovered_inventory,
        ))
    }
}

fn source_validator_from_asset(
    asset: &ResolvedAsset,
) -> Result<NisarSubsetSourceValidatorV1, NisarSubsetPlanningErrorV2> {
    let remote = asset.remote_validator();
    let value = remote
        .etag
        .as_deref()
        .filter(|value| {
            let value = value.trim();
            value.len() >= 2
                && value.len() <= 1024
                && value.starts_with('"')
                && value.ends_with('"')
                && !value.starts_with("W/")
                && value[1..value.len() - 1]
                    .bytes()
                    .all(|byte| byte >= 0x21 && byte != 0x22 && byte != 0x7f)
        })
        .ok_or(NisarSubsetPlanningErrorV2::BootstrapStrongValidatorRequired)?;
    let content_length = remote
        .content_length
        .filter(|size| *size > 0)
        .ok_or(NisarSubsetPlanningErrorV2::BootstrapResponseInvalid)?;
    if asset.expected_size() != Some(content_length) {
        return Err(NisarSubsetPlanningErrorV2::BootstrapResponseInvalid);
    }
    Ok(NisarSubsetSourceValidatorV1 {
        kind: NisarSubsetValidatorKind::StrongEtag,
        value: value.to_owned(),
        content_length,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NisarSubsetIoErrorClassV2 {
    Timeout,
    Connection,
    UnexpectedEof,
    Interrupted,
    WouldBlock,
    Other,
}

impl NisarSubsetIoErrorClassV2 {
    fn from_kind(kind: std::io::ErrorKind) -> Self {
        match kind {
            std::io::ErrorKind::TimedOut => Self::Timeout,
            std::io::ErrorKind::ConnectionRefused
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::NotConnected
            | std::io::ErrorKind::BrokenPipe => Self::Connection,
            std::io::ErrorKind::UnexpectedEof => Self::UnexpectedEof,
            std::io::ErrorKind::Interrupted => Self::Interrupted,
            std::io::ErrorKind::WouldBlock => Self::WouldBlock,
            _ => Self::Other,
        }
    }

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::Connection => "connection",
            Self::UnexpectedEof => "unexpected_eof",
            Self::Interrupted => "interrupted",
            Self::WouldBlock => "would_block",
            Self::Other => "other",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NisarSubsetTransportDiagnosticV2 {
    ExactRangeSend(crate::http_transfer::ExactRangeTransportCause),
    PostBodyEofProbeRead {
        io_error_class: NisarSubsetIoErrorClassV2,
    },
}

impl NisarSubsetTransportDiagnosticV2 {
    pub(crate) const fn cause(self) -> &'static str {
        match self {
            Self::ExactRangeSend(cause) => cause.as_str(),
            Self::PostBodyEofProbeRead { .. } => "post_body_eof_probe_read",
        }
    }

    pub(crate) const fn io_error_class(self) -> Option<NisarSubsetIoErrorClassV2> {
        match self {
            Self::ExactRangeSend(_) => None,
            Self::PostBodyEofProbeRead { io_error_class } => Some(io_error_class),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NisarSubsetRangeFailureObservationV2 {
    pub(crate) stage: String,
    pub(crate) start: u64,
    pub(crate) end_exclusive: u64,
    pub(crate) source_size_bytes: u64,
    pub(crate) disposition: &'static str,
    pub(crate) request_count: u64,
    pub(crate) http_status: Option<u16>,
    pub(crate) response_body_bytes: Option<u64>,
    pub(crate) transport_diagnostic: Option<NisarSubsetTransportDiagnosticV2>,
    pub(crate) validator_sha256: Option<String>,
    pub(crate) failure_code: &'static str,
    pub(crate) redirect_occurred: bool,
    pub(crate) redirect_hop_index: Option<u8>,
    pub(crate) redirect_source_host_class: Option<String>,
    pub(crate) redirect_destination_host_class: Option<String>,
    pub(crate) redirect_source_path_class: Option<String>,
    pub(crate) redirect_destination_path_class: Option<String>,
    pub(crate) redirect_same_origin: Option<bool>,
    pub(crate) redirect_scheme_transition: Option<String>,
    pub(crate) redirect_policy_result: Option<String>,
    pub(crate) redirect_rejection_reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct NisarPrefixDurableFailureObservationV2 {
    stage: String,
    start: u64,
    end_exclusive: u64,
    source_size_bytes: u64,
    disposition: String,
    request_count: u64,
    http_status: Option<u16>,
    response_body_bytes: Option<u64>,
    validator_sha256: Option<String>,
    failure_code: String,
    redirect_occurred: bool,
    redirect_hop_index: Option<u8>,
    redirect_source_host_class: Option<String>,
    redirect_destination_host_class: Option<String>,
    redirect_source_path_class: Option<String>,
    redirect_destination_path_class: Option<String>,
    redirect_same_origin: Option<bool>,
    redirect_scheme_transition: Option<String>,
    redirect_policy_result: Option<String>,
    redirect_rejection_reason: Option<String>,
}

impl NisarPrefixDurableFailureObservationV2 {
    fn from_runtime(observation: &NisarSubsetRangeFailureObservationV2) -> Self {
        Self {
            stage: observation.stage.clone(),
            start: observation.start,
            end_exclusive: observation.end_exclusive,
            source_size_bytes: observation.source_size_bytes,
            disposition: observation.disposition.to_owned(),
            request_count: observation.request_count,
            http_status: observation.http_status,
            response_body_bytes: observation.response_body_bytes,
            validator_sha256: observation.validator_sha256.clone(),
            failure_code: observation.failure_code.to_owned(),
            redirect_occurred: observation.redirect_occurred,
            redirect_hop_index: observation.redirect_hop_index,
            redirect_source_host_class: observation.redirect_source_host_class.clone(),
            redirect_destination_host_class: observation.redirect_destination_host_class.clone(),
            redirect_source_path_class: observation.redirect_source_path_class.clone(),
            redirect_destination_path_class: observation.redirect_destination_path_class.clone(),
            redirect_same_origin: observation.redirect_same_origin,
            redirect_scheme_transition: observation.redirect_scheme_transition.clone(),
            redirect_policy_result: observation.redirect_policy_result.clone(),
            redirect_rejection_reason: observation.redirect_rejection_reason.clone(),
        }
    }

    fn to_runtime(
        &self,
    ) -> Result<NisarSubsetRangeFailureObservationV2, NisarSubsetPlanningErrorV2> {
        let disposition = match self.disposition.as_str() {
            "before_send_rejected" => "before_send_rejected",
            "sent_outcome_indeterminate" => "sent_outcome_indeterminate",
            "sent_response_rejected" => "sent_response_rejected",
            _ => return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid),
        };
        let failure_code = match self.failure_code.as_str() {
            "NISAR_SUBSET_V2_DELIVERY_KIND_INVALID" => "NISAR_SUBSET_V2_DELIVERY_KIND_INVALID",
            "NISAR_SUBSET_V2_PREPARED_INTENT_INVALID" => "NISAR_SUBSET_V2_PREPARED_INTENT_INVALID",
            "NISAR_SUBSET_V2_FROZEN_PLAN_INVALID" => "NISAR_SUBSET_V2_FROZEN_PLAN_INVALID",
            "NISAR_SUBSET_V2_PLANNING_WORKSPACE_MISSING" => {
                "NISAR_SUBSET_V2_PLANNING_WORKSPACE_MISSING"
            }
            "NISAR_SUBSET_V2_PLANNING_WORKSPACE_IDENTITY_MISMATCH" => {
                "NISAR_SUBSET_V2_PLANNING_WORKSPACE_IDENTITY_MISMATCH"
            }
            "NISAR_SUBSET_V2_RECOVERY_INVALID" => "NISAR_SUBSET_V2_RECOVERY_INVALID",
            "NISAR_SUBSET_V2_BOOTSTRAP_FAILED" => "NISAR_SUBSET_V2_BOOTSTRAP_FAILED",
            "NISAR_SUBSET_V2_SOURCE_UNAVAILABLE" => "NISAR_SUBSET_V2_SOURCE_UNAVAILABLE",
            "NISAR_SUBSET_V2_RANGE_UNSUPPORTED" => "NISAR_SUBSET_V2_RANGE_UNSUPPORTED",
            "NISAR_SUBSET_V2_RESPONSE_INVALID" => "NISAR_SUBSET_V2_RESPONSE_INVALID",
            "NISAR_SUBSET_V2_PARTIAL_RESPONSE" => "NISAR_SUBSET_V2_PARTIAL_RESPONSE",
            "NISAR_SUBSET_V2_TRANSPORT_FAILED" => "NISAR_SUBSET_V2_TRANSPORT_FAILED",
            "NISAR_SUBSET_V2_STRONG_VALIDATOR_REQUIRED" => {
                "NISAR_SUBSET_V2_STRONG_VALIDATOR_REQUIRED"
            }
            "NISAR_SUBSET_V2_RANGE_NOT_SATISFIABLE" => "NISAR_SUBSET_V2_RANGE_NOT_SATISFIABLE",
            "NISAR_SUBSET_V2_BOOTSTRAP_STATISTICS_INVALID" => {
                "NISAR_SUBSET_V2_BOOTSTRAP_STATISTICS_INVALID"
            }
            "NISAR_SUBSET_V2_COMBINED_BUDGET_EXCEEDED" => {
                "NISAR_SUBSET_V2_COMBINED_BUDGET_EXCEEDED"
            }
            "NISAR_SUBSET_V2_PLANNING_SCIENCE_OVERLAP" => {
                "NISAR_SUBSET_V2_PLANNING_SCIENCE_OVERLAP"
            }
            "NISAR_SUBSET_V2_INVENTORY_INVALID" => "NISAR_SUBSET_V2_INVENTORY_INVALID",
            "NISAR_SUBSET_V2_SELECTED_SCIENCE_MISSING" => {
                "NISAR_SUBSET_V2_SELECTED_SCIENCE_MISSING"
            }
            "NISAR_SUBSET_V2_PHYSICAL_MAPPING_UNAVAILABLE" => {
                "NISAR_SUBSET_V2_PHYSICAL_MAPPING_UNAVAILABLE"
            }
            "NISAR_SUBSET_V2_PHYSICAL_PLAN_INVALID" => "NISAR_SUBSET_V2_PHYSICAL_PLAN_INVALID",
            "NISAR_SUBSET_V2_VALIDATOR_CHANGED" => "NISAR_SUBSET_V2_VALIDATOR_CHANGED",
            "NISAR_SUBSET_V2_DEPENDENCY_GRAPH_INVALID" => {
                "NISAR_SUBSET_V2_DEPENDENCY_GRAPH_INVALID"
            }
            "NISAR_SUBSET_V2_PLAN_FREEZE_FAILED" => "NISAR_SUBSET_V2_PLAN_FREEZE_FAILED",
            "NISAR_SUBSET_V2_EXECUTION_STOPPED" => "NISAR_SUBSET_V2_EXECUTION_STOPPED",
            _ => return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid),
        };
        Ok(NisarSubsetRangeFailureObservationV2 {
            stage: self.stage.clone(),
            start: self.start,
            end_exclusive: self.end_exclusive,
            source_size_bytes: self.source_size_bytes,
            disposition,
            request_count: self.request_count,
            http_status: self.http_status,
            response_body_bytes: self.response_body_bytes,
            transport_diagnostic: None,
            validator_sha256: self.validator_sha256.clone(),
            failure_code,
            redirect_occurred: self.redirect_occurred,
            redirect_hop_index: self.redirect_hop_index,
            redirect_source_host_class: self.redirect_source_host_class.clone(),
            redirect_destination_host_class: self.redirect_destination_host_class.clone(),
            redirect_source_path_class: self.redirect_source_path_class.clone(),
            redirect_destination_path_class: self.redirect_destination_path_class.clone(),
            redirect_same_origin: self.redirect_same_origin,
            redirect_scheme_transition: self.redirect_scheme_transition.clone(),
            redirect_policy_result: self.redirect_policy_result.clone(),
            redirect_rejection_reason: self.redirect_rejection_reason.clone(),
        })
    }
}

#[derive(Clone, Debug)]
struct NisarSubsetRangeParseFailureV2 {
    error: NisarSubsetPlanningErrorV2,
    observation: NisarSubsetRangeFailureObservationV2,
}

#[derive(Debug)]
enum NisarSubsetRangeFetchFailureV2 {
    Planning(NisarSubsetPlanningErrorV2),
    Parse(Box<NisarSubsetRangeParseFailureV2>),
}

const NISAR_METADATA_PREFIX_MAX_ATTEMPTS_V2: u8 = 3;

fn metadata_prefix_failure_is_retryable_v2(failure: &NisarSubsetRangeParseFailureV2) -> bool {
    matches!(
        failure.error,
        NisarSubsetPlanningErrorV2::BootstrapPartialResponse
            | NisarSubsetPlanningErrorV2::BootstrapTransport
    ) || (failure.error == NisarSubsetPlanningErrorV2::BootstrapResponseInvalid
        && failure
            .observation
            .http_status
            .is_some_and(|status| (500..=599).contains(&status)))
}

fn actual_failure_charge_v2(observation: &NisarSubsetRangeFailureObservationV2) -> (u64, u64) {
    if observation.disposition == "before_send_rejected" {
        (0, 0)
    } else {
        (
            observation.request_count,
            observation.response_body_bytes.unwrap_or(0),
        )
    }
}

fn metadata_prefix_observation_retryable_v2(
    observation: &NisarSubsetRangeFailureObservationV2,
) -> bool {
    observation.failure_code == "NISAR_SUBSET_V2_PARTIAL_RESPONSE"
        || observation.failure_code == "NISAR_SUBSET_V2_TRANSPORT_FAILED"
        || (observation.failure_code == "NISAR_SUBSET_V2_RESPONSE_INVALID"
            && observation
                .http_status
                .is_some_and(|status| (500..=599).contains(&status)))
}

fn range_failure_disposition(request_count: u64) -> &'static str {
    if request_count == 0 {
        "before_send_rejected"
    } else {
        "sent_outcome_indeterminate"
    }
}

fn planning_failure_code(error: NisarSubsetPlanningErrorV2) -> &'static str {
    match error {
        NisarSubsetPlanningErrorV2::DeliveryKindInvalid => "NISAR_SUBSET_V2_DELIVERY_KIND_INVALID",
        NisarSubsetPlanningErrorV2::PreparedIntentInvalid
        | NisarSubsetPlanningErrorV2::PreparedIntentChanged => {
            "NISAR_SUBSET_V2_PREPARED_INTENT_INVALID"
        }
        NisarSubsetPlanningErrorV2::FrozenPlanMissing
        | NisarSubsetPlanningErrorV2::FrozenPlanInvalid
        | NisarSubsetPlanningErrorV2::FinalPlanInvalid => "NISAR_SUBSET_V2_FROZEN_PLAN_INVALID",
        NisarSubsetPlanningErrorV2::PlanningWorkspaceMissing => {
            "NISAR_SUBSET_V2_PLANNING_WORKSPACE_MISSING"
        }
        NisarSubsetPlanningErrorV2::PlanningWorkspaceIdentityMismatch => {
            "NISAR_SUBSET_V2_PLANNING_WORKSPACE_IDENTITY_MISMATCH"
        }
        NisarSubsetPlanningErrorV2::RecoveryInvalid => "NISAR_SUBSET_V2_RECOVERY_INVALID",
        NisarSubsetPlanningErrorV2::BootstrapFailed => "NISAR_SUBSET_V2_BOOTSTRAP_FAILED",
        NisarSubsetPlanningErrorV2::BootstrapSourceUnavailable => {
            "NISAR_SUBSET_V2_SOURCE_UNAVAILABLE"
        }
        NisarSubsetPlanningErrorV2::BootstrapRangeUnsupported => {
            "NISAR_SUBSET_V2_RANGE_UNSUPPORTED"
        }
        NisarSubsetPlanningErrorV2::BootstrapResponseInvalid => "NISAR_SUBSET_V2_RESPONSE_INVALID",
        NisarSubsetPlanningErrorV2::BootstrapPartialResponse => "NISAR_SUBSET_V2_PARTIAL_RESPONSE",
        NisarSubsetPlanningErrorV2::BootstrapTransport => "NISAR_SUBSET_V2_TRANSPORT_FAILED",
        NisarSubsetPlanningErrorV2::BootstrapStrongValidatorRequired => {
            "NISAR_SUBSET_V2_STRONG_VALIDATOR_REQUIRED"
        }
        NisarSubsetPlanningErrorV2::BootstrapRangeNotSatisfiable => {
            "NISAR_SUBSET_V2_RANGE_NOT_SATISFIABLE"
        }
        NisarSubsetPlanningErrorV2::BootstrapStatisticsInvalid => {
            "NISAR_SUBSET_V2_BOOTSTRAP_STATISTICS_INVALID"
        }
        NisarSubsetPlanningErrorV2::BootstrapBudgetExceeded
        | NisarSubsetPlanningErrorV2::CombinedBudgetExceeded => {
            "NISAR_SUBSET_V2_COMBINED_BUDGET_EXCEEDED"
        }
        NisarSubsetPlanningErrorV2::ScienceTransferObserved => {
            "NISAR_SUBSET_V2_PLANNING_SCIENCE_OVERLAP"
        }
        NisarSubsetPlanningErrorV2::InventoryBindingInvalid
        | NisarSubsetPlanningErrorV2::InventoryMismatch => "NISAR_SUBSET_V2_INVENTORY_INVALID",
        NisarSubsetPlanningErrorV2::SelectedScienceMissing => {
            "NISAR_SUBSET_V2_SELECTED_SCIENCE_MISSING"
        }
        NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable => {
            "NISAR_SUBSET_V2_PHYSICAL_MAPPING_UNAVAILABLE"
        }
        NisarSubsetPlanningErrorV2::PhysicalPlanInvalid => "NISAR_SUBSET_V2_PHYSICAL_PLAN_INVALID",
        NisarSubsetPlanningErrorV2::ValidatorChanged
        | NisarSubsetPlanningErrorV2::SourceValidatorChanged => "NISAR_SUBSET_V2_VALIDATOR_CHANGED",
        NisarSubsetPlanningErrorV2::DependencyGraphInvalid => {
            "NISAR_SUBSET_V2_DEPENDENCY_GRAPH_INVALID"
        }
        NisarSubsetPlanningErrorV2::FreezeFailed => "NISAR_SUBSET_V2_PLAN_FREEZE_FAILED",
        NisarSubsetPlanningErrorV2::ExecutionStopped => "NISAR_SUBSET_V2_EXECUTION_STOPPED",
        NisarSubsetPlanningErrorV2::OutputLimitExceeded { .. } => {
            "NISAR_SUBSET_V2_WRITER_CAPTURE_LIMIT"
        }
        NisarSubsetPlanningErrorV2::OutputDiskSpaceInsufficient { .. } => {
            "NISAR_SUBSET_V2_WRITER_DISK_SPACE_INSUFFICIENT"
        }
    }
}

fn planning_error_from_failure_code(code: &str) -> Option<NisarSubsetPlanningErrorV2> {
    Some(match code {
        "NISAR_SUBSET_V2_DELIVERY_KIND_INVALID" => NisarSubsetPlanningErrorV2::DeliveryKindInvalid,
        "NISAR_SUBSET_V2_PREPARED_INTENT_INVALID" => {
            NisarSubsetPlanningErrorV2::PreparedIntentInvalid
        }
        "NISAR_SUBSET_V2_FROZEN_PLAN_INVALID" => NisarSubsetPlanningErrorV2::FrozenPlanInvalid,
        "NISAR_SUBSET_V2_PLANNING_WORKSPACE_MISSING" => {
            NisarSubsetPlanningErrorV2::PlanningWorkspaceMissing
        }
        "NISAR_SUBSET_V2_PLANNING_WORKSPACE_IDENTITY_MISMATCH" => {
            NisarSubsetPlanningErrorV2::PlanningWorkspaceIdentityMismatch
        }
        "NISAR_SUBSET_V2_RECOVERY_INVALID" => NisarSubsetPlanningErrorV2::RecoveryInvalid,
        "NISAR_SUBSET_V2_BOOTSTRAP_FAILED" => NisarSubsetPlanningErrorV2::BootstrapFailed,
        "NISAR_SUBSET_V2_SOURCE_UNAVAILABLE" => {
            NisarSubsetPlanningErrorV2::BootstrapSourceUnavailable
        }
        "NISAR_SUBSET_V2_RANGE_UNSUPPORTED" => {
            NisarSubsetPlanningErrorV2::BootstrapRangeUnsupported
        }
        "NISAR_SUBSET_V2_RESPONSE_INVALID" => NisarSubsetPlanningErrorV2::BootstrapResponseInvalid,
        "NISAR_SUBSET_V2_PARTIAL_RESPONSE" => NisarSubsetPlanningErrorV2::BootstrapPartialResponse,
        "NISAR_SUBSET_V2_TRANSPORT_FAILED" => NisarSubsetPlanningErrorV2::BootstrapTransport,
        "NISAR_SUBSET_V2_STRONG_VALIDATOR_REQUIRED" => {
            NisarSubsetPlanningErrorV2::BootstrapStrongValidatorRequired
        }
        "NISAR_SUBSET_V2_RANGE_NOT_SATISFIABLE" => {
            NisarSubsetPlanningErrorV2::BootstrapRangeNotSatisfiable
        }
        "NISAR_SUBSET_V2_BOOTSTRAP_STATISTICS_INVALID" => {
            NisarSubsetPlanningErrorV2::BootstrapStatisticsInvalid
        }
        "NISAR_SUBSET_V2_COMBINED_BUDGET_EXCEEDED" => {
            NisarSubsetPlanningErrorV2::CombinedBudgetExceeded
        }
        "NISAR_SUBSET_V2_PLANNING_SCIENCE_OVERLAP" => {
            NisarSubsetPlanningErrorV2::ScienceTransferObserved
        }
        "NISAR_SUBSET_V2_INVENTORY_INVALID" => NisarSubsetPlanningErrorV2::InventoryBindingInvalid,
        "NISAR_SUBSET_V2_SELECTED_SCIENCE_MISSING" => {
            NisarSubsetPlanningErrorV2::SelectedScienceMissing
        }
        "NISAR_SUBSET_V2_PHYSICAL_MAPPING_UNAVAILABLE" => {
            NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable
        }
        "NISAR_SUBSET_V2_PHYSICAL_PLAN_INVALID" => NisarSubsetPlanningErrorV2::PhysicalPlanInvalid,
        "NISAR_SUBSET_V2_VALIDATOR_CHANGED" => NisarSubsetPlanningErrorV2::SourceValidatorChanged,
        "NISAR_SUBSET_V2_DEPENDENCY_GRAPH_INVALID" => {
            NisarSubsetPlanningErrorV2::DependencyGraphInvalid
        }
        "NISAR_SUBSET_V2_PLAN_FREEZE_FAILED" => NisarSubsetPlanningErrorV2::FreezeFailed,
        "NISAR_SUBSET_V2_EXECUTION_STOPPED" => NisarSubsetPlanningErrorV2::ExecutionStopped,
        _ => return None,
    })
}

fn response_validator_sha256(
    response: &reqwest::blocking::Response,
    source_size: u64,
) -> Option<String> {
    let value = response.headers().get(ETAG)?.to_str().ok()?;
    digest_serializable(&NisarSubsetSourceValidatorV1 {
        kind: NisarSubsetValidatorKind::StrongEtag,
        value: value.to_owned(),
        content_length: source_size,
    })
    .ok()
}

#[allow(clippy::too_many_arguments)]
fn range_parse_failure(
    error: NisarSubsetPlanningErrorV2,
    start: u64,
    end_exclusive: u64,
    source_size: u64,
    disposition: &'static str,
    request_count: u64,
    http_status: Option<u16>,
    response_body_bytes: Option<u64>,
    validator_sha256: Option<String>,
) -> Box<NisarSubsetRangeParseFailureV2> {
    Box::new(NisarSubsetRangeParseFailureV2 {
        error,
        observation: NisarSubsetRangeFailureObservationV2 {
            stage: String::new(),
            start,
            end_exclusive,
            source_size_bytes: source_size,
            disposition,
            request_count,
            http_status,
            response_body_bytes,
            transport_diagnostic: None,
            validator_sha256,
            failure_code: planning_failure_code(error),
            redirect_occurred: false,
            redirect_hop_index: None,
            redirect_source_host_class: None,
            redirect_destination_host_class: None,
            redirect_source_path_class: None,
            redirect_destination_path_class: None,
            redirect_same_origin: None,
            redirect_scheme_transition: None,
            redirect_policy_result: None,
            redirect_rejection_reason: None,
        },
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PostBodyEofProbeV2 {
    Eof,
    ExtraByte,
    ReadFailed(NisarSubsetIoErrorClassV2),
}

fn post_body_eof_probe(reader: &mut impl Read) -> PostBodyEofProbeV2 {
    let mut extra = [0_u8; 1];
    match reader.read(&mut extra) {
        Ok(0) => PostBodyEofProbeV2::Eof,
        Ok(_) => PostBodyEofProbeV2::ExtraByte,
        Err(error) => {
            PostBodyEofProbeV2::ReadFailed(NisarSubsetIoErrorClassV2::from_kind(error.kind()))
        }
    }
}

fn retain_post_body_eof_probe_read_diagnostic(
    failure: &mut NisarSubsetRangeParseFailureV2,
    io_error_class: NisarSubsetIoErrorClassV2,
) {
    failure.observation.transport_diagnostic =
        Some(NisarSubsetTransportDiagnosticV2::PostBodyEofProbeRead { io_error_class });
}

#[allow(clippy::too_many_arguments)]
fn parse_exact_range_response(
    outcome: crate::http_transfer::ExactRangeSendOutcome,
    start: u64,
    end_exclusive: u64,
    source_size: u64,
    expected_etag: Option<&str>,
    control: &WorkerControl,
    transfer_progress: &mut dyn FnMut(u64),
    body_activity: Option<&crate::transfer_activity::TaskBodyActivity>,
) -> Result<(Vec<u8>, String, u64), Box<NisarSubsetRangeParseFailureV2>> {
    let (mut response, request_count) = match outcome {
        crate::http_transfer::ExactRangeSendOutcome::Response {
            response,
            request_count,
        } => {
            let status = Some(response.status().as_u16());
            let validator_sha256 = response_validator_sha256(&response, source_size);
            if !is_control_active(control) {
                return Err(range_parse_failure(
                    NisarSubsetPlanningErrorV2::ExecutionStopped,
                    start,
                    end_exclusive,
                    source_size,
                    "sent_outcome_indeterminate",
                    request_count,
                    status,
                    None,
                    validator_sha256,
                ));
            }
            (response, request_count)
        }
        crate::http_transfer::ExactRangeSendOutcome::Cancelled { request_count } => {
            return Err(range_parse_failure(
                NisarSubsetPlanningErrorV2::ExecutionStopped,
                start,
                end_exclusive,
                source_size,
                range_failure_disposition(request_count),
                request_count,
                None,
                None,
                None,
            ))
        }
        crate::http_transfer::ExactRangeSendOutcome::RetryableExhausted {
            request_count,
            diagnostic,
        } => {
            let mut failure = range_parse_failure(
                NisarSubsetPlanningErrorV2::BootstrapTransport,
                start,
                end_exclusive,
                source_size,
                range_failure_disposition(request_count),
                request_count,
                diagnostic.http_status,
                None,
                None,
            );
            failure.observation.transport_diagnostic = Some(
                NisarSubsetTransportDiagnosticV2::ExactRangeSend(diagnostic.cause),
            );
            return Err(failure);
        }
        crate::http_transfer::ExactRangeSendOutcome::ProtocolRejected { request_count } => {
            return Err(range_parse_failure(
                NisarSubsetPlanningErrorV2::BootstrapResponseInvalid,
                start,
                end_exclusive,
                source_size,
                range_failure_disposition(request_count),
                request_count,
                None,
                None,
                None,
            ))
        }
        crate::http_transfer::ExactRangeSendOutcome::TrustedRedirectRejected {
            request_count,
            evidence,
        } => {
            let mut failure = range_parse_failure(
                NisarSubsetPlanningErrorV2::BootstrapResponseInvalid,
                start,
                end_exclusive,
                source_size,
                "sent_response_rejected",
                request_count,
                evidence.response_status,
                None,
                None,
            );
            failure.observation.redirect_occurred = evidence.redirect_occurred;
            failure.observation.redirect_hop_index = evidence.redirect_hop_index;
            failure.observation.redirect_source_host_class = Some(evidence.source_host_class);
            failure.observation.redirect_destination_host_class =
                Some(evidence.destination_host_class);
            failure.observation.redirect_source_path_class = Some(evidence.source_path_class);
            failure.observation.redirect_destination_path_class =
                Some(evidence.destination_path_class);
            failure.observation.redirect_same_origin = Some(evidence.same_origin);
            failure.observation.redirect_scheme_transition = Some(evidence.scheme_transition);
            failure.observation.redirect_policy_result = Some(evidence.policy_result);
            failure.observation.redirect_rejection_reason = Some(evidence.rejection_reason);
            return Err(failure);
        }
        crate::http_transfer::ExactRangeSendOutcome::BudgetExceeded { request_count } => {
            return Err(range_parse_failure(
                NisarSubsetPlanningErrorV2::BootstrapBudgetExceeded,
                start,
                end_exclusive,
                source_size,
                range_failure_disposition(request_count),
                request_count,
                None,
                None,
                None,
            ))
        }
    };
    let status = Some(response.status().as_u16());
    let validator_sha256 = response_validator_sha256(&response, source_size);
    match response.status() {
        StatusCode::NOT_FOUND | StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            return Err(range_parse_failure(
                NisarSubsetPlanningErrorV2::BootstrapSourceUnavailable,
                start,
                end_exclusive,
                source_size,
                "sent_response_rejected",
                request_count,
                status,
                None,
                validator_sha256,
            ))
        }
        StatusCode::OK => {
            return Err(range_parse_failure(
                NisarSubsetPlanningErrorV2::BootstrapRangeUnsupported,
                start,
                end_exclusive,
                source_size,
                "sent_response_rejected",
                request_count,
                status,
                None,
                validator_sha256,
            ))
        }
        StatusCode::RANGE_NOT_SATISFIABLE => {
            return Err(range_parse_failure(
                NisarSubsetPlanningErrorV2::BootstrapRangeNotSatisfiable,
                start,
                end_exclusive,
                source_size,
                "sent_response_rejected",
                request_count,
                status,
                None,
                validator_sha256,
            ))
        }
        StatusCode::PARTIAL_CONTENT => {}
        _ => {
            return Err(range_parse_failure(
                NisarSubsetPlanningErrorV2::BootstrapResponseInvalid,
                start,
                end_exclusive,
                source_size,
                "sent_response_rejected",
                request_count,
                status,
                None,
                validator_sha256,
            ))
        }
    }
    let etag = match response_strong_etag(&response) {
        Ok(etag) => etag,
        Err(_) => {
            return Err(range_parse_failure(
                NisarSubsetPlanningErrorV2::BootstrapStrongValidatorRequired,
                start,
                end_exclusive,
                source_size,
                "sent_response_rejected",
                request_count,
                status,
                None,
                validator_sha256,
            ))
        }
    };
    if expected_etag.is_some_and(|expected| expected != etag) {
        return Err(range_parse_failure(
            NisarSubsetPlanningErrorV2::SourceValidatorChanged,
            start,
            end_exclusive,
            source_size,
            "sent_response_rejected",
            request_count,
            status,
            None,
            validator_sha256,
        ));
    }
    if validate_content_range(&response, start, end_exclusive, source_size).is_err() {
        return Err(range_parse_failure(
            NisarSubsetPlanningErrorV2::BootstrapResponseInvalid,
            start,
            end_exclusive,
            source_size,
            "sent_response_rejected",
            request_count,
            status,
            None,
            validator_sha256,
        ));
    }
    let expected = match end_exclusive
        .checked_sub(start)
        .and_then(|bytes| usize::try_from(bytes).ok())
    {
        Some(expected) => expected,
        None => {
            return Err(range_parse_failure(
                NisarSubsetPlanningErrorV2::BootstrapBudgetExceeded,
                start,
                end_exclusive,
                source_size,
                "sent_response_rejected",
                request_count,
                status,
                None,
                validator_sha256,
            ))
        }
    };
    if response.content_length() != Some(expected as u64) {
        return Err(range_parse_failure(
            NisarSubsetPlanningErrorV2::BootstrapResponseInvalid,
            start,
            end_exclusive,
            source_size,
            "sent_response_rejected",
            request_count,
            status,
            None,
            validator_sha256,
        ));
    }
    let mut body = Vec::with_capacity(expected);
    let mut body_activity = body_activity.map(|activity| activity.start());
    let mut buffer = [0_u8; 64 * 1024];
    while body.len() < expected {
        if !is_control_active(control) {
            return Err(range_parse_failure(
                NisarSubsetPlanningErrorV2::ExecutionStopped,
                start,
                end_exclusive,
                source_size,
                "sent_outcome_indeterminate",
                request_count,
                status,
                Some(body.len() as u64),
                validator_sha256,
            ));
        }
        let limit = (expected - body.len()).min(buffer.len());
        let count = match response.read(&mut buffer[..limit]) {
            Ok(count) => count,
            Err(_) => {
                return Err(range_parse_failure(
                    NisarSubsetPlanningErrorV2::BootstrapPartialResponse,
                    start,
                    end_exclusive,
                    source_size,
                    "sent_response_rejected",
                    request_count,
                    status,
                    Some(body.len() as u64),
                    validator_sha256,
                ))
            }
        };
        if let Some(activity) = &mut body_activity {
            activity.observe(count);
        }
        if count == 0 {
            return Err(range_parse_failure(
                NisarSubsetPlanningErrorV2::BootstrapPartialResponse,
                start,
                end_exclusive,
                source_size,
                "sent_response_rejected",
                request_count,
                status,
                Some(body.len() as u64),
                validator_sha256,
            ));
        }
        body.extend_from_slice(&buffer[..count]);
        transfer_progress(count as u64);
    }
    let extra_count = match post_body_eof_probe(&mut response) {
        PostBodyEofProbeV2::Eof => 0,
        PostBodyEofProbeV2::ExtraByte => 1,
        PostBodyEofProbeV2::ReadFailed(io_error_class) => {
            let mut failure = range_parse_failure(
                NisarSubsetPlanningErrorV2::BootstrapTransport,
                start,
                end_exclusive,
                source_size,
                "sent_response_rejected",
                request_count,
                status,
                Some(body.len() as u64),
                validator_sha256,
            );
            retain_post_body_eof_probe_read_diagnostic(&mut failure, io_error_class);
            return Err(failure);
        }
    };
    if let Some(activity) = &mut body_activity {
        activity.observe(extra_count);
    }
    if extra_count != 0 {
        return Err(range_parse_failure(
            NisarSubsetPlanningErrorV2::BootstrapResponseInvalid,
            start,
            end_exclusive,
            source_size,
            "sent_response_rejected",
            request_count,
            status,
            Some(body.len() as u64 + extra_count as u64),
            validator_sha256,
        ));
    }
    if !is_control_active(control) {
        return Err(range_parse_failure(
            NisarSubsetPlanningErrorV2::ExecutionStopped,
            start,
            end_exclusive,
            source_size,
            "sent_outcome_indeterminate",
            request_count,
            status,
            Some(body.len() as u64),
            validator_sha256,
        ));
    }
    Ok((body, etag, request_count))
}

pub(crate) struct NisarSubsetHttpMetadataBootstrapV2 {
    http: Arc<HttpTransfer>,
    unbound_asset: Mutex<Option<ResolvedAsset>>,
    bound_asset: OnceLock<Arc<ResolvedAsset>>,
    workspace: Hdf5RangeWorkspaceV1,
    range_failure_observations: Mutex<Vec<NisarSubsetRangeFailureObservationV2>>,
    body_activity: Option<crate::transfer_activity::TaskBodyActivity>,
    /// The wait before an automatic retry of a planning Range (preflight since
    /// downloader enhancement D1c; metadata prefix, metadata discovery and
    /// additional geolocation since D1d): the bounded science backoff.
    planning_retry_backoff: Arc<crate::nisar_science_transfer_v2::ScienceRetryBackoff>,
}

impl sealed::Sealed for NisarSubsetHttpMetadataBootstrapV2 {}

impl NisarSubsetHttpMetadataBootstrapV2 {
    pub(crate) fn new(
        http: Arc<HttpTransfer>,
        asset: ResolvedAsset,
        workspace: Hdf5RangeWorkspaceV1,
    ) -> Result<Self, NisarSubsetPlanningErrorV2> {
        if asset.method() != ResolvedMethod::Get
            || !matches!(asset.effective_url().scheme(), "http" | "https")
            || asset.expected_size().is_none_or(|size| size == 0)
        {
            return Err(NisarSubsetPlanningErrorV2::BootstrapFailed);
        }
        Ok(Self {
            http,
            unbound_asset: Mutex::new(Some(asset)),
            bound_asset: OnceLock::new(),
            workspace,
            range_failure_observations: Mutex::new(Vec::new()),
            body_activity: None,
            planning_retry_backoff: Arc::new(
                crate::nisar_science_transfer_v2::science_retry_backoff,
            ),
        })
    }

    pub(crate) fn science_workspace_v2(&self) -> &Hdf5RangeWorkspaceV1 {
        &self.workspace
    }

    /// Replaces the 5 s / 20 s / 30 s planning retry backoff in tests.
    #[cfg(test)]
    pub(crate) fn with_planning_retry_backoff_for_test(
        mut self,
        backoff: Arc<crate::nisar_science_transfer_v2::ScienceRetryBackoff>,
    ) -> Self {
        self.planning_retry_backoff = backoff;
        self
    }

    /// Waits the bounded planning backoff before retry number `retry`; true
    /// when a pause or cancellation ended the wait.
    fn wait_before_planning_retry(
        &self,
        retry: u32,
        http_status: Option<u16>,
        control: &WorkerControl,
    ) -> bool {
        let backoff = (self.planning_retry_backoff)(retry, http_status);
        crate::nisar_science_transfer_v2::wait_for_science_retry(backoff, control)
    }

    pub(crate) fn with_body_activity(
        mut self,
        activity: crate::transfer_activity::TaskBodyActivity,
    ) -> Self {
        self.body_activity = Some(activity);
        self
    }

    fn record_range_failure_observation(
        &self,
        stage: &str,
        observation: &NisarSubsetRangeFailureObservationV2,
    ) {
        let mut observation = observation.clone();
        observation.stage = stage.to_owned();
        if let Ok(mut values) = self.range_failure_observations.lock() {
            values.push(observation);
        }
    }

    pub(crate) fn take_range_failure_observations(
        &self,
    ) -> Vec<NisarSubsetRangeFailureObservationV2> {
        self.range_failure_observations
            .lock()
            .map(|mut values| std::mem::take(&mut *values))
            .unwrap_or_default()
    }

    pub(crate) fn resolved_asset_v2(
        &self,
    ) -> Result<Arc<ResolvedAsset>, NisarSubsetPlanningErrorV2> {
        self.bound_asset
            .get()
            .cloned()
            .ok_or(NisarSubsetPlanningErrorV2::BootstrapStrongValidatorRequired)
    }

    fn expected_source_size(&self) -> Result<u64, NisarSubsetPlanningErrorV2> {
        if let Some(asset) = self.bound_asset.get() {
            return asset
                .expected_size()
                .filter(|size| *size > 0)
                .ok_or(NisarSubsetPlanningErrorV2::BootstrapResponseInvalid);
        }
        self.unbound_asset
            .lock()
            .map_err(|_| NisarSubsetPlanningErrorV2::BootstrapFailed)?
            .as_ref()
            .and_then(ResolvedAsset::expected_size)
            .filter(|size| *size > 0)
            .ok_or(NisarSubsetPlanningErrorV2::BootstrapResponseInvalid)
    }

    fn bind_source_validator(
        &self,
        validator: &NisarSubsetSourceValidatorV1,
    ) -> Result<Arc<ResolvedAsset>, NisarSubsetPlanningErrorV2> {
        if let Some(asset) = self.bound_asset.get() {
            let current = source_validator_from_asset(asset)?;
            return (current == *validator)
                .then(|| Arc::clone(asset))
                .ok_or(NisarSubsetPlanningErrorV2::SourceValidatorChanged);
        }
        let mut unbound = self
            .unbound_asset
            .lock()
            .map_err(|_| NisarSubsetPlanningErrorV2::BootstrapFailed)?;
        let asset = unbound
            .take()
            .ok_or(NisarSubsetPlanningErrorV2::BootstrapFailed)?;
        if asset.expected_size() != Some(validator.content_length) {
            *unbound = Some(asset);
            return Err(NisarSubsetPlanningErrorV2::BootstrapResponseInvalid);
        }
        let asset = Arc::new(asset.with_remote_validator(RemoteValidator {
            etag: Some(validator.value.clone()),
            last_modified: None,
            content_length: Some(validator.content_length),
        }));
        self.bound_asset
            .set(Arc::clone(&asset))
            .map_err(|_| NisarSubsetPlanningErrorV2::BootstrapFailed)?;
        Ok(asset)
    }

    fn source_validator(&self) -> Result<NisarSubsetSourceValidatorV1, NisarSubsetPlanningErrorV2> {
        let asset = self.resolved_asset_v2()?;
        source_validator_from_asset(&asset)
    }

    fn preflight_and_bind(
        &self,
        max_request_count: u64,
        control: &WorkerControl,
        transfer_progress: &mut dyn FnMut(u64),
    ) -> Result<(NisarSubsetSourceValidatorV1, BootstrapLedger), NisarSubsetPlanningErrorV2> {
        ensure_active(control)?;
        let source_size = self.expected_source_size()?;
        let mut ledger =
            BootstrapLedger::new_preflight_with_request_limit(source_size, max_request_count)?;
        // Downloader enhancement D1c (L4 S-band regression 2026-10-18): a
        // transient preflight failure is retried like a science Range, at
        // most three attempts, after the bounded science backoff (5 s, then
        // 20 s; an HTTP 429 at least 30 s). Each failed attempt is charged to
        // the planning budget and recorded as a failure observation.
        let mut attempt = 0_u32;
        let (reservation, (body, etag, request_count)) = loop {
            attempt += 1;
            let reservation = ledger.reserve(BootstrapCategory::Preflight, 0, 1, 1)?;
            let outcome = {
                let guard = self
                    .unbound_asset
                    .lock()
                    .map_err(|_| NisarSubsetPlanningErrorV2::BootstrapFailed)?;
                let asset = guard
                    .as_ref()
                    .ok_or(NisarSubsetPlanningErrorV2::BootstrapFailed)?;
                self.http
                    .send_resolved_preflight_range_once_trusted_redirect(asset, 0, 1, &|| {
                        control.is_cancelled() || control.is_paused()
                    })
            };
            match parse_exact_range_response(
                outcome,
                0,
                1,
                source_size,
                None,
                control,
                transfer_progress,
                self.body_activity.as_ref(),
            ) {
                Ok(result) => break (reservation, result),
                Err(failure) => {
                    self.record_range_failure_observation(
                        BootstrapCategory::Preflight.evidence_stage(),
                        &failure.observation,
                    );
                    ledger.fail(reservation, &failure.observation)?;
                    if attempt < crate::nisar_science_transfer_v2::SCIENCE_RANGE_MAX_ATTEMPTS
                        && metadata_prefix_failure_is_retryable_v2(&failure)
                    {
                        if self.wait_before_planning_retry(
                            attempt,
                            failure.observation.http_status,
                            control,
                        ) {
                            return Err(NisarSubsetPlanningErrorV2::ExecutionStopped);
                        }
                        continue;
                    }
                    return Err(failure.error);
                }
            }
        };
        let validator = NisarSubsetSourceValidatorV1 {
            kind: NisarSubsetValidatorKind::StrongEtag,
            value: etag,
            content_length: source_size,
        };
        ledger.bind_source_validator(digest_serializable(&validator)?)?;
        ledger.commit_with_request_count(reservation, body.len() as u64, request_count)?;
        self.bind_source_validator(&validator)?;
        ensure_active(control)?;
        Ok((validator, ledger))
    }

    fn fetch_exact_range(
        &self,
        stage: &str,
        start: u64,
        end_exclusive: u64,
        source_validator: &NisarSubsetSourceValidatorV1,
        control: &WorkerControl,
        transfer_progress: &mut dyn FnMut(u64),
    ) -> Result<(Vec<u8>, u64), NisarSubsetRangeFetchFailureV2> {
        let asset = self
            .resolved_asset_v2()
            .map_err(NisarSubsetRangeFetchFailureV2::Planning)?;
        let outcome = self.http.send_resolved_exact_range_once_trusted_redirect(
            &asset,
            start,
            end_exclusive,
            &source_validator.value,
            &|| control.is_cancelled() || control.is_paused(),
        );
        let (body, _, request_count) = match parse_exact_range_response(
            outcome,
            start,
            end_exclusive,
            source_validator.content_length,
            Some(&source_validator.value),
            control,
            transfer_progress,
            self.body_activity.as_ref(),
        ) {
            Ok(result) => result,
            Err(failure) => {
                self.record_range_failure_observation(stage, &failure.observation);
                return Err(NisarSubsetRangeFetchFailureV2::Parse(failure));
            }
        };
        Ok((body, request_count))
    }

    fn fetch_reserved_range(
        &self,
        ledger: &mut BootstrapLedger,
        target: &mut File,
        reserved_range: (BootstrapCategory, (u64, u64)),
        source_validator: &NisarSubsetSourceValidatorV1,
        control: &WorkerControl,
        transfer_progress: &mut dyn FnMut(u64),
    ) -> Result<(), NisarSubsetPlanningErrorV2> {
        let (category, (start, end_exclusive)) = reserved_range;
        let expected = end_exclusive
            .checked_sub(start)
            .ok_or(NisarSubsetPlanningErrorV2::BootstrapStatisticsInvalid)?;
        let reservation = ledger.reserve(category, start, end_exclusive, expected)?;
        // Downloader enhancement D1d (L5 recheck 2026-10-19): one transient
        // failure of a metadata-discovery Range (transport_cause=request_other,
        // request_count=1) failed the S-band task after 195 MB. A transient
        // failure of a metadata-discovery or additional-geolocation Range is
        // retried like a preflight or science Range: at most three attempts,
        // after the bounded backoff (5 s, then 20 s; an HTTP 429 at least
        // 30 s). Every failed attempt stays charged and observed.
        let mut attempt = 0_u32;
        let (body, request_count) = loop {
            attempt += 1;
            if attempt > 1 {
                ledger.check_attempt_budget(
                    &reservation,
                    u64::from(crate::http_transfer::MAX_REDIRECT_HOPS) + 1,
                    expected,
                )?;
            }
            match self.fetch_exact_range(
                category.evidence_stage(),
                start,
                end_exclusive,
                source_validator,
                control,
                transfer_progress,
            ) {
                Ok(result) => break result,
                Err(NisarSubsetRangeFetchFailureV2::Parse(failure)) => {
                    if attempt < crate::nisar_science_transfer_v2::SCIENCE_RANGE_MAX_ATTEMPTS
                        && metadata_prefix_failure_is_retryable_v2(&failure)
                    {
                        let (charged_requests, charged_response_body_bytes) =
                            actual_failure_charge_v2(&failure.observation);
                        if charged_requests != 0 {
                            ledger.charge_retryable_attempt(
                                &reservation,
                                charged_requests,
                                charged_response_body_bytes,
                            )?;
                        } else if charged_response_body_bytes != 0 {
                            return Err(NisarSubsetPlanningErrorV2::BootstrapStatisticsInvalid);
                        }
                        if self.wait_before_planning_retry(
                            attempt,
                            failure.observation.http_status,
                            control,
                        ) {
                            ledger.cancel(reservation)?;
                            return Err(NisarSubsetPlanningErrorV2::ExecutionStopped);
                        }
                        continue;
                    }
                    ledger.fail(reservation, &failure.observation)?;
                    return Err(failure.error);
                }
                Err(NisarSubsetRangeFetchFailureV2::Planning(error)) => {
                    ledger.cancel(reservation)?;
                    return Err(error);
                }
            }
        };
        if let Err(error) = target
            .seek(SeekFrom::Start(start))
            .and_then(|_| target.write_all(&body))
        {
            ledger.cancel(reservation)?;
            let _ = error;
            return Err(NisarSubsetPlanningErrorV2::BootstrapFailed);
        }
        ledger.commit_with_request_count(reservation, body.len() as u64, request_count)
    }
}

impl NisarSubsetMetadataBootstrapV2 for NisarSubsetHttpMetadataBootstrapV2 {
    fn recover_prepared_plan(
        &self,
        work: &DownloadWork,
        prepared_intent: &NisarSubsetPreparedIntentV2,
        control: &WorkerControl,
    ) -> Result<Option<NisarSubsetPlanV2>, NisarSubsetPlanningErrorV2> {
        recover_prepared_checkpoint_plan_v2(self, &self.workspace, work, prepared_intent, control)
    }

    fn repair_missing_planning_workspace(
        &self,
        work: &DownloadWork,
        plan: &NisarSubsetPlanV2,
        control: &WorkerControl,
        transfer_progress: &mut dyn FnMut(u64),
    ) -> Result<(), NisarSubsetPlanningErrorV2> {
        // Old versions could freeze in Store before physical mapping/checkpointing.
        // Re-read only metadata using the task's own verified prefix. The bootstrap
        // below requires an existing prefix for frozen work and cannot truncate it.
        let prepared = read_prepared_intent(work)?
            .intent()
            .map_err(|_| NisarSubsetPlanningErrorV2::PreparedIntentInvalid)?;
        let mut output =
            self.bootstrap_with_progress(work, &prepared, control, transfer_progress)?;
        validate_inventory_against_frozen_plan_v2(&output.inventory, plan)?;
        let reservation = output
            .ledger
            .reserve(BootstrapCategory::Revalidation, 0, 1, 1)?;
        let revalidated = self.revalidate_before_freeze(
            work,
            &prepared.canonical_intent_sha256,
            output.inventory.evidence().source_validator_sha256(),
            control,
        )?;
        if revalidated.source_validator_sha256
            != output.inventory.evidence().source_validator_sha256()
            || revalidated.prepared_intent_sha256 != prepared.canonical_intent_sha256
        {
            return Err(NisarSubsetPlanningErrorV2::SourceValidatorChanged);
        }
        output.ledger.commit_with_request_count(
            reservation,
            revalidated.actual_response_body_bytes,
            revalidated.request_count,
        )?;
        self.build_and_checkpoint_physical_plan(
            work,
            plan,
            &output.inventory,
            &output.ledger.statistics()?,
            control,
        )?;
        Ok(())
    }

    fn bootstrap(
        &self,
        work: &DownloadWork,
        prepared_intent: &NisarSubsetPreparedIntentV2,
        control: &WorkerControl,
    ) -> Result<NisarSubsetMetadataBootstrapOutputV2, NisarSubsetPlanningErrorV2> {
        self.bootstrap_with_progress(work, prepared_intent, control, &mut |_| {})
    }

    fn bootstrap_with_progress(
        &self,
        work: &DownloadWork,
        prepared_intent: &NisarSubsetPreparedIntentV2,
        control: &WorkerControl,
        transfer_progress: &mut dyn FnMut(u64),
    ) -> Result<NisarSubsetMetadataBootstrapOutputV2, NisarSubsetPlanningErrorV2> {
        ensure_active(control)?;
        let science_root = science_root_for_identity(&prepared_intent.source_product_identity)?;
        let (source_validator, mut ledger) = self.preflight_and_bind(
            planning_request_limit_v2(science_root),
            control,
            transfer_progress,
        )?;
        let asset = self.resolved_asset_v2()?;
        if asset
            .suggested_filename()
            .is_some_and(|file_name| file_name != prepared_intent.source_asset_identity.file_name)
            || work
                .item
                .asset_access
                .as_ref()
                .and_then(|access| access.expected_size_bytes)
                .is_some_and(|size| size != source_validator.content_length)
        {
            return Err(NisarSubsetPlanningErrorV2::InventoryBindingInvalid);
        }
        let initial_material_ranges =
            remote_planning_initial_material_ranges(source_validator.content_length)?;
        let mut materialized_ranges = canonical_material_ranges_v2(
            initial_material_ranges.clone(),
            source_validator.content_length,
        )?;
        let source_validator_sha256 = digest_serializable(&source_validator)?;
        let source_identity_sha256 = prefix_source_identity_sha256(prepared_intent)?;
        let frozen_work = parse_nisar_subset_plan_v2_envelope(&work.item.metadata)
            .map_err(|_| NisarSubsetPlanningErrorV2::FrozenPlanInvalid)?
            .is_some();
        #[cfg(test)]
        let prefix_ranges = validate_deferred_fixture_materialization_plan(
            source_validator.content_length,
            &initial_material_ranges,
        )?;
        #[cfg(not(test))]
        let prefix_ranges = validate_prefix_materialization_plan(
            source_validator.content_length,
            &initial_material_ranges,
        )?;
        let mut prefix_session = NisarPrefixMaterializationSessionV2::open_or_initialize_validated(
            &self.workspace,
            source_validator.content_length,
            &source_validator_sha256,
            prefix_ranges,
            &source_identity_sha256,
            cfg!(test),
            !frozen_work,
        )?;
        let persisted_failure_observations = prefix_session.persisted_failure_observations()?;
        for observation in persisted_failure_observations {
            let (failed_requests, failed_response_body_bytes) =
                actual_failure_charge_v2(&observation);
            ledger.charge_persisted_failure(failed_requests, failed_response_body_bytes)?;
            self.record_range_failure_observation(&observation.stage, &observation);
        }
        for &(start, end_exclusive) in prefix_session.verified_ranges() {
            let expected = end_exclusive
                .checked_sub(start)
                .ok_or(NisarSubsetPlanningErrorV2::RecoveryInvalid)?;
            let request_count =
                prefix_session.request_count_for_verified_range((start, end_exclusive))?;
            let reservation = ledger.reserve(
                BootstrapCategory::MetadataPrefix,
                start,
                end_exclusive,
                expected,
            )?;
            ledger.commit_with_request_count(reservation, expected, request_count)?;
        }
        for (start, end_exclusive) in prefix_session.incomplete_ranges() {
            let expected = end_exclusive
                .checked_sub(start)
                .ok_or(NisarSubsetPlanningErrorV2::BootstrapStatisticsInvalid)?;
            let reservation = ledger.reserve(
                BootstrapCategory::MetadataPrefix,
                start,
                end_exclusive,
                expected,
            )?;
            let mut accepted = None;
            let consumed_attempts = prefix_session.attempts_for_range((start, end_exclusive))?;
            if consumed_attempts >= NISAR_METADATA_PREFIX_MAX_ATTEMPTS_V2 {
                return Err(NisarSubsetPlanningErrorV2::BootstrapTransport);
            }
            for attempt in (consumed_attempts + 1)..=NISAR_METADATA_PREFIX_MAX_ATTEMPTS_V2 {
                // A single logical attempt may follow the bounded redirect chain.
                // Reserve its worst-case request count and full accepted body before
                // any network I/O; actual counts are committed or charged below.
                ledger.check_attempt_budget(
                    &reservation,
                    u64::from(crate::http_transfer::MAX_REDIRECT_HOPS) + 1,
                    expected,
                )?;
                match self.fetch_exact_range(
                    BootstrapCategory::MetadataPrefix.evidence_stage(),
                    start,
                    end_exclusive,
                    &source_validator,
                    control,
                    transfer_progress,
                ) {
                    Ok(result) => {
                        accepted = Some(result);
                        break;
                    }
                    Err(NisarSubsetRangeFetchFailureV2::Parse(failure)) => {
                        let retryable = metadata_prefix_failure_is_retryable_v2(&failure);
                        let (charged_requests, charged_response_body_bytes) =
                            actual_failure_charge_v2(&failure.observation);
                        prefix_session.record_failed_attempt(&failure.observation, retryable)?;
                        if retryable && attempt < NISAR_METADATA_PREFIX_MAX_ATTEMPTS_V2 {
                            if charged_requests != 0 {
                                ledger.charge_retryable_attempt(
                                    &reservation,
                                    charged_requests,
                                    charged_response_body_bytes,
                                )?;
                            } else if charged_response_body_bytes != 0 {
                                return Err(NisarSubsetPlanningErrorV2::BootstrapStatisticsInvalid);
                            }
                            // Downloader enhancement D1d (D1c report risk 5): the
                            // retry waits the bounded planning backoff instead of
                            // following at once; a stop during the wait pauses.
                            if self.wait_before_planning_retry(
                                u32::from(attempt),
                                failure.observation.http_status,
                                control,
                            ) {
                                ledger.cancel(reservation)?;
                                return Err(NisarSubsetPlanningErrorV2::ExecutionStopped);
                            }
                            continue;
                        }
                        ledger.fail_with_charge(
                            reservation,
                            charged_requests,
                            charged_response_body_bytes,
                        )?;
                        return Err(failure.error);
                    }
                    Err(NisarSubsetRangeFetchFailureV2::Planning(error)) => {
                        ledger.cancel(reservation)?;
                        return Err(error);
                    }
                }
            }
            let (body, request_count) =
                accepted.ok_or(NisarSubsetPlanningErrorV2::BootstrapFailed)?;
            if let Err(error) = prefix_session.materialize_verified_range_with_request_count(
                (start, end_exclusive),
                &body,
                request_count,
            ) {
                ledger.cancel(reservation)?;
                return Err(error);
            }
            ledger.commit_with_request_count(reservation, body.len() as u64, request_count)?;
        }
        let target = prefix_session.into_file();
        target
            .sync_all()
            .map_err(|_| NisarSubsetPlanningErrorV2::BootstrapFailed)?;
        // Windows does not allow HDF5 to open the sparse part while this
        // bootstrap writer still owns a live handle to the same file.
        drop(target);
        if science_root == NisarScienceRoot::Ssar {
            let mut discovery_response_bytes = 0_u64;
            let mut converged = false;
            for _pass in 0..NISAR_SSAR_METADATA_DISCOVERY_MAX_PASSES_V2 {
                ensure_active(control)?;
                let missing = discover_ssar_v1_metadata_ranges_v2(
                    &self.workspace.part_path(),
                    source_validator.content_length,
                    &materialized_ranges,
                )?;
                if missing.is_empty() {
                    converged = true;
                    break;
                }
                let pass_bytes = missing.iter().try_fold(0_u64, |total, (start, end)| {
                    total
                        .checked_add(end - start)
                        .ok_or(NisarSubsetPlanningErrorV2::BootstrapBudgetExceeded)
                })?;
                discovery_response_bytes = discovery_response_bytes
                    .checked_add(pass_bytes)
                    .ok_or(NisarSubsetPlanningErrorV2::BootstrapBudgetExceeded)?;
                if discovery_response_bytes > NISAR_SSAR_METADATA_DISCOVERY_MAX_RESPONSE_BYTES_V2 {
                    return Err(NisarSubsetPlanningErrorV2::BootstrapBudgetExceeded);
                }
                let mut target = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(self.workspace.part_path())
                    .map_err(|_| NisarSubsetPlanningErrorV2::BootstrapFailed)?;
                for &(start, end) in &missing {
                    self.fetch_reserved_range(
                        &mut ledger,
                        &mut target,
                        (BootstrapCategory::MetadataDiscovery, (start, end)),
                        &source_validator,
                        control,
                        transfer_progress,
                    )?;
                    materialized_ranges.push((start, end));
                }
                target
                    .sync_all()
                    .map_err(|_| NisarSubsetPlanningErrorV2::BootstrapFailed)?;
                drop(target);
                materialized_ranges = canonical_material_ranges_v2(
                    materialized_ranges,
                    source_validator.content_length,
                )?;
            }
            if !converged {
                return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
            }
        }
        let selected_paths = selected_science_paths_from_selection(
            science_root,
            &prepared_intent.selected_frequency,
            &prepared_intent.selected_polarizations,
        );
        let catalog = inspect_nisar_planning_physical_ranges_v2(
            &self.workspace.part_path(),
            &selected_paths,
            science_root,
        )
        .map_err(|_| NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
        ledger.bind_selected_science_ranges(catalog.selected_science_ranges.clone())?;
        // HDF5 may place a science chunk inside the mandatory metadata prefix.
        // Those bytes are fetched once as part of the prefix; no logical science
        // Range has been issued. The ledger excludes this prefix overlap from
        // science-transfer accounting while preserving fail-closed checks for
        // every later metadata/geolocation Range.
        let mut target = OpenOptions::new()
            .read(true)
            .write(true)
            .open(self.workspace.part_path())
            .map_err(|_| NisarSubsetPlanningErrorV2::BootstrapFailed)?;
        let mut required = Vec::new();
        for &(start, end) in &catalog.metadata_ranges {
            for gap in missing_material_ranges_v2(
                (start, end),
                &materialized_ranges,
                source_validator.content_length,
            )? {
                required.push((gap, BootstrapCategory::MetadataDiscovery));
            }
        }
        for &(start, end) in &catalog.geolocation_ranges {
            for gap in missing_material_ranges_v2(
                (start, end),
                &materialized_ranges,
                source_validator.content_length,
            )? {
                required.push((gap, BootstrapCategory::AdditionalGeolocation));
            }
        }
        required.sort_unstable_by_key(|(range, _)| *range);
        let mut canonical_required = Vec::<((u64, u64), BootstrapCategory)>::new();
        for ((start, end), category) in required {
            if let Some(((_, previous_end), previous_category)) = canonical_required.last_mut() {
                if start <= *previous_end {
                    *previous_end = (*previous_end).max(end);
                    if matches!(category, BootstrapCategory::AdditionalGeolocation) {
                        *previous_category = BootstrapCategory::AdditionalGeolocation;
                    }
                    continue;
                }
            }
            canonical_required.push(((start, end), category));
        }
        for ((start, end), category) in canonical_required {
            if catalog
                .all_science_ranges
                .iter()
                .any(|science| tuple_ranges_overlap((start, end), *science))
            {
                return Err(NisarSubsetPlanningErrorV2::ScienceTransferObserved);
            }
            self.fetch_reserved_range(
                &mut ledger,
                &mut target,
                (category, (start, end)),
                &source_validator,
                control,
                transfer_progress,
            )?;
        }
        target
            .sync_all()
            .map_err(|_| NisarSubsetPlanningErrorV2::BootstrapFailed)?;
        drop(target);
        let inventory = NisarSubsetVerifiedMetadataInventoryV2::from_verified_live_metadata_hdf5(
            prepared_intent,
            source_validator,
            &self.workspace.part_path(),
            &|| control.is_cancelled() || control.is_paused(),
        )
        .map_err(|error| {
            map_live_inventory_error(error, NisarSubsetPlanningErrorV2::InventoryMismatch)
        })?;
        Ok(NisarSubsetMetadataBootstrapOutputV2 { inventory, ledger })
    }

    fn revalidate_before_freeze(
        &self,
        _work: &DownloadWork,
        startup_prepared_intent_sha256: &str,
        _startup_source_validator_sha256: &str,
        control: &WorkerControl,
    ) -> Result<NisarSubsetBootstrapRevalidationV2, NisarSubsetPlanningErrorV2> {
        let validator = self.source_validator()?;
        let (body, request_count) = match self.fetch_exact_range(
            "planning_revalidation",
            0,
            1,
            &validator,
            control,
            &mut |_| {},
        ) {
            Ok(result) => result,
            Err(NisarSubsetRangeFetchFailureV2::Planning(error)) => return Err(error),
            Err(NisarSubsetRangeFetchFailureV2::Parse(failure)) => return Err(failure.error),
        };
        Ok(NisarSubsetBootstrapRevalidationV2 {
            prepared_intent_sha256: startup_prepared_intent_sha256.to_owned(),
            source_validator_sha256: digest_serializable(&validator)?,
            actual_response_body_bytes: body.len() as u64,
            request_count,
        })
    }

    fn build_and_checkpoint_physical_plan(
        &self,
        work: &DownloadWork,
        frozen_plan: &NisarSubsetPlanV2,
        inventory: &NisarSubsetVerifiedMetadataInventoryV2,
        statistics: &NisarSubsetBootstrapStatisticsV2,
        control: &WorkerControl,
    ) -> Result<NisarSubsetPhysicalExecutionPlanV2, NisarSubsetPlanningErrorV2> {
        ensure_active(control)?;
        let science_root = science_root_for_identity(&frozen_plan.source_product_identity)?;
        statistics.validate_for_science_root(science_root)?;
        let material_ranges = planning_material_ranges_from_statistics_v2(statistics)?;
        let basic_part_evidence = self
            .workspace
            .inspect_planning_part_evidence_v2(inventory.evidence().source_size(), &[], &|| {
                control.is_cancelled() || control.is_paused()
            })
            .map_err(map_workspace_load_error)?
            .ok_or(NisarSubsetPlanningErrorV2::PlanningWorkspaceMissing)?;
        let physical_plan = build_physical_execution_plan_v2(
            frozen_plan,
            inventory,
            statistics,
            &self.workspace.part_path(),
            Some(NisarSubsetCombinedBudgetPolicyV2::PhysicalExecutionUnionV2),
            &|| control.is_cancelled() || control.is_paused(),
        )?;
        physical_plan.validate_binding(frozen_plan)?;
        let part_evidence = self
            .workspace
            .inspect_planning_part_evidence_v2(
                inventory.evidence().source_size(),
                &material_ranges,
                &|| control.is_cancelled() || control.is_paused(),
            )
            .map_err(map_workspace_load_error)?
            .ok_or(NisarSubsetPlanningErrorV2::PlanningWorkspaceMissing)?;
        if part_evidence.identity_sha256 != basic_part_evidence.identity_sha256 {
            return Err(NisarSubsetPlanningErrorV2::PlanningWorkspaceIdentityMismatch);
        }
        let (workspace_canonical_path, workspace_identity) = self
            .workspace
            .bound_workspace_identity_v2()
            .map_err(|_| NisarSubsetPlanningErrorV2::PlanningWorkspaceIdentityMismatch)?;
        let mut checkpoint = NisarSubsetPlanningWorkspaceCheckpointV2 {
            checkpoint_contract: NISAR_SUBSET_PLANNING_WORKSPACE_CHECKPOINT_V2_CONTRACT.to_owned(),
            schema_version: NISAR_SUBSET_PLANNING_WORKSPACE_CHECKPOINT_V2_SCHEMA_VERSION,
            workspace_canonical_path,
            workspace_identity,
            task_id: work.task.task_id.clone(),
            item_id: work.item.item_id.clone(),
            prepared_intent_sha256: frozen_plan.prepared_intent_sha256.clone(),
            final_plan_sha256: frozen_plan.canonical_semantic_plan_digest.clone(),
            inventory_sha256: inventory.evidence().inventory_sha256().to_owned(),
            inventory_evidence_sha256: inventory.evidence().evidence_sha256().to_owned(),
            source_validator_sha256: inventory.evidence().source_validator_sha256().to_owned(),
            source_size: inventory.evidence().source_size(),
            sparse_part_identity_sha256: part_evidence.identity_sha256,
            sparse_part_material_ranges_sha256: planning_material_ranges_sha256(&material_ranges)?,
            sparse_part_material_sha256: part_evidence.material_sha256,
            bootstrap_statistics: statistics.clone(),
            physical_plan,
            checkpoint_sha256: String::new(),
        };
        checkpoint.checkpoint_sha256 = checkpoint.recompute_sha256()?;
        let canonical = serde_json::to_string(&checkpoint)
            .map_err(|_| NisarSubsetPlanningErrorV2::RecoveryInvalid)?;
        if let Some(existing) = self
            .workspace
            .load_planning_checkpoint_v2()
            .map_err(map_workspace_load_error)?
        {
            if parse_planning_workspace_checkpoint_v2(&existing)? != checkpoint {
                return Err(NisarSubsetPlanningErrorV2::PlanningWorkspaceIdentityMismatch);
            }
        } else {
            self.workspace
                .persist_planning_checkpoint_v2(&canonical)
                .map_err(|_| NisarSubsetPlanningErrorV2::RecoveryInvalid)?;
        }
        ensure_active(control)?;
        Ok(checkpoint.physical_plan)
    }

    fn reopen_planning_workspace(
        &self,
        work: &DownloadWork,
        frozen_plan: &NisarSubsetPlanV2,
        control: &WorkerControl,
    ) -> Result<
        (
            NisarSubsetPhysicalExecutionPlanV2,
            NisarSubsetBootstrapStatisticsV2,
            NisarSubsetVerifiedMetadataInventoryV2,
        ),
        NisarSubsetPlanningErrorV2,
    > {
        ensure_active(control)?;
        let text = self
            .workspace
            .load_planning_checkpoint_v2()
            .map_err(map_workspace_load_error)?
            .ok_or(NisarSubsetPlanningErrorV2::PlanningWorkspaceMissing)?;
        let checkpoint = parse_planning_workspace_checkpoint_v2(&text)?;
        let (workspace_canonical_path, workspace_identity) = self
            .workspace
            .bound_workspace_identity_v2()
            .map_err(|_| NisarSubsetPlanningErrorV2::PlanningWorkspaceIdentityMismatch)?;
        if checkpoint.workspace_canonical_path != workspace_canonical_path
            || checkpoint.workspace_identity != workspace_identity
            || checkpoint.task_id != work.task.task_id
            || checkpoint.item_id != work.item.item_id
            || checkpoint.prepared_intent_sha256 != frozen_plan.prepared_intent_sha256
            || checkpoint.final_plan_sha256 != frozen_plan.canonical_semantic_plan_digest
        {
            return Err(NisarSubsetPlanningErrorV2::PlanningWorkspaceIdentityMismatch);
        }
        self.bind_source_validator(&checkpoint.physical_plan.source_validator)?;
        let validator = self.source_validator()?;
        let current_validator_sha256 = digest_serializable(&validator)?;
        if checkpoint.source_validator_sha256 != current_validator_sha256
            || checkpoint.physical_plan.source_validator_sha256 != current_validator_sha256
            || checkpoint.source_size != validator.content_length
        {
            return Err(NisarSubsetPlanningErrorV2::ValidatorChanged);
        }
        checkpoint.bootstrap_statistics.validate_checkpoint()?;
        let prepared_intent = read_prepared_intent(work)?
            .intent()
            .map_err(|_| NisarSubsetPlanningErrorV2::PreparedIntentInvalid)?;
        checkpoint
            .bootstrap_statistics
            .validate_for_science_root(science_root_for_identity(
                &frozen_plan.source_product_identity,
            )?)?;
        let recovered_inventory =
            NisarSubsetVerifiedMetadataInventoryV2::from_verified_live_metadata_hdf5(
                &prepared_intent,
                validator,
                &self.workspace.part_path(),
                &|| control.is_cancelled() || control.is_paused(),
            )
            .map_err(|error| {
                map_live_inventory_error(error, NisarSubsetPlanningErrorV2::RecoveryInvalid)
            })?;
        validate_inventory_against_frozen_plan_v2(&recovered_inventory, frozen_plan)?;
        if recovered_inventory.evidence().inventory_sha256() != checkpoint.inventory_sha256
            || recovered_inventory.evidence().evidence_sha256()
                != checkpoint.inventory_evidence_sha256
        {
            return Err(NisarSubsetPlanningErrorV2::InventoryMismatch);
        }
        let material_ranges =
            planning_material_ranges_from_statistics_v2(&checkpoint.bootstrap_statistics)?;
        if checkpoint.sparse_part_material_ranges_sha256
            != planning_material_ranges_sha256(&material_ranges)?
        {
            return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
        }
        let part_evidence = self
            .workspace
            .inspect_planning_part_evidence_v2(checkpoint.source_size, &material_ranges, &|| {
                control.is_cancelled() || control.is_paused()
            })
            .map_err(map_workspace_load_error)?
            .ok_or(NisarSubsetPlanningErrorV2::PlanningWorkspaceMissing)?;
        if part_evidence.identity_sha256 != checkpoint.sparse_part_identity_sha256 {
            return Err(NisarSubsetPlanningErrorV2::PlanningWorkspaceIdentityMismatch);
        }
        if part_evidence.material_sha256 != checkpoint.sparse_part_material_sha256 {
            return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
        }
        let expected_physical_plan = build_physical_execution_plan_v2(
            frozen_plan,
            &recovered_inventory,
            &checkpoint.bootstrap_statistics,
            &self.workspace.part_path(),
            checkpoint.physical_plan.combined_budget.budget_policy,
            &|| control.is_cancelled() || control.is_paused(),
        )?;
        expected_physical_plan.validate_binding(frozen_plan)?;
        if checkpoint.physical_plan != expected_physical_plan {
            return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
        }
        ensure_active(control)?;
        Ok((
            checkpoint.physical_plan,
            checkpoint.bootstrap_statistics.mark_frozen_plan_fast_path(),
            recovered_inventory,
        ))
    }
}

fn default_prefix_request_count() -> u64 {
    1
}

fn is_default_prefix_request_count(request_count: &u64) -> bool {
    *request_count == 1
}

fn prefix_source_identity_sha256(
    prepared_intent: &NisarSubsetPreparedIntentV2,
) -> Result<String, NisarSubsetPlanningErrorV2> {
    #[derive(Serialize)]
    struct FrozenSourceIdentity<'a> {
        source_product_identity: &'a NisarSubsetSourceProductIdentityV2,
        source_asset_identity: &'a NisarSubsetSourceAssetIdentityV2,
    }

    digest_serializable(&FrozenSourceIdentity {
        source_product_identity: &prepared_intent.source_product_identity,
        source_asset_identity: &prepared_intent.source_asset_identity,
    })
}

const NISAR_PREFIX_MATERIALIZATION_CHECKPOINT_V2_CONTRACT: &str =
    "NisarPrefixMaterializationCheckpointV2";
const NISAR_PREFIX_MATERIALIZATION_CHECKPOINT_V2_SCHEMA_VERSION: u32 = 2;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct NisarPrefixCompletedRangeV2 {
    start: u64,
    end_exclusive: u64,
    response_bytes: u64,
    #[serde(
        default = "default_prefix_request_count",
        skip_serializing_if = "is_default_prefix_request_count"
    )]
    request_count: u64,
    source_size: u64,
    source_validator_sha256: String,
    payload_sha256: String,
    materialized: bool,
    verified: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct NisarPrefixRangeAttemptStateV2 {
    start: u64,
    end_exclusive: u64,
    #[serde(default)]
    attempts: u64,
    #[serde(default)]
    retryable: bool,
    #[serde(default)]
    failed_request_count: u64,
    #[serde(default)]
    failed_response_body_bytes: u64,
    #[serde(default)]
    failure_observations: Vec<NisarPrefixDurableFailureObservationV2>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct NisarPrefixMaterializationCheckpointV2 {
    checkpoint_contract: String,
    schema_version: u32,
    source_size: u64,
    source_validator_sha256: String,
    source_identity_sha256: String,
    planned_ranges: Vec<NisarSubsetBootstrapRangeV2>,
    completed_ranges: Vec<NisarPrefixCompletedRangeV2>,
    #[serde(default)]
    historical_failure_observations: Vec<NisarPrefixDurableFailureObservationV2>,
    #[serde(default)]
    attempt_states: Vec<NisarPrefixRangeAttemptStateV2>,
    checkpoint_sha256: String,
}

impl NisarPrefixMaterializationCheckpointV2 {
    fn recompute_sha256(&self) -> Result<String, NisarSubsetPlanningErrorV2> {
        #[derive(Serialize)]
        struct Identity<'a> {
            checkpoint_contract: &'a str,
            schema_version: u32,
            source_size: u64,
            source_validator_sha256: &'a str,
            source_identity_sha256: &'a str,
            planned_ranges: &'a [NisarSubsetBootstrapRangeV2],
            completed_ranges: &'a [NisarPrefixCompletedRangeV2],
            historical_failure_observations: &'a [NisarPrefixDurableFailureObservationV2],
            attempt_states: &'a [NisarPrefixRangeAttemptStateV2],
        }
        digest_serializable(&Identity {
            checkpoint_contract: &self.checkpoint_contract,
            schema_version: self.schema_version,
            source_size: self.source_size,
            source_validator_sha256: &self.source_validator_sha256,
            source_identity_sha256: &self.source_identity_sha256,
            planned_ranges: &self.planned_ranges,
            completed_ranges: &self.completed_ranges,
            historical_failure_observations: &self.historical_failure_observations,
            attempt_states: &self.attempt_states,
        })
    }

    fn recompute_sha256_without_attempt_states(
        &self,
    ) -> Result<String, NisarSubsetPlanningErrorV2> {
        #[derive(Serialize)]
        struct LegacyIdentity<'a> {
            checkpoint_contract: &'a str,
            schema_version: u32,
            source_size: u64,
            source_validator_sha256: &'a str,
            source_identity_sha256: &'a str,
            planned_ranges: &'a [NisarSubsetBootstrapRangeV2],
            completed_ranges: &'a [NisarPrefixCompletedRangeV2],
        }
        digest_serializable(&LegacyIdentity {
            checkpoint_contract: &self.checkpoint_contract,
            schema_version: self.schema_version,
            source_size: self.source_size,
            source_validator_sha256: &self.source_validator_sha256,
            source_identity_sha256: &self.source_identity_sha256,
            planned_ranges: &self.planned_ranges,
            completed_ranges: &self.completed_ranges,
        })
    }

    /// Digest format used by checkpoints written after retry attempts were
    /// introduced but before completed ranges retained their failure history.
    fn recompute_sha256_without_historical_failure_observations(
        &self,
    ) -> Result<String, NisarSubsetPlanningErrorV2> {
        #[derive(Serialize)]
        struct AttemptIdentity<'a> {
            checkpoint_contract: &'a str,
            schema_version: u32,
            source_size: u64,
            source_validator_sha256: &'a str,
            source_identity_sha256: &'a str,
            planned_ranges: &'a [NisarSubsetBootstrapRangeV2],
            completed_ranges: &'a [NisarPrefixCompletedRangeV2],
            attempt_states: &'a [NisarPrefixRangeAttemptStateV2],
        }
        digest_serializable(&AttemptIdentity {
            checkpoint_contract: &self.checkpoint_contract,
            schema_version: self.schema_version,
            source_size: self.source_size,
            source_validator_sha256: &self.source_validator_sha256,
            source_identity_sha256: &self.source_identity_sha256,
            planned_ranges: &self.planned_ranges,
            completed_ranges: &self.completed_ranges,
            attempt_states: &self.attempt_states,
        })
    }

    fn canonical_json(&self) -> Result<String, NisarSubsetPlanningErrorV2> {
        serde_json::to_string(self).map_err(|_| NisarSubsetPlanningErrorV2::RecoveryInvalid)
    }

    fn canonical_json_without_attempt_states(&self) -> Result<String, NisarSubsetPlanningErrorV2> {
        #[derive(Serialize)]
        struct LegacyCheckpoint<'a> {
            checkpoint_contract: &'a str,
            schema_version: u32,
            source_size: u64,
            source_validator_sha256: &'a str,
            source_identity_sha256: &'a str,
            planned_ranges: &'a [NisarSubsetBootstrapRangeV2],
            completed_ranges: &'a [NisarPrefixCompletedRangeV2],
            checkpoint_sha256: &'a str,
        }
        serde_json::to_string(&LegacyCheckpoint {
            checkpoint_contract: &self.checkpoint_contract,
            schema_version: self.schema_version,
            source_size: self.source_size,
            source_validator_sha256: &self.source_validator_sha256,
            source_identity_sha256: &self.source_identity_sha256,
            planned_ranges: &self.planned_ranges,
            completed_ranges: &self.completed_ranges,
            checkpoint_sha256: &self.checkpoint_sha256,
        })
        .map_err(|_| NisarSubsetPlanningErrorV2::RecoveryInvalid)
    }

    fn canonical_json_without_historical_failure_observations(
        &self,
    ) -> Result<String, NisarSubsetPlanningErrorV2> {
        #[derive(Serialize)]
        struct AttemptCheckpoint<'a> {
            checkpoint_contract: &'a str,
            schema_version: u32,
            source_size: u64,
            source_validator_sha256: &'a str,
            source_identity_sha256: &'a str,
            planned_ranges: &'a [NisarSubsetBootstrapRangeV2],
            completed_ranges: &'a [NisarPrefixCompletedRangeV2],
            attempt_states: &'a [NisarPrefixRangeAttemptStateV2],
            checkpoint_sha256: &'a str,
        }
        serde_json::to_string(&AttemptCheckpoint {
            checkpoint_contract: &self.checkpoint_contract,
            schema_version: self.schema_version,
            source_size: self.source_size,
            source_validator_sha256: &self.source_validator_sha256,
            source_identity_sha256: &self.source_identity_sha256,
            planned_ranges: &self.planned_ranges,
            completed_ranges: &self.completed_ranges,
            attempt_states: &self.attempt_states,
            checkpoint_sha256: &self.checkpoint_sha256,
        })
        .map_err(|_| NisarSubsetPlanningErrorV2::RecoveryInvalid)
    }
}

struct NisarPrefixMaterializationSessionV2 {
    workspace: Hdf5RangeWorkspaceV1,
    part: File,
    checkpoint: NisarPrefixMaterializationCheckpointV2,
    verified_ranges: Vec<(u64, u64)>,
}

impl NisarPrefixMaterializationSessionV2 {
    fn open_or_initialize(
        workspace: &Hdf5RangeWorkspaceV1,
        source_size: u64,
        source_validator_sha256: &str,
        planned_ranges: &[(u64, u64)],
        source_identity_sha256: &str,
    ) -> Result<Self, NisarSubsetPlanningErrorV2> {
        let planned_ranges = validate_prefix_materialization_plan(source_size, planned_ranges)?;
        Self::open_or_initialize_validated(
            workspace,
            source_size,
            source_validator_sha256,
            planned_ranges,
            source_identity_sha256,
            false,
            true,
        )
    }

    #[cfg(test)]
    fn open_or_initialize_deferred_fixture(
        workspace: &Hdf5RangeWorkspaceV1,
        source_size: u64,
        source_validator_sha256: &str,
        planned_ranges: &[(u64, u64)],
        source_identity_sha256: &str,
    ) -> Result<Self, NisarSubsetPlanningErrorV2> {
        let planned_ranges =
            validate_deferred_fixture_materialization_plan(source_size, planned_ranges)?;
        Self::open_or_initialize_validated(
            workspace,
            source_size,
            source_validator_sha256,
            planned_ranges,
            source_identity_sha256,
            true,
            true,
        )
    }

    fn open_or_initialize_validated(
        workspace: &Hdf5RangeWorkspaceV1,
        source_size: u64,
        source_validator_sha256: &str,
        planned_ranges: Vec<NisarSubsetBootstrapRangeV2>,
        source_identity_sha256: &str,
        allow_deferred_test_fixture: bool,
        allow_initialize: bool,
    ) -> Result<Self, NisarSubsetPlanningErrorV2> {
        if !valid_sha256(source_validator_sha256) || !valid_sha256(source_identity_sha256) {
            return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
        }
        let stored = workspace
            .load_planning_prefix_checkpoint_v2()
            .map_err(map_workspace_load_error)?;
        let (part, checkpoint) = if let Some(canonical) = stored {
            let checkpoint =
                parse_prefix_materialization_checkpoint(&canonical, allow_deferred_test_fixture)?;
            if checkpoint.source_size != source_size
                || checkpoint.source_validator_sha256 != source_validator_sha256
            {
                return Err(NisarSubsetPlanningErrorV2::SourceValidatorChanged);
            }
            if checkpoint.source_identity_sha256 != source_identity_sha256
                || checkpoint.planned_ranges != planned_ranges
            {
                return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
            }
            let mut part = workspace
                .reopen_planning_prefix_part_v2(source_size)
                .map_err(map_workspace_load_error)?;
            verify_prefix_completed_ranges(&checkpoint, &mut part)?;
            (part, checkpoint)
        } else {
            if !allow_initialize {
                return Err(NisarSubsetPlanningErrorV2::PlanningWorkspaceMissing);
            }
            let part = workspace
                .reset_planning_part_v2(source_size)
                .map_err(|_| NisarSubsetPlanningErrorV2::BootstrapFailed)?;
            let mut checkpoint = NisarPrefixMaterializationCheckpointV2 {
                checkpoint_contract: NISAR_PREFIX_MATERIALIZATION_CHECKPOINT_V2_CONTRACT.to_owned(),
                schema_version: NISAR_PREFIX_MATERIALIZATION_CHECKPOINT_V2_SCHEMA_VERSION,
                source_size,
                source_validator_sha256: source_validator_sha256.to_owned(),
                source_identity_sha256: source_identity_sha256.to_owned(),
                planned_ranges,
                completed_ranges: Vec::new(),
                historical_failure_observations: Vec::new(),
                attempt_states: Vec::new(),
                checkpoint_sha256: String::new(),
            };
            checkpoint.checkpoint_sha256 = checkpoint.recompute_sha256()?;
            workspace
                .persist_planning_prefix_checkpoint_v2(&checkpoint.canonical_json()?)
                .map_err(map_workspace_load_error)?;
            (part, checkpoint)
        };
        let verified_ranges = checkpoint
            .completed_ranges
            .iter()
            .map(|range| (range.start, range.end_exclusive))
            .collect();
        Ok(Self {
            workspace: workspace.clone(),
            part,
            checkpoint,
            verified_ranges,
        })
    }

    fn verified_ranges(&self) -> &[(u64, u64)] {
        &self.verified_ranges
    }

    fn request_count_for_verified_range(
        &self,
        range: (u64, u64),
    ) -> Result<u64, NisarSubsetPlanningErrorV2> {
        self.checkpoint
            .completed_ranges
            .iter()
            .find(|completed| (completed.start, completed.end_exclusive) == range)
            .map(|completed| completed.request_count)
            .filter(|request_count| *request_count > 0)
            .ok_or(NisarSubsetPlanningErrorV2::RecoveryInvalid)
    }

    fn incomplete_ranges(&self) -> Vec<(u64, u64)> {
        self.checkpoint
            .planned_ranges
            .iter()
            .map(|range| (range.start, range.end_exclusive))
            .filter(|range| !self.verified_ranges.contains(range))
            .collect()
    }

    fn attempts_for_range(&self, range: (u64, u64)) -> Result<u8, NisarSubsetPlanningErrorV2> {
        if !self
            .checkpoint
            .planned_ranges
            .iter()
            .any(|planned| (planned.start, planned.end_exclusive) == range)
            || self.verified_ranges.contains(&range)
        {
            return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
        }
        let Some(state) = self
            .checkpoint
            .attempt_states
            .iter()
            .find(|state| (state.start, state.end_exclusive) == range)
        else {
            return Ok(0);
        };
        let network_failures = prefix_network_failure_count(state)?;
        if !state.retryable
            && !state
                .failure_observations
                .last()
                .is_some_and(prefix_observation_is_control_stop)
        {
            return Err(planning_error_from_failure_code(
                state
                    .failure_observations
                    .last()
                    .map(|observation| observation.failure_code.as_str())
                    .unwrap_or_default(),
            )
            .unwrap_or(NisarSubsetPlanningErrorV2::RecoveryInvalid));
        }
        Ok(network_failures)
    }

    fn persisted_failure_observations(
        &self,
    ) -> Result<Vec<NisarSubsetRangeFailureObservationV2>, NisarSubsetPlanningErrorV2> {
        let mut observations = self
            .checkpoint
            .historical_failure_observations
            .iter()
            .map(NisarPrefixDurableFailureObservationV2::to_runtime)
            .collect::<Result<Vec<_>, _>>()?;
        observations.extend(
            self.checkpoint
                .attempt_states
                .iter()
                .flat_map(|state| state.failure_observations.iter())
                .map(NisarPrefixDurableFailureObservationV2::to_runtime)
                .collect::<Result<Vec<_>, _>>()?,
        );
        Ok(observations)
    }

    fn record_failed_attempt(
        &mut self,
        observation: &NisarSubsetRangeFailureObservationV2,
        retryable: bool,
    ) -> Result<(), NisarSubsetPlanningErrorV2> {
        let mut observation = observation.clone();
        if observation.stage.is_empty() {
            observation.stage = BootstrapCategory::MetadataPrefix
                .evidence_stage()
                .to_owned();
        }
        let range = (observation.start, observation.end_exclusive);
        let (failed_request_count, failed_response_body_bytes) =
            actual_failure_charge_v2(&observation);
        if !self
            .checkpoint
            .planned_ranges
            .iter()
            .any(|planned| (planned.start, planned.end_exclusive) == range)
            || self.verified_ranges.contains(&range)
            || (failed_request_count == 0 && failed_response_body_bytes != 0)
        {
            return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
        }
        let state = if let Some(state) = self
            .checkpoint
            .attempt_states
            .iter_mut()
            .find(|state| (state.start, state.end_exclusive) == range)
        {
            state
        } else {
            self.checkpoint
                .attempt_states
                .push(NisarPrefixRangeAttemptStateV2 {
                    start: range.0,
                    end_exclusive: range.1,
                    attempts: 0,
                    retryable,
                    failed_request_count: 0,
                    failed_response_body_bytes: 0,
                    failure_observations: Vec::new(),
                });
            self.checkpoint
                .attempt_states
                .last_mut()
                .ok_or(NisarSubsetPlanningErrorV2::RecoveryInvalid)?
        };
        let network_failures = prefix_network_failure_count(state)?;
        let durable_observation =
            NisarPrefixDurableFailureObservationV2::from_runtime(&observation);
        if retryable != metadata_prefix_observation_retryable_v2(&observation)
            || state
                .failure_observations
                .last()
                .is_some_and(|last| !state.retryable && !prefix_observation_is_control_stop(last))
            || (!prefix_observation_is_control_stop(&durable_observation)
                && network_failures >= NISAR_METADATA_PREFIX_MAX_ATTEMPTS_V2)
        {
            return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
        }
        let next_attempts = state
            .attempts
            .checked_add(1)
            .ok_or(NisarSubsetPlanningErrorV2::RecoveryInvalid)?;
        let next_failed_request_count = state
            .failed_request_count
            .checked_add(failed_request_count)
            .ok_or(NisarSubsetPlanningErrorV2::RecoveryInvalid)?;
        let next_failed_response_body_bytes = state
            .failed_response_body_bytes
            .checked_add(failed_response_body_bytes)
            .ok_or(NisarSubsetPlanningErrorV2::RecoveryInvalid)?;
        state.attempts = next_attempts;
        state.retryable = retryable;
        state.failed_request_count = next_failed_request_count;
        state.failed_response_body_bytes = next_failed_response_body_bytes;
        state.failure_observations.push(durable_observation);
        self.checkpoint
            .attempt_states
            .sort_unstable_by_key(|state| (state.start, state.end_exclusive));
        self.checkpoint.checkpoint_sha256 = self.checkpoint.recompute_sha256()?;
        self.workspace
            .persist_planning_prefix_checkpoint_v2(&self.checkpoint.canonical_json()?)
            .map_err(map_workspace_load_error)
    }

    fn materialize_verified_range(
        &mut self,
        range: (u64, u64),
        body: &[u8],
    ) -> Result<(), NisarSubsetPlanningErrorV2> {
        self.materialize_verified_range_with_request_count(range, body, 1)
    }

    fn materialize_verified_range_with_request_count(
        &mut self,
        range: (u64, u64),
        body: &[u8],
        request_count: u64,
    ) -> Result<(), NisarSubsetPlanningErrorV2> {
        if request_count == 0 {
            return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
        }
        let expected = range
            .1
            .checked_sub(range.0)
            .filter(|length| *length > 0)
            .ok_or(NisarSubsetPlanningErrorV2::RecoveryInvalid)?;
        if body.len() as u64 != expected {
            return Err(NisarSubsetPlanningErrorV2::BootstrapPartialResponse);
        }
        if !self
            .checkpoint
            .planned_ranges
            .iter()
            .any(|planned| (planned.start, planned.end_exclusive) == range)
            || self.verified_ranges.contains(&range)
        {
            return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
        }
        self.part
            .seek(SeekFrom::Start(range.0))
            .and_then(|_| self.part.write_all(body))
            .and_then(|_| self.part.sync_all())
            .map_err(|_| NisarSubsetPlanningErrorV2::BootstrapFailed)?;
        let payload_sha256 = prefix_file_range_sha256(&mut self.part, range)?;
        if payload_sha256 != format!("{:x}", Sha256::digest(body)) {
            return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
        }
        self.checkpoint
            .completed_ranges
            .push(NisarPrefixCompletedRangeV2 {
                start: range.0,
                end_exclusive: range.1,
                response_bytes: expected,
                request_count,
                source_size: self.checkpoint.source_size,
                source_validator_sha256: self.checkpoint.source_validator_sha256.clone(),
                payload_sha256,
                materialized: true,
                verified: true,
            });
        self.checkpoint
            .completed_ranges
            .sort_unstable_by_key(|completed| (completed.start, completed.end_exclusive));
        let historical_failure_observations = self
            .checkpoint
            .attempt_states
            .iter()
            .find(|state| (state.start, state.end_exclusive) == range)
            .map(|state| state.failure_observations.clone())
            .unwrap_or_default();
        self.checkpoint
            .historical_failure_observations
            .extend(historical_failure_observations);
        self.checkpoint
            .attempt_states
            .retain(|state| (state.start, state.end_exclusive) != range);
        self.checkpoint.checkpoint_sha256 = self.checkpoint.recompute_sha256()?;
        self.workspace
            .persist_planning_prefix_checkpoint_v2(&self.checkpoint.canonical_json()?)
            .map_err(map_workspace_load_error)?;
        self.verified_ranges = self
            .checkpoint
            .completed_ranges
            .iter()
            .map(|completed| (completed.start, completed.end_exclusive))
            .collect();
        Ok(())
    }

    fn final_prefix_sha256(&mut self) -> Result<String, NisarSubsetPlanningErrorV2> {
        if !self.incomplete_ranges().is_empty()
            || self
                .checkpoint
                .planned_ranges
                .first()
                .map(|range| range.start)
                != Some(0)
            || self
                .checkpoint
                .planned_ranges
                .windows(2)
                .any(|pair| pair[0].end_exclusive != pair[1].start)
        {
            return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
        }
        let end_exclusive = self
            .checkpoint
            .planned_ranges
            .last()
            .map(|range| range.end_exclusive)
            .ok_or(NisarSubsetPlanningErrorV2::RecoveryInvalid)?;
        prefix_file_range_sha256(&mut self.part, (0, end_exclusive))
    }

    fn into_file(self) -> File {
        self.part
    }
}

fn validate_prefix_materialization_plan(
    source_size: u64,
    ranges: &[(u64, u64)],
) -> Result<Vec<NisarSubsetBootstrapRangeV2>, NisarSubsetPlanningErrorV2> {
    if source_size == 0
        || ranges.is_empty()
        || ranges[0].0 != 0
        || ranges
            .iter()
            .any(|&(start, end)| start >= end || end > source_size)
        || ranges.windows(2).any(|pair| pair[0].1 != pair[1].0)
    {
        return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
    }
    ranges
        .iter()
        .map(|&(start, end)| NisarSubsetBootstrapRangeV2::new(start, end))
        .collect()
}

#[cfg(test)]
fn validate_deferred_fixture_materialization_plan(
    source_size: u64,
    ranges: &[(u64, u64)],
) -> Result<Vec<NisarSubsetBootstrapRangeV2>, NisarSubsetPlanningErrorV2> {
    let mut expected = NISAR_DEFERRED_FIXTURE_STRUCTURAL_RANGES_V2.to_vec();
    expected.push((NISAR_DEFERRED_FIXTURE_SCIENCE_PAYLOAD_END_V2, source_size));
    if source_size <= NISAR_DEFERRED_FIXTURE_SCIENCE_PAYLOAD_END_V2 || ranges != expected {
        return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
    }
    ranges
        .iter()
        .map(|&(start, end)| NisarSubsetBootstrapRangeV2::new(start, end))
        .collect()
}

fn validate_prefix_durable_failure_observation(
    checkpoint: &NisarPrefixMaterializationCheckpointV2,
    durable: &NisarPrefixDurableFailureObservationV2,
    expected_range: Option<(u64, u64)>,
) -> Result<NisarSubsetRangeFailureObservationV2, NisarSubsetPlanningErrorV2> {
    let range = (durable.start, durable.end_exclusive);
    let range_len = durable
        .end_exclusive
        .checked_sub(durable.start)
        .ok_or(NisarSubsetPlanningErrorV2::RecoveryInvalid)?;
    if durable.stage != BootstrapCategory::MetadataPrefix.evidence_stage()
        || expected_range.is_some_and(|expected| expected != range)
        || !checkpoint
            .planned_ranges
            .iter()
            .any(|planned| (planned.start, planned.end_exclusive) == range)
        || durable.source_size_bytes != checkpoint.source_size
        || durable
            .validator_sha256
            .as_deref()
            .is_some_and(|digest| !valid_sha256(digest))
    {
        return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
    }
    let observation = durable.to_runtime()?;
    let before_send = observation.disposition == "before_send_rejected";
    if before_send {
        if observation.request_count != 0
            || observation
                .response_body_bytes
                .is_some_and(|bytes| bytes != 0)
        {
            return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
        }
    } else if observation.request_count == 0 {
        return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
    }
    if observation
        .response_body_bytes
        .is_some_and(|bytes| bytes > range_len)
    {
        return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
    }
    Ok(observation)
}

fn prefix_observation_is_control_stop(
    observation: &NisarPrefixDurableFailureObservationV2,
) -> bool {
    observation.failure_code == planning_failure_code(NisarSubsetPlanningErrorV2::ExecutionStopped)
}

// A user stop ends the current execution; it is not an automatic retry.
// Retain every observation and its network charge, but allow a later explicit
// execution to continue without spending the network-failure retry allowance.
fn prefix_network_failure_count(
    state: &NisarPrefixRangeAttemptStateV2,
) -> Result<u8, NisarSubsetPlanningErrorV2> {
    let mut count = 0_u8;
    for (index, durable) in state.failure_observations.iter().enumerate() {
        let retryable = metadata_prefix_observation_retryable_v2(&durable.to_runtime()?);
        let stopped = prefix_observation_is_control_stop(durable);
        let last = index + 1 == state.failure_observations.len();
        if (last && state.retryable != retryable) || (!last && !retryable && !stopped) {
            return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
        }
        if !stopped {
            count = count
                .checked_add(1)
                .filter(|count| *count <= NISAR_METADATA_PREFIX_MAX_ATTEMPTS_V2)
                .ok_or(NisarSubsetPlanningErrorV2::RecoveryInvalid)?;
        }
    }
    Ok(count)
}

fn validate_prefix_attempt_states(
    checkpoint: &NisarPrefixMaterializationCheckpointV2,
) -> Result<(), NisarSubsetPlanningErrorV2> {
    if checkpoint
        .attempt_states
        .windows(2)
        .any(|pair| pair[0].start >= pair[1].start)
    {
        return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
    }
    for state in &checkpoint.attempt_states {
        let range = (state.start, state.end_exclusive);
        if state.attempts == 0
            || (state.failed_request_count == 0 && state.failed_response_body_bytes != 0)
            || !checkpoint
                .planned_ranges
                .iter()
                .any(|planned| (planned.start, planned.end_exclusive) == range)
            || checkpoint
                .completed_ranges
                .iter()
                .any(|completed| (completed.start, completed.end_exclusive) == range)
            || state.attempts != state.failure_observations.len() as u64
        {
            return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
        }
        prefix_network_failure_count(state)?;
        let (failed_requests, failed_response_body_bytes) = state
            .failure_observations
            .iter()
            .try_fold((0_u64, 0_u64), |(requests, bytes), durable| {
                let observation =
                    validate_prefix_durable_failure_observation(checkpoint, durable, Some(range))?;
                let (failed_requests, failed_response_body_bytes) =
                    actual_failure_charge_v2(&observation);
                Ok((
                    requests
                        .checked_add(failed_requests)
                        .ok_or(NisarSubsetPlanningErrorV2::RecoveryInvalid)?,
                    bytes
                        .checked_add(failed_response_body_bytes)
                        .ok_or(NisarSubsetPlanningErrorV2::RecoveryInvalid)?,
                ))
            })?;
        if failed_requests != state.failed_request_count
            || failed_response_body_bytes != state.failed_response_body_bytes
        {
            return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
        }
    }
    Ok(())
}

fn parse_prefix_materialization_checkpoint(
    canonical: &str,
    allow_deferred_test_fixture: bool,
) -> Result<NisarPrefixMaterializationCheckpointV2, NisarSubsetPlanningErrorV2> {
    let raw = serde_json::from_str::<serde_json::Value>(canonical)
        .map_err(|_| NisarSubsetPlanningErrorV2::RecoveryInvalid)?;
    let (has_historical_failure_observations, has_attempt_states) = raw
        .as_object()
        .map(|object| {
            (
                object.contains_key("historical_failure_observations"),
                object.contains_key("attempt_states"),
            )
        })
        .unwrap_or((false, false));
    let checkpoint = serde_json::from_value::<NisarPrefixMaterializationCheckpointV2>(raw)
        .map_err(|_| NisarSubsetPlanningErrorV2::RecoveryInvalid)?;
    let ranges = checkpoint
        .planned_ranges
        .iter()
        .map(|range| (range.start, range.end_exclusive))
        .collect::<Vec<_>>();
    let validated_ranges = if allow_deferred_test_fixture {
        #[cfg(test)]
        {
            validate_deferred_fixture_materialization_plan(checkpoint.source_size, &ranges)?
        }
        #[cfg(not(test))]
        {
            return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
        }
    } else {
        validate_prefix_materialization_plan(checkpoint.source_size, &ranges)?
    };
    if checkpoint.checkpoint_contract != NISAR_PREFIX_MATERIALIZATION_CHECKPOINT_V2_CONTRACT
        || checkpoint.schema_version != NISAR_PREFIX_MATERIALIZATION_CHECKPOINT_V2_SCHEMA_VERSION
        || !valid_sha256(&checkpoint.source_validator_sha256)
        || !valid_sha256(&checkpoint.source_identity_sha256)
        || !valid_sha256(&checkpoint.checkpoint_sha256)
        || validated_ranges != checkpoint.planned_ranges
    {
        return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
    }
    let mut historical_attempt_counts = BTreeMap::<(u64, u64), u8>::new();
    for durable in &checkpoint.historical_failure_observations {
        let observation = validate_prefix_durable_failure_observation(&checkpoint, durable, None)?;
        let range = (observation.start, observation.end_exclusive);
        let count = historical_attempt_counts.entry(range).or_default();
        if !prefix_observation_is_control_stop(durable) {
            *count = count
                .checked_add(1)
                .ok_or(NisarSubsetPlanningErrorV2::RecoveryInvalid)?;
        }
        // Keep the three-network-failure bound, including failures followed by
        // success. Explicit control stops retain charges but do not spend it.
        if *count > NISAR_METADATA_PREFIX_MAX_ATTEMPTS_V2 {
            return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
        }
        // An observation is moved to history only when this range has been
        // materialized successfully. A history entry for an incomplete range
        // would allow an attacker or a torn write to manufacture budget.
        if !checkpoint
            .completed_ranges
            .iter()
            .any(|completed| (completed.start, completed.end_exclusive) == range)
            || (!metadata_prefix_observation_retryable_v2(&observation)
                && !prefix_observation_is_control_stop(durable))
        {
            return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
        }
        let _ = actual_failure_charge_v2(&observation);
    }
    if has_historical_failure_observations {
        if !has_attempt_states
            || checkpoint.recompute_sha256()? != checkpoint.checkpoint_sha256
            || checkpoint.canonical_json()? != canonical
        {
            return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
        }
        validate_prefix_attempt_states(&checkpoint)?;
    } else if has_attempt_states {
        // Compatibility for checkpoints written by the previous schema: they
        // have durable attempt states but no completed-range failure history.
        if checkpoint.recompute_sha256_without_historical_failure_observations()?
            != checkpoint.checkpoint_sha256
            || checkpoint.canonical_json_without_historical_failure_observations()? != canonical
        {
            return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
        }
        validate_prefix_attempt_states(&checkpoint)?;
    } else {
        // The oldest checkpoint format has no durable attempt history. It is
        // safe to resume only after all ranges were already materialized;
        // otherwise unknown prior attempts could exceed the retry bound.
        if checkpoint.completed_ranges.len() != checkpoint.planned_ranges.len()
            || checkpoint.recompute_sha256_without_attempt_states()? != checkpoint.checkpoint_sha256
            || checkpoint.canonical_json_without_attempt_states()? != canonical
        {
            return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
        }
    }
    Ok(checkpoint)
}

fn verify_prefix_completed_ranges(
    checkpoint: &NisarPrefixMaterializationCheckpointV2,
    part: &mut File,
) -> Result<(), NisarSubsetPlanningErrorV2> {
    if checkpoint
        .completed_ranges
        .windows(2)
        .any(|pair| pair[0].start >= pair[1].start)
    {
        return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
    }
    for completed in &checkpoint.completed_ranges {
        let range = (completed.start, completed.end_exclusive);
        if completed.response_bytes != completed.end_exclusive.saturating_sub(completed.start)
            || completed.request_count == 0
            || completed.source_size != checkpoint.source_size
            || completed.source_validator_sha256 != checkpoint.source_validator_sha256
            || !completed.materialized
            || !completed.verified
            || !valid_sha256(&completed.payload_sha256)
            || !checkpoint
                .planned_ranges
                .iter()
                .any(|planned| (planned.start, planned.end_exclusive) == range)
            || prefix_file_range_sha256(part, range)? != completed.payload_sha256
        {
            return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
        }
    }
    Ok(())
}

fn prefix_file_range_sha256(
    part: &mut File,
    range: (u64, u64),
) -> Result<String, NisarSubsetPlanningErrorV2> {
    let mut remaining = range
        .1
        .checked_sub(range.0)
        .filter(|length| *length > 0)
        .ok_or(NisarSubsetPlanningErrorV2::RecoveryInvalid)?;
    part.seek(SeekFrom::Start(range.0))
        .map_err(|_| NisarSubsetPlanningErrorV2::RecoveryInvalid)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    while remaining > 0 {
        let count = usize::try_from(remaining.min(buffer.len() as u64))
            .map_err(|_| NisarSubsetPlanningErrorV2::RecoveryInvalid)?;
        part.read_exact(&mut buffer[..count])
            .map_err(|_| NisarSubsetPlanningErrorV2::RecoveryInvalid)?;
        digest.update(&buffer[..count]);
        remaining -= count as u64;
    }
    Ok(format!("{:x}", digest.finalize()))
}

const NISAR_SUBSET_METADATA_PREFIX_SUBRANGE_BYTES_V2: u64 = 16 * 1024 * 1024;
const NISAR_SSAR_METADATA_DISCOVERY_MAX_RESPONSE_BYTES_V2: u64 = 16 * 1024 * 1024;
const NISAR_SSAR_METADATA_DISCOVERY_MAX_RANGES_V2: usize = 128;
const NISAR_SSAR_METADATA_DISCOVERY_MAX_PASSES_V2: usize = 16;
const NISAR_SSAR_METADATA_PLANNING_MAX_REQUESTS_V2: u64 = 512;
const NISAR_SSAR_V1_OBJECT_HEADER_MAX_BYTES_V2: u64 = 1024 * 1024;
const NISAR_SSAR_V1_HEAP_MAX_BYTES_V2: u64 = 4 * 1024 * 1024;
const NISAR_SSAR_V1_BTREE_MAX_NODES_V2: usize = 128;
const NISAR_SSAR_V1_BTREE_MAX_CHILDREN_V2: usize = 256;
const NISAR_SSAR_V1_SNOD_MAX_SYMBOLS_V2: usize = 256;
const NISAR_SSAR_V1_SYMBOL_ENTRY_BYTES_V2: u64 = 40;

fn planning_request_limit_v2(science_root: NisarScienceRoot) -> u64 {
    match science_root {
        NisarScienceRoot::Lsar => NISAR_LIVE_MAX_REQUESTS,
        NisarScienceRoot::Ssar => NISAR_SSAR_METADATA_PLANNING_MAX_REQUESTS_V2,
    }
}

fn valid_planning_request_limit_v2(limit: u64) -> bool {
    matches!(
        limit,
        NISAR_LIVE_MAX_REQUESTS | NISAR_SSAR_METADATA_PLANNING_MAX_REQUESTS_V2
    )
}

fn total_request_limit_v2(
    science_root: NisarScienceRoot,
) -> Result<u64, NisarSubsetPlanningErrorV2> {
    match science_root {
        NisarScienceRoot::Lsar => Ok(NISAR_LIVE_MAX_REQUESTS),
        NisarScienceRoot::Ssar => NISAR_SSAR_METADATA_PLANNING_MAX_REQUESTS_V2
            .checked_add(NISAR_LIVE_MAX_REQUESTS)
            .ok_or(NisarSubsetPlanningErrorV2::CombinedBudgetExceeded),
    }
}
const NISAR_SSAR_V1_DEFAULT_INDEXED_STORAGE_K_V2: usize = 32;
#[cfg(test)]
const NISAR_DEFERRED_FIXTURE_STRUCTURAL_RANGES_V2: [(u64, u64); 3] =
    [(0, 4_015), (4_031, 34_915), (35_627, 96_625)];
#[cfg(test)]
const NISAR_DEFERRED_FIXTURE_SCIENCE_PAYLOAD_END_V2: u64 = 4_291_223;

fn partition_metadata_prefix_subranges_v2(
    prefix_end_exclusive: u64,
) -> Result<Vec<(u64, u64)>, NisarSubsetPlanningErrorV2> {
    if prefix_end_exclusive == 0 {
        return Err(NisarSubsetPlanningErrorV2::BootstrapFailed);
    }
    let mut ranges = Vec::new();
    let mut start = 0_u64;
    while start < prefix_end_exclusive {
        let end_exclusive = start
            .checked_add(NISAR_SUBSET_METADATA_PREFIX_SUBRANGE_BYTES_V2)
            .map(|end| end.min(prefix_end_exclusive))
            .ok_or(NisarSubsetPlanningErrorV2::BootstrapBudgetExceeded)?;
        ranges.push((start, end_exclusive));
        start = end_exclusive;
    }
    Ok(ranges)
}

fn production_metadata_prefix_ranges_v2(
    source_size: u64,
) -> Result<Vec<(u64, u64)>, NisarSubsetPlanningErrorV2> {
    if source_size < crate::NISAR_LIVE_METADATA_PREFIX_BYTES {
        return Err(NisarSubsetPlanningErrorV2::BootstrapResponseInvalid);
    }
    partition_metadata_prefix_subranges_v2(crate::NISAR_LIVE_METADATA_PREFIX_BYTES)
}

fn remote_planning_initial_material_ranges(
    source_size: u64,
) -> Result<Vec<(u64, u64)>, NisarSubsetPlanningErrorV2> {
    #[cfg(test)]
    {
        // The cfg(test) deferred-layout RSLC fixture intentionally leaves its
        // selected science, exception-mask, and orbit-axis payload allocations
        // outside these bounded structural extents. The bytes still travel through
        // the exact production HTTP 206 path; only this deterministic layout is
        // test-specific.
        if source_size <= NISAR_DEFERRED_FIXTURE_SCIENCE_PAYLOAD_END_V2 {
            return Err(NisarSubsetPlanningErrorV2::BootstrapFailed);
        }
        let mut ranges = NISAR_DEFERRED_FIXTURE_STRUCTURAL_RANGES_V2.to_vec();
        ranges.push((NISAR_DEFERRED_FIXTURE_SCIENCE_PAYLOAD_END_V2, source_size));
        Ok(ranges)
    }
    #[cfg(not(test))]
    {
        production_metadata_prefix_ranges_v2(source_size)
    }
}

fn canonical_material_ranges_v2(
    mut ranges: Vec<(u64, u64)>,
    source_size: u64,
) -> Result<Vec<(u64, u64)>, NisarSubsetPlanningErrorV2> {
    if source_size == 0
        || ranges
            .iter()
            .any(|(start, end)| start >= end || *end > source_size)
    {
        return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
    }
    ranges.sort_unstable();
    let mut canonical = Vec::<(u64, u64)>::new();
    for (start, end) in ranges {
        if let Some(last) = canonical.last_mut() {
            if start <= last.1 {
                last.1 = last.1.max(end);
                continue;
            }
        }
        canonical.push((start, end));
    }
    Ok(canonical)
}

fn missing_material_ranges_v2(
    required: (u64, u64),
    material_ranges: &[(u64, u64)],
    source_size: u64,
) -> Result<Vec<(u64, u64)>, NisarSubsetPlanningErrorV2> {
    if required.0 >= required.1 || required.1 > source_size {
        return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
    }
    let canonical = canonical_material_ranges_v2(material_ranges.to_vec(), source_size)?;
    let mut missing = Vec::new();
    let mut cursor = required.0;
    for (start, end) in canonical {
        if end <= cursor || start >= required.1 {
            continue;
        }
        if start > cursor {
            missing.push((cursor, start.min(required.1)));
        }
        cursor = cursor.max(end.min(required.1));
        if cursor >= required.1 {
            break;
        }
    }
    if cursor < required.1 {
        missing.push((cursor, required.1));
    }
    Ok(missing)
}

fn append_missing_material_range_v2(
    required: (u64, u64),
    material_ranges: &[(u64, u64)],
    source_size: u64,
    missing: &mut Vec<(u64, u64)>,
) -> Result<bool, NisarSubsetPlanningErrorV2> {
    let gaps = missing_material_ranges_v2(required, material_ranges, source_size)?;
    let has_missing = !gaps.is_empty();
    missing.extend(gaps);
    Ok(has_missing)
}

fn read_sparse_materialized_range_v2(
    path: &Path,
    range: (u64, u64),
) -> Result<Vec<u8>, NisarSubsetPlanningErrorV2> {
    let length = range
        .1
        .checked_sub(range.0)
        .and_then(|length| usize::try_from(length).ok())
        .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
    let mut file =
        File::open(path).map_err(|_| NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
    file.seek(SeekFrom::Start(range.0))
        .map_err(|_| NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
    let mut bytes = vec![0_u8; length];
    file.read_exact(&mut bytes)
        .map_err(|_| NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
    Ok(bytes)
}

fn ssar_v1_indexed_storage_k_v2(
    sparse_path: &Path,
) -> Result<Option<usize>, NisarSubsetPlanningErrorV2> {
    let bytes = read_sparse_materialized_range_v2(sparse_path, (0, 28))?;
    if &bytes[..8] != b"\x89HDF\r\n\x1a\n" {
        return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
    }
    match bytes[8] {
        0 => {
            if bytes[13] != 8 || bytes[14] != 8 {
                return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
            }
            Ok(Some(NISAR_SSAR_V1_DEFAULT_INDEXED_STORAGE_K_V2))
        }
        1 => {
            if bytes[13] != 8 || bytes[14] != 8 {
                return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
            }
            let k = usize::from(le_u16_v2(&bytes[24..26])?);
            if k == 0 || k > 32_768 {
                return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
            }
            Ok(Some(k))
        }
        2 | 3 => {
            if bytes[9] != 8 || bytes[10] != 8 {
                return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
            }
            // The custom walker below is deliberately limited to the legacy
            // superblock/object-index layout used by reviewed Bhoonidhi SSAR
            // products. Newer HDF5 files are left to the native HDF5 catalog;
            // if their sparse structural bytes are insufficient, that catalog
            // still fails closed before any science transfer.
            Ok(None)
        }
        _ => Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable),
    }
}

fn ssar_v1_chunk_btree_node_bytes_v2(
    dimensionality: usize,
    indexed_storage_k: usize,
) -> Result<usize, NisarSubsetPlanningErrorV2> {
    if !(2..=16).contains(&dimensionality) || indexed_storage_k == 0 {
        return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
    }
    let key_bytes = 8_usize
        .checked_add(
            dimensionality
                .checked_mul(8)
                .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?,
        )
        .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
    indexed_storage_k
        .checked_mul(2)
        .and_then(|entries| entries.checked_mul(key_bytes.checked_add(8)?))
        .and_then(|bytes| bytes.checked_add(24))
        .and_then(|bytes| bytes.checked_add(key_bytes))
        .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SsarV1SymbolTableV2 {
    btree_addr: u64,
    heap_addr: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SsarV1ChunkIndexV2 {
    btree_addr: u64,
    dimensionality: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct SsarV1ObjectHeaderMetadataV2 {
    symbol_table: Option<SsarV1SymbolTableV2>,
    chunk_index: Option<SsarV1ChunkIndexV2>,
}

enum SsarV1ObjectHeaderProbeV2 {
    Missing,
    Ready(SsarV1ObjectHeaderMetadataV2),
}

fn le_u16_v2(bytes: &[u8]) -> Result<u16, NisarSubsetPlanningErrorV2> {
    bytes
        .try_into()
        .map(u16::from_le_bytes)
        .map_err(|_| NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)
}

fn le_u32_v2(bytes: &[u8]) -> Result<u32, NisarSubsetPlanningErrorV2> {
    bytes
        .try_into()
        .map(u32::from_le_bytes)
        .map_err(|_| NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)
}

fn le_u64_v2(bytes: &[u8]) -> Result<u64, NisarSubsetPlanningErrorV2> {
    bytes
        .try_into()
        .map(u64::from_le_bytes)
        .map_err(|_| NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)
}

fn parse_ssar_v1_chunk_layout_v2(
    payload: &[u8],
    source_size: u64,
) -> Result<Option<SsarV1ChunkIndexV2>, NisarSubsetPlanningErrorV2> {
    if payload.len() < 2 {
        return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
    }
    if payload[1] != 2 {
        return Ok(None);
    }
    if payload[0] != 3 || payload.len() < 11 {
        return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
    }
    let dimensionality = usize::from(payload[2]);
    let required = 11_usize
        .checked_add(
            dimensionality
                .checked_mul(4)
                .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?,
        )
        .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
    if !(2..=16).contains(&dimensionality) || payload.len() < required {
        return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
    }
    let btree_addr = le_u64_v2(&payload[3..11])?;
    if btree_addr == u64::MAX {
        return Ok(None);
    }
    if btree_addr == 0 || btree_addr >= source_size {
        return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
    }
    Ok(Some(SsarV1ChunkIndexV2 {
        btree_addr,
        dimensionality,
    }))
}

fn inspect_ssar_v1_object_message_v2(
    message_type: u16,
    payload: &[u8],
    source_size: u64,
    material_ranges: &[(u64, u64)],
    missing: &mut Vec<(u64, u64)>,
    metadata: &mut SsarV1ObjectHeaderMetadataV2,
    continuations: &mut VecDeque<(u64, u64)>,
) -> Result<bool, NisarSubsetPlanningErrorV2> {
    match message_type {
        0x0008 => {
            if let Some(chunk_index) = parse_ssar_v1_chunk_layout_v2(payload, source_size)? {
                if metadata
                    .chunk_index
                    .is_some_and(|existing| existing != chunk_index)
                {
                    return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
                }
                metadata.chunk_index = Some(chunk_index);
            }
        }
        0x0010 => {
            if payload.len() < 16 {
                return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
            }
            let continuation_addr = le_u64_v2(&payload[..8])?;
            let continuation_size = le_u64_v2(&payload[8..16])?;
            if continuation_size == 0
                || continuation_size > NISAR_SSAR_V1_OBJECT_HEADER_MAX_BYTES_V2
            {
                return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
            }
            let continuation_end = continuation_addr
                .checked_add(continuation_size)
                .filter(|end| *end <= source_size)
                .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
            if append_missing_material_range_v2(
                (continuation_addr, continuation_end),
                material_ranges,
                source_size,
                missing,
            )? {
                return Ok(true);
            }
            continuations.push_back((continuation_addr, continuation_end));
        }
        0x0011 => {
            if payload.len() < 16 {
                return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
            }
            let symbol_table = SsarV1SymbolTableV2 {
                btree_addr: le_u64_v2(&payload[..8])?,
                heap_addr: le_u64_v2(&payload[8..16])?,
            };
            if symbol_table.btree_addr == 0
                || symbol_table.btree_addr == u64::MAX
                || symbol_table.btree_addr >= source_size
                || symbol_table.heap_addr == 0
                || symbol_table.heap_addr == u64::MAX
                || symbol_table.heap_addr >= source_size
                || metadata
                    .symbol_table
                    .is_some_and(|existing| existing != symbol_table)
            {
                return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
            }
            metadata.symbol_table = Some(symbol_table);
        }
        _ => {}
    }
    Ok(false)
}

fn probe_ssar_v1_object_header_v2(
    sparse_path: &Path,
    object_addr: u64,
    source_size: u64,
    material_ranges: &[(u64, u64)],
    missing: &mut Vec<(u64, u64)>,
) -> Result<SsarV1ObjectHeaderProbeV2, NisarSubsetPlanningErrorV2> {
    let prefix_end = object_addr
        .checked_add(16)
        .filter(|end| *end <= source_size)
        .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
    if append_missing_material_range_v2(
        (object_addr, prefix_end),
        material_ranges,
        source_size,
        missing,
    )? {
        return Ok(SsarV1ObjectHeaderProbeV2::Missing);
    }
    let prefix = read_sparse_materialized_range_v2(sparse_path, (object_addr, prefix_end))?;
    if prefix.first().copied() != Some(1) {
        return Ok(SsarV1ObjectHeaderProbeV2::Ready(
            SsarV1ObjectHeaderMetadataV2::default(),
        ));
    }
    let message_count = usize::from(le_u16_v2(&prefix[2..4])?);
    let chunk_bytes = u64::from(le_u32_v2(&prefix[8..12])?);
    if message_count == 0
        || message_count > 512
        || chunk_bytes == 0
        || chunk_bytes > NISAR_SSAR_V1_OBJECT_HEADER_MAX_BYTES_V2
    {
        return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
    }
    let object_end = prefix_end
        .checked_add(chunk_bytes)
        .filter(|end| *end <= source_size)
        .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
    if append_missing_material_range_v2(
        (prefix_end, object_end),
        material_ranges,
        source_size,
        missing,
    )? {
        return Ok(SsarV1ObjectHeaderProbeV2::Missing);
    }
    let bytes = read_sparse_materialized_range_v2(sparse_path, (object_addr, object_end))?;
    let mut position = 16_usize;
    let mut metadata = SsarV1ObjectHeaderMetadataV2::default();
    let mut continuations = VecDeque::new();
    let mut visited_continuations = BTreeSet::new();
    let mut continuation_missing = false;
    let mut processed_messages = 0_usize;
    while processed_messages < message_count {
        // HDF5 v1 stores the total message count across the primary object
        // header chunk and every continuation block. Reaching the end of the
        // primary chunk is therefore valid when a continuation message has
        // already accounted for the remaining messages.
        if position == bytes.len() {
            break;
        }
        let header_end = position
            .checked_add(8)
            .filter(|end| *end <= bytes.len())
            .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
        let message_type = le_u16_v2(&bytes[position..position + 2])?;
        let message_size = usize::from(le_u16_v2(&bytes[position + 2..position + 4])?);
        let payload_end = header_end
            .checked_add(message_size)
            .filter(|end| *end <= bytes.len())
            .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
        let payload = &bytes[header_end..payload_end];
        continuation_missing |= inspect_ssar_v1_object_message_v2(
            message_type,
            payload,
            source_size,
            material_ranges,
            missing,
            &mut metadata,
            &mut continuations,
        )?;
        position = payload_end;
        processed_messages += 1;
    }
    while !continuation_missing && processed_messages < message_count {
        let (continuation_addr, continuation_end) = continuations
            .pop_front()
            .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
        if !visited_continuations.insert(continuation_addr) || visited_continuations.len() > 32 {
            return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
        }
        let continuation =
            read_sparse_materialized_range_v2(sparse_path, (continuation_addr, continuation_end))?;
        let mut continuation_position = 0_usize;
        while continuation_position < continuation.len() && processed_messages < message_count {
            let header_end = continuation_position
                .checked_add(8)
                .filter(|end| *end <= continuation.len())
                .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
            let message_type =
                le_u16_v2(&continuation[continuation_position..continuation_position + 2])?;
            let message_size = usize::from(le_u16_v2(
                &continuation[continuation_position + 2..continuation_position + 4],
            )?);
            let payload_end = header_end
                .checked_add(message_size)
                .filter(|end| *end <= continuation.len())
                .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
            let payload = &continuation[header_end..payload_end];
            continuation_missing |= inspect_ssar_v1_object_message_v2(
                message_type,
                payload,
                source_size,
                material_ranges,
                missing,
                &mut metadata,
                &mut continuations,
            )?;
            continuation_position = payload_end;
            processed_messages += 1;
        }
    }
    if continuation_missing {
        Ok(SsarV1ObjectHeaderProbeV2::Missing)
    } else if processed_messages != message_count {
        Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)
    } else {
        Ok(SsarV1ObjectHeaderProbeV2::Ready(metadata))
    }
}

fn inspect_ssar_v1_heap_v2(
    sparse_path: &Path,
    heap_addr: u64,
    source_size: u64,
    material_ranges: &[(u64, u64)],
    missing: &mut Vec<(u64, u64)>,
) -> Result<(), NisarSubsetPlanningErrorV2> {
    let header_end = heap_addr
        .checked_add(32)
        .filter(|end| *end <= source_size)
        .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
    if append_missing_material_range_v2(
        (heap_addr, header_end),
        material_ranges,
        source_size,
        missing,
    )? {
        return Ok(());
    }
    let header = read_sparse_materialized_range_v2(sparse_path, (heap_addr, header_end))?;
    if &header[..4] != b"HEAP" || header[4] != 0 {
        return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
    }
    let data_size = le_u64_v2(&header[8..16])?;
    let data_addr = le_u64_v2(&header[24..32])?;
    if data_size == 0 || data_size > NISAR_SSAR_V1_HEAP_MAX_BYTES_V2 {
        return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
    }
    let data_end = data_addr
        .checked_add(data_size)
        .filter(|end| *end <= source_size)
        .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
    append_missing_material_range_v2((data_addr, data_end), material_ranges, source_size, missing)?;
    Ok(())
}

fn inspect_ssar_v1_symbol_node_v2(
    sparse_path: &Path,
    node_addr: u64,
    source_size: u64,
    material_ranges: &[(u64, u64)],
    missing: &mut Vec<(u64, u64)>,
) -> Result<(), NisarSubsetPlanningErrorV2> {
    let header_end = node_addr
        .checked_add(8)
        .filter(|end| *end <= source_size)
        .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
    if append_missing_material_range_v2(
        (node_addr, header_end),
        material_ranges,
        source_size,
        missing,
    )? {
        return Ok(());
    }
    let header = read_sparse_materialized_range_v2(sparse_path, (node_addr, header_end))?;
    if &header[..4] != b"SNOD" || header[4] != 1 {
        return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
    }
    let symbol_count = usize::from(le_u16_v2(&header[6..8])?);
    if symbol_count > NISAR_SSAR_V1_SNOD_MAX_SYMBOLS_V2 {
        return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
    }
    let entries_bytes = u64::try_from(symbol_count)
        .ok()
        .and_then(|count| count.checked_mul(NISAR_SSAR_V1_SYMBOL_ENTRY_BYTES_V2))
        .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
    let node_end = header_end
        .checked_add(entries_bytes)
        .filter(|end| *end <= source_size)
        .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
    append_missing_material_range_v2(
        (header_end, node_end),
        material_ranges,
        source_size,
        missing,
    )?;
    Ok(())
}

fn inspect_ssar_v1_btree_v2(
    sparse_path: &Path,
    btree_addr: u64,
    source_size: u64,
    material_ranges: &[(u64, u64)],
    missing: &mut Vec<(u64, u64)>,
    visited: &mut BTreeSet<u64>,
) -> Result<(), NisarSubsetPlanningErrorV2> {
    if !visited.insert(btree_addr) {
        return Ok(());
    }
    if visited.len() > NISAR_SSAR_V1_BTREE_MAX_NODES_V2 {
        return Err(NisarSubsetPlanningErrorV2::BootstrapBudgetExceeded);
    }
    let header_end = btree_addr
        .checked_add(24)
        .filter(|end| *end <= source_size)
        .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
    if append_missing_material_range_v2(
        (btree_addr, header_end),
        material_ranges,
        source_size,
        missing,
    )? {
        return Ok(());
    }
    let header = read_sparse_materialized_range_v2(sparse_path, (btree_addr, header_end))?;
    if &header[..4] != b"TREE" || header[4] != 0 {
        return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
    }
    let level = header[5];
    if level > 16 {
        return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
    }
    let child_count = usize::from(le_u16_v2(&header[6..8])?);
    if child_count > NISAR_SSAR_V1_BTREE_MAX_CHILDREN_V2 {
        return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
    }
    let body_bytes = u64::try_from(child_count)
        .ok()
        .and_then(|count| count.checked_mul(16))
        .and_then(|bytes| bytes.checked_add(8))
        .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
    let node_end = header_end
        .checked_add(body_bytes)
        .filter(|end| *end <= source_size)
        .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
    if append_missing_material_range_v2(
        (header_end, node_end),
        material_ranges,
        source_size,
        missing,
    )? {
        return Ok(());
    }
    let node = read_sparse_materialized_range_v2(sparse_path, (btree_addr, node_end))?;
    let mut child_addresses = Vec::with_capacity(child_count);
    let mut position = 24_usize;
    for _ in 0..child_count {
        position = position
            .checked_add(8)
            .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
        let address_end = position
            .checked_add(8)
            .filter(|end| *end <= node.len())
            .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
        let child_addr = le_u64_v2(&node[position..address_end])?;
        if child_addr == 0 || child_addr == u64::MAX || child_addr >= source_size {
            return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
        }
        child_addresses.push(child_addr);
        position = address_end;
    }
    for child_addr in child_addresses {
        if level == 0 {
            inspect_ssar_v1_symbol_node_v2(
                sparse_path,
                child_addr,
                source_size,
                material_ranges,
                missing,
            )?;
        } else {
            inspect_ssar_v1_btree_v2(
                sparse_path,
                child_addr,
                source_size,
                material_ranges,
                missing,
                visited,
            )?;
        }
    }
    Ok(())
}

fn inspect_ssar_v1_chunk_btree_v2(
    sparse_path: &Path,
    chunk_index: SsarV1ChunkIndexV2,
    indexed_storage_k: usize,
    source_size: u64,
    material_ranges: &[(u64, u64)],
    missing: &mut Vec<(u64, u64)>,
    visited: &mut BTreeSet<u64>,
) -> Result<(), NisarSubsetPlanningErrorV2> {
    let btree_addr = chunk_index.btree_addr;
    if !visited.insert(btree_addr) {
        return Ok(());
    }
    if visited.len() > 1024 {
        return Err(NisarSubsetPlanningErrorV2::BootstrapBudgetExceeded);
    }
    let node_bytes =
        ssar_v1_chunk_btree_node_bytes_v2(chunk_index.dimensionality, indexed_storage_k)?;
    let node_end = btree_addr
        .checked_add(
            u64::try_from(node_bytes)
                .map_err(|_| NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?,
        )
        .filter(|end| *end <= source_size)
        .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
    if append_missing_material_range_v2(
        (btree_addr, node_end),
        material_ranges,
        source_size,
        missing,
    )? {
        return Ok(());
    }
    let node = read_sparse_materialized_range_v2(sparse_path, (btree_addr, node_end))?;
    if &node[..4] != b"TREE" || node[4] != 1 || node[5] > 16 {
        return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
    }
    let maximum_entries = indexed_storage_k
        .checked_mul(2)
        .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
    let child_count = usize::from(le_u16_v2(&node[6..8])?);
    if child_count > maximum_entries {
        return Err(NisarSubsetPlanningErrorV2::BootstrapBudgetExceeded);
    }
    let key_bytes = 8_usize
        .checked_add(
            chunk_index
                .dimensionality
                .checked_mul(8)
                .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?,
        )
        .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
    let level = node[5];
    let mut position = 24_usize;
    let mut child_addresses = Vec::with_capacity(child_count);
    for _ in 0..child_count {
        position = position
            .checked_add(key_bytes)
            .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
        let address_end = position
            .checked_add(8)
            .filter(|end| *end <= node.len())
            .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
        let child_addr = le_u64_v2(&node[position..address_end])?;
        if child_addr == 0 || child_addr == u64::MAX || child_addr >= source_size {
            return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
        }
        child_addresses.push(child_addr);
        position = address_end;
    }
    let _final_key_end = position
        .checked_add(key_bytes)
        .filter(|end| *end <= node.len())
        .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
    if level > 0 {
        for child_addr in child_addresses {
            inspect_ssar_v1_chunk_btree_v2(
                sparse_path,
                SsarV1ChunkIndexV2 {
                    btree_addr: child_addr,
                    dimensionality: chunk_index.dimensionality,
                },
                indexed_storage_k,
                source_size,
                material_ranges,
                missing,
                visited,
            )?;
        }
    }
    Ok(())
}

fn inspect_ssar_v1_group_structure_v2(
    sparse_path: &Path,
    object_addr: u64,
    source_size: u64,
    material_ranges: &[(u64, u64)],
    missing: &mut Vec<(u64, u64)>,
) -> Result<bool, NisarSubsetPlanningErrorV2> {
    let before = missing.len();
    let probe = probe_ssar_v1_object_header_v2(
        sparse_path,
        object_addr,
        source_size,
        material_ranges,
        missing,
    )?;
    let SsarV1ObjectHeaderProbeV2::Ready(metadata) = probe else {
        return Ok(false);
    };
    let Some(symbol_table) = metadata.symbol_table else {
        return Ok(true);
    };
    inspect_ssar_v1_heap_v2(
        sparse_path,
        symbol_table.heap_addr,
        source_size,
        material_ranges,
        missing,
    )?;
    let mut visited = BTreeSet::new();
    inspect_ssar_v1_btree_v2(
        sparse_path,
        symbol_table.btree_addr,
        source_size,
        material_ranges,
        missing,
        &mut visited,
    )?;
    Ok(missing.len() == before)
}

fn ssar_hard_link_object_address_v2(
    file: &hdf5::File,
    group: &hdf5::Group,
    name: &str,
) -> Result<u64, NisarSubsetPlanningErrorV2> {
    use hdf5_sys::h5::H5free_memory;
    use hdf5_sys::h5l::{H5L_info2_t, H5Lget_info2, H5L_TYPE_HARD};
    use hdf5_sys::h5o::H5Otoken_to_str;
    use hdf5_sys::h5p::H5P_DEFAULT;

    if name.is_empty()
        || name.len() > 1024
        || name.contains('/')
        || name.chars().any(char::is_control)
    {
        return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
    }
    let name =
        CString::new(name).map_err(|_| NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
    let mut info = H5L_info2_t::default();
    let status = hdf5::sync::sync(|| unsafe {
        H5Lget_info2(group.id(), name.as_ptr(), &mut info, H5P_DEFAULT)
    });
    if status < 0 || info.type_ != H5L_TYPE_HARD {
        return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
    }
    let mut token_text = std::ptr::null_mut();
    let status = hdf5::sync::sync(|| unsafe {
        H5Otoken_to_str(file.id(), info.u.token().cast_const(), &mut token_text)
    });
    if status < 0 || token_text.is_null() {
        return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
    }
    let token = unsafe { CStr::from_ptr(token_text) }
        .to_str()
        .ok()
        .and_then(|value| value.parse::<u64>().ok());
    let _ = hdf5::sync::sync(|| unsafe { H5free_memory(token_text.cast()) });
    token.ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)
}

fn discover_ssar_v1_metadata_ranges_v2(
    sparse_path: &Path,
    source_size: u64,
    material_ranges: &[(u64, u64)],
) -> Result<Vec<(u64, u64)>, NisarSubsetPlanningErrorV2> {
    let material_ranges = canonical_material_ranges_v2(material_ranges.to_vec(), source_size)?;
    let Some(indexed_storage_k) = ssar_v1_indexed_storage_k_v2(sparse_path)? else {
        return Ok(Vec::new());
    };
    let file = crate::hdf5_file::open(sparse_path)
        .map_err(|_| NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
    let science = file
        .group("/science")
        .map_err(|_| NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
    let ssar_addr = ssar_hard_link_object_address_v2(&file, &science, "SSAR")?;
    let mut queue = VecDeque::from([("/science/SSAR".to_owned(), ssar_addr)]);
    let mut visited_groups = BTreeSet::new();
    let mut missing = Vec::new();

    while let Some((group_path, object_addr)) = queue.pop_front() {
        if !visited_groups.insert(object_addr) {
            continue;
        }
        if visited_groups.len() > 512 {
            return Err(NisarSubsetPlanningErrorV2::BootstrapBudgetExceeded);
        }
        if !inspect_ssar_v1_group_structure_v2(
            sparse_path,
            object_addr,
            source_size,
            &material_ranges,
            &mut missing,
        )? {
            continue;
        }
        let group = file
            .group(&group_path)
            .map_err(|_| NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
        let member_names = group
            .member_names()
            .map_err(|_| NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
        for name in member_names {
            let child_addr = ssar_hard_link_object_address_v2(&file, &group, &name)?;
            let child_probe = probe_ssar_v1_object_header_v2(
                sparse_path,
                child_addr,
                source_size,
                &material_ranges,
                &mut missing,
            )?;
            let SsarV1ObjectHeaderProbeV2::Ready(child_metadata) = child_probe else {
                continue;
            };
            if let Some(chunk_index) = child_metadata.chunk_index {
                let mut visited = BTreeSet::new();
                inspect_ssar_v1_chunk_btree_v2(
                    sparse_path,
                    chunk_index,
                    indexed_storage_k,
                    source_size,
                    &material_ranges,
                    &mut missing,
                    &mut visited,
                )?;
            }
            let child_path = format!("{group_path}/{name}");
            if file.group(&child_path).is_ok() {
                queue.push_back((child_path, child_addr));
            }
        }
    }

    let missing = canonical_material_ranges_v2(missing, source_size)?;
    if missing.len() > NISAR_SSAR_METADATA_DISCOVERY_MAX_RANGES_V2 {
        return Err(NisarSubsetPlanningErrorV2::BootstrapBudgetExceeded);
    }
    let response_bytes = missing.iter().try_fold(0_u64, |total, (start, end)| {
        total
            .checked_add(end - start)
            .ok_or(NisarSubsetPlanningErrorV2::BootstrapBudgetExceeded)
    })?;
    if response_bytes > NISAR_SSAR_METADATA_DISCOVERY_MAX_RESPONSE_BYTES_V2 {
        return Err(NisarSubsetPlanningErrorV2::BootstrapBudgetExceeded);
    }
    Ok(missing)
}

fn planning_material_ranges_from_statistics_v2(
    statistics: &NisarSubsetBootstrapStatisticsV2,
) -> Result<Vec<(u64, u64)>, NisarSubsetPlanningErrorV2> {
    let mut ranges = statistics
        .metadata_prefix
        .requested_ranges
        .iter()
        .chain(&statistics.metadata_discovery.requested_ranges)
        .chain(&statistics.additional_geolocation.requested_ranges)
        .map(|range| (range.start, range.end_exclusive))
        .collect::<Vec<_>>();
    ranges.sort_unstable();
    if ranges.is_empty()
        || ranges
            .iter()
            .any(|(start, end)| start >= end || *end > statistics.source_size)
        || ranges.windows(2).any(|pair| pair[0].1 > pair[1].0)
    {
        return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
    }
    Ok(ranges)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NisarSubsetVerifiedInventoryIdentityV2 {
    inventory_sha256: Option<String>,
    inventory_evidence_sha256: String,
    source_graph_sha256: String,
    source_validator_sha256: Option<String>,
    source_size: u64,
    source_dataset_count: u64,
    metadata_group_binding_count: u64,
    metadata_dataset_binding_count: u64,
}

impl NisarSubsetVerifiedInventoryIdentityV2 {
    pub fn inventory_sha256(&self) -> Option<&str> {
        self.inventory_sha256.as_deref()
    }
    pub fn inventory_evidence_sha256(&self) -> &str {
        &self.inventory_evidence_sha256
    }
    pub fn source_graph_sha256(&self) -> &str {
        &self.source_graph_sha256
    }
    pub fn source_validator_sha256(&self) -> Option<&str> {
        self.source_validator_sha256.as_deref()
    }
    pub fn source_size(&self) -> u64 {
        self.source_size
    }
    pub fn source_dataset_count(&self) -> u64 {
        self.source_dataset_count
    }
    pub fn metadata_group_binding_count(&self) -> u64 {
        self.metadata_group_binding_count
    }
    pub fn metadata_dataset_binding_count(&self) -> u64 {
        self.metadata_dataset_binding_count
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct NisarSubsetPlanningCompleteV2 {
    frozen_plan: NisarSubsetPlanV2,
    bootstrap_statistics: NisarSubsetBootstrapStatisticsV2,
    verified_inventory_identity: NisarSubsetVerifiedInventoryIdentityV2,
    physical_execution_plan: NisarSubsetPhysicalExecutionPlanV2,
    planning_workspace_checkpoint_reused: bool,
}

impl NisarSubsetPlanningCompleteV2 {
    pub fn frozen_plan(&self) -> &NisarSubsetPlanV2 {
        &self.frozen_plan
    }
    pub fn bootstrap_statistics(&self) -> &NisarSubsetBootstrapStatisticsV2 {
        &self.bootstrap_statistics
    }
    pub fn verified_inventory_identity(&self) -> &NisarSubsetVerifiedInventoryIdentityV2 {
        &self.verified_inventory_identity
    }
    pub(crate) fn physical_execution_plan(&self) -> &NisarSubsetPhysicalExecutionPlanV2 {
        &self.physical_execution_plan
    }
    pub(crate) fn planning_workspace_checkpoint_reused(&self) -> bool {
        self.planning_workspace_checkpoint_reused
    }
}

pub(crate) fn revalidate_science_authority_v2(
    work: &DownloadWork,
    completed: &NisarSubsetPlanningCompleteV2,
    bootstrap: &dyn NisarSubsetMetadataBootstrapV2,
    control: &WorkerControl,
) -> Result<
    (
        NisarSubsetPhysicalExecutionPlanV2,
        NisarSubsetBootstrapStatisticsV2,
    ),
    NisarSubsetPlanningErrorV2,
> {
    ensure_active(control)?;
    let prepared = read_prepared_intent(work)?;
    validate_plan_binding(&completed.frozen_plan, &prepared)?;
    let current = parse_nisar_subset_plan_v2_envelope(&work.item.metadata)
        .map_err(|_| NisarSubsetPlanningErrorV2::FrozenPlanInvalid)?
        .ok_or(NisarSubsetPlanningErrorV2::FrozenPlanMissing)?
        .plan()
        .map_err(|_| NisarSubsetPlanningErrorV2::FrozenPlanInvalid)?;
    if current != completed.frozen_plan {
        return Err(NisarSubsetPlanningErrorV2::FrozenPlanInvalid);
    }
    let (physical, statistics, _recovered_inventory) =
        bootstrap.reopen_planning_workspace(work, &current, control)?;
    if physical != completed.physical_execution_plan {
        return Err(NisarSubsetPlanningErrorV2::PhysicalPlanInvalid);
    }
    if statistics
        != completed
            .bootstrap_statistics
            .clone()
            .mark_frozen_plan_fast_path()
    {
        return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
    }
    Ok((physical, statistics))
}

pub(crate) fn revalidate_science_range_binding_v2(
    work: &DownloadWork,
    completed: &NisarSubsetPlanningCompleteV2,
    expected_plan: &NisarSubsetPhysicalExecutionPlanV2,
    control: &WorkerControl,
) -> Result<(), NisarSubsetPlanningErrorV2> {
    ensure_active(control)?;
    let prepared = read_prepared_intent(work)?;
    validate_plan_binding(&completed.frozen_plan, &prepared)?;
    let current = parse_nisar_subset_plan_v2_envelope(&work.item.metadata)
        .map_err(|_| NisarSubsetPlanningErrorV2::FrozenPlanInvalid)?
        .ok_or(NisarSubsetPlanningErrorV2::FrozenPlanMissing)?
        .plan()
        .map_err(|_| NisarSubsetPlanningErrorV2::FrozenPlanInvalid)?;
    if current != completed.frozen_plan {
        return Err(NisarSubsetPlanningErrorV2::FrozenPlanInvalid);
    }
    let completed_plan = &completed.physical_execution_plan;
    if expected_plan.physical_plan_sha256 != completed_plan.physical_plan_sha256
        || expected_plan.prepared_intent_sha256 != current.prepared_intent_sha256
        || expected_plan.canonical_semantic_plan_digest != current.canonical_semantic_plan_digest
        || expected_plan.source_product_identity != current.source_product_identity
        || expected_plan.source_asset_identity != current.source_asset_identity
    {
        return Err(NisarSubsetPlanningErrorV2::PhysicalPlanInvalid);
    }
    ensure_active(control)
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum NisarSubsetPlanningErrorV2 {
    #[error("work item is not a NISAR remote-derived V2 subset")]
    DeliveryKindInvalid,
    #[error("frozen NISAR V2 PreparedIntent is missing or invalid")]
    PreparedIntentInvalid,
    #[error("frozen NISAR V2 plan is missing")]
    FrozenPlanMissing,
    #[error("frozen NISAR V2 plan is invalid or not bound to PreparedIntent")]
    FrozenPlanInvalid,
    #[error("NISAR V2 durable planning workspace checkpoint is missing")]
    PlanningWorkspaceMissing,
    #[error("NISAR V2 durable planning workspace identity does not match")]
    PlanningWorkspaceIdentityMismatch,
    #[error("NISAR V2 durable recovery checkpoint is invalid")]
    RecoveryInvalid,
    #[error("NISAR V2 metadata bootstrap failed")]
    BootstrapFailed,
    #[error("NISAR V2 source is unavailable")]
    BootstrapSourceUnavailable,
    #[error("NISAR V2 source does not support exact Range responses")]
    BootstrapRangeUnsupported,
    #[error("NISAR V2 Range response protocol or headers are invalid")]
    BootstrapResponseInvalid,
    #[error("NISAR V2 Range response body is partial")]
    BootstrapPartialResponse,
    #[error("NISAR V2 Range transport failed")]
    BootstrapTransport,
    #[error("NISAR V2 source did not provide a strong ETag")]
    BootstrapStrongValidatorRequired,
    #[error("NISAR V2 Range is not satisfiable")]
    BootstrapRangeNotSatisfiable,
    #[error("NISAR V2 metadata bootstrap statistics are invalid")]
    BootstrapStatisticsInvalid,
    #[error("NISAR V2 metadata bootstrap budget was exceeded")]
    BootstrapBudgetExceeded,
    #[error("NISAR V2 bootstrap observed selected science payload bytes")]
    ScienceTransferObserved,
    #[error("NISAR V2 verified inventory is not bound to the current source")]
    InventoryBindingInvalid,
    #[error("NISAR V2 verified inventory does not match the frozen workspace")]
    InventoryMismatch,
    #[error("selected NISAR V2 science dataset is missing")]
    SelectedScienceMissing,
    #[error("NISAR V2 physical mapping is unavailable")]
    PhysicalMappingUnavailable,
    #[error("NISAR V2 physical execution plan is invalid")]
    PhysicalPlanInvalid,
    #[error("NISAR V2 combined planning and science budget was exceeded")]
    CombinedBudgetExceeded,
    #[error("NISAR V2 source validator changed after planning")]
    ValidatorChanged,
    #[error("NISAR V2 source dependency graph is invalid")]
    DependencyGraphInvalid,
    #[error("NISAR V2 final plan is invalid")]
    FinalPlanInvalid,
    #[error("NISAR V2 planning stopped before PlanReady")]
    ExecutionStopped,
    #[error("NISAR V2 PreparedIntent changed during planning")]
    PreparedIntentChanged,
    #[error("NISAR V2 strong source validator changed during planning")]
    SourceValidatorChanged,
    #[error("NISAR V2 Store plan freeze failed")]
    FreezeFailed,
    /// Issue #8 slice 6: the derived output exceeds the shared output bound.
    #[error("NISAR V2 derived output exceeds the output bound ({required} of {limit} bytes)")]
    OutputLimitExceeded { required: u64, limit: u64 },
    /// Issue #8 slice 6: the output volume cannot hold the derived output
    /// together with the Ranges still to transfer.
    #[error("NISAR V2 output volume lacks free space ({available} of {required} bytes)")]
    OutputDiskSpaceInsufficient { required: u64, available: u64 },
}

pub fn plan_and_freeze_nisar_subset_v2(
    work: &DownloadWork,
    control: &WorkerControl,
    plan_freeze: &NisarExecutionPlanFreezeAuthority,
    bootstrap: &dyn NisarSubsetMetadataBootstrapV2,
) -> Result<NisarSubsetPlanningCompleteV2, NisarSubsetPlanningErrorV2> {
    plan_and_freeze_nisar_subset_v2_with_progress(
        work,
        control,
        plan_freeze,
        bootstrap,
        &mut |_| {},
    )
}

pub fn plan_and_freeze_nisar_subset_v2_with_progress(
    work: &DownloadWork,
    control: &WorkerControl,
    plan_freeze: &NisarExecutionPlanFreezeAuthority,
    bootstrap: &dyn NisarSubsetMetadataBootstrapV2,
    transfer_progress: &mut dyn FnMut(u64),
) -> Result<NisarSubsetPlanningCompleteV2, NisarSubsetPlanningErrorV2> {
    plan_and_freeze_nisar_subset_v2_with_transfer_progress(
        work,
        control,
        bootstrap,
        |ready, control| {
            plan_freeze
                .freeze(ready, control)
                .map_err(|error| match error {
                    crate::NisarExecutionPlanFreezeError::ExecutionStopped => {
                        NisarSubsetPlanningErrorV2::ExecutionStopped
                    }
                    crate::NisarExecutionPlanFreezeError::Store(_)
                    | crate::NisarExecutionPlanFreezeError::CoordinatorUnavailable => {
                        NisarSubsetPlanningErrorV2::FreezeFailed
                    }
                })
        },
        transfer_progress,
    )
}

fn plan_and_freeze_nisar_subset_v2_with(
    work: &DownloadWork,
    control: &WorkerControl,
    bootstrap: &dyn NisarSubsetMetadataBootstrapV2,
    freeze: impl FnOnce(
        NisarExecutionPlanReadyV2,
        &WorkerControl,
    ) -> Result<NisarSubsetPlanV2, NisarSubsetPlanningErrorV2>,
) -> Result<NisarSubsetPlanningCompleteV2, NisarSubsetPlanningErrorV2> {
    plan_and_freeze_nisar_subset_v2_with_transfer_progress(
        work,
        control,
        bootstrap,
        freeze,
        &mut |_| {},
    )
}

fn plan_and_freeze_nisar_subset_v2_with_transfer_progress(
    work: &DownloadWork,
    control: &WorkerControl,
    bootstrap: &dyn NisarSubsetMetadataBootstrapV2,
    freeze: impl FnOnce(
        NisarExecutionPlanReadyV2,
        &WorkerControl,
    ) -> Result<NisarSubsetPlanV2, NisarSubsetPlanningErrorV2>,
    transfer_progress: &mut dyn FnMut(u64),
) -> Result<NisarSubsetPlanningCompleteV2, NisarSubsetPlanningErrorV2> {
    let prepared_envelope = read_prepared_intent(work)?;
    let prepared_intent = prepared_envelope
        .intent()
        .map_err(|_| NisarSubsetPlanningErrorV2::PreparedIntentInvalid)?;
    let startup_intent_sha256 = prepared_envelope.canonical_intent_sha256.clone();
    if let Some(frozen) = parse_nisar_subset_plan_v2_envelope(&work.item.metadata)
        .map_err(|_| NisarSubsetPlanningErrorV2::FrozenPlanInvalid)?
    {
        let plan = frozen
            .plan()
            .map_err(|_| NisarSubsetPlanningErrorV2::FrozenPlanInvalid)?;
        validate_plan_binding(&plan, &prepared_envelope)?;
        let source_size = plan
            .source_estimate
            .as_ref()
            .map(|estimate| estimate.bytes)
            .filter(|size| *size > 0)
            .ok_or(NisarSubsetPlanningErrorV2::FrozenPlanInvalid)?;
        if work
            .item
            .asset_access
            .as_ref()
            .and_then(|asset| asset.expected_size_bytes)
            .is_some_and(|expected| expected != source_size)
        {
            return Err(NisarSubsetPlanningErrorV2::FrozenPlanInvalid);
        }
        let mut checkpoint_reused = true;
        let recovered = match bootstrap.reopen_planning_workspace(work, &plan, control) {
            Err(NisarSubsetPlanningErrorV2::PlanningWorkspaceMissing) => {
                bootstrap.repair_missing_planning_workspace(
                    work,
                    &plan,
                    control,
                    transfer_progress,
                )?;
                checkpoint_reused = false;
                bootstrap.reopen_planning_workspace(work, &plan, control)?
            }
            result => result?,
        };
        let (physical_execution_plan, bootstrap_statistics, recovered_inventory) = recovered;
        let inventory_sha256 = physical_execution_plan.inventory_sha256.clone();
        let source_validator_sha256 = physical_execution_plan.source_validator_sha256.clone();
        return Ok(NisarSubsetPlanningCompleteV2 {
            verified_inventory_identity: NisarSubsetVerifiedInventoryIdentityV2 {
                inventory_sha256: Some(inventory_sha256),
                inventory_evidence_sha256: plan.source_dependency_graph.evidence_sha256.clone(),
                source_graph_sha256: plan.required_dependencies.source_graph_sha256.clone(),
                source_validator_sha256: Some(source_validator_sha256),
                source_size,
                source_dataset_count: recovered_inventory.evidence().source_dataset_count(),
                metadata_group_binding_count: recovered_inventory.groups().len() as u64,
                metadata_dataset_binding_count: recovered_inventory.datasets().len() as u64,
            },
            bootstrap_statistics,
            frozen_plan: plan,
            physical_execution_plan,
            planning_workspace_checkpoint_reused: checkpoint_reused,
        });
    }

    ensure_active(control)?;
    if let Some(proposed_plan) = bootstrap.recover_prepared_plan(work, &prepared_intent, control)? {
        // Retry only the acknowledgement for completed, independently verified
        // preparation. Like frozen-plan recovery this performs no new network I/O:
        // the checkpoint retains the original validated source and exact bootstrap
        // budget. Science execution still requires Store authority and validates
        // that same strong source validator on every Range request.
        ensure_active(control)?;
        let prepared_digest = proposed_plan.canonical_semantic_plan_digest.clone();
        let frozen_plan = freeze(
            NisarExecutionPlanReadyV2 {
                task_id: work.task.task_id.clone(),
                item_id: work.item.item_id.clone(),
                prepared_intent_sha256: startup_intent_sha256,
                proposed_plan,
            },
            control,
        )?;
        validate_plan_binding(&frozen_plan, &prepared_envelope)?;
        if frozen_plan.canonical_semantic_plan_digest != prepared_digest {
            return Err(NisarSubsetPlanningErrorV2::FrozenPlanInvalid);
        }
        // Recheck material after the Store acknowledgement; preparation alone
        // grants no science-transfer authority.
        let (physical_execution_plan, bootstrap_statistics, inventory) =
            bootstrap.reopen_planning_workspace(work, &frozen_plan, control)?;
        return Ok(NisarSubsetPlanningCompleteV2 {
            verified_inventory_identity: NisarSubsetVerifiedInventoryIdentityV2 {
                inventory_sha256: Some(inventory.evidence().inventory_sha256().to_owned()),
                inventory_evidence_sha256: inventory.evidence().evidence_sha256().to_owned(),
                source_graph_sha256: frozen_plan
                    .required_dependencies
                    .source_graph_sha256
                    .clone(),
                source_validator_sha256: Some(
                    inventory.evidence().source_validator_sha256().to_owned(),
                ),
                source_size: inventory.evidence().source_size(),
                source_dataset_count: inventory.evidence().source_dataset_count(),
                metadata_group_binding_count: inventory.groups().len() as u64,
                metadata_dataset_binding_count: inventory.datasets().len() as u64,
            },
            frozen_plan,
            physical_execution_plan,
            bootstrap_statistics,
            planning_workspace_checkpoint_reused: true,
        });
    }
    let mut output =
        bootstrap.bootstrap_with_progress(work, &prepared_intent, control, transfer_progress)?;
    validate_inventory_binding(&output.inventory, &prepared_intent)?;
    let proposed_plan = build_proposed_plan_v2(prepared_intent, &output.inventory)?;
    ensure_active(control)?;
    if read_prepared_intent(work)? != prepared_envelope {
        return Err(NisarSubsetPlanningErrorV2::PreparedIntentChanged);
    }
    let startup_validator_sha256 = output
        .inventory
        .evidence()
        .source_validator_sha256()
        .to_owned();
    let revalidation_reservation =
        output
            .ledger
            .reserve(BootstrapCategory::Revalidation, 0, 1, 1)?;
    let revalidated = match bootstrap.revalidate_before_freeze(
        work,
        &startup_intent_sha256,
        &startup_validator_sha256,
        control,
    ) {
        Ok(revalidated) => revalidated,
        Err(error) => {
            output.ledger.cancel(revalidation_reservation)?;
            return Err(error);
        }
    };
    output.ledger.commit_with_request_count(
        revalidation_reservation,
        revalidated.actual_response_body_bytes,
        revalidated.request_count,
    )?;
    ensure_active(control)?;
    if revalidated.prepared_intent_sha256 != startup_intent_sha256 {
        return Err(NisarSubsetPlanningErrorV2::PreparedIntentChanged);
    }
    if revalidated.source_validator_sha256 != startup_validator_sha256 {
        return Err(NisarSubsetPlanningErrorV2::SourceValidatorChanged);
    }
    let statistics = output.ledger.statistics()?;
    validate_plan_binding(&proposed_plan, &prepared_envelope)?;
    if !proposed_plan.required_dependencies.complete
        || proposed_plan.required_dependencies.unknown_dependency_count != 0
        || proposed_plan.source_dependency_graph.evidence_sha256
            != output.inventory.evidence().evidence_sha256()
        || statistics.science_request_count() != 0
        || statistics.science_response_body_bytes() != 0
    {
        return Err(NisarSubsetPlanningErrorV2::FinalPlanInvalid);
    }
    let identity = NisarSubsetVerifiedInventoryIdentityV2 {
        inventory_sha256: Some(output.inventory.evidence().inventory_sha256().to_owned()),
        inventory_evidence_sha256: output.inventory.evidence().evidence_sha256().to_owned(),
        source_graph_sha256: proposed_plan
            .required_dependencies
            .source_graph_sha256
            .clone(),
        source_validator_sha256: Some(startup_validator_sha256.to_owned()),
        source_size: output.inventory.evidence().source_size(),
        source_dataset_count: output.inventory.evidence().source_dataset_count(),
        metadata_group_binding_count: output.inventory.groups().len() as u64,
        metadata_dataset_binding_count: output.inventory.datasets().len() as u64,
    };
    // Persist recovery evidence before the Store CAS. A failed physical mapping
    // must not leave an unrecoverable frozen plan in Store.
    let physical_execution_plan = bootstrap.build_and_checkpoint_physical_plan(
        work,
        &proposed_plan,
        &output.inventory,
        &statistics,
        control,
    )?;
    admit_planned_output_v2(work, &physical_execution_plan)?;
    let prepared_digest = proposed_plan.canonical_semantic_plan_digest.clone();
    let frozen_plan = freeze(
        NisarExecutionPlanReadyV2 {
            task_id: work.task.task_id.clone(),
            item_id: work.item.item_id.clone(),
            prepared_intent_sha256: startup_intent_sha256,
            proposed_plan,
        },
        control,
    )?;
    validate_plan_binding(&frozen_plan, &prepared_envelope)?;
    if frozen_plan.canonical_semantic_plan_digest != prepared_digest {
        return Err(NisarSubsetPlanningErrorV2::FrozenPlanInvalid);
    }
    let (physical_execution_plan, _, _) =
        bootstrap.reopen_planning_workspace(work, &frozen_plan, control)?;
    Ok(NisarSubsetPlanningCompleteV2 {
        frozen_plan,
        bootstrap_statistics: statistics,
        verified_inventory_identity: identity,
        physical_execution_plan,
        planning_workspace_checkpoint_reused: false,
    })
}

fn build_proposed_plan_v2(
    prepared_intent: NisarSubsetPreparedIntentV2,
    inventory: &NisarSubsetVerifiedMetadataInventoryV2,
) -> Result<NisarSubsetPlanV2, NisarSubsetPlanningErrorV2> {
    let graph = build_nisar_subset_source_dependency_graph_v2(inventory)
        .map_err(|_| NisarSubsetPlanningErrorV2::DependencyGraphInvalid)?;
    if graph
        .nodes
        .iter()
        .any(|node| node.kind == NisarSubsetSourceDependencyNodeKindV2::Unknown)
    {
        return Err(NisarSubsetPlanningErrorV2::DependencyGraphInvalid);
    }
    NisarSubsetPlanV2::build_from_prepared_intent(
        prepared_intent,
        graph,
        NisarSubsetPlanMetadataEvidenceV2 {
            source_estimate: Some(NisarSubsetByteEstimateV2 {
                bytes: inventory.evidence().source_size(),
                evidence_sha256: inventory.evidence().evidence_sha256().to_owned(),
            }),
            transfer_estimate: None,
            derived_estimate: None,
            warnings: vec![
                "Physical subset byte estimates are deferred until verified window/chunk mapping."
                    .to_owned(),
            ],
        },
    )
    .map_err(|_| NisarSubsetPlanningErrorV2::FinalPlanInvalid)
}

fn recover_prepared_checkpoint_plan_v2(
    bootstrap: &dyn NisarSubsetMetadataBootstrapV2,
    workspace: &Hdf5RangeWorkspaceV1,
    work: &DownloadWork,
    intent: &NisarSubsetPreparedIntentV2,
    control: &WorkerControl,
) -> Result<Option<NisarSubsetPlanV2>, NisarSubsetPlanningErrorV2> {
    ensure_active(control)?;
    let Some(text) = workspace
        .load_planning_checkpoint_v2()
        .map_err(map_workspace_load_error)?
    else {
        return Ok(None);
    };
    let checkpoint = parse_planning_workspace_checkpoint_v2(&text)?;
    let inventory = NisarSubsetVerifiedMetadataInventoryV2::from_verified_live_metadata_hdf5(
        intent,
        checkpoint.physical_plan.source_validator.clone(),
        &workspace.part_path(),
        &|| control.is_cancelled() || control.is_paused(),
    )
    .map_err(|error| {
        map_live_inventory_error(error, NisarSubsetPlanningErrorV2::RecoveryInvalid)
    })?;
    validate_inventory_binding(&inventory, intent)?;
    let proposed = build_proposed_plan_v2(intent.clone(), &inventory)?;
    // Independently rebuild from intent + real metadata, then check all saved
    // task/item/source/material/physical bindings before offering it to Store.
    let (physical_execution_plan, _, _) =
        bootstrap.reopen_planning_workspace(work, &proposed, control)?;
    // A plan refused before an earlier freeze stays refused on recovery.
    admit_planned_output_v2(work, &physical_execution_plan)?;
    Ok(Some(proposed))
}

/// Issue #8 slice 6: an output that the writer cannot produce (shared output
/// bound) or that the output volume cannot hold together with every planned
/// Range is refused before the Store freezes the plan, so no science request
/// is ever spent on it. The frozen-plan estimate is not changed.
fn admit_planned_output_v2(
    work: &DownloadWork,
    plan: &NisarSubsetPhysicalExecutionPlanV2,
) -> Result<(), NisarSubsetPlanningErrorV2> {
    let pending_transfer_bytes = plan.estimated_execution_transfer_response_bytes()?;
    crate::nisar_subset_stream_v2::v2_admit_planned_output(
        plan,
        &work.task.output_root,
        pending_transfer_bytes,
    )
    .map_err(crate::nisar_subset_stream_v2::v2_planning_error_of)?;
    Ok(())
}

fn read_prepared_intent(
    work: &DownloadWork,
) -> Result<NisarSubsetPreparedIntentEnvelopeV2, NisarSubsetPlanningErrorV2> {
    if work
        .item
        .metadata
        .get("delivery_kind")
        .and_then(serde_json::Value::as_str)
        != Some(NISAR_SUBSET_V2_DELIVERY_KIND)
    {
        return Err(NisarSubsetPlanningErrorV2::DeliveryKindInvalid);
    }
    parse_nisar_subset_prepared_intent_v2_envelope(&work.item.metadata)
        .map_err(|_| NisarSubsetPlanningErrorV2::PreparedIntentInvalid)?
        .ok_or(NisarSubsetPlanningErrorV2::PreparedIntentInvalid)
}

fn validate_plan_binding(
    plan: &NisarSubsetPlanV2,
    intent: &NisarSubsetPreparedIntentEnvelopeV2,
) -> Result<(), NisarSubsetPlanningErrorV2> {
    plan.validate()
        .map_err(|_| NisarSubsetPlanningErrorV2::FrozenPlanInvalid)?;
    if plan.prepared_intent_sha256 != intent.canonical_intent_sha256
        || plan.source_product_identity != intent.source_product_identity
        || plan.source_asset_identity != intent.source_asset_identity
        || plan.source_footprint_sha256 != intent.source_footprint_sha256
        || plan.canonical_geometry_digest.requested_sha256 != intent.requested_geometry_sha256
        || plan.canonical_geometry_digest.effective_sha256 != intent.effective_geometry_sha256
        || plan.selected_frequency != intent.selected_frequency
        || plan.selected_polarizations != intent.selected_polarizations
        || plan.transfer_mode != intent.transfer_mode
        || plan.provenance.derived_identity_authority != intent.derived_identity_authority
    {
        return Err(NisarSubsetPlanningErrorV2::FrozenPlanInvalid);
    }
    Ok(())
}

fn validate_inventory_binding(
    inventory: &NisarSubsetVerifiedMetadataInventoryV2,
    intent: &NisarSubsetPreparedIntentV2,
) -> Result<(), NisarSubsetPlanningErrorV2> {
    let actual = inventory
        .selected_science()
        .iter()
        .map(|science| science.polarization().to_owned())
        .collect::<BTreeSet<_>>();
    let expected = intent
        .selected_polarizations
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if inventory.selection().prepared_intent_sha256() != intent.canonical_intent_sha256
        || inventory.selection().selected_frequency() != &intent.selected_frequency
        || actual != expected
        || inventory.evidence().prepared_intent_sha256() != intent.canonical_intent_sha256
        || !valid_sha256(inventory.evidence().inventory_sha256())
        || !valid_sha256(inventory.evidence().evidence_sha256())
    {
        return Err(NisarSubsetPlanningErrorV2::InventoryBindingInvalid);
    }
    Ok(())
}

/// Downloader enhancement D1c: `stop` is asked before every selected dataset,
/// window and chunk. A lookup by index (H5Dget_chunk_info) walks the chunk
/// index from its start, so a full-frame NISAR dataset with tens of thousands
/// of chunks took tens of seconds per planning revalidation, during which a
/// pause had no effect (L4 2026-10-18: 28.6 s); since D1d the chunks are
/// visited in one pass (crate::hdf5_chunk_visit).
fn build_physical_execution_plan_v2(
    frozen_plan: &NisarSubsetPlanV2,
    inventory: &NisarSubsetVerifiedMetadataInventoryV2,
    statistics: &NisarSubsetBootstrapStatisticsV2,
    sparse_path: &Path,
    budget_policy: Option<NisarSubsetCombinedBudgetPolicyV2>,
    stop: &dyn Fn() -> bool,
) -> Result<NisarSubsetPhysicalExecutionPlanV2, NisarSubsetPlanningErrorV2> {
    frozen_plan
        .validate()
        .map_err(|_| NisarSubsetPlanningErrorV2::FrozenPlanInvalid)?;
    validate_inventory_against_frozen_plan_v2(inventory, frozen_plan)?;
    let science_root = science_root_for_identity(&frozen_plan.source_product_identity)?;
    statistics.validate_for_science_root(science_root)?;
    let planning_material_ranges = planning_material_ranges_from_statistics_v2(statistics)?;
    if statistics.source_size() != inventory.evidence().source_size()
        || statistics.source_validator_sha256()
            != Some(inventory.evidence().source_validator_sha256())
    {
        return Err(NisarSubsetPlanningErrorV2::InventoryMismatch);
    }
    let file = crate::hdf5_file::open(sparse_path)
        .map_err(|_| NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
    let geolocation =
        read_physical_geolocation_inputs_v2(&file, science_root, &frozen_plan.selected_frequency)?;
    let mut dataset_mappings = Vec::new();
    let mut science_candidates = Vec::new();
    let mut derived_science_bytes = 0_u64;
    let mut derived_mask_cells = BTreeMap::<u16, u64>::new();
    for selected in inventory.selected_science() {
        if stop() {
            return Err(NisarSubsetPlanningErrorV2::ExecutionStopped);
        }
        let dataset_path = selected.source_path().to_owned();
        let dataset = file
            .dataset(&dataset_path)
            .map_err(|_| NisarSubsetPlanningErrorV2::SelectedScienceMissing)?;
        let shape = dataset.shape();
        let chunk = dataset
            .dcpl()
            .map_err(|_| NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?
            .get_chunk()
            .map_err(|_| NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?
            .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
        if shape.len() != 2 || chunk.len() != 2 || shape.contains(&0) || chunk.contains(&0) {
            return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
        }
        let dataset_shape = [shape[0] as u64, shape[1] as u64];
        let chunk_shape = [chunk[0] as u64, chunk[1] as u64];
        let inventory_dataset = inventory
            .datasets()
            .iter()
            .find(|candidate| candidate.path() == dataset_path)
            .ok_or(NisarSubsetPlanningErrorV2::InventoryMismatch)?;
        if inventory_dataset.shape() != dataset_shape
            || !valid_sha256(inventory_dataset.datatype_sha256())
        {
            return Err(NisarSubsetPlanningErrorV2::InventoryMismatch);
        }
        let evidence = insar_provider::asf::nisar_subset::build_live_geolocation_evidence(
            &frozen_plan.effective_geometry.canonical_geojson,
            frozen_plan.selected_frequency.clone(),
            selected.polarization(),
            science_root.swath(),
            &geolocation.grid_zero_doppler_time,
            &geolocation.grid_slant_range,
            &geolocation.swath_zero_doppler_time,
            &geolocation.swath_slant_range,
            geolocation.height_layers_m.clone(),
            geolocation.longitudes.clone(),
            geolocation.latitudes.clone(),
            shape
                .clone()
                .try_into()
                .map_err(|_| NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?,
            chunk
                .clone()
                .try_into()
                .map_err(|_| NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?,
            geolocation.minimum_hae_m,
            geolocation.maximum_hae_m,
        )
        .map_err(|_| NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
        if evidence.windows.len() != usize::from(frozen_plan.effective_geometry.component_count) {
            return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
        }
        let mut windows = Vec::new();
        for window in &evidence.windows {
            let component_sha256 = digest_serializable(&(
                &frozen_plan.canonical_geometry_digest.effective_sha256,
                window.component_id,
            ))?;
            let window_sha256 = digest_serializable(&(
                &component_sha256,
                &dataset_path,
                window.line_start,
                window.line_end_exclusive,
                window.sample_start,
                window.sample_end_exclusive,
                dataset_shape,
                chunk_shape,
            ))?;
            let physical_window = NisarSubsetPhysicalWindowV2 {
                component_id: window.component_id,
                component_sha256,
                window_sha256: window_sha256.clone(),
                line_start: window.line_start,
                line_end_exclusive: window.line_end_exclusive,
                sample_start: window.sample_start,
                sample_end_exclusive: window.sample_end_exclusive,
            };
            let cells = physical_window
                .line_end_exclusive
                .checked_sub(physical_window.line_start)
                .and_then(|lines| {
                    physical_window
                        .sample_end_exclusive
                        .checked_sub(physical_window.sample_start)
                        .and_then(|samples| lines.checked_mul(samples))
                })
                .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
            derived_science_bytes = derived_science_bytes
                .checked_add(
                    cells
                        .checked_mul(8)
                        .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?,
                )
                .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
            derived_mask_cells
                .entry(window.component_id)
                .and_modify(|current| *current = (*current).max(cells))
                .or_insert(cells);
            collect_science_chunk_candidates_v2(
                &dataset,
                &physical_window,
                selected.polarization(),
                &dataset_path,
                chunk_shape,
                inventory.evidence().source_size(),
                &mut science_candidates,
                stop,
            )?;
            windows.push(physical_window);
        }
        windows.sort_by_key(|window| window.component_id);
        dataset_mappings.push(NisarSubsetPhysicalDatasetMappingV2 {
            polarization: selected.polarization().to_owned(),
            source_dataset_path: dataset_path,
            dataset_shape,
            chunk_shape,
            datatype_sha256: inventory_dataset.datatype_sha256().to_owned(),
            windows,
        });
    }
    dataset_mappings.sort_by(|left, right| left.polarization.cmp(&right.polarization));
    let exact_selected_science_dataset_paths = dataset_mappings
        .iter()
        .map(|mapping| mapping.source_dataset_path.clone())
        .collect::<Vec<_>>();
    let science_ranges = canonicalize_physical_candidates_v2(
        science_candidates,
        inventory.evidence().source_size(),
        execution_range_limit_v2(budget_policy.as_ref()),
    )?;
    if science_ranges.is_empty() {
        return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
    }
    let selected_windows = dataset_mappings
        .iter()
        .flat_map(|mapping| mapping.windows.iter())
        .collect::<Vec<_>>();
    let mut dependency_candidates = Vec::new();
    let mut non_dataset_dependencies = Vec::new();
    for dependency in &frozen_plan.required_dependencies.items {
        if dependency.kind == NisarSubsetDependencyKindV2::SelectedScienceDataset {
            continue;
        }
        if dependency.kind == NisarSubsetDependencyKindV2::Attribute {
            if dependency.source_path != "/" {
                return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
            }
            let root = file
                .group("/")
                .map_err(|_| NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
            let attribute_names = frozen_plan
                .source_dependency_graph
                .nodes
                .iter()
                .filter(|node| {
                    node.kind == NisarSubsetSourceDependencyNodeKindV2::Attribute
                        && node.source_path == dependency.source_path
                })
                .filter_map(|node| node.attribute_name.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            if attribute_names.is_empty()
                || attribute_names.iter().any(|name| root.attr(name).is_err())
            {
                return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
            }
            non_dataset_dependencies.push(NisarSubsetPhysicalNonDatasetDependencyV2 {
                source_path: dependency.source_path.clone(),
                attribute_names,
            });
            continue;
        }
        let dataset = file
            .dataset(&dependency.source_path)
            .map_err(|_| NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
        let physical_ranges = if dependency.kind == NisarSubsetDependencyKindV2::ValidityMask {
            physical_dataset_ranges_for_windows_v2(
                &dataset,
                &selected_windows,
                statistics.source_size(),
            )?
        } else {
            physical_dataset_ranges_v2(&dataset)?
        };
        if physical_ranges.is_empty() {
            return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
        }
        let mut transfer_ranges = physical_ranges
            .iter()
            .copied()
            .filter(|range| !range_fully_materialized_v2(*range, &planning_material_ranges))
            .collect::<Vec<_>>();
        if transfer_ranges.is_empty() {
            // Keep one owner/path witness in the physical plan even when the
            // planning prefix already materialized every required byte. The
            // execution union will charge this bounded witness, while the
            // writer still verifies the full dependency from the sparse part.
            transfer_ranges.push(physical_ranges[0]);
        }
        for (start, end_exclusive) in transfer_ranges {
            dependency_candidates.push((
                start,
                end_exclusive,
                NisarSubsetPhysicalLogicalOwnerV2 {
                    dataset_path: dependency.source_path.clone(),
                    polarization: None,
                    component_id: None,
                    window_sha256: None,
                    logical_chunk: None,
                },
            ));
        }
    }
    non_dataset_dependencies.sort_by(|left, right| left.source_path.cmp(&right.source_path));
    non_dataset_dependencies.dedup();
    let dependency_ranges = canonicalize_physical_candidates_v2(
        dependency_candidates,
        inventory.evidence().source_size(),
        NISAR_SUBSET_PHYSICAL_MAX_DEPENDENCY_RANGES,
    )?;
    let execution_transfer_ranges = canonical_execution_transfer_ranges_with_policy_v2(
        &dependency_ranges,
        &science_ranges,
        inventory.evidence().source_size(),
        budget_policy.as_ref(),
    )?;
    let estimated_science_response_bytes =
        science_ranges.iter().try_fold(0_u64, |total, range| {
            total
                .checked_add(range.len()?)
                .ok_or(NisarSubsetPlanningErrorV2::CombinedBudgetExceeded)
        })?;
    let estimated_execution_transfer_response_bytes =
        execution_transfer_ranges
            .iter()
            .try_fold(0_u64, |total, range| {
                total
                    .checked_add(range.len()?)
                    .ok_or(NisarSubsetPlanningErrorV2::CombinedBudgetExceeded)
            })?;
    let combined_planning_request_count = statistics
        .total_planning_request_count()
        .checked_add(statistics.charged_failure_requests())
        .ok_or(NisarSubsetPlanningErrorV2::CombinedBudgetExceeded)?;
    let combined_planning_response_body_bytes = statistics
        .total_planning_response_body_bytes()
        .checked_add(statistics.charged_failure_response_bytes())
        .ok_or(NisarSubsetPlanningErrorV2::CombinedBudgetExceeded)?;
    let combined_budget = project_combined_budget_with_policy_v2(
        combined_planning_request_count,
        combined_planning_response_body_bytes,
        execution_transfer_ranges.len() as u64,
        estimated_execution_transfer_response_bytes,
        inventory.evidence().source_size(),
        total_request_limit_v2(science_root)?,
        budget_policy,
    )?;
    let estimated_derived_artifact_bytes =
        derived_mask_cells
            .values()
            .try_fold(derived_science_bytes, |total, cells| {
                total
                    .checked_add(*cells)
                    .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)
            })?;
    let mut physical_plan = NisarSubsetPhysicalExecutionPlanV2 {
        physical_plan_contract: NISAR_SUBSET_PHYSICAL_EXECUTION_PLAN_V2_CONTRACT.to_owned(),
        schema_version: NISAR_SUBSET_PHYSICAL_EXECUTION_PLAN_V2_SCHEMA_VERSION,
        prepared_intent_sha256: frozen_plan.prepared_intent_sha256.clone(),
        canonical_semantic_plan_digest: frozen_plan.canonical_semantic_plan_digest.clone(),
        source_product_identity: frozen_plan.source_product_identity.clone(),
        source_asset_identity: frozen_plan.source_asset_identity.clone(),
        source_validator: inventory.evidence().source_validator().clone(),
        source_validator_sha256: inventory.evidence().source_validator_sha256().to_owned(),
        source_size: inventory.evidence().source_size(),
        selected_frequency: frozen_plan.selected_frequency.clone(),
        selected_polarizations: frozen_plan.selected_polarizations.clone(),
        effective_geometry_sha256: frozen_plan
            .canonical_geometry_digest
            .effective_sha256
            .clone(),
        inventory_sha256: inventory.evidence().inventory_sha256().to_owned(),
        inventory_evidence_sha256: inventory.evidence().evidence_sha256().to_owned(),
        source_graph_sha256: frozen_plan
            .required_dependencies
            .source_graph_sha256
            .clone(),
        exact_selected_science_dataset_paths,
        dataset_mappings,
        dependency_ranges,
        non_dataset_dependencies,
        science_ranges,
        combined_budget,
        estimated_science_response_bytes,
        estimated_derived_artifact_bytes,
        physical_plan_sha256: String::new(),
    };
    physical_plan.physical_plan_sha256 = physical_plan.recompute_physical_plan_sha256()?;
    physical_plan.validate_binding(frozen_plan)?;
    Ok(physical_plan)
}

struct NisarSubsetPhysicalGeolocationInputsV2 {
    grid_zero_doppler_time: Vec<f64>,
    grid_slant_range: Vec<f64>,
    swath_zero_doppler_time: Vec<f64>,
    swath_slant_range: Vec<f64>,
    height_layers_m: Vec<f64>,
    longitudes: Vec<Vec<f64>>,
    latitudes: Vec<Vec<f64>>,
    minimum_hae_m: f64,
    maximum_hae_m: f64,
}

fn read_physical_geolocation_inputs_v2(
    file: &hdf5::File,
    science_root: NisarScienceRoot,
    frequency: &NisarSubsetFrequency,
) -> Result<NisarSubsetPhysicalGeolocationInputsV2, NisarSubsetPlanningErrorV2> {
    let group = file
        .group(&science_root.geolocation_path())
        .map_err(|_| NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
    let height_layers_m: Vec<f64> = group
        .dataset("heightAboveEllipsoid")
        .and_then(|dataset| dataset.read_raw())
        .map_err(|_| NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
    let coordinate_x = group
        .dataset("coordinateX")
        .map_err(|_| NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
    let coordinate_y = group
        .dataset("coordinateY")
        .map_err(|_| NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
    let shape = coordinate_x.shape();
    let x_values: Vec<f64> = coordinate_x
        .read_raw()
        .map_err(|_| NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
    let y_values: Vec<f64> = coordinate_y
        .read_raw()
        .map_err(|_| NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
    if shape.len() != 3
        || shape[0] != height_layers_m.len()
        || shape[1] < 2
        || shape[2] < 2
        || coordinate_y.shape() != shape
        || x_values.len() != y_values.len()
        || x_values.len() != shape[0].saturating_mul(shape[1]).saturating_mul(shape[2])
        || height_layers_m.is_empty()
    {
        return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
    }
    let stride = shape[1]
        .checked_mul(shape[2])
        .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
    let mut longitudes = Vec::with_capacity(shape[0]);
    let mut latitudes = Vec::with_capacity(shape[0]);
    for layer in 0..shape[0] {
        let start = layer
            .checked_mul(stride)
            .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
        longitudes.push(x_values[start..start + stride].to_vec());
        latitudes.push(y_values[start..start + stride].to_vec());
    }
    let frequency_label = match frequency {
        NisarSubsetFrequency::A => "A",
        NisarSubsetFrequency::B => "B",
    };
    let read_vector = |path: &str| {
        file.dataset(path)
            .and_then(|dataset| dataset.read_raw::<f64>())
            .map_err(|_| NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)
    };
    let minimum_hae_m = height_layers_m
        .iter()
        .copied()
        .fold(f64::INFINITY, f64::min);
    let maximum_hae_m = height_layers_m
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    if !minimum_hae_m.is_finite() || !maximum_hae_m.is_finite() {
        return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
    }
    Ok(NisarSubsetPhysicalGeolocationInputsV2 {
        grid_zero_doppler_time: read_vector(&format!(
            "{}/zeroDopplerTime",
            science_root.geolocation_path()
        ))?,
        grid_slant_range: read_vector(&format!("{}/slantRange", science_root.geolocation_path()))?,
        swath_zero_doppler_time: read_vector(&science_root.zero_doppler_time_path())?,
        swath_slant_range: read_vector(&science_root.slant_range_path(frequency_label))?,
        height_layers_m,
        longitudes,
        latitudes,
        minimum_hae_m,
        maximum_hae_m,
    })
}

#[allow(clippy::too_many_arguments)]
fn collect_science_chunk_candidates_v2(
    dataset: &hdf5::Dataset,
    window: &NisarSubsetPhysicalWindowV2,
    polarization: &str,
    dataset_path: &str,
    chunk_shape: [u64; 2],
    source_size: u64,
    output: &mut Vec<(u64, u64, NisarSubsetPhysicalLogicalOwnerV2)>,
    stop: &dyn Fn() -> bool,
) -> Result<(), NisarSubsetPlanningErrorV2> {
    // Downloader enhancement D1d: one pass over the chunk index instead of one
    // lookup from its start per chunk; `stop` is still asked before each chunk.
    crate::hdf5_chunk_visit::visit_stored_chunks(dataset, |chunk| {
        #[cfg(test)]
        crate::hdf5_range_transfer::long_step_hook_for_test::step();
        if stop() {
            return Err(NisarSubsetPlanningErrorV2::ExecutionStopped);
        }
        if chunk.offset.len() != 2
            || chunk.filter_mask != 0
            || chunk.address == u64::MAX
            || chunk.size == 0
        {
            return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
        }
        let line_start = chunk.offset[0];
        let sample_start = chunk.offset[1];
        let line_end = line_start
            .checked_add(chunk_shape[0])
            .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
        let sample_end = sample_start
            .checked_add(chunk_shape[1])
            .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
        if line_start >= window.line_end_exclusive
            || line_end <= window.line_start
            || sample_start >= window.sample_end_exclusive
            || sample_end <= window.sample_start
        {
            return Ok(());
        }
        let end_exclusive = chunk
            .address
            .checked_add(chunk.size)
            .filter(|end| *end <= source_size)
            .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
        output.push((
            chunk.address,
            end_exclusive,
            NisarSubsetPhysicalLogicalOwnerV2 {
                dataset_path: dataset_path.to_owned(),
                polarization: Some(polarization.to_owned()),
                component_id: Some(window.component_id),
                window_sha256: Some(window.window_sha256.clone()),
                logical_chunk: Some([line_start / chunk_shape[0], sample_start / chunk_shape[1]]),
            },
        ));
        Ok(())
    })
    .map(|_| ())
    .map_err(|error| match error {
        crate::hdf5_chunk_visit::ChunkVisitError::Ended(error) => error,
        _ => NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable,
    })
}

fn physical_dataset_ranges_v2(
    dataset: &hdf5::Dataset,
) -> Result<Vec<(u64, u64)>, NisarSubsetPlanningErrorV2> {
    let mut ranges = Vec::new();
    // Downloader enhancement D1d: one pass over the chunk index. A chunked
    // dataset contributes its stored chunks (none when nothing is stored); an
    // iteration failure fails closed.
    match crate::hdf5_chunk_visit::visit_stored_chunks(dataset, |chunk| {
        if chunk.address == u64::MAX {
            return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
        }
        if chunk.size > 0 {
            ranges.push((
                chunk.address,
                chunk
                    .address
                    .checked_add(chunk.size)
                    .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?,
            ));
        }
        Ok(())
    }) {
        Ok(_) => {}
        Err(crate::hdf5_chunk_visit::ChunkVisitError::Ended(error)) => return Err(error),
        Err(crate::hdf5_chunk_visit::ChunkVisitError::Hdf5) => {
            return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)
        }
        Err(crate::hdf5_chunk_visit::ChunkVisitError::NotChunked) => {
            let offset = hdf5::sync::sync(|| unsafe { hdf5_sys::h5d::H5Dget_offset(dataset.id()) });
            let storage =
                hdf5::sync::sync(|| unsafe { hdf5_sys::h5d::H5Dget_storage_size(dataset.id()) });
            if offset != u64::MAX && storage > 0 {
                ranges.push((
                    offset,
                    offset
                        .checked_add(storage)
                        .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?,
                ));
            }
        }
    }
    Ok(ranges)
}

fn physical_dataset_ranges_for_windows_v2(
    dataset: &hdf5::Dataset,
    windows: &[&NisarSubsetPhysicalWindowV2],
    source_size: u64,
) -> Result<Vec<(u64, u64)>, NisarSubsetPlanningErrorV2> {
    if windows.is_empty() {
        return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
    }
    let shape = dataset.shape();
    if shape.len() != 2 || shape.contains(&0) {
        return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
    }
    let chunk = dataset
        .dcpl()
        .map_err(|_| NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?
        .get_chunk()
        .map_err(|_| NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
    let mut ranges = Vec::new();
    if let Some(chunk) = chunk {
        if chunk.len() != 2 || chunk.contains(&0) {
            return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
        }
        let chunk_shape = [chunk[0] as u64, chunk[1] as u64];
        // Downloader enhancement D1d: one pass over the chunk index.
        let visited = crate::hdf5_chunk_visit::visit_stored_chunks(dataset, |chunk| {
            if chunk.offset.len() != 2
                || chunk.filter_mask != 0
                || chunk.address == u64::MAX
                || chunk.size == 0
            {
                return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
            }
            let (line_start, sample_start) = (chunk.offset[0], chunk.offset[1]);
            let line_end = line_start
                .checked_add(chunk_shape[0])
                .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
            let sample_end = sample_start
                .checked_add(chunk_shape[1])
                .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
            if !windows.iter().any(|window| {
                line_start < window.line_end_exclusive
                    && line_end > window.line_start
                    && sample_start < window.sample_end_exclusive
                    && sample_end > window.sample_start
            }) {
                return Ok(());
            }
            let end = chunk
                .address
                .checked_add(chunk.size)
                .filter(|end| *end <= source_size)
                .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
            ranges.push((chunk.address, end));
            Ok(())
        })
        .map_err(|error| match error {
            crate::hdf5_chunk_visit::ChunkVisitError::Ended(error) => error,
            _ => NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable,
        })?;
        if visited == 0 {
            return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
        }
    } else {
        let rows = u64::try_from(shape[0])
            .map_err(|_| NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
        let columns = u64::try_from(shape[1])
            .map_err(|_| NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
        let element_size = u64::try_from(
            dataset
                .dtype()
                .map_err(|_| NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?
                .size(),
        )
        .map_err(|_| NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
        if element_size == 0 {
            return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
        }
        let offset = hdf5::sync::sync(|| unsafe { hdf5_sys::h5d::H5Dget_offset(dataset.id()) });
        let storage =
            hdf5::sync::sync(|| unsafe { hdf5_sys::h5d::H5Dget_storage_size(dataset.id()) });
        let expected_storage = rows
            .checked_mul(columns)
            .and_then(|cells| cells.checked_mul(element_size))
            .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
        let storage_end = offset
            .checked_add(storage)
            .filter(|end| *end <= source_size)
            .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
        if offset == u64::MAX || storage == 0 || storage != expected_storage {
            return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
        }
        for window in windows {
            if window.line_start >= window.line_end_exclusive
                || window.sample_start >= window.sample_end_exclusive
                || window.line_end_exclusive > rows
                || window.sample_end_exclusive > columns
            {
                return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
            }
            let first_cell = window
                .line_start
                .checked_mul(columns)
                .and_then(|value| value.checked_add(window.sample_start))
                .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
            let last_line = window
                .line_end_exclusive
                .checked_sub(1)
                .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
            let end_cell = last_line
                .checked_mul(columns)
                .and_then(|value| value.checked_add(window.sample_end_exclusive))
                .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
            let start = offset
                .checked_add(
                    first_cell
                        .checked_mul(element_size)
                        .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?,
                )
                .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
            let end = offset
                .checked_add(
                    end_cell
                        .checked_mul(element_size)
                        .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?,
                )
                .filter(|end| *end <= storage_end)
                .ok_or(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable)?;
            if start >= end {
                return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
            }
            ranges.push((start, end));
        }
        ranges.sort_unstable();
        let mut merged = Vec::<(u64, u64)>::new();
        for (start, end) in ranges {
            if let Some(last) = merged.last_mut() {
                if start <= last.1 {
                    last.1 = last.1.max(end);
                    continue;
                }
            }
            merged.push((start, end));
        }
        ranges = merged;
    }
    if ranges.is_empty() {
        return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
    }
    Ok(ranges)
}

fn range_fully_materialized_v2(range: (u64, u64), material_ranges: &[(u64, u64)]) -> bool {
    material_ranges
        .iter()
        .any(|(start, end)| *start <= range.0 && *end >= range.1)
}

fn canonicalize_physical_candidates_v2(
    candidates: Vec<(u64, u64, NisarSubsetPhysicalLogicalOwnerV2)>,
    source_size: u64,
    maximum_ranges: usize,
) -> Result<Vec<NisarSubsetPhysicalRangeV2>, NisarSubsetPlanningErrorV2> {
    let mut grouped = BTreeMap::<(u64, u64), Vec<NisarSubsetPhysicalLogicalOwnerV2>>::new();
    for (start, end_exclusive, owner) in candidates {
        if start >= end_exclusive || end_exclusive > source_size || owner.dataset_path.is_empty() {
            return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
        }
        if end_exclusive - start > NISAR_LIVE_MAX_RESPONSE_BODY_BYTES {
            return Err(NisarSubsetPlanningErrorV2::CombinedBudgetExceeded);
        }
        grouped
            .entry((start, end_exclusive))
            .or_default()
            .push(owner);
    }
    if grouped.len() > maximum_ranges {
        return Err(NisarSubsetPlanningErrorV2::CombinedBudgetExceeded);
    }
    let keys = grouped.keys().copied().collect::<Vec<_>>();
    if keys
        .windows(2)
        .any(|pair| pair[0].1 > pair[1].0 && pair[0] != pair[1])
    {
        return Err(NisarSubsetPlanningErrorV2::PhysicalMappingUnavailable);
    }
    Ok(grouped
        .into_iter()
        .map(|((start, end_exclusive), mut logical_owners)| {
            logical_owners.sort();
            logical_owners.dedup();
            NisarSubsetPhysicalRangeV2 {
                start,
                end_exclusive,
                logical_owners,
            }
        })
        .collect())
}

#[cfg(test)]
fn canonical_execution_transfer_ranges_v2(
    dependency_ranges: &[NisarSubsetPhysicalRangeV2],
    science_ranges: &[NisarSubsetPhysicalRangeV2],
    source_size: u64,
) -> Result<Vec<NisarSubsetPhysicalRangeV2>, NisarSubsetPlanningErrorV2> {
    canonical_execution_transfer_ranges_with_policy_v2(
        dependency_ranges,
        science_ranges,
        source_size,
        Some(&NisarSubsetCombinedBudgetPolicyV2::PhysicalExecutionUnionV1),
    )
}

fn canonical_execution_transfer_ranges_with_policy_v2(
    dependency_ranges: &[NisarSubsetPhysicalRangeV2],
    science_ranges: &[NisarSubsetPhysicalRangeV2],
    source_size: u64,
    budget_policy: Option<&NisarSubsetCombinedBudgetPolicyV2>,
) -> Result<Vec<NisarSubsetPhysicalRangeV2>, NisarSubsetPlanningErrorV2> {
    let prefix_is_materialized = source_size >= crate::NISAR_LIVE_METADATA_PREFIX_BYTES;
    let candidates = dependency_ranges
        .iter()
        .filter(|range| {
            !prefix_is_materialized || range.end_exclusive > crate::NISAR_LIVE_METADATA_PREFIX_BYTES
        })
        .chain(science_ranges)
        .flat_map(|range| {
            range
                .logical_owners
                .iter()
                .cloned()
                .map(|owner| (range.start, range.end_exclusive, owner))
        })
        .collect();
    canonicalize_physical_candidates_v2(
        candidates,
        source_size,
        execution_range_limit_v2(budget_policy),
    )
}

fn execution_range_limit_v2(budget_policy: Option<&NisarSubsetCombinedBudgetPolicyV2>) -> usize {
    match budget_policy {
        Some(
            NisarSubsetCombinedBudgetPolicyV2::PhysicalExecutionUnionV1
            | NisarSubsetCombinedBudgetPolicyV2::PhysicalExecutionUnionV2,
        ) => NISAR_SUBSET_PHYSICAL_MAX_EXECUTION_RANGES_V2,
        None => NISAR_LIVE_MAX_REQUESTS as usize,
    }
}

fn validate_physical_ranges(
    ranges: &[NisarSubsetPhysicalRangeV2],
    source_size: u64,
    maximum_ranges: usize,
) -> Result<(), NisarSubsetPlanningErrorV2> {
    if ranges.len() > maximum_ranges
        || ranges.iter().any(|range| {
            range.start >= range.end_exclusive
                || range.end_exclusive > source_size
                || range.end_exclusive - range.start > NISAR_LIVE_MAX_RESPONSE_BODY_BYTES
                || range.logical_owners.is_empty()
                || range
                    .logical_owners
                    .windows(2)
                    .any(|pair| pair[0] >= pair[1])
                || range.logical_owners.iter().any(|owner| {
                    owner.dataset_path.is_empty()
                        || owner
                            .window_sha256
                            .as_deref()
                            .is_some_and(|digest| !valid_sha256(digest))
                })
        })
        || ranges
            .windows(2)
            .any(|pair| pair[0].start >= pair[1].start || pair[0].end_exclusive > pair[1].start)
    {
        return Err(NisarSubsetPlanningErrorV2::PhysicalPlanInvalid);
    }
    Ok(())
}

fn validate_science_owner_coverage_v2(
    mappings: &[NisarSubsetPhysicalDatasetMappingV2],
    science_ranges: &[NisarSubsetPhysicalRangeV2],
) -> Result<(), NisarSubsetPlanningErrorV2> {
    if mappings.is_empty() || science_ranges.is_empty() {
        return Err(NisarSubsetPlanningErrorV2::PhysicalPlanInvalid);
    }
    let mut actual_owners = science_ranges
        .iter()
        .flat_map(|range| range.logical_owners.iter().cloned())
        .collect::<Vec<_>>();
    if actual_owners.is_empty()
        || actual_owners.iter().any(|owner| {
            owner.polarization.is_none()
                || owner.component_id.is_none()
                || owner.window_sha256.is_none()
                || owner.logical_chunk.is_none()
        })
    {
        return Err(NisarSubsetPlanningErrorV2::PhysicalPlanInvalid);
    }
    actual_owners.sort();
    if actual_owners.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(NisarSubsetPlanningErrorV2::PhysicalPlanInvalid);
    }

    let mut expected_count = 0_u64;
    for mapping in mappings {
        for window in &mapping.windows {
            let expected_window_sha256 = digest_serializable(&(
                &window.component_sha256,
                &mapping.source_dataset_path,
                window.line_start,
                window.line_end_exclusive,
                window.sample_start,
                window.sample_end_exclusive,
                mapping.dataset_shape,
                mapping.chunk_shape,
            ))?;
            if window.window_sha256 != expected_window_sha256 {
                return Err(NisarSubsetPlanningErrorV2::PhysicalPlanInvalid);
            }
            let line_first = window.line_start / mapping.chunk_shape[0];
            let line_last = (window.line_end_exclusive - 1) / mapping.chunk_shape[0];
            let sample_first = window.sample_start / mapping.chunk_shape[1];
            let sample_last = (window.sample_end_exclusive - 1) / mapping.chunk_shape[1];
            let line_count = line_last
                .checked_sub(line_first)
                .and_then(|count| count.checked_add(1))
                .ok_or(NisarSubsetPlanningErrorV2::PhysicalPlanInvalid)?;
            let sample_count = sample_last
                .checked_sub(sample_first)
                .and_then(|count| count.checked_add(1))
                .ok_or(NisarSubsetPlanningErrorV2::PhysicalPlanInvalid)?;
            expected_count = expected_count
                .checked_add(
                    line_count
                        .checked_mul(sample_count)
                        .ok_or(NisarSubsetPlanningErrorV2::PhysicalPlanInvalid)?,
                )
                .ok_or(NisarSubsetPlanningErrorV2::PhysicalPlanInvalid)?;
        }
    }
    if usize::try_from(expected_count).ok() != Some(actual_owners.len()) {
        return Err(NisarSubsetPlanningErrorV2::PhysicalPlanInvalid);
    }

    let mut expected_owners = BTreeSet::new();
    for mapping in mappings {
        for window in &mapping.windows {
            let line_first = window.line_start / mapping.chunk_shape[0];
            let line_last = (window.line_end_exclusive - 1) / mapping.chunk_shape[0];
            let sample_first = window.sample_start / mapping.chunk_shape[1];
            let sample_last = (window.sample_end_exclusive - 1) / mapping.chunk_shape[1];
            for line in line_first..=line_last {
                for sample in sample_first..=sample_last {
                    expected_owners.insert(NisarSubsetPhysicalLogicalOwnerV2 {
                        dataset_path: mapping.source_dataset_path.clone(),
                        polarization: Some(mapping.polarization.clone()),
                        component_id: Some(window.component_id),
                        window_sha256: Some(window.window_sha256.clone()),
                        logical_chunk: Some([line, sample]),
                    });
                }
            }
        }
    }
    if expected_owners.len() != actual_owners.len()
        || expected_owners.into_iter().collect::<Vec<_>>() != actual_owners
    {
        return Err(NisarSubsetPlanningErrorV2::PhysicalPlanInvalid);
    }
    Ok(())
}

#[cfg(test)]
fn project_combined_budget_v2(
    planning_request_count: u64,
    planning_response_body_bytes: u64,
    science_request_count: u64,
    science_response_body_bytes: u64,
    source_size: u64,
    max_request_count: u64,
) -> Result<NisarSubsetCombinedBudgetProjectionV2, NisarSubsetPlanningErrorV2> {
    project_combined_budget_with_policy_v2(
        planning_request_count,
        planning_response_body_bytes,
        science_request_count,
        science_response_body_bytes,
        source_size,
        max_request_count,
        Some(NisarSubsetCombinedBudgetPolicyV2::PhysicalExecutionUnionV1),
    )
}

#[allow(clippy::too_many_arguments)]
fn project_combined_budget_with_policy_v2(
    planning_request_count: u64,
    planning_response_body_bytes: u64,
    science_request_count: u64,
    science_response_body_bytes: u64,
    source_size: u64,
    legacy_max_request_count: u64,
    budget_policy: Option<NisarSubsetCombinedBudgetPolicyV2>,
) -> Result<NisarSubsetCombinedBudgetProjectionV2, NisarSubsetPlanningErrorV2> {
    let (max_request_count, max_response_body_bytes) = combined_budget_limits_v2(
        budget_policy.as_ref(),
        planning_request_count,
        planning_response_body_bytes,
        science_request_count,
        science_response_body_bytes,
        source_size,
        legacy_max_request_count,
    )?;
    let remaining_science_request_count = max_request_count
        .checked_sub(planning_request_count)
        .ok_or(NisarSubsetPlanningErrorV2::CombinedBudgetExceeded)?;
    let remaining_science_response_body_bytes = max_response_body_bytes
        .checked_sub(planning_response_body_bytes)
        .ok_or(NisarSubsetPlanningErrorV2::CombinedBudgetExceeded)?;
    let max_science_request_count = match budget_policy {
        Some(
            NisarSubsetCombinedBudgetPolicyV2::PhysicalExecutionUnionV1
            | NisarSubsetCombinedBudgetPolicyV2::PhysicalExecutionUnionV2,
        ) => remaining_science_request_count,
        None => remaining_science_request_count.min(NISAR_LIVE_MAX_REQUESTS),
    };
    if science_request_count > max_science_request_count
        || science_request_count > remaining_science_request_count
        || science_response_body_bytes > remaining_science_response_body_bytes
    {
        return Err(NisarSubsetPlanningErrorV2::CombinedBudgetExceeded);
    }
    Ok(NisarSubsetCombinedBudgetProjectionV2 {
        max_request_count,
        max_response_body_bytes,
        committed_planning_request_count: planning_request_count,
        committed_planning_response_body_bytes: planning_response_body_bytes,
        projected_science_request_count: science_request_count,
        projected_science_response_body_bytes: science_response_body_bytes,
        remaining_science_request_count,
        remaining_science_response_body_bytes,
        budget_policy,
    })
}

#[allow(clippy::too_many_arguments)]
fn combined_budget_limits_v2(
    budget_policy: Option<&NisarSubsetCombinedBudgetPolicyV2>,
    planning_request_count: u64,
    planning_response_body_bytes: u64,
    science_request_count: u64,
    science_response_body_bytes: u64,
    source_size: u64,
    legacy_max_request_count: u64,
) -> Result<(u64, u64), NisarSubsetPlanningErrorV2> {
    match budget_policy {
        Some(policy) => {
            if science_request_count == 0
                || science_request_count > NISAR_SUBSET_PHYSICAL_MAX_EXECUTION_RANGES_V2 as u64
                || science_response_body_bytes == 0
                || science_response_body_bytes > source_size
            {
                return Err(NisarSubsetPlanningErrorV2::CombinedBudgetExceeded);
            }
            let request_multiplier = match policy {
                NisarSubsetCombinedBudgetPolicyV2::PhysicalExecutionUnionV1 => 2,
                NisarSubsetCombinedBudgetPolicyV2::PhysicalExecutionUnionV2 => {
                    PHYSICAL_EXECUTION_UNION_V2_REQUESTS_PER_RANGE
                }
            };
            let science_request_budget = science_request_count
                .checked_mul(request_multiplier)
                .ok_or(NisarSubsetPlanningErrorV2::CombinedBudgetExceeded)?;
            let science_response_body_budget = science_response_body_bytes
                .checked_mul(2)
                .ok_or(NisarSubsetPlanningErrorV2::CombinedBudgetExceeded)?;
            Ok((
                planning_request_count
                    .checked_add(science_request_budget)
                    .ok_or(NisarSubsetPlanningErrorV2::CombinedBudgetExceeded)?,
                planning_response_body_bytes
                    .checked_add(science_response_body_budget)
                    .ok_or(NisarSubsetPlanningErrorV2::CombinedBudgetExceeded)?,
            ))
        }
        None => Ok((
            legacy_max_request_count,
            nisar_live_budget_response_limit(source_size)
                .map_err(|_| NisarSubsetPlanningErrorV2::CombinedBudgetExceeded)?,
        )),
    }
}

fn validate_inventory_against_frozen_plan_v2(
    inventory: &NisarSubsetVerifiedMetadataInventoryV2,
    frozen_plan: &NisarSubsetPlanV2,
) -> Result<(), NisarSubsetPlanningErrorV2> {
    let actual = inventory
        .selected_science()
        .iter()
        .map(|science| science.polarization().to_owned())
        .collect::<Vec<_>>();
    if actual != frozen_plan.selected_polarizations
        || inventory.evidence().prepared_intent_sha256() != frozen_plan.prepared_intent_sha256
        || inventory.evidence().evidence_sha256()
            != frozen_plan.source_dependency_graph.evidence_sha256
        || inventory.evidence().source_size()
            != frozen_plan
                .source_estimate
                .as_ref()
                .map(|estimate| estimate.bytes)
                .unwrap_or_default()
    {
        return Err(NisarSubsetPlanningErrorV2::InventoryMismatch);
    }
    Ok(())
}

fn parse_planning_workspace_checkpoint_v2(
    text: &str,
) -> Result<NisarSubsetPlanningWorkspaceCheckpointV2, NisarSubsetPlanningErrorV2> {
    let checkpoint = serde_json::from_str::<NisarSubsetPlanningWorkspaceCheckpointV2>(text)
        .map_err(|_| NisarSubsetPlanningErrorV2::RecoveryInvalid)?;
    if checkpoint.checkpoint_contract != NISAR_SUBSET_PLANNING_WORKSPACE_CHECKPOINT_V2_CONTRACT
        || checkpoint.schema_version != NISAR_SUBSET_PLANNING_WORKSPACE_CHECKPOINT_V2_SCHEMA_VERSION
        || !valid_sha256(&checkpoint.checkpoint_sha256)
        || !valid_sha256(&checkpoint.sparse_part_identity_sha256)
        || !valid_sha256(&checkpoint.sparse_part_material_ranges_sha256)
        || !valid_sha256(&checkpoint.sparse_part_material_sha256)
        || checkpoint.recompute_sha256()? != checkpoint.checkpoint_sha256
        || checkpoint
            .bootstrap_statistics
            .validate_checkpoint()
            .is_err()
        || checkpoint.physical_plan.validate().is_err()
        || checkpoint.bootstrap_statistics.source_size != checkpoint.source_size
        || checkpoint
            .bootstrap_statistics
            .source_validator_sha256
            .as_deref()
            != Some(checkpoint.source_validator_sha256.as_str())
        || checkpoint.physical_plan.source_size != checkpoint.source_size
        || checkpoint.physical_plan.source_validator_sha256 != checkpoint.source_validator_sha256
        || checkpoint.physical_plan.inventory_sha256 != checkpoint.inventory_sha256
        || checkpoint.physical_plan.inventory_evidence_sha256
            != checkpoint.inventory_evidence_sha256
        || checkpoint
            .bootstrap_statistics
            .total_planning_request_count
            .checked_add(checkpoint.bootstrap_statistics.charged_failure_requests)
            != Some(
                checkpoint
                    .physical_plan
                    .combined_budget
                    .committed_planning_request_count,
            )
        || checkpoint
            .bootstrap_statistics
            .total_planning_response_body_bytes
            .checked_add(
                checkpoint
                    .bootstrap_statistics
                    .charged_failure_response_bytes,
            )
            != Some(
                checkpoint
                    .physical_plan
                    .combined_budget
                    .committed_planning_response_body_bytes,
            )
    {
        return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
    }
    Ok(checkpoint)
}

fn map_workspace_load_error(error: crate::Hdf5RangeTransferError) -> NisarSubsetPlanningErrorV2 {
    match error {
        crate::Hdf5RangeTransferError::WorkspaceInvalid => {
            NisarSubsetPlanningErrorV2::PlanningWorkspaceIdentityMismatch
        }
        // Downloader enhancement D1c: a stop during the planning material
        // digest is a stop, never invalid recovery evidence.
        crate::Hdf5RangeTransferError::Cancelled => NisarSubsetPlanningErrorV2::ExecutionStopped,
        _ => NisarSubsetPlanningErrorV2::RecoveryInvalid,
    }
}

fn map_live_inventory_error(
    error: NisarSubsetWriteError,
    fallback: NisarSubsetPlanningErrorV2,
) -> NisarSubsetPlanningErrorV2 {
    if error == NisarSubsetWriteError::Cancelled {
        NisarSubsetPlanningErrorV2::ExecutionStopped
    } else {
        fallback
    }
}

fn science_root_for_identity(
    identity: &NisarSubsetSourceProductIdentityV2,
) -> Result<NisarScienceRoot, NisarSubsetPlanningErrorV2> {
    NisarScienceRoot::from_source_identity(identity)
        .ok_or(NisarSubsetPlanningErrorV2::PreparedIntentInvalid)
}

fn selected_science_paths(
    intent: &NisarSubsetPreparedIntentV2,
) -> Result<(NisarScienceRoot, Vec<String>), NisarSubsetPlanningErrorV2> {
    let science_root = science_root_for_identity(&intent.source_product_identity)?;
    Ok((
        science_root,
        selected_science_paths_from_selection(
            science_root,
            &intent.selected_frequency,
            &intent.selected_polarizations,
        ),
    ))
}

fn selected_science_paths_from_selection(
    science_root: NisarScienceRoot,
    frequency: &NisarSubsetFrequency,
    polarizations: &[String],
) -> Vec<String> {
    let frequency = match frequency {
        NisarSubsetFrequency::A => "A",
        NisarSubsetFrequency::B => "B",
    };
    polarizations
        .iter()
        .map(|polarization| science_root.selected_science_path(frequency, polarization))
        .collect()
}

fn complement_ranges(
    excluded: &[(u64, u64)],
    extent: u64,
) -> Result<Vec<(u64, u64)>, NisarSubsetPlanningErrorV2> {
    let mut cursor = 0_u64;
    let mut output = Vec::new();
    for &(start, end) in excluded {
        if start >= end || end > extent || start < cursor {
            return Err(NisarSubsetPlanningErrorV2::BootstrapStatisticsInvalid);
        }
        if cursor < start {
            output.push((cursor, start));
        }
        cursor = end;
    }
    if cursor < extent {
        output.push((cursor, extent));
    }
    if output.is_empty() || output[0].0 != 0 {
        return Err(NisarSubsetPlanningErrorV2::BootstrapFailed);
    }
    Ok(output)
}

fn expected_bootstrap_checkpoint_v2(
    catalog: &NisarPlanningPhysicalRangeCatalogV2,
    source_size: u64,
    source_validator_sha256: &str,
    max_request_count: u64,
) -> Result<(NisarSubsetBootstrapStatisticsV2, Vec<(u64, u64)>), NisarSubsetPlanningErrorV2> {
    if source_size == 0
        || !valid_sha256(source_validator_sha256)
        || catalog.selected_science_ranges.is_empty()
        || catalog.all_science_ranges.is_empty()
        || catalog.selected_science_ranges.iter().any(|selected| {
            !catalog
                .all_science_ranges
                .iter()
                .any(|all| all.0 <= selected.0 && all.1 >= selected.1)
        })
    {
        return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
    }
    let materialization = complement_ranges(&catalog.all_science_ranges, source_size)
        .map_err(|_| NisarSubsetPlanningErrorV2::RecoveryInvalid)?;
    if materialization.iter().any(|material| {
        catalog
            .selected_science_ranges
            .iter()
            .any(|science| tuple_ranges_overlap(*material, *science))
    }) {
        return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
    }
    let mut metadata_prefix = NisarSubsetBootstrapCategoryStatisticsV2::default();
    let mut metadata_discovery = NisarSubsetBootstrapCategoryStatisticsV2::default();
    let mut additional_geolocation = NisarSubsetBootstrapCategoryStatisticsV2::default();
    for (index, &(start, end_exclusive)) in materialization.iter().enumerate() {
        let category = if index == 0 && start == 0 {
            &mut metadata_prefix
        } else if catalog
            .geolocation_ranges
            .iter()
            .any(|range| tuple_ranges_overlap((start, end_exclusive), *range))
        {
            &mut additional_geolocation
        } else {
            &mut metadata_discovery
        };
        category
            .requested_ranges
            .push(NisarSubsetBootstrapRangeV2::new(start, end_exclusive)?);
        category.request_counts.push(1);
        category.request_count += 1;
        category.response_body_bytes = category
            .response_body_bytes
            .checked_add(end_exclusive - start)
            .ok_or(NisarSubsetPlanningErrorV2::RecoveryInvalid)?;
    }
    if catalog
        .metadata_ranges
        .iter()
        .chain(&catalog.geolocation_ranges)
        .any(|required| {
            !materialization
                .iter()
                .any(|available| available.0 <= required.0 && available.1 >= required.1)
        })
    {
        return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
    }
    let preflight_range = NisarSubsetBootstrapRangeV2::new(0, 1)?;
    let preflight = NisarSubsetBootstrapCategoryStatisticsV2 {
        request_count: 1,
        response_body_bytes: 1,
        requested_ranges: vec![preflight_range.clone()],
        request_counts: vec![1],
    };
    let revalidation = NisarSubsetBootstrapCategoryStatisticsV2 {
        request_count: 1,
        response_body_bytes: 1,
        requested_ranges: vec![preflight_range],
        request_counts: vec![1],
    };
    let categories = [
        &preflight,
        &revalidation,
        &metadata_prefix,
        &metadata_discovery,
        &additional_geolocation,
    ];
    let total_planning_request_count = categories.iter().try_fold(0_u64, |total, category| {
        total
            .checked_add(category.request_count)
            .ok_or(NisarSubsetPlanningErrorV2::RecoveryInvalid)
    })?;
    let total_planning_response_body_bytes =
        categories.iter().try_fold(0_u64, |total, category| {
            total
                .checked_add(category.response_body_bytes)
                .ok_or(NisarSubsetPlanningErrorV2::RecoveryInvalid)
        })?;
    let statistics = NisarSubsetBootstrapStatisticsV2 {
        frozen_plan_fast_path: false,
        preflight,
        revalidation,
        metadata_prefix,
        metadata_discovery,
        additional_geolocation,
        total_planning_request_count,
        total_planning_response_body_bytes,
        charged_failure_requests: 0,
        charged_failure_response_bytes: 0,
        source_size,
        source_validator_sha256: Some(source_validator_sha256.to_owned()),
        bootstrap_budget: Some(NisarSubsetBootstrapBudgetV2 {
            max_request_count,
            max_response_body_bytes: nisar_live_budget_response_limit(source_size)
                .map_err(|_| NisarSubsetPlanningErrorV2::RecoveryInvalid)?,
        }),
        science_request_count: 0,
        science_response_body_bytes: 0,
    };
    statistics.validate_checkpoint()?;
    Ok((statistics, materialization))
}

fn planning_material_ranges_sha256(
    material_ranges: &[(u64, u64)],
) -> Result<String, NisarSubsetPlanningErrorV2> {
    digest_serializable(&("NisarPlanningSparsePartMaterialRangesV2", material_ranges))
}

fn planning_source_material_sha256(
    source_path: &Path,
    source_size: u64,
    material_ranges: &[(u64, u64)],
) -> Result<String, NisarSubsetPlanningErrorV2> {
    let mut source =
        File::open(source_path).map_err(|_| NisarSubsetPlanningErrorV2::RecoveryInvalid)?;
    if source
        .metadata()
        .map_err(|_| NisarSubsetPlanningErrorV2::RecoveryInvalid)?
        .len()
        != source_size
    {
        return Err(NisarSubsetPlanningErrorV2::ValidatorChanged);
    }
    let mut material_digest = Sha256::new();
    material_digest.update(b"NisarPlanningSparsePartMaterialV2");
    let mut buffer = [0_u8; 64 * 1024];
    for &(start, end_exclusive) in material_ranges {
        if start >= end_exclusive || end_exclusive > source_size {
            return Err(NisarSubsetPlanningErrorV2::RecoveryInvalid);
        }
        material_digest.update(start.to_be_bytes());
        material_digest.update(end_exclusive.to_be_bytes());
        source
            .seek(SeekFrom::Start(start))
            .map_err(|_| NisarSubsetPlanningErrorV2::RecoveryInvalid)?;
        let mut remaining = end_exclusive - start;
        while remaining > 0 {
            let count = usize::try_from(remaining.min(buffer.len() as u64))
                .map_err(|_| NisarSubsetPlanningErrorV2::RecoveryInvalid)?;
            source
                .read_exact(&mut buffer[..count])
                .map_err(|_| NisarSubsetPlanningErrorV2::RecoveryInvalid)?;
            material_digest.update(&buffer[..count]);
            remaining -= count as u64;
        }
    }
    Ok(format!("{:x}", material_digest.finalize()))
}

fn is_control_active(control: &WorkerControl) -> bool {
    !control.is_cancelled() && !control.is_paused()
}

fn ensure_active(control: &WorkerControl) -> Result<(), NisarSubsetPlanningErrorV2> {
    if control.is_cancelled() || control.is_paused() {
        return Err(NisarSubsetPlanningErrorV2::ExecutionStopped);
    }
    Ok(())
}

fn ranges_overlap(left: &NisarSubsetBootstrapRangeV2, right: &NisarSubsetBootstrapRangeV2) -> bool {
    left.start < right.end_exclusive && right.start < left.end_exclusive
}

fn tuple_ranges_overlap(left: (u64, u64), right: (u64, u64)) -> bool {
    left.0 < right.1 && right.0 < left.1
}

fn digest_serializable(value: &impl Serialize) -> Result<String, NisarSubsetPlanningErrorV2> {
    serde_json::to_vec(value)
        .map(|bytes| sha256(&bytes))
        .map_err(|_| NisarSubsetPlanningErrorV2::BootstrapFailed)
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

#[cfg(test)]
mod large_grid_mapping_tests {
    use super::*;

    #[test]
    fn physical_geolocation_mapping_admits_live_scale_grid_above_16384_cells() {
        let dir =
            std::env::temp_dir().join(format!("insar-nisar-large-grid-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let h5_path = dir.join("synthetic_large_grid.h5");
        {
            let file = hdf5::File::create(&h5_path).unwrap();
            let geo = file
                .create_group("/science/LSAR/RSLC/metadata/geolocationGrid")
                .unwrap();
            let swaths = file.create_group("/science/LSAR/RSLC/swaths").unwrap();
            let freq_a = file
                .create_group("/science/LSAR/RSLC/swaths/frequencyA")
                .unwrap();

            let n_rows = 140usize;
            let n_cols = 140usize;
            let radar_rows = 1400usize;
            let radar_cols = 1400usize;
            let heights = vec![-500.0_f64, 4500.0, 9000.0];
            let grid_t: Vec<f64> = (0..n_rows).map(|r| (r * 10) as f64).collect();
            let grid_r: Vec<f64> = (0..n_cols).map(|c| 800_000.0 + (c * 10) as f64).collect();
            let sw_t: Vec<f64> = (0..radar_rows).map(|r| r as f64).collect();
            let sw_r: Vec<f64> = (0..radar_cols).map(|c| 800_000.0 + c as f64).collect();

            let mut xs = Vec::with_capacity(heights.len() * n_rows * n_cols);
            let mut ys = Vec::with_capacity(heights.len() * n_rows * n_cols);
            for _layer in 0..heights.len() {
                for r in 0..n_rows {
                    for c in 0..n_cols {
                        xs.push(84.0 + (c as f64) * 0.002);
                        ys.push(28.0 + (r as f64) * 0.002);
                    }
                }
            }

            geo.new_dataset::<f64>()
                .shape([heights.len()])
                .create("heightAboveEllipsoid")
                .unwrap()
                .write_raw(&heights)
                .unwrap();
            geo.new_dataset::<f64>()
                .shape([n_rows])
                .create("zeroDopplerTime")
                .unwrap()
                .write_raw(&grid_t)
                .unwrap();
            geo.new_dataset::<f64>()
                .shape([n_cols])
                .create("slantRange")
                .unwrap()
                .write_raw(&grid_r)
                .unwrap();
            geo.new_dataset::<f64>()
                .shape([heights.len(), n_rows, n_cols])
                .create("coordinateX")
                .unwrap()
                .write_raw(&xs)
                .unwrap();
            geo.new_dataset::<f64>()
                .shape([heights.len(), n_rows, n_cols])
                .create("coordinateY")
                .unwrap()
                .write_raw(&ys)
                .unwrap();
            swaths
                .new_dataset::<f64>()
                .shape([radar_rows])
                .create("zeroDopplerTime")
                .unwrap()
                .write_raw(&sw_t)
                .unwrap();
            freq_a
                .new_dataset::<f64>()
                .shape([radar_cols])
                .create("slantRange")
                .unwrap()
                .write_raw(&sw_r)
                .unwrap();
        }

        let file = crate::hdf5_file::open(&h5_path).unwrap();
        let inputs = read_physical_geolocation_inputs_v2(
            &file,
            NisarScienceRoot::Lsar,
            &NisarSubsetFrequency::A,
        )
        .expect("geolocation inputs should load from HDF5");
        let aoi: serde_json::Map<String, serde_json::Value> = serde_json::from_value(serde_json::json!({
            "type": "Polygon",
            "coordinates": [[[84.004, 28.004], [84.274, 28.004], [84.274, 28.274], [84.004, 28.274], [84.004, 28.004]]]
        }))
        .unwrap();
        let evidence = insar_provider::asf::nisar_subset::build_live_geolocation_evidence(
            &aoi,
            NisarSubsetFrequency::A,
            "HH",
            NisarScienceRoot::Lsar.swath(),
            &inputs.grid_zero_doppler_time,
            &inputs.grid_slant_range,
            &inputs.swath_zero_doppler_time,
            &inputs.swath_slant_range,
            inputs.height_layers_m,
            inputs.longitudes,
            inputs.latitudes,
            [1400, 1400],
            [512, 512],
            inputs.minimum_hae_m,
            inputs.maximum_hae_m,
        )
        .expect("135x135 (>18,225) cells must map without PhysicalMappingUnavailable / candidate_budget");
        assert_eq!(evidence.windows.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
#[path = "nisar_planning_v2_tests.rs"]
mod tests;
