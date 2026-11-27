use std::collections::BTreeMap;

use chrono::{DateTime, SecondsFormat, Utc};
use insar_core::{CatalogCollectionDescriptor, CatalogSearchQuery, ContractError, ProviderResult};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use url::Url;

use crate::catalog::sentinel2_filters::{decode_sentinel2_spacecraft, Sentinel2Platform};
use crate::catalog::{CollectionProfileRegistry, DatasetRoutePolicyRegistry, RouteUse};

pub const CDSE_ODATA_PATH: &str = "Products";
pub const CDSE_ODATA_CURSOR_VERSION: &str = "cdse-odata.v1";
pub const CDSE_SENTINEL2_MAX_PAGE_SIZE: u64 = 100;

fn transport_page_size(result_limit: Option<u64>) -> u64 {
    result_limit
        .unwrap_or(CDSE_SENTINEL2_MAX_PAGE_SIZE)
        .min(CDSE_SENTINEL2_MAX_PAGE_SIZE)
}
pub const CDSE_SENTINEL2_MAX_NEXT_LINK_BYTES: usize = 8 * 1024;
pub const CDSE_SENTINEL2_MAX_CURSOR_BYTES: usize = 24 * 1024;
const MAX_POLYGONS: usize = 32;
const MAX_RINGS: usize = 128;
const MAX_POSITIONS: usize = 4096;
const MAX_PRODUCTION_POSITIONS: usize = 48;
const MAX_ENCODED_QUERY_BYTES: usize = 16 * 1024;

