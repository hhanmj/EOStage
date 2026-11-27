//! Sentinel-1 burst catalogue support via the CMR UMM-JSON endpoint.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use chrono::{DateTime, SecondsFormat, Utc};
use geo::{line_intersection::line_intersection, Area, Geometry, Intersects, Line, MultiPolygon};
use insar_core::polygonal_topology::{MAX_POLYGONAL_COORDS_PER_RING, MAX_POLYGONAL_TOTAL_COORDS};
use insar_core::{
    asf_catalog_capabilities, stable_asset_key, validated_polygonal_geojson, AssetRole,
    ContractError, ProviderAsset, ProviderResult, SarBurstSelection, SarProduct,
    SarRemoteSearchRequest, SarRemoteSearchResponse, SarSearchCountRequest, SarSearchCountResponse,
    SarSearchQuery,
};
use serde_json::{json, Map, Value};
use url::Url;

use super::query::{
    asf_datetime, catalogue_spatial_plan, footprint_intersects_exact_catalogue_aoi,
    CatalogueSpatialPlan,
};
use super::search::{MetadataTransport, SarSearchProviderProgress, SarSearchProviderProgressStage};

pub const BURST_COLLECTION_CONCEPT_ID: &str = "C2709161906-ASF";
const BURST_COLLECTION_SHORT_NAME: &str = "SENTINEL-1_BURSTS";
const BURST_COLLECTION_VERSION: &str = "1";
const BURST_ASSET_HOST: &str = "sentinel1-burst.asf.alaska.edu";
pub const BURST_PAGE_SIZE: u64 = 250;
const ALL_RESULTS_SAFETY_LIMIT: u64 = insar_core::SEARCH_SAFETY_CAP;
const DEFAULT_BURST_RESULT_LIMIT: u64 = BURST_PAGE_SIZE;
const MAX_RAW_BURST_RECORD_BYTES: usize = 2 * 1024 * 1024;
const MAX_BURST_GPOLYGONS: usize = 64;
const MAX_BURST_POINTS_PER_GPOLYGON: usize = 4_096;
const MAX_BURST_TOPOLOGY_EDGE_PAIRS: u64 = 10_000_000;
const RECOGNIZED_UMM_G_VERSIONS: &[&str] = &["1.6.7"];

#[derive(Clone, Debug, PartialEq)]
pub struct BurstCmrQuery {
    pub requested_limit: Option<u64>,
    pub safety_limit: u64,
    pub polygons: Vec<String>,
    canonical_geometry: Option<geo::MultiPolygon<f64>>,
    requires_exact_spatial_post_filter: bool,
    product_filter: BurstProductFilter,
    common_params: Vec<(String, String)>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct BurstProductFilter {
    platforms: Vec<String>,
    beam_modes: Vec<String>,
    polarizations: Vec<String>,
    flight_direction: Option<String>,
}

impl BurstProductFilter {
    fn is_active(&self) -> bool {
        !self.platforms.is_empty()
            || !self.beam_modes.is_empty()
            || !self.polarizations.is_empty()
            || self.flight_direction.is_some()
    }

    fn matches(&self, product: &SarProduct) -> bool {
        matches_requested(product.platform.as_deref(), &self.platforms)
            && matches_requested(product.beam_mode.as_deref(), &self.beam_modes)
            && matches_requested(product.polarization.as_deref(), &self.polarizations)
            && self.flight_direction.as_ref().is_none_or(|requested| {
                product
                    .flight_direction
                    .as_deref()
                    .is_some_and(|value| value.eq_ignore_ascii_case(requested))
            })
    }
}

impl BurstCmrQuery {
    pub fn page_params(
        &self,
        polygon: &str,
        page_number: u64,
        page_size: u64,
    ) -> Vec<(String, String)> {
        let mut params = self.common_params.clone();
        params.push(("page_size".to_owned(), page_size.to_string()));
        params.push(("page_num".to_owned(), page_number.to_string()));
        if !polygon.is_empty() {
            params.push(("polygon".to_owned(), polygon.to_owned()));
        }
        params
    }

