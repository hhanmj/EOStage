use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::net::IpAddr;
use std::time::Duration;

use insar_core::{
    canonicalized_authoritative_polygonal_geojson, ContractError, ProviderResult, ProxyPolicy,
};
use reqwest::blocking::Client;
use reqwest::redirect::Policy;
use serde_json::{json, Map, Value};
use url::Url;

// OSM's short official alias reaches the same Nominatim service while avoiding
// poisoned `openstreetmap.org` resolutions observed on the supported Windows
// network. TLS still authenticates an OSM-owned hostname; no alternate data
// provider or plaintext fallback is admitted.
pub const NOMINATIM_SEARCH_URL: &str = "https://nominatim.osm.org/search";
pub const DATAV_BOUNDARY_ROOT: &str = "https://geo.datav.aliyun.com/areas_v3/bound";
pub const OSM_ATTRIBUTION: &str = "© OpenStreetMap contributors";
pub const OSM_LICENSE: &str = "ODbL 1.0";
pub const NOMINATIM_ATTRIBUTION: &str = "© OpenStreetMap contributors；许可：ODbL 1.0";
/// DataV areas_v3 has no published open licence; EOStage only fetches it on
/// demand and never redistributes the geometry.
pub const DATAV_ATTRIBUTION: &str =
    "DataV.GeoAtlas（阿里云），数据来源：高德开放平台；许可：未公开开放许可，仅按需在线获取，不随软件分发";
pub const TIANDITU_ATTRIBUTION: &str = "国家地理信息公共服务平台（天地图）";
pub const TIANDITU_ADMIN_URL: &str = "https://api.tianditu.gov.cn/v2/administrative";
/// OSMF and the other boundary sources ask that automated clients identify the
/// application and a contact. Keep the version equal to `src-tauri/tauri.conf.json`.
pub const EOSTAGE_USER_AGENT: &str = "EOStage/2.2.0-beta.0 (+https://github.com/hhanmj/EOStage)";
pub const NOMINATIM_USER_AGENT: &str = EOSTAGE_USER_AGENT;

const MAX_RESULT_LIMIT: usize = 20;
const MAX_RESPONSE_BYTES: u64 = 8 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(12);

const DATAV_PROVINCES: &[(&str, &str)] = &[
    ("110000", "北京市"),
    ("120000", "天津市"),
    ("130000", "河北省"),
    ("140000", "山西省"),
    ("150000", "内蒙古自治区"),
    ("210000", "辽宁省"),
    ("220000", "吉林省"),
    ("230000", "黑龙江省"),
    ("310000", "上海市"),
    ("320000", "江苏省"),
    ("330000", "浙江省"),
    ("340000", "安徽省"),
    ("350000", "福建省"),
    ("360000", "江西省"),
    ("370000", "山东省"),
    ("410000", "河南省"),
    ("420000", "湖北省"),
    ("430000", "湖南省"),
    ("440000", "广东省"),
    ("450000", "广西壮族自治区"),
    ("460000", "海南省"),
    ("500000", "重庆市"),
    ("510000", "四川省"),
    ("520000", "贵州省"),
    ("530000", "云南省"),
    ("540000", "西藏自治区"),
    ("610000", "陕西省"),
    ("620000", "甘肃省"),
    ("630000", "青海省"),
    ("640000", "宁夏回族自治区"),
    ("650000", "新疆维吾尔自治区"),
    ("710000", "台湾省"),
    ("810000", "香港特别行政区"),
    ("820000", "澳门特别行政区"),
];

/// The DataV province-level name a free-text query names exactly, by full
/// name, short name (without 省/市/自治区/特别行政区) or a common alias of Hong
/// Kong, Macao and Taiwan. Partial matches never count.
pub fn china_province_for_text(text: &str) -> Option<&'static str> {
    let text = normalized_selection(text);
    if text.is_empty() {
        return None;
    }
    let alias = match text.as_str() {
        "hongkong" | "香港特區" | "香港特别行政區" | "香港特区" | "香港特別行政區" => {
            Some("810000")
        }
        "macau" | "macao" | "澳門" | "澳門特別行政區" | "澳门特区" => Some("820000"),
        "taiwan" | "台灣" | "臺灣" | "台灣省" | "臺灣省" => Some("710000"),
        _ => None,
    };
    DATAV_PROVINCES
        .iter()
        .find(|(code, name)| {
            alias == Some(*code) || {
                let short = [
                    "特别行政区",
                    "维吾尔自治区",
                    "壮族自治区",
                    "回族自治区",
                    "自治区",
                    "省",
                    "市",
                ]
                .iter()
                .find_map(|suffix| name.strip_suffix(suffix))
                .unwrap_or(name);
                text == *name || text == short
            }
        })
        .map(|(_, name)| *name)
}

