use insar_core::{
    dns_name_matches, AssetAccessDescriptor, AssetAccessScheme, FrozenAssetAccess,
    StableAssetReference,
};

use super::public_https::{parse_public_durable_https, suggested_filename};
use super::shadow_plan::plan_earthdata_https;
use super::types::ResolvedHostPolicy;
use super::{
    AssetResolver, CredentialProviderError, ResolveContext, ResolveError, ResolvedAsset,
    ResolvedMethod, ShadowAccessIntent, ShadowResolveContext,
};

pub struct EarthdataAssetResolver;

impl EarthdataAssetResolver {
    pub const fn official() -> Self {
        Self
    }
}

impl Default for EarthdataAssetResolver {
    fn default() -> Self {
        Self::official()
    }
}

impl AssetResolver for EarthdataAssetResolver {
    fn resolver_id(&self) -> &'static str {
        "earthdata_https"
    }

    fn supported_schemes(&self) -> &'static [AssetAccessScheme] {
        &[AssetAccessScheme::EarthdataHttps]
    }

    fn can_resolve(&self, descriptor: &AssetAccessDescriptor) -> bool {
        descriptor.scheme == AssetAccessScheme::EarthdataHttps
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
        if descriptor.scheme != AssetAccessScheme::EarthdataHttps
            || !descriptor.requires_credentials
        {
            return Err(ResolveError::descriptor_invalid());
        }
        let profile = descriptor
            .credential_profile
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(ResolveError::descriptor_invalid)?;
        context.check_cancelled()?;

        let href = descriptor
            .stable_href
            .as_deref()
            .ok_or_else(ResolveError::access_not_durable)?;
        let url = parse_public_durable_https(href)?;
        if !url.host_str().is_some_and(is_approved_earthdata_host) {
            return Err(ResolveError::access_not_durable());
        }

        let credential = context
            .credential_provider()
            .resolve(profile)
            .map_err(map_credential_error)?;
        context.check_cancelled()?;
        let filename = suggested_filename(&url);
        let mut resolved = ResolvedAsset::new(url.as_str(), ResolvedMethod::Get)?
            .with_suggested_filename(filename)
            .with_host_policy(ResolvedHostPolicy::EarthdataAsfV1);
        credential.apply_to(&mut resolved)?;
        Ok(resolved)
    }

    fn plan_shadow(
        &self,
        frozen: &FrozenAssetAccess,
        context: &ShadowResolveContext<'_>,
    ) -> Result<ShadowAccessIntent, ResolveError> {
        let descriptor = &frozen.descriptor;
        if descriptor.scheme != AssetAccessScheme::EarthdataHttps
            || !descriptor.requires_credentials
        {
            return Err(ResolveError::descriptor_invalid());
        }
        let profile = descriptor
            .credential_profile
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(ResolveError::descriptor_invalid)?;
        let auth_kind = context.credential_shape(profile)?;
        plan_earthdata_https(frozen, context, auth_kind)
    }
}

fn is_approved_earthdata_host(host: &str) -> bool {
    [
        "earthdata.nasa.gov",
        "earthdatacloud.nasa.gov",
        "asf.alaska.edu",
    ]
    .iter()
    .any(|allowed| dns_name_matches(host, allowed))
}

fn map_credential_error(error: CredentialProviderError) -> ResolveError {
    match error {
        CredentialProviderError::Missing => ResolveError::credentials_missing(),
        CredentialProviderError::Rejected => ResolveError::auth_failed(),
        CredentialProviderError::Timeout => ResolveError::timeout(),
        CredentialProviderError::Failed => ResolveError::auth_failed(),
        CredentialProviderError::BurstCookieRequired => ResolveError::burst_cookie_required(),
    }
}