    fn requires_post_filter(&self) -> bool {
        self.requires_exact_spatial_post_filter || self.product_filter.is_active()
    }
}

/// Projects the closed SLC-BURST query vocabulary into the authoritative CMR
/// collection and the three official burst identity attributes.
pub fn build_burst_cmr_query(query: &SarSearchQuery) -> ProviderResult<BurstCmrQuery> {
    query
        .validate()
        .map_err(|reason| burst_error("ASF_QUERY_INVALID", "query", &reason))?;
    if !query.provider.eq_ignore_ascii_case("ASF")
        || !query.dataset.eq_ignore_ascii_case("SLC-BURST")
    {
        return Err(burst_error(
            "ASF_QUERY_UNSUPPORTED",
            "dataset",
            "burst_route",
        ));
    }
    reject_unsupported_burst_query_fields(query)?;
    let capabilities = asf_catalog_capabilities()
        .datasets
        .into_iter()
        .find(|dataset| dataset.dataset.eq_ignore_ascii_case("SLC-BURST"))
        .ok_or_else(|| burst_error("ASF_QUERY_UNSUPPORTED", "dataset", "registry_missing"))?;
    let platforms =
        canonical_filter_values(&query.platforms, &capabilities.platforms, "platforms")?;
    canonical_optional_filter(
        query.instrument.as_deref(),
        &capabilities.instruments,
        "instrument",
    )?;
    canonical_filter_values(
        &query.processing_levels,
        &capabilities.processing_levels,
        "processing_levels",
    )?;
    canonical_filter_values(
        &query.product_families,
        &capabilities.product_families,
        "product_families",
    )?;
    let beam_modes =
        canonical_filter_values(&query.beam_modes, &capabilities.beam_modes, "beam_modes")?;
    let polarizations = canonical_filter_values(
        &query.polarizations,
        &capabilities.polarizations,
        "polarizations",
    )?;
    let flight_direction = query
        .flight_direction
        .as_deref()
        .map(str::trim)
        .map(|value| value.to_ascii_uppercase())
        .map(|value| {
            matches!(value.as_str(), "ASCENDING" | "DESCENDING")
                .then_some(value)
                .ok_or_else(|| {
                    burst_error(
                        "ASF_QUERY_UNSUPPORTED",
                        "flight_direction",
                        "registry_value",
                    )
                })
        })
        .transpose()?;
    let full = query
        .burst
        .as_ref()
        .map(|burst| optional_burst_text(burst.full_burst_id.as_deref(), "full_burst_id"))
        .transpose()?
        .flatten();
    let relative = query
        .burst
        .as_ref()
        .map(|burst| optional_positive(burst.relative_burst_id, "relative_burst_id"))
        .transpose()?
        .flatten();
    let absolute = query
        .burst
        .as_ref()
        .map(|burst| optional_positive(burst.absolute_burst_id, "absolute_burst_id"))
        .transpose()?
        .flatten();
    let temporal = temporal(query.start.as_deref(), query.end.as_deref())?;
    let spatial = match query.aoi_geojson.as_ref() {
        Some(aoi) => catalogue_spatial_plan(aoi)?,
        None => CatalogueSpatialPlan::empty(),
    };
    if query.aoi_geojson.is_some() && spatial.polygons.is_empty() {
        return Err(burst_error(
            "ASF_QUERY_INVALID",
            "aoi_geojson",
            "empty_polygon_plan",
        ));
    }
    // CMR returns oldest first unless told otherwise, so a search that hits
    // its result limit would keep the earliest bursts (#22). Ask for newest
    // first, matching the SLC CMR route.
    let mut common_params = vec![
        (
            "collection_concept_id".to_owned(),
            BURST_COLLECTION_CONCEPT_ID.to_owned(),
        ),
        ("sort_key".to_owned(), "-start_date".to_owned()),
    ];
    if let Some(temporal) = temporal {
        common_params.push(("temporal".to_owned(), temporal));
    }
    if let Some(full) = full {
        common_params.push((
            "attribute[]".to_owned(),
            format!("string,BURST_ID_FULL,{full}"),
        ));
    }
    if let Some(relative) = relative {
        common_params.push((
            "attribute[]".to_owned(),
            format!("int,BURST_ID_RELATIVE,{relative}"),
        ));
    }
    if let Some(absolute) = absolute {
        common_params.push((
            "attribute[]".to_owned(),
            format!("int,BURST_ID_ABSOLUTE,{absolute}"),
        ));
    }
    if let Some(flight_direction) = flight_direction.as_deref() {
        common_params.push((
            "attribute[]".to_owned(),
            format!("string,ASCENDING_DESCENDING,{flight_direction}"),
        ));
    }
    common_params.shrink_to_fit();
    let requested_limit = query
        .max_results
        .map(|limit| limit.min(insar_core::SEARCH_SAFETY_CAP));
    Ok(BurstCmrQuery {
        requested_limit,
        safety_limit: requested_limit.unwrap_or(DEFAULT_BURST_RESULT_LIMIT),
        polygons: spatial.polygons,
        canonical_geometry: spatial.canonical_geometry,
        requires_exact_spatial_post_filter: spatial.requires_exact_post_filter,
        product_filter: BurstProductFilter {
            platforms,
            beam_modes,
            polarizations,
            flight_direction,
        },
        common_params,
    })
}

fn reject_unsupported_burst_query_fields(query: &SarSearchQuery) -> ProviderResult<()> {
    if let Some(burst) = query.burst.as_ref() {
        for (present, field) in [
            (burst.burst_id.is_some(), "burst.burst_id"),
            (burst.subswath.is_some(), "burst.subswath"),
            (burst.burst_index.is_some(), "burst.burst_index"),
        ] {
            if present {
                return Err(burst_error(
                    "ASF_QUERY_UNSUPPORTED",
                    field,
                    "formal_burst_route",
                ));
            }
        }
    }
    for (present, field) in [
        (query.look_direction.is_some(), "look_direction"),
        (query.relative_orbit.is_some(), "relative_orbit"),
        (query.frame.is_some(), "frame"),
        (query.asf_frame.is_some(), "asf_frame"),
        (query.path.is_some(), "path"),
        (query.track_range.is_some(), "track_range"),
        (query.frame_range.is_some(), "frame_range"),
        (query.aoi_wkt.is_some(), "aoi_wkt"),
        (query.bbox.is_some(), "bbox"),
    ] {
        if present {
            return Err(burst_error(
                "ASF_QUERY_UNSUPPORTED",
                field,
                "formal_burst_route",
            ));
        }
    }
    Ok(())
}

fn canonical_filter_values(
    requested: &[String],
    allowed: &[String],
    field: &'static str,
) -> ProviderResult<Vec<String>> {
    requested
        .iter()
        .map(|value| {
            allowed
                .iter()
                .find(|candidate| candidate.eq_ignore_ascii_case(value.trim()))
                .cloned()
                .ok_or_else(|| burst_error("ASF_QUERY_UNSUPPORTED", field, "registry_value"))
        })
        .collect()
}

fn canonical_optional_filter(
    requested: Option<&str>,
    allowed: &[String],
    field: &'static str,
) -> ProviderResult<Option<String>> {
    requested
        .map(|value| canonical_filter_values(&[value.to_owned()], allowed, field))
        .transpose()
        .map(|values| values.and_then(|values| values.into_iter().next()))
}

fn matches_requested(actual: Option<&str>, requested: &[String]) -> bool {
    requested.is_empty()
        || actual.is_some_and(|actual| {
            requested
                .iter()
                .any(|requested| actual.eq_ignore_ascii_case(requested))
        })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BurstSearchLimits {
    pub max_requests: u64,
    pub max_pages: u64,
    pub max_records: u64,
    pub max_raw_records: u64,
    pub max_elapsed: Duration,
}

impl Default for BurstSearchLimits {
    fn default() -> Self {
        Self {
            max_requests: 512,
            max_pages: 512,
            max_records: ALL_RESULTS_SAFETY_LIMIT,
            max_raw_records: 100_000,
            max_elapsed: Duration::from_secs(600),
        }
    }
}

struct BurstPartState {
    polygon: String,
    next_page: u64,
    search_after: Option<String>,
    page_count: u64,
    hits: Option<u64>,
    fetched: u64,
    seen_pages: BTreeSet<Vec<u8>>,
    seen_identities: BTreeSet<String>,
    exhausted: bool,
    /// The part finished exhausted while CMR still claimed more `hits` than
    /// it served. The walk is complete, but the result is only possibly
    /// complete: the remote total may overcount what the index returns.
    served_fewer_than_hits: bool,
}

impl BurstPartState {
    fn new(polygon: String) -> Self {
        Self {
            polygon,
            next_page: 1,
            search_after: None,
            page_count: 0,
            hits: None,
            fetched: 0,
            seen_pages: BTreeSet::new(),
            seen_identities: BTreeSet::new(),
            exhausted: false,
            served_fewer_than_hits: false,
        }
    }
}

pub struct BurstCmrProvider<'a, T> {
    transport: &'a T,
    limits: BurstSearchLimits,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BurstDeadlineStage {
    BeforeRequest,
    AfterTransport,
    AfterProviderProgress,
    BeforeMapping,
    AfterMapping,
    AfterAdmission,
    AfterAdmissionProgress,
    BeforeResponse,
}

trait BurstDeadline {
    fn check(&self, stage: BurstDeadlineStage) -> ProviderResult<()>;
}

struct ElapsedDeadline {
    started: Instant,
    max_elapsed: Duration,
}

impl BurstDeadline for ElapsedDeadline {
    #[inline]
    fn check(&self, _stage: BurstDeadlineStage) -> ProviderResult<()> {
        check_elapsed(self.started, self.max_elapsed)
    }
}

impl<'a, T: MetadataTransport> BurstCmrProvider<'a, T> {
    pub fn with_limits(transport: &'a T, limits: BurstSearchLimits) -> ProviderResult<Self> {
        if limits.max_requests == 0
            || limits.max_pages == 0
            || limits.max_records == 0
            || limits.max_records > ALL_RESULTS_SAFETY_LIMIT
            || limits.max_raw_records == 0
            || limits.max_raw_records > 100_000
            || limits.max_elapsed.is_zero()
            || limits.max_elapsed > Duration::from_secs(600)
        {
            return Err(burst_error(
                "ASF_QUERY_INVALID",
                "limits",
                "bounded_ceiling",
            ));
        }
        Ok(Self { transport, limits })
    }

    pub fn search(
        &self,
        request: &SarRemoteSearchRequest,
        cancelled: &(dyn Fn() -> bool + Sync),
        progress: &dyn Fn(SarSearchProviderProgress),
    ) -> ProviderResult<SarRemoteSearchResponse> {
        let deadline = ElapsedDeadline {
            started: Instant::now(),
            max_elapsed: self.limits.max_elapsed,
        };
        self.search_with_deadline(request, cancelled, progress, &deadline)
    }

    fn search_with_deadline<D: BurstDeadline>(
        &self,
        request: &SarRemoteSearchRequest,
        cancelled: &(dyn Fn() -> bool + Sync),
        progress: &dyn Fn(SarSearchProviderProgress),
        deadline: &D,
    ) -> ProviderResult<SarRemoteSearchResponse> {
        check_cancelled(cancelled)?;
        let plan = build_burst_cmr_query(&request.query)?;
        if plan.requested_limit.is_some() && plan.safety_limit > self.limits.max_records {
            return Err(burst_error(
                "ASF_QUERY_INVALID",
                "max_results",
                "record_ceiling",
            ));
        }
        let safety_limit = plan.safety_limit.min(self.limits.max_records);
        let mut products = BTreeMap::new();
        let mut errors = Vec::new();
        let mut request_count = 0_u64;
        let mut raw_fetched_count = 0_u64;
        let mut mapped_count = 0_u64;
        let mut exact_matched_count = 0_u64;
        let mut effective_page_size = 0_u64;
        let polygons = if plan.polygons.is_empty() {
            vec![String::new()]
        } else {
            plan.polygons.clone()
        };
        let transport_part_count = polygons.len() as u64;
        let mut parts = polygons
            .into_iter()
            .map(BurstPartState::new)
            .collect::<Vec<_>>();
        'round_robin: loop {
            let mut made_request = false;
            for part in &mut parts {
                if part.exhausted {
                    continue;
                }
                check_cancelled(cancelled)?;
                deadline.check(BurstDeadlineStage::BeforeRequest)?;
                if request_count >= self.limits.max_requests
                    || part.page_count >= self.limits.max_pages
                {
                    return Err(burst_error("ASF_PAGINATION_INVALID", "page", "ceiling"));
                }
                request_count += 1;
                part.page_count += 1;
                made_request = true;
                if raw_fetched_count >= self.limits.max_raw_records {
                    return Err(burst_error("ASF_PAGINATION_INVALID", "records", "ceiling"));
                }
                // CMR Search-After cursors are tied to the original query. Keep
                // the page size stable for the entire cursor walk; shrinking the
                // last request at an internal ceiling invalidates the cursor.
                let page_size = BURST_PAGE_SIZE;
                effective_page_size = effective_page_size.max(page_size);
                let page = self.transport.cmr_umm_page(
                    &plan.page_params(&part.polygon, part.next_page, page_size),
                    part.search_after.as_deref(),
                    cancelled,
                )?;
                deadline.check(BurstDeadlineStage::AfterTransport)?;
                check_cancelled(cancelled)?;
                let search_after_capable = page.search_after_capable;
                let timed_out = page.timed_out;
                let next_search_after = page.search_after;
                let value = page.value;
                let claimed_total = value
                    .get("hits")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| burst_error("ASF_PAGINATION_INVALID", "hits", "missing"))?;
                match part.hits {
                    None => part.hits = Some(claimed_total),
                    Some(expected) if expected == claimed_total => {}
                    Some(_) => {
                        return Err(burst_error("ASF_PAGINATION_INVALID", "hits", "changed"))
                    }
                }
                let items = value
                    .get("items")
                    .and_then(Value::as_array)
                    .ok_or_else(|| burst_error("ASF_PAGINATION_INVALID", "items", "missing"))?;
                if items.len() > page_size as usize {
                    return Err(burst_error("ASF_PAGINATION_INVALID", "page", "size"));
                }
                let page_fingerprint = serde_json::to_vec(items)
                    .map_err(|_| burst_error("ASF_PAGINATION_INVALID", "page", "serialization"))?;
                if !items.is_empty() && !part.seen_pages.insert(page_fingerprint) {
                    return Err(burst_error("ASF_PAGINATION_INVALID", "page", "repeated"));
                }
                let page_identity_progress = items
                    .iter()
                    .filter_map(|item| item.pointer("/meta/native-id").and_then(Value::as_str))
                    .filter(|identity| !identity.is_empty())
                    .any(|identity| part.seen_identities.insert(identity.to_owned()));
                if !items.is_empty() && !page_identity_progress {
                    return Err(burst_error(
                        "ASF_PAGINATION_INVALID",
                        "page",
                        "non_progress",
                    ));
                }
                let page_len = items.len() as u64;
                part.fetched = part
                    .fetched
                    .checked_add(page_len)
                    .ok_or_else(|| burst_error("ASF_PAGINATION_INVALID", "fetched", "overflow"))?;
                raw_fetched_count = raw_fetched_count
                    .checked_add(page_len)
                    .ok_or_else(|| burst_error("ASF_PAGINATION_INVALID", "fetched", "overflow"))?;
                if part.fetched > claimed_total {
                    return Err(burst_error(
                        "ASF_PAGINATION_INVALID",
                        "fetched",
                        "exceeds_hits",
                    ));
                }
                if raw_fetched_count > self.limits.max_raw_records {
                    return Err(burst_error("ASF_PAGINATION_INVALID", "records", "ceiling"));
                }
                if !search_after_capable && page_len == 0 && part.fetched < claimed_total {
                    return Err(burst_error(
                        "ASF_PAGINATION_INVALID",
                        "page",
                        "empty_incomplete",
                    ));
                }
                let authoritative_remote_total =
                    transport_part_count == 1 && !plan.requires_post_filter();
                progress(burst_progress(
                    SarSearchProviderProgressStage::ProviderResponse,
                    request_count,
                    transport_part_count,
                    raw_fetched_count,
                    mapped_count,
                    products.len() as u64,
                    plan.requested_limit,
                    authoritative_remote_total.then_some(claimed_total),
                ));
                deadline.check(BurstDeadlineStage::AfterProviderProgress)?;
                // Slice 3(a): whether the target cut this page short.
                let mut page_cut_by_target = false;
                for (item_index, item) in items.iter().enumerate() {
                    check_cancelled(cancelled)?;
                    deadline.check(BurstDeadlineStage::BeforeMapping)?;
                    let mapped = map_burst_umm_granule(item);
                    deadline.check(BurstDeadlineStage::AfterMapping)?;
                    match mapped {
                        Ok(product) => {
                            mapped_count = mapped_count.checked_add(1).ok_or_else(|| {
                                burst_error("ASF_PAGINATION_INVALID", "mapped", "overflow")
                            })?;
                            let spatial_matches = !plan.requires_exact_spatial_post_filter
                                || plan.canonical_geometry.as_ref().is_some_and(|aoi| {
                                    product.footprint_geojson.as_ref().is_some_and(|footprint| {
                                        footprint_intersects_exact_catalogue_aoi(footprint, aoi)
                                    })
                                });
                            if !spatial_matches || !plan.product_filter.matches(&product) {
                                deadline.check(BurstDeadlineStage::AfterAdmission)?;
                                continue;
                            }
                            exact_matched_count =
                                exact_matched_count.checked_add(1).ok_or_else(|| {
                                    burst_error("ASF_PAGINATION_INVALID", "matched", "overflow")
                                })?;
                            match products.get(&product.product_id) {
                                Some(existing) if existing == &product => {}
                                Some(_) => {
                                    return Err(burst_error(
                                        "ASF_IDENTITY_CONFLICT",
                                        "native_id",
                                        "immutable_authority_or_assets",
                                    ));
                                }
                                None => {
                                    products.insert(product.product_id.clone(), product);
                                }
                            }
                            deadline.check(BurstDeadlineStage::AfterAdmission)?;
                            if products.len() as u64 >= safety_limit {
                                page_cut_by_target = item_index + 1 < items.len();
                                break;
                            }
                        }
                        Err(error) => {
                            errors.push(error);
                            deadline.check(BurstDeadlineStage::AfterAdmission)?;
                        }
                    }
                }
                progress(burst_progress(
                    SarSearchProviderProgressStage::AdmittingResults,
                    request_count,
                    transport_part_count,
                    raw_fetched_count,
                    mapped_count,
                    products.len() as u64,
                    plan.requested_limit,
                    authoritative_remote_total.then_some(claimed_total),
                ));
                deadline.check(BurstDeadlineStage::AfterAdmissionProgress)?;
                if products.len() as u64 >= safety_limit {
                    // Exactly N with every claimed record served on a fully
                    // consumed page is proven exhaustion, not a warning.
                    if !page_cut_by_target && part.fetched >= claimed_total {
                        part.exhausted = true;
                    }
                    break 'round_robin;
                }
                if search_after_capable {
                    if timed_out && next_search_after.is_none() {
                        return Err(burst_pagination_state_error(
                            "timeout_missing_search_after",
                            claimed_total,
                            part.fetched,
                            page_len,
                            page_size,
                        ));
                    }
                    if part.fetched >= claimed_total {
                        part.exhausted = true;
                    } else if page_len == 0 {
                        if timed_out {
                            let next = next_search_after.expect(
                                "timed-out Search-After page was checked for a cursor above",
                            );
                            if part.search_after.as_deref() == Some(next.as_str()) {
                                return Err(burst_pagination_state_error(
                                    "repeated_search_after",
                                    claimed_total,
                                    part.fetched,
                                    page_len,
                                    page_size,
                                ));
                            }
                            part.search_after = Some(next);
                        } else if next_search_after.is_some() {
                            return Err(burst_pagination_state_error(
                                "empty_with_search_after",
                                claimed_total,
                                part.fetched,
                                page_len,
                                page_size,
                            ));
                        } else if part.fetched == 0 {
                            return Err(burst_pagination_state_error(
                                "empty_incomplete",
                                claimed_total,
                                part.fetched,
                                page_len,
                                page_size,
                            ));
                        } else {
                            // CMR legitimately terminates a Search-After walk
                            // before serving `hits` items: the polygon-filtered
                            // burst collection can report more hits than the
                            // index will return, and the terminal page is empty
                            // with no cursor. There is nothing left to request,
                            // so the part is exhausted rather than failed, but
                            // the short walk is still reported honestly: the
                            // batch stays possibly incomplete (SAR-REG-008).
                            part.exhausted = true;
                            part.served_fewer_than_hits = true;
                        }
                    } else if let Some(next) = next_search_after {
                        if part.search_after.as_deref() == Some(next.as_str()) {
                            return Err(burst_pagination_state_error(
                                "repeated_search_after",
                                claimed_total,
                                part.fetched,
                                page_len,
                                page_size,
                            ));
                        }
                        part.search_after = Some(next);
                    } else {
                        return Err(burst_pagination_state_error(
                            "missing_search_after",
                            claimed_total,
                            part.fetched,
                            page_len,
                            page_size,
                        ));
                    }
                } else if part.fetched >= claimed_total {
                    part.exhausted = true;
                } else if page_len < page_size {
                    return Err(burst_pagination_state_error(
                        "short_incomplete",
                        claimed_total,
                        part.fetched,
                        page_len,
                        page_size,
                    ));
                } else {
                    part.next_page = part
                        .next_page
                        .checked_add(1)
                        .ok_or_else(|| burst_error("ASF_PAGINATION_INVALID", "page", "overflow"))?;
                }
            }
            if !made_request || parts.iter().all(|part| part.exhausted) {
                break;
            }
        }
        let remote_exhausted = parts.iter().all(|part| part.exhausted);
        let served_fewer_than_hits = parts.iter().any(|part| part.served_fewer_than_hits);
        let remote_total = (transport_part_count == 1).then(|| parts[0].hits).flatten();
        let mut products = products.into_values().collect::<Vec<_>>();
        products.truncate(safety_limit as usize);
        deadline.check(BurstDeadlineStage::BeforeResponse)?;
        // A terminal empty Search-After page with `fetched < hits` means the
        // remote total cannot be confirmed by what the index actually served.
        // The walk is complete, but the response must stay honest: mark the
        // batch possibly incomplete and attach a typed warning (SAR-REG-008).
        let warnings = if served_fewer_than_hits {
            let under = parts
                .iter()
                .filter(|part| part.served_fewer_than_hits)
                .filter_map(|part| part.hits.zip(Some(part.fetched)))
                .map(|(hits, fetched)| hits.saturating_sub(fetched))
                .try_fold(0_u64, |sum, gap| sum.checked_add(gap))
                .unwrap_or_default();
            vec![ContractError::new(
                "ASF_SEARCH_POSSIBLY_INCOMPLETE",
                "ASF 报告的匹配总数多于实际返回的记录",
                Map::from_iter([
                    (
                        "reason".to_owned(),
                        Value::String("served_fewer_than_hits".to_owned()),
                    ),
                    ("part_count".to_owned(), Value::from(transport_part_count)),
                    ("unserved_count".to_owned(), Value::from(under)),
                ]),
            )
            .expect("bounded Burst warning context is safe")]
        } else {
            Vec::new()
        };
        Ok(SarRemoteSearchResponse {
            stats: insar_core::AsfSearchStats {
                requested_limit: plan.safety_limit,
                query_limit: plan.safety_limit,
                metadata_request_count: Some(request_count),
                total_count: remote_total,
                candidate_count: exact_matched_count,
                returned_count: products.len() as u64,
                source: "CMR_BURST".to_owned(),
                cmr_enriched: Some(false),
                matched_count: remote_total,
                fetched_record_count: Some(raw_fetched_count),
                replayed_record_count: Some(0),
                mapped_record_count: Some(mapped_count),
                mapping_error_count: Some(errors.len() as u64),
                aggregated_asset_record_count: Some(
                    products
                        .iter()
                        .map(|product| product.assets.len() as u64)
                        .sum(),
                ),
                unique_provider_product_count: Some(products.len() as u64),
                asset_count: Some(
                    products
                        .iter()
                        .map(|product| product.assets.len() as u64)
                        .sum(),
                ),
                identity_conflict_count: Some(0),
                page_size: Some(effective_page_size),
                truncated: Some(!remote_exhausted || served_fewer_than_hits),
                item_errors: errors.clone(),
                warnings,
            },
            products,
            errors,
        })
    }

