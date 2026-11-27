//! Bounded, credential-free discovery and transport for one reviewed CDSE
//! quicklook asset.
//!
//! This module intentionally owns no durable cache, catalogue state, session,
//! or archive-download behavior. A caller supplies a frozen product identity;
//! asset evidence and the resulting image remain runtime-only.

use std::fmt;
use std::io::Read;
use std::time::Duration;

use insar_core::ProviderNetworkPolicy;
use insar_provider::cdse::odata_mapper::{select_quicklook_descriptor, CdseQuicklookDescriptor};
use insar_provider::cdse::odata_types::ODataProduct;
use reqwest::blocking::{Client, Response};
use reqwest::header::{CONTENT_LENGTH, CONTENT_TYPE, LOCATION};
use reqwest::redirect::Policy;
use reqwest::Url;
use uuid::Uuid;

/// Exact CDSE catalogue origin approved for static quicklook retrieval.
pub const CDSE_QUICKLOOK_ORIGIN: &str = "https://catalogue.dataspace.copernicus.eu/odata/v1/";

/// Maximum runtime-only preview body. This matches the bounded quicklook size
/// accepted by the CDSE OData mapper.
pub const CDSE_QUICKLOOK_MAX_BYTES: usize = 16 * 1024 * 1024;

const READ_CHUNK_BYTES: usize = 8 * 1024;
const CDSE_QUICKLOOK_DISCOVERY_MAX_BYTES: usize = 1024 * 1024;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(45);
const CATALOGUE_HOST: &str = "catalogue.dataspace.copernicus.eu";
const DOWNLOAD_HOST: &str = "download.dataspace.copernicus.eu";
const DOWNLOAD_ORIGIN: &str = "https://download.dataspace.copernicus.eu/odata/v1/";

/// Coarse, redaction-safe failure classes for a CDSE quicklook request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CdseQuicklookError {
    ConfigurationInvalid,
    AssetIdentityInvalid,
    ProductIdentityInvalid,
    DiscoveryResponseRejected,
    ExpectedMediaTypeInvalid,
    ExpectedLengthInvalid,
    Cancelled,
    Transport,
    RedirectRejected,
    HttpStatusRejected,
    EffectiveUrlRejected,
    MediaTypeRejected,
    BodyMalformed,
    ImageSignatureRejected,
    BodyTooLarge,
    BodyEmpty,
    LengthMismatch,
}

impl CdseQuicklookError {
    /// Stable, safe-to-surface error code. It deliberately carries no URL,
    /// response body, header value, credential, or proxy detail.
    pub const fn code(self) -> &'static str {
        match self {
            Self::ConfigurationInvalid => "CDSE_QUICKLOOK_CONFIGURATION_INVALID",
            Self::AssetIdentityInvalid => "CDSE_QUICKLOOK_ASSET_INVALID",
            Self::ProductIdentityInvalid => "CDSE_QUICKLOOK_PRODUCT_INVALID",
            Self::DiscoveryResponseRejected => "CDSE_QUICKLOOK_DISCOVERY_INVALID",
            Self::ExpectedMediaTypeInvalid => "CDSE_QUICKLOOK_MEDIA_TYPE_INVALID",
            Self::ExpectedLengthInvalid => "CDSE_QUICKLOOK_LENGTH_INVALID",
            Self::Cancelled => "CDSE_QUICKLOOK_CANCELLED",
            Self::Transport => "CDSE_QUICKLOOK_HTTP_FAILURE",
            Self::RedirectRejected => "CDSE_QUICKLOOK_REDIRECT_REJECTED",
            Self::HttpStatusRejected => "CDSE_QUICKLOOK_HTTP_STATUS_REJECTED",
            Self::EffectiveUrlRejected => "CDSE_QUICKLOOK_EFFECTIVE_URL_REJECTED",
            Self::MediaTypeRejected => "CDSE_QUICKLOOK_MEDIA_TYPE_REJECTED",
            Self::BodyMalformed => "CDSE_QUICKLOOK_BODY_MALFORMED",
            Self::ImageSignatureRejected => "CDSE_QUICKLOOK_IMAGE_SIGNATURE_REJECTED",
            Self::BodyTooLarge => "CDSE_QUICKLOOK_BODY_TOO_LARGE",
            Self::BodyEmpty => "CDSE_QUICKLOOK_BODY_EMPTY",
            Self::LengthMismatch => "CDSE_QUICKLOOK_LENGTH_MISMATCH",
        }
    }
}

impl fmt::Display for CdseQuicklookError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for CdseQuicklookError {}

/// One bounded image kept exclusively in runtime memory.
pub struct CdseQuicklookImage {
    media_type: String,
    bytes: Vec<u8>,
}

impl CdseQuicklookImage {
    pub fn media_type(&self) -> &str {
        &self.media_type
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Borrows image bytes without introducing serialization or persistence.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Moves image bytes to a caller-owned, still runtime-only buffer.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

impl fmt::Debug for CdseQuicklookImage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CdseQuicklookImage")
            .field("media_type", &self.media_type)
            .field("length", &self.bytes.len())
            .field("bytes", &"[REDACTED]")
            .finish()
    }
}

