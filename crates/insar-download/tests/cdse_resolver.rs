use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::SystemTime;

use insar_core::{
    provider_product_key, stable_asset_key, AssetAccessDescriptor, AssetAccessScheme,
    AssetResolverParameters, AssetRole, FrozenAssetAccess, StableAssetReference,
};
use insar_download::{
    AssetResolver, AssetResolverRegistry, CdseArchiveAssetResolver, Clock, CredentialMaterial,
    CredentialProvider, CredentialProviderError, ResolveContext, ResolvedMethod,
};

const PRODUCT_ID: &str = "00000000-0000-4000-8000-000000000001";
const SYNTHETIC_BEARER: &str = "SYNTHETIC_CDSE_BEARER_TOKEN";

struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> SystemTime {
        SystemTime::UNIX_EPOCH
    }
}

enum CredentialOutcome {
    Bearer,
    Basic,
    Cookie,
    Error(CredentialProviderError),
}

struct FixtureCredentials<'a> {
    outcome: CredentialOutcome,
    calls: AtomicUsize,
    cancel_after_lookup: Option<&'a AtomicBool>,
}

impl FixtureCredentials<'_> {
    fn bearer() -> Self {
        Self {
            outcome: CredentialOutcome::Bearer,
            calls: AtomicUsize::new(0),
            cancel_after_lookup: None,
        }
    }
}

impl CredentialProvider for FixtureCredentials<'_> {
    fn resolve(&self, profile: &str) -> Result<CredentialMaterial, CredentialProviderError> {
        assert_eq!(profile, "copernicus-dataspace-default");
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(cancelled) = self.cancel_after_lookup {
            cancelled.store(true, Ordering::SeqCst);
        }
        match self.outcome {
            CredentialOutcome::Bearer => CredentialMaterial::bearer(SYNTHETIC_BEARER),
            CredentialOutcome::Basic => CredentialMaterial::basic("fixture-user", "fixture-pass"),
            CredentialOutcome::Cookie => CredentialMaterial::cookie("fixture", "fixture-cookie"),
            CredentialOutcome::Error(error) => Err(error),
        }
    }
}

fn reference(product_id: &str) -> StableAssetReference {
    let provider_product_key =
        provider_product_key("copernicus.dataspace.odata", "SENTINEL-2", product_id)
            .expect("fixture product key");
    let stable_asset_key =
        stable_asset_key(&provider_product_key, "archive").expect("fixture asset key");
    StableAssetReference {
        provider_id: "copernicus.dataspace.odata".to_owned(),
        collection_id: "SENTINEL-2".to_owned(),
        provider_product_id: product_id.to_owned(),
        provider_product_key,
        canonical_product_key: None,
        asset_key: "archive".to_owned(),
        stable_asset_key,
    }
}

fn stable_href(product_id: &str) -> String {
    format!("https://download.dataspace.copernicus.eu/odata/v1/Products({product_id})/$value")
}

fn descriptor(product_id: &str) -> AssetAccessDescriptor {
    AssetAccessDescriptor {
        resolver_id: "cdse.archive.resolver.v1".to_owned(),
        scheme: AssetAccessScheme::OauthBearerHttps,
        stable_href: Some(stable_href(product_id)),
        requires_credentials: true,
        credential_profile: Some("copernicus-dataspace-default".to_owned()),
        resolver_parameters: AssetResolverParameters {},
    }
}

fn frozen() -> FrozenAssetAccess {
    FrozenAssetAccess {
        reference: reference(PRODUCT_ID),
        descriptor: descriptor(PRODUCT_ID),
        role: AssetRole::Primary,
        file_name: Some("S2_FIXTURE_ONE.SAFE.zip".to_owned()),
        expected_size_bytes: Some(2048),
        checksum_algorithm: Some("MD5".to_owned()),
        checksum_value: Some("00000000000000000000000000000000".to_owned()),
    }
}

fn context<'a>(
    credentials: &'a FixtureCredentials<'_>,
    cancelled: &'a (dyn Fn() -> bool + Sync),
    clock: &'a FixedClock,
) -> ResolveContext<'a> {
    ResolveContext::new(credentials, cancelled, clock)
}

