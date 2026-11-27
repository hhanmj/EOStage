use std::net::IpAddr;

use insar_core::{
    normalize_dns_name, AssetAccessDescriptor, AssetAccessScheme, FrozenAssetAccess,
    StableAssetReference,
};
use reqwest::Url;

use super::shadow_plan::plan_public_https;
use super::{
    AssetResolver, ResolveContext, ResolveError, ResolvedAsset, ResolvedMethod, ShadowAccessIntent,
    ShadowResolveContext,
};

pub struct PublicHttpsAssetResolver;

impl PublicHttpsAssetResolver {
    pub const fn new() -> Self {
        Self
    }
}

impl Default for PublicHttpsAssetResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl AssetResolver for PublicHttpsAssetResolver {
    fn resolver_id(&self) -> &'static str {
        "public_https"
    }

    fn supported_schemes(&self) -> &'static [AssetAccessScheme] {
        &[AssetAccessScheme::PublicHttps]
    }

    fn can_resolve(&self, descriptor: &AssetAccessDescriptor) -> bool {
        descriptor.scheme == AssetAccessScheme::PublicHttps
    }

    fn resolve(
        &self,
        reference: &StableAssetReference,
        descriptor: &AssetAccessDescriptor,
        context: &ResolveContext<'_>,
    ) -> Result<ResolvedAsset, ResolveError> {
        reference
            .validate()
            .map_err(|_| ResolveError::reference_invalid())?;
        if descriptor.scheme != AssetAccessScheme::PublicHttps
            || descriptor.requires_credentials
            || descriptor.credential_profile.is_some()
        {
            return Err(ResolveError::descriptor_invalid());
        }
        context.check_cancelled()?;
        let href = descriptor
            .stable_href
            .as_deref()
            .ok_or_else(ResolveError::access_not_durable)?;
        let url = parse_public_durable_https(href)?;
        let suggested_filename = suggested_filename(&url);
        ResolvedAsset::new(url.as_str(), ResolvedMethod::Get)
            .map(|asset| asset.with_suggested_filename(suggested_filename))
    }

    fn plan_shadow(
        &self,
        frozen: &FrozenAssetAccess,
        context: &ShadowResolveContext<'_>,
    ) -> Result<ShadowAccessIntent, ResolveError> {
        if frozen.descriptor.scheme != AssetAccessScheme::PublicHttps
            || frozen.descriptor.requires_credentials
            || frozen.descriptor.credential_profile.is_some()
        {
            return Err(ResolveError::descriptor_invalid());
        }
        plan_public_https(frozen, context)
    }
}

pub(super) fn parse_public_durable_https(href: &str) -> Result<Url, ResolveError> {
    let url = Url::parse(href).map_err(|_| ResolveError::access_not_durable())?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.port_or_known_default() != Some(443)
        || !url.host_str().is_some_and(is_public_host)
    {
        return Err(ResolveError::access_not_durable());
    }
    Ok(url)
}

pub(super) fn suggested_filename(url: &Url) -> Option<String> {
    url.path_segments()
        .and_then(|mut segments| segments.rfind(|part| !part.is_empty()))
        .filter(|name| name != &"." && name != &"..")
        .map(str::to_string)
}

fn is_public_host(host: &str) -> bool {
    let normalized = host
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if normalized.is_empty()
        || normalized == "localhost"
        || normalized.ends_with(".localhost")
        || normalized.ends_with(".local")
        || normalized.ends_with(".internal")
    {
        return false;
    }

    match normalized.parse::<IpAddr>() {
        Ok(IpAddr::V4(address)) => {
            !(address.is_private()
                || address.is_loopback()
                || address.is_link_local()
                || address.is_unspecified()
                || address.is_broadcast()
                || address.is_documentation()
                || address.is_multicast())
        }
        Ok(IpAddr::V6(address)) => {
            !(address.is_loopback()
                || address.is_unspecified()
                || address.is_unique_local()
                || address.is_unicast_link_local()
                || address.is_multicast())
        }
        Err(_) => normalize_dns_name(&normalized).is_ok_and(|host| host.contains('.')),
    }
}
