//! Bounded native HTTP validation. Local construction remains loopback-only;
//! the separate public probe pins one reviewed anonymous source and query.
//! Neither constructor grants production registration or download authority.

use std::io::Read;
use std::time::{Duration, Instant};

use insar_core::ProviderNetworkPolicy;
use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_TYPE};

use super::*;
use crate::catalog::spatial::PreparedCatalogueAoi;

#[derive(Clone, Copy, Debug)]
pub struct StacTransportLimits {
    pub request_timeout: Duration,
    pub overall_timeout: Duration,
    pub max_pages: usize,
    pub max_records: usize,
    pub max_page_bytes: usize,
    pub max_total_bytes: usize,
}

impl Default for StacTransportLimits {
    fn default() -> Self {
        Self {
            request_timeout: Duration::from_secs(5),
            overall_timeout: Duration::from_secs(30),
            max_pages: 16,
            max_records: 10_000,
            max_page_bytes: 2 * 1024 * 1024,
            max_total_bytes: 16 * 1024 * 1024,
        }
    }
}

impl StacTransportLimits {
    fn validate(&self) -> ProviderResult<()> {
        if self.request_timeout.is_zero()
            || self.request_timeout > Duration::from_secs(30)
            || self.overall_timeout < self.request_timeout
            || self.overall_timeout > Duration::from_secs(60)
            || !(1..=128).contains(&self.max_pages)
            || !(1..=10_000).contains(&self.max_records)
            || !(1..=16 * 1024 * 1024).contains(&self.max_page_bytes)
            || self.max_total_bytes < self.max_page_bytes
            || self.max_total_bytes > 64 * 1024 * 1024
        {
            return Err(binding_error("transport_limits"));
        }
        Ok(())
    }
}

pub(crate) struct StacTransport {
    client: Client,
    route: StacRouteProfile,
    binding: StacCollectionBinding,
    socket: Url,
    limits: StacTransportLimits,
    /// Request newest-first on dated searches; see `stac_sorts_newest_first`.
    newest_first: bool,
    /// Optional STAC Item Search `fields.exclude` projection applied to POST
    /// `/search` requests when a route does not consume heavy item fields
    /// (such as per-item `assets`, `links`, or `stac_extensions`).
    pub(crate) excluded_item_fields: Vec<String>,
}

/// One logical search owns one budget, including any spatial request parts.
pub(crate) struct StacSearchBudget {
    deadline: Instant,
    pages: usize,
    records: usize,
    total_bytes: usize,
}

impl StacSearchBudget {
    pub(crate) fn check_stop(&self, cancelled: &(dyn Fn() -> bool + Sync)) -> ProviderResult<()> {
        check_stop(cancelled, self.deadline)
    }
}

pub struct StacLoopbackClient {
    transport: StacTransport,
}

#[path = "stac_public_validation.rs"]
mod public_validation;
pub use public_validation::StacPublicValidationClient;

impl fmt::Debug for StacLoopbackClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StacLoopbackClient")
            .field("scope", &"literal loopback only")
            .field("limits", &self.transport.limits)
            .finish_non_exhaustive()
    }
}

impl StacLoopbackClient {
    /// No production constructor exists. HTTPS logical identity is retained;
    /// only its validated search path/query is mapped to this local socket.
    #[doc(hidden)]
    pub fn new_for_test(
        route: StacRouteProfile,
        binding: StacCollectionBinding,
        loopback_origin: &str,
        limits: StacTransportLimits,
    ) -> ProviderResult<Self> {
        StacOfflineFixtureAdmission::new_for_test(route.clone(), binding.clone())?;
        limits.validate()?;
        let socket = Url::parse(loopback_origin).map_err(|_| binding_error("loopback_origin"))?;
        if socket.scheme() != "http"
            || socket.host_str() != Some("127.0.0.1")
            || socket.port().is_none_or(|port| port == 0)
            || socket.path() != "/"
            || socket.query().is_some()
            || socket.fragment().is_some()
            || !socket.username().is_empty()
            || socket.password().is_some()
        {
            return Err(binding_error("literal_loopback_only"));
        }
        Ok(Self {
            transport: StacTransport::new(route, binding, socket, limits)?,
        })
    }

