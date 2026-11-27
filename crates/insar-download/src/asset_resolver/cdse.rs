use insar_core::{
    provider_product_key, stable_asset_key, AssetAccessDescriptor, AssetAccessScheme, AssetRole,
    FrozenAssetAccess, StableAssetReference,
};
use insar_provider::cdse::archive_filename::safe_cdse_odata_name;
use uuid::Uuid;

use super::types::ResolvedHostPolicy;
use super::{
    AssetResolver, CredentialProviderError, ResolveContext, ResolveError, ResolvedAsset,
    ResolvedMethod, ShadowAccessIntent, ShadowResolveContext,
};

pub const CDSE_ARCHIVE_RESOLVER_ID: &str = "cdse.archive.resolver.v1";
pub const CDSE_CREDENTIAL_PROFILE: &str = "copernicus-dataspace-default";
pub(crate) const CDSE_PROVIDER_ID: &str = "copernicus.dataspace.odata";
#[cfg(test)]
pub(crate) const CDSE_COLLECTION_ID: &str = "SENTINEL-2";
pub(crate) const CDSE_ARCHIVE_ASSET_KEY: &str = "archive";
pub(crate) const CDSE_DOWNLOAD_ORIGIN: &str = "https://download.dataspace.copernicus.eu";

/// Default-unregistered resolver for one frozen Sentinel-2 L2A archive.
///
/// It only derives runtime access and never creates a network request.
#[derive(Debug)]
pub struct CdseArchiveAssetResolver;

impl CdseArchiveAssetResolver {
    pub const fn new() -> Self {
        Self
    }
}

impl Default for CdseArchiveAssetResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl AssetResolver for CdseArchiveAssetResolver {
    fn resolver_id(&self) -> &'static str {
        CDSE_ARCHIVE_RESOLVER_ID
    }

    fn supported_schemes(&self) -> &'static [AssetAccessScheme] {
        &[AssetAccessScheme::OauthBearerHttps]
    }

    fn can_resolve(&self, descriptor: &AssetAccessDescriptor) -> bool {
        descriptor.resolver_id == CDSE_ARCHIVE_RESOLVER_ID
            && descriptor.scheme == AssetAccessScheme::OauthBearerHttps
            && descriptor.requires_credentials
            && descriptor.credential_profile.as_deref() == Some(CDSE_CREDENTIAL_PROFILE)
            && descriptor
                .stable_href
                .as_deref()
                .is_some_and(exact_cdse_archive_href)
    }

    fn validate_frozen(&self, frozen: &FrozenAssetAccess) -> Result<(), ResolveError> {
        if frozen.role != AssetRole::Primary
            || !frozen
                .file_name
                .as_deref()
                .is_some_and(safe_cdse_archive_basename)
            || frozen.expected_size_bytes.is_none_or(|size| size == 0)
            || !valid_cdse_checksum(
                frozen.checksum_algorithm.as_deref(),
                frozen.checksum_value.as_deref(),
            )
        {
            return Err(ResolveError::descriptor_invalid());
        }
        Ok(())
    }

    fn resolve(
        &self,
        reference: &StableAssetReference,
        descriptor: &AssetAccessDescriptor,
        context: &ResolveContext<'_>,
    ) -> Result<ResolvedAsset, ResolveError> {
        let expected_href = derive_stable_href(reference)?;
        if !self.can_resolve(descriptor) {
            return Err(ResolveError::descriptor_invalid());
        }
        if descriptor.stable_href.as_deref() != Some(expected_href.as_str()) {
            return Err(ResolveError::access_not_durable());
        }
        context.check_cancelled()?;
        let credential = context
            .credential_provider()
            .resolve(CDSE_CREDENTIAL_PROFILE)
            .map_err(map_credential_error)?;
        context.check_cancelled()?;
        let mut resolved = ResolvedAsset::new(&expected_href, ResolvedMethod::Get)?
            .with_host_policy(ResolvedHostPolicy::CdseDownloadV1);
        credential.apply_bearer_to(&mut resolved)?;
        Ok(resolved)
    }

    fn plan_shadow(
        &self,
        _frozen: &FrozenAssetAccess,
        _context: &ShadowResolveContext<'_>,
    ) -> Result<ShadowAccessIntent, ResolveError> {
        Err(ResolveError::not_configured())
    }
}

fn derive_stable_href(reference: &StableAssetReference) -> Result<String, ResolveError> {
    reference
        .validate()
        .map_err(|_| ResolveError::reference_invalid())?;
    if reference.provider_id != CDSE_PROVIDER_ID
        || !matches!(
            reference.collection_id.as_str(),
            "SENTINEL-2" | "SENTINEL-3"
        )
        || reference.asset_key != CDSE_ARCHIVE_ASSET_KEY
        || !canonical_cdse_odata_uuid(&reference.provider_product_id)
    {
        return Err(ResolveError::reference_invalid());
    }
    let expected_product_key = provider_product_key(
        CDSE_PROVIDER_ID,
        &reference.collection_id,
        &reference.provider_product_id,
    )
    .map_err(|_| ResolveError::reference_invalid())?;
    let expected_asset_key = stable_asset_key(&expected_product_key, CDSE_ARCHIVE_ASSET_KEY)
        .map_err(|_| ResolveError::reference_invalid())?;
    if reference.provider_product_key != expected_product_key
        || reference.stable_asset_key != expected_asset_key
    {
        return Err(ResolveError::reference_invalid());
    }
    stable_cdse_archive_href(&reference.provider_product_id)
        .ok_or_else(ResolveError::reference_invalid)
}

pub(crate) fn canonical_cdse_odata_uuid(value: &str) -> bool {
    Uuid::parse_str(value)
        .map(|parsed| parsed.hyphenated().to_string().eq_ignore_ascii_case(value))
        .unwrap_or(false)
}

pub(crate) fn stable_cdse_archive_href(product_id: &str) -> Option<String> {
    canonical_cdse_odata_uuid(product_id)
        .then(|| format!("{CDSE_DOWNLOAD_ORIGIN}/odata/v1/Products({product_id})/$value"))
}

pub(crate) fn exact_cdse_archive_href(value: &str) -> bool {
    let Some(product_id) = value
        .strip_prefix("https://download.dataspace.copernicus.eu/odata/v1/Products(")
        .and_then(|suffix| suffix.strip_suffix(")/$value"))
    else {
        return false;
    };
    stable_cdse_archive_href(product_id).as_deref() == Some(value)
}

pub(crate) fn safe_cdse_archive_basename(value: &str) -> bool {
    value.strip_suffix(".zip").is_some_and(safe_cdse_odata_name)
}

pub(crate) fn valid_cdse_checksum(algorithm: Option<&str>, value: Option<&str>) -> bool {
    let (Some(algorithm), Some(value)) = (algorithm, value) else {
        return false;
    };
    let expected_length = match algorithm {
        "MD5" => 32,
        "SHA256" => 64,
        _ => return false,
    };
    value.len() == expected_length && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn map_credential_error(error: CredentialProviderError) -> ResolveError {
    match error {
        CredentialProviderError::Missing => ResolveError::credentials_missing(),
        CredentialProviderError::Rejected
        | CredentialProviderError::Failed
        | CredentialProviderError::BurstCookieRequired => ResolveError::auth_failed(),
        CredentialProviderError::Timeout => ResolveError::timeout(),
    }
}
