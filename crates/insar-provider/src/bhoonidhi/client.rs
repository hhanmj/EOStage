use std::collections::BTreeMap;
use std::io::Read;
use std::sync::Arc;
use std::time::{Duration, Instant};

use geo::MultiPolygon;
use insar_core::{
    CatalogProvider, CatalogSearchQuery, CatalogSearchResponse, ContractError, ProviderProduct,
    ProviderResult, ProxyPolicy,
};
use reqwest::blocking::{Client, Response};
use reqwest::header::{HeaderValue, AUTHORIZATION};
#[cfg(test)]
use serde_json::json;
use serde_json::{Map, Value};
use url::Url;
use zeroize::Zeroizing;

use crate::catalog::spatial::{
    authoritative_catalogue_geometry, bounded_catalogue_request_tiles, PreparedCatalogueAoi,
};

use super::auth::{
    BhoonidhiAccessTokenLease, BhoonidhiAccessTokenProvider, BhoonidhiAuthError,
    BhoonidhiRefreshResult,
};
use super::mapper::{dedupe_products, map_bhoonidhi_feature, response_from_products};
use super::query::{
    build_search_request, parse_next_link, validate_next_href, BhoonidhiSearchRequest,
    BHOONIDHI_MAX_PAGE_SIZE,
};
use super::{BHOONIDHI_SEARCH_PATH, BHOONIDHI_SEARCH_URL};

const USER_AGENT: &str = "EOStage/3 Bhoonidhi S-RSLC catalogue";
const MAX_PRODUCTION_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_JSON_BYTES: usize = 16 * 1024 * 1024;
const MAX_REQUESTS: usize = 128;
const MAX_PAGES: usize = 64;
const MAX_RESULT_RECORDS: u64 = 10_000;
const MAX_SPATIAL_PARTS: usize = 8;
const TARGET_TILE_LONGITUDE_SPAN: f64 = 10.0;
const TARGET_TILE_LATITUDE_SPAN: f64 = 10.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BhoonidhiCatalogLimits {
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
    pub max_requests: usize,
    pub max_pages: usize,
    pub max_records: u64,
    pub max_json_bytes: usize,
}

impl Default for BhoonidhiCatalogLimits {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(15),
            request_timeout: Duration::from_secs(45),
            max_requests: MAX_REQUESTS,
            max_pages: MAX_PAGES,
            max_records: MAX_RESULT_RECORDS,
            max_json_bytes: MAX_JSON_BYTES,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BhoonidhiCatalogProgress {
    pub provider_request_count: u64,
    pub transport_part_count: u64,
    pub raw_fetched_count: u64,
    pub mapped_count: u64,
    pub unique_admitted_count: u64,
    pub exact_matched_count: u64,
    pub requested_limit: Option<u64>,
    pub remote_total: Option<u64>,
    pub elapsed_ms: u64,
}

#[derive(Clone)]
pub struct BhoonidhiCatalogClient {
    client: Client,
    endpoint: Url,
    token_provider: Arc<dyn BhoonidhiAccessTokenProvider>,
    limits: BhoonidhiCatalogLimits,
}

fn error(code: &str, message: &str, reason: &str) -> ContractError {
    ContractError::new(
        code,
        message,
        Map::from_iter([("reason".to_owned(), Value::String(reason.to_owned()))]),
    )
    .expect("bounded Bhoonidhi client error is safe")
}

fn http_error(status: u16) -> ContractError {
    let (code, message) = match status {
        401 => ("BHOONIDHI_UNAUTHORIZED", "Bhoonidhi 会话已失效"),
        403 => ("BHOONIDHI_FORBIDDEN", "Bhoonidhi 访问被拒绝"),
        429 => ("BHOONIDHI_RATE_LIMITED", "Bhoonidhi 请求受到限流"),
        500..=599 => ("BHOONIDHI_SERVER_ERROR", "Bhoonidhi 服务暂时不可用"),
        _ => ("BHOONIDHI_HTTP_ERROR", "Bhoonidhi 请求失败"),
    };
    ContractError::new(
        code,
        message,
        Map::from_iter([
            ("reason".to_owned(), Value::String("http_status".to_owned())),
            ("status".to_owned(), Value::Number(status.into())),
        ]),
    )
    .expect("bounded Bhoonidhi HTTP error is safe")
}

fn exact_search_aoi(query: &CatalogSearchQuery) -> ProviderResult<Option<MultiPolygon<f64>>> {
    query
        .aoi_geojson
        .as_ref()
        .map(|geometry| {
            authoritative_catalogue_geometry(geometry).ok_or_else(|| {
                error(
                    "BHOONIDHI_QUERY_INVALID",
                    "Bhoonidhi 检索区域无效",
                    "exact_aoi",
                )
            })
        })
        .transpose()
}

/// Larger, antimeridian, or dense AOIs use bounded transport bboxes; the original
/// geometry remains the only authority for local intersection.
fn transport_queries(
    query: &CatalogSearchQuery,
    exact_aoi: Option<&MultiPolygon<f64>>,
) -> ProviderResult<Vec<CatalogSearchQuery>> {
    let Some(aoi) = exact_aoi else {
        return Ok(vec![query.clone()]);
    };
    let tiles = bounded_catalogue_request_tiles(
        aoi,
        TARGET_TILE_LONGITUDE_SPAN,
        TARGET_TILE_LATITUDE_SPAN,
        MAX_SPATIAL_PARTS,
    )
    .ok_or_else(|| {
        error(
            "BHOONIDHI_QUERY_INVALID",
            "Bhoonidhi 检索区域无法规划",
            "spatial_plan",
        )
    })?;
    if tiles.len() > 1 && query.cursor.is_some() {
        return Err(error(
            "BHOONIDHI_QUERY_INVALID",
            "Bhoonidhi 多窗口分页查询不受支持",
            "multipart_cursor",
        ));
    }
    if tiles.len() > 1
        && tiles.iter().any(|tile| {
            super::query::derive_bbox_from_geojson(&Value::Object(tile.clone()))
                .map(|bbox| {
                    bbox.east - bbox.west > TARGET_TILE_LONGITUDE_SPAN
                        || bbox.north - bbox.south > TARGET_TILE_LATITUDE_SPAN
                })
                .unwrap_or(true)
        })
    {
        return Err(error(
            "BHOONIDHI_QUERY_INVALID",
            "Bhoonidhi 检索区域超过有界切块能力，请缩小范围",
            "spatial_window_limit",
        ));
    }
    Ok(tiles
        .into_iter()
        .map(|tile| {
            let mut part = query.clone();
            part.aoi_geojson = Some(tile);
            part
        })
        .collect())
}

enum ExactAoiFilter {
    Unconstrained,
    Prepared(Option<PreparedCatalogueAoi>),
}

impl ExactAoiFilter {
    fn from_exact_aoi(exact_aoi: Option<&MultiPolygon<f64>>) -> Self {
        match exact_aoi {
            None => Self::Unconstrained,
            Some(aoi) => Self::Prepared(PreparedCatalogueAoi::new(aoi)),
        }
    }

    fn matches(&self, product: &ProviderProduct) -> ProviderResult<bool> {
        let Self::Prepared(prepared) = self else {
            return Ok(true);
        };
        let footprint = product.footprint_geojson.as_ref().ok_or_else(|| {
            error(
                "BHOONIDHI_RESPONSE_INVALID",
                "Bhoonidhi 影像缺少覆盖范围",
                "footprint_missing",
            )
        })?;
        if insar_core::validated_polygonal_geojson(footprint).is_none() {
            return Err(error(
                "BHOONIDHI_RESPONSE_INVALID",
                "Bhoonidhi 影像覆盖范围无效",
                "footprint_invalid",
            ));
        }
        Ok(prepared
            .as_ref()
            .is_some_and(|aoi| aoi.intersects_footprint(footprint)))
    }
}

#[cfg(test)]
fn matches_exact_aoi(
    product: &ProviderProduct,
    aoi: Option<&MultiPolygon<f64>>,
) -> ProviderResult<bool> {
    ExactAoiFilter::from_exact_aoi(aoi).matches(product)
}

impl BhoonidhiCatalogClient {
    /// Construct a production client without making a network request.
    pub fn new<T>(token_provider: T) -> ProviderResult<Self>
    where
        T: BhoonidhiAccessTokenProvider + 'static,
    {
        Self::build(
            Url::parse(BHOONIDHI_SEARCH_URL).expect("reviewed Bhoonidhi URL is valid"),
            Arc::new(token_provider),
            BhoonidhiCatalogLimits::default(),
            false,
            None,
        )
    }

    pub fn production<T>(token_provider: T) -> ProviderResult<Self>
    where
        T: BhoonidhiAccessTokenProvider + 'static,
    {
        Self::new(token_provider)
    }

    /// Production client on the route Bhoonidhi transfers use, so search and
    /// transfer never take different network paths.
    pub fn production_with_route<T>(token_provider: T, route: ProxyPolicy) -> ProviderResult<Self>
    where
        T: BhoonidhiAccessTokenProvider + 'static,
    {
        Self::build(
            Url::parse(BHOONIDHI_SEARCH_URL).expect("reviewed Bhoonidhi URL is valid"),
            Arc::new(token_provider),
            BhoonidhiCatalogLimits::default(),
            false,
            Some(route),
        )
    }

    /// Test-only constructor on an explicit route (loopback proxy fixtures).
    #[doc(hidden)]
    pub fn with_endpoint_and_route_for_tests<T>(
        endpoint: &str,
        token_provider: T,
        limits: BhoonidhiCatalogLimits,
        route: ProxyPolicy,
    ) -> ProviderResult<Self>
    where
        T: BhoonidhiAccessTokenProvider + 'static,
    {
        let endpoint = Url::parse(endpoint).map_err(|_| {
            error(
                "BHOONIDHI_ENDPOINT_INVALID",
                "Bhoonidhi 测试端点无效",
                "parse",
            )
        })?;
        Self::build(
            endpoint,
            Arc::new(token_provider),
            limits,
            true,
            Some(route),
        )
    }

    /// Test-only constructor. It accepts an unauthenticated HTTP loopback
    /// endpoint and still obtains the Authorization value through the same
    /// runtime-only token provider boundary.
    #[doc(hidden)]
    pub fn with_endpoint_for_tests<T>(
        endpoint: &str,
        token_provider: T,
        limits: BhoonidhiCatalogLimits,
    ) -> ProviderResult<Self>
    where
        T: BhoonidhiAccessTokenProvider + 'static,
    {
        let endpoint = Url::parse(endpoint).map_err(|_| {
            error(
                "BHOONIDHI_ENDPOINT_INVALID",
                "Bhoonidhi 测试端点无效",
                "parse",
            )
        })?;
        Self::build(endpoint, Arc::new(token_provider), limits, true, None)
    }

