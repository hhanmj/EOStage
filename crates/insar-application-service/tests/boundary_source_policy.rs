//! Batch 4 boundary source policy at the application seam: attribution,
//! failure caching, Taiwan first-load guidance and geoBoundaries browsing.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use insar_application_service::{
    AdminBoundarySearchRequest, ApplicationService, ApplicationServiceConfig,
    ApplicationServiceError, GeoBoundaryLevelsRequest, GeoBoundaryResolveRequest,
    GeoBoundaryUnitsRequest,
};
use insar_core::{ContractError, ProviderResult};
use insar_provider::boundary::{
    ChinaAdminOptions, ChinaBoundaryProvider, GlobalBoundaryProvider, ProviderBoundaryCandidate,
    ProviderBoundaryFeature,
};
use insar_provider::geoboundaries::{
    GeoBoundariesRelease, GeoBoundaryCatalog, GeoBoundaryCountry, GeoBoundaryLayer,
    GeoBoundaryLevel, GeoBoundaryUnavailableUnit, GeoBoundaryUnit,
};
use serde_json::{json, Map, Value};

fn provider_error(code: &str) -> ContractError {
    ContractError::new(code, "fixture failure", Map::new()).expect("bounded fixture error")
}

fn provider_code(error: ApplicationServiceError) -> ContractError {
    match error {
        ApplicationServiceError::Provider(error) => error,
        other => panic!("expected a typed provider error, got {other:?}"),
    }
}

fn square(west: f64, south: f64, size: f64) -> Value {
    json!({
        "type": "Polygon",
        "coordinates": [[
            [west, south], [west + size, south], [west + size, south + size],
            [west, south + size], [west, south]
        ]]
    })
}

struct DatavWithoutAttribution;

impl ChinaBoundaryProvider for DatavWithoutAttribution {
    fn options(&self, _province: &str, _city: &str) -> ProviderResult<ChinaAdminOptions> {
        Ok(ChinaAdminOptions::default())
    }

    fn resolve_exact_adcode(&self, adcode: &str) -> ProviderResult<ProviderBoundaryFeature> {
        Ok(ProviderBoundaryFeature {
            provider_object_id: adcode.to_owned(),
            name: "巴东县".to_owned(),
            display_name: "湖北省 / 恩施土家族苗族自治州 / 巴东县".to_owned(),
            country_code: Some("CN".to_owned()),
            admin_level: Some("district".to_owned()),
            geometry: square(109.0, 30.0, 1.0),
            geometry_kind: "polygon".to_owned(),
            bbox: [109.0, 30.0, 110.0, 31.0],
            source: "DataV areas_v3".to_owned(),
            attribution: None,
        })
    }

    fn search(
        &self,
        _province: &str,
        _city: &str,
        _district: &str,
        _query: &str,
        _limit: usize,
    ) -> ProviderResult<Vec<ProviderBoundaryFeature>> {
        Ok(Vec::new())
    }
}

/// Global provider whose first `failures` calls fail with `code`.
struct FailingGlobal {
    code: &'static str,
    failures: usize,
    calls: Arc<AtomicUsize>,
}

impl FailingGlobal {
    fn attempt(&self) -> ProviderResult<()> {
        if self.calls.fetch_add(1, Ordering::SeqCst) < self.failures {
            Err(provider_error(self.code))
        } else {
            Ok(())
        }
    }
}

impl GlobalBoundaryProvider for FailingGlobal {
    fn search(&self, _query: &str, _limit: usize) -> ProviderResult<Vec<ProviderBoundaryFeature>> {
        self.attempt().map(|_| Vec::new())
    }

    fn discover(
        &self,
        query: &str,
        _limit: usize,
    ) -> ProviderResult<Vec<ProviderBoundaryCandidate>> {
        self.attempt()?;
        Ok(vec![ProviderBoundaryCandidate {
            provider_object_id: "relation:1".to_owned(),
            name: query.to_owned(),
            display_name: query.to_owned(),
            country_code: Some("NP".to_owned()),
            admin_level: Some("administrative".to_owned()),
            bbox: [84.0, 27.0, 86.0, 28.0],
            source: "OpenStreetMap Nominatim".to_owned(),
            attribution: Some("© OpenStreetMap contributors".to_owned()),
        }])
    }

