use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::SystemTime;

use insar_core::{
    provider_product_key, stable_asset_key, AssetAccessDescriptor, AssetAccessScheme,
    AssetExecutionAuthority, AssetExecutionPolicy, AssetResolverParameters, AssetRole,
    FrozenAssetAccess, ProviderNetworkPolicy, SnapshotItem, StableAssetReference,
};
use insar_download::{
    classify_frozen_execution, AsfCollectionProfileRegistry, AssetExecutionPolicyRegistry,
    AssetResolverRegistry, CdseAuthError, CdseLoginMaterial, CdseOAuthSession, CdseTokenEndpoint,
    CdseTokenGrant, Clock, CredentialMaterial, CredentialProvider, CredentialProviderError,
    FrozenExecution, ProductionCredentialProvider, ResolveContext, ResolverCutoverEligibility,
    ResolverExecutionRuntime, CDSE_SENTINEL2_L2A_POLICY_ID, CDSE_SENTINEL2_L2A_PROFILE_ID,
    EARTHDATA_CREDENTIAL_PROFILE,
};
use insar_provider::usgs_m2m::USGS_M2M_SESSION_PROFILE_ID;
use serde_json::{json, Map, Value};

const CDSE_CREDENTIAL_PROFILE: &str = "copernicus-dataspace-default";
const CDSE_RESOLVER_ID: &str = "cdse.archive.resolver.v1";
const CDSE_PROVIDER_ID: &str = "copernicus.dataspace.odata";
const CDSE_COLLECTION_ID: &str = "SENTINEL-2";
const PRODUCT_ID: &str = "00000000-0000-4000-8000-000000000001";
const CDSE_ACCESS_FIXTURE: &str = "CDSE_TEST_ACCESS_TOKEN";
const EARTHDATA_ACCESS_FIXTURE: &str = "EARTHDATA_TEST_ACCESS_TOKEN";

struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> SystemTime {
        SystemTime::UNIX_EPOCH
    }
}

struct FixtureTokenEndpoint {
    policy: ProviderNetworkPolicy,
}

impl CdseTokenEndpoint for FixtureTokenEndpoint {
    fn network_policy(&self) -> ProviderNetworkPolicy {
        self.policy
    }

    fn password_grant(
        &self,
        _material: &CdseLoginMaterial,
    ) -> Result<CdseTokenGrant, CdseAuthError> {
        CdseTokenGrant::new(CDSE_ACCESS_FIXTURE, None, None)
    }

    fn refresh_grant(&self, _refresh_token: &str) -> Result<CdseTokenGrant, CdseAuthError> {
        Err(CdseAuthError::RefreshUnavailable)
    }
}

struct RecordingEarthdataProvider {
    calls: AtomicUsize,
}

impl RecordingEarthdataProvider {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }
}

impl CredentialProvider for RecordingEarthdataProvider {
    fn resolve(&self, profile: &str) -> Result<CredentialMaterial, CredentialProviderError> {
        assert_eq!(profile, EARTHDATA_CREDENTIAL_PROFILE);
        self.calls.fetch_add(1, Ordering::SeqCst);
        CredentialMaterial::bearer(EARTHDATA_ACCESS_FIXTURE)
    }
}

fn session(policy: ProviderNetworkPolicy) -> Arc<CdseOAuthSession> {
    Arc::new(
        CdseOAuthSession::new(
            policy,
            Arc::new(FixtureTokenEndpoint { policy }),
            Arc::new(FixedClock),
        )
        .expect("matching fixture OAuth policy"),
    )
}

