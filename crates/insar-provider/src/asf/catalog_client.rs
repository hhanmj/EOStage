use std::collections::{BTreeMap, HashSet};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
#[cfg(feature = "nisar-subset-live-acceptance")]
use insar_core::{provider_product_key, AssetRole};
use insar_core::{
    CatalogCollectionDescriptor, CatalogSearchQuery, CatalogSearchResponse, CatalogSearchStats,
    ContractError, ProviderProduct, ProviderResult,
};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::catalog::{
    CollectionProfile, DatasetRoutePolicyError, DatasetRoutePolicyRegistry, RouteUse,
    ALOS2_PROVIDER_ID, ALOS2_SCAN_SAR_BEAM_MODES, NISAR_PROVIDER_ID,
};
#[cfg(feature = "nisar-subset-live-acceptance")]
use crate::catalog::{
    CollectionProfileRegistry, NISAR_NATIVE_COLLECTION_ID, NISAR_P05023_CRID,
    NISAR_P05023_PRODUCT_SPECIFICATION_VERSION, NISAR_PRODUCTION_CONFIGURATION, NISAR_PROFILE_ID,
    NISAR_QUALITY_STATUS, NISAR_REQUIRED_PRODUCT_TYPE,
};

use super::alos2::map_alos2_umm_granule;
use super::client::AsfHttpClient;
use super::nisar::{build_nisar_rslc_query, map_nisar_rslc_umm_granule};
#[cfg(feature = "nisar-subset-live-acceptance")]
use super::nisar_subset::{
    normalize_nisar_subset_aoi, validate_nisar_catalogue_footprint_intersection,
};
use super::query::{build_alos2_cmr_query, footprint_intersects_exact_catalogue_aoi, AsfCmrQuery};
use super::umm::CmrGranuleEnvelope;

pub const MAX_CATALOGUE_CANDIDATE_RECORDS: u64 = 40_000;

#[cfg(feature = "nisar-subset-live-acceptance")]
pub const NISAR_CATALOGUE_CANARY_GRANULE_ID: &str =
    "NISAR_L1_PR_RSLC_026_093_D_083_2005_DHDH_A_20260726T155803_20260726T155837_P05023_N_F_J_001";
#[cfg(feature = "nisar-subset-live-acceptance")]
pub const NISAR_CATALOGUE_CANARY_MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
#[cfg(feature = "nisar-subset-live-acceptance")]
const NISAR_CATALOGUE_CANARY_SOURCE_SIZE_BYTES: u64 = 14_252_244_992;
#[cfg(feature = "nisar-subset-live-acceptance")]
const NISAR_CATALOGUE_CANARY_SOURCE_MD5: &str = "059ada6521bff1ef314fb1141a89cd10";

#[cfg(feature = "nisar-subset-live-acceptance")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NisarExactIdentityCanaryResult {
    pub request_count: u8,
    pub http_status: u16,
    pub response_body_bytes: usize,
    pub query_parameter_digest: String,
    pub hits: u64,
    pub returned_item_count: u8,
    pub provider_id: String,
    pub collection_id: String,
    pub collection_profile_id: String,
    pub provider_product_id: String,
    pub provider_product_key: String,
    pub producer_granule_id: String,
    pub display_name: String,
    pub product_type: String,
    pub production_configuration: String,
    pub crid: String,
    pub product_specification_version: String,
    pub quality_status: String,
    pub primary_filename: String,
    pub primary_role: String,
    pub stable_asset_key: String,
    pub source_size_bytes: u64,
    pub source_checksum_algorithm: String,
    pub source_checksum_value: String,
    pub footprint_valid: bool,
    pub aoi_intersects: bool,
    pub primary_locator_present: bool,
}

#[derive(Clone, Copy)]
enum IntraPartDuplicatePolicy {
    Count,
    Reject,
}

/// Mapping a remote record can fail without making the whole catalogue
/// request unusable. Keep this distinction explicit so callers can publish
/// bounded per-record diagnostics while preserving the completed batch.
enum CatalogMapperOutcome {
    Product(ProviderProduct),
    ProfileFiltered,
    ItemError(ContractError),
}

struct CatalogPartState {
    polygon: String,
    next_page: u64,
    page_count: usize,
    hits: Option<u64>,
    hits_drifted: bool,
    fetched: u64,
    seen_pages: HashSet<[u8; 32]>,
    product_ids: HashSet<String>,
    exhausted: bool,
    budget_stopped: bool,
}

