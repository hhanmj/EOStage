use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::SystemTime;

use insar_core::{
    provider_product_key, AssetAccessScheme, AssetExecutionAuthority, AssetExecutionPolicy,
    AssetRole, FrozenAssetAccess, SnapshotItem, StableAssetReference,
};
use insar_download::{
    evaluate_resolver_cutover_eligibility, AsfCollectionProfileRegistry,
    AssetExecutionPolicyRegistry, AssetResolver, CatalogExecutionPolicyError,
    CatalogExecutionPolicyInput, CatalogExecutionPolicyRegistry, CatalogPolicyContext, Clock,
    CredentialMaterial, CredentialProvider, CredentialProviderError, EarthdataAssetResolver,
    EligibilityReason, ResolveContext, ResolverCutoverEligibility,
};
use insar_provider::{
    asf::{map_alos2_umm_granule, CmrGranuleEnvelope},
    catalog::{
        CollectionProfileRegistry, ALOS2_LOGICAL_DATASET, ALOS2_NATIVE_COLLECTION_ID,
        ALOS2_PROFILE_ID, ALOS2_PROVIDER_ID, ALOS2_RESOLVER_POLICY_ID,
    },
};
use serde_json::{json, Map, Value};

const FILE_NAME: &str = "ALOS2478123150-230331-WBDR1.1__D.zip";
const OFFICIAL_HREF: &str =
    "https://datapool.asf.alaska.edu/ALOS2/ALOS2478123150-230331-WBDR1.1__D.zip";

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
        CredentialMaterial::bearer("ALOS2_FIXTURE_TOKEN")
    }
}

fn raw_fixture() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("docs/fixtures/asf-alos2-p1/granule-wbd-cmr-umm.json");
    serde_json::from_str(&fs::read_to_string(path).expect("raw ALOS-2 fixture exists"))
        .expect("raw ALOS-2 fixture is valid JSON")
}

