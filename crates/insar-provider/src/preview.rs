use std::io::Read;
use std::time::Duration;

use insar_core::{
    AssetRole, PreviewAssetRole, PreviewCachePolicy, PreviewCredentialRequirement,
    PreviewDescriptor, PreviewErrorKind, PreviewKind, ProviderAsset,
};
use reqwest::blocking::{Client, Request, Response};
use reqwest::header::{CONTENT_LENGTH, CONTENT_TYPE, LOCATION};
use reqwest::{StatusCode, Url};
use thiserror::Error;

pub const PREVIEW_MAX_BYTES: usize = 16 * 1024 * 1024;
const PREVIEW_MAX_REDIRECTS: usize = 5;
const PREVIEW_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewProductClass {
    Sentinel1Grd,
    NisarCompleteRslc,
    BhoonidhiSsarRslc,
    EarthSearchSentinel2Quicklook,
}

#[derive(Clone, Debug)]
pub struct ResolvedPreviewCandidate {
    pub product_key: String,
    pub descriptor: PreviewDescriptor,
    pub source_asset_role: PreviewAssetRole,
    pub source_asset_key: String,
    pub expected_media_type: String,
    pub url: Url,
    pub product_class: PreviewProductClass,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviewImage {
    pub media_type: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum PreviewAssetError {
    #[error("the Provider has no authoritative Preview asset")]
    ProviderNoPreview,
    #[error("the Preview endpoint returned HTTP 403")]
    Http403,
    #[error("the Preview endpoint returned HTTP 404")]
    Http404,
    #[error("the Preview request timed out or was cancelled")]
    Timeout,
    #[error("the Preview response media type is invalid")]
    InvalidMediaType,
    #[error("the Preview response could not be decoded safely")]
    DecodeFailed,
}

impl PreviewAssetError {
    pub const fn kind(&self) -> PreviewErrorKind {
        match self {
            Self::ProviderNoPreview => PreviewErrorKind::ProviderNoPreview,
            Self::Http403 => PreviewErrorKind::PreviewHttp403,
            Self::Http404 => PreviewErrorKind::PreviewHttp404,
            Self::Timeout => PreviewErrorKind::PreviewTimeout,
            Self::InvalidMediaType => PreviewErrorKind::PreviewInvalidMediaType,
            Self::DecodeFailed => PreviewErrorKind::PreviewDecodeFailed,
        }
    }

    pub const fn code(&self) -> &'static str {
        match self.kind() {
            PreviewErrorKind::ProviderNoPreview => "provider_no_preview",
            PreviewErrorKind::PreviewHttp403 => "preview_http_403",
            PreviewErrorKind::PreviewHttp404 => "preview_http_404",
            PreviewErrorKind::PreviewTimeout => "preview_timeout",
            PreviewErrorKind::PreviewInvalidMediaType => "preview_invalid_media_type",
            PreviewErrorKind::PreviewDecodeFailed => "preview_decode_failed",
        }
    }
}

pub fn resolve_preview_candidate(
    product_key: &str,
    product_class: PreviewProductClass,
    assets: &[ProviderAsset],
) -> Result<ResolvedPreviewCandidate, PreviewAssetError> {
    let (required_role, resolved_role, render_profile_id, provenance) = match product_class {
        PreviewProductClass::Sentinel1Grd => (
            AssetRole::Browse,
            PreviewAssetRole::Browse,
            "asf.sentinel-1.grd-browse-geographic.v1",
            "ASF Sentinel-1 GRD official Browse; desktop-controlled Preview only",
        ),
        PreviewProductClass::NisarCompleteRslc => (
            AssetRole::Preview,
            PreviewAssetRole::Preview,
            "asf.nisar.complete-rslc.main-latlon.v1",
            "ASF NISAR complete-RSLC main LATLON Preview; desktop-controlled Preview only",
        ),
        PreviewProductClass::BhoonidhiSsarRslc => (
            AssetRole::Thumbnail,
            PreviewAssetRole::Thumbnail,
            "bhoonidhi.nisar-sband.thumbnail.v1",
            "Bhoonidhi NISAR S-band catalogue Thumbnail; desktop-controlled Preview only",
        ),
        PreviewProductClass::EarthSearchSentinel2Quicklook => (
            AssetRole::Thumbnail,
            PreviewAssetRole::Thumbnail,
            "earth-search.sentinel-2-l2a.thumbnail.v1",
            "Element 84 Earth Search Sentinel-2 L2A public S3 Quicklook; desktop-controlled Preview only",
        ),
    };
    let candidates = assets
        .iter()
        .filter(|asset| asset.role == required_role)
        .filter_map(|asset| safe_candidate(asset, product_class))
        .collect::<Vec<_>>();
    let [(asset, url, source_asset_key, expected_media_type)] = candidates.as_slice() else {
        return Err(PreviewAssetError::ProviderNoPreview);
    };
    let descriptor = PreviewDescriptor {
        preview_kind: PreviewKind::Quicklook,
        source_asset_keys: vec![source_asset_key.clone()],
        render_profile_id: render_profile_id.to_owned(),
        media_type: expected_media_type.clone(),
        credential_requirement: PreviewCredentialRequirement::None,
        cache_policy: PreviewCachePolicy::Session,
        provenance: provenance.to_owned(),
    };
    descriptor
        .validate()
        .map_err(|_| PreviewAssetError::ProviderNoPreview)?;
    debug_assert_eq!(asset.role, required_role);
    Ok(ResolvedPreviewCandidate {
        product_key: product_key.to_owned(),
        descriptor,
        source_asset_role: resolved_role,
        source_asset_key: source_asset_key.clone(),
        expected_media_type: expected_media_type.clone(),
        url: url.clone(),
        product_class,
    })
}

fn safe_candidate(
    asset: &ProviderAsset,
    product_class: PreviewProductClass,
) -> Option<(&ProviderAsset, Url, String, String)> {
    if asset.access_descriptor.is_some() || asset.native_roles.is_empty() {
        return None;
    }
    let source_asset_key = asset
        .stable_asset_key
        .as_deref()
        .or(asset.asset_key.as_deref())?
        .trim();
    if source_asset_key.is_empty() || source_asset_key.chars().any(char::is_control) {
        return None;
    }
    let expected_media_type = normalized_image_media_type(asset.media_type.as_deref()?)?;
    let url = Url::parse(asset.url.as_deref()?).ok()?;
    if !safe_initial_url(&url, product_class) {
        return None;
    }
    Some((
        asset,
        url,
        source_asset_key.to_owned(),
        expected_media_type.to_owned(),
    ))
}

pub struct PreviewHttpClient {
    client: Client,
}

impl PreviewHttpClient {
    pub fn new() -> Result<Self, PreviewAssetError> {
        Self::with_route(insar_core::ProxyPolicy::System)
    }

