//! Exhaustive A2-B evidence for exact-adcode DataV geometry resolution.
//!
//! This diagnostic module consumes identity exclusively from a loaded China
//! administrative authority. It is intentionally not wired into the current UI
//! or production selector route.

use crate::{
    ChinaAdminAuthority, ChinaAdminAuthorityStatus, ChinaAdminGeometryResolveRequest,
    ChinaAdminGeometryResult, ChinaAdminLevel, ChinaAdminUnit, ChinaAdminUnitType,
};
use insar_provider::boundary::{
    DataVBoundaryBatch, DataVBoundaryProvider, ProviderBoundaryFeature, DATAV_BOUNDARY_ROOT,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::Instant;
use thiserror::Error;

pub const DATAV_GEOMETRY_VERIFICATION_SCHEMA_VERSION: &str =
    "aoi-a2b-datav-geometry-verification-2.0.0";
pub const DATAV_EXACT_GEOMETRY_RESOLVER_ID: &str = "datav-areas-v3-exact-adcode";
pub const DATAV_BATCH_GEOMETRY_RESOLVER_ID: &str = "datav-areas-v3-full-batch-adcode-index";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatavGeometryExpectedUnit {
    pub adcode: String,
    pub name_zh: String,
    pub administrative_level: ChinaAdminLevel,
    pub unit_type: ChinaAdminUnitType,
    pub parent_adcode: Option<String>,
    pub supported: bool,
    pub selectable: bool,
    pub special_hierarchy: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DatavGeometryOutcome {
    Pass,
    Unavailable,
    InvalidGeometry,
    VerificationBlocked,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DatavGeometryTransportKind {
    #[default]
    Exact,
    FullBatch,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatavGeometryUnitEvidence {
    pub adcode: String,
    pub name_zh: String,
    pub administrative_level: ChinaAdminLevel,
    pub unit_type: ChinaAdminUnitType,
    pub parent_adcode: Option<String>,
    pub special_hierarchy: bool,
    #[serde(default)]
    pub transport_kind: DatavGeometryTransportKind,
    #[serde(default)]
    pub transport_adcode: Option<String>,
    #[serde(default)]
    pub reused_prior_pass: bool,
    pub requested_endpoint: String,
    pub outcome: DatavGeometryOutcome,
    pub identity_preserved: bool,
    pub authoritative_topology_valid: bool,
    pub bbox_geometry_derived_equal: bool,
    pub geometry_kind: Option<String>,
    pub bbox: Option<[f64; 4]>,
    pub source: Option<String>,
    pub error_code: Option<String>,
    pub error_reason: Option<String>,
    pub elapsed_ms: u64,
}

impl DatavGeometryUnitEvidence {
    pub fn passed(
        unit: &DatavGeometryExpectedUnit,
        geometry_kind: &str,
        bbox: [f64; 4],
        elapsed_ms: u64,
    ) -> Self {
        Self {
            adcode: unit.adcode.clone(),
            name_zh: unit.name_zh.clone(),
            administrative_level: unit.administrative_level,
            unit_type: unit.unit_type,
            parent_adcode: unit.parent_adcode.clone(),
            special_hierarchy: unit.special_hierarchy,
            transport_kind: DatavGeometryTransportKind::Exact,
            transport_adcode: Some(unit.adcode.clone()),
            reused_prior_pass: false,
            requested_endpoint: exact_endpoint(&unit.adcode),
            outcome: DatavGeometryOutcome::Pass,
            identity_preserved: true,
            authoritative_topology_valid: true,
            bbox_geometry_derived_equal: true,
            geometry_kind: Some(geometry_kind.to_owned()),
            bbox: Some(bbox),
            source: Some("DataV areas_v3".to_owned()),
            error_code: None,
            error_reason: None,
            elapsed_ms,
        }
    }

    pub fn failed(
        template: &Self,
        outcome: DatavGeometryOutcome,
        error_code: &str,
        error_reason: &str,
        elapsed_ms: u64,
    ) -> Self {
        debug_assert!(outcome != DatavGeometryOutcome::Pass);
        Self {
            adcode: template.adcode.clone(),
            name_zh: template.name_zh.clone(),
            administrative_level: template.administrative_level,
            unit_type: template.unit_type,
            parent_adcode: template.parent_adcode.clone(),
            special_hierarchy: template.special_hierarchy,
            transport_kind: template.transport_kind,
            transport_adcode: template.transport_adcode.clone(),
            reused_prior_pass: template.reused_prior_pass,
            requested_endpoint: template.requested_endpoint.clone(),
            outcome,
            identity_preserved: false,
            authoritative_topology_valid: false,
            bbox_geometry_derived_equal: false,
            geometry_kind: None,
            bbox: None,
            source: None,
            error_code: Some(error_code.to_owned()),
            error_reason: Some(error_reason.to_owned()),
            elapsed_ms,
        }
    }

    fn failed_unit(
        unit: &DatavGeometryExpectedUnit,
        outcome: DatavGeometryOutcome,
        error_code: &str,
        error_reason: &str,
        elapsed_ms: u64,
    ) -> Self {
        let template = Self::passed(unit, "polygon", [0.0, 0.0, 1.0, 1.0], elapsed_ms);
        Self::failed(&template, outcome, error_code, error_reason, elapsed_ms)
    }

    fn passed_from_batch(
        unit: &DatavGeometryExpectedUnit,
        transport_adcode: &str,
        geometry_kind: &str,
        bbox: [f64; 4],
        source: String,
        elapsed_ms: u64,
    ) -> Self {
        Self {
            adcode: unit.adcode.clone(),
            name_zh: unit.name_zh.clone(),
            administrative_level: unit.administrative_level,
            unit_type: unit.unit_type,
            parent_adcode: unit.parent_adcode.clone(),
            special_hierarchy: unit.special_hierarchy,
            transport_kind: DatavGeometryTransportKind::FullBatch,
            transport_adcode: Some(transport_adcode.to_owned()),
            reused_prior_pass: false,
            requested_endpoint: batch_endpoint(transport_adcode),
            outcome: DatavGeometryOutcome::Pass,
            identity_preserved: true,
            authoritative_topology_valid: true,
            bbox_geometry_derived_equal: true,
            geometry_kind: Some(geometry_kind.to_owned()),
            bbox: Some(bbox),
            source: Some(source),
            error_code: None,
            error_reason: None,
            elapsed_ms,
        }
    }

    fn invalid_from_batch(
        unit: &DatavGeometryExpectedUnit,
        transport_adcode: &str,
        code: &str,
        reason: &str,
        elapsed_ms: u64,
    ) -> Self {
        Self {
            adcode: unit.adcode.clone(),
            name_zh: unit.name_zh.clone(),
            administrative_level: unit.administrative_level,
            unit_type: unit.unit_type,
            parent_adcode: unit.parent_adcode.clone(),
            special_hierarchy: unit.special_hierarchy,
            transport_kind: DatavGeometryTransportKind::FullBatch,
            transport_adcode: Some(transport_adcode.to_owned()),
            reused_prior_pass: false,
            requested_endpoint: batch_endpoint(transport_adcode),
            outcome: DatavGeometryOutcome::InvalidGeometry,
            identity_preserved: true,
            authoritative_topology_valid: false,
            bbox_geometry_derived_equal: false,
            geometry_kind: None,
            bbox: None,
            source: Some("DataV areas_v3 batch".to_owned()),
            error_code: Some(code.to_owned()),
            error_reason: Some(reason.to_owned()),
            elapsed_ms,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatavGeometryRequestStats {
    pub batch_requests: usize,
    pub exact_fallback_requests: usize,
    pub rate_limit_events: usize,
    pub reused_pass_results: usize,
}

pub const DATAV_BATCH_RESUME_SCHEMA_VERSION: &str = "aoi-a2b1-datav-batch-resume-1.0.0";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DatavTransportRequestOutcome {
    Pass,
    Unavailable,
    InvalidResponse,
    VerificationBlocked,
    RateLimited,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatavTransportRequestEvidence {
    pub sequence: usize,
    pub transport_kind: DatavGeometryTransportKind,
    pub transport_adcode: String,
    pub requested_endpoint: String,
    pub outcome: DatavTransportRequestOutcome,
    pub elapsed_ms: u64,
    pub matched_pass: usize,
    pub matched_invalid_geometry: usize,
    pub ignored_non_mca_features: usize,
    pub error_code: Option<String>,
    pub error_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatavBatchResumeState {
    pub schema_version: String,
    pub authority_version: String,
    pub authority_content_sha256: String,
    pub planned_batch_adcodes: Vec<String>,
    pub completed_batch_adcodes: BTreeSet<String>,
    pub requests: Vec<DatavTransportRequestEvidence>,
    pub stopped_on_rate_limit: bool,
}

impl DatavBatchResumeState {
    pub fn new(authority: &ChinaAdminAuthority, planned_batch_adcodes: Vec<String>) -> Self {
        Self {
            schema_version: DATAV_BATCH_RESUME_SCHEMA_VERSION.to_owned(),
            authority_version: authority.authority_version().to_owned(),
            authority_content_sha256: authority.content_sha256().to_owned(),
            planned_batch_adcodes,
            completed_batch_adcodes: BTreeSet::new(),
            requests: Vec::new(),
            stopped_on_rate_limit: false,
        }
    }

    pub fn begin_resume(&mut self) {
        self.stopped_on_rate_limit = false;
    }

    pub fn record_request(
        &mut self,
        request: DatavTransportRequestEvidence,
    ) -> Result<(), DatavGeometryVerificationError> {
        self.validate_request(&request)?;
        if request.sequence != self.requests.len() + 1 {
            return Err(DatavGeometryVerificationError::Evidence(format!(
                "request sequence {} is not the next durable sequence {}",
                request.sequence,
                self.requests.len() + 1
            )));
        }
        if request.transport_kind == DatavGeometryTransportKind::FullBatch
            && matches!(
                request.outcome,
                DatavTransportRequestOutcome::Pass
                    | DatavTransportRequestOutcome::Unavailable
                    | DatavTransportRequestOutcome::InvalidResponse
            )
        {
            self.completed_batch_adcodes
                .insert(request.transport_adcode.clone());
        }
        if request.outcome == DatavTransportRequestOutcome::RateLimited {
            self.stopped_on_rate_limit = true;
        }
        self.requests.push(request);
        Ok(())
    }

    pub fn remaining_batch_adcodes(&self) -> Vec<String> {
        self.planned_batch_adcodes
            .iter()
            .filter(|adcode| !self.completed_batch_adcodes.contains(adcode.as_str()))
            .cloned()
            .collect()
    }

    pub fn request_stats(&self, reused_pass_results: usize) -> DatavGeometryRequestStats {
        DatavGeometryRequestStats {
            batch_requests: self
                .requests
                .iter()
                .filter(|request| request.transport_kind == DatavGeometryTransportKind::FullBatch)
                .count(),
            exact_fallback_requests: self
                .requests
                .iter()
                .filter(|request| request.transport_kind == DatavGeometryTransportKind::Exact)
                .count(),
            rate_limit_events: self
                .requests
                .iter()
                .filter(|request| request.outcome == DatavTransportRequestOutcome::RateLimited)
                .count(),
            reused_pass_results,
        }
    }

    pub fn validate_against(
        &self,
        authority: &ChinaAdminAuthority,
        planned_batch_adcodes: &[String],
    ) -> Result<(), DatavGeometryVerificationError> {
        if self.schema_version != DATAV_BATCH_RESUME_SCHEMA_VERSION
            || self.authority_version != authority.authority_version()
            || self.authority_content_sha256 != authority.content_sha256()
            || self.planned_batch_adcodes != planned_batch_adcodes
        {
            return Err(DatavGeometryVerificationError::Evidence(
                "resume state does not match the active MCA authority and batch plan".to_owned(),
            ));
        }
        if self
            .completed_batch_adcodes
            .iter()
            .any(|adcode| !self.planned_batch_adcodes.contains(adcode))
        {
            return Err(DatavGeometryVerificationError::Evidence(
                "resume state contains a completed batch outside the MCA plan".to_owned(),
            ));
        }
        for (index, request) in self.requests.iter().enumerate() {
            if request.sequence != index + 1 {
                return Err(DatavGeometryVerificationError::Evidence(
                    "resume request sequence is not contiguous".to_owned(),
                ));
            }
            self.validate_request(request)?;
        }
        Ok(())
    }

    fn validate_request(
        &self,
        request: &DatavTransportRequestEvidence,
    ) -> Result<(), DatavGeometryVerificationError> {
        validate_six_digit_adcode("request transport", &request.transport_adcode)?;
        let expected_endpoint = match request.transport_kind {
            DatavGeometryTransportKind::Exact => exact_endpoint(&request.transport_adcode),
            DatavGeometryTransportKind::FullBatch => {
                if !self
                    .planned_batch_adcodes
                    .contains(&request.transport_adcode)
                {
                    return Err(DatavGeometryVerificationError::Evidence(format!(
                        "batch request {} is outside the MCA-derived plan",
                        request.transport_adcode
                    )));
                }
                batch_endpoint(&request.transport_adcode)
            }
        };
        if request.requested_endpoint != expected_endpoint {
            return Err(DatavGeometryVerificationError::Evidence(format!(
                "request endpoint does not preserve transport identity {}",
                request.transport_adcode
            )));
        }
        match request.outcome {
            DatavTransportRequestOutcome::Pass => {
                if request.error_code.is_some() || request.error_reason.is_some() {
                    return Err(DatavGeometryVerificationError::Evidence(
                        "successful transport request cannot carry an error".to_owned(),
                    ));
                }
            }
            _ => {
                if request.error_code.as_deref().is_none_or(str::is_empty)
                    || request.error_reason.as_deref().is_none_or(str::is_empty)
                {
                    return Err(DatavGeometryVerificationError::Evidence(
                        "non-success transport request requires code and reason".to_owned(),
                    ));
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatavBatchApplyResult {
    pub matched_pass: usize,
    pub matched_invalid_geometry: usize,
    pub reused_pass_skipped: usize,
    pub ignored_non_mca_features: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatavGeometryBreakdown {
    pub total_expected: usize,
    pub geometry_pass: usize,
    pub geometry_fail: usize,
    pub unavailable: usize,
    pub invalid_geometry: usize,
    pub verification_blocked: usize,
}

impl DatavGeometryBreakdown {
    fn record(&mut self, outcome: DatavGeometryOutcome) {
        self.total_expected += 1;
        match outcome {
            DatavGeometryOutcome::Pass => self.geometry_pass += 1,
            DatavGeometryOutcome::Unavailable => {
                self.geometry_fail += 1;
                self.unavailable += 1;
            }
            DatavGeometryOutcome::InvalidGeometry => {
                self.geometry_fail += 1;
                self.invalid_geometry += 1;
            }
            DatavGeometryOutcome::VerificationBlocked => {
                self.verification_blocked += 1;
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum DatavGeometryGateStatus {
    Pass,
    Fail,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatavGeometryVerificationReport {
    pub schema_version: String,
    pub authority_version: String,
    pub authority_status: ChinaAdminAuthorityStatus,
    pub authority_effective_date: Option<String>,
    pub authority_content_sha256: String,
    pub identity_authority: String,
    pub geometry_resolver: String,
    pub endpoint_template: String,
    pub batch_endpoint_template: String,
    pub started_at: String,
    pub finished_at: String,
    pub workers: usize,
    pub total_expected: usize,
    pub geometry_pass: usize,
    pub geometry_fail: usize,
    pub unavailable_adcodes: Vec<String>,
    pub invalid_geometry_adcodes: Vec<String>,
    pub verification_blocked_adcodes: Vec<String>,
    pub verification_blocked: usize,
    pub batch_requests: usize,
    pub exact_fallback_requests: usize,
    pub rate_limit_events: usize,
    pub reused_pass_results: usize,
    pub run_admissible: bool,
    pub breakdown: BTreeMap<String, DatavGeometryBreakdown>,
    pub a2b_datav_geometry_gate: DatavGeometryGateStatus,
    pub datav_license_status: String,
    pub datav_distribution_risk_blocks_technical_verification: bool,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum DatavGeometryVerificationError {
    #[error("authority enumeration failed: {0}")]
    Authority(String),
    #[error("evidence contract failed: {0}")]
    Evidence(String),
}

pub fn enumerate_datav_geometry_expected_units(
    authority: &ChinaAdminAuthority,
) -> Result<Vec<DatavGeometryExpectedUnit>, DatavGeometryVerificationError> {
    let mut pending = VecDeque::from(authority.list_china_admin_roots());
    let mut seen = BTreeSet::new();
    let mut units = Vec::new();

    while let Some(unit) = pending.pop_front() {
        if !seen.insert(unit.adcode.clone()) {
            return Err(DatavGeometryVerificationError::Authority(format!(
                "duplicate active adcode {}",
                unit.adcode
            )));
        }
        if !unit.supported || !unit.selectable {
            return Err(DatavGeometryVerificationError::Authority(format!(
                "inactive unit {} reached active selector enumeration",
                unit.adcode
            )));
        }
        units.push(expected_unit(authority, unit));
        let children = authority
            .list_china_admin_children(&unit.adcode)
            .map_err(|error| DatavGeometryVerificationError::Authority(error.to_string()))?;
        pending.extend(children);
    }

    units.sort_by(|left, right| left.adcode.cmp(&right.adcode));
    Ok(units)
}

pub fn enumerate_datav_geometry_batch_adcodes(
    authority: &ChinaAdminAuthority,
) -> Result<Vec<String>, DatavGeometryVerificationError> {
    let expected = enumerate_datav_geometry_expected_units(authority)?;
    let active_adcodes = expected
        .iter()
        .map(|unit| unit.adcode.as_str())
        .collect::<BTreeSet<_>>();
    let batch_adcodes = expected
        .iter()
        .filter_map(|unit| unit.parent_adcode.as_ref())
        .filter(|parent| active_adcodes.contains(parent.as_str()))
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    Ok(batch_adcodes)
}

pub fn mark_reusable_pass_evidence(evidence: &mut [DatavGeometryUnitEvidence]) -> usize {
    let mut reused = 0;
    for row in evidence {
        if row.outcome == DatavGeometryOutcome::Pass {
            row.reused_prior_pass = true;
            row.transport_kind = DatavGeometryTransportKind::Exact;
            row.transport_adcode = Some(row.adcode.clone());
            row.requested_endpoint = exact_endpoint(&row.adcode);
            reused += 1;
        }
    }
    reused
}

pub fn apply_datav_geometry_batch(
    authority_version: &str,
    expected: &[DatavGeometryExpectedUnit],
    evidence: &mut [DatavGeometryUnitEvidence],
    batch: DataVBoundaryBatch,
    elapsed_ms: u64,
) -> Result<DatavBatchApplyResult, DatavGeometryVerificationError> {
    if expected.len() != evidence.len() {
        return Err(DatavGeometryVerificationError::Evidence(
            "batch merge requires one evidence row per expected unit".to_owned(),
        ));
    }
    validate_six_digit_adcode("batch transport", &batch.transport_adcode)?;
    let expected_index = expected
        .iter()
        .enumerate()
        .map(|(index, unit)| (unit.adcode.as_str(), index))
        .collect::<BTreeMap<_, _>>();
    let evidence_index = evidence
        .iter()
        .enumerate()
        .map(|(index, row)| (row.adcode.as_str(), index))
        .collect::<BTreeMap<_, _>>();
    if expected_index.keys().ne(evidence_index.keys()) {
        return Err(DatavGeometryVerificationError::Evidence(
            "batch merge evidence identities do not equal MCA expected identities".to_owned(),
        ));
    }

    let mut applied = DatavBatchApplyResult::default();
    for (adcode, feature) in batch.features {
        if feature.provider_object_id != adcode {
            return Err(DatavGeometryVerificationError::Evidence(format!(
                "batch feature map key does not preserve provider identity {adcode}"
            )));
        }
        let Some(index) = expected_index.get(adcode.as_str()).copied() else {
            applied.ignored_non_mca_features += 1;
            continue;
        };
        if evidence[index].outcome == DatavGeometryOutcome::Pass {
            applied.reused_pass_skipped += 1;
            continue;
        }
        evidence[index] = verify_datav_provider_feature(
            authority_version,
            &expected[index],
            feature,
            DatavGeometryTransportKind::FullBatch,
            &batch.transport_adcode,
            elapsed_ms,
        );
        match evidence[index].outcome {
            DatavGeometryOutcome::Pass => applied.matched_pass += 1,
            DatavGeometryOutcome::InvalidGeometry => applied.matched_invalid_geometry += 1,
            _ => {}
        }
    }

    for invalid in batch.invalid_features {
        let Some(adcode) = invalid.provider_object_id else {
            applied.ignored_non_mca_features += 1;
            continue;
        };
        let Some(index) = expected_index.get(adcode.as_str()).copied() else {
            applied.ignored_non_mca_features += 1;
            continue;
        };
        if evidence[index].outcome == DatavGeometryOutcome::Pass {
            applied.reused_pass_skipped += 1;
            continue;
        }
        evidence[index] = DatavGeometryUnitEvidence::invalid_from_batch(
            &expected[index],
            &batch.transport_adcode,
            &invalid.code,
            &invalid.reason,
            elapsed_ms,
        );
        applied.matched_invalid_geometry += 1;
    }
    Ok(applied)
}

pub fn bounded_exact_fallback_units(
    expected: &[DatavGeometryExpectedUnit],
    evidence: &[DatavGeometryUnitEvidence],
    limit: usize,
) -> Result<Vec<DatavGeometryExpectedUnit>, DatavGeometryVerificationError> {
    if expected.len() != evidence.len() {
        return Err(DatavGeometryVerificationError::Evidence(
            "fallback selection requires one evidence row per expected unit".to_owned(),
        ));
    }
    let evidence_by_adcode = evidence
        .iter()
        .map(|row| (row.adcode.as_str(), row))
        .collect::<BTreeMap<_, _>>();
    let mut fallback = Vec::new();
    for unit in expected {
        let row = evidence_by_adcode
            .get(unit.adcode.as_str())
            .ok_or_else(|| {
                DatavGeometryVerificationError::Evidence(format!(
                    "missing evidence for MCA unit {}",
                    unit.adcode
                ))
            })?;
        if row.outcome == DatavGeometryOutcome::VerificationBlocked {
            fallback.push(unit.clone());
            if fallback.len() == limit {
                break;
            }
        }
    }
    Ok(fallback)
}

pub fn verify_datav_geometry_unit(
    provider: &DataVBoundaryProvider,
    authority_version: &str,
    unit: &DatavGeometryExpectedUnit,
) -> DatavGeometryUnitEvidence {
    let started = Instant::now();
    let result = provider.resolve_exact_adcode(&unit.adcode);
    let elapsed_ms = || started.elapsed().as_millis().min(u64::MAX as u128) as u64;

    let feature = match result {
        Ok(feature) => feature,
        Err(error) => {
            let http_status = error
                .debug_context
                .get("http_status")
                .and_then(serde_json::Value::as_u64);
            let outcome = match (error.code.as_str(), http_status) {
                ("BOUNDARY_PROVIDER_HTTP_ERROR", Some(404 | 410)) => {
                    DatavGeometryOutcome::Unavailable
                }
                ("BOUNDARY_PROVIDER_RATE_LIMITED", _)
                | ("BOUNDARY_PROVIDER_UNAVAILABLE", _)
                | ("BOUNDARY_PROVIDER_HTTP_ERROR", _) => DatavGeometryOutcome::VerificationBlocked,
                _ => DatavGeometryOutcome::InvalidGeometry,
            };
            let reason = error
                .debug_context
                .get("reason")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("provider_error");
            return DatavGeometryUnitEvidence::failed_unit(
                unit,
                outcome,
                &error.code,
                reason,
                elapsed_ms(),
            );
        }
    };

    verify_datav_provider_feature(
        authority_version,
        unit,
        feature,
        DatavGeometryTransportKind::Exact,
        &unit.adcode,
        elapsed_ms(),
    )
}

fn verify_datav_provider_feature(
    authority_version: &str,
    unit: &DatavGeometryExpectedUnit,
    feature: ProviderBoundaryFeature,
    transport_kind: DatavGeometryTransportKind,
    transport_adcode: &str,
    elapsed_ms: u64,
) -> DatavGeometryUnitEvidence {
    let request = ChinaAdminGeometryResolveRequest {
        authority_version: authority_version.to_owned(),
        adcode: unit.adcode.clone(),
    };
    let resolved = ChinaAdminGeometryResult {
        authority_version: authority_version.to_owned(),
        adcode: feature.provider_object_id.clone(),
        resolver_id: DATAV_EXACT_GEOMETRY_RESOLVER_ID.to_owned(),
        geometry: feature.geometry,
        bbox: feature.bbox,
        source: feature.source.clone(),
        attribution: feature.attribution,
    };
    if let Err(error) = resolved.validate_against(&request) {
        let code = format!("{:?}", error.code());
        return match transport_kind {
            DatavGeometryTransportKind::Exact => DatavGeometryUnitEvidence::failed_unit(
                unit,
                DatavGeometryOutcome::InvalidGeometry,
                &code,
                error.detail(),
                elapsed_ms,
            ),
            DatavGeometryTransportKind::FullBatch => DatavGeometryUnitEvidence::invalid_from_batch(
                unit,
                transport_adcode,
                &code,
                error.detail(),
                elapsed_ms,
            ),
        };
    }

    match transport_kind {
        DatavGeometryTransportKind::Exact => {
            let mut evidence = DatavGeometryUnitEvidence::passed(
                unit,
                &feature.geometry_kind,
                resolved.bbox,
                elapsed_ms,
            );
            evidence.source = Some(feature.source);
            evidence
        }
        DatavGeometryTransportKind::FullBatch => DatavGeometryUnitEvidence::passed_from_batch(
            unit,
            transport_adcode,
            &feature.geometry_kind,
            resolved.bbox,
            feature.source,
            elapsed_ms,
        ),
    }
}

pub fn build_datav_geometry_verification_report(
    authority: &ChinaAdminAuthority,
    started_at: &str,
    finished_at: &str,
    workers: usize,
    expected: &[DatavGeometryExpectedUnit],
    evidence: &[DatavGeometryUnitEvidence],
) -> Result<DatavGeometryVerificationReport, DatavGeometryVerificationError> {
    build_datav_geometry_verification_report_with_stats(
        authority,
        started_at,
        finished_at,
        workers,
        expected,
        evidence,
        DatavGeometryRequestStats::default(),
    )
}

pub fn build_datav_geometry_verification_report_with_stats(
    authority: &ChinaAdminAuthority,
    started_at: &str,
    finished_at: &str,
    workers: usize,
    expected: &[DatavGeometryExpectedUnit],
    evidence: &[DatavGeometryUnitEvidence],
    stats: DatavGeometryRequestStats,
) -> Result<DatavGeometryVerificationReport, DatavGeometryVerificationError> {
    if workers == 0 {
        return Err(DatavGeometryVerificationError::Evidence(
            "workers must be greater than zero".to_owned(),
        ));
    }
    if expected.len() != evidence.len() {
        return Err(DatavGeometryVerificationError::Evidence(format!(
            "expected {} rows but received {}",
            expected.len(),
            evidence.len()
        )));
    }

    let expected_by_adcode = expected
        .iter()
        .map(|unit| (unit.adcode.as_str(), unit))
        .collect::<BTreeMap<_, _>>();
    let mut seen = BTreeSet::new();
    let mut breakdown = BTreeMap::from([
        ("country".to_owned(), DatavGeometryBreakdown::default()),
        ("province".to_owned(), DatavGeometryBreakdown::default()),
        ("prefecture".to_owned(), DatavGeometryBreakdown::default()),
        ("county".to_owned(), DatavGeometryBreakdown::default()),
        (
            "special_hierarchy".to_owned(),
            DatavGeometryBreakdown::default(),
        ),
    ]);
    let mut unavailable_adcodes = Vec::new();
    let mut invalid_geometry_adcodes = Vec::new();
    let mut verification_blocked_adcodes = Vec::new();

    for row in evidence {
        if !seen.insert(row.adcode.as_str()) {
            return Err(DatavGeometryVerificationError::Evidence(format!(
                "duplicate evidence for {}",
                row.adcode
            )));
        }
        let expected_unit = expected_by_adcode.get(row.adcode.as_str()).ok_or_else(|| {
            DatavGeometryVerificationError::Evidence(format!(
                "unexpected evidence adcode {}",
                row.adcode
            ))
        })?;
        validate_evidence_identity(expected_unit, row)?;
        let level_key = level_key(row.administrative_level);
        breakdown
            .get_mut(level_key)
            .expect("all authority levels have breakdown buckets")
            .record(row.outcome);
        if row.special_hierarchy {
            breakdown
                .get_mut("special_hierarchy")
                .expect("special hierarchy bucket exists")
                .record(row.outcome);
        }
        match row.outcome {
            DatavGeometryOutcome::Pass => {
                if !row.identity_preserved
                    || !row.authoritative_topology_valid
                    || !row.bbox_geometry_derived_equal
                    || row.geometry_kind.as_deref().is_none_or(|kind| {
                        !matches!(
                            kind.to_ascii_lowercase().as_str(),
                            "polygon" | "multipolygon"
                        )
                    })
                    || row.bbox.is_none()
                    || row.error_code.is_some()
                {
                    return Err(DatavGeometryVerificationError::Evidence(format!(
                        "PASS evidence contract is incomplete for {}",
                        row.adcode
                    )));
                }
            }
            DatavGeometryOutcome::Unavailable => unavailable_adcodes.push(row.adcode.clone()),
            DatavGeometryOutcome::InvalidGeometry => {
                invalid_geometry_adcodes.push(row.adcode.clone())
            }
            DatavGeometryOutcome::VerificationBlocked => {
                verification_blocked_adcodes.push(row.adcode.clone())
            }
        }
    }
    if seen.len() != expected_by_adcode.len() {
        return Err(DatavGeometryVerificationError::Evidence(
            "one or more expected adcodes have no evidence".to_owned(),
        ));
    }

    unavailable_adcodes.sort();
    invalid_geometry_adcodes.sort();
    verification_blocked_adcodes.sort();
    let geometry_fail = unavailable_adcodes.len() + invalid_geometry_adcodes.len();
    let verification_blocked = verification_blocked_adcodes.len();
    let geometry_pass = expected.len() - geometry_fail - verification_blocked;

    Ok(DatavGeometryVerificationReport {
        schema_version: DATAV_GEOMETRY_VERIFICATION_SCHEMA_VERSION.to_owned(),
        authority_version: authority.authority_version().to_owned(),
        authority_status: authority.authority_status(),
        authority_effective_date: authority.effective_date().map(ToOwned::to_owned),
        authority_content_sha256: authority.content_sha256().to_owned(),
        identity_authority: "Ministry of Civil Affairs production-candidate snapshot".to_owned(),
        geometry_resolver: "datav-full-batch-with-bounded-exact-fallback".to_owned(),
        endpoint_template: format!("{DATAV_BOUNDARY_ROOT}/{{adcode}}.json"),
        batch_endpoint_template: format!("{DATAV_BOUNDARY_ROOT}/{{adcode}}_full.json"),
        started_at: started_at.to_owned(),
        finished_at: finished_at.to_owned(),
        workers,
        total_expected: expected.len(),
        geometry_pass,
        geometry_fail,
        unavailable_adcodes,
        invalid_geometry_adcodes,
        run_admissible: verification_blocked_adcodes.is_empty(),
        verification_blocked,
        batch_requests: stats.batch_requests,
        exact_fallback_requests: stats.exact_fallback_requests,
        rate_limit_events: stats.rate_limit_events,
        reused_pass_results: stats.reused_pass_results,
        verification_blocked_adcodes,
        breakdown,
        a2b_datav_geometry_gate: if geometry_pass == expected.len() {
            DatavGeometryGateStatus::Pass
        } else {
            DatavGeometryGateStatus::Fail
        },
        datav_license_status: "release_distribution_review_required".to_owned(),
        datav_distribution_risk_blocks_technical_verification: false,
    })
}

fn expected_unit(
    authority: &ChinaAdminAuthority,
    unit: &ChinaAdminUnit,
) -> DatavGeometryExpectedUnit {
    DatavGeometryExpectedUnit {
        adcode: unit.adcode.clone(),
        name_zh: unit.name_zh.clone(),
        administrative_level: unit.administrative_level,
        unit_type: unit.unit_type,
        parent_adcode: unit.parent_adcode.clone(),
        supported: unit.supported,
        selectable: unit.selectable,
        special_hierarchy: authority
            .is_special_hierarchy_adcode(&unit.adcode)
            .unwrap_or(false),
    }
}

fn validate_evidence_identity(
    expected: &DatavGeometryExpectedUnit,
    row: &DatavGeometryUnitEvidence,
) -> Result<(), DatavGeometryVerificationError> {
    if row.name_zh != expected.name_zh
        || row.administrative_level != expected.administrative_level
        || row.unit_type != expected.unit_type
        || row.parent_adcode != expected.parent_adcode
        || row.special_hierarchy != expected.special_hierarchy
    {
        return Err(DatavGeometryVerificationError::Evidence(format!(
            "authority identity metadata changed for {}",
            expected.adcode
        )));
    }
    match row.transport_kind {
        DatavGeometryTransportKind::Exact => {
            if row.requested_endpoint != exact_endpoint(&expected.adcode)
                || row
                    .transport_adcode
                    .as_deref()
                    .is_some_and(|adcode| adcode != expected.adcode)
            {
                return Err(DatavGeometryVerificationError::Evidence(format!(
                    "exact transport identity changed for {}",
                    expected.adcode
                )));
            }
        }
        DatavGeometryTransportKind::FullBatch => {
            let transport_adcode = row.transport_adcode.as_deref().ok_or_else(|| {
                DatavGeometryVerificationError::Evidence(format!(
                    "batch transport adcode missing for {}",
                    expected.adcode
                ))
            })?;
            validate_six_digit_adcode("batch transport", transport_adcode)?;
            if row.requested_endpoint != batch_endpoint(transport_adcode) {
                return Err(DatavGeometryVerificationError::Evidence(format!(
                    "batch transport endpoint changed for {}",
                    expected.adcode
                )));
            }
        }
    }
    Ok(())
}

fn exact_endpoint(adcode: &str) -> String {
    format!("{DATAV_BOUNDARY_ROOT}/{adcode}.json")
}

fn batch_endpoint(adcode: &str) -> String {
    format!("{DATAV_BOUNDARY_ROOT}/{adcode}_full.json")
}

fn validate_six_digit_adcode(
    label: &str,
    adcode: &str,
) -> Result<(), DatavGeometryVerificationError> {
    if adcode.len() != 6 || !adcode.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(DatavGeometryVerificationError::Evidence(format!(
            "{label} adcode must be exactly six ASCII digits"
        )));
    }
    Ok(())
}

fn level_key(level: ChinaAdminLevel) -> &'static str {
    match level {
        ChinaAdminLevel::Country => "country",
        ChinaAdminLevel::Province => "province",
        ChinaAdminLevel::Prefecture => "prefecture",
        ChinaAdminLevel::County => "county",
    }
}
