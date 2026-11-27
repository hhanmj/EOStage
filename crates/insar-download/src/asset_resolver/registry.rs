use std::collections::HashMap;
use std::sync::Arc;

use insar_core::{
    AssetAccessDescriptor, AssetAccessScheme, FrozenAssetAccess, ProviderNetworkPolicy,
    StableAssetReference,
};

use super::{
    AssetResolver, CdseArchiveAssetResolver, EarthSearchSentinel2BandAssetResolver,
    EarthdataAssetResolver, LocalFileAssetResolver, PlanetaryComputerLandsatAssetResolver,
    PlanetaryComputerReviewedAssetResolver, PublicHttpsAssetResolver, ResolveContext, ResolveError,
    ResolvedAsset, ShadowAccessIntent, ShadowResolveContext,
};

/// Deterministic runtime resolver selection.
///
/// Registration is explicit: the registry never walks resolvers and guesses
/// from a hostname. A descriptor's exact resolver wins. Only when that exact
/// implementation is absent can an explicitly configured scheme fallback run.
pub struct AssetResolverRegistry {
    by_id: HashMap<String, Arc<dyn AssetResolver>>,
    scheme_fallbacks: HashMap<AssetAccessScheme, String>,
}

impl AssetResolverRegistry {
    pub fn new() -> Self {
        Self {
            by_id: HashMap::new(),
            scheme_fallbacks: HashMap::new(),
        }
    }

    pub fn with_standard_resolvers() -> Result<Self, ResolveError> {
        Self::with_standard_resolvers_for_network(ProviderNetworkPolicy::system())
    }

    pub fn with_standard_resolvers_for_network(
        network_policy: ProviderNetworkPolicy,
    ) -> Result<Self, ResolveError> {
        let mut registry = Self::new();
        registry.register(Arc::new(PublicHttpsAssetResolver::new()))?;
        registry.register(Arc::new(LocalFileAssetResolver::new()))?;
        registry.register(Arc::new(EarthdataAssetResolver::official()))?;
        // CDSE archives are resolver-authoritative and have an exact frozen
        // resolver ID. Deliberately do not add an OAuth scheme fallback.
        registry.register(Arc::new(CdseArchiveAssetResolver::new()))?;
        // Planetary Computer Landsat assets also require an exact resolver ID.
        // The generic public-HTTPS fallback must never receive their SAS refresh
        // authority.
        registry.register(Arc::new(PlanetaryComputerLandsatAssetResolver::new(
            network_policy,
        )?))?;
        registry.register(Arc::new(PlanetaryComputerLandsatAssetResolver::hls(
            network_policy,
        )?))?;
        registry.register(Arc::new(
            PlanetaryComputerReviewedAssetResolver::sentinel1_grd(network_policy)?,
        ))?;
        registry.register(Arc::new(
            PlanetaryComputerReviewedAssetResolver::cop_dem_glo30(network_policy)?,
        ))?;
        registry.register(Arc::new(
            PlanetaryComputerReviewedAssetResolver::sentinel2_l2a_band(network_policy)?,
        ))?;
        registry.register(Arc::new(EarthSearchSentinel2BandAssetResolver::new()))?;
        registry.register_scheme_fallback(AssetAccessScheme::PublicHttps, "public_https")?;
        registry.register_scheme_fallback(AssetAccessScheme::LocalFile, "local_file")?;
        registry.register_scheme_fallback(AssetAccessScheme::EarthdataHttps, "earthdata_https")?;
        Ok(registry)
    }

    pub fn register(&mut self, resolver: Arc<dyn AssetResolver>) -> Result<(), ResolveError> {
        let resolver_id = resolver.resolver_id();
        if resolver_id.trim().is_empty()
            || resolver_id.chars().any(char::is_control)
            || resolver.supported_schemes().is_empty()
            || self.by_id.contains_key(resolver_id)
        {
            return Err(ResolveError::descriptor_invalid());
        }
        self.by_id.insert(resolver_id.to_string(), resolver);
        Ok(())
    }

    pub fn register_scheme_fallback(
        &mut self,
        scheme: AssetAccessScheme,
        resolver_id: impl Into<String>,
    ) -> Result<(), ResolveError> {
        let resolver_id = resolver_id.into();
        let Some(resolver) = self.by_id.get(&resolver_id) else {
            return Err(ResolveError::not_configured());
        };
        if !resolver.supported_schemes().contains(&scheme)
            || self.scheme_fallbacks.contains_key(&scheme)
        {
            return Err(ResolveError::descriptor_invalid());
        }
        self.scheme_fallbacks.insert(scheme, resolver_id);
        Ok(())
    }