#[derive(Clone, Debug, PartialEq)]
pub struct NormalizedGeometry {
    pub geometry: Value,
    pub geometry_kind: String,
    pub bbox: [f64; 4],
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProviderBoundaryFeature {
    pub provider_object_id: String,
    pub name: String,
    pub display_name: String,
    pub country_code: Option<String>,
    pub admin_level: Option<String>,
    pub geometry: Value,
    pub geometry_kind: String,
    pub bbox: [f64; 4],
    pub source: String,
    pub attribution: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DataVBoundaryBatch {
    pub transport_adcode: String,
    pub features: BTreeMap<String, ProviderBoundaryFeature>,
    pub invalid_features: Vec<DataVBoundaryBatchInvalidFeature>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataVBoundaryBatchInvalidFeature {
    pub provider_object_id: Option<String>,
    pub code: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProviderBoundaryCandidate {
    pub provider_object_id: String,
    pub name: String,
    pub display_name: String,
    pub country_code: Option<String>,
    pub admin_level: Option<String>,
    pub bbox: [f64; 4],
    pub source: String,
    pub attribution: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChinaAdminOptions {
    pub provinces: Vec<ChinaAdminOption>,
    pub cities: Vec<ChinaAdminOption>,
    pub districts: Vec<ChinaAdminOption>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChinaAdminOption {
    pub name: String,
    pub provider_object_id: Option<String>,
}

impl ChinaAdminOption {
    pub fn identified(name: impl Into<String>, provider_object_id: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            provider_object_id: Some(provider_object_id.into()),
        }
    }

    pub fn unidentified(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            provider_object_id: None,
        }
    }
}

pub trait GlobalBoundaryProvider: Send + Sync {
    fn search(&self, query: &str, limit: usize) -> ProviderResult<Vec<ProviderBoundaryFeature>>;

    fn discover(&self, query: &str, limit: usize)
        -> ProviderResult<Vec<ProviderBoundaryCandidate>>;

    fn resolve(&self, provider_object_id: &str) -> ProviderResult<ProviderBoundaryFeature>;

    fn discover_china(
        &self,
        query: &str,
        limit: usize,
    ) -> ProviderResult<Vec<ProviderBoundaryCandidate>>;

    fn search_taiwan(
        &self,
        query: &str,
        limit: usize,
    ) -> ProviderResult<Vec<ProviderBoundaryFeature>> {
        self.search(query, limit)
    }
}

pub trait ChinaBoundaryProvider: Send + Sync {
    fn options(&self, province: &str, city: &str) -> ProviderResult<ChinaAdminOptions>;

    fn resolve_exact_adcode(&self, _adcode: &str) -> ProviderResult<ProviderBoundaryFeature> {
        Err(boundary_error(
            "BOUNDARY_PROVIDER_REQUEST_INVALID",
            "当前中国行政边界提供方不支持精确 adcode 解析",
            "exact_adcode_unsupported",
        ))
    }

    fn search(
        &self,
        province: &str,
        city: &str,
        district: &str,
        query: &str,
        limit: usize,
    ) -> ProviderResult<Vec<ProviderBoundaryFeature>>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NominatimQuery {
    pub query: String,
    pub limit: usize,
    country_codes: Option<String>,
    include_geometry: bool,
}

impl NominatimQuery {
    pub fn for_global(query: &str, limit: usize) -> Self {
        Self {
            query: query.trim().to_owned(),
            limit: limit.clamp(1, MAX_RESULT_LIMIT),
            country_codes: None,
            include_geometry: false,
        }
    }

    pub fn for_taiwan(query: &str, limit: usize) -> Self {
        Self {
            query: query.trim().to_owned(),
            limit: limit.clamp(1, MAX_RESULT_LIMIT),
            country_codes: Some("tw".to_owned()),
            include_geometry: true,
        }
    }

    pub fn for_china(query: &str, limit: usize) -> Self {
        Self {
            query: query.trim().to_owned(),
            limit: limit.clamp(1, MAX_RESULT_LIMIT),
            country_codes: Some("cn".to_owned()),
            include_geometry: false,
        }
    }

    pub fn params(&self) -> BTreeMap<String, String> {
        let mut params = BTreeMap::from([
            ("q".to_owned(), self.query.clone()),
            (
                "format".to_owned(),
                if self.include_geometry {
                    "geojson"
                } else {
                    "jsonv2"
                }
                .to_owned(),
            ),
            (
                "polygon_geojson".to_owned(),
                if self.include_geometry { "1" } else { "0" }.to_owned(),
            ),
            ("addressdetails".to_owned(), "1".to_owned()),
            ("dedupe".to_owned(), "1".to_owned()),
            ("limit".to_owned(), self.limit.to_string()),
            ("accept-language".to_owned(), "zh-CN,zh,en".to_owned()),
        ]);
        if let Some(country_codes) = &self.country_codes {
            params.insert("countrycodes".to_owned(), country_codes.clone());
        }
        params
    }
}

#[derive(Clone)]
pub struct NominatimBoundaryProvider {
    client: Client,
    endpoint: Url,
}

impl NominatimBoundaryProvider {
    pub fn production() -> ProviderResult<Self> {
        Self::production_with_route(ProxyPolicy::System)
    }

    pub fn production_with_route(route: ProxyPolicy) -> ProviderResult<Self> {
        let endpoint = Url::parse(NOMINATIM_SEARCH_URL).map_err(|_| {
            boundary_error(
                "BOUNDARY_PROVIDER_CONFIGURATION_INVALID",
                "全球行政边界服务地址无效",
                "nominatim_endpoint",
            )
        })?;
        Self::build(endpoint, route)
    }

    #[doc(hidden)]
    pub fn with_test_endpoint(endpoint: &str) -> ProviderResult<Self> {
        let endpoint = Url::parse(endpoint).map_err(|_| {
            boundary_error(
                "BOUNDARY_PROVIDER_CONFIGURATION_INVALID",
                "测试行政边界服务地址无效",
                "test_endpoint_parse",
            )
        })?;
        let is_loopback = endpoint
            .host_str()
            .and_then(|host| host.parse::<IpAddr>().ok())
            .is_some_and(|address| address.is_loopback());
        if endpoint.scheme() != "http"
            || !is_loopback
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
        {
            return Err(boundary_error(
                "BOUNDARY_PROVIDER_CONFIGURATION_INVALID",
                "测试行政边界服务只允许使用本机回环 HTTP 地址",
                "test_endpoint_scope",
            ));
        }
        Self::build(endpoint, ProxyPolicy::Direct)
    }

    fn build(endpoint: Url, route: ProxyPolicy) -> ProviderResult<Self> {
        let mut builder = Client::builder()
            .user_agent(NOMINATIM_USER_AGENT)
            .timeout(REQUEST_TIMEOUT)
            .redirect(Policy::none())
            .retry(reqwest::retry::never())
            .referer(false);
        if endpoint.scheme() == "http"
            && endpoint
                .host_str()
                .and_then(|host| host.parse::<IpAddr>().ok())
                .is_some_and(|address| address.is_loopback())
        {
            builder = builder.tls_built_in_root_certs(false);
        }
        let client = crate::network_proxy::blocking_builder_with_proxy(builder, route)
            .build()
            .map_err(|_| {
                boundary_error(
                    "BOUNDARY_PROVIDER_CONFIGURATION_INVALID",
                    "无法初始化全球行政边界服务",
                    "client_build",
                )
            })?;
        Ok(Self { client, endpoint })
    }

    fn get_json(&self, endpoint: Url, params: &BTreeMap<String, String>) -> ProviderResult<Value> {
        let is_loopback = matches!(
            endpoint.host_str(),
            Some("127.0.0.1" | "localhost" | "::1" | "[::1]")
        );
        let max_attempts = if is_loopback { 1 } else { 3 };
        let mut last_err = None;
        let mut response_opt = None;
        for attempt in 0..max_attempts {
            match self.client.get(endpoint.clone()).query(params).send() {
                Ok(resp) => {
                    response_opt = Some(resp);
                    break;
                }
                Err(err) => {
                    last_err = Some(err);
                    if attempt + 1 < max_attempts {
                        std::thread::sleep(Duration::from_millis(300));
                    }
                }
            }
        }
        let _ = last_err;
        let response = response_opt.ok_or_else(|| {
            boundary_error(
                "BOUNDARY_PROVIDER_UNAVAILABLE",
                "全球行政边界服务暂时不可用，请稍后重试",
                "request_failed",
            )
        })?;
        if !response.status().is_success() {
            return Err(boundary_error(
                "BOUNDARY_PROVIDER_HTTP_ERROR",
                "全球行政边界服务返回错误，请稍后重试",
                "http_status",
            ));
        }
        read_bounded_json(response, MAX_RESPONSE_BYTES, "全球行政边界服务")
    }

    fn full_search_with_query(
        &self,
        request: NominatimQuery,
    ) -> ProviderResult<Vec<ProviderBoundaryFeature>> {
        if request.query.is_empty() {
            return Err(boundary_error(
                "BOUNDARY_QUERY_REQUIRED",
                "请输入地名或行政区名称",
                "empty_query",
            ));
        }
        let value = self.get_json(self.endpoint.clone(), &request.params())?;
        parse_feature_collection(value)
    }

    fn discover_with_query(
        &self,
        request: NominatimQuery,
    ) -> ProviderResult<Vec<ProviderBoundaryCandidate>> {
        if request.query.is_empty() {
            return Err(boundary_error(
                "BOUNDARY_QUERY_REQUIRED",
                "请输入地名或行政区名称",
                "empty_query",
            ));
        }
        let is_loopback = matches!(
            self.endpoint.host_str(),
            Some("127.0.0.1" | "localhost" | "::1" | "[::1]")
        );
        let mut params = request.params();
        if !is_loopback {
            params.insert("limit".to_owned(), MAX_RESULT_LIMIT.to_string());
        }
        let value = self.get_json(self.endpoint.clone(), &params)?;
        let mut candidates = parse_candidate_rows(value)?;
        candidates.truncate(request.limit);
        Ok(candidates)
    }

    fn lookup_endpoint(&self) -> Url {
        let mut endpoint = self.endpoint.clone();
        let mut segments = endpoint
            .path_segments()
            .map(|segments| segments.collect::<Vec<_>>())
            .unwrap_or_default();
        if segments.last().copied() == Some("search") {
            segments.pop();
        }
        segments.push("lookup");
        endpoint.set_path(&segments.join("/"));
        endpoint.set_query(None);
        endpoint
    }
}

#[derive(Clone)]
pub struct DataVBoundaryProvider {
    client: Client,
    root: Url,
}

impl DataVBoundaryProvider {
    pub fn production() -> ProviderResult<Self> {
        Self::production_with_route(ProxyPolicy::System)
    }

    pub fn production_with_route(route: ProxyPolicy) -> ProviderResult<Self> {
        let root = Url::parse(&format!("{DATAV_BOUNDARY_ROOT}/")).map_err(|_| {
            boundary_error(
                "BOUNDARY_PROVIDER_CONFIGURATION_INVALID",
                "DataV 行政区服务地址无效",
                "datav_endpoint",
            )
        })?;
        Self::build(root, route)
    }

    #[doc(hidden)]
    pub fn with_test_root(root: &str) -> ProviderResult<Self> {
        let root = Url::parse(root).map_err(|_| {
            boundary_error(
                "BOUNDARY_PROVIDER_CONFIGURATION_INVALID",
                "测试 DataV 服务地址无效",
                "test_endpoint_parse",
            )
        })?;
        let is_loopback = root
            .host_str()
            .and_then(|host| host.parse::<IpAddr>().ok())
            .is_some_and(|address| address.is_loopback());
        if root.scheme() != "http" || !is_loopback {
            return Err(boundary_error(
                "BOUNDARY_PROVIDER_CONFIGURATION_INVALID",
                "测试 DataV 服务只允许使用本机回环 HTTP 地址",
                "test_endpoint_scope",
            ));
        }
        let builder = Client::builder()
            .user_agent(EOSTAGE_USER_AGENT)
            .timeout(REQUEST_TIMEOUT)
            .redirect(Policy::none())
            .retry(reqwest::retry::never())
            .referer(false)
            .tls_built_in_root_certs(false);
        let client =
            crate::network_proxy::blocking_builder_with_proxy(builder, ProxyPolicy::Direct)
                .build()
                .map_err(|_| {
                    boundary_error(
                        "BOUNDARY_PROVIDER_CONFIGURATION_INVALID",
                        "无法初始化 DataV 行政区服务",
                        "client_build",
                    )
                })?;
        Ok(Self { client, root })
    }

    fn build(root: Url, route: ProxyPolicy) -> ProviderResult<Self> {
        let builder = Client::builder()
            .user_agent(EOSTAGE_USER_AGENT)
            .timeout(REQUEST_TIMEOUT)
            .redirect(Policy::none())
            .retry(reqwest::retry::never())
            .referer(false);
        let client = crate::network_proxy::blocking_builder_with_proxy(builder, route)
            .build()
            .map_err(|_| {
                boundary_error(
                    "BOUNDARY_PROVIDER_CONFIGURATION_INVALID",
                    "无法初始化 DataV 行政区服务",
                    "client_build",
                )
            })?;
        Ok(Self { client, root })
    }

    fn get_json(&self, path: &str) -> ProviderResult<Value> {
        self.get_json_internal(path, false)
    }

    fn get_json_exact(&self, path: &str) -> ProviderResult<Value> {
        self.get_json_internal(path, true)
    }

    fn get_json_internal(&self, path: &str, preserve_http_status: bool) -> ProviderResult<Value> {
        let endpoint = self.root.join(path).map_err(|_| {
            boundary_error(
                "BOUNDARY_PROVIDER_CONFIGURATION_INVALID",
                "DataV 行政区路径无效",
                "datav_path",
            )
        })?;
        let response = self.client.get(endpoint).send().map_err(|_| {
            boundary_error(
                "BOUNDARY_PROVIDER_UNAVAILABLE",
                "DataV 行政区服务暂时不可用，请稍后重试",
                "request_failed",
            )
        })?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let rate_limited = status == 429
                || response
                    .headers()
                    .get("x-tengine-error")
                    .and_then(|value| value.to_str().ok())
                    .is_some_and(|value| value.eq_ignore_ascii_case("denied by rate limit"));
            return Err(if preserve_http_status {
                datav_http_error(status, rate_limited)
            } else {
                boundary_error(
                    "BOUNDARY_PROVIDER_HTTP_ERROR",
                    "DataV 行政区服务返回错误，请稍后重试",
                    "http_status",
                )
            });
        }
        read_bounded_json(response, MAX_RESPONSE_BYTES, "DataV 行政区服务")
    }

    /// Resolve one DataV geometry by an already-authoritative six-digit adcode.
    ///
    /// This bounded path deliberately does not discover hierarchy, inspect
    /// `_full.json`, or compare names. The response must declare the same
    /// adcode before its polygon topology is normalized and admitted.
    pub fn resolve_exact_adcode(&self, adcode: &str) -> ProviderResult<ProviderBoundaryFeature> {
        if adcode.len() != 6 || !adcode.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(boundary_error(
                "BOUNDARY_PROVIDER_REQUEST_INVALID",
                "DataV 行政区编码必须是六位数字",
                "adcode_shape",
            ));
        }

        let value = self.get_json_exact(&format!("{adcode}.json"))?;
        let properties = exact_datav_properties(&value, adcode)?;
        let name = if adcode == "100000" {
            // DataV currently calls the national feature “中华人民共和国”,
            // while the product selector's stable country identity is “中国”.
            // Canonicalize only this authoritative adcode; lower-level source
            // names remain untouched.
            "中国".to_owned()
        } else {
            properties
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(adcode)
                .to_owned()
        };
        let admin_level = properties
            .get("level")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let geometry = normalize_datav_geometry(value)?;

        Ok(ProviderBoundaryFeature {
            provider_object_id: adcode.to_owned(),
            name: name.clone(),
            display_name: name,
            country_code: Some("CN".to_owned()),
            admin_level,
            geometry: geometry.geometry,
            geometry_kind: geometry.geometry_kind,
            bbox: geometry.bbox,
            source: "DataV areas_v3".to_owned(),
            attribution: Some(DATAV_ATTRIBUTION.to_owned()),
        })
    }

    /// Retrieve a DataV `_full.json` document strictly as batch geometry
    /// transport. Returned identities come only from six-digit
    /// `properties.adcode`; names, order, and nesting have no identity or
    /// hierarchy semantics.
    pub fn resolve_adcode_geometry_batch(
        &self,
        transport_adcode: &str,
    ) -> ProviderResult<DataVBoundaryBatch> {
        validate_datav_adcode_request(transport_adcode)?;
        let value = self.get_json_exact(&format!("{transport_adcode}_full.json"))?;
        let features = value
            .get("features")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                boundary_error(
                    "BOUNDARY_PROVIDER_RESPONSE_INVALID",
                    "DataV 批量行政区结果缺少要素数组",
                    "batch_features",
                )
            })?;

        let mut indexed = BTreeMap::new();
        let mut seen = BTreeSet::new();
        let mut invalid_features = Vec::new();
        for feature in features {
            let properties = feature
                .get("properties")
                .and_then(Value::as_object)
                .ok_or_else(|| {
                    boundary_error(
                        "BOUNDARY_PROVIDER_RESPONSE_INVALID",
                        "DataV 批量行政区要素缺少身份属性",
                        "batch_identity_properties",
                    )
                })?;
            let adcode = properties
                .get("adcode")
                .and_then(strict_six_digit_adcode_value)
                .ok_or_else(|| {
                    boundary_error(
                        "BOUNDARY_PROVIDER_RESPONSE_INVALID",
                        "DataV 批量行政区要素缺少严格六位行政区编码",
                        "batch_identity_adcode",
                    )
                })?;
            if !seen.insert(adcode.clone()) {
                return Err(boundary_error(
                    "BOUNDARY_PROVIDER_RESPONSE_INVALID",
                    "DataV 批量行政区结果包含重复行政区编码",
                    "batch_duplicate_adcode",
                ));
            }

            match normalize_datav_geometry(feature.clone()) {
                Ok(geometry) => {
                    let name = properties
                        .get("name")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .unwrap_or(&adcode)
                        .to_owned();
                    let admin_level = properties
                        .get("level")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(ToOwned::to_owned);
                    indexed.insert(
                        adcode.clone(),
                        ProviderBoundaryFeature {
                            provider_object_id: adcode,
                            name: name.clone(),
                            display_name: name,
                            country_code: Some("CN".to_owned()),
                            admin_level,
                            geometry: geometry.geometry,
                            geometry_kind: geometry.geometry_kind,
                            bbox: geometry.bbox,
                            source: "DataV areas_v3 batch".to_owned(),
                            attribution: Some(DATAV_ATTRIBUTION.to_owned()),
                        },
                    );
                }
                Err(error) => {
                    let reason = error
                        .debug_context
                        .get("reason")
                        .and_then(Value::as_str)
                        .unwrap_or("batch_geometry_invalid")
                        .to_owned();
                    invalid_features.push(DataVBoundaryBatchInvalidFeature {
                        provider_object_id: Some(adcode),
                        code: error.code,
                        reason,
                    });
                }
            }
        }

        Ok(DataVBoundaryBatch {
            transport_adcode: transport_adcode.to_owned(),
            features: indexed,
            invalid_features,
        })
    }

    fn rows_for_code(&self, code: &str) -> ProviderResult<Vec<DatavRow>> {
        let value = self.get_json(&format!("{code}_full.json"))?;
        Ok(datav_rows_from_full(&value))
    }

    fn direct_controlled_municipality_wrappers(code: &str) -> &'static [&'static str] {
        match code {
            "110000" => &["110100", "110200"],
            "120000" => &["120100", "120200"],
            "310000" => &["310100", "310200"],
            "500000" => &["500100", "500200"],
            _ => &[],
        }
    }