    fn resolve(&self, provider_object_id: &str) -> ProviderResult<ProviderBoundaryFeature> {
        self.attempt()?;
        Ok(ProviderBoundaryFeature {
            provider_object_id: provider_object_id.to_owned(),
            name: "臺北市".to_owned(),
            display_name: "臺北市".to_owned(),
            country_code: Some("TW".to_owned()),
            admin_level: Some("administrative".to_owned()),
            geometry: square(121.4, 25.0, 0.2),
            geometry_kind: "polygon".to_owned(),
            bbox: [121.4, 25.0, 121.6, 25.2],
            source: "OpenStreetMap Nominatim".to_owned(),
            attribution: Some("© OpenStreetMap contributors".to_owned()),
        })
    }

    fn discover_china(
        &self,
        _query: &str,
        _limit: usize,
    ) -> ProviderResult<Vec<ProviderBoundaryCandidate>> {
        self.attempt().map(|_| Vec::new())
    }
}

fn service_with(global: FailingGlobal) -> ApplicationService {
    ApplicationService::with_boundary_providers(
        ApplicationServiceConfig::liveness_only(),
        Arc::new(global),
        Arc::new(DatavWithoutAttribution),
    )
}

struct ForbiddenGlobalForTaiwan;

impl GlobalBoundaryProvider for ForbiddenGlobalForTaiwan {
    fn search(&self, _query: &str, _limit: usize) -> ProviderResult<Vec<ProviderBoundaryFeature>> {
        panic!("Taiwan failure must not call Nominatim search")
    }
    fn discover(
        &self,
        _query: &str,
        _limit: usize,
    ) -> ProviderResult<Vec<ProviderBoundaryCandidate>> {
        panic!("Taiwan failure must not call Nominatim discover")
    }
    fn discover_china(
        &self,
        _query: &str,
        _limit: usize,
    ) -> ProviderResult<Vec<ProviderBoundaryCandidate>> {
        panic!("Taiwan failure must not call Nominatim discover_china")
    }
    fn resolve(&self, _provider_object_id: &str) -> ProviderResult<ProviderBoundaryFeature> {
        panic!("Taiwan failure must not call Nominatim resolve")
    }
    fn search_taiwan(
        &self,
        _query: &str,
        _limit: usize,
    ) -> ProviderResult<Vec<ProviderBoundaryFeature>> {
        panic!("Taiwan failure must not call Nominatim search_taiwan")
    }
}

struct FailingChina {
    code: &'static str,
}

impl ChinaBoundaryProvider for FailingChina {
    fn options(&self, _province: &str, _city: &str) -> ProviderResult<ChinaAdminOptions> {
        Ok(ChinaAdminOptions::default())
    }
    fn resolve_exact_adcode(&self, _adcode: &str) -> ProviderResult<ProviderBoundaryFeature> {
        Err(
            ContractError::new(self.code, "DataV fixture failure", serde_json::Map::new())
                .expect("bounded fixture error"),
        )
    }
    fn search(
        &self,
        _province: &str,
        _city: &str,
        _district: &str,
        _query: &str,
        _limit: usize,
    ) -> ProviderResult<Vec<ProviderBoundaryFeature>> {
        Err(
            ContractError::new(self.code, "DataV fixture failure", serde_json::Map::new())
                .expect("bounded fixture error"),
        )
    }
}

fn taiwan_service_with(china: FailingChina) -> ApplicationService {
    ApplicationService::with_boundary_providers(
        ApplicationServiceConfig::liveness_only(),
        Arc::new(ForbiddenGlobalForTaiwan),
        Arc::new(china),
    )
}

#[test]
fn datav_results_without_provider_attribution_gain_the_datav_notice() {
    let service = service_with(FailingGlobal {
        code: "BOUNDARY_PROVIDER_UNAVAILABLE",
        failures: 0,
        calls: Arc::new(AtomicUsize::new(0)),
    });
    let result = service
        .admin_boundary_search(&AdminBoundarySearchRequest {
            province: "湖北省".to_owned(),
            city: "恩施土家族苗族自治州".to_owned(),
            district: "巴东县".to_owned(),
            provider_object_id: Some("422823".to_owned()),
            ..Default::default()
        })
        .expect("DataV selector");
    assert_eq!(result.provider, "datav");
    let attribution = result.results[0]
        .attribution
        .as_deref()
        .expect("DataV attribution reaches the AOI transport");
    assert!(attribution.contains("DataV"), "{attribution}");
    assert!(attribution.contains("许可"), "{attribution}");
}