    fn build(
        endpoint: Url,
        token_provider: Arc<dyn BhoonidhiAccessTokenProvider>,
        limits: BhoonidhiCatalogLimits,
        test_endpoint: bool,
        route: Option<ProxyPolicy>,
    ) -> ProviderResult<Self> {
        validate_endpoint(&endpoint, test_endpoint)?;
        if limits.connect_timeout.is_zero()
            || limits.request_timeout.is_zero()
            || limits.request_timeout > MAX_PRODUCTION_TIMEOUT
            || limits.max_requests == 0
            || limits.max_requests > MAX_REQUESTS
            || limits.max_pages == 0
            || limits.max_pages > MAX_PAGES
            || limits.max_records == 0
            || limits.max_records > MAX_RESULT_RECORDS
            || limits.max_json_bytes == 0
            || limits.max_json_bytes > MAX_JSON_BYTES
        {
            return Err(error(
                "BHOONIDHI_CONFIG_INVALID",
                "Bhoonidhi 客户端配置无效",
                "limits",
            ));
        }
        let mut builder = Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(limits.connect_timeout)
            .timeout(limits.request_timeout)
            .redirect(reqwest::redirect::Policy::none())
            .http1_only()
            .retry(reqwest::retry::never());
        if test_endpoint {
            // Avoid loading macOS Keychain built-in TLS root certificates during
            // short-deadline loopback HTTP tests.
            builder = builder.tls_built_in_root_certs(false);
        }
        // Without an explicit route a production client keeps the system
        // proxy and a loopback test endpoint goes direct.
        let route = route.unwrap_or(if test_endpoint {
            ProxyPolicy::Direct
        } else {
            ProxyPolicy::System
        });
        let client = crate::network_proxy::blocking_builder_with_proxy(builder, route)
            .build()
            .map_err(|_| {
                error(
                    "BHOONIDHI_CONFIG_INVALID",
                    "无法初始化 Bhoonidhi 网络客户端",
                    "client_build",
                )
            })?;
        Ok(Self {
            client,
            endpoint,
            token_provider,
            limits,
        })
    }

    pub fn endpoint(&self) -> &str {
        self.endpoint.as_str()
    }

    pub fn search(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        self.search_with_progress(query, cancelled, &|_| {})
    }

    pub fn search_with_progress(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
        progress: &dyn Fn(BhoonidhiCatalogProgress),
    ) -> ProviderResult<CatalogSearchResponse> {
        let query = &insar_core::clamp_catalogue_search_query(query);
        let started = Instant::now();
        let exact_aoi = exact_search_aoi(query)?;
        let exact_aoi_filter = ExactAoiFilter::from_exact_aoi(exact_aoi.as_ref());
        let transport_parts = transport_queries(query, exact_aoi.as_ref())?;
        let transport_parts = split_bhoonidhi_transport_date_windows(&transport_parts)?;
        let target = query.result_limit;
        let mut request_count = 0_u64;
        let mut page_count = 0_usize;
        let mut fetched_record_count = 0_u64;
        let mut mapped_count = 0_u64;
        let mut exact_duplicate_record_count = 0_u64;
        let mut remote_total = Some(0_u64);
        let mut ordered_ids = Vec::new();
        let mut products = BTreeMap::new();
        let mut remote_exhausted = true;
        let mut attempted_parts = 0_usize;
        for part in &transport_parts {
            let mut request = build_search_request(part)?;
            let mut part_remote_total = None;
            let part_exhausted = loop {
                self.check_cancelled(cancelled)?;
                if request_count >= self.limits.max_requests as u64
                    || page_count >= self.limits.max_pages
                {
                    return Err(error(
                        "BHOONIDHI_SEARCH_BUDGET_EXCEEDED",
                        "Bhoonidhi 检索超过安全分页预算",
                        "page_budget",
                    ));
                }
                let page = self.fetch_page(&request, cancelled)?;
                request_count += 1;
                page_count += 1;
                let features = page
                    .get("features")
                    .and_then(Value::as_array)
                    .ok_or_else(|| {
                        error(
                            "BHOONIDHI_RESPONSE_INVALID",
                            "Bhoonidhi 响应无效",
                            "features",
                        )
                    })?;
                fetched_record_count = fetched_record_count
                    .checked_add(features.len() as u64)
                    .ok_or_else(|| {
                        error(
                            "BHOONIDHI_SEARCH_BUDGET_EXCEEDED",
                            "Bhoonidhi 检索超过安全预算",
                            "record_overflow",
                        )
                    })?;
                if fetched_record_count > self.limits.max_records {
                    return Err(error(
                        "BHOONIDHI_SEARCH_BUDGET_EXCEEDED",
                        "Bhoonidhi 检索超过安全记录预算",
                        "record_budget",
                    ));
                }
                if features.len() as u64 > BHOONIDHI_MAX_PAGE_SIZE {
                    return Err(error(
                        "BHOONIDHI_RESPONSE_INVALID",
                        "Bhoonidhi 响应无效",
                        "page_size",
                    ));
                }
                let page_total =
                    page.get("context")
                        .and_then(Value::as_object)
                        .and_then(|context| {
                            ["matched", "total", "matched_count", "numberMatched"]
                                .iter()
                                .find_map(|key| context.get(*key).and_then(Value::as_u64))
                        });
                if part_remote_total
                    .is_some_and(|previous| page_total.is_some_and(|current| previous != current))
                {
                    return Err(error(
                        "BHOONIDHI_RESPONSE_INVALID",
                        "Bhoonidhi 响应无效",
                        "matched_count_drift",
                    ));
                }
                if page_total.is_some() {
                    part_remote_total = page_total;
                }
                for feature in features {
                    self.check_cancelled(cancelled)?;
                    let product = map_bhoonidhi_feature(feature)?;
                    mapped_count += 1;
                    let exact_match = exact_aoi_filter.matches(&product)?;
                    match products.get(&product.product_id) {
                        Some(existing) if existing == &product => exact_duplicate_record_count += 1,
                        Some(_) => {
                            return Err(error(
                                "BHOONIDHI_PRODUCT_IDENTITY_CONFLICT",
                                "Bhoonidhi 返回了冲突的产品身份",
                                "product_identity",
                            ));
                        }
                        None => {
                            if exact_match {
                                ordered_ids.push(product.product_id.clone());
                            }
                            products.insert(product.product_id.clone(), product);
                        }
                    }
                }
                let exact_matched_count = ordered_ids.len() as u64;
                progress(BhoonidhiCatalogProgress {
                    provider_request_count: request_count,
                    transport_part_count: (attempted_parts + 1) as u64,
                    raw_fetched_count: fetched_record_count,
                    mapped_count,
                    unique_admitted_count: target
                        .map_or(exact_matched_count, |limit| exact_matched_count.min(limit)),
                    exact_matched_count,
                    requested_limit: target,
                    remote_total: remote_total.and_then(|total| {
                        part_remote_total.and_then(|count| total.checked_add(count))
                    }),
                    elapsed_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
                });

                let page_cursor = parse_next_link(&page, request.fingerprint())?;
                if target.is_some_and(|target| exact_matched_count >= target) {
                    // The requested working set is complete even when the archive
                    // has more items. Keep archive exhaustion as a separate fact;
                    // only search_page exposes a continuation to its caller.
                    break page_cursor.is_none();
                }
                let Some(cursor) = page_cursor else {
                    break true;
                };
                if features.is_empty() {
                    return Err(error(
                        "BHOONIDHI_CURSOR_INVALID",
                        "Bhoonidhi 续页标识无效",
                        "empty_page",
                    ));
                }
                let mut continued = part.clone();
                continued.cursor = Some(cursor);
                request = build_search_request(&continued)?;
            };
            attempted_parts += 1;
            remote_exhausted &= part_exhausted;
            remote_total = match (remote_total, part_remote_total) {
                (Some(total), Some(part_count)) => {
                    Some(total.checked_add(part_count).ok_or_else(|| {
                        error(
                            "BHOONIDHI_RESPONSE_INVALID",
                            "Bhoonidhi 目录计数无效",
                            "total_overflow",
                        )
                    })?)
                }
                _ => None,
            };
            if target.is_some_and(|target| ordered_ids.len() as u64 >= target) {
                break;
            }
        }
        remote_exhausted &= attempted_parts == transport_parts.len();

        let products = ordered_ids
            .into_iter()
            .filter_map(|id| products.remove(&id))
            .collect::<Vec<_>>();
        response_from_products(
            products,
            target,
            remote_total,
            request_count,
            attempted_parts as u64,
            fetched_record_count,
            exact_duplicate_record_count,
            None,
            remote_exhausted,
            started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
        )
    }

    pub fn search_page(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        self.search_page_with_progress(query, cancelled, &|_| {})
    }

