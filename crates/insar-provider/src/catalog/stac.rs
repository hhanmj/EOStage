use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fmt;

use chrono::{DateTime, SecondsFormat};
use insar_core::{
    provider_product_key, stable_asset_key, AssetRole, CatalogCollectionDescriptor,
    CatalogDeliveryModel, CatalogSearchQuery, CatalogSearchResponse, CatalogSearchStats,
    ContractError, DataSourceKind, ProductKind, ProviderAsset, ProviderProduct, ProviderResult,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Number, Value};
use sha2::{Digest, Sha256};
use url::{Position, Url};

const CURSOR_VERSION: &str = "sceneport-stac-cursor-v1";
const MAX_CURSOR_BYTES: usize = 32 * 1024;
const MAX_ITEM_BYTES: usize = 512 * 1024;
const MAX_ITEM_PROPERTIES: usize = 512;
const MAX_ITEM_ASSETS: usize = 256;
const MAX_ITEM_LINKS: usize = 128;
const MAX_EXTENSIONS: usize = 64;
const MAX_TEXT_CHARS: usize = 1024;

#[path = "stac_fixture.rs"]
mod offline_fixture;
#[doc(hidden)]
pub use offline_fixture::StacOfflineFixtureAdmission;

#[path = "stac_transport.rs"]
mod loopback_transport;
#[doc(hidden)]
pub use loopback_transport::{StacLoopbackClient, StacPublicValidationClient, StacTransportLimits};
pub(crate) use loopback_transport::{StacSearchBudget, StacTransport};

#[cfg(test)]
#[path = "stac_datetime_tests.rs"]
mod datetime_tests;

/// Reviewed STAC APIs that implement the Item Search sort extension. A dated
/// search there asks for newest-first so a capped result set keeps the most
/// recent scenes. Any other API (including loopback fixtures) gets no `sortby`.
const NEWEST_FIRST_STAC_PROVIDER_IDS: &[&str] = &[
    super::EARTH_SEARCH_LANDSAT_PROVIDER_ID,
    crate::planetary_computer_asset::PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID,
];

pub fn stac_sorts_newest_first(provider_id: &str) -> bool {
    NEWEST_FIRST_STAC_PROVIDER_IDS.contains(&provider_id)
}

/// Maximum coordinate positions in a simple exterior polygon ring before
/// remote STAC `/search` requests compact `intersects` to a 5-position bounding
/// envelope while retaining `PreparedCatalogueAoi` locally for exact
/// positive-area overlap admission (`SAR-REG-010`).
pub const COMPLEX_REQUEST_AOI_MAX_POSITIONS: usize = 16;

fn aoi_requires_compact_envelope(aoi: &Map<String, Value>) -> bool {
    if serde_json::to_vec(aoi)
        .map(|bytes| bytes.len() > super::spatial::COMPACT_REQUEST_AOI_BYTES)
        .unwrap_or(false)
    {
        return true;
    }
    match (
        aoi.get("type").and_then(Value::as_str),
        aoi.get("coordinates").and_then(Value::as_array),
    ) {
        (Some("Polygon"), Some(rings)) => {
            rings.len() > 1
                || rings
                    .first()
                    .and_then(Value::as_array)
                    .is_some_and(|ring| ring.len() > COMPLEX_REQUEST_AOI_MAX_POSITIONS)
        }
        (Some("MultiPolygon"), Some(polygons)) => {
            if polygons.len() > 1 {
                return true;
            }
            polygons.iter().any(|poly| {
                poly.as_array().is_some_and(|rings| {
                    rings.len() > 1
                        || rings
                            .first()
                            .and_then(Value::as_array)
                            .is_some_and(|ring| ring.len() > COMPLEX_REQUEST_AOI_MAX_POSITIONS)
                })
            })
        }
        _ => false,
    }
}

/// Replace complex or oversized administrative AOI geometries (> 16 vertices,
/// interior holes, multi-polygons, or > 24 KiB) with a compact bounding box
/// for the remote STAC request while returning the exact `PreparedCatalogueAoi`
/// for local positive-area intersection filtering (`SAR-REG-010`).
pub fn compact_complex_request_aoi(
    query: &CatalogSearchQuery,
) -> ProviderResult<(
    CatalogSearchQuery,
    Option<super::spatial::PreparedCatalogueAoi>,
)> {
    let Some(aoi) = query.aoi_geojson.as_ref() else {
        return Ok((query.clone(), None));
    };
    if !aoi_requires_compact_envelope(aoi) {
        return Ok((query.clone(), None));
    }
    let Some(canonical) = super::spatial::authoritative_catalogue_geometry(aoi) else {
        return Ok((query.clone(), None));
    };
    let Some(exact) = super::spatial::PreparedCatalogueAoi::new(&canonical) else {
        return Ok((query.clone(), None));
    };
    let Some(request_geojson) =
        super::spatial::bounded_catalogue_request_tiles(&canonical, 180.0, 90.0, 1)
            .and_then(|tiles| tiles.into_iter().next())
    else {
        return Ok((query.clone(), None));
    };
    let mut compact_query = query.clone();
    compact_query.aoi_geojson = Some(request_geojson);
    Ok((compact_query, Some(exact)))
}