    /// A client on the saved route of the preview's data source.
    pub fn with_route(route: insar_core::ProxyPolicy) -> Result<Self, PreviewAssetError> {
        let builder = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(PREVIEW_TIMEOUT);
        let client = crate::network_proxy::blocking_builder_with_proxy(builder, route)
            .build()
            .map_err(|_| PreviewAssetError::DecodeFailed)?;
        Ok(Self { client })
    }

    pub fn fetch(
        &self,
        candidate: &ResolvedPreviewCandidate,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<PreviewImage, PreviewAssetError> {
        if cancelled() {
            return Err(PreviewAssetError::Timeout);
        }
        let mut current = candidate.url.clone();
        for redirect_count in 0..=PREVIEW_MAX_REDIRECTS {
            let request = build_preview_request(
                &self.client,
                current.clone(),
                &candidate.expected_media_type,
            )?;
            let response = self.client.execute(request).map_err(transport_error)?;
            if response.status().is_redirection() {
                if redirect_count == PREVIEW_MAX_REDIRECTS {
                    return Err(PreviewAssetError::DecodeFailed);
                }
                let location = response
                    .headers()
                    .get(LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .ok_or(PreviewAssetError::DecodeFailed)?;
                let next = current
                    .join(location)
                    .map_err(|_| PreviewAssetError::DecodeFailed)?;
                if !safe_redirect_url(&next, candidate.product_class) {
                    return Err(PreviewAssetError::DecodeFailed);
                }
                current = next;
                continue;
            }
            return read_preview_response(response, &candidate.expected_media_type, cancelled);
        }
        Err(PreviewAssetError::DecodeFailed)
    }
}

fn build_preview_request(
    client: &Client,
    url: Url,
    expected_media_type: &str,
) -> Result<Request, PreviewAssetError> {
    client
        .get(url)
        .header(reqwest::header::ACCEPT, expected_media_type)
        .build()
        .map_err(|_| PreviewAssetError::DecodeFailed)
}

fn response_status_error(status: StatusCode) -> Option<PreviewAssetError> {
    match status {
        StatusCode::FORBIDDEN => Some(PreviewAssetError::Http403),
        StatusCode::NOT_FOUND => Some(PreviewAssetError::Http404),
        status if !status.is_success() => Some(PreviewAssetError::DecodeFailed),
        _ => None,
    }
}

fn response_media_type_allowed(response_media_type: &str, expected_media_type: &str) -> bool {
    response_media_type == expected_media_type
        || response_media_type == "application/octet-stream"
        || response_media_type == "binary/octet-stream"
}

fn declared_length_exceeds_cap(length: Option<u64>) -> bool {
    length.is_some_and(|value| value > PREVIEW_MAX_BYTES as u64)
}

fn read_preview_response(
    mut response: Response,
    expected_media_type: &str,
    cancelled: &dyn Fn() -> bool,
) -> Result<PreviewImage, PreviewAssetError> {
    if let Some(error) = response_status_error(response.status()) {
        return Err(error);
    }
    if declared_length_exceeds_cap(
        response
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok()),
    ) {
        return Err(PreviewAssetError::DecodeFailed);
    }
    let response_media_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .ok_or(PreviewAssetError::InvalidMediaType)?;
    if !response_media_type_allowed(&response_media_type, expected_media_type) {
        return Err(PreviewAssetError::InvalidMediaType);
    }
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 16 * 1024];
    loop {
        if cancelled() {
            return Err(PreviewAssetError::Timeout);
        }
        let count = response.read(&mut chunk).map_err(transport_error_io)?;
        if count == 0 {
            break;
        }
        if bytes.len().saturating_add(count) > PREVIEW_MAX_BYTES {
            return Err(PreviewAssetError::DecodeFailed);
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    if !image_magic_matches(expected_media_type, &bytes) {
        return Err(PreviewAssetError::DecodeFailed);
    }
    Ok(PreviewImage {
        media_type: expected_media_type.to_owned(),
        bytes,
    })
}

fn normalized_image_media_type(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "image/jpeg" => Some("image/jpeg"),
        "image/png" => Some("image/png"),
        "image/webp" => Some("image/webp"),
        _ => None,
    }
}

fn image_magic_matches(media_type: &str, bytes: &[u8]) -> bool {
    match media_type {
        "image/jpeg" => bytes.starts_with(&[0xff, 0xd8, 0xff]),
        "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/webp" => bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP",
        _ => false,
    }
}

fn safe_initial_url(url: &Url, product_class: PreviewProductClass) -> bool {
    safe_url_authority(url)
        && url.query().is_none()
        && url.fragment().is_none()
        && match product_class {
            PreviewProductClass::Sentinel1Grd => url.host_str().is_some_and(is_asf_host),
            PreviewProductClass::NisarCompleteRslc => {
                url.host_str() == Some("nisar.asf.earthdatacloud.nasa.gov")
            }
            PreviewProductClass::BhoonidhiSsarRslc => {
                url.host_str() == Some("bhoonidhi.nrsc.gov.in")
            }
            PreviewProductClass::EarthSearchSentinel2Quicklook => {
                url.host_str() == Some("sentinel-cogs.s3.us-west-2.amazonaws.com")
                    && url.path().starts_with("/sentinel-s2-l2a-cogs/")
                    && url.path().ends_with(".jpg")
            }
        }
}

fn safe_redirect_url(url: &Url, product_class: PreviewProductClass) -> bool {
    safe_url_authority(url)
        && url.fragment().is_none()
        && url.host_str().is_some_and(|host| match product_class {
            PreviewProductClass::Sentinel1Grd => is_asf_host(host) || is_cloudfront_host(host),
            PreviewProductClass::NisarCompleteRslc => {
                host == "nisar.asf.earthdatacloud.nasa.gov" || is_cloudfront_host(host)
            }
            PreviewProductClass::BhoonidhiSsarRslc => host == "bhoonidhi.nrsc.gov.in",
            PreviewProductClass::EarthSearchSentinel2Quicklook => {
                host == "sentinel-cogs.s3.us-west-2.amazonaws.com"
                    && url.path().starts_with("/sentinel-s2-l2a-cogs/")
                    && url.path().ends_with(".jpg")
            }
        })
}

fn safe_url_authority(url: &Url) -> bool {
    url.scheme() == "https"
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.host_str().is_some()
}

fn is_asf_host(host: &str) -> bool {
    host == "asf.alaska.edu" || host.ends_with(".asf.alaska.edu")
}

fn is_cloudfront_host(host: &str) -> bool {
    host.ends_with(".cloudfront.net") && host.len() > ".cloudfront.net".len()
}

fn transport_error(error: reqwest::Error) -> PreviewAssetError {
    if error.is_timeout() {
        PreviewAssetError::Timeout
    } else {
        PreviewAssetError::DecodeFailed
    }
}

fn transport_error_io(error: std::io::Error) -> PreviewAssetError {
    if matches!(
        error.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    ) {
        PreviewAssetError::Timeout
    } else {
        PreviewAssetError::DecodeFailed
    }
}

#[cfg(test)]
mod tests {
    use super::{
        build_preview_request, declared_length_exceeds_cap, image_magic_matches,
        response_media_type_allowed, response_status_error, safe_initial_url, safe_redirect_url,
        PreviewAssetError, PreviewProductClass, PREVIEW_MAX_BYTES,
    };
    use reqwest::blocking::Client;
    use reqwest::header::{ACCEPT, AUTHORIZATION};
    use reqwest::{StatusCode, Url};