    pub fn search_page_with_progress(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
        progress: &dyn Fn(BhoonidhiCatalogProgress),
    ) -> ProviderResult<CatalogSearchResponse> {
        let query = &insar_core::clamp_catalogue_search_query(query);
        let started = Instant::now();
        let exact_aoi = exact_search_aoi(query)?;
        let exact_aoi_filter = ExactAoiFilter::from_exact_aoi(exact_aoi.as_ref());
        let transport_parts = transport_queries(query, exact_aoi.as_ref())?;
        if transport_parts.len() != 1 {
            return Err(error(
                "BHOONIDHI_QUERY_INVALID",
                "Bhoonidhi 跨经线分页查询不受支持",
                "multipart_page",
            ));
        }
        let request = build_search_request(&transport_parts[0])?;
        let page = self.fetch_page(&request, cancelled)?;
        let features = page
            .get("features")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                error(
                    "BHOONIDHI_RESPONSE_INVALID",
                    "Bhoonidhi 响应无效",
                    "features",
                )
            })?;
        if features.len() as u64 > BHOONIDHI_MAX_PAGE_SIZE {
            return Err(error(
                "BHOONIDHI_RESPONSE_INVALID",
                "Bhoonidhi 响应无效",
                "page_size",
            ));
        }
        if features.len() as u64 > self.limits.max_records {
            return Err(error(
                "BHOONIDHI_SEARCH_BUDGET_EXCEEDED",
                "Bhoonidhi 检索超过安全记录预算",
                "record_budget",
            ));
        }
        let mut mapped = Vec::with_capacity(features.len());
        for feature in features {
            self.check_cancelled(cancelled)?;
            mapped.push(map_bhoonidhi_feature(feature)?);
        }
        let mapped_record_count = mapped.len() as u64;
        let (mapped, exact_duplicate_record_count) = dedupe_products(mapped)?;
        let mapped = mapped
            .into_iter()
            .map(|product| {
                exact_aoi_filter
                    .matches(&product)
                    .map(|matched| (matched, product))
            })
            .collect::<ProviderResult<Vec<_>>>()?
            .into_iter()
            .filter_map(|(matched, product)| matched.then_some(product))
            .collect::<Vec<_>>();
        let unique_count = mapped.len() as u64;
        let next_cursor = parse_next_link(&page, request.fingerprint())?;
        let remote_total = page
            .get("context")
            .and_then(Value::as_object)
            .and_then(|context| {
                ["matched", "total", "matched_count", "numberMatched"]
                    .iter()
                    .find_map(|key| context.get(*key).and_then(Value::as_u64))
            });
        progress(BhoonidhiCatalogProgress {
            provider_request_count: 1,
            transport_part_count: 1,
            raw_fetched_count: mapped_record_count,
            mapped_count: mapped_record_count,
            unique_admitted_count: unique_count,
            exact_matched_count: unique_count,
            requested_limit: query.result_limit,
            remote_total,
            elapsed_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
        });
        response_from_products(
            mapped,
            query.result_limit,
            remote_total,
            1,
            1,
            mapped_record_count,
            exact_duplicate_record_count,
            next_cursor.clone(),
            next_cursor.is_none(),
            started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
        )
    }

    fn fetch_page(
        &self,
        request: &BhoonidhiSearchRequest,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        self.check_cancelled(cancelled)?;
        let post_url = if let Some(continuation) = request.continuation() {
            validate_next_href(&continuation.href, &self.endpoint)?;
            if continuation.method != "POST" {
                return Err(error(
                    "BHOONIDHI_CURSOR_INVALID",
                    "Bhoonidhi 续页标识无效",
                    "next_method",
                ));
            }
            Url::parse(&continuation.href).map_err(|_| {
                error(
                    "BHOONIDHI_CURSOR_INVALID",
                    "Bhoonidhi 续页标识无效",
                    "next_href_parse",
                )
            })?
        } else {
            self.endpoint.clone()
        };
        let mut body = request.body().clone();
        if request.continuation().is_some() {
            // Bhoonidhi's STAC next-link serializer embeds a server-internal
            // `fields` exclusion object that triggers an unhandled HTTP 500 in
            // `/Bhoonidhi-DataSearch-1.0/v1/search` when echoed back.
            body.remove("fields");
            if let Some(filter) = body.get_mut("filter") {
                dedup_continuation_filter(filter);
            }
        }
        let token = self
            .token_provider
            .access_token_with_generation()
            .map_err(auth_error)?;
        let mut response = match self.send_page(&post_url, &body, &token) {
            Ok(resp) => resp,
            Err(first_err) if self.endpoint.scheme() == "https" => {
                let mut recovered = None;
                let mut last_err = first_err;
                for _ in 0..2 {
                    self.check_cancelled(cancelled)?;
                    std::thread::sleep(Duration::from_millis(600));
                    self.check_cancelled(cancelled)?;
                    match self.send_page(&post_url, &body, &token) {
                        Ok(resp) => {
                            recovered = Some(resp);
                            break;
                        }
                        Err(err) => last_err = err,
                    }
                }
                match recovered {
                    Some(resp) => resp,
                    None => return Err(last_err),
                }
            }
            Err(err) => return Err(err),
        };
        self.check_cancelled(cancelled)?;
        for _ in 0..2 {
            if !(self.endpoint.scheme() == "https"
                && (500..=599).contains(&response.status().as_u16()))
            {
                break;
            }
            drop(response);
            std::thread::sleep(Duration::from_millis(600));
            self.check_cancelled(cancelled)?;
            response = self.send_page(&post_url, &body, &token)?;
            self.check_cancelled(cancelled)?;
        }

        // A catalog 401 may refresh the exact generation used for this
        // request once.  A second 401 is terminal, and providers that do not
        // own a refreshable process session retain the old error behavior.
        if response.status().as_u16() == 401 {
            drop(response);
            let refresh = self
                .token_provider
                .refresh_after_rejection(token.generation())
                .map_err(auth_error)?;
            if matches!(refresh, BhoonidhiRefreshResult::Unsupported) {
                return Err(http_error(401));
            }
            self.check_cancelled(cancelled)?;
            let replacement = self
                .token_provider
                .access_token_with_generation()
                .map_err(auth_error)?;
            response = self.send_page(&post_url, &body, &replacement)?;
            self.check_cancelled(cancelled)?;
        }

        let status = response.status().as_u16();
        if status != 200 {
            if status == 404 {
                if let Ok(bytes) = read_bounded(response, self.limits.max_json_bytes, cancelled) {
                    if is_bhoonidhi_no_results_found(&bytes) {
                        return Ok(serde_json::json!({
                            "type": "FeatureCollection",
                            "features": [],
                            "links": []
                        }));
                    }
                }
            }
            return Err(http_error(status));
        }
        let bytes = read_bounded(response, self.limits.max_json_bytes, cancelled)?;
        serde_json::from_slice(&bytes).map_err(|_| {
            error(
                "BHOONIDHI_RESPONSE_INVALID",
                "Bhoonidhi 返回的 JSON 无效",
                "json",
            )
        })
    }

    fn send_page(
        &self,
        post_url: &Url,
        body: &Map<String, Value>,
        token: &BhoonidhiAccessTokenLease,
    ) -> ProviderResult<Response> {
        let mut bearer = Zeroizing::new(String::from("Bearer "));
        bearer.push_str(token.as_str());
        let mut authorization = HeaderValue::from_str(bearer.as_str()).map_err(|_| {
            error(
                "BHOONIDHI_AUTH_UNAVAILABLE",
                "Bhoonidhi 认证不可用",
                "token_invalid",
            )
        })?;
        authorization.set_sensitive(true);
        self.client
            .post(post_url.clone())
            .header(AUTHORIZATION, authorization)
            .header(reqwest::header::ACCEPT, "application/json")
            .json(&Value::Object(body.clone()))
            .send()
            .map_err(|_| {
                error(
                    "BHOONIDHI_NETWORK_ERROR",
                    "Bhoonidhi 网络请求失败",
                    "transport",
                )
            })
    }

    fn check_cancelled(&self, cancelled: &(dyn Fn() -> bool + Sync)) -> ProviderResult<()> {
        if cancelled() {
            Err(error(
                "BHOONIDHI_SEARCH_CANCELLED",
                "Bhoonidhi 检索已取消",
                "cancelled",
            ))
        } else {
            Ok(())
        }
    }
}

const BHOONIDHI_MAX_DATE_WINDOW_DAYS: i64 = 360;
const BHOONIDHI_MAX_TEMPORAL_PARTS: usize = 48;

fn parse_bhoonidhi_datetime_filter(
    value: &Value,
) -> Option<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)> {
    match value {
        Value::Object(map) => {
            let start = map.get("start").and_then(Value::as_str)?;
            let end = map.get("end").and_then(Value::as_str)?;
            let start = chrono::DateTime::parse_from_rfc3339(start)
                .ok()?
                .with_timezone(&chrono::Utc);
            let end = chrono::DateTime::parse_from_rfc3339(end)
                .ok()?
                .with_timezone(&chrono::Utc);
            Some((start, end))
        }
        Value::String(raw) => {
            let (start, end) = raw.split_once('/')?;
            let start = chrono::DateTime::parse_from_rfc3339(start.trim())
                .ok()?
                .with_timezone(&chrono::Utc);
            let end = chrono::DateTime::parse_from_rfc3339(end.trim())
                .ok()?
                .with_timezone(&chrono::Utc);
            Some((start, end))
        }
        _ => None,
    }
}

/// Bhoonidhi `/data/search` enforces a hard 366-day ceiling (HTTP 406) on a
/// single datetime range. Split longer queries into contiguous <=360-day windows
/// ordered newest-first so result limits surface the most recent scenes first.
fn split_bhoonidhi_transport_date_windows(
    parts: &[CatalogSearchQuery],
) -> ProviderResult<Vec<CatalogSearchQuery>> {
    let max_span = chrono::Duration::days(BHOONIDHI_MAX_DATE_WINDOW_DAYS);
    let mut expanded = Vec::with_capacity(parts.len());
    for part in parts {
        let Some((start, end)) = part
            .filters
            .get("datetime")
            .and_then(parse_bhoonidhi_datetime_filter)
        else {
            expanded.push(part.clone());
            continue;
        };
        if end <= start || (end - start) <= max_span {
            expanded.push(part.clone());
            continue;
        }
        if part.cursor.is_some() {
            return Err(error(
                "BHOONIDHI_QUERY_INVALID",
                "Bhoonidhi 多窗口分页查询不受支持",
                "multipart_cursor",
            ));
        }
        let mut windows = Vec::new();
        let mut window_end = end;
        while window_end > start && windows.len() < BHOONIDHI_MAX_TEMPORAL_PARTS {
            let candidate_start = window_end - max_span;
            let window_start = if candidate_start < start {
                start
            } else {
                candidate_start
            };
            let mut slice = part.clone();
            slice.filters.insert(
                "datetime".to_owned(),
                serde_json::json!({
                    "start": window_start.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                    "end": window_end.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                }),
            );
            windows.push(slice);
            if window_start <= start {
                break;
            }
            window_end = window_start;
        }
        expanded.extend(windows);
    }
    Ok(expanded)
}

fn is_bhoonidhi_no_results_found(bytes: &[u8]) -> bool {
    let Ok(Value::Object(map)) = serde_json::from_slice::<Value>(bytes) else {
        return false;
    };
    let code_ok = match map.get("ErrorCode") {
        Some(Value::String(code)) => code.trim() == "404",
        Some(Value::Number(code)) => code.as_u64() == Some(404),
        None | Some(Value::Null) => true,
        _ => false,
    };
    let desc_ok = map
        .get("Description")
        .or_else(|| map.get("description"))
        .or_else(|| map.get("message"))
        .and_then(Value::as_str)
        .is_some_and(|desc| {
            let normalized = desc.trim().to_ascii_lowercase();
            normalized == "no results found" || normalized == "no data found"
        });
    code_ok && desc_ok
}

fn collect_flat_and_clauses(value: &Value, out: &mut Vec<Value>) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    let Some(op) = object.get("op").and_then(Value::as_str) else {
        return false;
    };
    let Some(args) = object.get("args").and_then(Value::as_array) else {
        return false;
    };
    if op == "and" {
        for arg in args {
            if !collect_flat_and_clauses(arg, out) {
                return false;
            }
        }
        true
    } else {
        out.push(value.clone());
        true
    }
}

fn dedup_continuation_filter(filter: &mut Value) {
    let mut clauses = Vec::new();
    if !collect_flat_and_clauses(filter, &mut clauses) || clauses.is_empty() {
        return;
    }
    let mut deduped = Vec::with_capacity(clauses.len());
    for clause in clauses {
        if !deduped.contains(&clause) {
            deduped.push(clause);
        }
    }
    *filter = serde_json::json!({
        "op": "and",
        "args": deduped,
    });
}

impl CatalogProvider for BhoonidhiCatalogClient {
    fn collections(&self) -> ProviderResult<Vec<insar_core::CatalogCollectionDescriptor>> {
        Ok(vec![super::mapper::bhoonidhi_collection()])
    }

    fn search_catalog(&self, query: &CatalogSearchQuery) -> ProviderResult<CatalogSearchResponse> {
        self.search(query, &|| false)
    }
}

