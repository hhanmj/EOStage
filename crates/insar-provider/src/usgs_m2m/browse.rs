//! Runtime-only Landsat browse evidence and bounded image transport.

use std::collections::BTreeMap;
use std::fmt;
use std::io::Read;
use std::time::Duration;

use insar_core::ProviderNetworkPolicy;
use reqwest::blocking::{Client, Response};
use reqwest::header::{CONTENT_LENGTH, CONTENT_TYPE};
use reqwest::redirect::Policy;
use reqwest::Url;
use serde_json::Value;
use zeroize::Zeroizing;

use super::error::{UsgsM2mEndpointClass, UsgsM2mError, UsgsM2mErrorCode};

pub const LANDSAT_BROWSE_HOST: &str = "landsatlook.usgs.gov";
pub const LANDSAT_BROWSE_MAX_BYTES: usize = 16 * 1024 * 1024;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(45);
const READ_CHUNK_BYTES: usize = 8 * 1024;
const MAX_BROWSE_CACHE_ENTRIES: usize = 512;
const MAX_LOCATOR_BYTES: usize = 4096;
const MAX_QUERY_PAIRS: usize = 32;
const MAX_QUERY_COMPONENT_BYTES: usize = 512;

#[derive(Clone)]
enum CachedBrowse {
    Available(Zeroizing<String>),
    Unavailable,
    Invalid,
}

pub(crate) enum BrowseLookup {
    Available(Zeroizing<String>),
    Unavailable,
    Invalid,
    Missing,
}

#[derive(Default)]
pub(crate) struct UsgsM2mBrowseCache {
    entries: BTreeMap<String, CachedBrowse>,
}

impl UsgsM2mBrowseCache {
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }

    pub(crate) fn capture_page(&mut self, data: &Value) {
        let Some(results) = data.get("results").and_then(Value::as_array) else {
            return;
        };
        for result in results {
            let Some(result) = result.as_object() else {
                continue;
            };
            let Some(entity_id) = result.get("entityId").and_then(Value::as_str) else {
                continue;
            };
            if !valid_identifier(entity_id) {
                continue;
            }
            let browse = match result.get("browse").and_then(Value::as_array) {
                Some(items) if items.is_empty() => CachedBrowse::Unavailable,
                Some(items) if items.len() == 1 => items[0]
                    .as_object()
                    .and_then(|item| item.get("browsePath"))
                    .and_then(Value::as_str)
                    .filter(|value| value.len() <= MAX_LOCATOR_BYTES)
                    .map(|value| CachedBrowse::Available(Zeroizing::new(value.to_owned())))
                    .unwrap_or(CachedBrowse::Invalid),
                _ => CachedBrowse::Invalid,
            };
            self.record(entity_id.to_owned(), browse);
        }
    }

    pub(crate) fn merge(&mut self, page: Self) {
        for (entity_id, browse) in page.entries {
            self.record(entity_id, browse);
        }
    }

    pub(crate) fn lookup(&self, entity_id: &str) -> BrowseLookup {
        match self.entries.get(entity_id) {
            Some(CachedBrowse::Available(value)) => {
                BrowseLookup::Available(Zeroizing::new(value.as_str().to_owned()))
            }
            Some(CachedBrowse::Unavailable) => BrowseLookup::Unavailable,
            Some(CachedBrowse::Invalid) => BrowseLookup::Invalid,
            None => BrowseLookup::Missing,
        }
    }

    fn record(&mut self, entity_id: String, browse: CachedBrowse) {
        match self.entries.get(&entity_id) {
            Some(existing) if !same_cached_browse(existing, &browse) => {
                self.entries.insert(entity_id, CachedBrowse::Invalid);
            }
            Some(_) => {}
            None => {
                if self.entries.len() >= MAX_BROWSE_CACHE_ENTRIES {
                    if let Some(oldest) = self.entries.keys().next().cloned() {
                        self.entries.remove(&oldest);
                    }
                }
                self.entries.insert(entity_id, browse);
            }
        }
    }
}

