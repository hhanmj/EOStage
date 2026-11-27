//! geoBoundaries gbOpen country/level browsing for AOI selection outside China.
//!
//! Metadata (countries, levels, licence, source) comes from the public
//! geoBoundaries API; geometry always comes from the pinned gbOpen release on
//! GitHub LFS so a bound AOI never silently changes with a new upstream build.
//! China, Taiwan, Hong Kong, Macao and units touching the product-owned
//! disputed guard regions never route here (Owner decision 2026-10-24).

use std::collections::BTreeSet;
use std::net::IpAddr;
use std::sync::OnceLock;
use std::time::Duration;

use geo::{
    BoundingRect, Coord, Geometry, InteriorPoint, Intersects, LineString, MultiPolygon, Polygon,
};
use insar_core::{ContractError, ProviderResult, ProxyPolicy};
use reqwest::blocking::Client;
use reqwest::redirect::Policy;
use serde::Deserialize;
use serde_json::{Map, Value};
use url::Url;

use crate::boundary::{
    normalize_polygonal_geometry, read_bounded_json, GlobalBoundaryProvider,
    ProviderBoundaryCandidate, ProviderBoundaryFeature, EOSTAGE_USER_AGENT,
};

pub const GEOBOUNDARIES_API_ROOT: &str = "https://www.geoboundaries.org/api/current/";
pub const GEOBOUNDARIES_DATA_ROOT: &str =
    "https://media.githubusercontent.com/media/wmgeolab/geoBoundaries/";
pub const GEOBOUNDARIES_RELEASE_TYPE: &str = "gbOpen";
/// gbOpen 6.0.0; the public API's `current` download links point at this commit.
pub const GEOBOUNDARIES_PINNED_VERSION: &str = "6.0.0";
pub const GEOBOUNDARIES_PINNED_DATA_REF: &str = "9469f09";
pub const GEOBOUNDARIES_CITATION: &str =
    "Runfola et al. (2020) geoBoundaries, PLoS ONE 15(4): e0231866";

const ROUTE_POLICY_SOURCE: &str =
    include_str!("../../../contracts/geoboundaries/geoboundaries-route-policy.v1.json");
const SERVICE_LABEL: &str = "geoBoundaries 行政边界服务";
const METADATA_TIMEOUT: Duration = Duration::from_secs(30);
const LAYER_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_METADATA_BYTES: u64 = 8 * 1024 * 1024;
/// Simplified gbOpen layers; also bounds what one cached layer can hold.
pub const GEOBOUNDARIES_MAX_LAYER_BYTES: u64 = 16 * 1024 * 1024;
const MAX_SHAPE_ID_CHARS: usize = 128;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GeoBoundariesRelease {
    pub release_type: String,
    pub version: String,
    pub data_ref: String,
}

impl GeoBoundariesRelease {
    pub fn pinned() -> Self {
        Self {
            release_type: GEOBOUNDARIES_RELEASE_TYPE.to_owned(),
            version: GEOBOUNDARIES_PINNED_VERSION.to_owned(),
            data_ref: GEOBOUNDARIES_PINNED_DATA_REF.to_owned(),
        }
    }

    /// A configured gbOpen release: a dotted version label and the Git commit
    /// (7–40 lowercase hex characters) its geometry is read from.
    pub fn new(version: &str, data_ref: &str) -> ProviderResult<Self> {
        let version_valid = !version.is_empty()
            && version.len() <= 16
            && version
                .bytes()
                .all(|byte| byte.is_ascii_digit() || byte == b'.');
        let ref_valid = (7..=40).contains(&data_ref.len())
            && data_ref
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        if !version_valid || !ref_valid {
            return Err(error(
                "BOUNDARY_PROVIDER_CONFIGURATION_INVALID",
                "geoBoundaries 发布版本配置无效",
                "release_config",
            ));
        }
        Ok(Self {
            release_type: GEOBOUNDARIES_RELEASE_TYPE.to_owned(),
            version: version.to_owned(),
            data_ref: data_ref.to_owned(),
        })
    }