    pub fn count(
        &self,
        request: &SarSearchCountRequest,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<SarSearchCountResponse> {
        let started = Instant::now();
        check_cancelled(cancelled)?;
        let plan = build_burst_cmr_query(&request.query)?;
        let polygons = if plan.polygons.is_empty() {
            vec![String::new()]
        } else {
            plan.polygons.clone()
        };
        let mut total_count = None;
        for polygon in &polygons {
            check_cancelled(cancelled)?;
            check_elapsed(started, self.limits.max_elapsed)?;
            let value = self
                .transport
                .cmr_umm_json(&plan.page_params(polygon, 1, 1), cancelled)?;
            check_elapsed(started, self.limits.max_elapsed)?;
            check_cancelled(cancelled)?;
            if polygons.len() == 1 && !plan.requires_post_filter() {
                total_count = value.get("hits").and_then(Value::as_u64);
            }
        }
        check_elapsed(started, self.limits.max_elapsed)?;
        Ok(SarSearchCountResponse { total_count })
    }
}

pub fn map_burst_umm_granule(value: &Value) -> ProviderResult<SarProduct> {
    if serde_json::to_vec(value)
        .map_err(|_| {
            burst_error(
                "ASF_ASSET_CLASSIFICATION_INVALID",
                "record",
                "serialization",
            )
        })?
        .len()
        > MAX_RAW_BURST_RECORD_BYTES
    {
        return Err(burst_error(
            "ASF_ASSET_CLASSIFICATION_INVALID",
            "record",
            "size",
        ));
    }
    let meta = object(value, "meta")?;
    let umm = object(value, "umm")?;
    let native_id = text(meta, "native-id")?;
    let concept_id = text(meta, "concept-id")?;
    let collection_id = text(meta, "collection-concept-id")?;
    if text(meta, "concept-type")? != "granule"
        || text(meta, "provider-id")? != "ASF"
        || text(meta, "format")? != "application/vnd.nasa.cmr.umm+json"
    {
        return Err(burst_error(
            "ASF_COLLECTION_IDENTITY_MISMATCH",
            "cmr_meta",
            "authority",
        ));
    }
    if collection_id != BURST_COLLECTION_CONCEPT_ID {
        return Err(burst_error(
            "ASF_COLLECTION_IDENTITY_MISMATCH",
            "collection",
            "concept_id",
        ));
    }
    let collection_reference = map_object(umm, "CollectionReference")?;
    if text(collection_reference, "ShortName")? != BURST_COLLECTION_SHORT_NAME
        || text(collection_reference, "Version")? != BURST_COLLECTION_VERSION
    {
        return Err(burst_error(
            "ASF_COLLECTION_IDENTITY_MISMATCH",
            "collection",
            "short_name_or_version",
        ));
    }
    let granule_ur = text(umm, "GranuleUR")?;
    if native_id != granule_ur {
        return Err(burst_error(
            "ASF_PRODUCT_IDENTITY_MISSING",
            "native_id",
            "granule_ur",
        ));
    }
    let metadata_specification = map_object(umm, "MetadataSpecification")?;
    if text(metadata_specification, "Name")? != "UMM-G"
        || !RECOGNIZED_UMM_G_VERSIONS.contains(&text(metadata_specification, "Version")?.as_str())
    {
        return Err(burst_error(
            "ASF_PRODUCT_SCOPE_MISMATCH",
            "metadata_specification",
            "unrecognized",
        ));
    }
    let attributes = attributes(umm)?;
    let full_burst_id = attribute(&attributes, "BURST_ID_FULL")?;
    let relative_burst_id = attribute_positive_i64(&attributes, "BURST_ID_RELATIVE")?;
    let absolute_burst_id = attribute_positive_i64(&attributes, "BURST_ID_ABSOLUTE")?;
    let burst_index = attribute_i64(&attributes, "BURST_INDEX")?;
    let beam_swath = attribute(&attributes, "SUBSWATH_NAME")?;
    let polarization = attribute(&attributes, "POLARIZATION")?;
    let flight_direction = attribute(&attributes, "ASCENDING_DESCENDING")?;
    let relative_orbit = attribute_positive_i64(&attributes, "PATH_NUMBER")?;
    let byte_length = attribute_positive_i64(&attributes, "BYTE_LENGTH")?;
    let beam_mode = attribute(&attributes, "BEAM_MODE")?;
    if !matches!(
        (beam_mode.as_str(), beam_swath.as_str()),
        ("IW", "IW1" | "IW2" | "IW3") | ("EW", "EW1" | "EW2" | "EW3" | "EW4" | "EW5")
    ) {
        return Err(burst_error(
            "ASF_PRODUCT_SCOPE_MISMATCH",
            "beam_mode",
            "subswath_capability",
        ));
    }
    if attribute(&attributes, "PROCESSING_LEVEL")? != "L1"
        || attribute(&attributes, "PROCESSING_TYPE")? != "BURST"
    {
        return Err(burst_error(
            "ASF_PRODUCT_SCOPE_MISMATCH",
            "processing",
            "official_value",
        ));
    }
    if !matches!(flight_direction.as_str(), "ASCENDING" | "DESCENDING") {
        return Err(burst_error(
            "ASF_PRODUCT_SCOPE_MISMATCH",
            "flight_direction",
            "official_value",
        ));
    }
    let temporal = map_object(umm, "TemporalExtent")?;
    let range = map_object(temporal, "RangeDateTime")?;
    let acquisition_start = canonical_time(&text(range, "BeginningDateTime")?)?;
    let acquisition_stop = canonical_time(&text(range, "EndingDateTime")?)?;
    let absolute_orbit = umm
        .get("OrbitCalculatedSpatialDomains")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(Value::as_object)
        .and_then(|item| item.get("OrbitNumber"))
        .and_then(Value::as_i64)
        .ok_or_else(|| burst_error("ASF_PRODUCT_SCOPE_MISMATCH", "orbit", "missing"))?;
    let platform = umm
        .get("Platforms")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(Value::as_object)
        .ok_or_else(|| burst_error("ASF_PRODUCT_SCOPE_MISMATCH", "platform", "missing"))?;
    let platform_name = text(platform, "ShortName")?;
    let instrument = platform
        .get("Instruments")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(Value::as_object)
        .ok_or_else(|| burst_error("ASF_PRODUCT_SCOPE_MISMATCH", "instrument", "missing"))?;
    let instrument_name = text(instrument, "ShortName")?;
    let look_direction = instrument_look_direction(instrument)?;
    if !matches!(
        platform_name.as_str(),
        "SENTINEL-1A" | "SENTINEL-1B" | "SENTINEL-1C" | "SENTINEL-1D"
    ) || instrument_name != "C-SAR"
    {
        return Err(burst_error(
            "ASF_PRODUCT_SCOPE_MISMATCH",
            "platform_instrument",
            "unsupported",
        ));
    }
    let footprint_geojson = footprint(umm)?;
    let product_key = format!("ASF::SLC-BURST::{native_id}");
    let assets = burst_assets(umm, &product_key, &native_id, byte_length)?;
    let byte_length_remote = u64::try_from(byte_length)
        .map_err(|_| burst_error("ASF_PRODUCT_SCOPE_MISMATCH", "BYTE_LENGTH", "positive"))?;
    let browse = burst_browse(umm);
    let authority_projection = burst_authority_projection(meta, umm)?;
    let native_metadata = Map::from_iter([
        (
            "cmr_granule_concept_id".to_owned(),
            Value::String(concept_id.clone()),
        ),
        ("cmr_native_id".to_owned(), Value::String(native_id.clone())),
        (
            "raw_authority".to_owned(),
            Value::Object(authority_projection),
        ),
        (
            "collection".to_owned(),
            json!({
                "concept_id": collection_id.clone(),
                "short_name": BURST_COLLECTION_SHORT_NAME,
                "version": BURST_COLLECTION_VERSION,
                "provider_id": "ASF"
            }),
        ),
        (
            "burst".to_owned(),
            json!({
                "full_id": full_burst_id.clone(),
                "relative_id": relative_burst_id,
                "absolute_id": absolute_burst_id,
                "index": burst_index,
                "subswath": beam_swath.clone(),
                "byte_length": byte_length
            }),
        ),
        (
            "scope".to_owned(),
            json!({
                "beam_mode": beam_mode.clone(),
                "processing_level": "L1",
                "processing_type": "BURST",
                "path": relative_orbit,
                "flight_direction": flight_direction.clone(),
                "platform": platform_name.clone(),
                "instrument": instrument_name.clone(),
                "look_direction": look_direction.clone()
            }),
        ),
        ("browse".to_owned(), browse.clone()),
        (
            "preview_available".to_owned(),
            Value::Bool(!browse.is_null()),
        ),
    ]);
    Ok(SarProduct {
        product_key: product_key.clone(),
        provider_product_key: Some(product_key),
        provider_product_id: Some(concept_id),
        canonical_product_key: None,
        identity_rule_id: Some("asf.sentinel-1.burst.v1".to_owned()),
        product_id: native_id.clone(),
        scene_id: Some(native_id.clone()),
        provider: "asf.catalog".to_owned(),
        dataset: "SLC-BURST".to_owned(),
        platform: Some(platform_name),
        instrument: Some(instrument_name),
        processing_level: Some("L1".to_owned()),
        product_family: "BURST".to_owned(),
        beam_mode: Some(beam_mode),
        beam_swath: Some(beam_swath.clone()),
        polarization: Some(polarization),
        flight_direction: Some(flight_direction),
        look_direction,
        relative_orbit: Some(relative_orbit),
        absolute_orbit: Some(absolute_orbit),
        frame: None,
        asf_frame: None,
        path: Some(relative_orbit),
        acquisition_start: Some(acquisition_start),
        acquisition_stop: Some(acquisition_stop),
        footprint_geojson: Some(footprint_geojson),
        footprint_bbox: None,
        download_url: None,
        file_name: None,
        file_size_remote: Some(byte_length_remote),
        checksum: None,
        local_path: None,
        native_metadata,
        burst: Some(SarBurstSelection {
            burst_id: None,
            full_burst_id: Some(full_burst_id),
            relative_burst_id: Some(relative_burst_id),
            absolute_burst_id: Some(absolute_burst_id),
            subswath: Some(beam_swath),
            burst_index: Some(burst_index),
        }),
        assets,
    })
}

fn burst_assets(
    umm: &Map<String, Value>,
    product_key: &str,
    product_id: &str,
    primary_size_bytes: i64,
) -> ProviderResult<Vec<ProviderAsset>> {
    let mut assets = Vec::new();
    for related in umm
        .get("RelatedUrls")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(related) = related.as_object() else {
            continue;
        };
        if related.get("Type").and_then(Value::as_str) != Some("USE SERVICE API") {
            continue;
        }
        let Some(url) = related.get("URL").and_then(Value::as_str) else {
            continue;
        };
        let Some((role, media_type)) = (match related.get("Format").and_then(Value::as_str) {
            Some("GeoTIFF") => Some((AssetRole::Primary, "image/tiff")),
            Some("XML") => Some((AssetRole::Metadata, "application/xml")),
            _ => None,
        }) else {
            continue;
        };
        let file_name = validate_burst_asset_url(url, media_type)?;
        let asset_key = format!("{product_id}:{file_name}");
        let stable_asset_key = stable_asset_key(product_key, &asset_key)
            .map_err(|reason| burst_error("ASF_ASSET_IDENTITY_INVALID", "asset", &reason))?;
        let is_primary = role == AssetRole::Primary;
        assets.push(ProviderAsset {
            asset_key: Some(asset_key),
            stable_asset_key: Some(stable_asset_key),
            access_descriptor: None,
            role,
            native_roles: vec!["USE SERVICE API".to_owned()],
            url: Some(url.to_owned()),
            file_name: Some(file_name),
            media_type: Some(media_type.to_owned()),
            size_bytes: is_primary.then_some(primary_size_bytes),
        });
    }
    assets.sort_by_key(|asset| match asset.role {
        AssetRole::Primary => 0,
        AssetRole::Metadata => 1,
        _ => 2,
    });
    if !matches!(assets.as_slice(), [primary, metadata]
        if primary.role == AssetRole::Primary && metadata.role == AssetRole::Metadata)
    {
        return Err(burst_error(
            "ASF_ASSET_CLASSIFICATION_INVALID",
            "assets",
            "admitted_routes",
        ));
    }
    Ok(assets)
}