fn item() -> SnapshotItem {
    let envelope = CmrGranuleEnvelope::try_from(&raw_fixture()).expect("raw CMR UMM normalizes");
    let profile = CollectionProfileRegistry
        .get(ALOS2_PROFILE_ID)
        .expect("ALOS-2 profile is registered");
    let mut product =
        map_alos2_umm_granule(&envelope, profile).expect("raw CMR UMM maps to ALOS-2");
    let asset = &mut product.assets[0];
    asset
        .access_descriptor
        .as_mut()
        .expect("primary descriptor")
        .stable_href = Some(OFFICIAL_HREF.to_owned());
    let asset_key = asset.asset_key.as_deref().expect("asset key").to_owned();
    let stable_asset_key = asset
        .stable_asset_key
        .as_deref()
        .expect("stable asset key")
        .to_owned();
    let descriptor = asset
        .access_descriptor
        .as_ref()
        .expect("primary descriptor")
        .clone();
    let size_bytes = asset
        .size_bytes
        .and_then(|value| u64::try_from(value).ok())
        .expect("positive asset size");
    product.properties["asset_routes"][&asset_key] = json!([
        {"scheme":"https", "locator":OFFICIAL_HREF},
        {"scheme":"s3", "locator":format!("s3://example-invalid/alos2/{FILE_NAME}")}
    ]);
    let integrity = product.properties["asset_integrity"][&asset_key]
        .as_object()
        .expect("asset integrity");
    let checksum_algorithm = integrity["algorithm"].as_str().unwrap().to_owned();
    let checksum_value = integrity["value"].as_str().unwrap().to_owned();
    let product_key = provider_product_key(
        ALOS2_PROVIDER_ID,
        ALOS2_NATIVE_COLLECTION_ID,
        &product.product_id,
    )
    .expect("provider product key");
    let access = FrozenAssetAccess {
        reference: StableAssetReference {
            provider_id: ALOS2_PROVIDER_ID.to_owned(),
            collection_id: ALOS2_NATIVE_COLLECTION_ID.to_owned(),
            provider_product_id: product.product_id.clone(),
            provider_product_key: product_key.clone(),
            canonical_product_key: None,
            asset_key,
            stable_asset_key,
        },
        descriptor,
        role: AssetRole::Primary,
        file_name: Some(FILE_NAME.to_owned()),
        expected_size_bytes: Some(size_bytes),
        checksum_algorithm: Some(checksum_algorithm),
        checksum_value: Some(checksum_value),
    };
    let metadata = Map::from_iter([
        (
            "dataset".to_owned(),
            Value::String(ALOS2_LOGICAL_DATASET.to_owned()),
        ),
        (
            "logical_dataset".to_owned(),
            Value::String(ALOS2_LOGICAL_DATASET.to_owned()),
        ),
        (
            "native_collection_id".to_owned(),
            Value::String(ALOS2_NATIVE_COLLECTION_ID.to_owned()),
        ),
        (
            "collection_profile_id".to_owned(),
            Value::String(ALOS2_PROFILE_ID.to_owned()),
        ),
        (
            "identity_rule_id".to_owned(),
            Value::String(profile.identity_rule_id.to_owned()),
        ),
        (
            "filename_rule_id".to_owned(),
            Value::String(profile.filename_rule_id.to_owned()),
        ),
        (
            "delivery_model".to_owned(),
            Value::String("archive_first".to_owned()),
        ),
        (
            "resolver_policy_id".to_owned(),
            Value::String(ALOS2_RESOLVER_POLICY_ID.to_owned()),
        ),
        (
            "provider_product_id".to_owned(),
            Value::String(product.product_id.clone()),
        ),
        (
            "provider_product_key".to_owned(),
            Value::String(product_key.clone()),
        ),
        (
            "product_family".to_owned(),
            Value::String("ALOS2_L1_PSR2".to_owned()),
        ),
        (
            "processing_level".to_owned(),
            Value::String("1.1".to_owned()),
        ),
        ("source_kind".to_owned(), Value::String("sar".to_owned())),
        (
            "product_kind".to_owned(),
            Value::String("sar_product".to_owned()),
        ),
        ("properties".to_owned(), Value::Object(product.properties)),
        (
            "assets".to_owned(),
            serde_json::to_value(product.assets).expect("provider assets serialize"),
        ),
    ]);
    SnapshotItem {
        item_id: product_key,
        provider_id: ALOS2_PROVIDER_ID.to_owned(),
        display_name: "ALOS2478123150-230331-WBDR1.1__D".to_owned(),
        acquisition_datetime: product.acquisition_datetime,
        footprint_geojson: product.footprint_geojson,
        metadata,
        payload: Map::new(),
        asset_access: Some(access),
        execution_policy: Some(AssetExecutionPolicy {
            authority: AssetExecutionAuthority::ResolverAuthoritative,
            policy_id: ALOS2_RESOLVER_POLICY_ID.to_owned(),
            collection_profile_id: Some(ALOS2_PROFILE_ID.to_owned()),
        }),
    }
}

#[test]
fn registers_the_exact_production_alos2_catalogue_execution_policy() {
    let registry = CatalogExecutionPolicyRegistry;
    let profile = CollectionProfileRegistry
        .get(ALOS2_PROFILE_ID)
        .expect("ALOS-2 profile");
    let frozen = item();
    let access = frozen.asset_access.as_ref().expect("frozen access");
    let input = CatalogExecutionPolicyInput {
        policy_id: ALOS2_RESOLVER_POLICY_ID,
        collection_profile_id: profile.profile_id,
        normalized_role: AssetRole::Primary,
        descriptor: &access.descriptor,
    };

    let definition = registry
        .validate(&input, CatalogPolicyContext::OfflineFixture)
        .expect("exact ALOS-2 fixture policy validates");
    assert_eq!(definition.collection_profile_id, ALOS2_PROFILE_ID);
    assert_eq!(profile.provider_id, ALOS2_PROVIDER_ID);
    assert_eq!(profile.native_collection_id, ALOS2_NATIVE_COLLECTION_ID);
    assert_eq!(profile.required_product_type, "ALOS2_L1_PSR2");
    assert_eq!(definition.access_scheme, AssetAccessScheme::EarthdataHttps);
    assert!(!definition.offline_only);
    assert_eq!(
        registry
            .validate(&input, CatalogPolicyContext::Production)
            .expect("typed ALOS-2 production policy validates"),
        definition
    );

    let historical_input = CatalogExecutionPolicyInput {
        collection_profile_id: "asf.alos-palsar.v2",
        ..input
    };
    assert_eq!(
        registry
            .validate(&historical_input, CatalogPolicyContext::OfflineFixture)
            .unwrap_err(),
        CatalogExecutionPolicyError::ProfileMismatch
    );
}

