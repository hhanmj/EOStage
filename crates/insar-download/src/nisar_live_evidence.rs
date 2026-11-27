#![cfg(feature = "nisar-subset-live-acceptance")]

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use insar_core::{
    NisarSubsetArtifactManifestV1, NisarSubsetCheckStatus, NisarSubsetFrequency,
    NISAR_SUBSET_OUTPUT_AUTHORITY,
};
use serde::{Deserialize, Serialize};

use crate::production_worker::{
    NisarLiveBudgetLimitsInput, NisarLivePlanDigestsInput, NisarLiveRangeFailureObservationInput,
    NisarLiveRangeObservationInput, NisarLiveSourceIdentityInput,
};

#[cfg(feature = "nisar-subset-production")]
use crate::nisar_subset_writer::NisarSubsetArtifactManifestV2;

pub const NISAR_M2D_A_LOCAL_EXECUTION_PLAN_SHA256: &str =
    "05a98f570c2cd3e95421c350499edc640846f77e80aa7767063e72a7c2146a24";
/// Compatibility name for historical tests and evidence readers. It is an
/// execution identity only and is never the live cross-environment gate.
pub const NISAR_M2D_A_CURRENT_PLAN_SHA256: &str = NISAR_M2D_A_LOCAL_EXECUTION_PLAN_SHA256;
/// Explicitly qualified from the offline full-source local oracle. It is the
/// cross-environment semantic admission authority, not a live locator result.
pub const NISAR_M2D_B2_SEMANTIC_PLAN_AUTHORITY_SHA256: &str =
    "17e1c27704fd82bf9ae75939bcea466a7ce3ea3b639189484f5d1907c44a2192";
pub const NISAR_M2D_B_EVIDENCE_SCHEMA: &str =
    "nisar-remote-subset-m2d-b-live-acceptance-redacted-v2";
pub const NISAR_M2D_B_GATE: &str = "P0.3a-M2D-B";
pub const NISAR_M2D_C2_EVIDENCE_SCHEMA: &str =
    "nisar-remote-subset-m2d-c2-live-acceptance-redacted-v1";
pub const NISAR_M2D_C2_GATE: &str = "P0.3a-M2D-C2";
/// Independent scene-generic Frequency B admission identity. This is a
/// schema/gate label only; no precomputed semantic digest is used as an
/// authority for this path.
pub const NISAR_FREQ_B_EVIDENCE_SCHEMA: &str = "nisar-frequency-b-live-acceptance-redacted-v1";
pub const NISAR_FREQ_B_GATE: &str = "P0.3a-FREQ-B";

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NisarLiveBudgetEvidenceV2 {
    pub requests: u64,
    pub response_bytes: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NisarLiveGeometryEvidenceV2 {
    pub line_start: u64,
    pub line_end_exclusive: u64,
    pub sample_start: u64,
    pub sample_end_exclusive: u64,
}

/// Secret-free observation of one range response that passed the V2 HTTP
/// protocol checks. The validator is represented only by its SHA-256 digest;
/// raw ETags, locators, and authorization material never enter evidence.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NisarLiveRangeObservationV2 {
    pub stage: String,
    pub start: u64,
    pub end_exclusive: u64,
    pub source_size_bytes: u64,
    pub request_count: u64,
    pub http_status: u16,
    pub content_range_start: u64,
    pub content_range_end_exclusive: u64,
    pub content_range_total: u64,
    pub response_body_bytes: u64,
    pub validator_sha256: String,
}