impl CatalogPartState {
    fn new(polygon: String) -> Self {
        Self {
            polygon,
            next_page: 1,
            page_count: 0,
            hits: None,
            hits_drifted: false,
            fetched: 0,
            seen_pages: HashSet::new(),
            product_ids: HashSet::new(),
            exhausted: false,
            budget_stopped: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CatalogSearchLimits {
    pub max_requests: usize,
    pub max_pages: usize,
    pub max_records: u64,
    pub max_progress_idle: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CatalogSearchProviderProgress {
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

impl Default for CatalogSearchLimits {
    fn default() -> Self {
        Self {
            max_requests: 128,
            max_pages: 64,
            max_records: MAX_CATALOGUE_CANDIDATE_RECORDS,
            max_progress_idle: Duration::from_secs(120),
        }
    }
}

pub struct AsfCatalogClient {
    http: AsfHttpClient,
    limits: CatalogSearchLimits,
    admission: CatalogRouteAdmission,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CatalogRouteAdmission {
    Standard(RouteUse),
    #[cfg(feature = "nisar-subset-live-acceptance")]
    NisarControlledLiveAcceptance,
}

impl AsfCatalogClient {
    pub fn production() -> ProviderResult<Self> {
        Self::production_with_network_policy(insar_core::ProviderNetworkPolicy::system())
    }

    pub fn production_with_network_policy(
        network_policy: insar_core::ProviderNetworkPolicy,
    ) -> ProviderResult<Self> {
        Self::build(
            AsfHttpClient::production_with_network_policy(network_policy)?,
            CatalogSearchLimits::default(),
            CatalogRouteAdmission::Standard(RouteUse::Production),
        )
    }

    #[cfg(feature = "nisar-subset-live-acceptance")]
    #[doc(hidden)]
    pub fn nisar_controlled_live_acceptance() -> ProviderResult<Self> {
        Self::build(
            AsfHttpClient::production()?,
            CatalogSearchLimits::default(),
            CatalogRouteAdmission::NisarControlledLiveAcceptance,
        )
    }

    #[cfg(feature = "nisar-subset-live-acceptance")]
    #[doc(hidden)]
    pub fn nisar_exact_identity_canary() -> ProviderResult<Self> {
        Self::build(
            AsfHttpClient::production_for_nisar_exact_identity_canary()?,
            CatalogSearchLimits {
                max_requests: 1,
                max_pages: 1,
                max_records: 1,
                max_progress_idle: Duration::from_secs(30),
            },
            CatalogRouteAdmission::NisarControlledLiveAcceptance,
        )
    }

    #[doc(hidden)]
    pub fn with_http_client_for_tests(
        http: AsfHttpClient,
        limits: CatalogSearchLimits,
    ) -> ProviderResult<Self> {
        Self::build(
            http,
            limits,
            CatalogRouteAdmission::Standard(RouteUse::OfflineFixture),
        )
    }

    #[cfg(feature = "nisar-subset-live-acceptance")]
    #[doc(hidden)]
    pub fn with_http_client_for_nisar_controlled_live_acceptance_for_tests(
        http: AsfHttpClient,
        limits: CatalogSearchLimits,
    ) -> ProviderResult<Self> {
        Self::build(
            http,
            limits,
            CatalogRouteAdmission::NisarControlledLiveAcceptance,
        )
    }

    #[cfg(feature = "nisar-subset-live-acceptance")]
    #[doc(hidden)]
    pub fn with_http_client_for_nisar_exact_identity_canary_for_tests(
        http: AsfHttpClient,
    ) -> ProviderResult<Self> {
        Self::build(
            http,
            CatalogSearchLimits {
                max_requests: 1,
                max_pages: 1,
                max_records: 1,
                max_progress_idle: Duration::from_secs(30),
            },
            CatalogRouteAdmission::NisarControlledLiveAcceptance,
        )
    }

    fn build(
        http: AsfHttpClient,
        limits: CatalogSearchLimits,
        admission: CatalogRouteAdmission,
    ) -> ProviderResult<Self> {
        if limits.max_requests == 0
            || limits.max_pages == 0
            || limits.max_records == 0
            || limits.max_records > MAX_CATALOGUE_CANDIDATE_RECORDS
            || limits.max_progress_idle.is_zero()
            || limits.max_progress_idle > Duration::from_secs(120)
        {
            return Err(catalogue_error(
                "CATALOGUE_CONFIGURATION_INVALID",
                "ASF 目录检索预算配置无效",
                "limits",
            ));
        }
        Ok(Self {
            http,
            limits,
            admission,
        })
    }

    fn resolve_route(
        &self,
        query: &CatalogSearchQuery,
    ) -> Result<&'static crate::catalog::DatasetRoutePolicy, DatasetRoutePolicyError> {
        match self.admission {
            CatalogRouteAdmission::Standard(route_use) => DatasetRoutePolicyRegistry.resolve(
                &query.logical_dataset,
                &query.provider_id,
                &query.native_collection_id,
                route_use,
            ),
            #[cfg(feature = "nisar-subset-live-acceptance")]
            CatalogRouteAdmission::NisarControlledLiveAcceptance => DatasetRoutePolicyRegistry
                .resolve_nisar_live_acceptance_candidate(
                    &query.logical_dataset,
                    &query.provider_id,
                    &query.native_collection_id,
                ),
        }
    }

    pub fn search_alos2(
        &self,
        query: &CatalogSearchQuery,
        profile: &CollectionProfile,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        self.search_alos2_with_progress(query, profile, cancelled, &|_| {})
    }

    pub fn search_alos2_with_progress(
        &self,
        query: &CatalogSearchQuery,
        profile: &CollectionProfile,
        cancelled: &(dyn Fn() -> bool + Sync),
        progress: &dyn Fn(CatalogSearchProviderProgress),
    ) -> ProviderResult<CatalogSearchResponse> {
        let query = &insar_core::clamp_catalogue_search_query(query);
        self.resolve_route(query).map_err(route_policy_error)?;
        let plan = build_alos2_cmr_query(query, profile)?;
        let spatial_transport_mode = plan.spatial_transport_mode;
        self.search_complete(
            profile,
            &plan,
            |item| {
                let envelope = CmrGranuleEnvelope::try_from(item)?;
                let beam_mode = envelope.granule.get("beam_mode").and_then(Value::as_str);
                if !beam_mode
                    .is_some_and(|beam_mode| ALOS2_SCAN_SAR_BEAM_MODES.contains(&beam_mode))
                {
                    return Ok(CatalogMapperOutcome::ProfileFiltered);
                }
                let mut product = map_alos2_umm_granule(&envelope, profile)?;
                product.properties.insert(
                    "spatial_transport_mode".to_owned(),
                    Value::String(spatial_transport_mode.to_owned()),
                );
                Ok(CatalogMapperOutcome::Product(product))
            },
            |product| {
                alos2_product_matches_query(product, query)
                    && (!plan.requires_exact_spatial_post_filter
                        || plan.canonical_geometry.as_ref().is_some_and(|aoi| {
                            product.footprint_geojson.as_ref().is_some_and(|footprint| {
                                footprint_intersects_exact_catalogue_aoi(footprint, aoi)
                            })
                        }))
            },
            IntraPartDuplicatePolicy::Count,
            cancelled,
            progress,
        )
    }

    pub fn search_nisar(
        &self,
        query: &CatalogSearchQuery,
        profile: &CollectionProfile,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        self.search_nisar_with_progress(query, profile, cancelled, &|_| {})
    }

    pub fn search_nisar_with_progress(
        &self,
        query: &CatalogSearchQuery,
        profile: &CollectionProfile,
        cancelled: &(dyn Fn() -> bool + Sync),
        progress: &dyn Fn(CatalogSearchProviderProgress),
    ) -> ProviderResult<CatalogSearchResponse> {
        let query = &insar_core::clamp_catalogue_search_query(query);
        self.resolve_route(query).map_err(route_policy_error)?;
        let plan = build_nisar_rslc_query(query, profile)?;
        self.search_complete(
            profile,
            &plan,
            |item| {
                let envelope = CmrGranuleEnvelope::try_from(item)?;
                match map_nisar_rslc_umm_granule(&envelope, profile) {
                    Ok(product) => Ok(CatalogMapperOutcome::Product(product)),
                    Err(error) if error.code == "ASF_PRODUCT_SCOPE_MISMATCH" => {
                        Ok(CatalogMapperOutcome::ItemError(error))
                    }
                    Err(error) => Err(error),
                }
            },
            |product| {
                nisar_product_matches_complete_batch_filters(
                    product,
                    &plan.complete_batch_local_filters,
                ) && (!plan.requires_exact_spatial_post_filter
                    || plan.canonical_geometry.as_ref().is_some_and(|aoi| {
                        product.footprint_geojson.as_ref().is_some_and(|footprint| {
                            footprint_intersects_exact_catalogue_aoi(footprint, aoi)
                        })
                    }))
            },
            IntraPartDuplicatePolicy::Reject,
            cancelled,
            progress,
        )
    }

    #[cfg(feature = "nisar-subset-live-acceptance")]
    #[doc(hidden)]
    pub fn search_nisar_exact_identity_canary(
        &self,
        collection_id: &str,
        producer_granule_id: &str,
        aoi_geojson: &Map<String, Value>,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<NisarExactIdentityCanaryResult> {
        if !matches!(
            self.admission,
            CatalogRouteAdmission::NisarControlledLiveAcceptance
        ) {
            return Err(canary_error("route_not_admitted"));
        }
        if collection_id != NISAR_NATIVE_COLLECTION_ID {
            return Err(canary_error("collection_identity"));
        }
        if producer_granule_id != NISAR_CATALOGUE_CANARY_GRANULE_ID {
            return Err(canary_error("producer_granule_identity"));
        }
        check_cancelled(cancelled)?;
        normalize_nisar_subset_aoi(aoi_geojson)
            .map_err(|_| canary_error("aoi_invalid_before_request"))?;

        let params = vec![
            ("collection_concept_id".to_owned(), collection_id.to_owned()),
            (
                "producer_granule_id[]".to_owned(),
                producer_granule_id.to_owned(),
            ),
            ("page_size".to_owned(), "2".to_owned()),
        ];
        let query_parameter_digest = serde_json::to_vec(&params)
            .map(|bytes| format!("{:x}", Sha256::digest(bytes)))
            .map_err(|_| canary_error("query_digest"))?;
        let (http_status, response_body_bytes, value) = self
            .http
            .cmr_catalog_json_with_status(&params, cancelled)
            .map_err(normalize_transport_error)?;
        if http_status != 200 {
            return Err(canary_error("http_status"));
        }
        check_cancelled(cancelled)?;
        let page = parse_page(&value, 2)?;
        if page.hits != 1 || page.items.len() != 1 {
            return Err(canary_error("cardinality"));
        }

        let envelope = CmrGranuleEnvelope::try_from(page.items.first().expect("one item"))?;
        let primary_locator_present = nisar_primary_locator_present(&envelope)?;
        if !primary_locator_present {
            return Err(canary_error("primary_locator_missing"));
        }
        let profile = CollectionProfileRegistry
            .get(NISAR_PROFILE_ID)
            .ok_or_else(|| canary_error("profile_missing"))?;
        // The formal mapper is used for identity and integrity validation. Its
        // mapped ProviderProduct is discarded below; this API never reads or
        // returns an access descriptor, URL, credential profile, or route.
        let product = map_nisar_rslc_umm_granule(&envelope, profile)?;
        let footprint = product
            .footprint_geojson
            .as_ref()
            .ok_or_else(|| canary_error("footprint_missing"))?;
        validate_nisar_catalogue_footprint_intersection(footprint, aoi_geojson)
            .map_err(|_| canary_error("aoi_no_overlap"))?;
        validate_nisar_exact_identity_product(
            product,
            query_parameter_digest,
            primary_locator_present,
            http_status,
            response_body_bytes,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn search_complete(
        &self,
        profile: &CollectionProfile,
        plan: &AsfCmrQuery,
        mapper: impl Fn(&Value) -> ProviderResult<CatalogMapperOutcome>,
        complete_batch_filter: impl Fn(&ProviderProduct) -> bool,
        intra_part_duplicate_policy: IntraPartDuplicatePolicy,
        cancelled: &(dyn Fn() -> bool + Sync),
        progress: &dyn Fn(CatalogSearchProviderProgress),
    ) -> ProviderResult<CatalogSearchResponse> {
        let started = Instant::now();
        let mut last_progress_at = started;
        check_cancelled(cancelled)?;
        check_progress_idle(last_progress_at, self.limits.max_progress_idle)?;

        let mut request_count = 0_usize;
        let mut fetched_record_count = 0_u64;
        let mut mapped_record_count = 0_u64;
        let mut profile_filtered_record_count = 0_u64;
        let mut exact_duplicate_record_count = 0_u64;
        let mut item_errors = Vec::<ContractError>::new();
        let mut unique = BTreeMap::<String, ProviderProduct>::new();
        let polygons = if plan.polygons.is_empty() {
            vec![String::new()]
        } else {
            plan.polygons.clone()
        };
        let finite_target = plan.result_limit;
        let transport_part_count = polygons.len() as u64;
        let candidate_budget = finite_target
            .map(|target| {
                target
                    .saturating_mul(transport_part_count.max(4))
                    .max(target.saturating_add(plan.provider_page_size))
                    .min(self.limits.max_records)
                    .max(1)
            })
            .unwrap_or(self.limits.max_records);
        let provider_page_size = plan.provider_page_size.min(candidate_budget).max(1);
        let mut parts = polygons
            .into_iter()
            .map(CatalogPartState::new)
            .collect::<Vec<_>>();
        let mut ordered_product_ids = Vec::<String>::new();
        'round_robin: loop {
            let mut made_request = false;
            for (part_index, part) in parts.iter_mut().enumerate() {
                if part.exhausted || part.budget_stopped {
                    continue;
                }
                check_cancelled(cancelled)?;
                check_progress_idle(last_progress_at, self.limits.max_progress_idle)?;
                let matching_count = ordered_product_ids
                    .iter()
                    .filter_map(|product_id| unique.get(product_id))
                    .filter(|product| complete_batch_filter(product))
                    .count();
                if finite_target.is_some_and(|target| matching_count as u64 >= target)
                    || request_count >= self.limits.max_requests
                {
                    break 'round_robin;
                }
                if part.page_count >= self.limits.max_pages {
                    part.budget_stopped = true;
                    continue;
                }
                let remaining_candidate_budget =
                    candidate_budget.saturating_sub(fetched_record_count);
                if remaining_candidate_budget == 0 {
                    break 'round_robin;
                }
                let request_page_size = provider_page_size.min(remaining_candidate_budget);
                request_count += 1;
                part.page_count += 1;
                made_request = true;
                let params = plan.page_params(&part.polygon, part.next_page, request_page_size);
                catalogue_provider_diagnostic(
                    plan,
                    part_index,
                    part.next_page,
                    request_count,
                    &params,
                    started,
                );
                let value = self
                    .http
                    .cmr_catalog_json(&params, cancelled)
                    .map_err(normalize_transport_error)?;
                check_cancelled(cancelled)?;
                let page = parse_page(&value, request_page_size)?;
                if part.hits.is_some_and(|expected| expected != page.hits) {
                    part.hits_drifted = true;
                }
                part.hits = Some(page.hits);
                let fingerprint = page_fingerprint(page.items)?;
                if !part.seen_pages.insert(fingerprint) {
                    return Err(pagination_error("repeated_page"));
                }

                let page_len = page.items.len() as u64;
                part.fetched = part
                    .fetched
                    .checked_add(page_len)
                    .ok_or_else(result_limit_error)?;
                fetched_record_count = fetched_record_count
                    .checked_add(page_len)
                    .ok_or_else(result_limit_error)?;
                if part.fetched > page.hits && !part.hits_drifted {
                    return Err(pagination_error("fetched_exceeds_matched"));
                }
                if fetched_record_count > candidate_budget {
                    return Err(result_limit_error());
                }
                if page.items.is_empty() && part.fetched < page.hits {
                    return Err(pagination_error("empty_incomplete_page"));
                }

                for item in page.items {
                    check_cancelled(cancelled)?;
                    let product = match mapper(item)? {
                        CatalogMapperOutcome::Product(product) => product,
                        CatalogMapperOutcome::ProfileFiltered => {
                            profile_filtered_record_count = profile_filtered_record_count
                                .checked_add(1)
                                .ok_or_else(result_limit_error)?;
                            continue;
                        }
                        CatalogMapperOutcome::ItemError(error) => {
                            item_errors.push(error);
                            continue;
                        }
                    };
                    mapped_record_count = mapped_record_count
                        .checked_add(1)
                        .ok_or_else(result_limit_error)?;
                    if !part.product_ids.insert(product.product_id.clone())
                        && matches!(
                            intra_part_duplicate_policy,
                            IntraPartDuplicatePolicy::Reject
                        )
                    {
                        return Err(pagination_error("duplicate_within_polygon"));
                    }
                    match unique.get(&product.product_id) {
                        Some(existing) if existing == &product => {
                            exact_duplicate_record_count = exact_duplicate_record_count
                                .checked_add(1)
                                .ok_or_else(result_limit_error)?;
                        }
                        Some(_) => {
                            return Err(catalogue_error(
                                "CATALOGUE_PRODUCT_IDENTITY_CONFLICT",
                                "ASF 目录返回冲突的 Granule 身份",
                                "granule_conflict",
                            ));
                        }
                        None => {
                            ordered_product_ids.push(product.product_id.clone());
                            unique.insert(product.product_id.clone(), product);
                        }
                    }
                }
                let exact_matched_count = ordered_product_ids
                    .iter()
                    .filter_map(|product_id| unique.get(product_id))
                    .filter(|product| complete_batch_filter(product))
                    .count() as u64;
                progress(CatalogSearchProviderProgress {
                    provider_request_count: request_count as u64,
                    transport_part_count,
                    raw_fetched_count: fetched_record_count,
                    mapped_count: mapped_record_count,
                    unique_admitted_count: finite_target.map_or(exact_matched_count, |target| {
                        exact_matched_count.min(target)
                    }),
                    exact_matched_count,
                    requested_limit: finite_target,
                    remote_total: if plan.is_multi_polygon {
                        None
                    } else {
                        part.hits
                    },
                    elapsed_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
                });
                last_progress_at = Instant::now();
                if part.fetched >= page.hits {
                    part.exhausted = true;
                    continue;
                }
                if page_len < request_page_size {
                    return Err(pagination_error("short_incomplete_page"));
                }
                part.next_page = part
                    .next_page
                    .checked_add(1)
                    .ok_or_else(|| pagination_error("page_number_overflow"))?;
            }
            if !made_request {
                break;
            }
        }

        check_cancelled(cancelled)?;
        let remote_exhausted = parts.iter().all(|part| part.exhausted);
        let part_hit_diagnostics = parts
            .iter()
            .map(|part| part.hits.unwrap_or(0))
            .collect::<Vec<_>>();
        let unique_provider_product_count = unique.len() as u64;
        if unique_provider_product_count > self.limits.max_records {
            return Err(result_limit_error());
        }
        let exact_matched_count = ordered_product_ids
            .iter()
            .filter_map(|product_id| unique.get(product_id))
            .filter(|product| complete_batch_filter(product))
            .count() as u64;
        let search_complete = finite_target
            .map(|target| exact_matched_count >= target || remote_exhausted)
            .unwrap_or(remote_exhausted);
        if !search_complete {
            if parts.iter().any(|part| part.budget_stopped) {
                return Err(page_budget_error());
            }
            if request_count >= self.limits.max_requests {
                return Err(request_budget_error());
            }
            if fetched_record_count >= candidate_budget && finite_target.is_none() {
                return Err(result_limit_error());
            }
            return Err(working_set_incomplete_error());
        }
        let admitted_limit = finite_target.map_or(usize::MAX, |target| target as usize);
        let products = ordered_product_ids
            .iter()
            .filter_map(|product_id| unique.get(product_id))
            .filter(|product| complete_batch_filter(product))
            .take(admitted_limit)
            .cloned()
            .collect::<Vec<_>>();
        let response = CatalogSearchResponse {
            collection: collection_descriptor(profile),
            stats: CatalogSearchStats {
                requested_limit: finite_target,
                matched_count: if plan.is_multi_polygon {
                    None
                } else {
                    part_hit_diagnostics.first().copied()
                },
                request_count: request_count as u64,
                transport_part_count,
                fetched_record_count,
                mapped_record_count,
                profile_filtered_record_count,
                exact_duplicate_record_count,
                unique_provider_product_count,
                exact_matched_count,
                returned_count: products.len() as u64,
                remote_exhausted,
                working_set_satisfied: true,
                elapsed_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
                truncated: false,
            },
            products,
            next_cursor: None,
            item_errors,
        };
        response.validate().map_err(|reason| {
            ContractError::new(
                "CATALOGUE_RESPONSE_INVALID",
                "ASF 目录完整批次合同无效",
                Map::from_iter([("reason".to_owned(), Value::String(reason))]),
            )
            .expect("bounded catalogue response validation error is safe")
        })?;
        catalogue_search_complete_diagnostic(plan, &response.stats, &part_hit_diagnostics);
        check_cancelled(cancelled)?;
        Ok(response)
    }
}

fn catalogue_provider_diagnostic(
    plan: &AsfCmrQuery,
    part_index: usize,
    page_number: u64,
    request_count: usize,
    params: &[(String, String)],
    started: Instant,
) {
    if std::env::var_os("INSAR_DIAGNOSTIC_CATALOGUE").is_none() {
        return;
    }
    let query_bytes = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(
            params
                .iter()
                .map(|(key, value)| (key.as_str(), value.as_str())),
        )
        .finish()
        .len();
    let transport_vertices = plan
        .transport_vertex_counts
        .get(part_index)
        .copied()
        .unwrap_or(0);
    eprintln!(
        "INSAR_CATALOGUE_PROVIDER_DIAGNOSTIC mode={} canonical_polygons={} canonical_rings={} canonical_vertices={} transport_parts={} part={} transport_vertices={} page={} request={} query_bytes={} elapsed_ms={}",
        plan.spatial_transport_mode,
        plan.canonical_polygon_count,
        plan.canonical_ring_count,
        plan.canonical_vertex_count,
        plan.transport_part_count,
        part_index + 1,
        transport_vertices,
        page_number,
        request_count,
        query_bytes,
        started.elapsed().as_millis(),
    );
}

fn catalogue_search_complete_diagnostic(
    plan: &AsfCmrQuery,
    stats: &CatalogSearchStats,
    part_remote_hits: &[u64],
) {
    if std::env::var_os("INSAR_DIAGNOSTIC_CATALOGUE").is_none() {
        return;
    }
    eprintln!(
        "INSAR_CATALOGUE_COMPLETE_DIAGNOSTIC {}",
        serde_json::json!({
            "spatial_transport_mode": plan.spatial_transport_mode,
            "canonical_polygon_count": plan.canonical_polygon_count,
            "canonical_ring_count": plan.canonical_ring_count,
            "canonical_vertex_count": plan.canonical_vertex_count,
            "transport_part_count": stats.transport_part_count,
            "remote_hits_by_part": part_remote_hits,
            "requested_limit": stats.requested_limit,
            "request_count": stats.request_count,
            "fetched_record_count": stats.fetched_record_count,
            "mapped_record_count": stats.mapped_record_count,
            "profile_filtered_record_count": stats.profile_filtered_record_count,
            "exact_matched_count": stats.exact_matched_count,
            "returned_count": stats.returned_count,
            "remote_exhausted": stats.remote_exhausted,
            "working_set_satisfied": stats.working_set_satisfied,
            "elapsed_ms": stats.elapsed_ms,
        })
    );
}

fn nisar_product_matches_complete_batch_filters(
    product: &ProviderProduct,
    filters: &[(String, String)],
) -> bool {
    if product.provider_id != NISAR_PROVIDER_ID {
        return false;
    }
    filters.iter().all(|(key, expected)| match key.as_str() {
        "look_direction" => product
            .properties
            .get("look_direction")
            .and_then(Value::as_str)
            .is_some_and(|actual| actual.eq_ignore_ascii_case(expected)),
        _ => false,
    })
}

struct CmrPage<'a> {
    hits: u64,
    items: &'a [Value],
}

fn parse_page(value: &Value, page_size: u64) -> ProviderResult<CmrPage<'_>> {
    let root = value
        .as_object()
        .ok_or_else(|| pagination_error("page_shape"))?;
    let hits = root
        .get("hits")
        .and_then(Value::as_u64)
        .ok_or_else(|| pagination_error("matched_count"))?;
    let items = root
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| pagination_error("items"))?;
    if items.len() as u64 > page_size {
        return Err(pagination_error("page_size"));
    }
    Ok(CmrPage { hits, items })
}

fn page_fingerprint(items: &[Value]) -> ProviderResult<[u8; 32]> {
    let mut identities = items
        .iter()
        .map(|item| {
            item.pointer("/meta/concept-id")
                .or_else(|| item.pointer("/granule/concept_id"))
                .and_then(Value::as_str)
                .filter(|value| {
                    !value.is_empty() && value.len() <= 512 && !value.chars().any(char::is_control)
                })
                .map(str::to_owned)
                .ok_or_else(|| pagination_error("granule_identity"))
        })
        .collect::<ProviderResult<Vec<_>>>()?;
    identities.sort();
    let bytes =
        serde_json::to_vec(&identities).map_err(|_| pagination_error("page_serialization"))?;
    Ok(Sha256::digest(bytes).into())
}

fn collection_descriptor(profile: &CollectionProfile) -> CatalogCollectionDescriptor {
    CatalogCollectionDescriptor {
        logical_dataset: profile.logical_dataset.to_owned(),
        provider_id: profile.provider_id.to_owned(),
        native_collection_id: profile.native_collection_id.to_owned(),
        collection_profile_id: profile.profile_id.to_owned(),
        identity_rule_id: profile.identity_rule_id.to_owned(),
        filename_rule_id: profile.filename_rule_id.to_owned(),
        delivery_model: profile.delivery_model.clone(),
        production_enabled: profile.production_enabled,
    }
}

fn alos2_product_matches_query(product: &ProviderProduct, query: &CatalogSearchQuery) -> bool {
    if product.provider_id != ALOS2_PROVIDER_ID {
        return false;
    }
    for (filter, property) in [
        ("beam_mode", "beam_mode"),
        ("flight_direction", "flight_direction"),
        ("look_direction", "look_direction"),
    ] {
        if let Some(expected) = query.filters.get(filter).and_then(Value::as_str) {
            if !product
                .properties
                .get(property)
                .and_then(Value::as_str)
                .is_some_and(|actual| actual.eq_ignore_ascii_case(expected))
            {
                return false;
            }
        }
    }
    for key in ["path", "frame"] {
        if let Some(expected) = query.filters.get(key).and_then(Value::as_u64) {
            if product.properties.get(key).and_then(Value::as_u64) != Some(expected) {
                return false;
            }
        }
    }
    if let Some(expected) = query.filters.get("polarizations").and_then(Value::as_array) {
        let mut expected = expected
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_ascii_uppercase)
            .collect::<Vec<_>>();
        let mut actual = product
            .properties
            .get("polarizations")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_ascii_uppercase)
            .collect::<Vec<_>>();
        expected.sort();
        actual.sort();
        if expected != actual {
            return false;
        }
    }
    if let Some(window) = query.filters.get("datetime").and_then(Value::as_object) {
        let acquired = product
            .acquisition_datetime
            .as_deref()
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&Utc));
        let start = window
            .get("start")
            .and_then(Value::as_str)
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&Utc));
        let end = window
            .get("end")
            .and_then(Value::as_str)
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&Utc));
        if !acquired
            .zip(start.zip(end))
            .is_some_and(|(acquired, (start, end))| start <= acquired && acquired < end)
        {
            return false;
        }
    }
    true
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn nisar_primary_locator_present(envelope: &CmrGranuleEnvelope) -> ProviderResult<bool> {
    let records = envelope
        .granule
        .get("assets")
        .and_then(Value::as_object)
        .and_then(|assets| assets.get("records"))
        .and_then(Value::as_array)
        .ok_or_else(|| canary_error("asset_records"))?;
    let primary = records
        .iter()
        .filter(|record| record.get("normalized_role").and_then(Value::as_str) == Some("primary"))
        .collect::<Vec<_>>();
    let [primary] = primary.as_slice() else {
        return Err(canary_error("primary_asset_cardinality"));
    };
    Ok(primary
        .get("routes")
        .and_then(Value::as_array)
        .is_some_and(|routes| !routes.is_empty()))
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn validate_nisar_exact_identity_product(
    product: ProviderProduct,
    query_parameter_digest: String,
    primary_locator_present: bool,
    http_status: u16,
    response_body_bytes: usize,
) -> ProviderResult<NisarExactIdentityCanaryResult> {
    let expected_product_key = provider_product_key(
        NISAR_PROVIDER_ID,
        NISAR_NATIVE_COLLECTION_ID,
        &product.product_id,
    )
    .map_err(|_| canary_error("provider_product_key"))?;
    let provider_product_key_value = canary_property_text(&product, "provider_product_key")?;
    if product.provider_id != NISAR_PROVIDER_ID
        || product.display_name != NISAR_CATALOGUE_CANARY_GRANULE_ID
        || provider_product_key_value != expected_product_key
        || canary_property_text(&product, "provider_product_id")? != product.product_id
        || canary_property_text(&product, "producer_granule_id")?
            != NISAR_CATALOGUE_CANARY_GRANULE_ID
        || canary_property_text(&product, "collection_concept_id")? != NISAR_NATIVE_COLLECTION_ID
        || canary_property_text(&product, "collection_profile_id")? != NISAR_PROFILE_ID
        || canary_property_text(&product, "processing_level")? != NISAR_REQUIRED_PRODUCT_TYPE
        || canary_property_text(&product, "production_configuration")?
            != NISAR_PRODUCTION_CONFIGURATION
        || canary_property_text(&product, "crid")? != NISAR_P05023_CRID
        || canary_property_text(&product, "product_specification_version")?
            != NISAR_P05023_PRODUCT_SPECIFICATION_VERSION
        || canary_property_text(&product, "quality_status")? != NISAR_QUALITY_STATUS
        || !primary_locator_present
    {
        return Err(canary_error("mapped_identity"));
    }

    let primary_assets = product
        .assets
        .iter()
        .filter(|asset| asset.role == AssetRole::Primary)
        .collect::<Vec<_>>();
    let [primary] = primary_assets.as_slice() else {
        return Err(canary_error("primary_asset_cardinality"));
    };
    let primary_key = primary
        .asset_key
        .as_deref()
        .ok_or_else(|| canary_error("primary_asset_key"))?;
    if product
        .properties
        .get("primary_asset_key")
        .and_then(Value::as_str)
        != Some(primary_key)
        || primary.file_name.as_deref()
            != Some(format!("{}.h5", NISAR_CATALOGUE_CANARY_GRANULE_ID).as_str())
        || primary.size_bytes != Some(NISAR_CATALOGUE_CANARY_SOURCE_SIZE_BYTES as i64)
        || primary
            .stable_asset_key
            .as_deref()
            .is_none_or(str::is_empty)
    {
        return Err(canary_error("primary_asset_identity"));
    }
    let stable_asset_key = primary
        .stable_asset_key
        .clone()
        .ok_or_else(|| canary_error("stable_asset_key"))?;
    let integrity = product
        .properties
        .get("asset_integrity")
        .and_then(Value::as_object)
        .and_then(|assets| assets.get(primary_key))
        .and_then(Value::as_object)
        .ok_or_else(|| canary_error("primary_asset_integrity"))?;
    let checksum_algorithm = integrity
        .get("algorithm")
        .and_then(Value::as_str)
        .ok_or_else(|| canary_error("primary_asset_integrity"))?;
    let checksum_value = integrity
        .get("value")
        .and_then(Value::as_str)
        .ok_or_else(|| canary_error("primary_asset_integrity"))?;
    // The real CMR archive records the primary HDF5 integrity as MD5 (the
    // same value the live-acceptance harness freezes). The canary binds the
    // exact provider-provided value; algorithm and length are checked first so
    // a provider algorithm change fails closed instead of being accepted.
    if integrity.get("file_name").and_then(Value::as_str)
        != Some(format!("{}.h5", NISAR_CATALOGUE_CANARY_GRANULE_ID).as_str())
        || integrity.get("size_bytes").and_then(Value::as_u64)
            != Some(NISAR_CATALOGUE_CANARY_SOURCE_SIZE_BYTES)
        || checksum_algorithm != "MD5"
        || checksum_value.len() != 32
        || checksum_value != NISAR_CATALOGUE_CANARY_SOURCE_MD5
        || !checksum_value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(canary_error("primary_asset_integrity"));
    }

    Ok(NisarExactIdentityCanaryResult {
        request_count: 1,
        http_status,
        response_body_bytes,
        query_parameter_digest,
        hits: 1,
        returned_item_count: 1,
        provider_id: product.provider_id.clone(),
        collection_id: canary_property_text(&product, "collection_concept_id")?.to_owned(),
        collection_profile_id: canary_property_text(&product, "collection_profile_id")?.to_owned(),
        provider_product_id: product.product_id.clone(),
        provider_product_key: provider_product_key_value.to_owned(),
        producer_granule_id: canary_property_text(&product, "producer_granule_id")?.to_owned(),
        display_name: product.display_name.clone(),
        product_type: canary_property_text(&product, "processing_level")?.to_owned(),
        production_configuration: canary_property_text(&product, "production_configuration")?
            .to_owned(),
        crid: canary_property_text(&product, "crid")?.to_owned(),
        product_specification_version: canary_property_text(
            &product,
            "product_specification_version",
        )?
        .to_owned(),
        quality_status: canary_property_text(&product, "quality_status")?.to_owned(),
        primary_filename: format!("{}.h5", NISAR_CATALOGUE_CANARY_GRANULE_ID),
        primary_role: "Primary".to_owned(),
        stable_asset_key,
        source_size_bytes: NISAR_CATALOGUE_CANARY_SOURCE_SIZE_BYTES,
        source_checksum_algorithm: checksum_algorithm.to_owned(),
        source_checksum_value: checksum_value.to_owned(),
        footprint_valid: true,
        aoi_intersects: true,
        primary_locator_present,
    })
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn canary_property_text<'a>(
    product: &'a ProviderProduct,
    key: &'static str,
) -> ProviderResult<&'a str> {
    product
        .properties
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| canary_error(key))
}