#[test]
fn a_failed_global_lookup_is_not_cached() {
    let calls = Arc::new(AtomicUsize::new(0));
    let service = service_with(FailingGlobal {
        code: "BOUNDARY_PROVIDER_NON_DATA_RESPONSE",
        failures: 1,
        calls: Arc::clone(&calls),
    });
    let request = AdminBoundarySearchRequest {
        query: "Kathmandu".to_owned(),
        ..Default::default()
    };
    let error = provider_code(
        service
            .admin_boundary_search(&request)
            .expect_err("first lookup fails"),
    );
    assert_eq!(error.code, "BOUNDARY_PROVIDER_NON_DATA_RESPONSE");
    let result = service
        .admin_boundary_search(&request)
        .expect("retry reaches the provider again");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(result.results.len(), 1);
}

#[test]
fn taiwan_first_load_network_failure_is_typed_with_a_proxy_hint() {
    for upstream in [
        "BOUNDARY_PROVIDER_UNAVAILABLE",
        "BOUNDARY_PROVIDER_HTTP_ERROR",
        "BOUNDARY_PROVIDER_NON_DATA_RESPONSE",
    ] {
        let service = taiwan_service_with(FailingChina { code: upstream });
        let error = provider_code(
            service
                .admin_boundary_search(&AdminBoundarySearchRequest {
                    province: "台湾省".to_owned(),
                    city: "全部".to_owned(),
                    provider_object_id: Some("710000".to_owned()),
                    ..Default::default()
                })
                .expect_err("first Taiwan load fails"),
        );
        assert_eq!(
            error.code, "BOUNDARY_TAIWAN_FIRST_LOAD_FAILED",
            "{upstream}"
        );
        assert!(
            error.user_message.contains("请检查网络或在设置里切换代理"),
            "{}",
            error.user_message
        );
        assert_eq!(error.debug_context["upstream_code"], upstream);
        assert_eq!(
            error.debug_context["suggestion"],
            "check_network_or_switch_proxy"
        );
    }
}

#[derive(Default)]
struct FakeCatalog {
    level_calls: AtomicUsize,
    layer_calls: AtomicUsize,
    failing_layer_calls: usize,
    invalid_units: usize,
    /// Adds a unit inside a guard ring that this catalog does not withhold.
    unfiltered_disputed: bool,
    /// India with the real gbOpen Maharashtra polygon (6 685 vertices) as
    /// ADM1 and districts plus unverified units as ADM2.
    real_maharashtra: bool,
    /// Lists Okinawa without withholding it.
    unfiltered_identity: bool,
}

/// Real gbOpen 9469f09 units exported in L4 round 4 (provider test fixture).
const R4_REAL_UNITS: &str =
    include_str!("../../insar-provider/tests/fixtures/geoboundaries/r4-real-units.geojson");

fn real_geometry(name: &str) -> Value {
    let collection: Value = serde_json::from_str(R4_REAL_UNITS).expect("r4 real units");
    collection["features"]
        .as_array()
        .expect("features")
        .iter()
        .find(|feature| feature["properties"]["shapeName"] == name)
        .map(|feature| feature["geometry"].clone())
        .unwrap_or_else(|| panic!("{name} is in the r4 export"))
}

fn fake_unit(iso: &str, level: &str, shape: &str, name: &str, geometry: Value) -> GeoBoundaryUnit {
    let normalized =
        insar_provider::boundary::normalize_polygonal_geometry(geometry).expect("fixture polygon");
    GeoBoundaryUnit {
        provider_object_id: format!("gb:{iso}:{level}:{shape}"),
        name: name.to_owned(),
        iso3: iso.to_owned(),
        level: level.to_owned(),
        geometry: normalized.geometry,
        geometry_kind: normalized.geometry_kind,
        bbox: normalized.bbox,
    }
}

impl GeoBoundaryCatalog for FakeCatalog {
    fn release(&self) -> GeoBoundariesRelease {
        GeoBoundariesRelease::pinned()
    }

    fn countries(&self) -> ProviderResult<Vec<GeoBoundaryCountry>> {
        Ok(vec![GeoBoundaryCountry {
            iso3: "NPL".to_owned(),
            name: "Nepal".to_owned(),
        }])
    }