    pub fn label(&self) -> String {
        format!(
            "geoBoundaries {} {} ({})",
            self.release_type, self.version, self.data_ref
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GeoBoundaryCountry {
    pub iso3: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GeoBoundaryLevel {
    pub iso3: String,
    pub level: String,
    pub canonical_name: Option<String>,
    pub unit_count: Option<u32>,
    pub source: Option<String>,
    pub license: String,
    pub license_source: Option<String>,
    pub year_represented: Option<String>,
    pub attribution: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeoBoundaryUnit {
    pub provider_object_id: String,
    pub name: String,
    pub iso3: String,
    pub level: String,
    pub geometry: Value,
    pub geometry_kind: String,
    pub bbox: [f64; 4],
}

/// A unit whose geometry could not be verified as an authoritative polygon.
/// It is listed so the user sees it exists, but it can never be bound; it is
/// not a disputed unit (those are withheld instead).
#[derive(Clone, Debug, PartialEq)]
pub struct GeoBoundaryUnavailableUnit {
    pub provider_object_id: String,
    pub name: String,
    /// Envelope of the unverified coordinates, for display only.
    pub bbox: Option<[f64; 4]>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeoBoundaryLayer {
    pub iso3: String,
    pub level: String,
    pub release: GeoBoundariesRelease,
    /// The commit the API's current links point at when it differs from the
    /// pinned release. Geometry is still read from the pinned release.
    pub release_drift: Option<String>,
    pub attribution: String,
    pub license: String,
    pub units: Vec<GeoBoundaryUnit>,
    pub withheld_disputed: usize,
    /// Identities withheld by the disputed guard; resolving one is refused
    /// as disputed rather than reported as unknown.
    pub withheld_object_ids: Vec<String>,
    /// Listed but not bindable: geometry could not be verified.
    pub unavailable_units: Vec<GeoBoundaryUnavailableUnit>,
    pub invalid_units: usize,
}

/// Country → level → unit browsing. Implementations must apply
/// [`geoboundaries_route_refusal`] before any request.
pub trait GeoBoundaryCatalog: Send + Sync {
    fn release(&self) -> GeoBoundariesRelease;
    fn countries(&self) -> ProviderResult<Vec<GeoBoundaryCountry>>;
    fn levels(&self, iso3: &str) -> ProviderResult<Vec<GeoBoundaryLevel>>;
    fn layer(&self, iso3: &str, level: &str) -> ProviderResult<GeoBoundaryLayer>;
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRoutePolicy {
    schema_version: u32,
    authority_id: String,
    owner_decision: String,
    china_scope_iso3: Vec<String>,
    disputed_iso3: Vec<String>,
    disputed_iso3_semantics: String,
    disputed_guard_regions: Vec<RawGuardRegion>,
    withheld_by_identity: Vec<RawIdentityRule>,
    withheld_by_identity_semantics: String,
    guard_semantics: String,
    expected_withheld_impact: Vec<RawImpact>,
    owner_review_required: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGuardRegion {
    id: String,
    label: String,
    sector: String,
    basis: String,
    ring: Vec<[f64; 2]>,
}

/// A unit withheld by identity: its source assigns a claimed area to it but
/// its geometry does not draw that area, so no ring can catch it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawIdentityRule {
    iso3: String,
    level: String,
    shape_id: String,
    shape_name: String,
    guard_region: String,
    basis: String,
    status: String,
}

struct IdentityRule {
    iso3: String,
    level: String,
    shape_id: String,
    shape_name: String,
}

/// Owner review record of the units a guard is expected to withhold; it is
/// checked for consistency but not used for routing.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawImpact {
    iso3: String,
    adm0: String,
    adm1: Vec<String>,
    adm2: Vec<String>,
    guard_regions: Vec<String>,
}

struct RoutePolicy {
    china_scope_iso3: BTreeSet<String>,
    disputed_iso3: BTreeSet<String>,
    guards: Vec<(String, Polygon<f64>)>,
    identities: Vec<IdentityRule>,
}

fn route_policy() -> &'static RoutePolicy {
    static POLICY: OnceLock<RoutePolicy> = OnceLock::new();
    POLICY.get_or_init(|| {
        let raw: RawRoutePolicy =
            serde_json::from_str(ROUTE_POLICY_SOURCE).expect("reviewed route policy parses");
        assert!(
            raw.schema_version == 1
                && raw.authority_id == "eostage.geoboundaries-route-policy.v1"
                && !raw.owner_decision.is_empty()
                && !raw.disputed_iso3_semantics.is_empty()
                && !raw.guard_semantics.is_empty()
                && !raw.withheld_by_identity_semantics.is_empty()
                && !raw.owner_review_required.is_empty(),
            "reviewed route policy metadata"
        );
        let china_scope_iso3 = raw.china_scope_iso3.into_iter().collect::<BTreeSet<_>>();
        for required in ["CHN", "TWN", "HKG", "MAC"] {
            assert!(
                china_scope_iso3.contains(required),
                "China scope keeps {required}"
            );
        }
        let guard_ids = raw
            .disputed_guard_regions
            .iter()
            .map(|region| region.id.as_str())
            .collect::<BTreeSet<_>>();
        for impact in &raw.expected_withheld_impact {
            assert!(
                impact.iso3.len() == 3
                    && !impact.adm0.is_empty()
                    && !impact.adm1.is_empty()
                    && !impact.adm2.is_empty()
                    && impact
                        .guard_regions
                        .iter()
                        .all(|id| guard_ids.contains(id.as_str())),
                "impact record of {} names known guard regions",
                impact.iso3
            );
        }
        let identities = raw
            .withheld_by_identity
            .into_iter()
            .map(|rule| {
                assert!(
                    normalize_iso3(&rule.iso3).ok().as_deref() == Some(rule.iso3.as_str())
                        && normalize_adm_level(&rule.level).ok().as_deref()
                            == Some(rule.level.as_str())
                        && valid_shape_id(&rule.shape_id)
                        && !rule.shape_name.trim().is_empty()
                        && guard_ids.contains(rule.guard_region.as_str())
                        && !rule.basis.is_empty()
                        && matches!(
                            rule.status.as_str(),
                            "pending_owner_confirmation" | "owner_confirmed"
                        ),
                    "identity rule {} {} is documented",
                    rule.iso3,
                    rule.shape_name
                );
                IdentityRule {
                    iso3: rule.iso3,
                    level: rule.level,
                    shape_id: rule.shape_id,
                    shape_name: rule.shape_name.trim().to_lowercase(),
                }
            })
            .collect();
        let guards = raw
            .disputed_guard_regions
            .into_iter()
            .map(|region| {
                assert!(
                    !region.label.is_empty()
                        && !region.sector.is_empty()
                        && !region.basis.is_empty()
                        && region.ring.len() >= 4
                        && region.ring.first() == region.ring.last(),
                    "guard region {} is a documented closed ring",
                    region.id
                );
                let ring = region
                    .ring
                    .iter()
                    .map(|[x, y]| Coord { x: *x, y: *y })
                    .collect::<Vec<_>>();
                (region.id, Polygon::new(LineString::from(ring), Vec::new()))
            })
            .collect();
        RoutePolicy {
            china_scope_iso3,
            disputed_iso3: raw.disputed_iso3.into_iter().collect(),
            guards,
            identities,
        }
    })
}

/// The refusal for a country code that must never use geoBoundaries.
pub fn geoboundaries_route_refusal(iso3: &str) -> Option<ContractError> {
    let iso3 = iso3.trim().to_ascii_uppercase();
    let policy = route_policy();
    if policy.china_scope_iso3.contains(&iso3) {
        return Some(error_with(
            "BOUNDARY_ROUTE_REFUSED_CHINA_SCOPE",
            "中国（含台湾、香港、澳门）行政区请使用中国行政区选择器，不使用 geoBoundaries",
            [
                ("reason", "china_scope_iso3"),
                ("fallback", "china_selector"),
            ],
        ));
    }
    if policy.disputed_iso3.contains(&iso3) {
        return Some(disputed_route_refusal("disputed_iso3"));
    }
    None
}

/// The guard region a unit geometry touches, if any.
pub fn disputed_guard_region(geometry: &MultiPolygon<f64>) -> Option<&'static str> {
    route_policy()
        .guards
        .iter()
        .find(|(_, guard)| guard.intersects(geometry))
        .map(|(id, _)| id.as_str())
}

/// Whether a unit is withheld by identity (`withheld_by_identity` in the
/// route policy): same ISO3 and level, and the same shapeID or shapeName.
pub fn withheld_by_identity(iso3: &str, level: &str, shape_id: &str, name: &str) -> bool {
    let name = name.trim().to_lowercase();
    route_policy().identities.iter().any(|rule| {
        rule.iso3 == iso3
            && rule.level == level
            && (rule.shape_id == shape_id || rule.shape_name == name)
    })
}

/// [`withheld_by_identity`] for a listed unit.
pub fn unit_withheld_by_identity(unit: &GeoBoundaryUnit) -> bool {
    parse_geoboundaries_object_id(&unit.provider_object_id).is_ok_and(|(iso3, level, shape_id)| {
        withheld_by_identity(&iso3, &level, &shape_id, &unit.name)
    })
}

/// Result of checking one unit against the disputed guard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuardCheck {
    Clear,
    Disputed(&'static str),
    /// The geometry has no readable polygon coordinates. Never treated as
    /// disputed: the unit is unavailable, not withheld.
    Unverifiable,
}

/// Checks a unit against the guard rings.
pub fn unit_guard_check(unit: &GeoBoundaryUnit) -> GuardCheck {
    match polygonal_coordinates(&unit.geometry) {
        None => GuardCheck::Unverifiable,
        Some(polygons) => match disputed_guard_region(&polygons) {
            Some(region) => GuardCheck::Disputed(region),
            None => GuardCheck::Clear,
        },
    }
}

/// Reads Polygon/MultiPolygon coordinates for the guard and the spatial
/// hierarchy only. No vertex or component bounds apply: those of
/// `validated_polygonal_geojson` protect ASF footprint projection, and real
/// administrative units (Maharashtra, Hokkaido, island regions) exceed them.
/// Bound geometry is validated separately by `normalize_polygonal_geometry`.
fn polygonal_coordinates(geometry: &Value) -> Option<MultiPolygon<f64>> {
    let geometry = geojson::Geometry::from_json_value(geometry.clone()).ok()?;
    let polygons = match Geometry::<f64>::try_from(geometry.value).ok()? {
        Geometry::Polygon(polygon) => MultiPolygon::new(vec![polygon]),
        Geometry::MultiPolygon(polygons) => polygons,
        _ => return None,
    };
    let finite = polygons.0.iter().all(|polygon| {
        std::iter::once(polygon.exterior())
            .chain(polygon.interiors())
            .all(|ring| {
                ring.0.len() >= 4
                    && ring
                        .0
                        .iter()
                        .all(|coord| coord.x.is_finite() && coord.y.is_finite())
            })
    });
    (!polygons.0.is_empty() && finite).then_some(polygons)
}

/// The refusal for a listed unit whose geometry could not be verified.
pub fn geometry_unavailable_refusal() -> ContractError {
    error(
        "BOUNDARY_GEOMETRY_UNAVAILABLE",
        "该行政单元的 geoBoundaries 几何无法校验，暂不能用作 AOI",
        "geometry_unverifiable",
    )
}

/// The refusal for a unit inside or touching a restricted boundary guard region.
pub fn disputed_route_refusal(reason: &str) -> ContractError {
    error_with(
        "BOUNDARY_ROUTE_REFUSED_DISPUTED",
        "该行政单元不提供 geoBoundaries 边界；该区域请使用中国行政区选择器",
        [("reason", reason), ("fallback", "china_selector")],
    )
}

pub fn normalize_iso3(iso3: &str) -> ProviderResult<String> {
    let iso3 = iso3.trim();
    // `ALL` is the API's wildcard, never a country.
    if iso3.len() != 3
        || !iso3.bytes().all(|byte| byte.is_ascii_alphabetic())
        || iso3.eq_ignore_ascii_case("ALL")
    {
        return Err(error(
            "BOUNDARY_PROVIDER_REQUEST_INVALID",
            "国家代码必须是三位 ISO 3166-1 字母代码",
            "iso3",
        ));
    }
    Ok(iso3.to_ascii_uppercase())
}

pub fn normalize_adm_level(level: &str) -> ProviderResult<String> {
    let level = level.trim().to_ascii_uppercase();
    match level.as_str() {
        "ADM0" | "ADM1" | "ADM2" | "ADM3" | "ADM4" | "ADM5" => Ok(level),
        _ => Err(error(
            "BOUNDARY_PROVIDER_REQUEST_INVALID",
            "行政层级必须是 ADM0 到 ADM5",
            "adm_level",
        )),
    }
}

/// Split `gb:<ISO3>:<ADMn>:<shapeID>` into its parts.
pub fn parse_geoboundaries_object_id(
    provider_object_id: &str,
) -> ProviderResult<(String, String, String)> {
    let invalid = || {
        error(
            "BOUNDARY_PROVIDER_IDENTITY_INVALID",
            "geoBoundaries 行政单元身份无效",
            "provider_object_id",
        )
    };
    let rest = provider_object_id.strip_prefix("gb:").ok_or_else(invalid)?;
    let mut parts = rest.splitn(3, ':');
    let (Some(iso3), Some(level), Some(shape_id)) = (parts.next(), parts.next(), parts.next())
    else {
        return Err(invalid());
    };
    if iso3.len() != 3 || !iso3.bytes().all(|byte| byte.is_ascii_uppercase()) {
        return Err(invalid());
    }
    let level = normalize_adm_level(level).map_err(|_| invalid())?;
    if !valid_shape_id(shape_id) {
        return Err(invalid());
    }
    Ok((iso3.to_owned(), level, shape_id.to_owned()))
}

fn valid_shape_id(shape_id: &str) -> bool {
    !shape_id.is_empty()
        && shape_id.chars().count() <= MAX_SHAPE_ID_CHARS
        && shape_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

pub fn adm_level_rank(level: &str) -> Option<u8> {
    level
        .strip_prefix("ADM")
        .and_then(|rank| rank.parse::<u8>().ok())
        .filter(|rank| *rank <= 5)
}

#[derive(Clone)]
pub struct GeoBoundariesProvider {
    metadata_client: Client,
    layer_client: Client,
    api_root: Url,
    data_root: Url,
    release: GeoBoundariesRelease,
}

impl GeoBoundariesProvider {
    pub fn production_with_route(route: ProxyPolicy) -> ProviderResult<Self> {
        Self::production_with_release(route, GeoBoundariesRelease::pinned())
    }

    pub fn production_with_release(
        route: ProxyPolicy,
        release: GeoBoundariesRelease,
    ) -> ProviderResult<Self> {
        let api_root = parse_root(GEOBOUNDARIES_API_ROOT)?;
        let data_root = parse_root(GEOBOUNDARIES_DATA_ROOT)?;
        Self::build(api_root, data_root, release, route, true)
    }

    #[doc(hidden)]
    pub fn with_test_roots(
        api_root: &str,
        data_root: &str,
        release: GeoBoundariesRelease,
    ) -> ProviderResult<Self> {
        let api_root = parse_root(api_root)?;
        let data_root = parse_root(data_root)?;
        for root in [&api_root, &data_root] {
            let loopback = root
                .host_str()
                .and_then(|host| host.parse::<IpAddr>().ok())
                .is_some_and(|address| address.is_loopback());
            if root.scheme() != "http" || !loopback {
                return Err(error(
                    "BOUNDARY_PROVIDER_CONFIGURATION_INVALID",
                    "测试 geoBoundaries 服务只允许使用本机回环 HTTP 地址",
                    "test_endpoint_scope",
                ));
            }
        }
        Self::build(api_root, data_root, release, ProxyPolicy::Direct, false)
    }

    fn build(
        api_root: Url,
        data_root: Url,
        release: GeoBoundariesRelease,
        route: ProxyPolicy,
        tls_roots: bool,
    ) -> ProviderResult<Self> {
        let client = |timeout| {
            let mut builder = Client::builder()
                .user_agent(EOSTAGE_USER_AGENT)
                .timeout(timeout)
                .redirect(Policy::none())
                .retry(reqwest::retry::never())
                .referer(false);
            if !tls_roots {
                builder = builder.tls_built_in_root_certs(false);
            }
            crate::network_proxy::blocking_builder_with_proxy(builder, route)
                .build()
                .map_err(|_| {
                    error(
                        "BOUNDARY_PROVIDER_CONFIGURATION_INVALID",
                        "无法初始化 geoBoundaries 行政边界服务",
                        "client_build",
                    )
                })
        };
        Ok(Self {
            metadata_client: client(METADATA_TIMEOUT)?,
            layer_client: client(LAYER_TIMEOUT)?,
            api_root,
            data_root,
            release,
        })
    }

    fn get_json(&self, client: &Client, url: Url, max_bytes: u64) -> ProviderResult<Value> {
        let response = client.get(url).send().map_err(|_| {
            error(
                "BOUNDARY_PROVIDER_UNAVAILABLE",
                "geoBoundaries 行政边界服务暂时不可用，请检查网络或在设置里切换代理",
                "request_failed",
            )
        })?;
        let status = response.status();
        if !status.is_success() {
            return Err(error(
                if status.as_u16() == 429 {
                    "BOUNDARY_PROVIDER_RATE_LIMITED"
                } else {
                    "BOUNDARY_PROVIDER_HTTP_ERROR"
                },
                "geoBoundaries 行政边界服务返回错误，请稍后重试",
                "http_status",
            ));
        }
        read_bounded_json(response, max_bytes, SERVICE_LABEL)
    }

    fn api_url(&self, iso3: &str, level: &str) -> ProviderResult<Url> {
        self.api_root
            .join(&format!("{}/{iso3}/{level}/", self.release.release_type))
            .map_err(|_| path_error())
    }

    fn layer_url(&self, iso3: &str, level: &str) -> ProviderResult<Url> {
        self.release_file_url(iso3, level, "_simplified.geojson")
    }

    fn release_file_url(&self, iso3: &str, level: &str, suffix: &str) -> ProviderResult<Url> {
        self.data_root
            .join(&format!(
                "{}/releaseData/{}/{iso3}/{level}/geoBoundaries-{iso3}-{level}{suffix}",
                self.release.data_ref, self.release.release_type
            ))
            .map_err(|_| path_error())
    }

    /// Source and licence of the pinned release itself. The API only serves
    /// `current`; when its links point elsewhere, its source and licence may
    /// not describe the pinned geometry, so they are read from the pinned
    /// release's metadata file, or left to that release when unreadable.
    fn pinned_level(&self, iso3: &str, level: &str) -> GeoBoundaryLevel {
        let pinned = self
            .release_file_url(iso3, level, "-metaData.json")
            .and_then(|url| self.get_json(&self.metadata_client, url, MAX_METADATA_BYTES));
        let Ok(value) = pinned else {
            return unpinned_level(iso3, level, &self.release);
        };
        let found = metadata_objects(&value).into_iter().find_map(|object| {
            parse_level(object, &self.release)
                .map(|(parsed, _)| parsed)
                .filter(|parsed| parsed.iso3 == iso3 && parsed.level == level)
        });
        found.unwrap_or_else(|| unpinned_level(iso3, level, &self.release))
    }

    fn fetch_layer(&self, iso3: &str, level: &str) -> ProviderResult<GeoBoundaryLayer> {
        let iso3 = normalize_iso3(iso3)?;
        let level = normalize_adm_level(level)?;
        if let Some(refusal) = geoboundaries_route_refusal(&iso3) {
            return Err(refusal);
        }
        let metadata = self.get_json(
            &self.metadata_client,
            self.api_url(&iso3, &level)?,
            MAX_METADATA_BYTES,
        )?;
        let metadata = metadata_objects(&metadata)
            .into_iter()
            .find_map(|object| {
                parse_level(object, &self.release)
                    .filter(|parsed| parsed.0.iso3 == iso3 && parsed.0.level == level)
            })
            .ok_or_else(|| {
                error(
                    "BOUNDARY_PROVIDER_RESPONSE_INVALID",
                    "geoBoundaries 未返回所选国家与层级的元数据",
                    "level_metadata",
                )
            })?;
        let (level_metadata, api_ref) = metadata;
        let level_metadata = if api_ref.as_deref() == Some(self.release.data_ref.as_str()) {
            level_metadata
        } else {
            self.pinned_level(&iso3, &level)
        };
        let collection = self.get_json(
            &self.layer_client,
            self.layer_url(&iso3, &level)?,
            GEOBOUNDARIES_MAX_LAYER_BYTES,
        )?;
        let features = collection
            .get("features")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                error(
                    "BOUNDARY_PROVIDER_RESPONSE_INVALID",
                    "geoBoundaries 图层缺少要素数组",
                    "layer_features",
                )
            })?;

        let mut units = Vec::new();
        let mut seen = BTreeSet::new();
        let mut withheld = BTreeSet::new();
        let mut unavailable = Vec::new();
        let mut invalid_units = 0;
        for feature in features {
            let Some(properties) = feature.get("properties").and_then(Value::as_object) else {
                invalid_units += 1;
                continue;
            };
            let text = |key: &str| clean_text(properties.get(key));
            let shape_id = text("shapeID").filter(|value| valid_shape_id(value));
            let group_matches = text("shapeGroup").is_some_and(|value| value == iso3);
            let type_matches =
                text("shapeType").is_some_and(|value| value.eq_ignore_ascii_case(&level));
            let (Some(shape_id), Some(name), true, true) =
                (shape_id, text("shapeName"), group_matches, type_matches)
            else {
                invalid_units += 1;
                continue;
            };
            let raw_geometry = feature.get("geometry").cloned().unwrap_or(Value::Null);
            // Not polygonal at all: not a boundary unit.
            let Some(raw_polygons) = polygonal_coordinates(&raw_geometry) else {
                invalid_units += 1;
                continue;
            };
            if !seen.insert(shape_id.clone()) {
                invalid_units += 1;
                continue;
            }
            let provider_object_id = format!("gb:{iso3}:{level}:{shape_id}");
            if withheld_by_identity(&iso3, &level, &shape_id, &name) {
                withheld.insert(provider_object_id);
                continue;
            }
            let Ok(geometry) = normalize_polygonal_geometry(raw_geometry) else {
                // The guard still applies to unverified coordinates; only a
                // unit clear of every ring is listed as unavailable.
                if disputed_guard_region(&raw_polygons).is_some() {
                    withheld.insert(provider_object_id);
                } else {
                    unavailable.push(GeoBoundaryUnavailableUnit {
                        provider_object_id,
                        name,
                        bbox: raw_polygons
                            .bounding_rect()
                            .map(|rect| [rect.min().x, rect.min().y, rect.max().x, rect.max().y]),
                    });
                }
                continue;
            };
            let unit = GeoBoundaryUnit {
                provider_object_id,
                name,
                iso3: iso3.clone(),
                level: level.clone(),
                geometry: geometry.geometry,
                geometry_kind: geometry.geometry_kind,
                bbox: geometry.bbox,
            };
            match unit_guard_check(&unit) {
                GuardCheck::Clear => units.push(unit),
                GuardCheck::Disputed(_) => {
                    withheld.insert(unit.provider_object_id);
                }
                GuardCheck::Unverifiable => unavailable.push(GeoBoundaryUnavailableUnit {
                    provider_object_id: unit.provider_object_id,
                    name: unit.name,
                    bbox: Some(unit.bbox),
                }),
            }
        }
        units.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then_with(|| left.provider_object_id.cmp(&right.provider_object_id))
        });
        unavailable.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then_with(|| left.provider_object_id.cmp(&right.provider_object_id))
        });
        if units.is_empty() && withheld.is_empty() && unavailable.is_empty() {
            // A layer with no usable unit is a broken response, never an
            // empty country; it must not be cached as a success.
            return Err(error(
                "BOUNDARY_PROVIDER_RESPONSE_INVALID",
                "geoBoundaries 图层没有可用的行政单元",
                "layer_units",
            ));
        }
        let release_drift = api_ref.filter(|reference| *reference != self.release.data_ref);
        Ok(GeoBoundaryLayer {
            iso3,
            level,
            release: self.release.clone(),
            release_drift,
            attribution: level_metadata.attribution,
            license: level_metadata.license,
            units,
            withheld_disputed: withheld.len(),
            withheld_object_ids: withheld.into_iter().collect(),
            unavailable_units: unavailable,
            invalid_units,
        })
    }
}