    fn municipality_district_rows(&self, province_code: &str) -> ProviderResult<Vec<DatavRow>> {
        let wrappers = Self::direct_controlled_municipality_wrappers(province_code);
        let mut rows = Vec::new();
        for row in self.rows_for_code(province_code)? {
            if wrappers.contains(&row.code.as_str()) {
                rows.extend(self.rows_for_code(&row.code)?);
            } else {
                rows.push(row);
            }
        }
        rows.sort_by(|left, right| {
            left.code
                .cmp(&right.code)
                .then_with(|| left.name.cmp(&right.name))
        });
        rows.dedup_by(|left, right| left.code == right.code);
        Ok(rows)
    }

    fn taiwan_second_level_rows(&self, province: &DatavRow) -> ProviderResult<Vec<DatavRow>> {
        let mut rows = Vec::new();
        for row in self.rows_for_code(&province.code)? {
            let redundant_wrapper = normalized_selection(&row.name)
                == normalized_selection(&province.name)
                && row.code != province.code;
            if redundant_wrapper {
                rows.extend(self.rows_for_code(&row.code)?);
            } else if row.code != province.code {
                rows.push(row);
            }
        }
        rows.sort_by(|left, right| {
            left.code
                .cmp(&right.code)
                .then_with(|| left.name.cmp(&right.name))
        });
        rows.dedup_by(|left, right| left.code == right.code);
        Ok(rows)
    }

    fn province_row(value: &str) -> Option<DatavRow> {
        let value = value.trim();
        if Self::is_national(value) {
            return None;
        }
        DATAV_PROVINCES
            .iter()
            .find(|(code, name)| {
                *code == value || *name == value || name.contains(value) || value.contains(name)
            })
            .map(|(code, name)| DatavRow {
                code: (*code).to_owned(),
                name: (*name).to_owned(),
            })
    }

    fn row_match(row: &DatavRow, value: &str) -> bool {
        let value = value.trim();
        Self::selection_unrestricted(value)
            || row.name == value
            || row.name.contains(value)
            || value.contains(&row.name)
    }

    fn selection_unrestricted(value: &str) -> bool {
        matches!(normalized_selection(value).as_str(), "" | "全部" | "不限")
    }

    fn is_national(value: &str) -> bool {
        matches!(
            normalized_selection(value).as_str(),
            "" | "全国" | "全部" | "不限" | "中国" | "china"
        )
    }

    fn is_direct_controlled_municipality(code: &str) -> bool {
        matches!(code, "110000" | "120000" | "310000" | "500000")
    }

    fn is_datav_direct_second_level_region(code: &str) -> bool {
        matches!(code, "810000" | "820000")
    }

    fn uses_direct_second_level_rows(code: &str) -> bool {
        Self::is_direct_controlled_municipality(code)
            || Self::is_datav_direct_second_level_region(code)
    }

    fn is_two_level_province(code: &str) -> bool {
        Self::uses_direct_second_level_rows(code) || code == "710000"
    }