fn validate_burst_asset_url(url: &str, media_type: &str) -> ProviderResult<String> {
    if url
        .chars()
        .any(|character| character.is_control() || character.is_whitespace())
    {
        return Err(burst_error(
            "ASF_ASSET_CLASSIFICATION_INVALID",
            "asset",
            "url_trust",
        ));
    }
    let parsed = Url::parse(url)
        .map_err(|_| burst_error("ASF_ASSET_CLASSIFICATION_INVALID", "asset", "url"))?;
    if parsed.scheme() != "https"
        || parsed.host_str() != Some(BURST_ASSET_HOST)
        || parsed.port_or_known_default() != Some(443)
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(burst_error(
            "ASF_ASSET_CLASSIFICATION_INVALID",
            "asset",
            "url_trust",
        ));
    }
    let file_name = parsed
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .filter(|name| {
            !name.is_empty()
                && *name != "."
                && *name != ".."
                && name.len() <= 255
                && !name.contains('%')
                && !name.chars().any(char::is_whitespace)
                && !name.chars().any(char::is_control)
        })
        .ok_or_else(|| burst_error("ASF_ASSET_CLASSIFICATION_INVALID", "asset", "file_name"))?;
    let expected_extension = match media_type {
        "image/tiff" => ".tiff",
        "application/xml" => ".xml",
        _ => {
            return Err(burst_error(
                "ASF_ASSET_CLASSIFICATION_INVALID",
                "asset",
                "media_type",
            ))
        }
    };
    if !file_name.ends_with(expected_extension) {
        return Err(burst_error(
            "ASF_ASSET_CLASSIFICATION_INVALID",
            "asset",
            "extension",
        ));
    }
    Ok(file_name.to_owned())
}