fn normalize_transport_error(error: ContractError) -> ContractError {
    if error.code == "DL001" {
        return cancelled_error();
    }
    if matches!(
        error.code.as_str(),
        "ASF_RATE_LIMITED"
            | "ASF_SERVER_ERROR"
            | "ASF_TIMEOUT"
            | "ASF_TRANSPORT_FAILED"
            | "ASF_RESPONSE_TOO_LARGE"
    ) {
        return error;
    }
    if !matches!(error.code.as_str(), "ASF002") {
        return error;
    }
    let category = error
        .debug_context
        .get("category")
        .and_then(Value::as_str)
        .unwrap_or("transport");
    let status = error.debug_context.get("status").and_then(Value::as_u64);
    let polygon_failure = error
        .debug_context
        .get("field")
        .and_then(Value::as_str)
        .is_some_and(|field| field.eq_ignore_ascii_case("polygon"));
    let (code, message) = if polygon_failure {
        (
            "CATALOGUE_INVALID_POLYGON",
            "CMR 拒绝了当前空间检索分片，请重新加载 AOI 后再试",
        )
    } else if status.is_some_and(|status| (400..500).contains(&status)) {
        (
            "CATALOGUE_HTTP_CLIENT_FAILURE",
            "CMR 拒绝了当前目录请求，请检查筛选条件或稍后重试",
        )
    } else if status.is_some_and(|status| status >= 500) {
        (
            "CATALOGUE_HTTP_SERVER_FAILURE",
            "CMR 服务暂时不可用，请稍后重试",
        )
    } else {
        match category {
            "response_too_large" => (
                "CATALOGUE_RESPONSE_TOO_LARGE",
                "CMR 返回内容超过安全上限，请缩小检索范围",
            ),
            "invalid_json" | "invalid_json_shape" => (
                "CATALOGUE_RESPONSE_INVALID_JSON",
                "CMR 返回了无法解析的目录响应",
            ),
            "timeout" => (
                "CATALOGUE_PROVIDER_TIMEOUT",
                "CMR 请求超时，请稍后重试或缩小检索范围",
            ),
            _ => (
                "CATALOGUE_TRANSPORT_FAILURE",
                "无法完成 CMR 目录请求，请检查网络、DNS 或代理设置",
            ),
        }
    };
    ContractError::new(code, message, error.debug_context)
        .expect("sanitized ASF catalogue transport context remains safe")
}

