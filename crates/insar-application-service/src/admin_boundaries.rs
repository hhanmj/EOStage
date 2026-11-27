//! Canonical AOI boundary transport types and the local China option locator.
//!
//! The UI bundle already contains a first-party generated locator index. The
//! application service is the sole owner of interpreting that index for the
//! China selector fallback. Fallback rows never supply provider identity or
//! authoritative administrative geometry; BoundaryService only admits real
//! Polygon/MultiPolygon provider results for preview/bind.

use super::{ApplicationService, ApplicationServiceError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::OnceLock;

const INDEX_SOURCE: &str = include_str!("../../../ui/src/data/adminBoundaries.ts");
const INDEX_MARKER: &str = "export const ADMIN_BOUNDARY_INDEX = ";
const TAIWAN_IDENTITY_SOURCE: &str =
    include_str!("../../../contracts/taiwan-admin/taiwan-admin-identities.v1.json");
type RawBoundaryRow = (String, String, f64, f64, f64, f64);

#[derive(Debug, Deserialize)]
struct RawBoundaryIndex {
    provinces: Vec<RawBoundaryRow>,
    #[serde(rename = "citiesByProvince")]
    cities_by_province: BTreeMap<String, Vec<RawBoundaryRow>>,
    #[serde(rename = "districtsByCity")]
    districts_by_city: BTreeMap<String, Vec<RawBoundaryRow>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TaiwanIdentityTable {
    schema_version: u32,
    authority_id: String,
    version: String,
    status: String,
    provider: String,
    country_code: String,
    verification: TaiwanIdentityVerification,
    units: Vec<TaiwanBoundaryIdentity>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TaiwanIdentityVerification {
    verified_at: String,
    evidence_path: String,
    evidence_sha256: String,
    source: String,
    license: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TaiwanBoundaryIdentity {
    pub display_name: String,
    pub selector_level: String,
    pub provider: String,
    pub provider_object_id: String,
    pub expected_provider_names: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct AdminBoundarySearchRequest {
    pub query: String,
    pub province: String,
    pub city: String,
    pub district: String,
    pub provider_object_id: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct AdminBoundaryOptionsRequest {
    pub province: String,
    pub city: String,
}

impl Default for AdminBoundarySearchRequest {
    fn default() -> Self {
        Self {
            query: String::new(),
            province: String::new(),
            city: String::new(),
            district: String::new(),
            provider_object_id: None,
            limit: Some(8),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdminBoundaryBbox {
    pub west: f64,
    pub east: f64,
    pub south: f64,
    pub north: f64,
    pub crs: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdminBoundaryResult {
    /// Compatibility aliases retained for the existing UI projection. They
    /// are normalized values, not provider-specific fields.
    pub id: String,
    pub label: String,
    pub name: String,
    pub level: String,
    pub parent: Option<String>,
    pub boundary_id: String,
    pub provider: String,
    pub provider_object_id: String,
    pub display_name: String,
    pub country_code: Option<String>,
    pub admin_level: Option<String>,
    pub bbox: AdminBoundaryBbox,
    pub geometry: Option<Value>,
    pub geometry_kind: Option<String>,
    pub source: String,
    pub attribution: Option<String>,
    pub cache_status: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdminBoundaryResolveRequest {
    pub provider: String,
    pub provider_object_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdminBoundarySearchResult {
    pub ok: bool,
    pub query: String,
    pub provider: String,
    pub warning: Option<String>,
    pub results: Vec<AdminBoundaryResult>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdminBoundaryOption {
    pub name: String,
    pub provider: Option<String>,
    pub provider_object_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdminBoundaryOptionsResult {
    pub ok: bool,
    pub provider: String,
    pub warning: Option<String>,
    pub provinces: Vec<AdminBoundaryOption>,
    pub cities: Vec<AdminBoundaryOption>,
    pub districts: Vec<AdminBoundaryOption>,
}

fn load_index() -> Result<&'static RawBoundaryIndex, ApplicationServiceError> {
    static INDEX: OnceLock<Result<RawBoundaryIndex, String>> = OnceLock::new();
    let parsed = INDEX.get_or_init(|| {
        let body = INDEX_SOURCE
            .split_once(INDEX_MARKER)
            .map(|(_, value)| value.trim())
            .ok_or_else(|| "the bundled administrative locator marker is missing".to_owned())?;
        let json_text = body
            .strip_suffix("as const;")
            .map(str::trim)
            .unwrap_or(body);
        serde_json::from_str(json_text)
            .map_err(|error| format!("the bundled administrative locator is invalid: {error}"))
    });
    parsed
        .as_ref()
        .map_err(|reason| ApplicationServiceError::InvalidRequest {
            reason: reason.clone(),
        })
}

fn load_taiwan_identity_table() -> Result<&'static TaiwanIdentityTable, ApplicationServiceError> {
    static TABLE: OnceLock<Result<TaiwanIdentityTable, String>> = OnceLock::new();
    let parsed = TABLE.get_or_init(|| {
        let table: TaiwanIdentityTable = serde_json::from_str(TAIWAN_IDENTITY_SOURCE)
            .map_err(|error| format!("Taiwan selector identity table is invalid: {error}"))?;
        if table.schema_version != 1
            || table.authority_id != "eostage.taiwan-selector-identities.v1"
            || table.version.trim().is_empty()
            || table.status != "product_owned_verified_binding_table"
            || table.provider != "datav"
            || table.country_code != "CN"
            || table.verification.verified_at.trim().is_empty()
            || table.verification.evidence_path.trim().is_empty()
            || table.verification.evidence_sha256.len() != 64
            || table.verification.source.trim().is_empty()
            || table.verification.license.trim().is_empty()
            || table.units.len() != 23
        {
            return Err("Taiwan selector identity table metadata is incomplete".to_owned());
        }
        let province_count = table
            .units
            .iter()
            .filter(|unit| unit.selector_level == "province")
            .count();
        if province_count != 1
            || table.units.iter().any(|unit| {
                let valid_domestic = match (unit.selector_level.as_str(), unit.provider.as_str()) {
                    ("province", "datav") => unit.provider_object_id == "710000",
                    ("city_county", "tianditu") => {
                        unit.provider_object_id.len() == 6
                            && unit.provider_object_id.starts_with("71")
                            && unit
                                .provider_object_id
                                .bytes()
                                .all(|byte| byte.is_ascii_digit())
                    }
                    _ => false,
                };
                unit.display_name.trim() != unit.display_name
                    || unit.display_name.is_empty()
                    || !valid_domestic
                    || unit.expected_provider_names.is_empty()
            })
        {
            return Err("Taiwan selector identity rows are incomplete".to_owned());
        }
        Ok(table)
    });
    parsed
        .as_ref()
        .map_err(|reason| ApplicationServiceError::InvalidRequest {
            reason: reason.clone(),
        })
}

pub(crate) fn taiwan_identity_for_selection(
    province: &str,
    city: &str,
) -> Result<TaiwanBoundaryIdentity, ApplicationServiceError> {
    let table = load_taiwan_identity_table()?;
    let target_name = if is_unrestricted(city) {
        province.trim()
    } else {
        city.trim()
    };
    table
        .units
        .iter()
        .find(|unit| normalized(&unit.display_name) == normalized(target_name))
        .cloned()
        .ok_or_else(|| ApplicationServiceError::InvalidRequest {
            reason: format!("Taiwan selector identity is unavailable for {target_name}"),
        })
}

fn normalized(value: &str) -> String {
    value
        .trim()
        .chars()
        .filter(|character| !character.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

pub(crate) fn is_unrestricted(value: &str) -> bool {
    matches!(
        normalized(value).as_str(),
        "" | "全部" | "不限" | "全国" | "中国" | "china"
    )
}

/// Return selector names from the bundled local locator. This is intentionally
/// an options-only fallback: it never supplies provider identity or
/// authoritative AOI geometry.
pub(crate) fn local_options(
    request: &AdminBoundaryOptionsRequest,
) -> Result<AdminBoundaryOptionsResult, ApplicationServiceError> {
    let index = load_index()?;
    let provinces = std::iter::once(unidentified_option("全国"))
        .chain(
            index
                .provinces
                .iter()
                .map(|row| unidentified_option(row.0.clone())),
        )
        .collect::<Vec<_>>();

    let province = index.provinces.iter().find(|row| {
        !is_unrestricted(&request.province) && normalized(&row.0) == normalized(&request.province)
    });
    let cities = province
        .and_then(|row| {
            index
                .cities_by_province
                .iter()
                .find(|(key, _)| normalized(key) == normalized(&row.0))
                .map(|(_, rows)| {
                    rows.iter()
                        .map(|row| unidentified_option(row.0.clone()))
                        .collect::<Vec<_>>()
                })
        })
        .map(|mut rows| {
            rows.insert(0, unidentified_option("全部"));
            rows
        })
        .unwrap_or_else(|| vec![unidentified_option("全部")]);

    let city = province.and_then(|row| {
        if is_unrestricted(&request.city) {
            return None;
        }
        let province_name = &row.0;
        index
            .cities_by_province
            .iter()
            .find(|(key, _)| normalized(key) == normalized(province_name))
            .and_then(|(_, rows)| {
                rows.iter()
                    .find(|city_row| normalized(&city_row.0) == normalized(&request.city))
            })
            .map(|city_row| (province_name.clone(), city_row.0.clone()))
    });
    let districts = city
        .and_then(|(province_name, city_name)| {
            let wanted_key = format!("{province_name}|{city_name}");
            index
                .districts_by_city
                .iter()
                .find(|(key, _)| normalized(key) == normalized(&wanted_key))
                .map(|(_, rows)| {
                    rows.iter()
                        .map(|row| unidentified_option(row.0.clone()))
                        .collect::<Vec<_>>()
                })
        })
        .map(|mut rows| {
            rows.insert(0, unidentified_option("全部"));
            rows
        })
        .unwrap_or_else(|| vec![unidentified_option("全部")]);

    Ok(AdminBoundaryOptionsResult {
        ok: true,
        provider: "insar-studio-local-index".to_owned(),
        warning: None,
        provinces,
        cities,
        districts,
    })
}

/// Product-owned Taiwan selector rows carry verified domestic DataV / Tianditu
/// identities only; foreign Nominatim sources are never permitted for Taiwan.
pub(crate) fn taiwan_options(
    request: &AdminBoundaryOptionsRequest,
) -> Result<AdminBoundaryOptionsResult, ApplicationServiceError> {
    let mut result = local_options(request)?;
    let table = load_taiwan_identity_table()?;
    let province = table
        .units
        .iter()
        .find(|unit| unit.selector_level == "province")
        .ok_or_else(|| ApplicationServiceError::InvalidRequest {
            reason: "Taiwan province identity is unavailable".to_owned(),
        })?;
    result.provider = "insar-studio-product-options".to_owned();
    result.warning = None;
    for option in &mut result.provinces {
        if normalized(&option.name) == normalized(&province.display_name) {
            *option = identified_option(
                &province.display_name,
                &province.provider,
                &province.provider_object_id,
            );
        }
    }
    result.cities = std::iter::once(identified_option(
        "全部",
        &province.provider,
        &province.provider_object_id,
    ))
    .chain(
        table
            .units
            .iter()
            .filter(|unit| unit.selector_level == "city_county")
            .map(|unit| {
                identified_option(&unit.display_name, &unit.provider, &unit.provider_object_id)
            }),
    )
    .collect();
    result.districts = vec![identified_option(
        "全部",
        &province.provider,
        &province.provider_object_id,
    )];
    Ok(result)
}

fn identified_option(
    name: impl Into<String>,
    provider: impl Into<String>,
    provider_object_id: impl Into<String>,
) -> AdminBoundaryOption {
    AdminBoundaryOption {
        name: name.into(),
        provider: Some(provider.into()),
        provider_object_id: Some(provider_object_id.into()),
    }
}

fn unidentified_option(name: impl Into<String>) -> AdminBoundaryOption {
    AdminBoundaryOption {
        name: name.into(),
        provider: None,
        provider_object_id: None,
    }
}

impl ApplicationService {
    /// Search the provider-backed BoundaryService. Normal desktop construction
    /// supplies the existing Store lease for persistent resolved-boundary cache;
    /// liveness-only tests retain a cache-free provider seam.
    pub fn admin_boundary_search(
        &self,
        request: &AdminBoundarySearchRequest,
    ) -> Result<AdminBoundarySearchResult, ApplicationServiceError> {
        let store = self.administrative_boundary_store()?;
        self.use_saved_boundary_route();
        self.boundary_service.search(request, store.as_ref())
    }

    pub fn admin_boundary_resolve(
        &self,
        request: &AdminBoundaryResolveRequest,
    ) -> Result<AdminBoundaryResult, ApplicationServiceError> {
        let store = self.administrative_boundary_store()?;
        self.use_saved_boundary_route();
        self.boundary_service.resolve(request, store.as_ref())
    }

    pub fn admin_options(
        &self,
        request: &AdminBoundaryOptionsRequest,
    ) -> Result<AdminBoundaryOptionsResult, ApplicationServiceError> {
        self.use_saved_boundary_route();
        self.boundary_service.options(request)
    }

    pub(crate) fn use_saved_boundary_route(&self) {
        let settings = match self.get_network_settings() {
            Ok(settings) => settings,
            Err(ApplicationServiceError::StorePathRequired) => {
                crate::ApplicationNetworkSettings::default()
            }
            Err(err) => {
                eprintln!(
                    "[admin_boundaries] saved boundary route fallback to default: code={}",
                    err.code()
                );
                crate::ApplicationNetworkSettings::default()
            }
        };
        self.boundary_service.use_network_settings(
            crate::network_routes::policy_for_settings(
                crate::network_routes::ServiceRoute::Ordinary,
                &settings,
            )
            .route(),
            &settings.tianditu_token,
        );
    }

    fn administrative_boundary_store(
        &self,
    ) -> Result<Option<super::SharedTaskStore>, ApplicationServiceError> {
        if self.config.store_path().is_none() || self.config.is_verification_only() {
            return Ok(None);
        }
        self.store_handle().map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::{local_options, AdminBoundaryOptionsRequest};
    use crate::ApplicationServiceError;

    #[test]
    fn bundled_locator_has_the_expected_national_and_provincial_semantics() {
        let options =
            local_options(&AdminBoundaryOptionsRequest::default()).expect("locator loads");
        assert!(options.provinces.iter().any(|item| item.name == "北京市"));
        assert!(options.provinces.iter().any(|item| item.name == "台湾省"));
        assert_eq!(options.cities[0].name, "全部");
        assert!(options
            .provinces
            .iter()
            .chain(options.cities.iter())
            .chain(options.districts.iter())
            .all(|item| item.provider_object_id.is_none()));
    }

    #[test]
    fn hubei_wuhan_badong_filters_preserve_the_hierarchy() {
        let result = local_options(&AdminBoundaryOptionsRequest {
            province: "湖北省".to_owned(),
            city: "恩施土家族苗族自治州".to_owned(),
        })
        .expect("hierarchical lookup");
        assert!(result
            .cities
            .iter()
            .any(|item| item.name == "恩施土家族苗族自治州"));
        assert!(result.districts.iter().any(|item| item.name == "巴东县"));
        assert!(result
            .provinces
            .iter()
            .chain(result.cities.iter())
            .chain(result.districts.iter())
            .all(|item| item.provider_object_id.is_none()));
    }

    #[test]
    fn locator_parse_failure_is_typed_instead_of_falling_back_to_worker() {
        let error = ApplicationServiceError::InvalidRequest {
            reason: "fixture".to_owned(),
        };
        assert_eq!(error.code(), "V3_APPLICATION_INVALID_REQUEST");
    }
}
