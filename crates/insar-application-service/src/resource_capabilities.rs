use std::collections::{BTreeMap, HashSet};

use insar_core::{
    project_effective_capability, BuildCapabilityState, CapabilityDeclaration, CapabilityKind,
    CapabilityPolicyState, CredentialReadinessState, DeliveryCapabilityDescriptor,
    EffectiveCapabilityInputs, EffectiveCapabilityProjection, EvidenceAdmissionState,
    ProviderRouteCandidate, ProviderRouteDescriptor, ResourceDependency, ResourceDescriptor,
    ResourceRegistry, RuntimeReadinessState,
};
use serde::{Deserialize, Serialize};

pub const NISAR_REMOTE_SUBSET_CAPABILITY_ID: &str = "capability.nisar-rslc.remote-subset.v1";

const BUILD_MANIFEST_JSON: &str =
    include_str!("../../../contracts/v3/capability-build-manifest.v1.json");
const PRODUCT_POLICY_JSON: &str =
    include_str!("../../../contracts/v3/capability-product-policy.v1.json");
const ADMISSION_EVIDENCE_JSON: &str =
    include_str!("../../../contracts/v3/capability-admission-evidence.v1.json");

const NISAR_SUBSET_BINDING: &str = "production-worker.nisar-remote-subset.v1";
pub(crate) const NISAR_SUBSET_ADMISSION_POLICY: &str = "admission.nisar.remote-subset.p0.3a.v1";
const ALOS2_ADMISSION_POLICY: &str = "admission.alos2-scansar-l1.1.full-archive.p0.4a.v1";
const SENTINEL2_FULL_ARCHIVE_ADMISSION_POLICY: &str =
    "admission.sentinel-2.l2a.full-archive.p0.4a-s2.v1";
const SENTINEL2_FULL_ARCHIVE_EVIDENCE_ID: &str =
    "evidence.sentinel-2.l2a.complete-archive.p0.4a-s2.badong.v1";
const SENTINEL2_FULL_ARCHIVE_CAPABILITY_ID: &str = "capability.sentinel-2.l2a.full-archive.v1";
const SENTINEL2_FULL_ARCHIVE_BINDING: &str = "cdse.sentinel-2.l2a.archive-resolver-policy.v1";
const SENTINEL2_FULL_ARCHIVE_EVIDENCE_SCHEMA: &str =
    "sentinel-2-l2a-complete-archive-p04a-s2-live-acceptance-redacted-v1";
const SENTINEL2_FULL_ARCHIVE_GATE: &str = "P0.4A-S2";
const SENTINEL2_SOURCE_COMMIT: &str = "519621cf583735c8e67f575c4c3b902065d6dca3";
const SENTINEL2_PROVIDER_PRODUCT_ID: &str = "63e9bedb-deff-4acf-a8b6-57a4e927d928";
const SENTINEL2_PRODUCT_NAME: &str =
    "S2C_MSIL2A_20260703T031541_N0512_R118_T49RDQ_20260703T065712.SAFE";
const SENTINEL2_ARCHIVE_NAME: &str =
    "S2C_MSIL2A_20260703T031541_N0512_R118_T49RDQ_20260703T065712.SAFE.zip";
const SENTINEL2_ARCHIVE_SIZE_BYTES: u64 = 747_367_658;
const SENTINEL2_ARCHIVE_MD5: &str = "a6df28ea2e27125178120ce029b5ae0a";
const NISAR_C2_EVIDENCE_SCHEMA: &str = "nisar-remote-subset-m2d-c2-live-acceptance-redacted-v1";
const NISAR_C2_GATE: &str = "P0.3a-M2D-C2";
const NISAR_GEOMETRY_ALGORITHM_ID: &str =
    "NisarIsce3Geo2rdrOracleInputV1/NisarIsce3Geo2rdrOracleOutputV1";
const NISAR_GEOMETRY_ALGORITHM_VERSION: &str = "isce3-0.25.12";
const NISAR_SCIENCE_VALIDATION_CONTRACT_ID: &str =
    "NisarSubsetArtifactManifestV1.science_validation.v1";
