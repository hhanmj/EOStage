use std::fs;

use insar_core::{
    AssetAccessDescriptor, AssetAccessScheme, FrozenAssetAccess, StableAssetReference,
};
use reqwest::Url;

use super::shadow_plan::plan_local_file;
use super::{
    AssetResolver, ResolveContext, ResolveError, ResolvedAsset, ResolvedMethod, ShadowAccessIntent,
    ShadowResolveContext,
};

pub struct LocalFileAssetResolver;

impl LocalFileAssetResolver {
    pub const fn new() -> Self {
        Self
    }
}

impl Default for LocalFileAssetResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl AssetResolver for LocalFileAssetResolver {
    fn resolver_id(&self) -> &'static str {
        "local_file"
    }

    fn supported_schemes(&self) -> &'static [AssetAccessScheme] {
        &[AssetAccessScheme::LocalFile]
    }

    fn can_resolve(&self, descriptor: &AssetAccessDescriptor) -> bool {
        descriptor.scheme == AssetAccessScheme::LocalFile
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
        if descriptor.scheme != AssetAccessScheme::LocalFile
            || descriptor.requires_credentials
            || descriptor.credential_profile.is_some()
        {
            return Err(ResolveError::descriptor_invalid());
        }
        context.check_cancelled()?;
        let href = descriptor
            .stable_href
            .as_deref()
            .ok_or_else(ResolveError::reference_invalid)?;
        let url = Url::parse(href).map_err(|_| ResolveError::reference_invalid())?;
        if url.scheme() != "file"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(ResolveError::reference_invalid());
        }
        let path = url
            .to_file_path()
            .map_err(|_| ResolveError::reference_invalid())?;
        if !path.is_absolute() {
            return Err(ResolveError::reference_invalid());
        }
        let metadata = fs::metadata(&path).map_err(|_| ResolveError::reference_invalid())?;
        if !metadata.is_file() {
            return Err(ResolveError::reference_invalid());
        }
        let suggested_filename = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_string);
        ResolvedAsset::new(url.as_str(), ResolvedMethod::Get).map(|asset| {
            asset
                .with_expected_size(Some(metadata.len()))
                .with_suggested_filename(suggested_filename)
        })
    }

    fn plan_shadow(
        &self,
        frozen: &FrozenAssetAccess,
        context: &ShadowResolveContext<'_>,
    ) -> Result<ShadowAccessIntent, ResolveError> {
        if frozen.descriptor.scheme != AssetAccessScheme::LocalFile
            || frozen.descriptor.requires_credentials
            || frozen.descriptor.credential_profile.is_some()
        {
            return Err(ResolveError::descriptor_invalid());
        }
        plan_local_file(frozen, context)
    }
}
