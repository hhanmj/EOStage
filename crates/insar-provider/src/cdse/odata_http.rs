use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use insar_core::{
    CatalogCollectionDescriptor, CatalogSearchQuery, CatalogSearchResponse, CatalogSearchStats,
    ContractError, ProviderNetworkPolicy, ProviderProduct, ProviderResult,
};
use reqwest::Client;
use serde_json::{json, Map, Value};
use tokio::runtime::{Builder as RuntimeBuilder, Runtime};
use url::Url;

use super::odata_mapper::{
    map_odata_page, map_odata_record_value, map_odata_records, sentinel2_collection_descriptor,
};
use super::odata_types::ODataPage;
use super::query::{
    build_odata_request_for_use, validate_continuation_for_endpoint, wrap_continuation,
    ODataRequest,
};
use crate::catalog::RouteUse;

#[path = "spatial_search.rs"]
mod spatial_search;

pub(crate) const MAX_RESPONSE_BYTES: u64 = 1024 * 1024;
const MAX_RETRY_AFTER_SECONDS: u64 = 60;
const CDSE_SENTINEL2_MAX_LOADED_RECORDS: u64 = 40_000;
const CDSE_SENTINEL2_MAX_PAGES: usize = 128;
const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(25);

pub(crate) struct RawPage {
    start_skip: u64,
    response_bytes: u64,
    matched_count: Option<u64>,
    records: Vec<Value>,
    next_cursor: Option<String>,
    next_skip: Option<u64>,
}

pub(crate) struct ODataHttpEngine {
    endpoint: Url,
    client: Client,
    runtime: Runtime,
    network_policy: ProviderNetworkPolicy,
    route_use: RouteUse,
    timeout: Duration,
}

pub(crate) fn odata_error(code: &str, message: &str, reason: &str) -> ContractError {
    ContractError::new(
        code,
        message,
        Map::from_iter([("reason".to_string(), Value::String(reason.to_string()))]),
    )
    .expect("bounded OData context is safe")
}

impl ODataHttpEngine {
    pub(crate) fn new(
        endpoint: Url,
        timeout: Duration,
        network_policy: ProviderNetworkPolicy,
    ) -> ProviderResult<Self> {
        Self::new_for_route(endpoint, timeout, network_policy, RouteUse::OfflineFixture)
    }