fn attributes(umm: &Map<String, Value>) -> ProviderResult<BTreeMap<String, String>> {
    let values = umm
        .get("AdditionalAttributes")
        .and_then(Value::as_array)
        .ok_or_else(|| burst_error("ASF_PRODUCT_SCOPE_MISMATCH", "attributes", "missing"))?;
    let mut output = BTreeMap::new();
    for value in values {
        let value = value
            .as_object()
            .ok_or_else(|| burst_error("ASF_PRODUCT_SCOPE_MISMATCH", "attributes", "shape"))?;
        let name = text(value, "Name")?;
        let Some(raw) = value
            .get("Values")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
        else {
            continue;
        };
        let Some(raw) = raw.as_str() else { continue };
        if output.insert(name.clone(), raw.to_owned()).is_some() {
            return Err(burst_error(
                "ASF_PRODUCT_SCOPE_MISMATCH",
                "attributes",
                "duplicate",
            ));
        }
    }
    Ok(output)
}

fn attribute(attributes: &BTreeMap<String, String>, name: &'static str) -> ProviderResult<String> {
    attributes
        .get(name)
        .filter(|value| !value.trim().is_empty())
        .cloned()
        .ok_or_else(|| burst_error("ASF_PRODUCT_SCOPE_MISMATCH", name, "missing"))
}

