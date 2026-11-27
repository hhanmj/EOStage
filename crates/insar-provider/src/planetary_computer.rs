use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt;
use std::io::Read;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use crate::catalog::{
    CollectionProfileRegistry, EARTH_SEARCH_LANDSAT_COLLECTION_ID,
    EARTH_SEARCH_LANDSAT_PROVIDER_ID, LANDSAT_C2_L2_NATIVE_COLLECTION_ID,
    LANDSAT_C2_L2_PROVIDER_ID, MPC_LANDSAT_COLLECTION_ID, MPC_LANDSAT_PROVIDER_ID,
    MPC_LANDSAT_SCENE_PROFILE_ID,
};
use chrono::{DateTime, NaiveDate, SecondsFormat, Utc};
use insar_core::{
    AssetRole, CatalogCollectionDescriptor, CatalogDeliveryModel, CatalogSearchQuery,
    CatalogSearchResponse, CatalogSearchStats, ContractError, DataSourceKind, ProductKind,
    ProviderNetworkPolicy, ProviderProduct, ProviderResult,
};
use reqwest::blocking::{Client, Response};
use reqwest::header::{CONTENT_RANGE, CONTENT_TYPE, RANGE};
use reqwest::redirect::Policy;
use reqwest::{StatusCode, Url};
use serde::Deserialize;
use serde_json::{json, Map, Number, Value};
use zeroize::Zeroizing;

pub const PLANETARY_COMPUTER_LANDSAT_COLLECTION_ID: &str = "landsat-c2-l2";
pub const PLANETARY_COMPUTER_STAC_ORIGIN: &str = "https://planetarycomputer.microsoft.com";
pub const PLANETARY_COMPUTER_LANDSAT_ASSET_ORIGIN: &str =
    "https://landsateuwest.blob.core.windows.net";
pub const PLANETARY_COMPUTER_LANDSAT_ASSET_PREFIX: &str = "/landsat-c2/level-2/standard/oli-tirs/";
pub const PLANETARY_COMPUTER_LANDSAT_TM_ASSET_PREFIX: &str = "/landsat-c2/level-2/standard/tm/";
pub const PLANETARY_COMPUTER_LANDSAT_ETM_ASSET_PREFIX: &str = "/landsat-c2/level-2/standard/etm/";

pub fn is_valid_planetary_computer_landsat_path(path: &str) -> bool {
    path.starts_with(PLANETARY_COMPUTER_LANDSAT_ASSET_PREFIX)
        || path.starts_with(PLANETARY_COMPUTER_LANDSAT_TM_ASSET_PREFIX)
        || path.starts_with(PLANETARY_COMPUTER_LANDSAT_ETM_ASSET_PREFIX)
}

const STAC_ITEM_PREFIX: &str = "/api/stac/v1/collections/landsat-c2-l2/items/";
const STAC_SEARCH_PATH: &str = "/api/stac/v1/search";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_JSON_BYTES: u64 = 2 * 1024 * 1024;
/// STAC `/search` payload budget. Large compact-envelope Landsat pages with
/// `MAX_SEARCH_FEATURES = 500` items carry dense multi-band asset metadata and
/// routinely exceed 4 MiB on national/coastal queries.
const MAX_SEARCH_JSON_BYTES: u64 = 24 * 1024 * 1024;
const MAX_TOKEN_BYTES: u64 = 16 * 1024;
const MAX_ASSET_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 16 * 1024 * 1024 * 1024;
const MAX_SEARCH_FEATURES: usize = 500;
/// Explicit page budget for a compact-envelope public Landsat search: up to
/// 40 × 500 raw records (matching the 10,000-item catalog safety horizon) may
/// be inspected to fill the target with exact AOI hits on national/coastal AOIs.
const MAX_PUBLIC_ENVELOPE_SEARCH_PAGES: usize = 40;
/// One deadline for every page of a compact-envelope public search.
const PUBLIC_ENVELOPE_SEARCH_DEADLINE: Duration = Duration::from_secs(120);
const MAX_BROWSE_BYTES: usize = 16 * 1024 * 1024;
const TOKEN_REFRESH_MARGIN: Duration = Duration::from_secs(120);
const SAS_TOKEN_REQUEST_ATTEMPTS: usize = 4;
const SAS_TOKEN_RETRY_POLL: Duration = Duration::from_millis(25);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanetaryComputerEndpointClass {
    Configuration,
    StacSearch,
    StacItem,
    SasToken,
    AssetHead,
    Identity,
}

impl PlanetaryComputerEndpointClass {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Configuration => "configuration",
            Self::StacSearch => "stac_search",
            Self::StacItem => "stac_item",
            Self::SasToken => "sas_token",
            Self::AssetHead => "asset_head",
            Self::Identity => "identity",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanetaryComputerErrorCode {
    Cancelled,
    ConfigInvalid,
    TransportFailed,
    HttpRejected,
    ResponseTooLarge,
    ResponseInvalid,
    IdentityMismatch,
    AssetSetInvalid,
    AssetUnavailable,
    SizeMissing,
}

impl PlanetaryComputerErrorCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Cancelled => "PLANETARY_COMPUTER_CANCELLED",
            Self::ConfigInvalid => "PLANETARY_COMPUTER_CONFIG_INVALID",
            Self::TransportFailed => "PLANETARY_COMPUTER_TRANSPORT_FAILED",
            Self::HttpRejected => "PLANETARY_COMPUTER_HTTP_REJECTED",
            Self::ResponseTooLarge => "PLANETARY_COMPUTER_RESPONSE_TOO_LARGE",
            Self::ResponseInvalid => "PLANETARY_COMPUTER_RESPONSE_INVALID",
            Self::IdentityMismatch => "PLANETARY_COMPUTER_IDENTITY_MISMATCH",
            Self::AssetSetInvalid => "PLANETARY_COMPUTER_ASSET_SET_INVALID",
            Self::AssetUnavailable => "PLANETARY_COMPUTER_ASSET_UNAVAILABLE",
            Self::SizeMissing => "PLANETARY_COMPUTER_SIZE_MISSING",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlanetaryComputerError {
    code: PlanetaryComputerErrorCode,
    endpoint: PlanetaryComputerEndpointClass,
    http_status: Option<u16>,
}

impl PlanetaryComputerError {
    fn new(code: PlanetaryComputerErrorCode, endpoint: PlanetaryComputerEndpointClass) -> Self {
        Self {
            code,
            endpoint,
            http_status: None,
        }
    }

    fn with_status(
        code: PlanetaryComputerErrorCode,
        endpoint: PlanetaryComputerEndpointClass,
        status: StatusCode,
    ) -> Self {
        Self {
            code,
            endpoint,
            http_status: Some(status.as_u16()),
        }
    }

    pub const fn code(self) -> PlanetaryComputerErrorCode {
        self.code
    }

    pub const fn code_str(self) -> &'static str {
        self.code.as_str()
    }

    pub const fn endpoint(self) -> PlanetaryComputerEndpointClass {
        self.endpoint
    }

    pub const fn http_status(self) -> Option<u16> {
        self.http_status
    }
}

impl fmt::Display for PlanetaryComputerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} at {}",
            self.code.as_str(),
            self.endpoint.as_str()
        )
    }
}