/// Fixed-origin, credential-free CDSE quicklook client.
///
/// Construction performs no network I/O. `System` inherits operating-system
/// networking; explicit `Direct` bypasses system proxy configuration. Neither
/// mode accepts proxy credentials, cookies, authorization input, redirects, or
/// an arbitrary endpoint.
pub struct CdseQuicklookClient {
    client: Client,
    network_policy: ProviderNetworkPolicy,
    #[cfg(test)]
    fixture_origin: Option<Url>,
}

impl CdseQuicklookClient {
    pub fn new(network_policy: ProviderNetworkPolicy) -> Result<Self, CdseQuicklookError> {
        Self::with_timeout(network_policy, DEFAULT_TIMEOUT)
    }

    pub fn with_timeout(
        network_policy: ProviderNetworkPolicy,
        timeout: Duration,
    ) -> Result<Self, CdseQuicklookError> {
        if timeout.is_zero() {
            return Err(CdseQuicklookError::ConfigurationInvalid);
        }
        validate_catalogue_origin(&production_origin()?)?;
        let client = Client::builder()
            .redirect(Policy::none())
            .retry(reqwest::retry::never())
            .cookie_store(false)
            .referer(false)
            .connect_timeout(timeout)
            .timeout(timeout);
        let client = insar_provider::network_proxy::blocking_builder_with_proxy(
            client,
            network_policy.route(),
        )
        .build()
        .map_err(|_| CdseQuicklookError::ConfigurationInvalid)?;
        Ok(Self {
            client,
            network_policy,
            #[cfg(test)]
            fixture_origin: None,
        })
    }

    pub const fn network_policy(&self) -> ProviderNetworkPolicy {
        self.network_policy
    }

