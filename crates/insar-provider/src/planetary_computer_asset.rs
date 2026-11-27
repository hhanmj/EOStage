use std::collections::BTreeSet;
use std::fmt;
use std::io::Read;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use insar_core::{ContractError, ProviderNetworkPolicy, ProviderResult};
use reqwest::blocking::Client;
use reqwest::header::{HeaderValue, CONTENT_LENGTH, CONTENT_RANGE, RANGE, RETRY_AFTER};
use reqwest::redirect::Policy;
use reqwest::{StatusCode, Url};
use serde::Deserialize;
use serde_json::{Map, Value};

pub const PLANETARY_COMPUTER_ASSET_SIGN_ORIGIN: &str = "https://planetarycomputer.microsoft.com";
pub const PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID: &str = "microsoft.planetary-computer";
pub const MPC_SENTINEL1_GRD_COLLECTION_ID: &str = "sentinel-1-grd";
pub const MPC_COPDEM_GLO30_COLLECTION_ID: &str = "cop-dem-glo-30";
pub const MPC_SENTINEL1_GRD_LOGICAL_DATASET: &str = "sar.sentinel-1.grd";
pub const MPC_SENTINEL1_GRD_PROFILE_ID: &str = "planetary-computer.sentinel-1-grd.cog.v1";
pub const MPC_COPDEM_GLO30_LOGICAL_DATASET: &str = "dem.cop30";
pub const MPC_COPDEM_GLO30_PROFILE_ID: &str = "planetary-computer.cop-dem-glo-30.cog.v1";
pub const MPC_SENTINEL2_L2A_BAND_LOGICAL_DATASET: &str = "optical.sentinel-2.l2a";
pub const MPC_SENTINEL2_L2A_BAND_PROFILE_ID: &str = "planetary-computer.sentinel-2-l2a.band-cog.v1";
pub const MPC_SENTINEL2_L2A_BAND_ASSET_POLICY_ID: &str =
    "planetary-computer.sentinel-2-l2a.band-cog.asset.v1";
pub const MPC_REVIEWED_STAC_IDENTITY_RULE_ID: &str = "stac.item-id.exact.v1";
pub const MPC_SENTINEL1_GRD_FILENAME_RULE_ID: &str =
    "planetary-computer.sentinel-1-grd.cog-filename.v1";
pub const MPC_COPDEM_GLO30_FILENAME_RULE_ID: &str =
    "planetary-computer.cop-dem-glo-30.cog-filename.v1";
pub const MPC_SENTINEL1_GRD_ASSET_POLICY_ID: &str =
    "planetary-computer.sentinel-1-grd.cog.asset.v1";
pub const MPC_COPDEM_GLO30_ASSET_POLICY_ID: &str = "planetary-computer.cop-dem-glo-30.cog.asset.v1";
const SIGN_PATH: &str = "/api/sas/v1/sign";
const MAX_SIGN_RESPONSE_BYTES: u64 = 32 * 1024;
const MAX_SIGNED_HREF_CHARS: usize = 16 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const SIGN_REQUEST_ATTEMPTS: usize = 4;
const SIGN_TRANSPORT_ATTEMPTS: usize = 2;
const SIGN_RETRY_POLL: Duration = Duration::from_millis(25);
const MAX_REPORTED_RETRY_AFTER_SECONDS: u64 = 3_600;
const MAX_SIGN_RETRY_DELAY: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanetaryComputerReviewedAssetClass {
    Sentinel1GrdCog,
    CopDemGlo30Cog,
    Sentinel2L2aBandCog,
}

