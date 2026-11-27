//! geoBoundaries country/level browsing for AOI selection outside China.
//!
//! China, Taiwan, Hong Kong and Macao are refused here as well as in the
//! provider so an injected catalog cannot bypass the route rule.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use insar_core::{ContractError, ProviderResult};
use insar_provider::boundary::{
    ChinaAdminOptions, ChinaBoundaryProvider, GlobalBoundaryProvider, ProviderBoundaryCandidate,
    ProviderBoundaryFeature,
};
use insar_provider::geoboundaries::{
    adm_level_rank, disputed_route_refusal, geoboundaries_route_refusal,
    geometry_unavailable_refusal, normalize_adm_level, normalize_iso3,
    parse_geoboundaries_object_id, unavailable_units_inside, unit_feature, unit_guard_check,
    unit_withheld_by_identity, units_inside, withheld_by_identity, GeoBoundariesRelease,
    GeoBoundaryCatalog, GeoBoundaryCountry, GeoBoundaryLayer, GeoBoundaryLevel, GuardCheck,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::{ApplicationService, ApplicationServiceError};
use crate::admin_boundaries::{AdminBoundaryBbox, AdminBoundaryResult};

const CACHE_TTL: Duration = Duration::from_secs(30 * 60);
const MAX_CACHED_LAYERS: usize = 6;
const MAX_CACHED_LEVEL_LISTS: usize = 64;
const DEFAULT_UNIT_LIMIT: usize = 500;
const MAX_UNIT_LIMIT: usize = 2_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeoBoundaryReleaseInfo {
    pub release_type: String,
    pub version: String,
    pub data_ref: String,
    pub label: String,
}

impl From<GeoBoundariesRelease> for GeoBoundaryReleaseInfo {
    fn from(release: GeoBoundariesRelease) -> Self {
        Self {
            label: release.label(),
            release_type: release.release_type,
            version: release.version,
            data_ref: release.data_ref,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeoBoundaryCountryOption {
    pub iso3: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeoBoundaryCountriesResult {
    pub ok: bool,
    pub provider: String,
    pub release: GeoBoundaryReleaseInfo,
    pub countries: Vec<GeoBoundaryCountryOption>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GeoBoundaryLevelsRequest {
    pub iso3: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeoBoundaryLevelOption {
    pub level: String,
    pub canonical_name: Option<String>,
    pub unit_count: Option<u32>,
    pub source: Option<String>,
    pub license: String,
    pub attribution: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeoBoundaryLevelsResult {
    pub ok: bool,
    pub provider: String,
    pub release: GeoBoundaryReleaseInfo,
    pub iso3: String,
    pub levels: Vec<GeoBoundaryLevelOption>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GeoBoundaryUnitsRequest {
    pub iso3: String,
    pub level: String,
    /// A unit of a coarser level of the same country; only units whose
    /// interior lies inside it are listed.
    pub parent_provider_object_id: Option<String>,
    pub query: String,
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GeoBoundaryUnitOption {
    pub provider_object_id: String,
    pub name: String,
    /// Display envelope; absent only for an unavailable unit whose
    /// coordinates could not be read.
    pub bbox: Option<AdminBoundaryBbox>,
    /// False when the unit's geometry could not be verified: it is listed,
    /// but resolving it fails with `BOUNDARY_GEOMETRY_UNAVAILABLE`.
    pub bindable: bool,
    /// `geometry_unverifiable` for a unit that is not bindable.
    pub unavailable_reason: Option<String>,
}

/// One typed note about a listed layer, alongside the joined `warning` text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeoBoundaryNotice {
    /// `GEOBOUNDARIES_RELEASE_DRIFT`, `GEOBOUNDARIES_WITHHELD_DISPUTED`,
    /// `GEOBOUNDARIES_GEOMETRY_UNAVAILABLE` or `GEOBOUNDARIES_FEATURES_INVALID`.
    pub code: String,
    pub count: Option<usize>,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GeoBoundaryUnitsResult {
    pub ok: bool,
    pub provider: String,
    pub release: GeoBoundaryReleaseInfo,
    pub iso3: String,
    pub level: String,
    pub attribution: String,
    pub license: String,
    /// Matching units before `limit` is applied.
    pub total: usize,
    pub units: Vec<GeoBoundaryUnitOption>,
    pub withheld_disputed: usize,
    /// Listed units whose geometry could not be verified (not bindable).
    pub unavailable: usize,
    pub notices: Vec<GeoBoundaryNotice>,
    pub warning: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeoBoundaryResolveRequest {
    pub provider_object_id: String,
}

type Timed<T> = (Instant, Arc<T>);

/// In-memory cache of successful responses only; a failure is never stored,
/// so the next request reaches the provider again.
#[derive(Default)]
pub(crate) struct GeoBoundaryCache {
    countries: Mutex<Option<Timed<Vec<GeoBoundaryCountry>>>>,
    levels: Mutex<BTreeMap<String, Timed<Vec<GeoBoundaryLevel>>>>,
    layers: Mutex<BTreeMap<String, Timed<GeoBoundaryLayer>>>,
}

fn fresh<T>(entry: Option<&Timed<T>>) -> Option<Arc<T>> {
    entry
        .filter(|(inserted, _)| inserted.elapsed() <= CACHE_TTL)
        .map(|(_, value)| Arc::clone(value))
}

fn insert_bounded<T>(map: &mut BTreeMap<String, Timed<T>>, key: String, value: Arc<T>, max: usize) {
    map.retain(|_, (inserted, _)| inserted.elapsed() <= CACHE_TTL);
    if map.len() >= max && !map.contains_key(&key) {
        if let Some(oldest) = map
            .iter()
            .min_by_key(|(_, (inserted, _))| *inserted)
            .map(|(key, _)| key.clone())
        {
            map.remove(&oldest);
        }
    }
    map.insert(key, (Instant::now(), value));
}

impl GeoBoundaryCache {
    fn countries(
        &self,
        catalog: &dyn GeoBoundaryCatalog,
    ) -> Result<Arc<Vec<GeoBoundaryCountry>>, ApplicationServiceError> {
        if let Some(value) = fresh(
            self.countries
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?
                .as_ref(),
        ) {
            return Ok(value);
        }
        // Not held across the request: a slow or stuck request must not
        // block other callers.
        let value = Arc::new(
            catalog
                .countries()
                .map_err(ApplicationServiceError::Provider)?,
        );
        if value.is_empty() {
            return Err(empty_response(
                "geoBoundaries 未返回可用的国家列表",
                "country_list",
            ));
        }
        *self
            .countries
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)? =
            Some((Instant::now(), Arc::clone(&value)));
        Ok(value)
    }

    fn levels(
        &self,
        catalog: &dyn GeoBoundaryCatalog,
        iso3: &str,
    ) -> Result<Arc<Vec<GeoBoundaryLevel>>, ApplicationServiceError> {
        if let Some(value) = fresh(
            self.levels
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?
                .get(iso3),
        ) {
            return Ok(value);
        }
        let value = Arc::new(
            catalog
                .levels(iso3)
                .map_err(ApplicationServiceError::Provider)?,
        );
        if value.is_empty() {
            return Err(empty_response(
                "geoBoundaries 未返回所选国家的可用层级",
                "level_list",
            ));
        }
        let mut cached = self
            .levels
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?;
        insert_bounded(
            &mut *cached,
            iso3.to_owned(),
            Arc::clone(&value),
            MAX_CACHED_LEVEL_LISTS,
        );
        Ok(value)
    }

    fn layer(
        &self,
        catalog: &dyn GeoBoundaryCatalog,
        iso3: &str,
        level: &str,
    ) -> Result<Arc<GeoBoundaryLayer>, ApplicationServiceError> {
        let key = format!("{iso3}|{level}");
        if let Some(value) = fresh(
            self.layers
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?
                .get(&key),
        ) {
            return Ok(value);
        }
        // The layer download can take tens of seconds; the cache lock is not
        // held across it.
        let value = Arc::new(
            catalog
                .layer(iso3, level)
                .map_err(ApplicationServiceError::Provider)?,
        );
        if value.units.is_empty()
            && value.withheld_disputed == 0
            && value.unavailable_units.is_empty()
        {
            return Err(empty_response(
                "geoBoundaries 图层没有可用的行政单元",
                "layer_units",
            ));
        }
        let mut cached = self
            .layers
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?;
        insert_bounded(&mut *cached, key, Arc::clone(&value), MAX_CACHED_LAYERS);
        Ok(value)
    }
}

/// Stand-in when geoBoundaries is not available: injected boundary fixtures
/// that never configure it, or a production provider that failed to build
/// (its error is kept and returned).
pub(crate) struct UnavailableGeoCatalog {
    error: ContractError,
}

impl UnavailableGeoCatalog {
    pub(crate) fn not_configured() -> Self {
        Self {
            error: unavailable(),
        }
    }

    pub(crate) fn from_error(error: ContractError) -> Self {
        Self { error }
    }
}

impl GeoBoundaryCatalog for UnavailableGeoCatalog {
    fn release(&self) -> GeoBoundariesRelease {
        GeoBoundariesRelease::pinned()
    }

    fn countries(&self) -> ProviderResult<Vec<GeoBoundaryCountry>> {
        Err(self.error.clone())
    }

    fn levels(&self, _iso3: &str) -> ProviderResult<Vec<GeoBoundaryLevel>> {
        Err(self.error.clone())
    }

    fn layer(&self, _iso3: &str, _level: &str) -> ProviderResult<GeoBoundaryLayer> {
        Err(self.error.clone())
    }
}

fn unavailable() -> ContractError {
    ContractError::new(
        "BOUNDARY_PROVIDER_UNAVAILABLE",
        "geoBoundaries 行政边界服务未配置",
        Map::from_iter([(
            "reason".to_owned(),
            Value::String("geoboundaries_not_configured".to_owned()),
        )]),
    )
    .expect("bounded error context")
}

fn provider_error(error: ContractError) -> ApplicationServiceError {
    ApplicationServiceError::Provider(error)
}

/// An empty list or layer is a broken response, never cached as a success.
fn empty_response(message: &str, reason: &str) -> ApplicationServiceError {
    provider_error(
        ContractError::new(
            "BOUNDARY_PROVIDER_RESPONSE_INVALID",
            message,
            Map::from_iter([("reason".to_owned(), Value::String(reason.to_owned()))]),
        )
        .expect("bounded error context"),
    )
}

fn checked_iso3(iso3: &str) -> Result<String, ApplicationServiceError> {
    let iso3 = normalize_iso3(iso3).map_err(provider_error)?;
    match geoboundaries_route_refusal(&iso3) {
        Some(refusal) => Err(provider_error(refusal)),
        None => Ok(iso3),
    }
}

fn bbox(bbox: [f64; 4]) -> AdminBoundaryBbox {
    AdminBoundaryBbox {
        west: bbox[0],
        south: bbox[1],
        east: bbox[2],
        north: bbox[3],
        crs: "EPSG:4326".to_owned(),
    }
}

fn identity_mismatch(message: &str) -> ApplicationServiceError {
    provider_error(
        ContractError::new(
            "BOUNDARY_PROVIDER_IDENTITY_MISMATCH",
            message,
            Map::from_iter([(
                "reason".to_owned(),
                Value::String("provider_object_id".to_owned()),
            )]),
        )
        .expect("bounded error context"),
    )
}

fn request_invalid(message: &str, reason: &str) -> ApplicationServiceError {
    provider_error(
        ContractError::new(
            "BOUNDARY_PROVIDER_REQUEST_INVALID",
            message,
            Map::from_iter([("reason".to_owned(), Value::String(reason.to_owned()))]),
        )
        .expect("bounded error context"),
    )
}

fn normalized_query(value: &str) -> String {
    value
        .trim()
        .chars()
        .filter(|character| !character.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

impl ApplicationService {
    /// Explicit geoBoundaries injection for offline contract tests.
    #[doc(hidden)]
    pub fn with_geo_boundary_catalog(
        config: super::ApplicationServiceConfig,
        catalog: Arc<dyn GeoBoundaryCatalog>,
    ) -> Self {
        let mut service = Self::new(config);
        service.boundary_service = crate::boundary_service::BoundaryService::with_geo_catalog(
            Arc::new(NoGlobalProvider),
            Arc::new(NoChinaProvider),
            catalog,
        );
        service
    }

    pub fn geo_boundary_countries(
        &self,
    ) -> Result<GeoBoundaryCountriesResult, ApplicationServiceError> {
        self.use_saved_boundary_route();
        let catalog = self.boundary_service.geo_catalog();
        let countries = self
            .boundary_service
            .geo_cache()
            .countries(catalog.as_ref())?;
        Ok(GeoBoundaryCountriesResult {
            ok: true,
            provider: "geoboundaries".to_owned(),
            release: catalog.release().into(),
            countries: countries
                .iter()
                .filter(|country| geoboundaries_route_refusal(&country.iso3).is_none())
                .map(|country| GeoBoundaryCountryOption {
                    iso3: country.iso3.clone(),
                    name: country.name.clone(),
                })
                .collect(),
        })
    }

    pub fn geo_boundary_levels(
        &self,
        request: &GeoBoundaryLevelsRequest,
    ) -> Result<GeoBoundaryLevelsResult, ApplicationServiceError> {
        let iso3 = checked_iso3(&request.iso3)?;
        self.use_saved_boundary_route();
        let catalog = self.boundary_service.geo_catalog();
        let levels = self
            .boundary_service
            .geo_cache()
            .levels(catalog.as_ref(), &iso3)?;
        Ok(GeoBoundaryLevelsResult {
            ok: true,
            provider: "geoboundaries".to_owned(),
            release: catalog.release().into(),
            levels: levels
                .iter()
                .map(|level| GeoBoundaryLevelOption {
                    level: level.level.clone(),
                    canonical_name: level.canonical_name.clone(),
                    unit_count: level.unit_count,
                    source: level.source.clone(),
                    license: level.license.clone(),
                    attribution: level.attribution.clone(),
                })
                .collect(),
            iso3,
        })
    }

    pub fn geo_boundary_units(
        &self,
        request: &GeoBoundaryUnitsRequest,
    ) -> Result<GeoBoundaryUnitsResult, ApplicationServiceError> {
        let iso3 = checked_iso3(&request.iso3)?;
        let level = normalize_adm_level(&request.level).map_err(provider_error)?;
        self.use_saved_boundary_route();
        let catalog = self.boundary_service.geo_catalog();
        let cache = self.boundary_service.geo_cache();
        let parent = match request.parent_provider_object_id.as_deref() {
            None => None,
            Some(parent_id) => {
                let (parent_iso3, parent_level, _) =
                    parse_geoboundaries_object_id(parent_id).map_err(provider_error)?;
                if parent_iso3 != iso3 || adm_level_rank(&parent_level) >= adm_level_rank(&level) {
                    return Err(request_invalid(
                        "上级行政单元必须属于同一国家且层级更高",
                        "parent_scope",
                    ));
                }
                let parent_layer = cache.layer(catalog.as_ref(), &iso3, &parent_level)?;
                let parent = parent_layer
                    .units
                    .iter()
                    .find(|unit| unit.provider_object_id == parent_id)
                    .cloned()
                    .ok_or_else(|| identity_mismatch("geoBoundaries 图层中没有所选上级行政单元"))?;
                Some(parent)
            }
        };
        let layer = cache.layer(catalog.as_ref(), &iso3, &level)?;
        let query = normalized_query(&request.query);
        let limit = request
            .limit
            .unwrap_or(DEFAULT_UNIT_LIMIT)
            .clamp(1, MAX_UNIT_LIMIT);
        // Identity withholding is applied here as well, so a catalog that did
        // not withhold such a unit cannot list it.
        let identity_withheld = |id: &str, name: &str| {
            parse_geoboundaries_object_id(id)
                .is_ok_and(|(iso3, level, shape)| withheld_by_identity(&iso3, &level, &shape, name))
        };
        let extra_withheld = layer
            .units
            .iter()
            .filter(|unit| unit_withheld_by_identity(unit))
            .count()
            + layer
                .unavailable_units
                .iter()
                .filter(|unit| identity_withheld(&unit.provider_object_id, &unit.name))
                .count();
        let withheld_disputed = layer.withheld_disputed + extra_withheld;
        let name_matches = |name: &str| query.is_empty() || normalized_query(name).contains(&query);
        let named = layer
            .units
            .iter()
            .filter(|unit| !unit_withheld_by_identity(unit) && name_matches(&unit.name));
        let named_unavailable = layer.unavailable_units.iter().filter(|unit| {
            !identity_withheld(&unit.provider_object_id, &unit.name) && name_matches(&unit.name)
        });
        let (matching, matching_unavailable) = match &parent {
            Some(parent) => (
                units_inside(parent, named),
                unavailable_units_inside(parent, named_unavailable),
            ),
            None => (named.collect(), named_unavailable.collect()),
        };
        let mut notices = Vec::new();
        if let Some(drift) = &layer.release_drift {
            notices.push(GeoBoundaryNotice {
                code: "GEOBOUNDARIES_RELEASE_DRIFT".to_owned(),
                count: None,
                message: format!(
                    "geoBoundaries 当前发布已更新（{drift}），几何仍按固定版本 {} 获取",
                    layer.release.label()
                ),
            });
        }
        // The hint points at the whole level view; under a parent filter or
        // keyword query the withheld units are not in scope, so the pointer
        // would mislead. The count itself stays in the payload.
        if withheld_disputed > 0 && parent.is_none() && query.is_empty() {
            notices.push(GeoBoundaryNotice {
                code: "GEOBOUNDARIES_WITHHELD_DISPUTED".to_owned(),
                count: Some(withheld_disputed),
                message: format!(
                    "已隐藏 {withheld_disputed} 个需通过中国行政区选择器获取的行政单元，该区域请使用中国行政区选择器"
                ),
            });
        }
        if !matching_unavailable.is_empty() {
            notices.push(GeoBoundaryNotice {
                code: "GEOBOUNDARIES_GEOMETRY_UNAVAILABLE".to_owned(),
                count: Some(matching_unavailable.len()),
                message: format!(
                    "{} 个行政单元的几何无法校验，已列出但暂不能用作 AOI",
                    matching_unavailable.len()
                ),
            });
        }
        if layer.invalid_units > 0 {
            notices.push(GeoBoundaryNotice {
                code: "GEOBOUNDARIES_FEATURES_INVALID".to_owned(),
                count: Some(layer.invalid_units),
                message: format!(
                    "geoBoundaries 图层中有 {} 个要素无效，已跳过",
                    layer.invalid_units
                ),
            });
        }
        let warning = (!notices.is_empty()).then(|| {
            notices
                .iter()
                .map(|notice| notice.message.as_str())
                .collect::<Vec<_>>()
                .join("；")
        });
        let total = matching.len() + matching_unavailable.len();
        let options = matching
            .into_iter()
            .map(|unit| GeoBoundaryUnitOption {
                provider_object_id: unit.provider_object_id.clone(),
                name: unit.name.clone(),
                bbox: Some(bbox(unit.bbox)),
                bindable: true,
                unavailable_reason: None,
            })
            .chain(
                matching_unavailable
                    .into_iter()
                    .map(|unit| GeoBoundaryUnitOption {
                        provider_object_id: unit.provider_object_id.clone(),
                        name: unit.name.clone(),
                        bbox: unit.bbox.map(bbox),
                        bindable: false,
                        unavailable_reason: Some("geometry_unverifiable".to_owned()),
                    }),
            )
            .take(limit)
            .collect();
        Ok(GeoBoundaryUnitsResult {
            ok: true,
            provider: "geoboundaries".to_owned(),
            release: layer.release.clone().into(),
            iso3,
            level,
            attribution: layer.attribution.clone(),
            license: layer.license.clone(),
            total,
            units: options,
            withheld_disputed,
            unavailable: layer.unavailable_units.len(),
            notices,
            warning,
        })
    }

    /// Resolve one listed unit to the real polygon an AOI binds.
    pub fn geo_boundary_resolve(
        &self,
        request: &GeoBoundaryResolveRequest,
    ) -> Result<AdminBoundaryResult, ApplicationServiceError> {
        let provider_object_id = request.provider_object_id.trim();
        let (iso3, level, _) =
            parse_geoboundaries_object_id(provider_object_id).map_err(provider_error)?;
        let iso3 = checked_iso3(&iso3)?;
        self.use_saved_boundary_route();
        let catalog = self.boundary_service.geo_catalog();
        let layer = self
            .boundary_service
            .geo_cache()
            .layer(catalog.as_ref(), &iso3, &level)?;
        let disputed = || provider_error(disputed_route_refusal("disputed_guard_region"));
        if layer
            .withheld_object_ids
            .iter()
            .any(|withheld| withheld == provider_object_id)
        {
            return Err(disputed());
        }
        if let Some(unit) = layer
            .unavailable_units
            .iter()
            .find(|unit| unit.provider_object_id == provider_object_id)
        {
            let (iso3, level, shape) =
                parse_geoboundaries_object_id(provider_object_id).map_err(provider_error)?;
            if withheld_by_identity(&iso3, &level, &shape, &unit.name) {
                return Err(provider_error(disputed_route_refusal(
                    "withheld_by_identity",
                )));
            }
            return Err(provider_error(geometry_unavailable_refusal()));
        }
        let unit = layer
            .units
            .iter()
            .find(|unit| unit.provider_object_id == provider_object_id)
            .cloned()
            .ok_or_else(|| identity_mismatch("geoBoundaries 图层中没有所选行政单元"))?;
        // The guard is checked again before geometry leaves the service, so
        // a catalog that did not withhold a unit cannot bind it.
        if unit_withheld_by_identity(&unit) {
            return Err(provider_error(disputed_route_refusal(
                "withheld_by_identity",
            )));
        }
        match unit_guard_check(&unit) {
            GuardCheck::Clear => {}
            GuardCheck::Disputed(_) => return Err(disputed()),
            GuardCheck::Unverifiable => return Err(provider_error(geometry_unavailable_refusal())),
        }
        crate::boundary_service::normalize_feature(
            "geoboundaries",
            unit_feature(unit, &layer.release, &layer.attribution),
            "miss",
        )
    }
}

struct NoGlobalProvider;

impl GlobalBoundaryProvider for NoGlobalProvider {
    fn search(&self, _query: &str, _limit: usize) -> ProviderResult<Vec<ProviderBoundaryFeature>> {
        Err(unavailable())
    }

    fn discover(
        &self,
        _query: &str,
        _limit: usize,
    ) -> ProviderResult<Vec<ProviderBoundaryCandidate>> {
        Err(unavailable())
    }

    fn resolve(&self, _provider_object_id: &str) -> ProviderResult<ProviderBoundaryFeature> {
        Err(unavailable())
    }

    fn discover_china(
        &self,
        _query: &str,
        _limit: usize,
    ) -> ProviderResult<Vec<ProviderBoundaryCandidate>> {
        Err(unavailable())
    }
}

struct NoChinaProvider;

impl ChinaBoundaryProvider for NoChinaProvider {
    fn options(&self, _province: &str, _city: &str) -> ProviderResult<ChinaAdminOptions> {
        Err(unavailable())
    }

    fn search(
        &self,
        _province: &str,
        _city: &str,
        _district: &str,
        _query: &str,
        _limit: usize,
    ) -> ProviderResult<Vec<ProviderBoundaryFeature>> {
        Err(unavailable())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_production_build_keeps_its_error() {
        let built = ContractError::new(
            "BOUNDARY_PROVIDER_CONFIGURATION_INVALID",
            "无法初始化 geoBoundaries 行政边界服务",
            Map::from_iter([(
                "reason".to_owned(),
                Value::String("client_build".to_owned()),
            )]),
        )
        .expect("bounded error context");
        let catalog = UnavailableGeoCatalog::from_error(built.clone());
        assert_eq!(catalog.countries().expect_err("unavailable"), built);
        assert_eq!(catalog.levels("NPL").expect_err("unavailable"), built);
        assert_eq!(
            catalog.layer("NPL", "ADM1").expect_err("unavailable"),
            built
        );
        assert_eq!(
            UnavailableGeoCatalog::not_configured()
                .countries()
                .expect_err("not configured")
                .code,
            "BOUNDARY_PROVIDER_UNAVAILABLE"
        );
    }
}