    pub fn search(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        self.transport.search(query, cancelled)
    }
}

impl StacTransport {
    fn new(
        route: StacRouteProfile,
        binding: StacCollectionBinding,
        socket: Url,
        limits: StacTransportLimits,
    ) -> ProviderResult<Self> {
        Self::new_with_network_policy(
            route,
            binding,
            socket,
            limits,
            ProviderNetworkPolicy::direct(),
        )
    }

    pub(crate) fn new_with_network_policy(
        route: StacRouteProfile,
        binding: StacCollectionBinding,
        socket: Url,
        limits: StacTransportLimits,
        network_policy: ProviderNetworkPolicy,
    ) -> ProviderResult<Self> {
        binding.validate(&route)?;
        limits.validate()?;
        let mut builder = Client::builder()
            .http1_only()
            .tcp_nodelay(true)
            .pool_max_idle_per_host(4)
            .pool_idle_timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(limits.request_timeout)
            .timeout(limits.request_timeout);
        if socket.scheme() == "http"
            && socket
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
            .map_err(|_| network_error("client_construction"))?;
        Ok(Self {
            client,
            newest_first: stac_sorts_newest_first(&route.provider_id),
            route,
            binding,
            socket,
            limits,
            excluded_item_fields: Vec::new(),
        })
    }

    pub fn search(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        self.search_with_mapper(query, cancelled, |_, product| Ok(product))
    }

    /// The same reviewed route and binding, redirected to a literal loopback
    /// socket with test limits. Test-only; production keeps its fixed origin.
    #[cfg(test)]
    pub(crate) fn loopback_for_test(
        &self,
        loopback_origin: &str,
        limits: StacTransportLimits,
    ) -> ProviderResult<Self> {
        let socket = Url::parse(loopback_origin).map_err(|_| binding_error("loopback_origin"))?;
        let mut transport = Self::new(self.route.clone(), self.binding.clone(), socket, limits)?;
        transport
            .excluded_item_fields
            .clone_from(&self.excluded_item_fields);
        Ok(transport)
    }