    /// Resolves one optional static quicklook from an exact frozen product UUID.
    ///
    /// This request is anonymous, fixed to the approved catalogue origin, and
    /// accepts no caller-supplied URL, download link, or asset identifier.
    /// Missing Assets are a normal unavailable-preview result. Malformed or
    /// conflicting asset evidence is confined to this preview operation.
    pub fn discover(
        &self,
        product_id: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<Option<CdseQuicklookDescriptor>, CdseQuicklookError> {
        let canonical_product_id = canonical_product_uuid(product_id)?;
        let endpoint = self.product_assets_endpoint_for(&canonical_product_id)?;
        if cancelled() {
            return Err(CdseQuicklookError::Cancelled);
        }
        let response = self
            .client
            .get(endpoint.clone())
            .send()
            .map_err(|_| CdseQuicklookError::Transport)?;
        if cancelled() {
            return Err(CdseQuicklookError::Cancelled);
        }
        if response.url() != &endpoint {
            return Err(CdseQuicklookError::EffectiveUrlRejected);
        }
        if response.status().is_redirection() {
            return Err(CdseQuicklookError::RedirectRejected);
        }
        if !response.status().is_success() {
            return Err(CdseQuicklookError::HttpStatusRejected);
        }
        let body = read_bounded_body(response, cancelled, CDSE_QUICKLOOK_DISCOVERY_MAX_BYTES)?;
        let product = serde_json::from_slice::<ODataProduct>(&body)
            .map_err(|_| CdseQuicklookError::DiscoveryResponseRejected)?;
        let returned_id = product
            .id
            .as_deref()
            .ok_or(CdseQuicklookError::DiscoveryResponseRejected)
            .and_then(canonical_product_uuid)?;
        if returned_id != canonical_product_id {
            return Err(CdseQuicklookError::DiscoveryResponseRejected);
        }
        select_quicklook_descriptor(&product.assets)
            .map_err(|_| CdseQuicklookError::DiscoveryResponseRejected)
    }

    /// Fetches exactly one frozen quicklook without authentication.
    ///
    /// `asset_id` must already be a canonical lowercase hyphenated UUID.
    /// `expected_media_type` must be one of the reviewed image media types.
    /// `expected_length`, when frozen by the catalogue, is checked against the
    /// received body even when the response omits `Content-Length`.
    pub fn fetch(
        &self,
        asset_id: &str,
        expected_media_type: &str,
        expected_length: Option<u64>,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<CdseQuicklookImage, CdseQuicklookError> {
        let endpoint = self.endpoint_for(asset_id)?;
        let expected_media_type = validate_expected_media_type(expected_media_type)?;
        let expected_length = validate_expected_length(expected_length)?;
        if cancelled() {
            return Err(CdseQuicklookError::Cancelled);
        }

        let mut response = self
            .client
            .get(endpoint.clone())
            .send()
            .map_err(|_| CdseQuicklookError::Transport)?;
        if cancelled() {
            return Err(CdseQuicklookError::Cancelled);
        }
        if response.url() != &endpoint {
            return Err(CdseQuicklookError::EffectiveUrlRejected);
        }
        if response.status().is_redirection() {
            let redirect = self.reviewed_download_redirect(asset_id, &response)?;
            response = self
                .client
                .get(redirect.clone())
                .send()
                .map_err(|_| CdseQuicklookError::Transport)?;
            if cancelled() {
                return Err(CdseQuicklookError::Cancelled);
            }
            if response.url() != &redirect {
                return Err(CdseQuicklookError::EffectiveUrlRejected);
            }
            if response.status().is_redirection() {
                return Err(CdseQuicklookError::RedirectRejected);
            }
        }
        if !response.status().is_success() {
            return Err(CdseQuicklookError::HttpStatusRejected);
        }
        let response_media_type = reviewed_response_media_type(&response, expected_media_type)?;
        let declared_length = response_content_length(&response)?;
        if declared_length.is_some_and(|length| length > CDSE_QUICKLOOK_MAX_BYTES) {
            return Err(CdseQuicklookError::BodyTooLarge);
        }

        let bytes = read_bounded_body(response, cancelled, CDSE_QUICKLOOK_MAX_BYTES)?;
        if bytes.is_empty() {
            return Err(CdseQuicklookError::BodyEmpty);
        }
        if declared_length.is_some_and(|length| length != bytes.len()) {
            return Err(CdseQuicklookError::BodyMalformed);
        }
        if expected_length.is_some_and(|length| length != bytes.len() as u64) {
            return Err(CdseQuicklookError::LengthMismatch);
        }
        let media_type = match response_media_type {
            Some(media_type) => {
                if !has_expected_image_signature(media_type, &bytes) {
                    return Err(CdseQuicklookError::ImageSignatureRejected);
                }
                media_type
            }
            None => infer_reviewed_image_media_type(&bytes)
                .ok_or(CdseQuicklookError::ImageSignatureRejected)?,
        };
        Ok(CdseQuicklookImage {
            media_type: media_type.to_owned(),
            bytes,
        })
    }

    fn reviewed_download_redirect(
        &self,
        asset_id: &str,
        response: &Response,
    ) -> Result<Url, CdseQuicklookError> {
        if !matches!(response.status().as_u16(), 301 | 302 | 307 | 308) {
            return Err(CdseQuicklookError::RedirectRejected);
        }
        let values = response.headers().get_all(LOCATION);
        let mut values = values.iter();
        let location = values
            .next()
            .ok_or(CdseQuicklookError::RedirectRejected)?
            .to_str()
            .map_err(|_| CdseQuicklookError::RedirectRejected)?;
        if values.next().is_some() {
            return Err(CdseQuicklookError::RedirectRejected);
        }
        let parsed = Url::parse(location).map_err(|_| CdseQuicklookError::RedirectRejected)?;
        let expected = self.download_endpoint_for(asset_id)?;
        if parsed != expected {
            return Err(CdseQuicklookError::RedirectRejected);
        }
        Ok(expected)
    }

    fn download_endpoint_for(&self, asset_id: &str) -> Result<Url, CdseQuicklookError> {
        let uuid = canonical_lowercase_uuid(asset_id)?;
        let resource_path = format!("Assets({uuid})/$value");
        #[cfg(test)]
        if let Some(origin) = &self.fixture_origin {
            return origin
                .join(&format!("/odata/v1/{resource_path}"))
                .map_err(|_| CdseQuicklookError::ConfigurationInvalid);
        }
        let origin = production_download_origin()?;
        validate_download_origin(&origin)?;
        origin
            .join(&resource_path)
            .map_err(|_| CdseQuicklookError::ConfigurationInvalid)
    }

    fn endpoint_for(&self, asset_id: &str) -> Result<Url, CdseQuicklookError> {
        let uuid = canonical_lowercase_uuid(asset_id)?;
        let resource_path = format!("Assets({uuid})/$value");
        #[cfg(test)]
        if let Some(origin) = &self.fixture_origin {
            return origin
                .join(&format!("/odata/v1/{resource_path}"))
                .map_err(|_| CdseQuicklookError::ConfigurationInvalid);
        }
        production_origin()?
            .join(&resource_path)
            .map_err(|_| CdseQuicklookError::ConfigurationInvalid)
    }

    fn product_assets_endpoint_for(&self, product_id: &str) -> Result<Url, CdseQuicklookError> {
        let product_id = canonical_product_uuid(product_id)?;
        let resource_path = format!("Products({product_id})");
        let mut endpoint = {
            #[cfg(test)]
            if let Some(origin) = &self.fixture_origin {
                origin
                    .join(&format!("/odata/v1/{resource_path}"))
                    .map_err(|_| CdseQuicklookError::ConfigurationInvalid)?
            } else {
                production_origin()?
                    .join(&resource_path)
                    .map_err(|_| CdseQuicklookError::ConfigurationInvalid)?
            }
            #[cfg(not(test))]
            {
                production_origin()?
                    .join(&resource_path)
                    .map_err(|_| CdseQuicklookError::ConfigurationInvalid)?
            }
        };
        endpoint.query_pairs_mut().append_pair("$expand", "Assets");
        Ok(endpoint)
    }

    /// Test-only loopback retargeting. Production builds cannot compile this
    /// method and always retain the exact HTTPS catalogue origin.
    #[cfg(test)]
    pub(crate) fn retarget_for_loopback_fixture(
        &mut self,
        origin: &str,
    ) -> Result<(), CdseQuicklookError> {
        let origin = Url::parse(origin).map_err(|_| CdseQuicklookError::ConfigurationInvalid)?;
        validate_loopback_fixture_origin(&origin)?;
        self.fixture_origin = Some(origin);
        Ok(())
    }
}

/// A response header alone is not sufficient evidence that an untrusted body
/// is a reviewed image. These fixed magic values reject HTML/JSON/error bodies
/// labelled as image data without adding an image decoder or durable cache.
fn infer_reviewed_image_media_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"\x89PNG\r\n\x1A\n") {
        Some("image/png")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

fn has_expected_image_signature(media_type: &str, bytes: &[u8]) -> bool {
    infer_reviewed_image_media_type(bytes) == Some(media_type)
}

impl fmt::Debug for CdseQuicklookClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CdseQuicklookClient")
            .field("network_policy", &self.network_policy)
            .field("endpoint", &"[REDACTED]")
            .finish()
    }
}