#[test]
fn valid_cdse_archive_resolution_keeps_identity_and_secret_runtime_only() {
    let resolver = CdseArchiveAssetResolver::new();
    let credentials = FixtureCredentials::bearer();
    let cancelled = || false;
    let clock = FixedClock;
    let direct = resolver
        .resolve(
            &frozen().reference,
            &frozen().descriptor,
            &context(&credentials, &cancelled, &clock),
        )
        .expect("resolver supplies runtime access but no frozen evidence");
    assert_eq!(direct.suggested_filename(), None);
    assert_eq!(direct.expected_size(), None);
    assert_eq!(direct.expected_checksum(), None);
    assert_eq!(direct.asset_role(), None);

    let mut registry = AssetResolverRegistry::new();
    registry
        .register(std::sync::Arc::new(CdseArchiveAssetResolver::new()))
        .expect("explicit CDSE test registration");
    let resolved = registry
        .resolve_frozen(&frozen(), &context(&credentials, &cancelled, &clock))
        .expect("valid frozen CDSE archive resolves without a request");

    assert_eq!(resolved.effective_url().as_str(), stable_href(PRODUCT_ID));
    assert_eq!(resolved.method(), ResolvedMethod::Get);
    assert_eq!(
        resolved.suggested_filename(),
        Some("S2_FIXTURE_ONE.SAFE.zip")
    );
    assert_eq!(resolved.expected_size(), Some(2048));
    assert_eq!(resolved.expected_checksum().unwrap().algorithm, "MD5");
    assert_eq!(resolved.asset_role(), Some(&AssetRole::Primary));
    let debug = format!("{resolved:?}");
    assert!(debug.contains("Authorization"));
    assert!(debug.contains("CdseDownloadV1"));
    assert!(!debug.contains(SYNTHETIC_BEARER));
    assert!(!format!("{resolver:?}").contains(SYNTHETIC_BEARER));
    assert!(
        !format!("{:?}", CredentialMaterial::bearer(SYNTHETIC_BEARER)).contains(SYNTHETIC_BEARER)
    );
    assert!(!format!("{:?}", frozen()).contains(SYNTHETIC_BEARER));
    assert!(!format!("{:?}", frozen().descriptor).contains(SYNTHETIC_BEARER));
    let source = include_str!("../src/asset_resolver/cdse.rs");
    assert!(!source.contains("std::env"));
    assert!(!source.contains("env::"));
    assert!(!source.contains("reqwest"));
}

#[test]
fn cdse_resolver_requires_the_exact_descriptor_and_reference_identity() {
    let resolver = CdseArchiveAssetResolver::new();
    assert_eq!(resolver.resolver_id(), "cdse.archive.resolver.v1");
    assert_eq!(
        resolver.supported_schemes(),
        &[AssetAccessScheme::OauthBearerHttps]
    );
    assert!(resolver.can_resolve(&descriptor(PRODUCT_ID)));

    for mutate in [
        |descriptor: &mut AssetAccessDescriptor| descriptor.resolver_id = "other".to_owned(),
        |descriptor: &mut AssetAccessDescriptor| {
            descriptor.scheme = AssetAccessScheme::EarthdataHttps
        },
        |descriptor: &mut AssetAccessDescriptor| descriptor.requires_credentials = false,
        |descriptor: &mut AssetAccessDescriptor| {
            descriptor.credential_profile = Some("earthdata-default".to_owned())
        },
        |descriptor: &mut AssetAccessDescriptor| descriptor.stable_href = None,
        |descriptor: &mut AssetAccessDescriptor| {
            descriptor.stable_href = Some("https://example.invalid/file".to_owned())
        },
    ] {
        let mut value = descriptor(PRODUCT_ID);
        mutate(&mut value);
        assert!(!resolver.can_resolve(&value));
    }

    let credentials = FixtureCredentials::bearer();
    let cancelled = || false;
    let clock = FixedClock;
    for mutate in [
        |frozen: &mut FrozenAssetAccess| frozen.reference.provider_id = "other.provider".to_owned(),
        |frozen: &mut FrozenAssetAccess| {
            frozen.reference.collection_id = "other-collection".to_owned()
        },
        |frozen: &mut FrozenAssetAccess| frozen.reference.asset_key = "data".to_owned(),
        |frozen: &mut FrozenAssetAccess| {
            frozen.reference.provider_product_key = "attacker-key".to_owned()
        },
        |frozen: &mut FrozenAssetAccess| {
            frozen.reference.stable_asset_key = "attacker-key".to_owned()
        },
        |frozen: &mut FrozenAssetAccess| {
            frozen.reference.provider_product_id = "not-an-odata-uuid".to_owned()
        },
        |frozen: &mut FrozenAssetAccess| {
            frozen.reference.provider_product_id = "00000000000040008000000000000001".to_owned()
        },
    ] {
        let mut value = frozen();
        mutate(&mut value);
        assert_eq!(
            resolver
                .resolve(
                    &value.reference,
                    &value.descriptor,
                    &context(&credentials, &cancelled, &clock)
                )
                .unwrap_err()
                .code(),
            "ASSET_REFERENCE_INVALID"
        );
    }
}

