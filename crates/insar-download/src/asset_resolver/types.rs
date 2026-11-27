use std::fmt;
use std::time::SystemTime;

use insar_core::{dns_name_matches, AssetRole};
use reqwest::Url;
use zeroize::Zeroizing;

use super::{Clock, ResolveError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolvedMethod {
    Get,
    Head,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedChecksum {
    pub algorithm: String,
    pub value: String,
}

impl ResolvedChecksum {
    pub fn new(
        algorithm: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<Self, ResolveError> {
        let algorithm = algorithm.into();
        let value = value.into();
        if algorithm.trim().is_empty() || value.trim().is_empty() {
            return Err(ResolveError::descriptor_invalid());
        }
        Ok(Self { algorithm, value })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RemoteValidator {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub content_length: Option<u64>,
}

pub(crate) struct SecretValue(Zeroizing<String>);

impl SecretValue {
    pub(crate) fn new(value: impl Into<String>) -> Result<Self, ResolveError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(ResolveError::descriptor_invalid());
        }
        Ok(Self(Zeroizing::new(value)))
    }

    pub(crate) fn expose(&self) -> &str {
        self.0.as_str()
    }
}

pub(crate) enum ResolvedAuthorization {
    Bearer(SecretValue),
    Basic {
        username: SecretValue,
        password: SecretValue,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResolvedAuthKind {
    None,
    Bearer,
    Basic,
}

impl fmt::Debug for ResolvedAuthorization {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match self {
            Self::Bearer(_) => "bearer",
            Self::Basic { .. } => "basic",
        };
        formatter
            .debug_struct("ResolvedAuthorization")
            .field("kind", &kind)
            .field("secret", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResolvedHostPolicy {
    EarthdataAsfV1,
    CdseDownloadV1,
    UsgsLandsatDownloadV1,
    PlanetaryComputerLandsatV1,
    PlanetaryComputerSentinel1GrdV1,
    PlanetaryComputerCopDemGlo30V1,
    PlanetaryComputerSentinel2L2aV1,
    PlanetaryComputerHlsV1,
    EarthSearchSentinel2CogsV1,
    /// Exact authenticated Bhoonidhi NISAR S-RSLC `/download` route.
    BhoonidhiDownloadV1,
    #[cfg(test)]
    LoopbackFixture,
    #[cfg(test)]
    EarthdataBurstLoopbackFixture,
}

const ASF_EXTRACTOR_SIGNED_OBJECT_HOST: &str =
    "extractor-main-extractorstagingbucket-efrxkhdd3zlp.s3.amazonaws.com";

fn bhoonidhi_download_url(url: &Url) -> bool {
    if url.host_str() != Some("bhoonidhi-api.nrsc.gov.in")
        || url.fragment().is_some()
        || url.path() != "/download"
    {
        return false;
    }
    let pairs = url.query_pairs().collect::<Vec<_>>();
    pairs.len() == 2
        && pairs[0].0 == "id"
        && !pairs[0].1.is_empty()
        && pairs[0].1.len() <= 512
        && pairs[0]
            .1
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
        && pairs[1].0 == "collection"
        && pairs[1].1 == "NISAR_SSAR_RSLC"
}

impl ResolvedHostPolicy {
    pub(crate) fn allows_response_url(self, url: &Url) -> bool {
        #[cfg(test)]
        if matches!(
            self,
            Self::LoopbackFixture | Self::EarthdataBurstLoopbackFixture
        ) {
            return loopback_fixture_url(url);
        }
        if url.scheme() != "https"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port_or_known_default() != Some(443)
        {
            return false;
        }
        let Some(host) = url.host_str() else {
            return false;
        };
        match self {
            Self::EarthdataAsfV1 => {
                [
                    "earthdata.nasa.gov",
                    "earthdatacloud.nasa.gov",
                    "asf.alaska.edu",
                    // ASF cumulus objects terminate on CloudFront-signed object
                    // URLs. These two distribution hosts were verified against
                    // the real ALOS-2 and NISAR acceptance objects; the URL is
                    // https, port 443, contains a signed query, and the path is
                    // the ASF S3 object class (e.g. `/sds/...`). Unknown
                    // distribution hosts still fail closed.
                    "d1mv8zhcvry6x4.cloudfront.net",
                    "d1bctmng8vmwga.cloudfront.net",
                ]
                .iter()
                .any(|allowed| dns_name_matches(host, allowed))
                    || asf_extractor_signed_object_url(url)
            }
            Self::CdseDownloadV1 => host == "download.dataspace.copernicus.eu",
            Self::UsgsLandsatDownloadV1 => {
                super::usgs_m2m::approved_download_response_url_parsed(url)
            }
            Self::PlanetaryComputerLandsatV1 => {
                host == "landsateuwest.blob.core.windows.net"
                    && url.fragment().is_none()
                    && insar_provider::planetary_computer::is_valid_planetary_computer_landsat_path(
                        url.path(),
                    )
            }
            Self::PlanetaryComputerSentinel1GrdV1 => {
                insar_provider::planetary_computer_asset::validate_planetary_computer_runtime_asset_url(
                    insar_provider::planetary_computer_asset::PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog,
                    url,
                )
                .is_ok()
            }
            Self::PlanetaryComputerCopDemGlo30V1 => {
                insar_provider::planetary_computer_asset::validate_planetary_computer_runtime_asset_url(
                    insar_provider::planetary_computer_asset::PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog,
                    url,
                )
                .is_ok()
            }
            Self::PlanetaryComputerSentinel2L2aV1 => {
                insar_provider::planetary_computer_asset::validate_planetary_computer_runtime_asset_url(
                    insar_provider::planetary_computer_asset::PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog,
                    url,
                )
                .is_ok()
            }
            Self::PlanetaryComputerHlsV1 => {
                (host == "hls2euwest.blob.core.windows.net"
                    || host == "hlseuwest.blob.core.windows.net")
                    && (url.path().starts_with("/hls2/") || url.path().starts_with("/hls/"))
                    && (url.path().ends_with(".tif") || url.path().ends_with(".tiff"))
                    && url.fragment().is_none()
            }
            Self::EarthSearchSentinel2CogsV1 => {
                insar_provider::catalog::is_valid_earth_search_sentinel2_cog_url(url)
            }
            Self::BhoonidhiDownloadV1 => bhoonidhi_download_url(url),
            #[cfg(test)]
            Self::LoopbackFixture | Self::EarthdataBurstLoopbackFixture => {
                unreachable!("handled before HTTPS policy checks")
            }
        }
    }

    pub(crate) fn allows_redirect(self, _initial: &Url, candidate: &Url) -> bool {
        #[cfg(test)]
        if matches!(self, Self::EarthdataBurstLoopbackFixture) {
            return self.allows_response_url(candidate)
                && _initial.scheme() == candidate.scheme()
                && _initial.host_str() == candidate.host_str()
                && _initial.port_or_known_default() == candidate.port_or_known_default();
        }
        self.allows_response_url(candidate)
    }

    pub(crate) fn allows_authenticated_request_url(self, url: &Url) -> bool {
        #[cfg(test)]
        if matches!(
            self,
            Self::LoopbackFixture | Self::EarthdataBurstLoopbackFixture
        ) {
            return loopback_fixture_url(url);
        }
        if url.scheme() != "https"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port_or_known_default() != Some(443)
        {
            return false;
        }
        let Some(host) = url.host_str() else {
            return false;
        };
        match self {
            Self::EarthdataAsfV1 => [
                "earthdata.nasa.gov",
                "earthdatacloud.nasa.gov",
                "asf.alaska.edu",
            ]
            .iter()
            .any(|allowed| dns_name_matches(host, allowed)),
            Self::CdseDownloadV1
            | Self::UsgsLandsatDownloadV1
            | Self::PlanetaryComputerLandsatV1
            | Self::PlanetaryComputerSentinel1GrdV1
            | Self::PlanetaryComputerCopDemGlo30V1
            | Self::PlanetaryComputerSentinel2L2aV1
            | Self::PlanetaryComputerHlsV1
            | Self::EarthSearchSentinel2CogsV1
            | Self::BhoonidhiDownloadV1 => self.allows_response_url(url),
            #[cfg(test)]
            Self::LoopbackFixture | Self::EarthdataBurstLoopbackFixture => {
                unreachable!("handled before HTTPS policy checks")
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AsfExtractorRedirectRejection {
    Approved,
    Path,
    QueryKeys,
    Algorithm,
    CredentialScope,
    Date,
    Expires,
    SignedHeaders,
    TokenPresence,
    Signature,
    AsfUserId,
    ContentDisposition,
}

impl AsfExtractorRedirectRejection {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Approved => "approved",
            Self::Path => "path",
            Self::QueryKeys => "query_keys",
            Self::Algorithm => "algorithm",
            Self::CredentialScope => "credential_scope",
            Self::Date => "date",
            Self::Expires => "expires",
            Self::SignedHeaders => "signed_headers",
            Self::TokenPresence => "token_presence",
            Self::Signature => "signature",
            Self::AsfUserId => "asf_user_id",
            Self::ContentDisposition => "content_disposition",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AsfExtractorPathSegmentCount {
    Zero,
    One,
    Two,
    ThreePlus,
}

impl AsfExtractorPathSegmentCount {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Zero => "0",
            Self::One => "1",
            Self::Two => "2",
            Self::ThreePlus => "3plus",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AsfExtractorPathTerminal {
    Exact0Tiff,
    OtherTiff,
    Exact0Xml,
    OtherXml,
    Unsupported,
}

impl AsfExtractorPathTerminal {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Exact0Tiff => "exact_0_tiff",
            Self::OtherTiff => "other_tiff",
            Self::Exact0Xml => "exact_0_xml",
            Self::OtherXml => "other_xml",
            Self::Unsupported => "unsupported",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AsfExtractorRedirectDiagnostic {
    pub(crate) rejection: AsfExtractorRedirectRejection,
    pub(crate) path_segment_count: AsfExtractorPathSegmentCount,
    pub(crate) path_terminal: AsfExtractorPathTerminal,
    pub(crate) path_has_encoded_separator_or_dot: bool,
    pub(crate) path_trailing_slash: bool,
    pub(crate) query_keys: Vec<String>,
}

fn safe_extractor_query_key_name(key: &str) -> String {
    if !key.is_empty()
        && key.len() <= 64
        && key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        key.to_owned()
    } else {
        "invalid_key".to_owned()
    }
}

fn percent_encoded_control_or_path_syntax(path: &str) -> bool {
    let bytes = path.as_bytes();
    for index in 0..bytes.len() {
        if bytes[index] != b'%' || index + 2 >= bytes.len() {
            continue;
        }
        let hex = |byte: u8| match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            b'A'..=b'F' => Some(byte - b'A' + 10),
            _ => None,
        };
        let Some(value) = hex(bytes[index + 1])
            .zip(hex(bytes[index + 2]))
            .map(|(high, low)| high * 16 + low)
        else {
            continue;
        };
        if value <= 0x1f || value == 0x7f || matches!(value, b'.' | b'/' | b'\\') {
            return true;
        }
    }
    false
}

fn safe_extractor_filename(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && (value.ends_with(".tiff") || value.ends_with(".xml"))
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn safe_extractor_user_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'@'))
}

fn extractor_attachment_filename(value: &str) -> Option<&str> {
    if value.is_empty()
        || value.len() > 512
        || !value.is_ascii()
        || value.bytes().any(|byte| byte.is_ascii_control())
    {
        return None;
    }
    let mut parts = value.split(';');
    if !parts.next()?.trim().eq_ignore_ascii_case("attachment") {
        return None;
    }
    let parameter = parts.next()?.trim();
    if parts.next().is_some() {
        return None;
    }
    let (name, raw_filename) = parameter.split_once('=')?;
    if !name.trim().eq_ignore_ascii_case("filename") {
        return None;
    }
    let raw_filename = raw_filename.trim();
    let filename = if raw_filename.starts_with('"') && raw_filename.ends_with('"') {
        raw_filename.get(1..raw_filename.len().checked_sub(1)?)?
    } else {
        raw_filename
    };
    safe_extractor_filename(filename).then_some(filename)
}

pub(crate) fn asf_extractor_signed_object_diagnostic(
    url: &Url,
) -> Option<AsfExtractorRedirectDiagnostic> {
    if url.host_str() != Some(ASF_EXTRACTOR_SIGNED_OBJECT_HOST) {
        return None;
    }
    let path_segments = url
        .path_segments()
        .map(|segments| segments.collect::<Vec<_>>());
    let nonempty_segments = path_segments
        .as_ref()
        .map(|segments| {
            segments
                .iter()
                .copied()
                .filter(|segment| !segment.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let path_segment_count = match nonempty_segments.len() {
        0 => AsfExtractorPathSegmentCount::Zero,
        1 => AsfExtractorPathSegmentCount::One,
        2 => AsfExtractorPathSegmentCount::Two,
        _ => AsfExtractorPathSegmentCount::ThreePlus,
    };
    let path_terminal = match nonempty_segments.last().copied() {
        Some("0.tiff") => AsfExtractorPathTerminal::Exact0Tiff,
        Some(terminal) if terminal.ends_with(".tiff") => AsfExtractorPathTerminal::OtherTiff,
        Some("0.xml") => AsfExtractorPathTerminal::Exact0Xml,
        Some(terminal) if terminal.ends_with(".xml") => AsfExtractorPathTerminal::OtherXml,
        _ => AsfExtractorPathTerminal::Unsupported,
    };
    let path_has_encoded_separator_or_dot = percent_encoded_control_or_path_syntax(url.path());
    let path_trailing_slash = url.path().ends_with('/');
    let path_basename = nonempty_segments.last().copied();
    let bounded_safe_segments = path_segments.as_ref().is_some_and(|segments| {
        !segments.is_empty()
            && segments.iter().all(|segment| {
                !segment.is_empty()
                    && *segment != "."
                    && *segment != ".."
                    && segment.len() <= 255
                    && segment.bytes().all(|byte| !byte.is_ascii_control())
            })
    });
    let approved_terminal_shape = match path_terminal {
        AsfExtractorPathTerminal::Exact0Tiff | AsfExtractorPathTerminal::Exact0Xml => matches!(
            path_segment_count,
            AsfExtractorPathSegmentCount::Two | AsfExtractorPathSegmentCount::ThreePlus
        ),
        AsfExtractorPathTerminal::OtherTiff | AsfExtractorPathTerminal::OtherXml => {
            path_segment_count == AsfExtractorPathSegmentCount::ThreePlus
        }
        AsfExtractorPathTerminal::Unsupported => false,
    };
    let path_valid = url.fragment().is_none()
        && url.path().len() <= 2048
        && !path_has_encoded_separator_or_dot
        && !path_trailing_slash
        && bounded_safe_segments
        && path_basename.is_some_and(safe_extractor_filename)
        && approved_terminal_shape;
    let query = url
        .query_pairs()
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    let allowed_keys = [
        "X-Amz-Algorithm",
        "X-Amz-Credential",
        "X-Amz-Date",
        "X-Amz-Expires",
        "X-Amz-SignedHeaders",
        "X-Amz-Security-Token",
        "X-Amz-Signature",
        "X-Asf-UrsUserId",
        "response-content-disposition",
    ];
    let mut query_keys = query
        .iter()
        .map(|(key, _)| safe_extractor_query_key_name(key))
        .collect::<Vec<_>>();
    query_keys.sort_unstable();
    query_keys.dedup();
    let query_keys_valid = query.len() >= 8
        && query.len() <= 9
        && !query
            .iter()
            .any(|(key, _)| !allowed_keys.contains(&key.as_str()))
        && !allowed_keys.iter().any(|key| {
            let count = query
                .iter()
                .filter(|(candidate, _)| candidate == key)
                .count();
            count > 1 || (*key != "X-Amz-Security-Token" && count != 1)
        });
    let diagnostic = |rejection| AsfExtractorRedirectDiagnostic {
        rejection,
        path_segment_count,
        path_terminal,
        path_has_encoded_separator_or_dot,
        path_trailing_slash,
        query_keys: query_keys.clone(),
    };
    if !path_valid {
        return Some(diagnostic(AsfExtractorRedirectRejection::Path));
    }
    if !query_keys_valid {
        return Some(diagnostic(AsfExtractorRedirectRejection::QueryKeys));
    }
    let value = |key: &str| {
        query
            .iter()
            .find_map(|(candidate, value)| (candidate == key).then_some(value.as_str()))
    };
    if value("X-Amz-Algorithm") != Some("AWS4-HMAC-SHA256") {
        return Some(diagnostic(AsfExtractorRedirectRejection::Algorithm));
    }
    let credential_valid = value("X-Amz-Credential").is_some_and(|credential| {
        let credential = credential.split('/').collect::<Vec<_>>();
        credential.len() == 5
            && !credential[0].is_empty()
            && credential[1].len() == 8
            && credential[1].bytes().all(|byte| byte.is_ascii_digit())
            && !credential[2].is_empty()
            && credential[3] == "s3"
            && credential[4] == "aws4_request"
    });
    if !credential_valid {
        return Some(diagnostic(AsfExtractorRedirectRejection::CredentialScope));
    }
    let date_valid = value("X-Amz-Date").is_some_and(|date| {
        date.len() == 16
            && date.ends_with('Z')
            && date[..15]
                .bytes()
                .enumerate()
                .all(|(index, byte)| index == 8 && byte == b'T' || byte.is_ascii_digit())
    });
    if !date_valid {
        return Some(diagnostic(AsfExtractorRedirectRejection::Date));
    }
    let expires_valid = value("X-Amz-Expires")
        .and_then(|expires| expires.parse::<u64>().ok())
        .is_some_and(|expires| (1..=604_800).contains(&expires));
    if !expires_valid {
        return Some(diagnostic(AsfExtractorRedirectRejection::Expires));
    }
    if value("X-Amz-SignedHeaders") != Some("host") {
        return Some(diagnostic(AsfExtractorRedirectRejection::SignedHeaders));
    }
    if value("X-Amz-Security-Token").is_some_and(str::is_empty) {
        return Some(diagnostic(AsfExtractorRedirectRejection::TokenPresence));
    }
    let signature_valid = value("X-Amz-Signature").is_some_and(|signature| {
        signature.len() == 64 && signature.bytes().all(|byte| byte.is_ascii_hexdigit())
    });
    if !signature_valid {
        return Some(diagnostic(AsfExtractorRedirectRejection::Signature));
    }
    if !value("X-Asf-UrsUserId").is_some_and(safe_extractor_user_id) {
        return Some(diagnostic(AsfExtractorRedirectRejection::AsfUserId));
    }
    let disposition_valid = value("response-content-disposition")
        .and_then(extractor_attachment_filename)
        .zip(path_basename)
        .is_some_and(|(filename, basename)| filename == basename);
    if !disposition_valid {
        return Some(diagnostic(
            AsfExtractorRedirectRejection::ContentDisposition,
        ));
    }
    Some(diagnostic(AsfExtractorRedirectRejection::Approved))
}

fn asf_extractor_signed_object_url(url: &Url) -> bool {
    asf_extractor_signed_object_diagnostic(url)
        .is_some_and(|diagnostic| diagnostic.rejection == AsfExtractorRedirectRejection::Approved)
}

#[cfg(test)]
fn loopback_fixture_url(url: &Url) -> bool {
    url.scheme() == "http"
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && url
            .host_str()
            .and_then(|host| {
                host.trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .ok()
            })
            .is_some_and(|host| host.is_loopback())
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

struct SecretEntry {
    name: String,
    value: SecretValue,
}

impl fmt::Debug for SecretEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecretEntry")
            .field("name", &self.name)
            .field("value", &"[REDACTED]")
            .finish()
    }
}

/// Runtime-only resolved access material.
///
/// It deliberately cannot be serialized:
///
/// ```compile_fail
/// # use insar_download::ResolvedAsset;
/// fn persist(value: &ResolvedAsset) {
///     let _ = serde_json::to_value(value);
/// }
/// ```
///
/// The loopback retarget seam is unavailable to normal library consumers:
///
/// ```compile_fail
/// # use insar_download::ResolvedAsset;
/// # fn prove_normal_build_cannot_retarget(value: &mut ResolvedAsset) {
/// value.retarget_for_loopback_fixture("http://127.0.0.1/fixture");
/// # }
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ResolvedSizeAuthority {
    #[default]
    FrozenExpected,
    RemoteResponse,
}

pub struct ResolvedAsset {
    effective_url: Url,
    method: ResolvedMethod,
    headers: Vec<SecretEntry>,
    cookies: Vec<SecretEntry>,
    expires_at: Option<SystemTime>,
    expected_size: Option<u64>,
    size_authority: ResolvedSizeAuthority,
    expected_checksum: Option<ResolvedChecksum>,
    suggested_filename: Option<String>,
    remote_validator: RemoteValidator,
    authorization: Option<ResolvedAuthorization>,
    /// Runtime-only generation of the bearer token used to resolve this asset.
    /// It is intentionally not part of any durable descriptor or debug output;
    /// the download owner uses it to bind one 401 refresh to the exact request
    /// generation rather than whatever token happens to be current later.
    auth_generation: Option<u64>,
    host_policy: Option<ResolvedHostPolicy>,
    asset_role: Option<AssetRole>,
}

impl ResolvedAsset {
    pub fn new(url: &str, method: ResolvedMethod) -> Result<Self, ResolveError> {
        let effective_url = Url::parse(url).map_err(|_| ResolveError::descriptor_invalid())?;
        Ok(Self {
            effective_url,
            method,
            headers: Vec::new(),
            cookies: Vec::new(),
            expires_at: None,
            expected_size: None,
            size_authority: ResolvedSizeAuthority::FrozenExpected,
            expected_checksum: None,
            suggested_filename: None,
            remote_validator: RemoteValidator::default(),
            authorization: None,
            auth_generation: None,
            host_policy: None,
            asset_role: None,
        })
    }

    pub fn with_expiry(mut self, expires_at: SystemTime) -> Self {
        self.expires_at = Some(expires_at);
        self
    }

    pub fn with_expected_size(mut self, expected_size: Option<u64>) -> Self {
        self.expected_size = expected_size;
        self
    }

    pub(crate) fn with_remote_response_size_authority(mut self) -> Self {
        self.size_authority = ResolvedSizeAuthority::RemoteResponse;
        self
    }

    pub(crate) fn remote_response_size_is_authoritative(&self) -> bool {
        self.size_authority == ResolvedSizeAuthority::RemoteResponse
    }

    pub fn with_checksum(mut self, checksum: Option<ResolvedChecksum>) -> Self {
        self.expected_checksum = checksum;
        self
    }

    pub fn with_suggested_filename(mut self, file_name: Option<String>) -> Self {
        self.suggested_filename = file_name;
        self
    }

    pub fn with_remote_validator(mut self, validator: RemoteValidator) -> Self {
        self.remote_validator = validator;
        self
    }

    pub fn with_asset_role(mut self, role: AssetRole) -> Self {
        self.asset_role = Some(role);
        self
    }

    pub(crate) fn with_host_policy(mut self, policy: ResolvedHostPolicy) -> Self {
        self.host_policy = Some(policy);
        self
    }

    /// Retarget one already-resolved CDSE archive to a literal-loopback test
    /// fixture. This test-only seam deliberately preserves the exact `$value`
    /// path and cannot appear in a normal library build.
    #[cfg(test)]
    pub(crate) fn retarget_for_loopback_fixture(
        &mut self,
        target: &str,
    ) -> Result<(), ResolveError> {
        if self.host_policy != Some(ResolvedHostPolicy::CdseDownloadV1)
            || self.effective_url.scheme() != "https"
            || target.contains("/../")
            || target.ends_with("/..")
            || target.contains("/./")
            || target.ends_with("/.")
        {
            return Err(ResolveError::descriptor_invalid());
        }
        let target = Url::parse(target).map_err(|_| ResolveError::descriptor_invalid())?;
        if !loopback_fixture_url(&target) || target.path() != self.effective_url.path() {
            return Err(ResolveError::descriptor_invalid());
        }
        self.effective_url = target;
        self.host_policy = Some(ResolvedHostPolicy::LoopbackFixture);
        Ok(())
    }

    pub(crate) fn set_authorization(&mut self, authorization: ResolvedAuthorization) {
        self.authorization = Some(authorization);
    }

    #[cfg(test)]
    pub(crate) fn set_bearer_authorization_for_test(
        &mut self,
        token: &str,
    ) -> Result<(), ResolveError> {
        self.set_authorization(ResolvedAuthorization::Bearer(SecretValue::new(token)?));
        Ok(())
    }

    pub(crate) fn with_auth_generation(mut self, generation: u64) -> Self {
        self.auth_generation = Some(generation);
        self
    }

    pub(crate) fn auth_generation(&self) -> Option<u64> {
        self.auth_generation
    }

    pub fn insert_header(
        &mut self,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<(), ResolveError> {
        self.headers.push(secret_entry(name, value)?);
        Ok(())
    }

    pub fn insert_cookie(
        &mut self,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<(), ResolveError> {
        self.cookies.push(secret_entry(name, value)?);
        Ok(())
    }

    pub fn effective_url(&self) -> &Url {
        &self.effective_url
    }

    pub fn method(&self) -> ResolvedMethod {
        self.method
    }

    pub fn expires_at(&self) -> Option<SystemTime> {
        self.expires_at
    }

    pub fn expected_size(&self) -> Option<u64> {
        self.expected_size
    }

    pub fn expected_checksum(&self) -> Option<&ResolvedChecksum> {
        self.expected_checksum.as_ref()
    }

    pub fn suggested_filename(&self) -> Option<&str> {
        self.suggested_filename.as_deref()
    }

    pub fn remote_validator(&self) -> &RemoteValidator {
        &self.remote_validator
    }

    pub fn asset_role(&self) -> Option<&AssetRole> {
        self.asset_role.as_ref()
    }

    pub(crate) fn host_policy(&self) -> Option<ResolvedHostPolicy> {
        self.host_policy
    }

    pub(crate) fn apply_authorization(
        &self,
        builder: reqwest::blocking::RequestBuilder,
    ) -> reqwest::blocking::RequestBuilder {
        match self.authorization.as_ref() {
            Some(ResolvedAuthorization::Bearer(token)) => builder.bearer_auth(token.expose()),
            Some(ResolvedAuthorization::Basic { username, password }) => {
                builder.basic_auth(username.expose(), Some(password.expose()))
            }
            None => builder,
        }
    }

    pub(crate) fn apply_request_access(
        &self,
        builder: reqwest::blocking::RequestBuilder,
    ) -> reqwest::blocking::RequestBuilder {
        self.apply_non_authorization_access(self.apply_authorization(builder))
    }

    pub(crate) fn apply_request_access_with_bearer(
        &self,
        builder: reqwest::blocking::RequestBuilder,
        token: &str,
    ) -> reqwest::blocking::RequestBuilder {
        self.apply_non_authorization_access(builder.bearer_auth(token))
    }

    fn apply_non_authorization_access(
        &self,
        mut builder: reqwest::blocking::RequestBuilder,
    ) -> reqwest::blocking::RequestBuilder {
        for entry in &self.headers {
            if is_transfer_control_header(&entry.name) {
                continue;
            }
            builder = builder.header(entry.name.as_str(), entry.value.expose());
        }
        if !self.cookies.is_empty() {
            let mut cookie = Zeroizing::new(String::new());
            for (index, entry) in self.cookies.iter().enumerate() {
                if index > 0 {
                    cookie.push_str("; ");
                }
                cookie.push_str(&entry.name);
                cookie.push('=');
                cookie.push_str(entry.value.expose());
            }
            builder = builder.header(reqwest::header::COOKIE, cookie.as_str());
        }
        builder
    }

    pub(crate) fn auth_kind(&self) -> ResolvedAuthKind {
        match self.authorization.as_ref() {
            Some(ResolvedAuthorization::Bearer(_)) => ResolvedAuthKind::Bearer,
            Some(ResolvedAuthorization::Basic { .. }) => ResolvedAuthKind::Basic,
            None => ResolvedAuthKind::None,
        }
    }

    pub(crate) fn requires_earthdata_session(&self) -> bool {
        let production_host = self.host_policy == Some(ResolvedHostPolicy::EarthdataAsfV1);
        #[cfg(test)]
        let approved_host = production_host
            || self.host_policy == Some(ResolvedHostPolicy::EarthdataBurstLoopbackFixture);
        #[cfg(not(test))]
        let approved_host = production_host;
        approved_host && self.auth_kind() == ResolvedAuthKind::Basic
    }

    pub(crate) fn is_earthdata_burst_catalog_size_advisory_path(&self) -> bool {
        let production_path = self.host_policy == Some(ResolvedHostPolicy::EarthdataAsfV1)
            && self.effective_url.scheme() == "https"
            && self.effective_url.host_str() == Some("sentinel1-burst.asf.alaska.edu")
            && self.effective_url.port_or_known_default() == Some(443)
            && self.effective_url.username().is_empty()
            && self.effective_url.password().is_none();
        #[cfg(test)]
        let approved_path = production_path
            || self.host_policy == Some(ResolvedHostPolicy::EarthdataBurstLoopbackFixture);
        #[cfg(not(test))]
        let approved_path = production_path;
        approved_path
    }

    pub fn ensure_fresh(&self, clock: &dyn Clock) -> Result<(), ResolveError> {
        if self
            .expires_at
            .is_some_and(|expires_at| clock.now() >= expires_at)
        {
            return Err(ResolveError::expired());
        }
        Ok(())
    }
}

impl fmt::Debug for ResolvedAsset {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut redacted_url = self.effective_url.clone();
        redacted_url.set_query(None);
        redacted_url.set_fragment(None);
        if self.host_policy == Some(ResolvedHostPolicy::UsgsLandsatDownloadV1) {
            redacted_url.set_path("/[REDACTED]");
        }
        let mut header_names = self
            .headers
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>();
        if self.authorization.is_some() && !header_names.contains(&"Authorization") {
            header_names.push("Authorization");
        }
        let cookie_names = self
            .cookies
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>();
        formatter
            .debug_struct("ResolvedAsset")
            .field("effective_url", &redacted_url.as_str())
            .field("method", &self.method)
            .field("header_names", &header_names)
            .field("cookie_names", &cookie_names)
            .field("expires_at", &self.expires_at)
            .field("expected_size", &self.expected_size)
            .field("size_authority", &self.size_authority)
            .field("expected_checksum", &self.expected_checksum)
            .field("suggested_filename", &self.suggested_filename)
            .field("remote_validator", &self.remote_validator)
            .field("authorization", &self.authorization)
            .field("host_policy", &self.host_policy)
            .field("asset_role", &self.asset_role)
            .finish()
    }
}

fn is_transfer_control_header(name: &str) -> bool {
    [
        "authorization",
        "proxy-authorization",
        "cookie",
        "host",
        "content-length",
        "transfer-encoding",
        "connection",
        "range",
        "if-range",
    ]
    .iter()
    .any(|reserved| name.eq_ignore_ascii_case(reserved))
}

fn secret_entry(
    name: impl Into<String>,
    value: impl Into<String>,
) -> Result<SecretEntry, ResolveError> {
    let name = name.into();
    if name.trim().is_empty() || name.chars().any(char::is_control) {
        return Err(ResolveError::descriptor_invalid());
    }
    Ok(SecretEntry {
        name,
        value: SecretValue::new(value)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const OBSERVED_ASF_EXTRACTOR_TIFF: &str = "https://extractor-main-extractorstagingbucket-efrxkhdd3zlp.s3.amazonaws.com/S1_301495_IW3_20141003T054235_VV_D5C8-BURST/0.tiff?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=ASIAFIXTURE%2F20260826%2Fus-west-2%2Fs3%2Faws4_request&X-Amz-Date=20260826T120000Z&X-Amz-Expires=3600&X-Amz-SignedHeaders=host&X-Amz-Security-Token=FIXTURE&X-Amz-Signature=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef&X-Asf-UrsUserId=fixture-user&response-content-disposition=attachment%3B%20filename%3D%220.tiff%22";
    const REFINED_ASF_EXTRACTOR_TIFF: &str = "https://extractor-main-extractorstagingbucket-efrxkhdd3zlp.s3.amazonaws.com/products/bursts/S1_FIXTURE.tiff?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=ASIAFIXTURE%2F20260826%2Fus-west-2%2Fs3%2Faws4_request&X-Amz-Date=20260826T120000Z&X-Amz-Expires=3600&X-Amz-SignedHeaders=host&X-Amz-Security-Token=FIXTURE&X-Amz-Signature=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef&X-Asf-UrsUserId=fixture-user&response-content-disposition=attachment%3B%20filename%3D%22S1_FIXTURE.tiff%22";

    fn with_query_value(source: &Url, key: &str, replacement: &str) -> Url {
        let pairs = source
            .query_pairs()
            .map(|(candidate, value)| {
                let value = if candidate == key {
                    replacement.to_owned()
                } else {
                    value.into_owned()
                };
                (candidate.into_owned(), value)
            })
            .collect::<Vec<_>>();
        let mut updated = source.clone();
        updated.query_pairs_mut().clear().extend_pairs(pairs);
        updated
    }

    fn without_query_key(source: &Url, removed: &str) -> Url {
        let pairs = source
            .query_pairs()
            .filter(|(key, _)| key != removed)
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect::<Vec<_>>();
        let mut updated = source.clone();
        updated.query_pairs_mut().clear().extend_pairs(pairs);
        updated
    }

    #[test]
    fn extractor_redirect_diagnostic_has_fixed_safe_rejection_taxonomy() {
        let approved = Url::parse(OBSERVED_ASF_EXTRACTOR_TIFF).unwrap();
        let mut path = approved.clone();
        path.set_path("/S1/0.zip");
        let mut query_keys = approved.clone();
        query_keys
            .query_pairs_mut()
            .append_pair("x-id", "GetObject");
        let cases = [
            (approved.clone(), AsfExtractorRedirectRejection::Approved),
            (path, AsfExtractorRedirectRejection::Path),
            (query_keys.clone(), AsfExtractorRedirectRejection::QueryKeys),
            (
                with_query_value(&approved, "X-Amz-Algorithm", "unsupported"),
                AsfExtractorRedirectRejection::Algorithm,
            ),
            (
                with_query_value(
                    &approved,
                    "X-Amz-Credential",
                    "ASIAFIXTURE/20260826/us-west-2/ec2/aws4_request",
                ),
                AsfExtractorRedirectRejection::CredentialScope,
            ),
            (
                with_query_value(&approved, "X-Amz-Date", "not-a-date"),
                AsfExtractorRedirectRejection::Date,
            ),
            (
                with_query_value(&approved, "X-Amz-Expires", "0"),
                AsfExtractorRedirectRejection::Expires,
            ),
            (
                with_query_value(&approved, "X-Amz-SignedHeaders", "host;x-extra"),
                AsfExtractorRedirectRejection::SignedHeaders,
            ),
            (
                with_query_value(&approved, "X-Amz-Security-Token", ""),
                AsfExtractorRedirectRejection::TokenPresence,
            ),
            (
                with_query_value(&approved, "X-Amz-Signature", "short"),
                AsfExtractorRedirectRejection::Signature,
            ),
            (
                with_query_value(&approved, "X-Asf-UrsUserId", "unsafe/value"),
                AsfExtractorRedirectRejection::AsfUserId,
            ),
            (
                with_query_value(
                    &approved,
                    "response-content-disposition",
                    "attachment; filename=\"other.tiff\"",
                ),
                AsfExtractorRedirectRejection::ContentDisposition,
            ),
        ];
        for (url, expected) in cases {
            let diagnostic = asf_extractor_signed_object_diagnostic(&url).unwrap();
            assert_eq!(diagnostic.rejection, expected);
            assert!(diagnostic
                .query_keys
                .windows(2)
                .all(|pair| pair[0] <= pair[1]));
            let fingerprint = format!("{diagnostic:?}");
            assert!(!fingerprint.contains("ASIAFIXTURE"));
            assert!(!fingerprint.contains("0123456789abcdef"));
            assert!(!fingerprint.contains("GetObject"));
        }
        let query_key_diagnostic = asf_extractor_signed_object_diagnostic(&query_keys).unwrap();
        assert!(query_key_diagnostic
            .query_keys
            .iter()
            .any(|key| key == "x-id"));
        assert!(!query_key_diagnostic
            .query_keys
            .iter()
            .any(|key| key == "unknown"));

        let path_cases = [
            (
                "/",
                AsfExtractorPathSegmentCount::Zero,
                AsfExtractorPathTerminal::Unsupported,
                false,
                true,
            ),
            (
                "/0.tiff",
                AsfExtractorPathSegmentCount::One,
                AsfExtractorPathTerminal::Exact0Tiff,
                false,
                false,
            ),
            (
                "/scene/other.tiff",
                AsfExtractorPathSegmentCount::Two,
                AsfExtractorPathTerminal::OtherTiff,
                false,
                false,
            ),
            (
                "/a/b/0.xml",
                AsfExtractorPathSegmentCount::ThreePlus,
                AsfExtractorPathTerminal::Exact0Xml,
                false,
                false,
            ),
            (
                "/a/b/metadata.xml",
                AsfExtractorPathSegmentCount::ThreePlus,
                AsfExtractorPathTerminal::OtherXml,
                false,
                false,
            ),
            (
                "/a/b/0.zip",
                AsfExtractorPathSegmentCount::ThreePlus,
                AsfExtractorPathTerminal::Unsupported,
                false,
                false,
            ),
            (
                "/scene%2Fescape/0.tiff/",
                AsfExtractorPathSegmentCount::Two,
                AsfExtractorPathTerminal::Exact0Tiff,
                true,
                true,
            ),
        ];
        for (path, segment_count, terminal, encoded, trailing_slash) in path_cases {
            let mut url = approved.clone();
            url.set_path(path);
            let diagnostic = asf_extractor_signed_object_diagnostic(&url).unwrap();
            assert_eq!(diagnostic.path_segment_count, segment_count);
            assert_eq!(diagnostic.path_terminal, terminal);
            assert_eq!(diagnostic.path_has_encoded_separator_or_dot, encoded);
            assert_eq!(diagnostic.path_trailing_slash, trailing_slash);
            let fingerprint = format!("{diagnostic:?}");
            assert!(!fingerprint.contains("scene"));
            assert!(!fingerprint.contains("escape"));
        }

        for unsafe_name in [
            "unsafe/key".to_owned(),
            "非ASCII".to_owned(),
            "x".repeat(65),
        ] {
            let mut unsafe_key = approved.clone();
            unsafe_key
                .query_pairs_mut()
                .append_pair(&unsafe_name, "SECRET_QUERY_VALUE");
            let unsafe_key = asf_extractor_signed_object_diagnostic(&unsafe_key).unwrap();
            assert!(unsafe_key.query_keys.iter().any(|key| key == "invalid_key"));
            let fingerprint = format!("{unsafe_key:?}");
            assert!(!fingerprint.contains(&unsafe_name));
            assert!(!fingerprint.contains("SECRET_QUERY_VALUE"));
        }
    }

    #[test]
    fn earthdata_asf_extractor_redirect_is_exact_and_never_authenticated() {
        let policy = ResolvedHostPolicy::EarthdataAsfV1;
        let initial = Url::parse("https://sentinel1-burst.asf.alaska.edu/0.tiff").unwrap();
        let observed = Url::parse(OBSERVED_ASF_EXTRACTOR_TIFF).unwrap();
        assert!(policy.allows_response_url(&observed));
        assert!(policy.allows_redirect(&initial, &observed));
        assert!(!policy.allows_authenticated_request_url(&observed));

        for href in [
            "https://extractor-main-extractorstagingbucket-efrxkhdd3zlp.s3.amazonaws.com.evil.example/S1/0.tiff?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=x&X-Amz-Date=20260826T120000Z&X-Amz-Expires=3600&X-Amz-SignedHeaders=host&X-Amz-Signature=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "https://another-bucket.s3.amazonaws.com/S1/0.tiff?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=x&X-Amz-Date=20260826T120000Z&X-Amz-Expires=3600&X-Amz-SignedHeaders=host&X-Amz-Signature=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "http://extractor-main-extractorstagingbucket-efrxkhdd3zlp.s3.amazonaws.com/S1/0.tiff?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=x&X-Amz-Date=20260826T120000Z&X-Amz-Expires=3600&X-Amz-SignedHeaders=host&X-Amz-Signature=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "https://user@extractor-main-extractorstagingbucket-efrxkhdd3zlp.s3.amazonaws.com/S1/0.tiff?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=x&X-Amz-Date=20260826T120000Z&X-Amz-Expires=3600&X-Amz-SignedHeaders=host&X-Amz-Signature=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "https://extractor-main-extractorstagingbucket-efrxkhdd3zlp.s3.amazonaws.com/S1/0.xml?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=x&X-Amz-Date=20260826T120000Z&X-Amz-Expires=3600&X-Amz-SignedHeaders=host&X-Amz-Signature=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "https://extractor-main-extractorstagingbucket-efrxkhdd3zlp.s3.amazonaws.com/S1/../0.tiff?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=x&X-Amz-Date=20260826T120000Z&X-Amz-Expires=3600&X-Amz-SignedHeaders=host&X-Amz-Signature=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "https://extractor-main-extractorstagingbucket-efrxkhdd3zlp.s3.amazonaws.com/S1/0.tiff",
            "https://extractor-main-extractorstagingbucket-efrxkhdd3zlp.s3.amazonaws.com/S1/0.tiff?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=x&X-Amz-Date=20260826T120000Z&X-Amz-Expires=3600&X-Amz-SignedHeaders=host&X-Amz-Signature=short",
            "https://extractor-main-extractorstagingbucket-efrxkhdd3zlp.s3.amazonaws.com/S1/0.tiff?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=ASIAFIXTURE%2F20260826%2Fus-west-2%2Fs3%2Faws4_request&X-Amz-Date=20260826T120000Z&X-Amz-Expires=3600&X-Amz-SignedHeaders=host&X-Amz-Signature=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef&x-id=GetObject",
        ] {
            let candidate = Url::parse(href).unwrap();
            assert!(!policy.allows_response_url(&candidate), "unsafe redirect approved");
            assert!(!policy.allows_redirect(&initial, &candidate));
            assert!(!policy.allows_authenticated_request_url(&candidate));
        }
    }

    #[test]
    fn earthdata_asf_extractor_redirect_admits_a_signed_burst_xml_like_its_tiff() {
        // Real Burst XML assets redirect to the same signed extractor object host as the TIFF.
        let policy = ResolvedHostPolicy::EarthdataAsfV1;
        let initial = Url::parse("https://sentinel1-burst.asf.alaska.edu/0.xml").unwrap();
        let tiff = Url::parse(OBSERVED_ASF_EXTRACTOR_TIFF).unwrap();
        let signed = |path: &str, filename: &str| {
            let mut url = with_query_value(
                &tiff,
                "response-content-disposition",
                &format!("attachment; filename=\"{filename}\""),
            );
            url.set_path(path);
            url
        };
        for approved in [
            signed(
                "/S1_301495_IW3_20141003T054235_VV_D5C8-BURST/0.xml",
                "0.xml",
            ),
            signed("/products/bursts/S1_FIXTURE.xml", "S1_FIXTURE.xml"),
        ] {
            assert!(policy.allows_response_url(&approved), "{approved}");
            assert!(policy.allows_redirect(&initial, &approved));
            assert!(!policy.allows_authenticated_request_url(&approved));
        }
        for rejected in [
            signed(
                "/S1_301495_IW3_20141003T054235_VV_D5C8-BURST/0.xml",
                "0.tiff",
            ),
            signed(
                "/S1_301495_IW3_20141003T054235_VV_D5C8-BURST/0.tiff",
                "0.xml",
            ),
            signed("/S1/other.xml", "other.xml"),
            signed("/products/bursts/S1_FIXTURE.zip", "S1_FIXTURE.zip"),
            without_query_key(
                &signed("/products/bursts/S1_FIXTURE.xml", "S1_FIXTURE.xml"),
                "X-Amz-Signature",
            ),
        ] {
            assert!(!policy.allows_response_url(&rejected), "{rejected}");
            assert!(!policy.allows_redirect(&initial, &rejected));
        }
    }

    #[test]
    fn earthdata_asf_refined_extractor_redirect_is_narrow_and_value_bound() {
        let policy = ResolvedHostPolicy::EarthdataAsfV1;
        let initial = Url::parse("https://sentinel1-burst.asf.alaska.edu/0.tiff").unwrap();
        let refined = Url::parse(REFINED_ASF_EXTRACTOR_TIFF).unwrap();
        assert!(policy.allows_response_url(&refined));
        assert!(policy.allows_redirect(&initial, &refined));
        assert!(!policy.allows_authenticated_request_url(&refined));
        let without_session_token = without_query_key(&refined, "X-Amz-Security-Token");
        assert!(policy.allows_response_url(&without_session_token));
        assert!(!policy.allows_authenticated_request_url(&without_session_token));

        let mut bad_paths = Vec::new();
        for path in [
            "/products/other.tiff",
            "/products/bursts/not-tiff.xml",
            "/products//S1_FIXTURE.tiff",
            "/products/%2e%2e/S1_FIXTURE.tiff",
            "/products/bursts/S1_FIXTURE.tiff/",
            "/products/%0A/S1_FIXTURE.tiff",
        ] {
            let mut candidate = refined.clone();
            candidate.set_path(path);
            bad_paths.push(candidate);
        }
        let mut long_segment = refined.clone();
        long_segment.set_path(&format!("/products/{}/S1_FIXTURE.tiff", "x".repeat(256)));
        bad_paths.push(long_segment);
        let mut long_path = refined.clone();
        long_path.set_path(&format!(
            "/products/{}/{}/{}/{}/{}/{}/{}/{}/S1_FIXTURE.tiff",
            "x".repeat(255),
            "x".repeat(255),
            "x".repeat(255),
            "x".repeat(255),
            "x".repeat(255),
            "x".repeat(255),
            "x".repeat(255),
            "x".repeat(255),
        ));
        bad_paths.push(long_path);
        let mut fragment = refined.clone();
        fragment.set_fragment(Some("rejected"));
        bad_paths.push(fragment);
        let mut nondefault_port = refined.clone();
        nondefault_port.set_port(Some(444)).unwrap();
        bad_paths.push(nondefault_port);

        let mut duplicate_user = refined.clone();
        duplicate_user
            .query_pairs_mut()
            .append_pair("X-Asf-UrsUserId", "fixture-user");
        let mut duplicate_disposition = refined.clone();
        duplicate_disposition.query_pairs_mut().append_pair(
            "response-content-disposition",
            "attachment; filename=\"S1_FIXTURE.tiff\"",
        );
        let mut unknown = refined.clone();
        unknown.query_pairs_mut().append_pair("x-id", "GetObject");
        let bad_queries = [
            without_query_key(&refined, "X-Asf-UrsUserId"),
            without_query_key(&refined, "response-content-disposition"),
            duplicate_user,
            duplicate_disposition,
            unknown,
            with_query_value(&refined, "X-Asf-UrsUserId", ""),
            with_query_value(&refined, "X-Asf-UrsUserId", "unsafe/value"),
            with_query_value(&refined, "X-Asf-UrsUserId", &"x".repeat(129)),
            with_query_value(
                &refined,
                "response-content-disposition",
                "inline; filename=\"S1_FIXTURE.tiff\"",
            ),
            with_query_value(
                &refined,
                "response-content-disposition",
                "attachment; filename=\"other.tiff\"",
            ),
            with_query_value(
                &refined,
                "response-content-disposition",
                "attachment; filename=\"S1_FIXTURE.xml\"",
            ),
            with_query_value(
                &refined,
                "response-content-disposition",
                "attachment; filename=\"S1_FIXTURE.tiff\"\r\nX-Evil: yes",
            ),
        ];
        for candidate in bad_paths.into_iter().chain(bad_queries) {
            assert!(
                !policy.allows_response_url(&candidate),
                "unsafe redirect approved"
            );
            assert!(!policy.allows_redirect(&initial, &candidate));
            assert!(!policy.allows_authenticated_request_url(&candidate));
        }
    }

    #[test]
    fn cdse_download_host_policy_is_exact_and_never_uses_suffix_matching() {
        let policy = ResolvedHostPolicy::CdseDownloadV1;
        assert!(policy.allows_response_url(
            &Url::parse("https://download.dataspace.copernicus.eu/odata/v1/Products(00000000-0000-4000-8000-000000000001)/$value")
                .expect("exact CDSE URL")
        ));
        for href in [
            "https://catalogue.dataspace.copernicus.eu/odata/v1/Products(00000000-0000-4000-8000-000000000001)/$value",
            "https://identity.dataspace.copernicus.eu/odata/v1/Products(00000000-0000-4000-8000-000000000001)/$value",
            "https://sub.download.dataspace.copernicus.eu/file",
            "https://download.dataspace.copernicus.eu.evil.example/file",
            "http://download.dataspace.copernicus.eu/file",
            "https://download.dataspace.copernicus.eu:444/file",
            "https://user@download.dataspace.copernicus.eu/file",
            "https://example.invalid/file",
            "https://127.0.0.1/file",
        ] {
            assert!(
                !policy.allows_response_url(&Url::parse(href).expect("syntactic test URL")),
                "unexpected approved CDSE response host: {href}"
            );
        }
    }

    #[test]
    fn loopback_retarget_is_test_only_literal_and_path_preserving() {
        let production = "https://download.dataspace.copernicus.eu/odata/v1/Products(00000000-0000-4000-8000-000000000001)/$value";
        let path = "/odata/v1/Products(00000000-0000-4000-8000-000000000001)/$value";
        let asset = || {
            ResolvedAsset::new(production, ResolvedMethod::Get)
                .expect("production URL")
                .with_host_policy(ResolvedHostPolicy::CdseDownloadV1)
        };

        let mut retargeted = asset();
        retargeted
            .retarget_for_loopback_fixture(&format!("http://127.0.0.1:43123{path}"))
            .expect("literal-loopback test target");
        assert_eq!(retargeted.effective_url().path(), path);
        assert_eq!(
            retargeted.host_policy(),
            Some(ResolvedHostPolicy::LoopbackFixture)
        );
        assert!(retargeted
            .host_policy()
            .expect("loopback policy")
            .allows_response_url(retargeted.effective_url()));

        for target in [
            format!("https://127.0.0.1:43123{path}"),
            format!("http://localhost:43123{path}"),
            format!("http://user@127.0.0.1:43123{path}"),
            format!("http://127.0.0.1:43123{path}?query=1"),
            format!("http://127.0.0.1:43123{path}#fragment"),
            "http://127.0.0.1:43123/other".to_owned(),
            "http://127.0.0.1:43123/odata/v1/Products(00000000-0000-4000-8000-000000000001)/../$value".to_owned(),
            format!("http://example.invalid:43123{path}"),
        ] {
            assert!(
                asset().retarget_for_loopback_fixture(&target).is_err(),
                "test seam accepted unsafe target: {target}"
            );
        }
    }

    #[test]
    fn loopback_fixture_policy_accepts_only_literal_loopback_http_without_locator_suffixes() {
        let policy = ResolvedHostPolicy::LoopbackFixture;
        for href in [
            "http://127.0.0.1:43123/fixture",
            "http://[::1]:43123/fixture",
        ] {
            assert!(policy.allows_response_url(&Url::parse(href).expect("fixture URL")));
        }
        for href in [
            "https://127.0.0.1:43123/fixture",
            "http://localhost:43123/fixture",
            "http://127.0.0.1:43123/fixture?query=1",
            "http://127.0.0.1:43123/fixture#fragment",
            "http://user@127.0.0.1:43123/fixture",
        ] {
            assert!(
                !policy.allows_response_url(&Url::parse(href).expect("syntactic fixture URL")),
                "unsafe loopback fixture URL was approved: {href}"
            );
        }
    }
}
