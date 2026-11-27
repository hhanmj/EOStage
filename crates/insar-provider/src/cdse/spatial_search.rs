//! Compact request geometry is a transport detail, never the user's AOI.
use std::collections::BTreeMap;
use std::time::Instant;

use insar_core::{CatalogSearchStats, ProviderProduct};

use super::*;
use crate::catalog::spatial::{
    authoritative_catalogue_geometry, bounded_catalogue_request_tiles,
    order_request_tiles_by_exact_overlap, PreparedCatalogueAoi,
};
use crate::cdse::odata_mapper::{map_record, sentinel2_collection};
use crate::cdse::odata_types::ODataProduct;
use crate::cdse::query::decode_sentinel2_search_facts_for_use;

const MAX_SPATIAL_PARTS: usize = 8;
const MAX_SEARCH_BYTES: u64 = 64 * 1024 * 1024;
/// One compact-AOI logical search pages several request rectangles, each
/// page bounded by the engine's per-request timeout like the ordinary path.
/// The whole search still shares one deadline (never restarted per part), a
/// bounded multiple of the per-request timeout: live CDSE `$count` pages over
/// a province-sized rectangle were observed at 13–38 s each (2026-11-13), so
/// a single per-request timeout for every part could not fill a target.
const SPATIAL_DEADLINE_REQUEST_MULTIPLE: u32 = 4;

pub(super) fn can_replan(query: &CatalogSearchQuery, error: &ContractError) -> bool {
    crate::cdse::query::can_replan_aoi(query, error)
}

struct SpatialBudget {
    deadline: Instant,
    pages_left: usize,
    records_left: u64,
    bytes_left: u64,
}

impl SpatialBudget {
    fn new(started: Instant, timeout: Duration) -> Self {
        Self {
            deadline: started + timeout,
            pages_left: CDSE_SENTINEL2_MAX_PAGES,
            records_left: CDSE_SENTINEL2_MAX_LOADED_RECORDS,
            bytes_left: MAX_SEARCH_BYTES,
        }
    }

    fn check_stop(&self, cancelled: &dyn Fn() -> bool) -> ProviderResult<Duration> {
        if cancelled() {
            return Err(odata_error(
                "CDSE_ODATA_CANCELLED",
                "Sentinel-2 检索已取消",
                "spatial_search",
            ));
        }
        self.deadline
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| {
                odata_error(
                    "CDSE_ODATA_TIMEOUT",
                    "Sentinel-2 检索超时",
                    "search_deadline",
                )
            })
    }

    fn fetch(
        &mut self,
        engine: &ODataHttpEngine,
        query: &CatalogSearchQuery,
        request: &ODataRequest,
        cancelled: &dyn Fn() -> bool,
    ) -> ProviderResult<RawPage> {
        let remaining = self.check_stop(cancelled)?;
        if self.pages_left == 0 {
            return Err(odata_error(
                "CDSE_ODATA_PAGE_BUDGET_EXCEEDED",
                "Sentinel-2 检索超过分页上限",
                "search_page_budget",
            ));
        }
        if self.records_left == 0 {
            return Err(odata_error(
                "CDSE_ODATA_RESULT_LIMIT_EXCEEDED",
                "Sentinel-2 检索超过记录上限",
                "search_record_budget",
            ));
        }
        if self.bytes_left == 0 {
            return Err(odata_error(
                "CDSE_ODATA_RESPONSE_TOO_LARGE",
                "Sentinel-2 检索超过响应上限",
                "search_byte_budget",
            ));
        }
        self.pages_left -= 1;
        let response = engine.fetch_raw_page_with_limits(
            query,
            request,
            cancelled,
            remaining.min(engine.timeout),
            self.bytes_left.min(MAX_RESPONSE_BYTES),
        );
        self.check_stop(cancelled)?;
        let page = response?;
        self.bytes_left = self
            .bytes_left
            .checked_sub(page.response_bytes)
            .ok_or_else(|| {
                odata_error(
                    "CDSE_ODATA_RESPONSE_TOO_LARGE",
                    "Sentinel-2 检索超过响应上限",
                    "search_byte_budget",
                )
            })?;
        self.records_left = self
            .records_left
            .checked_sub(page.records.len() as u64)
            .ok_or_else(|| {
                odata_error(
                    "CDSE_ODATA_RESULT_LIMIT_EXCEEDED",
                    "Sentinel-2 检索超过记录上限",
                    "search_record_budget",
                )
            })?;
        self.check_stop(cancelled)?;
        Ok(page)
    }
}