impl GeoBoundaryCatalog for GeoBoundariesProvider {
    fn release(&self) -> GeoBoundariesRelease {
        self.release.clone()
    }

    fn countries(&self) -> ProviderResult<Vec<GeoBoundaryCountry>> {
        let value = self.get_json(
            &self.metadata_client,
            self.api_url("ALL", "ADM0")?,
            MAX_METADATA_BYTES,
        )?;
        let mut seen = BTreeSet::new();
        let mut countries = metadata_objects(&value)
            .into_iter()
            .filter_map(|object| {
                let iso3 = clean_text(object.get("boundaryISO"))?;
                let name = clean_text(object.get("boundaryName"))?;
                let iso3 = normalize_iso3(&iso3).ok()?;
                (geoboundaries_route_refusal(&iso3).is_none() && seen.insert(iso3.clone()))
                    .then_some(GeoBoundaryCountry { iso3, name })
            })
            .collect::<Vec<_>>();
        countries.sort_by(|left, right| left.iso3.cmp(&right.iso3));
        if countries.is_empty() {
            return Err(error(
                "BOUNDARY_PROVIDER_RESPONSE_INVALID",
                "geoBoundaries 未返回可用的国家列表",
                "country_list",
            ));
        }
        Ok(countries)
    }

    fn levels(&self, iso3: &str) -> ProviderResult<Vec<GeoBoundaryLevel>> {
        let iso3 = normalize_iso3(iso3)?;
        if let Some(refusal) = geoboundaries_route_refusal(&iso3) {
            return Err(refusal);
        }
        let value = self.get_json(
            &self.metadata_client,
            self.api_url(&iso3, "ALL")?,
            MAX_METADATA_BYTES,
        )?;
        let mut levels = metadata_objects(&value)
            .into_iter()
            .filter_map(|object| parse_level(object, &self.release))
            .filter(|(level, _)| level.iso3 == iso3)
            .map(|(level, api_ref)| {
                if api_ref.as_deref() == Some(self.release.data_ref.as_str()) {
                    level
                } else {
                    GeoBoundaryLevel {
                        canonical_name: level.canonical_name,
                        ..unpinned_level(&level.iso3, &level.level, &self.release)
                    }
                }
            })
            .collect::<Vec<_>>();
        levels.sort_by(|left, right| left.level.cmp(&right.level));
        levels.dedup_by(|left, right| left.level == right.level);
        if levels.is_empty() {
            return Err(error(
                "BOUNDARY_PROVIDER_RESPONSE_INVALID",
                "geoBoundaries 未返回所选国家的可用层级",
                "level_list",
            ));
        }
        Ok(levels)
    }