impl PlanetaryComputerReviewedAssetClass {
    pub const fn collection_id(self) -> &'static str {
        match self {
            Self::Sentinel1GrdCog => MPC_SENTINEL1_GRD_COLLECTION_ID,
            Self::CopDemGlo30Cog => MPC_COPDEM_GLO30_COLLECTION_ID,
            Self::Sentinel2L2aBandCog => "sentinel-2-l2a",
        }
    }

    pub const fn logical_dataset(self) -> &'static str {
        match self {
            Self::Sentinel1GrdCog => MPC_SENTINEL1_GRD_LOGICAL_DATASET,
            Self::CopDemGlo30Cog => MPC_COPDEM_GLO30_LOGICAL_DATASET,
            Self::Sentinel2L2aBandCog => MPC_SENTINEL2_L2A_BAND_LOGICAL_DATASET,
        }
    }

    pub const fn collection_profile_id(self) -> &'static str {
        match self {
            Self::Sentinel1GrdCog => MPC_SENTINEL1_GRD_PROFILE_ID,
            Self::CopDemGlo30Cog => MPC_COPDEM_GLO30_PROFILE_ID,
            Self::Sentinel2L2aBandCog => MPC_SENTINEL2_L2A_BAND_PROFILE_ID,
        }
    }

    const fn host(self) -> &'static str {
        match self {
            Self::Sentinel1GrdCog => "sentinel1euwest.blob.core.windows.net",
            Self::CopDemGlo30Cog => "elevationeuwest.blob.core.windows.net",
            Self::Sentinel2L2aBandCog => "sentinel2l2a01.blob.core.windows.net",
        }
    }

    const fn path_prefix(self) -> &'static str {
        match self {
            Self::Sentinel1GrdCog => "/s1-grd/GRD/",
            Self::CopDemGlo30Cog => "/copernicus-dem/COP30_hh/",
            Self::Sentinel2L2aBandCog => "/sentinel2-l2/",
        }
    }

    const fn file_suffix(self) -> &'static str {
        match self {
            Self::Sentinel1GrdCog => ".tiff",
            Self::CopDemGlo30Cog | Self::Sentinel2L2aBandCog => ".tif",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanetaryComputerReviewedAssetEvidence {
    pub file_name: String,
    pub expected_size_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanetaryComputerResolvedAssetEvidence {
    pub stable_href: String,
    pub file_name: String,
    pub expected_size_bytes: u64,
}
pub struct PlanetaryComputerRuntimeAsset {
    url: Url,
    expires_at: SystemTime,
}

impl PlanetaryComputerRuntimeAsset {
    pub fn into_parts(self) -> (Url, SystemTime) {
        (self.url, self.expires_at)
    }
}

impl fmt::Debug for PlanetaryComputerRuntimeAsset {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PlanetaryComputerRuntimeAsset")
            .field("url", &"[REDACTED_SIGNED_URL]")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

struct CollectionSasEntry {
    class: PlanetaryComputerReviewedAssetClass,
    url: Url,
    expires_at: SystemTime,
}

#[derive(Default)]
struct CollectionSasCache {
    entries: Vec<CollectionSasEntry>,
}

impl CollectionSasCache {
    fn insert(
        &mut self,
        class: PlanetaryComputerReviewedAssetClass,
        stable: &Url,
        runtime: PlanetaryComputerRuntimeAsset,
    ) {
        if validate_planetary_computer_runtime_asset_url(class, &runtime.url).is_err() {
            return;
        }
        let mut unsigned = runtime.url.clone();
        unsigned.set_query(None);
        if unsigned != *stable {
            return;
        }
        self.entries.retain(|entry| entry.class != class);
        self.entries.push(CollectionSasEntry {
            class,
            url: runtime.url,
            expires_at: runtime.expires_at,
        });
    }

    fn lookup(
        &self,
        class: PlanetaryComputerReviewedAssetClass,
        stable: &Url,
    ) -> Option<PlanetaryComputerRuntimeAsset> {
        let entry = self.entries.iter().find(|entry| entry.class == class)?;
        if SystemTime::now().checked_add(Duration::from_secs(30))? >= entry.expires_at {
            return None;
        }
        let mut url = stable.clone();
        url.set_query(entry.url.query());
        validate_planetary_computer_runtime_asset_url(class, &url).ok()?;
        Some(PlanetaryComputerRuntimeAsset {
            url,
            expires_at: entry.expires_at,
        })
    }
}

pub struct PlanetaryComputerAssetSigner {
    client: Client,
    network_policy: ProviderNetworkPolicy,
    origin: Url,
    collection_cache: Mutex<CollectionSasCache>,
}

impl PlanetaryComputerAssetSigner {
    pub fn new(network_policy: ProviderNetworkPolicy) -> ProviderResult<Self> {
        let origin = Url::parse(PLANETARY_COMPUTER_ASSET_SIGN_ORIGIN)
            .map_err(|_| asset_error("MPC_ASSET_CONFIG_INVALID", "sign_origin"))?;
        Self::with_origin(network_policy, origin)
    }

    fn with_origin(network_policy: ProviderNetworkPolicy, origin: Url) -> ProviderResult<Self> {
        let mut builder = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(REQUEST_TIMEOUT)
            .redirect(Policy::none())
            .retry(reqwest::retry::never())
            .referer(false)
            .user_agent("EOStage/PlanetaryComputerAssetSigner");
        if origin.scheme() == "http"
            && origin
                .host_str()
                .and_then(|host| host.parse::<std::net::IpAddr>().ok())
                .is_some_and(|ip| ip.is_loopback())
        {
            builder = builder.tls_built_in_root_certs(false);
        }
        let builder =
            crate::network_proxy::blocking_builder_with_proxy(builder, network_policy.route());
        let client = builder
            .build()
            .map_err(|_| asset_error("MPC_ASSET_CONFIG_INVALID", "http_client"))?;
        Ok(Self {
            client,
            network_policy,
            origin,
            collection_cache: Mutex::new(CollectionSasCache::default()),
        })
    }

    pub const fn network_policy(&self) -> ProviderNetworkPolicy {
        self.network_policy
    }

    pub fn invalidate_collection_sas(&self, class: PlanetaryComputerReviewedAssetClass) {
        if let Ok(mut cache) = self.collection_cache.lock() {
            cache.entries.retain(|entry| entry.class != class);
        }
    }

    pub fn sign_reviewed_asset(
        &self,
        class: PlanetaryComputerReviewedAssetClass,
        stable_href: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<PlanetaryComputerRuntimeAsset> {
        check_cancelled(cancelled)?;
        let stable = validate_planetary_computer_stable_asset(class, stable_href)?;
        let mut cache = self
            .collection_cache
            .lock()
            .map_err(|_| asset_error("MPC_ASSET_CONFIG_INVALID", "cache_lock"))?;
        if let Some(cached) = cache.lookup(class, &stable) {
            return Ok(cached);
        }
        let mut sign_url = self
            .origin
            .join(SIGN_PATH)
            .map_err(|_| asset_error("MPC_ASSET_CONFIG_INVALID", "sign_path"))?;
        {
            let mut query = sign_url.query_pairs_mut();
            query.clear().append_pair("href", stable.as_str());
        }
        let mut response = {
            let mut accepted = None;
            let mut transport_failures = 0_usize;
            for attempt in 0..SIGN_REQUEST_ATTEMPTS {
                check_cancelled(cancelled)?;
                match self.client.get(sign_url.clone()).send() {
                    Ok(candidate) if candidate.status() == StatusCode::OK => {
                        accepted = Some(candidate);
                        break;
                    }
                    Ok(candidate) => {
                        check_cancelled(cancelled)?;
                        let status = candidate.status();
                        if attempt + 1 < SIGN_REQUEST_ATTEMPTS && retryable_sign_status(status) {
                            wait_sign_retry_delay(
                                sign_retry_delay_for_response(
                                    attempt,
                                    status,
                                    candidate.headers().get(RETRY_AFTER),
                                ),
                                cancelled,
                            )?;
                            continue;
                        }
                        return Err(sign_status_error(
                            status,
                            candidate.headers().get(RETRY_AFTER),
                            attempt + 1,
                        ));
                    }
                    Err(error) => {
                        transport_failures += 1;
                        if transport_failures < SIGN_TRANSPORT_ATTEMPTS
                            && attempt + 1 < SIGN_REQUEST_ATTEMPTS
                        {
                            wait_sign_retry(attempt, cancelled)?;
                            continue;
                        }
                        return Err(sign_transport_error(&error, attempt + 1));
                    }
                }
            }
            accepted.ok_or_else(|| {
                sign_failure(
                    "MPC_ASSET_SIGN_TRANSPORT",
                    "transport",
                    &[("attempts", SIGN_REQUEST_ATTEMPTS.to_string())],
                )
            })?
        };
        check_cancelled(cancelled)?;
        if response
            .content_length()
            .is_some_and(|size| size > MAX_SIGN_RESPONSE_BYTES)
        {
            return Err(invalid_sign_response("response_size"));
        }
        let mut bytes = Vec::new();
        response
            .by_ref()
            .take(MAX_SIGN_RESPONSE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| {
                let transport = if error.kind() == std::io::ErrorKind::TimedOut {
                    "timeout"
                } else {
                    "other"
                };
                sign_failure(
                    "MPC_ASSET_SIGN_TRANSPORT",
                    "response_read",
                    &[("transport", transport.to_owned())],
                )
            })?;
        if bytes.is_empty() || bytes.len() as u64 > MAX_SIGN_RESPONSE_BYTES {
            return Err(invalid_sign_response("response_size"));
        }
        let payload: SignResponse =
            serde_json::from_slice(&bytes).map_err(|_| invalid_sign_response("response_json"))?;
        check_cancelled(cancelled)?;
        let runtime = validate_signed_response(class, &stable, payload)?;
        cache.insert(
            class,
            &stable,
            PlanetaryComputerRuntimeAsset {
                url: runtime.url.clone(),
                expires_at: runtime.expires_at,
            },
        );
        Ok(runtime)
    }

    /// Obtain immutable transfer evidence without downloading the object.
    /// The request is a single signed one-byte Range read; only the total object
    /// size from Content-Range is retained. Signed URLs remain runtime-only.
    pub fn probe_reviewed_asset(
        &self,
        class: PlanetaryComputerReviewedAssetClass,
        provider_product_id: &str,
        asset_key: &str,
        stable_href: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<PlanetaryComputerReviewedAssetEvidence> {
        let file_name = validate_planetary_computer_reviewed_asset_identity(
            class,
            provider_product_id,
            asset_key,
            stable_href,
        )?;
        let runtime = self.sign_reviewed_asset(class, stable_href, cancelled)?;
        check_cancelled(cancelled)?;
        let (url, _) = runtime.into_parts();
        let mut response = self
            .client
            .get(url)
            .header(RANGE, "bytes=0-0")
            .send()
            .map_err(|_| asset_error("MPC_ASSET_PROBE_FAILED", "transport"))?;
        check_cancelled(cancelled)?;
        if response.status() != StatusCode::PARTIAL_CONTENT
            || response
                .headers()
                .get(CONTENT_LENGTH)
                .and_then(|value| value.to_str().ok())
                != Some("1")
        {
            return Err(asset_error("MPC_ASSET_PROBE_FAILED", "range_status"));
        }
        let total = response
            .headers()
            .get(CONTENT_RANGE)
            .and_then(|value| value.to_str().ok())
            .and_then(parse_one_byte_content_range)
            .filter(|value| *value > 1)
            .ok_or_else(|| asset_error("MPC_ASSET_PROBE_FAILED", "content_range"))?;
        let mut byte = [0_u8; 2];
        let read = response
            .read(&mut byte)
            .map_err(|_| asset_error("MPC_ASSET_PROBE_FAILED", "range_read"))?;
        if read != 1 {
            return Err(asset_error("MPC_ASSET_PROBE_FAILED", "range_body"));
        }
        check_cancelled(cancelled)?;
        Ok(PlanetaryComputerReviewedAssetEvidence {
            file_name,
            expected_size_bytes: total,
        })
    }

    /// Re-resolve one exact reviewed STAC Item/asset and freeze only stable
    /// object identity plus one-byte Range size evidence. No signed URL leaves
    /// this provider boundary.
    pub fn resolve_reviewed_asset(
        &self,
        class: PlanetaryComputerReviewedAssetClass,
        provider_product_id: &str,
        asset_key: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<PlanetaryComputerResolvedAssetEvidence> {
        check_cancelled(cancelled)?;
        if !safe_identity(provider_product_id)
            || match class {
                PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog => {
                    !matches!(asset_key, "vv" | "vh" | "hh" | "hv")
                }
                PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog => asset_key != "data",
                PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog => {
                    sentinel2_l2a_reflectance_band_resolution(asset_key).is_none()
                }
            }
        {
            return Err(asset_error("MPC_ASSET_REJECTED", "item_identity"));
        }
        let item_url = self
            .origin
            .join(&format!(
                "/api/stac/v1/collections/{}/items/{provider_product_id}",
                class.collection_id()
            ))
            .map_err(|_| asset_error("MPC_ASSET_CONFIG_INVALID", "item_url"))?;
        let mut response = self
            .client
            .get(item_url)
            .send()
            .map_err(|_| asset_error("MPC_ASSET_ITEM_FAILED", "transport"))?;
        check_cancelled(cancelled)?;
        if response.status() != StatusCode::OK
            || response
                .content_length()
                .is_some_and(|size| size > 2 * 1024 * 1024)
        {
            return Err(asset_error("MPC_ASSET_ITEM_FAILED", "http_status_or_size"));
        }
        let mut bytes = Vec::new();
        response
            .by_ref()
            .take(2 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| asset_error("MPC_ASSET_ITEM_FAILED", "response_read"))?;
        if bytes.is_empty() || bytes.len() > 2 * 1024 * 1024 {
            return Err(asset_error("MPC_ASSET_ITEM_FAILED", "response_size"));
        }
        let item: Value = serde_json::from_slice(&bytes)
            .map_err(|_| asset_error("MPC_ASSET_ITEM_FAILED", "response_json"))?;
        let object = item
            .as_object()
            .ok_or_else(|| asset_error("MPC_ASSET_ITEM_FAILED", "item_object"))?;
        if object.get("type").and_then(Value::as_str) != Some("Feature")
            || object.get("id").and_then(Value::as_str) != Some(provider_product_id)
            || object.get("collection").and_then(Value::as_str) != Some(class.collection_id())
            || !matches!(
                object.get("stac_version").and_then(Value::as_str),
                Some("1.0.0" | "1.1.0")
            )
        {
            return Err(asset_error("MPC_ASSET_ITEM_FAILED", "item_scope"));
        }
        let assets = object
            .get("assets")
            .and_then(Value::as_object)
            .filter(|assets| assets.len() <= 64)
            .ok_or_else(|| asset_error("MPC_ASSET_ITEM_FAILED", "assets"))?;
        let asset = assets
            .get(asset_key)
            .and_then(Value::as_object)
            .ok_or_else(|| asset_error("MPC_ASSET_ITEM_FAILED", "asset"))?;
        let stable_href = asset
            .get("href")
            .and_then(Value::as_str)
            .ok_or_else(|| asset_error("MPC_ASSET_ITEM_FAILED", "asset_href"))?;
        if !asset
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(|value| value.starts_with("image/tiff"))
            || !asset
                .get("roles")
                .and_then(Value::as_array)
                .is_some_and(|roles| {
                    !roles.is_empty()
                        && roles.len() <= 8
                        && roles.iter().any(|role| role.as_str() == Some("data"))
                })
        {
            return Err(asset_error("MPC_ASSET_ITEM_FAILED", "asset_semantics"));
        }
        let file_name = validate_planetary_computer_reviewed_asset_identity(
            class,
            provider_product_id,
            asset_key,
            stable_href,
        )?;
        let evidence = self.probe_reviewed_asset(
            class,
            provider_product_id,
            asset_key,
            stable_href,
            cancelled,
        )?;
        if evidence.file_name != file_name {
            return Err(asset_error("MPC_ASSET_ITEM_FAILED", "asset_identity"));
        }
        Ok(PlanetaryComputerResolvedAssetEvidence {
            stable_href: stable_href.to_owned(),
            file_name,
            expected_size_bytes: evidence.expected_size_bytes,
        })
    }
}

impl fmt::Debug for PlanetaryComputerAssetSigner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PlanetaryComputerAssetSigner")
            .field("network_policy", &self.network_policy)
            .field("origin", &self.origin)
            .finish()
    }
}

pub fn validate_planetary_computer_stable_asset(
    class: PlanetaryComputerReviewedAssetClass,
    value: &str,
) -> ProviderResult<Url> {
    let url = Url::parse(value).map_err(|_| asset_error("MPC_ASSET_REJECTED", "stable_href"))?;
    let path_lower = url.path().to_ascii_lowercase();
    if url.scheme() != "https"
        || url.host_str() != Some(class.host())
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.path().starts_with(class.path_prefix())
        || !path_lower.ends_with(class.file_suffix())
        || ["%2f", "%5c", "%2e"]
            .iter()
            .any(|needle| path_lower.contains(needle))
    {
        return Err(asset_error("MPC_ASSET_REJECTED", "stable_scope"));
    }
    Ok(url)
}

pub fn validate_planetary_computer_reviewed_asset_identity(
    class: PlanetaryComputerReviewedAssetClass,
    provider_product_id: &str,
    asset_key: &str,
    stable_href: &str,
) -> ProviderResult<String> {
    if !safe_identity(provider_product_id) {
        return Err(asset_error("MPC_ASSET_REJECTED", "provider_product_id"));
    }
    let url = validate_planetary_computer_stable_asset(class, stable_href)?;
    let file_name = url
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .filter(|value| safe_identity(value))
        .ok_or_else(|| asset_error("MPC_ASSET_REJECTED", "file_name"))?;
    match class {
        PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog => {
            if !matches!(asset_key, "vv" | "vh" | "hh" | "hv")
                || !provider_product_id.starts_with("S1")
                || !provider_product_id.contains("_GRD")
                || file_name != format!("{}-{asset_key}.tiff", s1_measurement_prefix(&url)?)
            {
                return Err(asset_error("MPC_ASSET_REJECTED", "sentinel1_identity"));
            }
            let parent = s1_product_directory(&url)?;
            let suffix = parent
                .strip_prefix(provider_product_id)
                .ok_or_else(|| asset_error("MPC_ASSET_REJECTED", "sentinel1_product_path"))?;
            if !suffix.starts_with('_')
                || suffix.len() < 2
                || !suffix[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Err(asset_error("MPC_ASSET_REJECTED", "sentinel1_product_path"));
            }
        }
        PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog => {
            if asset_key != "data" || file_name != format!("{provider_product_id}.tif") {
                return Err(asset_error("MPC_ASSET_REJECTED", "copdem_identity"));
            }
        }
        PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog => {
            validate_sentinel2_l2a_band_identity(provider_product_id, asset_key, &url, file_name)?;
        }
    }
    Ok(file_name.to_owned())
}

pub fn validate_planetary_computer_runtime_asset_url(
    class: PlanetaryComputerReviewedAssetClass,
    url: &Url,
) -> ProviderResult<()> {
    if url.fragment().is_some() {
        return Err(asset_error("MPC_ASSET_REJECTED", "runtime_fragment"));
    }
    let mut stable = url.clone();
    stable.set_query(None);
    validate_planetary_computer_stable_asset(class, stable.as_str())?;
    validate_signed_query(url)
}

#[derive(Deserialize)]
struct SignResponse {
    href: String,
    #[serde(rename = "msft:expiry")]
    expiry: String,
}

fn validate_signed_response(
    class: PlanetaryComputerReviewedAssetClass,
    stable: &Url,
    response: SignResponse,
) -> ProviderResult<PlanetaryComputerRuntimeAsset> {
    if response.href.chars().count() > MAX_SIGNED_HREF_CHARS {
        return Err(invalid_sign_response("signed_href_size"));
    }
    let signed = Url::parse(&response.href).map_err(|_| invalid_sign_response("signed_href"))?;
    let mut unsigned = signed.clone();
    unsigned.set_query(None);
    if &unsigned != stable {
        return Err(invalid_sign_response("signed_identity"));
    }
    validate_planetary_computer_stable_asset(class, unsigned.as_str())?;
    validate_signed_query(&signed)?;
    let pairs = signed.query_pairs().collect::<Vec<_>>();
    let expiry_query = pairs
        .iter()
        .find(|(key, _)| key == "se")
        .map(|(_, value)| value.as_ref())
        .ok_or_else(|| invalid_sign_response("signed_expiry"))?;
    let expiry_text = parse_expiry(&response.expiry)?;
    let expiry_query = parse_expiry(expiry_query)?;
    if expiry_text != expiry_query {
        return Err(invalid_sign_response("signed_expiry"));
    }
    let now = SystemTime::now();
    if now
        .checked_add(Duration::from_secs(30))
        .is_none_or(|minimum| minimum >= expiry_text)
        || now
            .checked_add(Duration::from_secs(24 * 60 * 60))
            .is_some_and(|maximum| expiry_text > maximum)
    {
        return Err(invalid_sign_response("signed_expiry"));
    }
    Ok(PlanetaryComputerRuntimeAsset {
        url: signed,
        expires_at: expiry_text,
    })
}

fn validate_signed_query(url: &Url) -> ProviderResult<()> {
    let pairs = url.query_pairs().collect::<Vec<_>>();
    if pairs.is_empty() {
        return Err(invalid_sign_response("signed_query"));
    }
    let allowed = [
        "se", "sig", "ske", "skoid", "sks", "skt", "sktid", "skv", "sp", "sr", "st", "sv",
    ];
    let mut names = BTreeSet::new();
    for (key, value) in &pairs {
        if !allowed.contains(&key.as_ref()) || value.is_empty() || !names.insert(key.to_string()) {
            return Err(invalid_sign_response("signed_query"));
        }
    }
    for required in ["se", "sig", "sp", "sr", "sv"] {
        if !names.contains(required) {
            return Err(invalid_sign_response("signed_query"));
        }
    }
    let permissions = pairs
        .iter()
        .find(|(key, _)| key == "sp")
        .map(|(_, value)| value.as_ref())
        .ok_or_else(|| invalid_sign_response("signed_permissions"))?;
    let permission_set = permissions.chars().collect::<BTreeSet<_>>();
    if !permission_set.contains(&'r')
        || permission_set.len() != permissions.chars().count()
        || permission_set
            .iter()
            .any(|permission| !matches!(permission, 'r' | 'l'))
    {
        return Err(invalid_sign_response("signed_permissions"));
    }
    let resource_scope = pairs
        .iter()
        .find(|(key, _)| key == "sr")
        .map(|(_, value)| value.as_ref())
        .ok_or_else(|| invalid_sign_response("signed_resource_scope"))?;
    if resource_scope != "c" {
        return Err(invalid_sign_response("signed_resource_scope"));
    }
    Ok(())
}

fn parse_one_byte_content_range(value: &str) -> Option<u64> {
    value.strip_prefix("bytes 0-0/")?.parse::<u64>().ok()
}

pub fn sentinel2_l2a_reflectance_band_resolution(asset_key: &str) -> Option<u32> {
    match asset_key {
        "B02" | "B03" | "B04" | "B08" => Some(10),
        "B05" | "B06" | "B07" | "B8A" | "B11" | "B12" => Some(20),
        "B01" | "B09" => Some(60),
        _ => None,
    }
}

fn validate_sentinel2_l2a_band_identity(
    provider_product_id: &str,
    asset_key: &str,
    url: &Url,
    file_name: &str,
) -> ProviderResult<()> {
    let resolution = sentinel2_l2a_reflectance_band_resolution(asset_key)
        .ok_or_else(|| asset_error("MPC_ASSET_REJECTED", "sentinel2_band_key"))?;
    let segments = url
        .path_segments()
        .map(|segments| segments.collect::<Vec<_>>())
        .ok_or_else(|| asset_error("MPC_ASSET_REJECTED", "sentinel2_band_path"))?;
    let [root, utm_zone, latitude_band, square, year, month, day, safe_name, granule_root, granule_id, image_root, resolution_dir, actual_file_name] =
        segments.as_slice()
    else {
        return Err(asset_error("MPC_ASSET_REJECTED", "sentinel2_band_path"));
    };
    let safe_stem = safe_name
        .strip_suffix(".SAFE")
        .ok_or_else(|| asset_error("MPC_ASSET_REJECTED", "sentinel2_safe_name"))?;
    let parts = safe_stem.split('_').collect::<Vec<_>>();
    if parts.len() != 7
        || parts[1] != "MSIL2A"
        || parts[3].len() != 5
        || !parts[3].starts_with('N')
        || !parts[3][1..].bytes().all(|byte| byte.is_ascii_digit())
        || !parts[4].starts_with('R')
        || parts[4].len() != 4
        || !parts[4][1..].bytes().all(|byte| byte.is_ascii_digit())
        || !parts[5].starts_with('T')
        || parts[5].len() != 6
        || !parts[5][1..]
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric())
    {
        return Err(asset_error("MPC_ASSET_REJECTED", "sentinel2_safe_name"));
    }
    let expected_item_id = [parts[0], parts[1], parts[2], parts[4], parts[5], parts[6]].join("_");
    let acquisition_date = parts[2]
        .get(..8)
        .ok_or_else(|| asset_error("MPC_ASSET_REJECTED", "sentinel2_acquisition"))?;
    let tile = &parts[5][1..];
    let expected_file_name = format!("{}_{}_{}_{resolution}m.tif", parts[5], parts[2], asset_key);
    let expected_resolution_dir = format!("R{resolution}m");
    if *root != "sentinel2-l2"
        || provider_product_id != expected_item_id
        || *utm_zone != &tile[..2]
        || *latitude_band != &tile[2..3]
        || *square != &tile[3..]
        || *year != &acquisition_date[..4]
        || *month != &acquisition_date[4..6]
        || *day != &acquisition_date[6..8]
        || *granule_root != "GRANULE"
        || granule_id.is_empty()
        || *image_root != "IMG_DATA"
        || *resolution_dir != expected_resolution_dir
        || *actual_file_name != expected_file_name
        || file_name != expected_file_name
    {
        return Err(asset_error("MPC_ASSET_REJECTED", "sentinel2_band_identity"));
    }
    Ok(())
}

fn s1_measurement_prefix(url: &Url) -> ProviderResult<&str> {
    let file = url
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .ok_or_else(|| asset_error("MPC_ASSET_REJECTED", "sentinel1_measurement"))?;
    file.split_once('-')
        .map(|(prefix, _)| prefix)
        .filter(|prefix| matches!(*prefix, "iw" | "ew" | "sm"))
        .ok_or_else(|| asset_error("MPC_ASSET_REJECTED", "sentinel1_measurement"))
}

fn s1_product_directory(url: &Url) -> ProviderResult<&str> {
    let segments = url
        .path_segments()
        .map(|segments| segments.collect::<Vec<_>>())
        .ok_or_else(|| asset_error("MPC_ASSET_REJECTED", "sentinel1_product_path"))?;
    let measurement_index = segments
        .iter()
        .position(|value| *value == "measurement")
        .ok_or_else(|| asset_error("MPC_ASSET_REJECTED", "sentinel1_product_path"))?;
    measurement_index
        .checked_sub(1)
        .and_then(|index| segments.get(index).copied())
        .filter(|value| safe_identity(value))
        .ok_or_else(|| asset_error("MPC_ASSET_REJECTED", "sentinel1_product_path"))
}

fn safe_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value == value.trim()
        && !matches!(value, "." | "..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}
fn parse_expiry(value: &str) -> ProviderResult<SystemTime> {
    let parsed = DateTime::parse_from_rfc3339(value)
        .map_err(|_| invalid_sign_response("expiry"))?
        .with_timezone(&Utc);
    if parsed.timestamp() < 0 {
        return Err(invalid_sign_response("expiry"));
    }
    UNIX_EPOCH
        .checked_add(Duration::from_secs(parsed.timestamp() as u64))
        .and_then(|base| {
            base.checked_add(Duration::from_nanos(parsed.timestamp_subsec_nanos() as u64))
        })
        .ok_or_else(|| invalid_sign_response("expiry"))
}

fn retryable_sign_status(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::REQUEST_TIMEOUT
            | StatusCode::TOO_MANY_REQUESTS
            | StatusCode::INTERNAL_SERVER_ERROR
            | StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT
    )
}

fn sign_status_error(
    status: StatusCode,
    retry_after: Option<&HeaderValue>,
    attempts: usize,
) -> ContractError {
    let reason = if retryable_sign_status(status) {
        "http_status_transient"
    } else {
        "http_status_terminal"
    };
    let mut details = vec![("http_status", status.as_u16().to_string())];
    let code = match status.as_u16() {
        429 => {
            if let Some(value) = retry_after {
                details.push(retry_after_detail(value.to_str().unwrap_or_default()));
            }
            "MPC_ASSET_SIGN_RATE_LIMITED"
        }
        _ if retryable_sign_status(status) => "MPC_ASSET_SIGN_UNAVAILABLE",
        400..=499 => "MPC_ASSET_SIGN_REJECTED",
        _ => "MPC_ASSET_SIGN_RESPONSE_INVALID",
    };
    details.push(("attempts", attempts.to_string()));
    sign_failure(code, reason, &details)
}

fn sign_transport_error(error: &reqwest::Error, attempts: usize) -> ContractError {
    let transport = if error.is_timeout() {
        "timeout"
    } else if error.is_connect() {
        "connect"
    } else {
        "other"
    };
    sign_failure(
        "MPC_ASSET_SIGN_TRANSPORT",
        "transport",
        &[
            ("transport", transport.to_owned()),
            ("attempts", attempts.to_string()),
        ],
    )
}

/// Keeps only a bounded summary; the raw header value never leaves the provider.
fn retry_after_detail(value: &str) -> (&'static str, String) {
    let value = value.trim();
    if !value.is_empty() && value.len() <= 64 {
        if value.bytes().all(|byte| byte.is_ascii_digit()) {
            let seconds = value
                .parse::<u64>()
                .unwrap_or(u64::MAX)
                .min(MAX_REPORTED_RETRY_AFTER_SECONDS);
            return ("retry_after_seconds", seconds.to_string());
        }
        if DateTime::parse_from_rfc2822(value).is_ok() {
            return ("retry_after", "http_date".to_owned());
        }
    }
    ("retry_after", "invalid".to_owned())
}

fn sign_failure(code: &str, reason: &str, details: &[(&str, String)]) -> ContractError {
    let mut context = Map::from_iter([("reason".to_owned(), Value::String(reason.to_owned()))]);
    for (key, value) in details {
        context.insert((*key).to_owned(), Value::String(value.clone()));
    }
    ContractError::new(code, "Planetary Computer 资产签名失败", context)
        .expect("bounded MPC asset error is safe")
}

fn sign_retry_delay(attempt: usize) -> Duration {
    match attempt {
        0 => Duration::from_millis(250),
        1 => Duration::from_millis(750),
        _ => Duration::from_millis(1_500),
    }
}

fn sign_retry_delay_for_response(
    attempt: usize,
    status: StatusCode,
    retry_after: Option<&HeaderValue>,
) -> Duration {
    if status == StatusCode::TOO_MANY_REQUESTS {
        if let Some(value) = retry_after.and_then(|value| value.to_str().ok()) {
            let value = value.trim();
            if value.len() <= 64 {
                let seconds = value.parse::<u64>().ok().or_else(|| {
                    DateTime::parse_from_rfc2822(value).ok().map(|date| {
                        let remaining = date
                            .timestamp()
                            .saturating_sub(chrono::Utc::now().timestamp());
                        u64::try_from(remaining).unwrap_or(0)
                    })
                });
                if let Some(seconds) = seconds {
                    return Duration::from_secs(seconds)
                        .min(MAX_SIGN_RETRY_DELAY)
                        .max(sign_retry_delay(attempt));
                }
            }
        }
    }
    sign_retry_delay(attempt)
}

fn wait_sign_retry(attempt: usize, cancelled: &(dyn Fn() -> bool + Sync)) -> ProviderResult<()> {
    wait_sign_retry_delay(sign_retry_delay(attempt), cancelled)
}

fn wait_sign_retry_delay(
    delay: Duration,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> ProviderResult<()> {
    let mut waited = Duration::ZERO;
    while waited < delay {
        check_cancelled(cancelled)?;
        let remaining = delay.saturating_sub(waited);
        let step = remaining.min(SIGN_RETRY_POLL);
        std::thread::sleep(step);
        waited += step;
    }
    check_cancelled(cancelled)
}

fn check_cancelled(cancelled: &(dyn Fn() -> bool + Sync)) -> ProviderResult<()> {
    if cancelled() {
        Err(asset_error("MPC_ASSET_CANCELLED", "cancelled"))
    } else {
        Ok(())
    }
}

fn asset_error(code: &str, reason: &str) -> ContractError {
    ContractError::new(
        code,
        "Planetary Computer 资产签名失败",
        Map::from_iter([("reason".to_owned(), Value::String(reason.to_owned()))]),
    )
    .expect("bounded MPC asset error is safe")
}

fn invalid_sign_response(reason: &str) -> ContractError {
    asset_error("MPC_ASSET_SIGN_RESPONSE_INVALID", reason)
}

#[cfg(test)]
mod tests {
    use std::io::{ErrorKind, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::thread;

    use chrono::{Duration as ChronoDuration, SecondsFormat};

    use super::*;

    const SIGN_RATE_LIMITED: &str = "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 3\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
    const SIGN_FORBIDDEN: &str =
        "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
    const SIGN_UNAVAILABLE: &str =
        "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
    const SIGN_NOT_JSON: &str = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 8\r\nConnection: close\r\n\r\nnot json";

    /// Serves one scripted reply per connection; `None` closes the connection
    /// unanswered. Returns how many sign requests arrived, including extra ones.
    fn scripted_sign_server(
        script: Vec<Option<&'static str>>,
    ) -> (Url, Arc<AtomicBool>, thread::JoinHandle<usize>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback listener");
        listener
            .set_nonblocking(true)
            .expect("non-blocking listener");
        let origin = Url::parse(&format!(
            "http://{}",
            listener.local_addr().expect("listener address")
        ))
        .expect("loopback origin");
        let done = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&done);
        let worker = thread::spawn(move || {
            let mut served = 0_usize;
            while !stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_nonblocking(false).expect("blocking stream");
                        stream
                            .set_read_timeout(Some(Duration::from_secs(5)))
                            .expect("read timeout");
                        let mut request = Vec::new();
                        let mut chunk = [0_u8; 1024];
                        while !request.windows(4).any(|window| window == b"\r\n\r\n")
                            && request.len() < 16 * 1024
                        {
                            match stream.read(&mut chunk) {
                                Ok(0) | Err(_) => break,
                                Ok(read) => request.extend_from_slice(&chunk[..read]),
                            }
                        }
                        if let Some(Some(reply)) = script.get(served) {
                            let _ = stream.write_all(reply.as_bytes());
                            let _ = stream.flush();
                        }
                        served += 1;
                    }
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error)
                        if matches!(
                            error.kind(),
                            ErrorKind::ConnectionReset
                                | ErrorKind::ConnectionAborted
                                | ErrorKind::Interrupted
                        ) => {}
                    Err(_) => break,
                }
            }
            served
        });
        (origin, done, worker)
    }

    fn scripted_sign_failure(script: Vec<Option<&'static str>>) -> (ContractError, usize) {
        let (origin, done, worker) = scripted_sign_server(script);
        let signer =
            PlanetaryComputerAssetSigner::with_origin(ProviderNetworkPolicy::direct(), origin)
                .expect("loopback signer");
        let class = PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog;
        let error = signer
            .sign_reviewed_asset(class, stable(class), &|| false)
            .expect_err("scripted sign failure");
        done.store(true, Ordering::SeqCst);
        (error, worker.join().expect("sign server"))
    }

    fn detail(error: &ContractError, key: &str) -> Option<String> {
        error
            .debug_context
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_owned)
    }

    fn stable(class: PlanetaryComputerReviewedAssetClass) -> &'static str {
        match class {
            PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog => {
                "https://sentinel1euwest.blob.core.windows.net/s1-grd/GRD/2025/1/23/IW/DV/S1A_TEST/measurement/iw-vv.tiff"
            }
            PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog => {
                "https://elevationeuwest.blob.core.windows.net/copernicus-dem/COP30_hh/Copernicus_DSM_COG_10_N25_00_E121_00_DEM.tif"
            }
            PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog => {
                "https://sentinel2l2a01.blob.core.windows.net/sentinel2-l2/48/S/YC/2026/07/06/S2C_MSIL2A_20260706T032511_N0512_R018_T48SYC_20260706T065916.SAFE/GRANULE/L2A_T48SYC_A009569_20260706T033711/IMG_DATA/R10m/T48SYC_20260706T032511_B02_10m.tif"
            }
        }
    }

    #[test]
    fn reviewed_stable_asset_classes_are_exact_and_query_free() {
        for class in [
            PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog,
            PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog,
            PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog,
        ] {
            assert!(validate_planetary_computer_stable_asset(class, stable(class)).is_ok());
            assert!(validate_planetary_computer_stable_asset(
                class,
                &format!("{}?sig=forged", stable(class))
            )
            .is_err());
        }
        assert!(validate_planetary_computer_stable_asset(
            PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog,
            stable(PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog)
        )
        .is_err());
        assert!(validate_planetary_computer_stable_asset(
            PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog,
            "https://sentinel1euwest.blob.core.windows.net/s1-grd/OTHER/file.tiff"
        )
        .is_err());
    }

    #[test]
    fn sentinel2_band_identity_binds_item_safe_tile_band_and_resolution() {
        let class = PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog;
        let href = stable(class);
        assert_eq!(
            validate_planetary_computer_reviewed_asset_identity(
                class,
                "S2C_MSIL2A_20260706T032511_R018_T48SYC_20260706T065916",
                "B02",
                href,
            )
            .unwrap(),
            "T48SYC_20260706T032511_B02_10m.tif"
        );
        assert_eq!(
            validate_planetary_computer_reviewed_asset_identity(
                class,
                "S2C_MSIL2A_20260706T032511_R018_T48SYC_20260706T065916",
                "B11",
                "https://sentinel2l2a01.blob.core.windows.net/sentinel2-l2/48/S/YC/2026/07/06/S2C_MSIL2A_20260706T032511_N0512_R018_T48SYC_20260706T065916.SAFE/GRANULE/L2A_T48SYC_A009569_20260706T033711/IMG_DATA/R20m/T48SYC_20260706T032511_B11_20m.tif",
            )
            .unwrap(),
            "T48SYC_20260706T032511_B11_20m.tif"
        );
        assert!(validate_planetary_computer_reviewed_asset_identity(
            class,
            "S2C_MSIL2A_20260706T032511_R018_T48SYC_20260706T065916",
            "B11",
            href,
        )
        .is_err());
        assert!(validate_planetary_computer_reviewed_asset_identity(
            class,
            "S2C_MSIL2A_20260706T032511_R018_T48SYD_20260706T065916",
            "B02",
            href,
        )
        .is_err());
        assert!(validate_planetary_computer_reviewed_asset_identity(
            class,
            "S2C_MSIL2A_20260706T032511_R018_T48SYC_20260706T065916",
            "SCL",
            href,
        )
        .is_err());
    }

    #[test]
    fn sign_http_failure_reason_keeps_transient_and_terminal_statuses_distinct() {
        for status in [
            StatusCode::REQUEST_TIMEOUT,
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::BAD_GATEWAY,
            StatusCode::SERVICE_UNAVAILABLE,
            StatusCode::GATEWAY_TIMEOUT,
        ] {
            assert!(retryable_sign_status(status));
        }
        for status in [
            StatusCode::BAD_REQUEST,
            StatusCode::UNAUTHORIZED,
            StatusCode::FORBIDDEN,
            StatusCode::NOT_FOUND,
        ] {
            assert!(!retryable_sign_status(status));
        }
    }

    #[test]
    fn sign_retry_policy_is_limited_to_transient_transport_statuses() {
        assert_eq!(SIGN_REQUEST_ATTEMPTS, 4);
        assert_eq!(SIGN_TRANSPORT_ATTEMPTS, 2);
        assert_eq!(sign_retry_delay(0), Duration::from_millis(250));
        assert_eq!(sign_retry_delay(1), Duration::from_millis(750));
        assert_eq!(sign_retry_delay(2), Duration::from_millis(1_500));
        assert_eq!(sign_retry_delay(3), Duration::from_millis(1_500));
        for status in [
            StatusCode::REQUEST_TIMEOUT,
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::BAD_GATEWAY,
            StatusCode::SERVICE_UNAVAILABLE,
            StatusCode::GATEWAY_TIMEOUT,
        ] {
            assert!(retryable_sign_status(status), "{status}");
        }
        for status in [
            StatusCode::OK,
            StatusCode::BAD_REQUEST,
            StatusCode::UNAUTHORIZED,
            StatusCode::FORBIDDEN,
            StatusCode::NOT_FOUND,
            StatusCode::UNPROCESSABLE_ENTITY,
        ] {
            assert!(!retryable_sign_status(status), "{status}");
        }
    }

    #[test]
    fn rate_limited_sign_keeps_status_attempts_and_retry_after() {
        let (error, served) = scripted_sign_failure(vec![Some(SIGN_RATE_LIMITED); 4]);
        assert_eq!(error.code, "MPC_ASSET_SIGN_RATE_LIMITED");
        assert_eq!(
            detail(&error, "reason").as_deref(),
            Some("http_status_transient")
        );
        assert_eq!(detail(&error, "http_status").as_deref(), Some("429"));
        assert_eq!(detail(&error, "retry_after_seconds").as_deref(), Some("3"));
        assert_eq!(detail(&error, "attempts").as_deref(), Some("4"));
        assert_eq!(
            served, SIGN_REQUEST_ATTEMPTS,
            "the request budget is unchanged"
        );
    }

    #[test]
    fn rate_limited_sign_respects_bounded_retry_after_seconds_instead_of_fixed_backoff() {
        let retry_delay = |value: &str| {
            sign_retry_delay_for_response(
                0,
                StatusCode::TOO_MANY_REQUESTS,
                Some(&HeaderValue::from_str(value).expect("fixture Retry-After header")),
            )
        };
        assert_eq!(retry_delay("3"), Duration::from_secs(3));
        assert_eq!(
            retry_delay("3601"),
            Duration::from_secs(60),
            "wait must be capped"
        );
        assert_eq!(retry_delay("not-a-date"), sign_retry_delay(0));
        assert_eq!(
            sign_retry_delay_for_response(
                0,
                StatusCode::SERVICE_UNAVAILABLE,
                Some(&HeaderValue::from_static("50")),
            ),
            sign_retry_delay(0),
            "non-429 transient failures retain their existing bounded delay"
        );
    }

    #[test]
    fn rejected_sign_is_reported_after_one_request_without_retry() {
        let (error, served) = scripted_sign_failure(vec![Some(SIGN_FORBIDDEN); 2]);
        assert_eq!(error.code, "MPC_ASSET_SIGN_REJECTED");
        assert_eq!(
            detail(&error, "reason").as_deref(),
            Some("http_status_terminal")
        );
        assert_eq!(detail(&error, "http_status").as_deref(), Some("403"));
        assert_eq!(detail(&error, "attempts").as_deref(), Some("1"));
        assert_eq!(served, 1, "a refused signature must not be retried");
    }

    #[test]
    fn unavailable_sign_service_is_not_reported_as_rate_limiting() {
        let (error, served) = scripted_sign_failure(vec![Some(SIGN_UNAVAILABLE); 4]);
        assert_eq!(error.code, "MPC_ASSET_SIGN_UNAVAILABLE");
        assert_eq!(
            detail(&error, "reason").as_deref(),
            Some("http_status_transient")
        );
        assert_eq!(detail(&error, "http_status").as_deref(), Some("503"));
        assert_eq!(detail(&error, "attempts").as_deref(), Some("4"));
        assert_eq!(detail(&error, "retry_after_seconds"), None);
        assert_eq!(served, SIGN_REQUEST_ATTEMPTS);
    }

    #[test]
    fn dropped_sign_connections_are_reported_as_transport_failures() {
        let (error, served) = scripted_sign_failure(vec![None, None]);
        assert_eq!(error.code, "MPC_ASSET_SIGN_TRANSPORT");
        assert_eq!(detail(&error, "reason").as_deref(), Some("transport"));
        assert_eq!(detail(&error, "transport").as_deref(), Some("other"));
        assert_eq!(detail(&error, "attempts").as_deref(), Some("2"));
        assert_eq!(served, SIGN_TRANSPORT_ATTEMPTS);
    }

    #[test]
    fn malformed_sign_response_is_reported_as_an_invalid_response() {
        let (error, served) = scripted_sign_failure(vec![Some(SIGN_NOT_JSON)]);
        assert_eq!(error.code, "MPC_ASSET_SIGN_RESPONSE_INVALID");
        assert_eq!(detail(&error, "reason").as_deref(), Some("response_json"));
        assert_eq!(served, 1);
    }

    #[test]
    fn every_retried_sign_status_is_reported_as_a_retryable_class() {
        for code in 100_u16..=599 {
            let status = StatusCode::from_u16(code).expect("valid status code");
            let error = sign_status_error(status, None, 1);
            let reason = detail(&error, "reason");
            if retryable_sign_status(status) {
                assert!(
                    matches!(
                        error.code.as_str(),
                        "MPC_ASSET_SIGN_RATE_LIMITED" | "MPC_ASSET_SIGN_UNAVAILABLE"
                    ),
                    "{code}: {}",
                    error.code
                );
                assert_eq!(reason.as_deref(), Some("http_status_transient"), "{code}");
            } else {
                assert_ne!(error.code, "MPC_ASSET_SIGN_UNAVAILABLE", "{code}");
                assert_eq!(reason.as_deref(), Some("http_status_terminal"), "{code}");
            }
        }
    }

    #[test]
    fn retry_after_is_reduced_to_bounded_seconds_or_a_marker() {
        let seconds = |value: &str| ("retry_after_seconds", value.to_owned());
        assert_eq!(retry_after_detail("3"), seconds("3"));
        assert_eq!(retry_after_detail(" 120 "), seconds("120"));
        assert_eq!(retry_after_detail("86400"), seconds("3600"));
        assert_eq!(
            retry_after_detail("99999999999999999999999"),
            seconds("3600")
        );
        assert_eq!(
            retry_after_detail("Wed, 21 Oct 2026 07:28:00 GMT"),
            ("retry_after", "http_date".to_owned())
        );
        for value in ["", "soon", "-1", "3.5", "https://example.invalid"] {
            assert_eq!(
                retry_after_detail(value),
                ("retry_after", "invalid".to_owned()),
                "{value}"
            );
        }
    }

    #[test]
    fn validated_collection_sas_reuses_only_matching_collection_with_expiry_margin() {
        let mut cache = CollectionSasCache::default();
        let class = PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog;
        let stable = validate_planetary_computer_stable_asset(class, stable(class)).unwrap();
        let expiry = SystemTime::now() + Duration::from_secs(300);
        let mut signed = stable.clone();
        signed
            .query_pairs_mut()
            .append_pair(
                "se",
                &(Utc::now() + ChronoDuration::seconds(300))
                    .to_rfc3339_opts(SecondsFormat::Secs, true),
            )
            .append_pair("sp", "r")
            .append_pair("sr", "c")
            .append_pair("sv", "2025-07-05")
            .append_pair("sig", "fixture-signature");
        cache.insert(
            class,
            &stable,
            PlanetaryComputerRuntimeAsset {
                url: signed,
                expires_at: expiry,
            },
        );
        let other = validate_planetary_computer_stable_asset(
            class,
            "https://elevationeuwest.blob.core.windows.net/copernicus-dem/COP30_hh/another.tif",
        )
        .unwrap();
        let reused = cache
            .lookup(class, &other)
            .expect("same collection shares its signed collection SAS");
        assert_eq!(reused.url.path(), other.path());
        assert!(reused.url.query().is_some());
        assert!(cache
            .lookup(PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog, &other)
            .is_none());
        cache.entries[0].expires_at = SystemTime::now() + Duration::from_secs(10);
        assert!(
            cache.lookup(class, &other).is_none(),
            "expiring SAS must be renewed"
        );
        assert!(cache.entries.len() <= 3);
    }

    #[test]
    fn signed_response_must_preserve_stable_identity_and_redacts_debug() {
        let class = PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog;
        let stable = validate_planetary_computer_stable_asset(class, stable(class)).unwrap();
        let expiry =
            (Utc::now() + ChronoDuration::hours(1)).to_rfc3339_opts(SecondsFormat::Secs, true);
        let mut signed = stable.clone();
        signed
            .query_pairs_mut()
            .append_pair("st", "2026-09-19T00:00:00Z")
            .append_pair("se", &expiry)
            .append_pair("sp", "rl")
            .append_pair("sv", "2025-07-05")
            .append_pair("sr", "c")
            .append_pair("skoid", "00000000-0000-0000-0000-000000000000")
            .append_pair("sktid", "00000000-0000-0000-0000-000000000000")
            .append_pair("skt", "2026-09-19T00:00:00Z")
            .append_pair("ske", &expiry)
            .append_pair("sks", "b")
            .append_pair("skv", "2025-07-05")
            .append_pair("sig", "TOP-SECRET-SIGNATURE");
        let runtime = validate_signed_response(
            class,
            &stable,
            SignResponse {
                href: signed.to_string(),
                expiry,
            },
        )
        .unwrap();
        let debug = format!("{runtime:?}");
        assert!(debug.contains("[REDACTED_SIGNED_URL]"));
        assert!(!debug.contains("TOP-SECRET-SIGNATURE"));
        let (url, _) = runtime.into_parts();
        let mut unsigned = url;
        unsigned.set_query(None);
        assert_eq!(unsigned, stable);
    }

    #[test]
    fn signed_response_rejects_unknown_query_or_identity_drift() {
        let class = PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog;
        let stable = validate_planetary_computer_stable_asset(class, stable(class)).unwrap();
        let expiry =
            (Utc::now() + ChronoDuration::hours(1)).to_rfc3339_opts(SecondsFormat::Secs, true);
        let mut signed = stable.clone();
        signed
            .query_pairs_mut()
            .append_pair("se", &expiry)
            .append_pair("sp", "r")
            .append_pair("sv", "2025-07-05")
            .append_pair("sr", "b")
            .append_pair("sig", "fixture")
            .append_pair("unknown", "value");
        assert!(validate_signed_response(
            class,
            &stable,
            SignResponse {
                href: signed.to_string(),
                expiry: expiry.clone(),
            }
        )
        .is_err());

        let mut over_privileged = stable.clone();
        over_privileged
            .query_pairs_mut()
            .append_pair("se", &expiry)
            .append_pair("sp", "rw")
            .append_pair("sv", "2025-07-05")
            .append_pair("sr", "c")
            .append_pair("sig", "fixture");
        assert!(validate_signed_response(
            class,
            &stable,
            SignResponse {
                href: over_privileged.to_string(),
                expiry: expiry.clone(),
            }
        )
        .is_err());

        let mut wrong_scope = stable.clone();
        wrong_scope
            .query_pairs_mut()
            .append_pair("se", &expiry)
            .append_pair("sp", "r")
            .append_pair("sv", "2025-07-05")
            .append_pair("sr", "b")
            .append_pair("sig", "fixture");
        assert!(validate_signed_response(
            class,
            &stable,
            SignResponse {
                href: wrong_scope.to_string(),
                expiry: expiry.clone(),
            }
        )
        .is_err());

        let wrong = Url::parse(
            "https://sentinel1euwest.blob.core.windows.net/s1-grd/GRD/2025/1/23/IW/DV/OTHER/measurement/iw-vv.tiff?se=2026-09-19T23%3A59%3A59Z&sp=r&sv=1&sr=b&sig=x",
        )
        .unwrap();
        assert!(validate_signed_response(
            class,
            &stable,
            SignResponse {
                href: wrong.to_string(),
                expiry,
            }
        )
        .is_err());
    }
}