fn max_positions_for_use(use_case: RouteUse) -> usize {
    match use_case {
        RouteUse::OfflineFixture => MAX_POSITIONS,
        RouteUse::Production => MAX_PRODUCTION_POSITIONS,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sentinel2SearchFacts {
    pub product_type: String,
    pub product_types: Vec<String>,
    pub(crate) spacecraft: Vec<Sentinel2Platform>,
    pub cloud_cover_lte: Option<f64>,
    pub start: Option<DateTime<Utc>>,
    pub end: Option<DateTime<Utc>>,
    pub result_limit: Option<u64>,
    route_id: &'static str,
    profile_id: &'static str,
    collection_echo_policy: CollectionEchoPolicy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CollectionEchoPolicy {
    RequiredForOfflineFixture,
    OptionalForProduction,
}

impl Sentinel2SearchFacts {
    pub(crate) fn permits_missing_collection_echo(&self) -> bool {
        matches!(
            self.collection_echo_policy,
            CollectionEchoPolicy::OptionalForProduction
        )
    }

    pub(crate) fn collection_descriptor(&self) -> CatalogCollectionDescriptor {
        let profile = CollectionProfileRegistry
            .get(self.profile_id)
            .expect("validated Sentinel-2 facts retain a registered profile");
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
}

#[derive(Clone, Debug, PartialEq)]
pub struct ODataRequest {
    path_and_query: String,
    fingerprint: String,
    scope_options: BTreeMap<String, String>,
    continuation: Option<Url>,
    current_skip: u64,
    facts: Sentinel2SearchFacts,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidatedContinuation {
    pub url: Url,
    pub skip: u64,
}

impl ODataRequest {
    pub fn path_and_query(&self) -> &str {
        &self.path_and_query
    }

    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    pub fn continuation(&self) -> Option<&Url> {
        self.continuation.as_ref()
    }

    pub fn facts(&self) -> &Sentinel2SearchFacts {
        &self.facts
    }

    pub fn page_size(&self) -> u64 {
        self.scope_options["$top"]
            .parse()
            .expect("validated page size is numeric")
    }

    pub fn current_skip(&self) -> u64 {
        self.current_skip
    }

    pub fn initial_url(&self, endpoint: &Url) -> ProviderResult<Url> {
        endpoint
            .join(&self.path_and_query)
            .map_err(|_| query_error("CDSE_ODATA_ENDPOINT_INVALID", "离线 OData 端点无效", "join"))
    }

    pub(crate) fn url_for_skip(
        &self,
        endpoint: &Url,
        skip: u64,
        include_count: bool,
    ) -> ProviderResult<Url> {
        let mut pairs = Vec::with_capacity(self.scope_options.len() + 1);
        for (key, value) in &self.scope_options {
            if !include_count && key == "$count" {
                continue;
            }
            pairs.push((key.as_str(), value.clone()));
        }
        if skip > 0 {
            pairs.push(("$skip", skip.to_string()));
        }
        let query_string = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(pairs.iter().map(|(k, v)| (*k, v.as_str())))
            .finish();
        let path_and_query = format!("{CDSE_ODATA_PATH}?{query_string}");
        endpoint
            .join(&path_and_query)
            .map_err(|_| query_error("CDSE_ODATA_ENDPOINT_INVALID", "离线 OData 端点无效", "join"))
    }

    pub(crate) fn validate_next_link_skip_for_endpoint(
        &self,
        endpoint: &Url,
        next_link_raw: &str,
    ) -> ProviderResult<u64> {
        if next_link_raw.len() > CDSE_SENTINEL2_MAX_NEXT_LINK_BYTES {
            return Err(query_error(
                "CDSE_ODATA_CURSOR_UNTRUSTED",
                "Sentinel-2 续页地址超出离线限制",
                "next_link_size",
            ));
        }
        let continuation = Url::parse(next_link_raw).map_err(|_| {
            query_error(
                "CDSE_ODATA_CURSOR_UNTRUSTED",
                "Sentinel-2 续页地址无效",
                "parse",
            )
        })?;
        if continuation.scheme() != endpoint.scheme()
            || continuation.host_str() != endpoint.host_str()
            || continuation.port_or_known_default() != endpoint.port_or_known_default()
            || !continuation.username().is_empty()
            || continuation.password().is_some()
            || continuation.fragment().is_some()
        {
            return Err(query_error(
                "CDSE_ODATA_CURSOR_UNTRUSTED",
                "Sentinel-2 续页地址不受信任",
                "origin",
            ));
        }
        let expected_path = endpoint.join(CDSE_ODATA_PATH).map_err(|_| {
            query_error("CDSE_ODATA_ENDPOINT_INVALID", "离线 OData 端点无效", "path")
        })?;
        if continuation.path() != expected_path.path() {
            return Err(query_error(
                "CDSE_ODATA_CURSOR_UNTRUSTED",
                "Sentinel-2 续页路径不受信任",
                "path",
            ));
        }
        let mut actual = BTreeMap::new();
        for (key, value) in continuation.query_pairs() {
            if is_forbidden_parameter(&key)
                || actual.insert(key.to_string(), value.to_string()).is_some()
            {
                return Err(query_error(
                    "CDSE_ODATA_CURSOR_UNTRUSTED",
                    "Sentinel-2 续页参数不受信任",
                    "parameters",
                ));
            }
            if key != "$skip" && self.scope_options.get(key.as_ref()) != Some(&value.into_owned()) {
                return Err(query_error(
                    "CDSE_ODATA_CURSOR_UNTRUSTED",
                    "Sentinel-2 续页查询范围不匹配",
                    "scope",
                ));
            }
        }
        numeric_skip(&continuation)
    }
}

fn error(code: &str, user_message: &str, reason: &str) -> ContractError {
    ContractError::new(
        code,
        user_message,
        Map::from_iter([("reason".to_string(), Value::String(reason.to_string()))]),
    )
    .expect("bounded CDSE OData context is safe")
}

fn query_error(code: &str, message: &str, reason: &str) -> ContractError {
    error(code, message, reason)
}

fn require_exact_object<'a>(
    value: Option<&'a Value>,
    field: &str,
) -> ProviderResult<&'a Map<String, Value>> {
    value.and_then(Value::as_object).ok_or_else(|| {
        query_error(
            "CDSE_ODATA_FILTER_INVALID",
            "Sentinel-2 筛选条件无效",
            field,
        )
    })
}

pub(crate) fn normalize_utc(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::AutoSi, true)
}

pub(crate) fn parse_utc_text(value: &str, field: &str) -> ProviderResult<DateTime<Utc>> {
    let parsed = DateTime::parse_from_rfc3339(value)
        .map_err(|_| query_error("CDSE_ODATA_ITEM_INVALID", "Sentinel-2 时间字段无效", field))?;
    if parsed.offset().local_minus_utc() != 0 {
        return Err(query_error(
            "CDSE_ODATA_ITEM_INVALID",
            "Sentinel-2 时间必须使用 UTC",
            field,
        ));
    }
    Ok(parsed.with_timezone(&Utc))
}

fn parse_utc(value: Option<&Value>, field: &str) -> ProviderResult<DateTime<Utc>> {
    let value = value.and_then(Value::as_str).ok_or_else(|| {
        query_error(
            "CDSE_ODATA_FILTER_INVALID",
            "Sentinel-2 时间条件无效",
            field,
        )
    })?;
    parse_utc_text(value, field).map_err(|_| {
        query_error(
            "CDSE_ODATA_FILTER_INVALID",
            "Sentinel-2 时间条件无效",
            field,
        )
    })
}

pub fn decode_sentinel2_search_facts(
    query: &CatalogSearchQuery,
) -> ProviderResult<Sentinel2SearchFacts> {
    decode_sentinel2_search_facts_for_use(query, RouteUse::OfflineFixture)
}

const SENTINEL3_PLATFORMS: [Sentinel2Platform; 2] = [
    Sentinel2Platform {
        value: "sentinel_3a",
        prefix: "S3A_",
        stac_name: "Sentinel-3A",
    },
    Sentinel2Platform {
        value: "sentinel_3b",
        prefix: "S3B_",
        stac_name: "Sentinel-3B",
    },
];

fn decode_sentinel3_spacecraft(
    value: Option<&Value>,
) -> Result<Vec<Sentinel2Platform>, &'static str> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let values = value
        .as_array()
        .filter(|v| !v.is_empty() && v.len() <= SENTINEL3_PLATFORMS.len())
        .ok_or("spacecraft")?;
    let mut selected = Vec::with_capacity(values.len());
    for value in values {
        let platform = SENTINEL3_PLATFORMS
            .iter()
            .find(|p| value.as_str() == Some(p.value))
            .ok_or("spacecraft")?;
        if selected.contains(platform) {
            return Err("spacecraft");
        }
        selected.push(*platform);
    }
    selected.sort_by_key(|p| p.value);
    Ok(selected)
}

pub(crate) fn decode_sentinel2_search_facts_for_use(
    query: &CatalogSearchQuery,
    use_case: RouteUse,
) -> ProviderResult<Sentinel2SearchFacts> {
    query.validate().map_err(|_| {
        query_error(
            "CDSE_ODATA_QUERY_INVALID",
            "Sentinel 目录请求无效",
            "generic_query",
        )
    })?;
    let is_sentinel3 = query.native_collection_id == "SENTINEL-3";
    let (route_id, profile) = if is_sentinel3 {
        let profile = CollectionProfileRegistry
            .for_logical_dataset(&query.logical_dataset)
            .or_else(|| CollectionProfileRegistry.get("cdse.sentinel-3.standard.collection.v1"))
            .ok_or_else(|| {
                query_error(
                    "CDSE_ODATA_ROUTE_INVALID",
                    "Sentinel-3 集合档案无效",
                    "profile",
                )
            })?;
        if matches!(use_case, RouteUse::Production) && !profile.production_enabled {
            return Err(query_error(
                "CDSE_ODATA_ROUTE_INVALID",
                "Sentinel-3 生产路由已禁用",
                "production_disabled",
            ));
        }
        ("route.cdse.sentinel-3.standard.v1", profile)
    } else {
        let route = DatasetRoutePolicyRegistry
            .resolve(
                &query.logical_dataset,
                &query.provider_id,
                &query.native_collection_id,
                use_case,
            )
            .map_err(|_| {
                query_error(
                    "CDSE_ODATA_ROUTE_INVALID",
                    "Sentinel-2 目录路由无效",
                    "route",
                )
            })?;
        let profile = CollectionProfileRegistry
            .get(route.collection_profile_id)
            .ok_or_else(|| {
                query_error(
                    "CDSE_ODATA_ROUTE_INVALID",
                    "Sentinel-2 集合档案无效",
                    "profile",
                )
            })?;
        (route.route_id, profile)
    };
    if !(1..=4).contains(&query.filters.len())
        || !query.filters.contains_key("product_type")
        || query.filters.keys().any(|key| {
            !["product_type", "cloud_cover_lte", "datetime", "spacecraft"].contains(&key.as_str())
        })
    {
        return Err(query_error(
            "CDSE_ODATA_FILTER_INVALID",
            "Sentinel 筛选条件无效",
            "unknown_or_missing_filter",
        ));
    }
    let raw_types: Vec<&str> = match query.filters.get("product_type") {
        Some(Value::String(value)) => vec![value.as_str()],
        Some(Value::Array(values)) => {
            if values.is_empty() {
                return Err(query_error(
                    "CDSE_ODATA_FILTER_INVALID",
                    "Sentinel 产品类型列表不能为空",
                    "product_type",
                ));
            }
            let mut list = Vec::with_capacity(values.len());
            for value in values {
                let s = value.as_str().ok_or_else(|| {
                    query_error(
                        "CDSE_ODATA_FILTER_INVALID",
                        "Sentinel 产品类型无效",
                        "product_type",
                    )
                })?;
                list.push(s);
            }
            list
        }
        _ => {
            return Err(query_error(
                "CDSE_ODATA_FILTER_INVALID",
                "Sentinel 产品类型无效",
                "product_type",
            ));
        }
    };
    let mut product_types: Vec<String> = Vec::new();
    for pt in raw_types {
        let is_valid = if is_sentinel3 {
            [
                "OL_1_EFR___",
                "OL_1_ERR___",
                "OL_2_LFR___",
                "OL_2_LRR___",
                "SL_1_RBT___",
                "SL_2_LST___",
                "SR_1_SRA___",
                "SR_2_LAN___",
                profile.required_product_type,
            ]
            .contains(&pt)
        } else {
            pt == profile.required_product_type
        };
        if !is_valid {
            return Err(query_error(
                "CDSE_ODATA_FILTER_INVALID",
                "Sentinel 产品类型无效",
                "product_type",
            ));
        }
        if !product_types.iter().any(|existing| existing == pt) {
            product_types.push(pt.to_string());
        }
    }
    product_types.sort();
    let product_type = product_types.join(",");
    let cloud_cover_lte = match query.filters.get("cloud_cover_lte") {
        None => None,
        Some(Value::Number(value)) => {
            let value = value
                .as_f64()
                .filter(|value| value.is_finite())
                .ok_or_else(|| {
                    query_error(
                        "CDSE_ODATA_FILTER_INVALID",
                        "Sentinel 云量条件无效",
                        "cloud_cover_lte",
                    )
                })?;
            if !(0.0..=100.0).contains(&value) {
                return Err(query_error(
                    "CDSE_ODATA_FILTER_INVALID",
                    "Sentinel 云量条件无效",
                    "cloud_cover_lte",
                ));
            }
            Some(value)
        }
        Some(_) => {
            return Err(query_error(
                "CDSE_ODATA_FILTER_INVALID",
                "Sentinel 云量条件无效",
                "cloud_cover_lte",
            ))
        }
    };
    let (start, end) = match query.filters.get("datetime") {
        None => (None, None),
        Some(_) => {
            let datetime = require_exact_object(query.filters.get("datetime"), "datetime")?;
            if datetime.len() != 2 || datetime.keys().any(|key| key != "start" && key != "end") {
                return Err(query_error(
                    "CDSE_ODATA_FILTER_INVALID",
                    "Sentinel 时间条件无效",
                    "datetime",
                ));
            }
            let start = parse_utc(datetime.get("start"), "datetime.start")?;
            let end = parse_utc(datetime.get("end"), "datetime.end")?;
            if start >= end {
                return Err(query_error(
                    "CDSE_ODATA_FILTER_INVALID",
                    "Sentinel 时间区间必须为 [start, end)",
                    "datetime",
                ));
            }
            (Some(start), Some(end))
        }
    };
    let spacecraft = if is_sentinel3 {
        decode_sentinel3_spacecraft(query.filters.get("spacecraft")).map_err(|reason| {
            query_error(
                "CDSE_ODATA_FILTER_INVALID",
                "Sentinel-3 平台筛选条件无效",
                reason,
            )
        })?
    } else {
        decode_sentinel2_spacecraft(query.filters.get("spacecraft")).map_err(|reason| {
            query_error(
                "CDSE_ODATA_FILTER_INVALID",
                "Sentinel-2 平台筛选条件无效",
                reason,
            )
        })?
    };
    Ok(Sentinel2SearchFacts {
        product_type,
        product_types,
        spacecraft,
        cloud_cover_lte,
        start,
        end,
        result_limit: query.result_limit,
        route_id,
        profile_id: profile.profile_id,
        collection_echo_policy: match use_case {
            RouteUse::OfflineFixture => CollectionEchoPolicy::RequiredForOfflineFixture,
            RouteUse::Production => CollectionEchoPolicy::OptionalForProduction,
        },
    })
}

#[derive(Default)]
struct GeometryLimits {
    polygons: usize,
    rings: usize,
    positions: usize,
}

fn number(value: &Value, axis: &str) -> ProviderResult<f64> {
    let number = value
        .as_f64()
        .filter(|value| value.is_finite())
        .ok_or_else(|| query_error("CDSE_ODATA_AOI_INVALID", "Sentinel-2 AOI 坐标无效", axis))?;
    let valid = match axis {
        "longitude" => (-180.0..=180.0).contains(&number),
        _ => (-90.0..=90.0).contains(&number),
    };
    if !valid {
        return Err(query_error(
            "CDSE_ODATA_AOI_INVALID",
            "Sentinel-2 AOI 坐标超出 EPSG:4326 范围",
            axis,
        ));
    }
    Ok(number)
}

fn decimal(value: f64) -> String {
    if value == 0.0 {
        return "0".to_string();
    }
    let mut rendered = format!("{value:.15}");
    while rendered.contains('.') && rendered.ends_with('0') {
        rendered.pop();
    }
    if rendered.ends_with('.') {
        rendered.pop();
    }
    rendered
}

fn polygon_wkt(
    coordinates: &Value,
    limits: &mut GeometryLimits,
    max_positions: usize,
) -> ProviderResult<String> {
    limits.polygons += 1;
    let rings = coordinates
        .as_array()
        .filter(|rings| !rings.is_empty())
        .ok_or_else(|| {
            query_error(
                "CDSE_ODATA_AOI_INVALID",
                "Sentinel-2 AOI 多边形无效",
                "rings",
            )
        })?;
    let mut encoded_rings = Vec::with_capacity(rings.len());
    for ring in rings {
        limits.rings += 1;
        if limits.rings > MAX_RINGS {
            return Err(query_error(
                "CDSE_ODATA_AOI_TOO_COMPLEX",
                "Sentinel-2 AOI 过于复杂",
                "rings",
            ));
        }
        let positions = ring
            .as_array()
            .filter(|positions| !positions.is_empty())
            .ok_or_else(|| {
                query_error(
                    "CDSE_ODATA_AOI_INVALID",
                    "Sentinel-2 AOI 环无效",
                    "positions",
                )
            })?;
        let mut encoded_positions = Vec::with_capacity(positions.len());
        for position in positions {
            limits.positions += 1;
            if limits.positions > max_positions {
                return Err(query_error(
                    "CDSE_ODATA_AOI_TOO_COMPLEX",
                    "Sentinel-2 AOI 过于复杂",
                    "positions",
                ));
            }
            let values = position
                .as_array()
                .filter(|values| values.len() == 2)
                .ok_or_else(|| {
                    query_error(
                        "CDSE_ODATA_AOI_INVALID",
                        "Sentinel-2 AOI 坐标维度无效",
                        "position",
                    )
                })?;
            encoded_positions.push(format!(
                "{} {}",
                decimal(number(&values[0], "longitude")?),
                decimal(number(&values[1], "latitude")?)
            ));
        }
        encoded_rings.push(format!("({})", encoded_positions.join(",")));
    }
    Ok(format!("POLYGON({})", encoded_rings.join(",")))
}

fn collect_polygon_wkts(
    value: &Value,
    limits: &mut GeometryLimits,
    max_positions: usize,
    output: &mut Vec<String>,
) -> ProviderResult<()> {
    let object = value
        .as_object()
        .ok_or_else(|| query_error("CDSE_ODATA_AOI_INVALID", "Sentinel-2 AOI 无效", "geometry"))?;
    match object.get("type").and_then(Value::as_str) {
        Some("Polygon") => output.push(polygon_wkt(
            object.get("coordinates").unwrap_or(&Value::Null),
            limits,
            max_positions,
        )?),
        Some("MultiPolygon") => {
            for polygon in object
                .get("coordinates")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    query_error(
                        "CDSE_ODATA_AOI_INVALID",
                        "Sentinel-2 MultiPolygon 无效",
                        "coordinates",
                    )
                })?
            {
                output.push(polygon_wkt(polygon, limits, max_positions)?);
            }
        }
        Some("Feature") => collect_polygon_wkts(
            object.get("geometry").unwrap_or(&Value::Null),
            limits,
            max_positions,
            output,
        )?,
        Some("FeatureCollection") => {
            for feature in object
                .get("features")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    query_error(
                        "CDSE_ODATA_AOI_INVALID",
                        "Sentinel-2 FeatureCollection 无效",
                        "features",
                    )
                })?
            {
                collect_polygon_wkts(feature, limits, max_positions, output)?;
            }
        }
        _ => {
            return Err(query_error(
                "CDSE_ODATA_AOI_INVALID",
                "Sentinel-2 AOI 必须为多边形",
                "type",
            ))
        }
    }
    if limits.polygons > MAX_POLYGONS {
        return Err(query_error(
            "CDSE_ODATA_AOI_TOO_COMPLEX",
            "Sentinel-2 AOI 过于复杂",
            "polygons",
        ));
    }
    Ok(())
}