    pub fn resolve(
        &self,
        reference: &StableAssetReference,
        descriptor: &AssetAccessDescriptor,
        context: &ResolveContext<'_>,
    ) -> Result<ResolvedAsset, ResolveError> {
        reference
            .validate()
            .map_err(|_| ResolveError::reference_invalid())?;
        descriptor
            .validate()
            .map_err(|_| ResolveError::descriptor_invalid())?;
        context.check_cancelled()?;

        let resolver = if let Some(exact) = self.by_id.get(&descriptor.resolver_id) {
            exact
        } else if let Some(fallback_id) = self.scheme_fallbacks.get(&descriptor.scheme) {
            self.by_id
                .get(fallback_id)
                .ok_or_else(ResolveError::not_configured)?
        } else {
            return Err(ResolveError::not_configured());
        };

        if !resolver.supported_schemes().contains(&descriptor.scheme)
            || !resolver.can_resolve(descriptor)
        {
            return Err(ResolveError::descriptor_invalid());
        }
        resolver.resolve(reference, descriptor, context)
    }

    pub fn resolve_frozen(
        &self,
        frozen: &FrozenAssetAccess,
        context: &ResolveContext<'_>,
    ) -> Result<ResolvedAsset, ResolveError> {
        frozen
            .validate()
            .map_err(|_| ResolveError::descriptor_invalid())?;
        context.check_cancelled()?;
        let descriptor = &frozen.descriptor;
        let resolver = self
            .by_id
            .get(&descriptor.resolver_id)
            .ok_or_else(ResolveError::not_configured)?;
        if !resolver.supported_schemes().contains(&descriptor.scheme)
            || !resolver.can_resolve(descriptor)
        {
            return Err(ResolveError::descriptor_invalid());
        }
        resolver.validate_frozen(frozen)?;
        let mut resolved = resolver.resolve(&frozen.reference, descriptor, context)?;

        if let (Some(actual), Some(expected)) =
            (resolved.suggested_filename(), frozen.file_name.as_deref())
        {
            if actual != expected {
                return Err(ResolveError::failed());
            }
        }
        if let (Some(actual), Some(expected)) =
            (resolved.expected_size(), frozen.expected_size_bytes)
        {
            if actual != expected {
                return Err(ResolveError::failed());
            }
        }
        let frozen_checksum = match (
            frozen.checksum_algorithm.as_deref(),
            frozen.checksum_value.as_deref(),
        ) {
            (Some(algorithm), Some(value)) => Some(super::ResolvedChecksum::new(algorithm, value)?),
            (None, None) => None,
            _ => return Err(ResolveError::descriptor_invalid()),
        };
        if let (Some(actual), Some(expected)) =
            (resolved.expected_checksum(), frozen_checksum.as_ref())
        {
            if actual != expected {
                return Err(ResolveError::failed());
            }
        }
        if let Some(actual) = resolved.asset_role() {
            if actual != &frozen.role {
                return Err(ResolveError::failed());
            }
        }

        // A runtime resolver may discover immutable evidence that the
        // catalogue deliberately left deferred (Bhoonidhi uses the bounded
        // Range probe for this). Only overwrite the resolved size when the
        // frozen contract actually supplied one.
        if let Some(expected_size) = frozen.expected_size_bytes {
            resolved = resolved.with_expected_size(Some(expected_size));
        }
        resolved = resolved
            .with_checksum(frozen_checksum)
            .with_suggested_filename(frozen.file_name.clone())
            .with_asset_role(frozen.role.clone());
        Ok(resolved)
    }

    pub fn resolver_ids(&self) -> Vec<&str> {
        let mut ids = self.by_id.keys().map(String::as_str).collect::<Vec<_>>();
        ids.sort_unstable();
        ids
    }

    pub fn prepare_auth_refresh(&self, frozen: &FrozenAssetAccess) -> Result<(), ResolveError> {
        frozen
            .validate()
            .map_err(|_| ResolveError::descriptor_invalid())?;
        let resolver = self
            .by_id
            .get(&frozen.descriptor.resolver_id)
            .ok_or_else(ResolveError::not_configured)?;
        resolver.validate_frozen(frozen)?;
        resolver.prepare_auth_refresh(frozen)
    }

    pub fn plan_shadow(
        &self,
        frozen: &FrozenAssetAccess,
        context: &ShadowResolveContext<'_>,
    ) -> Result<ShadowAccessIntent, ResolveError> {
        frozen
            .validate()
            .map_err(|_| ResolveError::descriptor_invalid())?;
        context.checkpoint()?;
        let descriptor = &frozen.descriptor;
        let resolver = if let Some(exact) = self.by_id.get(&descriptor.resolver_id) {
            exact
        } else if let Some(fallback_id) = self.scheme_fallbacks.get(&descriptor.scheme) {
            self.by_id
                .get(fallback_id)
                .ok_or_else(ResolveError::not_configured)?
        } else {
            return Err(ResolveError::not_configured());
        };
        if !resolver.supported_schemes().contains(&descriptor.scheme)
            || !resolver.can_resolve(descriptor)
        {
            return Err(ResolveError::descriptor_invalid());
        }
        context.checkpoint()?;
        resolver.plan_shadow(frozen, context)
    }
}

impl Default for AssetResolverRegistry {
    fn default() -> Self {
        Self::new()
    }
}