fn attribute_i64(attributes: &BTreeMap<String, String>, name: &'static str) -> ProviderResult<i64> {
    attribute(attributes, name)?
        .parse()
        .ok()
        .filter(|value| *value >= 0)
        .ok_or_else(|| burst_error("ASF_PRODUCT_SCOPE_MISMATCH", name, "integer"))
}

fn attribute_positive_i64(
    attributes: &BTreeMap<String, String>,
    name: &'static str,
) -> ProviderResult<i64> {
    let value = attribute_i64(attributes, name)?;
    (value > 0)
        .then_some(value)
        .ok_or_else(|| burst_error("ASF_PRODUCT_SCOPE_MISMATCH", name, "positive"))
}

fn burst_browse(umm: &Map<String, Value>) -> Value {
    let urls = umm
        .get("RelatedUrls")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .filter(|related| {
            matches!(
                related.get("Type").and_then(Value::as_str),
                Some("BROWSE" | "GET RELATED VISUALIZATION")
            )
        })
        .filter_map(|related| related.get("URL").and_then(Value::as_str))
        .filter(|url| safe_burst_browse_url(url))
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    if urls.is_empty() {
        Value::Null
    } else {
        json!({"urls": urls})
    }
}

fn safe_burst_browse_url(url: &str) -> bool {
    let Ok(parsed) = Url::parse(url) else {
        return false;
    };
    parsed.scheme() == "https"
        && parsed.host_str() == Some(BURST_ASSET_HOST)
        && parsed.port_or_known_default() == Some(443)
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.query().is_none()
        && parsed.fragment().is_none()
        && !url
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
}

fn burst_authority_projection(
    meta: &Map<String, Value>,
    umm: &Map<String, Value>,
) -> ProviderResult<Map<String, Value>> {
    let projection = Map::from_iter([
        (
            "meta".to_owned(),
            sanitize_authority_value(&Value::Object(meta.clone())),
        ),
        (
            "umm".to_owned(),
            sanitize_authority_value(&Value::Object(umm.clone())),
        ),
    ]);
    if serde_json::to_vec(&projection)
        .map_err(|_| {
            burst_error(
                "ASF_ASSET_CLASSIFICATION_INVALID",
                "authority",
                "serialization",
            )
        })?
        .len()
        > MAX_RAW_BURST_RECORD_BYTES
    {
        return Err(burst_error(
            "ASF_ASSET_CLASSIFICATION_INVALID",
            "authority",
            "size",
        ));
    }
    Ok(projection)
}

fn sanitize_authority_value(value: &Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.iter().map(sanitize_authority_value).collect()),
        Value::Object(values) => Value::Object(
            values
                .iter()
                .filter(|(key, _)| !is_sensitive_authority_key(key))
                .map(|(key, value)| {
                    let value = if key.eq_ignore_ascii_case("url") {
                        sanitize_authority_url(value)
                    } else {
                        sanitize_authority_value(value)
                    };
                    (key.clone(), value)
                })
                .collect(),
        ),
        _ => value.clone(),
    }
}

fn is_sensitive_authority_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "token",
        "credential",
        "authorization",
        "password",
        "cookie",
        "secret",
    ]
    .iter()
    .any(|needle| key.contains(needle))
}

fn sanitize_authority_url(value: &Value) -> Value {
    let Some(url) = value.as_str() else {
        return sanitize_authority_value(value);
    };
    let Ok(mut parsed) = Url::parse(url) else {
        return Value::String(url.to_owned());
    };
    let had_userinfo = !parsed.username().is_empty() || parsed.password().is_some();
    parsed.set_query(None);
    parsed.set_fragment(None);
    if had_userinfo {
        let _ = parsed.set_username("");
        let _ = parsed.set_password(None);
    }
    Value::String(parsed.into())
}

/// The instrument's LookDirection characteristic, when the record carries one.
///
/// ASF's Sentinel-1D burst records (observed 2026-10-20) list the C-SAR
/// instrument without any Characteristics. An absent look direction stays
/// unknown (`None`): it is never inferred from the mission. A present
/// characteristic list must still be an array whose LookDirection, if any,
/// is a single LEFT or RIGHT.
fn instrument_look_direction(instrument: &Map<String, Value>) -> ProviderResult<Option<String>> {
    let Some(characteristics) = instrument.get("Characteristics") else {
        return Ok(None);
    };
    let characteristics = characteristics
        .as_array()
        .ok_or_else(|| burst_error("ASF_PRODUCT_SCOPE_MISMATCH", "look_direction", "shape"))?;
    let mut values = characteristics
        .iter()
        .filter_map(Value::as_object)
        .filter(|characteristic| {
            characteristic.get("Name").and_then(Value::as_str) == Some("LookDirection")
        })
        .map(|characteristic| text(characteristic, "Value"))
        .collect::<ProviderResult<Vec<_>>>()?;
    match values.as_mut_slice() {
        [] => Ok(None),
        [value] => match value.as_str() {
            "LEFT" | "RIGHT" => Ok(Some(value.clone())),
            _ => Err(burst_error(
                "ASF_PRODUCT_SCOPE_MISMATCH",
                "look_direction",
                "official_value",
            )),
        },
        _ => Err(burst_error(
            "ASF_PRODUCT_SCOPE_MISMATCH",
            "look_direction",
            "cardinality",
        )),
    }
}