    fn boundary_feature(
        &self,
        row: &DatavRow,
        label_parts: &[&str],
        admin_level: &str,
    ) -> ProviderResult<ProviderBoundaryFeature> {
        let value = self.get_json(&format!("{}.json", row.code))?;
        let geometry = normalize_datav_geometry(value)?;
        let display_name = label_parts
            .iter()
            .copied()
            .filter(|part| !part.trim().is_empty() && *part != "全部")
            .chain([row.name.as_str()])
            .collect::<Vec<_>>()
            .join(" / ");
        Ok(ProviderBoundaryFeature {
            provider_object_id: row.code.clone(),
            name: row.name.clone(),
            display_name,
            country_code: Some("CN".to_owned()),
            admin_level: Some(admin_level.to_owned()),
            geometry: geometry.geometry,
            geometry_kind: geometry.geometry_kind,
            bbox: geometry.bbox,
            source: "DataV areas_v3".to_owned(),
            attribution: Some(DATAV_ATTRIBUTION.to_owned()),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DatavRow {
    code: String,
    name: String,
}

fn datav_rows_from_full(value: &Value) -> Vec<DatavRow> {
    let mut rows = value
        .get("features")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|feature| {
            let properties = feature.get("properties")?.as_object()?;
            let code = parse_numeric_identifier(properties.get("adcode")?)?;
            let name = properties.get("name")?.as_str()?.trim();
            (!name.is_empty()).then(|| DatavRow {
                code,
                name: name.to_owned(),
            })
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        left.code
            .cmp(&right.code)
            .then_with(|| left.name.cmp(&right.name))
    });
    rows
}

impl ChinaBoundaryProvider for DataVBoundaryProvider {
    fn options(&self, province: &str, city: &str) -> ProviderResult<ChinaAdminOptions> {
        let mut options = ChinaAdminOptions {
            provinces: std::iter::once(ChinaAdminOption::identified("全国", "100000"))
                .chain(
                    DATAV_PROVINCES
                        .iter()
                        .map(|(code, name)| ChinaAdminOption::identified(*name, *code)),
                )
                .collect(),
            cities: vec![ChinaAdminOption::unidentified("全部")],
            districts: vec![ChinaAdminOption::unidentified("全部")],
        };
        let Some(province_row) = Self::province_row(province) else {
            return Ok(options);
        };
        let cities = if Self::is_direct_controlled_municipality(&province_row.code) {
            self.municipality_district_rows(&province_row.code)?
        } else if province_row.code == "710000" {
            self.taiwan_second_level_rows(&province_row)?
        } else {
            self.rows_for_code(&province_row.code)?
        };
        options.cities = std::iter::once(ChinaAdminOption::unidentified("全部"))
            .chain(
                cities
                    .iter()
                    .map(|row| ChinaAdminOption::identified(row.name.clone(), row.code.clone())),
            )
            .collect();
        if Self::is_two_level_province(&province_row.code) {
            return Ok(options);
        }
        if let Some(city_row) = cities.iter().find(|row| Self::row_match(row, city)) {
            let districts = self.rows_for_code(&city_row.code)?;
            options.districts = std::iter::once(ChinaAdminOption::unidentified("全部"))
                .chain(
                    districts
                        .into_iter()
                        .map(|row| ChinaAdminOption::identified(row.name, row.code)),
                )
                .collect();
        }
        Ok(options)
    }

    fn resolve_exact_adcode(&self, adcode: &str) -> ProviderResult<ProviderBoundaryFeature> {
        DataVBoundaryProvider::resolve_exact_adcode(self, adcode)
    }

    fn search(
        &self,
        province: &str,
        city: &str,
        district: &str,
        query: &str,
        limit: usize,
    ) -> ProviderResult<Vec<ProviderBoundaryFeature>> {
        let limit = limit.clamp(1, MAX_RESULT_LIMIT);
        if Self::is_national(province)
            && Self::selection_unrestricted(city)
            && Self::selection_unrestricted(district)
            && Self::selection_unrestricted(query)
        {
            let row = DatavRow {
                code: "100000".to_owned(),
                name: "中国".to_owned(),
            };
            return self
                .boundary_feature(&row, &[], "country")
                .map(|item| vec![item]);
        }
        let province_row = Self::province_row(province).ok_or_else(|| {
            boundary_error(
                "BOUNDARY_CHINA_SELECTION_INVALID",
                "未识别当前中国省级行政区",
                "province",
            )
        })?;
        if Self::selection_unrestricted(city)
            && Self::selection_unrestricted(district)
            && Self::selection_unrestricted(query)
        {
            return Ok(vec![self.boundary_feature(
                &province_row,
                &[],
                "province",
            )?]);
        }
        let cities = if Self::is_direct_controlled_municipality(&province_row.code) {
            self.municipality_district_rows(&province_row.code)?
        } else if province_row.code == "710000" {
            self.taiwan_second_level_rows(&province_row)?
        } else {
            self.rows_for_code(&province_row.code)?
        };
        let city_rows = if city.trim().is_empty() || city == "全部" {
            Vec::new()
        } else {
            cities
                .iter()
                .filter(|row| Self::row_match(row, city))
                .cloned()
                .collect()
        };
        let selected_city = city_rows.first();
        if let Some(city_row) = selected_city {
            if Self::uses_direct_second_level_rows(&province_row.code) {
                if Self::selection_unrestricted(district) && Self::selection_unrestricted(query) {
                    return Ok(vec![self.boundary_feature(
                        city_row,
                        &[&province_row.name],
                        "district",
                    )?]);
                }
                return Err(boundary_error(
                    "BOUNDARY_CHINA_SELECTION_INVALID",
                    "当前两级行政区选择不使用第三级行政区",
                    "direct_second_level_third_level",
                ));
            }
            if province_row.code == "710000" {
                if Self::selection_unrestricted(district) && Self::selection_unrestricted(query) {
                    return Ok(vec![self.boundary_feature(
                        city_row,
                        &[&province_row.name],
                        "city",
                    )?]);
                }
                return Err(boundary_error(
                    "BOUNDARY_CHINA_SELECTION_INVALID",
                    "台湾省市县选择不使用第三级行政区",
                    "taiwan_third_level",
                ));
            }
            if Self::selection_unrestricted(district) && Self::selection_unrestricted(query) {
                return Ok(vec![self.boundary_feature(
                    city_row,
                    &[&province_row.name],
                    "city",
                )?]);
            }
            let districts = self.rows_for_code(&city_row.code)?;
            let selected_districts = districts
                .iter()
                .filter(|row| Self::row_match(row, district) && Self::row_match(row, query))
                .take(limit)
                .cloned()
                .collect::<Vec<_>>();
            if !selected_districts.is_empty() {
                return selected_districts
                    .iter()
                    .map(|row| {
                        self.boundary_feature(
                            row,
                            &[&province_row.name, &city_row.name],
                            "district",
                        )
                    })
                    .collect();
            }
            if Self::row_match(city_row, query) || district.trim().is_empty() || district == "全部"
            {
                return Ok(vec![self.boundary_feature(
                    city_row,
                    &[&province_row.name],
                    "city",
                )?]);
            }
        }
        let mut rows = Vec::new();
        for row in &cities {
            if Self::row_match(row, query) {
                rows.push(self.boundary_feature(row, &[&province_row.name], "city")?);
                if rows.len() >= limit {
                    break;
                }
            }
        }
        if rows.is_empty() && Self::row_match(&province_row, query) {
            rows.push(self.boundary_feature(&province_row, &[], "province")?);
        }
        Ok(rows)
    }
}

impl GlobalBoundaryProvider for NominatimBoundaryProvider {
    fn search(&self, query: &str, limit: usize) -> ProviderResult<Vec<ProviderBoundaryFeature>> {
        let mut request = NominatimQuery::for_global(query, limit);
        request.include_geometry = true;
        self.full_search_with_query(request)
    }

    fn discover(
        &self,
        query: &str,
        limit: usize,
    ) -> ProviderResult<Vec<ProviderBoundaryCandidate>> {
        self.discover_with_query(NominatimQuery::for_global(query, limit))
    }

    fn resolve(&self, provider_object_id: &str) -> ProviderResult<ProviderBoundaryFeature> {
        let osm_id = lookup_osm_id(provider_object_id)?;
        let params = BTreeMap::from([
            ("osm_ids".to_owned(), osm_id),
            ("format".to_owned(), "geojson".to_owned()),
            ("polygon_geojson".to_owned(), "1".to_owned()),
            ("addressdetails".to_owned(), "1".to_owned()),
            ("accept-language".to_owned(), "zh-CN,zh,en".to_owned()),
        ]);
        let features = parse_feature_collection(self.get_json(self.lookup_endpoint(), &params)?)?;
        features
            .into_iter()
            .find(|feature| feature.provider_object_id == provider_object_id)
            .ok_or_else(|| {
                boundary_error(
                    "BOUNDARY_PROVIDER_IDENTITY_MISMATCH",
                    "行政边界服务返回了不一致的候选身份",
                    "provider_object_id",
                )
            })
    }

    fn discover_china(
        &self,
        query: &str,
        limit: usize,
    ) -> ProviderResult<Vec<ProviderBoundaryCandidate>> {
        self.discover_with_query(NominatimQuery::for_china(query, limit))
    }

    fn search_taiwan(
        &self,
        query: &str,
        limit: usize,
    ) -> ProviderResult<Vec<ProviderBoundaryFeature>> {
        self.full_search_with_query(NominatimQuery::for_taiwan(query, limit))
    }
}

fn lookup_osm_id(provider_object_id: &str) -> ProviderResult<String> {
    let (object_type, object_id) = provider_object_id.split_once(':').ok_or_else(|| {
        boundary_error(
            "BOUNDARY_PROVIDER_IDENTITY_INVALID",
            "行政边界候选身份无效",
            "provider_object_id",
        )
    })?;
    let prefix = match object_type {
        "node" => 'N',
        "way" => 'W',
        "relation" => 'R',
        _ => {
            return Err(boundary_error(
                "BOUNDARY_PROVIDER_IDENTITY_INVALID",
                "行政边界候选身份无效",
                "provider_object_type",
            ))
        }
    };
    let object_id = object_id
        .parse::<u64>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            boundary_error(
                "BOUNDARY_PROVIDER_IDENTITY_INVALID",
                "行政边界候选身份无效",
                "provider_object_id",
            )
        })?;
    Ok(format!("{prefix}{object_id}"))
}

fn parse_candidate_rows(value: Value) -> ProviderResult<Vec<ProviderBoundaryCandidate>> {
    let rows = value.as_array().ok_or_else(|| {
        boundary_error(
            "BOUNDARY_PROVIDER_RESPONSE_INVALID",
            "行政边界候选响应格式无效",
            "candidate_array",
        )
    })?;
    let candidates = rows
        .iter()
        .enumerate()
        .filter_map(|(index, row)| {
            let object = row.as_object()?;
            let object_type = osm_type(object.get("osm_type")?)?;
            let object_id = parse_positive_identifier(object.get("osm_id")?)?;
            if object_type != "relation" {
                return None;
            }
            let display_name = property_string(object, "display_name")
                .or_else(|| property_string(object, "name"))
                .unwrap_or_else(|| format!("行政边界 {}", index + 1));
            let name = property_string(object, "name").unwrap_or_else(|| {
                display_name
                    .split(',')
                    .next()
                    .unwrap_or(&display_name)
                    .trim()
                    .to_owned()
            });
            let bbox = parse_nominatim_bbox(object.get("boundingbox")?)?;
            let address = object.get("address").and_then(Value::as_object);
            let country_code = address
                .and_then(|address| address.get("country_code"))
                .and_then(Value::as_str)
                .map(|value| value.to_ascii_uppercase());
            Some(ProviderBoundaryCandidate {
                provider_object_id: format!("{object_type}:{object_id}"),
                name,
                display_name,
                country_code,
                admin_level: property_string(object, "type"),
                bbox,
                source: "OpenStreetMap Nominatim".to_owned(),
                attribution: Some(NOMINATIM_ATTRIBUTION.to_owned()),
            })
        })
        .collect::<Vec<_>>();
    Ok(candidates)
}

fn parse_nominatim_bbox(value: &Value) -> Option<[f64; 4]> {
    let values = value.as_array()?;
    let [south, north, west, east] = values.as_slice() else {
        return None;
    };
    let parse = parse_finite_number;
    let bbox = [parse(west)?, parse(south)?, parse(east)?, parse(north)?];
    (bbox[0] >= -180.0
        && bbox[2] <= 180.0
        && bbox[1] >= -90.0
        && bbox[3] <= 90.0
        && bbox[0] < bbox[2]
        && bbox[1] < bbox[3])
        .then_some(bbox)
}

fn parse_feature_collection(value: Value) -> ProviderResult<Vec<ProviderBoundaryFeature>> {
    let features = value
        .get("features")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            boundary_error(
                "BOUNDARY_PROVIDER_RESPONSE_INVALID",
                "行政边界服务缺少结果集合",
                "features_missing",
            )
        })?;
    let mut results = Vec::new();
    for (index, feature) in features.iter().enumerate() {
        let Some(geometry) = feature.get("geometry") else {
            continue;
        };
        let Ok(normalized) = normalize_polygonal_geometry(geometry.clone()) else {
            continue;
        };
        let properties = feature
            .get("properties")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let provider_object_id = provider_object_id(&properties, index);
        let display_name = property_string(&properties, "display_name")
            .or_else(|| property_string(&properties, "name"))
            .unwrap_or_else(|| format!("行政边界 {}", index + 1));
        let name = property_string(&properties, "name")
            .or_else(|| property_string(&properties, "name:zh"))
            .unwrap_or_else(|| {
                display_name
                    .split(',')
                    .next()
                    .unwrap_or(&display_name)
                    .trim()
                    .to_owned()
            });
        let address = properties
            .get("address")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let country_code = address
            .get("country_code")
            .and_then(Value::as_str)
            .map(|value| value.to_ascii_uppercase());
        let admin_level = property_string(&properties, "type");
        results.push(ProviderBoundaryFeature {
            provider_object_id,
            name,
            display_name,
            country_code,
            admin_level,
            geometry: normalized.geometry,
            geometry_kind: normalized.geometry_kind,
            bbox: normalized.bbox,
            source: "OpenStreetMap Nominatim".to_owned(),
            attribution: Some(NOMINATIM_ATTRIBUTION.to_owned()),
        });
    }
    Ok(results)
}

