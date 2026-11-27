use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use chrono::{SecondsFormat, Utc};
use insar_core::{ContractError, ProviderResult, ProxyPolicy};
use insar_download::SharedTaskStore;
use insar_provider::boundary::{
    china_province_for_text, default_boundary_attribution, normalize_polygonal_geometry,
    ChinaAdminOption, ChinaAdminOptions, ChinaBoundaryProvider, DataVBoundaryProvider,
    GlobalBoundaryProvider, NominatimBoundaryProvider, ProviderBoundaryCandidate,
    ProviderBoundaryFeature,
};
use insar_provider::geoboundaries::{GeoBoundariesProvider, GeoBoundaryCatalog};
use insar_store::{canonical_utc_identity_timestamp, StoreError, StoredAdministrativeBoundary};
use serde_json::{Map, Value};

use crate::admin_boundaries::{
    is_unrestricted, local_options, taiwan_identity_for_selection, taiwan_options,
    AdminBoundaryBbox, AdminBoundaryOption, AdminBoundaryOptionsRequest,
    AdminBoundaryOptionsResult, AdminBoundaryResolveRequest, AdminBoundaryResult,
    AdminBoundarySearchRequest, AdminBoundarySearchResult, TaiwanBoundaryIdentity,
};
use crate::geo_boundaries::{GeoBoundaryCache, UnavailableGeoCatalog};
use crate::ApplicationServiceError;

const MAX_RESULTS: usize = 20;
const MAX_CACHE_ENTRIES: usize = 64;
const CACHE_TTL: Duration = Duration::from_secs(5 * 60);
const NOMINATIM_MIN_INTERVAL: Duration = Duration::from_secs(1);

struct CacheEntry {
    inserted_at: Instant,
    result: AdminBoundarySearchResult,
}

/// Application-owned boundary orchestration. Provider adapters never cross
/// into the UI; successful resolved polygons may be persisted through the
/// Store lease supplied by ApplicationService.
pub(crate) struct BoundaryService {
    providers: Mutex<BoundaryProviders>,
    cache: Mutex<BTreeMap<String, CacheEntry>>,
    geo_cache: GeoBoundaryCache,
    last_nominatim_request: Mutex<Option<Instant>>,
    rate_limit_enabled: bool,
    route_policy: Mutex<ProxyPolicy>,
    tianditu_token: Mutex<String>,
}

struct BoundaryProviders {
    /// The route production providers were built on; `None` for injected
    /// providers, which are never rebuilt.
    route: Option<ProxyPolicy>,
    global: Arc<dyn GlobalBoundaryProvider>,
    china: Arc<dyn ChinaBoundaryProvider>,
    geo: Arc<dyn GeoBoundaryCatalog>,
}

impl BoundaryProviders {
    fn production(route: ProxyPolicy) -> Self {
        let global: Arc<dyn GlobalBoundaryProvider> =
            match NominatimBoundaryProvider::production_with_route(route) {
                Ok(provider) => Arc::new(provider),
                Err(error) => Arc::new(UnavailableGlobalProvider { error }),
            };
        let china: Arc<dyn ChinaBoundaryProvider> =
            match DataVBoundaryProvider::production_with_route(route) {
                Ok(provider) => Arc::new(provider),
                Err(error) => Arc::new(UnavailableChinaProvider { error }),
            };
        let geo: Arc<dyn GeoBoundaryCatalog> =
            match GeoBoundariesProvider::production_with_route(route) {
                Ok(provider) => Arc::new(provider),
                Err(error) => Arc::new(UnavailableGeoCatalog::from_error(error)),
            };
        Self {
            route: Some(route),
            global,
            china,
            geo,
        }
    }
}

impl BoundaryService {
    /// Built before the saved settings are read; the first request rebuilds
    /// the providers on the saved route (`use_network_route`).
    pub(crate) fn production() -> Self {
        Self::from_providers(BoundaryProviders::production(ProxyPolicy::System), true)
    }

    /// Rebuild production providers when the saved route changed.
    pub(crate) fn use_network_route(&self, route: ProxyPolicy) {
        {
            let mut policy = self
                .route_policy
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *policy = route;
        }
        let mut providers = self
            .providers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if providers.route.is_some_and(|current| current != route) {
            *providers = BoundaryProviders::production(route);
        }
    }

    pub(crate) fn use_network_settings(&self, route: ProxyPolicy, tianditu_token: &str) {
        self.use_network_route(route);
        let next_token = tianditu_token.trim().to_owned();
        let mut token = self
            .tianditu_token
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *token != next_token {
            *token = next_token;
            if let Ok(mut cache) = self.cache.lock() {
                cache.clear();
            }
        }
    }