    /// Exact lookup shares bounded JSON transport and collection validation.
    /// Accept a single safe Item ID, never a caller-provided URL or headers.
    pub(crate) fn read_exact_item(
        &self,
        item_id: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<(Value, ProviderProduct)> {
        if item_id.is_empty()
            || item_id.len() > 200
            || !item_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        {
            return Err(query_error("exact_item_id"));
        }
        let endpoint = Url::parse(&self.route.endpoint).map_err(|_| binding_error("endpoint"))?;
        let url = endpoint
            .join(&format!(
                "collections/{}/items/{item_id}",
                self.binding.native_collection_id
            ))
            .map_err(|_| query_error("exact_item_path"))?;
        if url.origin() != endpoint.origin() {
            return Err(query_error("exact_item_origin"));
        }
        let plan = StacSearchRequest {
            url,
            method: StacHttpMethod::Get,
            body: None,
            query_fingerprint: "exact-item".to_owned(),
        };
        let value = self.read_json(
            &plan,
            cancelled,
            Instant::now() + self.limits.overall_timeout,
            &mut 0,
        )?;
        let product = map_stac_item(&self.route, &self.binding, &value)?;
        if product.product_id != item_id {
            return Err(identity_error("exact_item_response"));
        }
        Ok((value, product))
    }

    pub(crate) fn search_with_mapper<F>(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
        map_feature: F,
    ) -> ProviderResult<CatalogSearchResponse>
    where
        F: FnMut(&Value, ProviderProduct) -> ProviderResult<ProviderProduct>,
    {
        self.search_with_mapper_in_budget(query, cancelled, &mut self.search_budget(), map_feature)
    }

    pub(crate) fn search_budget(&self) -> StacSearchBudget {
        StacSearchBudget {
            deadline: Instant::now() + self.limits.overall_timeout,
            pages: 0,
            records: 0,
            total_bytes: 0,
        }
    }

    pub(crate) fn search_with_mapper_in_budget<F>(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
        budget: &mut StacSearchBudget,
        map_feature: F,
    ) -> ProviderResult<CatalogSearchResponse>
    where
        F: FnMut(&Value, ProviderProduct) -> ProviderResult<ProviderProduct>,
    {
        self.search_with_mapper_exact_in_budget(query, cancelled, budget, None, map_feature)
    }

    /// The whole-response search with an optional exact AOI admission filter.
    /// When the request geometry is only a compact envelope of the user's
    /// AOI, `exact_aoi` is the original canonical AOI: a mapped product whose
    /// footprint has no positive-area overlap with it is observed (identity,
    /// duplicate and record budgets still apply) but never admitted, so it
    /// cannot count toward the requested cap. The request rectangle is never
    /// scientific authority (SAR-REG-010).
    pub(crate) fn search_with_mapper_exact<F>(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
        exact_aoi: Option<&PreparedCatalogueAoi>,
        map_feature: F,
    ) -> ProviderResult<CatalogSearchResponse>
    where
        F: FnMut(&Value, ProviderProduct) -> ProviderResult<ProviderProduct>,
    {
        self.search_with_mapper_exact_in_budget(
            query,
            cancelled,
            &mut self.search_budget(),
            exact_aoi,
            map_feature,
        )
    }

    pub(crate) fn search_with_mapper_exact_in_budget<F>(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
        budget: &mut StacSearchBudget,
        exact_aoi: Option<&PreparedCatalogueAoi>,
        map_feature: F,
    ) -> ProviderResult<CatalogSearchResponse>
    where
        F: FnMut(&Value, ProviderProduct) -> ProviderResult<ProviderProduct>,
    {
        // The whole-response form is the streaming loop with an accumulating
        // sink; the final order and cap match the frozen adapter contract.
        let query = &insar_core::clamp_catalogue_search_query(query);
        let mut products: Vec<ProviderProduct> = Vec::new();
        let (_collection, stats) = self.search_paged(
            query,
            cancelled,
            budget,
            exact_aoi,
            map_feature,
            &mut |page| {
                products.extend_from_slice(page);
                Ok(())
            },
            false,
        )?;
        // The classic whole-response adapter order: item-ID ascending, capped
        // by the requested limit. (The newest-first server order already
        // decided which records survive the cap inside the loop.)
        products.sort_by(|a, b| a.product_id.cmp(&b.product_id));
        if let Some(limit) = query.result_limit {
            products.truncate(limit as usize);
        }
        let response = CatalogSearchResponse {
            collection: self.binding.collection_descriptor(),
            stats,
            products,
            next_cursor: None,
            item_errors: Vec::new(),
        };
        response
            .validate()
            .map_err(|_| response_error("catalogue_contract"))?;
        Ok(response)
    }

    /// #22 B slice 3(b): the same reviewed paging loop as
    /// `search_with_mapper_in_budget`, but each page's mapped products are
    /// handed to `sink` in the transport's own order (newest-first first
    /// pages, then continuation order) instead of being accumulated until the
    /// search completes. The caller re-freezes the final item-ID order.
    ///
    /// The cap semantics of the whole-response adapter are preserved:
    /// - a newest-first search keeps the first `limit` records in server
    ///   order, and each page is flushed as soon as it is complete;
    /// - an undated search (server order carries no meaning) caps by
    ///   product-ID ascending over the whole result, which cannot be decided
    ///   while streaming. The same holds for a continuation that states a
    ///   different sort mid-search. Both fail closed with the typed
    ///   `streaming_cap_order` error so the caller can retry the exact same
    ///   query on the whole-response path.
    ///
    /// Only a bounded window (one page) is held.
    pub(crate) fn search_streaming<F, S>(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
        budget: &mut StacSearchBudget,
        map_feature: F,
        sink: &mut S,
    ) -> ProviderResult<(CatalogCollectionDescriptor, CatalogSearchStats)>
    where
        F: FnMut(&Value, ProviderProduct) -> ProviderResult<ProviderProduct>,
        S: FnMut(&[ProviderProduct]) -> ProviderResult<()>,
    {
        self.search_paged(query, cancelled, budget, None, map_feature, sink, true)
    }

    /// The streaming search with the optional exact AOI admission filter of
    /// [`Self::search_with_mapper_exact`]. Rejected records are never handed
    /// to `sink` and never fill the newest-first cap.
    pub(crate) fn search_streaming_exact<F, S>(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
        budget: &mut StacSearchBudget,
        exact_aoi: Option<&PreparedCatalogueAoi>,
        map_feature: F,
        sink: &mut S,
    ) -> ProviderResult<(CatalogCollectionDescriptor, CatalogSearchStats)>
    where
        F: FnMut(&Value, ProviderProduct) -> ProviderResult<ProviderProduct>,
        S: FnMut(&[ProviderProduct]) -> ProviderResult<()>,
    {
        self.search_paged(query, cancelled, budget, exact_aoi, map_feature, sink, true)
    }

    #[allow(clippy::too_many_arguments)]
    fn search_paged<F, S>(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
        budget: &mut StacSearchBudget,
        exact_aoi: Option<&PreparedCatalogueAoi>,
        mut map_feature: F,
        sink: &mut S,
        strict_streaming: bool,
    ) -> ProviderResult<(CatalogCollectionDescriptor, CatalogSearchStats)>
    where
        F: FnMut(&Value, ProviderProduct) -> ProviderResult<ProviderProduct>,
        S: FnMut(&[ProviderProduct]) -> ProviderResult<()>,
    {
        let started = Instant::now();
        let deadline = budget.deadline;
        check_stop(cancelled, deadline)?;
        let query = &insar_core::clamp_catalogue_search_query(query);
        if query.cursor.is_some()
            || query
                .result_limit
                .is_some_and(|n| n > self.limits.max_records as u64)
        {
            return Err(query_error("complete_query_bounds"));
        }
        // A streaming caller needs the server order to decide the cap before
        // each page is flushed. An undated search sends no sortby, and a
        // continuation that states a different sort breaks the assumption
        // mid-search; both are only detected inside the loop and fail closed
        // there. Dated newest-first searches can stream from the start.
        let mut current = query.clone();
        let mut page_products = Vec::new();
        let mut identities = BTreeSet::new();
        let mut continuations = BTreeSet::new();
        let mut matched = None;
        // merge:true next links merge onto the first page's request as sent.
        let mut original_body = None;
        // Server order decides the cap only if the search asked newest-first
        // and no continuation stated another order.
        let mut every_page_newest_first = true;
        let cloud_limit = query.filters.get("cloud_cover_lte").and_then(Value::as_f64);
        let mut cloud_filtered_any = false;
        let mut admitted_total: u64 = 0;
        // Every raw record the server returned; equal to `admitted_total`
        // unless an exact AOI or strict cloud_cover filter rejected a record.
        let mut observed_total: u64 = 0;

        for page_index in 0..self.limits.max_pages {
            check_stop(cancelled, deadline)?;
            if budget.pages >= self.limits.max_pages {
                return Err(limit_error("pages"));
            }
            let mut request = build_stac_search_request(&self.route, &self.binding, &current)?;
            // The first page must ask newest-first itself; a later page keeps
            // that order unless it states a different sortby.
            let sort = if self.newest_first {
                apply_newest_first_sort(&mut request)
            } else {
                Some(false)
            };
            let page_newest_first = sort.unwrap_or(page_index > 0);
            if strict_streaming && !page_newest_first {
                // The streamed cap must be decidable page-by-page. The first
                // page of an undated search states no order, and a
                // continuation stating a different sort breaks the running
                // order assumption; fail closed with a typed reason instead
                // of streaming a possibly-wrong prefix.
                return Err(response_error("streaming_cap_order"));
            }
            every_page_newest_first &= page_newest_first;
            if let Some(body) = request.body.as_mut() {
                if page_index == 0 && exact_aoi.is_some() {
                    body.insert(
                        "limit".to_owned(),
                        Value::Number(self.route.page_cap.into()),
                    );
                }
                if !self.excluded_item_fields.is_empty() {
                    body.entry("fields".to_owned())
                        .or_insert_with(|| json!({"exclude": self.excluded_item_fields}));
                }
            }
            if page_index == 0 {
                original_body = request.body.clone();
            }
            let value = self.read_page(&request, cancelled, deadline, &mut budget.total_bytes)?;
            // A rejected oversized request body did not perform an HTTP call.
            budget.pages += 1;
            let object = value
                .as_object()
                .ok_or_else(|| response_error("item_collection"))?;
            if object.get("type").and_then(Value::as_str) != Some("FeatureCollection") {
                return Err(response_error("item_collection_type"));
            }
            let features = object
                .get("features")
                .and_then(Value::as_array)
                .filter(|items| items.len() <= self.route.page_cap as usize)
                .ok_or_else(|| response_error("page_features"))?;
            if object
                .get("numberReturned")
                .is_some_and(|n| n.as_u64() != Some(features.len() as u64))
            {
                return Err(response_error("number_returned"));
            }
            if features.len() > self.limits.max_records.saturating_sub(budget.records) {
                return Err(limit_error("records"));
            }
            budget.records += features.len();
            page_products.clear();
            for feature in features {
                check_stop(cancelled, deadline)?;
                // A scientific projection may change display metadata, never
                // bypass the shared raw STAC/version/privacy/geometry checks.
                let validated = map_stac_item(&self.route, &self.binding, feature)?;
                let expected_id = validated.product_id.clone();
                let product = map_feature(feature, validated)?;
                if product.product_id != expected_id
                    || product.provider_id != self.binding.provider_id
                {
                    return Err(identity_error("custom_mapper_identity"));
                }
                if !identities.insert(product.product_id.clone()) {
                    return Err(identity_error("duplicate_product_across_pages"));
                }
                if exact_aoi.is_some_and(|aoi| {
                    !product
                        .footprint_geojson
                        .as_ref()
                        .is_some_and(|footprint| aoi.intersects_footprint(footprint))
                }) {
                    // Inside the compact request envelope only: not a hit.
                    continue;
                }
                let item_cloud = product
                    .properties
                    .get("cloud_cover")
                    .and_then(Value::as_f64)
                    .or_else(|| {
                        feature
                            .get("properties")
                            .and_then(|p| p.get("eo:cloud_cover"))
                            .and_then(Value::as_f64)
                    });
                if cloud_limit.is_some_and(|limit| item_cloud.is_some_and(|cloud| cloud > limit)) {
                    cloud_filtered_any = true;
                    continue;
                }
                page_products.push(product);
            }
            observed_total = observed_total
                .checked_add(features.len() as u64)
                .ok_or_else(|| limit_error("records"))?;
            admitted_total = admitted_total
                .checked_add(page_products.len() as u64)
                .ok_or_else(|| limit_error("records"))?;
            if let Some(total) = object.get("numberMatched") {
                let total = total
                    .as_u64()
                    .ok_or_else(|| response_error("number_matched"))?;
                if matched.is_some_and(|prior| prior != total) {
                    return Err(response_error("changed_matched_count"));
                }
                matched = Some(total);
            }
            if matched.is_some_and(|total| total < observed_total) {
                return Err(response_error("number_matched_below_observed"));
            }
            validate_link_overrides(object.get("links"))?;
            let next = parse_next_cursor(
                &self.route,
                &request.query_fingerprint,
                object.get("links"),
                original_body.as_ref(),
            )?;
            // Validate even an unused next Link before a finite result is returned.
            if let Some(cursor) = next.as_deref() {
                let continuation = unwrap_cursor(&self.route, &request.query_fingerprint, cursor)?;
                self.validate_search_url(&continuation.url)?;
            }
            let exhausted = next.is_none();
            let complete = query
                .result_limit
                .map_or(exhausted, |n| admitted_total >= n || exhausted);
            if complete {
                // A newest-first request decides which records survive the cap
                // in the server's order, before the sink sees them: the kept
                // records are the first `limit` in server order.
                if every_page_newest_first {
                    if let Some(limit) = query.result_limit {
                        if admitted_total > limit {
                            page_products.truncate(
                                (limit as usize)
                                    .checked_sub(admitted_total as usize - page_products.len())
                                    .expect("kept prefix fits within this page"),
                            );
                        }
                    }
                }
            }
            // The page's products flow to the sink now, before the completion
            // decision, so a newest-first cap keeps the server's order and a
            // streamed caller sees every page exactly once in transport order.
            sink(&page_products)?;
            if complete {
                check_stop(cancelled, deadline)?;
                let fetched = admitted_total;
                // A newest-first request decides which records survive the cap
                // in the server's order: the streamed prefix of the server's
                // order is exactly the kept set, so the count is the cap.
                let mut returned = fetched;
                if every_page_newest_first {
                    if let Some(limit) = query.result_limit {
                        returned = returned.min(limit);
                    }
                }
                if let Some(limit) = query.result_limit {
                    returned = returned.min(limit);
                }
                let stats = CatalogSearchStats {
                    requested_limit: query.result_limit,
                    // The server total counts the request envelope or pgstac rounded cloud buckets,
                    // not exact post-filtered hits.
                    matched_count: if exact_aoi.is_some() || cloud_filtered_any {
                        None
                    } else {
                        matched
                    },
                    request_count: page_index as u64 + 1,
                    // One spatial part: the request carries the whole AOI.
                    transport_part_count: 1,
                    fetched_record_count: observed_total,
                    mapped_record_count: observed_total,
                    profile_filtered_record_count: 0,
                    exact_duplicate_record_count: 0,
                    unique_provider_product_count: observed_total,
                    exact_matched_count: fetched,
                    returned_count: returned,
                    remote_exhausted: exhausted,
                    working_set_satisfied: true,
                    elapsed_ms: started.elapsed().as_millis().min(u64::MAX as u128) as u64,
                    truncated: false,
                };
                // The streamed products are not held here, so the statistics
                // alone are validated; the whole-response wrapper re-checks
                // the products/counts agreement on its own copy.
                stats
                    .validate()
                    .map_err(|_| response_error("catalogue_contract"))?;
                check_stop(cancelled, deadline)?;
                return Ok((self.binding.collection_descriptor(), stats));
            }
            let cursor = next.expect("incomplete page must have continuation");
            if !continuations.insert(cursor.clone()) {
                return Err(cursor_error("continuation_cycle"));
            }
            current.cursor = Some(cursor);
        }
        Err(limit_error("pages"))
    }

    fn validate_search_url(&self, url: &Url) -> ProviderResult<()> {
        validate_continuation_url(&self.route, url.as_str())?;
        if url.path() != self.route.search_url()?.path()
            || url.query_pairs().any(|(key, _)| is_sensitive_key(&key))
        {
            return Err(cursor_error("search_endpoint_only"));
        }
        Ok(())
    }

    fn read_page(
        &self,
        plan: &StacSearchRequest,
        cancelled: &(dyn Fn() -> bool + Sync),
        deadline: Instant,
        total_bytes: &mut usize,
    ) -> ProviderResult<Value> {
        self.validate_search_url(&plan.url)?;
        self.read_json(plan, cancelled, deadline, total_bytes)
    }

    fn read_json(
        &self,
        plan: &StacSearchRequest,
        cancelled: &(dyn Fn() -> bool + Sync),
        deadline: Instant,
        total_bytes: &mut usize,
    ) -> ProviderResult<Value> {
        check_stop(cancelled, deadline)?;
        let timeout = self
            .limits
            .request_timeout
            .min(deadline.saturating_duration_since(Instant::now()));
        if timeout.is_zero() {
            return Err(timeout_error("overall_deadline"));
        }
        let request_started = Instant::now();
        let actual = self
            .socket
            .join(&plan.url[Position::BeforePath..Position::AfterQuery])
            .map_err(|_| cursor_error("loopback_path"))?;
        // No caller-controlled origin or headers cross this boundary.
        if actual.origin() != self.socket.origin() {
            return Err(cursor_error("loopback_scope"));
        }
        let builder = match plan.method {
            StacHttpMethod::Get => self.client.get(actual),
            StacHttpMethod::Post => {
                let body = plan.body.as_ref().ok_or_else(|| query_error("post_body"))?;
                let bytes = serde_json::to_vec(body).map_err(|_| query_error("post_body"))?;
                if bytes.len() > MAX_CURSOR_BYTES {
                    return Err(limit_error("request_body"));
                }
                self.client
                    .post(actual)
                    .header(CONTENT_TYPE, "application/json")
                    .body(bytes)
            }
        };
        let sent = builder
            .header(ACCEPT, "application/geo+json, application/json")
            .header(ACCEPT_ENCODING, "identity")
            .timeout(timeout)
            .send();
        check_stop(cancelled, deadline)?;
        let mut response = sent.map_err(|err| {
            if err.is_timeout() {
                timeout_error("request_deadline")
            } else {
                network_error("request_failed")
            }
        })?;
        let status = response.status().as_u16();
        if status != 200 {
            return Err(status_error(status));
        }
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|v| {
                v.split(';')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase()
            });
        if !matches!(
            content_type.as_deref(),
            Some("application/json" | "application/geo+json")
        ) {
            return Err(response_error("content_type"));
        }
        if response
            .headers()
            .get(CONTENT_ENCODING)
            .is_some_and(|v| v != "identity")
        {
            return Err(response_error("content_encoding"));
        }
        let allowance = self
            .limits
            .max_page_bytes
            .min(self.limits.max_total_bytes.saturating_sub(*total_bytes));
        if allowance == 0
            || response
                .content_length()
                .is_some_and(|n| n > allowance as u64)
        {
            return Err(limit_error("response_bytes"));
        }
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 8192];
        loop {
            check_stop(cancelled, deadline)?;
            let read = response.read(&mut buffer);
            check_stop(cancelled, deadline)?;
            let n = read.map_err(|err| {
                if err.kind() == std::io::ErrorKind::TimedOut
                    || request_started.elapsed() >= timeout
                {
                    timeout_error("body_deadline")
                } else {
                    network_error("body_read")
                }
            })?;
            if n == 0 {
                break;
            }
            if n > allowance.saturating_sub(bytes.len()) {
                return Err(limit_error("response_bytes"));
            }
            bytes.extend_from_slice(&buffer[..n]);
        }
        *total_bytes += bytes.len();
        let page = serde_json::from_slice(&bytes).map_err(|_| response_error("json"))?;
        check_stop(cancelled, deadline)?;
        Ok(page)
    }
}