fn invalid_page(reason: &str) -> ContractError {
    odata_error(
        "CDSE_ODATA_PAGE_INVALID",
        "Sentinel-2 目录分页不完整",
        reason,
    )
}

impl ODataHttpEngine {
    pub(super) fn search_spatial_all(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &dyn Fn() -> bool,
        started: Instant,
    ) -> ProviderResult<CatalogSearchResponse> {
        let mut budget = SpatialBudget::new(
            started,
            self.timeout
                .saturating_mul(SPATIAL_DEADLINE_REQUEST_MULTIPLE),
        );
        self.search_spatial_in_budget(query, cancelled, started, &mut budget)
    }

    fn search_spatial_in_budget(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &dyn Fn() -> bool,
        started: Instant,
        budget: &mut SpatialBudget,
    ) -> ProviderResult<CatalogSearchResponse> {
        budget.check_stop(cancelled)?;
        let facts = decode_sentinel2_search_facts_for_use(query, self.route_use)?;
        let aoi = query
            .aoi_geojson
            .as_ref()
            .and_then(authoritative_catalogue_geometry)
            .ok_or_else(|| {
                odata_error(
                    "CDSE_ODATA_AOI_INVALID",
                    "Sentinel-2 AOI 几何无效",
                    "canonical_geometry",
                )
            })?;
        let tiles = bounded_catalogue_request_tiles(&aoi, 15.0, 8.0, MAX_SPATIAL_PARTS)
            .filter(|tiles| !tiles.is_empty())
            .ok_or_else(|| {
                odata_error(
                    "CDSE_ODATA_AOI_INVALID",
                    "Sentinel-2 AOI 无法生成检索范围",
                    "spatial_plan",
                )
            })?;
        let prepared_aoi = PreparedCatalogueAoi::new(&aoi);
        // A rectangle with no positive-area overlap cannot yield an admitted
        // product; the most-overlapping rectangle is paged first so a finite
        // target fills from true hits before the shared budget is spent.
        let tiles = order_request_tiles_by_exact_overlap(prepared_aoi.as_ref(), tiles);
        if tiles.is_empty() {
            return Err(odata_error(
                "CDSE_ODATA_AOI_INVALID",
                "Sentinel-2 AOI 无法生成检索范围",
                "spatial_plan",
            ));
        }
        let mut unique = BTreeMap::<String, ProviderProduct>::new();
        let mut exact = Vec::<ProviderProduct>::new();
        let mut request_count = 0_u64;
        let mut parts = 0_u64;
        let mut fetched = 0_u64;
        let mut duplicates = 0_u64;
        let mut remote_exhausted = false;

        'parts: for (index, tile) in tiles.iter().enumerate() {
            budget.check_stop(cancelled)?;
            let mut part = query.clone();
            part.aoi_geojson = Some(tile.clone());
            part.cursor = None;
            // Page capacity is not the requested number of exact AOI products.
            part.result_limit = Some(crate::cdse::query::CDSE_SENTINEL2_MAX_PAGE_SIZE);
            let mut request = build_odata_request_for_use(&part, self.route_use)?;
            let mut expected_count = None;
            let mut seen = HashSet::new();
            parts += 1;
            if self.route_use == RouteUse::Production {
                let page_size = crate::cdse::query::CDSE_SENTINEL2_MAX_PAGE_SIZE;
                let mut base_skip = 0_u64;
                'tile_waves: loop {
                    let remaining = budget.check_stop(cancelled)?;
                    if budget.pages_left == 0 {
                        return Err(odata_error(
                            "CDSE_ODATA_PAGE_BUDGET_EXCEEDED",
                            "Sentinel-2 检索超过分页上限",
                            "search_page_budget",
                        ));
                    }
                    if budget.records_left == 0 {
                        return Err(odata_error(
                            "CDSE_ODATA_RESULT_LIMIT_EXCEEDED",
                            "Sentinel-2 检索超过记录上限",
                            "search_record_budget",
                        ));
                    }
                    if budget.bytes_left == 0 {
                        return Err(odata_error(
                            "CDSE_ODATA_RESPONSE_TOO_LARGE",
                            "Sentinel-2 检索超过响应上限",
                            "search_byte_budget",
                        ));
                    }
                    let wave_len = facts
                        .result_limit
                        .map_or(3, |t| t.div_ceil(100).clamp(1, 3) as usize)
                        .min(budget.pages_left);
                    let skips: Vec<(u64, bool)> = (0..wave_len)
                        .map(|k| (base_skip + (k as u64) * page_size, false))
                        .collect();
                    let wave_result = self.fetch_concurrent_skip_wave(
                        &request,
                        &skips,
                        cancelled,
                        remaining.min(self.timeout),
                        budget.bytes_left.min(MAX_RESPONSE_BYTES),
                    );
                    budget.check_stop(cancelled)?;
                    let wave_pages = wave_result?;
                    budget.check_stop(cancelled)?;

                    for ((skip, _), (page, response_bytes)) in skips.into_iter().zip(wave_pages) {
                        if budget.pages_left == 0 {
                            return Err(odata_error(
                                "CDSE_ODATA_PAGE_BUDGET_EXCEEDED",
                                "Sentinel-2 检索超过分页上限",
                                "search_page_budget",
                            ));
                        }
                        budget.pages_left -= 1;
                        budget.bytes_left = budget
                            .bytes_left
                            .checked_sub(response_bytes as u64)
                            .ok_or_else(|| {
                                odata_error(
                                    "CDSE_ODATA_RESPONSE_TOO_LARGE",
                                    "Sentinel-2 检索超过响应上限",
                                    "search_byte_budget",
                                )
                            })?;
                        budget.records_left = budget
                            .records_left
                            .checked_sub(page.value.len() as u64)
                            .ok_or_else(|| {
                                odata_error(
                                    "CDSE_ODATA_RESULT_LIMIT_EXCEEDED",
                                    "Sentinel-2 检索超过记录上限",
                                    "search_record_budget",
                                )
                            })?;
                        request_count += 1;
                        if page.value.is_empty() && page.next_link.is_some() {
                            return Err(invalid_page("skip_progress"));
                        }
                        let expected_next_skip = skip
                            .checked_add(page.value.len() as u64)
                            .ok_or_else(|| invalid_page("count_progress"))?;
                        let page_exhausted = match page.next_link.as_deref() {
                            None => true,
                            Some(raw_link) => {
                                let next_skip = request.validate_next_link_skip_for_endpoint(
                                    &self.endpoint,
                                    raw_link,
                                )?;
                                if next_skip != expected_next_skip {
                                    return Err(invalid_page("skip_progress"));
                                }
                                false
                            }
                        };
                        fetched += page.value.len() as u64;
                        for raw in page.value {
                            budget.check_stop(cancelled)?;
                            let raw: ODataProduct = serde_json::from_value(raw).map_err(|_| {
                                odata_error(
                                    "CDSE_ODATA_ITEM_INVALID",
                                    "Sentinel-2 目录记录无效",
                                    "record_shape",
                                )
                            })?;
                            let product = map_record(&raw, &facts)?;
                            let footprint = product
                                .footprint_geojson
                                .as_ref()
                                .filter(|footprint| {
                                    insar_core::validated_polygonal_geojson(footprint).is_some()
                                })
                                .ok_or_else(|| {
                                    odata_error(
                                        "CDSE_ODATA_ITEM_INVALID",
                                        "Sentinel-2 影像缺少有效覆盖范围",
                                        "exact_footprint",
                                    )
                                })?;
                            match unique.get(&product.product_id) {
                                Some(existing) if existing == &product => {
                                    duplicates += 1;
                                }
                                Some(_) => {
                                    return Err(odata_error(
                                        "CDSE_ODATA_UUID_CONFLICT",
                                        "Sentinel-2 目录记录身份冲突",
                                        "spatial_duplicate",
                                    ))
                                }
                                None => {
                                    if prepared_aoi.as_ref().is_some_and(|prepared| {
                                        prepared.intersects_footprint(footprint)
                                    }) {
                                        exact.push(product.clone());
                                    }
                                    unique.insert(product.product_id.clone(), product);
                                }
                            }
                        }
                        let satisfied = facts
                            .result_limit
                            .is_some_and(|target| exact.len() as u64 >= target);
                        if satisfied {
                            remote_exhausted = page_exhausted && index + 1 == tiles.len();
                            break 'parts;
                        }
                        if page_exhausted {
                            break 'tile_waves;
                        }
                    }
                    base_skip += (wave_len as u64) * page_size;
                }
                remote_exhausted = index + 1 == tiles.len();
                continue 'parts;
            }
            loop {
                let page = budget.fetch(self, &part, &request, cancelled)?;
                request_count += 1;
                let expected = match expected_count {
                    None => page
                        .matched_count
                        .ok_or_else(|| invalid_page("missing_count"))?,
                    Some(expected) => {
                        if page.matched_count.is_some_and(|count| count != expected) {
                            return Err(invalid_page("count_changed"));
                        }
                        expected
                    }
                };
                expected_count = Some(expected);
                let loaded = page
                    .start_skip
                    .checked_add(page.records.len() as u64)
                    .filter(|loaded| *loaded <= expected)
                    .ok_or_else(|| invalid_page("count_progress"))?;
                if page.start_skip != request.current_skip()
                    || (page.next_cursor.is_some()
                        && (page.records.is_empty()
                            || page.next_skip != Some(loaded)
                            || loaded >= expected))
                {
                    return Err(invalid_page("skip_progress"));
                }
                fetched += page.records.len() as u64;
                // Map the entire page before cutting a finite target. Otherwise
                // a malformed/conflicting tail can be hidden behind truncation.
                for raw in page.records {
                    budget.check_stop(cancelled)?;
                    let raw: ODataProduct = serde_json::from_value(raw).map_err(|_| {
                        odata_error(
                            "CDSE_ODATA_ITEM_INVALID",
                            "Sentinel-2 目录记录无效",
                            "record_shape",
                        )
                    })?;
                    let product = map_record(&raw, &facts)?;
                    let footprint = product
                        .footprint_geojson
                        .as_ref()
                        .filter(|footprint| {
                            insar_core::validated_polygonal_geojson(footprint).is_some()
                        })
                        .ok_or_else(|| {
                            odata_error(
                                "CDSE_ODATA_ITEM_INVALID",
                                "Sentinel-2 影像缺少有效覆盖范围",
                                "exact_footprint",
                            )
                        })?;
                    match unique.get(&product.product_id) {
                        Some(existing) if existing == &product => {
                            duplicates += 1;
                        }
                        Some(_) => {
                            return Err(odata_error(
                                "CDSE_ODATA_UUID_CONFLICT",
                                "Sentinel-2 目录记录身份冲突",
                                "spatial_duplicate",
                            ))
                        }
                        None => {
                            if prepared_aoi
                                .as_ref()
                                .is_some_and(|prepared| prepared.intersects_footprint(footprint))
                            {
                                exact.push(product.clone());
                            }
                            unique.insert(product.product_id.clone(), product);
                        }
                    }
                }
                let satisfied = facts
                    .result_limit
                    .is_some_and(|target| exact.len() as u64 >= target);
                let part_exhausted = page.next_cursor.is_none() && loaded == expected;
                if satisfied {
                    remote_exhausted = part_exhausted && index + 1 == tiles.len();
                    break 'parts;
                }
                match page.next_cursor {
                    None if part_exhausted => break,
                    None => return Err(invalid_page("missing_continuation")),
                    Some(cursor) => {
                        if !seen.insert(cursor.clone()) {
                            return Err(invalid_page("cursor_cycle"));
                        }
                        part.cursor = Some(cursor);
                        request = build_odata_request_for_use(&part, self.route_use)?;
                    }
                }
            }
            remote_exhausted = index + 1 == tiles.len();
        }
        budget.check_stop(cancelled)?;
        let exact_count = exact.len() as u64;
        exact.sort_by(|a, b| {
            a.acquisition_datetime
                .cmp(&b.acquisition_datetime)
                .then_with(|| a.product_id.cmp(&b.product_id))
        });
        if let Some(target) = facts.result_limit {
            exact.truncate(target as usize);
        }
        let response = CatalogSearchResponse {
            collection: sentinel2_collection(&facts),
            stats: CatalogSearchStats {
                requested_limit: query.result_limit,
                matched_count: None,
                request_count,
                transport_part_count: parts,
                fetched_record_count: fetched,
                mapped_record_count: fetched,
                profile_filtered_record_count: 0,
                exact_duplicate_record_count: duplicates,
                unique_provider_product_count: unique.len() as u64,
                exact_matched_count: exact_count,
                returned_count: exact.len() as u64,
                remote_exhausted,
                working_set_satisfied: remote_exhausted
                    || facts
                        .result_limit
                        .is_some_and(|target| exact_count >= target),
                elapsed_ms: started.elapsed().as_millis().min(u64::MAX as u128) as u64,
                truncated: false,
            },
            products: exact,
            next_cursor: None,
            item_errors: Vec::new(),
        };
        response
            .validate()
            .map_err(|_| invalid_page("spatial_stats"))?;
        Ok(response)
    }
}

#[cfg(test)]
#[path = "spatial_search_tests.rs"]
mod tests;
