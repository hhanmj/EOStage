#[path = "support/nisar.rs"]
#[allow(dead_code)]
mod nisar_support;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::SystemTime;

use insar_core::{AssetAccessScheme, AssetRole, SnapshotItem};
use insar_download::{
    evaluate_resolver_cutover_eligibility, AsfCollectionProfileRegistry,
    AssetExecutionPolicyRegistry, AssetResolver, CatalogExecutionPolicyError,
    CatalogExecutionPolicyInput, CatalogExecutionPolicyRegistry, CatalogPolicyContext, Clock,
    CredentialMaterial, CredentialProvider, CredentialProviderError, EarthdataAssetResolver,
    ResolveContext, ResolverCutoverEligibility,
};
use insar_provider::catalog::{
    CollectionProfileRegistry, NISAR_LOGICAL_DATASET, NISAR_NATIVE_COLLECTION_ID, NISAR_PROFILE_ID,
    NISAR_PROVIDER_ID, NISAR_RESOLVER_POLICY_ID_V1, NISAR_RESOLVER_POLICY_ID_V2,
};
use nisar_support::{frozen_item, sha256_hex, FILE_NAME, OFFICIAL_HREF, QA_ASSET_KEY};
use serde_json::{json, Value};

const BODY: &[u8] = b"\x89HDF\r\n\x1a\nnisar-policy";

struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> SystemTime {
        SystemTime::UNIX_EPOCH
    }
}

#[derive(Default)]
struct RecordingCredentials {
    calls: AtomicUsize,
}

impl CredentialProvider for RecordingCredentials {
    fn resolve(&self, profile: &str) -> Result<CredentialMaterial, CredentialProviderError> {
        assert_eq!(profile, "earthdata-default");
        self.calls.fetch_add(1, Ordering::SeqCst);
        CredentialMaterial::bearer("NISAR_FIXTURE_TOKEN")
    }
}

fn item() -> SnapshotItem {
    let mut item = frozen_item(BODY, sha256_hex(BODY));
    item.execution_policy.as_mut().unwrap().policy_id = NISAR_RESOLVER_POLICY_ID_V2.to_owned();
    item.metadata["resolver_policy_id"] = json!(NISAR_RESOLVER_POLICY_ID_V2);
    item.metadata["properties"]["resolver_policy_id"] = json!(NISAR_RESOLVER_POLICY_ID_V2);
    item.metadata["properties"]["product_specification_version"] = json!("1.5.0");
    item
}

fn legacy_v1_item() -> SnapshotItem {
    let mut item = frozen_item(BODY, sha256_hex(BODY));
    item.execution_policy.as_mut().unwrap().policy_id = NISAR_RESOLVER_POLICY_ID_V1.to_owned();
    item.metadata["resolver_policy_id"] = json!(NISAR_RESOLVER_POLICY_ID_V1);
    item.metadata["properties"]["resolver_policy_id"] = json!(NISAR_RESOLVER_POLICY_ID_V1);
    item.metadata["properties"]
        .as_object_mut()
        .unwrap()
        .remove("crid");
    item.metadata["properties"]
        .as_object_mut()
        .unwrap()
        .remove("product_specification_version");
    item.metadata["properties"]
        .as_object_mut()
        .unwrap()
        .remove("product_specification_profile_registry_version");
    item
}

fn assert_not_eligible(item: &SnapshotItem, label: &str) {
    assert!(
        !matches!(
            evaluate_resolver_cutover_eligibility(
                item,
                &AsfCollectionProfileRegistry,
                &AssetExecutionPolicyRegistry,
            ),
            ResolverCutoverEligibility::Eligible(_)
        ),
        "{label}"
    );
}

