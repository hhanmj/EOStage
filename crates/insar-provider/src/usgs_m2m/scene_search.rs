use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, NaiveDate, NaiveDateTime, SecondsFormat, Utc};
use insar_core::{
    CatalogCollectionDescriptor, CatalogDeliveryModel, CatalogSearchQuery, CatalogSearchResponse,
    CatalogSearchStats, DataSourceKind, ProductKind, ProviderNetworkPolicy, ProviderProduct,
};
use serde_json::{json, Map, Number, Value};
use sha2::{Digest, Sha256};

use super::browse::{BrowseLookup, UsgsM2mBrowseCache, UsgsM2mBrowseClient, UsgsM2mBrowseImage};
use super::endpoint::{zeroize_json, SensitiveJson};
use super::error::{UsgsM2mEndpointClass, UsgsM2mError, UsgsM2mErrorCode};
#[cfg(feature = "usgs-m2m-live-acceptance")]
use super::live_replay::redact_scene_search_data;
use super::router::{UsgsM2mSceneSearchAcceptance, USGS_M2M_LANDSAT_C2_L2_SCENE_SEARCH_ACCEPTANCE};
use super::session::UsgsM2mSession;
use crate::catalog::{
    CollectionProfileRegistry, DatasetRoutePolicyRegistry, RouteUse, LANDSAT_C2_L2_LOGICAL_DATASET,
    LANDSAT_C2_L2_NATIVE_COLLECTION_ID, LANDSAT_C2_L2_PROVIDER_ID, LANDSAT_C2_L2_SCENE_PROFILE_ID,
    USGS_LANDSAT_C2_L2_BUNDLE_FILENAME_RULE_ID, USGS_LANDSAT_C2_L2_IDENTITY_RULE_ID,
};

pub const USGS_M2M_SCENE_SEARCH_ROUTE_ID: &str = "usgs.eros.m2m.landsat.c2.l2.scene-search.v1";
pub const USGS_M2M_SCENE_SEARCH_CAPABILITY: &str = "scene_search";
pub const USGS_M2M_SCENE_SEARCH_CURSOR_VERSION: &str = "usgs-m2m-scene.v1";

const MAX_PAGE_SIZE: u64 = 100;
const MAX_PAGES: usize = 400;
const MAX_SCENES: u64 = 40_000;