    fn levels(&self, iso3: &str) -> ProviderResult<Vec<GeoBoundaryLevel>> {
        self.level_calls.fetch_add(1, Ordering::SeqCst);
        Ok(vec![GeoBoundaryLevel {
            iso3: iso3.to_owned(),
            level: "ADM1".to_owned(),
            canonical_name: Some("Provinces".to_owned()),
            unit_count: Some(2),
            source: Some("Survey Department of Nepal".to_owned()),
            license: "CC BY 4.0".to_owned(),
            license_source: None,
            year_represented: Some("2020".to_owned()),
            attribution: "geoBoundaries gbOpen 6.0.0 (9469f09)；许可：CC BY 4.0".to_owned(),
        }])
    }

    fn layer(&self, iso3: &str, level: &str) -> ProviderResult<GeoBoundaryLayer> {
        if self.layer_calls.fetch_add(1, Ordering::SeqCst) < self.failing_layer_calls {
            return Err(provider_error("BOUNDARY_PROVIDER_UNAVAILABLE"));
        }
        let units = match level {
            "ADM1" if self.unfiltered_identity => vec![
                fake_unit(
                    iso3,
                    "ADM1",
                    "47310658B31569081891025",
                    "Okinawa Prefecture",
                    square(127.5, 26.2, 0.5),
                ),
                fake_unit(iso3, "ADM1", "HKD", "Hokkaido", square(142.0, 43.0, 1.0)),
            ],
            "ADM1" if self.real_maharashtra => vec![fake_unit(
                iso3,
                "ADM1",
                "MH",
                "Mahārāshtra",
                real_geometry("Mahārāshtra"),
            )],
            "ADM2" if self.real_maharashtra => vec![
                fake_unit(iso3, "ADM2", "PUNE", "Pune", square(73.8, 18.5, 0.1)),
                fake_unit(iso3, "ADM2", "KOL", "Kolkata", square(88.3, 22.5, 0.1)),
            ],
            "ADM1" if self.unfiltered_disputed => vec![
                fake_unit(iso3, "ADM1", "P3", "Bagmati", square(85.0, 27.0, 1.0)),
                fake_unit(iso3, "ADM1", "G1", "Guarded", square(93.0, 27.6, 0.5)),
            ],
            "ADM1" => vec![
                fake_unit(iso3, "ADM1", "P3", "Bagmati", square(85.0, 27.0, 1.0)),
                fake_unit(iso3, "ADM1", "P1", "Koshi", square(86.0, 26.5, 1.0)),
            ],
            _ => vec![
                fake_unit(iso3, "ADM2", "D1", "Kathmandu", square(85.2, 27.6, 0.2)),
                fake_unit(iso3, "ADM2", "D2", "Lalitpur", square(85.4, 27.4, 0.2)),
                fake_unit(iso3, "ADM2", "D3", "Ilam", square(87.8, 26.8, 0.2)),
            ],
        };
        Ok(GeoBoundaryLayer {
            iso3: iso3.to_owned(),
            level: level.to_owned(),
            release: GeoBoundariesRelease::pinned(),
            release_drift: None,
            attribution: "geoBoundaries gbOpen 6.0.0 (9469f09)；许可：CC BY 4.0".to_owned(),
            license: "CC BY 4.0".to_owned(),
            units,
            withheld_disputed: 1,
            withheld_object_ids: vec![format!("gb:{iso3}:{level}:W1")],
            unavailable_units: if self.real_maharashtra && level == "ADM2" {
                vec![
                    GeoBoundaryUnavailableUnit {
                        provider_object_id: format!("gb:{iso3}:ADM2:U1"),
                        name: "Unverified".to_owned(),
                        bbox: Some([74.0, 19.0, 74.2, 19.2]),
                    },
                    GeoBoundaryUnavailableUnit {
                        provider_object_id: format!("gb:{iso3}:ADM2:U2"),
                        name: "Far unverified".to_owned(),
                        bbox: Some([88.0, 22.0, 88.2, 22.2]),
                    },
                ]
            } else {
                Vec::new()
            },
            invalid_units: self.invalid_units,
        })
    }
}

fn geo_service(catalog: Arc<FakeCatalog>) -> ApplicationService {
    ApplicationService::with_geo_boundary_catalog(
        ApplicationServiceConfig::liveness_only(),
        catalog,
    )
}