fn validate_link_overrides(value: Option<&Value>) -> ProviderResult<()> {
    let Some(value) = value else {
        return Ok(());
    };
    let links = value.as_array().ok_or_else(|| response_error("links"))?;
    for link in links {
        if link.get("rel").and_then(Value::as_str) != Some("next") {
            continue;
        }
        // merge:true is honoured only for a POST link; its body is merged
        // onto the request that produced this page (see parse_next_cursor).
        let post = link
            .get("method")
            .and_then(Value::as_str)
            .is_some_and(|method| method.eq_ignore_ascii_case("POST"));
        if link.get("method").is_some_and(|v| !v.is_string())
            || link
                .get("merge")
                .is_some_and(|v| v.as_bool().is_none() || (v.as_bool() == Some(true) && !post))
        {
            return Err(cursor_error("unsupported_link_override"));
        }
    }
    Ok(())
}

fn check_stop(cancelled: &(dyn Fn() -> bool + Sync), deadline: Instant) -> ProviderResult<()> {
    if cancelled() {
        return Err(error(
            "STAC_SEARCH_CANCELLED",
            "目录检索已取消",
            "cancelled",
        ));
    }
    if Instant::now() >= deadline {
        return Err(timeout_error("overall_deadline"));
    }
    Ok(())
}
fn limit_error(reason: &str) -> ContractError {
    error("STAC_LIMIT_EXCEEDED", "目录检索超过安全限制", reason)
}
fn timeout_error(reason: &str) -> ContractError {
    error("STAC_TIMEOUT", "目录请求超时", reason)
}
fn network_error(reason: &str) -> ContractError {
    error("STAC_NETWORK_ERROR", "目录网络请求失败", reason)
}
fn status_error(status: u16) -> ContractError {
    let (code, message) = match status {
        404 => ("STAC_ITEM_NOT_FOUND", "该目录产品暂无可用数据"),
        300..=399 => ("STAC_REDIRECT_REJECTED", "目录跳转未获准"),
        401 | 403 => ("STAC_ACCESS_DENIED", "目录访问被拒绝"),
        429 => ("STAC_RATE_LIMITED", "目录请求受到限流"),
        500..=599 => ("STAC_SERVER_ERROR", "目录服务暂时不可用"),
        _ => ("STAC_HTTP_ERROR", "目录请求失败"),
    };
    error(code, message, "http_status")
}

#[cfg(test)]
#[path = "stac_transport_tests.rs"]
mod tests;