fn production_origin() -> Result<Url, CdseQuicklookError> {
    Url::parse(CDSE_QUICKLOOK_ORIGIN).map_err(|_| CdseQuicklookError::ConfigurationInvalid)
}

fn production_download_origin() -> Result<Url, CdseQuicklookError> {
    Url::parse(DOWNLOAD_ORIGIN).map_err(|_| CdseQuicklookError::ConfigurationInvalid)
}

fn validate_download_origin(origin: &Url) -> Result<(), CdseQuicklookError> {
    if origin.scheme() != "https"
        || origin.host_str() != Some(DOWNLOAD_HOST)
        || origin.port_or_known_default() != Some(443)
        || origin.path() != "/odata/v1/"
        || !origin.username().is_empty()
        || origin.password().is_some()
        || origin.query().is_some()
        || origin.fragment().is_some()
    {
        return Err(CdseQuicklookError::ConfigurationInvalid);
    }
    Ok(())
}

fn validate_catalogue_origin(origin: &Url) -> Result<(), CdseQuicklookError> {
    if origin.scheme() != "https"
        || origin.host_str() != Some(CATALOGUE_HOST)
        || origin.port_or_known_default() != Some(443)
        || origin.path() != "/odata/v1/"
        || !origin.username().is_empty()
        || origin.password().is_some()
        || origin.query().is_some()
        || origin.fragment().is_some()
    {
        return Err(CdseQuicklookError::ConfigurationInvalid);
    }
    Ok(())
}

fn canonical_lowercase_uuid(value: &str) -> Result<String, CdseQuicklookError> {
    let uuid = Uuid::parse_str(value).map_err(|_| CdseQuicklookError::AssetIdentityInvalid)?;
    let canonical = uuid.hyphenated().to_string();
    if value != canonical {
        return Err(CdseQuicklookError::AssetIdentityInvalid);
    }
    Ok(canonical)
}

fn canonical_product_uuid(value: &str) -> Result<String, CdseQuicklookError> {
    let uuid = Uuid::parse_str(value).map_err(|_| CdseQuicklookError::ProductIdentityInvalid)?;
    let canonical = uuid.hyphenated().to_string();
    if !value.eq_ignore_ascii_case(&canonical) {
        return Err(CdseQuicklookError::ProductIdentityInvalid);
    }
    Ok(canonical)
}

fn validate_expected_media_type(value: &str) -> Result<&str, CdseQuicklookError> {
    match value {
        "image/jpeg" | "image/png" | "image/webp" | "image/*" => Ok(value),
        _ => Err(CdseQuicklookError::ExpectedMediaTypeInvalid),
    }
}

fn validate_expected_length(value: Option<u64>) -> Result<Option<u64>, CdseQuicklookError> {
    if value.is_some_and(|length| length == 0 || length > CDSE_QUICKLOOK_MAX_BYTES as u64) {
        Err(CdseQuicklookError::ExpectedLengthInvalid)
    } else {
        Ok(value)
    }
}

fn reviewed_response_media_type(
    response: &Response,
    expected_media_type: &str,
) -> Result<Option<&'static str>, CdseQuicklookError> {
    let values = response.headers().get_all(CONTENT_TYPE);
    let mut values = values.iter();
    let actual = values.next().ok_or(CdseQuicklookError::MediaTypeRejected)?;
    if values.next().is_some() {
        return Err(CdseQuicklookError::MediaTypeRejected);
    }
    let actual = actual
        .to_str()
        .map_err(|_| CdseQuicklookError::MediaTypeRejected)?;
    let reviewed = match actual {
        "image/jpeg" => Some("image/jpeg"),
        "image/png" => Some("image/png"),
        "image/webp" => Some("image/webp"),
        "application/octet-stream" if expected_media_type == "image/*" => None,
        _ => return Err(CdseQuicklookError::MediaTypeRejected),
    };
    if expected_media_type != "image/*" && reviewed != Some(expected_media_type) {
        return Err(CdseQuicklookError::MediaTypeRejected);
    }
    Ok(reviewed)
}

fn response_content_length(response: &Response) -> Result<Option<usize>, CdseQuicklookError> {
    let values = response.headers().get_all(CONTENT_LENGTH);
    let mut values = values.iter();
    let Some(value) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Err(CdseQuicklookError::BodyMalformed);
    }
    let value = value
        .to_str()
        .map_err(|_| CdseQuicklookError::BodyMalformed)?;
    let value = value
        .parse::<usize>()
        .map_err(|_| CdseQuicklookError::BodyMalformed)?;
    Ok(Some(value))
}