fn frozen_access() -> FrozenAssetAccess {
    let provider_product_key =
        provider_product_key(CDSE_PROVIDER_ID, CDSE_COLLECTION_ID, PRODUCT_ID)
            .expect("product key");
    let stable_asset_key = stable_asset_key(&provider_product_key, "archive").expect("asset key");
    FrozenAssetAccess {
        reference: StableAssetReference {
            provider_id: CDSE_PROVIDER_ID.to_owned(),
            collection_id: CDSE_COLLECTION_ID.to_owned(),
            provider_product_id: PRODUCT_ID.to_owned(),
            provider_product_key,
            canonical_product_key: None,
            asset_key: "archive".to_owned(),
            stable_asset_key,
        },
        descriptor: AssetAccessDescriptor {
            resolver_id: CDSE_RESOLVER_ID.to_owned(),
            scheme: AssetAccessScheme::OauthBearerHttps,
            stable_href: Some(format!(
                "https://download.dataspace.copernicus.eu/odata/v1/Products({PRODUCT_ID})/$value"
            )),
            requires_credentials: true,
            credential_profile: Some(CDSE_CREDENTIAL_PROFILE.to_owned()),
            resolver_parameters: AssetResolverParameters {},
        },
        role: AssetRole::Primary,
        file_name: Some("S2_FIXTURE_ONE.SAFE.zip".to_owned()),
        expected_size_bytes: Some(2048),
        checksum_algorithm: Some("MD5".to_owned()),
        checksum_value: Some("00000000000000000000000000000000".to_owned()),
    }
}

fn item() -> SnapshotItem {
    let access = frozen_access();
    let mut metadata = Map::from_iter([
        (
            "logical_dataset".to_owned(),
            Value::String("optical.sentinel-2.l2a".to_owned()),
        ),
        (
            "native_collection_id".to_owned(),
            Value::String(CDSE_COLLECTION_ID.to_owned()),
        ),
        (
            "collection_profile_id".to_owned(),
            Value::String(CDSE_SENTINEL2_L2A_PROFILE_ID.to_owned()),
        ),
        (
            "identity_rule_id".to_owned(),
            Value::String("cdse.sentinel-2.l2a.identity.v1".to_owned()),
        ),
        (
            "delivery_model".to_owned(),
            Value::String("archive_first".to_owned()),
        ),
        (
            "provider_product_id".to_owned(),
            Value::String(PRODUCT_ID.to_owned()),
        ),
        (
            "provider_product_key".to_owned(),
            Value::String(access.reference.provider_product_key.clone()),
        ),
        (
            "properties".to_owned(),
            json!({"product_type": "S2MSI2A", "online": true}),
        ),
    ]);
    metadata.insert("canonical_product_key".to_owned(), Value::Null);
    SnapshotItem {
        item_id: access.reference.provider_product_key.clone(),
        provider_id: CDSE_PROVIDER_ID.to_owned(),
        display_name: "S2_FIXTURE_ONE.SAFE".to_owned(),
        acquisition_datetime: None,
        footprint_geojson: None,
        metadata,
        payload: Map::new(),
        asset_access: Some(access),
        execution_policy: Some(AssetExecutionPolicy {
            authority: AssetExecutionAuthority::ResolverAuthoritative,
            policy_id: CDSE_SENTINEL2_L2A_POLICY_ID.to_owned(),
            collection_profile_id: Some(CDSE_SENTINEL2_L2A_PROFILE_ID.to_owned()),
        }),
    }
}

#[test]
fn exact_sentinel2_cdse_archive_policy_is_eligible_and_rejects_offline_products() {
    let item = item();
    let classified = classify_frozen_execution(
        &item,
        &AsfCollectionProfileRegistry,
        &AssetExecutionPolicyRegistry,
    );
    let FrozenExecution::ResolverAuthoritative(eligible) = classified else {
        panic!("exact CDSE archive must be resolver-authoritative");
    };
    assert_eq!(eligible.policy.policy_id, CDSE_SENTINEL2_L2A_POLICY_ID);
    assert_eq!(
        eligible.policy.auth_refresh_policy_id,
        insar_download::AuthRefreshPolicyId::CdseOAuthBearerV1
    );

    let mut offline = item;
    offline
        .metadata
        .get_mut("properties")
        .and_then(Value::as_object_mut)
        .expect("fixture properties")
        .insert("online".to_owned(), Value::Bool(false));
    assert!(matches!(
        insar_download::evaluate_resolver_cutover_eligibility(
            &offline,
            &AsfCollectionProfileRegistry,
            &AssetExecutionPolicyRegistry,
        ),
        ResolverCutoverEligibility::Ineligible(insar_download::EligibilityReason::Online)
    ));
}

