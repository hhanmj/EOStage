use std::fmt;

use insar_core::{AssetAccessDescriptor, ProviderAsset};
use reqwest::Url;

use super::ResolveError;

#[derive(Debug)]
pub enum AssetAccessClassification<'a> {
    Durable(&'a AssetAccessDescriptor),
    LegacyAdapter(LegacyAdapter<'a>),
}

#[derive(Clone, Copy)]
pub struct LegacyAdapter<'a> {
    url: &'a str,
}

impl LegacyAdapter<'_> {
    pub fn url(&self) -> &str {
        self.url
    }
}

impl fmt::Debug for LegacyAdapter<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyAdapter")
            .field("url", &"[LEGACY_URL_REDACTED]")
            .finish()
    }
}

/// Classify one persisted Provider asset without executing either access path.
///
/// A present descriptor is authoritative. Invalid descriptors fail closed and
/// never fall through to a legacy URL. URL-only records remain explicitly
/// marked as legacy so old immutable snapshots stay readable.
pub fn classify_asset_access(
    asset: &ProviderAsset,
) -> Result<AssetAccessClassification<'_>, ResolveError> {
    if let Some(descriptor) = asset.access_descriptor.as_ref() {
        descriptor
            .validate()
            .map_err(|_| ResolveError::descriptor_invalid())?;
        if asset.url.as_deref().is_some_and(is_secret_fallback) {
            return Err(ResolveError::access_not_durable());
        }
        return Ok(AssetAccessClassification::Durable(descriptor));
    }

    let url = asset
        .url
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(ResolveError::reference_invalid)?;
    Ok(AssetAccessClassification::LegacyAdapter(LegacyAdapter {
        url,
    }))
}

fn is_secret_fallback(value: &str) -> bool {
    let Ok(url) = Url::parse(value) else {
        return true;
    };
    !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
}