fn footprint(umm: &Map<String, Value>) -> ProviderResult<Map<String, Value>> {
    let polygons = umm
        .get("SpatialExtent")
        .and_then(Value::as_object)
        .and_then(|value| value.get("HorizontalSpatialDomain"))
        .and_then(Value::as_object)
        .and_then(|value| value.get("Geometry"))
        .and_then(Value::as_object)
        .and_then(|value| value.get("GPolygons"))
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty() && items.len() <= MAX_BURST_GPOLYGONS)
        .ok_or_else(|| burst_error("ASF_PRODUCT_SCOPE_MISMATCH", "footprint", "missing"))?;
    let rings = polygons
        .iter()
        .map(|polygon| {
            let points = polygon
                .as_object()
                .and_then(|value| value.get("Boundary"))
                .and_then(Value::as_object)
                .and_then(|value| value.get("Points"))
                .and_then(Value::as_array)
                .filter(|points| points.len() >= 4 && points.len() <= MAX_BURST_POINTS_PER_GPOLYGON)
                .ok_or_else(|| burst_error("ASF_PRODUCT_SCOPE_MISMATCH", "footprint", "points"))?;
            let coordinates = points
                .iter()
                .map(|point| {
                    let point = point.as_object().ok_or_else(|| {
                        burst_error("ASF_PRODUCT_SCOPE_MISMATCH", "footprint", "point")
                    })?;
                    let latitude = point.get("Latitude").and_then(Value::as_f64);
                    let longitude = point.get("Longitude").and_then(Value::as_f64);
                    match (longitude, latitude) {
                        (Some(longitude), Some(latitude))
                            if longitude.is_finite()
                                && latitude.is_finite()
                                && (-180.0..=180.0).contains(&longitude)
                                && (-90.0..=90.0).contains(&latitude) =>
                        {
                            Ok(json!([longitude, latitude]))
                        }
                        _ => Err(burst_error(
                            "ASF_PRODUCT_SCOPE_MISMATCH",
                            "footprint",
                            "coordinate",
                        )),
                    }
                })
                .collect::<ProviderResult<Vec<_>>>()?;
            if coordinates.first() != coordinates.last() {
                return Err(burst_error(
                    "ASF_PRODUCT_SCOPE_MISMATCH",
                    "footprint",
                    "closure",
                ));
            }
            Ok(Value::Array(coordinates))
        })
        .collect::<ProviderResult<Vec<_>>>()?;
    let point_counts = rings
        .iter()
        .map(|ring| ring.as_array().map(Vec::len))
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| burst_error("ASF_PRODUCT_SCOPE_MISMATCH", "footprint", "shape"))?;
    let total_points = point_counts
        .iter()
        .try_fold(0usize, |total, count| total.checked_add(*count))
        .ok_or_else(|| burst_error("ASF_PRODUCT_SCOPE_MISMATCH", "footprint", "size"))?;
    let within_core_projection_limits = point_counts
        .iter()
        .all(|count| *count <= MAX_POLYGONAL_COORDS_PER_RING)
        && total_points <= MAX_POLYGONAL_TOTAL_COORDS;
    let footprint = if rings.len() == 1 {
        Map::from_iter([
            ("type".to_owned(), Value::String("Polygon".to_owned())),
            ("coordinates".to_owned(), Value::Array(rings)),
        ])
    } else {
        Map::from_iter([
            ("type".to_owned(), Value::String("MultiPolygon".to_owned())),
            (
                "coordinates".to_owned(),
                Value::Array(
                    rings
                        .into_iter()
                        .map(|ring| Value::Array(vec![ring]))
                        .collect(),
                ),
            ),
        ])
    };
    if within_core_projection_limits {
        return validated_polygonal_geojson(&footprint)
            .map(|_| footprint)
            .ok_or_else(|| burst_error("ASF_PRODUCT_SCOPE_MISMATCH", "footprint", "topology"));
    }
    let raw_geometry = geojson::Geometry::from_json_value(Value::Object(footprint.clone()))
        .ok()
        .and_then(|geometry| Geometry::try_from(geometry.value).ok())
        .and_then(|geometry| match geometry {
            Geometry::Polygon(polygon) => Some(MultiPolygon(vec![polygon])),
            Geometry::MultiPolygon(polygons) => Some(polygons),
            _ => None,
        })
        .ok_or_else(|| burst_error("ASF_PRODUCT_SCOPE_MISMATCH", "footprint", "shape"))?;
    burst_large_footprint_has_strict_topology(&raw_geometry)
        .map_err(|reason| burst_error("ASF_PRODUCT_SCOPE_MISMATCH", "footprint", reason))?;
    Ok(footprint)
}

fn burst_large_footprint_has_strict_topology(
    geometry: &MultiPolygon<f64>,
) -> Result<(), &'static str> {
    if geometry.0.is_empty()
        || geometry.0.iter().any(|polygon| {
            !polygon.interiors().is_empty()
                || !polygon.unsigned_area().is_finite()
                || polygon.unsigned_area() <= 0.0
                || polygon.exterior().0.iter().any(|coordinate| {
                    !coordinate.x.is_finite()
                        || !coordinate.y.is_finite()
                        || !(-180.0..=180.0).contains(&coordinate.x)
                        || !(-90.0..=90.0).contains(&coordinate.y)
                })
        })
    {
        return Err("topology");
    }
    let required_work = burst_large_topology_work(geometry).ok_or("topology_work_ceiling")?;
    if required_work > MAX_BURST_TOPOLOGY_EDGE_PAIRS {
        return Err("topology_work_ceiling");
    }
    let mut remaining_pairs = MAX_BURST_TOPOLOGY_EDGE_PAIRS;
    if !geometry.0.iter().all(|polygon| {
        polygon.interiors().is_empty()
            && burst_ring_is_simple(&polygon.exterior().0, &mut remaining_pairs)
    }) {
        return Err("topology");
    }
    for (index, polygon) in geometry.0.iter().enumerate() {
        let first_edges =
            u64::try_from(polygon.exterior().0.len() - 1).map_err(|_| "topology_work_ceiling")?;
        for other in &geometry.0[index + 1..] {
            let second_edges =
                u64::try_from(other.exterior().0.len() - 1).map_err(|_| "topology_work_ceiling")?;
            let work =
                cross_component_work(first_edges, second_edges).ok_or("topology_work_ceiling")?;
            remaining_pairs = remaining_pairs
                .checked_sub(work)
                .ok_or("topology_work_ceiling")?;
            if polygon.intersects(other) {
                return Err("topology");
            }
        }
    }
    Ok(())
}

fn burst_large_topology_work(geometry: &MultiPolygon<f64>) -> Option<u64> {
    let edges = geometry
        .0
        .iter()
        .map(|polygon| u64::try_from(polygon.exterior().0.len().checked_sub(1)?).ok())
        .collect::<Option<Vec<_>>>()?;
    let mut required = 0_u64;
    for edge_count in &edges {
        let non_adjacent_pairs = edge_count
            .checked_mul(edge_count.checked_sub(3)?)?
            .checked_div(2)?;
        required = required.checked_add(non_adjacent_pairs)?;
    }
    for (index, first) in edges.iter().enumerate() {
        for second in &edges[index + 1..] {
            required = required.checked_add(cross_component_work(*first, *second)?)?;
        }
    }
    Some(required)
}

fn cross_component_work(first_edges: u64, second_edges: u64) -> Option<u64> {
    first_edges
        .checked_mul(second_edges)?
        .checked_add(first_edges)?
        .checked_add(second_edges)
}

fn burst_ring_is_simple(coordinates: &[geo::Coord<f64>], remaining_pairs: &mut u64) -> bool {
    if coordinates.len() < 4 || coordinates.first() != coordinates.last() {
        return false;
    }
    let unique = coordinates[..coordinates.len() - 1]
        .iter()
        .map(|coordinate| (coordinate.x.to_bits(), coordinate.y.to_bits()))
        .collect::<BTreeSet<_>>();
    if unique.len() + 1 != coordinates.len() {
        return false;
    }
    let edge_count = coordinates.len() - 1;
    for first in 0..edge_count {
        for second in (first + 1)..edge_count {
            let adjacent = second == first + 1 || (first == 0 && second + 1 == edge_count);
            if adjacent {
                continue;
            }
            let Some(next_budget) = remaining_pairs.checked_sub(1) else {
                return false;
            };
            *remaining_pairs = next_budget;
            if line_intersection(
                Line::new(coordinates[first], coordinates[first + 1]),
                Line::new(coordinates[second], coordinates[second + 1]),
            )
            .is_some()
            {
                return false;
            }
        }
    }
    true
}

fn object<'a>(value: &'a Value, key: &'static str) -> ProviderResult<&'a Map<String, Value>> {
    value
        .get(key)
        .and_then(Value::as_object)
        .ok_or_else(|| burst_error("ASF_ASSET_CLASSIFICATION_INVALID", key, "missing"))
}

fn map_object<'a>(
    value: &'a Map<String, Value>,
    key: &'static str,
) -> ProviderResult<&'a Map<String, Value>> {
    value
        .get(key)
        .and_then(Value::as_object)
        .ok_or_else(|| burst_error("ASF_ASSET_CLASSIFICATION_INVALID", key, "missing"))
}

fn text(value: &Map<String, Value>, key: &'static str) -> ProviderResult<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| burst_error("ASF_ASSET_CLASSIFICATION_INVALID", key, "missing"))
}

fn canonical_time(value: &str) -> ProviderResult<String> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| {
            value
                .with_timezone(&Utc)
                .to_rfc3339_opts(SecondsFormat::AutoSi, true)
        })
        .map_err(|_| burst_error("ASF_PRODUCT_SCOPE_MISMATCH", "temporal", "invalid"))
}

fn temporal(start: Option<&str>, end: Option<&str>) -> ProviderResult<Option<String>> {
    let start = start.filter(|value| !value.trim().is_empty());
    let end = end.filter(|value| !value.trim().is_empty());
    let (start, end) = match (start, end) {
        (None, None) => return Ok(None),
        (Some(start), Some(end)) => (start, end),
        (Some(_), None) => return Err(burst_error("ASF_QUERY_INVALID", "end", "paired_temporal")),
        (None, Some(_)) => {
            return Err(burst_error("ASF_QUERY_INVALID", "start", "paired_temporal"))
        }
    };
    let start = canonical_query_time(start, false, "start")?;
    let end = canonical_query_time(end, true, "end")?;
    Ok(Some(format!("{start},{end}")))
}