fn expand_for_use(use_case: RouteUse) -> &'static str {
    match use_case {
        RouteUse::OfflineFixture => "Attributes",
        // Product discovery only needs the closed Attributes shape. Quicklooks
        // are resolved later from the frozen product UUID, so an optional Asset
        // response can never reject or widen a downloadable search result.
        RouteUse::Production => "Attributes",
    }
}

fn fingerprint(
    query: &CatalogSearchQuery,
    facts: &Sentinel2SearchFacts,
    spatial_clause: &str,
    expand: &str,
) -> String {
    let mut canonical = format!(
        "route={}|profile={}|provider={}|collection={}|spatial={}|product_type={}|cloud={}|start={}|end={}|result_limit={}|orderby=ContentDate/Start asc|count=True|expand={}",
        facts.route_id,
        facts.profile_id,
        query.provider_id,
        query.native_collection_id,
        spatial_clause,
        facts.product_type,
        facts.cloud_cover_lte.map(decimal).unwrap_or_default(),
        facts.start.map(normalize_utc).unwrap_or_else(|| "ALL".to_owned()),
        facts.end.map(normalize_utc).unwrap_or_else(|| "ALL".to_owned()),
        query
            .result_limit
            .map_or_else(|| "ALL".to_owned(), |value| value.to_string()),
        expand,
    );
    if !facts.spacecraft.is_empty() {
        canonical.push_str("|spacecraft=");
        canonical.push_str(
            &facts
                .spacecraft
                .iter()
                .map(|p| p.value)
                .collect::<Vec<_>>()
                .join(","),
        );
    }
    Sha256::digest(canonical.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn hex_encode(value: &str) -> String {
    value
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn is_lower_hex(value: &str, exact_len: Option<usize>) -> bool {
    exact_len.is_none_or(|length| value.len() == length)
        && !value.is_empty()
        && value.is_ascii()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn hex_decode(value: &str) -> Option<String> {
    if !is_lower_hex(value, None) || !value.len().is_multiple_of(2) {
        return None;
    }
    let bytes = value
        .as_bytes()
        .chunks_exact(2)
        .map(|chunk| Some((hex_nibble(chunk[0])? << 4) | hex_nibble(chunk[1])?))
        .collect::<Option<Vec<_>>>()?;
    String::from_utf8(bytes).ok()
}

pub fn wrap_continuation(fingerprint: &str, next_link: &Url) -> ProviderResult<String> {
    if !is_lower_hex(fingerprint, Some(64)) {
        return Err(query_error(
            "CDSE_ODATA_CURSOR_UNTRUSTED",
            "Sentinel-2 续页标识无效",
            "fingerprint",
        ));
    }
    if next_link.as_str().len() > CDSE_SENTINEL2_MAX_NEXT_LINK_BYTES {
        return Err(query_error(
            "CDSE_ODATA_CURSOR_UNTRUSTED",
            "Sentinel-2 续页地址超出离线限制",
            "next_link_size",
        ));
    }
    let cursor = format!(
        "{CDSE_ODATA_CURSOR_VERSION}:{fingerprint}:{}",
        hex_encode(next_link.as_str())
    );
    if cursor.len() > CDSE_SENTINEL2_MAX_CURSOR_BYTES {
        return Err(query_error(
            "CDSE_ODATA_CURSOR_UNTRUSTED",
            "Sentinel-2 续页标识超出离线限制",
            "cursor_size",
        ));
    }
    Ok(cursor)
}

fn decode_continuation(value: &str, fingerprint: &str) -> ProviderResult<Url> {
    if value.len() > CDSE_SENTINEL2_MAX_CURSOR_BYTES {
        return Err(query_error(
            "CDSE_ODATA_CURSOR_UNTRUSTED",
            "Sentinel-2 续页标识超出离线限制",
            "cursor_size",
        ));
    }
    let parts = value.split(':').collect::<Vec<_>>();
    if parts.len() != 3
        || parts[0] != CDSE_ODATA_CURSOR_VERSION
        || !is_lower_hex(fingerprint, Some(64))
        || !is_lower_hex(parts[1], Some(64))
        || parts[1] != fingerprint
        || !is_lower_hex(parts[2], None)
        || !parts[2].len().is_multiple_of(2)
    {
        return Err(query_error(
            "CDSE_ODATA_CURSOR_UNTRUSTED",
            "Sentinel-2 续页标识与查询范围不匹配",
            "fingerprint",
        ));
    }
    let link = hex_decode(parts[2])
        .and_then(|value| Url::parse(&value).ok())
        .ok_or_else(|| {
            query_error(
                "CDSE_ODATA_CURSOR_UNTRUSTED",
                "Sentinel-2 续页标识无效",
                "encoding",
            )
        })?;
    Ok(link)
}

pub(super) fn can_replan_aoi(query: &CatalogSearchQuery, error: &ContractError) -> bool {
    query.cursor.is_none()
        && error.code == "CDSE_ODATA_AOI_TOO_COMPLEX"
        && matches!(
            error.debug_context.get("reason").and_then(Value::as_str),
            Some("encoded_query" | "positions" | "rings" | "polygons")
        )
}

/// The application uses the same request planner to distinguish compact AOI
/// transport statistics from ordinary OData totals. It never trusts a response
/// flag to opt a simple query out of its existing required-total contract.
pub fn requires_compact_aoi_transport(query: &CatalogSearchQuery, use_case: RouteUse) -> bool {
    build_odata_request_for_use(query, use_case).is_err_and(|error| {
        can_replan_aoi(query, &error)
            && query
                .aoi_geojson
                .as_ref()
                .and_then(crate::catalog::spatial::authoritative_catalogue_geometry)
                .is_some()
    })
}

pub fn build_odata_request(query: &CatalogSearchQuery) -> ProviderResult<ODataRequest> {
    build_odata_request_for_use(query, RouteUse::OfflineFixture)
}

/// Build a bounded OData request for one explicit route scope. Production
/// requests expand the reviewed static-preview metadata; fixture requests keep
/// their historical, locally reviewed response shape.
pub fn build_odata_request_for_use(
    query: &CatalogSearchQuery,
    use_case: RouteUse,
) -> ProviderResult<ODataRequest> {
    let query = &insar_core::clamp_catalogue_search_query(query);
    let facts = decode_sentinel2_search_facts_for_use(query, use_case)?;
    let aoi_geojson = query.aoi_geojson.as_ref().ok_or_else(|| {
        query_error(
            "CDSE_ODATA_AOI_INVALID",
            "Sentinel-2 路由需要明确 AOI",
            "missing",
        )
    })?;
    let mut limits = GeometryLimits::default();
    let mut polygons = Vec::new();
    collect_polygon_wkts(
        &Value::Object(aoi_geojson.clone()),
        &mut limits,
        max_positions_for_use(use_case),
        &mut polygons,
    )?;
    if polygons.is_empty() {
        return Err(query_error(
            "CDSE_ODATA_AOI_INVALID",
            "Sentinel-2 AOI 必须包含多边形",
            "empty",
        ));
    }
    let mut intersects = polygons
        .iter()
        .map(|polygon| format!("OData.CSC.Intersects(area=geography'SRID=4326;{polygon}')"))
        .collect::<Vec<_>>();
    let spatial_clause = if intersects.len() == 1 {
        intersects.remove(0)
    } else {
        format!("({})", intersects.join(" or "))
    };
    let collection_name = if query.native_collection_id == "SENTINEL-3" {
        "SENTINEL-3"
    } else {
        "SENTINEL-2"
    };
    let product_type_clause = if facts.product_types.len() == 1 {
        format!(
            "Attributes/OData.CSC.StringAttribute/any(att:att/Name eq 'productType' and att/OData.CSC.StringAttribute/Value eq '{}')",
            facts.product_types[0]
        )
    } else {
        // Live CDSE OData v4 rejects nested `(att/... eq '...' or ...)` inside a single
        // `any(att:...)` lambda with HTTP 400 `{"detail":{"message":"Invalid field: att"}}`.
        // Disjoining per-value `any(...)` predicates returns HTTP 200 on live CDSE OData.
        let predicates = facts
            .product_types
            .iter()
            .map(|pt| {
                format!(
                    "Attributes/OData.CSC.StringAttribute/any(att:att/Name eq 'productType' and att/OData.CSC.StringAttribute/Value eq '{pt}')"
                )
            })
            .collect::<Vec<_>>()
            .join(" or ");
        format!("({predicates})")
    };
    let mut clauses = vec![
        format!("Collection/Name eq '{collection_name}'"),
        product_type_clause,
        spatial_clause.clone(),
    ];
    if let (Some(start), Some(end)) = (facts.start, facts.end) {
        clauses.push(format!("ContentDate/Start ge {}", normalize_utc(start)));
        clauses.push(format!("ContentDate/Start lt {}", normalize_utc(end)));
    }
    if let Some(cloud) = facts.cloud_cover_lte {
        clauses.push(format!("Attributes/OData.CSC.DoubleAttribute/any(att:att/Name eq 'cloudCover' and att/OData.CSC.DoubleAttribute/Value le {})", decimal(cloud)));
    }
    if !facts.spacecraft.is_empty() {
        clauses.push(format!(
            "({})",
            facts
                .spacecraft
                .iter()
                .map(|platform| format!("startswith(Name,'{}')", platform.prefix))
                .collect::<Vec<_>>()
                .join(" or ")
        ));
    }
    let filter = clauses.join(" and ");
    let expand = expand_for_use(use_case);
    let scope_options = BTreeMap::from([
        ("$count".to_string(), "True".to_string()),
        ("$expand".to_string(), expand.to_string()),
        ("$filter".to_string(), filter),
        ("$orderby".to_string(), "ContentDate/Start asc".to_string()),
        (
            "$top".to_string(),
            transport_page_size(query.result_limit).to_string(),
        ),
    ]);
    let query_string = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(
            scope_options
                .iter()
                .map(|(key, value)| (key.as_str(), value.as_str())),
        )
        .finish();
    let path_and_query = format!("{CDSE_ODATA_PATH}?{query_string}");
    if path_and_query.len() > MAX_ENCODED_QUERY_BYTES {
        return Err(query_error(
            "CDSE_ODATA_AOI_TOO_COMPLEX",
            "Sentinel-2 查询过长",
            "encoded_query",
        ));
    }
    let fingerprint = fingerprint(query, &facts, &spatial_clause, expand);
    let continuation = query
        .cursor
        .as_deref()
        .map(|cursor| decode_continuation(cursor, &fingerprint))
        .transpose()?;
    let current_skip = continuation
        .as_ref()
        .map(numeric_skip)
        .transpose()?
        .unwrap_or(0);
    Ok(ODataRequest {
        path_and_query,
        fingerprint,
        scope_options,
        continuation,
        current_skip,
        facts,
    })
}

fn is_forbidden_parameter(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    [
        "token",
        "signature",
        "sig",
        "authorization",
        "cookie",
        "password",
        "secret",
    ]
    .iter()
    .any(|needle| name.contains(needle))
}

fn numeric_skip(continuation: &Url) -> ProviderResult<u64> {
    let mut skip = None;
    for (key, value) in continuation.query_pairs() {
        if key == "$skiptoken" {
            return Err(query_error(
                "CDSE_ODATA_CURSOR_UNTRUSTED",
                "Sentinel-2 续页参数不受信任",
                "skiptoken",
            ));
        }
        if key == "$skip" && skip.replace(value.to_string()).is_some() {
            return Err(query_error(
                "CDSE_ODATA_CURSOR_UNTRUSTED",
                "Sentinel-2 续页参数不受信任",
                "duplicate_skip",
            ));
        }
    }
    skip.and_then(|value| value.parse::<u64>().ok().filter(|value| *value > 0))
        .ok_or_else(|| {
            query_error(
                "CDSE_ODATA_CURSOR_UNTRUSTED",
                "Sentinel-2 续页偏移无效",
                "skip",
            )
        })
}

pub fn validate_continuation_for_endpoint(
    request: &ODataRequest,
    endpoint: &Url,
) -> ProviderResult<ValidatedContinuation> {
    let continuation = request.continuation().ok_or_else(|| {
        query_error(
            "CDSE_ODATA_CURSOR_UNTRUSTED",
            "Sentinel-2 续页标识缺失",
            "missing",
        )
    })?;
    if continuation.as_str().len() > CDSE_SENTINEL2_MAX_NEXT_LINK_BYTES {
        return Err(query_error(
            "CDSE_ODATA_CURSOR_UNTRUSTED",
            "Sentinel-2 续页地址超出离线限制",
            "next_link_size",
        ));
    }
    if continuation.scheme() != endpoint.scheme()
        || continuation.host_str() != endpoint.host_str()
        || continuation.port_or_known_default() != endpoint.port_or_known_default()
        || !continuation.username().is_empty()
        || continuation.password().is_some()
        || continuation.fragment().is_some()
    {
        return Err(query_error(
            "CDSE_ODATA_CURSOR_UNTRUSTED",
            "Sentinel-2 续页地址不受信任",
            "origin",
        ));
    }
    let expected_path = endpoint
        .join(CDSE_ODATA_PATH)
        .map_err(|_| query_error("CDSE_ODATA_ENDPOINT_INVALID", "离线 OData 端点无效", "path"))?;
    if continuation.path() != expected_path.path() {
        return Err(query_error(
            "CDSE_ODATA_CURSOR_UNTRUSTED",
            "Sentinel-2 续页路径不受信任",
            "path",
        ));
    }
    let mut actual = BTreeMap::new();
    for (key, value) in continuation.query_pairs() {
        if is_forbidden_parameter(&key)
            || actual.insert(key.to_string(), value.to_string()).is_some()
        {
            return Err(query_error(
                "CDSE_ODATA_CURSOR_UNTRUSTED",
                "Sentinel-2 续页参数不受信任",
                "parameters",
            ));
        }
    }
    for (key, value) in &request.scope_options {
        if actual.get(key) != Some(value) {
            return Err(query_error(
                "CDSE_ODATA_CURSOR_UNTRUSTED",
                "Sentinel-2 续页查询范围不匹配",
                "scope",
            ));
        }
    }
    if actual
        .keys()
        .any(|key| !request.scope_options.contains_key(key) && key != "$skip")
    {
        return Err(query_error(
            "CDSE_ODATA_CURSOR_UNTRUSTED",
            "Sentinel-2 续页参数不受信任",
            "pagination",
        ));
    }
    let skip = numeric_skip(continuation)?;
    Ok(ValidatedContinuation {
        url: continuation.clone(),
        skip,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sentinel3_query(product_type: Value) -> CatalogSearchQuery {
        serde_json::from_value(json!({
            "logical_dataset": "optical.sentinel-3.standard",
            "provider_id": "copernicus.dataspace.odata",
            "native_collection_id": "SENTINEL-3",
            "aoi_geojson": {
                "type": "Polygon",
                "coordinates": [[[110.0, 30.0], [111.0, 30.0], [111.0, 31.0], [110.0, 30.0]]]
            },
            "cursor": null,
            "result_limit": 10,
            "filters": {
                "product_type": product_type,
                "datetime": {"start": "2026-01-01T00:00:00Z", "end": "2026-01-02T00:00:00Z"}
            }
        }))
        .expect("valid generic Sentinel-3 query")
    }

    #[test]
    fn sentinel3_product_type_string_and_array_build_reviewed_odata_clauses() {
        // 1. Single string product_type
        let q_str = sentinel3_query(json!("OL_2_LFR___"));
        let req_str = build_odata_request_for_use(&q_str, RouteUse::Production).unwrap();
        let filter_str = url::form_urlencoded::parse(
            req_str
                .path_and_query()
                .split_once('?')
                .unwrap()
                .1
                .as_bytes(),
        )
        .find(|(k, _)| k == "$filter")
        .unwrap()
        .1
        .into_owned();
        assert!(filter_str.contains("Collection/Name eq 'SENTINEL-3'"));
        assert!(filter_str.contains("Attributes/OData.CSC.StringAttribute/any(att:att/Name eq 'productType' and att/OData.CSC.StringAttribute/Value eq 'OL_2_LFR___')"));

        // 2. Single item array product_type
        let q_arr1 = sentinel3_query(json!(["OL_2_LFR___"]));
        let req_arr1 = build_odata_request_for_use(&q_arr1, RouteUse::Production).unwrap();
        let filter_arr1 = url::form_urlencoded::parse(
            req_arr1
                .path_and_query()
                .split_once('?')
                .unwrap()
                .1
                .as_bytes(),
        )
        .find(|(k, _)| k == "$filter")
        .unwrap()
        .1
        .into_owned();
        assert!(filter_arr1.contains("Attributes/OData.CSC.StringAttribute/any(att:att/Name eq 'productType' and att/OData.CSC.StringAttribute/Value eq 'OL_2_LFR___')"));

        // 3. Multi-item array product_type
        let q_arr2 = sentinel3_query(json!(["OL_2_LFR___", "SL_2_LST___"]));
        let req_arr2 = build_odata_request_for_use(&q_arr2, RouteUse::Production).unwrap();
        let filter_arr2 = url::form_urlencoded::parse(
            req_arr2
                .path_and_query()
                .split_once('?')
                .unwrap()
                .1
                .as_bytes(),
        )
        .find(|(k, _)| k == "$filter")
        .unwrap()
        .1
        .into_owned();
        assert!(filter_arr2.contains("(Attributes/OData.CSC.StringAttribute/any(att:att/Name eq 'productType' and att/OData.CSC.StringAttribute/Value eq 'OL_2_LFR___') or Attributes/OData.CSC.StringAttribute/any(att:att/Name eq 'productType' and att/OData.CSC.StringAttribute/Value eq 'SL_2_LST___'))"));

        // 4. Invalid product_type rejected
        let q_invalid = sentinel3_query(json!(["INVALID_TYPE"]));
        assert!(build_odata_request_for_use(&q_invalid, RouteUse::Production).is_err());

        // 5. Empty array rejected
        let q_empty = sentinel3_query(json!([]));
        assert!(build_odata_request_for_use(&q_empty, RouteUse::Production).is_err());
    }

    #[test]
    fn production_route_marks_dense_polygon_over_48_positions_for_compact_aoi_transport_while_offline_fixture_allows_up_to_4096(
    ) {
        let mut ring = (0..60)
            .map(|i| {
                let angle = i as f64 * std::f64::consts::TAU / 60.0;
                let lon = ((117.0 + 0.5 * angle.cos()) * 10_000.0).round() / 10_000.0;
                let lat = ((31.8 + 0.5 * angle.sin()) * 10_000.0).round() / 10_000.0;
                json!([lon, lat])
            })
            .collect::<Vec<_>>();
        ring.push(ring[0].clone());
        let query: CatalogSearchQuery = serde_json::from_value(json!({
            "logical_dataset": "optical.sentinel-2.l2a",
            "provider_id": "copernicus.dataspace.odata",
            "native_collection_id": "SENTINEL-2",
            "aoi_geojson": {
                "type": "Polygon",
                "coordinates": [ring]
            },
            "cursor": null,
            "result_limit": 250,
            "filters": {
                "product_type": "S2MSI2A"
            }
        }))
        .expect("valid Sentinel-2 query");

        let prod_err = build_odata_request_for_use(&query, RouteUse::Production)
            .expect_err("61-position polygon must exceed MAX_PRODUCTION_POSITIONS (48)");
        assert_eq!(prod_err.code, "CDSE_ODATA_AOI_TOO_COMPLEX");
        assert_eq!(
            prod_err.debug_context.get("reason").and_then(Value::as_str),
            Some("positions")
        );
        assert!(requires_compact_aoi_transport(&query, RouteUse::Production));

        assert!(build_odata_request_for_use(&query, RouteUse::OfflineFixture).is_ok());
        assert!(!requires_compact_aoi_transport(
            &query,
            RouteUse::OfflineFixture
        ));
    }
}