    #[test]
    fn redirect_policy_accepts_only_reviewed_https_provider_and_cloudfront_hosts() {
        let sentinel = PreviewProductClass::Sentinel1Grd;
        assert!(safe_initial_url(
            &Url::parse("https://datapool.asf.alaska.edu/path/browse.jpg").unwrap(),
            sentinel
        ));
        assert!(safe_redirect_url(
            &Url::parse("https://sentinel1.asf.alaska.edu/path/browse.jpg").unwrap(),
            sentinel
        ));
        assert!(safe_redirect_url(
            &Url::parse("https://example.cloudfront.net/path?signature=redacted").unwrap(),
            sentinel
        ));
        for unsafe_url in [
            "http://datapool.asf.alaska.edu/path/browse.jpg",
            "https://datapool.asf.alaska.edu.evil.example/path/browse.jpg",
            "https://user@datapool.asf.alaska.edu/path/browse.jpg",
            "https://example.invalid/path/browse.jpg",
        ] {
            assert!(!safe_redirect_url(
                &Url::parse(unsafe_url).unwrap(),
                sentinel
            ));
        }

        let bhoonidhi = PreviewProductClass::BhoonidhiSsarRslc;
        assert!(safe_initial_url(
            &Url::parse("https://bhoonidhi.nrsc.gov.in/stac/NISAR/item.jpg").unwrap(),
            bhoonidhi
        ));
        assert!(!safe_initial_url(
            &Url::parse("https://bhoonidhi-api.nrsc.gov.in/stac/NISAR/item.jpg").unwrap(),
            bhoonidhi
        ));
    }