    fn current_route_policy(&self) -> ProxyPolicy {
        *self
            .route_policy
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn current_tianditu_token(&self) -> String {
        self.tianditu_token
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn global_provider(&self) -> Arc<dyn GlobalBoundaryProvider> {
        Arc::clone(
            &self
                .providers
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .global,
        )
    }

    fn china_provider(&self) -> Arc<dyn ChinaBoundaryProvider> {
        Arc::clone(
            &self
                .providers
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .china,
        )
    }

    pub(crate) fn geo_catalog(&self) -> Arc<dyn GeoBoundaryCatalog> {
        Arc::clone(
            &self
                .providers
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .geo,
        )
    }

    pub(crate) fn geo_cache(&self) -> &GeoBoundaryCache {
        &self.geo_cache
    }

    #[doc(hidden)]
    pub(crate) fn with_providers(
        global_provider: Arc<dyn GlobalBoundaryProvider>,
        china_provider: Arc<dyn ChinaBoundaryProvider>,
    ) -> Self {
        Self::new(global_provider, china_provider, false)
    }

    #[doc(hidden)]
    pub(crate) fn with_geo_catalog(
        global_provider: Arc<dyn GlobalBoundaryProvider>,
        china_provider: Arc<dyn ChinaBoundaryProvider>,
        geo_catalog: Arc<dyn GeoBoundaryCatalog>,
    ) -> Self {
        Self::from_providers(
            BoundaryProviders {
                route: None,
                global: global_provider,
                china: china_provider,
                geo: geo_catalog,
            },
            false,
        )
    }

    fn new(
        global_provider: Arc<dyn GlobalBoundaryProvider>,
        china_provider: Arc<dyn ChinaBoundaryProvider>,
        rate_limit_enabled: bool,
    ) -> Self {
        Self::from_providers(
            BoundaryProviders {
                route: None,
                global: global_provider,
                china: china_provider,
                geo: Arc::new(UnavailableGeoCatalog::not_configured()),
            },
            rate_limit_enabled,
        )
    }

    fn from_providers(providers: BoundaryProviders, rate_limit_enabled: bool) -> Self {
        Self {
            providers: Mutex::new(providers),
            cache: Mutex::new(BTreeMap::new()),
            geo_cache: GeoBoundaryCache::default(),
            last_nominatim_request: Mutex::new(None),
            rate_limit_enabled,
            route_policy: Mutex::new(ProxyPolicy::System),
            tianditu_token: Mutex::new(String::new()),
        }
    }

    pub(crate) fn search(
        &self,
        request: &AdminBoundarySearchRequest,
        store: Option<&SharedTaskStore>,
    ) -> Result<AdminBoundarySearchResult, ApplicationServiceError> {
        let query = request.query.trim();
        let province = request.province.trim();
        let city = request.city.trim();
        let district = request.district.trim();
        let limit = request.limit.unwrap_or(8).clamp(1, MAX_RESULTS);
        // Free text that names a province-level unit of China (including
        // Hong Kong, Macao and Taiwan) or a recognized colloquial alias takes
        // the China/Taiwan or normalized global route cleanly while preserving
        // `request.query` in the returned envelope.
        let original_query = query;
        let alias_target = (!query.is_empty()
            && is_national(province)
            && is_unrestricted(city)
            && is_unrestricted(district))
        .then(|| resolve_free_text_admin_alias(query))
        .flatten();
        let china_text = (!query.is_empty()
            && is_national(province)
            && is_unrestricted(city)
            && is_unrestricted(district))
        .then(|| match alias_target {
            Some(FreeTextAliasTarget::ChinaProvince(name)) => Some(name),
            Some(FreeTextAliasTarget::Taiwan {
                province: tw_prov, ..
            }) => Some(tw_prov),
            Some(FreeTextAliasTarget::ChinaCity { .. }) | Some(FreeTextAliasTarget::Global(_)) => {
                None
            }
            None => china_province_for_text(query),
        })
        .flatten();
        let (province, city, query, selected_object_id) = match (china_text, alias_target) {
            (Some(name), Some(FreeTextAliasTarget::Taiwan { city: tw_city, .. }))
                if is_taiwan_selector(name) =>
            {
                (
                    name,
                    tw_city,
                    "",
                    Some(taiwan_identity_for_selection(name, tw_city)?.provider_object_id),
                )
            }
            (Some(name), _) if is_taiwan_selector(name) => (
                name,
                city,
                "",
                Some(taiwan_identity_for_selection(name, "")?.provider_object_id),
            ),
            (Some(name), _) => (name, city, "", None),
            (
                None,
                Some(FreeTextAliasTarget::ChinaCity {
                    province: c_prov,
                    city: c_city,
                    adcode,
                }),
            ) => (c_prov, c_city, "", Some(adcode.to_owned())),
            (None, Some(FreeTextAliasTarget::Global(global_query))) => (
                province,
                city,
                global_query,
                request.provider_object_id.clone(),
            ),
            (None, _) if is_taiwan_selector(province) && !query.is_empty() => {
                if let Some(FreeTextAliasTarget::Taiwan {
                    province: tw_prov,
                    city: tw_alias_city,
                }) = resolve_free_text_admin_alias(query)
                {
                    let tw_id =
                        taiwan_identity_for_selection(tw_prov, tw_alias_city)?.provider_object_id;
                    (tw_prov, tw_alias_city, "", Some(tw_id))
                } else {
                    let matched_city = match_taiwan_city_name(query).unwrap_or(city);
                    let tw_city = if is_unrestricted(matched_city) {
                        ""
                    } else {
                        matched_city
                    };
                    let tw_id =
                        taiwan_identity_for_selection(province, tw_city)?.provider_object_id;
                    (province, matched_city, "", Some(tw_id))
                }
            }
            (None, _) => (province, city, query, request.provider_object_id.clone()),
        };
        let national_selector = is_national(province)
            && is_unrestricted(city)
            && is_unrestricted(district)
            && query.is_empty();

        let route = if !query.is_empty()
            && is_national(province)
            && is_unrestricted(city)
            && is_unrestricted(district)
        {
            SearchRoute::Global {
                query: query.to_owned(),
            }
        } else if is_taiwan_selector(province) {
            SearchRoute::Taiwan {
                identity: validated_taiwan_identity(province, city, selected_object_id.as_deref())?,
            }
        } else if national_selector
            || !query.is_empty()
            || !is_unrestricted(province)
            || !is_unrestricted(city)
            || !is_unrestricted(district)
        {
            SearchRoute::China
        } else {
            return Err(boundary_error(
                "BOUNDARY_QUERY_REQUIRED",
                "请输入地名或选择省/市/区后再加载行政边界",
                "empty_search_scope",
            ));
        };

        let selected_datav_adcode = if matches!(route, SearchRoute::China) {
            validated_datav_adcode(selected_object_id.as_deref())?
        } else {
            None
        };
        let selected_provider_object_id = match &route {
            SearchRoute::Taiwan { identity } => Some(identity.provider_object_id.as_str()),
            SearchRoute::China => selected_datav_adcode.as_deref(),
            SearchRoute::Global { .. } => None,
        };
        let cache_key = cache_key(
            &route,
            province,
            city,
            district,
            query,
            selected_provider_object_id,
            limit,
        );
        if let Some(mut result) = self.cached(&cache_key)? {
            set_cache_status(&mut result, "hit");
            // Different texts can share one China route ("香港", "Hong Kong").
            result.query = request.query.clone();
            return Ok(result);
        }

        if !matches!(route, SearchRoute::Taiwan { .. }) {
            if let (Some(store), Some(adcode)) = (store, selected_datav_adcode.as_deref()) {
                if let Some(result) = load_persistent_boundary(store, "datav", adcode)? {
                    let cached_result = AdminBoundarySearchResult {
                        ok: true,
                        query: original_query.to_owned(),
                        provider: "datav".to_owned(),
                        warning: None,
                        results: vec![result],
                    };
                    self.insert_cache(cache_key, cached_result.clone())?;
                    return Ok(cached_result);
                }
            }
        }
        if let (Some(store), SearchRoute::Taiwan { identity }) = (store, &route) {
            if let Some((cached_provider, warning, result)) =
                self.load_cached_taiwan_boundary(store, identity)?
            {
                let cached_result = AdminBoundarySearchResult {
                    ok: true,
                    query: original_query.to_owned(),
                    provider: cached_provider.to_owned(),
                    warning,
                    results: vec![result],
                };
                self.insert_cache(cache_key, cached_result.clone())?;
                return Ok(cached_result);
            }
        }

        let (provider, warning, results) = match &route {
            SearchRoute::Global { query } => {
                self.wait_for_nominatim()?;
                let mut candidates = normalize_candidates(
                    "nominatim",
                    self.global_provider()
                        .discover(query, limit)
                        .map_err(ApplicationServiceError::Provider)?,
                    "miss",
                )?;
                let mut global_provider_name: &'static str = "nominatim";
                let mut global_warning: Option<String> = None;
                if candidates.is_empty() {
                    if let Some(typo_target) = fuzzy_typo_admin_target(original_query) {
                        match typo_target {
                            FreeTextAliasTarget::Taiwan {
                                province: tw_prov,
                                city: tw_city,
                            } => {
                                if let Ok(tw_id) = taiwan_identity_for_selection(tw_prov, tw_city) {
                                    let (tw_prov_name, tw_warn, tw_res) =
                                        self.search_taiwan_domestic(&tw_id, store)?;
                                    global_provider_name = tw_prov_name;
                                    global_warning = tw_warn;
                                    candidates = tw_res;
                                }
                            }
                            FreeTextAliasTarget::ChinaProvince(prov_name) => {
                                if let Ok(features) = self
                                    .china_provider()
                                    .search(prov_name, "全部", "全部", "", limit)
                                {
                                    if let Ok(res) = normalize_datav_features(features, None) {
                                        persist_boundaries(store, &res)?;
                                        global_provider_name = "datav";
                                        candidates = res;
                                    }
                                }
                            }
                            FreeTextAliasTarget::ChinaCity { adcode, .. } => {
                                if let Ok(feature) =
                                    self.china_provider().resolve_exact_adcode(adcode)
                                {
                                    if let Ok(res) =
                                        normalize_datav_features(vec![feature], Some(adcode))
                                    {
                                        persist_boundaries(store, &res)?;
                                        global_provider_name = "datav";
                                        candidates = res;
                                    }
                                }
                            }
                            FreeTextAliasTarget::Global(corrected_query) => {
                                if corrected_query != *query {
                                    self.wait_for_nominatim()?;
                                    candidates = normalize_candidates(
                                        "nominatim",
                                        self.global_provider()
                                            .discover(corrected_query, limit)
                                            .map_err(ApplicationServiceError::Provider)?,
                                        "miss",
                                    )?;
                                }
                            }
                        }
                    }
                }
                (global_provider_name, global_warning, candidates)
            }
            SearchRoute::Taiwan { identity } => self.search_taiwan_domestic(identity, store)?,
            SearchRoute::China => {
                let datav = selected_datav_adcode
                    .as_deref()
                    .map_or_else(
                        || {
                            self.china_provider()
                                .search(province, city, district, query, limit)
                        },
                        |adcode| {
                            self.china_provider()
                                .resolve_exact_adcode(adcode)
                                .map(|feature| vec![feature])
                        },
                    )
                    .map_err(ApplicationServiceError::Provider)
                    .and_then(|features| {
                        normalize_datav_features(features, selected_datav_adcode.as_deref())
                    });
                match datav {
                    Ok(results) => {
                        persist_boundaries(store, &results)?;
                        ("datav", None, results)
                    }
                    Err(err) if is_taiwan_selector(province) => return Err(err),
                    Err(_) => {
                        self.wait_for_nominatim()?;
                        let fallback_query = china_fallback_query(province, city, district, query);
                        let candidates = normalize_candidates(
                            "nominatim",
                            self.global_provider()
                                .discover_china(&fallback_query, limit)
                                .map_err(ApplicationServiceError::Provider)?,
                            "miss",
                        )?;
                        (
                            "nominatim",
                            Some(
                                "DataV 未返回可用的真实边界，已切换到 OSM 候选；请选择并预览后再设置为 AOI。"
                                    .to_owned(),
                            ),
                            candidates,
                        )
                    }
                }
            }
        };

        let mut result = AdminBoundarySearchResult {
            ok: true,
            query: request.query.clone(),
            provider: provider.to_owned(),
            warning,
            results,
        };
        let cache_search_result = !matches!(&route, SearchRoute::China) || provider == "datav";
        if cache_search_result {
            self.insert_cache(cache_key, result.clone())?;
        }
        set_cache_status(&mut result, "miss");
        Ok(result)
    }

    pub(crate) fn resolve(
        &self,
        request: &AdminBoundaryResolveRequest,
        store: Option<&SharedTaskStore>,
    ) -> Result<AdminBoundaryResult, ApplicationServiceError> {
        if request.provider.trim() != "nominatim" {
            // DataV candidates are bound through the China selector search
            // (an exact adcode), never through this resolve path.
            let fallback = (request.provider.trim() == "datav").then_some("china_selector");
            let mut context =
                Map::from_iter([("reason".to_owned(), Value::String("provider".to_owned()))]);
            if let Some(fallback) = fallback {
                context.insert("fallback".to_owned(), Value::String(fallback.to_owned()));
            }
            let message = if fallback.is_some() {
                "DataV 行政区请在中国行政区选择器中选择后加载，不经此处解析"
            } else {
                "当前行政边界候选来源不可解析"
            };
            return Err(ApplicationServiceError::Provider(
                ContractError::new("BOUNDARY_PROVIDER_IDENTITY_INVALID", message, context)
                    .expect("bounded boundary error context"),
            ));
        }
        let provider_object_id = request.provider_object_id.trim();
        if provider_object_id.is_empty() || provider_object_id.chars().count() > 128 {
            return Err(boundary_error(
                "BOUNDARY_PROVIDER_IDENTITY_INVALID",
                "行政边界候选身份无效",
                "provider_object_id",
            ));
        }
        let cache_key = format!("resolve|nominatim|{provider_object_id}");
        if let Some(mut cached) = self.cached(&cache_key)? {
            set_cache_status(&mut cached, "hit");
            return cached.results.into_iter().next().ok_or_else(|| {
                boundary_error(
                    "BOUNDARY_PROVIDER_RESPONSE_INVALID",
                    "行政边界解析缓存为空",
                    "empty_resolve_cache",
                )
            });
        }
        if let Some(store) = store {
            if let Some(cached) = load_persistent_boundary(store, "nominatim", provider_object_id)?
            {
                return Ok(cached);
            }
        }
        self.wait_for_nominatim()?;
        let feature = self
            .global_provider()
            .resolve(provider_object_id)
            .map_err(ApplicationServiceError::Provider)?;
        let result = normalize_feature("nominatim", feature, "miss")?;
        if result.provider_object_id != provider_object_id {
            return Err(boundary_error(
                "BOUNDARY_PROVIDER_IDENTITY_MISMATCH",
                "行政边界服务返回了不一致的候选身份",
                "provider_object_id",
            ));
        }
        persist_boundary(store, &result)?;
        self.insert_cache(
            cache_key,
            AdminBoundarySearchResult {
                ok: true,
                query: provider_object_id.to_owned(),
                provider: "nominatim".to_owned(),
                warning: None,
                results: vec![result.clone()],
            },
        )?;
        Ok(result)
    }

    pub(crate) fn options(
        &self,
        request: &AdminBoundaryOptionsRequest,
    ) -> Result<AdminBoundaryOptionsResult, ApplicationServiceError> {
        if is_taiwan_selector(request.province.trim()) {
            return taiwan_options(request);
        }
        match self
            .china_provider()
            .options(request.province.trim(), request.city.trim())
        {
            Ok(options) => options_result("datav", None, options),
            Err(error) => {
                let mut fallback = local_options(request)?;
                fallback.warning = Some(format!(
                    "DataV 行政区选项暂时不可用，已回退到本地选项索引：{}",
                    error.user_message
                ));
                Ok(fallback)
            }
        }
    }

    fn cached(
        &self,
        key: &str,
    ) -> Result<Option<AdminBoundarySearchResult>, ApplicationServiceError> {
        let mut cache = self
            .cache
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?;
        if let Some(entry) = cache.get(key) {
            if entry.inserted_at.elapsed() <= CACHE_TTL {
                return Ok(Some(entry.result.clone()));
            }
        }
        cache.remove(key);
        Ok(None)
    }

    fn insert_cache(
        &self,
        key: String,
        result: AdminBoundarySearchResult,
    ) -> Result<(), ApplicationServiceError> {
        let mut cache = self
            .cache
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?;
        if cache.len() >= MAX_CACHE_ENTRIES && !cache.contains_key(&key) {
            if let Some(oldest_key) = cache
                .iter()
                .min_by_key(|(_, entry)| entry.inserted_at)
                .map(|(key, _)| key.clone())
            {
                cache.remove(&oldest_key);
            }
        }
        cache.insert(
            key,
            CacheEntry {
                inserted_at: Instant::now(),
                result,
            },
        );
        Ok(())
    }

    fn wait_for_nominatim(&self) -> Result<(), ApplicationServiceError> {
        if !self.rate_limit_enabled {
            return Ok(());
        }
        reserve_rate_slot(&self.last_nominatim_request, NOMINATIM_MIN_INTERVAL)
    }

    #[allow(clippy::type_complexity)]
    fn load_cached_taiwan_boundary(
        &self,
        store: &SharedTaskStore,
        identity: &TaiwanBoundaryIdentity,
    ) -> Result<Option<(&'static str, Option<String>, AdminBoundaryResult)>, ApplicationServiceError>
    {
        if identity.provider == "tianditu" {
            let gb9 = format!("156{}", identity.provider_object_id);
            for key_id in [identity.provider_object_id.as_str(), gb9.as_str()] {
                if let Some(result) = load_persistent_boundary(store, "tianditu", key_id)? {
                    validate_taiwan_boundary(identity, &result)?;
                    return Ok(Some(("tianditu", None, result)));
                }
            }
            // If user still has no personal Tianditu token, serve cached DataV provincial
            // fallback domestically so offline/cold restarts never hit external OSM.
            if self.current_tianditu_token().trim().is_empty() {
                if let Some(result) = load_persistent_boundary(store, "datav", "710000")? {
                    let province_identity = taiwan_identity_for_selection("台湾省", "")?;
                    validate_taiwan_boundary(&province_identity, &result)?;
                    return Ok(Some((
                        "datav",
                        Some(TAIWAN_DATAV_FALLBACK_WARNING.to_owned()),
                        result,
                    )));
                }
            }
            return Ok(None);
        }
        if let Some(result) =
            load_persistent_boundary(store, "datav", &identity.provider_object_id)?
        {
            validate_taiwan_boundary(identity, &result)?;
            return Ok(Some(("datav", None, result)));
        }
        Ok(None)
    }

    fn search_taiwan_domestic(
        &self,
        identity: &TaiwanBoundaryIdentity,
        store: Option<&SharedTaskStore>,
    ) -> Result<(&'static str, Option<String>, Vec<AdminBoundaryResult>), ApplicationServiceError>
    {
        if identity.provider == "tianditu" {
            let token = self.current_tianditu_token();
            let keyword = identity
                .expected_provider_names
                .first()
                .map(String::as_str)
                .unwrap_or(identity.display_name.as_str());
            if !token.trim().is_empty() {
                if let Ok(feature) = insar_provider::boundary::fetch_tianditu_admin_boundary(
                    self.current_route_policy(),
                    keyword,
                    Some(&identity.provider_object_id),
                    token.trim(),
                ) {
                    if let Ok(result) = normalize_feature("tianditu", feature, "miss") {
                        if validate_taiwan_boundary(identity, &result).is_ok() {
                            let results = vec![result];
                            persist_boundaries(store, &results)?;
                            return Ok(("tianditu", None, results));
                        }
                    }
                }
            }
            // No personal Tianditu key or Tianditu call did not yield a fine boundary:
            // fall back strictly inside domestic China DataV ("710000"), never Nominatim/OSM.
            let province_identity = taiwan_identity_for_selection("台湾省", "")?;
            if let Some(store) = store {
                if let Some(cached) = load_persistent_boundary(store, "datav", "710000")? {
                    validate_taiwan_boundary(&province_identity, &cached)?;
                    return Ok((
                        "datav",
                        Some(TAIWAN_DATAV_FALLBACK_WARNING.to_owned()),
                        vec![cached],
                    ));
                }
            }
            let feature = self
                .china_provider()
                .resolve_exact_adcode("710000")
                .map_err(taiwan_first_load_error)?;
            let result = normalize_feature("datav", feature, "miss")?;
            validate_taiwan_boundary(&province_identity, &result)?;
            let results = vec![result];
            persist_boundaries(store, &results)?;
            return Ok((
                "datav",
                Some(TAIWAN_DATAV_FALLBACK_WARNING.to_owned()),
                results,
            ));
        }

        let feature = self
            .china_provider()
            .resolve_exact_adcode(&identity.provider_object_id)
            .map_err(taiwan_first_load_error)?;
        let result = normalize_feature("datav", feature, "miss")?;
        validate_taiwan_boundary(identity, &result)?;
        let results = vec![result];
        persist_boundaries(store, &results)?;
        Ok(("datav", None, results))
    }
}

const TAIWAN_DATAV_FALLBACK_WARNING: &str =
    "已使用国内阿里云 DataV 加载台湾省全域边界；如需台湾地市/区县级精细边界，请在设置中配置个人天地图密钥（不使用国外 OSM/Nominatim 数据源）";

fn normalize_datav_features(
    features: Vec<ProviderBoundaryFeature>,
    expected_adcode: Option<&str>,
) -> Result<Vec<AdminBoundaryResult>, ApplicationServiceError> {
    if features.is_empty() {
        return Err(boundary_error(
            "BOUNDARY_GEOMETRY_UNAVAILABLE",
            "DataV 未返回匹配的行政边界",
            "empty_datav_result",
        ));
    }
    let mut results = normalize_features("datav", features, "miss")?;
    if let Some(expected_adcode) = expected_adcode {
        results.retain(|result| result.provider_object_id == expected_adcode);
        if results.is_empty() {
            return Err(boundary_error(
                "BOUNDARY_PROVIDER_IDENTITY_MISMATCH",
                "DataV 未返回当前选择对应的精确行政边界",
                "provider_object_id",
            ));
        }
    }
    Ok(results)
}

fn validated_datav_adcode(
    provider_object_id: Option<&str>,
) -> Result<Option<String>, ApplicationServiceError> {
    let Some(provider_object_id) = provider_object_id else {
        return Ok(None);
    };
    if provider_object_id.trim() != provider_object_id
        || provider_object_id.len() != 6
        || !provider_object_id.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(boundary_error(
            "BOUNDARY_PROVIDER_IDENTITY_INVALID",
            "当前 DataV 行政区身份无效，请重新选择行政区",
            "provider_object_id",
        ));
    }
    Ok(Some(provider_object_id.to_owned()))
}

fn china_fallback_query(province: &str, city: &str, district: &str, query: &str) -> String {
    let mut terms = Vec::new();
    for value in [query, district, city, province] {
        let value = value.trim();
        if !is_unrestricted(value) && !terms.contains(&value) {
            terms.push(value);
        }
    }
    if terms.is_empty() {
        "中国".to_owned()
    } else {
        terms.join(" ")
    }
}

fn load_persistent_boundary(
    store: &SharedTaskStore,
    provider: &str,
    provider_object_id: &str,
) -> Result<Option<AdminBoundaryResult>, ApplicationServiceError> {
    let stored = match store
        .lock()
        .map_err(|_| ApplicationServiceError::StatePoisoned)?
        .load_administrative_boundary(provider, provider_object_id)
    {
        Ok(stored) => stored,
        Err(StoreError::InvalidSnapshot { .. } | StoreError::Corrupt(_)) => return Ok(None),
        Err(error) => return Err(ApplicationServiceError::from(error)),
    };
    stored
        .map(|stored| normalize_feature(provider, stored_feature(stored), "hit"))
        .transpose()
}

fn persist_boundaries(
    store: Option<&SharedTaskStore>,
    results: &[AdminBoundaryResult],
) -> Result<(), ApplicationServiceError> {
    for result in results {
        persist_boundary(store, result)?;
    }
    Ok(())
}

fn persist_boundary(
    store: Option<&SharedTaskStore>,
    result: &AdminBoundaryResult,
) -> Result<(), ApplicationServiceError> {
    let Some(store) = store else {
        return Ok(());
    };
    let Some(geometry) = result.geometry.as_ref().and_then(Value::as_object) else {
        return Ok(());
    };
    if !cacheable_identity(&result.provider, &result.provider_object_id) {
        return Ok(());
    }
    let timestamp = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let cached_at = canonical_utc_identity_timestamp(&timestamp).ok_or_else(|| {
        ApplicationServiceError::InvalidRequest {
            reason: "administrative boundary cache timestamp is invalid".to_owned(),
        }
    })?;
    store
        .lock()
        .map_err(|_| ApplicationServiceError::StatePoisoned)?
        .upsert_administrative_boundary(&StoredAdministrativeBoundary {
            provider: result.provider.clone(),
            provider_object_id: result.provider_object_id.clone(),
            name: result.name.clone(),
            display_name: result.display_name.clone(),
            country_code: result.country_code.clone(),
            admin_level: result.admin_level.clone(),
            geometry: geometry.clone(),
            bbox: [
                result.bbox.west,
                result.bbox.south,
                result.bbox.east,
                result.bbox.north,
            ],
            source: result.source.clone(),
            attribution: result.attribution.clone(),
            cached_at,
        })
        .map_err(ApplicationServiceError::from)
}

fn stored_feature(stored: StoredAdministrativeBoundary) -> ProviderBoundaryFeature {
    ProviderBoundaryFeature {
        provider_object_id: stored.provider_object_id,
        name: stored.name,
        display_name: stored.display_name,
        country_code: stored.country_code,
        admin_level: stored.admin_level,
        geometry: Value::Object(stored.geometry),
        geometry_kind: String::new(),
        bbox: stored.bbox,
        source: stored.source,
        attribution: stored.attribution,
    }
}

fn cacheable_identity(provider: &str, provider_object_id: &str) -> bool {
    match provider {
        "datav" => {
            provider_object_id.len() == 6
                && provider_object_id.bytes().all(|byte| byte.is_ascii_digit())
        }
        "tianditu" => {
            matches!(provider_object_id.len(), 6 | 9)
                && provider_object_id.bytes().all(|byte| byte.is_ascii_digit())
        }
        "nominatim" => provider_object_id
            .split_once(':')
            .is_some_and(|(kind, id)| {
                matches!(kind, "node" | "way" | "relation")
                    && id.parse::<u64>().ok().is_some_and(|value| value > 0)
            }),
        _ => false,
    }
}

fn reserve_rate_slot(
    last_request: &Mutex<Option<Instant>>,
    minimum_interval: Duration,
) -> Result<(), ApplicationServiceError> {
    // The lock deliberately covers both the wait and the timestamp update.
    // Releasing it before sleeping lets concurrent callers reserve the same
    // Nominatim slot and violates the provider's one-request-per-second policy.
    let mut guard = last_request
        .lock()
        .map_err(|_| ApplicationServiceError::StatePoisoned)?;
    if let Some(wait) = guard.and_then(|last| minimum_interval.checked_sub(last.elapsed())) {
        if !wait.is_zero() {
            thread::sleep(wait);
        }
    }
    *guard = Some(Instant::now());
    Ok(())
}

enum SearchRoute {
    Global { query: String },
    Taiwan { identity: TaiwanBoundaryIdentity },
    China,
}

fn cache_key(
    route: &SearchRoute,
    province: &str,
    city: &str,
    district: &str,
    query: &str,
    provider_object_id: Option<&str>,
    limit: usize,
) -> String {
    let route = match route {
        SearchRoute::Global { .. } => "global",
        SearchRoute::Taiwan { .. } => "taiwan",
        SearchRoute::China => "china",
    };
    format!(
        "{route}|{}|{}|{}|{}|{}|{limit}",
        normalized(province),
        normalized(city),
        normalized(district),
        normalized(query),
        provider_object_id.unwrap_or("")
    )
}

fn normalized(value: &str) -> String {
    value
        .trim()
        .chars()
        .filter(|character| !character.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

fn is_national(value: &str) -> bool {
    matches!(
        normalized(value).as_str(),
        "" | "全国" | "全部" | "不限" | "中国" | "china"
    )
}

fn is_taiwan_selector(value: &str) -> bool {
    matches!(normalized(value).as_str(), "台湾省" | "台湾")
}

fn validated_taiwan_identity(
    province: &str,
    city: &str,
    provider_object_id: Option<&str>,
) -> Result<TaiwanBoundaryIdentity, ApplicationServiceError> {
    let identity = taiwan_identity_for_selection(province, city)?;
    if provider_object_id != Some(identity.provider_object_id.as_str()) {
        return Err(boundary_error(
            "BOUNDARY_PROVIDER_IDENTITY_INVALID",
            "所选台湾行政区身份无效，请重新选择",
            "taiwan_provider_object_id",
        ));
    }
    Ok(identity)
}

fn validate_taiwan_boundary(
    identity: &TaiwanBoundaryIdentity,
    result: &AdminBoundaryResult,
) -> Result<(), ApplicationServiceError> {
    let name_matches = identity
        .expected_provider_names
        .iter()
        .any(|expected| normalized(expected) == normalized(&result.name));
    let id_matches = result.provider_object_id == identity.provider_object_id
        || (identity.provider == "tianditu"
            && result
                .provider_object_id
                .ends_with(&identity.provider_object_id));
    let country_ok = matches!(result.country_code.as_deref(), Some("CN") | Some("TW"));
    if result.provider != identity.provider || !id_matches || !country_ok || !name_matches {
        return Err(boundary_error(
            "BOUNDARY_PROVIDER_IDENTITY_MISMATCH",
            "未能加载所选台湾行政区的真实边界",
            "taiwan_exact_administrative_identity",
        ));
    }
    Ok(())
}

fn normalize_features(
    provider: &str,
    features: Vec<ProviderBoundaryFeature>,
    cache_status: &str,
) -> Result<Vec<AdminBoundaryResult>, ApplicationServiceError> {
    features
        .into_iter()
        .map(|feature| normalize_feature(provider, feature, cache_status))
        .collect()
}

fn normalize_candidates(
    provider: &str,
    candidates: Vec<ProviderBoundaryCandidate>,
    cache_status: &str,
) -> Result<Vec<AdminBoundaryResult>, ApplicationServiceError> {
    candidates
        .into_iter()
        .map(|candidate| normalize_candidate(provider, candidate, cache_status))
        .collect()
}

fn normalize_candidate(
    provider: &str,
    candidate: ProviderBoundaryCandidate,
    cache_status: &str,
) -> Result<AdminBoundaryResult, ApplicationServiceError> {
    let provider_object_id = candidate.provider_object_id.trim();
    let name = candidate.name.trim();
    let display_name = candidate.display_name.trim();
    if provider_object_id.is_empty() || name.is_empty() || display_name.is_empty() {
        return Err(boundary_error(
            "BOUNDARY_PROVIDER_RESPONSE_INVALID",
            "行政边界服务返回了不完整的候选标识",
            "missing_identity",
        ));
    }
    let [west, south, east, north] = candidate.bbox;
    if ![west, south, east, north]
        .iter()
        .all(|value| value.is_finite())
        || west < -180.0
        || east > 180.0
        || south < -90.0
        || north > 90.0
        || west >= east
        || south >= north
    {
        return Err(boundary_error(
            "BOUNDARY_PROVIDER_RESPONSE_INVALID",
            "行政边界服务返回了无效的候选范围",
            "bbox",
        ));
    }
    let boundary_id = format!("{provider}:{provider_object_id}");
    let admin_level = candidate.admin_level;
    Ok(AdminBoundaryResult {
        id: boundary_id.clone(),
        label: display_name.to_owned(),
        name: name.to_owned(),
        level: admin_level.clone().unwrap_or_default(),
        parent: None,
        boundary_id,
        provider: provider.to_owned(),
        provider_object_id: provider_object_id.to_owned(),
        display_name: display_name.to_owned(),
        country_code: candidate.country_code,
        admin_level,
        bbox: AdminBoundaryBbox {
            west,
            east,
            south,
            north,
            crs: "EPSG:4326".to_owned(),
        },
        geometry: None,
        geometry_kind: None,
        source: candidate.source,
        attribution: candidate
            .attribution
            .or_else(|| default_boundary_attribution(provider).map(str::to_owned)),
        cache_status: cache_status.to_owned(),
    })
}

pub(crate) fn normalize_feature(
    provider: &str,
    feature: ProviderBoundaryFeature,
    cache_status: &str,
) -> Result<AdminBoundaryResult, ApplicationServiceError> {
    let provider_object_id = feature.provider_object_id.trim();
    let name = feature.name.trim();
    let display_name = feature.display_name.trim();
    if provider_object_id.is_empty() || name.is_empty() || display_name.is_empty() {
        return Err(boundary_error(
            "BOUNDARY_PROVIDER_RESPONSE_INVALID",
            "行政边界服务返回了不完整的边界标识",
            "missing_identity",
        ));
    }
    let geometry = normalize_polygonal_geometry(feature.geometry).map_err(|reason| {
        boundary_error(
            "BOUNDARY_GEOMETRY_UNAVAILABLE",
            "行政边界服务未返回可预览的 Polygon/MultiPolygon 几何",
            &reason,
        )
    })?;
    let bbox = AdminBoundaryBbox {
        west: geometry.bbox[0],
        south: geometry.bbox[1],
        east: geometry.bbox[2],
        north: geometry.bbox[3],
        crs: "EPSG:4326".to_owned(),
    };
    let boundary_id = format!("{provider}:{provider_object_id}");
    let admin_level = feature.admin_level;
    let level = admin_level.clone().unwrap_or_default();
    Ok(AdminBoundaryResult {
        id: boundary_id.clone(),
        label: display_name.to_owned(),
        name: name.to_owned(),
        level,
        parent: None,
        boundary_id,
        provider: provider.to_owned(),
        provider_object_id: provider_object_id.to_owned(),
        display_name: display_name.to_owned(),
        country_code: feature.country_code,
        admin_level,
        bbox,
        geometry: Some(geometry.geometry),
        geometry_kind: Some(geometry.geometry_kind),
        source: feature.source,
        // Store rows cached before sources carried attribution have none.
        attribution: feature
            .attribution
            .or_else(|| default_boundary_attribution(provider).map(str::to_owned)),
        cache_status: cache_status.to_owned(),
    })
}

fn set_cache_status(result: &mut AdminBoundarySearchResult, status: &str) {
    for item in &mut result.results {
        item.cache_status = status.to_owned();
    }
}

fn options_result(
    provider: &str,
    warning: Option<String>,
    options: ChinaAdminOptions,
) -> Result<AdminBoundaryOptionsResult, ApplicationServiceError> {
    Ok(AdminBoundaryOptionsResult {
        ok: true,
        provider: provider.to_owned(),
        warning,
        provinces: normalize_admin_options(provider, options.provinces)?,
        cities: normalize_admin_options(provider, options.cities)?,
        districts: normalize_admin_options(provider, options.districts)?,
    })
}

fn normalize_admin_options(
    provider: &str,
    options: Vec<ChinaAdminOption>,
) -> Result<Vec<AdminBoundaryOption>, ApplicationServiceError> {
    let mut normalized = options
        .into_iter()
        .map(|option| {
            let name = option.name.trim();
            if name.is_empty() || name != option.name {
                return Err(boundary_error(
                    "BOUNDARY_PROVIDER_RESPONSE_INVALID",
                    "行政区选项缺少有效名称",
                    "option_name",
                ));
            }
            let provider_object_id = option
                .provider_object_id
                .as_deref()
                .map(|value| validated_datav_adcode(Some(value)))
                .transpose()?
                .flatten();
            if provider == "datav"
                && !matches!(name, "全部" | "不限")
                && provider_object_id.is_none()
            {
                return Err(boundary_error(
                    "BOUNDARY_PROVIDER_RESPONSE_INVALID",
                    "DataV 行政区选项缺少精确 adcode",
                    "option_provider_object_id",
                ));
            }
            Ok(AdminBoundaryOption {
                name: option.name,
                provider: provider_object_id.as_ref().map(|_| provider.to_owned()),
                provider_object_id,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if provider == "datav" {
        normalized.sort_by(|left, right| {
            let left_head = matches!(left.name.as_str(), "全国" | "全部" | "不限");
            let right_head = matches!(right.name.as_str(), "全国" | "全部" | "不限");
            right_head
                .cmp(&left_head)
                .then_with(|| left.provider_object_id.cmp(&right.provider_object_id))
                .then_with(|| left.name.cmp(&right.name))
        });
    }
    Ok(normalized)
}

/// Taiwan geometry is only reachable through Nominatim until the first
/// success is cached in the Store, so a first-load network failure must tell
/// the user what to change instead of leaving the selector blank. A rate
/// limit is not a network or proxy problem: waiting is the fix.
fn taiwan_first_load_error(error: ContractError) -> ApplicationServiceError {
    if !matches!(
        error.code.as_str(),
        "BOUNDARY_PROVIDER_UNAVAILABLE"
            | "BOUNDARY_PROVIDER_HTTP_ERROR"
            | "BOUNDARY_PROVIDER_RATE_LIMITED"
            | "BOUNDARY_PROVIDER_NON_DATA_RESPONSE"
            | "BOUNDARY_PROVIDER_RESPONSE_INVALID"
    ) {
        return ApplicationServiceError::Provider(error);
    }
    let (suggestion, message) = if error.code == "BOUNDARY_PROVIDER_RATE_LIMITED" {
        (
            "retry_later",
            "台湾行政区边界首次加载被服务限流：请稍后重试",
        )
    } else {
        (
            "check_network_or_switch_proxy",
            "台湾行政区边界首次加载失败：请检查网络或在设置里切换代理后重试",
        )
    };
    let context = Map::from_iter([
        (
            "reason".to_owned(),
            Value::String("taiwan_first_load".to_owned()),
        ),
        (
            "upstream_code".to_owned(),
            Value::String(error.code.clone()),
        ),
        (
            "suggestion".to_owned(),
            Value::String(suggestion.to_owned()),
        ),
    ]);
    ContractError::new("BOUNDARY_TAIWAN_FIRST_LOAD_FAILED", message, context)
        .map_or(ApplicationServiceError::Provider(error), |typed| {
            ApplicationServiceError::Provider(typed)
        })
}

fn boundary_error(code: &str, message: &str, reason: &str) -> ApplicationServiceError {
    let mut context = Map::new();
    context.insert("reason".to_owned(), Value::String(reason.to_owned()));
    let error = ContractError::new(code, message, context).unwrap_or_else(|_| ContractError {
        code: code.to_owned(),
        user_message: message.to_owned(),
        debug_context: Map::new(),
    });
    ApplicationServiceError::Provider(error)
}

struct UnavailableGlobalProvider {
    error: ContractError,
}

impl GlobalBoundaryProvider for UnavailableGlobalProvider {
    fn search(&self, _query: &str, _limit: usize) -> ProviderResult<Vec<ProviderBoundaryFeature>> {
        Err(self.error.clone())
    }

    fn discover(
        &self,
        _query: &str,
        _limit: usize,
    ) -> ProviderResult<Vec<ProviderBoundaryCandidate>> {
        Err(self.error.clone())
    }

    fn discover_china(
        &self,
        _query: &str,
        _limit: usize,
    ) -> ProviderResult<Vec<ProviderBoundaryCandidate>> {
        Err(self.error.clone())
    }

    fn resolve(&self, _provider_object_id: &str) -> ProviderResult<ProviderBoundaryFeature> {
        Err(self.error.clone())
    }
}

struct UnavailableChinaProvider {
    error: ContractError,
}

impl ChinaBoundaryProvider for UnavailableChinaProvider {
    fn options(&self, _province: &str, _city: &str) -> ProviderResult<ChinaAdminOptions> {
        Err(self.error.clone())
    }

    fn resolve_exact_adcode(&self, _adcode: &str) -> ProviderResult<ProviderBoundaryFeature> {
        Err(self.error.clone())
    }

    fn search(
        &self,
        _province: &str,
        _city: &str,
        _district: &str,
        _query: &str,
        _limit: usize,
    ) -> ProviderResult<Vec<ProviderBoundaryFeature>> {
        Err(self.error.clone())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FreeTextAliasTarget {
    Global(&'static str),
    ChinaProvince(&'static str),
    ChinaCity {
        province: &'static str,
        city: &'static str,
        adcode: &'static str,
    },
    Taiwan {
        province: &'static str,
        city: &'static str,
    },
}

fn clean_admin_search_text(raw: &str) -> String {
    raw.trim()
        .trim_matches(|ch: char| {
            matches!(
                ch,
                '\'' | '"'
                    | '“'
                    | '”'
                    | '‘'
                    | '’'
                    | '？'
                    | '?'
                    | '。'
                    | '.'
                    | '!'
                    | '！'
                    | '，'
                    | ','
            )
        })
        .trim()
        .to_owned()
}

fn match_taiwan_city_name(raw: &str) -> Option<&'static str> {
    let cleaned = clean_admin_search_text(raw);
    let lower = cleaned.to_ascii_lowercase();
    match lower.as_str() {
        "台北市" | "臺北市" | "台北" | "臺北" | "台北士" | "臺北士" | "台比市" | "taipei"
        | "taipei city" | "taibei" | "taibei city" => Some("台北市"),
        "新北市" | "新北" | "新北士" | "新比市" | "台北县" | "臺北縣" | "new taipei"
        | "new taipei city" | "xinbei" | "xinbei city" => Some("新北市"),
        "桃园市" | "桃園市" | "桃园" | "桃園" | "桃园县" | "桃園縣" | "桃园士" | "桃園士"
        | "taoyuan" | "taoyuan city" => Some("桃园市"),
        "台中市" | "臺中市" | "台中" | "臺中" | "台中县" | "臺中縣" | "台中士" | "臺中士"
        | "taichung" | "taichung city" | "taizhong" => Some("台中市"),
        "台南市" | "臺南市" | "台南" | "臺南" | "台南县" | "臺南縣" | "台南士" | "臺南士"
        | "tainan" | "tainan city" => Some("台南市"),
        "高雄市" | "高雄" | "高雄县" | "高雄縣" | "高雄士" | "kaohsiung" | "kaohsiung city"
        | "gaoxiong" => Some("高雄市"),
        "基隆市" | "基隆" | "keelung" | "keelung city" | "jilong" => Some("基隆市"),
        "新竹市" | "新竹" | "hsinchu" | "hsinchu city" => Some("新竹市"),
        "嘉义市" | "嘉義市" | "嘉义" | "嘉義" | "chiayi" | "chiayi city" => {
            Some("嘉义市")
        }
        "新竹县" | "新竹縣" | "hsinchu county" => Some("新竹县"),
        "苗栗县" | "苗栗縣" | "苗栗" | "miaoli" | "miaoli county" => Some("苗栗县"),
        "彰化县" | "彰化縣" | "彰化" | "changhua" | "changhua county" => Some("彰化县"),
        "南投县" | "南投縣" | "南投" | "nantou" | "nantou county" => Some("南投县"),
        "云林县" | "雲林縣" | "云林" | "雲林" | "yunlin" | "yunlin county" => {
            Some("云林县")
        }
        "嘉义县" | "嘉義縣" | "chiayi county" => Some("嘉义县"),
        "屏东县" | "屏東縣" | "屏东" | "屏東" | "pingtung" | "pingtung county" => {
            Some("屏东县")
        }
        "宜兰县" | "宜蘭縣" | "宜兰" | "宜蘭" | "yilan" | "yilan county" | "ilan"
        | "ilan county" => Some("宜兰县"),
        "花莲县" | "花蓮縣" | "花莲" | "花蓮" | "hualien" | "hualien county" | "hualian" => {
            Some("花莲县")
        }
        "台东县" | "臺東縣" | "台东" | "臺東" | "taitung" | "taitung county" | "taidong" => {
            Some("台东县")
        }
        "澎湖县" | "澎湖縣" | "澎湖" | "penghu" | "penghu county" | "pescadores" => {
            Some("澎湖县")
        }
        "金门县" | "金門縣" | "金门" | "金門" | "kinmen" | "kinmen county" | "jinmen"
        | "quemoy" => Some("金门县"),
        "连江县" | "連江縣" | "连江" | "連江" | "马祖" | "馬祖" | "lienchiang"
        | "lienchiang county" | "matsu" | "lianjiang taiwan" => Some("连江县"),
        _ => None,
    }
}

/// Domestic-only Taiwan ASCII typo matcher that runs BEFORE any global route selection so
/// typos such as `"taipai"` or `"taiwn"` NEVER hit Nominatim/OSM.
fn fuzzy_taiwan_typo_target(lower: &str) -> Option<FreeTextAliasTarget> {
    if !lower.is_ascii() {
        return None;
    }
    let q_len = lower.chars().count();
    if q_len < 5 {
        return None;
    }
    let taiwan_ascii_targets: &[(&str, &str)] = &[
        ("taiwan", ""),
        ("taipei", "台北市"),
        ("taibei", "台北市"),
        ("kaohsiung", "高雄市"),
        ("taichung", "台中市"),
        ("tainan", "台南市"),
        ("taoyuan", "桃园市"),
        ("keelung", "基隆市"),
        ("hsinchu", "新竹市"),
        ("chiayi", "嘉义市"),
        ("miaoli", "苗栗县"),
        ("changhua", "彰化县"),
        ("nantou", "南投县"),
        ("yunlin", "云林县"),
        ("pingtung", "屏东县"),
        ("yilan", "宜兰县"),
        ("hualien", "花莲县"),
        ("taitung", "台东县"),
        ("penghu", "澎湖县"),
        ("kinmen", "金门县"),
        ("matsu", "连江县"),
    ];
    for &(cand, city) in taiwan_ascii_targets {
        if char_edit_distance(lower, cand) == 1 {
            return Some(FreeTextAliasTarget::Taiwan {
                province: "台湾省",
                city,
            });
        }
    }
    None
}

fn resolve_free_text_admin_alias(raw: &str) -> Option<FreeTextAliasTarget> {
    let cleaned = clean_admin_search_text(raw);
    if cleaned.is_empty() {
        return None;
    }
    let lower = cleaned.to_lowercase();
    match lower.as_str() {
        // Taiwan & Taiwan 22 cities/counties ALWAYS stay domestic (zero Nominatim/OSM).
        "宝岛" | "宝岛台湾" | "台湾" | "台灣" | "臺灣" | "台湾省" | "台灣省" | "臺灣省"
        | "中华台北" | "台北省" | "台弯" | "台完" | "台弯省" | "台完省" | "taiwan"
        | "taiwan province" | "taiwan, china" | "taiwan china" | "formosa" => {
            Some(FreeTextAliasTarget::Taiwan {
                province: "台湾省",
                city: "",
            })
        }
        // Global colloquial aliases
        "小日本" | "日本国" | "霓虹国" | "扶桑" | "东瀛" => {
            Some(FreeTextAliasTarget::Global("日本"))
        }
        "小韩国" | "南韩" | "泡菜国" => Some(FreeTextAliasTarget::Global("韩国")),
        "小美国" | "美帝" | "漂亮国" | "山姆大叔" => {
            Some(FreeTextAliasTarget::Global("美国"))
        }
        "腐国" | "大英" => Some(FreeTextAliasTarget::Global("英国")),
        "战斗民族" | "毛熊" => Some(FreeTextAliasTarget::Global("俄罗斯")),
        "狮城" | "坡县" => Some(FreeTextAliasTarget::Global("新加坡")),
        // China municipality nicknames
        "帝都" | "京城" | "燕京" | "北平" => {
            Some(FreeTextAliasTarget::ChinaProvince("北京市"))
        }
        "魔都" | "沪上" | "申城" => Some(FreeTextAliasTarget::ChinaProvince("上海市")),
        "津门" | "津沽" => Some(FreeTextAliasTarget::ChinaProvince("天津市")),
        "山城" | "雾都" | "渝都" => Some(FreeTextAliasTarget::ChinaProvince("重庆市")),
        // China major city nicknames (DataV exact adcode)
        "霸都" | "庐州" => Some(FreeTextAliasTarget::ChinaCity {
            province: "安徽省",
            city: "合肥市",
            adcode: "340100",
        }),
        "妖都" | "羊城" | "花城" | "穗城" => Some(FreeTextAliasTarget::ChinaCity {
            province: "广东省",
            city: "广州市",
            adcode: "440100",
        }),
        "鹏城" => Some(FreeTextAliasTarget::ChinaCity {
            province: "广东省",
            city: "深圳市",
            adcode: "440300",
        }),
        "江城" => Some(FreeTextAliasTarget::ChinaCity {
            province: "湖北省",
            city: "武汉市",
            adcode: "420100",
        }),
        "蓉城" | "锦城" => Some(FreeTextAliasTarget::ChinaCity {
            province: "四川省",
            city: "成都市",
            adcode: "510100",
        }),
        "星城" | "潭州" => Some(FreeTextAliasTarget::ChinaCity {
            province: "湖南省",
            city: "长沙市",
            adcode: "430100",
        }),
        "春城" => Some(FreeTextAliasTarget::ChinaCity {
            province: "云南省",
            city: "昆明市",
            adcode: "530100",
        }),
        "冰城" => Some(FreeTextAliasTarget::ChinaCity {
            province: "黑龙江省",
            city: "哈尔滨市",
            adcode: "230100",
        }),
        "榕城" => Some(FreeTextAliasTarget::ChinaCity {
            province: "福建省",
            city: "福州市",
            adcode: "350100",
        }),
        "泉城" => Some(FreeTextAliasTarget::ChinaCity {
            province: "山东省",
            city: "济南市",
            adcode: "370100",
        }),
        "金陵" | "石城" => Some(FreeTextAliasTarget::ChinaCity {
            province: "江苏省",
            city: "南京市",
            adcode: "320100",
        }),
        "姑苏" | "苏城" => Some(FreeTextAliasTarget::ChinaCity {
            province: "江苏省",
            city: "苏州市",
            adcode: "320500",
        }),
        "杭城" | "临安" => Some(FreeTextAliasTarget::ChinaCity {
            province: "浙江省",
            city: "杭州市",
            adcode: "330100",
        }),
        "长安" | "镐京" => Some(FreeTextAliasTarget::ChinaCity {
            province: "陕西省",
            city: "西安市",
            adcode: "610100",
        }),
        _ => match_taiwan_city_name(&cleaned)
            .map(|tw_city| FreeTextAliasTarget::Taiwan {
                province: "台湾省",
                city: tw_city,
            })
            .or_else(|| fuzzy_taiwan_typo_target(&lower)),
    }
}

fn char_edit_distance(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    if a_chars.is_empty() {
        return b_chars.len();
    }
    if b_chars.is_empty() {
        return a_chars.len();
    }
    let mut prev: Vec<usize> = (0..=b_chars.len()).collect();
    let mut curr = vec![0; b_chars.len() + 1];
    for (i, &ca) in a_chars.iter().enumerate() {
        curr[0] = i + 1;
        for (j, &cb) in b_chars.iter().enumerate() {
            let cost = usize::from(ca != cb);
            curr[j + 1] = (prev[j + 1] + 1).min(curr[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b_chars.len()]
}

fn fuzzy_typo_admin_target(raw: &str) -> Option<FreeTextAliasTarget> {
    let cleaned = clean_admin_search_text(raw);
    let q_len = cleaned.chars().count();
    if !(2..=32).contains(&q_len) {
        return None;
    }
    let q_lower = cleaned.to_lowercase();

    // Explicit high-confidence visual / pinyin CJK typos. Short 2-char and
    // 3-char `XX市` names MUST NOT run generic edit-distance-1 matching because
    // real Chinese cities collide (e.g. "日照" != "日本", "长治市" != "长沙市",
    // "南宁市" != "南京市", "德阳"/"德州" != "德国", "台州市" != "台中市").
    let explicit_cjk_typos = match q_lower.as_str() {
        "日木" | "日笨" => Some(FreeTextAliasTarget::Global("日本")),
        "美围" => Some(FreeTextAliasTarget::Global("美国")),
        "韩围" => Some(FreeTextAliasTarget::Global("韩国")),
        "法围" => Some(FreeTextAliasTarget::Global("法国")),
        "德围" => Some(FreeTextAliasTarget::Global("德国")),
        "合淝" | "合淝市" => Some(FreeTextAliasTarget::ChinaCity {
            province: "安徽省",
            city: "合肥市",
            adcode: "340100",
        }),
        "北经" | "北经市" => Some(FreeTextAliasTarget::ChinaProvince("北京市")),
        "上嗨" | "上嗨市" => Some(FreeTextAliasTarget::ChinaProvince("上海市")),
        "南经" | "南经市" => Some(FreeTextAliasTarget::ChinaCity {
            province: "江苏省",
            city: "南京市",
            adcode: "320100",
        }),
        "长砂" | "长砂市" => Some(FreeTextAliasTarget::ChinaCity {
            province: "湖南省",
            city: "长沙市",
            adcode: "430100",
        }),
        "武汗" | "武汗市" => Some(FreeTextAliasTarget::ChinaCity {
            province: "湖北省",
            city: "武汉市",
            adcode: "420100",
        }),
        "广洲" | "广洲市" => Some(FreeTextAliasTarget::ChinaCity {
            province: "广东省",
            city: "广州市",
            adcode: "440100",
        }),
        "深训" | "深训市" => Some(FreeTextAliasTarget::ChinaCity {
            province: "广东省",
            city: "深圳市",
            adcode: "440300",
        }),
        "抗州" | "抗州市" => Some(FreeTextAliasTarget::ChinaCity {
            province: "浙江省",
            city: "杭州市",
            adcode: "330100",
        }),
        _ => None,
    };
    if explicit_cjk_typos.is_some() {
        return explicit_cjk_typos;
    }

    let candidates: &[(&str, FreeTextAliasTarget)] = &[
        (
            "哈尔滨市",
            FreeTextAliasTarget::ChinaCity {
                province: "黑龙江省",
                city: "哈尔滨市",
                adcode: "230100",
            },
        ),
        (
            "乌鲁木齐市",
            FreeTextAliasTarget::ChinaCity {
                province: "新疆维吾尔自治区",
                city: "乌鲁木齐市",
                adcode: "650100",
            },
        ),
        (
            "呼和浩特市",
            FreeTextAliasTarget::ChinaCity {
                province: "内蒙古自治区",
                city: "呼和浩特市",
                adcode: "150100",
            },
        ),
        (
            "石家庄市",
            FreeTextAliasTarget::ChinaCity {
                province: "河北省",
                city: "石家庄市",
                adcode: "130100",
            },
        ),
        ("新加坡", FreeTextAliasTarget::Global("新加坡")),
        ("俄罗斯", FreeTextAliasTarget::Global("俄罗斯")),
        ("澳大利亚", FreeTextAliasTarget::Global("澳大利亚")),
        ("tokyo", FreeTextAliasTarget::Global("Tokyo")),
        ("beijing", FreeTextAliasTarget::ChinaProvince("北京市")),
        ("shanghai", FreeTextAliasTarget::ChinaProvince("上海市")),
        (
            "hefei",
            FreeTextAliasTarget::ChinaCity {
                province: "安徽省",
                city: "合肥市",
                adcode: "340100",
            },
        ),
        (
            "taipei",
            FreeTextAliasTarget::Taiwan {
                province: "台湾省",
                city: "台北市",
            },
        ),
        ("london", FreeTextAliasTarget::Global("London")),
        ("paris", FreeTextAliasTarget::Global("Paris")),
        ("seoul", FreeTextAliasTarget::Global("Seoul")),
        ("singapore", FreeTextAliasTarget::Global("Singapore")),
        ("bangkok", FreeTextAliasTarget::Global("Bangkok")),
    ];
    for &(cand, target) in candidates {
        let cand_len = cand.chars().count();
        if cand.is_ascii() {
            if !(q_lower.is_ascii() && q_len >= 5 && cand_len >= 5) {
                continue;
            }
        } else if q_len != cand_len || q_len < 3 {
            continue;
        }
        if char_edit_distance(&q_lower, cand) == 1 {
            return Some(target);
        }
    }
    None
}

#[cfg(test)]
mod rate_limit_tests {
    use std::sync::{Arc, Barrier, Mutex};
    use std::time::{Duration, Instant};

    use super::reserve_rate_slot;

    #[test]
    fn concurrent_nominatim_calls_reserve_distinct_serial_slots() {
        let last_request = Arc::new(Mutex::new(None));
        let start_barrier = Arc::new(Barrier::new(3));
        let start = Instant::now();
        let completions = Arc::new(Mutex::new(Vec::new()));
        let mut threads = Vec::new();
        for _ in 0..2 {
            let last_request = Arc::clone(&last_request);
            let start_barrier = Arc::clone(&start_barrier);
            let completions = Arc::clone(&completions);
            threads.push(std::thread::spawn(move || {
                start_barrier.wait();
                reserve_rate_slot(&last_request, Duration::from_millis(40))
                    .expect("reserve one serialized provider slot");
                completions
                    .lock()
                    .expect("completion lock")
                    .push(start.elapsed());
            }));
        }
        start_barrier.wait();
        for thread in threads {
            thread.join().expect("rate-limit worker completes");
        }
        let mut completions = completions.lock().expect("completion lock").clone();
        completions.sort();
        assert_eq!(completions.len(), 2);
        assert!(
            completions[1].saturating_sub(completions[0]) >= Duration::from_millis(35),
            "concurrent requests must not reserve the same provider slot: {completions:?}"
        );
    }
}

#[cfg(test)]
mod option_order_tests {
    use insar_provider::boundary::ChinaAdminOption;

    use super::normalize_admin_options;

    #[test]
    fn application_boundary_keeps_datav_options_in_exact_adcode_order() {
        let options = vec![
            ChinaAdminOption::unidentified("全部"),
            ChinaAdminOption::identified("黄石市", "420200"),
            ChinaAdminOption::identified("恩施土家族苗族自治州", "422800"),
            ChinaAdminOption::identified("武汉市", "420100"),
        ];

        let normalized = normalize_admin_options("datav", options).expect("valid DataV options");
        assert_eq!(
            normalized
                .iter()
                .map(|option| option.provider_object_id.as_deref())
                .collect::<Vec<_>>(),
            vec![None, Some("420100"), Some("420200"), Some("422800")],
        );
    }
}

#[cfg(test)]
mod route_tests {
    use super::*;

    #[test]
    fn production_providers_follow_the_saved_route_and_injected_ones_never_change() {
        let service = BoundaryService::production();
        let startup = service.global_provider();
        service.use_network_route(ProxyPolicy::System);
        assert!(Arc::ptr_eq(&startup, &service.global_provider()));
        service.use_network_route(ProxyPolicy::Direct);
        assert!(!Arc::ptr_eq(&startup, &service.global_provider()));
        assert_eq!(
            service.providers.lock().unwrap().route,
            Some(ProxyPolicy::Direct)
        );

        let injected =
            BoundaryService::with_providers(service.global_provider(), service.china_provider());
        let fixture = injected.global_provider();
        injected.use_network_route(ProxyPolicy::System);
        assert!(Arc::ptr_eq(&fixture, &injected.global_provider()));
    }
}
