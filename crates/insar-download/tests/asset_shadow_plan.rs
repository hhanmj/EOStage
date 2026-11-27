use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use insar_core::{
    AssetAccessDescriptor, AssetAccessScheme, AssetResolverParameters, AssetRole,
    FrozenAssetAccess, StableAssetReference,
};
use insar_download::{
    AssetResolverRegistry, Clock, ShadowAuthKind, ShadowCredentialShapeProvider, ShadowHostClass,
    ShadowNetworkAccess, ShadowPathClass, ShadowResolveContext,
};

struct MutableClock(Mutex<SystemTime>);

impl MutableClock {
    fn new(now: SystemTime) -> Self {
        Self(Mutex::new(now))
    }

    fn set(&self, now: SystemTime) {
        *self.0.lock().unwrap() = now;
    }
}

impl Clock for MutableClock {
    fn now(&self) -> SystemTime {
        *self.0.lock().unwrap()
    }
}

struct ShapeProvider<'a> {
    calls: &'a AtomicUsize,
}

impl ShadowCredentialShapeProvider for ShapeProvider<'_> {
    fn auth_kind(&self, profile: &str) -> Result<ShadowAuthKind, insar_download::ResolveError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(match profile {
            "earthdata-burst-session" => ShadowAuthKind::EarthdataSessionCookie,
            "earthdata-default" => ShadowAuthKind::EarthdataBasic,
            _ => return Err(insar_download::ResolveError::credentials_missing()),
        })
    }
}

fn frozen(
    scheme: AssetAccessScheme,
    resolver_id: &str,
    href: &str,
    profile: Option<&str>,
) -> FrozenAssetAccess {
    FrozenAssetAccess {
        reference: StableAssetReference {
            provider_id: "asf.catalog".to_string(),
            collection_id: "SENTINEL-1".to_string(),
            provider_product_id: "fixture-product".to_string(),
            provider_product_key: "fixture::product".to_string(),
            canonical_product_key: None,
            asset_key: "fixture.zip".to_string(),
            stable_asset_key: "fixture::product::ASSET::fixture.zip".to_string(),
        },
        descriptor: AssetAccessDescriptor {
            resolver_id: resolver_id.to_string(),
            scheme,
            stable_href: Some(href.to_string()),
            requires_credentials: profile.is_some(),
            credential_profile: profile.map(str::to_string),
            resolver_parameters: AssetResolverParameters {},
        },
        role: AssetRole::Primary,
        file_name: Some("fixture.zip".to_string()),
        expected_size_bytes: Some(4096),
        checksum_algorithm: Some("sha256".to_string()),
        checksum_value: Some("fixture-checksum".to_string()),
    }
}

#[test]
fn earthdata_shadow_plan_uses_only_auth_shape_and_never_credential_values() {
    let shape_calls = AtomicUsize::new(0);
    let credential_value_calls = AtomicUsize::new(0);
    let shapes = ShapeProvider {
        calls: &shape_calls,
    };
    let cancelled = || false;
    let clock = MutableClock::new(SystemTime::UNIX_EPOCH);
    let context =
        ShadowResolveContext::new(&shapes, &cancelled, &clock, Duration::from_millis(25)).unwrap();
    assert_eq!(context.network_access(), ShadowNetworkAccess::Forbidden);

    let registry = AssetResolverRegistry::with_standard_resolvers().unwrap();
    let intent = registry
        .plan_shadow(
            &frozen(
                AssetAccessScheme::EarthdataHttps,
                "earthdata_https",
                "https://earthdatacloud.nasa.gov/fixture.zip",
                Some("earthdata-default"),
            ),
            &context,
        )
        .unwrap();

    assert_eq!(intent.auth_kind(), ShadowAuthKind::EarthdataBasic);
    assert_eq!(intent.host_class(), ShadowHostClass::EarthdataCloud);
    assert_eq!(intent.path_class(), ShadowPathClass::Archive);
    assert_eq!(shape_calls.load(Ordering::SeqCst), 1);
    assert_eq!(credential_value_calls.load(Ordering::SeqCst), 0);
    assert!(!format!("{intent:?}").contains("earthdatacloud.nasa.gov"));
    assert!(!format!("{intent:?}").contains("fixture-checksum"));
}