    fn layer(&self, iso3: &str, level: &str) -> ProviderResult<GeoBoundaryLayer> {
        self.fetch_layer(iso3, level)
    }
}

impl GlobalBoundaryProvider for GeoBoundariesProvider {
    fn search(&self, _query: &str, _limit: usize) -> ProviderResult<Vec<ProviderBoundaryFeature>> {
        Err(browse_only())
    }

    fn discover(
        &self,
        _query: &str,
        _limit: usize,
    ) -> ProviderResult<Vec<ProviderBoundaryCandidate>> {
        Err(browse_only())
    }

    fn resolve(&self, provider_object_id: &str) -> ProviderResult<ProviderBoundaryFeature> {
        let (iso3, level, _) = parse_geoboundaries_object_id(provider_object_id)?;
        if let Some(refusal) = geoboundaries_route_refusal(&iso3) {
            return Err(refusal);
        }
        let layer = self.fetch_layer(&iso3, &level)?;
        if layer
            .withheld_object_ids
            .iter()
            .any(|withheld| withheld == provider_object_id)
        {
            return Err(disputed_route_refusal("disputed_guard_region"));
        }
        if layer
            .unavailable_units
            .iter()
            .any(|unit| unit.provider_object_id == provider_object_id)
        {
            return Err(geometry_unavailable_refusal());
        }
        let unit = layer
            .units
            .into_iter()
            .find(|unit| unit.provider_object_id == provider_object_id)
            .ok_or_else(|| {
                error(
                    "BOUNDARY_PROVIDER_IDENTITY_MISMATCH",
                    "geoBoundaries 图层中没有所选行政单元",
                    "provider_object_id",
                )
            })?;
        Ok(unit_feature(unit, &layer.release, &layer.attribution))
    }