fn route_policy_error(error: DatasetRoutePolicyError) -> ContractError {
    match error {
        DatasetRoutePolicyError::ProductionDisabled(_) => catalogue_error(
            "CATALOGUE_ROUTE_DISABLED",
            "ASF 目录路由尚未启用",
            "production_disabled",
        ),
        _ => catalogue_error(
            "ASF_COLLECTION_IDENTITY_MISMATCH",
            "ASF 目录 Collection 身份不匹配",
            "route_identity",
        ),
    }
}

fn check_cancelled(cancelled: &(dyn Fn() -> bool + Sync)) -> ProviderResult<()> {
    if cancelled() {
        Err(cancelled_error())
    } else {
        Ok(())
    }
}

fn check_progress_idle(last_progress_at: Instant, maximum: Duration) -> ProviderResult<()> {
    if last_progress_at.elapsed() > maximum {
        Err(catalogue_error(
            "CATALOGUE_TIME_BUDGET_EXCEEDED",
            "ASF 目录检索长时间没有取得新进展",
            "progress_idle",
        ))
    } else {
        Ok(())
    }
}

fn cancelled_error() -> ContractError {
    catalogue_error(
        "CATALOGUE_REQUEST_CANCELLED",
        "ASF 目录检索已取消",
        "cancelled",
    )
}