/// Secret-free redirect diagnostics attached to a failed logical Range.
/// Values are coarse policy classes only; raw locators and credentials never enter evidence.
/// Secret-free observation of a range attempt that did not pass the V2 HTTP
/// protocol checks. A failed attempt is deliberately kept out of
/// `range_observations`; optional metadata stays absent when it was not
/// observed completely.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NisarLiveRangeFailureObservationV2 {
    pub stage: String,
    pub start: u64,
    pub end_exclusive: u64,
    pub source_size_bytes: u64,
    pub disposition: String,
    pub request_count: u64,
    pub http_status: Option<u16>,
    pub response_body_bytes: Option<u64>,
    pub validator_sha256: Option<String>,
    pub failure_code: String,
    #[serde(default)]
    pub redirect_occurred: bool,
    #[serde(default)]
    pub redirect_hop_index: Option<u8>,
    #[serde(default)]
    pub redirect_source_host_class: Option<String>,
    #[serde(default)]
    pub redirect_destination_host_class: Option<String>,
    #[serde(default)]
    pub redirect_source_path_class: Option<String>,
    #[serde(default)]
    pub redirect_destination_path_class: Option<String>,
    #[serde(default)]
    pub redirect_same_origin: Option<bool>,
    #[serde(default)]
    pub redirect_scheme_transition: Option<String>,
    #[serde(default)]
    pub redirect_policy_result: Option<String>,
    #[serde(default)]
    pub redirect_rejection_reason: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NisarLiveScientificValidationEvidenceV2 {
    pub complex_mismatch: u64,
    pub amplitude_mismatch: u64,
    pub wrapped_phase_mismatch: u64,
    pub mask_mismatch: u64,
    pub logical_science_equality: bool,
    pub identification_semantics: bool,
    pub dimension_scales: bool,
    pub metadata_semantics: bool,
    pub provenance_semantics: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NisarLiveAcceptanceEvidenceV2 {
    pub schema_version: String,
    pub gate: String,
    pub baseline_commit: String,
    pub harness_commit: String,
    pub status: String,
    pub typed_stop_code: String,
    pub granule_id: String,
    pub source_size_bytes: u64,
    pub source_checksum_algorithm: String,
    pub source_checksum: String,
    pub aoi_digest: String,
    pub frequency: String,
    pub polarization: String,
    /// Frequency-specific capability as advertised by authoritative metadata.
    #[serde(default)]
    pub advertised_polarizations: Vec<String>,
    /// Canonical sorted selection frozen into PreparedIntent and the artifact.
    #[serde(default)]
    pub selected_polarizations: Vec<String>,
    #[serde(default)]
    pub provider_id: String,
    #[serde(default)]
    pub provider_product_id: String,
    #[serde(default)]
    pub provider_product_key: String,
    #[serde(default)]
    pub collection_id: String,
    #[serde(default)]
    pub asset_key: String,
    #[serde(default)]
    pub stable_asset_key: String,
    pub final_host_class: String,
    pub final_path_class: String,
    pub locator_hash: String,
    pub validator_hash: String,
    #[serde(default)]
    pub source_validator_sha256: String,
    #[serde(default)]
    pub source_product_identity_sha256: String,
    #[serde(default)]
    pub source_asset_identity_sha256: String,
    #[serde(default)]
    pub source_footprint_sha256: String,
    #[serde(default)]
    pub requested_geometry_sha256: String,
    #[serde(default)]
    pub effective_geometry_sha256: String,
    #[serde(default)]
    pub prepared_intent_sha256: String,
    #[serde(default)]
    pub final_plan_sha256: String,
    #[serde(default)]
    pub physical_plan_sha256: String,
    #[serde(default)]
    pub inventory_evidence_sha256: String,
    #[serde(default)]
    pub source_graph_sha256: String,
    pub preflight: NisarLiveBudgetEvidenceV2,
    pub metadata_prefix: NisarLiveBudgetEvidenceV2,
    pub metadata_discovery: NisarLiveBudgetEvidenceV2,
    #[serde(default)]
    pub additional_geolocation: NisarLiveBudgetEvidenceV2,
    #[serde(default)]
    pub planning_revalidation: NisarLiveBudgetEvidenceV2,
    pub science_range: NisarLiveBudgetEvidenceV2,
    pub preflight_requests: u64,
    pub preflight_response_bytes: u64,
    pub prefix_requests: u64,
    pub prefix_response_bytes: u64,
    pub discovery_requests: u64,
    pub discovery_response_bytes: u64,
    #[serde(default)]
    pub additional_geolocation_requests: u64,
    #[serde(default)]
    pub additional_geolocation_response_bytes: u64,
    #[serde(default)]
    pub planning_revalidation_requests: u64,
    #[serde(default)]
    pub planning_revalidation_response_bytes: u64,
    pub science_requests: u64,
    pub science_response_bytes: u64,
    pub total_requests: u64,
    pub total_response_bytes: u64,
    #[serde(default)]
    pub max_requests: u64,
    #[serde(default)]
    pub max_response_bytes: u64,
    #[serde(default)]
    pub remaining_requests: u64,
    #[serde(default)]
    pub remaining_response_bytes: u64,
    #[serde(default)]
    pub planning_requests: u64,
    #[serde(default)]
    pub planning_response_bytes: u64,
    #[serde(default)]
    pub reserved_science_requests: u64,
    #[serde(default)]
    pub reserved_science_response_bytes: u64,
    #[serde(default)]
    pub committed_science_requests: u64,
    #[serde(default)]
    pub committed_science_response_bytes: u64,
    pub source_byte_ratio: f64,
    pub source_dataset_count: u64,
    pub metadata_group_binding_count: u64,
    pub metadata_dataset_binding_count: u64,
    pub metadata_binding_count: u64,
    pub inventory_sha256: String,
    pub local_qualification_execution_plan_sha256: String,
    pub execution_plan_sha256: String,
    pub expected_semantic_plan_sha256: String,
    pub semantic_plan_sha256: String,
    pub geometry: NisarLiveGeometryEvidenceV2,
    pub scientific_validation: NisarLiveScientificValidationEvidenceV2,
    #[serde(default)]
    pub range_observations: Vec<NisarLiveRangeObservationV2>,
    #[serde(default)]
    pub range_failure_observations: Vec<NisarLiveRangeFailureObservationV2>,
    /// Conservative accounting for failed sent attempts, kept separate from
    /// accepted 206 totals.
    #[serde(default)]
    pub charged_failure_requests: u64,
    #[serde(default)]
    pub charged_failure_response_bytes: u64,
    /// Durable science attempts that failed before range commit; separate from planning failures.
    #[serde(default)]
    pub failed_science_requests: u64,
    #[serde(default)]
    pub failed_science_response_bytes: u64,
    #[serde(default)]
    pub exact_http_206: bool,
    #[serde(default)]
    pub exact_content_range: bool,
    #[serde(default)]
    pub validator_continuity: bool,
    #[serde(default)]
    pub output_authority: String,
    #[serde(default)]
    pub output_class: String,
    #[serde(default)]
    pub one_hdf5: bool,
    pub output_size_bytes: u64,
    pub output_sha256: String,
    pub atomic_promotion: bool,
    pub cleanup_completed: bool,
    #[serde(default)]
    pub store_completed: String,
    pub store_reopen: String,
}

impl NisarLiveAcceptanceEvidenceV2 {
    fn new(baseline_commit: String, harness_commit: String) -> Self {
        Self {
            schema_version: NISAR_M2D_B_EVIDENCE_SCHEMA.to_owned(),
            gate: NISAR_M2D_B_GATE.to_owned(),
            baseline_commit: safe_atom(&baseline_commit),
            harness_commit: safe_atom(&harness_commit),
            status: "pending".to_owned(),
            typed_stop_code: "NISAR_LIVE_NOT_STARTED".to_owned(),
            granule_id: "not_observed".to_owned(),
            source_size_bytes: 0,
            source_checksum_algorithm: "not_observed".to_owned(),
            source_checksum: "not_observed".to_owned(),
            aoi_digest: "not_observed".to_owned(),
            frequency: "not_observed".to_owned(),
            polarization: "not_observed".to_owned(),
            advertised_polarizations: Vec::new(),
            selected_polarizations: Vec::new(),
            provider_id: "not_observed".to_owned(),
            provider_product_id: "not_observed".to_owned(),
            provider_product_key: "not_observed".to_owned(),
            collection_id: "not_observed".to_owned(),
            asset_key: "not_observed".to_owned(),
            stable_asset_key: "not_observed".to_owned(),
            final_host_class: "not_observed".to_owned(),
            final_path_class: "not_observed".to_owned(),
            locator_hash: "not_observed".to_owned(),
            validator_hash: "not_observed".to_owned(),
            source_validator_sha256: "not_observed".to_owned(),
            source_product_identity_sha256: "not_observed".to_owned(),
            source_asset_identity_sha256: "not_observed".to_owned(),
            source_footprint_sha256: "not_observed".to_owned(),
            requested_geometry_sha256: "not_observed".to_owned(),
            effective_geometry_sha256: "not_observed".to_owned(),
            prepared_intent_sha256: "not_observed".to_owned(),
            final_plan_sha256: "not_observed".to_owned(),
            physical_plan_sha256: "not_observed".to_owned(),
            inventory_evidence_sha256: "not_observed".to_owned(),
            source_graph_sha256: "not_observed".to_owned(),
            preflight: NisarLiveBudgetEvidenceV2::default(),
            metadata_prefix: NisarLiveBudgetEvidenceV2::default(),
            metadata_discovery: NisarLiveBudgetEvidenceV2::default(),
            additional_geolocation: NisarLiveBudgetEvidenceV2::default(),
            planning_revalidation: NisarLiveBudgetEvidenceV2::default(),
            science_range: NisarLiveBudgetEvidenceV2::default(),
            preflight_requests: 0,
            preflight_response_bytes: 0,
            prefix_requests: 0,
            prefix_response_bytes: 0,
            discovery_requests: 0,
            discovery_response_bytes: 0,
            additional_geolocation_requests: 0,
            additional_geolocation_response_bytes: 0,
            planning_revalidation_requests: 0,
            planning_revalidation_response_bytes: 0,
            science_requests: 0,
            science_response_bytes: 0,
            total_requests: 0,
            total_response_bytes: 0,
            max_requests: 0,
            max_response_bytes: 0,
            remaining_requests: 0,
            remaining_response_bytes: 0,
            planning_requests: 0,
            planning_response_bytes: 0,
            reserved_science_requests: 0,
            reserved_science_response_bytes: 0,
            committed_science_requests: 0,
            committed_science_response_bytes: 0,
            source_byte_ratio: 0.0,
            source_dataset_count: 0,
            metadata_group_binding_count: 0,
            metadata_dataset_binding_count: 0,
            metadata_binding_count: 0,
            inventory_sha256: "not_observed".to_owned(),
            local_qualification_execution_plan_sha256: NISAR_M2D_A_LOCAL_EXECUTION_PLAN_SHA256
                .to_owned(),
            execution_plan_sha256: "not_observed".to_owned(),
            expected_semantic_plan_sha256: NISAR_M2D_B2_SEMANTIC_PLAN_AUTHORITY_SHA256.to_owned(),
            semantic_plan_sha256: "not_observed".to_owned(),
            geometry: NisarLiveGeometryEvidenceV2::default(),
            scientific_validation: NisarLiveScientificValidationEvidenceV2::default(),
            range_observations: Vec::new(),
            range_failure_observations: Vec::new(),
            charged_failure_requests: 0,
            charged_failure_response_bytes: 0,
            failed_science_requests: 0,
            failed_science_response_bytes: 0,
            exact_http_206: false,
            exact_content_range: false,
            validator_continuity: false,
            output_authority: "not_observed".to_owned(),
            output_class: "not_observed".to_owned(),
            one_hdf5: false,
            output_size_bytes: 0,
            output_sha256: "not_observed".to_owned(),
            atomic_promotion: false,
            cleanup_completed: false,
            store_completed: "not_observed".to_owned(),
            store_reopen: "not_in_scope_m2d_b".to_owned(),
        }
    }

    fn new_scene_generic(
        baseline_commit: String,
        harness_commit: String,
        schema_version: &str,
        gate: &str,
    ) -> Self {
        let mut evidence = Self::new(baseline_commit, harness_commit);
        evidence.schema_version = safe_atom(schema_version);
        evidence.gate = safe_atom(gate);
        evidence.local_qualification_execution_plan_sha256 =
            "not_applicable_scene_generic".to_owned();
        evidence.expected_semantic_plan_sha256 = "not_applicable_scene_generic".to_owned();
        evidence.store_reopen = "not_observed".to_owned();
        evidence
    }
}

#[derive(Clone)]
#[doc(hidden)]
pub struct NisarLiveAcceptanceEvidenceSink {
    path: Arc<PathBuf>,
    state: Arc<Mutex<NisarLiveAcceptanceEvidenceV2>>,
}

impl NisarLiveAcceptanceEvidenceSink {
    pub fn new(
        live_root: impl Into<PathBuf>,
        baseline_commit: impl Into<String>,
        harness_commit: impl Into<String>,
    ) -> io::Result<Self> {
        let live_root = live_root.into();
        if !live_root.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "M2D-B live root must be absolute",
            ));
        }
        let evidence_dir = live_root.join("evidence");
        fs::create_dir_all(&evidence_dir)?;
        let sink = Self {
            path: Arc::new(evidence_dir.join("live-acceptance-redacted.json")),
            state: Arc::new(Mutex::new(NisarLiveAcceptanceEvidenceV2::new(
                baseline_commit.into(),
                harness_commit.into(),
            ))),
        };
        sink.persist()?;
        Ok(sink)
    }

    pub fn new_for_scene_generic_acceptance(
        live_root: impl Into<PathBuf>,
        baseline_commit: impl Into<String>,
        harness_commit: impl Into<String>,
        schema_version: &str,
        gate: &str,
    ) -> io::Result<Self> {
        let live_root = live_root.into();
        if !live_root.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "scene-generic live root must be absolute",
            ));
        }
        let evidence_dir = live_root.join("evidence");
        fs::create_dir_all(&evidence_dir)?;
        let sink = Self {
            path: Arc::new(evidence_dir.join("live-acceptance-redacted.json")),
            state: Arc::new(Mutex::new(
                NisarLiveAcceptanceEvidenceV2::new_scene_generic(
                    baseline_commit.into(),
                    harness_commit.into(),
                    schema_version,
                    gate,
                ),
            )),
        };
        sink.persist()?;
        Ok(sink)
    }

    pub fn evidence_path(&self) -> PathBuf {
        self.path.as_ref().clone()
    }

    pub fn snapshot(&self) -> NisarLiveAcceptanceEvidenceV2 {
        self.state
            .lock()
            .expect("M2D-B evidence state lock")
            .clone()
    }

    pub fn record_failure(&self, code: &str) -> io::Result<()> {
        self.update(|evidence| {
            evidence.status = "fail".to_owned();
            evidence.typed_stop_code = safe_stop_code(code);
        })
    }

    pub fn mark_cleanup_completed(&self, completed: bool) -> io::Result<()> {
        self.update(|evidence| evidence.cleanup_completed = completed)
    }

    pub fn mark_store_reopen(&self, state: &str) -> io::Result<()> {
        let state = match state {
            "observed" | "not_in_scope_m2d_b" => state.to_owned(),
            _ => "not_in_scope_m2d_b".to_owned(),
        };
        self.update(|evidence| evidence.store_reopen = state)
    }

    pub fn mark_store_completed(&self, state: &str) -> io::Result<()> {
        let state = match state {
            "observed" | "not_in_scope_m2d_b" => state.to_owned(),
            _ => "not_in_scope_m2d_b".to_owned(),
        };
        self.update(|evidence| evidence.store_completed = state)
    }

    pub(crate) fn mark_running(&self) -> io::Result<()> {
        self.update(|evidence| evidence.status = "running".to_owned())
    }

    pub(crate) fn record_scope(
        &self,
        granule_id: &str,
        source_size_bytes: u64,
        source_checksum_algorithm: &str,
        source_checksum: &str,
        frequency: NisarSubsetFrequency,
        polarization: &str,
    ) -> io::Result<()> {
        self.update(|evidence| {
            evidence.granule_id = safe_atom(granule_id);
            evidence.source_size_bytes = source_size_bytes;
            evidence.source_checksum_algorithm = safe_atom(source_checksum_algorithm);
            evidence.source_checksum = safe_atom(source_checksum);
            evidence.frequency = match frequency {
                NisarSubsetFrequency::A => "A".to_owned(),
                NisarSubsetFrequency::B => "B".to_owned(),
            };
            evidence.polarization = safe_atom(polarization);
        })
    }

    pub(crate) fn record_aoi_digest(&self, aoi_digest: &str) -> io::Result<()> {
        self.update(|evidence| evidence.aoi_digest = safe_atom(aoi_digest))
    }

    pub(crate) fn record_source_identity(
        &self,
        input: NisarLiveSourceIdentityInput<'_>,
    ) -> io::Result<()> {
        let identity_values = [
            input.provider_id,
            input.provider_product_id,
            input.provider_product_key,
            input.collection_id,
            input.asset_key,
            input.stable_asset_key,
        ];
        if identity_values
            .iter()
            .any(|value| value.is_empty() || safe_atom(value) == "[REDACTED]")
            || !is_sha256(input.source_product_identity_sha256)
            || !is_sha256(input.source_asset_identity_sha256)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid NISAR source identity evidence",
            ));
        }
        self.update(|evidence| {
            evidence.provider_id = safe_atom(input.provider_id);
            evidence.provider_product_id = safe_atom(input.provider_product_id);
            evidence.provider_product_key = safe_atom(input.provider_product_key);
            evidence.collection_id = safe_atom(input.collection_id);
            evidence.asset_key = safe_atom(input.asset_key);
            evidence.stable_asset_key = safe_atom(input.stable_asset_key);
            evidence.source_product_identity_sha256 =
                safe_atom(input.source_product_identity_sha256);
            evidence.source_asset_identity_sha256 = safe_atom(input.source_asset_identity_sha256);
        })
    }

    pub(crate) fn record_polarizations(
        &self,
        advertised: &[String],
        selected: &[String],
    ) -> io::Result<()> {
        if advertised.is_empty()
            || selected.is_empty()
            || advertised.iter().any(|value| !safe_polarization(value))
            || selected.iter().any(|value| !safe_polarization(value))
            || advertised.windows(2).any(|pair| pair[0] >= pair[1])
            || selected.windows(2).any(|pair| pair[0] >= pair[1])
            || selected.iter().any(|value| !advertised.contains(value))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid NISAR polarization evidence",
            ));
        }
        self.update(|evidence| {
            evidence.advertised_polarizations = advertised.to_vec();
            evidence.selected_polarizations = selected.to_vec();
        })
    }

    pub(crate) fn record_geometry_digests(
        &self,
        source_footprint_sha256: &str,
        requested_geometry_sha256: &str,
        effective_geometry_sha256: &str,
    ) -> io::Result<()> {
        if !is_sha256(source_footprint_sha256)
            || !is_sha256(requested_geometry_sha256)
            || !is_sha256(effective_geometry_sha256)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid NISAR geometry digest evidence",
            ));
        }
        self.update(|evidence| {
            evidence.source_footprint_sha256 = safe_atom(source_footprint_sha256);
            evidence.requested_geometry_sha256 = safe_atom(requested_geometry_sha256);
            evidence.effective_geometry_sha256 = safe_atom(effective_geometry_sha256);
            evidence.aoi_digest = safe_atom(requested_geometry_sha256);
        })
    }

    pub(crate) fn record_plan_digests(
        &self,
        input: NisarLivePlanDigestsInput<'_>,
    ) -> io::Result<()> {
        if [
            input.prepared_intent_sha256,
            input.final_plan_sha256,
            input.physical_plan_sha256,
            input.inventory_sha256,
            input.inventory_evidence_sha256,
            input.source_graph_sha256,
            input.source_validator_sha256,
        ]
        .iter()
        .any(|value| !is_sha256(value))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid NISAR plan digest evidence",
            ));
        }
        self.update(|evidence| {
            evidence.prepared_intent_sha256 = safe_atom(input.prepared_intent_sha256);
            evidence.final_plan_sha256 = safe_atom(input.final_plan_sha256);
            evidence.physical_plan_sha256 = safe_atom(input.physical_plan_sha256);
            evidence.inventory_sha256 = safe_atom(input.inventory_sha256);
            evidence.inventory_evidence_sha256 = safe_atom(input.inventory_evidence_sha256);
            evidence.source_graph_sha256 = safe_atom(input.source_graph_sha256);
            evidence.source_validator_sha256 = safe_atom(input.source_validator_sha256);
        })
    }

    pub(crate) fn record_budget_limits(&self, input: NisarLiveBudgetLimitsInput) -> io::Result<()> {
        let request_totals_valid = input
            .planning_requests
            .checked_add(input.committed_science_requests)
            .and_then(|value| value.checked_add(input.remaining_requests))
            .is_some_and(|value| value <= input.max_requests);
        let response_totals_valid = input
            .planning_response_bytes
            .checked_add(input.committed_science_response_bytes)
            .and_then(|value| value.checked_add(input.remaining_response_bytes))
            .is_some_and(|value| value <= input.max_response_bytes);
        if input.max_requests == 0
            || input.max_response_bytes == 0
            || input.reserved_science_requests < input.committed_science_requests
            || input.reserved_science_response_bytes < input.committed_science_response_bytes
            || !request_totals_valid
            || !response_totals_valid
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid NISAR budget evidence",
            ));
        }
        self.update(|evidence| {
            evidence.max_requests = input.max_requests;
            evidence.max_response_bytes = input.max_response_bytes;
            evidence.remaining_requests = input.remaining_requests;
            evidence.remaining_response_bytes = input.remaining_response_bytes;
            evidence.planning_requests = input.planning_requests;
            evidence.planning_response_bytes = input.planning_response_bytes;
            evidence.reserved_science_requests = input.reserved_science_requests;
            evidence.reserved_science_response_bytes = input.reserved_science_response_bytes;
            evidence.committed_science_requests = input.committed_science_requests;
            evidence.committed_science_response_bytes = input.committed_science_response_bytes;
        })
    }

    pub(crate) fn begin_planning_range_snapshot(
        &self,
        stage: &str,
        source_size: u64,
        validator_sha256: &str,
    ) -> io::Result<()> {
        if !matches!(
            stage,
            "preflight"
                | "metadata_prefix"
                | "metadata_discovery"
                | "additional_geolocation"
                | "planning_revalidation"
        ) || source_size == 0
            || !is_sha256(validator_sha256)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid planning snapshot",
            ));
        }
        let previous = self.snapshot();
        if (previous.source_size_bytes != 0 && previous.source_size_bytes != source_size)
            || previous.range_observations.iter().any(|range| {
                range.source_size_bytes != source_size || range.validator_sha256 != validator_sha256
            })
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "planning snapshot authority drift",
            ));
        }
        // Bootstrap statistics include cached ranges on resume. Replace that
        // stage's snapshot; science observations and actual failures stay intact.
        self.update(|evidence| {
            evidence
                .range_observations
                .retain(|range| range.stage != stage)
        })
    }

    pub(crate) fn record_range_observation(
        &self,
        input: NisarLiveRangeObservationInput<'_>,
    ) -> io::Result<()> {
        if input.stage.is_empty()
            || safe_atom(input.stage) == "[REDACTED]"
            || input.start >= input.end_exclusive
            || input.end_exclusive > input.source_size_bytes
            || input.request_count == 0
            || input.http_status != 206
            || input.response_body_bytes != input.end_exclusive - input.start
            || !is_sha256(input.validator_sha256)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid exact NISAR Range evidence",
            ));
        }
        let snapshot = self.snapshot();
        if (snapshot.source_size_bytes != 0
            && snapshot.source_size_bytes != input.source_size_bytes)
            || snapshot.range_observations.iter().any(|observation| {
                observation.source_size_bytes != input.source_size_bytes
                    || observation.validator_sha256 != input.validator_sha256
            })
            || snapshot
                .range_failure_observations
                .iter()
                .any(|observation| {
                    observation.source_size_bytes != input.source_size_bytes
                        || observation
                            .validator_sha256
                            .as_deref()
                            .zip(Some(input.validator_sha256))
                            .is_some_and(|(previous, current)| previous != current)
                })
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "NISAR Range evidence source identity or validator drift",
            ));
        }
        self.update(|evidence| {
            evidence
                .range_observations
                .push(NisarLiveRangeObservationV2 {
                    stage: safe_atom(input.stage),
                    start: input.start,
                    end_exclusive: input.end_exclusive,
                    source_size_bytes: input.source_size_bytes,
                    request_count: input.request_count,
                    http_status: input.http_status,
                    content_range_start: input.start,
                    content_range_end_exclusive: input.end_exclusive,
                    content_range_total: input.source_size_bytes,
                    response_body_bytes: input.response_body_bytes,
                    validator_sha256: safe_atom(input.validator_sha256),
                });
            evidence.exact_http_206 = true;
            evidence.exact_content_range = true;
            evidence.validator_continuity = true;
        })
    }

    pub(crate) fn record_range_failure_observation(
        &self,
        input: NisarLiveRangeFailureObservationInput<'_>,
    ) -> io::Result<()> {
        let disposition = match input.disposition {
            "before_send_rejected" | "sent_response_rejected" | "sent_outcome_indeterminate" => {
                input.disposition
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "invalid NISAR failed Range disposition",
                ))
            }
        };
        let redirect_fields_valid = if input.redirect_occurred {
            input.http_status.is_some()
                && input.redirect_hop_index.is_some_and(|hop| hop > 0)
                && input
                    .redirect_source_host_class
                    .is_some_and(|value| safe_atom(value) != "[REDACTED]")
                && input
                    .redirect_destination_host_class
                    .is_some_and(|value| safe_atom(value) != "[REDACTED]")
                && input
                    .redirect_source_path_class
                    .is_some_and(|value| safe_atom(value) != "[REDACTED]")
                && input.redirect_same_origin.is_some()
                && input
                    .redirect_destination_path_class
                    .is_some_and(|value| safe_atom(value) != "[REDACTED]")
                && input
                    .redirect_scheme_transition
                    .is_some_and(|value| safe_atom(value) != "[REDACTED]")
                && input
                    .redirect_policy_result
                    .is_some_and(|value| safe_atom(value) != "[REDACTED]")
                && input
                    .redirect_rejection_reason
                    .is_some_and(|value| safe_atom(value) != "[REDACTED]")
        } else {
            input.redirect_hop_index.is_none()
                && input.redirect_source_host_class.is_none()
                && input.redirect_destination_host_class.is_none()
                && input.redirect_source_path_class.is_none()
                && input.redirect_destination_path_class.is_none()
                && input.redirect_same_origin.is_none()
                && input.redirect_scheme_transition.is_none()
                && input.redirect_policy_result.is_none()
                && input.redirect_rejection_reason.is_none()
        };
        if input.stage.is_empty()
            || safe_atom(input.stage) == "[REDACTED]"
            || input.start >= input.end_exclusive
            || input.end_exclusive > input.source_size_bytes
            || (disposition == "before_send_rejected" && input.request_count != 0)
            || (disposition != "before_send_rejected" && input.request_count == 0)
            || input.http_status.is_some_and(|status| status > 599)
            || input
                .validator_sha256
                .is_some_and(|digest| !is_sha256(digest))
            || safe_stop_code(input.failure_code) != input.failure_code
            || !redirect_fields_valid
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid NISAR failed Range evidence",
            ));
        }
        let validator_sha256 = input.validator_sha256.map(safe_atom);
        let charged_requests = if disposition == "before_send_rejected" {
            0
        } else {
            input.request_count
        };
        let charged_response_bytes = if disposition == "before_send_rejected" {
            0
        } else {
            input.response_body_bytes.unwrap_or(0)
        };
        let snapshot = self.snapshot();
        let next_charged_requests = snapshot
            .charged_failure_requests
            .checked_add(charged_requests)
            .ok_or_else(|| io::Error::other("NISAR failed Range request counter overflow"))?;
        let next_charged_response_bytes = snapshot
            .charged_failure_response_bytes
            .checked_add(charged_response_bytes)
            .ok_or_else(|| io::Error::other("NISAR failed Range byte counter overflow"))?;
        if snapshot.source_size_bytes != 0 && snapshot.source_size_bytes != input.source_size_bytes
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "NISAR failed Range source identity drift",
            ));
        }
        self.update(|evidence| {
            evidence
                .range_failure_observations
                .push(NisarLiveRangeFailureObservationV2 {
                    stage: safe_atom(input.stage),
                    start: input.start,
                    end_exclusive: input.end_exclusive,
                    source_size_bytes: input.source_size_bytes,
                    disposition: disposition.to_owned(),
                    request_count: input.request_count,
                    http_status: input.http_status,
                    response_body_bytes: input.response_body_bytes,
                    validator_sha256,
                    failure_code: input.failure_code.to_owned(),
                    redirect_occurred: input.redirect_occurred,
                    redirect_hop_index: input.redirect_hop_index,
                    redirect_source_host_class: input.redirect_source_host_class.map(safe_atom),
                    redirect_destination_host_class: input
                        .redirect_destination_host_class
                        .map(safe_atom),
                    redirect_source_path_class: input.redirect_source_path_class.map(safe_atom),
                    redirect_destination_path_class: input
                        .redirect_destination_path_class
                        .map(safe_atom),
                    redirect_same_origin: input.redirect_same_origin,
                    redirect_scheme_transition: input.redirect_scheme_transition.map(safe_atom),
                    redirect_policy_result: input.redirect_policy_result.map(safe_atom),
                    redirect_rejection_reason: input.redirect_rejection_reason.map(safe_atom),
                });
            evidence.charged_failure_requests = next_charged_requests;
            evidence.charged_failure_response_bytes = next_charged_response_bytes;
            if evidence.max_requests != 0 {
                evidence.remaining_requests =
                    evidence.remaining_requests.saturating_sub(charged_requests);
            }
            if evidence.max_response_bytes != 0 {
                evidence.remaining_response_bytes = evidence
                    .remaining_response_bytes
                    .saturating_sub(charged_response_bytes);
            }
        })
    }

    pub(crate) fn record_preflight(
        &self,
        requests: u64,
        response_bytes: u64,
        final_host_class: &str,
        final_path_class: &str,
        locator_hash: &str,
        validator_hash: &str,
    ) -> io::Result<()> {
        self.update(|evidence| {
            evidence.preflight = NisarLiveBudgetEvidenceV2 {
                requests,
                response_bytes,
            };
            evidence.preflight_requests = requests;
            evidence.preflight_response_bytes = response_bytes;
            evidence.final_host_class = safe_atom(final_host_class);
            evidence.final_path_class = safe_atom(final_path_class);
            evidence.locator_hash = safe_atom(locator_hash);
            evidence.validator_hash = safe_atom(validator_hash);
        })
    }

    pub(crate) fn record_metadata_prefix(
        &self,
        requests: u64,
        response_bytes: u64,
    ) -> io::Result<()> {
        self.update(|evidence| {
            evidence.metadata_prefix = NisarLiveBudgetEvidenceV2 {
                requests,
                response_bytes,
            };
            evidence.prefix_requests = requests;
            evidence.prefix_response_bytes = response_bytes;
        })
    }

    pub(crate) fn record_metadata_discovery(
        &self,
        requests: u64,
        response_bytes: u64,
    ) -> io::Result<()> {
        self.update(|evidence| {
            evidence.metadata_discovery = NisarLiveBudgetEvidenceV2 {
                requests,
                response_bytes,
            };
            evidence.discovery_requests = requests;
            evidence.discovery_response_bytes = response_bytes;
        })
    }

    pub(crate) fn record_additional_geolocation(
        &self,
        requests: u64,
        response_bytes: u64,
    ) -> io::Result<()> {
        self.update(|evidence| {
            evidence.additional_geolocation = NisarLiveBudgetEvidenceV2 {
                requests,
                response_bytes,
            };
            evidence.additional_geolocation_requests = requests;
            evidence.additional_geolocation_response_bytes = response_bytes;
        })
    }

    pub(crate) fn record_planning_revalidation(
        &self,
        requests: u64,
        response_bytes: u64,
    ) -> io::Result<()> {
        self.update(|evidence| {
            evidence.planning_revalidation = NisarLiveBudgetEvidenceV2 {
                requests,
                response_bytes,
            };
            evidence.planning_revalidation_requests = requests;
            evidence.planning_revalidation_response_bytes = response_bytes;
        })
    }

    pub(crate) fn record_science_failures(
        &self,
        requests: u64,
        response_bytes: u64,
    ) -> io::Result<()> {
        if requests == 0 && response_bytes != 0 {
            return Err(io::Error::other(
                "science failure bytes without an issued request",
            ));
        }
        self.update(|evidence| {
            evidence.failed_science_requests = requests;
            evidence.failed_science_response_bytes = response_bytes;
        })
    }

    pub(crate) fn record_science_range(
        &self,
        requests: u64,
        response_bytes: u64,
    ) -> io::Result<()> {
        self.update(|evidence| {
            evidence.science_range = NisarLiveBudgetEvidenceV2 {
                requests,
                response_bytes,
            };
            evidence.science_requests = requests;
            evidence.science_response_bytes = response_bytes;
        })
    }

    pub(crate) fn record_metadata_inventory(
        &self,
        source_dataset_count: u64,
        metadata_group_binding_count: u64,
        metadata_dataset_binding_count: u64,
        sha256: &str,
    ) -> io::Result<()> {
        self.update(|evidence| {
            evidence.source_dataset_count = source_dataset_count;
            evidence.metadata_group_binding_count = metadata_group_binding_count;
            evidence.metadata_dataset_binding_count = metadata_dataset_binding_count;
            evidence.metadata_binding_count =
                metadata_group_binding_count.saturating_add(metadata_dataset_binding_count);
            evidence.inventory_sha256 = safe_atom(sha256);
        })
    }

    pub(crate) fn record_execution_plan(&self, plan_sha256: &str) -> io::Result<()> {
        self.update(|evidence| evidence.execution_plan_sha256 = safe_atom(plan_sha256))
    }

    pub(crate) fn record_semantic_plan(&self, semantic_plan_sha256: &str) -> io::Result<()> {
        self.update(|evidence| evidence.semantic_plan_sha256 = safe_atom(semantic_plan_sha256))
    }

    pub(crate) fn record_prepared_intent(&self, prepared_intent_sha256: &str) -> io::Result<()> {
        self.update(|evidence| evidence.prepared_intent_sha256 = safe_atom(prepared_intent_sha256))
    }

    pub(crate) fn record_geometry(
        &self,
        line_start: u64,
        line_end_exclusive: u64,
        sample_start: u64,
        sample_end_exclusive: u64,
    ) -> io::Result<()> {
        self.update(|evidence| {
            evidence.geometry = NisarLiveGeometryEvidenceV2 {
                line_start,
                line_end_exclusive,
                sample_start,
                sample_end_exclusive,
            }
        })
    }

    pub(crate) fn record_output(&self, manifest: &NisarSubsetArtifactManifestV1) -> io::Result<()> {
        let checks = [
            &manifest.science_validation.structure,
            &manifest.science_validation.complex_values,
            &manifest.science_validation.phase,
            &manifest.science_validation.amplitude,
            &manifest.science_validation.coordinates,
            &manifest.science_validation.metadata,
            &manifest.science_validation.provenance,
            &manifest.science_validation.recovery,
        ];
        let passed = checks
            .iter()
            .all(|check| check.status == NisarSubsetCheckStatus::Passed);
        self.update(|evidence| {
            evidence.output_authority = safe_atom(&manifest.output_authority);
            evidence.output_class = if manifest.complete && passed {
                "client_derived_provisional_nisar_rslc_aoi_subset".to_owned()
            } else {
                "not_promoted".to_owned()
            };
            evidence.one_hdf5 = manifest.complete && passed;
            evidence.output_size_bytes = manifest.artifact_size_bytes;
            evidence.output_sha256 = safe_atom(&manifest.artifact_sha256);
            evidence.atomic_promotion = manifest.complete && passed;
            evidence.geometry = NisarLiveGeometryEvidenceV2 {
                line_start: manifest.actual_extent.line_start,
                line_end_exclusive: manifest.actual_extent.line_end_exclusive,
                sample_start: manifest.actual_extent.sample_start,
                sample_end_exclusive: manifest.actual_extent.sample_end_exclusive,
            };
            evidence.scientific_validation = NisarLiveScientificValidationEvidenceV2 {
                complex_mismatch: if manifest.science_validation.complex_values.status
                    != NisarSubsetCheckStatus::Passed
                {
                    1
                } else {
                    0
                },
                amplitude_mismatch: if manifest.science_validation.amplitude.status
                    != NisarSubsetCheckStatus::Passed
                {
                    1
                } else {
                    0
                },
                wrapped_phase_mismatch: if manifest.science_validation.phase.status
                    != NisarSubsetCheckStatus::Passed
                {
                    1
                } else {
                    0
                },
                mask_mismatch: if !passed { 1 } else { 0 },
                logical_science_equality: passed,
                identification_semantics: manifest.science_validation.structure.status
                    == NisarSubsetCheckStatus::Passed,
                dimension_scales: manifest.science_validation.structure.status
                    == NisarSubsetCheckStatus::Passed,
                metadata_semantics: manifest.science_validation.metadata.status
                    == NisarSubsetCheckStatus::Passed,
                provenance_semantics: manifest.science_validation.provenance.status
                    == NisarSubsetCheckStatus::Passed,
            };
        })
    }

    #[cfg(feature = "nisar-subset-production")]
    pub(crate) fn record_output_v2(
        &self,
        manifest: &NisarSubsetArtifactManifestV2,
    ) -> io::Result<()> {
        let selection_matches =
            self.snapshot().selected_polarizations.as_slice() == manifest.selected_polarizations();
        let passed = manifest.complete()
            && manifest.dataset_window_count() > 0
            && !manifest.selected_polarizations().is_empty()
            && manifest
                .selected_polarizations()
                .iter()
                .all(|polarization| safe_polarization(polarization))
            && manifest.output_authority() == NISAR_SUBSET_OUTPUT_AUTHORITY
            && manifest.quality_status() == "PROVISIONAL"
            && selection_matches
            && manifest.artifact_size_bytes() > 0
            && is_sha256(manifest.artifact_sha256());
        self.update(|evidence| {
            evidence.output_authority = safe_atom(manifest.output_authority());
            evidence.output_class = if passed {
                "client_derived_provisional_nisar_rslc_aoi_subset".to_owned()
            } else {
                "not_promoted".to_owned()
            };
            evidence.one_hdf5 = passed;
            evidence.output_size_bytes = manifest.artifact_size_bytes();
            evidence.output_sha256 = safe_atom(manifest.artifact_sha256());
            evidence.atomic_promotion = passed;
            if let Some((line_start, line_end_exclusive, sample_start, sample_end_exclusive)) =
                manifest.actual_extent()
            {
                evidence.geometry = NisarLiveGeometryEvidenceV2 {
                    line_start,
                    line_end_exclusive,
                    sample_start,
                    sample_end_exclusive,
                };
            }
            evidence.scientific_validation = NisarLiveScientificValidationEvidenceV2 {
                complex_mismatch: if passed { 0 } else { 1 },
                amplitude_mismatch: if passed { 0 } else { 1 },
                wrapped_phase_mismatch: if passed { 0 } else { 1 },
                mask_mismatch: if passed { 0 } else { 1 },
                logical_science_equality: passed,
                identification_semantics: passed,
                dimension_scales: passed,
                metadata_semantics: passed,
                provenance_semantics: passed,
            };
        })
    }

    pub(crate) fn finish_pass(&self) -> io::Result<()> {
        self.update(|evidence| {
            evidence.status = "pass".to_owned();
            evidence.typed_stop_code = "PASS".to_owned();
        })
    }

    pub(crate) fn finish_failure(&self, code: &str) -> io::Result<()> {
        self.record_failure(code)
    }

    fn update(&self, update: impl FnOnce(&mut NisarLiveAcceptanceEvidenceV2)) -> io::Result<()> {
        let snapshot = {
            let mut evidence = self
                .state
                .lock()
                .map_err(|_| io::Error::other("M2D-B evidence state poisoned"))?;
            update(&mut evidence);
            recompute_aggregate_fields(&mut evidence)?;
            evidence.clone()
        };
        self.persist_snapshot(&snapshot)
    }

    fn persist(&self) -> io::Result<()> {
        let snapshot = self.snapshot();
        self.persist_snapshot(&snapshot)
    }

    fn persist_snapshot(&self, snapshot: &NisarLiveAcceptanceEvidenceV2) -> io::Result<()> {
        let bytes = serde_json::to_vec_pretty(snapshot)
            .map_err(|error| io::Error::other(format!("serialize M2D-B evidence: {error}")))?;
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(self.path.as_ref())?;
        file.write_all(&bytes)?;
        file.write_all(b"\n")?;
        file.sync_all()
    }
}