fn auth_error(error: BhoonidhiAuthError) -> ContractError {
    super::error(error.code(), error.user_message(), error.reason())
}

fn validate_endpoint(endpoint: &Url, test_endpoint: bool) -> ProviderResult<()> {
    if endpoint.path() != BHOONIDHI_SEARCH_PATH
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
    {
        return Err(error(
            "BHOONIDHI_ENDPOINT_INVALID",
            "Bhoonidhi 端点无效",
            "endpoint",
        ));
    }
    if test_endpoint {
        let loopback = endpoint
            .host_str()
            .and_then(|host| host.parse::<std::net::IpAddr>().ok())
            .is_some_and(|address| address.is_loopback());
        if endpoint.scheme() != "http" || !loopback {
            return Err(error(
                "BHOONIDHI_ENDPOINT_INVALID",
                "Bhoonidhi 测试端点无效",
                "loopback",
            ));
        }
    } else if endpoint.scheme() != "https"
        || endpoint.host_str() != Some("bhoonidhi-api.nrsc.gov.in")
        || endpoint.port_or_known_default() != Some(443)
    {
        return Err(error(
            "BHOONIDHI_ENDPOINT_INVALID",
            "Bhoonidhi 生产端点无效",
            "origin",
        ));
    }
    Ok(())
}