fn property_string(properties: &Map<String, Value>, key: &str) -> Option<String> {
    properties
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn provider_object_id(properties: &Map<String, Value>, index: usize) -> String {
    let object_type = properties.get("osm_type").and_then(osm_type);
    let object_id = properties.get("osm_id").and_then(parse_positive_identifier);
    match (object_type, object_id) {
        (Some(object_type), Some(object_id)) => format!("{object_type}:{object_id}"),
        _ => format!("result:{index}"),
    }
}

fn normalized_selection(value: &str) -> String {
    value
        .trim()
        .chars()
        .filter(|character| !character.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

fn parse_finite_number(value: &Value) -> Option<f64> {
    let number = match value {
        Value::Number(number) => number.as_f64()?,
        Value::String(text) => text.trim().parse::<f64>().ok()?,
        _ => return None,
    };
    number.is_finite().then_some(number)
}

fn parse_positive_identifier(value: &Value) -> Option<u64> {
    match value {
        Value::Number(number) => number.as_u64().filter(|value| *value > 0).or_else(|| {
            number
                .as_i64()
                .filter(|value| *value > 0)
                .map(|value| value as u64)
        }),
        Value::String(text) => text.trim().parse::<u64>().ok().filter(|value| *value > 0),
        _ => None,
    }
}

fn parse_numeric_identifier(value: &Value) -> Option<String> {
    parse_positive_identifier(value).map(|value| value.to_string())
}

fn strict_six_digit_adcode_value(value: &Value) -> Option<String> {
    let adcode = match value {
        Value::Number(number) => number.as_u64()?.to_string(),
        Value::String(value) if value.trim() == value => value.clone(),
        _ => return None,
    };
    (adcode.len() == 6 && adcode.bytes().all(|byte| byte.is_ascii_digit())).then_some(adcode)
}

fn validate_datav_adcode_request(adcode: &str) -> ProviderResult<()> {
    if adcode.len() != 6 || !adcode.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(boundary_error(
            "BOUNDARY_PROVIDER_REQUEST_INVALID",
            "DataV 行政区编码必须是六位数字",
            "adcode_shape",
        ));
    }
    Ok(())
}

fn exact_datav_properties<'a>(
    value: &'a Value,
    expected_adcode: &str,
) -> ProviderResult<&'a Map<String, Value>> {
    let features = match value.get("type").and_then(Value::as_str) {
        Some("FeatureCollection") => value
            .get("features")
            .and_then(Value::as_array)
            .filter(|features| !features.is_empty())
            .ok_or_else(|| {
                boundary_error(
                    "BOUNDARY_PROVIDER_RESPONSE_INVALID",
                    "DataV 行政区结果缺少要素",
                    "feature_collection_empty",
                )
            })?,
        Some("Feature") => std::slice::from_ref(value),
        _ => {
            return Err(boundary_error(
                "BOUNDARY_PROVIDER_RESPONSE_INVALID",
                "DataV 行政区结果缺少可验证的身份",
                "identity_container",
            ))
        }
    };

    let mut first_properties = None;
    for feature in features {
        let properties = feature
            .get("properties")
            .and_then(Value::as_object)
            .ok_or_else(|| {
                boundary_error(
                    "BOUNDARY_PROVIDER_RESPONSE_INVALID",
                    "DataV 行政区结果缺少身份属性",
                    "identity_properties",
                )
            })?;
        let actual_adcode = properties
            .get("adcode")
            .and_then(parse_numeric_identifier)
            .ok_or_else(|| {
                boundary_error(
                    "BOUNDARY_PROVIDER_RESPONSE_INVALID",
                    "DataV 行政区结果缺少有效行政区编码",
                    "identity_adcode",
                )
            })?;
        if actual_adcode != expected_adcode {
            return Err(boundary_error(
                "BOUNDARY_PROVIDER_IDENTITY_MISMATCH",
                "DataV 行政区结果与请求编码不一致",
                "response_adcode",
            ));
        }
        first_properties.get_or_insert(properties);
    }

    first_properties.ok_or_else(|| {
        boundary_error(
            "BOUNDARY_PROVIDER_RESPONSE_INVALID",
            "DataV 行政区结果缺少可验证的身份",
            "identity_missing",
        )
    })
}

fn osm_type(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
}