fn canonical_query_time(value: &str, end: bool, field: &'static str) -> ProviderResult<String> {
    let expanded = asf_datetime(value, end)
        .map_err(|_| burst_error("ASF_QUERY_INVALID", field, "temporal"))?
        .ok_or_else(|| burst_error("ASF_QUERY_INVALID", field, "required"))?;
    DateTime::parse_from_rfc3339(&expanded)
        .map(|value| {
            value
                .with_timezone(&Utc)
                .to_rfc3339_opts(SecondsFormat::AutoSi, true)
        })
        .map_err(|_| burst_error("ASF_QUERY_INVALID", field, "temporal"))
}

fn optional_burst_text(value: Option<&str>, field: &'static str) -> ProviderResult<Option<String>> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let parts = value.split('_').collect::<Vec<_>>();
    let valid_swath = parts.get(2).is_some_and(|swath| {
        matches!(
            *swath,
            "IW1" | "IW2" | "IW3" | "EW1" | "EW2" | "EW3" | "EW4" | "EW5"
        )
    });
    if parts.len() != 3
        || parts[0].len() != 3
        || parts[1].len() != 6
        || !parts[0].chars().all(|character| character.is_ascii_digit())
        || !parts[1].chars().all(|character| character.is_ascii_digit())
        || !valid_swath
    {
        return Err(burst_error("ASF_QUERY_INVALID", field, "official_format"));
    }
    Ok(Some(value.to_ascii_uppercase()))
}

fn optional_positive(value: Option<i64>, field: &'static str) -> ProviderResult<Option<i64>> {
    value
        .map(|value| {
            (value > 0)
                .then_some(value)
                .ok_or_else(|| burst_error("ASF_QUERY_INVALID", field, "positive"))
        })
        .transpose()
}

fn check_cancelled(cancelled: &(dyn Fn() -> bool + Sync)) -> ProviderResult<()> {
    if cancelled() {
        Err(burst_error("DL001", "cancelled", "true"))
    } else {
        Ok(())
    }
}

fn check_elapsed(started: Instant, max_elapsed: Duration) -> ProviderResult<()> {
    if started.elapsed() > max_elapsed {
        Err(burst_error("ASF_PAGINATION_INVALID", "elapsed", "ceiling"))
    } else {
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
fn burst_progress(
    stage: SarSearchProviderProgressStage,
    request_count: u64,
    transport_part_count: u64,
    raw_fetched_count: u64,
    mapped_count: u64,
    unique_admitted_count: u64,
    requested_limit: Option<u64>,
    remote_total: Option<u64>,
) -> SarSearchProviderProgress {
    SarSearchProviderProgress {
        stage,
        provider_request_count: request_count,
        transport_part_count,
        raw_fetched_count,
        mapped_count,
        unique_admitted_count,
        requested_limit,
        remote_total,
        fetched_count: unique_admitted_count,
        total_count: requested_limit.or(remote_total),
    }
}

fn burst_error(code: &'static str, field: &'static str, reason: &str) -> ContractError {
    ContractError::new(
        code,
        "ASF Burst CMR request or response is invalid",
        Map::from_iter([
            ("field".to_owned(), Value::String(field.to_owned())),
            ("reason".to_owned(), Value::String(reason.to_owned())),
        ]),
    )
    .expect("bounded Burst error context is safe")
}

fn burst_pagination_state_error(
    reason: &'static str,
    claimed_total: u64,
    fetched: u64,
    page_len: u64,
    requested_page_size: u64,
) -> ContractError {
    ContractError::new(
        "ASF_PAGINATION_INVALID",
        "ASF Burst CMR request or response is invalid",
        Map::from_iter([
            ("field".to_owned(), Value::String("page".to_owned())),
            ("reason".to_owned(), Value::String(reason.to_owned())),
            ("claimed_total".to_owned(), Value::from(claimed_total)),
            ("fetched".to_owned(), Value::from(fetched)),
            ("page_len".to_owned(), Value::from(page_len)),
            (
                "requested_page_size".to_owned(),
                Value::from(requested_page_size),
            ),
        ]),
    )
    .expect("bounded Burst pagination context is safe")
}

#[cfg(test)]
mod deadline_tests {
    use std::sync::Mutex;

    use super::*;

    const FIXTURE: &str =
        include_str!("../../tests/fixtures/asf/burst/sentinel-1-bursts-first-page.json");

    struct OnePageTransport {
        response: Value,
        requests: Mutex<u64>,
    }

    impl MetadataTransport for OnePageTransport {
        fn asf_geojson(
            &self,
            _params: &BTreeMap<String, String>,
            _cancelled: &(dyn Fn() -> bool + Sync),
        ) -> ProviderResult<Value> {
            panic!("Burst test must not call SearchAPI")
        }

        fn asf_count(
            &self,
            _params: &BTreeMap<String, String>,
            _cancelled: &(dyn Fn() -> bool + Sync),
        ) -> ProviderResult<Option<u64>> {
            panic!("Burst test must not call SearchAPI count")
        }

        fn cmr_json(
            &self,
            _params: &[(String, String)],
            _cancelled: &(dyn Fn() -> bool + Sync),
        ) -> ProviderResult<Value> {
            *self.requests.lock().unwrap() += 1;
            Ok(self.response.clone())
        }
    }

    struct FailingStageDeadline {
        fail_at: BurstDeadlineStage,
        seen: Mutex<Vec<BurstDeadlineStage>>,
    }

    impl BurstDeadline for FailingStageDeadline {
        fn check(&self, stage: BurstDeadlineStage) -> ProviderResult<()> {
            self.seen.lock().unwrap().push(stage);
            if stage == self.fail_at {
                Err(burst_error("ASF_PAGINATION_INVALID", "elapsed", "ceiling"))
            } else {
                Ok(())
            }
        }
    }

    fn request() -> SarRemoteSearchRequest {
        SarRemoteSearchRequest {
            query: serde_json::from_value(json!({
                "provider": "ASF",
                "dataset": "SLC-BURST",
                "start": "2014-10-03T00:00:00Z",
                "end": "2014-10-04T00:00:00Z",
                "max_results": 1
            }))
            .unwrap(),
        }
    }

    fn transport() -> OnePageTransport {
        let mut response: Value = serde_json::from_str(FIXTURE).unwrap();
        response["hits"] = json!(1);
        OnePageTransport {
            response,
            requests: Mutex::new(0),
        }
    }

    #[test]
    fn mapping_over_budget_fails_before_any_admission() {
        let transport = transport();
        let provider =
            BurstCmrProvider::with_limits(&transport, BurstSearchLimits::default()).unwrap();
        let deadline = FailingStageDeadline {
            fail_at: BurstDeadlineStage::AfterMapping,
            seen: Mutex::new(Vec::new()),
        };
        let progress = Mutex::new(Vec::new());

        let error = provider
            .search_with_deadline(
                &request(),
                &|| false,
                &|value| progress.lock().unwrap().push(value),
                &deadline,
            )
            .unwrap_err();

        assert_eq!(error.debug_context["field"], "elapsed");
        assert_eq!(*transport.requests.lock().unwrap(), 1);
        assert_eq!(
            deadline
                .seen
                .lock()
                .unwrap()
                .iter()
                .filter(|stage| **stage == BurstDeadlineStage::AfterMapping)
                .count(),
            1
        );
        assert!(!deadline
            .seen
            .lock()
            .unwrap()
            .contains(&BurstDeadlineStage::AfterAdmission));
        assert!(progress
            .lock()
            .unwrap()
            .iter()
            .all(|value| value.stage != SarSearchProviderProgressStage::AdmittingResults));
    }

    #[test]
    fn response_cannot_succeed_when_budget_expires_after_admission() {
        let transport = transport();
        let provider =
            BurstCmrProvider::with_limits(&transport, BurstSearchLimits::default()).unwrap();
        let deadline = FailingStageDeadline {
            fail_at: BurstDeadlineStage::BeforeResponse,
            seen: Mutex::new(Vec::new()),
        };
        let progress = Mutex::new(Vec::new());

        let error = provider
            .search_with_deadline(
                &request(),
                &|| false,
                &|value| progress.lock().unwrap().push(value),
                &deadline,
            )
            .unwrap_err();

        assert_eq!(error.debug_context["reason"], "ceiling");
        assert!(deadline
            .seen
            .lock()
            .unwrap()
            .contains(&BurstDeadlineStage::AfterAdmission));
        assert!(deadline
            .seen
            .lock()
            .unwrap()
            .contains(&BurstDeadlineStage::BeforeResponse));
        assert!(progress
            .lock()
            .unwrap()
            .iter()
            .any(|value| value.stage == SarSearchProviderProgressStage::AdmittingResults));
    }
}
