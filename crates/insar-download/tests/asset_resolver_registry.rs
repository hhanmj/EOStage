use std::sync::Arc;
use std::time::SystemTime;

use insar_core::{
    AssetAccessDescriptor, AssetAccessScheme, AssetResolverParameters, AssetRole,
    FrozenAssetAccess, StableAssetReference,
};
use insar_download::{
    AssetResolver, AssetResolverRegistry, Clock, CredentialMaterial, CredentialProvider,
    CredentialProviderError, ResolveContext, ResolveError, ResolvedAsset, ResolvedChecksum,
    ResolvedMethod, ShadowAccessIntent, ShadowResolveContext,
};

struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> SystemTime {
        SystemTime::UNIX_EPOCH
    }
}

struct NoCredentials;

impl CredentialProvider for NoCredentials {
    fn resolve(&self, _profile: &str) -> Result<CredentialMaterial, CredentialProviderError> {
        Err(CredentialProviderError::Missing)
    }
}

struct MarkerResolver {
    id: &'static str,
    marker: &'static str,
}

struct ConflictingResolver;

impl AssetResolver for ConflictingResolver {
    fn resolver_id(&self) -> &'static str {
        "conflicting"
    }

    fn supported_schemes(&self) -> &'static [AssetAccessScheme] {
        &[AssetAccessScheme::PublicHttps]
    }

    fn can_resolve(&self, descriptor: &AssetAccessDescriptor) -> bool {
        descriptor.scheme == AssetAccessScheme::PublicHttps
    }

    fn resolve(
        &self,
        _reference: &StableAssetReference,
        descriptor: &AssetAccessDescriptor,
        _context: &ResolveContext<'_>,
    ) -> Result<ResolvedAsset, ResolveError> {
        Ok(ResolvedAsset::new(
            descriptor.stable_href.as_deref().unwrap(),
            ResolvedMethod::Get,
        )?
        .with_suggested_filename(Some("other.bin".to_string()))
        .with_expected_size(Some(2048))
        .with_checksum(Some(ResolvedChecksum::new("sha256", "other").unwrap()))
        .with_asset_role(AssetRole::Metadata))
    }

    fn plan_shadow(
        &self,
        _frozen: &FrozenAssetAccess,
        _context: &ShadowResolveContext<'_>,
    ) -> Result<ShadowAccessIntent, ResolveError> {
        Err(ResolveError::not_configured())
    }
}

impl AssetResolver for MarkerResolver {
    fn resolver_id(&self) -> &'static str {
        self.id
    }

    fn supported_schemes(&self) -> &'static [AssetAccessScheme] {
        &[AssetAccessScheme::PublicHttps]
    }

    fn can_resolve(&self, descriptor: &AssetAccessDescriptor) -> bool {
        descriptor.scheme == AssetAccessScheme::PublicHttps
    }

    fn resolve(
        &self,
        _reference: &StableAssetReference,
        descriptor: &AssetAccessDescriptor,
        _context: &ResolveContext<'_>,
    ) -> Result<ResolvedAsset, ResolveError> {
        ResolvedAsset::new(
            descriptor.stable_href.as_deref().unwrap(),
            ResolvedMethod::Get,
        )
        .map(|asset| asset.with_suggested_filename(Some(self.marker.to_string())))
    }

    fn plan_shadow(
        &self,
        _frozen: &FrozenAssetAccess,
        _context: &ShadowResolveContext<'_>,
    ) -> Result<ShadowAccessIntent, ResolveError> {
        Err(ResolveError::not_configured())
    }
}

fn reference() -> StableAssetReference {
    StableAssetReference {
        provider_id: "fixture".to_string(),
        collection_id: "fixture-collection".to_string(),
        provider_product_id: "fixture-product".to_string(),
        provider_product_key: "fixture::product".to_string(),
        canonical_product_key: None,
        asset_key: "primary".to_string(),
        stable_asset_key: "fixture::product::primary".to_string(),
    }
}

fn descriptor(resolver_id: &str) -> AssetAccessDescriptor {
    AssetAccessDescriptor {
        resolver_id: resolver_id.to_string(),
        scheme: AssetAccessScheme::PublicHttps,
        stable_href: Some("https://example.org/product.bin".to_string()),
        requires_credentials: false,
        credential_profile: None,
        resolver_parameters: AssetResolverParameters {},
    }
}

fn frozen(resolver_id: &str, file_name: &str) -> FrozenAssetAccess {
    FrozenAssetAccess {
        reference: reference(),
        descriptor: descriptor(resolver_id),
        role: AssetRole::Primary,
        file_name: Some(file_name.to_string()),
        expected_size_bytes: Some(1024),
        checksum_algorithm: Some("sha256".to_string()),
        checksum_value: Some("fixture-checksum".to_string()),
    }
}

fn context<'a>(
    credentials: &'a NoCredentials,
    cancelled: &'a (dyn Fn() -> bool + Sync),
    clock: &'a FixedClock,
) -> ResolveContext<'a> {
    ResolveContext::new(credentials, cancelled, clock)
}

