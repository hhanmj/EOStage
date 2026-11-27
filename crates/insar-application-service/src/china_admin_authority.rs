//! Versioned administrative identity authority for the deterministic China selector.
//!
//! This module owns identity and hierarchy only. Geometry providers, the current
//! production selector, and global free-text discovery remain separate domains.

use chrono::{DateTime, NaiveDate};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::Path;
use thiserror::Error;

pub const CHINA_ADMIN_AUTHORITY_SCHEMA_VERSION: &str = "1.0.0";
pub const CHINA_ADMIN_CONTENT_HASH_SCOPE: &str = "canonical_json_without_content_sha256_v1";
pub const CHINA_ADMIN_COUNTRY_ADCODE: &str = "100000";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChinaAdminLevel {
    Country,
    Province,
    Prefecture,
    County,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChinaAdminUnitType {
    Country,
    Province,
    AutonomousRegion,
    Municipality,
    SpecialAdministrativeRegion,
    PrefectureCity,
    AutonomousPrefecture,
    League,
    Region,
    UrbanDistrict,
    County,
    AutonomousCounty,
    CountyCity,
    Banner,
    AutonomousBanner,
    ForestryDistrict,
    SpecialDistrict,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChinaAdminSpecialTag {
    Municipality,
    AutonomousPrefecture,
    League,
    Region,
    ProvinceDirectCounty,
    MunicipalityDirectCounty,
    UrbanDistrict,
    County,
    AutonomousCounty,
    CountyCity,
    Banner,
    AutonomousBanner,
    ForestryDistrict,
    SpecialDistrict,
    SpecialAdministrativeRegion,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChinaAdminAuthorityStatus {
    NonProductionMigrationReference,
    ProductionCandidate,
    ProductionApproved,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChinaAdminSourceKind {
    RepositoryLegacyIndex,
    OfficialSnapshot,
    LicensedSnapshot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChinaAdminLicenseStatus {
    Unresolved,
    ReviewRequired,
    Restricted,
    Approved,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChinaAdminSourceManifest {
    pub source_kind: ChinaAdminSourceKind,
    pub source_id: String,
    pub source_path: Option<String>,
    pub source_commit: Option<String>,
    pub source_sha256: String,
    pub source_title: Option<String>,
    pub publishing_authority: Option<String>,
    pub reference_date: Option<String>,
    pub retrieval_date: Option<String>,
    pub source_location: Option<String>,
    pub raw_sha256: Option<String>,
    pub normalization_version: Option<String>,
    pub administrative_effective_date_verified: bool,
    pub license_status: ChinaAdminLicenseStatus,
    pub production_distribution_approved: bool,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChinaAdminUnit {
    pub adcode: String,
    pub name_zh: String,
    pub aliases: Vec<String>,
    pub administrative_level: ChinaAdminLevel,
    pub unit_type: ChinaAdminUnitType,
    pub parent_adcode: Option<String>,
    pub selectable: bool,
    pub supported: bool,
    pub special_tags: Vec<ChinaAdminSpecialTag>,
    pub sort_order: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChinaAdminAuthorityArtifact {
    schema_version: String,
    authority_version: String,
    authority_status: ChinaAdminAuthorityStatus,
    effective_date: Option<String>,
    generated_at: String,
    content_sha256: String,
    content_hash_scope: String,
    source_manifest: ChinaAdminSourceManifest,
    policy_version: String,
    units: Vec<ChinaAdminUnit>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ChinaAdminAuthorityErrorCode {
    Io,
    InvalidJson,
    UnsupportedSchema,
    InvalidMetadata,
    ContentHashMismatch,
    InvalidAdcode,
    DuplicateAdcode,
    MissingParent,
    CycleDetected,
    UnreachableNode,
    AliasConflict,
    SortOrderConflict,
    PolicyViolation,
    UnknownAdcode,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error("{stage}:{code:?}: {detail}")]
pub struct ChinaAdminAuthorityError {
    code: ChinaAdminAuthorityErrorCode,
    stage: &'static str,
    detail: String,
}

impl ChinaAdminAuthorityError {
    pub fn code(&self) -> ChinaAdminAuthorityErrorCode {
        self.code
    }

    pub fn stage(&self) -> &'static str {
        self.stage
    }

    pub fn detail(&self) -> &str {
        &self.detail
    }
}

fn authority_error(
    code: ChinaAdminAuthorityErrorCode,
    stage: &'static str,
    detail: impl Into<String>,
) -> ChinaAdminAuthorityError {
    ChinaAdminAuthorityError {
        code,
        stage,
        detail: detail.into(),
    }
}

#[derive(Clone, Debug)]
pub struct ChinaAdminAuthority {
    schema_version: String,
    authority_version: String,
    authority_status: ChinaAdminAuthorityStatus,
    effective_date: Option<String>,
    generated_at: String,
    content_sha256: String,
    source_manifest: ChinaAdminSourceManifest,
    policy_version: String,
    units: BTreeMap<String, ChinaAdminUnit>,
    roots: Vec<String>,
    children: BTreeMap<String, Vec<String>>,
}

impl ChinaAdminAuthority {
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, ChinaAdminAuthorityError> {
        let path = path.as_ref();
        let text = fs::read_to_string(path).map_err(|error| {
            authority_error(
                ChinaAdminAuthorityErrorCode::Io,
                "read",
                format!("{}: {error}", path.display()),
            )
        })?;
        Self::from_json(&text)
    }

    pub fn from_json(text: &str) -> Result<Self, ChinaAdminAuthorityError> {
        let value: Value = serde_json::from_str(text).map_err(|error| {
            authority_error(
                ChinaAdminAuthorityErrorCode::InvalidJson,
                "parse",
                error.to_string(),
            )
        })?;
        validate_content_hash(&value)?;
        let artifact: ChinaAdminAuthorityArtifact =
            serde_json::from_value(value).map_err(|error| {
                authority_error(
                    ChinaAdminAuthorityErrorCode::InvalidJson,
                    "schema",
                    error.to_string(),
                )
            })?;
        Self::build(artifact)
    }

    fn build(artifact: ChinaAdminAuthorityArtifact) -> Result<Self, ChinaAdminAuthorityError> {
        validate_metadata(&artifact)?;
        if artifact.units.is_empty() {
            return Err(authority_error(
                ChinaAdminAuthorityErrorCode::InvalidMetadata,
                "units",
                "authority contains no units",
            ));
        }

        let mut units = BTreeMap::new();
        let mut aliases = BTreeMap::<String, String>::new();
        for unit in artifact.units {
            validate_unit_shape(&unit)?;
            if units.contains_key(&unit.adcode) {
                return Err(authority_error(
                    ChinaAdminAuthorityErrorCode::DuplicateAdcode,
                    "identity",
                    format!("duplicate adcode {}", unit.adcode),
                ));
            }
            let mut local_aliases = BTreeSet::new();
            for alias in &unit.aliases {
                if !local_aliases.insert(alias) {
                    return Err(authority_error(
                        ChinaAdminAuthorityErrorCode::AliasConflict,
                        "identity",
                        format!("duplicate alias {alias} on {}", unit.adcode),
                    ));
                }
                if let Some(existing) = aliases.insert(alias.clone(), unit.adcode.clone()) {
                    return Err(authority_error(
                        ChinaAdminAuthorityErrorCode::AliasConflict,
                        "identity",
                        format!("alias {alias} maps to both {existing} and {}", unit.adcode),
                    ));
                }
            }
            units.insert(unit.adcode.clone(), unit);
        }

        for unit in units.values() {
            if let Some(parent) = &unit.parent_adcode {
                if !units.contains_key(parent) {
                    return Err(authority_error(
                        ChinaAdminAuthorityErrorCode::MissingParent,
                        "graph",
                        format!("{} references missing parent {parent}", unit.adcode),
                    ));
                }
            }
        }

        let mut sibling_orders = BTreeMap::<Option<String>, BTreeMap<u64, String>>::new();
        for unit in units.values() {
            let siblings = sibling_orders
                .entry(unit.parent_adcode.clone())
                .or_default();
            if let Some(existing) = siblings.insert(unit.sort_order, unit.adcode.clone()) {
                return Err(authority_error(
                    ChinaAdminAuthorityErrorCode::SortOrderConflict,
                    "ordering",
                    format!(
                        "siblings {existing} and {} share sort_order {}",
                        unit.adcode, unit.sort_order
                    ),
                ));
            }
        }

        detect_cycles(&units)?;
        let (roots, mut children) = build_graph_indexes(&units);
        validate_reachability(&units, &children)?;
        for unit in units.values() {
            validate_unit_policy(unit, &units)?;
        }
        sort_indexes(&units, &mut children);
        let mut roots = roots;
        roots.sort_by(|left, right| compare_unit_ids(&units, left, right));

        Ok(Self {
            schema_version: artifact.schema_version,
            authority_version: artifact.authority_version,
            authority_status: artifact.authority_status,
            effective_date: artifact.effective_date,
            generated_at: artifact.generated_at,
            content_sha256: artifact.content_sha256,
            source_manifest: artifact.source_manifest,
            policy_version: artifact.policy_version,
            units,
            roots,
            children,
        })
    }

    pub fn schema_version(&self) -> &str {
        &self.schema_version
    }

    pub fn authority_version(&self) -> &str {
        &self.authority_version
    }

    pub fn authority_status(&self) -> ChinaAdminAuthorityStatus {
        self.authority_status
    }

    pub fn effective_date(&self) -> Option<&str> {
        self.effective_date.as_deref()
    }

    pub fn generated_at(&self) -> &str {
        &self.generated_at
    }

    pub fn content_sha256(&self) -> &str {
        &self.content_sha256
    }

    pub fn policy_version(&self) -> &str {
        &self.policy_version
    }

    pub fn source_manifest(&self) -> &ChinaAdminSourceManifest {
        &self.source_manifest
    }

    pub fn production_admitted(&self) -> bool {
        self.authority_status == ChinaAdminAuthorityStatus::ProductionApproved
            && self.effective_date.is_some()
            && self.source_manifest.administrative_effective_date_verified
            && self.source_manifest.license_status == ChinaAdminLicenseStatus::Approved
            && self.source_manifest.production_distribution_approved
    }

    pub fn list_china_admin_roots(&self) -> Vec<&ChinaAdminUnit> {
        self.roots
            .iter()
            .filter_map(|adcode| self.units.get(adcode))
            .filter(|unit| is_active_selector_unit(unit))
            .collect()
    }

    pub fn list_china_admin_children(
        &self,
        parent_adcode: &str,
    ) -> Result<Vec<&ChinaAdminUnit>, ChinaAdminAuthorityError> {
        if !self.units.contains_key(parent_adcode) {
            return Err(authority_error(
                ChinaAdminAuthorityErrorCode::UnknownAdcode,
                "lookup",
                format!("unknown parent adcode {parent_adcode}"),
            ));
        }
        Ok(self
            .children
            .get(parent_adcode)
            .into_iter()
            .flatten()
            .filter_map(|adcode| self.units.get(adcode))
            .filter(|unit| is_active_selector_unit(unit))
            .collect())
    }

    pub fn get_china_admin_unit(&self, adcode: &str) -> Option<&ChinaAdminUnit> {
        self.units.get(adcode)
    }

    pub fn is_special_hierarchy_adcode(&self, adcode: &str) -> Option<bool> {
        self.units.get(adcode).map(is_special_hierarchy_unit)
    }

    pub fn verify_exhaustive(&self) -> ChinaAdminAuthorityVerificationReport {
        let mut evidence = Vec::new();
        let mut province = ChinaAdminVerificationCount::default();
        let mut prefecture_level = ChinaAdminVerificationCount::default();
        let mut county_level = ChinaAdminVerificationCount::default();
        let mut special_hierarchy = ChinaAdminVerificationCount::default();
        let mut special_type_counts = BTreeMap::<String, ChinaAdminVerificationCount>::new();
        let mut failed_units = Vec::new();
        let unsupported_units = self
            .units
            .values()
            .filter(|unit| !unit.supported)
            .map(|unit| unit.adcode.clone())
            .collect::<Vec<_>>();

        for unit in self.units.values().filter(|unit| unit.supported) {
            let exact_lookup = self
                .units
                .get(&unit.adcode)
                .is_some_and(|found| found.adcode == unit.adcode);
            let parent_child_membership = unit.parent_adcode.as_ref().is_none_or(|parent| {
                self.children
                    .get(parent)
                    .is_some_and(|children| children.contains(&unit.adcode))
            });
            let root_reachable = reaches_country_root(unit, &self.units);
            let policy_valid = validate_unit_policy(unit, &self.units).is_ok();
            let mut failures = Vec::new();
            if !exact_lookup {
                failures.push("exact_lookup".to_owned());
            }
            if !parent_child_membership {
                failures.push("parent_child_membership".to_owned());
            }
            if !root_reachable {
                failures.push("root_reachable".to_owned());
            }
            if !policy_valid {
                failures.push("policy_valid".to_owned());
            }
            let passed = failures.is_empty();
            if !passed {
                failed_units.push(unit.adcode.clone());
            }

            match unit.administrative_level {
                ChinaAdminLevel::Country => {}
                ChinaAdminLevel::Province => province.record(passed),
                ChinaAdminLevel::Prefecture => prefecture_level.record(passed),
                ChinaAdminLevel::County => county_level.record(passed),
            }
            if is_special_hierarchy_unit(unit) {
                special_hierarchy.record(passed);
                for class in special_classes(unit) {
                    special_type_counts.entry(class).or_default().record(passed);
                }
            }

            evidence.push(ChinaAdminUnitVerificationEvidence {
                adcode: unit.adcode.clone(),
                name_zh: unit.name_zh.clone(),
                administrative_level: unit.administrative_level,
                unit_type: unit.unit_type,
                parent_adcode: unit.parent_adcode.clone(),
                exact_lookup,
                parent_child_membership,
                root_reachable,
                policy_valid,
                passed,
                failures,
            });
        }

        let structural_gate_status = if failed_units.is_empty() {
            ChinaAdminAuthorityGateStatus::Pass
        } else {
            ChinaAdminAuthorityGateStatus::Fail
        };
        let structural_gate = match self.authority_status {
            ChinaAdminAuthorityStatus::NonProductionMigrationReference => {
                ChinaAdminStructuralGate::A1Reference {
                    a1_reference_structural_gate: structural_gate_status,
                }
            }
            ChinaAdminAuthorityStatus::ProductionCandidate => {
                ChinaAdminStructuralGate::A2ACandidate {
                    a2a_candidate_structural_gate: structural_gate_status,
                }
            }
            ChinaAdminAuthorityStatus::ProductionApproved => {
                ChinaAdminStructuralGate::ProductionIdentity {
                    production_identity_structural_gate: structural_gate_status,
                }
            }
        };
        ChinaAdminAuthorityVerificationReport {
            schema_version: self.schema_version.clone(),
            authority_version: self.authority_version.clone(),
            authority_status: self.authority_status,
            effective_date: self.effective_date.clone(),
            generated_at: self.generated_at.clone(),
            content_sha256: self.content_sha256.clone(),
            source_manifest: self.source_manifest.clone(),
            policy_version: self.policy_version.clone(),
            production_admitted: self.production_admitted(),
            supported_units: evidence.len(),
            province,
            prefecture_level,
            county_level,
            special_hierarchy,
            special_type_counts,
            structural_gate,
            // Identity structure alone never admits the final production gate. That
            // gate also requires exhaustive production geometry and bind evidence.
            china_production_administrative_gate: ChinaAdminAuthorityGateStatus::Fail,
            evidence,
            failed_units,
            unsupported_units,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ChinaAdminVerificationCount {
    pub passed: usize,
    pub total: usize,
}

impl ChinaAdminVerificationCount {
    fn record(&mut self, passed: bool) {
        self.total += 1;
        if passed {
            self.passed += 1;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ChinaAdminAuthorityGateStatus {
    Pass,
    Fail,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ChinaAdminStructuralGate {
    A1Reference {
        a1_reference_structural_gate: ChinaAdminAuthorityGateStatus,
    },
    A2ACandidate {
        a2a_candidate_structural_gate: ChinaAdminAuthorityGateStatus,
    },
    ProductionIdentity {
        production_identity_structural_gate: ChinaAdminAuthorityGateStatus,
    },
}

impl ChinaAdminStructuralGate {
    fn status(&self) -> ChinaAdminAuthorityGateStatus {
        match self {
            Self::A1Reference {
                a1_reference_structural_gate,
            } => *a1_reference_structural_gate,
            Self::A2ACandidate {
                a2a_candidate_structural_gate,
            } => *a2a_candidate_structural_gate,
            Self::ProductionIdentity {
                production_identity_structural_gate,
            } => *production_identity_structural_gate,
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::A1Reference { .. } => "A1 reference structural gate",
            Self::A2ACandidate { .. } => "A2-A candidate structural gate",
            Self::ProductionIdentity { .. } => "Production identity structural gate",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ChinaAdminUnitVerificationEvidence {
    pub adcode: String,
    pub name_zh: String,
    pub administrative_level: ChinaAdminLevel,
    pub unit_type: ChinaAdminUnitType,
    pub parent_adcode: Option<String>,
    pub exact_lookup: bool,
    pub parent_child_membership: bool,
    pub root_reachable: bool,
    pub policy_valid: bool,
    pub passed: bool,
    pub failures: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ChinaAdminAuthorityVerificationReport {
    pub schema_version: String,
    pub authority_version: String,
    pub authority_status: ChinaAdminAuthorityStatus,
    pub effective_date: Option<String>,
    pub generated_at: String,
    pub content_sha256: String,
    pub source_manifest: ChinaAdminSourceManifest,
    pub policy_version: String,
    pub production_admitted: bool,
    pub supported_units: usize,
    pub province: ChinaAdminVerificationCount,
    pub prefecture_level: ChinaAdminVerificationCount,
    pub county_level: ChinaAdminVerificationCount,
    pub special_hierarchy: ChinaAdminVerificationCount,
    pub special_type_counts: BTreeMap<String, ChinaAdminVerificationCount>,
    #[serde(flatten)]
    pub structural_gate: ChinaAdminStructuralGate,
    pub china_production_administrative_gate: ChinaAdminAuthorityGateStatus,
    pub evidence: Vec<ChinaAdminUnitVerificationEvidence>,
    pub failed_units: Vec<String>,
    pub unsupported_units: Vec<String>,
}

impl ChinaAdminAuthorityVerificationReport {
    pub fn structural_gate_status(&self) -> ChinaAdminAuthorityGateStatus {
        self.structural_gate.status()
    }

    pub fn structural_gate_label(&self) -> &'static str {
        self.structural_gate.label()
    }

    pub fn evidence_jsonl(&self) -> Result<String, serde_json::Error> {
        let mut output = String::new();
        for row in &self.evidence {
            output.push_str(&serde_json::to_string(row)?);
            output.push('\n');
        }
        Ok(output)
    }

    pub fn summary_lines(&self) -> Vec<String> {
        vec![
            format!("Province: {}/{}", self.province.passed, self.province.total),
            format!(
                "Prefecture-level: {}/{}",
                self.prefecture_level.passed, self.prefecture_level.total
            ),
            format!(
                "County-level: {}/{}",
                self.county_level.passed, self.county_level.total
            ),
            format!(
                "Special hierarchy: {}/{}",
                self.special_hierarchy.passed, self.special_hierarchy.total
            ),
            format!(
                "{}: {}",
                self.structural_gate_label(),
                match self.structural_gate_status() {
                    ChinaAdminAuthorityGateStatus::Pass => "PASS",
                    ChinaAdminAuthorityGateStatus::Fail => "FAIL",
                }
            ),
            format!(
                "China production administrative gate: {}",
                match self.china_production_administrative_gate {
                    ChinaAdminAuthorityGateStatus::Pass => "PASS",
                    ChinaAdminAuthorityGateStatus::Fail => "FAIL",
                }
            ),
        ]
    }
}

fn validate_content_hash(value: &Value) -> Result<(), ChinaAdminAuthorityError> {
    let object = value.as_object().ok_or_else(|| {
        authority_error(
            ChinaAdminAuthorityErrorCode::InvalidJson,
            "schema",
            "authority root must be an object",
        )
    })?;
    let expected = object
        .get("content_sha256")
        .and_then(Value::as_str)
        .filter(|value| is_lower_hex(value, 64))
        .ok_or_else(|| {
            authority_error(
                ChinaAdminAuthorityErrorCode::InvalidMetadata,
                "hash",
                "content_sha256 must be 64 lowercase hexadecimal characters",
            )
        })?;
    let mut content = value.clone();
    content
        .as_object_mut()
        .expect("object checked above")
        .remove("content_sha256");
    let actual = format!("{:x}", Sha256::digest(canonical_json(&content).as_bytes()));
    if actual != expected {
        return Err(authority_error(
            ChinaAdminAuthorityErrorCode::ContentHashMismatch,
            "hash",
            format!("expected {expected}, calculated {actual}"),
        ));
    }
    Ok(())
}

fn canonical_json(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => serde_json::to_string(value).expect("string serialization"),
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",")
        ),
        Value::Object(values) => {
            let mut keys = values.keys().collect::<Vec<_>>();
            keys.sort();
            format!(
                "{{{}}}",
                keys.into_iter()
                    .map(|key| format!(
                        "{}:{}",
                        serde_json::to_string(key).expect("key serialization"),
                        canonical_json(&values[key])
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
    }
}

fn validate_metadata(
    artifact: &ChinaAdminAuthorityArtifact,
) -> Result<(), ChinaAdminAuthorityError> {
    if artifact.schema_version != CHINA_ADMIN_AUTHORITY_SCHEMA_VERSION {
        return Err(authority_error(
            ChinaAdminAuthorityErrorCode::UnsupportedSchema,
            "metadata",
            format!("unsupported schema_version {}", artifact.schema_version),
        ));
    }
    if artifact.content_hash_scope != CHINA_ADMIN_CONTENT_HASH_SCOPE {
        return Err(authority_error(
            ChinaAdminAuthorityErrorCode::InvalidMetadata,
            "metadata",
            "unsupported content_hash_scope",
        ));
    }
    validate_identifier("authority_version", &artifact.authority_version)?;
    validate_identifier("policy_version", &artifact.policy_version)?;
    DateTime::parse_from_rfc3339(&artifact.generated_at).map_err(|_| {
        authority_error(
            ChinaAdminAuthorityErrorCode::InvalidMetadata,
            "metadata",
            "generated_at must be RFC3339",
        )
    })?;
    if let Some(date) = &artifact.effective_date {
        NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(|_| {
            authority_error(
                ChinaAdminAuthorityErrorCode::InvalidMetadata,
                "metadata",
                "effective_date must be YYYY-MM-DD",
            )
        })?;
    }
    if artifact.authority_status != ChinaAdminAuthorityStatus::NonProductionMigrationReference
        && artifact.effective_date.is_none()
    {
        return Err(authority_error(
            ChinaAdminAuthorityErrorCode::InvalidMetadata,
            "metadata",
            "production candidate/approved authorities require effective_date",
        ));
    }
    if artifact.authority_status == ChinaAdminAuthorityStatus::ProductionApproved {
        let manifest = &artifact.source_manifest;
        if !manifest.administrative_effective_date_verified
            || manifest.license_status != ChinaAdminLicenseStatus::Approved
            || !manifest.production_distribution_approved
        {
            return Err(authority_error(
                ChinaAdminAuthorityErrorCode::InvalidMetadata,
                "metadata",
                "production_approved requires verified effective date, approved license, and distribution approval",
            ));
        }
    }
    validate_source_manifest(&artifact.source_manifest)?;
    if artifact.source_manifest.source_kind == ChinaAdminSourceKind::OfficialSnapshot
        && artifact.effective_date.as_deref() != artifact.source_manifest.reference_date.as_deref()
    {
        return Err(authority_error(
            ChinaAdminAuthorityErrorCode::InvalidMetadata,
            "metadata",
            "official snapshot effective_date must equal source reference_date",
        ));
    }
    Ok(())
}

fn validate_source_manifest(
    manifest: &ChinaAdminSourceManifest,
) -> Result<(), ChinaAdminAuthorityError> {
    validate_bounded_text("source_id", &manifest.source_id, 256)?;
    if !is_lower_hex(&manifest.source_sha256, 64) {
        return Err(authority_error(
            ChinaAdminAuthorityErrorCode::InvalidMetadata,
            "source_manifest",
            "source_sha256 must be 64 lowercase hexadecimal characters",
        ));
    }
    for (field, value, max_chars) in [
        ("source_title", manifest.source_title.as_deref(), 512),
        (
            "publishing_authority",
            manifest.publishing_authority.as_deref(),
            256,
        ),
        ("source_location", manifest.source_location.as_deref(), 1024),
    ] {
        if let Some(value) = value {
            validate_bounded_text(field, value, max_chars)?;
        }
    }
    for (field, value) in [
        ("reference_date", manifest.reference_date.as_deref()),
        ("retrieval_date", manifest.retrieval_date.as_deref()),
    ] {
        if let Some(value) = value {
            NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| {
                authority_error(
                    ChinaAdminAuthorityErrorCode::InvalidMetadata,
                    "source_manifest",
                    format!("{field} must be YYYY-MM-DD"),
                )
            })?;
        }
    }
    if let Some(raw_sha256) = &manifest.raw_sha256 {
        if !is_lower_hex(raw_sha256, 64) {
            return Err(authority_error(
                ChinaAdminAuthorityErrorCode::InvalidMetadata,
                "source_manifest",
                "raw_sha256 must be 64 lowercase hexadecimal characters",
            ));
        }
    }
    if let Some(normalization_version) = &manifest.normalization_version {
        validate_identifier("normalization_version", normalization_version)?;
    }
    if let Some(commit) = &manifest.source_commit {
        if !is_lower_hex(commit, 40) {
            return Err(authority_error(
                ChinaAdminAuthorityErrorCode::InvalidMetadata,
                "source_manifest",
                "source_commit must be 40 lowercase hexadecimal characters",
            ));
        }
    }
    if manifest
        .source_path
        .as_ref()
        .is_some_and(|path| path.len() > 512)
    {
        return Err(authority_error(
            ChinaAdminAuthorityErrorCode::InvalidMetadata,
            "source_manifest",
            "source_path is too long",
        ));
    }
    if manifest.notes.len() > 32 {
        return Err(authority_error(
            ChinaAdminAuthorityErrorCode::InvalidMetadata,
            "source_manifest",
            "too many source notes",
        ));
    }
    for note in &manifest.notes {
        validate_bounded_text("source note", note, 1024)?;
    }
    if manifest.source_kind == ChinaAdminSourceKind::OfficialSnapshot {
        let required = [
            ("source_title", manifest.source_title.as_deref()),
            (
                "publishing_authority",
                manifest.publishing_authority.as_deref(),
            ),
            ("reference_date", manifest.reference_date.as_deref()),
            ("retrieval_date", manifest.retrieval_date.as_deref()),
            ("source_location", manifest.source_location.as_deref()),
            ("raw_sha256", manifest.raw_sha256.as_deref()),
            (
                "normalization_version",
                manifest.normalization_version.as_deref(),
            ),
        ];
        if let Some((field, _)) = required.into_iter().find(|(_, value)| value.is_none()) {
            return Err(authority_error(
                ChinaAdminAuthorityErrorCode::InvalidMetadata,
                "source_manifest",
                format!("official_snapshot requires {field}"),
            ));
        }
        if manifest.source_path.is_none() {
            return Err(authority_error(
                ChinaAdminAuthorityErrorCode::InvalidMetadata,
                "source_manifest",
                "official_snapshot requires a preserved source_path",
            ));
        }
        if !manifest
            .source_location
            .as_deref()
            .is_some_and(|value| value.starts_with("https://"))
        {
            return Err(authority_error(
                ChinaAdminAuthorityErrorCode::InvalidMetadata,
                "source_manifest",
                "official_snapshot source_location must use HTTPS",
            ));
        }
        if manifest.raw_sha256.as_deref() != Some(manifest.source_sha256.as_str()) {
            return Err(authority_error(
                ChinaAdminAuthorityErrorCode::InvalidMetadata,
                "source_manifest",
                "official_snapshot raw_sha256 must equal source_sha256",
            ));
        }
        let reference_date = NaiveDate::parse_from_str(
            manifest.reference_date.as_deref().expect("required above"),
            "%Y-%m-%d",
        )
        .expect("validated above");
        let retrieval_date = NaiveDate::parse_from_str(
            manifest.retrieval_date.as_deref().expect("required above"),
            "%Y-%m-%d",
        )
        .expect("validated above");
        if retrieval_date < reference_date {
            return Err(authority_error(
                ChinaAdminAuthorityErrorCode::InvalidMetadata,
                "source_manifest",
                "retrieval_date cannot precede reference_date",
            ));
        }
    }
    Ok(())
}

fn validate_identifier(field: &str, value: &str) -> Result<(), ChinaAdminAuthorityError> {
    if value.is_empty()
        || value.len() > 128
        || !value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric() || (index > 0 && b"._-".contains(&byte))
        })
    {
        return Err(authority_error(
            ChinaAdminAuthorityErrorCode::InvalidMetadata,
            "metadata",
            format!("invalid {field}"),
        ));
    }
    Ok(())
}

fn validate_bounded_text(
    field: &str,
    value: &str,
    max_chars: usize,
) -> Result<(), ChinaAdminAuthorityError> {
    if value.trim() != value || value.is_empty() || value.chars().count() > max_chars {
        return Err(authority_error(
            ChinaAdminAuthorityErrorCode::InvalidMetadata,
            "text",
            format!("invalid {field}"),
        ));
    }
    Ok(())
}

fn validate_unit_shape(unit: &ChinaAdminUnit) -> Result<(), ChinaAdminAuthorityError> {
    if !is_adcode(&unit.adcode) {
        return Err(authority_error(
            ChinaAdminAuthorityErrorCode::InvalidAdcode,
            "identity",
            format!("invalid adcode {}", unit.adcode),
        ));
    }
    validate_bounded_text("name_zh", &unit.name_zh, 128)?;
    if unit.aliases.len() > 32 || unit.special_tags.len() > 16 {
        return Err(authority_error(
            ChinaAdminAuthorityErrorCode::InvalidMetadata,
            "unit",
            format!("unit {} exceeds bounded list limits", unit.adcode),
        ));
    }
    for alias in &unit.aliases {
        validate_bounded_text("alias", alias, 128)?;
    }
    if unit
        .parent_adcode
        .as_ref()
        .is_some_and(|parent| !is_adcode(parent))
    {
        return Err(authority_error(
            ChinaAdminAuthorityErrorCode::InvalidAdcode,
            "identity",
            format!("invalid parent adcode on {}", unit.adcode),
        ));
    }
    if unit.selectable && !unit.supported {
        return Err(authority_error(
            ChinaAdminAuthorityErrorCode::PolicyViolation,
            "policy",
            format!("selectable unit {} is not supported", unit.adcode),
        ));
    }
    let unique_tags = unit.special_tags.iter().copied().collect::<BTreeSet<_>>();
    if unique_tags.len() != unit.special_tags.len() {
        return Err(authority_error(
            ChinaAdminAuthorityErrorCode::PolicyViolation,
            "policy",
            format!("duplicate special tag on {}", unit.adcode),
        ));
    }
    Ok(())
}

fn detect_cycles(units: &BTreeMap<String, ChinaAdminUnit>) -> Result<(), ChinaAdminAuthorityError> {
    fn visit(
        adcode: &str,
        units: &BTreeMap<String, ChinaAdminUnit>,
        states: &mut BTreeMap<String, u8>,
    ) -> Result<(), ChinaAdminAuthorityError> {
        match states.get(adcode).copied().unwrap_or(0) {
            1 => {
                return Err(authority_error(
                    ChinaAdminAuthorityErrorCode::CycleDetected,
                    "graph",
                    format!("cycle includes {adcode}"),
                ))
            }
            2 => return Ok(()),
            _ => {}
        }
        states.insert(adcode.to_owned(), 1);
        if let Some(parent) = units[adcode].parent_adcode.as_deref() {
            visit(parent, units, states)?;
        }
        states.insert(adcode.to_owned(), 2);
        Ok(())
    }

    let mut states = BTreeMap::new();
    for adcode in units.keys() {
        visit(adcode, units, &mut states)?;
    }
    Ok(())
}

fn build_graph_indexes(
    units: &BTreeMap<String, ChinaAdminUnit>,
) -> (Vec<String>, BTreeMap<String, Vec<String>>) {
    let mut roots = Vec::new();
    let mut children = BTreeMap::<String, Vec<String>>::new();
    for unit in units.values() {
        if let Some(parent) = &unit.parent_adcode {
            children
                .entry(parent.clone())
                .or_default()
                .push(unit.adcode.clone());
        } else {
            roots.push(unit.adcode.clone());
        }
    }
    (roots, children)
}

fn sort_indexes(
    units: &BTreeMap<String, ChinaAdminUnit>,
    children: &mut BTreeMap<String, Vec<String>>,
) {
    for values in children.values_mut() {
        values.sort_by(|left, right| compare_unit_ids(units, left, right));
    }
}

fn compare_unit_ids(
    units: &BTreeMap<String, ChinaAdminUnit>,
    left: &str,
    right: &str,
) -> std::cmp::Ordering {
    units[left]
        .sort_order
        .cmp(&units[right].sort_order)
        .then_with(|| left.cmp(right))
}

fn validate_reachability(
    units: &BTreeMap<String, ChinaAdminUnit>,
    children: &BTreeMap<String, Vec<String>>,
) -> Result<(), ChinaAdminAuthorityError> {
    if !units.contains_key(CHINA_ADMIN_COUNTRY_ADCODE) {
        return Err(authority_error(
            ChinaAdminAuthorityErrorCode::UnreachableNode,
            "graph",
            "country root 100000 is missing",
        ));
    }
    let mut reached = BTreeSet::new();
    let mut queue = VecDeque::from([CHINA_ADMIN_COUNTRY_ADCODE.to_owned()]);
    while let Some(adcode) = queue.pop_front() {
        if !reached.insert(adcode.clone()) {
            continue;
        }
        if let Some(unit_children) = children.get(&adcode) {
            queue.extend(unit_children.iter().cloned());
        }
    }
    if reached.len() != units.len() {
        let missing = units
            .keys()
            .find(|adcode| !reached.contains(*adcode))
            .expect("length mismatch guarantees one missing unit");
        return Err(authority_error(
            ChinaAdminAuthorityErrorCode::UnreachableNode,
            "graph",
            format!("{missing} is not reachable from 100000"),
        ));
    }
    Ok(())
}

fn validate_unit_policy(
    unit: &ChinaAdminUnit,
    units: &BTreeMap<String, ChinaAdminUnit>,
) -> Result<(), ChinaAdminAuthorityError> {
    match unit.administrative_level {
        ChinaAdminLevel::Country => {
            if unit.adcode != CHINA_ADMIN_COUNTRY_ADCODE
                || unit.unit_type != ChinaAdminUnitType::Country
                || unit.parent_adcode.is_some()
            {
                return policy_error(unit, "country must be the root identity 100000");
            }
        }
        ChinaAdminLevel::Province => {
            if !matches!(
                unit.unit_type,
                ChinaAdminUnitType::Province
                    | ChinaAdminUnitType::AutonomousRegion
                    | ChinaAdminUnitType::Municipality
                    | ChinaAdminUnitType::SpecialAdministrativeRegion
            ) || unit.parent_adcode.as_deref() != Some(CHINA_ADMIN_COUNTRY_ADCODE)
            {
                return policy_error(unit, "province-level unit must be a child of 100000");
            }
        }
        ChinaAdminLevel::Prefecture => {
            if !matches!(
                unit.unit_type,
                ChinaAdminUnitType::PrefectureCity
                    | ChinaAdminUnitType::AutonomousPrefecture
                    | ChinaAdminUnitType::League
                    | ChinaAdminUnitType::Region
            ) {
                return policy_error(unit, "invalid prefecture unit_type");
            }
            let parent = parent_unit(unit, units)?;
            if parent.administrative_level != ChinaAdminLevel::Province
                || matches!(
                    parent.unit_type,
                    ChinaAdminUnitType::Municipality
                        | ChinaAdminUnitType::SpecialAdministrativeRegion
                )
            {
                return policy_error(unit, "prefecture parent is not an eligible province");
            }
        }
        ChinaAdminLevel::County => {
            if !matches!(
                unit.unit_type,
                ChinaAdminUnitType::UrbanDistrict
                    | ChinaAdminUnitType::County
                    | ChinaAdminUnitType::AutonomousCounty
                    | ChinaAdminUnitType::CountyCity
                    | ChinaAdminUnitType::Banner
                    | ChinaAdminUnitType::AutonomousBanner
                    | ChinaAdminUnitType::ForestryDistrict
                    | ChinaAdminUnitType::SpecialDistrict
            ) {
                return policy_error(unit, "invalid county unit_type");
            }
            let parent = parent_unit(unit, units)?;
            match parent.administrative_level {
                ChinaAdminLevel::Prefecture => {}
                ChinaAdminLevel::Province
                    if parent.unit_type == ChinaAdminUnitType::Municipality => {}
                ChinaAdminLevel::Province
                    if parent.unit_type != ChinaAdminUnitType::SpecialAdministrativeRegion => {}
                _ => return policy_error(unit, "county parent level/type is invalid"),
            }
        }
    }

    let actual_tags = unit.special_tags.iter().copied().collect::<BTreeSet<_>>();
    let expected_tags = expected_special_tags(unit, units)?;
    if actual_tags != expected_tags {
        return policy_error(
            unit,
            "special_tags do not exactly match unit type and parent relation",
        );
    }
    Ok(())
}

fn expected_special_tags(
    unit: &ChinaAdminUnit,
    units: &BTreeMap<String, ChinaAdminUnit>,
) -> Result<BTreeSet<ChinaAdminSpecialTag>, ChinaAdminAuthorityError> {
    let mut tags = BTreeSet::new();
    match unit.unit_type {
        ChinaAdminUnitType::Municipality => {
            tags.insert(ChinaAdminSpecialTag::Municipality);
        }
        ChinaAdminUnitType::SpecialAdministrativeRegion => {
            tags.insert(ChinaAdminSpecialTag::SpecialAdministrativeRegion);
        }
        ChinaAdminUnitType::AutonomousPrefecture => {
            tags.insert(ChinaAdminSpecialTag::AutonomousPrefecture);
        }
        ChinaAdminUnitType::League => {
            tags.insert(ChinaAdminSpecialTag::League);
        }
        ChinaAdminUnitType::Region => {
            tags.insert(ChinaAdminSpecialTag::Region);
        }
        ChinaAdminUnitType::UrbanDistrict => {
            tags.insert(ChinaAdminSpecialTag::UrbanDistrict);
        }
        ChinaAdminUnitType::County => {
            tags.insert(ChinaAdminSpecialTag::County);
        }
        ChinaAdminUnitType::AutonomousCounty => {
            tags.insert(ChinaAdminSpecialTag::AutonomousCounty);
        }
        ChinaAdminUnitType::CountyCity => {
            tags.insert(ChinaAdminSpecialTag::CountyCity);
        }
        ChinaAdminUnitType::Banner => {
            tags.insert(ChinaAdminSpecialTag::Banner);
        }
        ChinaAdminUnitType::AutonomousBanner => {
            tags.insert(ChinaAdminSpecialTag::AutonomousBanner);
        }
        ChinaAdminUnitType::ForestryDistrict => {
            tags.insert(ChinaAdminSpecialTag::ForestryDistrict);
        }
        ChinaAdminUnitType::SpecialDistrict => {
            tags.insert(ChinaAdminSpecialTag::SpecialDistrict);
        }
        _ => {}
    }

    if unit.administrative_level == ChinaAdminLevel::County {
        let parent = parent_unit(unit, units)?;
        if parent.administrative_level == ChinaAdminLevel::Province {
            if parent.unit_type == ChinaAdminUnitType::Municipality {
                tags.insert(ChinaAdminSpecialTag::MunicipalityDirectCounty);
            } else if parent.unit_type != ChinaAdminUnitType::SpecialAdministrativeRegion {
                tags.insert(ChinaAdminSpecialTag::ProvinceDirectCounty);
            }
        }
    }

    Ok(tags)
}

fn parent_unit<'a>(
    unit: &ChinaAdminUnit,
    units: &'a BTreeMap<String, ChinaAdminUnit>,
) -> Result<&'a ChinaAdminUnit, ChinaAdminAuthorityError> {
    unit.parent_adcode
        .as_ref()
        .and_then(|adcode| units.get(adcode))
        .ok_or_else(|| {
            authority_error(
                ChinaAdminAuthorityErrorCode::MissingParent,
                "policy",
                format!("{} has no usable parent", unit.adcode),
            )
        })
}

fn policy_error(unit: &ChinaAdminUnit, detail: &str) -> Result<(), ChinaAdminAuthorityError> {
    Err(authority_error(
        ChinaAdminAuthorityErrorCode::PolicyViolation,
        "policy",
        format!("{}: {detail}", unit.adcode),
    ))
}

fn reaches_country_root(unit: &ChinaAdminUnit, units: &BTreeMap<String, ChinaAdminUnit>) -> bool {
    let mut current = Some(unit);
    let mut seen = BTreeSet::new();
    while let Some(value) = current {
        if !seen.insert(value.adcode.as_str()) {
            return false;
        }
        if value.adcode == CHINA_ADMIN_COUNTRY_ADCODE {
            return value.parent_adcode.is_none();
        }
        current = value
            .parent_adcode
            .as_ref()
            .and_then(|parent| units.get(parent));
    }
    false
}

fn is_active_selector_unit(unit: &ChinaAdminUnit) -> bool {
    unit.supported && unit.selectable
}

fn is_special_hierarchy_unit(unit: &ChinaAdminUnit) -> bool {
    !special_classes(unit).is_empty()
}

fn special_classes(unit: &ChinaAdminUnit) -> Vec<String> {
    let mut classes = BTreeSet::new();
    match unit.unit_type {
        ChinaAdminUnitType::Municipality => {
            classes.insert("municipality");
        }
        ChinaAdminUnitType::AutonomousPrefecture => {
            classes.insert("autonomous_prefecture");
        }
        ChinaAdminUnitType::League => {
            classes.insert("league");
        }
        ChinaAdminUnitType::Region => {
            classes.insert("region");
        }
        ChinaAdminUnitType::UrbanDistrict => {
            classes.insert("urban_district");
        }
        ChinaAdminUnitType::County => {
            classes.insert("county");
        }
        ChinaAdminUnitType::AutonomousCounty => {
            classes.insert("autonomous_county");
        }
        ChinaAdminUnitType::CountyCity => {
            classes.insert("county_city");
        }
        ChinaAdminUnitType::Banner => {
            classes.insert("banner");
        }
        ChinaAdminUnitType::AutonomousBanner => {
            classes.insert("autonomous_banner");
        }
        ChinaAdminUnitType::ForestryDistrict => {
            classes.insert("forestry_district");
        }
        ChinaAdminUnitType::SpecialDistrict => {
            classes.insert("special_district");
        }
        _ => {}
    }
    if unit
        .special_tags
        .contains(&ChinaAdminSpecialTag::ProvinceDirectCounty)
    {
        classes.insert("province_direct_county");
    }
    classes.into_iter().map(str::to_owned).collect()
}

fn is_adcode(value: &str) -> bool {
    value.len() == 6 && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