#[test]
fn geo_boundary_browsing_refuses_china_scope_at_the_application_seam() {
    let catalog = Arc::new(FakeCatalog::default());
    let service = geo_service(Arc::clone(&catalog));
    for iso3 in ["CHN", "TWN", "hkg", "MAC"] {
        let error = provider_code(
            service
                .geo_boundary_levels(&GeoBoundaryLevelsRequest {
                    iso3: iso3.to_owned(),
                })
                .expect_err("China scope stays on the China selector"),
        );
        assert_eq!(error.code, "BOUNDARY_ROUTE_REFUSED_CHINA_SCOPE", "{iso3}");
        assert_eq!(error.debug_context["fallback"], "china_selector");
        let error = provider_code(
            service
                .geo_boundary_units(&GeoBoundaryUnitsRequest {
                    iso3: iso3.to_owned(),
                    level: "ADM1".to_owned(),
                    ..Default::default()
                })
                .expect_err("China scope stays on the China selector"),
        );
        assert_eq!(error.code, "BOUNDARY_ROUTE_REFUSED_CHINA_SCOPE", "{iso3}");
    }
    let error = provider_code(
        service
            .geo_boundary_resolve(&GeoBoundaryResolveRequest {
                provider_object_id: "gb:TWN:ADM1:x".to_owned(),
            })
            .expect_err("China scope identity is refused"),
    );
    assert_eq!(error.code, "BOUNDARY_ROUTE_REFUSED_CHINA_SCOPE");
    assert_eq!(catalog.level_calls.load(Ordering::SeqCst), 0);
    assert_eq!(catalog.layer_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn geo_boundary_browsing_lists_levels_and_child_units_under_a_parent() {
    let catalog = Arc::new(FakeCatalog::default());
    let service = geo_service(Arc::clone(&catalog));
    let countries = service.geo_boundary_countries().expect("countries");
    assert_eq!(countries.provider, "geoboundaries");
    assert_eq!(countries.release.version, "6.0.0");
    assert_eq!(countries.release.data_ref, "9469f09");
    assert_eq!(countries.countries[0].iso3, "NPL");

    let levels = service
        .geo_boundary_levels(&GeoBoundaryLevelsRequest {
            iso3: "npl".to_owned(),
        })
        .expect("levels");
    assert_eq!(levels.iso3, "NPL");
    assert_eq!(levels.levels[0].level, "ADM1");
    assert_eq!(levels.levels[0].unit_count, Some(2));
    assert_eq!(levels.levels[0].license, "CC BY 4.0");

    let provinces = service
        .geo_boundary_units(&GeoBoundaryUnitsRequest {
            iso3: "NPL".to_owned(),
            level: "ADM1".to_owned(),
            ..Default::default()
        })
        .expect("ADM1 units");
    assert_eq!(
        provinces
            .units
            .iter()
            .map(|unit| unit.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Bagmati", "Koshi"]
    );
    assert_eq!(provinces.withheld_disputed, 1);
    let warning = provinces
        .warning
        .as_deref()
        .expect("withheld units are reported");
    assert!(warning.contains("中国行政区选择器"), "{warning}");
    assert!(!warning.contains("地名搜索"), "{warning}");
    assert!(provinces.attribution.contains("CC BY 4.0"));

    let districts = service
        .geo_boundary_units(&GeoBoundaryUnitsRequest {
            iso3: "NPL".to_owned(),
            level: "ADM2".to_owned(),
            parent_provider_object_id: Some("gb:NPL:ADM1:P3".to_owned()),
            ..Default::default()
        })
        .expect("ADM2 units inside Bagmati");
    assert_eq!(
        districts
            .units
            .iter()
            .map(|unit| unit.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Kathmandu", "Lalitpur"]
    );
    // Under a parent filter or keyword query the withheld units are outside
    // the visible scope, so the layer-wide China-selector hint is not added.
    let districts_warning = districts.warning.clone().unwrap_or_default();
    assert!(
        !districts_warning.contains("中国行政区选择器"),
        "{districts_warning}"
    );
    assert_eq!(districts.withheld_disputed, 1);

    let searched = service
        .geo_boundary_units(&GeoBoundaryUnitsRequest {
            iso3: "NPL".to_owned(),
            level: "ADM2".to_owned(),
            query: "lalit".to_owned(),
            ..Default::default()
        })
        .expect("ADM2 search");
    assert_eq!(searched.units.len(), 1);
    assert_eq!(searched.units[0].provider_object_id, "gb:NPL:ADM2:D2");
    let searched_warning = searched.warning.clone().unwrap_or_default();
    assert!(
        !searched_warning.contains("中国行政区选择器"),
        "{searched_warning}"
    );
    assert_eq!(searched.withheld_disputed, 1);
    // ADM1 once, ADM2 once: both layers are cached across the four calls.
    assert_eq!(catalog.layer_calls.load(Ordering::SeqCst), 2);
}

#[test]
fn geo_boundary_failed_layer_is_not_cached_and_resolve_binds_exact_identity() {
    let catalog = Arc::new(FakeCatalog {
        failing_layer_calls: 1,
        ..Default::default()
    });
    let service = geo_service(Arc::clone(&catalog));
    let request = GeoBoundaryResolveRequest {
        provider_object_id: "gb:NPL:ADM1:P1".to_owned(),
    };
    let error = provider_code(
        service
            .geo_boundary_resolve(&request)
            .expect_err("first layer fetch fails"),
    );
    assert_eq!(error.code, "BOUNDARY_PROVIDER_UNAVAILABLE");
    let result = service
        .geo_boundary_resolve(&request)
        .expect("retry fetches the layer again");
    assert_eq!(catalog.layer_calls.load(Ordering::SeqCst), 2);
    assert_eq!(result.provider, "geoboundaries");
    assert_eq!(result.provider_object_id, "gb:NPL:ADM1:P1");
    assert_eq!(result.boundary_id, "geoboundaries:gb:NPL:ADM1:P1");
    assert_eq!(result.name, "Koshi");
    assert_eq!(result.geometry_kind.as_deref(), Some("polygon"));
    assert!(result.geometry.is_some());
    assert!(result
        .attribution
        .as_deref()
        .is_some_and(|text| text.contains("geoBoundaries") && text.contains("CC BY 4.0")));

    let error = provider_code(
        service
            .geo_boundary_resolve(&GeoBoundaryResolveRequest {
                provider_object_id: "gb:NPL:ADM1:missing".to_owned(),
            })
            .expect_err("unknown shape"),
    );
    assert_eq!(error.code, "BOUNDARY_PROVIDER_IDENTITY_MISMATCH");
    assert_eq!(catalog.layer_calls.load(Ordering::SeqCst), 2);
}

/// Answers every request with an empty list or a layer of invalid features.
#[derive(Default)]
struct EmptyCatalog {
    calls: AtomicUsize,
}

impl GeoBoundaryCatalog for EmptyCatalog {
    fn release(&self) -> GeoBoundariesRelease {
        GeoBoundariesRelease::pinned()
    }

    fn countries(&self) -> ProviderResult<Vec<GeoBoundaryCountry>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(Vec::new())
    }

    fn levels(&self, _iso3: &str) -> ProviderResult<Vec<GeoBoundaryLevel>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(Vec::new())
    }

    fn layer(&self, iso3: &str, level: &str) -> ProviderResult<GeoBoundaryLayer> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(GeoBoundaryLayer {
            iso3: iso3.to_owned(),
            level: level.to_owned(),
            release: GeoBoundariesRelease::pinned(),
            release_drift: None,
            attribution: "geoBoundaries gbOpen 6.0.0 (9469f09)".to_owned(),
            license: "CC BY 4.0".to_owned(),
            units: Vec::new(),
            withheld_disputed: 0,
            withheld_object_ids: Vec::new(),
            unavailable_units: Vec::new(),
            invalid_units: 3,
        })
    }
}

#[test]
fn geo_boundary_empty_lists_and_layers_are_invalid_and_never_cached() {
    let catalog = Arc::new(EmptyCatalog::default());
    let service = ApplicationService::with_geo_boundary_catalog(
        ApplicationServiceConfig::liveness_only(),
        Arc::clone(&catalog) as Arc<dyn GeoBoundaryCatalog>,
    );
    for _ in 0..2 {
        let error = provider_code(
            service
                .geo_boundary_countries()
                .expect_err("an empty country list is not data"),
        );
        assert_eq!(error.code, "BOUNDARY_PROVIDER_RESPONSE_INVALID");
        let error = provider_code(
            service
                .geo_boundary_levels(&GeoBoundaryLevelsRequest {
                    iso3: "NPL".to_owned(),
                })
                .expect_err("an empty level list is not data"),
        );
        assert_eq!(error.code, "BOUNDARY_PROVIDER_RESPONSE_INVALID");
        let error = provider_code(
            service
                .geo_boundary_units(&GeoBoundaryUnitsRequest {
                    iso3: "NPL".to_owned(),
                    level: "ADM1".to_owned(),
                    ..Default::default()
                })
                .expect_err("a layer of invalid features is not data"),
        );
        assert_eq!(error.code, "BOUNDARY_PROVIDER_RESPONSE_INVALID");
    }
    // Nothing was cached: every call reached the catalog again.
    assert_eq!(catalog.calls.load(Ordering::SeqCst), 6);
}

#[test]
fn geo_boundary_skipped_invalid_features_are_reported() {
    let catalog = Arc::new(FakeCatalog {
        invalid_units: 2,
        ..Default::default()
    });
    let units = geo_service(catalog)
        .geo_boundary_units(&GeoBoundaryUnitsRequest {
            iso3: "NPL".to_owned(),
            level: "ADM1".to_owned(),
            ..Default::default()
        })
        .expect("ADM1 units");
    let warning = units.warning.expect("skipped features are reported");
    assert!(warning.contains("2 个要素无效"), "{warning}");
}

#[test]
fn geo_boundary_resolve_refuses_withheld_and_guarded_units() {
    let catalog = Arc::new(FakeCatalog {
        unfiltered_disputed: true,
        ..Default::default()
    });
    let service = geo_service(catalog);
    for id in ["gb:NPL:ADM1:W1", "gb:NPL:ADM1:G1"] {
        let error = provider_code(
            service
                .geo_boundary_resolve(&GeoBoundaryResolveRequest {
                    provider_object_id: id.to_owned(),
                })
                .expect_err("a disputed unit never binds"),
        );
        assert_eq!(error.code, "BOUNDARY_ROUTE_REFUSED_DISPUTED", "{id}");
        assert_eq!(error.debug_context["fallback"], "china_selector", "{id}");
    }
}

#[test]
fn taiwan_first_load_rate_limit_asks_to_retry_later() {
    let service = taiwan_service_with(FailingChina {
        code: "BOUNDARY_PROVIDER_RATE_LIMITED",
    });
    let error = provider_code(
        service
            .admin_boundary_search(&AdminBoundarySearchRequest {
                province: "台湾省".to_owned(),
                city: "全部".to_owned(),
                provider_object_id: Some("710000".to_owned()),
                ..Default::default()
            })
            .expect_err("first Taiwan load is rate limited"),
    );
    assert_eq!(error.code, "BOUNDARY_TAIWAN_FIRST_LOAD_FAILED");
    assert!(
        error.user_message.contains("请稍后重试"),
        "{}",
        error.user_message
    );
    assert!(
        !error.user_message.contains("切换代理"),
        "{}",
        error.user_message
    );
    assert_eq!(error.debug_context["suggestion"], "retry_later");
}

/// The first country request takes 1.5 s; later ones answer at once.
#[derive(Default)]
struct SlowFirstCountries {
    calls: AtomicUsize,
}

impl GeoBoundaryCatalog for SlowFirstCountries {
    fn release(&self) -> GeoBoundariesRelease {
        GeoBoundariesRelease::pinned()
    }

    fn countries(&self) -> ProviderResult<Vec<GeoBoundaryCountry>> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            std::thread::sleep(std::time::Duration::from_millis(1500));
        }
        Ok(vec![GeoBoundaryCountry {
            iso3: "NPL".to_owned(),
            name: "Nepal".to_owned(),
        }])
    }

    fn levels(&self, _iso3: &str) -> ProviderResult<Vec<GeoBoundaryLevel>> {
        Err(provider_error("BOUNDARY_PROVIDER_UNAVAILABLE"))
    }

    fn layer(&self, _iso3: &str, _level: &str) -> ProviderResult<GeoBoundaryLayer> {
        Err(provider_error("BOUNDARY_PROVIDER_UNAVAILABLE"))
    }
}