impl fmt::Debug for UsgsM2mBrowseCache {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UsgsM2mBrowseCache")
            .field("entry_count", &self.entries.len())
            .field("locators", &"[REDACTED]")
            .finish()
    }
}

fn same_cached_browse(left: &CachedBrowse, right: &CachedBrowse) -> bool {
    match (left, right) {
        (CachedBrowse::Available(left), CachedBrowse::Available(right)) => left == right,
        (CachedBrowse::Unavailable, CachedBrowse::Unavailable)
        | (CachedBrowse::Invalid, CachedBrowse::Invalid) => true,
        _ => false,
    }
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value == value.trim()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

pub struct UsgsM2mBrowseImage {
    media_type: String,
    bytes: Vec<u8>,
}

impl UsgsM2mBrowseImage {
    pub fn media_type(&self) -> &str {
        &self.media_type
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

impl fmt::Debug for UsgsM2mBrowseImage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UsgsM2mBrowseImage")
            .field("media_type", &self.media_type)
            .field("byte_length", &self.bytes.len())
            .field("bytes", &"[REDACTED]")
            .finish()
    }
}

pub struct UsgsM2mBrowseClient {
    client: Client,
    network_policy: ProviderNetworkPolicy,
    #[cfg(test)]
    fixture_origin: Option<Url>,
}

impl UsgsM2mBrowseClient {
    pub fn new(network_policy: ProviderNetworkPolicy) -> Result<Self, UsgsM2mError> {
        Self::with_timeout(network_policy, DEFAULT_TIMEOUT)
    }

    pub fn with_timeout(
        network_policy: ProviderNetworkPolicy,
        timeout: Duration,
    ) -> Result<Self, UsgsM2mError> {
        if timeout.is_zero() || timeout > Duration::from_secs(60) {
            return Err(browse_error(UsgsM2mErrorCode::ConfigInvalid));
        }
        let client = Client::builder()
            .redirect(Policy::none())
            .retry(reqwest::retry::never())
            .referer(false)
            .connect_timeout(timeout.min(Duration::from_secs(15)))
            .timeout(timeout);
        let client =
            crate::network_proxy::blocking_builder_with_proxy(client, network_policy.route())
                .build()
                .map_err(|_| browse_error(UsgsM2mErrorCode::ConfigInvalid))?;
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

    pub fn fetch(
        &self,
        locator: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<UsgsM2mBrowseImage, UsgsM2mError> {
        let endpoint = self.endpoint_for(locator)?;
        if cancelled() {
            return Err(browse_error(UsgsM2mErrorCode::Cancelled));
        }
        let response = self.client.get(endpoint.clone()).send().map_err(|error| {
            browse_error(if error.is_timeout() {
                UsgsM2mErrorCode::Timeout
            } else {
                UsgsM2mErrorCode::TransportFailed
            })
        })?;
        if cancelled() {
            return Err(browse_error(UsgsM2mErrorCode::Cancelled));
        }
        if response.url() != &endpoint {
            return Err(browse_error(UsgsM2mErrorCode::RedirectRejected));
        }
        if response.status().is_redirection() {
            return Err(browse_error(UsgsM2mErrorCode::RedirectRejected));
        }
        if !response.status().is_success() {
            return Err(browse_error(UsgsM2mErrorCode::RemoteError)
                .with_http_status(response.status().as_u16()));
        }
        let media_type = response_media_type(&response)?;
        let declared_length = response_content_length(&response)?;
        if declared_length == Some(0) {
            return Err(
                browse_error(UsgsM2mErrorCode::ResponseInvalid).with_detail("browse_body_empty")
            );
        }
        if declared_length.is_some_and(|length| length > LANDSAT_BROWSE_MAX_BYTES) {
            return Err(browse_error(UsgsM2mErrorCode::ResponseTooLarge));
        }
        let bytes = read_bounded_body(response, cancelled)?;
        if bytes.is_empty() {
            return Err(
                browse_error(UsgsM2mErrorCode::ResponseInvalid).with_detail("browse_body_empty")
            );
        }
        if declared_length.is_some_and(|length| length != bytes.len()) {
            return Err(browse_error(UsgsM2mErrorCode::ResponseInvalid)
                .with_detail("browse_length_mismatch"));
        }
        if !has_expected_image_signature(media_type, &bytes) {
            return Err(browse_error(UsgsM2mErrorCode::ResponseInvalid)
                .with_detail("browse_image_signature"));
        }
        Ok(UsgsM2mBrowseImage {
            media_type: media_type.to_owned(),
            bytes,
        })
    }

    fn endpoint_for(&self, locator: &str) -> Result<Url, UsgsM2mError> {
        let production = validated_browse_locator(locator)
            .ok_or_else(|| browse_error(UsgsM2mErrorCode::ConfigInvalid))?;
        #[cfg(test)]
        if let Some(origin) = &self.fixture_origin {
            let mut endpoint = origin.clone();
            endpoint.set_path(production.path());
            endpoint.set_query(production.query());
            return Ok(endpoint);
        }
        Ok(production)
    }

    #[cfg(test)]
    fn retarget_for_loopback_fixture(&mut self, origin: &str) -> Result<(), UsgsM2mError> {
        let origin =
            Url::parse(origin).map_err(|_| browse_error(UsgsM2mErrorCode::ConfigInvalid))?;
        if origin.scheme() != "http"
            || !matches!(origin.host_str(), Some("127.0.0.1") | Some("::1"))
            || origin.port().is_none()
            || !matches!(origin.path(), "" | "/")
            || !origin.username().is_empty()
            || origin.password().is_some()
            || origin.query().is_some()
            || origin.fragment().is_some()
        {
            return Err(browse_error(UsgsM2mErrorCode::ConfigInvalid));
        }
        self.fixture_origin = Some(origin);
        Ok(())
    }
}

impl fmt::Debug for UsgsM2mBrowseClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UsgsM2mBrowseClient")
            .field("network_policy", &self.network_policy)
            .field("endpoint", &"[REDACTED]")
            .finish()
    }
}

fn validated_browse_locator(value: &str) -> Option<Url> {
    if value.is_empty()
        || value.len() > MAX_LOCATOR_BYTES
        || value != value.trim()
        || value.chars().any(char::is_control)
    {
        return None;
    }
    let url = Url::parse(value).ok()?;
    if url.scheme() != "https"
        || url.host_str() != Some(LANDSAT_BROWSE_HOST)
        || url.port_or_known_default() != Some(443)
        || url.path().is_empty()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.query().is_none()
    {
        return None;
    }
    let mut query_count = 0_usize;
    for (key, value) in url.query_pairs() {
        query_count += 1;
        if query_count > MAX_QUERY_PAIRS
            || key.is_empty()
            || key.len() > MAX_QUERY_COMPONENT_BYTES
            || value.len() > MAX_QUERY_COMPONENT_BYTES
            || credential_like_query_key(&key)
            || key.chars().chain(value.chars()).any(char::is_control)
        {
            return None;
        }
    }
    (query_count > 0).then_some(url)
}

fn credential_like_query_key(value: &str) -> bool {
    let normalized = value.to_ascii_lowercase().replace(['-', '.'], "_");
    [
        "token",
        "api_key",
        "apikey",
        "auth",
        "credential",
        "password",
        "secret",
        "signature",
        "signed",
    ]
    .iter()
    .any(|forbidden| normalized.contains(forbidden))
}

fn response_media_type(response: &Response) -> Result<&'static str, UsgsM2mError> {
    let values = response.headers().get_all(CONTENT_TYPE);
    let mut values = values.iter();
    let raw = values
        .next()
        .ok_or_else(|| browse_error(UsgsM2mErrorCode::ResponseInvalid))?;
    if values.next().is_some() {
        return Err(browse_error(UsgsM2mErrorCode::ResponseInvalid));
    }
    let raw = raw
        .to_str()
        .map_err(|_| browse_error(UsgsM2mErrorCode::ResponseInvalid))?;
    match raw.split(';').next().map(str::trim) {
        Some("image/jpeg") => Ok("image/jpeg"),
        Some("image/png") => Ok("image/png"),
        Some("image/webp") => Ok("image/webp"),
        _ => Err(browse_error(UsgsM2mErrorCode::ResponseInvalid).with_detail("browse_media_type")),
    }
}