/// Ask a sort-capable API for newest-first on a dated request: the POST body
/// form `sortby=[{field,direction}]`, or the GET form `sortby=-properties.datetime`.
/// Returns `Some(true)` when the request now carries exactly that sort,
/// `Some(false)` when it already carries a different `sortby` (left unchanged),
/// and `None` when it states no order and has no `datetime` (for example a
/// token-only continuation, which continues the first page's order).
pub(crate) fn apply_newest_first_sort(request: &mut StacSearchRequest) -> Option<bool> {
    const GET_SORT: &str = "-properties.datetime";
    let post_sort = json!([{"field": "properties.datetime", "direction": "desc"}]);
    match request.method {
        StacHttpMethod::Post => {
            let body = request.body.as_mut()?;
            if let Some(existing) = body.get("sortby") {
                return Some(existing == &post_sort);
            }
            if !body.contains_key("datetime") {
                return None;
            }
            body.insert("sortby".to_owned(), post_sort);
            Some(true)
        }
        StacHttpMethod::Get => {
            let mut dated = false;
            let mut sorts = Vec::new();
            for (key, value) in request.url.query_pairs() {
                dated |= key == "datetime";
                if key == "sortby" {
                    sorts.push(value.into_owned());
                }
            }
            if !sorts.is_empty() {
                return Some(sorts == [GET_SORT]);
            }
            if !dated {
                return None;
            }
            request
                .url
                .query_pairs_mut()
                .append_pair("sortby", GET_SORT);
            Some(true)
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StacHttpMethod {
    Get,
    Post,
}

impl StacHttpMethod {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
        }
    }

    fn parse(value: &str) -> ProviderResult<Self> {
        match value.to_ascii_uppercase().as_str() {
            "GET" => Ok(Self::Get),
            "POST" => Ok(Self::Post),
            _ => Err(cursor_error("next_method")),
        }
    }
}

#[derive(Clone, PartialEq)]
pub struct StacSearchRequest {
    pub url: Url,
    pub method: StacHttpMethod,
    pub body: Option<Map<String, Value>>,
    pub query_fingerprint: String,
}

impl fmt::Debug for StacSearchRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let safe_url = format!(
            "{}://{}{}",
            self.url.scheme(),
            self.url.host_str().unwrap_or("[invalid-host]"),
            self.url.path()
        );
        formatter
            .debug_struct("StacSearchRequest")
            .field("url", &safe_url)
            .field("method", &self.method)
            .field(
                "body",
                &self.body.as_ref().map(|_| "[REDACTED_REQUEST_BODY]"),
            )
            .field("query_fingerprint", &self.query_fingerprint)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StacQueryOperator {
    Eq,
    In,
    Lte,
    Gte,
}

impl StacQueryOperator {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Eq => "eq",
            Self::In => "in",
            Self::Lte => "lte",
            Self::Gte => "gte",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StacQueryValueKind {
    String,
    StringList,
    Number,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StacQueryableBinding {
    pub sceneport_filter_key: String,
    pub stac_property: String,
    pub operator: StacQueryOperator,
    pub value_kind: StacQueryValueKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StacAssetRoleBinding {
    pub stac_asset_key: String,
    pub role: AssetRole,
    pub required: bool,
    pub allowed_hosts: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StacRouteProfile {
    pub provider_id: String,
    pub endpoint: String,
    pub search_path: String,
    pub page_cap: u64,
    pub accepted_stac_versions: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StacCollectionBinding {
    pub logical_dataset: String,
    pub provider_id: String,
    pub native_collection_id: String,
    pub collection_profile_id: String,
    pub identity_rule_id: String,
    pub filename_rule_id: String,
    pub delivery_model: CatalogDeliveryModel,
    pub source_kind: DataSourceKind,
    pub product_kind: ProductKind,
    pub production_enabled: bool,
    pub product_id_property: Option<String>,
    pub display_name_property: Option<String>,
    pub queryables: Vec<StacQueryableBinding>,
    pub asset_role_bindings: Vec<StacAssetRoleBinding>,
}

impl StacRouteProfile {
    pub fn validate(&self) -> ProviderResult<()> {
        validate_text(&self.provider_id, "provider_id").map_err(binding_error)?;
        if !(1..=10_000).contains(&self.page_cap) {
            return Err(binding_error("page_cap"));
        }
        if self.accepted_stac_versions.is_empty() || self.accepted_stac_versions.len() > 8 {
            return Err(binding_error("accepted_stac_versions"));
        }
        let mut versions = BTreeSet::new();
        for version in &self.accepted_stac_versions {
            validate_text(version, "stac_version").map_err(binding_error)?;
            if !matches!(version.as_str(), "1.0.0" | "1.1.0") || !versions.insert(version) {
                return Err(binding_error("accepted_stac_versions"));
            }
        }
        let endpoint = self.endpoint_url()?;
        if endpoint.scheme() != "https"
            || endpoint.host_str().is_none()
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
            || !endpoint.path().ends_with('/')
            || endpoint.port_or_known_default() != Some(443)
        {
            return Err(binding_error("endpoint"));
        }
        if self.search_path.is_empty()
            || self.search_path.starts_with('/')
            || self.search_path.contains(['?', '#', '\\'])
            || self
                .search_path
                .split('/')
                .any(|part| part.is_empty() || part == "..")
        {
            return Err(binding_error("search_path"));
        }
        Ok(())
    }

    fn endpoint_url(&self) -> ProviderResult<Url> {
        Url::parse(&self.endpoint).map_err(|_| binding_error("endpoint"))
    }

    fn search_url(&self) -> ProviderResult<Url> {
        self.endpoint_url()?
            .join(&self.search_path)
            .map_err(|_| binding_error("search_path"))
    }

    fn accepts_version(&self, version: &str) -> bool {
        self.accepted_stac_versions
            .iter()
            .any(|accepted| accepted == version)
    }
}

impl StacCollectionBinding {
    pub fn validate(&self, route: &StacRouteProfile) -> ProviderResult<()> {
        route.validate()?;
        for (value, reason) in [
            (&self.logical_dataset, "logical_dataset"),
            (&self.provider_id, "provider_id"),
            (&self.native_collection_id, "native_collection_id"),
            (&self.collection_profile_id, "collection_profile_id"),
            (&self.identity_rule_id, "identity_rule_id"),
            (&self.filename_rule_id, "filename_rule_id"),
        ] {
            validate_text(value, reason).map_err(binding_error)?;
        }
        if self.provider_id != route.provider_id {
            return Err(binding_error("provider_route_mismatch"));
        }
        for property in [
            self.product_id_property.as_deref(),
            self.display_name_property.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            validate_property_name(property).map_err(binding_error)?;
        }
        let mut filter_keys = BTreeSet::new();
        let mut stac_properties = BTreeSet::new();
        for queryable in &self.queryables {
            validate_text(&queryable.sceneport_filter_key, "queryable_key")
                .map_err(binding_error)?;
            validate_property_name(&queryable.stac_property).map_err(binding_error)?;
            if queryable.sceneport_filter_key == "datetime"
                || !filter_keys.insert(&queryable.sceneport_filter_key)
                || !stac_properties.insert(&queryable.stac_property)
            {
                return Err(binding_error("queryables"));
            }
            match (&queryable.operator, &queryable.value_kind) {
                (StacQueryOperator::In, StacQueryValueKind::StringList)
                | (StacQueryOperator::Eq, StacQueryValueKind::String)
                | (StacQueryOperator::Lte | StacQueryOperator::Gte, StacQueryValueKind::Number) => {
                }
                _ => return Err(binding_error("queryable_operator_kind")),
            }
        }
        let mut asset_keys = BTreeSet::new();
        for asset in &self.asset_role_bindings {
            validate_text(&asset.stac_asset_key, "asset_key").map_err(binding_error)?;
            if is_sensitive_key(&asset.stac_asset_key)
                || !asset_keys.insert(&asset.stac_asset_key)
                || asset.allowed_hosts.is_empty()
            {
                return Err(binding_error("asset_bindings"));
            }
            let mut hosts = BTreeSet::new();
            for host in &asset.allowed_hosts {
                validate_host(host).map_err(binding_error)?;
                if !hosts.insert(host.to_ascii_lowercase()) {
                    return Err(binding_error("asset_hosts"));
                }
            }
        }
        self.collection_descriptor()
            .validate()
            .map_err(|_| binding_error("collection_descriptor"))?;
        Ok(())
    }

    pub fn collection_descriptor(&self) -> CatalogCollectionDescriptor {
        CatalogCollectionDescriptor {
            logical_dataset: self.logical_dataset.clone(),
            provider_id: self.provider_id.clone(),
            native_collection_id: self.native_collection_id.clone(),
            collection_profile_id: self.collection_profile_id.clone(),
            identity_rule_id: self.identity_rule_id.clone(),
            filename_rule_id: self.filename_rule_id.clone(),
            delivery_model: self.delivery_model.clone(),
            production_enabled: self.production_enabled,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CursorEnvelope {
    version: String,
    fingerprint: String,
    href_path_query: String,
    method: String,
    body: Option<Map<String, Value>>,
}

#[derive(Clone, Debug, PartialEq)]
struct StacContinuation {
    url: Url,
    method: StacHttpMethod,
    body: Option<Map<String, Value>>,
}

pub fn build_stac_search_request(
    route: &StacRouteProfile,
    binding: &StacCollectionBinding,
    query: &CatalogSearchQuery,
) -> ProviderResult<StacSearchRequest> {
    let query = &insar_core::clamp_catalogue_search_query(query);
    binding.validate(route)?;
    validate_query_scope(binding, query)?;
    let fingerprint = query_fingerprint(route, binding, query)?;
    if let Some(cursor) = query.cursor.as_deref() {
        let continuation = unwrap_cursor(route, &fingerprint, cursor)?;
        return Ok(StacSearchRequest {
            url: continuation.url,
            method: continuation.method,
            body: continuation.body,
            query_fingerprint: fingerprint,
        });
    }
    let body = build_base_search_body(route, binding, query)?;
    Ok(StacSearchRequest {
        url: route.search_url()?,
        method: StacHttpMethod::Post,
        body: Some(body),
        query_fingerprint: fingerprint,
    })
}

pub fn map_stac_item(
    route: &StacRouteProfile,
    binding: &StacCollectionBinding,
    item: &Value,
) -> ProviderResult<ProviderProduct> {
    binding.validate(route)?;
    let item_bytes = serde_json::to_vec(item).map_err(|_| response_error("item_json"))?;
    if item_bytes.len() > MAX_ITEM_BYTES {
        return Err(response_error("item_size"));
    }
    let object = item
        .as_object()
        .ok_or_else(|| response_error("item_object"))?;
    if object.get("type").and_then(Value::as_str) != Some("Feature") {
        return Err(response_error("item_type"));
    }
    let version = object
        .get("stac_version")
        .and_then(Value::as_str)
        .filter(|value| route.accepts_version(value))
        .ok_or_else(|| response_error("stac_version"))?;
    if object.get("collection").and_then(Value::as_str)
        != Some(binding.native_collection_id.as_str())
    {
        return Err(identity_error("collection"));
    }
    let item_id = object
        .get("id")
        .and_then(Value::as_str)
        .and_then(valid_item_text)
        .ok_or_else(|| identity_error("item_id"))?;
    let properties = object
        .get("properties")
        .and_then(Value::as_object)
        .filter(|properties| properties.len() <= MAX_ITEM_PROPERTIES)
        .ok_or_else(|| response_error("properties"))?;
    if contains_sensitive_key(&Value::Object(properties.clone())) {
        return Err(response_error("sensitive_properties"));
    }
    let acquisition_datetime = properties
        .get("datetime")
        .and_then(Value::as_str)
        .map(normalized_utc)
        .transpose()
        .map_err(|_| response_error("datetime"))?
        .ok_or_else(|| response_error("datetime"))?;
    let product_id = identity_value(properties, binding.product_id_property.as_deref(), item_id)?;
    let display_name = identity_value(
        properties,
        binding.display_name_property.as_deref(),
        item_id,
    )?;
    let footprint = object
        .get("geometry")
        .and_then(Value::as_object)
        .cloned()
        .ok_or_else(|| response_error("geometry"))?;
    let product_key = provider_product_key(
        &binding.provider_id,
        &binding.native_collection_id,
        &product_id,
    )
    .map_err(|_| identity_error("provider_product_key"))?;
    let assets_object = object
        .get("assets")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if assets_object.len() > MAX_ITEM_ASSETS {
        return Err(asset_error("asset_count"));
    }
    if assets_object
        .keys()
        .any(|key| valid_item_text(key).is_none() || is_sensitive_key(key))
    {
        return Err(asset_error("asset_key"));
    }
    let assets = map_assets(binding, &product_key, item_id, &assets_object)?;
    let mapped_keys = binding
        .asset_role_bindings
        .iter()
        .map(|asset| asset.stac_asset_key.as_str())
        .collect::<HashSet<_>>();
    let mut unmapped_asset_keys = assets_object
        .keys()
        .filter(|key| !mapped_keys.contains(key.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    unmapped_asset_keys.sort();

    let extensions = parse_extensions(object.get("stac_extensions"))?;
    let link_relations = parse_link_relations(object.get("links"))?;
    let mut mapped_properties = Map::from_iter([
        (
            "provider_product_key".to_owned(),
            Value::String(product_key),
        ),
        (
            "provider_product_id".to_owned(),
            Value::String(product_id.clone()),
        ),
        (
            "collection_profile_id".to_owned(),
            Value::String(binding.collection_profile_id.clone()),
        ),
        ("stac_item_id".to_owned(), Value::String(item_id.to_owned())),
        (
            "stac_properties".to_owned(),
            Value::Object(properties.clone()),
        ),
    ]);
    if let Some(title) = properties.get("title").and_then(Value::as_str) {
        if valid_item_text(title).is_some() {
            mapped_properties.insert("title".to_owned(), Value::String(title.to_owned()));
        }
    }
    let payload = Map::from_iter([(
        "stac".to_owned(),
        json!({
            "version": version,
            "extensions": extensions,
            "link_relations": link_relations,
            "unmapped_asset_keys": unmapped_asset_keys,
        }),
    )]);
    let product = ProviderProduct {
        provider_id: binding.provider_id.clone(),
        product_id,
        display_name,
        source_kind: binding.source_kind.clone(),
        product_kind: binding.product_kind.clone(),
        acquisition_datetime: Some(acquisition_datetime),
        footprint_geojson: Some(footprint),
        footprint_bbox: None,
        assets,
        properties: std::mem::take(&mut mapped_properties),
        payload,
    };
    product
        .validate()
        .map_err(|_| response_error("provider_product"))?;
    Ok(product)
}

pub fn map_stac_complete_search_response(
    route: &StacRouteProfile,
    binding: &StacCollectionBinding,
    query: &CatalogSearchQuery,
    page: &Value,
) -> ProviderResult<CatalogSearchResponse> {
    let query = &insar_core::clamp_catalogue_search_query(query);
    binding.validate(route)?;
    validate_query_scope(binding, query)?;
    let object = page
        .as_object()
        .ok_or_else(|| response_error("item_collection"))?;
    if object.get("type").and_then(Value::as_str) != Some("FeatureCollection") {
        return Err(response_error("item_collection_type"));
    }
    let features = object
        .get("features")
        .and_then(Value::as_array)
        .filter(|features| features.len() <= route.page_cap as usize)
        .ok_or_else(|| response_error("features"))?;
    if object
        .get("numberReturned")
        .is_some_and(|value| value.as_u64() != Some(features.len() as u64))
    {
        return Err(response_error("number_returned"));
    }
    let mut unique = BTreeMap::new();
    for feature in features {
        let product = map_stac_item(route, binding, feature)?;
        if unique.insert(product.product_id.clone(), product).is_some() {
            return Err(identity_error("duplicate_product_id"));
        }
    }
    let fetched_count = features.len() as u64;
    let unique_count = unique.len() as u64;
    let matched_count = object
        .get("numberMatched")
        .map(|value| {
            value
                .as_u64()
                .filter(|matched| *matched >= unique_count)
                .ok_or_else(|| response_error("number_matched"))
        })
        .transpose()?;
    let fingerprint = query_fingerprint(route, binding, query)?;
    let base_body = build_base_search_body(route, binding, query)?;
    let provider_next =
        parse_next_cursor(route, &fingerprint, object.get("links"), Some(&base_body))?;
    let remote_exhausted = provider_next.is_none();
    let working_set_satisfied = query
        .result_limit
        .map(|target| unique_count >= target || remote_exhausted)
        .unwrap_or(remote_exhausted);
    if !working_set_satisfied {
        return Err(response_error("working_set_incomplete"));
    }
    let mut products = unique.into_values().collect::<Vec<_>>();
    if let Some(target) = query.result_limit {
        products.truncate(target as usize);
    }
    let returned_count = products.len() as u64;
    let response = CatalogSearchResponse {
        collection: binding.collection_descriptor(),
        products,
        stats: CatalogSearchStats {
            requested_limit: query.result_limit,
            matched_count,
            request_count: 1,
            transport_part_count: 1,
            fetched_record_count: fetched_count,
            mapped_record_count: fetched_count,
            profile_filtered_record_count: 0,
            exact_duplicate_record_count: 0,
            unique_provider_product_count: unique_count,
            exact_matched_count: unique_count,
            returned_count,
            remote_exhausted,
            working_set_satisfied,
            elapsed_ms: 0,
            // This response is suitable for freezing. A finite fulfilled target
            // suppresses the Provider continuation while retaining the truthful
            // remote_exhausted=false fact.
            truncated: false,
        },
        next_cursor: None,
        item_errors: Vec::new(),
    };
    response
        .validate()
        .map_err(|_| response_error("catalogue_contract"))?;
    Ok(response)
}

fn validate_query_scope(
    binding: &StacCollectionBinding,
    query: &CatalogSearchQuery,
) -> ProviderResult<()> {
    query
        .validate()
        .map_err(|_| query_error("catalogue_contract"))?;
    if query.logical_dataset != binding.logical_dataset
        || query.provider_id != binding.provider_id
        || query.native_collection_id != binding.native_collection_id
    {
        return Err(query_error("collection_scope"));
    }
    Ok(())
}

fn build_base_search_body(
    route: &StacRouteProfile,
    binding: &StacCollectionBinding,
    query: &CatalogSearchQuery,
) -> ProviderResult<Map<String, Value>> {
    let mut body = Map::new();
    body.insert(
        "collections".to_owned(),
        json!([binding.native_collection_id]),
    );
    if let Some(aoi) = &query.aoi_geojson {
        body.insert("intersects".to_owned(), Value::Object(aoi.clone()));
    }
    if let Some(datetime) = query.filters.get("datetime") {
        body.insert(
            "datetime".to_owned(),
            Value::String(datetime_filter(datetime)?),
        );
    }
    let requested_page = query
        .result_limit
        .unwrap_or(route.page_cap)
        .min(route.page_cap);
    body.insert("limit".to_owned(), Value::Number(requested_page.into()));

    let bindings = binding
        .queryables
        .iter()
        .map(|binding| (binding.sceneport_filter_key.as_str(), binding))
        .collect::<BTreeMap<_, _>>();
    let mut query_object = Map::new();
    for (key, value) in &query.filters {
        if key == "datetime" {
            continue;
        }
        let declared = bindings
            .get(key.as_str())
            .ok_or_else(|| query_error("undeclared_filter"))?;
        let value = queryable_value(declared, value)?;
        query_object.insert(
            declared.stac_property.clone(),
            json!({ declared.operator.as_str(): value }),
        );
    }
    if !query_object.is_empty() {
        body.insert("query".to_owned(), Value::Object(query_object));
    }
    Ok(body)
}

fn queryable_value(binding: &StacQueryableBinding, value: &Value) -> ProviderResult<Value> {
    match binding.value_kind {
        StacQueryValueKind::String => value
            .as_str()
            .and_then(valid_item_text)
            .map(|value| Value::String(value.to_owned()))
            .ok_or_else(|| query_error("filter_string")),
        StacQueryValueKind::StringList => {
            let values = value
                .as_array()
                .filter(|values| !values.is_empty() && values.len() <= 64)
                .ok_or_else(|| query_error("filter_string_list"))?;
            let mut unique = BTreeSet::new();
            for value in values {
                let value = value
                    .as_str()
                    .and_then(valid_item_text)
                    .ok_or_else(|| query_error("filter_string_list"))?;
                if !unique.insert(value.to_owned()) {
                    return Err(query_error("filter_string_list_duplicate"));
                }
            }
            Ok(Value::Array(
                unique.into_iter().map(Value::String).collect(),
            ))
        }
        StacQueryValueKind::Number => value
            .as_f64()
            .filter(|value| value.is_finite())
            .and_then(Number::from_f64)
            .map(Value::Number)
            .ok_or_else(|| query_error("filter_number")),
    }
}

fn datetime_filter(value: &Value) -> ProviderResult<String> {
    let object = value.as_object().ok_or_else(|| query_error("datetime"))?;
    if object.len() != 2 {
        return Err(query_error("datetime"));
    }
    let start = object
        .get("start")
        .and_then(Value::as_str)
        .ok_or_else(|| query_error("datetime_start"))?;
    let end = object
        .get("end")
        .and_then(Value::as_str)
        .ok_or_else(|| query_error("datetime_end"))?;
    let start = normalized_utc(start).map_err(|_| query_error("datetime_start"))?;
    let end = normalized_utc(end).map_err(|_| query_error("datetime_end"))?;
    let start_time =
        DateTime::parse_from_rfc3339(&start).map_err(|_| query_error("datetime_start"))?;
    let end_time = DateTime::parse_from_rfc3339(&end).map_err(|_| query_error("datetime_end"))?;
    if start_time > end_time {
        return Err(query_error("datetime_order"));
    }
    Ok(format!("{start}/{end}"))
}

fn normalized_utc(value: &str) -> Result<String, ()> {
    // Providers may pad fractional seconds (e.g. .024000Z). Canonicalizing
    // that spelling does not alter the instant; the raw property is retained.
    // Reject sub-nanosecond input before chrono can truncate excess digits.
    let local = value
        .strip_suffix('Z')
        .or_else(|| value.strip_suffix("+00:00"))
        .ok_or(())?;
    let (_, time) = local.split_once('T').ok_or(())?;
    if let Some((_, fraction)) = time.split_once('.') {
        if !(1..=9).contains(&fraction.len()) || !fraction.bytes().all(|b| b.is_ascii_digit()) {
            return Err(());
        }
    }
    let parsed = DateTime::parse_from_rfc3339(value).map_err(|_| ())?;
    if parsed.offset().local_minus_utc() != 0 {
        return Err(());
    }
    Ok(parsed.to_rfc3339_opts(SecondsFormat::AutoSi, true))
}

fn identity_value(
    properties: &Map<String, Value>,
    property: Option<&str>,
    item_id: &str,
) -> ProviderResult<String> {
    let Some(property) = property else {
        return Ok(item_id.to_owned());
    };
    properties
        .get(property)
        .and_then(Value::as_str)
        .and_then(valid_item_text)
        .map(str::to_owned)
        .ok_or_else(|| identity_error("identity_property"))
}

fn map_assets(
    binding: &StacCollectionBinding,
    product_key: &str,
    item_id: &str,
    assets: &Map<String, Value>,
) -> ProviderResult<Vec<ProviderAsset>> {
    let mut mapped = Vec::new();
    for rule in &binding.asset_role_bindings {
        let Some(value) = assets.get(&rule.stac_asset_key) else {
            if rule.required {
                return Err(asset_error("required_asset_missing"));
            }
            continue;
        };
        let object = value
            .as_object()
            .ok_or_else(|| asset_error("asset_object"))?;
        let href = object
            .get("href")
            .and_then(Value::as_str)
            .ok_or_else(|| asset_error("asset_href"))?;
        let url = validate_asset_url(href, &rule.allowed_hosts)?;
        let native_roles = parse_native_roles(object.get("roles"), &rule.stac_asset_key)?;
        let file_name = url
            .path_segments()
            .and_then(|mut segments| segments.rfind(|segment| !segment.is_empty()))
            .and_then(valid_item_text)
            .map(str::to_owned);
        let media_type = object
            .get("type")
            .map(|value| {
                value
                    .as_str()
                    .and_then(valid_item_text)
                    .map(str::to_owned)
                    .ok_or_else(|| asset_error("asset_media_type"))
            })
            .transpose()?;
        let size_bytes = object
            .get("file:size")
            .map(|value| {
                value
                    .as_u64()
                    .and_then(|value| i64::try_from(value).ok())
                    .ok_or_else(|| asset_error("asset_size"))
            })
            .transpose()?;
        let stable_key = stable_asset_key(product_key, &rule.stac_asset_key)
            .map_err(|_| asset_error("stable_asset_key"))?;
        let asset = ProviderAsset {
            asset_key: Some(rule.stac_asset_key.clone()),
            stable_asset_key: Some(stable_key),
            // Provider discovery alone does not grant transfer authority.
            access_descriptor: None,
            role: rule.role.clone(),
            native_roles,
            url: Some(url.to_string()),
            file_name,
            media_type,
            size_bytes,
        };
        asset
            .validate()
            .map_err(|_| asset_error("provider_asset"))?;
        mapped.push(asset);
    }
    mapped.sort_by(|left, right| left.asset_key.cmp(&right.asset_key));
    if item_id.is_empty() {
        return Err(identity_error("item_id"));
    }
    Ok(mapped)
}

fn validate_asset_url(value: &str, allowed_hosts: &[String]) -> ProviderResult<Url> {
    let url = Url::parse(value).map_err(|_| asset_error("asset_href"))?;
    let host = url
        .host_str()
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| asset_error("asset_host"))?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.port_or_known_default() != Some(443)
        || !allowed_hosts
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(&host))
    {
        return Err(asset_error("asset_scope"));
    }
    Ok(url)
}

fn parse_native_roles(value: Option<&Value>, fallback: &str) -> ProviderResult<Vec<String>> {
    let Some(value) = value else {
        return Ok(vec![fallback.to_owned()]);
    };
    let roles = value
        .as_array()
        .filter(|roles| !roles.is_empty() && roles.len() <= 16)
        .ok_or_else(|| asset_error("asset_roles"))?;
    let mut result = BTreeSet::new();
    for role in roles {
        let role = role
            .as_str()
            .and_then(valid_item_text)
            .ok_or_else(|| asset_error("asset_roles"))?;
        if !result.insert(role.to_owned()) {
            return Err(asset_error("asset_roles_duplicate"));
        }
    }
    Ok(result.into_iter().collect())
}

fn parse_extensions(value: Option<&Value>) -> ProviderResult<Vec<String>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let values = value
        .as_array()
        .filter(|values| values.len() <= MAX_EXTENSIONS)
        .ok_or_else(|| response_error("stac_extensions"))?;
    let mut result = BTreeSet::new();
    for value in values {
        let value = value
            .as_str()
            .and_then(valid_item_text)
            .ok_or_else(|| response_error("stac_extensions"))?;
        let url = Url::parse(value).map_err(|_| response_error("stac_extensions"))?;
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.port_or_known_default() != Some(443)
        {
            return Err(response_error("stac_extensions"));
        }
        if !result.insert(value.to_owned()) {
            return Err(response_error("stac_extensions_duplicate"));
        }
    }
    Ok(result.into_iter().collect())
}

fn parse_link_relations(value: Option<&Value>) -> ProviderResult<Vec<String>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let links = value
        .as_array()
        .filter(|links| links.len() <= MAX_ITEM_LINKS)
        .ok_or_else(|| response_error("links"))?;
    let mut relations = BTreeSet::new();
    for link in links {
        let object = link.as_object().ok_or_else(|| response_error("link"))?;
        let relation = object
            .get("rel")
            .and_then(Value::as_str)
            .and_then(valid_item_text)
            .ok_or_else(|| response_error("link_rel"))?;
        relations.insert(relation.to_owned());
    }
    Ok(relations.into_iter().collect())
}

/// `request_body` is the original first-page body as sent (including any
/// `sortby`); a POST next link with `merge: true` is merged onto it with the
/// link's fields winning (STAC API Item Search pagination). Merging onto the
/// previous page's body would carry stale fields from earlier links.
fn parse_next_cursor(
    route: &StacRouteProfile,
    fingerprint: &str,
    links_value: Option<&Value>,
    request_body: Option<&Map<String, Value>>,
) -> ProviderResult<Option<String>> {
    let Some(value) = links_value else {
        return Ok(None);
    };
    let links = value
        .as_array()
        .filter(|links| links.len() <= MAX_ITEM_LINKS)
        .ok_or_else(|| response_error("links"))?;
    let mut next = None;
    for link in links {
        let object = link.as_object().ok_or_else(|| response_error("link"))?;
        if object.get("rel").and_then(Value::as_str) != Some("next") {
            continue;
        }
        if next.is_some() {
            return Err(cursor_error("multiple_next_links"));
        }
        next = Some(wrap_next_cursor(route, fingerprint, object, request_body)?);
    }
    Ok(next)
}

fn wrap_next_cursor(
    route: &StacRouteProfile,
    fingerprint: &str,
    link: &Map<String, Value>,
    request_body: Option<&Map<String, Value>>,
) -> ProviderResult<String> {
    if link.contains_key("headers") {
        return Err(cursor_error("next_headers"));
    }
    let href = link
        .get("href")
        .and_then(Value::as_str)
        .ok_or_else(|| cursor_error("next_href"))?;
    let url = validate_continuation_url(route, href)?;
    let method =
        StacHttpMethod::parse(link.get("method").and_then(Value::as_str).unwrap_or("GET"))?;
    let merge = match link.get("merge") {
        None => false,
        Some(value) => value.as_bool().ok_or_else(|| cursor_error("next_merge"))?,
    };
    let body = match method {
        StacHttpMethod::Get => {
            if merge {
                return Err(cursor_error("get_merge"));
            }
            if link.get("body").is_some_and(|body| !body.is_null()) {
                return Err(cursor_error("get_body"));
            }
            None
        }
        StacHttpMethod::Post => {
            let link_body = match link.get("body") {
                None if merge => Map::new(),
                value => value
                    .and_then(Value::as_object)
                    .cloned()
                    .ok_or_else(|| cursor_error("post_body"))?,
            };
            let body = if merge {
                let mut merged = request_body
                    .cloned()
                    .ok_or_else(|| cursor_error("merge_without_request_body"))?;
                merged.extend(link_body);
                merged
            } else {
                link_body
            };
            if contains_sensitive_key(&Value::Object(body.clone())) {
                return Err(cursor_error("sensitive_body"));
            }
            Some(body)
        }
    };
    let href_path_query = url[Position::BeforePath..Position::AfterQuery].to_owned();
    let envelope = CursorEnvelope {
        version: CURSOR_VERSION.to_owned(),
        fingerprint: fingerprint.to_owned(),
        href_path_query,
        method: method.as_str().to_owned(),
        body,
    };
    let encoded = serde_json::to_vec(&envelope).map_err(|_| cursor_error("cursor_json"))?;
    if encoded.len() > MAX_CURSOR_BYTES {
        return Err(cursor_error("cursor_size"));
    }
    Ok(hex_encode(&encoded))
}

fn unwrap_cursor(
    route: &StacRouteProfile,
    fingerprint: &str,
    cursor: &str,
) -> ProviderResult<StacContinuation> {
    if cursor.len() > MAX_CURSOR_BYTES * 2 {
        return Err(cursor_error("cursor_size"));
    }
    let decoded = hex_decode(cursor).ok_or_else(|| cursor_error("cursor_encoding"))?;
    let envelope: CursorEnvelope =
        serde_json::from_slice(&decoded).map_err(|_| cursor_error("cursor_json"))?;
    if envelope.version != CURSOR_VERSION || envelope.fingerprint != fingerprint {
        return Err(cursor_error("cursor_scope"));
    }
    if !envelope.href_path_query.starts_with('/')
        || envelope.href_path_query.contains('#')
        || envelope.href_path_query.chars().count() > 16 * 1024
    {
        return Err(cursor_error("cursor_href"));
    }
    let endpoint = route.endpoint_url()?;
    let url = endpoint
        .join(&envelope.href_path_query)
        .map_err(|_| cursor_error("cursor_href"))?;
    validate_continuation_url(route, url.as_str())?;
    let method = StacHttpMethod::parse(&envelope.method)?;
    match (&method, &envelope.body) {
        (StacHttpMethod::Get, None) => {}
        (StacHttpMethod::Post, Some(body))
            if !contains_sensitive_key(&Value::Object(body.clone())) => {}
        _ => return Err(cursor_error("cursor_body")),
    }
    Ok(StacContinuation {
        url,
        method,
        body: envelope.body,
    })
}

fn validate_continuation_url(route: &StacRouteProfile, value: &str) -> ProviderResult<Url> {
    let endpoint = route.endpoint_url()?;
    let url = endpoint
        .join(value)
        .map_err(|_| cursor_error("next_href"))?;
    let endpoint_path = endpoint.path();
    if url.scheme() != endpoint.scheme()
        || url.host_str() != endpoint.host_str()
        || url.port_or_known_default() != endpoint.port_or_known_default()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || !url.path().starts_with(endpoint_path)
    {
        return Err(cursor_error("next_scope"));
    }
    Ok(url)
}

fn query_fingerprint(
    route: &StacRouteProfile,
    binding: &StacCollectionBinding,
    query: &CatalogSearchQuery,
) -> ProviderResult<String> {
    let value = json!({
        "route": {
            "provider_id": route.provider_id,
            "endpoint": route.endpoint,
            "search_path": route.search_path,
        },
        "binding": {
            "logical_dataset": binding.logical_dataset,
            "provider_id": binding.provider_id,
            "native_collection_id": binding.native_collection_id,
            "collection_profile_id": binding.collection_profile_id,
        },
        "query": {
            "logical_dataset": query.logical_dataset,
            "provider_id": query.provider_id,
            "native_collection_id": query.native_collection_id,
            "aoi_geojson": query.aoi_geojson,
            "result_limit": query.result_limit,
            "filters": query.filters,
        }
    });
    let canonical = canonicalize(value);
    let bytes = serde_json::to_vec(&canonical).map_err(|_| query_error("query_fingerprint"))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn canonicalize(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let sorted = map
                .into_iter()
                .map(|(key, value)| (key, canonicalize(value)))
                .collect::<BTreeMap<_, _>>();
            Value::Object(Map::from_iter(sorted))
        }
        Value::Array(values) => Value::Array(values.into_iter().map(canonicalize).collect()),
        other => other,
    }
}

fn contains_sensitive_key(value: &Value) -> bool {
    match value {
        Value::Object(object) => object
            .iter()
            .any(|(key, value)| is_sensitive_key(key) || contains_sensitive_key(value)),
        Value::Array(values) => values.iter().any(contains_sensitive_key),
        _ => false,
    }
}

fn is_sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "password",
        "access_token",
        "refresh_token",
        "authorization",
        "cookie",
        "secret",
        "credential",
        "signed_url",
    ]
    .iter()
    .any(|needle| key.contains(needle))
}

fn validate_property_name(value: &str) -> Result<(), &'static str> {
    validate_text(value, "property")?;
    if is_sensitive_key(value) || value.contains(['/', '\\', '?', '#']) {
        return Err("property");
    }
    Ok(())
}

fn validate_host(value: &str) -> Result<(), &'static str> {
    validate_text(value, "host")?;
    if value.contains(['/', '\\', ':', '@']) || value.starts_with('.') || value.ends_with('.') {
        return Err("host");
    }
    Ok(())
}

fn validate_text(value: &str, reason: &'static str) -> Result<(), &'static str> {
    let chars = value.chars().count();
    if !(1..=MAX_TEXT_CHARS).contains(&chars) || value.chars().any(char::is_control) {
        return Err(reason);
    }
    Ok(())
}

fn valid_item_text(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    let chars = trimmed.chars().count();
    (trimmed == value
        && (1..=MAX_TEXT_CHARS).contains(&chars)
        && !trimmed.chars().any(char::is_control))
    .then_some(trimmed)
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn hex_decode(value: &str) -> Option<Vec<u8>> {
    if value.is_empty() || !value.len().is_multiple_of(2) {
        return None;
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| Some((hex_nibble(pair[0])? << 4) | hex_nibble(pair[1])?))
        .collect()
}

fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

fn error(code: &str, message: &str, reason: &str) -> ContractError {
    ContractError::new(
        code,
        message,
        Map::from_iter([("reason".to_owned(), Value::String(reason.to_owned()))]),
    )
    .expect("bounded STAC adapter error is safe")
}

fn binding_error(reason: &str) -> ContractError {
    error("STAC_BINDING_INVALID", "STAC 数据源配置无效", reason)
}

fn query_error(reason: &str) -> ContractError {
    error("STAC_QUERY_INVALID", "STAC 目录查询无效", reason)
}

fn cursor_error(reason: &str) -> ContractError {
    error(
        "STAC_CURSOR_INVALID",
        "STAC 续页标识无效或与原始查询不匹配",
        reason,
    )
}

fn response_error(reason: &str) -> ContractError {
    error("STAC_RESPONSE_INVALID", "STAC 目录响应无效", reason)
}

fn identity_error(reason: &str) -> ContractError {
    error(
        "STAC_IDENTITY_MISMATCH",
        "STAC 产品身份与已审核 Collection 不一致",
        reason,
    )
}

fn asset_error(reason: &str) -> ContractError {
    error("STAC_ASSET_INVALID", "STAC 产品资产无效", reason)
}

#[cfg(test)]
#[path = "stac_tests.rs"]
mod tests;