#[test]
fn registers_the_exact_production_nisar_complete_hdf5_policy() {
    let registry = CatalogExecutionPolicyRegistry;
    let profile = CollectionProfileRegistry
        .get(NISAR_PROFILE_ID)
        .expect("NISAR profile");
    let frozen = item();
    let access = frozen.asset_access.as_ref().expect("frozen access");
    let input = CatalogExecutionPolicyInput {
        policy_id: NISAR_RESOLVER_POLICY_ID_V2,
        collection_profile_id: profile.profile_id,
        normalized_role: AssetRole::Primary,
        descriptor: &access.descriptor,
    };

    let definition = registry
        .validate(&input, CatalogPolicyContext::OfflineFixture)
        .expect("exact NISAR fixture policy validates");
    assert_eq!(definition.collection_profile_id, NISAR_PROFILE_ID);
    assert_eq!(profile.provider_id, NISAR_PROVIDER_ID);
    assert_eq!(profile.logical_dataset, NISAR_LOGICAL_DATASET);
    assert_eq!(profile.native_collection_id, NISAR_NATIVE_COLLECTION_ID);
    assert_eq!(profile.required_product_type, "RSLC");
    assert_eq!(definition.normalized_role, AssetRole::Primary);
    assert_eq!(definition.access_scheme, AssetAccessScheme::EarthdataHttps);
    assert!(!definition.offline_only);
    assert_eq!(
        registry
            .validate(&input, CatalogPolicyContext::Production)
            .expect("typed NISAR production policy validates"),
        definition
    );

    for locator in [
        format!("https://evil.example/{FILE_NAME}"),
        format!("https://nisar.asf.earthdatacloud.nasa.gov/NISAR/OTHER/{FILE_NAME}/{FILE_NAME}"),
    ] {
        let mut descriptor = access.descriptor.clone();
        descriptor.stable_href = Some(locator);
        let invalid = CatalogExecutionPolicyInput {
            descriptor: &descriptor,
            ..input.clone()
        };
        assert!(matches!(
            registry.validate(&invalid, CatalogPolicyContext::OfflineFixture),
            Err(CatalogExecutionPolicyError::InvalidDescriptor(_))
        ));
    }
}