pub fn normalize_polygonal_geometry(value: Value) -> Result<NormalizedGeometry, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "polygon geometry is not an object".to_owned())?;
    let canonical = canonicalized_authoritative_polygonal_geojson(object).ok_or_else(|| {
        "polygon geometry is not a valid authoritative Polygon/MultiPolygon".to_owned()
    })?;
    let geometry_kind = canonical
        .geojson
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| "canonical geometry type is missing".to_owned())?
        .to_ascii_lowercase();
    Ok(NormalizedGeometry {
        geometry: Value::Object(canonical.geojson),
        geometry_kind,
        bbox: canonical.bbox,
    })
}

fn normalize_datav_geometry(mut value: Value) -> ProviderResult<NormalizedGeometry> {
    let mut geometry = extract_datav_geometry(value.take()).map_err(|reason| {
        boundary_error(
            "BOUNDARY_GEOMETRY_UNAVAILABLE",
            "DataV 行政区结果没有可用的真实面几何",
            &reason,
        )
    })?;
    geometry = canonicalize_datav_components(geometry).map_err(|reason| {
        boundary_error(
            "BOUNDARY_GEOMETRY_UNAVAILABLE",
            "DataV 行政区结果没有可用的真实面几何",
            &reason,
        )
    })?;
    // Normalize shared provider boundaries before applying the nonlinear
    // GCJ-02 -> WGS84 coordinate transform. Transforming adjacent source
    // components independently can create microscopic sliver overlaps even
    // when the original authoritative boundary is a clean union.
    transform_datav_coordinates(&mut geometry);
    normalize_polygonal_geometry(geometry).map_err(|reason| {
        boundary_error(
            "BOUNDARY_GEOMETRY_UNAVAILABLE",
            "DataV 行政区结果没有可用的真实面几何",
            &reason,
        )
    })
}

fn canonicalize_datav_components(value: Value) -> Result<Value, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "DataV geometry is not an object".to_owned())?;
    canonicalized_authoritative_polygonal_geojson(object)
        .map(|canonical| Value::Object(canonical.geojson))
        .ok_or_else(|| "DataV polygon geometry could not be normalized".to_owned())
}

fn extract_datav_geometry(value: Value) -> Result<Value, String> {
    match value.get("type").and_then(Value::as_str) {
        Some("Feature") => value
            .get("geometry")
            .cloned()
            .ok_or_else(|| "feature geometry is missing".to_owned()),
        Some("FeatureCollection") => {
            let features = value
                .get("features")
                .and_then(Value::as_array)
                .ok_or_else(|| "feature collection is missing features".to_owned())?;
            let mut polygons = Vec::new();
            for feature in features {
                let Some(geometry) = feature.get("geometry") else {
                    continue;
                };
                match geometry.get("type").and_then(Value::as_str) {
                    Some("Polygon") => {
                        if let Some(coordinates) = geometry.get("coordinates") {
                            polygons.push(coordinates.clone());
                        }
                    }
                    Some("MultiPolygon") => {
                        if let Some(coordinates) =
                            geometry.get("coordinates").and_then(Value::as_array)
                        {
                            polygons.extend(coordinates.iter().cloned());
                        }
                    }
                    _ => {}
                }
            }
            match polygons.len() {
                0 => Err("feature collection has no polygon geometry".to_owned()),
                1 => Ok(serde_json::json!({"type": "Polygon", "coordinates": polygons.remove(0)})),
                _ => Ok(serde_json::json!({"type": "MultiPolygon", "coordinates": polygons})),
            }
        }
        _ => Ok(value),
    }
}

fn transform_datav_coordinates(value: &mut Value) {
    match value {
        Value::Object(object) => {
            if let Some(coordinates) = object.get_mut("coordinates") {
                transform_datav_coordinates(coordinates);
            }
            for (key, child) in object.iter_mut() {
                if key != "coordinates" {
                    transform_datav_coordinates(child);
                }
            }
        }
        Value::Array(items) if items.len() >= 2 && items[0].is_number() && items[1].is_number() => {
            let Some(lng) = items[0].as_f64() else {
                return;
            };
            let Some(lat) = items[1].as_f64() else {
                return;
            };
            let (lng, lat) = gcj02_to_wgs84(lng, lat);
            items[0] = serde_json::json!(lng);
            items[1] = serde_json::json!(lat);
        }
        Value::Array(items) => {
            for child in items {
                transform_datav_coordinates(child);
            }
        }
        _ => {}
    }
}

fn gcj02_to_wgs84(lng: f64, lat: f64) -> (f64, f64) {
    match gcj02_delta(lng, lat) {
        Some((dlng, dlat)) => (lng * 2.0 - (lng + dlng), lat * 2.0 - (lat + dlat)),
        None => (lng, lat),
    }
}

/// The GCJ-02 offset at a point, or `None` outside the GCJ-02 domain.
fn gcj02_delta(lng: f64, lat: f64) -> Option<(f64, f64)> {
    if !(73.66 < lng && lng < 135.05 && 3.86 < lat && lat < 53.55) {
        return None;
    }
    let x = lng - 105.0;
    let y = lat - 35.0;
    let pi = std::f64::consts::PI;
    let mut dlat = -100.0 + 2.0 * x + 3.0 * y + 0.2 * y * y + 0.1 * x * y + 0.2 * x.abs().sqrt();
    dlat += (20.0 * (6.0 * x * pi).sin() + 20.0 * (2.0 * x * pi).sin()) * 2.0 / 3.0;
    dlat += (20.0 * (y * pi).sin() + 40.0 * (y / 3.0 * pi).sin()) * 2.0 / 3.0;
    dlat += (160.0 * (y / 12.0 * pi).sin() + 320.0 * (y * pi / 30.0).sin()) * 2.0 / 3.0;

    let mut dlng = 300.0 + x + 2.0 * y + 0.1 * x * x + 0.1 * x * y + 0.1 * x.abs().sqrt();
    dlng += (20.0 * (6.0 * x * pi).sin() + 20.0 * (2.0 * x * pi).sin()) * 2.0 / 3.0;
    dlng += (20.0 * (x * pi).sin() + 40.0 * (x / 3.0 * pi).sin()) * 2.0 / 3.0;
    dlng += (150.0 * (x / 12.0 * pi).sin() + 300.0 * (x / 30.0 * pi).sin()) * 2.0 / 3.0;

    let a = 6_378_245.0;
    let ee = 0.006_693_421_622_965_943;
    let radlat = lat / 180.0 * pi;
    let magic = 1.0 - ee * radlat.sin() * radlat.sin();
    let sqrtmagic = magic.sqrt();
    dlat = (dlat * 180.0) / ((a * (1.0 - ee)) / (magic * sqrtmagic) * pi);
    dlng = (dlng * 180.0) / (a / sqrtmagic * radlat.cos() * pi);
    Some((dlng, dlat))
}

/// Whether a DataV geometry is in GCJ-02 or already in WGS84, judged against
/// an independent WGS84 outline of the same unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DatavCrsVerdict {
    Gcj02,
    Wgs84,
    Inconclusive,
}

/// Forward WGS84 -> GCJ-02 shift (identity outside the GCJ-02 domain).
pub fn wgs84_to_gcj02(lng: f64, lat: f64) -> (f64, f64) {
    match gcj02_delta(lng, lat) {
        Some((dlng, dlat)) => (lng + dlng, lat + dlat),
        None => (lng, lat),
    }
}

/// Compare every DataV vertex with the reference outline twice: as given and
/// after the GCJ-02 -> WGS84 correction. Coastlines and maritime extents
/// differ between sources, so the lower quartile of vertex distances is used:
/// the shared land border lines up within tens of metres in the right CRS and
/// stays about one GCJ-02 offset (hundreds of metres) away in the wrong one.
pub fn classify_datav_crs(raw_datav_geometry: &Value, wgs84_reference: &Value) -> DatavCrsVerdict {
    const DECISIVE_METRES: f64 = 150.0;
    let (Some(raw), Some(reference)) = (
        polygon_rings(raw_datav_geometry),
        polygon_rings(wgs84_reference),
    ) else {
        return DatavCrsVerdict::Inconclusive;
    };
    let vertices = raw.iter().flatten().copied().collect::<Vec<_>>();
    if vertices.is_empty() || reference.is_empty() {
        return DatavCrsVerdict::Inconclusive;
    }
    let as_given = lower_quartile_distance(&vertices, &reference);
    let corrected = lower_quartile_distance(
        &vertices
            .iter()
            .map(|&(lng, lat)| gcj02_to_wgs84(lng, lat))
            .collect::<Vec<_>>(),
        &reference,
    );
    if corrected <= DECISIVE_METRES && corrected * 2.0 < as_given {
        DatavCrsVerdict::Gcj02
    } else if as_given <= DECISIVE_METRES && as_given * 2.0 < corrected {
        DatavCrsVerdict::Wgs84
    } else {
        DatavCrsVerdict::Inconclusive
    }
}

fn polygon_rings(value: &Value) -> Option<Vec<Vec<(f64, f64)>>> {
    let geometry = match value.get("type").and_then(Value::as_str)? {
        "Feature" => value.get("geometry")?,
        "FeatureCollection" => value
            .get("features")?
            .as_array()?
            .first()?
            .get("geometry")?,
        _ => value,
    };
    let polygons = match geometry.get("type").and_then(Value::as_str)? {
        "Polygon" => vec![geometry.get("coordinates")?],
        "MultiPolygon" => geometry.get("coordinates")?.as_array()?.iter().collect(),
        _ => return None,
    };
    let mut rings = Vec::new();
    for polygon in polygons {
        for ring in polygon.as_array()? {
            let points = ring
                .as_array()?
                .iter()
                .map(|position| {
                    let position = position.as_array()?;
                    Some((position.first()?.as_f64()?, position.get(1)?.as_f64()?))
                })
                .collect::<Option<Vec<_>>>()?;
            rings.push(points);
        }
    }
    Some(rings)
}