    #[test]
    fn controlled_fetch_contract_is_typed_bounded_and_credential_free() {
        assert_eq!(
            response_status_error(StatusCode::FORBIDDEN),
            Some(PreviewAssetError::Http403)
        );
        assert_eq!(
            response_status_error(StatusCode::NOT_FOUND),
            Some(PreviewAssetError::Http404)
        );
        assert_eq!(response_status_error(StatusCode::OK), None);
        assert!(response_media_type_allowed("image/jpeg", "image/jpeg"));
        assert!(response_media_type_allowed(
            "application/octet-stream",
            "image/png"
        ));
        assert!(response_media_type_allowed(
            "binary/octet-stream",
            "image/png"
        ));
        assert!(!response_media_type_allowed("text/html", "image/png"));
        assert!(!declared_length_exceeds_cap(Some(PREVIEW_MAX_BYTES as u64)));
        assert!(declared_length_exceeds_cap(Some(
            PREVIEW_MAX_BYTES as u64 + 1
        )));

        let request = build_preview_request(
            &Client::new(),
            Url::parse("https://datapool.asf.alaska.edu/path/browse.jpg").unwrap(),
            "image/jpeg",
        )
        .unwrap();
        assert_eq!(request.headers().get(ACCEPT).unwrap(), "image/jpeg");
        assert!(request.headers().get(AUTHORIZATION).is_none());
    }

    #[test]
    fn media_magic_is_exact_and_size_cap_is_sixteen_mib() {
        assert_eq!(PREVIEW_MAX_BYTES, 16 * 1024 * 1024);
        assert!(image_magic_matches("image/jpeg", &[0xff, 0xd8, 0xff, 0xe0]));
        assert!(image_magic_matches("image/png", b"\x89PNG\r\n\x1a\nrest"));
        assert!(image_magic_matches(
            "image/webp",
            b"RIFF\x04\x00\x00\x00WEBP"
        ));
        assert!(!image_magic_matches("image/png", b"not-an-image"));
        assert!(!image_magic_matches("image/tiff", b"II*\0"));
    }
}