impl std::error::Error for PlanetaryComputerError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanetaryComputerLandsatAsset {
    pub asset_key: String,
    pub file_name: String,
    pub role: AssetRole,
    pub stable_href: String,
    pub expected_size_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanetaryComputerLandsatManifest {
    pub entity_id: String,
    pub display_id: String,
    pub stac_item_id: String,
    pub processing_level: String,
    pub assets: Vec<PlanetaryComputerLandsatAsset>,
    pub total_size_bytes: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlanetaryComputerRequestCounts {
    pub stac_item: u64,
    pub sas_token: u64,
    pub asset_head: u64,
}

pub struct PlanetaryComputerSignedAsset {
    url: Url,
    expires_at: SystemTime,
}

impl PlanetaryComputerSignedAsset {
    pub fn new(url: Url, expires_at: SystemTime) -> Result<Self, PlanetaryComputerError> {
        if url.query().is_none() || url.fragment().is_some() {
            return Err(identity_error());
        }
        let mut stable = url.clone();
        stable.set_query(None);
        if validate_asset_origin_and_prefix(&stable).is_err()
            && validate_hls_asset_origin_and_path(&stable).is_err()
        {
            return Err(identity_error());
        }
        Ok(Self { url, expires_at })
    }

    pub fn into_parts(self) -> (Url, SystemTime) {
        (self.url, self.expires_at)
    }
}

impl fmt::Debug for PlanetaryComputerSignedAsset {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PlanetaryComputerSignedAsset")
            .field("url", &"[REDACTED_SIGNED_URL]")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

struct CachedSasToken {
    token: Zeroizing<String>,
    expires_at: SystemTime,
}

pub struct SasLease {
    pub token: Zeroizing<String>,
    pub expires_at: SystemTime,
}

pub struct PlanetaryComputerLandsatClient {
    client: Client,
    network_policy: ProviderNetworkPolicy,
    stac_origin: Url,
    asset_origin: Url,
    sas_cache: Mutex<HashMap<String, CachedSasToken>>,
    stac_item_requests: AtomicU64,
    sas_token_requests: AtomicU64,
    asset_head_requests: AtomicU64,
}

/// One page request of a public Landsat search.
enum PublicSearchRequest {
    Post { url: Url, body: Value },
    Get { url: Url },
}

impl PublicSearchRequest {
    fn fingerprint(&self) -> String {
        match self {
            Self::Post { url, body } => format!("POST {url} {body}"),
            Self::Get { url } => format!("GET {url}"),
        }
    }
}

impl PlanetaryComputerLandsatClient {
    pub fn new(network_policy: ProviderNetworkPolicy) -> Result<Self, PlanetaryComputerError> {
        Self::with_origins(
            network_policy,
            Url::parse(PLANETARY_COMPUTER_STAC_ORIGIN).map_err(|_| config_error())?,
            Url::parse(PLANETARY_COMPUTER_LANDSAT_ASSET_ORIGIN).map_err(|_| config_error())?,
            DEFAULT_TIMEOUT,
        )
    }

    /// Loopback-only test constructor for deterministic offline SAS/STAC tests.
    /// Rejects any non-loopback host, HTTPS scheme, path, query, fragment, or userinfo,
    /// while preserving `PLANETARY_COMPUTER_LANDSAT_ASSET_ORIGIN` pinning.
    #[doc(hidden)]
    pub fn with_loopback_stac_origin(
        stac_origin: &str,
        timeout: Duration,
    ) -> Result<Self, PlanetaryComputerError> {
        let parsed = Url::parse(stac_origin).map_err(|_| config_error())?;
        if timeout.is_zero()
            || timeout > Duration::from_secs(120)
            || parsed.scheme() != "http"
            || !parsed
                .host_str()
                .and_then(|host| host.parse::<std::net::IpAddr>().ok())
                .is_some_and(|ip| ip.is_loopback())
            || parsed.port().is_none_or(|port| port == 0)
            || parsed.path() != "/"
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
        {
            return Err(config_error());
        }
        let asset_origin =
            Url::parse(PLANETARY_COMPUTER_LANDSAT_ASSET_ORIGIN).map_err(|_| config_error())?;
        let client = Client::builder()
            .connect_timeout(timeout.min(Duration::from_secs(15)))
            .timeout(timeout)
            .redirect(Policy::none())
            .retry(reqwest::retry::never())
            .referer(false)
            .tls_built_in_root_certs(false)
            .no_proxy()
            .build()
            .map_err(|_| config_error())?;
        Ok(Self {
            client,
            network_policy: ProviderNetworkPolicy::direct(),
            stac_origin: parsed,
            asset_origin,
            sas_cache: Mutex::new(HashMap::new()),
            stac_item_requests: AtomicU64::new(0),
            sas_token_requests: AtomicU64::new(0),
            asset_head_requests: AtomicU64::new(0),
        })
    }

    fn with_origins(
        network_policy: ProviderNetworkPolicy,
        stac_origin: Url,
        asset_origin: Url,
        timeout: Duration,
    ) -> Result<Self, PlanetaryComputerError> {
        if timeout.is_zero()
            || timeout > Duration::from_secs(120)
            || !exact_origin(&stac_origin, "planetarycomputer.microsoft.com")
            || !exact_origin(&asset_origin, "landsateuwest.blob.core.windows.net")
        {
            return Err(config_error());
        }
        let builder = Client::builder()
            .connect_timeout(timeout.min(Duration::from_secs(15)))
            .timeout(timeout)
            .redirect(Policy::none())
            .retry(reqwest::retry::never())
            .referer(false);
        let builder =
            crate::network_proxy::blocking_builder_with_proxy(builder, network_policy.route());
        let client = builder.build().map_err(|_| config_error())?;
        Ok(Self {
            client,
            network_policy,
            stac_origin,
            asset_origin,
            sas_cache: Mutex::new(HashMap::new()),
            stac_item_requests: AtomicU64::new(0),
            sas_token_requests: AtomicU64::new(0),
            asset_head_requests: AtomicU64::new(0),
        })
    }

    pub const fn network_policy(&self) -> ProviderNetworkPolicy {
        self.network_policy
    }

    pub fn request_counts(&self) -> PlanetaryComputerRequestCounts {
        PlanetaryComputerRequestCounts {
            stac_item: self.stac_item_requests.load(Ordering::Relaxed),
            sas_token: self.sas_token_requests.load(Ordering::Relaxed),
            asset_head: self.asset_head_requests.load(Ordering::Relaxed),
        }
    }

    /// Search Landsat Collection 2 Level-2 through the public Planetary
    /// Computer STAC catalogue. New results retain Microsoft's native Item ID
    /// and source collection; a Landsat scene ID is separate scientific metadata.
    pub fn search_catalog(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<CatalogSearchResponse, PlanetaryComputerError> {
        let filters = validate_public_landsat_query(query)?;
        let aoi_geojson = query.aoi_geojson.as_ref().ok_or_else(|| {
            PlanetaryComputerError::new(
                PlanetaryComputerErrorCode::ConfigInvalid,
                PlanetaryComputerEndpointClass::StacSearch,
            )
        })?;
        check_cancelled(cancelled, PlanetaryComputerEndpointClass::StacSearch)?;
        let search_url = self
            .stac_origin
            .join(STAC_SEARCH_PATH)
            .map_err(|_| config_error())?;
        let requested_limit = query
            .result_limit
            .unwrap_or(MAX_SEARCH_FEATURES as u64)
            .min(MAX_SEARCH_FEATURES as u64);
        // An oversized AOI is sent as one compact envelope; the original
        // canonical AOI then decides admission. The one bounded page asks for
        // the full page so the cap can fill with exact hits.
        let compact = crate::catalog::spatial::compact_oversized_request_aoi(aoi_geojson);
        let (request_aoi, request_limit) = match &compact {
            Some(compact) => (&compact.request_geojson, MAX_SEARCH_FEATURES as u64),
            None => (aoi_geojson, requested_limit),
        };
        let newest_first = filters.start.is_some()
            && filters.end.is_some()
            && crate::catalog::stac_sorts_newest_first(MPC_LANDSAT_PROVIDER_ID);
        let payload = build_public_landsat_search_payload(&filters, request_aoi, request_limit)?;
        // A small AOI keeps the reviewed one bounded page. An envelope page
        // may be mostly outside the AOI, so it follows the reviewed `next`
        // link until the target fills with exact hits, the archive is
        // exhausted, or the explicit page budget / deadline is spent.
        let target = query.result_limit;
        let page_budget = if compact.is_some() || target.is_some_and(|limit| limit > request_limit)
        {
            MAX_PUBLIC_ENVELOPE_SEARCH_PAGES
        } else {
            1
        };
        let deadline = Instant::now() + PUBLIC_ENVELOPE_SEARCH_DEADLINE;
        let mut seen = BTreeSet::<String>::new();
        // Server order is kept until the cap decision (newest-first).
        let mut admitted = Vec::<ProviderProduct>::new();
        let mut seen_continuations = BTreeSet::<String>::new();
        let mut matched_count = None;
        let mut remote_exhausted;
        let mut cloud_filtered_any = false;
        let mut request_count = 0_u64;
        let mut next_request = PublicSearchRequest::Post {
            url: search_url,
            body: payload.clone(),
        };
        loop {
            check_cancelled(cancelled, PlanetaryComputerEndpointClass::StacSearch)?;
            if request_count > 0 && Instant::now() >= deadline {
                return Err(PlanetaryComputerError::new(
                    PlanetaryComputerErrorCode::TransportFailed,
                    PlanetaryComputerEndpointClass::StacSearch,
                ));
            }
            request_count += 1;
            let request = match &next_request {
                PublicSearchRequest::Post { url, body } => self.client.post(url.clone()).json(body),
                PublicSearchRequest::Get { url } => self.client.get(url.clone()),
            };
            let response = request.send().map_err(|_| {
                PlanetaryComputerError::new(
                    PlanetaryComputerErrorCode::TransportFailed,
                    PlanetaryComputerEndpointClass::StacSearch,
                )
            })?;
            let value = read_json(
                response,
                MAX_SEARCH_JSON_BYTES,
                PlanetaryComputerEndpointClass::StacSearch,
            )?;
            check_cancelled(cancelled, PlanetaryComputerEndpointClass::StacSearch)?;
            let features = value
                .get("features")
                .and_then(Value::as_array)
                .filter(|features| features.len() <= MAX_SEARCH_FEATURES)
                .ok_or_else(|| response_error_at(PlanetaryComputerEndpointClass::StacSearch))?;
            let observed = (seen.len() + features.len()) as u64;
            let (page_matched, page_exhausted) =
                public_landsat_page_counts(&value, features.len() as u64, observed)?;
            if request_count > 1 && page_matched != matched_count {
                return Err(response_error_at(
                    PlanetaryComputerEndpointClass::StacSearch,
                ));
            }
            matched_count = page_matched;
            remote_exhausted = page_exhausted;
            for feature in features {
                check_cancelled(cancelled, PlanetaryComputerEndpointClass::StacSearch)?;
                let product = public_landsat_feature_to_product(feature)?;
                if !seen.insert(product.product_id.clone()) {
                    return Err(PlanetaryComputerError::new(
                        PlanetaryComputerErrorCode::IdentityMismatch,
                        PlanetaryComputerEndpointClass::StacSearch,
                    ));
                }
                if compact.as_ref().is_some_and(|compact| {
                    !product
                        .footprint_geojson
                        .as_ref()
                        .is_some_and(|footprint| compact.exact.intersects_footprint(footprint))
                }) {
                    continue;
                }
                if product
                    .properties
                    .get("cloud_cover")
                    .and_then(Value::as_f64)
                    .is_none_or(|cloud| cloud > filters.cloud_cover_lte)
                {
                    cloud_filtered_any = true;
                    continue;
                }
                admitted.push(product);
            }
            let effective_page_budget = if compact.is_some() || cloud_filtered_any {
                MAX_PUBLIC_ENVELOPE_SEARCH_PAGES
            } else {
                page_budget
            };
            let filled = target.is_some_and(|target| admitted.len() as u64 >= target);
            if filled || remote_exhausted || effective_page_budget == 1 {
                break;
            }
            if request_count as usize >= effective_page_budget {
                // Explicit envelope page budget spent before the target filled.
                return Err(PlanetaryComputerError::new(
                    PlanetaryComputerErrorCode::ResponseTooLarge,
                    PlanetaryComputerEndpointClass::StacSearch,
                ));
            }
            next_request = self.public_search_continuation(&value, &payload)?;
            if !seen_continuations.insert(next_request.fingerprint()) {
                return Err(response_error_at(
                    PlanetaryComputerEndpointClass::StacSearch,
                ));
            }
        }
        let fetched_count = seen.len() as u64;
        let exact_count = admitted.len() as u64;
        let working_set_satisfied = target
            .map(|target| exact_count >= target || remote_exhausted)
            .unwrap_or(remote_exhausted);
        if !working_set_satisfied {
            return Err(response_error_at(
                PlanetaryComputerEndpointClass::StacSearch,
            ));
        }
        if !newest_first {
            admitted.sort_by(|a, b| a.product_id.cmp(&b.product_id));
        }
        if let Some(target) = query.result_limit {
            admitted.truncate(target as usize);
        }
        admitted.sort_by(|a, b| a.product_id.cmp(&b.product_id));
        let products = admitted;
        let returned_count = products.len() as u64;
        let response = CatalogSearchResponse {
            collection: public_landsat_collection_descriptor(),
            products,
            stats: CatalogSearchStats {
                requested_limit: query.result_limit,
                // The server total counts the compact/pgstac-rounded response, not exact post-filtered hits.
                matched_count: if compact.is_some() || cloud_filtered_any {
                    None
                } else {
                    matched_count
                },
                request_count,
                transport_part_count: u64::from(query.aoi_geojson.is_some()),
                fetched_record_count: fetched_count,
                mapped_record_count: fetched_count,
                profile_filtered_record_count: 0,
                exact_duplicate_record_count: 0,
                unique_provider_product_count: fetched_count,
                exact_matched_count: exact_count,
                returned_count,
                remote_exhausted,
                working_set_satisfied,
                elapsed_ms: 0,
                truncated: false,
            },
            next_cursor: None,
            item_errors: Vec::new(),
        };
        response
            .validate()
            .map_err(|_| response_error_at(PlanetaryComputerEndpointClass::StacSearch))?;
        Ok(response)
    }

    /// Progressive page-by-page STAC search for Planetary Computer Landsat C2 L2.
    ///
    /// Page 1 requests at most 250 items (when no envelope compaction is active)
    /// so page 1 flushes immediately while subsequent pages stream in the
    /// background up to `query.result_limit` (e.g. 2,000). Undated bounded
    /// queries fail closed with `streaming_cap_order` so `dispatcher` retries on
    /// whole-response `search_catalog` (`SAR-REG-010`).
    pub fn search_catalog_streaming<S, P>(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
        sink: &mut S,
        progress: &P,
    ) -> ProviderResult<(CatalogCollectionDescriptor, CatalogSearchStats)>
    where
        S: FnMut(&[ProviderProduct]) -> ProviderResult<()>,
        P: Fn(crate::asf::catalog_client::CatalogSearchProviderProgress),
    {
        let map_err = |error: PlanetaryComputerError| -> ContractError {
            let mut context = Map::from_iter([(
                "endpoint_class".to_owned(),
                Value::String(error.endpoint().as_str().to_owned()),
            )]);
            if let Some(status) = error.http_status() {
                context.insert("http_status".to_owned(), Value::Number(status.into()));
            }
            ContractError::new(
                error.code_str(),
                "Planetary Computer 公开 Landsat 目录请求失败",
                context,
            )
            .expect("fixed Planetary Computer catalogue context is safe")
        };

        let query = &insar_core::clamp_catalogue_search_query(query);
        let filters = validate_public_landsat_query(query).map_err(map_err)?;
        let aoi_geojson = query.aoi_geojson.as_ref().ok_or_else(|| {
            map_err(PlanetaryComputerError::new(
                PlanetaryComputerErrorCode::ConfigInvalid,
                PlanetaryComputerEndpointClass::StacSearch,
            ))
        })?;
        let newest_first = filters.start.is_some()
            && filters.end.is_some()
            && crate::catalog::stac_sorts_newest_first(MPC_LANDSAT_PROVIDER_ID);
        if !newest_first && query.result_limit.is_some() {
            return Err(ContractError::new(
                "STAC_RESPONSE_INVALID",
                "Planetary Computer Landsat 流式检索需要明确时间范围排序",
                Map::from_iter([(
                    "reason".to_owned(),
                    Value::String("streaming_cap_order".to_owned()),
                )]),
            )
            .expect("streaming_cap_order context is safe"));
        }

        let search_url = self
            .stac_origin
            .join(STAC_SEARCH_PATH)
            .map_err(|_| map_err(config_error()))?;
        let compact = crate::catalog::spatial::compact_oversized_request_aoi(aoi_geojson);
        let per_page_limit = query.result_limit.unwrap_or(250).min(250);
        let (request_aoi, request_limit) = match &compact {
            Some(compact) => (&compact.request_geojson, MAX_SEARCH_FEATURES as u64),
            None => (aoi_geojson, per_page_limit),
        };
        let payload = build_public_landsat_search_payload(&filters, request_aoi, request_limit)
            .map_err(map_err)?;
        let target = query.result_limit;
        let page_budget = if compact.is_some() || target.is_some_and(|limit| limit > request_limit)
        {
            MAX_PUBLIC_ENVELOPE_SEARCH_PAGES
        } else {
            1
        };
        let started = Instant::now();
        let deadline = started + PUBLIC_ENVELOPE_SEARCH_DEADLINE;
        let mut seen = BTreeSet::<String>::new();
        let mut seen_continuations = BTreeSet::<String>::new();
        let mut matched_count = None;
        let mut remote_exhausted;
        let mut cloud_filtered_any = false;
        let mut request_count = 0_u64;
        let mut exact_count = 0_u64;
        let mut returned_count = 0_u64;
        let mut next_request = PublicSearchRequest::Post {
            url: search_url,
            body: payload.clone(),
        };

        loop {
            check_cancelled(cancelled, PlanetaryComputerEndpointClass::StacSearch)
                .map_err(map_err)?;
            if request_count > 0 && Instant::now() >= deadline {
                return Err(map_err(PlanetaryComputerError::new(
                    PlanetaryComputerErrorCode::TransportFailed,
                    PlanetaryComputerEndpointClass::StacSearch,
                )));
            }
            request_count += 1;
            let request = match &next_request {
                PublicSearchRequest::Post { url, body } => self.client.post(url.clone()).json(body),
                PublicSearchRequest::Get { url } => self.client.get(url.clone()),
            };
            let response = request.send().map_err(|_| {
                map_err(PlanetaryComputerError::new(
                    PlanetaryComputerErrorCode::TransportFailed,
                    PlanetaryComputerEndpointClass::StacSearch,
                ))
            })?;
            let value = read_json(
                response,
                MAX_SEARCH_JSON_BYTES,
                PlanetaryComputerEndpointClass::StacSearch,
            )
            .map_err(map_err)?;
            check_cancelled(cancelled, PlanetaryComputerEndpointClass::StacSearch)
                .map_err(map_err)?;
            let features = value
                .get("features")
                .and_then(Value::as_array)
                .filter(|features| features.len() <= MAX_SEARCH_FEATURES)
                .ok_or_else(|| {
                    map_err(response_error_at(
                        PlanetaryComputerEndpointClass::StacSearch,
                    ))
                })?;
            let observed = (seen.len() + features.len()) as u64;
            let (page_matched, page_exhausted) =
                public_landsat_page_counts(&value, features.len() as u64, observed)
                    .map_err(map_err)?;
            if request_count > 1 && page_matched != matched_count {
                return Err(map_err(response_error_at(
                    PlanetaryComputerEndpointClass::StacSearch,
                )));
            }
            matched_count = page_matched;
            remote_exhausted = page_exhausted;
            let mut page_admitted = Vec::with_capacity(features.len());
            for feature in features {
                check_cancelled(cancelled, PlanetaryComputerEndpointClass::StacSearch)
                    .map_err(map_err)?;
                let product = public_landsat_feature_to_product(feature).map_err(map_err)?;
                if !seen.insert(product.product_id.clone()) {
                    return Err(map_err(PlanetaryComputerError::new(
                        PlanetaryComputerErrorCode::IdentityMismatch,
                        PlanetaryComputerEndpointClass::StacSearch,
                    )));
                }
                if compact.as_ref().is_some_and(|compact| {
                    !product
                        .footprint_geojson
                        .as_ref()
                        .is_some_and(|footprint| compact.exact.intersects_footprint(footprint))
                }) {
                    continue;
                }
                if product
                    .properties
                    .get("cloud_cover")
                    .and_then(Value::as_f64)
                    .is_none_or(|cloud| cloud > filters.cloud_cover_lte)
                {
                    cloud_filtered_any = true;
                    continue;
                }
                exact_count += 1;
                if target.is_none_or(|limit| returned_count < limit) {
                    returned_count += 1;
                    page_admitted.push(product);
                }
            }
            if !page_admitted.is_empty() {
                sink(&page_admitted)?;
            }
            let reported_remote_total = if compact.is_some() || cloud_filtered_any {
                None
            } else {
                matched_count
            };
            progress(crate::asf::catalog_client::CatalogSearchProviderProgress {
                provider_request_count: request_count,
                transport_part_count: u64::from(query.aoi_geojson.is_some()),
                raw_fetched_count: seen.len() as u64,
                mapped_count: seen.len() as u64,
                exact_matched_count: exact_count,
                unique_admitted_count: returned_count,
                requested_limit: query.result_limit,
                remote_total: reported_remote_total,
                elapsed_ms: started.elapsed().as_millis().min(u64::MAX as u128) as u64,
            });
            let effective_page_budget = if compact.is_some() || cloud_filtered_any {
                MAX_PUBLIC_ENVELOPE_SEARCH_PAGES
            } else {
                page_budget
            };
            let filled = target.is_some_and(|limit| returned_count >= limit);
            if filled || remote_exhausted || effective_page_budget == 1 {
                break;
            }
            if request_count as usize >= effective_page_budget {
                return Err(map_err(PlanetaryComputerError::new(
                    PlanetaryComputerErrorCode::ResponseTooLarge,
                    PlanetaryComputerEndpointClass::StacSearch,
                )));
            }
            next_request = self
                .public_search_continuation(&value, &payload)
                .map_err(map_err)?;
            if !seen_continuations.insert(next_request.fingerprint()) {
                return Err(map_err(response_error_at(
                    PlanetaryComputerEndpointClass::StacSearch,
                )));
            }
        }
        let fetched_count = seen.len() as u64;
        let working_set_satisfied = target
            .map(|limit| exact_count >= limit || remote_exhausted)
            .unwrap_or(remote_exhausted);
        if !working_set_satisfied {
            return Err(map_err(response_error_at(
                PlanetaryComputerEndpointClass::StacSearch,
            )));
        }
        let stats = CatalogSearchStats {
            requested_limit: query.result_limit,
            matched_count: if compact.is_some() || cloud_filtered_any {
                None
            } else {
                matched_count
            },
            request_count,
            transport_part_count: u64::from(query.aoi_geojson.is_some()),
            fetched_record_count: fetched_count,
            mapped_record_count: fetched_count,
            profile_filtered_record_count: 0,
            exact_duplicate_record_count: 0,
            unique_provider_product_count: fetched_count,
            exact_matched_count: exact_count,
            returned_count,
            remote_exhausted,
            working_set_satisfied,
            elapsed_ms: started.elapsed().as_millis().min(u64::MAX as u128) as u64,
            truncated: false,
        };
        Ok((public_landsat_collection_descriptor(), stats))
    }

    /// The reviewed continuation of one public search page. Only the fixed
    /// search endpoint of this client's STAC origin is followed; a POST body
    /// must keep the original collection, geometry, query and time scope
    /// (`merge: true` merges the link body onto the first request body).
    fn public_search_continuation(
        &self,
        page: &Value,
        original: &Value,
    ) -> Result<PublicSearchRequest, PlanetaryComputerError> {
        let invalid = || response_error_at(PlanetaryComputerEndpointClass::StacSearch);
        let link = page
            .get("links")
            .and_then(Value::as_array)
            .and_then(|links| {
                links
                    .iter()
                    .find(|link| link.get("rel").and_then(Value::as_str) == Some("next"))
            })
            .and_then(Value::as_object)
            .ok_or_else(invalid)?;
        let search_url = self
            .stac_origin
            .join(STAC_SEARCH_PATH)
            .map_err(|_| config_error())?;
        let href = link
            .get("href")
            .and_then(Value::as_str)
            .ok_or_else(invalid)?;
        let url = search_url.join(href).map_err(|_| invalid())?;
        if url.origin() != search_url.origin()
            || url.path() != search_url.path()
            || url.fragment().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(invalid());
        }
        match link.get("method").and_then(Value::as_str).unwrap_or("GET") {
            "POST" => {
                // Live Planetary Computer POST `rel="next"` links may retain
                // `?collections=landsat-c2-l2` on the search endpoint query string.
                if url.query().is_some()
                    && (url.query_pairs().count() == 0
                        || !url.query_pairs().all(|(k, v)| {
                            k == "collections" && v == PLANETARY_COMPUTER_LANDSAT_COLLECTION_ID
                        }))
                {
                    return Err(invalid());
                }
                let link_body = link
                    .get("body")
                    .and_then(Value::as_object)
                    .ok_or_else(invalid)?;
                let mut body = if link.get("merge").and_then(Value::as_bool) == Some(true) {
                    original.as_object().cloned().ok_or_else(invalid)?
                } else {
                    Map::new()
                };
                for (key, value) in link_body {
                    body.insert(key.clone(), value.clone());
                }
                if !body.contains_key("fields") {
                    if let Some(fields) = original.get("fields") {
                        body.insert("fields".to_owned(), fields.clone());
                    }
                }
                for key in ["collections", "intersects", "query", "datetime"] {
                    if !stac_scope_value_eq(body.get(key), original.get(key)) {
                        return Err(invalid());
                    }
                }
                Ok(PublicSearchRequest::Post {
                    url,
                    body: Value::Object(body),
                })
            }
            "GET" => Ok(PublicSearchRequest::Get { url }),
            _ => Err(invalid()),
        }
    }

    /// Fetch a public STAC preview for one scene already present in the frozen
    /// query. The application passes only the opaque scene identity; this
    /// method re-resolves the public STAC item and never accepts a URL from UI.
    pub fn fetch_browse(
        &self,
        query: &CatalogSearchQuery,
        entity_id: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<Option<(String, Vec<u8>)>, PlanetaryComputerError> {
        // Old frozen USGS-tagged scenes already used this public preview port.
        // Adapt only this runtime lookup, never the persisted batch/identity.
        let legacy = query.provider_id == LANDSAT_C2_L2_PROVIDER_ID
            && query.native_collection_id == LANDSAT_C2_L2_NATIVE_COLLECTION_ID;
        let mut lookup = query.clone();
        if legacy {
            lookup.provider_id = MPC_LANDSAT_PROVIDER_ID.to_owned();
            lookup.native_collection_id = MPC_LANDSAT_COLLECTION_ID.to_owned();
        }
        let response = self.search_catalog(&lookup, cancelled)?;
        let Some(product) = response.products.iter().find(|product| {
            if legacy {
                product
                    .properties
                    .get("landsat_scene_id")
                    .and_then(Value::as_str)
                    == Some(entity_id)
            } else {
                product.product_id == entity_id
            }
        }) else {
            return Err(PlanetaryComputerError::new(
                PlanetaryComputerErrorCode::IdentityMismatch,
                PlanetaryComputerEndpointClass::StacSearch,
            ));
        };
        let identity = ParsedProductIdentity::parse(&product.display_name)?;
        let item_url = self
            .stac_origin
            .join(&format!("{STAC_ITEM_PREFIX}{}", identity.stac_item_id))
            .map_err(|_| config_error())?;
        check_cancelled(cancelled, PlanetaryComputerEndpointClass::StacItem)?;
        let item = request_json(
            &self.client,
            item_url,
            MAX_JSON_BYTES,
            PlanetaryComputerEndpointClass::StacItem,
        )?;
        let assets = item
            .get("assets")
            .and_then(Value::as_object)
            .ok_or_else(response_error)?;
        let href = assets
            .get("rendered_preview")
            .or_else(|| assets.get("thumbnail"))
            .and_then(Value::as_object)
            .and_then(|asset| asset.get("href"))
            .and_then(Value::as_str);
        let Some(href) = href else {
            return Ok(None);
        };
        let url = validate_public_browse_url(href)?;
        check_cancelled(cancelled, PlanetaryComputerEndpointClass::StacItem)?;
        let response = self.client.get(url).send().map_err(|_| {
            PlanetaryComputerError::new(
                PlanetaryComputerErrorCode::TransportFailed,
                PlanetaryComputerEndpointClass::StacItem,
            )
        })?;
        if response.status() != StatusCode::OK {
            return Err(PlanetaryComputerError::with_status(
                PlanetaryComputerErrorCode::AssetUnavailable,
                PlanetaryComputerEndpointClass::StacItem,
                response.status(),
            ));
        }
        let media_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .filter(|value| matches!(*value, "image/jpeg" | "image/png" | "image/webp"))
            .ok_or_else(response_error)?
            .to_owned();
        if response
            .content_length()
            .is_some_and(|length| length == 0 || length as usize > MAX_BROWSE_BYTES)
        {
            return Err(PlanetaryComputerError::new(
                PlanetaryComputerErrorCode::ResponseTooLarge,
                PlanetaryComputerEndpointClass::StacItem,
            ));
        }
        let bytes = response.bytes().map_err(|_| {
            PlanetaryComputerError::new(
                PlanetaryComputerErrorCode::TransportFailed,
                PlanetaryComputerEndpointClass::StacItem,
            )
        })?;
        if bytes.is_empty() || bytes.len() > MAX_BROWSE_BYTES {
            return Err(response_error());
        }
        Ok(Some((media_type, bytes.to_vec())))
    }

    pub fn manifest(
        &self,
        dataset_alias: &str,
        entity_id: &str,
        display_id: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<PlanetaryComputerLandsatManifest, PlanetaryComputerError> {
        check_cancelled(cancelled, PlanetaryComputerEndpointClass::StacItem)?;
        if !matches!(
            dataset_alias,
            LANDSAT_C2_L2_NATIVE_COLLECTION_ID
                | "landsat_tm_c2_l2"
                | "landsat_etm_c2_l2"
                | MPC_LANDSAT_COLLECTION_ID
        ) || !valid_entity_id(entity_id)
        {
            return Err(identity_error());
        }
        let identity = ParsedProductIdentity::parse(display_id)?;
        let item_url = self
            .stac_origin
            .join(&format!("{STAC_ITEM_PREFIX}{}", identity.stac_item_id))
            .map_err(|_| config_error())?;
        self.stac_item_requests.fetch_add(1, Ordering::Relaxed);
        let item = match request_json(
            &self.client,
            item_url.clone(),
            MAX_JSON_BYTES,
            PlanetaryComputerEndpointClass::StacItem,
        ) {
            Ok(v) => v,
            Err(err)
                if self.stac_origin.scheme() == "https"
                    && retryable_planetary_computer_request(err) =>
            {
                request_json(
                    &self.client,
                    item_url,
                    MAX_JSON_BYTES,
                    PlanetaryComputerEndpointClass::StacItem,
                )?
            }
            Err(err) => return Err(err),
        };
        let discovered = parse_source_item(dataset_alias, entity_id, &identity, &item)?;
        check_cancelled(cancelled, PlanetaryComputerEndpointClass::SasToken)?;
        let sas = self.sas_token(cancelled)?;
        let mut assets = Vec::with_capacity(discovered.len());
        let mut total_size_bytes = 0_u64;
        if self.stac_origin.scheme() == "https" {
            let mut prepared = Vec::with_capacity(discovered.len());
            for asset in discovered {
                check_cancelled(cancelled, PlanetaryComputerEndpointClass::AssetHead)?;
                let stable_url = validate_stable_asset_url(&asset.stable_href, &identity, &asset)?;
                let signed = append_sas(stable_url, sas.token.as_str())?;
                self.asset_head_requests.fetch_add(1, Ordering::Relaxed);
                prepared.push((asset, signed));
            }
            let sizes: Vec<Result<u64, PlanetaryComputerError>> = std::thread::scope(|s| {
                let handles: Vec<_> = prepared
                    .iter()
                    .map(|(_, signed)| {
                        let client = &self.client;
                        let url = signed.clone();
                        s.spawn(move || match head_size(client, url.clone()) {
                            Ok(sz) => Ok(sz),
                            Err(_) => head_size(client, url),
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|h| {
                        h.join().unwrap_or_else(|_| {
                            Err(PlanetaryComputerError::new(
                                PlanetaryComputerErrorCode::TransportFailed,
                                PlanetaryComputerEndpointClass::AssetHead,
                            ))
                        })
                    })
                    .collect()
            });
            for ((asset, _), size_res) in prepared.into_iter().zip(sizes) {
                let size = size_res?;
                total_size_bytes = total_size_bytes
                    .checked_add(size)
                    .filter(|total| *total <= MAX_MANIFEST_BYTES)
                    .ok_or_else(asset_set_error)?;
                assets.push(PlanetaryComputerLandsatAsset {
                    asset_key: asset.asset_key.to_owned(),
                    file_name: asset.file_name,
                    role: asset.role,
                    stable_href: asset.stable_href,
                    expected_size_bytes: size,
                });
            }
        } else {
            for asset in discovered {
                check_cancelled(cancelled, PlanetaryComputerEndpointClass::AssetHead)?;
                let stable_url = validate_stable_asset_url(&asset.stable_href, &identity, &asset)?;
                let signed = append_sas(stable_url, sas.token.as_str())?;
                self.asset_head_requests.fetch_add(1, Ordering::Relaxed);
                let size = head_size(&self.client, signed)?;
                total_size_bytes = total_size_bytes
                    .checked_add(size)
                    .filter(|total| *total <= MAX_MANIFEST_BYTES)
                    .ok_or_else(asset_set_error)?;
                assets.push(PlanetaryComputerLandsatAsset {
                    asset_key: asset.asset_key.to_owned(),
                    file_name: asset.file_name,
                    role: asset.role,
                    stable_href: asset.stable_href,
                    expected_size_bytes: size,
                });
            }
        }
        Ok(PlanetaryComputerLandsatManifest {
            entity_id: entity_id.to_owned(),
            display_id: identity.display_id,
            stac_item_id: identity.stac_item_id,
            processing_level: identity.processing_level.to_ascii_lowercase(),
            assets,
            total_size_bytes,
        })
    }

    pub fn sign_asset(
        &self,
        stable_href: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<PlanetaryComputerSignedAsset, PlanetaryComputerError> {
        check_cancelled(cancelled, PlanetaryComputerEndpointClass::SasToken)?;
        let stable = Url::parse(stable_href).map_err(|_| identity_error())?;
        validate_asset_origin_and_prefix(&stable)?;
        let sas = self.sas_token(cancelled)?;
        PlanetaryComputerSignedAsset::new(append_sas(stable, sas.token.as_str())?, sas.expires_at)
    }

    pub fn sign_hls_asset(
        &self,
        collection_id: &str,
        stable_href: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<PlanetaryComputerSignedAsset, PlanetaryComputerError> {
        check_cancelled(cancelled, PlanetaryComputerEndpointClass::SasToken)?;
        let stable = Url::parse(stable_href).map_err(|_| identity_error())?;
        validate_hls_asset_origin_and_path(&stable)?;
        let sas = self.collection_sas_token(collection_id, cancelled)?;
        PlanetaryComputerSignedAsset::new(append_sas(stable, sas.token.as_str())?, sas.expires_at)
    }

    pub fn invalidate_sas(&self) -> Result<(), PlanetaryComputerError> {
        let mut cache = self.sas_cache.lock().map_err(|_| config_error())?;
        cache.clear();
        Ok(())
    }

    pub fn invalidate_hls_sas(&self, collection: &str) -> Result<(), PlanetaryComputerError> {
        let mut cache = self.sas_cache.lock().map_err(|_| config_error())?;
        cache.remove(collection);
        Ok(())
    }

    pub fn collection_sas_token(
        &self,
        collection: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<SasLease, PlanetaryComputerError> {
        if !matches!(collection, "landsat-c2-l2" | "hls2-l30" | "hls2-s30") {
            return Err(identity_error());
        }
        let now = SystemTime::now();
        // Keep the cache lock across refresh so concurrent download workers do
        // not stampede the collection-token endpoint when the cache is empty.
        let mut cache = self.sas_cache.lock().map_err(|_| config_error())?;
        if let Some(cached) = cache.get(collection) {
            if now
                .checked_add(TOKEN_REFRESH_MARGIN)
                .is_some_and(|minimum| minimum < cached.expires_at)
            {
                return Ok(SasLease {
                    token: Zeroizing::new(cached.token.to_string()),
                    expires_at: cached.expires_at,
                });
            }
        }

        check_cancelled(cancelled, PlanetaryComputerEndpointClass::SasToken)?;
        let token_path = format!("/api/sas/v1/token/{collection}");
        let token_url = self
            .stac_origin
            .join(&token_path)
            .map_err(|_| config_error())?;
        let value = {
            let mut accepted = None;
            for attempt in 0..SAS_TOKEN_REQUEST_ATTEMPTS {
                check_cancelled(cancelled, PlanetaryComputerEndpointClass::SasToken)?;
                self.sas_token_requests.fetch_add(1, Ordering::Relaxed);
                match request_json(
                    &self.client,
                    token_url.clone(),
                    MAX_TOKEN_BYTES,
                    PlanetaryComputerEndpointClass::SasToken,
                ) {
                    Ok(value) => {
                        accepted = Some(value);
                        break;
                    }
                    Err(error)
                        if attempt + 1 < SAS_TOKEN_REQUEST_ATTEMPTS
                            && retryable_planetary_computer_request(error) =>
                    {
                        wait_sas_token_retry(attempt, cancelled)?;
                    }
                    Err(error) => return Err(error),
                }
            }
            accepted.ok_or_else(|| {
                PlanetaryComputerError::new(
                    PlanetaryComputerErrorCode::TransportFailed,
                    PlanetaryComputerEndpointClass::SasToken,
                )
            })?
        };
        let response: SasResponse = serde_json::from_value(value).map_err(|_| {
            PlanetaryComputerError::new(
                PlanetaryComputerErrorCode::ResponseInvalid,
                PlanetaryComputerEndpointClass::SasToken,
            )
        })?;
        let token = validate_sas_token(response.token)?;
        let expires_at = parse_expiry(&response.expiry)?;
        let refreshed_at = SystemTime::now();
        if refreshed_at
            .checked_add(Duration::from_secs(60))
            .is_none_or(|minimum| minimum >= expires_at)
            || refreshed_at
                .checked_add(Duration::from_secs(24 * 60 * 60))
                .is_some_and(|maximum| expires_at > maximum)
        {
            return Err(PlanetaryComputerError::new(
                PlanetaryComputerErrorCode::ResponseInvalid,
                PlanetaryComputerEndpointClass::SasToken,
            ));
        }
        let lease = SasLease {
            token: Zeroizing::new(token.to_string()),
            expires_at,
        };
        cache.insert(collection.to_owned(), CachedSasToken { token, expires_at });
        Ok(lease)
    }

    fn sas_token(
        &self,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<SasLease, PlanetaryComputerError> {
        self.collection_sas_token(PLANETARY_COMPUTER_LANDSAT_COLLECTION_ID, cancelled)
    }
}

impl fmt::Debug for PlanetaryComputerLandsatClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PlanetaryComputerLandsatClient")
            .field("network_policy", &self.network_policy)
            .field("stac_origin", &self.stac_origin)
            .field("asset_origin", &self.asset_origin)
            .field("sas_token", &"[REDACTED]")
            .finish()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SasResponse {
    token: String,
    #[serde(rename = "msft:expiry")]
    expiry: String,
}

#[derive(Clone, Debug)]
struct DiscoveredAsset {
    asset_key: &'static str,
    file_name: String,
    role: AssetRole,
    stable_href: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LandsatAssetDefinition {
    pub key: &'static str,
    pub suffix: &'static str,
    pub role: AssetRole,
}

type AssetDefinition = LandsatAssetDefinition;

struct ParsedProductIdentity {
    display_id: String,
    stac_item_id: String,
    platform: &'static str,
    processing_level: String,
    path: String,
    row: String,
    acquired: String,
    collection: String,
    category: String,
}

impl ParsedProductIdentity {
    fn parse(display_id: &str) -> Result<Self, PlanetaryComputerError> {
        if display_id.len() > 96
            || !display_id.is_ascii()
            || display_id.chars().any(char::is_control)
        {
            return Err(identity_error());
        }
        let parts = display_id.split('_').collect::<Vec<_>>();
        let [mission, processing_level, path_row, acquired, processed, collection, category] =
            parts.as_slice()
        else {
            return Err(identity_error());
        };
        let platform = match *mission {
            "LT04" => "landsat-4",
            "LT05" => "landsat-5",
            "LE07" => "landsat-7",
            "LC08" => "landsat-8",
            "LC09" => "landsat-9",
            _ => return Err(identity_error()),
        };
        if !matches!(*processing_level, "L2SP" | "L2SR")
            || path_row.len() != 6
            || !path_row.bytes().all(|value| value.is_ascii_digit())
            || *collection != "02"
            || !matches!(*category, "T1" | "T2")
            || NaiveDate::parse_from_str(acquired, "%Y%m%d").is_err()
            || NaiveDate::parse_from_str(processed, "%Y%m%d").is_err()
        {
            return Err(identity_error());
        }
        let path = path_row[..3].to_owned();
        let row = path_row[3..].to_owned();
        let stac_item_id =
            format!("{mission}_{processing_level}_{path_row}_{acquired}_{collection}_{category}");
        Ok(Self {
            display_id: display_id.to_owned(),
            stac_item_id,
            platform,
            processing_level: (*processing_level).to_owned(),
            path,
            row,
            acquired: (*acquired).to_owned(),
            collection: (*collection).to_owned(),
            category: (*category).to_owned(),
        })
    }

    fn directory_prefix(&self) -> String {
        let instrument_folder = match self.platform {
            "landsat-4" | "landsat-5" => "tm",
            "landsat-7" => "etm",
            _ => "oli-tirs",
        };
        format!(
            "/landsat-c2/level-2/standard/{instrument_folder}/{}/{}/{}/{}/",
            &self.acquired[..4],
            self.path,
            self.row,
            self.display_id
        )
    }
}

struct PublicLandsatQueryFilters {
    start: Option<String>,
    end: Option<String>,
    cloud_cover_lte: f64,
    spacecraft: Vec<String>,
    processing_levels: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LandsatStacSearchSource {
    PlanetaryComputer,
    EarthSearch,
}

impl LandsatStacSearchSource {
    const fn provider_id(self) -> &'static str {
        match self {
            Self::PlanetaryComputer => MPC_LANDSAT_PROVIDER_ID,
            Self::EarthSearch => EARTH_SEARCH_LANDSAT_PROVIDER_ID,
        }
    }

    const fn collection_id(self) -> &'static str {
        match self {
            Self::PlanetaryComputer => MPC_LANDSAT_COLLECTION_ID,
            Self::EarthSearch => EARTH_SEARCH_LANDSAT_COLLECTION_ID,
        }
    }

    fn browse_available(self, feature: &Map<String, Value>) -> bool {
        matches!(self, Self::PlanetaryComputer)
            && feature
                .get("assets")
                .and_then(Value::as_object)
                .is_some_and(|assets| {
                    assets.contains_key("rendered_preview") || assets.contains_key("thumbnail")
                })
    }

    fn validate_identity_asset(self, url: &Url) -> Result<(), PlanetaryComputerError> {
        match self {
            Self::PlanetaryComputer => validate_asset_origin_and_prefix(url),
            Self::EarthSearch => {
                if url.scheme() != "s3"
                    || url.host_str() != Some("usgs-landsat")
                    || !url.username().is_empty()
                    || url.password().is_some()
                    || url.query().is_some()
                    || url.fragment().is_some()
                    || !url.path().starts_with("/collection02/level-2/standard/")
                    || url.path().contains("/../")
                    || url.path().contains("/./")
                {
                    return Err(identity_error());
                }
                Ok(())
            }
        }
    }
}

fn validate_public_landsat_query(
    query: &CatalogSearchQuery,
) -> Result<PublicLandsatQueryFilters, PlanetaryComputerError> {
    validate_public_landsat_query_for_source(query, LandsatStacSearchSource::PlanetaryComputer)
}

fn validate_public_landsat_query_for_source(
    query: &CatalogSearchQuery,
    source: LandsatStacSearchSource,
) -> Result<PublicLandsatQueryFilters, PlanetaryComputerError> {
    if query.validate().is_err()
        || query.logical_dataset != "optical.landsat.c2.l2"
        || query.provider_id != source.provider_id()
        || query.native_collection_id != source.collection_id()
        || query.cursor.is_some()
        || query.filters.keys().any(|key| {
            !matches!(
                key.as_str(),
                "datetime" | "cloud_cover_lte" | "spacecraft" | "processing_levels"
            )
        })
    {
        return Err(config_error());
    }
    let (start, end) = match query.filters.get("datetime") {
        None => (None, None),
        Some(value) => {
            let datetime = value
                .as_object()
                .filter(|value| value.len() == 2)
                .ok_or_else(config_error)?;
            let start = normalized_utc_filter(datetime.get("start"))?;
            let end = normalized_utc_filter(datetime.get("end"))?;
            if DateTime::parse_from_rfc3339(&start).map_err(|_| config_error())?
                >= DateTime::parse_from_rfc3339(&end).map_err(|_| config_error())?
            {
                return Err(config_error());
            }
            (Some(start), Some(end))
        }
    };
    let cloud_cover_lte = match query.filters.get("cloud_cover_lte") {
        None => 100.0,
        Some(value) => value.as_f64().ok_or_else(config_error)?,
    };
    if !cloud_cover_lte.is_finite() || !(0.0..=100.0).contains(&cloud_cover_lte) {
        return Err(config_error());
    }
    let spacecraft = closed_public_values(
        query.filters.get("spacecraft"),
        &[
            "landsat_4",
            "landsat_5",
            "landsat_7",
            "landsat_8",
            "landsat_9",
        ],
    )?;
    let processing_levels =
        closed_public_values(query.filters.get("processing_levels"), &["l2sp", "l2sr"])?;
    Ok(PublicLandsatQueryFilters {
        start,
        end,
        cloud_cover_lte,
        spacecraft,
        processing_levels,
    })
}

#[cfg(test)]
pub(crate) fn normalize_earth_search_landsat_query(
    query: &CatalogSearchQuery,
) -> ProviderResult<CatalogSearchQuery> {
    normalize_earth_search_landsat_query_with_exact_aoi(query).map(|(normalized, _)| normalized)
}

/// The normalized Earth Search query plus the exact admission filter that is
/// required when an oversized AOI was replaced by its compact envelope.
pub(crate) fn normalize_earth_search_landsat_query_with_exact_aoi(
    query: &CatalogSearchQuery,
) -> ProviderResult<(
    CatalogSearchQuery,
    Option<crate::catalog::spatial::PreparedCatalogueAoi>,
)> {
    let filters =
        validate_public_landsat_query_for_source(query, LandsatStacSearchSource::EarthSearch)
            .map_err(|error| earth_search_mapping_error(error, "query"))?;
    let platforms = filters
        .spacecraft
        .iter()
        .map(|value| public_landsat_platform_name(value))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| earth_search_mapping_error(error, "query"))?;
    let levels = filters
        .processing_levels
        .iter()
        .map(|value| Value::String(value.to_ascii_uppercase()))
        .collect::<Vec<_>>();
    let mut normalized = query.clone();
    let mut exact = None;
    if let Some(compact) = normalized
        .aoi_geojson
        .as_ref()
        .and_then(crate::catalog::spatial::compact_oversized_request_aoi)
    {
        normalized.aoi_geojson = Some(compact.request_geojson);
        exact = Some(compact.exact);
    }
    normalized.provider_id = EARTH_SEARCH_LANDSAT_PROVIDER_ID.to_owned();
    normalized.native_collection_id = EARTH_SEARCH_LANDSAT_COLLECTION_ID.to_owned();
    normalized.filters = Map::from_iter([
        (
            "cloud_cover_lte".to_owned(),
            Number::from_f64(filters.cloud_cover_lte)
                .map(Value::Number)
                .ok_or_else(|| earth_search_mapping_error(config_error(), "query"))?,
        ),
        (
            "spacecraft".to_owned(),
            Value::Array(
                platforms
                    .into_iter()
                    .map(|value| Value::String(value.to_owned()))
                    .collect(),
            ),
        ),
        ("processing_levels".to_owned(), Value::Array(levels)),
    ]);
    if let (Some(start), Some(end)) = (filters.start, filters.end) {
        normalized
            .filters
            .insert("datetime".to_owned(), json!({"start": start, "end": end}));
    }
    Ok((normalized, exact))
}

fn build_public_landsat_search_payload(
    filters: &PublicLandsatQueryFilters,
    aoi_geojson: &Map<String, Value>,
    requested_limit: u64,
) -> Result<Value, PlanetaryComputerError> {
    if requested_limit == 0 || requested_limit > MAX_SEARCH_FEATURES as u64 {
        return Err(config_error());
    }
    // The caller already decided the request geometry: an oversized AOI is
    // replaced by `compact_oversized_request_aoi` only together with an exact
    // admission filter over the returned page.
    let effective_aoi = aoi_geojson.clone();
    let platforms = filters
        .spacecraft
        .iter()
        .map(|value| public_landsat_platform_name(value))
        .collect::<Result<Vec<_>, _>>()?;
    let processing_levels = filters
        .processing_levels
        .iter()
        .map(|value| value.to_ascii_uppercase())
        .collect::<Vec<_>>();
    let mut stac_query = Map::new();
    stac_query.insert(
        "eo:cloud_cover".to_owned(),
        json!({"lte": filters.cloud_cover_lte}),
    );
    stac_query.insert("platform".to_owned(), json!({"in": platforms}));
    stac_query.insert(
        "landsat:correction".to_owned(),
        json!({"in": processing_levels}),
    );
    let mut payload = Map::from_iter([
        (
            "collections".to_owned(),
            json!([PLANETARY_COMPUTER_LANDSAT_COLLECTION_ID]),
        ),
        ("intersects".to_owned(), Value::Object(effective_aoi)),
        ("limit".to_owned(), Value::Number(requested_limit.into())),
        ("query".to_owned(), Value::Object(stac_query)),
        (
            "fields".to_owned(),
            json!({
                "include": [
                    "id",
                    "type",
                    "collection",
                    "geometry",
                    "properties",
                    "assets.red.href",
                    "assets.rendered_preview.href",
                    "assets.thumbnail.href"
                ],
                "exclude": ["links", "stac_extensions"]
            }),
        ),
    ]);
    if let (Some(start), Some(end)) = (filters.start.as_deref(), filters.end.as_deref()) {
        payload.insert(
            "datetime".to_owned(),
            Value::String(format!("{start}/{end}")),
        );
        // One bounded page: newest-first decides which scenes fill the cap.
        if crate::catalog::stac_sorts_newest_first(MPC_LANDSAT_PROVIDER_ID) {
            payload.insert(
                "sortby".to_owned(),
                json!([{"field": "properties.datetime", "direction": "desc"}]),
            );
        }
    }
    Ok(Value::Object(payload))
}

fn public_landsat_platform_name(value: &str) -> Result<&'static str, PlanetaryComputerError> {
    match value {
        "landsat_4" => Ok("landsat-4"),
        "landsat_5" => Ok("landsat-5"),
        "landsat_7" => Ok("landsat-7"),
        "landsat_8" => Ok("landsat-8"),
        "landsat_9" => Ok("landsat-9"),
        _ => Err(config_error()),
    }
}

fn normalized_utc_filter(value: Option<&Value>) -> Result<String, PlanetaryComputerError> {
    let text = value.and_then(Value::as_str).ok_or_else(config_error)?;
    let parsed = DateTime::parse_from_rfc3339(text).map_err(|_| config_error())?;
    if parsed.offset().local_minus_utc() != 0
        || parsed.to_rfc3339_opts(SecondsFormat::AutoSi, true) != text
    {
        return Err(config_error());
    }
    Ok(text.to_owned())
}

fn closed_public_values(
    value: Option<&Value>,
    allowed: &[&str],
) -> Result<Vec<String>, PlanetaryComputerError> {
    let values = value
        .and_then(Value::as_array)
        .filter(|values| !values.is_empty() && values.len() <= allowed.len())
        .ok_or_else(config_error)?;
    let mut result = BTreeSet::new();
    for value in values {
        let value = value.as_str().ok_or_else(config_error)?;
        if !allowed.contains(&value) || !result.insert(value.to_owned()) {
            return Err(config_error());
        }
    }
    Ok(result.into_iter().collect())
}

fn public_landsat_feature_to_product(
    feature: &Value,
) -> Result<ProviderProduct, PlanetaryComputerError> {
    public_landsat_feature_to_product_for_source(
        feature,
        LandsatStacSearchSource::PlanetaryComputer,
    )
}

pub(crate) fn map_earth_search_landsat_feature(feature: &Value) -> ProviderResult<ProviderProduct> {
    public_landsat_feature_to_product_for_source(feature, LandsatStacSearchSource::EarthSearch)
        .map_err(|error| earth_search_mapping_error(error, "item"))
}

fn public_landsat_feature_to_product_for_source(
    feature: &Value,
    source: LandsatStacSearchSource,
) -> Result<ProviderProduct, PlanetaryComputerError> {
    let object = feature.as_object().ok_or_else(response_error)?;
    if object.get("type").and_then(Value::as_str) != Some("Feature")
        || object.get("collection").and_then(Value::as_str) != Some(source.collection_id())
    {
        return Err(identity_error());
    }
    let properties = object
        .get("properties")
        .and_then(Value::as_object)
        .ok_or_else(response_error)?;
    let item_id = object
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| valid_entity_id(value))
        .ok_or_else(identity_error)?;
    let mission = match properties.get("platform").and_then(Value::as_str) {
        Some("landsat-4") => "LT04",
        Some("landsat-5") => "LT05",
        Some("landsat-7") => "LE07",
        Some("landsat-8") => "LC08",
        Some("landsat-9") => "LC09",
        _ => return Err(identity_error()),
    };
    let processing_level = properties
        .get("landsat:correction")
        .or_else(|| properties.get("landsat:processing_level"))
        .and_then(Value::as_str)
        .filter(|value| matches!(*value, "L2SP" | "L2SR"))
        .ok_or_else(identity_error)?;
    let path = three_digit_property(properties.get("landsat:wrs_path"), 233)
        .map_err(|_| identity_error())?;
    let row = three_digit_property(properties.get("landsat:wrs_row"), 248)
        .map_err(|_| identity_error())?;
    let acquired = property_date(properties.get("datetime"))?;
    let collection = property_code(properties.get("landsat:collection_number"))?;
    let category = property_code(properties.get("landsat:collection_category"))?;
    if collection != "02" || !matches!(category.as_str(), "T1" | "T2") {
        return Err(identity_error());
    }
    let display_id = public_landsat_display_id_from_assets(
        object,
        source,
        mission,
        processing_level,
        &path,
        &row,
        &acquired,
        &collection,
        &category,
    )?;
    let identity = ParsedProductIdentity::parse(&display_id)?;
    if identity.stac_item_id != item_id {
        return Err(identity_error());
    }
    let scene_id = property_text(properties.get("landsat:scene_id"))?;
    let cloud_cover = properties
        .get("eo:cloud_cover")
        .or_else(|| properties.get("landsat:cloud_cover"))
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && (0.0..=100.0).contains(value))
        .ok_or_else(response_error)?;
    let acquisition_datetime = property_datetime(properties.get("datetime"))?;
    let acquisition_precision = "timestamp";
    let footprint = object
        .get("geometry")
        .and_then(Value::as_object)
        .cloned()
        .ok_or_else(response_error)?;
    let browse_available = source.browse_available(object);
    let spacecraft = match mission {
        "LT04" => "landsat_4",
        "LT05" => "landsat_5",
        "LE07" => "landsat_7",
        "LC08" => "landsat_8",
        "LC09" => "landsat_9",
        _ => unreachable!("mission has already been validated"),
    };
    let mut product_properties = Map::from_iter([
        ("entity_id".to_owned(), Value::String(item_id.to_owned())),
        (
            "landsat_product_id".to_owned(),
            Value::String(display_id.clone()),
        ),
        (
            "dataset_alias".to_owned(),
            Value::String(source.collection_id().to_owned()),
        ),
        (
            "spacecraft".to_owned(),
            Value::String(spacecraft.to_owned()),
        ),
        (
            "processing_level".to_owned(),
            Value::String(processing_level.to_ascii_lowercase()),
        ),
        (
            "data_type_l2".to_owned(),
            Value::String(format!(
                "LANDSAT_{}_{}",
                spacecraft.strip_prefix("landsat_").unwrap(),
                processing_level
            )),
        ),
        ("collection_category".to_owned(), Value::String(category)),
        (
            "cloud_cover".to_owned(),
            Number::from_f64(cloud_cover)
                .map(Value::Number)
                .ok_or_else(response_error)?,
        ),
        ("wrs_path".to_owned(), Value::String(path)),
        ("wrs_row".to_owned(), Value::String(row)),
        (
            "acquisition_date".to_owned(),
            Value::String(format!(
                "{}/{}/{}",
                &acquired[0..4],
                &acquired[4..6],
                &acquired[6..8]
            )),
        ),
        (
            "acquisition_precision".to_owned(),
            Value::String(acquisition_precision.to_owned()),
        ),
        ("browse_available".to_owned(), Value::Bool(browse_available)),
        ("landsat_scene_id".to_owned(), Value::String(scene_id)),
    ]);
    let product = ProviderProduct {
        provider_id: source.provider_id().to_owned(),
        product_id: product_properties
            .get("entity_id")
            .and_then(Value::as_str)
            .ok_or_else(response_error)?
            .to_owned(),
        display_name: display_id,
        source_kind: DataSourceKind::Landsat,
        product_kind: ProductKind::OpticalScene,
        acquisition_datetime: Some(acquisition_datetime),
        footprint_geojson: Some(footprint),
        footprint_bbox: None,
        assets: Vec::new(),
        properties: std::mem::take(&mut product_properties),
        payload: Map::new(),
    };
    product.validate().map_err(|_| response_error())?;
    Ok(product)
}

fn earth_search_mapping_error(error: PlanetaryComputerError, stage: &str) -> ContractError {
    let code = match error.code() {
        PlanetaryComputerErrorCode::IdentityMismatch => "EARTH_SEARCH_LANDSAT_IDENTITY_MISMATCH",
        PlanetaryComputerErrorCode::AssetSetInvalid
        | PlanetaryComputerErrorCode::AssetUnavailable => {
            "EARTH_SEARCH_LANDSAT_ASSET_METADATA_INVALID"
        }
        _ => "EARTH_SEARCH_LANDSAT_RESPONSE_INVALID",
    };
    ContractError::new(
        code,
        "Earth Search Landsat 目录元数据不符合已审核产品规则",
        Map::from_iter([
            ("stage".to_owned(), Value::String(stage.to_owned())),
            (
                "source_error".to_owned(),
                Value::String(error.code_str().to_owned()),
            ),
        ]),
    )
    .expect("bounded Earth Search mapping context is safe")
}

#[cfg(test)]
fn public_landsat_response_counts(
    value: &Value,
    returned: u64,
) -> Result<(Option<u64>, bool), PlanetaryComputerError> {
    public_landsat_page_counts(value, returned, returned)
}

/// One page of a possibly multi-page public search: `numberReturned` is
/// this page's length, while the remote total and exhaustion are judged
/// against every raw record observed so far (`observed`).
fn public_landsat_page_counts(
    value: &Value,
    page_returned: u64,
    observed: u64,
) -> Result<(Option<u64>, bool), PlanetaryComputerError> {
    let invalid = || response_error_at(PlanetaryComputerEndpointClass::StacSearch);
    if value
        .get("numberReturned")
        .is_some_and(|count| count.as_u64() != Some(page_returned))
    {
        return Err(invalid());
    }
    let returned = observed;
    let matched = match value.get("numberMatched") {
        None | Some(Value::Null) => None,
        Some(count) => Some(
            count
                .as_u64()
                .filter(|total| *total >= returned)
                .ok_or_else(invalid)?,
        ),
    };
    let links = value
        .get("links")
        .and_then(Value::as_array)
        .ok_or_else(invalid)?;
    let next = links
        .iter()
        .filter(|link| link.get("rel").and_then(Value::as_str) == Some("next"))
        .count();
    if next > 1 || (next == 1 && matched == Some(returned)) {
        return Err(invalid());
    }
    // A missing remote total stays unknown; a page length is not an archive
    // count. Links and, when present, the remote total independently constrain
    // whether the one-page result is exhausted.
    let exhausted = next == 0 && matched.is_none_or(|total| total == returned);
    Ok((matched, exhausted))
}

fn stac_scope_value_eq(a: Option<&Value>, b: Option<&Value>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(lhs), Some(rhs)) => json_value_equiv(lhs, rhs),
        _ => false,
    }
}

fn json_value_equiv(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::String(x), Value::String(y)) => x == y,
        (Value::Number(x), Value::Number(y)) => {
            x == y
                || matches!(
                    (x.as_f64(), y.as_f64()),
                    (Some(xf), Some(yf)) if xf.is_finite() && yf.is_finite() && xf == yf
                )
        }
        (Value::Array(xs), Value::Array(ys)) => {
            xs.len() == ys.len()
                && xs
                    .iter()
                    .zip(ys.iter())
                    .all(|(lhs, rhs)| json_value_equiv(lhs, rhs))
        }
        (Value::Object(xo), Value::Object(yo)) => {
            xo.len() == yo.len()
                && xo
                    .iter()
                    .all(|(k, v)| yo.get(k).is_some_and(|rv| json_value_equiv(v, rv)))
        }
        _ => false,
    }
}

fn public_landsat_collection_descriptor() -> insar_core::CatalogCollectionDescriptor {
    let profile = CollectionProfileRegistry
        .get(MPC_LANDSAT_SCENE_PROFILE_ID)
        .expect("reviewed Microsoft Landsat profile exists");
    insar_core::CatalogCollectionDescriptor {
        logical_dataset: profile.logical_dataset.to_owned(),
        provider_id: profile.provider_id.to_owned(),
        native_collection_id: profile.native_collection_id.to_owned(),
        collection_profile_id: profile.profile_id.to_owned(),
        identity_rule_id: profile.identity_rule_id.to_owned(),
        filename_rule_id: profile.filename_rule_id.to_owned(),
        delivery_model: CatalogDeliveryModel::ArchiveFirst,
        production_enabled: profile.production_enabled,
    }
}

fn property_text(value: Option<&Value>) -> Result<String, PlanetaryComputerError> {
    value
        .and_then(Value::as_str)
        .filter(|value| valid_entity_id(value))
        .map(str::to_owned)
        .ok_or_else(response_error)
}

fn property_code(value: Option<&Value>) -> Result<String, PlanetaryComputerError> {
    value
        .and_then(Value::as_str)
        .filter(|value| {
            (1..=16).contains(&value.len())
                && value.is_ascii()
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        })
        .map(str::to_owned)
        .ok_or_else(response_error)
}

fn property_datetime(value: Option<&Value>) -> Result<String, PlanetaryComputerError> {
    let text = value
        .and_then(Value::as_str)
        .filter(|value| {
            !value.is_empty() && value.len() <= 64 && !value.chars().any(char::is_control)
        })
        .ok_or_else(response_error)?;
    DateTime::parse_from_rfc3339(text).map_err(|_| response_error())?;
    Ok(text.to_owned())
}

#[allow(clippy::too_many_arguments)]
fn public_landsat_display_id_from_assets(
    feature: &Map<String, Value>,
    source: LandsatStacSearchSource,
    mission: &str,
    processing_level: &str,
    path: &str,
    row: &str,
    acquired: &str,
    collection: &str,
    category: &str,
) -> Result<String, PlanetaryComputerError> {
    let href = feature
        .get("assets")
        .and_then(Value::as_object)
        .and_then(|assets| assets.get("red"))
        .and_then(Value::as_object)
        .and_then(|asset| asset.get("href"))
        .and_then(Value::as_str)
        .ok_or_else(asset_set_error)?;
    let url = Url::parse(href).map_err(|_| identity_error())?;
    source.validate_identity_asset(&url)?;
    if url.query().is_some() || url.fragment().is_some() {
        return Err(identity_error());
    }
    let file_name = url
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .filter(|value| !value.is_empty())
        .ok_or_else(identity_error)?;
    let display_id = file_name
        .strip_suffix("_SR_B4.TIF")
        .or_else(|| file_name.strip_suffix("_SR_B3.TIF"))
        .ok_or_else(identity_error)?
        .to_owned();
    let identity = ParsedProductIdentity::parse(&display_id)?;
    let expected_platform = match mission {
        "LT04" => "landsat-4",
        "LT05" => "landsat-5",
        "LE07" => "landsat-7",
        "LC08" => "landsat-8",
        "LC09" => "landsat-9",
        _ => return Err(identity_error()),
    };
    if identity.platform != expected_platform
        || identity.processing_level != processing_level
        || identity.path != path
        || identity.row != row
        || identity.acquired != acquired
        || identity.collection != collection
        || identity.category != category
    {
        return Err(identity_error());
    }
    Ok(display_id)
}

fn property_date(value: Option<&Value>) -> Result<String, PlanetaryComputerError> {
    let text = value.and_then(Value::as_str).ok_or_else(response_error)?;
    let date = text.get(..10).ok_or_else(response_error)?;
    NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(|_| response_error())?;
    Ok(date.replace('-', ""))
}

fn three_digit_property(
    value: Option<&Value>,
    maximum: u16,
) -> Result<String, PlanetaryComputerError> {
    let number = match value {
        Some(Value::Number(value)) => value.as_u64().ok_or_else(response_error)?,
        Some(Value::String(value)) => value.parse::<u64>().map_err(|_| response_error())?,
        _ => return Err(response_error()),
    };
    if !(1..=u64::from(maximum)).contains(&number) {
        return Err(response_error());
    }
    Ok(format!("{number:03}"))
}

fn validate_public_browse_url(value: &str) -> Result<Url, PlanetaryComputerError> {
    let url = Url::parse(value).map_err(|_| identity_error())?;
    if url.scheme() != "https"
        || url.username() != ""
        || url.password().is_some()
        || url.fragment().is_some()
        || !matches!(
            url.host_str(),
            Some("planetarycomputer.microsoft.com") | Some("landsateuwest.blob.core.windows.net")
        )
    {
        return Err(identity_error());
    }
    Ok(url)
}

fn parse_source_item(
    collection_id: &str,
    source_id: &str,
    identity: &ParsedProductIdentity,
    value: &Value,
) -> Result<Vec<DiscoveredAsset>, PlanetaryComputerError> {
    match collection_id {
        LANDSAT_C2_L2_NATIVE_COLLECTION_ID | "landsat_tm_c2_l2" | "landsat_etm_c2_l2" => {
            parse_item(source_id, identity, value)
        }
        MPC_LANDSAT_COLLECTION_ID if source_id == identity.stac_item_id => {
            let scene_id = value
                .get("properties")
                .and_then(|properties| properties.get("landsat:scene_id"))
                .and_then(Value::as_str)
                .filter(|id| valid_entity_id(id))
                .ok_or_else(identity_error)?;
            parse_item(scene_id, identity, value)
        }
        _ => Err(identity_error()),
    }
}

/// Reuse the asset parser's scientific identity check for native Item IDs.
pub fn validate_planetary_computer_landsat_source_id(
    source_id: &str,
    display_id: &str,
) -> Result<(), PlanetaryComputerError> {
    let identity = ParsedProductIdentity::parse(display_id)?;
    if source_id != identity.stac_item_id {
        return Err(identity_error());
    }
    Ok(())
}

fn parse_item(
    entity_id: &str,
    identity: &ParsedProductIdentity,
    value: &Value,
) -> Result<Vec<DiscoveredAsset>, PlanetaryComputerError> {
    let object = value.as_object().ok_or_else(response_error)?;
    if object.get("id").and_then(Value::as_str) != Some(identity.stac_item_id.as_str())
        || object.get("collection").and_then(Value::as_str)
            != Some(PLANETARY_COMPUTER_LANDSAT_COLLECTION_ID)
    {
        return Err(identity_error());
    }
    let properties = object
        .get("properties")
        .and_then(Value::as_object)
        .ok_or_else(response_error)?;
    if properties.get("platform").and_then(Value::as_str) != Some(identity.platform)
        || properties.get("landsat:wrs_path").and_then(Value::as_str)
            != Some(identity.path.as_str())
        || properties.get("landsat:wrs_row").and_then(Value::as_str) != Some(identity.row.as_str())
        || properties.get("landsat:scene_id").and_then(Value::as_str) != Some(entity_id)
        || properties.get("landsat:correction").and_then(Value::as_str)
            != Some(identity.processing_level.as_str())
        || properties
            .get("landsat:collection_number")
            .and_then(Value::as_str)
            != Some(identity.collection.as_str())
        || properties
            .get("landsat:collection_category")
            .and_then(Value::as_str)
            != Some(identity.category.as_str())
    {
        return Err(identity_error());
    }
    let assets = object
        .get("assets")
        .and_then(Value::as_object)
        .ok_or_else(response_error)?;
    let definitions = asset_definitions_for_sensor(identity.platform, &identity.processing_level)?;
    let defined_keys = definitions
        .iter()
        .map(|definition| definition.key)
        .collect::<HashSet<_>>();
    let allowed_ignored = HashSet::from(["tilejson", "rendered_preview", "mtl.json", "thumbnail"]);
    if assets
        .keys()
        .any(|key| !defined_keys.contains(key.as_str()) && !allowed_ignored.contains(key.as_str()))
        || definitions.iter().any(|definition| {
            !is_optional_landsat_asset(identity.platform, definition.key)
                && !assets.contains_key(definition.key)
        })
    {
        return Err(asset_set_error());
    }
    definitions
        .into_iter()
        .filter(|definition| {
            !is_optional_landsat_asset(identity.platform, definition.key)
                || assets.contains_key(definition.key)
        })
        .map(|definition| {
            let asset = assets
                .get(definition.key)
                .and_then(Value::as_object)
                .ok_or_else(asset_set_error)?;
            let stable_href = asset
                .get("href")
                .and_then(Value::as_str)
                .ok_or_else(asset_set_error)?
                .to_owned();
            let file_name = format!("{}{}", identity.display_id, definition.suffix);
            let discovered = DiscoveredAsset {
                asset_key: definition.key,
                file_name,
                role: definition.role,
                stable_href,
            };
            validate_stable_asset_url(&discovered.stable_href, identity, &discovered)?;
            Ok(discovered)
        })
        .collect()
}

pub fn is_optional_landsat_asset(sensor_or_platform: &str, key: &str) -> bool {
    matches!(
        sensor_or_platform,
        "tm" | "etm"
            | "etm+"
            | "landsat-4"
            | "landsat-5"
            | "landsat-7"
            | "landsat_4"
            | "landsat_5"
            | "landsat_7"
            | "LT04"
            | "LT05"
            | "LE07"
    ) && key == "ang"
}

pub fn asset_definitions_for_sensor(
    sensor: &str,
    processing_level: &str,
) -> Result<Vec<LandsatAssetDefinition>, PlanetaryComputerError> {
    let is_l2sp = match processing_level {
        "L2SP" | "l2sp" => true,
        "L2SR" | "l2sr" => false,
        _ => return Err(identity_error()),
    };
    let is_oli_tirs = match sensor {
        "oli-tirs" | "landsat-8" | "landsat-9" | "landsat_8" | "landsat_9" | "LC08" | "LC09" => {
            true
        }
        "tm" | "etm" | "etm+" | "landsat-4" | "landsat-5" | "landsat-7" | "landsat_4"
        | "landsat_5" | "landsat_7" | "LT04" | "LT05" | "LE07" => false,
        _ => return Err(identity_error()),
    };
    if is_oli_tirs {
        let mut definitions = vec![
            definition("coastal", "_SR_B1.TIF", AssetRole::Primary),
            definition("blue", "_SR_B2.TIF", AssetRole::Primary),
            definition("green", "_SR_B3.TIF", AssetRole::Primary),
            definition("red", "_SR_B4.TIF", AssetRole::Primary),
            definition("nir08", "_SR_B5.TIF", AssetRole::Primary),
            definition("swir16", "_SR_B6.TIF", AssetRole::Primary),
            definition("swir22", "_SR_B7.TIF", AssetRole::Primary),
        ];
        if is_l2sp {
            definitions.extend([
                definition("lwir11", "_ST_B10.TIF", AssetRole::Primary),
                definition("drad", "_ST_DRAD.TIF", AssetRole::Primary),
                definition("emis", "_ST_EMIS.TIF", AssetRole::Primary),
                definition("emsd", "_ST_EMSD.TIF", AssetRole::Primary),
                definition("trad", "_ST_TRAD.TIF", AssetRole::Primary),
                definition("urad", "_ST_URAD.TIF", AssetRole::Primary),
                definition("atran", "_ST_ATRAN.TIF", AssetRole::Primary),
                definition("cdist", "_ST_CDIST.TIF", AssetRole::Primary),
                definition("qa", "_ST_QA.TIF", AssetRole::Auxiliary),
            ]);
        }
        definitions.extend([
            definition("qa_pixel", "_QA_PIXEL.TIF", AssetRole::Auxiliary),
            definition("qa_radsat", "_QA_RADSAT.TIF", AssetRole::Auxiliary),
            definition("qa_aerosol", "_SR_QA_AEROSOL.TIF", AssetRole::Auxiliary),
            definition("ang", "_ANG.txt", AssetRole::Metadata),
            definition("mtl.txt", "_MTL.txt", AssetRole::Metadata),
            definition("mtl.xml", "_MTL.xml", AssetRole::Metadata),
        ]);
        Ok(definitions)
    } else {
        let mut definitions = vec![
            definition("blue", "_SR_B1.TIF", AssetRole::Primary),
            definition("green", "_SR_B2.TIF", AssetRole::Primary),
            definition("red", "_SR_B3.TIF", AssetRole::Primary),
            definition("nir08", "_SR_B4.TIF", AssetRole::Primary),
            definition("swir16", "_SR_B5.TIF", AssetRole::Primary),
            definition("swir22", "_SR_B7.TIF", AssetRole::Primary),
        ];
        if is_l2sp {
            definitions.extend([
                definition("lwir", "_ST_B6.TIF", AssetRole::Primary),
                definition("drad", "_ST_DRAD.TIF", AssetRole::Primary),
                definition("emis", "_ST_EMIS.TIF", AssetRole::Primary),
                definition("emsd", "_ST_EMSD.TIF", AssetRole::Primary),
                definition("trad", "_ST_TRAD.TIF", AssetRole::Primary),
                definition("urad", "_ST_URAD.TIF", AssetRole::Primary),
                definition("atran", "_ST_ATRAN.TIF", AssetRole::Primary),
                definition("cdist", "_ST_CDIST.TIF", AssetRole::Primary),
                definition("qa", "_ST_QA.TIF", AssetRole::Auxiliary),
            ]);
        }
        definitions.extend([
            definition(
                "atmos_opacity",
                "_SR_ATMOS_OPACITY.TIF",
                AssetRole::Auxiliary,
            ),
            definition("cloud_qa", "_SR_CLOUD_QA.TIF", AssetRole::Auxiliary),
            definition("qa_pixel", "_QA_PIXEL.TIF", AssetRole::Auxiliary),
            definition("qa_radsat", "_QA_RADSAT.TIF", AssetRole::Auxiliary),
            definition("ang", "_ANG.txt", AssetRole::Metadata),
            definition("mtl.txt", "_MTL.txt", AssetRole::Metadata),
            definition("mtl.xml", "_MTL.xml", AssetRole::Metadata),
        ]);
        Ok(definitions)
    }
}

pub fn validate_planetary_computer_landsat_manifest_asset_keys(
    display_id: &str,
    processing_level: &str,
    asset_keys: &HashSet<&str>,
) -> Result<(), PlanetaryComputerError> {
    let identity = ParsedProductIdentity::parse(display_id)?;
    if !identity
        .processing_level
        .eq_ignore_ascii_case(processing_level)
    {
        return Err(identity_error());
    }
    let definitions = asset_definitions_for_sensor(identity.platform, &identity.processing_level)?;
    let defined_keys = definitions
        .iter()
        .map(|definition| definition.key)
        .collect::<HashSet<_>>();
    if asset_keys.iter().any(|key| !defined_keys.contains(*key))
        || definitions.iter().any(|definition| {
            !is_optional_landsat_asset(identity.platform, definition.key)
                && !asset_keys.contains(definition.key)
        })
    {
        return Err(asset_set_error());
    }
    Ok(())
}

fn definition(key: &'static str, suffix: &'static str, role: AssetRole) -> AssetDefinition {
    AssetDefinition { key, suffix, role }
}

fn validate_stable_asset_url(
    stable_href: &str,
    identity: &ParsedProductIdentity,
    asset: &DiscoveredAsset,
) -> Result<Url, PlanetaryComputerError> {
    let url = Url::parse(stable_href).map_err(|_| identity_error())?;
    validate_asset_origin_and_prefix(&url)?;
    if !url.path().starts_with(&identity.directory_prefix())
        || url.path().rsplit('/').next() != Some(asset.file_name.as_str())
    {
        return Err(identity_error());
    }
    Ok(url)
}

pub fn validate_planetary_computer_landsat_stable_href(
    stable_href: &str,
) -> Result<(), PlanetaryComputerError> {
    let url = Url::parse(stable_href).map_err(|_| identity_error())?;
    validate_asset_origin_and_prefix(&url)
}

pub fn validate_planetary_computer_landsat_asset(
    stable_href: &str,
    display_id: &str,
    asset_key: &str,
    file_name: &str,
    role: &AssetRole,
) -> Result<(), PlanetaryComputerError> {
    let identity = ParsedProductIdentity::parse(display_id)?;
    let definition = asset_definitions_for_sensor(identity.platform, &identity.processing_level)?
        .into_iter()
        .find(|definition| definition.key == asset_key)
        .ok_or_else(asset_set_error)?;
    let expected_file_name = format!("{}{}", identity.display_id, definition.suffix);
    if file_name != expected_file_name || role != &definition.role {
        return Err(identity_error());
    }
    let asset = DiscoveredAsset {
        asset_key: definition.key,
        file_name: expected_file_name,
        role: definition.role,
        stable_href: stable_href.to_owned(),
    };
    validate_stable_asset_url(stable_href, &identity, &asset).map(|_| ())
}

fn validate_asset_origin_and_prefix(url: &Url) -> Result<(), PlanetaryComputerError> {
    let path = url.path();
    if !exact_https_host(url, "landsateuwest.blob.core.windows.net")
        || url.query().is_some()
        || url.fragment().is_some()
        || !is_valid_planetary_computer_landsat_path(path)
        || path.contains("/../")
        || path.contains("/./")
    {
        return Err(identity_error());
    }
    Ok(())
}

pub fn validate_hls_asset_origin_and_path(url: &Url) -> Result<(), PlanetaryComputerError> {
    let path = url.path();
    let host_ok = exact_https_host(url, "hls2euwest.blob.core.windows.net")
        || exact_https_host(url, "hlseuwest.blob.core.windows.net");
    if !host_ok
        || url.query().is_some()
        || url.fragment().is_some()
        || !(path.starts_with("/hls2/") || path.starts_with("/hls/"))
        || !(path.ends_with(".tif") || path.ends_with(".tiff"))
        || path.contains("/../")
        || path.contains("/./")
    {
        return Err(identity_error());
    }
    Ok(())
}

fn exact_origin(url: &Url, host: &str) -> bool {
    exact_https_host(url, host)
        && url.path() == "/"
        && url.query().is_none()
        && url.fragment().is_none()
}

fn exact_https_host(url: &Url, host: &str) -> bool {
    url.scheme() == "https"
        && url.host_str() == Some(host)
        && url.port_or_known_default() == Some(443)
        && url.username().is_empty()
        && url.password().is_none()
}

fn retryable_planetary_computer_request(error: PlanetaryComputerError) -> bool {
    if error.code() == PlanetaryComputerErrorCode::TransportFailed {
        return true;
    }
    error.code() == PlanetaryComputerErrorCode::HttpRejected
        && matches!(error.http_status(), Some(408 | 429 | 500 | 502 | 503 | 504))
}

fn sas_token_retry_delay(attempt: usize) -> Duration {
    match attempt {
        0 => Duration::from_millis(250),
        1 => Duration::from_millis(750),
        _ => Duration::from_millis(1_500),
    }
}

fn wait_sas_token_retry(
    attempt: usize,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<(), PlanetaryComputerError> {
    let delay = sas_token_retry_delay(attempt);
    let mut waited = Duration::ZERO;
    while waited < delay {
        check_cancelled(cancelled, PlanetaryComputerEndpointClass::SasToken)?;
        let remaining = delay.saturating_sub(waited);
        let step = remaining.min(SAS_TOKEN_RETRY_POLL);
        std::thread::sleep(step);
        waited += step;
    }
    check_cancelled(cancelled, PlanetaryComputerEndpointClass::SasToken)
}

fn request_json(
    client: &Client,
    url: Url,
    limit: u64,
    endpoint: PlanetaryComputerEndpointClass,
) -> Result<Value, PlanetaryComputerError> {
    let response = client.get(url).send().map_err(|_| {
        PlanetaryComputerError::new(PlanetaryComputerErrorCode::TransportFailed, endpoint)
    })?;
    read_json(response, limit, endpoint)
}

fn read_json(
    response: Response,
    limit: u64,
    endpoint: PlanetaryComputerEndpointClass,
) -> Result<Value, PlanetaryComputerError> {
    if response.status() != StatusCode::OK {
        return Err(PlanetaryComputerError::with_status(
            PlanetaryComputerErrorCode::HttpRejected,
            endpoint,
            response.status(),
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > limit)
    {
        return Err(PlanetaryComputerError::new(
            PlanetaryComputerErrorCode::ResponseTooLarge,
            endpoint,
        ));
    }
    let bytes = response.bytes().map_err(|_| {
        PlanetaryComputerError::new(PlanetaryComputerErrorCode::TransportFailed, endpoint)
    })?;
    if bytes.is_empty() || bytes.len() as u64 > limit {
        return Err(PlanetaryComputerError::new(
            PlanetaryComputerErrorCode::ResponseTooLarge,
            endpoint,
        ));
    }
    serde_json::from_slice(&bytes).map_err(|_| {
        PlanetaryComputerError::new(PlanetaryComputerErrorCode::ResponseInvalid, endpoint)
    })
}

fn head_size(client: &Client, signed_url: Url) -> Result<u64, PlanetaryComputerError> {
    let response = client.head(signed_url.clone()).send().map_err(|_| {
        PlanetaryComputerError::new(
            PlanetaryComputerErrorCode::TransportFailed,
            PlanetaryComputerEndpointClass::AssetHead,
        )
    })?;
    if response.status() != StatusCode::OK {
        return Err(PlanetaryComputerError::with_status(
            PlanetaryComputerErrorCode::AssetUnavailable,
            PlanetaryComputerEndpointClass::AssetHead,
            response.status(),
        ));
    }
    if let Some(size) = response
        .content_length()
        .filter(|size| (1..=MAX_ASSET_BYTES).contains(size))
    {
        return Ok(size);
    }
    drop(response);

    let response = client
        .get(signed_url)
        .header(RANGE, "bytes=0-0")
        .send()
        .map_err(|_| {
            PlanetaryComputerError::new(
                PlanetaryComputerErrorCode::TransportFailed,
                PlanetaryComputerEndpointClass::AssetHead,
            )
        })?;
    if response.status() != StatusCode::PARTIAL_CONTENT {
        return Err(PlanetaryComputerError::with_status(
            PlanetaryComputerErrorCode::AssetUnavailable,
            PlanetaryComputerEndpointClass::AssetHead,
            response.status(),
        ));
    }
    let total = response
        .headers()
        .get(CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_single_byte_content_range)
        .ok_or_else(|| {
            PlanetaryComputerError::new(
                PlanetaryComputerErrorCode::SizeMissing,
                PlanetaryComputerEndpointClass::AssetHead,
            )
        })?;
    let mut bounded = response.take(2);
    let mut body = Vec::with_capacity(2);
    bounded.read_to_end(&mut body).map_err(|_| {
        PlanetaryComputerError::new(
            PlanetaryComputerErrorCode::TransportFailed,
            PlanetaryComputerEndpointClass::AssetHead,
        )
    })?;
    if body.len() != 1 {
        return Err(PlanetaryComputerError::new(
            PlanetaryComputerErrorCode::SizeMissing,
            PlanetaryComputerEndpointClass::AssetHead,
        ));
    }
    Ok(total)
}

fn parse_single_byte_content_range(value: &str) -> Option<u64> {
    let total = value.strip_prefix("bytes 0-0/")?;
    if total.is_empty() || !total.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    total
        .parse::<u64>()
        .ok()
        .filter(|size| (1..=MAX_ASSET_BYTES).contains(size))
}

fn append_sas(mut stable: Url, token: &str) -> Result<Url, PlanetaryComputerError> {
    if stable.query().is_some() || stable.fragment().is_some() {
        return Err(identity_error());
    }
    stable.set_query(Some(token));
    if stable.query().is_none() {
        return Err(PlanetaryComputerError::new(
            PlanetaryComputerErrorCode::ResponseInvalid,
            PlanetaryComputerEndpointClass::SasToken,
        ));
    }
    Ok(stable)
}

fn validate_sas_token(value: String) -> Result<Zeroizing<String>, PlanetaryComputerError> {
    if value.len() < 32
        || value.len() > 8192
        || !value.is_ascii()
        || value.chars().any(char::is_control)
        || value.contains('#')
    {
        return Err(token_response_error());
    }
    let fields = url::form_urlencoded::parse(value.as_bytes())
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<HashMap<_, _>>();
    if !["se", "sig", "sp", "sr", "sv"]
        .iter()
        .all(|key| fields.get(*key).is_some_and(|value| !value.is_empty()))
        || !fields.get("sp").is_some_and(|value| value.contains('r'))
    {
        return Err(token_response_error());
    }
    Ok(Zeroizing::new(value))
}

fn parse_expiry(value: &str) -> Result<SystemTime, PlanetaryComputerError> {
    let parsed = DateTime::parse_from_rfc3339(value).map_err(|_| token_response_error())?;
    let timestamp = parsed.with_timezone(&Utc).timestamp();
    let seconds = u64::try_from(timestamp).map_err(|_| token_response_error())?;
    SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(seconds))
        .ok_or_else(token_response_error)
}

fn check_cancelled(
    cancelled: &(dyn Fn() -> bool + Sync),
    endpoint: PlanetaryComputerEndpointClass,
) -> Result<(), PlanetaryComputerError> {
    if cancelled() {
        Err(PlanetaryComputerError::new(
            PlanetaryComputerErrorCode::Cancelled,
            endpoint,
        ))
    } else {
        Ok(())
    }
}

fn valid_entity_id(value: &str) -> bool {
    (8..=96).contains(&value.len())
        && value.is_ascii()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn config_error() -> PlanetaryComputerError {
    PlanetaryComputerError::new(
        PlanetaryComputerErrorCode::ConfigInvalid,
        PlanetaryComputerEndpointClass::Configuration,
    )
}

fn response_error() -> PlanetaryComputerError {
    response_error_at(PlanetaryComputerEndpointClass::StacItem)
}

fn response_error_at(endpoint: PlanetaryComputerEndpointClass) -> PlanetaryComputerError {
    PlanetaryComputerError::new(PlanetaryComputerErrorCode::ResponseInvalid, endpoint)
}

fn identity_error() -> PlanetaryComputerError {
    PlanetaryComputerError::new(
        PlanetaryComputerErrorCode::IdentityMismatch,
        PlanetaryComputerEndpointClass::Identity,
    )
}

fn asset_set_error() -> PlanetaryComputerError {
    PlanetaryComputerError::new(
        PlanetaryComputerErrorCode::AssetSetInvalid,
        PlanetaryComputerEndpointClass::Identity,
    )
}

fn token_response_error() -> PlanetaryComputerError {
    PlanetaryComputerError::new(
        PlanetaryComputerErrorCode::ResponseInvalid,
        PlanetaryComputerEndpointClass::SasToken,
    )
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Map};

    use super::*;

    const ENTITY_ID: &str = "LC90130352023085LGN00";
    const DISPLAY_ID: &str = "LC09_L2SP_013035_20230326_20230328_02_T1";

    fn item(display_id: &str, entity_id: &str) -> Value {
        let identity = ParsedProductIdentity::parse(display_id).unwrap();
        let mut assets = Map::new();
        for definition in
            asset_definitions_for_sensor(identity.platform, &identity.processing_level).unwrap()
        {
            let file_name = format!("{display_id}{}", definition.suffix);
            assets.insert(
                definition.key.to_owned(),
                json!({
                    "href": format!(
                        "{PLANETARY_COMPUTER_LANDSAT_ASSET_ORIGIN}{}{file_name}",
                        identity.directory_prefix()
                    )
                }),
            );
        }
        assets.insert("mtl.json".to_owned(), json!({"href": "ignored"}));
        assets.insert("tilejson".to_owned(), json!({"href": "ignored"}));
        assets.insert("rendered_preview".to_owned(), json!({"href": "ignored"}));
        json!({
            "id": identity.stac_item_id,
            "collection": PLANETARY_COMPUTER_LANDSAT_COLLECTION_ID,
            "properties": {
                "platform": identity.platform,
                "landsat:wrs_path": identity.path,
                "landsat:wrs_row": identity.row,
                "landsat:scene_id": entity_id,
                "landsat:correction": identity.processing_level,
                "landsat:collection_number": identity.collection,
                "landsat:collection_category": identity.category
            },
            "assets": assets
        })
    }

    #[test]
    fn l2sp_and_l2sr_items_reduce_to_closed_complete_asset_sets() {
        let l2sp = ParsedProductIdentity::parse(DISPLAY_ID).unwrap();
        let mapped = parse_item(ENTITY_ID, &l2sp, &item(DISPLAY_ID, ENTITY_ID)).unwrap();
        assert_eq!(mapped.len(), 22);
        assert_eq!(mapped.first().unwrap().asset_key, "coastal");
        assert_eq!(mapped.last().unwrap().asset_key, "mtl.xml");
        assert_eq!(
            mapped
                .iter()
                .filter(|asset| asset.role == AssetRole::Metadata)
                .count(),
            3
        );

        let display = "LC08_L2SR_085082_20230326_20230401_02_T1";
        let l2sr = ParsedProductIdentity::parse(display).unwrap();
        let mapped = parse_item(
            "LC80850822023085LGN00",
            &l2sr,
            &item(display, "LC80850822023085LGN00"),
        )
        .unwrap();
        assert_eq!(mapped.len(), 13);
        assert!(!mapped.iter().any(|asset| asset.asset_key == "lwir11"));
    }

    #[test]
    fn identity_asset_set_and_stable_paths_fail_closed() {
        let identity = ParsedProductIdentity::parse(DISPLAY_ID).unwrap();
        let mut wrong_scene = item(DISPLAY_ID, ENTITY_ID);
        wrong_scene["properties"]["landsat:scene_id"] = json!("OTHER");
        assert_eq!(
            parse_item(ENTITY_ID, &identity, &wrong_scene)
                .unwrap_err()
                .code(),
            PlanetaryComputerErrorCode::IdentityMismatch
        );

        let mut missing = item(DISPLAY_ID, ENTITY_ID);
        missing["assets"].as_object_mut().unwrap().remove("blue");
        assert_eq!(
            parse_item(ENTITY_ID, &identity, &missing)
                .unwrap_err()
                .code(),
            PlanetaryComputerErrorCode::AssetSetInvalid
        );

        let mut extra = item(DISPLAY_ID, ENTITY_ID);
        extra["assets"]["vendor-secret"] = json!({"href": "https://example.invalid"});
        assert_eq!(
            parse_item(ENTITY_ID, &identity, &extra).unwrap_err().code(),
            PlanetaryComputerErrorCode::AssetSetInvalid
        );

        let mut signed = item(DISPLAY_ID, ENTITY_ID);
        let href = signed["assets"]["blue"]["href"]
            .as_str()
            .unwrap()
            .to_owned();
        signed["assets"]["blue"]["href"] = json!(format!("{href}?sig=secret"));
        assert_eq!(
            parse_item(ENTITY_ID, &identity, &signed)
                .unwrap_err()
                .code(),
            PlanetaryComputerErrorCode::IdentityMismatch
        );
    }

    #[test]
    fn product_identity_and_sas_validation_are_closed_and_redacted() {
        let identity = ParsedProductIdentity::parse(DISPLAY_ID).unwrap();
        assert_eq!(identity.stac_item_id, "LC09_L2SP_013035_20230326_02_T1");
        for invalid in [
            "LC07_L2SP_013035_20230326_20230328_02_T1",
            "LC09_L1TP_013035_20230326_20230328_02_T1",
            "LC09_L2SP_013035_20230326_20230328_01_T1",
            "LC09_L2SP_013035_20230326_20230328_02_RT",
        ] {
            assert!(ParsedProductIdentity::parse(invalid).is_err());
        }
        let token = validate_sas_token(
            "st=2026-07-29T00%3A00%3A00Z&se=2026-07-29T01%3A00%3A00Z&sp=rl&sv=1&sr=c&sig=fixture-secret"
                .to_owned(),
        )
        .unwrap();
        let signed = PlanetaryComputerSignedAsset {
            url: append_sas(
                Url::parse(&format!(
                    "{PLANETARY_COMPUTER_LANDSAT_ASSET_ORIGIN}{}{}_MTL.txt",
                    identity.directory_prefix(),
                    DISPLAY_ID
                ))
                .unwrap(),
                token.as_str(),
            )
            .unwrap(),
            expires_at: SystemTime::UNIX_EPOCH + Duration::from_secs(1),
        };
        assert!(!format!("{signed:?}").contains("fixture-secret"));
    }

    #[test]
    fn native_and_legacy_manifest_inputs_reuse_the_same_scientific_validation() {
        let identity = ParsedProductIdentity::parse(DISPLAY_ID).unwrap();
        let response = item(DISPLAY_ID, ENTITY_ID);
        let legacy = parse_source_item(
            LANDSAT_C2_L2_NATIVE_COLLECTION_ID,
            ENTITY_ID,
            &identity,
            &response,
        )
        .unwrap();
        let native = parse_source_item(
            MPC_LANDSAT_COLLECTION_ID,
            &identity.stac_item_id,
            &identity,
            &response,
        )
        .unwrap();
        assert_eq!(legacy.len(), 22);
        assert_eq!(native.len(), legacy.len());
        for (old, new) in legacy.iter().zip(native.iter()) {
            assert_eq!(old.asset_key, new.asset_key);
            assert_eq!(old.file_name, new.file_name);
            assert_eq!(old.stable_href, new.stable_href);
        }
        assert!(
            parse_source_item(MPC_LANDSAT_COLLECTION_ID, ENTITY_ID, &identity, &response).is_err()
        );
        assert!(parse_source_item(
            LANDSAT_C2_L2_NATIVE_COLLECTION_ID,
            &identity.stac_item_id,
            &identity,
            &response
        )
        .is_err());
        let mut invalid = response;
        invalid["properties"]["landsat:correction"] = json!("L1TP");
        assert!(parse_source_item(
            MPC_LANDSAT_COLLECTION_ID,
            &identity.stac_item_id,
            &identity,
            &invalid
        )
        .is_err());
    }

    #[test]
    fn source_id_validation_never_turns_a_scene_id_or_other_product_into_an_item_id() {
        let identity = ParsedProductIdentity::parse(DISPLAY_ID).unwrap();
        assert!(
            validate_planetary_computer_landsat_source_id(&identity.stac_item_id, DISPLAY_ID)
                .is_ok()
        );
        assert!(validate_planetary_computer_landsat_source_id(ENTITY_ID, DISPLAY_ID).is_err());
        assert!(validate_planetary_computer_landsat_source_id(
            "LC09_L2SP_013036_20230326_02_T1",
            DISPLAY_ID
        )
        .is_err());
    }

    #[test]
    fn public_stac_feature_preserves_science_with_truthful_microsoft_identity() {
        let feature = json!({
            "type": "Feature",
            "id": "LC09_L2SP_093061_20230326_02_T2",
            "collection": PLANETARY_COMPUTER_LANDSAT_COLLECTION_ID,
            "geometry": {
                "type": "Polygon",
                "coordinates": [[[151.9, -2.4], [152.0, -2.4], [152.0, -2.3], [151.9, -2.4]]]
            },
            "properties": {
                "datetime": "2023-03-26T23:59:40.503132Z",
                "platform": "landsat-9",
                "landsat:wrs_path": "093",
                "landsat:wrs_row": "061",
                "landsat:scene_id": "LC90930612023085LGN00",
                "landsat:correction": "L2SP",
                "landsat:collection_number": "02",
                "landsat:collection_category": "T2",
                "eo:cloud_cover": 3.34
            },
            "assets": {
                "red": {
                    "href": "https://landsateuwest.blob.core.windows.net/landsat-c2/level-2/standard/oli-tirs/2023/093/061/LC09_L2SP_093061_20230326_20230329_02_T2/LC09_L2SP_093061_20230326_20230329_02_T2_SR_B4.TIF"
                },
                "rendered_preview": {
                    "href": "https://planetarycomputer.microsoft.com/api/data/v1/item/preview.png"
                }
            }
        });
        let product =
            public_landsat_feature_to_product(&feature).expect("public STAC feature maps");
        assert_eq!(product.provider_id, MPC_LANDSAT_PROVIDER_ID);
        assert_eq!(product.product_id, "LC09_L2SP_093061_20230326_02_T2");
        assert_eq!(product.properties["entity_id"], product.product_id);
        assert_eq!(
            product.properties["dataset_alias"],
            MPC_LANDSAT_COLLECTION_ID
        );
        assert_eq!(
            product.properties["landsat_scene_id"],
            "LC90930612023085LGN00"
        );
        assert_eq!(
            product.display_name,
            "LC09_L2SP_093061_20230326_20230329_02_T2"
        );
        assert_eq!(
            product.acquisition_datetime.as_deref(),
            Some("2023-03-26T23:59:40.503132Z")
        );
        assert_eq!(product.properties["acquisition_date"], "2023/03/26");
        assert_eq!(product.properties["acquisition_precision"], "timestamp");
        assert_eq!(product.properties["browse_available"], true);
        assert_eq!(product.properties["processing_level"], "l2sp");

        let mut midnight_feature = feature.clone();
        midnight_feature["properties"]["datetime"] = json!("2023-03-26T00:00:00Z");
        let midnight_product = public_landsat_feature_to_product(&midnight_feature)
            .expect("a midnight STAC datetime remains a timestamp");
        assert_eq!(
            midnight_product.acquisition_datetime.as_deref(),
            Some("2023-03-26T00:00:00Z")
        );
        assert_eq!(
            midnight_product.properties["acquisition_precision"],
            "timestamp"
        );
    }

    #[test]
    fn earth_search_reuses_landsat_science_mapping_but_keeps_search_only_source_identity() {
        let earth = json!({
            "type": "Feature",
            "id": "LC09_L2SP_093061_20230326_02_T2",
            "collection": EARTH_SEARCH_LANDSAT_COLLECTION_ID,
            "geometry": {
                "type": "Polygon",
                "coordinates": [[[151.9, -2.4], [152.0, -2.4], [152.0, -2.3], [151.9, -2.4]]]
            },
            "properties": {
                "datetime": "2023-03-26T23:59:40.503132Z",
                "platform": "landsat-9",
                "landsat:wrs_path": "093",
                "landsat:wrs_row": "061",
                "landsat:scene_id": "LC90930612023085LGN00",
                "landsat:correction": "L2SP",
                "landsat:collection_number": "02",
                "landsat:collection_category": "T2",
                "eo:cloud_cover": 3.34
            },
            "assets": {
                "red": {
                    "href": "s3://usgs-landsat/collection02/level-2/standard/oli-tirs/2023/093/061/LC09_L2SP_093061_20230326_20230329_02_T2/LC09_L2SP_093061_20230326_20230329_02_T2_SR_B4.TIF"
                }
            }
        });
        let product = map_earth_search_landsat_feature(&earth).expect("Earth Search item maps");
        assert_eq!(product.provider_id, EARTH_SEARCH_LANDSAT_PROVIDER_ID);
        assert_eq!(product.product_id, "LC09_L2SP_093061_20230326_02_T2");
        assert_eq!(
            product.display_name,
            "LC09_L2SP_093061_20230326_20230329_02_T2"
        );
        assert_eq!(
            product.properties["dataset_alias"],
            EARTH_SEARCH_LANDSAT_COLLECTION_ID
        );
        assert_eq!(
            product.properties["landsat_scene_id"],
            "LC90930612023085LGN00"
        );
        assert_eq!(product.properties["processing_level"], "l2sp");
        assert_eq!(product.properties["browse_available"], false);
        assert!(
            product.assets.is_empty(),
            "search-only source never grants executable assets"
        );

        let mut missing = earth.clone();
        missing["properties"]
            .as_object_mut()
            .unwrap()
            .remove("landsat:wrs_path");
        assert_eq!(
            map_earth_search_landsat_feature(&missing).unwrap_err().code,
            "EARTH_SEARCH_LANDSAT_IDENTITY_MISMATCH"
        );
        let mut wrong_type = earth.clone();
        wrong_type["properties"]["landsat:wrs_row"] = json!({"value":"061"});
        assert_eq!(
            map_earth_search_landsat_feature(&wrong_type)
                .unwrap_err()
                .code,
            "EARTH_SEARCH_LANDSAT_IDENTITY_MISMATCH"
        );
        let mut wrong_asset = earth;
        wrong_asset["assets"]["red"]["href"] = json!(
            "s3://other-bucket/collection02/level-2/standard/LC09_L2SP_093061_20230326_20230329_02_T2_SR_B4.TIF"
        );
        assert_eq!(
            map_earth_search_landsat_feature(&wrong_asset)
                .unwrap_err()
                .code,
            "EARTH_SEARCH_LANDSAT_IDENTITY_MISMATCH"
        );
    }

    #[test]
    fn public_search_payload_preserves_filters_and_uses_official_platform_names() {
        let filters = PublicLandsatQueryFilters {
            start: Some("2023-03-26T00:00:00Z".to_owned()),
            end: Some("2023-03-27T00:00:00Z".to_owned()),
            cloud_cover_lte: 100.0,
            spacecraft: vec!["landsat_8".to_owned(), "landsat_9".to_owned()],
            processing_levels: vec!["l2sp".to_owned(), "l2sr".to_owned()],
        };
        let aoi = json!({
            "type": "Polygon",
            "coordinates": [[
                [-75.446, 36.3793],
                [-76.9196, 36.3793],
                [-76.9196, 35.5713],
                [-75.446, 35.5713],
                [-75.446, 36.3793]
            ]]
        })
        .as_object()
        .expect("AOI fixture object")
        .clone();

        let payload = build_public_landsat_search_payload(&filters, &aoi, 100)
            .expect("validated filters build a STAC search payload");
        assert_eq!(
            payload["query"]["platform"]["in"],
            json!(["landsat-8", "landsat-9"])
        );
        assert_eq!(
            payload["query"]["landsat:correction"]["in"],
            json!(["L2SP", "L2SR"])
        );
        assert_eq!(
            payload["datetime"],
            "2023-03-26T00:00:00Z/2023-03-27T00:00:00Z"
        );
        assert_eq!(payload["intersects"], Value::Object(aoi));
        assert_eq!(
            payload["collections"],
            json!([PLANETARY_COMPUTER_LANDSAT_COLLECTION_ID])
        );
        assert_eq!(payload["limit"].as_u64(), Some(100));
        assert_eq!(
            payload["fields"],
            json!({
                "include": [
                    "id",
                    "type",
                    "collection",
                    "geometry",
                    "properties",
                    "assets.red.href",
                    "assets.rendered_preview.href",
                    "assets.thumbnail.href"
                ],
                "exclude": ["links", "stac_extensions"]
            })
        );
    }

    #[test]
    fn public_search_payload_omits_datetime_when_the_user_requests_all_time() {
        let filters = PublicLandsatQueryFilters {
            start: None,
            end: None,
            cloud_cover_lte: 60.0,
            spacecraft: vec!["landsat_8".to_owned(), "landsat_9".to_owned()],
            processing_levels: vec!["l2sp".to_owned(), "l2sr".to_owned()],
        };
        let aoi = json!({
            "type": "Polygon",
            "coordinates": [[[81.16, 27.53], [92.10, 27.53], [92.10, 33.18], [81.16, 27.53]]]
        })
        .as_object()
        .expect("AOI fixture object")
        .clone();

        let payload = build_public_landsat_search_payload(&filters, &aoi, 100)
            .expect("an all-time search remains a bounded STAC request");
        assert!(payload.get("datetime").is_none());
        assert_eq!(
            payload["query"]["platform"]["in"],
            json!(["landsat-8", "landsat-9"])
        );
        assert_eq!(
            payload["query"]["landsat:correction"]["in"],
            json!(["L2SP", "L2SR"])
        );
    }

    #[test]
    fn public_landsat_query_accepts_all_time_without_relaxing_platform_or_product_type() {
        let query: CatalogSearchQuery = serde_json::from_value(json!({
            "logical_dataset": "optical.landsat.c2.l2",
            "provider_id": MPC_LANDSAT_PROVIDER_ID,
            "native_collection_id": MPC_LANDSAT_COLLECTION_ID,
            "aoi_geojson": {
                "type": "Polygon",
                "coordinates": [[[81.16, 27.53], [92.10, 27.53], [92.10, 33.18], [81.16, 27.53]]]
            },
            "cursor": null,
            "result_limit": 100,
            "filters": {
                "cloud_cover_lte": 60.0,
                "spacecraft": ["landsat_8", "landsat_9"],
                "processing_levels": ["l2sp", "l2sr"]
            }
        }))
        .expect("reviewed all-time Landsat query");

        let filters = validate_public_landsat_query(&query).expect("all-time query stays admitted");
        assert!(filters.start.is_none());
        assert!(filters.end.is_none());
        assert_eq!(filters.spacecraft, ["landsat_8", "landsat_9"]);
        assert_eq!(filters.processing_levels, ["l2sp", "l2sr"]);

        let mut valid_l7 = query.clone();
        valid_l7
            .filters
            .insert("spacecraft".to_owned(), json!(["landsat_7"]));
        assert!(validate_public_landsat_query(&valid_l7).is_ok());

        let mut invalid = query;
        invalid
            .filters
            .insert("spacecraft".to_owned(), json!(["landsat_unknown"]));
        assert!(validate_public_landsat_query(&invalid).is_err());
    }

    #[test]
    fn public_search_payload_asks_for_newest_first_only_when_dated() {
        let dated = PublicLandsatQueryFilters {
            start: Some("2023-03-01T00:00:00Z".to_owned()),
            end: Some("2023-03-31T00:00:00Z".to_owned()),
            cloud_cover_lte: 100.0,
            spacecraft: vec!["landsat_9".to_owned()],
            processing_levels: vec!["l2sp".to_owned()],
        };
        let aoi = json!({"type": "Polygon", "coordinates": [[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 0.0]]]})
            .as_object()
            .expect("AOI fixture object")
            .clone();
        let payload = build_public_landsat_search_payload(&dated, &aoi, 100).unwrap();
        assert_eq!(
            payload["sortby"],
            json!([{"field": "properties.datetime", "direction": "desc"}])
        );

        let all_time = PublicLandsatQueryFilters {
            start: None,
            end: None,
            ..dated
        };
        let payload = build_public_landsat_search_payload(&all_time, &aoi, 100).unwrap();
        assert!(payload.get("sortby").is_none());
    }

    #[test]
    fn public_search_payload_rejects_noncanonical_platform_filters() {
        let filters = PublicLandsatQueryFilters {
            start: Some("2023-03-26T00:00:00Z".to_owned()),
            end: Some("2023-03-27T00:00:00Z".to_owned()),
            cloud_cover_lte: 100.0,
            spacecraft: vec!["landsat-landsat_9".to_owned()],
            processing_levels: vec!["l2sp".to_owned()],
        };
        let aoi = json!({"type": "Polygon", "coordinates": [[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 0.0]]]})
            .as_object()
            .expect("AOI fixture object")
            .clone();
        assert_eq!(
            build_public_landsat_search_payload(&filters, &aoi, 100)
                .unwrap_err()
                .code(),
            PlanetaryComputerErrorCode::ConfigInvalid
        );
    }

    #[test]
    #[ignore = "public network"]
    fn public_stac_manifest_live_shape() {
        let client = PlanetaryComputerLandsatClient::new(ProviderNetworkPolicy::system()).unwrap();
        let manifest = client
            .manifest(
                "landsat_ot_c2_l2",
                "LC90930612023085LGN00",
                "LC09_L2SP_093061_20230326_20230329_02_T2",
                &|| false,
            )
            .unwrap();
        assert_eq!(manifest.stac_item_id, "LC09_L2SP_093061_20230326_02_T2");
        assert_eq!(manifest.assets.len(), 22);
        assert!(manifest.total_size_bytes > 0);
    }

    #[test]
    fn sas_token_retry_policy_is_bounded_to_transient_failures() {
        assert!(retryable_planetary_computer_request(
            PlanetaryComputerError::new(
                PlanetaryComputerErrorCode::TransportFailed,
                PlanetaryComputerEndpointClass::SasToken,
            ),
        ));
        for status in [408_u16, 429, 500, 502, 503, 504] {
            assert!(retryable_planetary_computer_request(
                PlanetaryComputerError {
                    code: PlanetaryComputerErrorCode::HttpRejected,
                    endpoint: PlanetaryComputerEndpointClass::SasToken,
                    http_status: Some(status),
                },
            ));
        }
        for status in [400_u16, 401, 403, 404] {
            assert!(!retryable_planetary_computer_request(
                PlanetaryComputerError {
                    code: PlanetaryComputerErrorCode::HttpRejected,
                    endpoint: PlanetaryComputerEndpointClass::SasToken,
                    http_status: Some(status),
                },
            ));
        }
        assert!(!retryable_planetary_computer_request(
            PlanetaryComputerError::new(
                PlanetaryComputerErrorCode::ResponseInvalid,
                PlanetaryComputerEndpointClass::SasToken,
            ),
        ));
    }

    #[test]
    fn public_catalogue_unknown_totals_are_not_invented_from_page_length() {
        assert_eq!(
            public_landsat_response_counts(&json!({"links":[],"numberReturned":2}), 2).unwrap(),
            (None, true)
        );
        assert_eq!(
            public_landsat_response_counts(
                &json!({"links":[{"rel":"next"}],"numberReturned":2}),
                2
            )
            .unwrap(),
            (None, false)
        );
        assert_eq!(
            public_landsat_response_counts(&json!({"links":[],"numberMatched":5}), 2).unwrap(),
            (Some(5), false)
        );
        assert_eq!(
            public_landsat_response_counts(&json!({"links":[],"numberMatched":2}), 2).unwrap(),
            (Some(2), true)
        );
        for value in [
            json!({"links":[],"numberMatched":1}),
            json!({"links":[],"numberMatched":"unknown"}),
            json!({"links":[],"numberReturned":3}),
            json!({"links":[{"rel":"next"}],"numberMatched":2}),
        ] {
            assert!(public_landsat_response_counts(&value, 2).is_err());
        }
    }

    #[test]
    fn one_byte_range_size_evidence_is_strict_and_bounded() {
        assert_eq!(
            parse_single_byte_content_range("bytes 0-0/87750834"),
            Some(87_750_834)
        );
        for invalid in [
            "bytes 0-1/87750834",
            "bytes 1-1/87750834",
            "bytes 0-0/*",
            "bytes 0-0/0",
            "bytes 0-0/4294967297",
            "bytes 0-0/1 extra",
        ] {
            assert_eq!(parse_single_byte_content_range(invalid), None, "{invalid}");
        }
    }

    #[test]
    fn landsat_asset_prefix_accepts_tm_etm_and_oli_tirs() {
        let valid_cases = [
            "https://landsateuwest.blob.core.windows.net/landsat-c2/level-2/standard/oli-tirs/2023/013/035/LC09_L2SP_013035_20230326_20230328_02_T1/LC09_L2SP_013035_20230326_20230328_02_T1_SR_B4.TIF",
            "https://landsateuwest.blob.core.windows.net/landsat-c2/level-2/standard/tm/1992/039/037/LT05_L2SP_039037_19920715_20200915_02_T1/LT05_L2SP_039037_19920715_20200915_02_T1_SR_B3.TIF",
            "https://landsateuwest.blob.core.windows.net/landsat-c2/level-2/standard/etm/2003/039/037/LE07_L2SP_039037_20030510_20200915_02_T1/LE07_L2SP_039037_20030510_20200915_02_T1_SR_B3.TIF",
        ];
        for href in valid_cases {
            assert!(
                validate_planetary_computer_landsat_stable_href(href).is_ok(),
                "Expected valid href for {href}"
            );
        }

        let invalid_cases = [
            "https://landsateuwest.blob.core.windows.net/landsat-c2/level-2/standard/mss/1975/039/037/LM01.TIF",
            "https://landsateuwest.blob.core.windows.net/landsat-c2/level-2/standard/tm/../escape.TIF",
            "https://otherhost.blob.core.windows.net/landsat-c2/level-2/standard/tm/1992/039/037/LT05.TIF",
            "https://landsateuwest.blob.core.windows.net/landsat-c2/level-2/standard/tm/file.TIF?token=secret",
        ];
        for href in invalid_cases {
            assert!(
                validate_planetary_computer_landsat_stable_href(href).is_err(),
                "Expected rejection for {href}"
            );
        }
    }

    #[test]
    fn lt04_lt05_and_le07_sensor_aware_manifests_parse_and_validate_cleanly() {
        let cases = [
            (
                "LT04_L2SP_013045_19931118_20200913_02_T2",
                "LT40130451993322XXX01",
                "landsat_tm_c2_l2",
                22usize,
                true,
            ),
            (
                "LT04_L2SR_091055_19930715_20200914_02_T2",
                "LT40910551993196XXX02",
                "landsat_tm_c2_l2",
                13usize,
                false,
            ),
            (
                "LT05_L2SP_038037_20120505_20200820_02_T1",
                "LT50380372012126EDC00",
                "landsat_tm_c2_l2",
                22usize,
                true,
            ),
            (
                "LT05_L2SR_089076_20111116_20200820_02_T2",
                "LT50890762011320ASA00",
                "landsat_tm_c2_l2",
                13usize,
                false,
            ),
            (
                "LE07_L2SP_135047_20240119_20240213_02_T1",
                "LE71350472024019EDC00",
                "landsat_etm_c2_l2",
                22usize,
                true,
            ),
            (
                "LE07_L2SR_071086_20240118_20240227_02_T2",
                "LE70710862024018ASN00",
                "landsat_etm_c2_l2",
                13usize,
                false,
            ),
        ];

        for (display_id, scene_id, usgs_alias, expected_len, has_lwir) in cases {
            let identity = ParsedProductIdentity::parse(display_id).unwrap();
            let stac_item = item(display_id, scene_id);

            let mpc_assets = parse_source_item(
                MPC_LANDSAT_COLLECTION_ID,
                &identity.stac_item_id,
                &identity,
                &stac_item,
            )
            .expect("MPC STAC item should parse for TM/ETM+");
            assert_eq!(mpc_assets.len(), expected_len, "{display_id}");
            assert_eq!(mpc_assets.first().unwrap().asset_key, "blue");
            assert!(mpc_assets
                .first()
                .unwrap()
                .file_name
                .ends_with("_SR_B1.TIF"));
            assert!(mpc_assets
                .iter()
                .any(|a| a.asset_key == "red" && a.file_name.ends_with("_SR_B3.TIF")));
            assert!(mpc_assets
                .iter()
                .any(|a| a.asset_key == "nir08" && a.file_name.ends_with("_SR_B4.TIF")));
            assert!(mpc_assets.iter().any(|a| a.asset_key == "atmos_opacity"
                && a.file_name.ends_with("_SR_ATMOS_OPACITY.TIF")));
            assert!(mpc_assets
                .iter()
                .any(|a| a.asset_key == "cloud_qa" && a.file_name.ends_with("_SR_CLOUD_QA.TIF")));
            assert_eq!(
                mpc_assets
                    .iter()
                    .any(|a| a.asset_key == "lwir" && a.file_name.ends_with("_ST_B6.TIF")),
                has_lwir
            );
            assert!(!mpc_assets.iter().any(|a| a.asset_key == "coastal"));
            assert!(!mpc_assets.iter().any(|a| a.asset_key == "lwir11"));
            assert!(!mpc_assets.iter().any(|a| a.asset_key == "qa_aerosol"));

            for discovered in &mpc_assets {
                validate_planetary_computer_landsat_asset(
                    &discovered.stable_href,
                    display_id,
                    discovered.asset_key,
                    &discovered.file_name,
                    &discovered.role,
                )
                .expect("every discovered TM/ETM+ asset must pass exact validation");
            }

            let key_set = mpc_assets
                .iter()
                .map(|a| a.asset_key)
                .collect::<HashSet<_>>();
            validate_planetary_computer_landsat_manifest_asset_keys(
                display_id,
                &identity.processing_level,
                &key_set,
            )
            .expect("manifest asset key set must pass closed validation");

            let usgs_assets = parse_source_item(usgs_alias, scene_id, &identity, &stac_item)
                .expect("USGS TM/ETM+ alias should also parse");
            assert_eq!(usgs_assets.len(), expected_len);

            // Optional `ang` omission on legacy TM/ETM+ scenes must also succeed cleanly.
            let mut without_ang = stac_item.clone();
            without_ang["assets"].as_object_mut().unwrap().remove("ang");
            let parsed_no_ang = parse_source_item(
                MPC_LANDSAT_COLLECTION_ID,
                &identity.stac_item_id,
                &identity,
                &without_ang,
            )
            .expect("TM/ETM+ scene without optional ang must parse");
            assert_eq!(parsed_no_ang.len(), expected_len - 1);
            let no_ang_keys = parsed_no_ang
                .iter()
                .map(|a| a.asset_key)
                .collect::<HashSet<_>>();
            validate_planetary_computer_landsat_manifest_asset_keys(
                display_id,
                &identity.processing_level,
                &no_ang_keys,
            )
            .expect("manifest without optional ang must pass closed validation");
        }
    }

    fn public_landsat_feature_at(row: &str, west: f64, south: f64) -> Value {
        json!({
            "type": "Feature",
            "id": format!("LC09_L2SP_093{row}_20230326_02_T2"),
            "collection": PLANETARY_COMPUTER_LANDSAT_COLLECTION_ID,
            "geometry": {"type": "Polygon", "coordinates": [[
                [west, south], [west + 1.0, south], [west + 1.0, south + 1.0],
                [west, south + 1.0], [west, south]
            ]]},
            "properties": {
                "datetime": "2023-03-26T23:59:40.503132Z",
                "platform": "landsat-9",
                "landsat:wrs_path": "093",
                "landsat:wrs_row": row,
                "landsat:scene_id": format!("LC9093{row}2023085LGN00"),
                "landsat:correction": "L2SP",
                "landsat:collection_number": "02",
                "landsat:collection_category": "T2",
                "eo:cloud_cover": 3.34
            },
            "assets": {
                "red": {
                    "href": format!("https://landsateuwest.blob.core.windows.net/landsat-c2/level-2/standard/oli-tirs/2023/093/{row}/LC09_L2SP_093{row}_20230326_20230329_02_T2/LC09_L2SP_093{row}_20230326_20230329_02_T2_SR_B4.TIF")
                }
            }
        })
    }

    fn oversized_public_landsat_query(limit: u64, dated: bool) -> CatalogSearchQuery {
        let mut filters = json!({
            "spacecraft": ["landsat_9"],
            "processing_levels": ["l2sp"]
        });
        if dated {
            filters["datetime"] =
                json!({"start": "2023-03-01T00:00:00Z", "end": "2023-04-01T00:00:00Z"});
        }
        CatalogSearchQuery {
            logical_dataset: "optical.landsat.c2.l2".to_owned(),
            provider_id: MPC_LANDSAT_PROVIDER_ID.to_owned(),
            native_collection_id: MPC_LANDSAT_COLLECTION_ID.to_owned(),
            aoi_geojson: Some(crate::catalog::spatial::dense_disjoint_china_like_aoi_for_test()),
            cursor: None,
            result_limit: Some(limit),
            filters: filters.as_object().unwrap().clone(),
        }
    }

    fn public_page(features: Vec<Value>, next: Option<Value>) -> Value {
        json!({
            "type": "FeatureCollection",
            "features": features,
            "links": next.into_iter().collect::<Vec<_>>()
        })
    }

    #[test]
    fn oversized_aoi_public_search_follows_next_until_exact_hits_fill_the_target() {
        use crate::catalog::stac_test_server::JsonPageServer;
        // Page 1 holds one exact hit and one envelope-only gap scene, so a
        // target of 2 is underfilled and the reviewed `merge: true` POST
        // continuation is followed; page 2 fills the target.
        let server = JsonPageServer::new(vec![
            public_page(
                vec![
                    public_landsat_feature_at("061", 108.0, 43.0),
                    public_landsat_feature_at("062", 121.0, 49.0),
                ],
                Some(json!({"rel": "next", "method": "POST", "merge": true,
                    "href": "/api/stac/v1/search", "body": {"token": "next:page-2"}})),
            ),
            public_page(
                vec![
                    public_landsat_feature_at("063", 98.0, 38.0),
                    public_landsat_feature_at("064", 109.0, 44.0),
                ],
                Some(json!({"rel": "next", "href": "/api/stac/v1/search?token=next:page-3"})),
            ),
        ]);
        let client = PlanetaryComputerLandsatClient::with_loopback_stac_origin(
            &server.origin,
            Duration::from_secs(10),
        )
        .unwrap();
        let response = client
            .search_catalog(&oversized_public_landsat_query(2, true), &|| false)
            .unwrap();
        assert_eq!(server.count(), 2, "stops as soon as the target is filled");
        let ids = response
            .products
            .iter()
            .map(|product| product.product_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            [
                "LC09_L2SP_093062_20230326_02_T2",
                "LC09_L2SP_093063_20230326_02_T2"
            ]
        );
        assert_eq!(response.stats.request_count, 2);
        assert_eq!(response.stats.fetched_record_count, 4);
        assert_eq!(response.stats.exact_matched_count, 2);
        assert!(!response.stats.remote_exhausted);
        assert!(response.stats.working_set_satisfied);
    }

    #[test]
    fn oversized_aoi_public_search_fails_closed_when_the_page_budget_is_spent() {
        use crate::catalog::stac_test_server::JsonPageServer;
        let pages = (0..MAX_PUBLIC_ENVELOPE_SEARCH_PAGES)
            .map(|index| {
                public_page(
                    vec![public_landsat_feature_at(
                        &format!("{:03}", 70 + index),
                        108.0,
                        43.0,
                    )],
                    Some(json!({"rel": "next",
                        "href": format!("/api/stac/v1/search?token=next:{index}")})),
                )
            })
            .collect::<Vec<_>>();
        let server = JsonPageServer::new(pages);
        let client = PlanetaryComputerLandsatClient::with_loopback_stac_origin(
            &server.origin,
            Duration::from_secs(10),
        )
        .unwrap();
        let error = client
            .search_catalog(&oversized_public_landsat_query(1, true), &|| false)
            .unwrap_err();
        assert_eq!(error.code(), PlanetaryComputerErrorCode::ResponseTooLarge);
        assert_eq!(server.count(), MAX_PUBLIC_ENVELOPE_SEARCH_PAGES);
    }

    #[test]
    fn public_search_continuation_never_widens_scope_or_leaves_the_search_endpoint() {
        use crate::catalog::stac_test_server::JsonPageServer;
        for next in [
            json!({"rel": "next", "method": "POST", "href": "/api/stac/v1/search",
                "body": {"collections": ["other"], "token": "t"}}),
            json!({"rel": "next", "method": "POST", "merge": true,
                "href": "/api/stac/v1/search?collections=other", "body": {"token": "t"}}),
            json!({"rel": "next", "href": "https://example.com/api/stac/v1/search?token=t"}),
            json!({"rel": "next", "href": "/api/stac/v1/collections?token=t"}),
        ] {
            let server = JsonPageServer::new(vec![public_page(
                vec![public_landsat_feature_at("061", 108.0, 43.0)],
                Some(next),
            )]);
            let client = PlanetaryComputerLandsatClient::with_loopback_stac_origin(
                &server.origin,
                Duration::from_secs(10),
            )
            .unwrap();
            let error = client
                .search_catalog(&oversized_public_landsat_query(1, true), &|| false)
                .unwrap_err();
            assert_eq!(error.code(), PlanetaryComputerErrorCode::ResponseInvalid);
            assert_eq!(server.count(), 1, "no continuation request was sent");
        }
    }

    #[test]
    fn oversized_aoi_public_search_follows_live_mpc_post_next_link_with_collections_query() {
        use crate::catalog::stac_test_server::JsonPageServer;
        let query = oversized_public_landsat_query(1, true);
        let filters = validate_public_landsat_query(&query).unwrap();
        let compact = crate::catalog::spatial::compact_oversized_request_aoi(
            query.aoi_geojson.as_ref().unwrap(),
        )
        .unwrap();
        let mut echoed_body =
            build_public_landsat_search_payload(&filters, &compact.request_geojson, 500).unwrap();
        echoed_body["query"]["eo:cloud_cover"]["lte"] = json!(100);
        echoed_body["token"] = json!("next:landsat-c2-l2:page-2");

        let server = JsonPageServer::new(vec![
            public_page(
                vec![public_landsat_feature_at("061", 108.0, 43.0)],
                Some(json!({
                    "rel": "next",
                    "type": "application/geo+json",
                    "method": "POST",
                    "href": "/api/stac/v1/search?collections=landsat-c2-l2",
                    "body": echoed_body
                })),
            ),
            public_page(vec![public_landsat_feature_at("062", 121.0, 49.0)], None),
        ]);
        let client = PlanetaryComputerLandsatClient::with_loopback_stac_origin(
            &server.origin,
            Duration::from_secs(10),
        )
        .unwrap();
        let response = client.search_catalog(&query, &|| false).unwrap();
        assert_eq!(server.count(), 2);
        assert_eq!(response.products.len(), 1);
        assert_eq!(
            response.products[0].product_id,
            "LC09_L2SP_093062_20230326_02_T2"
        );
        assert_eq!(response.stats.request_count, 2);
        assert!(response.stats.working_set_satisfied);
    }

    #[test]
    fn oversized_aoi_public_search_admits_only_scenes_inside_the_real_aoi() {
        use crate::catalog::stac_test_server::JsonPageServer;
        // Server order: gap (envelope only), north-east, west.
        let page = json!({
            "type": "FeatureCollection",
            "features": [
                public_landsat_feature_at("061", 108.0, 43.0),
                public_landsat_feature_at("062", 121.0, 49.0),
                public_landsat_feature_at("063", 98.0, 38.0)
            ],
            "links": []
        });
        let server = JsonPageServer::new(vec![page.clone()]);
        let client = PlanetaryComputerLandsatClient::with_loopback_stac_origin(
            &server.origin,
            Duration::from_secs(10),
        )
        .unwrap();
        let response = client
            .search_catalog(&oversized_public_landsat_query(2, false), &|| false)
            .unwrap();
        let ids = response
            .products
            .iter()
            .map(|product| product.product_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            [
                "LC09_L2SP_093062_20230326_02_T2",
                "LC09_L2SP_093063_20230326_02_T2"
            ]
        );
        assert_eq!(response.stats.fetched_record_count, 3);
        assert_eq!(response.stats.exact_matched_count, 2);
        assert_eq!(response.stats.matched_count, None);

        // Dated newest-first: the cap keeps the first exact hit in server
        // order, never the envelope-only gap scene.
        let server = JsonPageServer::new(vec![page]);
        let client = PlanetaryComputerLandsatClient::with_loopback_stac_origin(
            &server.origin,
            Duration::from_secs(10),
        )
        .unwrap();
        let response = client
            .search_catalog(&oversized_public_landsat_query(1, true), &|| false)
            .unwrap();
        assert_eq!(response.products.len(), 1);
        assert_eq!(
            response.products[0].product_id,
            "LC09_L2SP_093062_20230326_02_T2"
        );
        assert!(response.stats.working_set_satisfied);
    }

    #[test]
    fn public_landsat_search_accepts_multi_megabyte_500_feature_stac_page_within_24mb_cap() {
        use crate::catalog::stac_test_server::JsonPageServer;
        let mut page = public_page(vec![public_landsat_feature_at("062", 121.0, 49.0)], None);
        page.as_object_mut().unwrap().insert(
            "stac_padding".to_owned(),
            Value::String("x".repeat(5 * 1024 * 1024)),
        );
        let serialized_len = serde_json::to_vec(&page).unwrap().len();
        assert!(
            serialized_len > 4 * 1024 * 1024,
            "regression fixture must exceed old 4 MiB MAX_SEARCH_JSON_BYTES cap"
        );
        let server = JsonPageServer::new(vec![page]);
        let client = PlanetaryComputerLandsatClient::with_loopback_stac_origin(
            &server.origin,
            Duration::from_secs(10),
        )
        .unwrap();
        let response = client
            .search_catalog(&oversized_public_landsat_query(1, true), &|| false)
            .unwrap();
        assert_eq!(response.products.len(), 1);
        assert_eq!(
            response.products[0].product_id,
            "LC09_L2SP_093062_20230326_02_T2"
        );
    }

    #[test]
    fn public_landsat_search_filters_out_pgstac_fractional_cloud_cover_leak() {
        use crate::catalog::stac_test_server::JsonPageServer;
        let mut leaky = public_landsat_feature_at("062", 121.0, 49.0);
        leaky["id"] = json!("LC08_L2SP_093062_20230326_02_T2");
        leaky["properties"]["platform"] = json!("landsat-8");
        leaky["properties"]["landsat:scene_id"] = json!("LC80930622023085LGN00");
        leaky["properties"]["eo:cloud_cover"] = json!(20.36);
        leaky["assets"]["red"]["href"] = json!(
            "https://landsateuwest.blob.core.windows.net/landsat-c2/level-2/standard/oli-tirs/2023/093/062/LC08_L2SP_093062_20230326_20230329_02_T2/LC08_L2SP_093062_20230326_20230329_02_T2_SR_B4.TIF"
        );
        let mut valid = public_landsat_feature_at("063", 121.0, 49.0);
        valid["properties"]["eo:cloud_cover"] = json!(19.8);

        let aoi = json!({
            "type": "Polygon",
            "coordinates": [[[120.5, 48.5], [122.5, 48.5], [122.5, 50.5], [120.5, 50.5], [120.5, 48.5]]]
        })
        .as_object()
        .unwrap()
        .clone();
        let make_query = |limit: u64| CatalogSearchQuery {
            logical_dataset: "optical.landsat.c2.l2".to_owned(),
            provider_id: MPC_LANDSAT_PROVIDER_ID.to_owned(),
            native_collection_id: MPC_LANDSAT_COLLECTION_ID.to_owned(),
            aoi_geojson: Some(aoi.clone()),
            cursor: None,
            result_limit: Some(limit),
            filters: json!({
                "cloud_cover_lte": 20.0,
                "spacecraft": ["landsat_8", "landsat_9"],
                "processing_levels": ["l2sp"],
                "datetime": {"start": "2023-03-01T00:00:00Z", "end": "2023-04-01T00:00:00Z"}
            })
            .as_object()
            .unwrap()
            .clone(),
        };

        // Case 1: Single page contains a pgstac integer-bucket leak (20.36 > 20.0) plus a valid 19.8% scene.
        let server =
            JsonPageServer::new(vec![public_page(vec![leaky.clone(), valid.clone()], None)]);
        let client = PlanetaryComputerLandsatClient::with_loopback_stac_origin(
            &server.origin,
            Duration::from_secs(10),
        )
        .unwrap();
        let response = client.search_catalog(&make_query(10), &|| false).unwrap();
        assert_eq!(response.products.len(), 1);
        assert_eq!(
            response.products[0].product_id,
            "LC09_L2SP_093063_20230326_02_T2"
        );
        assert_eq!(response.stats.matched_count, None);

        // Case 2: Page 1 only has leaked 20.36% scene + rel=next; continues to page 2 to satisfy limit=1.
        let server = JsonPageServer::new(vec![
            public_page(
                vec![leaky],
                Some(json!({"rel": "next", "href": "/api/stac/v1/search?token=page-2"})),
            ),
            public_page(vec![valid], None),
        ]);
        let client = PlanetaryComputerLandsatClient::with_loopback_stac_origin(
            &server.origin,
            Duration::from_secs(10),
        )
        .unwrap();
        let response = client.search_catalog(&make_query(1), &|| false).unwrap();
        assert_eq!(server.count(), 2);
        assert_eq!(response.products.len(), 1);
        assert_eq!(
            response.products[0].product_id,
            "LC09_L2SP_093063_20230326_02_T2"
        );
    }

    #[test]
    fn public_search_continuation_preserves_fields_projection_on_merge_and_full_post() {
        let client = PlanetaryComputerLandsatClient::with_loopback_stac_origin(
            "http://127.0.0.1:9999",
            Duration::from_secs(10),
        )
        .unwrap();
        let filters = PublicLandsatQueryFilters {
            start: Some("2023-03-26T00:00:00Z".to_owned()),
            end: Some("2023-03-27T00:00:00Z".to_owned()),
            cloud_cover_lte: 100.0,
            spacecraft: vec!["landsat_8".to_owned(), "landsat_9".to_owned()],
            processing_levels: vec!["l2sp".to_owned(), "l2sr".to_owned()],
        };
        let aoi = json!({
            "type": "Polygon",
            "coordinates": [[[116.6, 30.9], [118.0, 30.9], [118.0, 32.6], [116.6, 32.6], [116.6, 30.9]]]
        })
        .as_object()
        .unwrap()
        .clone();
        let original = build_public_landsat_search_payload(&filters, &aoi, 100).unwrap();

        // (a) merge: true POST next link inherits original["fields"]
        let merged_page = public_page(
            vec![public_landsat_feature_at("062", 117.0, 31.8)],
            Some(json!({
                "rel": "next",
                "method": "POST",
                "merge": true,
                "href": "/api/stac/v1/search?collections=landsat-c2-l2",
                "body": { "token": "next:page-2" }
            })),
        );
        let PublicSearchRequest::Post {
            body: merged_body, ..
        } = client
            .public_search_continuation(&merged_page, &original)
            .expect("merge: true continuation succeeds")
        else {
            panic!("expected POST continuation");
        };
        assert_eq!(merged_body["fields"], original["fields"]);
        assert_eq!(merged_body["token"], json!("next:page-2"));

        // (b) full-body POST next link omitting "fields" still inherits original["fields"]
        let full_body_omitted = json!({
            "collections": original["collections"],
            "intersects": original["intersects"],
            "query": original["query"],
            "datetime": original["datetime"],
            "limit": 100,
            "token": "next:page-3"
        });
        let full_page = public_page(
            vec![public_landsat_feature_at("062", 117.0, 31.8)],
            Some(json!({
                "rel": "next",
                "method": "POST",
                "href": "/api/stac/v1/search?collections=landsat-c2-l2",
                "body": full_body_omitted
            })),
        );
        let PublicSearchRequest::Post {
            body: full_body, ..
        } = client
            .public_search_continuation(&full_page, &original)
            .expect("full-body POST continuation inherits omitted fields")
        else {
            panic!("expected POST continuation");
        };
        assert_eq!(full_body["fields"], original["fields"]);

        // (c) explicit "fields" on POST next link wins
        let custom_fields = json!({
            "include": ["id", "properties", "assets.red.href"],
            "exclude": ["links"]
        });
        let override_page = public_page(
            vec![public_landsat_feature_at("062", 117.0, 31.8)],
            Some(json!({
                "rel": "next",
                "method": "POST",
                "merge": true,
                "href": "/api/stac/v1/search",
                "body": { "fields": custom_fields, "token": "next:page-4" }
            })),
        );
        let PublicSearchRequest::Post {
            body: override_body,
            ..
        } = client
            .public_search_continuation(&override_page, &original)
            .expect("explicit link fields wins")
        else {
            panic!("expected POST continuation");
        };
        assert_eq!(override_body["fields"], custom_fields);
    }
}