fn recompute_aggregate_fields(evidence: &mut NisarLiveAcceptanceEvidenceV2) -> io::Result<()> {
    let total_requests = evidence
        .preflight
        .requests
        .checked_add(evidence.metadata_prefix.requests)
        .and_then(|value| value.checked_add(evidence.metadata_discovery.requests))
        .and_then(|value| value.checked_add(evidence.additional_geolocation.requests))
        .and_then(|value| value.checked_add(evidence.planning_revalidation.requests))
        .and_then(|value| value.checked_add(evidence.charged_failure_requests))
        .and_then(|value| value.checked_add(evidence.science_range.requests))
        .and_then(|value| value.checked_add(evidence.failed_science_requests))
        .ok_or_else(|| io::Error::other("M2D-B request counter overflow"))?;
    let total_response_bytes = evidence
        .preflight
        .response_bytes
        .checked_add(evidence.metadata_prefix.response_bytes)
        .and_then(|value| value.checked_add(evidence.metadata_discovery.response_bytes))
        .and_then(|value| value.checked_add(evidence.additional_geolocation.response_bytes))
        .and_then(|value| value.checked_add(evidence.planning_revalidation.response_bytes))
        .and_then(|value| value.checked_add(evidence.charged_failure_response_bytes))
        .and_then(|value| value.checked_add(evidence.science_range.response_bytes))
        .and_then(|value| value.checked_add(evidence.failed_science_response_bytes))
        .ok_or_else(|| io::Error::other("M2D-B response counter overflow"))?;
    evidence.total_requests = total_requests;
    evidence.total_response_bytes = total_response_bytes;
    evidence.source_byte_ratio = if evidence.source_size_bytes == 0 {
        0.0
    } else {
        total_response_bytes as f64 / evidence.source_size_bytes as f64
    };
    Ok(())
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn safe_polarization(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 8
        && value.bytes().all(|byte| byte.is_ascii_uppercase())
        && matches!(value, "HH" | "HV" | "VH" | "VV")
}

fn safe_atom(value: &str) -> String {
    if value.len() > 256
        || value.contains("://")
        || value.contains('?')
        || value.contains('&')
        || value.contains('=')
        || value.to_ascii_lowercase().contains("authorization")
        || value.to_ascii_lowercase().contains("token")
        || value.to_ascii_lowercase().contains("cookie")
        || value.to_ascii_lowercase().contains("etag")
    {
        "[REDACTED]".to_owned()
    } else {
        value.to_owned()
    }
}

fn safe_stop_code(value: &str) -> String {
    if value.is_ascii()
        && !value.is_empty()
        && value.len() <= 96
        && value
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
    {
        value.to_owned()
    } else {
        "NISAR_LIVE_UNSAFE_STOP_CODE".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_root() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "nisar-live-evidence-v2-{}-{nonce}",
            std::process::id()
        ))
    }

    #[test]
    fn v2_persists_redacted_evidence_and_recomputes_stage_aggregates() {
        let root = test_root();
        let sink =
            NisarLiveAcceptanceEvidenceSink::new(root.clone(), "baseline-commit", "harness-commit")
                .unwrap();
        sink.record_scope(
            "granule",
            100,
            "sha256",
            &"a".repeat(64),
            NisarSubsetFrequency::A,
            "HH",
        )
        .unwrap();

        sink.record_preflight(
            2,
            10,
            "earthdata_asf_v1",
            "approved_nisar_hdf5_object",
            "https://example.invalid/?token=secret",
            "\"raw-etag-secret\"",
        )
        .unwrap();
        let preflight = sink.snapshot();
        assert_eq!(preflight.total_requests, 2);
        assert_eq!(preflight.total_response_bytes, 10);
        assert_eq!(preflight.source_byte_ratio, 0.1);

        sink.record_metadata_prefix(3, 20).unwrap();
        let prefix = sink.snapshot();
        assert_eq!(prefix.total_requests, 5);
        assert_eq!(prefix.total_response_bytes, 30);
        assert_eq!(prefix.source_byte_ratio, 0.3);

        sink.record_metadata_discovery(4, 30).unwrap();
        sink.record_science_range(5, 40).unwrap();
        let final_snapshot = sink.snapshot();
        assert_eq!(final_snapshot.total_requests, 14);
        assert_eq!(final_snapshot.total_response_bytes, 100);
        assert_eq!(final_snapshot.source_byte_ratio, 1.0);
        assert_eq!(final_snapshot.preflight_requests, 2);
        assert_eq!(final_snapshot.prefix_requests, 3);
        assert_eq!(final_snapshot.discovery_requests, 4);
        assert_eq!(final_snapshot.science_requests, 5);

        let raw = fs::read_to_string(sink.evidence_path()).unwrap();
        let json: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(json["schema_version"], NISAR_M2D_B_EVIDENCE_SCHEMA);
        for field in [
            "source_dataset_count",
            "metadata_group_binding_count",
            "metadata_dataset_binding_count",
            "metadata_binding_count",
            "inventory_sha256",
            "local_qualification_execution_plan_sha256",
            "execution_plan_sha256",
            "expected_semantic_plan_sha256",
            "semantic_plan_sha256",
        ] {
            assert!(json.get(field).is_some(), "missing V2 field {field}");
        }
        for forbidden in [
            "https://example.invalid",
            "raw-etag-secret",
            "authorization",
            "token",
            "cookie",
            ".netrc",
        ] {
            assert!(!raw.to_ascii_lowercase().contains(forbidden));
        }

        drop(sink);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn durable_science_failure_totals_survive_republication_without_charging_reserved_bytes() {
        let root = test_root();
        let sink =
            NisarLiveAcceptanceEvidenceSink::new(root.clone(), "baseline", "harness").unwrap();
        sink.record_science_range(24, 39_413_593).unwrap();
        for _ in 0..2 {
            sink.record_science_failures(2, 15_360).unwrap();
        }
        let evidence = sink.snapshot();
        assert_eq!(evidence.total_requests, 26);
        assert_eq!(evidence.total_response_bytes, 39_428_953);
        assert_eq!(evidence.science_response_bytes, 39_413_593);
        assert!(sink.record_science_failures(0, 1).is_err());
        drop(sink);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn resumed_planning_snapshots_replace_cached_records_without_hiding_science_duplicates() {
        let root = test_root();
        let sink =
            NisarLiveAcceptanceEvidenceSink::new(root.clone(), "baseline", "harness").unwrap();
        let digest = "a".repeat(64);
        let record = |stage| {
            sink.record_range_observation(NisarLiveRangeObservationInput {
                stage,
                start: 0,
                end_exclusive: 1,
                source_size_bytes: 100,
                request_count: 2,
                http_status: 206,
                response_body_bytes: 1,
                validator_sha256: &digest,
            })
            .unwrap()
        };
        record("science");
        record("science");
        for _ in 0..2 {
            sink.begin_planning_range_snapshot("metadata_prefix", 100, &digest)
                .unwrap();
            record("metadata_prefix");
        }
        let snapshot = sink.snapshot();
        assert_eq!(
            snapshot
                .range_observations
                .iter()
                .filter(|r| r.stage == "metadata_prefix")
                .count(),
            1
        );
        assert_eq!(
            snapshot
                .range_observations
                .iter()
                .filter(|r| r.stage == "science")
                .count(),
            2
        );
        assert!(sink
            .begin_planning_range_snapshot("science", 100, &digest)
            .is_err());
        assert!(sink
            .begin_planning_range_snapshot("metadata_prefix", 100, &"b".repeat(64))
            .is_err());
        assert_eq!(sink.snapshot().range_observations.len(), 3);
        drop(sink);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn v2_records_failed_range_attempts_separately_from_success_observations() {
        let root = test_root();
        let sink =
            NisarLiveAcceptanceEvidenceSink::new(root.clone(), "baseline-commit", "harness-commit")
                .unwrap();
        sink.record_scope(
            "granule",
            100,
            "sha256",
            &"a".repeat(64),
            NisarSubsetFrequency::A,
            "HH",
        )
        .unwrap();

        sink.record_range_failure_observation(NisarLiveRangeFailureObservationInput {
            stage: "preflight",
            start: 0,
            end_exclusive: 1,
            source_size_bytes: 100,
            disposition: "sent_response_rejected",
            request_count: 2,
            http_status: Some(200),
            response_body_bytes: Some(1),
            validator_sha256: None,
            failure_code: "NISAR_SUBSET_V2_RESPONSE_INVALID",
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
        })
        .unwrap();
        let snapshot = sink.snapshot();
        assert!(snapshot.range_observations.is_empty());
        assert_eq!(snapshot.range_failure_observations.len(), 1);
        let failure = &snapshot.range_failure_observations[0];
        assert_eq!(failure.disposition, "sent_response_rejected");
        assert_eq!(failure.request_count, 2);
        assert_eq!(failure.response_body_bytes, Some(1));
        assert_eq!(failure.http_status, Some(200));
        assert_eq!(failure.failure_code, "NISAR_SUBSET_V2_RESPONSE_INVALID");
        assert_eq!(snapshot.charged_failure_requests, 2);
        assert_eq!(snapshot.charged_failure_response_bytes, 1);
        assert_eq!(snapshot.total_requests, 2);
        assert_eq!(snapshot.total_response_bytes, 1);

        sink.record_range_failure_observation(NisarLiveRangeFailureObservationInput {
            stage: "preflight",
            start: 1,
            end_exclusive: 2,
            source_size_bytes: 100,
            disposition: "before_send_rejected",
            request_count: 0,
            http_status: None,
            response_body_bytes: None,
            validator_sha256: None,
            failure_code: "NISAR_SUBSET_V2_BOOTSTRAP_FAILED",
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
        })
        .unwrap();
        let snapshot = sink.snapshot();
        assert_eq!(snapshot.range_failure_observations.len(), 2);
        assert_eq!(snapshot.charged_failure_requests, 2);
        assert_eq!(snapshot.charged_failure_response_bytes, 1);
        assert_eq!(snapshot.total_requests, 2);
        assert_eq!(snapshot.total_response_bytes, 1);

        let raw = fs::read_to_string(sink.evidence_path()).unwrap();
        assert!(raw.contains("range_failure_observations"));
        assert!(!raw.contains("authorization"));
        drop(sink);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn v2_records_secret_free_redirect_rejection_diagnostics() {
        let root = test_root();
        let sink =
            NisarLiveAcceptanceEvidenceSink::new(root.clone(), "baseline-commit", "harness-commit")
                .unwrap();
        sink.record_scope(
            "granule",
            100,
            "sha256",
            &"a".repeat(64),
            NisarSubsetFrequency::A,
            "HH+HV",
        )
        .unwrap();

        sink.record_range_failure_observation(NisarLiveRangeFailureObservationInput {
            stage: "preflight",
            start: 0,
            end_exclusive: 1,
            source_size_bytes: 100,
            disposition: "sent_response_rejected",
            request_count: 1,
            http_status: Some(302),
            response_body_bytes: Some(0),
            validator_sha256: None,
            failure_code: "NISAR_SUBSET_V2_RESPONSE_INVALID",
            redirect_occurred: true,
            redirect_hop_index: Some(1),
            redirect_source_host_class: Some("loopback"),
            redirect_destination_host_class: Some("loopback"),
            redirect_source_path_class: Some("fixture_range"),
            redirect_destination_path_class: Some("fixture_range"),
            redirect_same_origin: Some(false),
            redirect_scheme_transition: Some("http-to-http"),
            redirect_policy_result: Some("rejected"),
            redirect_rejection_reason: Some("destination_policy_rejected"),
        })
        .unwrap();

        let snapshot = sink.snapshot();
        let failure = &snapshot.range_failure_observations[0];
        assert!(failure.redirect_occurred);
        assert_eq!(failure.redirect_hop_index, Some(1));
        assert_eq!(
            failure.redirect_source_host_class.as_deref(),
            Some("loopback")
        );
        assert_eq!(
            failure.redirect_destination_host_class.as_deref(),
            Some("loopback")
        );
        assert_eq!(failure.redirect_same_origin, Some(false));
        assert_eq!(
            failure.redirect_scheme_transition.as_deref(),
            Some("http-to-http")
        );
        assert_eq!(failure.redirect_policy_result.as_deref(), Some("rejected"));
        assert_eq!(
            failure.redirect_rejection_reason.as_deref(),
            Some("destination_policy_rejected")
        );
        assert_eq!(snapshot.charged_failure_requests, 1);
        assert_eq!(snapshot.charged_failure_response_bytes, 0);

        let raw = fs::read_to_string(sink.evidence_path()).unwrap();
        for forbidden in ["Authorization", "Cookie", "Bearer ", ".netrc"] {
            assert!(!raw.contains(forbidden));
        }
        drop(sink);
        fs::remove_dir_all(root).unwrap();
    }
}