#[test]
fn a_slow_country_request_does_not_block_other_callers() {
    let service = Arc::new(ApplicationService::with_geo_boundary_catalog(
        ApplicationServiceConfig::liveness_only(),
        Arc::new(SlowFirstCountries::default()),
    ));
    let slow = {
        let service = Arc::clone(&service);
        std::thread::spawn(move || service.geo_boundary_countries().map(|_| ()))
    };
    std::thread::sleep(std::time::Duration::from_millis(200));
    let started = std::time::Instant::now();
    let fast = service
        .geo_boundary_countries()
        .expect("a second caller is served");
    let waited = started.elapsed();
    assert!(
        waited < std::time::Duration::from_millis(800),
        "the country cache lock was held across the request: {waited:?}"
    );
    assert_eq!(fast.countries[0].iso3, "NPL");
    slow.join()
        .expect("first caller thread")
        .expect("first caller");
}

#[test]
fn a_large_real_unit_binds_and_lists_its_children() {
    let catalog = Arc::new(FakeCatalog {
        real_maharashtra: true,
        ..Default::default()
    });
    let service = geo_service(catalog);
    // L4 r4: Maharashtra was refused as disputed because its geometry was
    // read through the ASF footprint vertex bounds.
    let result = service
        .geo_boundary_resolve(&GeoBoundaryResolveRequest {
            provider_object_id: "gb:IND:ADM1:MH".to_owned(),
        })
        .expect("a large real unit clear of every ring binds");
    assert_eq!(result.name, "Mahārāshtra");
    assert!(result.geometry.is_some());

    let districts = service
        .geo_boundary_units(&GeoBoundaryUnitsRequest {
            iso3: "IND".to_owned(),
            level: "ADM2".to_owned(),
            parent_provider_object_id: Some("gb:IND:ADM1:MH".to_owned()),
            ..Default::default()
        })
        .expect("districts inside Maharashtra");
    assert_eq!(
        districts
            .units
            .iter()
            .map(|unit| (unit.name.as_str(), unit.bindable))
            .collect::<Vec<_>>(),
        vec![("Pune", true), ("Unverified", false)]
    );
    assert_eq!(districts.total, 2);
}