fn read_bounded(
    mut response: Response,
    limit: usize,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> ProviderResult<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        if cancelled() {
            return Err(error(
                "BHOONIDHI_SEARCH_CANCELLED",
                "Bhoonidhi 检索已取消",
                "cancelled",
            ));
        }
        let count = response.read(&mut buffer).map_err(|_| {
            error(
                "BHOONIDHI_NETWORK_ERROR",
                "Bhoonidhi 网络响应读取失败",
                "read",
            )
        })?;
        if count == 0 {
            break;
        }
        if bytes.len().saturating_add(count) > limit {
            return Err(error(
                "BHOONIDHI_RESPONSE_TOO_LARGE",
                "Bhoonidhi 响应超过安全大小",
                "response_bytes",
            ));
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bhoonidhi::fixtures::CATALOGUE_FEATURE_JSON;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn multipolygon_exact_filter_paginates_past_bbox_only_products() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}/data/search", listener.local_addr().unwrap());
        let query = CatalogSearchQuery {
            logical_dataset: super::super::BHOONIDHI_LOGICAL_DATASET.to_owned(),
            provider_id: super::super::BHOONIDHI_PROVIDER_ID.to_owned(),
            native_collection_id: super::super::BHOONIDHI_COLLECTION_ID.to_owned(),
            aoi_geojson: Some(
                serde_json::json!({
                    "type":"MultiPolygon",
                    "coordinates":[
                        [[[75.5,5.65],[75.75,5.65],[75.75,5.85],[75.5,5.85],[75.5,5.65]]],
                        [[[76.8,5.95],[77.0,5.95],[77.0,6.05],[76.8,6.05],[76.8,5.95]]]
                    ]
                })
                .as_object()
                .unwrap()
                .clone(),
            ),
            filters: Map::new(),
            cursor: None,
            result_limit: Some(2),
        };
        let expected_bbox = build_search_request(&query).unwrap().body()["bbox"].clone();
        let fixture_endpoint = endpoint.clone();
        let server = std::thread::spawn(move || {
            let mut observed = Vec::new();
            for page in 0..3 {
                let deadline = Instant::now() + Duration::from_secs(10);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(Instant::now() < deadline, "expected three catalogue pages");
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        Err(error) => panic!("local catalogue accept failed: {error}"),
                    }
                };
                // On Windows the accepted socket can inherit the listener's
                // nonblocking mode; only accept polling should be nonblocking.
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let body = {
                    let mut reader = BufReader::new(&mut stream);
                    let mut content_length = 0;
                    loop {
                        let mut header = String::new();
                        reader.read_line(&mut header).unwrap();
                        assert!(!header.is_empty(), "HTTP request headers must end");
                        if header == "\r\n" {
                            break;
                        }
                        if let Some((name, value)) = header.split_once(':') {
                            if name.eq_ignore_ascii_case("content-length") {
                                content_length = value.trim().parse::<usize>().unwrap();
                            }
                        }
                    }
                    assert!(content_length > 0 && content_length < 8192);
                    let mut body = vec![0; content_length];
                    reader.read_exact(&mut body).unwrap();
                    serde_json::from_slice::<Value>(&body).unwrap()
                };
                assert_eq!(body["bbox"], expected_bbox);
                assert_eq!(body["collections"], serde_json::json!(["NISAR_SSAR_RSLC"]));
                assert!(
                    body.get("fields").is_none(),
                    "continuation requests must strip server-internal fields exclusion"
                );
                if page == 0 {
                    assert!(body.get("token").is_none());
                } else {
                    assert_eq!(body["token"], format!("page-{}", page + 1));
                }
                observed.push(body);
                let mut feature: Value = serde_json::from_str(CATALOGUE_FEATURE_JSON).unwrap();
                let identity = feature["id"]
                    .as_str()
                    .unwrap()
                    .replace("_001", &format!("_00{}", page + 1));
                feature["id"] = serde_json::json!(identity);
                feature["properties"]["ID"] = feature["id"].clone();
                if page == 0 {
                    // The first footprint is in the bbox gap, outside both AOI islands.
                    feature["geometry"] = serde_json::json!({
                        "type":"Polygon",
                        "coordinates":[[[76.0,5.7],[76.2,5.7],[76.2,5.8],[76.0,5.8],[76.0,5.7]]]
                    });
                }
                let links = if page < 2 {
                    let mut next_body = observed[0].as_object().unwrap().clone();
                    next_body.insert("fields".to_owned(), serde_json::json!({"exclude": ["assets.data", "properties.MetaData", "properties.IsDeleted"]}));
                    next_body.insert(
                        "token".to_owned(),
                        serde_json::json!(format!("page-{}", page + 2)),
                    );
                    vec![serde_json::json!({
                        "rel":"next", "href":fixture_endpoint, "method":"POST", "body":next_body
                    })]
                } else {
                    vec![]
                };
                let response = serde_json::json!({
                    "type":"FeatureCollection", "features":[feature], "links":links,
                    "context":{"matched":3}
                })
                .to_string();
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", response.len(), response).unwrap();
                stream.flush().unwrap();
            }
            observed
        });
        let client = BhoonidhiCatalogClient::with_endpoint_for_tests(
            &endpoint,
            TokenProvider {
                calls: Arc::new(AtomicUsize::new(0)),
            },
            BhoonidhiCatalogLimits::default(),
        )
        .unwrap();
        let result = client.search(&query, &|| false).unwrap();
        let observed = server.join().unwrap();
        assert_eq!(
            observed.len(),
            3,
            "bbox-only first page must not satisfy the working set"
        );
        assert_eq!(result.products.len(), 2);
        assert_eq!(result.stats.request_count, 3);
        assert_eq!(result.stats.fetched_record_count, 3);
        assert_eq!(result.stats.unique_provider_product_count, 3);
        assert_eq!(result.stats.exact_matched_count, 2);
        assert_eq!(
            result.stats.matched_count,
            Some(3),
            "provider total remains a remote count"
        );
        assert!(result.stats.remote_exhausted);
        assert!(result.next_cursor.is_none());
    }

    #[test]
    fn antimeridian_two_http_parts_share_budget_and_merge_exact_results() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}/data/search", listener.local_addr().unwrap());
        let query = CatalogSearchQuery {
            logical_dataset: super::super::BHOONIDHI_LOGICAL_DATASET.to_owned(),
            provider_id: super::super::BHOONIDHI_PROVIDER_ID.to_owned(),
            native_collection_id: super::super::BHOONIDHI_COLLECTION_ID.to_owned(),
            aoi_geojson: Some(json!({
                "type":"Polygon",
                "coordinates":[[[179.0,10.0],[-179.0,10.0],[-179.0,11.0],[179.0,11.0],[179.0,10.0]]]
            }).as_object().unwrap().clone()),
            filters: Map::new(),
            cursor: None,
            result_limit: Some(2),
        };
        let server = std::thread::spawn(move || {
            let mut observed = Vec::new();
            for index in 0..2 {
                let deadline = Instant::now() + Duration::from_secs(10);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(Instant::now() < deadline, "expected two transport windows");
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        Err(error) => panic!("local catalogue accept failed: {error}"),
                    }
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let request = {
                    let mut reader = BufReader::new(&mut stream);
                    let mut content_length = 0;
                    loop {
                        let mut line = String::new();
                        reader.read_line(&mut line).unwrap();
                        assert!(!line.is_empty(), "HTTP request headers must end");
                        if line == "\r\n" {
                            break;
                        }
                        if let Some((name, value)) = line.split_once(':') {
                            if name.eq_ignore_ascii_case("content-length") {
                                content_length = value.trim().parse::<usize>().unwrap();
                            }
                        }
                    }
                    assert!(content_length > 0 && content_length < 8192);
                    let mut body = vec![0; content_length];
                    reader.read_exact(&mut body).unwrap();
                    serde_json::from_slice::<Value>(&body).unwrap()
                };
                assert!(request.get("token").is_none());
                assert_eq!(request["collections"], json!(["NISAR_SSAR_RSLC"]));
                let bbox = request["bbox"].as_array().unwrap();
                let west = bbox[0].as_f64().unwrap();
                let east = bbox[2].as_f64().unwrap();
                assert!(east > west && east - west <= 2.0);
                let longitude = if west < 0.0 { -179.5 } else { 179.5 };
                observed.push((west, east));
                let mut feature: Value = serde_json::from_str(CATALOGUE_FEATURE_JSON).unwrap();
                let identity = feature["id"]
                    .as_str()
                    .unwrap()
                    .replace("_001", &format!("_00{}", index + 1));
                feature["id"] = json!(identity);
                feature["properties"]["ID"] = feature["id"].clone();
                feature["geometry"] = json!({
                    "type":"Polygon",
                    "coordinates":[[
                        [longitude - 0.1,10.2],[longitude + 0.1,10.2],
                        [longitude + 0.1,10.4],[longitude - 0.1,10.4],
                        [longitude - 0.1,10.2]
                    ]]
                });
                let response = json!({
                    "type":"FeatureCollection", "features":[feature],
                    "links":[], "context":{"matched":1}
                })
                .to_string();
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",response.len(),response).unwrap();
                stream.flush().unwrap();
            }
            observed
        });
        let client = BhoonidhiCatalogClient::with_endpoint_for_tests(
            &endpoint,
            TokenProvider {
                calls: Arc::new(AtomicUsize::new(0)),
            },
            BhoonidhiCatalogLimits::default(),
        )
        .unwrap();
        let result = client.search(&query, &|| false).unwrap();
        let windows = server.join().unwrap();
        assert_eq!(windows.len(), 2);
        assert!(windows.iter().any(|(west, _)| *west == -180.0));
        assert!(windows.iter().any(|(_, east)| *east == 180.0));
        assert_eq!(result.products.len(), 2);
        assert_eq!(result.stats.transport_part_count, 2);
        assert_eq!(result.stats.request_count, 2);
        assert_eq!(result.stats.matched_count, Some(2));
        assert_eq!(result.stats.exact_matched_count, 2);
        assert_eq!(result.stats.unique_provider_product_count, 2);
        assert!(result.stats.remote_exhausted);
    }

    #[test]
    fn antimeridian_aoi_uses_two_bounded_transport_bboxes() {
        let query = CatalogSearchQuery {
            logical_dataset: super::super::BHOONIDHI_LOGICAL_DATASET.to_owned(),
            provider_id: super::super::BHOONIDHI_PROVIDER_ID.to_owned(),
            native_collection_id: super::super::BHOONIDHI_COLLECTION_ID.to_owned(),
            aoi_geojson: Some(json!({
                "type":"Polygon",
                "coordinates":[[[179.0,10.0],[-179.0,10.0],[-179.0,11.0],[179.0,11.0],[179.0,10.0]]]
            }).as_object().unwrap().clone()),
            filters: Map::new(),
            cursor: None,
            result_limit: Some(2),
        };
        let aoi = exact_search_aoi(&query).unwrap().unwrap();
        let parts = transport_queries(&query, Some(&aoi)).unwrap();
        assert_eq!(parts.len(), 2);
        let mut spans = Vec::new();
        for part in parts {
            let request = build_search_request(&part).unwrap();
            let bbox = request.body()["bbox"].as_array().unwrap();
            let west = bbox[0].as_f64().unwrap();
            let east = bbox[2].as_f64().unwrap();
            assert!(east - west > 0.0 && east - west <= 2.0);
            assert!(west >= -180.0 && east <= 180.0);
            spans.push((west, east));
        }
        assert!(spans.iter().any(|(west, _)| *west == -180.0));
        assert!(spans.iter().any(|(_, east)| *east == 180.0));
        let client = BhoonidhiCatalogClient::with_endpoint_for_tests(
            "http://127.0.0.1:65433/data/search",
            TokenProvider {
                calls: Arc::new(AtomicUsize::new(0)),
            },
            BhoonidhiCatalogLimits::default(),
        )
        .unwrap();
        assert_eq!(
            client.search_page(&query, &|| false).unwrap_err().code,
            "BHOONIDHI_QUERY_INVALID",
        );
    }

    #[test]
    fn oversized_aoi_is_split_into_bounded_rectangles_without_changing_exact_authority() {
        let query = CatalogSearchQuery {
            logical_dataset: super::super::BHOONIDHI_LOGICAL_DATASET.to_owned(),
            provider_id: super::super::BHOONIDHI_PROVIDER_ID.to_owned(),
            native_collection_id: super::super::BHOONIDHI_COLLECTION_ID.to_owned(),
            aoi_geojson: Some(json!({
                "type": "Polygon",
                "coordinates": [[[70.0, 20.0], [110.0, 20.0], [110.0, 40.0], [70.0, 40.0], [70.0, 20.0]]]
            }).as_object().unwrap().clone()),
            filters: Map::new(),
            cursor: None,
            result_limit: Some(250),
        };
        let aoi = exact_search_aoi(&query).unwrap().unwrap();
        let parts = transport_queries(&query, Some(&aoi)).unwrap();
        assert!(
            parts.len() > 1,
            "area beyond a single bounded transport rectangle must be tiled"
        );
        assert!(parts.len() <= 8);
        for part in &parts {
            let bbox = build_search_request(part).unwrap().body()["bbox"]
                .as_array()
                .unwrap()
                .clone();
            assert!(bbox[2].as_f64().unwrap() - bbox[0].as_f64().unwrap() <= 10.0);
            assert!(bbox[3].as_f64().unwrap() - bbox[1].as_f64().unwrap() <= 10.0);
        }
        assert_eq!(query.aoi_geojson.as_ref().unwrap()["type"], "Polygon");
    }

    #[test]
    fn aoi_exceeding_bounded_tiling_fails_before_network_without_partial_results() {
        let query = CatalogSearchQuery {
            logical_dataset: super::super::BHOONIDHI_LOGICAL_DATASET.to_owned(),
            provider_id: super::super::BHOONIDHI_PROVIDER_ID.to_owned(),
            native_collection_id: super::super::BHOONIDHI_COLLECTION_ID.to_owned(),
            aoi_geojson: Some(json!({
                "type":"Polygon",
                "coordinates":[[[-80.0,-60.0],[80.0,-60.0],[80.0,60.0],[-80.0,60.0],[-80.0,-60.0]]]
            }).as_object().unwrap().clone()),
            filters: Map::new(),
            cursor: None,
            result_limit: Some(250),
        };
        let client = BhoonidhiCatalogClient::with_endpoint_for_tests(
            "http://127.0.0.1:65433/data/search",
            TokenProvider {
                calls: Arc::new(AtomicUsize::new(0)),
            },
            BhoonidhiCatalogLimits::default(),
        )
        .unwrap();
        let error = client.search(&query, &|| false).unwrap_err();
        assert_eq!(error.code, "BHOONIDHI_QUERY_INVALID");
        assert_eq!(error.debug_context["reason"], "spatial_window_limit");
    }

    #[test]
    fn upstream_http_errors_keep_status_without_relabeling_as_no_results() {
        for (status, expected) in [
            (404, "BHOONIDHI_HTTP_ERROR"),
            (503, "BHOONIDHI_SERVER_ERROR"),
        ] {
            let error = http_error(status);
            assert_eq!(error.code, expected);
            assert_eq!(error.debug_context["status"], status);
            assert_eq!(error.debug_context["reason"], "http_status");
        }
    }

    #[test]
    fn bbox_candidates_are_admitted_only_if_footprint_overlaps_true_aoi() {
        let feature: Value = serde_json::from_str(CATALOGUE_FEATURE_JSON).unwrap();
        let product = map_bhoonidhi_feature(&feature).unwrap();
        // Both AOIs are strictly within the provider footprint's bounding box.
        // The first is above the skewed radar footprint, so bbox-only admission
        // would incorrectly include the scene.
        let outside = serde_json::json!({
            "type":"Polygon",
            "coordinates":[[[76.8,5.95],[77.0,5.95],[77.0,6.05],[76.8,6.05],[76.8,5.95]]]
        });
        let inside = serde_json::json!({
            "type":"Polygon",
            "coordinates":[[[75.6,5.7],[75.7,5.7],[75.7,5.8],[75.6,5.8],[75.6,5.7]]]
        });
        let footprint_box = product.footprint_bbox.as_ref().unwrap();
        let outside_box = super::super::derive_bbox_from_geojson(&outside).unwrap();
        assert!(outside_box.west > footprint_box.west && outside_box.east < footprint_box.east);
        assert!(outside_box.south > footprint_box.south && outside_box.north < footprint_box.north);
        let outside_aoi = authoritative_catalogue_geometry(outside.as_object().unwrap()).unwrap();
        let inside_aoi = authoritative_catalogue_geometry(inside.as_object().unwrap()).unwrap();
        assert!(!matches_exact_aoi(&product, Some(&outside_aoi)).unwrap());
        assert!(matches_exact_aoi(&product, Some(&inside_aoi)).unwrap());
        assert!(matches_exact_aoi(&product, None).unwrap());
    }

    #[test]
    fn invalid_true_aoi_cannot_silently_fall_back_to_bbox_filter() {
        let query = CatalogSearchQuery {
            logical_dataset: super::super::BHOONIDHI_LOGICAL_DATASET.to_owned(),
            provider_id: super::super::BHOONIDHI_PROVIDER_ID.to_owned(),
            native_collection_id: super::super::BHOONIDHI_COLLECTION_ID.to_owned(),
            aoi_geojson: Some(
                serde_json::json!({"type":"Point","coordinates":[76.0,5.7]})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
            filters: Map::new(),
            cursor: None,
            result_limit: Some(1),
        };
        let error = exact_search_aoi(&query).unwrap_err();
        assert_eq!(error.code, "BHOONIDHI_QUERY_INVALID");
    }

    struct TokenProvider {
        calls: Arc<AtomicUsize>,
    }

    impl BhoonidhiAccessTokenProvider for TokenProvider {
        fn access_token(&self) -> Result<zeroize::Zeroizing<String>, BhoonidhiAuthError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(zeroize::Zeroizing::new("offline-token".to_owned()))
        }
    }

    #[test]
    fn hunan_404_remains_typed_http_error() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/data/search", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("one Hunan catalogue request");
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let body = {
                let mut reader = BufReader::new(&mut stream);
                let mut request_line = String::new();
                reader.read_line(&mut request_line).unwrap();
                assert_eq!(request_line, "POST /data/search HTTP/1.1\r\n");
                let mut content_length = 0;
                let mut authorization_seen = false;
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).unwrap();
                    assert!(!header.is_empty(), "HTTP request headers must end");
                    if header == "\r\n" {
                        break;
                    }
                    if let Some((name, value)) = header.split_once(':') {
                        if name.eq_ignore_ascii_case("content-length") {
                            content_length = value.trim().parse::<usize>().unwrap();
                        } else if name.eq_ignore_ascii_case("authorization") {
                            authorization_seen = true;
                            assert_eq!(value.trim(), "Bearer offline-token");
                        }
                    }
                }
                assert!(authorization_seen);
                assert!(content_length > 0 && content_length < 8192);
                let mut body = vec![0; content_length];
                reader.read_exact(&mut body).unwrap();
                serde_json::from_slice::<Value>(&body).unwrap()
            };
            assert_eq!(
                body["bbox"],
                json!([
                    108.78801079615435,
                    24.639537509750618,
                    114.25488646849476,
                    30.128649782609237
                ])
            );
            assert_eq!(body["collections"], json!(["NISAR_SSAR_RSLC"]));
            assert_eq!(
                body["datetime"],
                "2026-07-04T00:00:00Z/2026-10-04T23:59:59Z"
            );
            assert_eq!(body["limit"], 250);
            assert_eq!(
                body["filter"],
                json!({
                    "op": "and",
                    "args": [
                        {"op": "eq", "args": [{"property": "Online"}, "Y"]},
                        {"op": "eq", "args": [{"property": "Mode"}, "W"]},
                        {"op": "eq", "args": [{"property": "SoftwareVersion"}, "1.4.00"]}
                    ]
                })
            );

            let response_body = "fixture body must remain private";
            write!(
                stream,
                "HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            )
            .unwrap();
            stream.flush().unwrap();
        });
        let query: CatalogSearchQuery = serde_json::from_value(json!({
            "logical_dataset": super::super::BHOONIDHI_LOGICAL_DATASET,
            "provider_id": super::super::BHOONIDHI_PROVIDER_ID,
            "native_collection_id": super::super::BHOONIDHI_COLLECTION_ID,
            "aoi_geojson": {
                "type": "Polygon",
                "coordinates": [[
                    [108.78801079615435, 24.639537509750618],
                    [114.25488646849476, 24.639537509750618],
                    [114.25488646849476, 30.128649782609237],
                    [108.78801079615435, 30.128649782609237],
                    [108.78801079615435, 24.639537509750618]
                ]]
            },
            "result_limit": 250,
            "filters": {
                "datetime": {
                    "start": "2026-07-04T00:00:00Z",
                    "end": "2026-10-04T23:59:59Z"
                },
                "mode": "W",
                "software_version": "1.4.00"
            }
        }))
        .expect("Hunan query fixture");
        let client = BhoonidhiCatalogClient::with_endpoint_for_tests(
            &endpoint,
            TokenProvider {
                calls: Arc::new(AtomicUsize::new(0)),
            },
            BhoonidhiCatalogLimits::default(),
        )
        .unwrap();

        let failure = client.search(&query, &|| false).unwrap_err();
        server.join().unwrap();
        assert_eq!(failure.code, "BHOONIDHI_HTTP_ERROR");
        assert_eq!(failure.debug_context["reason"], "http_status");
        assert_eq!(failure.debug_context["status"], 404);
        let rendered = serde_json::to_string(&failure.debug_context).unwrap();
        for forbidden in [
            "offline-token",
            "authorization",
            "127.0.0.1",
            "fixture body must remain private",
        ] {
            assert!(!rendered.to_ascii_lowercase().contains(forbidden));
        }
    }

    #[test]
    fn authentication_failures_keep_fixed_codes_and_non_sensitive_reasons() {
        let cases = [
            (
                BhoonidhiAuthError::Missing,
                "BHOONIDHI_CREDENTIALS_MISSING",
                "credentials_missing",
            ),
            (
                BhoonidhiAuthError::NotActivated,
                "BHOONIDHI_ACTIVATION_REQUIRED",
                "api_not_activated",
            ),
            (
                BhoonidhiAuthError::CredentialInvalid,
                "BHOONIDHI_CREDENTIALS_INVALID",
                "credentials_invalid",
            ),
            (
                BhoonidhiAuthError::Rejected,
                "BHOONIDHI_AUTH_REJECTED",
                "auth_rejected",
            ),
            (
                BhoonidhiAuthError::MaxSessions,
                "BHOONIDHI_MAX_SESSIONS",
                "max_sessions",
            ),
            (
                BhoonidhiAuthError::RateLimited,
                "BHOONIDHI_RATE_LIMITED",
                "rate_limited",
            ),
            (
                BhoonidhiAuthError::Transport,
                "BHOONIDHI_AUTH_TRANSPORT_FAILED",
                "auth_transport",
            ),
            (
                BhoonidhiAuthError::Timeout,
                "BHOONIDHI_AUTH_TIMEOUT",
                "auth_timeout",
            ),
            (
                BhoonidhiAuthError::ResponseInvalid,
                "BHOONIDHI_AUTH_RESPONSE_INVALID",
                "auth_response_invalid",
            ),
            (
                BhoonidhiAuthError::RefreshUnavailable,
                "BHOONIDHI_REFRESH_UNAVAILABLE",
                "refresh_unavailable",
            ),
            (
                BhoonidhiAuthError::RefreshExhausted,
                "BHOONIDHI_REFRESH_EXHAUSTED",
                "refresh_exhausted",
            ),
            (
                BhoonidhiAuthError::SessionChanged,
                "BHOONIDHI_SESSION_CHANGED",
                "session_changed",
            ),
            (
                BhoonidhiAuthError::SessionUnavailable,
                "BHOONIDHI_SESSION_UNAVAILABLE",
                "session_unavailable",
            ),
            (
                BhoonidhiAuthError::Synchronization,
                "BHOONIDHI_SYNCHRONIZATION_FAILED",
                "synchronization",
            ),
            (
                BhoonidhiAuthError::Configuration,
                "BHOONIDHI_OAUTH_CONFIGURATION_INVALID",
                "oauth_configuration",
            ),
            (
                BhoonidhiAuthError::Unavailable,
                "BHOONIDHI_AUTH_UNAVAILABLE",
                "auth_unavailable",
            ),
            (
                BhoonidhiAuthError::InvalidToken,
                "BHOONIDHI_AUTH_UNAVAILABLE",
                "token_invalid",
            ),
        ];
        for (category, code, reason) in cases {
            let error = auth_error(category);
            assert_eq!(error.code, code);
            assert_eq!(
                error.debug_context.get("reason").and_then(Value::as_str),
                Some(reason)
            );
            assert!(!error.user_message.contains("fixture"));
        }
    }

    /// A single credential-free request through the actual production client.
    /// Keep this opt-in and distinct from login, catalogue and download acceptance.
    #[test]
    #[ignore = "requires EOSTAGE_BHOONIDHI_PUBLIC_PROBE=1; one fixed-origin anonymous GET, no saved credentials"]
    fn anonymous_catalogue_request_reaches_auth_boundary() {
        assert_eq!(
            std::env::var("EOSTAGE_BHOONIDHI_PUBLIC_PROBE").as_deref(),
            Ok("1")
        );
        let calls = Arc::new(AtomicUsize::new(0));
        let client = BhoonidhiCatalogClient::new(TokenProvider {
            calls: Arc::clone(&calls),
        })
        .unwrap();
        let started = Instant::now();
        let response = client.client.get(client.endpoint.clone()).send();
        let status = response.as_ref().ok().map(|value| value.status().as_u16());
        let failure = response.as_ref().err().map(|cause| {
            if cause.is_timeout() {
                "timeout"
            } else if cause.is_connect() {
                "connect"
            } else {
                "transport"
            }
        });
        println!(
            "{}",
            serde_json::json!({
                "phase": "anonymous_native_catalogue_probe", "network_route": "system",
                "http_status": status, "failure_category": failure,
                "elapsed_ms": started.elapsed().as_millis(),
                "credential_reads": calls.load(Ordering::SeqCst), "authentication_attempted": false,
                "downloads_started": false
            })
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(status, Some(401), "anonymous production request did not reach the expected authentication boundary; see the safe probe record");
    }

    #[test]
    fn a_manual_route_carries_catalogue_requests_through_the_proxy() {
        // Nothing listens on the endpoint: only the proxy route can answer.
        let closed = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/data/search", closed.local_addr().unwrap());
        drop(closed);
        let (proxy, seen) = crate::network_proxy::loopback::serve_once(
            "HTTP/1.1 404 Not Found
Content-Length: 0
Connection: close

",
        );
        let route = ProxyPolicy::Manual(
            insar_core::ManualProxy::new(&proxy.ip().to_string(), proxy.port()).unwrap(),
        );
        let query: CatalogSearchQuery = serde_json::from_value(json!({
            "logical_dataset": super::super::BHOONIDHI_LOGICAL_DATASET,
            "provider_id": super::super::BHOONIDHI_PROVIDER_ID,
            "native_collection_id": super::super::BHOONIDHI_COLLECTION_ID,
            "aoi_geojson": {
                "type": "Polygon",
                "coordinates": [[[110.0, 25.0], [111.0, 25.0], [111.0, 26.0], [110.0, 26.0], [110.0, 25.0]]]
            },
            "result_limit": 250,
            "filters": {
                "datetime": {"start": "2026-07-04T00:00:00Z", "end": "2026-10-04T23:59:59Z"},
                "mode": "W",
                "software_version": "1.4.00"
            }
        }))
        .expect("query fixture");
        let client = BhoonidhiCatalogClient::with_endpoint_and_route_for_tests(
            &endpoint,
            TokenProvider {
                calls: Arc::new(AtomicUsize::new(0)),
            },
            BhoonidhiCatalogLimits::default(),
            route,
        )
        .unwrap();
        let _ = client.search(&query, &|| false);
        let request_line = seen
            .join()
            .unwrap()
            .expect("search reached the manual proxy");
        assert!(
            request_line.starts_with(&format!("POST {endpoint}")),
            "{request_line}"
        );
    }

    #[test]
    fn production_constructor_is_side_effect_free_and_endpoint_is_exact() {
        let calls = Arc::new(AtomicUsize::new(0));
        let client = BhoonidhiCatalogClient::new(TokenProvider {
            calls: Arc::clone(&calls),
        })
        .expect("production client");
        assert_eq!(client.endpoint(), BHOONIDHI_SEARCH_URL);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn invalid_test_endpoint_and_limits_fail_closed() {
        let calls = Arc::new(AtomicUsize::new(0));
        let invalid = BhoonidhiCatalogClient::with_endpoint_for_tests(
            "https://example.invalid/data/search",
            TokenProvider {
                calls: Arc::clone(&calls),
            },
            BhoonidhiCatalogLimits::default(),
        );
        let invalid_error = match invalid {
            Ok(_) => panic!("invalid endpoint unexpectedly succeeded"),
            Err(error) => error,
        };
        assert_eq!(invalid_error.code, "BHOONIDHI_ENDPOINT_INVALID");
        let limits = BhoonidhiCatalogLimits {
            max_records: 0,
            ..BhoonidhiCatalogLimits::default()
        };
        let invalid = BhoonidhiCatalogClient::with_endpoint_for_tests(
            "http://127.0.0.1:1/data/search",
            TokenProvider { calls },
            limits,
        );
        let invalid_error = match invalid {
            Ok(_) => panic!("invalid limits unexpectedly succeeded"),
            Err(error) => error,
        };
        assert_eq!(invalid_error.code, "BHOONIDHI_CONFIG_INVALID");
    }

    /// #22 B slice 3(a): a loopback catalogue that serves `available`
    /// generated items in 500-record pages; the last page keeps a next link
    /// unless the catalogue is `exhausted`. Returns the request count.
    fn serve_generated_pages(
        available: usize,
        exhausted: bool,
    ) -> (String, std::thread::JoinHandle<usize>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/data/search", listener.local_addr().unwrap());
        let fixture_endpoint = endpoint.clone();
        let template: Value = serde_json::from_str(CATALOGUE_FEATURE_JSON).unwrap();
        let pages = available.div_ceil(500);
        let server = std::thread::spawn(move || {
            let mut first_body = None::<Value>;
            let mut served = 0;
            for page in 0..pages {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(10)))
                    .unwrap();
                let body = {
                    let mut reader = BufReader::new(&mut stream);
                    let mut content_length = 0;
                    loop {
                        let mut header = String::new();
                        if reader.read_line(&mut header).unwrap_or(0) == 0 {
                            return served;
                        }
                        if header == "\r\n" {
                            break;
                        }
                        if let Some((name, value)) = header.split_once(':') {
                            if name.eq_ignore_ascii_case("content-length") {
                                content_length = value.trim().parse::<usize>().unwrap();
                            }
                        }
                    }
                    let mut body = vec![0; content_length];
                    reader.read_exact(&mut body).unwrap();
                    serde_json::from_slice::<Value>(&body).unwrap()
                };
                served += 1;
                let first = first_body.get_or_insert(body).clone();
                let features = (page * 500..((page + 1) * 500).min(available))
                    .map(|index| {
                        let mut feature = template.clone();
                        let id = format!("NISAR_GENERATED_RSLC_{index:05}");
                        feature["id"] = serde_json::json!(id);
                        feature["properties"]["ID"] = serde_json::json!(id);
                        feature
                    })
                    .collect::<Vec<_>>();
                let last = page + 1 == pages;
                let links = if !last || !exhausted {
                    let mut next_body = first.as_object().unwrap().clone();
                    next_body.insert(
                        "token".to_owned(),
                        serde_json::json!(format!("page-{}", page + 2)),
                    );
                    vec![serde_json::json!({
                        "rel":"next", "href":fixture_endpoint, "method":"POST", "body":next_body
                    })]
                } else {
                    vec![]
                };
                let response = serde_json::json!({
                    "type":"FeatureCollection", "features":features, "links":links,
                    "context":{"matched":available}
                })
                .to_string();
                if write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", response.len(), response).is_err() {
                    break;
                }
                let _ = stream.flush();
            }
            served
        });
        (endpoint, server)
    }

    fn generated_query(result_limit: Option<u64>) -> CatalogSearchQuery {
        CatalogSearchQuery {
            logical_dataset: super::super::BHOONIDHI_LOGICAL_DATASET.to_owned(),
            provider_id: super::super::BHOONIDHI_PROVIDER_ID.to_owned(),
            native_collection_id: super::super::BHOONIDHI_COLLECTION_ID.to_owned(),
            aoi_geojson: Some(
                serde_json::json!({"type":"Polygon","coordinates":[[[74.5,5.0],[77.5,5.0],[77.5,6.5],[74.5,6.5],[74.5,5.0]]]})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
            filters: Map::new(),
            cursor: None,
            result_limit,
        }
    }

    fn generated_client(endpoint: &str) -> BhoonidhiCatalogClient {
        BhoonidhiCatalogClient::with_endpoint_for_tests(
            endpoint,
            TokenProvider {
                calls: Arc::new(AtomicUsize::new(0)),
            },
            BhoonidhiCatalogLimits::default(),
        )
        .unwrap()
    }

    #[test]
    fn ten_thousand_target_keeps_n_without_proving_exhaustion() {
        for requested in [10_000, 10_001] {
            let (endpoint, server) = serve_generated_pages(10_001, true);
            let result = generated_client(&endpoint)
                .search(&generated_query(Some(requested)), &|| false)
                .unwrap();
            drop(std::net::TcpStream::connect(
                endpoint
                    .trim_start_matches("http://")
                    .trim_end_matches("/data/search"),
            ));
            assert_eq!(
                server.join().unwrap(),
                20,
                "the 21st page is never requested"
            );
            assert_eq!(result.stats.requested_limit, Some(10_000));
            assert_eq!(result.products.len(), 10_000);
            assert!(!result.stats.remote_exhausted);
        }
        let (endpoint, server) = serve_generated_pages(10_000, true);
        let result = generated_client(&endpoint)
            .search(&generated_query(Some(10_000)), &|| false)
            .unwrap();
        assert_eq!(server.join().unwrap(), 20);
        assert_eq!(result.products.len(), 10_000);
        assert!(result.stats.remote_exhausted);
        // An explicit target above the ceiling is fingerprinted as it.
        assert_eq!(
            build_search_request(&generated_query(Some(10_001)))
                .unwrap()
                .fingerprint(),
            build_search_request(&generated_query(Some(10_000)))
                .unwrap()
                .fingerprint()
        );
    }

    #[test]
    fn bhoonidhi_long_date_range_splits_into_bounded_windows_and_guards_cursor() {
        let mut long_query = generated_query(Some(50));
        long_query.filters.insert(
            "datetime".to_owned(),
            serde_json::json!({
                "start": "2024-01-01T00:00:00Z",
                "end": "2026-01-20T00:00:00Z"
            }),
        );
        let windows = split_bhoonidhi_transport_date_windows(&[long_query]).unwrap();
        assert_eq!(
            windows.len(),
            3,
            "750-day range should split into 3 windows <=360d"
        );
        // Ordered newest-first so result_limit prioritizes recent scenes
        assert_eq!(
            windows[0].filters["datetime"]["end"],
            serde_json::json!("2026-01-20T00:00:00Z")
        );
        assert_eq!(
            windows[2].filters["datetime"]["start"],
            serde_json::json!("2024-01-01T00:00:00Z")
        );

        let mut short_query = generated_query(Some(50));
        short_query.cursor = Some("cursor-1".to_owned());
        short_query.filters.insert(
            "datetime".to_owned(),
            serde_json::json!("2025-01-01T00:00:00Z/2025-06-01T00:00:00Z"),
        );
        let short_windows = split_bhoonidhi_transport_date_windows(&[short_query]).unwrap();
        assert_eq!(short_windows.len(), 1);

        let mut long_cursor_query = generated_query(Some(50));
        long_cursor_query.cursor = Some("cursor-2".to_owned());
        long_cursor_query.filters.insert(
            "datetime".to_owned(),
            serde_json::json!({
                "start": "2024-01-01T00:00:00Z",
                "end": "2026-01-20T00:00:00Z"
            }),
        );
        let err = split_bhoonidhi_transport_date_windows(&[long_cursor_query]).unwrap_err();
        assert_eq!(err.code, "BHOONIDHI_QUERY_INVALID");
        assert_eq!(err.debug_context["reason"], "multipart_cursor");
    }

    #[test]
    fn bhoonidhi_no_results_found_matches_404_variants_without_masking_real_http_404() {
        assert!(is_bhoonidhi_no_results_found(
            br#"{"ErrorCode":"404","Description":"NO RESULTS FOUND"}"#
        ));
        assert!(is_bhoonidhi_no_results_found(
            br#"{"ErrorCode":404,"description":"no data found"}"#
        ));
        assert!(is_bhoonidhi_no_results_found(
            br#"{"message":"No Results Found"}"#
        ));
        assert!(!is_bhoonidhi_no_results_found(
            br#"{"ErrorCode":"404","Description":"Endpoint not found"}"#
        ));
    }

    #[test]
    fn bhoonidhi_search_splits_long_date_range_and_sends_accept_json() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let mut seen_requests = Vec::new();
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buf = [0u8; 8192];
                let n = stream.read(&mut buf).unwrap();
                let raw = String::from_utf8_lossy(&buf[..n]).to_string();
                seen_requests.push(raw);
                let body = r#"{"type":"FeatureCollection","features":[],"links":[],"context":{"matched":0}}"#;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream.write_all(resp.as_bytes()).unwrap();
            }
            seen_requests
        });

        let endpoint = format!("http://{addr}/data/search");
        let mut query = generated_query(Some(10));
        query.filters.insert(
            "datetime".to_owned(),
            serde_json::json!({
                "start": "2024-06-01T00:00:00Z",
                "end": "2026-01-01T00:00:00Z"
            }),
        );
        let result = generated_client(&endpoint)
            .search(&query, &|| false)
            .unwrap();
        assert!(result.products.is_empty());
        let requests = handle.join().unwrap();
        assert_eq!(
            requests.len(),
            2,
            "579-day range splits into 2 live requests"
        );
        assert!(
            requests.iter().all(|raw| raw
                .to_ascii_lowercase()
                .contains("accept: application/json")),
            "send_page must send Accept: application/json"
        );
    }

    fn dense_l_shaped_admin_polygon_geojson() -> Map<String, Value> {
        // Concave L-shape inside [75.0, 18.0, 76.0, 19.0] (not tiled: 1° x 1°),
        // omitting notch [75.4, 76.0] x [18.4, 19.0], with > BHOONIDHI_MAX_AOI_POSITIONS vertices.
        let segments = super::super::query::BHOONIDHI_MAX_AOI_POSITIONS + 640;
        let mut ring = Vec::with_capacity(segments + 6);
        for i in 0..=segments {
            let x = 75.0 + (i as f64) / (segments as f64);
            ring.push(json!([x, 18.0]));
        }
        ring.push(json!([76.0, 18.4]));
        ring.push(json!([75.4, 18.4]));
        ring.push(json!([75.4, 19.0]));
        ring.push(json!([75.0, 19.0]));
        ring.push(json!([75.0, 18.0]));
        json!({
            "type": "Polygon",
            "coordinates": [ring]
        })
        .as_object()
        .unwrap()
        .clone()
    }

    fn bhoonidhi_feature_for_rect(id: &str, west: f64, south: f64, east: f64, north: f64) -> Value {
        let mut feature: Value = serde_json::from_str(CATALOGUE_FEATURE_JSON).unwrap();
        feature["id"] = json!(id);
        feature["bbox"] = json!([west, north, east, south]);
        feature["geometry"] = json!({
            "type": "Polygon",
            "coordinates": [[
                [west, south],
                [east, south],
                [east, north],
                [west, north],
                [west, south]
            ]]
        });
        feature["properties"]["ID"] = json!(id);
        let latlon = json!([
            [west.to_string(), north.to_string()],
            [east.to_string(), north.to_string()],
            [east.to_string(), south.to_string()],
            [west.to_string(), south.to_string()],
            [west.to_string(), north.to_string()]
        ]);
        feature["properties"]["ImgLatLon"] = latlon.clone();
        feature["properties"]["ProdLatLon"] = latlon;
        feature
    }

    fn read_http_post_json(stream: &mut std::net::TcpStream) -> Value {
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut reader = BufReader::new(stream);
        let mut content_length = 0usize;
        loop {
            let mut header = String::new();
            reader.read_line(&mut header).unwrap();
            assert!(!header.is_empty(), "HTTP request headers must terminate");
            if header == "\r\n" {
                break;
            }
            if let Some((name, value)) = header.split_once(':') {
                if name.eq_ignore_ascii_case("content-length") {
                    content_length = value.trim().parse::<usize>().unwrap();
                }
            }
        }
        assert!(
            content_length > 0 && content_length < 8192,
            "Bhoonidhi search body must send compact bbox instead of raw vertices (got {content_length} bytes)"
        );
        let mut body = vec![0u8; content_length];
        reader.read_exact(&mut body).unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    #[test]
    fn dense_single_window_admin_polygon_sends_bbox_and_filters_by_exact_aoi() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/data/search", listener.local_addr().unwrap());
        let inside_feature =
            bhoonidhi_feature_for_rect("NISAR_IN_EXACT_L", 75.05, 18.5, 75.35, 18.8);
        let notch_feature =
            bhoonidhi_feature_for_rect("NISAR_BBOX_NOTCH_ONLY", 75.55, 18.55, 75.85, 18.85);
        let payload = json!({
            "type": "FeatureCollection",
            "features": [inside_feature, notch_feature],
            "links": [],
            "context": { "matched": 2 }
        })
        .to_string();

        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let req_body = read_http_post_json(&mut stream);
            assert_eq!(req_body["bbox"], json!([75.0, 18.0, 76.0, 19.0]));
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                payload.len(),
                payload
            );
            stream.write_all(resp.as_bytes()).unwrap();
            req_body
        });

        let query = CatalogSearchQuery {
            logical_dataset: super::super::BHOONIDHI_LOGICAL_DATASET.to_owned(),
            provider_id: super::super::BHOONIDHI_PROVIDER_ID.to_owned(),
            native_collection_id: super::super::BHOONIDHI_COLLECTION_ID.to_owned(),
            aoi_geojson: Some(dense_l_shaped_admin_polygon_geojson()),
            filters: Map::new(),
            cursor: None,
            result_limit: Some(10),
        };
        let result = generated_client(&endpoint)
            .search(&query, &|| false)
            .unwrap();
        let captured = server.join().unwrap();
        assert_eq!(captured["bbox"], json!([75.0, 18.0, 76.0, 19.0]));
        assert_eq!(result.stats.fetched_record_count, 2);
        assert_eq!(result.products.len(), 1);
        assert_eq!(result.products[0].product_id, "NISAR_IN_EXACT_L");
    }

    #[test]
    fn dense_polygon_with_multi_year_range_splits_dates_and_preserves_exact_aoi() {
        let mut long_query = CatalogSearchQuery {
            logical_dataset: super::super::BHOONIDHI_LOGICAL_DATASET.to_owned(),
            provider_id: super::super::BHOONIDHI_PROVIDER_ID.to_owned(),
            native_collection_id: super::super::BHOONIDHI_COLLECTION_ID.to_owned(),
            aoi_geojson: Some(dense_l_shaped_admin_polygon_geojson()),
            filters: Map::from_iter([(
                "datetime".to_owned(),
                json!({
                    "start": "2023-11-25T00:00:00Z",
                    "end": "2026-11-25T23:59:59Z"
                }),
            )]),
            cursor: None,
            result_limit: Some(10),
        };

        let expected_windows = split_bhoonidhi_transport_date_windows(&[CatalogSearchQuery {
            aoi_geojson: None,
            ..long_query.clone()
        }])
        .unwrap()
        .len();
        assert!(
            expected_windows >= 3,
            "3-year range must split across multiple <=366d windows"
        );

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/data/search", listener.local_addr().unwrap());
        let total_expected_requests = expected_windows + 1; // multi-year search windows + 1 search_page call
        let server = std::thread::spawn(move || {
            let mut datetimes = Vec::new();
            for req_idx in 0..total_expected_requests {
                let (mut stream, _) = listener.accept().unwrap();
                let req_body = read_http_post_json(&mut stream);
                assert_eq!(req_body["bbox"], json!([75.0, 18.0, 76.0, 19.0]));
                datetimes.push(req_body["datetime"].as_str().unwrap_or_default().to_owned());
                let features = if req_idx == 0 || req_idx == 1 || req_idx == expected_windows {
                    vec![
                        bhoonidhi_feature_for_rect("NISAR_IN_EXACT_L", 75.05, 18.5, 75.35, 18.8),
                        bhoonidhi_feature_for_rect(
                            "NISAR_BBOX_NOTCH_ONLY",
                            75.55,
                            18.55,
                            75.85,
                            18.85,
                        ),
                    ]
                } else {
                    Vec::new()
                };
                let body = json!({
                    "type": "FeatureCollection",
                    "features": features,
                    "links": [],
                    "context": { "matched": if req_idx == 0 || req_idx == 1 || req_idx == expected_windows { 2 } else { 0 } }
                })
                .to_string();
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream.write_all(resp.as_bytes()).unwrap();
            }
            datetimes
        });

        let client = generated_client(&endpoint);
        let search_result = client.search(&long_query, &|| false).unwrap();
        assert_eq!(search_result.stats.request_count as usize, expected_windows);
        assert_eq!(search_result.products.len(), 1);
        assert_eq!(search_result.products[0].product_id, "NISAR_IN_EXACT_L");

        // Also verify search_page handles dense >16k-vertex single-window polygon cleanly.
        long_query.filters.insert(
            "datetime".to_owned(),
            json!({
                "start": "2026-01-01T00:00:00Z",
                "end": "2026-11-25T23:59:59Z"
            }),
        );
        let page_result = client.search_page(&long_query, &|| false).unwrap();
        assert_eq!(page_result.products.len(), 1);
        assert_eq!(page_result.products[0].product_id, "NISAR_IN_EXACT_L");

        let datetimes = server.join().unwrap();
        assert_eq!(datetimes.len(), total_expected_requests);
    }

    #[test]
    fn dense_coastal_multipolygon_exceeding_projected_topology_budget_succeeds_with_exact_aoi() {
        // Construct a single <=10°x10° coastal MultiPolygon inside [73.0, 16.0, 80.5, 22.0]
        // whose mainland boundary (~3,500 edges) plus 30 coastal islands inside the bay
        // [73.05, 74.4] x [16.05, 17.4] exceeds MAX_PROJECTED_TOPOLOGY_WORK (250_000),
        // mimicking OSM coastal states like Maharashtra.
        let mut mainland_ring = Vec::with_capacity(3_600);
        let top_segments = 3_500usize;
        for i in 0..=top_segments {
            let x = 73.0 + 7.5 * (i as f64) / (top_segments as f64);
            let y = if i == 0 || i == top_segments {
                22.0
            } else {
                22.0 - 0.001 * ((i % 2) as f64)
            };
            mainland_ring.push(json!([x, y]));
        }
        mainland_ring.push(json!([80.5, 16.0]));
        mainland_ring.push(json!([74.5, 16.0]));
        mainland_ring.push(json!([74.5, 17.5]));
        mainland_ring.push(json!([73.0, 17.5]));
        mainland_ring.push(json!([73.0, 22.0]));

        let mut polygons = Vec::with_capacity(31);
        polygons.push(json!([mainland_ring]));
        for k in 0..30usize {
            let x0 = 73.05 + (k as f64) * 0.02;
            let y0 = 16.05 + ((k % 5) as f64) * 0.05;
            polygons.push(json!([[
                [x0, y0],
                [x0 + 0.01, y0],
                [x0 + 0.01, y0 + 0.01],
                [x0, y0 + 0.01],
                [x0, y0]
            ]]));
        }
        let coastal_aoi = json!({
            "type": "MultiPolygon",
            "coordinates": polygons
        })
        .as_object()
        .unwrap()
        .clone();

        let exact_aoi = authoritative_catalogue_geometry(&coastal_aoi).unwrap();
        assert!(
            insar_core::wrapped_aoi_envelope(&exact_aoi).is_none(),
            "test fixture must exceed ProjectedTopologyBudget so wrapped_aoi_envelope returns None"
        );

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/data/search", listener.local_addr().unwrap());
        let inside_mainland =
            bhoonidhi_feature_for_rect("NISAR_COASTAL_MAINLAND_HIT", 76.0, 18.0, 77.0, 19.0);
        let bay_gap_only =
            bhoonidhi_feature_for_rect("NISAR_SEA_BAY_OUTSIDE_AOI", 73.9, 16.6, 74.4, 17.2);
        let payload = json!({
            "type": "FeatureCollection",
            "features": [inside_mainland, bay_gap_only],
            "links": [],
            "context": { "matched": 2 }
        })
        .to_string();

        let server = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let req_body = read_http_post_json(&mut stream);
                assert_eq!(req_body["bbox"], json!([73.0, 16.0, 80.5, 22.0]));
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    payload.len(),
                    payload
                );
                stream.write_all(resp.as_bytes()).unwrap();
            }
        });

        let query = CatalogSearchQuery {
            logical_dataset: super::super::BHOONIDHI_LOGICAL_DATASET.to_owned(),
            provider_id: super::super::BHOONIDHI_PROVIDER_ID.to_owned(),
            native_collection_id: super::super::BHOONIDHI_COLLECTION_ID.to_owned(),
            aoi_geojson: Some(coastal_aoi),
            filters: Map::new(),
            cursor: None,
            result_limit: Some(10),
        };
        let client = generated_client(&endpoint);
        let search_result = client.search(&query, &|| false).unwrap();
        assert_eq!(search_result.products.len(), 1);
        assert_eq!(
            search_result.products[0].product_id,
            "NISAR_COASTAL_MAINLAND_HIT"
        );

        let page_result = client.search_page(&query, &|| false).unwrap();
        assert_eq!(page_result.products.len(), 1);
        assert_eq!(
            page_result.products[0].product_id,
            "NISAR_COASTAL_MAINLAND_HIT"
        );
        server.join().unwrap();
    }

    #[test]
    fn wide_over_180_degree_aoi_when_envelope_unavailable_fails_closed() {
        // 60 concentric annuli spanning [-100, 100] (200° > 180°, <=64 components and <=4,096
        // total positions, with midpoint x=0 so every edge has |dx| <= 100°). This passes
        // authoritative_catalogue_geometry, exhausts ProjectedTopologyBudget so
        // wrapped_aoi_envelope returns None, and fails closed in native_catalogue_bbox (>180°).
        let ring = |w: f64, s: f64, e: f64, n: f64| {
            json!([[w, s], [0.0, s], [e, s], [e, n], [0.0, n], [w, n], [w, s]])
        };
        let polygons = (0..60usize)
            .map(|k| {
                let inset_x = (k as f64) * 0.25;
                let inset_y = (k as f64) * 0.08;
                let outer = ring(
                    -100.0 + inset_x,
                    0.0 + inset_y,
                    100.0 - inset_x,
                    20.0 - inset_y,
                );
                let inner = ring(
                    -100.0 + inset_x + 0.1,
                    0.0 + inset_y + 0.03,
                    100.0 - inset_x - 0.1,
                    20.0 - inset_y - 0.03,
                );
                json!([outer, inner])
            })
            .collect::<Vec<_>>();
        let aoi_map = json!({
            "type": "MultiPolygon",
            "coordinates": polygons
        })
        .as_object()
        .unwrap()
        .clone();
        let exact_aoi = authoritative_catalogue_geometry(&aoi_map).unwrap();
        assert!(insar_core::wrapped_aoi_envelope(&exact_aoi).is_none());
        let query = CatalogSearchQuery {
            logical_dataset: super::super::BHOONIDHI_LOGICAL_DATASET.to_owned(),
            provider_id: super::super::BHOONIDHI_PROVIDER_ID.to_owned(),
            native_collection_id: super::super::BHOONIDHI_COLLECTION_ID.to_owned(),
            aoi_geojson: Some(aoi_map),
            filters: Map::new(),
            cursor: None,
            result_limit: Some(10),
        };
        let err = generated_client("http://127.0.0.1:65433/data/search")
            .search(&query, &|| false)
            .unwrap_err();
        assert_eq!(err.code, "BHOONIDHI_QUERY_INVALID");
        assert_eq!(err.debug_context["reason"], "spatial_plan");
    }
}