fn read_bounded_body(
    mut response: Response,
    cancelled: &(dyn Fn() -> bool + Sync),
    maximum_bytes: usize,
) -> Result<Vec<u8>, CdseQuicklookError> {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; READ_CHUNK_BYTES];
    loop {
        if cancelled() {
            return Err(CdseQuicklookError::Cancelled);
        }
        let count = response
            .read(&mut chunk)
            .map_err(|_| CdseQuicklookError::BodyMalformed)?;
        if cancelled() {
            return Err(CdseQuicklookError::Cancelled);
        }
        if count == 0 {
            return Ok(bytes);
        }
        if bytes.len().saturating_add(count) > maximum_bytes {
            return Err(CdseQuicklookError::BodyTooLarge);
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
}

#[cfg(test)]
fn validate_loopback_fixture_origin(origin: &Url) -> Result<(), CdseQuicklookError> {
    let literal_loopback = matches!(
        origin.host_str(),
        Some("127.0.0.1") | Some("[::1]") | Some("::1")
    );
    if origin.scheme() != "http"
        || !literal_loopback
        || origin.port().is_none()
        || !matches!(origin.path(), "" | "/")
        || !origin.username().is_empty()
        || origin.password().is_some()
        || origin.query().is_some()
        || origin.fragment().is_some()
    {
        return Err(CdseQuicklookError::ConfigurationInvalid);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::{Duration, Instant};

    use super::*;

    const ASSET_ID: &str = "00000000-0000-4000-8000-000000000001";
    const PRODUCT_ID: &str = "00000000-0000-4000-8000-000000000010";
    const MAX_HEADER_BYTES: usize = 8 * 1024;

    #[derive(Clone, Debug)]
    struct CapturedRequest {
        request_line: String,
        headers: Vec<(String, String)>,
    }

    impl CapturedRequest {
        fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .iter()
                .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
                .map(|(_, value)| value.as_str())
        }
    }

    struct FixtureServer {
        origin: String,
        captures: Arc<Mutex<Vec<CapturedRequest>>>,
        stop: Arc<AtomicBool>,
        worker: Option<thread::JoinHandle<()>>,
    }

    impl FixtureServer {
        fn new(responses: Vec<Vec<u8>>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("loopback listener");
            listener
                .set_nonblocking(true)
                .expect("nonblocking listener");
            let address = listener.local_addr().expect("listener address");
            let captures = Arc::new(Mutex::new(Vec::new()));
            let captured = Arc::clone(&captures);
            let stop = Arc::new(AtomicBool::new(false));
            let stop_worker = Arc::clone(&stop);
            let worker = thread::spawn(move || {
                let mut response_index = 0;
                while response_index < responses.len() && !stop_worker.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            let request = read_request(&mut stream);
                            captured.lock().expect("capture lock").push(request);
                            stream
                                .write_all(&responses[response_index])
                                .expect("fixture response");
                            stream.flush().expect("flush fixture response");
                            response_index += 1;
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(2));
                        }
                        Err(error) => panic!("fixture accept failed: {error}"),
                    }
                }
            });
            Self {
                origin: format!("http://{address}"),
                captures,
                stop,
                worker: Some(worker),
            }
        }

        fn finish(mut self) -> Vec<CapturedRequest> {
            self.stop.store(true, Ordering::Release);
            self.worker
                .take()
                .expect("fixture worker")
                .join()
                .expect("fixture worker completed");
            self.captures.lock().expect("capture lock").clone()
        }
    }

    impl Drop for FixtureServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }

    fn read_request(stream: &mut TcpStream) -> CapturedRequest {
        stream
            .set_nonblocking(false)
            .expect("fixture request uses bounded blocking reads");
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("fixture request timeout");
        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 256];
        let header_end = loop {
            let count = stream.read(&mut chunk).expect("fixture request read");
            assert!(count > 0, "request ended before header terminator");
            bytes.extend_from_slice(&chunk[..count]);
            assert!(
                bytes.len() <= MAX_HEADER_BYTES,
                "request headers exceeded bound"
            );
            if let Some(position) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                break position + 4;
            }
        };
        let request = std::str::from_utf8(&bytes[..header_end]).expect("request is UTF-8");
        let mut lines = request.split("\r\n");
        let request_line = lines.next().expect("request line").to_owned();
        let headers = lines
            .take_while(|line| !line.is_empty())
            .map(|line| {
                let (name, value) = line.split_once(':').expect("header syntax");
                (name.trim().to_owned(), value.trim().to_owned())
            })
            .collect();
        CapturedRequest {
            request_line,
            headers,
        }
    }

    fn response(status: &str, headers: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
        let mut value = format!("HTTP/1.1 {status}\r\nConnection: close\r\n");
        for (name, content) in headers {
            value.push_str(name);
            value.push_str(": ");
            value.push_str(content);
            value.push_str("\r\n");
        }
        value.push_str("\r\n");
        let mut value = value.into_bytes();
        value.extend_from_slice(body);
        value
    }

    fn client(policy: ProviderNetworkPolicy) -> CdseQuicklookClient {
        CdseQuicklookClient::with_timeout(policy, Duration::from_secs(2)).expect("quicklook client")
    }

    fn fixture_client(server: &FixtureServer) -> CdseQuicklookClient {
        let mut value = client(ProviderNetworkPolicy::direct());
        value
            .retarget_for_loopback_fixture(&server.origin)
            .expect("literal loopback retarget");
        value
    }

    fn required_live_value(name: &str) -> String {
        std::env::var(name)
            .ok()
            .filter(|value| !value.trim().is_empty() && value == value.trim())
            .unwrap_or_else(|| {
                panic!("CDSE_LIVE_QUICKLOOK outcome=stopped code=LIVE_INPUT_REQUIRED")
            })
    }

    #[test]
    fn construction_binds_system_or_direct_without_starting_network() {
        for policy in [
            ProviderNetworkPolicy::system(),
            ProviderNetworkPolicy::direct(),
        ] {
            let client = client(policy);
            assert_eq!(client.network_policy(), policy);
            assert!(!format!("{client:?}").contains(CATALOGUE_HOST));
        }
        assert_eq!(
            CdseQuicklookClient::with_timeout(ProviderNetworkPolicy::system(), Duration::ZERO)
                .unwrap_err(),
            CdseQuicklookError::ConfigurationInvalid
        );
    }

    #[test]
    fn exact_production_endpoint_and_test_seam_reject_untrusted_urls() {
        let production_client = client(ProviderNetworkPolicy::system());
        assert_eq!(
            production_client
                .endpoint_for(ASSET_ID)
                .expect("canonical endpoint")
                .as_str(),
            format!("https://catalogue.dataspace.copernicus.eu/odata/v1/Assets({ASSET_ID})/$value")
        );
        assert_eq!(
            production_client
                .download_endpoint_for(ASSET_ID)
                .expect("reviewed download redirect endpoint")
                .as_str(),
            format!("https://download.dataspace.copernicus.eu/odata/v1/Assets({ASSET_ID})/$value")
        );
        assert_eq!(
            production_client
                .product_assets_endpoint_for(PRODUCT_ID)
                .expect("exact product asset endpoint")
                .as_str(),
            format!(
                "https://catalogue.dataspace.copernicus.eu/odata/v1/Products({PRODUCT_ID})?%24expand=Assets"
            )
        );
        for asset_id in [
            "00000000-0000-4000-8000-000000000001/extra",
            "00000000-0000-4000-8000-000000000001?x=1",
            "00000000-0000-4000-8000-000000000001#fragment",
            "00000000-0000-4000-8000-00000000000A",
            "00000000000040008000000000000001",
        ] {
            assert_eq!(
                production_client.endpoint_for(asset_id).unwrap_err(),
                CdseQuicklookError::AssetIdentityInvalid,
                "{asset_id}"
            );
        }
        for origin in [
            "https://127.0.0.1:1234",
            "http://localhost:1234",
            "http://127.0.0.1:1234/odata/v1/",
            "http://127.0.0.1:1234?token=fixture",
            "http://user@127.0.0.1:1234",
        ] {
            let mut client = client(ProviderNetworkPolicy::direct());
            assert_eq!(
                client.retarget_for_loopback_fixture(origin).unwrap_err(),
                CdseQuicklookError::ConfigurationInvalid,
                "{origin}"
            );
        }
        assert_eq!(
            production_client
                .fetch(ASSET_ID, "image/gif", None, &|| false)
                .unwrap_err(),
            CdseQuicklookError::ExpectedMediaTypeInvalid
        );
        assert_eq!(
            production_client
                .fetch(ASSET_ID, "image/jpeg", Some(0), &|| false)
                .unwrap_err(),
            CdseQuicklookError::ExpectedLengthInvalid
        );
        assert_eq!(
            production_client
                .discover("00000000000040008000000000000010", &|| false)
                .unwrap_err(),
            CdseQuicklookError::ProductIdentityInvalid
        );
    }

    #[test]
    fn discovers_one_optional_quicklook_from_the_exact_product_before_fetching_it() {
        let discovery_body = format!(
            r#"{{"Id":"{PRODUCT_ID}","Assets":[{{"Id":"{ASSET_ID}","Name":"S2_quicklook.jpg","ContentType":"image/jpeg","ContentLength":3,"DownloadLink":"https://catalogue.dataspace.copernicus.eu/odata/v1/Assets({ASSET_ID})/$value"}}]}}"#
        );
        let server = FixtureServer::new(vec![
            response("200 OK", &[], discovery_body.as_bytes()),
            response(
                "200 OK",
                &[("Content-Type", "image/jpeg"), ("Content-Length", "3")],
                b"\xFF\xD8\xFF",
            ),
        ]);
        let client = fixture_client(&server);
        let descriptor = client
            .discover(PRODUCT_ID, &|| false)
            .expect("exact product Assets response")
            .expect("one quicklook");
        assert_eq!(descriptor.asset_id, ASSET_ID);
        assert_eq!(descriptor.media_type, "image/jpeg");
        assert_eq!(descriptor.expected_length, Some(3));
        let image = client
            .fetch(
                &descriptor.asset_id,
                &descriptor.media_type,
                descriptor.expected_length,
                &|| false,
            )
            .expect("bounded discovered image");
        assert_eq!(image.bytes(), b"\xFF\xD8\xFF");
        let captures = server.finish();
        assert_eq!(captures.len(), 2);
        assert_eq!(
            captures[0].request_line,
            format!("GET /odata/v1/Products({PRODUCT_ID})?%24expand=Assets HTTP/1.1")
        );
        assert_eq!(
            captures[1].request_line,
            format!("GET /odata/v1/Assets({ASSET_ID})/$value HTTP/1.1")
        );
        for capture in captures {
            assert!(capture.header("Authorization").is_none());
            assert!(capture.header("Cookie").is_none());
            assert!(capture.header("Referer").is_none());
        }
    }

    #[test]
    fn current_cdse_typed_quicklook_shape_uses_bounded_octet_stream_image_inference() {
        let discovery_body = format!(
            r#"{{"Id":"{PRODUCT_ID}","Assets":[{{"Type":"QUICKLOOK","Id":"{ASSET_ID}","DownloadLink":"https://catalogue.dataspace.copernicus.eu/odata/v1/Assets({ASSET_ID})/$value","S3Path":"/eodata/Sentinel-2/MSI/L2A/example.SAFE"}}]}}"#
        );
        let server = FixtureServer::new(vec![
            response("200 OK", &[], discovery_body.as_bytes()),
            response(
                "200 OK",
                &[
                    ("Content-Type", "application/octet-stream"),
                    ("Content-Length", "3"),
                ],
                b"\xFF\xD8\xFF",
            ),
        ]);
        let client = fixture_client(&server);
        let descriptor = client
            .discover(PRODUCT_ID, &|| false)
            .expect("current CDSE product Assets response")
            .expect("typed QUICKLOOK exists");
        assert_eq!(descriptor.asset_id, ASSET_ID);
        assert_eq!(descriptor.media_type, "image/*");
        assert_eq!(descriptor.expected_length, None);
        let image = client
            .fetch(
                &descriptor.asset_id,
                &descriptor.media_type,
                descriptor.expected_length,
                &|| false,
            )
            .expect("octet-stream QUICKLOOK is verified by image signature");
        assert_eq!(image.media_type(), "image/jpeg");
        assert_eq!(image.bytes(), b"\xFF\xD8\xFF");
        assert_eq!(server.finish().len(), 2);
    }

    #[test]
    fn product_asset_discovery_keeps_missing_optional_and_rejects_untrusted_evidence() {
        let missing_server = FixtureServer::new(vec![response(
            "200 OK",
            &[],
            format!(r#"{{"Id":"{PRODUCT_ID}","Assets":[]}}"#).as_bytes(),
        )]);
        assert_eq!(
            fixture_client(&missing_server)
                .discover(PRODUCT_ID, &|| false)
                .expect("missing quicklook is normal"),
            None
        );
        assert_eq!(missing_server.finish().len(), 1);

        for body in [
            format!(
                r#"{{"Id":"{PRODUCT_ID}","Assets":[{{"Id":"{ASSET_ID}","Name":"S2_quicklook.jpg","ContentType":"image/jpeg","DownloadLink":"http://example.invalid/a.jpg"}}]}}"#
            ),
            format!(
                r#"{{"Id":"{PRODUCT_ID}","Assets":[{{"Id":"00000000-0000-4000-8000-00000000000A","Name":"S2_quicklook.jpg","ContentType":"image/jpeg"}}]}}"#
            ),
            format!(
                r#"{{"Id":"{PRODUCT_ID}","Assets":[{{"Id":"{ASSET_ID}","Name":"S2_quicklook.jpg","ContentType":"image/jpeg","ContentLength":3}},{{"Id":"{ASSET_ID}","Name":"S2_quicklook.jpg","ContentType":"image/jpeg","ContentLength":4}}]}}"#
            ),
            r#"{"Id":"00000000-0000-4000-8000-000000000011","Assets":[]}"#.to_string(),
        ] {
            let server = FixtureServer::new(vec![response("200 OK", &[], body.as_bytes())]);
            assert_eq!(
                fixture_client(&server)
                    .discover(PRODUCT_ID, &|| false)
                    .unwrap_err(),
                CdseQuicklookError::DiscoveryResponseRejected
            );
            assert_eq!(server.finish().len(), 1);
        }
    }

    #[test]
    fn fetches_only_the_exact_asset_without_credentials_or_cookies() {
        let server = FixtureServer::new(vec![response(
            "200 OK",
            &[("Content-Type", "image/jpeg"), ("Content-Length", "3")],
            b"\xFF\xD8\xFF",
        )]);
        let image = fixture_client(&server)
            .fetch(ASSET_ID, "image/jpeg", Some(3), &|| false)
            .expect("bounded image");
        assert_eq!(image.media_type(), "image/jpeg");
        assert_eq!(image.len(), 3);
        assert_eq!(image.bytes(), b"\xFF\xD8\xFF");
        let debug = format!("{image:?}");
        assert!(debug.contains("[REDACTED]"));
        let captures = server.finish();
        assert_eq!(captures.len(), 1);
        assert_eq!(
            captures[0].request_line,
            format!("GET /odata/v1/Assets({ASSET_ID})/$value HTTP/1.1")
        );
        assert!(captures[0].header("Authorization").is_none());
        assert!(captures[0].header("Cookie").is_none());
        assert!(captures[0].header("Referer").is_none());
    }

    #[test]
    fn redirects_media_and_body_evidence_fail_closed() {
        let redirect_server = FixtureServer::new(vec![response(
            "302 Found",
            &[("Location", "http://example.invalid/preview")],
            b"",
        )]);
        assert_eq!(
            fixture_client(&redirect_server)
                .fetch(ASSET_ID, "image/jpeg", None, &|| false)
                .unwrap_err(),
            CdseQuicklookError::RedirectRejected
        );
        assert_eq!(redirect_server.finish().len(), 1);

        let media_server = FixtureServer::new(vec![response(
            "200 OK",
            &[("Content-Type", "text/html"), ("Content-Length", "5")],
            b"html!",
        )]);
        assert_eq!(
            fixture_client(&media_server)
                .fetch(ASSET_ID, "image/jpeg", None, &|| false)
                .unwrap_err(),
            CdseQuicklookError::MediaTypeRejected
        );
        assert_eq!(media_server.finish().len(), 1);

        let mislabeled_server = FixtureServer::new(vec![response(
            "200 OK",
            &[("Content-Type", "image/jpeg"), ("Content-Length", "5")],
            b"html!",
        )]);
        assert_eq!(
            fixture_client(&mislabeled_server)
                .fetch(ASSET_ID, "image/jpeg", None, &|| false)
                .unwrap_err(),
            CdseQuicklookError::ImageSignatureRejected
        );
        assert_eq!(mislabeled_server.finish().len(), 1);

        let too_large_server = FixtureServer::new(vec![response(
            "200 OK",
            &[
                ("Content-Type", "image/png"),
                ("Content-Length", "16777217"),
            ],
            b"x",
        )]);
        assert_eq!(
            fixture_client(&too_large_server)
                .fetch(ASSET_ID, "image/png", None, &|| false)
                .unwrap_err(),
            CdseQuicklookError::BodyTooLarge
        );
        assert_eq!(too_large_server.finish().len(), 1);

        let size_server = FixtureServer::new(vec![response(
            "200 OK",
            &[("Content-Type", "image/webp"), ("Content-Length", "12")],
            b"RIFF\0\0\0\0WEBP",
        )]);
        assert_eq!(
            fixture_client(&size_server)
                .fetch(ASSET_ID, "image/webp", Some(13), &|| false)
                .unwrap_err(),
            CdseQuicklookError::LengthMismatch
        );
        assert_eq!(size_server.finish().len(), 1);
    }

    #[test]
    fn cancellation_is_checked_before_and_after_each_body_read() {
        let before_server = FixtureServer::new(vec![response(
            "200 OK",
            &[("Content-Type", "image/jpeg"), ("Content-Length", "1")],
            b"x",
        )]);
        assert_eq!(
            fixture_client(&before_server)
                .fetch(ASSET_ID, "image/jpeg", None, &|| true)
                .unwrap_err(),
            CdseQuicklookError::Cancelled
        );
        assert!(before_server.finish().is_empty());

        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback listener");
        let address = listener.local_addr().expect("listener address");
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("fixture request");
            let _ = read_request(&mut stream);
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: image/jpeg\r\nContent-Length: 4\r\n\r\na",
                )
                .expect("first chunk");
            stream.flush().expect("flush first chunk");
            thread::sleep(Duration::from_millis(600));
            let _ = stream.write_all(b"bcd");
        });
        let server = FixtureServer {
            origin: format!("http://{address}"),
            captures: Arc::new(Mutex::new(Vec::new())),
            stop: Arc::new(AtomicBool::new(false)),
            worker: None,
        };
        let client = fixture_client(&server);
        let checks = AtomicUsize::new(0);
        let started = Instant::now();
        let error = client
            .fetch(ASSET_ID, "image/jpeg", None, &|| {
                checks.fetch_add(1, Ordering::SeqCst) >= 3
            })
            .expect_err("cancellation interrupts between body chunks");
        assert_eq!(error, CdseQuicklookError::Cancelled);
        assert!(
            started.elapsed() < Duration::from_millis(350),
            "client waited for the delayed body instead of checking cancellation"
        );
        worker.join().expect("fixture worker");
    }

    #[test]
    #[ignore = "requires explicit opt-in and one frozen CDSE quicklook asset"]
    fn controlled_live_cdse_quicklook_acceptance() {
        if std::env::var("INSAR_CDSE_LIVE_ACCEPTANCE").ok().as_deref() != Some("1") {
            panic!("CDSE_LIVE_QUICKLOOK outcome=stopped code=LIVE_INPUT_REQUIRED");
        }
        let asset_id = required_live_value("INSAR_CDSE_LIVE_QUICKLOOK_ASSET_ID");
        let media_type = required_live_value("INSAR_CDSE_LIVE_QUICKLOOK_MEDIA_TYPE");
        let expected_length = required_live_value("INSAR_CDSE_LIVE_QUICKLOOK_EXPECTED_BYTES")
            .parse::<u64>()
            .ok()
            .filter(|length| (1..=CDSE_QUICKLOOK_MAX_BYTES as u64).contains(length))
            .unwrap_or_else(|| {
                panic!("CDSE_LIVE_QUICKLOOK outcome=stopped code=LIVE_INPUT_INVALID")
            });

        let client =
            CdseQuicklookClient::new(ProviderNetworkPolicy::system()).unwrap_or_else(|error| {
                panic!("CDSE_LIVE_QUICKLOOK outcome=stopped code={}", error.code())
            });
        let image = client
            .fetch(&asset_id, &media_type, Some(expected_length), &|| false)
            .unwrap_or_else(|error| {
                panic!("CDSE_LIVE_QUICKLOOK outcome=stopped code={}", error.code())
            });
        assert_eq!(image.len(), expected_length as usize);
        assert!(has_expected_image_signature(
            image.media_type(),
            image.bytes()
        ));
        eprintln!(
            "CDSE_LIVE_QUICKLOOK outcome=completed media_type={} byte_length={}",
            image.media_type(),
            image.len()
        );
    }
}