fn pagination_error(reason: &'static str) -> ContractError {
    catalogue_error(
        "CATALOGUE_PAGINATION_INVALID",
        "ASF 目录分页未能形成完整批次",
        reason,
    )
}

fn page_budget_error() -> ContractError {
    catalogue_error(
        "CATALOGUE_PAGE_BUDGET_EXCEEDED",
        "ASF 目录检索超过安全分页上限",
        "page_budget",
    )
}

fn request_budget_error() -> ContractError {
    catalogue_error(
        "CATALOGUE_REQUEST_BUDGET_EXCEEDED",
        "ASF 目录检索超过安全请求上限",
        "request_budget",
    )
}

fn working_set_incomplete_error() -> ContractError {
    catalogue_error(
        "CATALOGUE_WORKING_SET_INCOMPLETE",
        "ASF 目录候选预算内无法完成当前本地筛选，请缩小范围或减少筛选条件",
        "candidate_budget",
    )
}

fn result_limit_error() -> ContractError {
    catalogue_error(
        "CATALOGUE_RESULT_LIMIT_EXCEEDED",
        "ASF 目录候选记录超过安全扫描上限",
        "candidate_record_limit",
    )
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn canary_error(reason: &'static str) -> ContractError {
    catalogue_error(
        "NISAR_CATALOGUE_CANARY_REJECTED",
        "NISAR 精确身份 catalogue canary 拒绝了不满足门禁的结果",
        reason,
    )
}

fn catalogue_error(
    code: &'static str,
    message: &'static str,
    reason: &'static str,
) -> ContractError {
    ContractError::new(
        code,
        message,
        Map::from_iter([("reason".to_owned(), Value::String(reason.to_owned()))]),
    )
    .expect("bounded ASF catalogue error is safe")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_transport_failures_survive_catalogue_context_normalization() {
        for (code, status) in [
            ("ASF_RATE_LIMITED", Some(429_u16)),
            ("ASF_SERVER_ERROR", Some(503_u16)),
            ("ASF_TIMEOUT", None),
            ("ASF_TRANSPORT_FAILED", None),
            ("ASF_RESPONSE_TOO_LARGE", None),
        ] {
            let mut context =
                Map::from_iter([("category".to_owned(), Value::String("transport".to_owned()))]);
            if let Some(status) = status {
                context.insert("status".to_owned(), Value::from(status));
            }
            let error = ContractError::new(code, "typed transport failure", context)
                .expect("test error is bounded");
            assert_eq!(normalize_transport_error(error).code, code);
        }
    }

    #[test]
    fn generic_polygon_rejection_still_receives_catalogue_context() {
        let error = ContractError::new(
            "ASF002",
            "generic rejection",
            Map::from_iter([
                (
                    "category".to_owned(),
                    Value::String("http_status".to_owned()),
                ),
                ("status".to_owned(), Value::from(400_u16)),
                ("field".to_owned(), Value::String("polygon".to_owned())),
            ]),
        )
        .expect("test error is bounded");

        assert_eq!(
            normalize_transport_error(error).code,
            "CATALOGUE_INVALID_POLYGON"
        );
    }
}