#[test]
fn logged_out_cdse_session_is_typed_and_never_queries_earthdata_for_either_route() {
    for policy in [
        ProviderNetworkPolicy::system(),
        ProviderNetworkPolicy::direct(),
    ] {
        let earthdata = Arc::new(RecordingEarthdataProvider::new());
        let cdse_session = session(policy);
        let credentials =
            ProductionCredentialProvider::new(earthdata.clone(), cdse_session.clone());
        let cancelled = || false;
        let clock = FixedClock;
        let context =
            ResolveContext::new(&credentials, &cancelled, &clock).with_proxy_policy(policy.route());
        let error = AssetResolverRegistry::with_standard_resolvers()
            .expect("standard resolver registry")
            .resolve_frozen(&frozen_access(), &context)
            .expect_err("logged-out CDSE resolver must fail closed");

        assert_eq!(error.code(), "ASSET_CREDENTIALS_MISSING");
        assert_eq!(earthdata.calls.load(Ordering::SeqCst), 0);
        let runtime = ResolverExecutionRuntime::production_with_cdse_session(cdse_session, policy)
            .expect("runtime with matching shared session");
        assert_eq!(runtime.network_policy(), policy);
        assert!(matches!(
            runtime.classify(&item()),
            FrozenExecution::ResolverAuthoritative(_)
        ));
    }
}

#[test]
fn production_credential_router_keeps_earthdata_and_cdse_material_separate() {
    for policy in [
        ProviderNetworkPolicy::system(),
        ProviderNetworkPolicy::direct(),
    ] {
        let earthdata = Arc::new(RecordingEarthdataProvider::new());
        let cdse_session = session(policy);
        let credentials =
            ProductionCredentialProvider::new(earthdata.clone(), cdse_session.clone());

        assert!(credentials.resolve(EARTHDATA_CREDENTIAL_PROFILE).is_ok());
        assert_eq!(earthdata.calls.load(Ordering::SeqCst), 1);
        assert!(matches!(
            credentials.resolve(CDSE_CREDENTIAL_PROFILE),
            Err(CredentialProviderError::Missing)
        ));
        assert_eq!(earthdata.calls.load(Ordering::SeqCst), 1);

        cdse_session
            .login(
                CdseLoginMaterial::new("fixture-user", "fixture-password", None)
                    .expect("fixture login material"),
            )
            .expect("offline fixture login");
        let cdse_material = credentials
            .resolve(CDSE_CREDENTIAL_PROFILE)
            .expect("shared session supplies CDSE bearer");
        assert!(format!("{cdse_material:?}").contains("bearer"));
        assert_eq!(earthdata.calls.load(Ordering::SeqCst), 1);
        assert!(matches!(
            credentials.resolve("unapproved-profile"),
            Err(CredentialProviderError::Missing)
        ));
        assert_eq!(earthdata.calls.load(Ordering::SeqCst), 1);
        assert!(matches!(
            credentials.resolve(USGS_M2M_SESSION_PROFILE_ID),
            Err(CredentialProviderError::Missing)
        ));
        assert_eq!(earthdata.calls.load(Ordering::SeqCst), 1);

        let debug = format!("{credentials:?} {cdse_session:?}");
        assert!(!debug.contains(CDSE_ACCESS_FIXTURE));
        assert!(!debug.contains(EARTHDATA_ACCESS_FIXTURE));
    }
}