#[test]
fn exact_resolver_wins_before_registered_scheme_fallback() {
    let mut registry = AssetResolverRegistry::new();
    registry
        .register(Arc::new(MarkerResolver {
            id: "exact",
            marker: "exact.bin",
        }))
        .unwrap();
    registry
        .register(Arc::new(MarkerResolver {
            id: "fallback",
            marker: "fallback.bin",
        }))
        .unwrap();
    registry
        .register_scheme_fallback(AssetAccessScheme::PublicHttps, "fallback")
        .unwrap();

    let credentials = NoCredentials;
    let cancelled = || false;
    let clock = FixedClock;
    let resolved = registry
        .resolve(
            &reference(),
            &descriptor("exact"),
            &context(&credentials, &cancelled, &clock),
        )
        .unwrap();

    assert_eq!(resolved.suggested_filename(), Some("exact.bin"));
}

#[test]
fn compatible_scheme_fallback_is_explicit_and_deterministic() {
    let mut registry = AssetResolverRegistry::new();
    registry
        .register(Arc::new(MarkerResolver {
            id: "fallback",
            marker: "fallback.bin",
        }))
        .unwrap();
    registry
        .register_scheme_fallback(AssetAccessScheme::PublicHttps, "fallback")
        .unwrap();

    let credentials = NoCredentials;
    let cancelled = || false;
    let clock = FixedClock;
    let resolved = registry
        .resolve(
            &reference(),
            &descriptor("provider-specific-not-installed"),
            &context(&credentials, &cancelled, &clock),
        )
        .unwrap();

    assert_eq!(resolved.suggested_filename(), Some("fallback.bin"));
}

#[test]
fn duplicate_registration_and_unknown_resolver_are_typed_failures() {
    let mut registry = AssetResolverRegistry::new();
    registry
        .register(Arc::new(MarkerResolver {
            id: "same",
            marker: "one.bin",
        }))
        .unwrap();
    let duplicate = registry
        .register(Arc::new(MarkerResolver {
            id: "same",
            marker: "two.bin",
        }))
        .unwrap_err();
    assert_eq!(duplicate.code(), "ASSET_ACCESS_DESCRIPTOR_INVALID");

    let credentials = NoCredentials;
    let cancelled = || false;
    let clock = FixedClock;
    let unknown = AssetResolverRegistry::new()
        .resolve(
            &reference(),
            &descriptor("missing"),
            &context(&credentials, &cancelled, &clock),
        )
        .unwrap_err();
    assert_eq!(unknown.code(), "ASSET_RESOLVER_NOT_CONFIGURED");
}

#[test]
fn standard_registry_has_the_reviewed_exact_resolver_routes() {
    let registry = AssetResolverRegistry::with_standard_resolvers().unwrap();
    assert_eq!(
        registry.resolver_ids(),
        vec![
            "cdse.archive.resolver.v1",
            "earth-search.sentinel-2-l2a.band-cog.v1",
            "earthdata_https",
            "local_file",
            "planetary-computer.cop-dem-glo-30.cog.v1",
            "planetary-computer.hls.assets.v1",
            "planetary-computer.landsat-c2-l2.assets.v1",
            "planetary-computer.sentinel-1-grd.cog.v1",
            "planetary-computer.sentinel-2-l2a.band-cog.v1",
            "public_https",
        ]
    );
}

#[test]
fn resolve_frozen_returns_the_complete_reviewed_asset() {
    let mut registry = AssetResolverRegistry::new();
    registry
        .register(Arc::new(MarkerResolver {
            id: "exact",
            marker: "exact.bin",
        }))
        .unwrap();
    let credentials = NoCredentials;
    let cancelled = || false;
    let clock = FixedClock;

    let resolved = registry
        .resolve_frozen(
            &frozen("exact", "exact.bin"),
            &context(&credentials, &cancelled, &clock),
        )
        .unwrap();

    assert_eq!(resolved.expected_size(), Some(1024));
    assert_eq!(resolved.expected_checksum().unwrap().algorithm, "sha256");
    assert_eq!(resolved.suggested_filename(), Some("exact.bin"));
    assert_eq!(resolved.asset_role(), Some(&AssetRole::Primary));
}

#[test]
fn resolve_frozen_rejects_runtime_conflicts_and_never_uses_scheme_fallback() {
    let mut registry = AssetResolverRegistry::new();
    registry.register(Arc::new(ConflictingResolver)).unwrap();
    registry
        .register(Arc::new(MarkerResolver {
            id: "fallback",
            marker: "fallback.bin",
        }))
        .unwrap();
    registry
        .register_scheme_fallback(AssetAccessScheme::PublicHttps, "fallback")
        .unwrap();
    let credentials = NoCredentials;
    let cancelled = || false;
    let clock = FixedClock;
    let context = context(&credentials, &cancelled, &clock);

    assert_eq!(
        registry
            .resolve_frozen(&frozen("conflicting", "fixture.bin"), &context)
            .unwrap_err()
            .code(),
        "ASSET_RESOLUTION_FAILED"
    );
    assert_eq!(
        registry
            .resolve_frozen(
                &frozen("provider-specific-not-installed", "fallback.bin"),
                &context
            )
            .unwrap_err()
            .code(),
        "ASSET_RESOLVER_NOT_CONFIGURED"
    );
}