    fn discover_china(
        &self,
        _query: &str,
        _limit: usize,
    ) -> ProviderResult<Vec<ProviderBoundaryCandidate>> {
        Err(geoboundaries_route_refusal("CHN").expect("China scope is always refused"))
    }

    fn search_taiwan(
        &self,
        _query: &str,
        _limit: usize,
    ) -> ProviderResult<Vec<ProviderBoundaryFeature>> {
        Err(geoboundaries_route_refusal("TWN").expect("Taiwan is always refused"))
    }
}

/// The units whose interior point lies inside `parent` (a coarser level of
/// the same country). geoBoundaries layers carry no parent identity, so the
/// hierarchy is spatial.
pub fn units_inside<'a>(
    parent: &GeoBoundaryUnit,
    units: impl IntoIterator<Item = &'a GeoBoundaryUnit>,
) -> Vec<&'a GeoBoundaryUnit> {
    let Some(parent) = polygonal_coordinates(&parent.geometry) else {
        return Vec::new();
    };
    units
        .into_iter()
        .filter(|unit| {
            polygonal_coordinates(&unit.geometry)
                .and_then(|polygons| polygons.interior_point())
                .is_some_and(|point| parent.intersects(&point))
        })
        .collect()
}

/// The unavailable units whose envelope centre lies inside `parent`; their
/// geometry was not verified, so only the envelope is used.
pub fn unavailable_units_inside<'a>(
    parent: &GeoBoundaryUnit,
    units: impl IntoIterator<Item = &'a GeoBoundaryUnavailableUnit>,
) -> Vec<&'a GeoBoundaryUnavailableUnit> {
    let Some(parent) = polygonal_coordinates(&parent.geometry) else {
        return Vec::new();
    };
    units
        .into_iter()
        .filter(|unit| {
            unit.bbox.is_some_and(|[west, south, east, north]| {
                parent.intersects(&Coord {
                    x: (west + east) / 2.0,
                    y: (south + north) / 2.0,
                })
            })
        })
        .collect()
}