#[test]
fn classifies_only_the_primary_hdf5_and_resolves_credentials_just_in_time() {
    let credentials = RecordingCredentials::default();
    let frozen = item();
    let eligible = evaluate_resolver_cutover_eligibility(
        &frozen,
        &AsfCollectionProfileRegistry,
        &AssetExecutionPolicyRegistry,
    );
    let ResolverCutoverEligibility::Eligible(eligible) = eligible else {
        panic!("exact NISAR item must be resolver-authoritative");
    };
    assert_eq!(eligible.policy.policy_id, NISAR_RESOLVER_POLICY_ID_V2);
    assert_eq!(
        eligible
            .artifact_validation_plan()
            .unwrap()
            .binding()
            .version(),
        2
    );
    assert_eq!(eligible.policy.approved_asset_roles, &[AssetRole::Primary]);
    assert_eq!(
        eligible.frozen_access.reference.asset_key,
        "science-rslc-hdf5"
    );
    assert_ne!(eligible.frozen_access.reference.asset_key, QA_ASSET_KEY);
    assert_eq!(eligible.frozen_access.file_name.as_deref(), Some(FILE_NAME));
    assert_eq!(credentials.calls.load(Ordering::SeqCst), 0);

    let resolver = EarthdataAssetResolver::official();
    let cancelled = || false;
    let resolved = resolver
        .resolve(
            &eligible.frozen_access.reference,
            &eligible.frozen_access.descriptor,
            &ResolveContext::new(&credentials, &cancelled, &FixedClock),
        )
        .expect("execution-time Earthdata resolution succeeds");
    assert_eq!(resolved.effective_url().as_str(), OFFICIAL_HREF);
    assert_eq!(credentials.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn preview_is_validated_as_ancillary_graph_evidence_without_execution_authority() {
    let mut preview = item();
    preview.metadata["assets"][1]["role"] = json!("preview");
    let ResolverCutoverEligibility::Eligible(eligible) = evaluate_resolver_cutover_eligibility(
        &preview,
        &AsfCollectionProfileRegistry,
        &AssetExecutionPolicyRegistry,
    ) else {
        panic!("a valid main Preview must remain eligible graph evidence");
    };
    assert_eq!(eligible.policy.approved_asset_roles, &[AssetRole::Primary]);
    assert_eq!(eligible.frozen_access.role, AssetRole::Primary);
    assert_eq!(
        eligible.frozen_access.reference.asset_key,
        "science-rslc-hdf5"
    );

    let mut browse_only = item();
    browse_only.metadata["assets"]
        .as_array_mut()
        .unwrap()
        .retain(|asset| asset["role"] != "thumbnail");
    assert!(matches!(
        evaluate_resolver_cutover_eligibility(
            &browse_only,
            &AsfCollectionProfileRegistry,
            &AssetExecutionPolicyRegistry,
        ),
        ResolverCutoverEligibility::Eligible(_)
    ));

    let mut thumbnail_only = item();
    thumbnail_only.metadata["assets"]
        .as_array_mut()
        .unwrap()
        .retain(|asset| asset["role"] != "browse");
    thumbnail_only.metadata["properties"]["preview_available"] = json!(false);
    assert!(matches!(
        evaluate_resolver_cutover_eligibility(
            &thumbnail_only,
            &AsfCollectionProfileRegistry,
            &AssetExecutionPolicyRegistry,
        ),
        ResolverCutoverEligibility::Eligible(_)
    ));

    let mut no_visualization = thumbnail_only.clone();
    no_visualization.metadata["assets"]
        .as_array_mut()
        .unwrap()
        .retain(|asset| asset["role"] != "thumbnail");
    assert!(matches!(
        evaluate_resolver_cutover_eligibility(
            &no_visualization,
            &AsfCollectionProfileRegistry,
            &AssetExecutionPolicyRegistry,
        ),
        ResolverCutoverEligibility::Eligible(_)
    ));

    let mut malformed_preview = preview.clone();
    malformed_preview.metadata["assets"][1]["media_type"] = json!("application/octet-stream");
    assert_not_eligible(&malformed_preview, "malformed-preview");

    let mut unknown_role = preview;
    unknown_role.metadata["assets"][1]["role"] = json!("derived");
    assert_not_eligible(&unknown_role, "unknown-role");
}

#[test]
fn runtime_policy_rejects_scope_identity_and_ancillary_execution_drift() {
    type ItemMutation = (&'static str, fn(&mut SnapshotItem));

    let mutations: Vec<ItemMutation> = vec![
        ("policy", |item| {
            item.execution_policy.as_mut().unwrap().policy_id =
                "asf.nisar.unreviewed.v1".to_owned();
        }),
        ("profile", |item| {
            item.execution_policy
                .as_mut()
                .unwrap()
                .collection_profile_id = Some("asf.alos-2.palsar-2.l1.1.v1".to_owned());
        }),
        ("dataset", |item| {
            item.metadata["dataset"] = json!("NISAR_BETA")
        }),
        ("quality", |item| {
            item.metadata["quality_status"] = json!("Beta");
            item.metadata["properties"]["quality_status"] = json!("Beta");
        }),
        ("missing-crid", |item| {
            item.metadata["properties"]
                .as_object_mut()
                .unwrap()
                .remove("crid");
        }),
        ("unknown-crid", |item| {
            item.metadata["properties"]["crid"] = json!("P99999")
        }),
        ("missing-specification", |item| {
            item.metadata["properties"]
                .as_object_mut()
                .unwrap()
                .remove("product_specification_version");
        }),
        ("crid-specification-mismatch", |item| {
            item.metadata["properties"]["product_specification_version"] = json!("1.2.1")
        }),
        ("missing-profile-registry-version", |item| {
            item.metadata["properties"]
                .as_object_mut()
                .unwrap()
                .remove("product_specification_profile_registry_version");
        }),
        ("profile-registry-version-mismatch", |item| {
            item.metadata["properties"]["product_specification_profile_registry_version"] = json!(2)
        }),
        ("configuration", |item| {
            item.metadata["properties"]["production_configuration"] = json!("UR")
        }),
        ("band", |item| {
            item.metadata["properties"]["band"] = json!("S")
        }),
        ("frequency-less", |item| {
            item.metadata["properties"]
                .as_object_mut()
                .unwrap()
                .remove("frequency_a_polarization");
        }),
        ("null-frequency", |item| {
            item.metadata["properties"]["frequency_a_polarization"] = Value::Null;
        }),
        ("ancillary-role", |item| {
            let access = item.asset_access.as_mut().unwrap();
            access.role = AssetRole::Auxiliary;
            access.reference.asset_key = QA_ASSET_KEY.to_owned();
        }),
        ("filename", |item| {
            item.asset_access.as_mut().unwrap().file_name = Some("QA_STATS.h5".to_owned());
        }),
        ("size", |item| {
            item.asset_access.as_mut().unwrap().expected_size_bytes = Some(1);
        }),
        ("checksum", |item| {
            item.asset_access.as_mut().unwrap().checksum_value = Some("0".repeat(64));
        }),
        ("primary-graph", |item| {
            item.metadata["assets"][0]["role"] = json!("auxiliary")
        }),
        ("missing-archive-role", |item| {
            item.metadata["assets"][0]["native_roles"] =
                json!(["GET DATA", "GET DATA VIA DIRECT ACCESS"]);
        }),
        ("malformed-optional-checksum", |item| {
            item.metadata["assets"][1]["checksum"] =
                json!({"algorithm": "SHA-256", "value": "bad"});
        }),
        ("unreviewed-primary-host", |item| {
            let locator = format!("https://evil.example/{FILE_NAME}");
            item.asset_access.as_mut().unwrap().descriptor.stable_href = Some(locator.clone());
            item.metadata["assets"][0]["routes"][0]["locator"] = Value::String(locator);
        }),
        ("wrong-primary-path", |item| {
            let locator = format!(
                "https://nisar.asf.earthdatacloud.nasa.gov/NISAR/OTHER/{FILE_NAME}/{FILE_NAME}"
            );
            item.asset_access.as_mut().unwrap().descriptor.stable_href = Some(locator.clone());
            item.metadata["assets"][0]["routes"][0]["locator"] = Value::String(locator);
        }),
        ("signed-locator", |item| {
            let locator = format!("{OFFICIAL_HREF}?token=fixture");
            item.asset_access.as_mut().unwrap().descriptor.stable_href = Some(locator.clone());
            item.metadata["assets"][0]["routes"][0]["locator"] = Value::String(locator);
        }),
        ("s-band-evidence", |item| {
            item.metadata["assets"][3]["native_evidence"]["url_types"] = json!(["S-BAND PRODUCT"]);
        }),
    ];

    for (label, mutate) in mutations {
        let mut frozen = item();
        mutate(&mut frozen);
        assert_not_eligible(&frozen, label);
    }
}

#[test]
fn legacy_v1_frozen_tasks_bypass_the_new_crid_specification_profile_without_upgrade() {
    let legacy = legacy_v1_item();
    let ResolverCutoverEligibility::Eligible(eligible) = evaluate_resolver_cutover_eligibility(
        &legacy,
        &AsfCollectionProfileRegistry,
        &AssetExecutionPolicyRegistry,
    ) else {
        panic!("an already-frozen @1 task must retain its historical behavior");
    };
    assert_eq!(eligible.policy.policy_id, NISAR_RESOLVER_POLICY_ID_V1);
    assert_eq!(
        eligible
            .artifact_validation_plan()
            .unwrap()
            .binding()
            .version(),
        1
    );
}

#[test]
fn earthdata_rejects_untrusted_or_cancelled_nisar_access_before_credentials() {
    let resolver = EarthdataAssetResolver::official();
    let credentials = RecordingCredentials::default();
    let frozen = item();
    let access = frozen.asset_access.as_ref().unwrap();
    let not_cancelled = || false;

    for locator in [
        format!("https://nisar.asf.earthdatacloud.nasa.gov.evil.example/{FILE_NAME}"),
        format!("https://user@nisar.asf.earthdatacloud.nasa.gov/{FILE_NAME}"),
        format!("{OFFICIAL_HREF}?token=fixture"),
    ] {
        let mut invalid = access.descriptor.clone();
        invalid.stable_href = Some(locator);
        assert!(resolver
            .resolve(
                &access.reference,
                &invalid,
                &ResolveContext::new(&credentials, &not_cancelled, &FixedClock),
            )
            .is_err());
        assert_eq!(credentials.calls.load(Ordering::SeqCst), 0);
    }

    let cancelled = || true;
    assert!(resolver
        .resolve(
            &access.reference,
            &access.descriptor,
            &ResolveContext::new(&credentials, &cancelled, &FixedClock),
        )
        .is_err());
    assert_eq!(credentials.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn invalid_graph_reports_a_closed_policy_result_not_legacy_compatibility() {
    let mut frozen = item();
    frozen.metadata["assets"][0]["stable_asset_key"] = json!("mismatched");
    assert!(matches!(
        evaluate_resolver_cutover_eligibility(
            &frozen,
            &AsfCollectionProfileRegistry,
            &AssetExecutionPolicyRegistry,
        ),
        ResolverCutoverEligibility::Invalid(_)
    ));
}