fn lower_quartile_distance(vertices: &[(f64, f64)], reference: &[Vec<(f64, f64)>]) -> f64 {
    let mut distances = vertices
        .iter()
        .map(|&vertex| {
            reference
                .iter()
                .flat_map(|ring| ring.windows(2))
                .map(|segment| segment_distance_metres(vertex, segment[0], segment[1]))
                .fold(f64::INFINITY, f64::min)
        })
        .collect::<Vec<_>>();
    distances.sort_by(f64::total_cmp);
    distances[distances.len() / 4]
}

fn segment_distance_metres(point: (f64, f64), start: (f64, f64), end: (f64, f64)) -> f64 {
    // Local equirectangular metres around the point; adequate for the
    // sub-kilometre distances being compared.
    let metres_per_lng = 111_320.0 * point.1.to_radians().cos();
    let metres_per_lat = 110_540.0;
    let project = |(lng, lat): (f64, f64)| {
        (
            (lng - point.0) * metres_per_lng,
            (lat - point.1) * metres_per_lat,
        )
    };
    let (ax, ay) = project(start);
    let (bx, by) = project(end);
    let (dx, dy) = (bx - ax, by - ay);
    let length_squared = dx * dx + dy * dy;
    let t = if length_squared == 0.0 {
        0.0
    } else {
        ((-ax * dx - ay * dy) / length_squared).clamp(0.0, 1.0)
    };
    let (cx, cy) = (ax + t * dx, ay + t * dy);
    (cx * cx + cy * cy).sqrt()
}

/// Read a successful response body as JSON within `max_bytes`. A proxy login
/// page or CDN error page served with 200 is reported as non-data instead of
/// a generic parse failure, and is never handed to a cache.
pub(crate) fn read_bounded_json(
    response: reqwest::blocking::Response,
    max_bytes: u64,
    service: &str,
) -> ProviderResult<Value> {
    let declared_html = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .trim_start()
                .to_ascii_lowercase()
                .starts_with("text/html")
        });
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes)
    {
        return Err(boundary_error(
            "BOUNDARY_PROVIDER_RESPONSE_TOO_LARGE",
            &format!("{service}返回的数据超过应用限制"),
            "content_length",
        ));
    }
    let mut bytes = Vec::new();
    response
        .take(max_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| {
            boundary_error(
                "BOUNDARY_PROVIDER_RESPONSE_INVALID",
                &format!("{service}响应读取失败"),
                "read_body",
            )
        })?;
    if bytes.len() as u64 > max_bytes {
        return Err(boundary_error(
            "BOUNDARY_PROVIDER_RESPONSE_TOO_LARGE",
            &format!("{service}返回的数据超过应用限制"),
            "body_length",
        ));
    }
    let body = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
    let starts_with_markup = body
        .iter()
        .find(|byte| !byte.is_ascii_whitespace())
        .is_some_and(|byte| *byte == b'<');
    if declared_html || starts_with_markup {
        return Err(boundary_error(
            "BOUNDARY_PROVIDER_NON_DATA_RESPONSE",
            &format!("{service}返回了网页而不是边界数据（可能是代理或网络错误页），请检查网络或在设置里切换代理"),
            "html_body",
        ));
    }
    serde_json::from_slice(body).map_err(|_| {
        boundary_error(
            "BOUNDARY_PROVIDER_RESPONSE_INVALID",
            &format!("{service}返回了无法解析的数据"),
            "json_parse",
        )
    })
}

/// The attribution a stored or injected result of `provider` falls back to.
pub fn default_boundary_attribution(provider: &str) -> Option<&'static str> {
    match provider {
        "datav" => Some(DATAV_ATTRIBUTION),
        "tianditu" => Some(TIANDITU_ATTRIBUTION),
        "nominatim" => Some(NOMINATIM_ATTRIBUTION),
        _ => None,
    }
}

/// Fetch one domestic administrative boundary (e.g. Taiwan province city/county)
/// from Tianditu (`api.tianditu.gov.cn/v2/administrative`) using the user's
/// personal key. Returned coordinates are CGCS2000 / WGS84 and never shifted via
/// GCJ-02 -> WGS84. Token material never appears in errors, logs, or metadata.
pub fn fetch_tianditu_admin_boundary(
    route: ProxyPolicy,
    keyword: &str,
    expected_gb: Option<&str>,
    token: &str,
) -> ProviderResult<ProviderBoundaryFeature> {
    let keyword = keyword.trim();
    let token = token.trim();
    if keyword.is_empty() || token.is_empty() {
        return Err(boundary_error(
            "BOUNDARY_PROVIDER_CONFIGURATION_INVALID",
            "未配置可用的个人天地图服务密钥",
            "tianditu_token_missing",
        ));
    }
    let mut endpoint = Url::parse(TIANDITU_ADMIN_URL).map_err(|_| {
        boundary_error(
            "BOUNDARY_PROVIDER_CONFIGURATION_INVALID",
            "天地图行政区划服务地址无效",
            "tianditu_endpoint",
        )
    })?;
    endpoint
        .query_pairs_mut()
        .append_pair("keyword", keyword)
        .append_pair("childLevel", "0")
        .append_pair("extensions", "true")
        .append_pair("tk", token);

    let builder = Client::builder()
        .user_agent(EOSTAGE_USER_AGENT)
        .timeout(REQUEST_TIMEOUT)
        .redirect(Policy::none())
        .retry(reqwest::retry::never())
        .referer(false);
    let client = crate::network_proxy::blocking_builder_with_proxy(builder, route)
        .build()
        .map_err(|_| {
            boundary_error(
                "BOUNDARY_PROVIDER_CONFIGURATION_INVALID",
                "无法初始化天地图行政区客户端",
                "tianditu_client_build",
            )
        })?;
    let response = client.get(endpoint).send().map_err(|_| {
        boundary_error(
            "BOUNDARY_PROVIDER_UNAVAILABLE",
            "天地图行政区划服务暂时不可用，请稍后重试",
            "tianditu_request_failed",
        )
    })?;
    let status = response.status();
    if !status.is_success() {
        return Err(boundary_error(
            if status.as_u16() == 429 {
                "BOUNDARY_PROVIDER_RATE_LIMITED"
            } else {
                "BOUNDARY_PROVIDER_HTTP_ERROR"
            },
            "天地图行政区划服务返回错误，请检查个人密钥配额或稍后重试",
            "tianditu_http_status",
        ));
    }
    let value = read_bounded_json(response, MAX_RESPONSE_BYTES, "天地图行政区划服务")?;
    extract_tianditu_admin_feature(&value, keyword, expected_gb)
}

fn extract_tianditu_admin_feature(
    value: &Value,
    keyword: &str,
    expected_gb: Option<&str>,
) -> ProviderResult<ProviderBoundaryFeature> {
    let districts = value
        .get("data")
        .and_then(|d| d.get("district"))
        .and_then(Value::as_array)
        .ok_or_else(|| {
            boundary_error(
                "BOUNDARY_PROVIDER_RESPONSE_INVALID",
                "天地图未返回可用的行政区划记录",
                "tianditu_no_district",
            )
        })?;
    let wanted_norm = normalized_selection(keyword);
    let item = districts
        .iter()
        .find(|entry| {
            entry
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(|n| normalized_selection(n) == wanted_norm)
        })
        .or_else(|| districts.first())
        .ok_or_else(|| {
            boundary_error(
                "BOUNDARY_GEOMETRY_UNAVAILABLE",
                "天地图未命中目标行政区",
                "tianditu_empty_district",
            )
        })?;
    let raw_gb = item
        .get("gb")
        .and_then(|v| {
            v.as_str()
                .map(str::trim)
                .map(ToOwned::to_owned)
                .or_else(|| v.as_u64().map(|n| n.to_string()))
        })
        .filter(|s| (s.len() == 6 || s.len() == 9) && s.bytes().all(|b| b.is_ascii_digit()))
        .or_else(|| expected_gb.map(str::to_owned))
        .ok_or_else(|| {
            boundary_error(
                "BOUNDARY_PROVIDER_RESPONSE_INVALID",
                "天地图返回结果缺少合法国标编码",
                "tianditu_missing_gb",
            )
        })?;
    let raw_boundary = item.get("boundary").ok_or_else(|| {
        boundary_error(
            "BOUNDARY_GEOMETRY_UNAVAILABLE",
            "天地图返回结果不包含行政区边界坐标",
            "tianditu_missing_boundary",
        )
    })?;
    let geojson = parse_tianditu_boundary_geometry(raw_boundary).ok_or_else(|| {
        boundary_error(
            "BOUNDARY_GEOMETRY_UNAVAILABLE",
            "天地图返回的行政区边界坐标无法解析为多边形",
            "tianditu_boundary_parse",
        )
    })?;
    let geometry = normalize_polygonal_geometry(geojson).map_err(|reason| {
        boundary_error(
            "BOUNDARY_GEOMETRY_UNAVAILABLE",
            "天地图返回的边界几何不合法",
            &reason,
        )
    })?;
    let name = item
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(keyword)
        .to_owned();
    let display_name = if name.contains("台湾") || name == keyword {
        name.clone()
    } else {
        format!("台湾省 / {name}")
    };
    Ok(ProviderBoundaryFeature {
        provider_object_id: raw_gb,
        name,
        display_name,
        country_code: Some("CN".to_owned()),
        admin_level: Some("city".to_owned()),
        geometry: geometry.geometry,
        geometry_kind: geometry.geometry_kind,
        bbox: geometry.bbox,
        source: "天地图行政区划 API".to_owned(),
        attribution: Some(TIANDITU_ATTRIBUTION.to_owned()),
    })
}