#[test]
fn cdse_resolver_preserves_the_authoritative_uuid_case_in_the_exact_locator() {
    let product_id = "00000000-0000-4000-8000-0000000000AB";
    let resolver = CdseArchiveAssetResolver::new();
    let credentials = FixtureCredentials::bearer();
    let cancelled = || false;
    let clock = FixedClock;
    let reference = reference(product_id);
    let descriptor = descriptor(product_id);
    let resolved = resolver
        .resolve(
            &reference,
            &descriptor,
            &context(&credentials, &cancelled, &clock),
        )
        .expect("canonical uppercase OData UUID remains authoritative");
    assert_eq!(resolved.effective_url().as_str(), stable_href(product_id));

    let mut rewritten = descriptor;
    rewritten.stable_href = Some(stable_href(&product_id.to_ascii_lowercase()));
    assert_eq!(
        resolver
            .resolve(
                &reference,
                &rewritten,
                &context(&credentials, &cancelled, &clock),
            )
            .unwrap_err()
            .code(),
        "ASSET_ACCESS_NOT_DURABLE"
    );
}

#[test]
fn cdse_resolver_rejects_any_noncanonical_value_locator_shape() {
    let resolver = CdseArchiveAssetResolver::new();
    let credentials = FixtureCredentials::bearer();
    let cancelled = || false;
    let clock = FixedClock;
    for href in [
        "http://download.dataspace.copernicus.eu/odata/v1/Products(00000000-0000-4000-8000-000000000001)/$value",
        "https://catalogue.dataspace.copernicus.eu/odata/v1/Products(00000000-0000-4000-8000-000000000001)/$value",
        "https://download.dataspace.copernicus.eu:444/odata/v1/Products(00000000-0000-4000-8000-000000000001)/$value",
        "https://user@download.dataspace.copernicus.eu/odata/v1/Products(00000000-0000-4000-8000-000000000001)/$value",
        "https://download.dataspace.copernicus.eu/odata/v1/Products(00000000-0000-4000-8000-000000000001)/$value?token=synthetic",
        "https://download.dataspace.copernicus.eu/odata/v1/Products(00000000-0000-4000-8000-000000000001)/$value#fragment",
        "https://download.dataspace.copernicus.eu/odata/v1/Products%2800000000-0000-4000-8000-000000000001%29/$value",
        "https://download.dataspace.copernicus.eu/odata/v1/Products(00000000-0000-4000-8000-000000000001)/$zip",
        "https://download.dataspace.copernicus.eu/odata/v1/Products(00000000-0000-4000-8000-000000000001)/Nodes",
        "https://download.dataspace.copernicus.eu/odata/v1/Products(00000000-0000-4000-8000-000000000001)/$value/extra",
    ] {
        let mut value = frozen();
        value.descriptor.stable_href = Some(href.to_owned());
        assert_eq!(
            resolver
                .resolve(&value.reference, &value.descriptor, &context(&credentials, &cancelled, &clock))
                .unwrap_err()
                .code(),
            "ASSET_ACCESS_DESCRIPTOR_INVALID",
            "{href}"
        );
    }

    let mut mismatched_uuid = frozen();
    mismatched_uuid.descriptor.stable_href =
        Some(stable_href("00000000-0000-4000-8000-000000000002"));
    assert_eq!(
        resolver
            .resolve(
                &mismatched_uuid.reference,
                &mismatched_uuid.descriptor,
                &context(&credentials, &cancelled, &clock),
            )
            .unwrap_err()
            .code(),
        "ASSET_ACCESS_NOT_DURABLE"
    );
}