#[test]
fn unverified_units_are_listed_unbindable_with_a_typed_notice() {
    let catalog = Arc::new(FakeCatalog {
        real_maharashtra: true,
        ..Default::default()
    });
    let service = geo_service(catalog);
    let districts = service
        .geo_boundary_units(&GeoBoundaryUnitsRequest {
            iso3: "IND".to_owned(),
            level: "ADM2".to_owned(),
            ..Default::default()
        })
        .expect("ADM2 units");
    let unverified = districts
        .units
        .iter()
        .find(|unit| unit.name == "Unverified")
        .expect("an unverified unit is still listed");
    assert!(!unverified.bindable);
    assert_eq!(
        unverified.unavailable_reason.as_deref(),
        Some("geometry_unverifiable")
    );
    assert_eq!(unverified.bbox.as_ref().map(|bbox| bbox.west), Some(74.0));
    assert_eq!(districts.unavailable, 2);
    assert_eq!(districts.total, 4);
    let notice = districts
        .notices
        .iter()
        .find(|notice| notice.code == "GEOBOUNDARIES_GEOMETRY_UNAVAILABLE")
        .expect("typed unavailable notice");
    assert_eq!(notice.count, Some(2));
    // Unavailable is not disputed: the disputed notice counts only W1.
    let disputed = districts
        .notices
        .iter()
        .find(|notice| notice.code == "GEOBOUNDARIES_WITHHELD_DISPUTED")
        .expect("typed disputed notice");
    assert_eq!(disputed.count, Some(1));

    let error = provider_code(
        service
            .geo_boundary_resolve(&GeoBoundaryResolveRequest {
                provider_object_id: "gb:IND:ADM2:U1".to_owned(),
            })
            .expect_err("an unverified unit never binds"),
    );
    assert_eq!(error.code, "BOUNDARY_GEOMETRY_UNAVAILABLE");
    assert_ne!(error.code, "BOUNDARY_ROUTE_REFUSED_DISPUTED");
}

#[test]
fn identity_withheld_units_are_hidden_even_when_a_catalog_lists_them() {
    let catalog = Arc::new(FakeCatalog {
        unfiltered_identity: true,
        ..Default::default()
    });
    let service = geo_service(catalog);
    let prefectures = service
        .geo_boundary_units(&GeoBoundaryUnitsRequest {
            iso3: "JPN".to_owned(),
            level: "ADM1".to_owned(),
            ..Default::default()
        })
        .expect("JPN ADM1");
    assert_eq!(
        prefectures
            .units
            .iter()
            .map(|unit| unit.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Hokkaido"]
    );
    // W1 from the catalog plus Okinawa withheld here.
    assert_eq!(prefectures.withheld_disputed, 2);
    let error = provider_code(
        service
            .geo_boundary_resolve(&GeoBoundaryResolveRequest {
                provider_object_id: "gb:JPN:ADM1:47310658B31569081891025".to_owned(),
            })
            .expect_err("Okinawa never binds"),
    );
    assert_eq!(error.code, "BOUNDARY_ROUTE_REFUSED_DISPUTED");
    assert_eq!(error.debug_context["reason"], "withheld_by_identity");
    assert_eq!(error.debug_context["fallback"], "china_selector");
}