fn parse_tianditu_ring(text: &str) -> Option<Vec<Vec<f64>>> {
    let mut ring = Vec::new();
    for pair in text.split(',') {
        let mut nums = pair
            .split_whitespace()
            .filter_map(|p| p.parse::<f64>().ok());
        let (Some(lon), Some(lat)) = (nums.next(), nums.next()) else {
            continue;
        };
        if lon.is_finite() && lat.is_finite() {
            ring.push(vec![lon, lat]);
        }
    }
    if ring.len() < 3 {
        return None;
    }
    if ring.first() != ring.last() {
        if let Some(first) = ring.first().cloned() {
            ring.push(first);
        }
    }
    (ring.len() >= 4).then_some(ring)
}

fn split_paren_groups(body: &str) -> Vec<&str> {
    let mut groups = Vec::new();
    let mut depth = 0usize;
    let mut start = None;
    for (idx, ch) in body.char_indices() {
        match ch {
            '(' => {
                if depth == 0 {
                    start = Some(idx + 1);
                }
                depth += 1;
            }
            ')' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    if let Some(begin) = start.take() {
                        groups.push(body[begin..idx].trim());
                    }
                }
            }
            _ => {}
        }
    }
    groups
}

fn parse_tianditu_boundary_geometry(raw: &Value) -> Option<Value> {
    if raw.is_object() {
        return Some(raw.clone());
    }
    let text = raw.as_str()?.trim();
    if text.is_empty() {
        return None;
    }
    let upper = text.to_ascii_uppercase();
    if upper.starts_with("MULTIPOLYGON") {
        let outer = text.find('(')?;
        let end = text.rfind(')')?;
        if end <= outer {
            return None;
        }
        let inner = &text[outer + 1..end];
        let mut polygons = Vec::new();
        for poly_group in split_paren_groups(inner) {
            let mut rings = Vec::new();
            let ring_groups = split_paren_groups(poly_group);
            if ring_groups.is_empty() {
                if let Some(r) = parse_tianditu_ring(poly_group) {
                    rings.push(r);
                }
            } else {
                for rg in ring_groups {
                    if let Some(r) = parse_tianditu_ring(rg) {
                        rings.push(r);
                    }
                }
            }
            if !rings.is_empty() {
                polygons.push(rings);
            }
        }
        return (!polygons.is_empty())
            .then(|| json!({ "type": "MultiPolygon", "coordinates": polygons }));
    }
    if upper.starts_with("POLYGON") {
        let outer = text.find('(')?;
        let end = text.rfind(')')?;
        if end <= outer {
            return None;
        }
        let inner = &text[outer + 1..end];
        let mut rings = Vec::new();
        let ring_groups = split_paren_groups(inner);
        if ring_groups.is_empty() {
            if let Some(r) = parse_tianditu_ring(inner) {
                rings.push(r);
            }
        } else {
            for rg in ring_groups {
                if let Some(r) = parse_tianditu_ring(rg) {
                    rings.push(r);
                }
            }
        }
        return (!rings.is_empty()).then(|| json!({ "type": "Polygon", "coordinates": rings }));
    }
    let polygons: Vec<Vec<Vec<Vec<f64>>>> = text
        .split(';')
        .filter_map(|seg| parse_tianditu_ring(seg).map(|r| vec![r]))
        .collect();
    if polygons.len() == 1 {
        Some(json!({ "type": "Polygon", "coordinates": &polygons[0] }))
    } else if polygons.len() > 1 {
        Some(json!({ "type": "MultiPolygon", "coordinates": polygons }))
    } else {
        None
    }
}

fn boundary_error(code: &str, message: &str, reason: &str) -> ContractError {
    ContractError::new(
        code,
        message,
        Map::from_iter([("reason".to_owned(), Value::String(reason.to_owned()))]),
    )
    .expect("bounded boundary error context is safe")
}

fn datav_http_error(status: u16, rate_limited: bool) -> ContractError {
    ContractError::new(
        if rate_limited {
            "BOUNDARY_PROVIDER_RATE_LIMITED"
        } else {
            "BOUNDARY_PROVIDER_HTTP_ERROR"
        },
        "DataV 行政区服务返回错误，请稍后重试",
        Map::from_iter([
            (
                "reason".to_owned(),
                Value::String(if rate_limited {
                    format!("http_status_{status}_rate_limited")
                } else {
                    format!("http_status_{status}")
                }),
            ),
            (
                "http_status".to_owned(),
                Value::Number(serde_json::Number::from(status)),
            ),
            ("rate_limited".to_owned(), Value::Bool(rate_limited)),
        ]),
    )
    .expect("bounded DataV HTTP error context is safe")
}

#[cfg(test)]
mod tests {
    use super::*;
    use insar_core::validated_polygonal_geojson;
    use serde_json::json;

    fn dense_ring(position_count: usize) -> Vec<Value> {
        let bottom_vertices = position_count - 3;
        let mut ring = (0..bottom_vertices)
            .map(|index| {
                let fraction = index as f64 / (bottom_vertices - 1) as f64;
                json!([113.0 + 7.0 * fraction, 36.0])
            })
            .collect::<Vec<_>>();
        ring.extend([
            json!([120.0, 43.0]),
            json!([113.0, 43.0]),
            json!([113.0, 36.0]),
        ]);
        ring
    }

    #[test]
    fn datav_overlap_is_canonicalized_after_each_component_is_validated() {
        let raw = json!({
            "type": "MultiPolygon",
            "coordinates": [
                [dense_ring(1_549)],
                [[[118.80, 39.10], [118.85, 39.10], [118.85, 39.15], [118.80, 39.15], [118.80, 39.10]]]
            ]
        });

        let canonical = canonicalize_datav_components(raw).expect("reviewed overlap normalizes");
        let canonical = canonical.as_object().expect("canonical geometry object");
        let geometry = validated_polygonal_geojson(canonical).expect("canonical topology");
        assert_eq!(geometry.0.len(), 1);
    }

    #[test]
    fn datav_invalid_component_is_not_repaired_by_union() {
        let raw = json!({
            "type": "MultiPolygon",
            "coordinates": [[[[0.0, 0.0], [2.0, 2.0], [0.0, 2.0], [2.0, 0.0], [0.0, 0.0]]]]
        });
        assert!(canonicalize_datav_components(raw).is_err());
    }

    #[test]
    fn datav_shared_boundary_is_unioned_before_nonlinear_projection() {
        // Both components describe the same shared provider edge, but only the
        // right component samples its midpoint. Projecting them independently
        // before union creates a GCJ/WGS84 sliver at that edge.
        let raw = json!({
            "type": "MultiPolygon",
            "coordinates": [
                [[
                    [115.0, 39.0], [116.0, 39.0], [116.0, 40.0],
                    [115.0, 40.0], [115.0, 39.0]
                ]],
                [[
                    [116.0, 39.0], [117.0, 39.0], [117.0, 40.0],
                    [116.0, 40.0], [116.0, 39.5], [116.0, 39.0]
                ]]
            ]
        });

        let normalized = normalize_datav_geometry(raw).expect("shared boundary normalizes");
        assert!(validated_polygonal_geojson(normalized.geometry.as_object().unwrap()).is_some());
    }

    #[test]
    fn datav_admin_rows_are_stably_ordered_by_exact_adcode() {
        let value = json!({
            "type": "FeatureCollection",
            "features": [
                {"properties": {"adcode": 420200, "name": "黄石市"}},
                {"properties": {"adcode": 420100, "name": "武汉市"}},
                {"properties": {"adcode": 422800, "name": "恩施土家族苗族自治州"}}
            ]
        });

        let rows = datav_rows_from_full(&value);
        assert_eq!(
            rows.iter().map(|row| row.code.as_str()).collect::<Vec<_>>(),
            vec!["420100", "420200", "422800"]
        );
    }

    #[test]
    fn tianditu_wkt_multipolygon_parses_in_native_wgs84_without_gcj_shift() {
        let payload = json!({
            "status": 200,
            "data": {
                "district": [{
                    "gb": "156710100",
                    "name": "台北市",
                    "boundary": "MULTIPOLYGON(((121.45 25.00, 121.65 25.00, 121.65 25.20, 121.45 25.20, 121.45 25.00)))"
                }]
            }
        });
        let feature = extract_tianditu_admin_feature(&payload, "台北市", Some("710100"))
            .expect("valid Tianditu Taipei polygon");
        assert_eq!(feature.provider_object_id, "156710100");
        assert_eq!(feature.country_code.as_deref(), Some("CN"));
        let expected_bbox = [121.45, 25.00, 121.65, 25.20];
        for (actual, expected) in feature.bbox.iter().zip(expected_bbox.iter()) {
            assert!(
                (actual - expected).abs() < 1e-6,
                "bbox mismatch: actual={actual} expected={expected}"
            );
        }
    }
}