#[test]
fn cdse_resolver_accepts_only_bearer_credentials_and_is_cancellable() {
    let resolver = CdseArchiveAssetResolver::new();
    let clock = FixedClock;
    let never_cancelled = || false;

    for outcome in [CredentialOutcome::Basic, CredentialOutcome::Cookie] {
        let credentials = FixtureCredentials {
            outcome,
            calls: AtomicUsize::new(0),
            cancel_after_lookup: None,
        };
        assert_eq!(
            resolver
                .resolve(
                    &frozen().reference,
                    &frozen().descriptor,
                    &context(&credentials, &never_cancelled, &clock)
                )
                .unwrap_err()
                .code(),
            "ASSET_AUTH_FAILED"
        );
    }
    for (outcome, expected) in [
        (
            CredentialOutcome::Error(CredentialProviderError::Missing),
            "ASSET_CREDENTIALS_MISSING",
        ),
        (
            CredentialOutcome::Error(CredentialProviderError::Rejected),
            "ASSET_AUTH_FAILED",
        ),
        (
            CredentialOutcome::Error(CredentialProviderError::Timeout),
            "ASSET_RESOLUTION_TIMEOUT",
        ),
        (
            CredentialOutcome::Error(CredentialProviderError::Failed),
            "ASSET_AUTH_FAILED",
        ),
    ] {
        let credentials = FixtureCredentials {
            outcome,
            calls: AtomicUsize::new(0),
            cancel_after_lookup: None,
        };
        assert_eq!(
            resolver
                .resolve(
                    &frozen().reference,
                    &frozen().descriptor,
                    &context(&credentials, &never_cancelled, &clock)
                )
                .unwrap_err()
                .code(),
            expected
        );
    }

    let before_credentials = FixtureCredentials::bearer();
    let before_cancelled = || true;
    assert_eq!(
        resolver
            .resolve(
                &frozen().reference,
                &frozen().descriptor,
                &context(&before_credentials, &before_cancelled, &clock),
            )
            .unwrap_err()
            .code(),
        "ASSET_RESOLUTION_CANCELLED"
    );
    assert_eq!(before_credentials.calls.load(Ordering::SeqCst), 0);

    let cancelled_after_lookup = AtomicBool::new(false);
    let after_credentials = FixtureCredentials {
        outcome: CredentialOutcome::Bearer,
        calls: AtomicUsize::new(0),
        cancel_after_lookup: Some(&cancelled_after_lookup),
    };
    let after_cancelled = || cancelled_after_lookup.load(Ordering::SeqCst);
    let cancelled_error = resolver
        .resolve(
            &frozen().reference,
            &frozen().descriptor,
            &context(&after_credentials, &after_cancelled, &clock),
        )
        .unwrap_err();
    assert_eq!(cancelled_error.code(), "ASSET_RESOLUTION_CANCELLED");
    assert_eq!(after_credentials.calls.load(Ordering::SeqCst), 1);
    assert!(!format!("{cancelled_error:?}").contains(SYNTHETIC_BEARER));
}