    pub(crate) fn new_for_route(
        endpoint: Url,
        timeout: Duration,
        network_policy: ProviderNetworkPolicy,
        route_use: RouteUse,
    ) -> ProviderResult<Self> {
        let mut client = Client::builder()
            .timeout(timeout)
            .tcp_nodelay(true)
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .referer(false);
        if endpoint.scheme() == "http"
            && endpoint
                .host_str()
                .and_then(|host| host.parse::<std::net::IpAddr>().ok())
                .is_some_and(|ip| ip.is_loopback())
        {
            client = client.tls_built_in_root_certs(false);
        }
        let client = crate::network_proxy::async_builder_with_proxy(client, network_policy.route())
            .build()
            .map_err(|_| {
                odata_error(
                    "CDSE_ODATA_ENDPOINT_INVALID",
                    "OData 客户端初始化失败",
                    "client",
                )
            })?;
        let runtime = RuntimeBuilder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| {
                odata_error(
                    "CDSE_ODATA_ENDPOINT_INVALID",
                    "OData 客户端初始化失败",
                    "runtime",
                )
            })?;
        Ok(Self {
            endpoint,
            client,
            runtime,
            network_policy,
            route_use,
            timeout,
        })
    }

    pub(crate) const fn network_policy(&self) -> ProviderNetworkPolicy {
        self.network_policy
    }

    pub(crate) fn search_page(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &dyn Fn() -> bool,
    ) -> ProviderResult<CatalogSearchResponse> {
        let query = &insar_core::clamp_catalogue_search_query(query);
        let request = build_odata_request_for_use(query, self.route_use)?;
        let page = self.fetch_raw_page(query, &request, cancelled)?;
        if query.cursor.is_none() && page.matched_count.is_none() {
            return Err(odata_error(
                "CDSE_ODATA_PAGE_INVALID",
                "Sentinel-2 OData 首页缺少匹配总数",
                "matched_count",
            ));
        }
        if page.next_skip.is_some_and(|skip| {
            page.start_skip.checked_add(page.records.len() as u64) != Some(skip)
        }) {
            return Err(odata_error(
                "CDSE_ODATA_CURSOR_UNTRUSTED",
                "Sentinel-2 续页偏移未连续推进",
                "skip_progress",
            ));
        }
        let next_cursor = page.next_cursor;
        let page = ODataPage {
            matched_count: page.matched_count,
            next_link: None,
            value: page.records,
        };
        map_odata_page(page, request.facts(), next_cursor)
    }

    pub(crate) fn search_all(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &dyn Fn() -> bool,
    ) -> ProviderResult<CatalogSearchResponse> {
        let query = &insar_core::clamp_catalogue_search_query(query);
        let mut current = query.clone();
        let started = std::time::Instant::now();
        let mut request = match build_odata_request_for_use(&current, self.route_use) {
            Ok(request) => request,
            Err(error) if spatial_search::can_replan(query, &error) => {
                return self.search_spatial_all(query, cancelled, started);
            }
            Err(error) => return Err(error),
        };
        let facts = request.facts().clone();
        if self.route_use == RouteUse::Production && current.cursor.is_none() {
            if let Some(target) = facts
                .result_limit
                .filter(|t| *t > crate::cdse::query::CDSE_SENTINEL2_MAX_PAGE_SIZE)
            {
                return self.search_all_production_concurrent(&request, &facts, target, cancelled);
            }
        }
        let initial_skip = request.current_skip();
        let mut records = Vec::new();
        let mut expected_matched_count = None;
        let mut pages_loaded = 0_usize;
        let mut seen_cursors = current.cursor.clone().into_iter().collect::<HashSet<_>>();

        loop {
            if pages_loaded >= CDSE_SENTINEL2_MAX_PAGES {
                return Err(odata_error(
                    "CDSE_ODATA_PAGE_BUDGET_EXCEEDED",
                    "Sentinel-2 OData 查询超过安全分页上限",
                    "page_budget",
                ));
            }
            if records.len() as u64 >= CDSE_SENTINEL2_MAX_LOADED_RECORDS {
                return Err(odata_error(
                    "CDSE_ODATA_RESULT_LIMIT_EXCEEDED",
                    "Sentinel-2 OData 查询超过安全记录上限",
                    "record_budget",
                ));
            }
            let page = self.fetch_raw_page(&current, &request, cancelled)?;
            let page_count = page.matched_count;
            let expected = match expected_matched_count {
                None => page_count.ok_or_else(|| {
                    odata_error(
                        "CDSE_ODATA_PAGE_INVALID",
                        "Sentinel-2 OData 首页缺少匹配总数",
                        "matched_count",
                    )
                })?,
                Some(expected) => {
                    if page_count.is_some_and(|actual| actual != expected) {
                        return Err(odata_error(
                            "CDSE_ODATA_PAGE_INVALID",
                            "Sentinel-2 OData 页面匹配总数不一致",
                            "matched_count",
                        ));
                    }
                    expected
                }
            };
            expected_matched_count = Some(expected);
            let loaded_after_page = initial_skip
                .checked_add(records.len() as u64)
                .and_then(|value| value.checked_add(page.records.len() as u64))
                .ok_or_else(|| {
                    odata_error(
                        "CDSE_ODATA_PAGE_INVALID",
                        "Sentinel-2 OData 页面偏移无效",
                        "loaded_skip_overflow",
                    )
                })?;
            if page.start_skip != request.current_skip() {
                return Err(odata_error(
                    "CDSE_ODATA_PAGE_INVALID",
                    "Sentinel-2 OData 页面匹配总数无效",
                    "matched_count_progress",
                ));
            }
            if page.records.is_empty() && page.next_cursor.is_some() {
                return Err(odata_error(
                    "CDSE_ODATA_CURSOR_UNTRUSTED",
                    "Sentinel-2 OData 空页不能继续分页",
                    "empty_page",
                ));
            }
            records.extend(page.records);
            pages_loaded += 1;
            if records.len() as u64 > CDSE_SENTINEL2_MAX_LOADED_RECORDS {
                return Err(odata_error(
                    "CDSE_ODATA_RESULT_LIMIT_EXCEEDED",
                    "Sentinel-2 OData 查询超过安全记录上限",
                    "record_budget",
                ));
            }
            match page.next_cursor {
                None => {
                    let limit_reached = facts.result_limit.is_some_and(|target| {
                        records.len() as u64 >= target && loaded_after_page < expected
                    });
                    if loaded_after_page != expected && !limit_reached {
                        return Err(odata_error(
                            "CDSE_ODATA_PAGE_INVALID",
                            "Sentinel-2 OData 缺少必要续页",
                            "matched_count_closure",
                        ));
                    }
                    return map_odata_records(
                        &records,
                        &facts,
                        expected_matched_count,
                        None,
                        pages_loaded as u64,
                        loaded_after_page == expected,
                    );
                }
                Some(cursor) => {
                    if facts
                        .result_limit
                        .is_some_and(|target| records.len() as u64 >= target)
                    {
                        return map_odata_records(
                            &records,
                            &facts,
                            expected_matched_count,
                            None,
                            pages_loaded as u64,
                            false,
                        );
                    }
                    if !seen_cursors.insert(cursor.clone()) {
                        return Err(odata_error(
                            "CDSE_ODATA_CURSOR_UNTRUSTED",
                            "Sentinel-2 OData 续页地址重复",
                            "cursor_cycle",
                        ));
                    }
                    if page.next_skip != Some(loaded_after_page) || loaded_after_page >= expected {
                        return Err(odata_error(
                            "CDSE_ODATA_CURSOR_UNTRUSTED",
                            "Sentinel-2 OData 续页超出匹配总数",
                            "matched_count_closure",
                        ));
                    }
                    current.cursor = Some(cursor);
                    request = build_odata_request_for_use(&current, self.route_use)?;
                    if cancelled() {
                        return Err(odata_error(
                            "CDSE_ODATA_CANCELLED",
                            "Sentinel-2 OData 查询已取消",
                            "between_pages",
                        ));
                    }
                }
            }
        }
    }

    fn search_all_production_concurrent(
        &self,
        request: &ODataRequest,
        facts: &super::query::Sentinel2SearchFacts,
        target: u64,
        cancelled: &dyn Fn() -> bool,
    ) -> ProviderResult<CatalogSearchResponse> {
        let page_size = crate::cdse::query::CDSE_SENTINEL2_MAX_PAGE_SIZE;
        let mut records = Vec::new();
        let mut expected_matched_count = None;
        let mut pages_loaded = 0_usize;
        let mut base_skip = 0_u64;

        loop {
            if pages_loaded >= CDSE_SENTINEL2_MAX_PAGES {
                return Err(odata_error(
                    "CDSE_ODATA_PAGE_BUDGET_EXCEEDED",
                    "Sentinel-2 OData 查询超过安全分页上限",
                    "page_budget",
                ));
            }
            if records.len() as u64 >= CDSE_SENTINEL2_MAX_LOADED_RECORDS {
                return Err(odata_error(
                    "CDSE_ODATA_RESULT_LIMIT_EXCEEDED",
                    "Sentinel-2 OData 查询超过安全记录上限",
                    "record_budget",
                ));
            }
            let wave_goal = match expected_matched_count {
                None => target,
                Some(expected) => target.min(expected).saturating_sub(records.len() as u64),
            };
            let wave = wave_goal.div_ceil(100).clamp(1, 3) as usize;
            let wave_len = wave.min(CDSE_SENTINEL2_MAX_PAGES - pages_loaded);
            let skips: Vec<(u64, bool)> = (0..wave_len)
                .map(|k| {
                    let skip = base_skip + (k as u64) * page_size;
                    let include_count = base_skip == 0 && k == 0;
                    (skip, include_count)
                })
                .collect();
            let wave_pages = self.fetch_concurrent_skip_wave(
                request,
                &skips,
                cancelled,
                self.timeout,
                MAX_RESPONSE_BYTES,
            )?;

            for ((skip, _), (page, _)) in skips.into_iter().zip(wave_pages) {
                let expected = match expected_matched_count {
                    None => page.matched_count.ok_or_else(|| {
                        odata_error(
                            "CDSE_ODATA_PAGE_INVALID",
                            "Sentinel-2 OData 首页缺少匹配总数",
                            "matched_count",
                        )
                    })?,
                    Some(expected) => {
                        if page.matched_count.is_some_and(|actual| actual != expected) {
                            return Err(odata_error(
                                "CDSE_ODATA_PAGE_INVALID",
                                "Sentinel-2 OData 页面匹配总数不一致",
                                "matched_count",
                            ));
                        }
                        expected
                    }
                };
                expected_matched_count = Some(expected);

                let loaded_after_page =
                    skip.checked_add(page.value.len() as u64).ok_or_else(|| {
                        odata_error(
                            "CDSE_ODATA_PAGE_INVALID",
                            "Sentinel-2 OData 页面偏移无效",
                            "loaded_skip_overflow",
                        )
                    })?;
                if loaded_after_page > expected {
                    return Err(odata_error(
                        "CDSE_ODATA_PAGE_INVALID",
                        "Sentinel-2 OData 页面匹配总数无效",
                        "matched_count_progress",
                    ));
                }
                if page.value.is_empty() && page.next_link.is_some() {
                    return Err(odata_error(
                        "CDSE_ODATA_CURSOR_UNTRUSTED",
                        "Sentinel-2 OData 空页不能继续分页",
                        "empty_page",
                    ));
                }
                let next_skip = match page.next_link.as_deref() {
                    None => None,
                    Some(raw_link) => Some(
                        request.validate_next_link_skip_for_endpoint(&self.endpoint, raw_link)?,
                    ),
                };
                records.extend(page.value);
                pages_loaded += 1;
                if records.len() as u64 > CDSE_SENTINEL2_MAX_LOADED_RECORDS {
                    return Err(odata_error(
                        "CDSE_ODATA_RESULT_LIMIT_EXCEEDED",
                        "Sentinel-2 OData 查询超过安全记录上限",
                        "record_budget",
                    ));
                }
                match next_skip {
                    None => {
                        let limit_reached =
                            records.len() as u64 >= target && loaded_after_page < expected;
                        if loaded_after_page != expected && !limit_reached {
                            return Err(odata_error(
                                "CDSE_ODATA_PAGE_INVALID",
                                "Sentinel-2 OData 缺少必要续页",
                                "matched_count_closure",
                            ));
                        }
                        return map_odata_records(
                            &records,
                            facts,
                            expected_matched_count,
                            None,
                            pages_loaded as u64,
                            loaded_after_page == expected,
                        );
                    }
                    Some(next_skip) => {
                        if records.len() as u64 >= target {
                            return map_odata_records(
                                &records,
                                facts,
                                expected_matched_count,
                                None,
                                pages_loaded as u64,
                                false,
                            );
                        }
                        if next_skip != loaded_after_page || loaded_after_page >= expected {
                            return Err(odata_error(
                                "CDSE_ODATA_CURSOR_UNTRUSTED",
                                "Sentinel-2 OData 续页超出匹配总数",
                                "matched_count_closure",
                            ));
                        }
                    }
                }
            }

            if cancelled() {
                return Err(odata_error(
                    "CDSE_ODATA_CANCELLED",
                    "Sentinel-2 OData 查询已取消",
                    "between_pages",
                ));
            }
            base_skip += (wave_len as u64) * page_size;
        }
    }

    pub(crate) fn search_streaming<S, P>(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &dyn Fn() -> bool,
        sink: &mut S,
        progress: &P,
    ) -> ProviderResult<(CatalogCollectionDescriptor, CatalogSearchStats)>
    where
        S: FnMut(&[ProviderProduct]) -> ProviderResult<()>,
        P: Fn(crate::asf::catalog_client::CatalogSearchProviderProgress),
    {
        let query = &insar_core::clamp_catalogue_search_query(query);
        let mut current = query.clone();
        let started = Instant::now();
        let mut request = match build_odata_request_for_use(&current, self.route_use) {
            Ok(request) => request,
            Err(error) if spatial_search::can_replan(query, &error) => {
                return Err(ContractError::new(
                    "STAC_RESPONSE_INVALID",
                    "Sentinel-2 OData 分块空间检索需要完整结果归并排序",
                    Map::from_iter([(
                        "reason".to_owned(),
                        Value::String("streaming_cap_order".to_owned()),
                    )]),
                )
                .expect("streaming_cap_order context is safe"));
            }
            Err(error) => return Err(error),
        };
        let facts = request.facts().clone();
        let page_size = crate::cdse::query::CDSE_SENTINEL2_MAX_PAGE_SIZE;

        if self.route_use == RouteUse::Production && current.cursor.is_none() {
            if let Some(target) = facts.result_limit.filter(|t| *t > page_size) {
                return self.search_streaming_production_concurrent(
                    &request, &facts, target, cancelled, started, sink, progress,
                );
            }
        }

        let initial_skip = request.current_skip();
        let mut fetched_record_count = 0_u64;
        let mut mapped_record_count = 0_u64;
        let mut exact_duplicate_record_count = 0_u64;
        let mut expected_matched_count = None;
        let mut pages_loaded = 0_usize;
        let mut seen_cursors = current.cursor.clone().into_iter().collect::<HashSet<_>>();
        let mut admitted_by_id = HashMap::<String, ProviderProduct>::new();
        let mut item_errors = Vec::new();
        let mut returned_count = 0_u64;

        loop {
            if pages_loaded >= CDSE_SENTINEL2_MAX_PAGES {
                return Err(odata_error(
                    "CDSE_ODATA_PAGE_BUDGET_EXCEEDED",
                    "Sentinel-2 OData 查询超过安全分页上限",
                    "page_budget",
                ));
            }
            if fetched_record_count >= CDSE_SENTINEL2_MAX_LOADED_RECORDS {
                return Err(odata_error(
                    "CDSE_ODATA_RESULT_LIMIT_EXCEEDED",
                    "Sentinel-2 OData 查询超过安全记录上限",
                    "record_budget",
                ));
            }
            let page = self.fetch_raw_page(&current, &request, cancelled)?;
            let page_count = page.matched_count;
            let expected = match expected_matched_count {
                None => page_count.ok_or_else(|| {
                    odata_error(
                        "CDSE_ODATA_PAGE_INVALID",
                        "Sentinel-2 OData 首页缺少匹配总数",
                        "matched_count",
                    )
                })?,
                Some(expected) => {
                    if page_count.is_some_and(|actual| actual != expected) {
                        return Err(odata_error(
                            "CDSE_ODATA_PAGE_INVALID",
                            "Sentinel-2 OData 页面匹配总数不一致",
                            "matched_count",
                        ));
                    }
                    expected
                }
            };
            expected_matched_count = Some(expected);
            let loaded_after_page = initial_skip
                .checked_add(fetched_record_count)
                .and_then(|value| value.checked_add(page.records.len() as u64))
                .ok_or_else(|| {
                    odata_error(
                        "CDSE_ODATA_PAGE_INVALID",
                        "Sentinel-2 OData 页面偏移无效",
                        "loaded_skip_overflow",
                    )
                })?;
            if page.start_skip != request.current_skip() {
                return Err(odata_error(
                    "CDSE_ODATA_PAGE_INVALID",
                    "Sentinel-2 OData 页面匹配总数无效",
                    "matched_count_progress",
                ));
            }
            if page.records.is_empty() && page.next_cursor.is_some() {
                return Err(odata_error(
                    "CDSE_ODATA_CURSOR_UNTRUSTED",
                    "Sentinel-2 OData 空页不能继续分页",
                    "empty_page",
                ));
            }
            fetched_record_count = fetched_record_count.saturating_add(page.records.len() as u64);
            pages_loaded += 1;
            if fetched_record_count > CDSE_SENTINEL2_MAX_LOADED_RECORDS {
                return Err(odata_error(
                    "CDSE_ODATA_RESULT_LIMIT_EXCEEDED",
                    "Sentinel-2 OData 查询超过安全记录上限",
                    "record_budget",
                ));
            }

            let mut page_admitted = Vec::with_capacity(page.records.len());
            for record in &page.records {
                match map_odata_record_value(record, &facts) {
                    Ok(product) => {
                        if let Some(existing) = admitted_by_id.get(&product.product_id) {
                            if existing == &product {
                                mapped_record_count += 1;
                                exact_duplicate_record_count += 1;
                            } else {
                                item_errors.push(odata_error(
                                    "CDSE_ODATA_UUID_CONFLICT",
                                    "Sentinel-2 OData UUID 存在冲突记录",
                                    "conflicting_authoritative_fields",
                                ));
                            }
                        } else {
                            mapped_record_count += 1;
                            admitted_by_id.insert(product.product_id.clone(), product.clone());
                            if facts
                                .result_limit
                                .is_none_or(|limit| returned_count < limit)
                            {
                                returned_count += 1;
                                page_admitted.push(product);
                            }
                        }
                    }
                    Err(err) => item_errors.push(err),
                }
            }
            if !item_errors.is_empty() {
                return Err(item_errors.remove(0));
            }

            let remote_exhausted = match page.next_cursor.as_deref() {
                None => {
                    let limit_reached = facts.result_limit.is_some_and(|target| {
                        admitted_by_id.len() as u64 >= target && loaded_after_page < expected
                    });
                    if loaded_after_page != expected && !limit_reached {
                        return Err(odata_error(
                            "CDSE_ODATA_PAGE_INVALID",
                            "Sentinel-2 OData 缺少必要续页",
                            "matched_count_closure",
                        ));
                    }
                    Some(loaded_after_page == expected)
                }
                Some(_)
                    if facts
                        .result_limit
                        .is_some_and(|target| admitted_by_id.len() as u64 >= target) =>
                {
                    Some(false)
                }
                Some(_) => None,
            };

            if !page_admitted.is_empty() {
                sink(&page_admitted)?;
            }
            progress(crate::asf::catalog_client::CatalogSearchProviderProgress {
                provider_request_count: pages_loaded as u64,
                transport_part_count: 1,
                raw_fetched_count: fetched_record_count,
                mapped_count: mapped_record_count,
                exact_matched_count: admitted_by_id.len() as u64,
                unique_admitted_count: returned_count,
                requested_limit: facts.result_limit,
                remote_total: expected_matched_count,
                elapsed_ms: started.elapsed().as_millis().min(u64::MAX as u128) as u64,
            });

            if let Some(exhausted) = remote_exhausted {
                let unique_count = admitted_by_id.len() as u64;
                let working_set_satisfied = facts
                    .result_limit
                    .map(|target| unique_count >= target || exhausted)
                    .unwrap_or(exhausted);
                if !working_set_satisfied {
                    return Err(odata_error(
                        "CDSE_ODATA_WORKING_SET_INCOMPLETE",
                        "Sentinel-2 OData 未完成请求结果集",
                        "working_set",
                    ));
                }
                let stats = CatalogSearchStats {
                    requested_limit: facts.result_limit,
                    matched_count: expected_matched_count,
                    request_count: pages_loaded as u64,
                    transport_part_count: 1,
                    fetched_record_count,
                    mapped_record_count,
                    profile_filtered_record_count: 0,
                    exact_duplicate_record_count,
                    unique_provider_product_count: unique_count,
                    exact_matched_count: unique_count,
                    returned_count,
                    remote_exhausted: exhausted,
                    working_set_satisfied,
                    elapsed_ms: started.elapsed().as_millis().min(u64::MAX as u128) as u64,
                    truncated: false,
                };
                return Ok((sentinel2_collection_descriptor(&facts), stats));
            }

            let cursor = page.next_cursor.expect("non-terminal page has cursor");
            if !seen_cursors.insert(cursor.clone()) {
                return Err(odata_error(
                    "CDSE_ODATA_CURSOR_UNTRUSTED",
                    "Sentinel-2 OData 续页地址重复",
                    "cursor_cycle",
                ));
            }
            if page.next_skip != Some(loaded_after_page) || loaded_after_page >= expected {
                return Err(odata_error(
                    "CDSE_ODATA_CURSOR_UNTRUSTED",
                    "Sentinel-2 OData 续页超出匹配总数",
                    "matched_count_closure",
                ));
            }
            current.cursor = Some(cursor);
            request = build_odata_request_for_use(&current, self.route_use)?;
            if cancelled() {
                return Err(odata_error(
                    "CDSE_ODATA_CANCELLED",
                    "Sentinel-2 OData 查询已取消",
                    "between_pages",
                ));
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn search_streaming_production_concurrent<S, P>(
        &self,
        request: &ODataRequest,
        facts: &super::query::Sentinel2SearchFacts,
        target: u64,
        cancelled: &dyn Fn() -> bool,
        started: Instant,
        sink: &mut S,
        progress: &P,
    ) -> ProviderResult<(CatalogCollectionDescriptor, CatalogSearchStats)>
    where
        S: FnMut(&[ProviderProduct]) -> ProviderResult<()>,
        P: Fn(crate::asf::catalog_client::CatalogSearchProviderProgress),
    {
        let page_size = crate::cdse::query::CDSE_SENTINEL2_MAX_PAGE_SIZE;
        let mut fetched_record_count = 0_u64;
        let mut mapped_record_count = 0_u64;
        let mut exact_duplicate_record_count = 0_u64;
        let mut expected_matched_count = None;
        let mut pages_loaded = 0_usize;
        let mut base_skip = 0_u64;
        let mut admitted_by_id = HashMap::<String, ProviderProduct>::new();
        let mut returned_count = 0_u64;
        let mut item_errors = Vec::new();

        loop {
            if pages_loaded >= CDSE_SENTINEL2_MAX_PAGES {
                return Err(odata_error(
                    "CDSE_ODATA_PAGE_BUDGET_EXCEEDED",
                    "Sentinel-2 OData 查询超过安全分页上限",
                    "page_budget",
                ));
            }
            if fetched_record_count >= CDSE_SENTINEL2_MAX_LOADED_RECORDS {
                return Err(odata_error(
                    "CDSE_ODATA_RESULT_LIMIT_EXCEEDED",
                    "Sentinel-2 OData 查询超过安全记录上限",
                    "record_budget",
                ));
            }
            let wave_goal = match expected_matched_count {
                None => target,
                Some(expected) => target
                    .min(expected)
                    .saturating_sub(admitted_by_id.len() as u64),
            };
            let wave = wave_goal.div_ceil(100).clamp(1, 3) as usize;
            let wave_len = wave.min(CDSE_SENTINEL2_MAX_PAGES - pages_loaded);
            let skips: Vec<(u64, bool)> = (0..wave_len)
                .map(|k| {
                    let skip = base_skip + (k as u64) * page_size;
                    let include_count = base_skip == 0 && k == 0;
                    (skip, include_count)
                })
                .collect();
            let wave_pages = self.fetch_concurrent_skip_wave(
                request,
                &skips,
                cancelled,
                self.timeout,
                MAX_RESPONSE_BYTES,
            )?;

            for ((skip, _), (page, _)) in skips.into_iter().zip(wave_pages) {
                let expected = match expected_matched_count {
                    None => page.matched_count.ok_or_else(|| {
                        odata_error(
                            "CDSE_ODATA_PAGE_INVALID",
                            "Sentinel-2 OData 首页缺少匹配总数",
                            "matched_count",
                        )
                    })?,
                    Some(expected) => {
                        if page.matched_count.is_some_and(|actual| actual != expected) {
                            return Err(odata_error(
                                "CDSE_ODATA_PAGE_INVALID",
                                "Sentinel-2 OData 页面匹配总数不一致",
                                "matched_count",
                            ));
                        }
                        expected
                    }
                };
                expected_matched_count = Some(expected);

                let loaded_after_page =
                    skip.checked_add(page.value.len() as u64).ok_or_else(|| {
                        odata_error(
                            "CDSE_ODATA_PAGE_INVALID",
                            "Sentinel-2 OData 页面偏移无效",
                            "loaded_skip_overflow",
                        )
                    })?;
                if loaded_after_page > expected {
                    return Err(odata_error(
                        "CDSE_ODATA_PAGE_INVALID",
                        "Sentinel-2 OData 页面匹配总数无效",
                        "matched_count_progress",
                    ));
                }
                if page.value.is_empty() && page.next_link.is_some() {
                    return Err(odata_error(
                        "CDSE_ODATA_CURSOR_UNTRUSTED",
                        "Sentinel-2 OData 空页不能继续分页",
                        "empty_page",
                    ));
                }
                let next_skip = match page.next_link.as_deref() {
                    None => None,
                    Some(raw_link) => Some(
                        request.validate_next_link_skip_for_endpoint(&self.endpoint, raw_link)?,
                    ),
                };
                fetched_record_count = fetched_record_count.saturating_add(page.value.len() as u64);
                pages_loaded += 1;
                if fetched_record_count > CDSE_SENTINEL2_MAX_LOADED_RECORDS {
                    return Err(odata_error(
                        "CDSE_ODATA_RESULT_LIMIT_EXCEEDED",
                        "Sentinel-2 OData 查询超过安全记录上限",
                        "record_budget",
                    ));
                }

                let mut page_admitted = Vec::with_capacity(page.value.len());
                for record in &page.value {
                    match map_odata_record_value(record, facts) {
                        Ok(product) => {
                            if let Some(existing) = admitted_by_id.get(&product.product_id) {
                                if existing == &product {
                                    mapped_record_count += 1;
                                    exact_duplicate_record_count += 1;
                                } else {
                                    item_errors.push(odata_error(
                                        "CDSE_ODATA_UUID_CONFLICT",
                                        "Sentinel-2 OData UUID 存在冲突记录",
                                        "conflicting_authoritative_fields",
                                    ));
                                }
                            } else {
                                mapped_record_count += 1;
                                admitted_by_id.insert(product.product_id.clone(), product.clone());
                                if returned_count < target {
                                    returned_count += 1;
                                    page_admitted.push(product);
                                }
                            }
                        }
                        Err(err) => item_errors.push(err),
                    }
                }
                if !item_errors.is_empty() {
                    return Err(item_errors.remove(0));
                }

                let done_exhausted = match next_skip {
                    None => {
                        let limit_reached =
                            admitted_by_id.len() as u64 >= target && loaded_after_page < expected;
                        if loaded_after_page != expected && !limit_reached {
                            return Err(odata_error(
                                "CDSE_ODATA_PAGE_INVALID",
                                "Sentinel-2 OData 缺少必要续页",
                                "matched_count_closure",
                            ));
                        }
                        Some(loaded_after_page == expected)
                    }
                    Some(ns) => {
                        if admitted_by_id.len() as u64 >= target {
                            Some(false)
                        } else if ns != loaded_after_page || loaded_after_page >= expected {
                            return Err(odata_error(
                                "CDSE_ODATA_CURSOR_UNTRUSTED",
                                "Sentinel-2 OData 续页超出匹配总数",
                                "matched_count_closure",
                            ));
                        } else {
                            None
                        }
                    }
                };

                if !page_admitted.is_empty() {
                    sink(&page_admitted)?;
                }
                progress(crate::asf::catalog_client::CatalogSearchProviderProgress {
                    provider_request_count: pages_loaded as u64,
                    transport_part_count: 1,
                    raw_fetched_count: fetched_record_count,
                    mapped_count: mapped_record_count,
                    exact_matched_count: admitted_by_id.len() as u64,
                    unique_admitted_count: returned_count,
                    requested_limit: facts.result_limit,
                    remote_total: expected_matched_count,
                    elapsed_ms: started.elapsed().as_millis().min(u64::MAX as u128) as u64,
                });

                if let Some(exhausted) = done_exhausted {
                    let unique_count = admitted_by_id.len() as u64;
                    let working_set_satisfied = unique_count >= target || exhausted;
                    if !working_set_satisfied {
                        return Err(odata_error(
                            "CDSE_ODATA_WORKING_SET_INCOMPLETE",
                            "Sentinel-2 OData 未完成请求结果集",
                            "working_set",
                        ));
                    }
                    let stats = CatalogSearchStats {
                        requested_limit: facts.result_limit,
                        matched_count: expected_matched_count,
                        request_count: pages_loaded as u64,
                        transport_part_count: 1,
                        fetched_record_count,
                        mapped_record_count,
                        profile_filtered_record_count: 0,
                        exact_duplicate_record_count,
                        unique_provider_product_count: unique_count,
                        exact_matched_count: unique_count,
                        returned_count,
                        remote_exhausted: exhausted,
                        working_set_satisfied,
                        elapsed_ms: started.elapsed().as_millis().min(u64::MAX as u128) as u64,
                        truncated: false,
                    };
                    return Ok((sentinel2_collection_descriptor(facts), stats));
                }
            }

            if cancelled() {
                return Err(odata_error(
                    "CDSE_ODATA_CANCELLED",
                    "Sentinel-2 OData 查询已取消",
                    "between_pages",
                ));
            }
            base_skip += (wave_len as u64) * page_size;
        }
    }

    pub(super) fn fetch_concurrent_skip_wave(
        &self,
        request: &ODataRequest,
        skips: &[(u64, bool)],
        cancelled: &dyn Fn() -> bool,
        timeout: Duration,
        max_response_bytes: u64,
    ) -> ProviderResult<Vec<(ODataPage, usize)>> {
        type SkipPageFuture<'a> = std::pin::Pin<
            Box<dyn std::future::Future<Output = ProviderResult<(ODataPage, usize)>> + 'a>,
        >;
        self.runtime.block_on(async {
            let mut futures: Vec<SkipPageFuture<'_>> = skips
                .iter()
                .map(|(skip, include_count)| {
                    Box::pin(self.fetch_skip_page_async(
                        request,
                        *skip,
                        *include_count,
                        cancelled,
                        timeout,
                        max_response_bytes,
                    )) as _
                })
                .collect();
            let mut outputs: Vec<Option<(ODataPage, usize)>> =
                (0..futures.len()).map(|_| None).collect();
            std::future::poll_fn(|cx| {
                let mut all_ready = true;
                for (idx, fut) in futures.iter_mut().enumerate() {
                    if outputs[idx].is_none() {
                        match fut.as_mut().poll(cx) {
                            std::task::Poll::Ready(Ok(val)) => {
                                outputs[idx] = Some(val);
                            }
                            std::task::Poll::Ready(Err(err)) => {
                                return std::task::Poll::Ready(Err(err));
                            }
                            std::task::Poll::Pending => {
                                all_ready = false;
                            }
                        }
                    }
                }
                if all_ready {
                    let collected = outputs
                        .iter_mut()
                        .map(|slot| slot.take().expect("all futures ready"))
                        .collect();
                    std::task::Poll::Ready(Ok(collected))
                } else {
                    std::task::Poll::Pending
                }
            })
            .await
        })
    }

    async fn fetch_skip_page_async(
        &self,
        request: &ODataRequest,
        skip: u64,
        include_count: bool,
        cancelled: &dyn Fn() -> bool,
        timeout: Duration,
        max_response_bytes: u64,
    ) -> ProviderResult<(ODataPage, usize)> {
        if cancelled() {
            return Err(odata_error(
                "CDSE_ODATA_CANCELLED",
                "Sentinel-2 OData 查询已取消",
                "before_request",
            ));
        }
        let url = request.url_for_skip(&self.endpoint, skip, include_count)?;
        let response = self
            .fetch_response_async(url, cancelled, timeout, max_response_bytes)
            .await?;
        if !(200..=299).contains(&response.status_code) {
            return Err(
                self.status_error_contract(response.status_code, response.retry_after_seconds)
            );
        }
        let page: ODataPage = serde_json::from_slice(&response.body).map_err(|_| {
            odata_error(
                "CDSE_ODATA_JSON_INVALID",
                "Sentinel-2 OData 响应格式无效",
                "json",
            )
        })?;
        if page.value.len() as u64 > request.page_size() {
            return Err(odata_error(
                "CDSE_ODATA_PAGE_INVALID",
                "Sentinel-2 OData 页面超过请求上限",
                "page_size",
            ));
        }
        Ok((page, response.body.len()))
    }

    fn fetch_raw_page(
        &self,
        query: &CatalogSearchQuery,
        request: &ODataRequest,
        cancelled: &dyn Fn() -> bool,
    ) -> ProviderResult<RawPage> {
        self.fetch_raw_page_with_limits(query, request, cancelled, self.timeout, MAX_RESPONSE_BYTES)
    }

    fn fetch_raw_page_with_limits(
        &self,
        query: &CatalogSearchQuery,
        request: &ODataRequest,
        cancelled: &dyn Fn() -> bool,
        timeout: Duration,
        max_response_bytes: u64,
    ) -> ProviderResult<RawPage> {
        if cancelled() {
            return Err(odata_error(
                "CDSE_ODATA_CANCELLED",
                "Sentinel-2 OData 查询已取消",
                "before_request",
            ));
        }
        let url = match request.continuation() {
            Some(_) => validate_continuation_for_endpoint(request, &self.endpoint)?.url,
            None => request.initial_url(&self.endpoint)?,
        };
        let response = self.fetch_response(url, cancelled, timeout, max_response_bytes)?;
        if !(200..=299).contains(&response.status_code) {
            return self.status_error(response.status_code, response.retry_after_seconds);
        }
        let page: ODataPage = serde_json::from_slice(&response.body).map_err(|_| {
            odata_error(
                "CDSE_ODATA_JSON_INVALID",
                "Sentinel-2 OData 响应格式无效",
                "json",
            )
        })?;
        if page.value.len() as u64 > request.page_size() {
            return Err(odata_error(
                "CDSE_ODATA_PAGE_INVALID",
                "Sentinel-2 OData 页面超过请求上限",
                "page_size",
            ));
        }
        let (next_cursor, next_skip) = match page.next_link.as_deref() {
            None => (None, None),
            Some(next_link) => {
                let next_link = Url::parse(next_link).map_err(|_| {
                    odata_error(
                        "CDSE_ODATA_CURSOR_UNTRUSTED",
                        "Sentinel-2 续页地址无效",
                        "parse",
                    )
                })?;
                let cursor = wrap_continuation(request.fingerprint(), &next_link)?;
                let continuation_query = CatalogSearchQuery {
                    cursor: Some(cursor.clone()),
                    ..query.clone()
                };
                let continuation_request =
                    build_odata_request_for_use(&continuation_query, self.route_use)?;
                let validated =
                    validate_continuation_for_endpoint(&continuation_request, &self.endpoint)?;
                (Some(cursor), Some(validated.skip))
            }
        };
        Ok(RawPage {
            start_skip: request.current_skip(),
            response_bytes: response.body.len() as u64,
            matched_count: page.matched_count,
            records: page.value,
            next_cursor,
            next_skip,
        })
    }

    fn fetch_response(
        &self,
        url: Url,
        cancelled: &dyn Fn() -> bool,
        timeout: Duration,
        max_response_bytes: u64,
    ) -> ProviderResult<RawHttpResponse> {
        self.runtime.block_on(self.fetch_response_async(
            url,
            cancelled,
            timeout,
            max_response_bytes,
        ))
    }

    async fn fetch_response_async(
        &self,
        url: Url,
        cancelled: &dyn Fn() -> bool,
        timeout: Duration,
        max_response_bytes: u64,
    ) -> ProviderResult<RawHttpResponse> {
        let mut request = Box::pin(self.client.get(url).timeout(timeout).send());
        let mut response = loop {
            match tokio::time::timeout(CANCELLATION_POLL_INTERVAL, request.as_mut()).await {
                Ok(result) => {
                    break result.map_err(|request_error| {
                        if request_error.is_timeout() {
                            odata_error(
                                "CDSE_ODATA_TIMEOUT",
                                "Sentinel-2 OData 请求超时",
                                "timeout",
                            )
                        } else {
                            odata_error(
                                "CDSE_ODATA_HTTP_FAILURE",
                                "Sentinel-2 OData 请求失败",
                                "transport",
                            )
                        }
                    })?;
                }
                Err(_) if cancelled() => {
                    return Err(odata_error(
                        "CDSE_ODATA_CANCELLED",
                        "Sentinel-2 OData 查询已取消",
                        "awaiting_response_headers",
                    ));
                }
                Err(_) => continue,
            }
        };
        let status_code = response.status().as_u16();
        let retry_after_seconds = response
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value <= MAX_RETRY_AFTER_SECONDS);
        if !(200..=299).contains(&status_code) {
            return Ok(RawHttpResponse {
                status_code,
                retry_after_seconds,
                body: Vec::new(),
            });
        }

        let mut body = Vec::new();
        loop {
            if cancelled() {
                return Err(odata_error(
                    "CDSE_ODATA_CANCELLED",
                    "Sentinel-2 OData 查询已取消",
                    "after_response",
                ));
            }
            let mut next_chunk = Box::pin(response.chunk());
            let chunk = loop {
                match tokio::time::timeout(CANCELLATION_POLL_INTERVAL, next_chunk.as_mut()).await {
                    Ok(result) => {
                        break result.map_err(|_| {
                            odata_error(
                                "CDSE_ODATA_HTTP_FAILURE",
                                "Sentinel-2 OData 响应读取失败",
                                "body",
                            )
                        })?;
                    }
                    Err(_) if cancelled() => {
                        return Err(odata_error(
                            "CDSE_ODATA_CANCELLED",
                            "Sentinel-2 OData 查询已取消",
                            "after_response",
                        ));
                    }
                    Err(_) => continue,
                }
            };
            let Some(chunk) = chunk else {
                break;
            };
            if body.len().saturating_add(chunk.len()) as u64 > max_response_bytes {
                return Err(odata_error(
                    "CDSE_ODATA_RESPONSE_TOO_LARGE",
                    "Sentinel-2 OData 响应过大",
                    "response_size",
                ));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(RawHttpResponse {
            status_code,
            retry_after_seconds,
            body,
        })
    }

    fn status_error(&self, status_code: u16, retry_after: Option<u64>) -> ProviderResult<RawPage> {
        Err(self.status_error_contract(status_code, retry_after))
    }

    fn status_error_contract(&self, status_code: u16, retry_after: Option<u64>) -> ContractError {
        let code = match status_code {
            300..=399 => "CDSE_ODATA_HTTP_REDIRECT",
            401 => "CDSE_ODATA_HTTP_UNAUTHORIZED",
            403 => "CDSE_ODATA_HTTP_FORBIDDEN",
            404 => "CDSE_ODATA_HTTP_NOT_FOUND",
            407 => "CDSE_ODATA_HTTP_PROXY_AUTH_REQUIRED",
            429 => "CDSE_ODATA_HTTP_RATE_LIMITED",
            500..=599 => "CDSE_ODATA_HTTP_SERVER",
            _ => "CDSE_ODATA_HTTP_STATUS",
        };
        let mut context = Map::from_iter([
            ("reason".to_string(), Value::String("status".to_string())),
            ("status_code".to_string(), json!(status_code)),
        ]);
        if let Some(retry_after) = retry_after {
            context.insert("retry_after_seconds".to_string(), json!(retry_after));
        }
        ContractError::new(code, "Sentinel-2 OData 服务返回错误", context)
            .expect("bounded status context is safe")
    }
}

struct RawHttpResponse {
    status_code: u16,
    retry_after_seconds: Option<u64>,
    body: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::sync::Mutex;
    use std::thread;
    use std::time::Instant;

    use serde_json::json;

    use super::*;

    static SYSTEM_PROXY_ENVIRONMENT_LOCK: Mutex<()> = Mutex::new(());
    const PROXY_ENVIRONMENT_VARIABLES: [&str; 8] = [
        "HTTP_PROXY",
        "http_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
        "NO_PROXY",
        "no_proxy",
    ];

    struct RestoredProxyEnvironment {
        values: Vec<(&'static str, Option<OsString>)>,
    }

    impl RestoredProxyEnvironment {
        fn use_http_proxy(proxy: &str) -> Self {
            let values = PROXY_ENVIRONMENT_VARIABLES
                .into_iter()
                .map(|name| (name, std::env::var_os(name)))
                .collect::<Vec<_>>();
            for (name, _) in &values {
                // This test has the process-local environment lock until Drop restores it.
                unsafe { std::env::remove_var(name) };
            }
            unsafe {
                std::env::set_var("HTTP_PROXY", proxy);
                std::env::set_var("http_proxy", proxy);
            }
            Self { values }
        }
    }

    impl Drop for RestoredProxyEnvironment {
        fn drop(&mut self) {
            for (name, value) in &self.values {
                unsafe {
                    match value {
                        Some(value) => std::env::set_var(name, value),
                        None => std::env::remove_var(name),
                    }
                }
            }
        }
    }

    fn query() -> CatalogSearchQuery {
        serde_json::from_value(json!({
            "logical_dataset": "optical.sentinel-2.l2a",
            "provider_id": "copernicus.dataspace.odata",
            "native_collection_id": "SENTINEL-2",
            "aoi_geojson": {
                "type": "Polygon",
                "coordinates": [[[110.0, 30.0], [111.0, 30.0], [111.0, 31.0], [110.0, 30.0]]]
            },
            "cursor": null,
            "result_limit": 2,
            "filters": {
                "product_type": "S2MSI2A",
                "datetime": {
                    "start": "2026-01-01T00:00:00Z",
                    "end": "2026-01-02T00:00:00Z"
                }
            }
        }))
        .expect("valid OData test query")
    }

    #[test]
    fn chunked_body_read_observes_cancellation_before_the_full_response_arrives() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback listener");
        let address = listener.local_addr().expect("listener address");
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accepted request");
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request);
            let body = br#"{"@odata.count":0,"value":[]}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .expect("response header");
            stream.write_all(&body[..8]).expect("first body chunk");
            stream.flush().expect("flush first body chunk");
            thread::sleep(Duration::from_millis(700));
            let _ = stream.write_all(&body[8..]);
        });
        let endpoint =
            Url::parse(&format!("http://{address}/odata/v1/")).expect("loopback endpoint");
        let engine = ODataHttpEngine::new(
            endpoint,
            Duration::from_secs(2),
            ProviderNetworkPolicy::direct(),
        )
        .expect("engine");
        let checks = AtomicUsize::new(0);
        let started = Instant::now();
        let error = engine
            .search_all(&query(), &|| checks.fetch_add(1, Ordering::SeqCst) >= 2)
            .expect_err("cancellation must interrupt a chunked response");

        assert_eq!(error.code, "CDSE_ODATA_CANCELLED");
        assert!(
            started.elapsed() < Duration::from_millis(350),
            "client waited for the delayed body instead of checking cancellation between chunks"
        );
        worker.join().expect("server worker");
    }

    #[test]
    fn system_route_uses_the_configured_proxy_without_adding_secret_or_cookie_headers() {
        let _environment_lock = SYSTEM_PROXY_ENVIRONMENT_LOCK
            .lock()
            .expect("proxy environment lock");
        let direct_listener =
            TcpListener::bind("127.0.0.1:0").expect("unused direct target listener");
        let direct_target_port = direct_listener
            .local_addr()
            .expect("direct target address")
            .port();
        drop(direct_listener);
        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback proxy listener");
        let address = listener.local_addr().expect("proxy listener address");
        let (request_tx, request_rx) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("proxy accepted request");
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .expect("proxy read timeout");
            let mut request = Vec::new();
            let mut chunk = [0_u8; 1024];
            loop {
                let read = stream.read(&mut chunk).expect("proxy request bytes");
                assert!(read > 0, "proxy request ended before its headers");
                request.extend_from_slice(&chunk[..read]);
                assert!(
                    request.len() <= 8192,
                    "proxy request headers exceed the bound"
                );
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            request_tx.send(request).expect("captured proxy request");
            let body = br#"{"@odata.count":0,"value":[]}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .expect("proxy response header");
            stream.write_all(body).expect("proxy response body");
        });

        let _environment = RestoredProxyEnvironment::use_http_proxy(&format!("http://{address}"));
        let endpoint = Url::parse(&format!("http://127.0.0.1:{direct_target_port}/odata/v1/"))
            .expect("proxy-only OData endpoint");
        let engine = ODataHttpEngine::new(
            endpoint,
            Duration::from_secs(2),
            ProviderNetworkPolicy::system(),
        )
        .expect("system-route engine");
        let response = engine
            .search_page(&query(), &|| false)
            .expect("system route reaches only the local proxy");
        assert_eq!(response.stats.matched_count, Some(0));

        let request = request_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("system route reached proxy");
        let request = String::from_utf8(request).expect("ASCII proxy request");
        let normalized = request.to_ascii_lowercase();
        assert!(request.starts_with(&format!(
            "GET http://127.0.0.1:{direct_target_port}/odata/v1/Products?"
        )));
        assert!(!normalized.contains("\r\nauthorization:"));
        assert!(!normalized.contains("\r\nproxy-authorization:"));
        assert!(!normalized.contains("\r\ncookie:"));
        assert!(!normalized.contains("\r\nreferer:"));
        worker.join().expect("proxy worker");
    }
}
