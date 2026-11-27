use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use insar_core::{
    normalize_asf_features, normalize_ranges, parse_sentinel1_identifier, AsfNormalizeError,
    AsfNormalizeRequest, AsfRawFeature, AsfRemoteSearchRequest, AsfRemoteSearchResponse,
    AsfSceneMetadata, AsfSearchQuery, AsfSearchStats, ContractError, InclusiveRange,
    ProviderResult, SarRemoteSearchRequest, SarRemoteSearchResponse, SarSearchCountRequest,
    SarSearchCountResponse,
};
use serde_json::{Map, Value};

use super::burst::{BurstCmrProvider, BurstSearchLimits};
use super::client::AsfHttpClient;
use super::cmr::{cmr_entry_to_feature, cmr_page_param_variants, collection_ids};
use super::identity::map_and_aggregate_sar_features;
use super::query::{
    apply_remote_count, bisect_window, format_utc_seconds, next_refill_limit, plan_asf_query,
    plan_sar_query, split_query_windows, split_query_windows_for_dataset, AsfQueryPlan,
    QueryWindow,
};

const SPLIT_THRESHOLD: u64 = 2_000;
const SPLIT_WINDOW_LIMIT: u64 = 2_000;
const ALL_RESULTS_BATCH_LIMIT: u64 = 200;
const DIRECT_RESPONSE_LIMIT: u64 = 10_000;
const CMR_PAGE_SIZE: u64 = 200;
const CMR_ENRICH_LIMIT: u64 = 500;
const ENRICH_CHUNK_SIZE: usize = 25;
const MAX_BATCH_SPLIT_REQUESTS: usize = 64;
const MAX_SAR_BATCH_SPLIT_REQUESTS: usize = 256;
const MAX_FINITE_SAR_METADATA_REQUESTS: usize = 8;
const MAX_CMR_PAGES: u64 = 100;