#[test]
fn public_shadow_plan_does_not_request_auth_shape() {
    let shape_calls = AtomicUsize::new(0);
    let shapes = ShapeProvider {
        calls: &shape_calls,
    };
    let cancelled = || false;
    let clock = MutableClock::new(SystemTime::UNIX_EPOCH);
    let context =
        ShadowResolveContext::new(&shapes, &cancelled, &clock, Duration::from_millis(25)).unwrap();
    let registry = AssetResolverRegistry::with_standard_resolvers().unwrap();

    let mut public = frozen(
        AssetAccessScheme::PublicHttps,
        "public_https",
        "https://example.org/native.tif",
        None,
    );
    public.file_name = Some("native.tif".to_string());
    let intent = registry.plan_shadow(&public, &context).unwrap();

    assert_eq!(intent.auth_kind(), ShadowAuthKind::None);
    assert_eq!(intent.host_class(), ShadowHostClass::PublicHttpsOther);
    assert_eq!(intent.path_class(), ShadowPathClass::NativeFile);
    assert_eq!(shape_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn cancellation_and_injected_deadline_bound_shadow_planning() {
    let shape_calls = AtomicUsize::new(0);
    let shapes = ShapeProvider {
        calls: &shape_calls,
    };
    let clock = MutableClock::new(SystemTime::UNIX_EPOCH);
    let cancelled = || true;
    let cancelled_context =
        ShadowResolveContext::new(&shapes, &cancelled, &clock, Duration::from_millis(25)).unwrap();
    let registry = AssetResolverRegistry::with_standard_resolvers().unwrap();
    let public = frozen(
        AssetAccessScheme::PublicHttps,
        "public_https",
        "https://example.org/fixture.zip",
        None,
    );
    assert_eq!(
        registry
            .plan_shadow(&public, &cancelled_context)
            .unwrap_err()
            .code(),
        "ASSET_RESOLUTION_CANCELLED"
    );

    let active = || false;
    clock.set(SystemTime::UNIX_EPOCH);
    let deadline_context =
        ShadowResolveContext::new(&shapes, &active, &clock, Duration::from_millis(25)).unwrap();
    clock.set(SystemTime::UNIX_EPOCH + Duration::from_millis(25));
    assert_eq!(
        registry
            .plan_shadow(&public, &deadline_context)
            .unwrap_err()
            .code(),
        "ASSET_RESOLUTION_TIMEOUT"
    );
}

#[test]
fn burst_profile_maps_to_session_cookie_without_login() {
    let shape_calls = AtomicUsize::new(0);
    let shapes = ShapeProvider {
        calls: &shape_calls,
    };
    let cancelled = || false;
    let clock = MutableClock::new(SystemTime::UNIX_EPOCH);
    let context =
        ShadowResolveContext::new(&shapes, &cancelled, &clock, Duration::from_millis(25)).unwrap();
    let registry = AssetResolverRegistry::with_standard_resolvers().unwrap();
    let mut burst = frozen(
        AssetAccessScheme::EarthdataHttps,
        "earthdata_https",
        "https://sentinel1-burst.asf.alaska.edu/parent/IW2/VV/7.tiff",
        Some("earthdata-burst-session"),
    );
    burst.file_name = Some("7.tiff".to_string());

    let intent = registry.plan_shadow(&burst, &context).unwrap();
    assert_eq!(intent.auth_kind(), ShadowAuthKind::EarthdataSessionCookie);
    assert_eq!(intent.host_class(), ShadowHostClass::AsfBurst);
    assert_eq!(intent.path_class(), ShadowPathClass::BurstTiff);
}

#[test]
fn shadow_https_plans_preserve_resolver_host_security_without_dns() {
    let shape_calls = AtomicUsize::new(0);
    let shapes = ShapeProvider {
        calls: &shape_calls,
    };
    let cancelled = || false;
    let clock = MutableClock::new(SystemTime::UNIX_EPOCH);
    let context =
        ShadowResolveContext::new(&shapes, &cancelled, &clock, Duration::from_millis(25)).unwrap();
    let registry = AssetResolverRegistry::with_standard_resolvers().unwrap();

    let lookalike = frozen(
        AssetAccessScheme::EarthdataHttps,
        "earthdata_https",
        "https://fakeearthdatacloud.nasa.gov/fixture.zip",
        Some("earthdata-default"),
    );
    assert_eq!(
        registry
            .plan_shadow(&lookalike, &context)
            .unwrap_err()
            .code(),
        "ASSET_ACCESS_NOT_DURABLE"
    );

    let private = frozen(
        AssetAccessScheme::PublicHttps,
        "public_https",
        "https://127.0.0.1/fixture.zip",
        None,
    );
    assert_eq!(
        registry.plan_shadow(&private, &context).unwrap_err().code(),
        "ASSET_ACCESS_NOT_DURABLE"
    );
}