/// The boundary feature an AOI binds for one geoBoundaries unit.
pub fn unit_feature(
    unit: GeoBoundaryUnit,
    release: &GeoBoundariesRelease,
    attribution: &str,
) -> ProviderBoundaryFeature {
    ProviderBoundaryFeature {
        display_name: format!("{}（{} {}）", unit.name, unit.iso3, unit.level),
        provider_object_id: unit.provider_object_id,
        name: unit.name,
        country_code: None,
        admin_level: Some(unit.level),
        geometry: unit.geometry,
        geometry_kind: unit.geometry_kind,
        bbox: unit.bbox,
        source: release.label(),
        attribution: Some(attribution.to_owned()),
    }
}

fn metadata_objects(value: &Value) -> Vec<&Map<String, Value>> {
    match value {
        Value::Array(items) => items.iter().filter_map(Value::as_object).collect(),
        Value::Object(object) => vec![object],
        _ => Vec::new(),
    }
}

/// One level's metadata and the release commit its download links point at.
fn parse_level(
    object: &Map<String, Value>,
    release: &GeoBoundariesRelease,
) -> Option<(GeoBoundaryLevel, Option<String>)> {
    let text = |key: &str| clean_text(object.get(key));
    let iso3 = normalize_iso3(&text("boundaryISO")?).ok()?;
    let level = normalize_adm_level(&text("boundaryType")?).ok()?;
    let canonical_name = text("boundaryCanonical")
        .filter(|name| !matches!(name.to_ascii_lowercase().as_str(), "unknown" | "gbopen"));
    let unit_count = text("admUnitCount")
        .and_then(|count| count.parse::<f64>().ok())
        .filter(|count| count.is_finite() && *count >= 0.0 && count.fract() == 0.0)
        .and_then(|count| u32::try_from(count as u64).ok());
    let source = text("boundarySource");
    let license = text("boundaryLicense")
        .unwrap_or_else(|| "未注明（以 geoBoundaries 元数据为准）".to_owned());
    let attribution = match &source {
        Some(source) => format!(
            "{}，来源：{source}；许可：{license}；引用：{GEOBOUNDARIES_CITATION}",
            release.label()
        ),
        None => format!(
            "{}；许可：{license}；引用：{GEOBOUNDARIES_CITATION}",
            release.label()
        ),
    };
    let api_ref = text("simplifiedGeometryGeoJSON")
        .or_else(|| text("gjDownloadURL"))
        .and_then(|url| release_ref_from_download_url(&url));
    Some((
        GeoBoundaryLevel {
            iso3,
            level,
            canonical_name,
            unit_count,
            source,
            license,
            license_source: text("licenseSource"),
            year_represented: text("boundaryYearRepresented"),
            attribution,
        },
        api_ref,
    ))
}