fn transport_page_size(result_limit: Option<u64>) -> u64 {
    result_limit.unwrap_or(MAX_PAGE_SIZE).min(MAX_PAGE_SIZE)
}
const MAX_CURSOR_BYTES: usize = 256;
const MAX_DATASET_FILTERS: usize = 128;
const MAX_METADATA_ITEMS: usize = 512;
const MAX_TEXT_BYTES: usize = 256;
const MAX_POLYGONS: usize = 32;
const MAX_RINGS: usize = 128;
const MAX_POSITIONS: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsgsM2mSceneSearchState {
    OfflineImplemented,
    LiveAccepted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SceneRouteAdmission {
    Standard,
    #[cfg(feature = "usgs-m2m-live-acceptance")]
    LiveAcceptanceCandidate,
}

impl UsgsM2mSceneSearchState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OfflineImplemented => "implemented_not_live_accepted",
            Self::LiveAccepted => "live_accepted",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LandsatC2L2SearchFacts {
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    cloud_cover_lte: f64,
    spacecraft: BTreeSet<&'static str>,
    processing_levels: BTreeSet<&'static str>,
    fingerprint: String,
    starting_number: u64,
    result_limit: Option<u64>,
}

impl LandsatC2L2SearchFacts {
    pub fn start(&self) -> DateTime<Utc> {
        self.start
    }

    pub fn end(&self) -> DateTime<Utc> {
        self.end
    }

    pub const fn cloud_cover_lte(&self) -> f64 {
        self.cloud_cover_lte
    }

    pub fn spacecraft(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.spacecraft.iter().copied()
    }

    pub fn processing_levels(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.processing_levels.iter().copied()
    }

    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    pub const fn starting_number(&self) -> u64 {
        self.starting_number
    }
}

pub struct UsgsM2mSceneSearchClient {
    session: Arc<UsgsM2mSession>,
    browse_cache: Mutex<UsgsM2mBrowseCache>,
    #[cfg(feature = "usgs-m2m-live-acceptance")]
    live_acceptance_replays: Mutex<Vec<Value>>,
}

impl UsgsM2mSceneSearchClient {
    pub fn new(session: Arc<UsgsM2mSession>) -> Self {
        Self {
            session,
            browse_cache: Mutex::new(UsgsM2mBrowseCache::default()),
            #[cfg(feature = "usgs-m2m-live-acceptance")]
            live_acceptance_replays: Mutex::new(Vec::new()),
        }
    }

    pub fn network_policy(&self) -> ProviderNetworkPolicy {
        self.session.network_policy()
    }

    /// Resolve an explicitly selected, fully versioned product for official
    /// delivery. This is not a catalogue fallback and creates no remote scene list.
    pub fn exact_product(
        &self,
        display_id: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<ProviderProduct, UsgsM2mError> {
        let parts = display_id.split('_').collect::<Vec<_>>();
        if parts.len() != 7 || !matches!(parts[0], "LT04" | "LT05" | "LE07" | "LC08" | "LC09") {
            return Err(scene_error(UsgsM2mErrorCode::ConfigInvalid));
        }
        let native_id = format!(
            "{}_{}_{}_{}_{}_{}",
            parts[0], parts[1], parts[2], parts[3], parts[5], parts[6]
        );
        crate::planetary_computer::validate_planetary_computer_landsat_source_id(
            &native_id, display_id,
        )
        .map_err(|_| scene_error(UsgsM2mErrorCode::ConfigInvalid))?;
        if cancelled() {
            return Err(scene_error(UsgsM2mErrorCode::Cancelled));
        }
        let dataset_name = match parts[0] {
            "LT04" | "LT05" => "landsat_tm_c2_l2",
            "LE07" => "landsat_etm_c2_l2",
            _ => LANDSAT_C2_L2_NATIVE_COLLECTION_ID,
        };
        let filters = self
            .session
            .request_dataset_filters(dataset_name, cancelled)?;
        let filter = exact_product_filter(filters.value(), display_id)?;
        if cancelled() {
            return Err(scene_error(UsgsM2mErrorCode::Cancelled));
        }
        let payload = json!({
            "datasetName": dataset_name,
            "metadataType": "full", "maxResults": 2, "startingNumber": 1,
            "sceneFilter": {"metadataFilter": filter}
        });
        let mut data = self.session.request_scene_search(&payload, cancelled)?;
        if cancelled() {
            return Err(scene_error(UsgsM2mErrorCode::Cancelled));
        }
        let mut page = parse_page_envelope(data.take(), 2, 1)?;
        if page.total_hits == 0 {
            return Err(scene_error(UsgsM2mErrorCode::DownloadUnavailable));
        }
        if page.total_hits != 1 || page.records.len() != 1 || page.next_starting_number.is_some() {
            return Err(scene_error(UsgsM2mErrorCode::SceneConflict));
        }
        let product = map_scene(page.records.remove(0))?;
        if product.display_name != display_id {
            return Err(scene_error(UsgsM2mErrorCode::DownloadIdentityConflict));
        }
        Ok(product)
    }

    #[cfg(feature = "usgs-m2m-live-acceptance")]
    #[doc(hidden)]
    pub fn take_live_acceptance_redacted_replays(&self) -> Vec<Value> {
        self.live_acceptance_replays
            .lock()
            .map(|mut replays| std::mem::take(&mut *replays))
            .unwrap_or_default()
    }

    #[cfg(test)]
    fn search_page_for_use(
        &self,
        query: &CatalogSearchQuery,
        use_case: RouteUse,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<CatalogSearchResponse, UsgsM2mError> {
        let query = &insar_core::clamp_catalogue_search_query(query);
        let facts = decode_search_facts(query, use_case)?;
        let satellite_filter = self.satellite_filter(&facts, cancelled)?;
        self.fetch_page(query, &facts, satellite_filter.as_ref(), cancelled)?
            .into_response(&facts, use_case)
    }

    fn search_all_for_use(
        &self,
        query: &CatalogSearchQuery,
        use_case: RouteUse,
        admission: SceneRouteAdmission,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<CatalogSearchResponse, UsgsM2mError> {
        let query = &insar_core::clamp_catalogue_search_query(query);
        self.browse_cache
            .lock()
            .map_err(|_| scene_error(UsgsM2mErrorCode::ConfigInvalid))?
            .clear();
        #[cfg(feature = "usgs-m2m-live-acceptance")]
        if let Ok(mut replays) = self.live_acceptance_replays.lock() {
            replays.clear();
        }
        if query.cursor.is_some() {
            return Err(scene_error(UsgsM2mErrorCode::ConfigInvalid));
        }
        let initial_facts = decode_search_facts_for_admission(query, use_case, admission)?;
        let satellite_filter = self.satellite_filter(&initial_facts, cancelled)?;
        let mut current_query = query.clone();
        let mut pages = 0_usize;
        let mut matched_count = None;
        let mut fetched_record_count = 0_u64;
        let mut mapped_record_count = 0_u64;
        let mut exact_duplicate_record_count = 0_u64;
        let mut remote_exhausted = false;
        let mut unique = BTreeMap::<String, ProviderProduct>::new();
        let collection = collection_descriptor_for_admission(use_case, admission)?;

        loop {
            if cancelled() {
                return Err(scene_error(UsgsM2mErrorCode::Cancelled));
            }
            pages += 1;
            if pages > MAX_PAGES {
                return Err(scene_error(UsgsM2mErrorCode::SceneLimitExceeded));
            }
            let facts = decode_search_facts_for_admission(&current_query, use_case, admission)?;
            let page =
                self.fetch_page(&current_query, &facts, satellite_filter.as_ref(), cancelled)?;
            let page_total = page.matched_count;
            if page_total > MAX_SCENES {
                return Err(scene_error(UsgsM2mErrorCode::SceneLimitExceeded));
            }
            match matched_count {
                None => matched_count = Some(page_total),
                Some(expected) if expected == page_total => {}
                Some(_) => return Err(scene_error(UsgsM2mErrorCode::PaginationInvalid)),
            }
            fetched_record_count = fetched_record_count
                .checked_add(page.fetched_record_count)
                .ok_or_else(|| scene_error(UsgsM2mErrorCode::SceneLimitExceeded))?;
            mapped_record_count = mapped_record_count
                .checked_add(page.mapped_record_count)
                .ok_or_else(|| scene_error(UsgsM2mErrorCode::SceneLimitExceeded))?;
            exact_duplicate_record_count = exact_duplicate_record_count
                .checked_add(page.exact_duplicate_record_count)
                .ok_or_else(|| scene_error(UsgsM2mErrorCode::SceneLimitExceeded))?;
            for product in page.products.into_values() {
                match unique.get(&product.product_id) {
                    Some(existing) if existing == &product => {
                        exact_duplicate_record_count += 1;
                    }
                    Some(_) => return Err(scene_error(UsgsM2mErrorCode::SceneConflict)),
                    None => {
                        unique.insert(product.product_id.clone(), product);
                    }
                }
            }

            let exact_matched_count = unique.values().try_fold(0_u64, |count, product| {
                scene_matches_query(product, &initial_facts)
                    .map(|matches| count + u64::from(matches))
            })?;
            match page.next_cursor {
                Some(cursor) => {
                    if initial_facts
                        .result_limit
                        .is_some_and(|target| exact_matched_count >= target)
                    {
                        break;
                    }
                    if fetched_record_count >= page_total {
                        return Err(scene_error(UsgsM2mErrorCode::PaginationInvalid));
                    }
                    current_query.cursor = Some(cursor);
                }
                None => {
                    remote_exhausted = true;
                    break;
                }
            }
        }

        if (remote_exhausted && matched_count != Some(fetched_record_count))
            || mapped_record_count != fetched_record_count
        {
            return Err(scene_error(UsgsM2mErrorCode::PaginationInvalid));
        }
        let unique_provider_product_count = unique.len() as u64;
        let mut products = unique
            .into_values()
            .filter_map(
                |product| match scene_matches_query(&product, &initial_facts) {
                    Ok(true) => Some(Ok(product)),
                    Ok(false) => None,
                    Err(error) => Some(Err(error)),
                },
            )
            .collect::<Result<Vec<_>, _>>()?;
        let exact_matched_count = products.len() as u64;
        let working_set_satisfied = initial_facts
            .result_limit
            .map(|target| exact_matched_count >= target || remote_exhausted)
            .unwrap_or(remote_exhausted);
        if !working_set_satisfied {
            return Err(scene_error(UsgsM2mErrorCode::SceneLimitExceeded));
        }
        if let Some(target) = initial_facts.result_limit {
            products.truncate(target as usize);
        }
        let returned_count = products.len() as u64;
        let response = CatalogSearchResponse {
            collection,
            products,
            stats: CatalogSearchStats {
                requested_limit: initial_facts.result_limit,
                matched_count,
                request_count: pages as u64,
                transport_part_count: 1,
                fetched_record_count,
                mapped_record_count,
                profile_filtered_record_count: 0,
                exact_duplicate_record_count,
                unique_provider_product_count,
                exact_matched_count,
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
            .map_err(|_| scene_error(UsgsM2mErrorCode::ResponseInvalid))?;
        Ok(response)
    }

    fn satellite_filter(
        &self,
        facts: &LandsatC2L2SearchFacts,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<Option<Value>, UsgsM2mError> {
        if facts.spacecraft.len() != 1 {
            return Ok(None);
        }
        if cancelled() {
            return Err(scene_error(UsgsM2mErrorCode::Cancelled));
        }
        let requested = match facts.spacecraft.iter().next().copied() {
            Some("landsat_4") => "4",
            Some("landsat_5") => "5",
            Some("landsat_7") => "7",
            Some("landsat_8") => "8",
            Some("landsat_9") => "9",
            _ => return Err(scene_error(UsgsM2mErrorCode::ConfigInvalid)),
        };
        let dataset_name = match facts.spacecraft.iter().next().copied() {
            Some("landsat_4" | "landsat_5") => "landsat_tm_c2_l2",
            Some("landsat_7") => "landsat_etm_c2_l2",
            _ => LANDSAT_C2_L2_NATIVE_COLLECTION_ID,
        };
        let data = self
            .session
            .request_dataset_filters(dataset_name, cancelled)?;
        parse_satellite_filter(data, requested).map(Some)
    }

    fn fetch_page(
        &self,
        query: &CatalogSearchQuery,
        facts: &LandsatC2L2SearchFacts,
        satellite_filter: Option<&Value>,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<ParsedScenePage, UsgsM2mError> {
        if cancelled() {
            return Err(scene_error(UsgsM2mErrorCode::Cancelled));
        }
        let payload = build_scene_search_payload(query, facts, satellite_filter)?;
        let data = self.session.request_scene_search(&payload, cancelled)?;
        #[cfg(feature = "usgs-m2m-live-acceptance")]
        if let Ok(mut replays) = self.live_acceptance_replays.lock() {
            if replays.len() < MAX_PAGES {
                replays.push(redact_scene_search_data(data.value()));
            }
        }
        if cancelled() {
            return Err(scene_error(UsgsM2mErrorCode::Cancelled));
        }
        let mut browse_page = UsgsM2mBrowseCache::default();
        browse_page.capture_page(data.value());
        let page = parse_scene_page(data, query, facts)?;
        self.browse_cache
            .lock()
            .map_err(|_| scene_error(UsgsM2mErrorCode::ConfigInvalid))?
            .merge(browse_page);
        Ok(page)
    }

    fn browse_for_query(
        &self,
        query: &CatalogSearchQuery,
        use_case: RouteUse,
        admission: SceneRouteAdmission,
        entity_id: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<Option<UsgsM2mBrowseImage>, UsgsM2mError> {
        let query = &insar_core::clamp_catalogue_search_query(query);
        if !valid_identifier(entity_id, MAX_TEXT_BYTES) {
            return Err(scene_error(UsgsM2mErrorCode::ConfigInvalid));
        }
        let mut lookup = self
            .browse_cache
            .lock()
            .map_err(|_| scene_error(UsgsM2mErrorCode::ConfigInvalid))?
            .lookup(entity_id);
        if matches!(lookup, BrowseLookup::Missing) {
            let response = self.search_all_for_use(query, use_case, admission, cancelled)?;
            if !response
                .products
                .iter()
                .any(|product| product.product_id == entity_id)
            {
                return Err(
                    scene_error(UsgsM2mErrorCode::SceneConflict).with_detail("browse_scene_absent")
                );
            }
            lookup = self
                .browse_cache
                .lock()
                .map_err(|_| scene_error(UsgsM2mErrorCode::ConfigInvalid))?
                .lookup(entity_id);
        }
        match lookup {
            BrowseLookup::Available(locator) => UsgsM2mBrowseClient::new(self.network_policy())
                .and_then(|client| client.fetch(locator.as_str(), cancelled))
                .map(Some),
            BrowseLookup::Unavailable => Ok(None),
            BrowseLookup::Invalid | BrowseLookup::Missing => {
                Err(scene_error(UsgsM2mErrorCode::ResponseInvalid)
                    .with_detail("browse_evidence_invalid"))
            }
        }
    }
}

pub struct UsgsM2mSceneSearchRouter {
    client: Arc<UsgsM2mSceneSearchClient>,
    live_acceptance: Option<UsgsM2mSceneSearchAcceptance>,
    admission: SceneRouteAdmission,
}

impl UsgsM2mSceneSearchRouter {
    pub fn new(client: Arc<UsgsM2mSceneSearchClient>) -> Self {
        Self {
            client,
            live_acceptance: USGS_M2M_LANDSAT_C2_L2_SCENE_SEARCH_ACCEPTANCE,
            admission: SceneRouteAdmission::Standard,
        }
    }

    #[cfg(feature = "usgs-m2m-live-acceptance")]
    #[doc(hidden)]
    pub fn new_live_acceptance_candidate(client: Arc<UsgsM2mSceneSearchClient>) -> Self {
        Self {
            client,
            live_acceptance: None,
            admission: SceneRouteAdmission::LiveAcceptanceCandidate,
        }
    }

    pub const fn state(&self) -> UsgsM2mSceneSearchState {
        if self.live_acceptance.is_some() {
            UsgsM2mSceneSearchState::LiveAccepted
        } else {
            UsgsM2mSceneSearchState::OfflineImplemented
        }
    }

    pub const fn production_enabled(&self) -> bool {
        self.live_acceptance.is_some()
    }

    pub fn network_policy(&self) -> ProviderNetworkPolicy {
        self.client.network_policy()
    }

    pub fn search_catalog(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<CatalogSearchResponse, UsgsM2mError> {
        if self.live_acceptance.is_none() && matches!(self.admission, SceneRouteAdmission::Standard)
        {
            return Err(UsgsM2mError::new(
                UsgsM2mErrorCode::ConfigInvalid,
                UsgsM2mEndpointClass::SceneSearchRouter,
            ));
        }
        self.client
            .search_all_for_use(query, RouteUse::Production, self.admission, cancelled)
    }

    pub fn fetch_browse(
        &self,
        query: &CatalogSearchQuery,
        entity_id: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<Option<UsgsM2mBrowseImage>, UsgsM2mError> {
        if self.live_acceptance.is_none() && matches!(self.admission, SceneRouteAdmission::Standard)
        {
            return Err(UsgsM2mError::new(
                UsgsM2mErrorCode::ConfigInvalid,
                UsgsM2mEndpointClass::SceneSearchRouter,
            ));
        }
        self.client.browse_for_query(
            query,
            RouteUse::Production,
            self.admission,
            entity_id,
            cancelled,
        )
    }

    #[cfg(test)]
    fn search_catalog_offline(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<CatalogSearchResponse, UsgsM2mError> {
        self.client.search_all_for_use(
            query,
            RouteUse::OfflineFixture,
            SceneRouteAdmission::Standard,
            cancelled,
        )
    }

    #[cfg(test)]
    fn search_page_offline(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<CatalogSearchResponse, UsgsM2mError> {
        self.client
            .search_page_for_use(query, RouteUse::OfflineFixture, cancelled)
    }
}

#[cfg(test)]
fn decode_search_facts(
    query: &CatalogSearchQuery,
    use_case: RouteUse,
) -> Result<LandsatC2L2SearchFacts, UsgsM2mError> {
    decode_search_facts_for_admission(query, use_case, SceneRouteAdmission::Standard)
}

fn decode_search_facts_for_admission(
    query: &CatalogSearchQuery,
    use_case: RouteUse,
    admission: SceneRouteAdmission,
) -> Result<LandsatC2L2SearchFacts, UsgsM2mError> {
    query
        .validate()
        .map_err(|_| scene_error(UsgsM2mErrorCode::ConfigInvalid))?;
    match admission {
        SceneRouteAdmission::Standard => DatasetRoutePolicyRegistry.resolve(
            &query.logical_dataset,
            &query.provider_id,
            &query.native_collection_id,
            use_case,
        ),
        #[cfg(feature = "usgs-m2m-live-acceptance")]
        SceneRouteAdmission::LiveAcceptanceCandidate if use_case == RouteUse::Production => {
            DatasetRoutePolicyRegistry.resolve_live_acceptance_candidate(
                &query.logical_dataset,
                &query.provider_id,
                &query.native_collection_id,
            )
        }
        #[cfg(feature = "usgs-m2m-live-acceptance")]
        SceneRouteAdmission::LiveAcceptanceCandidate => {
            return Err(scene_error(UsgsM2mErrorCode::ConfigInvalid));
        }
    }
    .map_err(|_| scene_error(UsgsM2mErrorCode::ConfigInvalid))?;
    if query.filters.len() != 4
        || query.filters.keys().any(|key| {
            ![
                "cloud_cover_lte",
                "datetime",
                "spacecraft",
                "processing_levels",
            ]
            .contains(&key.as_str())
        })
    {
        return Err(scene_error(UsgsM2mErrorCode::ConfigInvalid));
    }
    validate_direct_aoi(
        query
            .aoi_geojson
            .as_ref()
            .ok_or_else(|| scene_error(UsgsM2mErrorCode::ConfigInvalid))?,
    )?;
    let datetime = exact_object(query.filters.get("datetime"))?;
    if datetime.len() != 2 || datetime.keys().any(|key| key != "start" && key != "end") {
        return Err(scene_error(UsgsM2mErrorCode::ConfigInvalid));
    }
    let start = parse_normalized_utc(datetime.get("start"))?;
    let end = parse_normalized_utc(datetime.get("end"))?;
    if start >= end {
        return Err(scene_error(UsgsM2mErrorCode::ConfigInvalid));
    }
    let cloud_cover_lte = query
        .filters
        .get("cloud_cover_lte")
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && (0.0..=100.0).contains(value))
        .ok_or_else(|| scene_error(UsgsM2mErrorCode::ConfigInvalid))?;
    let spacecraft = closed_string_set(
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
        closed_string_set(query.filters.get("processing_levels"), &["l2sp", "l2sr"])?;
    let fingerprint = query_fingerprint(
        query,
        start,
        end,
        cloud_cover_lte,
        &spacecraft,
        &processing_levels,
    )?;
    let starting_number = match query.cursor.as_deref() {
        None => 1,
        Some(cursor) => unwrap_cursor(cursor, &fingerprint)?,
    };
    Ok(LandsatC2L2SearchFacts {
        start,
        end,
        cloud_cover_lte,
        spacecraft,
        processing_levels,
        fingerprint,
        starting_number,
        result_limit: query.result_limit,
    })
}

fn exact_object(value: Option<&Value>) -> Result<&Map<String, Value>, UsgsM2mError> {
    value
        .and_then(Value::as_object)
        .ok_or_else(|| scene_error(UsgsM2mErrorCode::ConfigInvalid))
}

fn parse_normalized_utc(value: Option<&Value>) -> Result<DateTime<Utc>, UsgsM2mError> {
    let text = value
        .and_then(Value::as_str)
        .ok_or_else(|| scene_error(UsgsM2mErrorCode::ConfigInvalid))?;
    let parsed = DateTime::parse_from_rfc3339(text)
        .map_err(|_| scene_error(UsgsM2mErrorCode::ConfigInvalid))?;
    if parsed.offset().local_minus_utc() != 0
        || parsed.to_rfc3339_opts(SecondsFormat::AutoSi, true) != text
    {
        return Err(scene_error(UsgsM2mErrorCode::ConfigInvalid));
    }
    Ok(parsed.with_timezone(&Utc))
}

fn closed_string_set(
    value: Option<&Value>,
    allowed: &[&'static str],
) -> Result<BTreeSet<&'static str>, UsgsM2mError> {
    let values = value
        .and_then(Value::as_array)
        .filter(|values| !values.is_empty() && values.len() <= allowed.len())
        .ok_or_else(|| scene_error(UsgsM2mErrorCode::ConfigInvalid))?;
    let mut result = BTreeSet::new();
    for value in values {
        let value = value
            .as_str()
            .ok_or_else(|| scene_error(UsgsM2mErrorCode::ConfigInvalid))?;
        let canonical = allowed
            .iter()
            .copied()
            .find(|allowed| *allowed == value)
            .ok_or_else(|| scene_error(UsgsM2mErrorCode::ConfigInvalid))?;
        if !result.insert(canonical) {
            return Err(scene_error(UsgsM2mErrorCode::ConfigInvalid));
        }
    }
    Ok(result)
}

#[derive(Default)]
struct GeometryLimits {
    polygons: usize,
    rings: usize,
    positions: usize,
}

fn validate_direct_aoi(aoi: &Map<String, Value>) -> Result<(), UsgsM2mError> {
    if aoi.len() != 2 || !aoi.contains_key("type") || !aoi.contains_key("coordinates") {
        return Err(scene_error(UsgsM2mErrorCode::ConfigInvalid));
    }
    let mut limits = GeometryLimits::default();
    match aoi.get("type").and_then(Value::as_str) {
        Some("Polygon") => validate_polygon(aoi.get("coordinates"), &mut limits)?,
        Some("MultiPolygon") => {
            let polygons = aoi
                .get("coordinates")
                .and_then(Value::as_array)
                .filter(|values| !values.is_empty())
                .ok_or_else(|| scene_error(UsgsM2mErrorCode::ConfigInvalid))?;
            for polygon in polygons {
                validate_polygon(Some(polygon), &mut limits)?;
            }
        }
        _ => return Err(scene_error(UsgsM2mErrorCode::ConfigInvalid)),
    }
    Ok(())
}

fn validate_polygon(
    coordinates: Option<&Value>,
    limits: &mut GeometryLimits,
) -> Result<(), UsgsM2mError> {
    limits.polygons += 1;
    let rings = coordinates
        .and_then(Value::as_array)
        .filter(|values| !values.is_empty())
        .ok_or_else(|| scene_error(UsgsM2mErrorCode::ConfigInvalid))?;
    limits.rings += rings.len();
    if limits.polygons > MAX_POLYGONS || limits.rings > MAX_RINGS {
        return Err(scene_error(UsgsM2mErrorCode::ConfigInvalid));
    }
    for ring in rings {
        let positions = ring
            .as_array()
            .ok_or_else(|| scene_error(UsgsM2mErrorCode::ConfigInvalid))?;
        limits.positions += positions.len();
        if limits.positions > MAX_POSITIONS {
            return Err(scene_error(UsgsM2mErrorCode::ConfigInvalid));
        }
    }
    Ok(())
}

fn query_fingerprint(
    query: &CatalogSearchQuery,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    cloud_cover_lte: f64,
    spacecraft: &BTreeSet<&str>,
    processing_levels: &BTreeSet<&str>,
) -> Result<String, UsgsM2mError> {
    let aoi = serde_json::to_string(&query.aoi_geojson)
        .map_err(|_| scene_error(UsgsM2mErrorCode::ConfigInvalid))?;
    let canonical = format!(
        "route={USGS_M2M_SCENE_SEARCH_ROUTE_ID}|profile={LANDSAT_C2_L2_SCENE_PROFILE_ID}|provider={}|dataset={}|aoi={aoi}|start={}|end={}|cloud={cloud_cover_lte}|spacecraft={}|levels={}|result_limit={}",
        query.provider_id,
        query.native_collection_id,
        start.to_rfc3339_opts(SecondsFormat::AutoSi, true),
        end.to_rfc3339_opts(SecondsFormat::AutoSi, true),
        spacecraft.iter().copied().collect::<Vec<_>>().join(","),
        processing_levels.iter().copied().collect::<Vec<_>>().join(","),
        query
            .result_limit
            .map_or_else(|| "ALL".to_owned(), |value| value.to_string()),
    );
    Ok(hex_digest(canonical.as_bytes()))
}

fn hex_digest(value: &[u8]) -> String {
    Sha256::digest(value)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn wrap_cursor(fingerprint: &str, starting_number: u64) -> Result<String, UsgsM2mError> {
    if !lower_hex_64(fingerprint) || starting_number == 0 {
        return Err(scene_error(UsgsM2mErrorCode::PaginationInvalid));
    }
    Ok(format!(
        "{USGS_M2M_SCENE_SEARCH_CURSOR_VERSION}:{fingerprint}:{starting_number}"
    ))
}

fn unwrap_cursor(cursor: &str, fingerprint: &str) -> Result<u64, UsgsM2mError> {
    if cursor.len() > MAX_CURSOR_BYTES {
        return Err(scene_error(UsgsM2mErrorCode::PaginationInvalid));
    }
    let mut parts = cursor.split(':');
    let version = parts.next();
    let actual_fingerprint = parts.next();
    let starting_number = parts.next();
    if parts.next().is_some()
        || version != Some(USGS_M2M_SCENE_SEARCH_CURSOR_VERSION)
        || actual_fingerprint != Some(fingerprint)
        || !lower_hex_64(fingerprint)
    {
        return Err(scene_error(UsgsM2mErrorCode::PaginationInvalid));
    }
    starting_number
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| scene_error(UsgsM2mErrorCode::PaginationInvalid))
}

fn lower_hex_64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn parse_satellite_filter(mut data: SensitiveJson, requested: &str) -> Result<Value, UsgsM2mError> {
    let records = match data.take() {
        Value::Array(records) if records.len() <= MAX_DATASET_FILTERS => records,
        mut invalid => {
            zeroize_json(&mut invalid);
            return Err(filter_error(UsgsM2mErrorCode::ResponseInvalid));
        }
    };
    let mut selected: Option<(String, Value)> = None;
    for record in records {
        let Value::Object(mut record) = record else {
            return Err(filter_error(UsgsM2mErrorCode::ResponseInvalid));
        };
        let is_satellite = record
            .remove("fieldLabel")
            .and_then(|value| value.as_str().map(str::to_owned))
            .as_deref()
            == Some("Satellite");
        if !is_satellite {
            let mut ignored = Value::Object(record);
            zeroize_json(&mut ignored);
            continue;
        }
        let filter_id = take_filter_identifier(&mut record, "id")?;
        let field_config = record
            .remove("fieldConfig")
            .and_then(|value| value.as_object().cloned())
            .ok_or_else(|| filter_error(UsgsM2mErrorCode::ResponseInvalid))?;
        if field_config.get("type").and_then(Value::as_str) != Some("Select") {
            return Err(filter_error(UsgsM2mErrorCode::ResponseInvalid));
        }
        let value_list = record
            .remove("valueList")
            .and_then(|value| value.as_object().cloned())
            .ok_or_else(|| filter_error(UsgsM2mErrorCode::ResponseInvalid))?;
        if value_list
            .get(requested)
            .and_then(Value::as_str)
            .is_none_or(|label| !valid_text(label, MAX_TEXT_BYTES))
        {
            return Err(filter_error(UsgsM2mErrorCode::ResponseInvalid));
        }
        let filter = json!({
            "filterType": "value",
            "filterId": filter_id,
            "value": requested,
        });
        match &selected {
            Some((existing_id, existing)) if existing_id == &filter_id && existing == &filter => {}
            Some(_) => return Err(filter_error(UsgsM2mErrorCode::DatasetConflict)),
            None => selected = Some((filter_id, filter)),
        }
        let mut ignored = Value::Object(record);
        zeroize_json(&mut ignored);
    }
    selected
        .map(|(_, filter)| filter)
        .ok_or_else(|| filter_error(UsgsM2mErrorCode::DatasetNotFound))
}

fn exact_product_filter(data: &Value, display_id: &str) -> Result<Value, UsgsM2mError> {
    let records = data
        .as_array()
        .filter(|rows| rows.len() <= MAX_DATASET_FILTERS)
        .ok_or_else(|| filter_error(UsgsM2mErrorCode::ResponseInvalid))?;
    let mut ids = BTreeSet::new();
    for record in records {
        let record = record
            .as_object()
            .ok_or_else(|| filter_error(UsgsM2mErrorCode::ResponseInvalid))?;
        if record.get("fieldLabel").and_then(Value::as_str) != Some("Landsat Product Identifier L2")
        {
            continue;
        }
        let id = record
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| valid_identifier(id, MAX_TEXT_BYTES))
            .ok_or_else(|| filter_error(UsgsM2mErrorCode::ResponseInvalid))?;
        ids.insert(id);
    }
    if ids.len() != 1 {
        return Err(filter_error(UsgsM2mErrorCode::DatasetConflict));
    }
    Ok(json!({"filterType": "value", "filterId": ids.first().unwrap(), "value": display_id}))
}

fn build_scene_search_payload(
    query: &CatalogSearchQuery,
    facts: &LandsatC2L2SearchFacts,
    satellite_filter: Option<&Value>,
) -> Result<Value, UsgsM2mError> {
    let aoi_geojson = query
        .aoi_geojson
        .as_ref()
        .ok_or_else(|| scene_error(UsgsM2mErrorCode::ConfigInvalid))?;
    let mut scene_filter = Map::from_iter([
        (
            "spatialFilter".to_owned(),
            json!({
                "filterType": "geojson",
                "geoJson": Value::Object(aoi_geojson.clone()),
            }),
        ),
        (
            "acquisitionFilter".to_owned(),
            json!({
                "start": facts.start.format("%Y-%m-%d").to_string(),
                "end": facts.end.format("%Y-%m-%d").to_string(),
            }),
        ),
        (
            "cloudCoverFilter".to_owned(),
            json!({"min": 0, "max": facts.cloud_cover_lte}),
        ),
    ]);
    if let Some(filter) = satellite_filter {
        scene_filter.insert("metadataFilter".to_owned(), filter.clone());
    }
    let dataset_name = if query.native_collection_id == "landsat_tm_c2_l2"
        || (facts
            .spacecraft
            .iter()
            .all(|s| *s == "landsat_4" || *s == "landsat_5")
            && !facts.spacecraft.is_empty())
    {
        "landsat_tm_c2_l2"
    } else if query.native_collection_id == "landsat_etm_c2_l2"
        || (facts.spacecraft.iter().all(|s| *s == "landsat_7") && !facts.spacecraft.is_empty())
    {
        "landsat_etm_c2_l2"
    } else {
        LANDSAT_C2_L2_NATIVE_COLLECTION_ID
    };
    Ok(json!({
        "datasetName": dataset_name,
        "metadataType": "full",
        "maxResults": transport_page_size(query.result_limit),
        "startingNumber": facts.starting_number,
        "sceneFilter": scene_filter,
    }))
}

struct ParsedPageEnvelope {
    records: Vec<Value>,
    records_returned: u64,
    total_hits: u64,
    next_starting_number: Option<u64>,
}

struct ParsedScenePage {
    products: BTreeMap<String, ProviderProduct>,
    matched_count: u64,
    fetched_record_count: u64,
    mapped_record_count: u64,
    exact_duplicate_record_count: u64,
    next_cursor: Option<String>,
}

impl ParsedScenePage {
    #[cfg(test)]
    fn into_response(
        self,
        facts: &LandsatC2L2SearchFacts,
        use_case: RouteUse,
    ) -> Result<CatalogSearchResponse, UsgsM2mError> {
        let unique_provider_product_count = self.products.len() as u64;
        let mut products = self
            .products
            .into_values()
            .filter_map(|product| match scene_matches_query(&product, facts) {
                Ok(true) => Some(Ok(product)),
                Ok(false) => None,
                Err(error) => Some(Err(error)),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let exact_matched_count = products.len() as u64;
        let remote_exhausted = self.next_cursor.is_none();
        let working_set_satisfied = facts
            .result_limit
            .map(|target| exact_matched_count >= target || remote_exhausted)
            .unwrap_or(remote_exhausted);
        if !working_set_satisfied {
            return Err(scene_error(UsgsM2mErrorCode::SceneLimitExceeded));
        }
        if let Some(target) = facts.result_limit {
            products.truncate(target as usize);
        }
        let returned_count = products.len() as u64;
        let response = CatalogSearchResponse {
            collection: collection_descriptor(use_case)?,
            products,
            stats: CatalogSearchStats {
                requested_limit: facts.result_limit,
                matched_count: Some(self.matched_count),
                request_count: 1,
                transport_part_count: 1,
                fetched_record_count: self.fetched_record_count,
                mapped_record_count: self.mapped_record_count,
                profile_filtered_record_count: 0,
                exact_duplicate_record_count: self.exact_duplicate_record_count,
                unique_provider_product_count,
                exact_matched_count,
                returned_count,
                remote_exhausted,
                working_set_satisfied,
                elapsed_ms: 0,
                truncated: self.next_cursor.is_some(),
            },
            next_cursor: self.next_cursor,
            item_errors: Vec::new(),
        };
        response
            .validate()
            .map_err(|_| scene_error(UsgsM2mErrorCode::ResponseInvalid))?;
        Ok(response)
    }
}

fn parse_scene_page(
    mut data: SensitiveJson,
    query: &CatalogSearchQuery,
    facts: &LandsatC2L2SearchFacts,
) -> Result<ParsedScenePage, UsgsM2mError> {
    let parsed = parse_page_envelope(
        data.take(),
        transport_page_size(query.result_limit),
        facts.starting_number,
    )
    .map_err(|error| error.with_detail("page_envelope"))?;
    let fetched_record_count = parsed.records_returned;
    let mut unique = BTreeMap::<String, ProviderProduct>::new();
    let mut duplicate_count = 0_u64;
    for record in parsed.records {
        let mapped = map_scene(record).map_err(|error| error.with_detail("scene_record"))?;
        match unique.get(&mapped.product_id) {
            Some(existing) if existing == &mapped => duplicate_count += 1,
            Some(_) => return Err(scene_error(UsgsM2mErrorCode::SceneConflict)),
            None => {
                unique.insert(mapped.product_id.clone(), mapped);
            }
        }
    }
    let next_cursor = parsed
        .next_starting_number
        .map(|starting| wrap_cursor(&facts.fingerprint, starting))
        .transpose()?;
    Ok(ParsedScenePage {
        products: unique,
        matched_count: parsed.total_hits,
        fetched_record_count,
        mapped_record_count: fetched_record_count,
        exact_duplicate_record_count: duplicate_count,
        next_cursor,
    })
}

fn parse_page_envelope(
    value: Value,
    page_size: u64,
    expected_starting_number: u64,
) -> Result<ParsedPageEnvelope, UsgsM2mError> {
    let Value::Object(mut map) = value else {
        return Err(scene_error(UsgsM2mErrorCode::ResponseInvalid));
    };
    let records = match map.remove("results") {
        Some(Value::Array(records)) => records,
        _ => return Err(scene_error(UsgsM2mErrorCode::ResponseInvalid)),
    };
    let records_returned = take_u64(&mut map, "recordsReturned")?;
    let total_hits = take_u64(&mut map, "totalHits")?;
    let starting_number = take_u64(&mut map, "startingNumber")?;
    let next_record = take_u64(&mut map, "nextRecord")?;
    let total_hits_accuracy = take_string(&mut map, "totalHitsAccuracy")?;
    let is_customized = take_bool(&mut map, "isCustomized")?;
    let num_excluded = take_u64(&mut map, "numExcluded")?;
    let mut ignored = Value::Object(map);
    zeroize_json(&mut ignored);

    if records_returned != records.len() as u64
        || records_returned > page_size
        || starting_number != expected_starting_number
        || total_hits_accuracy != "calculated"
        || is_customized
        || num_excluded != 0
        || (total_hits == 0
            && (records_returned != 0 || starting_number != 1 || !matches!(next_record, 0 | 1)))
        || (total_hits > 0 && records_returned == 0)
    {
        return Err(scene_error(UsgsM2mErrorCode::PaginationInvalid));
    }
    let last_record = if records_returned == 0 {
        0
    } else {
        starting_number
            .checked_add(records_returned - 1)
            .ok_or_else(|| scene_error(UsgsM2mErrorCode::PaginationInvalid))?
    };
    if last_record > total_hits {
        return Err(scene_error(UsgsM2mErrorCode::PaginationInvalid));
    }
    let next_starting_number = if last_record < total_hits {
        let expected = last_record
            .checked_add(1)
            .ok_or_else(|| scene_error(UsgsM2mErrorCode::PaginationInvalid))?;
        if next_record != expected || expected <= starting_number {
            return Err(scene_error(UsgsM2mErrorCode::PaginationInvalid));
        }
        Some(expected)
    } else {
        if total_hits > 0 && next_record != total_hits && next_record != last_record + 1 {
            return Err(scene_error(UsgsM2mErrorCode::PaginationInvalid));
        }
        None
    };
    Ok(ParsedPageEnvelope {
        records,
        records_returned,
        total_hits,
        next_starting_number,
    })
}

#[derive(Debug)]
struct SceneAuthority {
    entity_id: String,
    product_id: String,
    scene_id: Option<String>,
    acquisition_datetime: String,
    acquisition_date: String,
    spacecraft: &'static str,
    processing_level: &'static str,
    data_type_l2: String,
    collection_category: String,
    cloud_cover: f64,
    wrs_path: String,
    wrs_row: String,
    footprint: Map<String, Value>,
    browse_available: bool,
}

fn map_scene(value: Value) -> Result<ProviderProduct, UsgsM2mError> {
    let Value::Object(mut map) = value else {
        return Err(scene_error(UsgsM2mErrorCode::ResponseInvalid).with_detail("scene_object"));
    };
    let entity_id = take_identifier(&mut map, "entityId")
        .map_err(|error| error.with_detail("scene_entity_id"))?;
    let display_id = take_identifier(&mut map, "displayId")
        .map_err(|error| error.with_detail("scene_display_id"))?;
    let cloud_cover = take_number(&mut map, "cloudCover")
        .map_err(|error| error.with_detail("scene_cloud_cover"))?;
    if !(0.0..=100.0).contains(&cloud_cover) {
        return Err(scene_error(UsgsM2mErrorCode::ResponseInvalid).with_detail("scene_cloud_cover"));
    }
    let metadata = map
        .remove("metadata")
        .and_then(|value| value.as_array().cloned())
        .filter(|values| values.len() <= MAX_METADATA_ITEMS)
        .ok_or_else(|| {
            scene_error(UsgsM2mErrorCode::ResponseInvalid).with_detail("scene_metadata_array")
        })?;
    let browse = map
        .remove("browse")
        .and_then(|value| value.as_array().cloned())
        .ok_or_else(|| {
            scene_error(UsgsM2mErrorCode::ResponseInvalid).with_detail("scene_browse_array")
        })?;
    let footprint = take_footprint(&mut map, "spatialCoverage")
        .map_err(|error| error.with_detail("scene_spatial_coverage"))?;
    let temporal = map
        .remove("temporalCoverage")
        .and_then(|value| value.as_object().cloned())
        .ok_or_else(|| {
            scene_error(UsgsM2mErrorCode::ResponseInvalid).with_detail("scene_temporal_coverage")
        })?;
    let mut ignored = Value::Object(map);
    zeroize_json(&mut ignored);

    let metadata = authoritative_metadata(metadata)?;
    let product_id = required_metadata(&metadata, "Landsat Product Identifier L2")
        .map_err(|error| error.with_detail("metadata_product_id"))?;
    if product_id != display_id {
        return Err(scene_error(UsgsM2mErrorCode::SceneConflict));
    }
    let scene_id = metadata.get("Landsat Scene Identifier").cloned();
    if scene_id
        .as_deref()
        .is_some_and(|value| !valid_identifier(value, 96))
    {
        return Err(scene_error(UsgsM2mErrorCode::ResponseInvalid).with_detail("metadata_scene_id"));
    }
    let acquisition_date = required_metadata(&metadata, "Date Acquired")
        .map_err(|error| error.with_detail("metadata_acquisition_date"))?;
    let acquisition_date_parsed = NaiveDate::parse_from_str(&acquisition_date, "%Y/%m/%d")
        .map_err(|_| {
            scene_error(UsgsM2mErrorCode::ResponseInvalid).with_detail("metadata_acquisition_date")
        })?;
    validate_temporal_coverage(&temporal, acquisition_date_parsed)
        .map_err(|error| error.with_detail("scene_temporal_coverage"))?;
    let acquisition_datetime = acquisition_date_parsed
        .and_hms_opt(0, 0, 0)
        .expect("midnight is valid")
        .and_utc()
        .to_rfc3339_opts(SecondsFormat::Secs, true);
    let satellite = required_metadata(&metadata, "Satellite")
        .map_err(|error| error.with_detail("metadata_satellite"))?;
    let spacecraft = match satellite.as_str() {
        "LANDSAT_4" => "landsat_4",
        "LANDSAT_5" => "landsat_5",
        "LANDSAT_7" => "landsat_7",
        "LANDSAT_8" => "landsat_8",
        "LANDSAT_9" => "landsat_9",
        _ => {
            return Err(
                scene_error(UsgsM2mErrorCode::ResponseInvalid).with_detail("metadata_satellite")
            )
        }
    };
    let data_type_l2 = required_metadata(&metadata, "Data Type L2")
        .map_err(|error| error.with_detail("metadata_data_type_l2"))?;
    let processing_level = if data_type_l2.ends_with("_L2SP") {
        "l2sp"
    } else if data_type_l2.ends_with("_L2SR") {
        "l2sr"
    } else {
        return Err(
            scene_error(UsgsM2mErrorCode::ResponseInvalid).with_detail("metadata_data_type_l2")
        );
    };
    let collection_category = required_metadata(&metadata, "Collection Category")
        .map_err(|error| error.with_detail("metadata_collection_category"))?;
    if !matches!(collection_category.as_str(), "T1" | "T2" | "RT") {
        return Err(scene_error(UsgsM2mErrorCode::ResponseInvalid)
            .with_detail("metadata_collection_category"));
    }
    let wrs_path = required_metadata(&metadata, "WRS Path")
        .map_err(|error| error.with_detail("metadata_wrs_path"))?;
    let wrs_row = required_metadata(&metadata, "WRS Row")
        .map_err(|error| error.with_detail("metadata_wrs_row"))?;
    validate_wrs(&wrs_path, 233).map_err(|error| error.with_detail("metadata_wrs_path"))?;
    validate_wrs(&wrs_row, 248).map_err(|error| error.with_detail("metadata_wrs_row"))?;
    let authority = SceneAuthority {
        entity_id,
        product_id,
        scene_id,
        acquisition_datetime,
        acquisition_date,
        spacecraft,
        processing_level,
        data_type_l2,
        collection_category,
        cloud_cover,
        wrs_path,
        wrs_row,
        footprint,
        browse_available: !browse.is_empty(),
    };
    authority_to_product(authority)
}

fn authority_to_product(authority: SceneAuthority) -> Result<ProviderProduct, UsgsM2mError> {
    let mut properties = Map::from_iter([
        (
            "entity_id".to_owned(),
            Value::String(authority.entity_id.clone()),
        ),
        (
            "landsat_product_id".to_owned(),
            Value::String(authority.product_id.clone()),
        ),
        (
            "dataset_alias".to_owned(),
            Value::String(LANDSAT_C2_L2_NATIVE_COLLECTION_ID.to_owned()),
        ),
        (
            "spacecraft".to_owned(),
            Value::String(authority.spacecraft.to_owned()),
        ),
        (
            "processing_level".to_owned(),
            Value::String(authority.processing_level.to_owned()),
        ),
        (
            "data_type_l2".to_owned(),
            Value::String(authority.data_type_l2),
        ),
        (
            "collection_category".to_owned(),
            Value::String(authority.collection_category),
        ),
        (
            "cloud_cover".to_owned(),
            Number::from_f64(authority.cloud_cover)
                .map(Value::Number)
                .ok_or_else(|| scene_error(UsgsM2mErrorCode::ResponseInvalid))?,
        ),
        ("wrs_path".to_owned(), Value::String(authority.wrs_path)),
        ("wrs_row".to_owned(), Value::String(authority.wrs_row)),
        (
            "acquisition_date".to_owned(),
            Value::String(authority.acquisition_date),
        ),
        (
            "acquisition_precision".to_owned(),
            Value::String("day".to_owned()),
        ),
        (
            "browse_available".to_owned(),
            Value::Bool(authority.browse_available),
        ),
    ]);
    if let Some(scene_id) = authority.scene_id {
        properties.insert("landsat_scene_id".to_owned(), Value::String(scene_id));
    }
    let product = ProviderProduct {
        provider_id: LANDSAT_C2_L2_PROVIDER_ID.to_owned(),
        product_id: authority.entity_id,
        display_name: authority.product_id,
        source_kind: DataSourceKind::Landsat,
        product_kind: ProductKind::OpticalScene,
        acquisition_datetime: Some(authority.acquisition_datetime),
        footprint_geojson: Some(authority.footprint),
        footprint_bbox: None,
        assets: Vec::new(),
        properties,
        payload: Map::new(),
    };
    product.validate().map_err(|_| {
        scene_error(UsgsM2mErrorCode::ResponseInvalid).with_detail("provider_product_validation")
    })?;
    Ok(product)
}

fn authoritative_metadata(values: Vec<Value>) -> Result<BTreeMap<String, String>, UsgsM2mError> {
    const RELEVANT: &[&str] = &[
        "Landsat Product Identifier L2",
        "Landsat Scene Identifier",
        "Date Acquired",
        "Collection Category",
        "WRS Path",
        "WRS Row",
        "Data Type L2",
        "Satellite",
    ];
    let mut result = BTreeMap::new();
    for value in values {
        let Value::Object(mut item) = value else {
            return Err(
                scene_error(UsgsM2mErrorCode::ResponseInvalid).with_detail("metadata_item_shape")
            );
        };
        let field_name = take_string(&mut item, "fieldName")
            .map_err(|error| error.with_detail("metadata_field_name"))?;
        if !valid_text(&field_name, MAX_TEXT_BYTES) {
            return Err(
                scene_error(UsgsM2mErrorCode::ResponseInvalid).with_detail("metadata_field_name")
            );
        }
        if RELEVANT.contains(&field_name.as_str()) {
            let field_value = take_authoritative_metadata_value(&mut item, &field_name)
                .map_err(|error| error.with_detail("metadata_authoritative_value"))?;
            if let Some(field_value) = field_value {
                if !valid_text(&field_value, MAX_TEXT_BYTES) {
                    return Err(scene_error(UsgsM2mErrorCode::ResponseInvalid)
                        .with_detail("metadata_authoritative_value"));
                }
                match result.get(&field_name) {
                    Some(existing) if existing == &field_value => {}
                    Some(_) => return Err(scene_error(UsgsM2mErrorCode::SceneConflict)),
                    None => {
                        result.insert(field_name, field_value);
                    }
                }
            }
        }
        let mut ignored = Value::Object(item);
        zeroize_json(&mut ignored);
    }
    Ok(result)
}

fn take_authoritative_metadata_value(
    item: &mut Map<String, Value>,
    field_name: &str,
) -> Result<Option<String>, UsgsM2mError> {
    let mut value = item
        .remove("value")
        .ok_or_else(|| scene_error(UsgsM2mErrorCode::ResponseInvalid))?;
    let normalized = match (&value, field_name) {
        (Value::String(value), "Landsat Scene Identifier") if value.trim().is_empty() => Some(None),
        (Value::String(value), _) => Some(Some(value.trim().to_owned())),
        (Value::Number(value), "Satellite") => match value.as_f64() {
            Some(8.0) => Some(Some("LANDSAT_8".to_owned())),
            Some(9.0) => Some(Some("LANDSAT_9".to_owned())),
            _ => None,
        },
        _ => None,
    };
    zeroize_json(&mut value);
    normalized.ok_or_else(|| scene_error(UsgsM2mErrorCode::ResponseInvalid))
}

fn required_metadata(
    metadata: &BTreeMap<String, String>,
    field: &str,
) -> Result<String, UsgsM2mError> {
    metadata
        .get(field)
        .cloned()
        .ok_or_else(|| scene_error(UsgsM2mErrorCode::ResponseInvalid))
}

fn validate_temporal_coverage(
    temporal: &Map<String, Value>,
    acquisition_date: NaiveDate,
) -> Result<(), UsgsM2mError> {
    if temporal.len() != 2 {
        return Err(scene_error(UsgsM2mErrorCode::ResponseInvalid));
    }
    for field in ["startDate", "endDate"] {
        let value = temporal
            .get(field)
            .and_then(Value::as_str)
            .ok_or_else(|| scene_error(UsgsM2mErrorCode::ResponseInvalid))?;
        let parsed = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
            .map_err(|_| scene_error(UsgsM2mErrorCode::ResponseInvalid))?;
        if parsed.date() != acquisition_date || parsed.time() != chrono::NaiveTime::MIN {
            return Err(scene_error(UsgsM2mErrorCode::SceneConflict));
        }
    }
    Ok(())
}

fn take_footprint(
    map: &mut Map<String, Value>,
    field: &str,
) -> Result<Map<String, Value>, UsgsM2mError> {
    let footprint = map
        .remove(field)
        .and_then(|value| value.as_object().cloned())
        .ok_or_else(|| scene_error(UsgsM2mErrorCode::ResponseInvalid))?;
    validate_direct_aoi(&footprint).map_err(|_| scene_error(UsgsM2mErrorCode::ResponseInvalid))?;
    Ok(footprint)
}

fn scene_matches_query(
    product: &ProviderProduct,
    facts: &LandsatC2L2SearchFacts,
) -> Result<bool, UsgsM2mError> {
    let acquisition = product
        .acquisition_datetime
        .as_deref()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc))
        .ok_or_else(|| scene_error(UsgsM2mErrorCode::ResponseInvalid))?;
    let cloud_cover = product
        .properties
        .get("cloud_cover")
        .and_then(Value::as_f64)
        .ok_or_else(|| scene_error(UsgsM2mErrorCode::ResponseInvalid))?;
    let spacecraft = product
        .properties
        .get("spacecraft")
        .and_then(Value::as_str)
        .ok_or_else(|| scene_error(UsgsM2mErrorCode::ResponseInvalid))?;
    let processing_level = product
        .properties
        .get("processing_level")
        .and_then(Value::as_str)
        .ok_or_else(|| scene_error(UsgsM2mErrorCode::ResponseInvalid))?;
    Ok(acquisition >= facts.start
        && acquisition < facts.end
        && cloud_cover <= facts.cloud_cover_lte
        && facts.spacecraft.contains(spacecraft)
        && facts.processing_levels.contains(processing_level))
}

#[cfg(test)]
fn collection_descriptor(use_case: RouteUse) -> Result<CatalogCollectionDescriptor, UsgsM2mError> {
    collection_descriptor_for_admission(use_case, SceneRouteAdmission::Standard)
}

fn collection_descriptor_for_admission(
    use_case: RouteUse,
    admission: SceneRouteAdmission,
) -> Result<CatalogCollectionDescriptor, UsgsM2mError> {
    let profile = CollectionProfileRegistry
        .get(LANDSAT_C2_L2_SCENE_PROFILE_ID)
        .ok_or_else(|| scene_error(UsgsM2mErrorCode::ConfigInvalid))?;
    let accepted =
        USGS_M2M_LANDSAT_C2_L2_SCENE_SEARCH_ACCEPTANCE.is_some() && profile.production_enabled;
    #[cfg(feature = "usgs-m2m-live-acceptance")]
    let accepted = accepted || matches!(admission, SceneRouteAdmission::LiveAcceptanceCandidate);
    #[cfg(not(feature = "usgs-m2m-live-acceptance"))]
    let _ = admission;
    Ok(CatalogCollectionDescriptor {
        logical_dataset: LANDSAT_C2_L2_LOGICAL_DATASET.to_owned(),
        provider_id: LANDSAT_C2_L2_PROVIDER_ID.to_owned(),
        native_collection_id: LANDSAT_C2_L2_NATIVE_COLLECTION_ID.to_owned(),
        collection_profile_id: profile.profile_id.to_owned(),
        identity_rule_id: USGS_LANDSAT_C2_L2_IDENTITY_RULE_ID.to_owned(),
        filename_rule_id: USGS_LANDSAT_C2_L2_BUNDLE_FILENAME_RULE_ID.to_owned(),
        delivery_model: CatalogDeliveryModel::ArchiveFirst,
        production_enabled: matches!(use_case, RouteUse::Production) && accepted,
    })
}

fn take_u64(map: &mut Map<String, Value>, field: &str) -> Result<u64, UsgsM2mError> {
    map.remove(field)
        .and_then(|value| value.as_u64())
        .ok_or_else(|| scene_error(UsgsM2mErrorCode::ResponseInvalid))
}

fn take_bool(map: &mut Map<String, Value>, field: &str) -> Result<bool, UsgsM2mError> {
    map.remove(field)
        .and_then(|value| value.as_bool())
        .ok_or_else(|| scene_error(UsgsM2mErrorCode::ResponseInvalid))
}

fn take_number(map: &mut Map<String, Value>, field: &str) -> Result<f64, UsgsM2mError> {
    map.remove(field)
        .and_then(|value| value.as_f64())
        .filter(|value| value.is_finite())
        .ok_or_else(|| scene_error(UsgsM2mErrorCode::ResponseInvalid))
}

fn take_string(map: &mut Map<String, Value>, field: &str) -> Result<String, UsgsM2mError> {
    map.remove(field)
        .and_then(|value| value.as_str().map(str::to_owned))
        .ok_or_else(|| scene_error(UsgsM2mErrorCode::ResponseInvalid))
}

fn take_identifier(map: &mut Map<String, Value>, field: &str) -> Result<String, UsgsM2mError> {
    let value = take_string(map, field)?;
    if valid_identifier(&value, 96) {
        Ok(value)
    } else {
        Err(scene_error(UsgsM2mErrorCode::ResponseInvalid))
    }
}

fn take_filter_identifier(
    map: &mut Map<String, Value>,
    field: &str,
) -> Result<String, UsgsM2mError> {
    let value = map
        .remove(field)
        .and_then(|value| value.as_str().map(str::to_owned))
        .ok_or_else(|| filter_error(UsgsM2mErrorCode::ResponseInvalid))?;
    if valid_identifier(&value, 96) {
        Ok(value)
    } else {
        Err(filter_error(UsgsM2mErrorCode::ResponseInvalid))
    }
}

fn valid_identifier(value: &str, max_bytes: usize) -> bool {
    valid_text(value, max_bytes)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}

fn valid_text(value: &str, max_bytes: usize) -> bool {
    !value.is_empty()
        && value == value.trim()
        && value.len() <= max_bytes
        && !value.chars().any(char::is_control)
}

fn validate_wrs(value: &str, maximum: u16) -> Result<(), UsgsM2mError> {
    let parsed = (value.len() == 3 && value.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| value.parse::<u16>().ok())
        .flatten()
        .filter(|value| (1..=maximum).contains(value));
    parsed
        .map(|_| ())
        .ok_or_else(|| scene_error(UsgsM2mErrorCode::ResponseInvalid))
}

fn filter_error(code: UsgsM2mErrorCode) -> UsgsM2mError {
    UsgsM2mError::new(code, UsgsM2mEndpointClass::DatasetFilters)
}

fn scene_error(code: UsgsM2mErrorCode) -> UsgsM2mError {
    UsgsM2mError::new(code, UsgsM2mEndpointClass::SceneSearch)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use super::*;
    use crate::usgs_m2m::endpoint::UsgsM2mSessionEndpoint;
    use crate::usgs_m2m::{
        UsgsM2mApiKey, UsgsM2mApplicationToken, UsgsM2mLoginMaterial, UsgsM2mSessionStatus,
    };

    fn query() -> CatalogSearchQuery {
        serde_json::from_value(json!({
            "logical_dataset": LANDSAT_C2_L2_LOGICAL_DATASET,
            "provider_id": LANDSAT_C2_L2_PROVIDER_ID,
            "native_collection_id": LANDSAT_C2_L2_NATIVE_COLLECTION_ID,
            "aoi_geojson": {
                "type": "Polygon",
                "coordinates": [[[110.0, 30.0], [111.0, 30.0], [111.0, 31.0], [110.0, 30.0]]]
            },
            "result_limit": 2,
            "filters": {
                "datetime": {"start": "2026-01-01T00:00:00Z", "end": "2026-01-04T00:00:00Z"},
                "cloud_cover_lte": 40.0,
                "spacecraft": ["landsat_9"],
                "processing_levels": ["l2sp", "l2sr"]
            }
        }))
        .expect("closed Landsat query")
    }

    fn metadata(field_name: &str, value: &str) -> Value {
        json!({"id":"public-field-id", "fieldName":field_name, "dictionaryLink":"https://example.invalid", "value":value})
    }

    fn scene(entity_id: &str, product_id: &str, date: &str, level: &str) -> Value {
        json!({
            "entityId": entity_id,
            "displayId": product_id,
            "cloudCover": 12.5,
            "metadata": [
                metadata("Landsat Product Identifier L2", product_id),
                metadata("Landsat Scene Identifier", entity_id),
                metadata("Date Acquired", date),
                metadata("Collection Category", "T1"),
                metadata("WRS Path", "123"),
                metadata("WRS Row", "045"),
                metadata("Data Type L2", &format!("OLI_TIRS_{}", level.to_ascii_uppercase())),
                metadata("Satellite", "LANDSAT_9")
            ],
            "browse": [{"url":"https://browse.invalid/must-not-persist"}],
            "options": {"download": true},
            "spatialBounds": {"type":"Polygon","coordinates":[[[109.0,29.0],[112.0,29.0],[112.0,32.0],[109.0,29.0]]]},
            "spatialCoverage": {"type":"Polygon","coordinates":[[[110.0,30.0],[111.0,30.0],[111.0,31.0],[110.0,30.0]]]},
            "temporalCoverage": {"startDate":date.replace('/', "-") + " 00:00:00", "endDate":date.replace('/', "-") + " 00:00:00"}
        })
    }

    fn page(results: Vec<Value>, total_hits: u64, starting_number: u64, next_record: u64) -> Value {
        json!({
            "results": results,
            "recordsReturned": results.len(),
            "totalHits": total_hits,
            "totalHitsAccuracy": "calculated",
            "isCustomized": false,
            "numExcluded": 0,
            "startingNumber": starting_number,
            "nextRecord": next_record
        })
    }

    struct ExactProductEndpoint {
        filters: Value,
        response: Value,
        requests: Mutex<Vec<Value>>,
        filter_calls: AtomicUsize,
        cancel_after_filters: bool,
        cancelled: std::sync::atomic::AtomicBool,
    }

    impl UsgsM2mSessionEndpoint for ExactProductEndpoint {
        fn network_policy(&self) -> ProviderNetworkPolicy {
            ProviderNetworkPolicy::direct()
        }
        fn login_token(
            &self,
            _: &UsgsM2mLoginMaterial,
            _: &(dyn Fn() -> bool + Sync),
        ) -> Result<UsgsM2mApiKey, UsgsM2mError> {
            UsgsM2mApiKey::from_wire("fixture-runtime-key".into())
        }
        fn logout(
            &self,
            _: &UsgsM2mApiKey,
            _: &(dyn Fn() -> bool + Sync),
        ) -> Result<(), UsgsM2mError> {
            Ok(())
        }
        fn dataset_search(
            &self,
            _: &UsgsM2mApiKey,
            _: &(dyn Fn() -> bool + Sync),
        ) -> Result<SensitiveJson, UsgsM2mError> {
            panic!("exact matching must not perform unbounded discovery")
        }
        fn dataset_filters(
            &self,
            _: &UsgsM2mApiKey,
            dataset: &str,
            _: &(dyn Fn() -> bool + Sync),
        ) -> Result<SensitiveJson, UsgsM2mError> {
            assert_eq!(dataset, LANDSAT_C2_L2_NATIVE_COLLECTION_ID);
            self.filter_calls.fetch_add(1, Ordering::SeqCst);
            if self.cancel_after_filters {
                self.cancelled.store(true, Ordering::SeqCst);
            }
            Ok(SensitiveJson::new(self.filters.clone()))
        }
        fn scene_search(
            &self,
            _: &UsgsM2mApiKey,
            payload: &Value,
            _: &(dyn Fn() -> bool + Sync),
        ) -> Result<SensitiveJson, UsgsM2mError> {
            self.requests.lock().unwrap().push(payload.clone());
            Ok(SensitiveJson::new(self.response.clone()))
        }
    }

    const EXACT_DISPLAY: &str = "LC09_L2SP_123045_20260101_20260103_02_T1";
    fn exact_filters() -> Value {
        json!([{"id":"dynamic-product-filter", "fieldLabel":"Landsat Product Identifier L2", "fieldConfig":{"type":"Text"}}])
    }
    fn exact_client(
        filters: Value,
        response: Value,
        login: bool,
        cancel_after_filters: bool,
    ) -> (UsgsM2mSceneSearchClient, Arc<ExactProductEndpoint>) {
        let endpoint = Arc::new(ExactProductEndpoint {
            filters,
            response,
            requests: Mutex::new(Vec::new()),
            filter_calls: AtomicUsize::new(0),
            cancel_after_filters,
            cancelled: std::sync::atomic::AtomicBool::new(false),
        });
        let session = Arc::new(
            UsgsM2mSession::from_endpoint(ProviderNetworkPolicy::direct(), endpoint.clone())
                .unwrap(),
        );
        if login {
            session
                .login(
                    UsgsM2mLoginMaterial::new(
                        "fixture-user",
                        UsgsM2mApplicationToken::new("fixture-application-token").unwrap(),
                    )
                    .unwrap(),
                    &|| false,
                )
                .unwrap();
        }
        (UsgsM2mSceneSearchClient::new(session), endpoint)
    }
    fn exact_scene() -> Value {
        scene("LC91230452026001LGN00", EXACT_DISPLAY, "2026/01/01", "l2sp")
    }

    #[test]
    fn exact_official_product_uses_dynamic_filter_and_retains_authoritative_entity() {
        let (client, endpoint) = exact_client(
            exact_filters(),
            page(vec![exact_scene()], 1, 1, 2),
            true,
            false,
        );
        let product = client.exact_product(EXACT_DISPLAY, &|| false).unwrap();
        assert_eq!(product.product_id, "LC91230452026001LGN00");
        assert_eq!(product.display_name, EXACT_DISPLAY);
        assert_eq!(product.provider_id, LANDSAT_C2_L2_PROVIDER_ID);
        assert_eq!(
            endpoint.requests.lock().unwrap().as_slice(),
            &[json!({
                "datasetName": LANDSAT_C2_L2_NATIVE_COLLECTION_ID, "metadataType":"full",
                "maxResults":2, "startingNumber":1,
                "sceneFilter":{"metadataFilter":{"filterType":"value","filterId":"dynamic-product-filter","value":EXACT_DISPLAY}}
            })]
        );
        assert_eq!(client.session.request_counts().download_request, 0);
        assert!(!serde_json::to_string(&product)
            .unwrap()
            .contains("browse.invalid"));
    }

    #[test]
    fn exact_official_product_rejects_absent_ambiguous_reprocessed_and_conflicting_metadata() {
        let reprocessed = scene(
            "LC91230452026001LGN00",
            &EXACT_DISPLAY.replace("20260103", "20260104"),
            "2026/01/01",
            "l2sp",
        );
        let mut conflict = exact_scene();
        conflict["metadata"][0]["value"] = json!(EXACT_DISPLAY.replace("20260103", "20260104"));
        for response in [
            page(vec![], 0, 1, 0),
            page(vec![exact_scene(), exact_scene()], 2, 1, 3),
            page(vec![exact_scene()], 2, 1, 2),
            page(vec![reprocessed], 1, 1, 2),
            page(vec![conflict], 1, 1, 2),
        ] {
            let (client, endpoint) = exact_client(exact_filters(), response, true, false);
            assert!(client.exact_product(EXACT_DISPLAY, &|| false).is_err());
            assert_eq!(endpoint.requests.lock().unwrap().len(), 1);
        }
    }

    #[test]
    fn exact_official_product_has_no_request_after_invalid_filter_cancel_or_missing_session() {
        for filters in [
            json!([]),
            json!([
            {"id":"one", "fieldLabel":"Landsat Product Identifier L2"},
            {"id":"two", "fieldLabel":"Landsat Product Identifier L2"}]),
        ] {
            let (client, endpoint) =
                exact_client(filters, page(vec![exact_scene()], 1, 1, 2), true, false);
            assert!(client.exact_product(EXACT_DISPLAY, &|| false).is_err());
            assert!(endpoint.requests.lock().unwrap().is_empty());
        }
        for (login, cancel) in [(false, false), (true, true)] {
            let (client, endpoint) = exact_client(
                exact_filters(),
                page(vec![exact_scene()], 1, 1, 2),
                login,
                cancel,
            );
            assert!(client
                .exact_product(EXACT_DISPLAY, &|| endpoint.cancelled.load(Ordering::SeqCst))
                .is_err());
            assert!(endpoint.requests.lock().unwrap().is_empty());
            if !login {
                assert_eq!(endpoint.filter_calls.load(Ordering::SeqCst), 0);
            }
        }
        let (client, endpoint) = exact_client(exact_filters(), page(vec![], 0, 1, 0), true, false);
        assert!(client
            .exact_product("LC09_L2SP_123045_20260101_02_T1", &|| false)
            .is_err());
        assert!(client.exact_product(EXACT_DISPLAY, &|| true).is_err());
        assert_eq!(endpoint.filter_calls.load(Ordering::SeqCst), 0);
    }

    struct FixtureEndpoint {
        responses: Mutex<Vec<Value>>,
        filters_calls: AtomicUsize,
        scene_calls: AtomicUsize,
    }

    impl UsgsM2mSessionEndpoint for FixtureEndpoint {
        fn network_policy(&self) -> ProviderNetworkPolicy {
            ProviderNetworkPolicy::direct()
        }

        fn login_token(
            &self,
            _material: &UsgsM2mLoginMaterial,
            _cancelled: &(dyn Fn() -> bool + Sync),
        ) -> Result<UsgsM2mApiKey, UsgsM2mError> {
            UsgsM2mApiKey::from_wire("fixture-runtime-key".to_owned())
        }

        fn logout(
            &self,
            _api_key: &UsgsM2mApiKey,
            _cancelled: &(dyn Fn() -> bool + Sync),
        ) -> Result<(), UsgsM2mError> {
            Ok(())
        }

        fn dataset_search(
            &self,
            _api_key: &UsgsM2mApiKey,
            _cancelled: &(dyn Fn() -> bool + Sync),
        ) -> Result<SensitiveJson, UsgsM2mError> {
            Err(scene_error(UsgsM2mErrorCode::ConfigInvalid))
        }

        fn dataset_filters(
            &self,
            _api_key: &UsgsM2mApiKey,
            dataset_name: &str,
            _cancelled: &(dyn Fn() -> bool + Sync),
        ) -> Result<SensitiveJson, UsgsM2mError> {
            assert_eq!(dataset_name, LANDSAT_C2_L2_NATIVE_COLLECTION_ID);
            self.filters_calls.fetch_add(1, Ordering::SeqCst);
            Ok(SensitiveJson::new(json!([{
                "id":"current-satellite-filter-id",
                "fieldLabel":"Satellite",
                "fieldConfig":{"type":"Select"},
                "valueList":{"8":"Landsat 8", "9":"Landsat 9"}
            }])))
        }

        fn scene_search(
            &self,
            _api_key: &UsgsM2mApiKey,
            payload: &Value,
            _cancelled: &(dyn Fn() -> bool + Sync),
        ) -> Result<SensitiveJson, UsgsM2mError> {
            assert_eq!(payload["datasetName"], LANDSAT_C2_L2_NATIVE_COLLECTION_ID);
            assert_eq!(payload["metadataType"], "full");
            assert_eq!(
                payload["sceneFilter"]["spatialFilter"]["filterType"],
                "geojson"
            );
            assert!(payload["sceneFilter"]["spatialFilter"]
                .get("lowerLeft")
                .is_none());
            assert_eq!(
                payload["sceneFilter"]["metadataFilter"],
                json!({"filterType":"value","filterId":"current-satellite-filter-id","value":"9"})
            );
            self.scene_calls.fetch_add(1, Ordering::SeqCst);
            let mut responses = self.responses.lock().expect("fixture responses");
            if responses.is_empty() {
                return Err(scene_error(UsgsM2mErrorCode::ResponseInvalid));
            }
            Ok(SensitiveJson::new(responses.remove(0)))
        }
    }

    fn fixture_client(
        responses: Vec<Value>,
    ) -> (Arc<UsgsM2mSceneSearchClient>, Arc<FixtureEndpoint>) {
        let endpoint = Arc::new(FixtureEndpoint {
            responses: Mutex::new(responses),
            filters_calls: AtomicUsize::new(0),
            scene_calls: AtomicUsize::new(0),
        });
        let session = Arc::new(
            UsgsM2mSession::from_endpoint(ProviderNetworkPolicy::direct(), endpoint.clone())
                .expect("fixture session"),
        );
        assert_eq!(
            session
                .login(
                    UsgsM2mLoginMaterial::new(
                        "fixture-user",
                        UsgsM2mApplicationToken::new("fixture-application-token").unwrap(),
                    )
                    .unwrap(),
                    &|| false,
                )
                .unwrap(),
            UsgsM2mSessionStatus::Authenticated
        );
        let client = Arc::new(UsgsM2mSceneSearchClient::new(session));
        (client, endpoint)
    }

    fn fixture_router(responses: Vec<Value>) -> (UsgsM2mSceneSearchRouter, Arc<FixtureEndpoint>) {
        let (client, endpoint) = fixture_client(responses);
        (UsgsM2mSceneSearchRouter::new(client), endpoint)
    }

    #[cfg(feature = "usgs-m2m-live-acceptance")]
    #[test]
    fn live_acceptance_candidate_uses_production_rules_without_public_enablement() {
        let mut live_shape = scene(
            "LC91230452026001LGN00",
            "LC09_L2SP_123045_20260101_20260103_02_T1",
            "2026/01/01",
            "l2sp",
        );
        live_shape["metadata"][7]["value"] = json!(9);
        let (client, endpoint) = fixture_client(vec![page(vec![live_shape], 1, 1, 1)]);
        let replay_client = Arc::clone(&client);
        let router = UsgsM2mSceneSearchRouter::new_live_acceptance_candidate(client);

        assert_eq!(router.state(), UsgsM2mSceneSearchState::OfflineImplemented);
        assert!(!router.production_enabled());
        let response = router.search_catalog(&query(), &|| false).unwrap();
        assert!(response.collection.production_enabled);
        assert_eq!(response.products.len(), 1);
        assert_eq!(endpoint.filters_calls.load(Ordering::SeqCst), 1);
        assert_eq!(endpoint.scene_calls.load(Ordering::SeqCst), 1);

        let replays = replay_client.take_live_acceptance_redacted_replays();
        assert_eq!(replays.len(), 1);
        let serialized = serde_json::to_string(&replays).unwrap();
        assert!(!serialized.contains("LC91230452026001LGN00"));
        assert!(!serialized.contains("browse.invalid"));
        let facts = decode_search_facts(&query(), RouteUse::OfflineFixture).unwrap();
        let parsed = parse_scene_page(
            SensitiveJson::new(replays.into_iter().next().unwrap()),
            &query(),
            &facts,
        )
        .unwrap();
        assert_eq!(parsed.products.len(), 1);
    }

    #[test]
    fn query_builds_exact_geojson_request_and_query_bound_cursor() {
        let query = query();
        let facts = decode_search_facts(&query, RouteUse::OfflineFixture).unwrap();
        let cursor = wrap_cursor(facts.fingerprint(), 3).unwrap();
        let mut continued = query.clone();
        continued.cursor = Some(cursor);
        assert_eq!(
            decode_search_facts(&continued, RouteUse::OfflineFixture)
                .unwrap()
                .starting_number(),
            3
        );
        continued
            .filters
            .insert("cloud_cover_lte".to_owned(), json!(39));
        assert_eq!(
            decode_search_facts(&continued, RouteUse::OfflineFixture)
                .unwrap_err()
                .code(),
            UsgsM2mErrorCode::PaginationInvalid
        );

        let mut with_bbox = query.clone();
        with_bbox
            .aoi_geojson
            .as_mut()
            .expect("fixture query includes AOI")
            .insert("bbox".to_owned(), json!([110, 30, 111, 31]));
        let mut unclosed = query.clone();
        unclosed
            .aoi_geojson
            .as_mut()
            .expect("fixture query includes AOI")["coordinates"] =
            json!([[[110.0, 30.0], [111.0, 30.0], [111.0, 31.0], [110.0, 31.0]]]);
        let mut out_of_bounds = query.clone();
        out_of_bounds
            .aoi_geojson
            .as_mut()
            .expect("fixture query includes AOI")["coordinates"] =
            json!([[[181.0, 30.0], [181.0, 31.0], [180.0, 31.0], [181.0, 30.0]]]);
        let mut feature_wrapper = query.clone();
        feature_wrapper.aoi_geojson = serde_json::from_value(json!({
            "type": "Feature",
            "properties": {},
            "geometry": query.aoi_geojson
        }))
        .unwrap();

        for invalid in [with_bbox, unclosed, out_of_bounds, feature_wrapper] {
            assert_eq!(
                decode_search_facts(&invalid, RouteUse::OfflineFixture)
                    .unwrap_err()
                    .code(),
                UsgsM2mErrorCode::ConfigInvalid
            );
        }

        let malformed_filter = SensitiveJson::new(json!([{
            "id": "unsafe filter id",
            "fieldLabel": "Satellite",
            "fieldConfig": {"type": "Select"},
            "valueList": {"9": "9"}
        }]));
        assert_eq!(
            parse_satellite_filter(malformed_filter, "9")
                .unwrap_err()
                .endpoint(),
            UsgsM2mEndpointClass::DatasetFilters
        );
    }

    #[test]
    fn scene_mapping_preserves_authority_and_drops_urls_and_assets() {
        let (router, endpoint) = fixture_router(vec![page(
            vec![scene(
                "LC91230452026001LGN00",
                "LC09_L2SP_123045_20260101_20260103_02_T1",
                "2026/01/01",
                "l2sp",
            )],
            1,
            1,
            1,
        )]);
        let response = router.search_page_offline(&query(), &|| false).unwrap();
        response.validate().unwrap();
        assert_eq!(response.products.len(), 1);
        let product = &response.products[0];
        assert_eq!(product.source_kind, DataSourceKind::Landsat);
        assert!(product.assets.is_empty());
        assert!(product.payload.is_empty());
        assert_eq!(product.properties["entity_id"], "LC91230452026001LGN00");
        assert_eq!(
            product.properties["landsat_scene_id"],
            "LC91230452026001LGN00"
        );
        assert_eq!(product.properties["processing_level"], "l2sp");
        assert_eq!(product.properties["wrs_path"], "123");
        assert_eq!(product.properties["wrs_row"], "045");
        assert_eq!(product.properties["browse_available"], true);
        let serialized = serde_json::to_string(product).unwrap();
        assert!(!serialized.contains("browse.invalid"));
        assert!(!serialized.contains("options"));
        assert_eq!(endpoint.filters_calls.load(Ordering::SeqCst), 1);
        assert_eq!(endpoint.scene_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn full_metadata_ignores_bounded_non_authoritative_nulls_but_closes_authority() {
        let mut with_optional = scene(
            "LC91230452026001LGN00",
            "LC09_L2SP_123045_20260101_20260103_02_T1",
            "2026/01/01",
            "l2sp",
        );
        let metadata = with_optional["metadata"].as_array_mut().unwrap();
        for index in 0..160 {
            metadata.push(json!({
                "fieldName": format!("Optional field {index}"),
                "value": if index % 2 == 0 { Value::Null } else { json!(index) }
            }));
        }
        assert!(metadata.len() > 128 && metadata.len() <= MAX_METADATA_ITEMS);
        let (router, _) = fixture_router(vec![page(vec![with_optional], 1, 1, 1)]);
        assert_eq!(
            router
                .search_page_offline(&query(), &|| false)
                .unwrap()
                .products
                .len(),
            1
        );

        let mut missing_authority = scene(
            "LC91230452026001LGN00",
            "LC09_L2SP_123045_20260101_20260103_02_T1",
            "2026/01/01",
            "l2sp",
        );
        missing_authority["metadata"][0]["value"] = Value::Null;
        let (router, _) = fixture_router(vec![page(vec![missing_authority], 1, 1, 1)]);
        let error = router.search_page_offline(&query(), &|| false).unwrap_err();
        assert_eq!(error.code(), UsgsM2mErrorCode::ResponseInvalid);
        assert_eq!(error.detail(), Some("metadata_authoritative_value"));
    }

    #[test]
    fn satellite_authority_accepts_only_closed_numeric_mission_values() {
        let mut numeric_satellite = scene(
            "LC91230452026001LGN00",
            "LC09_L2SP_123045_20260101_20260103_02_T1",
            "2026/01/01",
            "l2sp",
        );
        numeric_satellite["metadata"][7]["value"] = json!(9);
        let (router, _) = fixture_router(vec![page(vec![numeric_satellite], 1, 1, 1)]);
        let response = router.search_page_offline(&query(), &|| false).unwrap();
        assert_eq!(response.products[0].properties["spacecraft"], "landsat_9");

        for invalid in [
            json!(0),
            json!(7),
            json!(10),
            json!(9.5),
            Value::Bool(true),
            Value::Null,
        ] {
            let mut scene = scene(
                "LC91230452026001LGN00",
                "LC09_L2SP_123045_20260101_20260103_02_T1",
                "2026/01/01",
                "l2sp",
            );
            scene["metadata"][7]["value"] = invalid;
            let (router, _) = fixture_router(vec![page(vec![scene], 1, 1, 1)]);
            let error = router.search_page_offline(&query(), &|| false).unwrap_err();
            assert_eq!(error.code(), UsgsM2mErrorCode::ResponseInvalid);
            assert_eq!(error.detail(), Some("metadata_authoritative_value"));
        }

        let mut numeric_product_id = scene(
            "LC91230452026001LGN00",
            "LC09_L2SP_123045_20260101_20260103_02_T1",
            "2026/01/01",
            "l2sp",
        );
        numeric_product_id["metadata"][0]["value"] = json!(9);
        let (router, _) = fixture_router(vec![page(vec![numeric_product_id], 1, 1, 1)]);
        assert_eq!(
            router
                .search_page_offline(&query(), &|| false)
                .unwrap_err()
                .detail(),
            Some("metadata_authoritative_value")
        );
    }

    #[test]
    fn authoritative_text_normalizes_transport_padding_and_omits_blank_optional_scene_id() {
        let mut padded = scene(
            "LC91230452026001LGN00",
            "LC09_L2SP_123045_20260101_20260103_02_T1",
            "2026/01/01",
            "l2sp",
        );
        for item in padded["metadata"].as_array_mut().unwrap() {
            if let Some(value) = item["value"].as_str() {
                item["value"] = json!(format!("  {value}  "));
            }
        }
        let (router, _) = fixture_router(vec![page(vec![padded], 1, 1, 1)]);
        let response = router.search_page_offline(&query(), &|| false).unwrap();
        let product = &response.products[0];
        assert_eq!(
            product.display_name,
            "LC09_L2SP_123045_20260101_20260103_02_T1"
        );
        assert_eq!(
            product.properties["landsat_scene_id"],
            "LC91230452026001LGN00"
        );
        assert_eq!(product.properties["spacecraft"], "landsat_9");

        let mut blank_optional = scene(
            "LC91230452026001LGN00",
            "LC09_L2SP_123045_20260101_20260103_02_T1",
            "2026/01/01",
            "l2sp",
        );
        blank_optional["metadata"][1]["value"] = json!("   ");
        let (router, _) = fixture_router(vec![page(vec![blank_optional], 1, 1, 1)]);
        let response = router.search_page_offline(&query(), &|| false).unwrap();
        assert!(response.products[0]
            .properties
            .get("landsat_scene_id")
            .is_none());

        for invalid_product_id in [json!("   "), json!("LC09\nINVALID")] {
            let mut invalid = scene(
                "LC91230452026001LGN00",
                "LC09_L2SP_123045_20260101_20260103_02_T1",
                "2026/01/01",
                "l2sp",
            );
            invalid["metadata"][0]["value"] = invalid_product_id;
            let (router, _) = fixture_router(vec![page(vec![invalid], 1, 1, 1)]);
            assert_eq!(
                router
                    .search_page_offline(&query(), &|| false)
                    .unwrap_err()
                    .detail(),
                Some("metadata_authoritative_value")
            );
        }
    }

    #[test]
    fn bounded_pagination_deduplicates_exact_records_and_rejects_conflicts() {
        let first = scene(
            "LC91230452026001LGN00",
            "LC09_L2SP_123045_20260101_20260103_02_T1",
            "2026/01/01",
            "l2sp",
        );
        let second = scene(
            "LC91230452026002LGN00",
            "LC09_L2SR_123045_20260102_20260104_02_T1",
            "2026/01/02",
            "l2sr",
        );
        let (router, endpoint) = fixture_router(vec![
            page(vec![first.clone(), first], 3, 1, 3),
            page(vec![second], 3, 3, 3),
        ]);
        let response = router.search_catalog_offline(&query(), &|| false).unwrap();
        response.validate().unwrap();
        assert_eq!(response.products.len(), 2);
        assert_eq!(response.stats.fetched_record_count, 3);
        assert_eq!(response.stats.exact_duplicate_record_count, 1);
        assert_eq!(endpoint.filters_calls.load(Ordering::SeqCst), 1);
        assert_eq!(endpoint.scene_calls.load(Ordering::SeqCst), 2);

        let original = scene(
            "LC91230452026001LGN00",
            "LC09_L2SP_123045_20260101_20260103_02_T1",
            "2026/01/01",
            "l2sp",
        );
        let mut conflict = original.clone();
        conflict["cloudCover"] = json!(13.5);
        let (router, _) = fixture_router(vec![page(vec![original, conflict], 2, 1, 2)]);
        assert_eq!(
            router
                .search_catalog_offline(&query(), &|| false)
                .unwrap_err()
                .code(),
            UsgsM2mErrorCode::SceneConflict
        );
    }

    #[test]
    fn local_filtering_keeps_truthful_stats_and_conflicts_closed_across_pages() {
        let mut filtered_query = query();
        filtered_query.result_limit = Some(3);
        filtered_query
            .filters
            .insert("processing_levels".to_owned(), json!(["l2sp"]));
        let selected = scene(
            "LC91230452026001LGN00",
            "LC09_L2SP_123045_20260101_20260103_02_T1",
            "2026/01/01",
            "l2sp",
        );
        let excluded_level = scene(
            "LC91230452026002LGN00",
            "LC09_L2SR_123045_20260102_20260104_02_T1",
            "2026/01/02",
            "l2sr",
        );
        let excluded_end = scene(
            "LC91230452026004LGN00",
            "LC09_L2SP_123045_20260104_20260106_02_T1",
            "2026/01/04",
            "l2sp",
        );
        let (router, _) = fixture_router(vec![page(
            vec![selected, excluded_level, excluded_end],
            3,
            1,
            3,
        )]);
        let response = router
            .search_catalog_offline(&filtered_query, &|| false)
            .unwrap();
        response.validate().unwrap();
        assert_eq!(response.products.len(), 1);
        assert_eq!(response.stats.fetched_record_count, 3);
        assert_eq!(response.stats.mapped_record_count, 3);
        assert_eq!(response.stats.unique_provider_product_count, 3);
        assert_eq!(response.stats.returned_count, 1);

        let first = scene(
            "LC91230452026002LGN00",
            "LC09_L2SR_123045_20260102_20260104_02_T1",
            "2026/01/02",
            "l2sr",
        );
        let mut conflict = first.clone();
        conflict["cloudCover"] = json!(13.5);
        let (router, _) = fixture_router(vec![
            page(vec![first], 2, 1, 2),
            page(vec![conflict], 2, 2, 2),
        ]);
        assert_eq!(
            router
                .search_catalog_offline(&filtered_query, &|| false)
                .unwrap_err()
                .code(),
            UsgsM2mErrorCode::SceneConflict
        );
    }

    /// #22 B slice 3(a): `available` generated scenes in 100-record pages;
    /// exhaustion follows `totalHits` as the M2M envelope states it.
    fn generated_pages(available: u64, pages: u64) -> Vec<Value> {
        (0..pages)
            .map(|page_index| {
                let start = page_index * 100 + 1;
                let last = (start + 99).min(available);
                let scenes = (start..=last)
                    .map(|index| {
                        scene(
                            &format!("LC9{index:019}"),
                            &format!("LC09_L2SP_{index:06}_20260101_20260103_02_T1"),
                            "2026/01/01",
                            "l2sp",
                        )
                    })
                    .collect();
                page(scenes, available, start, last + 1)
            })
            .collect()
    }

    #[test]
    fn ten_thousand_target_keeps_n_and_reports_exhaustion_truthfully() {
        for requested in [10_000, 10_001] {
            let mut capped = query();
            capped.result_limit = Some(requested);
            let (router, endpoint) = fixture_router(generated_pages(10_001, 100));
            let response = router.search_catalog_offline(&capped, &|| false).unwrap();
            response.validate().unwrap();
            assert_eq!(endpoint.scene_calls.load(Ordering::SeqCst), 100);
            assert_eq!(response.stats.requested_limit, Some(10_000));
            assert_eq!(response.products.len(), 10_000);
            assert_eq!(response.stats.matched_count, Some(10_001));
            assert!(!response.stats.remote_exhausted);
        }
        let mut exact = query();
        exact.result_limit = Some(10_000);
        let (router, _) = fixture_router(generated_pages(10_000, 100));
        let response = router.search_catalog_offline(&exact, &|| false).unwrap();
        assert_eq!(response.products.len(), 10_000);
        assert!(response.stats.remote_exhausted);
    }

    #[test]
    fn pagination_limits_and_non_progress_fail_closed() {
        let first = scene(
            "LC91230452026001LGN00",
            "LC09_L2SP_123045_20260101_20260103_02_T1",
            "2026/01/01",
            "l2sp",
        );
        let (router, _) = fixture_router(vec![page(vec![first.clone()], 2, 1, 1)]);
        assert_eq!(
            router
                .search_catalog_offline(&query(), &|| false)
                .unwrap_err()
                .code(),
            UsgsM2mErrorCode::PaginationInvalid
        );

        let (router, _) = fixture_router(vec![page(vec![first], MAX_SCENES + 1, 1, 2)]);
        assert_eq!(
            router
                .search_catalog_offline(&query(), &|| false)
                .unwrap_err()
                .code(),
            UsgsM2mErrorCode::SceneLimitExceeded
        );

        let mut one_per_page = query();
        one_per_page.result_limit = None;
        let responses = (1..=MAX_PAGES)
            .map(|index| {
                page(
                    vec![scene(
                        &format!("LC9{index:019}"),
                        &format!("LC09_L2SP_{index:06}_20260101_20260103_02_T1"),
                        "2026/01/01",
                        "l2sp",
                    )],
                    (MAX_PAGES + 1) as u64,
                    index as u64,
                    index as u64 + 1,
                )
            })
            .collect();
        let (router, endpoint) = fixture_router(responses);
        assert_eq!(
            router
                .search_catalog_offline(&one_per_page, &|| false)
                .unwrap_err()
                .code(),
            UsgsM2mErrorCode::SceneLimitExceeded
        );
        assert_eq!(endpoint.scene_calls.load(Ordering::SeqCst), MAX_PAGES);
    }

    #[test]
    fn malformed_counters_and_unaccepted_production_route_fail_closed() {
        let (empty_router, _) = fixture_router(vec![page(Vec::new(), 0, 1, 0)]);
        let empty = empty_router
            .search_page_offline(&query(), &|| false)
            .unwrap();
        empty.validate().unwrap();
        assert!(empty.products.is_empty());

        let (invalid_empty_router, _) = fixture_router(vec![page(Vec::new(), 0, 1, 2)]);
        assert_eq!(
            invalid_empty_router
                .search_page_offline(&query(), &|| false)
                .unwrap_err()
                .code(),
            UsgsM2mErrorCode::PaginationInvalid
        );

        let (client, endpoint) = fixture_client(vec![page(Vec::new(), 1, 1, 1)]);
        let router = UsgsM2mSceneSearchRouter {
            client,
            live_acceptance: None,
            admission: SceneRouteAdmission::Standard,
        };
        assert_eq!(
            router
                .search_page_offline(&query(), &|| false)
                .unwrap_err()
                .code(),
            UsgsM2mErrorCode::PaginationInvalid
        );
        assert!(!router.production_enabled());
        assert_eq!(router.state(), UsgsM2mSceneSearchState::OfflineImplemented);
        assert_eq!(
            router
                .search_catalog(&query(), &|| false)
                .unwrap_err()
                .endpoint(),
            UsgsM2mEndpointClass::SceneSearchRouter
        );
        assert_eq!(endpoint.scene_calls.load(Ordering::SeqCst), 1);
    }
}