#[test]
fn classifies_only_the_exact_alos2_frozen_archive_without_credentials() {
    let credentials = RecordingCredentials::default();
    let frozen = item();
    let eligible = evaluate_resolver_cutover_eligibility(
        &frozen,
        &AsfCollectionProfileRegistry,
        &AssetExecutionPolicyRegistry,
    );
    let ResolverCutoverEligibility::Eligible(eligible) = eligible else {
        panic!("exact ALOS-2 item must be resolver-authoritative");
    };
    assert_eq!(eligible.policy.policy_id, ALOS2_RESOLVER_POLICY_ID);
    assert_eq!(
        eligible.frozen_access,
        *frozen.asset_access.as_ref().unwrap()
    );
    assert_eq!(credentials.calls.load(Ordering::SeqCst), 0);

    let resolver = EarthdataAssetResolver::official();
    let cancelled = || false;
    resolver
        .resolve(
            &eligible.frozen_access.reference,
            &eligible.frozen_access.descriptor,
            &ResolveContext::new(&credentials, &cancelled, &FixedClock),
        )
        .expect("execution-time Earthdata resolution succeeds");
    assert_eq!(credentials.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn alos2_runtime_policy_fails_closed_for_historical_and_access_mutations() {
    let mut wrong_policy = item();
    wrong_policy.execution_policy.as_mut().unwrap().policy_id =
        "asf.alos-2.unreviewed.v1".to_owned();
    assert!(matches!(
        evaluate_resolver_cutover_eligibility(
            &wrong_policy,
            &AsfCollectionProfileRegistry,
            &AssetExecutionPolicyRegistry,
        ),
        ResolverCutoverEligibility::Invalid(_)
    ));

    let mut historical = item();
    historical
        .execution_policy
        .as_mut()
        .unwrap()
        .collection_profile_id = Some("asf.alos-palsar.v2".to_owned());
    assert!(matches!(
        evaluate_resolver_cutover_eligibility(
            &historical,
            &AsfCollectionProfileRegistry,
            &AssetExecutionPolicyRegistry,
        ),
        ResolverCutoverEligibility::Invalid(_)
    ));

    let mut wrong_dataset = item();
    wrong_dataset.metadata["dataset"] = Value::String("ALOS PALSAR".to_owned());
    assert!(matches!(
        evaluate_resolver_cutover_eligibility(
            &wrong_dataset,
            &AsfCollectionProfileRegistry,
            &AssetExecutionPolicyRegistry,
        ),
        ResolverCutoverEligibility::Ineligible(EligibilityReason::Dataset)
    ));

    let mut wrong_scheme = item();
    wrong_scheme
        .asset_access
        .as_mut()
        .unwrap()
        .descriptor
        .scheme = AssetAccessScheme::PublicHttps;
    assert!(matches!(
        evaluate_resolver_cutover_eligibility(
            &wrong_scheme,
            &AsfCollectionProfileRegistry,
            &AssetExecutionPolicyRegistry,
        ),
        ResolverCutoverEligibility::Invalid(_)
    ));

    let mut wrong_role = item();
    wrong_role.asset_access.as_mut().unwrap().role = AssetRole::Browse;
    assert!(matches!(
        evaluate_resolver_cutover_eligibility(
            &wrong_role,
            &AsfCollectionProfileRegistry,
            &AssetExecutionPolicyRegistry,
        ),
        ResolverCutoverEligibility::Ineligible(EligibilityReason::AssetRole)
    ));

    for suffix in ["?token=fixture", "#fragment"] {
        let mut non_durable = item();
        let locator = format!("{OFFICIAL_HREF}{suffix}");
        non_durable
            .asset_access
            .as_mut()
            .unwrap()
            .descriptor
            .stable_href = Some(locator.clone());
        non_durable.metadata["assets"][0]["access_descriptor"]["stable_href"] =
            Value::String(locator.clone());
        non_durable.metadata["properties"]["asset_routes"][FILE_NAME][0]["locator"] =
            Value::String(locator);
        assert_not_eligible(&non_durable);
    }
}

#[test]
fn earthdata_rejects_invalid_or_cancelled_access_before_credentials() {
    let resolver = EarthdataAssetResolver::official();
    let credentials = RecordingCredentials::default();
    let frozen = item();
    let access = frozen.asset_access.as_ref().unwrap();
    let mut invalid = access.descriptor.clone();
    invalid.stable_href = Some("https://asf.alaska.edu.evil.example/archive.zip".to_owned());
    let not_cancelled = || false;
    assert!(resolver
        .resolve(
            &access.reference,
            &invalid,
            &ResolveContext::new(&credentials, &not_cancelled, &FixedClock),
        )
        .is_err());
    assert_eq!(credentials.calls.load(Ordering::SeqCst), 0);

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

fn assert_not_eligible(item: &SnapshotItem) {
    assert!(!matches!(
        evaluate_resolver_cutover_eligibility(
            item,
            &AsfCollectionProfileRegistry,
            &AssetExecutionPolicyRegistry,
        ),
        ResolverCutoverEligibility::Eligible(_)
    ));
}

#[test]
fn alos2_runtime_policy_revalidates_the_frozen_asset_graph_and_integrity() {
    let mut changed_size = item();
    changed_size
        .asset_access
        .as_mut()
        .unwrap()
        .expected_size_bytes = Some(60_018_405_389);
    assert_not_eligible(&changed_size);

    let mut changed_checksum = item();
    changed_checksum
        .asset_access
        .as_mut()
        .unwrap()
        .checksum_value = Some("ffffffffffffffffffffffffffffffff".to_owned());
    assert_not_eligible(&changed_checksum);

    let mut changed_property_identity = item();
    changed_property_identity.metadata["properties"]["provider_product_key"] =
        Value::String("historical-alos-key".to_owned());
    assert_not_eligible(&changed_property_identity);

    let mut changed_asset = item();
    changed_asset.metadata["assets"][0]["file_name"] =
        Value::String("different-authoritative.zip".to_owned());
    assert_not_eligible(&changed_asset);

    let mut malformed_provenance = item();
    malformed_provenance.metadata["properties"]["asset_routes"][FILE_NAME][1]["locator"] =
        Value::String(format!("not-a-url/{FILE_NAME}"));
    assert_not_eligible(&malformed_provenance);

    for (key, value) in [
        ("processing_level", json!("2.1")),
        ("beam_mode", json!("FBS")),
        ("polarizations", json!(["VV"])),
    ] {
        let mut changed_scope = item();
        changed_scope.metadata["properties"][key] = value;
        assert_not_eligible(&changed_scope);
    }

    let mut changed_top_level_processing = item();
    changed_top_level_processing.metadata["processing_level"] = json!("2.1");
    assert_not_eligible(&changed_top_level_processing);

    let mut changed_top_level_family = item();
    changed_top_level_family.metadata["product_family"] = json!("ALOS_PALSAR");
    assert_not_eligible(&changed_top_level_family);
}