/// Level metadata when only the API's `current` metadata is at hand and it
/// does not describe the pinned release: no source or licence is borrowed
/// from another build.
fn unpinned_level(iso3: &str, level: &str, release: &GeoBoundariesRelease) -> GeoBoundaryLevel {
    let label = release.label();
    GeoBoundaryLevel {
        iso3: iso3.to_owned(),
        level: level.to_owned(),
        canonical_name: None,
        unit_count: None,
        source: None,
        license: format!("以 {label} 元数据为准"),
        license_source: None,
        year_represented: None,
        attribution: format!(
            "{label}；来源与许可以该版本元数据为准；引用：{GEOBOUNDARIES_CITATION}"
        ),
    }
}

fn release_ref_from_download_url(url: &str) -> Option<String> {
    let url = Url::parse(url).ok()?;
    if url.host_str() != Some("github.com") {
        return None;
    }
    let mut segments = url.path_segments()?;
    let (Some("wmgeolab"), Some("geoBoundaries"), Some("raw"), Some(reference)) = (
        segments.next(),
        segments.next(),
        segments.next(),
        segments.next(),
    ) else {
        return None;
    };
    Some(reference.to_owned())
}

/// API strings, with pandas' `nan` and empty values treated as missing.
fn clean_text(value: Option<&Value>) -> Option<String> {
    let text = match value? {
        Value::String(text) => text.trim().to_owned(),
        Value::Number(number) => number.to_string(),
        _ => return None,
    };
    (!text.is_empty() && !matches!(text.to_ascii_lowercase().as_str(), "nan" | "none" | "null"))
        .then_some(text)
}