pub trait MetadataTransport: Send + Sync {
    fn asf_geojson(
        &self,
        params: &BTreeMap<String, String>,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value>;

    fn asf_count(
        &self,
        params: &BTreeMap<String, String>,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Option<u64>>;

    fn cmr_json(
        &self,
        params: &[(String, String)],
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value>;

    /// Dedicated UMM-G transport for products whose authoritative mapper
    /// consumes the CMR `hits/items` representation. Existing scripted
    /// transports remain compatible, while production may bind a distinct
    /// endpoint from the classic CMR JSON feed.
    fn cmr_umm_json(
        &self,
        params: &[(String, String)],
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        self.cmr_json(params, cancelled)
    }

    /// One UMM-G page plus the bounded CMR pagination metadata needed by
    /// Search After. Scripted and legacy transports remain page-number based
    /// unless they explicitly advertise Search After support.
    fn cmr_umm_page(
        &self,
        params: &[(String, String)],
        search_after: Option<&str>,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CmrUmmPage> {
        let _ = search_after;
        self.cmr_umm_json(params, cancelled).map(CmrUmmPage::legacy)
    }

    /// One authenticated CMR granule JSON page for the restricted legacy SAR
    /// missions. Production attaches an Earthdata bearer token; scripted
    /// transports answer from fixtures through the same seam as `cmr_json`.
    fn cmr_legacy_json(
        &self,
        params: &[(String, String)],
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        self.cmr_json(params, cancelled)
    }

    /// Authenticated CMR granule count for the restricted legacy SAR
    /// missions, from the `CMR-Hits` response header.
    fn cmr_legacy_count(
        &self,
        params: &[(String, String)],
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Option<u64>> {
        let _ = params;
        let _ = cancelled;
        Ok(None)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CmrUmmPage {
    pub value: Value,
    pub search_after: Option<String>,
    pub timed_out: bool,
    pub search_after_capable: bool,
}

impl CmrUmmPage {
    pub fn legacy(value: Value) -> Self {
        Self {
            value,
            search_after: None,
            timed_out: false,
            search_after_capable: false,
        }
    }
}

impl MetadataTransport for AsfHttpClient {
    fn asf_geojson(
        &self,
        params: &BTreeMap<String, String>,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        AsfHttpClient::asf_geojson(self, params, cancelled)
    }

    fn asf_count(
        &self,
        params: &BTreeMap<String, String>,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Option<u64>> {
        AsfHttpClient::asf_count(self, params, cancelled)
    }

    fn cmr_json(
        &self,
        params: &[(String, String)],
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        AsfHttpClient::cmr_json(self, params, cancelled)
    }

    fn cmr_umm_json(
        &self,
        params: &[(String, String)],
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        AsfHttpClient::cmr_catalog_json(self, params, cancelled)
    }

    fn cmr_legacy_json(
        &self,
        params: &[(String, String)],
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        AsfHttpClient::cmr_legacy_json(self, params, cancelled)
    }

    fn cmr_legacy_count(
        &self,
        params: &[(String, String)],
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Option<u64>> {
        AsfHttpClient::cmr_legacy_count(self, params, cancelled)
    }

    fn cmr_umm_page(
        &self,
        params: &[(String, String)],
        search_after: Option<&str>,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CmrUmmPage> {
        AsfHttpClient::cmr_catalog_page(self, params, search_after, cancelled)
    }
}

pub struct AsfProvider<T> {
    transport: T,
    now: Arc<dyn Fn() -> String + Send + Sync>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SarSearchProviderProgressStage {
    #[default]
    ProviderResponse,
    SplittingTransport,
    AdmittingResults,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SarSearchProviderProgress {
    pub stage: SarSearchProviderProgressStage,
    pub provider_request_count: u64,
    pub transport_part_count: u64,
    pub raw_fetched_count: u64,
    pub mapped_count: u64,
    pub unique_admitted_count: u64,
    pub requested_limit: Option<u64>,
    pub remote_total: Option<u64>,
    /// Compatibility alias for authoritative admitted results.
    pub fetched_count: u64,
    /// Compatibility alias for the honest foreground denominator.
    pub total_count: Option<u64>,
}

impl Default for SarSearchProviderProgress {
    fn default() -> Self {
        Self::work(
            SarSearchProviderProgressStage::ProviderResponse,
            0,
            0,
            0,
            0,
            0,
            None,
            None,
        )
    }
}

impl SarSearchProviderProgress {
    #[allow(clippy::too_many_arguments)]
    fn work(
        stage: SarSearchProviderProgressStage,
        provider_request_count: u64,
        transport_part_count: u64,
        raw_fetched_count: u64,
        mapped_count: u64,
        unique_admitted_count: u64,
        requested_limit: Option<u64>,
        remote_total: Option<u64>,
    ) -> Self {
        let total_count = requested_limit.or_else(|| {
            (unique_admitted_count > 0)
                .then_some(remote_total)
                .flatten()
        });
        Self {
            stage,
            provider_request_count,
            transport_part_count,
            raw_fetched_count,
            mapped_count,
            unique_admitted_count,
            requested_limit,
            remote_total,
            fetched_count: unique_admitted_count,
            total_count,
        }
    }
}

fn raw_feature_start(feature: &AsfRawFeature) -> Option<&str> {
    ["startTime", "acquisitionStart", "start"]
        .into_iter()
        .find_map(|key| feature.properties.get(key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn prioritize_finite_refill_windows(
    windows: Vec<QueryWindow>,
    seed_rows: &[AsfRawFeature],
) -> Vec<QueryWindow> {
    let mut observed = seed_rows.iter().filter_map(raw_feature_start);
    let Some(first) = observed.next() else {
        return windows;
    };
    let (mut minimum, mut maximum) = (first, first);
    for value in observed {
        minimum = minimum.min(value);
        maximum = maximum.max(value);
    }

    // A full-range finite response is normally ordered from one temporal edge.
    // Refill immediately outside the observed interval instead of rescanning
    // years of known-empty archive windows from the dataset-wide boundary.
    let mut older = windows
        .iter()
        .filter(|window| window.end.as_str() < minimum)
        .cloned()
        .collect::<VecDeque<_>>();
    let mut newer = windows
        .iter()
        .rev()
        .filter(|window| window.start.as_str() > maximum)
        .cloned()
        .collect::<VecDeque<_>>();
    let overlap = windows
        .into_iter()
        .filter(|window| window.end.as_str() >= minimum && window.start.as_str() <= maximum);
    let mut prioritized = Vec::new();
    while !older.is_empty() || !newer.is_empty() {
        if let Some(window) = older.pop_front() {
            prioritized.push(window);
        }
        if let Some(window) = newer.pop_front() {
            prioritized.push(window);
        }
    }
    prioritized.extend(overlap);
    prioritized
}

fn coarsen_finite_refill_windows(
    windows: Vec<QueryWindow>,
    max_requests: usize,
) -> Vec<QueryWindow> {
    if max_requests == 0 {
        return Vec::new();
    }
    if windows.len() <= max_requests {
        return windows;
    }
    let chunk_size = windows.len().div_ceil(max_requests);
    windows
        .chunks(chunk_size)
        .map(|chunk| QueryWindow {
            start: chunk
                .last()
                .expect("finite window chunk is non-empty")
                .start
                .clone(),
            end: chunk
                .first()
                .expect("finite window chunk is non-empty")
                .end
                .clone(),
        })
        .collect()
}

impl AsfProvider<AsfHttpClient> {
    pub fn production() -> ProviderResult<Self> {
        Self::production_with_network_policy(insar_core::ProviderNetworkPolicy::system())
    }

    pub fn production_with_network_policy(
        network_policy: insar_core::ProviderNetworkPolicy,
    ) -> ProviderResult<Self> {
        Ok(Self {
            transport: AsfHttpClient::production_with_network_policy(network_policy)?,
            now: Arc::new(system_now),
        })
    }

    /// Production provider whose restricted legacy SAR missions (ERS-1/2,
    /// RADARSAT-1, JERS-1) search CMR with an Earthdata bearer token
    /// resolved by the injected provider. The closure is invoked per legacy
    /// search request; returning `None` makes those searches fail closed
    /// with a login-required typed reason.
    pub fn production_with_legacy_cmr_token(
        network_policy: insar_core::ProviderNetworkPolicy,
        token_provider: Arc<dyn Fn() -> Option<String> + Send + Sync>,
    ) -> ProviderResult<Self> {
        Ok(Self {
            transport: AsfHttpClient::production_with_network_policy(network_policy)?
                .with_legacy_cmr_token_provider(token_provider),
            now: Arc::new(system_now),
        })
    }
}

impl<T> AsfProvider<T> {
    #[doc(hidden)]
    pub fn with_transport(transport: T, now: impl Into<String>) -> Self {
        let now = now.into();
        Self {
            transport,
            now: Arc::new(move || now.clone()),
        }
    }

    pub fn transport(&self) -> &T {
        &self.transport
    }
}

impl<T: MetadataTransport> AsfProvider<T> {
    pub fn count_sar(
        &self,
        request: &SarSearchCountRequest,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<SarSearchCountResponse> {
        request
            .validate()
            .map_err(|reason| search_error("ASF SAR 候选计数请求无效", reason))?;
        if request.query.dataset == "SLC-BURST" {
            return BurstCmrProvider::with_limits(&self.transport, BurstSearchLimits::default())?
                .count(request, cancelled);
        }
        if super::legacy_cmr::is_legacy_cmr_dataset(&request.query.dataset) {
            return self.count_sar_legacy_cmr(&request.query, cancelled);
        }
        check_cancelled(cancelled)?;
        let plan = plan_sar_query(&request.query)?;
        let response = SarSearchCountResponse {
            total_count: self.transport.asf_count(&plan.params, cancelled)?,
        };
        response
            .validate()
            .map_err(|reason| search_error("ASF SAR 候选计数响应无效", reason))?;
        Ok(response)
    }

    /// Count for the restricted legacy missions through authenticated CMR,
    /// read from the `CMR-Hits` response header.
    fn count_sar_legacy_cmr(
        &self,
        query: &insar_core::SarSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<SarSearchCountResponse> {
        check_cancelled(cancelled)?;
        // ASF removed the legacy missions from its searchable interfaces
        // (Owner evidence 2026-10-31), so their counts can never be honest
        // either; reject with the same typed removal reason the search path
        // uses. The rest of the seam stays in place for a future restore.
        plan_sar_query(query)?;
        let bbox = legacy_cmr_bbox(query);
        let Some(params) = super::legacy_cmr::legacy_cmr_count_page(query, bbox.as_ref()) else {
            return Err(search_error(
                "ASF SAR 候选计数请求无效",
                "invalid_legacy_cmr_projection",
            ));
        };
        let total = self.transport.cmr_legacy_count(&params, cancelled)?;
        Ok(SarSearchCountResponse { total_count: total })
    }

    /// Search for the restricted legacy missions (ERS-1/2, RADARSAT-1,
    /// JERS-1) through authenticated CMR. The anonymous `/param` endpoint
    /// hides these restricted collections entirely (verified 2026-10-31), so
    /// this path is the only way results can appear. Pagination walks CMR
    /// pages up to the requested limit; mapping and deduplication stay with
    /// the legacy identity pipeline.
    fn search_sar_legacy_cmr(
        &self,
        request: &SarRemoteSearchRequest,
        cancelled: &(dyn Fn() -> bool + Sync),
        progress: &dyn Fn(SarSearchProviderProgress),
    ) -> ProviderResult<SarRemoteSearchResponse> {
        let query = &request.query;
        check_cancelled(cancelled)?;
        let plan = plan_sar_query(query)?;
        let bbox = legacy_cmr_bbox(query);
        let mut features: Vec<AsfRawFeature> = Vec::new();
        let mut seen = HashSet::new();
        let mut request_count = 0_u64;
        let mut mapped_record_count = 0_u64;
        let mut mapping_error_count = 0_u64;
        let mut page_number = 1_u64;
        let target = plan.requested_limit as usize;
        // Slice 3(a): only a short or empty page proves remote exhaustion. A
        // walk the 10,000 safety ceiling stopped leaves it unproven; meeting a
        // smaller requested or default target is a normal result.
        let mut stopped_by_ceiling = false;
        loop {
            check_cancelled(cancelled)?;
            if page_number > super::legacy_cmr::MAX_LEGACY_CMR_PAGES {
                return Err(search_error(
                    "ASF SAR 检索超过 CMR 分页安全上限",
                    "legacy_cmr_page_limit",
                ));
            }
            let Some(page) = super::legacy_cmr::legacy_cmr_page(query, bbox.as_ref(), page_number)
            else {
                return Err(search_error(
                    "ASF SAR 检索请求无效",
                    "invalid_legacy_cmr_projection",
                ));
            };
            request_count = request_count.saturating_add(1);
            let value = self.transport.cmr_legacy_json(&page.params, cancelled)?;
            check_cancelled(cancelled)?;
            let entries = value
                .get("feed")
                .and_then(|feed| feed.get("entry"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let page_len = entries.len();
            for entry in entries.iter().filter_map(Value::as_object) {
                let Some(feature) = super::legacy_cmr::legacy_cmr_entry_to_feature(entry) else {
                    mapping_error_count = mapping_error_count.saturating_add(1);
                    continue;
                };
                mapped_record_count = mapped_record_count.saturating_add(1);
                // Deduplicate by the feature's identity-bearing properties;
                // CMR pages are stable under one sort key, but overlapping
                // short names (L0/L1 of one mission) never collide on
                // granule identity.
                let identity = feature
                    .properties
                    .get("productID")
                    .or_else(|| feature.properties.get("sceneName"))
                    .cloned()
                    .unwrap_or_else(|| Value::String(format!("page{page_number}")));
                if seen.insert(identity) {
                    features.push(feature);
                }
            }
            let mapped = map_and_aggregate_sar_features(&query.dataset, &features);
            progress(SarSearchProviderProgress::work(
                SarSearchProviderProgressStage::ProviderResponse,
                request_count,
                request_count,
                features.len() as u64,
                mapped_record_count,
                mapped.unique_provider_product_count,
                Some(plan.requested_limit),
                None,
            ));
            if page_len == 0 || (page_len as u64) < super::legacy_cmr::LEGACY_CMR_PAGE_SIZE {
                break;
            }
            if features.len() >= target {
                stopped_by_ceiling = plan.requested_limit >= insar_core::SEARCH_SAFETY_CAP;
                break;
            }
            page_number += 1;
        }
        let mut mapped = map_and_aggregate_sar_features(&query.dataset, &features);
        super::coverage::prioritize_sar_products(&mut mapped.products, query, plan.requested_limit);
        mapped.products.truncate(plan.requested_limit as usize);
        let returned_count = mapped.products.len() as u64;
        let response = SarRemoteSearchResponse {
            products: mapped.products,
            errors: mapped.item_errors.clone(),
            stats: AsfSearchStats {
                requested_limit: plan.requested_limit,
                query_limit: plan.query_limit,
                metadata_request_count: Some(request_count),
                total_count: None,
                candidate_count: features.len() as u64,
                returned_count,
                source: "ASF_CMR".to_string(),
                cmr_enriched: None,
                matched_count: None,
                fetched_record_count: Some(mapped.fetched_record_count),
                replayed_record_count: Some(mapped.replayed_record_count),
                mapped_record_count: Some(mapped_record_count),
                mapping_error_count: Some(
                    mapping_error_count.saturating_add(mapped.mapping_error_count),
                ),
                aggregated_asset_record_count: Some(mapped.aggregated_asset_record_count),
                unique_provider_product_count: Some(mapped.unique_provider_product_count),
                asset_count: Some(mapped.asset_count),
                identity_conflict_count: Some(mapped.identity_conflict_count),
                page_size: Some(super::legacy_cmr::LEGACY_CMR_PAGE_SIZE),
                truncated: Some(
                    stopped_by_ceiling || returned_count < mapped.unique_provider_product_count,
                ),
                item_errors: mapped.item_errors,
                warnings: Vec::new(),
            },
        };
        response
            .validate()
            .map_err(|reason| search_error("ASF SAR 检索结果无效", reason))?;
        Ok(response)
    }

    pub fn search_sar(
        &self,
        request: &SarRemoteSearchRequest,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<SarRemoteSearchResponse> {
        self.search_sar_with_progress(request, cancelled, &|_| {})
    }

    pub fn search_sar_with_progress(
        &self,
        request: &SarRemoteSearchRequest,
        cancelled: &(dyn Fn() -> bool + Sync),
        progress: &dyn Fn(SarSearchProviderProgress),
    ) -> ProviderResult<SarRemoteSearchResponse> {
        let started = Instant::now();
        let result = self.search_sar_with_progress_inner(request, cancelled, progress);
        write_sar_search_diagnostic(request, started, &result);
        result
    }

    fn search_sar_with_progress_inner(
        &self,
        request: &SarRemoteSearchRequest,
        cancelled: &(dyn Fn() -> bool + Sync),
        progress: &dyn Fn(SarSearchProviderProgress),
    ) -> ProviderResult<SarRemoteSearchResponse> {
        request
            .validate()
            .map_err(|reason| search_error("ASF SAR 检索请求无效", reason))?;
        if request.query.dataset == "SLC-BURST" {
            return BurstCmrProvider::with_limits(&self.transport, BurstSearchLimits::default())?
                .search(request, cancelled, progress);
        }
        if super::legacy_cmr::is_legacy_cmr_dataset(&request.query.dataset) {
            return self.search_sar_legacy_cmr(request, cancelled, progress);
        }
        check_cancelled(cancelled)?;
        let mut plan = plan_sar_query(&request.query)?;
        let metadata_query_limit = plan.query_limit;
        let total_count = if plan.should_count {
            match self.transport.asf_count(&plan.params, cancelled) {
                Ok(count) => count,
                Err(error) if error.code == "DL001" => return Err(error),
                Err(error) if plan.requested_was_unbounded => return Err(error),
                Err(_) => None,
            }
        } else {
            None
        };
        if plan.requested_was_unbounded && total_count.is_none() {
            return Err(search_error(
                "ASF 全量检索需要先获得真实候选总量",
                "all_results_count_required",
            ));
        }
        apply_remote_count(&mut plan, total_count);
        if plan.aoi_mode != super::query::AoiPlanMode::None
            && !plan.requested_was_unbounded
            && metadata_query_limit > plan.query_limit
        {
            plan.query_limit = metadata_query_limit;
            plan.params
                .insert("maxResults".to_string(), metadata_query_limit.to_string());
        }
        check_cancelled(cancelled)?;
        if let Some(total_count) =
            total_count.filter(|total| plan.requested_was_unbounded && *total > 0)
        {
            progress(SarSearchProviderProgress::work(
                SarSearchProviderProgressStage::ProviderResponse,
                0,
                0,
                0,
                0,
                0,
                None,
                Some(total_count),
            ));
        }
        let mut source = "ASF";
        let mut metadata_request_count = 0_u64;
        let features = if plan.requested_was_unbounded && total_count == Some(0) {
            source = "ASF_BATCH";
            Vec::new()
        } else if plan.requested_was_unbounded {
            source = "ASF_BATCH";
            self.split_sar_search(
                &request.query,
                &plan,
                ALL_RESULTS_BATCH_LIMIT,
                total_count,
                cancelled,
                progress,
                &mut metadata_request_count,
                Vec::new(),
                MAX_SAR_BATCH_SPLIT_REQUESTS,
            )?
        } else if plan.query_limit > DIRECT_RESPONSE_LIMIT {
            source = "ASF_BATCH";
            self.split_sar_search(
                &request.query,
                &plan,
                SPLIT_WINDOW_LIMIT,
                total_count,
                cancelled,
                progress,
                &mut metadata_request_count,
                Vec::new(),
                MAX_FINITE_SAR_METADATA_REQUESTS,
            )?
        } else {
            metadata_request_count = metadata_request_count.saturating_add(1);
            let direct = self.transport.asf_geojson(&plan.params, cancelled);
            match direct {
                Ok(value) => {
                    let rows = raw_features(&value);
                    let direct_mapped =
                        map_and_aggregate_sar_features(&request.query.dataset, &rows);
                    progress(SarSearchProviderProgress::work(
                        SarSearchProviderProgressStage::ProviderResponse,
                        metadata_request_count,
                        1,
                        rows.len() as u64,
                        direct_mapped.mapped_record_count,
                        direct_mapped.unique_provider_product_count,
                        (!plan.requested_was_unbounded).then_some(plan.requested_limit),
                        total_count,
                    ));
                    let expected = plan
                        .query_limit
                        .min(total_count.unwrap_or(plan.query_limit));
                    let truncated = plan.requested_limit >= SPLIT_THRESHOLD
                        && rows.len() < plan.requested_limit as usize
                        && rows.len() < expected as usize;
                    if truncated {
                        source = "ASF_BATCH";
                        self.split_sar_search(
                            &request.query,
                            &plan,
                            SPLIT_WINDOW_LIMIT,
                            total_count,
                            cancelled,
                            progress,
                            &mut metadata_request_count,
                            rows,
                            MAX_FINITE_SAR_METADATA_REQUESTS.saturating_sub(1),
                        )?
                    } else {
                        rows
                    }
                }
                Err(error) if error.code == "DL001" => return Err(error),
                Err(error)
                    if plan.requested_limit >= SPLIT_THRESHOLD
                        && is_split_recoverable_error(&error) =>
                {
                    source = "ASF_BATCH";
                    self.split_sar_search(
                        &request.query,
                        &plan,
                        SPLIT_WINDOW_LIMIT,
                        total_count,
                        cancelled,
                        progress,
                        &mut metadata_request_count,
                        Vec::new(),
                        MAX_FINITE_SAR_METADATA_REQUESTS.saturating_sub(1),
                    )?
                }
                Err(_error) if plan.requested_limit >= SPLIT_THRESHOLD => {
                    source = "ASF_BATCH";
                    self.split_sar_search(
                        &request.query,
                        &plan,
                        SPLIT_WINDOW_LIMIT,
                        total_count,
                        cancelled,
                        progress,
                        &mut metadata_request_count,
                        Vec::new(),
                        MAX_FINITE_SAR_METADATA_REQUESTS.saturating_sub(1),
                    )?
                }
                Err(error) => return Err(error),
            }
        };
        check_cancelled(cancelled)?;
        let mut mapped = map_and_aggregate_sar_features(&request.query.dataset, &features);
        if !plan.requested_was_unbounded {
            let admitted = u64::try_from(mapped.products.len())
                .unwrap_or(u64::MAX)
                .min(plan.requested_limit);
            progress(SarSearchProviderProgress::work(
                SarSearchProviderProgressStage::AdmittingResults,
                metadata_request_count,
                metadata_request_count,
                mapped.fetched_record_count,
                mapped.mapped_record_count,
                admitted,
                Some(plan.requested_limit),
                total_count,
            ));
        }
        let unique_provider_product_count = mapped.unique_provider_product_count;
        super::coverage::prioritize_sar_products(
            &mut mapped.products,
            &request.query,
            plan.requested_limit,
        );
        mapped.products.truncate(plan.requested_limit as usize);
        let returned_count = mapped.products.len() as u64;
        // Slice 3(a): without a remote total, a window filled at the 10,000
        // safety ceiling does not prove the remote side was exhausted.
        let truncated = returned_count < unique_provider_product_count
            || total_count.map_or(
                !plan.requested_was_unbounded
                    && plan.requested_limit >= insar_core::SEARCH_SAFETY_CAP
                    && features.len() as u64 >= plan.query_limit,
                |matched| returned_count < matched,
            );
        let response = SarRemoteSearchResponse {
            products: mapped.products,
            errors: mapped.item_errors.clone(),
            stats: AsfSearchStats {
                requested_limit: plan.requested_limit,
                query_limit: plan.query_limit,
                metadata_request_count: Some(metadata_request_count),
                total_count,
                candidate_count: features.len() as u64,
                returned_count,
                source: source.to_string(),
                cmr_enriched: None,
                matched_count: total_count,
                fetched_record_count: Some(mapped.fetched_record_count),
                replayed_record_count: Some(mapped.replayed_record_count),
                mapped_record_count: Some(mapped.mapped_record_count),
                mapping_error_count: Some(mapped.mapping_error_count),
                aggregated_asset_record_count: Some(mapped.aggregated_asset_record_count),
                unique_provider_product_count: Some(unique_provider_product_count),
                asset_count: Some(mapped.asset_count),
                identity_conflict_count: Some(mapped.identity_conflict_count),
                page_size: Some(if source == "ASF_BATCH" {
                    if plan.requested_was_unbounded {
                        ALL_RESULTS_BATCH_LIMIT
                    } else {
                        SPLIT_WINDOW_LIMIT
                    }
                } else {
                    plan.query_limit
                }),
                truncated: Some(truncated),
                item_errors: mapped.item_errors,
                warnings: Vec::new(),
            },
        };
        response
            .validate()
            .map_err(|reason| search_error("ASF SAR 检索结果无效", reason))?;
        Ok(response)
    }

    #[allow(clippy::too_many_arguments)]
    fn split_sar_search(
        &self,
        query: &insar_core::SarSearchQuery,
        plan: &AsfQueryPlan,
        batch_limit: u64,
        total_count: Option<u64>,
        cancelled: &(dyn Fn() -> bool + Sync),
        progress: &dyn Fn(SarSearchProviderProgress),
        metadata_request_count: &mut u64,
        seed_rows: Vec<AsfRawFeature>,
        max_requests: usize,
    ) -> ProviderResult<Vec<AsfRawFeature>> {
        let now = (self.now)();
        let windows = split_query_windows_for_dataset(
            &query.dataset,
            query.start.as_deref(),
            query.end.as_deref(),
            &now,
        )?;
        let windows = if plan.requested_was_unbounded {
            windows
        } else {
            coarsen_finite_refill_windows(windows, max_requests)
        };
        let mut queue = VecDeque::from(if plan.requested_was_unbounded || seed_rows.is_empty() {
            windows
        } else {
            prioritize_finite_refill_windows(windows, &seed_rows)
        });
        let target = if plan.requested_was_unbounded {
            plan.query_limit.max(plan.requested_limit)
        } else {
            plan.requested_limit
        } as usize;
        let fetch_budget = plan.query_limit.max(plan.requested_limit) as usize;
        let mut merged = seed_rows;
        let mut raw_fetched_count = merged.len() as u64;
        let mut request_count = 0;
        while let Some(window) = queue.pop_front() {
            check_cancelled(cancelled)?;
            if !plan.requested_was_unbounded && merged.len() >= fetch_budget {
                break;
            }
            if request_count >= max_requests {
                if plan.requested_was_unbounded {
                    return Err(search_error(
                        "ASF SAR 分段检索超过安全请求上限",
                        "split_request_limit",
                    ));
                }
                break;
            }
            request_count += 1;
            let remaining_budget = fetch_budget.saturating_sub(merged.len()) as u64;
            let limit = batch_limit.min(if plan.requested_was_unbounded {
                target as u64
            } else {
                remaining_budget
            });
            if limit == 0 {
                break;
            }
            let mut params = plan.params.clone();
            params.insert("start".to_string(), window.start.clone());
            params.insert("end".to_string(), window.end.clone());
            params.insert("maxResults".to_string(), limit.to_string());
            *metadata_request_count = metadata_request_count.saturating_add(1);
            let rows = match self.transport.asf_geojson(&params, cancelled) {
                Ok(value) => raw_features(&value),
                Err(error) if error.code == "DL001" => return Err(error),
                Err(error) if is_split_recoverable_error(&error) => {
                    let subwindows = bisect_window(&window)?;
                    if subwindows.is_empty() {
                        return Err(error);
                    }
                    for subwindow in subwindows.into_iter().rev() {
                        queue.push_front(subwindow);
                    }
                    progress(SarSearchProviderProgress::work(
                        SarSearchProviderProgressStage::SplittingTransport,
                        *metadata_request_count,
                        request_count as u64,
                        raw_fetched_count,
                        map_and_aggregate_sar_features(&query.dataset, &merged).mapped_record_count,
                        map_and_aggregate_sar_features(&query.dataset, &merged)
                            .unique_provider_product_count,
                        (!plan.requested_was_unbounded).then_some(plan.requested_limit),
                        total_count,
                    ));
                    continue;
                }
                Err(error) => return Err(error),
            };
            raw_fetched_count = raw_fetched_count.saturating_add(rows.len() as u64);
            let admitted_before = map_and_aggregate_sar_features(&query.dataset, &merged);
            progress(SarSearchProviderProgress::work(
                SarSearchProviderProgressStage::ProviderResponse,
                *metadata_request_count,
                request_count as u64,
                raw_fetched_count,
                admitted_before.mapped_record_count,
                admitted_before.unique_provider_product_count,
                (!plan.requested_was_unbounded).then_some(plan.requested_limit),
                total_count,
            ));
            let saturated = rows.len() >= limit as usize;
            if saturated && plan.requested_was_unbounded {
                let subwindows = bisect_window(&window)?;
                if !subwindows.is_empty() {
                    for subwindow in subwindows.into_iter().rev() {
                        queue.push_front(subwindow);
                    }
                    progress(SarSearchProviderProgress::work(
                        SarSearchProviderProgressStage::SplittingTransport,
                        *metadata_request_count,
                        request_count as u64,
                        raw_fetched_count,
                        admitted_before.mapped_record_count,
                        admitted_before.unique_provider_product_count,
                        None,
                        total_count,
                    ));
                    continue;
                }
            }
            merged.extend(rows.into_iter().take(remaining_budget as usize));
            let fetched = map_and_aggregate_sar_features(&query.dataset, &merged)
                .products
                .len()
                .min(target);
            let progress_total = if plan.requested_was_unbounded {
                total_count
            } else {
                Some(plan.requested_limit)
            };
            let admitted = map_and_aggregate_sar_features(&query.dataset, &merged);
            if progress_total.is_some() || plan.requested_was_unbounded {
                progress(SarSearchProviderProgress::work(
                    SarSearchProviderProgressStage::AdmittingResults,
                    *metadata_request_count,
                    request_count as u64,
                    raw_fetched_count,
                    admitted.mapped_record_count,
                    u64::try_from(fetched).unwrap_or(u64::MAX),
                    (!plan.requested_was_unbounded).then_some(plan.requested_limit),
                    total_count,
                ));
            }
            if fetched >= target {
                break;
            }
        }
        Ok(merged)
    }

    pub fn search(
        &self,
        request: &AsfRemoteSearchRequest,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<AsfRemoteSearchResponse> {
        request
            .validate()
            .map_err(|reason| search_error("ASF 检索请求无效", reason))?;
        check_cancelled(cancelled)?;
        let mut plan = plan_asf_query(&request.query)?;
        let total_count = if plan.should_count {
            match self.transport.asf_count(&plan.params, cancelled) {
                Ok(count) => count,
                Err(error) if error.code == "DL001" => return Err(error),
                Err(_) => None,
            }
        } else {
            None
        };
        check_cancelled(cancelled)?;
        apply_remote_count(&mut plan, total_count);

        let mut source = "ASF";
        let mut features = None;
        let mut primary_error = None;
        if plan.query_limit > DIRECT_RESPONSE_LIMIT {
            match self.split_search(&request.query, &plan, cancelled) {
                Ok(split) => {
                    features = Some(split);
                    source = "ASF_BATCH";
                }
                Err(error) => primary_error = Some(error),
            }
        } else {
            let direct = self.transport.asf_geojson(&plan.params, cancelled);
            check_cancelled(cancelled)?;
            match direct {
                Ok(value) => {
                    let direct_features = raw_features(&value);
                    let expected = plan
                        .query_limit
                        .min(total_count.unwrap_or(plan.query_limit));
                    let truncated = plan.requested_limit >= SPLIT_THRESHOLD
                        && direct_features.len() < plan.requested_limit as usize
                        && direct_features.len() < expected as usize;
                    if truncated {
                        match self.split_search(&request.query, &plan, cancelled) {
                            Ok(split) => {
                                features = Some(split);
                                source = "ASF_BATCH";
                            }
                            Err(error) => primary_error = Some(error),
                        }
                    } else {
                        features = Some(direct_features);
                    }
                }
                Err(error) if error.code == "DL001" => return Err(error),
                Err(error) => {
                    primary_error = Some(error);
                    if plan.requested_limit >= SPLIT_THRESHOLD {
                        match self.split_search(&request.query, &plan, cancelled) {
                            Ok(split) => {
                                features = Some(split);
                                source = "ASF_BATCH";
                            }
                            Err(error) => primary_error = Some(error),
                        }
                    }
                }
            }
        }
        check_cancelled(cancelled)?;

        let Some(mut features) = features else {
            let error = primary_error
                .unwrap_or_else(|| search_error("ASF 元数据检索未完成", "missing_search_result"));
            if !request.allow_cmr_fallback {
                return Err(error);
            }
            return self.search_cmr(request, plan, total_count, cancelled);
        };

        let mut normalized = normalize(&request.query, &features, false)?;
        let mut refill_attempts = 0;
        while source == "ASF"
            && plan.aoi_mode.as_str() != "none"
            && plan.requested_limit > 1
            && normalized.scenes.len() < plan.requested_limit as usize
            && features.len() >= plan.query_limit as usize
            && refill_attempts < 3
        {
            let Some(next_limit) = next_refill_limit(plan.query_limit, total_count) else {
                break;
            };
            refill_attempts += 1;
            plan.query_limit = next_limit;
            plan.params
                .insert("maxResults".to_string(), next_limit.to_string());
            check_cancelled(cancelled)?;
            let value = self.transport.asf_geojson(&plan.params, cancelled)?;
            check_cancelled(cancelled)?;
            features = raw_features(&value);
            normalized = normalize(&request.query, &features, false)?;
        }
        publish_response(normalized, plan, total_count, source, None, cancelled)
    }

    fn split_search(
        &self,
        query: &AsfSearchQuery,
        plan: &AsfQueryPlan,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Vec<AsfRawFeature>> {
        let now = (self.now)();
        let mut queue = VecDeque::from(split_query_windows(
            Some(&query.start),
            Some(&query.end),
            &now,
        )?);
        let target = plan.query_limit.max(plan.requested_limit) as usize;
        let mut merged = Vec::new();
        let mut merged_indices = HashMap::new();
        let mut request_count = 0;
        while let Some(window) = queue.pop_front() {
            check_cancelled(cancelled)?;
            if merged.len() >= target {
                break;
            }
            request_count += 1;
            if request_count > MAX_BATCH_SPLIT_REQUESTS {
                return Err(search_error(
                    "ASF 分段检索超过安全请求上限",
                    "split_request_limit",
                ));
            }
            let limit = SPLIT_WINDOW_LIMIT.min(target as u64);
            let mut params = plan.params.clone();
            params.insert("start".to_string(), window.start.clone());
            params.insert("end".to_string(), window.end.clone());
            params.insert("maxResults".to_string(), limit.to_string());
            let value = self.transport.asf_geojson(&params, cancelled)?;
            check_cancelled(cancelled)?;
            let features = raw_features(&value);
            if features.len() >= limit as usize {
                let subwindows = bisect_window(&window)?;
                if !subwindows.is_empty() {
                    for subwindow in subwindows.into_iter().rev() {
                        queue.push_front(subwindow);
                    }
                    continue;
                }
            }
            merge_feature_rows(&mut merged, &mut merged_indices, features);
        }
        Ok(merged)
    }

    fn search_cmr(
        &self,
        request: &AsfRemoteSearchRequest,
        plan: AsfQueryPlan,
        total_count: Option<u64>,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<AsfRemoteSearchResponse> {
        if collection_ids(&request.query.product_type).is_none() {
            return Err(search_error(
                "CMR 备用检索暂不支持该产品类型",
                "unsupported_cmr_product",
            ));
        }
        let Some(probe_variants) = cmr_page_param_variants(&request.query, 1, 1) else {
            return Err(cmr_projection_error());
        };
        if probe_variants.is_empty() || probe_variants.len() > 2 {
            return Err(search_error(
                "CMR 备用检索空间窗口无效",
                "invalid_cmr_spatial_variants",
            ));
        }
        let ignore_direction = plan.requested_limit > CMR_ENRICH_LIMIT;
        let mut features = Vec::new();
        let mut feature_indices = HashMap::new();
        let mut page_number = 1;
        let mut active_variants = vec![true; probe_variants.len()];
        loop {
            check_cancelled(cancelled)?;
            if page_number > MAX_CMR_PAGES {
                return Err(search_error(
                    "CMR 备用检索超过安全分页上限",
                    "cmr_page_limit",
                ));
            }
            let page_size = CMR_PAGE_SIZE;
            let variants = cmr_page_param_variants(&request.query, page_size, page_number)
                .expect("CMR product was checked");
            if variants.len() != active_variants.len() {
                return Err(search_error(
                    "CMR 备用检索空间窗口不稳定",
                    "unstable_cmr_spatial_variants",
                ));
            }
            for (variant_index, params) in variants.into_iter().enumerate() {
                if !active_variants[variant_index] {
                    continue;
                }
                check_cancelled(cancelled)?;
                let value = self.transport.cmr_json(&params, cancelled)?;
                check_cancelled(cancelled)?;
                let entries = value
                    .get("feed")
                    .and_then(|feed| feed.get("entry"))
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                active_variants[variant_index] = entries.len() >= page_size as usize;
                merge_canonical_feature_rows_into(
                    &mut features,
                    &mut feature_indices,
                    entries
                        .iter()
                        .filter_map(Value::as_object)
                        .filter_map(cmr_entry_to_feature),
                );
            }
            let current = normalize(&request.query, &features, ignore_direction)?;
            if current.scenes.len() >= plan.query_limit as usize
                || active_variants.iter().all(|active| !active)
            {
                break;
            }
            page_number += 1;
        }

        let enriched = plan.requested_limit <= CMR_ENRICH_LIMIT;
        if enriched && !features.is_empty() {
            features =
                self.enrich_cmr_features(features, &request.query.product_type, cancelled)?;
        }
        check_cancelled(cancelled)?;
        let normalized = normalize(&request.query, &features, !enriched)?;
        publish_response(
            normalized,
            plan,
            total_count,
            "CMR",
            Some(enriched),
            cancelled,
        )
    }

    fn enrich_cmr_features(
        &self,
        features: Vec<AsfRawFeature>,
        product_type: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Vec<AsfRawFeature>> {
        let mut merged = features;
        let indices = merged
            .iter()
            .enumerate()
            .filter_map(|(index, feature)| feature_key(feature).map(|key| (key, index)))
            .collect::<HashMap<_, _>>();
        let mut query_ids = Vec::new();
        for feature in &merged {
            let Some(identifier) = feature_key(feature) else {
                continue;
            };
            match product_type.trim().to_ascii_uppercase().as_str() {
                "SLC" => {
                    query_ids.push(format!("{identifier}-SLC"));
                    query_ids.push(identifier);
                }
                "GRD" | "RAW" | "OCN" => {
                    query_ids.push(identifier.clone());
                    query_ids.push(format!(
                        "{identifier}-{}",
                        product_type.trim().to_ascii_uppercase()
                    ));
                }
                _ => query_ids.push(identifier),
            }
        }
        let mut seen = HashSet::new();
        query_ids.retain(|identifier| seen.insert(identifier.clone()));
        for chunk in query_ids.chunks(ENRICH_CHUNK_SIZE) {
            check_cancelled(cancelled)?;
            let params = BTreeMap::from([
                ("granule_list".to_string(), chunk.join(",")),
                ("output".to_string(), "geojson".to_string()),
            ]);
            match self.transport.asf_geojson(&params, cancelled) {
                Ok(value) => {
                    for remote in raw_features(&value) {
                        if let Some(key) = feature_key(&remote) {
                            if let Some(index) = indices.get(&key) {
                                merge_feature(&mut merged[*index], remote);
                            }
                        }
                    }
                }
                Err(error) if error.code == "DL001" => return Err(error),
                Err(_) => return Ok(merged),
            }
        }
        Ok(merged)
    }
}

struct Normalized {
    scenes: Vec<AsfSceneMetadata>,
    errors: Vec<AsfNormalizeError>,
    candidate_count: usize,
}

fn normalize(
    query: &AsfSearchQuery,
    features: &[AsfRawFeature],
    ignore_direction: bool,
) -> ProviderResult<Normalized> {
    let mut normalization_query = query.clone();
    normalization_query.max_results = None;
    if ignore_direction {
        normalization_query.orbit_direction.clear();
    }
    let response = normalize_asf_features(&AsfNormalizeRequest {
        query: normalization_query,
        features: features.to_vec(),
    })?;
    let mut seen = HashSet::new();
    let scenes = response
        .scenes
        .into_iter()
        .filter(|scene| exact_scene_filter(scene, query, ignore_direction))
        .filter(|scene| seen.insert(scene.scene_id.clone()))
        .collect::<Vec<_>>();
    let candidate_count = scenes.len();
    Ok(Normalized {
        scenes,
        errors: response.errors,
        candidate_count,
    })
}

fn exact_scene_filter(
    scene: &AsfSceneMetadata,
    query: &AsfSearchQuery,
    ignore_direction: bool,
) -> bool {
    let acquired = scene.acquisition_datetime.get(..10).unwrap_or_default();
    let start = query.start.get(..10).unwrap_or(query.start.as_str());
    let end = query.end.get(..10).unwrap_or(query.end.as_str());
    if (!start.is_empty() && !acquired.is_empty() && acquired < start)
        || (!end.is_empty() && !acquired.is_empty() && acquired > end)
    {
        return false;
    }
    let beam = filter_values(&query.beam_mode);
    let polarization = filter_values(&query.polarization);
    let direction = filter_values(&query.orbit_direction);
    matches_processing_level(scene, &query.product_type)
        && (beam.is_empty() || beam.contains(&scene.beam_mode.to_ascii_uppercase()))
        && (polarization.is_empty()
            || polarization.contains(&scene.polarization.to_ascii_uppercase()))
        && (ignore_direction
            || direction.is_empty()
            || scene
                .orbit_direction
                .as_ref()
                .is_some_and(|value| direction.contains(&value.to_ascii_uppercase())))
        && in_ranges(
            scene.path,
            &normalize_ranges(query.relative_orbit.as_deref().unwrap_or("")),
        )
        && in_ranges(
            scene.frame,
            &normalize_ranges(query.frame.as_deref().unwrap_or("")),
        )
}

fn matches_processing_level(scene: &AsfSceneMetadata, requested: &str) -> bool {
    let requested = requested.trim().to_ascii_uppercase();
    let expected = match requested.as_str() {
        "GRD_HD" => Some(("H", true)),
        "GRD_HS" => Some(("H", false)),
        "GRD_MD" => Some(("M", true)),
        "GRD_MS" => Some(("M", false)),
        "GRD_FD" => Some(("F", true)),
        _ => None,
    };
    let Some((resolution, dual)) = expected else {
        return requested.eq_ignore_ascii_case(&scene.product_type);
    };
    let Ok(parsed) = parse_sentinel1_identifier(&scene.scene_id) else {
        return false;
    };
    parsed.product_type == "GRD"
        && parsed.resolution == resolution
        && parsed
            .polarization
            .starts_with(if dual { 'D' } else { 'S' })
}

fn filter_values(value: &str) -> HashSet<String> {
    value
        .split([',', ';', '|'])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_uppercase)
        .collect()
}

fn in_ranges(value: Option<u64>, ranges: &[InclusiveRange]) -> bool {
    ranges.is_empty()
        || value.is_some_and(|value| {
            ranges
                .iter()
                .any(|range| range.start <= value && value <= range.end)
        })
}

fn publish_response(
    mut normalized: Normalized,
    plan: AsfQueryPlan,
    total_count: Option<u64>,
    source: &str,
    cmr_enriched: Option<bool>,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> ProviderResult<AsfRemoteSearchResponse> {
    check_cancelled(cancelled)?;
    let candidate_count = total_count.unwrap_or(normalized.candidate_count as u64);
    normalized.scenes.truncate(plan.requested_limit as usize);
    let response = AsfRemoteSearchResponse {
        stats: AsfSearchStats {
            requested_limit: plan.requested_limit,
            query_limit: plan.query_limit,
            metadata_request_count: None,
            total_count,
            candidate_count,
            returned_count: normalized.scenes.len() as u64,
            source: source.to_string(),
            cmr_enriched,
            matched_count: None,
            fetched_record_count: None,
            replayed_record_count: None,
            mapped_record_count: None,
            mapping_error_count: None,
            aggregated_asset_record_count: None,
            unique_provider_product_count: None,
            asset_count: None,
            identity_conflict_count: None,
            page_size: None,
            truncated: None,
            item_errors: Vec::new(),
            warnings: Vec::new(),
        },
        scenes: normalized.scenes,
        errors: normalized.errors,
    };
    check_cancelled(cancelled)?;
    Ok(response)
}

fn raw_features(value: &Value) -> Vec<AsfRawFeature> {
    value
        .get("features")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .map(|feature| AsfRawFeature {
            r#type: feature
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("Feature")
                .to_string(),
            properties: feature
                .get("properties")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default(),
            geometry: feature.get("geometry").and_then(Value::as_object).cloned(),
        })
        .collect()
}

fn feature_key(feature: &AsfRawFeature) -> Option<String> {
    let source = ["sceneName", "fileName", "fileID", "granuleName"]
        .iter()
        .find_map(|name| feature.properties.get(*name).and_then(Value::as_str))?;
    if let Ok(parsed) = parse_sentinel1_identifier(source) {
        return Some(parsed.scene_id.to_ascii_uppercase());
    }
    let mut key = source
        .trim()
        .replace('\\', "/")
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_string();
    if key.to_ascii_lowercase().ends_with(".zip") {
        key.truncate(key.len() - 4);
    }
    for suffix in ["-SLC", "-GRD", "-RAW", "-OCN"] {
        if key.ends_with(suffix) {
            key.truncate(key.len() - suffix.len());
            break;
        }
    }
    (!key.is_empty()).then_some(key)
}

fn feature_rank(feature: &AsfRawFeature) -> u8 {
    let level = feature
        .properties
        .get("processingLevel")
        .and_then(Value::as_str)
        .unwrap_or("");
    let granule = feature
        .properties
        .get("granuleType")
        .and_then(Value::as_str)
        .unwrap_or("");
    let file = feature
        .properties
        .get("fileName")
        .and_then(Value::as_str)
        .unwrap_or("");
    u8::from(level.eq_ignore_ascii_case("SLC")) * 2
        + u8::from(granule.to_ascii_uppercase().contains("FRAME")) * 2
        + u8::from(file.to_ascii_lowercase().ends_with(".zip"))
}

fn merge_feature_rows(
    target: &mut Vec<AsfRawFeature>,
    indices: &mut HashMap<String, usize>,
    features: Vec<AsfRawFeature>,
) {
    for feature in features {
        let Some(key) = feature_key(&feature) else {
            continue;
        };
        if let Some(index) = indices.get(&key).copied() {
            if feature_rank(&feature) >= feature_rank(&target[index]) {
                target[index] = feature;
            }
        } else {
            indices.insert(key, target.len());
            target.push(feature);
        }
    }
}

fn merge_canonical_feature_rows_into(
    merged: &mut Vec<AsfRawFeature>,
    indices: &mut HashMap<String, usize>,
    features: impl IntoIterator<Item = AsfRawFeature>,
) {
    for feature in features {
        let Some(key) = feature_key(&feature) else {
            merged.push(feature);
            continue;
        };
        if let Some(index) = indices.get(&key).copied() {
            merge_feature(&mut merged[index], feature);
        } else {
            indices.insert(key, merged.len());
            merged.push(feature);
        }
    }
}

fn merge_feature(target: &mut AsfRawFeature, remote: AsfRawFeature) {
    for (key, value) in remote.properties {
        let identity_already_present = matches!(
            key.as_str(),
            "sceneName" | "fileName" | "fileID" | "granuleName"
        ) && target.properties.contains_key(&key);
        if !identity_already_present {
            target.properties.insert(key, value);
        }
    }
    if remote.geometry.is_some() {
        target.geometry = remote.geometry;
    }
}

fn check_cancelled(cancelled: &(dyn Fn() -> bool + Sync)) -> ProviderResult<()> {
    if cancelled() {
        Err(ContractError::new(
            "DL001",
            "ASF 检索已停止",
            Map::from_iter([("reason".to_string(), Value::String("cancelled".to_string()))]),
        )
        .expect("bounded cancellation context is safe"))
    } else {
        Ok(())
    }
}

fn split_recovery_allowed(error: &ContractError) -> bool {
    match error.debug_context.get("status").and_then(Value::as_u64) {
        Some(408 | 425 | 429) => true,
        Some(400..=499) => false,
        _ => true,
    }
}

fn is_split_recoverable_error(error: &ContractError) -> bool {
    matches!(
        error.code.as_str(),
        "ASF002"
            | "ASF_RATE_LIMITED"
            | "ASF_SERVER_ERROR"
            | "ASF_TIMEOUT"
            | "ASF_TRANSPORT_FAILED"
            | "ASF_RESPONSE_TOO_LARGE"
    ) && split_recovery_allowed(error)
}

fn asf_search_diagnostic_enabled() -> bool {
    std::env::var("INSAR_DIAGNOSTIC_ASF_SEARCH")
        .ok()
        .is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes"
            )
        })
}

fn coordinate_pair_count(value: &Value) -> u64 {
    let Some(items) = value.as_array() else {
        return 0;
    };
    if items.len() >= 2 && items[0].is_number() && items[1].is_number() {
        return 1;
    }
    items.iter().map(coordinate_pair_count).sum()
}

fn sar_aoi_summary(request: &SarRemoteSearchRequest) -> (&str, u64) {
    if let Some(aoi) = &request.query.aoi_geojson {
        let geometry = aoi
            .get("geometry")
            .and_then(Value::as_object)
            .unwrap_or(aoi);
        let geometry_type = geometry
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("GeoJSON");
        let vertices = geometry
            .get("coordinates")
            .map(coordinate_pair_count)
            .unwrap_or(0);
        return (geometry_type, vertices);
    }
    if request.query.bbox.is_some() {
        return ("BBox", 5);
    }
    if let Some(wkt) = request.query.aoi_wkt.as_deref() {
        let geometry_type = wkt.split([' ', '(']).next().unwrap_or("WKT");
        let vertices =
            u64::try_from(wkt.matches(',').count().saturating_add(1)).unwrap_or(u64::MAX);
        return (geometry_type, vertices);
    }
    ("None", 0)
}

fn write_sar_search_diagnostic(
    request: &SarRemoteSearchRequest,
    started: Instant,
    result: &ProviderResult<SarRemoteSearchResponse>,
) {
    if !asf_search_diagnostic_enabled() {
        return;
    }
    let (aoi_geometry_type, aoi_vertex_count) = sar_aoi_summary(request);
    let planned_count_requests = plan_sar_query(&request.query)
        .ok()
        .map(|plan| u64::from(plan.should_count));
    let stats = result.as_ref().ok().map(|response| &response.stats);
    let event = serde_json::json!({
        "event": "asf_sar_search_finished",
        "dataset": request.query.dataset,
        "aoi_geometry_type": aoi_geometry_type,
        "aoi_vertex_count": aoi_vertex_count,
        "requested_n": request.query.max_results,
        "count_request_count": planned_count_requests,
        "metadata_request_count": stats.and_then(|value| value.metadata_request_count),
        "raw_fetched_count": stats
            .and_then(|value| value.fetched_record_count)
            .or_else(|| stats.map(|value| value.candidate_count)),
        "mapped_count": stats.and_then(|value| value.mapped_record_count),
        "unique_count": stats.and_then(|value| value.unique_provider_product_count),
        "returned_count": stats.map(|value| value.returned_count),
        "elapsed_ms": u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        "error_code": result.as_ref().err().map(|error| error.code.as_str()),
    });
    eprintln!("INSAR_ASF_SEARCH_DIAGNOSTIC {event}");
}

fn search_error(message: &str, reason: impl Into<String>) -> ContractError {
    ContractError::new(
        "ASF002",
        message,
        Map::from_iter([("reason".to_string(), Value::String(reason.into()))]),
    )
    .expect("bounded search context is safe")
}

/// Single bounding box for the legacy CMR projection, derived with the same
/// AOI authority as the `/param` path. A wrapped AOI with two request windows
/// (antimeridian) uses the combined conservative envelope; exact geometric
/// filtering stays with the legacy normalization stage.
fn legacy_cmr_bbox(query: &insar_core::SarSearchQuery) -> Option<insar_core::BBox> {
    if let Some(aoi) = query.aoi_geojson.as_ref() {
        let geometry = insar_core::validated_polygonal_geojson(aoi)?;
        if let Some(envelope) = insar_core::wrapped_aoi_envelope(&geometry) {
            let mut west = f64::MAX;
            let mut east = f64::MIN;
            let mut south = f64::MAX;
            let mut north = f64::MIN;
            for window in &envelope.request_windows {
                west = west.min(window.west);
                east = east.max(window.east);
                south = south.min(window.south);
                north = north.max(window.north);
            }
            return (west <= east && south <= north).then_some(insar_core::BBox {
                west,
                east,
                south,
                north,
                crs: "EPSG:4326".to_string(),
            });
        }
        let rect = geo::BoundingRect::bounding_rect(&geometry)?;
        return Some(insar_core::BBox {
            west: rect.min().x,
            east: rect.max().x,
            south: rect.min().y,
            north: rect.max().y,
            crs: "EPSG:4326".to_string(),
        });
    }
    query.bbox.clone()
}

fn cmr_projection_error() -> ContractError {
    ContractError::new(
        "ASF_QUERY_UNSUPPORTED",
        "CMR 备用检索无法为当前 AOI 证明安全的有界空间投影",
        Map::from_iter([(
            "reason".to_string(),
            Value::String("aoi_bbox_projection_unavailable".to_string()),
        )]),
    )
    .expect("bounded CMR projection context is safe")
}

fn system_now() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .min(i64::MAX as u64) as i64;
    format_utc_seconds(seconds)
}