#[test]
fn cdse_frozen_access_requires_primary_archive_and_frozen_evidence() {
    let mut registry = AssetResolverRegistry::new();
    registry
        .register(std::sync::Arc::new(CdseArchiveAssetResolver::new()))
        .unwrap();
    let credentials = FixtureCredentials::bearer();
    let cancelled = || false;
    let clock = FixedClock;
    for mutate in [
        |frozen: &mut FrozenAssetAccess| frozen.role = AssetRole::Metadata,
        |frozen: &mut FrozenAssetAccess| frozen.file_name = None,
        |frozen: &mut FrozenAssetAccess| frozen.expected_size_bytes = None,
        |frozen: &mut FrozenAssetAccess| frozen.checksum_algorithm = None,
        |frozen: &mut FrozenAssetAccess| frozen.file_name = Some("../unsafe.zip".to_owned()),
    ] {
        let mut value = frozen();
        mutate(&mut value);
        assert_eq!(
            registry
                .resolve_frozen(&value, &context(&credentials, &cancelled, &clock))
                .unwrap_err()
                .code(),
            "ASSET_ACCESS_DESCRIPTOR_INVALID"
        );
    }
}

#[test]
fn standard_registry_registers_cdse_by_exact_id_without_oauth_scheme_fallback() {
    let standard = AssetResolverRegistry::with_standard_resolvers().unwrap();
    assert!(standard
        .resolver_ids()
        .contains(&"cdse.archive.resolver.v1"));

    let credentials = FixtureCredentials::bearer();
    let cancelled = || false;
    let clock = FixedClock;
    assert!(standard
        .resolve_frozen(&frozen(), &context(&credentials, &cancelled, &clock))
        .is_ok());

    let mut unknown = frozen();
    unknown.descriptor.resolver_id = "unregistered-cdse-resolver".to_owned();
    assert_eq!(
        standard
            .resolve_frozen(&unknown, &context(&credentials, &cancelled, &clock))
            .unwrap_err()
            .code(),
        "ASSET_RESOLVER_NOT_CONFIGURED"
    );

    let mut explicit = AssetResolverRegistry::new();
    explicit
        .register(std::sync::Arc::new(CdseArchiveAssetResolver::new()))
        .expect("first explicit CDSE resolver registration");
    assert_eq!(
        explicit
            .register(std::sync::Arc::new(CdseArchiveAssetResolver::new()))
            .unwrap_err()
            .code(),
        "ASSET_ACCESS_DESCRIPTOR_INVALID"
    );
}

#[test]
fn cdse_resolver_accepts_sentinel3_archive_collection_identity() {
    let registry = AssetResolverRegistry::with_standard_resolvers().unwrap();
    let credentials = FixtureCredentials::bearer();
    let cancelled = || false;
    let clock = FixedClock;
    let provider_product_key =
        provider_product_key("copernicus.dataspace.odata", "SENTINEL-3", PRODUCT_ID)
            .expect("sentinel-3 fixture product key");
    let stable_asset_key =
        stable_asset_key(&provider_product_key, "archive").expect("sentinel-3 fixture asset key");
    let mut s3_frozen = frozen();
    s3_frozen.reference = StableAssetReference {
        provider_id: "copernicus.dataspace.odata".to_owned(),
        collection_id: "SENTINEL-3".to_owned(),
        provider_product_id: PRODUCT_ID.to_owned(),
        provider_product_key,
        canonical_product_key: None,
        asset_key: "archive".to_owned(),
        stable_asset_key,
    };
    s3_frozen.file_name =
        Some("S3A_OL_1_EFR____20261124T020000_20261124T020300.SEN3.zip".to_owned());

    let resolved = registry
        .resolve_frozen(&s3_frozen, &context(&credentials, &cancelled, &clock))
        .expect("Sentinel-3 collection identity must resolve to exact CDSE OData $value href");
    assert_eq!(resolved.effective_url().as_str(), stable_href(PRODUCT_ID));
    assert_eq!(resolved.method(), ResolvedMethod::Get);
    assert_eq!(
        resolved.suggested_filename(),
        Some("S3A_OL_1_EFR____20261124T020000_20261124T020300.SEN3.zip")
    );
}