fn parse_root(root: &str) -> ProviderResult<Url> {
    Url::parse(root).map_err(|_| {
        error(
            "BOUNDARY_PROVIDER_CONFIGURATION_INVALID",
            "geoBoundaries 服务地址无效",
            "endpoint",
        )
    })
}

fn path_error() -> ContractError {
    error(
        "BOUNDARY_PROVIDER_CONFIGURATION_INVALID",
        "geoBoundaries 请求路径无效",
        "path",
    )
}

fn browse_only() -> ContractError {
    error(
        "BOUNDARY_PROVIDER_REQUEST_INVALID",
        "geoBoundaries 只支持按国家与层级浏览，不支持自由文本搜索",
        "geoboundaries_browse_only",
    )
}

fn error(code: &str, message: &str, reason: &str) -> ContractError {
    error_with(code, message, [("reason", reason)])
}

fn error_with<const N: usize>(
    code: &str,
    message: &str,
    context: [(&str, &str); N],
) -> ContractError {
    ContractError::new(
        code,
        message,
        context
            .into_iter()
            .map(|(key, value)| (key.to_owned(), Value::String(value.to_owned())))
            .collect(),
    )
    .expect("bounded geoBoundaries error context is safe")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reviewed_route_policy_loads_and_guards_the_china_scope() {
        for iso3 in ["CHN", "TWN", "HKG", "MAC", "chn"] {
            assert!(geoboundaries_route_refusal(iso3).is_some(), "{iso3}");
        }
        assert!(geoboundaries_route_refusal("NPL").is_none());
        for wildcard in ["ALL", "all", " All "] {
            assert!(normalize_iso3(wildcard).is_err(), "{wildcard}");
        }
        let ids = route_policy()
            .guards
            .iter()
            .map(|(id, _)| id.as_str())
            .collect::<BTreeSet<_>>();
        for required in [
            "guard-east-01",
            "guard-east-02",
            "guard-west-01",
            "guard-west-02",
            "guard-middle-01",
            "guard-middle-02",
            "guard-middle-03",
            "guard-middle-04",
            "guard-southwest-01",
            "guard-maritime-01",
            "guard-maritime-02",
            "guard-maritime-03",
            "guard-maritime-04",
        ] {
            assert!(ids.contains(required), "{required}");
        }
    }

    #[test]
    fn download_url_release_ref_is_read_only_from_the_geoboundaries_repository() {
        assert_eq!(
            release_ref_from_download_url(
                "https://github.com/wmgeolab/geoBoundaries/raw/9469f09/releaseData/gbOpen/NPL/ADM1/x.geojson"
            )
            .as_deref(),
            Some("9469f09")
        );
        assert_eq!(
            release_ref_from_download_url("https://github.com/other/geoBoundaries/raw/abc/x"),
            None
        );
    }
}