fn response_content_length(response: &Response) -> Result<Option<usize>, UsgsM2mError> {
    let values = response.headers().get_all(CONTENT_LENGTH);
    let mut values = values.iter();
    let Some(raw) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Err(browse_error(UsgsM2mErrorCode::ResponseInvalid));
    }
    let raw = raw
        .to_str()
        .map_err(|_| browse_error(UsgsM2mErrorCode::ResponseInvalid))?;
    raw.parse::<usize>()
        .map(Some)
        .map_err(|_| browse_error(UsgsM2mErrorCode::ResponseInvalid))
}

fn read_bounded_body(
    mut response: Response,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<Vec<u8>, UsgsM2mError> {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; READ_CHUNK_BYTES];
    loop {
        if cancelled() {
            return Err(browse_error(UsgsM2mErrorCode::Cancelled));
        }
        let count = response
            .read(&mut chunk)
            .map_err(|_| browse_error(UsgsM2mErrorCode::ResponseInvalid))?;
        if count == 0 {
            return Ok(bytes);
        }
        if bytes.len().saturating_add(count) > LANDSAT_BROWSE_MAX_BYTES {
            return Err(browse_error(UsgsM2mErrorCode::ResponseTooLarge));
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
}

fn has_expected_image_signature(media_type: &str, bytes: &[u8]) -> bool {
    match media_type {
        "image/jpeg" => bytes.starts_with(&[0xFF, 0xD8, 0xFF]),
        "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1A\n"),
        "image/webp" => bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP",
        _ => false,
    }
}

fn browse_error(code: UsgsM2mErrorCode) -> UsgsM2mError {
    UsgsM2mError::new(code, UsgsM2mEndpointClass::BrowseFetch)
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    use super::*;

    const LOCATOR: &str =
        "https://landsatlook.usgs.gov/gen-browse?size=rrb&entityId=LC90000000000000000001";

    fn serve_once(response: Vec<u8>) -> (String, Arc<Mutex<Vec<u8>>>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback listener");
        let origin = format!(
            "http://{}",
            listener.local_addr().expect("listener address")
        );
        let captured = Arc::new(Mutex::new(Vec::new()));
        let captured_for_thread = Arc::clone(&captured);
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("one browse request");
            let mut request = Vec::new();
            let mut chunk = [0_u8; 2048];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let count = stream.read(&mut chunk).expect("read browse request");
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..count]);
            }
            *captured_for_thread.lock().expect("captured request") = request;
            stream.write_all(&response).expect("write browse response");
        });
        (origin, captured, handle)
    }

    fn fixture_client(origin: &str) -> UsgsM2mBrowseClient {
        let mut client = UsgsM2mBrowseClient::new(ProviderNetworkPolicy::direct())
            .expect("fixture browse client");
        client
            .retarget_for_loopback_fixture(origin)
            .expect("literal loopback origin");
        client
    }

    #[test]
    fn production_browse_locator_is_exact_and_credential_free() {
        let accepted =
            "https://landsatlook.usgs.gov/gen-browse?size=rrb&entityId=LC90000000000000000001";
        assert_eq!(
            validated_browse_locator(accepted)
                .and_then(|url| url.host_str().map(str::to_owned))
                .as_deref(),
            Some(LANDSAT_BROWSE_HOST)
        );

        for rejected in [
            "http://landsatlook.usgs.gov/gen-browse?size=rrb",
            "https://landsatlook.usgs.gov:444/gen-browse?size=rrb",
            "https://user@landsatlook.usgs.gov/gen-browse?size=rrb",
            "https://landsatlook.usgs.gov/gen-browse#fragment",
            "https://example.invalid/gen-browse?size=rrb",
            "https://landsatlook.usgs.gov/gen-browse?token=secret",
            "https://landsatlook.usgs.gov/gen-browse?api_key=secret",
            "https://landsatlook.usgs.gov/gen-browse?signature=secret",
        ] {
            assert!(validated_browse_locator(rejected).is_none(), "{rejected}");
        }
    }

    #[test]
    fn browse_cache_is_bounded_and_rejects_ambiguous_or_conflicting_records() {
        let mut cache = UsgsM2mBrowseCache::default();
        cache.capture_page(&serde_json::json!({"results":[
            {"entityId":"LC90000000000000000001","browse":[]},
            {"entityId":"LC90000000000000000002","browse":[
                {"browsePath":"https://landsatlook.usgs.gov/gen-browse?size=rrb"},
                {"browsePath":"https://landsatlook.usgs.gov/gen-browse?size=thumb"}
            ]}
        ]}));
        assert!(matches!(
            cache.lookup("LC90000000000000000001"),
            BrowseLookup::Unavailable
        ));
        assert!(matches!(
            cache.lookup("LC90000000000000000002"),
            BrowseLookup::Invalid
        ));
        assert!(!format!("{cache:?}").contains("landsatlook"));
    }

    #[test]
    fn image_signatures_are_media_type_specific() {
        assert!(has_expected_image_signature(
            "image/jpeg",
            &[0xFF, 0xD8, 0xFF, 0x00]
        ));
        assert!(has_expected_image_signature(
            "image/png",
            b"\x89PNG\r\n\x1A\nfixture"
        ));
        assert!(has_expected_image_signature(
            "image/webp",
            b"RIFF0000WEBPfixture"
        ));
        assert!(!has_expected_image_signature("image/jpeg", b"<html>"));
    }

    #[test]
    fn browse_fetch_is_bounded_and_never_forwards_m2m_authorization() {
        let body = [vec![0xFF, 0xD8, 0xFF], b"fixture-jpeg".to_vec()].concat();
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes()
        .into_iter()
        .chain(body)
        .collect();
        let (origin, captured, handle) = serve_once(response);
        let image = fixture_client(&origin)
            .fetch(LOCATOR, &|| false)
            .expect("reviewed JPEG browse");
        handle.join().expect("loopback server");
        assert_eq!(image.media_type(), "image/jpeg");
        assert!(!image.is_empty());
        let request = String::from_utf8(captured.lock().unwrap().clone()).unwrap();
        assert!(request.starts_with("GET /gen-browse?size=rrb&entityId="));
        let lowercase = request.to_ascii_lowercase();
        for forbidden in ["x-auth-token:", "authorization:", "cookie:", "referer:"] {
            assert!(!lowercase.contains(forbidden), "{forbidden}");
        }
    }

    #[test]
    fn browse_fetch_rejects_redirects_html_empty_and_oversized_bodies() {
        for (response, expected) in [
            (
                b"HTTP/1.1 302 Found\r\nLocation: https://example.invalid/image.jpg\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
                UsgsM2mErrorCode::RedirectRejected,
            ),
            (
                b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 6\r\nConnection: close\r\n\r\n<html>".to_vec(),
                UsgsM2mErrorCode::ResponseInvalid,
            ),
            (
                b"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
                UsgsM2mErrorCode::ResponseInvalid,
            ),
            (
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    LANDSAT_BROWSE_MAX_BYTES + 1
                )
                .into_bytes(),
                UsgsM2mErrorCode::ResponseTooLarge,
            ),
        ] {
            let (origin, _, handle) = serve_once(response);
            let error = fixture_client(&origin)
                .fetch(LOCATOR, &|| false)
                .expect_err("unsafe browse response");
            handle.join().expect("loopback server");
            assert_eq!(error.code(), expected);
        }
    }

    #[test]
    fn browse_fetch_honors_cancellation_before_transport() {
        let client = UsgsM2mBrowseClient::new(ProviderNetworkPolicy::direct()).unwrap();
        assert_eq!(
            client.fetch(LOCATOR, &|| true).unwrap_err().code(),
            UsgsM2mErrorCode::Cancelled
        );
    }
}