const NISAR_WRITER_CONTRACT_ID: &str = "NisarSubsetArtifactManifestV1";
const NISAR_MAPPING_ALGORITHM_ID: &str = "sparse_grid_polygon_cells_v1";
const NISAR_MAPPING_ALGORITHM_VERSION: u32 = 1;
const NISAR_PRODUCT_SPECIFICATION_PROFILE_ID: &str = "nisar.p05023.product-specification.1.5.0";
const NISAR_ADMISSION_CONTRACT_ID: &str = "nisar.remote-subset.admission.v2";
const NISAR_TRANSPORT_INTEGRITY_CONTRACT_ID: &str = "NisarSubsetTransferManifestV1";
const NISAR_SEMANTIC_PLAN_FINGERPRINT: &str = "not_applicable_scene_generic";

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CapabilityBuildManifest {
    schema_id: String,
    schema_version: u32,
    manifest_id: String,
    build_profile_id: String,
    entries: Vec<CapabilityBuildEntry>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CapabilityBuildEntry {
    implementation_binding_id: String,
    required_features: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CapabilityProductPolicy {
    schema_id: String,
    schema_version: u32,
    policy_id: String,
    entries: Vec<CapabilityProductPolicyEntry>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CapabilityProductPolicyEntry {
    capability_id: String,
    implemented: bool,
    production_admitted: bool,
    default_enabled: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CapabilityAdmissionEvidenceAuthority {
    schema_id: String,
    schema_version: u32,
    authority_id: String,
    #[serde(deserialize_with = "deserialize_evidence_records")]
    records: Vec<CapabilityAdmissionEvidenceRecord>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "record_kind", deny_unknown_fields)]
enum CapabilityAdmissionEvidenceRecord {
    #[serde(rename = "nisar_remote_subset")]
    Nisar {
        evidence_id: String,
        capability_id: String,
        admission_policy_id: String,
        acceptance_status: String,
        evidence_schema: String,
        evidence_gate: String,
        implementation_binding_id: String,
        source_granule_id: String,
        provider_product_id: String,
        source_size_bytes: u64,
        source_md5: String,
        aoi_sha256: String,
        frequency: String,
        polarization: String,
        reviewed_hashes: Vec<CapabilityEvidenceHash>,
        reviewed_references: Vec<String>,
        applicability: NisarEvidenceApplicability,
        scientific_scope: NisarScientificScope,
        reviewed_at: String,
        superseded: bool,
        revoked: bool,
    },
    #[serde(rename = "alos2_complete_archive")]
    Alos2 {
        evidence_id: String,
        capability_id: String,
        admission_policy_id: String,
        acceptance_status: String,
        evidence_schema: String,
        evidence_gate: String,
        implementation_binding_id: String,
        provider_product_id: String,
        file_name: String,
        expected_size_bytes: u64,
        final_size_bytes: u64,
        source_md5: String,
        reviewed_hashes: Vec<CapabilityEvidenceHash>,
        reviewed_references: Vec<String>,
        lifecycle_facts: Alos2LifecycleFacts,
        applicability: Alos2EvidenceApplicability,
        scientific_scope: Alos2ScientificScope,
        reviewed_at: String,
        superseded: bool,
        revoked: bool,
    },
    #[serde(rename = "sentinel2_l2a_complete_archive")]
    Sentinel2 {
        evidence_id: String,
        capability_id: String,
        admission_policy_id: String,
        acceptance_status: String,
        evidence_schema: String,
        evidence_gate: String,
        implementation_binding_id: String,
        source_commit: String,
        provider_product_id: String,
        product_name: String,
        file_name: String,
        product_type: String,
        processing_level: String,
        acquisition_datetime: String,
        expected_size_bytes: u64,
        final_size_bytes: u64,
        checksum_algorithm: String,
        expected_checksum: String,
        observed_checksum: String,
        reviewed_hashes: Vec<CapabilityEvidenceHash>,
        reviewed_references: Vec<String>,
        lifecycle_facts: Sentinel2LifecycleFacts,
        applicability: Sentinel2EvidenceApplicability,
        scientific_scope: Sentinel2ScientificScope,
        reviewed_at: String,
        superseded: bool,
        revoked: bool,
    },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CapabilityEvidenceHash {
    kind: String,
    value: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NisarEvidenceApplicability {
    geometry_algorithm_id: String,
    geometry_algorithm_version: String,
    science_validation_contract_id: String,
    writer_contract_id: String,
    mapping_algorithm_id: String,
    mapping_algorithm_version: u32,
    product_specification_profile_id: String,
    admission_contract_id: String,
    transport_integrity_contract_id: String,
    semantic_plan_fingerprint: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NisarScientificScope {
    scene: String,
    aoi: String,
    frequency: String,
    polarization: String,
    claims: Vec<String>,
    limitations: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Alos2LifecycleFacts {
    store_runtime_status: String,
    store_completed_after_validation: bool,
    store_reopen_completed: bool,
    atomic_promotion: bool,
    part_absent_after_promotion: bool,
    credential_route_class: String,
    credential_redaction_contract: String,
    credential_material_written: bool,
    credential_values_logged: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Sentinel2LifecycleFacts {
    store_runtime_status: String,
    store_completed_after_validation: bool,
    store_reopen_completed: bool,
    atomic_promotion: bool,
    part_absent_after_promotion: bool,
    source_size_enforced: bool,
    checksum_verified: bool,
    credential_route_class: String,
    credential_redaction_contract: String,
    credential_material_written: bool,
    credential_values_logged: bool,
    transfer_started: bool,
    transfer_count: u64,
    quicklook_fetched: bool,
    archive_structure_validation: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Alos2EvidenceApplicability {
    resolver_policy_id: String,
    asset_schema_id: String,
    delivery_profile_id: String,
    validation_profile_id: String,
    implementation_binding_id: String,
    credential_redaction_contract_id: String,
    transport_integrity_contract_id: String,
    store_lifecycle_contract_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Sentinel2EvidenceApplicability {
    resolver_policy_id: String,
    asset_schema_id: String,
    delivery_profile_id: String,
    validation_profile_id: String,
    implementation_binding_id: String,
    collection_binding_id: String,
    credential_redaction_contract_id: String,
    transport_integrity_contract_id: String,
    store_lifecycle_contract_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Alos2ScientificScope {
    product: String,
    subset: bool,
    claims: Vec<String>,
    limitations: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Sentinel2ScientificScope {
    product: String,
    subset: bool,
    claims: Vec<String>,
    limitations: Vec<String>,
}

#[derive(Clone, Debug)]
struct EmbeddedCapabilityAuthorities {
    build: CapabilityBuildManifest,
    policy: CapabilityProductPolicy,
    admission: CapabilityAdmissionEvidenceAuthority,
}

fn embedded_capability_authorities() -> Result<EmbeddedCapabilityAuthorities, String> {
    let build = serde_json::from_str(BUILD_MANIFEST_JSON)
        .map_err(|error| format!("capability build manifest invalid: {error}"))?;
    let policy = serde_json::from_str(PRODUCT_POLICY_JSON)
        .map_err(|error| format!("capability product policy invalid: {error}"))?;
    let admission = serde_json::from_str(ADMISSION_EVIDENCE_JSON)
        .map_err(|error| format!("capability admission evidence invalid: {error}"))?;
    let authorities = EmbeddedCapabilityAuthorities {
        build,
        policy,
        admission,
    };
    validate_admission_record_identities(&authorities.admission.records)?;
    if authorities.build.schema_id != "capability-build-manifest.v1"
        || authorities.build.schema_version != 1
        || authorities.build.manifest_id.is_empty()
        || authorities.build.build_profile_id.is_empty()
        || authorities.policy.schema_id != "capability-product-policy.v1"
        || authorities.policy.schema_version != 1
        || authorities.policy.policy_id.is_empty()
        || authorities.admission.schema_id != "capability-admission-evidence.v1"
        || authorities.admission.schema_version != 1
        || authorities.admission.authority_id.is_empty()
    {
        return Err("capability authority identity/version invalid".to_owned());
    }
    Ok(authorities)
}

fn deserialize_evidence_records<'de, D>(
    deserializer: D,
) -> Result<Vec<CapabilityAdmissionEvidenceRecord>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let mut records = Vec::<serde_json::Value>::deserialize(deserializer)?;
    for record in &mut records {
        let object = record.as_object_mut().ok_or_else(|| {
            serde::de::Error::custom("admission evidence record must be an object")
        })?;
        object
            .entry("record_kind")
            .or_insert_with(|| serde_json::Value::String("nisar_remote_subset".to_owned()));
    }
    serde_json::from_value(serde_json::Value::Array(records)).map_err(serde::de::Error::custom)
}

fn validate_admission_record_identities(
    records: &[CapabilityAdmissionEvidenceRecord],
) -> Result<(), String> {
    let mut evidence_ids = HashSet::new();
    let mut admission_policy_ids = HashSet::new();
    for record in records {
        if !evidence_ids.insert(record.evidence_id()) {
            return Err(format!(
                "duplicate capability admission evidence ID: {}",
                record.evidence_id()
            ));
        }
        if !admission_policy_ids.insert(record.admission_policy_id()) {
            return Err(format!(
                "duplicate capability admission policy ID: {}",
                record.admission_policy_id()
            ));
        }
    }
    Ok(())
}

impl CapabilityAdmissionEvidenceRecord {
    fn evidence_id(&self) -> &str {
        match self {
            Self::Nisar { evidence_id, .. }
            | Self::Alos2 { evidence_id, .. }
            | Self::Sentinel2 { evidence_id, .. } => evidence_id,
        }
    }

    fn admission_policy_id(&self) -> &str {
        match self {
            Self::Nisar {
                admission_policy_id,
                ..
            }
            | Self::Alos2 {
                admission_policy_id,
                ..
            }
            | Self::Sentinel2 {
                admission_policy_id,
                ..
            } => admission_policy_id,
        }
    }

    fn accepted_and_applicable(&self) -> bool {
        match self {
            Self::Nisar {
                evidence_id,
                capability_id,
                admission_policy_id,
                acceptance_status,
                evidence_schema,
                evidence_gate,
                implementation_binding_id,
                source_granule_id,
                provider_product_id,
                source_size_bytes,
                source_md5,
                aoi_sha256,
                frequency,
                polarization,
                reviewed_hashes,
                reviewed_references,
                applicability,
                scientific_scope,
                reviewed_at,
                superseded,
                revoked,
            } => {
                !evidence_id.is_empty()
                    && acceptance_status == "accepted"
                    && !superseded
                    && !revoked
                    && capability_id == NISAR_REMOTE_SUBSET_CAPABILITY_ID
                    && admission_policy_id == NISAR_SUBSET_ADMISSION_POLICY
                    && evidence_schema == NISAR_C2_EVIDENCE_SCHEMA
                    && evidence_gate == NISAR_C2_GATE
                    && implementation_binding_id == NISAR_SUBSET_BINDING
                    && source_granule_id
                        == "NISAR_L1_PR_RSLC_025_017_A_147_7700_SHNA_A_20260709T100325_20260709T100342_P05023_N_P_J_001"
                    && provider_product_id == "G4244267663-ASF"
                    && *source_size_bytes == 5_280_628_736
                    && source_md5 == "3fb9b2412c4f78fa7185eb33c4585791"
                    && aoi_sha256
                        == "ad2107fbc106108c132f6a5efb1b12881ca92f33f732b3a16db2ab38826c320b"
                    && frequency == "A"
                    && polarization == "HH"
                    && reviewed_hashes.iter().any(|hash| {
                        hash.kind == "source_md5" && hash.value == *source_md5
                    })
                    && reviewed_hashes.iter().any(|hash| {
                        hash.kind == "aoi_sha256" && hash.value == *aoi_sha256
                    })
                    && portable_reviewed_references(reviewed_references)
                    && scientific_scope.scene == "single_scene"
                    && scientific_scope.aoi == "second_scene_aoi_v1"
                    && scientific_scope.frequency == *frequency
                    && scientific_scope.polarization == *polarization
                    && !scientific_scope.claims.is_empty()
                    && !scientific_scope.limitations.is_empty()
                    && !reviewed_at.trim().is_empty()
                    && applicability.matches_current()
            }
            Self::Alos2 {
                evidence_id,
                capability_id,
                admission_policy_id,
                acceptance_status,
                evidence_schema,
                evidence_gate,
                implementation_binding_id,
                provider_product_id,
                file_name,
                expected_size_bytes,
                final_size_bytes,
                source_md5,
                reviewed_hashes,
                reviewed_references,
                lifecycle_facts,
                applicability,
                scientific_scope,
                reviewed_at,
                superseded,
                revoked,
            } => {
                !evidence_id.is_empty()
                    && acceptance_status == "accepted"
                    && !superseded
                    && !revoked
                    && capability_id == "capability.alos2-scansar-l1.1.full-archive.v1"
                    && admission_policy_id == ALOS2_ADMISSION_POLICY
                    && evidence_schema
                        == "alos2-complete-archive-p04a-e1-live-acceptance-redacted-v1"
                    && evidence_gate == "P0.4A-E1"
                    && implementation_binding_id == "production-worker.complete.v1"
                    && provider_product_id == "G3463311647-ASF"
                    && file_name == "ALOS2478123150-230331-WBDR1.1__D.zip"
                    && *expected_size_bytes == 60_018_405_388
                    && expected_size_bytes == final_size_bytes
                    && source_md5 == "0f558597e19d964f24ca3a4bff11696c"
                    && reviewed_hashes.iter().any(|hash| {
                        hash.kind == "artifact_md5" && hash.value == *source_md5
                    })
                    && reviewed_hashes.iter().any(|hash| {
                        hash.kind == "evidence_json_sha256"
                            && hash.value
                                == "4b390c6de0142357357b09f2d7c045fa3ab1a5d86a94fef774b0a791ad0b0457"
                    })
                    && reviewed_hashes.iter().any(|hash| {
                        hash.kind == "freeze_json_sha256"
                            && hash.value
                                == "c6bd3897987cab0413578d5a86566c80eaa8d7e5f871379dd1ef5035dc1124cb"
                    })
                    && portable_reviewed_references(reviewed_references)
                    && lifecycle_facts.store_runtime_status == "completed"
                    && lifecycle_facts.store_completed_after_validation
                    && lifecycle_facts.store_reopen_completed
                    && lifecycle_facts.atomic_promotion
                    && lifecycle_facts.part_absent_after_promotion
                    && lifecycle_facts.credential_route_class
                        == "earthdata.auto.netrc-keyring-env"
                    && lifecycle_facts.credential_redaction_contract
                        == "P0.4A-credential-redaction-v1"
                    && !lifecycle_facts.credential_material_written
                    && !lifecycle_facts.credential_values_logged
                    && scientific_scope.product == "PALSAR-2 ScanSAR L1.1 Complete Product"
                    && !scientific_scope.subset
                    && !scientific_scope.claims.is_empty()
                    && !scientific_scope.limitations.is_empty()
                    && !reviewed_at.trim().is_empty()
                    && applicability.matches_current()
            },
            Self::Sentinel2 {
                evidence_id,
                capability_id,
                admission_policy_id,
                acceptance_status,
                evidence_schema,
                evidence_gate,
                implementation_binding_id,
                source_commit,
                provider_product_id,
                product_name,
                file_name,
                product_type,
                processing_level,
                acquisition_datetime,
                expected_size_bytes,
                final_size_bytes,
                checksum_algorithm,
                expected_checksum,
                observed_checksum,
                reviewed_hashes,
                reviewed_references,
                lifecycle_facts,
                applicability,
                scientific_scope,
                reviewed_at,
                superseded,
                revoked,
            } => {
                !evidence_id.is_empty()
                    && acceptance_status == "accepted"
                    && !superseded
                    && !revoked
                    && evidence_id == SENTINEL2_FULL_ARCHIVE_EVIDENCE_ID
                    && capability_id == SENTINEL2_FULL_ARCHIVE_CAPABILITY_ID
                    && admission_policy_id == SENTINEL2_FULL_ARCHIVE_ADMISSION_POLICY
                    && evidence_schema == SENTINEL2_FULL_ARCHIVE_EVIDENCE_SCHEMA
                    && evidence_gate == SENTINEL2_FULL_ARCHIVE_GATE
                    && implementation_binding_id == SENTINEL2_FULL_ARCHIVE_BINDING
                    && source_commit == SENTINEL2_SOURCE_COMMIT
                    && provider_product_id == SENTINEL2_PROVIDER_PRODUCT_ID
                    && product_name == SENTINEL2_PRODUCT_NAME
                    && file_name == SENTINEL2_ARCHIVE_NAME
                    && product_type == "S2MSI2A"
                    && processing_level == "L2A"
                    && acquisition_datetime == "2026-07-03T03:15:41.025Z"
                    && *expected_size_bytes == SENTINEL2_ARCHIVE_SIZE_BYTES
                    && expected_size_bytes == final_size_bytes
                    && checksum_algorithm == "MD5"
                    && expected_checksum == SENTINEL2_ARCHIVE_MD5
                    && observed_checksum == expected_checksum
                    && reviewed_hashes.iter().any(|hash| {
                        hash.kind == "artifact_md5" && hash.value == *expected_checksum
                    })
                    && reviewed_hashes.iter().any(|hash| {
                        hash.kind == "evidence_json_sha256"
                            && hash.value
                                == "ceef84105c2cbcb666d84eef63cff180fb88733d4ef43a7f81b1106863766275"
                    })
                    && reviewed_hashes.iter().any(|hash| {
                        hash.kind == "freeze_json_sha256"
                            && hash.value
                                == "e6391a57f0155e55932ba5bb799aa9bf4294be998879751cd78e953943718b0b"
                    })
                    && reviewed_hashes.iter().any(|hash| {
                        hash.kind == "stable_locator_sha256"
                            && hash.value
                                == "603c887adb48034fcc8c9778c0571e4aff2a0e6c6145ce9683026475adff2d8d"
                    })
                    && portable_reviewed_references(reviewed_references)
                    && lifecycle_facts.store_runtime_status == "completed"
                    && lifecycle_facts.store_completed_after_validation
                    && lifecycle_facts.store_reopen_completed
                    && lifecycle_facts.atomic_promotion
                    && lifecycle_facts.part_absent_after_promotion
                    && lifecycle_facts.source_size_enforced
                    && lifecycle_facts.checksum_verified
                    && lifecycle_facts.credential_route_class == "cdse.system-mode.session"
                    && lifecycle_facts.credential_redaction_contract
                        == "P0.4A-credential-redaction-v1"
                    && !lifecycle_facts.credential_material_written
                    && !lifecycle_facts.credential_values_logged
                    && lifecycle_facts.transfer_started
                    && lifecycle_facts.transfer_count == 1
                    && !lifecycle_facts.quicklook_fetched
                    && lifecycle_facts.archive_structure_validation
                        == "not_supported_by_current_validator"
                    && applicability.matches_current()
                    && scientific_scope.product == "Sentinel-2 Level-2A Complete Product"
                    && !scientific_scope.subset
                    && !scientific_scope.claims.is_empty()
                    && !scientific_scope.limitations.is_empty()
                    && !reviewed_at.trim().is_empty()
            }
        }
    }
}

fn portable_reviewed_references(references: &[String]) -> bool {
    !references.is_empty()
        && references.iter().all(|reference| {
            let path = reference.split("::").next().unwrap_or_default();
            let segments_are_safe = path
                .split('/')
                .all(|segment| !segment.is_empty() && segment != "." && segment != "..");
            !reference.contains("://")
                && !reference.contains(":/")
                && !reference.contains(":\\")
                && !reference.contains('\\')
                && !reference.starts_with('/')
                && segments_are_safe
                && (path.starts_with("crates/")
                    || path.starts_with("contracts/")
                    || path.starts_with("docs/"))
        })
}

impl NisarEvidenceApplicability {
    fn matches_current(&self) -> bool {
        self.geometry_algorithm_id == NISAR_GEOMETRY_ALGORITHM_ID
            && self.geometry_algorithm_version == NISAR_GEOMETRY_ALGORITHM_VERSION
            && self.science_validation_contract_id == NISAR_SCIENCE_VALIDATION_CONTRACT_ID
            && self.writer_contract_id == NISAR_WRITER_CONTRACT_ID
            && self.mapping_algorithm_id == NISAR_MAPPING_ALGORITHM_ID
            && self.mapping_algorithm_version == NISAR_MAPPING_ALGORITHM_VERSION
            && self.product_specification_profile_id == NISAR_PRODUCT_SPECIFICATION_PROFILE_ID
            && self.admission_contract_id == NISAR_ADMISSION_CONTRACT_ID
            && self.transport_integrity_contract_id == NISAR_TRANSPORT_INTEGRITY_CONTRACT_ID
            && self.semantic_plan_fingerprint == NISAR_SEMANTIC_PLAN_FINGERPRINT
    }
}

impl Alos2EvidenceApplicability {
    fn matches_current(&self) -> bool {
        self.resolver_policy_id == "asf.alos-2.earthdata-complete-zip.v1"
            && self.asset_schema_id == "assets.alos2-scansar-l1.1.archive.v1"
            && self.delivery_profile_id == "delivery.alos2-scansar-l1.1.archive.v1"
            && self.validation_profile_id == "validation.alos2-scansar-l1.1.archive.v1"
            && self.implementation_binding_id == "production-worker.complete.v1"
            && self.credential_redaction_contract_id == "P0.4A-credential-redaction-v1"
            && self.transport_integrity_contract_id
                == "expected-size-md5-provider-identity-atomic-promotion.v1"
            && self.store_lifecycle_contract_id == "store-completed-after-validation-reopen.v1"
    }
}

impl Sentinel2EvidenceApplicability {
    fn matches_current(&self) -> bool {
        self.resolver_policy_id == "cdse.sentinel-2.l2a.archive-resolver-policy.v1"
            && self.asset_schema_id == "assets.sentinel-2.l2a.v1"
            && self.delivery_profile_id == "delivery.sentinel-2.l2a.v1"
            && self.validation_profile_id == "validation.sentinel-2.l2a.v1"
            && self.implementation_binding_id == SENTINEL2_FULL_ARCHIVE_BINDING
            && self.collection_binding_id == "cdse.sentinel-2.l2a.collection.v1"
            && self.credential_redaction_contract_id == "P0.4A-credential-redaction-v1"
            && self.transport_integrity_contract_id
                == "expected-size-md5-provider-identity-atomic-promotion.v1"
            && self.store_lifecycle_contract_id == "store-completed-after-validation-reopen.v1"
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceDescriptorKind {
    ScienceDataset,
    Orbit,
    Dem,
    Atmospheric,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnownResourceSummary {
    pub resource_id: String,
    pub display_name: String,
    pub descriptor_kind: ResourceDescriptorKind,
    pub capability_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceDeliveryCapability {
    pub resource_id: String,
    pub capability: CapabilityDeclaration,
    pub delivery: DeliveryCapabilityDescriptor,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectiveCapabilityStatus {
    pub resource_id: String,
    pub capability: CapabilityDeclaration,
    pub inputs: EffectiveCapabilityInputs,
    pub projection: EffectiveCapabilityProjection,
}

/// Immutable observations supplied by the runtime owners named by Registry
/// identities. The maps contain no product semantics and perform no probing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CapabilityRuntimeObservations {
    build_by_binding: BTreeMap<String, BuildCapabilityState>,
    runtime_by_binding: BTreeMap<String, RuntimeReadinessState>,
    credential_by_route: BTreeMap<String, CredentialReadinessState>,
    admission_by_policy: BTreeMap<String, EvidenceAdmissionState>,
    product_policy_by_capability: BTreeMap<String, CapabilityPolicyState>,
}

impl CapabilityRuntimeObservations {
    pub fn with_build(
        mut self,
        implementation_binding_id: impl Into<String>,
        state: BuildCapabilityState,
    ) -> Self {
        self.build_by_binding
            .insert(implementation_binding_id.into(), state);
        self
    }

    pub fn with_runtime(
        mut self,
        implementation_binding_id: impl Into<String>,
        state: RuntimeReadinessState,
    ) -> Self {
        self.runtime_by_binding
            .insert(implementation_binding_id.into(), state);
        self
    }

    pub fn with_credential(
        mut self,
        provider_route_id: impl Into<String>,
        state: CredentialReadinessState,
    ) -> Self {
        self.credential_by_route
            .insert(provider_route_id.into(), state);
        self
    }

    pub fn with_admission(
        mut self,
        admission_policy_id: impl Into<String>,
        state: EvidenceAdmissionState,
    ) -> Self {
        self.admission_by_policy
            .insert(admission_policy_id.into(), state);
        self
    }

    pub fn with_policy(
        mut self,
        capability_id: impl Into<String>,
        state: CapabilityPolicyState,
    ) -> Self {
        self.product_policy_by_capability
            .insert(capability_id.into(), state);
        self
    }

    pub fn without_runtime(mut self, implementation_binding_id: &str) -> Self {
        self.runtime_by_binding.remove(implementation_binding_id);
        self
    }

    pub fn without_credential(mut self, provider_route_id: &str) -> Self {
        self.credential_by_route.remove(provider_route_id);
        self
    }

    pub fn without_admission(mut self, admission_policy_id: &str) -> Self {
        self.admission_by_policy.remove(admission_policy_id);
        self
    }

    pub(crate) fn without_admission_and_policy(mut self) -> Self {
        self.admission_by_policy.clear();
        self.product_policy_by_capability.clear();
        self
    }

    pub(crate) fn merge(&mut self, other: Self) {
        self.build_by_binding.extend(other.build_by_binding);
        self.runtime_by_binding.extend(other.runtime_by_binding);
        self.credential_by_route.extend(other.credential_by_route);
        self.admission_by_policy.extend(other.admission_by_policy);
        self.product_policy_by_capability
            .extend(other.product_policy_by_capability);
    }

    fn inputs_for(&self, declaration: &CapabilityDeclaration) -> EffectiveCapabilityInputs {
        let build = declaration
            .implementation_binding_id
            .as_deref()
            .and_then(|binding| self.build_by_binding.get(binding).copied());
        let runtime = declaration
            .implementation_binding_id
            .as_deref()
            .and_then(|binding| self.runtime_by_binding.get(binding).copied());
        let credential =
            aggregate_route_credential(&declaration.provider_route_ids, &self.credential_by_route);
        let admission = declaration
            .admission_policy_id
            .as_deref()
            .and_then(|policy| self.admission_by_policy.get(policy).copied());
        let policy = self
            .product_policy_by_capability
            .get(&declaration.capability_id)
            .copied();
        EffectiveCapabilityInputs {
            build,
            runtime,
            credential,
            admission,
            policy,
        }
    }

    fn inputs_for_route(
        &self,
        declaration: &CapabilityDeclaration,
        route: &ProviderRouteDescriptor,
    ) -> EffectiveCapabilityInputs {
        let mut inputs = self.inputs_for(declaration);
        inputs.credential = self.credential_by_route.get(&route.route_id).copied();
        inputs
    }
}

fn aggregate_route_credential(
    route_ids: &[String],
    observations: &BTreeMap<String, CredentialReadinessState>,
) -> Option<CredentialReadinessState> {
    let states = route_ids
        .iter()
        .filter_map(|route_id| observations.get(route_id).copied())
        .collect::<Vec<_>>();
    if states.contains(&CredentialReadinessState::Ready) {
        Some(CredentialReadinessState::Ready)
    } else if states.contains(&CredentialReadinessState::Missing) {
        Some(CredentialReadinessState::Missing)
    } else {
        None
    }
}

/// The single combination owner for static declarations and independent
/// runtime observations. Every query is deterministic and network-free.
pub struct EffectiveCapabilityService<'a> {
    registry: &'a ResourceRegistry,
    observations: CapabilityRuntimeObservations,
}

impl EffectiveCapabilityService<'static> {
    pub fn embedded(observations: CapabilityRuntimeObservations) -> Result<Self, String> {
        Ok(Self::new(ResourceRegistry::embedded_ref()?, observations))
    }
}

impl<'a> EffectiveCapabilityService<'a> {
    pub fn new(
        registry: &'a ResourceRegistry,
        observations: CapabilityRuntimeObservations,
    ) -> Self {
        Self {
            registry,
            observations,
        }
    }

    pub fn list_known_resources(&self) -> Vec<KnownResourceSummary> {
        let mut resources = self
            .registry
            .resources
            .iter()
            .map(|resource| {
                let common = resource.common();
                KnownResourceSummary {
                    resource_id: common.resource_id.clone(),
                    display_name: common.display_name.clone(),
                    descriptor_kind: descriptor_kind(resource),
                    capability_ids: common
                        .capabilities
                        .iter()
                        .map(|capability| capability.capability_id.clone())
                        .collect(),
                }
            })
            .collect::<Vec<_>>();
        resources.sort_by(|left, right| left.resource_id.cmp(&right.resource_id));
        resources
    }

    pub fn resource_descriptor(&self, resource_id: &str) -> Option<ResourceDescriptor> {
        self.registry.resource(resource_id).cloned()
    }

    pub fn supported_search_capabilities(&self, resource_id: &str) -> Vec<CapabilityDeclaration> {
        self.registry
            .resource(resource_id)
            .into_iter()
            .flat_map(|resource| &resource.common().capabilities)
            .filter(|capability| capability.kind == CapabilityKind::Search)
            .cloned()
            .collect()
    }

    pub fn delivery_capabilities(&self, resource_id: &str) -> Vec<ResourceDeliveryCapability> {
        let Some(resource) = self.registry.resource(resource_id) else {
            return Vec::new();
        };
        let common = resource.common();
        common
            .capabilities
            .iter()
            .filter_map(|capability| {
                let delivery_profile_id = capability.delivery_profile_id.as_deref()?;
                let delivery = common
                    .delivery_profiles
                    .iter()
                    .find(|profile| profile.delivery_profile_id == delivery_profile_id)?;
                Some(ResourceDeliveryCapability {
                    resource_id: common.resource_id.clone(),
                    capability: capability.clone(),
                    delivery: delivery.clone(),
                })
            })
            .collect()
    }

    pub fn effective_capability(&self, capability_id: &str) -> Option<EffectiveCapabilityStatus> {
        self.registry.resources.iter().find_map(|resource| {
            let declaration = resource
                .common()
                .capabilities
                .iter()
                .find(|capability| capability.capability_id == capability_id)?;
            let inputs = self.observations.inputs_for(declaration);
            Some(EffectiveCapabilityStatus {
                resource_id: resource.common().resource_id.clone(),
                capability: declaration.clone(),
                projection: project_effective_capability(declaration, inputs),
                inputs,
            })
        })
    }

    /// Choose the first reviewed provider route that is executable under the
    /// current runtime observations. The user supplies only the product and
    /// operation; route priority and eligibility remain registry-owned.
    /// Return every currently executable reviewed route in registry priority order.
    /// This is the bounded source-fallback plan; it never expands admission.
    pub fn executable_provider_routes(
        &self,
        resource_id: &str,
        capability_kind: CapabilityKind,
    ) -> Result<Vec<ProviderRouteCandidate<'a>>, String> {
        Ok(self
            .registry
            .provider_route_candidates(resource_id, capability_kind)?
            .into_iter()
            .filter(|candidate| {
                project_effective_capability(
                    candidate.capability,
                    self.observations
                        .inputs_for_route(candidate.capability, candidate.route),
                )
                .state
                    == insar_core::EffectiveCapabilityState::Executable
            })
            .collect())
    }

    /// Resolve a catalogue product from the exact seed source and, only when
    /// that product declares multiple Search routes, return the currently
    /// executable fallback plan. Single-route products deliberately return
    /// `None` so their existing request path is preserved unchanged.
    pub fn executable_catalogue_fallback_routes_for_source(
        &self,
        logical_dataset: &str,
        provider_id: &str,
        native_collection_id: &str,
    ) -> Result<Option<Vec<ProviderRouteCandidate<'a>>>, String> {
        let Some(resource) = self.registry.catalogue_search_resource_for_source(
            logical_dataset,
            provider_id,
            native_collection_id,
        )?
        else {
            return Ok(None);
        };
        let resource_id = resource.common().resource_id.as_str();
        if self
            .registry
            .provider_route_candidates(resource_id, CapabilityKind::Search)?
            .len()
            <= 1
        {
            return Ok(None);
        }
        self.executable_provider_routes(resource_id, CapabilityKind::Search)
            .map(Some)
    }

    pub fn select_provider_route(
        &self,
        resource_id: &str,
        capability_kind: CapabilityKind,
    ) -> Result<Option<ProviderRouteCandidate<'a>>, String> {
        Ok(self
            .executable_provider_routes(resource_id, capability_kind)?
            .into_iter()
            .next())
    }
    pub fn resource_dependencies(
        &self,
        resource_id: Option<&str>,
        capability_id: Option<&str>,
        planning_context_id: Option<&str>,
    ) -> Vec<ResourceDependency> {
        self.registry
            .dependencies
            .iter()
            .filter(|dependency| {
                optional_identity_matches(resource_id, dependency.consumer.resource_id.as_deref())
                    && optional_identity_matches(
                        capability_id,
                        dependency.consumer.capability_id.as_deref(),
                    )
                    && optional_identity_matches(
                        planning_context_id,
                        dependency.consumer.planning_context_id.as_deref(),
                    )
            })
            .cloned()
            .collect()
    }
}

fn descriptor_kind(resource: &ResourceDescriptor) -> ResourceDescriptorKind {
    match resource {
        ResourceDescriptor::ScienceDataset { .. } => ResourceDescriptorKind::ScienceDataset,
        ResourceDescriptor::Orbit { .. } => ResourceDescriptorKind::Orbit,
        ResourceDescriptor::Dem { .. } => ResourceDescriptorKind::Dem,
        ResourceDescriptor::Atmospheric { .. } => ResourceDescriptorKind::Atmospheric,
    }
}

fn optional_identity_matches(requested: Option<&str>, actual: Option<&str>) -> bool {
    requested.is_none_or(|requested| actual == Some(requested))
}

pub(crate) fn application_default_capability_observations() -> CapabilityRuntimeObservations {
    let authorities = embedded_capability_authorities()
        .expect("embedded capability authorities are valid repository contracts");
    let mut observations = CapabilityRuntimeObservations::default();
    for entry in authorities.build.entries {
        let state = if entry
            .required_features
            .iter()
            .all(|feature| compile_time_feature_available(feature))
        {
            BuildCapabilityState::Available
        } else {
            BuildCapabilityState::Unavailable
        };
        observations = observations.with_build(entry.implementation_binding_id, state);
    }
    for entry in authorities.policy.entries {
        observations = observations.with_policy(
            entry.capability_id,
            if entry.implemented && entry.production_admitted {
                CapabilityPolicyState::Allowed
            } else {
                CapabilityPolicyState::Denied
            },
        );
    }
    for record in &authorities.admission.records {
        observations = observations.with_admission(
            record.admission_policy_id(),
            if record.accepted_and_applicable() {
                EvidenceAdmissionState::Accepted
            } else {
                EvidenceAdmissionState::Rejected
            },
        );
    }
    observations
}

pub(crate) fn application_default_enabled(capability_id: &str) -> bool {
    embedded_capability_authorities()
        .expect("embedded capability authorities are valid repository contracts")
        .policy
        .entries
        .into_iter()
        .find(|entry| entry.capability_id == capability_id)
        .is_some_and(|entry| entry.implemented && entry.default_enabled)
}

pub(crate) fn with_opentopography_readiness(
    mut observations: CapabilityRuntimeObservations,
    status: &str,
) -> CapabilityRuntimeObservations {
    let readiness = if status == "set" {
        CredentialReadinessState::Ready
    } else {
        CredentialReadinessState::Missing
    };
    let registry = ResourceRegistry::embedded_ref()
        .expect("embedded resource registry is a valid repository contract");
    for resource in &registry.resources {
        for route in &resource.common().provider_routes {
            if route.provider_id == "opentopography" {
                observations = observations.with_credential(&route.route_id, readiness);
            }
        }
    }
    observations
}

#[cfg(test)]
#[path = "opentopography_readiness_tests.rs"]
mod opentopography_readiness_tests;

pub(crate) fn application_earthdata_route_ids() -> &'static [&'static str] {
    &[
        "route.asf.alos2-scansar-l1.1.v1",
        "route.asf.alos-palsar.v1",
        "route.asf.ers.v1",
        "route.asf.jers-1.v1",
        "route.asf.nisar-rslc.archive.v1",
        "route.asf.nisar-rslc.catalog.v1",
        "route.asf.nisar-rslc.remote-subset.v1",
        "route.asf.radarsat-1.v1",
        "route.asf.seasat.v1",
        "route.asf.sentinel-1.poeorb.v1",
        "route.asf.sentinel-1.slc-burst.v1",
        "route.asf.sentinel-1.standard.v1",
        // The NISAR DEM tiles and ASTER GDEM v3 tiles are served by the same
        // NASA Earthdata cloud and reuse the same Earthdata account session.
        "route.asf.nisar-dem.v1",
        "route.lpdaac.aster-gdem-v3.earthdata.v1",
        // OPERA DISP-S1, ARIA S1-GUNW, and CDDIS IGS GIM archive routes.
        "route.asf.opera-s1.archive.v1",
        "route.asf.aria-gunw.archive.v1",
        "route.cddis.ionex.v1",
        // Open NASA Earthdata Cloud (LAADS DAAC & LP DAAC) science download routes.
        "route.laads.nasa.black-marble.download.v1",
        "route.lpdaac.nasa.emit.download.v1",
        "route.lpdaac.nasa.ecostress.download.v1",
    ]
}

fn compile_time_feature_available(feature: &str) -> bool {
    matches!(feature, "nisar-subset-production") && cfg!(feature = "nisar-subset-production")
}

#[cfg(test)]
mod tests {
    use super::{
        application_default_capability_observations, embedded_capability_authorities,
        CapabilityRuntimeObservations, EffectiveCapabilityService, ALOS2_ADMISSION_POLICY,
        NISAR_GEOMETRY_ALGORITHM_VERSION, SENTINEL2_FULL_ARCHIVE_ADMISSION_POLICY,
    };
    use insar_core::{
        BuildCapabilityState, CapabilityKind, CapabilityPolicyState, CredentialReadinessState,
        EvidenceAdmissionState, ResourceRegistry, RuntimeReadinessState,
    };

    const LANDSAT_RESOURCE_ID: &str = "science.landsat.c2-l2.v1";
    const MPC_BINDING: &str = "planetary-computer.landsat-c2-l2.v1";
    const MPC_ROUTE: &str = "route.mpc.landsat.c2-l2.v1";
    const MPC_SEARCH_CAPABILITY: &str = "capability.landsat.c2-l2.search.v1";
    const EARTH_BINDING: &str = "element84.earth-search.landsat-c2-l2.v1";
    const EARTH_ROUTE: &str = "route.element84.earth-search.landsat.c2-l2.v1";
    const EARTH_SEARCH_CAPABILITY: &str = "capability.landsat.c2-l2.earth-search-search.v1";
    const USGS_BINDING: &str = "usgs.eros.m2m.landsat.c2.l2.scene.v1";
    const USGS_ROUTE: &str = "route.usgs.landsat.c2-l2.v1";
    const USGS_SEARCH_CAPABILITY: &str = "capability.landsat.c2-l2.usgs-search.v1";
    const USGS_ADMISSION: &str = "admission.usgs.landsat-c2-l2.scene-search.v1";
    const CDSE_S2_BINDING: &str = "cdse.sentinel-2.l2a.collection.v1";
    const CDSE_S2_ROUTE: &str = "route.cdse.sentinel-2.l2a.search.v1";
    const CDSE_S2_SEARCH_CAPABILITY: &str = "capability.sentinel-2.l2a.search.v1";
    const MPC_S2_BINDING: &str = "planetary-computer.sentinel-2-l2a.v1";
    const MPC_S2_ROUTE: &str = "route.mpc.sentinel-2.l2a.v1";
    const MPC_S2_SEARCH_CAPABILITY: &str = "capability.sentinel-2.l2a.mpc-search.v1";

    fn dual_landsat_observations() -> CapabilityRuntimeObservations {
        CapabilityRuntimeObservations::default()
            .with_build(MPC_BINDING, BuildCapabilityState::Available)
            .with_runtime(MPC_BINDING, RuntimeReadinessState::Ready)
            .with_policy(MPC_SEARCH_CAPABILITY, CapabilityPolicyState::Allowed)
            .with_build(EARTH_BINDING, BuildCapabilityState::Available)
            .with_runtime(EARTH_BINDING, RuntimeReadinessState::Ready)
            .with_policy(EARTH_SEARCH_CAPABILITY, CapabilityPolicyState::Allowed)
            .with_build(USGS_BINDING, BuildCapabilityState::Available)
            .with_runtime(USGS_BINDING, RuntimeReadinessState::Ready)
            .with_credential(USGS_ROUTE, CredentialReadinessState::Ready)
            .with_admission(USGS_ADMISSION, EvidenceAdmissionState::Accepted)
            .with_policy(USGS_SEARCH_CAPABILITY, CapabilityPolicyState::Allowed)
    }

    fn dual_sentinel2_observations() -> CapabilityRuntimeObservations {
        CapabilityRuntimeObservations::default()
            .with_build(CDSE_S2_BINDING, BuildCapabilityState::Available)
            .with_runtime(CDSE_S2_BINDING, RuntimeReadinessState::Ready)
            .with_credential(CDSE_S2_ROUTE, CredentialReadinessState::Ready)
            .with_policy(CDSE_S2_SEARCH_CAPABILITY, CapabilityPolicyState::Allowed)
            .with_build(MPC_S2_BINDING, BuildCapabilityState::Available)
            .with_runtime(MPC_S2_BINDING, RuntimeReadinessState::Ready)
            .with_policy(MPC_S2_SEARCH_CAPABILITY, CapabilityPolicyState::Allowed)
    }

    #[test]
    fn provider_selection_uses_route_specific_readiness_and_never_user_choice() {
        let registry = ResourceRegistry::embedded_ref().unwrap();
        let preferred = EffectiveCapabilityService::new(registry, dual_landsat_observations())
            .select_provider_route(LANDSAT_RESOURCE_ID, CapabilityKind::Search)
            .unwrap()
            .expect("one reviewed route is executable");
        assert_eq!(preferred.route.route_id, MPC_ROUTE);
        let ordered = EffectiveCapabilityService::new(registry, dual_landsat_observations())
            .executable_provider_routes(LANDSAT_RESOURCE_ID, CapabilityKind::Search)
            .unwrap();
        assert_eq!(
            ordered
                .iter()
                .map(|candidate| candidate.route.route_id.as_str())
                .collect::<Vec<_>>(),
            vec![MPC_ROUTE, EARTH_ROUTE, USGS_ROUTE]
        );

        let fallback = EffectiveCapabilityService::new(
            registry,
            dual_landsat_observations().with_runtime(MPC_BINDING, RuntimeReadinessState::NotReady),
        )
        .select_provider_route(LANDSAT_RESOURCE_ID, CapabilityKind::Search)
        .unwrap()
        .expect("reviewed Earth Search fallback is executable");
        assert_eq!(fallback.route.route_id, EARTH_ROUTE);

        let usgs_fallback = EffectiveCapabilityService::new(
            registry,
            dual_landsat_observations()
                .with_runtime(MPC_BINDING, RuntimeReadinessState::NotReady)
                .with_runtime(EARTH_BINDING, RuntimeReadinessState::NotReady),
        )
        .select_provider_route(LANDSAT_RESOURCE_ID, CapabilityKind::Search)
        .unwrap()
        .expect("explicitly admitted USGS remains the third candidate");
        assert_eq!(usgs_fallback.route.route_id, USGS_ROUTE);

        let primary_only = EffectiveCapabilityService::new(
            registry,
            dual_landsat_observations()
                .with_credential(USGS_ROUTE, CredentialReadinessState::Missing)
                .with_admission(USGS_ADMISSION, EvidenceAdmissionState::Rejected),
        )
        .select_provider_route(LANDSAT_RESOURCE_ID, CapabilityKind::Search)
        .unwrap()
        .expect("backup failure must not block the primary route");
        assert_eq!(primary_only.route.route_id, MPC_ROUTE);

        let catalogue_plan = EffectiveCapabilityService::new(registry, dual_landsat_observations())
            .executable_catalogue_fallback_routes_for_source(
                "optical.landsat.c2.l2",
                "usgs.eros.m2m",
                "landsat_ot_c2_l2",
            )
            .unwrap()
            .expect("multi-route catalogue product owns a fallback plan");
        assert_eq!(
            catalogue_plan
                .iter()
                .map(|candidate| candidate.route.route_id.as_str())
                .collect::<Vec<_>>(),
            vec![MPC_ROUTE, EARTH_ROUTE, USGS_ROUTE]
        );
        let sentinel2_plan =
            EffectiveCapabilityService::new(registry, dual_sentinel2_observations())
                .executable_catalogue_fallback_routes_for_source(
                    "optical.sentinel-2.l2a",
                    "copernicus.dataspace.odata",
                    "SENTINEL-2",
                )
                .unwrap()
                .expect("multi-route Sentinel-2 search owns a bounded fallback plan");
        assert_eq!(
            sentinel2_plan
                .iter()
                .map(|candidate| candidate.route.route_id.as_str())
                .collect::<Vec<_>>(),
            vec![CDSE_S2_ROUTE, MPC_S2_ROUTE]
        );
        let anonymous_only = EffectiveCapabilityService::new(
            registry,
            dual_sentinel2_observations()
                .with_credential(CDSE_S2_ROUTE, CredentialReadinessState::Missing),
        )
        .executable_catalogue_fallback_routes_for_source(
            "optical.sentinel-2.l2a",
            "copernicus.dataspace.odata",
            "SENTINEL-2",
        )
        .unwrap()
        .expect("public CDSE search does not require a download login");
        assert_eq!(
            anonymous_only
                .iter()
                .map(|candidate| candidate.route.route_id.as_str())
                .collect::<Vec<_>>(),
            vec![CDSE_S2_ROUTE, MPC_S2_ROUTE]
        );
        // Runtime failure may still choose Microsoft; missing login alone may not.
        let no_cdse_runtime = EffectiveCapabilityService::new(
            registry,
            dual_sentinel2_observations()
                .with_runtime(CDSE_S2_BINDING, RuntimeReadinessState::NotReady),
        )
        .executable_catalogue_fallback_routes_for_source(
            "optical.sentinel-2.l2a",
            "copernicus.dataspace.odata",
            "SENTINEL-2",
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            no_cdse_runtime
                .iter()
                .map(|c| c.route.route_id.as_str())
                .collect::<Vec<_>>(),
            vec![MPC_S2_ROUTE]
        );
        let archive = registry
            .resources
            .iter()
            .flat_map(|r| &r.common().capabilities)
            .find(|c| c.capability_id == "capability.sentinel-2.l2a.full-archive.v1")
            .unwrap();
        assert!(archive.runtime_requirements.credential_readiness);
        assert!(archive.runtime_requirements.admission_evidence);
    }

    #[test]
    fn default_authorities_do_not_silently_enable_unadmitted_usgs_fallback() {
        let registry = ResourceRegistry::embedded_ref().unwrap();
        let observations = application_default_capability_observations()
            .with_runtime(MPC_BINDING, RuntimeReadinessState::NotReady)
            .with_runtime(USGS_BINDING, RuntimeReadinessState::Ready)
            .with_credential(USGS_ROUTE, CredentialReadinessState::Ready);
        let selected = EffectiveCapabilityService::new(registry, observations)
            .select_provider_route(LANDSAT_RESOURCE_ID, CapabilityKind::Search)
            .unwrap();
        assert!(selected.is_none());
        let candidates = EffectiveCapabilityService::new(
            registry,
            application_default_capability_observations()
                .with_runtime(MPC_BINDING, RuntimeReadinessState::NotReady)
                .with_runtime(USGS_BINDING, RuntimeReadinessState::Ready)
                .with_credential(USGS_ROUTE, CredentialReadinessState::Ready),
        )
        .executable_provider_routes(LANDSAT_RESOURCE_ID, CapabilityKind::Search)
        .unwrap();
        assert!(
            candidates.is_empty(),
            "unadmitted USGS cannot enter fallback plan"
        );
    }

    #[test]
    fn route_specific_credential_does_not_inherit_another_route_ready_state() {
        let registry = ResourceRegistry::embedded_ref().unwrap();
        let landsat = registry.resource(LANDSAT_RESOURCE_ID).unwrap();
        let common = landsat.common();
        let declaration = common
            .capabilities
            .iter()
            .find(|capability| capability.capability_id == USGS_SEARCH_CAPABILITY)
            .unwrap();
        let route = common
            .provider_routes
            .iter()
            .find(|route| route.route_id == USGS_ROUTE)
            .unwrap();
        let mut multi_route_declaration = declaration.clone();
        multi_route_declaration
            .provider_route_ids
            .push("route.fixture.ready.v1".to_owned());
        let observations = dual_landsat_observations()
            .with_credential(USGS_ROUTE, CredentialReadinessState::Missing)
            .with_credential("route.fixture.ready.v1", CredentialReadinessState::Ready);

        assert_eq!(
            observations.inputs_for(&multi_route_declaration).credential,
            Some(CredentialReadinessState::Ready),
            "aggregate display keeps its existing any-ready behavior"
        );
        assert_eq!(
            observations
                .inputs_for_route(&multi_route_declaration, route)
                .credential,
            Some(CredentialReadinessState::Missing),
            "provider selection evaluates the candidate route itself"
        );
    }

    #[test]
    fn changed_scientific_applicability_fails_closed() {
        let mut authorities = embedded_capability_authorities().unwrap();
        let record = authorities
            .admission
            .records
            .first_mut()
            .expect("frozen C2 record");
        let super::CapabilityAdmissionEvidenceRecord::Nisar { applicability, .. } = record else {
            panic!("first frozen record must remain NISAR C2");
        };
        applicability.geometry_algorithm_version =
            format!("{NISAR_GEOMETRY_ALGORITHM_VERSION}-changed");
        assert!(!record.accepted_and_applicable());
    }

    #[test]
    fn embedded_authorities_are_versioned_and_have_frozen_records() {
        let authorities = embedded_capability_authorities().unwrap();
        assert_eq!(authorities.build.schema_id, "capability-build-manifest.v1");
        assert_eq!(authorities.build.schema_version, 1);
        assert!(!authorities.build.manifest_id.is_empty());
        assert!(!authorities.build.build_profile_id.is_empty());
        assert_eq!(authorities.policy.schema_id, "capability-product-policy.v1");
        assert_eq!(authorities.policy.schema_version, 1);
        assert!(!authorities.policy.policy_id.is_empty());
        assert_eq!(
            authorities.admission.schema_id,
            "capability-admission-evidence.v1"
        );
        assert_eq!(authorities.admission.schema_version, 1);
        assert!(!authorities.admission.authority_id.is_empty());
        assert_eq!(authorities.admission.records.len(), 3);
    }

    #[test]
    fn alos2_complete_product_remains_default_off() {
        assert!(!super::application_default_enabled(
            "capability.alos2-scansar-l1.1.full-archive.v1"
        ));
    }

    #[test]
    fn legacy_v1_nisar_record_without_kind_remains_compatible() {
        let mut authority: serde_json::Value =
            serde_json::from_str(super::ADMISSION_EVIDENCE_JSON).unwrap();
        authority["records"][0]
            .as_object_mut()
            .unwrap()
            .remove("record_kind");
        let parsed: super::CapabilityAdmissionEvidenceAuthority =
            serde_json::from_value(authority).unwrap();
        assert_eq!(parsed.records.len(), 3);
        assert!(parsed.records[0].accepted_and_applicable());
    }

    #[test]
    fn duplicate_admission_identity_fails_closed() {
        let mut authorities = embedded_capability_authorities().unwrap();
        let duplicate = authorities.admission.records[0].clone();
        authorities.admission.records.push(duplicate);
        assert!(
            super::validate_admission_record_identities(&authorities.admission.records).is_err()
        );
    }

    #[test]
    fn alos2_evidence_identity_and_applicability_mutations_fail_closed() {
        let mutations: [fn(&mut super::CapabilityAdmissionEvidenceRecord); 8] = [
            |record| {
                if let super::CapabilityAdmissionEvidenceRecord::Alos2 {
                    provider_product_id,
                    ..
                } = record
                {
                    provider_product_id.push_str("-changed");
                }
            },
            |record| {
                if let super::CapabilityAdmissionEvidenceRecord::Alos2 { file_name, .. } = record {
                    file_name.push_str(".changed");
                }
            },
            |record| {
                if let super::CapabilityAdmissionEvidenceRecord::Alos2 {
                    expected_size_bytes,
                    ..
                } = record
                {
                    *expected_size_bytes += 1;
                }
            },
            |record| {
                if let super::CapabilityAdmissionEvidenceRecord::Alos2 { source_md5, .. } = record {
                    source_md5.replace_range(0..1, "1");
                }
            },
            |record| {
                if let super::CapabilityAdmissionEvidenceRecord::Alos2 {
                    lifecycle_facts, ..
                } = record
                {
                    lifecycle_facts.store_reopen_completed = false;
                }
            },
            |record| {
                if let super::CapabilityAdmissionEvidenceRecord::Alos2 { applicability, .. } =
                    record
                {
                    applicability.validation_profile_id.push_str("-changed");
                }
            },
            |record| {
                if let super::CapabilityAdmissionEvidenceRecord::Alos2 {
                    scientific_scope, ..
                } = record
                {
                    scientific_scope.subset = true;
                }
            },
            |record| {
                if let super::CapabilityAdmissionEvidenceRecord::Alos2 {
                    reviewed_hashes, ..
                } = record
                {
                    reviewed_hashes[0].value.replace_range(0..1, "1");
                }
            },
        ];

        for mutate in mutations {
            let mut authorities = embedded_capability_authorities().unwrap();
            let record = authorities
                .admission
                .records
                .iter_mut()
                .find(|record| record.admission_policy_id() == ALOS2_ADMISSION_POLICY)
                .expect("ALOS-2 admission record");
            mutate(record);
            assert!(!record.accepted_and_applicable());
        }
    }

    #[test]
    fn alos2_evidence_rejects_absolute_or_raw_locator_references() {
        for locator in [
            "https://datapool.asf.alaska.edu/product.zip",
            "data:application/octet-stream;base64,AAAA",
            "mailto:operator@example.invalid",
            "http:product.zip",
            "E:/evidence/evidence.json",
            "docs/../../external/evidence.json",
            "docs//evidence.json",
        ] {
            let mut authorities = embedded_capability_authorities().unwrap();
            let record = authorities
                .admission
                .records
                .iter_mut()
                .find(|record| record.admission_policy_id() == ALOS2_ADMISSION_POLICY)
                .expect("ALOS-2 admission record");
            {
                let super::CapabilityAdmissionEvidenceRecord::Alos2 {
                    reviewed_references,
                    ..
                } = record
                else {
                    panic!("ALOS-2 admission record kind changed");
                };
                reviewed_references.push(locator.to_owned());
            }
            assert!(!record.accepted_and_applicable());
        }
    }

    #[test]
    fn sentinel2_evidence_identity_and_applicability_mutations_fail_closed() {
        let mutations: [fn(&mut super::CapabilityAdmissionEvidenceRecord); 9] = [
            |record| {
                if let super::CapabilityAdmissionEvidenceRecord::Sentinel2 { evidence_id, .. } =
                    record
                {
                    evidence_id.push_str("-changed");
                }
            },
            |record| {
                if let super::CapabilityAdmissionEvidenceRecord::Sentinel2 {
                    provider_product_id,
                    ..
                } = record
                {
                    provider_product_id.push_str("-changed");
                }
            },
            |record| {
                if let super::CapabilityAdmissionEvidenceRecord::Sentinel2 { file_name, .. } =
                    record
                {
                    file_name.push_str(".changed");
                }
            },
            |record| {
                if let super::CapabilityAdmissionEvidenceRecord::Sentinel2 {
                    expected_size_bytes,
                    ..
                } = record
                {
                    *expected_size_bytes += 1;
                }
            },
            |record| {
                if let super::CapabilityAdmissionEvidenceRecord::Sentinel2 {
                    observed_checksum,
                    ..
                } = record
                {
                    observed_checksum.replace_range(0..1, "1");
                }
            },
            |record| {
                if let super::CapabilityAdmissionEvidenceRecord::Sentinel2 {
                    lifecycle_facts, ..
                } = record
                {
                    lifecycle_facts.store_reopen_completed = false;
                }
            },
            |record| {
                if let super::CapabilityAdmissionEvidenceRecord::Sentinel2 {
                    applicability, ..
                } = record
                {
                    applicability.collection_binding_id.push_str("-changed");
                }
            },
            |record| {
                if let super::CapabilityAdmissionEvidenceRecord::Sentinel2 {
                    scientific_scope,
                    ..
                } = record
                {
                    scientific_scope.subset = true;
                }
            },
            |record| {
                if let super::CapabilityAdmissionEvidenceRecord::Sentinel2 {
                    reviewed_hashes, ..
                } = record
                {
                    reviewed_hashes[0].value.replace_range(0..1, "1");
                }
            },
        ];

        for mutate in mutations {
            let mut authorities = embedded_capability_authorities().unwrap();
            let record = authorities
                .admission
                .records
                .iter_mut()
                .find(|record| {
                    record.admission_policy_id() == SENTINEL2_FULL_ARCHIVE_ADMISSION_POLICY
                })
                .expect("Sentinel-2 admission record");
            mutate(record);
            assert!(!record.accepted_and_applicable());
        }
    }

    #[test]
    fn sentinel2_evidence_is_accepted_and_applicable_before_mutation() {
        let authorities = embedded_capability_authorities().unwrap();
        let record = authorities
            .admission
            .records
            .iter()
            .find(|record| record.admission_policy_id() == SENTINEL2_FULL_ARCHIVE_ADMISSION_POLICY)
            .expect("Sentinel-2 admission record");
        assert!(record.accepted_and_applicable());
    }

    #[test]
    fn open_eo_earthdata_download_capabilities_follow_earthdata_readiness() {
        use super::application_earthdata_route_ids;
        use insar_core::EffectiveCapabilityState;

        let base_obs = application_default_capability_observations()
            .with_runtime("laads.viirs.black-marble.v1", RuntimeReadinessState::Ready)
            .with_runtime("lpdaac.emit.l2a.v1", RuntimeReadinessState::Ready)
            .with_runtime("lpdaac.ecostress.l2t.v1", RuntimeReadinessState::Ready);

        let mut ready_obs = base_obs.clone();
        let mut missing_obs = base_obs;
        for route in application_earthdata_route_ids() {
            ready_obs = ready_obs.with_credential(*route, CredentialReadinessState::Ready);
            missing_obs = missing_obs.with_credential(*route, CredentialReadinessState::Missing);
        }

        let ready_svc = EffectiveCapabilityService::embedded(ready_obs).unwrap();
        let missing_svc = EffectiveCapabilityService::embedded(missing_obs).unwrap();

        for cap_id in [
            "capability.nasa.black-marble.download.v1",
            "capability.nasa.emit.download.v1",
            "capability.nasa.ecostress.download.v1",
        ] {
            let ready_status = ready_svc
                .effective_capability(cap_id)
                .unwrap_or_else(|| panic!("missing capability {cap_id}"));
            assert_eq!(
                ready_status.projection.state,
                EffectiveCapabilityState::Executable,
                "configured Earthdata must unlock {cap_id}, got {:?}",
                ready_status.projection
            );

            let missing_status = missing_svc
                .effective_capability(cap_id)
                .unwrap_or_else(|| panic!("missing capability {cap_id}"));
            assert_eq!(
                missing_status.projection.state,
                EffectiveCapabilityState::BlockedCredential,
                "missing Earthdata must block {cap_id}, got {:?}",
                missing_status.projection
            );
        }
    }

    #[test]
    fn application_earthdata_routes_cover_every_required_earthdata_route_and_unlock_aster_gdem() {
        use super::application_earthdata_route_ids;
        use insar_core::{CredentialRequirement, EffectiveCapabilityState, ResourceRegistry};

        let listed: std::collections::BTreeSet<&str> =
            application_earthdata_route_ids().iter().copied().collect();
        let registry = ResourceRegistry::embedded_ref().unwrap();
        for resource in &registry.resources {
            for route in &resource.common().provider_routes {
                if route.provider_id.contains("earthdata")
                    && route.credential_requirement == CredentialRequirement::Required
                {
                    assert!(
                        listed.contains(route.route_id.as_str()),
                        "required Earthdata route {} on {} missing from application_earthdata_route_ids()",
                        route.route_id,
                        resource.common().resource_id
                    );
                }
            }
        }

        let base_obs = application_default_capability_observations()
            .with_build("lpdaac.aster-gdem.v3", BuildCapabilityState::Available)
            .with_runtime("lpdaac.aster-gdem.v3", RuntimeReadinessState::Ready);
        let mut ready_obs = base_obs.clone();
        let mut missing_obs = base_obs;
        for route in application_earthdata_route_ids() {
            ready_obs = ready_obs.with_credential(*route, CredentialReadinessState::Ready);
            missing_obs = missing_obs.with_credential(*route, CredentialReadinessState::Missing);
        }
        let ready = EffectiveCapabilityService::embedded(ready_obs)
            .unwrap()
            .effective_capability("capability.dem.aster-gdem-v3.download.v1")
            .unwrap();
        assert_eq!(ready.projection.state, EffectiveCapabilityState::Executable);

        let missing = EffectiveCapabilityService::embedded(missing_obs)
            .unwrap()
            .effective_capability("capability.dem.aster-gdem-v3.download.v1")
            .unwrap();
        assert_eq!(
            missing.projection.state,
            EffectiveCapabilityState::BlockedCredential
        );
    }
}
